use std::collections::HashMap;

use crate::{
    app::debug_log,
    asana::{
        dto::{CustomFieldValueDto, TaskDto},
        AsanaClient,
    },
    domain::{CustomFieldDefinition, Project, TaskRecord, TaskTableModel},
    error::Result,
    input::Action,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskFocusMode {
    #[default]
    Projects,
    Tasks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskReviewStatus {
    Idle,
    Loading,
    Ready,
    Empty,
    Error(String),
}

impl Default for TaskReviewStatus {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskReviewState {
    visible: bool,
    focus_mode: TaskFocusMode,
    status: TaskReviewStatus,
    selected: Option<usize>,
    table: TaskTableModel,
}

impl TaskReviewState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if self.visible && self.selected.is_none() && !self.table.rows.is_empty() {
            self.selected = Some(0);
        }
    }

    pub fn toggle_visible(&mut self) {
        self.set_visible(!self.visible);
    }

    pub fn focus_mode(&self) -> TaskFocusMode {
        self.focus_mode
    }

    pub fn toggle_focus_mode(&mut self) {
        self.focus_mode = match self.focus_mode {
            TaskFocusMode::Projects => TaskFocusMode::Tasks,
            TaskFocusMode::Tasks => TaskFocusMode::Projects,
        };
        if self.focus_mode == TaskFocusMode::Tasks {
            self.visible = true;
        }
    }

    pub fn status(&self) -> &TaskReviewStatus {
        &self.status
    }

    pub fn table(&self) -> &TaskTableModel {
        &self.table
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn load_for_projects<C: AsanaClient>(
        &mut self,
        client: &C,
        projects: &[Project],
    ) -> Result<()> {
        debug_log(&format!(
            "task load start: project_count={}",
            projects.len()
        ));
        self.status = TaskReviewStatus::Loading;

        if projects.is_empty() {
            self.table = TaskTableModel::empty();
            self.selected = None;
            self.status = TaskReviewStatus::Empty;
            return Ok(());
        }

        let mut records = Vec::new();
        let mut definitions_by_gid: HashMap<String, String> = HashMap::new();

        for project in projects {
            let sections = client.list_sections(&project.id)?;
            debug_log(&format!(
                "task load project={} sections={}",
                project.id,
                sections.len()
            ));
            let section_map = sections
                .into_iter()
                .map(|section| (section.gid, section.name))
                .collect::<HashMap<_, _>>();

            for setting in client.list_project_custom_field_settings(&project.id)? {
                debug_log(&format!(
                    "task load project={} custom_field={}",
                    project.id, setting.custom_field.name
                ));
                definitions_by_gid
                    .entry(setting.custom_field.gid.clone())
                    .or_insert(setting.custom_field.name.clone());
            }

            let tasks = client.list_tasks(&project.id)?;
            debug_log(&format!(
                "task load project={} tasks={}",
                project.id,
                tasks.len()
            ));
            for task in tasks {
                add_task_record(
                    &mut records,
                    &project.name,
                    &section_map,
                    task,
                );
            }
        }

        let definitions = definitions_by_gid
            .into_iter()
            .map(|(gid, name)| CustomFieldDefinition::new(gid, name))
            .collect::<Vec<_>>();

        let mut definitions = definitions;
        definitions.sort_by(|left, right| left.name.cmp(&right.name).then_with(|| left.gid.cmp(&right.gid)));

        self.table = TaskTableModel::from_records(records, definitions);
        self.selected = if self.table.rows.is_empty() {
            None
        } else {
            Some(0)
        };
        self.status = if self.table.rows.is_empty() {
            TaskReviewStatus::Empty
        } else {
            TaskReviewStatus::Ready
        };
        debug_log(&format!(
            "task load complete: rows={} columns={}",
            self.table.rows.len(),
            self.table.columns.len()
        ));
        Ok(())
    }

    pub fn move_up(&mut self) {
        match self.selected {
            Some(0) | None => {}
            Some(index) => self.selected = Some(index - 1),
        }
    }

    pub fn move_down(&mut self) {
        if let Some(index) = self.selected {
            if index + 1 < self.table.rows.len() {
                self.selected = Some(index + 1);
            }
        } else if !self.table.rows.is_empty() {
            self.selected = Some(0);
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        let step = page_size.max(1);
        match self.selected {
            Some(0) | None => {}
            Some(index) => self.selected = Some(index.saturating_sub(step)),
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        let step = page_size.max(1);
        if let Some(index) = self.selected {
            if index + 1 < self.table.rows.len() {
                self.selected = Some((index + step).min(self.table.rows.len() - 1));
            }
        } else if !self.table.rows.is_empty() {
            self.selected = Some(0);
        }
    }

    pub fn jump_top(&mut self) {
        self.selected = if self.table.rows.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    pub fn jump_bottom(&mut self) {
        self.selected = if self.table.rows.is_empty() {
            None
        } else {
            Some(self.table.rows.len() - 1)
        };
    }

    pub fn apply_action(&mut self, action: &Action, page_size: usize) -> Option<crate::input::AppCommand> {
        match action {
            Action::MoveUp => {
                self.move_up();
                None
            }
            Action::MoveDown => {
                self.move_down();
                None
            }
            Action::PageUp => {
                self.page_up(page_size);
                None
            }
            Action::PageDown => {
                self.page_down(page_size);
                None
            }
            Action::JumpTop => {
                self.jump_top();
                None
            }
            Action::JumpBottom => {
                self.jump_bottom();
                None
            }
            _ => None,
        }
    }
}

fn add_task_record(
    records: &mut Vec<TaskRecord>,
    project_name: &str,
    section_map: &HashMap<String, String>,
    task: TaskDto,
) {
    let mut record = TaskRecord::new(task.gid, task.name);
    record.completed = task.completed;
    if let Some(assignee) = task.assignee {
        record.assignee = assignee.display_name.or(assignee.name).or(Some(assignee.gid));
    }
    record.due_date = task.due_on;
    record.start_date = task.start_on;
    record.projects.push(project_name.to_string());

    for membership in task.memberships {
        if let Some(section) = membership.section {
            record
                .sections
                .push(section_map.get(&section.gid).cloned().unwrap_or(section.name));
        }
    }

    for field in task.custom_fields {
        if let Some(value) = custom_field_value(&field) {
            record
                .custom_fields
                .entry(field.gid)
                .or_default()
                .push(value);
        }
    }

    records.push(record);
}

fn custom_field_value(field: &CustomFieldValueDto) -> Option<String> {
    field
        .display_value
        .clone()
        .or_else(|| field.enum_value.as_ref().map(|value| value.name.clone()))
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use crate::{
        asana::{
            dto::{
                CustomFieldDto, CustomFieldValueDto, EnumOptionDto, ProjectCustomFieldSettingDto,
                SectionDto, TaskDto, TaskMembershipDto, TaskMembershipProjectDto,
                TaskMembershipSectionDto, UserDto,
            },
            fake::FakeAsanaClient,
        },
        domain::Project,
    };

    use super::{TaskFocusMode, TaskReviewState, TaskReviewStatus};

    fn task(
        gid: &str,
        name: &str,
        project_gid: &str,
        project_name: &str,
        section_gid: &str,
        section_name: &str,
        field_gid: &str,
        field_name: &str,
        field_value: &str,
    ) -> TaskDto {
        TaskDto {
            gid: gid.to_string(),
            name: name.to_string(),
            completed: false,
            due_on: Some("2026-06-01".to_string()),
            start_on: Some("2026-05-28".to_string()),
            assignee: Some(UserDto {
                gid: "user-1".to_string(),
                name: Some("Alex".to_string()),
                display_name: Some("Alex".to_string()),
            }),
            memberships: vec![TaskMembershipDto {
                project: TaskMembershipProjectDto {
                    gid: project_gid.to_string(),
                    name: project_name.to_string(),
                },
                section: Some(TaskMembershipSectionDto {
                    gid: section_gid.to_string(),
                    name: section_name.to_string(),
                }),
            }],
            custom_fields: vec![CustomFieldValueDto {
                gid: field_gid.to_string(),
                name: field_name.to_string(),
                display_value: Some(field_value.to_string()),
                enum_value: Some(EnumOptionDto {
                    gid: "opt-1".to_string(),
                    name: field_value.to_string(),
                }),
            }],
        }
    }

    #[test]
    fn loads_tasks_for_selected_projects_and_builds_a_table_model() {
        let client = FakeAsanaClient::new(vec![
            Project::new("p1", "Inbox", true),
            Project::new("p2", "Backlog", false),
        ])
        .with_sections(
            "p1",
            vec![SectionDto {
                gid: "s1".to_string(),
                name: "Today".to_string(),
            }],
        )
        .with_custom_field_settings(
            "p1",
            vec![ProjectCustomFieldSettingDto {
                gid: "cfs1".to_string(),
                custom_field: CustomFieldDto {
                    gid: "cf1".to_string(),
                    name: "Priority".to_string(),
                },
            }],
        )
        .with_tasks(
            "p1",
            vec![
                task("t1", "Ship release", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                task("t2", "Write docs", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "Low"),
            ],
        )
        .with_sections(
            "p2",
            vec![SectionDto {
                gid: "s2".to_string(),
                name: "Later".to_string(),
            }],
        )
        .with_custom_field_settings(
            "p2",
            vec![ProjectCustomFieldSettingDto {
                gid: "cfs2".to_string(),
                custom_field: CustomFieldDto {
                    gid: "cf2".to_string(),
                    name: "Effort".to_string(),
                },
            }],
        )
        .with_tasks(
            "p2",
            vec![task("t1", "Ship release", "p2", "Backlog", "s2", "Later", "cf2", "Effort", "S")],
        );

        let mut state = TaskReviewState::new();

        state
            .load_for_projects(
                &client,
                &[Project::new("p1", "Inbox", true), Project::new("p2", "Backlog", false)],
            )
            .expect("tasks load");

        assert_eq!(state.status(), &TaskReviewStatus::Ready);
        assert_eq!(state.focus_mode(), TaskFocusMode::Projects);
        assert_eq!(state.table().columns[0], "Task");
        assert!(state.table().columns.iter().any(|column| column == "Priority"));
        assert!(state.table().columns.iter().any(|column| column == "Effort"));
        assert_eq!(state.table().rows.len(), 2);
        assert_eq!(state.table().rows[0].cells[0], "Ship release");
        assert_eq!(state.table().rows[0].cells[1], "Today | Later");
        assert_eq!(state.table().rows[0].cells[2], "Alex");
        assert_eq!(state.table().rows[0].cells[3], "2026-06-01");
        assert_eq!(state.table().rows[0].cells[4], "2026-05-28");
        assert_eq!(state.table().rows[0].cells[5], "open");
        assert_eq!(state.table().rows[0].cells[6], "Inbox | Backlog");
    }

    #[test]
    fn switching_to_task_mode_makes_the_view_visible() {
        let mut state = TaskReviewState::new();

        state.toggle_focus_mode();

        assert_eq!(state.focus_mode(), TaskFocusMode::Tasks);
        assert!(state.visible());
    }
}
