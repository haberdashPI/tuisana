use std::{collections::HashMap, fmt::Display, str::FromStr};

use crate::{
    config::Bind,
    error::{Error, Result},
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeyBinding {
    Char(char),
    Ctrl(char),
    Enter,
    Esc,
    Up,
    Down,
    PageUp,
    PageDown,
}

impl FromStr for KeyBinding {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let normalized = s.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "enter" => Ok(Self::Enter),
            "esc" | "escape" => Ok(Self::Esc),
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            "pageup" | "page-up" => Ok(Self::PageUp),
            "pagedown" | "page-down" => Ok(Self::PageDown),
            _ if normalized.len() == 1 => Ok(Self::Char(normalized.chars().next().expect("len checked"))),
            _ if normalized.starts_with("ctrl-") && normalized.len() == 6 => {
                Ok(Self::Ctrl(normalized.chars().last().expect("len checked")))
            }
            _ => Err(Error::Backend(format!("unsupported key binding: {s}"))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Quit,
    MoveUp,
    MoveDown,
    Open,
    Refresh,
    PageUp,
    PageDown,
}

impl Action {
    pub fn from_command(command: &str) -> Result<Self> {
        match command.trim().to_ascii_lowercase().as_str() {
            "quit" => Ok(Self::Quit),
            "move_up" => Ok(Self::MoveUp),
            "move_down" => Ok(Self::MoveDown),
            "open" => Ok(Self::Open),
            "refresh" => Ok(Self::Refresh),
            "page_up" => Ok(Self::PageUp),
            "page_down" => Ok(Self::PageDown),
            other => Err(Error::Backend(format!("unsupported command: {other}"))),
        }
    }
}

impl Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Action::Quit => "quit",
            Action::MoveUp => "move_up",
            Action::MoveDown => "move_down",
            Action::Open => "open",
            Action::Refresh => "refresh",
            Action::PageUp => "page_up",
            Action::PageDown => "page_down",
        };
        f.write_str(name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyMap {
    bindings: HashMap<KeyBinding, Action>,
}

impl KeyMap {
    pub fn from_bindings(bindings: &[Bind]) -> Result<Self> {
        let mut map = HashMap::new();
        for bind in bindings {
            let key = bind.key.parse()?;
            let action = Action::from_command(&bind.command)?;
            map.insert(key, action);
        }
        Ok(Self { bindings: map })
    }

    pub fn action_for(&self, key: &KeyBinding) -> Option<&Action> {
        self.bindings.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, KeyBinding, KeyMap};
    use crate::config::Bind;

    #[test]
    fn parses_keys_and_builds_keymap() {
        let keymap = KeyMap::from_bindings(&[
            Bind {
                key: "x".to_string(),
                command: "quit".to_string(),
            },
            Bind {
                key: "j".to_string(),
                command: "move_down".to_string(),
            },
            Bind {
                key: "ctrl-u".to_string(),
                command: "page_up".to_string(),
            },
        ])
        .expect("keymap parses");

        assert_eq!(keymap.action_for(&KeyBinding::Char('x')), Some(&Action::Quit));
        assert_eq!(keymap.action_for(&KeyBinding::Char('j')), Some(&Action::MoveDown));
        assert_eq!(keymap.action_for(&KeyBinding::Ctrl('u')), Some(&Action::PageUp));
    }

    #[test]
    fn parses_key_aliases_and_commands() {
        assert_eq!("enter".parse::<KeyBinding>().expect("enter parses"), KeyBinding::Enter);
        assert_eq!("esc".parse::<KeyBinding>().expect("esc parses"), KeyBinding::Esc);
        assert_eq!("page-up".parse::<KeyBinding>().expect("page-up parses"), KeyBinding::PageUp);
        assert_eq!(Action::from_command("open").expect("open parses"), Action::Open);
        assert_eq!(Action::from_command("refresh").expect("refresh parses"), Action::Refresh);
    }

    #[test]
    fn rejects_unknown_key_and_command() {
        let key_err = "meta-x"
            .parse::<KeyBinding>()
            .expect_err("unknown key should fail");
        assert!(format!("{key_err}").contains("unsupported key binding"));

        let command_err = Action::from_command("launch").expect_err("unknown command should fail");
        assert!(format!("{command_err}").contains("unsupported command"));
    }
}
