//! Rendering helpers for the project list pane.
//!
//! A project row carries four independent facts — cursor, multi-select, starred,
//! hidden. The old rendering spent three ASCII bracket groups on them
//! (`[ ] [*] [hidden] Name`) before the reader reached the name. Each fact now
//! owns one glyph cell in a fixed gutter, so the names line up and the markers
//! read as columns.

use crate::{
    app::project_list::{ProjectListState, ProjectListStatus, SearchMode},
    domain::ProjectKind,
    ui::{
        chrome::{Chip, PaneMessage, Tone},
        text::{pad_cell, slice_spans, visible_width},
        theme::Theme,
    },
};

use ratatui::text::{Line, Span};

/// Width of the marker gutter: selection glyph, gap, star glyph, gap.
pub const MARKER_WIDTH: usize = 4;

/// One project row, as facts rather than pre-formatted text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRow {
    /// The project name.
    pub name: String,
    /// Whether the project is in the multi-select set.
    pub selected: bool,
    /// Whether the project is starred.
    pub starred: bool,
    /// Whether the project is marked hidden.
    pub hidden: bool,
    /// Whether this is the pinned "assigned to me" row.
    pub pinned: bool,
}

/// The active project search, shown only while it is doing something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchLine {
    /// The current query.
    pub query: String,
    /// Which matching strategy is active.
    pub mode: &'static str,
    /// Whether the search is accepting input but has no query yet.
    pub awaiting: bool,
    /// A regex compilation error, if the query is invalid.
    pub error: Option<String>,
}

/// Snapshot of the project list used by the UI renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectListView {
    /// The pane title.
    pub title: String,
    /// Counts shown on the right of the pane border.
    pub counts: Vec<Chip>,
    /// One entry per visible project.
    pub rows: Vec<ProjectRow>,
    /// The search footer, when a search is active or has a query.
    pub search: Option<SearchLine>,
    /// Set when there are no rows to show, explaining why.
    pub message: Option<PaneMessage>,
}

/// Renders the project list state into a UI-friendly snapshot.
pub fn render_project_list(state: &ProjectListState) -> ProjectListView {
    let rows = state
        .items()
        .iter()
        .map(|project| ProjectRow {
            name: project.name.clone(),
            selected: state.is_selected(&project.id),
            starred: project.starred,
            hidden: project.hidden,
            pinned: matches!(project.kind, ProjectKind::AssignedToMe),
        })
        .collect::<Vec<_>>();

    ProjectListView {
        title: "Projects".to_string(),
        counts: counts(state),
        message: message(state, rows.is_empty()),
        rows,
        search: search_line(state),
    }
}

/// Renders one project row, right-aligning the hidden marker.
pub fn project_row_line(row: &ProjectRow, theme: &Theme, width: usize) -> Line<'static> {
    let glyphs = &theme.glyphs;

    let (marker, marker_style) = if row.selected {
        (glyphs.selected, theme.marker)
    } else {
        (" ", theme.text)
    };

    // The pinned row is not a real project, so a star would be meaningless
    // there; it gets its own marker instead.
    let (badge, badge_style) = match (row.pinned, row.starred) {
        (true, _) => (glyphs.pinned, theme.info),
        (false, true) => (glyphs.star_on, theme.star),
        (false, false) => (glyphs.star_off, theme.muted),
    };

    let hidden_chip = if row.hidden { "hidden" } else { "" };
    let name_width = width
        .saturating_sub(MARKER_WIDTH)
        .saturating_sub(if hidden_chip.is_empty() {
            0
        } else {
            visible_width(hidden_chip) + 2
        });
    let name_style = if row.hidden { theme.hidden } else { theme.text };

    let mut spans = vec![
        Span::styled(marker.to_string(), marker_style),
        Span::raw(" "),
        Span::styled(badge.to_string(), badge_style),
        Span::raw(" "),
        Span::styled(pad_cell(&row.name, name_width, glyphs.ellipsis), name_style),
    ];

    if !hidden_chip.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(hidden_chip.to_string(), theme.hidden));
    }

    // The markers and the hidden chip have fixed widths, so on a very narrow
    // pane the finished run is sliced rather than merely padded.
    Line::from(slice_spans(&spans, 0, width))
}

/// Renders the search footer shown in the pane's bottom border.
pub fn search_footer_line(search: &SearchLine, theme: &Theme) -> Line<'static> {
    if let Some(error) = &search.error {
        return Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("/{}", search.query), theme.danger),
            Span::raw(" "),
            Span::styled(error.clone(), theme.danger),
            Span::raw(" "),
        ]);
    }

    let query = if search.awaiting {
        format!("/{}{}", search.query, theme.glyphs.edit_cursor)
    } else {
        format!("/{}", search.query)
    };

    Line::from(vec![
        Span::raw(" "),
        Span::styled(query, theme.accent),
        Span::raw("  "),
        Span::styled(search.mode.to_string(), theme.muted),
        Span::raw(" "),
    ])
}

fn counts(state: &ProjectListState) -> Vec<Chip> {
    if !matches!(state.status(), ProjectListStatus::Ready) || state.items().is_empty() {
        return Vec::new();
    }

    let mut chips = vec![Chip::new(format!("{} shown", state.items().len()))];

    if state.selected_count() > 0 {
        chips.push(Chip::toned(
            format!("{} selected", state.selected_count()),
            Tone::Accent,
        ));
    }
    if state.hidden_count() > 0 {
        chips.push(Chip::new(format!("{} hidden", state.hidden_count())));
    }
    if state.show_selected_only() {
        chips.push(Chip::toned("selected only", Tone::Warn));
    }

    chips
}

fn message(state: &ProjectListState, empty: bool) -> Option<PaneMessage> {
    if let Some(error) = state.search_error() {
        return Some(PaneMessage::new(
            format!("Invalid search: {error}"),
            Tone::Danger,
        ));
    }

    match state.status() {
        ProjectListStatus::Loading => Some(PaneMessage::new("Loading projects", Tone::Muted)),
        ProjectListStatus::Error(message) => {
            Some(PaneMessage::new(format!("Error: {message}"), Tone::Danger)
                .with_hint("r to retry"))
        }
        ProjectListStatus::Empty => {
            Some(PaneMessage::new("No projects", Tone::Muted).with_hint("r to refresh"))
        }
        ProjectListStatus::Idle if empty => Some(PaneMessage::new("Idle", Tone::Muted)),
        _ if empty && !state.search_query().is_empty() => Some(
            PaneMessage::new(
                format!("Nothing matches \"{}\"", state.search_query()),
                Tone::Muted,
            )
            .with_hint("ctrl-l to clear the search"),
        ),
        _ if empty => Some(
            PaneMessage::new("Every project is filtered out", Tone::Muted)
                .with_hint("v to show hidden projects"),
        ),
        _ => None,
    }
}

fn search_line(state: &ProjectListState) -> Option<SearchLine> {
    if !state.search_active() && state.search_query().is_empty() {
        return None;
    }

    Some(SearchLine {
        query: state.search_query().to_string(),
        mode: search_mode_label(state.search_mode()),
        awaiting: state.search_active(),
        error: state.search_error().map(|error| error.to_string()),
    })
}

fn search_mode_label(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Fuzzy => "fuzzy",
        SearchMode::Substring => "contains",
        SearchMode::Regex => "regex",
    }
}

#[cfg(test)]
mod tests {
    use super::{project_row_line, render_project_list, search_footer_line, MARKER_WIDTH};
    use crate::{
        app::project_list::ProjectListState,
        config::ProjectVisibilityConfig,
        domain::Project,
        ui::{text::visible_width, theme::Theme},
    };

    #[test]
    fn rows_carry_selection_star_and_hidden_state_as_facts() {
        let state = ProjectListState::from_projects(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
        ]);

        let view = render_project_list(&state);

        assert_eq!(view.title, "Projects");
        assert_eq!(view.rows.len(), 2);
        assert_eq!(view.rows[0].name, "Inbox");
        assert!(view.rows[0].starred);
        assert!(!view.rows[0].selected);
        assert!(!view.rows[1].starred);
        assert!(view.message.is_none());
        assert!(view.search.is_none());
    }

    #[test]
    fn counts_only_mention_selection_and_hidden_when_they_apply() {
        let mut state = ProjectListState::from_projects(vec![
            Project::new("1", "Inbox", true),
            Project::new("2", "Backlog", false),
        ]);

        let counts = render_project_list(&state)
            .counts
            .iter()
            .map(|chip| chip.text.clone())
            .collect::<Vec<_>>();
        assert_eq!(counts, vec!["2 shown".to_string()]);

        state.toggle_current_selection();
        let counts = render_project_list(&state)
            .counts
            .iter()
            .map(|chip| chip.text.clone())
            .collect::<Vec<_>>();
        assert_eq!(counts, vec!["2 shown".to_string(), "1 selected".to_string()]);
    }

    #[test]
    fn a_hidden_project_is_dimmed_and_labeled() {
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

        assert_eq!(view.rows.len(), 1);
        assert!(!view.rows[0].hidden);
        assert_eq!(
            view.counts
                .iter()
                .map(|chip| chip.text.clone())
                .collect::<Vec<_>>(),
            vec!["1 shown".to_string(), "1 hidden".to_string()]
        );
    }

    #[test]
    fn the_assigned_to_me_row_is_pinned_and_has_no_star() {
        let mut state = ProjectListState::from_projects(vec![Project::new("1", "Inbox", true)]);
        state.set_assigned_to_me(Some(Project::assigned_to_me("user_1")));

        let view = render_project_list(&state);

        assert!(view.rows[0].pinned);
        assert_eq!(view.rows[0].name, "No Project (Assigned to Me)");
        assert!(!view.rows[1].pinned);
    }

    #[test]
    fn a_row_line_is_exactly_the_requested_width() {
        let theme = Theme::default();
        let state = ProjectListState::from_projects(vec![Project::new(
            "1",
            "A project name that is far longer than the pane",
            true,
        )]);
        let view = render_project_list(&state);

        for width in [MARKER_WIDTH + 1, 20, 40, 100] {
            let line = project_row_line(&view.rows[0], &theme, width);
            assert_eq!(
                visible_width(&line.to_string()),
                width,
                "row line was not {width} cells wide"
            );
        }
    }

    #[test]
    fn an_empty_result_explains_itself_instead_of_rendering_a_blank_pane() {
        let mut state = ProjectListState::from_projects(vec![Project::new("1", "Inbox", false)]);
        state.start_search();
        state.push_search_char('z');
        state.push_search_char('z');

        let view = render_project_list(&state);

        assert!(view.rows.is_empty());
        let message = view.message.expect("a message explains the empty pane");
        assert!(message.text.contains("zz"));
        assert!(message.hint.is_some());
    }

    #[test]
    fn the_search_footer_appears_only_while_searching() {
        let theme = Theme::default();
        let mut state = ProjectListState::from_projects(vec![Project::new("1", "Inbox", false)]);

        assert!(render_project_list(&state).search.is_none());

        state.start_search();
        state.push_search_char('i');
        let search = render_project_list(&state)
            .search
            .expect("search footer is shown while searching");

        assert_eq!(search.query, "i");
        assert_eq!(search.mode, "contains");
        assert!(search_footer_line(&search, &theme).to_string().contains("/i"));
    }
}
