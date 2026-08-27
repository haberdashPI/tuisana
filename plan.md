# Tuisana Plan

## Context

This repository is a greenfield Rust project for a minimal terminal UI that reviews and edits Asana tasks. The app should be built as a small, testable Rust CLI/TUI with a clear separation between:

- terminal UI rendering
- input handling and key binding configuration
- application state and navigation
- Asana API access
- normalized domain models
- tests with a fake Asana backend

Primary product goal:

- start by listing Asana projects, then review tasks for one or more selected projects, then add navigation, filtering, sorting, and editing capabilities

Design constraints:

- use a modern Rust TUI framework, preferably `ratatui` with `crossterm`
- keep the UI thin and move business logic into domain/app modules
- make keybindings configurable from TOML
- support vim-like shortcuts
- support unit tests for logic and a few integration tests for end-to-end state transitions

Implementation assumptions:

- Asana API access should be wrapped behind a trait such as `AsanaClient`
- the UI should never call HTTP directly
- use an in-memory or fake Asana implementation for tests
- store config in TOML and keep the schema small and explicit
- prefer simple, predictable state machines over over-engineered abstractions

If an implementation detail is unclear, use the simplest choice that preserves testability and keeps future extension easy.

## Recommended Architecture

Top-level source layout:

- `src/main.rs`
- `src/app.rs`
- `src/ui.rs`
- `src/error.rs`
- `src/config/mod.rs`
- `src/config/schema.rs`
- `src/config/keys.rs`
- `src/config/defaults.rs`
- `src/input/mod.rs`
- `src/input/action.rs`
- `src/input/keymap.rs`
- `src/domain/mod.rs`
- `src/domain/project.rs`
- `src/domain/task.rs`
- `src/domain/section.rs`
- `src/domain/filter.rs`
- `src/domain/sort.rs`
- `src/domain/task_query.rs`
- `src/domain/task_table.rs`
- `src/domain/task_edit.rs`
- `src/asana/mod.rs`
- `src/asana/client.rs`
- `src/asana/dto.rs`
- `src/asana/projects.rs`
- `src/asana/tasks.rs`
- `src/asana/sections.rs`
- `src/asana/custom_fields.rs`
- `src/asana/dependencies.rs`
- `src/asana/fake.rs`
- `src/app/project_list.rs`
- `src/app/task_review.rs`
- `src/app/task_edit.rs`
- `src/ui/project_list.rs`
- `src/ui/task_table.rs`
- `src/ui/help_overlay.rs`

Tests:

- `tests/config_tests.rs`
- `tests/keymap_parse_tests.rs`
- `tests/keymap_action_tests.rs`
- `tests/app_state_tests.rs`
- `tests/project_sort_tests.rs`
- `tests/project_list_integration.rs`
- `tests/task_table_model_tests.rs`
- `tests/multi_project_merge_tests.rs`
- `tests/custom_field_mapping_tests.rs`
- `tests/task_filter_tests.rs`
- `tests/task_sort_tests.rs`
- `tests/subtask_filter_tests.rs`
- `tests/task_nav_tests.rs`
- `tests/task_edit_parse_tests.rs`
- `tests/task_mutation_tests.rs`
- `tests/task_edit_integration.rs`
- `tests/error_handling_tests.rs`
- `tests/help_overlay_tests.rs`
- `tests/default_config_tests.rs`

## Milestone 0: Project skeleton

Goal:

- establish the module boundaries, config plumbing, and test seams before UI and API work

Deliverables:

- crate structure with the modules above
- minimal app state type
- config loader for TOML
- input action enum and keymap parser
- fake Asana client for tests
- basic error type

Acceptance criteria:

- the project builds
- config can be parsed from TOML
- app state can be instantiated with a fake backend
- unit tests cover basic config and state setup

## Milestone 1: List projects

Goal:

- fetch and display all Asana projects
- show starred projects before non-starred projects

Deliverables:

- `AsanaClient` project listing method
- `Project` domain model
- project list state and UI
- deterministic ordering logic

Implementation notes:

- sort first by starred flag, then by name as a stable secondary key
- support loading, empty, and error states
- keep the list renderable without a live Asana account by using the fake client

Acceptance criteria:

- project list renders
- starred projects appear above non-starred projects
- selection state works for the list
- tests verify ordering and rendering model behavior

## Milestone 2: Asana API setup

Goal:

- define the real Asana API configuration and wire a production client behind the existing trait boundary

Deliverables:

- config schema for Asana authentication and API settings
- real `AsanaClient` implementation for authenticated project listing
- minimal client construction flow in the app startup path
- tests for config parsing, validation, and client wiring with fakes or injected HTTP boundaries

Implementation notes:

- keep authentication inputs explicit and small
- prefer config fields that can support both local development and future deployment modes
- avoid exposing HTTP details to UI or app state code
- keep the real client behind the existing trait so project-list and task-review code can stay testable

Acceptance criteria:

- config can express the information needed to authenticate with Asana
- the app can construct a real Asana client from config
- project listing can use the real client through the trait boundary
- tests cover the new config and client construction seams without requiring live Asana credentials

## Milestone 3: Configurable vim-like project navigation

Goal:

- navigate the project list using configurable vim-like shortcuts from TOML

Deliverables:

- action model for navigation and app commands
- key binding config schema
- key-to-action resolution
- basic navigation commands such as up, down, page up/down, open, refresh, quit

Implementation notes:

- keep the action enum small and explicit
- prefer data-driven key bindings over hardcoded keys
- do not let UI code interpret raw key events directly; translate them into actions first

Acceptance criteria:

- navigation works in the project list
- key bindings are configurable from TOML
- tests cover config parsing and action mapping

## Milestone 4: Project visibility management

Goal:

- let users mark projects as starred or hidden and persist those preferences in `tuisana.toml`
- keep hidden projects available behind a toggle, with a visible marker and hint between
the unhidden projects and the (visible or invisible) hidden projects
- sort starred projects before unstarred projects, and place hidden projects after the visible group when hidden items are shown

Deliverables:

- config schema for per-project visibility metadata
- project list state for applying star and hidden preferences
- ordering logic that groups starred projects first, then unstarred visible projects, then hidden projects when visible
- UI markers and hints that make hidden projects obvious in the list
- toggle handling for showing and hiding the hidden group

Implementation notes:

- keep visibility preferences explicit and stored in the existing TOML config
- make the hidden-project toggle affect presentation only, not the underlying project data
- preserve deterministic ordering within each visibility group
- keep the marker and hint unobtrusive but easy to notice

Acceptance criteria:

- starred projects appear before non-starred projects
- hidden projects can be toggled visible and hidden
- when visible, hidden projects appear after the remaining projects
- hidden projects have an obvious marker and a hint explaining how to toggle their visibility
- tests cover the ordering and visibility toggling behavior

## Milestone 5: Multi-select project management

goal:

- let users manage the project view directly from the list
- support selecting multiple projects for bulk actions
- support filtering the project list with fuzzy, substring, and regex search modes
- let users focus the view down to only selected projects when needed

Deliverables:

- multi-select project state in the project view
- bulk actions for toggling starred and hidden state on the selected projects
- search/filter state with toggles for fuzzy, substring, and regex matching
- selected-project count in the status area
- a toggle to show only the selected projects
- selection behavior that remains stable even when filtering hides rows

Implementation notes:

- keep selection state separate from filter state so hidden rows can remain selected
- treat filter mode as presentation only, not as a destructive data change
- make bulk actions apply to the current selected set, not just the cursor row
- preserve deterministic ordering for filtered and selected subsets
- keep the search mode shortcuts small and explicit

Acceptance criteria:

- multiple projects can be selected in the project view
- bulk toggles can mark selected projects starred or hidden
- the status line shows how many projects are selected
- filter mode can be switched between fuzzy, substring, and regex matching
- filtered-out rows do not lose their selection state
- a toggle exists to show only selected projects
- tests cover selection, bulk actions, and filter-mode behavior

## Milestone 5.5: Improve Selection Commands

Goal:

- improve the usability of project search and selection

Deliverables:

- commands to clear the search string and reveal all items
- commands to select all visible items
- commands to invert the selection for visible items
- commands to jump to the top or bottom
- undo and redo selection actions
- page up and page down behavior that jumps by page size instead of one item
- a more visually prominent label of the current search state

Implementation notes:

- only show search toggles when search is non-empty
- only show the command to clear search when search is non-empty
- only show selection modifiers when there is at least one item selected
- keep the search-state label descriptive for the current mode and input state

Acceptance criteria:

- the search string can be cleared to reveal all items
- visible items can be selected in bulk
- visible selection can be inverted
- the cursor can jump to the top or bottom
- selection actions can be undone and redone
- page up and page down move by page size
- search toggles are only shown when search is active
- selection modifiers are only shown when something is selected

## Milestone 6: Review tasks for one or many projects

Goal:

- show tasks in a table for one selected project or a user-defined set of projects

Deliverables:

- a task view that shows up below the project list
- task loading from Asana
- section data loading
- custom field discovery for project-specific fields
- task table model with common columns and project-specific columns
- when visible the task view should take up most of the screen: e.g. just show the first 4-6 lines of the project view.
- task view can be easily toggled as visible / hidden
- user can toggle between "project" mode (which interacts with the project list) and "task" mode (which interacts with the tasks)
    - toggling between these modes only changes visibility of the task list
      when the user switches to the task mode: in this case the task list is
      made visible if it isn't already

Suggested default table fields:

- task name
- section
- assignee
- due date
- start date
- completion state
- project-specific custom fields
- any other useful metadata that is cheap to fetch

Implementation notes:

- keep the table model separate from raw API DTOs
- normalize Asana data before rendering
- support a single-project view and a multi-project aggregated view
- decide explicitly how duplicate tasks across selected projects should be handled

Acceptance criteria:

- tasks can be loaded for one project
- tasks can be loaded across a configured project set
- table columns include key fields and project-specific fields
- tests cover field mapping and multi-project merging

## Milestone 6.5: Improve task view behavior

Goal:

- make the task pane feel responsive, predictable, and easy to read

Deliverables:

- defer task loading until task mode is activated
- show a visible indication that tasks are pending when a project is selected
- load tasks asynchronously with a spinner or other loading indicator
- render task columns with readable spacing and truncation
- group tasks by project and section with clear visual separation

Implementation notes:

- avoid loading task data on every project cursor move
- keep task loading non-blocking so the project list remains usable
- use column alignment and truncation instead of raw tab-separated text
- make the default sort order stable and section-aware
- preserve the single-project and multi-project task table model introduced in Milestone 6

Acceptance criteria:

- selecting a project does not immediately trigger task loading
- switching into task mode loads the relevant tasks for the current selection
- the UI makes it clear when task data is pending or loading
- task loading does not freeze the rest of the interface
- task rows are legible with aligned columns and truncated overflow
- tasks are visually grouped by project and section
- tests cover deferred loading, loading-state presentation, and table formatting

## Milestone 7: Navigate, filter, and sort tasks

Goal:

- navigate the task list with vim-like shortcuts
    - up, down, page-up, page-down
    - move by section
    - move by project
- filter by:
    - field
    - subtask visibility
    - complete/incomplete tasks
    - task owner
    - date range
- sort tasks predictably

Deliverables:

- task navigation actions
- filter model
- sort model
- subtask include/exclude toggle
- stable ordering rules
- keybinding defaults for all commands

Implementation notes:

- filters should be composable where practical
- support filtering by explicit field, text, fuzzy and regex search
- make subtask visibility a first-class filter, not an afterthought
- subtasks always show up under their associated super tasks
- if subtasks are visible, the super task is visible when any of the subtasks match a filter or when the super task matches the filter
- use a stable fallback sort to avoid jumpy rows

Acceptance criteria:

- row navigation works
- filtering by field works
- subtask visibility can be toggled
- sorting is deterministic
- tests cover filter and sort combinations

## Milestone 7.5: Refine views / binding setup

Goal:

- refine the project, filter, and task views so the UI feels consistent and the bindings are easier to learn

Deliverables:

For the task view:
- a spacebar command that toggles selection and then moves to the next item
- command to select all starred items
- command to select all non-hidden items

For window organization:
- a shared top window for project and filter views, with only one visible at a time
- a consistent mode model for `task`, `filter`, and `project`, with `m` removed
- a dedicated mode line that prominently shows the current mode and available mode-switch bindings
- commands for resizing the task/filter window, including move, minimize, maximize, and restore behavior
- a single help area that shows the current mode's relevant bindings, with a compact default view and `?` for expansion
- a single status area that shows only the statuses relevant to the current mode

Implementation notes:

- keep project and filter views in the same screen region so they behave like alternate uses of the same window
- make the task/filter window resizing preserve the previous intermediate size when minimized or maximized
- keep mode-specific bindings visible only when they apply to the current mode
- verify that all keybindings are documented in the README

Acceptance criteria:

- space selects an item and moves the cursor down
- starred items can be selected in bulk
- hidden items can be excluded from bulk selection
- only one of the project view or filter view is visible in the shared top window at a time
- the mode line clearly shows the active mode and the bindings for switching modes
- task and filter panes can be resized, minimized, maximized, and restored
- the help area shows only the relevant bindings for the active mode by default
- statuses are shown in one place only and are filtered to the current mode
- all keybindings are documented in the README

## Milestone 8: Code legibility

In progress

Manual review status:

- [X] src/main.rs
- [X] src/app.rs
- [X] src/ui/runtime.rs
- [X] src/config/mode.rs
- [X] src/input/mod.rs
- [X] src/ui/task_table.rs
- [X] src/app/task_review.rs

The remaining files have yet to be cleaned up at all

Goal

- ensure that a someone new to the project can easily understand
  and contribute the code

Deliverables:

- a developer.md doc contains documentation for getting started in
  reading through the code.
- the core data structures are documented with their role in the
  application, especially the app state machine and the runtime input
  flow.
- the names of functions and modules are self-explanatory
- code is often self-documenting: it is clear from the calls
  and functions what is happening.
- There are basic primitives used throughout the project that
  that encaspulate common patterns used throughout the project
- Functions are relatively short: when necessary, large functions have clear documented
  sections.
- The new organization is not needlessly inflexible: it should be easy
  to add or change functionality.
- The new organization is not needlessly abstract: there are a limited
  number of levels of indirection and it is general clear what
  a given piece of code is doing in concrete terms.

Implementation notes:

- This milestone may require a substantial rewrite. Consider whether
  the boundaries and categories of the code actually make sense
  now that we've implemented most of the functionality. The goal
  is to optimize for clarity and legibility over the most
  efficient possible implementation.
- We should keep in mind that we probably want to eventually introduce more error handling;
  that should not substantially reduce legibility once we do it in a future milestone.

Acceptance criteria:
- The behavior and appearance of the application remains unchanged.

## Milestone 8.5: Structural cleanup

Goal:

- eliminate the concrete legibility problems that remain after Milestone 8's manual review
- target four specific areas: the duplicated fuzzy-match function, the overly long filter input
  handler, the repeated mode-transition boilerplate, and the implicit task loading state machine

These are all internal refactors. The UI and test behavior must stay identical.

Deliverables:

- a single `fuzzy_match` function in a shared location, removing the duplicate in `task.rs`
- `handle_filter_field_input` broken into smaller pieces so neither editing-mode nor
  non-editing-mode logic exceeds ~40 lines
- the five `set_*_mode` helpers collapsed using a shared setup step so the restore-pane and
  end-search boilerplate is written once
- `TaskLoadingState` refactored into an explicit state-machine enum so valid and invalid state
  combinations are visible at the type level

Implementation notes:

- place `fuzzy_match` where both callers can reach it without a circular dependency; a small
  `util` module at the crate root is a natural home
- when splitting `handle_filter_field_input`, keep the two halves (editing vs. browsing) as
  separate named functions; do not introduce a new struct or trait unless a simpler split still
  leaves functions too long
- the guard `matches!(selected_kind, Some(TaskFieldFilterKind::Labels))` appears six times in
  a row inside the editing branch; extract it into a single check at the top of that branch
- the shared setup for mode transitions (restore pane if minimized, end project search if
  active) should live in one `fn prepare_mode_switch(&mut self)` called by each `set_*_mode`
  helper; keep per-mode differences in each helper
- for `TaskLoadingState`, a minimal enum with variants `Idle`, `Loading { … }`, and `Ready { … }`
  is sufficient; `OutOfDate` and `Error` can be added later if they simplify other code
- do not reorganize the public API of `TaskState` or `App` beyond what is required by the
  structural changes above; API renaming is out of scope for this milestone

Acceptance criteria:

- `fuzzy_match` is defined once; both the project-list and task filter call the same function
- `handle_filter_field_input` and each sub-function it calls are ≤ 60 lines each
- the six repeated label-kind guards in the editing branch are replaced by a single early exit
- `prepare_mode_switch` (or equivalent) is called instead of repeated inline boilerplate in
  each `set_*_mode` helper
- `TaskLoadingState` is an enum; the compiler rejects states that mix loading and ready fields
- all existing tests pass unchanged

## Milestone 9: Lazy, filter-aware task loading

Goal:

- avoid loading a large task set eagerly when only a small subset is needed
- push task retrieval decisions down to the current task filters and selected projects

Deliverables:

- a task query model that can express the active task filters and project scope
- Asana client support for requesting tasks using that query model
- app logic that requests only the data needed for the current task view state
- fallback behavior for cases where the requested query cannot be expressed lazily

Implementation notes:

- keep the query model explicit so the Asana client and fake backend can both implement it
- tailor the request to the active filter set instead of always loading whole project task trees
- preserve the existing eager path as a fallback for unsupported query combinations
- keep lazy loading incremental and cache-aware so switching projects does not discard useful data

Acceptance criteria:

- the app avoids querying the full task set when the active view only needs a narrower subset
- filter-aware task requests work through the existing client abstraction
- unsupported query combinations fall back to the current eager behavior
- tests cover the query model, lazy loading behavior, and fallback path

## Milestone 10: Task interaction ✓

Goal:

- make basic task interaction possible: opening tasks in Asana and copying task links

Deliverables:

- the currently highlighted task can be opened in the Asana app (default binding `enter`)
- the user can select tasks for bulk actions
  - like project selection: select all/none, invert; selections depend on the filtered state
  - like project selection: a task not currently visible can remain selected
  - the selection can be cleared of tasks that are not visible
- a "copy to clipboard" action produces a markdown checklist item for each selected task (default binding `y`):
  `- [ ] [task title](task link)`

Implementation notes:

- follow the project selection model for task selection state
- selected tasks not visible in the current filter should remain selected until explicitly cleared
- the clipboard output should be consistent markdown that renders correctly in common tools

Acceptance criteria:

- the highlighted task can be opened in Asana from the task view
- tasks can be selected and deselected; selection state persists through filter changes
- select all, invert, and clear-hidden-selection work on the current filtered task set
- the clipboard action produces a markdown checklist item for each selected task
- tests cover selection state transitions and clipboard output format

## Milestone 11: Assigned-to-me task view

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

## Milestone 11.5: Visual redesign ✓

Delivered. Three deliberate deviations from the plan below, each because the
planned version would have changed behavior:

1. **The filter panel is not grouped by kind.** Grouping into Text / Dates /
   Labels / Custom fields would either reorder the fields — making `j`/`k`
   appear to skip around, since they walk the list in index order — or repeat a
   heading when the kinds interleave (Title, Assignee, Due, Start, State,
   Projects). The match-mode chip in the second column carries the same
   information without touching navigation order, and keeping one line per
   field means the panel's scroll offset stays a plain field index.
2. **The help overlay does not scroll and does not close on `esc`.** Both would
   require capturing `j`/`k`/`esc` while it is open, which changes what those
   keys do. `?` toggles it, exactly as it toggled the inline help. The two
   columns are balanced to minimize the taller one so the overlay fits without
   scrolling.
3. **The `State` cell keeps `open`/`done` in the domain model.** The `State`
   label filter matches on those literal values, so only the *glyph* moved to
   the renderer. The subtask indent did move out of the domain, as planned:
   `TaskRow` now carries `subtask_depth` and the renderer draws the marker.

Also shipped beyond the plan: an ASCII border set so `glyphs = "ascii"` produces
an entirely ASCII screen, and a `columns n/m` chip in the task pane border so
horizontally clipped columns announce themselves.

Goal:

- replace the current flat, monochrome, text-dump presentation with a coherent visual design that is
  denser where it matters, quieter where it doesn't, and legible at a glance
- introduce one shared theme/glyph vocabulary instead of ad-hoc `Style::default()` calls scattered
  across the render functions
- move all long-form help out of the main layout and behind a `?` overlay
- **behavior must not change**: same keys, same actions, same modes, same row/selection indexing,
  same filter/sort/load semantics. This milestone only changes what is drawn and where.

### Current state (measured, 120x40 terminal)

Rendered snapshots of the three modes show the concrete problems this milestone fixes:

Project mode:

```
mode: project  p: project  f: filter  t: task  tasks: hidden
?: more hints, j/k: move, space: select+down, /: search (project), p/f/t: modes, r: refresh, q: quit
Projects
3 visible, 0 selected, 3 hidden
Search: not searching (substring)
┌Asana Projects──────────────────────────────────────────────────┐
│> [ ] [*] [hidden] Northwind BTX 4412                           │
│  [ ] [*] [hidden] Platform Infra                               │
│  [ ] [ ] [hidden] Backlog                                      │
│                          (28 blank lines of bordered nothing)  │
└────────────────────────────────────────────────────────────────┘
```

Task mode (expanded help):

```
mode: task  p: project  f: filter  t: task  tasks: visible
?: fewer hints
navigation: j/down,k/up move; ctrl-u/d page; home/end top/bottom
scroll: left/right columns (0/3)

grouping/sort: [] section jumps; {} project jumps; , project group; . section group; s sort

task interaction: enter open in Asana; space select; a all; i invert; x clear; ctrl-x clear hidden; y copy

filters: f panel; s cycle mode; esc/enter done; c completed; z subtasks

p/f/t: modes, r: refresh, q: quit
Projects
3 visible, 1 selected, 3 hidden
Search: not searching (substring)
┌Asana Projects──────────────────────────────────────────────────┐
│  [x] [*] [hidden] Northwind BTX 4412                           │
└────────────────────────────────────────────────────────────────┘
Task review (task focus)
2 tasks, row 1, ready, grp p:on s:on; comp open; sub show; sort date; filters off
┌Tasks───────────────────────────────────────────────────────────┐
│Task                        | Assignee     | Due        | State │
│────────────────────────────────────────────────────────────────│
│Northwind BTX 4412          |              |            |       │
│                            |              |            |       │
│Study Kit Design            |              |            |       │
│Review shipment requirements| Morgan Ellis | 2026-06-10 | open  │
└────────────────────────────────────────────────────────────────┘
```

Specific defects:

1. **No color anywhere.** Every span is `Style::default()` plus `BOLD`/`REVERSED`/`UNDERLINED`.
2. **Help dominates the layout.** Expanded help costs 11 lines at the top of the screen and pushes
   all content down; the layout reflows every time `?` is pressed.
3. **Triple-redundant pane chrome.** Each pane spends 3 unstyled lines (title, status, search) above
   a bordered block that repeats the title (`Projects` above `┌Asana Projects┐`, `Task review (task
   focus)` above `┌Tasks┐`).
4. **Mode line is noise.** `mode: project  p: project  f: filter  t: task  tasks: hidden` says
   "project" twice and spends most of its width on bindings that belong in the hint bar.
5. **Bracket soup in project rows.** `[ ] [*] [hidden] Name` is three ASCII markers before the only
   thing the user is reading.
6. **Pipes on empty rows.** `ProjectHeader`, `SectionHeader`, and `SectionSpacer` rows are padded to
   the full column grid, so group headings render as a name followed by a trail of empty `|` cells.
7. **Cryptic status line.** `grp p:on s:on; comp open; sub show; sort date; filters off` mixes commas
   and semicolons, abbreviates everything, and shows five facts that are usually at their defaults.
8. **Dates burn 22 columns.** `Due` and `Start` both render full `YYYY-MM-DD` with no indication that
   something is overdue or due today.
9. **Cursor vs. selection is hard to read.** Cursor is `REVERSED`, multi-select is `UNDERLINED`, and
   a row that is both is `REVERSED | UNDERLINED`.
10. **Truncation is inconsistent.** Only the title column gets an `…`; other columns are cut
    mid-word (`Priority` renders as `Prior`).
11. **Filter panel is unaligned.** `> Title [string:fuzzy]:` with no column alignment, no grouping,
    no indication of which filters are actually doing anything.
12. **Dead space.** A three-project list still draws a 30-line empty bordered box.

### Target design

Header bar (1 line, top), panes in the middle, hint bar and status bar (2 lines, bottom). The help
block disappears from the flow entirely. Focus is shown by a **heavy border** on the active pane so
the cue survives in monochrome terminals, reinforced by an accent-colored border and title.

Task mode:

```
 TUISANA  ▸ 2 projects  ▸ 14 tasks                              1 filter · sort due ▲ · open only
┌─ Projects ─────────────────────────────────────── 3 shown · 1 selected · 2 hidden ─────────────┐
│ ▍ ✓  ★  Northwind BTX 4412                                                                     │
│      ☆  Platform Infra                                                                         │
│      ☆  Backlog                                                                       hidden   │
└────────────────────────────────────────────────────────────────────────────────────────────────┘
┏━ Tasks ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 14 tasks · row 3 · 2 selected ━━━━━━━━━━━━━━━┓
┃    Task                                    Assignee       Due ▲    Start   St  Priority        ┃
┃ ▌Northwind BTX 4412 ───────────────────────────────────────────────────────────────────────    ┃
┃   Study Kit Design ·····                                                                       ┃
┃ ▍ ○ Ship release candidate to the study …  Alex Chen       Jun 10   Jun 1    ○  High           ┃
┃ ● ○ Review shipment requirements doc       Morgan Ellis    Today    Jun 1    ○  High           ┃
┃   ✓ Close out packaging vendor contract    Alex Chen       Jun 3    May 28   ✓  Low            ┃
┃     ↳ Confirm carrier pickup window        Alex Chen       —        —        ○  —              ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
 TASK   j/k move   ⏎ open   ␣ select   f filters   s sort   ? help              r refresh   q quit
```

Filter mode (the filter pane replaces the project pane in the shared top window, as today):

```
 TUISANA  ▸ 2 projects  ▸ 14 tasks                              1 filter · sort due ▲ · open only
┏━ Filters ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 1 active ━━━━━━━━━━━━━━━━━━┓
┃  Text                                                                                          ┃
┃ ▍ Title       fuzzy      ship▏                                                                 ┃
┃   Assignee    contains   —                                                                     ┃
┃   Projects    contains   —                                                                     ┃
┃  Dates                                                                                         ┃
┃   Due         date       —                                                                     ┃
┃   Start       date       —                                                                     ┃
┃  Labels                                                                                        ┃
┃ ● State       labels     open  ·  done                                                         ┃
┃  Custom fields                                                                                 ┃
┃   Priority    labels     —                                                                     ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
┌─ Tasks ─────────────────────────────────────────── 3 tasks · row 1 ────────────────────────────┐
...
 FILTER  ⏎ edit   j/k field   s mode   ctrl-l clear   f close   ? help                   q quit
```

Help overlay (`?`, centered modal, scrollable, `?`/`esc` closes):

```
        ┌─ Help ─ task mode ──────────────────────────────────────────────────┐
        │                                                                     │
        │  Move                            Select                             │
        │   j / ↓      down                 ␣      select, then down          │
        │   k / ↑      up                   a      select all                  │
        │   ctrl-d     page down            i      invert                      │
        │   ctrl-u     page up              x      clear                       │
        │   home/end   top / bottom         ctrl-x clear hidden                │
        │                                                                     │
        │  Group & sort                    Task                               │
        │   [ ]        prev/next section    ⏎      open in Asana              │
        │   { }        prev/next project    y      copy link                   │
        │   ,          group by project                                        │
        │   .          group by section    View                               │
        │   s          cycle sort           c      completed filter            │
        │                                   z      subtasks                    │
        │  Panes                            f      filter panel                │
        │   [ ] { } 0  size                 ← →    scroll columns (0/3)        │
        │                                                                     │
        │  Modes: p project   f filter   t task            r refresh   q quit  │
        │                                                    esc / ? to close  │
        └─────────────────────────────────────────────────────────────────────┘
```

Design rules:

- **Never rely on color alone.** Every color-coded state also carries a glyph or a modifier: overdue
  is red *and* prefixed, completed is dim *and* `✓`, cursor is accent-background *and* `▍`.
- **Show deviations, not defaults.** The status bar lists only settings that differ from their
  defaults, so a fresh session reads `14 tasks · row 3` instead of five `off` chips.
- **One line per row stays invariant.** The task body renders exactly one terminal line per
  `TaskRow`, including `SectionSpacer`, so selection indexing, `vertical_scroll`, and
  `ensure_selected_visible` keep working unchanged.
- **Fixed chrome height.** Header 1 + hint 1 + status 1; toggling help no longer reflows the layout.
- **Every glyph is width 1.** Validate the glyph set against `unicode-width` in a test; ship an ASCII
  fallback set for terminals with ambiguous-width fonts.

### Deliverables

Theme and glyph system (`src/ui/theme.rs`):

- a `Theme` with semantic roles, not raw colors: `border`, `border_focus`, `title`, `subtitle`,
  `muted`, `accent`, `cursor_bg`, `marker`, `star`, `hidden`, `ok`, `warn`, `danger`, `info`,
  `header_bg`, `key`
- default palette built from the 16 indexed ANSI colors so it inherits the user's terminal theme;
  an opt-in truecolor palette and a `mono` variant that resolves every role to modifiers only
- `GlyphSet` with `unicode` (default) and `ascii` variants covering: cursor `▍`/`>`, selected
  `●`/`*`, star `★ ☆`/`* -`, open `○`/`o`, done `✓`/`x`, subtask `↳`/`\`, rule `─`/`-`,
  section dots `·`/`.`, chip separator `·`/`|`, spinner `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`/`|/-\`
- `NO_COLOR` env var and a `--no-color` flag select the `mono` + `ascii` combination
- a `[theme]` config section: `variant = "ansi" | "truecolor" | "mono"`, `glyphs = "unicode" |
  "ascii"`, `accent = "cyan"`, `zebra = false`

Layout and chrome (`src/ui/layout.rs`, `src/ui/chrome.rs`):

- `layout.rs` owns all geometry: one function maps the frame `Rect` to named regions
  (`header`, `top_pane`, `task_pane`, `hint`, `status`, `overlay`) and returns the page size, so
  geometry math lives in one place instead of inline `Rect::new` arithmetic in `draw`
- `chrome.rs` renders the header bar, hint bar, status bar, and the pane `Block` factory that folds
  the pane title into the left of the top border and the pane status into the right of it
- the standalone title/status/search paragraphs are deleted; `format_mode_line` is deleted
- the focused pane gets `BorderType::Thick` plus `border_focus`; unfocused panes get
  `BorderType::Plain` plus `border`
- mode is shown as a single colored chip at the left of the status bar (project = blue,
  task = green, filter = magenta, project-search / filter-edit = yellow), matching the focused
  pane's border color

Hints and help overlay (`src/ui/hints.rs`, `src/ui/help_overlay.rs`):

- replace `hint_lines: Vec<String>` with a `Hint { keys: Vec<String>, label: String }` model;
  the hint bar renders keys in `key` style and labels in `muted`, and drops entries from the right
  when the terminal is too narrow rather than wrapping
- derive the key text for each hint from the resolved keymap rather than hardcoding it, so
  rebinding a key updates the hints and the overlay automatically
- `?` opens a centered modal overlay instead of expanding inline help; it keeps the existing
  `help_details_visible` toggle semantics (same key, same toggle, same state flag)
- the overlay groups bindings by intent (Move / Select / Group & sort / Task / View / Panes /
  Modes) in a two-column grid, scrolls with `j`/`k` when it doesn't fit, and closes on `?` or `esc`
- the overlay shows the active mode's bindings first, then the always-available ones; it introduces
  no new keybindings

Project list (`src/ui/project_list.rs`):

- `ProjectListView.rows` becomes styled rows: a 2-cell cursor/selection gutter, a star column, the
  name, and a right-aligned `hidden` chip only when hidden projects are being shown
- `[x]`/`[ ]` becomes `●`/blank in `accent`; `[*]` becomes `★` in `star` and unstarred `☆` in
  `muted`; `[hidden]` becomes a dim right-aligned chip and a dimmed name
- the assigned-to-me row is pinned at the top with an `info`-colored `◆` and a dim rule beneath it
- the search line renders only when a search is active or a query exists, as `/query` with a mode
  chip; an invalid regex renders the message in `danger`
- counts move to the right side of the pane border: `3 shown · 1 selected · 2 hidden`
- empty and loading states render a centered muted message instead of an empty bordered box

Task table (`src/ui/task_table.rs`, split with `src/ui/text.rs`):

- replace ` | ` separators with two spaces of padding plus a `muted` `│` rule drawn only between
  cells of task rows and the header row
- the header row gets a `header_bg` band, and the active sort column gets a `▲`/`▼` suffix
- `ProjectHeader` renders as `▌Name ─────` (accent bar, bold name, dim rule to the pane edge);
  `SectionHeader` renders indented as `Name ·····` in `subtitle`; `SectionSpacer` renders truly
  blank; none of the three draw column rules or empty cells
- a 2-cell gutter carries cursor (`▍`, `cursor_bg` on the row) and multi-select (`●`, `accent`);
  the `REVERSED | UNDERLINED` combination is retired
- `State` becomes a centered glyph column: `○` open, `✓ ok` done; completed rows render the title
  `CROSSED_OUT` and `muted`
- `Due` and `Start` render short and relative: `Today`, `Tomorrow`, `Mon`, `Jun 10`, and
  `2025-11-04` only when the year differs; `Due` is colored `danger` overdue, `warn` today,
  `accent` within three days, `muted` otherwise, and prefixed with `!` when overdue
- empty cells render a `muted` `—` instead of blank so the grid stays readable
- all columns truncate with `…`; per-column caps get minimum widths so headers like `Priority`
  are never cut mid-word
- the subtask indent marker moves out of `domain::task` and into the renderer as a `muted` `↳`,
  so the domain model stops carrying presentation strings
- optional `zebra` striping via a very low-contrast background on alternating task rows, off by
  default

Filter panel (`src/ui/filter_panel.rs`, extracted from `task_table.rs`):

- three aligned columns: label (bold when active), kind chip (`muted`), value
- fields are grouped under `muted` headings — Text, Dates, Labels, Custom fields — and a group is
  omitted when it has no fields
- an empty filter shows a `muted` `—`; an active filter shows a `●` in the gutter
- label filters render each value as a chip; the focused chip is `REVERSED` while editing
- editing shows a `▏` cursor after the query and switches the pane border to `warn`
- the active count moves to the pane border (`Filters ─ 1 active`); `(active)`/`(editing)` title
  suffixes are removed

Status summary (`src/domain/task.rs`, `src/app/task.rs`):

- `TaskTableSettings::summary` and `TaskState::filter_summary` return a structured
  `Vec<StatusChip>` instead of `grp p:on s:on; comp open; sub show; sort date; filters off`
- only non-default settings produce chips: `group: project+section`, `open only`, `no subtasks`,
  `sort: due ▲`, `1 filter`
- the row/count portion (`14 tasks · row 3 · 2 selected`) moves to the task pane border; the
  settings chips go to the right of the status bar

Loading, empty, and error states:

- loading renders a braille spinner plus target names, centered and `muted`
- empty renders a centered `muted` message with the single most useful key (`No tasks · r refresh`)
- errors render a `danger`-bordered pane with the message and a recovery hint instead of a bare
  `Error: …` line
- `OutOfDate` renders a `warn` chip in the pane border rather than a `stale: …` status string

Snapshot tests (`tests/ui_snapshot.rs`):

- a fixture app is rendered through `TestBackend` at 80, 120, and 200 columns in project, task,
  filter, filter-edit, and help-overlay states
- each render is compared against a committed plain-text snapshot under `tests/snapshots/`
- a `UPDATE_SNAPSHOTS=1` env var rewrites them, so the redesign work has a fast visual diff loop
- a test asserts every glyph in `GlyphSet::unicode` and `GlyphSet::ascii` has display width 1

### Target module layout

```
src/ui/
  mod.rs             re-exports
  theme.rs           Theme, palette variants, GlyphSet
  layout.rs          frame geometry, one function, returns named regions + page size
  chrome.rs          header bar, hint bar, status bar, pane block factory
  hints.rs           Hint model, keymap-derived key text, per-mode hint sets
  help_overlay.rs    centered modal help
  project_list.rs    styled project rows
  task_table.rs      styled task grid
  filter_panel.rs    styled filter fields (extracted from task_table.rs)
  text.rs            width, padding, truncation, span slicing (extracted from task_table.rs)
  runtime.rs         event loop only; no drawing beyond calling into the above
```

### Implementation notes

- do it in this order, keeping the build and tests green at each step:
  1. `theme.rs` + `text.rs` extraction + `[theme]` config, wired to borders and titles only
  2. `layout.rs` + `chrome.rs`: header/hint/status bars, pane blocks, delete the inline help block
     and the title/status/search paragraphs
  3. `hints.rs` + `help_overlay.rs`
  4. project list restyle
  5. task table restyle
  6. filter panel extraction and restyle
  7. status chips
  8. loading / empty / error states
  9. snapshot tests, then README and `developer.md` updates
- `runtime.rs` currently computes geometry inline with `Rect::new` and calls `render_task_table`
  twice per frame (once to read `hint_lines`, once to draw). Fold that into one `render_task_table`
  call per frame while moving geometry into `layout.rs`.
- the hint bar is a fixed one line and the help block is gone, which changes the body height and
  therefore `page_size` for `ctrl-d`/`ctrl-u`. That is an accepted consequence of the layout change;
  the paging *semantics* stay the same.
- keep the `TaskRow` set and ordering exactly as the domain produces it. Only `cells[0]`'s subtask
  prefix moves out of the domain; row kinds, counts, and indices are untouched.
- `Line::style` on a whole row (used today for cursor/selection) does not compose well with
  per-cell colors. Apply the cursor background per span while building the row instead.
- prefer indexed ANSI colors for the default palette. Truecolor values look wrong against light
  terminal themes, and this app has no way to detect background luminance.
- do not add keybindings in this milestone. The overlay, chips, and grouping are presentation only.

### Tests that will need updating

These assert on the current strings and must be rewritten against the new output:

- `src/ui/project_list.rs`: `renders_selection_and_star_state`, `renders_empty_state`,
  `renders_assigned_to_me_row_without_star_or_hidden_markers`,
  `renders_hidden_project_hint_and_marker`, `renders_contextual_selection_commands_…`,
  `renders_expanded_help_with_toggle_at_front`
- `src/ui/task_table.rs`: `renders_task_table_columns_and_rows`,
  `bolds_project_and_section_labels_without_bolding_separators`,
  `renders_loading_state_with_spinner_and_targets`, `renders_out_of_date_status_with_refresh_hint`,
  `renders_task_filter_and_sort_hints`, `renders_expanded_task_help_details`,
  `keeps_column_markers_aligned_for_varied_lengths`,
  `keeps_column_markers_aligned_when_scrolled_horizontally`, `scroll_hint_reflects_overflowing_view`,
  `truncates_the_title_column_with_an_ellipsis_when_it_exceeds_the_view`
- `src/ui/runtime.rs`: `moves_selection_until_quit` (asserts `"Projects"`),
  `project_panel_keeps_space_for_rows_when_tasks_are_visible` (test-only `project_panel_height`
  helper; delete it with the old geometry code)
- `src/domain/task.rs`: `renders_subtasks_with_an_indent_marker` (`"  L Child task"`) and the
  `cells[4] == "done"` assertion, both of which move to the render layer
- `src/app/task.rs`: the `filter_summary` assertions (`"comp done"`, `"grp p:off s:off"`,
  `"sub hide"`, `"sort title"`)
- `tests/task_view_integration.rs`: `"Task review"`, `"Tasks"` assertions
- `tests/project_list_keyboard_navigation.rs`: `"Projects"` assertion

Prefer converting these to assertions about structure (row kinds, styles, chip contents) rather than
exact rendered text, and let `tests/ui_snapshot.rs` own the full-text checks.

### Acceptance criteria

- no key, action, mode, filter, sort, or load behavior differs from before the milestone; every
  existing behavioral test passes unchanged or with only string-expectation edits
- there is exactly one `Theme` and one `GlyphSet`; no render function constructs a raw `Color`
- chrome is a fixed 3 lines (header, hint, status); pressing `?` does not reflow the layout
- `?` opens a centered, scrollable, grouped help overlay and closes on `?` or `esc`
- the hint bar is one line, shows keys resolved from the active keymap, and truncates from the right
- the focused pane is identifiable in a monochrome terminal by its border weight alone
- project rows show cursor, selection, star, and hidden state without ASCII bracket markers
- group header rows draw no column rules and no empty cells
- cursor row, multi-selected row, and a row that is both are visually distinct without using
  `REVERSED | UNDERLINED`
- overdue, due-today, and completed tasks are each identifiable without color
- every column truncates with `…`; no header is cut mid-word at 80 columns
- the status bar shows only settings that differ from their defaults
- loading, empty, and error states render a centered message rather than an empty bordered box
- `NO_COLOR=1` and `theme.variant = "mono"` both produce readable output with no color codes
- `tests/ui_snapshot.rs` covers project, task, filter, filter-edit, and help-overlay states at 80,
  120, and 200 columns, and a test proves every glyph has display width 1
- README documents the `[theme]` section and the current keybindings

### Notes from implementation

- The one-line-per-row invariant held up: `SectionSpacer` renders as a blank
  line rather than being dropped, so selection indices and `vertical_scroll`
  needed no changes at all.
- `KeyMap::keys_for` is the inverse of `action_for` and agrees with it, so a key
  bound globally but shadowed in a mode is never advertised for the global
  action. This is what makes keymap-derived hints trustworthy — it caught that
  `toggle_task_filters` is unbound in task mode (`f` there is `set_filter_mode`)
  and that the pane-resize keys are shadowed by the section/project jumps.
- Snapshot determinism needed two things: `TUISANA_TODAY` to pin relative dates,
  and sending keys in *batches* with an async-load drain between them. Sending
  a whole script at once made the result depend on whether the worker thread
  finished first.
- Dates are resolved in UTC because the standard library has no timezone
  database. For a date-only field the worst case is `Today` vs `Tomorrow` for a
  few hours around midnight west of UTC.

## Milestone 11.75: Date selection ✓

Goal:

- resolve dates in the local timezone, and make picking one obvious

Deliverables:

- one date module, `src/domain/date.rs`, replacing the two duplicated
  hand-rolled civil-calendar implementations (`src/ui/date.rs` and an inline copy
  in `src/app/task.rs`)
- `today()` resolves in the system timezone via `jiff`, so relative rendering,
  client-side date filters, and the server-side `due_on.after` / `due_on.before`
  push-down all agree on what day it is
- `TUISANA_TODAY` now pins the filter path too, not just rendering
- date parsing rejects dates that are shaped right but do not exist
  (`2026-02-31`, `2026-99-99`)
- editing a date filter opens a calendar overlay, pre-populated to the current
  month with today highlighted
- the overlay is a cursor picker: `h`/`l` day, `j`/`k` month, `t` today,
  `enter` pick, `esc` close, `d` clear
- flipping months lands on an edge of the month — the first going forward, the
  last going back — rather than carrying the old day number along
- typed text goes into the filter field, not a buffer inside the overlay, and the
  table refilters as it lands
- the field's text has a caret: `left`/`right` or `ctrl-b`/`ctrl-f` move it,
  `backspace` deletes at it, and `ctrl-a`/`ctrl-e` jump between a range's two ends
- the caret works on every filter field, not just date fields, so text values can
  be edited in the middle instead of only appended to
- the caret is drawn by reversing the character it sits on, so it costs no columns
  and the value does not shift as it moves
- text and grid follow each other both ways: `2026-09` shows September with no day
  highlighted, a year alone shows that January, and moving the highlight rewrites
  only the end of the range the caret is in
- a range shades the days between its two ends, with both ends picked out, and the
  overlay says which end is being edited
- an unparseable date query is reported on the overlay rather than silently
  filtering the table to zero rows
- `?` works inside the picker, so its keys are discoverable from the help overlay
  as well as the hint bar
- the `start..end` range form, which already worked, is documented
- all keybindings are configurable, under a new `calendar` mode

Also fixed here, found while using it:

- the loading indicator — spinner and word together — moved from the task pane's
  right-hand chips to the header's left, where it is the first thing on the line
- one filter row *and one table column* per custom-field *name* rather than per
  id. The same field usually exists separately in each project, so reviewing five
  projects produced five identically-labelled rows and five identical columns.
  Both group through `domain::group_custom_fields_by_name`, so they cannot
  disagree about how many fields there are
- a resize no longer freezes the screen. `CrosstermKeySource` looped on
  `event::read()` until a key arrived, so one resize or mouse event blocked the
  tick and left the frame broken until the user pressed something

Implementation notes:

- date logic moved from `src/ui/date.rs` to `src/domain/date.rs`; the UI module
  keeps only relative formatting and re-exports the types it renders
- `jiff` is used minimally: the current local date, and month-length arithmetic
  so `add_months` can clamp without hand-rolled leap-year rules
- date-only values are never converted between zones — a zoneless date has no
  zone to convert from. Only "what is today's date here" needs the system zone
- `CalendarState` lives in `src/app/calendar.rs`, independent of the filter
  editor, so Milestone 12's task-date editing can reuse it
- `Mode::Calendar` is excluded from `allows_any_fallback`, the same mechanism
  `project_search` and `filter_edit` use to let unbound keys type
- the filter field is the one text buffer: the picker mirrors its query into the
  field on every change, and the field is what the panel draws and the table
  filters on. There is no second copy of the value to get out of step
- `TaskFilterPanelEntry` carries a caret position rather than an is-editing flag,
  because the picker's caret can sit anywhere in the text
- `CivilDate::days_in_month` is hand-rolled rather than delegated to `jiff`, whose
  `civil::date` panics on an out-of-range year — and a half-typed year reaches it.
  `jiff` is used only for the system timezone, which is the part std cannot do
- `CivilDate::new` caps the year at four digits, so a fat-fingered `99999` is not
  mistaken for a date the user meant
- an empty end of a range falls back to the *other* end, so filling in
  `2026-09-01..` starts next to the start date instead of jumping to today
- the month label is drawn inside the overlay rather than in its border: the box
  is 23 columns and the border truncates chips from the left, which turned
  "Aug 2026" into "6"
- `ctrl-b`/`ctrl-f` move the caret and `ctrl-a`/`ctrl-e` jump between range ends,
  following readline. The original sketch used `ctrl-b` for both, which one key
  cannot do
- `search_fuzzy` moved from `ctrl-f` to `ctrl-z` so `ctrl-b`/`ctrl-f` mean caret
  motion in every mode that edits text, rather than only in the calendar
- `classify` maps every terminal event to an `InputEvent` so the loop can never
  block on one it does not act on
- `help_overlay`'s private `centered` was promoted to `ui::layout::centered` and
  is now shared by both overlays

Acceptance criteria:

- there is exactly one civil-calendar implementation
- `today` resolves in the system timezone, and `TUISANA_TODAY` overrides it
  everywhere
- a date filter can be set entirely from the calendar, without typing
- a date range can be built and both ends adjusted without leaving the picker
- flipping forward lands on the first of the month and back on the last
- half-typed text moves the grid without highlighting a day
- tests cover the date module, the picker's state machine, the filter predicate,
  the push-down bounds, and the overlay's layout, plus end-to-end key-driven
  pick and cancel flows and snapshots at 80/120/200 columns

## Milestone 12: Edit tasks

Goal:

- make it possible to create, edit, and move tasks

Deliverables:

- users can toggle completion of a task
- user can create new tasks that use default values based on the task they are above
  - if the task they are above is a subtask, the new task is a subtask
  - the dates from the task above are used
- user can change the columns of a task; editing follows the same pattern as filter editing
  - dates are entered in YYYY-MM-DD, MM-DD, or keyword, and are translated to YYYY-MM-DD
  - label options can be selected using j/k and deleted using d
  - titles are typed; backspace and cursor motions are possible; basic vim normal mode
    support for editing text (h/l, d, y, c, b, w, $, 0, C, D all work as in vim)
- users can increase the subtask level: the task becomes a subtask of the task directly above it (default keybinding `>`)
- users can decrease the subtask level of a task (default keybinding `<`)
- when subtask adjustment (`<`/`>`) is a bulk action, it adjusts subtask level based on
  the task directly above the first item selected
- when the sort view is "natural", users can move tasks up and down in the list (`shift+j`, `shift+k`)
  and into or out of sections (`m [` / `m ]`, `m {`, `m }`)
- all editing operations can be applied to multiple tasks at once using the selection mechanism from Milestone 10
- all keybindings are configurable

Implementation notes:

- follow the same edit model as the filter editor for consistent field editing UX
- use the selection mechanism from Milestone 10 for bulk edits
- prefer optimistic local updates when safe; reconcile state after server confirmation
- model edits as domain mutations rather than UI-specific actions

Acceptance criteria:

- task completion can be toggled
- new tasks can be created with sensible defaults from the adjacent task
- task fields can be edited inline following the same UX as filter editing
- subtask level can be increased and decreased
- tasks can be reordered in natural sort view
- all editing operations apply to the current selection
- all keybindings are configurable
- tests cover field editing, task creation, subtask adjustment, and bulk operations

## Milestone 13: Hardening and polish

Goal:

- make the app reliable and pleasant to use

Deliverables:

- error handling and recovery
- loading and empty states
- help overlay
- persisted default project sets and views
- optional caching if startup latency becomes a concern
- browser-based Asana login using the OAuth authorization code flow with PKCE where practical
- token persistence and refresh handling for the browser login flow
- browser-open and callback handling for login from the terminal

Implementation notes:

- keep errors user-facing and actionable
- avoid silent failures
- make the help screen discoverable from the main views
- keep the browser login flow behind the existing auth/client boundary so it remains testable
- keep the PAT-based path only as a fallback if it remains useful during development
- prefer a loopback or local callback flow so the user can authenticate from the terminal without copy-pasting long codes

Acceptance criteria:

- common failure modes are handled gracefully
- help is accessible from the UI
- defaults persist across runs
- the app can prompt the user to open a browser-based login flow from the terminal
- the user can complete Asana auth without manually creating or pasting a PAT
- the app can exchange the authorization code for usable API tokens
- tokens persist across runs or a clear re-auth flow exists
- tests cover auth URL construction, callback handling, and token exchange seams

## Testing Strategy

Unit tests should focus on:

- config parsing
- keymap parsing and binding resolution
- project ordering
- task table column mapping
- filter and sort logic
- state transitions
- edit command parsing and validation

Integration tests should focus on:

- loading and ordering projects
- loading tasks into a table model
- multi-project aggregation
- filter and sort workflows
- edit workflows against the fake backend

Test guidance:

- keep the HTTP client thin so it can be mocked
- avoid integration tests that depend on live Asana credentials
- prefer deterministic fake data fixtures

## Dependency Guidance

Recommended crates:

- `ratatui` for drawing the terminal UI
- `crossterm` for terminal events and backend integration
- `serde` for serialization and deserialization
- `toml` for config parsing
- `reqwest` for HTTP if using an async client, or a lighter blocking client if simplicity matters more
- `thiserror` for structured errors
- `tokio` only if the Asana client is async

## Execution Order

Implement in this order:

1. project skeleton and config
2. project list
3. Asana API setup
4. navigation and key bindings
5. project visibility management
6. multi-select project management
7. task review table
8. filtering and sorting
9. editing
10. hardening and polish, including browser-based Asana login

## Definition of Done

The project is done when:

- projects can be listed with starred items first
- a project or project set can be selected
- tasks can be reviewed in a table with useful fields
- tasks can be navigated, filtered, and sorted with configurable vim-like shortcuts
- tasks can be edited for the specified fields and relationships
- tests cover the key logic and a few end-to-end flows
- the code is organized into small, testable modules
