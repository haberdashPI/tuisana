use std::collections::HashMap;

use crate::{
    asana::{
        dto::{ProjectCustomFieldSettingDto, SectionDto, TaskDto},
        AsanaClient,
    },
    domain::Project,
    error::Result,
};

#[derive(Clone, Debug, Default)]
pub struct FakeAsanaClient {
    projects: Vec<Project>,
    tasks_by_project: HashMap<String, Vec<TaskDto>>,
    sections_by_project: HashMap<String, Vec<SectionDto>>,
    custom_field_settings_by_project: HashMap<String, Vec<ProjectCustomFieldSettingDto>>,
}

impl FakeAsanaClient {
    pub fn new(projects: Vec<Project>) -> Self {
        Self {
            projects,
            ..Self::default()
        }
    }

    pub fn with_default_projects() -> Self {
        Self::new(vec![Project::new("1", "Inbox", true), Project::new("2", "Backlog", false)])
    }

    pub fn with_tasks(mut self, project_gid: impl Into<String>, tasks: Vec<TaskDto>) -> Self {
        self.tasks_by_project.insert(project_gid.into(), tasks);
        self
    }

    pub fn with_sections(mut self, project_gid: impl Into<String>, sections: Vec<SectionDto>) -> Self {
        self.sections_by_project.insert(project_gid.into(), sections);
        self
    }

    pub fn with_custom_field_settings(
        mut self,
        project_gid: impl Into<String>,
        settings: Vec<ProjectCustomFieldSettingDto>,
    ) -> Self {
        self.custom_field_settings_by_project
            .insert(project_gid.into(), settings);
        self
    }
}

impl AsanaClient for FakeAsanaClient {
    fn list_projects(&self) -> Result<Vec<Project>> {
        Ok(self.projects.clone())
    }

    fn list_tasks(&self, project_gid: &str) -> Result<Vec<TaskDto>> {
        Ok(self
            .tasks_by_project
            .get(project_gid)
            .cloned()
            .unwrap_or_default())
    }

    fn list_sections(&self, project_gid: &str) -> Result<Vec<SectionDto>> {
        Ok(self
            .sections_by_project
            .get(project_gid)
            .cloned()
            .unwrap_or_default())
    }

    fn list_project_custom_field_settings(
        &self,
        project_gid: &str,
    ) -> Result<Vec<ProjectCustomFieldSettingDto>> {
        Ok(self
            .custom_field_settings_by_project
            .get(project_gid)
            .cloned()
            .unwrap_or_default())
    }
}
