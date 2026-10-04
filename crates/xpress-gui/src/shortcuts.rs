//! The app's global shortcuts, which can be changed in Preferences. They're
//! stored as `global_hotkey` strings (`control+super+KeyV`); an empty string
//! turns a shortcut off.

use eframe::egui;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Clipboard,
    Show,
    History,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Clipboard, Action::Show, Action::History];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Clipboard => "Optimise clipboard",
            Action::Show => "Open xpress",
            Action::History => "Clipboard history",
        }
    }

    pub fn default_shortcut(self) -> &'static str {
        match self {
            Action::Clipboard => "shift+super+KeyO",
            Action::Show => "shift+super+KeyX",
            Action::History => "control+super+KeyV",
        }
    }
}

/// A stored shortcut; `None` when it's off (or unreadable).
pub fn parse(s: &str) -> Option<HotKey> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        s.parse().ok()
    }
}

/// How macOS writes it: ⌃⌥⇧⌘ then the key, e.g. "⌃⌘V".
pub fn display(hotkey: &HotKey) -> String {
    let mut out = String::new();
    for (m, sign) in [
        (Modifiers::CONTROL, "⌃"),
        (Modifiers::ALT, "⌥"),
        (Modifiers::SHIFT, "⇧"),
        (Modifiers::SUPER, "⌘"),
    ] {
        if hotkey.mods.contains(m) {
            out.push_str(sign);
        }
    }
    out.push_str(&key_label(hotkey.key));
    out
}

/// A stored shortcut for display ("Off" when there's none).
pub fn display_str(s: &str) -> String {
    parse(s).map_or_else(|| "Off".into(), |h| display(&h))
}

fn key_label(code: Code) -> String {
    let name = code.to_string();
    if let Some(letter) = name.strip_prefix("Key") {
        return letter.to_string();
    }
    if let Some(digit) = name.strip_prefix("Digit") {
        return digit.to_string();
    }
    let symbol = match code {
        Code::Space => "Space",
        Code::Enter => "↩",
        Code::Tab => "⇥",
        Code::ArrowLeft => "←",
        Code::ArrowRight => "→",
        Code::ArrowUp => "↑",
        Code::ArrowDown => "↓",
        Code::Backquote => "`",
        Code::Minus => "-",
        Code::Equal => "=",
        Code::BracketLeft => "[",
        Code::BracketRight => "]",
        Code::Backslash => "\\",
        Code::Semicolon => ";",
        Code::Quote => "'",
        Code::Comma => ",",
        Code::Period => ".",
        Code::Slash => "/",
        _ => return name,
    };
    symbol.to_string()
}

/// Why a key press can't be a shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Without ⌘, ⌃ or ⌥ it would swallow normal typing.
    NeedsModifier,
    /// Not a key we can listen for globally.
    UnsupportedKey,
}

impl Refused {
    pub fn message(&self) -> &'static str {
        match self {
            Refused::NeedsModifier => "Use ⌘, ⌃ or ⌥ with the key.",
            Refused::UnsupportedKey => "That key can't be a shortcut.",
        }
    }
}

/// A shortcut from a key pressed in the app (by its position, as global
/// shortcuts work).
pub fn from_key(key: egui::Key, mods: egui::Modifiers) -> Result<HotKey, Refused> {
    let code = code_for(key).ok_or(Refused::UnsupportedKey)?;
    let mut m = Modifiers::empty();
    if mods.mac_cmd || (cfg!(not(target_os = "macos")) && mods.command && !mods.ctrl) {
        m |= Modifiers::SUPER;
    }
    if mods.ctrl {
        m |= Modifiers::CONTROL;
    }
    if mods.alt {
        m |= Modifiers::ALT;
    }
    if mods.shift {
        m |= Modifiers::SHIFT;
    }
    if !m.intersects(Modifiers::SUPER | Modifiers::CONTROL | Modifiers::ALT) {
        return Err(Refused::NeedsModifier);
    }
    Ok(HotKey::new(Some(m), code))
}

fn code_for(key: egui::Key) -> Option<Code> {
    use egui::Key as K;
    let name = key.name();
    if name.len() == 1 {
        let c = name.chars().next()?;
        if c.is_ascii_alphabetic() {
            return format!("Key{}", c.to_ascii_uppercase()).parse().ok();
        }
        if c.is_ascii_digit() {
            return format!("Digit{c}").parse().ok();
        }
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
        if (1..=20).contains(&n) {
            return format!("F{n}").parse().ok();
        }
    }
    Some(match key {
        K::Space => Code::Space,
        K::Enter => Code::Enter,
        K::Tab => Code::Tab,
        K::ArrowLeft => Code::ArrowLeft,
        K::ArrowRight => Code::ArrowRight,
        K::ArrowUp => Code::ArrowUp,
        K::ArrowDown => Code::ArrowDown,
        K::Backtick => Code::Backquote,
        K::Minus => Code::Minus,
        K::Equals => Code::Equal,
        K::OpenBracket => Code::BracketLeft,
        K::CloseBracket => Code::BracketRight,
        K::Backslash => Code::Backslash,
        K::Semicolon => Code::Semicolon,
        K::Quote => Code::Quote,
        K::Comma => Code::Comma,
        K::Period => Code::Period,
        K::Slash => Code::Slash,
        _ => return None,
    })
}

/// The stored form of a shortcut.
pub fn store(hotkey: &HotKey) -> String {
    hotkey.into_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(cmd: bool, ctrl: bool, alt: bool, shift: bool) -> egui::Modifiers {
        egui::Modifiers {
            alt,
            ctrl,
            shift,
            mac_cmd: cmd,
            command: cmd,
        }
    }

    #[test]
    fn defaults_parse_and_display_like_macos() {
        let shown: Vec<String> = Action::ALL
            .iter()
            .map(|a| display_str(a.default_shortcut()))
            .collect();
        assert_eq!(shown, ["⇧⌘O", "⇧⌘X", "⌃⌘V"]);
        assert_eq!(display_str(""), "Off");
        assert_eq!(display_str("nonsense+++"), "Off");
    }

    #[test]
    fn key_presses_become_shortcuts() {
        let hk = from_key(egui::Key::V, mods(true, true, false, false)).unwrap();
        assert_eq!(display(&hk), "⌃⌘V");
        assert_eq!(parse(&store(&hk)), Some(hk), "round trip");
        let hk = from_key(egui::Key::Num2, mods(false, false, true, true)).unwrap();
        assert_eq!(display(&hk), "⌥⇧2");
        let hk = from_key(egui::Key::F5, mods(true, false, false, false)).unwrap();
        assert_eq!(display(&hk), "⌘F5");
        let hk = from_key(egui::Key::Space, mods(false, true, true, false)).unwrap();
        assert_eq!(display(&hk), "⌃⌥Space");
        let hk = from_key(egui::Key::Period, mods(true, false, false, true)).unwrap();
        assert_eq!(display(&hk), "⇧⌘.");
    }

    #[test]
    fn plain_keys_are_refused() {
        assert_eq!(
            from_key(egui::Key::V, mods(false, false, false, false)),
            Err(Refused::NeedsModifier)
        );
        assert_eq!(
            from_key(egui::Key::V, mods(false, false, false, true)),
            Err(Refused::NeedsModifier),
            "⇧ alone just types a capital"
        );
        assert_eq!(
            from_key(egui::Key::Escape, mods(true, false, false, false)),
            Err(Refused::UnsupportedKey)
        );
    }
}
