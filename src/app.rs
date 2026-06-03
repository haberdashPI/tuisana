use crate::{
    asana::AsanaClient,
    config::Config,
    domain::Project,
    error::Result,
    input::KeyMap,
};

#[derive(Debug)]
pub struct App<C> {
    pub config: Config,
    pub projects: Vec<Project>,
    client: C,
}

impl<C: AsanaClient> App<C> {
    pub fn new(config: Config, client: C) -> Self {
        Self {
            config,
            projects: Vec::new(),
            client,
        }
    }

    pub fn load_projects(&mut self) -> Result<()> {
        self.projects = self.client.list_projects()?;
        Ok(())
    }

    pub fn keymap(&self) -> Result<KeyMap> {
        KeyMap::from_key_bindings(&self.config.keys)
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

        assert_eq!(app.projects.len(), 1);
        assert_eq!(app.projects[0].name, "Inbox");
    }
}

