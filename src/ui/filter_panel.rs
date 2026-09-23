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
    config::Mode,
    ui::{
        chrome::{Chip, PaneMessage, Tone},
        text::{
            caret_spans, caret_window, clip_spans, pad_cell, slice_spans, spans_width,
            truncate_with_ellipsis, visible_width,
        },
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
    /// Whether the set's whole verdict is inverted.
    pub negated: bool,
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
    /// Whether this row's verdict is inverted.
    pub negated: bool,
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
    // The rows below state a positive filter and the set then inverts all of
    // them at once, so the tab's marker is not enough on its own: this says it
    // about the fields you are actually looking at.
    if state.filter_active_set_negated() {
        counts.push(Chip::toned("negated set", Tone::Danger));
    }

    // A lone `1` tab is noise, and with one set the pane then renders
    // byte-identically to how it did before sets existed.
    //
    // There is no matching `set n/m` count on the right of the border: the
    // strip is on the same line and says it already.
    let (active_set, total_sets) = state.filter_set_position();
    let tabs = if total_sets > 1 {
        let negations = state.filter_set_negations();
        state
            .filter_set_counts()
            .into_iter()
            .enumerate()
            .map(|(index, filters)| FilterTab {
                label: (index + 1).to_string(),
                active: index == active_set,
                filters,
                negated: negations.get(index).copied().unwrap_or(false),
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
/// It rides the pane's top border, immediately right of the `Filters` title,
/// because which set you are editing is a fact *about* that title. As an
/// interior line it read as one more filter row with numbers in it.
///
/// Each set is a cell in a rule-separated strip, which is what makes it read as
/// a row of tabs rather than a run of numbers, and the whole strip hangs off a
/// stub of the border rule so it looks mounted on the frame. Two things mark
/// the current set, because colour alone is not allowed to carry it: the cell
/// is filled, and it is the only one that spells out the word the strip is
/// naming.
///
/// A negated set is marked three ways, none of which is only colour: the cell
/// carries the same `¬` its fields would, it is drawn in the danger style, and
/// the two dividers around it thicken into the negated edge — so the boundary
/// of "this tab means the opposite" is visible even between two other tabs.
///
/// The number of sets is unbounded, so a strip past its `budget` collapses to a
/// single `set n/m` cell rather than being cut off mid-tab.
pub fn tab_strip_spans(
    view: &FilterPanelView,
    theme: &Theme,
    focused: bool,
    mode: Mode,
    budget: usize,
) -> Vec<Span<'static>> {
    if view.tabs.is_empty() || budget == 0 {
        return Vec::new();
    }

    let filled = theme
        .pane_title(focused, mode)
        .add_modifier(Modifier::REVERSED);
    let active = view.tabs.iter().position(|tab| tab.active).unwrap_or(0);

    // A stub of the pane's own rule, so the strip hangs off the frame instead
    // of floating in the gap after the title.
    let rule = || {
        Span::styled(
            theme.border_set(focused).horizontal_top.to_string(),
            theme.pane_border(focused, mode),
        )
    };
    // A divider belongs to both tabs it sits between, so either one being
    // negated is enough to change it.
    let divider = |left: Option<&FilterTab>, right: Option<&FilterTab>| {
        let negated = [left, right]
            .iter()
            .flatten()
            .any(|tab: &&FilterTab| tab.negated);
        match negated {
            true => Span::styled(theme.glyphs.negated_edge.to_string(), theme.danger),
            false => Span::styled(theme.glyphs.column_rule.to_string(), theme.border),
        }
    };

    // `any of` is the whole semantics of having more than one set, and there is
    // nowhere else on screen that says it.
    let mut spans = vec![
        rule(),
        Span::raw(" "),
        Span::styled("any of ".to_string(), theme.muted),
    ];

    for (index, tab) in view.tabs.iter().enumerate() {
        spans.push(divider(index.checked_sub(1).and_then(|i| view.tabs.get(i)), Some(tab)));
        spans.push(Span::styled(
            tab_text(tab, &theme.glyphs),
            match (tab.negated, tab.active) {
                (true, true) => theme.danger.add_modifier(Modifier::REVERSED),
                (true, false) => theme.danger,
                (false, true) => filled,
                (false, false) => theme.muted,
            },
        ));
    }
    spans.push(divider(view.tabs.last(), None));
    spans.push(Span::raw(" "));

    if spans_width(&spans) > budget {
        let negated = match view.tabs[active].negated {
            true => theme.glyphs.negate,
            false => "",
        };
        spans = vec![
            rule(),
            Span::styled(
                format!(" {negated}set {}/{} ", active + 1, view.tabs.len()),
                match view.tabs[active].negated {
                    true => theme.danger.add_modifier(Modifier::REVERSED),
                    false => filled,
                },
            ),
        ];
    }

    clip_spans(spans, budget)
}

/// The text inside one tab cell.
///
/// A set that is filtering something carries the same marker the rows use, so
/// a working set left behind on another tab is visible from here, and a
/// negated one leads with the same `¬` its rows would.
fn tab_text(tab: &FilterTab, glyphs: &crate::ui::theme::GlyphSet) -> String {
    let negate = match tab.negated {
        true => glyphs.negate,
        false => "",
    };
    let marker = match tab.filters > 0 {
        true => glyphs.active,
        false => "",
    };
    match tab.active {
        true => format!(" {negate}set {}{marker} ", tab.label),
        false => format!(" {negate}{}{marker} ", tab.label),
    }
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

    // The gutter's second slot answers "what is this row doing", and being
    // inverted is a different answer from being on: the negate glyph replaces
    // the active one rather than sitting beside it, so the column stays one
    // cell wide and every row still lines up.
    let (marker, marker_style) = match (row.negated, active) {
        (true, _) => (glyphs.negate, theme.danger),
        (false, true) => (glyphs.active, theme.accent),
        (false, false) => (" ", theme.accent),
    };

    let mut spans = vec![
        Span::styled(
            if row.selected { glyphs.cursor } else { " " }.to_string(),
            theme.marker,
        ),
        Span::raw(" "),
        Span::styled(marker.to_string(), marker_style),
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

    // `¬ alex` in front of the value is what makes the row *read* correctly:
    // the gutter says the row is inverted, this says which half of it is.
    if row.negated {
        spans.push(Span::styled(
            format!("{} ", glyphs.negate),
            theme.danger,
        ));
    }

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
        FilterValue::Text(query) => match row.caret {
            // A caret past the column's width is a caret off the screen, so
            // an edited value scrolls under it instead of truncating.
            Some(caret) => {
                let window = caret_window(query, caret, width, glyphs.ellipsis, 0);
                caret_spans(&window.text, Some(window.caret), theme.text)
            }
            None => vec![Span::styled(
                truncate_with_ellipsis(query, width, glyphs.ellipsis),
                theme.text,
            )],
        },
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
        negated: entry.negated,
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
        filter_panel_lines, kind_chip, render_filter_panel, tab_strip_spans, FilterValue,
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
        config::Mode,
        domain::Project,
        ui::{
            text::{spans_width, visible_width},
            theme::Theme,
        },
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
    fn a_long_filter_value_scrolls_under_the_caret_rather_than_truncating() {
        // The value column has the same bug the table cell had: a caret past
        // the cut is a caret nobody can see.
        let theme = Theme::default();
        let mut state = panel_state();
        state.filter_edit_begin();
        for ch in "a value far longer than the column can show".chars() {
            state.filter_push_char(ch);
        }

        let view = render_filter_panel(&state).expect("panel is open");
        let line = filter_panel_lines(&view, &theme, 40)[0].to_string();

        assert!(
            line.contains("show") || line.contains("can"),
            "the tail the caret is in is on screen: {line}"
        );
        assert!(
            line.contains(theme.glyphs.ellipsis),
            "and the clipped side is marked: {line}"
        );
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

    /// The rendered text of the tab strip at a generous budget.
    fn strip_text(view: &super::FilterPanelView, theme: &Theme, budget: usize) -> String {
        tab_strip_spans(view, theme, true, Mode::Filter, budget)
            .iter()
            .map(|span| span.content.to_string())
            .collect()
    }

    #[test]
    fn a_single_set_draws_no_tab_strip() {
        // The common case has to look exactly as it did before sets existed.
        let theme = Theme::default();
        let view = render_filter_panel(&panel_state()).expect("panel is open");

        assert!(view.tabs.is_empty());
        assert!(tab_strip_spans(&view, &theme, true, Mode::Filter, 80).is_empty());
        assert!(view.counts.iter().all(|chip| !chip.text.starts_with("set ")));
    }

    #[test]
    fn a_second_set_earns_a_tab_strip() {
        let theme = Theme::default();
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

        let text = strip_text(&view, &theme, 80);
        assert!(text.contains("any of"), "the strip says how sets combine");
        assert!(text.contains("set 2"), "and which one the keys act on");
        assert!(
            text.contains(&format!("1{}", theme.glyphs.active)),
            "the set left behind is marked as filtering"
        );
        assert!(
            view.counts.iter().all(|chip| !chip.text.starts_with("set ")),
            "the strip is the only place the position is stated"
        );
    }

    #[test]
    fn the_tab_strip_never_outgrows_its_budget() {
        // It shares the top border with the pane's counts, so an overlong strip
        // would draw straight through them.
        let theme = Theme::default();
        let mut state = panel_state();
        for _ in 0..8 {
            state.filter_add_set();
        }
        let view = render_filter_panel(&state).expect("panel is open");

        for budget in [0, 1, MARKER_WIDTH, 12, 40, 80, 160] {
            let spans = tab_strip_spans(&view, &theme, true, Mode::Filter, budget);
            assert!(spans_width(&spans) <= budget, "overran a {budget} budget");
        }
    }

    #[test]
    fn too_many_sets_for_the_border_collapse_to_a_position() {
        // Cutting the strip mid-tab would leave a half-drawn number that reads
        // as a different set than it is.
        let theme = Theme::default();
        let mut state = panel_state();
        for _ in 0..8 {
            state.filter_add_set();
        }
        let view = render_filter_panel(&state).expect("panel is open");

        let text = strip_text(&view, &theme, 24);
        assert!(text.ends_with(" set 9/9 "), "collapsed to {text:?}");
    }

    #[test]
    fn the_active_tab_is_told_apart_without_colour() {
        // The mono theme collapses the styles, so the active tab has to carry a
        // modifier — the same rule the calendar's day styles follow.
        let theme = Theme::default();
        let mut state = panel_state();
        state.filter_add_set();
        let view = render_filter_panel(&state).expect("panel is open");
        let spans = tab_strip_spans(&view, &theme, true, Mode::Filter, 60);

        assert!(spans
            .iter()
            .any(|span| span.style.add_modifier.contains(Modifier::REVERSED)));
    }

    #[test]
    fn a_negated_row_is_marked_in_the_gutter_and_in_front_of_its_value() {
        // Two marks, not one: the gutter is what you scan, and the prefix is
        // what makes the row read as "assignee is not alex".
        let theme = Theme::default();
        let mut state = panel_state();
        state.move_filter_down();
        state.filter_edit_begin();
        for ch in "alex".chars() {
            state.filter_push_char(ch);
        }
        state.filter_edit_done();
        state.filter_toggle_negate_field();

        let view = render_filter_panel(&state).expect("panel is open");
        let line = filter_panel_lines(&view, &theme, 60)[1].to_string();

        assert!(view.rows[1].negated);
        assert_eq!(
            line.matches(theme.glyphs.negate).count(),
            2,
            "one in the gutter and one on the value: {line:?}"
        );
        assert!(!line.contains(theme.glyphs.active), "which it replaces");
        assert!(line.contains(&format!("{} alex", theme.glyphs.negate)));
    }

    #[test]
    fn a_negated_row_keeps_its_columns_aligned_with_the_others() {
        // The gutter is one cell wide for every row, so a negated row must not
        // push its label along.
        let theme = Theme::default();
        let mut state = panel_state();
        state.filter_toggle_negate_field();
        let view = render_filter_panel(&state).expect("panel is open");
        let lines = filter_panel_lines(&view, &theme, 60);

        // Display columns, not byte offsets: the cursor and negate glyphs are
        // one cell but several bytes each.
        let column = |line: &str| {
            let at = line.find("fuzzy").expect("the kind column");
            visible_width(&line[..at])
        };
        assert_eq!(column(&lines[0].to_string()), column(&lines[1].to_string()));
    }

    #[test]
    fn a_negated_set_is_marked_in_its_tab_and_in_the_pane_counts() {
        let theme = Theme::default();
        let mut state = panel_state();
        state.filter_add_set();
        state.filter_toggle_negate_set();
        let view = render_filter_panel(&state).expect("panel is open");

        assert_eq!(
            view.tabs.iter().map(|tab| tab.negated).collect::<Vec<_>>(),
            vec![false, true]
        );
        assert!(
            view.counts.iter().any(|chip| chip.text == "negated set"),
            "the rows on screen are the ones being inverted"
        );

        let text = strip_text(&view, &theme, 80);
        assert!(text.contains(&format!("{}set 2", theme.glyphs.negate)));
        assert_eq!(
            text.matches(theme.glyphs.negated_edge).count(),
            2,
            "both of the negated tab's edges change: {text:?}"
        );
        assert_eq!(
            text.matches(theme.glyphs.column_rule).count(),
            1,
            "and the untouched tab keeps its own: {text:?}"
        );
    }

    #[test]
    fn the_negated_edge_is_shared_by_the_tabs_it_sits_between() {
        // A divider belongs to both of its neighbours, so one negated tab is
        // enough to change it — otherwise the boundary would be drawn twice
        // between two negated sets and inconsistently beside one.
        let theme = Theme::default();
        let mut state = panel_state();
        state.filter_add_set();
        state.filter_add_set();
        state.filter_toggle_negate_set();
        let view = render_filter_panel(&state).expect("panel is open");

        let text = strip_text(&view, &theme, 80);
        assert_eq!(text.matches(theme.glyphs.negated_edge).count(), 2);
        assert_eq!(text.matches(theme.glyphs.column_rule).count(), 2);
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
