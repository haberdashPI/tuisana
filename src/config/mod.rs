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
    #[serde(default = "unversioned_header")]
    pub header: Header,
    #[serde(default)]
    #[serde(skip_serializing_if = "ThemeConfig::is_default")]
    pub theme: ThemeConfig,
    #[serde(default)]
    #[serde(skip_serializing_if = "GanttConfig::is_default")]
    pub gantt: GanttConfig,
    #[serde(default)]
    #[serde(skip_serializing_if = "ViewConfig::is_default")]
    pub view: ViewConfig,
    #[serde(default)]
    #[serde(skip_serializing_if = "EditConfig::is_default")]
    pub edit: EditConfig,
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
    /// Set when [`migrate`] rewrote this config on the way in.
    ///
    /// The app asks before it writes the migrated file back, so it has to
    /// know that the file on disk is not what it is now holding.
    #[serde(skip)]
    migrated: bool,
    /// The version the file on disk declared, kept for the prompt's wording.
    #[serde(skip)]
    migrated_from: Option<f64>,
}

impl PartialEq for Config {
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header
            && self.theme == other.theme
            && self.gantt == other.gantt
            && self.view == other.view
            && self.edit == other.edit
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
            view: ViewConfig::default(),
            edit: EditConfig::default(),
            auth: None,
            bind: default_bindings(),
            project_visibility: Vec::new(),
            filter_sets: Vec::new(),
            source_path: None,
            migrated: false,
            migrated_from: None,
        }
    }
}

impl Config {
    /// Parse config from an in-memory TOML string.
    ///
    /// A version-1 file is migrated in memory, between parsing and
    /// validation: §2.1 of milestone 15 renamed six commands and one mode,
    /// and an unknown command is a hard startup error, so without the rename
    /// a file that was valid yesterday would refuse to load at all.
    pub fn from_toml_str(input: &str) -> Result<Self> {
        let config: Self = toml::from_str(input)?;
        // Before migration, because `[view].projects` only exists until
        // migration moves it: a bad gid in it has to be reported against the
        // key the file actually contains, not against the `[[filter_set]]` it
        // is about to become.
        config.view.validate()?;
        let mut config = migrate(config);
        // Before validation, because collapsing a duplicate is the parse
        // deciding what the file meant rather than a rule it has to pass.
        for entry in &mut config.filter_sets {
            entry.normalize();
        }
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

    /// Whether this config was read in the version-1 format and rewritten.
    ///
    /// True only for a file that exists on disk in the old format: a missing
    /// config, or one already at version 2, has nothing to migrate.
    pub fn needs_migration(&self) -> bool {
        self.migrated && self.source_path.is_some()
    }

    /// Clears the migration flag, once the file on disk has caught up — or
    /// once the user has been asked and said no.
    pub fn clear_migration(&mut self) {
        self.migrated = false;
    }

    /// The version the file on disk was written in, for the prompt to name.
    ///
    /// Recorded at parse time because [`migrate`] rewrites `header.version`
    /// in memory, so by the time the prompt is raised the config no longer
    /// remembers where it came from.
    pub fn migrated_from_version(&self) -> f64 {
        self.migrated_from.unwrap_or(Header::CURRENT_VERSION)
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
        let mut entries = self
            .filter_sets
            .iter()
            .filter(|entry| !entry.scratch)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.name.to_lowercase());
        entries
    }

    /// How many entries the sidebar numbers, which is what decides whether
    /// there is a second page.
    pub fn named_filter_set_count(&self) -> usize {
        self.filter_sets.iter().filter(|entry| !entry.scratch).count()
    }

    /// A saved entry by name. Never the scratch entry: that one is the app's
    /// own slot and its name is cosmetic.
    pub fn named_filter_set(&self, name: &str) -> Option<&NamedFilterSet> {
        self.filter_sets
            .iter()
            .find(|entry| !entry.scratch && entry.name.eq_ignore_ascii_case(name))
    }

    /// The selection an unnamed panel left behind, if a session ever did.
    pub fn scratch_projects(&self) -> &[String] {
        self.filter_sets
            .iter()
            .find(|entry| entry.scratch)
            .map(NamedFilterSet::projects)
            .unwrap_or(&[])
    }

    /// Records the selection of an unnamed panel, creating the slot if this
    /// is the first session to need one.
    pub fn set_scratch_projects(&mut self, projects: Vec<String>) {
        match self.filter_sets.iter_mut().find(|entry| entry.scratch) {
            Some(entry) => entry.projects = Some(projects),
            None => self.filter_sets.push(NamedFilterSet::scratch(projects)),
        }
    }

    fn validate(&self) -> Result<()> {
        // A range rather than one number: version 1 is still *readable*, it
        // is just no longer writable, and `migrate_v1` is what closes the
        // gap. A file from the future is still rejected — the app cannot
        // guess what a key it has never heard of means.
        let version = self.header.version();
        if !(Header::OLDEST_VERSION..=Header::CURRENT_VERSION).contains(&version) {
            return Err(Error::ConfigValidation(format!(
                "expected header.version = {}, found {version}",
                Header::CURRENT_VERSION,
            )));
        }

        if self.header.kind.trim().is_empty() {
            return Err(Error::ConfigValidation(
                "header.type must not be empty".to_string(),
            ));
        }

        self.theme.validate()?;
        self.gantt.validate()?;
        self.view.validate()?;

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
        let mut scratch_entries = 0;
        for entry in &self.filter_sets {
            entry.validate()?;
            if entry.scratch {
                // Nothing lists it and nothing looks it up by name, so its
                // name is not held to the uniqueness rule — but two slots for
                // one unnamed panel would leave the app guessing which it
                // just wrote to.
                scratch_entries += 1;
                if scratch_entries > 1 {
                    return Err(Error::ConfigValidation(
                        "only one filter_set may set scratch = true".to_string(),
                    ));
                }
                continue;
            }
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
///
/// `list` is only meaningful on a row whose values come from a directory;
/// one saved against any other row parks unapplied rather than being
/// rejected here, exactly as an unknown field key does.
const SAVED_MATCH_MODES: [&str; 4] = ["fuzzy", "contains", "regex", "list"];

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
    /// Whether this is the app's own slot for an **unnamed** panel's project
    /// selection, rather than an entry the user saved.
    ///
    /// Since version 3 a project selection is only ever recorded on a
    /// `[[filter_set]]`; there is no second home for it. An unnamed panel
    /// still has one, so it gets a reserved entry — hidden from the sidebar,
    /// unaddressable by a digit, and never found by name. It holds the
    /// selection and nothing else: the panel is genuinely unnamed, its
    /// filters still exist nowhere but on screen, and the discard
    /// confirmation still means what it says.
    ///
    /// At most one entry may carry it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub scratch: bool,
    /// The projects the set is asked of, by gid, with `me` standing for the
    /// assigned-to-me row.
    ///
    /// One list per entry rather than one per tab: tabs OR, and a fetch scope
    /// cannot — there is one set of projects being read, whatever the tabs
    /// then ask of it.
    ///
    /// Read through [`Self::projects`], which reads a missing key as the
    /// empty selection. The `Option` survives only so that **migration** can
    /// tell a version-2 entry that never had the key from one that was
    /// saved selecting nothing: the first adopts `[view].projects`, the
    /// second is already right. At runtime the two are the same thing, and
    /// loading either leaves nothing selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<String>>,
    /// The ORed sets, in tab order.
    #[serde(default, rename = "set", skip_serializing_if = "Vec::is_empty")]
    pub sets: Vec<SavedFilterSet>,
}

impl NamedFilterSet {
    /// The name the app writes on its scratch entry.
    ///
    /// Cosmetic: the entry is found by its `scratch` flag and never by name,
    /// so a user's own entry called `unnamed` does not collide with it.
    pub const SCRATCH_NAME: &'static str = "unnamed";

    /// The projects this set is asked of, with a missing key read as none.
    pub fn projects(&self) -> &[String] {
        self.projects.as_deref().unwrap_or(&[])
    }

    /// The app's scratch entry, holding a selection and nothing else.
    pub fn scratch(projects: Vec<String>) -> Self {
        Self {
            name: Self::SCRATCH_NAME.to_string(),
            scratch: true,
            projects: Some(projects),
            sets: Vec::new(),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::ConfigValidation(
                "filter_set.name must not be empty".to_string(),
            ));
        }

        for project in self.projects.iter().flatten() {
            if project.trim().is_empty() {
                return Err(Error::ConfigValidation(
                    "filter_set.projects entries must not be empty".to_string(),
                ));
            }
        }

        for set in &self.sets {
            set.validate()?;
        }

        Ok(())
    }

    /// Collapses a hand-written `projects` list that names the same project
    /// twice.
    ///
    /// Collapsed rather than refused: a file that names a project twice meant
    /// it once, and nothing about the selection is ambiguous. Nothing checks
    /// that a gid *exists* — at parse time the workspace has not been asked
    /// yet.
    fn normalize(&mut self) {
        if let Some(projects) = self.projects.as_mut() {
            let mut seen = HashSet::new();
            projects.retain(|project| seen.insert(project.clone()));
        }
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

/// How much of a bulk edit goes through without being asked about.
///
/// One knob, because there is only one judgement call here: how many rows is
/// "a few". Everything else about the write path — how many actions go out at
/// once and how fast — is a property of Asana's limits rather than a taste,
/// and lives in [`crate::asana::throttle`] as constants.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct EditConfig {
    /// How many tasks an edit may change before it has to be confirmed.
    ///
    /// At or below this, the edit goes. Above it, a modal says how many rows
    /// are about to change and waits. `0` confirms every bulk edit; a number
    /// larger than any selection turns the confirmation off.
    #[serde(default = "default_confirm_threshold")]
    pub confirm_threshold: usize,
}

fn default_confirm_threshold() -> usize {
    5
}

impl Default for EditConfig {
    fn default() -> Self {
        Self {
            confirm_threshold: default_confirm_threshold(),
        }
    }
}

impl EditConfig {
    /// Returns `true` when nothing has been customized, which keeps the
    /// `[edit]` table out of configs the app rewrites.
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// Whether changing `count` tasks at once needs to be confirmed.
    ///
    /// Strictly greater: a threshold of 5 lets five rows through and asks
    /// about six, which is what "more than a few" means.
    pub fn needs_confirmation(&self, count: usize) -> bool {
        count > self.confirm_threshold
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

/// How tall the top pane is, in the only three steps worth remembering.
///
/// Deliberately not a row count: a pane sized to one terminal is noise in
/// another, and the thing the user actually chose was "give this pane the
/// screen" or "give it back".
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopPaneState {
    #[default]
    Normal,
    Minimized,
    Maximized,
}

impl TopPaneState {
    fn is_default(&self) -> bool {
        matches!(self, Self::Normal)
    }
}

/// The view as it stood when the app last wrote the config.
///
/// This is the *serialization* of what the panes and the project list are
/// showing, written after every settled keystroke that changes it and read
/// back once at startup. Like `[gantt]`, it is a section the app owns: hand
/// edits are honoured, but the next keypress that changes the view rewrites
/// them.
///
/// It records what is *open*, never how big it is. `top_pane` is the one
/// exception, and it is three named states rather than a height — see
/// [`TopPaneState`]. Scroll offsets, cursor positions, the sort, the
/// grouping, and the completed filter all stay out, for the same reason
/// [`GanttConfig`] leaves its timeline window out: they are where you were
/// looking, not what you had set up.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ViewConfig {
    /// The named entry the filter panel was bound to, if any.
    ///
    /// Only a *saved* panel is recorded. An unnamed one lives nowhere but on
    /// screen, and writing it here would quietly give it a second home that
    /// the sidebar never lists.
    ///
    /// A name that no longer matches an entry is ignored at startup rather
    /// than rejected: deleting a `[[filter_set]]` by hand must not make the
    /// config unloadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_set: Option<String>,
    #[serde(default, skip_serializing_if = "TopPaneState::is_default")]
    pub top_pane: TopPaneState,
    /// The task table shares the body with the top pane.
    #[serde(default, skip_serializing_if = "is_false")]
    pub tasks: bool,
    /// The top pane holds the filter panel rather than the project list.
    #[serde(default, skip_serializing_if = "is_false")]
    pub filters: bool,
    /// The `Sets` sidebar down the left of the filter panel.
    #[serde(default, skip_serializing_if = "is_false")]
    pub filter_sidebar: bool,
    /// The recently-edited pane, which is on unless it was toggled off.
    ///
    /// The toggle, not whether the pane is drawn: it holds what *this*
    /// session edited, so at startup it is always empty and always hidden.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub recent: bool,
    /// A version-1 or -2 file's selected projects, on their way out.
    ///
    /// Read, never written: since version 3 a project selection lives on a
    /// `[[filter_set]]` and nowhere else, so this exists only for
    /// [`migrate`] to move an old file's selection onto the entry that was
    /// bound — or onto the scratch entry, when nothing was. It is empty from
    /// the moment migration has run.
    ///
    /// Keeping the field rather than ignoring the key is what makes that move
    /// possible: a selection someone has been using every day should survive
    /// the upgrade, not be silently dropped.
    ///
    /// Last, because `toml` cannot emit a value after a table and this is the
    /// only field here that is not a scalar.
    #[serde(default, rename = "projects", skip_serializing)]
    pub legacy_projects: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

impl Default for ViewConfig {
    fn default() -> Self {
        Self {
            filter_set: None,
            top_pane: TopPaneState::default(),
            tasks: false,
            filters: false,
            filter_sidebar: false,
            recent: true,
            legacy_projects: Vec::new(),
        }
    }
}

impl ViewConfig {
    /// Returns `true` when nothing has been recorded.
    ///
    /// Keeps `[view]` out of a config the app rewrites before the user has
    /// opened anything, exactly as `[theme]` and `[gantt]` are kept out.
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    fn validate(&self) -> Result<()> {
        if let Some(name) = &self.filter_set {
            if name.trim().is_empty() {
                return Err(Error::ConfigValidation(
                    "view.filter_set must not be empty".to_string(),
                ));
            }
        }

        if self.legacy_projects.iter().any(|gid| gid.trim().is_empty()) {
            return Err(Error::ConfigValidation(
                "view.projects entries must not be empty".to_string(),
            ));
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
    /// The structural edits: the keys that change which rows exist.
    ///
    /// A sibling of [`Mode::Task`] the way [`Mode::Gantt`] is — the same pane,
    /// the same rows, the same cursor and selection, different keys. Task mode
    /// is about what the table *says*; this one is about what it *contains*.
    Edit,
    /// A bulk edit is waiting to be confirmed.
    ///
    /// Owns every key while it is up, so it allows no `Any` fallback: a
    /// modal that asks a question and then acts on `j` would be worse than
    /// no modal at all.
    Confirm,
    /// A task table cell is open for editing.
    ///
    /// `task_edit` is the version-1 spelling, kept as an alias so a config in
    /// the old format parses at all — the migration is what rewrites it, and
    /// it cannot run on a file that failed to deserialize.
    #[serde(alias = "task_edit")]
    ColumnEdit,
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
                | Self::Confirm
                // An unbound letter has to type into the cell rather than
                // fire the global binding that letter carries.
                | Self::ColumnEdit
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
            Self::Confirm => "confirm",
            Self::Task => "task",
            Self::Edit => "edit",
            Self::ColumnEdit => "column",
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
    /// The format version the file was written in.
    ///
    /// `None` means the file carries no `[header]` at all, which is read as
    /// **version 1** rather than as the current version. Defaulting it to
    /// what the app writes would be the wrong answer for a file the app has
    /// never written: it would skip the migration and then fail validation on
    /// the old command names it was supposed to rename.
    #[serde(default)]
    pub version: Option<f64>,
}

impl Header {
    /// The version the app writes, and the only one it will write.
    pub const CURRENT_VERSION: f64 = 3.0;
    /// The oldest version still readable.
    ///
    /// Every version in between is still *readable* and migrated in memory,
    /// which is why validation takes a range rather than one number.
    pub const OLDEST_VERSION: f64 = 1.0;
    /// The version that moved the project selection onto the filter sets.
    pub const PROJECTS_ON_SETS_VERSION: f64 = 3.0;

    /// The version this header declares, treating "absent" as the oldest.
    pub fn version(&self) -> f64 {
        self.version.unwrap_or(Self::OLDEST_VERSION)
    }
}

impl Default for Header {
    fn default() -> Self {
        Self {
            kind: default_header_type(),
            version: Some(Header::CURRENT_VERSION),
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

/// The §2.1 renames, as `(version 1, version 2)` pairs.
///
/// One list so the migration and its tests read from the same place. Mode
/// names and command names are kept apart because a config spells them in
/// different keys, and `task_edit` as a *command* has never existed.
const V1_COMMAND_RENAMES: [(&str, &str); 6] = [
    ("begin_task_edit", "edit_column"),
    ("commit_task_edit", "commit_column_edit"),
    ("cancel_task_edit", "cancel_column_edit"),
    ("task_edit_next_value", "column_edit_next_value"),
    ("task_edit_prev_value", "column_edit_prev_value"),
    ("task_edit_clear", "column_edit_clear"),
];

/// Brings an older config up to [`Header::CURRENT_VERSION`], in memory.
///
/// Each step is gated on the version that needed it, so a version-1 file gets
/// both and a version-2 file gets only the second. The whole thing is a no-op
/// on a file already at the current version.
///
/// **Version 1 → 2.** Two changes, and deliberately no more:
///
/// - every `[[bind]]` whose command appears in [`V1_COMMAND_RENAMES`] is
///   renamed. The mode rename is handled by serde, which accepts both
///   `task_edit` and `column_edit` for the same variant.
/// - every single-letter `key` is **lowercased**. Version 1's reader
///   lowercased it on the way in, so `key = "G"` meant `g`; milestone 15
///   makes that a different key, and a migration that left it alone would
///   silently rebind it.
///
/// Unknown commands are left exactly as they are and still fail validation:
/// this renames what was renamed, and is not a licence to accept typos.
///
/// **Version 2 → 3.** `[view].projects` moves onto a `[[filter_set]]`, which
/// is the only place a project selection lives from here on:
///
/// - onto the entry `[view].filter_set` named, if it named one that has no
///   list of its own. That entry *was* the panel on screen, and the
///   selection beside it was the selection it was being asked of.
/// - onto the scratch entry otherwise, because an unnamed panel has a
///   selection too and this is where it now lives.
///
/// An entry that already carries a list keeps it: a file written by a
/// version-2 build that had already learned about `projects` is right
/// already, and the empty list it may hold is a real answer rather than a
/// gap.
fn migrate(mut config: Config) -> Config {
    let version = config.header.version();
    if version >= Header::CURRENT_VERSION {
        return config;
    }
    config.migrated_from = Some(version);

    if version < 2.0 {
        for bind in &mut config.bind {
            if let Some((_, renamed)) = V1_COMMAND_RENAMES
                .iter()
                .find(|(old, _)| bind.command.trim().eq_ignore_ascii_case(old))
            {
                bind.command = (*renamed).to_string();
            }
            let key = bind.key.trim();
            if key.chars().count() == 1 {
                bind.key = key.to_lowercase();
            }
        }
    }

    if version < Header::PROJECTS_ON_SETS_VERSION {
        let projects = std::mem::take(&mut config.view.legacy_projects);
        let bound = config.view.filter_set.clone().and_then(|name| {
            config
                .filter_sets
                .iter_mut()
                .find(|entry| !entry.scratch && entry.name.eq_ignore_ascii_case(&name))
        });
        match bound {
            // The entry that was bound was the panel on screen, so the
            // selection beside it was the selection it was being asked of —
            // unless it already states one, in which case the file is right
            // already and the old key is simply dropped.
            Some(entry) => {
                if entry.projects.is_none() {
                    entry.projects = Some(projects);
                }
            }
            // Nothing bound — or a name whose entry someone has since deleted
            // — makes it an unnamed panel's selection, which is what the
            // scratch slot is for. Created only when there is something to
            // put in it: a migrated file that had nothing selected should not
            // grow a `[[filter_set]]` saying so.
            None => {
                if !projects.is_empty() {
                    config.set_scratch_projects(projects);
                }
            }
        }
    }

    config.header.version = Some(Header::CURRENT_VERSION);
    config.migrated = true;
    config
}

fn default_header_type() -> String {
    "tuisana".to_string()
}

/// The header a file with no `[header]` table at all is read as.
///
/// Deliberately not [`Header::default`], which carries the version the app
/// *writes*: that would be the wrong answer for a file the app has never
/// written. "Absent" means unversioned, so the oldest — which is what sends
/// it through [`migrate_v1`] rather than past it.
fn unversioned_header() -> Header {
    Header {
        kind: default_header_type(),
        version: None,
    }
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
        // The sets keys, a second time, for the project view: a named set
        // carries its project list, so the same key does the same thing in
        // both halves of the question. All of these were unbound in project
        // mode, so nothing is displaced — and the per-tab keys deliberately
        // do not come across, because there are no fields and no tab strip
        // here and four of those letters already mean something.
        Bind::with_mode("b", Mode::Project, "filter_sets_toggle"),
        Bind::with_mode("w", Mode::Project, "filter_set_save"),
        Bind::with_mode("y", Mode::Project, "filter_set_copy_to_new"),
        Bind::with_mode("n", Mode::Project, "filter_set_new"),
        Bind::with_mode("d", Mode::Project, "filter_set_delete"),
        Bind::with_mode("<", Mode::Project, "filter_sets_page_back"),
        Bind::with_mode(">", Mode::Project, "filter_sets_page_forward"),
        Bind::with_mode("1", Mode::Project, "filter_set_load_1"),
        Bind::with_mode("2", Mode::Project, "filter_set_load_2"),
        Bind::with_mode("3", Mode::Project, "filter_set_load_3"),
        Bind::with_mode("4", Mode::Project, "filter_set_load_4"),
        Bind::with_mode("5", Mode::Project, "filter_set_load_5"),
        Bind::with_mode("6", Mode::Project, "filter_set_load_6"),
        Bind::with_mode("7", Mode::Project, "filter_set_load_7"),
        Bind::with_mode("8", Mode::Project, "filter_set_load_8"),
        Bind::with_mode("9", Mode::Project, "filter_set_load_9"),
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
        // The same two keys in the panel, where the `list` match mode runs
        // the same completion editor over the same people.
        Bind::with_mode("tab", Mode::FilterEdit, "complete_next_candidate"),
        Bind::with_mode("shift-tab", Mode::FilterEdit, "complete_prev_candidate"),
        Bind::with_mode("alt-b", Mode::FilterEdit, "text_caret_word_back"),
        Bind::with_mode("alt-f", Mode::FilterEdit, "text_caret_word_forward"),
        Bind::with_mode("ctrl-a", Mode::FilterEdit, "text_caret_start"),
        Bind::with_mode("ctrl-e", Mode::FilterEdit, "text_caret_end"),
        // Readline's three forward cuts, which are the deletions the motions
        // above imply: each one takes what it removes to the clipboard, so a
        // cut can be pasted back.
        Bind::with_mode("ctrl-d", Mode::FilterEdit, "text_cut_char"),
        Bind::with_mode("alt-d", Mode::FilterEdit, "text_cut_word"),
        Bind::with_mode("ctrl-k", Mode::FilterEdit, "text_cut_to_end"),
        Bind::with_mode("h", Mode::Calendar, "calendar_prev_day"),
        Bind::with_mode("l", Mode::Calendar, "calendar_next_day"),
        Bind::with_mode("k", Mode::Calendar, "calendar_prev_month"),
        Bind::with_mode("j", Mode::Calendar, "calendar_next_month"),
        Bind::with_mode("t", Mode::Calendar, "calendar_today"),
        Bind::with_mode("d", Mode::Calendar, "calendar_clear"),
        // Punctuation on purpose. Every key above is a letter that also
        // appears in a day name, so hiding the grid has to be reachable by a
        // key that does not: `;` is never part of a date.
        Bind::with_mode(";", Mode::Calendar, "calendar_toggle_grid"),
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
        Bind::with_mode("s", Mode::Task, "toggle_column_sort"),
        Bind::with_mode("o", Mode::Task, "open"),
        Bind::with_mode("space", Mode::Task, "toggle_task_selection"),
        Bind::with_mode("a", Mode::Task, "select_all_visible_tasks"),
        Bind::with_mode("i", Mode::Task, "invert_task_selection"),
        Bind::with_mode("x", Mode::Task, "clear_task_selection"),
        Bind::with_mode("ctrl-x", Mode::Task, "clear_hidden_task_selection"),
        Bind::with_mode("y", Mode::Task, "copy_tasks_to_clipboard"),
        Bind::with_mode("g", Mode::Task, "set_gantt_mode"),
        // The same letter the filter pane uses for its sidebar: both are
        // "show me the panel beside this one".
        Bind::with_mode("b", Mode::Task, "toggle_recent_pane"),
        // The column cursor and the cell editor. `h`, `l`, and `d` are all
        // free in task mode, and `Mode::Any` binds none of them. Editing the
        // cell under the cursor is what `enter` means everywhere else in the
        // app, so it means that here too, and opening in Asana takes `o`.
        Bind::with_mode("h", Mode::Task, "task_column_prev"),
        Bind::with_mode("l", Mode::Task, "task_column_next"),
        Bind::with_mode("enter", Mode::Task, "edit_column"),
        Bind::with_mode("d", Mode::Task, "toggle_task_completed"),
        // `t` is `set_task_mode` globally, which is what gets you *out* of
        // edit mode by way of task mode; from task mode it is the way in.
        Bind::with_mode("t", Mode::Task, "set_edit_mode"),
        // Edit mode. The case split is the whole mnemonic: lowercase acts on
        // the task, uppercase on the structure around it. `i`/`I` is a task
        // and a nested task, `x`/`X` is a task and the section holding it,
        // `j`/`k` move the cursor and `J`/`K` move the task.
        //
        // `i` and `x` mean insert and delete here, which is what they mean
        // in every editor; they are still `invert_task_selection` and
        // `clear_task_selection` in task mode, because the two modes no
        // longer compete for them.
        Bind::with_mode("i", Mode::Edit, "insert_task"),
        Bind::with_mode("I", Mode::Edit, "insert_subtask"),
        Bind::with_mode("x", Mode::Edit, "mark_for_deletion"),
        Bind::with_mode("X", Mode::Edit, "delete_section"),
        Bind::with_mode("S", Mode::Edit, "insert_section"),
        Bind::with_mode("J", Mode::Edit, "move_task_to_next_section"),
        Bind::with_mode("K", Mode::Edit, "move_task_to_prev_section"),
        // Building a selection is part of the flow: `x` acts on it.
        Bind::with_mode("space", Mode::Edit, "toggle_task_selection"),
        Bind::with_mode("enter", Mode::Edit, "delete_marked_tasks"),
        Bind::with_mode("esc", Mode::Edit, "edit_cancel"),
        Bind::with_mode("enter", Mode::ColumnEdit, "commit_column_edit"),
        Bind::with_mode("esc", Mode::ColumnEdit, "cancel_column_edit"),
        // Letters type in this mode, so the value picker's keys are only
        // reachable because a picker reads no text at all.
        Bind::with_mode("j", Mode::ColumnEdit, "column_edit_next_value"),
        Bind::with_mode("k", Mode::ColumnEdit, "column_edit_prev_value"),
        Bind::with_mode("d", Mode::ColumnEdit, "column_edit_clear"),
        Bind::with_mode("ctrl-l", Mode::ColumnEdit, "column_edit_clear"),
        Bind::with_mode("left", Mode::ColumnEdit, "filter_caret_left"),
        Bind::with_mode("right", Mode::ColumnEdit, "filter_caret_right"),
        Bind::with_mode("ctrl-b", Mode::ColumnEdit, "filter_caret_left"),
        Bind::with_mode("ctrl-f", Mode::ColumnEdit, "filter_caret_right"),
        Bind::with_mode("alt-b", Mode::ColumnEdit, "text_caret_word_back"),
        Bind::with_mode("alt-f", Mode::ColumnEdit, "text_caret_word_forward"),
        Bind::with_mode("ctrl-a", Mode::ColumnEdit, "text_caret_start"),
        Bind::with_mode("ctrl-e", Mode::ColumnEdit, "text_caret_end"),
        // The same three cuts, so a cell editor and a filter field still read
        // the same keys.
        Bind::with_mode("ctrl-d", Mode::ColumnEdit, "text_cut_char"),
        Bind::with_mode("alt-d", Mode::ColumnEdit, "text_cut_word"),
        Bind::with_mode("ctrl-k", Mode::ColumnEdit, "text_cut_to_end"),
        // Completion, on the two cells whose values are names the backend
        // knows. `tab` is free in every mode: nothing else reads it.
        Bind::with_mode("tab", Mode::ColumnEdit, "complete_next_candidate"),
        Bind::with_mode("shift-tab", Mode::ColumnEdit, "complete_prev_candidate"),
        // Walking the candidate list without taking any of it, so `enter` is
        // what picks. Free here; in filter-edit mode `ctrl-n` is already the
        // field negation, which is why only `ctrl-p` is bound there and the
        // negation itself is gated on the overlay being open.
        Bind::with_mode("ctrl-n", Mode::ColumnEdit, "highlight_next_candidate"),
        Bind::with_mode("ctrl-p", Mode::ColumnEdit, "highlight_prev_candidate"),
        Bind::with_mode("ctrl-p", Mode::FilterEdit, "highlight_prev_candidate"),
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

    use super::{
        default_bindings, Config, Header, Mode, NamedFilterSet, SavedFilterField, SavedFilterSet,
        V1_COMMAND_RENAMES,
    };
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
        assert_eq!(config.header.version, Some(Header::CURRENT_VERSION));
        assert_eq!(config.bind.len(), 2);
        assert_eq!(config.bind[0].key, "x");
        assert_eq!(config.bind[0].command, "quit");
    }

    // ---- Version 2, and the migration ----------------------------------

    #[test]
    fn a_version_one_file_renames_every_command_and_mode_it_has_to() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [[bind]]
                key = "e"
                mode = "task"
                command = "begin_task_edit"

                [[bind]]
                key = "enter"
                mode = "task_edit"
                command = "commit_task_edit"

                [[bind]]
                key = "esc"
                mode = "task_edit"
                command = "cancel_task_edit"

                [[bind]]
                key = "j"
                mode = "task_edit"
                command = "task_edit_next_value"

                [[bind]]
                key = "k"
                mode = "task_edit"
                command = "task_edit_prev_value"

                [[bind]]
                key = "d"
                mode = "task_edit"
                command = "task_edit_clear"
            "#,
        )
        .expect("a version 1 file still loads");

        assert_eq!(config.header.version, Some(Header::CURRENT_VERSION));
        assert!(config.needs_migration() || config.source_path().is_none());
        // Every pair in the rename table, and the mode name with them.
        for (index, (_, renamed)) in V1_COMMAND_RENAMES.iter().enumerate() {
            assert_eq!(&config.bind[index].command, renamed);
        }
        for bind in &config.bind[1..] {
            assert_eq!(bind.mode, Mode::ColumnEdit, "task_edit is now column_edit");
        }
        // And the renamed names are the ones the action layer accepts.
        KeyMap::from_bindings(&config.effective_bindings()).expect("the renamed config binds");
    }

    #[test]
    fn a_version_one_file_lowercases_its_single_letter_keys() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [[bind]]
                key = "G"
                command = "jump_bottom"

                [[bind]]
                key = "Ctrl-D"
                command = "page_down"

                [[bind]]
                key = "?"
                command = "toggle_help_details"
            "#,
        )
        .expect("a version 1 file still loads");

        // Version 1's reader lowercased it anyway, so `G` meant `g`. Leaving
        // it alone would silently rebind it now that case is a namespace.
        assert_eq!(config.bind[0].key, "g");
        // Modifier forms were never case-sensitive and are left as written;
        // the parser lowercases them.
        assert_eq!(config.bind[1].key, "Ctrl-D");
        assert_eq!(config.bind[2].key, "?");
    }

    #[test]
    fn a_file_with_no_header_at_all_migrates_as_version_one() {
        // `default_header_version` would be the wrong answer for a file the
        // app has never written: it would skip the rename and then fail
        // validation on the old name it was meant to fix.
        let config = Config::from_toml_str(
            r#"
                [[bind]]
                key = "E"
                mode = "task"
                command = "begin_task_edit"
            "#,
        )
        .expect("an unversioned file loads");

        assert_eq!(config.header.version, Some(Header::CURRENT_VERSION));
        assert_eq!(config.bind[0].command, "edit_column");
        assert_eq!(config.bind[0].key, "e");
    }

    #[test]
    fn a_version_two_file_is_left_exactly_as_written() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 2.0

                [[bind]]
                key = "J"
                mode = "edit"
                command = "move_task_to_next_section"
            "#,
        )
        .expect("a version 2 file loads");

        assert!(!config.needs_migration(), "there is nothing to migrate");
        assert_eq!(config.bind[0].key, "J", "from version 2 on, the case written is the case meant");
        assert_eq!(config.bind[0].mode, Mode::Edit);
    }

    #[test]
    fn a_migration_is_not_a_licence_to_accept_a_typo() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [[bind]]
                key = "e"
                command = "begin_task_edi"
            "#,
        )
        .expect("the file itself is valid toml");

        assert_eq!(
            config.bind[0].command, "begin_task_edi",
            "a name that was never renamed is left alone"
        );
        let error = KeyMap::from_bindings(&config.effective_bindings())
            .expect_err("and still fails to bind");
        assert!(format!("{error}").contains("unsupported command"), "{error}");
    }

    /// A file from the future is still rejected: the app cannot guess what a
    /// key it has never heard of means.
    #[test]
    fn rejects_a_version_from_the_future() {
        let err = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 4.0
            "#,
        )
        .expect_err("version should be rejected");

        assert!(format!("{err}").contains("expected header.version = 3"), "{err}");
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
    fn the_confirm_threshold_defaults_to_five_and_asks_above_it() {
        let edit = super::EditConfig::default();

        assert_eq!(edit.confirm_threshold, 5);
        // Strictly greater, so five rows is the largest edit that still
        // happens on the keystroke.
        assert!(!edit.needs_confirmation(5));
        assert!(edit.needs_confirmation(6));
        // A single-row edit is never a bulk edit at the default.
        assert!(!edit.needs_confirmation(1));
    }

    #[test]
    fn a_zero_threshold_asks_about_every_write_and_a_huge_one_asks_about_none() {
        let always = super::EditConfig {
            confirm_threshold: 0,
        };
        let never = super::EditConfig {
            confirm_threshold: usize::MAX,
        };

        assert!(always.needs_confirmation(1));
        assert!(!never.needs_confirmation(10_000));
    }

    #[test]
    fn the_edit_section_round_trips_and_stays_out_of_a_default_config() {
        let config = Config::from_toml_str(
            r#"
                [header]
                type = "tuisana"
                version = 1.0

                [edit]
                confirm_threshold = 20
            "#,
        )
        .expect("the edit section parses");

        assert_eq!(config.edit.confirm_threshold, 20);
        assert!(
            toml::to_string_pretty(&config)
                .expect("serializes")
                .contains("confirm_threshold = 20"),
            "a customized threshold is written back"
        );
        assert!(
            !toml::to_string_pretty(&Config::default())
                .expect("serializes")
                .contains("[edit]"),
            "and a default one does not churn the file"
        );
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
                scratch: false,
                projects: None,
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
                scratch: false,
                projects: None,
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
                scratch: false,
                projects: None,
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
    fn a_named_filter_set_round_trips_its_project_selection() {
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 2.0

[[filter_set]]
name = "Sprint triage"
projects = ["me", "1201", "1202"]
"#,
        )
        .expect("the projects key parses");

        assert_eq!(
            config.filter_sets[0].projects.as_deref(),
            Some(["me".to_string(), "1201".to_string(), "1202".to_string()].as_slice()),
            "gids in file order, with `me` for the assigned-to-me row"
        );

        let text = toml::to_string_pretty(&config).expect("serializes");
        assert_eq!(
            Config::from_toml_str(&text)
                .expect("reparses")
                .filter_sets[0]
                .projects,
            config.filter_sets[0].projects,
        );
    }

    /// Absent is "no opinion" and `[]` is "select nothing", so the two cannot
    /// collapse into each other anywhere along the round trip.
    #[test]
    fn a_missing_projects_key_and_an_empty_one_are_different_values() {
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 2.0

[[filter_set]]
name = "Silent"

[[filter_set]]
name = "Nothing"
projects = []
"#,
        )
        .expect("both parse");

        assert_eq!(config.filter_sets[0].projects, None, "no opinion");
        assert_eq!(
            config.filter_sets[1].projects,
            Some(Vec::new()),
            "an opinion that selects nothing"
        );

        let text = toml::to_string_pretty(&config).expect("serializes");
        assert!(
            text.contains("projects = []"),
            "an empty opinion is still written: {text}"
        );
        let reparsed = Config::from_toml_str(&text).expect("reparses");
        assert_eq!(reparsed.filter_sets[0].projects, None);
        assert_eq!(reparsed.filter_sets[1].projects, Some(Vec::new()));
    }

    /// A version-2 file's selection was being used with the panel that was
    /// bound, so that is the entry it belongs to.
    #[test]
    fn a_version_two_selection_moves_onto_the_entry_that_was_bound() {
        let text = r#"
[header]
type = "tuisana"
version = 2.0

[view]
projects = ["1201"]
filter_set = "Mine"

[[filter_set]]
name = "Mine"

  [[filter_set.set]]

    [[filter_set.set.field]]
    key = "assignee"
    query = "alex"
"#;
        let config = Config::from_toml_str(text).expect("the old shape loads");

        assert_eq!(config.header.version(), Header::CURRENT_VERSION);
        assert_eq!(config.migrated_from_version(), 2.0);
        assert_eq!(config.filter_sets[0].projects(), ["1201".to_string()]);
        assert!(
            config.scratch_projects().is_empty(),
            "the bound entry took it, so no scratch slot was needed"
        );
        assert!(
            config.view.legacy_projects.is_empty(),
            "and `[view]` no longer carries it"
        );
    }

    /// With nothing bound there is no entry to adopt the selection, so it
    /// goes to the slot an unnamed panel uses from here on.
    #[test]
    fn an_unbound_version_two_selection_moves_onto_the_scratch_entry() {
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 2.0

[view]
projects = ["1201", "1202"]
"#,
        )
        .expect("the old shape loads");

        assert_eq!(
            config.scratch_projects(),
            ["1201".to_string(), "1202".to_string()]
        );
        let scratch = config
            .filter_sets
            .iter()
            .find(|entry| entry.scratch)
            .expect("the slot was created");
        assert!(scratch.sets.is_empty(), "it holds a selection and nothing else");
        assert!(
            config.sorted_filter_sets().is_empty(),
            "and the sidebar does not list it"
        );
        assert_eq!(config.named_filter_set_count(), 0);
        assert_eq!(config.named_filter_set("unnamed"), None, "nor is it found by name");
    }

    /// An entry that already states its projects is right already, and the
    /// empty list it may hold is a real answer rather than a gap.
    #[test]
    fn migration_does_not_overwrite_a_selection_an_entry_already_states() {
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 2.0

[view]
projects = ["1201"]
filter_set = "Mine"

[[filter_set]]
name = "Mine"
projects = []
"#,
        )
        .expect("loads");

        assert_eq!(config.filter_sets[0].projects(), [] as [String; 0]);
        assert!(
            config.scratch_projects().is_empty(),
            "and the old list is dropped rather than parked somewhere else"
        );
        assert_eq!(config.filter_sets.len(), 1, "no slot was created for it");
    }

    /// A version-1 file needs both steps, and the second one does not care
    /// which version it was reached from.
    #[test]
    fn a_version_one_config_gets_the_renames_and_the_selection_move() {
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0

[view]
projects = ["1201"]

[[bind]]
key = "G"
command = "begin_task_edit"
"#,
        )
        .expect("loads");

        assert_eq!(config.header.version(), Header::CURRENT_VERSION);
        assert_eq!(config.migrated_from_version(), 1.0);
        assert_eq!(config.bind[0].key, "g");
        assert_eq!(config.bind[0].command, "edit_column");
        assert_eq!(config.scratch_projects(), ["1201".to_string()]);
    }

    #[test]
    fn rejects_a_second_scratch_entry() {
        let error = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 3.0

[[filter_set]]
name = "unnamed"
scratch = true

[[filter_set]]
name = "also unnamed"
scratch = true
"#,
        )
        .expect_err("two slots for one unnamed panel is not a thing");

        assert!(
            error.to_string().contains("only one filter_set may set scratch"),
            "{error}"
        );
    }

    /// Nothing lists a scratch entry and nothing looks it up by name, so its
    /// name is free to collide with a real one.
    #[test]
    fn a_scratch_entry_does_not_collide_with_a_real_name() {
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 3.0

[[filter_set]]
name = "unnamed"
scratch = true
projects = ["1201"]

[[filter_set]]
name = "unnamed"
projects = ["1202"]
"#,
        )
        .expect("the flag is the identity, not the name");

        assert_eq!(config.scratch_projects(), ["1201".to_string()]);
        assert_eq!(
            config.named_filter_set("unnamed").map(NamedFilterSet::projects),
            Some(["1202".to_string()].as_slice()),
        );
    }

    #[test]
    fn a_projects_list_naming_the_same_project_twice_is_collapsed_not_refused() {
        // A hand-edited file that names a project twice meant it once.
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 2.0

[[filter_set]]
name = "Mine"
projects = ["1201", "me", "1201", "me"]
"#,
        )
        .expect("duplicates are collapsed rather than rejected");

        assert_eq!(
            config.filter_sets[0].projects.as_deref(),
            Some(["1201".to_string(), "me".to_string()].as_slice()),
            "first mention wins, so the file's order survives"
        );
    }

    #[test]
    fn rejects_a_blank_project_in_a_named_filter_set() {
        let error = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 2.0

[[filter_set]]
name = "Mine"
projects = ["1201", "  "]
"#,
        )
        .expect_err("a project that names nothing is refused");

        assert!(
            error
                .to_string()
                .contains("filter_set.projects entries must not be empty"),
            "{error}"
        );
    }

    /// A saved entry with nothing but a name, for the ordering assertions.
    fn named(name: &str) -> NamedFilterSet {
        NamedFilterSet {
            name: name.to_string(),
            scratch: false,
            projects: None,
            sets: Vec::new(),
        }
    }

    #[test]
    fn the_sidebar_order_is_by_name_rather_than_by_file_order() {
        let config = Config {
            filter_sets: vec![named("zebra"), named("Apple"), named("mango")],
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
            Some(&Action::ToggleColumnSort)
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
            (KeyBinding::Enter, Action::EditColumn),
            (KeyBinding::Char('o'), Action::Open),
            (KeyBinding::Char('d'), Action::ToggleTaskCompleted),
            (KeyBinding::Char('b'), Action::ToggleRecentPane),
        ] {
            assert_eq!(keymap.action_for(&key, Mode::Task), Some(&action));
        }
    }

    /// The completion keys, in both panes that run the same editor.
    #[test]
    fn default_bindings_complete_in_the_cell_editor_and_the_filter_panel() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        for mode in [Mode::ColumnEdit, Mode::FilterEdit] {
            assert_eq!(
                keymap.action_for(&KeyBinding::Tab, mode),
                Some(&Action::CompleteCandidate(1)),
                "{mode:?}"
            );
            assert_eq!(
                keymap.action_for(&KeyBinding::BackTab, mode),
                Some(&Action::CompleteCandidate(-1)),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn default_bindings_include_task_edit_controls() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        for (key, action) in [
            (KeyBinding::Enter, Action::CommitColumnEdit),
            (KeyBinding::Esc, Action::CancelColumnEdit),
            (KeyBinding::Char('j'), Action::ColumnEditCycleValue(1)),
            (KeyBinding::Char('k'), Action::ColumnEditCycleValue(-1)),
            (KeyBinding::Char('d'), Action::ColumnEditClear),
            (KeyBinding::Alt('b'), Action::TextCaretWordBack),
            (KeyBinding::Alt('f'), Action::TextCaretWordForward),
            (KeyBinding::Ctrl('a'), Action::TextCaretStart),
            (KeyBinding::Ctrl('e'), Action::TextCaretEnd),
            (KeyBinding::Ctrl('d'), Action::TextCutChar),
            (KeyBinding::Alt('d'), Action::TextCutWord),
            (KeyBinding::Ctrl('k'), Action::TextCutToEnd),
        ] {
            assert_eq!(
                keymap.action_for(&key, Mode::ColumnEdit),
                Some(&action),
                "{key:?} in column-edit mode"
            );
        }
    }

    /// Edit mode's twelve keys, and the two rules behind them: lowercase acts
    /// on the task, uppercase on the structure around it.
    #[test]
    fn default_bindings_include_the_edit_mode_controls() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        for (key, action) in [
            (KeyBinding::Char('i'), Action::InsertTask),
            (KeyBinding::Char('I'), Action::InsertSubtask),
            (KeyBinding::Char('x'), Action::MarkForDeletion),
            (KeyBinding::Char('X'), Action::DeleteSection),
            (KeyBinding::Char('S'), Action::InsertSection),
            (KeyBinding::Char('J'), Action::MoveTaskToSection(1)),
            (KeyBinding::Char('K'), Action::MoveTaskToSection(-1)),
            (KeyBinding::Char(' '), Action::ToggleTaskSelection),
            (KeyBinding::Enter, Action::DeleteMarkedTasks),
            (KeyBinding::Esc, Action::EditCancel),
        ] {
            assert_eq!(
                keymap.action_for(&key, Mode::Edit),
                Some(&action),
                "{key:?} in edit mode"
            );
        }

        // `t` is the way in, from task mode.
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('t'), Mode::Task),
            Some(&Action::SetEditMode)
        );
        // And `i` and `x` still mean what they always meant one mode over.
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('i'), Mode::Task),
            Some(&Action::InvertTaskSelection)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('x'), Mode::Task),
            Some(&Action::ClearTaskSelection)
        );
    }

    /// Edit mode takes the `Any` fallback, because moving the cursor is most
    /// of what you do between edits — but not task mode's own bindings.
    #[test]
    fn edit_mode_keeps_the_global_keys_and_none_of_task_modes() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        for (key, action) in [
            (KeyBinding::Char('j'), Action::MoveDown),
            (KeyBinding::Char('k'), Action::MoveUp),
            (KeyBinding::Ctrl('u'), Action::PageUp),
            (KeyBinding::Ctrl('d'), Action::PageDown),
            (KeyBinding::Home, Action::JumpTop),
            (KeyBinding::End, Action::JumpBottom),
            (KeyBinding::Char('r'), Action::Refresh),
            (KeyBinding::Char('q'), Action::Quit),
            (KeyBinding::Char('?'), Action::ToggleHelpDetails),
            // `[` and `]` resize the top pane here rather than moving by
            // section, because they are the global bindings.
            (KeyBinding::Char('['), Action::ResizeTopPaneDown),
            (KeyBinding::Char(']'), Action::ResizeTopPaneUp),
        ] {
            assert_eq!(
                keymap.action_for(&key, Mode::Edit),
                Some(&action),
                "{key:?} in edit mode"
            );
        }

        // Task mode's own keys are off here.
        for key in [
            KeyBinding::Char('h'),
            KeyBinding::Char('l'),
            KeyBinding::Char('c'),
            KeyBinding::Char('s'),
            KeyBinding::Char('z'),
            KeyBinding::Char('g'),
        ] {
            assert_eq!(
                keymap.action_for(&key, Mode::Edit),
                None,
                "{key:?} is a task-mode key"
            );
        }
    }

    /// Nothing in `Mode::Any` may take an uppercase letter: the text-entry
    /// modes do not fall back to it, so it would be safe today and a key that
    /// silently stopped typing in any mode added later.
    #[test]
    fn no_global_binding_takes_an_uppercase_letter() {
        for bind in default_bindings() {
            if !bind.mode.is_any() {
                continue;
            }
            let key: KeyBinding = bind.key.parse().expect("default bindings parse");
            if let KeyBinding::Char(ch) = key {
                assert!(
                    !ch.is_uppercase(),
                    "{} is a global binding on an uppercase letter",
                    bind.key
                );
            }
        }
    }

    /// The cuts are bound wherever the motions are, because they are the
    /// deletions those motions imply: a field that can move a word can cut
    /// one.
    #[test]
    fn the_cut_keys_are_the_same_in_every_text_editing_mode() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        for mode in [Mode::FilterEdit, Mode::ColumnEdit] {
            for (key, action) in [
                (KeyBinding::Ctrl('d'), Action::TextCutChar),
                (KeyBinding::Alt('d'), Action::TextCutWord),
                (KeyBinding::Ctrl('k'), Action::TextCutToEnd),
            ] {
                assert_eq!(
                    keymap.action_for(&key, mode),
                    Some(&action),
                    "{key:?} in {mode:?}"
                );
            }
            // `d` still types, and still clears a value picker, which is why
            // cutting a word needs the alt- pair.
            assert_ne!(
                keymap.action_for(&KeyBinding::Char('d'), mode),
                Some(&Action::TextCutWord),
                "{mode:?}"
            );
        }
    }

    /// An unbound letter has to type into the cell rather than fire the
    /// global binding that letter carries.
    #[test]
    fn task_edit_mode_does_not_fall_back_to_the_global_bindings() {
        let keymap = KeyMap::from_bindings(&Config::default().effective_bindings())
            .expect("default bindings parse");

        assert!(!Mode::ColumnEdit.allows_any_fallback());
        assert_eq!(keymap.action_for(&KeyBinding::Char('q'), Mode::ColumnEdit), None);
    }
}

#[cfg(test)]
mod theme_config_tests {
    use super::{Config, GanttConfig, ThemeConfig, ThemeGlyphs, ThemeVariant, TopPaneState, ViewConfig};
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
    fn view_defaults_are_omitted_when_the_config_is_serialized() {
        // Same rule as `[gantt]`: the app rewrites the whole file, and a
        // section nobody has touched must not start appearing in it.
        let serialized = toml::to_string_pretty(&Config::default()).expect("serializes");

        assert!(!serialized.contains("[view]"), "{serialized}");
    }

    #[test]
    fn a_recorded_view_survives_a_serialize_round_trip() {
        let config = Config {
            view: ViewConfig {
                filter_set: Some("Overdue mine".to_string()),
                top_pane: TopPaneState::Maximized,
                tasks: true,
                filters: true,
                filter_sidebar: true,
                recent: false,
                legacy_projects: Vec::new(),
            },
            ..Config::default()
        };

        let serialized = toml::to_string_pretty(&config).expect("serializes");
        let parsed = Config::from_toml_str(&serialized).expect("reparses");

        assert_eq!(parsed.view, config.view);
    }

    #[test]
    fn a_view_section_reads_back_what_it_says_and_defaults_the_rest() {
        let config = Config::from_toml_str(
            r#"
[header]
type = "tuisana"
version = 1.0

[view]
tasks = true
projects = ["1201"]
"#,
        )
        .expect("config parses");

        assert!(config.view.tasks);
        assert_eq!(
            config.scratch_projects(),
            ["1201"],
            "a version-1 selection lands in the slot an unnamed panel uses"
        );
        assert!(config.view.legacy_projects.is_empty());
        assert!(!config.view.filters);
        assert_eq!(config.view.top_pane, TopPaneState::Normal);
        assert!(
            config.view.recent,
            "the recently-edited pane is on unless it was turned off"
        );
        assert_eq!(config.view.filter_set, None);
    }

    #[test]
    fn rejects_a_view_that_names_an_empty_filter_set_or_project() {
        for (body, expected) in [
            ("filter_set = \"  \"", "view.filter_set"),
            ("projects = [\"\"]", "view.projects"),
        ] {
            let error = Config::from_toml_str(&format!(
                "[header]\ntype = \"tuisana\"\nversion = 1.0\n\n[view]\n{body}\n"
            ))
            .expect_err("an empty name is rejected");

            assert!(
                error.to_string().contains(expected),
                "unexpected error: {error}"
            );
        }
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

#[cfg(test)]
mod example_tests {
    use super::Config;

    /// `tuisana.toml.example` is documentation that has to stay true: every
    /// command it names must parse, and its header must be the version the
    /// app writes.
    #[test]
    fn the_example_config_is_a_valid_version_two_config() {
        let text = include_str!("../../tuisana.toml.example");
        let config = Config::from_toml_str(text).expect("the example config loads");

        assert!(
            !config.needs_migration(),
            "the example is a version 2 file and needs no migration"
        );
        crate::input::KeyMap::from_bindings(&config.effective_bindings())
            .expect("every command the example names binds");
    }
}

