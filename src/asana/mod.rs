pub mod client;
pub mod dto;
pub mod fake;

use crate::{
    asana::dto::{ProjectCustomFieldSettingDto, SectionDto, TaskDto},
    domain::Project,
    error::Result,
};

pub trait AsanaClient {
    fn list_projects(&self) -> Result<Vec<Project>>;
    fn list_tasks(&self, project_gid: &str) -> Result<Vec<TaskDto>>;
    fn list_subtasks(&self, task_gid: &str) -> Result<Vec<TaskDto>>;
    fn list_sections(&self, project_gid: &str) -> Result<Vec<SectionDto>>;
    fn list_project_custom_field_settings(
        &self,
        project_gid: &str,
    ) -> Result<Vec<ProjectCustomFieldSettingDto>>;
}
