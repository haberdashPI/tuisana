//! The mutation model: one field of one task, set to one value.
//!
//! Everything here is about *writing*, which is why it is a module of its own
//! rather than more functions in `task.rs`. Reading merges: two projects'
//! "Priority" fields become one column, a display name stands in for a user.
//! Writing cannot, so the types here keep the gid, the option gid, and the
//! handle Asana will actually accept.

use crate::domain::{date, CivilDate, TaskRecord};

/// One field of one task, set to one value.
///
/// Modelled as a field change rather than a whole-task patch because the UI
/// edits one cell at a time, and because a patch would have to distinguish
/// "leave this alone" from "clear this" on every field.
#[derive(Clone, Debug, PartialEq)]
pub enum TaskFieldEdit {
    Name(String),
    Completed(bool),
    Due(Option<String>),
    Start(Option<String>),
    /// `None` unassigns.
    Assignee(Option<AssigneeRef>),
    CustomField {
        gid: String,
        value: Option<CustomFieldValue>,
    },
}

/// Who a task is being assigned to.
///
/// Asana takes a user gid, an email address, or the literal `me`; it does not
/// take a display name. `display` is what the local record shows until a
/// reload brings the real one, which for an email is the email itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssigneeRef {
    pub handle: String,
    pub display: String,
}

impl AssigneeRef {
    pub fn new(handle: impl Into<String>, display: impl Into<String>) -> Self {
        Self {
            handle: handle.into(),
            display: display.into(),
        }
    }
}

/// A custom field's value, in the shape the API takes it.
#[derive(Clone, Debug, PartialEq)]
pub enum CustomFieldValue {
    Enum { option_gid: String, name: String },
    Text(String),
    Number { value: f64, text: String },
}

impl CustomFieldValue {
    /// What the table cell shows for this value.
    pub fn display(&self) -> String {
        match self {
            Self::Enum { name, .. } => name.clone(),
            Self::Text(text) => text.clone(),
            Self::Number { text, .. } => text.clone(),
        }
    }
}

/// One pending change: what to do, and what to put back if it fails.
///
/// `previous` is the same type as `field`, so a rollback is the same code path
/// as an apply.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskEdit {
    pub gid: String,
    pub field: TaskFieldEdit,
    pub previous: TaskFieldEdit,
}

impl TaskFieldEdit {
    /// Writes this change onto a record.
    ///
    /// Used for the optimistic update *and* for the rollback, and deliberately
    /// not routed through `merge_task_record`: the merge is monotone, so it
    /// would refuse to un-complete a task or to clear a date.
    pub fn apply(&self, record: &mut TaskRecord) {
        match self {
            Self::Name(name) => record.name = name.clone(),
            Self::Completed(completed) => record.completed = *completed,
            Self::Due(value) => record.due_date = value.clone(),
            Self::Start(value) => record.start_date = value.clone(),
            Self::Assignee(assignee) => match assignee {
                Some(assignee) => {
                    record.assignee = Some(assignee.display.clone());
                    record.assignee_gid = Some(assignee.handle.clone());
                }
                None => {
                    record.assignee = None;
                    record.assignee_gid = None;
                }
            },
            Self::CustomField { gid, value } => match value {
                Some(value) => {
                    record
                        .custom_fields
                        .insert(gid.clone(), vec![value.display()]);
                }
                None => {
                    // Emptied rather than removed: the column is built from
                    // the keys every record carries, and a task that just lost
                    // its only value still belongs in the column.
                    record.custom_fields.insert(gid.clone(), Vec::new());
                }
            },
        }
    }

    /// The change that puts a record back the way it is now.
    pub fn undo_for(&self, record: &TaskRecord) -> Self {
        match self {
            Self::Name(_) => Self::Name(record.name.clone()),
            Self::Completed(_) => Self::Completed(record.completed),
            Self::Due(_) => Self::Due(record.due_date.clone()),
            Self::Start(_) => Self::Start(record.start_date.clone()),
            Self::Assignee(_) => Self::Assignee(record.assignee.clone().map(|display| {
                AssigneeRef::new(
                    record.assignee_gid.clone().unwrap_or_else(|| display.clone()),
                    display,
                )
            })),
            Self::CustomField { gid, .. } => Self::CustomField {
                gid: gid.clone(),
                value: record
                    .custom_fields
                    .get(gid)
                    .and_then(|values| values.first())
                    .map(|value| CustomFieldValue::Text(value.clone())),
            },
        }
    }
}

/// Parses the text of a task date cell.
///
/// Empty clears the date. A range is refused rather than half-applied: a task
/// has one due date, and `2026-09-01..2026-09-08` is a sensible *filter* and a
/// nonsensical due date. Everything else goes through the same grammar the
/// filter panel's date fields use, normalized to `YYYY-MM-DD`.
pub fn parse_date_value(text: &str, today: CivilDate) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.contains("..") {
        return Err("a task date is one day, not a range".to_string());
    }

    match date::parse_token(text, today) {
        Some(Some(date)) => Ok(Some(date.iso())),
        _ => Err(format!("{text} is not a date")),
    }
}

/// Resolves the text of an assignee cell against the people already loaded.
///
/// `me` is the current user, anything with an `@` is sent verbatim as an email
/// (Asana accepts one in place of a gid), and anything else is matched
/// case-insensitively against the names the loaded records carry. A display
/// name is the one thing Asana will *not* take, so a name that matches nothing
/// is a refusal rather than a guess.
pub fn resolve_assignee(
    text: &str,
    directory: &[(String, String)],
    me: Option<&str>,
) -> Result<Option<AssigneeRef>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }

    if text.eq_ignore_ascii_case("me") {
        let gid = me.ok_or_else(|| "no current user is known".to_string())?;
        let display = directory
            .iter()
            .find(|(candidate, _)| candidate == gid)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| "me".to_string());
        return Ok(Some(AssigneeRef::new(gid, display)));
    }

    if text.contains('@') {
        return Ok(Some(AssigneeRef::new(text, text)));
    }

    let matches = directory
        .iter()
        .filter(|(_, name)| name.eq_ignore_ascii_case(text))
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [] => Err(format!("no one called {text} is loaded")),
        [(gid, name)] => Ok(Some(AssigneeRef::new(gid.clone(), name.clone()))),
        _ => Err(format!("{text} is ambiguous")),
    }
}

/// What a custom field holds, as far as parsing a typed value is concerned.
///
/// A thin mirror of `CustomFieldKind` so this module can stay free of the
/// table's field definitions.
#[derive(Clone, Debug, PartialEq)]
pub enum CustomValueKind<'a> {
    Text,
    Number,
    /// `(option gid, option name)`, as the field declares them.
    Enum(&'a [(String, String)]),
    /// A kind this milestone does not write.
    Unsupported,
}

/// Parses the text of a custom-field cell into the value Asana takes.
pub fn parse_custom_value(
    kind: CustomValueKind<'_>,
    text: &str,
) -> Result<Option<CustomFieldValue>, String> {
    let text = text.trim();

    match kind {
        CustomValueKind::Unsupported => {
            Err("this field cannot be edited here yet".to_string())
        }
        _ if text.is_empty() => Ok(None),
        CustomValueKind::Text => Ok(Some(CustomFieldValue::Text(text.to_string()))),
        CustomValueKind::Number => match text.parse::<f64>() {
            Ok(value) => Ok(Some(CustomFieldValue::Number {
                value,
                text: text.to_string(),
            })),
            Err(_) => Err(format!("{text} is not a number")),
        },
        CustomValueKind::Enum(options) => options
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(text))
            .map(|(gid, name)| {
                Some(CustomFieldValue::Enum {
                    option_gid: gid.clone(),
                    name: name.clone(),
                })
            })
            .ok_or_else(|| format!("{text} is not one of this field's options")),
    }
}
