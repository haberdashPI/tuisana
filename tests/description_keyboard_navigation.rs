//! End-to-end key handling for the description pane.
//!
//! These drive the real session loop rather than calling `handle_action`, so
//! they exercise what unit tests cannot: that `v` resolves in both modes, that
//! `ctrl-d` means one thing in description mode and another in task mode, and
//! — the point of the whole fetch design — that a burst of `j` costs one
//! request rather than one per row.
//!
//! The source reports its own queue through `pending()`, which is what lets a
//! burst actually be a burst here: with a source that always says `false`,
//! every key would settle on its own and the burst test would prove nothing.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use std::{thread, time::Duration};
use tuisana::{
    app::App,
    asana::{
        dto::{TaskDto, TaskMembershipDto, TaskMembershipProjectDto},
        fake::FakeAsanaClient,
    },
    config::{Config, Mode},
    domain::Project,
    ui::runtime::{run_session, InputEvent, KeySource, RecordingHost},
};

/// A key source that knows what is still queued behind the key it just handed
/// over, so a run of keys arrives as one burst.
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

    fn pending(&mut self) -> bool {
        !self.keys.is_empty()
    }
}

fn key(code: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(code), KeyModifiers::NONE)
}

fn ctrl(code: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL)
}

fn esc() -> KeyEvent {
    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
}

fn task(gid: &str, name: &str) -> TaskDto {
    TaskDto {
        gid: gid.to_string(),
        name: name.to_string(),
        completed: false,
        modified_at: Some("2026-08-01T00:00:00Z".to_string()),
        due_on: Some("2026-09-01".to_string()),
        start_on: None,
        assignee: None,
        num_subtasks: 0,
        memberships: vec![TaskMembershipDto {
            project: TaskMembershipProjectDto {
                gid: "1".to_string(),
                name: "Inbox".to_string(),
            },
            section: None,
        }],
        parent: None,
        custom_fields: Vec::new(),
    }
}

/// A description long enough to need scrolling in a forty-line terminal.
fn long_text() -> String {
    (1..=60)
        .map(|n| format!("line {n} of the description"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn client() -> FakeAsanaClient {
    FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)])
        .with_tasks(
            "1",
            // Named so the default sort puts them in gid order: the tests
            // below talk about rows by gid, and "Fifth task" sorting above
            // "First task" would make every one of them a puzzle.
            vec![
                task("t1", "Task 1"),
                task("t2", "Task 2"),
                task("t3", "Task 3"),
                task("t4", "Task 4"),
                task("t5", "Task 5"),
            ],
        )
        .with_description("t1", long_text())
        .with_description("t3", "Short and sweet.")
        .with_comments(
            "t1",
            vec![("Alex Chen", "2026-08-02T16:40:00.000Z", "Looks right to me.")],
        )
        // Comments and no description: the pane still has something to show.
        .with_comments(
            "t2",
            vec![("Jo Park", "2026-08-03T09:00:00.000Z", "Needs doing.")],
        )
}

/// The app and a clone of the client whose call log it shares.
///
/// The fake's logs are `Arc<Mutex<_>>` precisely so a clone handed to a worker
/// thread records where the test can see it.
fn app_with(client: FakeAsanaClient) -> (App<FakeAsanaClient>, FakeAsanaClient) {
    let mut app = App::new(Config::default(), client.clone());
    app.load_projects().expect("projects load");
    (app, client)
}

/// Runs one batch of keys, then waits for whatever it set going.
fn run(app: &mut App<FakeAsanaClient>, terminal: &mut Terminal<TestBackend>, keys: Vec<KeyEvent>) {
    run_session(
        app,
        &mut ScriptedSource { keys },
        terminal,
        &mut RecordingHost::default(),
    )
    .expect("session runs");

    for _ in 0..100 {
        app.poll_task_data();
        app.poll_description();
        if !matches!(app.tasks.status(), tuisana::app::task::TaskStatus::Loading) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    // The description fetch is its own thread and its own channel, so it needs
    // its own wait: the task data can be settled while the text is still out.
    for _ in 0..100 {
        app.poll_description();
        if !app.tasks.description().is_fetching() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(120, 40)).expect("terminal")
}

/// Selects the project and moves the keys to the table.
fn open_tasks(app: &mut App<FakeAsanaClient>, terminal: &mut Terminal<TestBackend>) {
    run(app, terminal, vec![key(' ')]);
    run(app, terminal, vec![key('t')]);
}

#[test]
fn v_opens_the_pane_takes_the_mode_and_fetches_the_cursor_row() {
    let (mut app, client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);

    run(&mut app, &mut terminal, vec![key('v')]);

    assert_eq!(app.mode(), Mode::Description);
    assert!(app.tasks.description().visible());
    assert_eq!(app.tasks.description().target(), Some("t1"));
    assert_eq!(client.description_calls(), vec!["t1".to_string()]);
}

#[test]
fn the_comments_are_read_with_the_description_not_after_it() {
    let (mut app, client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);

    run(&mut app, &mut terminal, vec![key('v')]);

    // One round of work per task: both reads go out together, so the pane is
    // never half-loaded.
    assert_eq!(client.description_calls(), vec!["t1".to_string()]);
    assert_eq!(client.comment_calls(), vec!["t1".to_string()]);
}

#[test]
fn a_burst_of_j_costs_one_comment_fetch_too() {
    let (mut app, client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);
    run(&mut app, &mut terminal, vec![key('v')]);

    run(
        &mut app,
        &mut terminal,
        vec![key('j'), key('j'), key('j'), key('j')],
    );

    assert_eq!(
        client.comment_calls(),
        vec!["t1".to_string(), "t5".to_string()],
        "the same settled-cursor gate covers both reads"
    );
}

#[test]
fn a_task_with_comments_and_no_description_still_has_something_to_read() {
    let (mut app, _client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);
    run(&mut app, &mut terminal, vec![key('v')]);

    run(&mut app, &mut terminal, vec![key('j')]);

    assert_eq!(app.tasks.description().target(), Some("t2"));
    assert!(
        !app.tasks.description().is_empty(),
        "t2 has no description but one comment"
    );
}

#[test]
fn a_failed_comment_fetch_fails_the_pane_rather_than_half_filling_it() {
    let (mut app, _client) = app_with(client().with_comment_failure("t1"));
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);

    run(&mut app, &mut terminal, vec![key('v')]);

    // The description read succeeded; the comment read did not. A pane that
    // showed the description and silently dropped the comments would be
    // telling the reader this task has none.
    assert!(app.tasks.description().failed());
}

#[test]
fn r_still_refreshes_from_task_mode() {
    let (mut app, _client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);

    // The whole reason the key is `v`: `r` must not have been displaced.
    run(&mut app, &mut terminal, vec![key('r')]);

    assert_eq!(app.mode(), Mode::Task);
    assert!(!app.tasks.description().visible());
}

#[test]
fn a_burst_of_j_costs_one_fetch() {
    let (mut app, client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);
    run(&mut app, &mut terminal, vec![key('v')]);

    // Four rows in one burst. Only the row the cursor lands on is worth
    // asking about — this is the test the settled-queue gate exists for.
    run(
        &mut app,
        &mut terminal,
        vec![key('j'), key('j'), key('j'), key('j')],
    );

    assert_eq!(app.tasks.description().target(), Some("t5"));
    assert_eq!(
        client.description_calls(),
        vec!["t1".to_string(), "t5".to_string()],
        "the row it started on and the row it landed on, and nothing between"
    );
}

#[test]
fn a_revisited_row_is_served_from_the_cache() {
    let (mut app, client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);
    run(&mut app, &mut terminal, vec![key('v')]);
    run(&mut app, &mut terminal, vec![key('j')]);
    let before = client.description_calls();

    run(&mut app, &mut terminal, vec![key('k')]);

    assert_eq!(app.tasks.description().target(), Some("t1"));
    assert_eq!(
        client.description_calls(),
        before,
        "t1 was already fetched"
    );
}

#[test]
fn refresh_clears_the_cache_so_the_next_look_refetches() {
    let (mut app, client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);
    run(&mut app, &mut terminal, vec![key('v')]);
    assert_eq!(client.description_calls().len(), 1);

    run(&mut app, &mut terminal, vec![key('r')]);

    assert_eq!(
        client.description_calls(),
        vec!["t1".to_string(), "t1".to_string()],
        "a description edited in the browser is re-read"
    );
}

#[test]
fn ctrl_d_scrolls_the_text_and_esc_hands_the_key_back_to_the_table() {
    let (mut app, _client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);
    run(&mut app, &mut terminal, vec![key('v')]);
    assert_eq!(app.tasks.description().scroll(), 0);

    run(&mut app, &mut terminal, vec![ctrl('d')]);
    let scrolled = app.tasks.description().scroll();
    assert!(scrolled > 0, "the text moved: {scrolled}");

    run(&mut app, &mut terminal, vec![ctrl('u')]);
    assert_eq!(app.tasks.description().scroll(), 0);

    // `esc` leaves the pane open and returns the keys, so `ctrl-d` is the
    // table's page key again.
    run(&mut app, &mut terminal, vec![esc()]);
    assert_eq!(app.mode(), Mode::Task);
    assert!(app.tasks.description().visible(), "the pane stays");

    run(&mut app, &mut terminal, vec![ctrl('d')]);
    assert_eq!(
        app.tasks.description().scroll(),
        0,
        "the key went to the table, not the prose"
    );
}

#[test]
fn moving_to_another_task_resets_the_scroll() {
    let (mut app, _client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);
    // Two batches, not one: in a single burst the fetch is still out when
    // `ctrl-d` arrives, and there is no text to scroll yet.
    run(&mut app, &mut terminal, vec![key('v')]);
    run(&mut app, &mut terminal, vec![ctrl('d')]);
    assert!(app.tasks.description().scroll() > 0);

    run(&mut app, &mut terminal, vec![key('j')]);

    assert_eq!(app.tasks.description().scroll(), 0);
}

#[test]
fn v_from_inside_the_mode_closes_the_pane_and_goes_back_to_tasks() {
    let (mut app, _client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);
    run(&mut app, &mut terminal, vec![key('v')]);

    run(&mut app, &mut terminal, vec![key('v')]);

    assert_eq!(app.mode(), Mode::Task);
    assert!(!app.tasks.description().visible());
}

#[test]
fn the_pane_and_the_chart_never_share_the_space() {
    let (mut app, _client) = app_with(client());
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);

    run(&mut app, &mut terminal, vec![key('g')]);
    run(&mut app, &mut terminal, vec![key('=')]);
    let windowed = *app.tasks.gantt().timeline();
    run(&mut app, &mut terminal, vec![esc()]);

    run(&mut app, &mut terminal, vec![key('v')]);
    assert!(app.tasks.description().visible());
    assert!(
        app.tasks.gantt().visible(),
        "the chart's own flag is untouched"
    );

    run(&mut app, &mut terminal, vec![key('v')]);
    assert_eq!(
        *app.tasks.gantt().timeline(),
        windowed,
        "and it comes back exactly as it was"
    );
}

#[test]
fn a_failed_fetch_is_reported_and_the_session_carries_on() {
    let (mut app, client) = app_with(client().with_description_failure("t1"));
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);

    run(&mut app, &mut terminal, vec![key('v')]);
    assert!(app.tasks.description().failed(), "the pane says so");

    // Not retried on every frame, and moving on works.
    let after_failure = client.description_calls().len();
    run(&mut app, &mut terminal, vec![ctrl('d')]);
    assert_eq!(client.description_calls().len(), after_failure);

    run(&mut app, &mut terminal, vec![key('j')]);
    assert_eq!(app.tasks.description().target(), Some("t2"));
    assert!(!app.tasks.description().failed());
}

#[test]
fn o_opens_the_task_from_inside_the_mode() {
    let (mut app, _client) = app_with(client());
    let mut terminal = terminal();
    let mut host = RecordingHost::default();
    open_tasks(&mut app, &mut terminal);
    run(&mut app, &mut terminal, vec![key('v')]);

    // Through the session loop with a host that records, because the hint bar
    // advertises this key from this mode and a hint for a dead key is worse
    // than no hint.
    run_session(
        &mut app,
        &mut ScriptedSource { keys: vec![key('o')] },
        &mut terminal,
        &mut host,
    )
    .expect("session runs");

    assert_eq!(
        host.opened_urls,
        vec!["https://app.asana.com/0/1/t1".to_string()],
    );
}

#[test]
fn a_cursor_on_no_task_asks_for_nothing() {
    let (mut app, client) = app_with(FakeAsanaClient::new(vec![Project::new(
        "1", "Inbox", true,
    )]));
    let mut terminal = terminal();
    open_tasks(&mut app, &mut terminal);

    run(&mut app, &mut terminal, vec![key('v')]);

    assert_eq!(app.mode(), Mode::Description);
    assert_eq!(app.tasks.description().target(), None);
    assert!(client.description_calls().is_empty());
}
