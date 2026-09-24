//! The floating notice pane: what the last edit refused, or failed, to do.
//!
//! This used to be one chip on the task pane's top border, beside the counts.
//! A border chip is the right size for `12 tasks` and the wrong size for a
//! backend error: the interesting half — what the server actually said — was
//! the half that fell off the end, and the chip competed for the same row as
//! the state that is always true.
//!
//! So it is a box in the bottom-right corner instead. The corner rather than
//! the centre because a notice reports on something the user already did, and
//! must not land on the row they are reading. Its own box because the message
//! can be several lines: the line breaks a sender put in an error are part of
//! what the error says, and long text wraps instead of being cut.
//!
//! The pane captures no keys. `esc` dismisses it wherever it is showing, and
//! goes on to do whatever else it did in that mode.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use crate::{
    app::task::TaskState,
    config::Mode,
    ui::{
        chrome::pane_block,
        layout,
        text::{pad_cell, pad_cell_right_aligned, visible_width, wrap_to_width},
        theme::Theme,
    },
};

/// Narrowest and widest the box will be, border and gutter aside.
const MIN_WIDTH: usize = 24;
const MAX_WIDTH: usize = 56;
/// Most rows of message the box will show.
///
/// Past this it is no longer an aside, it is a pane — and a notice that
/// covered the table would be worse than the chip it replaced.
const MAX_ROWS: usize = 10;
/// What the footer says, and the key it names.
const DISMISS_HINT: &str = "esc to dismiss";

/// Snapshot of the notice used by the renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeView {
    /// The message, newlines and all.
    pub text: String,
}

/// Builds the view when there is a notice to show.
pub(crate) fn notice_view(state: &TaskState) -> Option<NoticeView> {
    state.edit_notice().map(|text| NoticeView {
        text: text.to_string(),
    })
}

/// Draws the notice in the bottom-right corner of `area`.
pub fn render(frame: &mut Frame<'_>, area: Rect, theme: &Theme, mode: Mode, view: &NoticeView) {
    if area.width < 4 || area.height < 3 {
        return;
    }

    let width = notice_width(&view.text, area.width);
    let lines = notice_lines(&view.text, width, theme);

    // Two columns for the border and two for the gutter the text sits in.
    let box_area = layout::bottom_right(
        area,
        (width as u16).saturating_add(4),
        (lines.len() as u16).saturating_add(2),
    );
    let block = pane_block(theme, true, mode, "Notice", &[]);
    let inner = block.inner(box_area);

    frame.render_widget(Clear, box_area);
    frame.render_widget(block, box_area);
    frame.render_widget(
        Paragraph::new(lines),
        Rect {
            x: inner.x.saturating_add(1),
            width: inner.width.saturating_sub(2),
            ..inner
        },
    );
}

/// How wide the text column should be: as wide as the message wants, within
/// the bounds and within the room the region has.
fn notice_width(text: &str, available: u16) -> usize {
    let room = (available as usize).saturating_sub(4);
    let longest = text.lines().map(visible_width).max().unwrap_or(0);
    longest
        .max(visible_width(DISMISS_HINT))
        .clamp(MIN_WIDTH, MAX_WIDTH)
        .min(room)
}

fn notice_lines(text: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let wrapped = wrap_to_width(text, width);
    let shown = wrapped.len().min(MAX_ROWS);

    let mut lines = wrapped
        .iter()
        .take(shown)
        .map(|row| {
            Line::from(Span::styled(
                pad_cell(row, width, theme.glyphs.ellipsis),
                theme.danger,
            ))
        })
        .collect::<Vec<_>>();

    // A cut message has to say it was cut, or the last line it does show
    // reads as the whole of what went wrong.
    if wrapped.len() > shown {
        lines.push(Line::from(Span::styled(
            pad_cell(
                &format!("+{} more lines", wrapped.len() - shown),
                width,
                theme.glyphs.ellipsis,
            ),
            theme.muted,
        )));
    }

    lines.push(Line::from(Span::styled(
        pad_cell_right_aligned(DISMISS_HINT, width, theme.glyphs.ellipsis),
        theme.muted,
    )));

    lines
}

#[cfg(test)]
mod tests {
    use super::{
        notice_lines, notice_width, render, NoticeView, DISMISS_HINT, MAX_ROWS, MAX_WIDTH,
        MIN_WIDTH,
    };
    use crate::ui::{text::visible_width, theme::Theme};

    fn rendered(text: &str, available: u16) -> Vec<String> {
        let width = notice_width(text, available);
        notice_lines(text, width, &Theme::default())
            .iter()
            .map(|line| line.to_string().trim_end().to_string())
            .collect()
    }

    #[test]
    fn the_newlines_in_a_message_are_the_lines_on_screen() {
        let lines = rendered(
            "could not update 2 of 2:\nbackend error: 403\nbackend error: 404",
            80,
        );

        assert_eq!(
            lines,
            vec![
                "could not update 2 of 2:".to_string(),
                "backend error: 403".to_string(),
                "backend error: 404".to_string(),
                // Right-aligned, so the hint carries its leading padding.
                format!("{DISMISS_HINT:>24}"),
            ]
        );
    }

    #[test]
    fn a_long_line_wraps_instead_of_being_cut() {
        let text = "could not update 1 of 1: backend error: HTTP status client error \
                    (403 Forbidden) for url (https://app.asana.com/api/1.0/tasks/1234)";
        let lines = rendered(text, 200);

        assert!(lines.len() > 2, "it wrapped: {lines:?}");
        assert!(
            lines.iter().all(|line| visible_width(line) <= MAX_WIDTH),
            "nothing overflows the box: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("1234")),
            "the tail of the url survives: {lines:?}"
        );
    }

    #[test]
    fn a_message_too_tall_to_show_says_how_much_is_missing() {
        let text = (0..MAX_ROWS + 4)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = rendered(&text, 80);

        assert_eq!(lines.len(), MAX_ROWS + 2, "the rows, the count, the hint");
        assert!(lines[MAX_ROWS].contains("+4 more lines"));
    }

    /// The corner geometry is the new part, so it is drawn rather than
    /// computed: a box anchored a cell off the edge reads as a bug, and a box
    /// wider than a narrow terminal panics.
    #[test]
    fn the_box_is_drawn_in_the_corner_and_survives_a_narrow_terminal() {
        use ratatui::{backend::TestBackend, layout::Rect, Terminal};

        let draw = |width: u16, height: u16| -> Vec<String> {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
            terminal
                .draw(|frame| {
                    let view = NoticeView {
                        text: "backend error: 403".to_string(),
                    };
                    render(
                        frame,
                        Rect::new(0, 0, width, height),
                        &Theme::default(),
                        crate::config::Mode::Task,
                        &view,
                    );
                })
                .expect("it draws");

            let buffer = terminal.backend().buffer().clone();
            buffer
                .content
                .chunks(buffer.area.width as usize)
                .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect()
        };

        let screen = draw(60, 10);
        assert!(
            screen[0].trim().is_empty(),
            "the top of the region is left alone: {:?}",
            screen[0]
        );
        let last = screen.last().expect("a last row");
        assert!(
            last.ends_with('┛') && last.starts_with(' '),
            "the box is anchored to the bottom-right: {last:?}"
        );
        assert!(screen.iter().any(|row| row.contains("backend error: 403")));

        // Narrow enough that the box is all border, and narrower still than
        // the box can be drawn at all.
        draw(10, 6);
        draw(3, 2);
    }

    #[test]
    fn the_box_never_outgrows_the_room_it_has() {
        // Wider than the bounds, and narrower: the width is clamped both ways
        // before the region gets a say, and by the region after it.
        assert_eq!(notice_width("short", 200), MIN_WIDTH);
        assert_eq!(notice_width(&"x".repeat(200), 200), MAX_WIDTH);
        assert_eq!(notice_width(&"x".repeat(200), 30), 26);

        let lines = rendered(&"x".repeat(200), 30);
        assert!(lines.iter().all(|line| visible_width(line) <= 26), "{lines:?}");
    }
}
