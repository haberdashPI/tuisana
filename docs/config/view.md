# Saved view state

```toml
[view]
filter_set = "Sprint triage"   # the named entry the filter panel is bound to
top_pane = "normal"            # "normal" | "minimized" | "maximized"
tasks = true                   # the task table is open
filters = true                 # the top pane holds the filter panel
filter_sidebar = false         # the `Sets` sidebar beside it
recent = true                  # the recently-edited pane is switched on
```

`[view]` is where tuisana leaves the screen as you found it. It is written
after any keystroke that changes what is open, and read back once at startup.

It records **what is open, never how big it is** — `top_pane` is the one
exception, and even that is three named states rather than a height.

## Keys

### `filter_set`

The named [`[[filter_set]]`](/config/filter-sets) entry the filter panel is
bound to. The panel comes back **bound**, so editing a field still writes
through.

- Left out when the panel is unnamed.
- A name whose entry has since been deleted by hand is ignored, and restores
  **nothing selected** — rather than falling back to the scratch slot. That
  selection belongs to a different panel, and inheriting it would quietly ask a
  saved question of projects the saved question never named.

### `top_pane`

`"normal"`, `"minimized"`, or `"maximized"`. Defaults to `"normal"`.

`"minimized"` starts in the task table, the same way minimizing moves there —
there would be nothing else on screen.

The height you resized a pane to is **not** saved: a pane sized to one terminal
is noise in another.

### `tasks`

`true` opens the task table. Restoring a selection with `tasks = true` starts
the fetch for it.

### `filters`

`true` means the top pane holds the filter panel rather than the project list.

### `filter_sidebar`

`true` draws the `Sets` sidebar down the left of the top pane. The sidebar
appears in the project view as well as the filter view, from one toggle.

### `recent`

`true` switches on the recently-edited pane. On unless it was toggled off; it
holds what *this* session edited, so it starts empty either way.

## There is no `projects` key

The selection lives on a [`[[filter_set]]`](/config/filter-sets) and nowhere
else: the entry `filter_set` names, or — when it names none — the
`scratch = true` entry an unnamed panel writes to.

A version-1 or -2 file's `projects` list is moved onto one of those two during
[migration](/reference/migrations), so nothing is lost in the upgrade.

## What stays out

Scroll offsets, the cursor, the sort, the grouping, the completed filter, and
which pane had focus. Those are where you were looking, not what you had set
up — the same reason the Gantt chart's timeline window is not saved either.
