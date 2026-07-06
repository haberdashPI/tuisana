//! Core application data models and rules.
//!
//! This layer sits between the Asana client and the app/UI layers. It owns
//! the canonical project and task representations plus the logic that turns
//! raw task records into the table rows the UI renders.

mod project;
mod task;

/// The app's canonical project model.
pub use project::{Project, ProjectKind};
/// Task-domain types and helpers for filtering, sorting, and table building.
pub use task::{
    CustomFieldColumn, CustomFieldDefinition, Section, SortDirection, SubtaskVisibility,
    TaskDateRange, TaskFieldFilter, TaskFilter, TaskRecord, TaskRow, TaskRowKind, TaskSort,
    TaskSortField, TaskSortRule, TaskTableModel, TaskTableSettings, merge_task_record,
};
