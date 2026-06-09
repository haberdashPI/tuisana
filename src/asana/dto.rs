//! Serde DTOs for the subset of the Asana API used by the app.

use serde::Deserialize;

/// A paginated Asana collection response.
#[derive(Debug, Clone, Deserialize)]
pub struct CollectionResponse<T> {
    /// The current page of data.
    pub data: Vec<T>,
    /// The next page cursor, if any.
    #[serde(default)]
    pub next_page: Option<Page>,
}

/// Pagination cursor for a collection response.
#[derive(Debug, Clone, Deserialize)]
pub struct Page {
    /// The opaque offset used to request the next page.
    pub offset: String,
}

/// Project payload returned by the Asana API.
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectDto {
    /// The project id.
    pub gid: String,
    /// The project name.
    pub name: String,
}

/// Section payload returned by the Asana API.
#[derive(Debug, Clone, Deserialize)]
pub struct SectionDto {
    /// The section id.
    pub gid: String,
    /// The section name.
    pub name: String,
}

/// User payload used for assignee fields.
#[derive(Debug, Clone, Deserialize)]
pub struct UserDto {
    /// The user id.
    pub gid: String,
    /// The user name, if present.
    #[serde(default)]
    pub name: Option<String>,
    /// The user display name, if present.
    #[serde(default)]
    pub display_name: Option<String>,
}

/// The project portion of a task membership.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskMembershipProjectDto {
    /// The project id.
    pub gid: String,
    /// The project name.
    pub name: String,
}

/// The section portion of a task membership.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskMembershipSectionDto {
    /// The section id.
    pub gid: String,
    /// The section name.
    pub name: String,
}

/// A task membership record.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskMembershipDto {
    /// The project associated with the membership.
    pub project: TaskMembershipProjectDto,
    /// The section associated with the membership, if any.
    #[serde(default)]
    pub section: Option<TaskMembershipSectionDto>,
}

/// Enum option payload used by custom fields.
#[derive(Debug, Clone, Deserialize)]
pub struct EnumOptionDto {
    /// The option id.
    pub gid: String,
    /// The option name.
    pub name: String,
}

/// A single task custom-field value.
#[derive(Debug, Clone, Deserialize)]
pub struct CustomFieldValueDto {
    /// The custom-field id.
    pub gid: String,
    /// The custom-field name.
    pub name: String,
    /// The display value returned by Asana, if any.
    #[serde(default)]
    pub display_value: Option<String>,
    /// Enum metadata, if the field is enum-based.
    #[serde(default)]
    pub enum_value: Option<EnumOptionDto>,
}

/// The task payload returned by the Asana API.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskDto {
    /// The task id.
    pub gid: String,
    /// The task title.
    pub name: String,
    /// Whether the task is completed.
    #[serde(default)]
    pub completed: bool,
    /// The last modification timestamp.
    #[serde(default)]
    pub modified_at: Option<String>,
    /// The due date, if present.
    #[serde(default)]
    pub due_on: Option<String>,
    /// The start date, if present.
    #[serde(default)]
    pub start_on: Option<String>,
    /// The assignee, if present.
    #[serde(default)]
    pub assignee: Option<UserDto>,
    /// The number of subtasks.
    #[serde(default)]
    pub num_subtasks: usize,
    /// All membership records for the task.
    #[serde(default)]
    pub memberships: Vec<TaskMembershipDto>,
    /// All custom field values for the task.
    #[serde(default)]
    pub custom_fields: Vec<CustomFieldValueDto>,
}

/// A custom field definition payload for a project.
#[derive(Debug, Clone, Deserialize)]
pub struct CustomFieldDto {
    /// The custom-field id.
    pub gid: String,
    /// The custom-field name.
    pub name: String,
}

/// The project-to-custom-field settings payload.
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectCustomFieldSettingDto {
    /// The setting id.
    pub gid: String,
    /// The custom field attached to the project.
    pub custom_field: CustomFieldDto,
}
