use crate::app::task_review::{TaskFocusMode, TaskReviewState, TaskReviewStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskTableView {
    pub title: String,
    pub status_line: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn render_task_table(state: &TaskReviewState) -> TaskTableView {
    let status_line = match state.status() {
        TaskReviewStatus::Idle => "Tasks idle".to_string(),
        TaskReviewStatus::Loading => "Loading tasks...".to_string(),
        TaskReviewStatus::Ready => task_status(state, "ready"),
        TaskReviewStatus::Empty => "No tasks available".to_string(),
        TaskReviewStatus::Error(message) => format!("Error: {message}"),
    };

    TaskTableView {
        title: match state.focus_mode() {
            TaskFocusMode::Projects => "Task review (project focus)".to_string(),
            TaskFocusMode::Tasks => "Task review (task focus)".to_string(),
        },
        status_line,
        columns: state.table().columns.clone(),
        rows: state
            .table()
            .rows
            .iter()
            .map(|row| row.cells.clone())
            .collect(),
    }
}

fn task_status(state: &TaskReviewState, suffix: &str) -> String {
    let visible = if state.visible() { "visible" } else { "hidden" };
    match state.selected_index() {
        Some(index) => format!("{} tasks, row {}, {}, {}", state.table().rows.len(), index + 1, visible, suffix),
        None => format!("{} tasks, {}, {}", state.table().rows.len(), visible, suffix),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::task_review::TaskReviewState,
        asana::{
            dto::{
                CustomFieldDto, CustomFieldValueDto, ProjectCustomFieldSettingDto, SectionDto,
                TaskDto,
            },
            fake::FakeAsanaClient,
        },
        domain::Project,
    };

    use super::render_task_table;

    #[test]
    fn renders_task_table_columns_and_rows() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
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
                vec![TaskDto {
                    gid: "t1".to_string(),
                    name: "Ship".to_string(),
                    completed: false,
                    due_on: Some("2026-06-10".to_string()),
                    start_on: Some("2026-06-01".to_string()),
                    assignee: None,
                    memberships: vec![],
                    custom_fields: vec![CustomFieldValueDto {
                        gid: "cf1".to_string(),
                        name: "Priority".to_string(),
                        display_value: Some("High".to_string()),
                        enum_value: None,
                    }],
                }],
            );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.set_visible(true);

        let view = render_task_table(&state);

        assert_eq!(view.title, "Task review (project focus)");
        assert!(view.status_line.contains("1 tasks"));
        assert_eq!(view.columns[0], "Task");
        assert_eq!(view.rows[0][0], "Ship");
        assert_eq!(view.rows[0][3], "2026-06-10");
    }
}
