use crate::{
    asana::{AsanaClient, TaskQuery, TaskTarget},
    config::{Config, Mode},
    domain::{Project, ProjectKind},
    error::Result,
    input::{Action, AppCommand, KeyBinding, KeyMap},
};

use std::{
    sync::mpsc::{self, Receiver},
    thread,
};

pub mod calendar;
pub mod gantt;
pub mod project_list;
pub mod task;

use self::gantt::MoveTo;
use self::project_list::ProjectListState;
use self::task::TaskState;

/// Internal state for the top pane's size and transient maximize/minimize mode.
///
/// The pane is shared between the project list and the filter view, so this
/// type remembers the preferred height and whether the pane is temporarily
/// hidden or expanded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneMode {
    Normal,
    Minimized,
    Maximized,
}

/// Tracks the preferred height of the top pane and how it should be rendered.
///
/// The project list and filter panel both live in this region. The app uses
/// this state to remember the user's preferred split and to support temporary
/// minimize/maximize toggles.
#[derive(Clone, Debug)]
pub struct PaneSizeState {
    preferred: u16,
    saved_preferred: u16,
    mode: PaneMode,
}

impl Default for PaneSizeState {
    fn default() -> Self {
        Self {
            preferred: 11,
            saved_preferred: 11,
            mode: PaneMode::Normal,
        }
    }
}

impl PaneSizeState {
    pub fn preferred(&self) -> u16 {
        self.preferred
    }

    pub fn is_minimized(&self) -> bool {
        matches!(self.mode, PaneMode::Minimized)
    }

    pub fn is_maximized(&self) -> bool {
        matches!(self.mode, PaneMode::Maximized)
    }

    pub fn is_normal(&self) -> bool {
        matches!(self.mode, PaneMode::Normal)
    }

    pub fn resize(&mut self, delta: i16) {
        let updated = (self.preferred as i16 + delta).max(4) as u16;
        self.preferred = updated;
        if matches!(self.mode, PaneMode::Normal) {
            self.saved_preferred = self.preferred;
        }
    }

    pub fn minimize(&mut self) {
        if !matches!(self.mode, PaneMode::Minimized) {
            self.saved_preferred = self.preferred;
            self.mode = PaneMode::Minimized;
        }
    }

    pub fn maximize(&mut self) {
        if !matches!(self.mode, PaneMode::Maximized) {
            self.saved_preferred = self.preferred;
            self.mode = PaneMode::Maximized;
        }
    }

    pub fn restore(&mut self) {
        if !matches!(self.mode, PaneMode::Normal) {
            self.preferred = self.saved_preferred.max(4);
            self.mode = PaneMode::Normal;
        }
    }

    pub fn actual_height(&self, available: u16, min_height: u16) -> u16 {
        let min_height = min_height.max(1);
        let max_height = available.max(min_height);
        match self.mode {
            PaneMode::Normal => self.preferred.clamp(min_height, max_height),
            PaneMode::Minimized => 0,
            PaneMode::Maximized => max_height,
        }
    }
}

/// Central application state and coordinator.
///
/// `App` owns the project list state, task state, bind mode, pane
/// sizing, and asynchronous task loading. The runtime and tests drive this
/// type directly so the UI stays thin and the behavior remains easy to verify.
#[derive(Debug)]
pub struct App<C> {
    pub config: Config,
    pub projects: ProjectListState,
    pub tasks: TaskState,
    mode: Mode,
    panel_size: PaneSizeState,
    client: C,
    /// Monotonic tag for the current async task-data request.
    ///
    /// Each new task-data request increments this counter. Worker messages carry the
    /// generation they were started under so `poll_task_data` can ignore stale
    /// results from an earlier request after the user has triggered a newer one.
    task_data_generation: u64,
    /// Receiver for the in-flight async task-data worker, if one is active.
    ///
    /// The app keeps the receiver here so the main loop can poll for partial
    /// results, completion, or failure without blocking the UI.
    task_data_receiver: Option<Receiver<TaskDataMessage>>,
}

/// Internal message sent back from the task-data worker thread.
///
/// The app uses this to merge task data incrementally and to know when a request
/// has completed or failed.
struct TaskDataMessage {
    generation: u64,
    project_id: Option<String>,
    query: TaskQuery,
    result: Result<crate::app::task::TaskDataset>,
    done: bool,
}

impl<C: AsanaClient + Clone + Send + 'static> App<C> {
    pub fn new(config: Config, client: C) -> Self {
        let mut tasks = TaskState::new();
        tasks.apply_gantt_config(&config.gantt);
        Self {
            config,
            projects: ProjectListState::new(),
            tasks,
            mode: Mode::Project,
            panel_size: PaneSizeState::default(),
            client,
            task_data_generation: 0,
            task_data_receiver: None,
        }
    }

    pub fn load_projects(&mut self) -> Result<()> {
        self.projects
            .load_with_visibility(&self.client, &self.config.project_visibility)?;
        // The "assigned to me" row is best-effort: if the client can't resolve
        // who's logged in (e.g. the token lacks permission, or a test double
        // has no fake user configured), just omit the row instead of failing
        // project loading entirely.
        let assigned_to_me = match self.client.current_user_gid() {
            Ok(gid) => Some(Project::assigned_to_me(gid)),
            Err(err) => {
                debug_log(&format!("current_user_gid unavailable: {err}"));
                None
            }
        };
        self.projects.set_assigned_to_me(assigned_to_me);
        Ok(())
    }

    /// Start the asynchronous task-data request for the currently selected projects.
    pub fn request_task_data(&mut self) -> Result<()> {
        self.start_task_data_fetch();
        Ok(())
    }

    /// Reload the project list and force a fresh task fetch for the current
    /// selection.
    ///
    /// A plain `ensure_task_data` call would see the selection already
    /// "covered" by the cache and skip fetching, so a manual refresh needs to
    /// invalidate that cache first — otherwise pressing refresh on a project
    /// that's already loaded looks like it does nothing.
    pub fn refresh(&mut self) -> Result<()> {
        self.load_projects()?;
        self.tasks.invalidate_cache();
        if self.task_data_receiver.is_some() {
            self.task_data_generation = self.task_data_generation.wrapping_add(1);
        }
        if self.tasks.visible() {
            self.start_task_data_fetch();
        } else {
            self.tasks
                .mark_out_of_date("refreshed; switch to task view to reload");
        }
        Ok(())
    }

    pub fn keymap(&self) -> Result<KeyMap> {
        KeyMap::from_bindings(&self.config.effective_bindings())
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn panel_size(&self) -> &PaneSizeState {
        &self.panel_size
    }

    /// Shared setup for mode transitions that use the top pane: restores the
    /// pane if it was minimized and closes any active project search.
    fn prepare_mode_switch(&mut self) {
        if self.panel_size.is_minimized() {
            self.restore_top_pane();
        }
        if self.projects.search_active() {
            self.projects.end_search();
        }
    }

    fn set_project_mode(&mut self) {
        self.prepare_mode_switch();
        if self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
        self.mode = Mode::Project;
    }

    fn set_project_search_mode(&mut self) {
        if self.panel_size.is_minimized() {
            self.restore_top_pane();
        }
        if self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
        self.mode = Mode::ProjectSearch;
    }

    fn set_filter_mode(&mut self) {
        self.prepare_mode_switch();
        self.mode = Mode::Filter;
        self.tasks.set_visible(true);
        if !self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
        if self.tasks.filter_panel_editing() {
            self.tasks.filter_edit_done();
        }
    }

    fn set_filter_edit_mode(&mut self) {
        self.prepare_mode_switch();
        self.mode = Mode::FilterEdit;
        self.tasks.set_visible(true);
        if !self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
        if !self.tasks.filter_panel_editing() {
            self.tasks.filter_edit_begin();
        }
    }

    /// Enters the date picker. The picker owns the whole date edit, so the
    /// filter panel's own text-edit flag stays off.
    fn set_calendar_mode(&mut self) {
        self.prepare_mode_switch();
        self.mode = Mode::Calendar;
        self.tasks.set_visible(true);
        if !self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
    }

    /// Enters gantt mode, drawing the chart. Follows `set_filter_mode`: the
    /// mode and the thing it drives are turned on together.
    fn set_gantt_mode(&mut self) {
        self.prepare_mode_switch();
        self.mode = Mode::Gantt;
        self.tasks.set_visible(true);
        self.tasks.gantt_mut().set_visible(true);
        if self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
    }

    fn set_task_mode(&mut self) {
        if self.projects.search_active() {
            self.projects.end_search();
        }
        self.mode = Mode::Task;
        self.tasks.set_visible(true);
        if self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
    }

    fn adjust_top_pane_height(&mut self, delta: i16) {
        self.panel_size.resize(delta);
    }

    fn minimize_top_pane(&mut self) {
        self.panel_size.minimize();
    }

    fn maximize_top_pane(&mut self) {
        self.panel_size.maximize();
    }

    fn restore_top_pane(&mut self) {
        self.panel_size.restore();
    }

    fn task_data_needs_refresh(&self) -> bool {
        let targets = self.task_target_projects();
        let query_template = self.tasks.desired_task_query();
        self.task_data_receiver.is_none()
            && !self.tasks.can_serve_query_for_targets(&targets, &query_template)
    }

    fn ensure_task_data(&mut self) {
        if self.tasks.visible() && self.task_data_needs_refresh() {
            self.start_task_data_fetch();
        }
    }

    fn update_task_data_after_action(&mut self, task_targets_before: Option<Vec<String>>) {
        if let Some(before) = task_targets_before {
            let after = self.task_target_project_ids();
            if before != after
                && !matches!(
                    self.tasks.status(),
                    crate::app::task::TaskStatus::Idle
                )
            {
                if self.task_data_receiver.is_some() {
                    self.task_data_generation = self.task_data_generation.wrapping_add(1);
                }
                if self.tasks.visible() {
                    self.start_task_data_fetch();
                } else {
                    self.tasks.mark_out_of_date(
                        "selected projects changed; switch to task view to refresh",
                    );
                }
            }
        }

        if self.tasks.visible() && self.task_data_needs_refresh() {
            self.start_task_data_fetch();
        }
    }

    /// Handle text input when the filter panel is in edit mode.
    ///
    /// `action` is the label-navigation action the keymap resolved for this event,
    /// if any. On a labels field the action is applied; on any other field the key
    /// falls back to pushing its character. This lets label-navigation bindings be
    /// fully configurable while still typing normally on text and date fields.
    fn handle_filter_field_input(
        &mut self,
        event: crossterm::event::KeyEvent,
        action: Option<&Action>,
    ) -> Result<bool> {
        if !self.tasks.filter_panel_editing() {
            return Ok(false);
        }

        use crossterm::event::{KeyCode, KeyModifiers};
        use crate::app::task::TaskFieldFilterKind;

        let is_plain_char = !event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let is_labels = matches!(self.tasks.filter_selected_kind(), Some(TaskFieldFilterKind::Labels));

        if let KeyCode::Backspace = event.code {
            self.tasks.filter_pop_char();
            return Ok(true);
        }

        if is_labels {
            match action {
                Some(Action::FilterMoveLabelLeft) => { self.tasks.filter_move_label_left(); return Ok(true); }
                Some(Action::FilterMoveLabelRight) => { self.tasks.filter_move_label_right(); return Ok(true); }
                Some(Action::FilterCycleLabelDown) => { self.tasks.filter_cycle_label_value(1); return Ok(true); }
                Some(Action::FilterCycleLabelUp) => { self.tasks.filter_cycle_label_value(-1); return Ok(true); }
                Some(Action::FilterAddLabel) => { self.tasks.filter_add_label(); return Ok(true); }
                Some(Action::FilterDeleteLabel) => { self.tasks.filter_delete_label(); return Ok(true); }
                _ => {}
            }
        }

        if is_plain_char {
            if let KeyCode::Char(c) = event.code {
                self.tasks.filter_push_char(c);
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Move the caret in whichever filter editor is active.
    ///
    /// A date field being picked keeps its caret in the calendar, since that is
    /// what decides which end of a range the navigation keys rewrite; every other
    /// field keeps its own.
    fn move_filter_caret(&mut self, delta: i64) {
        if self.tasks.filter_calendar_open() {
            self.tasks.filter_calendar_move_caret(delta);
        } else {
            self.tasks.filter_move_caret(delta);
        }
    }

    /// Handle typed characters while the date picker is open.
    ///
    /// Typed text goes into the filter field the panel is showing, not into a
    /// buffer hidden in the overlay, and the table refilters as it lands. Only
    /// keys the keymap did not claim reach here, so the navigation letters stay
    /// navigation.
    fn handle_calendar_input(&mut self, event: crossterm::event::KeyEvent) -> Result<bool> {
        if !self.tasks.filter_calendar_open() {
            return Ok(false);
        }

        use crossterm::event::{KeyCode, KeyModifiers};

        match event.code {
            KeyCode::Backspace => {
                self.tasks.filter_calendar_pop_char();
                Ok(true)
            }
            KeyCode::Char(c)
                if !event
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.tasks.filter_calendar_push_char(c);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn handle_project_search_input(&mut self, event: crossterm::event::KeyEvent) -> Result<bool> {
        use crossterm::event::{KeyCode, KeyModifiers};

        match event.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.projects.end_search();
                self.set_project_mode();
                Ok(true)
            }
            KeyCode::Backspace => {
                self.projects.pop_search_char();
                Ok(true)
            }
            KeyCode::Char(c)
                if !event
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.projects.push_search_char(c.to_ascii_lowercase());
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub fn handle_action(
        &mut self,
        action: &Action,
        page_size: usize,
    ) -> Result<Option<AppCommand>> {
        if let Some(command) = action.as_app_command() {
            debug_log(&format!("app command: {command:?}"));
            return Ok(Some(command));
        }

        let task_targets_before = if !matches!(
            self.tasks.status(),
            crate::app::task::TaskStatus::Idle
        ) {
            Some(self.task_target_project_ids())
        } else {
            None
        };

        let mode = self.mode;
        debug_log(&format!(
            "handle_action: action={action} mode={:?} visible_tasks={}",
            mode,
            self.tasks.visible(),
        ));

        match action {
            Action::BeginFilterEdit => {
                // A date field is edited on the calendar rather than as raw
                // text, so it gets its own mode instead of the text buffer.
                if self.tasks.filter_calendar_begin() {
                    self.set_calendar_mode();
                } else {
                    self.tasks.filter_edit_begin();
                    self.set_filter_edit_mode();
                }
                return Ok(None);
            }
            Action::CalendarCommit => {
                self.tasks.filter_calendar_commit();
                self.set_filter_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::CalendarClose => {
                self.tasks.filter_calendar_close();
                self.set_filter_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::FilterCaretLeft => {
                self.move_filter_caret(-1);
                return Ok(None);
            }
            Action::FilterCaretRight => {
                self.move_filter_caret(1);
                return Ok(None);
            }
            Action::CalendarJumpToStart => {
                self.tasks.filter_calendar_jump_to_start();
                return Ok(None);
            }
            Action::CalendarJumpToEnd => {
                self.tasks.filter_calendar_jump_to_end();
                return Ok(None);
            }
            Action::CalendarClear => {
                self.tasks.filter_calendar_clear();
                self.set_filter_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::CalendarPrevDay => {
                self.tasks.filter_calendar_move_days(-1);
                return Ok(None);
            }
            Action::CalendarNextDay => {
                self.tasks.filter_calendar_move_days(1);
                return Ok(None);
            }
            Action::CalendarPrevMonth => {
                self.tasks.filter_calendar_move_months(-1);
                return Ok(None);
            }
            Action::CalendarNextMonth => {
                self.tasks.filter_calendar_move_months(1);
                return Ok(None);
            }
            Action::CalendarToday => {
                self.tasks.filter_calendar_today();
                return Ok(None);
            }
            Action::FilterDoneEditing => {
                self.tasks.filter_calendar_close();
                self.tasks.filter_edit_done();
                self.set_filter_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::FilterCancelEditing => {
                self.tasks.filter_calendar_close();
                self.tasks.filter_edit_done();
                self.tasks.toggle_filter_panel();
                self.set_task_mode();
                return Ok(None);
            }
            Action::SetProjectMode => {
                self.set_project_mode();
                return Ok(None);
            }
            Action::SetFilterMode => {
                self.set_filter_mode();
                return Ok(None);
            }
            Action::SetTaskMode => {
                self.set_task_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::SetGanttMode => {
                self.set_gantt_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::ToggleGantt => {
                self.tasks.gantt_mut().toggle_visible();
                // Turning the chart off leaves nothing for gantt mode to
                // drive, so the mode goes with it.
                if !self.tasks.gantt().visible() {
                    self.set_task_mode();
                }
                return Ok(None);
            }
            Action::GanttAddColumn => {
                self.tasks.gantt_add_column();
                return Ok(None);
            }
            Action::GanttRemoveColumn => {
                self.tasks.gantt_remove_column();
                return Ok(None);
            }
            Action::CycleGanttColorKey => {
                self.tasks.cycle_gantt_color_key();
                return Ok(None);
            }
            // Scrolling and zooming move a viewport over the rows already in
            // the table. None of these issues a request or changes a filter.
            Action::GanttScrollLeft => {
                self.tasks.gantt_scroll(false);
                return Ok(None);
            }
            Action::GanttScrollRight => {
                self.tasks.gantt_scroll(true);
                return Ok(None);
            }
            Action::GanttZoomIn => {
                self.tasks.gantt_zoom(true);
                return Ok(None);
            }
            Action::GanttZoomOut => {
                self.tasks.gantt_zoom(false);
                return Ok(None);
            }
            Action::GanttZoomFit => {
                self.tasks.gantt_fit();
                return Ok(None);
            }
            Action::GanttToday => {
                self.tasks.gantt_today();
                return Ok(None);
            }
            Action::GanttOpenOrder => {
                self.tasks.gantt_open_order();
                self.mode = Mode::GanttOrder;
                return Ok(None);
            }
            Action::GanttOrderMoveUp => {
                self.tasks.gantt_order_move(MoveTo::Up);
                return Ok(None);
            }
            Action::GanttOrderMoveDown => {
                self.tasks.gantt_order_move(MoveTo::Down);
                return Ok(None);
            }
            Action::GanttOrderMoveTop => {
                self.tasks.gantt_order_move(MoveTo::Top);
                return Ok(None);
            }
            Action::GanttOrderMoveBottom => {
                self.tasks.gantt_order_move(MoveTo::Bottom);
                return Ok(None);
            }
            Action::GanttOrderCommit => {
                self.tasks.gantt_mut().dialog_commit();
                self.mode = Mode::Gantt;
                self.persist_gantt_colors()?;
                return Ok(None);
            }
            Action::GanttOrderCancel => {
                self.tasks.gantt_mut().dialog_cancel();
                self.mode = Mode::Gantt;
                return Ok(None);
            }
            // The dialog is a list of its own, so the cursor keys drive it
            // rather than the task rows underneath. Same shape as the filter
            // panel's interception of the same two actions.
            Action::MoveUp if self.mode == Mode::GanttOrder => {
                self.tasks.gantt_mut().dialog_move_cursor(-1);
                return Ok(None);
            }
            Action::MoveDown if self.mode == Mode::GanttOrder => {
                self.tasks.gantt_mut().dialog_move_cursor(1);
                return Ok(None);
            }
            Action::ToggleTaskMode => {
                if self.mode == Mode::Task {
                    self.set_project_mode();
                } else {
                    self.set_task_mode();
                    self.ensure_task_data();
                }
                return Ok(None);
            }
            Action::ResizeTopPaneUp => {
                self.adjust_top_pane_height(2);
                return Ok(None);
            }
            Action::ResizeTopPaneDown => {
                self.adjust_top_pane_height(-2);
                return Ok(None);
            }
            Action::MinimizeTopPane => {
                if self.panel_size.is_minimized() {
                    self.restore_top_pane();
                } else {
                    self.minimize_top_pane();
                }
                self.set_task_mode();
                return Ok(None);
            }
            Action::MaximizeTopPane => {
                if self.panel_size.is_maximized() {
                    self.restore_top_pane();
                } else {
                    self.maximize_top_pane();
                }
                return Ok(None);
            }
            Action::RestoreTopPane => {
                self.restore_top_pane();
                return Ok(None);
            }
            Action::Open => {
                let url = if matches!(mode, Mode::Task) {
                    self.tasks.selected_task_url()
                } else if matches!(mode, Mode::Project | Mode::ProjectSearch) {
                    self.projects.selected_project().and_then(|p| match p.kind {
                        ProjectKind::Normal => Some(format!("https://app.asana.com/0/{}", p.id)),
                        ProjectKind::AssignedToMe => None,
                    })
                } else {
                    None
                };
                if let Some(url) = url {
                    return Ok(Some(AppCommand::OpenUrl(url)));
                }
                return Ok(None);
            }
            Action::ToggleTaskFilters => {
                if self.tasks.filter_panel_visible() {
                    self.tasks.toggle_filter_panel();
                    self.set_task_mode();
                } else {
                    self.set_filter_mode();
                }
                return Ok(None);
            }
            Action::ScrollLeft => {
                if self.tasks.visible() {
                    self.tasks.scroll_left();
                    return Ok(None);
                }
            }
            Action::ScrollRight => {
                if self.tasks.visible() {
                    self.tasks.scroll_right();
                    return Ok(None);
                }
            }
            Action::ToggleCompletedFilter if self.tasks.visible() => {
                let targets = self.task_target_projects();
                self.tasks.cycle_completed_filter_without_refresh();
                let query_template = self.tasks.desired_task_query();
                if self.tasks.can_serve_query_for_targets(&targets, &query_template) {
                    if self.task_data_receiver.is_some() {
                        self.task_data_generation = self.task_data_generation.wrapping_add(1);
                        self.task_data_receiver = None;
                    }
                    self.tasks.refresh_from_cache();
                } else {
                    self.start_task_data_fetch();
                }
                return Ok(None);
            }
            _ => {}
        }

        let task_action = action.is_task_view_action() && self.tasks.visible();

        let result = if matches!(mode, Mode::Project | Mode::ProjectSearch) && !task_action
        {
            self.projects.apply_action(action, page_size)
        } else {
            self.tasks.apply_action(action, page_size)
        };

        if matches!(action, Action::StartSearch) && matches!(mode, Mode::Project) {
            self.set_project_search_mode();
        } else if matches!(action, Action::ClearSearch)
            && matches!(mode, Mode::ProjectSearch)
        {
            self.set_project_mode();
        }

        if matches!(
            action,
            Action::ToggleStarredSelected | Action::ToggleHiddenSelected
        ) {
            self.persist_project_visibility()?;
        }

        self.update_task_data_after_action(task_targets_before);

        Ok(result)
    }

    pub fn handle_key_event(
        &mut self,
        keymap: &KeyMap,
        event: crossterm::event::KeyEvent,
        page_size: usize,
    ) -> Result<Option<AppCommand>> {
        if matches!(
            KeyBinding::from_crossterm_event(event),
            Some(KeyBinding::Ctrl('c'))
        ) {
            return Ok(Some(AppCommand::Quit));
        }

        self.poll_task_data();

        debug_log(&format!(
            "key event: {:?} {:?}",
            event.code, event.modifiers
        ));

        if let Some(binding) = KeyBinding::from_crossterm_event(event) {
            debug_log(&format!("resolved binding: {binding:?}"));
            if let Some(action) = keymap.action_for(&binding, self.mode).cloned() {
                debug_log(&format!("resolved action: {action}"));
                if action.is_label_filter_action() {
                    // Context-sensitive: navigate labels when a labels field is selected,
                    // otherwise fall back to pushing the character.
                    self.handle_filter_field_input(event, Some(&action))?;
                    return Ok(None);
                }
                return self.handle_action(&action, page_size);
            }
            debug_log("no action for binding");
        }

        if self.handle_calendar_input(event)? {
            return Ok(None);
        }

        if self.tasks.filter_panel_visible() {
            if self.handle_filter_field_input(event, None)? {
                return Ok(None);
            }
        }

        if self.projects.search_active() {
            if self.handle_project_search_input(event)? {
                return Ok(None);
            }
        }

        Ok(None)
    }

    pub fn poll_task_data(&mut self) {
        loop {
            let result = {
                let Some(receiver) = self.task_data_receiver.as_ref() else {
                    return;
                };
                receiver.try_recv()
            };

            match result {
                Ok(message) => {
                    if message.generation == self.task_data_generation {
                        if message.done {
                            self.tasks.finish_loading_targets();
                            self.task_data_receiver = None;
                            break;
                        }

                        let Some(project_id) = message.project_id.as_deref() else {
                            continue;
                        };
                        match message.result {
                            Ok(dataset) => {
                                self.tasks
                                    .ingest_loaded_project(project_id, message.query.clone(), dataset)
                            }
                            Err(err) => {
                                debug_log(&format!("task data error: {err}"));
                                self.tasks.set_error(err.to_string());
                                self.task_data_receiver = None;
                                break;
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.task_data_receiver = None;
                    break;
                }
            }
        }
    }

    fn persist_project_visibility(&mut self) -> Result<()> {
        self.config.project_visibility = self.projects.project_visibility_config();
        self.config.save_to_source_path()?;
        Ok(())
    }

    /// Writes the chart's colour choices back to the config file.
    ///
    /// Only the dimension and its value order: hand-ordering a team is real
    /// work and losing it on exit would be worse than the cost of a write.
    /// Visibility, the column count, and the timeline window are view state of
    /// the same kind as sort and grouping, which have never persisted.
    fn persist_gantt_colors(&mut self) -> Result<()> {
        self.config.gantt.color_by = self.tasks.gantt().color_key().to_string();
        self.config.gantt.order = self.tasks.gantt().orders().clone();
        self.config.save_to_source_path()?;
        Ok(())
    }

    fn task_target_projects(&self) -> Vec<crate::domain::Project> {
        let selected_projects = self.projects.selected_projects();

        if !selected_projects.is_empty() {
            return selected_projects;
        }

        self.projects
            .selected_project()
            .cloned()
            .into_iter()
            .collect()
    }

    fn task_target_project_ids(&self) -> Vec<String> {
        self.task_target_projects()
            .into_iter()
            .map(|project| project.id)
            .collect()
    }

    fn start_task_data_fetch(&mut self) {
        let targets = self.task_target_projects();
        let query_template = self.tasks.desired_task_query();
        let projects_to_load = self.tasks.projects_requiring_load(&targets, &query_template);
        debug_log(&format!(
            "start_task_data_fetch: visible={} scope={:?} targets={} data={}",
            self.tasks.visible(),
            query_template.scope,
            targets
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>()
                .join(","),
            projects_to_load
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));

        if targets.is_empty() {
            self.tasks
                .finish_loading_dataset(crate::app::task::TaskDataset::default());
            return;
        }

        if projects_to_load.is_empty() {
            self.tasks.begin_loading(&targets);
            self.tasks.finish_loading_targets();
            return;
        }

        self.task_data_generation = self.task_data_generation.wrapping_add(1);
        let generation = self.task_data_generation;
        self.tasks.begin_loading(&targets);

        let (sender, receiver) = mpsc::channel();
        let client = self.client.clone();
        self.task_data_receiver = Some(receiver);

        thread::spawn(move || {
            for project in projects_to_load {
                let query = TaskQuery { target: TaskTarget::for_project(&project), ..query_template.clone() };
                let result = crate::app::task::TaskState::build_dataset_for_projects(
                    &client,
                    &[project.clone()],
                    &query,
                );
                let _ = sender.send(TaskDataMessage {
                    generation,
                    project_id: Some(project.id.clone()),
                    query: query.clone(),
                    result,
                    done: false,
                });
            }

            let _ = sender.send(TaskDataMessage {
                generation,
                project_id: None,
                query: query_template,
                result: Ok(crate::app::task::TaskDataset::default()),
                done: true,
            });
        });
    }
}

pub fn debug_log(message: &str) {
    if std::env::var_os("TUISANA_DEBUG").is_some() {
        eprintln!("[tuisana] {message}");
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        asana::{
            dto::{
                CustomFieldDto, CustomFieldValueDto, ProjectCustomFieldSettingDto, TaskDto,
                TaskMembershipDto, TaskMembershipProjectDto, TaskMembershipSectionDto, UserDto,
            },
            fake::FakeAsanaClient,
        },
        config::{Config, ProjectVisibilityConfig},
        domain::{GanttColorKey, Project},
        input::{Action, KeyBinding},
    };

    use super::App;
    use crate::config::Mode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn app_can_be_created_with_fake_backend_and_load_projects() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);

        app.load_projects().expect("projects load");

        assert_eq!(app.projects.items().len(), 1);
        assert_eq!(app.projects.items()[0].name, "Inbox");
        assert_eq!(app.projects.selected_index(), Some(0));
    }

    #[test]
    fn app_handles_project_list_actions() {
        let client = FakeAsanaClient::new(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
        ]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        assert_eq!(
            app.handle_action(&crate::input::Action::MoveDown, 10)
                .expect("action ok"),
            None
        );
        assert_eq!(app.projects.selected_index(), Some(1));
        assert_eq!(
            app.handle_action(&crate::input::Action::Quit, 10)
                .expect("action ok"),
            Some(crate::input::AppCommand::Quit)
        );
    }

    #[test]
    fn app_applies_project_visibility_config_from_config() {
        let client = FakeAsanaClient::new(vec![
            Project::new("1", "Inbox", false),
            Project::new("2", "Backlog", false),
        ]);
        let mut config = Config::default();
        config.project_visibility = vec![ProjectVisibilityConfig {
            gid: "2".to_string(),
            starred: true,
            hidden: true,
        }];
        let mut app = App::new(config, client);

        app.load_projects().expect("projects load");

        assert_eq!(app.projects.items().len(), 2);
        assert_eq!(app.projects.items()[0].id, "2");
        assert_eq!(app.projects.items()[1].id, "1");
        assert_eq!(app.projects.hidden_count(), 2);
    }

    #[test]
    fn app_uses_default_bindings_when_config_does_not_override_them() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [[bind]]
                key = "x"
                command = "quit"
            "#,
        )
        .expect("config parses");
        let app = App::new(config, FakeAsanaClient::default());

        let keymap = app.keymap().expect("keymap builds");

        assert_eq!(
            keymap.action_for(&KeyBinding::Char('x'), Mode::Any),
            Some(&Action::Quit)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('j'), Mode::Any),
            Some(&Action::MoveDown)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('t'), Mode::Project),
            Some(&Action::SetTaskMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('f'), Mode::Project),
            Some(&Action::SetFilterMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('/'), Mode::Project),
            Some(&Action::StartSearch)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('p'), Mode::Any),
            Some(&Action::SetProjectMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('m'), Mode::Any),
            Some(&Action::ToggleTaskMode)
        );
    }

    #[test]
    fn braces_toggle_the_top_pane_between_restored_minimized_and_maximized() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);

        app.handle_action(&Action::MinimizeTopPane, 10)
            .expect("minimize top pane");

        assert!(app.panel_size().is_minimized());
        assert_eq!(app.panel_size().actual_height(20, 6), 0);
        assert_eq!(app.mode(), Mode::Task);

        app.handle_action(&Action::MinimizeTopPane, 10)
            .expect("un-minimize top pane");

        assert!(app.panel_size().is_normal());

        app.handle_action(&Action::MaximizeTopPane, 10)
            .expect("maximize top pane");

        assert!(app.panel_size().is_maximized());
        assert_eq!(app.panel_size().actual_height(20, 6), 20);
        assert_eq!(app.mode(), Mode::Task);

        app.handle_action(&Action::MaximizeTopPane, 10)
            .expect("un-maximize top pane");

        assert!(app.panel_size().is_normal());
    }

    #[test]
    fn switching_to_project_or_filter_mode_restores_a_minimized_top_pane() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);

        app.handle_action(&Action::MinimizeTopPane, 10)
            .expect("minimize top pane");
        assert!(app.panel_size().is_minimized());
        assert_eq!(app.mode(), Mode::Task);

        app.handle_action(&Action::SetProjectMode, 10)
            .expect("switch to project mode");
        assert!(app.panel_size().is_normal());
        assert_eq!(app.mode(), Mode::Project);

        app.handle_action(&Action::MinimizeTopPane, 10)
            .expect("minimize top pane again");
        assert!(app.panel_size().is_minimized());
        assert_eq!(app.mode(), Mode::Task);

        app.handle_action(&Action::SetFilterMode, 10)
            .expect("switch to filter mode");
        assert!(app.panel_size().is_normal());
        assert_eq!(app.mode(), Mode::Filter);
    }

    /// An app with one selected project holding two dated tasks.
    fn gantt_app() -> App<FakeAsanaClient> {
        fn task(gid: &str, name: &str, due: &str) -> TaskDto {
            TaskDto {
                gid: gid.to_string(),
                name: name.to_string(),
                completed: false,
                modified_at: None,
                due_on: Some(due.to_string()),
                start_on: Some("2026-06-01".to_string()),
                assignee: Some(UserDto {
                    gid: format!("u-{gid}"),
                    name: Some(format!("Owner {gid}")),
                    display_name: Some(format!("Owner {gid}")),
                }),
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

        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]).with_tasks(
            "1",
            vec![task("t1", "Ship it", "2026-07-20"), task("t2", "Pack it", "2026-08-20")],
        );
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.handle_action(&Action::ToggleSelection, 10)
            .expect("select the project");
        app
    }

    fn press(app: &mut App<FakeAsanaClient>, code: KeyCode) {
        let keymap = app.keymap().expect("bindings parse");
        app.handle_key_event(&keymap, KeyEvent::new(code, KeyModifiers::NONE), 10)
            .expect("key handled");
    }

    fn settle(app: &mut App<FakeAsanaClient>) {
        for _ in 0..100 {
            app.poll_task_data();
            if !matches!(app.tasks.status(), crate::app::task::TaskStatus::Loading) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn g_draws_the_chart_and_takes_its_controls() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('g'));

        assert_eq!(app.mode(), Mode::Gantt);
        assert!(app.tasks.gantt().visible());
        assert!(app.tasks.visible());
    }

    #[test]
    fn esc_leaves_gantt_mode_with_the_chart_still_drawn() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Esc);

        assert_eq!(app.mode(), Mode::Task);
        assert!(app.tasks.gantt().visible(), "esc is not a way to hide it");
    }

    #[test]
    fn g_again_hides_the_chart_and_the_mode_goes_with_it() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('g'));

        assert!(!app.tasks.gantt().visible());
        assert_eq!(app.mode(), Mode::Task, "the mode has nothing left to drive");
    }

    #[test]
    fn the_cursor_still_moves_through_tasks_while_in_gantt_mode() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        let before = app.tasks.selected_index();
        press(&mut app, KeyCode::Char('j'));

        assert_ne!(
            app.tasks.selected_index(),
            before,
            "j/k are global bindings and gantt mode falls back to them"
        );
    }

    #[test]
    fn angle_brackets_change_how_many_table_columns_are_visible() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));

        let total = app.tasks.table().columns.len();
        let before = app.tasks.gantt().columns(total);
        press(&mut app, KeyCode::Char('>'));
        assert_eq!(app.tasks.gantt().columns(total), before + 1);

        press(&mut app, KeyCode::Char('<'));
        assert_eq!(app.tasks.gantt().columns(total), before);
    }

    #[test]
    fn c_cycles_the_colour_key_and_wraps() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));

        assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::Assignee);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::Section);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::State);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::Assignee);
    }

    #[test]
    fn c_still_cycles_the_completed_filter_in_task_mode() {
        // `c` is bound in both modes; the mode-specific binding is what makes
        // that safe, so a regression here would be silent.
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        let before = app.tasks.task_settings().filter.completed;
        press(&mut app, KeyCode::Char('c'));

        assert_ne!(app.tasks.task_settings().filter.completed, before);
        assert_eq!(app.tasks.gantt().color_key(), &GanttColorKey::Assignee);
    }

    /// A config backed by a real file, so save_to_source_path has somewhere
    /// to write.
    fn config_on_disk() -> (Config, std::path::PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tuisana-gantt-{unique}.toml"));
        std::fs::write(
            &path,
            "[header]\ntype = \"tuisana\"\nversion = 1.0\n",
        )
        .expect("write config");
        (Config::load_from_path(&path).expect("config loads"), path)
    }

    #[test]
    fn the_dialog_opens_over_the_current_dimensions_values() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.mode(), Mode::GanttOrder);
        let dialog = app.tasks.gantt().dialog().expect("the dialog is open");
        assert_eq!(dialog.key(), &GanttColorKey::Assignee);
        assert_eq!(dialog.entries().len(), 2, "two assignees, none unassigned");
    }

    #[test]
    fn j_and_k_drive_the_dialog_rather_than_the_task_rows() {
        let mut app = gantt_app();

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        let row = app.tasks.selected_index();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('j'));

        assert_eq!(app.tasks.gantt().dialog().expect("open").selected(), 1);
        assert_eq!(app.tasks.selected_index(), row, "the table did not move");
    }

    #[test]
    fn saving_the_dialog_writes_the_order_to_the_config_file() {
        let (config, path) = config_on_disk();
        let mut app = gantt_app();
        app.config = config;

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('b')); // send the first value to the bottom
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.mode(), Mode::Gantt);
        let written = std::fs::read_to_string(&path).expect("config was written");
        let reloaded = Config::from_toml_str(&written).expect("it reparses");
        assert_eq!(
            reloaded.gantt.order_for(&GanttColorKey::Assignee),
            ["Owner t2".to_string(), "Owner t1".to_string()],
        );
        assert_eq!(reloaded.gantt.color_by, "assignee");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cancelling_the_dialog_writes_nothing() {
        let (config, path) = config_on_disk();
        let before = std::fs::read_to_string(&path).expect("config exists");
        let mut app = gantt_app();
        app.config = config;

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Esc);

        assert_eq!(app.mode(), Mode::Gantt);
        assert!(!app.tasks.gantt().dialog_open());
        assert_eq!(
            std::fs::read_to_string(&path).expect("config exists"),
            before
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_chart_visibility_and_column_count_are_not_persisted() {
        // View state of the same kind as sort and grouping, which have never
        // persisted. Only the colour choices are the user's own work.
        let (config, path) = config_on_disk();
        let mut app = gantt_app();
        app.config = config;

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('>'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);

        let written = std::fs::read_to_string(&path).expect("config was written");
        let reloaded = Config::from_toml_str(&written).expect("it reparses");
        assert!(!reloaded.gantt.visible);
        assert_eq!(reloaded.gantt.columns, 2, "the default, not the session's 3");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn changing_project_visibility_after_using_the_chart_writes_no_gantt_table() {
        // save_to_source_path rewrites the whole file, so an untouched section
        // must not start appearing in the user's config just because the chart
        // was opened.
        let (config, path) = config_on_disk();
        let mut app = gantt_app();
        app.config = config;

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('>'));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('p'));
        // Hiding rather than starring: the fixture project starts starred, so
        // a star toggle would return it to the default and write nothing.
        press(&mut app, KeyCode::Char('h'));

        let written = std::fs::read_to_string(&path).expect("config was written");
        assert!(
            written.contains("[[project]]"),
            "the visibility change was saved: {written}"
        );
        assert!(!written.contains("[gantt]"), "{written}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_saved_colour_order_is_in_effect_after_a_restart() {
        let (config, path) = config_on_disk();
        let mut app = gantt_app();
        app.config = config;

        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Enter);

        let reloaded = Config::load_from_path(&path).expect("config reloads");
        let restarted = App::new(reloaded, FakeAsanaClient::new(Vec::new()));

        assert_eq!(
            restarted.tasks.gantt().order(),
            ["Owner t2".to_string(), "Owner t1".to_string()],
            "no hand-editing needed between sessions"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn config_can_start_the_session_with_the_chart_already_drawn() {
        let mut config = Config::default();
        config.gantt.visible = true;
        let app = App::new(config, FakeAsanaClient::new(Vec::new()));

        assert!(app.tasks.gantt().visible());
    }

    #[test]
    fn pressing_t_in_project_mode_switches_to_task_mode() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);
        assert_eq!(app.mode(), Mode::Project);

        app.handle_action(&Action::SetTaskMode, 10)
            .expect("set task mode");

        assert_eq!(app.mode(), Mode::Task);
        assert!(app.tasks.visible());
    }

    #[test]
    fn r_still_triggers_refresh() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);
        let keymap = app.keymap().expect("keymap builds");

        let command = app
            .handle_key_event(
                &keymap,
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
                10,
            )
            .expect("refresh command");

        assert_eq!(command, Some(crate::input::AppCommand::Refresh));
    }

    /// Once a project's tasks are loaded, the cache reports the query as
    /// already covered — so a naive refresh that only checks
    /// `task_data_needs_refresh` would see nothing to do and skip fetching
    /// entirely. `App::refresh` has to invalidate that cache itself.
    #[test]
    fn refresh_reloads_tasks_even_when_the_cache_already_covers_the_query() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.handle_action(&Action::SelectAllVisible, 10)
            .expect("select every project");
        app.tasks.set_visible(true);
        app.request_task_data().expect("tasks load");
        settle(&mut app);

        let targets = app.task_target_projects();
        let query = app.tasks.desired_task_query();
        assert!(
            app.tasks.can_serve_query_for_targets(&targets, &query),
            "cache should already cover the loaded selection"
        );

        app.refresh().expect("refresh");

        assert_eq!(
            app.tasks.status(),
            &crate::app::task::TaskStatus::Loading,
            "refresh must force a fetch instead of trusting the cache"
        );
    }

    #[test]
    fn app_marks_tasks_out_of_date_when_project_selection_changes() {
        let client = FakeAsanaClient::new(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
        ]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        let table = crate::app::task::TaskState::build_table_for_projects(
            &app.client,
            &[Project::new("1", "Inbox", true)],
        )
        .expect("table builds");
        app.tasks.begin_loading(&[Project::new("1", "Inbox", true)]);
        app.tasks.finish_loading(table);

        assert_eq!(
            app.tasks.status(),
            &crate::app::task::TaskStatus::Empty
        );

        app.handle_action(&Action::MoveDown, 10).expect("move down");
        app.handle_action(&Action::ToggleSelection, 10)
            .expect("toggle selection");

        assert!(matches!(
            app.tasks.status(),
            crate::app::task::TaskStatus::OutOfDate(message)
                if message.contains("switch to task view")
        ));
    }

    #[test]
    fn page_navigation_uses_the_task_view_when_it_is_visible() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)])
            .with_sections(
                "1",
                vec![crate::asana::dto::SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_custom_field_settings(
                "1",
                vec![ProjectCustomFieldSettingDto {
                    gid: "cfs1".to_string(),
                    custom_field: CustomFieldDto {
                        gid: "cf1".to_string(),
                        name: "Priority".to_string(),
                    },
                }],
            )
            .with_tasks(
                "1",
                vec![
                    TaskDto {
                        gid: "t1".to_string(),
                        name: "Task 1".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
                        assignee: Some(UserDto {
                            gid: "user-1".to_string(),
                            name: Some("Alex".to_string()),
                            display_name: Some("Alex".to_string()),
                        }),
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
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("High".to_string()),
                            enum_value: None,
                        }],
                    },
                    TaskDto {
                        gid: "t2".to_string(),
                        name: "Task 2".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
                        assignee: Some(UserDto {
                            gid: "user-1".to_string(),
                            name: Some("Alex".to_string()),
                            display_name: Some("Alex".to_string()),
                        }),
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
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("High".to_string()),
                            enum_value: None,
                        }],
                    },
                    TaskDto {
                        gid: "t3".to_string(),
                        name: "Task 3".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
                        assignee: Some(UserDto {
                            gid: "user-1".to_string(),
                            name: Some("Alex".to_string()),
                            display_name: Some("Alex".to_string()),
                        }),
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
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("High".to_string()),
                            enum_value: None,
                        }],
                    },
                    TaskDto {
                        gid: "t4".to_string(),
                        name: "Task 4".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
                        assignee: Some(UserDto {
                            gid: "user-1".to_string(),
                            name: Some("Alex".to_string()),
                            display_name: Some("Alex".to_string()),
                        }),
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
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("High".to_string()),
                            enum_value: None,
                        }],
                    },
                ],
            );
        let table = crate::app::task::TaskState::build_table_for_projects(
            &client,
            &[Project::new("1", "Inbox", true)],
        )
        .expect("tasks load");
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.tasks.begin_loading(&[Project::new("1", "Inbox", true)]);
        app.tasks.finish_loading(table);
        app.tasks.set_visible(true);

        app.handle_action(&Action::PageDown, 2).expect("page down");
        assert_eq!(app.tasks.selected_index(), Some(6));

        app.handle_action(&Action::PageUp, 2).expect("page up");
        assert_eq!(app.tasks.selected_index(), Some(4));
    }

    #[test]
    fn task_view_bindings_override_project_bindings_when_tasks_are_visible() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [[bind]]
                key = "c"
                command = "clear_selection"
            "#,
        )
        .expect("config parses");

        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)])
            .with_sections(
                "1",
                vec![crate::asana::dto::SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "1",
                vec![
                    TaskDto {
                        gid: "t1".to_string(),
                        name: "Open task".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
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
                    },
                    TaskDto {
                        gid: "t2".to_string(),
                        name: "Closed task".to_string(),
                        completed: true,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
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
                    },
                ],
            );

        let mut app = App::new(config, client);
        app.load_projects().expect("projects load");
        app.tasks.set_completed_filter(None);
        app.tasks
            .load_task_dataset_for_projects(&app.client, &[Project::new("1", "Inbox", true)])
            .expect("tasks load");
        app.tasks.set_visible(true);

        assert_eq!(app.tasks.table().task_count(), 2);
        app.handle_action(&Action::ToggleCompletedFilter, 10)
            .expect("task filter action");
        assert_eq!(app.tasks.table().task_count(), 1);
        assert!(app.tasks.filter_summary().contains("comp open"));
    }

    #[test]
    fn toggle_task_filters_closes_the_panel_when_it_is_already_visible() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.tasks.set_visible(true);
        app.tasks.toggle_filter_panel();
        assert!(app.tasks.filter_panel_visible());

        app.handle_action(&Action::ToggleTaskFilters, 10)
            .expect("toggle filters");

        assert!(!app.tasks.filter_panel_visible());
    }

    #[test]
    fn task_filter_panel_separates_browsing_from_editing_and_hides_without_losing_filters() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)])
            .with_sections(
                "1",
                vec![crate::asana::dto::SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "1",
                vec![
                    TaskDto {
                        gid: "t1".to_string(),
                        name: "Open task".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: Some("2026-05-28".to_string()),
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
                    },
                    TaskDto {
                        gid: "t2".to_string(),
                        name: "Another task".to_string(),
                        completed: false,
                        modified_at: None,
                        due_on: Some("2026-06-02".to_string()),
                        start_on: Some("2026-05-29".to_string()),
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
                    },
                ],
            );

        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.tasks
            .load_task_dataset_for_projects(&app.client, &[Project::new("1", "Inbox", true)])
            .expect("tasks load");
        app.tasks.set_visible(true);

        let keymap = app.keymap().expect("keymap builds");

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
            10,
        )
        .expect("open filter panel");
        assert!(app.tasks.filter_panel_visible());
        assert!(!app.tasks.filter_panel_editing());

        let original_kind = app
            .tasks
            .filter_panel_entries()
            .into_iter()
            .find(|row| row.selected)
            .map(|row| row.kind)
            .expect("selected filter exists");

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
            10,
        )
        .expect("cycle string mode");
        let cycled_kind = app
            .tasks
            .filter_panel_entries()
            .into_iter()
            .find(|row| row.selected)
            .map(|row| row.kind)
            .expect("selected filter exists");
        assert_ne!(original_kind, cycled_kind);

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            10,
        )
        .expect("move filter selection down");
        assert_eq!(
            app.tasks
                .filter_panel_entries()
                .iter()
                .position(|row| row.selected),
            Some(1)
        );

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            2,
        )
        .expect("page filter selection down");
        assert_eq!(
            app.tasks
                .filter_panel_entries()
                .iter()
                .position(|row| row.selected),
            Some(3)
        );

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            2,
        )
        .expect("page filter selection up");
        assert_eq!(
            app.tasks
                .filter_panel_entries()
                .iter()
                .position(|row| row.selected),
            Some(1)
        );

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
            10,
        )
        .expect("move filter selection up");
        assert_eq!(
            app.tasks
                .filter_panel_entries()
                .iter()
                .position(|row| row.selected),
            Some(0)
        );

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            10,
        )
        .expect("enter edit mode");
        assert!(app.tasks.filter_panel_editing());

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('O'), KeyModifiers::NONE),
            10,
        )
        .expect("type into the selected filter");
        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
            10,
        )
        .expect("type into the selected filter");
        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
            10,
        )
        .expect("type into the selected filter");
        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
            10,
        )
        .expect("type into the selected filter");
        assert_eq!(
            app.tasks
                .filter_panel_entries()
                .into_iter()
                .find(|row| row.selected)
                .map(|row| row.query),
            Some("Open".to_string())
        );
        assert_eq!(app.tasks.table().task_count(), 1);

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            10,
        )
        .expect("leave edit mode");
        assert!(!app.tasks.filter_panel_editing());

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
            10,
        )
        .expect("hide filter panel");

        assert!(!app.tasks.filter_panel_visible());
        assert!(app.tasks.filter_summary().contains("filters 1"));
        assert_eq!(app.tasks.table().task_count(), 1);
    }

    #[test]
    fn load_projects_adds_assigned_to_me_row_only_when_the_client_resolves_a_current_user() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client.clone());
        app.load_projects().expect("projects load");
        assert_eq!(app.projects.items().len(), 1);

        let client = client.with_current_user_gid("user_1");
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        assert_eq!(app.projects.items()[0].id, "user_1");
        assert_eq!(app.projects.items()[0].name, "No Project (Assigned to Me)");
    }

    #[test]
    fn opening_the_assigned_to_me_row_does_not_produce_an_asana_url() {
        let client = FakeAsanaClient::new(vec![]).with_current_user_gid("user_1");
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        let command = app.handle_action(&Action::Open, 10).expect("open action ok");

        assert_eq!(command, None);
    }

    #[test]
    fn selecting_assigned_to_me_loads_tasks_via_the_assignee_scoped_query() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)])
            .with_assigned_to_me_tasks(vec![TaskDto {
                gid: "t1".to_string(),
                name: "My task".to_string(),
                completed: false,
                modified_at: None,
                due_on: None,
                start_on: None,
                assignee: None,
                num_subtasks: 0,
                memberships: vec![],
                parent: None,
                custom_fields: vec![],
            }]);

        let mut app = App::new(Config::default(), client);
        app.tasks
            .load_task_dataset_for_projects(&app.client, &[Project::assigned_to_me("user_1")])
            .expect("tasks load");

        assert_eq!(app.tasks.table().task_count(), 1);
    }

    /// The project list pins starred projects above the rest, so the task
    /// table's project groups have to lead with them too — otherwise the two
    /// panes disagree about the order and the eye has to hunt for the group.
    #[test]
    fn task_project_groups_follow_the_project_list_order() {
        fn task(gid: &str, name: &str, project_gid: &str, project: &str) -> TaskDto {
            TaskDto {
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
                        gid: project_gid.to_string(),
                        name: project.to_string(),
                    },
                    section: None,
                }],
                parent: None,
                custom_fields: vec![],
            }
        }

        let client = FakeAsanaClient::new(vec![
            Project::new("pa", "Alpha", false),
            Project::new("pz", "Zeta", true),
        ])
        .with_tasks("pa", vec![task("t1", "Task in Alpha", "pa", "Alpha")])
        .with_tasks("pz", vec![task("t2", "Task in Zeta", "pz", "Zeta")]);

        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.handle_action(&Action::SelectAllVisible, 10)
            .expect("select every project");
        app.tasks.set_visible(true);
        app.request_task_data().expect("tasks load");
        settle(&mut app);

        let headers = app
            .tasks
            .table()
            .rows
            .iter()
            .filter(|row| row.kind == crate::domain::TaskRowKind::ProjectHeader)
            .map(|row| row.cells[0].clone())
            .collect::<Vec<_>>();
        assert_eq!(
            headers,
            vec!["Zeta".to_string(), "Alpha".to_string()],
            "starred Zeta leads the project list, so it leads the table too"
        );
    }
}
