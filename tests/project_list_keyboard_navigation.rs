use tuisana::{
    app::App,
    asana::fake::FakeAsanaClient,
    config::{Config, NamedFilterSet},
    ui::runtime::{run_session, InputEvent, KeySource, RecordingHost},
    domain::Project,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use std::time::Duration;

struct ScriptedSource {
    keys: Vec<KeyEvent>,
}

impl KeySource for ScriptedSource {
    fn next_event(&mut self, _timeout: Duration) -> std::io::Result<InputEvent> {
        if self.keys.is_empty() {
            Ok(InputEvent::Closed)
        } else {
            Ok(InputEvent::Key(self.keys.remove(0)))
        }
    }
}

#[test]
fn keyboard_input_moves_project_selection() {
    let client = FakeAsanaClient::new(vec![
        Project::new("1", "Inbox", true),
        Project::new("2", "Backlog", false),
    ]);
    let mut app = App::new(Config::default(), client);
    app.load_projects().expect("projects load");

    let mut source = ScriptedSource {
        keys: vec![
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ],
    };
    let backend = TestBackend::new(60, 10);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_session(&mut app, &mut source, &mut terminal, &mut RecordingHost::default())
            .expect("session runs");

    assert_eq!(app.projects.selected_index(), Some(1));
    let buffer = terminal.backend_mut().buffer().clone();
    let text = buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Backlog"));
    assert!(text.contains("Projects"));
}

/// `b` and a digit, pressed in the project view, with the whole runtime in
/// the loop: the sidebar has to be on screen for the digit to address a row,
/// and the row it addresses has to move the selection.
#[test]
fn the_sets_sidebar_and_its_digits_work_from_the_project_view() {
    let client = FakeAsanaClient::new(vec![
        Project::new("1", "Inbox", true),
        Project::new("2", "Backlog", true),
    ]);
    let mut config = Config::default();
    config.filter_sets = vec![NamedFilterSet {
        name: "Backlog only".to_string(),
        scratch: false,
        projects: Some(vec!["2".to_string()]),
        sets: Vec::new(),
    }];
    let mut app = App::new(config, client);
    app.load_projects().expect("projects load");

    let mut source = ScriptedSource {
        keys: vec![
            // The first project selected, then the entry that selects the
            // other one instead.
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ],
    };
    // Wide enough that the sidebar survives beside the project list.
    let backend = TestBackend::new(100, 16);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_session(&mut app, &mut source, &mut terminal, &mut RecordingHost::default())
            .expect("session runs");

    let selected = app
        .projects
        .selected_projects()
        .into_iter()
        .map(|project| project.id)
        .collect::<Vec<_>>();
    assert_eq!(selected, vec!["2".to_string()]);
    assert_eq!(app.tasks.filter_set_loaded_name(), Some("Backlog only"));

    let buffer = terminal.backend_mut().buffer().clone();
    let text = buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    // The sidebar beside the project list, and the project pane's border
    // naming the entry the selection now belongs to.
    assert!(text.contains("Sets"), "{text}");
    assert!(text.contains("1 project"), "{text}");
    assert!(text.contains("Projects"), "{text}");
    assert!(
        text.matches("Backlog only").count() >= 2,
        "the sidebar and the project pane's chip both name it: {text}"
    );
}
