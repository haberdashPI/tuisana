//! The `?` help overlay.
//!
//! Long-form help used to live inline at the top of the screen, where it cost
//! up to eleven lines and reflowed the whole layout every time it was toggled.
//! It is now a centered modal drawn over the panes: the layout underneath never
//! moves, and the bindings can be grouped by intent instead of crammed onto
//! comma-separated lines.
//!
//! The overlay captures no keys. `?` toggles it, exactly as it toggled the
//! inline help before, and every other key keeps doing what it always did.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use crate::{
    config::Mode,
    input::{Action, KeyMap},
    ui::{
        chrome::pane_block,
        hints::{all_key_texts, key_column_spans, Hint},
        text::{pad_cell, visible_width},
        theme::Theme,
    },
};

/// Widest the overlay will grow, so lines stay comfortably readable.
const MAX_WIDTH: u16 = 84;
/// Gap between the two columns of groups.
const COLUMN_GAP: usize = 4;
/// How many alternate keys to list per binding.
const KEYS_PER_ACTION: usize = 2;

/// A titled group of related bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpGroup {
    /// The group heading.
    pub title: &'static str,
    /// The bindings in the group.
    pub entries: Vec<Hint>,
}

impl HelpGroup {
    fn new(title: &'static str, entries: Vec<Hint>) -> Self {
        Self { title, entries }
    }
}

/// The binding groups relevant to a mode, mode-specific first.
pub fn groups_for(mode: Mode) -> Vec<HelpGroup> {
    let mut groups = match mode {
        Mode::Task => task_groups(),
        Mode::Filter | Mode::FilterEdit => filter_groups(),
        Mode::Project | Mode::ProjectSearch | Mode::Any => project_groups(),
    };
    groups.extend(shared_groups());
    groups
}

fn project_groups() -> Vec<HelpGroup> {
    vec![
        HelpGroup::new(
            "Select",
            vec![
                Hint::new(&[Action::ToggleSelection], "select, then move down"),
                Hint::new(&[Action::SelectAllVisible], "select all visible"),
                Hint::new(&[Action::SelectAllStarredVisible], "select starred"),
                Hint::new(&[Action::SelectAllNonHiddenVisible], "select non-hidden"),
                Hint::new(&[Action::InvertSelection], "invert"),
                Hint::new(&[Action::ClearSelection], "clear"),
                Hint::new(&[Action::UndoSelection], "undo"),
                Hint::new(&[Action::RedoSelection], "redo"),
            ],
        ),
        HelpGroup::new(
            "Projects",
            vec![
                Hint::new(&[Action::Open], "open in Asana"),
                Hint::new(&[Action::ToggleStarredSelected], "star selected"),
                Hint::new(&[Action::ToggleHiddenSelected], "hide selected"),
                Hint::new(&[Action::ToggleHiddenGroup], "show hidden"),
                Hint::new(&[Action::ToggleOnlySelected], "only selected"),
            ],
        ),
        HelpGroup::new(
            "Search",
            vec![
                Hint::new(&[Action::StartSearch], "start"),
                Hint::new(&[Action::ClearSearch], "clear"),
                Hint::new(&[Action::SearchFuzzy], "fuzzy"),
                Hint::new(&[Action::SearchSubstring], "contains"),
                Hint::new(&[Action::SearchRegex], "regex"),
            ],
        ),
    ]
}

fn task_groups() -> Vec<HelpGroup> {
    vec![
        HelpGroup::new(
            "Tasks",
            vec![
                Hint::new(&[Action::Open], "open in Asana"),
                Hint::new(&[Action::ToggleTaskSelection], "select"),
                Hint::new(&[Action::SelectAllVisibleTasks], "select all"),
                Hint::new(&[Action::InvertTaskSelection], "invert"),
                Hint::new(&[Action::ClearTaskSelection], "clear"),
                Hint::new(&[Action::ClearHiddenTaskSelection], "clear hidden"),
                Hint::new(&[Action::CopyTasksToClipboard], "copy links"),
            ],
        ),
        HelpGroup::new(
            "Group & sort",
            vec![
                Hint::new(
                    &[Action::MoveSectionUp, Action::MoveSectionDown],
                    "jump section",
                ),
                Hint::new(
                    &[Action::MoveProjectUp, Action::MoveProjectDown],
                    "jump project",
                ),
                Hint::new(&[Action::ToggleProjectGrouping], "group by project"),
                Hint::new(&[Action::ToggleSectionGrouping], "group by section"),
                Hint::new(&[Action::CycleTaskSort], "cycle sort field"),
            ],
        ),
        HelpGroup::new(
            "View",
            vec![
                Hint::new(&[Action::SetFilterMode], "filter panel"),
                Hint::new(&[Action::ToggleCompletedFilter], "open / done / all"),
                Hint::new(&[Action::ToggleSubtaskVisibility], "subtasks"),
                Hint::new(&[Action::ScrollLeft, Action::ScrollRight], "scroll columns"),
            ],
        ),
    ]
}

fn filter_groups() -> Vec<HelpGroup> {
    vec![
        HelpGroup::new(
            "Filters",
            vec![
                Hint::new(&[Action::BeginFilterEdit], "edit field"),
                Hint::new(&[Action::FilterDoneEditing], "commit edit"),
                Hint::new(&[Action::FilterCancelEditing], "cancel edit"),
                Hint::new(&[Action::CycleFilterStringMode], "cycle match mode"),
                Hint::new(&[Action::ClearSearch], "clear field"),
                Hint::new(&[Action::ToggleTaskFilters], "close panel"),
            ],
        ),
        HelpGroup::new(
            "Label values",
            vec![
                Hint::new(
                    &[Action::FilterCycleLabelDown, Action::FilterCycleLabelUp],
                    "cycle value",
                ),
                Hint::new(
                    &[Action::FilterMoveLabelLeft, Action::FilterMoveLabelRight],
                    "move between values",
                ),
                Hint::new(&[Action::FilterAddLabel], "add value"),
                Hint::new(&[Action::FilterDeleteLabel], "remove value"),
            ],
        ),
        HelpGroup::new(
            "Match modes",
            vec![
                Hint::new(&[Action::SearchFuzzy], "fuzzy"),
                Hint::new(&[Action::SearchSubstring], "contains"),
                Hint::new(&[Action::SearchRegex], "regex"),
                Hint::literal("date", "YYYY-MM-DD, MM-DD, today, mon"),
            ],
        ),
    ]
}

fn shared_groups() -> Vec<HelpGroup> {
    vec![
        HelpGroup::new(
            "Move",
            vec![
                Hint::new(&[Action::MoveDown, Action::MoveUp], "down / up"),
                Hint::new(&[Action::PageDown, Action::PageUp], "page down / up"),
                Hint::new(&[Action::JumpTop, Action::JumpBottom], "top / bottom"),
            ],
        ),
        HelpGroup::new(
            "Modes",
            vec![
                Hint::new(&[Action::SetProjectMode], "projects"),
                Hint::new(&[Action::SetFilterMode], "filters"),
                Hint::new(&[Action::SetTaskMode], "tasks"),
                Hint::new(&[Action::ToggleTaskMode], "toggle tasks"),
            ],
        ),
        HelpGroup::new(
            "Panes",
            vec![
                Hint::new(
                    &[Action::ResizeTopPaneDown, Action::ResizeTopPaneUp],
                    "shrink / grow",
                ),
                Hint::new(&[Action::MinimizeTopPane], "minimize"),
                Hint::new(&[Action::MaximizeTopPane], "maximize"),
                Hint::new(&[Action::RestoreTopPane], "restore"),
            ],
        ),
        HelpGroup::new(
            "App",
            vec![
                Hint::new(&[Action::Refresh], "refresh"),
                Hint::new(&[Action::Quit], "quit"),
                Hint::new(&[Action::ToggleHelpDetails], "close this help"),
            ],
        ),
    ]
}

/// Renders the overlay centered over `area`.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    keymap: &KeyMap,
    mode: Mode,
) {
    let groups = resolve(groups_for(mode), keymap, mode, theme);
    if groups.is_empty() {
        return;
    }

    let key_width = groups
        .iter()
        .flat_map(|group| group.1.iter())
        .map(|(keys, _)| visible_width(keys))
        .max()
        .unwrap_or(0);

    let content_width = groups
        .iter()
        .flat_map(|group| group.1.iter())
        .map(|(_, label)| key_width + 2 + visible_width(label))
        .chain(groups.iter().map(|group| visible_width(group.0)))
        .max()
        .unwrap_or(20);

    let columns = column_lines(&groups, key_width, content_width, theme);
    let inner_width = (content_width * 2 + COLUMN_GAP).min(MAX_WIDTH as usize - 2);
    let lines = merge_columns(columns, content_width, inner_width, theme);

    let box_area = centered(
        area,
        (inner_width as u16).saturating_add(2),
        (lines.len() as u16).saturating_add(2),
    );
    let block = pane_block(theme, true, mode, &format!("Help · {} mode", mode.label()), &[]);
    let inner = block.inner(box_area);

    frame.render_widget(Clear, box_area);
    frame.render_widget(block, box_area);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Drops unbound entries and empty groups, resolving each entry's key text.
fn resolve(
    groups: Vec<HelpGroup>,
    keymap: &KeyMap,
    mode: Mode,
    theme: &Theme,
) -> Vec<(&'static str, Vec<(String, &'static str)>)> {
    let mut resolved = Vec::new();

    for group in groups {
        let entries = group
            .entries
            .into_iter()
            .filter_map(|entry| entry_keys(&entry, keymap, mode, theme).map(|keys| (keys, entry.label)))
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            resolved.push((group.title, entries));
        }
    }

    resolved
}

fn entry_keys(entry: &Hint, keymap: &KeyMap, mode: Mode, theme: &Theme) -> Option<String> {
    if let Some(literal) = entry.literal {
        return Some(literal.to_string());
    }

    let limit = if entry.keys.len() > 1 { 1 } else { KEYS_PER_ACTION };
    let keys = entry
        .keys
        .iter()
        .map(|action| all_key_texts(action, keymap, mode, &theme.glyphs, limit).join(" "))
        .filter(|keys| !keys.is_empty())
        .collect::<Vec<_>>();

    (!keys.is_empty()).then(|| keys.join("/"))
}

/// Packs the groups into two columns, split where the taller column is shortest.
///
/// Groups are never reordered or split: the reader should be able to skim the
/// left column top to bottom and then the right one.
fn column_lines(
    groups: &[(&'static str, Vec<(String, &'static str)>)],
    key_width: usize,
    content_width: usize,
    theme: &Theme,
) -> [Vec<Line<'static>>; 2] {
    let split = balanced_split(groups);

    let mut columns: [Vec<Line<'static>>; 2] = [Vec::new(), Vec::new()];

    for (position, (title, entries)) in groups.iter().enumerate() {
        let index = usize::from(position >= split);

        if !columns[index].is_empty() {
            columns[index].push(Line::default());
        }
        columns[index].push(Line::from(Span::styled(
            pad_cell(title, content_width, theme.glyphs.ellipsis),
            theme.subtitle,
        )));
        for (keys, label) in entries {
            columns[index].push(Line::from(key_column_spans(
                keys,
                label,
                key_width,
                theme.key,
                theme.text,
            )));
        }
    }

    columns
}

/// Finds the group index to break at so the taller column is as short as
/// possible.
fn balanced_split(groups: &[(&'static str, Vec<(String, &'static str)>)]) -> usize {
    // A group costs its entries plus a heading, and every group after the
    // first in a column also costs a blank separator line.
    let heights = groups
        .iter()
        .map(|group| group.1.len() + 1)
        .collect::<Vec<_>>();
    let column_height = |range: &[usize]| -> usize {
        range.iter().sum::<usize>() + range.len().saturating_sub(1)
    };

    (1..=heights.len())
        .min_by_key(|split| {
            let (left, right) = heights.split_at(*split);
            column_height(left).max(column_height(right))
        })
        .unwrap_or(heights.len())
}

/// Zips the two columns into one set of lines, padding the left column.
fn merge_columns(
    columns: [Vec<Line<'static>>; 2],
    content_width: usize,
    inner_width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let [left, right] = columns;
    let height = left.len().max(right.len());
    let mut lines = Vec::with_capacity(height);

    for index in 0..height {
        let mut spans = Vec::new();
        let left_line = left.get(index);
        let used = left_line
            .map(|line| visible_width(&line.to_string()))
            .unwrap_or(0);
        if let Some(line) = left_line {
            spans.extend(line.spans.clone());
        }
        if let Some(line) = right.get(index) {
            spans.push(Span::raw(
                " ".repeat(content_width.saturating_sub(used) + COLUMN_GAP),
            ));
            spans.extend(line.spans.clone());
        }
        lines.push(Line::from(spans));
    }

    // A trailing dim reminder of how to dismiss the overlay.
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        crate::ui::text::pad_cell_right_aligned(
            "? to close",
            inner_width,
            theme.glyphs.ellipsis,
        ),
        theme.muted,
    )));

    lines
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::{groups_for, resolve};
    use crate::{
        config::{Config, Mode},
        input::KeyMap,
        ui::theme::Theme,
    };

    fn keymap() -> KeyMap {
        KeyMap::from_bindings(&Config::default().effective_bindings()).expect("keymap builds")
    }

    #[test]
    fn every_mode_gets_mode_specific_groups_before_the_shared_ones() {
        for (mode, first) in [
            (Mode::Task, "Tasks"),
            (Mode::Filter, "Filters"),
            (Mode::Project, "Select"),
        ] {
            let groups = groups_for(mode);
            assert_eq!(groups.first().expect("a group").title, first);
            assert_eq!(groups.last().expect("a group").title, "App");
        }
    }

    #[test]
    fn resolved_entries_have_keys_and_unbound_entries_are_dropped() {
        let theme = Theme::default();
        let resolved = resolve(groups_for(Mode::Task), &keymap(), Mode::Task, &theme);

        assert!(!resolved.is_empty());
        for (title, entries) in &resolved {
            assert!(!entries.is_empty(), "group {title} should have been dropped");
            for (keys, label) in entries {
                assert!(!keys.is_empty(), "{title}/{label} resolved to no keys");
            }
        }
    }

    #[test]
    fn the_column_split_minimizes_the_taller_column() {
        let group = |title: &'static str, count: usize| {
            (
                title,
                (0..count)
                    .map(|index| (format!("k{index}"), "label"))
                    .collect::<Vec<_>>(),
            )
        };

        // Two small groups and one large one: breaking after the two small
        // ones gives 5 and 11 rather than 2 and 14.
        assert_eq!(
            super::balanced_split(&[group("a", 1), group("b", 1), group("c", 10)]),
            2
        );
        // Equal groups split down the middle.
        assert_eq!(
            super::balanced_split(&[group("a", 4), group("b", 4), group("c", 4), group("d", 4)]),
            2
        );
        assert_eq!(super::balanced_split(&[group("a", 1)]), 1);
    }

    #[test]
    fn groups_with_nothing_bound_are_dropped_entirely() {
        let theme = Theme::default();
        let empty = KeyMap::from_bindings(&[]).expect("empty keymap builds");

        let resolved = resolve(groups_for(Mode::Task), &empty, Mode::Task, &theme);

        // Only literal entries survive with no bindings at all.
        assert!(resolved.iter().all(|(_, entries)| entries
            .iter()
            .all(|(keys, _)| !keys.is_empty())));
        assert!(resolved.len() <= 1);
    }
}
