//! What a write actually names, when reading has merged things that writing
//! must keep apart.
//!
//! One field name usually exists once per project, with a different gid in
//! each. The table merges them into one column; a write that picked the first
//! gid would set a field the task does not have — silently, in the wrong
//! project's data.

use tuisana::{
    app::{
        task::{DraftCommit, TaskState},
        task_edit::EditContext,
    },
    asana::{
        dto::{
            CustomFieldDto, CustomFieldValueDto, EnumOptionDto, ProjectCustomFieldSettingDto,
            SectionDto, TaskDto, TaskMembershipDto, TaskMembershipProjectDto,
            TaskMembershipSectionDto,
        },
        fake::{FakeAsanaClient, StructuralCall},
        AsanaClient,
    },
    domain::{CustomFieldValue, NewTask, ParentEdit, Project, Section, TaskFieldEdit},
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
///
/// From the top every time, so a test can walk back to a row it has already
/// moved past.
fn select_task(state: &mut TaskState, gid: &str) {
    state.jump_top();
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
    let edits = state.commit_cell_edit(&EditContext::default()).expect("the option resolves")
        .fields;

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
    let edits = state.commit_cell_edit(&EditContext::default()).expect("the option resolves")
        .fields;

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
    let edits = state.commit_cell_edit(&EditContext::default()).expect("an empty value resolves")
        .fields;

    assert_eq!(
        edits[0].field,
        TaskFieldEdit::CustomField {
            gid: "priority-in-beta".to_string(),
            value: None,
        },
        "omitting the key would mean no change, which cannot clear anything"
    );
}

// ---- Structural edits ------------------------------------------------------
//
// Creating a task, re-parenting one, moving one between sections, and adding
// or removing a section. These change *which rows exist*, which is why none of
// them is a `TaskFieldEdit`: each one is a different endpoint with a different
// rollback.

/// Two sections, three tasks, one of them a subtask.
///
/// Enough shape to ask every structural question: a section to move out of
/// and one to move into, a parent to hang something off, and a subtask to
/// refuse a section move on.
fn sectioned_client() -> (FakeAsanaClient, Vec<Project>) {
    let projects = vec![Project::new("p1", "Northwind", true)];
    let client = FakeAsanaClient::new(projects.clone())
        .with_sections(
            "p1",
            vec![
                SectionDto {
                    gid: "sec-design".to_string(),
                    name: "Design".to_string(),
                },
                SectionDto {
                    gid: "sec-ship".to_string(),
                    name: "Ship".to_string(),
                },
            ],
        )
        .with_tasks(
            "p1",
            vec![
                placed_task("t1", "Draft the protocol", "sec-design", "Design", 1),
                placed_task("t2", "Ship the candidate", "sec-ship", "Ship", 0),
            ],
        )
        .with_subtasks(
            "t1",
            vec![placed_task("t1a", "Collect the samples", "sec-design", "Design", 0)],
        );
    (client, projects)
}

fn placed_task(
    gid: &str,
    name: &str,
    section_gid: &str,
    section: &str,
    num_subtasks: usize,
) -> TaskDto {
    TaskDto {
        gid: gid.to_string(),
        name: name.to_string(),
        completed: false,
        modified_at: Some("2026-06-01T00:00:00Z".to_string()),
        due_on: None,
        start_on: None,
        assignee: None,
        num_subtasks,
        parent: None,
        memberships: vec![TaskMembershipDto {
            project: TaskMembershipProjectDto {
                gid: "p1".to_string(),
                name: "Northwind".to_string(),
            },
            section: Some(TaskMembershipSectionDto {
                gid: section_gid.to_string(),
                name: section.to_string(),
            }),
        }],
        custom_fields: Vec::new(),
    }
}

/// A loaded pane over [`sectioned_client`], and a second handle on the fake so
/// a test can read what went over the wire.
fn sectioned_state() -> (TaskState, FakeAsanaClient, EditContext) {
    let (client, projects) = sectioned_client();
    let mut state = TaskState::new();
    state.set_completed_filter(None);
    state
        .load_task_dataset_for_projects(&client, &projects)
        .expect("tasks load");
    let context = EditContext {
        projects: vec![("p1".to_string(), "Northwind".to_string())],
        workspace_gid: Some("ws-1".to_string()),
        ..EditContext::default()
    };
    (state, client, context)
}

/// Types into whatever cell editor is open, a character at a time.
fn type_cell(state: &mut TaskState, text: &str) {
    for ch in text.chars() {
        state.cell_edit_push_char(ch);
    }
}

#[test]
fn a_new_task_inherits_the_project_and_section_the_view_was_showing() {
    let (mut state, _, context) = sectioned_state();
    select_task(&mut state, "t2");

    state.begin_draft_task(false, &context).expect("a draft opens");
    type_cell(&mut state, "Book the courier");

    assert_eq!(
        state.draft_commit(),
        Some(DraftCommit::Task(NewTask {
            name: "Book the courier".to_string(),
            parent_gid: None,
            project_gid: Some("p1".to_string()),
            workspace_gid: Some("ws-1".to_string()),
            section_gid: Some("sec-ship".to_string()),
        })),
        "project grouping and section grouping are both on, so both are named"
    );
}

#[test]
fn a_new_task_beside_a_subtask_is_a_sibling_subtask() {
    let (mut state, _, context) = sectioned_state();
    select_task(&mut state, "t1a");

    state.begin_draft_task(false, &context).expect("a draft opens");
    type_cell(&mut state, "Log the batch numbers");

    let Some(DraftCommit::Task(task)) = state.draft_commit() else {
        panic!("a task draft");
    };
    assert_eq!(task.parent_gid.as_deref(), Some("t1"));
    assert_eq!(
        task.section_gid, None,
        "a subtask belongs to its parent, so no section is sent"
    );
}

#[test]
fn a_subtask_draft_hangs_off_the_cursor_row() {
    let (mut state, _, context) = sectioned_state();
    select_task(&mut state, "t2");

    state.begin_draft_task(true, &context).expect("a draft opens");
    type_cell(&mut state, "Confirm the pickup window");

    let Some(DraftCommit::Task(task)) = state.draft_commit() else {
        panic!("a task draft");
    };
    assert_eq!(task.parent_gid.as_deref(), Some("t2"));
    assert_eq!(task.section_gid, None);
}

#[test]
fn a_draft_the_view_could_not_name_a_project_for_leaves_it_blank() {
    let (mut state, _, context) = sectioned_state();
    select_task(&mut state, "t2");
    // Grouping off and the `Projects` column scrolled out of view: the screen
    // never said which project the row was in.
    state.toggle_project_grouping();
    state.toggle_section_grouping();
    state.set_visible_columns(0..2);

    state.begin_draft_task(false, &context).expect("a draft opens");
    type_cell(&mut state, "Something loose");

    let Some(DraftCommit::Task(task)) = state.draft_commit() else {
        panic!("a task draft");
    };
    assert_eq!(task.project_gid, None, "a blank project is recoverable; a wrong one is not");
    assert_eq!(task.section_gid, None);
    assert_eq!(task.workspace_gid.as_deref(), Some("ws-1"));
}

#[test]
fn a_draft_is_on_screen_and_an_empty_title_sends_nothing() {
    let (mut state, _, context) = sectioned_state();
    select_task(&mut state, "t2");
    state.begin_draft_task(false, &context).expect("a draft opens");

    let draft = state.draft_gid().expect("a draft is open").to_string();
    let at = state
        .table()
        .rows
        .iter()
        .position(|row| row.gid == draft)
        .expect("the draft is a row");
    assert_eq!(
        state.selected_index(),
        Some(at),
        "the cursor follows the draft, so `i` costs one keystroke before typing"
    );

    assert_eq!(state.draft_commit(), None, "nothing to send");
    state.cancel_draft();
    assert_eq!(state.draft_gid(), None);
    assert!(
        !state.table().rows.iter().any(|row| row.gid == draft),
        "and the row goes with it"
    );
}

#[test]
fn a_draft_is_exempt_from_the_filters_that_would_hide_it() {
    let (mut state, _, context) = sectioned_state();
    select_task(&mut state, "t2");
    state.begin_draft_task(false, &context).expect("a draft opens");
    let draft = state.draft_gid().expect("a draft is open").to_string();

    // A filter no untitled row could match. A task that vanished the instant
    // you created it would not be a feature.
    state.filter_push_char('z');
    state.settle_table();

    assert!(state.table().rows.iter().any(|row| row.gid == draft));
}

#[test]
fn a_created_task_replaces_the_draft_and_keeps_the_cursor() {
    let (mut state, client, context) = sectioned_state();
    select_task(&mut state, "t2");
    state.begin_draft_task(false, &context).expect("a draft opens");
    type_cell(&mut state, "Book the courier");

    let Some(DraftCommit::Task(new_task)) = state.draft_commit() else {
        panic!("a task draft");
    };
    let created = client.create_task(&new_task).expect("the create succeeds");
    let gid = created.gid.clone();
    state.finish_draft_task(created);

    assert_eq!(state.draft_gid(), None);
    let row = state
        .table()
        .rows
        .iter()
        .find(|row| row.gid == gid)
        .expect("the real row is in the table");
    assert_eq!(row.cells[0], "Book the courier");
    assert_eq!(
        state.selected_index(),
        state.table().rows.iter().position(|row| row.gid == gid),
        "the cursor follows it to its real gid"
    );

    // Two requests, the second only because the draft carried a section.
    assert_eq!(
        client.structural_calls(),
        vec![
            StructuralCall::CreateTask(new_task),
            StructuralCall::AddTaskToSection {
                section_gid: "sec-ship".to_string(),
                task_gid: gid,
            },
        ]
    );
}

#[test]
fn moving_a_task_between_sections_appends_it_and_rolls_back_on_failure() {
    let (mut state, _, context) = sectioned_state();
    select_task(&mut state, "t1");

    let moved = state
        .section_move(1, &context)
        .expect("the move resolves")
        .expect("there is a section below");
    assert_eq!(moved.section_gid, "sec-ship");
    assert_eq!(moved.previous_gid.as_deref(), Some("sec-design"));

    state.apply_section_move_locally(&moved);
    assert_eq!(section_of(&state, "t1"), Some("Ship".to_string()));

    state.undo_section_move_locally(&moved);
    assert_eq!(section_of(&state, "t1"), Some("Design".to_string()));
}

#[test]
fn a_section_move_off_the_end_does_nothing() {
    let (mut state, _, context) = sectioned_state();
    select_task(&mut state, "t2");

    assert_eq!(
        state.section_move(1, &context).expect("the move resolves"),
        None,
        "there is no `no section` position below the last one"
    );

    select_task(&mut state, "t1");
    assert_eq!(state.section_move(-1, &context).expect("the move resolves"), None);
}

#[test]
fn a_section_move_is_refused_on_a_subtask() {
    let (mut state, _, context) = sectioned_state();
    select_task(&mut state, "t1a");

    let error = state
        .section_move(1, &context)
        .expect_err("a subtask is not in a section");
    assert!(error.contains("belongs to its parent"), "{error}");
}

#[test]
fn a_new_section_is_created_after_the_cursors_own() {
    let (mut state, _, context) = sectioned_state();
    select_task(&mut state, "t1");

    state.begin_draft_section(&context).expect("a draft opens");
    type_cell(&mut state, "Review");

    assert_eq!(
        state.draft_commit(),
        Some(DraftCommit::Section {
            project_gid: "p1".to_string(),
            name: "Review".to_string(),
            insert_after: Some("sec-design".to_string()),
        })
    );
}

#[test]
fn a_created_section_is_one_the_next_move_can_reach() {
    let (mut state, client, context) = sectioned_state();
    select_task(&mut state, "t1");
    state.begin_draft_section(&context).expect("a draft opens");
    type_cell(&mut state, "Review");

    let Some(DraftCommit::Section {
        project_gid,
        name,
        insert_after,
    }) = state.draft_commit()
    else {
        panic!("a section draft");
    };
    let created = client
        .create_section(&project_gid, &name, insert_after.as_deref())
        .expect("the create succeeds");
    let gid = created.gid.clone();
    state.finish_draft_section(Section::new(created.gid, created.name));

    select_task(&mut state, "t1");
    let moved = state
        .section_move(1, &context)
        .expect("the move resolves")
        .expect("the new section is below Design");
    assert_eq!(moved.section_gid, gid, "it landed between Design and Ship");
}

/// `X` takes the project's first *empty* section, not the cursor row's own.
///
/// Asana deletes a section only when it holds no tasks, and a section with no
/// tasks has no row for the cursor to stand on — so a key that read the cursor
/// row's section could only ever refuse.
#[test]
fn a_populated_project_has_no_section_to_delete() {
    let (mut state, _, context) = sectioned_state();
    select_task(&mut state, "t1");

    let error = state
        .section_to_delete(&context)
        .expect_err("both sections hold tasks");
    assert!(error.contains("Northwind"), "{error}");
    assert!(error.contains("still holds tasks"), "{error}");
}

#[test]
fn the_section_s_just_made_is_the_one_x_takes_back() {
    let (mut state, client, context) = sectioned_state();
    select_task(&mut state, "t1");
    state.begin_draft_section(&context).expect("a draft opens");
    type_cell(&mut state, "Review");
    let Some(DraftCommit::Section {
        project_gid,
        name,
        insert_after,
    }) = state.draft_commit()
    else {
        panic!("a section draft");
    };
    let created = client
        .create_section(&project_gid, &name, insert_after.as_deref())
        .expect("the create succeeds");
    state.finish_draft_section(Section::new(created.gid.clone(), created.name));

    select_task(&mut state, "t1");
    let target = state
        .section_to_delete(&context)
        .expect("Review is the only empty one");
    assert_eq!(target.section_gid, created.gid);
    assert_eq!(target.section_name, "Review");

    state.remove_section_locally(&target);
    assert!(
        state.section_to_delete(&context).is_err(),
        "and there is nothing left to take back"
    );
}

#[test]
fn marking_a_parent_shows_its_loaded_subtasks_as_going_too() {
    let (mut state, _, _) = sectioned_state();
    select_task(&mut state, "t1");

    state.toggle_deletion_marks();
    assert_eq!(state.marked_for_deletion_count(), 1, "one mark, one request");
    assert!(state.is_marked_for_deletion("t1"));
    assert!(
        state.is_marked_for_deletion("t1a"),
        "Asana deletes a parent's subtasks, so the confirmation has to show it"
    );
    assert!(!state.is_marked_for_deletion("t2"));

    state.toggle_deletion_marks();
    assert_eq!(state.marked_for_deletion_count(), 0, "`x` again unmarks");
}

#[test]
fn confirming_a_deletion_takes_the_subtasks_with_it_and_can_put_them_back() {
    let (mut state, _, _) = sectioned_state();
    select_task(&mut state, "t1");
    state.toggle_deletion_marks();

    let marked = state.marked_for_deletion();
    assert_eq!(marked, vec!["t1".to_string()]);

    let removed = state.remove_tasks_locally(&marked);
    assert_eq!(
        removed.iter().map(|record| record.gid.as_str()).collect::<Vec<_>>(),
        vec!["t1", "t1a"],
        "the limb goes whole"
    );
    assert_eq!(visible_gids(&state), vec!["t2"]);

    state.restore_tasks_locally(removed);
    assert_eq!(visible_gids(&state), vec!["t1", "t1a", "t2"]);
}

#[test]
fn the_parent_column_offers_the_cursors_own_section_first() {
    let (mut state, _, context) = sectioned_state();
    let parent = column_index(&state, "Parent");
    select_task(&mut state, "t1a");
    state.move_column(parent as i64);
    state.begin_cell_edit(&context).expect("the picker opens");

    assert_eq!(
        offered_candidates(&state),
        vec!["Ship the candidate"],
        "the parent it already has is not offered again"
    );

    state.cell_edit_clear();
    assert_eq!(
        offered_candidates(&state),
        vec!["Draft the protocol", "Ship the candidate"],
        "the task's own section first, then the rest of the project"
    );
}

/// The reported bug: the cell fell back to the gid as soon as the parent left
/// the table, which a filter does routinely — and a gid is not a name.
#[test]
fn the_parent_column_names_a_parent_the_filter_has_dropped() {
    let (mut state, _, _) = sectioned_state();
    assert_eq!(
        parent_cell(&state, "t1a"),
        Some("Draft the protocol".to_string()),
        "the parent is on screen, so its title is read off the row"
    );

    // A filter the parent cannot match but the subtask can. The parent is
    // still loaded — it is just not a row any more. `Title` is the panel's
    // first row and the cursor starts there.
    state.toggle_filter_panel();
    assert_eq!(
        state.filter_panel_rows().first().map(|(label, _)| label.as_str()),
        Some("Title")
    );
    for ch in "samples".chars() {
        state.filter_push_char(ch);
    }
    state.settle_table();

    assert_eq!(
        visible_gids(&state),
        vec!["t1a".to_string()],
        "only the subtask survives the filter"
    );
    assert_eq!(
        parent_cell(&state, "t1a"),
        Some("Draft the protocol".to_string()),
        "and it still says what it hangs off, by name"
    );
}

/// A parent the session has never loaded has no name to show, so the gid is
/// the honest answer — the same fallback `Projects` makes for a project it
/// never saw.
#[test]
fn a_parent_the_session_never_loaded_shows_its_gid() {
    let (mut state, _, _) = sectioned_state();

    // A gid no loaded task carries and no name came with — which is all a
    // reload of a subtask whose parent is someone else's can ever give.
    state.apply_parent_edits_locally(&[ParentEdit {
        gid: "t2".to_string(),
        parent_gid: Some("1209999999".to_string()),
        parent_name: None,
        previous_gid: None,
        previous_name: None,
    }]);

    assert_eq!(parent_cell(&state, "t2"), Some("1209999999".to_string()));
}

#[test]
fn the_parent_column_never_offers_the_task_itself_or_its_descendants() {
    let (mut state, _, context) = sectioned_state();
    let parent = column_index(&state, "Parent");
    select_task(&mut state, "t1");
    state.move_column(parent as i64);
    state.begin_cell_edit(&context).expect("the picker opens");

    let offered = offered_candidates(&state);
    assert_eq!(
        offered,
        vec!["Ship the candidate"],
        "a cycle Asana would reject anyway is not worth offering"
    );
}

#[test]
fn clearing_the_parent_cell_promotes_a_subtask_and_a_failure_rolls_back() {
    let (mut state, _, context) = sectioned_state();
    let parent = column_index(&state, "Parent");
    select_task(&mut state, "t1a");
    state.move_column(parent as i64);
    state.begin_cell_edit(&context).expect("the picker opens");
    state.cell_edit_clear();

    let edits = state.commit_cell_edit(&context).expect("an empty cell resolves").parents;
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].gid, "t1a");
    assert_eq!(edits[0].parent_gid, None);
    assert_eq!(edits[0].previous_gid.as_deref(), Some("t1"));

    state.apply_parent_edits_locally(&edits);
    assert_eq!(parent_cell(&state, "t1a"), Some(String::new()));

    let undo = edits.iter().map(|edit| edit.undo()).collect::<Vec<_>>();
    state.apply_parent_edits_locally(&undo);
    assert_eq!(parent_cell(&state, "t1a"), Some("Draft the protocol".to_string()));
}

#[test]
fn re_parenting_moves_the_whole_limb() {
    let (mut state, _, context) = sectioned_state();
    let parent = column_index(&state, "Parent");
    select_task(&mut state, "t1");
    state.move_column(parent as i64);
    state.begin_cell_edit(&context).expect("the picker opens");
    type_cell(&mut state, "Ship the candidate");

    let edits = state.commit_cell_edit(&context).expect("the title resolves").parents;
    assert_eq!(edits[0].parent_gid.as_deref(), Some("t2"));
    state.apply_parent_edits_locally(&edits);

    // `t1a` was two levels down from nothing; it is now three.
    let depth = |gid: &str| {
        state
            .table()
            .rows
            .iter()
            .find(|row| row.gid == gid)
            .map(|row| row.subtask_depth)
    };
    assert_eq!(depth("t2"), Some(0));
    assert_eq!(depth("t1"), Some(1));
    assert_eq!(depth("t1a"), Some(2), "the depth is recomputed from the chain");
}

/// The section heading a row is drawn under.
fn section_of(state: &TaskState, gid: &str) -> Option<String> {
    state
        .table()
        .rows
        .iter()
        .find(|row| row.gid == gid)
        .and_then(|row| row.section.clone())
        .filter(|section| !section.is_empty())
}

/// The `Parent` cell of a row.
fn parent_cell(state: &TaskState, gid: &str) -> Option<String> {
    state
        .table()
        .rows
        .iter()
        .find(|row| row.gid == gid)
        .and_then(|row| row.cells.get(tuisana::domain::PARENT_COLUMN))
        .cloned()
}

/// The task gids the table is showing, in order.
fn visible_gids(state: &TaskState) -> Vec<String> {
    state
        .table()
        .rows
        .iter()
        .filter(|row| row.kind.is_task())
        .map(|row| row.gid.clone())
        .collect()
}

fn column_index(state: &TaskState, name: &str) -> usize {
    state
        .table()
        .columns
        .iter()
        .position(|column| column == name)
        .unwrap_or_else(|| panic!("no {name} column"))
}

/// What the open completion editor is offering, in offer order.
fn offered_candidates(state: &TaskState) -> Vec<String> {
    state
        .open_completion_candidates()
        .into_iter()
        .map(|candidate| candidate.to_string())
        .collect()
}
