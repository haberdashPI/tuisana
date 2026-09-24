//! In-memory Asana client used by tests.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use crate::{
    asana::{
        dto::{
            CustomFieldValueDto, EnumOptionDto, ProjectCustomFieldSettingDto, SectionDto, TaskDto,
            TaskMembershipDto, TaskMembershipProjectDto, UserDto,
        },
        AsanaClient, TaskLoadScope, TaskQuery, TaskTarget,
    },
    domain::{CustomFieldValue, Project, ProjectEdit, ProjectMembership, TaskFieldEdit},
    error::{Error, Result},
};

/// The `modified_at` the fake stamps on a task it has just updated.
///
/// Later than any timestamp a fixture is going to carry, which is what makes
/// "a stale fetch does not undo a confirmed edit" testable at all.
pub const FAKE_EDIT_MODIFIED_AT: &str = "9999-01-01T00:00:00.000Z";

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
    standalone_tasks: HashMap<String, TaskDto>,
    get_task_calls: RefCell<Vec<String>>,
    /// Every `update_task` call, in order.
    ///
    /// Shared across clones rather than owned per clone: the app hands each
    /// write to a worker thread with a *clone* of the client, so a per-clone
    /// log would record the calls somewhere the test cannot see them.
    update_calls: Arc<Mutex<Vec<(String, TaskFieldEdit)>>>,
    /// The edits applied so far, per task, so a later read sees them.
    applied_edits: Arc<Mutex<HashMap<String, Vec<TaskFieldEdit>>>>,
    /// Task gids whose updates fail, for the rollback tests.
    ///
    /// Covers both kinds of write: a test that wants a membership rollback
    /// and one that wants a field rollback are asking the same question.
    update_failures: HashSet<String>,
    /// Every `update_task_project` call, in order.
    project_update_calls: Arc<Mutex<Vec<ProjectEdit>>>,
    /// The membership changes applied so far, per task.
    applied_project_edits: Arc<Mutex<HashMap<String, Vec<ProjectEdit>>>>,
    /// The workspace directory `list_users` answers with.
    users: Vec<UserDto>,
    /// Set to make `list_users` fail, for the fallback test.
    users_unavailable: bool,
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

    /// Adds tasks that only `get_task` can reach.
    ///
    /// Use this for a task no list request returns — the parent of a subtask
    /// that is filtered out of every project and assignee query, for instance.
    pub fn with_standalone_tasks(mut self, tasks: Vec<TaskDto>) -> Self {
        for task in tasks {
            self.standalone_tasks.insert(task.gid.clone(), task);
        }
        self
    }

    /// The gids `get_task` has been called with, in order.
    pub fn get_task_calls(&self) -> Vec<String> {
        self.get_task_calls.borrow().clone()
    }

    /// Makes `update_task` fail for one task, for the rollback tests.
    pub fn with_update_failure(mut self, task_gid: impl Into<String>) -> Self {
        self.update_failures.insert(task_gid.into());
        self
    }

    /// Adds the users `list_users` answers with.
    pub fn with_users(mut self, users: Vec<(&str, &str)>) -> Self {
        self.users = users
            .into_iter()
            .map(|(gid, name)| UserDto {
                gid: gid.to_string(),
                name: Some(name.to_string()),
                display_name: Some(name.to_string()),
            })
            .collect();
        self
    }

    /// Makes `list_users` fail, so the caller has to fall back.
    pub fn with_users_unavailable(mut self) -> Self {
        self.users_unavailable = true;
        self
    }

    /// The membership changes `update_task_project` has been called with.
    pub fn project_update_calls(&self) -> Vec<ProjectEdit> {
        self.project_update_calls
            .lock()
            .expect("project update calls are not poisoned")
            .clone()
    }

    /// The `(task gid, change)` pairs `update_task` has been called with.
    pub fn update_calls(&self) -> Vec<(String, TaskFieldEdit)> {
        self.update_calls
            .lock()
            .expect("update calls are not poisoned")
            .clone()
    }

    /// Replays the edits this task has taken onto a stored fixture.
    ///
    /// The fixtures are plain maps, so an edit is remembered beside them and
    /// applied on the way out. That is enough for "a later `list_tasks` sees
    /// the change" without making every builder method thread-shared.
    fn with_applied_edits(&self, mut task: TaskDto) -> TaskDto {
        let applied = self
            .applied_edits
            .lock()
            .expect("applied edits are not poisoned");
        let Some(edits) = applied.get(&task.gid) else {
            return task;
        };
        for edit in edits {
            apply_to_dto(&mut task, edit);
        }
        drop(applied);

        let memberships = self
            .applied_project_edits
            .lock()
            .expect("applied project edits are not poisoned");
        if let Some(edits) = memberships.get(&task.gid) {
            for edit in edits {
                apply_membership_to_dto(&mut task, edit);
            }
        }
        task
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
                self.with_applied_edits(task)
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
                self.with_applied_edits(task)
            })
            .collect())
    }

    fn get_task(&self, task_gid: &str) -> Result<TaskDto> {
        self.get_task_calls.borrow_mut().push(task_gid.to_string());
        self.standalone_tasks
            .get(task_gid)
            .cloned()
            .or_else(|| {
                self.tasks_by_project
                    .values()
                    .chain(self.subtasks_by_task.values())
                    .flatten()
                    .chain(self.assigned_to_me_tasks.iter())
                    .find(|task| task.gid == task_gid)
                    .cloned()
            })
            .map(|task| self.with_applied_edits(task))
            .ok_or_else(|| Error::Backend(format!("no fake task {task_gid}")))
    }

    fn update_task(&self, task_gid: &str, edit: &TaskFieldEdit) -> Result<TaskDto> {
        self.update_calls
            .lock()
            .expect("update calls are not poisoned")
            .push((task_gid.to_string(), edit.clone()));

        if self.update_failures.contains(task_gid) {
            return Err(Error::Backend("403".to_string()));
        }

        self.applied_edits
            .lock()
            .expect("applied edits are not poisoned")
            .entry(task_gid.to_string())
            .or_default()
            .push(edit.clone());

        let mut task = self.get_task(task_gid)?;
        task.modified_at = Some(FAKE_EDIT_MODIFIED_AT.to_string());
        Ok(task)
    }

    fn update_task_project(&self, edit: &ProjectEdit) -> Result<()> {
        self.project_update_calls
            .lock()
            .expect("project update calls are not poisoned")
            .push(edit.clone());

        if self.update_failures.contains(&edit.gid) {
            return Err(Error::Backend("403".to_string()));
        }

        self.applied_project_edits
            .lock()
            .expect("applied project edits are not poisoned")
            .entry(edit.gid.clone())
            .or_default()
            .push(edit.clone());
        Ok(())
    }

    fn list_users(&self) -> Result<Vec<UserDto>> {
        if self.users_unavailable {
            return Err(Error::Backend("no fake user directory".to_string()));
        }
        Ok(self.users.clone())
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

/// Writes one membership change onto a stored task fixture.
///
/// A removal drops the whole membership; an add appends one with no section,
/// which is what Asana does when a task joins a project without being placed.
fn apply_membership_to_dto(task: &mut TaskDto, edit: &ProjectEdit) {
    match edit.membership {
        ProjectMembership::Remove => task
            .memberships
            .retain(|membership| membership.project.gid != edit.project_gid),
        ProjectMembership::Add => {
            if task
                .memberships
                .iter()
                .any(|membership| membership.project.gid == edit.project_gid)
            {
                return;
            }
            task.memberships.push(TaskMembershipDto {
                project: TaskMembershipProjectDto {
                    gid: edit.project_gid.clone(),
                    name: edit.project_name.clone(),
                },
                section: None,
            });
        }
    }
}

/// Writes one change onto a stored task fixture.
///
/// The mirror of `TaskFieldEdit::apply` one layer down: that one writes onto a
/// `TaskRecord`, this one onto the DTO a read will hand back.
fn apply_to_dto(task: &mut TaskDto, edit: &TaskFieldEdit) {
    match edit {
        TaskFieldEdit::Name(name) => task.name = name.clone(),
        TaskFieldEdit::Completed(completed) => task.completed = *completed,
        TaskFieldEdit::Due(date) => task.due_on = date.clone(),
        TaskFieldEdit::Start(date) => task.start_on = date.clone(),
        TaskFieldEdit::Assignee(assignee) => {
            task.assignee = assignee.as_ref().map(|assignee| UserDto {
                gid: assignee.handle.clone(),
                name: Some(assignee.display.clone()),
                display_name: Some(assignee.display.clone()),
            })
        }
        TaskFieldEdit::CustomField { gid, value } => {
            let name = task
                .custom_fields
                .iter()
                .find(|field| &field.gid == gid)
                .map(|field| field.name.clone())
                .unwrap_or_default();
            task.custom_fields.retain(|field| &field.gid != gid);
            if let Some(value) = value {
                task.custom_fields.push(CustomFieldValueDto {
                    gid: gid.clone(),
                    name,
                    display_value: Some(value.display()),
                    enum_value: match value {
                        CustomFieldValue::Enum { option_gid, name } => Some(EnumOptionDto {
                            gid: option_gid.clone(),
                            name: name.clone(),
                            enabled: true,
                        }),
                        _ => None,
                    },
                });
            }
        }
    }
}
