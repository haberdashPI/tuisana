//! Key bindings, actions, and command parsing.
//!
//! This module translates raw terminal keys and config command names into the
//! semantic actions that the app state machine understands.

use std::{collections::HashMap, fmt::Display, str::FromStr};

use crate::{
    config::{Bind, Mode},
    error::{Error, Result},
};

/// A normalized key understood by the binding system.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeyBinding {
    Char(char),
    Ctrl(char),
    Enter,
    Esc,
    Backspace,
    Home,
    End,
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
}

impl FromStr for KeyBinding {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let normalized = s.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "enter" => Ok(Self::Enter),
            "esc" | "escape" => Ok(Self::Esc),
            "backspace" => Ok(Self::Backspace),
            "home" => Ok(Self::Home),
            "end" => Ok(Self::End),
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            "pageup" | "page-up" => Ok(Self::PageUp),
            "pagedown" | "page-down" => Ok(Self::PageDown),
            "space" => Ok(Self::Char(' ')),
            _ if normalized.len() == 1 => Ok(Self::Char(normalized.chars().next().expect("len checked"))),
            _ if normalized.starts_with("ctrl-") && normalized.len() == 6 => {
                Ok(Self::Ctrl(normalized.chars().last().expect("len checked")))
            }
            _ => Err(Error::Backend(format!("unsupported key binding: {s}"))),
        }
    }
}

impl KeyBinding {
    /// A stable ordering used when listing the keys bound to an action.
    ///
    /// Plain letters come first because they are what a user reaches for, then
    /// punctuation, named keys, arrows, and finally control combinations. The
    /// ordering only exists so hints and help render deterministically rather
    /// than in `HashMap` iteration order.
    fn sort_rank(&self) -> (u8, u32) {
        match self {
            Self::Char(c) if c.is_ascii_alphanumeric() => (0, *c as u32),
            Self::Char(c) => (1, *c as u32),
            Self::Enter => (2, 0),
            Self::Esc => (2, 1),
            Self::Backspace => (2, 2),
            Self::Home => (3, 0),
            Self::End => (3, 1),
            Self::Up => (4, 0),
            Self::Down => (4, 1),
            Self::Left => (4, 2),
            Self::Right => (4, 3),
            Self::PageUp => (5, 0),
            Self::PageDown => (5, 1),
            Self::Ctrl(c) => (6, *c as u32),
        }
    }

    /// Converts a crossterm key event into a normalized key binding.
    pub fn from_crossterm_event(event: crossterm::event::KeyEvent) -> Option<Self> {
        use crossterm::event::{KeyCode, KeyModifiers};

        match event.code {
            KeyCode::Char(c) if event.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Self::Ctrl(c.to_ascii_lowercase()))
            }
            KeyCode::Char(c) => Some(Self::Char(c.to_ascii_lowercase())),
            KeyCode::Enter => Some(Self::Enter),
            KeyCode::Esc => Some(Self::Esc),
            KeyCode::Backspace => Some(Self::Backspace),
            KeyCode::Home => Some(Self::Home),
            KeyCode::End => Some(Self::End),
            KeyCode::Left => Some(Self::Left),
            KeyCode::Right => Some(Self::Right),
            KeyCode::Up => Some(Self::Up),
            KeyCode::Down => Some(Self::Down),
            KeyCode::PageUp => Some(Self::PageUp),
            KeyCode::PageDown => Some(Self::PageDown),
            _ => None,
        }
    }
}

/// A semantic action produced by the keybinding system.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Quit,
    MoveUp,
    MoveDown,
    Open,
    Refresh,
    ToggleHelpDetails,
    StartSearch,
    ClearSearch,
    ToggleSelection,
    SelectAllVisible,
    InvertSelection,
    ClearSelection,
    UndoSelection,
    RedoSelection,
    ToggleOnlySelected,
    SelectAllStarredVisible,
    SelectAllNonHiddenVisible,
    ToggleStarredSelected,
    ToggleHiddenSelected,
    ToggleHiddenGroup,
    ToggleTaskMode,
    SetProjectMode,
    SetFilterMode,
    SetTaskMode,
    ToggleTaskFilters,
    BeginFilterEdit,
    CycleFilterStringMode,
    SearchFuzzy,
    SearchSubstring,
    SearchRegex,
    JumpTop,
    JumpBottom,
    PageUp,
    PageDown,
    ResizeTopPaneUp,
    ResizeTopPaneDown,
    MinimizeTopPane,
    MaximizeTopPane,
    RestoreTopPane,
    ScrollLeft,
    ScrollRight,
    MoveSectionUp,
    MoveSectionDown,
    MoveProjectUp,
    MoveProjectDown,
    ToggleCompletedFilter,
    ToggleSubtaskVisibility,
    ToggleProjectGrouping,
    ToggleSectionGrouping,
    CycleTaskSort,
    /// Commit the current filter field edit and return to filter-browse mode.
    FilterDoneEditing,
    /// Discard the current filter field edit, close the filter panel, and go to task mode.
    FilterCancelEditing,
    FilterMoveLabelLeft,
    FilterMoveLabelRight,
    FilterCycleLabelUp,
    FilterCycleLabelDown,
    FilterAddLabel,
    FilterDeleteLabel,
    ToggleTaskSelection,
    SelectAllVisibleTasks,
    InvertTaskSelection,
    ClearTaskSelection,
    ClearHiddenTaskSelection,
    CopyTasksToClipboard,
}

impl Action {
    /// Parses a config command name into an action.
    pub fn from_command(command: &str) -> Result<Self> {
        match command.trim().to_ascii_lowercase().as_str() {
            "quit" => Ok(Self::Quit),
            "move_up" => Ok(Self::MoveUp),
            "move_down" => Ok(Self::MoveDown),
            "open" => Ok(Self::Open),
            "refresh" => Ok(Self::Refresh),
            "toggle_help_details" => Ok(Self::ToggleHelpDetails),
            "start_search" => Ok(Self::StartSearch),
            "clear_search" => Ok(Self::ClearSearch),
            "toggle_selection" => Ok(Self::ToggleSelection),
            "select_all_visible" => Ok(Self::SelectAllVisible),
            "invert_selection" => Ok(Self::InvertSelection),
            "clear_selection" => Ok(Self::ClearSelection),
            "undo_selection" => Ok(Self::UndoSelection),
            "redo_selection" => Ok(Self::RedoSelection),
            "toggle_only_selected" => Ok(Self::ToggleOnlySelected),
            "select_all_starred_visible" => Ok(Self::SelectAllStarredVisible),
            "select_all_non_hidden_visible" => Ok(Self::SelectAllNonHiddenVisible),
            "toggle_starred_selected" => Ok(Self::ToggleStarredSelected),
            "toggle_hidden_selected" => Ok(Self::ToggleHiddenSelected),
            "toggle_hidden_group" => Ok(Self::ToggleHiddenGroup),
            "toggle_task_mode" => Ok(Self::ToggleTaskMode),
            "set_project_mode" | "set_project_bind_mode" => Ok(Self::SetProjectMode),
            "set_filter_mode" => Ok(Self::SetFilterMode),
            "set_task_mode" | "set_task_bind_mode" => Ok(Self::SetTaskMode),
            "toggle_task_filters" => Ok(Self::ToggleTaskFilters),
            "begin_filter_edit" => Ok(Self::BeginFilterEdit),
            "cycle_filter_string_mode" => Ok(Self::CycleFilterStringMode),
            "search_fuzzy" => Ok(Self::SearchFuzzy),
            "search_substring" => Ok(Self::SearchSubstring),
            "search_regex" => Ok(Self::SearchRegex),
            "jump_top" => Ok(Self::JumpTop),
            "jump_bottom" => Ok(Self::JumpBottom),
            "page_up" => Ok(Self::PageUp),
            "page_down" => Ok(Self::PageDown),
            "resize_top_pane_up" | "resize_window_up" => Ok(Self::ResizeTopPaneUp),
            "resize_top_pane_down" | "resize_window_down" => Ok(Self::ResizeTopPaneDown),
            "minimize_top_pane" | "minimize_window" => Ok(Self::MinimizeTopPane),
            "maximize_top_pane" | "maximize_window" => Ok(Self::MaximizeTopPane),
            "restore_top_pane" | "restore_window" => Ok(Self::RestoreTopPane),
            "scroll_left" => Ok(Self::ScrollLeft),
            "scroll_right" => Ok(Self::ScrollRight),
            "move_section_up" => Ok(Self::MoveSectionUp),
            "move_section_down" => Ok(Self::MoveSectionDown),
            "move_project_up" => Ok(Self::MoveProjectUp),
            "move_project_down" => Ok(Self::MoveProjectDown),
            "toggle_completed_filter" => Ok(Self::ToggleCompletedFilter),
            "toggle_subtask_visibility" => Ok(Self::ToggleSubtaskVisibility),
            "toggle_project_grouping" => Ok(Self::ToggleProjectGrouping),
            "toggle_section_grouping" => Ok(Self::ToggleSectionGrouping),
            "cycle_task_sort" => Ok(Self::CycleTaskSort),
            "filter_done_editing" => Ok(Self::FilterDoneEditing),
            "filter_cancel_editing" => Ok(Self::FilterCancelEditing),
            "filter_move_label_left" => Ok(Self::FilterMoveLabelLeft),
            "filter_move_label_right" => Ok(Self::FilterMoveLabelRight),
            "filter_cycle_label_up" => Ok(Self::FilterCycleLabelUp),
            "filter_cycle_label_down" => Ok(Self::FilterCycleLabelDown),
            "filter_add_label" => Ok(Self::FilterAddLabel),
            "filter_delete_label" => Ok(Self::FilterDeleteLabel),
            "toggle_task_selection" => Ok(Self::ToggleTaskSelection),
            "select_all_visible_tasks" => Ok(Self::SelectAllVisibleTasks),
            "invert_task_selection" => Ok(Self::InvertTaskSelection),
            "clear_task_selection" => Ok(Self::ClearTaskSelection),
            "clear_hidden_task_selection" => Ok(Self::ClearHiddenTaskSelection),
            "copy_tasks_to_clipboard" => Ok(Self::CopyTasksToClipboard),
            other => Err(Error::Backend(format!("unsupported command: {other}"))),
        }
    }

    /// Returns `true` for task-view actions that should be routed to the task pane
    /// even when the project list is also visible.
    pub fn is_task_view_action(&self) -> bool {
        matches!(
            self,
            Action::MoveSectionUp
                | Action::MoveSectionDown
                | Action::MoveProjectUp
                | Action::MoveProjectDown
                | Action::ScrollLeft
                | Action::ScrollRight
                | Action::PageUp
                | Action::PageDown
                | Action::ToggleCompletedFilter
                | Action::ToggleSubtaskVisibility
                | Action::ToggleProjectGrouping
                | Action::ToggleSectionGrouping
                | Action::CycleTaskSort
        )
    }

    /// Returns `true` for label-navigation actions used in filter-edit mode.
    ///
    /// These are context-sensitive: they navigate label columns when a labels field
    /// is selected, but fall back to pushing the character when on a text field.
    /// They must be routed to `handle_filter_field_input` rather than `handle_action`.
    pub fn is_label_filter_action(&self) -> bool {
        matches!(
            self,
            Action::FilterMoveLabelLeft
                | Action::FilterMoveLabelRight
                | Action::FilterCycleLabelUp
                | Action::FilterCycleLabelDown
                | Action::FilterAddLabel
                | Action::FilterDeleteLabel
        )
    }

}

/// Top-level commands the app can trigger directly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppCommand {
    Quit,
    Refresh,
    OpenUrl(String),
    CopyToClipboard(String),
}

impl Action {
    /// Maps an action to a direct application command when one exists.
    pub fn as_app_command(&self) -> Option<AppCommand> {
        match self {
            Action::Quit => Some(AppCommand::Quit),
            Action::Refresh => Some(AppCommand::Refresh),
            _ => None,
        }
    }
}

impl Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Action::Quit => "quit",
            Action::MoveUp => "move_up",
            Action::MoveDown => "move_down",
            Action::Open => "open",
            Action::Refresh => "refresh",
            Action::ToggleHelpDetails => "toggle_help_details",
            Action::StartSearch => "start_search",
            Action::ClearSearch => "clear_search",
            Action::ToggleSelection => "toggle_selection",
            Action::SelectAllVisible => "select_all_visible",
            Action::InvertSelection => "invert_selection",
            Action::ClearSelection => "clear_selection",
            Action::UndoSelection => "undo_selection",
            Action::RedoSelection => "redo_selection",
            Action::ToggleOnlySelected => "toggle_only_selected",
            Action::SelectAllStarredVisible => "select_all_starred_visible",
            Action::SelectAllNonHiddenVisible => "select_all_non_hidden_visible",
            Action::ToggleStarredSelected => "toggle_starred_selected",
            Action::ToggleHiddenSelected => "toggle_hidden_selected",
            Action::ToggleHiddenGroup => "toggle_hidden_group",
            Action::ToggleTaskMode => "toggle_task_mode",
            Action::SetProjectMode => "set_project_mode",
            Action::SetFilterMode => "set_filter_mode",
            Action::SetTaskMode => "set_task_mode",
            Action::ToggleTaskFilters => "toggle_task_filters",
            Action::BeginFilterEdit => "begin_filter_edit",
            Action::CycleFilterStringMode => "cycle_filter_string_mode",
            Action::SearchFuzzy => "search_fuzzy",
            Action::SearchSubstring => "search_substring",
            Action::SearchRegex => "search_regex",
            Action::JumpTop => "jump_top",
            Action::JumpBottom => "jump_bottom",
            Action::PageUp => "page_up",
            Action::PageDown => "page_down",
            Action::ResizeTopPaneUp => "resize_top_pane_up",
            Action::ResizeTopPaneDown => "resize_top_pane_down",
            Action::MinimizeTopPane => "minimize_top_pane",
            Action::MaximizeTopPane => "maximize_top_pane",
            Action::RestoreTopPane => "restore_top_pane",
            Action::ScrollLeft => "scroll_left",
            Action::ScrollRight => "scroll_right",
            Action::MoveSectionUp => "move_section_up",
            Action::MoveSectionDown => "move_section_down",
            Action::MoveProjectUp => "move_project_up",
            Action::MoveProjectDown => "move_project_down",
            Action::ToggleCompletedFilter => "toggle_completed_filter",
            Action::ToggleSubtaskVisibility => "toggle_subtask_visibility",
            Action::ToggleProjectGrouping => "toggle_project_grouping",
            Action::ToggleSectionGrouping => "toggle_section_grouping",
            Action::CycleTaskSort => "cycle_task_sort",
            Action::FilterDoneEditing => "filter_done_editing",
            Action::FilterCancelEditing => "filter_cancel_editing",
            Action::FilterMoveLabelLeft => "filter_move_label_left",
            Action::FilterMoveLabelRight => "filter_move_label_right",
            Action::FilterCycleLabelUp => "filter_cycle_label_up",
            Action::FilterCycleLabelDown => "filter_cycle_label_down",
            Action::FilterAddLabel => "filter_add_label",
            Action::FilterDeleteLabel => "filter_delete_label",
            Action::ToggleTaskSelection => "toggle_task_selection",
            Action::SelectAllVisibleTasks => "select_all_visible_tasks",
            Action::InvertTaskSelection => "invert_task_selection",
            Action::ClearTaskSelection => "clear_task_selection",
            Action::ClearHiddenTaskSelection => "clear_hidden_task_selection",
            Action::CopyTasksToClipboard => "copy_tasks_to_clipboard",
        };
        f.write_str(name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyMap {
    bindings: HashMap<(Mode, KeyBinding), Action>,
}

impl KeyMap {
    pub fn from_bindings(bindings: &[Bind]) -> Result<Self> {
        let mut map = HashMap::new();
        for bind in bindings {
            let key = bind.key.parse()?;
            let action = Action::from_command(&bind.command)?;
            map.insert((bind.mode, key), action);
        }
        Ok(Self { bindings: map })
    }

    pub fn action_for(&self, key: &KeyBinding, mode: Mode) -> Option<&Action> {
        self.bindings.get(&(mode, key.clone())).or_else(|| {
            if mode.allows_any_fallback() {
                self.bindings.get(&(Mode::Any, key.clone()))
            } else {
                None
            }
        })
    }

    /// Every key that triggers `action` in `mode`, in a stable display order.
    ///
    /// This is the inverse of [`KeyMap::action_for`] and agrees with it: a key
    /// bound globally but shadowed by a mode-specific binding is not reported
    /// for the global action, so hints and help never advertise a key that
    /// would do something else if pressed.
    pub fn keys_for(&self, action: &Action, mode: Mode) -> Vec<KeyBinding> {
        let mut keys = self
            .bindings
            .iter()
            .filter(|((bind_mode, _), bound)| {
                *bound == action
                    && (*bind_mode == mode
                        || (*bind_mode == Mode::Any && mode.allows_any_fallback()))
            })
            .map(|((_, key), _)| key.clone())
            .filter(|key| self.action_for(key, mode) == Some(action))
            .collect::<Vec<_>>();

        keys.sort_by_key(KeyBinding::sort_rank);
        keys.dedup();
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, AppCommand, KeyBinding, KeyMap};
    use crate::config::Mode;
    use crate::config::Bind;

    #[test]
    fn parses_keys_and_builds_keymap() {
        let keymap = KeyMap::from_bindings(&[
            Bind::new("?", "toggle_help_details"),
            Bind::new("x", "quit"),
            Bind::new("j", "move_down"),
            Bind::new("ctrl-u", "page_up"),
            Bind::new("space", "toggle_selection"),
            Bind::new("home", "jump_top"),
        ])
        .expect("keymap parses");

        assert_eq!(
            keymap.action_for(&KeyBinding::Char('x'), Mode::Any),
            Some(&Action::Quit)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('j'), Mode::Any),
            Some(&Action::MoveDown)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('u'), Mode::Any),
            Some(&Action::PageUp)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('?'), Mode::Any),
            Some(&Action::ToggleHelpDetails)
        );
    }

    #[test]
    fn parses_key_aliases_and_commands() {
        assert_eq!("enter".parse::<KeyBinding>().expect("enter parses"), KeyBinding::Enter);
        assert_eq!("esc".parse::<KeyBinding>().expect("esc parses"), KeyBinding::Esc);
        assert_eq!(
            "backspace".parse::<KeyBinding>().expect("backspace parses"),
            KeyBinding::Backspace
        );
        assert_eq!("home".parse::<KeyBinding>().expect("home parses"), KeyBinding::Home);
        assert_eq!("end".parse::<KeyBinding>().expect("end parses"), KeyBinding::End);
        assert_eq!("left".parse::<KeyBinding>().expect("left parses"), KeyBinding::Left);
        assert_eq!(
            "right".parse::<KeyBinding>().expect("right parses"),
            KeyBinding::Right
        );
        assert_eq!("space".parse::<KeyBinding>().expect("space parses"), KeyBinding::Char(' '));
        assert_eq!("page-up".parse::<KeyBinding>().expect("page-up parses"), KeyBinding::PageUp);
        assert_eq!(Action::from_command("open").expect("open parses"), Action::Open);
        assert_eq!(Action::from_command("refresh").expect("refresh parses"), Action::Refresh);
        assert_eq!(
            Action::from_command("toggle_help_details").expect("toggle_help_details parses"),
            Action::ToggleHelpDetails
        );
        assert_eq!(
            Action::from_command("toggle_selection").expect("toggle_selection parses"),
            Action::ToggleSelection
        );
        assert_eq!(
            Action::from_command("clear_search").expect("clear_search parses"),
            Action::ClearSearch
        );
        assert_eq!(
            Action::from_command("select_all_visible").expect("select_all_visible parses"),
            Action::SelectAllVisible
        );
        assert_eq!(
            Action::from_command("toggle_hidden_group").expect("toggle_hidden_group parses"),
            Action::ToggleHiddenGroup
        );
        assert_eq!(
            Action::from_command("cycle_filter_string_mode")
                .expect("cycle_filter_string_mode parses"),
            Action::CycleFilterStringMode
        );
        assert_eq!(
            Action::from_command("search_regex").expect("search_regex parses"),
            Action::SearchRegex
        );
        assert_eq!(Action::from_command("jump_top").expect("jump_top parses"), Action::JumpTop);
        assert_eq!(
            Action::from_command("scroll_left").expect("scroll_left parses"),
            Action::ScrollLeft
        );
        assert_eq!(
            Action::from_command("scroll_right").expect("scroll_right parses"),
            Action::ScrollRight
        );
        assert_eq!(
            Action::from_command("resize_top_pane_up").expect("resize_top_pane_up parses"),
            Action::ResizeTopPaneUp
        );
        assert_eq!(
            Action::from_command("resize_window_up").expect("legacy resize_window_up parses"),
            Action::ResizeTopPaneUp
        );
    }

    #[test]
    fn rejects_unknown_key_and_command() {
        let key_err = "meta-x"
            .parse::<KeyBinding>()
            .expect_err("unknown key should fail");
        assert!(format!("{key_err}").contains("unsupported key binding"));

        let command_err = Action::from_command("launch").expect_err("unknown command should fail");
        assert!(format!("{command_err}").contains("unsupported command"));
    }

    #[test]
    fn maps_app_commands() {
        assert_eq!(Action::Quit.as_app_command(), Some(AppCommand::Quit));
        assert_eq!(Action::Refresh.as_app_command(), Some(AppCommand::Refresh));
        assert_eq!(Action::MoveDown.as_app_command(), None);
        assert_eq!(Action::ToggleSelection.as_app_command(), None);
        assert_eq!(Action::ToggleHiddenSelected.as_app_command(), None);
    }

    #[test]
    fn mode_specific_bindings_override_any_bindings() {
        let keymap = KeyMap::from_bindings(&[
            Bind::with_mode("[", Mode::Any, "scroll_left"),
            Bind::with_mode("[", Mode::Project, "resize_top_pane_down"),
            Bind::with_mode("[", Mode::ProjectSearch, "clear_search"),
            Bind::with_mode("[", Mode::Filter, "resize_top_pane_up"),
            Bind::with_mode("[", Mode::FilterEdit, "toggle_help_details"),
            Bind::with_mode("[", Mode::Task, "move_section_up"),
        ])
        .expect("keymap parses");

        assert_eq!(
            keymap.action_for(&KeyBinding::Char('['), Mode::Project),
            Some(&Action::ResizeTopPaneDown)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('['), Mode::ProjectSearch),
            Some(&Action::ClearSearch)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('['), Mode::Filter),
            Some(&Action::ResizeTopPaneUp)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('['), Mode::FilterEdit),
            Some(&Action::ToggleHelpDetails)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('['), Mode::Task),
            Some(&Action::MoveSectionUp)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('['), Mode::Any),
            Some(&Action::ScrollLeft)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('x'), Mode::ProjectSearch),
            None
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('x'), Mode::FilterEdit),
            None
        );
    }

    #[test]
    fn keys_for_lists_bound_keys_in_a_stable_order() {
        let keymap = KeyMap::from_bindings(&[
            Bind::new("j", "move_down"),
            Bind::new("down", "move_down"),
            Bind::new("ctrl-n", "move_down"),
            Bind::new("k", "move_up"),
        ])
        .expect("keymap parses");

        assert_eq!(
            keymap.keys_for(&Action::MoveDown, Mode::Any),
            vec![
                KeyBinding::Char('j'),
                KeyBinding::Down,
                KeyBinding::Ctrl('n')
            ]
        );
        assert!(keymap.keys_for(&Action::Quit, Mode::Any).is_empty());
    }

    #[test]
    fn keys_for_excludes_keys_shadowed_by_a_mode_specific_binding() {
        let keymap = KeyMap::from_bindings(&[
            Bind::new("j", "move_down"),
            Bind::new("down", "move_down"),
            Bind::with_mode("j", Mode::FilterEdit, "filter_cycle_label_down"),
        ])
        .expect("keymap parses");

        // `j` means something else while editing a filter, so it must not be
        // advertised as move_down there.
        assert_eq!(keymap.keys_for(&Action::MoveDown, Mode::Task), vec![
            KeyBinding::Char('j'),
            KeyBinding::Down
        ]);
        assert!(keymap.keys_for(&Action::MoveDown, Mode::FilterEdit).is_empty());
        assert_eq!(
            keymap.keys_for(&Action::FilterCycleLabelDown, Mode::FilterEdit),
            vec![KeyBinding::Char('j')]
        );
    }
}
