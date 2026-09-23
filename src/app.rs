use crate::{
    asana::{dto::TaskDto, AsanaClient, TaskQuery, TaskTarget},
    config::{Config, Mode, NamedFilterSet},
    domain::{Project, ProjectKind, TaskEdit},
    error::Result,
    input::{Action, AppCommand, KeyBinding, KeyMap},
};

use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

pub mod calendar;
pub mod gantt;
pub mod project_list;
pub mod task;
pub mod task_edit;
pub mod text_edit;

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
    /// Results from in-flight task writes.
    ///
    /// A permanent pair rather than one receiver per batch: `d` on twelve
    /// tasks is twelve requests, and the next edit must not have to wait for
    /// them. Each message names its task, so no generation counter is needed —
    /// a late reply to a superseded edit is reconciled by gid, not discarded.
    task_edit_events: (Sender<TaskEditMessage>, Receiver<TaskEditMessage>),
    /// Writes sent, still outstanding, and failed in the current burst.
    ///
    /// Counted rather than reported one by one: twelve failures is one border
    /// chip, `could not update 3 of 12: …`, not three that overwrite each
    /// other. Reset once the last reply of a burst has landed.
    task_edits_sent: usize,
    task_edits_outstanding: usize,
    task_edit_failures: usize,
    /// The last write error, which is what the chip quotes.
    task_edit_error: Option<String>,
    /// The logged-in user's gid, for resolving `me` in an assignee edit.
    current_user_gid: Option<String>,
}

/// One finished write, on its way back to the main thread.
struct TaskEditMessage {
    edit: TaskEdit,
    result: Result<TaskDto>,
}

/// One of the shared text motions, for [`App::move_text_caret`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextMotion {
    WordBack,
    WordForward,
    Start,
    End,
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
            task_edit_events: mpsc::channel(),
            task_edits_sent: 0,
            task_edits_outstanding: 0,
            task_edit_failures: 0,
            task_edit_error: None,
            current_user_gid: None,
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
            Ok(gid) => {
                self.current_user_gid = Some(gid.clone());
                Some(Project::assigned_to_me(gid))
            }
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

    /// Enters the mode the open cell editor reads its keys in.
    ///
    /// A date cell is edited on the calendar, which already owns a mode and a
    /// full set of keys; everything else types.
    fn set_task_edit_mode(&mut self) {
        self.mode = match self.tasks.cell_edit_owns_calendar() {
            true => Mode::Calendar,
            false => Mode::TaskEdit,
        };
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

    /// Handle typed characters while a task cell is being edited.
    ///
    /// Modelled on `handle_filter_field_input`, and tried before it: the cell
    /// editor owns every key the keymap did not claim, so an unbound letter
    /// types rather than falling through to a task-mode binding.
    fn handle_task_edit_input(&mut self, event: crossterm::event::KeyEvent) -> Result<bool> {
        use crossterm::event::{KeyCode, KeyModifiers};

        if !self.tasks.cell_edit_open() {
            return Ok(false);
        }
        // A value picker holds no text, so there is nothing for a character
        // to go into.
        if self.tasks.cell_edit_is_options() {
            return Ok(false);
        }

        match event.code {
            KeyCode::Backspace => {
                self.tasks.cell_edit_pop_char();
                Ok(true)
            }
            KeyCode::Char(c)
                if !event
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.tasks.cell_edit_push_char(c);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Move the caret in whichever filter editor is active.
    ///
    /// A date field being picked keeps its caret in the calendar, since that is
    /// what decides which end of a range the navigation keys rewrite; every other
    /// field keeps its own.
    fn move_filter_caret(&mut self, delta: i64) {
        if self.tasks.calendar_open() {
            self.tasks.filter_calendar_move_caret(delta);
        } else if self.tasks.cell_edit_open() {
            self.tasks.cell_edit_move_caret(delta);
        } else {
            self.tasks.filter_move_caret(delta);
        }
    }

    /// Runs one of the shared text motions on whichever buffer is being typed
    /// into.
    ///
    /// One implementation for both panes, because there is one buffer type
    /// behind them.
    fn move_text_caret(&mut self, motion: TextMotion) {
        if self.tasks.cell_edit_open() {
            match motion {
                TextMotion::WordBack => self.tasks.cell_edit_move_word(-1),
                TextMotion::WordForward => self.tasks.cell_edit_move_word(1),
                TextMotion::Start => self.tasks.cell_edit_jump_start(),
                TextMotion::End => self.tasks.cell_edit_jump_end(),
            }
            return;
        }
        match motion {
            TextMotion::WordBack => self.tasks.filter_move_word(-1),
            TextMotion::WordForward => self.tasks.filter_move_word(1),
            TextMotion::Start => self.tasks.filter_caret_to_start(),
            TextMotion::End => self.tasks.filter_caret_to_end(),
        }
    }

    /// Handle typed characters while the date picker is open.
    ///
    /// Typed text goes into the filter field the panel is showing, not into a
    /// buffer hidden in the overlay, and the table refilters as it lands. Only
    /// keys the keymap did not claim reach here, so the navigation letters stay
    /// navigation.
    fn handle_calendar_input(&mut self, event: crossterm::event::KeyEvent) -> Result<bool> {
        if !self.tasks.calendar_open() {
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

    /// Handle the sidebar's one-line prompt: typing a name for `w`, or
    /// answering the `y`/`n` confirmation for `d`.
    ///
    /// Read outside the keymap, like project search, so an unbound letter
    /// types instead of firing `b` or `d`. One mode covers both prompts: the
    /// handler already has the prompt in hand to know which it is.
    fn handle_filter_set_name_input(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<bool> {
        use crate::app::task::SidebarPrompt;
        use crossterm::event::{KeyCode, KeyModifiers};

        let Some(prompt) = self.tasks.filter_set_prompt().cloned() else {
            // The mode outlived its prompt, which nothing should do; leaving
            // it would swallow every key from here on.
            self.set_filter_mode();
            return Ok(false);
        };

        let typed = !event
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

        match prompt {
            SidebarPrompt::Save { .. } => match event.code {
                KeyCode::Enter => {
                    self.commit_filter_set_save();
                    Ok(true)
                }
                KeyCode::Esc => {
                    self.tasks.filter_set_prompt_cancel();
                    self.set_filter_mode();
                    Ok(true)
                }
                KeyCode::Backspace => {
                    self.tasks.filter_set_prompt_pop_char();
                    Ok(true)
                }
                KeyCode::Left => {
                    self.tasks.filter_set_prompt_move_caret(-1);
                    Ok(true)
                }
                KeyCode::Right => {
                    self.tasks.filter_set_prompt_move_caret(1);
                    Ok(true)
                }
                KeyCode::Char(c) if typed => {
                    self.tasks.filter_set_prompt_push_char(c);
                    Ok(true)
                }
                _ => Ok(false),
            },
            SidebarPrompt::ConfirmDelete { .. } | SidebarPrompt::ConfirmLoad { .. } => {
                match event.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') if typed => {
                        match prompt {
                            SidebarPrompt::ConfirmLoad { name } => {
                                self.commit_filter_set_load(&name)
                            }
                            _ => self.commit_filter_set_delete(),
                        }
                        Ok(true)
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') if typed => {
                        self.tasks.filter_set_prompt_cancel();
                        self.set_filter_mode();
                        Ok(true)
                    }
                    KeyCode::Esc => {
                        self.tasks.filter_set_prompt_cancel();
                        self.set_filter_mode();
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
        }
    }

    /// Commits the open cell editor, or says why it cannot be committed.
    ///
    /// A refusal leaves the editor open: the value that could not be resolved
    /// is still on screen, and still the one to fix.
    fn commit_open_cell_edit(&mut self) {
        let context = self.edit_context();
        match self.tasks.commit_cell_edit(&context) {
            Ok(edits) => {
                self.dispatch_task_edits(edits);
                self.set_task_mode();
            }
            Err(message) => self.tasks.set_edit_notice(message),
        }
    }

    /// Commits `w`: saves the panel under the typed name and binds to it.
    ///
    /// An empty name is refused with the prompt left open — an entry nothing
    /// can name is an entry nothing can load or delete.
    fn commit_filter_set_save(&mut self) {
        let name = self
            .tasks
            .filter_set_prompt_text()
            .unwrap_or_default()
            .trim()
            .to_string();
        if name.is_empty() {
            self.tasks.set_filter_sets_notice("a name is required");
            return;
        }

        let entry = NamedFilterSet {
            name: name.clone(),
            sets: self.tasks.filter_sets_to_saved(),
        };
        match self
            .config
            .filter_sets
            .iter_mut()
            .find(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            Some(existing) => *existing = entry,
            None => self.config.filter_sets.push(entry),
        }

        self.tasks.filter_set_prompt_cancel();
        self.tasks.filter_set_bind(name);
        self.set_filter_mode();
        self.write_config_or_report();
    }

    /// Commits `d`: removes the loaded entry and detaches the panel.
    fn commit_filter_set_delete(&mut self) {
        if let Some(name) = self.tasks.filter_set_loaded_name().map(str::to_string) {
            self.config
                .filter_sets
                .retain(|entry| !entry.name.eq_ignore_ascii_case(&name));
        }

        self.tasks.filter_set_prompt_cancel();
        self.tasks.filter_set_detach();
        self.clamp_filter_sets_page();
        self.set_filter_mode();
        self.write_config_or_report();
    }

    /// The saved entry a digit addresses, if there is one there.
    fn filter_set_at(&self, position: u8) -> Option<NamedFilterSet> {
        let index = self
            .tasks
            .filter_sets_page_start()
            .saturating_add(position.saturating_sub(1) as usize);
        self.config
            .sorted_filter_sets()
            .get(index)
            .map(|entry| (*entry).clone())
    }

    /// Replaces the panel with a saved entry, by name.
    ///
    /// By name rather than by position because a confirmation can sit between
    /// the digit and the load, and a window that moved in between must not
    /// load a different entry than the one the prompt named.
    fn load_named_filter_set(&mut self, name: &str) {
        // Anything the bound panel was still holding goes to disk before it
        // is replaced. The write-through normally runs *after* the key, by
        // which point this key has already thrown the panel away.
        self.sync_named_filter_set();

        let Some(entry) = self
            .config
            .filter_sets
            .iter()
            .find(|entry| entry.name == name)
            .cloned()
        else {
            return;
        };

        self.tasks.filter_sets_load(&entry.name, &entry.sets);
    }

    /// Commits the `y` at a load confirmation: the unnamed panel goes.
    fn commit_filter_set_load(&mut self, name: &str) {
        self.tasks.filter_set_prompt_cancel();
        self.set_filter_mode();

        let task_targets_before = self.task_targets_before();
        self.load_named_filter_set(name);
        self.update_task_data_after_action(task_targets_before);
    }

    /// Keeps the numbered window pointing at entries that still exist.
    fn clamp_filter_sets_page(&mut self) {
        let total = self.config.filter_sets.len();
        self.tasks.filter_sets_page(0, total);
    }

    /// Writes the config, reporting a failure on the sidebar rather than
    /// returning it.
    ///
    /// Project visibility can afford to propagate a write error because it
    /// happens on one deliberate keypress; this also runs while someone is
    /// typing into a filter, and an unwritable config must not take the
    /// session down mid-word.
    fn write_config_or_report(&mut self) {
        match self.config.save_to_source_path() {
            Ok(()) => self.tasks.clear_filter_sets_notice(),
            Err(err) => {
                debug_log(&format!("filter set write failed: {err}"));
                self.tasks
                    .set_filter_sets_notice(format!("could not save: {err}"));
            }
        }
    }

    /// Writes the panel back to the named entry it was loaded from.
    ///
    /// No-op unless the panel is bound and something actually changed. The
    /// comparison is what makes the deliberately over-eager `dirty` flag safe.
    fn sync_named_filter_set(&mut self) {
        if !self.tasks.filter_set_dirty() {
            return;
        }
        // Cleared whether or not anything is written, so a config that cannot
        // be written reports once rather than once per keystroke.
        self.tasks.clear_filter_set_dirty();

        let Some(name) = self.tasks.filter_set_loaded_name().map(str::to_string) else {
            return;
        };
        let sets = self.tasks.filter_sets_to_saved();
        let Some(entry) = self
            .config
            .filter_sets
            .iter_mut()
            .find(|entry| entry.name.eq_ignore_ascii_case(&name))
        else {
            return;
        };
        if entry.sets == sets {
            return;
        }

        entry.sets = sets;
        self.write_config_or_report();
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

    /// The task targets as they stand, for `update_task_data_after_action` to
    /// compare against once an action has run.
    fn task_targets_before(&self) -> Option<Vec<String>> {
        match self.tasks.status() {
            crate::app::task::TaskStatus::Idle => None,
            _ => Some(self.task_target_project_ids()),
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

        let task_targets_before = self.task_targets_before();

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
                // A task date picked on the calendar commits the cell, not a
                // filter: the same key, one layer over.
                if self.tasks.cell_edit_owns_calendar() {
                    self.tasks.calendar_normalize();
                    self.commit_open_cell_edit();
                    return Ok(None);
                }
                self.tasks.filter_calendar_commit();
                self.set_filter_mode();
                self.ensure_task_data();
                return Ok(None);
            }
            Action::CalendarClose => {
                if self.tasks.cell_edit_owns_calendar() {
                    self.tasks.cancel_cell_edit();
                    self.set_task_mode();
                    return Ok(None);
                }
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
                // The picker stays open on a task date: `d` empties the value
                // and `enter` is still what sends it, so clearing a date is
                // one key-path rather than two.
                if self.tasks.cell_edit_owns_calendar() {
                    self.tasks.calendar_clear_text();
                    return Ok(None);
                }
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
            Action::FilterSetsToggle => {
                self.tasks.filter_sets_toggle_sidebar();
                self.clamp_filter_sets_page();
                return Ok(None);
            }
            Action::FilterSetsPageBack => {
                self.tasks.filter_sets_page(-1, self.config.filter_sets.len());
                return Ok(None);
            }
            Action::FilterSetsPageForward => {
                self.tasks.filter_sets_page(1, self.config.filter_sets.len());
                return Ok(None);
            }
            Action::FilterSetLoad(position) => {
                let Some(entry) = self.filter_set_at(*position) else {
                    return Ok(None);
                };
                // A bound panel is already on disk, so loading over it costs
                // nothing. An unnamed one that is filtering exists nowhere
                // else, and the digit did not ask for it to be thrown away.
                if self.tasks.filter_set_is_unsaved() {
                    self.tasks.filter_set_prompt_confirm_load(&entry.name);
                    self.mode = Mode::FilterSetName;
                    return Ok(None);
                }

                self.load_named_filter_set(&entry.name);
                // Deliberately not an early return past the fetch decision:
                // loading an entry can widen the due window pushed down to
                // Asana, and `TaskQuery::covers` records a fetched window as
                // cached — so skipping this leaves rows permanently missing
                // rather than merely late.
                self.update_task_data_after_action(task_targets_before);
                return Ok(None);
            }
            Action::FilterSetSave => {
                self.tasks.filter_set_prompt_save();
                self.mode = Mode::FilterSetName;
                return Ok(None);
            }
            Action::FilterSetDelete => {
                if self.tasks.filter_set_prompt_delete() {
                    self.mode = Mode::FilterSetName;
                }
                return Ok(None);
            }
            Action::FilterSetCopyToNew => {
                self.tasks.filter_set_detach();
                return Ok(None);
            }
            Action::FilterSetNew => {
                // Same reason as a load: the panel is about to go, and the
                // write-through does not run until after this key.
                self.sync_named_filter_set();
                self.tasks.filter_set_new();
                // Clearing a due filter widens the window pushed down to
                // Asana, for the same reason loading an entry can, so this
                // takes the same route past the fetch decision.
                self.update_task_data_after_action(task_targets_before);
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
            Action::BeginTaskEdit => {
                let context = self.edit_context();
                match self.tasks.begin_cell_edit(&context) {
                    Ok(()) => self.set_task_edit_mode(),
                    Err(message) => self.tasks.set_edit_notice(message),
                }
                return Ok(None);
            }
            Action::CancelTaskEdit => {
                self.tasks.cancel_cell_edit();
                self.set_task_mode();
                return Ok(None);
            }
            Action::TaskEditCycleValue(delta) => {
                self.tasks.cell_edit_cycle_value(*delta);
                return Ok(None);
            }
            Action::TaskEditClear => {
                self.tasks.cell_edit_clear();
                return Ok(None);
            }
            Action::TextCaretWordBack => {
                self.move_text_caret(TextMotion::WordBack);
                return Ok(None);
            }
            Action::TextCaretWordForward => {
                self.move_text_caret(TextMotion::WordForward);
                return Ok(None);
            }
            Action::TextCaretStart => {
                self.move_text_caret(TextMotion::Start);
                return Ok(None);
            }
            Action::TextCaretEnd => {
                self.move_text_caret(TextMotion::End);
                return Ok(None);
            }
            // Deliberately not early returns: both send a write, and the tail
            // is what keeps the fetch decision running after an action, for
            // the same reason `FilterSetLoad` falls through to it.
            Action::CommitTaskEdit => {
                self.commit_open_cell_edit();
            }
            Action::ToggleTaskCompleted if self.tasks.visible() => {
                let edits = self.tasks.toggle_completed_edits();
                self.dispatch_task_edits(edits);
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
        let result = self.handle_key_event_inner(keymap, event, page_size);
        // The last key of a burst is the one with an empty queue behind it,
        // which is the same moment `settle_table` picks to rebuild. Typing a
        // filter is one write, not one per character.
        //
        // Here rather than in `handle_action` because the prompt's keys, and
        // the filter-field ones, are read outside the keymap entirely.
        if !self.tasks.input_pending() {
            self.sync_named_filter_set();
        }
        result
    }

    fn handle_key_event_inner(
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
        self.poll_task_edits();

        debug_log(&format!(
            "key event: {:?} {:?}",
            event.code, event.modifiers
        ));

        // Ahead of the keymap: the prompt owns every key it is shown, so an
        // unbound letter types rather than falling through to a binding.
        if matches!(self.mode, Mode::FilterSetName)
            && self.handle_filter_set_name_input(event)?
        {
            return Ok(None);
        }

        if let Some(binding) = KeyBinding::from_crossterm_event(event) {
            debug_log(&format!("resolved binding: {binding:?}"));
            if let Some(action) = keymap.action_for(&binding, self.mode).cloned() {
                debug_log(&format!("resolved action: {action}"));
                // `j`, `k`, and `d` drive a value picker and type into
                // anything else, the same way the filter panel's label keys
                // are context-sensitive. Only a plain character is ambiguous:
                // `ctrl-l` clears whatever the editor holds.
                if action.is_task_edit_value_action()
                    && matches!(self.mode, Mode::TaskEdit)
                    && !self.tasks.cell_edit_is_options()
                    && !event
                        .modifiers
                        .intersects(crossterm::event::KeyModifiers::CONTROL
                            | crossterm::event::KeyModifiers::ALT)
                {
                    self.handle_task_edit_input(event)?;
                    return Ok(None);
                }
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

        if self.handle_task_edit_input(event)? {
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

    /// What resolving an edit needs that the task pane does not own.
    fn edit_context(&self) -> crate::app::task_edit::EditContext {
        crate::app::task_edit::EditContext {
            today: Some(crate::domain::today()),
            current_user_gid: self.current_user_gid.clone(),
        }
    }

    /// Applies a batch of edits locally, then sends each one in the background.
    ///
    /// One thread per edit, following `start_task_data_fetch`: the table does
    /// not freeze while twelve tasks are marked done.
    fn dispatch_task_edits(&mut self, edits: Vec<TaskEdit>) {
        if edits.is_empty() {
            return;
        }

        self.tasks.clear_edit_notice();
        self.tasks.apply_edits_locally(&edits);
        self.task_edits_sent += edits.len();
        self.task_edits_outstanding += edits.len();

        for edit in edits {
            let client = self.client.clone();
            let sender = self.task_edit_events.0.clone();
            thread::spawn(move || {
                let result = client.update_task(&edit.gid, &edit.field);
                let _ = sender.send(TaskEditMessage { edit, result });
            });
        }
    }

    /// Reconciles whatever writes have come back.
    ///
    /// A success takes the server's `modified_at`; a failure puts the field
    /// back the way it was. Both are keyed by gid, so replies arriving out of
    /// order — or belonging to different batches — need no bookkeeping.
    pub fn poll_task_edits(&mut self) {
        loop {
            match self.task_edit_events.1.try_recv() {
                Ok(message) => self.reconcile_task_edit(message),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
    }

    /// Waits for every in-flight write to be reconciled.
    ///
    /// The event loop never calls this; it polls. Tests and a shutdown that
    /// wants its writes accounted for do.
    pub fn settle_task_edits(&mut self) {
        while self.task_edits_outstanding > 0 {
            let Ok(message) = self.task_edit_events.1.recv() else {
                break;
            };
            self.reconcile_task_edit(message);
        }
    }

    fn reconcile_task_edit(&mut self, message: TaskEditMessage) {
        self.task_edits_outstanding = self.task_edits_outstanding.saturating_sub(1);

        match message.result {
            Ok(task) => self.tasks.confirm_edit(&message.edit.gid, task.modified_at),
            Err(err) => {
                debug_log(&format!("task write failed: {err}"));
                self.task_edit_failures += 1;
                self.task_edit_error = Some(err.to_string());
                // Optimism is worth it — nearly every write succeeds — but an
                // optimistic update that quietly diverges from the server is
                // worse than either, so the rollback is not optional.
                let rollback = TaskEdit {
                    gid: message.edit.gid.clone(),
                    field: message.edit.previous.clone(),
                    previous: message.edit.field.clone(),
                };
                self.tasks.apply_edit_locally(&rollback);
            }
        }

        if self.task_edits_outstanding > 0 {
            return;
        }

        if self.task_edit_failures > 0 {
            let error = self
                .task_edit_error
                .clone()
                .unwrap_or_else(|| "unknown error".to_string());
            self.tasks.set_edit_notice(format!(
                "could not update {} of {}: {error}",
                self.task_edit_failures, self.task_edits_sent
            ));
        }
        self.task_edits_sent = 0;
        self.task_edit_failures = 0;
        self.task_edit_error = None;
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


    // ---- Named filter sets -------------------------------------------------

    /// An app in filter mode, fully loaded, with a config file to write to.
    fn filter_sets_app() -> (App<FakeAsanaClient>, std::path::PathBuf) {
        let (config, path) = config_on_disk();
        let mut app = gantt_app();
        app.config = config;
        press(&mut app, KeyCode::Char('t'));
        settle(&mut app);
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(app.mode(), Mode::Filter);
        (app, path)
    }

    fn reread(path: &std::path::Path) -> Config {
        Config::from_toml_str(&std::fs::read_to_string(path).expect("config exists"))
            .expect("the written config reparses")
    }

    /// `w`, a name, `enter`, clearing whatever the prompt was pre-filled with.
    fn save_as(app: &mut App<FakeAsanaClient>, name: &str) {
        press(app, KeyCode::Char('w'));
        assert_eq!(app.mode(), Mode::FilterSetName);
        for _ in 0..40 {
            press(app, KeyCode::Backspace);
        }
        for ch in name.chars() {
            press(app, KeyCode::Char(ch));
        }
        press(app, KeyCode::Enter);
    }

    /// Moves the field cursor to a row by label, from filter-browse mode.
    fn move_to_field(app: &mut App<FakeAsanaClient>, label: &str) {
        // Back to the top first: `j` only goes one way, and the cursor is
        // shared with wherever the last test step left it.
        for _ in 0..20 {
            press(app, KeyCode::Char('k'));
        }
        for _ in 0..20 {
            let rows = app.tasks.filter_panel_rows();
            let selected = app
                .tasks
                .filter_panel_entries()
                .iter()
                .position(|entry| entry.selected)
                .expect("a row is selected");
            if rows[selected].0 == label {
                return;
            }
            press(app, KeyCode::Char('j'));
        }
        panic!("never reached the {label} row");
    }

    /// `enter`, some text, `enter` — the browse-mode way to fill a text field.
    fn type_into_field(app: &mut App<FakeAsanaClient>, label: &str, text: &str) {
        move_to_field(app, label);
        press(app, KeyCode::Enter);
        for ch in text.chars() {
            press(app, KeyCode::Char(ch));
        }
        press(app, KeyCode::Enter);
    }

    #[test]
    fn w_saves_the_panel_under_a_name_and_binds_to_it() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");

        save_as(&mut app, "mine");

        assert_eq!(app.mode(), Mode::Filter, "the prompt closed");
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("mine"));
        let written = reread(&path);
        assert_eq!(written.filter_sets.len(), 1);
        assert_eq!(written.filter_sets[0].name, "mine");
        assert_eq!(written.filter_sets[0].sets[0].fields[0].key, "assignee");
        assert_eq!(written.filter_sets[0].sets[0].fields[0].query, "alex");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_over_an_existing_name_replaces_it_rather_than_adding_a_second() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");

        type_into_field(&mut app, "Title", "ship");
        // `w` then `enter`: the prompt comes pre-filled with the loaded name,
        // so re-saving where you are takes two keys.
        press(&mut app, KeyCode::Char('w'));
        assert_eq!(app.tasks.filter_set_prompt_text(), Some("mine"));
        press(&mut app, KeyCode::Enter);

        let written = reread(&path);
        assert_eq!(written.filter_sets.len(), 1, "one entry, not two");
        assert_eq!(written.filter_sets[0].sets[0].fields.len(), 2);

        // A name that differs only in case is the same entry, because the
        // sidebar could not tell the two rows apart.
        save_as(&mut app, "MINE");
        assert_eq!(reread(&path).filter_sets.len(), 1);
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("MINE"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_empty_name_is_refused_with_the_prompt_still_open() {
        let (mut app, path) = filter_sets_app();

        press(&mut app, KeyCode::Char('w'));
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.mode(), Mode::FilterSetName, "still typing");
        assert!(app.tasks.filter_set_prompt_text().is_some());
        assert!(app.config.filter_sets.is_empty());
        assert!(
            app.tasks
                .filter_sets_notice()
                .is_some_and(|notice| notice.contains("name")),
            "and it says why"
        );

        // esc backs out without saving anything.
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode(), Mode::Filter);
        assert!(app.tasks.filter_set_prompt_text().is_none());
        assert_eq!(app.tasks.filter_set_loaded_name(), None);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_burst_of_typing_into_a_bound_panel_costs_exactly_one_write() {
        let (mut app, path) = filter_sets_app();
        save_as(&mut app, "mine");
        let before = reread(&path);

        move_to_field(&mut app, "Assignee");
        press(&mut app, KeyCode::Enter);

        // Mid-burst: the table rebuild is deferred and so is the write.
        app.tasks.set_input_pending(true);
        for ch in "alex".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(
            reread(&path).filter_sets,
            before.filter_sets,
            "nothing written mid-burst"
        );

        // The key that empties the queue writes once, with the whole word.
        app.tasks.set_input_pending(false);
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(
            reread(&path).filter_sets[0].sets[0].fields[0].query,
            "alexx"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_change_that_changes_nothing_does_not_rewrite_the_config() {
        // The `dirty` flag is deliberately over-eager, so the writer's
        // comparison is the only thing stopping the file from churning.
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");

        // A marker the app would erase if it rewrote the file.
        let marked = format!(
            "# untouched\n{}",
            std::fs::read_to_string(&path).expect("config exists")
        );
        std::fs::write(&path, &marked).expect("mark the config");

        // `ctrl-l` clears a row that is already clear: a real refresh_table,
        // so the panel is marked dirty, but nothing about it changed.
        move_to_field(&mut app, "Title");
        let keymap = app.keymap().expect("bindings parse");
        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
            10,
        )
        .expect("key handled");
        // And a plain cursor move, which does not even rebuild.
        press(&mut app, KeyCode::Char('j'));

        assert_eq!(
            std::fs::read_to_string(&path).expect("config exists"),
            marked,
            "the config was rewritten for a no-op change"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn y_copies_the_panel_to_a_new_unnamed_one_and_leaves_the_entry_as_it_was() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");
        let before = reread(&path);

        press(&mut app, KeyCode::Char('y'));

        assert_eq!(app.tasks.filter_set_loaded_name(), None);
        assert_eq!(
            app.tasks.filter_panel_rows()[1].1,
            "alex",
            "the panel keeps what it was showing"
        );

        type_into_field(&mut app, "Title", "ship");
        assert_eq!(
            reread(&path).filter_sets,
            before.filter_sets,
            "and later edits no longer reach the entry"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn n_starts_a_fresh_panel_without_disturbing_the_entry_it_came_from() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");
        let before = reread(&path);

        press(&mut app, KeyCode::Char('n'));

        assert_eq!(app.tasks.filter_set_loaded_name(), None);
        assert_eq!(app.tasks.active_filter_count(), 0, "nothing is set");
        assert_eq!(app.tasks.filter_set_position(), (0, 1), "one empty tab");
        assert_eq!(
            reread(&path).filter_sets,
            before.filter_sets,
            "the entry keeps what was last written to it"
        );

        // And the now-unbound panel writes nothing on a later edit.
        type_into_field(&mut app, "Title", "ship");
        assert_eq!(reread(&path).filter_sets, before.filter_sets);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_fresh_panel_refetches_what_the_filter_it_dropped_was_hiding() {
        // `n` clears a due filter, which widens the window pushed down to
        // Asana — and a window recorded as covered but never fetched leaves
        // rows missing for good.
        let (mut app, path) = filter_sets_app();

        move_to_field(&mut app, "Due");
        press(&mut app, KeyCode::Enter);
        for ch in "2026-07-01..2026-07-31".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        press(&mut app, KeyCode::Enter);

        app.tasks.invalidate_cache();
        app.request_task_data().expect("fetch starts");
        settle(&mut app);
        app.poll_task_data();
        assert!(app.task_data_receiver.is_none());

        press(&mut app, KeyCode::Char('n'));

        assert_eq!(app.tasks.desired_task_query().due_after, None);
        assert!(
            app.task_data_receiver.is_some(),
            "the dropped filter has to be refetched, not filtered from cache"
        );

        settle(&mut app);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn d_deletes_the_loaded_entry_after_a_confirmation_and_detaches() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");

        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.mode(), Mode::FilterSetName, "it asks first");
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.mode(), Mode::Filter);
        assert_eq!(reread(&path).filter_sets.len(), 1, "`n` keeps it");
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("mine"));

        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('y'));

        assert_eq!(app.mode(), Mode::Filter);
        assert!(reread(&path).filter_sets.is_empty());
        assert_eq!(app.tasks.filter_set_loaded_name(), None);
        assert_eq!(
            app.tasks.filter_panel_rows()[1].1,
            "alex",
            "deleting the entry does not empty the panel"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn d_is_refused_when_nothing_is_loaded() {
        // With no cursor in the sidebar there is no other unambiguous target.
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");
        press(&mut app, KeyCode::Char('y'));

        press(&mut app, KeyCode::Char('d'));

        assert_eq!(app.mode(), Mode::Filter, "no prompt opened");
        assert_eq!(reread(&path).filter_sets.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_digit_loads_the_entry_at_that_position_in_the_window() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "bravo");
        // `n` between them, so the second entry starts from nothing rather
        // than inheriting the first one's Assignee.
        press(&mut app, KeyCode::Char('n'));
        type_into_field(&mut app, "Title", "ship");
        save_as(&mut app, "alpha");
        press(&mut app, KeyCode::Char('n'));

        // Sorted by name, so `1` is alpha and `2` is bravo whatever order
        // they were written in. An empty unnamed panel has nothing to lose,
        // so no confirmation stands in the way.
        press(&mut app, KeyCode::Char('2'));

        assert_eq!(app.tasks.filter_set_loaded_name(), Some("bravo"));
        assert_eq!(app.tasks.filter_panel_rows()[1].1, "alex");
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "");

        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("alpha"));
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "ship");

        // A digit past the end of the list does nothing rather than clearing
        // the panel.
        press(&mut app, KeyCode::Char('9'));
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("alpha"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_digit_asks_before_throwing_away_an_unnamed_panel() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "saved");
        // An unnamed panel with a filter in it: work that exists nowhere but
        // on screen, and the digit did not ask for it to be thrown away.
        press(&mut app, KeyCode::Char('n'));
        type_into_field(&mut app, "Title", "unsaved work");

        press(&mut app, KeyCode::Char('1'));

        assert_eq!(app.mode(), Mode::FilterSetName, "it asks first");
        assert_eq!(
            app.tasks.filter_panel_rows()[0].1,
            "unsaved work",
            "and nothing has been loaded yet"
        );

        // `n` backs out, leaving the panel exactly as it was.
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.mode(), Mode::Filter);
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "unsaved work");
        assert_eq!(app.tasks.filter_set_loaded_name(), None);

        // `y` goes through with it.
        press(&mut app, KeyCode::Char('1'));
        press(&mut app, KeyCode::Char('y'));

        assert_eq!(app.mode(), Mode::Filter);
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("saved"));
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "");
        assert_eq!(app.tasks.filter_panel_rows()[1].1, "alex");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_digit_asks_nothing_when_there_is_nothing_to_lose() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "saved");

        // Bound: every change is already on disk, so loading over it costs
        // nothing.
        assert!(!app.tasks.filter_set_is_unsaved());
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.mode(), Mode::Filter, "no prompt for a bound panel");
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("saved"));

        // Unnamed but empty: nothing worth a keypress to confirm.
        press(&mut app, KeyCode::Char('n'));
        assert!(!app.tasks.filter_set_is_unsaved());
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.mode(), Mode::Filter, "nor for an empty one");
        assert_eq!(app.tasks.filter_set_loaded_name(), Some("saved"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_digit_past_the_end_of_the_list_asks_nothing_and_does_nothing() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Title", "unsaved work");

        press(&mut app, KeyCode::Char('9'));

        assert_eq!(app.mode(), Mode::Filter, "no entry, so no question");
        assert_eq!(app.tasks.filter_panel_rows()[0].1, "unsaved work");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_edit_still_in_hand_reaches_disk_before_the_panel_is_replaced() {
        // The write-through runs *after* the key, by which point a load has
        // already thrown the panel away — so the load has to flush first.
        let (mut app, path) = filter_sets_app();
        save_as(&mut app, "alpha");
        press(&mut app, KeyCode::Char('n'));
        save_as(&mut app, "bravo");

        move_to_field(&mut app, "Assignee");
        press(&mut app, KeyCode::Enter);
        app.tasks.set_input_pending(true);
        for ch in "alex".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        // Still mid-burst, so committing the edit does not flush it either.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode(), Mode::Filter);
        assert!(
            reread(&path).filter_sets[1].sets[0].fields.is_empty(),
            "still unwritten, mid-burst"
        );

        // `1` loads alpha. The edit bravo was still holding must not go with
        // the panel it was typed into.
        app.tasks.set_input_pending(false);
        press(&mut app, KeyCode::Char('1'));

        assert_eq!(app.tasks.filter_set_loaded_name(), Some("alpha"));
        let bravo = reread(&path)
            .filter_sets
            .into_iter()
            .find(|entry| entry.name == "bravo")
            .expect("bravo is still there");
        assert_eq!(bravo.sets[0].fields[0].query, "alex");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn b_toggles_the_sidebar_without_taking_the_field_cursor() {
        let (mut app, path) = filter_sets_app();
        let before = app
            .tasks
            .filter_panel_entries()
            .iter()
            .position(|entry| entry.selected);

        press(&mut app, KeyCode::Char('b'));
        assert!(app.tasks.filter_sets_sidebar_visible());

        // `j` still walks the filter fields, which is what the unfocused
        // border is promising.
        press(&mut app, KeyCode::Char('j'));
        let after = app
            .tasks
            .filter_panel_entries()
            .iter()
            .position(|entry| entry.selected);
        assert_eq!(after, before.map(|index| index + 1));

        press(&mut app, KeyCode::Char('b'));
        assert!(!app.tasks.filter_sets_sidebar_visible());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn loading_an_entry_that_widens_the_due_window_starts_a_fetch() {
        // The Milestone 13 hazard, one level up: `TaskQuery::covers` records
        // a fetched window as cached, so a load that skips the fetch decision
        // leaves rows permanently missing rather than merely late.
        let (mut app, path) = filter_sets_app();
        save_as(&mut app, "everything");
        // Unbound first, or the due window below would write straight through
        // into the entry this test needs to stay wide.
        press(&mut app, KeyCode::Char('n'));

        // A narrow due window, picked on the calendar the way a user would.
        move_to_field(&mut app, "Due");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode(), Mode::Calendar);
        for ch in "2026-07-01..2026-07-31".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        press(&mut app, KeyCode::Enter);
        save_as(&mut app, "july");

        // Re-fetch so the cache records the narrow window, not the broad one
        // the first load used.
        app.tasks.invalidate_cache();
        app.request_task_data().expect("fetch starts");
        settle(&mut app);
        app.poll_task_data();
        assert!(
            app.task_data_receiver.is_none(),
            "the narrow fetch finished before the load"
        );
        let targets = app.task_target_projects();
        let narrow = app.tasks.desired_task_query();
        assert_eq!(narrow.due_after.as_deref(), Some("2026-07-01"));
        assert!(app.tasks.can_serve_query_for_targets(&targets, &narrow));

        // `everything` sorts before `july`, so `1` is the wider entry. The
        // panel is bound to `july`, so nothing is at risk and no
        // confirmation stands in the way.
        press(&mut app, KeyCode::Char('1'));

        assert_eq!(app.tasks.filter_set_loaded_name(), Some("everything"));
        let widened = app.tasks.desired_task_query();
        assert_eq!(widened.due_after, None, "no due filter left to push down");
        assert_eq!(widened.due_before, None);
        assert!(
            app.task_data_receiver.is_some(),
            "the load has to go through the same fetch decision every other \
             filter change does"
        );

        settle(&mut app);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_config_that_cannot_be_written_reports_instead_of_ending_the_session() {
        let (mut app, path) = filter_sets_app();
        type_into_field(&mut app, "Assignee", "alex");
        save_as(&mut app, "mine");

        // A directory where the file should be: the write fails, the session
        // does not.
        std::fs::remove_file(&path).expect("remove the config");
        std::fs::create_dir(&path).expect("put a directory in its way");

        type_into_field(&mut app, "Title", "ship");

        assert!(
            app.tasks
                .filter_sets_notice()
                .is_some_and(|notice| notice.contains("could not save")),
            "the failure is reported"
        );
        assert_eq!(
            app.tasks.filter_panel_rows()[0].1,
            "ship",
            "and the edit still happened"
        );
        // Cleared with the write attempt, so a broken config produces one
        // message rather than one per keystroke.
        assert!(!app.tasks.filter_set_dirty());

        let _ = std::fs::remove_dir(&path);
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
                        resource_subtype: None,
                        enum_options: Vec::new(),
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
