use serde::Deserialize;
use std::{fs, path::Path};

use crate::error::Result;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default)]
    pub keys: KeyBindings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keys: KeyBindings::default(),
        }
    }
}

impl Config {
    pub fn from_toml_str(input: &str) -> Result<Self> {
        Ok(toml::from_str(input)?)
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path)?;
        Self::from_toml_str(&contents)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct KeyBindings {
    #[serde(default = "default_quit")]
    pub quit: Vec<String>,
    #[serde(default = "default_up")]
    pub up: Vec<String>,
    #[serde(default = "default_down")]
    pub down: Vec<String>,
    #[serde(default = "default_open")]
    pub open: Vec<String>,
    #[serde(default = "default_refresh")]
    pub refresh: Vec<String>,
    #[serde(default = "default_page_up")]
    pub page_up: Vec<String>,
    #[serde(default = "default_page_down")]
    pub page_down: Vec<String>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            quit: default_quit(),
            up: default_up(),
            down: default_down(),
            open: default_open(),
            refresh: default_refresh(),
            page_up: default_page_up(),
            page_down: default_page_down(),
        }
    }
}

fn default_quit() -> Vec<String> {
    vec!["q".to_string(), "ctrl-c".to_string()]
}

fn default_up() -> Vec<String> {
    vec!["k".to_string(), "up".to_string()]
}

fn default_down() -> Vec<String> {
    vec!["j".to_string(), "down".to_string()]
}

fn default_open() -> Vec<String> {
    vec!["enter".to_string()]
}

fn default_refresh() -> Vec<String> {
    vec!["r".to_string()]
}

fn default_page_up() -> Vec<String> {
    vec!["ctrl-u".to_string()]
}

fn default_page_down() -> Vec<String> {
    vec!["ctrl-d".to_string()]
}

#[cfg(test)]
mod tests {
    use super::{Config, KeyBindings};

    #[test]
    fn parses_toml_and_keeps_defaults() {
        let config = Config::from_toml_str(
            r#"
                [keys]
                quit = ["x"]
                down = ["j", "down"]
            "#,
        )
        .expect("config parses");

        assert_eq!(
            config.keys,
            KeyBindings {
                quit: vec!["x".to_string()],
                up: vec!["k".to_string(), "up".to_string()],
                down: vec!["j".to_string(), "down".to_string()],
                open: vec!["enter".to_string()],
                refresh: vec!["r".to_string()],
                page_up: vec!["ctrl-u".to_string()],
                page_down: vec!["ctrl-d".to_string()],
            }
        );
    }
}

