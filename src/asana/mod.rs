//! Asana client abstractions and backend DTOs.
//!
//! The rest of the app talks to the `AsanaClient` trait so tests can use the
//! in-memory fake client while production uses the HTTP implementation.

pub mod client;
pub mod dto;
pub mod fake;

use crate::{
    asana::dto::{ProjectCustomFieldSettingDto, SectionDto, TaskDto},
    domain::Project,
    error::Result,
};

/// The completion scope the Asana client should use when fetching tasks.
///
/// `OpenOnly` matches the default UI behavior; `All` includes completed tasks
/// as well.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskLoadScope {
    #[default]
    OpenOnly,
    All,
}

impl TaskLoadScope {
    /// Returns `true` when completed tasks should be included.
    pub fn includes_completed(self) -> bool {
        matches!(self, Self::All)
    }
}

/// Server-side filter parameters that can be pushed down to an Asana task
/// list request. Filters Asana cannot express (full-text search, custom
/// field values, regex) must be applied client-side; they are absent here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskQuery {
    /// GID of the project whose tasks to fetch.
    pub project_gid: String,
    /// Whether to fetch all tasks or only incomplete ones.
    pub scope: TaskLoadScope,
    /// Only include tasks with a due date on or after this date (YYYY-MM-DD).
    pub due_after: Option<String>,
    /// Only include tasks with a due date on or before this date (YYYY-MM-DD).
    pub due_before: Option<String>,
}

impl TaskQuery {
    /// Builds a query for a project with no date bounds.
    pub fn for_project(project_gid: impl Into<String>, scope: TaskLoadScope) -> Self {
        Self { project_gid: project_gid.into(), scope, ..Self::default() }
    }

    /// Returns `true` if data loaded with `self` can serve a request for
    /// `other` through client-side filtering alone.
    ///
    /// A cached query covers a new one when it is at least as broad along
    /// every dimension: All covers OpenOnly; no date bound covers any bound;
    /// a wider date range covers a narrower one.
    pub fn covers(&self, other: &Self) -> bool {
        let scope_ok = matches!(self.scope, TaskLoadScope::All)
            || matches!(other.scope, TaskLoadScope::OpenOnly);
        let after_ok = match (&self.due_after, &other.due_after) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(x), Some(y)) => x <= y,
        };
        let before_ok = match (&self.due_before, &other.due_before) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(x), Some(y)) => x >= y,
        };
        scope_ok && after_ok && before_ok
    }
}

/// The minimal Asana client surface used by the app and tests.
pub trait AsanaClient {
    fn list_projects(&self) -> Result<Vec<Project>>;
    fn list_tasks(&self, query: &TaskQuery) -> Result<Vec<TaskDto>>;
    fn list_subtasks(&self, task_gid: &str, scope: TaskLoadScope) -> Result<Vec<TaskDto>>;
    fn list_sections(&self, project_gid: &str) -> Result<Vec<SectionDto>>;
    fn list_project_custom_field_settings(
        &self,
        project_gid: &str,
    ) -> Result<Vec<ProjectCustomFieldSettingDto>>;
}
