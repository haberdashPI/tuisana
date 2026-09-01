//! Asana client abstractions and backend DTOs.
//!
//! The rest of the app talks to the `AsanaClient` trait so tests can use the
//! in-memory fake client while production uses the HTTP implementation.

pub mod client;
pub mod dto;
pub mod fake;

use crate::{
    asana::dto::{ProjectCustomFieldSettingDto, SectionDto, TaskDto},
    domain::{Project, ProjectKind},
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

/// What a task query is scoped to: a single Asana project, or the current
/// user's assigned tasks independent of any project.
///
/// This is matched exhaustively everywhere a query is built or dispatched, so
/// adding a new scope is a compile-time-checked change rather than a new
/// magic id to thread through string comparisons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskTarget {
    Project(String),
    /// Fetch tasks assigned to this Asana user gid, independent of project.
    ///
    /// The gid is resolved once at startup via `AsanaClient::current_user_gid`
    /// (the currently logged-in user), not configured by hand.
    AssignedToMe(String),
}

impl Default for TaskTarget {
    fn default() -> Self {
        Self::Project(String::new())
    }
}

impl TaskTarget {
    /// Derives the query target for a project from its `ProjectKind`.
    ///
    /// For `ProjectKind::AssignedToMe`, `project.id` already holds the
    /// resolved current-user gid (see `Project::assigned_to_me`).
    pub fn for_project(project: &Project) -> Self {
        match project.kind {
            ProjectKind::Normal => Self::Project(project.id.clone()),
            ProjectKind::AssignedToMe => Self::AssignedToMe(project.id.clone()),
        }
    }
}

/// Server-side filter parameters that can be pushed down to an Asana task
/// list request. Filters Asana cannot express (full-text search, custom
/// field values, regex) must be applied client-side; they are absent here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskQuery {
    /// What the query is scoped to: a project or the current user's assigned tasks.
    pub target: TaskTarget,
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
        Self { target: TaskTarget::Project(project_gid.into()), scope, ..Self::default() }
    }

    /// Builds a query for the current user's assigned tasks with no date bounds.
    pub fn for_assigned_to_me(user_gid: impl Into<String>, scope: TaskLoadScope) -> Self {
        Self { target: TaskTarget::AssignedToMe(user_gid.into()), scope, ..Self::default() }
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
    /// Fetches a single task by gid, regardless of project or assignee.
    ///
    /// Needed to look up a task that no list request returned — notably the
    /// parent of a subtask fetched by assignee, which decides the project the
    /// subtask is grouped under even when the parent itself is filtered out of
    /// the loaded set.
    fn get_task(&self, task_gid: &str) -> Result<TaskDto>;
    fn list_sections(&self, project_gid: &str) -> Result<Vec<SectionDto>>;
    fn list_project_custom_field_settings(
        &self,
        project_gid: &str,
    ) -> Result<Vec<ProjectCustomFieldSettingDto>>;
    /// Resolves the gid of the currently authenticated Asana user ("me").
    ///
    /// Used to offer the "assigned to me" pseudo-project without any manual
    /// config value; callers should treat failure as "no current user known"
    /// rather than a fatal error.
    fn current_user_gid(&self) -> Result<String>;
}

#[cfg(test)]
mod tests {
    use super::{TaskLoadScope, TaskQuery, TaskTarget};
    use crate::domain::Project;

    #[test]
    fn task_target_for_project_matches_on_kind() {
        let normal = Project::new("1", "Inbox", false);
        let assigned_to_me = Project::assigned_to_me("user_1");

        assert_eq!(TaskTarget::for_project(&normal), TaskTarget::Project("1".to_string()));
        assert_eq!(
            TaskTarget::for_project(&assigned_to_me),
            TaskTarget::AssignedToMe("user_1".to_string())
        );
    }

    #[test]
    fn for_project_and_for_assigned_to_me_build_distinct_targets() {
        let project_query = TaskQuery::for_project("1", TaskLoadScope::All);
        let assigned_query = TaskQuery::for_assigned_to_me("user_1", TaskLoadScope::All);

        assert_eq!(project_query.target, TaskTarget::Project("1".to_string()));
        assert_eq!(assigned_query.target, TaskTarget::AssignedToMe("user_1".to_string()));
    }
}
