use crate::{
    asana::AsanaClient,
    config::Config,
    error::Result,
    input::KeyMap,
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
        self.projects.load(&self.client)?;
        Ok(())
    }

    pub fn keymap(&self) -> Result<KeyMap> {
        KeyMap::from_bindings(&self.config.bind)
    }
}

#[cfg(test)]
mod tests {
    use crate::{asana::fake::FakeAsanaClient, config::Config, domain::Project};

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
}
