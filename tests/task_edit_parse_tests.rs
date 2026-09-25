//! Parsing and resolution for an edited cell, with no app state in the way.
//!
//! The messages are asserted on rather than just the `Err`: they are what the
//! pane border shows, and they are the only explanation the user gets.

use tuisana::domain::{
    parse_custom_value, parse_date_value, resolve_assignee, AssigneeRef, CivilDate,
    CustomFieldValue, CustomValueKind,
};

/// A fixed "today", passed in rather than read from the clock.
///
/// `parse_date_value` takes the date as an argument for exactly this reason:
/// the integration tests share one process, and `TUISANA_TODAY` is global.
fn today() -> CivilDate {
    CivilDate::new(2026, 6, 1).expect("a real day")
}

#[test]
fn a_date_parses_every_form_the_picker_accepts_and_normalizes_it() {
    for (input, expected) in [
        ("today", "2026-06-01"),
        ("tomorrow", "2026-06-02"),
        ("yesterday", "2026-05-31"),
        ("today-5", "2026-05-27"),
        ("today+5", "2026-06-06"),
        ("09-28", "2026-09-28"),
        ("2026-09-28", "2026-09-28"),
        (" 2026-09-28 ", "2026-09-28"),
        // A name covering a whole span commits as the day it starts on: a
        // task has one date and nowhere to put the rest of the month.
        ("this month", "2026-06-01"),
        ("next month", "2026-07-01"),
        ("last month", "2026-05-01"),
        ("next year", "2027-01-01"),
        ("this month-1", "2026-05-01"),
    ] {
        assert_eq!(
            parse_date_value(input, today()),
            Ok(Some(expected.to_string())),
            "{input}"
        );
    }
}

/// A filter keeps the word and re-reads it; a task date is fixed on the day it
/// was set. The two are the same grammar and deliberately not the same
/// lifetime — Asana stores a date, not an expression.
#[test]
fn a_task_date_is_fixed_against_the_day_it_was_edited_on() {
    let june = CivilDate::new(2026, 6, 1).expect("a real date");
    let july = CivilDate::new(2026, 7, 1).expect("a real date");

    assert_eq!(
        parse_date_value("tomorrow", june),
        Ok(Some("2026-06-02".to_string()))
    );
    assert_eq!(
        parse_date_value("tomorrow", july),
        Ok(Some("2026-07-02".to_string())),
        "the same word, resolved a month later, is a different date"
    );
}

#[test]
fn an_empty_date_clears_the_field() {
    assert_eq!(parse_date_value("", today()), Ok(None));
    assert_eq!(parse_date_value("   ", today()), Ok(None));
}

#[test]
fn a_range_is_refused_rather_than_half_applied() {
    // A sensible filter and a nonsensical due date.
    assert_eq!(
        parse_date_value("2026-09-01..2026-09-08", today()),
        Err("a task date is one day, not a range".to_string())
    );
}

#[test]
fn a_date_that_is_shaped_right_but_does_not_exist_is_refused() {
    assert_eq!(
        parse_date_value("2026-02-31", today()),
        Err("2026-02-31 is not a date".to_string())
    );
    assert_eq!(
        parse_date_value("someday", today()),
        Err("someday is not a date".to_string())
    );
}

fn directory() -> Vec<(String, String)> {
    vec![
        ("user-alex".to_string(), "alex".to_string()),
        ("user-jo".to_string(), "jo".to_string()),
        ("user-jo-2".to_string(), "jo".to_string()),
    ]
}

#[test]
fn an_assignee_resolves_by_name_by_email_and_by_me() {
    assert_eq!(
        resolve_assignee("Alex", &directory(), Some("user-alex")),
        Ok(Some(AssigneeRef::new("user-alex", "alex"))),
        "matched case-insensitively against the people on screen"
    );
    assert_eq!(
        resolve_assignee("someone@example.com", &directory(), None),
        Ok(Some(AssigneeRef::new(
            "someone@example.com",
            "someone@example.com"
        ))),
        "Asana takes an email in place of a gid, so it goes verbatim"
    );
    assert_eq!(
        resolve_assignee("me", &directory(), Some("user-alex")),
        Ok(Some(AssigneeRef::new("user-alex", "alex")))
    );
    assert_eq!(resolve_assignee("", &directory(), None), Ok(None), "unassigns");
}

#[test]
fn an_unknown_or_ambiguous_name_is_refused_with_a_reason() {
    assert_eq!(
        resolve_assignee("Robin", &directory(), None),
        Err("no one called Robin is loaded".to_string())
    );
    assert_eq!(
        resolve_assignee("jo", &directory(), None),
        Err("jo is ambiguous".to_string()),
        "two people share the name, and a write has to pick one gid"
    );
    assert_eq!(
        resolve_assignee("me", &directory(), None),
        Err("no current user is known".to_string())
    );
}

fn options() -> Vec<(String, String)> {
    vec![
        ("opt-high".to_string(), "High".to_string()),
        ("opt-low".to_string(), "Low".to_string()),
    ]
}

#[test]
fn an_enum_value_resolves_to_the_option_gid() {
    let options = options();

    assert_eq!(
        parse_custom_value(CustomValueKind::Enum(&options), "high"),
        Ok(Some(CustomFieldValue::Enum {
            option_gid: "opt-high".to_string(),
            name: "High".to_string(),
        }))
    );
    assert_eq!(
        parse_custom_value(CustomValueKind::Enum(&options), "Blocked"),
        Err("Blocked is not one of this field's options".to_string())
    );
    assert_eq!(
        parse_custom_value(CustomValueKind::Enum(&options), ""),
        Ok(None),
        "an empty value clears the field"
    );
}

#[test]
fn a_number_field_refuses_text() {
    assert_eq!(
        parse_custom_value(CustomValueKind::Number, "3.5"),
        Ok(Some(CustomFieldValue::Number {
            value: 3.5,
            text: "3.5".to_string(),
        }))
    );
    assert_eq!(
        parse_custom_value(CustomValueKind::Number, "soon"),
        Err("soon is not a number".to_string())
    );
}

#[test]
fn a_kind_this_milestone_does_not_write_is_refused_outright() {
    // `date`, `multi_enum`, and `people` custom fields each need an editor of
    // their own, and are deferred.
    assert_eq!(
        parse_custom_value(CustomValueKind::Unsupported, "anything"),
        Err("this field cannot be edited here yet".to_string())
    );
    assert_eq!(
        parse_custom_value(CustomValueKind::Unsupported, ""),
        Err("this field cannot be edited here yet".to_string()),
        "refused even when it would only be clearing the value"
    );
}

#[test]
fn a_text_field_keeps_what_was_typed() {
    assert_eq!(
        parse_custom_value(CustomValueKind::Text, " notes here "),
        Ok(Some(CustomFieldValue::Text("notes here".to_string())))
    );
}
