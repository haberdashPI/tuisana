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
    text::Span,
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
        calendar, filter_panel, gantt, gantt_order, help_overlay, hints, layout, project_list,
        task_table::{self, TaskTableView},
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
    /// The terminal changed size, so the next frame has to be drawn from
    /// scratch rather than diffed against a buffer of the old size.
    Resize,
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

        Ok(classify(event::read()?))
    }
}

/// Maps a crossterm event onto the app's input model.
///
/// Everything maps to *something*. This used to loop on `event::read()` until a
/// key arrived, discarding resizes and mouse events — which meant one stray
/// non-key event blocked the loop, stopping the tick and freezing the screen
/// until the user happened to press something. A resize was the obvious way to
/// hit it, and the frame stayed broken for as long as you left it alone.
fn classify(event: Event) -> InputEvent {
    match event {
        Event::Key(key_event) => InputEvent::Key(key_event),
        Event::Resize(_, _) => InputEvent::Resize,
        // A mouse move or a focus change carries nothing the app acts on, but a
        // redraw costs little and never blocks.
        _ => InputEvent::Tick,
    }
}

/// Which pane the active mode is driving.
///
/// Focus decides the thick border and the mode-colored title, so the answer to
/// "where do my keys go" is visible at both ends of the screen.
fn focused_pane(mode: Mode) -> FocusedPane {
    match mode {
        Mode::Task | Mode::Gantt | Mode::GanttOrder => FocusedPane::Task,
        Mode::Project
        | Mode::ProjectSearch
        | Mode::Filter
        | Mode::FilterEdit
        | Mode::Calendar
        | Mode::Any => FocusedPane::Top,
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
                    Mode::Filter | Mode::FilterEdit | Mode::Calendar => {
                        render_filter_pane(frame, area, app, &theme, mode, focused)
                    }
                    _ => render_project_pane(frame, area, app, &theme, mode, focused),
                };
            }

            // Resolved before drawing so the hint bar can tell whether there
            // are columns off-screen worth mentioning.
            let task_view = regions.task_pane.map(|area| {
                task_table::render_task_table(&app.tasks, pane_inner(area).width as usize, &theme)
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
            // unchanged, which is the whole point of an overlay. Only one shows
            // at a time, and help wins: it is reachable from inside the picker
            // via `?`, so asking for it has to actually show it.
            if help_visible(app, mode) {
                help_overlay::render(frame, regions.body, &theme, keymap, mode);
            } else if let Some(dialog) = app.tasks.gantt().dialog() {
                gantt_order::render(frame, regions.body, &theme, keymap, dialog);
            } else if let Some(view) = calendar::calendar_view(app.tasks.filter_calendar()) {
                calendar::render(frame, regions.body, &theme, &view);
            }
        })
        .map(|_| page_size)
}

/// Whether the help overlay is showing, using the same per-pane toggle the
/// inline help used before.
fn help_visible<C: AsanaClient + Clone + Send + 'static>(app: &App<C>, mode: Mode) -> bool {
    match mode {
        Mode::Task
        | Mode::Gantt
        | Mode::GanttOrder
        | Mode::Filter
        | Mode::FilterEdit
        | Mode::Calendar => {
            app.tasks.help_details_visible()
        }
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

    // The whole loading indicator — spinner and word together — used to sit in
    // the task pane's right-hand chips, where it was easy to miss. The top left
    // is the first place the eye lands.
    let leading = task_table::loading_frame(&app.tasks, theme)
        .map(|frame| Span::styled(format!("{frame} loading "), theme.info));

    // When the task pane is hidden nothing else can report that the loaded
    // task data no longer matches the project selection, so the header does.
    let right = match app.tasks.status() {
        crate::app::task::TaskStatus::OutOfDate(_) if !app.tasks.visible() => {
            chrome::chip_spans(&[Chip::toned("tasks stale", Tone::Warn)], theme)
        }
        _ => Vec::new(),
    };

    chrome::render_header(frame, area, theme, leading, &crumbs, right);
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
            Mode::Filter | Mode::FilterEdit | Mode::Calendar => false,
            _ => app.projects.selected_count() > 0,
        },
        can_scroll: task_view.is_some_and(|view| view.max_scroll > 0),
        on_label_filter: app.tasks.filter_selected_is_labels(),
        on_date_range: app.tasks.filter_calendar_is_range(),
        timeline_windowed: app.tasks.gantt().timeline_windowed(),
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
    let mut block = chrome::pane_block(theme, focused, mode, &view.title, &view.counts);
    // The legend rides the bottom border, so turning the chart on costs no
    // body lines and reflows nothing.
    if let Some(chart) = &view.chart {
        // Two cells for the border and two for the spaces that keep the
        // legend off it, matching how pane_block pads its title chips.
        let width = area.width.saturating_sub(4) as usize;
        let mut spans = vec![ratatui::text::Span::raw(" ")];
        spans.extend(gantt::legend_line(chart, theme, width).spans);
        spans.push(ratatui::text::Span::raw(" "));
        block = block.title_bottom(ratatui::text::Line::from(spans).left_aligned());
    }
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
                        app.refresh()
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
            InputEvent::Resize => {
                // Clearing first discards the diff against the old geometry, so
                // nothing is left behind from the previous size.
                terminal.clear()?;
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
    fn every_terminal_event_maps_to_something_drawable() {
        use crossterm::event::{Event, MouseEvent, MouseEventKind};
        use ratatui::crossterm::event::KeyModifiers as Mods;

        let key = KeyEvent::new(KeyCode::Char('j'), Mods::NONE);
        assert_eq!(
            super::classify(Event::Key(key)),
            InputEvent::Key(key),
            "a key is still a key"
        );
        assert_eq!(
            super::classify(Event::Resize(80, 24)),
            InputEvent::Resize,
            "a resize asks for a fresh frame"
        );

        // The regression: these used to be swallowed by a loop that blocked on
        // `event::read()` until a key arrived, freezing the screen.
        assert_eq!(
            super::classify(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 0,
                row: 0,
                modifiers: Mods::NONE,
            })),
            InputEvent::Tick
        );
        assert_eq!(super::classify(Event::FocusGained), InputEvent::Tick);
        assert_eq!(super::classify(Event::FocusLost), InputEvent::Tick);
    }

    #[test]
    fn the_loading_spinner_sits_at_the_top_left() {
        let projects = vec![Project::new("1", "Inbox", true)];
        let client = FakeAsanaClient::new(projects.clone());
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.tasks.begin_loading(&projects);

        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let keymap = app.keymap().expect("keymap");
        super::draw(&mut terminal, &mut app, &keymap).expect("draws");

        let theme = crate::ui::theme::Theme::default();
        let lines = screen(&mut terminal);
        let header = lines[0].clone();
        let frames = theme.glyphs.spinner;

        // In the top-right chips it was easy to miss; it belongs on the side the
        // eye starts from — and the spinner and the word travel together.
        // The spinner and the word are one indicator, so assert them as one
        // string rather than as two positions.
        assert!(
            frames
                .iter()
                .any(|frame| header.contains(&format!("{frame} loading"))),
            "the header shows the spinner and the word together: {header:?}"
        );

        let word_at = header
            .char_indices()
            .position(|(byte, _)| header[byte..].starts_with("loading"))
            .expect("the header says what it is doing");
        assert!(
            word_at < header.chars().count() / 2,
            "on the left half of the header: {header:?}"
        );
        assert!(
            lines[1..].iter().all(|line| !line.contains("loading")),
            "and nothing repeats it further down the screen"
        );
    }

    /// A resize used to block the event loop: the key source looped on
    /// `event::read()` until a key arrived, so nothing redrew in between.
    #[test]
    fn a_resize_redraws_instead_of_waiting_for_a_keypress() {
        struct ResizeThenClose {
            sent: bool,
        }

        impl KeySource for ResizeThenClose {
            fn next_event(&mut self, _timeout: Duration) -> io::Result<InputEvent> {
                if self.sent {
                    return Ok(InputEvent::Closed);
                }
                self.sent = true;
                Ok(InputEvent::Resize)
            }
        }

        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        // Draw at one size, then resize the backend under the app so the old
        // frame's geometry no longer matches.
        run_session(&mut app, &mut ResizeThenClose { sent: true }, &mut terminal)
            .expect("first session runs");
        terminal
            .backend_mut()
            .resize(60, 20);

        run_session(&mut app, &mut ResizeThenClose { sent: false }, &mut terminal)
            .expect("session runs");

        let lines = screen(&mut terminal);
        assert_eq!(lines.len(), 20, "the frame was redrawn at the new height");
        assert!(
            lines[0].contains("TUISANA"),
            "and the header came back: {:?}",
            lines[0]
        );
        assert!(
            lines.iter().any(|line| line.contains("Projects")),
            "along with the panes"
        );
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
