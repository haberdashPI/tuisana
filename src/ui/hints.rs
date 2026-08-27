//! The contextual hint bar and the shared key-label vocabulary.
//!
//! Hints never hardcode key names. Each hint names the *actions* it describes
//! and the keys are resolved from the live keymap, so rebinding a key in config
//! updates the hint bar and the help overlay without touching this module.
//!
//! The hint bar is exactly one line. When the terminal is too narrow it sheds
//! hints from the right rather than wrapping, so the layout never reflows.

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::{
    config::Mode,
    input::{Action, KeyBinding, KeyMap},
    ui::{
        text::{spans_width, visible_width},
        theme::{GlyphSet, Theme},
    },
};

/// The gap drawn between adjacent hints.
const HINT_GAP: &str = "   ";

/// One hint: a set of actions to look keys up for, and what they do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// The actions whose keys this hint shows, in display order.
    pub keys: &'static [Action],
    /// A literal key name, for keys the input layer handles outside the keymap.
    pub literal: Option<&'static str>,
    /// What pressing the key does.
    pub label: &'static str,
}

impl Hint {
    /// A hint resolved from one or more bound actions.
    pub const fn new(keys: &'static [Action], label: &'static str) -> Self {
        Self {
            keys,
            literal: None,
            label,
        }
    }

    /// A hint for a key the keymap does not own, such as `esc` while searching.
    pub const fn literal(literal: &'static str, label: &'static str) -> Self {
        Self {
            keys: &[],
            literal: Some(literal),
            label,
        }
    }
}

/// What the current view can offer, so hints stay relevant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HintContext {
    /// A project search is active or has a query.
    pub searching: bool,
    /// Something is selected in the active pane.
    pub has_selection: bool,
    /// The task table has columns off-screen to the right.
    pub can_scroll: bool,
    /// The selected filter field holds label values.
    pub on_label_filter: bool,
    /// The date being picked is a range, so it has two ends to move between.
    pub on_date_range: bool,
}

/// Hints shown on the right of the bar in every mode.
const GLOBAL_HINTS: &[Hint] = &[
    Hint::new(&[Action::Refresh], "refresh"),
    Hint::new(&[Action::Quit], "quit"),
];

/// The hints for a mode, most important first.
pub fn hints_for(mode: Mode, context: HintContext) -> Vec<Hint> {
    match mode {
        Mode::Project | Mode::Any => project_hints(context),
        Mode::ProjectSearch => vec![
            Hint::literal("esc", "done"),
            Hint::new(&[Action::ClearSearch], "clear"),
            Hint::new(&[Action::SearchFuzzy], "fuzzy"),
            Hint::new(&[Action::SearchSubstring], "contains"),
            Hint::new(&[Action::SearchRegex], "regex"),
        ],
        Mode::Filter => vec![
            Hint::new(&[Action::BeginFilterEdit], "edit"),
            Hint::new(&[Action::MoveDown, Action::MoveUp], "field"),
            Hint::new(&[Action::CycleFilterStringMode], "match mode"),
            Hint::new(&[Action::ClearSearch], "clear"),
            Hint::new(&[Action::ToggleTaskFilters], "close"),
            Hint::new(&[Action::ToggleHelpDetails], "help"),
        ],
        Mode::FilterEdit => filter_edit_hints(context),
        Mode::Calendar => {
            let mut hints = vec![
                Hint::new(&[Action::CalendarPrevDay, Action::CalendarNextDay], "day"),
                Hint::new(
                    &[Action::CalendarPrevMonth, Action::CalendarNextMonth],
                    "month",
                ),
                Hint::new(&[Action::CalendarToday], "today"),
            ];
            // The range keys only mean anything once there are two ends, so they
            // appear only when the query has a `..` in it.
            if context.on_date_range {
                hints.push(Hint::new(
                    &[Action::CalendarJumpToStart, Action::CalendarJumpToEnd],
                    "from/to",
                ));
            }
            hints.push(Hint::new(
                &[Action::FilterCaretLeft, Action::FilterCaretRight],
                "caret",
            ));
            hints.push(Hint::new(&[Action::CalendarCommit], "pick"));
            hints.push(Hint::new(&[Action::CalendarClear], "clear"));
            hints.push(Hint::new(&[Action::CalendarClose], "close"));
            hints
        }
        Mode::Task => task_hints(context),
    }
}

fn project_hints(context: HintContext) -> Vec<Hint> {
    let mut hints = vec![
        Hint::new(&[Action::MoveDown, Action::MoveUp], "move"),
        Hint::new(&[Action::ToggleSelection], "select"),
        Hint::new(&[Action::SetTaskMode], "tasks"),
        Hint::new(&[Action::StartSearch], "search"),
    ];
    if context.has_selection {
        hints.push(Hint::new(&[Action::ToggleStarredSelected], "star"));
        hints.push(Hint::new(&[Action::ToggleHiddenSelected], "hide"));
        hints.push(Hint::new(&[Action::ClearSelection], "clear"));
    } else {
        hints.push(Hint::new(&[Action::ToggleHiddenGroup], "show hidden"));
    }
    hints.push(Hint::new(&[Action::ToggleHelpDetails], "help"));
    hints
}

fn task_hints(context: HintContext) -> Vec<Hint> {
    let mut hints = vec![
        Hint::new(&[Action::MoveDown, Action::MoveUp], "move"),
        Hint::new(&[Action::Open], "open"),
        Hint::new(&[Action::ToggleTaskSelection], "select"),
        Hint::new(&[Action::SetFilterMode], "filters"),
        Hint::new(&[Action::CycleTaskSort], "sort"),
    ];
    // Ranked above the filter toggles: when columns are off-screen, how to
    // reach them is the most urgent thing the bar can say.
    if context.can_scroll {
        hints.push(Hint::new(
            &[Action::ScrollLeft, Action::ScrollRight],
            "columns",
        ));
    }
    hints.push(Hint::new(&[Action::ToggleCompletedFilter], "completed"));
    if context.has_selection {
        hints.push(Hint::new(&[Action::CopyTasksToClipboard], "copy"));
    }
    hints.push(Hint::new(&[Action::ToggleHelpDetails], "help"));
    hints
}

fn filter_edit_hints(context: HintContext) -> Vec<Hint> {
    let mut hints = vec![
        Hint::new(&[Action::FilterDoneEditing], "done"),
        Hint::new(&[Action::FilterCancelEditing], "cancel"),
    ];
    if context.on_label_filter {
        hints.push(Hint::new(
            &[Action::FilterCycleLabelDown, Action::FilterCycleLabelUp],
            "value",
        ));
        hints.push(Hint::new(
            &[Action::FilterMoveLabelLeft, Action::FilterMoveLabelRight],
            "move",
        ));
        hints.push(Hint::new(&[Action::FilterAddLabel], "add"));
        hints.push(Hint::new(&[Action::FilterDeleteLabel], "remove"));
    } else {
        hints.push(Hint::new(
            &[Action::FilterCaretLeft, Action::FilterCaretRight],
            "caret",
        ));
        hints.push(Hint::literal("bksp", "delete"));
        hints.push(Hint::new(&[Action::ClearSearch], "clear"));
    }
    hints
}

/// Renders the hint bar: mode-specific hints on the left, globals on the right.
///
/// The globals are reserved first, because "how do I get out of here" should
/// never be the thing a narrow terminal drops.
pub fn hint_line(
    hints: &[Hint],
    keymap: &KeyMap,
    mode: Mode,
    theme: &Theme,
    width: usize,
) -> Line<'static> {
    let globals = hint_spans(GLOBAL_HINTS, keymap, mode, theme, width);
    let globals_width = spans_width(&globals);
    let left_budget = width.saturating_sub(globals_width);

    let mut spans = hint_spans(hints, keymap, mode, theme, left_budget);
    let used = spans_width(&spans);
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used + globals_width))));
    spans.extend(globals);

    Line::from(spans)
}

/// Renders as many whole hints as fit in `budget`.
///
/// Hints are dropped whole. Clipping mid-hint would leave a bare key with no
/// label, which reads as noise rather than as a truncated list.
fn hint_spans(
    hints: &[Hint],
    keymap: &KeyMap,
    mode: Mode,
    theme: &Theme,
    budget: usize,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;

    for hint in hints {
        let Some(keys) = hint_key_text(hint, keymap, mode, &theme.glyphs) else {
            continue;
        };

        let gap = if spans.is_empty() { "" } else { HINT_GAP };
        let hint_width =
            visible_width(gap) + visible_width(&keys) + 1 + visible_width(hint.label);
        if used + hint_width > budget {
            break;
        }
        used += hint_width;

        if !gap.is_empty() {
            spans.push(Span::raw(gap));
        }
        spans.push(Span::styled(keys, theme.key));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(hint.label.to_string(), theme.muted));
    }

    spans
}

/// The key text for a hint, or `None` when nothing is bound to it.
///
/// Only the first key per action is shown so the bar stays scannable; the help
/// overlay shows alternates.
pub fn hint_key_text(
    hint: &Hint,
    keymap: &KeyMap,
    mode: Mode,
    glyphs: &GlyphSet,
) -> Option<String> {
    if let Some(literal) = hint.literal {
        return Some(literal.to_string());
    }

    let keys = hint
        .keys
        .iter()
        .filter_map(|action| keymap.keys_for(action, mode).into_iter().next())
        .map(|key| key_text(&key, glyphs))
        .collect::<Vec<_>>();

    if keys.is_empty() {
        None
    } else {
        Some(keys.join("/"))
    }
}

/// All keys bound to an action in a mode, formatted for display.
pub fn all_key_texts(
    action: &Action,
    keymap: &KeyMap,
    mode: Mode,
    glyphs: &GlyphSet,
    limit: usize,
) -> Vec<String> {
    keymap
        .keys_for(action, mode)
        .iter()
        .take(limit)
        .map(|key| key_text(key, glyphs))
        .collect()
}

/// Formats one key for display.
pub fn key_text(key: &KeyBinding, glyphs: &GlyphSet) -> String {
    let unicode = glyphs == &GlyphSet::UNICODE;
    match key {
        KeyBinding::Char(' ') if unicode => "␣".to_string(),
        KeyBinding::Char(' ') => "space".to_string(),
        KeyBinding::Char(c) => c.to_string(),
        KeyBinding::Ctrl(c) => format!("^{c}"),
        KeyBinding::Enter if unicode => "⏎".to_string(),
        KeyBinding::Enter => "enter".to_string(),
        KeyBinding::Esc => "esc".to_string(),
        KeyBinding::Backspace => "bksp".to_string(),
        KeyBinding::Home => "home".to_string(),
        KeyBinding::End => "end".to_string(),
        KeyBinding::Left => "left".to_string(),
        KeyBinding::Right => "right".to_string(),
        KeyBinding::Up => "up".to_string(),
        KeyBinding::Down => "down".to_string(),
        KeyBinding::PageUp => "pgup".to_string(),
        KeyBinding::PageDown => "pgdn".to_string(),
    }
}

/// A key run and its label, padded so the labels in a column line up.
pub fn key_column_spans(
    keys: &str,
    label: &str,
    key_width: usize,
    key_style: Style,
    label_style: Style,
) -> Vec<Span<'static>> {
    let padding = key_width.saturating_sub(visible_width(keys));
    vec![
        Span::styled(keys.to_string(), key_style),
        Span::raw(" ".repeat(padding + 2)),
        Span::styled(label.to_string(), label_style),
    ]
}

#[cfg(test)]
mod tests {
    use super::{hint_line, hint_key_text, hints_for, Hint, HintContext};
    use crate::{
        config::{Config, Mode},
        input::{Action, KeyMap},
        ui::{text::visible_width, theme::Theme},
    };

    fn keymap() -> KeyMap {
        KeyMap::from_bindings(&Config::default().effective_bindings()).expect("keymap builds")
    }

    #[test]
    fn resolves_hint_keys_from_the_keymap() {
        let keymap = keymap();
        let theme = Theme::default();

        let hint = Hint::new(&[Action::MoveDown, Action::MoveUp], "move");

        assert_eq!(
            hint_key_text(&hint, &keymap, Mode::Task, &theme.glyphs),
            Some("j/k".to_string())
        );
    }

    #[test]
    fn a_mode_specific_binding_shadows_the_any_binding_in_hints() {
        let keymap = keymap();
        let theme = Theme::default();

        // `j` is move_down globally but cycles label values while editing a filter.
        assert_eq!(
            hint_key_text(
                &Hint::new(&[Action::MoveDown], "move"),
                &keymap,
                Mode::FilterEdit,
                &theme.glyphs
            ),
            None
        );
        assert_eq!(
            hint_key_text(
                &Hint::new(&[Action::FilterCycleLabelDown], "value"),
                &keymap,
                Mode::FilterEdit,
                &theme.glyphs
            ),
            Some("j".to_string())
        );
    }

    #[test]
    fn omits_hints_whose_actions_are_unbound() {
        let keymap = KeyMap::from_bindings(&[]).expect("empty keymap builds");
        let theme = Theme::default();

        assert_eq!(
            hint_key_text(
                &Hint::new(&[Action::MoveDown], "move"),
                &keymap,
                Mode::Task,
                &theme.glyphs
            ),
            None
        );
        assert_eq!(
            hint_key_text(&Hint::literal("esc", "done"), &keymap, Mode::Task, &theme.glyphs),
            Some("esc".to_string())
        );
    }

    #[test]
    fn hint_bar_is_one_line_and_never_exceeds_the_width() {
        let keymap = keymap();
        let theme = Theme::default();

        for width in [20usize, 40, 80, 120, 200] {
            for mode in [
                Mode::Project,
                Mode::ProjectSearch,
                Mode::Filter,
                Mode::FilterEdit,
                Mode::Task,
            ] {
                let hints = hints_for(mode, HintContext::default());
                let line = hint_line(&hints, &keymap, mode, &theme, width);
                assert!(
                    visible_width(&line.to_string()) <= width,
                    "mode {mode:?} overflowed width {width}"
                );
            }
        }
    }

    #[test]
    fn a_narrow_bar_drops_whole_hints_rather_than_clipping_one() {
        let keymap = keymap();
        let theme = Theme::default();
        let hints = hints_for(Mode::Task, HintContext::default());

        // Every hint that fits is rendered in full, so the visible text is
        // always a whole-hint prefix of the full list.
        let full = hints
            .iter()
            .filter_map(|hint| {
                hint_key_text(hint, &keymap, Mode::Task, &theme.glyphs)
                    .map(|keys| format!("{keys} {}", hint.label))
            })
            .collect::<Vec<_>>();

        for width in [20usize, 30, 45, 60, 80, 120] {
            let rendered = hint_line(&hints, &keymap, Mode::Task, &theme, width).to_string();
            let left = rendered
                .split("r refresh")
                .next()
                .expect("a left segment")
                .trim_end();

            let prefixes = (0..=full.len())
                .map(|count| full[..count].join("   "))
                .collect::<Vec<_>>();
            assert!(
                prefixes.iter().any(|prefix| prefix == left),
                "width {width} rendered a partial hint: {left:?}"
            );
        }
    }

    #[test]
    fn keeps_refresh_and_quit_visible_on_the_right() {
        let keymap = keymap();
        let theme = Theme::default();

        let hints = hints_for(Mode::Task, HintContext::default());
        let rendered = hint_line(&hints, &keymap, Mode::Task, &theme, 100).to_string();

        assert!(rendered.trim_end().ends_with("q quit"));
        assert!(rendered.contains("r refresh"));
    }

    #[test]
    fn shows_selection_hints_only_when_something_is_selected() {
        let keymap = keymap();
        let theme = Theme::default();
        let context = HintContext {
            has_selection: true,
            ..HintContext::default()
        };

        let idle = hint_line(
            &hints_for(Mode::Task, HintContext::default()),
            &keymap,
            Mode::Task,
            &theme,
            120,
        )
        .to_string();
        let selected =
            hint_line(&hints_for(Mode::Task, context), &keymap, Mode::Task, &theme, 120).to_string();

        assert!(!idle.contains("copy"));
        assert!(selected.contains("copy"));
    }
}
