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
    /// The month laid out as weeks starting Sunday.
    pub weeks: Vec<Vec<CalendarCell>>,
}

/// Builds the view from picker state, or `None` when the picker is closed.
pub(crate) fn calendar_view(state: Option<&CalendarState>) -> Option<CalendarView> {
    use crate::app::calendar::Side;

    let state = state?;
    let month = state.visible_month();
    let cursor = state.active_date();
    let today = state.today();
    let range = state.range();

    let weeks = state
        .weeks()
        .into_iter()
        .map(|week| {
            week.iter()
                .map(|day| {
                    let date = day.and_then(|day| CivilDate::new(month.year, month.month, day));
                    let (endpoint, in_range) = match (date, range) {
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
        .collect();

    Some(CalendarView {
        title: state.field_label().to_string(),
        month_label: format!("{} {}", month_name(month.month), month.year),
        parses: state.parses(),
        side_label: match state.side() {
            Side::Whole => None,
            Side::Start => Some("from"),
            Side::End => Some("to"),
        },
        weeks,
    })
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

    let box_area = layout::centered(
        area,
        (GRID_WIDTH as u16).saturating_add(2),
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
        calendar_view(Some(&state(query)))
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
        assert!(calendar_view(None).is_none());
        assert!(calendar_view(Some(&state(""))).is_some());
    }

    #[test]
    fn labels_the_field_and_the_month_on_display() {
        let view = calendar_view(Some(&state(""))).expect("the picker is open");

        assert_eq!(view.title, "Due");
        assert_eq!(view.month_label, "Aug 2026");
        assert!(view.parses);
        assert_eq!(view.side_label, None, "a single date has no two ends");
    }

    #[test]
    fn marks_today_and_the_highlighted_day() {
        let mut picker = state("");
        picker.move_days(1);
        let view = calendar_view(Some(&picker)).expect("the picker is open");
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
        let view = calendar_view(Some(&picker)).expect("the picker is open");

        assert_eq!(view.month_label, "Sep 2026");
        assert!(
            view.weeks.iter().flatten().all(|cell| !cell.cursor),
            "no day is named yet, so none is highlighted"
        );
    }

    #[test]
    fn a_month_away_from_today_marks_no_day_as_today() {
        let view = calendar_view(Some(&state("2027-01-04"))).expect("the picker is open");

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
            calendar_view(Some(&picker)).expect("open").side_label,
            Some("to")
        );

        picker.jump_to_start();
        assert_eq!(
            calendar_view(Some(&picker)).expect("open").side_label,
            Some("from")
        );
    }

    #[test]
    fn every_grid_line_is_exactly_the_grid_width() {
        let theme = Theme::default();

        for query in ["", "2026-02-01", "2027-01-04"] {
            let view = calendar_view(Some(&state(query))).expect("the picker is open");
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
        let view = calendar_view(Some(&picker)).expect("the picker is open");

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
        let view = calendar_view(Some(&state(""))).expect("the picker is open");
        let lines = calendar_lines(&view, &theme);

        assert_eq!(lines[1].to_string(), "Su Mo Tu We Th Fr Sa ");
    }

    #[test]
    fn the_grid_holds_every_day_of_the_month_once() {
        let view = calendar_view(Some(&state("2026-02-01"))).expect("the picker is open");

        let days = view
            .weeks
            .iter()
            .flatten()
            .filter_map(|cell| cell.day)
            .collect::<Vec<_>>();
        assert_eq!(days, (1..=28).collect::<Vec<_>>());
    }
}
