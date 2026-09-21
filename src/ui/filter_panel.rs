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

use ratatui::{style::Modifier, text::{Line, Span}};

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
/// What a require-empty filter shows as its value.
///
/// A word rather than the `empty` glyph: that glyph already means "unset" in
/// this column, and "unset" and "require unset" are opposites.
pub const EMPTY_REQUIRED_TEXT: &str = "(none)";
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
    /// The field must have no value at all.
    RequireEmpty,
}

impl FilterValue {
    /// Whether this field is currently filtering anything out.
    pub fn is_active(&self) -> bool {
        match self {
            FilterValue::Text(query) => !query.trim().is_empty(),
            FilterValue::Labels { values, .. } => !values.is_empty(),
            FilterValue::RequireEmpty => true,
        }
    }
}

/// One tab in the filter pane's tab strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterTab {
    /// The tab's number, as shown: `1`, `2`, ...
    pub label: String,
    /// Whether this is the set the keys act on.
    pub active: bool,
    /// How many of its fields filter anything, so a working set is visible from
    /// another tab.
    pub filters: usize,
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
    /// Where the edit caret sits in the value, as a char index.
    ///
    /// `None` when this row is not being edited. A date field being picked on
    /// the calendar can have its caret anywhere in the text, because the arrow
    /// keys move it, so this is a position rather than a flag.
    pub caret: Option<usize>,
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
    /// One tab per filter set, or empty when there is only one.
    pub tabs: Vec<FilterTab>,
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

    // A lone `1` tab is noise, and with one set the pane then renders
    // byte-identically to how it did before sets existed.
    let (active_set, total_sets) = state.filter_set_position();
    let tabs = if total_sets > 1 {
        counts.push(Chip::toned(
            format!("set {}/{total_sets}", active_set + 1),
            Tone::Info,
        ));
        state
            .filter_set_counts()
            .into_iter()
            .enumerate()
            .map(|(index, filters)| FilterTab {
                label: (index + 1).to_string(),
                active: index == active_set,
                filters,
            })
            .collect()
    } else {
        Vec::new()
    };

    Some(FilterPanelView {
        title: "Filters".to_string(),
        counts,
        message: rows.is_empty().then(|| {
            PaneMessage::new("No filter fields", Tone::Muted).with_hint("load tasks first")
        }),
        rows,
        tabs,
        editing,
    })
}

/// Renders the tab strip: one tab per filter set, the active one picked out.
///
/// Drawn as the pane's first interior line rather than in the border. The
/// number of sets is unbounded and the border truncates from the left — the
/// mistake that turned "Aug 2026" into "6" in the calendar overlay.
///
/// The separator is the word `or`, because that is the whole semantics of the
/// strip and there is nowhere else on screen to say it.
pub fn tab_strip_line(view: &FilterPanelView, theme: &Theme, width: usize) -> Line<'static> {
    let glyphs = &theme.glyphs;
    // The same gutter the rows use, so the tabs line up with the label column.
    let mut spans = vec![Span::raw(" ".repeat(MARKER_WIDTH))];

    for (index, tab) in view.tabs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" or ".to_string(), theme.muted));
        }
        let text = if tab.filters > 0 {
            format!(" {}{} ", tab.label, glyphs.active)
        } else {
            format!(" {} ", tab.label)
        };
        spans.push(Span::styled(
            text,
            if tab.active {
                theme.accent.add_modifier(Modifier::REVERSED)
            } else {
                theme.muted
            },
        ));
    }

    Line::from(slice_spans(&spans, 0, width))
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
        FilterValue::RequireEmpty => {
            vec![Span::styled(EMPTY_REQUIRED_TEXT.to_string(), theme.accent)]
        }
        FilterValue::Text(query) if query.is_empty() && row.caret.is_none() => {
            vec![Span::styled(glyphs.empty.to_string(), theme.muted)]
        }
        FilterValue::Text(query) => {
            let text = truncate_with_ellipsis(query, width, glyphs.ellipsis);
            caret_spans(&text, row.caret, theme)
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

/// Splits a value around the caret so the caret can be drawn as a style.
///
/// The caret used to be a glyph spliced into the text, which pushed the
/// characters after it along and read as a stray space that wandered as the
/// caret moved. Reversing the character *under* the caret instead costs no
/// columns, so the text stays put while the caret travels through it. Only at
/// the very end, where there is no character to reverse, does a cell get added.
fn caret_spans(text: &str, caret: Option<usize>, theme: &Theme) -> Vec<Span<'static>> {
    let Some(caret) = caret else {
        return vec![Span::styled(text.to_string(), theme.text)];
    };

    let chars = text.chars().collect::<Vec<_>>();
    let at = caret.min(chars.len());
    let head = chars[..at].iter().collect::<String>();
    let under = chars.get(at).copied();
    let tail = if at < chars.len() {
        chars[at + 1..].iter().collect::<String>()
    } else {
        String::new()
    };

    let caret_style = theme.text.add_modifier(Modifier::REVERSED);
    let mut spans = Vec::with_capacity(3);
    if !head.is_empty() {
        spans.push(Span::styled(head, theme.text));
    }
    spans.push(Span::styled(
        under.map_or_else(|| " ".to_string(), |ch| ch.to_string()),
        caret_style,
    ));
    if !tail.is_empty() {
        spans.push(Span::styled(tail, theme.text));
    }
    spans
}

fn filter_row(entry: TaskFilterPanelEntry) -> FilterRow {
    // Checked before the kind: a require-empty looks the same on every row,
    // and `(none)` is a different statement from the `—` of an unset field.
    let value = if entry.empty_required {
        FilterValue::RequireEmpty
    } else if entry.kind == "labels" {
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
        caret: entry.caret,
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
        filter_panel_lines, kind_chip, render_filter_panel, tab_strip_line, FilterValue,
        MARKER_WIDTH,
    };
    use ratatui::style::Modifier;

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
                    parent: None,
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
    fn assignee_matches_fuzzily_by_default() {
        // A person's name is the field most likely to be half-remembered, so it
        // gets the same default Title has.
        let view = render_filter_panel(&panel_state()).expect("panel is open");
        assert_eq!(view.rows[1].kind, "fuzzy");
    }

    #[test]
    fn match_modes_render_as_short_chips() {
        let view = render_filter_panel(&panel_state()).expect("panel is open");

        assert_eq!(view.rows[0].kind, "fuzzy");
        assert_eq!(view.rows[1].kind, "fuzzy");
        assert_eq!(view.rows[5].kind, "contains");
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
        assert!(view.counts.iter().any(|chip| chip.text == "editing"));
        assert_eq!(view.rows[0].caret, Some(1), "the caret follows the typing");
        assert!(
            reversed(&lines[0]).is_some(),
            "and is drawn as a reversed cell"
        );
    }

    /// The text of the caret cell on a line, if one is drawn.
    fn reversed(line: &ratatui::text::Line<'_>) -> Option<String> {
        line.spans
            .iter()
            .find(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .map(|span| span.content.to_string())
    }

    #[test]
    fn the_caret_costs_no_width_until_it_reaches_the_end_of_the_text() {
        let theme = Theme::default();
        let mut state = panel_state();
        state.filter_edit_begin();
        for ch in "shipit".chars() {
            state.filter_push_char(ch);
        }

        // At the end there is no character to reverse, so the caret is a cell of
        // its own.
        let at_end = render_filter_panel(&state).expect("panel is open");
        assert_eq!(at_end.rows[0].caret, Some(6));
        assert_eq!(
            reversed(&filter_panel_lines(&at_end, &theme, 60)[0]).as_deref(),
            Some(" ")
        );

        // Moved back into the text, the caret reverses the character it is on
        // rather than pushing the rest along. This is what stopped it reading as
        // a stray space wandering through the value.
        state.filter_move_caret(-2);
        let inside = render_filter_panel(&state).expect("panel is open");
        let line = filter_panel_lines(&inside, &theme, 60)[0].to_string();

        assert_eq!(inside.rows[0].caret, Some(4));
        assert_eq!(
            reversed(&filter_panel_lines(&inside, &theme, 60)[0]).as_deref(),
            Some("i"),
            "the caret sits on the character it is in front of"
        );
        assert!(line.contains("shipit"), "and the value is unbroken");
    }

    #[test]
    fn the_caret_stops_at_both_ends_of_a_text_field() {
        let mut state = panel_state();
        state.filter_edit_begin();
        for ch in "ab".chars() {
            state.filter_push_char(ch);
        }

        state.filter_move_caret(-99);
        assert_eq!(
            render_filter_panel(&state).expect("panel is open").rows[0].caret,
            Some(0)
        );

        // Backspace at the start has nothing to delete.
        state.filter_pop_char();
        let view = render_filter_panel(&state).expect("panel is open");
        assert!(matches!(&view.rows[0].value, FilterValue::Text(query) if query == "ab"));

        state.filter_move_caret(99);
        assert_eq!(
            render_filter_panel(&state).expect("panel is open").rows[0].caret,
            Some(2)
        );
    }

    #[test]
    fn a_single_set_draws_no_tab_strip() {
        // The common case has to look exactly as it did before sets existed.
        let view = render_filter_panel(&panel_state()).expect("panel is open");

        assert!(view.tabs.is_empty());
        assert!(view.counts.iter().all(|chip| !chip.text.starts_with("set ")));
    }

    #[test]
    fn a_second_set_earns_a_tab_strip_and_a_chip() {
        let mut state = panel_state();
        state.filter_push_char('a');
        state.filter_add_set();
        let view = render_filter_panel(&state).expect("panel is open");

        assert_eq!(
            view.tabs
                .iter()
                .map(|tab| (tab.label.clone(), tab.active, tab.filters))
                .collect::<Vec<_>>(),
            vec![("1".to_string(), false, 1), ("2".to_string(), true, 0)],
            "the set left behind still shows that it is filtering"
        );
        assert!(view.counts.iter().any(|chip| chip.text == "set 2/2"));
    }

    #[test]
    fn the_tab_strip_is_one_line_at_exactly_the_requested_width() {
        // Same invariant the rows have: the pane's scroll offset is a field
        // index, so nothing here may wrap.
        let theme = Theme::default();
        let mut state = panel_state();
        for _ in 0..8 {
            state.filter_add_set();
        }
        let view = render_filter_panel(&state).expect("panel is open");

        for width in [MARKER_WIDTH + 8, 40, 80, 160] {
            let line = tab_strip_line(&view, &theme, width);
            assert_eq!(visible_width(&line.to_string()), width);
        }
    }

    #[test]
    fn the_active_tab_is_told_apart_without_colour() {
        // The mono theme collapses the styles, so the active tab has to carry a
        // modifier — the same rule the calendar's day styles follow.
        let theme = Theme::default();
        let mut state = panel_state();
        state.filter_add_set();
        let view = render_filter_panel(&state).expect("panel is open");
        let line = tab_strip_line(&view, &theme, 60);

        assert!(line
            .spans
            .iter()
            .any(|span| span.style.add_modifier.contains(Modifier::REVERSED)));
    }

    #[test]
    fn a_require_empty_row_shows_a_value_rather_than_the_unset_placeholder() {
        // `—` means "no filter"; `(none)` means "filter to no value". They are
        // opposites and must not look alike.
        let theme = Theme::default();
        let mut state = panel_state();
        state.move_filter_down();
        state.filter_toggle_require_empty();
        let view = render_filter_panel(&state).expect("panel is open");
        let lines = filter_panel_lines(&view, &theme, 60);

        assert_eq!(view.rows[1].value, FilterValue::RequireEmpty);
        assert!(view.rows[1].value.is_active());
        assert!(lines[1].to_string().contains("(none)"));
        assert!(!lines[1].to_string().contains(theme.glyphs.empty));
    }

    #[test]
    fn typing_inserts_at_the_caret_rather_than_appending() {
        let mut state = panel_state();
        state.filter_edit_begin();
        for ch in "ac".chars() {
            state.filter_push_char(ch);
        }
        state.filter_move_caret(-1);
        state.filter_push_char('b');

        let view = render_filter_panel(&state).expect("panel is open");
        assert!(matches!(&view.rows[0].value, FilterValue::Text(query) if query == "abc"));
        assert_eq!(view.rows[0].caret, Some(2), "and the caret moves with it");
    }
}
