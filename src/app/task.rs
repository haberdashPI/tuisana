//! State and behavior for the task pane.
//!
//! This module owns the task table's live state, the task-loading cache, and
//! the task filter editor used by the UI.

use std::{
    collections::{HashMap, VecDeque},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    app::debug_log,
    asana::{
        dto::{CustomFieldValueDto, TaskDto},
        AsanaClient, TaskLoadScope,
    },
    domain::{
        merge_task_record, CustomFieldDefinition, Project, TaskRecord, TaskTableModel,
        TaskTableSettings,
    },
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
    loaded_project_scopes: HashMap<String, TaskLoadScope>,
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
}

/// Mutable state for one filter row, including the user query and any selected
/// label values.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TaskFilterFieldState {
    spec: TaskFilterFieldSpec,
    query: String,
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
}

/// Tracks the filter panel's visibility, edit mode, selected row, and fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TaskFilterEditorState {
    visible: bool,
    editing: bool,
    selected: usize,
    fields: Vec<TaskFilterFieldState>,
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
                },
                TaskFieldStringMode::Fuzzy,
                Vec::new(),
            ),
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "assignee".to_string(),
                    label: "Assignee".to_string(),
                    kind: TaskFieldFilterKind::String,
                },
                TaskFieldStringMode::Substring,
                Vec::new(),
            ),
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "due".to_string(),
                    label: "Due".to_string(),
                    kind: TaskFieldFilterKind::Date,
                },
                TaskFieldStringMode::Substring,
                Vec::new(),
            ),
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "start".to_string(),
                    label: "Start".to_string(),
                    kind: TaskFieldFilterKind::Date,
                },
                TaskFieldStringMode::Substring,
                Vec::new(),
            ),
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "state".to_string(),
                    label: "State".to_string(),
                    kind: TaskFieldFilterKind::Labels,
                },
                TaskFieldStringMode::Substring,
                vec!["open".to_string(), "done".to_string()],
            ),
            TaskFilterFieldState::new(
                TaskFilterFieldSpec {
                    key: "projects".to_string(),
                    label: "Projects".to_string(),
                    kind: TaskFieldFilterKind::String,
                },
                TaskFieldStringMode::Substring,
                Vec::new(),
            ),
        ];

        for definition in &dataset.custom_field_definitions {
            let mut values = dataset
                .records
                .iter()
                .filter_map(|record| record.custom_fields.get(&definition.gid))
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
                    key: format!("custom:{}", definition.gid),
                    label: definition.name.clone(),
                    kind,
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
            field.query.push(ch);
        }
    }

    fn pop_char(&mut self) {
        if let Some(field) = self.fields.get_mut(self.selected) {
            if matches!(field.spec.kind, TaskFieldFilterKind::Labels) {
                return;
            }
            field.query.pop();
        }
    }

    fn start_editing(&mut self) {
        self.editing = true;
    }

    fn stop_editing(&mut self) {
        self.editing = false;
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
                    key if key.starts_with("custom:") => record
                        .custom_fields
                        .get(key.trim_start_matches("custom:"))
                        .map(|values| values.join(" "))
                        .unwrap_or_default(),
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
                    key if key.starts_with("custom:") => record
                        .custom_fields
                        .get(key.trim_start_matches("custom:"))
                        .cloned()
                        .unwrap_or_default(),
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

fn date_filter_matches(value: Option<&str>, query: &str) -> bool {
    let Some(value) = value else {
        return query.trim().is_empty();
    };

    let query = query.trim();
    if query.is_empty() {
        return true;
    }

    let today = current_date_parts();
    if let Some((start, end)) = query.split_once("..") {
        let start = start.trim();
        let end = end.trim();
        let Some(start) = parse_date_token(start, today) else {
            return false;
        };
        let Some(end) = parse_date_token(end, today) else {
            return false;
        };
        if !start.is_empty() && value < start.as_str() {
            return false;
        }
        if !end.is_empty() && value > end.as_str() {
            return false;
        }
        true
    } else {
        parse_date_token(query, today).is_some_and(|date| value == date)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DateParts {
    year: i32,
    month: u32,
    day: u32,
    weekday: Weekday,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Weekday {
    Sun,
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
}

impl Weekday {
    fn from_str(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "sun" | "sunday" => Some(Self::Sun),
            "mon" | "monday" => Some(Self::Mon),
            "tue" | "tues" | "tuesday" => Some(Self::Tue),
            "wed" | "wednesday" => Some(Self::Wed),
            "thu" | "thur" | "thurs" | "thursday" => Some(Self::Thu),
            "fri" | "friday" => Some(Self::Fri),
            "sat" | "saturday" => Some(Self::Sat),
            _ => None,
        }
    }

    fn index(self) -> i64 {
        match self {
            Self::Sun => 0,
            Self::Mon => 1,
            Self::Tue => 2,
            Self::Wed => 3,
            Self::Thu => 4,
            Self::Fri => 5,
            Self::Sat => 6,
        }
    }
}

fn current_date_parts() -> DateParts {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let days = (now.as_secs() / 86_400) as i64;
    date_parts_from_days(days)
}

fn date_parts_from_days(days: i64) -> DateParts {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    let weekday = match (days + 4).rem_euclid(7) {
        0 => Weekday::Sun,
        1 => Weekday::Mon,
        2 => Weekday::Tue,
        3 => Weekday::Wed,
        4 => Weekday::Thu,
        5 => Weekday::Fri,
        _ => Weekday::Sat,
    };
    DateParts {
        year: year as i32,
        month: month as u32,
        day: day as u32,
        weekday,
    }
}

fn days_from_date_parts(date: DateParts) -> i64 {
    let year = date.year as i64 - if date.month <= 2 { 1 } else { 0 };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month = date.month as i64;
    let day = date.day as i64;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_date_token(token: &str, today: DateParts) -> Option<String> {
    let token = token.trim();
    if token.is_empty() {
        return Some(String::new());
    }

    let parts = if token.eq_ignore_ascii_case("today") {
        today
    } else if token.eq_ignore_ascii_case("tomorrow") {
        date_parts_from_days(days_from_date_parts(today) + 1)
    } else if let Some(weekday) = Weekday::from_str(token) {
        let current = today.weekday.index();
        let target = weekday.index();
        let delta = (target - current).rem_euclid(7);
        date_parts_from_days(days_from_date_parts(today) + delta)
    } else if let Some((month, day)) = token.split_once('-') {
        if token.len() == 5 && token.chars().nth(2) == Some('-') {
            let year = today.year;
            let month = month.parse::<u32>().ok()?;
            let day = day.parse::<u32>().ok()?;
            DateParts {
                year,
                month,
                day,
                weekday: today.weekday,
            }
        } else {
            let year = token.get(0..4)?.parse::<i32>().ok()?;
            let month = token.get(5..7)?.parse::<u32>().ok()?;
            let day = token.get(8..10)?.parse::<u32>().ok()?;
            DateParts {
                year,
                month,
                day,
                weekday: today.weekday,
            }
        }
    } else {
        return None;
    };

    Some(format!("{:04}-{:02}-{:02}", parts.year, parts.month, parts.day))
}

impl TaskState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(&self) -> bool {
        self.view.visible
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
        let scope = self.desired_load_scope();
        for project_id in self.loading.progress.target_ids().to_vec() {
            self.loading.loaded_project_scopes
                .insert(project_id, scope);
        }
        self.finish_loading_targets();
    }

    /// Merge a partial dataset load for one project and refresh the table.
    #[allow(dead_code)]
    pub(crate) fn ingest_loaded_dataset(&mut self, dataset: TaskDataset) {
        self.loading.cache.merge_dataset(dataset);
        self.rebuild_visible_dataset();
    }

    /// Merge a partial dataset for one project and remember which scope was fetched.
    pub(crate) fn ingest_loaded_project(
        &mut self,
        project_id: &str,
        scope: TaskLoadScope,
        dataset: TaskDataset,
    ) {
        self.loading.cache.merge_dataset(dataset);
        self.loading.loaded_project_scopes.insert(project_id.to_string(), scope);
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
                lines.push(
                    "date: YYYY-MM-DD | MM-DD | today | tomorrow | mon/tue/...".to_string(),
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
    }

    pub fn move_filter_down(&mut self) {
        self.view.filter_editor.move_down();
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

    pub(crate) fn filter_edit_begin(&mut self) {
        self.view.filter_editor.start_editing();
    }

    pub(crate) fn filter_edit_done(&mut self) {
        self.view.filter_editor.stop_editing();
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
            })
            .collect()
    }

    /// Translate the completed-task filter into the Asana fetch scope.
    pub fn desired_load_scope(&self) -> TaskLoadScope {
        match self.view.settings.filter.completed {
            Some(false) => TaskLoadScope::OpenOnly,
            Some(true) | None => TaskLoadScope::All,
        }
    }

    /// Return `true` if the cache already covers every selected target project
    /// at the requested scope.
    pub fn can_serve_scope_for_targets(&self, target_ids: &[String], scope: TaskLoadScope) -> bool {
        target_ids.iter().all(|project_id| match self.loading.loaded_project_scopes.get(project_id) {
            Some(TaskLoadScope::All) => true,
            Some(TaskLoadScope::OpenOnly) => matches!(scope, TaskLoadScope::OpenOnly),
            None => false,
        })
    }

    /// Return the subset of projects that still need to be loaded for the requested scope.
    pub fn projects_requiring_load(
        &self,
        projects: &[Project],
        scope: TaskLoadScope,
    ) -> Vec<Project> {
        projects
            .iter()
            .filter(|project| match self.loading.loaded_project_scopes.get(&project.id) {
                Some(TaskLoadScope::All) => false,
                Some(TaskLoadScope::OpenOnly) => matches!(scope, TaskLoadScope::All),
                None => true,
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

    /// Load the full task dataset for the given projects, then rebuild the
    /// visible table and filter state from the cached records.
    pub fn load_task_dataset_for_projects<C: AsanaClient>(
        &mut self,
        client: &C,
        projects: &[Project],
    ) -> Result<()> {
        debug_log(&format!("task data start: project_count={}", projects.len()));
        // TODO(lazy-task-queries): replace this eager project-wide load with
        // filter-aware task requests once the Asana client can express the
        // active task filter set directly.
        let scope = self.desired_load_scope();
        let dataset = Self::build_dataset_for_projects(client, projects, scope)?;
        self.loading.cache.merge_dataset(dataset);
        self.loading.loaded_target_ids = projects.iter().map(|project| project.id.clone()).collect();
        for project in projects {
            self.loading.loaded_project_scopes
                .insert(project.id.clone(), scope);
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
        let dataset = Self::build_dataset_for_projects(client, projects, TaskLoadScope::All)?;
        Ok(TaskTableModel::from_records_with_settings(
            dataset.records,
            dataset.custom_field_definitions,
            &TaskTableSettings::default(),
        ))
    }

    pub(crate) fn build_dataset_for_projects<C: AsanaClient>(
        client: &C,
        projects: &[Project],
        scope: TaskLoadScope,
    ) -> Result<TaskDataset> {
        if projects.is_empty() {
            return Ok(TaskDataset::default());
        }

        let mut records = Vec::new();
        let mut definitions_by_gid: HashMap<String, String> = HashMap::new();
        let mut natural_order = 0usize;

        for project in projects {
            let sections = client.list_sections(&project.id)?;
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

            for setting in client.list_project_custom_field_settings(&project.id)? {
                debug_log(&format!(
                    "task data project={} custom_field={}",
                    project.id, setting.custom_field.name
                ));
                definitions_by_gid
                    .entry(setting.custom_field.gid.clone())
                    .or_insert(setting.custom_field.name.clone());
            }

            let tasks = client.list_tasks(&project.id, scope)?;
            debug_log(&format!(
                "task data project={} tasks={}",
                project.id,
                tasks.len()
            ));
            for task in tasks {
                add_task_tree(
                    client,
                    &project.id,
                    &project.name,
                    scope,
                    &section_map,
                    &section_order_map,
                    None,
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

fn add_task_tree<C: AsanaClient>(
    client: &C,
    project_gid: &str,
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
    record.project_gids.push(project_gid.to_string());
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
                project_gid,
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
                TaskMembershipSectionDto, UserDto,
            },
            fake::FakeAsanaClient,
            TaskLoadScope,
        },
        domain::Project,
    };

    use super::{TaskFieldFilterKind, TaskState, TaskStatus};

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
            TaskLoadScope::All,
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
        assert!(state.filter_summary().contains("sort title"));
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

        let task_rows = state
            .table()
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.cells[0].clone())
            .collect::<Vec<_>>();

        assert_eq!(task_rows, vec!["Parent task", "  L Child task"]);
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
            TaskLoadScope::All,
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
    fn date_filters_support_shortcuts_and_implicit_years() {
        let today = super::current_date_parts();
        let tomorrow = super::date_parts_from_days(super::days_from_date_parts(today) + 1);
        let monday_offset = (super::Weekday::Mon.index() - today.weekday.index()).rem_euclid(7);
        let monday = super::date_parts_from_days(super::days_from_date_parts(today) + monday_offset);

        assert_eq!(
            super::parse_date_token("today", today),
            Some(format!(
                "{:04}-{:02}-{:02}",
                today.year, today.month, today.day
            ))
        );
        assert_eq!(
            super::parse_date_token("tomorrow", today),
            Some(format!(
                "{:04}-{:02}-{:02}",
                tomorrow.year, tomorrow.month, tomorrow.day
            ))
        );
        assert_eq!(
            super::parse_date_token("mon", today),
            Some(format!(
                "{:04}-{:02}-{:02}",
                monday.year, monday.month, monday.day
            ))
        );
        assert_eq!(
            super::parse_date_token("06-01", today),
            Some(format!("{:04}-06-01", today.year))
        );
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
            TaskLoadScope::All,
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
}
