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
pub struct TaskRow {
    pub gid: String,
    pub cells: Vec<String>,
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
        merged.sort_by(|left, right| {
            left.completed
                .cmp(&right.completed)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.gid.cmp(&right.gid))
        });

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

        let rows = merged
            .iter()
            .enumerate()
            .map(|(index, record)| {
                let mut cells = vec![
                    record.name.clone(),
                    join_non_empty(&record.sections),
                    record.assignee.clone().unwrap_or_default(),
                    record.due_date.clone().unwrap_or_default(),
                    record.start_date.clone().unwrap_or_default(),
                    if record.completed {
                        "done".to_string()
                    } else {
                        "open".to_string()
                    },
                    join_non_empty(&record.projects),
                ];

                for column in &custom_field_columns {
                    let value = column.values.get(index).cloned().unwrap_or_default();
                    cells.push(value);
                }

                TaskRow {
                    gid: record.gid.clone(),
                    cells,
                }
            })
            .collect();

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
        if !value.trim().is_empty() && !target.iter().any(|existing| existing == value) {
            target.push(value.clone());
        }
    }
}

fn join_non_empty(values: &[String]) -> String {
    values
        .iter()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::{CustomFieldDefinition, TaskRecord, TaskTableModel};

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
        assert_eq!(model.rows.len(), 1);
        assert_eq!(model.rows[0].cells[0], "Draft release");
        assert_eq!(model.rows[0].cells[1], "Backlog | Ready");
        assert_eq!(model.rows[0].cells[2], "Alex");
        assert_eq!(model.rows[0].cells[5], "done");
        assert_eq!(model.rows[0].cells[6], "Alpha | Beta");
        assert_eq!(model.rows[0].cells[7], "High | Urgent");
    }
}
