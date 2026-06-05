use tuisana::{
    app::App,
    asana::fake::FakeAsanaClient,
    config::Config,
    ui::runtime::{run_project_list_session, InputEvent, KeySource},
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

    run_project_list_session(&mut app, &mut source, &mut terminal).expect("session runs");

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
