use crate::app::project_list::{ProjectListState, ProjectListStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectListView {
    pub title: String,
    pub status_line: String,
    pub search_line: String,
    pub hint_lines: Vec<String>,
    pub rows: Vec<String>,
}

pub fn render_project_list(state: &ProjectListState) -> ProjectListView {
    let status_line = build_status_line(state);
    let search_line = build_search_line(state);
    let hint_lines = build_hint_lines(state);

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
        search_line,
        hint_lines,
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
        assert_eq!(view.search_line, "Search: not searching (substring)");
        assert_eq!(view.hint_lines, vec!["?: more hints, j/k: move, space: select, /: search, r: refresh, q: quit"]);
        assert_eq!(view.rows, vec!["[ ] [*] Inbox", "[ ] [ ] Backlog"]);
    }

    #[test]
    fn renders_empty_state() {
        let state = ProjectListState::from_projects(vec![]);

        let view = render_project_list(&state);

        assert_eq!(view.status_line, "No projects");
        assert_eq!(view.search_line, "Search: not searching (substring)");
        assert_eq!(view.hint_lines, vec!["?: more hints, j/k: move, space: select, /: search, r: refresh, q: quit"]);
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
        assert_eq!(view.search_line, "Search: not searching (substring)");
        assert_eq!(view.hint_lines, vec!["?: more hints, j/k: move, space: select, /: search, r: refresh, q: quit"]);
        assert_eq!(view.rows, vec!["[ ] [ ] Visible"]);
    }

    #[test]
    fn renders_contextual_selection_commands_when_items_are_selected() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Alpha", false),
            Project::new("2", "Beta", false),
        ]);

        state.toggle_current_selection();
        state.toggle_help_details();

        let view = render_project_list(&state);

        assert!(view.hint_lines.iter().any(|line| line.contains("c: clear selection")));
        assert!(view.hint_lines.iter().any(|line| line.contains("a: select all visible")));
        assert!(view.hint_lines.iter().any(|line| line.contains("i: invert selection")));
        assert!(view.hint_lines.iter().any(|line| line.contains("u: undo")));
    }

    #[test]
    fn renders_expanded_help_with_toggle_at_front() {
        let mut state = ProjectListState::from_projects(vec![Project::new("1", "Alpha", false)]);
        state.toggle_help_details();

        let view = render_project_list(&state);

        assert_eq!(view.hint_lines[0], "?: fewer hints");
        assert!(view.hint_lines.len() > 1);
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

fn build_search_line(state: &ProjectListState) -> String {
    let mode = match state.search_mode() {
        crate::app::project_list::SearchMode::Fuzzy => "fuzzy",
        crate::app::project_list::SearchMode::Substring => "substring",
        crate::app::project_list::SearchMode::Regex => "regex",
    };

    if state.search_active() && state.search_query().is_empty() {
        return format!("Search: awaiting input ({mode})");
    }

    if state.search_query().is_empty() {
        return format!("Search: not searching ({mode})");
    }

    format!("Search: {mode} \"{}\"", state.search_query())
}

fn build_hint_lines(state: &ProjectListState) -> Vec<String> {
    if !state.help_details_visible() {
        return vec!["?: more hints, j/k: move, space: select, /: search, r: refresh, q: quit".to_string()];
    }

    let mut lines = vec![
        "?: fewer hints".to_string(),
        "j/down: move down, k/up: move up, ctrl-u: page up, ctrl-d: page down, home: top, end: bottom"
            .to_string(),
        "space: select".to_string(),
    ];

    if state.selected_count() > 0 {
        lines.push("a: select all visible, i: invert selection, c: clear selection".to_string());
    }

    let mut selection_line = vec!["*: star selected".to_string(), "h: hide selected".to_string()];
    selection_line.push("v: toggle hidden projects".to_string());
    selection_line.push("o: only selected".to_string());
    if state.can_undo_selection() {
        selection_line.push("u: undo".to_string());
    }
    if state.can_redo_selection() {
        selection_line.push("ctrl-y: redo".to_string());
    }
    lines.push(selection_line.join(", "));

    let mut search_line = vec!["/: search".to_string()];
    if state.search_active() || !state.search_query().is_empty() {
        search_line.push("ctrl-l: clear search".to_string());
    }
    search_line.push("ctrl-f: fuzzy".to_string());
    search_line.push("ctrl-s: substring".to_string());
    search_line.push("ctrl-r: regex".to_string());
    search_line.push("r: refresh".to_string());
    search_line.push("q/ctrl-c: quit".to_string());
    lines.push(search_line.join(", "));

    lines
}
