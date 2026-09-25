//! State for the calendar date picker overlay.
//!
//! Date fields used to be edited as a bare append-only text buffer, which meant
//! knowing the grammar by heart and never seeing what weekday a date lands on.
//! This is the state behind the overlay that replaces that: a month grid with a
//! highlighted day, driven by either the navigation keys or by typing.
//!
//! Three things shape the design:
//!
//! - **The text is the filter field.** Typed characters go into the query the
//!   filter panel shows, not into a buffer hidden in the overlay, and the table
//!   refilters as they land. The overlay is a view onto that text plus a way to
//!   move it around.
//! - **Text and grid stay in step, in both directions.** Typing `2026-09` moves
//!   the grid to September even though no day is named yet; moving the highlight
//!   rewrites the text. Whichever the user reaches for, the other follows.
//! - **A range has two sides, and only one is active.** `2026-09-01..2026-09-30`
//!   is edited one end at a time: the caret decides which end the navigation keys
//!   rewrite, and the other end is left alone.

use crate::domain::{date, CivilDate};

/// The separator between the two ends of a range query.
const RANGE: &str = "..";

/// Which end of a range query the caret is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    /// There is no range; the whole query is one date.
    Whole,
    /// The text before `..`.
    Start,
    /// The text after `..`.
    End,
}

/// The picker's state while a date field is being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CalendarState {
    /// The field being edited, shown as the overlay title.
    field_label: String,
    /// The query being edited. Mirrored into the filter field on every change.
    query: String,
    /// The caret, as a char index into `query`.
    caret: usize,
    /// The month on display. Always the first of the month.
    visible_month: CivilDate,
    /// Today, resolved once on open so the grid cannot shift mid-edit.
    today: CivilDate,
    /// Whether the field being edited can hold a range at all.
    ///
    /// A filter can; a task's due date is one day, and the API has nowhere to
    /// put a second. It is what decides whether a bare keyword covering more
    /// than one day is edited as the end of a range or collapsed to the single
    /// day it would commit as.
    ranges: bool,
}

impl CalendarState {
    /// Opens the picker on a filter field holding `query`, caret at the end.
    ///
    /// The grid starts on the month the query's active end names, so reopening a
    /// set filter shows where you left off. An empty or unreadable query starts
    /// on the current month.
    pub fn open(field_label: impl Into<String>, query: &str, today: CivilDate) -> Self {
        Self::new(field_label, query, today, true)
    }

    /// Opens the picker on a field that holds one day and no range.
    ///
    /// A task's due or start date. The difference is only felt on a keyword
    /// covering more than one day: there is no second end for it to be edited
    /// as, so it stays the single date it commits as.
    pub fn open_one_day(field_label: impl Into<String>, value: &str, today: CivilDate) -> Self {
        Self::new(field_label, value, today, false)
    }

    fn new(
        field_label: impl Into<String>,
        query: &str,
        today: CivilDate,
        ranges: bool,
    ) -> Self {
        let query = query.trim().to_string();
        let caret = query.chars().count();
        let mut state = Self {
            field_label: field_label.into(),
            query,
            caret,
            visible_month: today.first_of_month(),
            today,
            ranges,
        };
        state.follow_text();
        state
    }

    /// The field being edited.
    pub fn field_label(&self) -> &str {
        &self.field_label
    }

    /// The query, for writing back into the filter field.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The caret position, as a char index, for drawing it in the filter panel.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The month on display.
    pub fn visible_month(&self) -> CivilDate {
        self.visible_month
    }

    /// Today, as resolved when the picker opened.
    pub fn today(&self) -> CivilDate {
        self.today
    }

    /// Which end of a range the caret is in.
    pub fn side(&self) -> Side {
        match self.separator() {
            None => Side::Whole,
            Some((start, _)) if self.caret <= start => Side::Start,
            Some(_) => Side::End,
        }
    }

    /// The day the highlight sits on, when the active end names one.
    ///
    /// A name covering more than one day has two edges, and which one the
    /// highlight takes is the edge that end of the range contributes: the
    /// first day before the `..`, the last day after it. So the caret sitting
    /// in `this week` of `this week..next week` puts the highlight on the
    /// Sunday that opens the range, and moving it into `next week` puts the
    /// highlight on the Saturday that closes it — in both cases on the day the
    /// keys would actually be moving.
    ///
    /// A bare keyword with no `..` is treated as the end on a field that takes
    /// a range, because that is the edge a picked date replaces; see
    /// [`set_active_date`](Self::set_active_date).
    pub fn active_date(&self) -> Option<CivilDate> {
        let span = self.span_in(self.active_text())?;
        Some(match self.side() {
            Side::Start => span.start(),
            Side::End => span.end(),
            // No `..` to say which edge is meant. On a filter the end is
            // assumed, because that is the edge a picked date replaces; on a
            // one-day field there is no end to assume, and the highlight has
            // to sit on the day the field would actually commit.
            Side::Whole => match self.ranges {
                true => span.end(),
                false => span.start(),
            },
        })
    }

    /// The range's two ends, or `None` when the query is not a range.
    ///
    /// Either end may be absent, both because a range can be left open and
    /// because it may still be half-typed.
    ///
    /// Each end contributes the *outside* of whatever span it names: the first
    /// day of the one before the `..` and the last day of the one after. That
    /// is what makes `last week..this week` reach from the Sunday that starts
    /// last week to the Saturday that ends this one, rather than stopping on
    /// the Sunday `this week` begins on. [`DateQuery::parse`] reads a written
    /// range the same way, and this is what the grid shades, so the two have
    /// to agree — otherwise the overlay draws a narrower range than the filter
    /// is actually matching.
    pub fn range(&self) -> Option<(Option<CivilDate>, Option<CivilDate>)> {
        let (start, end) = self.query.split_once(RANGE)?;
        Some((
            self.span_in(start).map(|span| span.start()),
            self.span_in(end).map(|span| span.end()),
        ))
    }

    /// The days the grid should shade, or `None` when there is nothing to
    /// shade.
    ///
    /// A superset of [`range`](Self::range): a written range shades between
    /// its two ends, and so does a single token that names more than one day,
    /// because `this month` covers the month whether or not a `..` says so. A
    /// token naming one day shades nothing — that is what the highlight is
    /// for.
    pub fn span(&self) -> Option<(Option<CivilDate>, Option<CivilDate>)> {
        if let Some(range) = self.range() {
            return Some(range);
        }
        let span = date::parse_span(self.active_text(), self.today).flatten()?;
        (!span.is_day()).then(|| (Some(span.start()), Some(span.end())))
    }

    /// The whole query as the filter will read it, for the collapsed summary.
    ///
    /// The whole query and not the active end: with the grid hidden this line
    /// is the only view of the value, and half of a range is not a value.
    pub fn resolution(&self) -> Option<crate::domain::DateQuery> {
        crate::domain::DateQuery::parse(&self.query, self.today)
    }

    /// Whether the query is a date expression the filter can use.
    ///
    /// An empty query is fine — it is simply not filtering. Anything else that
    /// fails to parse would silently empty the table, so the overlay says so.
    pub fn parses(&self) -> bool {
        self.query.trim().is_empty()
            || crate::domain::DateQuery::parse(&self.query, self.today).is_some()
    }

    /// Empties the query, leaving the visible month where it is.
    ///
    /// The grid deliberately stays put: clearing a date is usually the first
    /// half of picking a different one, and jumping back to today would throw
    /// away the month the user had already navigated to.
    pub fn clear(&mut self) {
        self.query.clear();
        self.caret = 0;
    }

    /// Moves the highlight by whole days, rewriting the active end.
    ///
    /// Stepping off the end of a month lands on the first of the next one, since
    /// this counts days rather than clamping within the month.
    pub fn move_days(&mut self, delta: i64) {
        let base = self.active_date().unwrap_or_else(|| self.fallback_date());
        self.set_active_date(base.add_days(delta));
    }

    /// Flips the displayed month, rewriting the active end.
    ///
    /// Forward lands on the first of the next month and backward on the last of
    /// the previous one, so a flip always lands on a real edge of the month
    /// rather than carrying the old day number along.
    pub fn move_months(&mut self, delta: i64) {
        let month = self.visible_month.add_months(delta);
        let day = if delta < 0 { month.days_in_month() } else { 1 };
        self.set_active_date(CivilDate {
            day,
            ..month
        });
    }

    /// Jumps the highlight to today, rewriting the active end.
    pub fn jump_today(&mut self) {
        self.set_active_date(self.today);
    }

    /// Inserts a typed character at the caret.
    pub fn push_char(&mut self, ch: char) {
        let mut chars = self.chars();
        let at = self.caret.min(chars.len());
        chars.insert(at, ch);
        self.query = chars.into_iter().collect();
        self.caret = at + 1;
        self.follow_text();
    }

    /// Deletes the character before the caret.
    pub fn pop_char(&mut self) {
        if self.caret == 0 {
            return;
        }
        let mut chars = self.chars();
        let at = self.caret.min(chars.len());
        if at == 0 {
            return;
        }
        chars.remove(at - 1);
        self.query = chars.into_iter().collect();
        self.caret = at - 1;
        self.follow_text();
    }

    /// Moves the caret one character, clamped to the query.
    pub fn move_caret(&mut self, delta: i64) {
        let len = self.chars().len();
        self.caret = (self.caret as i64 + delta).clamp(0, len as i64) as usize;
        self.follow_text();
    }

    /// Puts the caret on the range's start end, at the end of its text.
    ///
    /// Answers whether it moved, which is `false` when the query is not a range
    /// and there is no other end to jump to.
    pub fn jump_to_start(&mut self) -> bool {
        let Some((start, _)) = self.separator() else {
            return false;
        };
        self.caret = start;
        self.follow_text();
        true
    }

    /// Puts the caret on the range's end end, after its text.
    pub fn jump_to_end(&mut self) -> bool {
        if self.separator().is_none() {
            return false;
        }
        self.caret = self.chars().len();
        self.follow_text();
        true
    }

    /// Fills in the highlighted day when the active end does not already name a
    /// date, so committing an untouched picker still picks something.
    ///
    /// A complete date is left exactly as typed, keywords included: rewriting
    /// `today` to an ISO date would throw away the part the user chose.
    pub fn normalize(&mut self) {
        if date::parse_token(self.active_text(), self.today)
            .flatten()
            .is_some()
        {
            return;
        }
        let date = self.active_date().unwrap_or_else(|| self.fallback_date());
        self.set_active_date(date);
    }

    /// The month laid out as weeks starting Sunday.
    ///
    /// Leading and trailing cells are `None` rather than spilling into the
    /// neighboring months, which keeps the grid readable at a glance.
    pub fn weeks(&self) -> Vec<[Option<u32>; 7]> {
        let first = self.visible_month.first_of_month();
        let offset = first.calendar_column();
        let days = first.days_in_month();

        let mut weeks = Vec::new();
        let mut week = [None; 7];
        for day in 1..=days {
            let index = (offset + (day as usize - 1)) % 7;
            week[index] = Some(day);
            if index == 6 {
                weeks.push(week);
                week = [None; 7];
            }
        }
        if week.iter().any(Option::is_some) {
            weeks.push(week);
        }
        weeks
    }

    /// The query as chars, which is what the caret indexes.
    fn chars(&self) -> Vec<char> {
        self.query.chars().collect()
    }

    /// The char range the `..` separator occupies, if there is one.
    fn separator(&self) -> Option<(usize, usize)> {
        let byte = self.query.find(RANGE)?;
        let start = self.query[..byte].chars().count();
        Some((start, start + RANGE.len()))
    }

    /// The char range of the end the caret is in.
    fn active_span(&self) -> (usize, usize) {
        let len = self.chars().len();
        match self.side() {
            Side::Whole => (0, len),
            Side::Start => (0, self.separator().expect("a range has a separator").0),
            Side::End => (self.separator().expect("a range has a separator").1, len),
        }
    }

    /// The text of the end the caret is in.
    fn active_text(&self) -> &str {
        let (start, end) = self.active_span();
        let bytes = |index: usize| {
            self.query
                .char_indices()
                .nth(index)
                .map_or(self.query.len(), |(byte, _)| byte)
        };
        self.query[bytes(start)..bytes(end)].trim()
    }

    /// The complete date named by one side's text, if any.
    ///
    /// The first day of its span, so a name covering more than one collapses
    /// the way it does everywhere a single date is wanted. [`range`](Self::range)
    /// deliberately does not use this: a range wants each end's outer edge.
    fn date_in(&self, text: &str) -> Option<CivilDate> {
        date::parse_partial(text, self.today).and_then(|partial| partial.day)
    }

    /// The days one side's text names, if it names any yet.
    fn span_in(&self, text: &str) -> Option<date::DateSpan> {
        date::parse_span(text, self.today).flatten()
    }

    /// The date named by the end of the range the caret is *not* in.
    fn other_end_date(&self) -> Option<CivilDate> {
        let (start, end) = self.query.split_once(RANGE)?;
        match self.side() {
            Side::Start => self.date_in(end),
            Side::End => self.date_in(start),
            Side::Whole => None,
        }
    }

    /// Where a movement starts from when the active end names no date yet.
    ///
    /// The other end of the range first, so filling in `2026-09-01..` starts
    /// next to the start date instead of jumping back to today's month. Failing
    /// that, today when today is on screen, and otherwise the first of whatever
    /// month the user has navigated to.
    fn fallback_date(&self) -> CivilDate {
        if let Some(other) = self.other_end_date() {
            return other;
        }
        if self.visible_month == self.today.first_of_month() {
            self.today
        } else {
            self.visible_month
        }
    }

    /// Replaces the active end's text with a date and shows its month.
    ///
    /// A bare keyword covering more than one day is spelled out as a range
    /// first. `this week` moved on a day has to become
    /// `2026-08-23..2026-08-30`, because the keyword has nowhere to put "but
    /// ending a day later" — and overwriting the whole of it with the one
    /// picked date would throw the week's start away, which is not what
    /// nudging one edge of it means. The highlight sits on the end edge for
    /// exactly this reason, so it is the end the picked date lands on.
    fn set_active_date(&mut self, date: CivilDate) {
        if self.ranges && self.side() == Side::Whole {
            let span = self.span_in(&self.query);
            if let Some(span) = span.filter(|span| !span.is_day()) {
                let start = span.start().iso();
                let end = date.iso();
                self.caret = start.chars().count() + RANGE.chars().count() + end.chars().count();
                self.query = format!("{start}{RANGE}{end}");
                self.visible_month = date.first_of_month();
                return;
            }
        }

        let (start, end) = self.active_span();
        let chars = self.chars();
        let head = chars[..start.min(chars.len())].iter().collect::<String>();
        let tail = chars[end.min(chars.len())..].iter().collect::<String>();
        let iso = date.iso();

        self.caret = head.chars().count() + iso.chars().count();
        self.query = format!("{head}{iso}{tail}");
        self.visible_month = date.first_of_month();
    }

    /// Moves the grid to whatever month the active end's text points at.
    ///
    /// Called after every text or caret change, so a half-typed `2026-09` shows
    /// September and jumping to the other end of a range shows that end's month.
    /// Text that says nothing usable leaves the grid where it is.
    fn follow_text(&mut self) {
        // The highlight's own month first, so a keyword whose active edge
        // falls outside the month it starts in — `next week` can end in the
        // next one — shows the square the highlight is actually on.
        if let Some(date) = self.active_date() {
            self.visible_month = date.first_of_month();
        } else if let Some(partial) = date::parse_partial(self.active_text(), self.today) {
            self.visible_month = partial.month;
        } else if let Some(other) = self.other_end_date() {
            // An empty end of a range shows the other end's month, so opening
            // `2026-09-01..` to fill in the finish starts in September.
            self.visible_month = other.first_of_month();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CalendarState, Side};
    use crate::domain::CivilDate;

    fn date(year: i32, month: u32, day: u32) -> CivilDate {
        CivilDate::new(year, month, day).expect("a real date")
    }

    /// A Monday, matching the snapshot harness's pinned date.
    fn today() -> CivilDate {
        date(2026, 8, 24)
    }

    fn open(query: &str) -> CalendarState {
        CalendarState::open("Due", query, today())
    }

    fn type_text(state: &mut CalendarState, text: &str) {
        for ch in text.chars() {
            state.push_char(ch);
        }
    }

    #[test]
    fn an_empty_field_opens_on_the_current_month_with_today_highlighted() {
        let state = open("");

        assert_eq!(state.visible_month(), date(2026, 8, 1));
        assert_eq!(state.active_date(), None, "nothing is typed, so nothing is set");
        assert_eq!(state.query(), "");
        assert_eq!(state.caret(), 0);
        assert_eq!(state.field_label(), "Due");
        assert_eq!(state.side(), Side::Whole);
    }

    #[test]
    fn a_field_holding_a_date_opens_on_that_month() {
        let state = open("2026-03-04");

        assert_eq!(state.visible_month(), date(2026, 3, 1));
        assert_eq!(state.active_date(), Some(date(2026, 3, 4)));
        assert_eq!(state.caret(), 10, "the caret starts after the text");
    }

    #[test]
    fn typed_text_lands_in_the_query_and_moves_the_grid_with_it() {
        let mut state = open("");
        type_text(&mut state, "2026");

        assert_eq!(state.query(), "2026");
        assert_eq!(
            state.visible_month(),
            date(2026, 1, 1),
            "a year alone shows January"
        );
        assert_eq!(state.active_date(), None, "and highlights no day");

        type_text(&mut state, "-09");
        assert_eq!(state.visible_month(), date(2026, 9, 1));
        assert_eq!(state.active_date(), None);

        type_text(&mut state, "-15");
        assert_eq!(state.query(), "2026-09-15");
        assert_eq!(state.visible_month(), date(2026, 9, 1));
        assert_eq!(state.active_date(), Some(date(2026, 9, 15)));
        assert!(state.parses());
    }

    #[test]
    fn a_month_typed_alone_shows_that_month_of_the_current_year() {
        let mut state = open("");
        type_text(&mut state, "09");

        assert_eq!(state.visible_month(), date(2026, 9, 1));
        assert_eq!(state.active_date(), None);
    }

    #[test]
    fn text_that_says_nothing_usable_leaves_the_grid_alone() {
        let mut state = open("2026-09-15");
        type_text(&mut state, "zz");

        assert_eq!(state.query(), "2026-09-15zz");
        assert_eq!(state.visible_month(), date(2026, 9, 1));
        assert!(!state.parses(), "but the overlay says the text is unusable");
    }

    #[test]
    fn the_caret_moves_by_character_and_backspace_deletes_in_place() {
        let mut state = open("2026-09-15");

        state.move_caret(-1);
        assert_eq!(state.caret(), 9);
        state.pop_char();
        assert_eq!(state.query(), "2026-09-5", "the character before the caret goes");
        assert_eq!(state.caret(), 8);

        state.push_char('2');
        assert_eq!(state.query(), "2026-09-25");
        assert_eq!(state.active_date(), Some(date(2026, 9, 25)));
    }

    #[test]
    fn the_caret_stops_at_both_ends_of_the_query() {
        let mut state = open("2026");

        state.move_caret(-99);
        assert_eq!(state.caret(), 0);
        state.pop_char();
        assert_eq!(state.query(), "2026", "backspace at the start does nothing");

        state.move_caret(99);
        assert_eq!(state.caret(), 4);
    }

    #[test]
    fn flipping_forward_lands_on_the_first_and_backward_on_the_last() {
        let mut state = open("2026-08-24");

        state.move_months(1);
        assert_eq!(state.active_date(), Some(date(2026, 9, 1)));
        assert_eq!(state.query(), "2026-09-01");

        state.move_months(-1);
        assert_eq!(
            state.active_date(),
            Some(date(2026, 8, 31)),
            "flipping back lands on the end of the month"
        );

        state.move_months(-1);
        assert_eq!(state.active_date(), Some(date(2026, 7, 31)));
        state.move_months(-1);
        assert_eq!(state.active_date(), Some(date(2026, 6, 30)));
    }

    #[test]
    fn flipping_months_crosses_years_in_both_directions() {
        let mut state = open("2026-01-15");

        state.move_months(-1);
        assert_eq!(state.active_date(), Some(date(2025, 12, 31)));

        for _ in 0..13 {
            state.move_months(1);
        }
        assert_eq!(state.active_date(), Some(date(2027, 1, 1)));
    }

    #[test]
    fn stepping_a_day_off_the_end_of_a_month_lands_on_the_next_first() {
        let mut state = open("2026-08-31");

        state.move_days(1);
        assert_eq!(state.active_date(), Some(date(2026, 9, 1)));
        assert_eq!(state.visible_month(), date(2026, 9, 1));

        state.move_days(-1);
        assert_eq!(state.active_date(), Some(date(2026, 8, 31)));
        assert_eq!(state.visible_month(), date(2026, 8, 1));
    }

    #[test]
    fn navigating_from_an_empty_field_starts_at_today() {
        let mut state = open("");

        state.move_days(1);
        assert_eq!(state.active_date(), Some(date(2026, 8, 25)));
        assert_eq!(state.query(), "2026-08-25");
    }

    #[test]
    fn navigating_replaces_whatever_was_typed_on_the_active_side() {
        let mut state = open("");
        type_text(&mut state, "2026-09");

        state.move_days(1);
        assert_eq!(
            state.query(),
            "2026-09-02",
            "the half-typed month became a date, stepped one day on"
        );
    }

    #[test]
    fn jumping_to_today_returns_the_grid_to_the_current_month() {
        let mut state = open("2027-01-04");
        state.jump_today();

        assert_eq!(state.active_date(), Some(today()));
        assert_eq!(state.visible_month(), date(2026, 8, 1));
        assert_eq!(state.query(), "2026-08-24");
    }

    #[test]
    fn the_caret_decides_which_end_of_a_range_navigation_rewrites() {
        let mut state = open("2026-09-01..2026-09-30");

        assert_eq!(state.side(), Side::End, "the caret starts after the text");
        state.move_days(1);
        assert_eq!(state.query(), "2026-09-01..2026-10-01");

        assert!(state.jump_to_start());
        assert_eq!(state.side(), Side::Start);
        assert_eq!(
            state.visible_month(),
            date(2026, 9, 1),
            "and the grid follows to that end's month"
        );

        state.move_days(-1);
        assert_eq!(
            state.query(),
            "2026-08-31..2026-10-01",
            "only the start moved"
        );
    }

    #[test]
    fn jumping_between_ends_does_nothing_without_a_range() {
        let mut state = open("2026-09-01");

        assert!(!state.jump_to_start());
        assert!(!state.jump_to_end());
        assert_eq!(state.caret(), 10, "and the caret stays put");
    }

    #[test]
    fn jumping_to_the_end_puts_the_caret_after_the_last_character() {
        let mut state = open("2026-09-01..2026-09-30");
        state.jump_to_start();

        assert!(state.jump_to_end());
        assert_eq!(state.caret(), 22);
        assert_eq!(state.side(), Side::End);
    }

    #[test]
    fn an_open_end_of_a_range_starts_from_the_other_end() {
        let mut state = open("");
        type_text(&mut state, "2026-09-01..");

        assert_eq!(state.side(), Side::End);
        assert_eq!(
            state.visible_month(),
            date(2026, 9, 1),
            "the grid stays where the start date put it"
        );

        state.move_days(1);
        assert_eq!(
            state.query(),
            "2026-09-01..2026-09-02",
            "filling in the finish starts next to the start, not at today"
        );
    }

    #[test]
    fn opening_a_half_typed_range_shows_the_month_of_the_end_that_is_set() {
        let state = open("2026-09-01..");

        assert_eq!(state.side(), Side::End);
        assert_eq!(state.visible_month(), date(2026, 9, 1));
    }

    #[test]
    fn reports_both_ends_of_a_range_for_the_grid_to_shade() {
        let state = open("2026-09-01..2026-09-30");

        assert_eq!(
            state.range(),
            Some((Some(date(2026, 9, 1)), Some(date(2026, 9, 30))))
        );
        assert_eq!(open("2026-09-01").range(), None, "one date is not a range");
        assert_eq!(
            open("2026-09-01..").range(),
            Some((Some(date(2026, 9, 1)), None)),
            "an open end is still a range"
        );
        assert_eq!(
            open("..2026-09-30").range(),
            Some((None, Some(date(2026, 9, 30))))
        );
    }

    #[test]
    fn a_range_of_keywords_reaches_the_far_edge_of_each_end() {
        // `..` unions the two spans. Reading both ends as the day they start
        // on would have cut `last week..this week` short by six days, and the
        // filter behind it was already matching all fourteen.
        assert_eq!(
            open("last week..this week").range(),
            Some((Some(date(2026, 8, 16)), Some(date(2026, 8, 29))))
        );
        assert_eq!(
            open("last month..this month").range(),
            Some((Some(date(2026, 7, 1)), Some(date(2026, 8, 31))))
        );
        // Mixed ends work the same: only the span the keyword names is wider
        // than a day, and only that end moves.
        assert_eq!(
            open("2026-08-01..this week").range(),
            Some((Some(date(2026, 8, 1)), Some(date(2026, 8, 29))))
        );
        assert_eq!(
            open("this week..2026-08-31").range(),
            Some((Some(date(2026, 8, 23)), Some(date(2026, 8, 31)))),
            "the start end still contributes its first day"
        );
        // An open end names nothing, so there is no span to take an edge of.
        assert_eq!(
            open("this week..").range(),
            Some((Some(date(2026, 8, 23)), None))
        );
        assert_eq!(
            open("..this week").range(),
            Some((None, Some(date(2026, 8, 29))))
        );
    }

    #[test]
    fn the_highlight_takes_the_edge_of_a_keyword_that_its_end_contributes() {
        // `this week` is Aug 23-29 and `next week` is Aug 30 - Sep 5. Which
        // day of each the highlight sits on is the day the keys would move,
        // and that is the edge its own end of the range hands to the filter.
        let mut state = open("this week..next week");

        assert_eq!(state.side(), Side::End);
        assert_eq!(
            state.active_date(),
            Some(date(2026, 9, 5)),
            "the end contributes its last day"
        );

        state.jump_to_start();
        assert_eq!(state.side(), Side::Start);
        assert_eq!(
            state.active_date(),
            Some(date(2026, 8, 23)),
            "the start contributes its first"
        );

        // A one-day name has one edge, so neither end changes it.
        let mut single = open("today..tomorrow");
        assert_eq!(single.active_date(), Some(date(2026, 8, 25)));
        single.jump_to_start();
        assert_eq!(single.active_date(), Some(date(2026, 8, 24)));
    }

    #[test]
    fn a_bare_keyword_is_edited_as_the_end_of_the_range_it_already_is() {
        // With no `..` there is no caret to say which edge is meant, so the
        // end is assumed: that is the edge a picked date replaces.
        let mut state = open("this week");

        assert_eq!(state.side(), Side::Whole);
        assert_eq!(state.active_date(), Some(date(2026, 8, 29)));

        // Moving it spells the week out, keeping the start it came with. The
        // keyword has nowhere to hold "but ending a day later".
        state.move_days(1);
        assert_eq!(state.query(), "2026-08-23..2026-08-30");
        assert_eq!(state.side(), Side::End, "and the caret is on the end it moved");
        assert_eq!(state.active_date(), Some(date(2026, 8, 30)));

        // Moving again edits that end alone, leaving the start alone.
        state.move_days(1);
        assert_eq!(state.query(), "2026-08-23..2026-08-31");
    }

    #[test]
    fn a_one_day_field_keeps_the_highlight_on_the_day_it_would_commit() {
        // A task date is one day, so `this week` there commits the Sunday it
        // starts on. Highlighting the Saturday would show a day the write was
        // never going to send, and nudging it into a range would make the
        // field uncommittable outright.
        let mut cell = CalendarState::open_one_day("Due", "this week", today());

        assert_eq!(cell.active_date(), Some(date(2026, 8, 23)));
        assert_eq!(cell.visible_month(), date(2026, 8, 1));

        cell.move_days(1);
        assert_eq!(
            cell.query(),
            "2026-08-24",
            "one date, not a range the API has nowhere to put"
        );

        // A filter field is the one that spells the week out instead.
        let mut filter = open("this week");
        filter.move_days(1);
        assert_eq!(filter.query(), "2026-08-23..2026-08-30");
    }

    #[test]
    fn a_bare_one_day_keyword_stays_one_date() {
        // Only a name covering more than one day has a start worth keeping.
        // `today` moved is a different day, not a range out of nowhere.
        let mut state = open("today");
        state.move_days(1);
        assert_eq!(state.query(), "2026-08-25");

        let mut typed = open("2026-09-15");
        typed.move_days(-1);
        assert_eq!(typed.query(), "2026-09-14");
    }

    #[test]
    fn the_grid_shows_the_month_the_highlight_landed_in() {
        // `next week` runs Aug 30 - Sep 5, so its end edge is in September
        // while the week starts in August. The highlight has to be on screen.
        let state = open("next week");

        assert_eq!(state.active_date(), Some(date(2026, 9, 5)));
        assert_eq!(state.visible_month(), date(2026, 9, 1));
    }

    #[test]
    fn a_keyword_naming_a_whole_month_reports_it_as_a_span() {
        let state = open("this month");

        assert_eq!(
            state.span(),
            Some((Some(date(2026, 8, 1)), Some(date(2026, 8, 31)))),
            "the grid shades the month the keyword names"
        );
        assert_eq!(state.range(), None, "but it is not a two-ended range");
        assert_eq!(state.visible_month(), date(2026, 8, 1));
        assert!(state.parses());

        let year = open("next year");
        assert_eq!(
            year.span(),
            Some((Some(date(2027, 1, 1)), Some(date(2027, 12, 31))))
        );

        // A single day shades nothing; the highlight is what marks it.
        assert_eq!(open("today").span(), None);
        assert_eq!(open("2026-09-15").span(), None);
        assert_eq!(open("").span(), None);
    }

    #[test]
    fn a_keyword_is_resolved_for_the_summary_shown_in_place_of_the_grid() {
        use crate::domain::DateQuery;

        assert_eq!(
            open("tue").resolution(),
            Some(DateQuery::Exact(date(2026, 8, 25)))
        );
        assert_eq!(
            open("last month").resolution(),
            Some(DateQuery::Range {
                start: Some(date(2026, 7, 1)),
                end: Some(date(2026, 7, 31)),
            })
        );
        assert_eq!(
            open("today-5").resolution(),
            Some(DateQuery::Exact(date(2026, 8, 19)))
        );
        assert_eq!(open("").resolution(), None);
        assert_eq!(open("zz").resolution(), None);
    }

    #[test]
    fn keywords_survive_a_commit_but_incomplete_text_is_filled_in() {
        let mut kept = open("today");
        kept.normalize();
        assert_eq!(kept.query(), "today", "a keyword is what the user chose");

        let mut exact = open("2026-09-15");
        exact.normalize();
        assert_eq!(exact.query(), "2026-09-15");

        let mut partial = open("2026-09");
        partial.normalize();
        assert_eq!(
            partial.query(),
            "2026-09-01",
            "a month alone commits as its first day"
        );

        let mut empty = open("");
        empty.normalize();
        assert_eq!(empty.query(), today().iso(), "and nothing commits as today");
    }

    #[test]
    fn normalizing_only_touches_the_active_end_of_a_range() {
        let mut state = open("2026-09-01..");
        state.normalize();

        assert_eq!(
            state.query(),
            "2026-09-01..2026-09-01",
            "the empty end fills in from the start, and the start is untouched"
        );
    }

    #[test]
    fn lays_the_month_out_as_weeks_starting_sunday() {
        // August 2026 starts on a Saturday and has 31 days, so day 1 sits alone
        // in the last column of the first row and day 2, a Sunday, starts the
        // second. Monday-first fitted this month into five rows; Sunday-first
        // needs six.
        let weeks = open("").weeks();

        assert_eq!(weeks.len(), 6);
        assert_eq!(weeks[0], [None, None, None, None, None, None, Some(1)]);
        assert_eq!(
            weeks[1],
            [
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                Some(8)
            ]
        );
        assert_eq!(
            weeks[5],
            [Some(30), Some(31), None, None, None, None, None],
            "the tail does not spill into September"
        );

        let all_days = weeks
            .iter()
            .flatten()
            .filter_map(|day| *day)
            .collect::<Vec<_>>();
        assert_eq!(all_days, (1..=31).collect::<Vec<_>>());
    }

    #[test]
    fn lays_out_a_february_that_starts_on_a_sunday() {
        // February 2026 starts on a Sunday and has 28 days: exactly four weeks
        // with no leading padding at all — the case a Monday-first grid padded
        // with six blanks.
        let weeks = CalendarState::open("Due", "2026-02-01", today()).weeks();

        assert_eq!(weeks.len(), 4);
        assert_eq!(weeks[0][0], Some(1));
        assert_eq!(weeks[3][6], Some(28));
    }
}
