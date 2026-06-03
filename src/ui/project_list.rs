use crate::app::project_list::{ProjectListState, ProjectListStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectListView {
    pub title: String,
    pub status_line: String,
    pub rows: Vec<String>,
}

pub fn render_project_list(state: &ProjectListState) -> ProjectListView {
    let status_line = match state.status() {
        ProjectListStatus::Idle => "Project list idle".to_string(),
        ProjectListStatus::Loading => "Loading projects...".to_string(),
        ProjectListStatus::Ready => format!("{} project(s)", state.items().len()),
        ProjectListStatus::Empty => "No projects".to_string(),
        ProjectListStatus::Error(message) => format!("Error: {message}"),
    };

    let rows = state
        .items()
        .iter()
        .enumerate()
        .map(|(index, project)| {
            let selected = if state.selected_index() == Some(index) { ">" } else { " " };
            let starred = if project.starred { "*" } else { " " };
            format!("{selected} [{starred}] {}", project.name)
        })
        .collect();

    ProjectListView {
        title: "Projects".to_string(),
        status_line,
        rows,
    }
}

#[cfg(test)]
mod tests {
    use crate::{app::project_list::ProjectListState, domain::Project};

    use super::render_project_list;

    #[test]
    fn renders_selection_and_star_state() {
        let state = ProjectListState::from_projects(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
        ]);

        let view = render_project_list(&state);

        assert_eq!(view.title, "Projects");
        assert_eq!(view.status_line, "2 project(s)");
        assert_eq!(view.rows, vec!["> [*] Inbox", "  [ ] Backlog"]);
    }

    #[test]
    fn renders_empty_state() {
        let state = ProjectListState::from_projects(vec![]);

        let view = render_project_list(&state);

        assert_eq!(view.status_line, "No projects");
        assert!(view.rows.is_empty());
    }
}
