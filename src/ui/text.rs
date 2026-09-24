//! Width-aware text helpers shared by the pane renderers.
//!
//! Every function here measures in *display cells* rather than bytes or chars,
//! so wide CJK characters and multi-byte glyphs never break column alignment.

use ratatui::{
    style::{Modifier, Style},
    text::Span,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The display width of a string in terminal cells.
pub fn visible_width(value: &str) -> usize {
    value.width()
}

/// Truncates to `width` cells, hard-cutting without a marker.
pub fn truncate_to_width(value: &str, width: usize) -> String {
    if visible_width(value) <= width {
        return value.to_string();
    }

    let mut result = String::new();
    let mut current_width = 0usize;

    for ch in value.chars() {
        let char_width = ch.width().unwrap_or(0);
        if current_width + char_width > width {
            break;
        }
        result.push(ch);
        current_width += char_width;
    }

    result
}

/// Truncates to `width` cells, replacing the tail with `ellipsis`.
///
/// Used for every column so a clipped value always announces itself, rather
/// than silently reading as a shorter value.
pub fn truncate_with_ellipsis(value: &str, width: usize, ellipsis: &str) -> String {
    if visible_width(value) <= width {
        return value.to_string();
    }

    if width == 0 {
        return String::new();
    }

    let marker_width = visible_width(ellipsis).max(1);
    if width <= marker_width {
        return truncate_to_width(ellipsis, width);
    }

    let mut result = truncate_to_width(value, width - marker_width);
    result.push_str(ellipsis);
    result
}

/// Right-pads to exactly `width` cells, truncating with `ellipsis` if needed.
pub fn pad_cell(value: &str, width: usize, ellipsis: &str) -> String {
    let truncated = truncate_with_ellipsis(value, width, ellipsis);
    let padding = width.saturating_sub(visible_width(&truncated));
    format!("{truncated}{}", " ".repeat(padding))
}

/// Left-pads to exactly `width` cells, truncating with `ellipsis` if needed.
pub fn pad_cell_right_aligned(value: &str, width: usize, ellipsis: &str) -> String {
    let truncated = truncate_with_ellipsis(value, width, ellipsis);
    let padding = width.saturating_sub(visible_width(&truncated));
    format!("{}{truncated}", " ".repeat(padding))
}

/// Centers within exactly `width` cells, truncating with `ellipsis` if needed.
pub fn pad_cell_centered(value: &str, width: usize, ellipsis: &str) -> String {
    let truncated = truncate_with_ellipsis(value, width, ellipsis);
    let padding = width.saturating_sub(visible_width(&truncated));
    let left = padding / 2;
    format!(
        "{}{truncated}{}",
        " ".repeat(left),
        " ".repeat(padding - left)
    )
}

/// Wraps `value` to `width` cells, breaking on whitespace and keeping the
/// newlines already in it.
///
/// Every other helper here clips to one row, because a column is one row.
/// A message is not: an error worth reading is worth reading whole, and the
/// line breaks the sender put in it are part of what it says.
///
/// A word too long to fit on a line of its own is cut rather than allowed to
/// overflow — a gid or a url is still readable split across two rows, and
/// there is no width at which the pane could show it whole.
pub fn wrap_to_width(value: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();

    for paragraph in value.split('\n') {
        let before = lines.len();
        let mut current = String::new();

        for word in paragraph.split_whitespace() {
            for piece in split_to_width(word, width) {
                let joined = visible_width(&current) + 1 + visible_width(&piece);
                if !current.is_empty() && joined > width {
                    lines.push(std::mem::take(&mut current));
                }
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(&piece);
            }
        }

        if !current.is_empty() {
            lines.push(current);
        }
        // A blank line in the source is a blank line on screen: it is how the
        // sender separated one thing from the next.
        if lines.len() == before {
            lines.push(String::new());
        }
    }

    lines
}

/// Cuts a word with nowhere to break into `width`-cell pieces.
fn split_to_width(word: &str, width: usize) -> Vec<String> {
    if visible_width(word) <= width {
        return vec![word.to_string()];
    }

    let mut pieces = Vec::new();
    let mut rest = word;

    while visible_width(rest) > width {
        let mut head = truncate_to_width(rest, width);
        // A single glyph wider than the line would otherwise cut to nothing
        // and loop forever; one overflowing cell beats no progress.
        if head.is_empty() {
            let Some(first) = rest.chars().next() else {
                break;
            };
            head = first.to_string();
        }
        rest = &rest[head.len()..];
        pieces.push(head);
    }

    if !rest.is_empty() {
        pieces.push(rest.to_string());
    }

    pieces
}

/// A window of a value, sized to a column, with the caret inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaretWindow {
    /// The text to draw, including any ellipsis.
    pub text: String,
    /// Where the caret sits in `text`, as a char index.
    pub caret: usize,
    /// Where the window starts in the original value, as a char index.
    ///
    /// Handed back so the caller can keep it and pass it in again: that is
    /// what makes the window sticky rather than re-centred on every frame.
    pub start: usize,
}

/// The `width` cells of `text` that contain the caret.
///
/// An ellipsis marks each clipped side, and costs a cell from the window, so
/// the caret is never the character the ellipsis replaced. Measured in display
/// cells like everything else in this module, so a CJK title scrolls by
/// columns rather than by chars.
///
/// `start` is where the window sat last time. It is honoured unless the caret
/// has left it, which is what keeps the text still while the caret travels
/// through it — a window recomputed from scratch every frame slides under the
/// reader on every keystroke.
pub fn caret_window(
    text: &str,
    caret: usize,
    width: usize,
    ellipsis: &str,
    start: usize,
) -> CaretWindow {
    let chars = text.chars().collect::<Vec<_>>();
    let caret = caret.min(chars.len());
    if width == 0 {
        return CaretWindow {
            text: String::new(),
            caret: 0,
            start: 0,
        };
    }

    // The caret needs a cell of its own when it sits past the last character,
    // which is where a value that only just fits stops fitting.
    let natural = visible_width(text) + usize::from(caret == chars.len());
    if natural <= width {
        return CaretWindow {
            text: text.to_string(),
            caret,
            start: 0,
        };
    }

    let marker = visible_width(ellipsis).max(1);
    let mut start = start.min(chars.len()).min(caret);

    loop {
        let (end, used, right_clipped) = window_end(&chars, start, width, marker);
        let visible = match right_clipped {
            // The last cell is the ellipsis, so the caret cannot sit on it.
            true => caret < end,
            // Past the last character the caret needs a cell of its own.
            false => caret < end || used + marker_left(start, marker) < width,
        };

        if visible || start >= caret {
            let mut window = String::new();
            if start > 0 {
                window.push_str(ellipsis);
            }
            window.extend(chars[start..end].iter());
            if right_clipped {
                window.push_str(ellipsis);
            }
            let caret_offset = marker_left(start, marker)
                + visible_width(&chars[start..caret.min(end)].iter().collect::<String>());
            return CaretWindow {
                text: window,
                caret: caret_offset,
                start,
            };
        }

        start += 1;
    }
}

/// The cells the left-hand ellipsis costs, or zero when nothing is clipped.
fn marker_left(start: usize, marker: usize) -> usize {
    match start {
        0 => 0,
        _ => marker,
    }
}

/// How far a window starting at `start` reaches, and whether it clips.
fn window_end(
    chars: &[char],
    start: usize,
    width: usize,
    marker: usize,
) -> (usize, usize, bool) {
    let budget = width.saturating_sub(marker_left(start, marker));
    let fill = |budget: usize| {
        let mut end = start;
        let mut used = 0usize;
        while end < chars.len() {
            let char_width = chars[end].width().unwrap_or(0);
            if used + char_width > budget {
                break;
            }
            used += char_width;
            end += 1;
        }
        (end, used)
    };

    let (end, used) = fill(budget);
    if end == chars.len() {
        return (end, used, false);
    }

    // Something is left over, so the right-hand ellipsis has to be paid for.
    let (end, used) = fill(budget.saturating_sub(marker));
    (end, used, true)
}

/// Splits a value around the caret so the caret can be drawn as a style.
///
/// The caret used to be a glyph spliced into the text, which pushed the
/// characters after it along and read as a stray space that wandered as the
/// caret moved. Reversing the character *under* the caret instead costs no
/// columns, so the text stays put while the caret travels through it. Only at
/// the very end, where there is no character to reverse, does a cell get added.
pub fn caret_spans(text: &str, caret: Option<usize>, style: Style) -> Vec<Span<'static>> {
    let Some(caret) = caret else {
        return vec![Span::styled(text.to_string(), style)];
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

    let caret_style = style.add_modifier(Modifier::REVERSED);
    let mut spans = Vec::with_capacity(3);
    if !head.is_empty() {
        spans.push(Span::styled(head, style));
    }
    spans.push(Span::styled(
        under.map_or_else(|| " ".to_string(), |ch| ch.to_string()),
        caret_style,
    ));
    if !tail.is_empty() {
        spans.push(Span::styled(tail, style));
    }
    spans
}

/// Repeats `glyph` until it fills `width` cells.
pub fn fill(glyph: &str, width: usize) -> String {
    let glyph_width = visible_width(glyph).max(1);
    glyph.repeat(width / glyph_width)
}

/// The total display width of a run of spans.
pub fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| visible_width(span.content.as_ref()))
        .sum()
}

/// Pads a run of spans with trailing blanks until it is `width` cells wide.
///
/// Rows are padded rather than left ragged so a row background (the cursor
/// band, zebra striping) covers the full pane width.
pub fn pad_spans(mut spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let current = spans_width(&spans);
    if current < width {
        spans.push(Span::raw(" ".repeat(width - current)));
    }
    spans
}

/// Extracts the `width` cells starting at `offset` from a run of spans.
///
/// This is how horizontal scrolling works: the full row is built once at its
/// natural width, then a window is sliced out of it. Styles are preserved per
/// span and the result is padded to exactly `width`.
pub fn slice_spans(spans: &[Span<'static>], offset: usize, width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }

    let mut cursor = 0usize;
    let end = offset.saturating_add(width);
    let mut out: Vec<Span<'static>> = Vec::new();

    for span in spans {
        let mut segment = String::new();
        let style = span.style;

        for ch in span.content.chars() {
            let char_width = ch.width().unwrap_or(0);
            let next_cursor = cursor + char_width;
            if next_cursor > offset && cursor < end {
                segment.push(ch);
            }
            cursor = next_cursor;
            if cursor >= end {
                break;
            }
        }

        if !segment.is_empty() {
            out.push(Span::styled(segment, style));
        }

        if cursor >= end {
            break;
        }
    }

    pad_spans(out, width)
}

/// Drops spans from the end until the run fits in `width` cells.
///
/// Used by the hint bar, which sheds the least important hints on a narrow
/// terminal instead of wrapping onto a second line.
pub fn clip_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;

    for span in spans {
        let span_width = visible_width(span.content.as_ref());
        if used + span_width > width {
            break;
        }
        used += span_width;
        out.push(span);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{
        caret_window, clip_spans, fill, pad_cell, pad_cell_centered, pad_cell_right_aligned,
        pad_spans, slice_spans, truncate_with_ellipsis, visible_width, wrap_to_width,
    };
    use ratatui::text::Span;

    /// The caret has to land on a cell the window actually draws, and the
    /// window has to fit the column: a caret past the cut is the whole bug
    /// this replaces truncation to fix.
    fn assert_window(text: &str, caret: usize, width: usize, start: usize) -> super::CaretWindow {
        let window = caret_window(text, caret, width, "…", start);
        assert!(
            visible_width(&window.text) <= width,
            "{:?} is wider than {width}",
            window.text
        );
        assert!(
            window.caret <= visible_width(&window.text),
            "the caret at {} is outside {:?}",
            window.caret,
            window.text
        );
        // The caret past the last character needs a cell of its own, and it
        // has to be a cell the column actually has.
        assert!(window.caret < width, "the caret at {} needs a cell", window.caret);
        window
    }

    #[test]
    fn a_caret_window_keeps_the_caret_visible_at_both_ends() {
        let text = "Ship the release before the summit";

        let start = assert_window(text, 0, 12, 0);
        assert_eq!(start.text, "Ship the re…");
        assert_eq!(start.caret, 0);

        let end = assert_window(text, text.chars().count(), 12, 0);
        assert!(end.text.starts_with('…'), "{:?} marks the clipped left", end.text);
        assert_eq!(
            end.caret,
            visible_width(&end.text),
            "the caret sits in the blank past the last character"
        );

        let middle = assert_window(text, 20, 12, 0);
        assert!(middle.text.ends_with('…'), "{:?} marks the clipped right", middle.text);
    }

    #[test]
    fn a_value_that_fits_is_not_windowed_at_all() {
        let window = assert_window("Ship", 4, 12, 0);

        assert_eq!(window.text, "Ship");
        assert_eq!(window.start, 0);
    }

    #[test]
    fn a_window_only_moves_when_the_caret_would_leave_it() {
        let text = "Ship the release before the summit";
        let scrolled = caret_window(text, 30, 12, "…", 0).start;

        // The caret steps back one; the window it was already in still holds
        // it, so the text does not slide.
        let held = caret_window(text, 29, 12, "…", scrolled).start;

        assert_eq!(held, scrolled);
    }

    #[test]
    fn a_caret_window_counts_a_wide_character_as_two_cells() {
        let window = assert_window("日本語のタスク", 6, 9, 0);

        assert!(visible_width(&window.text) <= 9);
    }

    #[test]
    fn truncates_with_an_ellipsis_and_never_exceeds_the_width() {
        assert_eq!(truncate_with_ellipsis("short", 10, "…"), "short");
        assert_eq!(truncate_with_ellipsis("Priority", 5, "…"), "Prio…");
        assert_eq!(visible_width(&truncate_with_ellipsis("Priority", 5, "…")), 5);
        assert_eq!(truncate_with_ellipsis("Priority", 1, "…"), "…");
        assert_eq!(truncate_with_ellipsis("Priority", 0, "…"), "");
    }

    #[test]
    fn pads_to_exact_widths_in_every_alignment() {
        assert_eq!(pad_cell("ab", 5, "…"), "ab   ");
        assert_eq!(pad_cell_right_aligned("ab", 5, "…"), "   ab");
        assert_eq!(pad_cell_centered("ab", 6, "…"), "  ab  ");
        assert_eq!(pad_cell_centered("ab", 5, "…"), " ab  ");
    }

    #[test]
    fn measures_wide_characters_in_display_cells() {
        assert_eq!(visible_width("日本"), 4);
        assert_eq!(visible_width(&pad_cell("日本", 6, "…")), 6);
        // A wide character cannot be split, so truncation may land one cell
        // short of the budget. It must never exceed it.
        assert_eq!(visible_width(&truncate_with_ellipsis("日本語", 4, "…")), 3);
        assert!(visible_width(&truncate_with_ellipsis("日本語", 5, "…")) <= 5);
    }

    #[test]
    fn fills_a_rule_to_the_requested_width() {
        assert_eq!(fill("─", 4), "────");
        assert_eq!(visible_width(&fill("·", 7)), 7);
    }

    #[test]
    fn slices_a_scroll_window_and_pads_it_to_width() {
        let spans = vec![Span::raw("abcdef"), Span::raw("ghij")];

        let sliced = slice_spans(&spans, 4, 4);

        assert_eq!(
            sliced
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "efgh"
        );
        assert_eq!(super::spans_width(&sliced), 4);
        assert_eq!(super::spans_width(&slice_spans(&spans, 8, 6)), 6);
    }

    #[test]
    fn pads_and_clips_span_runs() {
        assert_eq!(super::spans_width(&pad_spans(vec![Span::raw("ab")], 5)), 5);

        let clipped = clip_spans(
            vec![Span::raw("keep"), Span::raw("also"), Span::raw("drop")],
            8,
        );

        assert_eq!(clipped.len(), 2);
    }

    #[test]
    fn wrapping_breaks_on_spaces_and_keeps_the_newlines_it_was_given() {
        assert_eq!(
            wrap_to_width("could not update 1 of 1", 12),
            vec!["could not", "update 1 of", "1"]
        );
        // The break the sender wrote is a break, even where the line would
        // have fitted.
        assert_eq!(
            wrap_to_width("first\nsecond", 40),
            vec!["first", "second"]
        );
        assert_eq!(
            wrap_to_width("one\n\ntwo", 40),
            vec!["one", "", "two"],
            "a blank line separates, so it survives"
        );
    }

    #[test]
    fn a_word_with_nowhere_to_break_is_cut_rather_than_overflowing() {
        let lines = wrap_to_width("gid:1234567890123456", 8);

        assert!(lines.iter().all(|line| visible_width(line) <= 8), "{lines:?}");
        assert_eq!(lines.concat(), "gid:1234567890123456");
    }

    #[test]
    fn wrapping_measures_in_cells_so_a_wide_glyph_takes_two() {
        // Four CJK characters are eight cells, so six cells holds three.
        let lines = wrap_to_width("日本語訳", 6);

        assert_eq!(lines, vec!["日本語", "訳"]);
        assert_eq!(wrap_to_width("ab", 0), Vec::<String>::new());
    }
}
