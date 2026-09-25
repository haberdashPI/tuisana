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
    /// Option/Alt plus a letter, written `alt-x`.
    ///
    /// Needs the terminal to send Option as a modifier rather than as an
    /// escape prefix. The prefix form is deliberately not supported: a lone
    /// `ESC` is `esc` here, and telling the two apart by timing is how editors
    /// get famously confused.
    Alt(char),
    Enter,
    Esc,
    Backspace,
    Tab,
    /// Shift plus tab, written `shift-tab`.
    ///
    /// Its own key rather than a modifier on `Tab`: terminals send it as a
    /// distinct code, and `KeyBinding` has no modifier for shift precisely
    /// because a shifted letter is indistinguishable from the letter.
    BackTab,
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
            "tab" => Ok(Self::Tab),
            "shift-tab" | "backtab" => Ok(Self::BackTab),
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
            _ if normalized.starts_with("alt-") && normalized.len() == 5 => {
                Ok(Self::Alt(normalized.chars().last().expect("len checked")))
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
            Self::Tab => (2, 3),
            Self::BackTab => (2, 4),
            Self::Home => (3, 0),
            Self::End => (3, 1),
            Self::Up => (4, 0),
            Self::Down => (4, 1),
            Self::Left => (4, 2),
            Self::Right => (4, 3),
            Self::PageUp => (5, 0),
            Self::PageDown => (5, 1),
            Self::Ctrl(c) => (6, *c as u32),
            Self::Alt(c) => (7, *c as u32),
        }
    }

    /// Converts a crossterm key event into a normalized key binding.
    pub fn from_crossterm_event(event: crossterm::event::KeyEvent) -> Option<Self> {
        use crossterm::event::{KeyCode, KeyModifiers};

        match event.code {
            KeyCode::Char(c) if event.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Self::Ctrl(c.to_ascii_lowercase()))
            }
            // Before the bare `Char` arm, or every Alt key arrives as a plain
            // character and types itself into whatever is being edited.
            KeyCode::Char(c) if event.modifiers.contains(KeyModifiers::ALT) => {
                Some(Self::Alt(c.to_ascii_lowercase()))
            }
            KeyCode::Char(c) => Some(Self::Char(c.to_ascii_lowercase())),
            KeyCode::Enter => Some(Self::Enter),
            KeyCode::Esc => Some(Self::Esc),
            KeyCode::Backspace => Some(Self::Backspace),
            KeyCode::Tab => Some(Self::Tab),
            KeyCode::BackTab => Some(Self::BackTab),
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
    /// Flip the primary sort rule between ascending and descending.
    ToggleTaskSortDirection,
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
    FilterSetNext,
    FilterSetPrev,
    FilterSetAdd,
    FilterSetRemove,
    FilterRequireEmpty,
    FilterNegateField,
    FilterNegateSet,
    /// Show or hide the named-filter-set sidebar.
    FilterSetsToggle,
    /// Load the saved entry at this position in the sidebar's visible window.
    ///
    /// The position rather than nine variants: `from_command` parses the
    /// suffix and `Display` writes it back, and a `u8` payload costs nothing
    /// on an enum that already derives `Clone, Debug, PartialEq, Eq, Hash`.
    FilterSetLoad(u8),
    /// Show the previous window of saved entries.
    FilterSetsPageBack,
    /// Show the next window of saved entries.
    FilterSetsPageForward,
    /// Save the panel under a name.
    FilterSetSave,
    /// Keep what is on screen as an unnamed copy, stop being the named entry.
    ///
    /// A detach, named for what it is *for*: taking a saved filter as a
    /// starting point and going somewhere else with it.
    FilterSetCopyToNew,
    /// Throw the panel away and start from nothing.
    FilterSetNew,
    /// Delete the loaded entry.
    FilterSetDelete,
    /// Move the calendar's highlighted day back one day.
    CalendarPrevDay,
    /// Move the calendar's highlighted day forward one day.
    CalendarNextDay,
    /// Flip the calendar back one month.
    CalendarPrevMonth,
    /// Flip the calendar forward one month.
    CalendarNextMonth,
    /// Jump the calendar's highlight back to today.
    CalendarToday,
    /// Write the picked date into the field and close the calendar.
    CalendarCommit,
    /// Close the calendar, keeping the text as edited.
    CalendarClose,
    /// Move the edit caret one character left, in any filter field.
    FilterCaretLeft,
    /// Move the edit caret one character right, in any filter field.
    FilterCaretRight,
    /// Put the caret on a range's start date. Does nothing without a range.
    CalendarJumpToStart,
    /// Put the caret on a range's end date. Does nothing without a range.
    CalendarJumpToEnd,
    /// Clear the field the calendar is editing, then close it.
    CalendarClear,
    /// Show or hide the calendar's month grid.
    ///
    /// Hiding it also disables the keys that drive it, which is the point:
    /// `t` and `h` are `today` and "back one day" while the grid is up, and
    /// they are also the first two letters of `thursday`.
    CalendarToggleGrid,
    ToggleTaskSelection,
    SelectAllVisibleTasks,
    InvertTaskSelection,
    ClearTaskSelection,
    ClearHiddenTaskSelection,
    CopyTasksToClipboard,
    /// Draw the Gantt chart and take its controls.
    SetGanttMode,
    /// Hide the Gantt chart and return to task mode.
    ToggleGantt,
    /// Show one more table column beside the chart.
    GanttAddColumn,
    /// Show one fewer table column beside the chart.
    GanttRemoveColumn,
    /// Colour the bars by the next available dimension.
    CycleGanttColorKey,
    /// Move the timeline window back.
    GanttScrollLeft,
    /// Move the timeline window forward.
    GanttScrollRight,
    /// Show a shorter span of time.
    GanttZoomIn,
    /// Show a longer span of time.
    GanttZoomOut,
    /// Return the timeline to fitting the loaded tasks.
    GanttZoomFit,
    /// Centre the timeline on today.
    GanttToday,
    /// Open the colour order dialog.
    GanttOpenOrder,
    /// Move the selected value one place up the colour order.
    GanttOrderMoveUp,
    /// Move the selected value one place down the colour order.
    GanttOrderMoveDown,
    /// Move the selected value to the front of the colour order.
    GanttOrderMoveTop,
    /// Move the selected value to the back of the colour order.
    GanttOrderMoveBottom,
    /// Keep the edited order, persist it, and close the dialog.
    GanttOrderCommit,
    /// Restore the order the dialog opened with and close it.
    GanttOrderCancel,
    /// Move the column cursor one column left.
    TaskColumnPrev,
    /// Move the column cursor one column right.
    TaskColumnNext,
    /// Open the editor for the cell under the column cursor.
    BeginTaskEdit,
    /// Set every target to the opposite of the cursor row's completion.
    ToggleTaskCompleted,
    /// Send the open cell edit.
    CommitTaskEdit,
    /// Throw the open cell edit away.
    CancelTaskEdit,
    /// Step a value picker, by `1` or `-1`.
    ///
    /// The direction is a payload rather than two variants, following
    /// [`Action::FilterSetLoad`].
    TaskEditCycleValue(i32),
    /// Empty the cell being edited.
    TaskEditClear,
    /// Show or hide the recently-edited pane.
    ToggleRecentPane,
    /// Complete the typed prefix to the next candidate, or the previous one.
    ///
    /// The direction is a payload, following [`Action::TaskEditCycleValue`].
    /// One action for both panes: the cell editor and the filter panel's
    /// `list` row run the same completion state machine.
    CompleteCandidate(i32),
    /// Move the text caret back one word.
    TextCaretWordBack,
    /// Move the text caret forward one word.
    TextCaretWordForward,
    /// Move the text caret to the start of the line.
    TextCaretStart,
    /// Move the text caret to the end of the line.
    TextCaretEnd,
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
            "toggle_task_sort_direction" => Ok(Self::ToggleTaskSortDirection),
            "filter_done_editing" => Ok(Self::FilterDoneEditing),
            "filter_cancel_editing" => Ok(Self::FilterCancelEditing),
            "filter_move_label_left" => Ok(Self::FilterMoveLabelLeft),
            "filter_move_label_right" => Ok(Self::FilterMoveLabelRight),
            "filter_cycle_label_up" => Ok(Self::FilterCycleLabelUp),
            "filter_cycle_label_down" => Ok(Self::FilterCycleLabelDown),
            "filter_add_label" => Ok(Self::FilterAddLabel),
            "filter_delete_label" => Ok(Self::FilterDeleteLabel),
            "filter_set_next" => Ok(Self::FilterSetNext),
            "filter_set_prev" => Ok(Self::FilterSetPrev),
            "filter_set_add" => Ok(Self::FilterSetAdd),
            "filter_set_remove" => Ok(Self::FilterSetRemove),
            "filter_require_empty" => Ok(Self::FilterRequireEmpty),
            "filter_negate_field" => Ok(Self::FilterNegateField),
            "filter_negate_set" => Ok(Self::FilterNegateSet),
            "filter_sets_toggle" => Ok(Self::FilterSetsToggle),
            "filter_sets_page_back" => Ok(Self::FilterSetsPageBack),
            "filter_sets_page_forward" => Ok(Self::FilterSetsPageForward),
            "filter_set_save" => Ok(Self::FilterSetSave),
            "filter_set_copy_to_new" => Ok(Self::FilterSetCopyToNew),
            "filter_set_new" => Ok(Self::FilterSetNew),
            "filter_set_delete" => Ok(Self::FilterSetDelete),
            "calendar_prev_day" => Ok(Self::CalendarPrevDay),
            "calendar_next_day" => Ok(Self::CalendarNextDay),
            "calendar_prev_month" => Ok(Self::CalendarPrevMonth),
            "calendar_next_month" => Ok(Self::CalendarNextMonth),
            "calendar_today" => Ok(Self::CalendarToday),
            "calendar_commit" => Ok(Self::CalendarCommit),
            "calendar_close" => Ok(Self::CalendarClose),
            "filter_caret_left" => Ok(Self::FilterCaretLeft),
            "filter_caret_right" => Ok(Self::FilterCaretRight),
            "calendar_jump_to_start" => Ok(Self::CalendarJumpToStart),
            "calendar_jump_to_end" => Ok(Self::CalendarJumpToEnd),
            "calendar_clear" => Ok(Self::CalendarClear),
            "calendar_toggle_grid" => Ok(Self::CalendarToggleGrid),
            "toggle_task_selection" => Ok(Self::ToggleTaskSelection),
            "select_all_visible_tasks" => Ok(Self::SelectAllVisibleTasks),
            "invert_task_selection" => Ok(Self::InvertTaskSelection),
            "clear_task_selection" => Ok(Self::ClearTaskSelection),
            "clear_hidden_task_selection" => Ok(Self::ClearHiddenTaskSelection),
            "copy_tasks_to_clipboard" => Ok(Self::CopyTasksToClipboard),
            "set_gantt_mode" => Ok(Self::SetGanttMode),
            "toggle_gantt" => Ok(Self::ToggleGantt),
            "gantt_add_column" => Ok(Self::GanttAddColumn),
            "gantt_remove_column" => Ok(Self::GanttRemoveColumn),
            "cycle_gantt_color_key" => Ok(Self::CycleGanttColorKey),
            "gantt_scroll_left" => Ok(Self::GanttScrollLeft),
            "gantt_scroll_right" => Ok(Self::GanttScrollRight),
            "gantt_zoom_in" => Ok(Self::GanttZoomIn),
            "gantt_zoom_out" => Ok(Self::GanttZoomOut),
            "gantt_zoom_fit" => Ok(Self::GanttZoomFit),
            "gantt_today" => Ok(Self::GanttToday),
            "gantt_open_order" => Ok(Self::GanttOpenOrder),
            "gantt_order_move_up" => Ok(Self::GanttOrderMoveUp),
            "gantt_order_move_down" => Ok(Self::GanttOrderMoveDown),
            "gantt_order_move_top" => Ok(Self::GanttOrderMoveTop),
            "gantt_order_move_bottom" => Ok(Self::GanttOrderMoveBottom),
            "gantt_order_commit" => Ok(Self::GanttOrderCommit),
            "gantt_order_cancel" => Ok(Self::GanttOrderCancel),
            "task_column_prev" => Ok(Self::TaskColumnPrev),
            "task_column_next" => Ok(Self::TaskColumnNext),
            "begin_task_edit" => Ok(Self::BeginTaskEdit),
            "toggle_task_completed" => Ok(Self::ToggleTaskCompleted),
            "commit_task_edit" => Ok(Self::CommitTaskEdit),
            "cancel_task_edit" => Ok(Self::CancelTaskEdit),
            "task_edit_next_value" => Ok(Self::TaskEditCycleValue(1)),
            "task_edit_prev_value" => Ok(Self::TaskEditCycleValue(-1)),
            "task_edit_clear" => Ok(Self::TaskEditClear),
            "toggle_recent_pane" => Ok(Self::ToggleRecentPane),
            "complete_next_candidate" => Ok(Self::CompleteCandidate(1)),
            "complete_prev_candidate" => Ok(Self::CompleteCandidate(-1)),
            "text_caret_word_back" => Ok(Self::TextCaretWordBack),
            "text_caret_word_forward" => Ok(Self::TextCaretWordForward),
            "text_caret_start" => Ok(Self::TextCaretStart),
            "text_caret_end" => Ok(Self::TextCaretEnd),
            // The one piece of string handling in the action layer: nine
            // load commands would otherwise be nine variants that differ only
            // by a number.
            other => other
                .strip_prefix("filter_set_load_")
                .and_then(|position| position.parse::<u8>().ok())
                .filter(|position| (1..=9).contains(position))
                .map(Self::FilterSetLoad)
                .ok_or_else(|| Error::Backend(format!("unsupported command: {other}"))),
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
                | Action::ToggleTaskSortDirection
        )
    }

    /// Returns `true` for the value-picker keys used in task-edit mode.
    ///
    /// Context-sensitive in the same way the label keys are: they step a
    /// picker on a State or enum cell, and type their character on any cell
    /// that holds text. They must be routed to `handle_task_edit_input`
    /// rather than straight to `handle_action`.
    pub fn is_task_edit_value_action(&self) -> bool {
        matches!(
            self,
            Action::TaskEditCycleValue(_) | Action::TaskEditClear
        )
    }

    /// Returns `true` for the keys that drive the calendar's month grid.
    ///
    /// Every one of them is a plain letter by default, and every one of those
    /// letters appears in a day name. With the grid hidden they are routed to
    /// the text instead, which is what lets `tue` be typed.
    pub fn is_calendar_grid_action(&self) -> bool {
        matches!(
            self,
            Action::CalendarPrevDay
                | Action::CalendarNextDay
                | Action::CalendarPrevMonth
                | Action::CalendarNextMonth
                | Action::CalendarToday
                | Action::CalendarClear
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
        // The one variant with a payload, so the one that cannot be a
        // borrowed name.
        if let Action::FilterSetLoad(position) = self {
            return write!(f, "filter_set_load_{position}");
        }
        if let Action::CompleteCandidate(delta) = self {
            return f.write_str(match *delta >= 0 {
                true => "complete_next_candidate",
                false => "complete_prev_candidate",
            });
        }
        if let Action::TaskEditCycleValue(delta) = self {
            return f.write_str(match *delta >= 0 {
                true => "task_edit_next_value",
                false => "task_edit_prev_value",
            });
        }

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
            Action::ToggleTaskSortDirection => "toggle_task_sort_direction",
            Action::FilterDoneEditing => "filter_done_editing",
            Action::FilterCancelEditing => "filter_cancel_editing",
            Action::FilterMoveLabelLeft => "filter_move_label_left",
            Action::FilterMoveLabelRight => "filter_move_label_right",
            Action::FilterCycleLabelUp => "filter_cycle_label_up",
            Action::FilterCycleLabelDown => "filter_cycle_label_down",
            Action::FilterAddLabel => "filter_add_label",
            Action::FilterDeleteLabel => "filter_delete_label",
            Action::FilterSetNext => "filter_set_next",
            Action::FilterSetPrev => "filter_set_prev",
            Action::FilterSetAdd => "filter_set_add",
            Action::FilterSetRemove => "filter_set_remove",
            Action::FilterRequireEmpty => "filter_require_empty",
            Action::FilterNegateField => "filter_negate_field",
            Action::FilterNegateSet => "filter_negate_set",
            Action::FilterSetsToggle => "filter_sets_toggle",
            Action::FilterSetsPageBack => "filter_sets_page_back",
            Action::FilterSetsPageForward => "filter_sets_page_forward",
            Action::FilterSetSave => "filter_set_save",
            Action::FilterSetCopyToNew => "filter_set_copy_to_new",
            Action::FilterSetNew => "filter_set_new",
            Action::FilterSetDelete => "filter_set_delete",
            // Handled above: it carries a position rather than a fixed name.
            Action::FilterSetLoad(_)
            | Action::TaskEditCycleValue(_)
            | Action::CompleteCandidate(_) => {
                unreachable!("handled before the match")
            }
            Action::CalendarPrevDay => "calendar_prev_day",
            Action::CalendarNextDay => "calendar_next_day",
            Action::CalendarPrevMonth => "calendar_prev_month",
            Action::CalendarNextMonth => "calendar_next_month",
            Action::CalendarToday => "calendar_today",
            Action::CalendarCommit => "calendar_commit",
            Action::CalendarClose => "calendar_close",
            Action::FilterCaretLeft => "filter_caret_left",
            Action::FilterCaretRight => "filter_caret_right",
            Action::CalendarJumpToStart => "calendar_jump_to_start",
            Action::CalendarJumpToEnd => "calendar_jump_to_end",
            Action::CalendarClear => "calendar_clear",
            Action::CalendarToggleGrid => "calendar_toggle_grid",
            Action::ToggleTaskSelection => "toggle_task_selection",
            Action::SelectAllVisibleTasks => "select_all_visible_tasks",
            Action::InvertTaskSelection => "invert_task_selection",
            Action::ClearTaskSelection => "clear_task_selection",
            Action::ClearHiddenTaskSelection => "clear_hidden_task_selection",
            Action::CopyTasksToClipboard => "copy_tasks_to_clipboard",
            Action::SetGanttMode => "set_gantt_mode",
            Action::ToggleGantt => "toggle_gantt",
            Action::GanttAddColumn => "gantt_add_column",
            Action::GanttRemoveColumn => "gantt_remove_column",
            Action::CycleGanttColorKey => "cycle_gantt_color_key",
            Action::GanttScrollLeft => "gantt_scroll_left",
            Action::GanttScrollRight => "gantt_scroll_right",
            Action::GanttZoomIn => "gantt_zoom_in",
            Action::GanttZoomOut => "gantt_zoom_out",
            Action::GanttZoomFit => "gantt_zoom_fit",
            Action::GanttToday => "gantt_today",
            Action::GanttOpenOrder => "gantt_open_order",
            Action::GanttOrderMoveUp => "gantt_order_move_up",
            Action::GanttOrderMoveDown => "gantt_order_move_down",
            Action::GanttOrderMoveTop => "gantt_order_move_top",
            Action::GanttOrderMoveBottom => "gantt_order_move_bottom",
            Action::GanttOrderCommit => "gantt_order_commit",
            Action::GanttOrderCancel => "gantt_order_cancel",
            Action::TaskColumnPrev => "task_column_prev",
            Action::TaskColumnNext => "task_column_next",
            Action::BeginTaskEdit => "begin_task_edit",
            Action::ToggleTaskCompleted => "toggle_task_completed",
            Action::CommitTaskEdit => "commit_task_edit",
            Action::CancelTaskEdit => "cancel_task_edit",
            Action::TaskEditClear => "task_edit_clear",
            Action::ToggleRecentPane => "toggle_recent_pane",
            Action::TextCaretWordBack => "text_caret_word_back",
            Action::TextCaretWordForward => "text_caret_word_forward",
            Action::TextCaretStart => "text_caret_start",
            Action::TextCaretEnd => "text_caret_end",
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

    /// Every command name has to be added in three places — the `Action` enum,
    /// `from_command`, and `Display`. This catches the edit that was done in
    /// two of them, which otherwise breaks the config round-trip silently.
    #[test]
    fn every_filter_set_command_round_trips_through_its_name() {
        for action in [
            Action::FilterSetNext,
            Action::FilterSetPrev,
            Action::FilterSetAdd,
            Action::FilterSetRemove,
            Action::FilterRequireEmpty,
            Action::FilterNegateField,
            Action::FilterNegateSet,
        ] {
            assert_eq!(
                Action::from_command(&action.to_string()).expect("parses"),
                action
            );
        }
    }

    /// The parsed suffix is the one piece of string handling in the action
    /// layer, so it gets a round trip of its own.
    #[test]
    fn every_named_set_command_round_trips_through_its_name() {
        let mut actions = vec![
            Action::FilterSetsToggle,
            Action::FilterSetsPageBack,
            Action::FilterSetsPageForward,
            Action::FilterSetSave,
            Action::FilterSetCopyToNew,
            Action::FilterSetNew,
            Action::FilterSetDelete,
        ];
        actions.extend((1..=9u8).map(Action::FilterSetLoad));

        for action in actions {
            assert_eq!(
                Action::from_command(&action.to_string()).expect("parses"),
                action,
                "{action} did not round trip"
            );
        }

        assert_eq!(
            Action::FilterSetLoad(4).to_string(),
            "filter_set_load_4"
        );
        // Out of range and unparseable suffixes are errors, not silent
        // fallbacks to position zero.
        assert!(Action::from_command("filter_set_load_0").is_err());
        assert!(Action::from_command("filter_set_load_10").is_err());
        assert!(Action::from_command("filter_set_load_x").is_err());
        assert!(Action::from_command("filter_set_load_").is_err());
    }

    /// The twelve commands this milestone added, through the same three
    /// places every other one has to be added in.
    #[test]
    fn every_task_edit_command_round_trips_through_its_name() {
        for action in [
            Action::TaskColumnPrev,
            Action::TaskColumnNext,
            Action::BeginTaskEdit,
            Action::ToggleTaskCompleted,
            Action::CommitTaskEdit,
            Action::CancelTaskEdit,
            Action::TaskEditCycleValue(1),
            Action::TaskEditCycleValue(-1),
            Action::TaskEditClear,
            Action::ToggleRecentPane,
            Action::CompleteCandidate(1),
            Action::CompleteCandidate(-1),
            Action::TextCaretWordBack,
            Action::TextCaretWordForward,
            Action::TextCaretStart,
            Action::TextCaretEnd,
        ] {
            assert_eq!(
                Action::from_command(&action.to_string()).expect("parses"),
                action,
                "{action} did not round trip"
            );
        }

        // The direction is a payload, so it needs two names rather than one.
        assert_eq!(
            Action::TaskEditCycleValue(1).to_string(),
            "task_edit_next_value"
        );
        assert_eq!(
            Action::TaskEditCycleValue(-1).to_string(),
            "task_edit_prev_value"
        );
        assert_eq!(
            Action::CompleteCandidate(1).to_string(),
            "complete_next_candidate"
        );
    }

    #[test]
    fn alt_keys_parse_and_arrive_as_a_modifier() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        assert_eq!("alt-b".parse::<KeyBinding>().expect("parses"), KeyBinding::Alt('b'));
        assert_eq!("ALT-F".parse::<KeyBinding>().expect("parses"), KeyBinding::Alt('f'));
        assert!("alt-".parse::<KeyBinding>().is_err());

        // Before the bare `Char` arm, or every Alt key would arrive as a
        // plain character and type itself into whatever is being edited.
        assert_eq!(
            KeyBinding::from_crossterm_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT)),
            Some(KeyBinding::Alt('b'))
        );
        // Ctrl still wins, so `ctrl-alt-b` is not silently an Alt binding.
        assert_eq!(
            KeyBinding::from_crossterm_event(KeyEvent::new(
                KeyCode::Char('b'),
                KeyModifiers::ALT | KeyModifiers::CONTROL
            )),
            Some(KeyBinding::Ctrl('b'))
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
        assert_eq!("tab".parse::<KeyBinding>().expect("tab parses"), KeyBinding::Tab);
        assert_eq!(
            "shift-tab".parse::<KeyBinding>().expect("shift-tab parses"),
            KeyBinding::BackTab
        );
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
        assert_eq!(
            Action::from_command("filter_set_add").expect("filter_set_add parses"),
            Action::FilterSetAdd
        );
        assert_eq!(
            Action::from_command("filter_require_empty").expect("filter_require_empty parses"),
            Action::FilterRequireEmpty
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
