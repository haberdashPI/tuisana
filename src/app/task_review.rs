use std::{
    collections::HashMap,
    time::Instant,
};

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

const HORIZONTAL_SCROLL_STEP: usize = 8;

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
    OutOfDate(String),
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
    horizontal_scroll: usize,
    loading_started_at: Option<Instant>,
    loading_targets: Vec<String>,
    loading_target_ids: Vec<String>,
    loaded_target_ids: Vec<String>,
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
        if self.visible && self.selected.is_none() {
            self.selected = self.table.first_selectable_row_index();
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

    pub fn begin_loading(&mut self, projects: &[Project]) {
        self.status = TaskReviewStatus::Loading;
        self.selected = None;
        self.table = TaskTableModel::empty();
        self.horizontal_scroll = 0;
        self.loading_started_at = Some(Instant::now());
        self.loading_targets = projects.iter().map(|project| project.name.clone()).collect();
        self.loading_target_ids = projects.iter().map(|project| project.id.clone()).collect();
        self.loaded_target_ids.clear();
    }

    pub fn finish_loading(&mut self, table: TaskTableModel) {
        self.table = table;
        self.horizontal_scroll = 0;
        self.selected = self.table.first_selectable_row_index();
        self.loaded_target_ids = self.loading_target_ids.clone();
        self.status = if self.table.task_count() == 0 {
            TaskReviewStatus::Empty
        } else {
            TaskReviewStatus::Ready
        };
        self.loading_started_at = None;
        self.loading_targets.clear();
        self.loading_target_ids.clear();
    }

    pub fn mark_out_of_date(&mut self, message: impl Into<String>) {
        if matches!(self.status, TaskReviewStatus::Idle) {
            return;
        }

        self.status = TaskReviewStatus::OutOfDate(message.into());
        self.loading_started_at = None;
        self.loading_targets.clear();
        self.loading_target_ids.clear();
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.status = TaskReviewStatus::Error(message.into());
        self.selected = None;
        self.table = TaskTableModel::empty();
        self.horizontal_scroll = 0;
        self.loading_started_at = None;
        self.loading_targets.clear();
        self.loading_target_ids.clear();
        self.loaded_target_ids.clear();
    }

    pub fn loading_started_at(&self) -> Option<Instant> {
        self.loading_started_at
    }

    pub fn loading_targets(&self) -> &[String] {
        &self.loading_targets
    }

    pub fn loaded_target_ids(&self) -> &[String] {
        &self.loaded_target_ids
    }

    pub fn loading_spinner(&self) -> &'static str {
        const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
        let elapsed = self
            .loading_started_at
            .map(|started| started.elapsed().as_millis())
            .unwrap_or_default();
        let index = ((elapsed / 120) as usize) % FRAMES.len();
        FRAMES[index]
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

    pub fn selected_task_position(&self) -> Option<usize> {
        self.selected
            .and_then(|index| self.table.selectable_position(index))
    }

    pub fn horizontal_scroll(&self) -> usize {
        self.horizontal_scroll
    }

    pub fn scroll_left(&mut self) {
        self.horizontal_scroll = self.horizontal_scroll.saturating_sub(HORIZONTAL_SCROLL_STEP);
    }

    pub fn scroll_right(&mut self) {
        self.horizontal_scroll = self.horizontal_scroll.saturating_add(HORIZONTAL_SCROLL_STEP);
    }

    pub fn load_for_projects<C: AsanaClient>(
        &mut self,
        client: &C,
        projects: &[Project],
    ) -> Result<()> {
        debug_log(&format!("task load start: project_count={}", projects.len()));
        let table = Self::build_table_for_projects(client, projects)?;
        self.finish_loading(table);
        debug_log(&format!(
            "task load complete: tasks={} columns={}",
            self.table.task_count(),
            self.table.columns.len()
        ));
        Ok(())
    }

    pub fn build_table_for_projects<C: AsanaClient>(
        client: &C,
        projects: &[Project],
    ) -> Result<TaskTableModel> {
        if projects.is_empty() {
            return Ok(TaskTableModel::empty());
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
                add_task_record(&mut records, &project.name, &section_map, task);
            }
        }

        let mut definitions = definitions_by_gid
            .into_iter()
            .map(|(gid, name)| CustomFieldDefinition::new(gid, name))
            .collect::<Vec<_>>();
        definitions.sort_by(|left, right| left.name.cmp(&right.name).then_with(|| left.gid.cmp(&right.gid)));

        Ok(TaskTableModel::from_records(records, definitions))
    }

    pub fn move_up(&mut self) {
        if let Some(index) = self.selected {
            if let Some(previous) = self.table.previous_selectable_row_index(index, 1) {
                self.selected = Some(previous);
            }
        }
    }

    pub fn move_down(&mut self) {
        if let Some(index) = self.selected {
            if let Some(next) = self.table.next_selectable_row_index(index, 1) {
                self.selected = Some(next);
            }
        } else {
            self.selected = self.table.first_selectable_row_index();
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        let step = page_size.max(1);
        if let Some(index) = self.selected {
            if let Some(previous) = self.table.previous_selectable_row_index(index, step) {
                self.selected = Some(previous);
            }
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        let step = page_size.max(1);
        if let Some(index) = self.selected {
            if let Some(next) = self.table.next_selectable_row_index(index, step) {
                self.selected = Some(next);
            }
        } else {
            self.selected = self.table.first_selectable_row_index();
        }
    }

    pub fn jump_top(&mut self) {
        self.selected = self.table.first_selectable_row_index();
    }

    pub fn jump_bottom(&mut self) {
        self.selected = self.table.last_selectable_row_index();
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
            Action::ScrollLeft => {
                self.scroll_left();
                None
            }
            Action::ScrollRight => {
                self.scroll_right();
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
        assert_eq!(state.table().task_count(), 2);
        assert_eq!(state.table().rows.len(), 4);
        assert_eq!(state.table().rows[0].kind, crate::domain::TaskRowKind::ProjectHeader);
        assert_eq!(state.table().rows[0].cells[0], "Backlog");
        assert_eq!(state.table().rows[1].kind, crate::domain::TaskRowKind::Task);
        assert_eq!(state.table().rows[1].cells[0], "Ship release");
        assert_eq!(state.table().rows[1].cells[1], "Later | Today");
        assert_eq!(state.table().rows[1].cells[2], "Alex");
        assert_eq!(state.table().rows[1].cells[3], "2026-06-01");
        assert_eq!(state.table().rows[1].cells[4], "2026-05-28");
        assert_eq!(state.table().rows[1].cells[5], "open");
        assert_eq!(state.table().rows[1].cells[6], "Backlog | Inbox");
        assert_eq!(state.table().rows[2].kind, crate::domain::TaskRowKind::ProjectHeader);
        assert_eq!(state.table().rows[2].cells[0], "Inbox");
        assert_eq!(state.table().rows[3].kind, crate::domain::TaskRowKind::Task);
        assert_eq!(state.table().rows[3].cells[0], "Write docs");
    }

    #[test]
    fn switching_to_task_mode_makes_the_view_visible() {
        let mut state = TaskReviewState::new();

        state.toggle_focus_mode();

        assert_eq!(state.focus_mode(), TaskFocusMode::Tasks);
        assert!(state.visible());
    }
}
