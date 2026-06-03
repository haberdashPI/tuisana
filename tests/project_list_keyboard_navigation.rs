use tuisana::{
    app::App,
    asana::fake::FakeAsanaClient,
    config::Config,
    input::KeyBinding,
    ui::runtime::{run_project_list_session, KeySource},
    domain::Project,
};
use ratatui::{backend::TestBackend, Terminal};

struct ScriptedSource {
    keys: Vec<KeyBinding>,
}

impl KeySource for ScriptedSource {
    fn next_key(&mut self) -> std::io::Result<Option<KeyBinding>> {
        if self.keys.is_empty() {
            Ok(None)
        } else {
            Ok(Some(self.keys.remove(0)))
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
        keys: vec![KeyBinding::Char('j'), KeyBinding::Char('q')],
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
