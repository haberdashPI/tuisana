//! End-to-end key handling for the Gantt chart.
//!
//! These drive the real session loop rather than calling `handle_action`, so
//! they exercise the parts that unit tests cannot: that the bindings resolve,
//! that mode-specific keys shadow the globals in the right modes, and that the
//! whole sequence a user would actually type lands where they expect.
//!
//! Task loading is asynchronous, so keys are sent in batches with a drain
//! between them; sending the whole script at once would make the result depend
//! on whether the worker thread finished first.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use std::{thread, time::Duration};
use tuisana::{
    app::App,
    asana::{
        dto::{TaskDto, TaskMembershipDto, TaskMembershipProjectDto, UserDto},
        fake::FakeAsanaClient,
    },
    config::{Config, Mode},
    domain::{GanttColorKey, Project, TimelineView},
    ui::runtime::{run_session, InputEvent, KeySource},
};

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

fn key(code: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE)
}

fn ctrl(code: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL)
}

fn task(gid: &str, assignee: &str, start: &str, due: &str) -> TaskDto {
    TaskDto {
        gid: gid.to_string(),
        name: format!("Task {gid}"),
        completed: false,
        modified_at: None,
        due_on: Some(due.to_string()),
        start_on: Some(start.to_string()),
        assignee: Some(UserDto {
            gid: format!("u-{assignee}"),
            name: Some(assignee.to_string()),
            display_name: Some(assignee.to_string()),
        }),
        num_subtasks: 0,
        memberships: vec![TaskMembershipDto {
            project: TaskMembershipProjectDto {
                gid: "1".to_string(),
                name: "Inbox".to_string(),
            },
            section: None,
        }],
        custom_fields: Vec::new(),
    }
}

fn app() -> App<FakeAsanaClient> {
    let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]).with_tasks(
        "1",
        vec![
            task("t1", "Ada", "2026-06-01", "2026-07-01"),
            task("t2", "Grace", "2026-07-01", "2026-08-01"),
            task("t3", "Alan", "2026-08-01", "2026-09-01"),
        ],
    );
    let mut app = App::new(Config::default(), client);
    app.load_projects().expect("projects load");
    app
}

/// Runs one batch of keys, then waits for any task load it triggered.
fn run(app: &mut App<FakeAsanaClient>, terminal: &mut Terminal<TestBackend>, keys: Vec<KeyEvent>) {
    run_session(app, &mut ScriptedSource { keys }, terminal).expect("session runs");
    for _ in 0..100 {
        app.poll_task_data();
        if !matches!(app.tasks.status(), tuisana::app::task::TaskStatus::Loading) {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(120, 40)).expect("terminal")
}

#[test]
fn a_whole_gantt_session_lands_where_the_keys_say() {
    let mut app = app();
    let mut terminal = terminal();

    run(&mut app, &mut terminal, vec![key(' '), key('t')]);
    run(&mut app, &mut terminal, vec![key('g')]);
    assert_eq!(app.mode(), Mode::Gantt);
    assert!(app.tasks.gantt().visible());

    // Zoom, scroll, then centre on today: each leaves the window moved.
    run(&mut app, &mut terminal, vec![key('='), key('l'), key('t')]);
    assert!(app.tasks.gantt().timeline_windowed());

    run(&mut app, &mut terminal, vec![key('z')]);
    assert_eq!(app.tasks.gantt().timeline(), &TimelineView::Fit);

    // Two more columns, then one back.
    let total = app.tasks.table().columns.len();
    let columns = app.tasks.gantt().columns(total);
    run(&mut app, &mut terminal, vec![key('>'), key('>'), key('<')]);
    assert_eq!(app.tasks.gantt().columns(total), columns + 1);

    run(&mut app, &mut terminal, vec![key('c')]);
    assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::Section);
    run(&mut app, &mut terminal, vec![key('c'), key('c')]);
    assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::Assignee);

    run(
        &mut app,
        &mut terminal,
        vec![KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
    );
    assert_eq!(app.mode(), Mode::GanttOrder);

    // Ada, Alan, Grace alphabetically; send Grace to the top.
    run(&mut app, &mut terminal, vec![key('j'), key('j'), ctrl('k'), ctrl('k')]);
    let dialog = app.tasks.gantt().dialog().expect("the dialog is open");
    assert_eq!(dialog.entries()[0].value, "Grace");
    assert_eq!(dialog.selected(), 0, "the cursor followed the value");

    run(
        &mut app,
        &mut terminal,
        vec![KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
    );
    assert_eq!(app.mode(), Mode::Gantt);
    assert_eq!(
        app.tasks.gantt().order().first().map(String::as_str),
        Some("Grace"),
    );

    run(
        &mut app,
        &mut terminal,
        vec![KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
    );
    assert_eq!(app.mode(), Mode::Task);
    assert!(app.tasks.gantt().visible(), "esc does not hide the chart");

    run(&mut app, &mut terminal, vec![key('g'), key('g')]);
    assert!(!app.tasks.gantt().visible());
}

#[test]
fn cancelling_the_dialog_restores_the_order_it_opened_with() {
    let mut app = app();
    let mut terminal = terminal();

    run(&mut app, &mut terminal, vec![key(' '), key('t')]);
    run(&mut app, &mut terminal, vec![key('g')]);
    run(
        &mut app,
        &mut terminal,
        vec![KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
    );
    run(&mut app, &mut terminal, vec![key('b')]);
    assert_eq!(
        app.tasks.gantt().order().first().map(String::as_str),
        Some("Alan"),
        "Ada was sent to the bottom, so Alan leads"
    );

    run(
        &mut app,
        &mut terminal,
        vec![KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
    );
    assert_eq!(app.mode(), Mode::Gantt);
    assert!(
        app.tasks.gantt().order().is_empty(),
        "nothing was configured before, so nothing is left behind"
    );
}

#[test]
fn the_dialog_keys_do_not_leak_into_task_mode() {
    // `b` and ctrl-k mean nothing in task mode; if the dialog's bindings were
    // global they would silently reorder colours from the task list.
    let mut app = app();
    let mut terminal = terminal();

    run(&mut app, &mut terminal, vec![key(' '), key('t')]);
    let row = app.tasks.selected_index();
    run(&mut app, &mut terminal, vec![key('b'), ctrl('k')]);

    assert_eq!(app.mode(), Mode::Task);
    assert_eq!(app.tasks.selected_index(), row);
    assert!(app.tasks.gantt().order().is_empty());
}
