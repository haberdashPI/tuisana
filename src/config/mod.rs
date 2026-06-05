use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::{fs, path::Path};

use crate::error::{Error, Result};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            header: Header::default(),
            auth: None,
            bind: default_bindings(),
            project_visibility: Vec::new(),
        }
    }
}

impl Config {
    pub fn from_toml_str(input: &str) -> Result<Self> {
        let config: Self = toml::from_str(input)?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path)?;
        Self::from_toml_str(&contents)
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<()> {
        let contents = toml::to_string_pretty(self)
            .map_err(|err| Error::ConfigValidation(format!("failed to serialize config: {err}")))?;
        fs::write(path, contents)?;
        Ok(())
    }

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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
pub struct Bind {
    pub key: String,
    pub command: String,
}

fn default_header_type() -> String {
    "tuisana".to_string()
}

fn default_header_version() -> f64 {
    1.0
}

fn default_bindings() -> Vec<Bind> {
    vec![
        Bind {
            key: "?".to_string(),
            command: "toggle_help_details".to_string(),
        },
        Bind {
            key: "q".to_string(),
            command: "quit".to_string(),
        },
        Bind {
            key: "ctrl-c".to_string(),
            command: "quit".to_string(),
        },
        Bind {
            key: "k".to_string(),
            command: "move_up".to_string(),
        },
        Bind {
            key: "up".to_string(),
            command: "move_up".to_string(),
        },
        Bind {
            key: "j".to_string(),
            command: "move_down".to_string(),
        },
        Bind {
            key: "down".to_string(),
            command: "move_down".to_string(),
        },
        Bind {
            key: "enter".to_string(),
            command: "open".to_string(),
        },
        Bind {
            key: "r".to_string(),
            command: "refresh".to_string(),
        },
        Bind {
            key: "/".to_string(),
            command: "start_search".to_string(),
        },
        Bind {
            key: "ctrl-l".to_string(),
            command: "clear_search".to_string(),
        },
        Bind {
            key: "space".to_string(),
            command: "toggle_selection".to_string(),
        },
        Bind {
            key: "a".to_string(),
            command: "select_all_visible".to_string(),
        },
        Bind {
            key: "i".to_string(),
            command: "invert_selection".to_string(),
        },
        Bind {
            key: "c".to_string(),
            command: "clear_selection".to_string(),
        },
        Bind {
            key: "u".to_string(),
            command: "undo_selection".to_string(),
        },
        Bind {
            key: "ctrl-y".to_string(),
            command: "redo_selection".to_string(),
        },
        Bind {
            key: "home".to_string(),
            command: "jump_top".to_string(),
        },
        Bind {
            key: "end".to_string(),
            command: "jump_bottom".to_string(),
        },
        Bind {
            key: "*".to_string(),
            command: "toggle_starred_selected".to_string(),
        },
        Bind {
            key: "h".to_string(),
            command: "toggle_hidden_selected".to_string(),
        },
        Bind {
            key: "v".to_string(),
            command: "toggle_hidden_group".to_string(),
        },
        Bind {
            key: "t".to_string(),
            command: "toggle_task_view".to_string(),
        },
        Bind {
            key: "m".to_string(),
            command: "toggle_task_mode".to_string(),
        },
        Bind {
            key: "o".to_string(),
            command: "toggle_only_selected".to_string(),
        },
        Bind {
            key: "ctrl-f".to_string(),
            command: "search_fuzzy".to_string(),
        },
        Bind {
            key: "ctrl-s".to_string(),
            command: "search_substring".to_string(),
        },
        Bind {
            key: "ctrl-r".to_string(),
            command: "search_regex".to_string(),
        },
        Bind {
            key: "ctrl-u".to_string(),
            command: "page_up".to_string(),
        },
        Bind {
            key: "ctrl-d".to_string(),
            command: "page_down".to_string(),
        },
        Bind {
            key: "left".to_string(),
            command: "scroll_left".to_string(),
        },
        Bind {
            key: "right".to_string(),
            command: "scroll_right".to_string(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::Config;
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
}
