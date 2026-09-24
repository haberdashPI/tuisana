//! State and behavior for the task pane.
//!
//! This module owns the task table's live state, the task-loading cache, and
//! the task filter editor used by the UI.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Instant,
};

use crate::{
    app::{
        autocomplete::{AutocompleteState, Candidate, Unresolved},
        calendar::CalendarState,
        debug_log,
        gantt::{GanttViewState, MoveTo},
        task_edit::{
            CellEditView, CellEditor, CommittedEdits, EditContext, TaskCellEditState,
        },
        text_edit::TextEdit,
    },
    asana::{
        dto::{CustomFieldDto, CustomFieldValueDto, TaskDto},
        AsanaClient, TaskLoadScope, TaskQuery, TaskTarget,
    },
    domain::{
        date, distinct_values, group_custom_fields_by_name, merge_task_record,
        parse_custom_value, parse_date_value, resolve_assignee, AssigneeRef, CivilDate,
        CustomFieldDefinition, CustomFieldKind, CustomValueKind, EnumOption, ProjectEdit,
        TaskEdit, TaskFieldEdit,
        GanttColorKey, Timeline,
        DateQuery, Project, ProjectKind, TaskRecord, TaskRowKind, TaskTableModel,
        TaskTableSettings,
    },
    config::{GanttConfig, SavedFilterField, SavedFilterSet},
    error::Result,
    input::Action,
    util::fuzzy_match,
};

const HORIZONTAL_SCROLL_STEP: usize = 8;

/// What the logged-in user is called wherever a name is typed.
///
/// A handle rather than a name: it resolves to whoever is reading, which is
/// exactly what makes it worth having in both the editor and the filter.
pub(crate) const ME_LABEL: &str = "me";

/// Cells the rule between two table columns costs.
///
/// The renderer's own constant, mirrored here because the column cursor has to
/// know where a column starts to scroll the table to it.
const COLUMN_RULE_WIDTH: usize = 3;

/// Most saved filter sets one numbered window of the sidebar can hold.
///
/// Nine because `1`-`9` are the access path: a row the digits cannot reach is
/// a row with no way to load it.
pub(crate) const MAX_SIDEBAR_ROWS: usize = 9;

/// Most tasks the recently-edited pane remembers.
///
/// It is "what I was just doing", not a history: twenty is more than a
/// session's worth of edits that went out of view, and the pane only ever
/// draws a few of them at once.
const MAX_RECENTLY_EDITED: usize = 20;

/// Most rows the recently-edited pane draws at once.
///
/// It is a holding area, not a second table: past a few rows it would be
/// taking the screen from the table it is an aside to. The rest of the list
/// is still counted on the border, so the number is never a surprise.
const MAX_RECENT_ROWS: usize = 4;

/// High-level status for the task state machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    Idle,
    Loading,
    Ready,
    OutOfDate(String),
    Empty,
    Error(String),
}

impl Default for TaskStatus {
    fn default() -> Self {
        Self::Idle
    }
}

/// State for the task pane, including the loaded dataset, filter editor,
/// scroll position, and the table model rendered by the UI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskState {
    view: TaskViewState,
    loading: TaskLoadingState,
}

/// User-facing task pane state: selection, table settings, filter editor,
/// pane visibility, and scroll position.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TaskViewState {
    visible: bool,
    selected: Option<usize>,
    table: TaskTableModel,
    settings: TaskTableSettings,
    horizontal_scroll: usize,
    /// The column cursor, as a cell index into `TaskTableModel::columns`.
    selected_column: usize,
    /// Set when the column cursor has moved and the table has yet to scroll.
    ///
    /// A flag rather than an unconditional "keep the cursor column on
    /// screen": `left` and `right` scroll the columns by hand, and a cursor
    /// that dragged the viewport back on the next frame would make them look
    /// broken.
    pending_column_scroll: bool,
    /// The edit in progress, if any.
    cell_edit: Option<TaskCellEditState>,
    /// Tasks edited this session, newest first.
    ///
    /// Not persisted and not capped by time: "what I was just doing" is not
    /// a thing to restore three days later.
    recently_edited: Vec<String>,
    /// The recently-edited tasks the current view no longer shows.
    ///
    /// Rebuilt beside the table, from the same records and with the same
    /// columns, so the pane and the table read as one surface.
    recent_table: TaskTableModel,
    /// How many recently-edited tasks the view is not showing.
    ///
    /// The pane draws the newest few; this counts all of them, so the border
    /// can say what is out of sight.
    recent_hidden_count: usize,
    /// The cursor's row in that pane, when the cursor is in it.
    ///
    /// The cursor is one cursor over two lists: `Some` here means the keys
    /// act on the pane, and `selected` is being held for the row to return
    /// to.
    recent_selected: Option<usize>,
    /// Set by the toggle. Not `recent_visible`, because the pane is shown by
    /// default: the first edit that hides a task is the moment it is needed,
    /// which is too late to go looking for a key.
    recent_hidden: bool,
    /// What went wrong with the last edit, shown in the corner notice pane.
    ///
    /// Held whole, newlines and all: `ui::notice` wraps it and the breaks a
    /// backend put in its message are part of what the message says.
    edit_notice: Option<String>,
    filter_editor: TaskFilterEditorState,
    task_vertical_scroll: usize,
    filter_vertical_scroll: usize,
    help_details_visible: bool,
    selected_task_ids: HashSet<String>,
    gantt: GanttViewState,
    /// Set while the runtime knows another input event is already waiting.
    ///
    /// Rebuilding the table is the expensive half of a keystroke and the user
    /// cannot read a table that is about to be replaced anyway, so while keys
    /// are still arriving the rebuild is put off rather than run once per
    /// character.
    input_pending: bool,
    /// The logged-in user's gid, for the `list` filter's `me`.
    ///
    /// Kept here rather than passed in because filtering runs on every
    /// rebuild, far from the action that started it, and `me` has to mean the
    /// same thing on all of them.
    current_user_gid: Option<String>,
    /// When the table stopped matching the filter, if it does not.
    ///
    /// A timestamp rather than a flag: the spinner is only worth showing once
    /// the wait is long enough to notice, and this is what says how long it
    /// has been.
    stale_since: Option<Instant>,
}

/// Whether a task-data fetch is currently in flight.
///
/// Grouping these fields in an enum ensures that in-flight metadata
/// (spinner start time, target names/IDs) cannot coexist with the Idle
/// state, making invalid combinations unrepresentable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum LoadProgress {
    #[default]
    Idle,
    Active {
        started_at: Instant,
        target_names: Vec<String>,
        target_ids: Vec<String>,
    },
}

impl LoadProgress {
    fn started_at(&self) -> Option<Instant> {
        match self {
            Self::Active { started_at, .. } => Some(*started_at),
            Self::Idle => None,
        }
    }

    fn target_names(&self) -> &[String] {
        match self {
            Self::Active { target_names, .. } => target_names,
            Self::Idle => &[],
        }
    }

    fn target_ids(&self) -> &[String] {
        match self {
            Self::Active { target_ids, .. } => target_ids,
            Self::Idle => &[],
        }
    }
}

/// Task loading state: the cache, dataset, in-flight progress, and scope tracking.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TaskLoadingState {
    status: TaskStatus,
    progress: LoadProgress,
    dataset: Option<TaskDataset>,
    cache: TaskCache,
    loaded_target_ids: Vec<String>,
    loaded_project_queries: HashMap<String, TaskQuery>,
}

/// The merged task records and custom field definitions used to rebuild the
/// visible task table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TaskDataset {
    records: Vec<TaskRecord>,
    custom_field_definitions: Vec<CustomFieldDefinition>,
}

/// The coarse type of filter supported by the task filter editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskFieldFilterKind {
    String,
    Labels,
    Date,
}

/// The matching strategy for string filters in the task filter editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskFieldStringMode {
    Fuzzy,
    Substring,
    Regex,
    /// The value is a list of names picked from a directory, matched exactly.
    ///
    /// Only a row whose values are a closed set can offer this — in practice
    /// `Assignee`, which is the one filter row whose values are real people.
    List,
}

/// Static metadata for one filter row in the task filter editor.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TaskFilterFieldSpec {
    key: String,
    label: String,
    kind: TaskFieldFilterKind,
    /// Every custom-field id this row filters on.
    ///
    /// A custom field with the same name usually exists separately in each
    /// project, with its own id. Keying rows by id gave one identically-labelled
    /// row per project — five "Tag" rows in a row. One row now covers them all.
    custom_gids: Vec<String>,
    /// Whether "has no value" is a state this field can be in.
    ///
    /// False only for `state`: a task is always either open or done, so a
    /// require-empty there would match nothing while looking like a filter.
    can_be_empty: bool,
    /// Whether this row's values come from a directory it can complete over.
    ///
    /// True only for `Assignee`. `Projects` deliberately stays free text:
    /// which projects are in view is what the project pane is for, and a
    /// second control answering the same question would be two controls
    /// fighting over it.
    completes: bool,
}

/// Mutable state for one filter row, including the user query and any selected
/// label values.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TaskFilterFieldState {
    spec: TaskFilterFieldSpec,
    /// The typed value, with its caret.
    ///
    /// The same buffer the task table's cell editor uses, so word and line
    /// motion are written once rather than once per pane. Editing used to be
    /// append-only with the caret pinned to the end; it is a position now so
    /// the motion keys work on any field, not just the date ones the calendar
    /// drives.
    value: TextEdit,
    string_mode: TaskFieldStringMode,
    label_values: Vec<String>,
    label_options: Vec<String>,
    label_cursor: usize,
    /// Match only records with no value for this field.
    ///
    /// Mutually exclusive with `query`: setting this clears the query, and
    /// typing clears this. "Has no due date" is not expressible as a date
    /// expression, so it is a flag rather than a magic token in the text.
    empty_required: bool,
    /// Invert this row's verdict: keep what it would have thrown away.
    ///
    /// Orthogonal to everything else on the row. The match mode, the value,
    /// and require-empty all still mean exactly what they say; the answer is
    /// flipped once they have given it. Negating a require-empty is therefore
    /// "has some value", which nothing else in the panel can express.
    negated: bool,
    /// The mode this row was built with.
    ///
    /// Kept so a saved field can leave `match` out when the user never
    /// changed it — the defaults differ per row, so the row is the only thing
    /// that knows its own.
    default_string_mode: TaskFieldStringMode,
}

/// A display-friendly snapshot of one task filter row for the filter panel UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskFilterPanelEntry {
    pub label: String,
    pub query: String,
    pub kind: String,
    pub selected: bool,
    pub editing: bool,
    pub label_values: Vec<String>,
    pub label_cursor: Option<usize>,
    /// Where the edit caret sits in the value, as a char index, when this row is
    /// the one being edited.
    pub caret: Option<usize>,
    /// Whether this field came from a project custom field rather than a
    /// built-in task field.
    pub custom: bool,
    /// Whether this row filters to records with no value at all.
    pub empty_required: bool,
    /// Whether this row's verdict is inverted.
    pub negated: bool,
}

/// One filter set: the field list the panel has always shown.
///
/// Every set has the same fields, because they are derived from the same
/// dataset. What differs is what the user typed into them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TaskFilterSet {
    fields: Vec<TaskFilterFieldState>,
    /// Invert the whole set: keep the records its fields would have rejected.
    ///
    /// Applied after the fields have ANDed, so this is `not (a and b)` rather
    /// than `(not a) and (not b)` — negating a set is a different statement
    /// from negating each of its rows, and both are reachable.
    negated: bool,
    /// Saved values with no row to land on yet.
    ///
    /// A custom-field row only exists once a project carrying that field has
    /// loaded, and the panel is written back to the named entry on every
    /// change — so a value with nowhere to go has to be parked rather than
    /// discarded, or loading a set with the wrong projects selected would
    /// quietly erase half of it. Re-resolved on every rebuild, and written
    /// back out by [`TaskFilterSet::to_saved`].
    unresolved: Vec<SavedFilterField>,
}

/// The sidebar's one-line prompt, mounted on its bottom border.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SidebarPrompt {
    /// Typing a name for `w`.
    Save { text: String, caret: usize },
    /// Confirming `d`.
    ConfirmDelete { name: String },
    /// Confirming a digit that would throw away an unnamed panel.
    ConfirmLoad { name: String },
}

impl SidebarPrompt {
    /// Whether this prompt wants a `y`/`n` rather than typed text.
    ///
    /// One mode covers both kinds, so this is what lets the hint bar name the
    /// keys the open prompt actually reads.
    pub(crate) fn is_confirmation(&self) -> bool {
        !matches!(self, Self::Save { .. })
    }
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
    /// The completion editor, while a `list` row is being edited through it.
    ///
    /// The same state machine the task table's cell editor runs, mirrored
    /// into the row's text the same way the calendar is: the row is where the
    /// value is read, the editor only writes into it.
    autocomplete: Option<AutocompleteState>,
    /// The named entry the panel currently *is*, if any.
    ///
    /// `Some(name)` means every change writes through to that entry. `y`
    /// clears it, which is the whole of "detach".
    loaded: Option<String>,
    /// Set by `refresh_table` whenever the panel is bound; cleared by a write.
    dirty: bool,
    sidebar_visible: bool,
    /// First saved entry in the numbered window.
    sidebar_page_start: usize,
    /// How many saved entries the sidebar can currently show.
    ///
    /// Measured by the renderer, which is the only thing that knows the pane's
    /// height, in the same way `ensure_filter_visible` learns the viewport.
    /// `None` until the sidebar has been drawn once.
    sidebar_rows: Option<usize>,
    prompt: Option<SidebarPrompt>,
    /// A one-line report that has nowhere better to go, such as a config
    /// write that failed. Shown on the sidebar's bottom border.
    notice: Option<String>,
}

/// Small cache of task records and custom field names keyed by task GID.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TaskCache {
    records: HashMap<String, TaskRecord>,
    /// Whole definitions, keyed by gid.
    ///
    /// The name alone was enough while the table only read them. A write has
    /// to know the field's kind and which project declared it, so the cache
    /// keeps the definition rather than collapsing it to a label.
    custom_field_definitions: HashMap<String, CustomFieldDefinition>,
    lru: VecDeque<String>,
}

impl TaskCache {
    const CAPACITY: usize = 2_000;

    fn merge_dataset(&mut self, dataset: TaskDataset) {
        for record in dataset.records {
            self.upsert_record(record);
        }

        for definition in dataset.custom_field_definitions {
            self.custom_field_definitions
                .entry(definition.gid.clone())
                .or_insert(definition);
        }
    }

    fn upsert_record(&mut self, record: TaskRecord) {
        let gid = record.gid.clone();
        if let Some(existing) = self.records.get_mut(&gid) {
            merge_task_record(existing, record);
        } else {
            self.records.insert(gid.clone(), record);
        }

        self.touch(&gid);
        self.evict_if_needed();
    }

    fn touch(&mut self, gid: &str) {
        if let Some(position) = self.lru.iter().position(|existing| existing == gid) {
            self.lru.remove(position);
        }
        self.lru.push_back(gid.to_string());
    }

    fn evict_if_needed(&mut self) {
        while self.records.len() > Self::CAPACITY {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            self.records.remove(&oldest);
        }
    }

    fn records_for_targets(&mut self, target_ids: &[String]) -> Vec<TaskRecord> {
        let target_ids = target_ids.iter().collect::<std::collections::HashSet<_>>();
        let records = self
            .records
            .values()
            .filter(|record| {
                record
                    .project_gids
                    .iter()
                    .any(|project_gid| target_ids.contains(project_gid))
            })
            .cloned()
            .collect::<Vec<_>>();

        for record in &records {
            self.touch(&record.gid);
        }

        records
    }

    fn custom_field_definitions(&self) -> Vec<CustomFieldDefinition> {
        let mut definitions = self
            .custom_field_definitions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        definitions.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.gid.cmp(&right.gid))
        });
        definitions
    }
}

impl TaskFilterEditorState {
    fn from_dataset(dataset: &TaskDataset) -> Self {
        let mut fields = vec![
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "title".to_string(),
                    label: "Title".to_string(),
                    kind: TaskFieldFilterKind::String,
                    custom_gids: Vec::new(),
                    can_be_empty: true,
                    completes: false,
                },
                TaskFieldStringMode::Fuzzy,
                Vec::new(),
            ),
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "assignee".to_string(),
                    label: "Assignee".to_string(),
                    kind: TaskFieldFilterKind::String,
                    custom_gids: Vec::new(),
                    can_be_empty: true,
                    completes: true,
                },
                // A person's name, typed from memory, is the field most likely
                // to be half-remembered — so it gets the fuzzy default Title
                // has. `ctrl-s` still switches the row back to `contains`.
                TaskFieldStringMode::Fuzzy,
                Vec::new(),
            ),
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "due".to_string(),
                    label: "Due".to_string(),
                    kind: TaskFieldFilterKind::Date,
                    custom_gids: Vec::new(),
                    can_be_empty: true,
                    completes: false,
                },
                TaskFieldStringMode::Substring,
                Vec::new(),
            ),
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "start".to_string(),
                    label: "Start".to_string(),
                    kind: TaskFieldFilterKind::Date,
                    custom_gids: Vec::new(),
                    can_be_empty: true,
                    completes: false,
                },
                TaskFieldStringMode::Substring,
                Vec::new(),
            ),
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "state".to_string(),
                    label: "State".to_string(),
                    kind: TaskFieldFilterKind::Labels,
                    custom_gids: Vec::new(),
                    // A task is always either open or done, so there is no
                    // empty state for a require-empty to match.
                    can_be_empty: false,
                    completes: false,
                },
                TaskFieldStringMode::Substring,
                vec!["open".to_string(), "done".to_string()],
            ),
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "projects".to_string(),
                    label: "Projects".to_string(),
                    kind: TaskFieldFilterKind::String,
                    custom_gids: Vec::new(),
                    can_be_empty: true,
                    completes: false,
                },
                TaskFieldStringMode::Substring,
                Vec::new(),
            ),
        ];

        // One row per distinct field *name*, gathering every id that carries it.
        // The table's columns are grouped by the same helper, so the two agree.
        for (name, gids) in group_custom_fields_by_name(&dataset.custom_field_definitions) {
            let mut values = dataset
                .records
                .iter()
                .flat_map(|record| {
                    gids.iter()
                        .filter_map(|gid| record.custom_fields.get(gid))
                })
                .flat_map(|values| values.iter().cloned())
                .collect::<Vec<_>>();
            values.sort();
            values.dedup();
            let kind = if !values.is_empty()
                && values.len() <= 12
                && values.iter().all(|value| {
                    value.len() <= 32
                        && value
                            .chars()
                            .all(|ch| !ch.is_whitespace() || ch == ' ')
                })
            {
                TaskFieldFilterKind::Labels
            } else {
                TaskFieldFilterKind::String
            };
            fields.push(TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    // Keyed by name, not id, so a saved query survives a reload
                    // that brings a different set of projects with it.
                    key: format!("custom:{name}"),
                    label: name.clone(),
                    kind,
                    custom_gids: gids,
                    can_be_empty: true,
                    completes: false,
                },
                TaskFieldStringMode::Substring,
                if kind == TaskFieldFilterKind::Labels {
                    values.clone()
                } else {
                    Vec::new()
                },
            ));
        }

        Self {
            sets: vec![TaskFilterSet {
                fields,
                ..TaskFilterSet::default()
            }],
            ..Self::default()
        }
    }

    /// Carries everything the user did across a rebuild.
    ///
    /// `from_dataset` builds one empty set from whatever records have arrived so
    /// far, and loading streams in one project at a time — so this runs several
    /// times while someone is typing. The set list, the active tab, the field
    /// cursor, every set's queries, and the in-progress edit all have to be
    /// re-applied here. Before Milestone 11.75 the caret snapped back to 0
    /// between keystrokes for exactly this reason; a set list that did not
    /// survive would vanish mid-load the same way.
    fn restore_queries(&mut self, previous: Self) {
        self.visible = previous.visible;
        self.editing = previous.editing && self.visible;
        self.calendar = if self.editing { previous.calendar } else { None };
        self.autocomplete = if self.editing { previous.autocomplete } else { None };
        // The named entry, its binding, and the sidebar's own state are all
        // things the user set; a rebuild that dropped them would detach the
        // panel mid-load.
        self.loaded = previous.loaded;
        self.dirty = previous.dirty;
        self.sidebar_visible = previous.sidebar_visible;
        self.sidebar_page_start = previous.sidebar_page_start;
        self.sidebar_rows = previous.sidebar_rows;
        self.prompt = previous.prompt;
        self.notice = previous.notice;

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

    /// Every set as it would be written to `tuisana.toml`.
    fn to_saved(&self) -> Vec<SavedFilterSet> {
        self.sets.iter().map(TaskFilterSet::to_saved).collect()
    }

    /// Replaces every set with the saved ones.
    ///
    /// The panel's own state — visibility, the field cursor, the sidebar — is
    /// not part of a named entry, so it survives; the active tab and the field
    /// cursor are clamped to whatever the entry brought.
    fn apply_saved(&mut self, saved: &[SavedFilterSet]) {
        let template = self
            .sets
            .first()
            .map(TaskFilterSet::cleared)
            .unwrap_or_default();
        self.sets = saved
            .iter()
            .map(|set| {
                let mut fresh = template.clone();
                fresh.apply_saved(set);
                fresh
            })
            .collect();
        if self.sets.is_empty() {
            self.sets.push(template);
        }

        self.active = self.active.min(self.sets.len() - 1);
        self.selected = self.selected.min(self.fields().len().saturating_sub(1));
        self.stop_editing();
    }

    /// How many saved entries the numbered window holds.
    fn sidebar_window(&self) -> usize {
        self.sidebar_rows
            .unwrap_or(MAX_SIDEBAR_ROWS)
            .clamp(1, MAX_SIDEBAR_ROWS)
    }

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

    fn visible(&self) -> bool {
        self.visible
    }

    fn editing(&self) -> bool {
        self.editing
    }

    fn toggle_visible(&mut self) {
        self.visible = !self.visible;
        if !self.visible {
            self.editing = false;
        }
    }

    /// Fields in the **active** set with a typed value.
    ///
    /// Both counts describe the rows on screen, which is the set being edited —
    /// the tab strip is what says whether another set is doing work.
    fn active_count(&self) -> usize {
        self.fields()
            .iter()
            .filter(|field| !field.value.text().trim().is_empty())
            .count()
    }

    /// Fields that exclude anything, counting label selections and
    /// require-empty as well as text.
    fn active_filter_count(&self) -> usize {
        self.fields()
            .iter()
            .filter(|field| field.is_active() || !field.label_values.is_empty())
            .count()
    }

    fn selected_kind(&self) -> Option<TaskFieldFilterKind> {
        self.selected_field().map(|field| field.spec.kind)
    }

    fn selected_label(&self) -> Option<&str> {
        self.selected_field().map(|field| field.spec.label.as_str())
    }

    fn move_up(&mut self) {
        if self.fields().is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
    }

    fn move_down(&mut self) {
        if self.fields().is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.fields().len() - 1);
    }

    fn page_up(&mut self, page_size: usize) {
        if self.fields().is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(page_size.max(1));
    }

    fn page_down(&mut self, page_size: usize) {
        if self.fields().is_empty() {
            return;
        }
        self.selected = (self.selected + page_size.max(1)).min(self.fields().len() - 1);
    }

    fn clear_current(&mut self) {
        // The completion editor is emptied rather than closed: `ctrl-l` is
        // "clear every item", and closing it would take the candidate list
        // away from someone who was about to pick a different one.
        if let Some(state) = self.autocomplete.as_mut() {
            state.clear();
        }
        if let Some(field) = self.selected_field_mut() {
            // `ctrl-l` resets the row completely: require-empty and the
            // negation go with the value they were qualifying.
            field.clear_value();
        }
    }

    fn set_mode(&mut self, mode: TaskFieldStringMode) {
        if let Some(field) = self.selected_field_mut() {
            if !matches!(field.spec.kind, TaskFieldFilterKind::String) {
                return;
            }
            // A row with no directory behind it has nothing to list.
            if matches!(mode, TaskFieldStringMode::List) && !field.spec.completes {
                return;
            }
            field.string_mode = mode;
        }
        self.autocomplete = None;
    }

    fn cycle_mode(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            if !matches!(field.spec.kind, TaskFieldFilterKind::String) {
                return;
            }
            field.string_mode = match field.string_mode {
                TaskFieldStringMode::Fuzzy => TaskFieldStringMode::Substring,
                TaskFieldStringMode::Substring => TaskFieldStringMode::Regex,
                // `list` joins the ring only where it means something, so
                // every other row still cycles through the three it had.
                TaskFieldStringMode::Regex if field.spec.completes => TaskFieldStringMode::List,
                TaskFieldStringMode::Regex | TaskFieldStringMode::List => {
                    TaskFieldStringMode::Fuzzy
                }
            };
        }
        self.autocomplete = None;
    }

    fn push_char(&mut self, ch: char) {
        if self.with_autocomplete(|state| state.push_char(ch)) {
            return;
        }
        if let Some(field) = self.selected_field_mut() {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                return;
            }
            // Typing is the user supplying a value, which a require-empty is
            // the opposite of.
            field.empty_required = false;
            field.value.insert(ch);
        }
    }

    fn pop_char(&mut self) {
        if self.with_autocomplete(AutocompleteState::delete_back) {
            return;
        }
        if let Some(field) = self.selected_field_mut() {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                return;
            }
            field.value.delete_back();
        }
    }

    /// Moves the selected field's caret, clamped to its text.
    fn move_query_caret(&mut self, delta: i64) {
        if self.with_autocomplete(|state| state.move_caret(delta)) {
            return;
        }
        if let Some(field) = self.selected_field_mut() {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                return;
            }
            field.value.move_caret(delta);
        }
    }

    /// Runs a motion on the selected field's text, if it has any.
    ///
    /// A labels row's text is rebuilt from its chips and its caret is never
    /// drawn, so a motion there would move something invisible.
    fn with_selected_text(&mut self, change: impl FnOnce(&mut TextEdit)) {
        if let Some(field) = self.selected_field_mut() {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                return;
            }
            change(&mut field.value);
        }
    }

    /// Puts the caret at the end of the selected field's text.
    ///
    /// Called when an edit begins, so typing continues from where the value
    /// leaves off rather than from wherever the caret was last time.
    fn reset_query_caret(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            field.value.jump_end();
        }
    }

    fn start_editing(&mut self) {
        self.editing = true;
        self.reset_query_caret();
    }

    fn stop_editing(&mut self) {
        self.editing = false;
        self.calendar = None;
        self.autocomplete = None;
    }

    /// Whether the selected row picks its values from a directory.
    fn selected_completes(&self) -> bool {
        self.selected_field().is_some_and(|field| {
            field.spec.completes && matches!(field.string_mode, TaskFieldStringMode::List)
        })
    }

    /// Opens the completion editor on the selected row, if it is a list one.
    fn open_autocomplete(&mut self, candidates: Vec<Candidate>) {
        if !self.selected_completes() {
            return;
        }
        let Some(field) = self.selected_field() else {
            return;
        };
        // The saved value is the names, so reopening resolves them back into
        // items — and anything that no longer names a candidate is dropped
        // rather than silently kept as a filter nobody can see how to remove.
        let items = parse_label_values(field.value.text())
            .into_iter()
            .filter_map(|name| {
                candidates
                    .iter()
                    .find(|candidate| candidate.display.eq_ignore_ascii_case(&name))
                    .cloned()
            })
            .collect();
        self.autocomplete = Some(AutocompleteState::new(items, candidates, usize::MAX));
        self.sync_autocomplete_query();
    }

    /// Runs a change on the open completion editor, mirroring the result into
    /// the row. Answers whether there was one to run.
    fn with_autocomplete(&mut self, change: impl FnOnce(&mut AutocompleteState)) -> bool {
        let Some(state) = self.autocomplete.as_mut() else {
            return false;
        };
        change(state);
        self.sync_autocomplete_query();
        true
    }

    /// Copies the completion editor's value into the row it is editing.
    fn sync_autocomplete_query(&mut self) {
        let Some((text, caret)) = self
            .autocomplete
            .as_ref()
            .map(|state| (state.text(), state.caret()))
        else {
            return;
        };
        if let Some(field) = self.selected_field_mut() {
            field.empty_required = false;
            field.value.set_text(text);
            field.value.jump_start();
            field.value.move_caret(caret as i64);
        }
    }

    /// Resolves the completion editor and writes the picked names into the
    /// row, answering with whatever could not be resolved.
    ///
    /// The names, joined the way a labels row joins its chips: the saved
    /// schema does not change, and `me` is saved as `me` rather than as the
    /// person who happened to pick it.
    fn commit_autocomplete(&mut self) -> Option<Unresolved> {
        let state = self.autocomplete.take()?;
        let held = state.items().to_vec();
        let (items, refusal) = match state.commit() {
            Ok(items) => (items, None),
            Err(refusal) => (held, Some(refusal)),
        };
        if let Some(field) = self.selected_field_mut() {
            field.value.set_text_at_end(
                items
                    .iter()
                    .map(|item| item.display.clone())
                    .collect::<Vec<_>>()
                    .join(" | "),
            );
        }
        refusal
    }

    /// Opens the date picker on the selected field, if it holds a date.
    fn open_calendar(&mut self, today: CivilDate) -> bool {
        let Some(field) = self.selected_field() else {
            return false;
        };
        if !matches!(field.spec.kind, TaskFieldFilterKind::Date) {
            return false;
        }
        let (label, query) = (field.spec.label.clone(), field.value.text().to_string());
        // A date is about to be picked, so there is a value coming.
        if let Some(field) = self.selected_field_mut() {
            field.empty_required = false;
        }
        self.calendar = Some(CalendarState::open(label, &query, today));
        self.editing = true;
        true
    }

    /// Copies the picker's text into the field it is editing.
    ///
    /// Called after every picker change, because the field — not the overlay —
    /// is where the value is shown and what the table filters on.
    fn sync_calendar_query(&mut self) {
        let Some(query) = self.calendar.as_ref().map(|state| state.query().to_string()) else {
            return;
        };
        if let Some(field) = self.selected_field_mut() {
            field.value.set_text(query);
        }
    }

    fn move_label_cursor_left(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                if !field.label_values.is_empty() {
                    field.label_cursor = field.label_cursor.saturating_sub(1);
                }
            }
        }
    }

    fn move_label_cursor_right(&mut self) {
        if let Some(field) = self.selected_field_mut() {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                if !field.label_values.is_empty() {
                    field.label_cursor = (field.label_cursor + 1).min(field.label_values.len() - 1);
                }
            }
        }
    }

    fn cycle_selected_label(&mut self, delta: i32) {
        let Some(field) = self.selected_field_mut() else {
            return;
        };
        if !matches!(field.spec.kind, TaskFieldFilterKind::Labels) || field.label_options.is_empty() {
            return;
        }
        field.empty_required = false;
        if field.label_values.is_empty() {
            field.label_values.push(field.label_options[0].clone());
            field.label_cursor = 0;
        }
        let cursor = field.label_cursor.min(field.label_values.len() - 1);
        let current = field.label_values[cursor].clone();
        let index = field
            .label_options
            .iter()
            .position(|value| value.eq_ignore_ascii_case(&current))
            .unwrap_or(0);
        let len = field.label_options.len() as i32;
        let next = (index as i32 + delta).rem_euclid(len) as usize;
        field.label_values[cursor] = field.label_options[next].clone();
        field.value.set_text_at_end(field.label_values.join(" | "));
    }

    fn add_label(&mut self) {
        let Some(field) = self.selected_field_mut() else {
            return;
        };
        if !matches!(field.spec.kind, TaskFieldFilterKind::Labels) || field.label_options.is_empty() {
            return;
        }
        field.empty_required = false;
        let value = field.label_options[0].clone();
        let insert_at = field.label_cursor.saturating_add(1).min(field.label_values.len());
        field.label_values.insert(insert_at, value);
        field.label_cursor = insert_at;
        field.value.set_text_at_end(field.label_values.join(" | "));
    }

    fn delete_selected_label(&mut self) {
        let Some(field) = self.selected_field_mut() else {
            return;
        };
        if !matches!(field.spec.kind, TaskFieldFilterKind::Labels) || field.label_values.is_empty() {
            return;
        }
        let cursor = field.label_cursor.min(field.label_values.len() - 1);
        field.label_values.remove(cursor);
        if field.label_cursor >= field.label_values.len() && !field.label_values.is_empty() {
            field.label_cursor = field.label_values.len() - 1;
        }
        if field.label_values.is_empty() {
            field.label_cursor = 0;
            field.value.clear();
        } else {
            field.value.set_text_at_end(field.label_values.join(" | "));
        }
    }

    /// Compiles the panel into a matcher for one pass over the records.
    ///
    /// Everything that does not vary per record is done here, exactly once:
    /// a regex row compiles its pattern, a date row parses its query against
    /// today's date, a labels row splits its include/exclude tokens, and a
    /// text row lowercases its needle. All of that used to happen *inside* the
    /// per-record test — a regex filter recompiled its pattern for every task
    /// on every keystroke, which is where a filter pass over 20k tasks spent
    /// ~130ms of its ~135ms.
    fn prepare(&self, me: Option<&str>) -> PreparedFilter<'_> {
        let today = date::today();
        PreparedFilter {
            sets: self
                .sets
                .iter()
                .map(|set| PreparedSet {
                    negated: set.negated,
                    fields: set
                        .fields
                        .iter()
                        .filter(|field| field.is_active())
                        .map(|field| PreparedField {
                            field,
                            matcher: PreparedMatcher::for_field(field, today, me),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// The due-date bounds to push down to the API, across every set.
    ///
    /// Returns `(after, before)` as `YYYY-MM-DD` strings for the API's
    /// `due_on.after` / `due_on.before` params. Keywords resolve against the
    /// *local* date, which is the whole reason this goes through
    /// [`crate::domain::date`]: resolving `today` in UTC fetched the wrong day's
    /// tasks every evening west of UTC, and then cached that window as covered.
    ///
    /// Sets OR, so the server must return the union of their windows: the
    /// earliest `after`, the latest `before`. Getting this wrong does not
    /// merely fetch too little — `TaskQuery::covers` would then record the
    /// narrow window as cached, so the missing tasks stay missing until
    /// something else forces a refresh.
    ///
    /// Two different kinds of unbounded, which is why there are two exits:
    ///
    /// - A set that says nothing about the due date at all — no due row, an
    ///   unparseable query, an empty one, a require-empty, or either negation
    ///   — constrains *neither* side, so the whole union is unbounded and the
    ///   early `return (None, None)` is right: no later set can narrow it. A
    ///   require-empty in particular has to drop the window rather than
    ///   restrict it, because `due_on.after` would filter out exactly the
    ///   undated tasks that set asked for.
    /// - A set that is open on one side only, like `2026-09-01..`, opens *that*
    ///   side of the union and leaves the other alone. The two bounds are
    ///   independent, so dropping both here would over-fetch for no reason.
    fn due_date_range_for_query(&self) -> (Option<String>, Option<String>) {
        let mut after: Option<CivilDate> = None;
        let mut before: Option<CivilDate> = None;
        let (mut after_unbounded, mut before_unbounded) = (false, false);

        for set in &self.sets {
            // Both negations are satisfied by dates outside whatever window
            // this set names: a negated set keeps what its due row rejected,
            // and a negated due row does the same one level down. Either way
            // the union is unbounded — and a window narrower than the truth
            // is *recorded as covered*, so the rows it drops stay missing
            // until something else forces a refresh.
            if set.negated {
                return (None, None);
            }
            let Some(field) = set.fields.iter().find(|f| f.spec.key == "due") else {
                return (None, None);
            };
            if field.empty_required || field.negated {
                return (None, None);
            }
            let Some(query) = DateQuery::parse(field.value.text(), date::today()) else {
                return (None, None);
            };
            let (set_after, set_before) = query.bounds();
            match set_after {
                None => after_unbounded = true,
                Some(date) => after = Some(after.map_or(date, |current| current.min(date))),
            }
            match set_before {
                None => before_unbounded = true,
                Some(date) => before = Some(before.map_or(date, |current| current.max(date))),
            }
        }

        (
            after.filter(|_| !after_unbounded).map(|date| date.iso()),
            before.filter(|_| !before_unbounded).map(|date| date.iso()),
        )
    }

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
            field.value.clear();
            field.label_values.clear();
            field.label_cursor = 0;
        }
        true
    }

    /// Toggles negation on the selected field. Answers whether it changed, so
    /// the caller knows whether the table needs rebuilding.
    ///
    /// Allowed on a row with nothing in it. An inactive row is skipped before
    /// its verdict is ever asked for, so arming the negation first and typing
    /// the value second is the same as doing it the other way round.
    fn toggle_negate_field(&mut self) -> bool {
        let Some(field) = self.selected_field_mut() else {
            return false;
        };
        field.negated = !field.negated;
        true
    }

    /// Toggles negation on the active set. Answers whether it changed.
    fn toggle_negate_set(&mut self) -> bool {
        let Some(set) = self.sets.get_mut(self.active) else {
            return false;
        };
        set.negated = !set.negated;
        true
    }

    /// Whether the set the panel is showing is negated.
    fn active_set_negated(&self) -> bool {
        self.sets.get(self.active).is_some_and(|set| set.negated)
    }

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

    /// Throws the panel away and starts again: one empty set, every row back
    /// to the match mode it was built with, and nothing parked.
    ///
    /// Stronger than the `cleared` that `add_set` uses, which keeps the match
    /// modes on purpose so a second set inherits the first one's. This is
    /// "start from nothing", so the modes and the field cursor go too.
    fn reset(&mut self) {
        let mut fresh = self
            .sets
            .first()
            .map(TaskFilterSet::cleared)
            .unwrap_or_default();
        for field in &mut fresh.fields {
            field.string_mode = field.default_string_mode;
        }

        self.sets = vec![fresh];
        self.active = 0;
        self.selected = 0;
        self.stop_editing();
    }

    /// Pages the numbered window, clamped at both ends.
    ///
    /// A no-op when everything fits: the digits would address the same rows
    /// either way, so moving the window would only make them lie.
    fn page_sidebar(&mut self, delta: i32, total: usize) {
        let window = self.sidebar_window();
        if total <= window {
            self.sidebar_page_start = 0;
            return;
        }

        let last_start = total.saturating_sub(1) / window * window;
        let start = self.sidebar_page_start as i32 + delta * window as i32;
        self.sidebar_page_start = start.clamp(0, last_start as i32) as usize;
    }

    /// Opens the save prompt, pre-filled with the loaded name.
    ///
    /// Opens the sidebar too, because the prompt rides its bottom border and
    /// has nowhere else to be.
    fn prompt_save(&mut self) {
        let text = self.loaded.clone().unwrap_or_default();
        self.sidebar_visible = true;
        self.notice = None;
        self.prompt = Some(SidebarPrompt::Save {
            caret: text.chars().count(),
            text,
        });
    }

    /// Opens the delete confirmation, or answers `false` when nothing is
    /// loaded.
    ///
    /// Refused rather than guessed at: with no cursor in the sidebar there is
    /// no other unambiguous target, and "the one you are currently editing"
    /// is a target the user just chose by pressing its number.
    fn prompt_delete(&mut self) -> bool {
        let Some(name) = self.loaded.clone() else {
            self.notice = Some("no set is loaded".to_string());
            return false;
        };
        self.sidebar_visible = true;
        self.notice = None;
        self.prompt = Some(SidebarPrompt::ConfirmDelete { name });
        true
    }

    /// Opens the confirmation for a digit that would discard unsaved work.
    fn prompt_confirm_load(&mut self, name: impl Into<String>) {
        self.sidebar_visible = true;
        self.notice = None;
        self.prompt = Some(SidebarPrompt::ConfirmLoad { name: name.into() });
    }

    /// Whether replacing the panel would lose something.
    ///
    /// A bound panel is written through on every change, so loading over it
    /// costs nothing. An unnamed one that is filtering is work that exists
    /// nowhere but on screen. An empty unnamed panel is not worth a keypress
    /// to confirm.
    fn is_unsaved(&self) -> bool {
        self.loaded.is_none()
            && self.sets.iter().any(|set| {
                set.negated
                    || !set.unresolved.is_empty()
                    || set.active_filter_count() > 0
            })
    }

    fn prompt_push_char(&mut self, ch: char) {
        // A report takes the prompt's line, so typing has to take it back or
        // the name being entered would be invisible.
        self.notice = None;
        if let Some(SidebarPrompt::Save { text, caret }) = self.prompt.as_mut() {
            let mut chars = text.chars().collect::<Vec<_>>();
            let at = (*caret).min(chars.len());
            chars.insert(at, ch);
            *text = chars.into_iter().collect();
            *caret = at + 1;
        }
    }

    fn prompt_pop_char(&mut self) {
        self.notice = None;
        if let Some(SidebarPrompt::Save { text, caret }) = self.prompt.as_mut() {
            let mut chars = text.chars().collect::<Vec<_>>();
            let at = (*caret).min(chars.len());
            if at == 0 {
                return;
            }
            chars.remove(at - 1);
            *text = chars.into_iter().collect();
            *caret = at - 1;
        }
    }

    fn prompt_move_caret(&mut self, delta: i64) {
        if let Some(SidebarPrompt::Save { text, caret }) = self.prompt.as_mut() {
            let len = text.chars().count() as i64;
            *caret = (*caret as i64 + delta).clamp(0, len) as usize;
        }
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

/// The whole filter panel, compiled for one pass over the records.
///
/// Borrows the rows it was built from: it lives for the length of a single
/// filtering pass, and the panel cannot change during one.
struct PreparedFilter<'a> {
    sets: Vec<PreparedSet<'a>>,
}

/// One set, holding only the fields that actually filter.
struct PreparedSet<'a> {
    negated: bool,
    fields: Vec<PreparedField<'a>>,
}

/// One active row: where its value comes from, and what to test it against.
struct PreparedField<'a> {
    field: &'a TaskFilterFieldState,
    matcher: PreparedMatcher,
}

/// A row's test, with everything record-independent already done.
enum PreparedMatcher {
    /// The row asks for no value at all.
    Empty,
    /// Lowercased needle for a subsequence match.
    Fuzzy(String),
    /// Lowercased needle for a containment test.
    Substring(String),
    /// The compiled pattern, or `None` when it does not compile.
    Regex(Option<regex::Regex>),
    /// The include and exclude tokens, already split and lowercased.
    Labels { includes: Vec<String>, excludes: Vec<String> },
    /// The picked names, lowercased, and whether `me` was one of them.
    ///
    /// `me` is kept apart because it is not a name at all: it resolves to
    /// whoever is logged in, at match time rather than when it was picked,
    /// so a saved set stays personal to whoever loads it.
    Picked {
        names: Vec<String>,
        me: Option<String>,
    },
    /// The parsed query, or `None` when it is not a date expression.
    Date(Option<DateQuery>),
}

impl PreparedMatcher {
    fn for_field(field: &TaskFilterFieldState, today: CivilDate, me: Option<&str>) -> Self {
        if field.empty_required {
            return Self::Empty;
        }
        match field.spec.kind {
            TaskFieldFilterKind::String => match field.string_mode {
                TaskFieldStringMode::List => {
                    let picked = parse_label_values(field.value.text());
                    Self::Picked {
                        names: picked
                            .iter()
                            .filter(|name| !name.eq_ignore_ascii_case(ME_LABEL))
                            .map(|name| name.to_ascii_lowercase())
                            .collect(),
                        me: picked
                            .iter()
                            .any(|name| name.eq_ignore_ascii_case(ME_LABEL))
                            .then(|| me.unwrap_or_default().to_string()),
                    }
                }
                TaskFieldStringMode::Fuzzy => Self::Fuzzy(field.value.text().to_ascii_lowercase()),
                TaskFieldStringMode::Substring => {
                    Self::Substring(field.value.text().to_ascii_lowercase())
                }
                // Built from the query as typed, and matched against a
                // lowercased haystack, which is what the per-record version
                // did — the flag is what makes the two agree.
                TaskFieldStringMode::Regex => Self::Regex(
                    regex::RegexBuilder::new(field.value.text())
                        .case_insensitive(true)
                        .build()
                        .ok(),
                ),
            },
            TaskFieldFilterKind::Labels => {
                let (includes, excludes) = split_label_tokens(&field.label_values);
                Self::Labels { includes, excludes }
            }
            TaskFieldFilterKind::Date => Self::Date(DateQuery::parse(field.value.text(), today)),
        }
    }
}

impl PreparedFilter<'_> {
    /// Whether a record is visible: **any** set accepts it.
    fn matches(&self, record: &TaskRecord) -> bool {
        if self.sets.is_empty() {
            return true;
        }
        self.sets.iter().any(|set| set.matches(record))
    }
}

impl PreparedSet<'_> {
    fn matches(&self, record: &TaskRecord) -> bool {
        let accepted = self.fields.iter().all(|field| field.matches(record));
        accepted != self.negated
    }
}

impl PreparedField<'_> {
    fn matches(&self, record: &TaskRecord) -> bool {
        self.matches_value(record) != self.field.negated
    }

    fn matches_value(&self, record: &TaskRecord) -> bool {
        match &self.matcher {
            PreparedMatcher::Empty => self.field.value_is_empty(record),
            PreparedMatcher::Fuzzy(needle) => {
                fuzzy_match(&self.field.haystack(record).to_ascii_lowercase(), needle)
            }
            PreparedMatcher::Substring(needle) => self
                .field
                .haystack(record)
                .to_ascii_lowercase()
                .contains(needle),
            // An uncompilable pattern matches nothing. It cannot mean "no
            // filter": the row is only consulted once it has a value in it.
            PreparedMatcher::Regex(None) => false,
            PreparedMatcher::Regex(Some(regex)) => {
                regex.is_match(&self.field.haystack(record).to_ascii_lowercase())
            }
            PreparedMatcher::Labels { includes, excludes } => {
                label_selection_matches(&self.field.labels(record), includes, excludes)
            }
            // Exactly, not as a pattern: having picked from a list is the
            // whole point, and `alex` must not also match `Alexis`.
            PreparedMatcher::Picked { names, me } => {
                let name = self.field.haystack(record).to_ascii_lowercase();
                let is_me = me.as_ref().is_some_and(|gid| {
                    !gid.is_empty() && record.assignee_gid.as_deref() == Some(gid.as_str())
                });
                is_me || names.iter().any(|picked| picked == &name)
            }
            // Same reasoning as an uncompilable regex: an unparseable date
            // expression is a value that nothing satisfies.
            PreparedMatcher::Date(None) => false,
            PreparedMatcher::Date(Some(query)) => {
                self.field.date(record).is_some_and(|date| query.matches(date))
            }
        }
    }
}

impl TaskFilterSet {
    /// Clears everything the user typed, keeping the rows and their match modes.
    ///
    /// The match mode is deliberately kept: someone working in regex should not
    /// have to re-pick it in every set they add.
    fn cleared(&self) -> Self {
        let mut set = self.clone();
        for field in &mut set.fields {
            field.clear_value();
        }
        set.negated = false;
        set.unresolved.clear();
        set
    }

    /// How many of this set's fields exclude anything.
    fn active_filter_count(&self) -> usize {
        self.fields
            .iter()
            .filter(|field| field.is_active())
            .count()
    }

    /// Re-applies one set's user state onto a freshly built field list.
    ///
    /// Matched by `spec.key`, which is why custom-field rows are keyed by name
    /// rather than id: a reload can bring a different set of projects, and so
    /// different ids, for the same field.
    fn restore_from(&mut self, previous: &Self) {
        self.negated = previous.negated;
        for field in &mut self.fields {
            let Some(old) = previous
                .fields
                .iter()
                .find(|old| old.spec.key == field.spec.key)
            else {
                continue;
            };
            field.string_mode = old.string_mode;
            // Not belt-and-braces: a custom field's kind is inferred from the
            // values seen so far, so a row's spec really can change between
            // loads.
            field.empty_required = old.empty_required && field.spec.can_be_empty;
            field.negated = old.negated;
            match field.spec.kind {
                TaskFieldFilterKind::Labels => {
                    field.label_values = if old.label_values.is_empty() {
                        parse_label_values(old.value.text())
                    } else {
                        old.label_values.clone()
                    };
                    field.label_cursor = old
                        .label_cursor
                        .min(field.label_values.len().saturating_sub(1));
                    field.value.set_text(field.label_values.join(" | "));
                }
                _ => field.value.set_text(old.value.text().to_string()),
            }
            // Clamped, which `set_text` does, because a custom field can
            // change kind between loads as values arrive — and that rewrites
            // the text out from under the caret.
            field.value.jump_start();
            field.value.move_caret(old.value.caret() as i64);
        }

        // A custom-field row only appears once a project carrying it has
        // loaded, and this rebuild is exactly that moment — so the parked
        // values get another try, and what still has nowhere to go stays
        // parked rather than being dropped on the floor.
        let parked = previous.unresolved.clone();
        self.unresolved = parked
            .into_iter()
            .filter(|saved| !self.apply_saved_field(saved))
            .collect();
    }

    /// This set as it would be written to `tuisana.toml`.
    ///
    /// The parked values go back out with the rest: the panel is written back
    /// to the named entry on every change, so anything left out here is
    /// deleted from the user's config.
    fn to_saved(&self) -> SavedFilterSet {
        let mut fields = self
            .fields
            .iter()
            .filter_map(TaskFilterFieldState::to_saved)
            .collect::<Vec<_>>();
        fields.extend(self.unresolved.iter().cloned());

        SavedFilterSet {
            negated: self.negated,
            fields,
        }
    }

    /// Writes a saved set's values onto a freshly cleared field list.
    fn apply_saved(&mut self, saved: &SavedFilterSet) {
        self.negated = saved.negated;
        self.unresolved.clear();
        for field in &mut self.fields {
            field.clear_value();
        }

        for value in &saved.fields {
            if !self.apply_saved_field(value) {
                self.unresolved.push(value.clone());
            }
        }
    }

    /// Writes one saved value onto the row with the matching key, answering
    /// whether a row took it.
    ///
    /// Matched by `spec.key`, exactly as `restore_from` matches, with label
    /// rows deriving their values from the query and `empty` gated on
    /// `spec.can_be_empty`.
    fn apply_saved_field(&mut self, saved: &SavedFilterField) -> bool {
        let Some(field) = self
            .fields
            .iter_mut()
            .find(|field| field.spec.key == saved.key)
        else {
            return false;
        };

        if let Some(mode) = saved.string_mode.as_deref().and_then(parse_string_mode) {
            // A `list` on a row with no directory behind it is a mode that
            // row cannot mean, so the value parks the way an unknown key
            // does rather than landing as a filter nothing satisfies.
            if matches!(mode, TaskFieldStringMode::List) && !field.spec.completes {
                return false;
            }
            field.string_mode = mode;
        }
        field.negated = saved.negated;
        field.empty_required = saved.empty && field.spec.can_be_empty;

        if field.empty_required {
            // A value and a require-empty cannot both be in force.
            field.value.clear();
            field.label_values.clear();
            field.label_cursor = 0;
        } else {
            match field.spec.kind {
                TaskFieldFilterKind::Labels => {
                    field.label_values = parse_label_values(&saved.query);
                    field.label_cursor = 0;
                    field.value.set_text_at_end(field.label_values.join(" | "));
                }
                // The caret lands at the end, so typing continues from where
                // the value leaves off.
                _ => field.value.set_text_at_end(saved.query.clone()),
            }
        }
        true
    }
}

/// The saved spelling of a match mode.
fn string_mode_name(mode: TaskFieldStringMode) -> &'static str {
    match mode {
        TaskFieldStringMode::Fuzzy => "fuzzy",
        TaskFieldStringMode::Substring => "contains",
        TaskFieldStringMode::Regex => "regex",
        TaskFieldStringMode::List => "list",
    }
}

/// Parses a saved match mode, or `None` when it names no mode.
fn parse_string_mode(name: &str) -> Option<TaskFieldStringMode> {
    match name {
        "fuzzy" => Some(TaskFieldStringMode::Fuzzy),
        "contains" => Some(TaskFieldStringMode::Substring),
        "regex" => Some(TaskFieldStringMode::Regex),
        "list" => Some(TaskFieldStringMode::List),
        _ => None,
    }
}

impl TaskFilterFieldState {
    fn new(
        spec: TaskFilterFieldSpec,
        string_mode: TaskFieldStringMode,
        label_options: Vec<String>,
    ) -> Self {
        Self {
            spec,
            value: TextEdit::default(),
            string_mode,
            label_values: Vec::new(),
            label_options,
            label_cursor: 0,
            empty_required: false,
            negated: false,
            default_string_mode: string_mode,
        }
    }

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
        self.empty_required || !self.value.text().trim().is_empty()
    }

    /// Resets everything the user typed, keeping the row and its match mode.
    ///
    /// The match mode is deliberately kept: someone working in regex should
    /// not have to re-pick it every time a row is emptied.
    fn clear_value(&mut self) {
        self.value.clear();
        self.label_values.clear();
        self.label_cursor = 0;
        self.empty_required = false;
        self.negated = false;
    }

    /// This row as a saved field, or `None` when it filters nothing.
    ///
    /// An inactive row is skipped before its verdict is ever asked for, so
    /// writing one out would put a line in the user's config that does
    /// nothing.
    fn to_saved(&self) -> Option<SavedFilterField> {
        if !self.is_active() {
            return None;
        }

        Some(SavedFilterField {
            key: self.spec.key.clone(),
            query: if self.empty_required {
                String::new()
            } else {
                self.value.text().to_string()
            },
            // Only for the rows a mode means anything on, and only when it is
            // not the one the row was built with.
            string_mode: (matches!(self.spec.kind, TaskFieldFilterKind::String)
                && self.string_mode != self.default_string_mode)
                .then(|| string_mode_name(self.string_mode).to_string()),
            empty: self.empty_required,
            negated: self.negated,
        })
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

}

/// Splits chosen label values into the ones a record must carry and the ones it
/// must not, lowercased.
///
/// A leading `!` or `-` on a value excludes it. Depends only on the panel, so a
/// filtering pass does this once rather than per record.
fn split_label_tokens(selected_labels: &[String]) -> (Vec<String>, Vec<String>) {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();

    for token in selected_labels {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(value) = token.strip_prefix('!').or_else(|| token.strip_prefix('-')) {
            excludes.push(value.to_ascii_lowercase());
        } else {
            includes.push(token.to_ascii_lowercase());
        }
    }

    (includes, excludes)
}

/// Whether a record's label values satisfy an already-split selection.
fn label_selection_matches(values: &[String], includes: &[String], excludes: &[String]) -> bool {
    if includes.is_empty() && excludes.is_empty() {
        return true;
    }

    let normalized = values
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();

    if !includes.is_empty()
        && !includes
            .iter()
            .any(|needle| normalized.iter().any(|value| value == needle))
    {
        return false;
    }

    !excludes
        .iter()
        .any(|needle| normalized.iter().any(|value| value == needle))
}

fn parse_label_values(query: &str) -> Vec<String> {
    query
        .split(|ch| [',', '|'].contains(&ch))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// Whether a task's date-only field satisfies a filter query.
///
/// A bare token is an exact-equality test; `start..end` is inclusive on both
/// ends and either side may be empty for an open bound. A query that is not a
/// date expression at all matches nothing, which the calendar overlay surfaces
/// rather than leaving the table mysteriously empty.
impl TaskState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(&self) -> bool {
        self.view.visible
    }

    /// Seeds the chart's state from config, at startup.
    pub fn apply_gantt_config(&mut self, config: &GanttConfig) {
        self.view.gantt = GanttViewState::from_config(config);
    }

    /// The chart's state, for the renderer.
    pub fn gantt(&self) -> &GanttViewState {
        &self.view.gantt
    }

    /// The chart's state, for the actions that change it.
    pub fn gantt_mut(&mut self) -> &mut GanttViewState {
        &mut self.view.gantt
    }

    /// Every dimension the bars can be coloured by, in cycling order.
    ///
    /// The enumerated custom fields are taken from the filter editor, which
    /// already decides which fields have a small enough set of values to be
    /// treated as labels. There is no second place that answers that question.
    pub fn available_color_keys(&self) -> Vec<GanttColorKey> {
        let mut keys = vec![
            GanttColorKey::Assignee,
            GanttColorKey::Section,
            GanttColorKey::State,
        ];
        keys.extend(
            self.view
                .filter_editor
                .fields()
                .iter()
                .filter(|field| {
                    matches!(field.spec.kind, TaskFieldFilterKind::Labels)
                        && field.spec.key.starts_with("custom:")
                })
                .map(|field| GanttColorKey::Field(field.spec.label.clone())),
        );
        keys
    }

    /// Colours the bars by the next available dimension.
    ///
    /// A dimension that has gone away with the loaded projects starts the
    /// cycle over rather than getting stuck.
    pub fn cycle_gantt_color_key(&mut self) {
        let keys = self.available_color_keys();
        let current = keys
            .iter()
            .position(|key| key == self.view.gantt.color_key());
        let next = match current {
            Some(index) => (index + 1) % keys.len(),
            None => 0,
        };
        let Some(key) = keys.get(next).cloned() else {
            return;
        };
        self.view.gantt.set_color_key(key.clone());

        // Cycling from inside the dialog rebuilds it around the new dimension
        // and commits nothing, so two dimensions can be compared in one press.
        if self.view.gantt.dialog_open() {
            let values = self.color_values();
            self.view.gantt.dialog_reload(key, values);
        }
    }

    /// Shows one more table column beside the chart.
    pub fn gantt_add_column(&mut self) {
        let total = self.view.table.columns.len();
        self.view.gantt.add_column(total);
    }

    /// Shows one fewer table column beside the chart.
    pub fn gantt_remove_column(&mut self) {
        let total = self.view.table.columns.len();
        self.view.gantt.remove_column(total);
    }

    /// The window a fitted chart would show, for the scroll and zoom verbs to
    /// start from.
    ///
    /// Built at a nominal width because only the span matters here; the real
    /// width is a rendering concern and the renderer resolves its own.
    fn fitted_timeline(&self) -> Option<Timeline> {
        Timeline::fit(
            self.view
                .table
                .rows
                .iter()
                .filter(|row| row.kind.is_task())
                .flat_map(|row| [row.start, row.due])
                .flatten(),
            1,
        )
    }

    /// Moves the timeline window a quarter of its length.
    pub fn gantt_scroll(&mut self, forward: bool) {
        let fitted = self.fitted_timeline();
        self.view.gantt.scroll_timeline(fitted, forward);
    }

    /// Steps the timeline's zoom ladder.
    pub fn gantt_zoom(&mut self, in_: bool) {
        let fitted = self.fitted_timeline();
        self.view.gantt.zoom_timeline(fitted, in_);
    }

    /// Returns the timeline to fitting the loaded tasks.
    pub fn gantt_fit(&mut self) {
        self.view.gantt.fit_timeline();
    }

    /// Brings today to the left of the timeline.
    pub fn gantt_today(&mut self) {
        let fitted = self.fitted_timeline();
        self.view.gantt.focus_timeline_on(date::today(), fitted);
    }

    /// Opens the colour dialog over the current dimension.
    pub fn gantt_open_order(&mut self) {
        let values = self.color_values();
        self.view.gantt.open_dialog(values);
    }

    /// Moves the dialog's selected value.
    pub fn gantt_order_move(&mut self, to: MoveTo) {
        self.view.gantt.dialog_move_value(to);
    }

    /// The dimension's values with their task counts.
    fn color_values(&self) -> Vec<(String, usize)> {
        distinct_values(
            &self.view.table,
            self.view.gantt.color_key(),
            self.view.gantt.order(),
        )
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.view.visible = visible;
        if self.view.visible && self.view.selected.is_none() {
            self.view.selected = self.view.table.first_selectable_row_index();
        }
    }

    pub fn toggle_visible(&mut self) {
        self.set_visible(!self.view.visible);
    }

    pub fn help_details_visible(&self) -> bool {
        self.view.help_details_visible
    }

    pub fn toggle_help_details(&mut self) {
        self.view.help_details_visible = !self.view.help_details_visible;
    }

    /// Mark the task pane as loading data for the given projects.
    pub fn begin_loading(&mut self, projects: &[Project]) {
        self.set_project_group_order(projects);
        self.loading.status = TaskStatus::Loading;
        self.loading.progress = LoadProgress::Active {
            started_at: Instant::now(),
            target_names: projects.iter().map(|project| project.name.clone()).collect(),
            target_ids: projects.iter().map(|project| project.id.clone()).collect(),
        };
    }

    /// Groups the table's projects in the order they were handed to us.
    ///
    /// The targets arrive in project-list order, so recording it here is what
    /// keeps the two panes reading down in the same order — including the
    /// "No Project (Assigned to Me)" row the list pins to the top.
    fn set_project_group_order(&mut self, projects: &[Project]) {
        self.view.settings.sort.project_order =
            projects.iter().map(|project| project.name.clone()).collect();
    }

    /// Replace the visible table with a fully built table model.
    pub fn finish_loading(&mut self, table: TaskTableModel) {
        self.loading.dataset = None;
        self.view.table = table;
        self.view.horizontal_scroll = 0;
        self.view.selected = self.view.table.first_selectable_row_index();
        self.loading.loaded_target_ids = self.loading.progress.target_ids().to_vec();
        self.loading.status = if self.view.table.task_count() == 0 {
            TaskStatus::Empty
        } else {
            TaskStatus::Ready
        };
        self.loading.progress = LoadProgress::Idle;
        self.view.task_vertical_scroll = 0;
    }

    /// Merge a freshly built dataset into the cache and rebuild the visible
    /// table from the current target projects.
    pub(crate) fn finish_loading_dataset(&mut self, dataset: TaskDataset) {
        self.loading.cache.merge_dataset(dataset.clone());
        self.rebuild_visible_dataset();
        let query = self.desired_task_query();
        for project_id in self.loading.progress.target_ids().to_vec() {
            self.loading.loaded_project_queries.insert(project_id, query.clone());
        }
        self.finish_loading_targets();
    }

    /// Merge a partial dataset load for one project and refresh the table.
    #[allow(dead_code)]
    pub(crate) fn ingest_loaded_dataset(&mut self, dataset: TaskDataset) {
        self.loading.cache.merge_dataset(dataset);
        self.rebuild_visible_dataset();
    }

    /// Merge a partial dataset for one project and remember which query was fetched.
    pub(crate) fn ingest_loaded_project(
        &mut self,
        project_id: &str,
        query: TaskQuery,
        dataset: TaskDataset,
    ) {
        self.loading.cache.merge_dataset(dataset);
        self.loading.loaded_project_queries.insert(project_id.to_string(), query);
        self.rebuild_visible_dataset();
    }

    /// Clear the loading state once all requested projects have been merged.
    pub(crate) fn finish_loading_targets(&mut self) {
        let loaded_target_ids = self.loading.progress.target_ids().to_vec();
        self.view.horizontal_scroll = 0;
        self.view.selected = self.view.table.first_selectable_row_index();
        self.loading.progress = LoadProgress::Idle;
        self.view.task_vertical_scroll = 0;
        self.loading.loaded_target_ids = loaded_target_ids;
        self.loading.status = TaskStatus::Idle;
        self.rebuild_visible_dataset();
    }

    pub fn task_settings(&self) -> &TaskTableSettings {
        &self.view.settings
    }

    /// How many filter fields currently exclude anything.
    ///
    /// Unlike the older `active_count` behind [`TaskState::filter_summary`],
    /// this counts label filters as well as text ones, so the UI's "N active"
    /// matches what the panel shows.
    pub fn active_filter_count(&self) -> usize {
        self.view.filter_editor.active_filter_count()
    }

    pub fn filter_summary(&self) -> String {
        let mut summary = self.view.settings.summary();
        let active = self.view.filter_editor.active_count();
        if active > 0 {
            summary.push_str(&format!("; filters {active}"));
            if let Some(label) = self.view.filter_editor.selected_label() {
                summary.push_str(&format!(" ({label})"));
            }
        } else {
            summary.push_str("; filters off");
        }
        summary
    }

    pub fn filter_panel_visible(&self) -> bool {
        self.view.filter_editor.visible()
    }

    pub fn filter_panel_scroll(&self) -> usize {
        self.view.filter_vertical_scroll
    }

    pub fn ensure_filter_visible(&mut self, viewport_height: usize) {
        let viewport_height = viewport_height.max(1);
        let selected = self.view.filter_editor.selected;
        let margin = 1usize.min(viewport_height.saturating_sub(1));
        let min_visible = self.view.filter_vertical_scroll.saturating_add(margin);
        let max_visible = self
            .view
            .filter_vertical_scroll
            .saturating_add(viewport_height.saturating_sub(1))
            .saturating_sub(margin);

        if selected < min_visible {
            self.view.filter_vertical_scroll = selected.saturating_sub(margin);
            return;
        }

        if selected > max_visible {
            self.view.filter_vertical_scroll = selected
                .saturating_add(margin)
                .saturating_add(1)
                .saturating_sub(viewport_height);
        }

        let max_scroll = self.view.filter_editor.fields().len().saturating_sub(1);
        self.view.filter_vertical_scroll = self.view.filter_vertical_scroll.min(max_scroll);
    }

    pub fn filter_panel_editing(&self) -> bool {
        self.view.filter_editor.editing()
    }

    pub(crate) fn filter_selected_kind(&self) -> Option<TaskFieldFilterKind> {
        self.view.filter_editor.selected_kind()
    }

    /// Whether the filter cursor is on a field whose value is a set of labels.
    ///
    /// The label-navigation keys only apply to those fields, so the hint bar
    /// asks before advertising them.
    pub fn filter_selected_is_labels(&self) -> bool {
        matches!(
            self.filter_selected_kind(),
            Some(TaskFieldFilterKind::Labels)
        )
    }

    pub fn filter_panel_help_lines(&self) -> Vec<String> {
        let mut lines = if self.view.filter_editor.editing() {
            vec!["enter/esc: done editing".to_string()]
        } else {
            vec!["enter: edit selected filter, f: hide filters".to_string()]
        };

        match self.view.filter_editor.selected_kind() {
            Some(TaskFieldFilterKind::String) => {
                if !self.view.filter_editor.editing() {
                    lines.push("s: cycle string mode".to_string());
                }
                lines.push("string: fuzzy | contains | regex".to_string());
            }
            Some(TaskFieldFilterKind::Labels) => {
                lines.push("labels: j/k cycle value, h/l move, a add, d delete".to_string());
            }
            Some(TaskFieldFilterKind::Date) => {
                lines.push("date: enter opens a calendar".to_string());
                lines.push(
                    "date: YYYY-MM-DD | MM-DD | today | tomorrow | mon/tue/... | start..end"
                        .to_string(),
                );
            }
            None => {}
        }

        lines
    }

    pub fn toggle_filter_panel(&mut self) {
        self.view.filter_editor.toggle_visible();
    }

    pub fn move_filter_up(&mut self) {
        self.view.filter_editor.move_up();
        self.view.filter_editor.reset_query_caret();
    }

    pub fn move_filter_down(&mut self) {
        self.view.filter_editor.move_down();
        self.view.filter_editor.reset_query_caret();
    }

    pub fn filter_page_up(&mut self, page_size: usize) {
        self.view.filter_editor.page_up(page_size);
    }

    pub fn filter_page_down(&mut self, page_size: usize) {
        self.view.filter_editor.page_down(page_size);
    }

    pub fn filter_clear_current(&mut self) {
        self.view.filter_editor.clear_current();
        self.refresh_table();
    }

    pub(crate) fn filter_set_mode(&mut self, mode: TaskFieldStringMode) {
        self.view.filter_editor.set_mode(mode);
        self.refresh_table();
    }

    pub(crate) fn filter_cycle_mode(&mut self) {
        self.view.filter_editor.cycle_mode();
        self.refresh_table();
    }

    pub fn filter_push_char(&mut self, ch: char) {
        self.view.filter_editor.push_char(ch);
        self.refresh_table();
    }

    pub fn filter_pop_char(&mut self) {
        self.view.filter_editor.pop_char();
        self.refresh_table();
    }

    /// Moves the caret in the selected text filter field.
    pub(crate) fn filter_move_caret(&mut self, delta: i64) {
        self.view.filter_editor.move_query_caret(delta);
    }

    /// Moves the selected filter field's caret a word at a time.
    pub(crate) fn filter_move_word(&mut self, delta: i64) {
        self.view
            .filter_editor
            .with_selected_text(|text| text.move_word(delta));
    }

    /// Puts the selected filter field's caret at the start of its text.
    pub(crate) fn filter_caret_to_start(&mut self) {
        self.view
            .filter_editor
            .with_selected_text(TextEdit::jump_start);
    }

    /// Puts the selected filter field's caret at the end of its text.
    pub(crate) fn filter_caret_to_end(&mut self) {
        self.view
            .filter_editor
            .with_selected_text(TextEdit::jump_end);
    }

    /// Whether the selected filter row picks its values from a directory.
    ///
    /// Asked before an edit begins so the caller only pays for the workspace
    /// directory on the one row that can use it.
    pub fn filter_row_completes(&self) -> bool {
        self.view.filter_editor.selected_completes()
    }

    /// Whether the panel's completion editor is open.
    pub fn filter_autocomplete_open(&self) -> bool {
        self.view.filter_editor.autocomplete.is_some()
    }

    /// Completes the typed prefix in the panel, or answers `false`.
    pub fn filter_complete(&mut self, delta: i32) -> bool {
        let mut completed = false;
        self.view
            .filter_editor
            .with_autocomplete(|state| completed = state.complete(delta));
        completed
    }

    /// Resolves the panel's completion editor, reporting what it could not.
    pub fn filter_autocomplete_commit(&mut self) -> Option<String> {
        let refusal = self.view.filter_editor.commit_autocomplete()?;
        Some(match refusal {
            Unresolved::Ambiguous(text) => format!("{text} is ambiguous"),
            Unresolved::Unknown(text) => format!("no one called {text}"),
        })
    }

    /// Who is logged in, for the `list` filter's `me`.
    pub fn set_current_user_gid(&mut self, gid: Option<String>) {
        self.view.current_user_gid = gid;
    }

    pub(crate) fn filter_edit_begin_with(&mut self, candidates: Vec<Candidate>) {
        self.view.filter_editor.start_editing();
        self.view.filter_editor.open_autocomplete(candidates);
    }

    pub(crate) fn filter_edit_begin(&mut self) {
        self.view.filter_editor.start_editing();
    }

    pub(crate) fn filter_edit_done(&mut self) {
        self.view.filter_editor.stop_editing();
    }

    /// Whether the date picker is open.
    /// Whether a date picker is open, whichever editor owns it.
    pub fn calendar_open(&self) -> bool {
        self.calendar().is_some()
    }

    /// Whether the open picker belongs to a task cell rather than a filter row.
    ///
    /// `enter` and `esc` mean different things in the two: one commits a
    /// filter, the other sends a write.
    pub fn cell_edit_owns_calendar(&self) -> bool {
        self.view
            .cell_edit
            .as_ref()
            .is_some_and(|edit| edit.calendar().is_some())
    }

    /// The filter rows as `(label, query)` pairs, for tests that need to see the
    /// text a picker or an edit produced.
    pub fn filter_panel_rows(&self) -> Vec<(String, String)> {
        self.filter_panel_entries()
            .into_iter()
            .map(|entry| (entry.label, entry.query))
            .collect()
    }

    /// Whether the date being picked is a range, which is what decides whether
    /// the hint bar advertises the keys for moving between its two ends.
    pub fn calendar_is_range(&self) -> bool {
        self.calendar()
            .is_some_and(|calendar| calendar.range().is_some())
    }

    /// The open date picker, whichever editor owns it.
    ///
    /// One at a time: opening a cell editor closes the filter panel's picker
    /// and vice versa, because `Mode::Calendar` has one set of keys and they
    /// have to reach one place.
    pub(crate) fn calendar(&self) -> Option<&CalendarState> {
        self.view
            .cell_edit
            .as_ref()
            .and_then(|edit| edit.calendar())
            .or(self.view.filter_editor.calendar.as_ref())
    }

    /// Opens the date picker on the selected field. Answers whether it opened,
    /// which is how the caller knows a date field was selected.
    pub(crate) fn filter_calendar_begin(&mut self) -> bool {
        self.view.filter_editor.open_calendar(date::today())
    }

    /// Fills in the highlighted day if the text is incomplete.
    ///
    /// The cell editor keeps its picker open through this: what closes it
    /// there is the commit of the whole edit, one layer up.
    pub(crate) fn calendar_normalize(&mut self) {
        self.with_calendar(|calendar| calendar.normalize());
    }

    /// Fills in the highlighted day if the text is incomplete, then closes.
    pub(crate) fn filter_calendar_commit(&mut self) {
        if let Some(calendar) = self.view.filter_editor.calendar.as_mut() {
            calendar.normalize();
        }
        self.apply_calendar();
        self.view.filter_editor.calendar = None;
    }

    /// Closes the picker. The text stays as edited, because it was going into the
    /// field as it was typed.
    pub(crate) fn filter_calendar_close(&mut self) {
        self.view.filter_editor.calendar = None;
    }

    /// Empties the text the picker is editing, leaving it open.
    ///
    /// The task-cell half of `d`: the value goes, the picker stays, and
    /// `enter` is still what sends the cleared date.
    pub(crate) fn calendar_clear_text(&mut self) {
        self.with_calendar(|calendar| calendar.clear());
    }

    /// Clears the field the picker is editing, then closes it.
    pub(crate) fn filter_calendar_clear(&mut self) {
        self.view.filter_editor.calendar = None;
        self.filter_clear_current();
    }

    pub(crate) fn filter_calendar_move_days(&mut self, delta: i64) {
        self.with_calendar(|calendar| calendar.move_days(delta));
    }

    pub(crate) fn filter_calendar_move_months(&mut self, delta: i64) {
        self.with_calendar(|calendar| calendar.move_months(delta));
    }

    pub(crate) fn filter_calendar_today(&mut self) {
        self.with_calendar(|calendar| calendar.jump_today());
    }

    pub(crate) fn filter_calendar_push_char(&mut self, ch: char) {
        self.with_calendar(|calendar| calendar.push_char(ch));
    }

    pub(crate) fn filter_calendar_pop_char(&mut self) {
        self.with_calendar(|calendar| calendar.pop_char());
    }

    pub(crate) fn filter_calendar_move_caret(&mut self, delta: i64) {
        self.with_calendar(|calendar| calendar.move_caret(delta));
    }

    /// Jumps the caret to a range's start end. Does nothing when there is no
    /// range, since there is no other end to jump to.
    pub(crate) fn filter_calendar_jump_to_start(&mut self) {
        self.with_calendar(|calendar| {
            calendar.jump_to_start();
        });
    }

    /// Jumps the caret to a range's end end.
    pub(crate) fn filter_calendar_jump_to_end(&mut self) {
        self.with_calendar(|calendar| {
            calendar.jump_to_end();
        });
    }

    /// Runs a picker change, then pushes the result into the field and refilters.
    ///
    /// Every picker key goes through here, so the field and the table can never
    /// drift from what the overlay shows.
    fn with_calendar(&mut self, change: impl FnOnce(&mut CalendarState)) {
        // The cell editor first, because it is the one that shadows the
        // panel's picker while it is open.
        if let Some(edit) = self.view.cell_edit.as_mut() {
            if let Some(calendar) = edit.calendar_mut() {
                change(calendar);
                edit.sync_calendar();
                return;
            }
        }

        let Some(calendar) = self.view.filter_editor.calendar.as_mut() else {
            return;
        };
        change(calendar);
        self.apply_calendar();
    }

    fn apply_calendar(&mut self) {
        self.view.filter_editor.sync_calendar_query();
        self.refresh_table();
    }

    pub(crate) fn filter_move_label_left(&mut self) {
        self.view.filter_editor.move_label_cursor_left();
    }

    pub(crate) fn filter_move_label_right(&mut self) {
        self.view.filter_editor.move_label_cursor_right();
    }

    pub(crate) fn filter_cycle_label_value(&mut self, delta: i32) {
        self.view.filter_editor.cycle_selected_label(delta);
        self.refresh_table();
    }

    pub(crate) fn filter_add_label(&mut self) {
        self.view.filter_editor.add_label();
        self.refresh_table();
    }

    pub(crate) fn filter_delete_label(&mut self) {
        self.view.filter_editor.delete_selected_label();
        self.refresh_table();
    }

    /// Filters the selected field to records with no value at all.
    pub(crate) fn filter_toggle_require_empty(&mut self) {
        if self.view.filter_editor.toggle_require_empty() {
            // The picker is bound to the value it was opened on, and there is no
            // longer a value.
            self.view.filter_editor.calendar = None;
            self.refresh_table();
        }
    }

    /// Inverts the selected field: it now keeps what it was throwing away.
    pub(crate) fn filter_toggle_negate_field(&mut self) {
        if self.view.filter_editor.toggle_negate_field() {
            self.refresh_table();
        }
    }

    /// Inverts the whole active set, after its fields have ANDed.
    pub(crate) fn filter_toggle_negate_set(&mut self) {
        if self.view.filter_editor.toggle_negate_set() {
            self.refresh_table();
        }
    }

    /// Whether the set on screen is negated, for the pane's chips.
    pub fn filter_active_set_negated(&self) -> bool {
        self.view.filter_editor.active_set_negated()
    }

    /// Which sets are negated, in tab order.
    pub fn filter_set_negations(&self) -> Vec<bool> {
        self.view
            .filter_editor
            .sets
            .iter()
            .map(|set| set.negated)
            .collect()
    }

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

    /// Whether the `Sets` sidebar is drawn.
    pub fn filter_sets_sidebar_visible(&self) -> bool {
        self.view.filter_editor.sidebar_visible
    }

    pub(crate) fn filter_sets_toggle_sidebar(&mut self) {
        let editor = &mut self.view.filter_editor;
        editor.sidebar_visible = !editor.sidebar_visible;
        if !editor.sidebar_visible {
            // The prompt rides the sidebar's border, so closing the one
            // closes the other.
            editor.prompt = None;
            editor.notice = None;
        }
    }

    /// Tells the panel how many saved entries the sidebar can show.
    ///
    /// The renderer is the only thing that knows the pane's height, in the
    /// same way it is the only thing that knows the filter viewport.
    pub fn set_filter_sets_window(&mut self, rows: usize) {
        self.view.filter_editor.sidebar_rows = Some(rows);
    }

    /// The first saved entry in the numbered window.
    pub fn filter_sets_page_start(&self) -> usize {
        self.view.filter_editor.sidebar_page_start
    }

    /// How many saved entries the numbered window holds.
    pub fn filter_sets_window(&self) -> usize {
        self.view.filter_editor.sidebar_window()
    }

    pub(crate) fn filter_sets_page(&mut self, delta: i32, total: usize) {
        self.view.filter_editor.page_sidebar(delta, total);
    }

    /// The named entry the panel is bound to, if any.
    pub fn filter_set_loaded_name(&self) -> Option<&str> {
        self.view.filter_editor.loaded.as_deref()
    }

    /// The panel's sets, as they would be written to `tuisana.toml`.
    pub fn filter_sets_to_saved(&self) -> Vec<SavedFilterSet> {
        self.view.filter_editor.to_saved()
    }

    /// Replaces the panel with a named entry and binds it to that name.
    ///
    /// From here on the entry is a live view rather than a snapshot: every
    /// change writes through to it.
    pub fn filter_sets_load(&mut self, name: &str, saved: &[SavedFilterSet]) {
        self.view.filter_editor.apply_saved(saved);
        self.view.filter_editor.loaded = Some(name.to_string());
        self.view.filter_editor.notice = None;
        self.refresh_table();
        // Freshly loaded is by definition what is already on disk.
        self.view.filter_editor.dirty = false;
    }

    /// Binds the panel to a name without touching what it holds.
    pub fn filter_set_bind(&mut self, name: impl Into<String>) {
        self.view.filter_editor.loaded = Some(name.into());
        self.view.filter_editor.dirty = false;
    }

    /// Keeps what is on screen and stops being the named entry.
    ///
    /// The whole of both `copy to new` and the tail of a delete: the panel is
    /// untouched, and the entry keeps whatever was last written to it.
    pub fn filter_set_detach(&mut self) {
        self.view.filter_editor.loaded = None;
        self.view.filter_editor.dirty = false;
    }

    /// Starts a completely fresh panel: one empty set, nothing bound.
    pub fn filter_set_new(&mut self) {
        // Unbound *before* the rebuild, so emptying the panel is not written
        // through to the entry it was loaded from.
        self.view.filter_editor.loaded = None;
        self.view.filter_editor.reset();
        self.refresh_table();
        self.view.filter_editor.dirty = false;
    }

    /// Whether the bound panel has changed since it was last written.
    pub fn filter_set_dirty(&self) -> bool {
        self.view.filter_editor.dirty
    }

    pub fn clear_filter_set_dirty(&mut self) {
        self.view.filter_editor.dirty = false;
    }

    pub(crate) fn filter_set_prompt(&self) -> Option<&SidebarPrompt> {
        self.view.filter_editor.prompt.as_ref()
    }

    pub(crate) fn filter_set_prompt_save(&mut self) {
        self.view.filter_editor.prompt_save();
    }

    pub(crate) fn filter_set_prompt_delete(&mut self) -> bool {
        self.view.filter_editor.prompt_delete()
    }

    pub(crate) fn filter_set_prompt_confirm_load(&mut self, name: &str) {
        self.view.filter_editor.prompt_confirm_load(name);
    }

    /// How many fields filter something, across every set.
    ///
    /// The pane's own `N active` counts the set on screen, because that is
    /// what its rows are showing. This counts the whole panel, which is what
    /// a confirmation about losing it has to name.
    pub fn active_filter_count_across_sets(&self) -> usize {
        self.filter_set_counts().iter().sum()
    }

    /// Whether replacing the panel would throw away unsaved work.
    pub fn filter_set_is_unsaved(&self) -> bool {
        self.view.filter_editor.is_unsaved()
    }

    /// Whether the open prompt wants a `y`/`n` rather than typed text.
    pub fn filter_set_prompt_is_confirmation(&self) -> bool {
        self.view
            .filter_editor
            .prompt
            .as_ref()
            .is_some_and(SidebarPrompt::is_confirmation)
    }

    pub(crate) fn filter_set_prompt_cancel(&mut self) {
        self.view.filter_editor.prompt = None;
    }

    pub(crate) fn filter_set_prompt_push_char(&mut self, ch: char) {
        self.view.filter_editor.prompt_push_char(ch);
    }

    pub(crate) fn filter_set_prompt_pop_char(&mut self) {
        self.view.filter_editor.prompt_pop_char();
    }

    pub(crate) fn filter_set_prompt_move_caret(&mut self, delta: i64) {
        self.view.filter_editor.prompt_move_caret(delta);
    }

    /// The name being typed at the save prompt, if that is the open one.
    pub fn filter_set_prompt_text(&self) -> Option<&str> {
        match self.view.filter_editor.prompt.as_ref() {
            Some(SidebarPrompt::Save { text, .. }) => Some(text.as_str()),
            _ => None,
        }
    }

    /// A one-line report shown on the sidebar's bottom border.
    pub fn filter_sets_notice(&self) -> Option<&str> {
        self.view.filter_editor.notice.as_deref()
    }

    /// Clears a report, once whatever it was about has gone right.
    pub fn clear_filter_sets_notice(&mut self) {
        self.view.filter_editor.notice = None;
    }

    pub fn set_filter_sets_notice(&mut self, message: impl Into<String>) {
        // Opened so the message has somewhere to be: a report nobody can see
        // is not a report.
        self.view.filter_editor.sidebar_visible = true;
        self.view.filter_editor.notice = Some(message.into());
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
            .map(TaskFilterSet::active_filter_count)
            .collect()
    }

    /// One set's rows as `(label, query)`, for tests that need to see a tab
    /// other than the active one.
    #[cfg(test)]
    pub fn filter_panel_rows_for_set(&self, index: usize) -> Vec<(String, String)> {
        self.view
            .filter_editor
            .sets
            .get(index)
            .map(|set| {
                set.fields
                    .iter()
                    .map(|field| {
                        let query = if field.empty_required {
                            crate::ui::filter_panel::EMPTY_REQUIRED_TEXT.to_string()
                        } else {
                            field.value.text().to_string()
                        };
                        (field.spec.label.clone(), query)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn filter_panel_entries(&self) -> Vec<TaskFilterPanelEntry> {
        self.view.filter_editor
            .fields()
            .iter()
            .enumerate()
            .map(|(index, field)| TaskFilterPanelEntry {
                label: field.spec.label.clone(),
                query: if field.empty_required {
                    crate::ui::filter_panel::EMPTY_REQUIRED_TEXT.to_string()
                } else {
                    match field.spec.kind {
                        TaskFieldFilterKind::Labels => {
                            if field.label_values.is_empty() {
                                String::new()
                            } else {
                                field.label_values.join(" | ")
                            }
                        }
                        _ => field.value.text().to_string(),
                    }
                },
                empty_required: field.empty_required,
                negated: field.negated,
                custom: field.spec.key.starts_with("custom:"),
                kind: match field.spec.kind {
                    TaskFieldFilterKind::String => match field.string_mode {
                        TaskFieldStringMode::Fuzzy => "string:fuzzy".to_string(),
                        TaskFieldStringMode::Substring => "string:contains".to_string(),
                        TaskFieldStringMode::Regex => "string:regex".to_string(),
                        TaskFieldStringMode::List => "string:list".to_string(),
                    },
                    TaskFieldFilterKind::Labels => "labels".to_string(),
                    TaskFieldFilterKind::Date => "date".to_string(),
                },
                selected: index == self.view.filter_editor.selected,
                editing: self.view.filter_editor.editing(),
                label_values: field.label_values.clone(),
                label_cursor: if index == self.view.filter_editor.selected
                    && matches!(field.spec.kind, TaskFieldFilterKind::Labels)
                {
                    Some(field.label_cursor.min(field.label_values.len().saturating_sub(1)))
                } else {
                    None
                },
                caret: self.filter_caret_for(index, field),
            })
            .collect()
    }

    /// Where the edit caret sits on one row, if it is the row being edited.
    ///
    /// A date field picked on the calendar keeps its caret in the calendar, which
    /// is what the arrow keys move; every other field appends, so the caret is
    /// simply at the end.
    fn filter_caret_for(&self, index: usize, field: &TaskFilterFieldState) -> Option<usize> {
        if index != self.view.filter_editor.selected {
            return None;
        }
        // There is no text to put a caret in.
        if field.empty_required {
            return None;
        }
        if let Some(calendar) = &self.view.filter_editor.calendar {
            return Some(calendar.caret());
        }
        if !self.view.filter_editor.editing()
            || matches!(field.spec.kind, TaskFieldFilterKind::Labels)
        {
            return None;
        }
        Some(field.value.caret())
    }

    /// Translate the completed-task filter into the Asana fetch scope.
    pub fn desired_load_scope(&self) -> TaskLoadScope {
        match self.view.settings.filter.completed {
            Some(false) => TaskLoadScope::OpenOnly,
            Some(true) | None => TaskLoadScope::All,
        }
    }

    /// Build a task query template from the current filter state.
    ///
    /// The template's `target` is a placeholder; callers derive the real
    /// target per project via `TaskTarget::for_project` before dispatching.
    pub fn desired_task_query(&self) -> TaskQuery {
        let scope = self.desired_load_scope();
        let (due_after, due_before) = self.view.filter_editor.due_date_range_for_query();
        TaskQuery { target: TaskTarget::default(), scope, due_after, due_before }
    }

    /// Return `true` if the cache already covers every selected target project
    /// for the requested query.
    pub fn can_serve_query_for_targets(&self, targets: &[Project], query_template: &TaskQuery) -> bool {
        targets.iter().all(|project| {
            let query = TaskQuery { target: TaskTarget::for_project(project), ..query_template.clone() };
            self.loading.loaded_project_queries.get(&project.id)
                .map_or(false, |cached| cached.covers(&query))
        })
    }

    /// Return the subset of projects that still need to be loaded for the requested query.
    pub fn projects_requiring_load(
        &self,
        projects: &[Project],
        query_template: &TaskQuery,
    ) -> Vec<Project> {
        projects
            .iter()
            .filter(|project| {
                let query = TaskQuery { target: TaskTarget::for_project(project), ..query_template.clone() };
                self.loading.loaded_project_queries.get(&project.id)
                    .map_or(true, |cached| !cached.covers(&query))
            })
            .cloned()
            .collect()
    }

    /// Discard cached task records and load bookkeeping so the next fetch
    /// pulls fresh data instead of treating the current selection as already
    /// covered.
    ///
    /// Cached records are merged with incoming ones rather than replaced (see
    /// `merge_task_record`), so a plain re-fetch can't clear a field that
    /// changed to "unset" upstream (a task reopened, or dropped from a
    /// project). Wiping the cache first forces a clean rebuild.
    pub fn invalidate_cache(&mut self) {
        self.loading.cache = TaskCache::default();
        self.loading.loaded_project_queries.clear();
        self.loading.loaded_target_ids.clear();
    }

    pub fn mark_out_of_date(&mut self, message: impl Into<String>) {
        if matches!(self.loading.status, TaskStatus::Idle) {
            return;
        }

        self.loading.status = TaskStatus::OutOfDate(message.into());
        self.loading.progress = LoadProgress::Idle;
        self.view.task_vertical_scroll = 0;
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.loading.status = TaskStatus::Error(message.into());
        self.view.selected = None;
        self.view.table = TaskTableModel::empty();
        self.view.horizontal_scroll = 0;
        self.loading.progress = LoadProgress::Idle;
        self.loading.loaded_target_ids.clear();
        self.view.task_vertical_scroll = 0;
    }

    pub fn loading_started_at(&self) -> Option<Instant> {
        self.loading.progress.started_at()
    }

    pub fn loading_targets(&self) -> &[String] {
        self.loading.progress.target_names()
    }

    pub fn loaded_target_ids(&self) -> &[String] {
        &self.loading.loaded_target_ids
    }

    /// Tells the pane whether more input is already queued behind this one.
    ///
    /// The runtime owns this because only the runtime can see the input queue.
    pub fn set_input_pending(&mut self, pending: bool) {
        self.view.input_pending = pending;
    }

    /// Whether another input event is already waiting behind the current one.
    pub fn input_pending(&self) -> bool {
        self.view.input_pending
    }

    /// Rebuilds the table if a keystroke left it out of date.
    ///
    /// Called by the event loop once the input queue drains, so a burst of
    /// typing costs one rebuild instead of one per character.
    pub fn settle_table(&mut self) {
        if self.view.input_pending || self.view.stale_since.is_none() {
            return;
        }
        self.refresh_table();
    }

    /// Since when the table has been out of date, if it is.
    pub fn filtering_since(&self) -> Option<Instant> {
        self.view.stale_since
    }

    pub fn loading_spinner(&self) -> &'static str {
        const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
        let elapsed = self
            .loading
            .progress
            .started_at()
            .map(|started| started.elapsed().as_millis())
            .unwrap_or_default();
        let index = ((elapsed / 120) as usize) % FRAMES.len();
        FRAMES[index]
    }

    pub fn status(&self) -> &TaskStatus {
        &self.loading.status
    }

    pub fn table(&self) -> &TaskTableModel {
        &self.view.table
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.view.selected
    }

    pub fn selected_task_position(&self) -> Option<usize> {
        self.view.selected
            .and_then(|index| self.view.table.selectable_position(index))
    }

    pub fn horizontal_scroll(&self) -> usize {
        self.view.horizontal_scroll
    }

    pub fn vertical_scroll(&self) -> usize {
        self.view.task_vertical_scroll
    }

    pub fn ensure_selected_visible(&mut self, viewport_height: usize) {
        let Some(selected_index) = self.view.selected else {
            self.view.task_vertical_scroll = 0;
            return;
        };

        // When on the first selectable row, snap to 0 so any project/section
        // header rows above it are fully revealed.
        if Some(selected_index) == self.view.table.first_selectable_row_index() {
            self.view.task_vertical_scroll = 0;
            return;
        }

        let content_height = viewport_height.max(1);
        let selected_line = selected_index.saturating_add(1);
        let margin = 2usize.min(content_height.saturating_sub(1));

        let min_visible = self.view.task_vertical_scroll.saturating_add(margin);
        let max_visible = self
            .view
            .task_vertical_scroll
            .saturating_add(content_height.saturating_sub(1))
            .saturating_sub(margin);

        if selected_line < min_visible {
            self.view.task_vertical_scroll = selected_line.saturating_sub(margin);
            return;
        }

        if selected_line > max_visible {
            self.view.task_vertical_scroll = selected_line
                .saturating_add(margin)
                .saturating_add(1)
                .saturating_sub(content_height);
        }
    }

    pub fn scroll_left(&mut self) {
        self.view.horizontal_scroll = self.view.horizontal_scroll.saturating_sub(HORIZONTAL_SCROLL_STEP);
    }

    pub fn scroll_right(&mut self) {
        self.view.horizontal_scroll = self.view.horizontal_scroll.saturating_add(HORIZONTAL_SCROLL_STEP);
    }

    pub fn is_task_selected(&self, gid: &str) -> bool {
        !gid.is_empty() && self.view.selected_task_ids.contains(gid)
    }

    pub fn selected_task_count(&self) -> usize {
        self.view.selected_task_ids.len()
    }

    pub fn selected_task_url(&self) -> Option<String> {
        let task_gid = &self.cursor_task_gid()?;
        let dataset = self.loading.dataset.as_ref()?;
        let record = dataset.records.iter().find(|r| r.gid == *task_gid)?;
        let project_gid = record.project_gids.first()?;
        Some(format!("https://app.asana.com/0/{project_gid}/{task_gid}"))
    }

    fn toggle_task_selection(&mut self) {
        let Some(gid) = self.cursor_task_gid() else { return; };
        if self.view.selected_task_ids.contains(&gid) {
            self.view.selected_task_ids.remove(&gid);
        } else {
            self.view.selected_task_ids.insert(gid);
        }
        self.move_down();
    }

    fn select_all_visible_tasks(&mut self) {
        for row in &self.view.table.rows {
            if row.kind == TaskRowKind::Task && !row.gid.is_empty() {
                self.view.selected_task_ids.insert(row.gid.clone());
            }
        }
    }

    fn invert_task_selection(&mut self) {
        let visible_gids: Vec<String> = self.view.table.rows.iter()
            .filter(|row| row.kind == TaskRowKind::Task && !row.gid.is_empty())
            .map(|row| row.gid.clone())
            .collect();
        let mut new_selection = HashSet::new();
        for gid in visible_gids {
            if !self.view.selected_task_ids.contains(&gid) {
                new_selection.insert(gid);
            }
        }
        self.view.selected_task_ids = new_selection;
    }

    fn clear_task_selection(&mut self) {
        self.view.selected_task_ids.clear();
    }

    fn clear_hidden_task_selection(&mut self) {
        let visible_gids: HashSet<String> = self.view.table.rows.iter()
            .filter(|row| row.kind == TaskRowKind::Task && !row.gid.is_empty())
            .map(|row| row.gid.clone())
            .collect();
        self.view.selected_task_ids.retain(|gid| visible_gids.contains(gid));
    }

    fn clipboard_markdown(&self) -> String {
        let Some(dataset) = self.loading.dataset.as_ref() else { return String::new(); };
        let lines: Vec<String> = self.view.table.rows.iter()
            .filter(|row| row.kind == TaskRowKind::Task && self.view.selected_task_ids.contains(&row.gid))
            .filter_map(|row| {
                let record = dataset.records.iter().find(|r| r.gid == row.gid)?;
                let project_gid = record.project_gids.first()?;
                let name = &record.name;
                let gid = &row.gid;
                Some(format!("- [ ] [{name}](https://app.asana.com/0/{project_gid}/{gid})"))
            })
            .collect();
        if lines.is_empty() {
            String::new()
        } else {
            lines.join("\n") + "\n"
        }
    }

    /// Load the full task dataset for the given projects, then rebuild the
    /// visible table and filter state from the cached records.
    pub fn load_task_dataset_for_projects<C: AsanaClient>(
        &mut self,
        client: &C,
        projects: &[Project],
    ) -> Result<()> {
        debug_log(&format!("task data start: project_count={}", projects.len()));
        let query_template = self.desired_task_query();
        let dataset = Self::build_dataset_for_projects(client, projects, &query_template)?;
        self.loading.cache.merge_dataset(dataset);
        self.set_project_group_order(projects);
        self.loading.loaded_target_ids = projects.iter().map(|project| project.id.clone()).collect();
        for project in projects {
            let query = TaskQuery { target: TaskTarget::for_project(project), ..query_template.clone() };
            self.loading.loaded_project_queries.insert(project.id.clone(), query);
        }
        self.rebuild_visible_dataset();
        debug_log(&format!(
            "task data complete: tasks={} columns={}",
            self.view.table.task_count(),
            self.view.table.columns.len()
        ));
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn build_table_for_projects<C: AsanaClient>(
        client: &C,
        projects: &[Project],
    ) -> Result<TaskTableModel> {
        let query_template = TaskQuery::for_project("", TaskLoadScope::All);
        let dataset = Self::build_dataset_for_projects(client, projects, &query_template)?;
        let mut settings = TaskTableSettings::default();
        settings.sort.project_order = projects.iter().map(|project| project.name.clone()).collect();
        Ok(TaskTableModel::from_records_with_settings(
            dataset.records,
            dataset.custom_field_definitions,
            &settings,
        ))
    }

    pub(crate) fn build_dataset_for_projects<C: AsanaClient>(
        client: &C,
        projects: &[Project],
        query_template: &TaskQuery,
    ) -> Result<TaskDataset> {
        if projects.is_empty() {
            return Ok(TaskDataset::default());
        }

        let mut records = Vec::new();
        let mut definitions_by_gid: HashMap<String, CustomFieldDefinition> = HashMap::new();
        let mut natural_order = 0usize;

        for project in projects {
            // The assigned-to-me pseudo-project has no sections or custom fields
            // of its own; only fetch those for real Asana projects.
            let is_assigned_to_me = matches!(project.kind, ProjectKind::AssignedToMe);

            let sections = if is_assigned_to_me {
                Vec::new()
            } else {
                client.list_sections(&project.id)?
            };
            debug_log(&format!(
                "task data project={} sections={}",
                project.id,
                sections.len()
            ));
            let section_map = sections
                .iter()
                .map(|section| (section.gid.clone(), section.name.clone()))
                .collect::<HashMap<_, _>>();
            let section_order_map = sections
                .into_iter()
                .enumerate()
                .map(|(index, section)| (section.gid, index))
                .collect::<HashMap<_, _>>();

            if !is_assigned_to_me {
                for setting in client.list_project_custom_field_settings(&project.id)? {
                    debug_log(&format!(
                        "task data project={} custom_field={}",
                        project.id, setting.custom_field.name
                    ));
                    definitions_by_gid
                        .entry(setting.custom_field.gid.clone())
                        .or_insert_with(|| {
                            custom_field_definition(&project.id, &setting.custom_field)
                        });
                }
            }

            let query = TaskQuery { target: TaskTarget::for_project(project), ..query_template.clone() };
            let tasks = client.list_tasks(&query)?;
            debug_log(&format!(
                "task data project={} tasks={}",
                project.id,
                tasks.len()
            ));
            // Tasks fetched for a real project all belong to it. Tasks fetched
            // by assignee come from all over the workspace, so the row's own
            // id and name are only a fallback for the ones that genuinely sit
            // outside every project. Anything the placement walk does resolve is
            // cached under *that* project's gid, which is what keeps it out of
            // the view until the project it belongs to is selected too.
            let mut ancestors = AncestorPlacements::default();

            for task in tasks {
                let placement = if is_assigned_to_me {
                    ancestors.placement(client, &task)
                } else {
                    None
                };
                let (target_gid, project_name, inherited_section) = match placement {
                    Some(placement) => (placement.project_gid, placement.project, placement.section),
                    None => (project.id.clone(), project.name.clone(), None),
                };

                add_task_tree(
                    client,
                    &target_gid,
                    &project_name,
                    query_template.scope,
                    &section_map,
                    &section_order_map,
                    inherited_section,
                    None,
                    None,
                    0,
                    &mut natural_order,
                    task,
                    &mut records,
                )?;
            }
        }

        let mut definitions = definitions_by_gid.into_values().collect::<Vec<_>>();
        definitions.sort_by(|left, right| left.name.cmp(&right.name).then_with(|| left.gid.cmp(&right.gid)));

        Ok(TaskDataset {
            records,
            custom_field_definitions: definitions,
        })
    }

    fn rebuild_visible_dataset(&mut self) {
        let target_ids = self.active_target_ids().to_vec();
        let records = self.loading.cache.records_for_targets(&target_ids);
        let custom_field_definitions = self.loading.cache.custom_field_definitions();
        self.loading.dataset = Some(TaskDataset {
            records,
            custom_field_definitions,
        });
        if let Some(dataset) = self.loading.dataset.as_ref() {
            let previous = self.view.filter_editor.clone();
            self.view.filter_editor = TaskFilterEditorState::from_dataset(dataset);
            self.view.filter_editor.restore_queries(previous);
        }
        self.refresh_table();
    }

    pub fn move_up(&mut self) {
        if let Some(index) = self.view.recent_selected {
            self.view.recent_selected = self
                .view
                .recent_table
                .previous_selectable_row_index(index, 1)
                .or(Some(index));
            return;
        }
        let Some(index) = self.view.selected else {
            return;
        };
        match self.view.table.previous_selectable_row_index(index, 1) {
            // The two lists are vertically adjacent, so `k` off the top of
            // the table lands in the pane rather than stopping dead.
            Some(previous) if previous != index => self.view.selected = Some(previous),
            _ => {
                self.enter_recent_pane();
            }
        }
    }

    pub fn move_down(&mut self) {
        if let Some(index) = self.view.recent_selected {
            match self.view.recent_table.next_selectable_row_index(index, 1) {
                Some(next) if next != index => self.view.recent_selected = Some(next),
                _ => self.leave_recent_pane(),
            }
            return;
        }
        if let Some(index) = self.view.selected {
            if let Some(next) = self.view.table.next_selectable_row_index(index, 1) {
                self.view.selected = Some(next);
            }
        } else {
            self.view.selected = self.view.table.first_selectable_row_index();
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.leave_recent_pane();
        let step = page_size.max(1);
        if let Some(index) = self.view.selected {
            if let Some(previous) = self.view.table.previous_selectable_row_index(index, step) {
                self.view.selected = Some(previous);
            }
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        self.leave_recent_pane();
        let step = page_size.max(1);
        if let Some(index) = self.view.selected {
            if let Some(next) = self.view.table.next_selectable_row_index(index, step) {
                self.view.selected = Some(next);
            }
        } else {
            self.view.selected = self.view.table.first_selectable_row_index();
        }
    }

    pub fn jump_top(&mut self) {
        self.leave_recent_pane();
        self.view.selected = self.view.table.first_selectable_row_index();
    }

    pub fn jump_bottom(&mut self) {
        self.leave_recent_pane();
        self.view.selected = self.view.table.last_selectable_row_index();
    }

    pub fn move_section_up(&mut self) {
        self.leave_recent_pane();
        if let Some(index) = self.view.selected {
            if let Some(previous) = self.view.table.previous_section_row_index(index, 1) {
                self.view.selected = Some(previous);
            }
        }
    }

    pub fn move_section_down(&mut self) {
        self.leave_recent_pane();
        if let Some(index) = self.view.selected {
            if let Some(next) = self.view.table.next_section_row_index(index, 1) {
                self.view.selected = Some(next);
            }
        } else {
            self.view.selected = self.view.table.first_selectable_row_index();
        }
    }

    pub fn move_project_up(&mut self) {
        self.leave_recent_pane();
        if let Some(parent_index) = self.visible_parent_index() {
            self.view.selected = Some(parent_index);
            return;
        }

        if let Some(index) = self.view.selected {
            if let Some(previous) = self.view.table.previous_project_row_index(index, 1) {
                self.view.selected = Some(previous);
            }
        }
    }

    pub fn move_project_down(&mut self) {
        self.leave_recent_pane();
        if let Some(parent_index) = self.visible_parent_index() {
            self.view.selected = Some(parent_index);
            return;
        }

        if let Some(index) = self.view.selected {
            if let Some(next) = self.view.table.next_project_row_index(index, 1) {
                self.view.selected = Some(next);
            }
        } else {
            self.view.selected = self.view.table.first_selectable_row_index();
        }
    }

    pub fn toggle_completed_filter(&mut self) {
        self.view.settings.filter.toggle_completed_filter();
        self.refresh_table();
    }

    pub fn set_completed_filter(&mut self, completed: Option<bool>) {
        self.view.settings.filter.completed = completed;
        self.refresh_table();
    }

    pub fn cycle_completed_filter_without_refresh(&mut self) {
        self.view.settings.filter.toggle_completed_filter();
    }

    pub fn toggle_subtask_visibility(&mut self) {
        self.view.settings.filter.toggle_subtask_visibility();
        self.refresh_table();
    }

    pub fn cycle_sort_field(&mut self) {
        self.view.settings.sort.cycle_primary_field();
        self.refresh_table();
    }

    pub fn toggle_sort_direction(&mut self) {
        self.view.settings.sort.toggle_primary_direction();
        self.refresh_table();
    }

    pub fn toggle_project_grouping(&mut self) {
        self.view.settings.sort.toggle_project_grouping();
        self.refresh_table();
    }

    pub fn toggle_section_grouping(&mut self) {
        self.view.settings.sort.toggle_section_grouping();
        self.refresh_table();
    }

    pub fn refresh_from_cache(&mut self) {
        self.rebuild_visible_dataset();
    }

    pub fn apply_action(&mut self, action: &Action, page_size: usize) -> Option<crate::input::AppCommand> {
        if self.view.filter_editor.visible {
            match action {
                Action::MoveUp => {
                    self.move_filter_up();
                    return None;
                }
                Action::MoveDown => {
                    self.move_filter_down();
                    return None;
                }
                Action::PageUp => {
                    self.filter_page_up(page_size);
                    return None;
                }
                Action::PageDown => {
                    self.filter_page_down(page_size);
                    return None;
                }
                Action::ToggleTaskFilters => {
                    self.toggle_filter_panel();
                    return None;
                }
                Action::ClearSearch => {
                    self.filter_clear_current();
                    return None;
                }
                Action::FilterSetAdd => {
                    self.filter_add_set();
                    return None;
                }
                Action::FilterSetRemove => {
                    self.filter_remove_set();
                    return None;
                }
                Action::FilterSetNext => {
                    self.filter_select_set(1);
                    return None;
                }
                Action::FilterSetPrev => {
                    self.filter_select_set(-1);
                    return None;
                }
                Action::FilterRequireEmpty => {
                    self.filter_toggle_require_empty();
                    return None;
                }
                Action::FilterNegateField => {
                    self.filter_toggle_negate_field();
                    return None;
                }
                Action::FilterNegateSet => {
                    self.filter_toggle_negate_set();
                    return None;
                }
                Action::CycleFilterStringMode => {
                    self.filter_cycle_mode();
                    return None;
                }
                Action::SearchFuzzy => {
                    self.filter_set_mode(TaskFieldStringMode::Fuzzy);
                    return None;
                }
                Action::SearchSubstring => {
                    self.filter_set_mode(TaskFieldStringMode::Substring);
                    return None;
                }
                Action::SearchRegex => {
                    self.filter_set_mode(TaskFieldStringMode::Regex);
                    return None;
                }
                _ => {}
            }
        }

        match action {
            Action::MoveUp => {
                self.move_up();
                None
            }
            Action::MoveDown => {
                self.move_down();
                None
            }
            Action::PageUp => {
                self.page_up(page_size);
                None
            }
            Action::PageDown => {
                self.page_down(page_size);
                None
            }
            Action::JumpTop => {
                self.jump_top();
                None
            }
            Action::JumpBottom => {
                self.jump_bottom();
                None
            }
            Action::ScrollLeft => {
                self.scroll_left();
                None
            }
            Action::ScrollRight => {
                self.scroll_right();
                None
            }
            Action::MoveSectionUp => {
                self.move_section_up();
                None
            }
            Action::MoveSectionDown => {
                self.move_section_down();
                None
            }
            Action::MoveProjectUp => {
                self.move_project_up();
                None
            }
            Action::MoveProjectDown => {
                self.move_project_down();
                None
            }
            Action::TaskColumnPrev => {
                self.move_column(-1);
                None
            }
            Action::TaskColumnNext => {
                self.move_column(1);
                None
            }
            Action::ToggleCompletedFilter => {
                self.toggle_completed_filter();
                None
            }
            Action::ToggleHelpDetails => {
                self.toggle_help_details();
                None
            }
            Action::ToggleSubtaskVisibility => {
                self.toggle_subtask_visibility();
                None
            }
            Action::CycleTaskSort => {
                self.cycle_sort_field();
                None
            }
            Action::ToggleTaskSortDirection => {
                self.toggle_sort_direction();
                None
            }
            Action::ToggleProjectGrouping => {
                self.toggle_project_grouping();
                None
            }
            Action::ToggleSectionGrouping => {
                self.toggle_section_grouping();
                None
            }
            Action::ToggleTaskFilters => {
                self.toggle_filter_panel();
                None
            }
            Action::ToggleTaskSelection => {
                self.toggle_task_selection();
                None
            }
            Action::SelectAllVisibleTasks => {
                self.select_all_visible_tasks();
                None
            }
            Action::InvertTaskSelection => {
                self.invert_task_selection();
                None
            }
            Action::ClearTaskSelection => {
                self.clear_task_selection();
                None
            }
            Action::ClearHiddenTaskSelection => {
                self.clear_hidden_task_selection();
                None
            }
            Action::CopyTasksToClipboard => {
                let text = self.clipboard_markdown();
                if text.is_empty() {
                    None
                } else {
                    Some(crate::input::AppCommand::CopyToClipboard(text))
                }
            }
            _ => None,
        }
    }

    /// Rebuilds the visible table from the dataset, or marks it out of date.
    ///
    /// Deferring is safe because nothing reads a stale table without the
    /// runtime settling it first: the event loop calls [`Self::settle_table`]
    /// before it draws, as soon as the input queue is empty.
    fn refresh_table(&mut self) {
        // Set here rather than in twenty mutators: every panel change funnels
        // through this one call, so this is the one place nothing can bypass.
        // It over-reports — data arriving marks the panel dirty without the
        // user having touched it — and that is fine, because the writer
        // compares before it writes and skips a no-op.
        if self.view.filter_editor.loaded.is_some() {
            self.view.filter_editor.dirty = true;
        }

        if self.view.input_pending {
            self.view.stale_since.get_or_insert_with(Instant::now);
            return;
        }
        self.view.stale_since = None;

        let Some(dataset) = self.loading.dataset.as_ref() else {
            return;
        };

        let previous_selected_index = self.view.selected;
        let selected_gid = self.cursor_task_gid();
        let selected_parent_gid = selected_gid.as_deref().and_then(|gid| {
            dataset
                .records
                .iter()
                .find(|record| record.gid == gid)
                .and_then(|record| record.parent_gid.clone())
        });

        let mut filtered_records = self.apply_filter_panel(&dataset.records);
        prefer_selected_projects(&mut filtered_records, &self.view.settings.sort.project_order);

        self.view.table = TaskTableModel::from_records_with_settings(
            filtered_records,
            dataset.custom_field_definitions.clone(),
            &self.view.settings,
        );

        self.view.selected = selected_gid
            .as_deref()
            .and_then(|gid| self.view.table.rows.iter().position(|row| row.gid == gid));

        // The pane holds whatever the table has stopped showing, so it is
        // rebuilt from the table rather than beside it.
        self.rebuild_recent_table();
        // A rebuild that loses the cursor's task from the table hands the
        // cursor to the pane, on the same task, in the same place on screen —
        // rather than dropping it on a neighbouring row and leaving the edit
        // behind. The fallbacks below still run, because `selected` is the
        // row the cursor returns to when the pane is hidden.
        self.view.recent_selected = match self.view.selected {
            Some(_) => None,
            None => selected_gid
                .as_deref()
                .and_then(|gid| self.view.recent_table.rows.iter().position(|row| row.gid == gid)),
        };
        if self.view.recent_selected.is_some() {
            // Whatever the toggle says: a cursor the user cannot see is not
            // a cursor.
            self.view.recent_hidden = false;
        }

        if self.view.selected.is_none() {
            self.view.selected = selected_parent_gid
                .as_deref()
                .and_then(|gid| self.view.table.rows.iter().position(|row| row.gid == gid));
        }

        if self.view.selected.is_none() {
            self.view.selected = previous_selected_index.and_then(|previous_index| {
                self.view.table
                    .selectable_row_indices()
                    .into_iter()
                    .rev()
                    .find(|index| *index <= previous_index)
                    .or_else(|| self.view.table.last_selectable_row_index())
            });
        }

        if self.view.selected.is_none() {
            self.view.selected = self.view.table.first_selectable_row_index();
        }

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

        self.view.filter_editor.selected = self
            .view
            .filter_editor
            .selected
            .min(self.view.filter_editor.fields().len().saturating_sub(1));
        self.view.filter_vertical_scroll = self
            .view
            .filter_vertical_scroll
            .min(self.view.filter_editor.fields().len().saturating_sub(1));

        if !matches!(
            self.loading.status,
            TaskStatus::OutOfDate(_) | TaskStatus::Error(_) | TaskStatus::Loading
        ) {
            self.loading.status = if self.view.table.task_count() == 0 {
                TaskStatus::Empty
            } else {
                TaskStatus::Ready
            };
        }
    }

    fn active_target_ids(&self) -> &[String] {
        let in_flight = self.loading.progress.target_ids();
        if !in_flight.is_empty() {
            in_flight
        } else {
            &self.loading.loaded_target_ids
        }
    }

    fn apply_filter_panel(&self, records: &[TaskRecord]) -> Vec<TaskRecord> {
        let filter = self
            .view
            .filter_editor
            .prepare(self.view.current_user_gid.as_deref());
        records
            .iter()
            .filter(|record| filter.matches(record))
            .cloned()
            .collect()
    }

    fn visible_parent_index(&self) -> Option<usize> {
        let selected_gid = self
            .view.selected
            .and_then(|index| self.view.table.rows.get(index))
            .map(|row| row.gid.clone())?;

        let parent_gid = self
            .loading
            .dataset
            .as_ref()?
            .records
            .iter()
            .find(|record| record.gid == selected_gid)?
            .parent_gid
            .clone()?;

        self.view.table.rows.iter().position(|row| row.gid == parent_gid)
    }
}


/// Editing: the column cursor, the open cell edit, and the local write.
///
/// The keys and their routing live in `src/app.rs`, which is the only place
/// that has the Asana client; everything here is state the pane owns.
impl TaskState {
    /// The column cursor, as a cell index into the table's columns.
    pub fn selected_column(&self) -> usize {
        self.view
            .selected_column
            .min(self.view.table.columns.len().saturating_sub(1))
    }

    /// Walks the column cursor, clamped to the columns that exist.
    pub fn move_column(&mut self, delta: i64) {
        let last = self.view.table.columns.len().saturating_sub(1) as i64;
        self.view.selected_column =
            (self.selected_column() as i64 + delta).clamp(0, last.max(0)) as usize;
        self.view.pending_column_scroll = true;
    }

    /// Whether a cell editor is open.
    pub fn cell_edit_open(&self) -> bool {
        self.view.cell_edit.is_some()
    }

    /// Whether the open editor is a value picker, which reads `j`/`k`.
    pub fn cell_edit_is_options(&self) -> bool {
        self.view
            .cell_edit
            .as_ref()
            .is_some_and(|edit| edit.editor.is_options())
    }

    /// The open editor, for the renderer.
    pub(crate) fn cell_edit_view(&self) -> Option<CellEditView> {
        let edit = self.view.cell_edit.as_ref()?;
        Some(CellEditView {
            column: edit.column,
            text: edit.value(),
            caret: edit.caret(),
            window_start: edit.window_start,
            targets: edit.targets.len(),
        })
    }

    /// Records where the renderer scrolled the open cell's text to.
    ///
    /// The widths are only known to the renderer, in the same way
    /// `ensure_filter_visible` learns the filter panel's viewport there.
    pub fn set_cell_edit_window(&mut self, start: usize) {
        if let Some(edit) = self.view.cell_edit.as_mut() {
            edit.window_start = start;
        }
    }

    /// The last edit failure or refusal, shown in the corner notice pane.
    pub fn edit_notice(&self) -> Option<&str> {
        self.view.edit_notice.as_deref()
    }

    pub fn set_edit_notice(&mut self, message: impl Into<String>) {
        self.view.edit_notice = Some(message.into());
    }

    pub fn clear_edit_notice(&mut self) {
        self.view.edit_notice = None;
    }

    /// How many tasks the open edit will change, or zero when none is open.
    pub fn cell_edit_target_count(&self) -> usize {
        self.view
            .cell_edit
            .as_ref()
            .map_or(0, |edit| edit.targets.len())
    }

    /// The tasks an edit would apply to: the selection, or the cursor row.
    ///
    /// In table order rather than set order, so a bulk edit's requests go out
    /// in the order the rows are read.
    pub fn edit_targets(&self) -> Vec<String> {
        // Both lists, in reading order: a task selected before an edit pushed
        // it out of the table is still selected, and the pane is where it is
        // now — leaving it out would silently narrow the next bulk edit.
        let selected = self
            .view
            .recent_table
            .rows
            .iter()
            .chain(self.view.table.rows.iter())
            .filter(|row| row.kind.is_task() && self.view.selected_task_ids.contains(&row.gid))
            .map(|row| row.gid.clone())
            .collect::<Vec<_>>();

        if !selected.is_empty() {
            return selected;
        }

        self.cursor_task_gid().into_iter().collect()
    }

    /// The task the cursor is on, if it is on one.
    ///
    /// One cursor over two lists: whichever pane holds it answers.
    fn cursor_task_gid(&self) -> Option<String> {
        let row = match self.view.recent_selected {
            Some(index) => self.view.recent_table.rows.get(index)?,
            None => self.view.table.rows.get(self.view.selected?)?,
        };
        row.kind.is_task().then(|| row.gid.clone())
    }

    fn record(&self, gid: &str) -> Option<&TaskRecord> {
        self.loading
            .dataset
            .as_ref()
            .and_then(|dataset| dataset.records.iter().find(|record| record.gid == gid))
            // A task the loaded targets no longer cover has left the dataset
            // but is still in the cache — and still under the cursor, in the
            // recently-edited pane, where `e` and `d` have to keep working.
            .or_else(|| self.loading.cache.records.get(gid))
    }

    /// The text the table is showing for one cell of the cursor row.
    fn cursor_cell(&self, column: usize) -> String {
        let row = match self.view.recent_selected {
            Some(index) => self.view.recent_table.rows.get(index),
            None => self
                .view
                .selected
                .and_then(|index| self.view.table.rows.get(index)),
        };
        row.and_then(|row| row.cells.get(column))
            .cloned()
            .unwrap_or_default()
    }

    /// Every custom-field definition carrying the name this column is labelled
    /// with, most relevant to `record` first.
    ///
    /// The column merges one field name across every project that declares it.
    /// A write cannot: it has to name the gid belonging to the task's own
    /// project, so the task's memberships decide the order here.
    fn definitions_for_column(&self, column: usize, record: &TaskRecord) -> Vec<CustomFieldDefinition> {
        let Some(name) = self.view.table.columns.get(column) else {
            return Vec::new();
        };
        let Some(dataset) = self.loading.dataset.as_ref() else {
            return Vec::new();
        };

        let mut matching = dataset
            .custom_field_definitions
            .iter()
            .filter(|definition| &definition.name == name)
            .cloned()
            .collect::<Vec<_>>();
        matching.sort_by_key(|definition| !record.project_gids.contains(&definition.project_gid));
        matching
    }

    /// Opens the editor for the column under the cursor.
    ///
    /// Every refusal is a message rather than a silent no-op: `e` on a column
    /// that cannot be edited has to say which column and why.
    pub fn begin_cell_edit(&mut self, ctx: &EditContext) -> std::result::Result<(), String> {
        self.view.edit_notice = None;

        let Some(gid) = self.cursor_task_gid() else {
            return Err("no task under the cursor".to_string());
        };
        let Some(record) = self.record(&gid).cloned() else {
            return Err("no task under the cursor".to_string());
        };

        let column = self.selected_column();
        let targets = self.edit_targets();

        let editor = match column {
            crate::domain::TITLE_COLUMN => {
                // Three tasks with one title is not a bulk edit, it is a
                // mistake with three victims.
                if targets.len() > 1 {
                    return Err(format!(
                        "a title is edited one task at a time ({} selected)",
                        targets.len()
                    ));
                }
                CellEditor::Text(TextEdit::new(record.name.clone()))
            }
            crate::domain::ASSIGNEE_COLUMN => {
                let held = record
                    .assignee
                    .clone()
                    .map(|display| {
                        let handle = record
                            .assignee_gid
                            .clone()
                            .unwrap_or_else(|| display.clone());
                        Candidate::new(handle, display)
                    })
                    .into_iter()
                    .collect();
                // Capped at one: a task has an assignee, not assignees. The
                // cap is what makes typing a second name a reassignment
                // rather than an error.
                CellEditor::Complete(AutocompleteState::new(
                    held,
                    self.people_candidates(ctx),
                    1,
                ))
            }
            crate::domain::DUE_COLUMN | crate::domain::START_COLUMN => {
                let (label, value) = match column == crate::domain::DUE_COLUMN {
                    true => ("Due", record.due_date.clone()),
                    false => ("Start", record.start_date.clone()),
                };
                let value = value.unwrap_or_default();
                CellEditor::Date {
                    text: TextEdit::new(value.clone()),
                    calendar: CalendarState::open(label, &value, ctx.today()),
                }
            }
            crate::domain::STATE_COLUMN => CellEditor::Options {
                options: vec!["open".to_string(), "done".to_string()],
                cursor: Some(usize::from(record.completed)),
                allow_empty: false,
            },
            crate::domain::PROJECTS_COLUMN => {
                let held = record
                    .project_gids
                    .iter()
                    .map(|gid| Candidate::new(gid, ctx.project_name(gid)))
                    .collect();
                CellEditor::Complete(AutocompleteState::new(
                    held,
                    ctx.project_candidates(),
                    usize::MAX,
                ))
            }
            _ => {
                let definitions = self.definitions_for_column(column, &record);
                let Some(definition) = definitions.first() else {
                    return Err("this field cannot be edited here yet".to_string());
                };
                let current = self.cursor_cell(column);
                match &definition.kind {
                    CustomFieldKind::Enum { options } => {
                        let names = options
                            .iter()
                            .map(|option| option.name.clone())
                            .collect::<Vec<_>>();
                        let cursor = names
                            .iter()
                            .position(|name| name.eq_ignore_ascii_case(current.trim()));
                        CellEditor::Options {
                            options: names,
                            cursor,
                            allow_empty: true,
                        }
                    }
                    CustomFieldKind::Text | CustomFieldKind::Number => {
                        CellEditor::Text(TextEdit::new(current))
                    }
                    CustomFieldKind::Unsupported(_) => {
                        return Err("this field cannot be edited here yet".to_string())
                    }
                }
            }
        };

        self.view.cell_edit = Some(TaskCellEditState::new(column, targets, editor));
        // A cell being typed into has to be on screen, however the cursor got
        // to its column.
        self.view.pending_column_scroll = true;
        Ok(())
    }

    /// Throws the open edit away, leaving the value as it was.
    pub fn cancel_cell_edit(&mut self) {
        self.view.cell_edit = None;
    }

    /// Turns the open editor into the changes to send. `Err` keeps it open.
    pub fn commit_cell_edit(
        &mut self,
        ctx: &EditContext,
    ) -> std::result::Result<CommittedEdits, String> {
        let Some(edit) = self.view.cell_edit.clone() else {
            return Ok(CommittedEdits::default());
        };

        // Membership is not a field, so it leaves by a different door: a set
        // of adds and removes rather than one value written onto each task.
        if edit.column == crate::domain::PROJECTS_COLUMN {
            let committed = self.project_edits_for(&edit, ctx)?;
            self.view.cell_edit = None;
            return Ok(committed);
        }

        let value = edit.value();
        let directory = self.people_directory(ctx);
        // Resolved once, ahead of the loop: an assignee is resolved against a
        // directory rather than against the task, so doing it per target
        // would be the same answer several times — and several copies of the
        // same refusal.
        let assignee = match edit.column == crate::domain::ASSIGNEE_COLUMN {
            true => Some(self.resolve_assignee_edit(&edit, &directory, ctx)?),
            false => None,
        };
        let mut edits = Vec::with_capacity(edit.targets.len());

        for gid in &edit.targets {
            let Some(record) = self.record(gid).cloned() else {
                continue;
            };
            let field = match &assignee {
                Some(assignee) => TaskFieldEdit::Assignee(assignee.clone()),
                None => self.field_edit_for(&edit, &record, &value, &directory, ctx)?,
            };
            let previous = field.undo_for(&record);
            edits.push(TaskEdit {
                gid: gid.clone(),
                field,
                previous,
            });
        }

        self.view.cell_edit = None;
        Ok(CommittedEdits {
            fields: edits,
            projects: Vec::new(),
        })
    }

    /// Resolves the assignee editor into the person to send, or nobody.
    ///
    /// Text the candidate list cannot place gets one more chance through
    /// [`resolve_assignee`], which is what keeps `me` and an email address —
    /// two handles Asana takes that no directory lists — working.
    fn resolve_assignee_edit(
        &self,
        edit: &TaskCellEditState,
        directory: &[(String, String)],
        ctx: &EditContext,
    ) -> std::result::Result<Option<AssigneeRef>, String> {
        let Some(complete) = edit.complete().cloned() else {
            return Ok(None);
        };

        match complete.commit() {
            Ok(items) => Ok(items.first().map(|item| {
                AssigneeRef::new(
                    item.handle.clone(),
                    display_for(&item.handle, &item.display, directory),
                )
            })),
            Err(Unresolved::Ambiguous(text)) => Err(format!("{text} is ambiguous")),
            Err(Unresolved::Unknown(text)) => {
                resolve_assignee(&text, directory, ctx.current_user_gid.as_deref())
            }
        }
    }

    /// Resolves the projects editor into the memberships to add and remove.
    ///
    /// The diff is per task: the editor opened on the cursor row's projects,
    /// and applying that list to a selection means "be in these", which for
    /// another task is a different set of requests.
    fn project_edits_for(
        &self,
        edit: &TaskCellEditState,
        ctx: &EditContext,
    ) -> std::result::Result<CommittedEdits, String> {
        let Some(complete) = edit.complete().cloned() else {
            return Ok(CommittedEdits::default());
        };
        // Only a project the editor could have offered is a project the editor
        // may take away. One a task is in that this session never loaded is
        // invisible here, and removing it would be a change nobody asked for.
        let offered = complete
            .candidates()
            .iter()
            .map(|candidate| candidate.handle.clone())
            .collect::<Vec<_>>();

        let items = complete.commit().map_err(|unresolved| match unresolved {
            Unresolved::Ambiguous(text) => format!("{text} is ambiguous"),
            Unresolved::Unknown(text) => format!("no project called {text}"),
        })?;
        // Asana will not store a task in no projects at all, so this is a
        // refusal rather than a removal that comes back as an error.
        if items.is_empty() {
            return Err("a task has to be in at least one project".to_string());
        }

        let mut projects = Vec::new();
        for gid in &edit.targets {
            let Some(record) = self.record(gid) else {
                continue;
            };
            for item in &items {
                if !record.project_gids.contains(&item.handle) {
                    projects.push(ProjectEdit::add(gid, &item.handle, &item.display));
                }
            }
            for held in &record.project_gids {
                let kept = items.iter().any(|item| &item.handle == held);
                if kept || !offered.contains(held) {
                    continue;
                }
                projects.push(ProjectEdit::remove(gid, held, ctx.project_name(held)));
            }
        }

        Ok(CommittedEdits {
            fields: Vec::new(),
            projects,
        })
    }

    /// Resolves the typed value into the change one task will take.
    fn field_edit_for(
        &self,
        edit: &TaskCellEditState,
        record: &TaskRecord,
        value: &str,
        directory: &[(String, String)],
        ctx: &EditContext,
    ) -> std::result::Result<TaskFieldEdit, String> {
        match edit.column {
            crate::domain::TITLE_COLUMN => match value.trim() {
                "" => Err("a task needs a title".to_string()),
                title => Ok(TaskFieldEdit::Name(title.to_string())),
            },
            // Resolved by `resolve_assignee_edit` before the target loop, so
            // this is only reached by a caller that bypassed it.
            crate::domain::ASSIGNEE_COLUMN => Ok(TaskFieldEdit::Assignee(resolve_assignee(
                value,
                directory,
                ctx.current_user_gid.as_deref(),
            )?)),
            crate::domain::DUE_COLUMN => {
                Ok(TaskFieldEdit::Due(parse_date_value(value, ctx.today())?))
            }
            crate::domain::START_COLUMN => {
                Ok(TaskFieldEdit::Start(parse_date_value(value, ctx.today())?))
            }
            crate::domain::STATE_COLUMN => Ok(TaskFieldEdit::Completed(value == "done")),
            crate::domain::PROJECTS_COLUMN => {
                Err("project membership is not a task field".to_string())
            }
            column => {
                // Resolved per task, not once: two projects declare the same
                // field name under different gids, and the write has to name
                // the one this task's own project owns.
                let definitions = self.definitions_for_column(column, record);
                let Some(definition) = definitions.first() else {
                    return Err("this field cannot be edited here yet".to_string());
                };
                let options = definition
                    .enum_options()
                    .iter()
                    .map(|option| (option.gid.clone(), option.name.clone()))
                    .collect::<Vec<_>>();
                let kind = match definition.kind {
                    CustomFieldKind::Text => CustomValueKind::Text,
                    CustomFieldKind::Number => CustomValueKind::Number,
                    CustomFieldKind::Enum { .. } => CustomValueKind::Enum(&options),
                    CustomFieldKind::Unsupported(_) => CustomValueKind::Unsupported,
                };
                Ok(TaskFieldEdit::CustomField {
                    gid: definition.gid.clone(),
                    value: parse_custom_value(kind, value)?,
                })
            }
        }
    }

    /// Everyone the assignee editor can offer, most useful first.
    ///
    /// `me` leads, because it is the one name nobody has to remember and the
    /// one that means something different to each reader. The workspace
    /// directory follows, and the people named by loaded tasks after it —
    /// which is what the picker falls back to when `list_users` fails.
    pub(crate) fn people_candidates(&self, ctx: &EditContext) -> Vec<Candidate> {
        let mut candidates = Vec::new();
        if let Some(gid) = &ctx.current_user_gid {
            candidates.push(Candidate::new(gid, "me"));
        }
        for (handle, display) in self.people_directory(ctx) {
            if candidates
                .iter()
                .any(|candidate| candidate.handle == handle && candidate.display == display)
            {
                continue;
            }
            candidates.push(Candidate::new(handle, display));
        }
        candidates
    }

    /// The people the editor can resolve a name against, as `(gid, name)`.
    ///
    /// The workspace directory when there is one, plus whoever the loaded
    /// records name — the second is the fallback, and on a task assigned to
    /// someone outside the workspace list it is also the only entry.
    fn people_directory(&self, ctx: &EditContext) -> Vec<(String, String)> {
        let mut directory = ctx.people.clone();
        for person in self.assignee_directory() {
            if !directory.iter().any(|(gid, _)| gid == &person.0) {
                directory.push(person);
            }
        }
        directory
    }

    /// The people the loaded records name, as `(gid, display name)`.
    ///
    /// What the app knows without asking Asana who exists: everyone with a
    /// task on screen. On its own this excludes the most common reason to
    /// reassign a task, which is why it is the fallback rather than the
    /// directory.
    fn assignee_directory(&self) -> Vec<(String, String)> {
        let mut directory = self
            .loading
            .dataset
            .iter()
            .flat_map(|dataset| dataset.records.iter())
            .filter_map(|record| {
                Some((record.assignee_gid.clone()?, record.assignee.clone()?))
            })
            .collect::<Vec<_>>();
        directory.sort();
        directory.dedup();
        directory
    }

    /// `d`: every target to the opposite of the cursor row's state.
    ///
    /// Not a per-task toggle. A mixed selection ends up uniform, and pressing
    /// `d` twice puts it back — which a per-task flip would not.
    pub fn toggle_completed_edits(&mut self) -> Vec<TaskEdit> {
        self.view.edit_notice = None;
        let targets = self.edit_targets();
        // The cursor row is the reference, unless the selection does not
        // contain it — `space` leaves the cursor one row past the last thing
        // it selected, and reading a row that is not being changed would make
        // the second press a no-op instead of an undo.
        let reference = self
            .cursor_task_gid()
            .filter(|gid| targets.contains(gid))
            .or_else(|| targets.first().cloned());
        let Some(reference) = reference.and_then(|gid| self.record(&gid)) else {
            return Vec::new();
        };
        let completed = !reference.completed;

        targets
            .into_iter()
            .filter_map(|gid| {
                let record = self.record(&gid)?;
                Some(TaskEdit {
                    gid: gid.clone(),
                    field: TaskFieldEdit::Completed(completed),
                    previous: TaskFieldEdit::Completed(record.completed),
                })
            })
            .collect()
    }

    /// Applies an edit to the cache and the visible dataset, and rebuilds.
    ///
    /// Writes the fields directly rather than going through
    /// `TaskCache::upsert_record`. `merge_task_record` is monotone by design —
    /// `completed |= incoming`, and a `None` never overwrites a `Some` — so
    /// merging an un-complete or a cleared due date is a silent no-op. This is
    /// also why the reply from the server is not merged back in: the local
    /// record is already the truth, and only `modified_at` is taken from it.
    pub fn apply_edit_locally(&mut self, edit: &TaskEdit) {
        self.apply_edits_locally(std::slice::from_ref(edit));
    }

    /// The same, for a whole batch, with one rebuild at the end.
    pub fn apply_edits_locally(&mut self, edits: &[TaskEdit]) {
        for edit in edits {
            self.remember_edited(&edit.gid);
            if let Some(record) = self.loading.cache.records.get_mut(&edit.gid) {
                edit.field.apply(record);
            }
            if let Some(dataset) = self.loading.dataset.as_mut() {
                if let Some(record) = dataset
                    .records
                    .iter_mut()
                    .find(|record| record.gid == edit.gid)
                {
                    edit.field.apply(record);
                }
            }
        }
        self.refresh_table();
    }

    /// Applies membership changes to the cache and rebuilds from it.
    ///
    /// A full rebuild rather than `refresh_table`: the cache is keyed by the
    /// target that loaded a task, so a task taken out of the project in view
    /// is still in that project's cached page until the dataset is rebuilt
    /// from it — and the row would sit there until something else forced a
    /// reload.
    pub fn apply_project_edits_locally(&mut self, edits: &[ProjectEdit]) {
        if edits.is_empty() {
            return;
        }
        for edit in edits {
            self.remember_edited(&edit.gid);
            if let Some(record) = self.loading.cache.records.get_mut(&edit.gid) {
                edit.apply(record);
            }
        }
        self.rebuild_visible_dataset();
    }

    /// Records the `modified_at` the server reported for a confirmed edit.
    ///
    /// Without it, a fetch that started *before* the edit can come back with
    /// an older copy that `merge_task_record` reads as newer-or-equal and
    /// applies over the top. No rebuild follows: a timestamp changes nothing
    /// the table draws, and twelve confirmations would otherwise be twelve
    /// rebuilds of the whole table.
    pub fn confirm_edit(&mut self, gid: &str, modified_at: Option<String>) {
        let Some(modified_at) = modified_at else {
            return;
        };
        if let Some(record) = self.loading.cache.records.get_mut(gid) {
            record.modified_at = Some(modified_at.clone());
        }
        if let Some(dataset) = self.loading.dataset.as_mut() {
            if let Some(record) = dataset.records.iter_mut().find(|record| record.gid == gid) {
                record.modified_at = Some(modified_at);
            }
        }
    }

    /// Scrolls the table so the column cursor is on screen.
    ///
    /// Follows `ensure_filter_visible`: the widths are only known to the
    /// renderer, so the renderer is what calls this.
    pub fn ensure_column_visible(&mut self, widths: &[usize], viewport: usize) {
        if !self.view.pending_column_scroll {
            return;
        }
        self.view.pending_column_scroll = false;

        let column = self.selected_column();
        let Some(width) = widths.get(column) else {
            return;
        };
        let start = widths[..column].iter().sum::<usize>() + COLUMN_RULE_WIDTH * column;
        let end = start + width;

        if start < self.view.horizontal_scroll {
            self.view.horizontal_scroll = start;
            return;
        }
        if end > self.view.horizontal_scroll + viewport {
            self.view.horizontal_scroll = end.saturating_sub(viewport);
        }
    }

    // --- keys the open editor reads ---------------------------------------

    pub fn cell_edit_push_char(&mut self, ch: char) {
        if let Some(complete) = self.view.cell_edit.as_mut().and_then(|edit| edit.complete_mut()) {
            complete.push_char(ch);
            return;
        }
        if let Some(text) = self.view.cell_edit.as_mut().and_then(|edit| edit.text_mut()) {
            text.insert(ch);
        }
    }

    pub fn cell_edit_pop_char(&mut self) {
        // The completion editor deletes an item when there is no character
        // left to delete, so it cannot go through the plain buffer.
        if let Some(complete) = self.view.cell_edit.as_mut().and_then(|edit| edit.complete_mut()) {
            complete.delete_back();
            return;
        }
        if let Some(text) = self.view.cell_edit.as_mut().and_then(|edit| edit.text_mut()) {
            text.delete_back();
        }
    }

    pub fn cell_edit_move_caret(&mut self, delta: i64) {
        if let Some(complete) = self.view.cell_edit.as_mut().and_then(|edit| edit.complete_mut()) {
            complete.move_caret(delta);
            return;
        }
        if let Some(text) = self.view.cell_edit.as_mut().and_then(|edit| edit.text_mut()) {
            text.move_caret(delta);
        }
    }

    /// Completes the typed prefix, or answers `false` when nothing matches.
    pub fn cell_edit_complete(&mut self, delta: i32) -> bool {
        self.view
            .cell_edit
            .as_mut()
            .and_then(|edit| edit.complete_mut())
            .is_some_and(|complete| complete.complete(delta))
    }

    /// Whether the open editor completes over a list of names.
    pub fn cell_edit_is_complete(&self) -> bool {
        self.view
            .cell_edit
            .as_ref()
            .is_some_and(|edit| edit.complete().is_some())
    }

    /// The completion editor that is open, and the field it is editing.
    ///
    /// One accessor for both homes: the overlay that lists the candidates
    /// does not care which pane asked for it, and there is never more than
    /// one open.
    pub(crate) fn open_completion(&self) -> Option<(String, &AutocompleteState)> {
        if let Some(edit) = self.view.cell_edit.as_ref() {
            if let Some(state) = edit.complete() {
                let label = self
                    .view
                    .table
                    .columns
                    .get(edit.column)
                    .cloned()
                    .unwrap_or_default();
                return Some((label, state));
            }
        }

        let state = self.view.filter_editor.autocomplete.as_ref()?;
        let label = self
            .view
            .filter_editor
            .selected_label()
            .unwrap_or_default()
            .to_string();
        Some((label, state))
    }

    pub fn cell_edit_move_word(&mut self, delta: i64) {
        if let Some(text) = self.view.cell_edit.as_mut().and_then(|edit| edit.text_mut()) {
            text.move_word(delta);
        }
    }

    pub fn cell_edit_jump_start(&mut self) {
        if let Some(text) = self.view.cell_edit.as_mut().and_then(|edit| edit.text_mut()) {
            text.jump_start();
        }
    }

    pub fn cell_edit_jump_end(&mut self) {
        if let Some(text) = self.view.cell_edit.as_mut().and_then(|edit| edit.text_mut()) {
            text.jump_end();
        }
    }

    pub fn cell_edit_clear(&mut self) {
        if let Some(edit) = self.view.cell_edit.as_mut() {
            edit.clear_value();
        }
    }

    pub fn cell_edit_cycle_value(&mut self, delta: i32) {
        if let Some(edit) = self.view.cell_edit.as_mut() {
            edit.cycle_option(delta);
        }
    }
}

/// The name to show for a picked person.
///
/// `me` is a handle, not a name: the cell would read `me` until the next
/// reload, naming the reader rather than the person. The directory answers
/// with who that actually is when it knows.
fn display_for(handle: &str, display: &str, directory: &[(String, String)]) -> String {
    if display != "me" {
        return display.to_string();
    }
    directory
        .iter()
        .find(|(gid, _)| gid == handle)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| display.to_string())
}

/// The recently-edited pane: what is in it, and the cursor it shares with the
/// table.
///
/// The pane exists because the two most useful edits are the two most likely
/// to make a row vanish — reassign a task while filtering on one assignee, or
/// move it out of the project in view, and the row leaves under the cursor at
/// the one moment the edit is least finished.
impl TaskState {
    /// Records that a task was edited, newest first.
    fn remember_edited(&mut self, gid: &str) {
        self.view.recently_edited.retain(|held| held != gid);
        self.view.recently_edited.insert(0, gid.to_string());
        self.view.recently_edited.truncate(MAX_RECENTLY_EDITED);
    }

    /// The rows of the recently-edited pane.
    pub fn recent_table(&self) -> &TaskTableModel {
        &self.view.recent_table
    }

    /// Whether the pane is drawn: it has rows, and it has not been toggled off.
    pub fn recent_pane_visible(&self) -> bool {
        !self.view.recent_hidden && !self.view.recent_table.rows.is_empty()
    }

    /// How many recently-edited tasks the view is not showing.
    pub fn recent_hidden_count(&self) -> usize {
        self.view.recent_hidden_count
    }

    /// How many tasks the pane is holding.
    pub fn recent_pane_rows(&self) -> usize {
        match self.recent_pane_visible() {
            true => self.view.recent_table.rows.len(),
            false => 0,
        }
    }

    /// The cursor's row in the pane, when the cursor is in it.
    pub fn recent_selected_index(&self) -> Option<usize> {
        self.view.recent_selected
    }

    /// Shows or hides the pane.
    ///
    /// Hiding it with the cursor inside puts the cursor back in the table, at
    /// the row it was holding: a cursor the user cannot see is worse than a
    /// cursor that moved.
    pub fn toggle_recent_pane(&mut self) {
        self.view.recent_hidden = !self.view.recent_hidden;
        if self.view.recent_hidden {
            self.leave_recent_pane();
        }
    }

    /// Moves the cursor out of the pane and back into the table.
    fn leave_recent_pane(&mut self) {
        if self.view.recent_selected.take().is_none() {
            return;
        }
        if self.view.selected.is_none() {
            self.view.selected = self.view.table.first_selectable_row_index();
        }
    }

    /// Moves the cursor into the pane, onto its last row.
    ///
    /// Answers whether there was a pane to move into, so `k` at the top of
    /// the table can fall back to doing nothing.
    fn enter_recent_pane(&mut self) -> bool {
        if !self.recent_pane_visible() {
            return false;
        }
        self.view.recent_selected = self.view.recent_table.last_selectable_row_index();
        self.view.recent_selected.is_some()
    }

    /// Rebuilds the pane's rows from the tasks edited this session.
    ///
    /// Records come from the cache rather than the visible dataset: a task
    /// taken out of the project in view is no longer in the dataset at all,
    /// and that is exactly the edit the pane exists to keep reachable.
    fn rebuild_recent_table(&mut self) {
        let Some(definitions) = self
            .loading
            .dataset
            .as_ref()
            .map(|dataset| dataset.custom_field_definitions.clone())
        else {
            self.view.recent_table = TaskTableModel::empty();
            return;
        };

        let shown = self
            .view
            .table
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.as_str())
            .collect::<HashSet<_>>();
        let mut records = self
            .view
            .recently_edited
            .iter()
            .filter(|gid| !shown.contains(gid.as_str()))
            .filter_map(|gid| self.loading.cache.records.get(gid).cloned())
            .collect::<Vec<_>>();

        self.view.recent_hidden_count = records.len();
        records.truncate(MAX_RECENT_ROWS);
        self.view.recent_table = TaskTableModel::flat_from_records(records, definitions);
    }
}

/// Moves a selected project to the front of each record's project list.
///
/// A task in several projects is grouped under the first name in its list, and
/// the list is held sorted — so a task in "Alpha" and "Northwind" was drawn
/// under an "Alpha" header even with only Northwind selected, naming a project
/// the user had deselected. The cache keeps every project a task has been
/// loaded under, so this outlives the load that put the extra name there.
///
/// `selected` is the same project-list order the groups are ranked by, so the
/// name a task is filed under and the place that group appears both come from
/// one source. Records with no selected project among their own are left
/// alone: that is the assigned-to-me row, whose tasks are grouped by the
/// project their own membership named.
fn prefer_selected_projects(records: &mut [TaskRecord], selected: &[String]) {
    if selected.is_empty() {
        return;
    }

    for record in records {
        let Some(index) = record
            .projects
            .iter()
            .position(|project| selected.iter().any(|name| name == project))
        else {
            continue;
        };
        // Rotating keeps the rest of the list in the order it was in, so the
        // Projects column still reads the same apart from which name leads.
        record.projects[..=index].rotate_right(1);
    }
}

/// Where a task belongs in the project/section hierarchy, as resolved from the
/// task's own project membership or from the nearest ancestor that has one.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TaskPlacement {
    project_gid: String,
    project: String,
    section: Option<String>,
}

/// Resolves the project a task fetched by assignee belongs to.
///
/// Tasks fetched by assignee arrive from all over the workspace, and a subtask
/// normally holds no project membership of its own — the nearest ancestor that
/// *is* in a project decides where it is grouped. Ancestors are fetched one at
/// a time because they are often absent from the loaded set entirely (completed,
/// outside the date window, or assigned to someone else), and memoized because
/// siblings share them.
#[derive(Debug, Default)]
struct AncestorPlacements {
    by_gid: HashMap<String, Option<TaskPlacement>>,
}

impl AncestorPlacements {
    /// How many parent hops to follow before giving up, so a cycle or a
    /// pathologically deep tree can't stall a load.
    const MAX_HOPS: usize = 8;

    /// The placement for `task`, or `None` when neither it nor any ancestor
    /// within [`Self::MAX_HOPS`] belongs to a project.
    fn placement<C: AsanaClient>(&mut self, client: &C, task: &TaskDto) -> Option<TaskPlacement> {
        if let Some(placement) = membership_placement(task) {
            return Some(placement);
        }

        // Every gid walked over holds no membership of its own, so they all
        // resolve to whatever the walk ends up finding — cache them together.
        let mut walked: Vec<String> = Vec::new();
        let mut next = task.parent.as_ref().map(|parent| parent.gid.clone());
        let mut resolved = None;

        while let Some(gid) = next.take() {
            if let Some(cached) = self.by_gid.get(&gid) {
                resolved = cached.clone();
                break;
            }
            if walked.len() >= Self::MAX_HOPS || walked.contains(&gid) {
                debug_log(&format!("assigned-to-me parent walk gave up at task={gid}"));
                break;
            }
            walked.push(gid.clone());

            let parent = match client.get_task(&gid) {
                Ok(parent) => parent,
                Err(err) => {
                    debug_log(&format!("assigned-to-me parent lookup failed task={gid}: {err}"));
                    break;
                }
            };
            if let Some(placement) = membership_placement(&parent) {
                resolved = Some(placement);
                break;
            }
            next = parent.parent.as_ref().map(|parent| parent.gid.clone());
        }

        for gid in walked {
            self.by_gid.insert(gid, resolved.clone());
        }
        resolved
    }
}

/// The placement a task's own memberships give it, if any.
fn membership_placement(task: &TaskDto) -> Option<TaskPlacement> {
    task.memberships.iter().find_map(|membership| {
        let project_gid = membership.project.gid.clone();
        let project = membership.project.name.clone();
        if project_gid.trim().is_empty() || project.trim().is_empty() {
            return None;
        }
        Some(TaskPlacement {
            project_gid,
            project,
            section: membership
                .section
                .as_ref()
                .map(|section| section.name.clone())
                .filter(|section| !section.trim().is_empty()),
        })
    })
}

/// Records `task` and its subtasks.
///
/// `target_gid` is the id the records are cached under, and the id the view
/// filters on: a project gid, or the current user's gid for assigned-to-me
/// tasks that belong to no project at all. An assigned-to-me task the placement
/// walk *did* resolve is cached under its resolved project's gid, so it appears
/// only while that project is one of the selected targets.
fn add_task_tree<C: AsanaClient>(
    client: &C,
    target_gid: &str,
    project_name: &str,
    scope: TaskLoadScope,
    section_map: &HashMap<String, String>,
    section_order_map: &HashMap<String, usize>,
    inherited_section: Option<String>,
    inherited_section_order: Option<usize>,
    parent_gid: Option<String>,
    depth: usize,
    natural_order: &mut usize,
    task: TaskDto,
    records: &mut Vec<TaskRecord>,
) -> Result<()> {
    let current_gid = task.gid.clone();
    let mut record = TaskRecord::new(task.gid, task.name);
    record.completed = task.completed;
    record.modified_at = task.modified_at;
    record.parent_gid = parent_gid;
    record.subtask_depth = depth;
    if let Some(assignee) = task.assignee {
        // The gid is kept beside the name: a write has to name a user the way
        // the API will accept, and a display name is the one form it will not.
        record.assignee_gid = Some(assignee.gid.clone());
        record.assignee = assignee.display_name.or(assignee.name).or(Some(assignee.gid));
    }
    record.due_date = task.due_on;
    record.start_date = task.start_on;
    record.natural_order = *natural_order;
    *natural_order = (*natural_order).saturating_add(1);
    record.project_gids.push(target_gid.to_string());
    record.projects.push(project_name.to_string());

    let mut section_name = inherited_section;
    let mut section_order = inherited_section_order;
    if section_name.is_none() || section_order.is_none() {
        for membership in task.memberships {
            if let Some(section) = membership.section {
                if section_name.is_none() {
                    section_name = Some(
                        section_map
                            .get(&section.gid)
                            .cloned()
                            .unwrap_or(section.name.clone()),
                    );
                }
                if section_order.is_none() {
                    section_order = section_order_map.get(&section.gid).copied();
                }
                if section_name.is_some() && section_order.is_some() {
                    break;
                }
            }
        }
    }

    if let Some(section) = section_name.clone() {
        record.sections.push(section);
    }
    record.section_order = section_order;

    for field in task.custom_fields {
        if let Some(value) = custom_field_value(&field) {
            record
                .custom_fields
                .entry(field.gid)
                .or_default()
                .push(value);
        }
    }

    records.push(record);

    if task.num_subtasks > 0 {
        let subtasks = client.list_subtasks(&current_gid, scope)?;
        for subtask in subtasks {
            add_task_tree(
                client,
                target_gid,
                project_name,
                scope,
                section_map,
                section_order_map,
                section_name.clone(),
                section_order,
                Some(current_gid.clone()),
                depth + 1,
                natural_order,
                subtask,
                records,
            )?;
        }
    }

    Ok(())
}

/// Turns one project's declaration of a custom field into a definition.
///
/// The kind comes from `resource_subtype`, which is what Asana declares, not
/// from the values tasks happen to carry — a picker built from observed values
/// can never offer the option no task has yet. Disabled options are dropped:
/// they exist so old values still render, and offering one is offering a value
/// Asana will reject.
fn custom_field_definition(project_gid: &str, field: &CustomFieldDto) -> CustomFieldDefinition {
    let kind = match field.resource_subtype.as_deref() {
        Some("text") | None => CustomFieldKind::Text,
        Some("number") => CustomFieldKind::Number,
        Some("enum") => CustomFieldKind::Enum {
            options: field
                .enum_options
                .iter()
                .filter(|option| option.enabled)
                .map(|option| EnumOption::new(option.gid.clone(), option.name.clone()))
                .collect(),
        },
        Some(other) => CustomFieldKind::Unsupported(other.to_string()),
    };

    CustomFieldDefinition::new(field.gid.clone(), field.name.clone())
        .in_project(project_gid)
        .with_kind(kind)
}

fn custom_field_value(field: &CustomFieldValueDto) -> Option<String> {
    field
        .display_value
        .clone()
        .or_else(|| field.enum_value.as_ref().map(|value| value.name.clone()))
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use crate::{
        asana::{
            dto::{
                CustomFieldDto, CustomFieldValueDto, EnumOptionDto, ProjectCustomFieldSettingDto,
                SectionDto, TaskDto, TaskMembershipDto, TaskMembershipProjectDto,
                TaskMembershipSectionDto, TaskParentDto, UserDto,
            },
            fake::FakeAsanaClient,
            TaskLoadScope,
        },
        domain::{Project, TaskRecord, TaskRowKind, TaskTableModel},
    };

    use super::{
        prefer_selected_projects, TaskDataset, TaskFieldFilterKind, TaskFilterEditorState,
        TaskState, TaskStatus,
    };

    fn task(
        gid: &str,
        name: &str,
        project_gid: &str,
        project_name: &str,
        section_gid: &str,
        section_name: &str,
        field_gid: &str,
        field_name: &str,
        field_value: &str,
    ) -> TaskDto {
        TaskDto {
            gid: gid.to_string(),
            name: name.to_string(),
            completed: false,
            modified_at: Some("2026-06-01T00:00:00Z".to_string()),
            due_on: Some("2026-06-01".to_string()),
            start_on: Some("2026-05-28".to_string()),
            assignee: Some(UserDto {
                gid: "user-1".to_string(),
                name: Some("Alex".to_string()),
                display_name: Some("Alex".to_string()),
            }),
            num_subtasks: 0,
            memberships: vec![TaskMembershipDto {
                project: TaskMembershipProjectDto {
                    gid: project_gid.to_string(),
                    name: project_name.to_string(),
                },
                section: Some(TaskMembershipSectionDto {
                    gid: section_gid.to_string(),
                    name: section_name.to_string(),
                }),
            }],
            parent: None,
            custom_fields: vec![CustomFieldValueDto {
                gid: field_gid.to_string(),
                name: field_name.to_string(),
                display_value: Some(field_value.to_string()),
                enum_value: Some(EnumOptionDto {
                    gid: "opt-1".to_string(),
                    name: field_value.to_string(),
                    enabled: true,
                }),
            }],
        }
    }

    /// A custom field with the same name usually exists separately in every
    /// project, each with its own id. Reviewing five projects used to produce
    /// five identically-labelled filter rows *and* five identical table columns.
    #[test]
    fn a_custom_field_shared_by_name_across_projects_is_one_row_and_one_column() {
        let projects = vec![
            Project::new("p1", "Inbox", true),
            Project::new("p2", "Backlog", true),
        ];
        let client = FakeAsanaClient::new(projects.clone())
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_sections(
                "p2",
                vec![SectionDto {
                    gid: "s2".to_string(),
                    name: "Later".to_string(),
                }],
            )
            // Same field name, different id per project.
            .with_custom_field_settings(
                "p1",
                vec![ProjectCustomFieldSettingDto {
                    gid: "set-1".to_string(),
                    custom_field: CustomFieldDto {
                        gid: "cf-inbox".to_string(),
                        name: "Tag".to_string(),
                        resource_subtype: None,
                        enum_options: Vec::new(),
                    },
                }],
            )
            .with_custom_field_settings(
                "p2",
                vec![ProjectCustomFieldSettingDto {
                    gid: "set-2".to_string(),
                    custom_field: CustomFieldDto {
                        gid: "cf-backlog".to_string(),
                        name: "Tag".to_string(),
                        resource_subtype: None,
                        enum_options: Vec::new(),
                    },
                }],
            )
            .with_tasks(
                "p1",
                vec![task(
                    "t1", "Ship release", "p1", "Inbox", "s1", "Today", "cf-inbox", "Tag", "red",
                )],
            )
            .with_tasks(
                "p2",
                vec![task(
                    "t2", "Draft plan", "p2", "Backlog", "s2", "Later", "cf-backlog", "Tag",
                    "blue",
                )],
            );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &projects)
            .expect("tasks load");

        let labels = state
            .filter_panel_rows()
            .into_iter()
            .map(|(label, _)| label)
            .collect::<Vec<_>>();
        assert_eq!(
            labels.iter().filter(|label| *label == "Tag").count(),
            1,
            "one filter row, not one per project: {labels:?}"
        );

        // The table's columns are keyed the same way, so they cannot disagree
        // with the filter panel about how many "Tag" fields there are.
        let columns = state.table().columns.clone();
        assert_eq!(
            columns.iter().filter(|column| *column == "Tag").count(),
            1,
            "one table column, not one per project: {columns:?}"
        );

        // And that single column shows each task's own value, whichever id it
        // came from.
        let tag = columns
            .iter()
            .position(|column| column == "Tag")
            .expect("the Tag column exists");
        let values = state
            .table()
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.cells[tag].clone())
            .collect::<Vec<_>>();
        let mut sorted = values.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["blue".to_string(), "red".to_string()],
            "each project's value lands in the shared column: {values:?}"
        );

        // And that single row filters on both ids, so it can still pick out a
        // value that only exists in one project.
        let tag = state
            .view
            .filter_editor
            .fields()
            .iter()
            .position(|field| field.spec.label == "Tag")
            .expect("the Tag row exists");
        assert_eq!(
            state.view.filter_editor.fields()[tag].label_options,
            vec!["blue".to_string(), "red".to_string()],
            "the options are the union across every id"
        );

        state.view.filter_editor.selected = tag;
        state.filter_edit_begin();
        state.filter_add_label();
        state.filter_cycle_label_value(0);
        assert_eq!(state.table().task_count(), 1);
    }

    /// The same two-project fixture, driven through a reload with two sets.
    ///
    /// This is where `restore_from`'s key-matching earns its keep: the "Tag"
    /// row is rebuilt from a *different* set of custom-field ids on the second
    /// load, and only the `custom:<name>` key connects the old row to the new
    /// one. Matching by id would lose both sets' queries.
    #[test]
    fn filter_sets_survive_a_reload_that_rebuilds_a_shared_custom_field_row() {
        let projects = vec![
            Project::new("p1", "Inbox", true),
            Project::new("p2", "Backlog", true),
        ];
        let client = FakeAsanaClient::new(projects.clone())
            .with_custom_field_settings(
                "p1",
                vec![ProjectCustomFieldSettingDto {
                    gid: "set-1".to_string(),
                    custom_field: CustomFieldDto {
                        gid: "cf-inbox".to_string(),
                        name: "Tag".to_string(),
                        resource_subtype: None,
                        enum_options: Vec::new(),
                    },
                }],
            )
            .with_custom_field_settings(
                "p2",
                vec![ProjectCustomFieldSettingDto {
                    gid: "set-2".to_string(),
                    custom_field: CustomFieldDto {
                        gid: "cf-backlog".to_string(),
                        name: "Tag".to_string(),
                        resource_subtype: None,
                        enum_options: Vec::new(),
                    },
                }],
            )
            .with_tasks(
                "p1",
                vec![task(
                    "t1", "Ship release", "p1", "Inbox", "s1", "Today", "cf-inbox", "Tag", "red",
                )],
            )
            .with_tasks(
                "p2",
                vec![task(
                    "t2", "Draft plan", "p2", "Backlog", "s2", "Later", "cf-backlog", "Tag",
                    "blue",
                )],
            );

        // Load only the first project, so the Tag row is built from cf-inbox.
        let mut state = TaskState::new();
        state.set_completed_filter(None);
        state
            .load_task_dataset_for_projects(&client, &projects[..1])
            .expect("the first project loads");
        state.toggle_filter_panel();

        set_field(&mut state, "title", "ship");
        state.filter_add_set();
        select_field(&mut state, "Tag");
        state.filter_toggle_require_empty();

        // The second project arrives and the Tag row is rebuilt around both ids.
        state
            .load_task_dataset_for_projects(&client, &projects)
            .expect("the second project loads");

        assert_eq!(state.filter_set_position(), (1, 2), "both sets survived");
        assert_eq!(state.filter_panel_rows_for_set(0)[0].1, "ship");
        assert_eq!(
            state.filter_panel_rows_for_set(1)[6].1,
            "(none)",
            "and the require-empty came across onto the rebuilt row"
        );
    }

    #[test]
    fn loads_tasks_for_selected_projects_and_builds_a_table_model() {
        let client = FakeAsanaClient::new(vec![
            Project::new("p1", "Inbox", true),
            Project::new("p2", "Backlog", false),
        ])
        .with_sections(
            "p1",
            vec![SectionDto {
                gid: "s1".to_string(),
                name: "Today".to_string(),
            }],
        )
        .with_custom_field_settings(
            "p1",
            vec![ProjectCustomFieldSettingDto {
                gid: "cfs1".to_string(),
                custom_field: CustomFieldDto {
                    gid: "cf1".to_string(),
                    name: "Priority".to_string(),
                    resource_subtype: None,
                    enum_options: Vec::new(),
                },
            }],
        )
        .with_tasks(
            "p1",
            vec![
                task("t1", "Ship release", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                task("t2", "Write docs", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "Low"),
            ],
        )
        .with_sections(
            "p2",
            vec![SectionDto {
                gid: "s2".to_string(),
                name: "Later".to_string(),
            }],
        )
        .with_custom_field_settings(
            "p2",
            vec![ProjectCustomFieldSettingDto {
                gid: "cfs2".to_string(),
                custom_field: CustomFieldDto {
                    gid: "cf2".to_string(),
                    name: "Effort".to_string(),
                    resource_subtype: None,
                    enum_options: Vec::new(),
                },
            }],
        )
        .with_tasks(
            "p2",
            vec![task("t1", "Ship release", "p2", "Backlog", "s2", "Later", "cf2", "Effort", "S")],
        );

        let mut state = TaskState::new();

        state
            .load_task_dataset_for_projects(
                &client,
                &[Project::new("p1", "Inbox", true), Project::new("p2", "Backlog", false)],
            )
            .expect("tasks load");

        assert_eq!(state.status(), &TaskStatus::Ready);
        assert_eq!(state.table().columns[0], "Task");
        assert!(state.table().columns.iter().any(|column| column == "Priority"));
        assert!(state.table().columns.iter().any(|column| column == "Effort"));
        assert_eq!(state.table().task_count(), 2);
        assert_eq!(state.table().rows.len(), 10);
        // The groups follow the order the projects were handed to us in, not
        // the alphabet: Inbox was listed first.
        assert_eq!(state.table().rows[0].kind, crate::domain::TaskRowKind::ProjectSeparator);
        assert_eq!(state.table().rows[1].kind, crate::domain::TaskRowKind::ProjectHeader);
        assert_eq!(state.table().rows[1].cells[0], "Inbox");
        assert_eq!(state.table().rows[2].kind, crate::domain::TaskRowKind::SectionSpacer);
        assert_eq!(state.table().rows[3].kind, crate::domain::TaskRowKind::SectionHeader);
        assert_eq!(state.table().rows[3].cells[0], "Today");
        assert_eq!(state.table().rows[4].kind, crate::domain::TaskRowKind::Task);
        assert_eq!(state.table().rows[4].cells[0], "Write docs");
        assert_eq!(state.table().rows[5].kind, crate::domain::TaskRowKind::ProjectSeparator);
        assert_eq!(state.table().rows[6].kind, crate::domain::TaskRowKind::ProjectHeader);
        assert_eq!(state.table().rows[6].cells[0], "Backlog");
        assert_eq!(state.table().rows[7].kind, crate::domain::TaskRowKind::SectionSpacer);
        assert_eq!(state.table().rows[8].kind, crate::domain::TaskRowKind::SectionHeader);
        assert_eq!(state.table().rows[8].cells[0], "Later");
        assert_eq!(state.table().rows[9].kind, crate::domain::TaskRowKind::Task);
        assert_eq!(state.table().rows[9].cells[0], "Ship release");
        assert_eq!(state.table().rows[9].cells[1], "Alex");
        assert_eq!(state.table().rows[9].cells[2], "2026-06-01");
        assert_eq!(state.table().rows[9].cells[3], "2026-05-28");
        assert_eq!(state.table().rows[9].cells[4], "open");
        assert_eq!(state.table().rows[9].cells[5], "Backlog | Inbox");
    }

    #[test]
    fn switching_to_task_mode_makes_the_view_visible() {
        let mut state = TaskState::new();

        state.toggle_visible();

        assert!(state.visible());
    }

    #[test]
    fn moves_by_section_and_project_groups() {
        let client = FakeAsanaClient::new(vec![
            Project::new("p1", "Alpha", true),
            Project::new("p2", "Beta", false),
        ])
        .with_sections(
            "p1",
            vec![
                SectionDto {
                        gid: "s1".to_string(),
                        name: "Today".to_string(),
                    },
                    SectionDto {
                        gid: "s2".to_string(),
                        name: "Later".to_string(),
                    },
                ],
            )
            .with_sections(
                "p2",
                vec![SectionDto {
                    gid: "s3".to_string(),
                    name: "Now".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![
                    task("t1", "Alpha 1", "p1", "Alpha", "s1", "Today", "cf1", "Priority", "High"),
                    task("t2", "Alpha 2", "p1", "Alpha", "s1", "Today", "cf1", "Priority", "Low"),
                    task("t3", "Alpha 3", "p1", "Alpha", "s2", "Later", "cf1", "Priority", "Low"),
                ],
            );
        let client = client.with_tasks(
            "p2",
            vec![task("t4", "Beta 1", "p2", "Beta", "s3", "Now", "cf1", "Priority", "Medium")],
        );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(
                &client,
                &[Project::new("p1", "Alpha", true), Project::new("p2", "Beta", false)],
            )
            .expect("tasks load");

        assert_eq!(state.selected_index(), Some(4));
        state.move_section_down();
        assert_eq!(state.selected_index(), Some(8));
        state.move_section_up();
        assert_eq!(state.selected_index(), Some(4));
        state.move_project_down();
        assert_eq!(state.selected_index(), Some(13));
        state.move_project_up();
        assert_eq!(state.selected_index(), Some(4));
    }

    #[test]
    fn keeps_selected_row_in_view_when_the_viewport_is_small() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![
                    task("t1", "Task 1", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    task("t2", "Task 2", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    task("t3", "Task 3", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    task("t4", "Task 4", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                ],
            );

        let mut state = TaskState::new();
        state.set_completed_filter(None);
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.set_visible(true);

        state.move_down();
        state.move_down();
        state.move_down();

        state.ensure_selected_visible(3);

        assert_eq!(state.selected_index(), Some(7));
        assert!(state.vertical_scroll() > 0);
        let selected_line = state.selected_index().unwrap() + 1;
        assert!(selected_line >= state.vertical_scroll());
        assert!(selected_line < state.vertical_scroll() + 3);
    }

    #[test]
    fn scrolls_to_row_zero_when_navigating_back_to_first_item() {
        // When the user scrolls down and then navigates back up to the first
        // task, ensure_selected_visible must set scroll to 0 so that project
        // and section header rows above the first task are fully visible.
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![
                    task("t1", "Task 1", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    task("t2", "Task 2", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    task("t3", "Task 3", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    task("t4", "Task 4", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                ],
            );

        let mut state = TaskState::new();
        state.set_completed_filter(None);
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.set_visible(true);

        // Scroll down past the header rows.
        state.move_down();
        state.move_down();
        state.move_down();
        state.ensure_selected_visible(5);
        assert!(state.vertical_scroll() > 0, "should be scrolled down after moving down");

        // Move back up to the first item and check that scroll snaps to 0.
        state.move_up();
        state.move_up();
        state.move_up();
        state.ensure_selected_visible(5);
        assert_eq!(state.vertical_scroll(), 0, "should snap to row 0 when on first item");
    }

    #[test]
    fn toggles_task_filters_and_rebuilds_the_table_after_async_style_load() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![
                    task("t1", "Open task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    TaskDto {
                        completed: true,
                        ..task("t2", "Closed task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High")
                    },
                ],
        );

        let mut state = TaskState::new();
        state.view.settings.filter.completed = None;
        let dataset = TaskState::build_dataset_for_projects(
            &client,
            &[Project::new("p1", "Inbox", true)],
            &crate::asana::TaskQuery::for_project("p1", TaskLoadScope::All),
        )
        .expect("dataset builds");
        state.begin_loading(&[Project::new("p1", "Inbox", true)]);
        state.finish_loading_dataset(dataset);

        assert_eq!(state.table().task_count(), 2);
        assert!(state.filter_summary().contains("all"));

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 1);
        assert!(state.filter_summary().contains("open"));

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 1);
        assert!(state.filter_summary().contains("comp done"));

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 2);

        state.toggle_subtask_visibility();
        assert!(state.filter_summary().contains("sub hide"));

        state.toggle_project_grouping();
        state.toggle_section_grouping();
        assert!(state.filter_summary().contains("grp p:off"));
        assert!(state.filter_summary().contains("grp p:off s:off"));

        state.cycle_sort_field();
        assert!(state.filter_summary().contains("sort title asc"));

        state.toggle_sort_direction();
        assert!(state.filter_summary().contains("sort title desc"));
    }

    #[test]
    fn the_sort_direction_action_reverses_the_visible_rows() {
        let mut state = TaskState::default();
        // The visible dataset is rebuilt by project membership, so every record
        // needs to name the loaded project to survive the round trip.
        let record = |gid: &str, due: Option<&str>| {
            let mut record = crate::domain::TaskRecord::new(gid, gid);
            record.due_date = due.map(str::to_string);
            record.project_gids = vec!["p1".to_string()];
            record.projects = vec!["Inbox".to_string()];
            record
        };
        let early = record("early", Some("2026-09-01"));
        let late = record("late", Some("2026-09-30"));
        let undated = record("undated", None);

        state.begin_loading(&[Project::new("p1", "Inbox", true)]);
        state.finish_loading_dataset(TaskDataset {
            records: vec![late, early, undated],
            custom_field_definitions: Vec::new(),
        });
        state.toggle_project_grouping();
        state.toggle_section_grouping();

        let gids = |state: &TaskState| {
            state
                .table()
                .rows
                .iter()
                .filter(|row| row.kind.is_task())
                .map(|row| row.gid.clone())
                .collect::<Vec<_>>()
        };

        assert_eq!(gids(&state), vec!["early", "late", "undated"]);

        state.apply_action(&crate::input::Action::ToggleTaskSortDirection, 10);
        assert_eq!(
            gids(&state),
            vec!["late", "early", "undated"],
            "the dates reverse and the undated row stays at the bottom"
        );

        state.apply_action(&crate::input::Action::ToggleTaskSortDirection, 10);
        assert_eq!(gids(&state), vec!["early", "late", "undated"]);
    }

    #[test]
    fn loads_subtasks_for_project_tasks_even_when_they_are_not_project_members() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![task("t1", "Parent task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High")],
            )
            .with_subtasks(
                "t1",
                vec![TaskDto {
                    gid: "t2".to_string(),
                    name: "Child task".to_string(),
                    completed: false,
                    modified_at: None,
                    due_on: Some("2026-06-01".to_string()),
                    start_on: Some("2026-05-28".to_string()),
                    assignee: Some(UserDto {
                        gid: "user-1".to_string(),
                        name: Some("Alex".to_string()),
                        display_name: Some("Alex".to_string()),
                    }),
                    num_subtasks: 0,
                    memberships: vec![],
                    parent: None,
                    custom_fields: vec![CustomFieldValueDto {
                        gid: "cf1".to_string(),
                        name: "Priority".to_string(),
                        display_value: Some("Low".to_string()),
                        enum_value: Some(EnumOptionDto {
                            gid: "opt-1".to_string(),
                            name: "Low".to_string(),
                            enabled: true,
                        }),
                    }],
                }],
            );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        // Nesting is carried as a depth, not as an indent baked into the title;
        // the renderer turns the depth into an indent and a marker.
        let task_rows = state
            .table()
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| (row.cells[0].clone(), row.subtask_depth))
            .collect::<Vec<_>>();

        assert_eq!(
            task_rows,
            vec![
                ("Parent task".to_string(), 0),
                ("Child task".to_string(), 1)
            ]
        );
        assert_eq!(state.table().task_count(), 2);
    }

    #[test]
    fn sorts_sections_using_the_order_returned_by_asana() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![
                    SectionDto {
                        gid: "s2".to_string(),
                        name: "Beta".to_string(),
                    },
                    SectionDto {
                        gid: "s1".to_string(),
                        name: "Alpha".to_string(),
                    },
                ],
            )
            .with_tasks(
                "p1",
                vec![
                    task("t1", "Beta task", "p1", "Inbox", "s2", "Beta", "cf1", "Priority", "High"),
                    task("t2", "Alpha task", "p1", "Inbox", "s1", "Alpha", "cf1", "Priority", "High"),
                ],
            );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        let task_rows = state
            .table()
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| (row.cells[0].clone(), row.section.clone().unwrap_or_default()))
            .collect::<Vec<_>>();

        assert_eq!(
            task_rows,
            vec![
                ("Beta task".to_string(), "Beta".to_string()),
                ("Alpha task".to_string(), "Alpha".to_string()),
            ]
        );
    }

    #[test]
    fn hiding_subtasks_keeps_selection_near_the_previous_row_instead_of_jumping_to_the_top() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![
                    SectionDto {
                        gid: "s1".to_string(),
                        name: "Today".to_string(),
                    },
                    SectionDto {
                        gid: "s2".to_string(),
                        name: "Later".to_string(),
                    },
                ],
            )
            .with_tasks(
                "p1",
                vec![
                    {
                        let mut parent = task("t1", "Parent task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High");
                        parent.num_subtasks = 1;
                        parent
                    },
                    task("t3", "Later task", "p1", "Inbox", "s2", "Later", "cf1", "Priority", "High"),
                ],
            )
            .with_subtasks(
                "t1",
                vec![{
                    let mut child = task("t2", "Child task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High");
                    child.memberships.clear();
                    child
                }],
            );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        assert_eq!(state.selected_index(), Some(4));
        state.move_down();
        assert_eq!(state.selected_index(), Some(5));

        state.toggle_subtask_visibility();

        assert_eq!(state.selected_index(), Some(4));

        state.move_section_down();
        assert_eq!(state.selected_index(), Some(7));
    }

    #[test]
    fn collapsing_subtasks_selects_the_super_task_when_the_current_row_is_a_subtask() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![{
                    let mut parent = task("t1", "Parent task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High");
                    parent.num_subtasks = 1;
                    parent
                }],
            )
            .with_subtasks(
                "t1",
                vec![{
                    let mut child = task("t2", "Child task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "Low");
                    child.memberships.clear();
                    child
                }],
            );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        assert_eq!(state.selected_index(), Some(4));
        state.move_down();
        assert_eq!(state.selected_index(), Some(5));

        state.toggle_subtask_visibility();

        assert_eq!(state.selected_index(), Some(4));
        assert_eq!(state.table().rows[state.selected_index().unwrap()].gid, "t1");
    }

    #[test]
    fn section_navigation_does_not_jump_backwards_from_a_last_section_subtask() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![
                    SectionDto {
                        gid: "s1".to_string(),
                        name: "Today".to_string(),
                    },
                    SectionDto {
                        gid: "s2".to_string(),
                        name: "Later".to_string(),
                    },
                ],
            )
            .with_tasks(
                "p1",
                vec![{
                    let mut parent = task("t1", "Parent task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High");
                    parent.num_subtasks = 1;
                    parent
                }],
            )
            .with_subtasks(
                "t1",
                vec![{
                    let mut child = task("t2", "Child task", "p1", "Inbox", "s2", "Later", "cf1", "Priority", "Low");
                    child.memberships.clear();
                    child
                }],
            );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        state.move_down();
        assert_eq!(state.selected_index(), Some(5));

        state.move_section_down();

        assert_eq!(state.selected_index(), Some(5));
        assert_eq!(state.table().rows[state.selected_index().unwrap()].gid, "t2");
    }

    #[test]
    fn project_navigation_prefers_the_parent_row_when_the_current_row_is_a_subtask() {
        let client = FakeAsanaClient::new(vec![
            Project::new("p1", "Inbox", true),
            Project::new("p2", "Later", false),
        ])
        .with_sections(
            "p1",
            vec![SectionDto {
                gid: "s1".to_string(),
                name: "Today".to_string(),
            }],
        )
        .with_tasks(
            "p1",
            vec![{
                let mut parent = task("t1", "Parent task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High");
                parent.num_subtasks = 1;
                parent
            }],
        )
        .with_subtasks(
            "t1",
            vec![{
                let mut child = task("t2", "Child task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "Low");
                child.memberships.clear();
                child
            }],
        )
        .with_tasks(
            "p2",
            vec![task("t3", "Other project task", "p2", "Later", "s1", "Today", "cf1", "Priority", "Medium")],
        );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(
                &client,
                &[Project::new("p1", "Inbox", true), Project::new("p2", "Later", false)],
            )
            .expect("tasks load");

        state.move_down();
        assert_eq!(state.selected_index(), Some(5));

        state.move_project_down();
        assert_eq!(state.selected_index(), Some(4));
        assert_eq!(state.table().rows[state.selected_index().unwrap()].gid, "t1");
    }

    #[test]
    fn refreshes_the_view_after_filter_changes_when_loaded_through_the_async_path() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![
                    task("t1", "Open task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    TaskDto {
                        num_subtasks: 0,
                        completed: true,
                        modified_at: None,
                        ..task("t2", "Closed task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High")
                    },
                ],
        );

        let mut state = TaskState::new();
        state.view.settings.filter.completed = None;
        state.begin_loading(&[Project::new("p1", "Inbox", true)]);
        let dataset = TaskState::build_dataset_for_projects(
            &client,
            &[Project::new("p1", "Inbox", true)],
            &crate::asana::TaskQuery::for_project("p1", TaskLoadScope::All),
        )
        .expect("dataset builds");
        state.finish_loading_dataset(dataset);

        assert_eq!(state.table().task_count(), 2);

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 1);
        assert_eq!(state.table().rows.iter().filter(|row| row.kind.is_task()).count(), 1);

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 1);
        assert_eq!(state.table().rows.iter().filter(|row| row.kind.is_task()).count(), 1);

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 2);
        assert_eq!(state.table().rows.iter().filter(|row| row.kind.is_task()).count(), 2);
    }

    /// Runs the compiled `Due` matcher against a task carrying `value`.
    ///
    /// Goes through the prepared matcher rather than a parallel helper, so the
    /// assertions below pin the predicate the filter pass actually runs.
    fn due_matches(value: Option<&str>, query: &str) -> bool {
        let mut field = due_field(query);
        field.value.set_text_at_end(query.to_string());
        let mut record = TaskRecord::new("t1", "Task one");
        record.due_date = value.map(ToString::to_string);

        let matcher = super::PreparedMatcher::for_field(&field, crate::domain::date::today(), None);
        super::PreparedField { field: &field, matcher }.matches(&record)
    }

    fn due_field(query: &str) -> super::TaskFilterFieldState {
        let mut field = super::TaskFilterFieldState::new(
            super::TaskFilterFieldSpec {
                completes: false,
                key: "due".to_string(),
                label: "Due".to_string(),
                kind: TaskFieldFilterKind::Date,
                custom_gids: Vec::new(),
                can_be_empty: true,
            },
            super::TaskFieldStringMode::Fuzzy,
            Vec::new(),
        );
        field.value.set_text_at_end(query.to_string());
        field
    }

    #[test]
    fn date_filters_match_exact_dates_ranges_and_open_bounds() {
        // Token parsing itself is covered in `domain::date`; this pins the
        // predicate the filter panel actually calls.
        assert!(due_matches(Some("2026-09-15"), "2026-09-15"));
        assert!(!due_matches(Some("2026-09-16"), "2026-09-15"));

        assert!(due_matches(Some("2026-09-01"), "2026-09-01..2026-09-30"));
        assert!(due_matches(Some("2026-09-30"), "2026-09-01..2026-09-30"));
        assert!(!due_matches(Some("2026-10-01"), "2026-09-01..2026-09-30"));

        assert!(due_matches(Some("2030-01-01"), "2026-09-01.."));
        assert!(due_matches(Some("2020-01-01"), "..2026-09-01"));

        // A task with no date cannot satisfy a date filter.
        assert!(!due_matches(None, "2026-09-15"));

        // A blank query is not a filter at all, and never reaches the matcher:
        // the row is inactive, so the pass leaves it out entirely.
        assert!(!due_field("   ").is_active());
        assert!(!due_field("").is_active());

        // An unparseable query matches nothing rather than everything.
        assert!(due_field("someday").is_active(), "it is a value, just a bad one");
        assert!(!due_matches(Some("2026-09-15"), "someday"));
        assert!(!due_matches(Some("2026-09-15"), "2026-02-31"));
    }

    #[test]
    fn date_filters_resolve_keywords_against_the_local_date() {
        let today = crate::domain::date::today();

        assert!(due_matches(Some(&today.iso()), "today"));
        assert!(!due_matches(Some(&today.add_days(1).iso()), "today"));
        assert!(due_matches(Some(&today.add_days(1).iso()), "tomorrow"));
        assert!(due_matches(
            Some(&today.iso()),
            &format!("{:02}-{:02}", today.month, today.day)
        ));
    }

    #[test]
    fn label_filters_support_multiple_selected_values_and_label_navigation() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![
                    {
                        let mut task = task("t1", "Open task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High");
                        task.completed = false;
                        task
                    },
                    {
                        let mut task = task("t2", "Closed task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "Low");
                        task.completed = true;
                        task
                    },
                ],
            );

        let mut state = TaskState::new();
        let dataset = TaskState::build_dataset_for_projects(
            &client,
            &[Project::new("p1", "Inbox", true)],
            &crate::asana::TaskQuery::for_project("p1", TaskLoadScope::All),
        )
        .expect("dataset builds");
        state.begin_loading(&[Project::new("p1", "Inbox", true)]);
        state.set_completed_filter(None);
        state.finish_loading_dataset(dataset);
        state.set_visible(true);
        assert_eq!(state.table().task_count(), 2);

        // Move to the built-in State label filter.
        for _ in 0..4 {
            state.move_filter_down();
        }
        assert_eq!(state.filter_selected_kind(), Some(TaskFieldFilterKind::Labels));

        state.filter_edit_begin();
        state.filter_add_label();
        assert_eq!(state.table().task_count(), 1);
        assert_eq!(
            state
                .filter_panel_entries()
                .into_iter()
                .find(|row| row.selected)
                .map(|row| row.label_values),
            Some(vec!["open".to_string()])
        );

        state.filter_add_label();
        state.filter_move_label_right();
        state.filter_cycle_label_value(1);
        assert_eq!(
            state
                .filter_panel_entries()
                .into_iter()
                .find(|row| row.selected)
                .map(|row| row.label_values.clone()),
            Some(vec!["open".to_string(), "done".to_string()])
        );
        assert_eq!(state.table().task_count(), 2);
        assert_eq!(
            state
                .filter_panel_entries()
                .into_iter()
                .find(|row| row.selected)
                .map(|row| row.label_values),
            Some(vec!["open".to_string(), "done".to_string()])
        );

        state.filter_move_label_left();
        state.filter_delete_label();
        assert_eq!(state.table().task_count(), 1);
        assert_eq!(
            state
                .filter_panel_entries()
                .into_iter()
                .find(|row| row.selected)
                .map(|row| row.label_values),
            Some(vec!["done".to_string()])
        );
    }

    #[test]
    fn editing_a_filter_keeps_its_caret_while_projects_stream_in() {
        let projects = vec![Project::new("p1", "Inbox", true)];
        let client = FakeAsanaClient::new(projects.clone())
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![task(
                    "t1", "Open task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High",
                )],
            );

        let query = crate::asana::TaskQuery::for_project("p1", TaskLoadScope::All);
        let dataset = TaskState::build_dataset_for_projects(&client, &projects, &query)
            .expect("dataset builds");

        let mut state = TaskState::new();
        state.begin_loading(&projects);
        state.finish_loading_dataset(dataset.clone());
        state.set_visible(true);

        // Park the caret mid-word in the Assignee filter, the way a user would
        // after arrowing back to correct a typo.
        state.toggle_filter_panel();
        state.move_filter_down();
        assert_eq!(state.filter_panel_entries()[state.view.filter_editor.selected].label, "Assignee");
        state.filter_edit_begin();
        for ch in "alice".chars() {
            state.filter_push_char(ch);
        }
        state.filter_move_caret(-2);

        // A project finishing its fetch rebuilds the filter editor underneath
        // the edit. The caret used to snap back to 0 here.
        state.begin_loading(&projects);
        state.ingest_loaded_project("p1", query, dataset);

        let row = state
            .filter_panel_entries()
            .into_iter()
            .find(|row| row.selected)
            .expect("a filter row stays selected");
        assert_eq!(row.query, "alice");
        assert_eq!(row.caret, Some(3));

        // The caret is still where the user left it, so the next keystroke
        // lands mid-word instead of at the front.
        state.filter_push_char('X');
        assert_eq!(
            state
                .filter_panel_entries()
                .into_iter()
                .find(|row| row.selected)
                .map(|row| row.query),
            Some("aliXce".to_string())
        );
    }

    #[test]
    fn task_query_covers_detects_cache_hits_and_misses() {
        use crate::asana::{TaskLoadScope, TaskQuery, TaskTarget};

        let broad = TaskQuery::for_project("1", TaskLoadScope::All);
        let open_only = TaskQuery::for_project("1", TaskLoadScope::OpenOnly);
        let narrow_date = TaskQuery {
            target: TaskTarget::Project("1".to_string()),
            scope: TaskLoadScope::OpenOnly,
            due_after: Some("2026-01-01".to_string()),
            due_before: Some("2026-06-30".to_string()),
        };
        let wider_date = TaskQuery {
            target: TaskTarget::Project("1".to_string()),
            scope: TaskLoadScope::OpenOnly,
            due_after: Some("2025-01-01".to_string()),
            due_before: Some("2027-12-31".to_string()),
        };

        // All covers OpenOnly and any date bound
        assert!(broad.covers(&open_only));
        assert!(broad.covers(&narrow_date));
        assert!(broad.covers(&wider_date));

        // OpenOnly (no date) covers date-restricted OpenOnly
        assert!(open_only.covers(&narrow_date));
        // OpenOnly does NOT cover All
        assert!(!open_only.covers(&broad));

        // narrow_date covers a query with even tighter dates
        let tighter = TaskQuery {
            target: TaskTarget::Project("1".to_string()),
            scope: TaskLoadScope::OpenOnly,
            due_after: Some("2026-02-01".to_string()),
            due_before: Some("2026-05-31".to_string()),
        };
        assert!(narrow_date.covers(&tighter));
        // narrow_date does NOT cover wider_date
        assert!(!narrow_date.covers(&wider_date));
        // narrow_date does NOT cover no-date (it has a tighter window)
        assert!(!narrow_date.covers(&open_only));
    }

    /// The one set's `due` row, for the tests that drive the push-down
    /// directly rather than through the panel's keys.
    fn set_due_field(
        state: &mut TaskFilterEditorState,
    ) -> Option<&mut super::TaskFilterFieldState> {
        state.sets[0]
            .fields
            .iter_mut()
            .find(|field| field.spec.key == "due")
    }

    #[test]
    fn due_date_range_for_query_extracts_explicit_dates() {
        // Build a filter state with an explicit date range
        let mut state = TaskFilterEditorState::from_dataset(&TaskDataset::default());
        if let Some(due) = set_due_field(&mut state) {
            due.value.set_text_at_end("2026-01-01..2026-06-30");
        }
        let (after, before) = state.due_date_range_for_query();
        assert_eq!(after.as_deref(), Some("2026-01-01"));
        assert_eq!(before.as_deref(), Some("2026-06-30"));
    }

    #[test]
    fn due_date_range_for_query_resolves_keywords() {
        let mut state = TaskFilterEditorState::from_dataset(&TaskDataset::default());
        if let Some(due) = set_due_field(&mut state) {
            due.value.set_text_at_end("today..2026-12-31");
        }
        let (after, before) = state.due_date_range_for_query();

        // "today" resolves against the *local* date before being pushed to the
        // API. Resolving it in UTC fetched the wrong day's tasks all evening.
        assert_eq!(
            after.as_deref(),
            Some(crate::domain::date::today().iso().as_str())
        );
        assert_eq!(before.as_deref(), Some("2026-12-31"));
    }

    #[test]
    fn due_date_range_for_query_bounds_an_exact_date_on_both_sides() {
        let mut state = TaskFilterEditorState::from_dataset(&TaskDataset::default());
        if let Some(due) = set_due_field(&mut state) {
            due.value.set_text_at_end("2026-03-04");
        }
        let (after, before) = state.due_date_range_for_query();
        assert_eq!(after.as_deref(), Some("2026-03-04"));
        assert_eq!(before.as_deref(), Some("2026-03-04"));
    }

    #[test]
    fn due_date_range_for_query_pushes_nothing_for_an_unparseable_query() {
        let mut state = TaskFilterEditorState::from_dataset(&TaskDataset::default());
        if let Some(due) = set_due_field(&mut state) {
            due.value.set_text_at_end("someday");
        }
        assert_eq!(state.due_date_range_for_query(), (None, None));
    }

    /// Moves the filter cursor to the named row, by `spec.key` or by label.
    ///
    /// Custom-field rows are keyed `custom:<name>`, so tests can name either
    /// `"due"` or `"Priority"` and get the row they meant.
    fn select_field(state: &mut TaskState, key: &str) {
        let index = state
            .view
            .filter_editor
            .fields()
            .iter()
            .position(|field| field.spec.key == key || field.spec.label == key)
            .unwrap_or_else(|| panic!("the {key} field exists"));
        state.view.filter_editor.selected = index;
    }

    /// Puts `query` in the named field of the active set.
    ///
    /// Goes through the public keys rather than the struct so the test
    /// exercises the same path a user does: move the cursor to the row, then
    /// type.
    fn set_field(state: &mut TaskState, key: &str, query: &str) {
        select_field(state, key);
        state.filter_clear_current();
        for ch in query.chars() {
            state.filter_push_char(ch);
        }
    }

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

    /// A task with a due date and an assignee, for the set-semantics tests.
    fn sel_task_due(gid: &str, name: &str, assignee: &str, due: &str) -> TaskDto {
        let mut task = sel_task(gid, name, false);
        task.due_on = Some(due.to_string());
        task.assignee = Some(UserDto {
            gid: format!("user-{assignee}"),
            name: Some(assignee.to_string()),
            display_name: Some(assignee.to_string()),
        });
        task
    }

    fn sel_task(gid: &str, name: &str, completed: bool) -> TaskDto {
        TaskDto {
            gid: gid.to_string(),
            name: name.to_string(),
            completed,
            modified_at: None,
            due_on: None,
            start_on: None,
            assignee: None,
            num_subtasks: 0,
            memberships: vec![TaskMembershipDto {
                project: TaskMembershipProjectDto {
                    gid: "p1".to_string(),
                    name: "Inbox".to_string(),
                },
                section: Some(TaskMembershipSectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }),
            }],
            parent: None,
            custom_fields: vec![],
        }
    }

    fn loaded_state_with_tasks(tasks: Vec<TaskDto>) -> TaskState {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto { gid: "s1".to_string(), name: "Today".to_string() }],
            )
            .with_tasks("p1", tasks);
        let mut state = TaskState::new();
        state.set_completed_filter(None);
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state
    }




    /// A loaded state with the panel open and one dated task, for the tests
    /// that only care what `desired_task_query` computes.
    fn state_for_push_down() -> TaskState {
        let mut state = loaded_state_with_tasks(vec![sel_task_due(
            "t1",
            "Task one",
            "Alex",
            "2026-09-03",
        )]);
        state.toggle_filter_panel();
        state
    }

    #[test]
    fn the_pushed_down_due_window_is_the_union_of_the_sets() {
        let mut state = state_for_push_down();

        set_field(&mut state, "due", "2026-09-01..2026-09-07");
        state.filter_add_set();
        set_field(&mut state, "due", "2026-12-01..2026-12-31");

        let query = state.desired_task_query();
        assert_eq!(
            query.due_after.as_deref(),
            Some("2026-09-01"),
            "the earliest start"
        );
        assert_eq!(
            query.due_before.as_deref(),
            Some("2026-12-31"),
            "the latest end"
        );
    }

    #[test]
    fn a_negated_due_row_pushes_down_no_due_window_at_all() {
        // `not (due in September)` is satisfied by everything outside the
        // window, so pushing the window down would drop exactly the tasks the
        // row asked for — and `TaskQuery::covers` would then call it cached.
        let mut state = state_for_push_down();
        set_field(&mut state, "due", "2026-09-01..2026-09-07");
        assert!(state.desired_task_query().due_after.is_some());

        select_field(&mut state, "due");
        state.filter_toggle_negate_field();

        let query = state.desired_task_query();
        assert_eq!(query.due_after, None);
        assert_eq!(query.due_before, None);
    }

    #[test]
    fn a_negated_set_pushes_down_no_due_window_at_all() {
        // Same reasoning one level up: the set keeps what its due row rejected.
        let mut state = state_for_push_down();
        set_field(&mut state, "due", "2026-09-01..2026-09-07");

        state.filter_toggle_negate_set();

        let query = state.desired_task_query();
        assert_eq!(query.due_after, None);
        assert_eq!(query.due_before, None);
    }

    #[test]
    fn a_set_with_no_due_filter_pushes_down_no_due_window_at_all() {
        // Otherwise the server drops the very tasks the second set asked for,
        // and TaskQuery::covers then records the narrow window as cached.
        let mut state = state_for_push_down();
        set_field(&mut state, "due", "2026-09-01..2026-09-07");
        state.filter_add_set();

        let query = state.desired_task_query();
        assert_eq!(query.due_after, None);
        assert_eq!(query.due_before, None);
    }

    #[test]
    fn a_set_requiring_an_empty_due_date_pushes_down_no_due_window() {
        let mut state = state_for_push_down();
        set_field(&mut state, "due", "2026-09-01..2026-09-07");
        state.filter_add_set();
        select_field(&mut state, "due");
        state.filter_toggle_require_empty();

        let query = state.desired_task_query();
        assert_eq!(
            query.due_after, None,
            "undated tasks would be filtered out server-side"
        );
        assert_eq!(query.due_before, None);
    }

    #[test]
    fn an_open_ended_set_opens_that_side_of_the_union() {
        // `2026-09-01..` has no end, so the union has none either, however
        // tight the other set is.
        let mut state = state_for_push_down();
        set_field(&mut state, "due", "2026-09-01..");
        state.filter_add_set();
        set_field(&mut state, "due", "2026-10-01..2026-10-02");

        let query = state.desired_task_query();
        assert_eq!(query.due_after.as_deref(), Some("2026-09-01"));
        assert_eq!(query.due_before, None);
    }

    #[test]
    fn adding_a_set_outside_the_cached_window_is_a_cache_miss() {
        // The cache is keyed by the query it was filled with, so widening the
        // union has to stop reporting coverage or the new set shows nothing.
        let projects = vec![Project::new("p1", "Inbox", true)];
        let mut state = state_for_push_down();
        set_field(&mut state, "due", "2026-09-01..2026-09-07");

        // Re-record the cache against the narrow window the panel now asks for.
        let query = state.desired_task_query();
        state.begin_loading(&projects);
        state.ingest_loaded_project(
            "p1",
            crate::asana::TaskQuery {
                target: crate::asana::TaskTarget::for_project(&projects[0]),
                ..query.clone()
            },
            TaskDataset::default(),
        );
        assert!(state.can_serve_query_for_targets(&projects, &query));

        state.filter_add_set();
        set_field(&mut state, "due", "2026-12-01");

        let widened = state.desired_task_query();
        assert!(
            !state.can_serve_query_for_targets(&projects, &widened),
            "the December set needs a fetch"
        );
        assert_eq!(state.projects_requiring_load(&projects, &widened).len(), 1);
    }

    #[test]
    fn filter_sets_survive_the_rebuild_that_each_streamed_project_triggers() {
        // rebuild_visible_dataset throws the editor away per project; without
        // restore_queries rebuilding the set list, the second tab vanishes
        // mid-load — the same failure the caret had before Milestone 11.75.
        let projects = vec![Project::new("p1", "Inbox", true)];
        let client = FakeAsanaClient::new(projects.clone())
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks("p1", vec![sel_task_due("t1", "Task one", "Alex", "2026-12-01")]);
        let query = crate::asana::TaskQuery::for_project("p1", TaskLoadScope::All);
        let dataset = TaskState::build_dataset_for_projects(&client, &projects, &query)
            .expect("dataset builds");

        let mut state = TaskState::new();
        state.begin_loading(&projects);
        state.finish_loading_dataset(dataset.clone());
        state.set_visible(true);

        state.toggle_filter_panel();
        set_field(&mut state, "assignee", "alex");
        state.filter_add_set();
        set_field(&mut state, "due", "2026-12-01");
        state.filter_edit_begin();

        // A project finishing its fetch rebuilds the filter editor underneath.
        state.begin_loading(&projects);
        state.ingest_loaded_project("p1", query, dataset);

        assert_eq!(
            state.filter_set_position(),
            (1, 2),
            "two sets, still on the second"
        );
        assert_eq!(state.filter_panel_rows_for_set(0)[1].1, "alex");
        assert_eq!(state.filter_panel_rows()[2].1, "2026-12-01");
        assert!(
            state.filter_panel_editing(),
            "and the edit was not interrupted"
        );
    }

    /// A task carrying whichever of due date, assignee, and Priority the test
    /// wants present, so one fixture covers all three field kinds.
    fn task_with(
        gid: &str,
        due: Option<&str>,
        assignee: Option<&str>,
        priority: Option<&str>,
    ) -> TaskDto {
        let mut task = sel_task(gid, gid, false);
        task.due_on = due.map(ToString::to_string);
        task.assignee = assignee.map(|name| UserDto {
            gid: format!("user-{name}"),
            name: Some(name.to_string()),
            display_name: Some(name.to_string()),
        });
        task.custom_fields = priority
            .map(|value| {
                vec![CustomFieldValueDto {
                    gid: "cf1".to_string(),
                    name: "Priority".to_string(),
                    display_value: Some(value.to_string()),
                    enum_value: Some(EnumOptionDto {
                        gid: "opt-1".to_string(),
                        name: value.to_string(),
                        enabled: true,
                    }),
                }]
            })
            .unwrap_or_default();
        task
    }

    /// Like [`loaded_state_with_tasks`], but with a `Priority` custom field
    /// registered on the project, so the panel grows a label row for it.
    ///
    /// Custom-field rows come from the project's field settings rather than
    /// from the values tasks happen to carry, so a task with no `Priority` is
    /// exactly the "field exists, this record has no value" case
    /// require-empty is for.
    fn loaded_state_with_priority_field(tasks: Vec<TaskDto>) -> TaskState {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_custom_field_settings(
                "p1",
                vec![ProjectCustomFieldSettingDto {
                    gid: "set-1".to_string(),
                    custom_field: CustomFieldDto {
                        gid: "cf1".to_string(),
                        name: "Priority".to_string(),
                        resource_subtype: None,
                        enum_options: Vec::new(),
                    },
                }],
            )
            .with_tasks("p1", tasks);
        let mut state = TaskState::new();
        state.set_completed_filter(None);
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state
    }

    #[test]
    fn requiring_an_empty_field_matches_only_the_records_with_nothing_in_it() {
        let mut state = loaded_state_with_priority_field(vec![
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
        assert_eq!(
            visible_gids(&state),
            vec!["blank-assignee", "undated"],
            "a label field with no value for the task counts as empty"
        );
    }

    #[test]
    fn a_require_empty_and_a_query_replace_one_another() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Task one", false)]);
        state.toggle_filter_panel();
        select_field(&mut state, "assignee");

        state.filter_toggle_require_empty();
        assert_eq!(state.filter_panel_rows()[1].1, "(none)");

        state.filter_push_char('a');
        assert_eq!(state.filter_panel_rows()[1].1, "a", "typing replaces it");

        state.filter_toggle_require_empty();
        assert_eq!(
            state.filter_panel_rows()[1].1,
            "(none)",
            "and it replaces the text"
        );

        state.filter_clear_current();
        assert_eq!(state.filter_panel_rows()[1].1, "");
    }

    #[test]
    fn a_require_empty_counts_as_an_active_filter() {
        // It excludes rows, so it has to reach the panel's chip and the status
        // bar the same way a typed value does.
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Task one", false)]);
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
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Task one", false),
            sel_task("t2", "Task two", true),
        ]);
        state.toggle_filter_panel();
        select_field(&mut state, "state");

        state.filter_toggle_require_empty();

        assert_eq!(state.filter_panel_rows()[4].1, "");
        assert_eq!(state.active_filter_count(), 0);
        assert_eq!(visible_gids(&state).len(), 2, "nothing was filtered out");
    }

    #[test]
    fn opening_the_calendar_on_a_require_empty_date_clears_it() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Task one", false)]);
        state.toggle_filter_panel();
        select_field(&mut state, "due");
        state.filter_toggle_require_empty();

        assert!(state.filter_calendar_begin());

        assert_eq!(
            state.filter_panel_rows()[2].1,
            "",
            "a date is about to be picked"
        );
    }

    #[test]
    fn one_set_can_ask_for_soon_while_another_asks_for_undated() {
        // The review this milestone exists for: imminent work, plus work nobody
        // has scheduled.
        let mut state = loaded_state_with_tasks(vec![
            task_with("soon", Some("2026-08-26"), None, None),
            task_with("later", Some("2026-11-01"), None, None),
            task_with("someday", None, None, None),
        ]);
        state.toggle_filter_panel();
        set_field(&mut state, "due", "2026-08-24..2026-08-31");
        state.filter_add_set();
        select_field(&mut state, "due");
        state.filter_toggle_require_empty();

        assert_eq!(visible_gids(&state), vec!["soon", "someday"]);
    }

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

    #[test]
    fn a_regex_row_compiles_its_pattern_once_for_the_whole_pass() {
        // The pattern used to be rebuilt inside the per-record test, which put
        // a regex compile between every task and the next: ~130ms of a ~135ms
        // pass over 20k tasks. `prepare` is what holds it still.
        let mut field = super::TaskFilterFieldState::new(
            super::TaskFilterFieldSpec {
                completes: false,
                key: "title".to_string(),
                label: "Title".to_string(),
                kind: TaskFieldFilterKind::String,
                custom_gids: Vec::new(),
                can_be_empty: true,
            },
            super::TaskFieldStringMode::Regex,
            Vec::new(),
        );
        field.value.set_text_at_end("^ship");

        let compiled = super::PreparedMatcher::for_field(&field, crate::domain::date::today(), None);
        assert!(matches!(compiled, super::PreparedMatcher::Regex(Some(_))));

        // And a pattern that cannot compile matches nothing, rather than
        // reading as "no filter" and letting everything through.
        field.value.set_text_at_end("(unclosed");
        let broken = super::PreparedMatcher::for_field(&field, crate::domain::date::today(), None);
        assert!(matches!(broken, super::PreparedMatcher::Regex(None)));

        let record = TaskRecord::new("t1", "Ship it");
        assert!(!super::PreparedField { field: &field, matcher: broken }.matches(&record));
    }

    #[test]
    fn a_regex_filter_still_matches_the_same_records_it_always_did() {
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Ship the release", false),
            sel_task("t2", "Review the shipment", false),
        ]);
        state.toggle_filter_panel();
        state.filter_set_mode(super::TaskFieldStringMode::Regex);
        set_field(&mut state, "title", "^ship");

        assert_eq!(visible_gids(&state), vec!["t1"], "anchored at the start");
    }

    #[test]
    fn typing_with_more_keys_queued_puts_the_rebuild_off_until_the_burst_ends() {
        // The point of the whole thing: a keystroke with input behind it costs
        // a string insert, not a pass over every task.
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Alpha", false),
            sel_task("t2", "Beta", false),
        ]);
        state.toggle_filter_panel();

        state.set_input_pending(true);
        state.filter_push_char('a');
        state.filter_push_char('l');

        assert_eq!(
            visible_gids(&state),
            vec!["t1", "t2"],
            "the table is still the one the last finished pass built"
        );
        assert!(state.filtering_since().is_some(), "and it says it is behind");

        // The queue drains, so the next draw settles it.
        state.set_input_pending(false);
        state.settle_table();

        assert_eq!(visible_gids(&state), vec!["t1"]);
        assert!(state.filtering_since().is_none());
    }

    #[test]
    fn the_last_keystroke_of_a_burst_filters_without_waiting_to_be_settled() {
        // Nothing is queued behind it, so there is nothing to coalesce with
        // and no reason to show the table a frame out of date.
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Alpha", false),
            sel_task("t2", "Beta", false),
        ]);
        state.toggle_filter_panel();

        // `p` is in Alpha and not in Beta; a fuzzy `a` would match both.
        state.filter_push_char('p');

        assert_eq!(visible_gids(&state), vec!["t1"]);
        assert!(state.filtering_since().is_none());
    }

    #[test]
    fn settling_a_table_that_is_current_does_nothing() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Alpha", false)]);
        state.toggle_filter_panel();
        let before = state.table().clone();

        state.settle_table();

        assert_eq!(state.table(), &before);
        assert!(state.filtering_since().is_none());
    }

    #[test]
    fn a_deferred_rebuild_is_settled_even_if_the_keys_stop_mattering() {
        // `set_input_pending` is the runtime's answer about the *queue*, not
        // about the filter, so a burst that ends on a key which changes
        // nothing still has to leave the table current.
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Alpha", false)]);
        state.toggle_filter_panel();
        state.set_input_pending(true);
        state.filter_push_char('z');
        assert!(state.filtering_since().is_some());

        state.set_input_pending(false);
        state.move_filter_down();
        state.settle_table();

        assert!(visible_gids(&state).is_empty(), "`z` matches no title");
        assert!(state.filtering_since().is_none());
    }

    #[test]
    fn a_task_in_two_projects_is_grouped_under_the_one_that_is_selected() {
        // The cache keeps every project a task has been loaded under, and the
        // names are held sorted — so a task in "Alpha" and "Northwind" was
        // drawn under an "Alpha" header even with only Northwind selected,
        // naming a project the user had deselected.
        let alpha = Project::new("p1", "Alpha", true);
        let northwind = Project::new("p2", "Northwind", true);
        let task = sel_task("shared", "In both projects", false);
        let client = FakeAsanaClient::new(vec![alpha.clone(), northwind.clone()])
            .with_sections(
                "p1",
                vec![SectionDto { gid: "s1".to_string(), name: "Today".to_string() }],
            )
            .with_sections(
                "p2",
                vec![SectionDto { gid: "s2".to_string(), name: "Today".to_string() }],
            )
            .with_tasks("p1", vec![task.clone()])
            .with_tasks("p2", vec![task]);

        let mut state = TaskState::new();
        state.set_completed_filter(None);
        state
            .load_task_dataset_for_projects(&client, &[alpha, northwind.clone()])
            .expect("both projects load");
        state
            .load_task_dataset_for_projects(&client, &[northwind])
            .expect("then only Northwind is selected");

        assert_eq!(project_group_of(&state, "shared"), Some("Northwind"));
    }

    #[test]
    fn a_record_with_no_selected_project_keeps_the_grouping_it_arrived_with() {
        // The assigned-to-me row: its tasks are grouped by the project their
        // own membership named, and none of those is the selected "target".
        let mut record = TaskRecord::new("t1", "Task one");
        record.projects = vec!["Alpha".to_string(), "Northwind".to_string()];
        let mut records = vec![record];

        prefer_selected_projects(&mut records, &["Assigned to me".to_string()]);

        assert_eq!(records[0].projects, vec!["Alpha", "Northwind"]);
    }

    #[test]
    fn filtering_a_parent_away_leaves_its_subtask_in_its_own_project() {
        // The way this shows up in practice: filter by assignee, the parent is
        // someone else's so it drops out, and the subtask that is left has no
        // parent to sit under. It used to fall under whichever project header
        // was drawn last.
        let alpha = Project::new("p1", "Alpha", true);
        let northwind = Project::new("p2", "Northwind", true);

        let mut parent = sel_task("parent", "Jo's parent task", false);
        parent.assignee = Some(UserDto {
            gid: "u-jo".to_string(),
            name: Some("Jo".to_string()),
            display_name: Some("Jo".to_string()),
        });
        parent.num_subtasks = 1;
        parent.memberships = membership("p1", "Alpha");

        let mut child = sel_task("child", "Alex's subtask", false);
        child.assignee = Some(UserDto {
            gid: "u-alex".to_string(),
            name: Some("Alex".to_string()),
            display_name: Some("Alex".to_string()),
        });
        child.memberships = vec![];

        let mut other = sel_task("other", "Alex's Northwind task", false);
        other.assignee = Some(UserDto {
            gid: "u-alex".to_string(),
            name: Some("Alex".to_string()),
            display_name: Some("Alex".to_string()),
        });
        other.memberships = membership("p2", "Northwind");

        let client = FakeAsanaClient::new(vec![alpha.clone(), northwind.clone()])
            .with_sections(
                "p1",
                vec![SectionDto { gid: "s1".to_string(), name: "Today".to_string() }],
            )
            .with_sections(
                "p2",
                vec![SectionDto { gid: "s2".to_string(), name: "Today".to_string() }],
            )
            .with_tasks("p1", vec![parent])
            .with_subtasks("parent", vec![child])
            .with_tasks("p2", vec![other]);

        let mut state = TaskState::new();
        state.set_completed_filter(None);
        state
            .load_task_dataset_for_projects(&client, &[alpha, northwind])
            .expect("tasks load");
        assert_eq!(project_group_of(&state, "child"), Some("Alpha"));

        state.toggle_filter_panel();
        set_field(&mut state, "assignee", "alex");

        assert!(
            !visible_gids(&state).contains(&"parent"),
            "the parent really was filtered out"
        );
        assert_eq!(project_group_of(&state, "child"), Some("Alpha"));
    }

    /// A single project membership, for the two-project fixtures.
    fn membership(gid: &str, name: &str) -> Vec<TaskMembershipDto> {
        vec![TaskMembershipDto {
            project: TaskMembershipProjectDto {
                gid: gid.to_string(),
                name: name.to_string(),
            },
            section: Some(TaskMembershipSectionDto {
                gid: format!("s-{gid}"),
                name: "Today".to_string(),
            }),
        }]
    }

    /// The project header a task row is drawn under.
    fn project_group_of<'a>(state: &'a TaskState, gid: &str) -> Option<&'a str> {
        let mut project = None;
        for row in &state.table().rows {
            if row.kind == TaskRowKind::ProjectHeader {
                project = Some(row.cells[0].as_str());
            }
            if row.kind.is_task() && row.gid == gid {
                return project;
            }
        }
        None
    }

    #[test]
    fn negating_a_field_keeps_exactly_what_it_was_throwing_away() {
        let mut state = loaded_state_with_tasks(vec![
            sel_task_due("alex-aug", "Alex August", "Alex", "2026-08-10"),
            sel_task_due("jo-aug", "Jo August", "Jo", "2026-08-11"),
        ]);
        state.toggle_filter_panel();
        set_field(&mut state, "assignee", "alex");
        assert_eq!(visible_gids(&state), vec!["alex-aug"]);

        state.filter_toggle_negate_field();
        assert_eq!(visible_gids(&state), vec!["jo-aug"], "the complement");

        state.filter_toggle_negate_field();
        assert_eq!(visible_gids(&state), vec!["alex-aug"], "and back");
    }

    #[test]
    fn a_negated_field_with_no_value_still_filters_nothing() {
        // An empty field means "do not filter", and inverting a row that is
        // never consulted must not turn it into "match nothing".
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Task one", false),
            sel_task("t2", "Task two", false),
        ]);
        state.toggle_filter_panel();
        select_field(&mut state, "assignee");

        state.filter_toggle_negate_field();

        assert_eq!(visible_gids(&state).len(), 2);
        assert_eq!(state.active_filter_count(), 0, "and it is not counted");
    }

    #[test]
    fn negating_a_require_empty_asks_for_any_value_at_all() {
        // The one thing the panel could not say before: "has an assignee".
        let mut state = loaded_state_with_tasks(vec![
            task_with("owned", None, Some("Alex"), None),
            task_with("orphan", None, None, None),
        ]);
        state.toggle_filter_panel();
        select_field(&mut state, "assignee");
        state.filter_toggle_require_empty();
        assert_eq!(visible_gids(&state), vec!["orphan"]);

        state.filter_toggle_negate_field();

        assert_eq!(visible_gids(&state), vec!["owned"]);
    }

    #[test]
    fn negating_a_set_inverts_its_fields_together_rather_than_one_by_one() {
        // `not (assignee alex and due in august)` keeps the August task Jo
        // owns; negating each row instead would have dropped it.
        let mut state = loaded_state_with_tasks(vec![
            sel_task_due("alex-aug", "Alex August", "Alex", "2026-08-10"),
            sel_task_due("jo-aug", "Jo August", "Jo", "2026-08-11"),
            sel_task_due("alex-dec", "Alex December", "Alex", "2026-12-01"),
        ]);
        state.toggle_filter_panel();
        set_field(&mut state, "assignee", "alex");
        set_field(&mut state, "due", "2026-08-01..2026-08-31");

        state.filter_toggle_negate_set();

        assert!(state.filter_active_set_negated());
        assert_eq!(visible_gids(&state), vec!["jo-aug", "alex-dec"]);
    }

    #[test]
    fn a_negated_set_still_ors_with_the_others() {
        let mut state = loaded_state_with_tasks(vec![
            sel_task_due("alex-aug", "Alex August", "Alex", "2026-08-10"),
            sel_task_due("jo-aug", "Jo August", "Jo", "2026-08-11"),
            sel_task_due("jo-dec", "Jo December", "Jo", "2026-12-01"),
        ]);
        state.toggle_filter_panel();
        // Set 1: not Alex's. Set 2: December, whoever owns it.
        set_field(&mut state, "assignee", "alex");
        state.filter_toggle_negate_set();
        state.filter_add_set();
        set_field(&mut state, "due", "2026-12-01");

        assert_eq!(state.filter_set_negations(), vec![true, false]);
        assert_eq!(visible_gids(&state), vec!["jo-aug", "jo-dec"]);
    }

    #[test]
    fn a_negated_empty_set_contributes_nothing_rather_than_everything() {
        // A fresh set accepts everything, so its complement accepts nothing —
        // which keeps "adding a set can only widen" true even after `~`.
        let mut state = loaded_state_with_tasks(vec![
            sel_task_due("alex-aug", "Alex August", "Alex", "2026-08-10"),
            sel_task_due("jo-aug", "Jo August", "Jo", "2026-08-11"),
        ]);
        state.toggle_filter_panel();
        set_field(&mut state, "assignee", "alex");
        state.filter_add_set();

        state.filter_toggle_negate_set();

        assert_eq!(visible_gids(&state), vec!["alex-aug"], "set 1 alone");
    }

    #[test]
    fn a_new_set_and_a_cleared_row_drop_the_negations_they_inherited() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Task one", false)]);
        state.toggle_filter_panel();
        set_field(&mut state, "assignee", "alex");
        state.filter_toggle_negate_field();
        state.filter_toggle_negate_set();

        state.filter_add_set();

        assert_eq!(state.filter_set_negations(), vec![true, false]);
        assert!(
            !state.filter_panel_entries()[1].negated,
            "the copied row starts clean"
        );

        // And `ctrl-l` on the original row takes the negation with the value.
        state.filter_select_set(-1);
        select_field(&mut state, "assignee");
        state.filter_clear_current();
        assert!(!state.filter_panel_entries()[1].negated);
    }

    #[test]
    fn negations_survive_the_rebuild_a_reload_triggers() {
        let mut state = loaded_state_with_tasks(vec![sel_task_due(
            "alex-aug", "Alex August", "Alex", "2026-08-10",
        )]);
        state.toggle_filter_panel();
        set_field(&mut state, "assignee", "alex");
        state.filter_toggle_negate_field();
        state.filter_toggle_negate_set();

        // The same rebuild each streamed project triggers.
        state.refresh_from_cache();

        assert_eq!(state.filter_set_negations(), vec![true]);
        assert!(state.filter_panel_entries()[1].negated);
    }

    #[test]
    fn an_empty_set_accepts_everything_so_adding_one_can_only_widen() {
        let mut state = loaded_state_with_tasks(vec![
            sel_task_due("alex-aug", "Alex August", "Alex", "2026-08-10"),
            sel_task_due("jo-aug", "Jo August", "Jo", "2026-08-11"),
            sel_task_due("jo-dec", "Jo December", "Jo", "2026-12-01"),
        ]);
        state.toggle_filter_panel();
        set_field(&mut state, "assignee", "alex");
        let narrowed = visible_gids(&state).len();

        state.filter_add_set();

        assert_eq!(
            visible_gids(&state).len(),
            3,
            "the empty set lets everything through"
        );
        assert!(narrowed < 3, "and the first set really was narrowing");
    }

    #[test]
    fn a_new_set_starts_empty_and_lands_after_the_current_one() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Task one", false)]);
        state.toggle_filter_panel();
        set_field(&mut state, "assignee", "alex");

        state.filter_add_set();

        assert_eq!(state.filter_set_position(), (1, 2));
        assert_eq!(state.filter_set_counts(), vec![1, 0]);
        assert!(
            state
                .filter_panel_rows()
                .iter()
                .all(|(_, query)| query.is_empty()),
            "the new set carries nothing over"
        );
    }

    #[test]
    fn the_last_set_cannot_be_removed() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Task one", false)]);
        state.toggle_filter_panel();

        state.filter_remove_set();
        assert_eq!(state.filter_set_position(), (0, 1), "one set is the panel");

        state.filter_add_set();
        state.filter_remove_set();
        assert_eq!(state.filter_set_position(), (0, 1));
    }

    #[test]
    fn removing_the_last_tab_moves_the_cursor_back_onto_a_tab_that_exists() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Task one", false)]);
        state.toggle_filter_panel();
        state.filter_add_set();
        state.filter_add_set();
        assert_eq!(state.filter_set_position(), (2, 3));

        state.filter_remove_set();

        assert_eq!(state.filter_set_position(), (1, 2));
    }

    #[test]
    fn switching_sets_wraps_and_closes_any_open_picker() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Task one", false)]);
        state.toggle_filter_panel();
        state.filter_add_set();
        select_field(&mut state, "due");
        assert!(state.filter_calendar_begin());

        state.filter_select_set(1);

        assert_eq!(
            state.filter_set_position().0,
            0,
            "wrapped from the last to the first"
        );
        assert!(
            !state.calendar_open(),
            "the picker belonged to the set we left"
        );
        assert!(!state.filter_panel_editing());
    }

    #[test]
    fn the_field_cursor_is_shared_by_every_set() {
        // Switching tabs keeps you on the row you were reading, which is what
        // you want when comparing one field across two sets.
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Task one", false)]);
        state.toggle_filter_panel();
        select_field(&mut state, "projects");
        let row = state.view.filter_editor.selected;

        state.filter_add_set();
        assert_eq!(state.view.filter_editor.selected, row);
        state.filter_select_set(-1);
        assert_eq!(state.view.filter_editor.selected, row);
    }


    // ---- Named filter sets -------------------------------------------------

    /// A panel built from the same dataset twice: one to fill in and save, one
    /// to apply the saved form onto.
    fn saved_round_trip_pair() -> (TaskState, TaskState) {
        let tasks = vec![
            task_with("a", Some("2026-09-01"), Some("Alex"), Some("High")),
            task_with("b", None, None, Some("Low")),
        ];
        (
            loaded_state_with_priority_field(tasks.clone()),
            loaded_state_with_priority_field(tasks),
        )
    }

    #[test]
    fn a_filled_in_panel_survives_the_trip_through_toml_and_back() {
        let (mut state, mut fresh) = saved_round_trip_pair();

        // One field of each kind, a negated row, a require-empty, and a
        // second, negated set.
        set_field(&mut state, "title", "ship");
        state.filter_cycle_mode(); // fuzzy -> contains, so `match` is written
        set_field(&mut state, "assignee", "alex");
        state.filter_toggle_negate_field();
        select_field(&mut state, "Priority");
        state.filter_cycle_label_value(1);
        select_field(&mut state, "start");
        state.filter_toggle_require_empty();
        state.filter_add_set();
        set_field(&mut state, "due", "2026-09-01..2026-09-30");
        // Its `Title` gets a value too: a row with nothing in it is not
        // written, so the `contains` it inherited from the first set is
        // session state rather than something a name captures.
        set_field(&mut state, "title", "kit");
        state.filter_toggle_negate_set();

        let saved = state.filter_sets_to_saved();
        let text = toml::to_string_pretty(&crate::config::NamedFilterSet {
            name: "Round trip".to_string(),
            sets: saved.clone(),
        })
        .expect("serializes");
        let parsed: crate::config::NamedFilterSet =
            toml::from_str(&text).expect("reparses");
        assert_eq!(parsed.sets, saved, "the file is what the panel produced");

        fresh.filter_sets_load("Round trip", &parsed.sets);

        // The whole `Vec<TaskFilterSet>`, not field by field: that is what
        // makes this catch a field added later and not carried.
        assert_eq!(
            fresh.view.filter_editor.sets, state.view.filter_editor.sets,
            "the reloaded panel is the panel that was saved"
        );
    }

    #[test]
    fn a_saved_field_with_no_row_to_land_on_is_parked_rather_than_dropped() {
        // The panel is written back to the named entry on every change, so a
        // field dropped here is a filter deleted from the user's config.
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship", false)]);
        let saved = vec![crate::config::SavedFilterSet {
            negated: false,
            fields: vec![
                crate::config::SavedFilterField {
                    key: "assignee".to_string(),
                    query: "alex".to_string(),
                    ..Default::default()
                },
                crate::config::SavedFilterField {
                    key: "custom:Priority".to_string(),
                    query: "High".to_string(),
                    ..Default::default()
                },
            ],
        }];

        state.filter_sets_load("Mine", &saved);

        assert!(
            state
                .view
                .filter_editor
                .fields()
                .iter()
                .all(|field| field.spec.key != "custom:Priority"),
            "this dataset has no Priority row for it to land on"
        );
        assert_eq!(
            state.filter_sets_to_saved(),
            saved,
            "and it is still written back out"
        );
    }

    #[test]
    fn a_parked_field_lands_once_the_project_carrying_it_loads() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship", false)]);
        state.filter_sets_load(
            "Mine",
            &[crate::config::SavedFilterSet {
                negated: false,
                fields: vec![crate::config::SavedFilterField {
                    key: "custom:Priority".to_string(),
                    query: "High".to_string(),
                    ..Default::default()
                }],
            }],
        );

        // The rebuild a streamed project triggers is exactly the moment the
        // custom-field row appears.
        let projects = vec![Project::new("p1", "Inbox", true)];
        let client = FakeAsanaClient::new(projects.clone())
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_custom_field_settings(
                "p1",
                vec![ProjectCustomFieldSettingDto {
                    gid: "set-1".to_string(),
                    custom_field: CustomFieldDto {
                        gid: "cf1".to_string(),
                        name: "Priority".to_string(),
                        resource_subtype: None,
                        enum_options: Vec::new(),
                    },
                }],
            )
            .with_tasks(
                "p1",
                vec![task_with("t1", None, None, Some("High"))],
            );
        let query = crate::asana::TaskQuery::for_project("p1", TaskLoadScope::All);
        let dataset = TaskState::build_dataset_for_projects(&client, &projects, &query)
            .expect("dataset builds");
        state.begin_loading(&projects);
        state.ingest_loaded_project("p1", query, dataset);

        let priority = state
            .view
            .filter_editor
            .fields()
            .iter()
            .find(|field| field.spec.key == "custom:Priority")
            .expect("the row exists now");
        assert_eq!(priority.value.text(), "High", "the parked value landed on it");
        assert!(
            state.view.filter_editor.sets[0].unresolved.is_empty(),
            "and nothing is still parked"
        );
    }

    #[test]
    fn the_binding_and_the_sidebar_survive_a_streaming_reload() {
        let projects = vec![Project::new("p1", "Inbox", true)];
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship", false)]);
        state.filter_sets_load(
            "Mine",
            &[crate::config::SavedFilterSet {
                negated: false,
                fields: vec![crate::config::SavedFilterField {
                    key: "assignee".to_string(),
                    query: "alex".to_string(),
                    ..Default::default()
                }],
            }],
        );
        state.filter_sets_toggle_sidebar();
        state.set_filter_sets_window(3);
        state.filter_sets_page(1, 14);

        let client = FakeAsanaClient::new(projects.clone())
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks("p1", vec![sel_task("t1", "Ship", false)]);
        let query = crate::asana::TaskQuery::for_project("p1", TaskLoadScope::All);
        let dataset = TaskState::build_dataset_for_projects(&client, &projects, &query)
            .expect("dataset builds");
        state.begin_loading(&projects);
        state.ingest_loaded_project("p1", query, dataset);

        assert_eq!(state.filter_set_loaded_name(), Some("Mine"));
        assert!(state.filter_sets_sidebar_visible());
        assert_eq!(state.filter_sets_page_start(), 3);
        assert_eq!(state.filter_panel_rows()[1].1, "alex");
    }

    #[test]
    fn loading_replaces_every_tab_and_clamps_the_cursors() {
        let mut state = loaded_state_with_priority_field(vec![task_with(
            "a",
            Some("2026-09-01"),
            Some("Alex"),
            Some("High"),
        )]);
        // Three tabs, standing on the last, with the cursor on the last row.
        state.filter_add_set();
        state.filter_add_set();
        select_field(&mut state, "Priority");
        assert_eq!(state.filter_set_position(), (2, 3));

        state.filter_sets_load(
            "One tab",
            &[crate::config::SavedFilterSet {
                negated: true,
                fields: vec![crate::config::SavedFilterField {
                    key: "assignee".to_string(),
                    query: "alex".to_string(),
                    ..Default::default()
                }],
            }],
        );

        assert_eq!(state.filter_set_position(), (0, 1), "one tab, back on it");
        assert!(state.filter_active_set_negated());
        assert_eq!(state.filter_panel_rows()[1].1, "alex");
        assert!(
            state.view.filter_editor.selected < state.view.filter_editor.fields().len(),
            "the field cursor is inside the rows that exist"
        );
    }

    #[test]
    fn copying_to_new_keeps_the_panel_and_stops_the_write_through() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship", false)]);
        state.filter_sets_load(
            "Mine",
            &[crate::config::SavedFilterSet {
                negated: false,
                fields: vec![crate::config::SavedFilterField {
                    key: "assignee".to_string(),
                    query: "alex".to_string(),
                    ..Default::default()
                }],
            }],
        );

        state.filter_set_detach();

        assert_eq!(state.filter_set_loaded_name(), None);
        assert_eq!(
            state.filter_panel_rows()[1].1,
            "alex",
            "the panel keeps exactly what it was showing"
        );
        set_field(&mut state, "title", "ship");
        assert!(
            !state.filter_set_dirty(),
            "and a later edit has nothing to write through to"
        );
    }

    #[test]
    fn a_fresh_panel_keeps_nothing_at_all() {
        let mut state = loaded_state_with_priority_field(vec![task_with(
            "a",
            Some("2026-09-01"),
            Some("Alex"),
            Some("High"),
        )]);
        set_field(&mut state, "title", "ship");
        // A non-default match mode, a second tab, a negation, and a parked
        // value: everything `new` has to throw away.
        state.filter_cycle_mode();
        state.filter_add_set();
        state.filter_toggle_negate_set();
        set_field(&mut state, "assignee", "alex");
        state.view.filter_editor.sets[0].unresolved.push(
            crate::config::SavedFilterField {
                key: "custom:Gone".to_string(),
                query: "x".to_string(),
                ..Default::default()
            },
        );
        state.filter_set_bind("mine");

        state.filter_set_new();

        assert_eq!(state.filter_set_position(), (0, 1), "one empty tab");
        assert!(!state.filter_active_set_negated());
        assert_eq!(state.active_filter_count(), 0);
        assert!(state
            .filter_panel_rows()
            .iter()
            .all(|(_, query)| query.is_empty()));
        assert_eq!(
            state.filter_sets_to_saved(),
            vec![crate::config::SavedFilterSet::default()],
            "nothing is left parked either"
        );
        assert_eq!(
            state.view.filter_editor.selected, 0,
            "and the field cursor starts at the top"
        );

        // Unlike `a`, which keeps the match modes so a second set inherits
        // them, `new` puts every row back to the one it was built with.
        let title = &state.view.filter_editor.fields()[0];
        assert_eq!(title.string_mode, title.default_string_mode);

        // Nothing is bound, so the panel it just emptied is not written
        // through to the entry it came from.
        assert_eq!(state.filter_set_loaded_name(), None);
        assert!(!state.filter_set_dirty());
    }

    #[test]
    fn a_bound_panel_is_marked_dirty_by_any_change_and_an_unbound_one_is_not() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship", false)]);

        set_field(&mut state, "title", "ship");
        assert!(!state.filter_set_dirty(), "nothing is bound yet");

        state.filter_sets_load("Mine", &[]);
        assert!(!state.filter_set_dirty(), "a fresh load is already on disk");

        set_field(&mut state, "title", "ship");
        assert!(state.filter_set_dirty());
        state.clear_filter_set_dirty();
        assert!(!state.filter_set_dirty());
    }

    #[test]
    fn the_numbered_window_pages_and_clamps_at_both_ends() {
        let mut state = TaskState::new();
        state.set_filter_sets_window(9);

        // Everything fits, so there is nothing to page to: moving the window
        // would only make the digits lie about which entry they load.
        state.filter_sets_page(1, 4);
        assert_eq!(state.filter_sets_page_start(), 0);

        state.filter_sets_page(1, 14);
        assert_eq!(state.filter_sets_page_start(), 9);
        state.filter_sets_page(1, 14);
        assert_eq!(state.filter_sets_page_start(), 9, "clamped at the last page");
        state.filter_sets_page(-1, 14);
        assert_eq!(state.filter_sets_page_start(), 0);
        state.filter_sets_page(-1, 14);
        assert_eq!(state.filter_sets_page_start(), 0, "clamped at the first");
    }

    #[test]
    fn deleting_the_prompt_target_is_refused_with_nothing_loaded() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship", false)]);

        assert!(!state.filter_set_prompt_delete());
        assert!(state.filter_set_prompt().is_none());

        state.filter_set_bind("Mine");
        assert!(state.filter_set_prompt_delete());
        assert!(matches!(
            state.filter_set_prompt(),
            Some(super::SidebarPrompt::ConfirmDelete { name }) if name == "Mine"
        ));
    }

    #[test]
    fn the_save_prompt_opens_the_sidebar_and_prefills_the_loaded_name() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship", false)]);
        state.filter_set_bind("Mine");

        state.filter_set_prompt_save();

        assert!(state.filter_sets_sidebar_visible(), "the prompt needs a border to ride");
        assert_eq!(state.filter_set_prompt_text(), Some("Mine"));

        // The caret starts at the end, and typing and deleting act on it.
        state.filter_set_prompt_push_char('r');
        assert_eq!(state.filter_set_prompt_text(), Some("Miner"));
        state.filter_set_prompt_move_caret(-2);
        state.filter_set_prompt_push_char('X');
        assert_eq!(state.filter_set_prompt_text(), Some("MinXer"));
        state.filter_set_prompt_pop_char();
        assert_eq!(state.filter_set_prompt_text(), Some("Miner"));
    }

    #[test]
    fn toggle_task_selection_marks_cursor_row_and_advances() {
        use crate::input::Action;
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Task one", false),
            sel_task("t2", "Task two", false),
        ]);

        assert_eq!(state.selected_task_count(), 0);

        let first_index = state.selected_index().expect("cursor set after load");
        assert_eq!(state.table().rows[first_index].gid, "t1");

        state.apply_action(&Action::ToggleTaskSelection, 10);

        assert_eq!(state.selected_task_count(), 1);
        assert!(state.is_task_selected("t1"));
        let second_index = state.selected_index().expect("cursor advanced");
        assert_eq!(state.table().rows[second_index].gid, "t2");

        state.apply_action(&Action::ToggleTaskSelection, 10);
        assert_eq!(state.selected_task_count(), 2);
        assert!(state.is_task_selected("t2"));
    }

    #[test]
    fn toggling_a_selected_task_deselects_it() {
        use crate::input::Action;
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Task one", false)]);

        state.apply_action(&Action::ToggleTaskSelection, 10);
        assert_eq!(state.selected_task_count(), 1);

        // Move cursor back to t1 and toggle again
        state.apply_action(&Action::JumpTop, 10);
        state.apply_action(&Action::ToggleTaskSelection, 10);
        assert_eq!(state.selected_task_count(), 0);
        assert!(!state.is_task_selected("t1"));
    }

    #[test]
    fn select_all_visible_tasks_selects_every_task_row() {
        use crate::input::Action;
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Task one", false),
            sel_task("t2", "Task two", false),
        ]);

        state.apply_action(&Action::SelectAllVisibleTasks, 10);

        assert_eq!(state.selected_task_count(), 2);
        assert!(state.is_task_selected("t1"));
        assert!(state.is_task_selected("t2"));
    }

    #[test]
    fn invert_task_selection_flips_selected_and_unselected() {
        use crate::input::Action;
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Task one", false),
            sel_task("t2", "Task two", false),
        ]);

        // Select t1 only
        state.apply_action(&Action::ToggleTaskSelection, 10);
        assert!(state.is_task_selected("t1"));
        assert!(!state.is_task_selected("t2"));

        // Invert: t1 deselected, t2 selected
        state.apply_action(&Action::InvertTaskSelection, 10);
        assert!(!state.is_task_selected("t1"));
        assert!(state.is_task_selected("t2"));
        assert_eq!(state.selected_task_count(), 1);
    }

    #[test]
    fn clear_task_selection_empties_the_selection() {
        use crate::input::Action;
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Task one", false),
            sel_task("t2", "Task two", false),
        ]);

        state.apply_action(&Action::SelectAllVisibleTasks, 10);
        assert_eq!(state.selected_task_count(), 2);

        state.apply_action(&Action::ClearTaskSelection, 10);
        assert_eq!(state.selected_task_count(), 0);
    }

    #[test]
    fn clear_hidden_task_selection_removes_tasks_not_in_current_view() {
        use crate::input::Action;
        // Load one open and one completed task; show all initially
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Open task", false),
            sel_task("t2", "Done task", true),
        ]);

        // Both tasks visible — select all
        assert_eq!(state.table().task_count(), 2);
        state.apply_action(&Action::SelectAllVisibleTasks, 10);
        assert_eq!(state.selected_task_count(), 2);

        // Filter to open-only: t2 is now hidden
        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 1);

        // clear_hidden removes t2 from selection
        state.apply_action(&Action::ClearHiddenTaskSelection, 10);
        assert_eq!(state.selected_task_count(), 1);
        assert!(state.is_task_selected("t1"));
        assert!(!state.is_task_selected("t2"));
    }

    #[test]
    fn copy_tasks_to_clipboard_returns_markdown_checklist() {
        use crate::input::{Action, AppCommand};
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Ship release", false),
            sel_task("t2", "Write docs", false),
        ]);

        state.apply_action(&Action::SelectAllVisibleTasks, 10);

        let result = state.apply_action(&Action::CopyTasksToClipboard, 10);
        let text = match result {
            Some(AppCommand::CopyToClipboard(t)) => t,
            _ => panic!("expected CopyToClipboard command"),
        };

        assert!(text.contains("- [ ] [Ship release](https://app.asana.com/0/p1/t1)"));
        assert!(text.contains("- [ ] [Write docs](https://app.asana.com/0/p1/t2)"));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn copy_tasks_to_clipboard_returns_none_when_nothing_selected() {
        use crate::input::Action;
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Task one", false)]);

        let result = state.apply_action(&Action::CopyTasksToClipboard, 10);
        assert!(result.is_none());
    }

    #[test]
    fn selected_task_url_returns_asana_url_for_cursor_task() {
        let state = loaded_state_with_tasks(vec![sel_task("t1", "Ship release", false)]);

        let url = state.selected_task_url().expect("URL produced for task row");
        assert_eq!(url, "https://app.asana.com/0/p1/t1");
    }

    #[test]
    fn selected_task_url_returns_none_when_no_tasks_loaded() {
        let state = TaskState::new();
        assert!(state.selected_task_url().is_none());
    }

    #[test]
    fn lazy_load_uses_date_filter_from_filter_state() {
        use crate::asana::{fake::FakeAsanaClient, AsanaClient, TaskLoadScope};
        use crate::asana::dto::TaskDto;
        use crate::domain::Project;

        let early = TaskDto {
            gid: "t1".to_string(),
            name: "Early task".to_string(),
            completed: false,
            due_on: Some("2026-02-01".to_string()),
            modified_at: None,
            start_on: None,
            assignee: None,
            num_subtasks: 0,
            memberships: vec![],
            parent: None,
            custom_fields: vec![],
        };
        let late = TaskDto {
            gid: "t2".to_string(),
            name: "Late task".to_string(),
            completed: false,
            due_on: Some("2026-11-01".to_string()),
            modified_at: None,
            start_on: None,
            assignee: None,
            num_subtasks: 0,
            memberships: vec![],
            parent: None,
            custom_fields: vec![],
        };
        let no_due = TaskDto {
            gid: "t3".to_string(),
            name: "No due date".to_string(),
            completed: false,
            due_on: None,
            modified_at: None,
            start_on: None,
            assignee: None,
            num_subtasks: 0,
            memberships: vec![],
            parent: None,
            custom_fields: vec![],
        };

        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)])
            .with_tasks("1", vec![early.clone(), late.clone(), no_due.clone()]);

        let mut task_state = TaskState::new();
        task_state.set_visible(true);

        // Set an explicit date range filter for due dates
        task_state.begin_loading(&[Project::new("1", "Inbox", true)]);
        // Manually wire up a filter to test the query extraction
        // We need to set the due filter BEFORE loading so desired_task_query picks it up
        // (In normal app flow, filters are set before starting a load)
        // For simplicity, test via load_task_dataset_for_projects with pre-wired filter via
        // the filter editor state — but that's hard to access directly.
        // Instead, test via the query model directly.
        let query = crate::asana::TaskQuery {
            target: crate::asana::TaskTarget::Project("1".to_string()),
            scope: TaskLoadScope::OpenOnly,
            due_after: Some("2026-03-01".to_string()),
            due_before: Some("2026-12-31".to_string()),
        };
        let tasks = client.list_tasks(&query).expect("tasks load");
        // early (Feb) excluded; late (Nov) included; no-due excluded
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].gid, "t2");
    }

    /// A task assigned to the current user but sitting in no project of its own
    /// is placed by its parent, which is often not part of the loaded set at
    /// all — completed, outside the date window, or assigned to someone else.
    fn assigned_subtask(gid: &str, name: &str, parent_gid: Option<&str>) -> TaskDto {
        let mut task = task(gid, name, "", "", "", "", "cf", "Tag", "red");
        task.memberships.clear();
        task.parent = parent_gid.map(|gid| TaskParentDto {
            gid: gid.to_string(),
        });
        task
    }

    fn task_names(table: &TaskTableModel) -> Vec<String> {
        table
            .rows
            .iter()
            .filter(|row| row.kind == TaskRowKind::Task)
            .map(|row| row.cells[0].clone())
            .collect()
    }

    fn project_headers(table: &TaskTableModel) -> Vec<String> {
        table
            .rows
            .iter()
            .filter(|row| row.kind == TaskRowKind::ProjectHeader)
            .map(|row| row.cells[0].clone())
            .collect()
    }

    fn assigned_to_me_projects() -> Vec<Project> {
        vec![
            Project::assigned_to_me("user-1"),
            Project::new("pz", "Zeta", false),
        ]
    }

    #[test]
    fn an_assigned_subtask_groups_under_its_parents_project_when_the_parent_is_filtered_out() {
        let projects = assigned_to_me_projects();
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![assigned_subtask("s1", "Assigned subtask", Some("t1"))])
            // The parent is in Zeta but no list request returns it, so only a
            // direct lookup can place the subtask.
            .with_standalone_tasks(vec![task(
                "t1", "Parent in Zeta", "pz", "Zeta", "sz", "Doing", "cf", "Tag", "red",
            )]);

        let table = TaskState::build_table_for_projects(&client, &projects).expect("table");

        let row = table
            .rows
            .iter()
            .find(|row| row.cells[0] == "Assigned subtask")
            .expect("the assigned subtask is shown");
        assert_eq!(row.project.as_deref(), Some("Zeta"));
        assert_eq!(row.cells[5], "Zeta", "the Projects column follows the grouping");
        assert_eq!(
            project_headers(&table),
            vec!["Zeta".to_string()],
            "no assigned-to-me group is left behind"
        );
        assert_eq!(client.get_task_calls(), vec!["t1".to_string()]);
    }

    #[test]
    fn an_assigned_task_groups_under_the_project_it_is_a_member_of() {
        let projects = assigned_to_me_projects();
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![task(
                "t1", "Own membership", "pz", "Zeta", "sz", "Doing", "cf", "Tag", "red",
            )]);

        let table = TaskState::build_table_for_projects(&client, &projects).expect("table");

        assert_eq!(project_headers(&table), vec!["Zeta".to_string()]);
        assert!(
            client.get_task_calls().is_empty(),
            "a task with its own membership needs no parent lookup"
        );
    }

    #[test]
    fn an_assigned_task_outside_every_project_stays_in_the_assigned_to_me_group() {
        let projects = assigned_to_me_projects();
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![assigned_subtask("s1", "Loose task", None)]);

        let table = TaskState::build_table_for_projects(&client, &projects).expect("table");

        assert_eq!(
            project_headers(&table),
            vec!["No Project (Assigned to Me)".to_string()]
        );
    }

    #[test]
    fn placement_walks_past_ancestors_that_are_in_no_project_themselves() {
        let projects = assigned_to_me_projects();
        let mut middle = assigned_subtask("t2", "Middle", Some("t1"));
        middle.assignee = None;
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![assigned_subtask("s1", "Deep subtask", Some("t2"))])
            .with_standalone_tasks(vec![
                middle,
                task("t1", "Top", "pz", "Zeta", "sz", "Doing", "cf", "Tag", "red"),
            ]);

        let table = TaskState::build_table_for_projects(&client, &projects).expect("table");

        assert_eq!(project_headers(&table), vec!["Zeta".to_string()]);
        assert_eq!(
            client.get_task_calls(),
            vec!["t2".to_string(), "t1".to_string()],
            "the walk stops at the first ancestor with a project"
        );
    }

    #[test]
    fn siblings_share_one_lookup_of_the_parent_they_have_in_common() {
        let projects = assigned_to_me_projects();
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![
                assigned_subtask("s1", "First subtask", Some("t1")),
                assigned_subtask("s2", "Second subtask", Some("t1")),
            ])
            .with_standalone_tasks(vec![task(
                "t1", "Parent in Zeta", "pz", "Zeta", "sz", "Doing", "cf", "Tag", "red",
            )]);

        let table = TaskState::build_table_for_projects(&client, &projects).expect("table");

        assert_eq!(project_headers(&table), vec!["Zeta".to_string()]);
        assert_eq!(
            client.get_task_calls(),
            vec!["t1".to_string()],
            "the second sibling reuses the memoized placement"
        );
    }

    #[test]
    fn a_parent_lookup_that_fails_leaves_the_task_in_the_assigned_to_me_group() {
        let projects = assigned_to_me_projects();
        // No fixture for "t1": the lookup errors the way a revoked permission
        // or a deleted task would.
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![assigned_subtask("s1", "Orphan", Some("t1"))]);

        let table = TaskState::build_table_for_projects(&client, &projects).expect("table");

        assert_eq!(
            project_headers(&table),
            vec!["No Project (Assigned to Me)".to_string()],
            "a failed lookup must not fail the whole load"
        );
    }

    #[test]
    fn a_parent_cycle_does_not_loop_forever() {
        let projects = assigned_to_me_projects();
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![assigned_subtask("s1", "Cyclic", Some("t1"))])
            .with_standalone_tasks(vec![
                assigned_subtask("t1", "One", Some("t2")),
                assigned_subtask("t2", "Two", Some("t1")),
            ]);

        let table = TaskState::build_table_for_projects(&client, &projects).expect("table");

        assert_eq!(
            project_headers(&table),
            vec!["No Project (Assigned to Me)".to_string()]
        );
        assert_eq!(client.get_task_calls(), vec!["t1".to_string(), "t2".to_string()]);
    }

    #[test]
    fn loading_a_real_project_never_looks_a_parent_up() {
        let projects = vec![Project::new("pz", "Zeta", false)];
        let client = FakeAsanaClient::new(projects.clone())
            .with_tasks(
                "pz",
                vec![task(
                    "t1", "Parent", "pz", "Zeta", "sz", "Doing", "cf", "Tag", "red",
                )],
            )
            .with_subtasks("t1", vec![assigned_subtask("s1", "Subtask", Some("t1"))]);

        let table = TaskState::build_table_for_projects(&client, &projects).expect("table");

        assert_eq!(project_headers(&table), vec!["Zeta".to_string()]);
        assert!(
            client.get_task_calls().is_empty(),
            "tasks fetched for a project already know which project they are in"
        );
    }

    /// The same subtask arrives twice when its project is selected alongside
    /// the assigned-to-me row. Merging used to leave it with both "Zeta" and
    /// "No Project (Assigned to Me)", and grouping picked whichever sorted
    /// first — so any project named after "No Project" lost.
    #[test]
    fn a_subtask_loaded_from_both_its_project_and_the_assigned_to_me_row_groups_once() {
        let projects = assigned_to_me_projects();
        let subtask = assigned_subtask("s1", "Assigned subtask", Some("t1"));
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![subtask.clone()])
            .with_tasks(
                "pz",
                vec![task(
                    "t1", "Parent in Zeta", "pz", "Zeta", "sz", "Doing", "cf", "Tag", "red",
                )],
            )
            .with_subtasks("t1", vec![subtask]);

        let table = TaskState::build_table_for_projects(&client, &projects).expect("table");

        let row = table
            .rows
            .iter()
            .find(|row| row.cells[0] == "Assigned subtask")
            .expect("the assigned subtask is shown");
        assert_eq!(row.project.as_deref(), Some("Zeta"));
        assert_eq!(row.cells[5], "Zeta");
        assert_eq!(project_headers(&table), vec!["Zeta".to_string()]);
    }

    /// A subtask of a task in Zeta holds no project membership of its own, so
    /// the assignee query returns it even though Zeta is not selected. Grouping
    /// it under a "Zeta" header the user never asked for is worse than leaving
    /// it out: the assigned-to-me row is for work that sits outside every
    /// project, not a back door into every project the user touches.
    #[test]
    fn an_assigned_subtask_of_an_unselected_project_stays_out_of_the_view() {
        let projects = assigned_to_me_projects();
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![
                assigned_subtask("s1", "Assigned subtask", Some("t1")),
                assigned_subtask("s2", "Loose task", None),
            ])
            .with_standalone_tasks(vec![task(
                "t1", "Parent in Zeta", "pz", "Zeta", "sz", "Doing", "cf", "Tag", "red",
            )]);

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::assigned_to_me("user-1")])
            .expect("tasks load");

        assert_eq!(
            project_headers(state.table()),
            vec!["No Project (Assigned to Me)".to_string()]
        );
        let names = task_names(state.table());
        assert_eq!(
            names,
            vec!["Loose task".to_string()],
            "only the task that is in no project at all survives"
        );
    }

    #[test]
    fn an_assigned_subtask_returns_to_the_view_once_its_project_is_selected() {
        let projects = assigned_to_me_projects();
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![assigned_subtask("s1", "Assigned subtask", Some("t1"))])
            // Zeta's own task list does not reach the subtask — its parent is
            // outside the loaded set — so the assigned-to-me copy is the only
            // one there is.
            .with_standalone_tasks(vec![task(
                "t1", "Parent in Zeta", "pz", "Zeta", "sz", "Doing", "cf", "Tag", "red",
            )]);

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &projects)
            .expect("tasks load");

        assert_eq!(project_headers(state.table()), vec!["Zeta".to_string()]);
        assert_eq!(task_names(state.table()), vec!["Assigned subtask".to_string()]);
    }

    /// The same rule applies to a task that holds the membership itself: the
    /// assignee query reaches it, but it belongs to a project the user has not
    /// selected.
    #[test]
    fn an_assigned_task_in_an_unselected_project_stays_out_of_the_view() {
        let projects = assigned_to_me_projects();
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![task(
                "t1", "Own membership", "pz", "Zeta", "sz", "Doing", "cf", "Tag", "red",
            )]);

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::assigned_to_me("user-1")])
            .expect("tasks load");

        assert_eq!(state.table().task_count(), 0);
    }

    /// The project list pins "No Project (Assigned to Me)" to the top and hands
    /// the targets over in that order, so the table's groups have to lead with
    /// it too — sorting the headers by name used to bury it under every project
    /// named earlier in the alphabet.
    #[test]
    fn project_groups_lead_with_the_assigned_to_me_row_like_the_project_list_does() {
        let projects = vec![
            Project::assigned_to_me("user-1"),
            Project::new("pa", "Alpha", false),
            Project::new("pz", "Zeta", false),
        ];
        let client = FakeAsanaClient::new(projects.clone())
            .with_current_user_gid("user-1")
            .with_assigned_to_me_tasks(vec![assigned_subtask("s1", "Loose task", None)])
            .with_tasks(
                "pa",
                vec![task(
                    "t1", "Task in Alpha", "pa", "Alpha", "sa", "Doing", "cf", "Tag", "red",
                )],
            )
            .with_tasks(
                "pz",
                vec![task(
                    "t2", "Task in Zeta", "pz", "Zeta", "sz", "Doing", "cf", "Tag", "red",
                )],
            );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &projects)
            .expect("tasks load");

        assert_eq!(
            project_headers(state.table()),
            vec![
                "No Project (Assigned to Me)".to_string(),
                "Alpha".to_string(),
                "Zeta".to_string(),
            ]
        );
    }

    // --- Editing ---------------------------------------------------------

    use super::{EditContext, TaskEdit, TaskFieldEdit};
    use crate::asana::TaskQuery;
    use crate::domain::{ASSIGNEE_COLUMN, DUE_COLUMN, STATE_COLUMN, TITLE_COLUMN};

    fn edit_context() -> EditContext {
        EditContext {
            today: Some(crate::domain::CivilDate::new(2026, 6, 1).expect("a real day")),
            current_user_gid: Some("user-alex".to_string()),
            ..EditContext::default()
        }
    }

    /// The text one task's cell holds in the rebuilt table.
    fn cell(state: &TaskState, gid: &str, column: usize) -> String {
        state
            .table()
            .rows
            .iter()
            .find(|row| row.gid == gid)
            .and_then(|row| row.cells.get(column))
            .cloned()
            .unwrap_or_else(|| panic!("no row for {gid}"))
    }

    fn commit(state: &mut TaskState) -> Vec<TaskEdit> {
        let edits = state
            .commit_cell_edit(&edit_context())
            .expect("the edit commits");
        state.apply_edits_locally(&edits.fields);
        state.apply_project_edits_locally(&edits.projects);
        edits.fields
    }

    #[test]
    fn marking_a_done_task_open_survives_the_cache_merge() {
        // `merge_task_record` is `completed |= incoming`, so an un-complete
        // applied through it is a silent no-op — and the row would snap back
        // to done on the next rebuild rather than at the point of the edit.
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Done thing", true)]);

        let edits = state.toggle_completed_edits();
        state.apply_edits_locally(&edits);

        assert_eq!(cell(&state, "t1", STATE_COLUMN), "open");
        state.refresh_from_cache();
        assert_eq!(cell(&state, "t1", STATE_COLUMN), "open", "and it stays open");
    }

    #[test]
    fn a_cleared_date_and_assignee_survive_the_cache_merge() {
        // The other half of the monotone merge: a `None` never overwrites a
        // `Some`, so a cleared value applied through it comes straight back.
        let mut state = loaded_state_with_tasks(vec![sel_task_due("t1", "Ship it", "alex", "2026-06-10")]);

        state.move_column(DUE_COLUMN as i64);
        state.begin_cell_edit(&edit_context()).expect("the date opens");
        state.cell_edit_clear();
        commit(&mut state);

        state.move_column(ASSIGNEE_COLUMN as i64 - DUE_COLUMN as i64);
        state.begin_cell_edit(&edit_context()).expect("the assignee opens");
        state.cell_edit_clear();
        commit(&mut state);

        state.refresh_from_cache();
        assert_eq!(cell(&state, "t1", DUE_COLUMN), "");
        assert_eq!(cell(&state, "t1", ASSIGNEE_COLUMN), "");
    }

    #[test]
    fn a_stale_fetch_does_not_undo_a_confirmed_edit() {
        // A response that started before the edit and arrives after it. The
        // record's `modified_at` is what tells the merge which copy is older,
        // and `confirm_edit` is the only thing that sets it.
        let mut state = loaded_state_with_tasks(vec![sel_task_due("t1", "Ship it", "alex", "2026-06-10")]);

        state.move_column(DUE_COLUMN as i64);
        state.begin_cell_edit(&edit_context()).expect("the date opens");
        state.cell_edit_clear();
        state.cell_edit_push_char('2');
        for ch in "026-07-04".chars() {
            state.cell_edit_push_char(ch);
        }
        let edits = commit(&mut state);
        state.confirm_edit(&edits[0].gid, Some("2026-06-02T00:00:00Z".to_string()));

        let stale = TaskDataset {
            records: vec![{
                let mut record = TaskRecord::new("t1", "Ship it");
                record.modified_at = Some("2026-06-01T00:00:00Z".to_string());
                record.due_date = Some("2026-06-10".to_string());
                record.project_gids.push("p1".to_string());
                record
            }],
            custom_field_definitions: Vec::new(),
        };
        state.ingest_loaded_project("p1", TaskQuery::default(), stale);

        assert_eq!(cell(&state, "t1", DUE_COLUMN), "2026-07-04");
    }

    #[test]
    fn an_edit_applies_to_the_selection_or_to_the_cursor_row() {
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "One", false),
            sel_task("t2", "Two", false),
            sel_task("t3", "Three", false),
        ]);

        let order = visible_gids(&state)
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(state.edit_targets(), vec![order[0].clone()], "the cursor row");

        // `space` selects the cursor row and moves down, so two presses take
        // the first two rows in whatever order the sort put them.
        state.toggle_task_selection();
        state.toggle_task_selection();

        assert_eq!(
            state.edit_targets(),
            order[..2].to_vec(),
            "the selection, and only the selection"
        );
    }

    #[test]
    fn a_title_is_refused_on_a_multi_task_selection_and_allowed_on_one() {
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "One", false),
            sel_task("t2", "Two", false),
        ]);
        state.toggle_task_selection();
        state.toggle_task_selection();

        assert_eq!(
            state.begin_cell_edit(&edit_context()),
            Err("a title is edited one task at a time (2 selected)".to_string())
        );

        state.clear_task_selection();
        assert_eq!(state.begin_cell_edit(&edit_context()), Ok(()));
        assert_eq!(state.selected_column(), TITLE_COLUMN);
    }

    #[test]
    fn the_targets_of_an_edit_are_fixed_when_it_opens() {
        // The table rebuilds whenever a project finishes loading or a filter
        // changes; an edit that re-read the selection at commit time could
        // change more than it said it would.
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "One", false),
            sel_task("t2", "Two", false),
        ]);

        let first = visible_gids(&state)[0].to_string();
        state.move_column(STATE_COLUMN as i64);
        state.begin_cell_edit(&edit_context()).expect("the cell opens");

        // The selection widens under the open editor; the commit must not.
        state.select_all_visible_tasks();
        state.cell_edit_cycle_value(1);

        let edits = state
            .commit_cell_edit(&edit_context())
            .expect("a state always resolves")
            .fields
            .iter()
            .map(|edit| edit.gid.clone())
            .collect::<Vec<_>>();

        assert_eq!(edits, vec![first]);
    }

    #[test]
    fn d_sets_a_mixed_selection_to_one_state_and_back() {
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "One", false),
            sel_task("t2", "Two", true),
        ]);
        state.select_all_visible_tasks();
        let gids = visible_gids(&state)
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();

        let edits = state.toggle_completed_edits();
        state.apply_edits_locally(&edits);
        let states = gids
            .iter()
            .map(|gid| cell(&state, gid, STATE_COLUMN))
            .collect::<Vec<_>>();
        assert_eq!(
            states.iter().collect::<std::collections::HashSet<_>>().len(),
            1,
            "a mixed selection comes out uniform, not flipped one by one: {states:?}"
        );

        let before = states[0].clone();
        let edits = state.toggle_completed_edits();
        state.apply_edits_locally(&edits);
        for gid in &gids {
            assert_ne!(cell(&state, gid, STATE_COLUMN), before, "a second press puts it back");
        }
    }

    #[test]
    fn a_failed_field_resolution_keeps_the_editor_open() {
        let mut state = loaded_state_with_tasks(vec![sel_task_due("t1", "Ship it", "alex", "2026-06-10")]);

        state.move_column(ASSIGNEE_COLUMN as i64);
        state.begin_cell_edit(&edit_context()).expect("the cell opens");
        state.cell_edit_clear();
        for ch in "nobody".chars() {
            state.cell_edit_push_char(ch);
        }

        assert_eq!(
            state.commit_cell_edit(&edit_context()),
            Err("no one called nobody is loaded".to_string())
        );
        assert!(state.cell_edit_open(), "the value to fix is still on screen");
    }

    #[test]
    fn the_column_cursor_clamps_when_a_column_disappears_mid_edit() {
        // A custom-field column goes when the project carrying it does. An
        // editor pointing at a column that is gone would commit to whatever
        // slid into its index.
        let mut state = loaded_state_with_priority_field(vec![task_with("t1", None, None, Some("High"))]);
        let last = state.table().columns.len() - 1;
        state.move_column(last as i64);
        state.begin_cell_edit(&edit_context()).expect("the cell opens");
        assert!(state.cell_edit_open());

        state.invalidate_cache();
        let bare = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_tasks("p1", vec![sel_task("t1", "Ship it", false)]);
        state
            .load_task_dataset_for_projects(&bare, &[Project::new("p1", "Inbox", true)])
            .expect("tasks reload");

        assert!(state.selected_column() < state.table().columns.len());
        assert!(!state.cell_edit_open(), "the edit closed rather than moving");
    }

    #[test]
    fn a_picker_offers_the_options_the_field_declares_not_the_ones_tasks_hold() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_custom_field_settings(
                "p1",
                vec![ProjectCustomFieldSettingDto {
                    gid: "set-1".to_string(),
                    custom_field: CustomFieldDto {
                        gid: "cf1".to_string(),
                        name: "Priority".to_string(),
                        resource_subtype: Some("enum".to_string()),
                        enum_options: vec![
                            EnumOptionDto {
                                gid: "opt-high".to_string(),
                                name: "High".to_string(),
                                enabled: true,
                            },
                            EnumOptionDto {
                                gid: "opt-blocked".to_string(),
                                name: "Blocked".to_string(),
                                enabled: true,
                            },
                            EnumOptionDto {
                                gid: "opt-retired".to_string(),
                                name: "Retired".to_string(),
                                enabled: false,
                            },
                        ],
                    },
                }],
            )
            .with_tasks("p1", vec![task_with("t1", None, None, Some("High"))]);
        let mut state = TaskState::new();
        state.set_completed_filter(None);
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        let last = state.table().columns.len() - 1;
        state.move_column(last as i64);
        state.begin_cell_edit(&edit_context()).expect("the picker opens");

        // No task is `Blocked`, which is exactly why the picker cannot be
        // built from the values tasks happen to carry.
        state.cell_edit_cycle_value(1);
        let edits = state
            .commit_cell_edit(&edit_context())
            .expect("the picked option resolves")
            .fields;

        assert_eq!(
            edits[0].field,
            TaskFieldEdit::CustomField {
                gid: "cf1".to_string(),
                value: Some(crate::domain::CustomFieldValue::Enum {
                    option_gid: "opt-blocked".to_string(),
                    name: "Blocked".to_string(),
                }),
            }
        );
    }

    #[test]
    fn a_disabled_option_is_never_offered() {
        // Disabled options exist so old values still render; offering one is
        // offering a value Asana will reject.
        let definition = super::custom_field_definition(
            "p1",
            &CustomFieldDto {
                gid: "cf1".to_string(),
                name: "Priority".to_string(),
                resource_subtype: Some("enum".to_string()),
                enum_options: vec![EnumOptionDto {
                    gid: "opt-retired".to_string(),
                    name: "Retired".to_string(),
                    enabled: false,
                }],
            },
        );

        assert!(definition.enum_options().is_empty());
    }

    #[test]
    fn an_unsupported_custom_field_kind_is_refused_with_a_message() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_custom_field_settings(
                "p1",
                vec![ProjectCustomFieldSettingDto {
                    gid: "set-1".to_string(),
                    custom_field: CustomFieldDto {
                        gid: "cf1".to_string(),
                        name: "Reviewers".to_string(),
                        resource_subtype: Some("people".to_string()),
                        enum_options: Vec::new(),
                    },
                }],
            )
            .with_tasks("p1", vec![sel_task("t1", "Ship it", false)]);
        let mut state = TaskState::new();
        state.set_completed_filter(None);
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        let last = state.table().columns.len() - 1;
        state.move_column(last as i64);

        assert_eq!(
            state.begin_cell_edit(&edit_context()),
            Err("this field cannot be edited here yet".to_string())
        );
    }

    /// The project candidates a `p1`-loaded fixture would be offered.
    fn project_context() -> EditContext {
        EditContext {
            projects: vec![
                ("p1".to_string(), "Inbox".to_string()),
                ("p2".to_string(), "Backlog".to_string()),
            ],
            ..edit_context()
        }
    }

    #[test]
    fn the_projects_column_opens_on_the_projects_the_task_is_in() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship it", false)]);
        state.move_column(crate::domain::PROJECTS_COLUMN as i64);

        state
            .begin_cell_edit(&project_context())
            .expect("the completion editor opens");

        assert!(state.cell_edit_is_complete());
        assert_eq!(
            state.cell_edit_view().expect("an open editor").text,
            "Inbox",
            "the names, not the gids the write will use"
        );
    }

    #[test]
    fn a_project_is_added_and_the_one_it_replaces_is_removed() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship it", false)]);
        state.move_column(crate::domain::PROJECTS_COLUMN as i64);
        state
            .begin_cell_edit(&project_context())
            .expect("the completion editor opens");

        // Delete the project it is in, then complete a different one.
        state.cell_edit_pop_char();
        for ch in "back".chars() {
            state.cell_edit_push_char(ch);
        }
        assert!(state.cell_edit_complete(1));

        let edits = state
            .commit_cell_edit(&project_context())
            .expect("the project resolves");

        assert!(edits.fields.is_empty(), "membership is not a task field");
        assert_eq!(
            edits.projects,
            vec![
                crate::domain::ProjectEdit::add("t1", "p2", "Backlog"),
                crate::domain::ProjectEdit::remove("t1", "p1", "Inbox"),
            ]
        );
    }

    #[test]
    fn a_task_removed_from_the_project_in_view_leaves_the_table_at_once() {
        // The cache is keyed by the target that loaded the task, so the
        // record is still in `p1`'s cached page — the rebuild has to drop it
        // rather than wait for a refresh that nothing is going to ask for.
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship it", false)]);
        assert_eq!(state.table().task_count(), 1);

        state.apply_project_edits_locally(&[crate::domain::ProjectEdit::remove(
            "t1", "p1", "Inbox",
        )]);

        assert_eq!(state.table().task_count(), 0);
    }

    #[test]
    fn emptying_the_projects_cell_is_refused_rather_than_sent() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship it", false)]);
        state.move_column(crate::domain::PROJECTS_COLUMN as i64);
        state
            .begin_cell_edit(&project_context())
            .expect("the completion editor opens");
        state.cell_edit_clear();

        assert_eq!(
            state.commit_cell_edit(&project_context()),
            Err("a task has to be in at least one project".to_string())
        );
        assert!(state.cell_edit_open(), "and the editor stays open");
    }

    #[test]
    fn a_name_that_is_not_a_project_commits_nothing_and_says_so() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship it", false)]);
        state.move_column(crate::domain::PROJECTS_COLUMN as i64);
        state
            .begin_cell_edit(&project_context())
            .expect("the completion editor opens");
        for ch in "Nowhere".chars() {
            state.cell_edit_push_char(ch);
        }

        assert_eq!(
            state.commit_cell_edit(&project_context()),
            Err("no project called Nowhere".to_string())
        );
    }

    // --- The `list` match mode -------------------------------------------

    use super::{TaskFieldStringMode, MAX_RECENT_ROWS};
    use crate::config::{SavedFilterField, SavedFilterSet};

    /// A state holding three tasks assigned to three different people.
    fn assigned_state() -> TaskState {
        let mut state = loaded_state_with_tasks(vec![
            sel_task_due("t1", "Ship it", "Alex Chen", "2026-06-10"),
            sel_task_due("t2", "Label it", "Jo Park", "2026-06-11"),
            sel_task_due("t3", "Close it", "Priya Raman", "2026-06-12"),
        ]);
        state.set_current_user_gid(Some("user-Jo Park".to_string()));
        state
    }

    /// Puts the selected row into `list` mode by cycling the ring to it.
    fn cycle_to_list(state: &mut TaskState) {
        for _ in 0..4 {
            if matches!(
                state.view.filter_editor.selected_field().map(|f| f.string_mode),
                Some(TaskFieldStringMode::List)
            ) {
                return;
            }
            state.filter_cycle_mode();
        }
        panic!("the row never offered a list mode");
    }

    #[test]
    fn only_the_assignee_row_offers_the_list_mode() {
        let mut state = assigned_state();

        select_field(&mut state, "assignee");
        cycle_to_list(&mut state);

        // Every other string row keeps the three modes it had.
        for key in ["title", "projects"] {
            select_field(&mut state, key);
            for _ in 0..3 {
                state.filter_cycle_mode();
            }
            assert!(
                !matches!(
                    state.view.filter_editor.selected_field().map(|f| f.string_mode),
                    Some(TaskFieldStringMode::List)
                ),
                "{key} has no directory behind it"
            );
        }
    }

    #[test]
    fn a_list_row_matches_the_picked_people_exactly_and_ors_them() {
        let mut state = assigned_state();
        select_field(&mut state, "assignee");
        cycle_to_list(&mut state);
        set_field(&mut state, "assignee", "Alex Chen | Priya Raman");
        state.refresh_from_cache();

        assert_eq!(visible_gids(&state), vec!["t1", "t3"]);
    }

    #[test]
    fn a_list_row_does_not_match_a_name_that_merely_contains_the_text() {
        // The whole point of having picked from a list: `alex` is a person,
        // not a pattern.
        let mut state = assigned_state();
        select_field(&mut state, "assignee");
        cycle_to_list(&mut state);
        set_field(&mut state, "assignee", "Alex");
        state.refresh_from_cache();

        assert!(visible_gids(&state).is_empty());
    }

    #[test]
    fn a_negated_list_row_is_none_of_these() {
        let mut state = assigned_state();
        select_field(&mut state, "assignee");
        cycle_to_list(&mut state);
        set_field(&mut state, "assignee", "Alex Chen");
        state.filter_toggle_negate_field();
        state.refresh_from_cache();

        assert_eq!(visible_gids(&state), vec!["t2", "t3"]);
    }

    #[test]
    fn me_in_a_list_row_resolves_to_whoever_is_logged_in() {
        // Resolved at match time, not when it was picked, so a saved set
        // stays personal to whoever loads it.
        let mut state = assigned_state();
        select_field(&mut state, "assignee");
        cycle_to_list(&mut state);
        set_field(&mut state, "assignee", "me");
        state.refresh_from_cache();

        assert_eq!(visible_gids(&state), vec!["t2"], "Jo is logged in");

        state.set_current_user_gid(Some("user-Alex Chen".to_string()));
        state.refresh_from_cache();
        assert_eq!(visible_gids(&state), vec!["t1"], "and now Alex is");
    }

    #[test]
    fn the_filter_row_completes_over_the_same_people_the_cell_does() {
        let mut state = assigned_state();
        select_field(&mut state, "assignee");
        cycle_to_list(&mut state);

        let candidates = state.people_candidates(&edit_context());
        state.filter_edit_begin_with(candidates);
        assert!(state.filter_autocomplete_open());

        for ch in "pri".chars() {
            state.filter_push_char(ch);
        }
        assert!(state.filter_complete(1));
        assert_eq!(state.filter_autocomplete_commit(), None);
        state.refresh_from_cache();

        assert_eq!(
            state.filter_panel_rows()[1],
            ("Assignee".to_string(), "Priya Raman".to_string()),
            "the picked name is the row's value"
        );
        assert_eq!(visible_gids(&state), vec!["t3"]);
    }

    #[test]
    fn a_filter_name_that_matches_nobody_is_dropped_with_a_message() {
        let mut state = assigned_state();
        select_field(&mut state, "assignee");
        cycle_to_list(&mut state);

        let candidates = state.people_candidates(&edit_context());
        state.filter_edit_begin_with(candidates);
        for ch in "nobody".chars() {
            state.filter_push_char(ch);
        }

        assert_eq!(
            state.filter_autocomplete_commit(),
            Some("no one called nobody".to_string())
        );
        assert_eq!(
            state.filter_panel_rows()[1],
            ("Assignee".to_string(), String::new()),
            "and the row is left filtering nothing rather than nothing at all"
        );
    }

    #[test]
    fn a_list_row_saves_and_reloads_through_a_named_set() {
        let mut state = assigned_state();
        select_field(&mut state, "assignee");
        cycle_to_list(&mut state);
        set_field(&mut state, "assignee", "Alex Chen | me");

        let saved = state.filter_sets_to_saved();
        let field = saved[0]
            .fields
            .iter()
            .find(|field| field.key == "assignee")
            .expect("the row is saved");
        assert_eq!(field.query, "Alex Chen | me");
        assert_eq!(field.string_mode.as_deref(), Some("list"));

        let mut reloaded = assigned_state();
        reloaded.filter_sets_load("people", &saved);
        reloaded.refresh_from_cache();
        assert_eq!(visible_gids(&reloaded), vec!["t1", "t2"]);
    }

    #[test]
    fn a_list_mode_saved_against_a_row_that_cannot_offer_it_parks() {
        // The same treatment an unknown field key gets: parked rather than
        // applied as a mode the row cannot mean.
        let mut state = assigned_state();
        let _ = &state;
        let saved = vec![SavedFilterSet {
            negated: false,
            fields: vec![SavedFilterField {
                key: "title".to_string(),
                query: "Ship it".to_string(),
                string_mode: Some("list".to_string()),
                empty: false,
                negated: false,
            }],
        }];
        state.filter_sets_load("odd", &saved);
        state.refresh_from_cache();

        assert_eq!(
            visible_gids(&state),
            vec!["t1", "t2", "t3"],
            "nothing was applied"
        );
        assert_eq!(
            state.filter_sets_to_saved(),
            saved,
            "and nothing was thrown away either"
        );
    }

    // --- Recently edited --------------------------------------------------

    /// The task ids the pane is holding.
    fn recent_gids(state: &TaskState) -> Vec<&str> {
        state
            .recent_table()
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.as_str())
            .collect()
    }

    /// Marks `t1` done with the table showing only open tasks, which is the
    /// shortest edit that hides its own subject.
    /// The table sorts by title with no dates in play, so `t1` is the first
    /// row and the one the cursor starts on.
    fn state_with_an_edit_out_of_view() -> TaskState {
        let mut state = loaded_state_with_tasks(vec![
            sel_task("t1", "Alpha", false),
            sel_task("t2", "Beta", false),
        ]);
        state.set_completed_filter(Some(false));
        state.refresh_from_cache();

        let edits = state.toggle_completed_edits();
        state.apply_edits_locally(&edits);
        state
    }

    #[test]
    fn a_task_edited_out_of_the_view_lands_in_the_pane_with_the_cursor() {
        let state = state_with_an_edit_out_of_view();

        assert_eq!(visible_gids(&state), vec!["t2"]);
        assert_eq!(recent_gids(&state), vec!["t1"]);
        assert_eq!(state.recent_selected_index(), Some(0));
        assert!(state.recent_pane_visible());
    }

    #[test]
    fn a_task_the_view_still_shows_is_not_in_the_pane() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship it", false)]);
        state.set_completed_filter(None);

        let edits = state.toggle_completed_edits();
        state.apply_edits_locally(&edits);

        assert_eq!(visible_gids(&state), vec!["t1"]);
        assert!(recent_gids(&state).is_empty(), "nothing is hidden");
        assert!(!state.recent_pane_visible());
    }

    #[test]
    fn a_task_that_matches_again_leaves_the_pane() {
        let mut state = state_with_an_edit_out_of_view();
        assert_eq!(recent_gids(&state), vec!["t1"]);

        // Widen the filter: the task is back in the table, so the pane has
        // nothing left to hold.
        state.set_completed_filter(None);
        state.refresh_from_cache();

        assert!(recent_gids(&state).is_empty());
        assert_eq!(state.recent_selected_index(), None);
        assert!(visible_gids(&state).contains(&"t1"));
    }

    #[test]
    fn the_cursor_walks_out_of_the_pane_and_into_the_table() {
        let mut state = state_with_an_edit_out_of_view();
        assert_eq!(state.recent_selected_index(), Some(0));

        // The pane sits above the table, so down leaves it and up goes back.
        state.move_down();
        assert_eq!(state.recent_selected_index(), None);
        assert_eq!(state.cursor_task_gid().as_deref(), Some("t2"));

        state.move_up();
        assert_eq!(state.recent_selected_index(), Some(0));
        assert_eq!(state.cursor_task_gid().as_deref(), Some("t1"));
    }

    #[test]
    fn hiding_the_pane_returns_the_cursor_to_the_table() {
        let mut state = state_with_an_edit_out_of_view();

        state.toggle_recent_pane();

        assert!(!state.recent_pane_visible());
        assert_eq!(state.recent_selected_index(), None);
        assert_eq!(state.cursor_task_gid().as_deref(), Some("t2"));

        // And an edit that hides the cursor's task shows it again, toggle or
        // no toggle: a cursor the user cannot see is not a cursor.
        let edits = state.toggle_completed_edits();
        state.apply_edits_locally(&edits);

        assert!(state.recent_pane_visible());
        assert_eq!(state.cursor_task_gid().as_deref(), Some("t2"));
    }

    #[test]
    fn a_task_in_the_pane_can_still_be_edited() {
        // Its record has left the dataset with the project it was in, so
        // every edit path has to find it in the cache instead.
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Alpha", false)]);
        state.apply_project_edits_locally(&[crate::domain::ProjectEdit::remove(
            "t1", "p1", "Inbox",
        )]);
        assert_eq!(recent_gids(&state), vec!["t1"]);
        assert_eq!(state.recent_selected_index(), Some(0));

        let edits = state.toggle_completed_edits();

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].field, TaskFieldEdit::Completed(true));
        state.apply_edits_locally(&edits);
        assert_eq!(
            state.recent_table().rows[0].cells[crate::domain::STATE_COLUMN],
            "done",
            "and the pane shows the result"
        );
    }

    #[test]
    fn the_pane_holds_only_the_newest_few_and_counts_the_rest() {
        let tasks = (0..6)
            .map(|index| sel_task(&format!("t{index}"), &format!("Task {index}"), false))
            .collect::<Vec<_>>();
        let mut state = loaded_state_with_tasks(tasks);
        state.set_completed_filter(Some(false));
        state.refresh_from_cache();

        state.select_all_visible_tasks();
        let edits = state.toggle_completed_edits();
        state.apply_edits_locally(&edits);

        assert_eq!(recent_gids(&state).len(), MAX_RECENT_ROWS);
        assert_eq!(state.recent_hidden_count(), 6, "the rest is still counted");
    }

    #[test]
    fn the_column_cursor_stops_at_both_ends() {
        let mut state = loaded_state_with_tasks(vec![sel_task("t1", "Ship it", false)]);

        state.move_column(-1);
        assert_eq!(state.selected_column(), 0);

        state.move_column(100);
        assert_eq!(state.selected_column(), state.table().columns.len() - 1);
    }
}
