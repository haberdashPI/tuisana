//! In-memory Asana client used by tests.

use std::collections::HashMap;

use crate::{
    asana::{
        dto::{ProjectCustomFieldSettingDto, SectionDto, TaskDto},
        AsanaClient, TaskLoadScope,
    },
    domain::Project,
    error::Result,
};

/// A simple in-memory `AsanaClient` implementation for tests.
#[derive(Clone, Debug, Default)]
pub struct FakeAsanaClient {
    projects: Vec<Project>,
    tasks_by_project: HashMap<String, Vec<TaskDto>>,
    subtasks_by_task: HashMap<String, Vec<TaskDto>>,
    sections_by_project: HashMap<String, Vec<SectionDto>>,
    custom_field_settings_by_project: HashMap<String, Vec<ProjectCustomFieldSettingDto>>,
}

impl FakeAsanaClient {
    /// Builds a fake client with the given projects.
    pub fn new(projects: Vec<Project>) -> Self {
        Self {
            projects,
            ..Self::default()
        }
    }

    /// Builds a fake client with the repository's default sample projects.
    pub fn with_default_projects() -> Self {
        Self::new(vec![Project::new("1", "Inbox", true), Project::new("2", "Backlog", false)])
    }

    /// Adds task fixtures for a project.
    pub fn with_tasks(mut self, project_gid: impl Into<String>, tasks: Vec<TaskDto>) -> Self {
        self.tasks_by_project.insert(project_gid.into(), tasks);
        self
    }

    /// Adds subtask fixtures for a task.
    pub fn with_subtasks(mut self, task_gid: impl Into<String>, subtasks: Vec<TaskDto>) -> Self {
        self.subtasks_by_task.insert(task_gid.into(), subtasks);
        self
    }

    /// Adds section fixtures for a project.
    pub fn with_sections(mut self, project_gid: impl Into<String>, sections: Vec<SectionDto>) -> Self {
        self.sections_by_project.insert(project_gid.into(), sections);
        self
    }

    /// Adds custom-field settings fixtures for a project.
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

    fn list_tasks(&self, project_gid: &str, scope: TaskLoadScope) -> Result<Vec<TaskDto>> {
        Ok(self
            .tasks_by_project
            .get(project_gid)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|task| matches!(scope, TaskLoadScope::All) || !task.completed)
            .into_iter()
            .map(|mut task| {
                if let Some(subtasks) = self.subtasks_by_task.get(&task.gid) {
                    task.num_subtasks = subtasks.len();
                }
                task
            })
            .collect())
    }

    fn list_subtasks(&self, task_gid: &str, scope: TaskLoadScope) -> Result<Vec<TaskDto>> {
        Ok(self
            .subtasks_by_task
            .get(task_gid)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|task| matches!(scope, TaskLoadScope::All) || !task.completed)
            .into_iter()
            .map(|mut task| {
                if let Some(subtasks) = self.subtasks_by_task.get(&task.gid) {
                    task.num_subtasks = subtasks.len();
                }
                task
            })
            .collect())
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
