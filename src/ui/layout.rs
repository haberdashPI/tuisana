//! Frame geometry: the single place that decides where each region goes.
//!
//! The chrome is a fixed three lines — a header bar at the top, and a hint bar
//! and status bar at the bottom. Because it never changes height, toggling help
//! or switching modes cannot reflow the panes underneath it.
//!
//! Whatever height is left is the body. It holds the shared top pane (the
//! project list or the filter panel, never both) above the task pane, sized by
//! the user's [`PaneSizeState`].

use ratatui::layout::Rect;

use crate::app::PaneSizeState;

/// Height of the header bar.
pub const HEADER_HEIGHT: u16 = 1;
/// Height of the hint bar.
pub const HINT_HEIGHT: u16 = 1;
/// Height of the status bar.
pub const STATUS_HEIGHT: u16 = 1;
/// Smallest useful top pane: a border, a header row, and a couple of rows.
const MIN_TOP_PANE_HEIGHT: u16 = 6;
/// Smallest task pane worth leaving behind: a border and two rows.
const MIN_TASK_PANE_HEIGHT: u16 = 4;

/// The regions of one drawn frame.
///
/// A pane is `None` when it has no room at all, which happens when the top pane
/// is minimized, when it is maximized (squeezing out the task pane), or on a
/// terminal too short to hold a body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Regions {
    /// The one-line header bar.
    pub header: Rect,
    /// The project list or filter panel.
    pub top_pane: Option<Rect>,
    /// The recently-edited pane, when it has anything to hold.
    pub recent_pane: Option<Rect>,
    /// The task table.
    pub task_pane: Option<Rect>,
    /// The one-line contextual hint bar.
    pub hint: Rect,
    /// The one-line mode and settings bar.
    pub status: Rect,
    /// Everything between the header and the bars, used to center the overlay.
    pub body: Rect,
}

/// Divides a frame into its regions.
///
/// `recent_rows` is how many rows the recently-edited pane has to show; zero
/// leaves it out entirely. Its height comes out of the task pane rather than
/// the top one, because it is an aside to the table and not to the project
/// list.
pub fn regions(
    area: Rect,
    tasks_visible: bool,
    pane: &PaneSizeState,
    recent_rows: usize,
) -> Regions {
    let header = Rect {
        height: HEADER_HEIGHT.min(area.height),
        ..area
    };

    // Chrome claims its lines in priority order so a very short terminal
    // degrades predictably instead of overlapping regions.
    let after_header = area.height - header.height;
    let status_height = STATUS_HEIGHT.min(after_header);
    let after_status = after_header - status_height;
    let hint_height = HINT_HEIGHT.min(after_status);
    let body_height = after_status - hint_height;

    let body = Rect {
        y: header.y + header.height,
        height: body_height,
        ..area
    };
    let hint = Rect {
        y: body.y + body.height,
        height: hint_height,
        ..area
    };
    let status = Rect {
        y: hint.y + hint.height,
        height: status_height,
        ..area
    };

    let (top_pane, task_pane) = split_body(body, tasks_visible, pane);
    let (recent_pane, task_pane) = split_recent(task_pane, recent_rows);

    Regions {
        header,
        top_pane,
        recent_pane,
        task_pane,
        hint,
        status,
        body,
    }
}

/// Takes the recently-edited pane off the top of the task pane.
///
/// It sits between the top pane and the table, so the cursor moving from one
/// list to the other moves one row on screen.
fn split_recent(task_pane: Option<Rect>, recent_rows: usize) -> (Option<Rect>, Option<Rect>) {
    let Some(task_pane) = task_pane else {
        return (None, None);
    };
    if recent_rows == 0 {
        return (None, Some(task_pane));
    }

    // Two for the pane's own border, and never so much that the table it is
    // an aside to has nowhere left to draw. How many rows are worth showing
    // is the pane's own decision, made where the list is built.
    let wanted = recent_rows as u16 + 2;
    let height = wanted.min(task_pane.height.saturating_sub(MIN_TASK_PANE_HEIGHT));
    if height < 3 {
        return (None, Some(task_pane));
    }

    let recent = Rect {
        height,
        ..task_pane
    };
    let tasks = Rect {
        y: task_pane.y + height,
        height: task_pane.height - height,
        ..task_pane
    };
    (non_empty(recent), non_empty(tasks))
}

fn split_body(
    body: Rect,
    tasks_visible: bool,
    pane: &PaneSizeState,
) -> (Option<Rect>, Option<Rect>) {
    if !tasks_visible {
        return (non_empty(body), None);
    }

    let top_height = pane
        .actual_height(body.height, MIN_TOP_PANE_HEIGHT)
        .min(body.height);
    let top = Rect {
        height: top_height,
        ..body
    };
    let bottom = Rect {
        y: body.y + top_height,
        height: body.height - top_height,
        ..body
    };

    (non_empty(top), non_empty(bottom))
}

fn non_empty(area: Rect) -> Option<Rect> {
    (area.height > 0 && area.width > 0).then_some(area)
}

/// Centers a box of the given size inside `area`, clamping to fit.
///
/// Shared by the overlays so a modal is positioned the same way no matter which
/// one is showing.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Anchors a box of the given size to the bottom-right of `area`, clamping to
/// fit.
///
/// The corner rather than the centre, because what goes here is an aside: it
/// reports on something the user already did, so it must not land on top of
/// the row they are looking at.
pub fn bottom_right(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width),
        y: area.y + (area.height - height),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::{bottom_right, regions, HEADER_HEIGHT, HINT_HEIGHT, STATUS_HEIGHT};
    use crate::app::PaneSizeState;
    use ratatui::layout::Rect;

    fn frame(width: u16, height: u16) -> Rect {
        Rect::new(0, 0, width, height)
    }

    #[test]
    fn chrome_is_always_three_lines_and_regions_never_overlap() {
        let pane = PaneSizeState::default();

        for tasks_visible in [false, true] {
            let layout = regions(frame(120, 40), tasks_visible, &pane, 0);

            assert_eq!(layout.header, Rect::new(0, 0, 120, HEADER_HEIGHT));
            assert_eq!(layout.body, Rect::new(0, 1, 120, 37));
            assert_eq!(layout.hint, Rect::new(0, 38, 120, HINT_HEIGHT));
            assert_eq!(layout.status, Rect::new(0, 39, 120, STATUS_HEIGHT));

            let body_bottom = layout
                .task_pane
                .or(layout.top_pane)
                .map(|pane| pane.y + pane.height)
                .expect("a body pane exists");
            assert_eq!(body_bottom, layout.hint.y);
        }
    }

    #[test]
    fn the_top_pane_takes_the_whole_body_when_tasks_are_hidden() {
        let layout = regions(frame(80, 20), false, &PaneSizeState::default(), 0);

        assert_eq!(layout.top_pane, Some(Rect::new(0, 1, 80, 17)));
        assert_eq!(layout.task_pane, None);
    }

    #[test]
    fn panes_share_the_body_when_tasks_are_visible() {
        let layout = regions(frame(80, 30), true, &PaneSizeState::default(), 0);

        let top = layout.top_pane.expect("top pane");
        let task = layout.task_pane.expect("task pane");
        assert_eq!(top.height, PaneSizeState::default().preferred());
        assert_eq!(task.y, top.y + top.height);
        assert_eq!(top.height + task.height, 27);
    }

    #[test]
    fn minimizing_and_maximizing_drop_the_other_pane_entirely() {
        let mut pane = PaneSizeState::default();
        pane.minimize();
        let minimized = regions(frame(80, 30), true, &pane, 0);
        assert_eq!(minimized.top_pane, None);
        assert_eq!(minimized.task_pane.expect("task pane").height, 27);

        let mut pane = PaneSizeState::default();
        pane.maximize();
        let maximized = regions(frame(80, 30), true, &pane, 0);
        assert_eq!(maximized.top_pane.expect("top pane").height, 27);
        assert_eq!(maximized.task_pane, None);
    }

    #[test]
    fn the_recent_pane_takes_its_height_from_the_task_pane() {
        let pane = PaneSizeState::default();
        let without = regions(frame(80, 30), true, &pane, 0);
        let with = regions(frame(80, 30), true, &pane, 2);

        let recent = with.recent_pane.expect("the pane is drawn");
        assert_eq!(recent.height, 4, "two rows and a border");
        assert_eq!(with.top_pane, without.top_pane, "the top pane is untouched");
        assert_eq!(recent.y, without.task_pane.expect("task pane").y);
        assert_eq!(
            with.task_pane.expect("task pane").height,
            without.task_pane.expect("task pane").height - recent.height
        );
    }

    #[test]
    fn the_recent_pane_gives_way_on_a_short_terminal() {
        let pane = PaneSizeState::default();

        // Nothing left to take it from: the table wins, because the pane is
        // an aside to it.
        let cramped = regions(frame(80, 18), true, &pane, 3);
        assert_eq!(cramped.recent_pane, None);
        assert!(cramped.task_pane.is_some());
    }

    #[test]
    fn a_terminal_too_short_for_a_body_still_places_chrome_in_priority_order() {
        let pane = PaneSizeState::default();

        let one_line = regions(frame(40, 1), true, &pane, 0);
        assert_eq!(one_line.header.height, 1);
        assert_eq!(one_line.status.height, 0);
        assert_eq!(one_line.top_pane, None);
        assert_eq!(one_line.task_pane, None);

        let three_lines = regions(frame(40, 3), true, &pane, 0);
        assert_eq!(three_lines.header.height, 1);
        assert_eq!(three_lines.hint.height, 1);
        assert_eq!(three_lines.status.height, 1);
        assert_eq!(three_lines.top_pane, None);
    }

    #[test]
    fn the_corner_anchor_sits_in_the_bottom_right_and_shrinks_to_fit() {
        let body = Rect::new(0, 1, 80, 30);

        let anchored = bottom_right(body, 30, 6);
        assert_eq!(anchored, Rect::new(50, 25, 30, 6));
        assert_eq!(anchored.right(), body.right());
        assert_eq!(anchored.bottom(), body.bottom());

        // A box bigger than the region is clamped rather than drawn off-screen.
        assert_eq!(bottom_right(body, 200, 100), body);
    }
}
