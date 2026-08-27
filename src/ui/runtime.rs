//! Terminal runtime: the event loop and the per-frame draw.
//!
//! `draw` is deliberately thin. It asks [`layout`] where things go, asks each
//! content module for a snapshot, and hands the snapshot to that module's line
//! builders. All geometry lives in `layout`, all styling in `theme`, and all
//! text formatting in the content modules — this file only wires them together
//! and owns the loop.

use std::{io, io::Stdout, time::Duration};

use crossterm::event::{self, Event, KeyEvent};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::Rect,
    widgets::{List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};

use crate::{
    app::App,
    asana::AsanaClient,
    config::Mode,
    input::KeyMap,
    ui::{
        chrome::{self, Chip, Tone},
        filter_panel, help_overlay, hints, layout, project_list,
        task_table::{self, TaskTableView, GUTTER_WIDTH},
        theme::Theme,
    },
};

/// A single event observed by the UI runtime.
///
/// The runtime converts crossterm input into this small set of events so the
/// application loop can stay deterministic in tests and simple in production.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// A key press read from the terminal.
    Key(KeyEvent),
    /// A polling interval elapsed without any input.
    Tick,
    /// The input source ended and no more events will arrive.
    Closed,
}

/// Source abstraction for key events used by the TUI runtime.
///
/// Production uses crossterm polling, while tests can provide scripted input
/// to exercise specific interaction sequences.
///
/// `timeout` is the maximum time to wait for input before returning `Tick`.
/// A source may also return `Closed` if it has no more events to provide.
pub trait KeySource {
    fn next_event(&mut self, timeout: Duration) -> io::Result<InputEvent>;
}

/// Production key source backed by crossterm.
pub struct CrosstermKeySource;

impl KeySource for CrosstermKeySource {
    fn next_event(&mut self, timeout: Duration) -> io::Result<InputEvent> {
        if !event::poll(timeout)? {
            return Ok(InputEvent::Tick);
        }

        // Resize and mouse events are not part of the app's input model, so
        // keep reading until a key arrives.
        loop {
            if let Event::Key(key_event) = event::read()? {
                return Ok(InputEvent::Key(key_event));
            }
        }
    }
}

/// Which pane the active mode is driving.
///
/// Focus decides the thick border and the mode-colored title, so the answer to
/// "where do my keys go" is visible at both ends of the screen.
fn focused_pane(mode: Mode) -> FocusedPane {
    match mode {
        Mode::Task => FocusedPane::Task,
        Mode::Project | Mode::ProjectSearch | Mode::Filter | Mode::FilterEdit | Mode::Any => {
            FocusedPane::Top
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusedPane {
    Top,
    Task,
}

fn draw<B: Backend, C: AsanaClient + Clone + Send + 'static>(
    terminal: &mut Terminal<B>,
    app: &mut App<C>,
    keymap: &KeyMap,
) -> io::Result<usize> {
    let mut page_size = 1usize;
    app.poll_task_data();
    let theme = Theme::new(&app.config.theme);

    terminal
        .draw(|frame| {
            let mode = app.mode();
            let tasks_visible = app.tasks.visible();
            let regions = layout::regions(frame.area(), tasks_visible, app.panel_size());
            let focus = focused_pane(mode);

            render_header(frame, regions.header, app, &theme);

            if let Some(area) = regions.top_pane {
                let focused = focus == FocusedPane::Top;
                page_size = match mode {
                    Mode::Filter | Mode::FilterEdit => {
                        render_filter_pane(frame, area, app, &theme, mode, focused)
                    }
                    _ => render_project_pane(frame, area, app, &theme, mode, focused),
                };
            }

            // Resolved before drawing so the hint bar can tell whether there
            // are columns off-screen worth mentioning.
            let task_view = regions.task_pane.map(|area| {
                task_table::render_task_table(
                    &app.tasks,
                    (pane_inner(area).width as usize).saturating_sub(GUTTER_WIDTH),
                    &theme,
                )
            });

            if let (Some(area), Some(view)) = (regions.task_pane, task_view.as_ref()) {
                let focused = focus == FocusedPane::Task;
                page_size = render_task_pane(frame, area, app, &theme, mode, focused, view);
            }

            render_hint_bar(
                frame,
                regions.hint,
                app,
                &theme,
                keymap,
                mode,
                task_view.as_ref(),
            );
            chrome::render_status(
                frame,
                regions.status,
                &theme,
                mode,
                &task_table::settings_chips(&app.tasks),
            );

            // Drawn last so it sits over the panes; the layout underneath is
            // unchanged, which is the whole point of an overlay.
            if help_visible(app, mode) {
                help_overlay::render(frame, regions.body, &theme, keymap, mode);
            }
        })
        .map(|_| page_size)
}

/// Whether the help overlay is showing, using the same per-pane toggle the
/// inline help used before.
fn help_visible<C: AsanaClient + Clone + Send + 'static>(app: &App<C>, mode: Mode) -> bool {
    match mode {
        Mode::Task | Mode::Filter | Mode::FilterEdit => app.tasks.help_details_visible(),
        Mode::Project | Mode::ProjectSearch | Mode::Any => app.projects.help_details_visible(),
    }
}

fn render_header<C: AsanaClient + Clone + Send + 'static>(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App<C>,
    theme: &Theme,
) {
    let mut crumbs = Vec::new();
    let selected = app.projects.selected_count();
    crumbs.push(match selected {
        0 => match app.projects.selected_project() {
            Some(project) => project.name.clone(),
            None => "no project".to_string(),
        },
        1 => "1 project".to_string(),
        count => format!("{count} projects"),
    });

    if app.tasks.visible() {
        let count = app.tasks.table().task_count();
        crumbs.push(format!("{count} task{}", if count == 1 { "" } else { "s" }));
    }

    // When the task pane is hidden nothing else can report that the loaded
    // task data no longer matches the project selection, so the header does.
    let right = match app.tasks.status() {
        crate::app::task::TaskStatus::OutOfDate(_) if !app.tasks.visible() => {
            chrome::chip_spans(&[Chip::toned("tasks stale", Tone::Warn)], theme)
        }
        _ => Vec::new(),
    };

    chrome::render_header(frame, area, theme, &crumbs, right);
}

fn render_hint_bar<C: AsanaClient + Clone + Send + 'static>(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App<C>,
    theme: &Theme,
    keymap: &KeyMap,
    mode: Mode,
    task_view: Option<&TaskTableView>,
) {
    let context = hints::HintContext {
        searching: app.projects.search_active() || !app.projects.search_query().is_empty(),
        has_selection: match mode {
            Mode::Task => app.tasks.selected_task_count() > 0,
            Mode::Filter | Mode::FilterEdit => false,
            _ => app.projects.selected_count() > 0,
        },
        can_scroll: task_view.is_some_and(|view| view.max_scroll > 0),
        on_label_filter: app.tasks.filter_selected_is_labels(),
    };

    let line = hints::hint_line(
        &hints::hints_for(mode, context),
        keymap,
        mode,
        theme,
        area.width as usize,
    );
    chrome::render_bar(frame, area, line);
}

fn render_project_pane<C: AsanaClient + Clone + Send + 'static>(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut App<C>,
    theme: &Theme,
    mode: Mode,
    focused: bool,
) -> usize {
    let view = project_list::render_project_list(&app.projects);
    let mut block = chrome::pane_block(theme, focused, mode, &view.title, &view.counts);
    if let Some(search) = &view.search {
        block = block.title_bottom(project_list::search_footer_line(search, theme).left_aligned());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(message) = &view.message {
        chrome::render_pane_message(frame, inner, theme, message);
        return inner.height.max(1) as usize;
    }

    // `highlight_symbol` reserves its width on every row, so it doubles as the
    // cursor gutter and keeps the names aligned.
    let cursor_symbol = format!("{} ", theme.glyphs.cursor);
    let row_width = (inner.width as usize)
        .saturating_sub(crate::ui::text::visible_width(&cursor_symbol));
    let items = view
        .rows
        .iter()
        .map(|row| ListItem::new(project_list::project_row_line(row, theme, row_width)))
        .collect::<Vec<_>>();

    let list = List::new(items)
        .highlight_symbol(&cursor_symbol)
        .highlight_style(theme.cursor);
    let mut state = list_state(app.projects.selected_index());
    frame.render_stateful_widget(list, inner, &mut state);

    inner.height.max(1) as usize
}

fn render_filter_pane<C: AsanaClient + Clone + Send + 'static>(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut App<C>,
    theme: &Theme,
    mode: Mode,
    focused: bool,
) -> usize {
    let Some(view) = filter_panel::render_filter_panel(&app.tasks) else {
        return 1;
    };

    let block = chrome::pane_block(theme, focused, mode, &view.title, &view.counts);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(message) = &view.message {
        chrome::render_pane_message(frame, inner, theme, message);
        return 1;
    }

    app.tasks.ensure_filter_visible(inner.height as usize);
    let lines = filter_panel::filter_panel_lines(&view, theme, inner.width as usize);
    frame.render_widget(
        Paragraph::new(lines).scroll((app.tasks.filter_panel_scroll() as u16, 0)),
        inner,
    );

    inner.height.max(1) as usize
}

fn render_task_pane<C: AsanaClient + Clone + Send + 'static>(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut App<C>,
    theme: &Theme,
    mode: Mode,
    focused: bool,
    view: &TaskTableView,
) -> usize {
    let block = chrome::pane_block(theme, focused, mode, &view.title, &view.counts);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(message) = &view.message {
        chrome::render_pane_message(frame, inner, theme, message);
        return inner.height.max(1) as usize;
    }

    let (header_area, body_area) = split_header(inner);
    frame.render_widget(
        Paragraph::new(task_table::task_header_line(
            view,
            theme,
            header_area.width as usize,
        )),
        header_area,
    );

    let body_height = body_area.height as usize;
    app.tasks.ensure_selected_visible(body_height);
    frame.render_widget(
        Paragraph::new(task_table::task_body_lines(
            view,
            app.tasks.selected_index(),
            theme,
            body_area.width as usize,
        ))
        .scroll((app.tasks.vertical_scroll() as u16, 0)),
        body_area,
    );

    body_height.max(1)
}

/// The interior of a pane frame, which always has a one-cell border all round.
///
/// Computed rather than taken from `Block::inner` so the task table's column
/// widths can be resolved before the block that will hold them is built.
fn pane_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Splits a pane's interior into a one-line column header and the body below.
fn split_header(inner: Rect) -> (Rect, Rect) {
    let header_height = 1u16.min(inner.height);
    (
        Rect {
            height: header_height,
            ..inner
        },
        Rect {
            y: inner.y + header_height,
            height: inner.height - header_height,
            ..inner
        },
    )
}

fn list_state(selected: Option<usize>) -> ListState {
    let mut state = ListState::default();
    state.select(selected);
    state
}

/// Run the TUI session until the user quits or the input source closes.
///
/// This owns the event loop for the terminal UI:
/// - draws the current `App` state into the provided terminal
/// - polls the supplied `KeySource` for key events and tick events
/// - forwards keys to `App::handle_key_event`
/// - reloads or redraws the UI when the app requests it
///
/// This function is split out from `run_app` so integration tests can drive the
/// session loop with a scripted `KeySource` and `TestBackend` without enabling
/// crossterm raw mode.
pub fn run_session<C, S, B>(
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

    let keymap = app
        .keymap()
        .map_err(|err| io::Error::other(err.to_string()))?;
    let mut page_size = draw(terminal, app, &keymap)?;

    loop {
        match source.next_event(TICK_RATE)? {
            InputEvent::Key(key_event) => {
                match app.handle_key_event(&keymap, key_event, page_size) {
                    Ok(Some(crate::input::AppCommand::Quit)) => break,
                    Ok(Some(crate::input::AppCommand::Refresh)) => {
                        app.load_projects()
                            .map_err(|err| io::Error::other(err.to_string()))?;
                        page_size = draw(terminal, app, &keymap)?;
                    }
                    Ok(Some(crate::input::AppCommand::OpenUrl(url))) => {
                        let _ = std::process::Command::new("open").arg(&url).spawn();
                        page_size = draw(terminal, app, &keymap)?;
                    }
                    Ok(Some(crate::input::AppCommand::CopyToClipboard(text))) => {
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            let _ = clipboard.set_text(text);
                        }
                        page_size = draw(terminal, app, &keymap)?;
                    }
                    Ok(None) => {
                        page_size = draw(terminal, app, &keymap)?;
                    }
                    Err(err) => {
                        return Err(io::Error::other(err.to_string()));
                    }
                }
            }
            InputEvent::Tick => {
                page_size = draw(terminal, app, &keymap)?;
            }
            InputEvent::Closed => break,
        }
    }

    Ok(())
}

/// Run the application in a real terminal by enabling raw mode around `run_session`.
///
/// This is a thin production wrapper around `run_session`; the separation keeps the
/// core session loop testable with fake input sources and test backends.
pub fn run_app<C, S>(
    app: &mut App<C>,
    source: &mut S,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> io::Result<()>
where
    C: AsanaClient + Clone + Send + 'static,
    S: KeySource,
{
    crossterm::terminal::enable_raw_mode()?;
    let result = run_session(app, source, terminal);
    let _ = crossterm::terminal::disable_raw_mode();
    result
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};
    use std::io;

    use crate::{asana::fake::FakeAsanaClient, config::Config, domain::Project};

    use super::{run_session, InputEvent, KeySource};
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

    fn screen(terminal: &mut Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend_mut().buffer().clone();
        buffer
            .content
            .chunks(buffer.area.width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect()
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
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");

        run_session(&mut app, &mut source, &mut terminal).expect("session runs");

        assert_eq!(app.projects.selected_index(), Some(1));
        let lines = screen(&mut terminal);
        assert!(lines.iter().any(|line| line.contains("Projects")));
        assert!(lines.iter().any(|line| line.contains("Backlog")));
    }

    #[test]
    fn chrome_occupies_the_first_and_last_two_lines() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        let mut source = ScriptedSource { keys: Vec::new() };
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        run_session(&mut app, &mut source, &mut terminal).expect("session runs");

        let lines = screen(&mut terminal);
        assert!(lines[0].contains("TUISANA"));
        assert!(lines[10].contains("quit"), "hint bar: {:?}", lines[10]);
        assert!(lines[11].contains("PROJECT"), "status bar: {:?}", lines[11]);
    }

    #[test]
    fn toggling_help_does_not_move_the_panes() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut source = ScriptedSource { keys: Vec::new() };
        run_session(&mut app, &mut source, &mut terminal).expect("session runs");
        let before = screen(&mut terminal);

        let mut source = ScriptedSource {
            keys: vec![KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)],
        };
        run_session(&mut app, &mut source, &mut terminal).expect("session runs");
        let with_help = screen(&mut terminal);

        assert!(app.projects.help_details_visible());
        assert!(with_help.iter().any(|line| line.contains("Help")));
        // The header, hint bar, and status bar are untouched by the overlay.
        assert_eq!(before[0], with_help[0]);
        assert_eq!(before[22], with_help[22]);
        assert_eq!(before[23], with_help[23]);
    }
}
