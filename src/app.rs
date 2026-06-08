use crate::{
    asana::{AsanaClient, TaskLoadScope},
    config::Config,
    error::Result,
    input::{Action, AppCommand, KeyBinding, KeyMap},
};

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
};

pub mod project_list;
pub mod task_review;

use self::project_list::ProjectListState;
use self::task_review::{TaskFocusMode, TaskReviewState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppMode {
    Project,
    Filter,
    Task,
}

impl Default for AppMode {
    fn default() -> Self {
        Self::Project
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneMode {
    Normal,
    Minimized,
    Maximized,
}

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

#[derive(Debug)]
pub struct App<C> {
    pub config: Config,
    pub projects: ProjectListState,
    pub tasks: TaskReviewState,
    mode: AppMode,
    panel_size: PaneSizeState,
    client: C,
    config_path: Option<PathBuf>,
    task_load_generation: u64,
    task_load_receiver: Option<Receiver<TaskLoadMessage>>,
}

struct TaskLoadMessage {
    generation: u64,
    project_id: Option<String>,
    scope: TaskLoadScope,
    result: Result<crate::app::task_review::TaskDataset>,
    done: bool,
}

impl<C: AsanaClient + Clone + Send + 'static> App<C> {
    pub fn new(config: Config, client: C) -> Self {
        Self::with_optional_config_path(None, config, client)
    }

    pub fn with_config_path(path: impl Into<PathBuf>, config: Config, client: C) -> Self {
        Self::with_optional_config_path(Some(path.into()), config, client)
    }

    fn with_optional_config_path(
        config_path: Option<PathBuf>,
        config: Config,
        client: C,
    ) -> Self {
        Self {
            config,
            projects: ProjectListState::new(),
            tasks: TaskReviewState::new(),
            mode: AppMode::default(),
            panel_size: PaneSizeState::default(),
            client,
            config_path,
            task_load_generation: 0,
            task_load_receiver: None,
        }
    }

    pub fn load_projects(&mut self) -> Result<()> {
        self.projects
            .load_with_visibility(&self.client, &self.config.project_visibility)?;
        Ok(())
    }

    pub fn load_tasks(&mut self) -> Result<()> {
        self.start_task_load();
        Ok(())
    }

    pub fn keymap(&self) -> Result<KeyMap> {
        KeyMap::from_bindings(&self.config.effective_bindings())
    }

    pub fn mode(&self) -> AppMode {
        self.mode
    }

    pub fn panel_size(&self) -> &PaneSizeState {
        &self.panel_size
    }

    fn set_project_mode(&mut self) {
        self.mode = AppMode::Project;
        self.tasks.set_focus_mode(TaskFocusMode::Projects);
    }

    fn set_filter_mode(&mut self) {
        self.mode = AppMode::Filter;
        self.tasks.set_visible(true);
        self.tasks.set_focus_mode(TaskFocusMode::Projects);
        if !self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
    }

    fn set_task_mode(&mut self) {
        self.mode = AppMode::Task;
        self.tasks.set_visible(true);
        self.tasks.set_focus_mode(TaskFocusMode::Tasks);
        if self.tasks.filter_panel_visible() {
            self.tasks.toggle_filter_panel();
        }
    }

    fn set_top_panel_height_delta(&mut self, delta: i16) {
        self.panel_size.resize(delta);
    }

    fn minimize_top_panel(&mut self) {
        self.panel_size.minimize();
    }

    fn maximize_top_panel(&mut self) {
        self.panel_size.maximize();
    }

    fn restore_top_panel(&mut self) {
        self.panel_size.restore();
    }

    pub fn handle_action(&mut self, action: &Action, page_size: usize) -> Result<Option<AppCommand>> {
        if let Some(command) = action.as_app_command() {
            debug_log(&format!("app command: {command:?}"));
            return Ok(Some(command));
        }

        let task_targets_before = if !matches!(self.tasks.status(), crate::app::task_review::TaskReviewStatus::Idle) {
            Some(self.task_target_project_ids())
        } else {
            None
        };

        debug_log(&format!(
            "handle_action: action={action} mode={:?} focus={:?} visible_tasks={}",
            self.mode,
            self.tasks.focus_mode(),
            self.tasks.visible(),
        ));

        match action {
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
                if self.task_load_receiver.is_none()
                    && !self.tasks.can_serve_scope_for_targets(
                        &self.task_target_project_ids(),
                        self.tasks.desired_load_scope(),
                    )
                {
                    self.start_task_load();
                }
                return Ok(None);
            }
            Action::ToggleTaskMode => {
                if self.mode == AppMode::Task {
                    self.set_project_mode();
                } else {
                    self.set_task_mode();
                    if self.task_load_receiver.is_none()
                        && !self.tasks.can_serve_scope_for_targets(
                            &self.task_target_project_ids(),
                            self.tasks.desired_load_scope(),
                        )
                    {
                        self.start_task_load();
                    }
                }
                return Ok(None);
            }
            Action::ResizeWindowUp => {
                self.set_top_panel_height_delta(2);
                return Ok(None);
            }
            Action::ResizeWindowDown => {
                self.set_top_panel_height_delta(-2);
                return Ok(None);
            }
            Action::MinimizeWindow => {
                if self.panel_size.is_minimized() {
                    self.restore_top_panel();
                } else {
                    self.minimize_top_panel();
                }
                return Ok(None);
            }
            Action::MaximizeWindow => {
                if self.panel_size.is_maximized() {
                    self.restore_top_panel();
                } else {
                    self.maximize_top_panel();
                }
                return Ok(None);
            }
            Action::RestoreWindow => {
                self.restore_top_panel();
                return Ok(None);
            }
            Action::ToggleTaskView => {
                self.tasks.toggle_visible();
                if !self.tasks.visible() {
                    self.set_project_mode();
                }
                debug_log(&format!(
                    "toggle_task_view -> visible={} focus={:?}",
                    self.tasks.visible(),
                    self.tasks.focus_mode()
                ));
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
                self.tasks.cycle_completed_filter_without_refresh();
                if self.tasks.can_serve_scope_for_targets(
                    &self.task_target_project_ids(),
                    self.tasks.desired_load_scope(),
                ) {
                    if self.task_load_receiver.is_some() {
                        self.task_load_generation = self.task_load_generation.wrapping_add(1);
                        self.task_load_receiver = None;
                    }
                    self.tasks.refresh_from_cache();
                } else {
                    self.start_task_load();
                }
                return Ok(None);
            }
            _ => {}
        }

        let task_action = matches!(
            action,
            Action::MoveSectionUp
                | Action::MoveSectionDown
                | Action::MoveProjectUp
                | Action::MoveProjectDown
                | Action::ScrollLeft
                | Action::ScrollRight
                | Action::PageUp
                | Action::PageDown
                | Action::ToggleCompletedFilter
                | Action::ToggleSubtaskVisibility
                | Action::ToggleProjectGrouping
                | Action::ToggleSectionGrouping
                | Action::CycleTaskSort
        ) && self.tasks.visible();

        let result = if self.mode == AppMode::Project && !task_action {
            self.projects.apply_action(action, page_size)
        } else {
            self.tasks.apply_action(action, page_size)
        };

        if matches!(
            action,
            Action::ToggleStarredSelected | Action::ToggleHiddenSelected
        ) {
            self.sync_visibility_preferences()?;
        }

        if let Some(before) = task_targets_before {
            let after = self.task_target_project_ids();
            if before != after
                && !matches!(self.tasks.status(), crate::app::task_review::TaskReviewStatus::Idle)
            {
                if self.task_load_receiver.is_some() {
                    self.task_load_generation = self.task_load_generation.wrapping_add(1);
                }
                if self.tasks.visible() {
                    self.start_task_load();
                } else {
                    self.tasks.mark_out_of_date(
                        "selected projects changed; switch to task view to refresh",
                    );
                }
            }
        }

        if self.task_load_receiver.is_none()
            && self.tasks.visible()
            && !self.tasks.can_serve_scope_for_targets(
                &self.task_target_project_ids(),
                self.tasks.desired_load_scope(),
            )
        {
            self.start_task_load();
        }

        Ok(result)
    }

    pub fn handle_key_event(
        &mut self,
        keymap: &KeyMap,
        event: crossterm::event::KeyEvent,
        page_size: usize,
    ) -> Result<Option<AppCommand>> {
        if matches!(KeyBinding::from_crossterm_event(event), Some(KeyBinding::Ctrl('c'))) {
            return Ok(Some(AppCommand::Quit));
        }

        self.poll_task_load();

        debug_log(&format!("key event: {:?} {:?}", event.code, event.modifiers));

        if self.tasks.filter_panel_visible() {
            use crossterm::event::{KeyCode, KeyModifiers};

            let editing = self.tasks.filter_panel_editing();
            let is_plain_char = !event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
            let selected_kind = self.tasks.filter_selected_kind();

            if editing {
                match event.code {
                    KeyCode::Esc | KeyCode::Enter => {
                        self.tasks.filter_edit_done();
                        if matches!(event.code, KeyCode::Esc) {
                            self.tasks.toggle_filter_panel();
                            self.set_task_mode();
                        }
                        return Ok(None);
                    }
                    KeyCode::Backspace => {
                        self.tasks.filter_pop_char();
                        return Ok(None);
                    }
                    KeyCode::Char('h')
                        if is_plain_char
                            && matches!(
                                selected_kind,
                                Some(crate::app::task_review::TaskFieldFilterKind::Labels)
                            ) =>
                    {
                        self.tasks.filter_move_label_left();
                        return Ok(None);
                    }
                    KeyCode::Char('l')
                        if is_plain_char
                            && matches!(
                                selected_kind,
                                Some(crate::app::task_review::TaskFieldFilterKind::Labels)
                            ) =>
                    {
                        self.tasks.filter_move_label_right();
                        return Ok(None);
                    }
                    KeyCode::Char('j')
                        if is_plain_char
                            && matches!(
                                selected_kind,
                                Some(crate::app::task_review::TaskFieldFilterKind::Labels)
                            ) =>
                    {
                        self.tasks.filter_cycle_label_value(1);
                        return Ok(None);
                    }
                    KeyCode::Char('k')
                        if is_plain_char
                            && matches!(
                                selected_kind,
                                Some(crate::app::task_review::TaskFieldFilterKind::Labels)
                            ) =>
                    {
                        self.tasks.filter_cycle_label_value(-1);
                        return Ok(None);
                    }
                    KeyCode::Char('a')
                        if is_plain_char
                            && matches!(
                                selected_kind,
                                Some(crate::app::task_review::TaskFieldFilterKind::Labels)
                            ) =>
                    {
                        self.tasks.filter_add_label();
                        return Ok(None);
                    }
                    KeyCode::Char('d')
                        if is_plain_char
                            && matches!(
                                selected_kind,
                                Some(crate::app::task_review::TaskFieldFilterKind::Labels)
                            ) =>
                    {
                        self.tasks.filter_delete_label();
                        return Ok(None);
                    }
                    KeyCode::Char('l')
                        if event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.tasks.filter_clear_current();
                        return Ok(None);
                    }
                    KeyCode::Char('f')
                        if event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.tasks
                            .filter_set_mode(crate::app::task_review::TaskFieldStringMode::Fuzzy);
                        return Ok(None);
                    }
                    KeyCode::Char('s')
                        if event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.tasks.filter_set_mode(
                            crate::app::task_review::TaskFieldStringMode::Substring,
                        );
                        return Ok(None);
                    }
                    KeyCode::Char('r')
                        if event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.tasks
                            .filter_set_mode(crate::app::task_review::TaskFieldStringMode::Regex);
                        return Ok(None);
                    }
                    KeyCode::Char(c) if is_plain_char => {
                        self.tasks.filter_push_char(c);
                        return Ok(None);
                    }
                    _ => {}
                }
            } else {
                match event.code {
                    KeyCode::Esc => {
                        self.tasks.toggle_filter_panel();
                        self.set_task_mode();
                        return Ok(None);
                    }
                    KeyCode::Enter => {
                        self.tasks.filter_edit_begin();
                        return Ok(None);
                    }
                    KeyCode::Up | KeyCode::Char('k') if is_plain_char => {
                        self.tasks.move_filter_up();
                        return Ok(None);
                    }
                    KeyCode::Down | KeyCode::Char('j') if is_plain_char => {
                        self.tasks.move_filter_down();
                        return Ok(None);
                    }
                    KeyCode::PageUp
                    | KeyCode::Char('u')
                        if event.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        self.tasks.filter_page_up(page_size);
                        return Ok(None);
                    }
                    KeyCode::PageDown
                    | KeyCode::Char('d')
                        if event.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        self.tasks.filter_page_down(page_size);
                        return Ok(None);
                    }
                    KeyCode::Char('f') if is_plain_char =>
                    {
                        self.tasks.toggle_filter_panel();
                        return Ok(None);
                    }
                    KeyCode::Char('s') if is_plain_char => {
                        self.tasks.filter_cycle_mode();
                        return Ok(None);
                    }
                    KeyCode::Char('l')
                        if event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.tasks.filter_clear_current();
                        return Ok(None);
                    }
                    KeyCode::Char('f')
                        if event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.tasks.filter_set_mode(crate::app::task_review::TaskFieldStringMode::Fuzzy);
                        return Ok(None);
                    }
                    KeyCode::Char('s')
                        if event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.tasks
                            .filter_set_mode(crate::app::task_review::TaskFieldStringMode::Substring);
                        return Ok(None);
                    }
                    KeyCode::Char('r')
                        if event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.tasks.filter_set_mode(crate::app::task_review::TaskFieldStringMode::Regex);
                        return Ok(None);
                    }
                    _ => {}
                }
            }
        }

        if self.projects.search_active() {
            use crossterm::event::{KeyCode, KeyModifiers};

            match event.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.projects.end_search();
                    return Ok(None);
                }
                KeyCode::Backspace => {
                    self.projects.pop_search_char();
                    return Ok(None);
                }
                KeyCode::Char(c)
                    if !event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.projects.push_search_char(c.to_ascii_lowercase());
                    return Ok(None);
                }
                _ => {}
            }
        }

        if let Some(binding) = KeyBinding::from_crossterm_event(event) {
            debug_log(&format!("resolved binding: {binding:?}"));
            if self.mode != AppMode::Task {
                match binding {
                    KeyBinding::Char('[') => {
                        return self.handle_action(&Action::ResizeWindowDown, page_size);
                    }
                    KeyBinding::Char(']') => {
                        return self.handle_action(&Action::ResizeWindowUp, page_size);
                    }
                    KeyBinding::Char('{') => {
                        return self.handle_action(&Action::MinimizeWindow, page_size);
                    }
                    KeyBinding::Char('}') => {
                        return self.handle_action(&Action::MaximizeWindow, page_size);
                    }
                    KeyBinding::Char('0') => {
                        return self.handle_action(&Action::RestoreWindow, page_size);
                    }
                    _ => {}
                }
            }
            if self.tasks.visible() && self.mode == AppMode::Task {
                if let Some(action) = task_view_action_for(&binding) {
                    debug_log(&format!("task overlay action: {action}"));
                    return self.handle_action(&action, page_size);
                }
            }
            if let Some(action) = keymap.action_for(&binding).cloned() {
                debug_log(&format!("resolved action: {action}"));
                return self.handle_action(&action, page_size);
            }
            debug_log("no action for binding");
        }

        Ok(None)
    }

    pub fn poll_task_load(&mut self) {
        loop {
            let result = {
                let Some(receiver) = self.task_load_receiver.as_ref() else {
                    return;
                };
                receiver.try_recv()
            };

            match result {
                Ok(message) => {
                    if message.generation == self.task_load_generation {
                        if message.done {
                            self.tasks.finish_loading_targets();
                            self.task_load_receiver = None;
                            break;
                        }

                        let Some(project_id) = message.project_id.as_deref() else {
                            continue;
                        };
                        match message.result {
                            Ok(dataset) => self.tasks.ingest_loaded_project(project_id, message.scope, dataset),
                            Err(err) => {
                                debug_log(&format!("task load error: {err}"));
                                self.tasks.set_error(err.to_string());
                                self.task_load_receiver = None;
                                break;
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.task_load_receiver = None;
                    break;
                }
            }
        }
    }

    fn sync_visibility_preferences(&mut self) -> Result<()> {
        self.config.project_visibility = self.projects.visibility_preferences();
        if let Some(path) = &self.config_path {
            self.config.save_to_path(path)?;
        }
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

    fn start_task_load(&mut self) {
        let targets = self.task_target_projects();
        let scope = self.tasks.desired_load_scope();
        let projects_to_load = self.tasks.projects_requiring_load(&targets, scope);
        debug_log(&format!(
            "start_task_load: visible={} focus={:?} scope={:?} targets={} load={}",
            self.tasks.visible(),
            self.tasks.focus_mode(),
            scope,
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
                .finish_loading_dataset(crate::app::task_review::TaskDataset::default());
            return;
        }

        if projects_to_load.is_empty() {
            self.tasks.begin_loading(&targets);
            self.tasks.finish_loading_targets();
            return;
        }

        self.task_load_generation = self.task_load_generation.wrapping_add(1);
        let generation = self.task_load_generation;
        self.tasks.begin_loading(&targets);

        let (sender, receiver) = mpsc::channel();
        let client = self.client.clone();
        self.task_load_receiver = Some(receiver);

        thread::spawn(move || {
            for project in projects_to_load {
                let result = crate::app::task_review::TaskReviewState::build_dataset_for_projects(
                    &client,
                    &[project.clone()],
                    scope,
                );
                let _ = sender.send(TaskLoadMessage {
                    generation,
                    project_id: Some(project.id.clone()),
                    scope,
                    result,
                    done: false,
                });
            }

            let _ = sender.send(TaskLoadMessage {
                generation,
                project_id: None,
                scope,
                result: Ok(crate::app::task_review::TaskDataset::default()),
                done: true,
            });
        });
    }
}

fn task_view_action_for(binding: &KeyBinding) -> Option<Action> {
    match binding {
        KeyBinding::Char('c') => Some(Action::ToggleCompletedFilter),
        KeyBinding::Char('z') => Some(Action::ToggleSubtaskVisibility),
        KeyBinding::Char('s') => Some(Action::CycleTaskSort),
        KeyBinding::Char('[') => Some(Action::MoveSectionUp),
        KeyBinding::Char(']') => Some(Action::MoveSectionDown),
        KeyBinding::Char('{') => Some(Action::MoveProjectUp),
        KeyBinding::Char('}') => Some(Action::MoveProjectDown),
        KeyBinding::Left => Some(Action::ScrollLeft),
        KeyBinding::Right => Some(Action::ScrollRight),
        KeyBinding::PageUp => Some(Action::PageUp),
        KeyBinding::PageDown => Some(Action::PageDown),
        _ => None,
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
        domain::Project,
        input::{Action, KeyBinding},
    };

    use super::App;
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
    fn app_applies_project_visibility_preferences_from_config() {
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

        assert_eq!(app.projects.items().len(), 1);
        assert_eq!(app.projects.items()[0].id, "1");
        assert_eq!(app.projects.hidden_count(), 1);
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

        assert_eq!(keymap.action_for(&KeyBinding::Char('x')), Some(&Action::Quit));
        assert_eq!(keymap.action_for(&KeyBinding::Char('j')), Some(&Action::MoveDown));
        assert_eq!(keymap.action_for(&KeyBinding::Char('t')), Some(&Action::SetTaskMode));
        assert_eq!(keymap.action_for(&KeyBinding::Char('f')), Some(&Action::SetFilterMode));
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('p')),
            Some(&Action::SetProjectMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('m')),
            None
        );
    }

    #[test]
    fn braces_toggle_the_top_panel_between_restored_minimized_and_maximized() {
        let client = FakeAsanaClient::new(vec![Project::new("1", "Inbox", true)]);
        let mut app = App::new(Config::default(), client);

        app.handle_action(&Action::MinimizeWindow, 10)
            .expect("minimize top panel");

        assert!(app.panel_size().is_minimized());
        assert_eq!(app.panel_size().actual_height(20, 6), 0);

        app.handle_action(&Action::MinimizeWindow, 10)
            .expect("un-minimize top panel");

        assert!(app.panel_size().is_normal());

        app.handle_action(&Action::MaximizeWindow, 10)
            .expect("maximize top panel");

        assert!(app.panel_size().is_maximized());
        assert_eq!(app.panel_size().actual_height(20, 6), 20);

        app.handle_action(&Action::MaximizeWindow, 10)
            .expect("un-maximize top panel");

        assert!(app.panel_size().is_normal());
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

    #[test]
    fn app_marks_tasks_out_of_date_when_project_selection_changes() {
        let client = FakeAsanaClient::new(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
        ]);
        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");

        let table = crate::app::task_review::TaskReviewState::build_table_for_projects(
            &app.client,
            &[Project::new("1", "Inbox", true)],
        )
        .expect("table builds");
        app.tasks.begin_loading(&[Project::new("1", "Inbox", true)]);
        app.tasks.finish_loading(table);

        assert_eq!(app.tasks.status(), &crate::app::task_review::TaskReviewStatus::Empty);

        app.handle_action(&Action::MoveDown, 10).expect("move down");
        app.handle_action(&Action::ToggleSelection, 10)
            .expect("toggle selection");

        assert!(matches!(
            app.tasks.status(),
            crate::app::task_review::TaskReviewStatus::OutOfDate(message)
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
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("High".to_string()),
                            enum_value: None,
                        }],
                    },
                ],
            );
        let table = crate::app::task_review::TaskReviewState::build_table_for_projects(
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
                        custom_fields: vec![],
                    },
                ],
            );

        let mut app = App::new(config, client);
        app.load_projects().expect("projects load");
        app.tasks.set_completed_filter(None);
        app.tasks
            .load_for_projects(&app.client, &[Project::new("1", "Inbox", true)])
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
                        custom_fields: vec![],
                    },
                ],
            );

        let mut app = App::new(Config::default(), client);
        app.load_projects().expect("projects load");
        app.tasks
            .load_for_projects(&app.client, &[Project::new("1", "Inbox", true)])
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

        app.handle_key_event(&keymap, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE), 10)
            .expect("cycle string mode");
        let cycled_kind = app
            .tasks
            .filter_panel_entries()
            .into_iter()
            .find(|row| row.selected)
            .map(|row| row.kind)
            .expect("selected filter exists");
        assert_ne!(original_kind, cycled_kind);

        app.handle_key_event(&keymap, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), 10)
            .expect("move filter selection down");
        assert_eq!(
            app.tasks.filter_panel_entries().iter().position(|row| row.selected),
            Some(1)
        );

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            2,
        )
        .expect("page filter selection down");
        assert_eq!(
            app.tasks.filter_panel_entries().iter().position(|row| row.selected),
            Some(3)
        );

        app.handle_key_event(
            &keymap,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            2,
        )
        .expect("page filter selection up");
        assert_eq!(
            app.tasks.filter_panel_entries().iter().position(|row| row.selected),
            Some(1)
        );

        app.handle_key_event(&keymap, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE), 10)
            .expect("move filter selection up");
        assert_eq!(
            app.tasks.filter_panel_entries().iter().position(|row| row.selected),
            Some(0)
        );

        app.handle_key_event(&keymap, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10)
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

        app.handle_key_event(&keymap, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10)
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
}
