use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ModifierKey {
    Alt,
    Control,
    Shift,
    Win,
}

impl ModifierKey {
    const fn order(self) -> u8 {
        match self {
            Self::Control => 0,
            Self::Alt => 1,
            Self::Shift => 2,
            Self::Win => 3,
        }
    }
}

impl fmt::Display for ModifierKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Alt => "Alt",
            Self::Control => "Ctrl",
            Self::Shift => "Shift",
            Self::Win => "Win",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VirtualKey {
    Letter(char),
    Digit(u8),
    Function(u8),
    Delete,
}

impl VirtualKey {
    pub fn from_name(value: &str) -> Option<Self> {
        let upper = value.trim().to_ascii_uppercase();
        if upper.len() == 1 {
            let character = upper.chars().next()?;
            if character.is_ascii_alphabetic() {
                return Some(Self::Letter(character));
            }
            if character.is_ascii_digit() {
                return Some(Self::Digit(character as u8 - b'0'));
            }
        }
        if upper == "DELETE" || upper == "DEL" {
            return Some(Self::Delete);
        }
        let function = upper.strip_prefix('F')?.parse::<u8>().ok()?;
        (1..=24)
            .contains(&function)
            .then_some(Self::Function(function))
    }

    pub const fn win32_code(self) -> u32 {
        match self {
            Self::Letter(character) => character as u32,
            Self::Digit(digit) => b'0' as u32 + digit as u32,
            Self::Function(function) => 0x70 + function as u32 - 1,
            Self::Delete => 0x2E,
        }
    }
}

impl fmt::Display for VirtualKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Letter(character) => write!(formatter, "{character}"),
            Self::Digit(digit) => write!(formatter, "{digit}"),
            Self::Function(function) => write!(formatter, "F{function}"),
            Self::Delete => formatter.write_str("Delete"),
        }
    }
}

impl Serialize for VirtualKey {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for VirtualKey {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_name(&value)
            .ok_or_else(|| serde::de::Error::custom(format!("unsupported virtual key '{value}'")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HotkeyBinding {
    pub enabled: bool,
    pub modifiers: Vec<ModifierKey>,
    pub key: VirtualKey,
}

impl HotkeyBinding {
    pub fn new(enabled: bool, mut modifiers: Vec<ModifierKey>, key: VirtualKey) -> Self {
        modifiers.sort_by_key(|modifier| modifier.order());
        modifiers.dedup();
        Self {
            enabled,
            modifiers,
            key,
        }
    }

    pub fn validate(&self) -> std::result::Result<(), HotkeyValidationError> {
        if self.modifiers.is_empty() {
            return Err(HotkeyValidationError::MissingModifier);
        }
        if self.is_dangerous() {
            return Err(HotkeyValidationError::DangerousCombination(
                self.to_string(),
            ));
        }
        Ok(())
    }

    pub fn has_modifier(&self, modifier: ModifierKey) -> bool {
        self.modifiers.contains(&modifier)
    }

    fn is_dangerous(&self) -> bool {
        let ctrl = self.has_modifier(ModifierKey::Control);
        let alt = self.has_modifier(ModifierKey::Alt);
        let win = self.has_modifier(ModifierKey::Win);
        match self.key {
            VirtualKey::Function(4) => alt,
            VirtualKey::Letter(character) if ctrl => {
                matches!(character, 'A' | 'C' | 'V' | 'X' | 'Z')
            }
            VirtualKey::Letter('L') => win,
            VirtualKey::Delete => ctrl && alt,
            _ => false,
        }
    }
}

impl fmt::Display for HotkeyBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for modifier in &self.modifiers {
            write!(formatter, "{modifier}+")?;
        }
        write!(formatter, "{}", self.key)
    }
}

impl FromStr for HotkeyBinding {
    type Err = HotkeyValidationError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let parts = value
            .split('+')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let (key_name, modifier_names) = parts
            .split_last()
            .ok_or(HotkeyValidationError::InvalidSyntax)?;
        let key = VirtualKey::from_name(key_name)
            .ok_or_else(|| HotkeyValidationError::UnsupportedKey((*key_name).to_owned()))?;
        let mut modifiers = Vec::with_capacity(modifier_names.len());
        for name in modifier_names {
            let modifier = match name.to_ascii_lowercase().as_str() {
                "alt" => ModifierKey::Alt,
                "ctrl" | "control" => ModifierKey::Control,
                "shift" => ModifierKey::Shift,
                "win" | "windows" => ModifierKey::Win,
                _ => return Err(HotkeyValidationError::UnknownModifier((*name).to_owned())),
            };
            modifiers.push(modifier);
        }
        let binding = Self::new(true, modifiers, key);
        binding.validate()?;
        Ok(binding)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HotkeyValidationError {
    #[error("hotkey syntax is invalid")]
    InvalidSyntax,

    #[error("unsupported key '{0}'")]
    UnsupportedKey(String),

    #[error("unknown modifier '{0}'")]
    UnknownModifier(String),

    #[error("at least one modifier key is required")]
    MissingModifier,

    #[error("dangerous system combination '{0}' is not allowed")]
    DangerousCombination(String),

    #[error("hotkey duplicates '{0}'")]
    Duplicate(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_canonicalizes_hotkey() {
        let binding = "shift + alt + q"
            .parse::<HotkeyBinding>()
            .expect("valid hotkey");
        assert_eq!(binding.to_string(), "Alt+Shift+Q");
        assert_eq!(binding.key.win32_code(), u32::from(b'Q'));
    }

    #[test]
    fn rejects_key_without_modifier() {
        assert_eq!(
            "Q".parse::<HotkeyBinding>(),
            Err(HotkeyValidationError::MissingModifier)
        );
    }

    #[test]
    fn rejects_dangerous_combinations() {
        for value in ["Alt+F4", "Ctrl+C", "Ctrl+V", "Win+L", "Ctrl+Alt+Delete"] {
            assert!(
                matches!(
                    value.parse::<HotkeyBinding>(),
                    Err(HotkeyValidationError::DangerousCombination(_))
                ),
                "{value} should be rejected"
            );
        }
    }

    #[test]
    fn serde_uses_readable_key_names() {
        let binding = "Alt+F12".parse::<HotkeyBinding>().expect("valid hotkey");
        let json = serde_json::to_string(&binding).expect("serialize hotkey");
        assert!(json.contains("\"F12\""));
        let decoded: HotkeyBinding = serde_json::from_str(&json).expect("deserialize hotkey");
        assert_eq!(decoded, binding);
    }
}
