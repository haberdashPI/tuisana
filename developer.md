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
13. `src/domain/task_edit.rs` is the write model: one field of one task, plus
    the parsing that turns a typed cell into it.
14. `src/app/task_edit.rs` owns the open cell edit; `src/app/text_edit.rs` is
    the one-line text buffer every editable field runs on.
15. `src/app/autocomplete.rs` is the completion state machine behind
    `Assignee`, `Projects`, and the filter panel's `list` match mode.
16. `src/asana/throttle.rs` is the write budget: how many actions may be in
    flight and how fast they may go.

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
- The top pane holds the project list **or** the filter panel, and which one
  is the panel's own `visible` flag rather than the mode. The panel stays on
  screen when the keys move to the table — `f` and `p` are what close it — so
  "the panel is showing" and "the panel has the keys" are two questions.
  `App::filter_panel_focused` answers the second, and is the single place that
  knows calendar mode belongs to whichever editor opened the picker; every
  routing decision (`TaskState::apply_action`, raw filter-field typing, the
  focused border) reads it rather than re-deriving it from the mode.
- A date filter stores the **text**, not the date it resolved to.
  `DateQuery::parse` runs against `date::today()` on every rebuild, so a saved
  `this month` means October in October. A task date does the opposite:
  `parse_date_value` pins it at commit time, because Asana stores a date and
  not an expression. The one grammar is in `src/domain/date.rs`; `parse_span`
  is the whole of it, and `parse_token` is that with a multi-day span collapsed
  to its first day for the callers that can only hold one date.
- Calendar mode's grid keys are **plain letters that also spell day names**, so
  `TaskState::calendar_grid_visible` gates them: with the grid hidden,
  `handle_key_event_inner` routes `Action::is_calendar_grid_action` to
  `handle_calendar_input` instead of firing it. Adding a key to that mode means
  deciding which side of that gate it belongs on.
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
- A named entry carries **the project selection** as well as the filters, so
  the `Sets` sidebar and its seven keys belong to the project view too. The
  two halves of the write-through are not symmetrical: the fields have a dirty
  flag the filter editor sets, and the projects have `App::bound_projects` —
  the selection as it stood when the binding was established. A *change* from
  that baseline is what writes, which is what leaves a gid the workspace no
  longer returns on disk until the selection is deliberately edited. The pair
  carries the entry's name — `None` for the scratch slot — so a load, a save
  under a new name, and the startup restore all re-baseline in one place
  rather than at each site. `me` stands for the assigned-to-me row in the
  file, because that row's `Project.id` is the logged-in user's gid;
  `src/app/project_list.rs` owns both directions of that translation. That
  identity is also a hazard on the write side: the gid is a *user*, and Asana
  answers `404 Not a recognized ID` for it as a project, so
  `EditContext::is_assigned_to_me_row` is what keeps it out of one.
  `cursor_project` returns `None` for a row under that heading, which is why
  a task created there goes to the workspace — with the user as its assignee,
  because the group is "tasks assigned to me" and an unassigned one would be
  gone on the next load.
- Since config **version 3** a project selection lives on a `[[filter_set]]`
  and nowhere else. An unnamed panel has one too, so it gets the entry with
  `scratch = true`: hidden from the sidebar, never addressed by a digit,
  never found by name, and holding a selection and nothing else — its
  `sets` stay empty so an unnamed panel's *filters* still exist nowhere but
  on screen, which is what the discard confirmation promises.
  `Config::sorted_filter_sets` and `ui::filter_sets::sorted_names` both leave
  it out, and they have to agree or a digit loads a different entry than the
  row it is drawn beside.
- **Nothing is in play until it is selected.** `task_target_projects` is the
  selection and only the selection; the cursor row used to stand in for an
  empty one, and that was the last implicit project list in the app. With it
  gone, "nothing selected" is a real state — `App::project_gate_closed` — and
  `set_filter_mode` refuses in it, because a filter panel's rows are built
  from the loaded projects' custom fields. The gate is judged only after the
  first `load_projects`: the panes are restored before Asana is spoken to, so
  `enforce_project_gate` is what closes a restored `filters = true` that
  turns out to have nothing behind it. `n` and a load of a project-less entry
  go through it too.
- `Action` is the shared input vocabulary.
- `KeyMap` is built from config bindings and resolves keys to actions. A binding
  with a `mode` shadows the global one, which is how `c`, `t`, and `h`/`l` mean
  different things in different modes.
- `Mode::Gantt` and `Mode::GanttOrder` drive the chart and its colour dialog.
  Both are siblings of `Mode::Task`: the same pane, the same rows, different
  keys. Both allow the `Any` fallback, so `j`/`k`, `q`, and `?` keep working.
- `GanttViewState` is the chart's session state; only its colour order is ever
  written back to config.
- A bulk edit **says how wide it is before it happens, and asks when it is
  wide**. `TaskTableView::is_edit_target` bands the cursor column's cell on
  every row the commit would write to, which is `TaskState::edit_targets`
  rendered — the selection when there is one, the cursor row when there is
  not. The two have to agree: a band is the only thing on screen that says how
  far the next edit reaches. It follows that the cursor row does **not** band
  while a selection it is not part of exists, because `space` leaves the
  cursor a row past the last thing it marked and a band there would promise an
  edit that is not going to happen. Past `edit.confirm_threshold` writes the
  question too, as `Mode::Confirm`: `submit_writes` holds the resolved writes
  on `App` and applies *nothing* — not even the optimistic update — until `y`,
  so `n` leaves a table that never moved.
- An edit is **optimistic**. `apply_and_enqueue` writes it onto the cached
  record and the visible dataset at once, then queues it for the write pool;
  `poll_task_edits` takes the server's `modified_at` on success and puts the
  old value back on failure. The local write deliberately bypasses
  `TaskCache::upsert_record`: `merge_task_record` is monotone — `completed |=
  incoming`, and a `None` never overwrites a `Some` — so un-completing a task
  or clearing a date through it is a silent no-op. `confirm_edit` exists for
  the other half of the same problem: without the server's timestamp, a fetch
  that started before the edit comes back looking newer and wins.
- **Sorting is a column gesture, not a field one.** `s` reads the column cursor
  and steps that column through descending, ascending, and unsorted, so
  `TaskSortRule` names a *column* — `Due` and `Start` are separate, and a
  custom field is named by its label because its id differs per project. The
  rules are a stack the user builds one column at a time: a column newly
  sorted goes to the front, a direction flip keeps its place, and everything
  falls through to `compare_default` (date, then title, then Asana's order),
  which is both the unsorted order and the tie-break that keeps the ordering
  total. Grouping is upstream of all of it and still wins. The one column that
  does not sort by the text in its cell is `Start`: an empty start date sorts
  as the due date, because Asana will not save a start without a due date, so
  a blank Start cell means the task starts the day it is due.
- Reading merges what writing must keep apart. One custom-field *name* usually
  exists once per project, with a different gid in each, and the table shows
  one column for it — so a write resolves the gid from the task's own project,
  per task, at commit time.
- `Assignee` and `Projects` are references, not text: both run
  `AutocompleteState`, which holds a list of picked items over a supplied list
  of `(handle, display)` candidates and refuses anything that is not one of
  them. The candidates arrive through `EditContext` — the project list belongs
  to the other pane, and the workspace directory is one `list_users` cached on
  `App` for the session — so the editor stays testable without a backend.
- A **start date is never written alone**. Asana rejects any request that
  sets or clears `start_on` without also naming `due_on` or `due_at`, so
  `HttpAsanaClient::update_task` reads the task's current due date back and
  restates it in the same body. It reads it from the server rather than from
  the cached record: a stale local copy would not just fail the write, it
  would quietly move the due date.
- Project membership is not a field on the task, so a commit resolves to
  `ProjectEdit`s rather than `TaskFieldEdit`s and goes out through
  `addProject` / `removeProject`. Both kinds share one write channel, one
  burst counter, and one rollback path: see `PendingEdit` in `src/app.rs`.
- **Batching buys round trips, not quota.** Asana's `/batch` takes ten actions
  and counts, in its own words, "as though you had made a separate HTTP request
  for every individual action" — against both the per-minute limiter and the
  15-concurrent-write one. So `AsanaClient::write_tasks` chunks to ten and
  `WriteThrottle` is what actually keeps us under the ceiling, counting
  **actions rather than requests**: a chunk costs its length, plus one for each
  start-date edit, because that edit cannot be built without a read of its own.
  The old shape — `thread::spawn` once per edit — opened forty sockets for
  forty selected rows, nearly three times the write ceiling. `WritePool` is a
  fixed two threads over a queue instead, and the permits rather than the
  thread count are the limit. `ReqwestTransport::send_with_retry` is the
  backstop under all of it: a `429` is waited out on its own `Retry-After`,
  because a rejected request counts against the quota too.
- The **recently-edited pane** is the price of those two edits: they are the
  ones most likely to make a row vanish. `TaskViewState` keeps the gids, and
  `refresh_table` rebuilds the pane from them right after the table, because
  what the pane holds is defined by what the table stopped showing. The cursor
  is one cursor over two lists — `recent_selected` is `Some` when the keys act
  on the pane — and `cursor_task_gid` is what every edit path asks.
- The `[view]` section is **what is open, not where you were**. The panes'
  open/closed flags, the top pane's three-state size, the bound filter set's
  name, and the selected project gids persist; heights, scroll offsets, the
  cursor, the sort, the grouping, and which pane had focus do not. It is
  written from `App::handle_key_event` on the same settled-burst rule as the
  filter-set write-through, and both are staged before either is written so a
  key that moves both costs one write.
- `FakeAsanaClient` is the test backend for deterministic state transitions.
- `HostEffects` is the seam for everything a session does **outside** the
  terminal: `o` opening a browser and `y` writing the clipboard. `SystemHost`
  is the real one and `run_app` is the only thing that builds it;
  `RecordingHost` writes both down and does nothing. `run_session` takes one
  as a required argument rather than defaulting, because a test that reached
  the real browser by forgetting to say otherwise opens a window on the
  machine running `cargo test` — which is exactly what used to happen.

## Common Change Paths

- To add or rename a shortcut, update `src/input/mod.rs`, `src/config/mod.rs`, and any help text that mentions the key.
- To change project-list behavior, update `src/app/project_list.rs` and the project list renderer in `src/ui/project_list.rs`.
- To change task-review behavior, update `src/app/task.rs` and the task table renderer in `src/ui/task_table.rs`.
- To change how a cell is edited, start at `TaskState::begin_cell_edit` in
  `src/app/task.rs`: it picks the editor from the column and is where every
  refusal lives. The editors themselves are in `src/app/task_edit.rs`, the
  parsing in `src/domain/task_edit.rs`, the keys in `App::handle_action`
  (`Mode::TaskEdit`), and the drawing in `cell_spans` in
  `src/ui/task_table.rs`.
- To change how a name is completed, start in `src/app/autocomplete.rs`; the
  candidate overlay is `src/ui/completion.rs` and is drawn for whichever of
  the two callers has an editor open (`TaskState::open_completion`).
- To change when a bulk edit is asked about, the threshold is
  `EditConfig::needs_confirmation` in `src/config/mod.rs`, the gate is
  `App::submit_writes`, the keys are `App::handle_confirm_input`, and the
  window is `ui::confirm::bulk_edit_view`. `ui::confirm` is one window shared
  by three callers — the filter sidebar, the config migration, and this — so a
  change to how it is drawn moves all three.
- To change how fast writes go out, the constants are in
  `src/asana/throttle.rs` and nothing else should hold a number: `TokenBucket`
  is pure and takes the instant, so every pacing rule is testable without a
  clock.
- To change what a failed or refused edit says, the message is built where the
  edit fails (`reconcile_task_edit` and the `Err` arms in `src/app.rs`), stored
  as `edit_notice` on `TaskState`, and drawn by `src/ui/notice.rs` — a box in
  the bottom-right corner, over every other overlay, wrapped and newline-aware.
  `esc` clears it from `handle_key_event_inner` without consuming the key.
- To change the recently-edited pane, start at `rebuild_recent_table` and the
  cursor handoff in `refresh_table` (`src/app/task.rs`); its geometry is
  `split_recent` in `src/ui/layout.rs` and it renders through
  `task_table::recent_pane_view`, which clones the table's own widths.
- To change the named-set sidebar, start in `src/ui/filter_sets.rs`.
  `split_sidebar` lives there rather than in `layout` — the sidebar is not a
  frame region — but it is called from the top-pane dispatch in
  `src/ui/runtime.rs`, because it belongs to whichever pane that window is
  holding and the two views give way at different widths.
- To add a **config format version**, bump `Header::CURRENT_VERSION`, add a
  version-gated step to `migrate` in `src/config/mod.rs`, and extend
  `migration_view` in `src/ui/runtime.rs` so the prompt describes the changes
  that file is actually behind on. A key being migrated *away* has to stay on
  the struct to be readable — `ViewConfig::legacy_projects` is one, read and
  never written — or the value is gone before migration can move it.
- To change the Gantt chart, start in `src/domain/gantt.rs` for anything about
  dates or colour assignment, and `src/ui/gantt.rs` for how it is drawn. The
  split between the table columns and the chart lives in `split_pane` in
  `src/ui/task_table.rs`.
- Snapshots in `tests/snapshots/` are the visual regression guard. Run
  `UPDATE_SNAPSHOTS=1 cargo test --test ui_snapshot` and read the diff before
  committing it.
- The README's screenshot is drawn from the same fixture, as an SVG of the
  terminal grid. When a change moves what it shows, regenerate it with
  `cargo test --test ui_snapshot readme_screenshot -- --ignored`.
- To change what the app remembers about the view between runs, the section
  is `ViewConfig` in `src/config/mod.rs`, the read is `App::apply_view_config`
  plus the one-shot selection restore in `App::load_projects`, and the write
  is `App::stage_view_state`. Adding a field means touching all three: the
  restore and the capture are deliberately separate functions, so a field
  added to one and not the other is written and never read back.
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
