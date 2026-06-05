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
    Backspace,
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
            "backspace" => Ok(Self::Backspace),
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            "pageup" | "page-up" => Ok(Self::PageUp),
            "pagedown" | "page-down" => Ok(Self::PageDown),
            "space" => Ok(Self::Char(' ')),
            _ if normalized.len() == 1 => Ok(Self::Char(normalized.chars().next().expect("len checked"))),
            _ if normalized.starts_with("ctrl-") && normalized.len() == 6 => {
                Ok(Self::Ctrl(normalized.chars().last().expect("len checked")))
            }
            _ => Err(Error::Backend(format!("unsupported key binding: {s}"))),
        }
    }
}

impl KeyBinding {
    pub fn from_crossterm_event(event: crossterm::event::KeyEvent) -> Option<Self> {
        use crossterm::event::{KeyCode, KeyModifiers};

        match event.code {
            KeyCode::Char(c) if event.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Self::Ctrl(c.to_ascii_lowercase()))
            }
            KeyCode::Char(c) => Some(Self::Char(c.to_ascii_lowercase())),
            KeyCode::Enter => Some(Self::Enter),
            KeyCode::Esc => Some(Self::Esc),
            KeyCode::Backspace => Some(Self::Backspace),
            KeyCode::Up => Some(Self::Up),
            KeyCode::Down => Some(Self::Down),
            KeyCode::PageUp => Some(Self::PageUp),
            KeyCode::PageDown => Some(Self::PageDown),
            _ => None,
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
    StartSearch,
    ToggleSelection,
    ToggleOnlySelected,
    ToggleStarredSelected,
    ToggleHiddenSelected,
    ToggleHiddenGroup,
    SearchFuzzy,
    SearchSubstring,
    SearchRegex,
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
            "start_search" => Ok(Self::StartSearch),
            "toggle_selection" => Ok(Self::ToggleSelection),
            "toggle_only_selected" => Ok(Self::ToggleOnlySelected),
            "toggle_starred_selected" => Ok(Self::ToggleStarredSelected),
            "toggle_hidden_selected" => Ok(Self::ToggleHiddenSelected),
            "toggle_hidden_group" => Ok(Self::ToggleHiddenGroup),
            "search_fuzzy" => Ok(Self::SearchFuzzy),
            "search_substring" => Ok(Self::SearchSubstring),
            "search_regex" => Ok(Self::SearchRegex),
            "page_up" => Ok(Self::PageUp),
            "page_down" => Ok(Self::PageDown),
            other => Err(Error::Backend(format!("unsupported command: {other}"))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppCommand {
    Quit,
    Refresh,
}

impl Action {
    pub fn as_app_command(&self) -> Option<AppCommand> {
        match self {
            Action::Quit => Some(AppCommand::Quit),
            Action::Refresh => Some(AppCommand::Refresh),
            _ => None,
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
            Action::StartSearch => "start_search",
            Action::ToggleSelection => "toggle_selection",
            Action::ToggleOnlySelected => "toggle_only_selected",
            Action::ToggleStarredSelected => "toggle_starred_selected",
            Action::ToggleHiddenSelected => "toggle_hidden_selected",
            Action::ToggleHiddenGroup => "toggle_hidden_group",
            Action::SearchFuzzy => "search_fuzzy",
            Action::SearchSubstring => "search_substring",
            Action::SearchRegex => "search_regex",
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
    use super::{Action, AppCommand, KeyBinding, KeyMap};
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
            Bind {
                key: "space".to_string(),
                command: "toggle_selection".to_string(),
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
        assert_eq!(
            "backspace".parse::<KeyBinding>().expect("backspace parses"),
            KeyBinding::Backspace
        );
        assert_eq!("space".parse::<KeyBinding>().expect("space parses"), KeyBinding::Char(' '));
        assert_eq!("page-up".parse::<KeyBinding>().expect("page-up parses"), KeyBinding::PageUp);
        assert_eq!(Action::from_command("open").expect("open parses"), Action::Open);
        assert_eq!(Action::from_command("refresh").expect("refresh parses"), Action::Refresh);
        assert_eq!(
            Action::from_command("toggle_selection").expect("toggle_selection parses"),
            Action::ToggleSelection
        );
        assert_eq!(
            Action::from_command("toggle_hidden_group").expect("toggle_hidden_group parses"),
            Action::ToggleHiddenGroup
        );
        assert_eq!(
            Action::from_command("search_regex").expect("search_regex parses"),
            Action::SearchRegex
        );
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

    #[test]
    fn maps_app_commands() {
        assert_eq!(Action::Quit.as_app_command(), Some(AppCommand::Quit));
        assert_eq!(Action::Refresh.as_app_command(), Some(AppCommand::Refresh));
        assert_eq!(Action::MoveDown.as_app_command(), None);
        assert_eq!(Action::ToggleSelection.as_app_command(), None);
        assert_eq!(Action::ToggleHiddenSelected.as_app_command(), None);
    }
}
