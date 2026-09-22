//! Rendering for the named-filter-set sidebar.
//!
//! The sidebar is a pane of its own — its own border, its own title — and it
//! is **never focused**. `j` and `k` keep walking the filter fields, and the
//! thin, unfocused border is what says so. Its access path is the digits, and
//! `1`-`9` address the rows as numbered here rather than entries as ordered on
//! disk, which is why the window arithmetic lives in one place.
//!
//! One line per row, as in the filter panel, so the window stays a plain
//! index.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
};

use crate::{
    app::task::{SidebarPrompt, TaskState, MAX_SIDEBAR_ROWS},
    config::NamedFilterSet,
    ui::{
        chrome::Chip,
        text::{fill, truncate_with_ellipsis, visible_width},
        theme::Theme,
    },
};

/// Width the sidebar asks for.
pub const SIDEBAR_WIDTH: u16 = 24;
/// Narrowest filter panel worth keeping beside it.
///
/// The three columns of label, match mode, and value are what the filter pane
/// is *for*, and the value column is the one that has to stay readable. Below
/// this the sidebar is dropped entirely rather than squeezing them: at 80
/// columns there is not room for both.
pub const MIN_FILTER_WIDTH: u16 = 64;
/// Lines the sidebar spends before the numbered entries: the live panel's
/// name, its counts, and the rule under them.
const HEADER_LINES: u16 = 3;

/// Splits the filter pane's area into the sidebar and what is left.
pub fn split_sidebar(area: Rect, visible: bool) -> (Option<Rect>, Rect) {
    if !visible {
        return (None, area);
    }

    // A sidebar wider than a third of the pane stops being a sidebar.
    let width = SIDEBAR_WIDTH.min(area.width / 3);
    if width == 0 || area.width.saturating_sub(width) < MIN_FILTER_WIDTH {
        return (None, area);
    }

    (
        Some(Rect { width, ..area }),
        Rect {
            x: area.x + width,
            width: area.width - width,
            ..area
        },
    )
}

/// How many saved entries fit in a sidebar of this size.
pub fn window_rows(area: Rect) -> usize {
    (area.height.saturating_sub(2 + HEADER_LINES) as usize).clamp(1, MAX_SIDEBAR_ROWS)
}

/// The live panel, pinned above the rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentSetRow {
    /// The entry the panel is bound to, or `None` when it is unnamed.
    pub name: Option<String>,
    pub sets: usize,
    pub filters: usize,
}

/// One saved entry in the visible window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterSetRow {
    /// The digit that loads it: numbered from the top of the window, not from
    /// the entry's index in the file.
    pub number: usize,
    pub name: String,
    /// Whether the panel is bound to this entry.
    pub loaded: bool,
}

/// The sidebar's border-mounted prompt line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptLine {
    /// What the prompt is asking.
    pub label: String,
    /// What has been typed, or the confirmation's choices.
    pub text: String,
    /// Where the caret sits in `text`, when there is one to type into.
    pub caret: Option<usize>,
    /// Whether this is a report rather than a question.
    pub error: bool,
}

/// Snapshot of the sidebar used by the UI renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterSetsView {
    /// The live panel: its name or `unnamed`, and what it holds.
    pub current: CurrentSetRow,
    /// The visible window of saved entries, numbered from 1.
    pub rows: Vec<FilterSetRow>,
    /// `1-9 of 14`, or `None` when they all fit.
    pub page: Option<String>,
    /// Counts shown on the right of the pane border.
    pub counts: Vec<Chip>,
    pub prompt: Option<PromptLine>,
}

/// Renders the sidebar snapshot, or `None` when it is closed.
pub fn render_filter_sets(
    state: &TaskState,
    saved: &[NamedFilterSet],
) -> Option<FilterSetsView> {
    if !state.filter_sets_sidebar_visible() {
        return None;
    }

    let entries = sorted_names(saved);
    let window = state.filter_sets_window();
    let start = state.filter_sets_page_start().min(entries.len());
    let loaded = state.filter_set_loaded_name();

    let rows = entries
        .iter()
        .enumerate()
        .skip(start)
        .take(window)
        .map(|(index, name)| FilterSetRow {
            number: index - start + 1,
            loaded: loaded.is_some_and(|current| current.eq_ignore_ascii_case(name)),
            name: name.clone(),
        })
        .collect::<Vec<_>>();

    // Silent when everything fits: the numbers already say where you are.
    let page = (entries.len() > window).then(|| {
        format!(
            "{}-{} of {}",
            start + 1,
            (start + rows.len()).max(start + 1),
            entries.len()
        )
    });

    Some(FilterSetsView {
        current: CurrentSetRow {
            name: loaded.map(str::to_string),
            sets: state.filter_set_position().1,
            filters: state.filter_set_counts().iter().sum(),
        },
        rows,
        counts: page
            .clone()
            .map(|page| vec![Chip::new(page)])
            .unwrap_or_default(),
        page,
        prompt: prompt_line(state),
    })
}

/// The saved names in the order the sidebar lists them, which is also the
/// order the digits address.
fn sorted_names(saved: &[NamedFilterSet]) -> Vec<String> {
    let mut names = saved
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// The prompt or report the panel currently has to show, if any.
///
/// Public because the filter pane borrows it when the sidebar has been
/// dropped for width: a prompt with nowhere to be would make `w` a blind
/// edit on a narrow terminal.
pub fn prompt_line(state: &TaskState) -> Option<PromptLine> {
    // A report wins the line: it is about the key that was just pressed, and
    // the prompt that produced it has already closed.
    if let Some(notice) = state.filter_sets_notice() {
        return Some(PromptLine {
            label: "!".to_string(),
            text: notice.to_string(),
            caret: None,
            error: true,
        });
    }

    match state.filter_set_prompt()? {
        SidebarPrompt::Save { text, caret } => Some(PromptLine {
            label: "save as".to_string(),
            text: text.clone(),
            caret: Some(*caret),
            error: false,
        }),
        SidebarPrompt::ConfirmDelete { name } => Some(PromptLine {
            label: format!("delete {name}?"),
            text: "y/n".to_string(),
            caret: None,
            error: false,
        }),
        // The entry the digit named, rather than what is being lost: the
        // panel about to go is the one on screen, and the `current` row above
        // already says it is `unnamed`.
        SidebarPrompt::ConfirmLoad { name } => Some(PromptLine {
            label: format!("discard for {name}?"),
            text: "y/n".to_string(),
            caret: None,
            error: false,
        }),
    }
}

/// Renders the sidebar's body. Exactly one line per row.
pub fn filter_sets_lines(
    view: &FilterSetsView,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let glyphs = &theme.glyphs;
    let mut lines = Vec::with_capacity(view.rows.len() + HEADER_LINES as usize);

    let named = view.current.name.is_some();
    let name = view
        .current
        .name
        .clone()
        .unwrap_or_else(|| "unnamed".to_string());
    lines.push(Line::from(vec![
        Span::styled(format!("{} ", glyphs.breadcrumb), theme.accent),
        Span::styled(
            truncate_with_ellipsis(&name, width.saturating_sub(2), glyphs.ellipsis),
            if named { theme.subtitle } else { theme.muted },
        ),
    ]));
    lines.push(Line::from(Span::styled(
        truncate_with_ellipsis(
            &format!(
                "  {} {} {} {} active",
                view.current.sets,
                plural("set", view.current.sets),
                glyphs.chip_sep,
                view.current.filters,
            ),
            width,
            glyphs.ellipsis,
        ),
        theme.muted,
    )));
    lines.push(Line::from(Span::styled(
        fill(glyphs.rule, width),
        theme.border,
    )));

    for row in &view.rows {
        lines.push(filter_set_row_line(row, theme, width));
    }

    lines
}

fn filter_set_row_line(row: &FilterSetRow, theme: &Theme, width: usize) -> Line<'static> {
    let glyphs = &theme.glyphs;
    // The number, a gap, and the loaded marker; whatever is left is the name.
    let gutter = 3;
    let spans = vec![
        Span::styled(row.number.to_string(), theme.key),
        Span::raw(" "),
        Span::styled(
            if row.loaded { glyphs.cursor } else { " " }.to_string(),
            theme.marker,
        ),
        Span::styled(
            truncate_with_ellipsis(&row.name, width.saturating_sub(gutter), glyphs.ellipsis),
            if row.loaded { theme.accent } else { theme.text },
        ),
    ];

    Line::from(spans)
}

/// Renders the prompt for the sidebar's bottom border.
///
/// It rides the border exactly as project search does, so it costs no body
/// line — and it is why `w` opens the sidebar: the prompt has somewhere to be.
///
/// The label is what gives when `width` runs out. A sidebar is 24 columns and
/// a saved name can be any length, so something has to — and the typed name
/// or the `y/n` is the half the user is answering with.
pub fn prompt_footer_line(
    prompt: &PromptLine,
    theme: &Theme,
    width: usize,
) -> Line<'static> {
    let style = if prompt.error { theme.danger } else { theme.accent };
    let text = match prompt.caret {
        Some(caret) => splice_caret(&prompt.text, caret, theme.glyphs.edit_cursor),
        None => prompt.text.clone(),
    };

    // Three spaces of padding: one each side, one between the two halves.
    let label_width = width
        .saturating_sub(visible_width(&text) + 3)
        .max(1);
    let label = truncate_with_ellipsis(&prompt.label, label_width, theme.glyphs.ellipsis);

    Line::from(vec![
        Span::raw(" "),
        Span::styled(label, theme.muted),
        Span::raw(" "),
        Span::styled(text, style),
        Span::raw(" "),
    ])
}

/// Puts the edit cursor at the caret, rather than always at the end.
fn splice_caret(text: &str, caret: usize, cursor: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let at = caret.min(chars.len());
    format!(
        "{}{cursor}{}",
        chars[..at].iter().collect::<String>(),
        chars[at..].iter().collect::<String>(),
    )
}

fn plural(word: &str, count: usize) -> String {
    match count {
        1 => word.to_string(),
        _ => format!("{word}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        filter_sets_lines, prompt_footer_line, render_filter_sets, split_sidebar,
        MIN_FILTER_WIDTH, SIDEBAR_WIDTH,
    };
    use ratatui::layout::Rect;

    use crate::{
        app::task::TaskState,
        config::NamedFilterSet,
        ui::{text::visible_width, theme::Theme},
    };

    fn entries(count: usize) -> Vec<NamedFilterSet> {
        (0..count)
            .map(|index| NamedFilterSet {
                // Zero-padded so name order and creation order agree, which
                // keeps the assertions about *which* row is which readable.
                name: format!("set {index:02}"),
                sets: Vec::new(),
            })
            .collect()
    }

    fn sidebar_state(rows: usize) -> TaskState {
        let mut state = TaskState::new();
        state.filter_sets_toggle_sidebar();
        state.set_filter_sets_window(rows);
        state
    }

    #[test]
    fn the_digits_number_the_window_rather_than_the_entries() {
        let mut state = sidebar_state(9);
        let saved = entries(14);
        state.filter_sets_page(1, saved.len());

        let view = render_filter_sets(&state, &saved).expect("the sidebar is open");

        assert_eq!(state.filter_sets_page_start(), 9);
        assert_eq!(view.rows.len(), 5, "only the tail is left");
        assert_eq!(view.rows[0].number, 1, "the tenth entry is row 1");
        assert_eq!(view.rows[0].name, "set 09");
        assert_eq!(view.rows[4].number, 5);
        assert_eq!(view.page.as_deref(), Some("10-14 of 14"));
    }

    #[test]
    fn a_list_that_fits_says_nothing_about_pages() {
        let state = sidebar_state(9);
        let saved = entries(4);

        let view = render_filter_sets(&state, &saved).expect("the sidebar is open");

        assert_eq!(view.rows.len(), 4);
        assert_eq!(view.page, None);
        assert!(view.counts.is_empty());
    }

    #[test]
    fn paging_forward_from_the_last_page_is_a_no_op() {
        let mut state = sidebar_state(9);
        let saved = entries(14);

        state.filter_sets_page(1, saved.len());
        state.filter_sets_page(1, saved.len());

        let view = render_filter_sets(&state, &saved).expect("the sidebar is open");
        assert_eq!(view.page.as_deref(), Some("10-14 of 14"));
    }

    #[test]
    fn a_short_sidebar_pages_by_what_it_can_actually_show() {
        // The window is what fits, not a fixed nine: a row the pane clips is
        // a row whose digit would load something invisible.
        let mut state = sidebar_state(3);
        let saved = entries(14);

        state.filter_sets_page(1, saved.len());

        let view = render_filter_sets(&state, &saved).expect("the sidebar is open");
        assert_eq!(state.filter_sets_page_start(), 3);
        assert_eq!(view.rows.len(), 3);
        assert_eq!(view.rows[0].name, "set 03");
        assert_eq!(view.page.as_deref(), Some("4-6 of 14"));
    }

    #[test]
    fn the_loaded_entry_is_marked_and_named_at_the_top() {
        let mut state = sidebar_state(9);
        let saved = entries(3);
        state.filter_set_bind("set 01");

        let view = render_filter_sets(&state, &saved).expect("the sidebar is open");

        assert_eq!(view.current.name.as_deref(), Some("set 01"));
        assert_eq!(
            view.rows.iter().filter(|row| row.loaded).count(),
            1,
            "exactly one row is the loaded one"
        );
        assert!(view.rows[1].loaded);
    }

    #[test]
    fn an_unbound_panel_reads_as_unnamed() {
        let state = sidebar_state(9);

        let view = render_filter_sets(&state, &entries(2)).expect("the sidebar is open");
        let lines = filter_sets_lines(&view, &Theme::default(), 24);

        assert_eq!(view.current.name, None);
        assert!(lines[0].to_string().contains("unnamed"), "{:?}", lines[0]);
    }

    #[test]
    fn a_closed_sidebar_renders_nothing() {
        assert!(render_filter_sets(&TaskState::new(), &entries(3)).is_none());
    }

    #[test]
    fn every_sidebar_line_is_one_line_and_fits_the_pane() {
        let mut state = sidebar_state(9);
        state.filter_set_bind("set 00");
        let view = render_filter_sets(&state, &entries(9)).expect("the sidebar is open");

        let width = SIDEBAR_WIDTH as usize - 2;
        let lines = filter_sets_lines(&view, &Theme::default(), width);

        // Three header lines and one line per entry: the window arithmetic
        // stays a plain index only while that holds.
        assert_eq!(lines.len(), 3 + 9);
        for line in &lines {
            assert!(
                visible_width(&line.to_string()) <= width,
                "a sidebar line overflowed: {line:?}"
            );
        }
    }

    #[test]
    fn the_sidebar_is_dropped_rather_than_squeezing_the_filter_rows() {
        // The three columns of label, match mode, and value are what the pane
        // is for; below the minimum the sidebar gives way instead.
        let pane = |width| Rect::new(0, 1, width, 11);

        let (sidebar, panel) = split_sidebar(pane(60), true);
        assert_eq!(sidebar, None, "no room for both at 60");
        assert_eq!(panel.width, 60, "and the panel keeps the whole pane");

        let (sidebar, panel) = split_sidebar(pane(80), true);
        assert_eq!(sidebar, None, "nor at 80");
        assert_eq!(panel.width, 80);

        let (sidebar, panel) = split_sidebar(pane(120), true);
        let sidebar = sidebar.expect("room for both at 120");
        assert_eq!(sidebar.width, SIDEBAR_WIDTH);
        assert_eq!(sidebar.x, 0);
        assert_eq!(panel.x, SIDEBAR_WIDTH);
        assert_eq!(sidebar.width + panel.width, 120);
        assert!(panel.width >= MIN_FILTER_WIDTH);
    }

    #[test]
    fn a_hidden_sidebar_leaves_the_pane_alone() {
        let area = Rect::new(0, 1, 200, 11);

        assert_eq!(split_sidebar(area, false), (None, area));
    }

    #[test]
    fn the_prompt_shows_the_caret_where_it_actually_sits() {
        let mut state = sidebar_state(9);
        state.filter_set_bind("Mine");
        state.filter_set_prompt_save();
        state.filter_set_prompt_move_caret(-2);

        let view = render_filter_sets(&state, &entries(1)).expect("the sidebar is open");
        let prompt = view.prompt.as_ref().expect("the prompt is open");
        let theme = Theme::default();
        let rendered = prompt_footer_line(prompt, &theme, 60).to_string();

        assert!(rendered.contains("save as"), "{rendered}");
        assert!(
            rendered.contains(&format!("Mi{}ne", theme.glyphs.edit_cursor)),
            "{rendered}"
        );
    }

    #[test]
    fn the_delete_prompt_names_what_it_would_remove() {
        let mut state = sidebar_state(9);
        state.filter_set_bind("Sprint triage");
        assert!(state.filter_set_prompt_delete());

        let view = render_filter_sets(&state, &entries(1)).expect("the sidebar is open");
        let rendered =
            prompt_footer_line(view.prompt.as_ref().expect("open"), &Theme::default(), 60)
                .to_string();

        assert!(rendered.contains("delete Sprint triage?"), "{rendered}");
        assert!(rendered.contains("y/n"), "{rendered}");
    }

    /// The filter pane borrows it when the sidebar has been dropped for
    /// width, so `w` is never a blind edit.
    #[test]
    fn the_prompt_is_readable_without_a_sidebar_to_hang_it_on() {
        let mut state = sidebar_state(9);
        state.filter_set_prompt_save();

        assert_eq!(split_sidebar(Rect::new(0, 1, 80, 11), true).0, None);
        assert!(super::prompt_line(&state).is_some());
    }

    #[test]
    fn the_load_confirmation_names_the_entry_the_digit_addressed() {
        let mut state = sidebar_state(9);
        state.filter_set_prompt_confirm_load("Sprint triage");

        let view = render_filter_sets(&state, &entries(4)).expect("the sidebar is open");
        let rendered =
            prompt_footer_line(view.prompt.as_ref().expect("open"), &Theme::default(), 60)
                .to_string();

        assert!(rendered.contains("discard for Sprint triage?"), "{rendered}");
        assert!(rendered.contains("y/n"), "{rendered}");
    }

    #[test]
    fn a_prompt_too_long_for_the_border_gives_up_its_label_not_its_answer() {
        // A sidebar is 24 columns and a saved name can be any length, so
        // something has to give — and `y/n` is the half being answered.
        let mut state = sidebar_state(9);
        state.filter_set_prompt_confirm_load("A name far longer than any sidebar");
        let view = render_filter_sets(&state, &entries(1)).expect("the sidebar is open");
        let theme = Theme::default();

        let width = SIDEBAR_WIDTH as usize - 2;
        let line = prompt_footer_line(view.prompt.as_ref().expect("open"), &theme, width);

        assert!(
            visible_width(&line.to_string()) <= width,
            "the prompt overflowed the border: {line:?}"
        );
        assert!(line.to_string().contains("y/n"), "{line:?}");
    }

    #[test]
    fn a_write_that_failed_takes_the_prompt_line_and_says_so() {
        let mut state = sidebar_state(9);
        state.set_filter_sets_notice("could not save: denied");

        let view = render_filter_sets(&state, &entries(1)).expect("the sidebar is open");
        let prompt = view.prompt.as_ref().expect("the report has somewhere to go");

        assert!(prompt.error);
        assert!(prompt.text.contains("could not save"));
    }
}
