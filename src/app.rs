use crate::{
    asana::AsanaClient,
    config::Config,
    error::Result,
    input::{Action, AppCommand, KeyMap},
};

pub mod project_list;

use self::project_list::ProjectListState;

#[derive(Debug)]
pub struct App<C> {
    pub config: Config,
    pub projects: ProjectListState,
    client: C,
}

impl<C: AsanaClient> App<C> {
    pub fn new(config: Config, client: C) -> Self {
        Self {
            config,
            projects: ProjectListState::new(),
            client,
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

    pub fn handle_action(&mut self, action: &Action) -> Option<AppCommand> {
        self.projects.apply_action(action)
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

        assert_eq!(app.handle_action(&crate::input::Action::MoveDown), None);
        assert_eq!(app.projects.selected_index(), Some(1));
        assert_eq!(
            app.handle_action(&crate::input::Action::Quit),
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
