# Milestone 12: Gantt chart view ✓

[← all milestones](../plan.md)

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

## Target design

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

Design rules carried forward from [Milestone 11.5](11.5-visual-redesign.md):

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

## Deliverables

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
  are punctuation and `ctrl-` pairs. (It also means [Milestone 14](14-edit-tasks.md)'s sketched
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

## Implementation notes

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

## Tests

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

## Acceptance criteria

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
