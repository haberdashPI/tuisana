# Milestone 15: Create and delete tasks and sections

[← all milestones](../plan.md)

Goal:

- split the structural edits out of task mode into an **edit mode** of their own
- create tasks and subtasks where the cursor is standing
- make the supertask an editable column
- move a task between sections, and add and remove sections
- delete tasks, deliberately, in a marked batch rather than one keystroke at a time
- give every completion list fuzzy ranking and a pair of keys that walk it
- version the config file, and migrate the old one with the user's consent

## 1. Why these together

[Milestone 14](14-edit-tasks.md) made the table writable and
[14.5](14.5-assign-people-and-projects.md) finished the two reference fields.
The README's "not yet editable" list is now almost entirely one thing: **where
a task sits**. Creating it, re-parenting it, moving it between sections, and
deleting it are the edits that change which rows exist and how they nest, and
they share a problem the field edits never had — the target does not exist yet,
or is about to stop existing.

They also share a cost. Each one wants a key in task mode, and task mode is
full. Rather than displace four reading keys for four writing keys, the
structural edits get a mode (§2), which is what makes the rest of this
milestone cheap in keys and clear in the help overlay.

The mode rename that falls out of this (§2.1) is the first change to break an
existing `tuisana.toml`, which is why the config finally gets a version number
and a migration (§3) rather than a line in the changelog.

The fuzzy-ranking work (§10) rides along because the supertask picker is the
first completion field whose candidate list is *long*. `Assignee` is a
workspace directory of tens; `Projects` is a list you chose yourself. A parent
picker offers every loaded task, and a substring filter over hundreds of
titles, ordered by "prefix first, then whatever order they loaded in", is not a
picker. Fixing it fixes the other two for free.

## 2. Edit mode

`Mode::Edit`, a sibling of `Mode::Task` the way `Mode::Gantt` is: the same
pane, the same rows, the same cursor and selection, different keys. `t` from
task mode enters it; `esc` leaves.

Everything that **creates or destroys a row** lives here. Everything that reads
— sort, grouping, the completed filter, the column cursor, the Gantt chart —
stays in task mode. The split is not "safe keys and dangerous keys"; it is
"keys about what the table *says* and keys about what the table *contains*".

Three things follow from it:

- **`i` and `x` go back to meaning insert and delete**, which is what they mean
  in every editor. They are still `invert_task_selection` and
  `clear_task_selection` in task mode, untouched, because the two modes no
  longer compete for them.
- **`enter` is free to mean "do the pending thing"**, which is what makes the
  marked-deletion flow in §9 work without a third mode.
- The hint bar and help overlay get a short, honest list. Task mode's help is
  already long enough that a user has to read it rather than glance at it.

Edit mode allows the `Any` fallback, so `j`, `k`, `ctrl-u`, `ctrl-d`, `home`,
`end`, `r`, `q`, and `?` keep working — moving the cursor is most of what you
do between edits. It does **not** inherit task mode's own bindings: `[` and `]`
resize the top pane here rather than moving by section, and `h`/`l`, `c`, `s`,
`z`, and `g` are off.

`space` is bound here as well as in task mode, because §9 acts on the
selection and building one is part of the flow.

### 2.1 The rename

There is already a `Mode::TaskEdit`, and it means something narrower: a single
table cell is open for typing. Two modes called "edit" in the same pane is a
help overlay nobody can read, so the narrower one is renamed after what it
actually edits — **a column** — and the name "edit" is freed for the mode this
milestone adds.

| v1 | v2 |
| --- | --- |
| `mode = "task_edit"` | `mode = "column_edit"` |
| `begin_task_edit` | `edit_column` |
| `commit_task_edit` | `commit_column_edit` |
| `cancel_task_edit` | `cancel_column_edit` |
| `task_edit_next_value` | `column_edit_next_value` |
| `task_edit_prev_value` | `column_edit_prev_value` |
| `task_edit_clear` | `column_edit_clear` |
| — | `mode = "edit"` (new) |

`Mode::TaskEdit` becomes `Mode::ColumnEdit`, and its badge reads `column`.
`edit_column` keeps `enter` in task mode and keeps opening the cell under the
column cursor; only the name changes.

This breaks every existing `tuisana.toml` that binds any of them — an unknown
command is a hard startup error today, not a warning — so it comes with a
config version and a migration, which is §3.

### 2.2 The keys

| key | what it does |
| --- | --- |
| `i` | a new task beside the one under the cursor |
| `I` | a new subtask of the one under the cursor |
| `x` | mark the task — or the selection — for deletion |
| `X` | delete the section under the cursor, when it is empty |
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

## 3. Config version 2, and the migration

`header.version` goes from `1.0` to `2.0`. Version 1 is still *readable*; it is
no longer writable. Every save emits version 2, so a config is migrated once
and never drifts back.

### 3.1 What the migration does

Applied to a version-1 file, in memory, after parsing and before validation:

- every `[[bind]]` whose `mode` or `command` appears in the §2.1 table is
  rewritten to its version-2 name
- every `key` that is a single letter is **lowercased**. In version 1 the
  reader lowercased it anyway, so `key = "G"` meant `g`; §4 makes that letter a
  different key, and a migration that left it alone would silently rebind it
- `header.version` is set to `2.0`

Nothing else changes. Unknown commands are left exactly as they are and still
fail validation: the migration renames what was renamed, and is not a licence
to start accepting typos.

A file with no `[header]` at all is treated as **version 1**, not as the
current version. `default_header_version` returns the version the app writes,
which is the wrong answer for a file the app has never written — so the
deserialized field becomes `Option<f64>` and "absent" means "unversioned, so
the oldest". A file from the future is still rejected, as it is today.

### 3.2 The prompt

A migration rewrites a file the user hand-wrote, so it asks first. On the first
run against a version-1 config, before the project list is drawn and **before
anything may write**, a centered window — the same shape as the one that guards
loading over an unnamed filter panel:

```
┌ Config format changed ─────────────────────────────────┐
│                                                        │
│  tuisana.toml is in the version 1 format. Six command  │
│  names and one mode name have been renamed, and        │
│  uppercase letters are now distinct keys.              │
│                                                        │
│  tuisana will rewrite it in the version 2 format.      │
│                                                        │
│  y  back it up to tuisana.backup.toml, then migrate    │
│  n  migrate without a backup                           │
│  q  quit and change nothing                            │
│                                                        │
└────────────────────────────────────────────────────────┘
```

- `y` copies the file **byte for byte** — a copy, not a re-serialization, so
  the backup keeps the user's comments, key order, and formatting, none of
  which survive `toml::to_string_pretty`. Then it writes the migrated config
  and carries on.
- `n` migrates with no backup.
- `q` and `esc` quit without touching the file. A user who would rather edit it
  by hand can, and the prompt returns next time.
- If `tuisana.backup.toml` already exists it is not clobbered:
  `tuisana.backup.2.toml`, and so on. The window names the file it will
  actually write, so the choice is made with the right name on screen.
- A missing config file, or one already at version 2, shows nothing.

The "before anything may write" is load-bearing. `stage_view_state` and the
filter-set write-through both fire on a settled burst of keystrokes, so without
a gate the first `j` would rewrite the file in version 2 behind the prompt.
`write_config_or_report` is the one funnel both go through and is where the
gate belongs.

## 4. Uppercase becomes a key namespace

`J`, `K`, `I`, `S`, and `X` do not exist yet.
`KeyBinding::from_crossterm_event` lowercases every `Char`, so `J` and `j` are
the same key today. That stops.

- `Ctrl` and `Alt` keep lowercasing. A terminal cannot reliably tell `ctrl-J`
  from `ctrl-j`, and pretending otherwise would bind a key that never fires —
  the same trap as `ctrl-i`, which is the byte `tab`, and is why `I` rather
  than `ctrl-i` is the subtask key.
- `KeyBinding::from_str` already preserves what it is given, so a config
  reading `key = "D"` starts meaning shift-D rather than `d`. The §3 migration
  lowercases single letters in a version-1 file precisely so that this change
  cannot silently rebind one; from version 2 on, the case the user wrote is the
  case that is meant. The punctuation that *is* shift-produced (`?`, `!`, `@`,
  `*`, `<`, `>`, `{`, `}`) arrives as its own character and is unaffected.
- The comment in `default_bindings` that says "`G` and `g` are the same key, so
  shift+letter is not an available namespace" is now false and must go.
- Nothing in `Mode::Any` may take an uppercase letter. The text-entry modes do
  not fall back to `Any`, so it would be safe today — but it would be a key
  that silently stops typing in any mode added later. Uppercase bindings stay
  mode-scoped.

## 5. Creating a task

### 5.1 What a new task inherits

A new task starts with whatever context the cursor can *see*. The rule is one
sentence: **a field is inherited when the view would have shown it, and left
blank when it would not.**

| what | `i` | `I` |
| --- | --- | --- |
| project | the cursor row's project, when project grouping is on **or** the `Projects` column is visible | same |
| section | the cursor row's section, when section grouping is on **or** a section is otherwise on screen | not sent — a subtask belongs to its parent |
| parent | the cursor row's parent, so a new task beside a subtask is a sibling subtask | the cursor row itself |

The "visible" test matters. Grouping off and the `Projects` column scrolled out
of view means the screen never said which project the cursor row was in, and
silently filing a new task there is a guess the user cannot see being made. A
blank project is also recoverable — the cell is editable — where a wrong one is
a task filed somewhere nobody will look.

With nothing to inherit, the new task is a bare workspace task. That needs a
workspace gid, which `auth.workspace_gid` only optionally carries; rather than
refuse, add `workspace.gid` to the project list's `opt_fields` and keep it on
`Project`, so the workspace is known whenever any project is. If it is not — no
projects loaded at all — the insert is refused with a message, which is the
shape every other refusal in the app already has.

### 5.2 The draft row

Nothing is sent when `i` is pressed. A **draft** is a local row:

- A `DraftTask` on `TaskViewState` holding the inherited project gid, section
  gid, parent gid, and a placeholder gid (`draft:1`) no Asana task can collide
  with.
- It is injected into the rows `refresh_table` builds, directly after the
  cursor row, and is **exempt from the filters**. A draft with no title cannot
  match a `Title` filter, and a task that vanishes the instant you create it is
  not a feature.
- The cursor moves to it and the `Task` cell opens immediately, so `i` costs
  one keystroke before typing.
- `esc`, or `enter` on an empty title, discards it. Nothing was sent, so there
  is nothing to undo.
- `enter` on a title sends one `POST /tasks` and returns to edit mode, so
  `i` … `enter` `i` … `enter` is a run of new tasks.

Only one draft at a time. A second `i` while one is open commits the first.

### 5.3 The write

`POST /tasks` with `name`, and whichever of `parent`, `projects`, and
`workspace` §5.1 resolved. Section is **not** a field on create: Asana takes it
through `POST /sections/{gid}/addTask`, so a draft with a section is two
requests, the second only on the first's success.

The reply carries the real gid. The draft is replaced by a real record built
from the returned `TaskDto` — the same code path as any fetched task — and the
cursor follows it. A failure puts the message in the corner notice and **leaves
the draft on screen with its title intact**, because the alternative is losing
what was just typed to a 403.

This is the one write in the app that is not optimistic, and deliberately so.
The other edits have a local value to show while the request is in flight; a
create has a gid it does not know yet, and every subsequent edit of that row
needs it.

## 6. The supertask column

A seventh built-in column, `Parent`, directly after `Projects`.
`PARENT_COLUMN = 6` and `FIRST_CUSTOM_COLUMN` moves to `7`.

It is a **column**, so it is edited with `edit_column` in task mode like every
other cell — it changes what a row says about itself, not which rows exist. It
shows the parent task's title, or the gid when the parent is not loaded, which
is the fallback `Projects` already makes for a project the session never saw.
Empty for a top-level task.

### 6.1 The candidate order

A completion field capped at one item, over every loaded task, in this order:

1. tasks in the cursor task's **section**
2. then tasks in its **project**
3. then everything else

Within each band, by the table's own order, so the list reads like the screen.
Three things are never candidates: the task itself, any of its descendants (a
cycle Asana would reject anyway), and the draft row.

Typing re-ranks by fuzzy score (§10) and the banding becomes the tie-break: two
equally good matches put the one in your section first. With nothing typed the
banding is the whole order, which is the case that matters — the parent you
want is nearly always a few rows up.

### 6.2 The write

Asana does not accept `parent` in a task update; it is
`POST /tasks/{gid}/setParent` with `{"parent": gid-or-null}`. So this is a
`ParentEdit`, a sibling of `ProjectEdit`: not a `TaskFieldEdit`, dispatched to
its own endpoint, and rejoining the others at `PendingEdit` so there is still
one burst counter, one border chip, and one rollback path.

Clearing the cell sends `parent: null`, promoting a subtask to a top-level
task. Optimistic like the field edits: `parent_gid` and `subtask_depth` are
written locally at commit and put back on failure. `layout_subtasks` recomputes
depth from the parent chain on every rebuild, so re-parenting a task that has
children of its own moves the whole limb with no extra work.

A bulk re-parent is allowed — unlike a title, "these five are subtasks of that
one" is a sentence someone means.

## 7. Moving a task between sections

`J` moves the task under the cursor into the next section down, `K` into the
previous one up. Sections are ordered as the project orders them, which is the
order the table already groups by.

- The task is **appended** to the target section. Position *within* a section
  is not addressed by this milestone and stays on the "not yet editable" list:
  Asana takes it through `insert_before` / `insert_after` on the same endpoint,
  so it is a later keystroke rather than a later design.
- One `POST /sections/{gid}/addTask`. Moving a task into a section implicitly
  removes it from its old one, so there is no paired removal.
- Off the end in either direction does nothing. There is no "no section"
  position to move into below the last one — a task without a section is a
  state Asana puts tasks in, not one a project offers as a destination.
- Refused, with a message, when the cursor row is a subtask (subtasks are not
  in sections), when its project has only one section, or when no project could
  be resolved for it.
- Optimistic: `sections`, `section_gids`, and `section_order` are written
  locally and put back on failure, so the row jumps to its new group on the
  keystroke.

Section membership is per project. A task in two projects moves within the
project the cursor row is **grouped under**, which is the one the screen is
showing it in; with project grouping off and a task in several projects, `J`
and `K` are refused rather than guessing.

## 8. Adding and removing sections

**`S` adds a section.** A **draft section**, the same shape as the draft task
in §5.2: a heading row inserted after the cursor's section, in the cursor's
project, with a `TextEdit` open on it. `enter` sends
`POST /projects/{gid}/sections` with `name` and `insert_after`; `esc` or an
empty name discards it with nothing sent. It reuses the cell editor's mode and
keys, so there is no new text-entry mode and no new keys to learn.

**`X` deletes the section the cursor is in**, with `DELETE /sections/{gid}`.
It is refused unless the section is **empty** — Asana refuses too, and saying
so before the request is a better error than the one the API returns.

`X` does not join the mark-and-confirm flow of §8. That flow exists because
deleting tasks destroys work; an empty section holds none. It reports what it
did in the notice pane like any other write.

## 9. Deleting tasks

Deletion is the one edit with nothing to put back, so it is the one edit that
is not a single keystroke.

- `x` marks the task under the cursor, or **every selected task** when there is
  a selection — "an edit is an edit of everything selected" is the rule
  everywhere else, and deletion is not where to make an exception.
- `x` on an already-marked task unmarks it.
- A marked row is drawn **struck through** — a glyph change, not a colour
  change, so it reads under `variant = "mono"`. The pane border says
  `deleting 3`, beside where it already says `editing 3`.
- `esc` clears the marks and stays in edit mode. With no marks, `esc` leaves
  edit mode. One key, one sentence: *back out of whatever is pending.*
- `enter` deletes them, one `DELETE /tasks/{gid}` per marked task.
  `Transport` gains `delete_json` beside `get_json`, `put_json`, and
  `post_json`.

The rows go from the cache and the table at confirm time and come back if the
request fails, with the usual `could not delete 1 of 3: …` in the notice. The
cursor lands where the first deleted row was.

**A deleted parent takes its subtasks with it.** Asana does this server-side,
so marking a parent strikes through its loaded subtasks too — they are going,
and a confirmation that does not show that is a confirmation of the wrong
thing. One request is still sent, for the parent.

## 10. Fuzzy completion, and keys to walk it

Three changes to `AutocompleteState`, all in `matches_for`:

- **Ranking, not filtering.** `util::fuzzy_match` answers yes or no;
  `util::fuzzy_score` is added beside it and answers `Option<i32>` — `None` for
  no match, a higher score for a tighter one. Contiguous runs, a match at a
  word boundary, and a match at the start all score better, which is enough to
  put `Alex Chen` above `Alexandra Pemberton-Clarke` for `alex`, and
  `Ship the release candidate` above `Shelve the pipeline` for `ship`.
- **Subsequence, not substring.** `shp rc` finds `Ship the release candidate`.
  That is what "fuzzy" buys, and the reason the current `contains` is not it.
- The existing prefix-first sort goes away, subsumed by the score. An exact
  display-name match still wins at commit, unchanged.

The candidate overlay (`src/ui/completion.rs`) gains a highlight on the current
candidate, which it can already draw for the `tab` cycle.

| key | what it does |
| --- | --- |
| `ctrl-n` | highlight the next candidate |
| `ctrl-p` | highlight the previous one |
| `tab` / `shift-tab` | unchanged — complete the typed prefix to the next/previous |

`ctrl-n` in filter-edit mode is `filter_negate_field` today. Rather than move
it, it is **gated on the overlay being open**, which is exactly the shape of
`TaskState::calendar_grid_visible`: with a completion overlay up, `ctrl-n` and
`ctrl-p` walk candidates; with none, `ctrl-n` negates as it always did. The
only row where this is ambiguous is `Assignee` in `list` mode, where the
overlay is open precisely when you are picking — and `!` on the row in browse
mode is the other way to negate it.

`enter` takes the highlighted candidate when there is one, and otherwise
resolves the typed text as it does now. That keeps `tab` meaning what it means
today — type, `tab` until it reads right, `enter` — and adds the other idiom
beside it rather than replacing it.

## 11. What the record has to carry

- `TaskRecord` gains `section_gids: Vec<String>` beside `sections`. The gids
  are already in `TASK_OPT_FIELDS` (`memberships.section.gid`) and already read
  in `record_from_dto`; they are simply thrown away today, and §5.3, §7, and §8
  all need one.
- The per-project **ordered section list** has to survive the fetch. `fetch`
  builds `section_map` and `section_order_map` and drops both; §7 needs to know
  what the next section *is* and §8 needs to insert after one. Keep them as a
  `Vec<SectionInfo>` per project on `TaskState`.
- `Project` gains `workspace_gid: Option<String>`, from `workspace.gid` added
  to the project list's `opt_fields`.
- `parent_gid` already exists and `merge_task_record` already carries it — but
  it is merged with `existing.or(incoming)`, which can never clear a parent. A
  re-parent to top-level has to go through the same `confirm_edit` escape the
  other non-monotone edits use.

## 12. Implementation notes

- The draft task and the draft section belong in `TaskViewState` beside
  `cell_edit`, not in the cache. Neither is a real thing yet, and a cache entry
  for something with no gid would have to be special-cased everywhere the cache
  is read.
- `CommittedEdits` grows a list for `ParentEdit`, and `App` grows a dispatch
  beside `dispatch_task_edits` / `dispatch_project_edits`. The create, delete,
  and section writes do **not** go through `PendingEdit`: they change which
  rows exist rather than what a row says, so the rollback is a different
  operation and sharing the channel would mean a `PendingEdit` variant that
  cannot roll back.
- `FakeAsanaClient` needs `create_task`, `delete_task`, `set_parent`,
  `create_section`, `delete_section`, and `add_task_to_section`, each recording
  its calls the way `update_task` already does, so the integration tests can
  assert what went over the wire.
- The strike-through is a `Modifier::CROSSED_OUT` on the row's spans in
  `task_table::cell_spans`, which keeps the chart's "one line per row"
  invariant intact.
- `src/ui/hints.rs` and `src/ui/help_overlay.rs` both switch on the mode and
  both need an `Edit` arm; the mode badge in `src/ui/chrome.rs` needs its
  colour, and `Mode::ColumnEdit`'s label changes there too.
- The migration is a function over the parsed `Config` — `migrate_v1` in
  `src/config/mod.rs`, called from `from_toml_str` between `toml::from_str` and
  `validate` — plus a flag on `Config` saying it ran. It takes and returns a
  `Config` rather than editing TOML text, so the rename table is one list and
  the tests are plain value comparisons. The *backup* is the one part that
  works on bytes, and it works on the file rather than on the parsed value.
- The prompt is `App` state, not config state: `PendingMigration { backup_path }`
  beside the filter-set confirmation, drawn by the same centered-window helper,
  and checked by `write_config_or_report` before it writes anything.
- `tuisana.toml.example` and the repo's own `tuisana.toml` are version-1 files
  today and both need bumping, the example by hand so its comments survive.
- `tests/task_mutation_tests.rs` is the home for the create, delete,
  re-parent, and section round-trips; `tests/ui_snapshot.rs` gets a
  marked-for-deletion frame, a draft-row frame, and an edit-mode hint bar.

## 13. Acceptance criteria

- `t` from task mode enters edit mode, `esc` leaves it, and the badge and hint
  bar say which mode is which
- `begin_task_edit` is renamed `edit_column` and still opens the cell on
  `enter`; `Mode::TaskEdit` becomes `Mode::ColumnEdit` and reads as `column`
- a version-1 config prompts before it is touched, and `q` or `esc` leaves the
  file exactly as it was
- `y` writes a byte-for-byte backup to `tuisana.backup.toml`, keeping comments
  and key order, and does not clobber an existing backup
- no keystroke can write the config while the prompt is up
- a migrated config renames every v1 command and mode, lowercases single-letter
  keys, writes `header.version = 2.0`, and still rejects unknown commands
- a config with no `[header]` migrates as version 1; one already at version 2
  prompts for nothing; one from the future is still rejected
- `i` and `x` keep their task-mode meanings and gain their edit-mode ones
- an uppercase letter in a binding is a distinct key, and `ctrl-` pairs are
  still case-insensitive
- `i` opens a draft beside the cursor row with the project, section, and parent
  the view could show, and blanks the ones it could not
- `I` opens a draft whose parent is the cursor row
- `esc` or an empty title discards a draft with nothing sent; `enter` creates
  it and the cursor follows it to its real gid
- a create that fails keeps the draft and its title on screen
- a task created into a section lands in that section
- `Parent` is a column after `Projects`, shows the parent's title, and edits
- its candidates are ordered section, then project, then everything else, and
  exclude the task itself and its descendants
- clearing `Parent` promotes a subtask; a failed re-parent rolls back
- `J` and `K` move the task between its project's sections, append it, roll
  back on failure, and are refused on a subtask or an unresolvable project
- `S` opens a draft section after the cursor's and creates it on `enter`
- `X` deletes an empty section and refuses a populated one with a message
- `x` marks the cursor row or the selection, `x` again unmarks, marked rows are
  struck through, `esc` clears, `enter` deletes
- a marked parent shows its loaded subtasks as going too
- a failed delete puts the row back and says so
- completion lists rank by fuzzy score, match subsequences, and are walked by
  `ctrl-n` / `ctrl-p`
- `ctrl-n` still negates a filter row when no completion overlay is open
- tests cover the migration table, the unversioned and already-migrated cases,
  the backup naming, the write gate, the mode split, the inheritance rules, the draft lifecycle
  including the failure path, section placement and movement, the parent
  candidate order and cycle exclusion, the delete mark set and its
  confirmation, and fuzzy ranking
