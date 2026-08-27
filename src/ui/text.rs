//! Width-aware text helpers shared by the pane renderers.
//!
//! Every function here measures in *display cells* rather than bytes or chars,
//! so wide CJK characters and multi-byte glyphs never break column alignment.

use ratatui::text::Span;
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
        clip_spans, fill, pad_cell, pad_cell_centered, pad_cell_right_aligned, pad_spans,
        slice_spans, truncate_with_ellipsis, visible_width,
    };
    use ratatui::text::Span;

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
}
