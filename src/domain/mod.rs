//! Core application data models and rules.
//!
//! This layer sits between the Asana client and the app/UI layers. It owns
//! the canonical project and task representations plus the logic that turns
//! raw task records into the table rows the UI renders.

pub mod date;
mod gantt;
mod project;
mod task;
mod task_edit;

/// Calendar dates, the filter date grammar, and the local-timezone `today`.
pub use date::{
    month_name, today, CivilDate, DateQuery, PartialDate, CALENDAR_WEEKDAYS, MONTHS, WEEKDAYS,
};
/// Gantt chart colour assignment and timeline arithmetic.
pub use gantt::{
    distinct_values, BarSpan, ColorSlot, GanttColorKey, GanttModel, GanttTrack, SlotAssignment,
    Tick, TickScale, Timeline, TimelineView, TrackShape, PALETTE_SLOTS,
};
/// The app's canonical project model.
pub use project::{Project, ProjectKind};
/// Task-domain types and helpers for filtering, sorting, and table building.
pub use task::{
    group_custom_fields_by_name, CustomFieldColumn, CustomFieldDefinition, CustomFieldKind,
    EnumOption, Section, SortDirection,
    SubtaskVisibility, ASSIGNEE_COLUMN, DUE_COLUMN, FIRST_CUSTOM_COLUMN, PROJECTS_COLUMN,
    START_COLUMN, STATE_COLUMN, TITLE_COLUMN,
    TaskFilter, TaskRecord, TaskRow, TaskRowKind, TaskSort,
    TaskSortField, TaskSortRule, TaskTableModel, TaskTableSettings, merge_task_record,
};
/// The write model: one field of one task, and the parsing that builds it.
pub use task_edit::{
    parse_custom_value, parse_date_value, resolve_assignee, AssigneeRef, CustomFieldValue,
    CustomValueKind, TaskEdit, TaskFieldEdit,
};
