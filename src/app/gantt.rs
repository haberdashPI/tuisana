//! Session state for the Gantt chart.
//!
//! This is the mutable half of the chart: what it is coloured by, how many
//! table columns it is sharing the pane with, and where the timeline is
//! pointing. The arithmetic all lives in [`crate::domain::gantt`]; this type
//! only remembers what the user has asked for.
//!
//! It is seeded from `[gantt]` at startup and, apart from the colour order,
//! never written back. Visibility, column count, and the timeline window are
//! view state of the same kind as sort and grouping, which have never
//! persisted.

use std::collections::BTreeMap;

use crate::{
    config::GanttConfig,
    domain::{CivilDate, ColorSlot, GanttColorKey, Timeline, TimelineView, PALETTE_SLOTS},
};

/// The window lengths `-` and `=` step between, in days.
///
/// A ladder rather than a multiplier so the steps are the spans people
/// actually think in -- a week, a fortnight, a month, a quarter, a year -- and
/// so zooming in and back out returns to where it started.
///
/// A week is the shortest rung because that is what puts weekday names on the
/// axis in a pane of ordinary width.
const ZOOM_LADDER: [u32; 9] = [7, 14, 30, 60, 90, 180, 365, 730, 1825];

/// The window keeps a breathing margin this fraction of itself in from the
/// left edge, and that is where zooming and `today` anchor.
///
/// Expressed as a fraction of the span rather than a column count, which comes
/// to the same thing and needs no plumbing: the window maps onto the pane
/// linearly, so a tenth of the span is always a tenth of the pane, at every
/// zoom and every terminal width.
const LEFT_MARGIN: u32 = 10;

/// Where a move sends the selected value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveTo {
    Up,
    Down,
    Top,
    Bottom,
}

/// One row of the colour dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderEntry {
    /// The value being coloured.
    pub value: String,
    /// How many task rows carry it.
    pub task_count: usize,
    /// Whether it can be reordered. The empty value cannot: it is always
    /// neutral, so letting it be dragged above the rule would be a lie.
    pub movable: bool,
}

/// The colour dialog's state.
///
/// Every move rewrites the live order immediately, so the chart behind the
/// modal recolours as the user works. `restore` holds the order the dialog
/// opened with, which is what makes `esc` a real cancel rather than a second
/// commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GanttOrderState {
    key: GanttColorKey,
    entries: Vec<OrderEntry>,
    selected: usize,
    restore: Option<Vec<String>>,
}

impl GanttOrderState {
    /// The dimension being ordered.
    pub fn key(&self) -> &GanttColorKey {
        &self.key
    }

    /// The rows to draw.
    pub fn entries(&self) -> &[OrderEntry] {
        &self.entries
    }

    /// Which row the cursor is on.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The slot an entry draws in, by its position among the movable values.
    pub fn slot(&self, index: usize) -> ColorSlot {
        match self.entries.get(index) {
            Some(entry) if entry.movable && index < PALETTE_SLOTS => ColorSlot::Indexed(index),
            _ => ColorSlot::Neutral,
        }
    }

    /// How many values can be reordered, which is where the palette rule goes.
    pub fn movable_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.movable).count()
    }

    fn moved_cursor(&self, delta: isize) -> usize {
        let last = self.entries.len().saturating_sub(1) as isize;
        (self.selected as isize + delta).clamp(0, last.max(0)) as usize
    }

    /// Moves the selected value, carrying the cursor with it.
    ///
    /// The cursor follows so that holding the key walks a value up the list
    /// rather than leaving the cursor behind after one step.
    fn move_value(&mut self, to: MoveTo) {
        let movable = self.movable_count();
        if !self.entries.get(self.selected).is_some_and(|e| e.movable) || movable < 2 {
            return;
        }

        let last = movable - 1;
        let target = match to {
            MoveTo::Up => self.selected.saturating_sub(1),
            MoveTo::Down => (self.selected + 1).min(last),
            MoveTo::Top => 0,
            MoveTo::Bottom => last,
        };

        let entry = self.entries.remove(self.selected);
        self.entries.insert(target, entry);
        self.selected = target;
    }

    /// The order to persist: what is on screen, then anything previously
    /// configured that is not.
    ///
    /// A value that lives in a project the user did not load this session must
    /// not be dropped from their config just for being absent.
    fn effective_order(&self) -> Vec<String> {
        let mut order = self
            .entries
            .iter()
            .filter(|entry| entry.movable)
            .map(|entry| entry.value.clone())
            .collect::<Vec<_>>();
        if let Some(previous) = &self.restore {
            let absent = previous
                .iter()
                .filter(|value| !order.contains(value))
                .cloned()
                .collect::<Vec<_>>();
            order.extend(absent);
        }
        order
    }
}

/// The chart's per-session state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GanttViewState {
    visible: bool,
    columns: usize,
    color_key: GanttColorKey,
    order: BTreeMap<String, Vec<String>>,
    timeline: TimelineView,
    dialog: Option<GanttOrderState>,
}

impl Default for GanttViewState {
    fn default() -> Self {
        Self::from_config(&GanttConfig::default())
    }
}

impl GanttViewState {
    /// Builds the starting state from config.
    pub fn from_config(config: &GanttConfig) -> Self {
        Self {
            visible: config.visible,
            columns: config.columns.max(1),
            color_key: config.color_key(),
            order: config.order.clone(),
            timeline: TimelineView::Fit,
            dialog: None,
        }
    }

    /// Whether the chart is drawn.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Draws or hides the chart.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Draws the chart if it is hidden, hides it if it is drawn.
    pub fn toggle_visible(&mut self) {
        self.visible = !self.visible;
    }

    /// How many table columns stay visible beside the chart.
    ///
    /// Clamped to at least the task title and at most every column the table
    /// has. `total` varies with the loaded custom fields, so the clamp happens
    /// on read rather than being baked in when the count changes.
    pub fn columns(&self, total: usize) -> usize {
        self.columns.clamp(1, total.max(1))
    }

    /// Shows one more table column, up to every column the table has.
    pub fn add_column(&mut self, total: usize) {
        self.columns = (self.columns(total) + 1).min(total.max(1));
    }

    /// Shows one fewer table column, never dropping the task title.
    pub fn remove_column(&mut self, total: usize) {
        self.columns = self.columns(total).saturating_sub(1).max(1);
    }

    /// Which dimension colours the bars.
    pub fn color_key(&self) -> &GanttColorKey {
        &self.color_key
    }

    /// Colours the bars by a different dimension.
    pub fn set_color_key(&mut self, key: GanttColorKey) {
        self.color_key = key;
    }

    /// The colour order for the current dimension, empty when unset.
    pub fn order(&self) -> &[String] {
        self.order_for(&self.color_key)
    }

    /// The colour order for one dimension, empty when unset.
    pub fn order_for(&self, key: &GanttColorKey) -> &[String] {
        self.order
            .get(&key.to_string())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Replaces the colour order for one dimension.
    pub fn set_order(&mut self, key: &GanttColorKey, values: Vec<String>) {
        self.order.insert(key.to_string(), values);
    }

    /// Every dimension's colour order, for persisting.
    pub fn orders(&self) -> &BTreeMap<String, Vec<String>> {
        &self.order
    }

    /// Where the timeline is pointing.
    pub fn timeline(&self) -> &TimelineView {
        &self.timeline
    }

    /// Whether the window has been moved off the fitted default.
    pub fn timeline_windowed(&self) -> bool {
        matches!(self.timeline, TimelineView::Window { .. })
    }

    /// Returns to fitting the loaded tasks.
    pub fn fit_timeline(&mut self) {
        self.timeline = TimelineView::Fit;
    }

    /// Moves the window by a fraction of its length.
    ///
    /// `fitted` is the window currently on screen, which is what a first
    /// scroll away from `Fit` has to start from -- otherwise the view would
    /// jump somewhere unrelated to what the user is looking at.
    pub fn scroll_timeline(&mut self, fitted: Option<Timeline>, forward: bool) {
        let (start, days) = self.window_or(fitted);
        let step = (days / 4).max(1) as i64;
        self.timeline = TimelineView::Window {
            start: start.add_days(if forward { step } else { -step }),
            days,
        };
    }

    /// Steps the zoom ladder, holding the left of the window.
    ///
    /// The date sitting at the left margin stays there, so zooming adds and
    /// removes time at the right: what you were looking at does not move, and
    /// later work comes into view or leaves it. Holding the centre instead
    /// slid the whole chart under the reader on every press.
    ///
    /// Zooming from `Fit` enters the ladder at the step nearest the span on
    /// screen, so the first press changes the scale by one notch rather than
    /// teleporting to whichever end of the ladder is closest.
    pub fn zoom_timeline(&mut self, fitted: Option<Timeline>, in_: bool) {
        let (start, days) = self.window_or(fitted);
        let nearest = ZOOM_LADDER
            .iter()
            .enumerate()
            .min_by_key(|(_, step)| step.abs_diff(days))
            .map(|(index, _)| index)
            .unwrap_or(0);

        let index = match (in_, self.timeline_windowed()) {
            // From Fit the nearest rung is itself a change of scale unless it
            // happens to match, so only step past it when it does not.
            (_, false) if ZOOM_LADDER[nearest] != days => nearest,
            (true, _) => nearest.saturating_sub(1),
            (false, _) => (nearest + 1).min(ZOOM_LADDER.len() - 1),
        };

        let next = ZOOM_LADDER[index];
        let anchor = start.add_days(margin_days(days));
        self.timeline = TimelineView::Window {
            start: anchor.add_days(-margin_days(next)),
            days: next,
        };
    }

    /// Brings a date to the left of the window, keeping the window's length.
    ///
    /// Placed at the margin rather than hard against the edge, so the day
    /// before it is still visible and the bar does not start flush with the
    /// divider.
    pub fn focus_timeline_on(&mut self, date: CivilDate, fitted: Option<Timeline>) {
        let (_, days) = self.window_or(fitted);
        self.timeline = TimelineView::Window {
            start: date.add_days(-margin_days(days)),
            days,
        };
    }

    /// Opens the colour dialog over the current dimension's values.
    pub fn open_dialog(&mut self, values: Vec<(String, usize)>) {
        let key = self.color_key.clone();
        let restore = self.order.get(&key.to_string()).cloned();
        self.dialog = Some(GanttOrderState {
            key,
            entries: entries_from(values),
            selected: 0,
            restore,
        });
    }

    /// Whether the colour dialog is open.
    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// The colour dialog, for the renderer.
    pub fn dialog(&self) -> Option<&GanttOrderState> {
        self.dialog.as_ref()
    }

    /// Moves the dialog's cursor.
    pub fn dialog_move_cursor(&mut self, delta: isize) {
        if let Some(dialog) = &mut self.dialog {
            dialog.selected = dialog.moved_cursor(delta);
        }
    }

    /// Moves the selected value and repaints the chart behind the modal.
    pub fn dialog_move_value(&mut self, to: MoveTo) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        dialog.move_value(to);
        let (key, order) = (dialog.key.to_string(), dialog.effective_order());
        self.order.insert(key, order);
    }

    /// Rebuilds the dialog around a different dimension, committing nothing.
    pub fn dialog_reload(&mut self, key: GanttColorKey, values: Vec<(String, usize)>) {
        let restore = self.order.get(&key.to_string()).cloned();
        if let Some(dialog) = &mut self.dialog {
            dialog.key = key;
            dialog.entries = entries_from(values);
            dialog.selected = 0;
            dialog.restore = restore;
        }
    }

    /// Closes the dialog, keeping the order as edited.
    pub fn dialog_commit(&mut self) {
        self.dialog = None;
    }

    /// Closes the dialog, putting back the order it opened with.
    pub fn dialog_cancel(&mut self) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let key = dialog.key.to_string();
        match dialog.restore {
            Some(order) => self.order.insert(key, order),
            // Nothing was configured before, so leave nothing behind rather
            // than an empty list that would serialize as `assignee = []`.
            None => self.order.remove(&key),
        };
    }

    /// The current window, falling back to what is on screen and then to a
    /// middle rung of the ladder when nothing is dated at all.
    fn window_or(&self, fitted: Option<Timeline>) -> (CivilDate, u32) {
        match self.timeline {
            TimelineView::Window { start, days } => (start, days),
            TimelineView::Fit => match fitted {
                Some(timeline) => (timeline.start, timeline.days()),
                None => (crate::domain::today(), ZOOM_LADDER[2]),
            },
        }
    }
}

/// How much of a window of `days` sits left of the anchor.
///
/// At least one day, so the margin does not vanish at the shortest zoom.
fn margin_days(days: u32) -> i64 {
    (days / LEFT_MARGIN).max(1) as i64
}

/// Turns counted values into dialog rows, pinning the empty value last.
fn entries_from(values: Vec<(String, usize)>) -> Vec<OrderEntry> {
    values
        .into_iter()
        .map(|(value, task_count)| OrderEntry {
            movable: !value.trim().is_empty(),
            value,
            task_count,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::GanttViewState;
    use crate::config::GanttConfig;
    use super::MoveTo;
    use crate::domain::{CivilDate, ColorSlot, GanttColorKey, Timeline, TimelineView};

    #[test]
    fn the_chart_starts_hidden_and_fitted() {
        let state = GanttViewState::default();

        assert!(!state.visible());
        assert_eq!(state.timeline(), &TimelineView::Fit);
        assert_eq!(state.color_key(), &GanttColorKey::Assignee);
    }

    #[test]
    fn config_seeds_the_starting_state() {
        let mut config = GanttConfig {
            visible: true,
            columns: 4,
            color_by: "section".to_string(),
            ..GanttConfig::default()
        };
        config
            .order
            .insert("section".to_string(), vec!["Shipment".to_string()]);

        let state = GanttViewState::from_config(&config);

        assert!(state.visible());
        assert_eq!(state.columns(7), 4);
        assert_eq!(state.color_key(), &GanttColorKey::Section);
        assert_eq!(state.order(), ["Shipment".to_string()]);
    }

    #[test]
    fn the_column_count_never_drops_the_title_or_exceeds_the_table() {
        let mut state = GanttViewState::default();

        for _ in 0..10 {
            state.remove_column(7);
        }
        assert_eq!(state.columns(7), 1, "the task title always stays");

        for _ in 0..20 {
            state.add_column(7);
        }
        assert_eq!(state.columns(7), 7);
    }

    #[test]
    fn a_column_count_is_clamped_to_whatever_the_table_currently_has() {
        // The custom-field columns come and go with the loaded projects, so a
        // count set against a wide table has to survive a narrow one.
        let mut state = GanttViewState::default();
        for _ in 0..6 {
            state.add_column(9);
        }
        assert_eq!(state.columns(9), 8);
        assert_eq!(state.columns(3), 3, "clamped down without being forgotten");
        assert_eq!(state.columns(9), 8, "and restored when the table is wide again");
    }

    #[test]
    fn each_dimension_keeps_its_own_order() {
        let mut state = GanttViewState::default();
        state.set_order(&GanttColorKey::Assignee, vec!["Alex".to_string()]);
        state.set_order(&GanttColorKey::Section, vec!["Shipment".to_string()]);

        assert_eq!(state.order(), ["Alex".to_string()]);
        state.set_color_key(GanttColorKey::Section);
        assert_eq!(state.order(), ["Shipment".to_string()]);
        assert_eq!(state.order_for(&GanttColorKey::State), [] as [String; 0]);
    }

    fn date(year: i32, month: u32, day: u32) -> CivilDate {
        CivilDate::new(year, month, day).expect("a real date")
    }

    /// A fitted Jun-Aug window, as the renderer would resolve it.
    fn fitted() -> Option<Timeline> {
        Timeline::fit(vec![date(2026, 6, 10), date(2026, 8, 20)], 1)
    }

    fn window(state: &GanttViewState) -> (CivilDate, u32) {
        match state.timeline() {
            TimelineView::Window { start, days } => (*start, *days),
            TimelineView::Fit => panic!("expected a moved window"),
        }
    }

    #[test]
    fn scrolling_moves_a_quarter_of_the_window_and_leaves_fit_behind() {
        let mut state = GanttViewState::default();
        assert!(!state.timeline_windowed());

        state.scroll_timeline(fitted(), true);

        let (start, days) = window(&state);
        assert!(state.timeline_windowed());
        assert_eq!(days, 92, "Jun 1 to Aug 31");
        assert_eq!(start, date(2026, 6, 1).add_days(23));
    }

    #[test]
    fn scrolling_back_and_forward_returns_to_the_same_place() {
        let mut state = GanttViewState::default();
        state.scroll_timeline(fitted(), true);
        let forward = window(&state);
        state.scroll_timeline(fitted(), false);
        state.scroll_timeline(fitted(), true);

        assert_eq!(window(&state), forward);
    }

    #[test]
    fn the_first_zoom_lands_on_the_ladder_rung_nearest_what_is_on_screen() {
        // 92 days on screen; the ladder's nearest rung is 90. Jumping to an
        // end of the ladder instead would lose the user's place entirely.
        let mut state = GanttViewState::default();
        state.zoom_timeline(fitted(), true);

        assert_eq!(window(&state).1, 90);
    }

    #[test]
    fn zooming_steps_one_rung_at_a_time_in_both_directions() {
        let mut state = GanttViewState::default();
        state.zoom_timeline(fitted(), true);
        assert_eq!(window(&state).1, 90);

        state.zoom_timeline(fitted(), true);
        assert_eq!(window(&state).1, 60);
        state.zoom_timeline(fitted(), true);
        assert_eq!(window(&state).1, 30);

        state.zoom_timeline(fitted(), false);
        assert_eq!(window(&state).1, 60);
    }

    /// Where the anchor sits in a window, as a fraction of its length.
    ///
    /// Because the window maps onto the pane linearly, this is also where the
    /// anchor sits on screen, which is the property that actually matters.
    fn anchor_fraction(state: &GanttViewState, anchor: CivilDate) -> f64 {
        let (start, days) = window(state);
        let offset = anchor.days_from_epoch() - start.days_from_epoch();
        offset as f64 / days as f64
    }

    #[test]
    fn zooming_keeps_the_date_at_the_left_margin_where_it_was() {
        let mut state = GanttViewState::default();
        let before = fitted().expect("a fitted window");
        // 92 days, so the margin is nine and the anchor is the tenth day.
        let anchor = before.start.add_days(9);

        for _ in 0..3 {
            state.zoom_timeline(fitted(), true);
            let placed = anchor_fraction(&state, anchor);
            assert!(
                (placed - 0.1).abs() < 0.06,
                "anchor drifted to {placed} of the way across"
            );
        }
    }

    #[test]
    fn zooming_adds_and_removes_time_at_the_right() {
        // The whole point: what you are looking at stays put and later work
        // comes into view or leaves it.
        let mut state = GanttViewState::default();
        state.zoom_timeline(fitted(), false);
        let (wide_start, wide_days) = window(&state);

        state.zoom_timeline(fitted(), true);
        let (narrow_start, narrow_days) = window(&state);

        assert!(narrow_days < wide_days);
        assert!(
            (narrow_start.days_from_epoch() - wide_start.days_from_epoch()).abs() < 20,
            "the left edge barely moved: {wide_start} then {narrow_start}"
        );
        assert!(
            narrow_start.add_days(narrow_days as i64)
                < wide_start.add_days(wide_days as i64),
            "the right edge is what pulled in"
        );
    }

    #[test]
    fn zooming_in_and_back_out_returns_to_the_same_window() {
        let mut state = GanttViewState::default();
        state.zoom_timeline(fitted(), true);
        let before = window(&state);

        state.zoom_timeline(fitted(), true);
        state.zoom_timeline(fitted(), false);

        assert_eq!(window(&state), before);
    }

    #[test]
    fn zooming_stops_at_both_ends_of_the_ladder() {
        let mut state = GanttViewState::default();
        for _ in 0..20 {
            state.zoom_timeline(fitted(), true);
        }
        assert_eq!(window(&state).1, 7);

        for _ in 0..20 {
            state.zoom_timeline(fitted(), false);
        }
        assert_eq!(window(&state).1, 1825);
    }

    #[test]
    fn focusing_a_date_keeps_the_span_and_puts_it_near_the_left() {
        let mut state = GanttViewState::default();
        state.zoom_timeline(fitted(), true);
        let span = window(&state).1;

        state.focus_timeline_on(date(2027, 3, 15), fitted());

        assert_eq!(window(&state).1, span, "focusing is not a zoom");
        let placed = anchor_fraction(&state, date(2027, 3, 15));
        assert!((placed - 0.1).abs() < 0.06, "landed at {placed}");
    }

    #[test]
    fn a_focused_date_is_not_flush_against_the_left_edge() {
        // A bar starting on the focused day would otherwise begin in the same
        // column as the divider, with no room to read it.
        let mut state = GanttViewState::default();
        state.focus_timeline_on(date(2027, 3, 15), fitted());

        assert!(window(&state).0 < date(2027, 3, 15));
    }

    #[test]
    fn the_margin_survives_the_shortest_zoom() {
        let mut state = GanttViewState::default();
        for _ in 0..20 {
            state.zoom_timeline(fitted(), true);
        }
        assert_eq!(window(&state).1, 7);

        state.focus_timeline_on(date(2027, 3, 15), fitted());
        assert!(
            window(&state).0 < date(2027, 3, 15),
            "a week-long window still leaves a day of margin"
        );
    }

    #[test]
    fn fitting_returns_to_following_the_data() {
        let mut state = GanttViewState::default();
        state.scroll_timeline(fitted(), true);
        state.fit_timeline();

        assert_eq!(state.timeline(), &TimelineView::Fit);
        assert!(!state.timeline_windowed());
    }

    #[test]
    fn scrolling_a_chart_with_no_dated_rows_still_produces_a_window() {
        // Nothing to fit to, so there is no natural span to start from. A
        // panic or a zero-length window here would be worse than a default.
        let mut state = GanttViewState::default();
        state.scroll_timeline(None, true);

        assert!(window(&state).1 > 0);
    }

    fn values(names: &[&str]) -> Vec<(String, usize)> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.to_string(), index + 1))
            .collect()
    }

    fn shown(state: &GanttViewState) -> Vec<String> {
        state
            .dialog()
            .expect("the dialog is open")
            .entries()
            .iter()
            .map(|entry| entry.value.clone())
            .collect()
    }

    fn open_with(names: &[&str]) -> GanttViewState {
        let mut state = GanttViewState::default();
        state.open_dialog(values(names));
        state
    }

    #[test]
    fn moving_a_value_carries_the_cursor_with_it() {
        let mut state = open_with(&["a", "b", "c"]);
        state.dialog_move_cursor(2);
        state.dialog_move_value(MoveTo::Up);

        assert_eq!(shown(&state), ["a", "c", "b"]);
        assert_eq!(
            state.dialog().expect("open").selected(),
            1,
            "holding the key has to keep walking the same value"
        );
    }

    #[test]
    fn the_four_moves_each_land_where_they_say() {
        let mut state = open_with(&["a", "b", "c", "d"]);
        state.dialog_move_cursor(2);

        state.dialog_move_value(MoveTo::Top);
        assert_eq!(shown(&state), ["c", "a", "b", "d"]);

        state.dialog_move_value(MoveTo::Bottom);
        assert_eq!(shown(&state), ["a", "b", "d", "c"]);

        state.dialog_move_value(MoveTo::Up);
        assert_eq!(shown(&state), ["a", "b", "c", "d"]);

        state.dialog_move_value(MoveTo::Down);
        assert_eq!(shown(&state), ["a", "b", "d", "c"]);
    }

    #[test]
    fn moving_past_either_end_does_nothing() {
        let mut state = open_with(&["a", "b"]);

        state.dialog_move_value(MoveTo::Up);
        assert_eq!(shown(&state), ["a", "b"]);

        state.dialog_move_cursor(1);
        state.dialog_move_value(MoveTo::Down);
        assert_eq!(shown(&state), ["a", "b"]);
    }

    #[test]
    fn the_empty_value_stays_last_and_refuses_to_move() {
        let mut state = GanttViewState::default();
        state.open_dialog(vec![
            ("a".to_string(), 1),
            ("b".to_string(), 2),
            (String::new(), 9),
        ]);
        state.dialog_move_cursor(2);
        state.dialog_move_value(MoveTo::Top);

        assert_eq!(shown(&state), ["a", "b", ""]);
        let dialog = state.dialog().expect("open");
        assert!(!dialog.entries()[2].movable);
        assert_eq!(dialog.slot(2), ColorSlot::Neutral);
    }

    #[test]
    fn only_the_first_six_movable_values_get_a_slot() {
        let state = open_with(&["a", "b", "c", "d", "e", "f", "g"]);
        let dialog = state.dialog().expect("open");

        assert_eq!(dialog.slot(5), ColorSlot::Indexed(5));
        assert_eq!(dialog.slot(6), ColorSlot::Neutral);
    }

    #[test]
    fn every_move_rewrites_the_live_order_so_the_chart_repaints() {
        let mut state = open_with(&["a", "b", "c"]);
        assert!(state.order().is_empty(), "nothing configured yet");

        state.dialog_move_cursor(2);
        state.dialog_move_value(MoveTo::Top);

        assert_eq!(state.order(), ["c".to_string(), "a".to_string(), "b".to_string()]);
    }

    #[test]
    fn cancelling_puts_back_the_order_the_dialog_opened_with() {
        let mut state = GanttViewState::default();
        state.set_order(&GanttColorKey::Assignee, vec!["b".to_string(), "a".to_string()]);
        state.open_dialog(values(&["a", "b"]));

        state.dialog_move_value(MoveTo::Bottom);
        assert_eq!(state.order(), ["b".to_string(), "a".to_string()]);

        state.dialog_cancel();
        assert_eq!(state.order(), ["b".to_string(), "a".to_string()]);
        assert!(!state.dialog_open());
    }

    #[test]
    fn cancelling_an_unconfigured_dimension_leaves_no_entry_behind() {
        // An empty list would serialize as `assignee = []`, which reads as a
        // deliberate choice rather than as the absence of one.
        let mut state = open_with(&["a", "b"]);
        state.dialog_move_value(MoveTo::Bottom);
        state.dialog_cancel();

        assert!(state.orders().is_empty());
    }

    #[test]
    fn committing_keeps_the_edited_order() {
        let mut state = open_with(&["a", "b"]);
        state.dialog_move_value(MoveTo::Bottom);
        state.dialog_commit();

        assert!(!state.dialog_open());
        assert_eq!(state.order(), ["b".to_string(), "a".to_string()]);
    }

    #[test]
    fn a_configured_value_that_is_not_on_screen_survives_a_reorder() {
        // It lives in a project this session did not load. Dropping it from
        // the config for being absent would lose work silently.
        let mut state = GanttViewState::default();
        state.set_order(
            &GanttColorKey::Assignee,
            vec!["absent".to_string(), "a".to_string()],
        );
        state.open_dialog(values(&["a", "b"]));
        state.dialog_move_value(MoveTo::Bottom);
        state.dialog_commit();

        assert!(state.order().contains(&"absent".to_string()));
    }

    #[test]
    fn switching_dimension_inside_the_dialog_rebuilds_it_and_commits_nothing() {
        let mut state = open_with(&["a", "b"]);
        state.dialog_move_cursor(1);

        state.set_color_key(GanttColorKey::Section);
        state.dialog_reload(GanttColorKey::Section, values(&["x", "y", "z"]));

        let dialog = state.dialog().expect("still open");
        assert_eq!(dialog.key(), &GanttColorKey::Section);
        assert_eq!(dialog.selected(), 0);
        assert_eq!(shown(&state), ["x", "y", "z"]);
    }
}

