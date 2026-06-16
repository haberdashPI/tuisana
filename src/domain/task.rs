//! Task-domain models and table-building logic.
//!
//! This module owns the app's task representation, the task table model, and
//! the filtering/sorting/grouping rules used to build the visible task rows.

use std::collections::HashMap;

/// A section within a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// The stable section identifier.
    pub gid: String,
    /// The display name shown in the UI.
    pub name: String,
}

impl Section {
    /// Constructs a section from an id and display name.
    pub fn new(gid: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            gid: gid.into(),
            name: name.into(),
        }
    }
}

/// A custom field definition attached to a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomFieldDefinition {
    /// The stable custom-field identifier.
    pub gid: String,
    /// The display name shown in the task table header.
    pub name: String,
}

impl CustomFieldDefinition {
    /// Constructs a custom-field definition.
    pub fn new(gid: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            gid: gid.into(),
            name: name.into(),
        }
    }
}

/// One task table column backed by a custom field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomFieldColumn {
    /// The custom field definition that labels this column.
    pub definition: CustomFieldDefinition,
    /// One display value per task row.
    pub values: Vec<String>,
}

impl CustomFieldColumn {
    /// Joins the non-empty cell values into a single display string.
    pub fn value(&self) -> String {
        join_non_empty(&self.values)
    }
}

/// Canonical task data used by the app and task table builder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRecord {
    /// The stable task identifier.
    pub gid: String,
    /// The task title.
    pub name: String,
    /// Whether the task is complete.
    pub completed: bool,
    /// The most recent modified timestamp seen for this task.
    pub modified_at: Option<String>,
    /// The assignee name or id used for display and filtering.
    pub assignee: Option<String>,
    /// The due date in display-friendly form.
    pub due_date: Option<String>,
    /// The start date in display-friendly form.
    pub start_date: Option<String>,
    /// The parent task id for subtasks.
    pub parent_gid: Option<String>,
    /// The nesting depth used when rendering subtasks.
    pub subtask_depth: usize,
    /// The natural API ordering of the task.
    pub natural_order: usize,
    /// The order of the task's section, if any.
    pub section_order: Option<usize>,
    /// All project ids associated with this task.
    pub project_gids: Vec<String>,
    /// Human-readable section names associated with this task.
    pub sections: Vec<String>,
    /// Human-readable project names associated with this task.
    pub projects: Vec<String>,
    /// Multi-valued custom fields keyed by field id.
    pub custom_fields: HashMap<String, Vec<String>>,
}

impl TaskRecord {
    /// Constructs a task record with default values for all optional fields.
    pub fn new(gid: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            gid: gid.into(),
            name: name.into(),
            completed: false,
            modified_at: None,
            assignee: None,
            due_date: None,
            start_date: None,
            parent_gid: None,
            subtask_depth: 0,
            natural_order: usize::MAX,
            section_order: None,
            project_gids: Vec::new(),
            sections: Vec::new(),
            projects: Vec::new(),
            custom_fields: HashMap::new(),
        }
    }

    /// Returns `true` when this record is a subtask.
    pub fn is_subtask(&self) -> bool {
        self.parent_gid.is_some()
    }
}

/// The non-task rows that can appear in a task table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskRowKind {
    /// A blank separator between projects.
    ProjectSeparator,
    /// A labeled row introducing a project.
    ProjectHeader,
    /// A blank spacer between sections.
    SectionSpacer,
    /// A labeled row introducing a section.
    SectionHeader,
    /// A selectable task row.
    Task,
}

impl TaskRowKind {
    /// Returns `true` for rows that represent selectable tasks.
    pub fn is_task(&self) -> bool {
        matches!(self, Self::Task)
    }
}

/// One rendered row in the task table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRow {
    /// The row kind used for rendering and navigation.
    pub kind: TaskRowKind,
    /// The row task id, or empty for separators and headers.
    pub gid: String,
    /// The project label for project rows.
    pub project: Option<String>,
    /// The section label for section rows.
    pub section: Option<String>,
    /// The section ordering used for navigation.
    pub section_order: Option<usize>,
    /// One cell per visible table column.
    pub cells: Vec<String>,
}

impl TaskRow {
    /// Builds a selectable task row.
    pub fn task(gid: impl Into<String>, cells: Vec<String>) -> Self {
        Self {
            kind: TaskRowKind::Task,
            gid: gid.into(),
            project: None,
            section: None,
            section_order: None,
            cells,
        }
    }

    /// Builds a blank separator row between project groups.
    pub fn project_separator(column_count: usize) -> Self {
        Self {
            kind: TaskRowKind::ProjectSeparator,
            gid: String::new(),
            project: None,
            section: None,
            section_order: None,
            cells: vec![String::new(); column_count],
        }
    }

    /// Builds a labeled project header row.
    pub fn project_header(label: impl Into<String>, column_count: usize) -> Self {
        let label = label.into();
        let mut cells = vec![String::new(); column_count];
        cells[0] = label.clone();
        Self {
            kind: TaskRowKind::ProjectHeader,
            gid: String::new(),
            project: Some(label),
            section: None,
            section_order: None,
            cells,
        }
    }

    /// Builds a labeled section header row.
    pub fn section_header(label: impl Into<String>, column_count: usize) -> Self {
        let label = label.into();
        let mut cells = vec![String::new(); column_count];
        cells[0] = label.clone();
        Self {
            kind: TaskRowKind::SectionHeader,
            gid: String::new(),
            project: None,
            section: None,
            section_order: None,
            cells,
        }
    }

    /// Builds a blank spacer row between section groups.
    pub fn section_spacer(column_count: usize) -> Self {
        Self {
            kind: TaskRowKind::SectionSpacer,
            gid: String::new(),
            project: None,
            section: None,
            section_order: None,
            cells: vec![String::new(); column_count],
        }
    }
}

/// The fully assembled task table used by the renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskTableModel {
    /// The visible column labels.
    pub columns: Vec<String>,
    /// Any custom-field columns appended after the built-in columns.
    pub custom_field_columns: Vec<CustomFieldColumn>,
    /// The rendered rows.
    pub rows: Vec<TaskRow>,
}

/// Task table filter and sort settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskTableSettings {
    /// The active filter.
    pub filter: TaskFilter,
    /// The active sort and grouping settings.
    pub sort: TaskSort,
}

impl Default for TaskTableSettings {
    fn default() -> Self {
        Self {
            filter: TaskFilter::default(),
            sort: TaskSort::default(),
        }
    }
}

/// The active task filter state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskFilter {
    /// Free-text filtering for task titles or other textual fields.
    pub text: Option<String>,
    /// A single named field filter.
    pub field: Option<TaskFieldFilter>,
    /// The assignee filter.
    pub assignee: Option<String>,
    /// `Some(false)` = open, `Some(true)` = done, `None` = all.
    pub completed: Option<bool>,
    /// Date-range filtering for due/start dates.
    pub date_range: Option<TaskDateRange>,
    /// Whether subtasks should be shown or hidden.
    pub subtasks: SubtaskVisibility,
}

impl Default for TaskFilter {
    fn default() -> Self {
        Self {
            text: None,
            field: None,
            assignee: None,
            completed: Some(false),
            date_range: None,
            subtasks: SubtaskVisibility::Show,
        }
    }
}

impl TaskFilter {
    /// Cycles the completed-state filter through open, done, and all.
    pub fn toggle_completed_filter(&mut self) {
        self.completed = match self.completed {
            Some(false) => Some(true),
            Some(true) => None,
            None => Some(false),
        };
    }

    /// Toggles whether subtasks are visible.
    pub fn toggle_subtask_visibility(&mut self) {
        self.subtasks = match self.subtasks {
            SubtaskVisibility::Show => SubtaskVisibility::Hide,
            SubtaskVisibility::Hide => SubtaskVisibility::Show,
        };
    }
}

/// A single field/value filter used by the task filter UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskFieldFilter {
    /// The field name.
    pub name: String,
    /// The selected value, if any.
    pub value: Option<String>,
}

/// A date range used by the task filter UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskDateRange {
    /// Inclusive start date, if set.
    pub start: Option<String>,
    /// Inclusive end date, if set.
    pub end: Option<String>,
}

/// Whether subtasks are included in the task table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubtaskVisibility {
    /// Show subtasks.
    Show,
    /// Hide subtasks.
    Hide,
}

/// The task fields the table can sort or group by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskSortField {
    Project,
    Section,
    Date,
    Title,
    Assignee,
    Completed,
    Natural,
}

/// The direction of a sort rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    /// Ascending order.
    Asc,
    /// Descending order.
    Desc,
}

/// One sort rule applied to the task table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskSortRule {
    /// The field to sort by.
    pub field: TaskSortField,
    /// The direction for this field.
    pub direction: SortDirection,
}

/// The active task-table grouping and sort configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskSort {
    /// Whether rows are grouped by project.
    pub group_by_project: bool,
    /// Whether rows are grouped by section.
    pub group_by_section: bool,
    /// The ordered list of sort rules.
    pub rules: Vec<TaskSortRule>,
}

impl Default for TaskSort {
    fn default() -> Self {
        Self {
            group_by_project: true,
            group_by_section: true,
            rules: vec![
                TaskSortRule {
                    field: TaskSortField::Date,
                    direction: SortDirection::Asc,
                },
                TaskSortRule {
                    field: TaskSortField::Title,
                    direction: SortDirection::Asc,
                },
                TaskSortRule {
                    field: TaskSortField::Natural,
                    direction: SortDirection::Asc,
                },
            ],
        }
    }
}

impl TaskSort {
    /// Toggles project grouping.
    pub fn toggle_project_grouping(&mut self) {
        self.group_by_project = !self.group_by_project;
    }

    /// Toggles section grouping.
    pub fn toggle_section_grouping(&mut self) {
        self.group_by_section = !self.group_by_section;
    }

    /// Cycles the primary sort field through the supported fields.
    pub fn cycle_primary_field(&mut self) {
        let next_field = match self.rules.first().map(|rule| rule.field) {
            Some(TaskSortField::Date) | None => TaskSortField::Title,
            Some(TaskSortField::Title) => TaskSortField::Assignee,
            Some(TaskSortField::Assignee) => TaskSortField::Completed,
            Some(TaskSortField::Completed) => TaskSortField::Natural,
            Some(TaskSortField::Natural) => TaskSortField::Date,
            Some(TaskSortField::Project) | Some(TaskSortField::Section) => TaskSortField::Date,
        };

        if let Some(rule) = self.rules.first_mut() {
            rule.field = next_field;
        } else {
            self.rules.push(TaskSortRule {
                field: next_field,
                direction: SortDirection::Asc,
            });
        }
    }

    /// Returns a short label for the primary sort field.
    pub fn primary_field_label(&self) -> &'static str {
        match self.rules.first().map(|rule| rule.field) {
            Some(TaskSortField::Date) | None => "date",
            Some(TaskSortField::Title) => "title",
            Some(TaskSortField::Assignee) => "assignee",
            Some(TaskSortField::Completed) => "completed",
            Some(TaskSortField::Natural) => "natural",
            Some(TaskSortField::Project) => "project",
            Some(TaskSortField::Section) => "section",
        }
    }
}

impl Default for TaskTableModel {
    fn default() -> Self {
        Self::empty()
    }
}

impl TaskTableSettings {
    /// Returns a compact human-readable summary of the active settings.
    pub fn summary(&self) -> String {
        let grouping = format!(
            "grp p:{} s:{}",
            if self.sort.group_by_project { "on" } else { "off" },
            if self.sort.group_by_section { "on" } else { "off" }
        );
        let completed = match self.filter.completed {
            Some(false) => "open",
            Some(true) => "done",
            None => "all",
        };
        let subtasks = match self.filter.subtasks {
            SubtaskVisibility::Show => "show",
            SubtaskVisibility::Hide => "hide",
        };

        format!(
            "{}; comp {}; sub {}; sort {}",
            grouping,
            completed,
            subtasks,
            self.sort.primary_field_label()
        )
    }
}

impl TaskTableModel {
    /// Returns an empty model with the built-in columns.
    pub fn empty() -> Self {
        Self {
            columns: default_columns(),
            custom_field_columns: Vec::new(),
            rows: Vec::new(),
        }
    }

    /// Builds a table model from raw task records and custom-field metadata.
    pub fn from_records(
        records: Vec<TaskRecord>,
        custom_field_definitions: Vec<CustomFieldDefinition>,
    ) -> Self {
        Self::from_records_with_settings(records, custom_field_definitions, &TaskTableSettings::default())
    }

    /// Builds a table model using the provided filter and sort settings.
    pub fn from_records_with_settings(
        records: Vec<TaskRecord>,
        custom_field_definitions: Vec<CustomFieldDefinition>,
        settings: &TaskTableSettings,
    ) -> Self {
        let mut merged = merge_records(records);
        merged.sort_by(|left, right| settings.sort.compare(left, right));
        merged = apply_task_filter(merged, &settings.filter);

        let custom_field_columns = custom_field_definitions
            .into_iter()
            .map(|definition| {
                let values = merged
                    .iter()
                    .map(|record| {
                        record
                            .custom_fields
                            .get(&definition.gid)
                            .map(|values| join_non_empty(values))
                            .unwrap_or_default()
                    })
                    .collect();
                CustomFieldColumn { definition, values }
            })
            .collect::<Vec<_>>();

        let rows = build_rows(&merged, &custom_field_columns, settings);

        Self {
            columns: default_columns()
                .into_iter()
                .chain(
                    custom_field_columns
                        .iter()
                        .map(|column| column.definition.name.clone()),
                )
                .collect(),
            custom_field_columns,
            rows,
        }
    }

    /// Counts only the selectable task rows.
    pub fn task_count(&self) -> usize {
        self.rows.iter().filter(|row| row.kind.is_task()).count()
    }

    /// Returns the row indices that correspond to selectable tasks.
    pub fn selectable_row_indices(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.kind.is_task().then_some(index))
            .collect()
    }

    /// Returns the first selectable row index, if any.
    pub fn first_selectable_row_index(&self) -> Option<usize> {
        self.selectable_row_indices().into_iter().next()
    }

    /// Returns the last selectable row index, if any.
    pub fn last_selectable_row_index(&self) -> Option<usize> {
        self.selectable_row_indices().into_iter().last()
    }

    /// Returns the next selectable row index after `current`.
    pub fn next_selectable_row_index(&self, current: usize, step: usize) -> Option<usize> {
        let selectable = self.selectable_row_indices();
        let current_position = selectable.iter().position(|&index| index == current)?;
        let next_position = (current_position + step.max(1)).min(selectable.len().saturating_sub(1));
        selectable.get(next_position).copied()
    }

    /// Returns the previous selectable row index before `current`.
    pub fn previous_selectable_row_index(&self, current: usize, step: usize) -> Option<usize> {
        let selectable = self.selectable_row_indices();
        let current_position = selectable.iter().position(|&index| index == current)?;
        let previous_position = current_position.saturating_sub(step.max(1));
        selectable.get(previous_position).copied()
    }

    /// Returns the 1-based position of a selectable row, if the row is selectable.
    pub fn selectable_position(&self, index: usize) -> Option<usize> {
        self.selectable_row_indices()
            .iter()
            .position(|&row_index| row_index == index)
            .map(|position| position + 1)
    }

    /// Returns the next row index that starts a section group.
    pub fn next_section_row_index(&self, current: usize, step: usize) -> Option<usize> {
        self.next_group_row_index(current, step, GroupScope::Section)
    }

    /// Returns the previous row index that starts a section group.
    pub fn previous_section_row_index(&self, current: usize, step: usize) -> Option<usize> {
        self.previous_group_row_index(current, step, GroupScope::Section)
    }

    /// Returns the next row index that starts a project group.
    pub fn next_project_row_index(&self, current: usize, step: usize) -> Option<usize> {
        self.next_group_row_index(current, step, GroupScope::Project)
    }

    /// Returns the previous row index that starts a project group.
    pub fn previous_project_row_index(&self, current: usize, step: usize) -> Option<usize> {
        self.previous_group_row_index(current, step, GroupScope::Project)
    }

    fn next_group_row_index(&self, current: usize, step: usize, scope: GroupScope) -> Option<usize> {
        let group_starts = self.group_starts(scope);
        let current_start = self.group_start_for_row(current, scope)?;
        let current_position = group_starts
            .iter()
            .position(|index| *index == current_start)?;
        let target_position = (current_position + step.max(1)).min(group_starts.len().saturating_sub(1));
        if target_position == current_position && current != current_start {
            Some(current)
        } else {
            group_starts.get(target_position).copied()
        }
    }

    fn previous_group_row_index(
        &self,
        current: usize,
        step: usize,
        scope: GroupScope,
    ) -> Option<usize> {
        let group_starts = self.group_starts(scope);
        let current_start = self.group_start_for_row(current, scope)?;
        let current_position = group_starts
            .iter()
            .position(|index| *index == current_start)?;
        let target_position = current_position.saturating_sub(step.max(1));
        if target_position == current_position && current != current_start {
            Some(current)
        } else {
            group_starts.get(target_position).copied()
        }
    }

    fn group_starts(&self, scope: GroupScope) -> Vec<usize> {
        let mut starts = Vec::new();
        let mut previous_key: Option<GroupKey> = None;

        for (index, row) in self.rows.iter().enumerate() {
            let Some(key) = group_key(row, scope) else {
                continue;
            };

            if previous_key.as_ref() != Some(&key) {
                starts.push(index);
                previous_key = Some(key);
            }
        }

        starts
    }

    fn group_start_for_row(&self, current: usize, scope: GroupScope) -> Option<usize> {
        let current_row = self.rows.get(current)?;
        let current_key = group_key(current_row, scope)?;

        self.group_starts(scope)
            .into_iter()
            .find(|index| self.rows.get(*index).and_then(|row| group_key(row, scope)) == Some(current_key.clone()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupScope {
    Project,
    Section,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GroupKey {
    project: String,
    section_order: Option<usize>,
    section: Option<String>,
}

fn group_key(row: &TaskRow, scope: GroupScope) -> Option<GroupKey> {
    if !row.kind.is_task() {
        return None;
    }

    Some(match scope {
        GroupScope::Project => GroupKey {
            project: row.project.clone().unwrap_or_default(),
            section_order: None,
            section: None,
        },
        GroupScope::Section => GroupKey {
            project: row.project.clone().unwrap_or_default(),
            section_order: row.section_order,
            section: if row.section_order.is_some() {
                None
            } else {
                Some(row.section.clone().unwrap_or_default())
            },
        },
    })
}

fn default_columns() -> Vec<String> {
    vec![
        "Task".to_string(),
        "Assignee".to_string(),
        "Due".to_string(),
        "Start".to_string(),
        "State".to_string(),
        "Projects".to_string(),
    ]
}

fn merge_records(records: Vec<TaskRecord>) -> Vec<TaskRecord> {
    let mut by_gid: HashMap<String, TaskRecord> = HashMap::new();

    for record in records {
        by_gid
            .entry(record.gid.clone())
            .and_modify(|existing| {
                merge_task_record(existing, record.clone());
            })
            .or_insert(record);
    }

    by_gid.into_values().collect()
}

pub fn merge_task_record(existing: &mut TaskRecord, record: TaskRecord) {
    let incoming_is_newer = match (&existing.modified_at, &record.modified_at) {
        (None, Some(_)) => true,
        (Some(left), Some(right)) => right >= left,
        _ => false,
    };

    existing.completed |= record.completed;
    if incoming_is_newer || existing.assignee.is_none() {
        if record.assignee.is_some() {
            existing.assignee = record.assignee.clone();
        }
    }
    if incoming_is_newer || existing.due_date.is_none() {
        if record.due_date.is_some() {
            existing.due_date = record.due_date.clone();
        }
    }
    if incoming_is_newer || existing.start_date.is_none() {
        if record.start_date.is_some() {
            existing.start_date = record.start_date.clone();
        }
    }
    if incoming_is_newer || existing.name != record.name {
        existing.name = record.name.clone();
    }
    if incoming_is_newer {
        existing.modified_at = record.modified_at.clone().or(existing.modified_at.clone());
    } else if existing.modified_at.is_none() {
        existing.modified_at = record.modified_at.clone();
    }
    existing.parent_gid = existing.parent_gid.clone().or(record.parent_gid.clone());
    existing.subtask_depth = existing.subtask_depth.max(record.subtask_depth);
    existing.section_order = match (existing.section_order, record.section_order) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (None, Some(right)) => Some(right),
        (Some(left), None) => Some(left),
        (None, None) => None,
    };
    merge_text_lists(&mut existing.sections, &record.sections);
    merge_text_lists(&mut existing.projects, &record.projects);
    merge_text_lists(&mut existing.project_gids, &record.project_gids);
    existing.natural_order = existing.natural_order.min(record.natural_order);
    for (field_gid, values) in &record.custom_fields {
        existing
            .custom_fields
            .entry(field_gid.clone())
            .and_modify(|existing_values| merge_text_lists(existing_values, values))
            .or_insert_with(|| values.clone());
    }
}

fn merge_text_lists(target: &mut Vec<String>, source: &[String]) {
    for value in source {
        let value = sanitize_display_text(value);
        if !value.trim().is_empty() && !target.iter().any(|existing| existing == &value) {
            target.push(value);
        }
    }
    target.sort();
}

fn join_non_empty(values: &[String]) -> String {
    values
        .iter()
        .map(|value| sanitize_display_text(value))
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
}

fn sanitize_display_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\n' | '\r' => ' ',
            other => other,
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn record_sort_key(record: &TaskRecord) -> (usize, String) {
    (record.natural_order, record.gid.clone())
}

impl TaskSort {
    fn compare(&self, left: &TaskRecord, right: &TaskRecord) -> std::cmp::Ordering {
        match (left.parent_gid.is_some(), right.parent_gid.is_some()) {
            (false, true) => return std::cmp::Ordering::Less,
            (true, false) => return std::cmp::Ordering::Greater,
            _ => {}
        }

        if self.group_by_project {
            let ordering = left
                .projects
                .first()
                .cloned()
                .unwrap_or_default()
                .cmp(&right.projects.first().cloned().unwrap_or_default());
            if ordering != std::cmp::Ordering::Equal {
                return ordering;
            }
        }

        if self.group_by_section {
            let ordering = compare_section_key(left, right);
            if ordering != std::cmp::Ordering::Equal {
                return ordering;
            }
        }

        for rule in &self.rules {
            let ordering = compare_rule(rule, left, right);
            if ordering != std::cmp::Ordering::Equal {
                return ordering;
            }
        }

        record_sort_key(left).cmp(&record_sort_key(right))
    }
}

fn compare_section_key(left: &TaskRecord, right: &TaskRecord) -> std::cmp::Ordering {
    left
        .section_order
        .unwrap_or(usize::MAX)
        .cmp(&right.section_order.unwrap_or(usize::MAX))
        .then_with(|| {
            left.sections
                .first()
                .cloned()
                .unwrap_or_default()
                .cmp(&right.sections.first().cloned().unwrap_or_default())
        })
}

fn compare_rule(rule: &TaskSortRule, left: &TaskRecord, right: &TaskRecord) -> std::cmp::Ordering {
    let ordering = match rule.field {
        TaskSortField::Project => left
            .projects
            .first()
            .cloned()
            .unwrap_or_default()
            .cmp(&right.projects.first().cloned().unwrap_or_default()),
        TaskSortField::Section => compare_section_key(left, right),
        TaskSortField::Date => left
            .due_date
            .clone()
            .or_else(|| left.start_date.clone())
            .unwrap_or_default()
            .cmp(
                &right
                    .due_date
                    .clone()
                    .or_else(|| right.start_date.clone())
                    .unwrap_or_default(),
            ),
        TaskSortField::Title => sanitize_display_text(&left.name).cmp(&sanitize_display_text(&right.name)),
        TaskSortField::Assignee => left
            .assignee
            .clone()
            .unwrap_or_default()
            .cmp(&right.assignee.clone().unwrap_or_default()),
        TaskSortField::Completed => left.completed.cmp(&right.completed),
        TaskSortField::Natural => left.natural_order.cmp(&right.natural_order),
    };

    match rule.direction {
        SortDirection::Asc => ordering,
        SortDirection::Desc => ordering.reverse(),
    }
}

fn apply_task_filter(records: Vec<TaskRecord>, filter: &TaskFilter) -> Vec<TaskRecord> {
    match filter.subtasks {
        SubtaskVisibility::Hide => records
            .into_iter()
            .filter(|record| !record.is_subtask() && record_matches(record, filter))
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

            let mut output = Vec::new();
            for key in group_order {
                let Some(group) = groups.remove(&key) else {
                    continue;
                };

                if group.iter().any(|record| record_matches(record, filter)) {
                    // Include the group, but always enforce the completed filter
                    // per-record — a parent matching doesn't bring in completed subtasks.
                    output.extend(group.into_iter().filter(|record| {
                        filter.completed.map_or(true, |want| record.completed == want)
                    }));
                }
            }

            output
        }
    }
}

fn record_matches(record: &TaskRecord, filter: &TaskFilter) -> bool {
    if let Some(completed) = filter.completed {
        if record.completed != completed {
            return false;
        }
    }

    if let Some(assignee) = &filter.assignee {
        let assignee = assignee.to_ascii_lowercase();
        let candidate = record
            .assignee
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !candidate.contains(&assignee) {
            return false;
        }
    }

    if let Some(range) = &filter.date_range {
        let date = record.due_date.as_ref().or(record.start_date.as_ref());
        let Some(date) = date else {
            return false;
        };
        if range.start.as_deref().is_some_and(|start| date.as_str() < start) {
            return false;
        }
        if range.end.as_deref().is_some_and(|end| date.as_str() > end) {
            return false;
        }
    }

    if let Some(field_filter) = &filter.field {
        let Some(values) = record.custom_fields.get(&field_filter.name) else {
            return false;
        };
        if let Some(expected) = &field_filter.value {
            let needle = expected.to_ascii_lowercase();
            if !values.iter().any(|value| value.to_ascii_lowercase().contains(&needle)) {
                return false;
            }
        }
    }

    if let Some(text) = &filter.text {
        let needle = text.to_ascii_lowercase();
        let haystacks = vec![
            record.name.clone(),
            record.assignee.clone().unwrap_or_default(),
            record.due_date.clone().unwrap_or_default(),
            record.start_date.clone().unwrap_or_default(),
            record.sections.join(" "),
            record.projects.join(" "),
        ];
        if !haystacks
            .iter()
            .any(|value| value.to_ascii_lowercase().contains(&needle))
            && !record
                .custom_fields
                .values()
                .flatten()
                .any(|value| value.to_ascii_lowercase().contains(&needle))
        {
            return false;
        }
    }

    true
}

fn build_rows(
    merged: &[TaskRecord],
    custom_field_columns: &[CustomFieldColumn],
    settings: &TaskTableSettings,
) -> Vec<TaskRow> {
    let column_count = default_columns().len() + custom_field_columns.len();
    let mut rows = Vec::new();
    let mut current_project: Option<String> = None;
    let mut current_section: Option<String> = None;

    for (index, record) in merged.iter().enumerate() {
        let project = record.projects.first().cloned().unwrap_or_default();
        let section = record.sections.first().cloned().unwrap_or_default();
        let has_section = !section.trim().is_empty();
        let is_subtask = record.subtask_depth > 0;

        // Subtasks inherit their parent's project/section context — don't emit
        // new headers for them, which would split a project group or duplicate
        // a project name when subtasks have a different (or empty) project membership.
        if !is_subtask && settings.sort.group_by_project && current_project.as_deref() != Some(project.as_str()) {
            let project_label = sanitize_display_text(&project);
            current_project = Some(project.clone());
            current_section = None;
            rows.push(TaskRow::project_separator(column_count));
            rows.push(TaskRow::project_header(project_label, column_count));
        }

        if !is_subtask && settings.sort.group_by_section && has_section && current_section.as_deref() != Some(section.as_str()) {
            rows.push(TaskRow::section_spacer(column_count));
            rows.push(TaskRow::section_header(sanitize_display_text(&section), column_count));
            current_section = Some(section.clone());
        }

        let mut cells = vec![
            sanitize_display_text(&record.name),
            record
                .assignee
                .as_deref()
                .map(sanitize_display_text)
                .unwrap_or_default(),
            record
                .due_date
                .as_deref()
                .map(sanitize_display_text)
                .unwrap_or_default(),
            record
                .start_date
                .as_deref()
                .map(sanitize_display_text)
                .unwrap_or_default(),
            if record.completed {
                "done".to_string()
            } else {
                "open".to_string()
            },
            join_non_empty(&record.projects),
        ];

        for column in custom_field_columns {
            let value = column.values.get(index).cloned().unwrap_or_default();
            cells.push(value);
        }

        let mut row = TaskRow::task(record.gid.clone(), cells);
        row.project = Some(project);
        row.section = Some(section);
        row.section_order = record.section_order;
        if record.subtask_depth > 0 {
            let indent = "  ".repeat(record.subtask_depth);
            row.cells[0] = format!("{indent}L {}", row.cells[0]);
        }
        rows.push(row);
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::{
        CustomFieldDefinition, SortDirection, SubtaskVisibility, TaskDateRange, TaskFieldFilter,
        TaskFilter, TaskRecord, TaskRowKind, TaskSort, TaskSortField, TaskSortRule,
        TaskTableModel, TaskTableSettings,
    };

    #[test]
    fn merges_duplicate_tasks_and_combines_sources() {
        let mut first = TaskRecord::new("1", "Draft release");
        first.completed = false;
        first.assignee = Some("Alex".to_string());
        first.sections = vec!["Backlog".to_string()];
        first.projects = vec!["Alpha".to_string()];
        first.custom_fields.insert("cf-1".to_string(), vec!["High".to_string()]);

        let mut second = TaskRecord::new("1", "Draft release");
        second.completed = true;
        second.sections = vec!["Ready".to_string()];
        second.projects = vec!["Beta".to_string()];
        second
            .custom_fields
            .insert("cf-1".to_string(), vec!["High".to_string(), "Urgent".to_string()]);

        let mut settings = TaskTableSettings::default();
        settings.filter.completed = None;
        let model = TaskTableModel::from_records_with_settings(
            vec![first, second],
            vec![CustomFieldDefinition::new("cf-1", "Priority")],
            &settings,
        );

        assert_eq!(model.columns[0], "Task");
        assert_eq!(model.columns.last().expect("custom field column"), "Priority");
        assert_eq!(model.task_count(), 1);
        assert_eq!(
            model.rows.iter().map(|row| row.kind.clone()).collect::<Vec<_>>(),
            vec![
                TaskRowKind::ProjectSeparator,
                TaskRowKind::ProjectHeader,
                TaskRowKind::SectionSpacer,
                TaskRowKind::SectionHeader,
                TaskRowKind::Task,
            ]
        );
        assert_eq!(model.rows[1].cells[0], "Alpha");
        assert_eq!(model.rows[3].cells[0], "Backlog");
        assert_eq!(model.rows[4].kind, TaskRowKind::Task);
        assert_eq!(model.rows[4].cells[0], "Draft release");
        assert_eq!(model.rows[4].cells[1], "Alex");
        assert_eq!(model.rows[4].cells[2], ""); // due
        assert_eq!(model.rows[4].cells[4], "done");
        assert_eq!(model.rows[4].cells[5], "Alpha | Beta");
        assert_eq!(model.rows[4].cells[6], "High | Urgent");
    }

    #[test]
    fn strips_newlines_from_display_cells() {
        let mut record = TaskRecord::new("1", "Northwind task\n");
        record.assignee = Some("Morgan Ellis\n".to_string());
        record.sections = vec!["Study Kit Design and Shipment requirements\n".to_string()];
        record.projects = vec!["Northwind BTX 4412 Ph1 PSG\n".to_string()];
        record
            .custom_fields
            .insert("cf-1".to_string(), vec!["High\n".to_string()]);

        let model = TaskTableModel::from_records(
            vec![record],
            vec![CustomFieldDefinition::new("cf-1", "Priority")],
        );

        assert_eq!(model.task_count(), 1);
        assert_eq!(
            model.rows.iter().map(|row| row.kind.clone()).collect::<Vec<_>>(),
            vec![
                TaskRowKind::ProjectSeparator,
                TaskRowKind::ProjectHeader,
                TaskRowKind::SectionSpacer,
                TaskRowKind::SectionHeader,
                TaskRowKind::Task,
            ]
        );
        assert_eq!(model.rows[1].cells[0], "Northwind BTX 4412 Ph1 PSG");
        assert_eq!(model.rows[3].cells[0], "Study Kit Design and Shipment requirements");
        assert_eq!(model.rows[4].kind, TaskRowKind::Task);
        assert_eq!(model.rows[4].cells[0], "Northwind task");
        assert_eq!(model.rows[4].cells[1], "Morgan Ellis");
        assert_eq!(model.rows[4].cells[5], "Northwind BTX 4412 Ph1 PSG");
        assert_eq!(model.rows[4].cells[6], "High");
    }

    #[test]
    fn filters_by_text_field_owner_completion_and_date() {
        let mut matching = TaskRecord::new("1", "Ship release");
        matching.assignee = Some("Alex".to_string());
        matching.due_date = Some("2026-06-10".to_string());
        matching.sections = vec!["Today".to_string()];
        matching.projects = vec!["Inbox".to_string()];
        matching
            .custom_fields
            .insert("cf-1".to_string(), vec!["High".to_string()]);

        let mut hidden = TaskRecord::new("2", "Write docs");
        hidden.assignee = Some("Jordan".to_string());
        hidden.due_date = Some("2026-07-10".to_string());
        hidden.sections = vec!["Later".to_string()];
        hidden.projects = vec!["Inbox".to_string()];
        hidden
            .custom_fields
            .insert("cf-1".to_string(), vec!["Low".to_string()]);

        let settings = TaskTableSettings {
            filter: TaskFilter {
                text: Some("ship".to_string()),
                field: Some(TaskFieldFilter {
                    name: "cf-1".to_string(),
                    value: Some("high".to_string()),
                }),
                assignee: Some("alex".to_string()),
                completed: Some(false),
                date_range: Some(TaskDateRange {
                    start: Some("2026-06-01".to_string()),
                    end: Some("2026-06-30".to_string()),
                }),
                subtasks: SubtaskVisibility::Show,
            },
            sort: TaskSort::default(),
        };

        let model = TaskTableModel::from_records_with_settings(
            vec![matching, hidden],
            vec![CustomFieldDefinition::new("cf-1", "Priority")],
            &settings,
        );

        assert_eq!(model.task_count(), 1);
        assert_eq!(model.rows.iter().filter(|row| row.kind.is_task()).count(), 1);
        assert_eq!(model.rows.iter().find(|row| row.kind.is_task()).unwrap().gid, "1");
    }

    #[test]
    fn keeps_parent_and_subtasks_together_when_visible() {
        let mut parent = TaskRecord::new("parent", "Parent");
        parent.sections = vec!["Today".to_string()];
        parent.projects = vec!["Inbox".to_string()];

        let mut child = TaskRecord::new("child", "Child match");
        child.parent_gid = Some("parent".to_string());
        child.sections = vec!["Today".to_string()];
        child.projects = vec!["Inbox".to_string()];

        let settings = TaskTableSettings {
            filter: TaskFilter {
                text: Some("match".to_string()),
                subtasks: SubtaskVisibility::Show,
                ..TaskFilter::default()
            },
            sort: TaskSort::default(),
        };

        let model = TaskTableModel::from_records_with_settings(
            vec![parent.clone(), child.clone()],
            vec![],
            &settings,
        );

        let task_rows = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.as_str())
            .collect::<Vec<_>>();

        assert_eq!(task_rows, vec!["parent", "child"]);

        let hidden_model = TaskTableModel::from_records_with_settings(
            vec![parent, child],
            vec![],
            &TaskTableSettings {
                filter: TaskFilter {
                    subtasks: SubtaskVisibility::Hide,
                    ..TaskFilter::default()
                },
                sort: TaskSort::default(),
            },
        );

        let hidden_task_rows = hidden_model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.as_str())
            .collect::<Vec<_>>();

        assert_eq!(hidden_task_rows, vec!["parent"]);
    }

    #[test]
    fn sorts_with_custom_rules_and_stable_fallback() {
        let mut alpha = TaskRecord::new("a", "Alpha");
        alpha.completed = true;
        alpha.projects = vec!["Inbox".to_string()];
        alpha.sections = vec!["Today".to_string()];

        let mut beta = TaskRecord::new("b", "Beta");
        beta.completed = false;
        beta.projects = vec!["Inbox".to_string()];
        beta.sections = vec!["Today".to_string()];

        let settings = TaskTableSettings {
            filter: TaskFilter {
                completed: None,
                ..TaskFilter::default()
            },
            sort: TaskSort {
                group_by_project: true,
                group_by_section: true,
                rules: vec![
                    TaskSortRule {
                        field: TaskSortField::Completed,
                        direction: SortDirection::Asc,
                    },
                    TaskSortRule {
                        field: TaskSortField::Title,
                        direction: SortDirection::Desc,
                    },
                ],
            },
        };

        let model = TaskTableModel::from_records_with_settings(vec![alpha, beta], vec![], &settings);
        let task_rows = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.as_str())
            .collect::<Vec<_>>();

        assert_eq!(task_rows, vec!["b", "a"]);
    }

    #[test]
    fn sorts_sections_using_their_recorded_order() {
        let mut beta = TaskRecord::new("b", "Beta task");
        beta.projects = vec!["Inbox".to_string()];
        beta.sections = vec!["Beta".to_string()];
        beta.section_order = Some(0);

        let mut alpha = TaskRecord::new("a", "Alpha task");
        alpha.projects = vec!["Inbox".to_string()];
        alpha.sections = vec!["Alpha".to_string()];
        alpha.section_order = Some(1);

        let model = TaskTableModel::from_records_with_settings(
            vec![alpha, beta],
            vec![],
            &TaskTableSettings::default(),
        );

        let task_rows = model
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
    fn section_navigation_uses_section_order_when_labels_repeat() {
        let mut first = TaskRecord::new("a", "First priority queue task");
        first.projects = vec!["Inbox".to_string()];
        first.sections = vec!["Priority Queue".to_string()];
        first.section_order = Some(0);

        let mut middle = TaskRecord::new("b", "Middle task");
        middle.projects = vec!["Inbox".to_string()];
        middle.sections = vec!["Study Specific Tasks".to_string()];
        middle.section_order = Some(1);

        let mut second = TaskRecord::new("c", "Second priority queue task");
        second.projects = vec!["Inbox".to_string()];
        second.sections = vec!["Priority Queue".to_string()];
        second.section_order = Some(2);

        let mut later = TaskRecord::new("d", "Later task");
        later.projects = vec!["Inbox".to_string()];
        later.sections = vec!["Clinical Intake".to_string()];
        later.section_order = Some(3);

        let model = TaskTableModel::from_records_with_settings(
            vec![second, later, first, middle],
            vec![],
            &TaskTableSettings::default(),
        );

        let second_index = model
            .rows
            .iter()
            .enumerate()
            .find_map(|(index, row)| (row.kind.is_task() && row.cells[0] == "Second priority queue task").then_some(index))
            .expect("second section row");
        let later_index = model
            .rows
            .iter()
            .enumerate()
            .find_map(|(index, row)| (row.kind.is_task() && row.cells[0] == "Later task").then_some(index))
            .expect("later section row");

        assert_eq!(model.next_section_row_index(second_index, 1), Some(later_index));
    }

    #[test]
    fn natural_sort_uses_ingest_order_instead_of_gid_order() {
        let mut later_gid = TaskRecord::new("z", "Later ingest");
        later_gid.projects = vec!["Inbox".to_string()];
        later_gid.sections = vec!["Today".to_string()];
        later_gid.natural_order = 1;

        let mut earlier_gid = TaskRecord::new("a", "Earlier ingest");
        earlier_gid.projects = vec!["Inbox".to_string()];
        earlier_gid.sections = vec!["Today".to_string()];
        earlier_gid.natural_order = 0;

        let settings = TaskTableSettings {
            filter: TaskFilter::default(),
            sort: TaskSort {
                group_by_project: true,
                group_by_section: true,
                rules: vec![TaskSortRule {
                    field: TaskSortField::Natural,
                    direction: SortDirection::Asc,
                }],
            },
        };

        let model = TaskTableModel::from_records_with_settings(
            vec![later_gid, earlier_gid],
            vec![],
            &settings,
        );

        let task_rows = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.as_str())
            .collect::<Vec<_>>();

        assert_eq!(task_rows, vec!["a", "z"]);
    }

    #[test]
    fn omits_section_spacers_when_section_grouping_is_disabled() {
        let mut first = TaskRecord::new("a", "First");
        first.projects = vec!["Inbox".to_string()];
        first.sections = vec!["Today".to_string()];
        first.section_order = Some(0);

        let mut second = TaskRecord::new("b", "Second");
        second.projects = vec!["Inbox".to_string()];
        second.sections = vec!["Later".to_string()];
        second.section_order = Some(1);

        let settings = TaskTableSettings {
            filter: TaskFilter::default(),
            sort: TaskSort {
                group_by_project: true,
                group_by_section: false,
                rules: vec![TaskSortRule {
                    field: TaskSortField::Date,
                    direction: SortDirection::Asc,
                }],
            },
        };

        let model = TaskTableModel::from_records_with_settings(vec![first, second], vec![], &settings);
        let row_kinds = model.rows.iter().map(|row| row.kind.clone()).collect::<Vec<_>>();

        assert_eq!(
            row_kinds,
            vec![
                TaskRowKind::ProjectSeparator,
                TaskRowKind::ProjectHeader,
                TaskRowKind::Task,
                TaskRowKind::Task,
            ]
        );
    }

    #[test]
    fn sorts_across_projects_when_project_grouping_is_disabled() {
        let mut later_project = TaskRecord::new("a", "Later project task");
        later_project.projects = vec!["Zeta".to_string()];
        later_project.sections = vec!["Today".to_string()];
        later_project.due_date = Some("2026-06-20".to_string());
        later_project.natural_order = 1;

        let mut earlier_project = TaskRecord::new("b", "Earlier project task");
        earlier_project.projects = vec!["Alpha".to_string()];
        earlier_project.sections = vec!["Today".to_string()];
        earlier_project.due_date = Some("2026-06-10".to_string());
        earlier_project.natural_order = 0;

        let settings = TaskTableSettings {
            filter: TaskFilter::default(),
            sort: TaskSort {
                group_by_project: false,
                group_by_section: false,
                rules: vec![TaskSortRule {
                    field: TaskSortField::Date,
                    direction: SortDirection::Asc,
                }],
            },
        };

        let model = TaskTableModel::from_records_with_settings(
            vec![later_project, earlier_project],
            vec![],
            &settings,
        );

        let task_rows = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.as_str())
            .collect::<Vec<_>>();

        assert_eq!(task_rows, vec!["b", "a"]);
        assert!(model.rows.iter().all(|row| row.kind.is_task()));
    }

    #[test]
    fn project_header_not_repeated_when_subtask_has_different_project() {
        // Regression test: subtasks whose project membership differs from their
        // parent (or is empty) must not trigger a duplicate project header.
        let mut parent = TaskRecord::new("parent", "Parent task");
        parent.projects = vec!["Alpha".to_string()];
        parent.sections = vec!["Today".to_string()];

        // Subtask belongs to a different project than its parent.
        let mut subtask = TaskRecord::new("sub", "Subtask");
        subtask.parent_gid = Some("parent".to_string());
        subtask.projects = vec!["Beta".to_string()];
        subtask.sections = vec!["Today".to_string()];
        subtask.subtask_depth = 1;

        // A second task in the original project — should still land under the
        // same "Alpha" header, not trigger a new one.
        let mut sibling = TaskRecord::new("sibling", "Sibling task");
        sibling.projects = vec!["Alpha".to_string()];
        sibling.sections = vec!["Today".to_string()];

        let settings = TaskTableSettings {
            filter: TaskFilter {
                subtasks: SubtaskVisibility::Show,
                ..TaskFilter::default()
            },
            sort: TaskSort {
                group_by_project: true,
                group_by_section: false,
                rules: vec![],
            },
        };

        let model = TaskTableModel::from_records_with_settings(
            vec![parent, subtask, sibling],
            vec![],
            &settings,
        );

        let project_headers: Vec<&str> = model
            .rows
            .iter()
            .filter(|row| row.kind == TaskRowKind::ProjectHeader)
            .map(|row| row.cells[0].as_str())
            .collect();

        // "Alpha" must appear exactly once — the subtask with project "Beta"
        // must not split the Alpha group or trigger a second Alpha header.
        assert_eq!(project_headers.iter().filter(|&&h| h == "Alpha").count(), 1, "Alpha header appeared more than once");
    }

    #[test]
    fn completed_subtask_excluded_when_open_only_filter_with_subtasks_visible() {
        // Regression test: a completed subtask of an open parent must not appear
        // when the completed filter is set to open-only with SubtaskVisibility::Show.
        // Before the fix, the whole group was included whenever the parent matched,
        // bypassing the per-record completed check for subtasks.
        let mut parent = TaskRecord::new("parent", "Open parent");
        parent.completed = false;
        parent.projects = vec!["Inbox".to_string()];
        parent.sections = vec!["Today".to_string()];

        let mut done_child = TaskRecord::new("done-child", "Completed subtask");
        done_child.completed = true;
        done_child.parent_gid = Some("parent".to_string());
        done_child.projects = vec!["Inbox".to_string()];
        done_child.sections = vec!["Today".to_string()];

        let mut open_child = TaskRecord::new("open-child", "Open subtask");
        open_child.completed = false;
        open_child.parent_gid = Some("parent".to_string());
        open_child.projects = vec!["Inbox".to_string()];
        open_child.sections = vec!["Today".to_string()];

        let settings = TaskTableSettings {
            filter: TaskFilter {
                completed: Some(false),
                subtasks: SubtaskVisibility::Show,
                ..TaskFilter::default()
            },
            sort: TaskSort::default(),
        };

        let model = TaskTableModel::from_records_with_settings(
            vec![parent, done_child, open_child],
            vec![],
            &settings,
        );

        let task_gids: Vec<&str> = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.as_str())
            .collect();

        assert!(
            !task_gids.contains(&"done-child"),
            "completed subtask should be excluded by open-only filter"
        );
        assert!(task_gids.contains(&"parent"), "open parent should be included");
        assert!(task_gids.contains(&"open-child"), "open subtask should be included");
    }
}
