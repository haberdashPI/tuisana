use crate::{
    asana::AsanaClient,
    config::Config,
    error::Result,
    input::{Action, AppCommand, KeyBinding, KeyMap},
};

use std::path::PathBuf;

pub mod project_list;

use self::project_list::ProjectListState;

#[derive(Debug)]
pub struct App<C> {
    pub config: Config,
    pub projects: ProjectListState,
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
            client,
            config_path,
        }
    }

    pub fn load_projects(&mut self) -> Result<()> {
        self.projects
            .load_with_visibility(&self.client, &self.config.project_visibility)?;
        Ok(())
    }

    pub fn keymap(&self) -> Result<KeyMap> {
        KeyMap::from_bindings(&self.config.bind)
    }

    pub fn handle_action(&mut self, action: &Action, page_size: usize) -> Result<Option<AppCommand>> {
        let result = self.projects.apply_action(action, page_size);
        if matches!(
            action,
            Action::ToggleStarredSelected | Action::ToggleHiddenSelected
        ) {
            self.sync_visibility_preferences()?;
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
            if let Some(action) = keymap.action_for(&binding).cloned() {
                return self.handle_action(&action, page_size);
            }
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
}

#[cfg(test)]
mod tests {
    use crate::{
        asana::fake::FakeAsanaClient,
        config::{Config, ProjectVisibilityConfig},
        domain::Project,
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
}
