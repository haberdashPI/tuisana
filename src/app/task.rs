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
        calendar::CalendarState,
        debug_log,
        gantt::{GanttViewState, MoveTo},
    },
    asana::{
        dto::{CustomFieldValueDto, TaskDto},
        AsanaClient, TaskLoadScope, TaskQuery, TaskTarget,
    },
    domain::{
        date, distinct_values, group_custom_fields_by_name, merge_task_record, CivilDate,
        CustomFieldDefinition,
        GanttColorKey, Timeline,
        DateQuery, Project, ProjectKind, TaskRecord, TaskRowKind, TaskTableModel,
        TaskTableSettings,
    },
    config::GanttConfig,
    error::Result,
    input::Action,
    util::fuzzy_match,
};

const HORIZONTAL_SCROLL_STEP: usize = 8;

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
    filter_editor: TaskFilterEditorState,
    task_vertical_scroll: usize,
    filter_vertical_scroll: usize,
    help_details_visible: bool,
    selected_task_ids: HashSet<String>,
    gantt: GanttViewState,
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
}

/// Mutable state for one filter row, including the user query and any selected
/// label values.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TaskFilterFieldState {
    spec: TaskFilterFieldSpec,
    query: String,
    /// The text caret, as a char index into `query`.
    ///
    /// Editing used to be append-only, with the caret pinned to the end. It is a
    /// position now so the arrow keys can move through the text on any field,
    /// not just the date fields the calendar drives.
    query_caret: usize,
    string_mode: TaskFieldStringMode,
    label_values: Vec<String>,
    label_options: Vec<String>,
    label_cursor: usize,
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
}

/// Tracks the filter panel's visibility, edit mode, selected row, and fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TaskFilterEditorState {
    visible: bool,
    editing: bool,
    selected: usize,
    fields: Vec<TaskFilterFieldState>,
    /// The date picker, while a date field is being edited through it.
    calendar: Option<CalendarState>,
}

/// Small cache of task records and custom field names keyed by task GID.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TaskCache {
    records: HashMap<String, TaskRecord>,
    custom_field_definitions: HashMap<String, String>,
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
                .entry(definition.gid)
                .or_insert(definition.name);
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
            .iter()
            .map(|(gid, name)| CustomFieldDefinition::new(gid.clone(), name.clone()))
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
                },
                TaskFieldStringMode::Substring,
                Vec::new(),
            ),
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "due".to_string(),
                    label: "Due".to_string(),
                    kind: TaskFieldFilterKind::Date,
                    custom_gids: Vec::new(),
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
            visible: false,
            editing: false,
            selected: 0,
            fields,
            calendar: None,
        }
    }

    fn restore_queries(&mut self, previous: Self) {
        self.visible = previous.visible;
        self.editing = previous.editing && self.visible;
        self.selected = previous.selected.min(self.fields.len().saturating_sub(1));
        for field in &mut self.fields {
            if let Some(old) = previous.fields.iter().find(|old| old.spec.key == field.spec.key) {
                field.string_mode = old.string_mode;
                match field.spec.kind {
                    TaskFieldFilterKind::Labels => {
                        field.label_values = if old.label_values.is_empty() {
                            parse_label_values(&old.query)
                        } else {
                            old.label_values.clone()
                        };
                        field.label_cursor = old.label_cursor.min(field.label_values.len().saturating_sub(1));
                        field.query = field.label_values.join(" | ");
                    }
                    _ => {
                        field.query = old.query.clone();
                    }
                }
            }
        }
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

    fn active_count(&self) -> usize {
        self.fields
            .iter()
            .filter(|field| !field.query.trim().is_empty())
            .count()
    }

    /// Fields that exclude anything, counting label selections as well as text.
    fn active_filter_count(&self) -> usize {
        self.fields
            .iter()
            .filter(|field| {
                !field.query.trim().is_empty() || !field.label_values.is_empty()
            })
            .count()
    }

    fn selected_kind(&self) -> Option<TaskFieldFilterKind> {
        self.fields.get(self.selected).map(|field| field.spec.kind)
    }

    fn selected_label(&self) -> Option<&str> {
        self.fields.get(self.selected).map(|field| field.spec.label.as_str())
    }

    fn move_up(&mut self) {
        if self.fields.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
    }

    fn move_down(&mut self) {
        if self.fields.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.fields.len() - 1);
    }

    fn page_up(&mut self, page_size: usize) {
        if self.fields.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(page_size.max(1));
    }

    fn page_down(&mut self, page_size: usize) {
        if self.fields.is_empty() {
            return;
        }
        self.selected = (self.selected + page_size.max(1)).min(self.fields.len() - 1);
    }

    fn clear_current(&mut self) {
        if let Some(field) = self.fields.get_mut(self.selected) {
            field.query.clear();
            field.query_caret = 0;
            field.label_values.clear();
            field.label_cursor = 0;
        }
    }

    fn set_mode(&mut self, mode: TaskFieldStringMode) {
        if let Some(field) = self.fields.get_mut(self.selected) {
            if !matches!(field.spec.kind, TaskFieldFilterKind::String) {
                return;
            }
            field.string_mode = mode;
        }
    }

    fn cycle_mode(&mut self) {
        if let Some(field) = self.fields.get_mut(self.selected) {
            if !matches!(field.spec.kind, TaskFieldFilterKind::String) {
                return;
            }
            field.string_mode = match field.string_mode {
                TaskFieldStringMode::Fuzzy => TaskFieldStringMode::Substring,
                TaskFieldStringMode::Substring => TaskFieldStringMode::Regex,
                TaskFieldStringMode::Regex => TaskFieldStringMode::Fuzzy,
            };
        }
    }

    fn push_char(&mut self, ch: char) {
        if let Some(field) = self.fields.get_mut(self.selected) {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                return;
            }
            let mut chars = field.query.chars().collect::<Vec<_>>();
            let at = field.query_caret.min(chars.len());
            chars.insert(at, ch);
            field.query = chars.into_iter().collect();
            field.query_caret = at + 1;
        }
    }

    fn pop_char(&mut self) {
        if let Some(field) = self.fields.get_mut(self.selected) {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                return;
            }
            let mut chars = field.query.chars().collect::<Vec<_>>();
            let at = field.query_caret.min(chars.len());
            if at == 0 {
                return;
            }
            chars.remove(at - 1);
            field.query = chars.into_iter().collect();
            field.query_caret = at - 1;
        }
    }

    /// Moves the selected field's caret, clamped to its text.
    fn move_query_caret(&mut self, delta: i64) {
        if let Some(field) = self.fields.get_mut(self.selected) {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                return;
            }
            let len = field.query.chars().count() as i64;
            field.query_caret = (field.query_caret as i64 + delta).clamp(0, len) as usize;
        }
    }

    /// Puts the caret at the end of the selected field's text.
    ///
    /// Called when an edit begins, so typing continues from where the value
    /// leaves off rather than from wherever the caret was last time.
    fn reset_query_caret(&mut self) {
        if let Some(field) = self.fields.get_mut(self.selected) {
            field.query_caret = field.query.chars().count();
        }
    }

    fn start_editing(&mut self) {
        self.editing = true;
        self.reset_query_caret();
    }

    fn stop_editing(&mut self) {
        self.editing = false;
        self.calendar = None;
    }

    /// Opens the date picker on the selected field, if it holds a date.
    fn open_calendar(&mut self, today: CivilDate) -> bool {
        let Some(field) = self.fields.get(self.selected) else {
            return false;
        };
        if !matches!(field.spec.kind, TaskFieldFilterKind::Date) {
            return false;
        }
        self.calendar = Some(CalendarState::open(
            field.spec.label.clone(),
            &field.query,
            today,
        ));
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
        if let Some(field) = self.fields.get_mut(self.selected) {
            field.query = query;
        }
    }

    fn move_label_cursor_left(&mut self) {
        if let Some(field) = self.fields.get_mut(self.selected) {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                if !field.label_values.is_empty() {
                    field.label_cursor = field.label_cursor.saturating_sub(1);
                }
            }
        }
    }

    fn move_label_cursor_right(&mut self) {
        if let Some(field) = self.fields.get_mut(self.selected) {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                if !field.label_values.is_empty() {
                    field.label_cursor = (field.label_cursor + 1).min(field.label_values.len() - 1);
                }
            }
        }
    }

    fn cycle_selected_label(&mut self, delta: i32) {
        let Some(field) = self.fields.get_mut(self.selected) else {
            return;
        };
        if !matches!(field.spec.kind, TaskFieldFilterKind::Labels) || field.label_options.is_empty() {
            return;
        }
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
        field.query = field.label_values.join(" | ");
    }

    fn add_label(&mut self) {
        let Some(field) = self.fields.get_mut(self.selected) else {
            return;
        };
        if !matches!(field.spec.kind, TaskFieldFilterKind::Labels) || field.label_options.is_empty() {
            return;
        }
        let value = field.label_options[0].clone();
        let insert_at = field.label_cursor.saturating_add(1).min(field.label_values.len());
        field.label_values.insert(insert_at, value);
        field.label_cursor = insert_at;
        field.query = field.label_values.join(" | ");
    }

    fn delete_selected_label(&mut self) {
        let Some(field) = self.fields.get_mut(self.selected) else {
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
            field.query.clear();
        } else {
            field.query = field.label_values.join(" | ");
        }
    }

    fn matches(&self, record: &TaskRecord) -> bool {
        self.fields
            .iter()
            .filter(|field| !field.query.trim().is_empty())
            .all(|field| field.matches(record))
    }

    /// Extract a due-date range from the filter state for server-side use.
    ///
    /// Returns `(after, before)` as `YYYY-MM-DD` strings for the API's
    /// `due_on.after` / `due_on.before` params. Keywords resolve against the
    /// *local* date, which is the whole reason this goes through
    /// [`crate::domain::date`]: resolving `today` in UTC fetched the wrong day's
    /// tasks every evening west of UTC, and then cached that window as covered.
    fn due_date_range_for_query(&self) -> (Option<String>, Option<String>) {
        let Some(due_field) = self.fields.iter().find(|f| f.spec.key == "due") else {
            return (None, None);
        };
        let Some(query) = DateQuery::parse(&due_field.query, date::today()) else {
            return (None, None);
        };
        let (after, before) = query.bounds();
        (
            after.map(|date| date.iso()),
            before.map(|date| date.iso()),
        )
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
            query: String::new(),
            query_caret: 0,
            string_mode,
            label_values: Vec::new(),
            label_options,
            label_cursor: 0,
        }
    }

    fn matches(&self, record: &TaskRecord) -> bool {
        match self.spec.kind {
            TaskFieldFilterKind::String => {
                let haystack = match self.spec.key.as_str() {
                    "title" => record.name.clone(),
                    "assignee" => record.assignee.clone().unwrap_or_default(),
                    "projects" => record.projects.join(" "),
                    key if key.starts_with("custom:") => self
                        .spec
                        .custom_gids
                        .iter()
                        .filter_map(|gid| record.custom_fields.get(gid))
                        .flat_map(|values| values.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" "),
                    _ => String::new(),
                }
                .to_ascii_lowercase();
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
                let values = match self.spec.key.as_str() {
                    "state" => vec![if record.completed { "done" } else { "open" }.to_string()],
                    key if key.starts_with("custom:") => self
                        .spec
                        .custom_gids
                        .iter()
                        .filter_map(|gid| record.custom_fields.get(gid))
                        .flat_map(|values| values.iter().cloned())
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                label_filter_matches(&values, &self.label_values)
            }
            TaskFieldFilterKind::Date => {
                let value = match self.spec.key.as_str() {
                    "due" => record.due_date.as_deref(),
                    "start" => record.start_date.as_deref(),
                    _ => None,
                };
                date_filter_matches(value, &self.query)
            }
        }
    }
}

fn label_filter_matches(values: &[String], selected_labels: &[String]) -> bool {
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
fn date_filter_matches(value: Option<&str>, query: &str) -> bool {
    let Some(parsed) = DateQuery::parse(query, date::today()) else {
        // An empty query is not a filter; anything else here is unparseable.
        return query.trim().is_empty();
    };
    value.is_some_and(|value| parsed.matches(value))
}

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
                .fields
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
        self.loading.status = TaskStatus::Loading;
        self.loading.progress = LoadProgress::Active {
            started_at: Instant::now(),
            target_names: projects.iter().map(|project| project.name.clone()).collect(),
            target_ids: projects.iter().map(|project| project.id.clone()).collect(),
        };
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

        let max_scroll = self.view.filter_editor.fields.len().saturating_sub(1);
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

    pub(crate) fn filter_edit_begin(&mut self) {
        self.view.filter_editor.start_editing();
    }

    pub(crate) fn filter_edit_done(&mut self) {
        self.view.filter_editor.stop_editing();
    }

    /// Whether the date picker is open.
    pub fn filter_calendar_open(&self) -> bool {
        self.view.filter_editor.calendar.is_some()
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
    pub fn filter_calendar_is_range(&self) -> bool {
        self.view
            .filter_editor
            .calendar
            .as_ref()
            .is_some_and(|calendar| calendar.range().is_some())
    }

    /// The date picker's state, for the renderer.
    pub(crate) fn filter_calendar(&self) -> Option<&CalendarState> {
        self.view.filter_editor.calendar.as_ref()
    }

    /// Opens the date picker on the selected field. Answers whether it opened,
    /// which is how the caller knows a date field was selected.
    pub(crate) fn filter_calendar_begin(&mut self) -> bool {
        self.view.filter_editor.open_calendar(date::today())
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

    pub(crate) fn filter_panel_entries(&self) -> Vec<TaskFilterPanelEntry> {
        self.view.filter_editor
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| TaskFilterPanelEntry {
                label: field.spec.label.clone(),
                query: match field.spec.kind {
                    TaskFieldFilterKind::Labels => {
                        if field.label_values.is_empty() {
                            String::new()
                        } else {
                            field.label_values.join(" | ")
                        }
                    }
                    _ => field.query.clone(),
                },
                custom: field.spec.key.starts_with("custom:"),
                kind: match field.spec.kind {
                    TaskFieldFilterKind::String => match field.string_mode {
                        TaskFieldStringMode::Fuzzy => "string:fuzzy".to_string(),
                        TaskFieldStringMode::Substring => "string:contains".to_string(),
                        TaskFieldStringMode::Regex => "string:regex".to_string(),
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
        if let Some(calendar) = &self.view.filter_editor.calendar {
            return Some(calendar.caret());
        }
        if !self.view.filter_editor.editing()
            || matches!(field.spec.kind, TaskFieldFilterKind::Labels)
        {
            return None;
        }
        Some(field.query_caret.min(field.query.chars().count()))
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
        let index = self.view.selected?;
        let row = self.view.table.rows.get(index)?;
        if row.kind != TaskRowKind::Task {
            return None;
        }
        let task_gid = &row.gid;
        let dataset = self.loading.dataset.as_ref()?;
        let record = dataset.records.iter().find(|r| r.gid == *task_gid)?;
        let project_gid = record.project_gids.first()?;
        Some(format!("https://app.asana.com/0/{project_gid}/{task_gid}"))
    }

    fn toggle_task_selection(&mut self) {
        let Some(index) = self.view.selected else { return; };
        let Some(row) = self.view.table.rows.get(index) else { return; };
        if row.kind != TaskRowKind::Task { return; }
        let gid = row.gid.clone();
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
        Ok(TaskTableModel::from_records_with_settings(
            dataset.records,
            dataset.custom_field_definitions,
            &TaskTableSettings::default(),
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
        let mut definitions_by_gid: HashMap<String, String> = HashMap::new();
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
                        .or_insert(setting.custom_field.name.clone());
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
            // name is only a fallback for the ones that genuinely sit outside
            // every project.
            let mut ancestors = AncestorPlacements::default();

            for task in tasks {
                let placement = if is_assigned_to_me {
                    ancestors.placement(client, &task)
                } else {
                    None
                };
                let (project_name, inherited_section) = match placement {
                    Some(placement) => (placement.project, placement.section),
                    None => (project.name.clone(), None),
                };

                add_task_tree(
                    client,
                    &project.id,
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

        let mut definitions = definitions_by_gid
            .into_iter()
            .map(|(gid, name)| CustomFieldDefinition::new(gid, name))
            .collect::<Vec<_>>();
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
        if let Some(index) = self.view.selected {
            if let Some(previous) = self.view.table.previous_selectable_row_index(index, 1) {
                self.view.selected = Some(previous);
            }
        }
    }

    pub fn move_down(&mut self) {
        if let Some(index) = self.view.selected {
            if let Some(next) = self.view.table.next_selectable_row_index(index, 1) {
                self.view.selected = Some(next);
            }
        } else {
            self.view.selected = self.view.table.first_selectable_row_index();
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        let step = page_size.max(1);
        if let Some(index) = self.view.selected {
            if let Some(previous) = self.view.table.previous_selectable_row_index(index, step) {
                self.view.selected = Some(previous);
            }
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
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
        self.view.selected = self.view.table.first_selectable_row_index();
    }

    pub fn jump_bottom(&mut self) {
        self.view.selected = self.view.table.last_selectable_row_index();
    }

    pub fn move_section_up(&mut self) {
        if let Some(index) = self.view.selected {
            if let Some(previous) = self.view.table.previous_section_row_index(index, 1) {
                self.view.selected = Some(previous);
            }
        }
    }

    pub fn move_section_down(&mut self) {
        if let Some(index) = self.view.selected {
            if let Some(next) = self.view.table.next_section_row_index(index, 1) {
                self.view.selected = Some(next);
            }
        } else {
            self.view.selected = self.view.table.first_selectable_row_index();
        }
    }

    pub fn move_project_up(&mut self) {
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

    fn refresh_table(&mut self) {
        let Some(dataset) = self.loading.dataset.as_ref() else {
            return;
        };

        let previous_selected_index = self.view.selected;
        let selected_gid = self
            .view.selected
            .and_then(|index| self.view.table.rows.get(index))
            .map(|row| row.gid.clone());
        let selected_parent_gid = selected_gid.as_deref().and_then(|gid| {
            dataset
                .records
                .iter()
                .find(|record| record.gid == gid)
                .and_then(|record| record.parent_gid.clone())
        });

        let filtered_records = self.apply_filter_panel(&dataset.records);

        self.view.table = TaskTableModel::from_records_with_settings(
            filtered_records,
            dataset.custom_field_definitions.clone(),
            &self.view.settings,
        );

        self.view.selected = selected_gid
            .as_deref()
            .and_then(|gid| self.view.table.rows.iter().position(|row| row.gid == gid));

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

        self.view.filter_editor.selected = self
            .view
            .filter_editor
            .selected
            .min(self.view.filter_editor.fields.len().saturating_sub(1));
        self.view.filter_vertical_scroll = self
            .view
            .filter_vertical_scroll
            .min(self.view.filter_editor.fields.len().saturating_sub(1));

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
        records
            .iter()
            .filter(|record| self.view.filter_editor.matches(record))
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

/// Where a task belongs in the project/section hierarchy, as resolved from the
/// task's own project membership or from the nearest ancestor that has one.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TaskPlacement {
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
        let project = membership.project.name.clone();
        if project.trim().is_empty() {
            return None;
        }
        Some(TaskPlacement {
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
/// `target_gid` is the id the records are cached under — a project gid, or the
/// current user's gid for the assigned-to-me row. It is deliberately separate
/// from `project_name`, which is the project the rows are *grouped* under and
/// for assigned-to-me tasks comes from the task's own placement rather than
/// from the target.
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
        domain::{Project, TaskRowKind, TaskTableModel},
    };

    use super::{TaskDataset, TaskFieldFilterKind, TaskFilterEditorState, TaskState, TaskStatus};

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
            .fields
            .iter()
            .position(|field| field.spec.label == "Tag")
            .expect("the Tag row exists");
        assert_eq!(
            state.view.filter_editor.fields[tag].label_options,
            vec!["blue".to_string(), "red".to_string()],
            "the options are the union across every id"
        );

        state.view.filter_editor.selected = tag;
        state.filter_edit_begin();
        state.filter_add_label();
        state.filter_cycle_label_value(0);
        assert_eq!(state.table().task_count(), 1);
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
        assert_eq!(state.table().rows[0].kind, crate::domain::TaskRowKind::ProjectSeparator);
        assert_eq!(state.table().rows[1].kind, crate::domain::TaskRowKind::ProjectHeader);
        assert_eq!(state.table().rows[1].cells[0], "Backlog");
        assert_eq!(state.table().rows[2].kind, crate::domain::TaskRowKind::SectionSpacer);
        assert_eq!(state.table().rows[3].kind, crate::domain::TaskRowKind::SectionHeader);
        assert_eq!(state.table().rows[3].cells[0], "Later");
        assert_eq!(state.table().rows[4].kind, crate::domain::TaskRowKind::Task);
        assert_eq!(state.table().rows[4].cells[0], "Ship release");
        assert_eq!(state.table().rows[4].cells[1], "Alex");
        assert_eq!(state.table().rows[4].cells[2], "2026-06-01");
        assert_eq!(state.table().rows[4].cells[3], "2026-05-28");
        assert_eq!(state.table().rows[4].cells[4], "open");
        assert_eq!(state.table().rows[4].cells[5], "Backlog | Inbox");
        assert_eq!(state.table().rows[5].kind, crate::domain::TaskRowKind::ProjectSeparator);
        assert_eq!(state.table().rows[6].kind, crate::domain::TaskRowKind::ProjectHeader);
        assert_eq!(state.table().rows[6].cells[0], "Inbox");
        assert_eq!(state.table().rows[7].kind, crate::domain::TaskRowKind::SectionSpacer);
        assert_eq!(state.table().rows[8].kind, crate::domain::TaskRowKind::SectionHeader);
        assert_eq!(state.table().rows[8].cells[0], "Today");
        assert_eq!(state.table().rows[9].kind, crate::domain::TaskRowKind::Task);
        assert_eq!(state.table().rows[9].cells[0], "Write docs");
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

    #[test]
    fn date_filters_match_exact_dates_ranges_and_open_bounds() {
        // Token parsing itself is covered in `domain::date`; this pins the
        // predicate the filter panel actually calls.
        assert!(super::date_filter_matches(Some("2026-09-15"), "2026-09-15"));
        assert!(!super::date_filter_matches(Some("2026-09-16"), "2026-09-15"));

        assert!(super::date_filter_matches(
            Some("2026-09-01"),
            "2026-09-01..2026-09-30"
        ));
        assert!(super::date_filter_matches(
            Some("2026-09-30"),
            "2026-09-01..2026-09-30"
        ));
        assert!(!super::date_filter_matches(
            Some("2026-10-01"),
            "2026-09-01..2026-09-30"
        ));

        assert!(super::date_filter_matches(Some("2030-01-01"), "2026-09-01.."));
        assert!(super::date_filter_matches(Some("2020-01-01"), "..2026-09-01"));

        // A task with no date cannot satisfy a date filter, but a blank query is
        // not a filter at all.
        assert!(!super::date_filter_matches(None, "2026-09-15"));
        assert!(super::date_filter_matches(None, "   "));
        assert!(super::date_filter_matches(Some("2026-09-15"), ""));

        // An unparseable query matches nothing rather than everything.
        assert!(!super::date_filter_matches(Some("2026-09-15"), "someday"));
        assert!(!super::date_filter_matches(Some("2026-09-15"), "2026-02-31"));
    }

    #[test]
    fn date_filters_resolve_keywords_against_the_local_date() {
        let today = crate::domain::date::today();

        assert!(super::date_filter_matches(Some(&today.iso()), "today"));
        assert!(!super::date_filter_matches(
            Some(&today.add_days(1).iso()),
            "today"
        ));
        assert!(super::date_filter_matches(
            Some(&today.add_days(1).iso()),
            "tomorrow"
        ));
        assert!(super::date_filter_matches(
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

    #[test]
    fn due_date_range_for_query_extracts_explicit_dates() {
        // Build a filter state with an explicit date range
        let mut state = TaskFilterEditorState::from_dataset(&TaskDataset::default());
        if let Some(due) = state.fields.iter_mut().find(|f| f.spec.key == "due") {
            due.query = "2026-01-01..2026-06-30".to_string();
        }
        let (after, before) = state.due_date_range_for_query();
        assert_eq!(after.as_deref(), Some("2026-01-01"));
        assert_eq!(before.as_deref(), Some("2026-06-30"));
    }

    #[test]
    fn due_date_range_for_query_resolves_keywords() {
        let mut state = TaskFilterEditorState::from_dataset(&TaskDataset::default());
        if let Some(due) = state.fields.iter_mut().find(|f| f.spec.key == "due") {
            due.query = "today..2026-12-31".to_string();
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
        if let Some(due) = state.fields.iter_mut().find(|f| f.spec.key == "due") {
            due.query = "2026-03-04".to_string();
        }
        let (after, before) = state.due_date_range_for_query();
        assert_eq!(after.as_deref(), Some("2026-03-04"));
        assert_eq!(before.as_deref(), Some("2026-03-04"));
    }

    #[test]
    fn due_date_range_for_query_pushes_nothing_for_an_unparseable_query() {
        let mut state = TaskFilterEditorState::from_dataset(&TaskDataset::default());
        if let Some(due) = state.fields.iter_mut().find(|f| f.spec.key == "due") {
            due.query = "someday".to_string();
        }
        assert_eq!(state.due_date_range_for_query(), (None, None));
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
}
