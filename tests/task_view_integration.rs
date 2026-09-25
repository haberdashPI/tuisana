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
                    resource_subtype: None,
                    enum_options: Vec::new(),
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
    assert!(!app.tasks.calendar_open());
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

    assert!(app.tasks.calendar_open());
    assert!(
        app.tasks.calendar_is_range(),
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

    assert!(!app.tasks.calendar_open(), "esc closes the picker");
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

/// Hiding the grid hands its letters back to the text, which is the only way a
/// day name can be typed: `t`, `h`, `j`, `k`, and `d` all steer the grid, and
/// between them they cover every weekday name there is.
#[test]
fn hiding_the_calendar_grid_lets_a_day_name_be_typed() {
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
    let mut open_picker = vec![
        key(' '),
        key('t'),
        key('f'),
        key('j'),
        key('j'),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    ];
    // `;` puts the grid away, and then every letter is just a letter. `d`
    // would otherwise have cleared the field and closed the picker outright.
    open_picker.push(key(';'));
    open_picker.extend("wednesday".chars().map(key));
    open_picker.push(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let mut source = ScriptedSource {
        keys: open_picker,
        wait_before_next: false,
    };
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_session(&mut app, &mut source, &mut terminal).expect("session runs");
    wait_for_task_data(&mut app);

    fn due(app: &App<FakeAsanaClient>) -> String {
        app.tasks
            .filter_panel_rows()
            .into_iter()
            .find(|(label, _)| label == "Due")
            .map(|(_, query)| query)
            .expect("the Due row exists")
    }
    assert_eq!(due(&app), "wednesday", "every letter landed in the text");
    assert!(!app.tasks.calendar_open(), "enter committed and closed it");
    assert_eq!(
        app.tasks.table().task_count(),
        1,
        "the pinned today is a Wednesday, which is the task's due date"
    );
    assert!(
        !app.tasks.calendar_grid_visible(),
        "and the picker is remembered as collapsed for next time"
    );

    // Showing the grid again puts the letters back to work: `t` is `today`
    // rather than the first letter of anything.
    let mut source = ScriptedSource {
        keys: vec![
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            key(';'),
            key('t'),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ],
        wait_before_next: false,
    };
    run_session(&mut app, &mut source, &mut terminal).expect("session runs");
    wait_for_task_data(&mut app);

    assert!(app.tasks.calendar_grid_visible());
    assert_eq!(due(&app), "2026-06-10", "`t` jumped the highlight to today");
}

/// The mirror of the test above: with the grid up, the only characters that
/// reach the text are the ones a written date is made of.
///
/// A letter there is always half a keyword — its other letters steer the grid —
/// so letting it land would leave the field holding text the user never got to
/// finish, over a date the same keystrokes had already moved.
#[test]
fn the_calendar_grid_only_takes_the_characters_of_a_written_date() {
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
    let mut keys = vec![
        key(' '),
        key('t'),
        key('f'),
        key('j'),
        key('j'),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    ];
    // `a`, `b`, `q`, and `z` are all unbound in calendar mode, so nothing else
    // claims them — before this rule they typed. Interleaved with the range so
    // a dropped character would show up as a gap rather than a shorter tail.
    keys.extend("2026a-06b-01..q2026-06-30z".chars().map(key));

    let mut source = ScriptedSource {
        keys,
        wait_before_next: false,
    };
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_session(&mut app, &mut source, &mut terminal).expect("session runs");
    wait_for_task_data(&mut app);

    let due = app
        .tasks
        .filter_panel_rows()
        .into_iter()
        .find(|(label, _)| label == "Due")
        .map(|(_, query)| query)
        .expect("the Due row exists");
    assert_eq!(
        due, "2026-06-01..2026-06-30",
        "the digits, dashes, and dots landed and the letters did not"
    );
    assert!(app.tasks.calendar_grid_visible(), "the grid was up throughout");
}

/// Four tasks spread across the year, for the filter-set tests: one due within
/// a week of the pinned today, one four months out, one in between, and one
/// with no due date at all.
///
/// The in-between task is what proves the sets are ORed rather than merged into
/// one widened range; the undated one is what catches a due-date window pushed
/// down when it should not have been.
///
/// The dates are placed around the same `2026-06-10` every other test in this
/// file pins, because `TUISANA_TODAY` is process-global and the test binary
/// runs its tests in parallel — a second value here would make the calendar
/// tests flake.
fn spread_task(gid: &str, name: &str, due: Option<&str>) -> TaskDto {
    let mut task = make_task();
    task.gid = gid.to_string();
    task.name = name.to_string();
    task.due_on = due.map(ToString::to_string);
    task.start_on = None;
    task.custom_fields = Vec::new();
    task
}

fn spread_client() -> FakeAsanaClient {
    FakeAsanaClient::new(vec![Project::new("project-1", "Inbox", true)])
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
                spread_task("t-soon", "Imminent", Some("2026-06-12")),
                spread_task("t-mid", "In between", Some("2026-08-05")),
                spread_task("t-far", "Far out", Some("2026-10-20")),
                spread_task("t-none", "Someday", None),
            ],
        )
}

fn visible_task_names<C: AsanaClient + Clone + Send + 'static>(app: &App<C>) -> Vec<String> {
    app.tasks
        .table()
        .rows
        .iter()
        .filter(|row| row.kind.is_task())
        .map(|row| row.cells[0].trim().to_string())
        .collect()
}

/// Runs one batch of keys and waits for whatever fetch it triggered.
fn press<C: AsanaClient + Clone + Send + 'static>(
    app: &mut App<C>,
    terminal: &mut Terminal<TestBackend>,
    keys: Vec<KeyEvent>,
) {
    let mut source = ScriptedSource {
        keys,
        wait_before_next: false,
    };
    run_session(app, &mut source, terminal).expect("session runs");
    wait_for_task_data(app);
}

fn spread_app() -> (App<FakeAsanaClient>, Terminal<TestBackend>) {
    std::env::set_var("TUISANA_TODAY", "2026-06-10");
    let mut app = App::new(Config::default(), spread_client());
    app.load_projects().expect("projects load");
    let terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
    (app, terminal)
}

fn chr(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn ret() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

/// Select the project, open the task view and the filter panel, and put a
/// one-week window in the first set's `Due` row.
///
/// Leaves the field cursor on `Due`. That cursor is shared by every set, so an
/// added set is already standing on the same row — pressing `j j` again after
/// `a` would walk off it onto `State`.
fn open_panel_with_a_due_window(app: &mut App<FakeAsanaClient>, terminal: &mut Terminal<TestBackend>) {
    press(app, terminal, vec![chr(' '), chr('t'), chr('f')]);
    press(app, terminal, vec![chr('j'), chr('j'), ret()]);
    press(
        app,
        terminal,
        "2026-06-10..2026-06-17".chars().map(chr).chain([ret()]).collect(),
    );
}

#[test]
fn two_filter_sets_show_the_union_of_their_results() {
    let (mut app, mut terminal) = spread_app();

    open_panel_with_a_due_window(&mut app, &mut terminal);
    assert_eq!(visible_task_names(&app), vec!["Imminent"]);

    // `a` adds a second set; its Due row asks for October instead.
    press(&mut app, &mut terminal, vec![chr('a'), ret()]);
    press(
        &mut app,
        &mut terminal,
        "2026-10-01..2026-10-31".chars().map(chr).chain([ret()]).collect(),
    );

    assert_eq!(
        visible_task_names(&app),
        vec!["Imminent", "Far out"],
        "the August task falls in neither set"
    );
    assert_eq!(app.tasks.filter_set_position(), (1, 2));
}

#[test]
fn a_set_requiring_no_due_date_still_loads_the_undated_tasks() {
    // The server-side date filter, seen from outside: with the union computed
    // wrongly, the `due_on.after` sent to Asana drops the undated tasks and the
    // cache records the window as covered, so they never arrive. The fake
    // honours due_after/due_before, so it drops them exactly as Asana would.
    let (mut app, mut terminal) = spread_app();

    open_panel_with_a_due_window(&mut app, &mut terminal);
    assert!(!visible_task_names(&app).contains(&"Someday".to_string()));

    // A second set asking for tasks with no due date at all: `a`, then `e` on
    // the Due row the cursor is already standing on.
    press(&mut app, &mut terminal, vec![chr('a'), chr('e')]);

    let names = visible_task_names(&app);
    assert!(
        names.contains(&"Someday".to_string()),
        "the undated task has to be fetched as well as shown: {names:?}"
    );
    assert!(names.contains(&"Imminent".to_string()));
    assert!(
        !names.contains(&"In between".to_string()),
        "and the first set still excludes what it excluded: {names:?}"
    );
}

#[test]
fn removing_a_set_puts_its_rows_back_behind_the_remaining_filter() {
    let (mut app, mut terminal) = spread_app();

    open_panel_with_a_due_window(&mut app, &mut terminal);
    press(&mut app, &mut terminal, vec![chr('a'), ret()]);
    press(
        &mut app,
        &mut terminal,
        "2026-10-01..2026-10-31".chars().map(chr).chain([ret()]).collect(),
    );
    assert_eq!(visible_task_names(&app), vec!["Imminent", "Far out"]);

    press(&mut app, &mut terminal, vec![chr('x')]);

    assert_eq!(app.tasks.filter_set_position(), (0, 1));
    assert_eq!(visible_task_names(&app), vec!["Imminent"]);
}

/// A [`spread_app`] whose config is backed by a real file, so
/// `save_to_source_path` has somewhere to write.
fn named_sets_app() -> (App<FakeAsanaClient>, Terminal<TestBackend>, std::path::PathBuf) {
    std::env::set_var("TUISANA_TODAY", "2026-06-10");
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("tuisana-named-sets-{unique}.toml"));
    std::fs::write(&path, "[header]\ntype = \"tuisana\"\nversion = 1.0\n")
        .expect("write config");

    let mut app = App::new(
        Config::load_from_path(&path).expect("config loads"),
        spread_client(),
    );
    app.load_projects().expect("projects load");
    let terminal = Terminal::new(TestBackend::new(140, 30)).expect("terminal");
    (app, terminal, path)
}

fn saved_sets(path: &std::path::Path) -> Vec<tuisana::config::NamedFilterSet> {
    Config::from_toml_str(&std::fs::read_to_string(path).expect("config exists"))
        .expect("it reparses")
        .filter_sets
}

#[test]
fn a_filter_set_can_be_named_kept_current_copied_to_new_and_loaded_back() {
    let (mut app, mut terminal, path) = named_sets_app();

    // Fill in Assignee, then save the panel under a name.
    press(&mut app, &mut terminal, vec![chr(' '), chr('t'), chr('f')]);
    press(&mut app, &mut terminal, vec![chr('j'), ret()]);
    press(
        &mut app,
        &mut terminal,
        "alex".chars().map(chr).chain([ret()]).collect(),
    );
    press(&mut app, &mut terminal, vec![chr('w')]);
    press(
        &mut app,
        &mut terminal,
        "mine".chars().map(chr).chain([ret()]).collect(),
    );

    assert_eq!(app.tasks.filter_set_loaded_name(), Some("mine"));
    let entry = saved_sets(&path);
    assert_eq!(entry.len(), 1);
    assert_eq!(entry[0].name, "mine");
    assert_eq!(entry[0].sets[0].fields.len(), 1);

    // A loaded entry stays current without a save step: `j` to Due, `e` to
    // require it empty, and the file has both without another `w`.
    press(&mut app, &mut terminal, vec![chr('j'), chr('e')]);

    let fields = &saved_sets(&path)[0].sets[0].fields;
    assert_eq!(fields.len(), 2, "both filters are on disk: {fields:?}");
    assert!(fields.iter().any(|field| field.key == "assignee"
        && field.query == "alex"));
    assert!(fields.iter().any(|field| field.key == "due" && field.empty));

    // `y` copies the panel to a new unnamed one: it keeps what it is
    // showing, the entry keeps what was last written to it, and a later edit
    // reaches neither.
    press(&mut app, &mut terminal, vec![chr('y')]);
    assert_eq!(app.tasks.filter_set_loaded_name(), None);
    let before = saved_sets(&path);
    press(&mut app, &mut terminal, vec![chr('j'), ret()]);
    press(
        &mut app,
        &mut terminal,
        "2026-10-01".chars().map(chr).chain([ret()]).collect(),
    );
    assert_eq!(saved_sets(&path), before, "an unnamed panel writes nothing");

    // `1` would replace what the unnamed copy wandered off to, and that copy
    // exists nowhere else — so it asks first.
    press(&mut app, &mut terminal, vec![chr('1')]);
    assert_eq!(app.mode(), tuisana::config::Mode::FilterSetName);
    assert_eq!(
        app.tasks.filter_panel_rows()[3].1,
        "2026-10-01",
        "nothing has been loaded over it yet"
    );
    press(&mut app, &mut terminal, vec![chr('y')]);

    assert_eq!(app.tasks.filter_set_loaded_name(), Some("mine"));
    let rows = app.tasks.filter_panel_rows();
    assert_eq!(rows[1], ("Assignee".to_string(), "alex".to_string()));
    assert_eq!(rows[2].0, "Due");
    assert_eq!(rows[2].1, "(none)", "the require-empty came back");
    assert_eq!(rows[3].1, "", "and the Start the unnamed copy picked did not");

    // `n` throws the panel away and starts from nothing, leaving the entry
    // it was bound to exactly as it was on disk.
    let before = saved_sets(&path);
    press(&mut app, &mut terminal, vec![chr('n')]);

    assert_eq!(app.tasks.filter_set_loaded_name(), None);
    assert!(app
        .tasks
        .filter_panel_rows()
        .iter()
        .all(|(_, query)| query.is_empty()));
    assert_eq!(saved_sets(&path), before);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_sidebar_lists_the_saved_entries_and_never_takes_the_field_cursor() {
    let (mut app, mut terminal, path) = named_sets_app();

    press(&mut app, &mut terminal, vec![chr(' '), chr('t'), chr('f')]);
    assert!(!app.tasks.filter_sets_sidebar_visible(), "closed until asked for");
    press(&mut app, &mut terminal, vec![chr('b')]);
    press(&mut app, &mut terminal, vec![chr('w')]);
    press(
        &mut app,
        &mut terminal,
        "sprint".chars().map(chr).chain([ret()]).collect(),
    );

    assert!(app.tasks.filter_sets_sidebar_visible());
    let text = screen(&mut terminal);
    assert!(text.contains("Sets"), "{text}");
    assert!(text.contains("sprint"), "{text}");

    // The sidebar is never focused, so the filter rows keep `j`/`k`.
    assert_eq!(app.tasks.filter_panel_rows()[0].0, "Title");
    press(&mut app, &mut terminal, vec![chr('j'), chr('j'), chr('e')]);
    assert_eq!(app.tasks.filter_panel_rows()[2].1, "(none)");

    // And `b` puts it away again.
    press(&mut app, &mut terminal, vec![chr('b')]);
    assert!(!app.tasks.filter_sets_sidebar_visible());
    assert!(!screen(&mut terminal).contains("Sets"));

    let _ = std::fs::remove_file(&path);
}

fn screen(terminal: &mut Terminal<TestBackend>) -> String {
    let buffer = terminal.backend_mut().buffer().clone();
    buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
