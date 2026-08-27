//! Semantic styles and glyphs shared by every UI renderer.
//!
//! Renderers never build a [`Color`] directly. They ask the [`Theme`] for a
//! *role* (`muted`, `danger`, `border_focus`, ...) so the whole UI can be
//! restyled, downgraded to a monochrome terminal, or re-accented from config in
//! one place.
//!
//! Two independent axes are configurable:
//!
//! - [`ThemeVariant`] decides how colors resolve. `Ansi` uses the terminal's own
//!   16-color palette so the UI inherits the user's chosen scheme, `Truecolor`
//!   adds a few indexed shades for subtle backgrounds, and `Mono` drops color
//!   entirely and leans on bold/dim/reverse.
//! - [`ThemeGlyphs`] decides which characters are drawn. Every glyph in both
//!   sets is exactly one cell wide so column math stays valid.

use ratatui::{
    style::{Color, Modifier, Style},
    symbols::border,
};

use crate::config::{Mode, ThemeConfig, ThemeGlyphs, ThemeVariant};

/// The characters the UI draws for markers, rules, chips, and spinners.
///
/// Every field is a single display cell wide; `ui::text::visible_width` is used
/// in tests to prove it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphSet {
    /// Cursor marker in the left gutter.
    pub cursor: &'static str,
    /// Multi-select marker in the left gutter.
    pub selected: &'static str,
    /// Starred project marker.
    pub star_on: &'static str,
    /// Unstarred project marker.
    pub star_off: &'static str,
    /// Marker for the pinned "assigned to me" row.
    pub pinned: &'static str,
    /// Open task marker.
    pub open: &'static str,
    /// Completed task marker.
    pub done: &'static str,
    /// Subtask indent marker.
    pub subtask: &'static str,
    /// Fill character for a project group rule.
    pub rule: &'static str,
    /// Fill character for a section group rule.
    pub section_rule: &'static str,
    /// Leading bar drawn before a project group label.
    pub group_bar: &'static str,
    /// Vertical rule drawn between table columns.
    pub column_rule: &'static str,
    /// Separator between status/count chips.
    pub chip_sep: &'static str,
    /// Separator between header breadcrumb segments.
    pub breadcrumb: &'static str,
    /// Prefix for an overdue due date.
    pub overdue: &'static str,
    /// Placeholder for an empty cell or unset filter.
    pub empty: &'static str,
    /// Truncation marker.
    pub ellipsis: &'static str,
    /// Insertion caret shown while editing a filter.
    pub edit_cursor: &'static str,
    /// Ascending sort indicator.
    pub sort_asc: &'static str,
    /// Descending sort indicator.
    pub sort_desc: &'static str,
    /// Marker for a filter that is doing something.
    pub active: &'static str,
    /// Animation frames for the loading spinner.
    pub spinner: &'static [&'static str],
}

impl GlyphSet {
    /// The default set: plain Unicode, no Nerd Font or powerline glyphs.
    pub const UNICODE: Self = Self {
        cursor: "▍",
        selected: "●",
        star_on: "★",
        star_off: "☆",
        pinned: "◆",
        open: "○",
        done: "✓",
        subtask: "↳",
        rule: "─",
        section_rule: "·",
        group_bar: "▌",
        column_rule: "│",
        chip_sep: "·",
        breadcrumb: "▸",
        overdue: "!",
        empty: "—",
        ellipsis: "…",
        edit_cursor: "▏",
        sort_asc: "▲",
        sort_desc: "▼",
        active: "●",
        spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
    };

    /// The fallback set for terminals with ambiguous-width or missing glyphs.
    pub const ASCII: Self = Self {
        cursor: ">",
        selected: "*",
        star_on: "*",
        star_off: "-",
        pinned: "+",
        open: "o",
        done: "x",
        subtask: "\\",
        rule: "-",
        section_rule: ".",
        group_bar: "|",
        column_rule: "|",
        chip_sep: "|",
        breadcrumb: ">",
        overdue: "!",
        empty: "-",
        ellipsis: "~",
        edit_cursor: "_",
        sort_asc: "^",
        sort_desc: "v",
        active: "*",
        spinner: &["|", "/", "-", "\\"],
    };

    /// Returns the glyph set selected by config.
    pub fn for_config(glyphs: ThemeGlyphs) -> Self {
        match glyphs {
            ThemeGlyphs::Unicode => Self::UNICODE,
            ThemeGlyphs::Ascii => Self::ASCII,
        }
    }

    /// Returns the spinner frame for a monotonically increasing tick.
    pub fn spinner_frame(&self, tick: usize) -> &'static str {
        self.spinner[tick % self.spinner.len()]
    }
}

/// Resolved styles for every semantic role in the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// The glyph vocabulary to draw with.
    pub glyphs: GlyphSet,
    /// Ordinary body text.
    pub text: Style,
    /// De-emphasized text: placeholders, counts, secondary labels.
    pub muted: Style,
    /// Section-level headings inside a pane.
    pub subtitle: Style,
    /// Pane titles.
    pub title: Style,
    /// The configured accent color.
    pub accent: Style,
    /// Border of an unfocused pane.
    pub border: Style,
    /// Border of the focused pane.
    pub border_focus: Style,
    /// Applied to the whole row under the cursor.
    pub cursor: Style,
    /// Gutter markers for cursor and multi-select.
    pub marker: Style,
    /// Starred marker.
    pub star: Style,
    /// Hidden projects.
    pub hidden: Style,
    /// Success / completed.
    pub ok: Style,
    /// Needs attention soon.
    pub warn: Style,
    /// Overdue / failed.
    pub danger: Style,
    /// Informational highlight.
    pub info: Style,
    /// The table header row.
    pub header: Style,
    /// A key name in the hint bar or help overlay.
    pub key: Style,
    /// The app-name badge in the header bar.
    pub brand: Style,
    /// Optional background for alternating task rows.
    pub zebra: Option<Style>,
    /// Whether color was resolved away entirely.
    mono: bool,
    /// Whether only ASCII may be drawn.
    ascii: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(&ThemeConfig::default())
    }
}

impl Theme {
    /// Builds a theme from config, honoring the `NO_COLOR` convention.
    ///
    /// `NO_COLOR` being set (to anything) forces the monochrome variant and the
    /// ASCII glyph set, because a terminal that cannot show color often cannot
    /// show box-drawing glyphs either.
    pub fn new(config: &ThemeConfig) -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            return Self::build(ThemeVariant::Mono, ThemeGlyphs::Ascii, Color::Reset, false);
        }

        Self::build(
            config.variant,
            config.glyphs,
            accent_color(&config.accent),
            config.zebra,
        )
    }

    fn build(
        variant: ThemeVariant,
        glyphs: ThemeGlyphs,
        accent: Color,
        zebra: bool,
    ) -> Self {
        let ascii = matches!(glyphs, ThemeGlyphs::Ascii);
        let glyphs = GlyphSet::for_config(glyphs);
        let plain = Style::default();

        if matches!(variant, ThemeVariant::Mono) {
            // Without color, weight and reverse-video carry the whole design.
            return Self {
                glyphs,
                text: plain,
                muted: plain.add_modifier(Modifier::DIM),
                subtitle: plain.add_modifier(Modifier::BOLD),
                title: plain.add_modifier(Modifier::BOLD),
                accent: plain.add_modifier(Modifier::BOLD),
                border: plain.add_modifier(Modifier::DIM),
                border_focus: plain.add_modifier(Modifier::BOLD),
                cursor: plain.add_modifier(Modifier::REVERSED),
                marker: plain.add_modifier(Modifier::BOLD),
                star: plain.add_modifier(Modifier::BOLD),
                hidden: plain.add_modifier(Modifier::DIM),
                ok: plain.add_modifier(Modifier::DIM),
                warn: plain.add_modifier(Modifier::BOLD),
                danger: plain.add_modifier(Modifier::BOLD),
                info: plain,
                header: plain.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                key: plain.add_modifier(Modifier::BOLD),
                brand: plain.add_modifier(Modifier::REVERSED | Modifier::BOLD),
                zebra: None,
                mono: true,
                ascii,
            };
        }

        // A dim gray works as a subtle band on both light and dark terminals;
        // the deeper shades are only safe when we know 256 colors are available.
        let cursor_bg = Color::Indexed(8);
        let zebra_style = match variant {
            ThemeVariant::Truecolor if zebra => Some(plain.bg(Color::Indexed(236))),
            _ => None,
        };

        Self {
            glyphs,
            text: plain,
            muted: plain.fg(Color::DarkGray),
            subtitle: plain.fg(Color::Blue).add_modifier(Modifier::BOLD),
            title: plain.fg(accent).add_modifier(Modifier::BOLD),
            accent: plain.fg(accent),
            border: plain.fg(Color::DarkGray),
            border_focus: plain.fg(accent),
            cursor: plain.bg(cursor_bg).add_modifier(Modifier::BOLD),
            marker: plain.fg(accent).add_modifier(Modifier::BOLD),
            star: plain.fg(Color::Yellow),
            hidden: plain.fg(Color::DarkGray),
            ok: plain.fg(Color::Green),
            warn: plain.fg(Color::Yellow),
            danger: plain.fg(Color::Red),
            info: plain.fg(Color::Blue),
            header: plain
                .fg(accent)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            key: plain.fg(accent).add_modifier(Modifier::BOLD),
            brand: plain
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
            zebra: zebra_style,
            mono: false,
            ascii,
        }
    }

    /// The border characters for a pane, respecting the glyph set.
    ///
    /// The focused pane gets a heavier border. That is the focus cue that
    /// survives with color disabled, so the ASCII set keeps a weight
    /// distinction too rather than falling back to one box style.
    pub fn border_set(&self, focused: bool) -> border::Set {
        if !self.ascii {
            return if focused { border::THICK } else { border::PLAIN };
        }

        border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: if focused { "#" } else { "|" },
            vertical_right: if focused { "#" } else { "|" },
            horizontal_top: if focused { "=" } else { "-" },
            horizontal_bottom: if focused { "=" } else { "-" },
        }
    }

    /// The badge style for a bind mode, matched to that mode's pane border.
    ///
    /// Mode and focus share a color so "which pane am I driving" is answerable
    /// from either end of the screen.
    pub fn mode_badge(&self, mode: Mode) -> Style {
        let color = self.mode_color(mode);
        match color {
            Some(color) => Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
            None => self.brand,
        }
    }

    /// The border style for a pane, colored by mode when it has focus.
    pub fn pane_border(&self, focused: bool, mode: Mode) -> Style {
        if !focused {
            return self.border;
        }
        match self.mode_color(mode) {
            Some(color) => Style::default().fg(color),
            None => self.border_focus,
        }
    }

    /// The title style for a pane, colored by mode when it has focus.
    pub fn pane_title(&self, focused: bool, mode: Mode) -> Style {
        if !focused {
            return self.muted.add_modifier(Modifier::BOLD);
        }
        match self.mode_color(mode) {
            Some(color) => Style::default()
                .fg(color)
                .add_modifier(Modifier::BOLD),
            None => self.title,
        }
    }

    fn mode_color(&self, mode: Mode) -> Option<Color> {
        if self.mono {
            return None;
        }
        Some(match mode {
            Mode::Project => Color::Blue,
            Mode::ProjectSearch => Color::Yellow,
            Mode::Filter => Color::Magenta,
            Mode::FilterEdit => Color::Yellow,
            // Editing, like the other edit modes: the overlay title says which
            // field, so the color only needs to signal "you are typing".
            Mode::Calendar => Color::Yellow,
            Mode::Task => Color::Green,
            Mode::Any => return None,
        })
    }
}

fn accent_color(name: &str) -> Color {
    match name.trim().to_ascii_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        _ => Color::Cyan,
    }
}

#[cfg(test)]
mod tests {
    use super::{GlyphSet, Theme};
    use crate::config::{Mode, ThemeConfig, ThemeGlyphs, ThemeVariant};
    use crate::ui::text::visible_width;

    fn all_glyphs(set: &GlyphSet) -> Vec<&'static str> {
        let mut glyphs = vec![
            set.cursor,
            set.selected,
            set.star_on,
            set.star_off,
            set.pinned,
            set.open,
            set.done,
            set.subtask,
            set.rule,
            set.section_rule,
            set.group_bar,
            set.column_rule,
            set.chip_sep,
            set.breadcrumb,
            set.overdue,
            set.empty,
            set.ellipsis,
            set.edit_cursor,
            set.sort_asc,
            set.sort_desc,
            set.active,
        ];
        glyphs.extend_from_slice(set.spinner);
        glyphs
    }

    #[test]
    fn every_glyph_is_exactly_one_cell_wide() {
        for set in [GlyphSet::UNICODE, GlyphSet::ASCII] {
            for glyph in all_glyphs(&set) {
                assert_eq!(visible_width(glyph), 1, "glyph {glyph:?} is not 1 cell wide");
            }
        }
    }

    #[test]
    fn mono_variant_resolves_every_role_without_color() {
        let theme = Theme::new(&ThemeConfig {
            variant: ThemeVariant::Mono,
            glyphs: ThemeGlyphs::Ascii,
            accent: "cyan".to_string(),
            zebra: false,
        });

        for style in [
            theme.text,
            theme.muted,
            theme.subtitle,
            theme.title,
            theme.accent,
            theme.border,
            theme.border_focus,
            theme.cursor,
            theme.marker,
            theme.header,
            theme.key,
            theme.danger,
        ] {
            assert!(style.fg.is_none(), "mono style should not set a foreground");
            assert!(style.bg.is_none(), "mono style should not set a background");
        }
        assert_eq!(theme.pane_border(true, Mode::Task), theme.border_focus);
        assert!(theme.zebra.is_none());
    }

    #[test]
    fn focused_pane_border_follows_the_active_mode() {
        let theme = Theme::new(&ThemeConfig::default());

        assert_ne!(
            theme.pane_border(true, Mode::Task),
            theme.pane_border(true, Mode::Project)
        );
        assert_eq!(theme.pane_border(false, Mode::Task), theme.border);
    }

    #[test]
    fn the_ascii_glyph_set_uses_ascii_borders_and_keeps_a_focus_weight() {
        let ascii = Theme::new(&ThemeConfig {
            variant: ThemeVariant::Ansi,
            glyphs: ThemeGlyphs::Ascii,
            accent: "cyan".to_string(),
            zebra: false,
        });
        let unicode = Theme::new(&ThemeConfig::default());

        for focused in [false, true] {
            let set = ascii.border_set(focused);
            for symbol in [
                set.top_left,
                set.top_right,
                set.bottom_left,
                set.bottom_right,
                set.vertical_left,
                set.vertical_right,
                set.horizontal_top,
                set.horizontal_bottom,
            ] {
                assert!(symbol.is_ascii(), "border symbol {symbol:?} is not ascii");
            }
        }

        assert_ne!(
            ascii.border_set(true).horizontal_top,
            ascii.border_set(false).horizontal_top
        );
        assert_ne!(
            unicode.border_set(true).horizontal_top,
            unicode.border_set(false).horizontal_top
        );
    }

    #[test]
    fn zebra_is_only_enabled_for_the_truecolor_variant() {
        let ansi = Theme::new(&ThemeConfig {
            variant: ThemeVariant::Ansi,
            glyphs: ThemeGlyphs::Unicode,
            accent: "cyan".to_string(),
            zebra: true,
        });
        let truecolor = Theme::new(&ThemeConfig {
            variant: ThemeVariant::Truecolor,
            glyphs: ThemeGlyphs::Unicode,
            accent: "cyan".to_string(),
            zebra: true,
        });

        assert!(ansi.zebra.is_none());
        assert!(truecolor.zebra.is_some());
    }
}
