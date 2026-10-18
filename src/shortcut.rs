//! Global shortcuts: the actions, key combinations as Carbon registers
//! them (virtual key code and modifier bits), and their names as macOS menus
//! show them. Registered by `crate::hotkey`.

pub const KEY_R: u32 = 15; // kVK_ANSI_R
pub const KEY_S: u32 = 1; // kVK_ANSI_S
pub const CMD: u32 = 1 << 8; // cmdKey
pub const SHIFT: u32 = 1 << 9; // shiftKey
pub const OPTION: u32 = 1 << 11; // optionKey
pub const CONTROL: u32 = 1 << 12; // controlKey

/// What a global shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Record,
    Pause,
    SaveReplay,
    Stream,
    NextScene,
    PrevScene,
}

impl Action {
    pub const ALL: [Action; 6] = [Action::Record, Action::Pause, Action::SaveReplay, Action::Stream, Action::NextScene, Action::PrevScene];

    pub fn label(self) -> &'static str {
        match self {
            Action::Record => "Start / stop recording",
            Action::Pause => "Pause / resume",
            Action::SaveReplay => "Save replay",
            Action::Stream => "Go live / stop stream",
            Action::NextScene => "Next scene",
            Action::PrevScene => "Previous scene",
        }
    }

    /// Registration ID (stable, nonzero).
    pub fn id(self) -> u32 {
        self as u32 + 1
    }

    pub fn default_combo(self) -> Option<Combo> {
        match self {
            Action::Record => Some(Combo { key: KEY_R, mods: CMD | SHIFT }),
            Action::SaveReplay => Some(Combo { key: KEY_S, mods: CMD | SHIFT }),
            _ => None,
        }
    }
}

/// A key with modifiers (Carbon virtual key code and modifier bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Combo {
    pub key: u32,
    pub mods: u32,
}

/// Virtual key codes (kVK_*), their label and egui's name for the key.
const KEYS: &[(u32, &str, &str)] = &[
    (0x00, "A", "A"), (0x0B, "B", "B"), (0x08, "C", "C"), (0x02, "D", "D"), (0x0E, "E", "E"), (0x03, "F", "F"),
    (0x05, "G", "G"), (0x04, "H", "H"), (0x22, "I", "I"), (0x26, "J", "J"), (0x28, "K", "K"), (0x25, "L", "L"),
    (0x2E, "M", "M"), (0x2D, "N", "N"), (0x1F, "O", "O"), (0x23, "P", "P"), (0x0C, "Q", "Q"), (0x0F, "R", "R"),
    (0x01, "S", "S"), (0x11, "T", "T"), (0x20, "U", "U"), (0x09, "V", "V"), (0x0D, "W", "W"), (0x07, "X", "X"),
    (0x10, "Y", "Y"), (0x06, "Z", "Z"),
    (0x1D, "0", "0"), (0x12, "1", "1"), (0x13, "2", "2"), (0x14, "3", "3"), (0x15, "4", "4"), (0x17, "5", "5"),
    (0x16, "6", "6"), (0x1A, "7", "7"), (0x1C, "8", "8"), (0x19, "9", "9"),
    (0x7A, "F1", "F1"), (0x78, "F2", "F2"), (0x63, "F3", "F3"), (0x76, "F4", "F4"), (0x60, "F5", "F5"), (0x61, "F6", "F6"),
    (0x62, "F7", "F7"), (0x64, "F8", "F8"), (0x65, "F9", "F9"), (0x6D, "F10", "F10"), (0x67, "F11", "F11"), (0x6F, "F12", "F12"),
    (0x69, "F13", "F13"), (0x6B, "F14", "F14"), (0x71, "F15", "F15"), (0x6A, "F16", "F16"), (0x40, "F17", "F17"),
    (0x4F, "F18", "F18"), (0x50, "F19", "F19"), (0x5A, "F20", "F20"),
    (0x31, "Space", "Space"), (0x24, "↩", "Enter"), (0x30, "⇥", "Tab"), (0x33, "⌫", "Backspace"), (0x75, "⌦", "Delete"),
    (0x73, "↖", "Home"), (0x77, "↘", "End"), (0x74, "⇞", "PageUp"), (0x79, "⇟", "PageDown"),
    (0x7B, "←", "Left"), (0x7C, "→", "Right"), (0x7E, "↑", "Up"), (0x7D, "↓", "Down"),
    (0x1B, "-", "Minus"), (0x18, "=", "Equals"), (0x21, "[", "OpenBracket"), (0x1E, "]", "CloseBracket"),
    (0x29, ";", "Semicolon"), (0x27, "'", "Quote"), (0x2B, ",", "Comma"), (0x2F, ".", "Period"), (0x2C, "/", "Slash"),
    (0x2A, "\\", "Backslash"), (0x32, "`", "Backtick"),
];

fn is_function_key(label: &str) -> bool {
    label.len() >= 2 && label.starts_with('F') && label[1..].chars().all(|c| c.is_ascii_digit())
}

impl Combo {
    /// As macOS menus show it: ⌃⌥⇧⌘ then the key.
    pub fn label(&self) -> String {
        let mut s = String::new();
        for (bit, sym) in [(CONTROL, "⌃"), (OPTION, "⌥"), (SHIFT, "⇧"), (CMD, "⌘")] {
            if self.mods & bit != 0 {
                s.push_str(sym);
            }
        }
        let key = KEYS.iter().find(|k| k.0 == self.key).map(|k| k.1.to_string()).unwrap_or_else(|| format!("#{}", self.key));
        s + &key
    }

    /// The combo for a key egui names `name` with these modifiers. A global
    /// shortcut needs ⌘, ⌃ or ⌥ (⇧ alone would eat typed capitals), except
    /// on function keys.
    pub fn from_key(name: &str, ctrl: bool, alt: bool, shift: bool, cmd: bool) -> Option<Combo> {
        let &(key, label, _) = KEYS.iter().find(|k| k.2 == name)?;
        if !(ctrl || alt || cmd || is_function_key(label)) {
            return None;
        }
        let mods = [(ctrl, CONTROL), (alt, OPTION), (shift, SHIFT), (cmd, CMD)].iter().filter(|m| m.0).fold(0, |a, m| a | m.1);
        Some(Combo { key, mods })
    }

    /// The menu key equivalent (a lowercase letter or digit, or a function
    /// key's private-use character), if the key has one.
    pub fn menu_key(&self) -> Option<String> {
        let label = KEYS.iter().find(|k| k.0 == self.key)?.1;
        if is_function_key(label) {
            // NSF1FunctionKey is U+F704.
            let n: u32 = label[1..].parse().ok()?;
            return char::from_u32(0xF703 + n).map(String::from);
        }
        (label.len() == 1 && label.chars().all(|c| c.is_ascii_alphanumeric())).then(|| label.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos() {
        let r = Action::Record.default_combo().unwrap();
        assert_eq!(r.label(), "⇧⌘R");
        assert_eq!(r.menu_key().as_deref(), Some("r"));
        assert_eq!(Combo::from_key("R", false, false, true, true), Some(r));
        // Shift alone is not enough; a function key is.
        assert_eq!(Combo::from_key("R", false, false, true, false), None);
        let f9 = Combo::from_key("F9", false, false, false, false).unwrap();
        assert_eq!(f9.label(), "F9");
        assert_eq!(f9.menu_key(), char::from_u32(0xF70C).map(String::from));
        assert_eq!(Combo::from_key("Left", true, true, false, false).unwrap().label(), "⌃⌥←");
        assert_eq!(Combo::from_key("Copy", false, false, false, true), None);
        // Every key code appears once.
        let mut codes: Vec<u32> = KEYS.iter().map(|k| k.0).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), KEYS.len());
    }

    #[test]
    fn actions_serialize_as_names() {
        let m: std::collections::BTreeMap<Action, Option<Combo>> = [(Action::NextScene, None)].into();
        assert_eq!(serde_json::to_string(&m).unwrap(), r#"{"next-scene":null}"#);
        assert!(Action::ALL.iter().all(|a| a.id() > 0));
    }
}
