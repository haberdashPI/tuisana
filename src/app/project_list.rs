use crate::{
    asana::AsanaClient,
    config::ProjectVisibilityConfig,
    domain::Project,
    error::Result,
    input::{Action, AppCommand},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectListState {
    all_projects: Vec<Project>,
    projects: Vec<Project>,
    selected: Option<usize>,
    status: ProjectListStatus,
    show_hidden: bool,
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
        self.load_with_visibility(client, &[])
    }

    pub fn load_with_visibility<C: AsanaClient>(
        &mut self,
        client: &C,
        visibility: &[ProjectVisibilityConfig],
    ) -> Result<()> {
        self.status = ProjectListStatus::Loading;
        let mut projects = match client.list_projects() {
            Ok(projects) => projects,
            Err(err) => {
                self.all_projects.clear();
                self.projects.clear();
                self.selected = None;
                self.status = ProjectListStatus::Error(err.to_string());
                return Err(err);
            }
        };
        apply_visibility_preferences(&mut projects, visibility);
        sort_projects(&mut projects);
        self.all_projects = projects;
        self.refresh_visible_projects(None);
        self.status = if self.projects.is_empty() {
            ProjectListStatus::Empty
        } else {
            ProjectListStatus::Ready
        };
        Ok(())
    }

    pub fn from_projects(projects: Vec<Project>) -> Self {
        Self::from_projects_with_visibility(projects, &[])
    }

    pub fn from_projects_with_visibility(
        mut projects: Vec<Project>,
        visibility: &[ProjectVisibilityConfig],
    ) -> Self {
        apply_visibility_preferences(&mut projects, visibility);
        sort_projects(&mut projects);
        let mut state = Self {
            all_projects: projects,
            projects: Vec::new(),
            selected: None,
            status: ProjectListStatus::Idle,
            show_hidden: false,
        };
        state.refresh_visible_projects(None);
        state.status = if state.projects.is_empty() {
            ProjectListStatus::Empty
        } else {
            ProjectListStatus::Ready
        };
        state
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

    pub fn hidden_visible(&self) -> bool {
        self.show_hidden
    }

    pub fn hidden_count(&self) -> usize {
        self.all_projects.iter().filter(|project| project.hidden).count()
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

    pub fn page_up(&mut self) {
        self.move_up();
    }

    pub fn page_down(&mut self) {
        self.move_down();
    }

    pub fn toggle_hidden(&mut self) {
        let previous_selected_index = self.selected;
        let previous_selected_id = self.selected_project().map(|project| project.id.clone());
        self.show_hidden = !self.show_hidden;
        self.refresh_visible_projects(previous_selected_index.or(Some(0)));
        if let Some(previous_selected_id) = previous_selected_id {
            if let Some(index) = self
                .projects
                .iter()
                .position(|project| project.id == previous_selected_id)
            {
                self.selected = Some(index);
            }
        }
    }

    pub fn apply_action(&mut self, action: &Action) -> Option<AppCommand> {
        match action {
            Action::MoveUp => {
                self.move_up();
                None
            }
            Action::MoveDown => {
                self.move_down();
                None
            }
            Action::PageUp => {
                self.page_up();
                None
            }
            Action::PageDown => {
                self.page_down();
                None
            }
            Action::ToggleHidden => {
                self.toggle_hidden();
                None
            }
            other => other.as_app_command(),
        }
    }

    fn refresh_visible_projects(&mut self, previous_selected: Option<usize>) {
        let previous_selected_id = previous_selected
            .and_then(|index| self.projects.get(index))
            .map(|project| project.id.clone());

        self.projects = self
            .all_projects
            .iter()
            .filter(|project| self.show_hidden || !project.hidden)
            .cloned()
            .collect();

        if self.projects.is_empty() {
            self.selected = None;
            return;
        }

        if let Some(previous_selected_id) = previous_selected_id {
            if let Some(index) = self
                .projects
                .iter()
                .position(|project| project.id == previous_selected_id)
            {
                self.selected = Some(index);
                return;
            }
        }

        let index = previous_selected.unwrap_or(0).min(self.projects.len() - 1);
        self.selected = Some(index);
    }
}

fn sort_projects(projects: &mut [Project]) {
    projects.sort_by(|left, right| {
        left.hidden
            .cmp(&right.hidden)
            .then_with(|| right.starred.cmp(&left.starred))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn apply_visibility_preferences(
    projects: &mut [Project],
    visibility: &[ProjectVisibilityConfig],
) {
    for project in projects.iter_mut() {
        if let Some(preference) = visibility.iter().find(|preference| preference.gid == project.id) {
            project.starred = preference.starred;
            project.hidden = preference.hidden;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        asana::{fake::FakeAsanaClient, AsanaClient},
        config::ProjectVisibilityConfig,
        domain::Project,
        error::{Error, Result},
        input::{Action, AppCommand},
    };

    use super::{ProjectListState, ProjectListStatus};

    struct FailingAsanaClient;

    impl AsanaClient for FailingAsanaClient {
        fn list_projects(&self) -> Result<Vec<Project>> {
            Err(Error::Backend("backend unavailable".to_string()))
        }
    }

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
    fn sorts_hidden_projects_after_visible_projects() {
        let state = ProjectListState::from_projects_with_visibility(
            vec![
                Project::new("1", "Visible Starred", false),
                Project::new("2", "Hidden Starred", false),
                Project::new("3", "Visible Unstarred", false),
            ],
            &[
                ProjectVisibilityConfig {
                    gid: "1".to_string(),
                    starred: true,
                    hidden: false,
                },
                ProjectVisibilityConfig {
                    gid: "2".to_string(),
                    starred: true,
                    hidden: true,
                },
            ],
        );

        let names: Vec<_> = state.items().iter().map(|project| project.name.as_str()).collect();

        assert_eq!(names, vec!["Visible Starred", "Visible Unstarred"]);
        assert_eq!(state.hidden_count(), 1);
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

    #[test]
    fn applies_navigation_actions_and_app_commands() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
        ]);

        assert_eq!(state.apply_action(&Action::MoveDown), None);
        assert_eq!(state.selected_index(), Some(1));
        assert_eq!(state.apply_action(&Action::PageUp), None);
        assert_eq!(state.selected_index(), Some(0));
        assert_eq!(state.apply_action(&Action::Quit), Some(AppCommand::Quit));
        assert_eq!(state.apply_action(&Action::Refresh), Some(AppCommand::Refresh));
        assert_eq!(state.apply_action(&Action::ToggleHidden), None);
    }

    #[test]
    fn records_error_state_when_backend_fails() {
        let mut state = ProjectListState::new();
        let err = state.load(&FailingAsanaClient).expect_err("load should fail");

        assert_eq!(err.to_string(), "backend error: backend unavailable");
        assert_eq!(
            state.status(),
            &ProjectListStatus::Error("backend error: backend unavailable".to_string())
        );
        assert!(state.items().is_empty());
        assert_eq!(state.selected_index(), None);
    }

    #[test]
    fn toggles_hidden_projects_into_view() {
        let mut state = ProjectListState::from_projects_with_visibility(
            vec![
                Project::new("1", "Visible", false),
                Project::new("2", "Hidden", false),
            ],
            &[ProjectVisibilityConfig {
                gid: "2".to_string(),
                starred: false,
                hidden: true,
            }],
        );

        assert_eq!(state.items().len(), 1);
        assert!(!state.hidden_visible());

        state.toggle_hidden();

        let names: Vec<_> = state.items().iter().map(|project| project.name.as_str()).collect();
        assert_eq!(names, vec!["Visible", "Hidden"]);
        assert!(state.hidden_visible());
    }
}
