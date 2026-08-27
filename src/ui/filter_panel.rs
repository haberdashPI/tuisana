//! Rendering for the task filter panel.
//!
//! The panel used to render as `> Title [string:fuzzy]:` with no alignment, no
//! way to tell an active filter from an empty one, and no cursor beyond a `>`.
//! It is now a three-column grid — label, match mode, value — with the gutter
//! carrying the cursor and an "this filter is doing something" marker.
//!
//! Fields stay in their keymap navigation order. They are deliberately *not*
//! regrouped by kind: `j`/`k` walk the list in index order, so any visual
//! reordering would make the cursor appear to skip around. The match-mode chip
//! in the second column carries the same information a grouping would.

use ratatui::text::{Line, Span};

use crate::{
    app::task::{TaskFilterPanelEntry, TaskState},
    ui::{
        chrome::{Chip, PaneMessage, Tone},
        text::{pad_cell, slice_spans, spans_width, truncate_with_ellipsis, visible_width},
        theme::Theme,
    },
};

/// Width of the marker gutter: cursor glyph, gap, active glyph, gap.
pub const MARKER_WIDTH: usize = 4;
/// Widest a field label column will grow to.
const LABEL_CAP: usize = 16;
/// Widest the match-mode column will grow to.
const KIND_CAP: usize = 9;

/// The value held by one filter field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterValue {
    /// A free-text or date query.
    Text(String),
    /// A set of chosen label values, with the edit cursor when focused.
    Labels {
        values: Vec<String>,
        cursor: Option<usize>,
    },
}

impl FilterValue {
    /// Whether this field is currently filtering anything out.
    pub fn is_active(&self) -> bool {
        match self {
            FilterValue::Text(query) => !query.trim().is_empty(),
            FilterValue::Labels { values, .. } => !values.is_empty(),
        }
    }
}

/// One row in the filter panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterRow {
    /// The field name.
    pub label: String,
    /// The match mode, as a short chip: `fuzzy`, `contains`, `regex`, `date`, `labels`.
    pub kind: String,
    /// The current value.
    pub value: FilterValue,
    /// Whether the cursor is on this row.
    pub selected: bool,
}

/// Snapshot of the filter panel used by the UI renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterPanelView {
    /// The pane title.
    pub title: String,
    /// Counts shown on the right of the pane border.
    pub counts: Vec<Chip>,
    /// The filter rows, in navigation order.
    pub rows: Vec<FilterRow>,
    /// Whether the selected row is being edited.
    pub editing: bool,
    /// Set when there are no filter fields at all.
    pub message: Option<PaneMessage>,
}

/// Renders the filter panel snapshot, or `None` when the panel is closed.
pub fn render_filter_panel(state: &TaskState) -> Option<FilterPanelView> {
    if !state.filter_panel_visible() {
        return None;
    }

    let editing = state.filter_panel_editing();
    let rows = state
        .filter_panel_entries()
        .into_iter()
        .map(filter_row)
        .collect::<Vec<_>>();

    let active = rows.iter().filter(|row| row.value.is_active()).count();
    let mut counts = Vec::new();
    if active > 0 {
        counts.push(Chip::toned(format!("{active} active"), Tone::Accent));
    }
    if editing {
        counts.push(Chip::toned("editing", Tone::Warn));
    }

    Some(FilterPanelView {
        title: "Filters".to_string(),
        counts,
        message: rows.is_empty().then(|| {
            PaneMessage::new("No filter fields", Tone::Muted).with_hint("load tasks first")
        }),
        rows,
        editing,
    })
}

/// Renders every filter row. Exactly one line per row, so the panel's scroll
/// offset stays a plain field index.
pub fn filter_panel_lines(
    view: &FilterPanelView,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let label_width = view
        .rows
        .iter()
        .map(|row| visible_width(&row.label))
        .max()
        .unwrap_or(0)
        .min(LABEL_CAP);
    let kind_width = view
        .rows
        .iter()
        .map(|row| visible_width(&row.kind))
        .max()
        .unwrap_or(0)
        .min(KIND_CAP);

    view.rows
        .iter()
        .map(|row| filter_row_line(row, view.editing, label_width, kind_width, theme, width))
        .collect()
}

fn filter_row_line(
    row: &FilterRow,
    editing: bool,
    label_width: usize,
    kind_width: usize,
    theme: &Theme,
    width: usize,
) -> Line<'static> {
    let glyphs = &theme.glyphs;
    let active = row.value.is_active();

    let mut spans = vec![
        Span::styled(
            if row.selected { glyphs.cursor } else { " " }.to_string(),
            theme.marker,
        ),
        Span::raw(" "),
        Span::styled(
            if active { glyphs.active } else { " " }.to_string(),
            theme.accent,
        ),
        Span::raw(" "),
        Span::styled(
            pad_cell(&row.label, label_width, glyphs.ellipsis),
            if active {
                theme.text.patch(theme.subtitle)
            } else {
                theme.text
            },
        ),
        Span::raw("  "),
        Span::styled(pad_cell(&row.kind, kind_width, glyphs.ellipsis), theme.muted),
        Span::raw("  "),
    ];

    let value_width = width.saturating_sub(spans_width(&spans));
    spans.extend(value_spans(row, editing, theme, value_width));

    // Fixed-width columns can outgrow a very narrow pane, so the finished run
    // is sliced to exactly the pane width rather than merely padded.
    let mut line = Line::from(slice_spans(&spans, 0, width));
    if row.selected {
        line = line.style(theme.cursor);
    }
    line
}

fn value_spans(
    row: &FilterRow,
    editing: bool,
    theme: &Theme,
    width: usize,
) -> Vec<Span<'static>> {
    let glyphs = &theme.glyphs;

    match &row.value {
        FilterValue::Text(query) if query.is_empty() && !(row.selected && editing) => {
            vec![Span::styled(glyphs.empty.to_string(), theme.muted)]
        }
        FilterValue::Text(query) => {
            let caret = if row.selected && editing {
                glyphs.edit_cursor
            } else {
                ""
            };
            vec![Span::styled(
                truncate_with_ellipsis(&format!("{query}{caret}"), width, glyphs.ellipsis),
                theme.text,
            )]
        }
        FilterValue::Labels { values, .. } if values.is_empty() => {
            vec![Span::styled(glyphs.empty.to_string(), theme.muted)]
        }
        FilterValue::Labels { values, cursor } => {
            let mut spans = Vec::new();
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::styled(
                        format!("  {}  ", glyphs.chip_sep),
                        theme.muted,
                    ));
                }
                // While editing, the focused chip is the one the label keys act
                // on, so it is picked out rather than merely bolded.
                let focused = row.selected && editing && *cursor == Some(index);
                spans.push(Span::styled(
                    value.clone(),
                    if focused {
                        theme.accent.add_modifier(ratatui::style::Modifier::REVERSED)
                    } else {
                        theme.accent
                    },
                ));
            }
            spans
        }
    }
}

fn filter_row(entry: TaskFilterPanelEntry) -> FilterRow {
    let value = if entry.kind == "labels" {
        FilterValue::Labels {
            values: entry.label_values,
            cursor: entry.label_cursor,
        }
    } else {
        FilterValue::Text(entry.query)
    };

    FilterRow {
        label: entry.label,
        kind: kind_chip(&entry.kind).to_string(),
        value,
        selected: entry.selected,
    }
}

/// Shortens the internal kind name into a chip.
///
/// `string:contains` reads as implementation detail; `contains` reads as the
/// answer to "how is this matched".
fn kind_chip(kind: &str) -> &str {
    kind.strip_prefix("string:").unwrap_or(kind)
}

#[cfg(test)]
mod tests {
    use super::{
        filter_panel_lines, kind_chip, render_filter_panel, FilterValue, MARKER_WIDTH,
    };
    use crate::{
        app::task::TaskState,
        asana::{
            dto::{SectionDto, TaskDto, TaskMembershipDto, TaskMembershipProjectDto,
                TaskMembershipSectionDto},
            fake::FakeAsanaClient,
        },
        domain::Project,
        ui::{text::visible_width, theme::Theme},
    };

    /// The filter fields are derived from a loaded dataset, so the panel is
    /// empty until tasks have been loaded at least once.
    fn panel_state() -> TaskState {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![TaskDto {
                    gid: "t1".to_string(),
                    name: "Ship release".to_string(),
                    completed: false,
                    modified_at: None,
                    due_on: Some("2026-06-10".to_string()),
                    start_on: None,
                    assignee: None,
                    num_subtasks: 0,
                    memberships: vec![TaskMembershipDto {
                        project: TaskMembershipProjectDto {
                            gid: "p1".to_string(),
                            name: "Inbox".to_string(),
                        },
                        section: Some(TaskMembershipSectionDto {
                            gid: "s1".to_string(),
                            name: "Today".to_string(),
                        }),
                    }],
                    custom_fields: vec![],
                }],
            );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.toggle_filter_panel();
        state
    }

    #[test]
    fn returns_nothing_while_the_panel_is_closed() {
        assert!(render_filter_panel(&TaskState::new()).is_none());
        assert!(render_filter_panel(&{
            let mut state = panel_state();
            state.toggle_filter_panel();
            state
        })
        .is_none());
        assert!(render_filter_panel(&panel_state()).is_some());
    }

    #[test]
    fn rows_keep_their_navigation_order() {
        let view = render_filter_panel(&panel_state()).expect("panel is open");

        let labels = view
            .rows
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec!["Title", "Assignee", "Due", "Start", "State", "Projects"]
        );
        assert!(view.rows[0].selected);
    }

    #[test]
    fn match_modes_render_as_short_chips() {
        let view = render_filter_panel(&panel_state()).expect("panel is open");

        assert_eq!(view.rows[0].kind, "fuzzy");
        assert_eq!(view.rows[1].kind, "contains");
        assert_eq!(view.rows[2].kind, "date");
        assert_eq!(view.rows[4].kind, "labels");
        assert_eq!(kind_chip("string:regex"), "regex");
        assert_eq!(kind_chip("labels"), "labels");
    }

    #[test]
    fn the_active_count_appears_only_once_a_filter_does_something() {
        let mut state = panel_state();
        assert!(render_filter_panel(&state)
            .expect("panel is open")
            .counts
            .is_empty());

        state.filter_push_char('a');
        let counts = render_filter_panel(&state)
            .expect("panel is open")
            .counts
            .iter()
            .map(|chip| chip.text.clone())
            .collect::<Vec<_>>();

        assert_eq!(counts, vec!["1 active".to_string()]);
    }

    #[test]
    fn an_empty_field_shows_a_placeholder_rather_than_nothing() {
        let theme = Theme::default();
        let view = render_filter_panel(&panel_state()).expect("panel is open");

        let lines = filter_panel_lines(&view, &theme, 60);

        assert!(lines[1].to_string().contains(theme.glyphs.empty));
        assert!(!view.rows[1].value.is_active());
        assert!(matches!(&view.rows[1].value, FilterValue::Text(query) if query.is_empty()));
    }

    #[test]
    fn one_line_per_row_at_exactly_the_requested_width() {
        let theme = Theme::default();
        let mut state = panel_state();
        state.filter_push_char('s');
        state.filter_push_char('h');
        let view = render_filter_panel(&state).expect("panel is open");

        for width in [MARKER_WIDTH + 8, 40, 80, 160] {
            let lines = filter_panel_lines(&view, &theme, width);
            assert_eq!(lines.len(), view.rows.len());
            for line in lines {
                assert_eq!(visible_width(&line.to_string()), width);
            }
        }
    }

    #[test]
    fn editing_marks_the_panel_and_shows_a_caret() {
        let theme = Theme::default();
        let mut state = panel_state();
        state.filter_edit_begin();
        state.filter_push_char('x');

        let view = render_filter_panel(&state).expect("panel is open");
        let lines = filter_panel_lines(&view, &theme, 60);

        assert!(view.editing);
        assert!(view
            .counts
            .iter()
            .any(|chip| chip.text == "editing"));
        assert!(lines[0].to_string().contains(theme.glyphs.edit_cursor));
    }
}
