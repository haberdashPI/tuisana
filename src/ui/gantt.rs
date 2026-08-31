//! Drawing the Gantt chart region.
//!
//! Every function here returns exactly as many cells as it was given, so the
//! caller can concatenate a row's table cells and its chart cells into one
//! `Line` without measuring anything. That is what keeps the chart on the same
//! line as its task row, and therefore what lets the cursor highlight, zebra
//! striping, and vertical scroll keep working untouched.
//!
//! Cells are filled through a [`Strip`], which paints in layers: whatever is
//! written last wins. The layer order is the precedence — blank, then today's
//! line, then the bar — so a bar covering today hides the line rather than
//! being punched through by it.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::{
    domain::{ColorSlot, GanttModel, GanttTrack, TrackShape, PALETTE_SLOTS},
    ui::{text::visible_width, theme::Theme},
};

/// Gap between two legend entries.
const LEGEND_GAP: &str = "  ";

/// A fixed-width run of styled cells, painted in layers.
struct Strip {
    cells: Vec<(String, Style)>,
}

impl Strip {
    fn blank(width: usize) -> Self {
        Self {
            cells: vec![(" ".to_string(), Style::default()); width],
        }
    }

    /// Paints one cell, ignoring a column past the end.
    fn put(&mut self, column: usize, glyph: &str, style: Style) {
        if let Some(cell) = self.cells.get_mut(column) {
            *cell = (glyph.to_string(), style);
        }
    }

    /// Paints text from `column` rightwards, one character per cell.
    fn write(&mut self, column: usize, text: &str, style: Style) {
        for (offset, character) in text.chars().enumerate() {
            self.put(column + offset, &character.to_string(), style);
        }
    }

    /// Coalesces runs of equally styled cells into spans.
    fn into_spans(self) -> Vec<Span<'static>> {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (glyph, style) in self.cells {
            match spans.last_mut() {
                Some(last) if last.style == style => last.content.to_mut().push_str(&glyph),
                _ => spans.push(Span::styled(glyph, style)),
            }
        }
        spans
    }
}

/// The time axis: month, week, or day marks, and a marker on today's column.
///
/// How finely it is marked is the timeline's decision; this only places what
/// fits. Labels are dropped whole rather than clipped, and month names are
/// placed before the finer marks so a crowded axis keeps the labels that give
/// the others their context.
pub fn axis_spans(model: &GanttModel, theme: &Theme, width: usize) -> Vec<Span<'static>> {
    let mut strip = Strip::blank(width);
    let Some(timeline) = model.timeline else {
        return strip.into_spans();
    };

    // Today is reserved first, so no label can land on it. "Se▼" reads as
    // neither a month nor a marker, and today is the more urgent of the two.
    let mut taken = vec![false; width];
    if let Some(today) = model.today_column {
        taken[today] = true;
    }

    let ticks = timeline.ticks();
    for pass in [true, false] {
        for tick in ticks.iter().filter(|tick| tick.emphasis == pass) {
            let style = match (tick.emphasis, tick.weekend) {
                (true, _) => theme.subtitle,
                // A weekend reads as an aside even on the axis, so it takes
                // the dimmer of the two label styles.
                (false, true) => theme.muted,
                (false, false) => theme.text,
            };
            if reserve(&mut taken, tick.column, visible_width(&tick.label)) {
                strip.write(tick.column, &tick.label, style);
            }
        }
    }

    if let Some(today) = model.today_column {
        strip.put(today, theme.glyphs.sort_desc, theme.warn);
    }

    strip.into_spans()
}

/// Claims `length` cells from `column`, plus one of gap, if all are free.
fn reserve(taken: &mut [bool], column: usize, length: usize) -> bool {
    let end = column + length;
    if end > taken.len() || taken[column..end].iter().any(|cell| *cell) {
        return false;
    }
    let reserved = (end + 1).min(taken.len());
    for cell in taken[column..reserved].iter_mut() {
        *cell = true;
    }
    true
}

/// One task row's bar.
pub fn track_spans(
    track: &GanttTrack,
    model: &GanttModel,
    theme: &Theme,
    width: usize,
) -> Vec<Span<'static>> {
    let mut strip = Strip::blank(width);
    paint_weekends(&mut strip, model, theme);
    paint_today(&mut strip, model, theme);

    let glyphs = &theme.glyphs;
    let mut style = theme.categorical_style(track.slot);
    if track.completed {
        // Agreeing with the state glyph and the crossed-out title, not
        // announcing completion on its own.
        style = style.add_modifier(Modifier::DIM);
    }

    match track.shape {
        TrackShape::Bar(span) => {
            let glyph = theme.bar_glyph(track.slot);
            for column in span.from..=span.to {
                strip.put(column, glyph, style);
            }
            if span.clipped_left {
                strip.put(span.from, glyphs.clip_left, style);
            }
            if span.clipped_right {
                strip.put(span.to, glyphs.clip_right, style);
            }
        }
        TrackShape::Milestone { at } => strip.put(at, glyphs.milestone, style),
        // A task with dates but none of them in view. Without this the row
        // would read as "no dates", which is a different fact.
        TrackShape::OffWindow { left } => {
            let column = if left { 0 } else { width.saturating_sub(1) };
            let glyph = if left { glyphs.clip_left } else { glyphs.clip_right };
            strip.put(column, glyph, theme.muted);
        }
        TrackShape::Empty => {}
    }

    strip.into_spans()
}

/// The chart region of a group header row: a ruler matching the axis.
///
/// Task rows get no gridlines. One mark per tick on every row is noise on
/// exactly the rows the reader is trying to follow, but a heading row has
/// nothing else to say in the chart region.
pub fn gridline_spans(model: &GanttModel, theme: &Theme, width: usize) -> Vec<Span<'static>> {
    let mut strip = Strip::blank(width);
    paint_weekends(&mut strip, model, theme);
    if let Some(timeline) = model.timeline {
        for tick in timeline.ticks() {
            strip.put(tick.column, theme.glyphs.gridline, theme.border);
        }
    }
    paint_today(&mut strip, model, theme);
    strip.into_spans()
}

/// Shades the Saturday and Sunday columns.
///
/// Painted under everything else, so a bar crossing a weekend covers it. The
/// gaps that show through are exactly the ones worth seeing: the weekends a
/// piece of work did not run through.
fn paint_weekends(strip: &mut Strip, model: &GanttModel, theme: &Theme) {
    let Some(timeline) = model.timeline else {
        return;
    };
    for column in timeline.weekend_columns() {
        strip.put(column, theme.weekend_glyph(), theme.border);
    }
}

fn paint_today(strip: &mut Strip, model: &GanttModel, theme: &Theme) {
    if let Some(today) = model.today_column {
        strip.put(
            today,
            theme.glyphs.today_line,
            theme.warn.add_modifier(Modifier::DIM),
        );
    }
}

/// The colour legend, for the task pane's bottom border.
///
/// Only the coloured values are named. Listing every value would defeat the
/// point of a legend on a board with thirty assignees, so the tail is summed
/// into one `other` entry and anything that still does not fit becomes
/// `+N more`.
pub fn legend_line(model: &GanttModel, theme: &Theme, width: usize) -> Line<'static> {
    let mut entries = model
        .assignment
        .ordered()
        .iter()
        .take(PALETTE_SLOTS)
        .enumerate()
        .map(|(slot, value)| (ColorSlot::Indexed(slot), value.clone()))
        .collect::<Vec<_>>();
    if model.assignment.has_neutral() {
        entries.push((ColorSlot::Neutral, "other".to_string()));
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let mut dropped = 0usize;

    for (index, (slot, value)) in entries.iter().enumerate() {
        let gap = if index == 0 { "" } else { LEGEND_GAP };
        let entry_width = visible_width(gap) + 2 + visible_width(value);
        // The overflow marker has to fit too, so reserve room for it while
        // there is still anything left to drop.
        let reserve = if index + 1 < entries.len() { 9 } else { 0 };

        if dropped > 0 || used + entry_width + reserve > width {
            dropped += 1;
            continue;
        }

        if !gap.is_empty() {
            spans.push(Span::raw(gap));
        }
        spans.push(Span::styled(
            theme.bar_glyph(*slot).to_string(),
            theme.categorical_style(*slot),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(value.clone(), theme.muted));
        used += entry_width;
    }

    if dropped > 0 {
        spans.push(Span::styled(format!("{LEGEND_GAP}+{dropped} more"), theme.muted));
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::{axis_spans, gridline_spans, legend_line, track_spans};
    use crate::domain::{
        CustomFieldDefinition, GanttColorKey, GanttModel, TaskRecord, TaskTableModel, TimelineView,
    };
    use crate::domain::date::CivilDate;
    use crate::ui::{text::visible_width, theme::Theme};
    use ratatui::text::Span;

    const WIDTH: usize = 40;

    fn date(year: i32, month: u32, day: u32) -> CivilDate {
        CivilDate::new(year, month, day).expect("a real date")
    }

    fn record(gid: &str, assignee: Option<&str>, start: Option<&str>, due: Option<&str>) -> TaskRecord {
        let mut record = TaskRecord::new(gid, format!("Task {gid}"));
        record.assignee = assignee.map(str::to_string);
        record.start_date = start.map(str::to_string);
        record.due_date = due.map(str::to_string);
        record
    }

    fn model_of(records: Vec<TaskRecord>, today: CivilDate) -> (TaskTableModel, GanttModel) {
        let table = TaskTableModel::from_records(
            records,
            vec![CustomFieldDefinition::new("cf-1", "Priority")],
        );
        let model = GanttModel::build(
            &table,
            &GanttColorKey::Assignee,
            &[],
            &TimelineView::Fit,
            WIDTH,
            today,
        );
        (table, model)
    }

    fn text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    /// The track for one task id, so a test never depends on the table's sort.
    fn track_text(table: &TaskTableModel, model: &GanttModel, gid: &str, theme: &Theme) -> String {
        let index = table
            .rows
            .iter()
            .position(|row| row.gid == gid)
            .expect("the task is in the table");
        text(&track_spans(&model.tracks[index], model, theme, WIDTH))
    }

    #[test]
    fn every_strip_is_exactly_as_wide_as_asked() {
        let theme = Theme::default();
        let (table, model) = model_of(
            vec![record("t1", Some("Alex"), Some("2026-06-10"), Some("2026-08-20"))],
            date(2026, 7, 15),
        );

        assert_eq!(visible_width(&text(&axis_spans(&model, &theme, WIDTH))), WIDTH);
        assert_eq!(
            visible_width(&text(&gridline_spans(&model, &theme, WIDTH))),
            WIDTH
        );
        assert_eq!(
            visible_width(&track_text(&table, &model, "t1", &theme)),
            WIDTH
        );
    }

    #[test]
    fn a_chart_with_no_timeline_draws_blanks_rather_than_nothing() {
        let theme = Theme::default();
        let (table, model) = model_of(vec![record("t1", Some("Alex"), None, None)], date(2026, 7, 15));

        assert_eq!(text(&axis_spans(&model, &theme, WIDTH)), " ".repeat(WIDTH));
        assert_eq!(track_text(&table, &model, "t1", &theme), " ".repeat(WIDTH));
    }

    #[test]
    fn the_axis_names_the_months_and_marks_today() {
        let theme = Theme::default();
        let (_, model) = model_of(
            vec![record("t1", Some("Alex"), Some("2026-06-10"), Some("2026-08-20"))],
            date(2026, 7, 15),
        );
        let axis = text(&axis_spans(&model, &theme, WIDTH));

        assert!(axis.contains("Jun"), "{axis}");
        assert!(axis.contains("Aug"), "{axis}");
        assert!(axis.contains(theme.glyphs.sort_desc), "{axis}");
    }

    #[test]
    fn the_axis_drops_labels_it_cannot_fit_rather_than_clipping_them() {
        let theme = Theme::default();
        // Two years across ten columns: the month boundaries are one third of
        // a column apart, so almost every label has to go.
        let (_, model) = model_of(
            vec![record("t1", Some("Alex"), Some("2025-01-01"), Some("2026-12-31"))],
            date(2030, 1, 1),
        );
        let axis = text(&axis_spans(&model, &theme, 10));

        assert_eq!(visible_width(&axis), 10);
        assert!(axis.starts_with("Jan"), "the first label always fits: {axis}");
        assert!(!axis.contains("Fe") && !axis.contains("Ma"), "{axis}");
    }

    #[test]
    fn a_bar_covers_todays_column_rather_than_being_punched_through_by_it() {
        let theme = Theme::default();
        let (table, model) = model_of(
            vec![record("t1", Some("Alex"), Some("2026-06-10"), Some("2026-08-20"))],
            date(2026, 7, 15),
        );
        let track = track_text(&table, &model, "t1", &theme);

        assert!(
            !track.contains(theme.glyphs.today_line),
            "the bar spans today, so the line is hidden: {track}"
        );
        assert!(track.contains(theme.glyphs.bar));
    }

    #[test]
    fn todays_line_shows_through_a_row_with_no_bar_there() {
        let theme = Theme::default();
        let (table, model) = model_of(
            vec![
                record("t1", Some("Alex"), Some("2026-06-01"), Some("2026-06-10")),
                record("t2", Some("Bo"), Some("2026-08-01"), Some("2026-08-20")),
            ],
            date(2026, 7, 15),
        );

        assert!(track_text(&table, &model, "t1", &theme).contains(theme.glyphs.today_line));
    }

    #[test]
    fn a_milestone_draws_one_glyph_and_no_bar() {
        let theme = Theme::default();
        let (table, model) = model_of(
            vec![
                record("t1", Some("Alex"), None, Some("2026-07-20")),
                record("t2", Some("Bo"), Some("2026-06-01"), Some("2026-08-20")),
            ],
            date(2026, 7, 15),
        );
        let track = track_text(&table, &model, "t1", &theme);

        assert_eq!(track.matches(theme.glyphs.milestone).count(), 1, "{track}");
        assert!(!track.contains(theme.glyphs.bar), "{track}");
    }

    #[test]
    fn a_clipped_bar_is_marked_at_the_edge_it_runs_past() {
        let theme = Theme::default();
        let table = TaskTableModel::from_records(
            vec![record("t1", Some("Alex"), Some("2026-01-01"), Some("2026-12-31"))],
            Vec::new(),
        );
        let model = GanttModel::build(
            &table,
            &GanttColorKey::Assignee,
            &[],
            &TimelineView::Window {
                start: date(2026, 6, 1),
                days: 30,
            },
            WIDTH,
            date(2026, 6, 15),
        );
        let track = track_text(&table, &model, "t1", &theme);

        assert!(track.starts_with(theme.glyphs.clip_left), "{track}");
        assert!(track.ends_with(theme.glyphs.clip_right), "{track}");
    }

    #[test]
    fn a_task_outside_the_window_is_marked_rather_than_left_blank() {
        let theme = Theme::default();
        let table = TaskTableModel::from_records(
            vec![
                record("t1", Some("Alex"), Some("2026-01-01"), Some("2026-01-10")),
                record("t2", Some("Bo"), Some("2026-12-01"), Some("2026-12-10")),
            ],
            Vec::new(),
        );
        let model = GanttModel::build(
            &table,
            &GanttColorKey::Assignee,
            &[],
            &TimelineView::Window {
                start: date(2026, 6, 1),
                days: 30,
            },
            WIDTH,
            date(2026, 6, 15),
        );

        // Blank would say "this task has no dates", which is a different fact.
        assert!(track_text(&table, &model, "t1", &theme).starts_with(theme.glyphs.clip_left));
        assert!(track_text(&table, &model, "t2", &theme).ends_with(theme.glyphs.clip_right));
    }

    #[test]
    fn gridlines_mark_the_month_boundaries() {
        let theme = Theme::default();
        let (_, model) = model_of(
            vec![record("t1", Some("Alex"), Some("2026-06-10"), Some("2026-08-20"))],
            date(2026, 7, 15),
        );
        let grid = text(&gridline_spans(&model, &theme, WIDTH));

        assert_eq!(grid.matches(theme.glyphs.gridline).count(), 3, "{grid}");
    }

    #[test]
    fn the_legend_names_the_coloured_values() {
        let theme = Theme::default();
        let (_, model) = model_of(
            vec![
                record("t1", Some("Alex"), Some("2026-06-10"), Some("2026-07-01")),
                record("t2", Some("Bo"), Some("2026-06-10"), Some("2026-07-01")),
            ],
            date(2026, 7, 15),
        );
        let legend = legend_line(&model, &theme, 80).to_string();

        assert!(legend.contains("Alex"), "{legend}");
        assert!(legend.contains("Bo"), "{legend}");
        assert!(!legend.contains("other"), "nothing fell past the palette");
    }

    #[test]
    fn the_legend_sums_everything_past_the_palette_into_one_entry() {
        let theme = Theme::default();
        let records = (0..9)
            .map(|index| {
                record(
                    &format!("t{index}"),
                    Some(&format!("Person{index}")),
                    Some("2026-06-10"),
                    Some("2026-07-01"),
                )
            })
            .collect();
        let (_, model) = model_of(records, date(2026, 7, 15));
        let legend = legend_line(&model, &theme, 200).to_string();

        assert!(legend.contains("other"), "{legend}");
        assert_eq!(legend.matches("Person").count(), 6, "{legend}");
    }

    #[test]
    fn a_legend_too_wide_for_its_border_counts_what_it_dropped() {
        let theme = Theme::default();
        let records = (0..5)
            .map(|index| {
                record(
                    &format!("t{index}"),
                    Some(&format!("A rather long name {index}")),
                    Some("2026-06-10"),
                    Some("2026-07-01"),
                )
            })
            .collect();
        let (_, model) = model_of(records, date(2026, 7, 15));
        let legend = legend_line(&model, &theme, 40).to_string();

        assert!(legend.contains("more"), "{legend}");
        assert!(visible_width(&legend) <= 40, "{legend}");
    }

    /// A one-week window at eight cells a day, where weekends are shaded.
    fn weekly(records: Vec<TaskRecord>) -> (TaskTableModel, GanttModel) {
        let table = TaskTableModel::from_records(records, Vec::new());
        let model = GanttModel::build(
            &table,
            &GanttColorKey::Assignee,
            &[],
            &TimelineView::Window {
                // Wednesday, so the weekend sits inside the window.
                start: date(2026, 6, 10),
                days: 7,
            },
            56,
            date(2030, 1, 1),
        );
        (table, model)
    }

    #[test]
    fn weekends_are_shaded_on_task_rows_and_heading_rows() {
        let theme = Theme::default();
        let (table, model) = weekly(vec![record("t1", Some("Alex"), None, None)]);

        let track = track_text(&table, &model, "t1", &theme);
        let grid = text(&gridline_spans(&model, &theme, 56));

        // Two days at eight cells each.
        assert_eq!(track.matches(theme.glyphs.weekend).count(), 16, "{track}");
        assert!(grid.contains(theme.glyphs.weekend), "{grid}");
    }

    #[test]
    fn a_bar_covers_the_weekend_it_runs_across() {
        // The gaps that show through are the point: they are the weekends the
        // work did not run through.
        let theme = Theme::default();
        let (table, model) = weekly(vec![record(
            "t1",
            Some("Alex"),
            Some("2026-06-10"),
            Some("2026-06-16"),
        )]);

        assert!(!track_text(&table, &model, "t1", &theme).contains(theme.glyphs.weekend));
    }

    #[test]
    fn a_long_window_shades_no_weekends_at_all() {
        let theme = Theme::default();
        let (table, model) = model_of(
            vec![record("t1", Some("Alex"), Some("2026-01-01"), Some("2026-12-31"))],
            date(2026, 7, 15),
        );

        assert!(!track_text(&table, &model, "t1", &theme).contains(theme.glyphs.weekend));
    }

    #[test]
    fn a_monochrome_theme_shades_weekends_with_a_dot_not_a_shade_block() {
        // Without colour, `░` and the neutral bar's `▒` are one shade apart
        // and read as the same thing.
        use crate::config::{ThemeConfig, ThemeVariant};

        let theme = Theme::new(&ThemeConfig {
            variant: ThemeVariant::Mono,
            ..ThemeConfig::default()
        });
        let (table, model) = weekly(vec![record("t1", Some("Alex"), None, None)]);
        let track = track_text(&table, &model, "t1", &theme);

        assert_eq!(theme.weekend_glyph(), ".");
        assert!(track.contains('.'), "{track}");
        assert!(!track.contains(theme.glyphs.bar_neutral), "{track}");
    }

    #[test]
    fn the_axis_names_weekdays_once_there_is_room() {
        let theme = Theme::default();
        let (_, model) = weekly(vec![record("t1", Some("Alex"), None, None)]);
        let axis = text(&axis_spans(&model, &theme, 56));

        assert!(axis.contains("Thu 11"), "{axis}");
        assert!(axis.contains("Sat 13"), "{axis}");
    }
}
