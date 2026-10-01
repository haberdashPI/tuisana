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
            CustomFieldDto, EnumOptionDto, ProjectCustomFieldSettingDto, ProjectDto, SectionDto,
            TaskDto, TaskMembershipDto, TaskMembershipProjectDto, TaskMembershipSectionDto,
            UserDto,
        },
        fake::{FakeAsanaClient, StructuralCall},
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

    /// A session that asks before changing more than `threshold` tasks.
    ///
    /// The fixtures are three tasks and the default threshold is five, so
    /// every other test here never meets a confirmation. Lowering it is how
    /// the gate is reached without a hundred rows of fixture.
    fn confirming_above(threshold: usize) -> Self {
        Self::start_with_config(client(), |config| {
            config.edit.confirm_threshold = threshold;
        })
    }

    fn start_with(client: FakeAsanaClient) -> Self {
        Self::start_with_config(client, |_| {})
    }

    fn start_with_config(client: FakeAsanaClient, tune: impl FnOnce(&mut Config)) -> Self {
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
        tune(&mut config);
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

    /// The task the row cursor is on.
    fn cursor_gid(&self) -> Option<String> {
        self.app
            .tasks
            .selected_index()
            .and_then(|index| self.app.tasks.table().rows.get(index))
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.clone())
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
    session.press(KeyCode::Enter, KeyModifiers::NONE);
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
    session.press(KeyCode::Enter, KeyModifiers::NONE);
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
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::CONTROL);
    session.type_keys("back");
    session.press(KeyCode::Tab, KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    // Sorted, because the two requests go out on their own threads and the
    // fake logs them in whatever order they land.
    let mut sent = session.project_updates();
    sent.sort_by_key(|edit| edit.project_gid.clone());
    assert_eq!(
        sent,
        vec![
            ProjectEdit::remove("t1", "project-1", "Inbox"),
            ProjectEdit::add("t1", "project-2", "Backlog"),
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
    session.press(KeyCode::Enter, KeyModifiers::NONE);
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

    session.press(KeyCode::Enter, KeyModifiers::NONE);
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
fn a_bulk_edit_past_the_threshold_sends_nothing_until_it_is_confirmed() {
    let mut session = Session::confirming_above(1);
    session.show_open_and_done();

    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char('d'), KeyModifiers::NONE);

    // Not even the optimistic local update: the row the user is looking at has
    // to agree with the server right up until they say yes.
    assert_eq!(session.app.mode(), tuisana::config::Mode::Confirm);
    assert_eq!(
        session.app.pending_bulk_edit_view(),
        Some((2, "set State to done".to_string()))
    );
    assert!(session.updates().is_empty(), "nothing went over the wire");
    assert_eq!(session.cell("t1", STATE_COLUMN), "open");
    assert_eq!(session.cell("t2", STATE_COLUMN), "open");

    session.press(KeyCode::Char('y'), KeyModifiers::NONE);

    assert_eq!(session.app.mode(), tuisana::config::Mode::Task);
    assert!(session.app.pending_bulk_edit_view().is_none());
    assert_eq!(session.updates().len(), 2);
    assert_eq!(session.cell("t1", STATE_COLUMN), "done");
    assert_eq!(session.cell("t2", STATE_COLUMN), "done");
}

#[test]
fn answering_no_leaves_every_row_exactly_as_it_was() {
    let mut session = Session::confirming_above(1);

    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char('d'), KeyModifiers::NONE);
    session.press(KeyCode::Char('n'), KeyModifiers::NONE);

    assert_eq!(session.app.mode(), tuisana::config::Mode::Task);
    assert!(session.updates().is_empty());
    assert_eq!(session.cell("t1", STATE_COLUMN), "open");
    // Said back, because a cancelled bulk edit that reported nothing would be
    // indistinguishable from a keypress that did not register.
    assert_eq!(
        session.app.tasks.edit_notice(),
        Some("cancelled: 2 tasks unchanged")
    );
    // Still selected: `n` means "not like that", not "forget what I picked".
    assert_eq!(session.app.tasks.selected_task_count(), 2);
}

#[test]
fn esc_answers_a_confirmation_the_same_way_no_does() {
    let mut session = Session::confirming_above(1);

    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char('d'), KeyModifiers::NONE);
    session.press(KeyCode::Esc, KeyModifiers::NONE);

    assert_eq!(session.app.mode(), tuisana::config::Mode::Task);
    assert!(session.updates().is_empty());
}

/// Otherwise `j` would move a cursor nobody can see, and the next `y` would
/// confirm an edit the user had stopped looking at.
#[test]
fn a_confirmation_swallows_every_key_that_is_not_an_answer() {
    let mut session = Session::confirming_above(1);

    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    let row = session.app.tasks.selected_task_position();
    session.press(KeyCode::Char('d'), KeyModifiers::NONE);

    session.press(KeyCode::Char('j'), KeyModifiers::NONE);
    session.press(KeyCode::Char('q'), KeyModifiers::NONE);
    session.press(KeyCode::Char('?'), KeyModifiers::NONE);

    assert_eq!(session.app.mode(), tuisana::config::Mode::Confirm);
    assert_eq!(session.app.tasks.selected_task_position(), row);
    assert!(session.updates().is_empty());
}

#[test]
fn an_edit_at_the_threshold_goes_without_asking() {
    // Two rows against a threshold of two: "more than a few" is strictly
    // more, so this is the largest edit that still happens on the keystroke.
    let mut session = Session::confirming_above(2);
    session.show_open_and_done();

    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char('d'), KeyModifiers::NONE);

    assert_eq!(session.app.mode(), tuisana::config::Mode::Task);
    assert_eq!(session.updates().len(), 2);
    assert_eq!(session.cell("t1", STATE_COLUMN), "done");
}

#[test]
fn a_confirmed_cell_edit_names_the_column_and_the_value_it_will_write() {
    let mut session = Session::confirming_above(1);

    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    // Onto Assignee, then type a name from the directory and commit.
    session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::CONTROL);
    session.type_keys("priya");
    session.press(KeyCode::Tab, KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        session.app.pending_bulk_edit_view(),
        Some((2, "set Assignee to Priya Raman".to_string()))
    );
    // The editor is already closed: the value is decided, and the question is
    // only about how many rows it lands on.
    assert!(!session.app.tasks.cell_edit_open());

    session.press(KeyCode::Char('y'), KeyModifiers::NONE);
    assert_eq!(session.updates().len(), 2);
    assert_eq!(session.cell("t1", ASSIGNEE_COLUMN), "Priya Raman");
}

#[test]
fn a_projects_edit_is_counted_in_writes_rather_than_in_tasks() {
    let mut session = Session::confirming_above(1);

    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    for _ in 0..PROJECTS_COLUMN {
        session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    }
    // Backlog added, Inbox kept: one write per task, phrased the same way, so
    // the summary is the phrase rather than a count.
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.type_keys("back");
    session.press(KeyCode::Tab, KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        session.app.pending_bulk_edit_view(),
        Some((2, "add to Backlog".to_string()))
    );

    session.press(KeyCode::Char('y'), KeyModifiers::NONE);
    assert_eq!(session.project_updates().len(), 2);
}

/// Two tasks, four writes — because "be in these projects" is an `add` for a
/// task that is not and a `remove` for one that is. The phrases disagree, so
/// the summary counts instead of picking one and misreporting the other.
#[test]
fn a_mixture_of_changes_is_summarised_as_a_count() {
    let mut session = Session::confirming_above(1);

    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    for _ in 0..PROJECTS_COLUMN {
        session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    }
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::CONTROL);
    session.type_keys("back");
    session.press(KeyCode::Tab, KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        session.app.pending_bulk_edit_view(),
        Some((4, "make 4 changes".to_string())),
        "two tasks leaving Inbox for Backlog is four requests"
    );
}

#[test]
fn a_title_is_refused_on_a_selection_and_nothing_is_sent() {
    let mut session = Session::start();

    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

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
    session.press(KeyCode::Enter, KeyModifiers::NONE);
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
fn clearing_a_date_on_the_calendar_sends_the_cleared_value_and_closes() {
    let mut session = Session::start();

    // Put a date on the cell first, so clearing has something to undo.
    session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.press(KeyCode::Char('t'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(session.cell("t1", 2), tuisana::domain::today().iso());

    // `d` in the picker clears and saves in one gesture: leaving it open
    // would mean `enter` filled the highlighted day back in.
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(session.app.mode(), tuisana::config::Mode::Calendar);
    session.press(KeyCode::Char('d'), KeyModifiers::NONE);

    assert_eq!(session.app.mode(), tuisana::config::Mode::Task);
    assert!(!session.app.tasks.cell_edit_open());
    assert_eq!(session.cell("t1", 2), "");
    assert_eq!(
        session.updates().last(),
        Some(&("t1".to_string(), TaskFieldEdit::Due(None)))
    );
}

#[test]
fn an_enum_custom_field_offers_the_options_the_project_declares() {
    let mut session = Session::start();

    // Title, Assignee, Due, Start, State, Projects, Parent, then Priority.
    for _ in 0..tuisana::domain::FIRST_CUSTOM_COLUMN {
        session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    }
    session.press(KeyCode::Enter, KeyModifiers::NONE);
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
fn a_failed_write_is_rolled_back_and_the_notice_says_so() {
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

#[test]
fn esc_dismisses_the_notice_without_costing_the_key_its_own_job() {
    let mut session = Session::start();

    // A refusal rather than a failed write: it needs no backend, and it is
    // the case where `esc` already means something in the same mode.
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    assert!(session.app.tasks.edit_notice().is_some());

    session.press(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(session.app.tasks.edit_notice(), None);

    // And the same press still cancels an open edit, rather than being spent
    // on the notice: `l` onto Assignee, `enter` to open it, then a bad name.
    session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::CONTROL);
    session.type_keys("nobody");
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        session.app.tasks.edit_notice(),
        Some("no one called nobody is loaded")
    );
    assert!(session.app.tasks.cell_edit_open(), "the edit stays open");

    session.press(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(session.app.tasks.edit_notice(), None);
    assert!(!session.app.tasks.cell_edit_open(), "one press did both");
    assert_eq!(session.app.mode(), tuisana::config::Mode::Task);
}

/// Keeps the fixture honest: `ProjectDto` is the shape the settings request
/// decodes, and the test project has to exist in the fake for a load to see
/// its custom fields at all.
#[test]
fn the_fixture_project_is_the_one_the_settings_belong_to() {
    let project = ProjectDto {
        gid: "project-1".to_string(),
        name: "Inbox".to_string(),
        workspace: None,
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


// ---- Edit mode -------------------------------------------------------------
//
// The structural edits, through the keys that drive them. A unit test of
// `begin_draft_task` would pass with `t` bound to nothing at all, and the mode
// split is the whole reason `i` and `x` can mean insert and delete here while
// still meaning invert and clear one mode over.

/// The fixture, with two sections and a subtask.
///
/// Enough shape to move a task between sections, refuse the move on a
/// subtask, and delete a section that is empty.
fn sectioned_client() -> FakeAsanaClient {
    fn placed(gid: &str, name: &str, section: (&str, &str), subtasks: usize) -> TaskDto {
        TaskDto {
            gid: gid.to_string(),
            name: name.to_string(),
            completed: false,
            modified_at: Some("2026-06-01T00:00:00Z".to_string()),
            due_on: None,
            start_on: None,
            assignee: None,
            num_subtasks: subtasks,
            parent: None,
            memberships: vec![TaskMembershipDto {
                project: TaskMembershipProjectDto {
                    gid: "project-1".to_string(),
                    name: "Inbox".to_string(),
                },
                section: Some(TaskMembershipSectionDto {
                    gid: section.0.to_string(),
                    name: section.1.to_string(),
                }),
            }],
            custom_fields: Vec::new(),
        }
    }

    FakeAsanaClient::new(vec![Project::new("project-1", "Inbox", true)])
        .with_current_user_gid("user-alex")
        .with_sections(
            "project-1",
            vec![
                SectionDto {
                    gid: "sec-open".to_string(),
                    name: "Open".to_string(),
                },
                SectionDto {
                    gid: "sec-done".to_string(),
                    name: "Done".to_string(),
                },
            ],
        )
        .with_tasks(
            "project-1",
            vec![
                placed("t1", "Ship the release", ("sec-open", "Open"), 1),
                placed("t2", "Write the changelog", ("sec-done", "Done"), 0),
            ],
        )
        .with_subtasks(
            "project-1-sub",
            Vec::new(),
        )
        .with_subtasks(
            "t1",
            vec![placed("t1a", "Tag the commit", ("sec-open", "Open"), 0)],
        )
}

/// A session in edit mode, over the sectioned fixture.
fn edit_session() -> Session {
    let mut session = Session::start_with(sectioned_client());
    session.press(KeyCode::Char('t'), KeyModifiers::NONE);
    assert_eq!(session.app.mode(), tuisana::config::Mode::Edit);
    session
}

/// The task gids the table is showing, in order.
fn visible_gids(session: &Session) -> Vec<String> {
    session
        .app
        .tasks
        .table()
        .rows
        .iter()
        .filter(|row| row.kind.is_task())
        .map(|row| row.gid.clone())
        .collect()
}

/// Moves the row cursor onto the task with this gid, from the top.
fn move_to_task(session: &mut Session, gid: &str) {
    session.press(KeyCode::Home, KeyModifiers::NONE);
    for _ in 0..session.app.tasks.table().rows.len() {
        if session.cursor_gid().as_deref() == Some(gid) {
            return;
        }
        session.press(KeyCode::Char('j'), KeyModifiers::NONE);
    }
    panic!("no row for {gid}");
}

#[test]
fn t_enters_edit_mode_and_esc_leaves_it() {
    let mut session = Session::start();

    session.press(KeyCode::Char('t'), KeyModifiers::NONE);
    assert_eq!(session.app.mode(), tuisana::config::Mode::Edit);

    session.press(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(session.app.mode(), tuisana::config::Mode::Task);
}

#[test]
fn i_and_x_keep_their_task_mode_meanings_and_gain_their_edit_mode_ones() {
    let mut session = Session::start();

    // Task mode: `space` then `i` inverts the selection, `x` clears it.
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    assert_eq!(session.app.tasks.selected_task_count(), 1);
    session.press(KeyCode::Char('i'), KeyModifiers::NONE);
    assert_eq!(session.app.tasks.selected_task_count(), 2, "inverted, not inserted");
    session.press(KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(session.app.tasks.selected_task_count(), 0, "cleared, not deleted");

    // Edit mode: the same two letters insert and delete.
    session.press(KeyCode::Char('t'), KeyModifiers::NONE);
    session.press(KeyCode::Char('i'), KeyModifiers::NONE);
    assert!(session.app.tasks.draft_gid().is_some(), "a draft opened");
    session.press(KeyCode::Esc, KeyModifiers::NONE);

    session.press(KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(session.app.tasks.marked_for_deletion_count(), 1);
}

#[test]
fn an_uppercase_letter_is_a_key_of_its_own() {
    let mut session = edit_session();
    move_to_task(&mut session, "t1");

    // `J` moves the task down a section; `j` moves the cursor.
    session.press(KeyCode::Char('J'), KeyModifiers::SHIFT);
    assert_eq!(
        session.client.structural_calls(),
        vec![StructuralCall::AddTaskToSection {
            section_gid: "sec-done".to_string(),
            task_gid: "t1".to_string(),
        }]
    );
}

#[test]
fn i_then_a_title_then_enter_creates_a_task_where_the_cursor_stood() {
    let mut session = edit_session();
    move_to_task(&mut session, "t2");

    session.press(KeyCode::Char('i'), KeyModifiers::NONE);
    assert_eq!(session.app.mode(), tuisana::config::Mode::ColumnEdit);
    session.type_keys("Cut the tag");
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(session.app.mode(), tuisana::config::Mode::Edit, "back to edit mode");
    assert_eq!(
        session.client.structural_calls(),
        vec![
            StructuralCall::CreateTask(tuisana::domain::NewTask {
                name: "Cut the tag".to_string(),
                parent_gid: None,
                project_gid: Some("project-1".to_string()),
                workspace_gid: None,
                section_gid: Some("sec-done".to_string()),
            }),
            StructuralCall::AddTaskToSection {
                section_gid: "sec-done".to_string(),
                task_gid: "new-1".to_string(),
            },
        ]
    );
    assert_eq!(session.cursor_gid().as_deref(), Some("new-1"), "the cursor follows it");
}

#[test]
fn a_create_that_fails_keeps_the_draft_and_its_title_on_screen() {
    let mut session = Session::start_with(sectioned_client().with_structural_failure("sec-done"));
    session.press(KeyCode::Char('t'), KeyModifiers::NONE);
    move_to_task(&mut session, "t2");

    session.press(KeyCode::Char('i'), KeyModifiers::NONE);
    session.type_keys("Cut the tag");
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(session.app.mode(), tuisana::config::Mode::ColumnEdit, "the editor stays open");
    assert!(session.app.tasks.draft_gid().is_some(), "and so does the draft");
    assert!(
        session
            .app
            .tasks
            .edit_notice()
            .is_some_and(|notice| notice.contains("could not create")),
        "with the reason in the corner: {:?}",
        session.app.tasks.edit_notice()
    );
}

#[test]
fn esc_on_a_draft_discards_it_with_nothing_sent() {
    let mut session = edit_session();

    session.press(KeyCode::Char('i'), KeyModifiers::NONE);
    session.type_keys("Never mind");
    session.press(KeyCode::Esc, KeyModifiers::NONE);

    assert_eq!(session.app.mode(), tuisana::config::Mode::Edit);
    assert_eq!(session.app.tasks.draft_gid(), None);
    assert!(session.client.structural_calls().is_empty());
}

#[test]
fn i_then_enter_repeats_as_a_run_of_new_tasks() {
    let mut session = edit_session();
    move_to_task(&mut session, "t2");

    // `i` types while the draft's cell is open — every letter does, which is
    // what makes a title typeable — so a run of new tasks is `i`, the title,
    // `enter`, and again.
    for title in ["First", "Second"] {
        session.press(KeyCode::Char('i'), KeyModifiers::NONE);
        session.type_keys(title);
        session.press(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(session.app.mode(), tuisana::config::Mode::Edit);
    }

    let created = session
        .client
        .structural_calls()
        .into_iter()
        .filter_map(|call| match call {
            StructuralCall::CreateTask(task) => Some(task.name),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(created, vec!["First".to_string(), "Second".to_string()]);
}

#[test]
fn capital_i_opens_a_draft_whose_parent_is_the_cursor_row() {
    let mut session = edit_session();
    move_to_task(&mut session, "t2");

    session.press(KeyCode::Char('I'), KeyModifiers::SHIFT);
    session.type_keys("Check the diff");
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        session.client.structural_calls(),
        vec![StructuralCall::CreateTask(tuisana::domain::NewTask {
            name: "Check the diff".to_string(),
            parent_gid: Some("t2".to_string()),
            // The parent, and nothing else. Naming the project as well would
            // make the subtask a direct member of it, which Asana files in
            // the project's first section — so it would come back as a
            // top-level row under an arbitrary heading rather than as a
            // subtask. Same reason there is no section, and so no second
            // request.
            project_gid: None,
            workspace_gid: None,
            section_gid: None,
        })]
    );
}

#[test]
fn capital_s_adds_a_section_after_the_cursors_own() {
    let mut session = edit_session();
    move_to_task(&mut session, "t1");

    session.press(KeyCode::Char('S'), KeyModifiers::SHIFT);
    assert_eq!(session.app.mode(), tuisana::config::Mode::ColumnEdit);
    session.type_keys("Review");
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        session.client.structural_calls(),
        vec![StructuralCall::CreateSection {
            project_gid: "project-1".to_string(),
            name: "Review".to_string(),
            insert_after: Some("sec-open".to_string()),
        }]
    );
}

/// `S` then `X` is the round trip: add a section, then take it back.
///
/// `X` never names a section that still holds tasks, because Asana would
/// refuse it — and a section that holds none has no row for the cursor to
/// stand on, so it takes the project's first empty one.
#[test]
fn capital_x_takes_back_the_section_capital_s_added() {
    let mut session = edit_session();
    move_to_task(&mut session, "t1");

    // Both of the fixture's sections hold tasks, so there is nothing to take.
    session.press(KeyCode::Char('X'), KeyModifiers::SHIFT);
    assert!(
        session
            .app
            .tasks
            .edit_notice()
            .is_some_and(|notice| notice.contains("still holds tasks")),
        "{:?}",
        session.app.tasks.edit_notice()
    );
    assert!(session.client.structural_calls().is_empty(), "nothing was sent");

    session.press(KeyCode::Char('S'), KeyModifiers::SHIFT);
    session.type_keys("Review");
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.press(KeyCode::Char('X'), KeyModifiers::SHIFT);

    assert_eq!(
        session.client.structural_calls(),
        vec![
            StructuralCall::CreateSection {
                project_gid: "project-1".to_string(),
                name: "Review".to_string(),
                insert_after: Some("sec-open".to_string()),
            },
            StructuralCall::DeleteSection("section-for-Review".to_string()),
        ]
    );
}

#[test]
fn x_marks_the_selection_and_enter_deletes_it() {
    let mut session = edit_session();
    move_to_task(&mut session, "t1");

    // `space` selects and advances, so two presses take `t1` and `t1a`.
    session.press(KeyCode::Char(' '), KeyModifiers::NONE);
    session.press(KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(session.app.tasks.marked_for_deletion_count(), 1);

    session.press(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        session.client.structural_calls(),
        vec![StructuralCall::DeleteTask("t1".to_string())]
    );
    assert_eq!(
        session
            .app
            .tasks
            .table()
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.gid.clone())
            .collect::<Vec<_>>(),
        vec!["t2".to_string()],
        "the parent took its subtask with it"
    );
}

/// A failed parent takes its whole limb back, however deep.
#[test]
fn a_failed_delete_restores_the_whole_limb_it_took() {
    let mut session = Session::start_with(sectioned_client().with_structural_failure("t1"));
    session.press(KeyCode::Char('t'), KeyModifiers::NONE);
    move_to_task(&mut session, "t1");

    session.press(KeyCode::Char('x'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        visible_gids(&session),
        vec!["t1".to_string(), "t1a".to_string(), "t2".to_string()],
        "the subtask comes back with its parent"
    );
}

/// A mark survives the row leaving the table, and `enter` still deletes it.
#[test]
fn a_mark_outlives_the_filter_that_hides_its_row() {
    let mut session = edit_session();
    move_to_task(&mut session, "t2");
    session.press(KeyCode::Char('x'), KeyModifiers::NONE);

    // A filter that hides `t2` but keeps the pane's own rows out of it.
    session.press(KeyCode::Char('f'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.type_keys("release");
    assert_eq!(
        visible_gids(&session),
        vec!["t1".to_string()],
        "the marked row is gone from the table"
    );

    assert_eq!(
        session.app.tasks.marked_for_deletion(),
        vec!["t2".to_string()],
        "a mark the filter hid is still a mark"
    );
}

#[test]
fn esc_clears_the_marks_before_it_leaves_the_mode() {
    let mut session = edit_session();

    session.press(KeyCode::Char('x'), KeyModifiers::NONE);
    session.press(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(session.app.tasks.marked_for_deletion_count(), 0);
    assert_eq!(
        session.app.mode(),
        tuisana::config::Mode::Edit,
        "one key, one sentence: back out of whatever is pending"
    );

    session.press(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(session.app.mode(), tuisana::config::Mode::Task);
}

#[test]
fn a_failed_delete_puts_the_row_back_and_says_so() {
    let mut session = Session::start_with(sectioned_client().with_structural_failure("t2"));
    session.press(KeyCode::Char('t'), KeyModifiers::NONE);
    move_to_task(&mut session, "t2");

    session.press(KeyCode::Char('x'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert!(
        session
            .app
            .tasks
            .table()
            .rows
            .iter()
            .any(|row| row.gid == "t2"),
        "the row comes back"
    );
    assert!(
        session
            .app
            .tasks
            .edit_notice()
            .is_some_and(|notice| notice.contains("could not delete 1 of 1")),
        "{:?}",
        session.app.tasks.edit_notice()
    );
}

#[test]
fn capital_j_and_k_move_the_task_and_are_refused_on_a_subtask() {
    let mut session = edit_session();
    move_to_task(&mut session, "t1a");

    session.press(KeyCode::Char('J'), KeyModifiers::SHIFT);
    assert!(
        session
            .app
            .tasks
            .edit_notice()
            .is_some_and(|notice| notice.contains("belongs to its parent")),
        "{:?}",
        session.app.tasks.edit_notice()
    );
    assert!(session.client.structural_calls().is_empty());

    move_to_task(&mut session, "t2");
    session.press(KeyCode::Char('K'), KeyModifiers::SHIFT);
    assert_eq!(
        session.client.structural_calls(),
        vec![StructuralCall::AddTaskToSection {
            section_gid: "sec-open".to_string(),
            task_gid: "t2".to_string(),
        }]
    );
}

#[test]
fn the_parent_column_is_edited_like_any_other_cell() {
    let mut session = Session::start_with(sectioned_client());
    move_to_task(&mut session, "t2");
    for _ in 0..tuisana::domain::PARENT_COLUMN {
        session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    }

    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.type_keys("Ship the release");
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        session.client.structural_calls(),
        vec![StructuralCall::SetParent {
            gid: "t2".to_string(),
            parent_gid: Some("t1".to_string()),
        }]
    );
    assert_eq!(
        session.cell("t2", tuisana::domain::PARENT_COLUMN),
        "Ship the release"
    );
}

#[test]
fn ctrl_n_walks_the_candidates_and_enter_takes_the_highlighted_one() {
    let mut session = Session::start();

    session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::CONTROL);
    // `me` leads the list, then the workspace directory.
    session.press(KeyCode::Char('n'), KeyModifiers::CONTROL);
    session.press(KeyCode::Char('n'), KeyModifiers::CONTROL);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(
        session.updates(),
        vec![(
            "t1".to_string(),
            TaskFieldEdit::Assignee(Some(tuisana::domain::AssigneeRef::new("user-alex", "alex")))
        )],
        "the second candidate, taken without a single character typed"
    );
}

#[test]
fn a_subsequence_completes_where_a_substring_would_not() {
    let mut session = Session::start();

    session.press(KeyCode::Char('l'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.press(KeyCode::Char('l'), KeyModifiers::CONTROL);
    session.type_keys("prrmn");
    session.press(KeyCode::Tab, KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);

    assert_eq!(session.cell("t1", ASSIGNEE_COLUMN), "Priya Raman");
}

#[test]
fn a_failed_section_move_puts_the_row_back_and_says_so() {
    let mut session =
        Session::start_with(sectioned_client().with_structural_failure("sec-done"));
    session.press(KeyCode::Char('t'), KeyModifiers::NONE);
    move_to_task(&mut session, "t1");

    session.press(KeyCode::Char('J'), KeyModifiers::SHIFT);

    let section = session
        .app
        .tasks
        .table()
        .rows
        .iter()
        .find(|row| row.gid == "t1")
        .and_then(|row| row.section.clone());
    assert_eq!(section.as_deref(), Some("Open"), "the row goes back");
    assert!(
        session
            .app
            .tasks
            .edit_notice()
            .is_some_and(|notice| notice.contains("could not move to Done")),
        "{:?}",
        session.app.tasks.edit_notice()
    );
}

#[test]
fn ctrl_n_still_negates_a_filter_row_when_no_overlay_is_open() {
    let mut session = Session::start();

    // `f` opens the filter panel, `enter` edits the `Title` row — free text,
    // so there is no candidate overlay for `ctrl-n` to walk.
    session.press(KeyCode::Char('f'), KeyModifiers::NONE);
    session.press(KeyCode::Enter, KeyModifiers::NONE);
    session.type_keys("ship");
    assert_eq!(visible_gids(&session), vec!["t1".to_string()]);
    assert!(!session.app.tasks.completion_overlay_open());

    session.press(KeyCode::Char('n'), KeyModifiers::CONTROL);
    assert_eq!(
        visible_gids(&session),
        vec!["t2".to_string(), "t3".to_string()],
        "the complement, which is what ctrl-n has always meant here"
    );
}
