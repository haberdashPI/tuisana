use crate::app::project_list::{ProjectListState, ProjectListStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectListView {
    pub title: String,
    pub status_line: String,
    pub hint_line: String,
    pub rows: Vec<String>,
}

pub fn render_project_list(state: &ProjectListState) -> ProjectListView {
    let status_line = match state.status() {
        ProjectListStatus::Idle => "Project list idle".to_string(),
        ProjectListStatus::Loading => "Loading projects...".to_string(),
        ProjectListStatus::Ready => {
            let visible_count = state.items().len();
            let hidden_count = state.hidden_count();

            if hidden_count == 0 {
                format!("{visible_count} project(s)")
            } else if state.hidden_visible() {
                format!("{visible_count} project(s) including {hidden_count} hidden")
            } else {
                format!(
                    "{visible_count} project(s), {hidden_count} hidden (press h to show)"
                )
            }
        }
        ProjectListStatus::Empty => {
            let hidden_count = state.hidden_count();
            if hidden_count == 0 {
                "No projects".to_string()
            } else if state.hidden_visible() {
                format!("No visible projects, {hidden_count} hidden shown")
            } else {
                format!("No visible projects, {hidden_count} hidden (press h to show)")
            }
        }
        ProjectListStatus::Error(message) => format!("Error: {message}"),
    };

    let hint_line = if state.hidden_count() > 0 {
        if state.hidden_visible() {
            "h: hide hidden projects, j/down: move down, k/up: move up, r: refresh, q/ctrl-c: quit"
                .to_string()
        } else {
            "h: show hidden projects, j/down: move down, k/up: move up, r: refresh, q/ctrl-c: quit"
                .to_string()
        }
    } else {
        "j/down: move down, k/up: move up, r: refresh, q/ctrl-c: quit".to_string()
    };

    let rows = state
        .items()
        .iter()
        .enumerate()
        .map(|(index, project)| {
            let selected = if state.selected_index() == Some(index) { ">" } else { " " };
            let starred = if project.starred { "[*] " } else { "[ ] " };
            let hidden = if project.hidden { "[hidden] " } else { "" };
            format!("{selected} {starred}{hidden}{}", project.name)
        })
        .collect();

    ProjectListView {
        title: "Projects".to_string(),
        status_line,
        hint_line,
        rows,
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::project_list::ProjectListState,
        config::ProjectVisibilityConfig,
        domain::Project,
    };

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
        assert_eq!(
            view.hint_line,
            "j/down: move down, k/up: move up, r: refresh, q/ctrl-c: quit"
        );
        assert_eq!(view.rows, vec!["> [*] Inbox", "  [ ] Backlog"]);
    }

    #[test]
    fn renders_empty_state() {
        let state = ProjectListState::from_projects(vec![]);

        let view = render_project_list(&state);

        assert_eq!(view.status_line, "No projects");
        assert_eq!(
            view.hint_line,
            "j/down: move down, k/up: move up, r: refresh, q/ctrl-c: quit"
        );
        assert!(view.rows.is_empty());
    }

    #[test]
    fn renders_hidden_project_hint_and_marker() {
        let state = ProjectListState::from_projects_with_visibility(
            vec![
                Project::new("1", "Visible", false),
                Project::new("2", "Hidden", false),
            ],
            &[ProjectVisibilityConfig {
                gid: "2".to_string(),
                starred: true,
                hidden: true,
            }],
        );

        let view = render_project_list(&state);

        assert_eq!(
            view.status_line,
            "1 project(s), 1 hidden (press h to show)"
        );
        assert_eq!(
            view.hint_line,
            "h: show hidden projects, j/down: move down, k/up: move up, r: refresh, q/ctrl-c: quit"
        );
        assert_eq!(view.rows, vec!["> [ ] Visible"]);
    }
}
