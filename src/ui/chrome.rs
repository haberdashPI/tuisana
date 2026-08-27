//! The frame around the content: header bar, status bar, and pane borders.
//!
//! Two rules shape everything here:
//!
//! - **A fact appears in exactly one place.** Pane titles and per-pane counts
//!   live in the pane's own border. Global context lives in the header. Active
//!   settings live in the status bar. Nothing is repeated on a line of its own.
//! - **Focus is legible without color.** The focused pane is drawn with a
//!   heavier border, so it survives a monochrome terminal, and is additionally
//!   tinted with the mode color that the status badge also uses.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::{
    config::Mode,
    ui::{
        text::{clip_spans, pad_cell_centered, spans_width, visible_width},
        theme::Theme,
    },
};

/// The app name shown in the header badge.
const BRAND: &str = "TUISANA";

/// A short piece of labeled state, rendered inline in a bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    /// The text to show.
    pub text: String,
    /// Which semantic role colors it.
    pub tone: Tone,
}

impl Chip {
    /// A neutral chip.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Muted,
        }
    }

    /// A chip carrying a semantic tone.
    pub fn toned(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            tone,
        }
    }
}

/// The semantic roles a chip or message can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// De-emphasized: counts, defaults, placeholders.
    Muted,
    /// Ordinary emphasis.
    Text,
    /// The accent color: something the user turned on.
    Accent,
    /// Complete or healthy.
    Ok,
    /// Wants attention soon.
    Warn,
    /// Overdue or failed.
    Danger,
    /// Informational.
    Info,
}

impl Tone {
    /// Resolves this tone against a theme.
    pub fn style(self, theme: &Theme) -> Style {
        match self {
            Tone::Muted => theme.muted,
            Tone::Text => theme.text,
            Tone::Accent => theme.accent,
            Tone::Ok => theme.ok,
            Tone::Warn => theme.warn,
            Tone::Danger => theme.danger,
            Tone::Info => theme.info,
        }
    }
}

/// A centered message shown in place of empty pane content.
///
/// A pane with nothing in it should say why, and say what to press next, rather
/// than drawing an empty box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneMessage {
    /// The primary line.
    pub text: String,
    /// An optional second line suggesting what to do.
    pub hint: Option<String>,
    /// The tone for the primary line.
    pub tone: Tone,
}

impl PaneMessage {
    /// A message with no follow-up hint.
    pub fn new(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            hint: None,
            tone,
        }
    }

    /// Attaches a suggested next step.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// Renders the header bar: brand badge, breadcrumb context, right-side state.
pub fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    leading: Option<Span<'static>>,
    crumbs: &[String],
    right: Vec<Span<'static>>,
) {
    if area.height == 0 {
        return;
    }

    let mut spans = vec![
        Span::styled(format!(" {BRAND} "), theme.brand),
        Span::raw(" "),
    ];

    // Anything urgent goes here, immediately after the brand, where it is the
    // first thing on the line rather than the last.
    if let Some(leading) = leading {
        spans.push(leading);
    }

    for (index, crumb) in crumbs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            theme.glyphs.breadcrumb.to_string(),
            theme.muted,
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(crumb.clone(), theme.text));
    }

    frame.render_widget(
        Paragraph::new(Line::from(justify(spans, right, area.width as usize))),
        area,
    );
}

/// Renders the status bar: the mode badge on the left, active settings right.
pub fn render_status(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    mode: Mode,
    chips: &[Chip],
) {
    if area.height == 0 {
        return;
    }

    let left = vec![
        Span::styled(
            format!(" {} ", mode.label().to_uppercase()),
            theme.mode_badge(mode),
        ),
        Span::raw(" "),
    ];

    frame.render_widget(
        Paragraph::new(Line::from(justify(
            left,
            chip_spans(chips, theme),
            area.width as usize,
        ))),
        area,
    );
}

/// Renders a pre-built one-line bar, such as the hint bar.
pub fn render_bar(frame: &mut Frame<'_>, area: Rect, line: Line<'static>) {
    if area.height == 0 {
        return;
    }
    frame.render_widget(Paragraph::new(line), area);
}

/// Builds the frame for a pane: title on the left of the border, status right.
///
/// The heavier border is the focus cue. It reads as "this pane has focus" even
/// with color disabled, which a border color alone cannot do.
pub fn pane_block(
    theme: &Theme,
    focused: bool,
    mode: Mode,
    title: &str,
    right: &[Chip],
) -> Block<'static> {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_set(theme.border_set(focused))
        .border_style(theme.pane_border(focused, mode))
        .title_top(
            Line::from(vec![
                Span::raw(" "),
                Span::styled(title.to_string(), theme.pane_title(focused, mode)),
                Span::raw(" "),
            ])
            .left_aligned(),
        );

    if !right.is_empty() {
        let mut spans = vec![Span::raw(" ")];
        spans.extend(chip_spans(right, theme));
        spans.push(Span::raw(" "));
        block = block.title_top(Line::from(spans).right_aligned());
    }

    block
}

/// Renders a centered message, vertically and horizontally, inside `area`.
pub fn render_pane_message(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    message: &PaneMessage,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let width = area.width as usize;
    let mut lines = vec![Line::from(Span::styled(
        pad_cell_centered(&message.text, width, theme.glyphs.ellipsis),
        message.tone.style(theme),
    ))];

    if let Some(hint) = &message.hint {
        lines.push(Line::from(Span::styled(
            pad_cell_centered(hint, width, theme.glyphs.ellipsis),
            theme.muted,
        )));
    }

    let top = area.height.saturating_sub(lines.len() as u16) / 2;
    let target = Rect {
        y: area.y + top,
        height: area.height - top,
        ..area
    };

    frame.render_widget(Paragraph::new(lines), target);
}

/// Renders chips separated by the theme's chip separator.
pub fn chip_spans(chips: &[Chip], theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();

    for chip in chips {
        if !spans.is_empty() {
            spans.push(Span::styled(
                format!(" {} ", theme.glyphs.chip_sep),
                theme.muted,
            ));
        }
        spans.push(Span::styled(chip.text.clone(), chip.tone.style(theme)));
    }

    spans
}

/// Pushes `right` against the right edge, clipping `left` if they would collide.
fn justify(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
) -> Vec<Span<'static>> {
    let right_width = spans_width(&right);
    let mut spans = clip_spans(left, width.saturating_sub(right_width));
    let used = spans_width(&spans);
    spans.push(Span::raw(
        " ".repeat(width.saturating_sub(used + right_width)),
    ));
    spans.extend(clip_spans(right, width));
    spans
}

/// The display width of a chip run, for callers budgeting a border.
pub fn chips_width(chips: &[Chip], theme: &Theme) -> usize {
    visible_width(
        &chip_spans(chips, theme)
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>(),
    )
}

#[cfg(test)]
mod tests {
    use super::{chip_spans, justify, Chip, PaneMessage, Tone};
    use crate::ui::{text::spans_width, theme::Theme};
    use ratatui::text::Span;

    #[test]
    fn justify_right_aligns_the_trailing_run() {
        let line = justify(
            vec![Span::raw("left")],
            vec![Span::raw("right")],
            20,
        );

        assert_eq!(spans_width(&line), 20);
        assert_eq!(
            line.iter().map(|span| span.content.as_ref()).collect::<String>(),
            "left           right"
        );
    }

    #[test]
    fn justify_clips_the_left_run_before_dropping_the_right_one() {
        let line = justify(
            vec![Span::raw("aaaa"), Span::raw("bbbb"), Span::raw("cccc")],
            vec![Span::raw("keep")],
            10,
        );

        let rendered = line
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(rendered.ends_with("keep"));
        assert!(spans_width(&line) <= 10);
    }

    #[test]
    fn chips_are_separated_only_between_entries() {
        let theme = Theme::default();

        let single = chip_spans(&[Chip::new("one")], &theme);
        let pair = chip_spans(&[Chip::new("one"), Chip::toned("two", Tone::Warn)], &theme);

        assert_eq!(single.len(), 1);
        assert_eq!(pair.len(), 3);
        assert_eq!(pair[1].content.as_ref(), " · ");
    }

    #[test]
    fn pane_messages_carry_an_optional_next_step() {
        let message = PaneMessage::new("No tasks", Tone::Muted).with_hint("r to refresh");

        assert_eq!(message.hint.as_deref(), Some("r to refresh"));
    }
}
