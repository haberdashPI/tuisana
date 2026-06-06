use std::{io, io::Stdout, time::Duration};

use crossterm::event::{self, Event, KeyEvent};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};

use crate::{
    app::App,
    asana::AsanaClient,
    ui::project_list::render_project_list,
    ui::task_table::{build_task_filter_lines, build_task_header_line, build_task_body_lines, render_task_filter_panel, render_task_table},
};

const PROJECT_VISIBLE_ROWS: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Key(KeyEvent),
    Tick,
    Closed,
}

pub trait KeySource {
    fn next_event(&mut self, timeout: Duration) -> io::Result<InputEvent>;
}

pub struct CrosstermKeySource;

impl KeySource for CrosstermKeySource {
    fn next_event(&mut self, timeout: Duration) -> io::Result<InputEvent> {
        if !event::poll(timeout)? {
            return Ok(InputEvent::Tick);
        }

        loop {
            match event::read()? {
                Event::Key(key_event) => return Ok(InputEvent::Key(key_event)),
                _ => {}
            }
        }
    }
}

fn draw<B: Backend, C: AsanaClient + Clone + Send + 'static>(
    terminal: &mut Terminal<B>,
    app: &mut App<C>,
) -> io::Result<usize> {
    let mut page_size = 1usize;
    app.poll_task_load();
    terminal
        .draw(|frame| {
            let view = render_project_list(&app.projects);
            let size = frame.area();
            let hint_height = view.hint_lines.len().max(1) as u16;
            let task_visible = app.tasks.visible();
            let project_height = if task_visible {
                project_panel_height(size.height, hint_height, view.rows.len())
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
            let project_page_size = chunks[4].height.saturating_sub(2).max(1) as usize;

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
                let task_view = render_task_table(&app.tasks, size.width.saturating_sub(2) as usize);
                let filter_view = render_task_filter_panel(&app.tasks);
                let task_area = Rect::new(
                    size.x,
                    size.y + project_height,
                    size.width,
                    size.height.saturating_sub(project_height),
                );
                let task_hint_height = task_view.hint_lines.len().max(1) as u16;
                let filter_height = filter_view
                    .as_ref()
                    .map(|view| build_task_filter_lines(view, size.width.saturating_sub(2) as usize).len().max(1) as u16 + 2)
                    .unwrap_or(0);
                let task_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(task_hint_height),
                        Constraint::Min(1),
                        Constraint::Length(filter_height),
                    ])
                    .split(task_area);
                let task_block = Block::default().borders(Borders::ALL).title("Tasks");
                frame.render_widget(task_block.clone(), task_chunks[3]);
                let task_inner = task_block.inner(task_chunks[3]);
                let task_body_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(1)])
                    .split(task_inner);
                let task_body_height = task_body_chunks[1].height as usize;
                app.tasks.ensure_selected_visible(task_body_height);
                page_size = task_body_height.max(1);

                frame.render_widget(Paragraph::new(task_view.title.as_str()), task_chunks[0]);
                frame.render_widget(Paragraph::new(task_view.status_line.as_str()), task_chunks[1]);
                frame.render_widget(Paragraph::new(task_view.hint_lines.join("\n")), task_chunks[2]);

                frame.render_widget(
                    Paragraph::new(build_task_header_line(&task_view, task_body_chunks[0].width as usize)),
                    task_body_chunks[0],
                );

                let body = Paragraph::new(build_task_body_lines(
                    &task_view,
                    app.tasks.selected_index(),
                    task_body_chunks[1].width as usize,
                ))
                .scroll((app.tasks.vertical_scroll() as u16, 0))
                ;
                frame.render_widget(body, task_body_chunks[1]);

                if let Some(filter_view) = filter_view {
                    let filter_lines = build_task_filter_lines(&filter_view, task_chunks[4].width as usize);
                    let filter_block = Block::default().borders(Borders::ALL).title("Filters");
                    frame.render_widget(filter_block.clone(), task_chunks[4]);
                    let filter_inner = filter_block.inner(task_chunks[4]);
                    let rendered = Paragraph::new(filter_lines);
                    frame.render_widget(rendered, filter_inner);
                }
            } else {
                page_size = project_page_size;
            }
        })
        .map(|_| page_size)
}

fn project_panel_height(total_height: u16, hint_height: u16, row_count: usize) -> u16 {
    let visible_rows = row_count.max(3).min(PROJECT_VISIBLE_ROWS) as u16;
    let desired_height = 3u16
        .saturating_add(hint_height)
        .saturating_add(2)
        .saturating_add(visible_rows);

    desired_height.min(total_height.max(1))
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
    C: AsanaClient + Clone + Send + 'static,
    S: KeySource,
    B: Backend,
{
    const TICK_RATE: Duration = Duration::from_millis(100);

    let mut page_size = draw(terminal, app)?;
    let keymap = app.keymap().map_err(|err| io::Error::other(err.to_string()))?;

    loop {
        match source.next_event(TICK_RATE)? {
            InputEvent::Key(key_event) => {
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
            InputEvent::Tick => {
                page_size = draw(terminal, app)?;
            }
            InputEvent::Closed => break,
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
    C: AsanaClient + Clone + Send + 'static,
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

    use super::{run_project_list_session, InputEvent, KeySource};
    use crate::app::App;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::Duration;

    struct ScriptedSource {
        keys: Vec<KeyEvent>,
    }

    impl KeySource for ScriptedSource {
        fn next_event(&mut self, _timeout: Duration) -> io::Result<InputEvent> {
            if self.keys.is_empty() {
                Ok(InputEvent::Closed)
            } else {
                Ok(InputEvent::Key(self.keys.remove(0)))
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

    #[test]
    fn project_panel_keeps_space_for_rows_when_tasks_are_visible() {
        assert_eq!(super::project_panel_height(40, 1, 20), 10);
        assert_eq!(super::project_panel_height(40, 2, 1), 10);
        assert_eq!(super::project_panel_height(8, 1, 20), 8);
    }
}
