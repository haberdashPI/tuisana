# Gantt colours

```toml
[gantt]
visible = false          # draw the chart at startup
columns = 2              # table columns kept visible beside the chart
color_by = "assignee"    # "assignee" | "section" | "state" | "field:<Name>"

[gantt.order]
assignee = ["Alex Chen", "Morgan Ellis"]
"field:Priority" = ["High", "Medium", "Low"]
```

`[gantt]` is where the colour dialog's result is stored. Committing the dialog
writes `color_by` and that dimension's order back here; nothing else in the
section is ever written by the app.

You can write all of it by hand, but you only *need* to for `visible` and
`columns` — the in-app keys set everything else.

## Keys

### `visible`

`true` draws the chart when the app starts. Defaults to `false`.

### `columns`

How many table columns stay visible beside the chart. Defaults to `2`.

### `color_by`

Which dimension the bars take their colour from.

| value | dimension |
| --- | --- |
| `"assignee"` | who the task is assigned to (the default) |
| `"section"` | the section it sits in |
| `"state"` | open or done |
| `"field:<Name>"` | an enumerated custom field |

A custom field is written `field:Priority` rather than bare, so a field
actually called "Section" cannot shadow the built-in dimension. Only
**enumerated** custom fields — ones with a small fixed set of values — can be
cycled to in the app.

### `[gantt.order]`

One list per dimension, keyed exactly the way `color_by` spells it.

Values a list names are coloured first, in that order; anything else follows
alphabetically. That is what keeps re-sorting or filtering the table from
repainting a bar.

**Exactly the first six values get a colour of their own.** Everything after
that shares a neutral one. The colour dialog draws a rule where the palette
runs out, and an unset value — an unassigned task, a task with no section — is
always neutral and can never be moved above the rule.

The map is stored sorted, so a config the app rewrites has a stable key order
and does not churn in version control.

## What is not saved

The chart's **timeline window** — where it is scrolled to and how far it is
zoomed. That is view state like the sort and the grouping.

`visible` and `columns` only set where a session *starts*; changing them in the
app does not write back here.
