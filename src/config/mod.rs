//! Parsed application configuration and binding mode definitions.
//!
//! `Config` is the single source of truth for the loaded TOML contents and the
//! path they came from. The app loads it once at startup, then uses the stored
//! source path later when persisting project visibility changes back to disk.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::domain::GanttColorKey;
use crate::error::{Error, Result};

/// Parsed application configuration.
///
/// This holds the user-facing TOML data plus the source path the config was
/// loaded from so the app can persist changes back to the same file.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub header: Header,
    #[serde(default)]
    #[serde(skip_serializing_if = "ThemeConfig::is_default")]
    pub theme: ThemeConfig,
    #[serde(default)]
    #[serde(skip_serializing_if = "GanttConfig::is_default")]
    pub gantt: GanttConfig,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthConfig>,
    #[serde(default)]
    pub bind: Vec<Bind>,
    #[serde(default, rename = "project")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub project_visibility: Vec<ProjectVisibilityConfig>,
    #[serde(default, rename = "filter_set")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filter_sets: Vec<NamedFilterSet>,
    #[serde(skip)]
    source_path: Option<PathBuf>,
}

impl PartialEq for Config {
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header
            && self.theme == other.theme
            && self.gantt == other.gantt
            && self.auth == other.auth
            && self.bind == other.bind
            && self.project_visibility == other.project_visibility
            && self.filter_sets == other.filter_sets
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            header: Header::default(),
            theme: ThemeConfig::default(),
            gantt: GanttConfig::default(),
            auth: None,
            bind: default_bindings(),
            project_visibility: Vec::new(),
            filter_sets: Vec::new(),
            source_path: None,
        }
    }
}

impl Config {
    /// Parse config from an in-memory TOML string.
    pub fn from_toml_str(input: &str) -> Result<Self> {
        let config: Self = toml::from_str(input)?;
        config.validate()?;
        Ok(config)
    }

    /// Load config from a TOML file path and remember that path for later saves.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut config = if !path.exists() {
            Self::default()
        } else {
            let contents = fs::read_to_string(path)?;
            Self::from_toml_str(&contents)?
        };
        config.source_path = Some(path.to_path_buf());
        Ok(config)
    }

    /// Save the current config back to the path it was loaded from.
    ///
    /// If no source path is known, this is a no-op.
    pub fn save_to_source_path(&self) -> Result<()> {
        if let Some(path) = self.source_path.as_deref() {
            let contents = toml::to_string_pretty(self).map_err(|err| {
                Error::ConfigValidation(format!("failed to serialize config: {err}"))
            })?;
            fs::write(path, contents)?;
        }
        Ok(())
    }

    /// Return the path the config was loaded from, if one is known.
    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    /// Merge the built-in default bindings with any user-defined overrides.
    pub fn effective_bindings(&self) -> Vec<Bind> {
        let mut bindings = default_bindings();
        bindings.extend(self.bind.clone());
        bindings
    }

    /// The named filter sets in the order the sidebar lists them.
    ///
    /// By name, case-insensitively, rather than by file order: the sidebar
    /// numbers the rows it shows and the digits address those numbers, so the
    /// order has to be one the user can predict from the names alone.
    pub fn sorted_filter_sets(&self) -> Vec<&NamedFilterSet> {
        let mut entries = self.filter_sets.iter().collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.name.to_lowercase());
        entries
    }

    fn validate(&self) -> Result<()> {
        if self.header.version != Header::EXPECTED_VERSION {
            return Err(Error::ConfigValidation(format!(
                "expected header.version = {}, found {}",
                Header::EXPECTED_VERSION,
                self.header.version
            )));
        }

        if self.header.kind.trim().is_empty() {
            return Err(Error::ConfigValidation(
                "header.type must not be empty".to_string(),
            ));
        }

        self.theme.validate()?;
        self.gantt.validate()?;

        if let Some(auth) = &self.auth {
            auth.validate()?;
        }

        let mut seen_project_ids = HashSet::new();
        for project in &self.project_visibility {
            project.validate()?;
            if !seen_project_ids.insert(project.gid.as_str()) {
                return Err(Error::ConfigValidation(format!(
                    "duplicate project.gid value: {}",
                    project.gid
                )));
            }
        }

        let mut seen_names = HashSet::new();
        for entry in &self.filter_sets {
            entry.validate()?;
            // The sidebar lists entries by number and two rows reading the
            // same is a trap: there would be no way to tell which one a digit
            // loads, or which one `w` overwrites.
            if !seen_names.insert(entry.name.trim().to_lowercase()) {
                return Err(Error::ConfigValidation(format!(
                    "duplicate filter_set.name value: {}",
                    entry.name
                )));
            }
        }

        Ok(())
    }
}

/// The match modes a saved string filter may name.
const SAVED_MATCH_MODES: [&str; 3] = ["fuzzy", "contains", "regex"];

/// One named filter set: a whole filter panel, saved under a name.
///
/// Every tab, each tab's negation, and each field's query, match mode,
/// require-empty flag, and negation — one name for one complete filter
/// expression, including the union across tabs that a single tab cannot say.
///
/// The scalars come before the `Vec`, here and in the two types below,
/// because `toml`'s serializer cannot emit a value after it has emitted a
/// table. Getting the order wrong round-trips fine through `Value` and fails
/// at [`Config::save_to_source_path`], on a real user's config.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct NamedFilterSet {
    pub name: String,
    /// The ORed sets, in tab order.
    #[serde(default, rename = "set", skip_serializing_if = "Vec::is_empty")]
    pub sets: Vec<SavedFilterSet>,
}

impl NamedFilterSet {
    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::ConfigValidation(
                "filter_set.name must not be empty".to_string(),
            ));
        }

        for set in &self.sets {
            set.validate()?;
        }

        Ok(())
    }
}

/// One tab of a named filter set.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SavedFilterSet {
    #[serde(default, skip_serializing_if = "is_false")]
    pub negated: bool,
    /// Only the fields that filter something; an untouched row is not written.
    #[serde(default, rename = "field", skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<SavedFilterField>,
}

impl SavedFilterSet {
    fn validate(&self) -> Result<()> {
        for field in &self.fields {
            field.validate()?;
        }
        Ok(())
    }
}

/// One filter row's value, keyed the way the panel keys its rows.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SavedFilterField {
    /// `title`, `assignee`, `due`, `start`, `state`, `projects`, or
    /// `custom:<Name>` — the panel's own row key, which is keyed by custom
    /// field *name* precisely so it survives a reload that brings different
    /// ids.
    ///
    /// An unknown key is **not** rejected: a `custom:Priority` belonging to a
    /// project that is not loaded this session is legitimate, and the panel
    /// parks it rather than dropping it.
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub query: String,
    /// `fuzzy` | `contains` | `regex`. Omitted when it is the row's default.
    #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
    pub string_mode: Option<String>,
    /// The row requires no value at all.
    #[serde(default, skip_serializing_if = "is_false")]
    pub empty: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub negated: bool,
}

impl SavedFilterField {
    fn validate(&self) -> Result<()> {
        if self.key.trim().is_empty() {
            return Err(Error::ConfigValidation(
                "filter_set.set.field.key must not be empty".to_string(),
            ));
        }

        if let Some(mode) = &self.string_mode {
            if !SAVED_MATCH_MODES.contains(&mode.as_str()) {
                return Err(Error::ConfigValidation(format!(
                    "filter_set.set.field.match must be one of {}, found {mode}",
                    SAVED_MATCH_MODES.join(", ")
                )));
            }
        }

        Ok(())
    }
}

/// `skip_serializing_if` predicate for flags that default to off.
fn is_false(value: &bool) -> bool {
    !*value
}

/// How the UI resolves colors for the terminal.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThemeVariant {
    /// Use the terminal's own 16-color palette so the UI inherits its scheme.
    #[default]
    Ansi,
    /// Additionally use indexed shades for subtle backgrounds such as zebra rows.
    Truecolor,
    /// Emit no color at all; rely on bold, dim, and reverse video.
    Mono,
}

/// Which character set the UI draws markers and rules with.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThemeGlyphs {
    /// Plain Unicode box-drawing and symbol glyphs.
    #[default]
    Unicode,
    /// ASCII-only fallback for terminals with ambiguous-width fonts.
    Ascii,
}

/// The accent colors the theme accepts.
const ACCENT_COLORS: [&str; 9] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "gray",
];

/// User-facing appearance settings.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ThemeConfig {
    /// How colors resolve for this terminal.
    #[serde(default)]
    pub variant: ThemeVariant,
    /// Which glyph set to draw with.
    #[serde(default)]
    pub glyphs: ThemeGlyphs,
    /// The accent color used for focus, titles, and key names.
    #[serde(default = "default_accent")]
    pub accent: String,
    /// Whether to stripe alternating task rows. Requires the truecolor variant.
    #[serde(default)]
    pub zebra: bool,
}

fn default_accent() -> String {
    "cyan".to_string()
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            variant: ThemeVariant::default(),
            glyphs: ThemeGlyphs::default(),
            accent: default_accent(),
            zebra: false,
        }
    }
}

impl ThemeConfig {
    /// Returns `true` when nothing has been customized.
    ///
    /// Used to keep the `[theme]` table out of configs the app rewrites when
    /// persisting project visibility.
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    fn validate(&self) -> Result<()> {
        let accent = self.accent.trim().to_ascii_lowercase();
        if !ACCENT_COLORS.contains(&accent.as_str()) {
            return Err(Error::ConfigValidation(format!(
                "theme.accent must be one of {}, found {}",
                ACCENT_COLORS.join(", "),
                self.accent
            )));
        }
        Ok(())
    }
}

/// Gantt chart appearance settings.
///
/// This is the *serialization* of what the gantt and colour-dialog modes set,
/// not the way to set them. Committing the colour dialog writes `color_by` and
/// that dimension's `order` back here; nothing else in the section is ever
/// written by the app.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GanttConfig {
    /// Whether the chart is drawn when the app starts.
    #[serde(default)]
    pub visible: bool,
    /// How many table columns stay visible beside the chart.
    #[serde(default = "default_gantt_columns")]
    pub columns: usize,
    /// Which dimension colours the bars, in [`GanttColorKey`]'s spelling.
    #[serde(default = "default_gantt_color_by")]
    pub color_by: String,
    /// Per-dimension colour order, keyed the same way as `color_by`.
    ///
    /// A `BTreeMap` rather than a `HashMap` so a config the app rewrites has a
    /// stable key order and does not churn in version control.
    #[serde(default)]
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub order: BTreeMap<String, Vec<String>>,
}

fn default_gantt_columns() -> usize {
    2
}

fn default_gantt_color_by() -> String {
    "assignee".to_string()
}

impl Default for GanttConfig {
    fn default() -> Self {
        Self {
            visible: false,
            columns: default_gantt_columns(),
            color_by: default_gantt_color_by(),
            order: BTreeMap::new(),
        }
    }
}

impl GanttConfig {
    /// Returns `true` when nothing has been customized.
    ///
    /// Keeps the `[gantt]` table out of configs the app rewrites when
    /// persisting project visibility, exactly as `[theme]` is kept out.
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// The parsed colour key, falling back to the default if it is unreadable.
    ///
    /// Validation rejects an unparseable value at load, so the fallback only
    /// covers a `GanttConfig` built in code rather than read from disk.
    pub fn color_key(&self) -> GanttColorKey {
        self.color_by
            .parse()
            .unwrap_or(GanttColorKey::Assignee)
    }

    /// The configured colour order for one dimension, empty when unset.
    pub fn order_for(&self, key: &GanttColorKey) -> &[String] {
        self.order
            .get(&key.to_string())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn validate(&self) -> Result<()> {
        if self.columns == 0 {
            return Err(Error::ConfigValidation(
                "gantt.columns must be at least 1".to_string(),
            ));
        }

        if self.color_by.parse::<GanttColorKey>().is_err() {
            return Err(Error::ConfigValidation(format!(
                "gantt.color_by must be assignee, section, state, or field:<Name>, found {}",
                self.color_by
            )));
        }

        // A mistyped key would otherwise sit in the file looking effective
        // while ordering nothing.
        for key in self.order.keys() {
            if key.parse::<GanttColorKey>().is_err() {
                return Err(Error::ConfigValidation(format!(
                    "gantt.order key must be assignee, section, state, or field:<Name>, found {key}"
                )));
            }
        }

        Ok(())
    }
}

/// Keybinding lookup context.
///
/// The concrete variants represent the current UI/input state. `Any` is only
/// used for binding fallback when a more specific context does not match.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Any,
    Project,
    ProjectSearch,
    Filter,
    FilterEdit,
    /// Typing a name for `w`, or answering the `d` confirmation.
    FilterSetName,
    Calendar,
    Task,
    /// A task table cell is open for editing.
    TaskEdit,
    Gantt,
    GanttOrder,
}

impl Default for Mode {
    fn default() -> Self {
        Self::Any
    }
}

impl Mode {
    pub fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }

    pub fn allows_any_fallback(&self) -> bool {
        !matches!(
            self,
            Self::ProjectSearch
                | Self::FilterEdit
                | Self::FilterSetName
                | Self::Calendar
                // An unbound letter has to type into the cell rather than
                // fire the global binding that letter carries.
                | Self::TaskEdit
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Project => "project",
            Self::ProjectSearch => "project-search",
            Self::Filter => "filter",
            Self::FilterEdit => "filter-edit",
            Self::FilterSetName => "set name",
            Self::Calendar => "calendar",
            Self::Task => "task",
            Self::TaskEdit => "task edit",
            Self::Gantt => "gantt",
            Self::GanttOrder => "colors",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
/// Asana authentication settings loaded from config.
pub struct AuthConfig {
    pub personal_access_token: String,
    #[serde(default)]
    pub workspace_gid: Option<String>,
}

impl AuthConfig {
    fn validate(&self) -> Result<()> {
        if self.personal_access_token.trim().is_empty() {
            return Err(Error::ConfigValidation(
                "auth.personal_access_token must not be empty".to_string(),
            ));
        }

        if self
            .workspace_gid
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(Error::ConfigValidation(
                "auth.workspace_gid must not be empty when provided".to_string(),
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
/// Per-project visibility preferences persisted in `tuisana.toml`.
pub struct ProjectVisibilityConfig {
    pub gid: String,
    #[serde(default)]
    pub starred: bool,
    #[serde(default)]
    pub hidden: bool,
}

impl ProjectVisibilityConfig {
    fn validate(&self) -> Result<()> {
        if self.gid.trim().is_empty() {
            return Err(Error::ConfigValidation(
                "project.gid must not be empty".to_string(),
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
/// Config file header used to validate schema/version compatibility.
pub struct Header {
    #[serde(rename = "type", default = "default_header_type")]
    pub kind: String,
    #[serde(default = "default_header_version")]
    pub version: f64,
}

impl Header {
    const EXPECTED_VERSION: f64 = 1.0;
}

impl Default for Header {
    fn default() -> Self {
        Self {
            kind: default_header_type(),
            version: default_header_version(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
/// A keybinding entry loaded from config.
pub struct Bind {
    pub key: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Mode::is_any")]
    pub mode: Mode,
    pub command: String,
}

impl Bind {
    pub fn new(key: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            mode: Mode::Any,
            command: command.into(),
        }
    }

    pub fn with_mode(
        key: impl Into<String>,
        mode: Mode,
        command: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            mode,
            command: command.into(),
        }
    }
}

fn default_header_type() -> String {
    "tuisana".to_string()
}

fn default_header_version() -> f64 {
    1.0
}

fn default_bindings() -> Vec<Bind> {
    vec![
        Bind::new("?", "toggle_help_details"),
        Bind::new("q", "quit"),
        Bind::new("ctrl-c", "quit"),
        Bind::new("k", "move_up"),
        Bind::new("up", "move_up"),
        Bind::new("j", "move_down"),
        Bind::new("down", "move_down"),
        Bind::new("ctrl-u", "page_up"),
        Bind::new("ctrl-d", "page_down"),
        Bind::new("home", "jump_top"),
        Bind::new("end", "jump_bottom"),
        Bind::new("left", "scroll_left"),
        Bind::new("right", "scroll_right"),
        Bind::new("f", "set_filter_mode"),
        Bind::new("p", "set_project_mode"),
        Bind::new("t", "set_task_mode"),
        Bind::new("m", "toggle_task_mode"),
        Bind::new("[", "resize_top_pane_down"),
        Bind::new("]", "resize_top_pane_up"),
        Bind::new("{", "minimize_top_pane"),
        Bind::new("}", "maximize_top_pane"),
        Bind::new("0", "restore_top_pane"),
        Bind::with_mode("enter", Mode::Project, "open"),
        Bind::new("r", "refresh"),
        Bind::with_mode("/", Mode::Project, "start_search"),
        Bind::with_mode("ctrl-l", Mode::ProjectSearch, "clear_search"),
        Bind::with_mode("space", Mode::Project, "toggle_selection"),
        Bind::with_mode("a", Mode::Project, "select_all_visible"),
        Bind::with_mode("!", Mode::Project, "select_all_starred_visible"),
        Bind::with_mode("@", Mode::Project, "select_all_non_hidden_visible"),
        Bind::with_mode("i", Mode::Project, "invert_selection"),
        Bind::with_mode("c", Mode::Project, "clear_selection"),
        Bind::with_mode("u", Mode::Project, "undo_selection"),
        Bind::with_mode("ctrl-y", Mode::Project, "redo_selection"),
        Bind::with_mode("*", Mode::Project, "toggle_starred_selected"),
        Bind::with_mode("h", Mode::Project, "toggle_hidden_selected"),
        Bind::with_mode("v", Mode::Project, "toggle_hidden_group"),
        Bind::with_mode("enter", Mode::Filter, "begin_filter_edit"),
        Bind::with_mode("esc", Mode::Filter, "set_task_mode"),
        Bind::with_mode("f", Mode::Filter, "toggle_task_filters"),
        Bind::with_mode("s", Mode::Filter, "cycle_filter_string_mode"),
        Bind::with_mode("ctrl-l", Mode::Filter, "clear_search"),
        Bind::with_mode("ctrl-z", Mode::Filter, "search_fuzzy"),
        Bind::with_mode("ctrl-s", Mode::Filter, "search_substring"),
        Bind::with_mode("ctrl-r", Mode::Filter, "search_regex"),
        Bind::with_mode("l", Mode::Filter, "filter_set_next"),
        Bind::with_mode("h", Mode::Filter, "filter_set_prev"),
        Bind::with_mode("a", Mode::Filter, "filter_set_add"),
        Bind::with_mode("x", Mode::Filter, "filter_set_remove"),
        Bind::with_mode("e", Mode::Filter, "filter_require_empty"),
        // `!` is the standard "not" on a keyboard, and `~` is the other one —
        // the pair keeps the two negations next to each other rather than
        // giving the set-level one a letter that reads as a word.
        Bind::with_mode("!", Mode::Filter, "filter_negate_field"),
        Bind::with_mode("~", Mode::Filter, "filter_negate_set"),
        // The named-set keys. `<` and `>` are safe here: the input layer
        // lowercases letters, so a shifted *letter* is unreachable, but
        // shifted punctuation arrives as its own character — and `,`/`.` are
        // bound in task mode only.
        Bind::with_mode("b", Mode::Filter, "filter_sets_toggle"),
        Bind::with_mode("w", Mode::Filter, "filter_set_save"),
        Bind::with_mode("y", Mode::Filter, "filter_set_copy_to_new"),
        Bind::with_mode("n", Mode::Filter, "filter_set_new"),
        Bind::with_mode("d", Mode::Filter, "filter_set_delete"),
        Bind::with_mode("<", Mode::Filter, "filter_sets_page_back"),
        Bind::with_mode(">", Mode::Filter, "filter_sets_page_forward"),
        // All nine digits spelled out, so a user can rebind any of them.
        // `0` is left alone: it is `restore_top_pane` globally.
        Bind::with_mode("1", Mode::Filter, "filter_set_load_1"),
        Bind::with_mode("2", Mode::Filter, "filter_set_load_2"),
        Bind::with_mode("3", Mode::Filter, "filter_set_load_3"),
        Bind::with_mode("4", Mode::Filter, "filter_set_load_4"),
        Bind::with_mode("5", Mode::Filter, "filter_set_load_5"),
        Bind::with_mode("6", Mode::Filter, "filter_set_load_6"),
        Bind::with_mode("7", Mode::Filter, "filter_set_load_7"),
        Bind::with_mode("8", Mode::Filter, "filter_set_load_8"),
        Bind::with_mode("9", Mode::Filter, "filter_set_load_9"),
        // Letters type while editing, so the require-empty key needs a ctrl-
        // pair there. ctrl-e is free in filter-edit mode; in calendar mode it
        // is already "jump to the end of a range", which is why the date
        // fields' require-empty is set from filter-browse mode, before the
        // picker opens.
        // `ctrl-e` is "end of line" everywhere text is edited, so the
        // require-empty toggle moved to `ctrl-q` rather than keep the emacs
        // key for a filter-only concept.
        Bind::with_mode("ctrl-q", Mode::FilterEdit, "filter_require_empty"),
        // `!` and `~` are ordinary characters in a query, so the negations need
        // ctrl- pairs here for the same reason require-empty does.
        Bind::with_mode("ctrl-n", Mode::FilterEdit, "filter_negate_field"),
        Bind::with_mode("ctrl-t", Mode::FilterEdit, "filter_negate_set"),
        Bind::with_mode("enter", Mode::FilterEdit, "filter_done_editing"),
        Bind::with_mode("esc", Mode::FilterEdit, "filter_cancel_editing"),
        Bind::with_mode("ctrl-l", Mode::FilterEdit, "clear_search"),
        Bind::with_mode("ctrl-z", Mode::FilterEdit, "search_fuzzy"),
        Bind::with_mode("ctrl-s", Mode::FilterEdit, "search_substring"),
        Bind::with_mode("ctrl-r", Mode::FilterEdit, "search_regex"),
        Bind::with_mode("h", Mode::FilterEdit, "filter_move_label_left"),
        Bind::with_mode("l", Mode::FilterEdit, "filter_move_label_right"),
        Bind::with_mode("j", Mode::FilterEdit, "filter_cycle_label_down"),
        Bind::with_mode("k", Mode::FilterEdit, "filter_cycle_label_up"),
        Bind::with_mode("a", Mode::FilterEdit, "filter_add_label"),
        Bind::with_mode("d", Mode::FilterEdit, "filter_delete_label"),
        // Text fields get the same caret motion the date picker has. `search_fuzzy`
        // moved to ctrl-z to free ctrl-f, so the motion keys mean the same thing
        // in every mode that edits text.
        Bind::with_mode("left", Mode::FilterEdit, "filter_caret_left"),
        Bind::with_mode("right", Mode::FilterEdit, "filter_caret_right"),
        Bind::with_mode("ctrl-b", Mode::FilterEdit, "filter_caret_left"),
        Bind::with_mode("ctrl-f", Mode::FilterEdit, "filter_caret_right"),
        // The emacs motions, shared with task-edit mode so a text field
        // behaves the same wherever it is. `alt-` needs the terminal to send
        // Option as a modifier; see the README.
        Bind::with_mode("alt-b", Mode::FilterEdit, "text_caret_word_back"),
        Bind::with_mode("alt-f", Mode::FilterEdit, "text_caret_word_forward"),
        Bind::with_mode("ctrl-a", Mode::FilterEdit, "text_caret_start"),
        Bind::with_mode("ctrl-e", Mode::FilterEdit, "text_caret_end"),
        Bind::with_mode("h", Mode::Calendar, "calendar_prev_day"),
        Bind::with_mode("l", Mode::Calendar, "calendar_next_day"),
        Bind::with_mode("k", Mode::Calendar, "calendar_prev_month"),
        Bind::with_mode("j", Mode::Calendar, "calendar_next_month"),
        Bind::with_mode("t", Mode::Calendar, "calendar_today"),
        Bind::with_mode("d", Mode::Calendar, "calendar_clear"),
        Bind::with_mode("enter", Mode::Calendar, "calendar_commit"),
        Bind::with_mode("esc", Mode::Calendar, "calendar_close"),
        // Readline's motion keys, so editing the date text feels like editing
        // text: ctrl-b/ctrl-f step a character, ctrl-a/ctrl-e go to the ends.
        Bind::with_mode("left", Mode::Calendar, "filter_caret_left"),
        Bind::with_mode("right", Mode::Calendar, "filter_caret_right"),
        Bind::with_mode("ctrl-b", Mode::Calendar, "filter_caret_left"),
        Bind::with_mode("ctrl-f", Mode::Calendar, "filter_caret_right"),
        Bind::with_mode("ctrl-a", Mode::Calendar, "calendar_jump_to_start"),
        Bind::with_mode("ctrl-e", Mode::Calendar, "calendar_jump_to_end"),
        // `?` is never part of a date, so it stays a help key here rather than
        // typing. Without it the calendar's own help would be unreachable,
        // because calendar mode does not fall back to global bindings.
        Bind::with_mode("?", Mode::Calendar, "toggle_help_details"),
        Bind::with_mode("[", Mode::Task, "move_section_up"),
        Bind::with_mode("]", Mode::Task, "move_section_down"),
        Bind::with_mode("{", Mode::Task, "move_project_up"),
        Bind::with_mode("}", Mode::Task, "move_project_down"),
        Bind::with_mode("o", Mode::Project, "toggle_only_selected"),
        Bind::with_mode("ctrl-z", Mode::Project, "search_fuzzy"),
        Bind::with_mode("ctrl-s", Mode::Project, "search_substring"),
        Bind::with_mode("ctrl-r", Mode::Project, "search_regex"),
        Bind::with_mode("c", Mode::Task, "toggle_completed_filter"),
        Bind::with_mode("z", Mode::Task, "toggle_subtask_visibility"),
        Bind::with_mode(",", Mode::Task, "toggle_project_grouping"),
        Bind::with_mode(".", Mode::Task, "toggle_section_grouping"),
        Bind::with_mode("s", Mode::Task, "cycle_task_sort"),
        Bind::with_mode("^", Mode::Task, "toggle_task_sort_direction"),
        Bind::with_mode("enter", Mode::Task, "open"),
        Bind::with_mode("space", Mode::Task, "toggle_task_selection"),
        Bind::with_mode("a", Mode::Task, "select_all_visible_tasks"),
        Bind::with_mode("i", Mode::Task, "invert_task_selection"),
        Bind::with_mode("x", Mode::Task, "clear_task_selection"),
        Bind::with_mode("ctrl-x", Mode::Task, "clear_hidden_task_selection"),
        Bind::with_mode("y", Mode::Task, "copy_tasks_to_clipboard"),
        Bind::with_mode("g", Mode::Task, "set_gantt_mode"),
        // The column cursor and the cell editor. `h`, `l`, `e`, and `d` are
        // all free in task mode, and `Mode::Any` binds none of them.
        Bind::with_mode("h", Mode::Task, "task_column_prev"),
        Bind::with_mode("l", Mode::Task, "task_column_next"),
        Bind::with_mode("e", Mode::Task, "begin_task_edit"),
        Bind::with_mode("d", Mode::Task, "toggle_task_completed"),
        Bind::with_mode("enter", Mode::TaskEdit, "commit_task_edit"),
        Bind::with_mode("esc", Mode::TaskEdit, "cancel_task_edit"),
        // Letters type in this mode, so the value picker's keys are only
        // reachable because a picker reads no text at all.
        Bind::with_mode("j", Mode::TaskEdit, "task_edit_next_value"),
        Bind::with_mode("k", Mode::TaskEdit, "task_edit_prev_value"),
        Bind::with_mode("d", Mode::TaskEdit, "task_edit_clear"),
        Bind::with_mode("ctrl-l", Mode::TaskEdit, "task_edit_clear"),
        Bind::with_mode("left", Mode::TaskEdit, "filter_caret_left"),
        Bind::with_mode("right", Mode::TaskEdit, "filter_caret_right"),
        Bind::with_mode("ctrl-b", Mode::TaskEdit, "filter_caret_left"),
        Bind::with_mode("ctrl-f", Mode::TaskEdit, "filter_caret_right"),
        Bind::with_mode("alt-b", Mode::TaskEdit, "text_caret_word_back"),
        Bind::with_mode("alt-f", Mode::TaskEdit, "text_caret_word_forward"),
        Bind::with_mode("ctrl-a", Mode::TaskEdit, "text_caret_start"),
        Bind::with_mode("ctrl-e", Mode::TaskEdit, "text_caret_end"),
        // Punctuation and ctrl- pairs throughout, because
        // KeyBinding::from_crossterm_event lowercases every char: `G` and `g`
        // are the same key, so shift+letter is not an available namespace.
        Bind::with_mode("esc", Mode::Gantt, "set_task_mode"),
        Bind::with_mode("g", Mode::Gantt, "toggle_gantt"),
        Bind::with_mode("<", Mode::Gantt, "gantt_remove_column"),
        Bind::with_mode(">", Mode::Gantt, "gantt_add_column"),
        Bind::with_mode("c", Mode::Gantt, "cycle_gantt_color_key"),
        // h/l are bound in project and filter-edit mode, not globally, so
        // taking them here costs nothing. left/right stay column scrolling:
        // two horizontal scrolls on one pane is confusing enough without the
        // arrow keys changing meaning as well.
        Bind::with_mode("h", Mode::Gantt, "gantt_scroll_left"),
        Bind::with_mode("l", Mode::Gantt, "gantt_scroll_right"),
        Bind::with_mode("-", Mode::Gantt, "gantt_zoom_out"),
        Bind::with_mode("=", Mode::Gantt, "gantt_zoom_in"),
        Bind::with_mode("+", Mode::Gantt, "gantt_zoom_in"),
        Bind::with_mode("z", Mode::Gantt, "gantt_zoom_fit"),
        // `t` is today here rather than set_task_mode, which is what esc is
        // for. Mode::Calendar already binds `t` to today, so the picker and
        // the chart agree.
        Bind::with_mode("t", Mode::Gantt, "gantt_today"),
        Bind::with_mode("enter", Mode::Gantt, "gantt_open_order"),
        // j/k are bound here so they shadow the global cursor movement; the
        // dialog still falls back to the globals for ?, r, and q.
        Bind::with_mode("j", Mode::GanttOrder, "move_down"),
        Bind::with_mode("k", Mode::GanttOrder, "move_up"),
        Bind::with_mode("ctrl-j", Mode::GanttOrder, "gantt_order_move_down"),
        Bind::with_mode("ctrl-k", Mode::GanttOrder, "gantt_order_move_up"),
        Bind::with_mode("t", Mode::GanttOrder, "gantt_order_move_top"),
        Bind::with_mode("b", Mode::GanttOrder, "gantt_order_move_bottom"),
        Bind::with_mode("c", Mode::GanttOrder, "cycle_gantt_color_key"),
        Bind::with_mode("enter", Mode::GanttOrder, "gantt_order_commit"),
        Bind::with_mode("esc", Mode::GanttOrder, "gantt_order_cancel"),
    ]
}

#[cfg(test)]
mod tests {
    use crate::input::{Action, KeyBinding, KeyMap};

    use super::{Config, Mode, NamedFilterSet, SavedFilterField, SavedFilterSet};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_toml_and_keeps_defaults() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [[bind]]
                key = "x"
                command = "quit"

                [[bind]]
                key = "j"
                command = "move_down"
            "#,
        )
        .expect("config parses");

        assert_eq!(config.header.kind, "tuisana");
        assert_eq!(config.header.version, 1.0);
        assert_eq!(config.bind.len(), 2);
        assert_eq!(config.bind[0].key, "x");
        assert_eq!(config.bind[0].command, "quit");
    }

    #[test]
    fn rejects_wrong_version() {
        let err = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 2.0
            "#,
        )
        .expect_err("version should be rejected");

        assert!(format!("{err}").contains("expected header.version = 1"));
    }

    #[test]
    fn rejects_empty_type() {
        let err = Config::from_toml_str(
            r#"
                [header]
                type = ""
                version = 1.0
            "#,
        )
        .expect_err("empty type should be rejected");

        assert!(format!("{err}").contains("header.type must not be empty"));
    }

    #[test]
    fn parses_asana_config() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [auth]
                personal_access_token = "pat_123"
                workspace_gid = "42"

                [[project]]
                gid = "123"
                starred = true
                hidden = false
            "#,
        )
        .expect("auth config parses");

        let auth = config.auth.expect("auth section present");
        assert_eq!(auth.personal_access_token, "pat_123");
        assert_eq!(auth.workspace_gid.as_deref(), Some("42"));
        assert_eq!(config.project_visibility.len(), 1);
        assert_eq!(config.project_visibility[0].gid, "123");
        assert!(config.project_visibility[0].starred);
        assert!(!config.project_visibility[0].hidden);
    }

    #[test]
    fn rejects_empty_asana_token() {
        let err = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [auth]
                personal_access_token = ""
            "#,
        )
        .expect_err("empty token should be rejected");

        assert!(format!("{err}").contains("auth.personal_access_token must not be empty"));
    }

    #[test]
    fn loads_default_when_path_is_missing() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time ok")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tuisana-missing-{unique}.toml"));

        let config = Config::load_from_path(&path).expect("missing path should fall back");

        assert_eq!(config, Config::default());
        assert_eq!(config.source_path(), Some(path.as_path()));
    }

    #[test]
    fn rejects_duplicate_project_visibility_entries() {
        let err = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [[project]]
                gid = "123"
                starred = true

                [[project]]
                gid = "123"
                hidden = true
            "#,
        )
        .expect_err("duplicate project ids should be rejected");

        assert!(format!("{err}").contains("duplicate project.gid value"));
    }

    #[test]
    fn parses_the_documented_named_filter_set_block() {
        // The literal shape from the milestone, so the documented spelling
        // and the serialized one are both pinned.
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0

[[filter_set]]
name = "Sprint triage"

  [[filter_set.set]]

    [[filter_set.set.field]]
    key = "assignee"
    query = "alex"
    match = "fuzzy"

    [[filter_set.set.field]]
    key = "due"
    query = "..today"

  [[filter_set.set]]
  negated = true

    [[filter_set.set.field]]
    key = "custom:Priority"
    query = "Low"
"#,
        )
        .expect("the documented block parses");

        assert_eq!(config.filter_sets.len(), 1);
        let entry = &config.filter_sets[0];
        assert_eq!(entry.name, "Sprint triage");
        assert_eq!(entry.sets.len(), 2);
        assert!(!entry.sets[0].negated);
        assert_eq!(entry.sets[0].fields.len(), 2);
        assert_eq!(entry.sets[0].fields[0].key, "assignee");
        assert_eq!(entry.sets[0].fields[0].query, "alex");
        assert_eq!(entry.sets[0].fields[0].string_mode.as_deref(), Some("fuzzy"));
        assert!(!entry.sets[0].fields[0].empty);
        assert!(entry.sets[1].negated);
        assert_eq!(entry.sets[1].fields[0].key, "custom:Priority");
    }

    fn config_with_two_named_sets() -> Config {
        let filter_sets = vec![
            NamedFilterSet {
                name: "Blocked".to_string(),
                sets: vec![SavedFilterSet {
                    negated: false,
                    fields: vec![SavedFilterField {
                        key: "custom:Priority".to_string(),
                        query: "High".to_string(),
                        ..SavedFilterField::default()
                    }],
                }],
            },
            NamedFilterSet {
                name: "Overdue mine".to_string(),
                sets: vec![
                    SavedFilterSet {
                        negated: false,
                        fields: vec![
                            SavedFilterField {
                                key: "assignee".to_string(),
                                query: "alex".to_string(),
                                string_mode: Some("contains".to_string()),
                                ..SavedFilterField::default()
                            },
                            SavedFilterField {
                                key: "due".to_string(),
                                query: "..today".to_string(),
                                negated: true,
                                ..SavedFilterField::default()
                            },
                        ],
                    },
                    SavedFilterSet {
                        negated: true,
                        fields: vec![SavedFilterField {
                            key: "start".to_string(),
                            empty: true,
                            ..SavedFilterField::default()
                        }],
                    },
                ],
            },
        ];

        Config {
            filter_sets,
            ..Config::default()
        }
    }

    #[test]
    fn a_config_holding_named_filter_sets_round_trips_through_the_serializer() {
        // The `toml` serializer cannot emit a value after a table, so a struct
        // with its `Vec` before its scalars serializes to something it cannot
        // read back — and only at `save_to_source_path`, on a real config.
        let config = config_with_two_named_sets();

        let text = toml::to_string_pretty(&config).expect("serializes");

        assert_eq!(Config::from_toml_str(&text).expect("parses"), config);
    }

    #[test]
    fn a_named_filter_set_leaves_out_everything_at_its_default() {
        let config = Config {
            filter_sets: vec![NamedFilterSet {
                name: "Mine".to_string(),
                sets: vec![SavedFilterSet {
                    negated: false,
                    fields: vec![SavedFilterField {
                        key: "assignee".to_string(),
                        query: "alex".to_string(),
                        ..SavedFilterField::default()
                    }],
                }],
            }],
            ..Config::default()
        };

        let text = toml::to_string_pretty(&config).expect("serializes");
        // Only the entry itself: the default bindings above it spell out
        // commands like `filter_require_empty`.
        let entry = text
            .split("[[filter_set]]")
            .nth(1)
            .expect("the entry was written");

        assert!(entry.contains("name = \"Mine\""), "{entry}");
        assert!(!entry.contains("negated"), "an off flag is not written: {entry}");
        assert!(!entry.contains("empty"), "an off flag is not written: {entry}");
        assert!(!entry.contains("match"), "a default mode is not written: {entry}");
    }

    #[test]
    fn an_untouched_config_writes_no_filter_set_section() {
        let text = toml::to_string_pretty(&Config::default()).expect("serializes");

        assert!(!text.contains("[[filter_set]]"), "{text}");
    }

    #[test]
    fn rejects_two_named_filter_sets_whose_names_differ_only_in_case() {
        // The sidebar lists them by number, and two rows reading the same is a
        // trap: there is no way to tell which one a digit loads.
        let error = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0

[[filter_set]]
name = "Mine"

[[filter_set]]
name = "mine"
"#,
        )
        .expect_err("duplicate names are rejected");

        assert!(
            error.to_string().contains("duplicate filter_set.name"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_an_empty_named_filter_set_name() {
        let error = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0

[[filter_set]]
name = "   "
"#,
        )
        .expect_err("a blank name is rejected");

        assert!(
            error.to_string().contains("filter_set.name must not be empty"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_a_saved_match_mode_that_names_no_mode() {
        let error = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0

[[filter_set]]
name = "Mine"

  [[filter_set.set]]

    [[filter_set.set.field]]
    key = "assignee"
    match = "glob"
"#,
        )
        .expect_err("an unknown match mode is rejected");

        assert!(
            error.to_string().contains("field.match"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_unknown_field_key_is_kept_rather_than_rejected() {
        // A `custom:` field belonging to a project that is not loaded this
        // session is legitimate; rejecting it would make the config
        // unloadable depending on which projects you had selected.
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0

[[filter_set]]
name = "Mine"

  [[filter_set.set]]

    [[filter_set.set.field]]
    key = "custom:Nobody Has This"
    query = "Low"
"#,
        )
        .expect("an unknown key is kept");

        assert_eq!(
            config.filter_sets[0].sets[0].fields[0].key,
            "custom:Nobody Has This"
        );
    }

    #[test]
    fn the_sidebar_order_is_by_name_rather_than_by_file_order() {
        let config = Config {
            filter_sets: vec![
                NamedFilterSet { name: "zebra".to_string(), sets: Vec::new() },
                NamedFilterSet { name: "Apple".to_string(), sets: Vec::new() },
                NamedFilterSet { name: "mango".to_string(), sets: Vec::new() },
            ],
            ..Config::default()
        };

        assert_eq!(
            config
                .sorted_filter_sets()
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["Apple", "mango", "zebra"]
        );
    }

    #[test]
    fn default_bindings_include_global_navigation() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        for mode in [Mode::Project, Mode::Filter] {
            assert_eq!(
                keymap.action_for(&KeyBinding::Char('k'), mode),
                Some(&Action::MoveUp)
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Char('j'), mode),
                Some(&Action::MoveDown)
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Ctrl('u'), mode),
                Some(&Action::PageUp)
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Ctrl('d'), mode),
                Some(&Action::PageDown)
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Left, mode),
                Some(&Action::ScrollLeft)
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Right, mode),
                Some(&Action::ScrollRight)
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Char('['), mode),
                Some(&Action::ResizeTopPaneDown)
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Char(']'), mode),
                Some(&Action::ResizeTopPaneUp)
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Char('{'), mode),
                Some(&Action::MinimizeTopPane)
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Char('}'), mode),
                Some(&Action::MaximizeTopPane)
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Char('0'), mode),
                Some(&Action::RestoreTopPane)
            );
        }

        assert_eq!(
            keymap.action_for(&KeyBinding::Char('['), Mode::Task),
            Some(&Action::MoveSectionUp)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char(']'), Mode::Task),
            Some(&Action::MoveSectionDown)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('{'), Mode::Task),
            Some(&Action::MoveProjectUp)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('}'), Mode::Task),
            Some(&Action::MoveProjectDown)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('0'), Mode::Task),
            Some(&Action::RestoreTopPane)
        );
    }

    #[test]
    fn default_bindings_include_project_controls() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        assert_eq!(
            keymap.action_for(&KeyBinding::Char('/'), Mode::Project),
            Some(&Action::StartSearch)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('/'), Mode::Filter),
            None
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('['), Mode::Project),
            Some(&Action::ResizeTopPaneDown)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char(']'), Mode::Project),
            Some(&Action::ResizeTopPaneUp)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char(' '), Mode::Project),
            Some(&Action::ToggleSelection)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('!'), Mode::Project),
            Some(&Action::SelectAllStarredVisible)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('@'), Mode::Project),
            Some(&Action::SelectAllNonHiddenVisible)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('f'), Mode::Project),
            Some(&Action::SetFilterMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('t'), Mode::Project),
            Some(&Action::SetTaskMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('m'), Mode::Project),
            Some(&Action::ToggleTaskMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('p'), Mode::Project),
            Some(&Action::SetProjectMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Enter, Mode::Project),
            Some(&Action::Open)
        );
    }

    /// `ctrl-b`/`ctrl-f` have to mean the same thing in every mode that edits
    /// text, which is why `search_fuzzy` gave up `ctrl-f`.
    #[test]
    fn the_caret_motion_keys_are_the_same_in_every_text_editing_mode() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        for mode in [Mode::FilterEdit, Mode::Calendar] {
            assert_eq!(
                keymap.action_for(&KeyBinding::Ctrl('b'), mode),
                Some(&Action::FilterCaretLeft),
                "ctrl-b moves the caret left in {mode:?}"
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Ctrl('f'), mode),
                Some(&Action::FilterCaretRight),
                "ctrl-f moves the caret right in {mode:?}"
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Left, mode),
                Some(&Action::FilterCaretLeft)
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::Right, mode),
                Some(&Action::FilterCaretRight)
            );
        }

        // Fuzzy matching keeps a key everywhere it had one, just a different one.
        for mode in [Mode::Filter, Mode::FilterEdit, Mode::Project] {
            assert_eq!(
                keymap.action_for(&KeyBinding::Ctrl('z'), mode),
                Some(&Action::SearchFuzzy),
                "ctrl-z switches to fuzzy in {mode:?}"
            );
        }
    }

    #[test]
    fn default_bindings_include_filter_controls() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        assert_eq!(
            keymap.action_for(&KeyBinding::Enter, Mode::Filter),
            Some(&Action::BeginFilterEdit)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Esc, Mode::Filter),
            Some(&Action::SetTaskMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('f'), Mode::Filter),
            Some(&Action::ToggleTaskFilters)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('s'), Mode::Filter),
            Some(&Action::CycleFilterStringMode)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('z'), Mode::Filter),
            Some(&Action::SearchFuzzy),
            "fuzzy moved off ctrl-f so the motion keys could have it"
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('s'), Mode::Filter),
            Some(&Action::SearchSubstring)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('r'), Mode::Filter),
            Some(&Action::SearchRegex)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Enter, Mode::FilterEdit),
            Some(&Action::FilterDoneEditing)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Esc, Mode::FilterEdit),
            Some(&Action::FilterCancelEditing)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('z'), Mode::FilterEdit),
            Some(&Action::SearchFuzzy)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('s'), Mode::FilterEdit),
            Some(&Action::SearchSubstring)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('r'), Mode::FilterEdit),
            Some(&Action::SearchRegex)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Enter, Mode::Project),
            Some(&Action::Open)
        );

        for (key, action) in [
            (KeyBinding::Char('l'), Action::FilterSetNext),
            (KeyBinding::Char('h'), Action::FilterSetPrev),
            (KeyBinding::Char('a'), Action::FilterSetAdd),
            (KeyBinding::Char('x'), Action::FilterSetRemove),
            (KeyBinding::Char('e'), Action::FilterRequireEmpty),
        ] {
            assert_eq!(keymap.action_for(&key, Mode::Filter), Some(&action));
        }
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('q'), Mode::FilterEdit),
            Some(&Action::FilterRequireEmpty),
            "letters type while editing, so require-empty needs a ctrl- pair"
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('e'), Mode::FilterEdit),
            Some(&Action::TextCaretEnd),
            "ctrl-e is end-of-line wherever text is edited, so require-empty moved"
        );
        // h/l keep their existing meanings in the modes that shadow them: you
        // switch sets from browse mode, not mid-edit.
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('h'), Mode::FilterEdit),
            Some(&Action::FilterMoveLabelLeft)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('h'), Mode::Calendar),
            Some(&Action::CalendarPrevDay)
        );
    }

    #[test]
    fn default_bindings_include_the_named_filter_set_controls() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        for (key, action) in [
            (KeyBinding::Char('b'), Action::FilterSetsToggle),
            (KeyBinding::Char('w'), Action::FilterSetSave),
            (KeyBinding::Char('y'), Action::FilterSetCopyToNew),
            (KeyBinding::Char('n'), Action::FilterSetNew),
            (KeyBinding::Char('d'), Action::FilterSetDelete),
            (KeyBinding::Char('<'), Action::FilterSetsPageBack),
            (KeyBinding::Char('>'), Action::FilterSetsPageForward),
        ] {
            assert_eq!(
                keymap.action_for(&key, Mode::Filter),
                Some(&action),
                "{key:?} should be bound in filter mode"
            );
        }

        // All nine digits, so every row the sidebar can show is reachable.
        for position in 1..=9u8 {
            let key = KeyBinding::Char(char::from_digit(position as u32, 10).expect("digit"));
            assert_eq!(
                keymap.action_for(&key, Mode::Filter),
                Some(&Action::FilterSetLoad(position))
            );
        }

        // `0` is left alone: it restores the top pane, everywhere.
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('0'), Mode::Filter),
            Some(&Action::RestoreTopPane)
        );
        // The keys these took are unbound in filter mode only; the modes that
        // already used them keep them.
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('y'), Mode::Task),
            Some(&Action::CopyTasksToClipboard)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('d'), Mode::FilterEdit),
            Some(&Action::FilterDeleteLabel)
        );
        // Nothing resolves in the prompt: its keys are read outside the
        // keymap, so an unbound letter has to type rather than fire `b`.
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('b'), Mode::FilterSetName),
            None
        );
    }

    #[test]
    fn default_bindings_include_project_search_controls() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('l'), Mode::ProjectSearch),
            Some(&Action::ClearSearch)
        );
        // ctrl-l clears the active filter in both filter modes
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('l'), Mode::Filter),
            Some(&Action::ClearSearch)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('l'), Mode::FilterEdit),
            Some(&Action::ClearSearch)
        );
        // but not in project mode, where it would be unintentional
        assert_eq!(
            keymap.action_for(&KeyBinding::Ctrl('l'), Mode::Project),
            None
        );
    }

    #[test]
    fn default_bindings_include_task_controls() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        assert_eq!(
            keymap.action_for(&KeyBinding::Char('['), Mode::Task),
            Some(&Action::MoveSectionUp)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char(']'), Mode::Task),
            Some(&Action::MoveSectionDown)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('{'), Mode::Task),
            Some(&Action::MoveProjectUp)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('}'), Mode::Task),
            Some(&Action::MoveProjectDown)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('c'), Mode::Task),
            Some(&Action::ToggleCompletedFilter)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('z'), Mode::Task),
            Some(&Action::ToggleSubtaskVisibility)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('s'), Mode::Task),
            Some(&Action::CycleTaskSort)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('^'), Mode::Task),
            Some(&Action::ToggleTaskSortDirection)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char(','), Mode::Task),
            Some(&Action::ToggleProjectGrouping)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('.'), Mode::Task),
            Some(&Action::ToggleSectionGrouping)
        );
        for (key, action) in [
            (KeyBinding::Char('h'), Action::TaskColumnPrev),
            (KeyBinding::Char('l'), Action::TaskColumnNext),
            (KeyBinding::Char('e'), Action::BeginTaskEdit),
            (KeyBinding::Char('d'), Action::ToggleTaskCompleted),
        ] {
            assert_eq!(keymap.action_for(&key, Mode::Task), Some(&action));
        }
    }

    #[test]
    fn default_bindings_include_task_edit_controls() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        for (key, action) in [
            (KeyBinding::Enter, Action::CommitTaskEdit),
            (KeyBinding::Esc, Action::CancelTaskEdit),
            (KeyBinding::Char('j'), Action::TaskEditCycleValue(1)),
            (KeyBinding::Char('k'), Action::TaskEditCycleValue(-1)),
            (KeyBinding::Char('d'), Action::TaskEditClear),
            (KeyBinding::Alt('b'), Action::TextCaretWordBack),
            (KeyBinding::Alt('f'), Action::TextCaretWordForward),
            (KeyBinding::Ctrl('a'), Action::TextCaretStart),
            (KeyBinding::Ctrl('e'), Action::TextCaretEnd),
        ] {
            assert_eq!(
                keymap.action_for(&key, Mode::TaskEdit),
                Some(&action),
                "{key:?} in task-edit mode"
            );
        }
    }

    /// An unbound letter has to type into the cell rather than fire the
    /// global binding that letter carries.
    #[test]
    fn task_edit_mode_does_not_fall_back_to_the_global_bindings() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        assert!(!Mode::TaskEdit.allows_any_fallback());
        assert_eq!(keymap.action_for(&KeyBinding::Char('q'), Mode::TaskEdit), None);
    }
}

#[cfg(test)]
mod theme_config_tests {
    use super::{Config, GanttConfig, ThemeConfig, ThemeGlyphs, ThemeVariant};
    use crate::domain::GanttColorKey;

    #[test]
    fn parses_the_gantt_section() {
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0

[gantt]
visible = true
columns = 4
color_by = "field:Priority"

[gantt.order]
assignee = ["Alex Chen", "Priya Raman"]
"field:Priority" = ["High", "Low"]
"#,
        )
        .expect("gantt config parses");

        assert!(config.gantt.visible);
        assert_eq!(config.gantt.columns, 4);
        assert_eq!(
            config.gantt.color_key(),
            GanttColorKey::Field("Priority".to_string())
        );
        assert_eq!(
            config.gantt.order_for(&GanttColorKey::Field("Priority".to_string())),
            ["High".to_string(), "Low".to_string()]
        );
        assert_eq!(
            config.gantt.order_for(&GanttColorKey::Assignee),
            ["Alex Chen".to_string(), "Priya Raman".to_string()]
        );
        assert!(
            config.gantt.order_for(&GanttColorKey::Section).is_empty(),
            "an unmentioned dimension has no order"
        );
    }

    #[test]
    fn an_omitted_gantt_section_falls_back_to_the_defaults() {
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0
"#,
        )
        .expect("config parses");

        assert_eq!(config.gantt, GanttConfig::default());
        assert!(config.gantt.is_default());
        assert!(!config.gantt.visible, "the chart is off until asked for");
    }

    #[test]
    fn gantt_defaults_are_omitted_when_the_config_is_serialized() {
        // The app rewrites the whole file when a project is starred, so an
        // untouched section must not start appearing in the user's config.
        let serialized = toml::to_string_pretty(&Config::default()).expect("serializes");

        assert!(!serialized.contains("[gantt]"), "{serialized}");
    }

    #[test]
    fn a_customized_gantt_section_survives_a_serialize_round_trip() {
        let mut config = Config::default();
        config.gantt.color_by = "section".to_string();
        config.gantt.order.insert(
            "section".to_string(),
            vec!["Shipment".to_string(), "Study Kit Design".to_string()],
        );

        let serialized = toml::to_string_pretty(&config).expect("serializes");
        let parsed = Config::from_toml_str(&serialized).expect("reparses");

        assert_eq!(parsed.gantt, config.gantt);
    }

    #[test]
    fn rejects_a_gantt_color_by_that_names_no_dimension() {
        let error = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0

[gantt]
color_by = "priority"
"#,
        )
        .expect_err("an unknown dimension is rejected");

        assert!(
            error.to_string().contains("gantt.color_by"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_a_mistyped_gantt_order_key() {
        // Left unchecked this sits in the file looking effective while
        // ordering nothing.
        let error = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0

[gantt.order]
assigne = ["Alex Chen"]
"#,
        )
        .expect_err("a mistyped key is rejected");

        assert!(
            error.to_string().contains("gantt.order"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_a_gantt_column_count_of_zero() {
        let error = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0

[gantt]
columns = 0
"#,
        )
        .expect_err("zero columns is rejected");

        assert!(
            error.to_string().contains("gantt.columns"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn theme_defaults_are_omitted_when_the_config_is_serialized() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).expect("serializes");

        assert!(
            !serialized.contains("[theme]"),
            "an untouched theme should not be written back into the user's config"
        );
    }

    #[test]
    fn a_customized_theme_round_trips_through_toml() {
        let parsed = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [theme]
                variant = "mono"
                glyphs = "ascii"
                accent = "magenta"
                zebra = true
            "#,
        )
        .expect("config parses");

        assert_eq!(parsed.theme.variant, ThemeVariant::Mono);
        assert_eq!(parsed.theme.glyphs, ThemeGlyphs::Ascii);
        assert_eq!(parsed.theme.accent, "magenta");
        assert!(parsed.theme.zebra);

        let serialized = toml::to_string_pretty(&parsed).expect("serializes");
        assert_eq!(
            Config::from_toml_str(&serialized).expect("round trips").theme,
            parsed.theme
        );
    }

    #[test]
    fn an_unknown_accent_is_rejected_rather_than_silently_ignored() {
        let error = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [theme]
                accent = "chartreuse"
            "#,
        )
        .expect_err("an unknown accent should fail validation");

        assert!(format!("{error}").contains("theme.accent"));
    }

    #[test]
    fn an_omitted_theme_section_falls_back_to_the_defaults() {
        let parsed = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0
            "#,
        )
        .expect("config parses");

        assert_eq!(parsed.theme, ThemeConfig::default());
        assert!(parsed.theme.is_default());
    }
}
