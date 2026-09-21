# Milestone 13: Filter sets

[← all milestones](../plan.md)

> **A note on the format.** Every milestone before this one is four bullet
> lists — goal, deliverables, implementation notes, acceptance criteria — which
> meant the same fact was written three times in three registers. This one is
> written once, in four passes that do different jobs: **§1** is what the user
> gets, in terms a user would recognize; **§2** is how to build it, complete
> enough to work from without reading the rest of the repository first; **§3**
> is what to test, as a short list someone can argue with; **§4** is how to
> write those tests. Nothing in §2 repeats §1, and §3 is the index to §4.

---

## 1. The plan

Four changes to filtering. The first is the feature; the other three are things
that have annoyed us since [Milestone 11.75](11.75-date-selection.md).

### 1.1 More than one filter set, ORed together

Today the filter panel is one list of fields — Title, Assignee, Due, Start,
State, Projects, and one row per custom field — and filling in more than one
field *narrows* the result. A Due of `..2026-09-01` and an Assignee of `alex`
shows Alex's tasks due before September, not both. That is an AND, and it is the
right default.

What it cannot express is a union. "Everything due in the next week, **plus**
everything due in four months, ignore the three months between" is two
questions, and there is no way to ask both at once.

After this milestone the panel holds one *or more* filter sets:

- Each set is exactly the panel that exists today: the same fields, the same
  match modes, the same calendar, and fields within a set still AND.
- Sets are ORed. A task is shown when it satisfies **any** set.
- A new set starts empty, so adding one never hides a task that was visible
  before — it can only add.
- Sets appear as tabs along the top of the filter pane, numbered `1`, `2`, `3`.
  The active tab is the one being edited; a tab with filters in it carries an
  active marker, so a set doing work is visible while you are standing on
  another one. With a single set there is no strip and the pane looks exactly as
  it does now.
- `a` adds a set after the current one, `x` removes the current one, `h` and `l`
  move between them. `x` is refused when there is only one set — one set is the
  panel, not a set of sets.
- The tab strip is not drawn when there is only one set, so the common case is
  uncluttered; the hint bar advertises `a` regardless, which is where the
  feature is discovered.

The worked example, end to end: `f` opens the panel, `j j` lands on Due, `enter`
opens the calendar, `today..2026-09-28` picks the next week, `enter` commits,
`a` adds a second set, `j j enter` opens its Due, `2027-01` picks four months
out, `enter`. The table now holds both groups and nothing in between.

### 1.2 Filters that require a field to be *empty*

There is currently no way to ask for tasks with no due date, no assignee, or no
value in a custom field. An empty filter field means "do not filter", which is
the opposite.

Pressing `e` on a filter row (`ctrl-e` while editing one) sets that row to
require-empty. The row's value shows as `(none)` in the accent color, it counts
towards the active-filter count like any other filter, and the table narrows to
tasks with nothing in that field. Pressing `e` again clears it; so does typing,
because a value and a require-empty are mutually exclusive.

What "empty" means per field kind:

| Kind | Empty means |
| --- | --- |
| text (Title, Assignee, Projects, text custom fields) | the value is missing or all whitespace |
| labels (custom fields with a small value set) | the task carries no value for the field |
| date (Due, Start) | the task has no such date |

The State row is the exception: a task is always either open or done, so State
can never be empty and the key does nothing there.

Combining it with §1.1 is the point: set 1 asks for tasks due this week, set 2
asks for tasks with no due date at all, and the table shows the work that is
either imminent or unscheduled — which is the review you actually want to do.

### 1.3 Assignee matches fuzzily by default

Title already defaults to fuzzy matching. Assignee defaults to `contains`, which
means `ae` finds nothing and `alex chen` has to be spelled in order. Assignee is
a person's name, typed from memory, and it is the field most likely to be
half-remembered — it gets the same fuzzy default Title has. `ctrl-s` still
switches the row back to `contains` and `ctrl-r` to regex, per row, as before.

### 1.4 The calendar starts the week on Sunday

The date picker's grid is laid out Monday-first: its header reads
`Mo Tu We Th Fr Sa Su`. It should read `Su Mo Tu We Th Fr Sa`, with the day
numbers moving to match.

This is the picker's grid only. The Gantt chart's weekend shading and its
week-boundary ticks are computed from the same weekday numbering and are
deliberately left counting from Monday — see §2.6.

### 1.5 And one change with no user-visible effect at all

The app carries two filter implementations. The panel described above is one;
the other is an older `TaskFilter` in the domain layer whose text, custom-field,
assignee, and date-range fields nothing can set any more — no key reaches them,
and the panel does that work itself now. They are four fields, two types, and a
predicate that only a unit test has exercised since [Milestone 9](09-lazy-filter-aware-task-loading.md).

They get deleted here (§2.11). Nothing on screen changes. The reason it belongs
in *this* milestone rather than a tidy-up of its own: §1.1 and §1.2 both add
meaning to "what filtering means" in this codebase, and leaving a second,
half-dead answer to that question next to the new one is how the next person
extends the wrong one.

---

## 2. How to implement it

### 2.0 Orientation: where filtering lives today

Read this section even if you know the repo; the names below are used without
introduction in the rest of §2.

There are **two** filter mechanisms, and only one of them is the panel.

1. `domain::TaskFilter`, in `src/domain/task.rs`, holds `completed` and
   `subtasks` — driven by the `c` and `z` keys — plus four fields no key
   reaches any more (`text`, `field`, `assignee`, `date_range`). It is applied
   by `apply_task_filter`/`record_matches` inside
   `TaskTableModel::from_records_with_settings`. **§2.11 deletes the four dead
   fields; `completed` and `subtasks` stay exactly as they are.** Read §2.11
   before touching this file, because one part of `apply_task_filter` that looks
   dead afterwards is not.
2. `TaskFilterEditorState`, in `src/app/task.rs` (~line 204), *is* the panel.
   It owns `Vec<TaskFilterFieldState>` and is applied by
   `TaskState::apply_filter_panel` (~line 2266), which runs **before** the table
   model is built:

   ```rust
   let filtered_records = self.apply_filter_panel(&dataset.records);
   self.view.table = TaskTableModel::from_records_with_settings(
       filtered_records, dataset.custom_field_definitions.clone(), &self.view.settings,
   );
   ```

   All four changes in §1 land in mechanism 2.

The pieces of mechanism 2, with line numbers as of [Milestone 12](12-gantt-chart-view.md):

| Item | Where | What it is |
| --- | --- | --- |
| `TaskFieldFilterKind` | `src/app/task.rs:138` | `String` / `Labels` / `Date` |
| `TaskFieldStringMode` | `src/app/task.rs:146` | `Fuzzy` / `Substring` / `Regex` |
| `TaskFilterFieldSpec` | `src/app/task.rs:154` | per-row static metadata: `key`, `label`, `kind`, `custom_gids` |
| `TaskFilterFieldState` | `src/app/task.rs:169` | per-row mutable state: `query`, `query_caret`, `string_mode`, `label_values`, `label_options`, `label_cursor` |
| `TaskFilterPanelEntry` | `src/app/task.rs:186` | the display snapshot handed to the renderer |
| `TaskFilterEditorState` | `src/app/task.rs:204` | `visible`, `editing`, `selected`, `fields`, `calendar` |
| `from_dataset` | `src/app/task.rs:301` | builds the six built-in rows plus one row per custom-field *name* |
| `restore_queries` | `src/app/task.rs:419` | re-applies user state after a rebuild |
| `matches` | `src/app/task.rs:720` | `fields.iter().filter(non-empty query).all(matches)` — the AND |
| `due_date_range_for_query` | `src/app/task.rs:734` | turns the Due row into the API's `due_on.after`/`due_on.before` |
| `TaskFilterFieldState::matches` | `src/app/task.rs:766` | the per-kind predicate |
| panel renderer | `src/ui/filter_panel.rs` | `FilterPanelView` / `FilterRow` / `FilterValue`, and the line builders |
| pane placement | `src/ui/runtime.rs:313` | `render_filter_pane` |
| the picker | `src/app/calendar.rs`, `src/ui/calendar.rs` | `CalendarState` and its overlay |
| actions | `src/input/mod.rs` | `Action` enum, `from_command`, `Display` — **all three must be edited together** |
| bindings | `src/config/mod.rs:496` | `default_bindings()` |

Three things constrain everything below:

- **A rebuild happens after every project finishes loading.**
  `rebuild_visible_dataset` (`src/app/task.rs:1873`) throws the whole editor away
  and rebuilds it from the dataset, then calls `restore_queries(previous)` to put
  the user's state back. Loading streams in one project at a time, so this runs
  several times while someone is typing. Anything new that the user can change
  must be carried across in `restore_queries` or it will disappear mid-load.
  This is not hypothetical: it is the bug 11.75 fixed for the caret.
- **The filter panel rows are one line each.** `filter_panel_lines` returns
  exactly one `Line` per field, which is why the panel's scroll offset can be a
  plain field index (`filter_vertical_scroll`). Do not break that.
- **Most filtering happens here, but the due date is filtered by Asana.**
  This is what the repo calls a *push-down*: sending a filter along with the
  request so the server applies it, instead of fetching everything and
  discarding rows afterwards. Asana's task-list endpoint accepts `due_on.after`
  and `due_on.before`, so a due-date filter can be pushed down; nothing else in
  the panel can be — Asana cannot express fuzzy text, regex, label sets, or
  custom-field values, so those are all matched locally against rows that were
  fetched regardless. `TaskQuery.due_after` / `due_before`
  (`src/asana/mod.rs:69`) are the pushed-down bounds, and
  `due_date_range_for_query` is what computes them from the panel.

  The consequence, which §2.5 is entirely about: a client-side filter can only
  *hide* a task, while a pushed-down one decides whether the app ever receives
  it. And `TaskQuery::covers` (`src/asana/mod.rs:104`) then records the window
  that was fetched as cached, so a push-down that was narrower than the user
  asked for is not corrected on the next keystroke — the rows stay missing
  until something else forces a refresh.

### 2.1 Split the editor into a container and a set

`TaskFilterEditorState` currently is both "the panel" and "the one set of
fields". Separate them. In `src/app/task.rs`, replace the struct at line 204
with:

```rust
/// One filter set: the field list the panel has always shown.
///
/// Every set has the same fields, because they are derived from the same
/// dataset. What differs is what the user typed into them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TaskFilterSet {
    fields: Vec<TaskFilterFieldState>,
}

/// Tracks the filter panel's visibility, edit mode, field cursor, and the
/// filter sets it holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TaskFilterEditorState {
    visible: bool,
    editing: bool,
    /// The field cursor, shared by every set.
    ///
    /// Every set has the same rows in the same order, so one cursor is enough —
    /// and switching sets then keeps you on the row you were looking at, which
    /// is what you want when comparing the same field across two sets.
    selected: usize,
    /// The sets, ORed together. Normally one; never zero once a dataset has
    /// loaded, though `Default` leaves it empty and every accessor tolerates
    /// that.
    sets: Vec<TaskFilterSet>,
    /// Which set the cursor and the keys act on.
    active: usize,
    /// The date picker, while a date field is being edited through it.
    calendar: Option<CalendarState>,
}
```

Then add the accessors that stand in for the old `self.fields`:

```rust
impl TaskFilterEditorState {
    /// The active set's fields, or nothing before a dataset has loaded.
    fn fields(&self) -> &[TaskFilterFieldState] {
        self.sets.get(self.active).map_or(&[], |set| &set.fields)
    }

    fn selected_field(&self) -> Option<&TaskFilterFieldState> {
        self.fields().get(self.selected)
    }

    fn selected_field_mut(&mut self) -> Option<&mut TaskFilterFieldState> {
        let selected = self.selected;
        self.sets
            .get_mut(self.active)
            .and_then(|set| set.fields.get_mut(selected))
    }
}
```

Now rewrite, mechanically, every method on `TaskFilterEditorState` that says
`self.fields`:

- `self.fields.get_mut(self.selected)` → `self.selected_field_mut()`, in
  `clear_current`, `set_mode`, `cycle_mode`, `push_char`, `pop_char`,
  `move_query_caret`, `reset_query_caret`, `sync_calendar_query`,
  `move_label_cursor_left`, `move_label_cursor_right`, `cycle_selected_label`,
  `add_label`, `delete_selected_label`.
- `self.fields.get(self.selected)` → `self.selected_field()`, in
  `selected_kind`, `selected_label`, `open_calendar`.
- `self.fields.is_empty()` / `self.fields.len()` → `self.fields().is_empty()` /
  `self.fields().len()`, in `move_up`, `move_down`, `page_up`, `page_down`.
- `self.fields.iter()` → `self.fields().iter()`, in `active_count` and
  `active_filter_count`. Both keep reporting the **active** set only: the
  panel's "N active" chip describes the rows on screen.

And in `TaskState`, four more call sites:

- `ensure_filter_visible` (line 1174): `self.view.filter_editor.fields.len()` →
  `self.view.filter_editor.fields().len()`.
- `filter_panel_entries` (line 1423): iterate `self.view.filter_editor.fields()`.
- `filter_caret_for` — unchanged except that its `field` argument now comes from
  `fields()`.
- `refresh_table` (lines 2235–2243): both clamps use `fields().len()`.

`impl Default for TaskFilterEditorState` is derived; `sets` starts empty and
every accessor above returns nothing, which is the same behavior
`fields: Vec::new()` had.

### 2.2 OR the sets

Replace `TaskFilterEditorState::matches` (line 720) with:

```rust
impl TaskFilterSet {
    /// Whether this set accepts a record. Fields AND.
    fn matches(&self, record: &TaskRecord) -> bool {
        self.fields
            .iter()
            .filter(|field| field.is_active())
            .all(|field| field.matches(record))
    }

    /// Whether any field in this set excludes anything.
    fn is_active(&self) -> bool {
        self.fields.iter().any(TaskFilterFieldState::is_active)
    }

    /// Clears everything the user typed, keeping the rows and their match modes.
    ///
    /// The match mode is deliberately kept: someone working in regex should not
    /// have to re-pick it in every set they add.
    fn cleared(&self) -> Self {
        let mut set = self.clone();
        for field in &mut set.fields {
            field.query.clear();
            field.query_caret = 0;
            field.label_values.clear();
            field.label_cursor = 0;
            field.empty_required = false;
        }
        set
    }
}

impl TaskFilterEditorState {
    /// Whether a record is visible: **any** set accepts it.
    ///
    /// Fields AND within a set, sets OR between them. A set with nothing in it
    /// accepts everything, which is what a freshly opened panel has always
    /// done — and is why a newly added set can only widen the result.
    fn matches(&self, record: &TaskRecord) -> bool {
        if self.sets.is_empty() {
            return true;
        }
        self.sets.iter().any(|set| set.matches(record))
    }
}
```

Note the filter predicate changed from `!field.query.trim().is_empty()` to
`field.is_active()`; that is §2.3's doing, and it is the only place the AND
needed to learn about require-empty.

`TaskState::apply_filter_panel` needs no change at all — it already delegates to
`matches`.

### 2.3 Require-empty

Add one field to `TaskFilterFieldSpec` and one to `TaskFilterFieldState`:

```rust
struct TaskFilterFieldSpec {
    key: String,
    label: String,
    kind: TaskFieldFilterKind,
    custom_gids: Vec<String>,
    /// Whether "has no value" is a state this field can be in.
    ///
    /// False only for `state`: a task is always either open or done, so a
    /// require-empty there would match nothing while looking like a filter.
    can_be_empty: bool,
}

struct TaskFilterFieldState {
    // ...
    /// Match only records with no value for this field.
    ///
    /// Mutually exclusive with `query`: setting this clears the query, and
    /// typing clears this. "Has no due date" is not expressible as a date
    /// expression, so it is a flag rather than a magic token in the text.
    empty_required: bool,
}
```

Set `can_be_empty: true` on every spec built in `from_dataset` except the
`state` row, and `false` there. `TaskFilterFieldState::new` initializes
`empty_required: false`.

Split the value extraction out of `TaskFilterFieldState::matches` (line 766) so
the require-empty test can share it. The three arms of the existing `match`
become three methods:

```rust
impl TaskFilterFieldState {
    /// The text this field matches against.
    fn haystack(&self, record: &TaskRecord) -> String {
        match self.spec.key.as_str() {
            "title" => record.name.clone(),
            "assignee" => record.assignee.clone().unwrap_or_default(),
            "projects" => record.projects.join(" "),
            key if key.starts_with("custom:") => self.custom_values(record).join(" "),
            _ => String::new(),
        }
    }

    /// The label values this field matches against.
    fn labels(&self, record: &TaskRecord) -> Vec<String> {
        match self.spec.key.as_str() {
            "state" => vec![if record.completed { "done" } else { "open" }.to_string()],
            key if key.starts_with("custom:") => self.custom_values(record),
            _ => Vec::new(),
        }
    }

    /// The date this field matches against.
    fn date<'a>(&self, record: &'a TaskRecord) -> Option<&'a str> {
        match self.spec.key.as_str() {
            "due" => record.due_date.as_deref(),
            "start" => record.start_date.as_deref(),
            _ => None,
        }
    }

    /// Every value this row's custom-field ids carry on a record.
    fn custom_values(&self, record: &TaskRecord) -> Vec<String> {
        self.spec
            .custom_gids
            .iter()
            .filter_map(|gid| record.custom_fields.get(gid))
            .flat_map(|values| values.iter().cloned())
            .collect()
    }

    /// Whether this field excludes anything.
    fn is_active(&self) -> bool {
        self.empty_required || !self.query.trim().is_empty()
    }

    /// Whether the record has nothing at all in this field.
    fn value_is_empty(&self, record: &TaskRecord) -> bool {
        match self.spec.kind {
            TaskFieldFilterKind::String => self.haystack(record).trim().is_empty(),
            TaskFieldFilterKind::Labels => self
                .labels(record)
                .iter()
                .all(|value| value.trim().is_empty()),
            TaskFieldFilterKind::Date => self.date(record).is_none(),
        }
    }

    fn matches(&self, record: &TaskRecord) -> bool {
        if self.empty_required {
            return self.value_is_empty(record);
        }
        match self.spec.kind {
            TaskFieldFilterKind::String => {
                let haystack = self.haystack(record).to_ascii_lowercase();
                let query = self.query.to_ascii_lowercase();
                match self.string_mode {
                    TaskFieldStringMode::Fuzzy => fuzzy_match(&haystack, &query),
                    TaskFieldStringMode::Substring => haystack.contains(&query),
                    TaskFieldStringMode::Regex => regex::RegexBuilder::new(&self.query)
                        .case_insensitive(true)
                        .build()
                        .is_ok_and(|regex| regex.is_match(&haystack)),
                }
            }
            TaskFieldFilterKind::Labels => {
                label_filter_matches(&self.labels(record), &self.label_values)
            }
            TaskFieldFilterKind::Date => date_filter_matches(self.date(record), &self.query),
        }
    }
}
```

The toggle, on the container:

```rust
impl TaskFilterEditorState {
    /// Toggles require-empty on the selected field. Answers whether it changed,
    /// so the caller knows whether to close an open picker.
    fn toggle_require_empty(&mut self) -> bool {
        let Some(field) = self.selected_field_mut() else {
            return false;
        };
        if !field.spec.can_be_empty {
            return false;
        }
        field.empty_required = !field.empty_required;
        if field.empty_required {
            // A value and a require-empty cannot both be in force, and the one
            // the user just asked for wins.
            field.query.clear();
            field.query_caret = 0;
            field.label_values.clear();
            field.label_cursor = 0;
        }
        true
    }
}
```

Four existing methods must clear `empty_required`, because each one is the user
supplying a value:

- `push_char` and `cycle_selected_label` / `add_label`: clear it before
  inserting.
- `open_calendar`: clear it — you are about to pick a date.
- `clear_current`: clear it, so `ctrl-l` resets the row completely.

Wire it through `TaskState`:

```rust
    pub(crate) fn filter_toggle_require_empty(&mut self) {
        if self.view.filter_editor.toggle_require_empty() {
            // The picker is bound to the value it was opened on, and there is no
            // longer a value.
            self.view.filter_editor.calendar = None;
            self.refresh_table();
        }
    }
```

Show it in the panel. `TaskFilterPanelEntry` gains `pub empty_required: bool`,
and `filter_panel_entries` renders the value as a shared constant:

```rust
/// What a require-empty filter shows as its value.
///
/// A word rather than the `empty` glyph: that glyph already means "unset" in
/// this column, and "unset" and "require unset" are opposites.
pub const EMPTY_REQUIRED_TEXT: &str = "(none)";
```

In `filter_panel_entries`, the `query` for a field with `empty_required` is
`EMPTY_REQUIRED_TEXT` regardless of kind, and `filter_caret_for` returns `None`
for it (there is no text to put a caret in).

In `src/ui/filter_panel.rs`, add a variant rather than smuggling a magic string
through `FilterValue::Text`:

```rust
pub enum FilterValue {
    Text(String),
    Labels { values: Vec<String>, cursor: Option<usize> },
    /// The field must have no value at all.
    RequireEmpty,
}
```

`FilterValue::is_active` returns `true` for `RequireEmpty`; `value_spans`
renders it as `Span::styled(EMPTY_REQUIRED_TEXT, theme.accent)`; `filter_row`
maps `entry.empty_required` to it before it looks at `entry.kind`.

### 2.4 Adding, removing, and switching sets

```rust
impl TaskFilterEditorState {
    fn set_count(&self) -> usize {
        self.sets.len()
    }

    fn active_set_index(&self) -> usize {
        self.active
    }

    /// Adds an empty set after the active one and moves to it.
    ///
    /// Empty, so the result can only widen: a set that excluded something would
    /// make "add a filter set" hide rows, which is the opposite of what the tab
    /// is for.
    fn add_set(&mut self) {
        let fresh = match self.sets.get(self.active) {
            Some(set) => set.cleared(),
            None => TaskFilterSet::default(),
        };
        self.sets.insert(self.active + 1, fresh);
        self.active += 1;
        self.stop_editing();
    }

    /// Removes the active set. Refused at one set: one set is the panel.
    fn remove_set(&mut self) {
        if self.sets.len() <= 1 {
            return;
        }
        self.sets.remove(self.active);
        self.active = self.active.min(self.sets.len() - 1);
        self.stop_editing();
    }

    /// Moves to another set, wrapping.
    fn select_set(&mut self, delta: i32) {
        if self.sets.len() <= 1 {
            return;
        }
        let len = self.sets.len() as i32;
        self.active = (self.active as i32 + delta).rem_euclid(len) as usize;
        // The picker and the text caret belong to the field they were opened
        // on, which is in the set being left.
        self.stop_editing();
        self.reset_query_caret();
    }
}
```

`stop_editing` already clears `editing` and `calendar`, which is why it is
called from all three.

`TaskState` wrappers, each refreshing the table because the visible rows change:

```rust
    pub(crate) fn filter_add_set(&mut self) {
        self.view.filter_editor.add_set();
        self.refresh_table();
    }

    pub(crate) fn filter_remove_set(&mut self) {
        self.view.filter_editor.remove_set();
        self.refresh_table();
    }

    pub(crate) fn filter_select_set(&mut self, delta: i32) {
        self.view.filter_editor.select_set(delta);
        self.refresh_table();
    }

    /// `(active index, total)`, for the pane's chips and the tab strip.
    pub fn filter_set_position(&self) -> (usize, usize) {
        (
            self.view.filter_editor.active_set_index(),
            self.view.filter_editor.set_count(),
        )
    }

    /// How many fields each set filters on, in tab order.
    pub fn filter_set_counts(&self) -> Vec<usize> {
        self.view
            .filter_editor
            .sets
            .iter()
            .map(|set| set.fields.iter().filter(|f| f.is_active()).count())
            .collect()
    }
```

Route the five new actions inside the block in `TaskState::apply_action` that
already intercepts keys while the panel is visible (line 2025). Add them next to
`Action::ClearSearch`:

```rust
                Action::FilterSetAdd => { self.filter_add_set(); return None; }
                Action::FilterSetRemove => { self.filter_remove_set(); return None; }
                Action::FilterSetNext => { self.filter_select_set(1); return None; }
                Action::FilterSetPrev => { self.filter_select_set(-1); return None; }
                Action::FilterRequireEmpty => { self.filter_toggle_require_empty(); return None; }
```

`src/app.rs` needs **no** new arm in `handle_action`: these fall through its
`_ => {}`, reach `self.tasks.apply_action(...)`, and then hit the
`update_task_data_after_action` call at the end of `handle_action`, which is what
re-fetches when the due-date window sent to Asana (§2.5) has widened. Do not add
an early `return` for them in `handle_action` or that re-fetch is skipped.

### 2.5 The due-date filter sent to Asana must cover the *union*

The one part of this milestone that can silently produce wrong results rather
than a visible bug, because it changes which tasks are fetched at all. See the
third bullet in §2.0 if "push-down" is new.

`desired_task_query` (line 1496) turns the Due row into `due_on.after` /
`due_on.before` on the Asana request, and `TaskQuery::covers`
(`src/asana/mod.rs:104`) records the fetched window as covering that range.
With two sets, the server has to return the union of their windows — the
earliest `after` and the latest `before` — and any set that is unbounded on a
side makes the union unbounded on that side. A set that *requires an empty* due
date is unbounded on both: `due_on.after` would drop exactly the undated tasks
it asked for, and the cache would then believe that window was covered, so the
rows would not come back until the next refresh.

Replace `due_date_range_for_query` (line 734) with:

```rust
    /// The due-date bounds to push down to the API, across every set.
    ///
    /// Sets OR, so the server must return the union of their windows: the
    /// earliest `after`, the latest `before`. Anything that makes one side
    /// unbounded — a set with no due filter, an unparseable one, a set asking
    /// for tasks with *no* due date — drops that side entirely. Narrowing here
    /// does not merely fetch too little: `TaskQuery::covers` would then record
    /// the narrow window as cached, so the missing tasks stay missing.
    fn due_date_range_for_query(&self) -> (Option<String>, Option<String>) {
        let mut after: Option<CivilDate> = None;
        let mut before: Option<CivilDate> = None;

        for set in &self.sets {
            let Some(field) = set.fields.iter().find(|f| f.spec.key == "due") else {
                return (None, None);
            };
            if field.empty_required {
                return (None, None);
            }
            let Some(query) = DateQuery::parse(&field.query, date::today()) else {
                return (None, None);
            };
            let (set_after, set_before) = query.bounds();
            match set_after {
                None => return (None, None),
                Some(date) => after = Some(after.map_or(date, |current| current.min(date))),
            }
            match set_before {
                None => return (None, None),
                Some(date) => before = Some(before.map_or(date, |current| current.max(date))),
            }
        }

        (after.map(|date| date.iso()), before.map(|date| date.iso()))
    }
```

An early `return (None, None)` is correct in every one of those branches: one
unbounded set makes the union unbounded, so no later set can narrow it. An empty
`sets` falls out of the loop with `(None, None)`, as before.

`CivilDate` derives `Ord`, so `min`/`max` work directly; it is already imported
in this module.

### 2.6 The calendar's week starts on Sunday

The grid offset comes from `CivilDate::weekday_index` (`src/domain/date.rs:160`),
which counts Monday as 0. **Do not change it.** Three other things depend on
that numbering:

- `src/domain/gantt.rs:410` and `:460` shade weekends with `weekday_index() >= 5`.
- `src/domain/gantt.rs:436` marks week ticks with `weekday_index() == 0`, i.e.
  Mondays, which is what puts the week boundary where a work week starts.
- `weekday_index_from_name` (`src/domain/date.rs:357`) maps `mon`…`sun` to the
  same indices, and `parse_token` uses it for the `mon`/`tue`/... filter
  keywords.

Add a calendar-specific projection instead. In `src/domain/date.rs`:

```rust
/// Weekday labels in the order a calendar grid shows them.
///
/// Separate from [`WEEKDAYS`], which starts on Monday because that is the order
/// [`CivilDate::weekday_index`] counts in — and the Gantt chart's weekend
/// shading and week ticks are written against those numbers. Only the picker's
/// grid starts on Sunday.
pub const CALENDAR_WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
```

and on `CivilDate`:

```rust
    /// The column this date occupies in a Sunday-first calendar grid.
    pub fn calendar_column(&self) -> usize {
        (self.weekday_index() + 1) % 7
    }
```

Export `CALENDAR_WEEKDAYS` from `src/domain/mod.rs:13` alongside `WEEKDAYS`.

Then two one-line changes:

- `src/app/calendar.rs:235`, in `weeks()`: `first.weekday_index()` →
  `first.calendar_column()`. The `if index == 6 { push }` end-of-week test is
  still right — Saturday is column 6 — and so is everything else in the
  function. Update its doc comment (line 229) from "starting Monday" to
  "starting Sunday".
- `src/ui/calendar.rs:174`: `crate::domain::WEEKDAYS` →
  `crate::domain::CALENDAR_WEEKDAYS`. Update the `CalendarView::weeks` doc
  comment (line 68) and the module docstring's mention of the grid.

Expect the overlay to grow a row in some months: August 2026 starts on a
Saturday, so Monday-first fitted it in five week rows (Aug 1–2 sharing the first)
while Sunday-first needs six (Aug 1 alone, then 2–8). The box is sized from
`lines.len()` in `render`, so it grows on its own — but every
`filter-date-calendar*` snapshot moves.

### 2.7 The assignee default

`src/app/task.rs:320`, in `from_dataset`: the `assignee` row's second argument to
`TaskFilterFieldState::new` becomes `TaskFieldStringMode::Fuzzy`. One word. It
changes the `contains` chip to `fuzzy` in every filter snapshot and in
`filter_panel::tests::match_modes_render_as_short_chips`.

### 2.8 Carry the sets across a reload

The most important twenty lines in this milestone. `from_dataset` builds one set
of empty fields; `restore_queries` has to rebuild the user's set list on top of
it. Replace `restore_queries` (line 419) with:

```rust
    /// Carries everything the user did across a rebuild.
    ///
    /// `from_dataset` builds one empty set from whatever records have arrived so
    /// far, and loading streams in one project at a time — so this runs several
    /// times while someone is typing. The set list, the active tab, the field
    /// cursor, every set's queries, and the in-progress edit all have to be
    /// re-applied here. Before 11.75 the caret snapped back to 0 between
    /// keystrokes for exactly this reason; a set list that did not survive
    /// would vanish mid-load the same way.
    fn restore_queries(&mut self, previous: Self) {
        self.visible = previous.visible;
        self.editing = previous.editing && self.visible;
        self.calendar = if self.editing { previous.calendar } else { None };

        let template = self.sets.first().cloned().unwrap_or_default();
        self.sets = previous
            .sets
            .iter()
            .map(|old| {
                let mut set = template.clone();
                set.restore_from(old);
                set
            })
            .collect();
        if self.sets.is_empty() {
            self.sets.push(template);
        }

        self.active = previous.active.min(self.sets.len() - 1);
        self.selected = previous.selected.min(self.fields().len().saturating_sub(1));
    }
```

and move the existing per-field loop onto the set:

```rust
impl TaskFilterSet {
    /// Re-applies one set's user state onto a freshly built field list.
    ///
    /// Matched by `spec.key`, which is why custom-field rows are keyed by name
    /// rather than id: a reload can bring a different set of projects, and so
    /// different ids, for the same field.
    fn restore_from(&mut self, previous: &Self) {
        for field in &mut self.fields {
            let Some(old) = previous
                .fields
                .iter()
                .find(|old| old.spec.key == field.spec.key)
            else {
                continue;
            };
            field.string_mode = old.string_mode;
            field.empty_required = old.empty_required && field.spec.can_be_empty;
            match field.spec.kind {
                TaskFieldFilterKind::Labels => {
                    field.label_values = if old.label_values.is_empty() {
                        parse_label_values(&old.query)
                    } else {
                        old.label_values.clone()
                    };
                    field.label_cursor =
                        old.label_cursor.min(field.label_values.len().saturating_sub(1));
                    field.query = field.label_values.join(" | ");
                }
                _ => field.query = old.query.clone(),
            }
            // Clamped because a custom field can change kind between loads as
            // values arrive, which rewrites `query` under the caret.
            field.query_caret = old.query_caret.min(field.query.chars().count());
        }
    }
}
```

`empty_required && field.spec.can_be_empty` is not belt-and-braces: a custom
field's kind is inferred from the values seen so far (`from_dataset`, line 379),
so a row's spec really can change between loads.

### 2.9 The tab strip

In `src/ui/filter_panel.rs`:

```rust
/// One tab in the filter pane's tab strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterTab {
    /// The tab's number, as shown: `1`, `2`, ...
    pub label: String,
    /// Whether this is the set the keys act on.
    pub active: bool,
    /// How many of its fields filter anything, so a working set is visible from
    /// another tab.
    pub filters: usize,
}
```

`FilterPanelView` gains `pub tabs: Vec<FilterTab>`. `render_filter_panel` fills
it from `state.filter_set_position()` and `state.filter_set_counts()`, and
**leaves it empty when there is one set** — a lone `1` tab is noise, and with one
set the pane then renders byte-identically to today, which keeps the existing
snapshots honest about what changed. Also push a set chip into `counts` when
there is more than one:

```rust
    let (active, total) = state.filter_set_position();
    if total > 1 {
        counts.push(Chip::toned(format!("set {}/{total}", active + 1), Tone::Info));
    }
```

The strip itself:

```rust
/// Renders the tab strip: one tab per filter set, the active one picked out.
///
/// Drawn as the pane's first interior line rather than in the border. The
/// number of sets is unbounded and the border truncates from the left — the
/// mistake that turned "Aug 2026" into "6" in the calendar overlay.
///
/// The separator is the word `or`, because that is the whole semantics of the
/// strip and there is nowhere else on screen to say it.
pub fn tab_strip_line(view: &FilterPanelView, theme: &Theme, width: usize) -> Line<'static> {
    let glyphs = &theme.glyphs;
    // The same gutter the rows use, so the tabs line up with the label column.
    let mut spans = vec![Span::raw(" ".repeat(MARKER_WIDTH))];

    for (index, tab) in view.tabs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" or ".to_string(), theme.muted));
        }
        let text = if tab.filters > 0 {
            format!(" {}{} ", tab.label, glyphs.active)
        } else {
            format!(" {} ", tab.label)
        };
        spans.push(Span::styled(
            text,
            if tab.active {
                theme.accent.add_modifier(Modifier::REVERSED)
            } else {
                theme.muted
            },
        ));
    }

    Line::from(slice_spans(&spans, 0, width))
}
```

In `src/ui/runtime.rs`, `render_filter_pane` (line 313), after
`frame.render_widget(block, area)` and the message early-return:

```rust
    // The strip is a line of the pane's interior, not of its border, so the
    // rows below it keep one line each and the scroll offset stays a plain
    // field index.
    let body = if view.tabs.is_empty() || inner.height < 2 {
        inner
    } else {
        let (strip, body) = split_header(inner);
        frame.render_widget(
            Paragraph::new(filter_panel::tab_strip_line(&view, theme, strip.width as usize)),
            strip,
        );
        body
    };

    app.tasks.ensure_filter_visible(body.height as usize);
    let lines = filter_panel::filter_panel_lines(&view, theme, body.width as usize);
    frame.render_widget(
        Paragraph::new(lines).scroll((app.tasks.filter_panel_scroll() as u16, 0)),
        body,
    );

    body.height.max(1) as usize
```

`split_header` already exists in that module (line 413) and does exactly this
for the task pane's column header; reuse it rather than writing a second one.
Returning `body.height` as the page size, not `inner.height`, keeps `ctrl-d`
paging by what is actually visible.

`MIN_TOP_PANE_HEIGHT` (`src/ui/layout.rs:22`) stays at 6: the `inner.height < 2`
guard drops the strip before it can squeeze the last row out, and at the minimum
height there are still three rows plus the strip.

### 2.10 Actions, bindings, hints, help, docs

Five new actions. Add each to **all three** places in `src/input/mod.rs` — the
`Action` enum, `from_command`, and `Display` — or the config round-trip breaks
silently:

| Action | Command name |
| --- | --- |
| `FilterSetNext` | `filter_set_next` |
| `FilterSetPrev` | `filter_set_prev` |
| `FilterSetAdd` | `filter_set_add` |
| `FilterSetRemove` | `filter_set_remove` |
| `FilterRequireEmpty` | `filter_require_empty` |

Default bindings, appended to `default_bindings()` in `src/config/mod.rs` next to
the other `Mode::Filter` binds:

```rust
        Bind::with_mode("l", Mode::Filter, "filter_set_next"),
        Bind::with_mode("h", Mode::Filter, "filter_set_prev"),
        Bind::with_mode("a", Mode::Filter, "filter_set_add"),
        Bind::with_mode("x", Mode::Filter, "filter_set_remove"),
        Bind::with_mode("e", Mode::Filter, "filter_require_empty"),
        // Letters type while editing, so the require-empty key needs a ctrl-
        // pair there. ctrl-e is free in filter-edit mode; in calendar mode it
        // is already "jump to the end of a range", which is why the date
        // fields' require-empty is set from filter-browse mode, before the
        // picker opens.
        Bind::with_mode("ctrl-e", Mode::FilterEdit, "filter_require_empty"),
```

Check each key is free in `Mode::Filter` before trusting this list: `h`, `l`,
`a`, `x`, and `e` are all unbound there today, and `Mode::Filter` falls back to
`Mode::Any`, which binds none of them either. `h` and `l` mean label motion in
`Mode::FilterEdit` and day motion in `Mode::Calendar`; both shadow these, which
is correct — you switch sets from browse mode, not mid-edit.

Hints (`src/ui/hints.rs`, the `Mode::Filter` arm, ~line 89). The set hints are
only worth a slot once they can do something, except `add`, which is how the
feature is found:

```rust
        Mode::Filter => {
            let mut hints = vec![
                Hint::new(&[Action::BeginFilterEdit], "edit"),
                Hint::new(&[Action::MoveDown, Action::MoveUp], "field"),
                Hint::new(&[Action::FilterRequireEmpty], "require empty"),
                Hint::new(&[Action::CycleFilterStringMode], "match mode"),
                Hint::new(&[Action::ClearSearch], "clear"),
                Hint::new(&[Action::FilterSetAdd], "add set"),
            ];
            if context.many_filter_sets {
                hints.push(Hint::new(
                    &[Action::FilterSetPrev, Action::FilterSetNext],
                    "set",
                ));
                hints.push(Hint::new(&[Action::FilterSetRemove], "remove set"));
            }
            hints.push(Hint::new(&[Action::ToggleTaskFilters], "close"));
            hints.push(Hint::new(&[Action::ToggleHelpDetails], "help"));
            hints
        }
```

`HintContext` gains `pub many_filter_sets: bool`, filled in
`render_hint_bar` (`src/ui/runtime.rs:249`) from
`app.tasks.filter_set_position().1 > 1`. This follows `timeline_windowed`
exactly.

Help overlay (`src/ui/help_overlay.rs`, `filter_groups`): a new group before
"Label values", and one line added to "Filters":

```rust
        HelpGroup::new(
            "Filter sets",
            vec![
                Hint::new(&[Action::FilterSetAdd], "add a set (ORed)"),
                Hint::new(&[Action::FilterSetRemove], "remove this set"),
                Hint::new(&[Action::FilterSetPrev], "previous set"),
                Hint::new(&[Action::FilterSetNext], "next set"),
                Hint::literal("within a set", "fields narrow (AND)"),
                Hint::literal("between sets", "results combine (OR)"),
            ],
        ),
```

and `Hint::new(&[Action::FilterRequireEmpty], "require no value")` in the
"Filters" group.

Docs — all four, in the same commit:

- `README.md` §"Key bindings": the five commands under **Filter mode** (and
  `filter_require_empty` under **Filter edit mode**); a "Filter sets" subsection
  under the filter-panel shortcuts explaining AND-within / OR-between, the tab
  strip, and the worked example from §1.1; the require-empty key and its
  per-kind table from §1.2; Assignee's new default in the filter panel
  description; and the calendar's week start where the picker is described.
- `tuisana.toml.example`: a `[[bind]]` block per new command, matching the
  documented defaults.
- `developer.md`: `src/app/task.rs`'s entry in "Start Here" already says it
  "manages task state, filters, sorting" — add a line to "Core Concepts" saying
  that the filter panel holds one or more `TaskFilterSet`s ORed together and
  that `restore_queries` is what carries them across a streaming reload.
- `codex-notes.md` needs nothing.

### 2.11 Delete the dead half of `domain::TaskFilter`

Independent of §2.1–§2.10: it shares no code with them, so it wants its own
commit and can land in either order.

**What is dead, and how it was confirmed.** `TaskFilter` (`src/domain/task.rs:332`)
has six fields. `completed` and `subtasks` are live — `TaskState` writes them
(lines 1982–1997), `ui::task_table::settings_chips` reads them, and
`desired_load_scope` turns `completed` into the fetch scope. The other four —
`text`, `field`, `assignee`, `date_range` — are written in exactly one place in
the whole repo: the test `filters_by_text_field_owner_completion_and_date`
(`src/domain/task.rs:1402`). Every other `TaskFilter { .. }` literal, in
`src/domain/task.rs` and `src/domain/gantt.rs:1273`, sets only `completed` or
`subtasks` and spreads `..TaskFilter::default()`, so all of them compile
unchanged after the removal. Re-run the check before starting:

```sh
grep -rn 'TaskFilter {' src tests     # every literal should use ..default()
grep -rEn 'filter\.(text|field|assignee|date_range)' src tests
grep -rEn 'TaskFieldFilter|TaskDateRange' src tests
```

**Delete:**

- the four fields from `TaskFilter`, and their lines from `impl Default`.
- `TaskFieldFilter` (`src/domain/task.rs:380`) and `TaskDateRange`
  (`src/domain/task.rs:388`) entirely — they exist only to hold two of those
  fields. Drop both from the re-export list in `src/domain/mod.rs:25`, and from
  the test module's `use super::{...}` at `src/domain/task.rs:1236`.
- the four predicate blocks from `record_matches`
  (`src/domain/task.rs:1094`–`1152`), which leaves only the `completed` check.

**Then fold what is left of `record_matches` into its caller.** A function whose
whole body is one `Option` comparison does not earn a name; and once it is
inlined, the group-keeping condition in `apply_task_filter` collapses. Replace
both functions (lines 1046–1155) with:

```rust
/// Whether a record satisfies the completed-state filter.
fn completion_matches(record: &TaskRecord, filter: &TaskFilter) -> bool {
    filter
        .completed
        .is_none_or(|completed| record.completed == completed)
}

/// Applies the settings-level filter: completion state and subtask visibility.
///
/// This is only what the `c` and `z` keys drive. Everything the filter panel
/// asks for was already applied by `TaskState::apply_filter_panel`, before the
/// records reached this module.
///
/// The `Show` branch's grouping is load-bearing and must not be simplified away
/// with the predicate that used to justify it: `TaskSort::compare` sorts *every*
/// parent before *every* subtask, so emitting one parent-group at a time is the
/// only thing putting a subtask underneath its own parent rather than in a heap
/// at the bottom of the table.
fn apply_task_filter(records: Vec<TaskRecord>, filter: &TaskFilter) -> Vec<TaskRecord> {
    match filter.subtasks {
        SubtaskVisibility::Hide => records
            .into_iter()
            .filter(|record| !record.is_subtask() && completion_matches(record, filter))
            .collect(),
        SubtaskVisibility::Show => {
            let mut group_order = Vec::new();
            let mut groups: HashMap<String, Vec<TaskRecord>> = HashMap::new();

            for record in records {
                let key = record
                    .parent_gid
                    .clone()
                    .unwrap_or_else(|| record.gid.clone());
                if !groups.contains_key(&key) {
                    group_order.push(key.clone());
                }
                groups.entry(key).or_default().push(record);
            }

            group_order
                .into_iter()
                .filter_map(|key| groups.remove(&key))
                .flatten()
                .filter(|record| completion_matches(record, filter))
                .collect()
        }
    }
}
```

The `if group.iter().any(record_matches)` guard is gone, and that is a real
change of shape rather than a rewrite: it existed so that a parent whose
*subtask* matched a text or date filter came along too. With those filters gone
it can only ever have been true for a group that also passes per-record, so it
decided nothing. Note what this means for the future, though — see below.

**Two things deliberately *not* done here:**

- **`TaskFilter` keeps its name**, even though "the completed and subtask
  toggles" is what it now is. Renaming it touches every literal in three test
  modules and buys nothing this milestone.
- **The parent-comes-along-too behaviour is not re-implemented for the panel.**
  It is a plausible feature — filter to `alex` and you arguably want the parent
  of Alex's subtask for context — but it is not what the panel does today, and
  `apply_filter_panel` judges each record alone. Deleting an unreachable
  implementation of it is not the same as deciding against it. If we want it,
  it belongs in `apply_filter_panel`, needs to decide what "the parent of a
  match in set 2" means now that sets exist, and is its own milestone item.
  It is recorded under "Deferred" below so that its absence is not rediscovered
  as a bug.

---

## 3. The tests that matter

Thirteen, grouped by what would break without them. The first three are the
ones that would let a real bug through; 13 guards the deletion in §2.11.

1. **Sets OR while fields AND.** The core semantics, at the unit level.
2. **The due-date filter sent to Asana is the union of the sets**, and is
   dropped entirely when any set is unbounded. The bug that loses rows *and*
   caches the loss, so they stay lost.
3. **Sets survive a streaming reload.** The 11.75 caret bug, one level up.
4. **Add and remove.** A new set is empty and widens nothing; remove is refused
   at one set; the field cursor and the active index stay in range.
5. **Require-empty matches only valueless records, per kind.**
6. **Require-empty and a query are mutually exclusive**, in both directions.
7. **Require-empty is refused on State**, the one field that can never be empty.
8. **Assignee defaults to fuzzy**, and can still be switched per row.
9. **The calendar grid starts on Sunday**, in the header and in the day
   placement.
10. **The Gantt chart's Monday-based weekends and week ticks are unchanged.**
    The regression guard for the change in 9.
11. **The five new commands parse and are bound**, and the hint bar and help
    overlay show rebound keys rather than the defaults.
12. **End-to-end, driven by keys**, plus snapshots at 80/120/200.
13. **Subtasks still sit under their parents, and `c` / `z` still work**, after
    the dead `TaskFilter` fields and the `any()` guard are gone. The existing
    test that covered the ordering did it *through* one of the deleted fields,
    so it has to be replaced rather than simply removed.

---

## 4. How to write them

### 4.1 Sets OR while fields AND — `src/app/task.rs`, `mod tests`

The module's tests build state through `FakeAsanaClient` and
`load_task_dataset_for_projects`; see `label_filters_support_multiple_selected_values_and_label_navigation`
(line 3443) for the pattern to copy, including how it reaches into
`state.view.filter_editor` from inside the module.

```rust
#[test]
fn filter_sets_or_while_their_fields_still_and() {
    // Set 1: Alex's tasks due in August. Set 2: anything due in December,
    // whoever owns it. The August-but-not-Alex task is in neither.
    let mut state = loaded_state_with_tasks(vec![
        sel_task_due("alex-aug", "Alex August", "Alex", "2026-08-10"),
        sel_task_due("jo-aug", "Jo August", "Jo", "2026-08-11"),
        sel_task_due("jo-dec", "Jo December", "Jo", "2026-12-01"),
    ]);
    state.toggle_filter_panel();

    set_field(&mut state, "assignee", "alex");
    set_field(&mut state, "due", "2026-08-01..2026-08-31");
    assert_eq!(visible_gids(&state), vec!["alex-aug"], "one set still ANDs");

    state.filter_add_set();
    set_field(&mut state, "due", "2026-12-01");

    assert_eq!(
        visible_gids(&state),
        vec!["alex-aug", "jo-dec"],
        "and the two sets union"
    );
}
```

Three helpers to add to the module's test section, because §4.1–4.7 all want
them:

```rust
/// Puts `query` in the named field of the active set.
///
/// Goes through the public keys rather than the struct so the test exercises
/// the same path a user does: move the cursor to the row, then type.
fn set_field(state: &mut TaskState, key: &str, query: &str) {
    let index = state
        .view
        .filter_editor
        .fields()
        .iter()
        .position(|field| field.spec.key == key)
        .expect("the field exists");
    state.view.filter_editor.selected = index;
    state.filter_clear_current();
    for ch in query.chars() {
        state.filter_push_char(ch);
    }
}

fn select_field(state: &mut TaskState, key: &str) { /* the position lookup alone */ }

/// The gids of the visible task rows, in table order.
fn visible_gids(state: &TaskState) -> Vec<&str> {
    state
        .table()
        .rows
        .iter()
        .filter(|row| row.kind.is_task())
        .map(|row| row.gid.as_str())
        .collect()
}
```

Also assert the degenerate cases in the same test file, because they are the
ones a future refactor will get wrong:

```rust
#[test]
fn an_empty_set_accepts_everything_so_adding_one_can_only_widen() {
    let mut state = loaded_state_with_tasks(vec![/* three tasks */]);
    state.toggle_filter_panel();
    set_field(&mut state, "assignee", "alex");
    let narrowed = visible_gids(&state).len();

    state.filter_add_set();

    assert_eq!(visible_gids(&state).len(), 3, "the empty set lets everything through");
    assert!(narrowed < 3, "and the first set really was narrowing");
}
```

### 4.2 The union of the due-date filters — `src/app/task.rs`, `mod tests`

Extend the four existing `due_date_range_for_query_*` tests (lines 3641–3688)
rather than replacing them; they cover the one-set cases and must keep passing.
Add:

```rust
#[test]
fn the_pushed_down_due_window_is_the_union_of_the_sets() {
    std::env::set_var("TUISANA_TODAY", "2026-08-24");
    let mut state = loaded_state_with_tasks(vec![/* any task */]);
    state.toggle_filter_panel();

    set_field(&mut state, "due", "2026-09-01..2026-09-07");
    state.filter_add_set();
    set_field(&mut state, "due", "2026-12-01..2026-12-31");

    let query = state.desired_task_query();
    assert_eq!(query.due_after.as_deref(), Some("2026-09-01"), "the earliest start");
    assert_eq!(query.due_before.as_deref(), Some("2026-12-31"), "the latest end");
}

#[test]
fn a_set_with_no_due_filter_pushes_down_no_due_window_at_all() {
    // Otherwise the server drops the very tasks the second set asked for, and
    // TaskQuery::covers then records the narrow window as cached.
    let mut state = /* ... */;
    set_field(&mut state, "due", "2026-09-01..2026-09-07");
    state.filter_add_set();

    let query = state.desired_task_query();
    assert_eq!(query.due_after, None);
    assert_eq!(query.due_before, None);
}

#[test]
fn a_set_requiring_an_empty_due_date_pushes_down_no_due_window() {
    let mut state = /* ... */;
    set_field(&mut state, "due", "2026-09-01..2026-09-07");
    state.filter_add_set();
    select_field(&mut state, "due");
    state.filter_toggle_require_empty();

    let query = state.desired_task_query();
    assert_eq!(query.due_after, None, "undated tasks would be filtered out server-side");
    assert_eq!(query.due_before, None);
}

#[test]
fn an_open_ended_set_opens_that_side_of_the_union() {
    // `2026-09-01..` has no end, so the union has none either, however tight
    // the other set is.
    let mut state = /* ... */;
    set_field(&mut state, "due", "2026-09-01..");
    state.filter_add_set();
    set_field(&mut state, "due", "2026-10-01..2026-10-02");

    let query = state.desired_task_query();
    assert_eq!(query.due_after.as_deref(), Some("2026-09-01"));
    assert_eq!(query.due_before, None);
}
```

Then the consequence, which is the part that actually bites — model it on
`lazy_load_uses_date_filter_from_filter_state` (line 3889) and
`task_query_covers_detects_cache_hits_and_misses` (line 3598):

```rust
#[test]
fn adding_a_set_outside_the_cached_window_is_a_cache_miss() {
    // The cache is keyed by the query it was filled with, so widening the
    // union has to stop reporting coverage or the new set shows nothing.
    let projects = vec![Project::new("p1", "Inbox", true)];
    let mut state = /* loaded with due 2026-09-01..2026-09-07 in one set */;
    assert!(state.can_serve_query_for_targets(&projects, &state.desired_task_query()));

    state.filter_add_set();
    set_field(&mut state, "due", "2026-12-01");

    assert!(
        !state.can_serve_query_for_targets(&projects, &state.desired_task_query()),
        "the December set needs a fetch"
    );
    assert_eq!(state.projects_requiring_load(&projects, &state.desired_task_query()).len(), 1);
}
```

### 4.3 Sets survive a streaming reload — `src/app/task.rs`, `mod tests`

Copy `editing_a_filter_keeps_its_caret_while_projects_stream_in` (line 3534): it
already loads one project, mutates the editor, loads a second, and asserts the
state survived. That is exactly the shape needed.

```rust
#[test]
fn filter_sets_survive_the_rebuild_that_each_streamed_project_triggers() {
    // rebuild_visible_dataset throws the editor away per project; without
    // restore_queries rebuilding the set list, the second tab vanishes
    // mid-load — the same failure the caret had before 11.75.
    let mut state = /* loaded with project p1 */;
    state.toggle_filter_panel();
    set_field(&mut state, "assignee", "alex");
    state.filter_add_set();
    set_field(&mut state, "due", "2026-12-01");
    state.filter_edit_begin();

    state.load_task_dataset_for_projects(&client, &[p1.clone(), p2.clone()]).expect("loads");

    assert_eq!(state.filter_set_position(), (1, 2), "two sets, still on the second");
    assert_eq!(state.filter_panel_rows_for_set(0)[1].1, "alex");
    assert_eq!(state.filter_panel_rows()[2].1, "2026-12-01");
    assert!(state.filter_panel_editing(), "and the edit was not interrupted");
}
```

That needs one new test-facing accessor next to `filter_panel_rows` (line 1294):

```rust
    /// One set's rows as `(label, query)`, for tests that need to see a tab
    /// other than the active one.
    pub fn filter_panel_rows_for_set(&self, index: usize) -> Vec<(String, String)> { /* ... */ }
```

Also assert the custom-field case, since that is where `restore_from`'s
key-matching earns its keep: load a project whose custom field arrives with a
different gid in the second batch, and check the set's query on that row
survived. `a_custom_field_shared_by_name_across_projects_is_one_row_and_one_column`
(line 2561) already builds two projects with same-named, different-id fields —
reuse its fixture with two sets.

### 4.4 Add and remove — `src/app/task.rs`, `mod tests`

```rust
#[test]
fn a_new_set_starts_empty_and_lands_after_the_current_one() {
    let mut state = /* loaded */;
    state.toggle_filter_panel();
    set_field(&mut state, "assignee", "alex");

    state.filter_add_set();

    assert_eq!(state.filter_set_position(), (1, 2));
    assert_eq!(state.filter_set_counts(), vec![1, 0]);
    assert!(
        state.filter_panel_rows().iter().all(|(_, query)| query.is_empty()),
        "the new set carries nothing over"
    );
}

#[test]
fn the_last_set_cannot_be_removed() {
    let mut state = /* loaded */;
    state.toggle_filter_panel();
    state.filter_remove_set();
    assert_eq!(state.filter_set_position(), (0, 1), "one set is the panel");

    state.filter_add_set();
    state.filter_remove_set();
    assert_eq!(state.filter_set_position(), (0, 1));
}

#[test]
fn removing_the_last_tab_moves_the_cursor_back_onto_a_tab_that_exists() {
    let mut state = /* loaded */;
    state.toggle_filter_panel();
    state.filter_add_set();
    state.filter_add_set();
    assert_eq!(state.filter_set_position(), (2, 3));

    state.filter_remove_set();

    assert_eq!(state.filter_set_position(), (1, 2));
}

#[test]
fn switching_sets_wraps_and_closes_any_open_picker() {
    let mut state = /* loaded */;
    state.toggle_filter_panel();
    state.filter_add_set();
    select_field(&mut state, "due");
    assert!(state.filter_calendar_begin());

    state.filter_select_set(1);

    assert_eq!(state.filter_set_position().0, 0, "wrapped from the last to the first");
    assert!(!state.filter_calendar_open(), "the picker belonged to the set we left");
    assert!(!state.filter_panel_editing());
}
```

### 4.5 Require-empty — `src/app/task.rs`, `mod tests`

One test per kind, because each reads a different part of `TaskRecord`:

```rust
#[test]
fn requiring_an_empty_field_matches_only_the_records_with_nothing_in_it() {
    let mut state = loaded_state_with_tasks(vec![
        task_with("dated", Some("2026-09-01"), Some("Alex"), Some("High")),
        task_with("undated", None, None, None),
        task_with("blank-assignee", Some("2026-09-02"), Some("   "), None),
    ]);
    state.toggle_filter_panel();

    select_field(&mut state, "due");
    state.filter_toggle_require_empty();
    assert_eq!(visible_gids(&state), vec!["undated"]);

    state.filter_toggle_require_empty();
    select_field(&mut state, "assignee");
    state.filter_toggle_require_empty();
    assert_eq!(
        visible_gids(&state),
        vec!["blank-assignee", "undated"],
        "whitespace is as empty as missing"
    );

    state.filter_toggle_require_empty();
    select_field(&mut state, "Priority");
    state.filter_toggle_require_empty();
    assert_eq!(visible_gids(&state), vec!["blank-assignee", "undated"]);
}
```

Row order in those assertions follows the default date-ascending sort with
undated last — take the order from a `visible_gids` call before asserting rather
than guessing it.

```rust
#[test]
fn a_require_empty_and_a_query_replace_one_another() {
    let mut state = /* loaded */;
    state.toggle_filter_panel();
    select_field(&mut state, "assignee");

    state.filter_toggle_require_empty();
    assert_eq!(state.filter_panel_rows()[1].1, "(none)");

    state.filter_push_char('a');
    assert_eq!(state.filter_panel_rows()[1].1, "a", "typing replaces it");

    state.filter_toggle_require_empty();
    assert_eq!(state.filter_panel_rows()[1].1, "(none)", "and it replaces the text");

    state.filter_clear_current();
    assert_eq!(state.filter_panel_rows()[1].1, "");
}

#[test]
fn a_require_empty_counts_as_an_active_filter() {
    // It excludes rows, so it has to reach the panel's chip and the status bar
    // the same way a typed value does.
    let mut state = /* loaded */;
    state.toggle_filter_panel();
    assert_eq!(state.active_filter_count(), 0);
    select_field(&mut state, "due");
    state.filter_toggle_require_empty();
    assert_eq!(state.active_filter_count(), 1);
}

#[test]
fn the_state_field_cannot_require_an_empty_value() {
    // A task is always open or done, so this would look like a filter while
    // matching nothing.
    let mut state = /* loaded with two tasks */;
    state.toggle_filter_panel();
    select_field(&mut state, "state");

    state.filter_toggle_require_empty();

    assert_eq!(state.filter_panel_rows()[4].1, "");
    assert_eq!(state.active_filter_count(), 0);
    assert_eq!(visible_gids(&state).len(), 2, "nothing was filtered out");
}

#[test]
fn opening_the_calendar_on_a_require_empty_date_clears_it() {
    let mut state = /* loaded */;
    state.toggle_filter_panel();
    select_field(&mut state, "due");
    state.filter_toggle_require_empty();

    assert!(state.filter_calendar_begin());

    assert_eq!(state.filter_panel_rows()[2].1, "", "a date is about to be picked");
}
```

And the combination that motivated the feature, which belongs in the same file
as §4.1:

```rust
#[test]
fn one_set_can_ask_for_soon_while_another_asks_for_undated() {
    // The review this milestone exists for: imminent work, plus work nobody
    // has scheduled.
    let mut state = loaded_state_with_tasks(vec![
        task_due("soon", "2026-08-26"),
        task_due("later", "2026-11-01"),
        task_undated("someday"),
    ]);
    state.toggle_filter_panel();
    set_field(&mut state, "due", "2026-08-24..2026-08-31");
    state.filter_add_set();
    select_field(&mut state, "due");
    state.filter_toggle_require_empty();

    assert_eq!(visible_gids(&state), vec!["soon", "someday"]);
}
```

### 4.6 The panel renderer — `src/ui/filter_panel.rs`, `mod tests`

`panel_state()` (line 324) already builds a loaded `TaskState` with the panel
open; every test here extends it.

```rust
#[test]
fn assignee_matches_fuzzily_by_default() {
    let view = render_filter_panel(&panel_state()).expect("panel is open");
    assert_eq!(view.rows[1].kind, "fuzzy");
}
```

and update `match_modes_render_as_short_chips` (line 396), whose
`assert_eq!(view.rows[1].kind, "contains")` is now wrong.

```rust
#[test]
fn a_single_set_draws_no_tab_strip() {
    // The common case has to look exactly as it did before.
    let view = render_filter_panel(&panel_state()).expect("panel is open");
    assert!(view.tabs.is_empty());
    assert!(view.counts.iter().all(|chip| !chip.text.starts_with("set ")));
}

#[test]
fn a_second_set_earns_a_tab_strip_and_a_chip() {
    let mut state = panel_state();
    state.filter_push_char('a');
    state.filter_add_set();
    let view = render_filter_panel(&state).expect("panel is open");

    assert_eq!(
        view.tabs.iter().map(|tab| (tab.label.clone(), tab.active, tab.filters)).collect::<Vec<_>>(),
        vec![("1".to_string(), false, 1), ("2".to_string(), true, 0)]
    );
    assert!(view.counts.iter().any(|chip| chip.text == "set 2/2"));
}

#[test]
fn the_tab_strip_is_one_line_at_exactly_the_requested_width() {
    // Same invariant the rows have: the pane's scroll offset is a field index,
    // so nothing here may wrap.
    let theme = Theme::default();
    let mut state = panel_state();
    for _ in 0..8 {
        state.filter_add_set();
    }
    let view = render_filter_panel(&state).expect("panel is open");

    for width in [MARKER_WIDTH + 8, 40, 80, 160] {
        let line = tab_strip_line(&view, &theme, width);
        assert_eq!(visible_width(&line.to_string()), width);
    }
}

#[test]
fn the_active_tab_is_told_apart_without_colour() {
    // The mono theme collapses the styles, so the active tab has to carry a
    // modifier — the same rule the calendar's day styles follow.
    let theme = Theme::default();
    let mut state = panel_state();
    state.filter_add_set();
    let view = render_filter_panel(&state).expect("panel is open");
    let line = tab_strip_line(&view, &theme, 60);

    assert!(line
        .spans
        .iter()
        .any(|span| span.style.add_modifier.contains(Modifier::REVERSED)));
}

#[test]
fn a_require_empty_row_shows_a_value_rather_than_the_unset_placeholder() {
    // `—` means "no filter"; `(none)` means "filter to no value". They are
    // opposites and must not look alike.
    let theme = Theme::default();
    let mut state = panel_state();
    state.move_filter_down();
    state.filter_toggle_require_empty();
    let view = render_filter_panel(&state).expect("panel is open");
    let lines = filter_panel_lines(&view, &theme, 60);

    assert_eq!(view.rows[1].value, FilterValue::RequireEmpty);
    assert!(view.rows[1].value.is_active());
    assert!(lines[1].to_string().contains("(none)"));
    assert!(!lines[1].to_string().contains(theme.glyphs.empty));
}
```

### 4.7 The calendar's week start — `src/app/calendar.rs` and `src/ui/calendar.rs`

In `src/app/calendar.rs`'s tests, against the fixed `today` of 2026-08-24 the
existing tests already use:

```rust
#[test]
fn the_grid_lays_the_month_out_from_sunday() {
    // 1 Aug 2026 is a Saturday, so it sits alone in the last column of the
    // first row; 2 Aug, a Sunday, starts the second.
    let weeks = state("2026-08-01").weeks();

    assert_eq!(weeks[0], [None, None, None, None, None, None, Some(1)]);
    assert_eq!(weeks[1], [Some(2), Some(3), Some(4), Some(5), Some(6), Some(7), Some(8)]);
    assert_eq!(weeks.len(), 6, "August 2026 needs six rows from Sunday");

    // 1 Feb 2026 is itself a Sunday, so February needs no leading padding at
    // all — the case a Monday-first grid padded with six blanks.
    let february = state("2026-02-01").weeks();
    assert_eq!(february[0][0], Some(1));
}
```

In `src/ui/calendar.rs`'s tests:

```rust
#[test]
fn the_weekday_header_starts_on_sunday() {
    let theme = Theme::default();
    let view = calendar_view(Some(&state(""))).expect("the picker is open");
    let lines = calendar_lines(&view, &theme);

    assert_eq!(lines[1].to_string(), "Su Mo Tu We Th Fr Sa ");
}
```

`every_grid_line_is_exactly_the_grid_width` (line 369) and
`the_grid_holds_every_day_of_the_month_once` (line 446) should both keep passing
untouched; if either fails, the layout — not the ordering — has changed.

### 4.8 The Gantt chart is untouched — `src/domain/gantt.rs`, `mod tests`

`weekend_columns_never_overlap_a_weekday` (line 1012) and
`a_week_with_room_to_spare_is_marked_by_weekday` (line 947) are already the right
guard; run them and do not edit them. Add one explicit statement of the
distinction so the next person does not "fix" the inconsistency:

```rust
#[test]
fn the_charts_week_still_starts_on_monday_even_though_the_picker_starts_on_sunday() {
    // Two different questions: a work week starts on Monday, a calendar grid
    // is read from Sunday. weekday_index answers the first; calendar_column
    // answers the second.
    let monday = CivilDate::new(2026, 8, 24).expect("a real date");
    let sunday = CivilDate::new(2026, 8, 23).expect("a real date");

    assert_eq!(monday.weekday_index(), 0);
    assert_eq!(monday.calendar_column(), 1);
    assert_eq!(sunday.weekday_index(), 6, "still a weekend by the chart's reckoning");
    assert_eq!(sunday.calendar_column(), 0, "and the first column of the grid");
}
```

Put it in `src/domain/date.rs`'s tests, beside
`names_the_weekday_from_monday` (line 381), which is where someone reading
`weekday_index` will look.

### 4.9 Commands and bindings — `src/config/mod.rs` and `src/input/mod.rs`

Extend `default_bindings_include_filter_controls` (`src/config/mod.rs:952`):

```rust
        for (key, action) in [
            (KeyBinding::Char('l'), Action::FilterSetNext),
            (KeyBinding::Char('h'), Action::FilterSetPrev),
            (KeyBinding::Char('a'), Action::FilterSetAdd),
            (KeyBinding::Char('x'), Action::FilterSetRemove),
            (KeyBinding::Char('e'), Action::FilterRequireEmpty),
        ] {
            assert_eq!(keymap.action_for(&key, Mode::Filter), Some(&action));
        }
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('e'), Mode::FilterEdit),
            Some(&Action::FilterRequireEmpty),
        );
        // h/l keep their existing meanings in the modes that shadow them: you
        // switch sets from browse mode, not mid-edit.
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('h'), Mode::FilterEdit),
            Some(&Action::FilterMoveLabelLeft),
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('h'), Mode::Calendar),
            Some(&Action::CalendarPrevDay),
        );
```

In `src/input/mod.rs`, add the five names to
`parses_key_aliases_and_commands` (line 597), and one round-trip check — worth
having because the three-place edit in §2.10 is exactly the kind of thing that
gets done in two:

```rust
#[test]
fn every_filter_set_command_round_trips_through_its_name() {
    for action in [
        Action::FilterSetNext,
        Action::FilterSetPrev,
        Action::FilterSetAdd,
        Action::FilterSetRemove,
        Action::FilterRequireEmpty,
    ] {
        assert_eq!(
            Action::from_command(&action.to_string()).expect("parses"),
            action
        );
    }
}
```

For the hint bar and help overlay, follow the existing pattern of resolving keys
through a rebound keymap rather than asserting default letters — the hint modules
never hardcode key names, and the test must not either:

```rust
#[test]
fn the_filter_hints_show_the_rebound_set_keys() {
    let keymap = KeyMap::from_bindings(&[Bind::with_mode("n", Mode::Filter, "filter_set_add")])
        .expect("keymap parses");
    let line = hints::hint_line(
        &hints::hints_for(Mode::Filter, HintContext::default()),
        &keymap, Mode::Filter, &Theme::default(), 200,
    );

    assert!(line.to_string().contains("n"));
    assert!(line.to_string().contains("add set"));
}

#[test]
fn the_set_navigation_hints_appear_only_once_there_are_sets_to_navigate() {
    let one = hints::hints_for(Mode::Filter, HintContext::default());
    let many = hints::hints_for(
        Mode::Filter,
        HintContext { many_filter_sets: true, ..HintContext::default() },
    );

    assert!(!one.iter().any(|hint| hint.label == "remove set"));
    assert!(many.iter().any(|hint| hint.label == "remove set"));
}
```

### 4.10 End to end — `tests/task_view_integration.rs`

Model it on `picking_a_date_on_the_calendar_filters_the_task_table` (line 293):
build a `FakeAsanaClient`, drive `run_session` with a `ScriptedSource`, call
`wait_for_task_data` between batches, and assert on `app.tasks`.

```rust
#[test]
fn two_filter_sets_show_the_union_of_their_results() {
    // Three tasks: one due this week, one due in four months, one in between.
    // The middle one is what proves the sets are ORed rather than merged into
    // one widened range.
    std::env::set_var("TUISANA_TODAY", "2026-08-24");
    // ... client with tasks due 2026-08-26, 2026-10-05, 2026-12-20

    // space t   select the project and open the task view
    // f         open the filter panel
    // j j enter open the Due row's calendar
    // type      2026-08-24..2026-08-31, then enter to commit
    // a         add a second set
    // j j enter its Due row
    // type      2026-12-01..2026-12-31, enter
    let keys = /* the above as KeyEvents, in batches with waits between */;

    assert_eq!(visible_task_names(&app), vec!["Imminent", "Far out"]);
    assert_eq!(app.tasks.filter_set_position(), (1, 2));
}
```

Two more in the same file:

```rust
#[test]
fn a_set_requiring_no_due_date_still_loads_the_undated_tasks() {
    // The server-side date filter, seen from outside: with the union computed
    // wrongly, the `due_on.after` sent to Asana drops the undated tasks and the
    // cache records the window as covered, so they never arrive.
    // ... keys: space t f, j j enter, type today..2026-08-31, enter, a, j j, e
    assert!(visible_task_names(&app).contains(&"Someday".to_string()));
}

#[test]
fn removing_a_set_puts_its_rows_back_behind_the_remaining_filter() {
    // ... two sets as above, then x on the second
    assert_eq!(app.tasks.filter_set_position(), (0, 1));
    assert_eq!(visible_task_names(&app), vec!["Imminent"]);
}
```

`FakeAsanaClient` already honours `due_after`/`due_before`
(`src/asana/fake.rs:116` and `:129`), so these tests really do exercise the
filter that is sent to the server, not just the one applied to the rows that come
back. That is what makes the second one meaningful: get the union in §2.5 wrong
and the fake drops the undated task exactly as the real API would.

### 4.11 Snapshots — `tests/ui_snapshot.rs`

Two new cases, both at all three widths:

```rust
/// Two sets, the second active and both filtering, so the tab strip, the set
/// chip, and the `or` between the tabs are all on screen.
#[test]
fn filter_mode_with_two_sets() {
    assert_snapshot("filter-sets", || {
        vec![
            enter_task_mode(),
            vec![key('f'), key('j'), key('h')],          // Assignee, then type
            "alex".chars().map(key).collect(),
            vec![key('a'), key('j'), key('j')],          // new set, Due row
            vec![key('e')],                              // require empty
        ]
    });
}

/// A require-empty on `Due`, so `(none)` is drawn next to the `—` of the
/// untouched rows and the two are visibly different.
#[test]
fn filter_mode_requiring_an_empty_due_date() {
    assert_snapshot("filter-require-empty", || {
        vec![enter_task_mode(), vec![key('f'), key('j'), key('j'), key('e')]]
    });
}
```

Note that `key('h')` after `key('j')` in the first case would switch sets, not
type — in filter *browse* mode `h` is bound. Reach the edit with `enter` first,
or set the value from browse mode via the keys that do not type. Write the batch,
run it, and read the snapshot before committing it; that is what the snapshots
are for.

Snapshots that will change and must be reviewed rather than blind-accepted:

| Snapshot | Why |
| --- | --- |
| `filter-mode-{80,120,200}` | Assignee's chip: `contains` → `fuzzy` |
| `filter-edit-{80,120,200}` | same |
| `filter-date-calendar*` (all 18) | Assignee's chip, plus the weekday header and day placement, plus one extra grid row in August 2026 |

Update with `UPDATE_SNAPSHOTS=1 cargo test --test ui_snapshot` and read the diff:
the calendar diffs should show `Su Mo Tu We Th Fr Sa` and the day numbers shifted
one column right, and nothing else should move.

Finally, `the_monochrome_theme_emits_no_color` (line 660) walks every cell
asserting `cell.symbol().is_ascii()`. `(none)` and the tab labels are ASCII;
`theme.glyphs.active` resolves per theme, so the tab's active marker is safe.
Extend that test's key script to add a second set, so the strip is inside the
pass rather than beside it.

### 4.12 The deletion — `src/domain/task.rs`, `mod tests`

Two existing tests write the fields being deleted and will not compile. Neither
should be simply deleted: one of them is the only guard on row ordering.

**`filters_by_text_field_owner_completion_and_date` (line 1402)** — delete it.
It is the sole writer of `text`, `field`, `assignee`, and `date_range`, and what
it asserts is now the filter panel's job, where `src/app/task.rs`'s tests cover
it against the code that actually runs (§4.1 and §4.5 add more). Replace it with
the part that is still real:

```rust
#[test]
fn filters_by_completion_state() {
    let mut open = TaskRecord::new("1", "Ship release");
    let mut done = TaskRecord::new("2", "Write docs");
    done.completed = true;

    let gids = |completed| {
        let settings = TaskTableSettings {
            filter: TaskFilter { completed, ..TaskFilter::default() },
            sort: TaskSort::default(),
        };
        TaskTableModel::from_records_with_settings(
            vec![open.clone(), done.clone()], Vec::new(), &settings,
        )
        .rows
        .iter()
        .filter(|row| row.kind.is_task())
        .map(|row| row.gid.clone())
        .collect::<Vec<_>>()
    };

    assert_eq!(gids(Some(false)), vec!["1"]);
    assert_eq!(gids(Some(true)), vec!["2"]);
    assert_eq!(gids(None).len(), 2);
}
```

**`keeps_parent_and_subtasks_together_when_visible` (line 1451)** — rewrite it.
Its `Show` half filters on `text: Some("match")` to prove a parent is kept when
its child matches; that behaviour goes with the field. Its `Hide` half is still
valid. What must *not* be lost is the row ordering it incidentally asserted
(`["parent", "child"]`), which is the only test in the repo that fails if the
grouping in `apply_task_filter` is simplified away. Make that the point of the
test, and make it strong enough to actually fail — one parent and one child pass
even without grouping, so use two of each:

```rust
#[test]
fn subtasks_sit_under_their_own_parent_rather_than_in_a_heap_at_the_bottom() {
    // TaskSort::compare puts every parent before every subtask, so the
    // grouping in apply_task_filter is the only thing interleaving them. With
    // one parent and one child this passes either way — hence two.
    let records = vec![
        parent("p1", "Alpha parent"),
        parent("p2", "Beta parent"),
        child("c1", "Alpha child", "p1"),
        child("c2", "Beta child", "p2"),
    ];

    let model = TaskTableModel::from_records_with_settings(
        records.clone(),
        Vec::new(),
        &TaskTableSettings {
            filter: TaskFilter { subtasks: SubtaskVisibility::Show, ..TaskFilter::default() },
            sort: TaskSort::default(),
        },
    );

    assert_eq!(task_gids(&model), vec!["p1", "c1", "p2", "c2"]);

    let hidden = TaskTableModel::from_records_with_settings(
        records,
        Vec::new(),
        &TaskTableSettings {
            filter: TaskFilter { subtasks: SubtaskVisibility::Hide, ..TaskFilter::default() },
            sort: TaskSort::default(),
        },
    );

    assert_eq!(task_gids(&hidden), vec!["p1", "p2"]);
}
```

`parent`, `child`, and `task_gids` are three-line helpers for this module; give
the two parents distinct projects *or* the same one deliberately, and say which
in a comment, because project grouping inserts header rows between them and
`task_gids` filters those out either way.

**Then check nothing else notices.** The rest of the suite is the real test of
the deletion:

```sh
mise test
```

Pay attention to three groups in particular, none of which should need an edit:

- `src/domain/gantt.rs` — its fixture at line 1273 builds a `TaskFilter` with
  `completed: None, ..default()`. If it fails to compile, a field was removed
  that something still sets.
- `src/app/task.rs`'s subtask tests —
  `hiding_subtasks_keeps_selection_near_the_previous_row_instead_of_jumping_to_the_top`
  (line 3149), `collapsing_subtasks_selects_the_super_task_when_the_current_row_is_a_subtask`
  (line 3202), and `an_assigned_subtask_groups_under_its_parents_project_when_the_parent_is_filtered_out`
  (line 3989). These drive real datasets through the whole path and are what
  would catch a reordering the new domain test somehow missed.
- `tests/ui_snapshot.rs` — **no snapshot may change.** A snapshot diff here means
  the deletion changed behaviour, not dead code; find out why before accepting
  it. This is the strongest single signal that §2.11 was a no-op, which is the
  whole claim.

---

## Deferred, deliberately

Recorded here because each one is a thing a reader of this milestone might
reasonably expect, and the reason it is absent is a decision rather than an
oversight.

- **A parent whose subtask matches is not pulled into the results.**
  `domain::TaskFilter` contained an unreachable implementation of this, and
  §2.11 deletes it. Doing it properly means deciding what "the parent of a match
  in set 2" is, and it belongs in `TaskState::apply_filter_panel`, which judges
  each record alone today.
- **Filter sets are not persisted.** They live for the session, like the Gantt
  chart's timeline window. [Milestone 15](15-hardening-and-polish.md)'s "persisted default project sets and
  views" is where saved sets would belong, and naming them is most of that work.
- **Sets cannot be reordered or named.** Tabs are numbered by position. Names
  would need an edit mode of their own, and the numbers are enough to tell two
  sets apart while the panel is open.
- **The tab strip is hidden when there is only one set** (§2.9), so nothing on
  screen advertises the feature; the hint bar's `add set` is what does. One line
  to reverse if that turns out to be too quiet.

---

## Definition of done

- Two sets with disjoint due windows show the union, and one set still ANDs.
- A new set never removes a row; the last set cannot be removed.
- Every set, the active tab, the field cursor, and an in-progress edit survive a
  multi-project streaming load.
- `desired_task_query` returns the union of the sets' due windows, and nothing at
  all when any set is unbounded or requires an empty due date; widening the union
  is a cache miss.
- `e` filters to tasks with no value in the selected field, for text, label, and
  date rows; it is refused on State; it and a typed value replace one another.
- Assignee starts on fuzzy and can still be switched per row.
- The picker's grid and header read `Su Mo Tu We Th Fr Sa`, and the Gantt chart's
  weekends and week ticks are unchanged.
- All five new commands are rebindable, and the hint bar and help overlay show
  the rebound key.
- README, `tuisana.toml.example`, and `developer.md` describe filter sets,
  require-empty, the assignee default, and the week start.
- `TaskFilter` holds `completed` and `subtasks` and nothing else; `TaskFieldFilter`
  and `TaskDateRange` are gone, including from `domain`'s re-exports; and
  `grep -rEn 'filter\.(text|field|assignee|date_range)' src tests` finds nothing.
- Subtasks still appear under their own parents, with a test that fails if the
  grouping in `apply_task_filter` is removed.
- `mise test` is green, and every snapshot diff has been read — and the §2.11
  commit changed no snapshot at all.
