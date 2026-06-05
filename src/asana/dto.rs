use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct CollectionResponse<T> {
    pub data: Vec<T>,
    #[serde(default)]
    pub next_page: Option<Page>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Page {
    pub offset: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectDto {
    pub gid: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SectionDto {
    pub gid: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserDto {
    pub gid: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskMembershipProjectDto {
    pub gid: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskMembershipSectionDto {
    pub gid: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskMembershipDto {
    pub project: TaskMembershipProjectDto,
    #[serde(default)]
    pub section: Option<TaskMembershipSectionDto>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EnumOptionDto {
    pub gid: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomFieldValueDto {
    pub gid: String,
    pub name: String,
    #[serde(default)]
    pub display_value: Option<String>,
    #[serde(default)]
    pub enum_value: Option<EnumOptionDto>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskDto {
    pub gid: String,
    pub name: String,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub due_on: Option<String>,
    #[serde(default)]
    pub start_on: Option<String>,
    #[serde(default)]
    pub assignee: Option<UserDto>,
    #[serde(default)]
    pub memberships: Vec<TaskMembershipDto>,
    #[serde(default)]
    pub custom_fields: Vec<CustomFieldValueDto>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomFieldDto {
    pub gid: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectCustomFieldSettingDto {
    pub gid: String,
    pub custom_field: CustomFieldDto,
}
