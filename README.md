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
- A spinner at the top left reports work in progress: `loading` while tasks are
  being fetched, `filtering` when the table has fallen behind what you have
  typed. The second only appears once the wait is long enough to notice, so a
  filter that keeps up never shows it.
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
  move the highlight to match. Its grid runs `Su Mo Tu We Th Fr Sa` — the Gantt
  chart's weekends and week boundaries still count from Monday, because a work
  week and a calendar grid are read differently.
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
narrowing it reduces what gets downloaded. With more than one filter set the
window sent is the *union* of theirs, and a set that is open-ended, unfiltered,
asking for an empty due date, or negating either the due row or itself opens
that side of it — otherwise the server would drop rows a set had asked for.

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
- `filter_require_empty`
- `filter_set_add`
- `filter_set_remove`
- `filter_set_next`
- `filter_set_prev`
- `filter_sets_toggle`
- `filter_set_load_1` ... `filter_set_load_9`
- `filter_sets_page_back`
- `filter_sets_page_forward`
- `filter_set_save`
- `filter_set_copy_to_new`
- `filter_set_new`
- `filter_set_delete`

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
- `filter_require_empty`
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
- `e` to require the field to be empty
- `a` to add a filter set, `x` to remove the current one, `h`/`l` to move between
- `!` to negate the selected field, `~` to negate the whole set
- `b` to show or hide the named-set sidebar
- `1`-`9` to load a named set, `<`/`>` to page the list
- `w` to save the panel under a name, `y` to copy it to a new unnamed one,
  `d` to delete it
- `n` to start a completely fresh panel
- `ctrl-l` to clear the search string
- `ctrl-z`, `ctrl-s`, `ctrl-r` to switch search mode

The panel's fields are `Title`, `Assignee`, `Due`, `Start`, `State`, `Projects`,
and one row per custom field. `Title` and `Assignee` match fuzzily by default —
a person's name is typed from memory and is the field most likely to be
half-remembered — and `ctrl-s` switches a row to `contains`, `ctrl-r` to regex.

**Filter sets.** Filling in more than one field *narrows*: a `Due` of
`..2026-09-01` and an `Assignee` of `alex` shows Alex's tasks due before
September. What that cannot express is a union, so the panel holds one or more
filter sets:

- Each set is the same list of fields, and fields within a set still AND.
- Sets are ORed: a task is shown when it satisfies **any** set.
- A new set starts empty, so adding one never hides a task that was visible
  before — it can only add.
- Sets appear as numbered tabs on the pane's top border, immediately right of
  the `Filters` title and introduced by `any of`. The set you are standing on
  is filled in and spells out `set`; an active marker on any other tab means it
  is filtering, so a set doing work is visible from a different tab. With a
  single set there is no strip at all.
- `x` is refused when there is only one set: one set is the panel, not a set of
  sets.

End to end: `f` opens the panel, `j j` lands on `Due`, `enter` opens the
calendar, `today..2026-09-28` picks the next week, `enter` commits, `a` adds a
second set, `enter` opens its `Due` — the field cursor is shared, so you are
already on the same row — `2027-01` picks four months out, `enter`. The table
now holds both groups and nothing in between.

**Named filter sets.** A filter you built once can be recalled by pressing a
digit. A **named filter set** is the whole panel — every tab, each tab's
negation, and each field's query, match mode, require-empty flag, and negation
— saved under one name.

It is deliberately *not* the sort, the grouping, the completed filter, subtask
visibility, or the project selection. Those are the other half of the screen:
a saved filter is a question about tasks, not about which projects it is asked
of.

- `b` opens a `Sets` pane down the left of the filter view. It is **never
  focused** — `j` and `k` keep walking the filter fields, and its thin border
  is what says so. On a narrow terminal it gives way rather than squeezing the
  filter rows.
- Pinned at the top is the live panel: its name, or `unnamed`, with how many
  sets and how many active filters it holds. Below the rule are the saved
  entries in name order, numbered from the top of the visible window.
- `1`-`9` load the entry at that position. `<` and `>` page the window when
  there are more entries than fit.
- `w` opens a one-line prompt on the sidebar's border, pre-filled with the
  loaded name. `enter` commits, `esc` cancels. An existing name is
  overwritten; a new one is created. A blank name is refused.
- `d` deletes the **loaded** entry after a `y`/`n` confirmation, and unbinds
  the panel. It is refused when nothing is loaded.
- `n` throws the panel away and starts from nothing: one empty set, every row
  back to the match mode it was built with, and nothing bound. Unlike `a`,
  which keeps the match modes so a second set inherits them, this is a blank
  slate. The entry you were on keeps whatever was last written to it.

**Loading binds the panel to the name.** From then on the entry is a live view
rather than a snapshot: type into a field, add a tab, negate a set, and the
named entry changes with it, on disk. A named set behaves like a file you have
open, not like a clipboard — the alternative, a snapshot you have to remember
to re-save, is the one that loses work. The write happens once per settled
burst of typing, not once per keystroke.

Two escapes: `y` **copies the panel to a new unnamed one**, keeping exactly
what is on screen while the entry keeps whatever was last written to it —
this is how you take a saved filter as a starting point and go somewhere else
with it without disturbing the original. `w` saves under a new name, which
rebinds to that one.

A filter naming a custom field from a project you have not loaded is *kept*,
not dropped: it is re-resolved each time a project's fields arrive, and
written back out meanwhile. Loading a set with the wrong projects selected
cannot quietly erase half of it.

**Requiring an empty field.** An empty filter field means "do not filter", so
there is a separate key for "has no value": `e` on a filter row, or `ctrl-e`
while editing one. The row's value shows as `(none)`, it counts towards the
active-filter count, and the table narrows to tasks with nothing in that field.
Pressing `e` again clears it; so does typing, because a value and a
require-empty are mutually exclusive.

| field kind | empty means |
| --- | --- |
| text (`Title`, `Assignee`, `Projects`, text custom fields) | the value is missing or all whitespace |
| labels (custom fields with a small value set) | the task carries no value for the field |
| date (`Due`, `Start`) | the task has no such date |

`State` is the exception: a task is always either open or done, so it can never
be empty and the key does nothing there.

Combining the two is the point: set 1 asks for tasks due this week, set 2 for
tasks with no due date at all, and the table shows the work that is either
imminent or unscheduled.

**Negation.** `!` on a filter row inverts it: the row keeps exactly what it was
throwing away. Everything else on the row still means what it says — the match
mode, the value, and require-empty are all evaluated first and the answer is
flipped afterwards — so `!` on a require-empty row reads "has some value",
which nothing else in the panel can express. `ctrl-n` does the same while
editing, where `!` is an ordinary character. Negating a row with nothing in it
changes nothing: an empty field still means "do not filter".

`~` negates the whole active set instead, after its fields have ANDed. That is
`not (a and b)`, which is a different statement from negating each row: with
`Assignee: alex` and `Due: August`, negating the rows asks for tasks that are
neither Alex's nor in August, while negating the set also keeps Jo's August
task. A negated *empty* set matches nothing rather than everything, so adding a
set still can only widen the result.

Both are marked without relying on colour. A negated row swaps the gutter's
active marker for `¬` and puts `¬` in front of its value, so the row reads
`Assignee ¬ alex`. A negated set's tab carries the same `¬`, and the dividers
on either side of it thicken from `│` to `║`; while you are standing on it the
pane's counts also say `negated set`, because the rows below show a positive
filter that the set then inverts wholesale.

A negation on a `Due` row, or on a set containing one, stops that due window
being pushed to the API — the negated form is satisfied by dates outside the
window, and a window narrower than the truth would be cached as covered.

Typing into a filter never waits for the table. While keys are still arriving
the rebuild is put off and one pass runs when you stop, rather than one pass per
character — so a slow filter costs a stale table and a spinner, never a
dropped or delayed keystroke.

**Filter field editing** (filter edit mode):

- Ordinary typing to edit a text field, inserted at the caret
- `left`/`right`, or `ctrl-b`/`ctrl-f`, to move the caret through the value
- `backspace` to delete the character before the caret
- `enter` to confirm the edit and return to filter browse mode
- `esc` to discard the edit, close the panel, and return to task mode
- `ctrl-z`, `ctrl-s`, `ctrl-r` to switch match mode, `ctrl-l` to clear
- `ctrl-n` to negate the field, `ctrl-t` to negate the set

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

### Named filter sets

The repeated `[[filter_set]]` section holds filter panels saved under a name.
It is normally written by the app — `w` in the filter panel, and then every
edit while the entry is loaded — but the file is writable by hand and is read
back exactly as written. Anything at its default is left out, so a simple
entry stays short.

```toml
[[filter_set]]
name = "Sprint triage"

  [[filter_set.set]]

    [[filter_set.set.field]]
    key = "assignee"
    query = "alex"
    match = "fuzzy"

    [[filter_set.set.field]]
    key = "due"
    query = "..today"

  [[filter_set.set]]
  negated = true

    [[filter_set.set.field]]
    key = "custom:Priority"
    query = "Low"
```

- `name` must be non-empty, and no two entries may have names that differ only
  in case: the sidebar lists them by number, and two rows reading the same is
  a trap.
- Each `[[filter_set.set]]` is one tab. Fields AND within a tab; tabs OR
  between them. `negated` inverts the whole tab, after its fields have ANDed.
- `key` is the panel's own row key: `title`, `assignee`, `due`, `start`,
  `state`, `projects`, or `custom:<Name>`. Custom fields are keyed by *name*
  so an entry survives a reload that brings different Asana ids. A key that
  matches no row in the loaded data is kept rather than rejected.
- `match` is `fuzzy`, `contains`, or `regex`, and only applies to text rows.
  It is omitted when it is the row's own default.
- `empty = true` is "has no value", the same filter `e` sets. It is ignored on
  a row that cannot be empty, such as `state`.
- `negated = true` on a field inverts that row's verdict alone.

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
