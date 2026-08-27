//! Full-screen snapshots of every view at three terminal widths.
//!
//! These are the visual regression guard for the UI. They render the real
//! runtime into a `TestBackend` and compare the resulting screen against a
//! committed text file, so any change to layout, chrome, alignment, or wording
//! shows up as a readable diff rather than as a surprise on someone's terminal.
//!
//! Run `UPDATE_SNAPSHOTS=1 cargo test --test ui_snapshot` to rewrite them, then
//! read the diff before committing it.
//!
//! Determinism: `TUISANA_TODAY` pins the date the relative due dates are
//! measured against, and each fixture is loaded to completion before drawing so
//! the loading spinner never appears.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use std::{fs, path::PathBuf, thread, time::Duration};
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
    config::{Config, ProjectVisibilityConfig, ThemeConfig, ThemeGlyphs, ThemeVariant},
    domain::Project,
    ui::runtime::{run_session, InputEvent, KeySource},
};

const WIDTHS: [u16; 3] = [80, 120, 200];
const HEIGHT: u16 = 30;
const TODAY: &str = "2026-08-24";

/// Feeds a fixed script, then reports the source as closed so the session draws
/// once more and returns.
struct ScriptedSource {
    keys: Vec<KeyEvent>,
}

impl KeySource for ScriptedSource {
    fn next_event(&mut self, _timeout: Duration) -> std::io::Result<InputEvent> {
        if self.keys.is_empty() {
            return Ok(InputEvent::Closed);
        }
        Ok(InputEvent::Key(self.keys.remove(0)))
    }
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

fn task(
    gid: &str,
    name: &str,
    section: &str,
    assignee: Option<&str>,
    due: Option<&str>,
    priority: Option<&str>,
    done: bool,
) -> TaskDto {
    TaskDto {
        gid: gid.to_string(),
        name: name.to_string(),
        completed: done,
        modified_at: Some("2026-08-01T00:00:00Z".to_string()),
        due_on: due.map(str::to_string),
        start_on: Some("2026-08-04".to_string()),
        assignee: assignee.map(|name| UserDto {
            gid: format!("user-{name}"),
            name: Some(name.to_string()),
            display_name: Some(name.to_string()),
        }),
        num_subtasks: 0,
        memberships: vec![TaskMembershipDto {
            project: TaskMembershipProjectDto {
                gid: "project-1".to_string(),
                name: "Northwind BTX 4412".to_string(),
            },
            section: Some(TaskMembershipSectionDto {
                gid: section.to_string(),
                name: section.to_string(),
            }),
        }],
        custom_fields: priority
            .map(|value| {
                vec![CustomFieldValueDto {
                    gid: "custom-1".to_string(),
                    name: "Priority".to_string(),
                    display_value: Some(value.to_string()),
                    enum_value: None,
                }]
            })
            .unwrap_or_default(),
    }
}

fn client() -> FakeAsanaClient {
    FakeAsanaClient::new(vec![
        Project::new("project-1", "Northwind BTX 4412", true),
        Project::new("project-2", "Platform Infrastructure", true),
        Project::new("project-3", "Backlog", false),
    ])
    .with_sections(
        "project-1",
        vec![
            SectionDto {
                gid: "Study Kit Design".to_string(),
                name: "Study Kit Design".to_string(),
            },
            SectionDto {
                gid: "Shipment".to_string(),
                name: "Shipment".to_string(),
            },
        ],
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
    .with_tasks(
        "project-1",
        vec![
            // Overdue, due today, due soon, far out, and unset, so every
            // urgency level and the empty placeholder appear in one screen.
            task(
                "t1",
                "Review shipment requirements doc",
                "Study Kit Design",
                Some("Morgan Ellis"),
                Some("2026-08-19"),
                Some("High"),
                false,
            ),
            task(
                "t2",
                "Ship release candidate to the study team",
                "Study Kit Design",
                Some("Alex Chen"),
                Some(TODAY),
                Some("High"),
                false,
            ),
            task(
                "t3",
                "Confirm carrier pickup window with the vendor",
                "Study Kit Design",
                None,
                Some("2026-08-26"),
                None,
                false,
            ),
            task(
                "t4",
                "Close out packaging vendor contract",
                "Shipment",
                Some("Alex Chen"),
                Some("2026-11-14"),
                Some("Low"),
                false,
            ),
            task(
                "t5",
                "Archive the pilot batch records",
                "Shipment",
                Some("Jo Park"),
                None,
                Some("Low"),
                true,
            ),
        ],
    )
}

/// The monochrome, ASCII-only fallback: no color codes and no glyphs beyond
/// plain ASCII, for terminals that cannot render either.
fn mono_config() -> Config {
    let mut config = config();
    config.theme = ThemeConfig {
        variant: ThemeVariant::Mono,
        glyphs: ThemeGlyphs::Ascii,
        accent: "cyan".to_string(),
        zebra: false,
    };
    config
}

fn config() -> Config {
    // Without visibility entries every project defaults to hidden, which is a
    // real state but an unrepresentative one to snapshot.
    let mut config = Config::default();
    config.project_visibility = vec![
        ProjectVisibilityConfig {
            gid: "project-1".to_string(),
            starred: true,
            hidden: false,
        },
        ProjectVisibilityConfig {
            gid: "project-2".to_string(),
            starred: true,
            hidden: false,
        },
        ProjectVisibilityConfig {
            gid: "project-3".to_string(),
            starred: false,
            hidden: false,
        },
    ];
    config
}

fn drain_task_data<C: AsanaClient + Clone + Send + 'static>(app: &mut App<C>) {
    for _ in 0..100 {
        app.poll_task_data();
        if !matches!(app.tasks.status(), tuisana::app::task::TaskStatus::Loading) {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Renders one screen from a sequence of key batches.
///
/// Task loading is asynchronous, so each batch is followed by a drain: keys in
/// a later batch always act on a settled table. Sending everything at once
/// makes the result depend on whether the worker thread happened to finish
/// first, which is exactly the flakiness a snapshot test must not have.
fn render_with(config: Config, width: u16, batches: Vec<Vec<KeyEvent>>) -> String {
    let mut app = App::new(config, client());
    app.load_projects().expect("projects load");

    let backend = TestBackend::new(width, HEIGHT);
    let mut terminal = Terminal::new(backend).expect("terminal");

    for keys in batches {
        run_session(&mut app, &mut ScriptedSource { keys }, &mut terminal)
            .expect("session runs");
        drain_task_data(&mut app);
    }

    // A final pass with no input redraws the settled state, so a snapshot never
    // captures a spinner mid-load.
    run_session(&mut app, &mut ScriptedSource { keys: Vec::new() }, &mut terminal)
        .expect("session redraws");

    let buffer = terminal.backend_mut().buffer().clone();
    buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.txt"))
}

fn assert_snapshot(name: &str, batches: fn() -> Vec<Vec<KeyEvent>>) {
    assert_snapshot_with(config(), WIDTHS.to_vec(), name, batches)
}

fn assert_snapshot_with(
    config: Config,
    widths: Vec<u16>,
    name: &str,
    batches: fn() -> Vec<Vec<KeyEvent>>,
) {
    std::env::set_var("TUISANA_TODAY", TODAY);

    for width in widths {
        let rendered = render_with(config.clone(), width, batches());
        let path = snapshot_path(&format!("{name}-{width}"));

        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            fs::create_dir_all(path.parent().expect("snapshot dir")).expect("create snapshot dir");
            fs::write(&path, format!("{rendered}\n")).expect("write snapshot");
            continue;
        }

        let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "missing snapshot {}; run UPDATE_SNAPSHOTS=1 cargo test --test ui_snapshot",
                path.display()
            )
        });

        assert_eq!(
            rendered,
            expected.trim_end_matches('\n'),
            "snapshot {name}-{width} changed; \
             run UPDATE_SNAPSHOTS=1 cargo test --test ui_snapshot to accept"
        );
    }
}

/// Select the first project and open the task view, leaving it fully loaded.
fn enter_task_mode() -> Vec<KeyEvent> {
    vec![key(' '), key('t')]
}

#[test]
fn project_mode() {
    assert_snapshot("project-mode", Vec::new);
}

#[test]
fn project_mode_with_help() {
    assert_snapshot("project-help", || vec![vec![key('?')]]);
}

#[test]
fn project_search() {
    assert_snapshot("project-search", || vec![vec![key('/'), key('l')]]);
}

#[test]
fn task_mode() {
    assert_snapshot("task-mode", || vec![enter_task_mode()]);
}

#[test]
fn task_mode_with_selection() {
    assert_snapshot("task-selection", || {
        vec![enter_task_mode(), vec![key(' '), key(' ')]]
    });
}

#[test]
fn task_mode_with_help() {
    assert_snapshot("task-help", || vec![enter_task_mode(), vec![key('?')]]);
}

#[test]
fn task_mode_showing_completed_tasks() {
    assert_snapshot("task-all-states", || {
        vec![enter_task_mode(), vec![key('c')], vec![key('c')]]
    });
}

#[test]
fn filter_mode() {
    assert_snapshot("filter-mode", || vec![enter_task_mode(), vec![key('f')]]);
}

#[test]
fn filter_edit_mode() {
    assert_snapshot("filter-edit", || {
        vec![
            enter_task_mode(),
            vec![key('f'), enter()],
            vec![key('s'), key('h')],
        ]
    });
}

/// The `Due` filter is the third row, so two `j`s land on it.
fn open_due_calendar() -> Vec<KeyEvent> {
    vec![key('f'), key('j'), key('j'), enter()]
}

#[test]
fn filter_date_calendar() {
    assert_snapshot("filter-date-calendar", || {
        vec![enter_task_mode(), open_due_calendar()]
    });
}

#[test]
fn filter_date_calendar_after_flipping_two_months_forward() {
    assert_snapshot("filter-date-calendar-month", || {
        vec![enter_task_mode(), open_due_calendar(), vec![key('j'), key('j')]]
    });
}

#[test]
fn filter_date_calendar_with_unusable_text() {
    // `a`, `b`, and `c` are unbound in calendar mode, so they type rather than
    // navigating. The overlay has to say the text is unusable, because the old
    // behavior was to silently filter the table to nothing.
    assert_snapshot("filter-date-calendar-invalid", || {
        vec![
            enter_task_mode(),
            open_due_calendar(),
            vec![key('a'), key('b'), key('c')],
        ]
    });
}

#[test]
fn filter_date_calendar_with_a_range() {
    // Digits, `-`, and `.` are unbound in calendar mode, so they type into the
    // filter field. The grid shades the span between the two ends.
    assert_snapshot("filter-date-calendar-range", || {
        vec![
            enter_task_mode(),
            open_due_calendar(),
            "2026-08-10..2026-08-20".chars().map(key).collect(),
        ]
    });
}

#[test]
fn filter_date_calendar_editing_the_start_of_a_range() {
    // `ctrl-a` moves the caret to the start end, so the navigation keys rewrite
    // that end and the overlay says which one it is editing.
    assert_snapshot("filter-date-calendar-range-start", || {
        vec![
            enter_task_mode(),
            open_due_calendar(),
            "2026-08-10..2026-08-20".chars().map(key).collect(),
            vec![ctrl('a'), key('h')],
        ]
    });
}

#[test]
fn filter_date_calendar_help() {
    // Calendar mode does not fall back to global bindings, so `?` has to be
    // bound there for the picker's own keys to be discoverable.
    assert_snapshot("filter-date-calendar-help", || {
        vec![enter_task_mode(), open_due_calendar(), vec![key('?')]]
    });
}

#[test]
fn task_mode_in_the_monochrome_ascii_theme() {
    assert_snapshot_with(mono_config(), vec![120], "task-mono", || {
        vec![enter_task_mode()]
    });
}

#[test]
fn the_monochrome_theme_emits_no_color() {
    // The buffer is inspected directly here rather than snapshotted, because a
    // text snapshot cannot show whether a style was applied.
    std::env::set_var("TUISANA_TODAY", TODAY);
    let mut app = App::new(mono_config(), client());
    app.load_projects().expect("projects load");

    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    run_session(
        &mut app,
        &mut ScriptedSource {
            keys: enter_task_mode(),
        },
        &mut terminal,
    )
    .expect("session runs");
    drain_task_data(&mut app);
    // The calendar overlay is checked in the same pass: it draws day numbers, a
    // highlight, and a shaded range, all of which are easy to reach for color or
    // a box-drawing glyph without noticing.
    let mut keys = open_due_calendar();
    keys.extend("2026-08-10..2026-08-20".chars().map(key));
    run_session(
        &mut app,
        &mut ScriptedSource { keys },
        &mut terminal,
    )
    .expect("session opens the calendar");
    drain_task_data(&mut app);
    run_session(&mut app, &mut ScriptedSource { keys: Vec::new() }, &mut terminal)
        .expect("session redraws");

    assert!(
        app.tasks.filter_calendar_open(),
        "the calendar is what this pass is checking"
    );

    let buffer = terminal.backend_mut().buffer().clone();
    for cell in buffer.content.iter() {
        assert_eq!(
            cell.fg,
            ratatui::style::Color::Reset,
            "the monochrome theme set a foreground color"
        );
        assert_eq!(
            cell.bg,
            ratatui::style::Color::Reset,
            "the monochrome theme set a background color"
        );
        assert!(
            cell.symbol().is_ascii(),
            "the ascii glyph set emitted {:?}",
            cell.symbol()
        );
    }
}

#[test]
fn the_example_config_parses() {
    let contents = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tuisana.toml.example"),
    )
    .expect("read the example config");

    Config::from_toml_str(&contents).expect("the documented example config is valid");
}

#[test]
fn task_pane_with_the_project_pane_minimized() {
    // `{` minimizes the top pane and switches to task mode, so the task table
    // gets the whole body.
    assert_snapshot("task-minimized-top", || vec![vec![key(' '), key('{')]]);
}
