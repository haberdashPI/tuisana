//! In-memory Asana client used by tests.

use std::collections::HashMap;

use crate::{
    asana::{
        dto::{ProjectCustomFieldSettingDto, SectionDto, TaskDto},
        AsanaClient, TaskLoadScope, TaskQuery, TaskTarget,
    },
    domain::Project,
    error::{Error, Result},
};

/// A simple in-memory `AsanaClient` implementation for tests.
#[derive(Clone, Debug, Default)]
pub struct FakeAsanaClient {
    projects: Vec<Project>,
    tasks_by_project: HashMap<String, Vec<TaskDto>>,
    assigned_to_me_tasks: Vec<TaskDto>,
    current_user_gid: Option<String>,
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

    /// Adds task fixtures returned for an assigned-to-me query.
    pub fn with_assigned_to_me_tasks(mut self, tasks: Vec<TaskDto>) -> Self {
        self.assigned_to_me_tasks = tasks;
        self
    }

    /// Sets the gid returned by `current_user_gid`, simulating a logged-in user.
    pub fn with_current_user_gid(mut self, gid: impl Into<String>) -> Self {
        self.current_user_gid = Some(gid.into());
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

    fn list_tasks(&self, query: &TaskQuery) -> Result<Vec<TaskDto>> {
        let tasks = match &query.target {
            TaskTarget::Project(project_gid) => {
                self.tasks_by_project.get(project_gid).cloned().unwrap_or_default()
            }
            TaskTarget::AssignedToMe(_) => self.assigned_to_me_tasks.clone(),
        };
        Ok(tasks
            .into_iter()
            .filter(|task| matches!(query.scope, TaskLoadScope::All) || !task.completed)
            .filter(|task| {
                if let Some(after) = &query.due_after {
                    match &task.due_on {
                        None => return false,
                        Some(due) => {
                            if due < after {
                                return false;
                            }
                        }
                    }
                }
                true
            })
            .filter(|task| {
                if let Some(before) = &query.due_before {
                    match &task.due_on {
                        None => return false,
                        Some(due) => {
                            if due > before {
                                return false;
                            }
                        }
                    }
                }
                true
            })
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

    fn current_user_gid(&self) -> Result<String> {
        self.current_user_gid
            .clone()
            .ok_or_else(|| Error::Backend("no fake current user configured".to_string()))
    }
}
