use crate::{asana::AsanaClient, domain::Project, error::Result};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectListState {
    projects: Vec<Project>,
    selected: Option<usize>,
    status: ProjectListStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectListStatus {
    Idle,
    Loading,
    Ready,
    Empty,
    Error(String),
}

impl Default for ProjectListStatus {
    fn default() -> Self {
        Self::Idle
    }
}

impl ProjectListState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load<C: AsanaClient>(&mut self, client: &C) -> Result<()> {
        self.status = ProjectListStatus::Loading;
        let mut projects = match client.list_projects() {
            Ok(projects) => projects,
            Err(err) => {
                self.projects.clear();
                self.selected = None;
                self.status = ProjectListStatus::Error(err.to_string());
                return Err(err);
            }
        };
        sort_projects(&mut projects);

        self.selected = if projects.is_empty() { None } else { Some(0) };
        self.status = if projects.is_empty() {
            ProjectListStatus::Empty
        } else {
            ProjectListStatus::Ready
        };
        self.projects = projects;
        Ok(())
    }

    pub fn from_projects(mut projects: Vec<Project>) -> Self {
        sort_projects(&mut projects);
        let selected = if projects.is_empty() { None } else { Some(0) };
        let status = if projects.is_empty() {
            ProjectListStatus::Empty
        } else {
            ProjectListStatus::Ready
        };

        Self {
            projects,
            selected,
            status,
        }
    }

    pub fn items(&self) -> &[Project] {
        &self.projects
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_project(&self) -> Option<&Project> {
        self.selected.and_then(|index| self.projects.get(index))
    }

    pub fn status(&self) -> &ProjectListStatus {
        &self.status
    }

    pub fn move_up(&mut self) {
        match self.selected {
            Some(0) | None => {}
            Some(index) => self.selected = Some(index.saturating_sub(1)),
        }
    }

    pub fn move_down(&mut self) {
        if let Some(index) = self.selected {
            if index + 1 < self.projects.len() {
                self.selected = Some(index + 1);
            }
        }
    }
}

fn sort_projects(projects: &mut [Project]) {
    projects.sort_by(|left, right| {
        right
            .starred
            .cmp(&left.starred)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
}

#[cfg(test)]
mod tests {
    use crate::{asana::fake::FakeAsanaClient, domain::Project};

    use super::{ProjectListState, ProjectListStatus};

    #[test]
    fn sorts_starred_projects_first_then_name() {
        let state = ProjectListState::from_projects(vec![
            Project::new("3", "Zeta", false),
            Project::new("1", "Backlog", true),
            Project::new("2", "Alpha", true),
        ]);

        let names: Vec<_> = state.items().iter().map(|project| project.name.as_str()).collect();

        assert_eq!(names, vec!["Alpha", "Backlog", "Zeta"]);
        assert_eq!(state.selected_index(), Some(0));
    }

    #[test]
    fn loads_empty_and_selects_nothing() {
        let client = FakeAsanaClient::new(vec![]);
        let mut state = ProjectListState::new();

        state.load(&client).expect("projects load");

        assert!(state.items().is_empty());
        assert_eq!(state.selected_index(), None);
        assert_eq!(state.status(), &ProjectListStatus::Empty);
    }

    #[test]
    fn navigation_respects_bounds() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
        ]);

        assert_eq!(state.selected_index(), Some(0));
        state.move_up();
        assert_eq!(state.selected_index(), Some(0));
        state.move_down();
        assert_eq!(state.selected_index(), Some(1));
        state.move_down();
        assert_eq!(state.selected_index(), Some(1));
    }
}
