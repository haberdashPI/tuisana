use std::collections::HashSet;

use regex::RegexBuilder;

use crate::{
    asana::AsanaClient,
    config::ProjectVisibilityConfig,
    domain::Project,
    error::Result,
    input::{Action, AppCommand},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchMode {
    Fuzzy,
    #[default]
    Substring,
    Regex,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectListState {
    all_projects: Vec<Project>,
    visible_projects: Vec<Project>,
    selected_ids: HashSet<String>,
    undo_selection_history: Vec<SelectionSnapshot>,
    redo_selection_history: Vec<SelectionSnapshot>,
    selected: Option<usize>,
    status: ProjectListStatus,
    show_hidden: bool,
    show_selected_only: bool,
    search_mode: SearchMode,
    search_query: String,
    search_active: bool,
    search_error: Option<String>,
    show_help_details: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectListStatus {
    Idle,
    Loading,
    Ready,
    Empty,
    Error(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SelectionSnapshot {
    selected_ids: HashSet<String>,
    focused_project_id: Option<String>,
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
                self.visible_projects.clear();
                self.selected_ids.clear();
                self.undo_selection_history.clear();
                self.redo_selection_history.clear();
                self.selected = None;
                self.status = ProjectListStatus::Error(err.to_string());
                return Err(err);
            }
        };
        apply_visibility_preferences(&mut projects, visibility);
        sort_projects(&mut projects);
        self.all_projects = projects;
        self.selected_ids.clear();
        self.undo_selection_history.clear();
        self.redo_selection_history.clear();
        self.selected = None;
        self.rebuild_visible_projects(None);
        self.status = if self.all_projects.is_empty() {
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
            visible_projects: Vec::new(),
            selected_ids: HashSet::new(),
            undo_selection_history: Vec::new(),
            redo_selection_history: Vec::new(),
            selected: None,
            status: ProjectListStatus::Idle,
            show_hidden: false,
            show_selected_only: false,
            search_mode: SearchMode::default(),
            search_query: String::new(),
            search_active: false,
            search_error: None,
            show_help_details: false,
        };
        state.rebuild_visible_projects(None);
        state.status = if state.all_projects.is_empty() {
            ProjectListStatus::Empty
        } else {
            ProjectListStatus::Ready
        };
        state
    }

    pub fn items(&self) -> &[Project] {
        &self.visible_projects
    }

    pub fn selected_projects(&self) -> Vec<Project> {
        self.all_projects
            .iter()
            .filter(|project| self.selected_ids.contains(&project.id))
            .cloned()
            .collect()
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_project(&self) -> Option<&Project> {
        self.selected
            .and_then(|index| self.visible_projects.get(index))
    }

    pub fn selected_count(&self) -> usize {
        self.selected_ids.len()
    }

    pub fn is_selected(&self, project_id: &str) -> bool {
        self.selected_ids.contains(project_id)
    }

    pub fn can_undo_selection(&self) -> bool {
        !self.undo_selection_history.is_empty()
    }

    pub fn can_redo_selection(&self) -> bool {
        !self.redo_selection_history.is_empty()
    }

    pub fn status(&self) -> &ProjectListStatus {
        &self.status
    }

    pub fn hidden_visible(&self) -> bool {
        self.show_hidden
    }

    pub fn show_selected_only(&self) -> bool {
        self.show_selected_only
    }

    pub fn search_mode(&self) -> SearchMode {
        self.search_mode
    }

    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    pub fn search_active(&self) -> bool {
        self.search_active
    }

    pub fn search_error(&self) -> Option<&str> {
        self.search_error.as_deref()
    }

    pub fn help_details_visible(&self) -> bool {
        self.show_help_details
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
            if index + 1 < self.visible_projects.len() {
                self.selected = Some(index + 1);
            }
        } else if !self.visible_projects.is_empty() {
            self.selected = Some(0);
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        let step = page_size.max(1);
        match self.selected {
            Some(0) | None => {}
            Some(index) => self.selected = Some(index.saturating_sub(step)),
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        let step = page_size.max(1);
        if let Some(index) = self.selected {
            if index + 1 < self.visible_projects.len() {
                self.selected = Some((index + step).min(self.visible_projects.len() - 1));
            }
        } else if !self.visible_projects.is_empty() {
            self.selected = Some(0);
        }
    }

    pub fn jump_top(&mut self) {
        if self.visible_projects.is_empty() {
            self.selected = None;
        } else {
            self.selected = Some(0);
        }
    }

    pub fn jump_bottom(&mut self) {
        if self.visible_projects.is_empty() {
            self.selected = None;
        } else {
            self.selected = Some(self.visible_projects.len() - 1);
        }
    }

    pub fn toggle_hidden_group(&mut self) {
        let cursor_id = self.selected_project().map(|project| project.id.clone());
        self.show_hidden = !self.show_hidden;
        self.rebuild_visible_projects(cursor_id);
    }

    pub fn toggle_help_details(&mut self) {
        self.show_help_details = !self.show_help_details;
    }

    pub fn toggle_selected_only(&mut self) {
        let cursor_id = self.selected_project().map(|project| project.id.clone());
        self.show_selected_only = !self.show_selected_only;
        self.rebuild_visible_projects(cursor_id);
    }

    pub fn clear_search(&mut self) {
        let cursor_id = self.selected_project().map(|project| project.id.clone());
        self.search_query.clear();
        self.search_error = None;
        self.search_active = false;
        self.rebuild_visible_projects(cursor_id);
    }

    pub fn set_search_mode(&mut self, mode: SearchMode) {
        let cursor_id = self.selected_project().map(|project| project.id.clone());
        self.search_mode = mode;
        self.search_error = None;
        self.rebuild_visible_projects(cursor_id);
    }

    pub fn start_search(&mut self) {
        self.search_active = true;
    }

    pub fn end_search(&mut self) {
        self.search_active = false;
    }

    pub fn push_search_char(&mut self, ch: char) {
        self.search_query.push(ch);
        let cursor_id = self.selected_project().map(|project| project.id.clone());
        self.rebuild_visible_projects(cursor_id);
    }

    pub fn pop_search_char(&mut self) {
        self.search_query.pop();
        let cursor_id = self.selected_project().map(|project| project.id.clone());
        self.rebuild_visible_projects(cursor_id);
    }

    pub fn toggle_current_selection(&mut self) {
        if let Some(project) = self.selected_project().cloned() {
            self.toggle_selection_for_project_id(&project.id);
        }
    }

    pub fn clear_selection(&mut self) {
        if self.selected_ids.is_empty() {
            return;
        }

        self.push_selection_history();
        self.selected_ids.clear();
        self.rebuild_visible_projects(self.selected_project().map(|project| project.id.clone()));
    }

    pub fn select_all_visible(&mut self) {
        if self.visible_projects.is_empty() {
            return;
        }

        self.push_selection_history();
        for project in &self.visible_projects {
            self.selected_ids.insert(project.id.clone());
        }
        self.rebuild_visible_projects(self.selected_project().map(|project| project.id.clone()));
    }

    pub fn invert_selection_visible(&mut self) {
        if self.visible_projects.is_empty() {
            return;
        }

        self.push_selection_history();
        for project in &self.visible_projects {
            if !self.selected_ids.insert(project.id.clone()) {
                self.selected_ids.remove(&project.id);
            }
        }
        self.rebuild_visible_projects(self.selected_project().map(|project| project.id.clone()));
    }

    pub fn undo_selection(&mut self) {
        if let Some(snapshot) = self.undo_selection_history.pop() {
            let current = self.capture_selection_snapshot();
            self.redo_selection_history.push(current);
            self.restore_selection_snapshot(snapshot);
        }
    }

    pub fn redo_selection(&mut self) {
        if let Some(snapshot) = self.redo_selection_history.pop() {
            let current = self.capture_selection_snapshot();
            self.undo_selection_history.push(current);
            self.restore_selection_snapshot(snapshot);
        }
    }

    pub fn toggle_starred_selected(&mut self) {
        self.toggle_project_flags(ProjectFlag::Starred);
    }

    pub fn toggle_hidden_selected(&mut self) {
        self.toggle_project_flags(ProjectFlag::Hidden);
    }

    pub fn apply_action(&mut self, action: &Action, page_size: usize) -> Option<AppCommand> {
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
                self.page_up(page_size);
                None
            }
            Action::PageDown => {
                self.page_down(page_size);
                None
            }
            Action::ToggleHiddenGroup => {
                self.toggle_hidden_group();
                None
            }
            Action::ToggleHelpDetails => {
                self.toggle_help_details();
                None
            }
            Action::ToggleSelection => {
                self.toggle_current_selection();
                None
            }
            Action::ToggleOnlySelected => {
                self.toggle_selected_only();
                None
            }
            Action::ClearSelection => {
                self.clear_selection();
                None
            }
            Action::SelectAllVisible => {
                self.select_all_visible();
                None
            }
            Action::InvertSelection => {
                self.invert_selection_visible();
                None
            }
            Action::UndoSelection => {
                self.undo_selection();
                None
            }
            Action::RedoSelection => {
                self.redo_selection();
                None
            }
            Action::ToggleStarredSelected => {
                self.toggle_starred_selected();
                None
            }
            Action::ToggleHiddenSelected => {
                self.toggle_hidden_selected();
                None
            }
            Action::StartSearch => {
                self.start_search();
                None
            }
            Action::ClearSearch => {
                self.clear_search();
                None
            }
            Action::SearchFuzzy => {
                self.set_search_mode(SearchMode::Fuzzy);
                None
            }
            Action::SearchSubstring => {
                self.set_search_mode(SearchMode::Substring);
                None
            }
            Action::SearchRegex => {
                self.set_search_mode(SearchMode::Regex);
                None
            }
            Action::JumpTop => {
                self.jump_top();
                None
            }
            Action::JumpBottom => {
                self.jump_bottom();
                None
            }
            other => other.as_app_command(),
        }
    }

    pub fn visibility_preferences(&self) -> Vec<ProjectVisibilityConfig> {
        let mut preferences: Vec<ProjectVisibilityConfig> = self
            .all_projects
            .iter()
            .filter(|project| project.starred || project.hidden)
            .map(|project| ProjectVisibilityConfig {
                gid: project.id.clone(),
                starred: project.starred,
                hidden: project.hidden,
            })
            .collect();
        preferences.sort_by(|left, right| left.gid.cmp(&right.gid));
        preferences
    }

    fn toggle_selection_for_project_id(&mut self, project_id: &str) {
        self.push_selection_history();
        if !self.selected_ids.insert(project_id.to_string()) {
            self.selected_ids.remove(project_id);
        }
        let cursor_id = self.selected_project().map(|project| project.id.clone());
        self.rebuild_visible_projects(cursor_id);
    }

    fn toggle_project_flags(&mut self, flag: ProjectFlag) {
        let target_ids = self.action_target_ids();
        if target_ids.is_empty() {
            return;
        }

        let cursor_id = self.selected_project().map(|project| project.id.clone());
        for target_id in target_ids {
            if let Some(project) = self.all_projects.iter_mut().find(|project| project.id == target_id)
            {
                match flag {
                    ProjectFlag::Starred => project.starred = !project.starred,
                    ProjectFlag::Hidden => project.hidden = !project.hidden,
                }
            }
        }

        sort_projects(&mut self.all_projects);
        self.rebuild_visible_projects(cursor_id);
    }

    fn action_target_ids(&self) -> Vec<String> {
        if !self.selected_ids.is_empty() {
            let mut ids: Vec<String> = self.selected_ids.iter().cloned().collect();
            ids.sort();
            ids
        } else {
            self.selected_project()
                .map(|project| vec![project.id.clone()])
                .unwrap_or_default()
        }
    }

    fn rebuild_visible_projects(&mut self, previous_cursor_id: Option<String>) {
        let previous_cursor_index = self.selected;
        self.search_error = None;

        let regex = if self.search_mode == SearchMode::Regex && !self.search_query.is_empty() {
            match RegexBuilder::new(&self.search_query)
                .case_insensitive(true)
                .build()
            {
                Ok(regex) => Some(regex),
                Err(err) => {
                    self.search_error = Some(err.to_string());
                    None
                }
            }
        } else {
            None
        };

        self.visible_projects = self
            .all_projects
            .iter()
            .filter(|project| self.project_matches(project, regex.as_ref()))
            .cloned()
            .collect();

        if self.visible_projects.is_empty() {
            self.selected = None;
            return;
        }

        if let Some(previous_cursor_id) = previous_cursor_id {
            if let Some(index) = self
                .visible_projects
                .iter()
                .position(|project| project.id == previous_cursor_id)
            {
                self.selected = Some(index);
                return;
            }
        }

        let index = previous_cursor_index
            .unwrap_or(0)
            .min(self.visible_projects.len() - 1);
        self.selected = Some(index);
    }

    fn project_matches(&self, project: &Project, regex: Option<&regex::Regex>) -> bool {
        if !self.show_hidden && project.hidden {
            return false;
        }

        if self.show_selected_only && !self.selected_ids.contains(&project.id) {
            return false;
        }

        if self.search_query.is_empty() {
            return true;
        }

        let haystack = format!("{} {}", project.name, project.id).to_ascii_lowercase();
        let query = self.search_query.to_ascii_lowercase();

        match self.search_mode {
            SearchMode::Fuzzy => fuzzy_match(&haystack, &query),
            SearchMode::Substring => haystack.contains(&query),
            SearchMode::Regex => regex.is_some_and(|regex| regex.is_match(&haystack)),
        }
    }

    fn push_selection_history(&mut self) {
        self.undo_selection_history
            .push(self.capture_selection_snapshot());
        self.redo_selection_history.clear();
    }

    fn capture_selection_snapshot(&self) -> SelectionSnapshot {
        SelectionSnapshot {
            selected_ids: self.selected_ids.clone(),
            focused_project_id: self.selected_project().map(|project| project.id.clone()),
        }
    }

    fn restore_selection_snapshot(&mut self, snapshot: SelectionSnapshot) {
        let focused_project_id = snapshot.focused_project_id.clone();
        self.selected_ids = snapshot.selected_ids;
        self.rebuild_visible_projects(focused_project_id);
    }
}

#[derive(Clone, Copy)]
enum ProjectFlag {
    Starred,
    Hidden,
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

fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let mut needle_chars = needle.chars();
    let mut current = needle_chars.next();

    if current.is_none() {
        return true;
    }

    for candidate in haystack.chars() {
        if Some(candidate) == current {
            current = needle_chars.next();
            if current.is_none() {
                return true;
            }
        }
    }

    false
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

    use super::{ProjectListState, ProjectListStatus, SearchMode};

    struct FailingAsanaClient;

    impl AsanaClient for FailingAsanaClient {
        fn list_projects(&self) -> Result<Vec<Project>> {
            Err(Error::Backend("backend unavailable".to_string()))
        }

        fn list_tasks(&self, _project_gid: &str) -> Result<Vec<crate::asana::dto::TaskDto>> {
            Err(Error::Backend("backend unavailable".to_string()))
        }

        fn list_sections(&self, _project_gid: &str) -> Result<Vec<crate::asana::dto::SectionDto>> {
            Err(Error::Backend("backend unavailable".to_string()))
        }

        fn list_project_custom_field_settings(
            &self,
            _project_gid: &str,
        ) -> Result<Vec<crate::asana::dto::ProjectCustomFieldSettingDto>> {
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

        assert_eq!(state.apply_action(&Action::MoveDown, 5), None);
        assert_eq!(state.selected_index(), Some(1));
        assert_eq!(state.apply_action(&Action::PageUp, 5), None);
        assert_eq!(state.selected_index(), Some(0));
        assert_eq!(state.apply_action(&Action::Quit, 5), Some(AppCommand::Quit));
        assert_eq!(state.apply_action(&Action::Refresh, 5), Some(AppCommand::Refresh));
        assert_eq!(state.apply_action(&Action::ToggleSelection, 5), None);
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
    fn toggles_selection_and_selection_only_filter() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
            Project::new("3", "Roadmap", false),
        ]);

        state.toggle_current_selection();
        assert_eq!(state.selected_count(), 1);
        state.move_down();
        state.toggle_current_selection();
        assert_eq!(state.selected_count(), 2);

        state.toggle_selected_only();

        let names: Vec<_> = state.items().iter().map(|project| project.name.as_str()).collect();
        assert_eq!(names, vec!["Inbox", "Backlog"]);
        assert_eq!(state.selected_index(), Some(1));
    }

    #[test]
    fn pages_and_jumps_by_page_size() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Alpha", false),
            Project::new("2", "Beta", false),
            Project::new("3", "Gamma", false),
        ]);

        state.page_down(2);
        assert_eq!(state.selected_index(), Some(2));
        state.page_up(2);
        assert_eq!(state.selected_index(), Some(0));
        state.jump_bottom();
        assert_eq!(state.selected_index(), Some(2));
        state.jump_top();
        assert_eq!(state.selected_index(), Some(0));
    }

    #[test]
    fn filters_projects_by_search_mode() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
            Project::new("3", "Roadmap", false),
        ]);

        state.set_search_mode(SearchMode::Substring);
        state.push_search_char('b');
        state.push_search_char('a');
        state.push_search_char('c');
        state.push_search_char('k');
        let names: Vec<_> = state.items().iter().map(|project| project.name.as_str()).collect();
        assert_eq!(names, vec!["Backlog"]);

        state.pop_search_char();
        state.pop_search_char();
        state.pop_search_char();
        state.pop_search_char();
        state.set_search_mode(SearchMode::Fuzzy);
        state.push_search_char('r');
        state.push_search_char('d');
        let names: Vec<_> = state.items().iter().map(|project| project.name.as_str()).collect();
        assert_eq!(names, vec!["Roadmap"]);
    }

    #[test]
    fn clears_search_and_restores_all_items() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
            Project::new("3", "Roadmap", false),
        ]);

        state.start_search();
        state.push_search_char('b');
        state.push_search_char('a');
        state.push_search_char('c');
        state.push_search_char('k');
        assert_eq!(state.items().len(), 1);

        state.clear_search();

        assert_eq!(state.items().len(), 3);
        assert!(!state.search_active());
        assert!(state.search_query().is_empty());
    }

    #[test]
    fn undoes_and_redoes_selection_changes() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Alpha", false),
            Project::new("2", "Beta", false),
            Project::new("3", "Gamma", false),
        ]);

        state.toggle_current_selection();
        state.move_down();
        state.toggle_current_selection();
        assert_eq!(state.selected_count(), 2);

        state.undo_selection();
        assert_eq!(state.selected_count(), 1);

        state.redo_selection();
        assert_eq!(state.selected_count(), 2);
    }

    #[test]
    fn selects_and_inverts_visible_projects() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Alpha", false),
            Project::new("2", "Beta", false),
            Project::new("3", "Gamma", false),
        ]);

        state.select_all_visible();
        assert_eq!(state.selected_count(), 3);

        state.invert_selection_visible();
        assert_eq!(state.selected_count(), 0);
    }

    #[test]
    fn rejects_invalid_regex_queries_without_losing_state() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
        ]);

        state.set_search_mode(SearchMode::Regex);
        state.push_search_char('[');

        assert!(state.items().is_empty());
        assert!(state.search_error().is_some());
        assert_eq!(state.selected_index(), None);
    }

    #[test]
    fn toggles_starred_and_hidden_selected_projects() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Inbox", false),
            Project::new("2", "Backlog", false),
        ]);

        state.move_down();
        state.toggle_current_selection();
        state.toggle_starred_selected();
        assert!(state
            .visibility_preferences()
            .iter()
            .any(|project| project.gid == "1" && project.starred));

        state.toggle_hidden_selected();
        assert!(state
            .visibility_preferences()
            .iter()
            .any(|project| project.gid == "1" && project.hidden));
    }
}
