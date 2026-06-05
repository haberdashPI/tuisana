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
- the current search state is clearly labeled as awaiting input, not searching, entered text, and search mode

## Milestone 6: Review tasks for one or many projects

Goal:

- show tasks in a table for one selected project or a user-defined set of projects

Deliverables:

- task loading from Asana
- section data loading
- custom field discovery for project-specific fields
- task table model with common columns and project-specific columns

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

## Milestone 7: Navigate, filter, and sort tasks

Goal:

- navigate the task list with vim-like shortcuts
- filter by field and subtask visibility
- sort tasks predictably

Deliverables:

- task navigation actions
- filter model
- sort model
- subtask include/exclude toggle
- stable ordering rules

Implementation notes:

- filters should be composable where practical
- support filtering by explicit field and by text search
- make subtask visibility a first-class filter, not an afterthought
- use a stable fallback sort to avoid jumpy rows

Acceptance criteria:

- row navigation works
- filtering by field works
- subtask visibility can be toggled
- sorting is deterministic
- tests cover filter and sort combinations

## Milestone 8: Edit tasks

Goal:

- edit tasks from the TUI using vim-like commands and modal interactions

Deliverables:

- field editing
- section moves
- subtask conversion
- dependency updates
- date editing

Implementation notes:

- prefer optimistic local updates when they are safe
- reconcile state after server updates
- keep mutation commands explicit and validate them before sending to Asana
- model edits as domain mutations rather than UI-specific actions

Acceptance criteria:

- tasks can be edited
- section changes work
- subtask state can be changed
- dependency changes work
- date updates work
- tests cover parsing, validation, and mutation application

## Milestone 9: Hardening and polish

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
