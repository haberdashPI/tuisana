use serde::Deserialize;
use std::{fs, path::Path};

use crate::error::{Error, Result};

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub header: Header,
    #[serde(default)]
    pub auth: Option<AuthConfig>,
    #[serde(default)]
    pub bind: Vec<Bind>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            header: Header::default(),
            auth: None,
            bind: default_bindings(),
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

        Ok(())
    }

}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
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

#[derive(Clone, Debug, Deserialize, PartialEq)]
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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
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
            key: "ctrl-u".to_string(),
            command: "page_up".to_string(),
        },
        Bind {
            key: "ctrl-d".to_string(),
            command: "page_down".to_string(),
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
            "#,
        )
        .expect("auth config parses");

        let auth = config.auth.expect("auth section present");
        assert_eq!(auth.personal_access_token, "pat_123");
        assert_eq!(auth.workspace_gid.as_deref(), Some("42"));
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
}
