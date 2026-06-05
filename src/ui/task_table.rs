use ratatui::{style::{Modifier, Style}, text::{Line, Span}};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::task_review::{TaskFocusMode, TaskReviewState, TaskReviewStatus},
    domain::{TaskRow, TaskRowKind},
};

const COLUMN_SEPARATOR: &str = " | ";
const COLUMN_SEPARATOR_WIDTH: usize = 3;
const DEFAULT_OTHER_COLUMN_CAP: usize = 18;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskTableView {
    pub title: String,
    pub status_line: String,
    pub scroll_hint_line: String,
    pub columns: Vec<String>,
    pub rows: Vec<TaskRow>,
    pub column_widths: Vec<usize>,
    pub total_width: usize,
    pub scroll_offset: usize,
}

pub fn render_task_table(state: &TaskReviewState, viewport_width: usize) -> TaskTableView {
    let status_line = match state.status() {
        TaskReviewStatus::Idle => "Tasks idle".to_string(),
        TaskReviewStatus::Loading => {
            let targets = if state.loading_targets().is_empty() {
                String::new()
            } else {
                format!(" for {}", state.loading_targets().join(", "))
            };
            format!("Loading tasks{} {}", targets, state.loading_spinner())
        }
        TaskReviewStatus::Ready => task_status(state, &format!("ready, {}", state.filter_summary())),
        TaskReviewStatus::OutOfDate(message) => task_status(
            state,
            &format!("out of date - {message}, {}", state.filter_summary()),
        ),
        TaskReviewStatus::Empty => "No tasks available".to_string(),
        TaskReviewStatus::Error(message) => format!("Error: {message}"),
    };

    let columns = state.table().columns.clone();
    let rows = state.table().rows.clone();
    let task_rows = rows
        .iter()
        .filter(|row| row.kind == TaskRowKind::Task)
        .map(|row| row.cells.clone())
        .collect::<Vec<_>>();
    let mut column_widths = natural_column_widths(&columns, &task_rows);

    for (index, width) in column_widths.iter_mut().enumerate().skip(1) {
        *width = (*width).min(column_cap(index));
    }

    let other_width_total = column_widths
        .iter()
        .skip(1)
        .copied()
        .sum::<usize>()
        + COLUMN_SEPARATOR_WIDTH * column_widths.len().saturating_sub(1);
    let widest_other = column_widths.iter().skip(1).copied().max().unwrap_or(0);
    let title_width = column_widths
        .first()
        .copied()
        .unwrap_or(0)
        .max(widest_other)
        .max(viewport_width.saturating_sub(other_width_total));

    if let Some(title) = column_widths.first_mut() {
        *title = title_width;
    }

    let total_width = title_width + other_width_total;
    let max_scroll = total_width.saturating_sub(viewport_width);
    let scroll = state.horizontal_scroll().min(max_scroll);
    let scroll_hint_line = if max_scroll > 0 {
        format!(
            "left/right: scroll columns ({}/{}), []: section, {{}}: project, c: completed, z: subtasks, s: sort",
            scroll, max_scroll
        )
    } else {
        "left/right: scroll columns, []: section, {}: project, c: completed, z: subtasks, s: sort"
            .to_string()
    };

    TaskTableView {
        title: match state.focus_mode() {
            TaskFocusMode::Projects => "Task review (project focus)".to_string(),
            TaskFocusMode::Tasks => "Task review (task focus)".to_string(),
        },
        status_line,
        scroll_hint_line,
        columns,
        rows,
        column_widths,
        total_width,
        scroll_offset: scroll,
    }
}

pub fn build_task_lines(
    view: &TaskTableView,
    selected_index: Option<usize>,
    viewport_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(view.rows.len().saturating_add(1));

    lines.push(Line::from(Span::styled(
        slice_row(
            &format_row(&view.columns, &view.column_widths),
            view.scroll_offset,
            viewport_width,
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )));

    for (index, row) in view.rows.iter().enumerate() {
        match row.kind {
            TaskRowKind::ProjectHeader => {
                lines.push(Line::from(Span::styled(
                    slice_text(&row.cells.first().cloned().unwrap_or_default(), viewport_width),
                    Style::default().add_modifier(Modifier::BOLD),
                )));
            }
            TaskRowKind::SectionSpacer => {
                lines.push(Line::from(slice_text("", viewport_width)));
            }
            TaskRowKind::Task => {
                let style = if Some(index) == selected_index {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                lines.push(Line::from(Span::styled(
                    slice_row(
                        &format_row(row.cells.as_slice(), &view.column_widths),
                        view.scroll_offset,
                        viewport_width,
                    ),
                    style,
                )));
            }
        }
    }

    lines
}

fn task_status(state: &TaskReviewState, suffix: &str) -> String {
    let visible = if state.visible() { "visible" } else { "hidden" };
    match state.selected_index() {
        Some(index) => format!(
            "{} tasks, row {}, {}, {}",
            state.table().task_count(),
            state.selected_task_position().unwrap_or(index + 1),
            visible,
            suffix
        ),
        None => format!("{} tasks, {}, {}", state.table().task_count(), visible, suffix),
    }
}

fn natural_column_widths(columns: &[String], rows: &[Vec<String>]) -> Vec<usize> {
    columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            rows.iter()
                .filter_map(|row| row.get(index))
                .map(|value| visible_width(value))
                .chain(std::iter::once(visible_width(column)))
                .max()
                .unwrap_or(0)
        })
        .collect()
}

fn column_cap(index: usize) -> usize {
    match index {
        1 => 18,
        2 => 16,
        3 | 4 => 12,
        5 => 8,
        6 => 18,
        _ => DEFAULT_OTHER_COLUMN_CAP,
    }
}

fn format_row(values: &[String], widths: &[usize]) -> String {
    values
        .iter()
        .zip(widths.iter())
        .map(|(value, width)| pad_cell(value, *width))
        .collect::<Vec<_>>()
        .join(COLUMN_SEPARATOR)
}

fn pad_cell(value: &str, width: usize) -> String {
    let truncated = truncate_to_width(value, width);
    let padding = width.saturating_sub(visible_width(&truncated));
    format!("{truncated}{}", " ".repeat(padding))
}

fn truncate_to_width(value: &str, width: usize) -> String {
    if visible_width(value) <= width {
        return value.to_string();
    }

    let mut result = String::new();
    let mut current_width = 0usize;

    for ch in value.chars() {
        let char_width = ch.width().unwrap_or(0);
        if current_width + char_width > width {
            break;
        }
        result.push(ch);
        current_width += char_width;
    }

    result
}

fn visible_width(value: &str) -> usize {
    value.width()
}

fn slice_row(row: &str, offset: usize, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let end = offset.saturating_add(width);
    let mut cursor = 0usize;
    let mut result = String::new();

    for ch in row.chars() {
        let char_width = ch.width().unwrap_or(0);
        let next_cursor = cursor + char_width;
        if next_cursor > offset && cursor < end {
            result.push(ch);
        }
        cursor = next_cursor;
        if cursor >= end {
            break;
        }
    }

    let current_width = visible_width(&result);
    if current_width < width {
        result.push_str(&" ".repeat(width - current_width));
    }

    result
}

fn slice_text(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut result = String::new();
    let mut current_width = 0usize;

    for ch in text.chars() {
        let char_width = ch.width().unwrap_or(0);
        if current_width + char_width > width {
            break;
        }
        result.push(ch);
        current_width += char_width;
    }

    if current_width < width {
        result.push_str(&" ".repeat(width - current_width));
    }

    result
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
        domain::{Project, TaskRowKind},
    };

    use super::{build_task_lines, render_task_table};

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
                    num_subtasks: 0,
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

        let view = render_task_table(&state, 120);

        assert_eq!(view.title, "Task review (project focus)");
        assert!(view.status_line.contains("1 tasks"));
        assert_eq!(view.columns[0], "Task");
        assert!(view.column_widths[0] >= view.column_widths[1]);
        assert_eq!(view.rows[0].kind, TaskRowKind::ProjectHeader);
        assert_eq!(view.rows[0].cells[0], "Inbox");
        assert_eq!(view.rows[1].kind, TaskRowKind::Task);
        assert_eq!(view.rows[1].cells[0], "Ship");
        assert_eq!(view.rows[1].cells[3], "2026-06-10");
        assert!(view.total_width <= 120);

        let lines = build_task_lines(&view, state.selected_index(), 120);
        assert!(lines[0].to_string().contains("Task"));
        assert!(lines[1].to_string().contains("Inbox"));
    }

    #[test]
    fn renders_loading_state_with_spinner_and_targets() {
        let mut state = TaskReviewState::new();
        state.begin_loading(&[Project::new("p1", "Inbox", true)]);

        let view = render_task_table(&state, 80);

        assert!(view.status_line.contains("Loading tasks"));
        assert!(view.status_line.contains("Inbox"));
        assert!(view.scroll_hint_line.contains("left/right"));
    }

    #[test]
    fn renders_out_of_date_status_with_refresh_hint() {
        let mut state = TaskReviewState::new();
        state.begin_loading(&[Project::new("p1", "Inbox", true)]);
        state.finish_loading(crate::domain::TaskTableModel::empty());
        state.mark_out_of_date("selected projects changed; switch to task view to refresh");

        let view = render_task_table(&state, 80);

        assert!(view.status_line.contains("out of date"));
        assert!(view.status_line.contains("switch to task view"));
    }

    #[test]
    fn renders_task_filter_and_sort_hints() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![crate::asana::dto::SectionDto {
                    gid: "s1".to_string(),
                    name: "Today".to_string(),
                }],
            )
            .with_tasks(
                "p1",
                vec![crate::asana::dto::TaskDto {
                    gid: "t1".to_string(),
                    name: "Ship".to_string(),
                    completed: false,
                    due_on: Some("2026-06-10".to_string()),
                    start_on: Some("2026-06-01".to_string()),
                    assignee: None,
                    num_subtasks: 0,
                    memberships: vec![crate::asana::dto::TaskMembershipDto {
                        project: crate::asana::dto::TaskMembershipProjectDto {
                            gid: "p1".to_string(),
                            name: "Inbox".to_string(),
                        },
                        section: Some(crate::asana::dto::TaskMembershipSectionDto {
                            gid: "s1".to_string(),
                            name: "Today".to_string(),
                        }),
                    }],
                    custom_fields: vec![],
                }],
            );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.toggle_completed_filter();
        state.toggle_subtask_visibility();
        state.cycle_sort_field();

        let view = render_task_table(&state, 80);

        assert!(view.scroll_hint_line.contains("c: completed"));
        assert!(view.scroll_hint_line.contains("z: subtasks"));
        assert!(view.scroll_hint_line.contains("s: sort"));
        assert!(view.status_line.contains("filters:"));
        assert!(view.status_line.contains("sort:"));
    }

    #[test]
    fn keeps_column_markers_aligned_for_varied_lengths() {
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
                        name: "A much longer section name".to_string(),
                    },
                ],
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
                    TaskDto {
                        gid: "t1".to_string(),
                        name: "Short".to_string(),
                        completed: false,
                        due_on: Some("2026-06-10".to_string()),
                        start_on: Some("2026-06-01".to_string()),
                        assignee: Some(crate::asana::dto::UserDto {
                            gid: "u1".to_string(),
                            name: Some("Al".to_string()),
                            display_name: Some("Al".to_string()),
                        }),
                        num_subtasks: 0,
                        memberships: vec![crate::asana::dto::TaskMembershipDto {
                            project: crate::asana::dto::TaskMembershipProjectDto {
                                gid: "p1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(crate::asana::dto::TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Today".to_string(),
                            }),
                        }],
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("High".to_string()),
                            enum_value: None,
                        }],
                    },
                    TaskDto {
                        gid: "t2".to_string(),
                        name: "A title that is intentionally far longer than the others to force width allocation".to_string(),
                        completed: false,
                        due_on: Some("2026-06-11".to_string()),
                        start_on: Some("2026-06-02".to_string()),
                        assignee: Some(crate::asana::dto::UserDto {
                            gid: "u2".to_string(),
                            name: Some("A very very long assignee name".to_string()),
                            display_name: Some("A very very long assignee name".to_string()),
                        }),
                        num_subtasks: 0,
                        memberships: vec![crate::asana::dto::TaskMembershipDto {
                            project: crate::asana::dto::TaskMembershipProjectDto {
                                gid: "p1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(crate::asana::dto::TaskMembershipSectionDto {
                                gid: "s2".to_string(),
                                name: "A much longer section name".to_string(),
                            }),
                        }],
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("Urgent".to_string()),
                            enum_value: None,
                        }],
                    },
                ],
            );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.set_visible(true);

        let view = render_task_table(&state, 200);
        let lines = build_task_lines(&view, state.selected_index(), 200);

        let separator_positions = lines
            .iter()
            .filter_map(|line| line.to_string().find(" | "))
            .collect::<Vec<_>>();

        assert!(!separator_positions.is_empty());
        assert!(separator_positions.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn inserts_project_headers_and_section_spacers() {
        let client = FakeAsanaClient::new(vec![Project::new("p1", "Inbox", true)])
            .with_sections(
                "p1",
                vec![
                    SectionDto {
                        gid: "s1".to_string(),
                        name: "Alpha".to_string(),
                    },
                    SectionDto {
                        gid: "s2".to_string(),
                        name: "Beta".to_string(),
                    },
                ],
            )
            .with_tasks(
                "p1",
                vec![
                    TaskDto {
                        gid: "t1".to_string(),
                        name: "First".to_string(),
                        completed: false,
                        due_on: Some("2026-06-01".to_string()),
                        start_on: None,
                        assignee: None,
                        num_subtasks: 0,
                        memberships: vec![crate::asana::dto::TaskMembershipDto {
                            project: crate::asana::dto::TaskMembershipProjectDto {
                                gid: "p1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(crate::asana::dto::TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Alpha".to_string(),
                            }),
                        }],
                        custom_fields: vec![],
                    },
                    TaskDto {
                        gid: "t2".to_string(),
                        name: "Second".to_string(),
                        completed: false,
                        due_on: Some("2026-06-02".to_string()),
                        start_on: None,
                        assignee: None,
                        num_subtasks: 0,
                        memberships: vec![crate::asana::dto::TaskMembershipDto {
                            project: crate::asana::dto::TaskMembershipProjectDto {
                                gid: "p1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(crate::asana::dto::TaskMembershipSectionDto {
                                gid: "s2".to_string(),
                                name: "Beta".to_string(),
                            }),
                        }],
                        custom_fields: vec![],
                    },
                ],
            );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        let view = render_task_table(&state, 120);
        assert_eq!(view.rows[0].kind, TaskRowKind::ProjectHeader);
        assert_eq!(view.rows[1].kind, TaskRowKind::Task);
        assert_eq!(view.rows[2].kind, TaskRowKind::SectionSpacer);
        assert_eq!(view.rows[3].kind, TaskRowKind::Task);
    }

    #[test]
    fn keeps_column_markers_aligned_when_scrolled_horizontally() {
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
                vec![
                    TaskDto {
                        gid: "t1".to_string(),
                        name: "Short".to_string(),
                        completed: false,
                        due_on: Some("2026-06-10".to_string()),
                        start_on: Some("2026-06-01".to_string()),
                        assignee: Some(crate::asana::dto::UserDto {
                            gid: "u1".to_string(),
                            name: Some("Al".to_string()),
                            display_name: Some("Al".to_string()),
                        }),
                        num_subtasks: 0,
                        memberships: vec![crate::asana::dto::TaskMembershipDto {
                            project: crate::asana::dto::TaskMembershipProjectDto {
                                gid: "p1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(crate::asana::dto::TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Today".to_string(),
                            }),
                        }],
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("High".to_string()),
                            enum_value: None,
                        }],
                    },
                    TaskDto {
                        gid: "t2".to_string(),
                        name: "A title that is intentionally far longer than the others to force width allocation".to_string(),
                        completed: false,
                        due_on: Some("2026-06-11".to_string()),
                        start_on: Some("2026-06-02".to_string()),
                        assignee: Some(crate::asana::dto::UserDto {
                            gid: "u2".to_string(),
                            name: Some("A very very long assignee name".to_string()),
                            display_name: Some("A very very long assignee name".to_string()),
                        }),
                        num_subtasks: 0,
                        memberships: vec![crate::asana::dto::TaskMembershipDto {
                            project: crate::asana::dto::TaskMembershipProjectDto {
                                gid: "p1".to_string(),
                                name: "Inbox".to_string(),
                            },
                            section: Some(crate::asana::dto::TaskMembershipSectionDto {
                                gid: "s1".to_string(),
                                name: "Today".to_string(),
                            }),
                        }],
                        custom_fields: vec![CustomFieldValueDto {
                            gid: "cf1".to_string(),
                            name: "Priority".to_string(),
                            display_value: Some("Urgent".to_string()),
                            enum_value: None,
                        }],
                    },
                ],
            );

        let mut state = TaskReviewState::new();
        state
            .load_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.set_visible(true);
        for _ in 0..12 {
            state.scroll_right();
        }

        let view = render_task_table(&state, 60);
        assert!(view.scroll_offset > 0);
        let lines = build_task_lines(&view, state.selected_index(), 60);
        let separator_positions = lines
            .iter()
            .filter_map(|line| line.to_string().find(" | "))
            .collect::<Vec<_>>();

        assert!(!separator_positions.is_empty());
        assert!(separator_positions.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn scroll_hint_reflects_overflowing_view() {
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
                    name: "Ship release with a very long title that should force scrolling".to_string(),
                    completed: false,
                    due_on: Some("2026-06-10".to_string()),
                    start_on: Some("2026-06-01".to_string()),
                    assignee: Some(crate::asana::dto::UserDto {
                        gid: "u1".to_string(),
                        name: Some("Alex".to_string()),
                        display_name: Some("Alex".to_string()),
                    }),
                    num_subtasks: 0,
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

        let view = render_task_table(&state, 40);

        assert!(view.total_width > 40);
        assert!(view.scroll_hint_line.contains("/"));
    }

    #[test]
    fn renders_subtasks_with_an_indent_marker() {
        let mut parent = crate::domain::TaskRecord::new("p1", "Parent task");
        parent.projects = vec!["Inbox".to_string()];
        parent.sections = vec!["Today".to_string()];

        let mut child = crate::domain::TaskRecord::new("p2", "Child task");
        child.parent_gid = Some("p1".to_string());
        child.subtask_depth = 1;
        child.projects = vec!["Inbox".to_string()];
        child.sections = vec!["Today".to_string()];

        let model = crate::domain::TaskTableModel::from_records_with_settings(
            vec![parent, child],
            vec![],
            &crate::domain::TaskTableSettings::default(),
        );

        let task_titles = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| row.cells[0].clone())
            .collect::<Vec<_>>();

        assert_eq!(task_titles, vec!["Parent task", "  L Child task"]);
    }
}
