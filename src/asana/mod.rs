//! Asana client abstractions and backend DTOs.
//!
//! The rest of the app talks to the `AsanaClient` trait so tests can use the
//! in-memory fake client while production uses the HTTP implementation.

pub mod client;
pub mod dto;
pub mod fake;
pub mod throttle;

use crate::{
    asana::dto::{ProjectCustomFieldSettingDto, SectionDto, TaskDto, UserDto},
    domain::{NewTask, Project, ProjectEdit, ProjectKind, TaskFieldEdit},
    error::Result,
};

/// Most actions Asana's `/batch` endpoint accepts in one request.
///
/// Its own hard cap, not a tuning choice: a longer `actions` array is refused
/// outright.
pub const MAX_BATCH_ACTIONS: usize = 10;

/// One write to one task, in the shape the write pipeline queues it.
///
/// The four variants are four different endpoints — `PUT /tasks/{gid}`,
/// `addProject`/`removeProject`, `setParent`, and `DELETE /tasks/{gid}` — and
/// the reason they share a type is that [`AsanaClient::write_tasks`] can send
/// any mixture of them in one batch. Deliberately free of rollback
/// information: `PendingEdit` in `app.rs` is the one that remembers how to
/// undo itself, and the client has no business knowing.
#[derive(Clone, Debug, PartialEq)]
pub enum TaskWrite {
    Field { gid: String, edit: TaskFieldEdit },
    Project(ProjectEdit),
    Parent {
        gid: String,
        parent_gid: Option<String>,
    },
    Delete { gid: String },
}

impl TaskWrite {
    pub fn gid(&self) -> &str {
        match self {
            Self::Field { gid, .. } => gid,
            Self::Project(edit) => &edit.gid,
            Self::Parent { gid, .. } => gid,
            Self::Delete { gid } => gid,
        }
    }

    /// Whether this write needs a read of its own before it can be sent.
    ///
    /// Only a start-date edit does: see
    /// [`client::HttpAsanaClient::current_due`]. It is asked here rather than
    /// matched on inside the batch builder because the *cost* of a chunk — what
    /// the throttle charges for it — has to be known before the chunk is built.
    pub fn needs_read(&self) -> bool {
        matches!(
            self,
            Self::Field {
                edit: TaskFieldEdit::Start(_),
                ..
            }
        )
    }
}

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

/// Server-side filter parameters, applied by Asana to a task list request.
///
/// Filters Asana cannot express (full-text search, custom field values, regex)
/// must be applied client-side; they are absent here.
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
    /// Applies one field change to one task and returns the task as the
    /// server now has it.
    ///
    /// Returns the task rather than `()` so the caller can pick up the new
    /// `modified_at` — without it the next merge cannot tell the server's copy
    /// from a stale one.
    fn update_task(&self, task_gid: &str, edit: &TaskFieldEdit) -> Result<TaskDto>;
    /// Adds a task to a project, or takes it out of one.
    ///
    /// Separate from [`AsanaClient::update_task`] because membership is not a
    /// field on the task: Asana takes it through `addProject` /
    /// `removeProject`, one request per project, and answers with nothing
    /// worth keeping.
    fn update_task_project(&self, edit: &ProjectEdit) -> Result<()>;
    /// Creates a task, answering it as the server now has it.
    ///
    /// The reply carries the gid, which is the whole reason this write is not
    /// optimistic: every edit of the new row needs it, and a placeholder
    /// cannot stand in.
    fn create_task(&self, task: &NewTask) -> Result<TaskDto>;
    /// Deletes a task. Asana takes its subtasks with it, server-side.
    fn delete_task(&self, task_gid: &str) -> Result<()>;
    /// Hangs a task off another task, or off nothing with `None`.
    ///
    /// Its own endpoint, like project membership: Asana does not accept
    /// `parent` in a task update.
    fn set_task_parent(&self, task_gid: &str, parent_gid: Option<&str>) -> Result<()>;
    /// Sends a run of writes, answering one result per write, in order.
    ///
    /// The bulk path. One result per write rather than one for the lot,
    /// because a batch where two of twelve fail has to roll back exactly
    /// those two — and Asana reports each action's status separately for
    /// precisely that reason. `Ok(Some(modified_at))` carries the server's
    /// timestamp where the endpoint reports one; `addProject`, `setParent`,
    /// and `DELETE` answer `Ok(None)`.
    ///
    /// `writes` must be no longer than [`MAX_BATCH_ACTIONS`]; the caller
    /// chunks. The default implementation sends them one at a time through
    /// the single-write methods above, which is both what the fake client
    /// wants and what keeps this method from being a second,
    /// separately-maintained spelling of each write.
    fn write_tasks(&self, writes: &[TaskWrite]) -> Vec<Result<Option<String>>> {
        writes.iter().map(|write| self.write_task(write)).collect()
    }
    /// One write, through whichever single-write method it names.
    ///
    /// Not part of the surface an implementation overrides — it is the
    /// default [`AsanaClient::write_tasks`] expressed once so the fallback
    /// and the per-action error paths agree.
    fn write_task(&self, write: &TaskWrite) -> Result<Option<String>> {
        match write {
            TaskWrite::Field { gid, edit } => {
                self.update_task(gid, edit).map(|task| task.modified_at)
            }
            TaskWrite::Project(edit) => self.update_task_project(edit).map(|()| None),
            TaskWrite::Parent { gid, parent_gid } => self
                .set_task_parent(gid, parent_gid.as_deref())
                .map(|()| None),
            TaskWrite::Delete { gid } => self.delete_task(gid).map(|()| None),
        }
    }
    /// Puts a task in a section, which takes it out of its old one.
    ///
    /// So there is no paired removal: moving a task between two sections of
    /// the same project is one request.
    fn add_task_to_section(&self, section_gid: &str, task_gid: &str) -> Result<()>;
    /// Creates a section in a project, after `insert_after` when given.
    fn create_section(
        &self,
        project_gid: &str,
        name: &str,
        insert_after: Option<&str>,
    ) -> Result<SectionDto>;
    /// Deletes a section. Asana refuses a populated one.
    fn delete_section(&self, section_gid: &str) -> Result<()>;
    /// Everyone in the workspace, for the assignee picker.
    ///
    /// The only directory the app can offer that is not "people who already
    /// have a task on screen" — which excludes the most common reason to
    /// reassign a task. Callers cache the answer for the session and fall
    /// back to the loaded records when it fails.
    fn list_users(&self) -> Result<Vec<UserDto>>;
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
