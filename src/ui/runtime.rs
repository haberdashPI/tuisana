use std::{io, io::Stdout};

use crossterm::event::{self, Event};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};

use crate::{
    app::App,
    asana::AsanaClient,
    input::{AppCommand, KeyBinding},
};

pub trait KeySource {
    fn next_key(&mut self) -> io::Result<Option<KeyBinding>>;
}

pub struct CrosstermKeySource;

impl KeySource for CrosstermKeySource {
    fn next_key(&mut self) -> io::Result<Option<KeyBinding>> {
        loop {
            match event::read()? {
                Event::Key(key_event) => {
                    if let Some(binding) = KeyBinding::from_crossterm_event(key_event) {
                        return Ok(Some(binding));
                    }
                }
                _ => {}
            }
        }
    }
}

fn draw<B: Backend, C: AsanaClient>(terminal: &mut Terminal<B>, app: &App<C>) -> io::Result<()> {
    terminal
        .draw(|frame| {
            let size = frame.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(size);

            frame.render_widget(Paragraph::new("Projects"), chunks[0]);
            frame.render_widget(Paragraph::new(status_line(&app.projects)), chunks[1]);

            let items: Vec<ListItem> = app
                .projects
                .items()
                .iter()
                .enumerate()
                .map(|(index, project)| {
                    let selected = if app.projects.selected_index() == Some(index) {
                        ">"
                    } else {
                        " "
                    };
                    let starred = if project.starred { "*" } else { " " };
                    ListItem::new(format!("{selected} [{starred}] {}", project.name))
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("Asana Projects"))
                .highlight_symbol("> ");

            let mut state = list_state(app.projects.selected_index());
            frame.render_stateful_widget(list, chunks[2], &mut state);

            frame.render_widget(
                Paragraph::new("j/down: move down, k/up: move up, r: refresh, q/ctrl-c: quit"),
                chunks[3],
            );
        })
        .map(|_| ())
}

fn status_line(state: &crate::app::project_list::ProjectListState) -> String {
    match state.status() {
        crate::app::project_list::ProjectListStatus::Idle => "Project list idle".to_string(),
        crate::app::project_list::ProjectListStatus::Loading => "Loading projects...".to_string(),
        crate::app::project_list::ProjectListStatus::Ready => {
            format!("{} project(s)", state.items().len())
        }
        crate::app::project_list::ProjectListStatus::Empty => "No projects".to_string(),
        crate::app::project_list::ProjectListStatus::Error(message) => format!("Error: {message}"),
    }
}

fn list_state(selected: Option<usize>) -> ListState {
    let mut state = ListState::default();
    state.select(selected);
    state
}

pub fn run_project_list_session<C, S, B>(
    app: &mut App<C>,
    source: &mut S,
    terminal: &mut Terminal<B>,
) -> io::Result<()>
where
    C: AsanaClient,
    S: KeySource,
    B: Backend,
{
    draw(terminal, app)?;
    let keymap = app
        .keymap()
        .map_err(|err| io::Error::other(err.to_string()))?;

    while let Some(key) = source.next_key()? {
        if let Some(action) = keymap.action_for(&key).cloned() {
            match app.handle_action(&action) {
                Some(AppCommand::Quit) => break,
                Some(AppCommand::Refresh) => {
                    app.load_projects()
                        .map_err(|err| io::Error::other(err.to_string()))?;
                    draw(terminal, app)?;
                }
                None => {
                    draw(terminal, app)?;
                }
            }
        }
    }

    Ok(())
}

pub fn run_project_list_app<C, S>(
    app: &mut App<C>,
    source: &mut S,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> io::Result<()>
where
    C: AsanaClient,
    S: KeySource,
{
    crossterm::terminal::enable_raw_mode()?;
    let result = run_project_list_session(app, source, terminal);
    let _ = crossterm::terminal::disable_raw_mode();
    result
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};
    use std::io;

    use crate::{asana::fake::FakeAsanaClient, config::Config, domain::Project};

    use super::{run_project_list_session, KeySource};
    use crate::{app::App, input::KeyBinding};

    struct ScriptedSource {
        keys: Vec<KeyBinding>,
    }

    impl KeySource for ScriptedSource {
        fn next_key(&mut self) -> io::Result<Option<KeyBinding>> {
            if self.keys.is_empty() {
                Ok(None)
            } else {
                Ok(Some(self.keys.remove(0)))
            }
        }
    }

    #[test]
    fn moves_selection_until_quit() {
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
        let lines: Vec<String> = buffer
            .content
            .chunks(buffer.area.width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect();

        assert!(lines.iter().any(|line| line.contains("Projects")));
        assert!(lines.iter().any(|line| line.contains("Backlog")));
    }
}
