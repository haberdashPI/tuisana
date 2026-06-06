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
    domain::{
        CustomFieldDefinition, Project, TaskRecord, TaskTableModel, TaskTableSettings,
    },
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
    settings: TaskTableSettings,
    dataset: Option<TaskDataset>,
    horizontal_scroll: usize,
    loading_started_at: Option<Instant>,
    loading_targets: Vec<String>,
    loading_target_ids: Vec<String>,
    loaded_target_ids: Vec<String>,
    task_vertical_scroll: usize,
    help_details_visible: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TaskDataset {
    records: Vec<TaskRecord>,
    custom_field_definitions: Vec<CustomFieldDefinition>,
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

    pub fn help_details_visible(&self) -> bool {
        self.help_details_visible
    }

    pub fn toggle_help_details(&mut self) {
        self.help_details_visible = !self.help_details_visible;
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
        self.dataset = None;
        self.horizontal_scroll = 0;
        self.loading_started_at = Some(Instant::now());
        self.loading_targets = projects.iter().map(|project| project.name.clone()).collect();
        self.loading_target_ids = projects.iter().map(|project| project.id.clone()).collect();
        self.loaded_target_ids.clear();
        self.task_vertical_scroll = 0;
    }

    pub fn finish_loading(&mut self, table: TaskTableModel) {
        self.dataset = None;
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
        self.task_vertical_scroll = 0;
    }

    pub(crate) fn finish_loading_dataset(&mut self, dataset: TaskDataset) {
        self.dataset = Some(dataset);
        self.refresh_table();
        self.horizontal_scroll = 0;
        self.selected = self.table.first_selectable_row_index();
        self.loaded_target_ids = self.loading_target_ids.clone();
        self.loading_started_at = None;
        self.loading_targets.clear();
        self.loading_target_ids.clear();
        self.task_vertical_scroll = 0;
    }

    pub fn task_settings(&self) -> &TaskTableSettings {
        &self.settings
    }

    pub fn filter_summary(&self) -> String {
        self.settings.summary()
    }

    pub fn mark_out_of_date(&mut self, message: impl Into<String>) {
        if matches!(self.status, TaskReviewStatus::Idle) {
            return;
        }

        self.status = TaskReviewStatus::OutOfDate(message.into());
        self.loading_started_at = None;
        self.loading_targets.clear();
        self.loading_target_ids.clear();
        self.task_vertical_scroll = 0;
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
        self.task_vertical_scroll = 0;
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

    pub fn vertical_scroll(&self) -> usize {
        self.task_vertical_scroll
    }

    pub fn ensure_selected_visible(&mut self, viewport_height: usize) {
        let Some(selected_index) = self.selected else {
            self.task_vertical_scroll = 0;
            return;
        };

        let content_height = viewport_height.max(1);
        let selected_line = selected_index.saturating_add(1);
        let margin = 2usize.min(content_height.saturating_sub(1));

        let min_visible = self.task_vertical_scroll.saturating_add(margin);
        let max_visible = self
            .task_vertical_scroll
            .saturating_add(content_height.saturating_sub(1))
            .saturating_sub(margin);

        if selected_line < min_visible {
            self.task_vertical_scroll = selected_line.saturating_sub(margin);
            return;
        }

        if selected_line > max_visible {
            self.task_vertical_scroll = selected_line
                .saturating_add(margin)
                .saturating_add(1)
                .saturating_sub(content_height);
        }
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
        let dataset = Self::build_dataset_for_projects(client, projects)?;
        self.dataset = Some(dataset);
        self.refresh_table();
        debug_log(&format!(
            "task load complete: tasks={} columns={}",
            self.table.task_count(),
            self.table.columns.len()
        ));
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn build_table_for_projects<C: AsanaClient>(
        client: &C,
        projects: &[Project],
    ) -> Result<TaskTableModel> {
        let dataset = Self::build_dataset_for_projects(client, projects)?;
        Ok(TaskTableModel::from_records_with_settings(
            dataset.records,
            dataset.custom_field_definitions,
            &TaskTableSettings::default(),
        ))
    }

    pub(crate) fn build_dataset_for_projects<C: AsanaClient>(
        client: &C,
        projects: &[Project],
    ) -> Result<TaskDataset> {
        if projects.is_empty() {
            return Ok(TaskDataset::default());
        }

        let mut records = Vec::new();
        let mut definitions_by_gid: HashMap<String, String> = HashMap::new();
        let mut natural_order = 0usize;

        for project in projects {
            let sections = client.list_sections(&project.id)?;
            debug_log(&format!(
                "task load project={} sections={}",
                project.id,
                sections.len()
            ));
            let section_map = sections
                .iter()
                .map(|section| (section.gid.clone(), section.name.clone()))
                .collect::<HashMap<_, _>>();
            let section_order_map = sections
                .into_iter()
                .enumerate()
                .map(|(index, section)| (section.gid, index))
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
                add_task_tree(
                    client,
                    &project.name,
                    &section_map,
                    &section_order_map,
                    None,
                    None,
                    None,
                    0,
                    &mut natural_order,
                    task,
                    &mut records,
                )?;
            }
        }

        let mut definitions = definitions_by_gid
            .into_iter()
            .map(|(gid, name)| CustomFieldDefinition::new(gid, name))
            .collect::<Vec<_>>();
        definitions.sort_by(|left, right| left.name.cmp(&right.name).then_with(|| left.gid.cmp(&right.gid)));

        Ok(TaskDataset {
            records,
            custom_field_definitions: definitions,
        })
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

    pub fn move_section_up(&mut self) {
        if let Some(index) = self.selected {
            if let Some(previous) = self.table.previous_section_row_index(index, 1) {
                self.selected = Some(previous);
            }
        }
    }

    pub fn move_section_down(&mut self) {
        if let Some(index) = self.selected {
            if let Some(next) = self.table.next_section_row_index(index, 1) {
                self.selected = Some(next);
            }
        } else {
            self.selected = self.table.first_selectable_row_index();
        }
    }

    pub fn move_project_up(&mut self) {
        if let Some(parent_index) = self.visible_parent_index() {
            self.selected = Some(parent_index);
            return;
        }

        if let Some(index) = self.selected {
            if let Some(previous) = self.table.previous_project_row_index(index, 1) {
                self.selected = Some(previous);
            }
        }
    }

    pub fn move_project_down(&mut self) {
        if let Some(parent_index) = self.visible_parent_index() {
            self.selected = Some(parent_index);
            return;
        }

        if let Some(index) = self.selected {
            if let Some(next) = self.table.next_project_row_index(index, 1) {
                self.selected = Some(next);
            }
        } else {
            self.selected = self.table.first_selectable_row_index();
        }
    }

    pub fn toggle_completed_filter(&mut self) {
        self.settings.filter.toggle_completed_filter();
        self.refresh_table();
    }

    pub fn toggle_subtask_visibility(&mut self) {
        self.settings.filter.toggle_subtask_visibility();
        self.refresh_table();
    }

    pub fn cycle_sort_field(&mut self) {
        self.settings.sort.cycle_primary_field();
        self.refresh_table();
    }

    pub fn toggle_project_grouping(&mut self) {
        self.settings.sort.toggle_project_grouping();
        self.refresh_table();
    }

    pub fn toggle_section_grouping(&mut self) {
        self.settings.sort.toggle_section_grouping();
        self.refresh_table();
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
            Action::MoveSectionUp => {
                self.move_section_up();
                None
            }
            Action::MoveSectionDown => {
                self.move_section_down();
                None
            }
            Action::MoveProjectUp => {
                self.move_project_up();
                None
            }
            Action::MoveProjectDown => {
                self.move_project_down();
                None
            }
            Action::ToggleCompletedFilter => {
                self.toggle_completed_filter();
                None
            }
            Action::ToggleHelpDetails => {
                self.toggle_help_details();
                None
            }
            Action::ToggleSubtaskVisibility => {
                self.toggle_subtask_visibility();
                None
            }
            Action::CycleTaskSort => {
                self.cycle_sort_field();
                None
            }
            Action::ToggleProjectGrouping => {
                self.toggle_project_grouping();
                None
            }
            Action::ToggleSectionGrouping => {
                self.toggle_section_grouping();
                None
            }
            _ => None,
        }
    }

    fn refresh_table(&mut self) {
        let Some(dataset) = self.dataset.as_ref() else {
            return;
        };

        let previous_selected_index = self.selected;
        let selected_gid = self
            .selected
            .and_then(|index| self.table.rows.get(index))
            .map(|row| row.gid.clone());
        let selected_parent_gid = selected_gid.as_deref().and_then(|gid| {
            dataset
                .records
                .iter()
                .find(|record| record.gid == gid)
                .and_then(|record| record.parent_gid.clone())
        });

        self.table = TaskTableModel::from_records_with_settings(
            dataset.records.clone(),
            dataset.custom_field_definitions.clone(),
            &self.settings,
        );

        self.selected = selected_gid
            .as_deref()
            .and_then(|gid| self.table.rows.iter().position(|row| row.gid == gid));

        if self.selected.is_none() {
            self.selected = selected_parent_gid
                .as_deref()
                .and_then(|gid| self.table.rows.iter().position(|row| row.gid == gid));
        }

        if self.selected.is_none() {
            self.selected = previous_selected_index.and_then(|previous_index| {
                self.table
                    .selectable_row_indices()
                    .into_iter()
                    .rev()
                    .find(|index| *index <= previous_index)
                    .or_else(|| self.table.last_selectable_row_index())
            });
        }

        if self.selected.is_none() {
            self.selected = self.table.first_selectable_row_index();
        }

        if !matches!(
            self.status,
            TaskReviewStatus::OutOfDate(_) | TaskReviewStatus::Error(_)
        ) {
            self.status = if self.table.task_count() == 0 {
                TaskReviewStatus::Empty
            } else {
                TaskReviewStatus::Ready
            };
        }
    }

    fn visible_parent_index(&self) -> Option<usize> {
        let selected_gid = self
            .selected
            .and_then(|index| self.table.rows.get(index))
            .map(|row| row.gid.clone())?;

        let parent_gid = self
            .dataset
            .as_ref()?
            .records
            .iter()
            .find(|record| record.gid == selected_gid)?
            .parent_gid
            .clone()?;

        self.table.rows.iter().position(|row| row.gid == parent_gid)
    }
}

fn add_task_tree<C: AsanaClient>(
    client: &C,
    project_name: &str,
    section_map: &HashMap<String, String>,
    section_order_map: &HashMap<String, usize>,
    inherited_section: Option<String>,
    inherited_section_order: Option<usize>,
    parent_gid: Option<String>,
    depth: usize,
    natural_order: &mut usize,
    task: TaskDto,
    records: &mut Vec<TaskRecord>,
) -> Result<()> {
    let current_gid = task.gid.clone();
    let mut record = TaskRecord::new(task.gid, task.name);
    record.completed = task.completed;
    record.parent_gid = parent_gid;
    record.subtask_depth = depth;
    if let Some(assignee) = task.assignee {
        record.assignee = assignee.display_name.or(assignee.name).or(Some(assignee.gid));
    }
    record.due_date = task.due_on;
    record.start_date = task.start_on;
    record.natural_order = *natural_order;
    *natural_order = (*natural_order).saturating_add(1);
    record.projects.push(project_name.to_string());

    let mut section_name = inherited_section;
    let mut section_order = inherited_section_order;
    if section_name.is_none() || section_order.is_none() {
        for membership in task.memberships {
            if let Some(section) = membership.section {
                if section_name.is_none() {
                    section_name = Some(
                        section_map
                            .get(&section.gid)
                            .cloned()
                            .unwrap_or(section.name.clone()),
                    );
                }
                if section_order.is_none() {
                    section_order = section_order_map.get(&section.gid).copied();
                }
                if section_name.is_some() && section_order.is_some() {
                    break;
                }
            }
        }
    }

    if let Some(section) = section_name.clone() {
        record.sections.push(section);
    }
    record.section_order = section_order;

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

    if task.num_subtasks > 0 {
        let subtasks = client.list_subtasks(&current_gid)?;
        for subtask in subtasks {
            add_task_tree(
                client,
                project_name,
                section_map,
                section_order_map,
                section_name.clone(),
                section_order,
                Some(current_gid.clone()),
                depth + 1,
                natural_order,
                subtask,
                records,
            )?;
        }
    }

    Ok(())
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
            num_subtasks: 0,
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
        assert_eq!(state.table().rows.len(), 10);
        assert_eq!(state.table().rows[0].kind, crate::domain::TaskRowKind::ProjectSeparator);
        assert_eq!(state.table().rows[1].kind, crate::domain::TaskRowKind::ProjectHeader);
        assert_eq!(state.table().rows[1].cells[0], "Backlog");
        assert_eq!(state.table().rows[2].kind, crate::domain::TaskRowKind::SectionSpacer);
        assert_eq!(state.table().rows[3].kind, crate::domain::TaskRowKind::SectionHeader);
        assert_eq!(state.table().rows[3].cells[0], "Later");
        assert_eq!(state.table().rows[4].kind, crate::domain::TaskRowKind::Task);
        assert_eq!(state.table().rows[4].cells[0], "Ship release");
        assert_eq!(state.table().rows[4].cells[1], "Alex");
        assert_eq!(state.table().rows[4].cells[2], "2026-06-01");
        assert_eq!(state.table().rows[4].cells[3], "2026-05-28");
        assert_eq!(state.table().rows[4].cells[4], "open");
        assert_eq!(state.table().rows[4].cells[5], "Backlog | Inbox");
        assert_eq!(state.table().rows[5].kind, crate::domain::TaskRowKind::ProjectSeparator);
        assert_eq!(state.table().rows[6].kind, crate::domain::TaskRowKind::ProjectHeader);
        assert_eq!(state.table().rows[6].cells[0], "Inbox");
        assert_eq!(state.table().rows[7].kind, crate::domain::TaskRowKind::SectionSpacer);
        assert_eq!(state.table().rows[8].kind, crate::domain::TaskRowKind::SectionHeader);
        assert_eq!(state.table().rows[8].cells[0], "Today");
        assert_eq!(state.table().rows[9].kind, crate::domain::TaskRowKind::Task);
        assert_eq!(state.table().rows[9].cells[0], "Write docs");
    }

    #[test]
    fn switching_to_task_mode_makes_the_view_visible() {
        let mut state = TaskReviewState::new();

        state.toggle_focus_mode();

        assert_eq!(state.focus_mode(), TaskFocusMode::Tasks);
        assert!(state.visible());
    }

    #[test]
    fn moves_by_section_and_project_groups() {
        let client = FakeAsanaClient::new(vec![
            Project::new("p1", "Alpha", true),
            Project::new("p2", "Beta", false),
        ])
        .with_sections(
            "p1",
            vec![
                SectionDto {
                        gid: "s1".to_string(),
                        name: "Today".to_string(),
                    },
                    SectionDto {
                        gid: "s2".to_string(),
                        name: "Later".to_string(),
                    },
                ],
            )
            .with_sections(
                "p2",
                vec![SectionDto {
                    gid: "s3".to_string(),
                    name: "Now".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![
                    task("t1", "Alpha 1", "p1", "Alpha", "s1", "Today", "cf1", "Priority", "High"),
                    task("t2", "Alpha 2", "p1", "Alpha", "s1", "Today", "cf1", "Priority", "Low"),
                    task("t3", "Alpha 3", "p1", "Alpha", "s2", "Later", "cf1", "Priority", "Low"),
                ],
            );
        let client = client.with_tasks(
            "p2",
            vec![task("t4", "Beta 1", "p2", "Beta", "s3", "Now", "cf1", "Priority", "Medium")],
        );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(
                &client,
                &[Project::new("p1", "Alpha", true), Project::new("p2", "Beta", false)],
            )
            .expect("tasks load");

        assert_eq!(state.selected_index(), Some(4));
        state.move_section_down();
        assert_eq!(state.selected_index(), Some(8));
        state.move_section_up();
        assert_eq!(state.selected_index(), Some(4));
        state.move_project_down();
        assert_eq!(state.selected_index(), Some(13));
        state.move_project_up();
        assert_eq!(state.selected_index(), Some(4));
    }

    #[test]
    fn keeps_selected_row_in_view_when_the_viewport_is_small() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![
                    task("t1", "Task 1", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    task("t2", "Task 2", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    task("t3", "Task 3", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    task("t4", "Task 4", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                ],
            );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.set_visible(true);

        state.move_down();
        state.move_down();
        state.move_down();

        state.ensure_selected_visible(3);

        assert_eq!(state.selected_index(), Some(7));
        assert!(state.vertical_scroll() > 0);
        let selected_line = state.selected_index().unwrap() + 1;
        assert!(selected_line >= state.vertical_scroll());
        assert!(selected_line < state.vertical_scroll() + 3);
    }

    #[test]
    fn toggles_task_filters_and_rebuilds_the_table_after_async_style_load() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![
                    task("t1", "Open task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    TaskDto {
                        completed: true,
                        ..task("t2", "Closed task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High")
                    },
                ],
            );

        let mut state = TaskReviewState::new();
        let dataset = TaskReviewState::build_dataset_for_projects(
            &client,
            &[Project::new("p1", "Inbox", true)],
        )
        .expect("dataset builds");
        state.begin_loading(&[Project::new("p1", "Inbox", true)]);
        state.finish_loading_dataset(dataset);

        assert_eq!(state.table().task_count(), 2);
        assert!(state.filter_summary().contains("all"));

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 1);
        assert!(state.filter_summary().contains("open"));

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 1);
        assert!(state.filter_summary().contains("comp done"));

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 2);

        state.toggle_subtask_visibility();
        assert!(state.filter_summary().contains("sub hide"));

        state.toggle_project_grouping();
        state.toggle_section_grouping();
        assert!(state.filter_summary().contains("grp p:off"));
        assert!(state.filter_summary().contains("grp p:off s:off"));

        state.cycle_sort_field();
        assert!(state.filter_summary().contains("sort title"));
    }

    #[test]
    fn loads_subtasks_for_project_tasks_even_when_they_are_not_project_members() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![task("t1", "Parent task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High")],
            )
            .with_subtasks(
                "t1",
                vec![TaskDto {
                    gid: "t2".to_string(),
                    name: "Child task".to_string(),
                    completed: false,
                    due_on: Some("2026-06-01".to_string()),
                    start_on: Some("2026-05-28".to_string()),
                    assignee: Some(UserDto {
                        gid: "user-1".to_string(),
                        name: Some("Alex".to_string()),
                        display_name: Some("Alex".to_string()),
                    }),
                    num_subtasks: 0,
                    memberships: vec![],
                    custom_fields: vec![CustomFieldValueDto {
                        gid: "cf1".to_string(),
                        name: "Priority".to_string(),
                        display_value: Some("Low".to_string()),
                        enum_value: Some(EnumOptionDto {
                            gid: "opt-1".to_string(),
                            name: "Low".to_string(),
                        }),
                    }],
                }],
            );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        let task_rows = state
            .table()
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.cells[0].clone())
            .collect::<Vec<_>>();

        assert_eq!(task_rows, vec!["Parent task", "  L Child task"]);
        assert_eq!(state.table().task_count(), 2);
    }

    #[test]
    fn sorts_sections_using_the_order_returned_by_asana() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![
                    SectionDto {
                        gid: "s2".to_string(),
                        name: "Beta".to_string(),
                    },
                    SectionDto {
                        gid: "s1".to_string(),
                        name: "Alpha".to_string(),
                    },
                ],
            )
            .with_tasks(
                "p1",
                vec![
                    task("t1", "Beta task", "p1", "Inbox", "s2", "Beta", "cf1", "Priority", "High"),
                    task("t2", "Alpha task", "p1", "Inbox", "s1", "Alpha", "cf1", "Priority", "High"),
                ],
            );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        let task_rows = state
            .table()
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| (row.cells[0].clone(), row.section.clone().unwrap_or_default()))
            .collect::<Vec<_>>();

        assert_eq!(
            task_rows,
            vec![
                ("Beta task".to_string(), "Beta".to_string()),
                ("Alpha task".to_string(), "Alpha".to_string()),
            ]
        );
    }

    #[test]
    fn hiding_subtasks_keeps_selection_near_the_previous_row_instead_of_jumping_to_the_top() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![
                    SectionDto {
                        gid: "s1".to_string(),
                        name: "Today".to_string(),
                    },
                    SectionDto {
                        gid: "s2".to_string(),
                        name: "Later".to_string(),
                    },
                ],
            )
            .with_tasks(
                "p1",
                vec![
                    {
                        let mut parent = task("t1", "Parent task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High");
                        parent.num_subtasks = 1;
                        parent
                    },
                    task("t3", "Later task", "p1", "Inbox", "s2", "Later", "cf1", "Priority", "High"),
                ],
            )
            .with_subtasks(
                "t1",
                vec![{
                    let mut child = task("t2", "Child task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High");
                    child.memberships.clear();
                    child
                }],
            );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        assert_eq!(state.selected_index(), Some(4));
        state.move_down();
        assert_eq!(state.selected_index(), Some(5));

        state.toggle_subtask_visibility();

        assert_eq!(state.selected_index(), Some(4));

        state.move_section_down();
        assert_eq!(state.selected_index(), Some(7));
    }

    #[test]
    fn collapsing_subtasks_selects_the_super_task_when_the_current_row_is_a_subtask() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![{
                    let mut parent = task("t1", "Parent task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High");
                    parent.num_subtasks = 1;
                    parent
                }],
            )
            .with_subtasks(
                "t1",
                vec![{
                    let mut child = task("t2", "Child task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "Low");
                    child.memberships.clear();
                    child
                }],
            );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        assert_eq!(state.selected_index(), Some(4));
        state.move_down();
        assert_eq!(state.selected_index(), Some(5));

        state.toggle_subtask_visibility();

        assert_eq!(state.selected_index(), Some(4));
        assert_eq!(state.table().rows[state.selected_index().unwrap()].gid, "t1");
    }

    #[test]
    fn section_navigation_does_not_jump_backwards_from_a_last_section_subtask() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![
                    SectionDto {
                        gid: "s1".to_string(),
                        name: "Today".to_string(),
                    },
                    SectionDto {
                        gid: "s2".to_string(),
                        name: "Later".to_string(),
                    },
                ],
            )
            .with_tasks(
                "p1",
                vec![{
                    let mut parent = task("t1", "Parent task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High");
                    parent.num_subtasks = 1;
                    parent
                }],
            )
            .with_subtasks(
                "t1",
                vec![{
                    let mut child = task("t2", "Child task", "p1", "Inbox", "s2", "Later", "cf1", "Priority", "Low");
                    child.memberships.clear();
                    child
                }],
            );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        state.move_down();
        assert_eq!(state.selected_index(), Some(5));

        state.move_section_down();

        assert_eq!(state.selected_index(), Some(5));
        assert_eq!(state.table().rows[state.selected_index().unwrap()].gid, "t2");
    }

    #[test]
    fn project_navigation_prefers_the_parent_row_when_the_current_row_is_a_subtask() {
        let client = FakeAsanaClient::new(vec![
            Project::new("p1", "Inbox", true),
            Project::new("p2", "Later", false),
        ])
        .with_sections(
            "p1",
            vec![SectionDto {
                gid: "s1".to_string(),
                name: "Today".to_string(),
            }],
        )
        .with_tasks(
            "p1",
            vec![{
                let mut parent = task("t1", "Parent task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High");
                parent.num_subtasks = 1;
                parent
            }],
        )
        .with_subtasks(
            "t1",
            vec![{
                let mut child = task("t2", "Child task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "Low");
                child.memberships.clear();
                child
            }],
        )
        .with_tasks(
            "p2",
            vec![task("t3", "Other project task", "p2", "Later", "s1", "Today", "cf1", "Priority", "Medium")],
        );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(
                &client,
                &[Project::new("p1", "Inbox", true), Project::new("p2", "Later", false)],
            )
            .expect("tasks load");

        state.move_down();
        assert_eq!(state.selected_index(), Some(5));

        state.move_project_down();
        assert_eq!(state.selected_index(), Some(4));
        assert_eq!(state.table().rows[state.selected_index().unwrap()].gid, "t1");
    }

    #[test]
    fn refreshes_the_view_after_filter_changes_when_loaded_through_the_async_path() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![
                    task("t1", "Open task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High"),
                    TaskDto {
                        num_subtasks: 0,
                        completed: true,
                        ..task("t2", "Closed task", "p1", "Inbox", "s1", "Today", "cf1", "Priority", "High")
                    },
                ],
            );

        let mut state = TaskReviewState::new();
        state.begin_loading(&[Project::new("p1", "Inbox", true)]);
        let dataset = TaskReviewState::build_dataset_for_projects(
            &client,
            &[Project::new("p1", "Inbox", true)],
        )
        .expect("dataset builds");
        state.finish_loading_dataset(dataset);

        assert_eq!(state.table().task_count(), 2);

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 1);
        assert_eq!(state.table().rows.iter().filter(|row| row.kind.is_task()).count(), 1);

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 1);
        assert_eq!(state.table().rows.iter().filter(|row| row.kind.is_task()).count(), 1);

        state.toggle_completed_filter();
        assert_eq!(state.table().task_count(), 2);
        assert_eq!(state.table().rows.iter().filter(|row| row.kind.is_task()).count(), 2);
    }
}
