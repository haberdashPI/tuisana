use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use std::{thread, time::Duration};
use tuisana::{
    app::App,
    asana::{
        dto::{
            CustomFieldDto, CustomFieldValueDto, ProjectCustomFieldSettingDto, SectionDto, TaskDto,
            TaskMembershipDto, TaskMembershipProjectDto, TaskMembershipSectionDto, UserDto,
        },
        fake::FakeAsanaClient,
        AsanaClient,
    },
    config::Config,
    domain::Project,
    ui::runtime::{run_session, InputEvent, KeySource},
};

struct ScriptedSource {
    keys: Vec<KeyEvent>,
    wait_before_next: bool,
}

impl KeySource for ScriptedSource {
    fn next_event(&mut self, _timeout: Duration) -> std::io::Result<InputEvent> {
        if self.keys.is_empty() {
            Ok(InputEvent::Closed)
        } else {
            if self.wait_before_next {
                thread::sleep(Duration::from_millis(200));
            }
            self.wait_before_next = true;
            Ok(InputEvent::Key(self.keys.remove(0)))
        }
    }
}

fn make_task() -> TaskDto {
    TaskDto {
        gid: "task-1".to_string(),
        name: "Ship release".to_string(),
        completed: false,
        modified_at: Some("2026-06-01T00:00:00Z".to_string()),
        due_on: Some("2026-06-10".to_string()),
        start_on: Some("2026-06-01".to_string()),
        assignee: Some(UserDto {
            gid: "user-1".to_string(),
            name: Some("Alex".to_string()),
            display_name: Some("Alex".to_string()),
        }),
        num_subtasks: 0,
        memberships: vec![TaskMembershipDto {
            project: TaskMembershipProjectDto {
                gid: "project-1".to_string(),
                name: "Inbox".to_string(),
            },
            section: Some(TaskMembershipSectionDto {
                gid: "section-1".to_string(),
                name: "Today".to_string(),
            }),
        }],
        parent: None,
        custom_fields: vec![CustomFieldValueDto {
            gid: "custom-1".to_string(),
            name: "Priority".to_string(),
            display_value: Some("High".to_string()),
            enum_value: None,
        }],
    }
}

fn wait_for_task_data<C: AsanaClient + Clone + Send + 'static>(app: &mut App<C>) {
    for _ in 0..50 {
        app.poll_task_data();
        if !matches!(
            app.tasks.status(),
            tuisana::app::task::TaskStatus::Loading
        ) {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    app.poll_task_data();
}

#[test]
fn pressing_m_displays_the_task_view() {
    let client = FakeAsanaClient::new(vec![Project::new("project-1", "Inbox", true)])
        .with_sections(
            "project-1",
            vec![SectionDto {
                gid: "section-1".to_string(),
                name: "Today".to_string(),
            }],
        )
        .with_custom_field_settings(
            "project-1",
            vec![ProjectCustomFieldSettingDto {
                gid: "setting-1".to_string(),
                custom_field: CustomFieldDto {
                    gid: "custom-1".to_string(),
                    name: "Priority".to_string(),
                },
            }],
        )
        .with_tasks("project-1", vec![make_task()]);

    let mut app = App::new(Config::default(), client);
    app.load_projects().expect("projects load");

    let mut source = ScriptedSource {
        keys: vec![
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ],
        wait_before_next: false,
    };
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_session(&mut app, &mut source, &mut terminal).expect("session runs");
    wait_for_task_data(&mut app);

    assert!(app.tasks.visible());
    assert_eq!(app.tasks.table().task_count(), 1);
    assert_eq!(app.tasks.status(), &tuisana::app::task::TaskStatus::Ready);
    assert!(app.tasks.horizontal_scroll() > 0);

    let buffer = terminal.backend_mut().buffer().clone();
    let text = buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");

    // The pane title lives in its border and the active mode in the status bar,
    // instead of each getting a line of its own above the pane.
    assert!(text.contains("Tasks"));
    assert!(text.contains("TASK"));
    assert!(text.contains("TUISANA"));
}

#[test]
fn task_view_scrolls_to_keep_the_selected_row_visible() {
    let tasks = (1..=8)
        .map(|index| TaskDto {
            gid: format!("task-{index}"),
            name: format!("Task {index}"),
            completed: false,
            modified_at: Some("2026-06-01T00:00:00Z".to_string()),
            due_on: Some("2026-06-10".to_string()),
            start_on: Some("2026-06-01".to_string()),
            assignee: Some(UserDto {
                gid: "user-1".to_string(),
                name: Some("Alex".to_string()),
                display_name: Some("Alex".to_string()),
            }),
            num_subtasks: 0,
            memberships: vec![TaskMembershipDto {
                project: TaskMembershipProjectDto {
                    gid: "project-1".to_string(),
                    name: "Inbox".to_string(),
                },
                section: Some(TaskMembershipSectionDto {
                    gid: "section-1".to_string(),
                    name: "Today".to_string(),
                }),
            }],
            parent: None,
            custom_fields: vec![],
        })
        .collect::<Vec<_>>();

    let client = FakeAsanaClient::new(vec![Project::new("project-1", "Inbox", true)])
        .with_sections(
            "project-1",
            vec![SectionDto {
                gid: "section-1".to_string(),
                name: "Today".to_string(),
            }],
        )
        .with_tasks("project-1", tasks);

    let mut app = App::new(Config::default(), client);
    app.load_projects().expect("projects load");

    let mut source = ScriptedSource {
        keys: vec![
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ],
        wait_before_next: false,
    };
    let backend = TestBackend::new(80, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_session(&mut app, &mut source, &mut terminal).expect("session runs");

    assert_eq!(app.tasks.selected_index(), Some(8));
    assert!(app.tasks.vertical_scroll() > 0);
}

#[test]
fn changing_the_completed_filter_updates_the_visible_task_rows() {
    let client = FakeAsanaClient::new(vec![Project::new("project-1", "Inbox", true)])
        .with_sections(
            "project-1",
            vec![SectionDto {
                gid: "section-1".to_string(),
                name: "Today".to_string(),
            }],
        )
        .with_tasks(
            "project-1",
            vec![
                make_task(),
                TaskDto {
                    gid: "task-2".to_string(),
                    name: "Closed task".to_string(),
                    completed: true,
                    modified_at: Some("2026-06-01T00:00:00Z".to_string()),
                    due_on: Some("2026-06-11".to_string()),
                    start_on: Some("2026-06-02".to_string()),
                    assignee: Some(UserDto {
                        gid: "user-1".to_string(),
                        name: Some("Alex".to_string()),
                        display_name: Some("Alex".to_string()),
                    }),
                    num_subtasks: 0,
                    memberships: vec![TaskMembershipDto {
                        project: TaskMembershipProjectDto {
                            gid: "project-1".to_string(),
                            name: "Inbox".to_string(),
                        },
                        section: Some(TaskMembershipSectionDto {
                            gid: "section-1".to_string(),
                            name: "Today".to_string(),
                        }),
                    }],
                    parent: None,
                    custom_fields: vec![CustomFieldValueDto {
                        gid: "custom-1".to_string(),
                        name: "Priority".to_string(),
                        display_value: Some("High".to_string()),
                        enum_value: None,
                    }],
                },
            ],
        );

    let mut app = App::new(Config::default(), client);
    app.load_projects().expect("projects load");

    let mut source = ScriptedSource {
        keys: vec![
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ],
        wait_before_next: false,
    };
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_session(&mut app, &mut source, &mut terminal).expect("session runs");
    wait_for_task_data(&mut app);

    assert_eq!(app.tasks.table().task_count(), 1);
    assert!(app.tasks.filter_summary().contains("comp done"));
    assert!(app.tasks.filter_summary().contains("grp p:"));

    let buffer = terminal.backend_mut().buffer().clone();
    let text = buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(text.contains("Tasks"));
    assert!(text.contains("TASK"));
}

/// Drives the whole date-picking flow through real key events: open the filter
/// panel, land on `Due`, open the calendar, move the highlight, and commit.
#[test]
fn picking_a_date_on_the_calendar_filters_the_task_table() {
    std::env::set_var("TUISANA_TODAY", "2026-06-10");

    let client = FakeAsanaClient::new(vec![Project::new("project-1", "Inbox", true)])
        .with_sections(
            "project-1",
            vec![SectionDto {
                gid: "section-1".to_string(),
                name: "Today".to_string(),
            }],
        )
        .with_tasks("project-1", vec![make_task()]);

    let mut app = App::new(Config::default(), client);
    app.load_projects().expect("projects load");

    let key = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    let mut source = ScriptedSource {
        keys: vec![
            key(' '),
            key('t'),
            key('f'),
            // Title, Assignee, Due.
            key('j'),
            key('j'),
            // Open the calendar on the Due field.
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            // The task is due 2026-06-10, which is the pinned today, so step
            // one day forward to a date that must filter it out.
            key('l'),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ],
        wait_before_next: false,
    };
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_session(&mut app, &mut source, &mut terminal).expect("session runs");
    wait_for_task_data(&mut app);

    assert_eq!(app.mode(), tuisana::config::Mode::Filter, "committing leaves the picker");
    assert!(!app.tasks.filter_calendar_open());
    assert_eq!(
        app.tasks.table().task_count(),
        0,
        "the picked day is not the task's due date"
    );

    // Reopen, jump back to today, and commit: the task returns.
    let mut source = ScriptedSource {
        keys: vec![
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            key('t'),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ],
        wait_before_next: false,
    };
    run_session(&mut app, &mut source, &mut terminal).expect("session runs");
    wait_for_task_data(&mut app);

    assert_eq!(app.tasks.table().task_count(), 1);
}

/// Typed text goes into the filter field, not a buffer inside the overlay, so
/// closing the picker keeps what was picked and the caret keys edit the query in
/// place.
#[test]
fn the_calendar_edits_the_filter_field_directly() {
    std::env::set_var("TUISANA_TODAY", "2026-06-10");

    let client = FakeAsanaClient::new(vec![Project::new("project-1", "Inbox", true)])
        .with_sections(
            "project-1",
            vec![SectionDto {
                gid: "section-1".to_string(),
                name: "Today".to_string(),
            }],
        )
        .with_tasks("project-1", vec![make_task()]);

    let mut app = App::new(Config::default(), client);
    app.load_projects().expect("projects load");

    let key = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    let mut source = ScriptedSource {
        keys: vec![
            key(' '),
            key('t'),
            key('f'),
            key('j'),
            key('j'),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            // Type a range covering the task's due date of 2026-06-10, digit by
            // digit, exactly as a user would.
            key('2'),
            key('0'),
            key('2'),
            key('6'),
            key('-'),
            key('0'),
            key('6'),
            key('-'),
            key('0'),
            key('1'),
            key('.'),
            key('.'),
            key('2'),
            key('0'),
            key('2'),
            key('6'),
            key('-'),
            key('0'),
            key('6'),
            key('-'),
            key('3'),
            key('0'),
        ],
        wait_before_next: false,
    };
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_session(&mut app, &mut source, &mut terminal).expect("session runs");
    wait_for_task_data(&mut app);

    assert!(app.tasks.filter_calendar_open());
    assert!(
        app.tasks.filter_calendar_is_range(),
        "the query is a range, so both ends can be moved between"
    );
    assert_eq!(
        app.tasks.table().task_count(),
        1,
        "the range covers the task, and the table refilters as the text lands"
    );

    // Back the end of the range up to before the task's due date. `ctrl-a` moves
    // to the start end, `ctrl-e` back to the finish, and `h` steps a day.
    let ctrl = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    let mut source = ScriptedSource {
        keys: vec![
            ctrl('a'),
            ctrl('e'),
            // 2026-06-30 back to 2026-06-09, one day at a time.
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            key('h'),
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        ],
        wait_before_next: false,
    };
    run_session(&mut app, &mut source, &mut terminal).expect("session runs");
    wait_for_task_data(&mut app);

    assert!(!app.tasks.filter_calendar_open(), "esc closes the picker");
    assert_eq!(
        app.tasks.table().task_count(),
        0,
        "only the range's end moved, and it no longer reaches the task"
    );
}

/// Flipping months lands on an edge of the month rather than dragging the old day
/// number along, and only the end the caret is in gets rewritten.
#[test]
fn flipping_months_in_the_calendar_lands_on_the_month_edges() {
    std::env::set_var("TUISANA_TODAY", "2026-06-10");

    let client = FakeAsanaClient::new(vec![Project::new("project-1", "Inbox", true)])
        .with_sections(
            "project-1",
            vec![SectionDto {
                gid: "section-1".to_string(),
                name: "Today".to_string(),
            }],
        )
        .with_tasks("project-1", vec![make_task()]);

    let mut app = App::new(Config::default(), client);
    app.load_projects().expect("projects load");

    let key = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    let mut source = ScriptedSource {
        keys: vec![
            key(' '),
            key('t'),
            key('f'),
            key('j'),
            key('j'),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            // Forward a month, then back two.
            key('j'),
            key('k'),
            key('k'),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ],
        wait_before_next: false,
    };
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_session(&mut app, &mut source, &mut terminal).expect("session runs");
    wait_for_task_data(&mut app);

    // Started empty on June 2026: `j` -> Jul 1, `k` -> Jun 30, `k` -> May 31.
    let due = app
        .tasks
        .filter_panel_rows()
        .into_iter()
        .find(|(label, _)| label == "Due")
        .map(|(_, query)| query)
        .expect("the Due row exists");
    assert_eq!(due, "2026-05-31");
}
