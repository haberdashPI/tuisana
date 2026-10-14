//! The description pane: a task's notes, down the right of the task table.
//!
//! The Gantt chart gets away with sharing the table's interior because it is
//! drawn *per row* — `src/ui/gantt.rs` returns spans that each row's own
//! `Line` carries, which is what lets the cursor highlight and the vertical
//! scroll cover it for free. A description has nothing to say row by row: it
//! is one block of wrapped prose about one task, and tying it to rows would
//! make it scroll with them. So it gets a pane of its own instead.
//!
//! [`split_description`] lives here rather than in [`crate::ui::layout`] for
//! the same reason `split_sidebar` lives in `filter_sets`: this is a split of
//! one pane's region, not a region of the frame.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{
    app::{
        description::{normalize, DescriptionContent, DescriptionState},
        task::TaskState,
    },
    asana::TaskComment,
    config::Mode,
    domain::CivilDate,
    ui::{
        chrome::{self, Chip, PaneMessage, Tone},
        date,
        text::{visible_width, wrap_to_width},
        theme::Theme,
    },
};

/// Widest a list marker may be before its hanging indent is given up.
///
/// A deeply nested item in a narrow pane would otherwise indent its text into
/// a column two words wide, which is less readable than no indent at all.
const MAX_HANGING_INDENT: usize = 8;

/// How much of the task pane the description asks for.
const DESCRIPTION_SHARE: f64 = 0.38;
/// Narrowest the pane is worth drawing.
///
/// Below this, prose wraps into a column of broken words and reads worse than
/// not being there.
const MIN_DESCRIPTION_WIDTH: u16 = 28;
/// Widest it will grow.
///
/// Past this it is a wall of text, and the width it is taking is coming off
/// the table that says which task the text belongs to.
const MAX_DESCRIPTION_WIDTH: u16 = 60;
/// What the table keeps, whatever the description would like.
const MIN_TABLE_WIDTH: u16 = 32;

/// Takes the description pane off the right of the task pane's region.
///
/// Returns the region the table gets and, when there is room for it, the one
/// the description gets. `None` means the pane is closed or the terminal is
/// too narrow to hold both — the caller reports the second case.
pub fn split_description(area: Rect, visible: bool) -> (Rect, Option<Rect>) {
    if !visible {
        return (area, None);
    }

    let wanted = ((area.width as f64) * DESCRIPTION_SHARE).floor() as u16;
    let width = wanted.clamp(MIN_DESCRIPTION_WIDTH, MAX_DESCRIPTION_WIDTH);
    if area.width.saturating_sub(width) < MIN_TABLE_WIDTH {
        return (area, None);
    }

    (
        Rect {
            width: area.width - width,
            ..area
        },
        Some(Rect {
            x: area.x + (area.width - width),
            width,
            ..area
        }),
    )
}

/// Whether the pane is open but had to be dropped for want of room.
///
/// The task pane says so with a chip, the way it does for a chart too narrow
/// to draw: a pane that silently does not appear reads as a broken key.
pub fn too_narrow(area: Option<Rect>, state: &DescriptionState) -> bool {
    state.visible() && area.is_some_and(|area| split_description(area, true).1.is_none())
}

/// One source line, classified by how it wants to be laid out.
#[derive(Debug, PartialEq, Eq)]
enum Block {
    /// A paragraph.
    Prose(String),
    /// A bulleted or numbered item.
    Item {
        /// The leading whitespace and the marker, drawn on the first line.
        prefix: String,
        /// The text after the marker.
        text: String,
    },
}

impl Block {
    fn is_item(&self) -> bool {
        matches!(self, Self::Item { .. })
    }
}

/// Reads one line as prose or as a list item.
///
/// A marker is a bullet (`-`, `*`, `+`, `•`) or a number (`1.`, `2)`) followed
/// by a space. The prefix that comes back includes the line's own indent, so a
/// nested item keeps its nesting.
fn classify(line: &str) -> Block {
    let indent = line.len() - line.trim_start().len();
    let (indent, rest) = line.split_at(indent);
    let marker = bullet_marker(rest).or_else(|| number_marker(rest));

    match marker {
        Some(marker) => {
            let text = rest[marker.len()..].trim_start();
            Block::Item {
                prefix: format!("{indent}{marker}"),
                text: text.to_string(),
            }
        }
        None => Block::Prose(line.trim_start().to_string()),
    }
}

/// `- `, `* `, `+ `, or `• ` at the start of `rest`.
fn bullet_marker(rest: &str) -> Option<&str> {
    let mut chars = rest.chars();
    let bullet = chars.next()?;
    if !matches!(bullet, '-' | '*' | '+' | '•') {
        return None;
    }
    // The space is what makes it a marker: `-10 degrees` is prose, and a
    // horizontal rule of dashes is not a list of one empty item.
    chars.next().filter(|next| *next == ' ')?;
    Some(&rest[..bullet.len_utf8() + 1])
}

/// `1. ` or `1) ` at the start of `rest`.
fn number_marker(rest: &str) -> Option<&str> {
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let mut chars = rest[digits..].chars();
    chars.next().filter(|next| matches!(next, '.' | ')'))?;
    chars.next().filter(|next| *next == ' ')?;
    Some(&rest[..digits + 2])
}

/// Wraps a description into the lines the pane draws.
///
/// Three things this does that plain [`wrap_to_width`] cannot, all of them
/// about reading prose rather than reading a message:
///
/// 1. **A blank line between paragraphs.** Asana's editor makes a new block
///    every time Enter is pressed, and `notes` separates those with a single
///    `\n` — so taking a newline at face value runs every paragraph of a real
///    description into the one before it. A source blank line is dropped and
///    re-inserted here instead, so `\n` and `\n\n` read the same and the
///    layout is the same however the text was typed.
/// 2. **A hanging indent on a wrapped list item**, so the second line of an
///    item sits under the first line's text rather than under its bullet.
///    Without it `- carrier pickup window confirmed in\nwriting` reads as two
///    items, one of which is the word "writing".
/// 3. **No blank line between items of one list**, because a list is one
///    thing. The blank goes before it and after it, not through it.
fn layout(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let blocks = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(classify)
        .collect::<Vec<_>>();

    let mut lines = Vec::new();
    let mut previous: Option<&Block> = None;

    for block in &blocks {
        if let Some(previous) = previous {
            // Between two items of one list, nothing: the list is the unit.
            if !(previous.is_item() && block.is_item()) {
                lines.push(String::new());
            }
        }

        match block {
            Block::Prose(text) => lines.extend(wrap_to_width(text, width)),
            Block::Item { prefix, text } => {
                let indent = visible_width(prefix);
                // A marker as wide as the whole pane leaves no budget to take
                // it out of. Nothing here can lay that out, so the marker
                // stops being one: the line goes through as prose, which
                // breaks long words rather than overrunning.
                if indent >= width {
                    lines.extend(wrap_to_width(&format!("{prefix}{text}"), width));
                    previous = Some(block);
                    continue;
                }
                // The marker's width always comes off the budget, hang or no
                // hang: it is drawn on the first line either way, and a line
                // measured without it overruns the pane.
                let body_width = width.saturating_sub(indent).max(1);
                // Too deep to hang from: the continuations go flush rather
                // than into a gutter with no room left for words.
                let hang = indent <= MAX_HANGING_INDENT;

                for (index, line) in wrap_to_width(text, body_width).into_iter().enumerate() {
                    match (index, hang) {
                        (0, _) => lines.push(format!("{prefix}{line}")),
                        (_, true) => lines.push(format!("{}{line}", " ".repeat(indent))),
                        (_, false) => lines.push(line),
                    }
                }
            }
        }
        previous = Some(block);
    }

    lines
}

/// One drawable line of the pane's body.
///
/// Semantic rather than styled: the layout decides *what* each line is and
/// `render` decides how it looks, which keeps the wrapping testable without a
/// theme and keeps the one place that constructs a style in the one module
/// that is allowed to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyLine {
    /// Prose — the description, or a comment's text.
    Text(String),
    /// The rule that separates the description from the comments.
    Heading(String),
    /// Who wrote the comment below, and when.
    Byline { author: String, when: Option<String> },
}

/// Lays the comments out under the description.
///
/// The two are one document, deliberately: one scroll covers both, and the
/// `line N/M` count is over the lot. A comment is a byline and then its text,
/// laid out by the same [`layout`] rules as the description — people write
/// lists in comments too.
fn comment_lines(comments: &[TaskComment], width: usize, today: CivilDate) -> Vec<BodyLine> {
    let mut lines = vec![BodyLine::Heading(heading(comments.len(), width))];

    for comment in comments {
        lines.push(BodyLine::Byline {
            author: comment
                .author
                .clone()
                .unwrap_or_else(|| "(unknown)".to_string()),
            when: comment
                .created_at
                .as_deref()
                .and_then(|stamp| comment_date(stamp, today)),
        });
        lines.extend(
            layout(&normalize(&comment.text), width)
                .into_iter()
                .map(BodyLine::Text),
        );
        // One blank line after each, so two comments do not read as one.
        lines.push(BodyLine::Text(String::new()));
    }

    lines.pop();
    lines
}

/// `─── 3 comments ───`, centred, or as much of it as fits.
fn heading(count: usize, width: usize) -> String {
    let label = match count {
        1 => " 1 comment ".to_string(),
        other => format!(" {other} comments "),
    };
    let rule = width.saturating_sub(visible_width(&label));
    if rule < 2 {
        return label.trim().to_string();
    }

    let left = rule / 2;
    format!(
        "{}{label}{}",
        "─".repeat(left),
        "─".repeat(rule - left)
    )
}

/// The date half of an ISO 8601 timestamp, read as a short absolute date.
///
/// Absolute rather than relative: a comment is always in the past, so it has
/// no urgency to report, and `3 days ago` is less useful than the date when
/// the next comment is from a year before it.
fn comment_date(stamp: &str, today: CivilDate) -> Option<String> {
    let date = CivilDate::parse(stamp.get(..10)?)?;
    Some(date::short_date_from(date, today))
}

/// The drawable snapshot of the pane.
pub struct DescriptionView {
    /// The laid-out body, or `None` when there is a message to draw instead.
    pub lines: Option<Vec<BodyLine>>,
    /// What to say when there is no text: loading, empty, absent, or failed.
    pub message: Option<PaneMessage>,
    /// The first visible line and the total, for the title.
    pub counts: Vec<Chip>,
    /// How many lines fall below the pane, for the bottom border.
    pub below: usize,
}

/// Resolves the pane's state into what to draw at this width and height.
///
/// Takes `&mut TaskState` because the wrap is only known here: the line count
/// and the body height go back onto the state so a scroll key has a page size
/// to work from, exactly as `set_visible_columns` reports what the table drew.
pub fn description_view(
    state: &mut TaskState,
    inner_width: usize,
    body_height: usize,
) -> DescriptionView {
    let modified_at = state.description_target_modified_at();
    let content = state.description().content(modified_at.as_deref());

    // One cell of gutter each side, so the text does not sit on the border.
    let text_width = inner_width.saturating_sub(2).max(1);
    let lines = match &content {
        DescriptionContent::Body { notes, comments } => {
            let mut lines = layout(notes, text_width)
                .into_iter()
                .map(BodyLine::Text)
                .collect::<Vec<_>>();
            if !comments.is_empty() {
                // A blank line before the rule, unless the description was
                // empty and the rule is the first thing in the pane.
                if !lines.is_empty() {
                    lines.push(BodyLine::Text(String::new()));
                }
                lines.extend(comment_lines(comments, text_width, date::today()));
            }
            lines
        }
        _ => Vec::new(),
    };

    state
        .description_mut()
        .set_metrics(lines.len(), body_height);
    let scroll = state.description().scroll();

    let message = match &content {
        DescriptionContent::Body { .. } => None,
        DescriptionContent::Loading => Some(PaneMessage::new("loading…", Tone::Muted)),
        DescriptionContent::Empty => Some(PaneMessage::new(
            "(no description or comments)",
            Tone::Muted,
        )),
        DescriptionContent::NoTask => {
            Some(PaneMessage::new("no task under the cursor", Tone::Muted))
        }
        DescriptionContent::Failed(message) => Some(
            PaneMessage::new(format!("could not load: {message}"), Tone::Warn)
                .with_hint("r to try again"),
        ),
    };

    // `line 0/0` for an empty description rather than nothing at all: the
    // count is how the reader tells "this task has no description" from "this
    // pane has not caught up yet".
    let counts = match &content {
        DescriptionContent::Body { .. } | DescriptionContent::Empty => vec![Chip::new(format!(
            "line {}/{}",
            lines.len().min(scroll + 1),
            lines.len()
        ))],
        _ => Vec::new(),
    };

    DescriptionView {
        below: lines.len().saturating_sub(scroll + body_height),
        lines: (!lines.is_empty()).then_some(lines),
        message,
        counts,
    }
}

/// Draws the pane into its region.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    mode: Mode,
    focused: bool,
    view: &DescriptionView,
    scroll: usize,
) {
    let mut block = chrome::pane_block(theme, focused, mode, "Description", &view.counts);
    // The count of what is still below rides the bottom border, the way the
    // chart's legend does, so saying it costs no body line. Only when there is
    // something below: a border that always says `+0 more lines` is noise.
    if view.below > 0 {
        let label = format!(" +{} more lines ", view.below);
        block = block.title_bottom(
            Line::from(ratatui::text::Span::styled(label, theme.muted)).right_aligned(),
        );
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(message) = &view.message {
        chrome::render_pane_message(frame, inner, theme, message);
        return;
    }

    let Some(lines) = &view.lines else {
        return;
    };

    let body = lines
        .iter()
        .map(|line| body_line(line, theme))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(body).scroll((scroll as u16, 0)),
        inner,
    );
}

/// Styles one body line. The gutter cell is added here, so every kind of line
/// is inset by the same amount whatever it is made of.
fn body_line(line: &BodyLine, theme: &Theme) -> Line<'static> {
    let spans = match line {
        BodyLine::Text(text) => vec![Span::styled(format!(" {text}"), theme.text)],
        // Muted, like every other rule in the app: it is a divider, not a
        // heading competing with the text under it.
        BodyLine::Heading(text) => vec![Span::styled(format!(" {text}"), theme.muted)],
        BodyLine::Byline { author, when } => {
            let mut spans = vec![
                Span::raw(" "),
                Span::styled(author.clone(), theme.subtitle),
            ];
            if let Some(when) = when {
                spans.push(Span::styled(
                    format!(" {} {when}", theme.glyphs.chip_sep),
                    theme.muted,
                ));
            }
            spans
        }
    };
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::{
        classify, comment_lines, layout, split_description, Block, BodyLine, CivilDate,
        TaskComment, MAX_DESCRIPTION_WIDTH, MIN_DESCRIPTION_WIDTH, MIN_TABLE_WIDTH,
    };
    use crate::ui::text::visible_width;
    use ratatui::layout::Rect;

    fn pane(width: u16) -> Rect {
        Rect::new(0, 5, width, 20)
    }

    fn item(prefix: &str, text: &str) -> Block {
        Block::Item {
            prefix: prefix.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn a_marker_is_a_bullet_or_a_number_followed_by_a_space() {
        assert_eq!(classify("- one"), item("- ", "one"));
        assert_eq!(classify("* one"), item("* ", "one"));
        assert_eq!(classify("+ one"), item("+ ", "one"));
        assert_eq!(classify("• one"), item("• ", "one"));
        assert_eq!(classify("1. one"), item("1. ", "one"));
        assert_eq!(classify("2) one"), item("2) ", "one"));
        assert_eq!(classify("10. one"), item("10. ", "one"));
        // The indent comes along, so a nested item stays nested.
        assert_eq!(classify("  - one"), item("  - ", "one"));
    }

    #[test]
    fn what_merely_starts_with_a_dash_or_a_digit_is_prose() {
        // No space after the marker.
        assert_eq!(classify("-10 degrees"), Block::Prose("-10 degrees".to_string()));
        assert_eq!(classify("1.5x the volume"), Block::Prose("1.5x the volume".to_string()));
        // A rule of dashes is not a list of one empty item.
        assert_eq!(classify("---"), Block::Prose("---".to_string()));
        assert_eq!(classify("4412 run"), Block::Prose("4412 run".to_string()));
    }

    #[test]
    fn paragraphs_are_spaced_however_the_source_separated_them() {
        // Asana writes one newline between blocks; a pasted description may
        // have two. They have to read the same.
        let one = layout("First para.\nSecond para.", 40);
        let two = layout("First para.\n\nSecond para.", 40);

        assert_eq!(one, vec!["First para.", "", "Second para."]);
        assert_eq!(one, two);
        assert_eq!(layout("A.\n\n\n\nB.", 40), one.iter().map(|line| line
            .replace("First para.", "A.")
            .replace("Second para.", "B."))
            .collect::<Vec<_>>());
    }

    #[test]
    fn a_wrapped_item_hangs_under_its_own_text() {
        let lines = layout("- carrier pickup window confirmed in writing", 24);

        assert_eq!(lines, vec![
            "- carrier pickup window",
            "  confirmed in writing",
        ]);
    }

    #[test]
    fn a_list_is_one_block_and_is_not_spaced_through() {
        let lines = layout("Before it ships:\n- one\n- two\nAfter.", 40);

        assert_eq!(lines, vec![
            "Before it ships:",
            "",
            "- one",
            "- two",
            "",
            "After.",
        ]);
    }

    #[test]
    fn a_nested_item_keeps_its_indent_on_both_lines() {
        let lines = layout("  - labels printed from the template", 22);

        assert_eq!(lines, vec![
            "  - labels printed",
            "    from the template",
        ]);
    }

    #[test]
    fn a_numbered_item_hangs_by_the_width_of_its_own_number() {
        let lines = layout("9. nine\n10. ten and more words", 16);

        assert_eq!(lines, vec![
            "9. nine",
            "10. ten and more",
            "    words",
        ]);
    }

    #[test]
    fn an_indent_too_deep_to_hang_from_drops_the_hang_but_never_the_budget() {
        // Ten cells of marker: past the hang limit, so the continuations go
        // flush — but the first line still has to leave room for the marker.
        let lines = layout("        - one two three four", 20);

        assert_eq!(lines[0], "        - one two");
        assert_eq!(lines[1], "three four", "flush, not in a ten-cell gutter");
        for line in &lines {
            assert!(visible_width(line) <= 20, "{line:?} overruns the pane");
        }
    }

    /// Nothing the layout emits may be wider than the pane, whatever the
    /// marker depth or the width — the renderer does not clip.
    #[test]
    fn no_laid_out_line_ever_overruns_the_pane() {
        let text = "Lead in:\n\
                    - one two three four five\n\
                      - nested and rather longer\n\
                    10. numbered and longer still\n\
                            - very deeply nested indeed\n\
                    Trailing paragraph.";

        for width in 4..60 {
            for line in layout(text, width) {
                assert!(
                    visible_width(&line) <= width,
                    "at width {width}: {line:?} is {} cells",
                    visible_width(&line)
                );
            }
        }
    }

    fn said(author: Option<&str>, when: Option<&str>, text: &str) -> TaskComment {
        TaskComment {
            author: author.map(str::to_string),
            created_at: when.map(str::to_string),
            text: text.to_string(),
        }
    }

    fn today() -> CivilDate {
        CivilDate::parse("2026-10-15").expect("a date")
    }

    #[test]
    fn a_comment_is_a_byline_and_then_its_text() {
        let lines = comment_lines(
            &[said(Some("Alex Chen"), Some("2026-08-02T16:40:00.000Z"), "Looks good.")],
            30,
            today(),
        );

        assert_eq!(lines, vec![
            BodyLine::Heading("───────── 1 comment ──────────".to_string()),
            BodyLine::Byline {
                author: "Alex Chen".to_string(),
                when: Some("Aug 2".to_string()),
            },
            BodyLine::Text("Looks good.".to_string()),
        ]);
    }

    #[test]
    fn two_comments_are_separated_by_a_blank_and_the_last_does_not_trail_one() {
        let lines = comment_lines(
            &[
                said(Some("A"), None, "one"),
                said(Some("B"), None, "two"),
            ],
            30,
            today(),
        );

        assert_eq!(lines, vec![
            BodyLine::Heading("───────── 2 comments ─────────".to_string()),
            BodyLine::Byline { author: "A".to_string(), when: None },
            BodyLine::Text("one".to_string()),
            BodyLine::Text(String::new()),
            BodyLine::Byline { author: "B".to_string(), when: None },
            BodyLine::Text("two".to_string()),
        ]);
    }

    #[test]
    fn a_comment_with_no_author_is_named_rather_than_blank() {
        let lines = comment_lines(&[said(None, None, "hello")], 30, today());

        assert_eq!(lines[1], BodyLine::Byline {
            author: "(unknown)".to_string(),
            when: None,
        });
    }

    #[test]
    fn a_comment_from_another_year_says_which() {
        let lines = comment_lines(
            &[said(Some("Jo"), Some("2025-11-30T11:02:00.000Z"), "x")],
            30,
            today(),
        );

        assert_eq!(lines[1], BodyLine::Byline {
            author: "Jo".to_string(),
            when: Some("Nov 30 2025".to_string()),
        });
    }

    #[test]
    fn a_timestamp_that_is_not_one_leaves_the_byline_dateless() {
        // Rather than printing the raw value: a byline is two facts, and one
        // of them being unreadable is better said by its absence.
        let lines = comment_lines(&[said(Some("Jo"), Some("whenever"), "x")], 30, today());

        assert_eq!(lines[1], BodyLine::Byline {
            author: "Jo".to_string(),
            when: None,
        });
    }

    #[test]
    fn a_comment_is_laid_out_by_the_same_rules_as_a_description() {
        // People write lists in comments too.
        let lines = comment_lines(
            &[said(Some("A"), None, "Two things:\n- one of them is long enough to wrap")],
            28,
            today(),
        );

        assert_eq!(&lines[2..], &[
            BodyLine::Text("Two things:".to_string()),
            BodyLine::Text(String::new()),
            BodyLine::Text("- one of them is long enough".to_string()),
            BodyLine::Text("  to wrap".to_string()),
        ]);
    }

    #[test]
    fn the_heading_counts_in_words_and_fits_the_pane() {
        assert!(super::heading(1, 30).contains(" 1 comment "));
        assert!(super::heading(4, 30).contains(" 4 comments "));
        assert_eq!(visible_width(&super::heading(4, 30)), 30);
        // Too narrow for a rule: the label alone, rather than an overrun.
        assert_eq!(super::heading(4, 8), "4 comments");
    }

    #[test]
    fn a_closed_pane_leaves_the_region_alone() {
        let area = pane(120);

        assert_eq!(split_description(area, false), (area, None));
    }

    #[test]
    fn the_two_regions_are_adjacent_and_fill_the_pane() {
        let area = pane(120);
        let (table, description) = split_description(area, true);
        let description = description.expect("there is room at 120");

        assert_eq!(table.x, area.x);
        assert_eq!(table.x + table.width, description.x);
        assert_eq!(table.width + description.width, area.width);
        assert_eq!(description.y, area.y);
        assert_eq!(description.height, area.height);
    }

    #[test]
    fn the_share_is_bounded_at_both_ends() {
        // 38% of 120 is 45, which is inside the bounds.
        let (_, middling) = split_description(pane(120), true);
        assert_eq!(middling.expect("drawn").width, 45);

        // 38% of 80 is 30, also inside them.
        let (_, narrow) = split_description(pane(80), true);
        assert_eq!(narrow.expect("drawn").width, 30);

        // 38% of 60 is 22, below the floor.
        let (_, floored) = split_description(pane(60), true);
        assert_eq!(floored.expect("drawn").width, MIN_DESCRIPTION_WIDTH);

        // 38% of 400 is 152, above the ceiling.
        let (_, capped) = split_description(pane(400), true);
        assert_eq!(capped.expect("drawn").width, MAX_DESCRIPTION_WIDTH);
    }

    #[test]
    fn the_table_keeps_its_floor_or_the_pane_is_dropped() {
        // The narrowest pane that still fits: the floor plus the table's.
        let fits = MIN_DESCRIPTION_WIDTH + MIN_TABLE_WIDTH;
        assert!(split_description(pane(fits), true).1.is_some());

        let (table, dropped) = split_description(pane(fits - 1), true);
        assert_eq!(dropped, None, "the table wins");
        assert_eq!(table.width, fits - 1, "and keeps the whole region");
    }
}
