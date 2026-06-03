pub mod fake;

use crate::{domain::Project, error::Result};

pub trait AsanaClient {
    fn list_projects(&self) -> Result<Vec<Project>>;
}

