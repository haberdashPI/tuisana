# Milestone 11: Assigned-to-me task view

[← all milestones](../plan.md)

Goal:

- let users view tasks assigned to them across all projects, independent of any project selection
- tuisana resolves who "me" is from the authenticated Asana account itself (via a login/`/users/me`
  style lookup), with no manual config value to keep in sync, and shows a synthetic "No Project
  (Assigned to Me)" entry in the project list that, when checked, loads and displays those tasks
  the same way a selected project does

Deliverables:

- README documents that assigned-to-me is automatic and only requires `auth.workspace_gid`
- a `ProjectKind` enum on `Project` distinguishing real Asana projects from the "assigned to me"
  pseudo-project, backing a row in the project list that can be selected and toggled like any
  other project
- an `AsanaClient::current_user_gid` method that resolves the logged-in user's gid, used instead
  of any config value; failure is treated as "no current user known," not a fatal error
- a `TaskTarget` enum on `TaskQuery`, replacing the single-project `project_gid: String` field,
  that models "fetch tasks for project X" and "fetch tasks assigned to user gid Y" as distinct,
  exhaustively-matched variants instead of a magic project id
- Asana client support (`HttpAsanaClient` and `FakeAsanaClient`) for dispatching on `TaskTarget`
  to fetch assigned-to-me tasks without a project scope
- task loading and table wiring that treats the assigned-to-me selection like a normal project
  selection

Implementation notes:

- `src/asana/mod.rs`:
  - add to the `AsanaClient` trait:
    `fn current_user_gid(&self) -> Result<String>;` — resolves who's logged in; callers must treat
    an `Err` as "omit the assigned-to-me row," not propagate it as a startup failure
  - replace `TaskQuery.project_gid: String` with
    ```
    pub enum TaskTarget {
        Project(String),
        AssignedToMe(String), // the resolved current-user gid
    }
    ```
    and a `pub target: TaskTarget` field (`Default` is implemented manually as
    `Project(String::new())`, since `#[derive(Default)]` on an enum needs a unit variant)
  - add `impl TaskTarget { pub fn for_project(project: &Project) -> Self { ... } }`, matching on
    `project.kind`; for `ProjectKind::AssignedToMe`, `project.id` already holds the resolved
    current-user gid (see `Project::assigned_to_me`), so this is the single place a `Project`
    becomes a query target — no other module re-derives "is this the pseudo-project" from a string
  - `TaskQuery::for_project(project_gid, scope)` builds `TaskTarget::Project(...)`; add
    `TaskQuery::for_assigned_to_me(user_gid, scope)` building `TaskTarget::AssignedToMe(user_gid)`
  - `list_sections`/`list_project_custom_field_settings` keep taking a plain `project_gid: &str`;
    callers simply won't invoke them for an `AssignedToMe` project (see `build_dataset_for_projects`)
- `src/domain/project.rs`: add
  ```
  pub enum ProjectKind { Normal, AssignedToMe }
  ```
  and a `pub kind: ProjectKind` field on `Project`; `Project::new`/`with_visibility` set
  `kind: ProjectKind::Normal`; add `Project::assigned_to_me(user_gid: impl Into<String>) -> Self`
  that sets `id: user_gid.into()` (the resolved current-user gid — there is no fixed sentinel
  constant, since the id is only known once `current_user_gid` resolves) and
  `kind: ProjectKind::AssignedToMe` with the fixed name "No Project (Assigned to Me)"
- `src/asana/dto.rs`: add a `ResourceResponse<T> { data: T }` wrapper for single-resource Asana
  responses (`/users/me` returns one object, not a paginated collection like `CollectionResponse`)
- `src/asana/client.rs`:
  - implement `current_user_gid`: `GET users/me` with `opt_fields=gid`, decode as
    `ResourceResponse<UserDto>`, return `.data.gid`; no new client state is needed for this since
    the caller (see `App::load_projects` below) is what turns the resolved gid into a `Project`
  - `list_tasks_page` matches on `task_query.target` instead of always building
    `projects/{gid}/tasks`:
    - `TaskTarget::Project(gid)` — existing behavior, `GET projects/{gid}/tasks`
    - `TaskTarget::AssignedToMe(user_gid)` — `GET tasks` with `assignee=<user_gid>` and
      `workspace=<workspace_gid>` query params; return a client error if `workspace_gid` isn't
      configured, since Asana's `/tasks` list requires it
  - `list_sections_page`/`list_custom_field_settings_page` are unchanged; they're simply not
    called for the assigned-to-me row (see `build_dataset_for_projects`)
- `src/asana/fake.rs`: add `assigned_to_me_tasks: Vec<TaskDto>` and `current_user_gid:
  Option<String>` fields with `with_assigned_to_me_tasks(tasks)` / `with_current_user_gid(gid)`
  builders; `current_user_gid()` returns the configured gid or an `Err` by default (so tests that
  don't opt in behave like a client that couldn't resolve a user); `list_tasks` matches on
  `query.target` the same way `HttpAsanaClient` does
- `src/app/task.rs`:
  - `TaskState::desired_task_query` no longer needs a `project_gid` parameter — every caller
    immediately overrides the target per project via struct-update syntax, so it becomes
    `pub fn desired_task_query(&self) -> TaskQuery` with a placeholder default target
  - `can_serve_query_for_targets` takes `&[Project]` instead of `&[String]` so it can build
    `TaskTarget::for_project(project)` per target instead of guessing a target from a bare id
    string; `App::task_data_needs_refresh` passes `task_target_projects()` instead of
    `task_target_project_ids()`
  - `projects_requiring_load` (already `&[Project]`) and `build_dataset_for_projects` build each
    project's query via `TaskTarget::for_project(project)`; `build_dataset_for_projects` skips the
    `list_sections`/`list_project_custom_field_settings` calls entirely when
    `project.kind == ProjectKind::AssignedToMe`
  - `active_target_ids`/`TaskCache::records_for_targets` and `TaskRecord.project_gids` stay
    `Vec<String>`/id-based — that machinery is pure grouping and caching by opaque id with no query
    semantics, so it doesn't need to know about `TaskTarget`; the assigned-to-me row's `id` (the
    resolved user gid) flows through it exactly like a normal project id
- `src/app.rs`:
  - `App::load_projects` calls `self.client.current_user_gid()` after loading projects; on `Ok`,
    builds `Project::assigned_to_me(gid)` and passes it to `ProjectListState::set_assigned_to_me`;
    on `Err`, logs via `debug_log` and passes `None` so the row is simply omitted
  - `start_task_data_fetch` builds each project's per-request query via
    `TaskQuery { target: TaskTarget::for_project(project), ..query_template.clone() }`
  - the `Action::Open` handler (which builds `format!("https://app.asana.com/0/{}", p.id)`)
    matches on `project.kind` and skips building a URL for `ProjectKind::AssignedToMe`
- `src/app/project_list.rs`: a new `ProjectListState::set_assigned_to_me(Option<Project>)` setter
  (called from `App::load_projects`, not threaded through `load_with_visibility`'s constructor
  arguments — that would have touched every existing test call site for no benefit) replaces or
  removes the row and re-sorts; `sort_projects` pins `ProjectKind::AssignedToMe` first;
  `toggle_project_flags` and `project_visibility_config` exclude it so it can't be starred, hidden,
  or persisted; selection itself (`toggle_current_selection`, `selected_ids`, `selected_projects`)
  needs no changes since it already operates generically on `Project.id`
- `src/ui/project_list.rs`: the row renderer suppresses the `[*]`/`[hidden]` markers for
  `ProjectKind::AssignedToMe`

Acceptance criteria:

- a "No Project (Assigned to Me)" row appears in the project list when the client resolves a
  current user, and is absent when it can't (no config value involved either way)
- the row can be selected using the existing project selection commands and always sorts to the
  top of the list
- checking the row and switching to task mode loads and displays tasks assigned to the resolved
  user, independent of any other project selection, and works alongside real projects selected at
  the same time
- the row cannot be starred, hidden, or persisted into the config's project visibility list
- opening the assigned-to-me row does not attempt to open a real Asana project URL
- the query built for the assigned-to-me row is a distinct `TaskTarget::AssignedToMe(user_gid)`
  value, not a project id compared against a constant, and `TaskTarget`/`ProjectKind` are matched
  exhaustively everywhere they're consumed
- tests cover `current_user_gid` resolution (including failure), `ProjectKind`-based sorting/
  selection/visibility-config exclusion, the assignee-scoped Asana request built from
  `TaskTarget::AssignedToMe`, and that section/custom-field requests are skipped for that target
