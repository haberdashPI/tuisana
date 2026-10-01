//! The confirmation window: a question that owns the keyboard until answered.
//!
//! One renderer, three callers. The filter sidebar asks before it throws
//! unnamed filters away or deletes a saved set; the migration prompt asks
//! before it rewrites a version-1 config; and a bulk edit asks before it
//! changes more rows than the user can take in at a glance. What they have in
//! common is the shape of the thing: a title, a line or two saying what is
//! about to happen, and the keys that answer it.
//!
//! A modal rather than a line on a pane border, and drawn with the focused
//! border, because every key goes to it until it is answered — which is
//! exactly what a prompt hidden in a frame fails to say. It lives here rather
//! than in `filter_sets`, where it started, because the third caller made the
//! sidebar a strange place to look for it.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use crate::{
    config::Mode,
    ui::{chrome::pane_block, hints::key_column_spans, layout, text::visible_width, theme::Theme},
};

/// Widest the confirmation will grow, so its lines stay readable.
const CONFIRM_WIDTH: u16 = 56;

/// A decision that has to be answered before anything else happens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmView {
    /// The window title.
    ///
    /// Owned rather than `&'static str` because a bulk edit's title carries
    /// the count, and the count is the thing being agreed to.
    pub title: String,
    /// What is about to happen, and to what.
    pub body: Vec<String>,
    /// `(keys, what they do)`, in the order they are offered.
    pub choices: Vec<(&'static str, String)>,
}

/// Renders the confirmation centred over `area`.
pub fn render(frame: &mut Frame<'_>, area: Rect, theme: &Theme, mode: Mode, view: &ConfirmView) {
    let key_width = view
        .choices
        .iter()
        .map(|(keys, _)| visible_width(keys))
        .max()
        .unwrap_or(0);

    let mut lines = vec![Line::default()];
    for text in &view.body {
        lines.push(Line::from(Span::styled(text.clone(), theme.text)));
    }
    lines.push(Line::default());
    for (keys, label) in &view.choices {
        lines.push(Line::from(key_column_spans(
            keys,
            label,
            key_width,
            theme.key,
            theme.text,
        )));
    }
    lines.push(Line::default());

    let content = lines
        .iter()
        .map(|line| visible_width(&line.to_string()))
        .max()
        .unwrap_or(0);
    // Two for the border and two for the gutter the text sits in.
    let width = ((content as u16).saturating_add(4))
        .min(CONFIRM_WIDTH)
        .min(area.width);

    let box_area = layout::centered(area, width, lines.len() as u16 + 2);
    let block = pane_block(theme, true, mode, &view.title, &[]);
    let inner = block.inner(box_area);

    frame.render_widget(Clear, box_area);
    frame.render_widget(block, box_area);
    // One column of gutter, so the text does not butt up against the frame.
    frame.render_widget(
        Paragraph::new(lines),
        Rect {
            x: inner.x.saturating_add(1),
            width: inner.width.saturating_sub(2),
            ..inner
        },
    );
}

/// The question a bulk edit asks before it goes.
///
/// `count` leads, in the title, because it is the number the user is agreeing
/// to and the one thing a glance has to land on. `summary` is the change
/// itself — "set Due to 2026-10-15" — so the window says both what will
/// happen and how widely, which between them are the two facts a bulk edit
/// can surprise someone with.
pub fn bulk_edit_view(count: usize, summary: &str) -> ConfirmView {
    let tasks = match count {
        1 => "1 task".to_string(),
        count => format!("{count} tasks"),
    };

    ConfirmView {
        title: format!("Change {tasks}?"),
        body: vec![
            format!("{summary} on the {tasks} you have"),
            "selected.".to_string(),
        ],
        choices: vec![
            ("y", format!("change {tasks}")),
            ("n / esc", "change nothing".to_string()),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::bulk_edit_view;

    #[test]
    fn the_title_leads_with_the_count_and_the_body_names_the_change() {
        let view = bulk_edit_view(23, "set Due to 2026-10-15");

        assert_eq!(view.title, "Change 23 tasks?");
        assert_eq!(
            view.body,
            vec![
                "set Due to 2026-10-15 on the 23 tasks you have".to_string(),
                "selected.".to_string(),
            ]
        );
        assert_eq!(view.choices[0].0, "y");
        assert_eq!(view.choices[1].0, "n / esc");
    }

    /// Reachable with `edit.confirm_threshold = 0`, which is how someone who
    /// wants to be asked every time says so.
    #[test]
    fn one_task_is_not_pluralised() {
        let view = bulk_edit_view(1, "clear Start");

        assert_eq!(view.title, "Change 1 task?");
        assert!(view.body[0].starts_with("clear Start on the 1 task"));
    }
}
