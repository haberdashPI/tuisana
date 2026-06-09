//! Rendering helpers for the task table and filter panel.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::task::{TaskFilterPanelEntry, TaskState, TaskStatus},
    config::Mode,
    domain::{TaskRow, TaskRowKind},
};

const COLUMN_SEPARATOR: &str = " | ";
const COLUMN_SEPARATOR_WIDTH: usize = 3;
const DEFAULT_OTHER_COLUMN_CAP: usize = 18;

/// Snapshot of the task table used by the UI renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskTableView {
    /// The pane title.
    pub title: String,
    /// The one-line task status summary.
    pub status_line: String,
    /// Help text for the task pane.
    pub hint_lines: Vec<String>,
    /// Visible column labels.
    pub columns: Vec<String>,
    /// The rendered table rows.
    pub rows: Vec<TaskRow>,
    /// Calculated column widths for rendering.
    pub column_widths: Vec<usize>,
    /// The full rendered width of the table.
    pub total_width: usize,
    /// Horizontal scroll offset.
    pub scroll_offset: usize,
}

/// Snapshot of the task filter panel used by the UI renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskFilterPanelView {
    /// The pane title.
    pub title: String,
    /// The filter rows shown in the panel.
    pub(crate) rows: Vec<TaskFilterPanelEntry>,
    /// Whether any filters are active.
    pub has_active_filters: bool,
    /// Whether the filter editor is in edit mode.
    pub editing: bool,
    /// Help text specific to the filter panel.
    pub help_lines: Vec<String>,
}

/// Renders the task table state into a UI-friendly snapshot.
pub fn render_task_table(
    state: &TaskState,
    viewport_width: usize,
    mode: Mode,
) -> TaskTableView {
    let status_line = match state.status() {
        TaskStatus::Idle => "Tasks idle".to_string(),
        TaskStatus::Loading => {
            let targets = if state.loading_targets().is_empty() {
                String::new()
            } else {
                format!(" for {}", state.loading_targets().join(", "))
            };
            format!("Loading tasks{} {}", targets, state.loading_spinner())
        }
        TaskStatus::Ready => {
            task_status(state, &format!("ready, {}", state.filter_summary()))
        }
        TaskStatus::OutOfDate(message) => task_status(
            state,
            &format!("stale: {message}, {}", state.filter_summary()),
        ),
        TaskStatus::Empty => "No tasks available".to_string(),
        TaskStatus::Error(message) => format!("Error: {message}"),
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

    let other_width_total = column_widths.iter().skip(1).copied().sum::<usize>()
        + COLUMN_SEPARATOR_WIDTH * column_widths.len().saturating_sub(1);
    let widest_other = column_widths.iter().skip(1).copied().max().unwrap_or(0);
    let title_cap = ((viewport_width as f64) * 0.6).floor() as usize;
    let title_width = column_widths
        .first()
        .copied()
        .unwrap_or(0)
        .max(widest_other)
        .max(viewport_width.saturating_sub(other_width_total))
        .min(title_cap.max(1));

    if let Some(title) = column_widths.first_mut() {
        *title = title_width;
    }

    let total_width = title_width + other_width_total;
    let max_scroll = total_width.saturating_sub(viewport_width);
    let scroll = state.horizontal_scroll().min(max_scroll);
    let hint_lines = task_hint_lines(state, scroll, max_scroll);

    TaskTableView {
        title: match mode {
            Mode::Task => "Task review (task focus)".to_string(),
            Mode::Project
            | Mode::ProjectSearch
            | Mode::Filter
            | Mode::FilterEdit => {
                "Task review (project focus)".to_string()
            }
            Mode::Any => "Task review".to_string(),
        },
        status_line,
        hint_lines,
        columns,
        rows,
        column_widths,
        total_width,
        scroll_offset: scroll,
    }
}

/// Renders the header row for the task table.
pub fn format_task_header_line(view: &TaskTableView, viewport_width: usize) -> Line<'static> {
    let spans = row_spans(&view.columns, &view.column_widths, true);
    Line::from(slice_spans(&spans, view.scroll_offset, viewport_width))
}

/// Renders the task body rows for the task table.
pub fn format_task_body(
    view: &TaskTableView,
    selected_index: Option<usize>,
    viewport_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(view.rows.len());

    for (index, row) in view.rows.iter().enumerate() {
        match row.kind {
            TaskRowKind::ProjectSeparator => {
                lines.push(Line::from(separator_line(viewport_width)));
            }
            TaskRowKind::ProjectHeader => {
                lines.push(row_line(
                    row.cells.as_slice(),
                    &view.column_widths,
                    view.scroll_offset,
                    viewport_width,
                    true,
                    None,
                ));
            }
            TaskRowKind::SectionSpacer => {
                lines.push(row_line(
                    row.cells.as_slice(),
                    &view.column_widths,
                    view.scroll_offset,
                    viewport_width,
                    false,
                    None,
                ));
            }
            TaskRowKind::SectionHeader => {
                lines.push(row_line(
                    row.cells.as_slice(),
                    &view.column_widths,
                    view.scroll_offset,
                    viewport_width,
                    true,
                    None,
                ));
            }
            TaskRowKind::Task => {
                let style = if Some(index) == selected_index {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                lines.push(row_line(
                    row.cells.as_slice(),
                    &view.column_widths,
                    view.scroll_offset,
                    viewport_width,
                    false,
                    Some(style),
                ));
            }
        }
    }

    lines
}

/// Renders the header plus body rows for the task table.
pub fn format_task_lines(
    view: &TaskTableView,
    selected_index: Option<usize>,
    viewport_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(view.rows.len().saturating_add(1));
    lines.push(format_task_header_line(view, viewport_width));
    lines.extend(format_task_body(view, selected_index, viewport_width));
    lines
}

/// Renders the task filter panel body.
pub(crate) fn format_task_filter_body(
    view: &TaskFilterPanelView,
    viewport_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut title = view.title.clone();
    if view.has_active_filters {
        title.push_str(" (active)");
    }
    if view.editing {
        title.push_str(" (editing)");
    }
    lines.push(Line::from(vec![Span::styled(
        pad_cell(&title, viewport_width),
        Style::default().add_modifier(Modifier::BOLD),
    )]));

    for row in &view.rows {
        let marker = if row.selected { ">" } else { " " };
        let prefix = format!("{marker} {} [{}]: ", row.label, row.kind);
        if row.kind == "labels" && !row.label_values.is_empty() {
            let mut spans = vec![Span::raw(prefix)];
            for (index, label) in row.label_values.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::raw(" | "));
                }
                let style = if row.selected && row.label_cursor == Some(index) {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(label.clone(), style));
            }
            lines.push(Line::from(spans));
        } else if row.kind == "labels" {
            lines.push(Line::from(vec![Span::raw(format!("{prefix}<none>"))]));
        } else {
            let mut content = prefix;
            content.push_str(&row.query);
            lines.push(Line::from(vec![Span::raw(truncate_to_width(
                &content,
                viewport_width,
            ))]));
        }
    }

    lines
}

fn task_status(state: &TaskState, suffix: &str) -> String {
    match state.selected_index() {
        Some(index) => format!(
            "{} tasks, row {}, {}",
            state.table().task_count(),
            state.selected_task_position().unwrap_or(index + 1),
            suffix
        ),
        None => format!("{} tasks, {}", state.table().task_count(), suffix),
    }
}

fn task_hint_lines(state: &TaskState, scroll: usize, max_scroll: usize) -> Vec<String> {
    let compact_scroll_line = if max_scroll > 0 {
        format!(
            "left/right: scroll columns ({}/{}), p/f/t: modes, [/]/{{}}/0: pane size, r: refresh, q: quit",
            scroll, max_scroll
        )
    } else {
        "left/right: scroll columns, p/f/t: modes, [/]/{}/0: pane size, r: refresh, q: quit"
            .to_string()
    };
    let expanded_scroll_line = if max_scroll > 0 {
        format!("scroll: left/right columns ({}/{})", scroll, max_scroll)
    } else {
        "scroll: left/right columns".to_string()
    };

    if !state.help_details_visible() {
        return vec![
            "?: more hints".to_string(),
            "j/down,k/up: move, ctrl-u/d: page, home/end: top/bottom".to_string(),
            "[]: section jumps, {}: project jumps, s: sort, , project group, . section group"
                .to_string(),
            "f: filters, enter: edit, s: mode, c: completed, z: subtasks".to_string(),
            compact_scroll_line,
        ];
    }

    vec![
        "?: fewer hints".to_string(),
        "navigation: j/down,k/up move; ctrl-u/d page; home/end top/bottom".to_string(),
        expanded_scroll_line,
        String::new(),
        "grouping/sort: [] section jumps; {} project jumps; , project group; . section group; s sort"
            .to_string(),
        String::new(),
        "filters: f panel; enter edit; s cycle mode; esc/enter done; c completed; z subtasks"
            .to_string(),
        String::new(),
        "p/f/t: modes, r: refresh, q: quit".to_string(),
    ]
}

/// Renders the filter panel snapshot from task state, if the panel is visible.
pub(crate) fn render_task_filter_panel(state: &TaskState) -> Option<TaskFilterPanelView> {
    if !state.filter_panel_visible() {
        return None;
    }

    let rows = state.filter_panel_entries();
    let has_active_filters = rows.iter().any(|row| !row.query.trim().is_empty());

    Some(TaskFilterPanelView {
        title: "Task filters".to_string(),
        rows,
        has_active_filters,
        editing: state.filter_panel_editing(),
        help_lines: state.filter_panel_help_lines(),
    })
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
        2 | 3 => 16,
        4 => 8,
        5 => 18,
        _ => DEFAULT_OTHER_COLUMN_CAP,
    }
}

fn row_line(
    values: &[String],
    widths: &[usize],
    scroll_offset: usize,
    viewport_width: usize,
    bold_first_cell: bool,
    style: Option<Style>,
) -> Line<'static> {
    let spans = row_spans(values, widths, bold_first_cell);
    let mut line = Line::from(slice_spans(&spans, scroll_offset, viewport_width));
    if let Some(style) = style {
        line = line.style(style);
    }
    line
}

fn row_spans(values: &[String], widths: &[usize], bold_first_cell: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::with_capacity(widths.len().saturating_mul(2).saturating_sub(1));

    for (index, (value, width)) in values.iter().zip(widths.iter()).enumerate() {
        let cell = if index == 0 {
            pad_title_cell(value, *width)
        } else {
            pad_cell(value, *width)
        };
        let cell_style = if index == 0 && bold_first_cell {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        spans.push(Span::styled(cell, cell_style));
        if index + 1 != widths.len() {
            spans.push(Span::raw(COLUMN_SEPARATOR));
        }
    }

    spans
}

fn slice_spans(spans: &[Span<'static>], offset: usize, width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }

    let mut cursor = 0usize;
    let end = offset.saturating_add(width);
    let mut out = Vec::new();

    for span in spans {
        let mut segment = String::new();
        let style = span.style;

        for ch in span.content.chars() {
            let char_width = ch.width().unwrap_or(0);
            let next_cursor = cursor + char_width;
            if next_cursor > offset && cursor < end {
                segment.push(ch);
            }
            cursor = next_cursor;
            if cursor >= end {
                break;
            }
        }

        if !segment.is_empty() {
            out.push(Span::styled(segment, style));
        }

        if cursor >= end {
            break;
        }
    }

    let current_width = out
        .iter()
        .map(|span| visible_width(span.content.as_ref()))
        .sum::<usize>();
    if current_width < width {
        out.push(Span::raw(" ".repeat(width - current_width)));
    }

    out
}

fn pad_title_cell(value: &str, width: usize) -> String {
    let truncated = truncate_title_to_width(value, width);
    let padding = width.saturating_sub(visible_width(&truncated));
    format!("{truncated}{}", " ".repeat(padding))
}

fn pad_cell(value: &str, width: usize) -> String {
    let truncated = truncate_to_width(value, width);
    let padding = width.saturating_sub(visible_width(&truncated));
    format!("{truncated}{}", " ".repeat(padding))
}

fn truncate_title_to_width(value: &str, width: usize) -> String {
    if visible_width(value) <= width {
        return value.to_string();
    }

    if width == 0 {
        return String::new();
    }

    if width == 1 {
        return "…".to_string();
    }

    let mut result = String::new();
    let mut current_width = 0usize;
    let target_width = width - 1;

    for ch in value.chars() {
        let char_width = ch.width().unwrap_or(0);
        if current_width + char_width > target_width {
            break;
        }
        result.push(ch);
        current_width += char_width;
    }

    result.push('…');
    result
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

fn separator_line(width: usize) -> String {
    "─".repeat(width)
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;

    use crate::{
        app::task::TaskState,
        config::Mode,
        asana::{
            dto::{
                CustomFieldDto, CustomFieldValueDto, ProjectCustomFieldSettingDto, SectionDto,
                TaskDto,
            },
            fake::FakeAsanaClient,
        },
        domain::{Project, TaskRowKind},
    };

    use super::{format_task_body, format_task_header_line, format_task_lines, render_task_table};

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
                    modified_at: None,
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
                    custom_fields: vec![CustomFieldValueDto {
                        gid: "cf1".to_string(),
                        name: "Priority".to_string(),
                        display_value: Some("High".to_string()),
                        enum_value: None,
                    }],
                }],
            );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.set_visible(true);

        let view = render_task_table(&state, 120, Mode::Project);

        assert_eq!(view.title, "Task review (project focus)");
        assert!(view.status_line.contains("1 tasks"));
        assert_eq!(view.columns[0], "Task");
        assert!(view.column_widths[0] >= view.column_widths[1]);
        assert_eq!(view.rows[0].kind, TaskRowKind::ProjectSeparator);
        assert_eq!(view.rows[1].kind, TaskRowKind::ProjectHeader);
        assert_eq!(view.rows[1].cells[0], "Inbox");
        assert_eq!(view.rows[2].kind, TaskRowKind::SectionSpacer);
        assert_eq!(view.rows[3].kind, TaskRowKind::SectionHeader);
        assert_eq!(view.rows[3].cells[0], "Today");
        assert_eq!(view.rows[4].kind, TaskRowKind::Task);
        assert_eq!(view.rows[4].cells[0], "Ship");
        assert_eq!(view.rows[4].cells[2], "2026-06-10");
        assert!(view.total_width <= 120);

        let lines = format_task_lines(&view, state.selected_index(), 120);
        assert!(lines[0].to_string().contains("Task"));
        assert!(lines[2].to_string().contains("Inbox"));
        assert!(format_task_header_line(&view, 120)
            .to_string()
            .contains("Assignee"));
        let body_lines = format_task_body(&view, state.selected_index(), 120);
        assert!(body_lines[2].to_string().contains(" | "));
        assert!(body_lines[3].to_string().contains(" | "));
    }

    #[test]
    fn bolds_project_and_section_labels_without_bolding_separators() {
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
                vec![TaskDto {
                    gid: "t1".to_string(),
                    name: "Ship".to_string(),
                    completed: false,
                    modified_at: None,
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

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        let view = render_task_table(&state, 120, Mode::Project);
        let body_lines = format_task_body(&view, state.selected_index(), 120);

        assert!(body_lines[1].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(body_lines[3].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(!body_lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn renders_loading_state_with_spinner_and_targets() {
        let mut state = TaskState::new();
        state.begin_loading(&[Project::new("p1", "Inbox", true)]);

        let view = render_task_table(&state, 80, Mode::Project);

        assert!(view.status_line.contains("Loading tasks"));
        assert!(view.status_line.contains("Inbox"));
        assert!(view.hint_lines.first().unwrap().contains("more hints"));
    }

    #[test]
    fn renders_out_of_date_status_with_refresh_hint() {
        let mut state = TaskState::new();
        state.begin_loading(&[Project::new("p1", "Inbox", true)]);
        state.finish_loading(crate::domain::TaskTableModel::empty());
        state.mark_out_of_date("selected projects changed; switch to task view to refresh");

        let view = render_task_table(&state, 80, Mode::Project);

        assert!(view.status_line.contains("stale"));
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
                vec![
                    crate::asana::dto::TaskDto {
                        gid: "t1".to_string(),
                        name: "Ship".to_string(),
                        completed: false,
                        modified_at: None,
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
                    },
                    crate::asana::dto::TaskDto {
                        gid: "t2".to_string(),
                        name: "Done".to_string(),
                        completed: true,
                        modified_at: None,
                        due_on: Some("2026-06-11".to_string()),
                        start_on: Some("2026-06-02".to_string()),
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
                    },
                ],
            );

        let mut state = TaskState::new();
        state.set_completed_filter(None);
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.toggle_completed_filter();
        state.toggle_subtask_visibility();
        state.toggle_project_grouping();
        state.toggle_section_grouping();
        state.cycle_sort_field();

        let view = render_task_table(&state, 80, Mode::Project);

        assert_eq!(
            view.hint_lines,
            vec![
                "?: more hints".to_string(),
                "j/down,k/up: move, ctrl-u/d: page, home/end: top/bottom".to_string(),
                "[]: section jumps, {}: project jumps, s: sort, , project group, . section group"
                    .to_string(),
                "f: filters, enter: edit, s: mode, c: completed, z: subtasks".to_string(),
                "left/right: scroll columns, p/f/t: modes, [/]/{}/0: pane size, r: refresh, q: quit"
                    .to_string(),
            ]
        );
        assert!(view.status_line.contains("grp p:off s:off"));
        assert!(view.status_line.contains("comp open"));
        assert!(view.status_line.contains("sub hide"));
        assert!(view.status_line.contains("sort title"));
    }

    #[test]
    fn renders_expanded_task_help_details() {
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
                    modified_at: None,
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

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.toggle_help_details();

        let view = render_task_table(&state, 80, Mode::Project);

        assert_eq!(
            view.hint_lines,
            vec![
                "?: fewer hints".to_string(),
                "navigation: j/down,k/up move; ctrl-u/d page; home/end top/bottom".to_string(),
                "scroll: left/right columns".to_string(),
                String::new(),
                "grouping/sort: [] section jumps; {} project jumps; , project group; . section group; s sort"
                    .to_string(),
                String::new(),
                "filters: f panel; enter edit; s cycle mode; esc/enter done; c completed; z subtasks"
                    .to_string(),
                String::new(),
                "p/f/t: modes, r: refresh, q: quit".to_string(),
            ]
        );
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
                        modified_at: None,
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
                        modified_at: None,
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

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.set_visible(true);

        let view = render_task_table(&state, 200, Mode::Project);
        let lines = format_task_lines(&view, state.selected_index(), 200);

        let separator_positions = lines
            .iter()
            .filter_map(|line| line.to_string().find(" | "))
            .collect::<Vec<_>>();

        assert!(!separator_positions.is_empty());
        assert!(separator_positions
            .windows(2)
            .all(|pair| pair[0] == pair[1]));
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
                        modified_at: None,
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
                        modified_at: None,
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

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        let view = render_task_table(&state, 120, Mode::Project);
        assert_eq!(view.rows[0].kind, TaskRowKind::ProjectSeparator);
        assert_eq!(view.rows[1].kind, TaskRowKind::ProjectHeader);
        assert_eq!(view.rows[2].kind, TaskRowKind::SectionSpacer);
        assert_eq!(view.rows[3].kind, TaskRowKind::SectionHeader);
        assert_eq!(view.rows[4].kind, TaskRowKind::Task);
        assert_eq!(view.rows[5].kind, TaskRowKind::SectionSpacer);
        assert_eq!(view.rows[6].kind, TaskRowKind::SectionHeader);
        assert_eq!(view.rows[7].kind, TaskRowKind::Task);
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
                        modified_at: None,
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
                        modified_at: None,
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

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.set_visible(true);
        for _ in 0..12 {
            state.scroll_right();
        }

        let view = render_task_table(&state, 60, Mode::Project);
        assert!(view.scroll_offset > 0);
        let lines = format_task_lines(&view, state.selected_index(), 60);
        let separator_positions = lines
            .iter()
            .filter_map(|line| line.to_string().find(" | "))
            .collect::<Vec<_>>();

        assert!(!separator_positions.is_empty());
        assert!(separator_positions
            .windows(2)
            .all(|pair| pair[0] == pair[1]));
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
                    name: "Ship release with a very long title that should force scrolling"
                        .to_string(),
                    completed: false,
                    modified_at: None,
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

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        let view = render_task_table(&state, 40, Mode::Project);

        assert!(view.total_width > 40);
        assert!(view
            .hint_lines
            .iter()
            .any(|line| line.contains("p/f/t: modes")));
    }

    #[test]
    fn truncates_the_title_column_with_an_ellipsis_when_it_exceeds_the_view() {
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
                vec![TaskDto {
                    gid: "t1".to_string(),
                    name: "A task title that is intentionally far longer than sixty percent of the viewport width".to_string(),
                    completed: false,
                    modified_at: None,
                    due_on: Some("2026-06-10".to_string()),
                    start_on: Some("2026-06-01".to_string()),
                    assignee: Some(crate::asana::dto::UserDto {
                        gid: "u1".to_string(),
                        name: Some("Alex".to_string()),
                        display_name: Some("Alex".to_string()),
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
                    custom_fields: vec![],
                }],
            );

        let mut state = TaskState::new();
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");

        let view = render_task_table(&state, 50, Mode::Project);
        let lines = format_task_lines(&view, state.selected_index(), 50);
        let rendered = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(view.column_widths[0] <= 30);
        assert!(rendered.iter().any(|line| line.contains('…')));
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
