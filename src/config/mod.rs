//! Parsed application configuration and binding mode definitions.
//!
//! `Config` is the single source of truth for the loaded TOML contents and the
//! path they came from. The app loads it once at startup, then uses the stored
//! source path later when persisting project visibility changes back to disk.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::{
    fs,
    path::{Path, PathBuf},
};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthConfig>,
    #[serde(default)]
    pub bind: Vec<Bind>,
    #[serde(default, rename = "project")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub project_visibility: Vec<ProjectVisibilityConfig>,
    #[serde(skip)]
    source_path: Option<PathBuf>,
}

impl PartialEq for Config {
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header
            && self.auth == other.auth
            && self.bind == other.bind
            && self.project_visibility == other.project_visibility
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            header: Header::default(),
            auth: None,
            bind: default_bindings(),
            project_visibility: Vec::new(),
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
    Task,
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
        !matches!(self, Self::ProjectSearch | Self::FilterEdit)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Project => "project",
            Self::ProjectSearch => "project-search",
            Self::Filter => "filter",
            Self::FilterEdit => "filter-edit",
            Self::Task => "task",
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
        Bind::new("t", "toggle_task_view"),
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
        Bind::with_mode("ctrl-f", Mode::Filter, "search_fuzzy"),
        Bind::with_mode("ctrl-s", Mode::Filter, "search_substring"),
        Bind::with_mode("ctrl-r", Mode::Filter, "search_regex"),
        Bind::with_mode("enter", Mode::FilterEdit, "filter_done_editing"),
        Bind::with_mode("esc", Mode::FilterEdit, "filter_cancel_editing"),
        Bind::with_mode("ctrl-l", Mode::FilterEdit, "clear_search"),
        Bind::with_mode("ctrl-f", Mode::FilterEdit, "search_fuzzy"),
        Bind::with_mode("ctrl-s", Mode::FilterEdit, "search_substring"),
        Bind::with_mode("ctrl-r", Mode::FilterEdit, "search_regex"),
        Bind::with_mode("h", Mode::FilterEdit, "filter_move_label_left"),
        Bind::with_mode("l", Mode::FilterEdit, "filter_move_label_right"),
        Bind::with_mode("j", Mode::FilterEdit, "filter_cycle_label_down"),
        Bind::with_mode("k", Mode::FilterEdit, "filter_cycle_label_up"),
        Bind::with_mode("a", Mode::FilterEdit, "filter_add_label"),
        Bind::with_mode("d", Mode::FilterEdit, "filter_delete_label"),
        Bind::with_mode("[", Mode::Task, "move_section_up"),
        Bind::with_mode("]", Mode::Task, "move_section_down"),
        Bind::with_mode("{", Mode::Task, "move_project_up"),
        Bind::with_mode("}", Mode::Task, "move_project_down"),
        Bind::with_mode("o", Mode::Project, "toggle_only_selected"),
        Bind::with_mode("ctrl-f", Mode::Project, "search_fuzzy"),
        Bind::with_mode("ctrl-s", Mode::Project, "search_substring"),
        Bind::with_mode("ctrl-r", Mode::Project, "search_regex"),
        Bind::with_mode("c", Mode::Task, "toggle_completed_filter"),
        Bind::with_mode("z", Mode::Task, "toggle_subtask_visibility"),
        Bind::with_mode(",", Mode::Task, "toggle_project_grouping"),
        Bind::with_mode(".", Mode::Task, "toggle_section_grouping"),
        Bind::with_mode("s", Mode::Task, "cycle_task_sort"),
    ]
}

#[cfg(test)]
mod tests {
    use crate::input::{Action, KeyBinding, KeyMap};

    use super::{Config, Mode};
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
            Some(&Action::ToggleTaskView)
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
            keymap.action_for(&KeyBinding::Ctrl('f'), Mode::Filter),
            Some(&Action::SearchFuzzy)
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
            keymap.action_for(&KeyBinding::Ctrl('f'), Mode::FilterEdit),
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
            keymap.action_for(&KeyBinding::Char(','), Mode::Task),
            Some(&Action::ToggleProjectGrouping)
        );
        assert_eq!(
            keymap.action_for(&KeyBinding::Char('.'), Mode::Task),
            Some(&Action::ToggleSectionGrouping)
        );
    }
}
