//! End-to-end editing, driven by the keys a user would actually press.
//!
//! Everything here goes through `handle_key_event`, so the bindings, the mode
//! switches, the context-sensitive picker keys, and the write path are all
//! under test together — a unit test of `commit_cell_edit` would pass with
//! `e` bound to nothing at all.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{thread, time::Duration};
use tuisana::{
    app::App,
    asana::{
        dto::{
            CustomFieldDto, EnumOptionDto, ProjectCustomFieldSettingDto, ProjectDto, TaskDto,
            TaskMembershipDto, TaskMembershipProjectDto, UserDto,
        },
        fake::FakeAsanaClient,
        AsanaClient,
    },
    config::{Config, ProjectVisibilityConfig},
    domain::{
        Project, ProjectEdit, TaskFieldEdit, ASSIGNEE_COLUMN, PROJECTS_COLUMN, STATE_COLUMN,
        TITLE_COLUMN,
    },
    input::KeyMap,
};

fn user(gid: &str, name: &str) -> UserDto {
    UserDto {
        gid: gid.to_string(),
        name: Some(name.to_string()),
        display_name: Some(name.to_string()),
    }
}

fn task(gid: &str, name: &str, assignee: &str, due: &str) -> TaskDto {
    TaskDto {
        gid: gid.to_string(),
        name: name.to_string(),
        completed: false,
        modified_at: Some("2026-06-01T00:00:00Z".to_string()),
        due_on: Some(due.to_string()),
        start_on: None,
        assignee: Some(match assignee {
            "alex" => user("user-alex", "alex"),
            _ => user("user-jo", "jo"),
        }),
        num_subtasks: 0,
        parent: None,
        memberships: vec![TaskMembershipDto {
            project: TaskMembershipProjectDto {
                gid: "project-1".to_string(),
                name: "Inbox".to_string(),
            },
            section: None,
        }],
        custom_fields: Vec::new(),
    }
}

fn client() -> FakeAsanaClient {
    FakeAsanaClient::new(vec![
        Project::new("project-1", "Inbox", true),
        // Somewhere to move a task to. It has no tasks of its own, so it
        // never loads — which is the point: a project you can file into is
        // not the same as a project you are looking at.
        Project::new("project-2", "Backlog", false),
    ])
        .with_current_user_gid("user-alex")
        .with_users(vec![
            ("user-alex", "alex"),
            ("user-jo", "jo"),
            // In the workspace, on no loaded task: the directory is the only
            // way to reach them.
            ("user-priya", "Priya Raman"),
        ])
        .with_custom_field_settings(
            "project-1",
            vec![ProjectCustomFieldSettingDto {
                gid: "setting-1".to_string(),
                custom_field: CustomFieldDto {
                    gid: "cf-priority".to_string(),
                    name: "Priority".to_string(),
                    resource_subtype: Some("enum".to_string()),
                    enum_options: vec![
                        EnumOptionDto {
                            gid: "opt-high".to_string(),
                            name: "High".to_string(),
                            enabled: true,
                        },
                        EnumOptionDto {
                            gid: "opt-low".to_string(),
                            name: "Low".to_string(),
                            enabled: true,
                        },
                    ],
                },
            }],
        )
        .with_tasks(
            "project-1",
            vec![
                task("t1", "Ship the release", "alex", "2026-06-10"),
                task("t2", "Write the changelog", "alex", "2026-06-11"),
                task("t3", "Cut the tag", "jo", "2026-06-12"),
            ],
        )
}

struct Session {
    app: App<FakeAsanaClient>,
    keymap: KeyMap,
    /// A second handle on the same fake backend.
    ///
    /// `App` owns its client, and the write log lives behind a shared handle,
    /// so this is how the test reads what was actually sent.
    client: FakeAsanaClient,
}

impl Session {
    fn start() -> Self {
        Self::start_with(client())
    }

    /// A session whose backend refuses every write to one task.
    fn refusing(gid: &str) -> Self {
        Self::start_with(client().with_update_failure(gid))
    }

    fn start_with(client: FakeAsanaClient) -> Self {
        // Projects the config never names start hidden, so the one with the
        // fixtures has to be named.
        let mut config = Config::default();
        config.project_visibility = vec![
            ProjectVisibilityConfig {
                gid: "project-1".to_string(),
                starred: true,
                hidden: false,
            },
            ProjectVisibilityConfig {
                gid: "project-2".to_string(),
                starred: false,
                hidden: false,
            },
        ];
        let mut app = App::new(config, client.clone());
        app.load_projects().expect("projects load");
        let keymap = app.keymap().expect("keymap builds");
        let mut session = Self {
            app,
            keymap,
            client,
        };
        // The assigned-to-me row sorts first, and the fake has no tasks
        // assigned; `j` moves onto Inbox, which is the project with fixtures.
        session.press(KeyCode::Char('j'), KeyModifiers::NONE);
        session.press(KeyCode::Char('t'), KeyModifiers::NONE);
        session.wait_for_tasks();
        session
    }

    fn press(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        self.app
            .handle_key_event(&self.keymap, KeyEvent::new(code, modifiers), 10)
            .expect("key handled");
        self.app.tasks.settle_table();
        self.app.settle_task_edits();
    }

    /// Cycles the completed filter round to "open + done".
    ///
    /// Without it a task marked done leaves the table, which is right and
    /// makes the row impossible to read back.
    fn show_open_and_done(&mut self) {
        for _ in 0..2 {
            self.press(KeyCode::Char('c'), KeyModifiers::NONE);
            self.wait_for_tasks();
        }
    }

    fn type_keys(&mut self, keys: &str) {
        for key in keys.chars() {
            self.press(KeyCode::Char(key), KeyModifiers::NONE);
        }
    }

    fn wait_for_tasks(&mut self) {
        for _ in 0..100 {
            self.app.poll_task_data();
            if !matches!(
                self.app.tasks.status(),
                tuisana::app::task::TaskStatus::Loading
            ) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.app.poll_task_data();
        self.app.tasks.settle_table();
    }

    /// The text one cell of one task row holds, by task gid.
    fn cell(&self, gid: &str, column: usize) -> String {
        self.app
            .tasks
            .table()
            .rows
            .iter()
            .find(|row| row.gid == gid)
            .and_then(|row| row.cells.get(column))
            .cloned()
            .unwrap_or_else(|| panic!("no row for {gid}"))
    }

    fn updates(&self) -> Vec<(String, TaskFieldEdit)> {
        self.client.update_calls()
    }

    fn project_updates(&self) -> Vec<ProjectEdit> {
        self.client.project_update_calls()
    }

    /// The task ids the recently-edited pane is holding.
    fn recent_gids(&self) -> Vec<String> {
        self.app
            .tasks
            .recent_table()
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.clone())
            .collect()
    }
}

#[test]
fn tab_completes_a_person_no_loaded_task_names() {
    // The directory built from loaded tasks can only offer people who
    // already have a task in view, which excludes the most common reason to
    // reassign one.
    let mut session = Session::start();

    session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    session.press(KeyCode::Char('e'), KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::CONTROL);
    session.type_keys("priya");
    session.press(KeyCode::Tab, KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(session.cell("t1", ASSIGNEE_COLUMN), "Priya Raman");
    assert_eq!(
        session.updates(),
        vec![(
            "t1".to_string(),
            TaskFieldEdit::Assignee(Some(tuisana::domain::AssigneeRef::new(
                "user-priya",
                "Priya Raman"
            )))
        )]
    );
}

#[test]
fn an_emptied_assignee_cell_unassigns_the_task() {
    let mut session = Session::start();

    session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    session.press(KeyCode::Char('e'), KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::CONTROL);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(session.cell("t1", ASSIGNEE_COLUMN), "");
    assert_eq!(
        session.updates(),
        vec![("t1".to_string(), TaskFieldEdit::Assignee(None))]
    );
}

#[test]
fn a_task_moved_out_of_the_project_in_view_lands_in_the_recently_edited_pane() {
    let mut session = Session::start();

    // Five columns right is Projects.
    for _ in 0..PROJECTS_COLUMN {
        session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    }
    session.press(KeyCode::Char('e'), KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::CONTROL);
    session.type_keys("back");
    session.press(KeyCode::Tab, KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        session.project_updates(),
        vec![
            ProjectEdit::add("t1", "project-2", "Backlog"),
            ProjectEdit::remove("t1", "project-1", "Inbox"),
        ],
        "one request each way, and no task field was touched"
    );
    assert!(session.updates().is_empty());

    // The cache is keyed by the project that loaded it, so the row has to be
    // dropped from the rebuild rather than wait for a refresh.
    assert!(
        !session
            .app
            .tasks
            .table()
            .rows
            .iter()
            .any(|row| row.gid == "t1"),
        "it is not in Inbox any more"
    );
    assert_eq!(session.recent_gids(), vec!["t1".to_string()]);
    assert_eq!(
        session.app.tasks.recent_selected_index(),
        Some(0),
        "and the cursor followed it there"
    );
}

#[test]
fn a_membership_write_that_fails_puts_the_task_back() {
    let mut session = Session::refusing("t1");

    for _ in 0..PROJECTS_COLUMN {
        session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    }
    session.press(KeyCode::Char('e'), KeyModifiers::NONE);
    session.type_keys("back");
    session.press(KeyCode::Tab, KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        session.project_updates(),
        vec![ProjectEdit::add("t1", "project-2", "Backlog")],
        "nothing was removed, because nothing was replaced"
    );
    assert_eq!(
        session.cell("t1", PROJECTS_COLUMN),
        "Inbox",
        "the optimistic add is rolled back"
    );
    assert!(session
        .app
        .tasks
        .edit_notice()
        .is_some_and(|notice| notice.contains("could not update 1 of 1")));
}

#[test]
fn an_assignee_is_retyped_on_the_cursor_row_and_sent_once() {
    let mut session = Session::start();

    // `l` onto Assignee, `e` to edit it, `ctrl-l` to empty it, then a name.
    session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    assert_eq!(session.app.tasks.selected_column(), ASSIGNEE_COLUMN);

    session.press(KeyCode::Char('e'), KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::CONTROL);
    // `j` is the value-picker key in this mode, and types on a text cell.
    session.type_keys("jo");
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(session.cell("t1", ASSIGNEE_COLUMN), "jo");
    assert_eq!(
        session.updates(),
        vec![(
            "t1".to_string(),
            TaskFieldEdit::Assignee(Some(tuisana::domain::AssigneeRef::new("user-jo", "jo")))
        )],
        "the gid behind the name, not the name"
    );
}

#[test]
fn d_marks_a_whole_selection_done_in_one_press() {
    let mut session = Session::start();
    session.show_open_and_done();

    // `space` selects and moves down, so two presses select two rows.
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    assert_eq!(session.app.tasks.selected_task_count(), 2);

    session.press(KeyCode::Char('d'), KeyModifiers::NONE);

    assert_eq!(session.cell("t1", STATE_COLUMN), "done");
    assert_eq!(session.cell("t2", STATE_COLUMN), "done");
    assert_eq!(session.updates().len(), 2, "one request per task");
    assert!(session
        .updates()
        .iter()
        .all(|(_, edit)| edit == &TaskFieldEdit::Completed(true)));

    // Pressing it again puts them back, which a per-task toggle on a mixed
    // selection could not do.
    session.press(KeyCode::Char('d'), KeyModifiers::NONE);
    assert_eq!(session.cell("t1", STATE_COLUMN), "open");
    assert_eq!(session.cell("t2", STATE_COLUMN), "open");
}

#[test]
fn a_title_is_refused_on_a_selection_and_nothing_is_sent() {
    let mut session = Session::start();

    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char('e'), KeyModifiers::NONE);

    assert_eq!(
        session.app.tasks.edit_notice(),
        Some("a title is edited one task at a time (2 selected)")
    );
    assert!(!session.app.tasks.cell_edit_open());
    assert!(session.updates().is_empty(), "nothing went over the wire");
    assert_eq!(session.app.tasks.selected_column(), TITLE_COLUMN);
}

#[test]
fn a_due_date_is_picked_on_the_calendar_and_committed_with_enter() {
    let mut session = Session::start();

    // `l` twice puts the column cursor on Due; `e` opens the picker.
    session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    session.press(KeyCode::Char('e'), KeyModifiers::NONE);
    assert_eq!(session.app.mode(), tuisana::config::Mode::Calendar);

    // `t` is today in the picker, and `enter` sends it.
    session.press(KeyCode::Char('t'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    let today = tuisana::domain::today().iso();
    assert_eq!(session.cell("t1", 2), today);
    assert_eq!(
        session.updates(),
        vec![("t1".to_string(), TaskFieldEdit::Due(Some(today)))]
    );
    assert_eq!(session.app.mode(), tuisana::config::Mode::Task);
}

#[test]
fn an_enum_custom_field_offers_the_options_the_project_declares() {
    let mut session = Session::start();

    // Title, Assignee, Due, Start, State, Projects, then Priority.
    for _ in 0..6 {
        session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    }
    session.press(KeyCode::Char('e'), KeyModifiers::NONE);
    assert!(
        session.app.tasks.cell_edit_is_options(),
        "an enum field is a picker: {:?}",
        session.app.tasks.edit_notice()
    );

    // No task carries a Priority at all, so `j` can only be offering a
    // declared option.
    session.press(KeyCode::Char('j'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        session.updates(),
        vec![(
            "t1".to_string(),
            TaskFieldEdit::CustomField {
                gid: "cf-priority".to_string(),
                value: Some(tuisana::domain::CustomFieldValue::Enum {
                    option_gid: "opt-high".to_string(),
                    name: "High".to_string(),
                }),
            }
        )]
    );
}

#[test]
fn a_failed_write_is_rolled_back_and_the_border_says_so() {
    // Optimism is worth it — nearly every write succeeds — but an optimistic
    // update that quietly diverges from the server is worse than either.
    let client = client().with_update_failure("t1");
    let mut config = Config::default();
    config.project_visibility = vec![ProjectVisibilityConfig {
        gid: "project-1".to_string(),
        starred: true,
        hidden: false,
    }];
    let mut app = App::new(config, client.clone());
    app.load_projects().expect("projects load");
    let keymap = app.keymap().expect("keymap builds");
    let mut session = Session {
        app,
        keymap,
        client,
    };
    session.press(KeyCode::Char('j'), KeyModifiers::NONE);
    session.press(KeyCode::Char('t'), KeyModifiers::NONE);
    session.wait_for_tasks();
    session.show_open_and_done();

    session.press(KeyCode::Char('d'), KeyModifiers::NONE);

    assert_eq!(
        session.cell("t1", STATE_COLUMN),
        "open",
        "the field went back to the value it had"
    );
    assert_eq!(
        session.app.tasks.edit_notice(),
        Some("could not update 1 of 1: backend error: 403")
    );
}

/// Keeps the fixture honest: `ProjectDto` is the shape the settings request
/// decodes, and the test project has to exist in the fake for a load to see
/// its custom fields at all.
#[test]
fn the_fixture_project_is_the_one_the_settings_belong_to() {
    let project = ProjectDto {
        gid: "project-1".to_string(),
        name: "Inbox".to_string(),
    };
    let client = client();

    assert_eq!(
        client
            .list_project_custom_field_settings(&project.gid)
            .expect("settings load")
            .len(),
        1
    );
}

