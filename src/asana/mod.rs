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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskLoadScope {
    OpenOnly,
    All,
}

impl TaskLoadScope {
    /// Returns `true` when completed tasks should be included.
    pub fn includes_completed(self) -> bool {
        matches!(self, Self::All)
    }
}

/// The minimal Asana client surface used by the app and tests.
pub trait AsanaClient {
    fn list_projects(&self) -> Result<Vec<Project>>;
    fn list_tasks(&self, project_gid: &str, scope: TaskLoadScope) -> Result<Vec<TaskDto>>;
    fn list_subtasks(&self, task_gid: &str, scope: TaskLoadScope) -> Result<Vec<TaskDto>>;
    fn list_sections(&self, project_gid: &str) -> Result<Vec<SectionDto>>;
    fn list_project_custom_field_settings(
        &self,
        project_gid: &str,
    ) -> Result<Vec<ProjectCustomFieldSettingDto>>;
}
