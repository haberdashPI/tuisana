# Named filter sets

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

A `[[filter_set]]` entry is a whole filter panel saved under a name — every
tab, each tab's negation, each field's query, match mode, require-empty flag,
and negation — **plus the project selection it is asked of**.

Normally written by the app: `w` saves the panel, and every subsequent edit
writes through while the entry is loaded. It is readable and writable by hand
all the same.

## Entry keys

### `name`

Required and non-empty. No two entries may have names that differ only in
case — the sidebar lists them by number, and two rows reading the same is a
trap.

### `projects`

The selection the set is asked of, as a list of Asana project GIDs.

- `"me"` stands for the **assigned-to-me** row, rather than that row's GID.
  The GID is whoever is logged in, and a shared file has to mean "my tasks" for
  each reader.
- It is **one list per entry, not one per tab**. Tabs OR; a fetch scope cannot.
- A project named twice is collapsed rather than refused.
- An entry naming **no** projects loads to nothing selected, which is the state
  that asks you to pick some. `w` always writes the key, so only a hand-written
  entry or one migrated from an older file is silent here.
- A GID the workspace no longer returns is **skipped on load and kept on
  disk**. Applying a set is not consent to rewrite it, and a project you lost
  access to for a week should not cost you the entry. It goes when you next
  change the selection while the entry is loaded, which is a deliberate edit.

::: tip Why the projects travel with the filter
The projects are the first clause of the question. `Assignee: alex` means
nothing until something says *where*. Saving the second clause and not the
first produces a set that is only half a set — load `Sprint triage` with
yesterday's projects still selected and you get a confident, precisely
filtered, wrong table.
:::

### `scratch`

`scratch = true` marks the app's own slot for an **unnamed** panel's selection.

It is written for you, kept out of the `Sets` sidebar, and never addressed by a
digit or found by name. It exists so that a selection made without naming
anything survives a restart, now that `[view]` has nowhere to keep one.

It holds a selection and nothing else — an unnamed panel's *filters* still
exist nowhere but on screen, which is what the discard confirmation is about.
At most one entry may set it.

## `[[filter_set.set]]` — one tab

Fields **AND** within a tab; tabs **OR** between them. A task is shown when it
satisfies any tab.

| key | meaning |
| --- | --- |
| `negated` | `true` inverts the whole tab, *after* its fields have ANDed |

A negated tab is `not (a and b)`, which is a different statement from negating
each row. With `Assignee: alex` and `Due: August`, negating the rows asks for
tasks that are neither Alex's nor in August; negating the tab also keeps Jo's
August task.

A negated *empty* tab matches nothing rather than everything, so adding a tab
can still only widen the result.

## `[[filter_set.set.field]]` — one row

| key | meaning |
| --- | --- |
| `key` | which row: `title`, `assignee`, `due`, `start`, `state`, `projects`, or `custom:<Name>` |
| `query` | the value typed into the row |
| `match` | `fuzzy`, `contains`, `regex`, or `list` — text rows only, omitted at the row's default |
| `empty` | `true` is "has no value", the filter `e` sets |
| `negated` | `true` inverts that row's verdict alone |

**Custom fields are keyed by name** (`custom:Priority`), so an entry survives a
reload that brings different Asana ids. A `key` matching no row in the loaded
data is **kept rather than rejected** — it is re-resolved each time a project's
fields arrive. Loading a set with the wrong projects selected cannot quietly
erase half of it.

**`empty` is ignored on a row that cannot be empty**, such as `state`: a task
is always either open or done.

What "empty" means depends on the field:

| field kind | empty means |
| --- | --- |
| text (`title`, `assignee`, `projects`, text custom fields) | the value is missing or all whitespace |
| labels (custom fields with a small value set) | the task carries no value for the field |
| date (`due`, `start`) | the task has no such date |

**`match = "list"`** is offered on `assignee` alone. Its values are a closed
set of real people, so they are picked from a completion editor rather than
typed as a pattern. Several picked people are ORed, the row's negation means
"none of these", and `me` resolves to whoever is logged in **at match time** —
so a saved set stays personal to whoever loads it.

## The `Projects` row is not the selection

They answer different questions, and the file keeps them apart for that reason:

| control | question | when it acts |
| --- | --- | --- |
| `projects` on the entry | which projects are *asked* | before the fetch |
| a `key = "projects"` field | which of the loaded tasks' memberships to keep | after it |

The row earns its keep on a task in several projects: with four projects
selected, `Projects: platform` reads "of everything I am looking at, the
platform work" — and it does that *per tab*, so one tab can narrow where
another does not.

## How loading and saving behave

**Loading binds the panel to the name.** From then on the entry is a live view
rather than a snapshot: type into a field, add a tab, negate a set, and the
named entry changes with it, on disk. A named set behaves like a file you have
open, not like a clipboard — the alternative, a snapshot you have to remember
to re-save, is the one that loses work.

**The selection writes through too**, whether or not the panel has a name.
Selecting a project while `Sprint triage` is loaded edits `Sprint triage` on
disk. With nothing bound, the change goes to the `scratch` entry instead.

**Loading moves the selection** and starts the fetch it implies. The load
pushes the selection history, so undo and redo work on it like any other
selection change. Undo acts on the selection *alone*: it does not unload the
set, and the entry then has the undone selection written back to it.

**Loading over an unnamed panel that is filtering asks first.** A bound panel
is already on disk and an empty one has nothing to lose, so neither costs a
keypress. The test is the filters alone — a selection is nearly always there,
and asking every time would spend the prompt's credibility on the case that is
never a mistake.

**Two escapes.** Copy-to-new keeps exactly what is on screen as a fresh unnamed
panel while the entry keeps whatever was last written to it — this is how you
take a saved filter as a starting point and go somewhere else with it. Saving
under a new name rebinds to that one.
