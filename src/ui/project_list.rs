use crate::app::project_list::{ProjectListState, ProjectListStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectListView {
    pub title: String,
    pub status_line: String,
    pub hint_line: String,
    pub rows: Vec<String>,
}

pub fn render_project_list(state: &ProjectListState) -> ProjectListView {
    let status_line = build_status_line(state);
    let hint_line = build_hint_line(state);

    let rows = state
        .items()
        .iter()
        .map(|project| {
            let selected = if state.is_selected(&project.id) { "[x]" } else { "[ ]" };
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
        assert_eq!(view.status_line, "2 visible, 0 selected");
        assert_eq!(
            view.hint_line,
            "space: select, *: star selected, h: hide selected, v: toggle hidden projects, o: only selected, /: search, ctrl-f: fuzzy, ctrl-s: substring, ctrl-r: regex, r: refresh, q/ctrl-c: quit"
        );
        assert_eq!(view.rows, vec!["[ ] [*] Inbox", "[ ] [ ] Backlog"]);
    }

    #[test]
    fn renders_empty_state() {
        let state = ProjectListState::from_projects(vec![]);

        let view = render_project_list(&state);

        assert_eq!(view.status_line, "No projects");
        assert_eq!(
            view.hint_line,
            "space: select, *: star selected, h: hide selected, v: toggle hidden projects, o: only selected, /: search, ctrl-f: fuzzy, ctrl-s: substring, ctrl-r: regex, r: refresh, q/ctrl-c: quit"
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

        assert_eq!(view.status_line, "1 visible, 0 selected, 1 hidden");
        assert_eq!(
            view.hint_line,
            "space: select, *: star selected, h: hide selected, v: toggle hidden projects, o: only selected, /: search, ctrl-f: fuzzy, ctrl-s: substring, ctrl-r: regex, r: refresh, q/ctrl-c: quit"
        );
        assert_eq!(view.rows, vec!["[ ] [ ] Visible"]);
    }
}

fn build_status_line(state: &ProjectListState) -> String {
    if let Some(error) = state.search_error() {
        return format!("Search error: {error}");
    }

    match state.status() {
        ProjectListStatus::Idle => "Project list idle".to_string(),
        ProjectListStatus::Loading => "Loading projects...".to_string(),
        ProjectListStatus::Error(message) => format!("Error: {message}"),
        ProjectListStatus::Empty => "No projects".to_string(),
        ProjectListStatus::Ready => {
            if state.items().is_empty() {
                if !state.search_query().is_empty() {
                    return format!("No projects match '{}'", state.search_query());
                }

                return "No projects match current filter".to_string();
            }

            let mut parts = vec![
                format!("{} visible", state.items().len()),
                format!("{} selected", state.selected_count()),
            ];

            if state.hidden_count() > 0 {
                parts.push(format!("{} hidden", state.hidden_count()));
            }

            if !state.search_query().is_empty() {
                parts.push(format!(
                    "query {}: {}",
                    match state.search_mode() {
                        crate::app::project_list::SearchMode::Fuzzy => "fuzzy",
                        crate::app::project_list::SearchMode::Substring => "substring",
                        crate::app::project_list::SearchMode::Regex => "regex",
                    },
                    state.search_query()
                ));
            }

            if state.show_selected_only() {
                parts.push("selected only".to_string());
            }

            parts.join(", ")
        }
    }
}

fn build_hint_line(state: &ProjectListState) -> String {
    let search_mode_hint = if state.search_active() {
        format!("search: {} (enter/esc/backspace)", state.search_query())
    } else {
        String::new()
    };

    let base = "space: select, *: star selected, h: hide selected, v: toggle hidden projects, o: only selected, /: search, ctrl-f: fuzzy, ctrl-s: substring, ctrl-r: regex, r: refresh, q/ctrl-c: quit";
    if search_mode_hint.is_empty() {
        base.to_string()
    } else {
        format!("{base} | {search_mode_hint}")
    }
}
