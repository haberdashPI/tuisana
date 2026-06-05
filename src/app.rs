use crate::{
    asana::AsanaClient,
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

#[derive(Debug)]
pub struct App<C> {
    pub config: Config,
    pub projects: ProjectListState,
    pub tasks: TaskReviewState,
    client: C,
    config_path: Option<PathBuf>,
    task_load_generation: u64,
    task_load_receiver: Option<Receiver<TaskLoadMessage>>,
}

struct TaskLoadMessage {
    generation: u64,
    result: Result<crate::domain::TaskTableModel>,
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

    pub fn handle_action(&mut self, action: &Action, page_size: usize) -> Result<Option<AppCommand>> {
        if let Some(command) = action.as_app_command() {
            debug_log(&format!("app command: {command:?}"));
            return Ok(Some(command));
        }

        let task_targets_before = if self.tasks.focus_mode() == TaskFocusMode::Projects {
            Some(self.task_target_project_ids())
        } else {
            None
        };

        debug_log(&format!(
            "handle_action: action={action} focus={:?} visible_tasks={}",
            self.tasks.focus_mode(),
            self.tasks.visible(),
        ));

        match action {
            Action::ScrollLeft => {
                self.tasks.scroll_left();
                return Ok(None);
            }
            Action::ScrollRight => {
                self.tasks.scroll_right();
                return Ok(None);
            }
            _ => {}
        }

        let result = if self.tasks.focus_mode() == TaskFocusMode::Tasks {
            self.tasks.apply_action(action, page_size)
        } else {
            self.projects.apply_action(action, page_size)
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
                self.tasks.mark_out_of_date("selected projects changed; switch to task view to refresh");
            }
        }

        match action {
            Action::ToggleTaskView => {
                self.tasks.toggle_visible();
                debug_log(&format!(
                    "toggle_task_view -> visible={} focus={:?}",
                    self.tasks.visible(),
                    self.tasks.focus_mode()
                ));
            }
            Action::ToggleTaskMode => {
                self.tasks.toggle_focus_mode();
                debug_log(&format!(
                    "toggle_task_mode -> visible={} focus={:?}",
                    self.tasks.visible(),
                    self.tasks.focus_mode()
                ));
                if self.tasks.focus_mode() == TaskFocusMode::Tasks {
                    if !self.tasks.visible() {
                        self.tasks.set_visible(true);
                    }
                    self.start_task_load();
                }
            }
            _ => {}
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
            if let Some(action) = keymap.action_for(&binding).cloned() {
                debug_log(&format!("resolved action: {action}"));
                return self.handle_action(&action, page_size);
            }
            debug_log("no action for binding");
        }

        Ok(None)
    }

    pub fn poll_task_load(&mut self) {
        let result = {
            let Some(receiver) = self.task_load_receiver.as_ref() else {
                return;
            };
            receiver.try_recv()
        };

        match result {
            Ok(message) => {
                if message.generation == self.task_load_generation {
                    match message.result {
                        Ok(table) => self.tasks.finish_loading(table),
                        Err(err) => {
                            debug_log(&format!("task load error: {err}"));
                            self.tasks.set_error(err.to_string());
                        }
                    }
                }
                self.task_load_receiver = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.task_load_receiver = None;
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
        debug_log(&format!(
            "start_task_load: visible={} focus={:?} targets={}",
            self.tasks.visible(),
            self.tasks.focus_mode(),
            targets
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));

        if targets.is_empty() {
            self.tasks.finish_loading(crate::domain::TaskTableModel::empty());
            return;
        }

        self.task_load_generation = self.task_load_generation.wrapping_add(1);
        let generation = self.task_load_generation;
        self.tasks.begin_loading(&targets);

        let (sender, receiver) = mpsc::channel();
        let client = self.client.clone();
        self.task_load_receiver = Some(receiver);

        thread::spawn(move || {
            let result = crate::app::task_review::TaskReviewState::build_table_for_projects(
                &client,
                &targets,
            );
            let _ = sender.send(TaskLoadMessage { generation, result });
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
        asana::fake::FakeAsanaClient,
        config::{Config, ProjectVisibilityConfig},
        domain::Project,
        input::{Action, KeyBinding},
    };

    use super::App;

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
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('m')),
            Some(&Action::ToggleTaskMode)
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
}
