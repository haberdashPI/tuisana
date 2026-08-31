//! Gantt chart logic: what colours a bar, and where a bar sits in time.
//!
//! Everything here is pure. It turns dates into column positions and values
//! into palette slots, and knows nothing about styles, glyphs, or terminals —
//! the renderer in [`crate::ui::gantt`] owns all of that.
//!
//! Two ideas carry the module:
//!
//! - a [`SlotAssignment`] ranks the values of one dimension and hands the first
//!   [`PALETTE_SLOTS`] of them a colour, leaving the rest neutral. The ranking
//!   is what the colour dialog edits and what config persists.
//! - a [`Timeline`] maps dates onto chart columns. It is resolved fresh for
//!   every frame from a [`TimelineView`], so the window follows the data while
//!   fitted and stays put once the user has scrolled or zoomed.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;

use crate::domain::date::{month_name, CivilDate};
use crate::domain::task::{TaskRow, TaskTableModel, ASSIGNEE_COLUMN, STATE_COLUMN};

/// How many values get a colour of their own before the rest go neutral.
pub const PALETTE_SLOTS: usize = 6;

/// The dimension a bar takes its colour from.
///
/// The `Field` variant carries the field's *name*, never an id: the same custom
/// field has a separate id in each project, so an id-keyed choice would stop
/// matching as soon as the selected project set changed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum GanttColorKey {
    Assignee,
    Section,
    State,
    Field(String),
}

impl GanttColorKey {
    /// The label shown in a status chip or a dialog title.
    pub fn label(&self) -> String {
        match self {
            Self::Assignee => "assignee".to_string(),
            Self::Section => "section".to_string(),
            Self::State => "state".to_string(),
            Self::Field(name) => name.clone(),
        }
    }
}

impl fmt::Display for GanttColorKey {
    /// The config spelling, which [`FromStr`] round-trips.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Assignee => formatter.write_str("assignee"),
            Self::Section => formatter.write_str("section"),
            Self::State => formatter.write_str("state"),
            Self::Field(name) => write!(formatter, "field:{name}"),
        }
    }
}

impl FromStr for GanttColorKey {
    type Err = ();

    /// Parses a config spelling.
    ///
    /// A custom field is written `field:Name` rather than bare, so a field
    /// actually called "Section" cannot shadow the built-in dimension.
    fn from_str(value: &str) -> Result<Self, ()> {
        let value = value.trim();
        if let Some(name) = value.strip_prefix("field:") {
            let name = name.trim();
            return if name.is_empty() {
                Err(())
            } else {
                Ok(Self::Field(name.to_string()))
            };
        }
        match value.to_ascii_lowercase().as_str() {
            "assignee" => Ok(Self::Assignee),
            "section" => Ok(Self::Section),
            "state" => Ok(Self::State),
            _ => Err(()),
        }
    }
}

/// Which colour a value draws in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSlot {
    /// One of the [`PALETTE_SLOTS`] distinct colours.
    Indexed(usize),
    /// The shared neutral colour, for everything past the palette.
    Neutral,
}

impl ColorSlot {
    /// Whether this slot has a colour of its own.
    pub fn is_indexed(self) -> bool {
        matches!(self, Self::Indexed(_))
    }
}

/// A dimension's values in colour order, and the slot each one drew.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotAssignment {
    ordered: Vec<String>,
    slots: HashMap<String, ColorSlot>,
}

impl SlotAssignment {
    /// Ranks `values` and hands out the palette.
    ///
    /// `order` comes from the colour dialog by way of config. Values it names
    /// lead, in its order; the rest follow alphabetically rather than in table
    /// order, so re-sorting or filtering the table never repaints a bar.
    ///
    /// An empty value is dropped before ranking. "Unassigned" is usually the
    /// most common value on the board, and letting it spend one of six colours
    /// to say nothing would be a poor trade.
    pub fn new(values: &[String], order: &[String]) -> Self {
        let present: HashSet<&str> = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect();

        let mut ordered: Vec<String> = Vec::with_capacity(present.len());
        let mut taken: HashSet<&str> = HashSet::new();
        for name in order {
            let name = name.trim();
            if present.contains(name) && taken.insert(name) {
                ordered.push(name.to_string());
            }
        }

        let mut rest: Vec<&str> = present
            .iter()
            .copied()
            .filter(|value| !taken.contains(value))
            .collect();
        rest.sort_by(|left, right| {
            left.to_lowercase()
                .cmp(&right.to_lowercase())
                .then_with(|| left.cmp(right))
        });
        ordered.extend(rest.into_iter().map(str::to_string));

        let slots = ordered
            .iter()
            .enumerate()
            .map(|(rank, value)| {
                let slot = if rank < PALETTE_SLOTS {
                    ColorSlot::Indexed(rank)
                } else {
                    ColorSlot::Neutral
                };
                (value.clone(), slot)
            })
            .collect();

        Self { ordered, slots }
    }

    /// The slot a value draws in. Unknown and empty values are neutral.
    pub fn slot(&self, value: &str) -> ColorSlot {
        self.slots
            .get(value.trim())
            .copied()
            .unwrap_or(ColorSlot::Neutral)
    }

    /// Every value, colours first, in the order the legend and dialog show.
    pub fn ordered(&self) -> &[String] {
        &self.ordered
    }

    /// Whether any value fell past the palette.
    pub fn has_neutral(&self) -> bool {
        self.ordered.len() > PALETTE_SLOTS
    }
}

/// The span of time a chart is showing.
///
/// `Fit` follows the data: the window is recomputed from whatever rows are in
/// the table, so filtering to one section zooms to that section. `Window` does
/// not, which is the whole difference — once the user has scrolled somewhere,
/// a filter change must not silently move them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimelineView {
    #[default]
    Fit,
    Window {
        start: CivilDate,
        days: u32,
    },
}

/// A resolved time window mapped onto chart columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeline {
    /// First day in the window.
    pub start: CivilDate,
    /// Last day in the window, inclusive.
    pub end: CivilDate,
    /// How many columns the window is drawn across.
    pub width: usize,
}

/// How finely the axis is marked.
///
/// Chosen from how much room a unit of time has, not from the zoom rung: the
/// same span reads differently in a 30-cell pane and a 100-cell one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickScale {
    Month,
    Week,
    Day,
    /// Day numbers with their weekday names, once there is room for both.
    Weekday,
}

/// One mark on the axis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tick {
    /// Where it sits.
    pub column: usize,
    /// What to write there.
    pub label: String,
    /// Whether this is a month boundary. Those are placed before the finer
    /// marks, so a crowded axis keeps the label that gives the others context.
    pub emphasis: bool,
    /// Whether the mark falls on a Saturday or a Sunday.
    pub weekend: bool,
}

/// Where a task's dates land on the chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarSpan {
    /// First column covered.
    pub from: usize,
    /// Last column covered, inclusive.
    pub to: usize,
    /// The task began before the window.
    pub clipped_left: bool,
    /// The task runs past the window.
    pub clipped_right: bool,
}

impl Timeline {
    /// Fits a window to every date in `dates`, snapped to whole months.
    ///
    /// Returns `None` when nothing is dated, which is the caller's cue to draw
    /// no chart at all rather than an arbitrary window.
    ///
    /// Today is deliberately not forced into the window. Tasks from two years
    /// ago would otherwise squash into a few columns to make room for a marker;
    /// there is a key for going to look at today instead.
    pub fn fit(dates: impl IntoIterator<Item = CivilDate>, width: usize) -> Option<Self> {
        let mut iterator = dates.into_iter();
        let first = iterator.next()?;
        let (min, max) = iterator.fold((first, first), |(min, max), date| {
            (min.min(date), max.max(date))
        });

        Some(Self {
            start: min.first_of_month(),
            end: CivilDate {
                day: max.days_in_month(),
                ..max
            },
            width: width.max(1),
        })
    }

    /// Builds a window of an exact length, with no snapping.
    ///
    /// Scrolling by a quarter of a window has to land where it says it lands,
    /// so this is the one construction that leaves month boundaries alone.
    pub fn windowed(start: CivilDate, days: u32, width: usize) -> Self {
        Self {
            start,
            end: start.add_days(days.max(1) as i64 - 1),
            width: width.max(1),
        }
    }

    /// Resolves the window a view is asking for.
    pub fn resolve(
        view: &TimelineView,
        dates: impl IntoIterator<Item = CivilDate>,
        width: usize,
    ) -> Option<Self> {
        match view {
            TimelineView::Fit => Self::fit(dates, width),
            TimelineView::Window { start, days } => Some(Self::windowed(*start, *days, width)),
        }
    }

    /// How many days the window covers.
    pub fn days(&self) -> u32 {
        (self.end.days_from_epoch() - self.start.days_from_epoch() + 1).max(1) as u32
    }

    /// Whether a date falls inside the window.
    pub fn contains(&self, date: CivilDate) -> bool {
        date >= self.start && date <= self.end
    }

    /// The column a date starts in, or `None` when it is outside the window.
    ///
    /// The mapping is proportional rather than a fixed days-per-column, so the
    /// window always fills the chart exactly and one implementation covers a
    /// two-week span and a five-year one.
    pub fn column_for(&self, date: CivilDate) -> Option<usize> {
        self.contains(date).then(|| self.column_clamped(date))
    }

    /// The column a date starts in, pinned to the window's edges.
    pub fn column_clamped(&self, date: CivilDate) -> usize {
        (self.offset_of(date) * self.width / self.days() as usize).min(self.width - 1)
    }

    /// The last column a date occupies, pinned to the window's edges.
    ///
    /// A day is a range of columns, not a point, whenever the chart is wider
    /// than the window is long. Without this the final column would never be
    /// drawn: at 40 columns over 30 days the last day starts at column 38 and
    /// the bar stopped an cell and a half short of the edge.
    pub fn column_end_clamped(&self, date: CivilDate) -> usize {
        let next = (self.offset_of(date) + 1) * self.width / self.days() as usize;
        next.saturating_sub(1).min(self.width - 1)
    }

    /// How many days into the window a date falls, pinned to its ends.
    fn offset_of(&self, date: CivilDate) -> usize {
        (date.days_from_epoch() - self.start.days_from_epoch())
            .clamp(0, self.days() as i64 - 1) as usize
    }

    /// Where a task running `from`..=`to` draws, or `None` when none of it is
    /// in view.
    ///
    /// A span always covers at least one column, so a one-day task is visible
    /// rather than rounding away to nothing.
    pub fn span(&self, from: CivilDate, to: CivilDate) -> Option<BarSpan> {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        if to < self.start || from > self.end {
            return None;
        }

        let start_column = self.column_clamped(from);
        Some(BarSpan {
            from: start_column,
            to: self.column_end_clamped(to).max(start_column),
            clipped_left: from < self.start,
            clipped_right: to > self.end,
        })
    }

    /// Whether a task entirely outside the window sits before or after it.
    pub fn side_of(&self, date: CivilDate) -> Option<bool> {
        match date {
            date if date < self.start => Some(true),
            date if date > self.end => Some(false),
            _ => None,
        }
    }

    /// How finely this window can be marked.
    ///
    /// `Mon 11` needs about seven cells, a bare `11` about three, and a week
    /// needs one per day to be worth marking. Below that there is only room to
    /// name months. Measured against the pane rather than the zoom rung,
    /// because the same span reads differently at 30 cells and at 100.
    ///
    /// Each step strictly adds information to the one below it, so zooming in
    /// never trades the day number away for the weekday name.
    pub fn tick_scale(&self) -> TickScale {
        let days = self.days() as usize;
        if days * 7 <= self.width {
            TickScale::Weekday
        } else if days * 3 <= self.width {
            TickScale::Day
        } else if days <= self.width {
            TickScale::Week
        } else {
            TickScale::Month
        }
    }

    /// The columns that fall on a Saturday or a Sunday.
    ///
    /// Empty below one cell per day: a column covering half a week is not a
    /// weekend, and shading it would be a lie rather than a hint.
    ///
    /// Walked forwards over the days rather than backwards from the columns.
    /// The column mapping has no exact inverse once a day is wider than one
    /// cell, and asking each day where it lands sidesteps the question.
    pub fn weekend_columns(&self) -> Vec<usize> {
        if self.days() as usize > self.width {
            return Vec::new();
        }

        let mut columns = Vec::new();
        let mut cursor = self.start;
        while cursor <= self.end {
            if cursor.weekday_index() >= 5 {
                columns.extend(self.column_clamped(cursor)..=self.column_end_clamped(cursor));
            }
            cursor = cursor.add_days(1);
        }
        columns
    }

    /// Every mark the axis could draw.
    ///
    /// All of them are returned, including ones too close together to label.
    /// Gridlines want every mark and the axis wants only the ones it has room
    /// for, and only the renderer knows how wide a label is.
    ///
    /// The first of a month is a mark at every scale, labelled with the
    /// month's name rather than a bare `1`: a row of day numbers with no month
    /// anywhere in sight says very little.
    pub fn ticks(&self) -> Vec<Tick> {
        let scale = self.tick_scale();
        let mut ticks: Vec<Tick> = Vec::new();
        let mut cursor = self.start;

        while cursor <= self.end {
            let starts_month = cursor.day == 1 || cursor == self.start;
            let wanted = match scale {
                TickScale::Month => starts_month,
                TickScale::Week => starts_month || cursor.weekday_index() == 0,
                TickScale::Day | TickScale::Weekday => true,
            };

            // The window's first day is named even when it is not the first
            // of the month: whatever else the leftmost label says, "which
            // month is this" is the most useful thing it can say.
            if wanted {
                if let Some(column) = self.column_for(cursor) {
                    push_tick(
                        &mut ticks,
                        Tick {
                            column,
                            label: match (starts_month, scale) {
                                // The month's name replaces the day's own
                                // label at every scale: its position already
                                // says which day it is.
                                (true, _) => month_name(cursor.month).to_string(),
                                (false, TickScale::Weekday) => {
                                    format!("{} {}", cursor.weekday(), cursor.day)
                                }
                                (false, _) => cursor.day.to_string(),
                            },
                            emphasis: starts_month,
                            weekend: cursor.weekday_index() >= 5,
                        },
                    );
                }
            }
            cursor = cursor.add_days(1);
        }

        ticks
    }
}

/// What one row draws in the chart region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackShape {
    /// A task with a span of time, possibly running past the window.
    Bar(BarSpan),
    /// A due date with no start date.
    Milestone { at: usize },
    /// A dated task with none of its span in view. `left` is true when it sits
    /// behind the window.
    OffWindow { left: bool },
    /// A row with no dates: an undated task, or a header or spacer.
    Empty,
}

/// One row's bar, ready for the renderer to style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GanttTrack {
    /// The colour the bar draws in.
    pub slot: ColorSlot,
    /// Where and how it draws.
    pub shape: TrackShape,
    /// Whether the task is complete, which dims the bar.
    pub completed: bool,
}

impl GanttTrack {
    /// The blank track drawn for headers, spacers, and undated tasks.
    const EMPTY: Self = Self {
        slot: ColorSlot::Neutral,
        shape: TrackShape::Empty,
        completed: false,
    };
}

/// Everything the renderer needs to draw one frame of the chart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GanttModel {
    /// The resolved window, or `None` when no row has a date.
    pub timeline: Option<Timeline>,
    /// Today's column, when today falls inside the window.
    pub today_column: Option<usize>,
    /// One track per row of the table, in the same order.
    pub tracks: Vec<GanttTrack>,
    /// The colour order, for the legend and the dialog.
    pub assignment: SlotAssignment,
}

impl GanttModel {
    /// Resolves a table into a drawable chart.
    pub fn build(
        model: &TaskTableModel,
        key: &GanttColorKey,
        order: &[String],
        view: &TimelineView,
        width: usize,
        today: CivilDate,
    ) -> Self {
        let values = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .map(|row| color_value(model, row, key))
            .collect::<Vec<_>>();
        let assignment = SlotAssignment::new(&values, order);

        let dates = model
            .rows
            .iter()
            .filter(|row| row.kind.is_task())
            .flat_map(|row| [row.start, row.due])
            .flatten();
        let timeline = Timeline::resolve(view, dates, width);

        let tracks = model
            .rows
            .iter()
            .map(|row| match (row.kind.is_task(), timeline) {
                (true, Some(timeline)) => GanttTrack {
                    slot: assignment.slot(&color_value(model, row, key)),
                    shape: shape_for(row, &timeline),
                    completed: row.cells.get(STATE_COLUMN).map(String::as_str) == Some("done"),
                },
                _ => GanttTrack::EMPTY,
            })
            .collect();

        Self {
            today_column: timeline.and_then(|timeline| timeline.column_for(today)),
            timeline,
            tracks,
            assignment,
        }
    }

    /// Whether there is a window to draw in.
    pub fn has_timeline(&self) -> bool {
        self.timeline.is_some()
    }
}

/// Where one row's dates put it on the chart.
fn shape_for(row: &TaskRow, timeline: &Timeline) -> TrackShape {
    let off_window = |date: CivilDate| match timeline.side_of(date) {
        Some(left) => TrackShape::OffWindow { left },
        // Unreachable: `side_of` only returns `None` for a date inside the
        // window, and every caller here has already failed to place one.
        None => TrackShape::Empty,
    };

    match (row.start, row.due) {
        // Reversed dates are drawn as the span between them rather than as a
        // milestone. The bar then covers both dates the API actually sent,
        // and the table's own columns still show which way round they are.
        (Some(start), Some(due)) => match timeline.span(start, due) {
            Some(span) => TrackShape::Bar(span),
            None => off_window(due),
        },
        (Some(start), None) => match timeline.span(start, start) {
            Some(span) => TrackShape::Bar(span),
            None => off_window(start),
        },
        (None, Some(due)) => match timeline.column_for(due) {
            Some(at) => TrackShape::Milestone { at },
            None => off_window(due),
        },
        (None, None) => TrackShape::Empty,
    }
}

/// The value a row takes its colour from, empty when it has none.
fn color_value(model: &TaskTableModel, row: &TaskRow, key: &GanttColorKey) -> String {
    let cell = |index: usize| {
        row.cells
            .get(index)
            .map(|value| value.trim().to_string())
            .unwrap_or_default()
    };

    match key {
        GanttColorKey::Assignee => cell(ASSIGNEE_COLUMN),
        GanttColorKey::State => cell(STATE_COLUMN),
        GanttColorKey::Section => row
            .section
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string(),
        GanttColorKey::Field(name) => model
            .custom_column_index(name)
            .map(cell)
            .unwrap_or_default(),
    }
}

/// Every value of one dimension with how many task rows carry it.
///
/// The empty value is included, so the colour dialog can show how many tasks
/// are unassigned even though it will never give them a colour. Ordered the
/// same way the palette is, with the empty value last.
pub fn distinct_values(
    model: &TaskTableModel,
    key: &GanttColorKey,
    order: &[String],
) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in model.rows.iter().filter(|row| row.kind.is_task()) {
        *counts.entry(color_value(model, row, key)).or_default() += 1;
    }

    let empty = counts.remove("").map(|count| (String::new(), count));
    let named = counts.keys().cloned().collect::<Vec<_>>();

    SlotAssignment::new(&named, order)
        .ordered()
        .iter()
        .map(|value| {
            let count = counts.get(value).copied().unwrap_or_default();
            (value.clone(), count)
        })
        .chain(empty)
        .collect()
}

/// Adds a tick, letting a month boundary win a column it has to share.
///
/// Two marks can land on one column when a scale is close to its limit.
/// Keeping the day number there would drop the month name for no gain.
fn push_tick(ticks: &mut Vec<Tick>, tick: Tick) {
    match ticks.last_mut() {
        Some(last) if last.column == tick.column => {
            if tick.emphasis {
                *last = tick;
            }
        }
        _ => ticks.push(tick),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        distinct_values, ColorSlot, GanttColorKey, GanttModel, SlotAssignment, TickScale,
        Timeline, TimelineView, TrackShape, PALETTE_SLOTS,
    };
    use crate::domain::date::CivilDate;
    use crate::domain::task::{CustomFieldDefinition, TaskRecord, TaskTableModel};
    use std::collections::HashMap;

    fn date(year: i32, month: u32, day: u32) -> CivilDate {
        CivilDate::new(year, month, day).expect("a real date")
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn color_keys_round_trip_their_config_spelling() {
        for spelling in ["assignee", "section", "state", "field:Priority"] {
            let parsed: GanttColorKey = spelling.parse().expect("parses");
            assert_eq!(parsed.to_string(), spelling);
        }
    }

    #[test]
    fn a_custom_field_needs_the_field_prefix_to_be_recognized() {
        assert_eq!(
            "field:Section".parse::<GanttColorKey>(),
            Ok(GanttColorKey::Field("Section".to_string())),
            "the prefix is what keeps a field called Section from shadowing the built-in"
        );
        assert_eq!("Priority".parse::<GanttColorKey>(), Err(()));
        assert_eq!("field:".parse::<GanttColorKey>(), Err(()));
    }

    #[test]
    fn configured_values_lead_and_the_rest_follow_alphabetically() {
        let values = strings(&["Zoe", "alice", "Bob", "Priya"]);
        let assignment = SlotAssignment::new(&values, &strings(&["Priya", "Zoe"]));

        assert_eq!(assignment.ordered(), strings(&["Priya", "Zoe", "alice", "Bob"]));
    }

    #[test]
    fn a_configured_value_that_is_not_present_is_skipped_rather_than_listed() {
        let values = strings(&["Bob"]);
        let assignment = SlotAssignment::new(&values, &strings(&["Priya", "Bob"]));

        assert_eq!(assignment.ordered(), strings(&["Bob"]));
    }

    #[test]
    fn only_the_first_six_values_get_a_colour() {
        let values = strings(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let assignment = SlotAssignment::new(&values, &[]);

        for (rank, value) in values.iter().enumerate().take(PALETTE_SLOTS) {
            assert_eq!(assignment.slot(value), ColorSlot::Indexed(rank));
        }
        assert_eq!(assignment.slot("g"), ColorSlot::Neutral);
        assert_eq!(assignment.slot("h"), ColorSlot::Neutral);
        assert!(assignment.has_neutral());
    }

    #[test]
    fn an_empty_value_is_neutral_and_never_spends_a_slot() {
        let values = strings(&["", "   ", "a", "b"]);
        let assignment = SlotAssignment::new(&values, &[]);

        assert_eq!(assignment.ordered(), strings(&["a", "b"]));
        assert_eq!(assignment.slot(""), ColorSlot::Neutral);
        assert_eq!(assignment.slot("   "), ColorSlot::Neutral);
        assert_eq!(assignment.slot("a"), ColorSlot::Indexed(0));
        assert!(!assignment.has_neutral());
    }

    #[test]
    fn an_unknown_value_is_neutral() {
        let assignment = SlotAssignment::new(&strings(&["a"]), &[]);
        assert_eq!(assignment.slot("somebody else"), ColorSlot::Neutral);
    }

    #[test]
    fn fitting_snaps_out_to_whole_months() {
        let timeline = Timeline::fit(vec![date(2026, 6, 15), date(2026, 8, 3)], 40)
            .expect("dated rows fit");

        assert_eq!(timeline.start, date(2026, 6, 1));
        assert_eq!(timeline.end, date(2026, 8, 31));
    }

    #[test]
    fn fitting_nothing_produces_no_window() {
        assert_eq!(Timeline::fit(Vec::new(), 40), None);
    }

    #[test]
    fn fitting_ignores_today_so_stale_tasks_stay_readable() {
        // Tasks two years behind today. Stretching the window to reach today
        // would squash all of them into the first column or two.
        let timeline = Timeline::fit(vec![date(2024, 1, 10), date(2024, 2, 20)], 40)
            .expect("dated rows fit");

        assert_eq!(timeline.start, date(2024, 1, 1));
        assert_eq!(timeline.end, date(2024, 2, 29));
    }

    #[test]
    fn a_window_is_exactly_as_long_as_asked_and_is_not_snapped() {
        let timeline = Timeline::windowed(date(2026, 6, 15), 30, 40);

        assert_eq!(timeline.start, date(2026, 6, 15));
        assert_eq!(timeline.end, date(2026, 7, 14));
        assert_eq!(timeline.days(), 30);
    }

    #[test]
    fn resolve_follows_the_data_when_fitted_and_ignores_it_when_windowed() {
        let dates = vec![date(2026, 6, 15), date(2026, 8, 3)];

        let fitted = Timeline::resolve(&TimelineView::Fit, dates.clone(), 40).expect("fits");
        assert_eq!(fitted.start, date(2026, 6, 1));

        let view = TimelineView::Window {
            start: date(2026, 1, 1),
            days: 31,
        };
        let windowed = Timeline::resolve(&view, dates, 40).expect("windowed");
        assert_eq!(windowed.start, date(2026, 1, 1));
        assert_eq!(windowed.end, date(2026, 1, 31));
    }

    #[test]
    fn a_window_resolves_even_when_no_row_has_a_date() {
        let view = TimelineView::Window {
            start: date(2026, 1, 1),
            days: 31,
        };
        assert!(Timeline::resolve(&view, Vec::new(), 40).is_some());
    }

    #[test]
    fn columns_run_from_zero_to_the_last_and_scale_in_between() {
        // 40 days across 40 columns: one column per day, so the arithmetic is
        // checkable by hand.
        let timeline = Timeline::windowed(date(2026, 6, 1), 40, 40);

        assert_eq!(timeline.column_for(date(2026, 6, 1)), Some(0));
        assert_eq!(timeline.column_for(date(2026, 6, 21)), Some(20));
        assert_eq!(timeline.column_for(date(2026, 7, 10)), Some(39));
        assert_eq!(timeline.column_for(date(2026, 5, 31)), None);
        assert_eq!(timeline.column_for(date(2026, 7, 11)), None);
    }

    #[test]
    fn a_bar_over_the_whole_window_reaches_the_last_column() {
        // A day is a range of columns once the chart is wider than the window
        // is long. Treating it as a point left the final column unpainted.
        let timeline = Timeline::windowed(date(2026, 6, 1), 30, 40);
        let span = timeline
            .span(timeline.start, timeline.end)
            .expect("the whole window");

        assert_eq!((span.from, span.to), (0, 39));
    }

    #[test]
    fn a_column_never_runs_off_the_end_when_days_do_not_divide_evenly() {
        let timeline = Timeline::windowed(date(2026, 6, 1), 100, 7);

        assert_eq!(timeline.column_clamped(timeline.end), 6);
        assert_eq!(timeline.column_clamped(date(2030, 1, 1)), 6);
        assert_eq!(timeline.column_clamped(date(2020, 1, 1)), 0);
    }

    #[test]
    fn a_one_day_task_still_covers_a_column() {
        let timeline = Timeline::windowed(date(2026, 6, 1), 100, 10);
        let span = timeline.span(date(2026, 6, 5), date(2026, 6, 5)).expect("in view");

        assert_eq!(span.from, span.to);
        assert!(!span.clipped_left && !span.clipped_right);
    }

    #[test]
    fn a_span_reports_the_ends_that_run_past_the_window() {
        let timeline = Timeline::windowed(date(2026, 6, 1), 30, 30);

        let both = timeline
            .span(date(2026, 5, 1), date(2026, 8, 1))
            .expect("crosses the window");
        assert_eq!((both.from, both.to), (0, 29));
        assert!(both.clipped_left && both.clipped_right);

        let left = timeline
            .span(date(2026, 5, 1), date(2026, 6, 10))
            .expect("starts before");
        assert!(left.clipped_left && !left.clipped_right);
    }

    #[test]
    fn a_task_entirely_outside_the_window_has_no_span() {
        let timeline = Timeline::windowed(date(2026, 6, 1), 30, 30);

        assert_eq!(timeline.span(date(2026, 1, 1), date(2026, 2, 1)), None);
        assert_eq!(timeline.span(date(2026, 9, 1), date(2026, 9, 2)), None);
        assert_eq!(timeline.side_of(date(2026, 1, 1)), Some(true));
        assert_eq!(timeline.side_of(date(2026, 9, 1)), Some(false));
        assert_eq!(timeline.side_of(date(2026, 6, 5)), None);
    }

    #[test]
    fn reversed_dates_are_read_as_a_span_rather_than_dropped() {
        let timeline = Timeline::windowed(date(2026, 6, 1), 30, 30);
        let span = timeline
            .span(date(2026, 6, 20), date(2026, 6, 10))
            .expect("still in view");

        assert!(span.from < span.to);
    }

    #[test]
    fn a_fitted_window_can_never_clip_a_bar() {
        let dates = vec![date(2026, 6, 15), date(2026, 8, 3)];
        let timeline = Timeline::fit(dates, 40).expect("fits");
        let span = timeline
            .span(date(2026, 6, 15), date(2026, 8, 3))
            .expect("in view");

        assert!(!span.clipped_left && !span.clipped_right);
    }

    fn labels(timeline: &Timeline) -> Vec<String> {
        timeline
            .ticks()
            .into_iter()
            .map(|tick| tick.label)
            .collect()
    }

    #[test]
    fn a_long_span_is_marked_by_month() {
        // 92 days across 40 cells: a day gets less than half a cell.
        let timeline = Timeline::fit(vec![date(2026, 6, 15), date(2026, 8, 3)], 40)
            .expect("fits");

        assert_eq!(timeline.tick_scale(), TickScale::Month);
        assert_eq!(labels(&timeline), vec!["Jun", "Jul", "Aug"]);
        assert_eq!(timeline.ticks()[0].column, 0);
        assert!(timeline.ticks().iter().all(|tick| tick.emphasis));
    }

    #[test]
    fn a_month_or_so_is_marked_by_week() {
        // 30 days across 40 cells: room for a number every seven days.
        let timeline = Timeline::windowed(date(2026, 6, 1), 30, 40);

        assert_eq!(timeline.tick_scale(), TickScale::Week);
        // Jun 1 2026 is a Monday, so the weeks land on the 8th, 15th, 22nd
        // and 29th.
        assert_eq!(labels(&timeline), vec!["Jun", "8", "15", "22", "29"]);
    }

    #[test]
    fn a_fortnight_is_marked_by_day() {
        // 14 days across 50 cells: three cells each, room for a number.
        let timeline = Timeline::windowed(date(2026, 6, 10), 14, 50);

        assert_eq!(timeline.tick_scale(), TickScale::Day);
        assert_eq!(labels(&timeline).len(), 14);
        assert_eq!(labels(&timeline)[0], "Jun");
        assert_eq!(labels(&timeline)[1], "11");
    }

    #[test]
    fn a_week_with_room_to_spare_is_marked_by_weekday() {
        // 7 days across 60 cells: eight each, room for "Mon 11".
        let timeline = Timeline::windowed(date(2026, 6, 10), 7, 60);

        assert_eq!(timeline.tick_scale(), TickScale::Weekday);
        assert_eq!(
            labels(&timeline),
            vec!["Jun", "Thu 11", "Fri 12", "Sat 13", "Sun 14", "Mon 15", "Tue 16"],
        );
    }

    #[test]
    fn each_step_of_the_scale_adds_to_the_one_below_it() {
        // Zooming in must never trade the day number away for the weekday
        // name; every step is strictly more information.
        let day = Timeline::windowed(date(2026, 6, 10), 14, 50);
        let weekday = Timeline::windowed(date(2026, 6, 10), 7, 60);

        assert!(labels(&day)[1].contains("11"));
        assert!(labels(&weekday)[1].contains("11"));
        assert!(labels(&weekday)[1].contains("Thu"));
    }

    #[test]
    fn ticks_know_which_marks_fall_on_a_weekend() {
        let timeline = Timeline::windowed(date(2026, 6, 10), 7, 60);
        let weekend = timeline
            .ticks()
            .into_iter()
            .filter(|tick| tick.weekend)
            .map(|tick| tick.label)
            .collect::<Vec<_>>();

        assert_eq!(weekend, vec!["Sat 13", "Sun 14"]);
    }

    #[test]
    fn weekend_columns_cover_the_saturdays_and_sundays() {
        // Jun 13 and 14 2026 are the Saturday and Sunday of a 7-day window
        // starting Wednesday the 10th.
        let timeline = Timeline::windowed(date(2026, 6, 10), 7, 7);

        assert_eq!(timeline.weekend_columns(), vec![3, 4]);
    }

    #[test]
    fn a_weekend_claims_every_column_its_days_are_drawn_across() {
        // Three cells per day, so each weekend day is three columns wide and
        // a gap between them would look like a missing day.
        let timeline = Timeline::windowed(date(2026, 6, 10), 7, 21);
        let columns = timeline.weekend_columns();

        assert_eq!(columns, (9..=14).collect::<Vec<_>>());
    }

    #[test]
    fn a_column_covering_several_days_is_not_a_weekend() {
        // Below one cell per day there is no such thing as a weekend column,
        // and shading one would be a lie rather than a hint.
        let timeline = Timeline::windowed(date(2026, 6, 1), 365, 40);

        assert!(timeline.weekend_columns().is_empty());
    }

    #[test]
    fn weekend_columns_never_overlap_a_weekday() {
        // Every column a weekday is drawn on must stay unshaded, at any
        // resolution where weekends are shaded at all.
        for width in [7usize, 15, 21, 40, 60] {
            let timeline = Timeline::windowed(date(2026, 6, 10), 7, width);
            let weekend = timeline.weekend_columns();

            let mut cursor = timeline.start;
            while cursor <= timeline.end {
                if cursor.weekday_index() < 5 {
                    for column in
                        timeline.column_clamped(cursor)..=timeline.column_end_clamped(cursor)
                    {
                        assert!(
                            !weekend.contains(&column),
                            "column {column} is both {} and a weekend at width {width}",
                            cursor.weekday()
                        );
                    }
                }
                cursor = cursor.add_days(1);
            }
        }
    }

    #[test]
    fn the_scale_follows_the_pane_and_not_just_the_span() {
        // The same month reads differently in a narrow pane and a wide one.
        let narrow = Timeline::windowed(date(2026, 6, 1), 30, 20);
        let wide = Timeline::windowed(date(2026, 6, 1), 30, 100);

        assert_eq!(narrow.tick_scale(), TickScale::Month);
        assert_eq!(wide.tick_scale(), TickScale::Day);
    }

    #[test]
    fn a_month_boundary_is_named_at_every_scale() {
        // A row of bare day numbers with no month in sight says very little.
        for timeline in [
            Timeline::windowed(date(2026, 6, 25), 14, 60),
            Timeline::windowed(date(2026, 6, 20), 30, 40),
        ] {
            assert!(
                labels(&timeline).contains(&"Jul".to_string()),
                "{:?} at {:?}",
                labels(&timeline),
                timeline.tick_scale()
            );
        }
    }

    #[test]
    fn a_month_boundary_wins_a_column_it_has_to_share() {
        // At the edge of the week scale two marks can land together; dropping
        // the month name for a day number would lose the context.
        let timeline = Timeline::windowed(date(2026, 6, 29), 30, 30);
        let ticks = timeline.ticks();

        let july = ticks
            .iter()
            .find(|tick| tick.label == "Jul")
            .expect("July is marked");
        assert!(july.emphasis);
        assert_eq!(
            ticks.iter().filter(|tick| tick.column == july.column).count(),
            1
        );
    }

    /// A one-project table whose rows all sit inside a Jun-Aug window.
    fn table(records: Vec<TaskRecord>) -> TaskTableModel {
        TaskTableModel::from_records(
            records,
            vec![CustomFieldDefinition::new("cf-1", "Priority")],
        )
    }

    fn record(gid: &str, assignee: Option<&str>, start: Option<&str>, due: Option<&str>) -> TaskRecord {
        let mut record = TaskRecord::new(gid, format!("Task {gid}"));
        record.assignee = assignee.map(str::to_string);
        record.start_date = start.map(str::to_string);
        record.due_date = due.map(str::to_string);
        record
    }

    /// Shapes keyed by task id, because the table sorts its rows and a
    /// positional lookup would quietly test the sort instead of the chart.
    fn shapes_of(model: &GanttModel, table: &TaskTableModel) -> HashMap<String, TrackShape> {
        table
            .rows
            .iter()
            .zip(&model.tracks)
            .filter(|(row, _)| row.kind.is_task())
            .map(|(row, track)| (row.gid.clone(), track.shape))
            .collect()
    }

    fn build(table: &TaskTableModel, key: GanttColorKey, order: &[String]) -> GanttModel {
        GanttModel::build(table, &key, order, &TimelineView::Fit, 40, date(2026, 7, 15))
    }

    #[test]
    fn each_pairing_of_dates_produces_its_own_shape() {
        let table = table(vec![
            record("t1", None, Some("2026-06-10"), Some("2026-07-20")),
            record("t2", None, None, Some("2026-08-05")),
            record("t3", None, Some("2026-06-20"), None),
            record("t4", None, None, None),
        ]);
        let model = build(&table, GanttColorKey::Assignee, &[]);
        let shapes = shapes_of(&model, &table);

        assert!(matches!(shapes["t1"], TrackShape::Bar(span) if span.from < span.to));
        assert!(matches!(shapes["t2"], TrackShape::Milestone { .. }));
        assert!(
            matches!(shapes["t3"], TrackShape::Bar(span) if span.from == span.to),
            "a start with no due is a one-day bar, not a milestone"
        );
        assert_eq!(shapes["t4"], TrackShape::Empty);
    }

    #[test]
    fn reversed_dates_draw_the_span_between_them() {
        let table = table(vec![record("t1", None, Some("2026-08-01"), Some("2026-06-01"))]);
        let model = build(&table, GanttColorKey::Assignee, &[]);

        // Deliberately not a milestone: the bar covers both dates the API
        // sent, and the table's own columns still show which way round the
        // values are.
        assert!(matches!(shapes_of(&model, &table)["t1"], TrackShape::Bar(_)));
    }

    #[test]
    fn a_fitted_chart_never_pushes_a_task_off_the_window() {
        let table = table(vec![
            record("t1", None, Some("2024-01-01"), Some("2024-02-01")),
            record("t2", None, Some("2026-06-10"), Some("2026-07-20")),
        ]);
        let model = build(&table, GanttColorKey::Assignee, &[]);

        for shape in shapes_of(&model, &table).values() {
            assert!(
                matches!(shape, TrackShape::Bar(span) if !span.clipped_left && !span.clipped_right)
            );
        }
    }

    #[test]
    fn a_scrolled_window_clips_and_drops_the_tasks_outside_it() {
        let table = table(vec![
            record("t1", None, Some("2026-01-01"), Some("2026-02-01")),
            record("t2", None, Some("2026-05-01"), Some("2026-08-01")),
            record("t3", None, Some("2026-12-01"), Some("2026-12-20")),
        ]);
        let view = TimelineView::Window {
            start: date(2026, 6, 1),
            days: 30,
        };
        let model = GanttModel::build(
            &table,
            &GanttColorKey::Assignee,
            &[],
            &view,
            30,
            date(2026, 6, 15),
        );
        let shapes = shapes_of(&model, &table);

        assert_eq!(shapes["t1"], TrackShape::OffWindow { left: true });
        assert!(
            matches!(shapes["t2"], TrackShape::Bar(span) if span.clipped_left && span.clipped_right)
        );
        assert_eq!(shapes["t3"], TrackShape::OffWindow { left: false });
    }

    #[test]
    fn a_table_with_no_dates_produces_no_timeline_and_no_bars() {
        let table = table(vec![record("t1", Some("Alex"), None, None)]);
        let model = build(&table, GanttColorKey::Assignee, &[]);

        assert!(!model.has_timeline());
        assert_eq!(model.today_column, None);
        assert!(model.tracks.iter().all(|track| track.shape == TrackShape::Empty));
    }

    #[test]
    fn today_gets_a_column_only_when_it_falls_inside_the_window() {
        let table = table(vec![record("t1", None, Some("2026-06-10"), Some("2026-08-20"))]);

        let inside = build(&table, GanttColorKey::Assignee, &[]);
        assert!(inside.today_column.is_some());

        let outside = GanttModel::build(
            &table,
            &GanttColorKey::Assignee,
            &[],
            &TimelineView::Fit,
            40,
            date(2030, 1, 1),
        );
        assert_eq!(outside.today_column, None);
    }

    #[test]
    fn tracks_line_up_with_the_tables_rows_including_headers() {
        let mut with_group = record("t1", None, Some("2026-06-10"), Some("2026-07-20"));
        with_group.projects = vec!["Project".to_string()];
        with_group.sections = vec!["Section".to_string()];
        let table = table(vec![with_group]);
        let model = build(&table, GanttColorKey::Assignee, &[]);

        assert_eq!(model.tracks.len(), table.rows.len());
        for (row, track) in table.rows.iter().zip(&model.tracks) {
            if !row.kind.is_task() {
                assert_eq!(track.shape, TrackShape::Empty, "headers draw no bar");
            }
        }
    }

    #[test]
    fn each_dimension_reads_its_own_value_off_the_row() {
        let mut record = record("t1", Some("Alex Chen"), Some("2026-06-10"), Some("2026-07-20"));
        record.sections = vec!["Shipment".to_string()];
        record.custom_fields.insert(
            "cf-1".to_string(),
            vec!["High".to_string()],
        );
        let table = table(vec![record]);

        for (key, expected) in [
            (GanttColorKey::Assignee, "Alex Chen"),
            (GanttColorKey::Section, "Shipment"),
            (GanttColorKey::State, "open"),
            (GanttColorKey::Field("Priority".to_string()), "High"),
        ] {
            let model = build(&table, key.clone(), &[]);
            assert_eq!(
                model.assignment.ordered(),
                [expected.to_string()],
                "{key} read the wrong cell"
            );
        }
    }

    #[test]
    fn a_dimension_that_names_no_column_colours_nothing() {
        let table = table(vec![record("t1", Some("Alex"), Some("2026-06-10"), None)]);
        let model = build(&table, GanttColorKey::Field("Nonexistent".to_string()), &[]);

        assert!(model.assignment.ordered().is_empty());
        assert_eq!(model.tracks[0].slot, ColorSlot::Neutral);
    }

    #[test]
    fn a_completed_task_is_marked_so_its_bar_can_be_dimmed() {
        let mut done = record("t1", None, Some("2026-06-10"), Some("2026-07-20"));
        done.completed = true;
        let table = TaskTableModel::from_records_with_settings(
            vec![done],
            Vec::new(),
            &crate::domain::TaskTableSettings {
                filter: crate::domain::TaskFilter {
                    completed: None,
                    ..crate::domain::TaskFilter::default()
                },
                ..crate::domain::TaskTableSettings::default()
            },
        );
        let model = build(&table, GanttColorKey::Assignee, &[]);

        assert!(model.tracks.iter().any(|track| track.completed));
    }

    #[test]
    fn a_value_keeps_its_colour_when_the_table_is_re_sorted() {
        // The fallback is alphabetical rather than first-appearance precisely
        // so that re-sorting or filtering never repaints a bar.
        use crate::domain::{SortDirection, TaskSort, TaskSortField, TaskSortRule, TaskTableSettings};

        let records = vec![
            record("t1", Some("Zoe"), Some("2026-06-01"), Some("2026-06-10")),
            record("t2", Some("alice"), Some("2026-07-01"), Some("2026-07-10")),
            record("t3", Some("Bob"), Some("2026-08-01"), Some("2026-08-10")),
        ];

        let slots_under = |direction| {
            let settings = TaskTableSettings {
                sort: TaskSort {
                    rules: vec![TaskSortRule {
                        field: TaskSortField::Title,
                        direction,
                    }],
                    ..TaskSort::default()
                },
                ..TaskTableSettings::default()
            };
            let table =
                TaskTableModel::from_records_with_settings(records.clone(), Vec::new(), &settings);
            let model = build(&table, GanttColorKey::Assignee, &[]);
            ["Zoe", "alice", "Bob"]
                .map(|value| model.assignment.slot(value))
                .to_vec()
        };

        assert_eq!(slots_under(SortDirection::Asc), slots_under(SortDirection::Desc));
    }

    #[test]
    fn distinct_values_counts_every_value_and_puts_the_empty_one_last() {
        let table = table(vec![
            record("t1", Some("Bob"), None, None),
            record("t2", Some("alice"), None, None),
            record("t3", Some("alice"), None, None),
            record("t4", None, None, None),
        ]);

        let values = distinct_values(&table, &GanttColorKey::Assignee, &[]);

        assert_eq!(
            values,
            vec![
                ("alice".to_string(), 2),
                ("Bob".to_string(), 1),
                (String::new(), 1),
            ]
        );
    }

    #[test]
    fn distinct_values_follows_the_configured_order() {
        let table = table(vec![
            record("t1", Some("alice"), None, None),
            record("t2", Some("Bob"), None, None),
        ]);

        let values = distinct_values(
            &table,
            &GanttColorKey::Assignee,
            &strings(&["Bob", "alice"]),
        );

        assert_eq!(
            values.iter().map(|(value, _)| value.as_str()).collect::<Vec<_>>(),
            vec!["Bob", "alice"]
        );
    }
}
