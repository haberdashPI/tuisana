# Milestone 14: Edit task fields ✓

[← all milestones](../plan.md)

Delivered. Seven deliberate deviations from the plan below.

1. **`caret_window` takes the previous window start and hands back the new
   one.** §2.10 gives it four arguments and returns `(String, usize)`, but the
   window is *sticky* and stickiness has to come from somewhere. Passing the
   old start in and returning the new one is what lets `TaskCellEditState`
   keep it across frames. The filter panel passes `0`: it has no per-row place
   to keep an offset, and recomputing the minimal window each frame is the
   right trade for a column that is rarely overrun.
2. **`d` reads the cursor row only when the cursor row is one of the
   targets.** §1.3 asks for two things that cannot both hold: every target set
   to the opposite of *the cursor row's* state, and a second press putting it
   back. `space` selects the cursor row **and moves down**, so after selecting
   two rows the cursor sits on a third that is not being changed — its state
   never flips, and `d` twice sends the same value twice. The reference is the
   cursor row when it is among the targets, and the first target otherwise,
   which is the same thing in the case §1.3 describes and reversible in the
   case it does not.
3. **`BeginTaskEdit`, `CancelTaskEdit`, `TaskEditCycleValue`, and
   `TaskEditClear` are arms in `App::handle_action`.** §2.7 calls the first two
   pure view state. Opening an editor decides which mode reads the next key —
   `Mode::Calendar` for a date cell, `Mode::TaskEdit` for everything else — and
   the mode lives on `App`. `TaskColumnPrev`/`Next` are genuinely pure and did
   go in `TaskState::apply_action`.
4. **`ensure_column_visible` scrolls only when the column cursor has moved.**
   §2.10 has the renderer call it every frame. That fights `left`/`right`,
   which scroll the columns by hand: the viewport snapped straight back to the
   cursor column on the next draw, and
   `pressing_m_displays_the_task_view` caught it. A flag, set by `move_column`
   and `begin_cell_edit` and cleared by the scroll, keeps both behaviours.
5. **`d` on a task date clears the text and leaves the picker open.** In the
   filter panel `d` clears the field *and* closes the picker, because there is
   nothing else to commit. A task date has a commit waiting on `enter`, so
   closing would either send the cleared date unasked or throw it away. The
   value goes, the picker stays, `enter` still sends it.
6. **A value picker's "no value" is a stop in the ring, not only a key.** `d`
   still clears, as §2.9 has it, but `j` past the last option also lands on
   empty — which is what makes the list walkable in one direction. `State` is
   the exception: a task is always open or done, so its ring has no empty slot.
7. **The request-shape tests live in `src/asana/client.rs`, not
   `tests/task_mutation_tests.rs`.** §4.5 puts them in the integration file,
   but `MockTransport` is `#[cfg(test)]` inside the client module and an
   integration test cannot reach it. The custom-field gid test — the one the
   file exists for — is in `tests/task_mutation_tests.rs` as planned.

Two smaller notes. §4.3's word-motion table has two expectations that the
`\b` rule it states does not produce: from 21, `alt-b` lands at 18 (before
`v2`), not 19, and the "before `release`" case starts from 17 rather than 19.
And a custom-field column only ever *grows* within a session — `TaskCache`
never forgets a definition — so test 14's disappearing column is reached
through `invalidate_cache` and a reload rather than by deselecting a project.

Goal:

- make the task table writable: put a cursor on a column, edit that cell, and
  send the change to Asana — for the row you are on or for everything selected

## 1. The plan

Everything in the app so far reads. You can pull five projects together, filter
them six ways, save the filter under a name and draw the result as a Gantt
chart — and then you have to open a browser to tick one box. This milestone is
the first step to closing that: editing the fields of tasks that already exist.

Creating tasks, deleting them, re-parenting them, and moving them around the
list are the other parts, and they are [deferred](#deferred-deliberately) to
milestones of their own. They are a different kind of change — they alter the
*shape* of the list rather than the contents of a cell — and they need this
one's write path to exist first.

### 1.1 A cursor on the columns

The table already has a row cursor. It gains a column cursor: `h` and `l` walk
it left and right through Title, Assignee, Due, Start, State, Projects, and
whatever custom fields the loaded projects carry. The column under it is picked
out in the header, and the cell under it — on the cursor row — is underlined.

```
┏ Tasks ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 14 tasks │ row 2 │ 2 selected ┓
┃   Title                │ Assignee │  Due │ Start │ St │ Priority          ┃
┃    Ship the release    │ alex     │  +3d │ Sep 1 │ ○  │ High              ┃
┃ ▸  Write the changelog │ alex     │  +5d │ Sep 2 │ ○  │ High              ┃
┃  ● Cut the tag         │ jo       │  +5d │ Sep 2 │ ○  │ Low               ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
                                      ▲
                          the Due header is accented, and the cursor
                          row's Due cell is underlined
```

The cursor is only drawn in task mode and while editing. In Gantt mode `h` and
`l` already scroll the timeline, and a cursor you cannot move is a cursor that
lies about what the keys do.

If the column is off to the right, moving onto it scrolls the table to it —
the same courtesy `ensure_selected_visible` does vertically.

### 1.2 Three editors, all of them ones you have already used

`e` opens the cell under the cursor. Which editor you get depends on the
column, and each one is the filter panel's editor for that kind of field:

| Column | Editor | Keys |
| --- | --- | --- |
| Title, Assignee, text custom fields | a text field with a caret | type, `bksp`, the motions in §1.5 |
| Due, Start | the date picker from [Milestone 11.75](11.75-date-selection.md) | `h`/`l` day, `j`/`k` month, `t` today, `d` clear |
| State, enum custom fields | a value picker | `j`/`k` through the options, `d` for no value |

`enter` commits, `esc` throws the edit away. This is deliberately the same
vocabulary as the filter panel: the filter panel taught you that `enter` starts
an edit and `esc` abandons it, that a date opens a calendar, and that a field
with a small set of values cycles with `j`/`k`. None of that is re-learned here.

Two differences from the filter panel, both because a task holds a value where
a filter holds a *query*:

- **A date is one day, not a range.** `2026-09-01..2026-09-08` is a sensible
  filter and a nonsensical due date, so a range is refused at commit rather
  than half-applied. Everything the picker accepts for a single date — a
  keyword, `MM-DD`, `YYYY-MM-DD` — is accepted and normalized to `YYYY-MM-DD`.
- **A value picker holds one value, not a list.** The filter's Priority row can
  ask for `High | Low`; a task's Priority is High, or Low, or nothing.

The options a picker offers come from the custom field's **declared** enum
options, not from the values tasks happen to carry. The filter panel infers its
options from what it has seen, which is right for filtering — you can only
filter to a value that exists — and wrong for editing, where the whole point
may be to be the first task marked `Blocked`. §2.4 is what fetches them.

### 1.3 `d` marks it done

Completion is a field like any other — the State column, two options — but it
is also the single most common edit in the app, so it gets a key of its own:
`d`, from anywhere in the row, toggles between open and done.

On a selection it is not a per-task toggle. Every target is set to the opposite
of **the cursor row's** state, so a mixed selection ends up uniform and
pressing `d` twice puts it back. A per-task toggle would leave a mixed
selection mixed, which is never what "mark these done" meant.

### 1.4 An edit is an edit of everything selected

The rule is the one [Milestone 10](10-task-interaction.md) established for
every other bulk action, with one addition for the case where nothing is
selected at all:

- With tasks selected, an edit applies to **all of them**.
- With nothing selected, it applies to **the cursor row**.

The editor opens on the cursor row's value even when the selection holds
several different ones; the pane border says `editing 3` so the blast radius is
on screen before you commit. The targets are resolved when the edit *opens*, so
a table that rebuilds underneath you — a project finishing its load, a filter
write-through — cannot widen what `enter` is about to change.

**Titles are the exception.** Setting three tasks to the same title is not a
bulk edit, it is a mistake with three victims, and there is no plausible reason
to want it. `e` on the Title column with more than one task selected is
refused, and says why: `a title is edited one task at a time (3 selected)`.

### 1.5 Text editing grows motions

Editing text today means typing, backspace, and one character of caret motion
in each direction. This milestone adds word motion and line motion, and they
follow emacs, which is what `ctrl-b`/`ctrl-f` and the existing
`ctrl-a`/`ctrl-e` already commit us to:

| key | what it does |
| --- | --- |
| `alt-b` / `alt-f` | back a word / forward a word |
| `ctrl-a` / `ctrl-e` | to the start / to the end |
| `ctrl-b` / `ctrl-f`, `left` / `right` | back / forward one character |
| `ctrl-l` | clear the field |

A "word" is the regex `\b` boundary: a run of `[A-Za-z0-9_]`. `alt-f` skips any
non-word characters and then lands after the run it reaches; `alt-b` mirrors
it. So from the start of `Ship the release (v2)`, three `alt-f`s land after
`release` and the fourth after `v2`.

These work **everywhere text is edited**, not just in the new cell editor: the
filter panel gets them in the same commit, because there is going to be one
text-buffer implementation rather than two (§2.1).

`alt-` is a new kind of key binding for this config; it needs the terminal to
send Option/Alt as a modifier rather than as an escape prefix (§2.9).

### 1.6 A cell too long for its column scrolls

A title is longer than the Title column and a filter value is longer than the
value column. Both truncate with an ellipsis today, which is right for reading
and useless for editing: put the caret past the cut and it is not on screen.

While a cell is being edited, the text scrolls inside the column to keep the
caret visible, with an ellipsis on whichever side is clipped:

```
  Title                                    Title
  ┌───────────────────┐                    ┌───────────────────┐
  │Ship the release (…│   ctrl-e →         │…release (v2) befo▮│
  └───────────────────┘                    └───────────────────┘
```

The filter panel's value column gets the same treatment, because it has the
same bug and it would be silly to write the helper twice.

### 1.7 What goes over the wire

An edit is applied locally the moment you commit it, and sent to Asana in the
background — one request per task, on a worker thread, exactly as loading
already works. The table does not freeze while twelve tasks are marked done.

If a request fails, that task's field is put back the way it was and the pane
border says so: `could not update 1 of 12: backend error: 403`. Optimism is
worth it — nearly every write succeeds, and the alternative is a spinner on
every keystroke — but an optimistic update that quietly diverges from the
server is worse than either, so the rollback is not optional.

Two consequences worth knowing before they surprise someone:

- **An edit can make a row vanish.** Re-assign a task while filtering to
  `alex` and it leaves the table. That is the filter doing its job; the cursor
  lands on the next row, as it does after any rebuild.
- **An edit does not re-fetch.** Moving a due date outside the window pushed
  down to Asana does not hide the task — it is already in the cache, and the
  cache is what the table is built from.

## 2. How to implement it

### 2.0 Orientation

[Milestone 13 §2.0](13-filter-sets.md) is the map of the filter panel, whose
editors this milestone reuses; read it if the panel is unfamiliar. The pieces
this milestone touches, with line numbers as of Milestone 13.5:

| Item | Where | What it is |
| --- | --- | --- |
| `TaskRecord` | `src/domain/task.rs:94` | the record every edit mutates |
| `CustomFieldDefinition` | `src/domain/task.rs:31` | today just `gid` + `name` |
| `merge_task_record` | `src/domain/task.rs:833` | the deliberately monotone merge — read §2.5 |
| `TaskTableModel::from_records_with_settings` | `src/domain/task.rs:586` | builds the cells |
| `ASSIGNEE_COLUMN` / `STATE_COLUMN` | `src/domain/task.rs:297` | built-in columns are positional |
| `TaskViewState` | `src/app/task.rs:70` | where the column cursor and the open edit live |
| `TaskCache` | `src/app/task.rs:338` | `upsert_record`, and the merge that must be bypassed |
| `TaskFilterFieldState` | `src/app/task.rs:193` | `query` + `query_caret`, becoming a `TextEdit` |
| `push_char` / `pop_char` / `move_query_caret` | `src/app/task.rs:753` | the buffer being replaced |
| `refresh_table` | `src/app/task.rs:3396` | every clamp goes here |
| `add_task_tree` | `src/app/task.rs:3654` | where records are built from DTOs |
| `build_dataset_for_projects` | `src/app/task.rs:2946` | where custom-field settings are read |
| `CalendarState::open` | `src/app/calendar.rs:58` | the picker, already independent of the filter |
| `App::handle_action` | `src/app.rs:727` | needs the client, so commits land here |
| `poll_task_data` | `src/app.rs:1198` | the polling loop the write results join |
| `start_task_data_fetch` | `src/app.rs:1282` | the worker-thread pattern to copy |
| `AsanaClient` | `src/asana/mod.rs:120` | read-only today |
| `Transport` | `src/asana/client.rs:33` | `get_json` only |
| `render_task_table` | `src/ui/task_table.rs:299` | builds the view; `&TaskState` becomes `&mut` |
| `resolve_cell` | `src/ui/task_table.rs:683` | renders dates *relatively* — see §2.10 |
| `key_text` | `src/ui/hints.rs:409` | needs an `alt-` arm |
| actions | `src/input/mod.rs` | `Action`, `from_command`, `Display` — **edit all three** |

Four constraints, each with a new way to be violated here:

- **A rebuild happens after every project finishes loading.** For the filter
  panel that is `restore_queries`; for this milestone it is the clamps in
  `refresh_table`. The column list grows as custom fields arrive, so a column
  cursor that is not clamped points off the end, and an open edit whose column
  disappears has to close.
- **`merge_task_record` is monotone on purpose.** `completed |= incoming`, and
  a `None` never overwrites a `Some`. An edit that un-completes a task or
  clears a date cannot be applied through it. §2.5.
- **Custom-field rows and columns are keyed by *name*, across several gids.**
  Reading merges them; writing cannot. The write has to pick the gid belonging
  to the task's own project. §2.4.
- **One line per row.** The cell editor draws inside the existing cell. It does
  not get a line, a popup, or an extra row.

### 2.1 One text buffer, in `src/app/text_edit.rs`

A new module beside `calendar.rs`, which is the precedent: a small interaction
state machine, owned by whoever is editing, with no UI and no domain knowledge.

```rust
/// A one-line text buffer with a caret.
///
/// Positions are char indices, never byte offsets, because the caret is drawn
/// by reversing the character it sits on and a multi-byte character is one
/// caret stop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextEdit {
    text: String,
    caret: usize,
}

impl TextEdit {
    /// A buffer holding `text`, with the caret at its end.
    pub fn new(text: impl Into<String>) -> Self;
    pub fn text(&self) -> &str;
    pub fn caret(&self) -> usize;
    /// Replaces the text, clamping the caret into it.
    pub fn set_text(&mut self, text: impl Into<String>);
    pub fn insert(&mut self, ch: char);
    pub fn delete_back(&mut self);
    pub fn clear(&mut self);
    pub fn move_caret(&mut self, delta: i64);
    /// Moves a word at a time: `-1` back, `1` forward.
    pub fn move_word(&mut self, delta: i64);
    pub fn jump_start(&mut self);
    pub fn jump_end(&mut self);
}

/// Whether a character is part of a word, for [`TextEdit::move_word`].
///
/// The regex `\w` class. Word motion stops on the `\b` boundaries this
/// implies, which is what makes `alt-f` in a hyphenated title stop at each
/// part rather than skipping the whole thing.
fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}
```

`move_word(1)` skips non-word characters, then skips word characters, and
stops; `move_word(-1)` is the mirror image, looking at the character *before*
the caret. Both clamp rather than wrap.

**Then adopt it in the filter panel**, in the same commit: replace
`TaskFilterFieldState`'s `query: String` and `query_caret: usize`
(`src/app/task.rs:193`) with `value: TextEdit`. It is mechanical —
`field.query` becomes `field.value.text()`, `field.query = x` becomes
`field.value.set_text(x)` — across `push_char`, `pop_char`,
`move_query_caret`, `reset_query_caret`, `clear_value`, `sync_calendar_query`,
the label methods that rewrite `query` from `label_values`, `prepare`,
`to_saved`, `restore_from`, `apply_saved_field`, and `filter_panel_entries`.
The saved-config types keep their `query: String`; only the in-memory field
changes.

Do **not** adopt it in `CalendarState` or `SidebarPrompt` here. The calendar's
caret is not a plain caret — it decides which end of a range the day keys
rewrite — and the prompt is four lines of code. Both are noted under Deferred
so the remaining duplication is a decision rather than an oversight.

### 2.2 The mutation model, in `src/domain/task_edit.rs`

A new module, named in `plan.md`'s architecture since the beginning.

```rust
/// One field of one task, set to one value.
///
/// Modelled as a field change rather than a whole-task patch because the UI
/// edits one cell at a time, and because a patch would have to distinguish
/// "leave this alone" from "clear this" on every field.
#[derive(Clone, Debug, PartialEq)]
pub enum TaskFieldEdit {
    Name(String),
    Completed(bool),
    Due(Option<String>),
    Start(Option<String>),
    /// `None` unassigns.
    Assignee(Option<AssigneeRef>),
    CustomField { gid: String, value: Option<CustomFieldValue> },
}

/// Who a task is being assigned to.
///
/// Asana takes a user gid, an email address, or the literal `me`; it does not
/// take a display name. `display` is what the local record shows until a
/// reload brings the real one, which for an email is the email itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssigneeRef { pub handle: String, pub display: String }

#[derive(Clone, Debug, PartialEq)]
pub enum CustomFieldValue {
    Enum { option_gid: String, name: String },
    Text(String),
    Number { value: f64, text: String },
}

/// One pending change: what to do, and what to put back if it fails.
///
/// `previous` is the same type as `field`, so a rollback is the same code path
/// as an apply.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskEdit {
    pub gid: String,
    pub field: TaskFieldEdit,
    pub previous: TaskFieldEdit,
}

impl TaskFieldEdit {
    /// Writes this change onto a record.
    ///
    /// Used for the optimistic update *and* for the rollback, and deliberately
    /// not routed through `merge_task_record`: the merge is monotone, so it
    /// would refuse to un-complete a task or to clear a date.
    pub fn apply(&self, record: &mut TaskRecord);
}
```

Parsing and validation live here too, as free functions returning
`Result<TaskFieldEdit, String>` — the `String` is the message the pane shows:

- `parse_date_value(text, today) -> Result<Option<String>, String>`: empty is
  `None`; a range (`..`) is `Err("a task date is one day, not a range")`;
  otherwise the existing `domain::date` parsing, normalized to `YYYY-MM-DD`.
- `resolve_assignee(text, directory, me) -> Result<Option<AssigneeRef>, String>`:
  empty unassigns; `me` is the current user; anything containing `@` is sent
  verbatim as an email; otherwise a case-insensitive match against the people
  seen in the loaded records, with "no one called X is loaded" and "X is
  ambiguous" as the two failures.
- `parse_custom_value(kind, text) -> Result<Option<CustomFieldValue>, String>`:
  an enum takes an option name and yields its gid; a number refuses text that
  is not a number; `date`, `multi_enum`, and `people` custom fields are refused
  outright with "this field cannot be edited here yet".

### 2.3 Writing, in `src/asana/`

`Transport` gains one method, and `MockTransport` in the module's tests gains
it with it:

```rust
    /// Sends a JSON body and returns the response.
    fn put_json(&self, path: &str, body: &Value, token: &str) -> Result<Value>;
```

`AsanaClient` gains one method:

```rust
    /// Applies one field change to one task and returns the task as the server
    /// now has it.
    ///
    /// Returns the task rather than `()` so the caller can pick up the new
    /// `modified_at` — without it the next merge cannot tell the server's copy
    /// from a stale one.
    fn update_task(&self, task_gid: &str, edit: &TaskFieldEdit) -> Result<TaskDto>;
```

The body is `{"data": { … }}` with one key: `name`, `completed`, `due_on`,
`start_on`, `assignee`, or `custom_fields: {"<gid>": <value>}`. A cleared value
is JSON `null`, which is how Asana unsets a field — omitting the key means "no
change" and would make clearing impossible. The request asks for
`TASK_OPT_FIELDS` back, so the response decodes as an ordinary `TaskDto`.

`FakeAsanaClient` applies the edit to its stored DTOs — so a subsequent
`list_tasks` sees it — records every call in a `RefCell<Vec<(String,
TaskFieldEdit)>>`, and gains

```rust
    /// Makes `update_task` fail for one task, for the rollback tests.
    pub fn with_update_failure(self, task_gid: impl Into<String>) -> Self;
```

### 2.4 What the record and the field definitions have to start carrying

Three additions, all of them because reading merges things that writing must
keep apart.

**`TaskRecord::assignee_gid: Option<String>`** (`src/domain/task.rs:94`). The
record keeps the display name today and throws the gid away, and a display
name is not something Asana will accept. Set it in `add_task_tree`
(`src/app/task.rs:3675`), which already has `assignee.gid` in hand;
`TASK_OPT_FIELDS` already asks for it. `merge_task_record` carries it beside
`assignee`, under the same condition.

**`CustomFieldDefinition` grows a kind and a project** (`src/domain/task.rs:31`):

```rust
pub struct CustomFieldDefinition {
    pub gid: String,
    pub name: String,
    /// The project whose settings declared this field.
    ///
    /// One field name usually exists once per project, with a different gid in
    /// each. Reading merges them by name; a write has to choose, and the only
    /// correct choice is the gid belonging to the task's own project.
    pub project_gid: String,
    pub kind: CustomFieldKind,
}

/// What a custom field holds, as Asana declares it.
///
/// Declared rather than inferred. The filter panel guesses a field's kind from
/// the values it has seen (`from_dataset`, `src/app/task.rs:513`), which is
/// right for filtering and wrong for editing: a picker built from observed
/// values can never offer the option no task has yet.
pub enum CustomFieldKind {
    Text,
    Number,
    Enum { options: Vec<EnumOption> },
    /// Everything else, which this milestone refuses to edit.
    Unsupported(String),
}

pub struct EnumOption { pub gid: String, pub name: String }
```

`CustomFieldDefinition::new` keeps its two-argument shape for the many test
fixtures that call it, defaulting to `Text` and an empty project; a
`with_kind`/`in_project` builder pair carries the rest.

**The settings request has to ask for them** (`src/asana/client.rs:225`):

```rust
("opt_fields", "gid,custom_field.gid,custom_field.name,\
  custom_field.resource_subtype,custom_field.enum_options.gid,\
  custom_field.enum_options.name,custom_field.enum_options.enabled".to_string()),
```

with matching fields on `CustomFieldDto` (`src/asana/dto.rs:161`). Disabled
enum options are dropped when the definition is built: they exist so old values
still render, and offering one is offering a value Asana will reject.

`build_dataset_for_projects` (`src/app/task.rs:2946`) currently collapses
settings into a `HashMap<gid, name>` and rebuilds definitions from it; it keeps
the whole definition instead. `TaskCache::custom_field_definitions` follows.

### 2.5 Applying an edit locally

On `TaskState`:

```rust
    /// Applies an edit to the cache and the visible dataset, and rebuilds.
    ///
    /// Writes the fields directly rather than going through
    /// `TaskCache::upsert_record`. `merge_task_record` is monotone by design —
    /// `completed |= incoming`, and a `None` never overwrites a `Some` — so
    /// merging an un-complete or a cleared due date is a silent no-op. This is
    /// also why the reply from the server is not merged back in: the local
    /// record is already the truth, and only `modified_at` is taken from it.
    pub(crate) fn apply_edit_locally(&mut self, edit: &TaskEdit);

    /// Records the `modified_at` the server reported for a confirmed edit.
    ///
    /// Without it, a fetch that started *before* the edit can come back with
    /// an older copy that `merge_task_record` reads as newer-or-equal and
    /// applies over the top.
    pub(crate) fn confirm_edit(&mut self, gid: &str, modified_at: Option<String>);
```

Both touch `loading.cache.records` and the copy in `loading.dataset`, then
`refresh_table()`. The rebuild is what re-applies the filter, so a row that no
longer matches leaves the table here, through the ordinary cursor-recovery path
already in `refresh_table` (`src/app/task.rs:3438`).

### 2.6 The column cursor and the open edit, in `src/app/task_edit.rs`

A new module holding the state; `TaskViewState` (`src/app/task.rs:70`) gains
two fields:

```rust
    /// The column cursor, as a cell index into `TaskTableModel::columns`.
    selected_column: usize,
    /// The edit in progress, if any.
    cell_edit: Option<TaskCellEditState>,
    /// What went wrong with the last edit, shown on the pane border.
    edit_notice: Option<String>,
```

```rust
/// The cell being edited, and what it will be applied to.
pub(crate) struct TaskCellEditState {
    /// The column, fixed for the life of the edit.
    column: usize,
    /// The tasks the commit applies to, resolved when the edit opened.
    ///
    /// Resolved once, on purpose: the table rebuilds whenever a project
    /// finishes loading or a filter changes, and an edit that re-read the
    /// selection at commit time could change more than it said it would.
    targets: Vec<String>,
    editor: CellEditor,
}

pub(crate) enum CellEditor {
    Text(TextEdit),
    /// One value out of a fixed set. `None` is "no value".
    Options { options: Vec<String>, cursor: Option<usize> },
    /// The picker, mirrored into the text the same way the filter does it:
    /// the buffer is the value, the overlay only writes into it.
    Date { text: TextEdit, calendar: CalendarState },
}
```

The methods on `TaskState`, all of them returning a message rather than
panicking, because every one of them can be refused:

```rust
    pub(crate) fn move_column(&mut self, delta: i64);
    /// The tasks an edit would apply to: the selection, or the cursor row.
    pub(crate) fn edit_targets(&self) -> Vec<String>;
    /// Opens the editor for the column under the cursor.
    pub(crate) fn begin_cell_edit(&mut self, today: CivilDate) -> Result<(), String>;
    pub(crate) fn cancel_cell_edit(&mut self);
    /// Turns the open editor into the changes to send. `Err` keeps it open.
    pub(crate) fn commit_cell_edit(&mut self, ctx: &EditContext) -> Result<Vec<TaskEdit>, String>;
    /// `d`: every target to the opposite of the cursor row's state.
    pub(crate) fn toggle_completed_edits(&mut self) -> Vec<TaskEdit>;
```

`begin_cell_edit` is where the refusals live: the Title rule from §1.4, the
Projects column ("project membership is not editable yet"), an unsupported
custom-field kind, and no task under the cursor at all. `EditContext` carries
what the resolution in §2.2 needs and `TaskState` does not own — the current
user gid and today's date.

`refresh_table` (`src/app/task.rs:3396`) gains the clamps, beside the two that
already clamp the filter cursor:

```rust
        self.view.selected_column = self
            .view
            .selected_column
            .min(self.view.table.columns.len().saturating_sub(1));
        // A custom-field column can disappear when the project carrying it is
        // deselected mid-edit. An editor pointing at a column that is gone
        // would commit to whatever slid into its index.
        if self
            .view
            .cell_edit
            .as_ref()
            .is_some_and(|edit| edit.column >= self.view.table.columns.len())
        {
            self.view.cell_edit = None;
        }
```

### 2.7 Sending it, in `src/app.rs`

`App` gains a channel that lives for the session, rather than one receiver per
batch — several batches can be in flight and their replies interleave:

```rust
    /// Results from in-flight task writes.
    ///
    /// A permanent pair rather than one receiver per batch: `d` on twelve
    /// tasks is twelve requests, and the next edit must not have to wait for
    /// them. Each message names its task, so no generation counter is needed —
    /// a late reply to a superseded edit is reconciled by gid, not discarded.
    task_edit_events: (Sender<TaskEditMessage>, Receiver<TaskEditMessage>),

struct TaskEditMessage {
    edit: TaskEdit,
    result: Result<TaskDto>,
}
```

`dispatch_task_edits(&mut self, edits: Vec<TaskEdit>)`: apply each one with
`apply_edit_locally`, then spawn one thread per edit with a clone of the client
and the sender, following `start_task_data_fetch` (`src/app.rs:1282`).

`poll_task_edits(&mut self)`, called from `poll_task_data`'s two call sites —
`draw` (`src/ui/runtime.rs:138`) and `handle_key_event_inner`
(`src/app.rs:1149`) — drains the receiver: on `Ok(dto)`, `confirm_edit` with
its `modified_at`; on `Err`, apply `edit.previous` and push onto a failure
count that becomes the notice. Count rather than message-per-task: twelve
failures is one border chip, `could not update 3 of 12: backend error: 403`,
not three that overwrite each other.

Routing in `handle_action` (`src/app.rs:727`): `TaskColumnPrev`/`Next`,
`BeginTaskEdit`, and `CancelTaskEdit` are pure view state and belong in
`TaskState::apply_action` (`src/app/task.rs:3203`). `CommitTaskEdit` and
`ToggleTaskCompleted` need the client, so they are arms in
`App::handle_action`. **They must not `return Ok(None)` early** — fall through
to the tail so `update_task_data_after_action` still runs, for the same reason
[Milestone 13.5 §2.5](13.5-named-filter-sets.md) gives.

Typed characters reach the editor the way the filter's do: a
`handle_task_edit_input` modelled on `handle_filter_field_input`
(`src/app.rs:375`), tried in `handle_key_event_inner` before the filter's.

### 2.8 One calendar, two owners

`CalendarState` was made independent of the filter editor in
[Milestone 11.75](11.75-date-selection.md) precisely for this. What is *not*
independent is the plumbing: `TaskState::filter_calendar()` and the twelve
`filter_calendar_*` wrappers (`src/app/task.rs:2139`–`2235`), and the overlay's
`calendar::calendar_view(app.tasks.filter_calendar())`
(`src/ui/runtime.rs:210`).

Generalize the accessor rather than duplicating the mode:

```rust
    /// The open date picker, whichever editor owns it.
    ///
    /// One at a time: opening a cell editor closes the filter panel's picker
    /// and vice versa, because `Mode::Calendar` has one set of keys and they
    /// have to reach one place.
    pub(crate) fn calendar(&self) -> Option<&CalendarState>;
```

and have the `filter_calendar_*` wrappers dispatch to whichever is open. No new
mode: `Mode::Calendar` keeps its bindings, and `esc` returns to whichever mode
opened it, which the app already tracks well enough to restore.

`ctrl-a`/`ctrl-e` stay bound to `calendar_jump_to_start`/`_end` in calendar
mode. On a task date there is no range to jump between, so both are no-ops
there — `jump_to_start` already returns `bool` for exactly this.

### 2.9 Actions, modes, bindings

A new key *kind* first. `KeyBinding` (`src/input/mod.rs:14`) gains
`Alt(char)`, with:

- `FromStr`: `alt-x`, alongside the existing `ctrl-x` arm.
- `from_crossterm_event`: `KeyCode::Char(c)` with `KeyModifiers::ALT` and not
  `CONTROL`, lowercased like the others. **Order matters** — the `ALT` arm goes
  before the bare `Char` arm or every Alt key arrives as a plain character.
- `sort_rank`: `(7, c)`, after `Ctrl`.
- `key_text` (`src/ui/hints.rs:409`): `format!("M-{c}")`.

Terminals differ on Alt: many send `ESC` then the character rather than setting
the modifier. macOS Terminal and iTerm2 both need Option configured as Meta.
The escape-prefix form is **not** supported — a lone `ESC` is `esc` here, and
guessing from timing is how editors get famously confused. The README says so
where it documents the key syntax, and every motion is rebindable.

In `src/input/mod.rs` — enum, `from_command`, and `Display`, all three:

| variant | command | default key | mode |
| --- | --- | --- | --- |
| `TaskColumnPrev` | `task_column_prev` | `h` | task |
| `TaskColumnNext` | `task_column_next` | `l` | task |
| `BeginTaskEdit` | `begin_task_edit` | `e` | task |
| `ToggleTaskCompleted` | `toggle_task_completed` | `d` | task |
| `CommitTaskEdit` | `commit_task_edit` | `enter` | task-edit |
| `CancelTaskEdit` | `cancel_task_edit` | `esc` | task-edit |
| `TaskEditCycleValue(i32)` | `task_edit_next_value` / `_prev_value` | `j` / `k` | task-edit |
| `TaskEditClear` | `task_edit_clear` | `d` | task-edit |
| `TextCaretWordBack` | `text_caret_word_back` | `alt-b` | task-edit, filter-edit |
| `TextCaretWordForward` | `text_caret_word_forward` | `alt-f` | task-edit, filter-edit |
| `TextCaretStart` | `text_caret_start` | `ctrl-a` | task-edit, filter-edit |
| `TextCaretEnd` | `text_caret_end` | `ctrl-e` | task-edit, filter-edit |

Check each against the current defaults (`src/config/mod.rs:638`) before
trusting the list. `h`, `l`, `e`, and `d` are all unbound in `Mode::Task`
today, and `Mode::Task` falls back to `Mode::Any`, which binds none of them.
`ctrl-a` and `ctrl-e` are unbound in `Mode::FilterEdit` — `ctrl-e` is
`filter_require_empty` there, so **that one moves** to `ctrl-q` and the README
and `tuisana.toml.example` move with it. Taking `ctrl-e` for "end of line" is
worth the churn: it is the emacs key, it is already what it means in
`Mode::Calendar`'s text, and a require-empty is a filter-only concept that does
not deserve the most-used editing key.

`TaskEditCycleValue` carries its direction rather than becoming two variants,
following `FilterSetLoad(u8)`.

**A new mode, `Mode::TaskEdit`**, added to the `Mode` enum
(`src/config/mod.rs:476`), `label()` (`"task edit"`),
`allows_any_fallback`'s exclusion list beside `FilterEdit` — so an unbound
letter types rather than firing `d` — `focused_pane` (→ `Task`),
`help_visible` (→ the task-side toggle), `groups_for`, and the parser that
turns `mode = "task_edit"` in config into it.

### 2.10 Rendering

**The column cursor.** `TaskTableView` gains `pub cursor_column: Option<usize>`
— `None` outside task and task-edit modes, which is what keeps every Gantt
snapshot unchanged. `task_header_line` (`src/ui/task_table.rs:387`) draws that
header with `theme.accent`; `task_line` (`:514`) underlines that cell on the
cursor row.

**The open editor.** `RenderRow` gains `pub editing: Option<CellEdit>` with the
text and caret, or the option list and which one is current. `cell_spans`
(`:603`) draws an editing cell through the new window helper instead of
`pad_cell`, and reuses `filter_panel::caret_spans` (`src/ui/filter_panel.rs:445`)
— promoted to `ui::text` — to reverse the character under the caret.

**Dates render differently while edited.** `resolve_cell` (`:683`) turns
`2026-09-28` into `+5d`; an editor showing `+5d` and committing `2026-09-28`
would be lying about what `bksp` is about to delete. An editing Due or Start
cell shows the raw buffer.

**The scrolling window**, in `src/ui/text.rs` beside `truncate_with_ellipsis`
(`src/ui/text.rs:39`):

```rust
/// The `width` cells of `text` that contain the caret, and where the caret
/// lands inside them.
///
/// An ellipsis marks each clipped side, and costs a cell from the window, so
/// the caret is never the character the ellipsis replaced. Measured in display
/// cells like everything else in this module, so a CJK title scrolls by
/// columns rather than by chars.
pub fn caret_window(text: &str, caret: usize, width: usize, ellipsis: &str)
    -> (String, usize);
```

The window is sticky — it only moves when the caret would leave it — which
needs somewhere to keep it. Keep the offset on `TaskCellEditState`, updated at
render time like `horizontal_scroll`, rather than recomputing a centred window
every frame and making the text jitter under a moving caret.

`value_spans` in `src/ui/filter_panel.rs:405` uses the same helper for its
`FilterValue::Text` arm, which is §1.6's other half.

**`ensure_column_visible`**, so moving onto an off-screen column scrolls to it.
The widths are only known to the renderer, so this follows
`ensure_filter_visible`'s precedent exactly: `render_task_table`
(`src/ui/task_table.rs:299`) takes `&mut TaskState` and calls it after
`column_widths`. In `draw` (`src/ui/runtime.rs:169`) the borrow ends before
`render_task_pane` takes `app` again, so this compiles as-is.

**The notice and the count.** `counts` (`src/ui/task_table.rs:834`) gains
`Chip::toned(notice, Tone::Danger)` when one is set, and
`Chip::toned(format!("editing {n}"), Tone::Accent)` while an edit is open on
more than one task.

### 2.11 Hints, help, docs

`HintContext` gains `on_task_cell: bool` (a cell editor is open) and
`task_edit_is_options: bool`. `Mode::Task` gains `h/l column`, `e edit`, and
`d done`; `Mode::TaskEdit` gets a list of its own — `⏎ save`, `esc cancel`,
`M-b/M-f word`, `^a/^e ends`, and `j/k value` only on an options cell.

`help_overlay.rs`: `task_groups()` gains an **Edit** group, and
`Mode::TaskEdit` maps to it plus a **Text editing** group that
`filter_groups()` also picks up, since the motions are shared.

Docs:

- `README.md`: an **Editing tasks** section after the Gantt chart section (line
  562); the four new task-mode commands in the key list (line 224-ish) and a
  **Task edit mode** block after **Filter edit mode**; `task_edit` in the list
  of modes (line 160); the `alt-` key syntax and its terminal caveat where key
  names are described; and the `ctrl-e` → `ctrl-q` move for
  `filter_require_empty`.
- `tuisana.toml.example`: the new binds, and the moved one.
- `developer.md`: `src/app/task_edit.rs`, `src/app/text_edit.rs`, and
  `src/domain/task_edit.rs` in "Start Here"; a Core Concepts paragraph on the
  optimistic write and why `merge_task_record` is bypassed; a Common Change
  Path for "to change how a cell is edited".

## 3. The tests that matter

Sixteen. The first four are the ones that would let a real bug through.

1. **An un-complete survives the cache.** The monotone-merge trap: mark a done
   task open and the record is open, in the cache and in the table.
2. **A cleared date stays cleared**, for the same reason, and a cleared
   assignee and custom field with it.
3. **A failed write rolls back**, to the value the field had, and says so.
4. **A stale fetch does not undo a confirmed edit.** A response that started
   before the edit and arrives after it does not resurrect the old value.
5. **A bulk edit hits every selected task, and only those.** With nothing
   selected it hits the cursor row.
6. **A title is refused on a multi-task selection**, with the count in the
   message, and allowed on one.
7. **Targets are fixed when the edit opens.** Selecting more tasks mid-edit
   does not widen the commit.
8. **`d` sets the whole selection to the cursor row's opposite**, so a mixed
   selection comes out uniform and a second press returns it.
9. **Word motion lands on `\b` boundaries**, in both directions, through
   punctuation, at both ends, and over multi-byte characters.
10. **Dates parse, normalize, and refuse a range.** `today`, `09-28`,
    `2026-09-28`, `2026-02-31`, `a..b`.
11. **An assignee resolves by name, by email, by `me`**, and refuses an unknown
    or ambiguous name without closing the editor.
12. **A picker offers declared options, including ones no task has**, and
    refuses a field kind this milestone does not write.
13. **A custom-field write picks the gid belonging to the task's project**,
    where two projects declare the same field name.
14. **The column cursor clamps across a streaming reload**, and an edit whose
    column disappears closes rather than committing to its replacement.
15. **A long value scrolls under the caret** rather than truncating, in both a
    table cell and a filter row.
16. **The twelve commands parse, round-trip through `Display`, and are bound**;
    `alt-b` parses and renders as `M-b`; end-to-end, driven by keys, plus
    snapshots at 80/120/200.

## 4. How to write them

### 4.1 The monotone-merge traps — `src/app/task.rs`, `mod tests`

Tests 1–4. Build state with `loaded_state_with_tasks` (`src/app/task.rs:5137`),
which already exists, then:

```rust
#[test]
fn marking_a_done_task_open_survives_the_cache_merge() {
    // `merge_task_record` is `completed |= incoming`, so an un-complete
    // applied through it is a silent no-op — and the row would snap back to
    // done on the next rebuild rather than at the point of the edit.
    let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Done thing", true)]);
    state.set_completed_filter(None);

    let edits = state.toggle_completed_edits();
    for edit in &edits { state.apply_edit_locally(edit); }

    assert_eq!(cell(&state, "t1", STATE_COLUMN), "open");
    state.refresh_from_cache();
    assert_eq!(cell(&state, "t1", STATE_COLUMN), "open", "and it stays open");
}
```

Test 4 needs the fetch path: apply the edit, then `ingest_loaded_project` with
a dataset holding the *pre-edit* DTO and an older `modified_at`, and assert the
edited value stands. This is the one that fails if `confirm_edit` is skipped.

### 4.2 Bulk rules — `src/app/task.rs`, `mod tests`

Tests 5–8. `edit_targets` is the seam: assert it directly for the
selection/cursor-row rule, then drive 6 and 7 through `begin_cell_edit`.
For 7, open the edit, call `select_all_visible_tasks`, commit, and assert the
returned `Vec<TaskEdit>` still names one gid.

### 4.3 Word motion — `src/app/text_edit.rs`, `mod tests`

Table-driven, because the interesting part is the boundary cases:

```rust
#[test]
fn word_motion_stops_on_word_boundaries() {
    // `\b`: a run of [A-Za-z0-9_]. Punctuation is skipped on the way and is
    // never a landing place, which is what makes `alt-f` in "(v2)" stop after
    // `v2` rather than after `)`.
    let cases = [
        ("Ship the release (v2)", 0, 1, 4),    // after "Ship"
        ("Ship the release (v2)", 4, 1, 8),    // after "the"
        ("Ship the release (v2)", 16, 1, 20),  // skips " (", stops after "v2"
        ("Ship the release (v2)", 21, 1, 21),  // clamps at the end
        ("Ship the release (v2)", 21, -1, 19), // before "v2"
        ("Ship the release (v2)", 0, -1, 0),   // clamps at the start
    ];
    for (text, from, delta, want) in cases { … }
}
```

Plus one over `"日本語 テスト"` to pin that positions are chars, not bytes.

### 4.4 Parsing and resolution — `tests/task_edit_parse_tests.rs`

Tests 10–12, against `src/domain/task_edit.rs` directly, with no app state.
The messages are asserted on, not just the `Err`: they are what the user reads.
Pin `TUISANA_TODAY` for the keyword cases, and place them around the existing
fixtures' date the way [Milestone 13](13-filter-sets.md) had to — the
integration tests share one process and the variable is global.

### 4.5 The custom-field gid — `tests/task_mutation_tests.rs`

Test 13, the one that is invisible until it corrupts the wrong project's data:

```rust
#[test]
fn a_custom_field_write_uses_the_gid_from_the_tasks_own_project() {
    // "Priority" exists in both projects, with a different gid in each, and
    // the table merges them into one column by name. A write that picked the
    // first gid would set a field the task does not have.
    let client = two_projects_each_declaring_priority();
    …
    assert_eq!(client.update_calls()[0].1, TaskFieldEdit::CustomField {
        gid: "priority-in-beta".to_string(),
        value: Some(CustomFieldValue::Enum { option_gid: "opt-high".into(), name: "High".into() }),
    });
}
```

The same file covers the request shapes: a clear sends JSON `null` rather than
omitting the key, and `update_task` asks for `TASK_OPT_FIELDS` back. Use
`MockTransport` (`src/asana/client.rs:378`) with its new `put_json`.

### 4.6 Reload and clamping — `src/app/task.rs`, `mod tests`

Test 14, modelled on `editing_a_filter_keeps_its_caret_while_projects_stream_in`
(`src/app/task.rs:4895`), which is the same hazard one pane over. Put the
column cursor on the last custom-field column, ingest a project that removes
it, and assert the cursor is in range and the editor is closed.

### 4.7 The scrolling window — `src/ui/text.rs`, `mod tests`

Test 15 at the unit level: `caret_window` keeps the caret inside the returned
window at both ends, never exceeds `width` cells, marks each clipped side, and
counts a wide character as two. Then one assertion each in
`src/ui/task_table.rs` and `src/ui/filter_panel.rs` that a caret past the
column width is visible in the rendered line — `rule_column`
(`src/ui/task_table.rs:1049`) is the existing helper for finding a column in a
rendered line.

### 4.8 Commands and bindings — `src/config/mod.rs` and `src/input/mod.rs`

Extend `default_bindings_include_task_controls` (`src/config/mod.rs:1569`) with
the new task-mode commands, add a task-edit equivalent, and add a
`Display`/`from_command` round trip covering `TaskEditCycleValue(1)` and
`(-1)`. One test in `src/input/mod.rs` for `alt-b` parsing, and one in
`src/ui/hints.rs` for it rendering as `M-b`.

### 4.9 End to end — `tests/task_edit_integration.rs`

`t` to the task view, `l` onto Assignee, `e`, `ctrl-l`, `jo`, `enter` — the
cell reads `jo` and `update_task` was called once with the gid behind `jo`.
Then `space space` to select two rows, `d`, and both are done with two calls.
Then `h h` to Title with two selected, `e`, and the border carries the refusal
while nothing was sent. Finally `l l` to Due, `e`, `t`, `enter`, and the cell
reads today's date.

### 4.10 Snapshots — `tests/ui_snapshot.rs`

Three scenarios at 80/120/200:

- `task-column-cursor`: the cursor on Due, nothing being edited — pins the
  header accent and the underline, and that Gantt snapshots did not move.
- `task-edit-title`: mid-edit on a title longer than its column, caret at the
  end — pins the scrolling window and the caret.
- `task-edit-options`: the Priority picker open with three tasks selected —
  pins the option display and the `editing 3` chip.

## Deferred, deliberately

The rest of the original Milestone 14, and three things found while planning
this one. Each becomes its own milestone rather than growing this one.

- **Creating and deleting tasks.** `POST /tasks` with defaults inherited from
  the row above — its project, its section, its subtask level, its dates —
  and a deletion that is a two-key confirm. This is the obvious next
  milestone, and it is the reason `d` for "done" is worth flagging: when
  delete arrives it must not take that key.
- **Subtask level (`<` / `>`) and moving tasks in the list** (`shift+j` /
  `shift+k`, `m [` / `m ]`). Both re-parent or re-order rather than setting a
  field, both need `addProject`/`section` calls with `insert_before`, and
  moving only means anything in natural sort order. Note that `shift+j` is not
  currently *expressible*: `from_crossterm_event` lowercases every letter.
- **Editing the Projects column.** Membership, not a field: `addProject` and
  `removeProject`, with a section to land in and a project picker to choose
  from.
- **`date`, `multi_enum`, and `people` custom fields.** Refused with a message
  in this milestone. Each needs its own editor, and `people` needs the same
  directory problem solved that §2.2 solves for the assignee.
- **A vim layer for text editing.** The original plan asked for `d`/`y`/`c`/`w`
  /`b`/`$`/`0`/`C`/`D` behaving as in vim. That is a modal editor inside a
  modal editor — an operator-pending state, a register — and it is a feature,
  not a binding table. The emacs motions here are the shared substrate it would
  be built on.
- **`TextEdit` in `CalendarState` and `SidebarPrompt`.** Two more hand-rolled
  caret buffers. The calendar's caret also decides which end of a range the day
  keys rewrite, so folding it in is a behaviour change rather than a
  substitution.
- **Declared enum options in the *filter* panel.** It infers its options from
  observed values; now that the declared ones are fetched, it could offer all
  of them. Tempting, but it changes what an existing filter row shows, and this
  milestone has enough surface already.
- **Undo.** The rollback in §2.7 exists to repair a failed write, not to undo a
  successful one. A real undo stack is worth having and is not free.

## Definition of done

- a column cursor moves through the task table with `h`/`l`, scrolls the table
  to reach an off-screen column, and is drawn only where it can be moved
- `e` edits the cell under it as text, as a date, or as a value picker,
  following the filter panel's vocabulary in each case
- `d` toggles completion from anywhere in the row, and sets a whole selection
  to one state rather than flipping each task
- an edit applies to every selected task, or to the cursor row when nothing is
  selected, with the targets fixed when the editor opens
- a title is refused on a multi-task selection, and says so
- text editing has word and line motions, following emacs, in every field that
  takes text, and a value too long for its column scrolls under the caret
- an edit is applied locally at once, sent to Asana in the background, and put
  back with a message if the request fails
- an un-complete and a cleared date survive the cache merge and the next
  rebuild
- a custom-field write targets the gid belonging to the task's own project, and
  a picker offers the options the field declares rather than the ones tasks
  happen to hold
- all the new keys are configurable, `alt-` bindings parse and render, and the
  hint bar and help overlay list them
- tests cover the merge traps, the rollback, the bulk rules, word motion, date
  and assignee resolution, the custom-field gid, the reload clamps, and the
  caret window, plus an end-to-end key-driven edit and snapshots at 80/120/200
