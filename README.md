# Tuisana

Tuisana is a terminal UI for reviewing Asana projects and tasks the way "power users" like me like to work with apps: with keyboard shortcuts for everything.

> [!WARNING]
>
> This is a early-stage project largely developed with/by AI. I have personally reviewed all
> of these files, but all code was written by Codex or Claude Code. Documentation is not yet
> optimized for users at this point. This is not a project I have the time/resources to
> manually develop by hand that I thought might be a plausible, well-constrained target for
> an LLM to write. Later milestones may lead to edit abilities, but for now I have focused
> on read-only features out of an abundance of caution.

## Installation

This is a rust app, you can install it like this:

```sh
cargo install --path .
```

## Configuration

The program reads `tuisana.toml` from the current directory.

Start from [`tuisana.toml.example`](./tuisana.toml.example) and copy it to `tuisana.toml`, then fill in the auth values and any project visibility preferences.

### Fixed header values

Leave these unchanged:

- `header.type = "tuisana"`
- `header.version = 1.0`

These values identify the config format and are validated by the app.

### `auth.personal_access_token`

This is the quickest way to authenticate with Asana today.

How to get it:

1. Open the Asana developer console.
2. Create a personal access token.
3. Copy the token into `auth.personal_access_token`.

Asana documents PAT creation and bearer-token usage here:

- https://developers.asana.com/docs/personal-access-token
- https://developers.asana.com/docs/authentication

Keep this value secret.

### `auth.workspace_gid`

This field is optional. Omit it if you want Tuisana to list projects visible to your token without narrowing to one workspace.

If you want to restrict project listing to a specific workspace, you need that workspace's GID.

How to find it:

1. Use Asana's API explorer or call `GET /workspaces` with your token.
2. Find the workspace you want.
3. Copy its `gid` into `auth.workspace_gid`.

Asana docs:

- https://developers.asana.com/reference/getworkspaces
- https://developers.asana.com/reference/getworkspace

### Appearance

The optional `[theme]` section controls colors and glyphs. Every value has a
sensible default, so the section can be omitted entirely.

```toml
[theme]
variant = "ansi"     # "ansi" | "truecolor" | "mono"
glyphs = "unicode"   # "unicode" | "ascii"
accent = "cyan"      # black, red, green, yellow, blue, magenta, cyan, white, gray
zebra = false        # stripe alternating task rows (requires variant = "truecolor")
```

- `variant = "ansi"` (the default) draws with the terminal's own 16-color
  palette, so the UI inherits whatever scheme you already use.
- `variant = "truecolor"` additionally uses indexed shades for subtle
  backgrounds, and is required for `zebra`.
- `variant = "mono"` emits no color at all and relies on bold, dim, and reverse
  video. Setting the `NO_COLOR` environment variable forces this, together with
  the ASCII glyph set.
- `glyphs = "ascii"` replaces every box-drawing and marker glyph with an ASCII
  equivalent, for terminals whose fonts render ambiguous-width characters as
  two cells.

Reading the screen:

- The **focused pane** has a thick border, tinted with the current mode's color.
  The mode badge at the bottom left uses the same color.
- The **header bar** names the app and the current project/task context. The
  **hint bar** above the mode badge lists the most useful keys for the current
  mode, resolved from your actual key bindings. The **status bar** lists only
  the settings that differ from their defaults, so an untouched session shows
  nothing there.
- Pane titles and counts live in the pane's own border.
- `?` opens a grouped help overlay for the current mode. It draws over the panes
  without moving them; press `?` again to dismiss it.
- Due dates render relatively (`Today`, `Tomorrow`, `Wed`, `Nov 14`). An overdue
  date is prefixed with `!` and colored red, one due today is yellow, and one
  within three days takes the accent color.
- In the task table the left gutter carries two markers: the cursor (`▍`) and
  multi-selection (`●`). An empty cell shows `—`.
- `g` draws a Gantt chart beside the task table. With `variant = "mono"` its six
  palette slots are told apart by the bar's texture rather than its colour, so
  the chart still reads with no colour at all.
- Editing a date filter opens a calendar overlay: it starts on the current month
  with today highlighted, `h`/`l` move by day, `j`/`k` flip months, and `enter`
  picks the highlighted day. Typed digits go straight into the filter field, and
  move the highlight to match.
- A date range shades the days between its two ends, with both ends picked out.

### Dates and time zones

Asana's due and start dates are date-only values, so they carry no time zone and
are never converted. What *is* resolved in your local zone is today's date, which
is what `Today` / `Tomorrow` / overdue and the `today` filter keyword are measured
against.

`TUISANA_TODAY=YYYY-MM-DD` pins that date, which is mostly useful for tests and
screenshots.

Date filters accept:

| form | example |
| --- | --- |
| a full date | `2026-09-15` |
| a date in the current year | `09-15` |
| a keyword | `today`, `tomorrow`, `yesterday` |
| the next occurrence of a weekday | `mon`, `friday` |
| an inclusive range | `2026-09-01..2026-09-30` |
| a range open on one side | `today..`, `..2026-12-31` |

A `due` filter is also pushed to the API as `due_on.after` / `due_on.before`, so
narrowing it reduces what gets downloaded.

### Key bindings

The `[[bind]]` section maps keyboard input to command names.
If you omit a command from your config, the built-in default binding for that command still applies.

Each binding may include an optional `mode` field to restrict it to a specific UI context.
Available modes are `any` (default), `project`, `project_search`, `filter`, `filter_edit`, `calendar`, `task`, `gantt`, and `gantt_order`.

Example:

```toml
[[bind]]
key = "j"
command = "move_down"

[[bind]]
key = "ctrl-j"
mode = "filter_edit"
command = "filter_done_editing"
```

All bindable commands:

**Global (any mode)**

- `quit`
- `move_up`
- `move_down`
- `page_up`
- `page_down`
- `jump_top`
- `jump_bottom`
- `scroll_left`
- `scroll_right`
- `refresh`
- `toggle_help_details`
- `toggle_task_view`
- `toggle_task_mode`
- `set_project_mode`
- `set_filter_mode`
- `set_task_mode`
- `resize_top_pane_up`
- `resize_top_pane_down`
- `minimize_top_pane`
- `maximize_top_pane`
- `restore_top_pane`

**Project mode**

- `open`
- `start_search`
- `toggle_selection`
- `select_all_visible`
- `select_all_starred_visible`
- `select_all_non_hidden_visible`
- `invert_selection`
- `clear_selection`
- `undo_selection`
- `redo_selection`
- `toggle_starred_selected`
- `toggle_hidden_selected`
- `toggle_hidden_group`
- `toggle_only_selected`
- `search_fuzzy`
- `search_substring`
- `search_regex`

**Project search mode**

- `clear_search`

**Filter mode**

- `begin_filter_edit`
- `toggle_task_filters`
- `cycle_filter_string_mode`
- `clear_search`
- `search_fuzzy`
- `search_substring`
- `search_regex`

**Calendar mode** (the date picker)

- `calendar_prev_day`
- `calendar_next_day`
- `calendar_prev_month`
- `calendar_next_month`
- `calendar_today`
- `calendar_commit`
- `calendar_close`
- `calendar_clear`
- `calendar_jump_to_start`
- `calendar_jump_to_end`

**Filter edit mode**

- `filter_done_editing`
- `filter_cancel_editing`
- `filter_move_label_left`
- `filter_move_label_right`
- `filter_cycle_label_up`
- `filter_cycle_label_down`
- `filter_add_label`
- `filter_delete_label`
- `filter_caret_left`
- `filter_caret_right`
- `clear_search`
- `search_fuzzy`
- `search_substring`
- `search_regex`

**Task mode**

- `toggle_completed_filter`
- `toggle_subtask_visibility`
- `toggle_project_grouping`
- `toggle_section_grouping`
- `cycle_task_sort`
- `toggle_task_sort_direction`
- `move_section_up`
- `move_section_down`
- `move_project_up`
- `move_project_down`
- `set_gantt_mode`

**Gantt mode**

- `gantt_scroll_left`
- `gantt_scroll_right`
- `gantt_zoom_in`
- `gantt_zoom_out`
- `gantt_zoom_fit`
- `gantt_today`
- `gantt_add_column`
- `gantt_remove_column`
- `cycle_gantt_color_key`
- `gantt_open_order`
- `toggle_gantt`

**Gantt order mode** (the colour dialog)

- `gantt_order_move_up`
- `gantt_order_move_down`
- `gantt_order_move_top`
- `gantt_order_move_bottom`
- `gantt_order_commit`
- `gantt_order_cancel`
- `cycle_gantt_color_key`

---

The top window is shared between the project list and the filter view. When the task panel is visible, the screen is split so the top window keeps the project or filter view and the task table gets the remaining space.

**Global shortcuts** (active in all modes unless overridden):

- `?` to open or close the help overlay for the current mode
- `j`/`down` to move down, `k`/`up` to move up
- `ctrl-u` and `ctrl-d` to page through the list
- `home` and `end` to jump to the top or bottom
- `left` and `right` to scroll columns
- `t` to toggle the task panel, `m` to toggle task mode
- `f` to switch to filter mode, `p` to switch to project mode
- `[` and `]` to shrink or grow the top window
- `{` to minimize the top window, `}` to maximize it, `0` to restore
- `r` to refresh

**Project view shortcuts**:

- `enter` to open the selected project in Asana
- `space` to toggle selection for the current project
- `a` to select all visible projects
- `i` to invert the visible selection
- `c` to clear the selection
- `u` to undo the last selection change, `ctrl-y` to redo
- `*` to toggle starred state for the current selection
- `h` to toggle hidden state for the current selection
- `v` to show or hide hidden projects
- `!` to select all starred visible projects
- `@` to select all visible non-hidden projects
- `o` to filter to selected projects only
- `/` to start search entry
- `ctrl-z`, `ctrl-s`, `ctrl-r` to switch search mode (fuzzy / substring / regex).
  Fuzzy is on `ctrl-z` rather than `ctrl-f` so that `ctrl-b`/`ctrl-f` can move the
  caret in every mode that edits text.

**Project search** accepts ordinary typing, `backspace`, `enter`, and `esc`.
`ctrl-l` clears the search string.

**Filter panel shortcuts** (filter browse mode):

- `j`/`k` to move between filter fields
- `enter` to start editing the selected field
- `esc` to close the panel and return to task mode
- `f` to toggle the filter panel
- `s` to cycle the string-match mode
- `ctrl-l` to clear the search string
- `ctrl-z`, `ctrl-s`, `ctrl-r` to switch search mode

**Filter field editing** (filter edit mode):

- Ordinary typing to edit a text field, inserted at the caret
- `left`/`right`, or `ctrl-b`/`ctrl-f`, to move the caret through the value
- `backspace` to delete the character before the caret
- `enter` to confirm the edit and return to filter browse mode
- `esc` to discard the edit, close the panel, and return to task mode
- `ctrl-z`, `ctrl-s`, `ctrl-r` to switch match mode, `ctrl-l` to clear

For fields include a fixed set of labels, the edit keys navigate instead of typing:

- `h`/`l` to move between the set of listed labeles
- `j`/`k` to cycle the selected label between the possible options
- `a` to add a label, `d` to delete the current one

**Date picking** (calendar mode). Pressing `enter` on the `Due` or `Start` filter
opens the calendar. The text you are editing stays in the filter field, where the
panel draws it with a caret; the overlay shows the month.

Moving the date:

- `h`/`l` to move the highlight by a day. Stepping off the end of a month lands on
  the first of the next one.
- `j`/`k` to flip months. Forward lands on the first of the next month, backward on
  the last of the previous one.
- `t` to jump back to today
- `enter` to pick and close, `esc` to close, `d` to clear the field

Editing the text:

- Ordinary typing goes into the filter field, and the table refilters as it lands
- `left`/`right`, or `ctrl-b`/`ctrl-f`, to move the caret; `backspace` deletes
- `ctrl-a` and `ctrl-e` to jump to a range's start and end dates. They do nothing
  when the field holds no `..`.

The two directions stay in step. Typing `2026-09` moves the grid to September even
though no day is named yet; a year alone shows that January. Moving the highlight
rewrites the date you are on — and only that one, so the other end of a range is
left alone.

**Task view shortcuts**:

- `c` to toggle the completed filter
- `z` to toggle subtask visibility
- `,` to toggle project grouping, `.` to toggle section grouping
- `s` to cycle the task sort field, `^` to flip between ascending and descending
- `[` and `]` to move by section
- `{` and `}` to move by project
- `g` to draw the Gantt chart and switch to gantt mode

### Gantt chart

`g` in task mode draws a timeline to the right of the task table and hands the
chart its own mode. Each task keeps its own row: one line carries the table
cells, a divider, and the bar, so the cursor, multi-selection, and scrolling
behave exactly as they do without it.

`esc` returns to task mode with the chart still drawn. `g` again hides it.

**Gantt mode shortcuts**:

- `h`/`l` to scroll the timeline by a quarter of the window
- `-` and `=` (or `+`) to zoom out and in through a ladder of spans:
  a week, a fortnight, a month, two, three, six, a year, two, five
- `z` to go back to fitting whatever tasks are loaded
- `t` to bring today to the left of the window
- `<` and `>` to show fewer or more table columns, giving the space to the chart
- `c` to colour the bars by the next dimension
- `enter` to open the colour dialog

Zooming holds the left of the window: the date near the left edge stays where
it is and time is added or removed at the right, so what you are looking at
does not move and later work comes into view or leaves it. `t` works the same
way, bringing today to the left rather than to the middle. Both leave a small
margin — a tenth of the window — so the anchored date is not flush against the
divider.

Scrolling and zooming move a viewport over the tasks already loaded. Neither
fetches anything or changes a filter.

Reading the chart:

| | |
| --- | --- |
| `█` | a bar, from the start date to the due date |
| `▒` | a bar whose value did not get one of the six colours |
| `◆` | a milestone: a due date with no start date |
| blank | the task has no dates at all |
| `▼` `┊` | today, on the axis and drawn through rows with no bar there |
| `·` | an axis mark, on group heading rows |
| `░` | a Saturday or Sunday, wherever no bar covers it |
| `‹` `›` | the bar runs past the window, or the task is entirely outside it |

The axis marks whatever it has room for. A year or a quarter is marked by
month; zoom in until a month fits the pane and it marks weeks; again and it
marks individual days; again and each day gets its weekday name too, so the
axis reads `Sat 22  Sun 23  Mon 24`. Every step adds to the one below it —
zooming in never trades the date away for the weekday. The scale follows the
pane's width as well as the span, so a wider terminal shows finer marks at the
same zoom, and a month boundary is always named so a run of day numbers never
leaves you wondering which month you are in.

Once there is at least one column per day, Saturdays and Sundays are shaded.
A bar drawn across a weekend covers the shading, so what shows through is
exactly the weekends a piece of work did not run through.

Fitted, the window covers every dated task, snapped out to whole months. It is
deliberately not stretched to include today — tasks from two years ago would
squash into a couple of columns to make room for a marker. Press `t` to go and
look at today instead.

**The colour dialog** (`enter` from gantt mode) lists the current dimension's
values in the order they are given colours, with a rule where the palette runs
out. Everything below the rule is drawn in the neutral colour, and an unset
value — an unassigned task, a task with no section — is always neutral and can
never be moved above the rule.

- `j`/`k` to move the cursor
- `ctrl-k`/`ctrl-j` to move the selected value up or down
- `t`/`b` to send it to the top or the bottom
- `c` to switch dimension and rebuild the list
- `enter` to save and close, `esc` to cancel

The chart behind the dialog recolours as you move values, so you can see the
effect before saving. Saving writes the dimension and its order into
`tuisana.toml`; cancelling writes nothing.

### Gantt configuration

The optional `[gantt]` section is where the dialog's result is stored. You can
write it by hand, but you do not have to — the keys above set all of it except
the starting visibility and column count.

```toml
[gantt]
visible = false          # true to draw the chart at startup
columns = 2              # table columns kept visible beside the chart
color_by = "assignee"    # "assignee" | "section" | "state" | "field:<Name>"

[gantt.order]
assignee = ["Alex Chen", "Morgan Ellis"]
"field:Priority" = ["High", "Medium", "Low"]
```

- `color_by` names the dimension the bars take their colour from. A custom field
  is written `field:<Name>` so a field actually called "Section" cannot be
  confused with the built-in dimension. Only enumerated custom fields — ones with
  a small fixed set of values — can be cycled to with `c`.
- `[gantt.order]` holds one list per dimension, keyed the same way as `color_by`.
  Values it names are coloured first, in that order; anything else follows
  alphabetically, so re-sorting or filtering the table never repaints a bar.
- Exactly the first six values get a colour of their own. The rest share a
  neutral one.
- The chart's visibility, column count, and timeline window are not saved. They
  are view state like sort and grouping; `visible` and `columns` only set where a
  session starts.

### Project visibility

The repeated `[[project]]` section lets you pin project-specific visibility preferences by Asana project GID. It is usually updated when interacting with the app to change project visibility and project stars, rather than modified directly by a user.

Each entry supports:

- `gid`
- `starred`
- `hidden`

How it works:

- `starred = true` sorts the project ahead of unstarred projects.
- `hidden = true` keeps the project in the hidden group.
- Hidden projects stay available in the list, but they are shown after the visible projects only when you toggle them on.
- Hidden projects are marked explicitly in the UI so they are easy to spot.

The default toggle for hidden projects is `v`.

Example:

```toml
[[project]]
gid = "123"
starred = true
hidden = false

[[project]]
gid = "456"
starred = false
hidden = true
```
## Running

Use the project tasks:

```bash
cargo run
mise test
mise coverage
```
