use crate::{
    asana::AsanaClient,
    config::Config,
    error::Result,
    input::{Action, AppCommand, KeyBinding, KeyMap},
};

use std::path::PathBuf;

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
}

impl<C: AsanaClient> App<C> {
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
        }
    }

    pub fn load_projects(&mut self) -> Result<()> {
        self.projects
            .load_with_visibility(&self.client, &self.config.project_visibility)?;
        Ok(())
    }

    pub fn load_tasks(&mut self) -> Result<()> {
        let targets = self.task_target_projects();
        debug_log(&format!(
            "load_tasks: visible={} focus={:?} targets={}",
            self.tasks.visible(),
            self.tasks.focus_mode(),
            targets
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));
        self.tasks.load_for_projects(&self.client, &targets)
    }

    pub fn keymap(&self) -> Result<KeyMap> {
        KeyMap::from_bindings(&self.config.effective_bindings())
    }

    pub fn handle_action(&mut self, action: &Action, page_size: usize) -> Result<Option<AppCommand>> {
        if let Some(command) = action.as_app_command() {
            debug_log(&format!("app command: {command:?}"));
            return Ok(Some(command));
        }

        let before_targets = self.task_target_projects();
        debug_log(&format!(
            "handle_action: action={action} focus={:?} visible_tasks={} before_targets={}",
            self.tasks.focus_mode(),
            self.tasks.visible(),
            before_targets
                .iter()
                .map(|project| project.id.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));
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

        match action {
            Action::ToggleTaskView => {
                self.tasks.toggle_visible();
                debug_log(&format!(
                    "toggle_task_view -> visible={} focus={:?}",
                    self.tasks.visible(),
                    self.tasks.focus_mode()
                ));
                if self.tasks.visible() {
                    self.load_tasks()?;
                }
            }
            Action::ToggleTaskMode => {
                self.tasks.toggle_focus_mode();
                debug_log(&format!(
                    "toggle_task_mode -> visible={} focus={:?}",
                    self.tasks.visible(),
                    self.tasks.focus_mode()
                ));
                if self.tasks.focus_mode() == TaskFocusMode::Tasks && !self.tasks.visible() {
                    self.tasks.set_visible(true);
                    self.load_tasks()?;
                } else if self.tasks.visible() {
                    self.load_tasks()?;
                }
            }
            _ => {}
        }

        let after_targets = self.task_target_projects();
        if before_targets != after_targets && self.tasks.visible() {
            self.load_tasks()?;
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
}
