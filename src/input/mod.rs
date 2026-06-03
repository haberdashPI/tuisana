use std::{collections::HashMap, fmt::Display, str::FromStr};

use crate::{config::KeyBindings, error::{Error, Result}};

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
    pub fn from_key_bindings(bindings: &KeyBindings) -> Result<Self> {
        let mut map = HashMap::new();
        insert_many(&mut map, Action::Quit, &bindings.quit)?;
        insert_many(&mut map, Action::MoveUp, &bindings.up)?;
        insert_many(&mut map, Action::MoveDown, &bindings.down)?;
        insert_many(&mut map, Action::Open, &bindings.open)?;
        insert_many(&mut map, Action::Refresh, &bindings.refresh)?;
        insert_many(&mut map, Action::PageUp, &bindings.page_up)?;
        insert_many(&mut map, Action::PageDown, &bindings.page_down)?;
        Ok(Self { bindings: map })
    }

    pub fn action_for(&self, key: &KeyBinding) -> Option<&Action> {
        self.bindings.get(key)
    }
}

fn insert_many(map: &mut HashMap<KeyBinding, Action>, action: Action, keys: &[String]) -> Result<()> {
    for key in keys {
        map.insert(key.parse()?, action.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Action, KeyBinding, KeyMap};
    use crate::config::KeyBindings;

    #[test]
    fn parses_keys_and_builds_keymap() {
        let keymap = KeyMap::from_key_bindings(&KeyBindings {
            quit: vec!["x".to_string()],
            up: vec!["k".to_string()],
            down: vec!["j".to_string()],
            open: vec!["enter".to_string()],
            refresh: vec!["r".to_string()],
            page_up: vec!["ctrl-u".to_string()],
            page_down: vec!["ctrl-d".to_string()],
        })
        .expect("keymap parses");

        assert_eq!(keymap.action_for(&KeyBinding::Char('x')), Some(&Action::Quit));
        assert_eq!(keymap.action_for(&KeyBinding::Char('j')), Some(&Action::MoveDown));
        assert_eq!(keymap.action_for(&KeyBinding::Ctrl('u')), Some(&Action::PageUp));
    }
}

