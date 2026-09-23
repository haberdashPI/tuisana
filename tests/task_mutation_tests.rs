//! What a write actually names, when reading has merged things that writing
//! must keep apart.
//!
//! One field name usually exists once per project, with a different gid in
//! each. The table merges them into one column; a write that picked the first
//! gid would set a field the task does not have — silently, in the wrong
//! project's data.

use tuisana::{
    app::{task::TaskState, task_edit::EditContext},
    asana::{
        dto::{
            CustomFieldDto, CustomFieldValueDto, EnumOptionDto, ProjectCustomFieldSettingDto,
            TaskDto, TaskMembershipDto, TaskMembershipProjectDto,
        },
        fake::FakeAsanaClient,
    },
    domain::{CustomFieldValue, Project, TaskFieldEdit},
};

fn priority_field(gid: &str) -> ProjectCustomFieldSettingDto {
    ProjectCustomFieldSettingDto {
        gid: format!("setting-{gid}"),
        custom_field: CustomFieldDto {
            gid: gid.to_string(),
            name: "Priority".to_string(),
            resource_subtype: Some("enum".to_string()),
            enum_options: vec![
                EnumOptionDto {
                    gid: format!("{gid}-high"),
                    name: "High".to_string(),
                    enabled: true,
                },
                EnumOptionDto {
                    gid: format!("{gid}-low"),
                    name: "Low".to_string(),
                    enabled: true,
                },
            ],
        },
    }
}

fn task(gid: &str, project: &str, field_gid: &str) -> TaskDto {
    TaskDto {
        gid: gid.to_string(),
        name: format!("Task {gid}"),
        completed: false,
        modified_at: Some("2026-06-01T00:00:00Z".to_string()),
        due_on: None,
        start_on: None,
        assignee: None,
        num_subtasks: 0,
        parent: None,
        memberships: vec![TaskMembershipDto {
            project: TaskMembershipProjectDto {
                gid: project.to_string(),
                name: project.to_string(),
            },
            section: None,
        }],
        custom_fields: vec![CustomFieldValueDto {
            gid: field_gid.to_string(),
            name: "Priority".to_string(),
            display_value: Some("Low".to_string()),
            enum_value: Some(EnumOptionDto {
                gid: format!("{field_gid}-low"),
                name: "Low".to_string(),
                enabled: true,
            }),
        }],
    }
}

/// Two projects, each declaring its own "Priority" under its own gid.
fn two_projects_each_declaring_priority() -> (TaskState, Vec<Project>) {
    let projects = vec![
        Project::new("alpha", "Alpha", true),
        Project::new("beta", "Beta", true),
    ];
    let client = FakeAsanaClient::new(projects.clone())
        .with_custom_field_settings("alpha", vec![priority_field("priority-in-alpha")])
        .with_custom_field_settings("beta", vec![priority_field("priority-in-beta")])
        .with_tasks("alpha", vec![task("a1", "alpha", "priority-in-alpha")])
        .with_tasks("beta", vec![task("b1", "beta", "priority-in-beta")]);

    let mut state = TaskState::new();
    state.set_completed_filter(None);
    state
        .load_task_dataset_for_projects(&client, &projects)
        .expect("tasks load");
    (state, projects)
}

/// Moves the row cursor onto the task with this gid.
fn select_task(state: &mut TaskState, gid: &str) {
    for _ in 0..state.table().rows.len() {
        let current = state
            .selected_index()
            .and_then(|index| state.table().rows.get(index))
            .map(|row| row.gid.clone());
        if current.as_deref() == Some(gid) {
            return;
        }
        state.move_down();
    }
    panic!("no row for {gid}");
}

#[test]
fn a_custom_field_write_uses_the_gid_from_the_tasks_own_project() {
    // "Priority" exists in both projects, with a different gid in each, and
    // the table merges them into one column by name.
    let (mut state, _) = two_projects_each_declaring_priority();
    let priority = state
        .table()
        .columns
        .iter()
        .position(|column| column == "Priority")
        .expect("one merged Priority column");
    assert_eq!(
        state
            .table()
            .columns
            .iter()
            .filter(|column| *column == "Priority")
            .count(),
        1,
        "reading merges the two declarations into one column"
    );

    select_task(&mut state, "b1");
    state.move_column(priority as i64);
    state.begin_cell_edit(&EditContext::default()).expect("the picker opens");
    state.cell_edit_cycle_value(-1);
    let edits = state.commit_cell_edit(&EditContext::default()).expect("the option resolves");

    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].gid, "b1");
    assert_eq!(
        edits[0].field,
        TaskFieldEdit::CustomField {
            gid: "priority-in-beta".to_string(),
            value: Some(CustomFieldValue::Enum {
                option_gid: "priority-in-beta-high".to_string(),
                name: "High".to_string(),
            }),
        },
        "the gid and the option gid both come from Beta's own declaration"
    );
}

#[test]
fn the_same_edit_on_the_other_projects_task_names_the_other_gid() {
    let (mut state, _) = two_projects_each_declaring_priority();
    let priority = state
        .table()
        .columns
        .iter()
        .position(|column| column == "Priority")
        .expect("one merged Priority column");

    select_task(&mut state, "a1");
    state.move_column(priority as i64);
    state.begin_cell_edit(&EditContext::default()).expect("the picker opens");
    state.cell_edit_cycle_value(-1);
    let edits = state.commit_cell_edit(&EditContext::default()).expect("the option resolves");

    assert_eq!(
        edits[0].field,
        TaskFieldEdit::CustomField {
            gid: "priority-in-alpha".to_string(),
            value: Some(CustomFieldValue::Enum {
                option_gid: "priority-in-alpha-high".to_string(),
                name: "High".to_string(),
            }),
        }
    );
}

#[test]
fn clearing_a_custom_field_sends_no_value_rather_than_leaving_it_alone() {
    let (mut state, _) = two_projects_each_declaring_priority();
    let priority = state
        .table()
        .columns
        .iter()
        .position(|column| column == "Priority")
        .expect("one merged Priority column");

    select_task(&mut state, "b1");
    state.move_column(priority as i64);
    state.begin_cell_edit(&EditContext::default()).expect("the picker opens");
    state.cell_edit_clear();
    let edits = state.commit_cell_edit(&EditContext::default()).expect("an empty value resolves");

    assert_eq!(
        edits[0].field,
        TaskFieldEdit::CustomField {
            gid: "priority-in-beta".to_string(),
            value: None,
        },
        "omitting the key would mean no change, which cannot clear anything"
    );
}
