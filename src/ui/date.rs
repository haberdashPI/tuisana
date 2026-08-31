//! Relative date formatting for the task table.
//!
//! Asana returns date-only values (`YYYY-MM-DD`). Rendering them verbatim costs
//! ten columns and tells the reader nothing about urgency, so the table shows a
//! short relative form instead and pairs it with an urgency level the renderer
//! turns into a color and a marker.
//!
//! The dates themselves, and the local-timezone [`today`], live in
//! [`crate::domain::date`]. This module only decides how they read.

pub use crate::domain::date::{month_name, today, CivilDate};

use crate::domain::date::MONTHS;

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
        _ => date.iso(),
    };

    RelativeDate { text, urgency }
}

#[cfg(test)]
mod tests {
    use super::{format_relative, month_name, CivilDate, Urgency};

    fn date(year: i32, month: u32, day: u32) -> CivilDate {
        CivilDate::new(year, month, day).expect("a real date")
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
    }

    #[test]
    fn names_months() {
        assert_eq!(month_name(1), "Jan");
        assert_eq!(month_name(8), "Aug");
        assert_eq!(month_name(12), "Dec");
    }
}
