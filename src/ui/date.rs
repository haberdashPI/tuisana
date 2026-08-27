//! Relative date formatting for the task table.
//!
//! Asana returns date-only values (`YYYY-MM-DD`). Rendering them verbatim costs
//! ten columns and tells the reader nothing about urgency, so the table shows a
//! short relative form instead and pairs it with an urgency level the renderer
//! turns into a color and a marker.
//!
//! "Today" is resolved in UTC because the standard library has no timezone
//! database. For a date-only field the worst case is a value reading `Today`
//! instead of `Tomorrow` for the few hours around midnight in a west-of-UTC
//! offset. Set `TUISANA_TODAY=YYYY-MM-DD` to pin it, which is also how the
//! snapshot tests stay deterministic.

use std::time::{SystemTime, UNIX_EPOCH};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// A calendar date with no time or zone attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CivilDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl CivilDate {
    /// Parses a `YYYY-MM-DD` string, rejecting anything else.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        let mut parts = value.split('-');
        let year = parts.next()?.parse::<i32>().ok()?;
        let month = parts.next()?.parse::<u32>().ok()?;
        let day = parts.next()?.parse::<u32>().ok()?;
        if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        Some(Self { year, month, day })
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

    /// The weekday label, where day 0 of the epoch was a Thursday.
    pub fn weekday(&self) -> &'static str {
        let index = (self.days_from_epoch() + 3).rem_euclid(7) as usize;
        WEEKDAYS[index]
    }
}

/// Today's date, honoring the `TUISANA_TODAY` override.
pub fn today() -> CivilDate {
    if let Some(pinned) = std::env::var_os("TUISANA_TODAY")
        .and_then(|value| value.into_string().ok())
        .as_deref()
        .and_then(CivilDate::parse)
    {
        return pinned;
    }

    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_secs() / 86_400) as i64)
        .unwrap_or(0);
    civil_from_days(days)
}

/// How urgently a date wants the reader's attention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Urgency {
    /// The date has passed.
    Overdue,
    /// The date is today.
    Today,
    /// The date is within the next three days.
    Soon,
    /// Further out, or not a parseable date.
    Later,
}

/// A date formatted for display alongside its urgency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelativeDate {
    /// The short display text.
    pub text: String,
    /// The urgency the renderer colors by.
    pub urgency: Urgency,
}

/// Formats a `YYYY-MM-DD` value relative to `today`.
///
/// Unparseable values are passed through unchanged with [`Urgency::Later`] so a
/// surprising API value is still readable rather than dropped.
pub fn format_relative(value: &str, today: CivilDate) -> RelativeDate {
    let Some(date) = CivilDate::parse(value) else {
        return RelativeDate {
            text: value.trim().to_string(),
            urgency: Urgency::Later,
        };
    };

    let diff = date.days_from_epoch() - today.days_from_epoch();
    let urgency = match diff {
        d if d < 0 => Urgency::Overdue,
        0 => Urgency::Today,
        1..=3 => Urgency::Soon,
        _ => Urgency::Later,
    };

    let text = match diff {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        -1 => "Yesterday".to_string(),
        2..=6 => date.weekday().to_string(),
        _ if date.year == today.year => {
            format!("{} {}", MONTHS[(date.month - 1) as usize], date.day)
        }
        _ => format!("{:04}-{:02}-{:02}", date.year, date.month, date.day),
    };

    RelativeDate { text, urgency }
}

fn civil_from_days(days: i64) -> CivilDate {
    let shifted = days + 719_468;
    let era = if shifted >= 0 { shifted } else { shifted - 146_096 } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = (shifted_month + if shifted_month < 10 { 3 } else { -9 }) as u32;

    CivilDate {
        year: (year + i64::from(month <= 2)) as i32,
        month,
        day,
    }
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, format_relative, CivilDate, Urgency};

    fn date(year: i32, month: u32, day: u32) -> CivilDate {
        CivilDate { year, month, day }
    }

    #[test]
    fn round_trips_civil_dates_through_epoch_days() {
        for value in [
            date(1970, 1, 1),
            date(2000, 2, 29),
            date(2026, 8, 24),
            date(2027, 12, 31),
        ] {
            assert_eq!(civil_from_days(value.days_from_epoch()), value);
        }
        assert_eq!(date(1970, 1, 1).days_from_epoch(), 0);
    }

    #[test]
    fn names_the_weekday() {
        assert_eq!(date(1970, 1, 1).weekday(), "Thu");
        assert_eq!(date(2026, 8, 24).weekday(), "Mon");
    }

    #[test]
    fn formats_near_dates_relatively_and_far_dates_absolutely() {
        let today = date(2026, 6, 10);

        assert_eq!(format_relative("2026-06-10", today).text, "Today");
        assert_eq!(format_relative("2026-06-11", today).text, "Tomorrow");
        assert_eq!(format_relative("2026-06-09", today).text, "Yesterday");
        assert_eq!(format_relative("2026-06-13", today).text, "Sat");
        assert_eq!(format_relative("2026-07-01", today).text, "Jul 1");
        assert_eq!(format_relative("2027-01-04", today).text, "2027-01-04");
    }

    #[test]
    fn grades_urgency_from_overdue_through_later() {
        let today = date(2026, 6, 10);

        assert_eq!(
            format_relative("2026-06-01", today).urgency,
            Urgency::Overdue
        );
        assert_eq!(format_relative("2026-06-10", today).urgency, Urgency::Today);
        assert_eq!(format_relative("2026-06-13", today).urgency, Urgency::Soon);
        assert_eq!(format_relative("2026-06-14", today).urgency, Urgency::Later);
    }

    #[test]
    fn passes_through_values_that_are_not_dates() {
        let today = date(2026, 6, 10);
        let rendered = format_relative("someday", today);

        assert_eq!(rendered.text, "someday");
        assert_eq!(rendered.urgency, Urgency::Later);
        assert!(CivilDate::parse("2026-13-01").is_none());
        assert!(CivilDate::parse("2026-06").is_none());
    }
}
