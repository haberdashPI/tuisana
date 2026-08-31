//! The colour order dialog.
//!
//! A centered modal listing the current dimension's values in the order they
//! are given colours, with a rule where the palette runs out. Reordering
//! repaints the chart behind it immediately, so the dialog is a preview as
//! much as an editor.
//!
//! Drawn the same way the calendar overlay is: [`layout::centered`], `Clear`,
//! a [`pane_block`], and a `Paragraph`. The layout underneath never moves.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use crate::{
    app::gantt::GanttOrderState,
    config::Mode,
    domain::PALETTE_SLOTS,
    input::{Action, KeyMap},
    ui::{
        chrome::{pane_block, Chip},
        hints::all_key_texts,
        layout,
        text::{pad_cell, visible_width},
        theme::Theme,
    },
};

/// Widest the dialog will grow.
const MAX_WIDTH: u16 = 62;
/// Cells reserved for the right-aligned task count.
const COUNT_WIDTH: usize = 9;
/// Cells before the value: cursor, gap, rank, gap, glyph, gap.
const LEAD_WIDTH: usize = 9;

/// Draws the dialog over `area`.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    keymap: &KeyMap,
    state: &GanttOrderState,
) {
    let width = MAX_WIDTH.min(area.width);
    let inner_width = width.saturating_sub(2) as usize;
    let lines = dialog_lines(state, theme, keymap, inner_width);

    let box_area = layout::centered(area, width, lines.len() as u16 + 2);
    let title = format!("Colours ─ by {}", state.key().label());
    let count = state.entries().len();
    let chips = vec![Chip::new(format!(
        "{count} value{}",
        if count == 1 { "" } else { "s" }
    ))];

    let block = pane_block(theme, true, Mode::GanttOrder, &title, &chips);
    let inner = block.inner(box_area);

    frame.render_widget(Clear, box_area);
    frame.render_widget(block, box_area);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The dialog's content: a blank line, the values, the rule, and the keys.
pub fn dialog_lines(
    state: &GanttOrderState,
    theme: &Theme,
    keymap: &KeyMap,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::default()];

    // The rule goes after the last value that gets a colour, so it moves as
    // values are reordered. It is the whole point of the dialog: everything
    // below it is drawn neutral.
    let rule_after = state.movable_count().min(PALETTE_SLOTS);
    let needs_rule = state.entries().len() > rule_after;

    for (index, entry) in state.entries().iter().enumerate() {
        if needs_rule && index == rule_after {
            lines.push(rule_line(theme, width));
        }
        lines.push(entry_line(state, index, entry, theme, width));
    }

    lines.push(Line::default());
    for keys in key_lines(keymap, theme, width) {
        lines.push(keys);
    }
    lines
}

fn entry_line(
    state: &GanttOrderState,
    index: usize,
    entry: &crate::app::gantt::OrderEntry,
    theme: &Theme,
    width: usize,
) -> Line<'static> {
    let slot = state.slot(index);
    let selected = index == state.selected();

    // The empty value is unnumbered: it has no place in the order, and a
    // number would suggest it could be moved into one.
    let rank = match entry.movable {
        true => format!("{:>2}", index + 1),
        false => "  ".to_string(),
    };

    let value_width = width.saturating_sub(LEAD_WIDTH + COUNT_WIDTH);
    let label = match entry.movable {
        true => entry.value.clone(),
        false => format!("(no {})", state.key().label()),
    };
    let count = format!(
        "{} task{}",
        entry.task_count,
        if entry.task_count == 1 { "" } else { "s" }
    );

    let line = Line::from(vec![
        Span::styled(
            if selected { theme.glyphs.cursor } else { " " }.to_string(),
            theme.marker,
        ),
        Span::raw(" "),
        Span::styled(rank, theme.muted),
        Span::raw("  "),
        Span::styled(theme.bar_glyph(slot).to_string(), theme.categorical_style(slot)),
        Span::raw("  "),
        Span::styled(
            pad_cell(&label, value_width, theme.glyphs.ellipsis),
            if entry.movable { theme.text } else { theme.muted },
        ),
        Span::styled(format!("{count:>COUNT_WIDTH$}"), theme.muted),
    ]);

    match selected {
        true => line.style(theme.cursor),
        false => line,
    }
}

fn rule_line(theme: &Theme, width: usize) -> Line<'static> {
    let label = " neutral below ";
    let dashes = width.saturating_sub(visible_width(label) + 2);
    let left = dashes / 2;

    Line::from(vec![
        Span::raw(" "),
        Span::styled(theme.glyphs.rule.repeat(left), theme.border),
        Span::styled(label.to_string(), theme.muted),
        Span::styled(theme.glyphs.rule.repeat(dashes - left), theme.border),
        Span::raw(" "),
    ])
}

/// The two key lines, resolved from the keymap so a rebind shows through.
fn key_lines(keymap: &KeyMap, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let keys = |action: &Action| {
        all_key_texts(action, keymap, Mode::GanttOrder, &theme.glyphs, 1)
            .first()
            .cloned()
            .unwrap_or_else(|| "-".to_string())
    };

    let pairs = [
        vec![
            (
                format!(
                    "{} / {}",
                    keys(&Action::GanttOrderMoveUp),
                    keys(&Action::GanttOrderMoveDown)
                ),
                "move",
            ),
            (
                format!(
                    "{} / {}",
                    keys(&Action::GanttOrderMoveTop),
                    keys(&Action::GanttOrderMoveBottom)
                ),
                "to top / bottom",
            ),
        ],
        vec![
            (keys(&Action::CycleGanttColorKey), "colour by"),
            (keys(&Action::GanttOrderCommit), "save"),
            (keys(&Action::GanttOrderCancel), "cancel"),
        ],
    ];

    pairs
        .into_iter()
        .map(|row| {
            let mut spans = vec![Span::raw("  ")];
            for (index, (key, label)) in row.into_iter().enumerate() {
                if index > 0 {
                    spans.push(Span::raw("    "));
                }
                spans.push(Span::styled(key, theme.key));
                spans.push(Span::raw("  "));
                spans.push(Span::styled(label.to_string(), theme.muted));
            }
            let used: usize = spans.iter().map(|span| visible_width(&span.content)).sum();
            spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::dialog_lines;
    use crate::app::gantt::{GanttViewState, MoveTo};
    use crate::config::Config;
    use crate::input::KeyMap;
    use crate::ui::theme::Theme;

    const WIDTH: usize = 58;

    fn keymap() -> KeyMap {
        KeyMap::from_bindings(&Config::default().effective_bindings()).expect("bindings parse")
    }

    fn opened(names: &[&str], empty: Option<usize>) -> GanttViewState {
        let mut values = names
            .iter()
            .map(|name| (name.to_string(), 1usize))
            .collect::<Vec<_>>();
        if let Some(count) = empty {
            values.push((String::new(), count));
        }
        let mut state = GanttViewState::default();
        state.open_dialog(values);
        state
    }

    fn rendered(state: &GanttViewState) -> Vec<String> {
        dialog_lines(
            state.dialog().expect("open"),
            &Theme::default(),
            &keymap(),
            WIDTH,
        )
        .iter()
        .map(ToString::to_string)
        .collect()
    }

    fn rule_position(lines: &[String]) -> Option<usize> {
        lines.iter().position(|line| line.contains("neutral below"))
    }

    #[test]
    fn the_rule_lands_after_the_sixth_value() {
        let lines = rendered(&opened(&["a", "b", "c", "d", "e", "f", "g", "h"], None));
        let rule = rule_position(&lines).expect("a rule");

        assert!(lines[rule - 1].contains('f'));
        assert!(lines[rule + 1].contains('g'));
    }

    #[test]
    fn there_is_no_rule_when_every_value_gets_a_colour() {
        let lines = rendered(&opened(&["a", "b"], None));

        assert_eq!(rule_position(&lines), None);
    }

    #[test]
    fn the_rule_moves_when_a_value_is_reordered() {
        // The rule is the point of the dialog: it has to track the reorder,
        // not sit at a fixed row.
        let mut state = opened(&["a", "b", "c", "d", "e", "f", "g"], None);
        let before = rendered(&state);
        let rule = rule_position(&before).expect("a rule");
        assert!(before[rule + 1].contains('g'));

        state.dialog_move_cursor(6);
        state.dialog_move_value(MoveTo::Top);
        let after = rendered(&state);
        let rule = rule_position(&after).expect("a rule");

        assert!(after[rule + 1].contains('f'), "{:?}", after[rule + 1]);
    }

    #[test]
    fn the_empty_value_renders_last_named_and_unnumbered() {
        let lines = rendered(&opened(&["a", "b"], Some(4)));
        let last = lines
            .iter()
            .rfind(|line| line.contains("no assignee"))
            .expect("the empty row");

        assert!(last.contains("(no assignee)"));
        assert!(last.contains("4 tasks"));
        assert!(!last.contains('3'), "it has no rank: {last}");
    }

    #[test]
    fn a_single_task_is_counted_in_the_singular() {
        let lines = rendered(&opened(&["a"], None));

        assert!(lines.iter().any(|line| line.contains("1 task")));
        assert!(!lines.iter().any(|line| line.contains("1 tasks")));
    }

    #[test]
    fn the_key_lines_resolve_from_the_keymap() {
        let lines = rendered(&opened(&["a"], None));
        let keys = lines.join("\n");

        assert!(keys.contains("^k / ^j"), "{keys}");
        assert!(keys.contains("t / b"), "{keys}");
        assert!(keys.contains("cancel"), "{keys}");
    }

    #[test]
    fn every_content_line_is_exactly_the_dialog_wide() {
        // The cursor row is styled as a whole Line, so a short one would
        // highlight only part of the row. The two spacers are deliberately
        // empty: the modal is drawn over Clear, so there is nothing to cover.
        use crate::ui::text::visible_width;

        for line in rendered(&opened(&["a", "b", "c", "d", "e", "f", "g"], Some(2))) {
            if line.is_empty() {
                continue;
            }
            assert_eq!(visible_width(&line), WIDTH, "{line:?}");
        }
    }
}
