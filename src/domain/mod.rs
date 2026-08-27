//! Core application data models and rules.
//!
//! This layer sits between the Asana client and the app/UI layers. It owns
//! the canonical project and task representations plus the logic that turns
//! raw task records into the table rows the UI renders.

pub mod date;
mod project;
mod task;

/// Calendar dates, the filter date grammar, and the local-timezone `today`.
pub use date::{today, CivilDate, DateQuery, PartialDate, WEEKDAYS};
/// The app's canonical project model.
pub use project::{Project, ProjectKind};
/// Task-domain types and helpers for filtering, sorting, and table building.
pub use task::{
    group_custom_fields_by_name, CustomFieldColumn, CustomFieldDefinition, Section, SortDirection,
    SubtaskVisibility,
    TaskDateRange, TaskFieldFilter, TaskFilter, TaskRecord, TaskRow, TaskRowKind, TaskSort,
    TaskSortField, TaskSortRule, TaskTableModel, TaskTableSettings, merge_task_record,
};
