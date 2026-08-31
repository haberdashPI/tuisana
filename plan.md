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
  editor, so Milestone 13's task-date editing can reuse it
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

## Milestone 12: Gantt chart view ✓

Delivered. Five deliberate deviations from the plan below.

1. **`TimelineView` lives in `src/domain/gantt.rs`, not `src/app/gantt.rs`.**
   `Timeline::resolve` needs it, and a domain function taking an app type would
   have the layering backwards. `app/gantt.rs` owns a mutable one and all the
   scroll/zoom verbs.
2. **Reversed dates draw the span between them rather than a milestone at due.**
   The bar then covers both dates the API actually sent, and the table's own
   columns still show which way round they are. A milestone would have hidden
   the start date and looked identical to "no start date".
3. **Month gridlines are drawn on group header rows only, not on empty task
   tracks.** One mark per month on every row is noise on exactly the rows the
   reader is following, and the heading rows have nothing else to say in the
   chart region.
4. **`month_starts` returns every boundary; the axis drops the labels that
   collide.** Gridlines want all of them and only the renderer knows how wide a
   label is. It also drops a label that would collide with today's marker —
   "Se▼" reads as neither a month nor a marker.
5. **No `gantt-narrow` snapshot.** The chart is only dropped entirely below
   about 32 columns, which is a screen of mostly border; a unit test covers it.
   The snapshot at 80 shows the more interesting degradation: the columns cap
   biting, clipped columns still reachable by scrolling, and the legend's
   `+N more`.

Also fixed here, found while building it:

- a day is a *range* of columns, not a point, once the chart is wider than the
  window is long. Treating it as a point left the last column permanently
  unpainted — at 40 columns over 30 days a full-window bar stopped at 38
- the split measured the columns' appetite from widths already stretched to
  fill the pane, so it handed them 85 cells to draw 66 in and the chart lost 19
  to a gap. `natural_widths` now answers "what do these columns want"
  separately from "what do they get"
- with the chart on, the title column has to fill its region *exactly* rather
  than grow to its natural length: at 80 columns a 45-character title took the
  space the split had reserved for the assignee column and pushed it off screen
- a title has no natural maximum, so the columns region is capped at a share of
  the pane. Without it one long task name squeezed the chart to its floor
- `tests/ui_snapshot.rs`'s fixture had three assignees and one shared start
  date, which cannot demonstrate a palette running out or a timeline worth
  scrolling. Widened to eleven tasks over eight assignees, in its own commit

Notes from implementation:

- the one-line-per-row invariant did all the work it was supposed to. Appending
  the chart's spans to the row's own `Line` meant the cursor background, zebra
  striping, multi-select markers, `vertical_scroll`, and `ensure_selected_visible`
  needed no changes at all — the chart is invisible to every one of them
- exhaustive matching on `Mode` and `Action` turned "add a mode" into a compiler
  worklist: six arms for `Mode::Gantt`, five for `Mode::GanttOrder`, each one a
  line. Nothing was silently missed
- `Fit` being a distinct state rather than "a window that happens to match the
  data" is what stops a filter change from stranding a user who scrolled
  somewhere deliberately. It also makes "a fitted window can never clip a bar" a
  property worth testing in both directions
- the hint bar sheds from the right, so adding `g gantt` to task mode pushed
  `? help` off at 120 columns. That is the design working, but it does mean each
  new binding costs a visible slot at common widths

Goal:

- draw a Gantt chart to the right of the task list, sharing one line per task row
  with the table so the cursor, selection, scroll, and grouping all keep working
- give the chart its own `gantt` bind mode holding every control it needs:
  scrolling and zooming the timeline, trading table columns for chart width, and
  choosing what colors the bars
- color the bars by a chosen dimension — assignee, section, task state, or one of
  the enumerated custom fields — six unique colors and a neutral color for
  everything past the sixth
- order the colors from a modal dialog that lists the dimension's values, shows
  which of them are getting a color, and moves the selected one to the top, up,
  down, or to the bottom
- **the keys are the interface; `[gantt]` in the config is where their result is
  written.** Committing the dialog persists the order, the way toggling a project's
  star already persists. Nothing has to be hand-written into TOML to use any of
  this.
- **behavior must not change when the chart is off.** The chart is off by default;
  with it off, every rendered frame is byte-identical to today's.

### Target design

The task pane's interior splits into a columns region and a chart region,
separated by the same three cells (space, rule, space) that already separate two
table columns. The pane's column-header line carries the time axis; the pane's
bottom border carries the color legend.

```
 TUISANA  ▸ 1 project ▸ 4 tasks                             gantt · color assignee · columns 2/7
┌─ Projects ────────────────────────────────────────────────── 3 shown · 1 selected · 2 hidden ┐
│  ● ★ Northwind BTX 4412                                                                      │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
┏━ Tasks ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 4 tasks · row 1 ┓
┃   Task                               │ Assignee      │ Aug          Sep      ▼   Oct         ┃
┃▌Northwind BTX 4412 ───────────────────────────────── │ ·            ·               ·        ┃
┃   Study Kit Design ································· │ ·            ·               ·        ┃
┃▍  Review shipment requirements doc   │ Morgan Ellis  │    ██████             ┊               ┃
┃ ● Ship release candidate to the stu… │ Alex Chen     │       ████████████████████            ┃
┃   ↳ Confirm carrier pickup window    │ —             │                       ┊  ◆            ┃
┃   Close out packaging vendor contra… │ Alex Chen     │   ▒▒▒▒▒               ┊               ┃
┗━ Alex Chen █  Morgan Ellis █  other ▒ ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
 GANTT  h/l scroll  -/= zoom  z fit  t today  </> columns  c color  ⏎ order    r refresh  q quit
```

The color dialog (`⏎` from gantt mode), a centered modal over the panes with the
chart live-updating behind it:

```
        ┌─ Colors ─ by assignee ──────────────────────── 8 values ─┐
        │                                                          │
        │   1  █  Alex Chen                               12 tasks │
        │   2  █  Morgan Ellis                             8 tasks │
        │▍  3  █  Pat Lee                                  5 tasks │
        │   4  █  Robin Fox                                3 tasks │
        │   5  █  Sam Okafor                               2 tasks │
        │   6  █  Dana Ruiz                                2 tasks │
        │ ───────────────────── neutral below ──────────────────── │
        │   7  ▒  Kim Alvarez                               1 task │
        │      ▒  (no assignee)                           14 tasks │
        │                                                          │
        │   ctrl-k / ctrl-j  move    t / b  to top / bottom        │
        │   c  color by      ⏎  save        esc  cancel            │
        └──────────────────────────────────────────────────────────┘
```

The rule after the sixth entry is the whole point of the dialog: it is where the
palette runs out, it moves as you reorder, and everything under it is drawn
neutral. `(no assignee)` sits below it unnumbered and cannot be moved — an empty
value is always neutral and never spends one of the six.

The mock cannot show color, which is most of what distinguishes two bars. Every
slotted bar draws the same `█`; the legend in the bottom border is what names the
mapping, and `▒` marks a value that fell past the sixth slot. Reading it:

- `▼` on the axis is today's column; `┊` is today's line drawn through rows whose
  track is empty at that column. A bar wins over the line.
- `·` marks the first column of each month. It is drawn on group header rows,
  where the chart region has nothing else to say; task rows stay clean.
- `◆` is a milestone: a task with a due date but no start date.
- `‹` and `›` at the chart's edges mark a bar that continues past the window,
  which can only happen once the timeline has been scrolled or zoomed.
- Group header rows draw their rule up to the divider and month gridlines past
  it, so the chart region stays a chart even on a heading line.

Design rules carried forward from Milestone 11.5:

- **One line per row stays invariant.** The chart is appended as spans to the same
  `Line` the table row already builds, not drawn as a second widget. Cursor
  background, zebra striping, and selection markers therefore extend across the
  chart for free, and the selection index, `vertical_scroll`, and
  `ensure_selected_visible` need no changes at all.
- **Never rely on color alone.** In the `mono` variant the bar glyph varies per
  slot instead of the color, and the legend shows each slot's glyph beside its
  value. In color variants every slotted bar is `█` and the slot is the color.
- **Show deviations, not defaults.** The chart is off by default and contributes
  no status chips until it is turned on.
- **Fixed chrome height.** The legend lives in the pane's bottom border and the
  axis in the existing column-header line, so turning the chart on costs zero
  body lines and reflows nothing. The dialog is a modal drawn over the panes, so
  it does not move them either.
- **Every binding is configurable.** The two new modes take their keys from the
  keymap like every other mode, and the hint bar and help overlay resolve their
  key names from it, so a rebind is reflected everywhere without touching a
  render function.

Explicitly out of scope, to be a later milestone if wanted: editing task dates
from the chart (dragging or nudging a bar), dependency arrows between bars, and
anything tuned specifically for the assigned-to-me view — the chart works there,
but a cross-project timeline has its own problems that are not being solved here.

### Deliverables

Two new bind modes (`src/config/mod.rs`, `src/app.rs`):

- `Mode::Gantt` — the chart has focus. It is a sibling of `Mode::Task` the way
  `Mode::Filter` is: the same task pane, the same rows, the same cursor, but the
  mode-specific keys drive the chart instead of the tasks. `j`/`k` still move the
  task cursor, because they are global bindings and gantt mode allows the `Any`
  fallback
- `Mode::GanttOrder` — the color dialog is open. It also allows the `Any`
  fallback, so `?`, `r`, and `q` keep working; its own `j`/`k` are bound
  explicitly and shadow the globals, which is how the filter panel already
  reuses `MoveUp`/`MoveDown` for a different list
- the plumbing each new mode needs, all of it a one-line arm:
  `Mode::label`, `focused_pane` (both → `FocusedPane::Task`), `help_visible`
  (both → `app.tasks.help_details_visible()`), and `Theme::mode_color`
  (`Gantt` → cyan, `GanttOrder` → yellow, matching the other "you are editing
  something" modes)
- `handle_action`'s pane dispatch needs no change: neither mode is
  `Project`/`ProjectSearch`, so actions already route to `self.tasks`
- `App::set_gantt_mode` follows `set_filter_mode`: `prepare_mode_switch`, make
  the task pane visible, make the chart visible, close the filter panel if it is
  open. `set_gantt_order_mode` additionally opens the dialog state

Actions and default bindings (`src/input/mod.rs`, `src/config/mod.rs`):

| mode | key | command | effect |
| --- | --- | --- | --- |
| task | `g` | `set_gantt_mode` | draw the chart and take its controls |
| gantt | `esc` | `set_task_mode` | back to task mode, chart stays |
| gantt | `g` | `toggle_gantt` | hide the chart and go back to task mode |
| gantt | `h` / `l` | `gantt_scroll_left` / `gantt_scroll_right` | scroll a quarter window |
| gantt | `-` / `=`, `+` | `gantt_zoom_out` / `gantt_zoom_in` | step the zoom ladder |
| gantt | `z` | `gantt_zoom_fit` | back to fitting the loaded tasks |
| gantt | `t` | `gantt_today` | center the window on today |
| gantt | `<` / `>` | `gantt_remove_column` / `gantt_add_column` | move the divider |
| gantt | `c` | `cycle_gantt_color_key` | next colorable dimension |
| gantt | `enter` | `gantt_open_order` | open the color dialog |
| gantt-order | `j` / `k` | `move_down` / `move_up` | move the dialog cursor |
| gantt-order | `ctrl-j` / `ctrl-k` | `gantt_order_move_down` / `gantt_order_move_up` | move the value one place |
| gantt-order | `t` / `b` | `gantt_order_move_top` / `gantt_order_move_bottom` | move it to an end |
| gantt-order | `c` | `cycle_gantt_color_key` | switch dimension, rebuild the list |
| gantt-order | `enter` | `gantt_order_commit` | keep the order, persist, close |
| gantt-order | `esc` | `gantt_order_cancel` | restore the order as it was, close |

- `Mode::allows_any_fallback` needs no new arm: it excludes only the three modes
  that type text, so both new modes fall back to the globals by default, which is
  what they want
- every shadowing choice below rests on `KeyMap::action_for` trying
  `(mode, key)` before `(Any, key)`, and on `keys_for` honoring the same
  precedence — so binding `t`, `c`, `z`, `h`, and `l` per-mode both works and is
  reported correctly by the hint bar and the overlay
- `t` means "today" in gantt mode and "to the top" in the dialog rather than
  `set_task_mode`, which is what `esc` is for. `t` for today already means that
  in `Mode::Calendar`, so the picker and the chart agree
- `h` and `l` are shadowed in gantt mode. They are bound in project mode
  (`toggle_hidden_selected`) and filter-edit mode (label motion), not globally, so
  nothing is lost by taking them here
- `KeyBinding::from_crossterm_event` lowercases every char, so `G` and `g` are
  the same key and shift+letter is not an available namespace. That is why these
  are punctuation and `ctrl-` pairs. (It also means Milestone 13's sketched
  `shift+j` / `shift+k` for reordering tasks cannot work as written and will need
  different keys.)
- `left`/`right` stay bound globally to column scrolling rather than being
  shadowed to timeline scrolling in gantt mode. Two different horizontal scrolls
  on one pane is confusing enough without the arrow keys silently changing
  meaning
- none of the gantt actions belong in `is_task_view_action`. That list exists so
  the *task* pane can be driven while the project list has focus; the chart has
  its own mode, and adding them would put chart keys into project mode

Timeline window (`src/app/gantt.rs`, new):

- `TimelineView` is the scroll/zoom state, held beside `CalendarState` in
  `src/app/gantt.rs` for the same reason the calendar lives outside the filter
  editor: it is a viewport, not a property of the data
  ```
  enum TimelineView {
      Fit,                                        // the default
      Window { start: CivilDate, days: u32 },     // scrolled or zoomed
  }
  ```
- `ZOOM_LADDER: [u32; 8] = [14, 30, 60, 90, 180, 365, 730, 1825]` days. Zooming
  from `Fit` enters the ladder at the step nearest the fitted span, so the first
  `-` or `=` does not jump somewhere unrelated to what is on screen
- zoom holds the date at the left margin fixed, so time is added and removed at
  the right; scroll moves `start` by `max(1, days / 4)`; `gantt_today` brings
  today to that same margin at the current span; `gantt_zoom_fit` returns to
  `Fit`
- scrolling and zooming never refetch and never refilter. They move a viewport
  over the rows already in the table — the milestone adds no new queries
- once the view is a `Window`, bars can fall outside it, so `TrackShape` gains
  clipped ends and the renderer draws `‹` / `›` at the chart edge. In `Fit` this
  is unreachable, which is worth a test in both directions

Color order dialog (`src/app/gantt.rs`, `src/ui/gantt_order.rs`, both new):

- `GanttOrderState { key: GanttColorKey, values: Vec<OrderEntry>, selected: usize, restore: Option<Vec<String>> }`
  where `OrderEntry { value: String, task_count: usize, movable: bool }`
- opening builds the list from the effective order for the current key — the
  configured values that are present, then the rest alphabetically — so the
  dialog opens showing exactly what the chart is already doing
- the empty value renders last, unnumbered, with `movable: false`. It is always
  neutral, so letting it be dragged above the rule would be a lie
- `move_up` / `move_down` / `move_to_top` / `move_to_bottom` operate on the
  selected entry and carry the cursor with it, so holding `ctrl-k` walks a value
  up the list rather than leaving the cursor behind
- every move updates the live order immediately, so the chart behind the modal
  recolors as the user works. `restore` holds the order captured on open and
  `esc` puts it back
- `cycle_gantt_color_key` inside the dialog commits nothing, swaps the key, and
  rebuilds the list, so comparing two dimensions costs one keypress
- `src/ui/gantt_order.rs` renders it exactly as the calendar overlay is rendered:
  `layout::centered`, `Clear`, `chrome::pane_block`, a `Paragraph`. Rows are
  slot number, slot glyph in its slot style, value, right-aligned task count; the
  rule after slot six is drawn with the theme's `rule` glyph and a `muted`
  `neutral below` label; the two key lines at the bottom resolve their key names
  from the keymap through `hints::key_column_spans`, like the help overlay
- `runtime.rs` draws one overlay at a time, currently help then calendar. The
  order becomes help, then the dialog, then the calendar; the dialog and the
  calendar cannot both be open, since one is reached from gantt mode and the
  other from filter mode

Domain logic (`src/domain/gantt.rs`, new):

- `GanttColorKey { Assignee, Section, State, Field(String) }` with `FromStr` and
  `Display` that round-trip the config spellings above. `Field` carries the field
  *name*, not an id, matching `group_custom_fields_by_name` and the filter
  panel's `custom:<name>` keys — the same field has a different id in each project
- `ColorSlot { Indexed(usize), Neutral }` with `PALETTE_SLOTS: usize = 6`
- `assign_slots(values: &[String], configured: &[String]) -> HashMap<String, ColorSlot>`:
  1. configured values that actually occur, in configured order
  2. remaining values, case-insensitively alphabetical, then exact, as a tiebreak
  3. the first six get `Indexed(0..6)`; the rest get `Neutral`
  4. an empty or whitespace-only value is always `Neutral` and never consumes a
     slot, so "unassigned" cannot eat a color
- `Timeline { start: CivilDate, end: CivilDate, width: usize }`:
  - `fit(dates, width) -> Option<Timeline>` takes the union of every date on every
    task row currently in the table, snaps the start down to the first of its
    month and the end up to the last of its, and returns `None` when no row has a
    date
  - `windowed(start, days, width) -> Timeline` builds the scrolled/zoomed case,
    with no snapping — scrolling by a quarter window has to land where it says it
    lands, not on the nearest month boundary
  - one `resolve(view: &TimelineView, dates, width) -> Option<Timeline>` picks
    between the two, so no caller branches on the view
  - `column_for(date) -> usize` maps proportionally across the full width, so the
    window always fills the chart exactly and the same code handles a two-week
    span and a two-year one
  - `month_starts() -> Vec<(usize, &'static str)>` for the axis and gridlines,
    dropping a label that would overlap the previous one
  - in `Fit`, today is *not* forced into the window. Tasks from 2024 viewed in
    2026 would otherwise squash into three columns to make room for a marker —
    and now there is a key (`t`) for going and looking at today instead
- `GanttTrack { slot: ColorSlot, shape: TrackShape, completed: bool }` and
  ```
  enum TrackShape {
      Bar { from: usize, to: usize, clipped_left: bool, clipped_right: bool },
      Milestone { at: usize },
      OffWindow { left: bool },   // nothing of this task is in view
      Empty,                      // this task has no dates at all
  }
  ```
  - start and due, `start <= due` — a bar, at least one cell wide
  - due only — a milestone at due
  - start only — a one-cell bar at start; a task that has begun with no deadline
    is not a milestone, and inventing an end date would be a lie
  - start after due (bad data) — a milestone at due
  - neither — `Empty`
  - any of the above falling wholly outside a scrolled window — `OffWindow`,
    drawn as a lone `‹` or `›` at the near edge so the row does not read as
    "this task has no dates"
  - `OffWindow` and the `clipped_*` flags are unreachable in `Fit`, by
    construction, and tested in both directions
- `GanttModel { timeline: Option<Timeline>, today_column: Option<usize>, tracks: Vec<GanttTrack>, legend: Vec<(String, ColorSlot)> }`
  built by one `build(model: &TaskTableModel, key: &GanttColorKey, order: &[String], view: &TimelineView, width: usize, today: CivilDate)`,
  with `tracks` parallel to `model.rows` so the renderer indexes them by row
- `distinct_values(model, key) -> Vec<(String, usize)>` — each value with the
  number of task rows carrying it. The dialog needs the counts and the chart
  needs the values; one function produces both so they cannot disagree about
  what values exist
- resolving a row's color value: `Assignee` and `State` read the cell at the role
  index the table already uses, `Section` reads `TaskRow.section`, and
  `Field(name)` finds the column by label in `TaskTableModel::columns`. `Section`
  is deliberately not a filter-panel key, so the color keys are their own small
  vocabulary rather than an overload of the filter keys

Typed dates on the row model (`src/domain/task.rs`):

- `TaskRow` gains `pub start: Option<CivilDate>` and `pub due: Option<CivilDate>`,
  populated in `build_rows` from the record and left `None` on header and spacer
  rows. The display cells stay strings; the chart needs arithmetic, and
  re-parsing `sanitize_display_text` output in the renderer would be fragile

Theme (`src/ui/theme.rs`):

- `categorical: [Style; 6]` and `categorical_neutral: Style`. Order the palette so
  the semantically loaded colors come last: blue, magenta, cyan, green, yellow,
  red — a bar should not read as "overdue" just for being first in the legend
- `Theme::bar_glyph(slot) -> &'static str`: `█` for every slot in the color
  variants; in `mono`, a distinct texture per slot (`#`, `=`, `+`, `*`, `o`, `~`)
  so six bars stay distinguishable with no color at all
- `GlyphSet` gains `bar`, `bar_neutral` (`▒` / `=`), `milestone` (`◆` / `<`),
  `today_line` (`┊` / `:`), `gridline` (`·` / `.`), and `clip_left` / `clip_right`
  (`‹` `›` / `<` `>`), all width 1 and all added to the existing glyph-width test

Chart rendering (`src/ui/gantt.rs`, new):

- `axis_spans(model, theme, width)` — month labels and the `▼` today marker
- `track_spans(track, model, theme, width)` — one row's cells: bar over today
  line over blank, in that precedence, with a clipped end or an `OffWindow`
  marker drawn in the edge cell. Task rows draw no month gridlines; one `·` per
  month on every row is noise on the rows that matter most
- `gridline_spans(model, theme, width)` — what a group header row draws past the
  divider
- `legend_line(model, theme, width)` — the bottom-border legend: each value
  prefixed by its slot glyph in its slot style, `other` appended when any value
  fell through to neutral, and `+N more` when the border is too narrow. Only built
  when the chart is drawn
- every function returns `Vec<Span<'static>>` of exactly `width` cells, so the
  caller can concatenate without measuring

Table integration (`src/ui/task_table.rs`):

- `TaskTableView` gains `visible_columns: usize`, `chart: Option<GanttModel>`, and
  `chart_width: usize`
- `render_task_table` takes the pane's whole inner width and does the split
  itself, since it is the only place that knows both the natural column widths and
  the chart state:
  - the columns budget is the natural width of the first `visible_columns`
    columns, capped so the chart keeps `MIN_CHART_WIDTH = 24`
  - the chart gets the remainder less `COLUMN_SEPARATOR_WIDTH`, reusing the
    constant so the divider matches the rules between columns
  - if even one column plus the minimum chart does not fit, the chart is dropped
    for that frame and a `Chip::toned("gantt too narrow", Tone::Warn)` says so
  - horizontal scroll keeps working *inside* the columns region, unchanged, so a
    column clipped by the narrower budget is still reachable and the existing
    `columns n/m` chip still reports it
- `column_widths` and `total_width` are measured over the visible columns only;
  `TITLE_SHARE` applies to the columns budget rather than the pane, so the title
  shrinks proportionally instead of pushing the chart out
- `task_header_line`, `task_line`, and `group_header_line` each append the divider
  and their chart spans; `TaskRowKind::ProjectSeparator` and `SectionSpacer` stay
  entirely blank, chart included, because a spacer exists to give the eye a break
- when `chart` is `None`, every one of these paths is the code that runs today —
  `visible_columns` is not consulted, so nothing about the default view moves
- `settings_chips` gains `gantt`, `color <key>`, and `columns n/m`, and only when
  the chart is on, per the "show deviations" rule. A timeline that is no longer
  fitted adds a fourth, `window Aug 1 – Oct 31`, because a scrolled window is the
  one chart state you cannot infer from the axis alone — the axis looks the same
  whether you fitted there or scrolled there

Task state (`src/app/task.rs`):

- `TaskViewState` gains one field, `gantt: GanttViewState`, rather than five loose
  ones. It holds `visible`, `visible_columns`, `color_key`, `order`, `timeline`,
  and `dialog: Option<GanttOrderState>`, and is seeded from `[gantt]` by
  `TaskState::apply_gantt_config(&GanttConfig)` called from `App::new`
- `TaskState` exposes the verbs the actions need — `toggle_gantt`,
  `add_visible_column` / `remove_visible_column` (clamped to `1..=columns.len()`),
  `cycle_gantt_color_key`, the four timeline verbs, and the dialog's open, move,
  commit, and cancel — and nothing else. `GanttViewState` stays private, as
  `TaskFilterEditorState` does
- `available_color_keys()` reuses the filter editor's existing work: the
  colorable dimensions are `Assignee`, `Section`, `State`, plus every filter field
  whose kind is already `TaskFieldFilterKind::Labels`. There is no second place
  that decides what counts as an enumerated field
- `apply_action` intercepts `MoveUp` / `MoveDown` when the dialog is open and
  routes them to the dialog cursor, which is exactly the shape of the existing
  `if self.view.filter_editor.visible` block at the top of that function

Persistence and config (`src/config/mod.rs`, `src/app.rs`):

- `[gantt]` is the serialization of what the two modes set, not the interface to
  them. It is optional and, when untouched, omitted from a rewritten config
  exactly as `[theme]` is:

  ```toml
  [gantt]
  visible = false          # true to start with the chart already drawn
  columns = 2              # table columns kept visible when the chart is drawn
  color_by = "assignee"    # "assignee" | "section" | "state" | "field:<Name>"

  [gantt.order]
  assignee = ["Alex Chen", "Morgan Ellis"]
  section = ["Study Kit Design", "Shipment"]
  "field:Priority" = ["High", "Medium", "Low"]
  ```

- `GanttConfig { visible: bool, columns: usize, color_by: String, order: BTreeMap<String, Vec<String>> }`
  with `is_default()` and `#[serde(skip_serializing_if)]`, because
  `save_to_source_path` rewrites the whole file whenever project visibility
  changes and must not start emitting a `[gantt]` table nobody asked for.
  `BTreeMap` rather than `HashMap` so a rewritten file has a stable key order and
  does not churn in version control
- validation, following `theme.accent`'s precedent of a validated string:
  `color_by` must parse as a color key, every key of `[gantt.order]` must parse as
  one too (so a typo is a config error rather than silently dead), and `columns`
  must be at least 1
- committing the dialog writes `color_by` and that dimension's `order`, then calls
  the same save path project starring already uses — generalized from
  `persist_project_visibility` into a `persist_config`, since there are now two
  callers. Hand-ordering a team's assignees is real work and losing it on exit
  would be worse than the cost of a file write
- what gets written is the list as displayed, plus any previously configured
  values for that dimension that are not currently present, appended in their
  prior relative order. A value that lives in a project you did not load must not
  be silently dropped from your config
- what is *not* persisted: `visible`, `columns`, and the timeline window. Those
  are view state of the same kind as sort and grouping, which have never
  persisted; `[gantt] visible` and `columns` set a starting point the same way
  `[theme]` does

Hints and help (`src/ui/hints.rs`, `src/ui/help_overlay.rs`):

- `hints_for` gains a `Mode::Gantt` arm (scroll, zoom, fit, today, columns, color,
  order, close) and a `Mode::GanttOrder` arm (move, to top/bottom, color by, save,
  cancel). Task mode gains one hint, `g gantt`
- `HintContext` gains `timeline_windowed`, so `z fit` is offered only once
  scrolling or zooming has actually moved the window off `Fit`
- `groups_for` gains `Mode::Gantt` and `Mode::GanttOrder` arms. The gantt group
  also carries literal entries naming the glyphs — `bar`, `milestone`, `today`,
  `off-window` — since a legend explains the colors but nothing on screen explains
  `◆` or `‹`

Runtime (`src/ui/runtime.rs`):

- `render_task_table` is called with `pane_inner(area).width` instead of a
  pre-subtracted columns width
- `render_task_pane` attaches the legend with `title_bottom`, the pattern the
  project pane already uses for its search footer
- the overlay precedence at the end of `draw` becomes help, then the color
  dialog, then the calendar. Help still wins, for the reason it already does: `?`
  is reachable from inside a modal, so asking for help has to actually show it

Docs:

- README: a "Gantt chart" section covering the two modes and their bindings, the
  color dialog and what the rule after the sixth value means, and how to read the
  glyphs; the `[gantt]` block documented as what the dialog writes rather than as
  the way to set any of it; and every new binding in the key table
- `developer.md`: `domain/gantt.rs`, `app/gantt.rs`, `ui/gantt.rs`, and
  `ui/gantt_order.rs` in the reading order, a note that the chart is spans
  appended to the table's lines rather than a second widget, and `Mode::Gantt` /
  `Mode::GanttOrder` in the mode list

### Implementation notes

- do it in this order, keeping the build and tests green at each step:
  1. `domain/gantt.rs` with `GanttColorKey`, `assign_slots`, and `Timeline` in
     `Fit` only, all unit-tested with no rendering involved
  2. typed `start`/`due` on `TaskRow`
  3. `[gantt]` config plus validation
  4. theme palette, `bar_glyph`, and the new glyphs
  5. `GanttModel::build` over a real `TaskTableModel`
  6. `ui/gantt.rs` span builders
  7. `task_table.rs` width split and span concatenation — at this point the chart
     draws, with no way to reach it yet
  8. `Mode::Gantt`, its actions and bindings, hints, help
  9. `TimelineView`, the zoom ladder, scrolling, and clipping
  10. `Mode::GanttOrder`, `GanttOrderState`, `ui/gantt_order.rs`, and persistence
  11. snapshots, then README and `developer.md`
- steps 1–7 are worth landing before any key exists to reach them. A chart that is
  wrong is much easier to diagnose from a unit test of `column_for` than from a
  screen you got to by pressing four keys
- the width split is the one piece of arithmetic worth writing a test for before
  the renderer exists: a chart region that is off by one is invisible in a diff
  and obvious in a snapshot only after everything else lands
- resist rendering the chart as a second `Paragraph` in its own `Rect`. It would
  need its own copy of the vertical scroll, and the cursor and zebra styles are
  applied per `Line` — the row and its bar would drift apart the first time either
  changed
- `Fit` is a distinct state, not "a window that happens to match the data". If
  scrolling stored a start date and refitting recomputed one, a filter change
  would silently strand the user somewhere the tasks are not. In `Fit` the window
  follows the data; in `Window` it does not, and that is the whole difference
- the dialog previews live and `esc` restores. That is only safe because
  reordering touches nothing but the slot assignment — no query, no filter, no
  fetch — so the restore is a `Vec<String>` swap rather than a reload
- the alphabetical fallback for unordered values is chosen over
  first-appearance-in-the-table order because a bar's color should not change when
  the user re-sorts or filters. It does mean a newly added value can shift colors,
  which is what the dialog is for
- bar-internal labels were considered and dropped. Bars are usually a handful of
  cells wide, and the legend in the border already names the mapping without
  competing with the bar for space
- a completed task's bar keeps its slot color and adds `DIM`. Completion is
  already carried by the `St` glyph and the crossed-out title, so the bar only has
  to agree, not announce
- `TUISANA_TODAY` already pins today everywhere, so the `▼` and `┊` markers are
  deterministic in snapshots with no new mechanism

### Tests

New unit tests:

- `domain/gantt.rs`: config-spelling round trip for all four `GanttColorKey`
  forms; `assign_slots` honoring the configured order, filling from alphabetical,
  capping at six with the seventh onward neutral, and never spending a slot on an
  empty value; `Timeline::fit` month-snapping, `None` for a dateless table, a
  window that excludes a distant today; `windowed` *not* snapping; `column_for` at
  both ends and proportional in the middle; each `TrackShape` case including
  start-after-due, start-only, clipped, and `OffWindow`; and that `Fit` can
  produce neither a clip nor an `OffWindow`
- `app/gantt.rs`: the zoom ladder entered from `Fit` at the nearest step; zoom
  holding the left margin; scroll stepping a quarter window; `gantt_today`
  bringing today to the left margin;
  `gantt_zoom_fit` returning to `Fit`; and the dialog's four moves, including
  moving the top entry up and the bottom entry down as no-ops, the cursor
  following the moved value, the empty value refusing to move, and `esc`
  restoring the order captured on open
- `ui/gantt.rs`: axis span count equals the width; a bar covering today's column
  wins over the today line; the clip glyph lands in the edge cell; gridlines
  appear on group header rows and not on task rows; legend contents and its
  `+N more` and `other` suffixes
- `ui/gantt_order.rs`: the rule lands after the sixth movable entry and moves with
  a reorder; the empty value renders unnumbered and last; the key lines resolve
  from the keymap, so a rebind changes them
- `ui/task_table.rs`: the columns/chart split at 80, 120, and 200 columns; the
  divider at the expected display column on header, task, and group-header rows;
  every row still exactly one line with the chart on; the chart suppressed with a
  warning chip in a pane too narrow; `visible_columns` ignored with the chart off
- `ui/theme.rs`: the new glyphs are width 1 in both sets, and the six `mono` bar
  glyphs are distinct
- `config/mod.rs`: `[gantt]` parses, defaults apply when omitted, an untouched
  `[gantt]` is omitted on serialize, and a bad `color_by`, a bad `[gantt.order]`
  key, and `columns = 0` are each rejected
- `app.rs`: `g` from task mode enters gantt mode with the chart drawn; `esc`
  returns leaving it drawn; `g` again hides it; `enter` opens the dialog and
  `enter` again closes it having written `color_by` and the order to the config
  file, including a previously configured value that was not on screen; `esc`
  writes nothing
- `app/task.rs`: column clamping at both ends, `cycle_gantt_color_key` covering
  the built-ins plus label-kind custom fields, and the gantt status chips

New integration test, `tests/gantt_keyboard_navigation.rs`, following
`tests/project_list_keyboard_navigation.rs`: drive a fake-backed app with a
scripted key sequence through `g`, a scroll, a zoom, `t`, `<` / `>`, `enter`,
`ctrl-k`, `enter`, and assert on the resulting state rather than on rendered
text.

New snapshots at 80, 120, and 200 columns:

- `gantt` — chart on, defaults, fitted
- `gantt-columns` — after `>` `>` widens the table side
- `gantt-zoom` — after `=` `=`, showing clipped bars and the `window` chip
- `gantt-color-section` — after `c` moves the color key
- `gantt-order` — the dialog open, with more than six values so the rule shows
- `gantt-narrow` — the too-narrow degradation, at 80 with several columns shown

These need a wider fixture. `fn client()` in `tests/ui_snapshot.rs` is the fake
backend every snapshot renders — three projects, five tasks, three distinct
assignees and one unassigned task. Six colors and a neutral fallback cannot be
shown by three values: the `gantt-order` snapshot would be three rows with no
rule through them, proving nothing about the part of this milestone that most
needs proving. Add tasks until there are eight or so distinct assignees, and give
them start dates spread widely enough that zooming and scrolling visibly change
the chart.

Widening the fixture rewrites the task pane in the 39 of the 49 committed
snapshots that draw it. Do that as its own commit, before any Gantt code: a
diff of 39 files that only shifts task rows is reviewable, and the same diff
tangled together with the chart landing is not.

Existing tests: no *behavior* should change. The chart is off by default, so every
behavioral test must pass untouched. Three things do move: `theme.rs`'s
glyph-width list gains entries, the config serialization tests gain a `[gantt]`
case, and the snapshots shift with the widened fixture. If a behavioral test needs
editing, something has leaked into the default view and should be fixed rather
than re-baselined.

### Acceptance criteria

- with `[gantt]` absent or `visible = false`, every frame is identical to before
  the milestone and all existing tests and snapshots pass unchanged
- `g` draws a chart to the right of the task table and `g` again removes it,
  costing no body lines and reflowing no pane either way
- every task row renders exactly one terminal line with the chart on, and the
  cursor, multi-select markers, zebra striping, vertical scroll, selection index,
  and group headings all behave as they do without it
- gantt mode has focus of its own: the task pane border and the mode badge both
  show it, `esc` returns to task mode with the chart still drawn, and `j`/`k`
  still move the task cursor while in it
- `<` and `>` change how many table columns are visible, clamped to at least the
  task title and at most every column, and the chart takes the space they release
- a column clipped by the narrower columns budget is still reachable with the
  existing horizontal scroll, and the `columns n/m` chip still reports it
- `h`/`l` scroll the timeline, `-`/`=` zoom it through the ladder, `t` brings
  today to the left of it, and `z` returns it to fitting the loaded tasks
- zooming and `t` both anchor a date near the left edge rather than the middle,
  so what is being looked at stays put and later work comes into view or leaves
- scrolling and zooming issue no request and change no filter; they only move a
  viewport over the rows already in the table
- a bar that runs past a scrolled window is marked at the edge, and a task
  entirely outside it is marked rather than rendering as though it had no dates
- bars span start to due; a task with only a due date renders a milestone glyph; a
  task with no dates renders an empty track; today's column is marked on the axis
  and drawn through empty tracks
- fitted, the time window covers every dated row in the table, snapped to whole
  months, and is not stretched to reach a distant today
- `c` cycles the color key through assignee, section, state, and every enumerated
  custom field present, in both gantt mode and the dialog
- `⏎` opens a modal listing the current dimension's values with their colors and
  task counts, and `ctrl-k` / `ctrl-j` / `t` / `b` move the selected value up,
  down, to the top, and to the bottom
- the dialog draws a rule where the palette runs out, the rule moves as values are
  reordered, and everything below it is drawn neutral
- the chart behind the dialog recolors as values move; `⏎` keeps the order and
  `esc` restores the one the dialog opened with
- `⏎` persists the color key and that dimension's order to `tuisana.toml`,
  preserving previously configured values that were not on screen, and the order
  survives a restart with no hand-editing
- exactly the first six unique values get unique colors and every further value
  gets the neutral color; an empty value is always neutral, never consumes one of
  the six, and cannot be reordered
- values the order does not name follow alphabetically, and a value keeps its
  color across re-sorts and filter changes
- the legend in the pane's bottom border names each colored value and marks the
  neutral group, truncating with `+N more` rather than wrapping
- with `theme.variant = "mono"` or `NO_COLOR=1` the six slots remain
  distinguishable by bar glyph alone, and with `glyphs = "ascii"` the whole chart
  and dialog are ASCII
- a pane too narrow for one column plus a minimum chart drops the chart and says
  so in the pane border instead of drawing an unreadable one
- every one of the new keys is rebindable, and the hint bar and help overlay show
  the rebound key rather than the default
- a rewritten config (from starring or hiding a project) does not gain a `[gantt]`
  table, and the chart's visibility, column count, and timeline window are not
  persisted
- README documents both modes, their bindings, the dialog, and the chart glyphs

## Milestone 13: Edit tasks

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

## Milestone 14: Hardening and polish

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
