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
    /// The chart's timeline has been scrolled or zoomed off its fitted window.
    pub timeline_windowed: bool,
    /// The filter panel holds more than one set, so there are tabs to move
    /// between and one that can be removed.
    pub many_filter_sets: bool,
    /// The `Sets` sidebar is open, so its keys can do something.
    pub filter_sets_sidebar: bool,
    /// How many named filter sets are saved, which is what decides whether
    /// there is a second page to move to.
    pub saved_filter_sets: usize,
    /// The panel is bound to a named entry, so there is something to copy
    /// away from and something to delete.
    pub filter_set_loaded: bool,
    /// The open sidebar prompt wants a `y`/`n` rather than typed text.
    pub filter_set_confirm: bool,
    /// A task table cell is open for editing.
    pub on_task_cell: bool,
    /// The open cell editor is a value picker, which reads `j`/`k`.
    pub task_edit_is_options: bool,
    /// The open editor completes over a list of names, so `tab` does
    /// something.
    pub task_edit_completes: bool,
    /// The recently-edited pane has rows, so the toggle can do something.
    pub has_recent_edits: bool,
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
        Mode::Filter => {
            let mut hints = vec![
                Hint::new(&[Action::BeginFilterEdit], "edit"),
                Hint::new(&[Action::MoveDown, Action::MoveUp], "field"),
                Hint::new(&[Action::FilterRequireEmpty], "require empty"),
                Hint::new(&[Action::FilterNegateField], "negate"),
                Hint::new(&[Action::CycleFilterStringMode], "match mode"),
                Hint::new(&[Action::ClearSearch], "clear"),
                // `add set` is how the feature is discovered, so it shows even
                // with one set; the rest can do nothing until there are two.
                Hint::new(&[Action::FilterSetAdd], "add set"),
            ];
            if context.many_filter_sets {
                hints.push(Hint::new(
                    &[Action::FilterSetPrev, Action::FilterSetNext],
                    "set",
                ));
                hints.push(Hint::new(&[Action::FilterSetRemove], "remove set"));
                // Negating the only set is legal but pointless — it turns the
                // panel into "show nothing" — so the key is offered once there
                // is a second set for it to be the complement of.
                hints.push(Hint::new(&[Action::FilterNegateSet], "negate set"));
            }
            // Always, so the feature is discoverable; the rest only once the
            // sidebar is open and they have somewhere to act.
            hints.push(Hint::new(&[Action::FilterSetsToggle], "sets"));
            if context.filter_sets_sidebar {
                hints.push(Hint::literal("1-9", "load"));
                hints.push(Hint::new(&[Action::FilterSetSave], "save"));
                hints.push(Hint::new(&[Action::FilterSetNew], "new"));
                if context.filter_set_loaded {
                    hints.push(Hint::new(&[Action::FilterSetCopyToNew], "copy to new"));
                    hints.push(Hint::new(&[Action::FilterSetDelete], "delete"));
                }
                if context.saved_filter_sets > crate::app::task::MAX_SIDEBAR_ROWS {
                    hints.push(Hint::new(
                        &[Action::FilterSetsPageBack, Action::FilterSetsPageForward],
                        "page",
                    ));
                }
            }
            hints.push(Hint::new(&[Action::ToggleTaskFilters], "close"));
            hints.push(Hint::new(&[Action::ToggleHelpDetails], "help"));
            hints
        }
        Mode::FilterEdit => filter_edit_hints(context),
        // Read outside the keymap, so every key here is a literal. One mode
        // covers two kinds of prompt, and they do not read the same keys.
        Mode::FilterSetName if context.filter_set_confirm => vec![
            Hint::literal("y", "confirm"),
            Hint::literal("n", "cancel"),
            Hint::literal("esc", "cancel"),
        ],
        Mode::FilterSetName => vec![
            Hint::literal("enter", "save"),
            Hint::literal("esc", "cancel"),
        ],
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
            // appear only when the query has a `..` in it — and a task date
            // is one day, so it never has them.
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
            // The same keys, one layer over: on a task cell `enter` sends a
            // write and `esc` throws the edit away, so they are not "pick"
            // and "close".
            let (commit, close) = match context.on_task_cell {
                true => ("save", "cancel"),
                false => ("pick", "close"),
            };
            hints.push(Hint::new(&[Action::CalendarCommit], commit));
            hints.push(Hint::new(&[Action::CalendarClear], "clear"));
            hints.push(Hint::new(&[Action::CalendarClose], close));
            hints
        }
        Mode::Task => task_hints(context),
        Mode::TaskEdit => task_edit_hints(context),
        Mode::Gantt => gantt_hints(context),
        Mode::GanttOrder => vec![
            Hint::new(&[Action::MoveDown, Action::MoveUp], "cursor"),
            Hint::new(
                &[Action::GanttOrderMoveUp, Action::GanttOrderMoveDown],
                "move",
            ),
            Hint::new(
                &[Action::GanttOrderMoveTop, Action::GanttOrderMoveBottom],
                "top/bottom",
            ),
            Hint::new(&[Action::CycleGanttColorKey], "color by"),
            Hint::new(&[Action::GanttOrderCommit], "save"),
            Hint::new(&[Action::GanttOrderCancel], "cancel"),
        ],
    }
}

fn gantt_hints(context: HintContext) -> Vec<Hint> {
    let mut hints = vec![
        Hint::new(&[Action::GanttScrollLeft, Action::GanttScrollRight], "scroll"),
        Hint::new(&[Action::GanttZoomOut, Action::GanttZoomIn], "zoom"),
    ];
    // Refitting a window that is already fitted does nothing, so it is only
    // worth a slot once scrolling or zooming has moved it.
    if context.timeline_windowed {
        hints.push(Hint::new(&[Action::GanttZoomFit], "fit"));
    }
    hints.push(Hint::new(&[Action::GanttToday], "today"));
    hints.push(Hint::new(
        &[Action::GanttRemoveColumn, Action::GanttAddColumn],
        "columns",
    ));
    hints.push(Hint::new(&[Action::CycleGanttColorKey], "color"));
    hints.push(Hint::new(&[Action::GanttOpenOrder], "order"));
    hints.push(Hint::new(&[Action::ToggleGantt], "close"));
    hints.push(Hint::new(&[Action::ToggleHelpDetails], "help"));
    hints
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
        Hint::new(&[Action::ToggleTaskSelection], "select"),
    ];
    // Next to `select`, because what a selection is *for* is the thing worth
    // saying once there is one.
    if context.has_selection {
        hints.push(Hint::new(&[Action::CopyTasksToClipboard], "copy"));
    }
    hints.push(Hint::new(&[Action::BeginTaskEdit], "edit"));
    hints.push(Hint::new(&[Action::ToggleTaskCompleted], "done"));
    hints.push(Hint::new(
        &[Action::TaskColumnPrev, Action::TaskColumnNext],
        "column",
    ));
    hints.push(Hint::new(&[Action::Open], "open"));
    hints.push(Hint::new(&[Action::SetFilterMode], "filters"));
    hints.push(Hint::new(&[Action::CycleTaskSort], "sort"));
    // Ranked above the view toggles: when columns are off-screen, how to
    // reach them is the most urgent thing the bar can say.
    if context.can_scroll {
        hints.push(Hint::new(
            &[Action::ScrollLeft, Action::ScrollRight],
            "scroll",
        ));
    }
    hints.push(Hint::new(&[Action::SetGanttMode], "gantt"));
    // Only once there is a pane to toggle: with nothing edited out of view
    // the key is a no-op, and a hint for a no-op is a hint in the way.
    if context.has_recent_edits {
        hints.push(Hint::new(&[Action::ToggleRecentPane], "recent"));
    }
    hints.push(Hint::new(&[Action::ToggleCompletedFilter], "completed"));
    hints.push(Hint::new(&[Action::ToggleHelpDetails], "help"));
    hints
}

/// The keys an open cell editor reads.
///
/// A value picker reads no text at all, so the caret motions are replaced by
/// the two keys that actually do something there.
fn task_edit_hints(context: HintContext) -> Vec<Hint> {
    let mut hints = vec![
        Hint::new(&[Action::CommitTaskEdit], "save"),
        Hint::new(&[Action::CancelTaskEdit], "cancel"),
    ];
    if context.task_edit_is_options {
        hints.push(Hint::new(
            &[
                Action::TaskEditCycleValue(1),
                Action::TaskEditCycleValue(-1),
            ],
            "value",
        ));
        hints.push(Hint::new(&[Action::TaskEditClear], "clear"));
        return hints;
    }
    // Ahead of the caret motions: on a field whose values are a closed set,
    // completing is the whole interaction and typing is only how you narrow
    // it.
    if context.task_edit_completes {
        hints.push(Hint::new(
            &[
                Action::CompleteCandidate(1),
                Action::CompleteCandidate(-1),
            ],
            "complete",
        ));
        // A literal, unlike every other hint here: `d` is bound to the same
        // action but types its letter into a field that reads text, so
        // naming both keys would advertise one that does something else.
        hints.push(Hint::literal("^l", "clear"));
    }
    hints.push(Hint::new(
        &[Action::TextCaretWordBack, Action::TextCaretWordForward],
        "word",
    ));
    hints.push(Hint::new(
        &[Action::TextCaretStart, Action::TextCaretEnd],
        "ends",
    ));
    hints.push(Hint::literal("bksp", "delete"));
    hints
}

fn filter_edit_hints(context: HintContext) -> Vec<Hint> {
    let mut hints = vec![
        Hint::new(&[Action::FilterDoneEditing], "done"),
        Hint::new(&[Action::FilterCancelEditing], "cancel"),
    ];
    if context.task_edit_completes {
        hints.push(Hint::new(
            &[
                Action::CompleteCandidate(1),
                Action::CompleteCandidate(-1),
            ],
            "complete",
        ));
    }
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
        hints.push(Hint::new(
            &[Action::TextCaretWordBack, Action::TextCaretWordForward],
            "word",
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
    // The gap is reserved, not just hoped for: a left run that filled the
    // budget exactly used to run straight into the globals — `^l clearr
    // refresh`, which reads as a typo. One fewer hint is the better trade,
    // and it is the same rule the hints already follow among themselves.
    let left_budget = width.saturating_sub(globals_width + visible_width(HINT_GAP));

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
        // `M-` rather than `⌥` or `alt-`: this is emacs' notation for the
        // meta modifier, and these are emacs' motions.
        KeyBinding::Alt(c) => format!("M-{c}"),
        KeyBinding::Enter if unicode => "⏎".to_string(),
        KeyBinding::Enter => "enter".to_string(),
        KeyBinding::Esc => "esc".to_string(),
        KeyBinding::Backspace => "bksp".to_string(),
        KeyBinding::Tab if unicode => "⇥".to_string(),
        KeyBinding::Tab => "tab".to_string(),
        KeyBinding::BackTab if unicode => "⇤".to_string(),
        KeyBinding::BackTab => "shift-tab".to_string(),
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
    fn the_filter_hints_show_the_rebound_set_keys() {
        // The hint modules never hardcode key names, so neither does the test:
        // it rebinds `add set` and expects the hint bar to follow.
        use crate::config::Bind;

        let keymap = KeyMap::from_bindings(&[Bind::with_mode(
            "n",
            Mode::Filter,
            "filter_set_add",
        )])
        .expect("keymap parses");
        let line = hint_line(
            &hints_for(Mode::Filter, HintContext::default()),
            &keymap,
            Mode::Filter,
            &Theme::default(),
            200,
        );

        assert!(line.to_string().contains('n'));
        assert!(line.to_string().contains("add set"));
    }

    #[test]
    fn the_set_navigation_hints_appear_only_once_there_are_sets_to_navigate() {
        let one = hints_for(Mode::Filter, HintContext::default());
        let many = hints_for(
            Mode::Filter,
            HintContext {
                many_filter_sets: true,
                ..HintContext::default()
            },
        );

        assert!(!one.iter().any(|hint| hint.label == "remove set"));
        assert!(many.iter().any(|hint| hint.label == "remove set"));
        assert!(
            one.iter().any(|hint| hint.label == "add set"),
            "add set is how the feature is discovered, so it always shows"
        );
    }

    #[test]
    fn the_sidebar_key_is_always_offered_and_the_rest_wait_for_it() {
        let closed = hints_for(Mode::Filter, HintContext::default());
        let open = hints_for(
            Mode::Filter,
            HintContext {
                filter_sets_sidebar: true,
                ..HintContext::default()
            },
        );

        assert!(
            closed.iter().any(|hint| hint.label == "sets"),
            "`sets` is how the feature is discovered, so it always shows"
        );
        assert!(!closed.iter().any(|hint| hint.label == "load"));
        assert!(open.iter().any(|hint| hint.label == "load"));
        assert!(open.iter().any(|hint| hint.label == "save"));
    }

    #[test]
    fn copy_to_new_and_delete_appear_only_once_the_panel_is_bound() {
        let unbound = hints_for(
            Mode::Filter,
            HintContext {
                filter_sets_sidebar: true,
                ..HintContext::default()
            },
        );
        let bound = hints_for(
            Mode::Filter,
            HintContext {
                filter_sets_sidebar: true,
                filter_set_loaded: true,
                ..HintContext::default()
            },
        );

        // Both act on the entry the panel is bound to, so neither is offered
        // before there is one. `new` needs no binding, so it always shows.
        assert!(!unbound.iter().any(|hint| hint.label == "copy to new"));
        assert!(!unbound.iter().any(|hint| hint.label == "delete"));
        assert!(unbound.iter().any(|hint| hint.label == "new"));
        assert!(bound.iter().any(|hint| hint.label == "copy to new"));
        assert!(bound.iter().any(|hint| hint.label == "delete"));
    }

    #[test]
    fn the_paging_hint_appears_only_once_there_is_a_second_page() {
        let page = |saved| {
            hints_for(
                Mode::Filter,
                HintContext {
                    filter_sets_sidebar: true,
                    saved_filter_sets: saved,
                    ..HintContext::default()
                },
            )
            .iter()
            .any(|hint| hint.label == "page")
        };

        assert!(!page(9), "nine entries are one window");
        assert!(page(10));
    }

    #[test]
    fn the_prompt_advertises_the_keys_it_reads_outside_the_keymap() {
        let keymap = keymap();
        let theme = Theme::default();

        let line = hint_line(
            &hints_for(Mode::FilterSetName, HintContext::default()),
            &keymap,
            Mode::FilterSetName,
            &theme,
            120,
        )
        .to_string();

        assert!(line.contains("enter save"), "{line}");
        assert!(line.contains("esc cancel"), "{line}");
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
                Mode::FilterSetName,
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
    fn the_global_hints_are_always_separated_from_the_ones_on_their_left() {
        // A left run that filled the budget exactly used to abut the globals,
        // rendering `^l clearr refresh`. Narrow widths are where it showed.
        let keymap = keymap();
        let theme = Theme::default();

        for width in 20..=200 {
            let rendered = hint_line(
                &hints_for(Mode::Filter, HintContext::default()),
                &keymap,
                Mode::Filter,
                &theme,
                width,
            )
            .to_string();

            assert!(
                !rendered.contains("clearr"),
                "hints ran together at width {width}: {rendered:?}"
            );
            if let Some(at) = rendered.find("r refresh") {
                assert!(
                    at == 0 || rendered[..at].ends_with(' '),
                    "no gap before the globals at width {width}: {rendered:?}"
                );
            }
        }
    }

    /// The picker is shared, so the same keys have to name what they do in
    /// whichever editor opened it.
    #[test]
    fn the_calendar_hints_say_save_and_cancel_on_a_task_cell() {
        let filter = hints_for(Mode::Calendar, HintContext::default());
        let cell = hints_for(
            Mode::Calendar,
            HintContext {
                on_task_cell: true,
                ..HintContext::default()
            },
        );

        assert!(filter.iter().any(|hint| hint.label == "pick"));
        assert!(cell.iter().any(|hint| hint.label == "save"));
        assert!(cell.iter().any(|hint| hint.label == "cancel"));
    }

    #[test]
    fn an_alt_binding_renders_as_the_emacs_meta_key() {
        use super::key_text;
        use crate::input::KeyBinding;
        use crate::ui::theme::GlyphSet;

        assert_eq!(
            key_text(&KeyBinding::Alt('b'), &GlyphSet::UNICODE),
            "M-b"
        );
        assert_eq!(key_text(&KeyBinding::Alt('f'), &GlyphSet::ASCII), "M-f");
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

    #[test]
    fn rebinding_a_gantt_key_changes_what_the_hint_bar_shows() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [[bind]]
                key = "w"
                mode = "gantt"
                command = "gantt_zoom_fit"
            "#,
        )
        .expect("config parses");
        let keymap =
            KeyMap::from_bindings(&config.effective_bindings()).expect("keymap builds");
        let theme = Theme::default();

        let line = hint_line(
            &hints_for(
                Mode::Gantt,
                HintContext {
                    timeline_windowed: true,
                    ..HintContext::default()
                },
            ),
            &keymap,
            Mode::Gantt,
            &theme,
            200,
        )
        .to_string();

        assert!(line.contains("w fit"), "{line}");
    }

    #[test]
    fn the_fit_hint_appears_only_once_the_window_has_moved() {
        let keymap = keymap();
        let theme = Theme::default();
        let render = |windowed| {
            hint_line(
                &hints_for(
                    Mode::Gantt,
                    HintContext {
                        timeline_windowed: windowed,
                        ..HintContext::default()
                    },
                ),
                &keymap,
                Mode::Gantt,
                &theme,
                200,
            )
            .to_string()
        };

        assert!(!render(false).contains("fit"));
        assert!(render(true).contains("fit"));
    }
}
