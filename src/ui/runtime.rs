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
        calendar, filter_panel, filter_sets, gantt, gantt_order, help_overlay, hints, layout,
        project_list,
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

    /// Whether another event is already waiting behind the one just returned.
    ///
    /// The loop uses this to hold off the expensive half of a keystroke —
    /// rebuilding the task table — while the user is still typing. A source
    /// that cannot see its own queue says `false`, which simply means every
    /// keystroke settles immediately, as they all used to.
    fn pending(&mut self) -> bool {
        false
    }
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

    fn pending(&mut self) -> bool {
        // A held-down key, a paste, and plain fast typing all put several
        // events in the queue at once; a zero timeout answers "is there
        // another one right now" without waiting for one.
        event::poll(Duration::ZERO).unwrap_or(false)
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
        Mode::Task | Mode::TaskEdit | Mode::Gantt | Mode::GanttOrder => FocusedPane::Task,
        Mode::Project
        | Mode::ProjectSearch
        | Mode::Filter
        | Mode::FilterEdit
        | Mode::FilterSetName
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
    // Anything a keystroke put off happens here, so the frame about to be
    // drawn is built from every key the user has actually typed. A no-op while
    // input is still queued, which is what keeps a burst to one rebuild.
    app.tasks.settle_table();
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
                    Mode::Filter
                    | Mode::FilterEdit
                    | Mode::FilterSetName
                    | Mode::Calendar => {
                        render_filter_pane(frame, area, app, &theme, mode, focused)
                    }
                    _ => render_project_pane(frame, area, app, &theme, mode, focused),
                };
            }

            // Resolved before drawing so the hint bar can tell whether there
            // are columns off-screen worth mentioning.
            // The cursor is drawn only where it can be moved: in Gantt mode
            // `h` and `l` scroll the timeline, so there is no column cursor
            // to show there.
            let cursor_column = matches!(mode, Mode::Task | Mode::TaskEdit)
                .then(|| app.tasks.selected_column());
            let task_view = regions.task_pane.map(|area| {
                task_table::render_task_table(
                    &mut app.tasks,
                    pane_inner(area).width as usize,
                    &theme,
                    cursor_column,
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
            // unchanged, which is the whole point of an overlay. Only one
            // shows at a time.
            //
            // A confirmation wins outright: every key goes to it until it is
            // answered, and `?` is not among them, so help showing over it
            // would be stale and unclosable. Below that help wins, because it
            // *is* reachable from inside the picker via `?` and asking for it
            // has to actually show it.
            if let Some(confirm) = filter_sets::confirm_view(&app.tasks) {
                filter_sets::render_confirm(frame, regions.body, &theme, mode, &confirm);
            } else if help_visible(app, mode) {
                help_overlay::render(frame, regions.body, &theme, keymap, mode);
            } else if let Some(dialog) = app.tasks.gantt().dialog() {
                gantt_order::render(frame, regions.body, &theme, keymap, dialog);
            } else if let Some(view) = calendar::calendar_view(app.tasks.calendar()) {
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
        | Mode::TaskEdit
        | Mode::Gantt
        | Mode::GanttOrder
        | Mode::Filter
        | Mode::FilterEdit
        | Mode::FilterSetName
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
    //
    // Filtering shares the spot rather than taking one of its own: they are
    // the same statement to the reader — the table is not the answer yet —
    // and a fetch in flight is the bigger of the two, so it wins the space.
    let leading = task_table::loading_frame(&app.tasks, theme)
        .map(|frame| Span::styled(format!("{frame} loading "), theme.info))
        .or_else(|| {
            task_table::filtering_frame(&app.tasks, theme)
                .map(|frame| Span::styled(format!("{frame} filtering "), theme.warn))
        });

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
            Mode::Task | Mode::TaskEdit => app.tasks.selected_task_count() > 0,
            Mode::Filter | Mode::FilterEdit | Mode::FilterSetName | Mode::Calendar => false,
            _ => app.projects.selected_count() > 0,
        },
        can_scroll: task_view.is_some_and(|view| view.max_scroll > 0),
        on_label_filter: app.tasks.filter_selected_is_labels(),
        on_date_range: app.tasks.calendar_is_range(),
        timeline_windowed: app.tasks.gantt().timeline_windowed(),
        many_filter_sets: app.tasks.filter_set_position().1 > 1,
        filter_sets_sidebar: app.tasks.filter_sets_sidebar_visible(),
        saved_filter_sets: app.config.filter_sets.len(),
        filter_set_loaded: app.tasks.filter_set_loaded_name().is_some(),
        filter_set_confirm: app.tasks.filter_set_prompt_is_confirmation(),
        on_task_cell: app.tasks.cell_edit_open(),
        task_edit_is_options: app.tasks.cell_edit_is_options(),
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

    // The split is inside the filter pane rather than in `layout`: the
    // sidebar belongs to the panel, not to the frame.
    let (sidebar, area) =
        filter_sets::split_sidebar(area, app.tasks.filter_sets_sidebar_visible());
    if let Some(sidebar) = sidebar {
        render_filter_sets_pane(frame, sidebar, app, theme, mode);
    }
    // The prompt normally rides the sidebar's border. When the sidebar has
    // given way to the filter rows it borrows theirs, because a prompt with
    // nowhere to be makes `w` a blind edit.
    let orphaned_prompt = sidebar
        .is_none()
        .then(|| filter_sets::prompt_line(&app.tasks))
        .flatten();

    // The tab strip rides the top border beside the title, so the sets cost no
    // interior line and the scroll offset stays a plain field index.
    let tabs = filter_panel::tab_strip_spans(
        &view,
        theme,
        focused,
        mode,
        chrome::title_extra_budget(theme, area.width, &view.title, &view.counts),
    );
    let mut block = chrome::pane_block_with_title_extra(
        theme,
        focused,
        mode,
        &view.title,
        tabs,
        &view.counts,
    );
    if let Some(prompt) = &orphaned_prompt {
        block = block.title_bottom(
            filter_sets::prompt_footer_line(
                prompt,
                theme,
                area.width.saturating_sub(2) as usize,
            )
            .left_aligned(),
        );
    }
    let body = block.inner(area);
    frame.render_widget(block, area);

    if let Some(message) = &view.message {
        chrome::render_pane_message(frame, body, theme, message);
        return 1;
    }

    app.tasks.ensure_filter_visible(body.height as usize);
    let lines = filter_panel::filter_panel_lines(&view, theme, body.width as usize);
    frame.render_widget(
        Paragraph::new(lines).scroll((app.tasks.filter_panel_scroll() as u16, 0)),
        body,
    );

    // The page size is what is actually visible, so ctrl-d pages by the rows
    // rather than by the pane's height.
    body.height.max(1) as usize
}

/// Draws the `Sets` sidebar.
///
/// Always unfocused: the thin border is the signal that keys do not go there.
fn render_filter_sets_pane<C: AsanaClient + Clone + Send + 'static>(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut App<C>,
    theme: &Theme,
    mode: Mode,
) {
    // The renderer is the only thing that knows how tall the pane is, so it
    // is what tells the panel how many entries a numbered window holds.
    app.tasks
        .set_filter_sets_window(filter_sets::window_rows(area));

    let Some(view) = filter_sets::render_filter_sets(&app.tasks, &app.config.filter_sets)
    else {
        return;
    };

    let mut block = chrome::pane_block(theme, false, mode, "Sets", &view.counts);
    if let Some(prompt) = &view.prompt {
        block = block.title_bottom(
            filter_sets::prompt_footer_line(
                prompt,
                theme,
                area.width.saturating_sub(2) as usize,
            )
            .left_aligned(),
        );
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    frame.render_widget(
        Paragraph::new(filter_sets::filter_sets_lines(
            &view,
            theme,
            inner.width as usize,
        )),
        inner,
    );
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
        let event = source.next_event(TICK_RATE)?;
        // Checked once per event, before anything acts on it: a keystroke with
        // more input behind it leaves the table to be rebuilt later, and
        // `draw` settles it as soon as the queue runs dry.
        app.tasks.set_input_pending(source.pending());

        match event {
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

        /// A scripted burst is exactly what fast typing looks like to the
        /// loop: every key but the last has another one behind it.
        fn pending(&mut self) -> bool {
            !self.keys.is_empty()
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

    /// A session with two tasks loaded and the filter panel open in edit mode.
    fn filtering_app() -> App<FakeAsanaClient> {
        use crate::asana::dto::{
            SectionDto, TaskDto, TaskMembershipDto, TaskMembershipProjectDto,
            TaskMembershipSectionDto,
        };

        let task = |gid: &str, name: &str| TaskDto {
            gid: gid.to_string(),
            name: name.to_string(),
            completed: false,
            modified_at: None,
            due_on: None,
            start_on: None,
            assignee: None,
            num_subtasks: 0,
            memberships: vec![TaskMembershipDto {
                project: TaskMembershipProjectDto {
                    gid: "1".to_string(),
                    name: "Inbox".to_string(),
                },
                section: Some(TaskMembershipSectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }),
            }],
            parent: None,
            custom_fields: vec![],
        };

        let projects = vec![Project::new("1", "Inbox", true)];
        let client = FakeAsanaClient::new(projects.clone())
            .with_sections(
                "1",
                vec![SectionDto { gid: "s1".to_string(), name: "Today".to_string() }],
            )
            .with_tasks("1", vec![task("t1", "Alpha"), task("t2", "Beta")]);

        let mut app = App::new(Config::default(), client.clone());
        app.load_projects().expect("projects load");
        app.tasks
            .load_task_dataset_for_projects(&client, &projects)
            .expect("tasks load");
        app.tasks.set_visible(true);
        app
    }

    #[test]
    fn a_burst_of_typing_leaves_the_table_current_once_the_keys_run_out() {
        // Every key but the last has another behind it, so the table is left
        // out of date while they arrive — and the loop has to settle it before
        // the frame the user finally reads.
        let mut app = filtering_app();
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");

        // `f` opens the panel, `enter` starts editing the Title row, then two
        // characters are typed: the first has the second queued behind it.
        let mut keys = vec![
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ];
        keys.extend(
            "ph".chars()
                .map(|ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)),
        );
        let mut source = ScriptedSource { keys };
        run_session(&mut app, &mut source, &mut terminal).expect("session runs");

        assert!(
            app.tasks.filtering_since().is_none(),
            "nothing is left pending once the burst ends"
        );
        let titles = app
            .tasks
            .table()
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.cells[0].clone())
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["Alpha".to_string()], "`ph` is only in Alpha");
    }

    #[test]
    fn a_key_with_more_behind_it_does_not_rebuild_the_table() {
        // The same burst, stopped one key early: the source still reports a
        // key queued, so the table is deliberately a frame behind.
        let mut app = filtering_app();
        let keymap = app.keymap().expect("keymap");
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");

        for key in [
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ] {
            app.handle_key_event(&keymap, key, 10)
                .expect("opens the filter panel and starts editing");
        }

        app.tasks.set_input_pending(true);
        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
            10,
        )
        .expect("types");

        assert!(app.tasks.filtering_since().is_some());
        assert_eq!(app.tasks.table().task_count(), 2, "still the old table");

        // And the draw that follows an emptied queue brings it up to date.
        app.tasks.set_input_pending(false);
        super::draw(&mut terminal, &mut app, &keymap).expect("draws");

        assert!(app.tasks.filtering_since().is_none());
        assert_eq!(app.tasks.table().task_count(), 1);
    }

    #[test]
    fn a_table_left_behind_says_so_in_the_header() {
        // The predicate is unit-tested next to the spinner; this is the wiring
        // — that a stale table actually reaches the one indicator on screen.
        let mut app = filtering_app();
        let keymap = app.keymap().expect("keymap");
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");

        for key in [
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ] {
            app.handle_key_event(&keymap, key, 10).expect("opens the panel");
        }

        app.tasks.set_input_pending(true);
        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
            10,
        )
        .expect("types");

        // Long enough to be worth reporting, which is the whole condition.
        std::thread::sleep(Duration::from_millis(180));
        super::draw(&mut terminal, &mut app, &keymap).expect("draws");

        let header = screen(&mut terminal).remove(0);
        assert!(
            header.contains("filtering"),
            "the header should report the wait: {header:?}"
        );
        assert!(
            app.tasks.filtering_since().is_some(),
            "the draw must not have settled it while keys are still queued"
        );
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
