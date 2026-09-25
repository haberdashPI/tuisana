//! The calendar date picker overlay.
//!
//! Drawn as a centered modal over the panes, like the help overlay, so opening
//! it never reflows the layout underneath. The grid is seven fixed columns three
//! characters wide, so the box is the same size in every month and the day
//! numbers line up under two-letter weekday labels.
//!
//! The text being edited is deliberately *not* here — it lives in the filter
//! field, where the filter panel draws it with a caret. This overlay shows only
//! the month, so there is one place to look for the value and one for the
//! calendar.
//!
//! Nothing here reads app state directly: [`calendar_view`] turns the picker's
//! state into a [`CalendarView`], and [`render`] draws it. That split is what
//! lets the layout be tested without a terminal.

use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use crate::{
    app::calendar::CalendarState,
    config::Mode,
    domain::CivilDate,
    ui::{
        chrome::{pane_block, Chip, Tone},
        date::month_name,
        layout,
        theme::Theme,
    },
};

/// Width of one day cell, including its trailing gap.
const CELL_WIDTH: usize = 3;
/// Columns of content inside the border: one cell per weekday.
const GRID_WIDTH: usize = CELL_WIDTH * 7;

/// The date grammar, spelled out for the collapsed overlay.
///
/// Only drawn with the grid hidden, which is exactly when the keywords can be
/// typed: with the grid up `t`, `d`, `h`, `j`, `k`, and `l` steer it, so the
/// letters of `today` never reach the text. Putting the key where the grid was
/// answers the question that state raises — what can I type here?
///
/// Left column is the forms, right column is what they mean. Written in plain
/// ASCII so it reads the same under the monochrome theme's glyph set.
const KEYWORD_KEY: [(&str, &str); 8] = [
    ("today  tomorrow  yesterday", "one day"),
    ("mon  friday", "a day of this week"),
    ("this/last/next week", "Sunday to Saturday"),
    ("this/last/next month", "the whole month"),
    ("this/last/next year", "the whole year"),
    ("2026-09-15  09-15", "a date, the year optional"),
    ("today+5  this month-1", "an offset, in its own unit"),
    ("today..fri  ..2026-12-31", "a range, either end open"),
];

/// Width of the key's left column, so the meanings line up in one column.
const KEY_FORM_WIDTH: usize = 28;

/// One day cell in the month grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarCell {
    /// The day of the month, or `None` for padding outside it.
    pub day: Option<u32>,
    /// Whether this is today.
    pub today: bool,
    /// Whether the highlight sits here.
    pub cursor: bool,
    /// Whether this is one end of a range.
    pub endpoint: bool,
    /// Whether this falls inside a range, endpoints aside.
    pub in_range: bool,
}

/// Snapshot of the picker used by the renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarView {
    /// The field being edited, e.g. `Due`.
    pub title: String,
    /// The month on display, e.g. `Aug 2026`.
    pub month_label: String,
    /// Whether the query is usable. `false` earns a warning chip.
    pub parses: bool,
    /// Which end of a range is being edited, when the query is a range.
    pub side_label: Option<&'static str>,
    /// Whether the month grid is showing. Hidden, the overlay shows the
    /// resolved line below plus the keyword key, and the grid's keys type
    /// instead of navigating.
    pub grid: bool,
    /// The whole query spelled out as dates, shown in place of the grid.
    pub resolved: String,
    /// The month laid out as weeks starting Sunday. Empty with the grid hidden.
    pub weeks: Vec<Vec<CalendarCell>>,
}

/// Builds the view from picker state, or `None` when the picker is closed.
///
/// `grid` is the user's show/hide toggle. With the grid hidden the weeks are
/// not built at all: there is nowhere to draw them, and the resolved line
/// answers the question the grid answers.
pub(crate) fn calendar_view(state: Option<&CalendarState>, grid: bool) -> Option<CalendarView> {
    use crate::app::calendar::Side;

    let state = state?;
    let month = state.visible_month();
    let cursor = state.active_date();
    let today = state.today();
    // The span rather than the range, so `this month` shades its month the way
    // a written `..` between the same two dates would.
    let span = state.span();

    let weeks = match grid {
        false => Vec::new(),
        true => build_weeks(state, month, today, cursor, span),
    };

    Some(CalendarView {
        title: state.field_label().to_string(),
        month_label: format!("{} {}", month_name(month.month), month.year),
        parses: state.parses(),
        side_label: match state.side() {
            Side::Whole => None,
            Side::Start => Some("from"),
            Side::End => Some("to"),
        },
        grid,
        resolved: resolved_label(state),
        weeks,
    })
}

/// The month grid, one cell per square, with the shading already decided.
fn build_weeks(
    state: &CalendarState,
    month: CivilDate,
    today: CivilDate,
    cursor: Option<CivilDate>,
    span: Option<(Option<CivilDate>, Option<CivilDate>)>,
) -> Vec<Vec<CalendarCell>> {
    state
        .weeks()
        .into_iter()
        .map(|week| {
            week.iter()
                .map(|day| {
                    let date = day.and_then(|day| CivilDate::new(month.year, month.month, day));
                    let (endpoint, in_range) = match (date, span) {
                        (Some(date), Some((start, end))) => (
                            Some(date) == start || Some(date) == end,
                            within(date, start, end),
                        ),
                        _ => (false, false),
                    };
                    CalendarCell {
                        day: *day,
                        today: date == Some(today),
                        cursor: date.is_some() && date == cursor,
                        endpoint,
                        in_range,
                    }
                })
                .collect()
        })
        .collect()
}

/// The query spelled out as the dates it resolves to.
///
/// Drawn in place of the grid, because it answers the same question the grid
/// does: which days does this text mean, today. That is the whole point of a
/// keyword — `next month` is not a date until something says which one.
fn resolved_label(state: &CalendarState) -> String {
    use crate::domain::DateQuery;

    // An open end of a range is drawn as nothing, matching the query's own
    // `today..`: there is no date there to name.
    let iso = |date: Option<CivilDate>| date.map(|date| date.iso()).unwrap_or_default();

    match state.resolution() {
        Some(DateQuery::Exact(date)) => format!("{} {}", date.weekday(), date.iso()),
        Some(DateQuery::Range { start, end }) => format!("{}..{}", iso(start), iso(end)),
        None if state.query().trim().is_empty() => "no date".to_string(),
        None => "not a date".to_string(),
    }
}

/// Whether a date falls inside a range whose ends may be open.
///
/// An open end reaches as far as the month shows, which is what `today..` means:
/// everything from today onward.
fn within(date: CivilDate, start: Option<CivilDate>, end: Option<CivilDate>) -> bool {
    if start.is_none() && end.is_none() {
        return false;
    }
    start.is_none_or(|start| date >= start) && end.is_none_or(|end| date <= end)
}

/// Draws the picker centered in `area`.
pub fn render(frame: &mut Frame<'_>, area: Rect, theme: &Theme, view: &CalendarView) {
    let lines = calendar_lines(view, theme);

    // Measured rather than fixed, because the collapsed form is one line as
    // wide as its dates need. With the grid up every line is exactly
    // GRID_WIDTH, so the box stays the same size in every month.
    let width = lines
        .iter()
        .map(|line| line.width())
        .max()
        .unwrap_or(GRID_WIDTH);
    let box_area = layout::centered(
        area,
        (width as u16).saturating_add(2),
        (lines.len() as u16).saturating_add(2),
    );

    // The month is drawn inside the box rather than in the border: at 23 columns
    // there is not room for both it and a chip, and the border truncates from the
    // left, which turned "Aug 2026" into "6".
    let mut chips = Vec::new();
    if let Some(side) = view.side_label {
        chips.push(Chip::toned(side, Tone::Accent));
    }
    if !view.parses {
        chips.push(Chip::toned("unparseable", Tone::Danger));
    }

    let block = pane_block(theme, true, Mode::Calendar, &view.title, &chips);
    let inner = block.inner(box_area);

    frame.render_widget(Clear, box_area);
    frame.render_widget(block, box_area);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The overlay's content lines: the month heading, the weekday header, and the
/// month grid.
fn calendar_lines(view: &CalendarView, theme: &Theme) -> Vec<Line<'static>> {
    // Collapsed, there is no month to head and no grid to head it over. What
    // takes the grid's place is the resolved value and the key to the names
    // that can now be typed, since hiding the grid is what frees their letters.
    if !view.grid {
        return keyword_key_lines(view, theme);
    }

    let mut lines = Vec::with_capacity(view.weeks.len() + 2);

    lines.push(Line::from(Span::styled(
        center(&view.month_label),
        theme.subtitle,
    )));

    // Two-letter labels, so a weekday header fits the same three-wide cell the
    // days use and the columns line up.
    lines.push(Line::from(
        crate::domain::CALENDAR_WEEKDAYS
            .iter()
            .map(|name| Span::styled(pad_day(&name[..2]), theme.muted))
            .collect::<Vec<_>>(),
    ));

    for week in &view.weeks {
        lines.push(Line::from(
            week.iter()
                .map(|cell| day_span(cell, theme))
                .collect::<Vec<_>>(),
        ));
    }

    lines
}

/// The collapsed overlay: the resolved value, then the date grammar's key.
///
/// The value leads, because it is the answer to what the text currently means;
/// the key follows it across a blank line as reference rather than result.
fn keyword_key_lines(view: &CalendarView, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(KEYWORD_KEY.len() + 2);

    lines.push(Line::from(Span::styled(view.resolved.clone(), theme.text)));
    lines.push(Line::default());

    for (form, meaning) in KEYWORD_KEY {
        lines.push(Line::from(vec![
            Span::styled(format!("{form:<KEY_FORM_WIDTH$}"), theme.text),
            Span::styled(meaning.to_string(), theme.muted),
        ]));
    }

    lines
}

fn day_span(cell: &CalendarCell, theme: &Theme) -> Span<'static> {
    let Some(day) = cell.day else {
        return Span::raw(pad_day(""));
    };

    // Every distinction has to survive the monochrome theme, where the styles
    // collapse to one color, so each one carries a modifier rather than leaning
    // on hue alone.
    let mut style = if cell.endpoint {
        theme.accent.add_modifier(Modifier::BOLD)
    } else if cell.in_range {
        // The subtle band between the two ends of a range.
        theme.text.add_modifier(Modifier::UNDERLINED)
    } else if cell.today {
        theme.accent
    } else {
        theme.text
    };

    if cell.cursor {
        style = style.patch(theme.cursor).add_modifier(Modifier::REVERSED);
    }

    Span::styled(pad_day(&day.to_string()), style)
}

/// Right-aligns a day in its cell so the columns line up under the headers.
fn pad_day(value: &str) -> String {
    format!("{value:>2} ")
}

/// Centers a label over the grid.
fn center(value: &str) -> String {
    let pad = GRID_WIDTH.saturating_sub(value.chars().count()) / 2;
    format!("{:pad$}{value}", "", pad = pad)
}

#[cfg(test)]
mod tests {
    use super::{calendar_lines, calendar_view, CalendarCell, GRID_WIDTH};
    use crate::{app::calendar::CalendarState, domain::CivilDate, ui::theme::Theme};
    use ratatui::style::Modifier;

    fn state(query: &str) -> CalendarState {
        CalendarState::open(
            "Due",
            query,
            CivilDate::new(2026, 8, 24).expect("a real date"),
        )
    }

    fn cells(query: &str) -> Vec<CalendarCell> {
        calendar_view(Some(&state(query)), true)
            .expect("the picker is open")
            .weeks
            .into_iter()
            .flatten()
            .filter(|cell| cell.day.is_some())
            .collect()
    }

    fn day(cells: &[CalendarCell], day: u32) -> CalendarCell {
        *cells
            .iter()
            .find(|cell| cell.day == Some(day))
            .expect("the day is in the month")
    }

    #[test]
    fn returns_nothing_while_the_picker_is_closed() {
        assert!(calendar_view(None, true).is_none());
        assert!(calendar_view(Some(&state("")), true).is_some());
    }

    #[test]
    fn labels_the_field_and_the_month_on_display() {
        let view = calendar_view(Some(&state("")), true).expect("the picker is open");

        assert_eq!(view.title, "Due");
        assert_eq!(view.month_label, "Aug 2026");
        assert!(view.parses);
        assert_eq!(view.side_label, None, "a single date has no two ends");
    }

    #[test]
    fn marks_today_and_the_highlighted_day() {
        let mut picker = state("");
        picker.move_days(1);
        let view = calendar_view(Some(&picker), true).expect("the picker is open");
        let cells = view
            .weeks
            .iter()
            .flatten()
            .filter(|cell| cell.day.is_some())
            .collect::<Vec<_>>();

        let today = cells.iter().filter(|cell| cell.today).collect::<Vec<_>>();
        let cursor = cells.iter().filter(|cell| cell.cursor).collect::<Vec<_>>();

        assert_eq!(today.len(), 1, "exactly one day is today");
        assert_eq!(today[0].day, Some(24));
        assert_eq!(cursor.len(), 1, "exactly one day is highlighted");
        assert_eq!(cursor[0].day, Some(25));
    }

    #[test]
    fn highlights_nothing_while_the_month_is_only_half_typed() {
        let mut picker = state("");
        for ch in "2026-09".chars() {
            picker.push_char(ch);
        }
        let view = calendar_view(Some(&picker), true).expect("the picker is open");

        assert_eq!(view.month_label, "Sep 2026");
        assert!(
            view.weeks.iter().flatten().all(|cell| !cell.cursor),
            "no day is named yet, so none is highlighted"
        );
    }

    #[test]
    fn a_month_away_from_today_marks_no_day_as_today() {
        let view = calendar_view(Some(&state("2027-01-04")), true).expect("the picker is open");

        assert!(view.weeks.iter().flatten().all(|cell| !cell.today));
        assert_eq!(view.month_label, "Jan 2027");
    }

    #[test]
    fn shades_the_days_between_the_two_ends_of_a_range() {
        let cells = cells("2026-08-10..2026-08-20");

        assert!(day(&cells, 10).endpoint, "the start is an end");
        assert!(day(&cells, 20).endpoint, "so is the finish");
        assert!(day(&cells, 15).in_range, "and the days between are shaded");
        assert!(!day(&cells, 15).endpoint);
        assert!(!day(&cells, 9).in_range, "outside the range is not");
        assert!(!day(&cells, 21).in_range);
    }

    #[test]
    fn an_open_ended_range_shades_everything_past_its_one_end() {
        let after = cells("2026-08-10..");
        assert!(day(&after, 10).endpoint);
        assert!(day(&after, 31).in_range);
        assert!(!day(&after, 9).in_range);

        let before = cells("..2026-08-10");
        assert!(day(&before, 10).endpoint);
        assert!(day(&before, 1).in_range);
        assert!(!day(&before, 11).in_range);
    }

    #[test]
    fn a_single_date_shades_nothing() {
        let cells = cells("2026-08-10");

        assert!(cells.iter().all(|cell| !cell.in_range));
        assert!(cells.iter().all(|cell| !cell.endpoint));
        assert!(day(&cells, 10).cursor, "it is just the highlight");
    }

    #[test]
    fn says_which_end_of_a_range_is_being_edited() {
        let mut picker = state("2026-08-10..2026-08-20");
        assert_eq!(
            calendar_view(Some(&picker), true).expect("open").side_label,
            Some("to")
        );

        picker.jump_to_start();
        assert_eq!(
            calendar_view(Some(&picker), true).expect("open").side_label,
            Some("from")
        );
    }

    #[test]
    fn a_keyword_naming_a_month_shades_the_whole_month() {
        // The same shading a written `2026-08-01..2026-08-31` earns, because
        // the two mean the same days.
        let cells = cells("this month");

        assert!(day(&cells, 1).endpoint, "the month starts here");
        assert!(day(&cells, 31).endpoint, "and ends here");
        assert!(day(&cells, 15).in_range);
        assert!(cells.iter().all(|cell| cell.in_range));
    }

    #[test]
    fn a_keyword_naming_a_week_shades_sunday_through_saturday() {
        // Today is a Monday, so the shading has to reach back over the Sunday
        // that starts the week rather than beginning where the cursor is.
        let cells = cells("this week");

        assert!(day(&cells, 23).endpoint, "Sunday starts the week");
        assert!(day(&cells, 29).endpoint, "and Saturday ends it");
        for day_of_month in 24..=28 {
            assert!(day(&cells, day_of_month).in_range);
        }
        assert!(!day(&cells, 22).in_range, "the week before is not shaded");
        assert!(!day(&cells, 30).in_range, "nor the week after");
    }

    #[test]
    fn a_range_of_keywords_shades_both_spans_whole() {
        // `..` unions the two ends: the start contributes the first day of its
        // span and the end the last. Stopping on the Sunday `this week` starts
        // on would have drawn six fewer days than the filter matches.
        let cells = cells("last week..this week");

        assert!(day(&cells, 16).endpoint, "last week starts here");
        assert!(day(&cells, 29).endpoint, "and this week ends here");
        for day_of_month in 17..=28 {
            assert!(
                day(&cells, day_of_month).in_range,
                "{day_of_month} is inside the union"
            );
        }
        assert!(!day(&cells, 15).in_range);
        assert!(!day(&cells, 30).in_range);
    }

    #[test]
    fn the_shading_covers_exactly_what_the_filter_matches() {
        // The grid and the filter read the same text, and a disagreement
        // between them is invisible until it has already hidden rows.
        use crate::domain::{CivilDate, DateQuery};

        for query in [
            "last week..this week",
            "this week..next week",
            "last month..this month",
            "this week..2026-08-31",
            "2026-08-01..this week",
            "this year..this year",
        ] {
            let today = CivilDate::new(2026, 8, 24).expect("a real date");
            let (from, to) = DateQuery::parse(query, today)
                .expect("a date query")
                .bounds();
            let (start, end) = state(query).span().expect("a shaded span");

            assert_eq!(start, from, "{query}: the grid starts where the filter does");
            assert_eq!(end, to, "{query}: and ends where it does");
        }
    }

    #[test]
    fn hiding_the_grid_replaces_it_with_the_dates_the_text_resolves_to() {
        let theme = Theme::default();
        let resolved = |query: &str| {
            let view = calendar_view(Some(&state(query)), false).expect("the picker is open");
            assert!(view.weeks.is_empty(), "there is no grid to build");
            assert!(!view.grid);
            let lines = calendar_lines(&view, &theme);
            // The value leads; the key to the grammar follows it.
            lines[0].to_string()
        };

        // A keyword is the whole reason the line exists: `tue` is not a date
        // until something says which Tuesday.
        assert_eq!(resolved("tue"), "Tue 2026-08-25");
        assert_eq!(resolved("this month"), "2026-08-01..2026-08-31");
        assert_eq!(resolved("today-5"), "Wed 2026-08-19");
        // An open end is drawn as nothing, matching the query's own `today..`.
        assert_eq!(resolved("today.."), "2026-08-24..");
        assert_eq!(resolved(""), "no date");
        assert_eq!(resolved("zz"), "not a date");

        // The grid is still there when it is asked for.
        let shown = calendar_view(Some(&state("tue")), true).expect("the picker is open");
        assert!(shown.grid);
        assert!(!shown.weeks.is_empty());
    }

    #[test]
    fn hiding_the_grid_puts_the_key_to_the_keywords_in_its_place() {
        // The key belongs here and only here: with the grid up its letters
        // steer the grid, so none of these names can be typed.
        let theme = Theme::default();
        let text = |grid: bool| {
            let view = calendar_view(Some(&state("tue")), grid).expect("the picker is open");
            calendar_lines(&view, &theme)
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        };

        let collapsed = text(false);
        for keyword in [
            "today", "tomorrow", "yesterday", "this/last/next week", "this/last/next month",
            "this/last/next year", "2026-09-15", "today+5", "today..fri",
        ] {
            assert!(collapsed.contains(keyword), "the key is missing {keyword}");
        }
        assert!(
            collapsed.starts_with("Tue 2026-08-25"),
            "the resolved value still leads"
        );

        assert!(
            !text(true).contains("tomorrow"),
            "with the grid up the key would only be a lie about what types"
        );
    }

    #[test]
    fn the_key_fits_the_narrowest_terminal_the_snapshots_cover() {
        // 80 columns less the task pane's two borders and a column of margin
        // either side. A wider key would be truncated rather than wrapped.
        let theme = Theme::default();
        let view = calendar_view(Some(&state("2026-08-10..2026-08-20")), false)
            .expect("the picker is open");

        for line in calendar_lines(&view, &theme) {
            assert!(line.width() <= 74, "{:?} is too wide for 80 columns", line.to_string());
        }
    }

    #[test]
    fn every_grid_line_is_exactly_the_grid_width() {
        let theme = Theme::default();

        for query in ["", "2026-02-01", "2027-01-04"] {
            let view = calendar_view(Some(&state(query)), true).expect("the picker is open");
            let lines = calendar_lines(&view, &theme);

            // The month heading may be shorter; the weekday header and every
            // week must be exactly one cell per day.
            let grid = &lines[1..];
            assert_eq!(grid.len(), view.weeks.len() + 1);
            for line in grid {
                assert_eq!(
                    line.to_string().len(),
                    GRID_WIDTH,
                    "the trailing gap keeps every row the same width"
                );
            }
        }
    }

    #[test]
    fn unparseable_text_is_reported_rather_than_silently_matching_nothing() {
        let mut picker = state("");
        for ch in "99-99".chars() {
            picker.push_char(ch);
        }
        let view = calendar_view(Some(&picker), true).expect("the picker is open");

        assert!(!view.parses);
    }

    #[test]
    fn each_kind_of_day_gets_its_own_style() {
        use super::day_span;
        let theme = Theme::default();
        let cell = |f: fn(&mut CalendarCell)| {
            let mut cell = CalendarCell {
                day: Some(15),
                today: false,
                cursor: false,
                endpoint: false,
                in_range: false,
            };
            f(&mut cell);
            day_span(&cell, &theme).style
        };

        let plain = cell(|_| {});
        let today = cell(|c| c.today = true);
        let endpoint = cell(|c| c.endpoint = true);
        let in_range = cell(|c| c.in_range = true);
        let cursor = cell(|c| c.cursor = true);

        let styles = [plain, today, endpoint, in_range, cursor];
        for (a, first) in styles.iter().enumerate() {
            for second in styles.iter().skip(a + 1) {
                assert_ne!(first, second, "every kind of day has to look different");
            }
        }

        // The range styles lean on modifiers, not hue, so they survive the
        // monochrome theme.
        assert!(endpoint.add_modifier.contains(Modifier::BOLD));
        assert!(in_range.add_modifier.contains(Modifier::UNDERLINED));
        assert!(cursor.add_modifier.contains(Modifier::REVERSED));

        // The highlight stays visible when it lands on an end of the range.
        let both = cell(|c| {
            c.cursor = true;
            c.endpoint = true;
        });
        assert!(both.add_modifier.contains(Modifier::REVERSED));
        assert!(both.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn the_weekday_header_starts_on_sunday() {
        let theme = Theme::default();
        let view = calendar_view(Some(&state("")), true).expect("the picker is open");
        let lines = calendar_lines(&view, &theme);

        assert_eq!(lines[1].to_string(), "Su Mo Tu We Th Fr Sa ");
    }

    #[test]
    fn the_grid_holds_every_day_of_the_month_once() {
        let view = calendar_view(Some(&state("2026-02-01")), true).expect("the picker is open");

        let days = view
            .weeks
            .iter()
            .flatten()
            .filter_map(|cell| cell.day)
            .collect::<Vec<_>>();
        assert_eq!(days, (1..=28).collect::<Vec<_>>());
    }
}
