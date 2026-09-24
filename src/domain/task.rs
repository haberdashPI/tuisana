//! Task-domain models and table-building logic.
//!
//! This module owns the app's task representation, the task table model, and
//! the filtering/sorting/grouping rules used to build the visible task rows.

use std::collections::HashMap;

use crate::domain::date::CivilDate;

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
    /// The project whose settings declared this field.
    ///
    /// One field name usually exists once per project, with a different gid in
    /// each. Reading merges them by name; a write has to choose, and the only
    /// correct choice is the gid belonging to the task's own project.
    pub project_gid: String,
    /// What the field holds, as Asana declares it.
    pub kind: CustomFieldKind,
}

impl CustomFieldDefinition {
    /// Constructs a custom-field definition with no project and a text kind.
    ///
    /// The two-argument shape is kept for the many fixtures that only care
    /// about the name a column is labelled with; [`Self::with_kind`] and
    /// [`Self::in_project`] carry the rest.
    pub fn new(gid: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            gid: gid.into(),
            name: name.into(),
            project_gid: String::new(),
            kind: CustomFieldKind::Text,
        }
    }

    /// The same definition, declaring `kind`.
    pub fn with_kind(mut self, kind: CustomFieldKind) -> Self {
        self.kind = kind;
        self
    }

    /// The same definition, belonging to `project_gid`.
    pub fn in_project(mut self, project_gid: impl Into<String>) -> Self {
        self.project_gid = project_gid.into();
        self
    }

    /// This field's declared enum options, empty for every other kind.
    pub fn enum_options(&self) -> &[EnumOption] {
        match &self.kind {
            CustomFieldKind::Enum { options } => options,
            _ => &[],
        }
    }
}

/// What a custom field holds, as Asana declares it.
///
/// Declared rather than inferred. The filter panel guesses a field's kind from
/// the values it has seen, which is right for filtering and wrong for editing:
/// a picker built from observed values can never offer the option no task has
/// yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustomFieldKind {
    Text,
    Number,
    Enum { options: Vec<EnumOption> },
    /// Everything else, which this milestone refuses to edit.
    Unsupported(String),
}

/// One declared option of an enum custom field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumOption {
    pub gid: String,
    pub name: String,
}

impl EnumOption {
    pub fn new(gid: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            gid: gid.into(),
            name: name.into(),
        }
    }
}

/// Groups custom field definitions by name, keeping every id that carries it.
///
/// The same custom field usually exists separately in each project, with its own
/// id, so anything keyed by id gets one duplicate per project — five projects
/// with a "Tag" field produced five "Tag" table columns and five "Tag" filter
/// rows. Both the table and the filter panel group through here.
///
/// Names keep their first-appearance order, so a sorted input stays sorted and
/// the table's columns line up with the filter panel's rows.
pub fn group_custom_fields_by_name(
    definitions: &[CustomFieldDefinition],
) -> Vec<(String, Vec<String>)> {
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    for definition in definitions {
        match grouped
            .iter_mut()
            .find(|(name, _)| *name == definition.name)
        {
            Some((_, gids)) => gids.push(definition.gid.clone()),
            None => grouped.push((definition.name.clone(), vec![definition.gid.clone()])),
        }
    }
    grouped
}

/// One task table column backed by a custom field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomFieldColumn {
    /// The field name this column is labelled with.
    pub name: String,
    /// Every custom-field id this column gathers values from. A field with one
    /// name usually has a separate id in each project.
    pub gids: Vec<String>,
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
    /// The assignee's Asana gid, which is what a write has to send.
    ///
    /// The display name is the one thing the API will not take, so the record
    /// keeps the gid beside it rather than throwing it away at load time.
    pub assignee_gid: Option<String>,
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
            assignee_gid: None,
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
    /// How deeply this task is nested under a parent task.
    ///
    /// The renderer turns this into an indent and a marker. The depth is kept
    /// as a number rather than baked into the title cell so the domain model
    /// stays free of presentation strings.
    pub subtask_depth: usize,
    /// The task's start date, parsed. `None` on header and spacer rows.
    ///
    /// The display cells stay strings, but the Gantt chart does arithmetic on
    /// these, and re-parsing a cell that has already been through
    /// `sanitize_display_text` would be one silent formatting change away from
    /// breaking.
    pub start: Option<CivilDate>,
    /// The task's due date, parsed. `None` on header and spacer rows.
    pub due: Option<CivilDate>,
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
            subtask_depth: 0,
            start: None,
            due: None,
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
            subtask_depth: 0,
            start: None,
            due: None,
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
            subtask_depth: 0,
            start: None,
            due: None,
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
            subtask_depth: 0,
            start: None,
            due: None,
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
            subtask_depth: 0,
            start: None,
            due: None,
            cells: vec![String::new(); column_count],
        }
    }
}

/// Cell index of the assignee column.
///
/// The built-in columns are identified by position because the model always
/// emits them in a fixed order and appends custom fields after them. Matching
/// on the label would misfire on a custom field of the same name.
pub const ASSIGNEE_COLUMN: usize = 1;
/// Cell index of the title column.
pub const TITLE_COLUMN: usize = 0;
/// Cell index of the due-date column.
pub const DUE_COLUMN: usize = 2;
/// Cell index of the start-date column.
pub const START_COLUMN: usize = 3;
/// Cell index of the completion-state column.
pub const STATE_COLUMN: usize = 4;
/// Cell index of the project-membership column.
pub const PROJECTS_COLUMN: usize = 5;
/// Cell index of the first custom-field column.
pub const FIRST_CUSTOM_COLUMN: usize = 6;

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

/// The settings-level task filter: the two toggles the task view owns.
///
/// Text, assignee, date, and custom-field filtering all live in the filter
/// panel (`TaskState`'s `TaskFilterEditorState`), which applies itself before
/// records reach this module. This holds only what the `c` and `z` keys drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskFilter {
    /// `Some(false)` = open, `Some(true)` = done, `None` = all.
    pub completed: Option<bool>,
    /// Whether subtasks should be shown or hidden.
    pub subtasks: SubtaskVisibility,
}

impl Default for TaskFilter {
    fn default() -> Self {
        Self {
            completed: Some(false),
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
    /// Project names in the order the project list shows them.
    ///
    /// Project groups follow this order so the table reads down in the same
    /// order the user picked the projects in — notably the pinned
    /// "No Project (Assigned to Me)" row stays at the top. A project missing
    /// from the list sorts after every listed one, by name.
    pub project_order: Vec<String>,
    /// The ordered list of sort rules.
    pub rules: Vec<TaskSortRule>,
}

impl Default for TaskSort {
    fn default() -> Self {
        Self {
            group_by_project: true,
            group_by_section: true,
            project_order: Vec::new(),
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

    /// Flips the primary sort rule between ascending and descending.
    ///
    /// Only the primary rule flips. The rules after it are tie-breakers, and
    /// reversing those too would reorder rows the user never asked about.
    pub fn toggle_primary_direction(&mut self) {
        match self.rules.first_mut() {
            Some(rule) => {
                rule.direction = match rule.direction {
                    SortDirection::Asc => SortDirection::Desc,
                    SortDirection::Desc => SortDirection::Asc,
                }
            }
            // No rules means the implicit date sort, which reads as ascending;
            // asking to flip it makes that sort explicit and descending.
            None => self.rules.push(TaskSortRule {
                field: TaskSortField::Date,
                direction: SortDirection::Desc,
            }),
        }
    }

    /// The direction of the primary sort rule, ascending when there is none.
    pub fn primary_direction(&self) -> SortDirection {
        self.rules
            .first()
            .map(|rule| rule.direction)
            .unwrap_or(SortDirection::Asc)
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

        let direction = match self.sort.primary_direction() {
            SortDirection::Asc => "asc",
            SortDirection::Desc => "desc",
        };

        format!(
            "{}; comp {}; sub {}; sort {} {}",
            grouping,
            completed,
            subtasks,
            self.sort.primary_field_label(),
            direction
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
        let merged = arrange_hierarchy(merged, &settings.sort);

        let custom_field_columns = custom_columns(&merged, &custom_field_definitions);
        let rows = build_rows(&merged, &custom_field_columns, settings);

        Self::from_parts(custom_field_columns, rows)
    }

    /// Builds a table of these records, in the order given, with no grouping.
    ///
    /// The recently-edited pane. It shows the same columns as the table so the
    /// two read as one surface, but its order is "most recently edited", which
    /// no [`TaskSort`] can express, and its rows are the ones the filter has
    /// already rejected — so neither the sort nor the filter runs here.
    pub fn flat_from_records(
        records: Vec<TaskRecord>,
        custom_field_definitions: Vec<CustomFieldDefinition>,
    ) -> Self {
        let ungrouped = TaskTableSettings {
            sort: TaskSort {
                group_by_project: false,
                group_by_section: false,
                ..TaskSort::default()
            },
            ..TaskTableSettings::default()
        };
        let custom_field_columns = custom_columns(&records, &custom_field_definitions);
        let rows = build_rows(&records, &custom_field_columns, &ungrouped);

        Self::from_parts(custom_field_columns, rows)
    }

    fn from_parts(custom_field_columns: Vec<CustomFieldColumn>, rows: Vec<TaskRow>) -> Self {
        Self {
            columns: default_columns()
                .into_iter()
                .chain(
                    custom_field_columns
                        .iter()
                        .map(|column| column.name.clone()),
                )
                .collect(),
            custom_field_columns,
            rows,
        }
    }

    /// The cell index of the custom-field column with this name.
    ///
    /// Searched among the custom columns only, so a custom field called
    /// "Assignee" resolves to itself rather than to the built-in column.
    pub fn custom_column_index(&self, name: &str) -> Option<usize> {
        let offset = self.columns.len() - self.custom_field_columns.len();
        self.custom_field_columns
            .iter()
            .position(|column| column.name == name)
            .map(|index| offset + index)
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
            existing.assignee_gid = record.assignee_gid.clone();
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

/// The project name a record is grouped under, matching what [`build_rows`]
/// puts in the group's header.
fn group_project(record: &TaskRecord) -> &str {
    record.projects.first().map(String::as_str).unwrap_or_default()
}

impl TaskSort {
    /// Where `project` sits in the project list, or last when it isn't listed.
    fn project_rank(&self, project: &str) -> usize {
        self.project_order
            .iter()
            .position(|listed| listed == project)
            .unwrap_or(usize::MAX)
    }

    /// Orders two records by the grouping and sort rules alone.
    ///
    /// Nesting is not its business: [`arrange_hierarchy`] runs afterwards and
    /// puts each subtask back under its parent, so a rule here that pushed
    /// subtasks down would only decide where an *orphan* — a subtask whose
    /// parent is not on screen — lands among the rows it is grouped with.
    fn compare(&self, left: &TaskRecord, right: &TaskRecord) -> std::cmp::Ordering {
        if self.group_by_project {
            let left_project = group_project(left);
            let right_project = group_project(right);
            // Rank first so groups appear in project-list order; the name is
            // only a tie-break for projects the list didn't mention.
            let ordering = self
                .project_rank(left_project)
                .cmp(&self.project_rank(right_project))
                .then_with(|| left_project.cmp(right_project));
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

/// The calendar date a record sorts by: its due date, or its start date when it
/// has no due date.
///
/// Records store `YYYY-MM-DD` as it came off the API; the relative "Today" /
/// "Tomorrow" / "Aug 31" text the table shows is produced at render time and
/// never reaches here. Parsing to a [`CivilDate`] rather than comparing the
/// strings means a value the API surprised us with — anything that is not a real
/// date — is treated as undated instead of ordering somewhere arbitrary.
fn record_sort_date(record: &TaskRecord) -> Option<CivilDate> {
    record
        .due_date
        .as_deref()
        .or(record.start_date.as_deref())
        .and_then(CivilDate::parse)
}

fn compare_rule(rule: &TaskSortRule, left: &TaskRecord, right: &TaskRecord) -> std::cmp::Ordering {
    // Undated tasks sort last in both directions. "No date" is not an early
    // date, so reversing the sort must not float them above everything that is
    // actually scheduled.
    if rule.field == TaskSortField::Date {
        return match (record_sort_date(left), record_sort_date(right)) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(left), Some(right)) => match rule.direction {
                SortDirection::Asc => left.cmp(&right),
                SortDirection::Desc => right.cmp(&left),
            },
        };
    }

    let ordering = match rule.field {
        TaskSortField::Project => left
            .projects
            .first()
            .cloned()
            .unwrap_or_default()
            .cmp(&right.projects.first().cloned().unwrap_or_default()),
        TaskSortField::Section => compare_section_key(left, right),
        TaskSortField::Date => unreachable!("handled above so undated rows can ignore direction"),
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
/// The `Show` branch's grouping is load-bearing and must not be simplified away:
/// `TaskSort::compare` sorts *every* parent before *every* subtask, so emitting
/// one parent-group at a time is the only thing putting a subtask underneath its
/// own parent rather than in a heap at the bottom of the table.
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

/// Lays the records out parent-first, each subtask directly under its parent.
///
/// A subtask only reads as one while its parent is on screen, and plenty of
/// them arrive without it: the assignee query returns subtasks whose parents
/// belong to someone else, and the filters drop parents out from under their
/// children. Such an orphan is the root of everything the user can actually
/// see, so it is laid out as a root — grouped under its own project and
/// section, unindented — instead of trailing the table under whichever project
/// header happened to come last.
///
/// Depth is recomputed from the parent chain that survived rather than trusted
/// from the loader, so nesting stays relative to the outermost visible task.
fn arrange_hierarchy(records: Vec<TaskRecord>, sort: &TaskSort) -> Vec<TaskRecord> {
    let index_by_gid: HashMap<&str, usize> = records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.gid.as_str(), index))
        .collect();

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); records.len()];
    let mut roots: Vec<usize> = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let parent = record
            .parent_gid
            .as_deref()
            .and_then(|gid| index_by_gid.get(gid).copied())
            .filter(|parent| *parent != index);
        match parent {
            Some(parent) => children[parent].push(index),
            None => roots.push(index),
        }
    }

    let by_sort = |left: &usize, right: &usize| sort.compare(&records[*left], &records[*right]);
    roots.sort_by(by_sort);
    for siblings in children.iter_mut() {
        siblings.sort_by(by_sort);
    }

    let mut layout: Vec<(usize, usize)> = Vec::with_capacity(records.len());
    let mut placed = vec![false; records.len()];
    let mut stack: Vec<(usize, usize)> = roots.iter().rev().map(|index| (*index, 0)).collect();
    while let Some((index, depth)) = stack.pop() {
        if placed[index] {
            continue;
        }
        placed[index] = true;
        layout.push((index, depth));
        for child in children[index].iter().rev() {
            stack.push((*child, depth + 1));
        }
    }

    // A parent cycle the API should never have sent would leave records
    // unreachable from any root. Show them as roots rather than lose them.
    let mut unreachable: Vec<usize> = (0..records.len()).filter(|index| !placed[*index]).collect();
    unreachable.sort_by(by_sort);
    layout.extend(unreachable.into_iter().map(|index| (index, 0)));

    let mut slots: Vec<Option<TaskRecord>> = records.into_iter().map(Some).collect();
    layout
        .into_iter()
        .filter_map(|(index, depth)| {
            let mut record = slots[index].take()?;
            record.subtask_depth = depth;
            if depth == 0 {
                record.parent_gid = None;
            }
            Some(record)
        })
        .collect()
}

/// One column per distinct custom-field *name*, valued per record.
fn custom_columns(
    records: &[TaskRecord],
    custom_field_definitions: &[CustomFieldDefinition],
) -> Vec<CustomFieldColumn> {
    group_custom_fields_by_name(custom_field_definitions)
        .into_iter()
        .map(|(name, gids)| {
            let values = records
                .iter()
                .map(|record| {
                    let values = gids
                        .iter()
                        .filter_map(|gid| record.custom_fields.get(gid))
                        .flat_map(|values| values.iter().cloned())
                        .collect::<Vec<_>>();
                    join_non_empty(&values)
                })
                .collect();
            CustomFieldColumn { name, gids, values }
        })
        .collect()
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
        let project = group_project(record).to_string();
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
        row.subtask_depth = record.subtask_depth;
        row.start = record.start_date.as_deref().and_then(CivilDate::parse);
        row.due = record.due_date.as_deref().and_then(CivilDate::parse);
        rows.push(row);
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::{
        CivilDate, CustomFieldDefinition, SortDirection, SubtaskVisibility, TaskFilter,
        TaskRecord, TaskRowKind, TaskSort, TaskSortField, TaskSortRule, TaskTableModel,
        TaskTableSettings,
    };

    /// The gids of the task rows a model holds, in table order.
    fn task_gids(model: &TaskTableModel) -> Vec<&str> {
        model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.as_str())
            .collect()
    }

    #[test]
    fn task_rows_carry_parsed_dates_alongside_their_display_cells() {
        let mut record = TaskRecord::new("t1", "Ship it");
        record.start_date = Some("2026-06-15".to_string());
        record.due_date = Some("2026-08-03".to_string());

        let model = TaskTableModel::from_records(vec![record], Vec::new());
        let row = model
            .rows
            .iter()
            .find(|row| row.kind.is_task())
            .expect("a task row");

        assert_eq!(row.start, CivilDate::new(2026, 6, 15));
        assert_eq!(row.due, CivilDate::new(2026, 8, 3));
        assert_eq!(row.cells[2], "2026-08-03", "the display cell stays a string");
    }

    #[test]
    fn a_date_the_api_sent_that_is_not_a_date_leaves_the_typed_field_empty() {
        let mut record = TaskRecord::new("t1", "Ship it");
        record.due_date = Some("someday".to_string());

        let model = TaskTableModel::from_records(vec![record], Vec::new());
        let row = model
            .rows
            .iter()
            .find(|row| row.kind.is_task())
            .expect("a task row");

        assert_eq!(row.due, None, "the chart draws nothing rather than guessing");
        assert_eq!(row.cells[2], "someday", "but the table still shows it");
    }

    #[test]
    fn header_and_spacer_rows_carry_no_dates() {
        let mut record = TaskRecord::new("t1", "Ship it");
        record.projects = vec!["Project".to_string()];
        record.sections = vec!["Section".to_string()];
        record.due_date = Some("2026-08-03".to_string());

        let model = TaskTableModel::from_records(vec![record], Vec::new());

        for row in model.rows.iter().filter(|row| !row.kind.is_task()) {
            assert_eq!(row.start, None);
            assert_eq!(row.due, None);
        }
    }

    #[test]
    fn groups_custom_fields_by_name_keeping_every_id() {
        use super::group_custom_fields_by_name;

        let definitions = vec![
            CustomFieldDefinition::new("cf-a", "Tag"),
            CustomFieldDefinition::new("cf-b", "Tag"),
            CustomFieldDefinition::new("cf-c", "Priority"),
            CustomFieldDefinition::new("cf-d", "Tag"),
        ];

        assert_eq!(
            group_custom_fields_by_name(&definitions),
            vec![
                (
                    "Tag".to_string(),
                    vec!["cf-a".to_string(), "cf-b".to_string(), "cf-d".to_string()]
                ),
                ("Priority".to_string(), vec!["cf-c".to_string()]),
            ],
            "one entry per name, in first-appearance order, and ids not adjacent \
             in the input are still gathered"
        );

        assert!(group_custom_fields_by_name(&[]).is_empty());
    }

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
    fn filters_by_completion_state() {
        let mut open = TaskRecord::new("1", "Ship release");
        open.projects = vec!["Inbox".to_string()];
        let mut done = TaskRecord::new("2", "Write docs");
        done.completed = true;
        done.projects = vec!["Inbox".to_string()];

        let gids = |completed| {
            let settings = TaskTableSettings {
                filter: TaskFilter {
                    completed,
                    ..TaskFilter::default()
                },
                sort: TaskSort::default(),
            };
            TaskTableModel::from_records_with_settings(
                vec![open.clone(), done.clone()],
                Vec::new(),
                &settings,
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

    #[test]
    fn subtasks_sit_under_their_own_parent_rather_than_in_a_heap_at_the_bottom() {
        // TaskSort::compare puts every parent before every subtask, so the
        // grouping in apply_task_filter is the only thing interleaving them. With
        // one parent and one child this passes either way — hence two.
        //
        // Both parents share one project and section, so the table inserts a
        // single pair of header rows ahead of all four tasks and `task_gids`
        // sees the task order alone.
        let parent = |gid: &str, name: &str| {
            let mut record = TaskRecord::new(gid, name);
            record.projects = vec!["Inbox".to_string()];
            record.sections = vec!["Today".to_string()];
            record
        };
        let child = |gid: &str, name: &str, parent_gid: &str| {
            let mut record = parent(gid, name);
            record.parent_gid = Some(parent_gid.to_string());
            record
        };

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
                filter: TaskFilter {
                    subtasks: SubtaskVisibility::Show,
                    ..TaskFilter::default()
                },
                sort: TaskSort::default(),
            },
        );

        assert_eq!(task_gids(&model), vec!["p1", "c1", "p2", "c2"]);

        let hidden = TaskTableModel::from_records_with_settings(
            records,
            Vec::new(),
            &TaskTableSettings {
                filter: TaskFilter {
                    subtasks: SubtaskVisibility::Hide,
                    ..TaskFilter::default()
                },
                sort: TaskSort::default(),
            },
        );

        assert_eq!(task_gids(&hidden), vec!["p1", "p2"]);
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
                project_order: Vec::new(),
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
                project_order: Vec::new(),
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
                project_order: Vec::new(),
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

    /// A record due on `due`, ordered `natural_order` by the API, in one
    /// ungrouped project so the date rule is the only thing deciding.
    fn dated(gid: &str, due: Option<&str>, natural_order: usize) -> TaskRecord {
        let mut record = TaskRecord::new(gid, format!("Task {gid}"));
        record.due_date = due.map(str::to_string);
        record.natural_order = natural_order;
        record
    }

    fn date_sorted_gids(records: Vec<TaskRecord>, direction: SortDirection) -> Vec<String> {
        let settings = TaskTableSettings {
            filter: TaskFilter::default(),
            sort: TaskSort {
                group_by_project: false,
                group_by_section: false,
                project_order: Vec::new(),
                rules: vec![TaskSortRule {
                    field: TaskSortField::Date,
                    direction,
                }],
            },
        };

        TaskTableModel::from_records_with_settings(records, vec![], &settings)
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.clone())
            .collect()
    }

    #[test]
    fn orders_dates_chronologically_across_month_and_year_boundaries() {
        // Every one of these pairs orders correctly by calendar date and would
        // still order correctly as ISO text; the point is that the rule reads
        // the date, not the "Today"/"Sep 1" text the table renders.
        let records = vec![
            dated("d", Some("2027-01-04"), 0),
            dated("b", Some("2026-09-01"), 1),
            dated("a", Some("2026-08-31"), 2),
            dated("c", Some("2026-12-31"), 3),
        ];

        assert_eq!(
            date_sorted_gids(records.clone(), SortDirection::Asc),
            vec!["a", "b", "c", "d"]
        );
        assert_eq!(
            date_sorted_gids(records, SortDirection::Desc),
            vec!["d", "c", "b", "a"]
        );
    }

    #[test]
    fn undated_tasks_sort_last_in_both_directions() {
        // An empty due date used to compare as the empty string, which sorted
        // ahead of every real date and put undated tasks at the top of an
        // ascending sort.
        let records = vec![
            dated("undated", None, 0),
            dated("late", Some("2026-09-30"), 1),
            dated("early", Some("2026-09-01"), 2),
        ];

        assert_eq!(
            date_sorted_gids(records.clone(), SortDirection::Asc),
            vec!["early", "late", "undated"]
        );
        assert_eq!(
            date_sorted_gids(records, SortDirection::Desc),
            vec!["late", "early", "undated"]
        );
    }

    #[test]
    fn a_value_that_is_not_a_real_date_sorts_with_the_undated() {
        let records = vec![
            dated("nonsense", Some("someday"), 0),
            dated("impossible", Some("2026-02-31"), 1),
            dated("real", Some("2026-09-01"), 2),
        ];

        assert_eq!(
            date_sorted_gids(records, SortDirection::Asc),
            vec!["real", "nonsense", "impossible"],
            "unparseable values fall back to natural order behind every real date"
        );
    }

    #[test]
    fn a_task_with_only_a_start_date_sorts_by_it() {
        let mut start_only = TaskRecord::new("start", "Start only");
        start_only.start_date = Some("2026-09-05".to_string());

        let records = vec![
            start_only,
            dated("before", Some("2026-09-01"), 1),
            dated("after", Some("2026-09-10"), 2),
        ];

        assert_eq!(
            date_sorted_gids(records, SortDirection::Asc),
            vec!["before", "start", "after"]
        );
    }

    #[test]
    fn toggling_the_direction_flips_only_the_primary_rule() {
        let mut sort = TaskSort::default();
        assert_eq!(sort.primary_direction(), SortDirection::Asc);

        sort.toggle_primary_direction();
        assert_eq!(sort.primary_direction(), SortDirection::Desc);
        assert!(
            sort.rules[1..]
                .iter()
                .all(|rule| rule.direction == SortDirection::Asc),
            "the tie-breakers keep their direction"
        );

        sort.toggle_primary_direction();
        assert_eq!(sort.primary_direction(), SortDirection::Asc);
    }

    #[test]
    fn toggling_the_direction_with_no_rules_makes_the_implicit_date_sort_explicit() {
        let mut sort = TaskSort {
            group_by_project: false,
            group_by_section: false,
            project_order: Vec::new(),
            rules: vec![],
        };

        sort.toggle_primary_direction();

        assert_eq!(
            sort.rules,
            vec![TaskSortRule {
                field: TaskSortField::Date,
                direction: SortDirection::Desc,
            }]
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
                project_order: Vec::new(),
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

    /// Project groups read down in the order the project list shows them, so
    /// the two panes agree — the pinned "No Project (Assigned to Me)" row
    /// leads the table even though its name sorts last alphabetically.
    #[test]
    fn project_groups_follow_the_project_list_order() {
        let record = |gid: &str, project: &str| {
            let mut record = TaskRecord::new(gid, format!("Task in {project}"));
            record.projects = vec![project.to_string()];
            record
        };

        let settings = TaskTableSettings {
            filter: TaskFilter::default(),
            sort: TaskSort {
                group_by_project: true,
                group_by_section: false,
                project_order: vec![
                    "No Project (Assigned to Me)".to_string(),
                    "Zeta".to_string(),
                    "Alpha".to_string(),
                ],
                rules: vec![],
            },
        };

        let model = TaskTableModel::from_records_with_settings(
            vec![
                record("a", "Alpha"),
                record("n", "No Project (Assigned to Me)"),
                record("z", "Zeta"),
            ],
            vec![],
            &settings,
        );

        let headers = model
            .rows
            .iter()
            .filter(|row| row.kind == TaskRowKind::ProjectHeader)
            .map(|row| row.cells[0].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            headers,
            vec!["No Project (Assigned to Me)", "Zeta", "Alpha"]
        );
    }

    /// A project the order says nothing about still needs a stable home: after
    /// every listed one, and alphabetical among its own kind.
    #[test]
    fn project_groups_missing_from_the_order_trail_the_listed_ones_by_name() {
        let record = |gid: &str, project: &str| {
            let mut record = TaskRecord::new(gid, format!("Task in {project}"));
            record.projects = vec![project.to_string()];
            record
        };

        let settings = TaskTableSettings {
            filter: TaskFilter::default(),
            sort: TaskSort {
                group_by_project: true,
                group_by_section: false,
                project_order: vec!["Zeta".to_string()],
                rules: vec![],
            },
        };

        let model = TaskTableModel::from_records_with_settings(
            vec![
                record("b", "Beta"),
                record("z", "Zeta"),
                record("a", "Alpha"),
            ],
            vec![],
            &settings,
        );

        let headers = model
            .rows
            .iter()
            .filter(|row| row.kind == TaskRowKind::ProjectHeader)
            .map(|row| row.cells[0].as_str())
            .collect::<Vec<_>>();
        assert_eq!(headers, vec!["Zeta", "Alpha", "Beta"]);
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
                project_order: Vec::new(),
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

    /// Regression test: a subtask whose parent is not in the record set — the
    /// assignee query returns these, and the filter panel makes more of them by
    /// dropping parents assigned to other people — used to sort to the very
    /// bottom of the table and render with no header of its own, so it read as
    /// part of whichever project group came last.
    #[test]
    fn an_orphan_subtask_is_grouped_under_its_own_project_not_the_last_header() {
        let mut root = TaskRecord::new("root", "Task in Zeta");
        root.projects = vec!["Zeta".to_string()];

        let mut orphan = TaskRecord::new("orphan", "Subtask in Alpha");
        orphan.parent_gid = Some("absent-parent".to_string());
        orphan.projects = vec!["Alpha".to_string()];
        orphan.subtask_depth = 1;

        let settings = TaskTableSettings {
            filter: TaskFilter {
                subtasks: SubtaskVisibility::Show,
                ..TaskFilter::default()
            },
            sort: TaskSort {
                group_by_project: true,
                group_by_section: false,
                project_order: Vec::new(),
                rules: vec![],
            },
        };

        let model =
            TaskTableModel::from_records_with_settings(vec![root, orphan], vec![], &settings);

        let headers = model
            .rows
            .iter()
            .filter(|row| row.kind == TaskRowKind::ProjectHeader)
            .map(|row| row.cells[0].as_str())
            .collect::<Vec<_>>();
        assert_eq!(headers, vec!["Alpha", "Zeta"]);

        let orphan_row = model
            .rows
            .iter()
            .find(|row| row.gid == "orphan")
            .expect("the orphan still has a row");
        assert_eq!(orphan_row.project.as_deref(), Some("Alpha"));
        assert_eq!(
            orphan_row.subtask_depth, 0,
            "with no parent on screen there is nothing to indent under"
        );

        let task_gids = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            task_gids,
            vec!["orphan", "root"],
            "the orphan sorts with its own project group instead of trailing the table"
        );
    }

    #[test]
    fn a_subtask_stays_under_its_parent_even_when_its_own_project_sorts_first() {
        let mut root = TaskRecord::new("root", "Task in Alpha");
        root.projects = vec!["Alpha".to_string()];

        let mut parent = TaskRecord::new("parent", "Parent in Zeta");
        parent.projects = vec!["Zeta".to_string()];

        let mut child = TaskRecord::new("child", "Subtask in Alpha");
        child.parent_gid = Some("parent".to_string());
        child.projects = vec!["Alpha".to_string()];
        child.subtask_depth = 1;

        let settings = TaskTableSettings {
            filter: TaskFilter {
                subtasks: SubtaskVisibility::Show,
                ..TaskFilter::default()
            },
            sort: TaskSort {
                group_by_project: true,
                group_by_section: false,
                project_order: Vec::new(),
                rules: vec![],
            },
        };

        let model = TaskTableModel::from_records_with_settings(
            vec![child, root, parent],
            vec![],
            &settings,
        );

        let layout = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| (row.gid.as_str(), row.subtask_depth))
            .collect::<Vec<_>>();
        assert_eq!(layout, vec![("root", 0), ("parent", 0), ("child", 1)]);
    }

    #[test]
    fn nesting_depth_counts_from_the_outermost_visible_task() {
        // The grandparent is missing, so the parent it left behind is the top of
        // the tree the user can see and its own child indents one level in.
        let mut parent = TaskRecord::new("parent", "Orphaned parent");
        parent.parent_gid = Some("absent-grandparent".to_string());
        parent.projects = vec!["Alpha".to_string()];
        parent.subtask_depth = 1;

        let mut child = TaskRecord::new("child", "Grandchild");
        child.parent_gid = Some("parent".to_string());
        child.projects = vec!["Alpha".to_string()];
        child.subtask_depth = 2;

        let settings = TaskTableSettings {
            filter: TaskFilter {
                subtasks: SubtaskVisibility::Show,
                ..TaskFilter::default()
            },
            sort: TaskSort::default(),
        };

        let model =
            TaskTableModel::from_records_with_settings(vec![parent, child], vec![], &settings);

        let layout = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| (row.gid.as_str(), row.subtask_depth))
            .collect::<Vec<_>>();
        assert_eq!(layout, vec![("parent", 0), ("child", 1)]);
    }

    /// A parent cycle is not something the API should ever send, but losing rows
    /// over it would be worse than showing them side by side.
    #[test]
    fn records_in_a_parent_cycle_are_still_shown() {
        let mut first = TaskRecord::new("a", "First");
        first.parent_gid = Some("b".to_string());
        first.projects = vec!["Alpha".to_string()];

        let mut second = TaskRecord::new("b", "Second");
        second.parent_gid = Some("a".to_string());
        second.projects = vec!["Alpha".to_string()];

        let settings = TaskTableSettings {
            filter: TaskFilter {
                subtasks: SubtaskVisibility::Show,
                ..TaskFilter::default()
            },
            sort: TaskSort::default(),
        };

        let model =
            TaskTableModel::from_records_with_settings(vec![first, second], vec![], &settings);

        let task_gids = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.as_str())
            .collect::<Vec<_>>();
        assert_eq!(task_gids.len(), 2, "both records survive the layout pass");
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
