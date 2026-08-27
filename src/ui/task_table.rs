//! Rendering for the task table.
//!
//! The table is built in two passes. [`render_task_table`] resolves every cell
//! to display text plus a semantic tone — relative dates, state glyphs, empty
//! placeholders — and only then measures column widths, so the widths always
//! match what is actually drawn. The line builders below turn that snapshot
//! into styled lines, one line per row.
//!
//! One line per row is a hard invariant: the app's selection index, vertical
//! scroll, and "keep the cursor visible" logic all address rows by position, so
//! a row that rendered as zero or two lines would desynchronize them.

use std::collections::HashSet;

use ratatui::text::{Line, Span};

use crate::{
    app::task::{TaskState, TaskStatus},
    domain::{TaskRowKind, TaskSortField},
    ui::{
        chrome::{Chip, PaneMessage, Tone},
        date::{self, Urgency},
        text::{
            fill, pad_cell, pad_cell_centered, pad_cell_right_aligned, pad_spans, slice_spans,
            spans_width, visible_width,
        },
        theme::Theme,
    },
};

/// Width of the marker gutter: cursor glyph, selection glyph, gap.
pub const GUTTER_WIDTH: usize = 3;
/// Cells consumed by the rule drawn between two columns.
const COLUMN_SEPARATOR_WIDTH: usize = 3;
/// Share of the viewport the title column may occupy.
const TITLE_SHARE: f64 = 0.6;

/// Horizontal alignment of a cell within its column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

/// What a column means. Drives width limits, alignment, and formatting.
///
/// Built-in columns are identified by position because the model always emits
/// them in a fixed order and appends custom fields after them. Matching on the
/// label would misfire on a custom field named "Due".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnRole {
    Title,
    Assignee,
    Due,
    Start,
    State,
    Projects,
    Custom,
}

impl ColumnRole {
    fn for_index(index: usize) -> Self {
        match index {
            0 => Self::Title,
            1 => Self::Assignee,
            2 => Self::Due,
            3 => Self::Start,
            4 => Self::State,
            5 => Self::Projects,
            _ => Self::Custom,
        }
    }

    /// The header text shown for this column.
    fn header(self, label: &str) -> String {
        match self {
            // The state column is a single glyph, so its header shrinks to match.
            Self::State => "St".to_string(),
            _ => label.to_string(),
        }
    }

    fn align(self) -> Align {
        match self {
            Self::Due | Self::Start => Align::Right,
            Self::State => Align::Center,
            _ => Align::Left,
        }
    }

    /// The smallest and largest useful width for this column.
    ///
    /// A minimum matters as much as a maximum: without one, `Priority` was
    /// clipped to `Prior` with no ellipsis and read as a different word.
    fn width_bounds(self) -> (usize, usize) {
        match self {
            Self::Title => (12, usize::MAX),
            Self::Assignee => (6, 16),
            Self::Due | Self::Start => (5, 10),
            Self::State => (2, 2),
            Self::Projects => (7, 20),
            Self::Custom => (6, 16),
        }
    }

    /// The sort field that orders by this column, if any.
    fn sort_field(self) -> Option<TaskSortField> {
        match self {
            Self::Title => Some(TaskSortField::Title),
            Self::Assignee => Some(TaskSortField::Assignee),
            Self::Due => Some(TaskSortField::Date),
            _ => None,
        }
    }
}

/// One resolved cell: the text to draw and how to draw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderCell {
    /// The display text.
    pub text: String,
    /// A dimmed prefix drawn before the text, used for the subtask marker.
    pub prefix: String,
    /// Which semantic role colors it.
    pub tone: Tone,
}

impl RenderCell {
    fn new(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            prefix: String::new(),
            tone,
        }
    }

    fn width(&self) -> usize {
        visible_width(&self.prefix) + visible_width(&self.text)
    }
}

/// One resolved row, ready to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderRow {
    /// The row kind, which decides how the line is drawn.
    pub kind: TaskRowKind,
    /// The task id, empty for headers and spacers.
    pub gid: String,
    /// The group label, for header rows.
    pub label: String,
    /// Whether this task is complete.
    pub completed: bool,
    /// How deeply the task is nested under a parent.
    pub subtask_depth: usize,
    /// One entry per column, for task rows.
    pub cells: Vec<RenderCell>,
}

/// Snapshot of the task table used by the UI renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskTableView {
    /// The pane title.
    pub title: String,
    /// Header labels, including the sort indicator.
    pub headers: Vec<String>,
    /// Per-column alignment.
    pub aligns: Vec<Align>,
    /// Resolved rows, one per model row.
    pub rows: Vec<RenderRow>,
    /// Resolved column widths.
    pub column_widths: Vec<usize>,
    /// Full rendered width of all columns.
    pub total_width: usize,
    /// Horizontal scroll offset, clamped to what is scrollable.
    pub scroll_offset: usize,
    /// How far the table can scroll horizontally.
    pub max_scroll: usize,
    /// GIDs of tasks in the multi-select set.
    pub selected_task_ids: HashSet<String>,
    /// Counts shown on the right of the pane border.
    pub counts: Vec<Chip>,
    /// Set when there are no rows, explaining why.
    pub message: Option<PaneMessage>,
}

impl TaskTableView {
    /// Whether the table has any content to draw.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Adds a scroll-position chip when columns extend past the pane.
    ///
    /// Columns clipped at the pane edge are otherwise invisible, so the border
    /// says how far right there is still to go.
    fn with_scroll_count(mut self) -> Self {
        if self.max_scroll > 0 {
            self.counts.push(Chip::toned(
                format!("columns {}/{}", self.scroll_offset, self.max_scroll),
                Tone::Info,
            ));
        }
        self
    }
}

/// Resolves the task state into a drawable snapshot.
///
/// `columns_width` is the space available to the columns themselves, i.e. the
/// pane's inner width minus [`GUTTER_WIDTH`].
pub fn render_task_table(
    state: &TaskState,
    columns_width: usize,
    theme: &Theme,
) -> TaskTableView {
    let model = state.table();
    let today = date::today();
    let sort_field = primary_sort_field(state);
    let ascending = primary_sort_ascending(state);

    let headers = model
        .columns
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let role = ColumnRole::for_index(index);
            let mut header = role.header(label);
            if role.sort_field() == Some(sort_field) {
                header.push_str(if ascending {
                    theme.glyphs.sort_asc
                } else {
                    theme.glyphs.sort_desc
                });
            }
            header
        })
        .collect::<Vec<_>>();

    let rows = model
        .rows
        .iter()
        .map(|row| resolve_row(row, today, theme))
        .collect::<Vec<_>>();

    let column_widths = column_widths(&headers, &rows, columns_width);
    let total_width = total_width(&column_widths);
    let max_scroll = total_width.saturating_sub(columns_width);

    TaskTableView {
        title: "Tasks".to_string(),
        aligns: (0..headers.len())
            .map(|index| ColumnRole::for_index(index).align())
            .collect(),
        headers,
        column_widths,
        total_width,
        scroll_offset: state.horizontal_scroll().min(max_scroll),
        max_scroll,
        selected_task_ids: model
            .rows
            .iter()
            .filter(|row| row.kind.is_task() && state.is_task_selected(&row.gid))
            .map(|row| row.gid.clone())
            .collect(),
        counts: counts(state, theme),
        message: message(state, rows.is_empty()),
        rows,
    }
    .with_scroll_count()
}

/// Renders the header row: bold, underlined, and marked with the sort column.
pub fn task_header_line(view: &TaskTableView, theme: &Theme, width: usize) -> Line<'static> {
    let columns_width = width.saturating_sub(GUTTER_WIDTH);
    let mut spans = vec![Span::styled(" ".repeat(GUTTER_WIDTH), theme.header)];

    let header_cells = view
        .headers
        .iter()
        .map(|header| RenderCell::new(header.clone(), Tone::Text))
        .collect::<Vec<_>>();

    let cells = cell_spans(&header_cells, view, theme, true);
    spans.extend(slice_spans(&cells, view.scroll_offset, columns_width));

    Line::from(spans).style(theme.header)
}

/// Renders the table body, one line per row.
pub fn task_body_lines(
    view: &TaskTableView,
    cursor_index: Option<usize>,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let columns_width = width.saturating_sub(GUTTER_WIDTH);
    let mut lines = Vec::with_capacity(view.rows.len());
    let mut task_ordinal = 0usize;

    for (index, row) in view.rows.iter().enumerate() {
        let line = match row.kind {
            // Spacers exist to give the eye a break between groups; drawing
            // column rules through them defeats that.
            TaskRowKind::ProjectSeparator | TaskRowKind::SectionSpacer => {
                Line::from(Span::raw(" ".repeat(width)))
            }
            TaskRowKind::ProjectHeader => group_header_line(&row.label, theme, width, true),
            TaskRowKind::SectionHeader => group_header_line(&row.label, theme, width, false),
            TaskRowKind::Task => {
                let is_cursor = Some(index) == cursor_index;
                let is_selected = view.selected_task_ids.contains(&row.gid);
                let line = task_line(row, view, theme, columns_width, is_cursor, is_selected);
                task_ordinal += 1;
                decorate_task_line(line, theme, is_cursor, task_ordinal)
            }
        };
        lines.push(line);
    }

    lines
}

fn task_line(
    row: &RenderRow,
    view: &TaskTableView,
    theme: &Theme,
    columns_width: usize,
    is_cursor: bool,
    is_selected: bool,
) -> Line<'static> {
    let glyphs = &theme.glyphs;
    let mut spans = vec![
        Span::styled(
            if is_cursor { glyphs.cursor } else { " " }.to_string(),
            theme.marker,
        ),
        Span::styled(
            if is_selected { glyphs.selected } else { " " }.to_string(),
            theme.marker,
        ),
        Span::raw(" "),
    ];

    let cells = cell_spans(&row.cells, view, theme, false);
    spans.extend(slice_spans(&cells, view.scroll_offset, columns_width));

    Line::from(spans)
}

fn decorate_task_line(
    line: Line<'static>,
    theme: &Theme,
    is_cursor: bool,
    task_ordinal: usize,
) -> Line<'static> {
    if is_cursor {
        return line.style(theme.cursor);
    }
    match theme.zebra {
        Some(zebra) if task_ordinal.is_multiple_of(2) => line.style(zebra),
        _ => line,
    }
}

/// Renders a project or section group heading as a labeled rule.
fn group_header_line(
    label: &str,
    theme: &Theme,
    width: usize,
    is_project: bool,
) -> Line<'static> {
    let glyphs = &theme.glyphs;

    let (indent, bar, label_style, rule, rule_style) = if is_project {
        (
            0,
            Some(glyphs.group_bar),
            theme.title,
            glyphs.rule,
            theme.border,
        )
    } else {
        (
            GUTTER_WIDTH,
            None,
            theme.subtitle,
            glyphs.section_rule,
            theme.muted,
        )
    };

    let mut spans = vec![Span::raw(" ".repeat(indent))];
    if let Some(bar) = bar {
        spans.push(Span::styled(bar.to_string(), theme.accent));
    }

    let used = spans_width(&spans);
    let label_width = width.saturating_sub(used + 2).max(1);
    spans.push(Span::styled(
        crate::ui::text::truncate_with_ellipsis(label, label_width, glyphs.ellipsis),
        label_style,
    ));
    spans.push(Span::raw(" "));

    let remaining = width.saturating_sub(spans_width(&spans));
    spans.push(Span::styled(fill(rule, remaining), rule_style));

    Line::from(pad_spans(spans, width))
}

/// Renders a row of cells with column rules between them.
fn cell_spans(
    cells: &[RenderCell],
    view: &TaskTableView,
    theme: &Theme,
    is_header: bool,
) -> Vec<Span<'static>> {
    let mut spans = Vec::with_capacity(cells.len() * 4);

    for (index, (cell, width)) in cells.iter().zip(view.column_widths.iter()).enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                theme.glyphs.column_rule.to_string(),
                if is_header { theme.header } else { theme.border },
            ));
            spans.push(Span::raw(" "));
        }

        let align = view.aligns.get(index).copied().unwrap_or(Align::Left);
        let prefix_width = visible_width(&cell.prefix);
        if prefix_width > 0 {
            spans.push(Span::styled(cell.prefix.clone(), theme.muted));
        }

        let text_width = width.saturating_sub(prefix_width);
        let padded = match align {
            Align::Left => pad_cell(&cell.text, text_width, theme.glyphs.ellipsis),
            Align::Right => pad_cell_right_aligned(&cell.text, text_width, theme.glyphs.ellipsis),
            Align::Center => pad_cell_centered(&cell.text, text_width, theme.glyphs.ellipsis),
        };
        spans.push(Span::styled(padded, cell.tone.style(theme)));
    }

    spans
}

/// Resolves one model row into display text and tones.
fn resolve_row(
    row: &crate::domain::TaskRow,
    today: date::CivilDate,
    theme: &Theme,
) -> RenderRow {
    if !row.kind.is_task() {
        return RenderRow {
            kind: row.kind.clone(),
            gid: row.gid.clone(),
            label: row.cells.first().cloned().unwrap_or_default(),
            completed: false,
            subtask_depth: 0,
            cells: Vec::new(),
        };
    }

    let completed = row.cells.get(4).map(String::as_str) == Some("done");
    let cells = row
        .cells
        .iter()
        .enumerate()
        .map(|(index, value)| {
            resolve_cell(
                ColumnRole::for_index(index),
                value,
                completed,
                row.subtask_depth,
                today,
                theme,
            )
        })
        .collect();

    RenderRow {
        kind: row.kind.clone(),
        gid: row.gid.clone(),
        label: String::new(),
        completed,
        subtask_depth: row.subtask_depth,
        cells,
    }
}

fn resolve_cell(
    role: ColumnRole,
    value: &str,
    completed: bool,
    subtask_depth: usize,
    today: date::CivilDate,
    theme: &Theme,
) -> RenderCell {
    match role {
        ColumnRole::Title => {
            let mut cell = RenderCell::new(
                value,
                if completed { Tone::Muted } else { Tone::Text },
            );
            if subtask_depth > 0 {
                // Two cells per level: indentation for the outer levels, then a
                // marker the renderer dims so the title stays the brightest
                // thing on the line.
                cell.prefix = format!(
                    "{}{} ",
                    "  ".repeat(subtask_depth - 1),
                    theme.glyphs.subtask
                );
            }
            cell
        }
        ColumnRole::State => RenderCell::new(
            if completed {
                theme.glyphs.done
            } else {
                theme.glyphs.open
            },
            if completed { Tone::Ok } else { Tone::Muted },
        ),
        ColumnRole::Due | ColumnRole::Start if !value.trim().is_empty() => {
            let rendered = date::format_relative(value, today);
            // A completed task's date is history, and a start date is context;
            // neither is an emergency, so only an open due date is graded.
            if completed || matches!(role, ColumnRole::Start) {
                return RenderCell::new(rendered.text, Tone::Muted);
            }
            match rendered.urgency {
                // Overdue is marked as well as colored, so it survives a
                // monochrome terminal.
                Urgency::Overdue => RenderCell::new(
                    format!("{}{}", theme.glyphs.overdue, rendered.text),
                    Tone::Danger,
                ),
                Urgency::Today => RenderCell::new(rendered.text, Tone::Warn),
                Urgency::Soon => RenderCell::new(rendered.text, Tone::Accent),
                Urgency::Later => RenderCell::new(rendered.text, Tone::Text),
            }
        }
        _ if value.trim().is_empty() => RenderCell::new(theme.glyphs.empty, Tone::Muted),
        _ => RenderCell::new(
            value,
            if completed { Tone::Muted } else { Tone::Text },
        ),
    }
}

/// Chips describing settings that differ from their defaults.
///
/// A fresh session shows none of these. Listing `off` for every switch, as the
/// old status line did, buried the one setting that had been changed.
pub fn settings_chips(state: &TaskState) -> Vec<Chip> {
    let settings = state.task_settings();
    let mut chips = Vec::new();

    match (settings.sort.group_by_project, settings.sort.group_by_section) {
        (true, true) => {}
        (false, false) => chips.push(Chip::toned("ungrouped", Tone::Accent)),
        (true, false) => chips.push(Chip::toned("group by project", Tone::Accent)),
        (false, true) => chips.push(Chip::toned("group by section", Tone::Accent)),
    }

    match settings.filter.completed {
        Some(false) => {}
        Some(true) => chips.push(Chip::toned("done only", Tone::Warn)),
        None => chips.push(Chip::toned("open + done", Tone::Accent)),
    }

    if matches!(
        settings.filter.subtasks,
        crate::domain::SubtaskVisibility::Hide
    ) {
        chips.push(Chip::toned("no subtasks", Tone::Accent));
    }

    if primary_sort_field(state) != TaskSortField::Date {
        chips.push(Chip::toned(
            format!("sort {}", settings.sort.primary_field_label()),
            Tone::Accent,
        ));
    }

    let filters = state.active_filter_count();
    if filters > 0 {
        chips.push(Chip::toned(
            format!("{filters} filter{}", if filters == 1 { "" } else { "s" }),
            Tone::Accent,
        ));
    }

    chips
}

fn counts(state: &TaskState, theme: &Theme) -> Vec<Chip> {
    let mut chips = Vec::new();

    if let TaskStatus::Loading = state.status() {
        let frame = loading_frame(state, theme).unwrap_or(theme.glyphs.active);
        chips.push(Chip::toned(format!("{frame} loading"), Tone::Info));
    }
    if let TaskStatus::OutOfDate(_) = state.status() {
        chips.push(Chip::toned("stale", Tone::Warn));
    }

    let task_count = state.table().task_count();
    if task_count == 0 {
        return chips;
    }

    chips.push(Chip::new(format!("{task_count} tasks")));
    if let Some(position) = state.selected_task_position() {
        chips.push(Chip::new(format!("row {position}")));
    }
    if state.selected_task_count() > 0 {
        chips.push(Chip::toned(
            format!("{} selected", state.selected_task_count()),
            Tone::Accent,
        ));
    }

    chips
}

fn message(state: &TaskState, no_rows: bool) -> Option<PaneMessage> {
    // A message replaces the grid only when there is nothing to show. Partial
    // results that arrive during a load stay visible.
    match state.status() {
        TaskStatus::Error(error) => Some(
            PaneMessage::new(format!("Could not load tasks: {error}"), Tone::Danger)
                .with_hint("r to retry"),
        ),
        _ if !no_rows => None,
        TaskStatus::Loading => Some(PaneMessage::new(
            loading_text(state),
            Tone::Muted,
        )),
        // Idle covers both "nothing selected" and "selected but never
        // requested", so the message names the action that resolves either.
        TaskStatus::Idle => Some(
            PaneMessage::new("No tasks loaded", Tone::Muted)
                .with_hint("select projects, then press t"),
        ),
        TaskStatus::OutOfDate(message) => {
            Some(PaneMessage::new(message.clone(), Tone::Warn).with_hint("r to refresh"))
        }
        _ => Some(PaneMessage::new("No tasks match", Tone::Muted).with_hint("f to edit filters")),
    }
}

fn loading_text(state: &TaskState) -> String {
    let targets = state.loading_targets();
    if targets.is_empty() {
        "Loading tasks".to_string()
    } else {
        format!("Loading {}", targets.join(", "))
    }
}

/// The spinner frame for the current instant, or `None` when not loading.
pub fn loading_frame(state: &TaskState, theme: &Theme) -> Option<&'static str> {
    let started = state.loading_started_at()?;
    let elapsed = started.elapsed().as_millis() as usize;
    Some(theme.glyphs.spinner_frame(elapsed / 120))
}

fn primary_sort_field(state: &TaskState) -> TaskSortField {
    state
        .task_settings()
        .sort
        .rules
        .first()
        .map(|rule| rule.field)
        .unwrap_or(TaskSortField::Date)
}

fn primary_sort_ascending(state: &TaskState) -> bool {
    state
        .task_settings()
        .sort
        .rules
        .first()
        .map(|rule| matches!(rule.direction, crate::domain::SortDirection::Asc))
        .unwrap_or(true)
}

/// Measures each column, clamped to its role's bounds, then gives the title
/// column whatever is left over.
fn column_widths(headers: &[String], rows: &[RenderRow], viewport: usize) -> Vec<usize> {
    let mut widths = headers
        .iter()
        .enumerate()
        .map(|(index, header)| {
            let role = ColumnRole::for_index(index);
            let (min, max) = role.width_bounds();
            let natural = rows
                .iter()
                .filter(|row| row.kind.is_task())
                .filter_map(|row| row.cells.get(index))
                .map(RenderCell::width)
                .chain(std::iter::once(visible_width(header)))
                .max()
                .unwrap_or(0);
            natural.clamp(min.min(max), max)
        })
        .collect::<Vec<_>>();

    let others: usize = widths.iter().skip(1).copied().sum();
    let separators = COLUMN_SEPARATOR_WIDTH * widths.len().saturating_sub(1);
    let widest_other = widths.iter().skip(1).copied().max().unwrap_or(0);
    let title_cap = ((viewport as f64) * TITLE_SHARE).floor() as usize;

    if let Some(title) = widths.first_mut() {
        *title = (*title)
            .max(widest_other)
            .max(viewport.saturating_sub(others + separators))
            .min(title_cap.max(1));
    }

    widths
}

fn total_width(widths: &[usize]) -> usize {
    widths.iter().sum::<usize>() + COLUMN_SEPARATOR_WIDTH * widths.len().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::{
        render_task_table, settings_chips, task_body_lines, task_header_line, Align, ColumnRole,
        GUTTER_WIDTH,
    };
    use crate::{
        app::task::TaskState,
        asana::{
            dto::{
                CustomFieldDto, CustomFieldValueDto, ProjectCustomFieldSettingDto, SectionDto,
                TaskDto, TaskMembershipDto, TaskMembershipProjectDto, TaskMembershipSectionDto,
                UserDto,
            },
            fake::FakeAsanaClient,
        },
        domain::{Project, TaskRowKind},
        ui::{chrome::Tone, text::visible_width, theme::Theme},
    };

    /// The display column of the first column rule, measured in cells rather
    /// than bytes so multi-byte glyphs earlier in the line do not skew it.
    fn rule_column(line: &str, rule: &str) -> Option<usize> {
        let marker = rule.chars().next()?;
        let mut column = 0usize;
        for ch in line.chars() {
            if ch == marker {
                return Some(column);
            }
            column += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        }
        None
    }

    fn task(gid: &str, name: &str, due: Option<&str>, done: bool) -> TaskDto {
        TaskDto {
            gid: gid.to_string(),
            name: name.to_string(),
            completed: done,
            modified_at: None,
            due_on: due.map(str::to_string),
            start_on: Some("2026-06-01".to_string()),
            assignee: Some(UserDto {
                gid: "u1".to_string(),
                name: Some("Alex Chen".to_string()),
                display_name: Some("Alex Chen".to_string()),
            }),
            num_subtasks: 0,
            memberships: vec![TaskMembershipDto {
                project: TaskMembershipProjectDto {
                    gid: "p1".to_string(),
                    name: "Inbox".to_string(),
                },
                section: Some(TaskMembershipSectionDto {
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
        }
    }

    fn state_with(tasks: Vec<TaskDto>) -> TaskState {
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
            .with_tasks("p1", tasks);

        let mut state = TaskState::new();
        state.set_completed_filter(None);
        state
            .load_task_dataset_for_projects(&client, &[Project::new("p1", "Inbox", true)])
            .expect("tasks load");
        state.set_visible(true);
        state
    }

    #[test]
    fn resolves_columns_headers_and_group_rows() {
        let theme = Theme::default();
        let state = state_with(vec![task("t1", "Ship release", Some("2026-06-10"), false)]);

        let view = render_task_table(&state, 120, &theme);

        assert_eq!(view.title, "Tasks");
        assert_eq!(view.headers[0], "Task");
        assert_eq!(view.headers[4], "St");
        assert!(view.headers[2].starts_with("Due"), "due column marks the sort");
        assert_eq!(view.aligns[2], Align::Right);
        assert_eq!(view.aligns[4], Align::Center);
        assert_eq!(view.rows[0].kind, TaskRowKind::ProjectSeparator);
        assert_eq!(view.rows[1].kind, TaskRowKind::ProjectHeader);
        assert_eq!(view.rows[1].label, "Inbox");
        assert_eq!(view.rows[3].kind, TaskRowKind::SectionHeader);
        assert_eq!(view.rows[3].label, "Today");
        assert_eq!(view.rows[4].kind, TaskRowKind::Task);
        assert_eq!(view.rows[4].cells[0].text, "Ship release");
    }

    #[test]
    fn every_line_is_exactly_the_pane_width() {
        let state = state_with(vec![
            task("t1", "Ship release", Some("2026-06-10"), false),
            task(
                "t2",
                "A title that is intentionally far longer than the pane can show",
                Some("2026-06-11"),
                true,
            ),
        ]);

        let theme = Theme::default();
        for width in [40usize, 80, 120, 200] {
            let view = render_task_table(&state, width.saturating_sub(GUTTER_WIDTH), &theme);

            assert_eq!(
                visible_width(&task_header_line(&view, &theme, width).to_string()),
                width
            );
            for line in task_body_lines(&view, state.selected_index(), &theme, width) {
                assert_eq!(
                    visible_width(&line.to_string()),
                    width,
                    "a body line was not {width} cells wide"
                );
            }
        }
    }

    #[test]
    fn column_rules_stay_aligned_across_rows_and_while_scrolled() {
        let state = state_with(vec![
            task("t1", "Short", Some("2026-06-10"), false),
            task(
                "t2",
                "A much longer title to force the width allocation to move",
                Some("2026-06-11"),
                false,
            ),
        ]);
        let theme = Theme::default();

        for scroll in [0usize, 6, 14] {
            let mut view = render_task_table(&state, 80 - GUTTER_WIDTH, &theme);
            view.scroll_offset = scroll.min(view.max_scroll);

            let mut lines = vec![task_header_line(&view, &theme, 80)];
            lines.extend(task_body_lines(&view, None, &theme, 80));

            let positions = lines
                .iter()
                .map(|line| line.to_string())
                .filter_map(|line| rule_column(&line, theme.glyphs.column_rule))
                .collect::<Vec<_>>();

            assert!(positions.len() >= 2);
            assert!(
                positions.windows(2).all(|pair| pair[0] == pair[1]),
                "rules misaligned at scroll {scroll}: {positions:?}"
            );
        }
    }

    #[test]
    fn group_headers_draw_no_column_rules_or_empty_cells() {
        let state = state_with(vec![task("t1", "Ship release", Some("2026-06-10"), false)]);
        let theme = Theme::default();
        let view = render_task_table(&state, 120 - GUTTER_WIDTH, &theme);

        let lines = task_body_lines(&view, None, &theme, 120);
        let project_header = lines[1].to_string();
        let section_header = lines[3].to_string();

        assert!(project_header.contains("Inbox"));
        assert!(!project_header.contains(theme.glyphs.column_rule));
        assert!(project_header.contains(theme.glyphs.rule));
        assert!(section_header.contains("Today"));
        assert!(!section_header.contains(theme.glyphs.column_rule));
        assert!(lines[0].to_string().trim().is_empty(), "separator is blank");
        assert!(lines[2].to_string().trim().is_empty(), "spacer is blank");
    }

    #[test]
    fn dates_render_relatively_and_carry_urgency() {
        std::env::set_var("TUISANA_TODAY", "2026-06-10");
        let theme = Theme::default();
        let state = state_with(vec![
            task("t1", "Due today", Some("2026-06-10"), false),
            task("t2", "Overdue", Some("2026-06-01"), false),
            task("t3", "Later", Some("2026-09-30"), false),
        ]);

        let view = render_task_table(&state, 120, &theme);
        let by_title = |title: &str| {
            view.rows
                .iter()
                .find(|row| row.cells.first().is_some_and(|cell| cell.text == title))
                .expect("row exists")
                .clone()
        };

        assert_eq!(by_title("Due today").cells[2].text, "Today");
        assert_eq!(by_title("Due today").cells[2].tone, Tone::Warn);
        assert_eq!(by_title("Overdue").cells[2].tone, Tone::Danger);
        assert!(
            by_title("Overdue").cells[2].text.starts_with(theme.glyphs.overdue),
            "an overdue date is marked as well as colored"
        );
        assert_eq!(by_title("Later").cells[2].text, "Sep 30");
        assert_eq!(by_title("Later").cells[2].tone, Tone::Text);
        std::env::remove_var("TUISANA_TODAY");
    }

    #[test]
    fn completed_tasks_are_dimmed_and_marked_done() {
        let state = state_with(vec![task("t1", "Closed", Some("2026-01-01"), true)]);
        let theme = Theme::default();
        let view = render_task_table(&state, 120, &theme);

        let row = view
            .rows
            .iter()
            .find(|row| row.kind.is_task())
            .expect("a task row");

        assert!(row.completed);
        assert_eq!(row.cells[0].tone, Tone::Muted);
        assert_eq!(row.cells[4].text, theme.glyphs.done);
        // An overdue date on a completed task is not an emergency.
        assert_eq!(row.cells[2].tone, Tone::Muted);
    }

    #[test]
    fn empty_cells_show_a_placeholder_rather_than_blank_space() {
        let mut dto = task("t1", "No assignee", None, false);
        dto.assignee = None;
        dto.custom_fields = vec![];
        let state = state_with(vec![dto]);
        let theme = Theme::default();
        let view = render_task_table(&state, 120, &theme);

        let row = view
            .rows
            .iter()
            .find(|row| row.kind.is_task())
            .expect("a task row");

        assert_eq!(row.cells[1].text, theme.glyphs.empty);
        assert_eq!(row.cells[2].text, theme.glyphs.empty);
    }

    #[test]
    fn no_header_is_cut_mid_word_at_eighty_columns() {
        let state = state_with(vec![task("t1", "Ship release", Some("2026-06-10"), false)]);
        let theme = Theme::default();
        let view = render_task_table(&state, 80 - GUTTER_WIDTH, &theme);

        for (index, header) in view.headers.iter().enumerate() {
            let width = view.column_widths[index];
            assert!(
                visible_width(header) <= width || width >= 4,
                "header {header} has no room at all"
            );
        }
        assert!(view.column_widths[5] >= 7, "Projects keeps a minimum width");
        assert!(view.column_widths[6] >= 6, "Priority keeps a minimum width");
    }

    #[test]
    fn subtasks_are_indented_with_a_dimmed_marker() {
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
        let mut state = TaskState::new();
        state.finish_loading(model);
        let theme = Theme::default();
        let view = render_task_table(&state, 120, &theme);

        let titles = view
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| (row.cells[0].prefix.clone(), row.cells[0].text.clone()))
            .collect::<Vec<_>>();

        assert_eq!(
            titles,
            vec![
                (String::new(), "Parent task".to_string()),
                (format!("{} ", theme.glyphs.subtask), "Child task".to_string()),
            ]
        );
    }

    #[test]
    fn counts_report_totals_position_and_selection() {
        let theme = Theme::default();
        let mut state = state_with(vec![
            task("t1", "One", Some("2026-06-10"), false),
            task("t2", "Two", Some("2026-06-11"), false),
        ]);

        let view = render_task_table(&state, 120, &theme);
        let texts = |view: &super::TaskTableView| {
            view.counts
                .iter()
                .map(|chip| chip.text.clone())
                .collect::<Vec<_>>()
        };

        assert_eq!(texts(&view), vec!["2 tasks".to_string(), "row 1".to_string()]);

        let _ = state.apply_action(&crate::input::Action::ToggleTaskSelection, 10);
        assert!(texts(&render_task_table(&state, 120, &theme))
            .iter()
            .any(|text| text == "1 selected"));
    }

    #[test]
    fn settings_chips_report_only_what_differs_from_the_defaults() {
        let mut state = state_with(vec![task("t1", "One", Some("2026-06-10"), false)]);
        state.set_completed_filter(Some(false));

        assert!(settings_chips(&state).is_empty());

        state.toggle_project_grouping();
        state.toggle_subtask_visibility();
        state.cycle_sort_field();

        let chips = settings_chips(&state)
            .iter()
            .map(|chip| chip.text.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            chips,
            vec![
                "group by section".to_string(),
                "no subtasks".to_string(),
                "sort title".to_string(),
            ]
        );
    }

    #[test]
    fn an_empty_table_explains_itself() {
        let theme = Theme::default();
        let mut state = TaskState::new();
        state.set_visible(true);
        let idle = render_task_table(&state, 80, &theme);
        assert!(idle
            .message
            .expect("idle explains itself")
            .text
            .contains("No tasks loaded"));

        state.begin_loading(&[Project::new("p1", "Inbox", true)]);
        let loading = render_task_table(&state, 80, &theme);
        assert!(loading
            .message
            .expect("loading explains itself")
            .text
            .contains("Inbox"));

        state.finish_loading(crate::domain::TaskTableModel::empty());
        state.set_error("token expired".to_string());
        let error = render_task_table(&state, 80, &theme);
        let message = error.message.expect("errors explain themselves");
        assert!(message.text.contains("token expired"));
        assert_eq!(message.tone, Tone::Danger);
    }

    #[test]
    fn column_roles_map_built_ins_by_position_and_the_rest_to_custom() {
        assert_eq!(ColumnRole::for_index(0), ColumnRole::Title);
        assert_eq!(ColumnRole::for_index(4), ColumnRole::State);
        assert_eq!(ColumnRole::for_index(5), ColumnRole::Projects);
        assert_eq!(ColumnRole::for_index(6), ColumnRole::Custom);
        assert_eq!(ColumnRole::for_index(20), ColumnRole::Custom);
    }
}
