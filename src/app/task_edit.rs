//! The column cursor's companion: the cell edit that `e` opens.
//!
//! Beside `calendar.rs` and `text_edit.rs`: the state an open edit needs, with
//! no UI and no knowledge of the table it sits in. `TaskState` owns one of
//! these at a time and drives it; the renderer reads [`CellEditView`].

use crate::{
    app::{calendar::CalendarState, text_edit::TextEdit},
    domain::CivilDate,
};

/// What resolving an edit needs that `TaskState` does not own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditContext {
    /// Today, for the date keywords the picker accepts.
    ///
    /// `None` reads the clock. Tests pin it instead, because the integration
    /// tests share a process and `TUISANA_TODAY` is global.
    pub today: Option<CivilDate>,
    /// The logged-in user's gid, which is what `me` resolves to.
    pub current_user_gid: Option<String>,
}

impl EditContext {
    pub fn today(&self) -> CivilDate {
        self.today.unwrap_or_else(crate::domain::today)
    }
}

/// The editor a cell opened with.
///
/// One per kind of field, and each is the filter panel's editor for that kind:
/// nothing here is a vocabulary the user has to learn twice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CellEditor {
    Text(TextEdit),
    /// One value out of a fixed set. `None` is "no value".
    Options {
        options: Vec<String>,
        cursor: Option<usize>,
        /// Whether "no value" is a state this field can be in.
        ///
        /// False for State: a task is always either open or done, so an empty
        /// slot in that ring would be a value the commit could not express.
        allow_empty: bool,
    },
    /// The picker, mirrored into the text the same way the filter does it:
    /// the buffer is the value, the overlay only writes into it.
    Date {
        text: TextEdit,
        calendar: CalendarState,
    },
}

/// The cell being edited, and what it will be applied to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskCellEditState {
    /// The column, fixed for the life of the edit.
    pub(crate) column: usize,
    /// The tasks the commit applies to, resolved when the edit opened.
    ///
    /// Resolved once, on purpose: the table rebuilds whenever a project
    /// finishes loading or a filter changes, and an edit that re-read the
    /// selection at commit time could change more than it said it would.
    pub(crate) targets: Vec<String>,
    pub(crate) editor: CellEditor,
    /// Where the visible window into a long value starts, as a char index.
    ///
    /// Kept here rather than recomputed each frame so the window is sticky:
    /// it moves only when the caret would leave it, instead of re-centring
    /// under every keystroke. Written by the renderer, which is the only
    /// thing that knows how wide the column is.
    pub(crate) window_start: usize,
}

impl TaskCellEditState {
    pub(crate) fn new(column: usize, targets: Vec<String>, editor: CellEditor) -> Self {
        Self {
            column,
            targets,
            editor,
            window_start: 0,
        }
    }

    /// The text buffer this editor types into, if it has one.
    pub(crate) fn text_mut(&mut self) -> Option<&mut TextEdit> {
        match &mut self.editor {
            CellEditor::Text(text) => Some(text),
            CellEditor::Date { text, .. } => Some(text),
            CellEditor::Options { .. } => None,
        }
    }

    pub(crate) fn text(&self) -> Option<&TextEdit> {
        match &self.editor {
            CellEditor::Text(text) => Some(text),
            CellEditor::Date { text, .. } => Some(text),
            CellEditor::Options { .. } => None,
        }
    }

    pub(crate) fn calendar(&self) -> Option<&CalendarState> {
        match &self.editor {
            CellEditor::Date { calendar, .. } => Some(calendar),
            _ => None,
        }
    }

    pub(crate) fn calendar_mut(&mut self) -> Option<&mut CalendarState> {
        match &mut self.editor {
            CellEditor::Date { calendar, .. } => Some(calendar),
            _ => None,
        }
    }

    /// Copies the picker's text into the buffer the cell shows.
    ///
    /// The same mirroring the filter panel does, and for the same reason: the
    /// cell is where the value is read, the overlay only writes into it.
    pub(crate) fn sync_calendar(&mut self) {
        let CellEditor::Date { text, calendar } = &mut self.editor else {
            return;
        };
        text.set_text_at_end(calendar.query().to_string());
    }

    /// Steps an options picker, wrapping through "no value" at the end.
    ///
    /// The empty slot is a stop rather than a special key on every field, so
    /// `j` past the last option means "clear this" without leaving the list.
    pub(crate) fn cycle_option(&mut self, delta: i32) {
        let CellEditor::Options {
            options,
            cursor,
            allow_empty,
        } = &mut self.editor
        else {
            return;
        };
        if options.is_empty() {
            return;
        }
        // One slot past the options is "no value", so the ring is one longer
        // than the list — on a field that can hold no value at all.
        let len = options.len() as i32 + i32::from(*allow_empty);
        let current = cursor.map_or(options.len() as i32, |index| index as i32);
        let next = (current + delta).rem_euclid(len);
        *cursor = (next < options.len() as i32).then_some(next as usize);
    }

    /// Puts an options picker on "no value", and clears a text one.
    pub(crate) fn clear_value(&mut self) {
        match &mut self.editor {
            CellEditor::Options {
                cursor, allow_empty, ..
            } => {
                if *allow_empty {
                    *cursor = None;
                }
            }
            CellEditor::Text(text) => text.clear(),
            CellEditor::Date { text, .. } => text.clear(),
        }
    }

    /// The value the editor currently holds, as text.
    pub(crate) fn value(&self) -> String {
        match &self.editor {
            CellEditor::Text(text) => text.text().to_string(),
            CellEditor::Date { text, .. } => text.text().to_string(),
            CellEditor::Options { options, cursor, .. } => cursor
                .and_then(|index| options.get(index))
                .cloned()
                .unwrap_or_default(),
        }
    }
}

impl CellEditor {
    /// Whether this is a value picker, which reads `j`/`k` rather than text.
    pub(crate) fn is_options(&self) -> bool {
        matches!(self, Self::Options { .. })
    }
}

/// What the renderer needs to draw an open editor inside its cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellEditView {
    /// The column the editor is open on.
    pub column: usize,
    /// The text to draw in the cell.
    pub text: String,
    /// Where the caret sits, as a char index, or `None` for a value picker.
    pub caret: Option<usize>,
    /// Where the visible window starts, as a char index into `text`.
    pub window_start: usize,
    /// How many tasks the commit will apply to.
    pub targets: usize,
}

#[cfg(test)]
mod tests {
    use super::{CellEditor, TaskCellEditState};

    fn options() -> TaskCellEditState {
        TaskCellEditState::new(
            6,
            vec!["t1".to_string()],
            CellEditor::Options {
                options: vec!["High".to_string(), "Low".to_string()],
                cursor: Some(0),
                allow_empty: true,
            },
        )
    }

    #[test]
    fn cycling_an_options_picker_passes_through_no_value_once_per_lap() {
        let mut state = options();

        state.cycle_option(1);
        assert_eq!(state.value(), "Low");
        state.cycle_option(1);
        assert_eq!(state.value(), "", "one slot past the last option is empty");
        state.cycle_option(1);
        assert_eq!(state.value(), "High", "and then it wraps");

        state.cycle_option(-1);
        assert_eq!(state.value(), "");
    }

    #[test]
    fn clearing_an_options_picker_selects_no_value() {
        let mut state = options();
        state.clear_value();

        assert_eq!(state.value(), "");
    }
}
