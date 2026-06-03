use crate::{asana::AsanaClient, domain::Project, error::Result};

#[derive(Clone, Debug, Default)]
pub struct FakeAsanaClient {
    projects: Vec<Project>,
}

impl FakeAsanaClient {
    pub fn new(projects: Vec<Project>) -> Self {
        Self { projects }
    }

    pub fn with_default_projects() -> Self {
        Self::new(vec![Project::new("1", "Inbox", true), Project::new("2", "Backlog", false)])
    }
}

impl AsanaClient for FakeAsanaClient {
    fn list_projects(&self) -> Result<Vec<Project>> {
        Ok(self.projects.clone())
    }
}

