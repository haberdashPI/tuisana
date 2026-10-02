# Tuisana

Tuisana is a terminal UI for reviewing Asana projects and tasks the way "power users" like me like to work with apps: with keyboard shortcuts for everything.

> [!WARNING]
>
> This is a early-stage project largely developed with/by AI. I have personally reviewed all
> of these files, but all code was written by Codex or Claude Code. Documentation is not yet
> optimized for users at this point. This is not a project I have the time/resources to
> manually develop by hand that I thought might be a plausible, well-constrained target for
> an LLM to write.

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
- `header.version = 3.0`

These values identify the config format and are validated by the app.

**Versions 1 and 2 are still readable, and are migrated once.** There are two
changes, and a file gets whichever ones it is behind on:

- **Version 2** renamed six command names and one mode name, and made
  shift+letter a key namespace of its own — so a version-1 binding of
  `key = "G"`, which used to mean `g`, would silently become a different key.
- **Version 3** moved the project selection out of `[view]` and onto the
  filter sets. A bound entry adopts the list that was sitting beside it; with
  nothing bound it goes to the reserved `scratch` entry, so the projects you
  were working with come back either way.

Rather than let either happen quietly, the first run against an older config
asks before touching it:

```
┌ Config format changed ──────────────────────────┐
│                                                 │
│ tuisana.toml is in the version 1 format.        │
│ Six command names and one mode name have        │
│ been renamed, and uppercase letters are now     │
│ distinct keys.                                  │
│                                                 │
│ The selected projects move from [view] onto     │
│ the filter sets, which is where a selection     │
│ now lives.                                      │
│                                                 │
│ tuisana will rewrite it in the version 3 format.│
│ A backup would go to tuisana.backup.toml.       │
│                                                 │
│ y        back it up first, then migrate         │
│ n        migrate without a backup               │
│ q / esc  quit and change nothing                │
│                                                 │
└─────────────────────────────────────────────────┘
```

A version-2 file sees only the second paragraph: the window describes the
changes that are actually going to happen to the file in front of it.

- `y` copies the file byte for byte first, so the backup keeps your comments,
  key order, and formatting — none of which survive being re-serialized. An
  existing `tuisana.backup.toml` is not clobbered; the next free name is
  `tuisana.backup.2.toml`, and the window names the file it will actually
  write.
- `q` and `esc` quit without touching anything, and the prompt returns next
  time. Nothing can write the config while it is up, so a file you meant to
  edit by hand is still the file you left.

A file with no `[header]` at all is read as version 1, not as the current
version. A file from the future is rejected.

The renames, if you would rather apply them yourself:

| version 1 | version 2 |
| --- | --- |
| `mode = "task_edit"` | `mode = "column_edit"` |
| `begin_task_edit` | `edit_column` |
| `commit_task_edit` | `commit_column_edit` |
| `cancel_task_edit` | `cancel_column_edit` |
| `task_edit_next_value` | `column_edit_next_value` |
| `task_edit_prev_value` | `column_edit_prev_value` |
| `task_edit_clear` | `column_edit_clear` |

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
- Due dates render relatively (`Today`, `Tomorrow`, `Wed`, `Nov 14`). A weekday
  name means a day of the week you are in now, which turns over on Sunday, so it
  can point backwards; anything outside that week gets its date. An overdue date
  is prefixed with `!` and colored red, one due today is yellow, and one within
  three days takes the accent color.
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
- With the grid up only the characters of a written date type: digits, `-`,
  `+`, and `.`. Every letter is either a key that steers the grid or one stroke
  of a name whose other strokes are.
- `;` puts the grid away and brings it back. Hidden, the keys that steer it —
  `h`, `l`, `j`, `k`, `t`, `d` — go back to being letters, which is what lets a
  name like `tue` or `next month` be typed at all. In the grid's place the
  overlay shows the dates the text resolves to and a key to the names, since
  that is the one state they can be typed in. The choice is remembered for the
  next date you edit.
- A date range shades the days between its two ends, with both ends picked out.
  So does a name that covers more than one day: `this month` shades its month.
  The shading is the union of the two ends' spans, which is what the filter
  matches — `last week..this week` shades all fourteen days.
- The highlight sits on the edge the caret's end of the range contributes: its
  first day before the `..`, its last day after it. So the caret in `this week`
  of `this week..next week` highlights the Sunday the range opens on, and moving
  it past the `..` highlights the Saturday it closes on — in both cases the day
  the keys are about to move.
- A bare name with no `..` is edited as the *end*, and moving it spells the name
  out: nudging `this week` on a day gives `2026-08-23..2026-08-30`, keeping the
  start it came with. A name covering a single day (`today`) just becomes that
  other day. On a task's date cell, which holds one day and no range, a name
  instead highlights — and commits — the day it starts on.

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
| a weekday of the current week (Sunday-first) | `mon`, `friday` |
| a whole week, Sunday to Saturday | `this week`, `last week`, `next week` |
| a whole month | `this month`, `last month`, `next month` |
| a whole year | `this year`, `last year`, `next year` |
| an offset, in the name's own unit | `today-5`, `next month+2`, `mon+7` |
| an inclusive range | `2026-09-01..2026-09-30` |
| a range open on one side | `today..`, `..2026-12-31` |

A name that covers more than one day is a range on its own: `this month` matches
every day of the month. Written between two of them, `..` unions their spans
rather than joining their first days — each end contributes its *outer* edge, so
`last month..this month` runs from the first of one to the last of the other and
`last week..this week` covers both weeks whole. An offset steps in the unit its name is written in, so
`today-5` is five days back and `this month-1` is another spelling of
`last month`. Case and the space are ignored, so `ThisMonth` works too. `+` is
never a date separator and can offset a written date (`2026-09-15+5`); `-` is,
so it only offsets a name.

**Names are re-read every time.** A filter stores the word, not the date it
resolved to, so a saved filter of `due: this month` means October in October.
A *task* date does the opposite: typing `tomorrow` into a due-date cell sends
the day it resolved to on the spot and it never moves again, because Asana
stores a date and not an expression. A name covering a span commits as the day
it starts on there, since a task has one due date and nowhere to put the rest.

A `due` filter is also pushed to the API as `due_on.after` / `due_on.before`, so
narrowing it reduces what gets downloaded. With more than one filter set the
window sent is the *union* of theirs, and a set that is open-ended, unfiltered,
asking for an empty due date, or negating either the due row or itself opens
that side of it — otherwise the server would drop rows a set had asked for.

### Key bindings

The `[[bind]]` section maps keyboard input to command names.
If you omit a command from your config, the built-in default binding for that command still applies.

Each binding may include an optional `mode` field to restrict it to a specific UI context.
Available modes are `any` (default), `project`, `project_search`, `filter`, `filter_edit`, `calendar`, `task`, `edit`, `column_edit`, `gantt`, and `gantt_order`.

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

**Key names.** A single character is itself (`j`, `?`, `<`). Named keys are
`enter`, `esc`, `backspace`, `space`, `home`, `end`, `left`, `right`, `up`,
`down`, `pageup`, and `pagedown`. A modifier is a prefix: `ctrl-x` for Control
and `alt-x` for Option/Alt.

**Case matters on a single letter.** `J` and `j` are different keys, which is
what edit mode's `I`, `J`, `K`, `S`, and `X` are bound in. Case does *not*
matter anywhere else: `ESC` is `esc`, and `ctrl-J` is `ctrl-j` because a
terminal cannot reliably tell it from the lowercase form — binding the
uppercase form would be binding a key that never fires. (This is also why the
subtask key is `I` rather than `ctrl-i`, which is the byte `tab`.) Punctuation
that is itself shift-produced — `?`, `!`, `@`, `*`, `<`, `>`, `{`, `}` —
arrives as its own character and is unaffected.

`alt-` needs your terminal to send Option as a **modifier** rather than as an
escape prefix — in macOS Terminal, Profiles → Keyboard → "Use Option as Meta
key"; in iTerm2, Profiles → Keys → Left/Right Option key → Esc+. The
escape-prefix form is not supported: a lone `ESC` is `esc` here, and telling
the two apart by timing is how editors get famously confused. Every binding
that uses `alt-` is rebindable if your terminal cannot send it.

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
- `calendar_toggle_grid`
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
- `complete_next_candidate`
- `complete_prev_candidate`
- `highlight_next_candidate`
- `highlight_prev_candidate`
- `filter_caret_left`
- `filter_caret_right`
- `text_caret_word_back`
- `text_caret_word_forward`
- `text_caret_start`
- `text_caret_end`
- `text_cut_char`
- `text_cut_word`
- `text_cut_to_end`
- `filter_require_empty`
- `clear_search`
- `search_fuzzy`
- `search_substring`
- `search_regex`

`filter_negate_field` is bound to `ctrl-n` here and keeps that meaning — but
with a completion overlay open, where `ctrl-n` is the obvious key for "next
candidate", it walks the candidates instead. `!` on the row in filter-browse
mode is the other way to negate it.

**Column edit mode** (a table cell is open for editing)

- `commit_column_edit`
- `cancel_column_edit`
- `column_edit_next_value`
- `column_edit_prev_value`
- `column_edit_clear`
- `complete_next_candidate`
- `complete_prev_candidate`
- `highlight_next_candidate`
- `highlight_prev_candidate`
- `filter_caret_left`
- `filter_caret_right`
- `text_caret_word_back`
- `text_caret_word_forward`
- `text_caret_start`
- `text_caret_end`
- `text_cut_char`
- `text_cut_word`
- `text_cut_to_end`

**Edit mode** (the keys that change which rows exist)

- `insert_task`
- `insert_subtask`
- `insert_section`
- `delete_section`
- `move_task_to_next_section`
- `move_task_to_prev_section`
- `mark_for_deletion`
- `delete_marked_tasks`
- `edit_cancel`
- `toggle_task_selection`

**Task mode**

- `toggle_completed_filter`
- `toggle_subtask_visibility`
- `toggle_project_grouping`
- `toggle_section_grouping`
- `toggle_column_sort`
- `move_section_up`
- `move_section_down`
- `move_project_up`
- `move_project_down`
- `task_column_prev`
- `task_column_next`
- `edit_column`
- `toggle_task_completed`
- `toggle_recent_pane`
- `set_gantt_mode`
- `set_edit_mode`

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

A session starts in the project list, and **`space` is what puts a project in
play**: the task table is drawn from the selection alone, and the filter view
cannot be opened until something is selected. A project merely under the
cursor is not loaded.

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
- `b` to show or hide the named-set sidebar
- `1`-`9` to load a named set, `<`/`>` to page the list
- `w` to save the panel under a name, `y` to copy it to a new unnamed one,
  `d` to delete it
- `n` to start a completely fresh panel
- `/` to start search entry
- `ctrl-z`, `ctrl-s`, `ctrl-r` to switch search mode (fuzzy / substring / regex).
  Fuzzy is on `ctrl-z` rather than `ctrl-f` so that `ctrl-b`/`ctrl-f` can move the
  caret in every mode that edits text.

**Project search** accepts ordinary typing, `backspace`, `enter`, and `esc`.
`ctrl-l` clears the search string.

**Filter panel shortcuts** (filter browse mode):

- `j`/`k` to move between filter fields
- `enter` to start editing the selected field
- `esc` to return to task mode, leaving the panel on screen
- `f` to close the panel and go back to the task table
- `s` to cycle the string-match mode
- `e` to require the field to be empty (`ctrl-q` while editing)
- `a` to add a filter set, `x` to remove the current one, `h`/`l` to move between
- `!` to negate the selected field, `~` to negate the whole set
- `b` to show or hide the named-set sidebar
- `1`-`9` to load a named set, `<`/`>` to page the list
- `w` to save the panel under a name, `y` to copy it to a new unnamed one,
  `d` to delete it
- `n` to start a completely fresh panel
- `ctrl-l` to clear the search string
- `ctrl-z`, `ctrl-s`, `ctrl-r` to switch search mode

The panel does not close when the keys leave it. `esc`, `t`, and `m` move the
cursor to the task table with the filters still up top, unfocused — the filters
you just built are usually what you are reading the table against. `f` closes
the panel and `p` swaps the project list back in, so putting it away is still
one keystroke.

The panel's fields are `Title`, `Assignee`, `Due`, `Start`, `State`, `Projects`,
and one row per custom field. `Title` and `Assignee` match fuzzily by default —
a person's name is typed from memory and is the field most likely to be
half-remembered — and `ctrl-s` switches a row to `contains`, `ctrl-r` to regex.

`Assignee` has a fourth mode, `list`, which `s` reaches and no other row
offers: its values are a closed set of real people, so they are picked from the
same completion editor the table's cell uses rather than typed as a pattern.
Several picked people are ORed, the row's negation means "none of these", and
`me` resolves to whoever is logged in at match time — so a saved set stays
personal to whoever loads it. Everything else is unchanged: the picked names
are the row's value, and the saved schema gains only `match = "list"`.

The `Projects` filter row is not the selection and does not become it. They
answer different questions:

| control | question | when it acts |
| --- | --- | --- |
| the project selection | which projects are *asked* | before the fetch |
| the `Projects` filter row | which of the loaded tasks' memberships to keep | after it |

The row earns its keep on a task in several projects: with four projects
selected, `Projects: platform` reads "of everything I am looking at, the
platform work", and it does that *per tab*, so one tab can narrow where
another does not. The selection cannot — it is one list, and narrowing it
would unload the tasks the other tab wants.

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

It also carries **the project selection**: the projects are the first clause
of the question, and `Assignee: alex` means nothing until something says
*where*. Saving the second clause and not the first produces a set that is
only half a set — load `Sprint triage` with yesterday's projects still
selected and you get a confident, precisely filtered, wrong table.

It is deliberately *not* the sort, the grouping, the completed filter, or
subtask visibility. Those are how you are *reading* the answer rather than
what was asked. Nor is project visibility: `*` and `h` keep writing
`[[project]]`, which is one record per project for the whole app. A star is a
fact about a project; a selection is a fact about a question. `o` (selected
only) and `v` (the hidden group) stay out for the same reason — both are ways
of looking at the project *list*.

- `b` opens a `Sets` pane down the left of the top window, in the project view
  as well as the filter view, from the one toggle. Switching between `p` and
  `f` does not open or close it. It is **never focused** — `j` and `k` keep
  walking the filter fields or the project rows, and its thin border is what
  says so. On a narrow terminal it gives way rather than squeezing the pane
  beside it, and at a narrower width in the project view than in the filter
  view: a project list is one column of names, where the filter panel has
  three.
- Pinned at the top is the live panel: its name, or `unnamed`, then how many
  sets and how many active filters it holds, then how many projects are
  selected — `no projects` when the answer is none, since that line is then
  the explanation for an empty table. Below the rule are the saved entries in
  name order, numbered from the top of the visible window.
- `1`-`9` load the entry at that position. `<` and `>` page the window when
  there are more entries than fit.
- Loading over an **unnamed panel that is filtering** asks first, in a
  centered window that says what goes and what stays: `y` goes through with
  it, `n` or `esc` backs out. A bound panel is already on disk, and an empty
  one has nothing to lose, so neither is worth a keypress to confirm. The
  test is the filters alone — a selection is nearly always there, and making
  the common opening move ask a question every time would spend the prompt's
  credibility on the case that is never a mistake. The window does name the
  projects the entry will select, and points at `u`, when the entry carries
  them.
- `w` opens a one-line prompt on the sidebar's border, pre-filled with the
  loaded name. `enter` commits, `esc` cancels. An existing name is
  overwritten; a new one is created. A blank name is refused. Naming a thing
  is not a decision worth a window; the two that discard something are, so
  those are the ones that get one.
- `d` deletes the **loaded** entry after the same confirmation, and unbinds
  the panel. It is refused when nothing is loaded.
- `n` throws the panel away and starts from nothing: one empty set, every row
  back to the match mode it was built with, nothing bound, and **nothing
  selected** — the projects are part of what there is to start from. It
  therefore leaves you in the project view, where the next set's projects get
  picked. Unlike `a`, which keeps the match modes so a second set inherits
  them, this is a blank slate. `u` puts the selection straight back. The entry
  you were on keeps whatever was last written to it.

**All seven keys work in the project view too** — `b`, `1`-`9`, `<`/`>`, `w`,
`y`, `n`, and `d` — and a digit pressed there leaves the keys there. What does
not come across is the per-tab half of the panel: `a` and `x` add and remove a
tab, `h` and `l` move between them, `~` negates one, and `e`, `s`, and `!` act
on a filter row. Those are about fields, and there are no fields and no tab
strip in the project view.

**Nothing is in play until you select it.** A project under the cursor is not
loaded; the task table is drawn from the selection and nothing else. With
nothing selected the **filter view cannot be opened** — `f` says so and leaves
you in the project list — because a filter panel with no projects is a
question with no subject, and its rows are built from the loaded projects'
custom fields. That is the state a fresh config, a migrated one, and `n` all
start from, and picking a project is what leaves it.

**Loading a set moves the selection.** A digit replaces the panel, replaces
the selection, and starts the fetch the new selection implies — or marks the
table out of date when it is closed, exactly as changing the selection by hand
does. An entry that names no projects loads to nothing selected and leaves the
keys in the project view.

The load pushes the selection history, so `u` undoes it and `ctrl-y` redoes
it, like every other selection change you made this session. `u` undoes the
selection *alone*: it does not unload the set, and the entry then has the
undone selection written back to it, because a bound entry is a file you have
open and `u` is an edit like any other. The two halves have different undo
stories because only one of them ever had an undo.

**The selection writes through as well** — and it does so whether or not the
panel has a name. `space` on a project while `Sprint triage` is loaded edits
`Sprint triage`, on disk, the same way typing into a filter field does; so do
`a`, `!`, `@`, `i`, `c`, `u`, and `ctrl-y`. With nothing bound, the same
change goes to the reserved `scratch` entry instead, which is what brings your
projects back next launch without your having named anything.
The project pane's border names the bound entry for exactly this reason: the
sidebar says the same thing, but the sidebar is the thing that can be turned
off. One write per settled burst, and it moves `[[filter_set]]` and `[view]`
together.

**Loading binds the panel to the name.** From then on the entry is a live view
rather than a snapshot: type into a field, add a tab, negate a set, and the
named entry changes with it, on disk. A named set behaves like a file you have
open, not like a clipboard — the alternative, a snapshot you have to remember
to re-save, is the one that loses work. The write happens once per settled
burst of typing, not once per keystroke, and an edit still in hand is flushed
before a load or `n` replaces the panel it was typed into.

That is also why loading over a *bound* panel asks nothing: there is no
unsaved state for it to lose. The confirmation exists for the one case that
has some — an unnamed panel you have been building up.

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
there is a separate key for "has no value": `e` on a filter row, or `ctrl-q`
while editing one. (`ctrl-e` used to do this; it is now "end of line",
which is what it means everywhere else text is edited.) The row's value shows as `(none)`, it counts towards the
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
- `alt-b`/`alt-f` to move a word, `ctrl-a`/`ctrl-e` to jump to the ends
- `backspace` to delete the character before the caret
- `ctrl-d` to cut the character at the caret, `alt-d` to cut the word ahead of
  it, `ctrl-k` to cut to the end of the line. All three put what they removed on
  the system clipboard, so a cut can be pasted back
- `enter` to confirm the edit and return to filter browse mode
- `esc` to discard the edit, close the panel, and return to task mode
- `ctrl-z`, `ctrl-s`, `ctrl-r` to switch match mode, `ctrl-l` to clear
- `ctrl-n` to negate the field, `ctrl-t` to negate the set, `ctrl-q` to
  require it empty

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
- `s` to sort by the column under the cursor: descending, then ascending, then
  unsorted. Sorting a second column keeps the first as its tie-break, and the
  column sorted most recently is the one the rows read by — the header marks
  say which way each one goes and, once there are two, which of them wins. An
  empty cell sorts last whichever way the column runs, with one exception: a
  task with no start date sorts by its due date, since a task Asana let you
  save without a start is one that starts the day it is due
- `[` and `]` to move by section
- `{` and `}` to move by project
- `h`/`l` to move the column cursor, `enter` to edit the cell under it
- `o` to open the selected task in Asana
- `d` to mark the task — or the whole selection — done
- `g` to draw the Gantt chart and switch to gantt mode

### Editing tasks

The task table is writable. `h` and `l` walk a **column cursor** left and right
through `Task`, `Assignee`, `Due`, `Start`, `State`, `Projects`, `Parent`, and
whatever custom fields the loaded projects carry. The column under it is banded
in the header, the cell under it on the cursor row is underlined, and moving
onto a column that is off to the right scrolls the table to it.

The cursor is only drawn in task, edit, and column-edit modes. In gantt mode
`h` and `l` already scroll the timeline, and a cursor you cannot move is a
cursor that lies about what the keys do.

`enter` opens the cell under the cursor. Which editor you get depends on the
column, and each one is the filter panel's editor for that kind of field:

| Column | Editor | Keys |
| --- | --- | --- |
| `Task`, text and number custom fields | a text field with a caret | type, `backspace`, the motions above, `ctrl-d`/`alt-d`/`ctrl-k` to cut |
| `Due`, `Start` | the date picker | `h`/`l` day, `j`/`k` month, `t` today, `d` clear |
| `State`, enum custom fields | a value picker | `j`/`k` through the options, `d` for no value |
| `Assignee`, `Projects`, `Parent` | completion over the names that exist | type to rank, `tab`/`shift-tab` to complete, `ctrl-n`/`ctrl-p` to walk, `backspace` to delete an item, `ctrl-l` to clear |

The three cuts are the deletions the word and line motions imply, and they work
in every field that has those motions — the filter panel and the cell editor
alike. Each one takes what it removed to the system clipboard. In a completing
field they cut the text being typed and leave the items already picked alone,
which is what `backspace` is for.

`enter` commits and `esc` throws the edit away, exactly as in the filter panel.
Two differences, both because a task holds a value where a filter holds a
*query*: a date is one day rather than a range (`2026-09-01..2026-09-08` is
refused), and a value picker holds one value rather than a list.

A picker offers the options the custom field **declares**, not the values tasks
happen to carry — the whole point may be to be the first task marked `Blocked`.

**`Assignee`, `Projects`, and `Parent` complete.** All three are references to
something with a name and a gid, so all three get the same editor: a list of
items, typed with completion over the names that actually exist. The candidates
appear in an overlay positioned like the date picker, because the cell is far
too narrow to list names in.

The list is **ranked, not filtered**, and matches a **subsequence** rather than
a substring: `shp rc` finds `Ship the release candidate`, and `alex` puts
`Alex Chen` above `Alexandra Pemberton-Clarke`. A contiguous run, a match at a
word boundary, and a match at the very start all score better, which is what
keeps a long directory readable without typing much of it.

Two ways to pick, and they compose:

- `tab` / `shift-tab` **complete** the typed prefix to the next candidate,
  walking them with the prefix intact — so a wrong first match costs one more
  keystroke instead of an undo. Type, `tab` until it reads right, `enter`.
- `ctrl-n` / `ctrl-p` **walk the highlight** without touching the text, and
  `enter` takes whatever it landed on. With nothing typed, that is the whole
  interaction.

Nothing is entered that is not a candidate: a typed prefix that matches nothing
commits nothing and says so.

- **Assignee** is a list capped at one, so typing a second name replaces the
  first. Committing it empty unassigns the task. The candidates are the
  workspace directory, fetched once per session, plus `me` and whoever the
  loaded tasks name — the fallback when the directory is unavailable. An email
  address is still accepted as typed, which is how you assign someone the
  workspace list does not cover.
- **Projects** is a list of any length, completing over every project the
  session loaded. A commit becomes one `addProject` or `removeProject` request
  per change. Asana will not store a task in no projects at all, so committing
  an empty list is refused with a message rather than sent.
- **Parent** is capped at one, like `Assignee`: a task hangs off one other
  task. It shows the parent's **title** — including when the parent is not a
  row on screen, because a filter can drop a parent out from under its
  children and a gid is not a name. The title travels with the subtask from
  the moment it is loaded, so it survives any filter; only a parent the
  session has never loaded at all falls back to the bare gid, which is the
  same fallback `Projects` makes for a project it never saw. The cell is empty
  for a top-level task. Clearing it promotes a subtask to a top-level task. Its candidates are every loaded task, ordered
  by **the cursor task's own section first, then the rest of its project, then
  everything else** — so with nothing typed the parent you want is usually at
  the top, because it is usually a few rows up. The task itself and all of its
  descendants are never offered: that would be a cycle Asana would reject
  anyway, and a refusal from the API is a worse answer than a list that never
  had it. A bulk re-parent is allowed — unlike a title, "these five are
  subtasks of that one" is a sentence someone means.

### Recently edited

Reassign a task while filtering on one assignee, or move it out of the project
you are looking at, and the row leaves the table under the cursor — at the one
moment the edit is least finished. A pane between the project list and the
table holds those tasks:

- A task is in it when it was edited this session **and** the current view no
  longer shows it. Edit it back into view — or change the filter — and it
  leaves on the next rebuild.
- It lists the same columns as the table, in the same widths, newest edit
  first, and is not persisted.
- The cursor moves into it when a rebuild would otherwise lose the task it was
  following, and the pane shows itself when that happens whatever the toggle
  says. `j` and `k` walk between the two lists; `enter`, `d`, and `space` work in
  the pane exactly as they do in the table.
- `b` shows or hides it. With nothing in it, it is not drawn at all.

**`d` marks it done.** Completion is a field like any other, but it is also the
most common edit in the app, so it gets a key of its own: `d` from anywhere in
the row toggles between open and done.

**An edit is an edit of everything selected.** With tasks selected it applies to
all of them; with nothing selected it applies to the cursor row. The editor
opens on the cursor row's value even when the selection holds several different
ones, and the pane border says `editing 3` so the blast radius is on screen
before you commit. The targets are fixed when the editor *opens*, so a table
that rebuilds underneath you cannot widen what `enter` is about to change.

On a selection, `d` is not a per-task toggle: every target is set to the
opposite of the reference row's state, so a mixed selection ends up uniform and
a second press puts it back.

**Titles are the exception.** Setting three tasks to the same title is not a
bulk edit, so `enter` on the `Task` column with more than one task selected is
refused, and says why.

**What goes over the wire.** An edit is applied locally the moment you commit
it and sent to Asana in the background, one request per task, so the table does
not freeze while twelve tasks are marked done. If a request fails, that task's
field is put back the way it was and the pane border says so:
`could not update 1 of 12: backend error: 403`.

Two consequences worth knowing before they surprise you:

- **An edit can make a row vanish.** Re-assign a task while filtering to `alex`
  and it leaves the table. That is the filter doing its job; the cursor lands
  on the next row, as it does after any rebuild.
- **An edit does not re-fetch.** Moving a due date outside the window pushed
  down to Asana does not hide the task — it is already in the cache, and the
  cache is what the table is built from.

Not yet editable: a task's position *within* a section, and `date`,
`multi_enum`, and `people` custom fields. Each says so rather than doing
nothing.

### Edit mode

Creating a task, re-parenting it, moving it between sections, and deleting it
are the edits that change *which rows exist*. They share a problem the field
edits never had — the target does not exist yet, or is about to stop existing —
and they would each want a key in task mode, which is full. So they get a mode
of their own.

`t` from task mode enters **edit mode**; `esc` leaves. Same pane, same rows,
same cursor and selection, different keys. Everything that reads the table —
sort, grouping, the completed filter, the column cursor, the Gantt chart —
stays in task mode. The split is not "safe keys and dangerous keys"; it is
"keys about what the table *says* and keys about what it *contains*".

| key | what it does |
| --- | --- |
| `i` | a new task beside the one under the cursor |
| `I` | a new subtask of the one under the cursor |
| `x` | mark the task — or the selection — for deletion |
| `X` | delete the first empty section of the cursor row's project |
| `S` | a new section after the cursor's section |
| `J` | move the task to the next section down |
| `K` | move the task to the previous section up |
| `space` | toggle selection, as in task mode |
| `enter` | carry out the marked deletions |
| `esc` | clear the marks, or leave edit mode when there are none |

The case split is the whole mnemonic: **lowercase acts on the task, uppercase
acts on the structure around it.** `i`/`I` is a task and a nested task, `x`/`X`
is a task and the section holding it, `j`/`k` moves the cursor and `J`/`K`
moves the task.

Because the structural keys live here, `i` and `x` go back to meaning insert
and delete — and they still mean `invert_task_selection` and
`clear_task_selection` in task mode, because the two modes no longer compete
for them. Edit mode keeps the global keys, so `j`, `k`, `ctrl-u`, `ctrl-d`,
`home`, `end`, `r`, `q`, and `?` all work; it does *not* inherit task mode's
own, so `[` and `]` resize the top pane here rather than moving by section.

**Creating a task.** `i` opens a **draft**: a local row, directly after the
cursor row, with the `Task` cell already open — so `i` costs one keystroke
before you start typing. Nothing is sent until `enter`, and `esc` or an empty
title throws it away with nothing having happened. `i`, a title, `enter`, and
again is a run of new tasks.

A draft is exempt from the filters, because a row with no title cannot match a
`Title` filter and a task that vanished the instant you created it would not be
a feature.

What it inherits is one sentence: **a field is inherited when the view would
have shown it, and left blank when it would not.**

| what | `i` | `I` |
| --- | --- | --- |
| project | the cursor row's project, when project grouping is on **or** the `Projects` column is on screen | not sent — see below |
| section | the cursor row's section, when section grouping is on | not sent — a subtask belongs to its parent |
| parent | the cursor row's parent, so a new task beside a subtask is a sibling subtask | the cursor row itself |

**A subtask is sent with its parent and nothing else.** Naming the project as
well would make it a *direct member* of that project, and Asana files a new
project member in the project's **first section** — so the subtask would come
back as a top-level row under an arbitrary heading, which is exactly what it is
not. The same reasoning as the section, one level up. A subtask created beside
a sibling (`i` on a subtask) is sent the same way.

If you do have subtasks that are direct project members — made in Asana, or by
an earlier version of tuisana — they are shown the way Asana shows them: as
top-level rows in the section they were filed in. tuisana reads the project's
task list and does not second-guess it.

The "would have shown it" test matters. Grouping off and the `Projects` column
scrolled out of view means the screen never said which project the cursor row
was in, and filing a new task there would be a guess you cannot see being made.
A blank project is recoverable — the cell is editable — where a wrong one is a
task filed somewhere nobody will look. With nothing to inherit the task is a
bare workspace task; with no project loaded at all there is no workspace to
name it in either, and the insert is refused with a message.

This is the one write in the app that is **not** optimistic. The other edits
have a local value to show while the request is in flight; a create has a gid
it does not know yet, and every subsequent edit of that row needs it. A create
that fails leaves the draft and its title on screen, because the alternative is
losing what you just typed to a 403.

**Sections.** `S` opens a draft section — the same shape as a draft task, a
heading row being typed, with `enter` to create and `esc` to discard.

`X` is how you take one back. Asana deletes a section only when it holds no
tasks, and a section with no tasks has no row for the cursor to stand on — the
table draws a heading only where there is something under it. So `X` takes the
cursor row's **project** and deletes the first section of it that is empty, in
the order the project puts them; the notice names the one that went, and
repeating it clears them one at a time. A populated section is never sent,
which is the point: Asana's own refusal is a worse error than saying so here.

`X` does not join the mark-and-confirm flow below. That exists because deleting
tasks destroys work, and an empty section holds none.

`J` and `K` move the task under the cursor between its project's sections, in
the order the project puts them, appending it to the target. Off the end in
either direction does nothing — there is no "no section" position below the
last one. They are refused, with a message, on a subtask (subtasks are not in
sections), when the project has only one section, and when no project can be
resolved for the row: section membership is per project, so a task in several
projects moves within the one it is *grouped under*, and with grouping off
there is no such answer.

**Deleting tasks.** Deletion is the one edit with nothing to put back, so it is
the one edit that is not a single keystroke.

- `x` marks the task under the cursor, or **every selected task** when there is
  a selection. `x` again unmarks.
- A marked row is drawn **struck through** — a glyph change, not a colour
  change, so it reads under `variant = "mono"` too — and the pane border says
  `deleting 3`, beside where it says `editing 3`.
- **A marked parent takes its subtasks with it.** Asana does that server-side,
  so its loaded subtasks are struck through too: they are going, and a
  confirmation that did not show it would be a confirmation of the wrong thing.
  One request is still sent, for the parent.
- `esc` clears the marks and stays in edit mode. With no marks, `esc` leaves
  the mode. One key, one sentence: back out of whatever is pending.
- `enter` deletes them, one request per marked task. The rows go at confirm
  time and come back if a request fails, with the usual
  `could not delete 1 of 3: …` in the corner. The cursor lands where the first
  deleted row was.

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
projects = ["me", "1201", "1202"]

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
- `projects` is the selection the set is asked of, by GID, with `me` for the
  assigned-to-me row — `me` rather than that row's GID because the GID is
  whoever is logged in, and a shared file has to mean "my tasks" for each
  reader. One list per entry rather than one per tab: tabs OR, and a fetch
  scope cannot.
- An entry that names no projects — one hand-written without the key, or one
  migrated from an older file — loads to **nothing selected**, which is the
  state that asks you to pick some. `w` always writes the key, so only those
  two cases are silent. A project named twice is collapsed rather than
  refused.
- `scratch = true` marks the app's own slot for an **unnamed** panel's
  selection. It is written for you, kept out of the `Sets` sidebar, and never
  addressed by a digit or found by name: it exists so a selection made without
  naming a set survives a restart, now that `[view]` has nowhere to keep one.
  It holds a selection and nothing else — an unnamed panel's *filters* still
  exist nowhere but on screen, which is what the discard confirmation is
  about. At most one entry may set it.
- A GID the workspace no longer returns is skipped when the set is applied and
  **kept on disk**: applying a set is not consent to rewrite it, and a project
  you lost access to for a week should not cost you the entry. It goes when
  you next change the selection while the entry is loaded, which is a
  deliberate edit of that entry.
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

### Saved view state

The optional `[view]` section is where tuisana leaves the screen as you found
it. It is written by the app after any keystroke that changes what is open,
and read back once at startup. You can edit it by hand; the next keypress that
moves a pane rewrites it.

```toml
[view]
filter_set = "Sprint triage"   # the named entry the filter panel is bound to
top_pane = "normal"            # "normal" | "minimized" | "maximized"
tasks = true                   # the task table is open
filters = true                 # the top pane holds the filter panel
filter_sidebar = false         # the `Sets` sidebar beside it
recent = true                  # the recently-edited pane is switched on
```

- **There is no `projects` key.** The selection lives on a `[[filter_set]]`
  and nowhere else: the entry `filter_set` names, or — when it names none —
  the `scratch = true` entry an unnamed panel writes to. A version-1 or -2
  file's `projects` list is moved onto one of those two on migration, so
  nothing is lost in the upgrade.
- A `filter_set` naming an entry that has since been deleted by hand restores
  **nothing selected** rather than falling back to the scratch slot: that
  selection belongs to a different panel, and inheriting it would quietly ask
  a saved question of projects the saved question never named.
- Restoring a selection with `tasks = true` starts the fetch for it.
- `filter_set` must name a `[[filter_set]]` entry. The panel comes back
  **bound** to it, so editing a field still writes through. An unnamed panel
  records nothing here, and a name whose entry has been deleted is ignored.
- `top_pane` is the only size that is kept, and only as one of three states.
  The height you resized a pane to is not saved: a pane sized to one terminal
  is noise in another. `"minimized"` starts in the task table, the same way
  `{` moves there as it minimizes — there would be nothing else on screen.
- Everything else stays out, for the same reason the Gantt chart's timeline
  window does. Scroll offsets, the cursor, the sort, the grouping, the
  completed filter, and which pane had focus are where you were looking, not
  what you had set up.

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
