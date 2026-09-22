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

## Milestones

One file per milestone, in [`plan/`](plan/), in the order they were worked.
A milestone file is the whole record of that piece of work: what it was for,
how it was built, and — for the delivered ones — what was decided differently
along the way.

| # | Milestone | Delivered |
| --- | --- | --- |
| 0 | [Project skeleton](plan/00-project-skeleton.md) | ✓ |
| 1 | [List projects](plan/01-list-projects.md) | ✓ |
| 2 | [Asana API setup](plan/02-asana-api-setup.md) | ✓ |
| 3 | [Configurable vim-like project navigation](plan/03-configurable-vim-like-project-navigation.md) | ✓ |
| 4 | [Project visibility management](plan/04-project-visibility-management.md) | ✓ |
| 5 | [Multi-select project management](plan/05-multi-select-project-management.md) | ✓ |
| 5.5 | [Improve Selection Commands](plan/05.5-improve-selection-commands.md) | ✓ |
| 6 | [Review tasks for one or many projects](plan/06-review-tasks-for-one-or-many-projects.md) | ✓ |
| 6.5 | [Improve task view behavior](plan/06.5-improve-task-view-behavior.md) | ✓ |
| 7 | [Navigate, filter, and sort tasks](plan/07-navigate-filter-and-sort-tasks.md) | ✓ |
| 7.5 | [Refine views / binding setup](plan/07.5-refine-views-binding-setup.md) | ✓ |
| 8 | [Code legibility](plan/08-code-legibility.md) | ✓ |
| 8.5 | [Structural cleanup](plan/08.5-structural-cleanup.md) | ✓ |
| 9 | [Lazy, filter-aware task loading](plan/09-lazy-filter-aware-task-loading.md) | ✓ |
| 10 | [Task interaction](plan/10-task-interaction.md) | ✓ |
| 11 | [Assigned-to-me task view](plan/11-assigned-to-me-task-view.md) | ✓ |
| 11.5 | [Visual redesign](plan/11.5-visual-redesign.md) | ✓ |
| 11.75 | [Date selection](plan/11.75-date-selection.md) | ✓ |
| 12 | [Gantt chart view](plan/12-gantt-chart-view.md) | ✓ |
| 13 | [Filter sets](plan/13-filter-sets.md) | ✓ |
| 13.5 | [Named filter sets](plan/13.5-named-filter-sets.md) | ✓ |
| 14 | [Edit tasks](plan/14-edit-tasks.md) |  |
| 15 | [Hardening and polish](plan/15-hardening-and-polish.md) |  |

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
