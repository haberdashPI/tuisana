//! The candidate list an open completion editor offers.
//!
//! Drawn as a centered modal over the panes, positioned and styled like the
//! date picker and for the same two reasons: the cell is far too narrow to
//! list names in, and the picker already taught the reader where to look for
//! the values a field will accept.
//!
//! The text being edited is deliberately *not* here — it stays in the cell or
//! the filter row, where the caret is drawn. This overlay shows only what
//! `tab` will reach, so there is one place to look for the value and one for
//! the choices.

use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use crate::{
    app::task::TaskState,
    config::Mode,
    ui::{
        chrome::{pane_block, Chip, Tone},
        layout,
        text::{pad_cell, visible_width},
        theme::Theme,
    },
};

/// Most candidates the overlay lists at once.
///
/// Past this the list is not being read, it is being scrolled past — and the
/// way to narrow it is to type another character, which the count says.
const MAX_ROWS: usize = 10;
/// Narrowest and widest the box will be, names aside.
const MIN_WIDTH: usize = 22;
const MAX_WIDTH: usize = 44;

/// Snapshot of the candidate list used by the renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionView {
    /// The field being edited, e.g. `Assignee`.
    pub title: String,
    /// The candidates on offer, in the order `tab` walks them.
    pub rows: Vec<String>,
    /// Which row `tab` has landed on, if it has been pressed.
    pub highlighted: Option<usize>,
    /// How many matches did not fit.
    pub more: usize,
    /// Whether anything matches what has been typed.
    pub matched: bool,
}

/// Builds the view from whichever completion editor is open.
pub(crate) fn completion_view(state: &TaskState) -> Option<CompletionView> {
    let (title, complete) = state.open_completion()?;
    let matches = complete.matches();
    let highlighted = complete.highlighted();

    Some(CompletionView {
        title,
        rows: matches
            .iter()
            .take(MAX_ROWS)
            .map(|candidate| candidate.display.clone())
            .collect(),
        // A highlight past the visible rows would point at nothing, so the
        // count below is what says the walk is still going.
        highlighted: highlighted.filter(|index| *index < MAX_ROWS),
        more: matches.len().saturating_sub(MAX_ROWS),
        matched: !matches.is_empty(),
    })
}

/// Draws the candidate list centered in `area`.
pub fn render(frame: &mut Frame<'_>, area: Rect, theme: &Theme, view: &CompletionView) {
    let lines = completion_lines(view, theme);
    let width = view
        .rows
        .iter()
        .map(|row| visible_width(row))
        .max()
        .unwrap_or(0)
        .clamp(MIN_WIDTH, MAX_WIDTH);

    let box_area = layout::centered(
        area,
        (width as u16).saturating_add(2),
        (lines.len() as u16).saturating_add(2),
    );

    let chips = match view.matched {
        true => Vec::new(),
        // Otherwise an empty box reads as a broken picker rather than as an
        // answer: there is nothing here by that name.
        false => vec![Chip::toned("no match", Tone::Danger)],
    };
    let block = pane_block(theme, true, Mode::TaskEdit, &view.title, &chips);
    let inner = block.inner(box_area);

    frame.render_widget(Clear, box_area);
    frame.render_widget(block, box_area);
    frame.render_widget(Paragraph::new(lines), inner);
}

fn completion_lines(view: &CompletionView, theme: &Theme) -> Vec<Line<'static>> {
    let width = view
        .rows
        .iter()
        .map(|row| visible_width(row))
        .max()
        .unwrap_or(0)
        .clamp(MIN_WIDTH, MAX_WIDTH);

    let mut lines = view
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let style = match view.highlighted == Some(index) {
                // Reversed rather than merely coloured: this is where `tab`
                // has put the text, so it is a cursor and not a hint.
                true => theme.accent.add_modifier(Modifier::REVERSED),
                false => theme.text,
            };
            Line::from(Span::styled(
                pad_cell(row, width, theme.glyphs.ellipsis),
                style,
            ))
        })
        .collect::<Vec<_>>();

    if view.rows.is_empty() {
        lines.push(Line::from(Span::styled(
            pad_cell("nothing matches", width, theme.glyphs.ellipsis),
            theme.muted,
        )));
    }
    if view.more > 0 {
        lines.push(Line::from(Span::styled(
            pad_cell(&format!("+{} more", view.more), width, theme.glyphs.ellipsis),
            theme.muted,
        )));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::{completion_lines, CompletionView, MAX_ROWS};
    use crate::ui::theme::Theme;

    fn view(rows: usize) -> CompletionView {
        CompletionView {
            title: "Assignee".to_string(),
            rows: (0..rows.min(MAX_ROWS))
                .map(|index| format!("Person {index}"))
                .collect(),
            highlighted: None,
            more: rows.saturating_sub(MAX_ROWS),
            matched: rows > 0,
        }
    }

    #[test]
    fn a_long_directory_is_cut_off_with_the_rest_counted() {
        let lines = completion_lines(&view(25), &Theme::default());

        assert_eq!(lines.len(), MAX_ROWS + 1);
        assert!(
            lines.last().expect("a last line").to_string().contains("+15 more"),
            "the way to narrow the list is to type, which the count says"
        );
    }

    #[test]
    fn an_empty_list_says_so_rather_than_drawing_an_empty_box() {
        let lines = completion_lines(&view(0), &Theme::default());

        assert_eq!(lines.len(), 1);
        assert!(lines[0].to_string().contains("nothing matches"));
    }
}
