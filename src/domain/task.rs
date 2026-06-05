use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub gid: String,
    pub name: String,
}

impl Section {
    pub fn new(gid: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            gid: gid.into(),
            name: name.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomFieldDefinition {
    pub gid: String,
    pub name: String,
}

impl CustomFieldDefinition {
    pub fn new(gid: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            gid: gid.into(),
            name: name.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomFieldColumn {
    pub definition: CustomFieldDefinition,
    pub values: Vec<String>,
}

impl CustomFieldColumn {
    pub fn value(&self) -> String {
        join_non_empty(&self.values)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRecord {
    pub gid: String,
    pub name: String,
    pub completed: bool,
    pub assignee: Option<String>,
    pub due_date: Option<String>,
    pub start_date: Option<String>,
    pub sections: Vec<String>,
    pub projects: Vec<String>,
    pub custom_fields: HashMap<String, Vec<String>>,
}

impl TaskRecord {
    pub fn new(gid: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            gid: gid.into(),
            name: name.into(),
            completed: false,
            assignee: None,
            due_date: None,
            start_date: None,
            sections: Vec::new(),
            projects: Vec::new(),
            custom_fields: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskRowKind {
    ProjectHeader,
    SectionSpacer,
    Task,
}

impl TaskRowKind {
    pub fn is_task(&self) -> bool {
        matches!(self, Self::Task)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRow {
    pub kind: TaskRowKind,
    pub gid: String,
    pub cells: Vec<String>,
}

impl TaskRow {
    pub fn task(gid: impl Into<String>, cells: Vec<String>) -> Self {
        Self {
            kind: TaskRowKind::Task,
            gid: gid.into(),
            cells,
        }
    }

    pub fn project_header(label: impl Into<String>, column_count: usize) -> Self {
        let mut cells = vec![String::new(); column_count];
        cells[0] = label.into();
        Self {
            kind: TaskRowKind::ProjectHeader,
            gid: String::new(),
            cells,
        }
    }

    pub fn section_spacer(column_count: usize) -> Self {
        Self {
            kind: TaskRowKind::SectionSpacer,
            gid: String::new(),
            cells: vec![String::new(); column_count],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskTableModel {
    pub columns: Vec<String>,
    pub custom_field_columns: Vec<CustomFieldColumn>,
    pub rows: Vec<TaskRow>,
}

impl Default for TaskTableModel {
    fn default() -> Self {
        Self::empty()
    }
}

impl TaskTableModel {
    pub fn empty() -> Self {
        Self {
            columns: default_columns(),
            custom_field_columns: Vec::new(),
            rows: Vec::new(),
        }
    }

    pub fn from_records(
        records: Vec<TaskRecord>,
        custom_field_definitions: Vec<CustomFieldDefinition>,
    ) -> Self {
        let mut merged = merge_records(records);
        merged.sort_by(|left, right| record_sort_key(left).cmp(&record_sort_key(right)));

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

        let rows = build_rows(&merged, &custom_field_columns);

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

    pub fn task_count(&self) -> usize {
        self.rows.iter().filter(|row| row.kind.is_task()).count()
    }

    pub fn selectable_row_indices(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.kind.is_task().then_some(index))
            .collect()
    }

    pub fn first_selectable_row_index(&self) -> Option<usize> {
        self.selectable_row_indices().into_iter().next()
    }

    pub fn last_selectable_row_index(&self) -> Option<usize> {
        self.selectable_row_indices().into_iter().last()
    }

    pub fn next_selectable_row_index(&self, current: usize, step: usize) -> Option<usize> {
        let selectable = self.selectable_row_indices();
        let current_position = selectable.iter().position(|&index| index == current)?;
        let next_position = (current_position + step.max(1)).min(selectable.len().saturating_sub(1));
        selectable.get(next_position).copied()
    }

    pub fn previous_selectable_row_index(&self, current: usize, step: usize) -> Option<usize> {
        let selectable = self.selectable_row_indices();
        let current_position = selectable.iter().position(|&index| index == current)?;
        let previous_position = current_position.saturating_sub(step.max(1));
        selectable.get(previous_position).copied()
    }

    pub fn selectable_position(&self, index: usize) -> Option<usize> {
        self.selectable_row_indices()
            .iter()
            .position(|&row_index| row_index == index)
            .map(|position| position + 1)
    }
}

fn default_columns() -> Vec<String> {
    vec![
        "Task".to_string(),
        "Section".to_string(),
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
                existing.completed |= record.completed;
                if existing.assignee.is_none() {
                    existing.assignee = record.assignee.clone();
                }
                if existing.due_date.is_none() {
                    existing.due_date = record.due_date.clone();
                }
                if existing.start_date.is_none() {
                    existing.start_date = record.start_date.clone();
                }
                merge_text_lists(&mut existing.sections, &record.sections);
                merge_text_lists(&mut existing.projects, &record.projects);
                for (field_gid, values) in &record.custom_fields {
                    existing
                        .custom_fields
                        .entry(field_gid.clone())
                        .and_modify(|existing_values| merge_text_lists(existing_values, values))
                        .or_insert_with(|| values.clone());
                }
            })
            .or_insert(record);
    }

    by_gid.into_values().collect()
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

fn record_sort_key(record: &TaskRecord) -> (String, String, String, String, String) {
    let project = record.projects.first().cloned().unwrap_or_default();
    let section = record.sections.first().cloned().unwrap_or_default();
    let date = record
        .due_date
        .clone()
        .or_else(|| record.start_date.clone())
        .unwrap_or_default();
    (
        project,
        section,
        date,
        sanitize_display_text(&record.name),
        record.gid.clone(),
    )
}

fn build_rows(merged: &[TaskRecord], custom_field_columns: &[CustomFieldColumn]) -> Vec<TaskRow> {
    let column_count = default_columns().len() + custom_field_columns.len();
    let mut rows = Vec::new();
    let mut current_project: Option<String> = None;
    let mut current_section: Option<String> = None;

    for (index, record) in merged.iter().enumerate() {
        let project = record.projects.first().cloned().unwrap_or_default();
        let section = record.sections.first().cloned().unwrap_or_default();

        if current_project.as_deref() != Some(project.as_str()) {
            let project_label = sanitize_display_text(&project);
            current_project = Some(project.clone());
            current_section = None;
            rows.push(TaskRow::project_header(project_label, column_count));
        }

        if current_section.as_deref() != Some(section.as_str()) {
            if current_section.is_some() {
                rows.push(TaskRow::section_spacer(column_count));
            }
            current_section = Some(section.clone());
        }

        let mut cells = vec![
            sanitize_display_text(&record.name),
            join_non_empty(&record.sections),
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

        rows.push(TaskRow::task(record.gid.clone(), cells));
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::{CustomFieldDefinition, TaskRecord, TaskRowKind, TaskTableModel};

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

        let model = TaskTableModel::from_records(
            vec![first, second],
            vec![CustomFieldDefinition::new("cf-1", "Priority")],
        );

        assert_eq!(model.columns[0], "Task");
        assert_eq!(model.columns.last().expect("custom field column"), "Priority");
        assert_eq!(model.task_count(), 1);
        assert_eq!(model.rows.len(), 2);
        assert_eq!(model.rows[0].kind, TaskRowKind::ProjectHeader);
        assert_eq!(model.rows[0].cells[0], "Alpha");
        assert_eq!(model.rows[1].kind, TaskRowKind::Task);
        assert_eq!(model.rows[1].cells[0], "Draft release");
        assert_eq!(model.rows[1].cells[1], "Backlog | Ready");
        assert_eq!(model.rows[1].cells[2], "Alex");
        assert_eq!(model.rows[1].cells[5], "done");
        assert_eq!(model.rows[1].cells[6], "Alpha | Beta");
        assert_eq!(model.rows[1].cells[7], "High | Urgent");
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
        assert_eq!(model.rows[0].kind, TaskRowKind::ProjectHeader);
        assert_eq!(model.rows[0].cells[0], "Northwind BTX 4412 Ph1 PSG");
        assert_eq!(model.rows[1].kind, TaskRowKind::Task);
        assert_eq!(model.rows[1].cells[0], "Northwind task");
        assert_eq!(model.rows[1].cells[1], "Study Kit Design and Shipment requirements");
        assert_eq!(model.rows[1].cells[2], "Morgan Ellis");
        assert_eq!(model.rows[1].cells[6], "Northwind BTX 4412 Ph1 PSG");
        assert_eq!(model.rows[1].cells[7], "High");
    }
}
