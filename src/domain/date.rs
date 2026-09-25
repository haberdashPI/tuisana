//! Calendar dates and the filter date grammar.
//!
//! Asana's `due_on` and `start_on` are date-only values: no time, no zone. They
//! are kept that way, because converting a zoneless date between zones is
//! meaningless. The one question that *does* need a timezone is "what is today's
//! date here", and [`today`] answers it from the system zone via `jiff`.
//!
//! This module used to exist twice — once under `src/ui/date.rs` for rendering
//! and once inline in `src/app/task.rs` for filtering — and the two disagreed
//! about the weekday index and about whether `TUISANA_TODAY` was honored. It is
//! now the single authority; `ui::date` formats what lives here.
//!
//! Set `TUISANA_TODAY=YYYY-MM-DD` to pin the current date, which is how the
//! snapshot tests stay deterministic.

/// Weekday names, indexed from Monday.
pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Weekday labels in the order a calendar grid shows them.
///
/// Separate from [`WEEKDAYS`], which starts on Monday because that is the order
/// [`CivilDate::weekday_index`] counts in — and the Gantt chart's weekend
/// shading and week ticks are written against those numbers. Only the picker's
/// grid starts on Sunday.
pub const CALENDAR_WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// Month abbreviations, indexed from January.
pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The month's abbreviated name, for a 1-based month number.
///
/// Out-of-range months clamp to December rather than panicking: a half-typed
/// year in the calendar's text can reach here.
pub fn month_name(month: u32) -> &'static str {
    MONTHS[((month.max(1) - 1) as usize).min(11)]
}

/// A calendar date with no time or zone attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CivilDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl CivilDate {
    /// Builds a date, returning `None` unless it names a real day.
    ///
    /// The year is capped at four digits, which is what the API accepts and what
    /// a `YYYY-MM-DD` field can hold. It also keeps a fat-fingered `99999` from
    /// being treated as a date the user meant.
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        let candidate = Self { year, month, day };
        ((1..=9999).contains(&year) && (1..=12).contains(&month))
            .then_some(candidate)
            .filter(|date| (1..=date.days_in_month()).contains(&day))
    }

    /// Parses a `YYYY-MM-DD` string, rejecting anything else.
    ///
    /// Unlike the old filter-side parser, this rejects dates that are shaped
    /// right but do not exist, such as `2026-02-31` or `2026-99-99`.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        let mut parts = value.split('-');
        let year = parts.next()?.parse::<i32>().ok()?;
        let month = parts.next()?.parse::<u32>().ok()?;
        let day = parts.next()?.parse::<u32>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Self::new(year, month, day)
    }

    /// Days since 1970-01-01, using Howard Hinnant's civil calendar algorithm.
    pub fn days_from_epoch(&self) -> i64 {
        let year = if self.month <= 2 {
            self.year - 1
        } else {
            self.year
        } as i64;
        let era = if year >= 0 { year } else { year - 399 } / 400;
        let year_of_era = year - era * 400;
        let month = self.month as i64;
        let day = self.day as i64;
        let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    /// The inverse of [`CivilDate::days_from_epoch`].
    pub fn from_days(days: i64) -> Self {
        let shifted = days + 719_468;
        let era = if shifted >= 0 {
            shifted
        } else {
            shifted - 146_096
        } / 146_097;
        let day_of_era = shifted - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
        let month = (shifted_month + if shifted_month < 10 { 3 } else { -9 }) as u32;

        Self {
            year: (year + i64::from(month <= 2)) as i32,
            month,
            day,
        }
    }

    /// This date shifted by whole days.
    pub fn add_days(&self, delta: i64) -> Self {
        Self::from_days(self.days_from_epoch() + delta)
    }

    /// This date shifted by whole months, clamping the day to the target month.
    ///
    /// January 31 plus one month is February 28, or February 29 in a leap year.
    /// Clamping rather than overflowing is what makes `j`/`k` in the calendar
    /// overlay feel like flipping pages instead of drifting.
    pub fn add_months(&self, delta: i64) -> Self {
        let total = (self.year as i64) * 12 + (self.month as i64 - 1) + delta;
        let year = total.div_euclid(12) as i32;
        let month = total.rem_euclid(12) as u32 + 1;
        let first = Self {
            year,
            month,
            day: 1,
        };
        Self {
            year,
            month,
            day: self.day.min(first.days_in_month()),
        }
    }

    /// How many days this date's month has, or 0 if the month is not a month.
    ///
    /// Hand-rolled rather than delegated to `jiff`, whose `civil::date` panics on
    /// an out-of-range year. A half-typed year in the calendar's text reaches
    /// here, and the leap rule is four lines.
    pub fn days_in_month(&self) -> u32 {
        match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if is_leap_year(self.year) => 29,
            2 => 28,
            _ => 0,
        }
    }

    /// The first day of this date's month.
    pub fn first_of_month(&self) -> Self {
        Self {
            year: self.year,
            month: self.month,
            day: 1,
        }
    }

    /// The weekday index, counting Monday as 0. Day 0 of the epoch was Thursday.
    pub fn weekday_index(&self) -> usize {
        (self.days_from_epoch() + 3).rem_euclid(7) as usize
    }

    /// The weekday label.
    pub fn weekday(&self) -> &'static str {
        WEEKDAYS[self.weekday_index()]
    }

    /// The column this date occupies in a Sunday-first calendar grid.
    pub fn calendar_column(&self) -> usize {
        (self.weekday_index() + 1) % 7
    }

    /// The Sunday that starts the week this date falls in.
    ///
    /// The week turning over on Sunday is the US convention, and it is the one
    /// both the filter grammar and the table's weekday labels follow, so that a
    /// name means the same day whichever end it is read from.
    pub fn week_start(&self) -> Self {
        self.add_days(-(self.calendar_column() as i64))
    }

    /// Whether both dates fall in the same [`week_start`](Self::week_start)
    /// week.
    pub fn same_week(&self, other: Self) -> bool {
        self.week_start() == other.week_start()
    }

    /// The `YYYY-MM-DD` form, which is also what the Asana API wants.
    pub fn iso(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl std::fmt::Display for CivilDate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.iso())
    }
}

/// Today's date in the system timezone, honoring the `TUISANA_TODAY` override.
pub fn today() -> CivilDate {
    if let Some(pinned) = std::env::var_os("TUISANA_TODAY")
        .and_then(|value| value.into_string().ok())
        .as_deref()
        .and_then(CivilDate::parse)
    {
        return pinned;
    }

    let now = jiff::Zoned::now().date();
    CivilDate {
        year: now.year() as i32,
        month: now.month() as u32,
        day: now.day() as u32,
    }
}

/// The granularity a named date token counts in.
///
/// It is what an offset steps by, which is the whole reason it is tracked:
/// `today-5` is five *days* back, and `this month-1` is one *month* back. A
/// single unit for both would make one of those two wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateUnit {
    Day,
    Week,
    Month,
    Year,
}

/// The stretch of days one token names.
///
/// Most tokens name a single day, but `this week`, `this month`, and
/// `this year` name a whole one, and a filter that matched only their first day would be useless.
/// Stored as an anchor plus a unit rather than as two dates, so an offset can
/// be applied in the unit the token was written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateSpan {
    /// Any day inside the span; the bounds are derived from it and the unit.
    pub anchor: CivilDate,
    pub unit: DateUnit,
}

impl DateSpan {
    /// A span of exactly one day.
    pub fn day(anchor: CivilDate) -> Self {
        Self {
            anchor,
            unit: DateUnit::Day,
        }
    }

    /// The whole week `anchor` falls in, Sunday through Saturday.
    pub fn week(anchor: CivilDate) -> Self {
        Self {
            anchor,
            unit: DateUnit::Week,
        }
    }

    /// The whole month `anchor` falls in.
    pub fn month(anchor: CivilDate) -> Self {
        Self {
            anchor,
            unit: DateUnit::Month,
        }
    }

    /// The whole year `anchor` falls in.
    pub fn year(anchor: CivilDate) -> Self {
        Self {
            anchor,
            unit: DateUnit::Year,
        }
    }

    /// The first day the span covers.
    pub fn start(&self) -> CivilDate {
        match self.unit {
            DateUnit::Day => self.anchor,
            DateUnit::Week => self.anchor.week_start(),
            DateUnit::Month => self.anchor.first_of_month(),
            DateUnit::Year => CivilDate {
                year: self.anchor.year,
                month: 1,
                day: 1,
            },
        }
    }

    /// The last day the span covers.
    pub fn end(&self) -> CivilDate {
        match self.unit {
            DateUnit::Day => self.anchor,
            DateUnit::Week => self.anchor.week_start().add_days(6),
            DateUnit::Month => CivilDate {
                day: self.anchor.days_in_month(),
                ..self.anchor
            },
            DateUnit::Year => CivilDate {
                year: self.anchor.year,
                month: 12,
                day: 31,
            },
        }
    }

    /// Whether the span is a single day, which is what decides whether a query
    /// built from it is [`DateQuery::Exact`] or a range.
    pub fn is_day(&self) -> bool {
        self.unit == DateUnit::Day
    }

    /// This span moved by `delta` of its own unit.
    fn shift(self, delta: i64) -> Self {
        let anchor = match self.unit {
            DateUnit::Day => self.anchor.add_days(delta),
            DateUnit::Week => self.anchor.add_days(delta * 7),
            DateUnit::Month => self.anchor.add_months(delta),
            DateUnit::Year => self.anchor.add_months(delta * 12),
        };
        Self { anchor, ..self }
    }
}

/// The largest offset a token may carry, in its own unit.
///
/// Not a limit anyone will meet on purpose. It is here because `add_days`
/// walks straight into an integer overflow on a pasted `today-99999999999`,
/// and because a year outside `CivilDate`'s four digits is not a date the API
/// would take anyway.
const MAX_OFFSET: i64 = 10_000;

/// Resolves one date token from a filter query to the days it covers.
///
/// Returns `None` when the token is not a date at all, and `Some(None)` for an
/// empty token, which a range uses to mean "no bound on this side".
///
/// Accepted forms, in order: empty, a name, a name or date with an offset
/// (`today-5`, `next month+2`), `MM-DD` in the current year, and a full
/// `YYYY-MM-DD`. Whitespace inside a token is ignored, so `this month` and
/// `thismonth` are the same thing.
///
/// The names are: `today`, `tomorrow`, `yesterday`, any weekday of the current
/// week, and `this`/`last`/`next` followed by `week`, `month`, or `year`.
pub fn parse_span(token: &str, today: CivilDate) -> Option<Option<DateSpan>> {
    let token = token
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if token.is_empty() {
        return Some(None);
    }

    // The whole token first, so an offset is only ever looked for in text that
    // is not already a date. `2026-09-15` would otherwise be read as September
    // 2026 minus fifteen of something.
    if let Some(span) = base_span(&token, today) {
        return Some(Some(span));
    }

    let (head, delta) = split_offset(&token)?;
    let span = base_span(head, today)?.shift(delta);
    // `add_days` and `add_months` do not police the year, and a date outside
    // four digits is not one Asana would take.
    (1..=9999).contains(&span.anchor.year).then_some(Some(span))
}

/// Resolves one date token from a filter query.
///
/// The first day of whatever [`parse_span`] makes of the token, which is the
/// token itself for every form that names a single day. A span that covers
/// more than one — `this month`, `next year` — collapses to the day it starts
/// on, because the callers that reach for this want one date: a task has one
/// due date, and the calendar grid highlights one square.
pub fn parse_token(token: &str, today: CivilDate) -> Option<Option<CivilDate>> {
    Some(parse_span(token, today)?.map(|span| span.start()))
}

/// Resolves a token carrying no offset.
///
/// Takes text already lowercased and stripped of whitespace, which is what
/// lets the names be matched against one spelling apiece.
fn base_span(token: &str, today: CivilDate) -> Option<DateSpan> {
    let span = match token {
        "today" => DateSpan::day(today),
        "tomorrow" => DateSpan::day(today.add_days(1)),
        "yesterday" => DateSpan::day(today.add_days(-1)),
        "thisweek" => DateSpan::week(today),
        "lastweek" => DateSpan::week(today.add_days(-7)),
        "nextweek" => DateSpan::week(today.add_days(7)),
        "thismonth" => DateSpan::month(today),
        "lastmonth" => DateSpan::month(today.add_months(-1)),
        "nextmonth" => DateSpan::month(today.add_months(1)),
        "thisyear" => DateSpan::year(today),
        "lastyear" => DateSpan::year(today.add_months(-12)),
        "nextyear" => DateSpan::year(today.add_months(12)),
        _ => {
            if let Some(index) = weekday_index_from_name(token) {
                // A weekday names a day of the week we are in now, not the next
                // one to come round: on a Thursday, `mon` is the Monday three
                // days back. The week turns over on Sunday, so the day is
                // counted from there, which lands behind today for a day
                // already spent.
                DateSpan::day(today.week_start().add_days(((index + 1) % 7) as i64))
            } else {
                // Two components mean `MM-DD` in the current year; three mean a
                // full date. Counting delimiters rather than measuring bytes
                // keeps a multi-byte token from taking the wrong branch.
                let parts = token.split('-').collect::<Vec<_>>();
                DateSpan::day(match parts.as_slice() {
                    [month, day] => {
                        CivilDate::new(today.year, month.parse().ok()?, day.parse().ok()?)?
                    }
                    [_, _, _] => CivilDate::parse(token)?,
                    _ => return None,
                })
            }
        }
    };
    Some(span)
}

/// Splits a trailing `+N` or `-N` off a token.
///
/// `-` is also the date separator, so a minus offset is only read off a token
/// whose head is a *name*: `today-5` is five days back, and `2026-06-10-01` is
/// still the nonsense it always was rather than the ninth of June. `+` is
/// never a separator, so it needs no such rule and `2026-06-10+5` works.
fn split_offset(token: &str) -> Option<(&str, i64)> {
    let index = token.rfind(['+', '-']).filter(|index| *index > 0)?;
    let (head, offset) = token.split_at(index);
    let digits = &offset[1..];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let plus = offset.starts_with('+');
    if !plus && !head.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let magnitude = digits.parse::<i64>().ok().filter(|n| *n <= MAX_OFFSET)?;
    Some((head, if plus { magnitude } else { -magnitude }))
}

/// Whether a character can appear in a written date, as opposed to a name.
///
/// The digits, the `-` that separates a date's parts and signs an offset, the
/// `+` that signs the other direction, and the `.` of a range. Everything a
/// query needs once the keywords are set aside — which is what the calendar
/// picker does while its grid has the letters.
pub fn is_date_char(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.')
}

/// A parsed date filter query: either a single date or an inclusive range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateQuery {
    /// Matches exactly this date.
    Exact(CivilDate),
    /// Matches dates within these bounds, inclusive. Either side may be open.
    Range {
        start: Option<CivilDate>,
        end: Option<CivilDate>,
    },
}

impl DateQuery {
    /// Parses a whole filter query, returning `None` when it is not a date
    /// expression. An empty query has nothing to match and is also `None`.
    ///
    /// A token that names more than one day is a range even without a `..`:
    /// `this month` is the whole month, and `last month..this month` runs from
    /// the first of one to the last of the other. Each end of a written range
    /// contributes the outside of its own span, which is the reading that
    /// makes a range of two months cover both of them whole.
    pub fn parse(query: &str, today: CivilDate) -> Option<Self> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }

        if let Some((start, end)) = query.split_once("..") {
            let start = parse_span(start, today)?.map(|span| span.start());
            let end = parse_span(end, today)?.map(|span| span.end());
            Some(Self::Range { start, end })
        } else {
            let span = parse_span(query, today)??;
            Some(match span.is_day() {
                true => Self::Exact(span.start()),
                false => Self::Range {
                    start: Some(span.start()),
                    end: Some(span.end()),
                },
            })
        }
    }

    /// Whether a date-only `YYYY-MM-DD` value satisfies this query.
    pub fn matches(&self, value: &str) -> bool {
        let Some(value) = CivilDate::parse(value) else {
            return false;
        };
        match self {
            Self::Exact(date) => value == *date,
            Self::Range { start, end } => {
                start.is_none_or(|start| value >= start)
                    && end.is_none_or(|end| value <= end)
            }
        }
    }

    /// The inclusive bounds this query implies, for pushing down to the API as
    /// `due_on.after` / `due_on.before`.
    pub fn bounds(&self) -> (Option<CivilDate>, Option<CivilDate>) {
        match self {
            Self::Exact(date) => (Some(*date), Some(*date)),
            Self::Range { start, end } => (*start, *end),
        }
    }
}

/// What an incomplete date token can still tell the calendar.
///
/// The picker follows the text as it is typed, so `2026-09` has to be enough to
/// show September even though it names no day yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartialDate {
    /// The month to show. Always the first of it.
    pub month: CivilDate,
    /// The day to highlight, set only when the token names a complete date.
    pub day: Option<CivilDate>,
}

/// Resolves a token that may still be half-typed.
///
/// A complete date resolves to both a month and a day. A year alone resolves to
/// that January; a year and month to that month; anything the parser cannot make
/// sense of at all resolves to `None`, which leaves the calendar where it is.
pub fn parse_partial(token: &str, today: CivilDate) -> Option<PartialDate> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    if let Some(date) = parse_token(token, today).flatten() {
        return Some(PartialDate {
            month: date.first_of_month(),
            day: Some(date),
        });
    }

    let parts = token.split('-').collect::<Vec<_>>();
    if parts.len() > 3 {
        return None;
    }

    let head = parts[0];
    if head.is_empty() || !head.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    // Three or more leading digits can only be a year; one or two is a month in
    // the current year, matching the `MM-DD` form the grammar already accepts.
    let (year, month) = if head.len() >= 3 {
        let year = head.parse::<i32>().ok().filter(|year| (1..=9999).contains(year))?;
        let month = parts
            .get(1)
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|month| (1..=12).contains(month))
            .unwrap_or(1);
        (year, month)
    } else {
        let month = head.parse::<u32>().ok().filter(|month| (1..=12).contains(month))?;
        (today.year, month)
    };

    Some(PartialDate {
        month: CivilDate { year, month, day: 1 },
        day: None,
    })
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn weekday_index_from_name(value: &str) -> Option<usize> {
    let name = value.to_ascii_lowercase();
    let index = match name.as_str() {
        "mon" | "monday" => 0,
        "tue" | "tues" | "tuesday" => 1,
        "wed" | "wednesday" => 2,
        "thu" | "thur" | "thurs" | "thursday" => 3,
        "fri" | "friday" => 4,
        "sat" | "saturday" => 5,
        "sun" | "sunday" => 6,
        _ => return None,
    };
    Some(index)
}

#[cfg(test)]
mod tests {
    use super::{parse_span, parse_token, CivilDate, DateQuery};

    fn date(year: i32, month: u32, day: u32) -> CivilDate {
        CivilDate::new(year, month, day).expect("a real date")
    }

    #[test]
    fn round_trips_civil_dates_through_epoch_days() {
        for value in [
            date(1970, 1, 1),
            date(2000, 2, 29),
            date(2026, 8, 24),
            date(2027, 12, 31),
        ] {
            assert_eq!(CivilDate::from_days(value.days_from_epoch()), value);
        }
        assert_eq!(date(1970, 1, 1).days_from_epoch(), 0);
    }

    #[test]
    fn names_the_weekday_from_monday() {
        assert_eq!(date(1970, 1, 1).weekday(), "Thu");
        assert_eq!(date(1970, 1, 1).weekday_index(), 3);
        assert_eq!(date(2026, 8, 24).weekday(), "Mon");
        assert_eq!(date(2026, 8, 24).weekday_index(), 0);
        assert_eq!(date(2026, 8, 30).weekday_index(), 6, "Sunday ends the week");
    }

    #[test]
    fn the_charts_week_still_starts_on_monday_even_though_the_picker_starts_on_sunday() {
        // Two different questions: a work week starts on Monday, a calendar grid
        // is read from Sunday. weekday_index answers the first — the Gantt
        // chart's weekend shading and week ticks count in it — and
        // calendar_column answers the second.
        let monday = date(2026, 8, 24);
        let sunday = date(2026, 8, 23);

        assert_eq!(monday.weekday_index(), 0);
        assert_eq!(monday.calendar_column(), 1);
        assert_eq!(
            sunday.weekday_index(),
            6,
            "still a weekend by the chart's reckoning"
        );
        assert_eq!(sunday.calendar_column(), 0, "and the first column of the grid");
    }

    #[test]
    fn rejects_dates_that_are_shaped_right_but_do_not_exist() {
        assert!(CivilDate::parse("2026-02-31").is_none());
        assert!(CivilDate::parse("2026-99-99").is_none());
        assert!(CivilDate::parse("2026-13-01").is_none());
        assert!(CivilDate::parse("2026-06-00").is_none());
        assert!(CivilDate::parse("2026-06").is_none());
        assert!(CivilDate::parse("2026-06-10-01").is_none());
        assert_eq!(CivilDate::parse(" 2026-06-10 "), Some(date(2026, 6, 10)));
    }

    #[test]
    fn counts_the_days_in_a_month_across_leap_years() {
        assert_eq!(date(2024, 2, 1).days_in_month(), 29);
        assert_eq!(date(2026, 2, 1).days_in_month(), 28);
        assert_eq!(date(2100, 2, 1).days_in_month(), 28, "not a leap year");
        assert_eq!(date(2000, 2, 1).days_in_month(), 29, "a leap year");
        assert_eq!(date(2026, 4, 1).days_in_month(), 30);
        assert_eq!(date(2026, 12, 1).days_in_month(), 31);
    }

    #[test]
    fn shifting_months_clamps_the_day_rather_than_overflowing() {
        assert_eq!(date(2026, 1, 31).add_months(1), date(2026, 2, 28));
        assert_eq!(date(2028, 1, 31).add_months(1), date(2028, 2, 29));
        assert_eq!(date(2026, 3, 31).add_months(-1), date(2026, 2, 28));
        assert_eq!(date(2026, 1, 15).add_months(-1), date(2025, 12, 15));
        assert_eq!(date(2026, 12, 15).add_months(1), date(2027, 1, 15));
        assert_eq!(date(2026, 8, 24).add_months(12), date(2027, 8, 24));
    }

    #[test]
    fn shifting_days_crosses_month_and_year_boundaries() {
        assert_eq!(date(2026, 8, 31).add_days(1), date(2026, 9, 1));
        assert_eq!(date(2026, 1, 1).add_days(-1), date(2025, 12, 31));
        assert_eq!(date(2024, 2, 28).add_days(1), date(2024, 2, 29));
    }

    #[test]
    fn resolves_every_accepted_token_form() {
        // A Monday, so the weekday cases are easy to read.
        let today = date(2026, 8, 24);

        assert_eq!(parse_token("", today), Some(None));
        assert_eq!(parse_token("today", today), Some(Some(today)));
        assert_eq!(parse_token("TODAY", today), Some(Some(today)));
        assert_eq!(parse_token("tomorrow", today), Some(Some(date(2026, 8, 25))));
        assert_eq!(parse_token("yesterday", today), Some(Some(date(2026, 8, 23))));
        assert_eq!(parse_token("09-15", today), Some(Some(date(2026, 9, 15))));
        assert_eq!(
            parse_token("2027-01-04", today),
            Some(Some(date(2027, 1, 4)))
        );
    }

    #[test]
    fn a_weekday_means_that_day_of_the_current_week() {
        let monday = date(2026, 8, 24);

        assert_eq!(
            parse_token("mon", monday),
            Some(Some(monday)),
            "today's own weekday is today, never a week out"
        );
        assert_eq!(parse_token("tuesday", monday), Some(Some(date(2026, 8, 25))));
        assert_eq!(
            parse_token("sat", monday),
            Some(Some(date(2026, 8, 29))),
            "the week runs to Saturday"
        );
        assert_eq!(
            parse_token("sun", monday),
            Some(Some(date(2026, 8, 23))),
            "the week turned over on Sunday, so Sunday is behind us"
        );
    }

    #[test]
    fn a_weekday_already_past_stays_in_the_week_that_has_it() {
        // A Thursday: everything from Sunday through Wednesday is spent, and
        // naming it must not skip ahead to next week.
        let thursday = date(2026, 8, 27);

        assert_eq!(parse_token("sun", thursday), Some(Some(date(2026, 8, 23))));
        assert_eq!(parse_token("mon", thursday), Some(Some(date(2026, 8, 24))));
        assert_eq!(parse_token("wed", thursday), Some(Some(date(2026, 8, 26))));
        assert_eq!(parse_token("thu", thursday), Some(Some(thursday)));
        assert_eq!(parse_token("fri", thursday), Some(Some(date(2026, 8, 28))));

        // Sunday itself starts a fresh week, so every other day is ahead.
        let sunday = date(2026, 8, 30);
        assert_eq!(parse_token("sun", sunday), Some(Some(sunday)));
        assert_eq!(parse_token("mon", sunday), Some(Some(date(2026, 8, 31))));
        assert_eq!(parse_token("sat", sunday), Some(Some(date(2026, 9, 5))));
    }

    #[test]
    fn a_week_names_the_sunday_to_saturday_it_falls_in() {
        // A Monday, so every bound below has to reach back over a Sunday
        // rather than starting where today does.
        let today = date(2026, 8, 24);
        let span = |token: &str| {
            parse_span(token, today)
                .expect("a date token")
                .expect("not empty")
        };

        let week = span("this week");
        assert_eq!(week.start(), date(2026, 8, 23), "Sunday starts the week");
        assert_eq!(week.end(), date(2026, 8, 29), "and Saturday ends it");
        assert!(!week.is_day());

        assert_eq!(span("last week").start(), date(2026, 8, 16));
        assert_eq!(span("last week").end(), date(2026, 8, 22));
        assert_eq!(span("next week").start(), date(2026, 8, 30));
        assert_eq!(span("next week").end(), date(2026, 9, 5));

        // An offset steps whole weeks, so the two spellings agree the way
        // `this month-1` and `last month` do.
        assert_eq!(span("this week-1"), span("last week"));
        assert_eq!(span("this week+1"), span("next week"));
        assert_eq!(span("this week+2").start(), date(2026, 9, 6));

        assert_eq!(span("ThisWeek"), span("this  week"));

        // A Sunday is the first day of its own week, not the last of the one
        // before: the bounds must not slide back seven days.
        let sunday = date(2026, 8, 30);
        let from_sunday = parse_span("this week", sunday)
            .expect("a date token")
            .expect("not empty");
        assert_eq!(from_sunday.start(), sunday);
        assert_eq!(from_sunday.end(), date(2026, 9, 5));
    }

    #[test]
    fn a_week_covers_every_day_between_its_ends() {
        let today = date(2026, 8, 24);
        let query = DateQuery::parse("this week", today).expect("parses");

        assert!(query.matches("2026-08-23"));
        assert!(query.matches("2026-08-26"));
        assert!(query.matches("2026-08-29"));
        assert!(!query.matches("2026-08-22"));
        assert!(!query.matches("2026-08-30"));
    }

    #[test]
    fn a_month_or_a_year_names_every_day_in_it() {
        let today = date(2026, 8, 24);
        let span = |token: &str| {
            parse_span(token, today)
                .expect("a date token")
                .expect("not empty")
        };

        let month = span("this month");
        assert_eq!(month.start(), date(2026, 8, 1));
        assert_eq!(month.end(), date(2026, 8, 31), "to the last day, not the first");
        assert!(!month.is_day());

        assert_eq!(span("last month").start(), date(2026, 7, 1));
        assert_eq!(span("last month").end(), date(2026, 7, 31));
        assert_eq!(span("next month").start(), date(2026, 9, 1));
        assert_eq!(span("next month").end(), date(2026, 9, 30));

        assert_eq!(span("this year").start(), date(2026, 1, 1));
        assert_eq!(span("this year").end(), date(2026, 12, 31));
        assert_eq!(span("last year").start(), date(2025, 1, 1));
        assert_eq!(span("next year").end(), date(2027, 12, 31));

        // The spelling is forgiving: case and the space are both noise.
        assert_eq!(span("ThisMonth"), span("this  month"));
    }

    #[test]
    fn an_offset_steps_in_the_unit_the_token_was_written_in() {
        let today = date(2026, 8, 24);
        let span = |token: &str| {
            parse_span(token, today)
                .expect("a date token")
                .expect("not empty")
        };

        // Days for a day token.
        assert_eq!(span("today-5").start(), date(2026, 8, 19));
        assert_eq!(span("today+5").start(), date(2026, 8, 29));
        assert_eq!(span("tomorrow-1").start(), today);
        assert_eq!(span("mon+7").start(), date(2026, 8, 31));

        // Months for a month token, which is what makes the two spellings of
        // last month agree.
        assert_eq!(span("this month-1"), span("last month"));
        assert_eq!(span("this month+1"), span("next month"));
        assert_eq!(span("next month+1").start(), date(2026, 10, 1));

        // And years for a year token.
        assert_eq!(span("this year-1"), span("last year"));
        assert_eq!(span("this year+2").start(), date(2028, 1, 1));

        // `+` is never a date separator, so it reads as an offset even on a
        // written date. `-` is, so it does not.
        assert_eq!(span("2026-08-24+1").start(), date(2026, 8, 25));
        assert_eq!(parse_span("2026-08-24-1", today), None);
    }

    #[test]
    fn a_span_of_more_than_a_day_filters_as_a_range() {
        let today = date(2026, 8, 24);

        assert_eq!(
            DateQuery::parse("this month", today),
            Some(DateQuery::Range {
                start: Some(date(2026, 8, 1)),
                end: Some(date(2026, 8, 31)),
            })
        );
        assert_eq!(
            DateQuery::parse("today", today),
            Some(DateQuery::Exact(today)),
            "a single day is still exact"
        );

        // Each end of a written range gives the outside of its own span, so
        // two months cover both of them whole.
        assert_eq!(
            DateQuery::parse("last month..this month", today),
            Some(DateQuery::Range {
                start: Some(date(2026, 7, 1)),
                end: Some(date(2026, 8, 31)),
            })
        );
        assert_eq!(
            DateQuery::parse("today..next month", today),
            Some(DateQuery::Range {
                start: Some(today),
                end: Some(date(2026, 9, 30)),
            })
        );

        let month = DateQuery::parse("this month", today).expect("parses");
        assert!(month.matches("2026-08-01"));
        assert!(month.matches("2026-08-31"));
        assert!(!month.matches("2026-09-01"));
    }

    #[test]
    fn named_times_are_measured_against_the_day_they_are_resolved_on() {
        // The same query, read on two days, names two different spans. This is
        // what makes a saved filter mean "this month" rather than "August".
        let query = "this month";

        let august = DateQuery::parse(query, date(2026, 8, 24)).expect("parses");
        let september = DateQuery::parse(query, date(2026, 9, 2)).expect("parses");

        assert!(august.matches("2026-08-15"));
        assert!(!august.matches("2026-09-15"));
        assert!(september.matches("2026-09-15"));
        assert!(!september.matches("2026-08-15"));

        assert_eq!(
            parse_token("today", date(2026, 8, 24)),
            Some(Some(date(2026, 8, 24)))
        );
        assert_eq!(
            parse_token("today", date(2026, 8, 25)),
            Some(Some(date(2026, 8, 25)))
        );
    }

    #[test]
    fn a_multi_day_span_collapses_to_its_first_day_for_a_single_date() {
        let today = date(2026, 8, 24);

        assert_eq!(parse_token("this month", today), Some(Some(date(2026, 8, 1))));
        assert_eq!(parse_token("next year", today), Some(Some(date(2027, 1, 1))));
    }

    #[test]
    fn an_offset_that_runs_off_the_calendar_is_refused() {
        let today = date(2026, 8, 24);

        // Past the four digits a date field holds.
        assert_eq!(parse_span("next year+9999", today), None);
        // Past the cap, before any arithmetic can overflow.
        assert_eq!(parse_span("today-99999", today), None);
        assert_eq!(parse_span("today-99999999999999999999", today), None);
        // Not an offset at all.
        assert_eq!(parse_span("today-", today), None);
        assert_eq!(parse_span("today-x", today), None);
        assert_eq!(parse_span("-5", today), None);
    }

    #[test]
    fn rejects_tokens_that_are_not_dates() {
        let today = date(2026, 8, 24);

        assert_eq!(parse_token("someday", today), None);
        assert_eq!(parse_token("2026-02-31", today), None);
        assert_eq!(parse_token("13-01", today), None);
        assert_eq!(parse_token("2026-06-10-01", today), None);
    }

    #[test]
    fn parses_exact_queries_and_ranges_with_open_bounds() {
        let today = date(2026, 8, 24);

        assert_eq!(
            DateQuery::parse("2026-09-15", today),
            Some(DateQuery::Exact(date(2026, 9, 15)))
        );
        assert_eq!(
            DateQuery::parse("2026-09-01..2026-09-30", today),
            Some(DateQuery::Range {
                start: Some(date(2026, 9, 1)),
                end: Some(date(2026, 9, 30)),
            })
        );
        assert_eq!(
            DateQuery::parse("today..", today),
            Some(DateQuery::Range {
                start: Some(today),
                end: None,
            })
        );
        assert_eq!(
            DateQuery::parse("..today", today),
            Some(DateQuery::Range {
                start: None,
                end: Some(today),
            })
        );
        assert_eq!(DateQuery::parse("", today), None);
        assert_eq!(DateQuery::parse("   ", today), None);
        assert_eq!(DateQuery::parse("garbage", today), None);
        assert_eq!(DateQuery::parse("2026-09-01..garbage", today), None);
    }

    #[test]
    fn matches_values_against_exact_dates_and_range_bounds() {
        let today = date(2026, 8, 24);
        let exact = DateQuery::parse("2026-09-15", today).expect("parses");

        assert!(exact.matches("2026-09-15"));
        assert!(!exact.matches("2026-09-16"));
        assert!(!exact.matches("not a date"));

        let range = DateQuery::parse("2026-09-01..2026-09-30", today).expect("parses");
        assert!(range.matches("2026-09-01"), "the start is inclusive");
        assert!(range.matches("2026-09-30"), "the end is inclusive");
        assert!(range.matches("2026-09-15"));
        assert!(!range.matches("2026-08-31"));
        assert!(!range.matches("2026-10-01"));

        let open = DateQuery::parse("2026-09-01..", today).expect("parses");
        assert!(open.matches("2030-01-01"));
        assert!(!open.matches("2026-08-31"));
    }

    #[test]
    fn reports_the_bounds_a_query_implies() {
        let today = date(2026, 8, 24);

        assert_eq!(
            DateQuery::parse("today", today).expect("parses").bounds(),
            (Some(today), Some(today)),
            "an exact date bounds the fetch window on both sides"
        );
        assert_eq!(
            DateQuery::parse("today..", today).expect("parses").bounds(),
            (Some(today), None)
        );
    }

    #[test]
    fn resolves_half_typed_tokens_to_a_month_and_maybe_a_day() {
        use super::parse_partial;
        let today = date(2026, 8, 24);
        let partial = |token: &str| parse_partial(token, today);

        // A complete date gives both.
        let full = partial("2026-09-15").expect("a complete date");
        assert_eq!(full.month, date(2026, 9, 1));
        assert_eq!(full.day, Some(date(2026, 9, 15)));

        // A year alone lands on January, per the picker's contract.
        assert_eq!(partial("2026").map(|p| p.month), Some(date(2026, 1, 1)));
        assert_eq!(partial("2026").and_then(|p| p.day), None);
        assert_eq!(partial("2026-").map(|p| p.month), Some(date(2026, 1, 1)));

        // A year and month give the month, with no day highlighted.
        assert_eq!(partial("2026-09").map(|p| p.month), Some(date(2026, 9, 1)));
        assert_eq!(partial("2026-09").and_then(|p| p.day), None);
        assert_eq!(partial("2026-9").map(|p| p.month), Some(date(2026, 9, 1)));
        assert_eq!(partial("2026-09-").map(|p| p.month), Some(date(2026, 9, 1)));

        // A date that does not exist still shows the month it was aiming at.
        let impossible = partial("2026-02-31").expect("the month is still usable");
        assert_eq!(impossible.month, date(2026, 2, 1));
        assert_eq!(impossible.day, None);

        // One or two digits are a month in the current year, not a year.
        assert_eq!(partial("9").map(|p| p.month), Some(date(2026, 9, 1)));
        assert_eq!(partial("09").map(|p| p.month), Some(date(2026, 9, 1)));
        assert_eq!(partial("09-").map(|p| p.month), Some(date(2026, 9, 1)));
        assert_eq!(partial("09-99").map(|p| p.month), Some(date(2026, 9, 1)));

        // Nothing usable leaves the caller's month alone.
        assert_eq!(partial(""), None);
        assert_eq!(partial("99"), None);
        assert_eq!(partial("0"), None);
        assert_eq!(partial("someday"), None);
        assert_eq!(partial("1-2-3-4"), None);
        assert_eq!(partial("99999-01-01"), None, "an absurd year is rejected");
    }

    #[test]
    fn counting_days_in_a_month_never_panics_on_nonsense() {
        // `jiff::civil::date` panics on an out-of-range year, and half-typed
        // text reaches this path.
        assert_eq!(CivilDate { year: 99_999, month: 2, day: 1 }.days_in_month(), 28);
        assert_eq!(CivilDate { year: 2026, month: 0, day: 1 }.days_in_month(), 0);
        assert_eq!(CivilDate { year: 2026, month: 99, day: 1 }.days_in_month(), 0);
    }

    #[test]
    fn today_honors_the_pinned_override() {
        // The suite runs threaded and `set_var` is process-wide, so this asserts
        // against whatever the snapshot harness already pinned rather than
        // setting the variable itself.
        if let Some(pinned) = std::env::var("TUISANA_TODAY")
            .ok()
            .as_deref()
            .and_then(CivilDate::parse)
        {
            assert_eq!(super::today(), pinned);
        } else {
            let now = super::today();
            assert!((1..=12).contains(&now.month), "a real month");
            assert!((1..=now.days_in_month()).contains(&now.day), "a real day");
        }
    }
}
