use std::{io, io::Stdout};

use crossterm::event::{self, Event, KeyEvent};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    prelude::*,
    widgets::{Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, TableState},
    Terminal,
};

use crate::{
    app::App,
    asana::AsanaClient,
    ui::project_list::render_project_list,
    ui::task_table::render_task_table,
};

pub trait KeySource {
    fn next_key(&mut self) -> io::Result<Option<KeyEvent>>;
}

pub struct CrosstermKeySource;

impl KeySource for CrosstermKeySource {
    fn next_key(&mut self) -> io::Result<Option<KeyEvent>> {
        loop {
            match event::read()? {
                Event::Key(key_event) => {
                    return Ok(Some(key_event));
                }
                _ => {}
            }
        }
    }
}

fn draw<B: Backend, C: AsanaClient>(terminal: &mut Terminal<B>, app: &App<C>) -> io::Result<usize> {
    let mut page_size = 1usize;
    terminal
        .draw(|frame| {
            let view = render_project_list(&app.projects);
            let size = frame.area();
            let hint_height = view.hint_lines.len().max(1) as u16;
            let task_visible = app.tasks.visible();
            let project_height = if task_visible {
                7u16.min(size.height.max(1))
            } else {
                size.height
            };
            let project_area = Rect::new(size.x, size.y, size.width, project_height);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(hint_height),
                    Constraint::Min(1),
                ])
                .split(project_area);
            page_size = chunks[4].height.saturating_sub(2).max(1) as usize;

            frame.render_widget(Paragraph::new(view.title.as_str()), chunks[0]);
            frame.render_widget(Paragraph::new(view.status_line.as_str()), chunks[1]);
            frame.render_widget(Paragraph::new(view.search_line.as_str()), chunks[2]);
            frame.render_widget(Paragraph::new(view.hint_lines.join("\n")), chunks[3]);

            let items: Vec<ListItem> = view
                .rows
                .iter()
                .cloned()
                .map(ListItem::new)
                .collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("Asana Projects"))
                .highlight_symbol("> ");

            let mut state = list_state(app.projects.selected_index());
            frame.render_stateful_widget(list, chunks[4], &mut state);

            if task_visible {
                let task_area = Rect::new(
                    size.x,
                    size.y + project_height,
                    size.width,
                    size.height.saturating_sub(project_height),
                );
                let task_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(1),
                    ])
                    .split(task_area);
                let task_view = render_task_table(&app.tasks);

                frame.render_widget(Paragraph::new(task_view.title.as_str()), task_chunks[0]);
                frame.render_widget(Paragraph::new(task_view.status_line.as_str()), task_chunks[1]);

                let rows = task_view
                    .rows
                    .iter()
                    .map(|row| Row::new(row.iter().cloned().map(Cell::from)))
                    .collect::<Vec<_>>();
                let widths = task_column_widths(task_view.columns.len());
                let table = Table::new(rows, widths)
                    .header(Row::new(
                        task_view
                            .columns
                            .iter()
                            .cloned()
                            .map(Cell::from)
                            .collect::<Vec<_>>(),
                    ))
                    .block(Block::default().borders(Borders::ALL).title("Tasks"))
                    .highlight_symbol("> ");
                let mut table_state = table_state(app.tasks.selected_index());
                frame.render_stateful_widget(table, task_chunks[2], &mut table_state);
            }
        })
        .map(|_| page_size)
}

fn list_state(selected: Option<usize>) -> ListState {
    let mut state = ListState::default();
    state.select(selected);
    state
}

fn table_state(selected: Option<usize>) -> TableState {
    let mut state = TableState::default();
    state.select(selected);
    state
}

fn task_column_widths(column_count: usize) -> Vec<Constraint> {
    let mut widths = vec![
        Constraint::Length(20),
        Constraint::Length(14),
        Constraint::Length(14),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(8),
        Constraint::Length(18),
    ];
    while widths.len() < column_count {
        widths.push(Constraint::Length(16));
    }
    widths.truncate(column_count);
    widths
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
    let mut page_size = draw(terminal, app)?;
    let keymap = app.keymap().map_err(|err| io::Error::other(err.to_string()))?;

    while let Some(key_event) = source.next_key()? {
        match app.handle_key_event(&keymap, key_event, page_size) {
            Ok(Some(crate::input::AppCommand::Quit)) => break,
            Ok(Some(crate::input::AppCommand::Refresh)) => {
                app.load_projects()
                    .map_err(|err| io::Error::other(err.to_string()))?;
                page_size = draw(terminal, app)?;
            }
            Ok(None) => {
                page_size = draw(terminal, app)?;
            }
            Err(err) => {
                return Err(io::Error::other(err.to_string()));
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
    use crate::app::App;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    struct ScriptedSource {
        keys: Vec<KeyEvent>,
    }

    impl KeySource for ScriptedSource {
        fn next_key(&mut self) -> io::Result<Option<KeyEvent>> {
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
        let lines: Vec<String> = buffer
            .content
            .chunks(buffer.area.width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect();

        assert!(lines.iter().any(|line| line.contains("Projects")));
        assert!(lines.iter().any(|line| line.contains("Backlog")));
    }
}
