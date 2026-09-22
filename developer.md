# Developer Guide

This file is the quickest way to understand the codebase without reading everything at once.

## Start Here

1. `src/main.rs` is the runtime entry point.
2. `src/ui/runtime.rs` wires keyboard input to application state and renders the TUI.
3. `src/input/mod.rs` defines the action model and key parsing.
4. `src/config/mod.rs` defines the TOML config format and default bindings.
5. `src/domain/mod.rs` is the public surface of the domain layer.
6. `src/domain/project.rs` defines project identity and visibility.
7. `src/domain/task.rs` defines task records, filtering, sorting, and table layout.
8. `src/app.rs` owns top-level app mode, pane sizing, and task loading.
9. `src/app/project_list.rs` manages project list state and selection behavior.
10. `src/app/task.rs` manages task state, filters, sorting, and task-table data.
11. `src/domain/gantt.rs` maps dates to chart columns and values to colours.
12. `src/app/gantt.rs` owns the chart's session state and the colour dialog.

The async task-data path is split across two modules:

- `src/app.rs` owns the request lifecycle, generation tracking, and polling
- `src/app/task.rs` owns the cached dataset, filters, and table rebuilds

## Reading Order

If you are trying to understand one user interaction end-to-end, read in this order:

1. `src/ui/runtime.rs` to see how key events are translated into `Action`s.
2. `src/input/mod.rs` to see how bindings and actions are represented.
3. `src/domain/mod.rs` to see the public domain exports.
4. `src/domain/project.rs` and `src/domain/task.rs` to see the core data models and rules.
5. `src/app.rs` to see how the app routes actions to project or task state.
6. The relevant state module:
   - `src/app/project_list.rs` for project selection and search
   - `src/app/task.rs` for task filtering, sorting, and navigation
7. The matching UI module in `src/ui/` if you need to understand rendering details.

The Gantt chart is the one renderer that does not own a region of the screen.
`src/ui/gantt.rs` returns spans that `src/ui/task_table.rs` appends to each
row's own `Line`, so a task and its bar are one line. That is what lets the
cursor highlight, zebra striping, and vertical scroll cover the chart without
knowing it exists — and why "one line per row" is a hard invariant rather than
a convention.

## Core Concepts

- `App` is the top-level state machine.
- `ProjectListState` owns visible projects, selection, and project search.
- `TaskState` owns task datasets, filters, selection, and the task table.
- `Domain` owns the core data models and rules orchestration built on top of them.
- `request_task_data` starts an async fetch; `poll_task_data` merges the results.
- The filter panel holds one or more `TaskFilterSet`s, ORed together: fields AND
  within a set, sets OR between them. `restore_queries` is what carries the set
  list, the active tab, and every query across the rebuild each streamed project
  triggers — state the user can change that is not re-applied there vanishes
  mid-load. A `due` filter is pushed down to Asana, so `due_date_range_for_query`
  has to send the *union* of the sets' windows or the cache records a window as
  covered that was never fetched.
- The panel can be **bound** to a named entry in `tuisana.toml`, and while it
  is, every change writes through to that entry — once per settled burst of
  typing, from `App::handle_key_event`, which is the one place that runs after
  every key including the ones read outside the keymap. `TaskFilterSet`'s
  `unresolved` is what keeps a filter naming a not-yet-loaded custom field
  from being erased by that write-through.
- `Action` is the shared input vocabulary.
- `KeyMap` is built from config bindings and resolves keys to actions. A binding
  with a `mode` shadows the global one, which is how `c`, `t`, and `h`/`l` mean
  different things in different modes.
- `Mode::Gantt` and `Mode::GanttOrder` drive the chart and its colour dialog.
  Both are siblings of `Mode::Task`: the same pane, the same rows, different
  keys. Both allow the `Any` fallback, so `j`/`k`, `q`, and `?` keep working.
- `GanttViewState` is the chart's session state; only its colour order is ever
  written back to config.
- `FakeAsanaClient` is the test backend for deterministic state transitions.

## Common Change Paths

- To add or rename a shortcut, update `src/input/mod.rs`, `src/config/mod.rs`, and any help text that mentions the key.
- To change project-list behavior, update `src/app/project_list.rs` and the project list renderer in `src/ui/project_list.rs`.
- To change task-review behavior, update `src/app/task.rs` and the task table renderer in `src/ui/task_table.rs`.
- To change the named-set sidebar, start in `src/ui/filter_sets.rs`. Its split
  from the filter panel lives there too, in `split_sidebar`, not in `layout`:
  the sidebar belongs to the panel rather than to the frame.
- To change the Gantt chart, start in `src/domain/gantt.rs` for anything about
  dates or colour assignment, and `src/ui/gantt.rs` for how it is drawn. The
  split between the table columns and the chart lives in `split_pane` in
  `src/ui/task_table.rs`.
- Snapshots in `tests/snapshots/` are the visual regression guard. Run
  `UPDATE_SNAPSHOTS=1 cargo test --test ui_snapshot` and read the diff before
  committing it.
- To change how input is dispatched, update `src/ui/runtime.rs` and `src/app.rs`.

## Testing Strategy

- Prefer unit tests for state transitions in `src/app/`, `src/input/`, and `src/config/`.
- Use integration tests in `tests/` for end-to-end key handling and rendering behavior.
- When changing behavior, add a regression test next to the code path that changed.
- Run `cargo test` before and after a refactor to confirm the behavior and appearance stay stable.

## Notes For Refactors

- Keep the UI thin and the state transitions explicit.
- Favor small helper functions over deep abstraction layers.
- Prefer names that describe the concrete behavior over names that describe the implementation.
- If a function is getting hard to scan, split it by the major user flow or state branch it handles.
