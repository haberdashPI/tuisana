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
pub fn regions(area: Rect, tasks_visible: bool, pane: &PaneSizeState) -> Regions {
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

    Regions {
        header,
        top_pane,
        task_pane,
        hint,
        status,
        body,
    }
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

#[cfg(test)]
mod tests {
    use super::{regions, HEADER_HEIGHT, HINT_HEIGHT, STATUS_HEIGHT};
    use crate::app::PaneSizeState;
    use ratatui::layout::Rect;

    fn frame(width: u16, height: u16) -> Rect {
        Rect::new(0, 0, width, height)
    }

    #[test]
    fn chrome_is_always_three_lines_and_regions_never_overlap() {
        let pane = PaneSizeState::default();

        for tasks_visible in [false, true] {
            let layout = regions(frame(120, 40), tasks_visible, &pane);

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
        let layout = regions(frame(80, 20), false, &PaneSizeState::default());

        assert_eq!(layout.top_pane, Some(Rect::new(0, 1, 80, 17)));
        assert_eq!(layout.task_pane, None);
    }

    #[test]
    fn panes_share_the_body_when_tasks_are_visible() {
        let layout = regions(frame(80, 30), true, &PaneSizeState::default());

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
        let minimized = regions(frame(80, 30), true, &pane);
        assert_eq!(minimized.top_pane, None);
        assert_eq!(minimized.task_pane.expect("task pane").height, 27);

        let mut pane = PaneSizeState::default();
        pane.maximize();
        let maximized = regions(frame(80, 30), true, &pane);
        assert_eq!(maximized.top_pane.expect("top pane").height, 27);
        assert_eq!(maximized.task_pane, None);
    }

    #[test]
    fn a_terminal_too_short_for_a_body_still_places_chrome_in_priority_order() {
        let pane = PaneSizeState::default();

        let one_line = regions(frame(40, 1), true, &pane);
        assert_eq!(one_line.header.height, 1);
        assert_eq!(one_line.status.height, 0);
        assert_eq!(one_line.top_pane, None);
        assert_eq!(one_line.task_pane, None);

        let three_lines = regions(frame(40, 3), true, &pane);
        assert_eq!(three_lines.header.height, 1);
        assert_eq!(three_lines.hint.height, 1);
        assert_eq!(three_lines.status.height, 1);
        assert_eq!(three_lines.top_pane, None);
    }
}
