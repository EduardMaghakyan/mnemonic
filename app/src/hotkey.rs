use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

pub fn parse_hotkey(combo: &str) -> Result<Shortcut, String> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err("empty hotkey".into());
    }
    let last_idx = parts.len() - 1;
    let mut mods = Modifiers::empty();
    for part in &parts[..last_idx] {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" | "option" | "opt" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "cmd" | "command" | "super" | "meta" => mods |= Modifiers::SUPER,
            other => return Err(format!("unknown modifier: {other}")),
        }
    }
    let code = parse_code(&parts[last_idx].to_ascii_lowercase())?;
    let mods_opt = if mods.is_empty() { None } else { Some(mods) };
    Ok(Shortcut::new(mods_opt, code))
}

fn parse_code(name: &str) -> Result<Code, String> {
    if let Some(c) = letter_code(name) {
        return Ok(c);
    }
    if let Some(c) = digit_code(name) {
        return Ok(c);
    }
    if let Some(c) = function_code(name) {
        return Ok(c);
    }
    match name {
        "space" => Ok(Code::Space),
        "enter" | "return" => Ok(Code::Enter),
        "esc" | "escape" => Ok(Code::Escape),
        "tab" => Ok(Code::Tab),
        "backspace" => Ok(Code::Backspace),
        "delete" | "del" => Ok(Code::Delete),
        "left" => Ok(Code::ArrowLeft),
        "right" => Ok(Code::ArrowRight),
        "up" => Ok(Code::ArrowUp),
        "down" => Ok(Code::ArrowDown),
        "home" => Ok(Code::Home),
        "end" => Ok(Code::End),
        "pageup" => Ok(Code::PageUp),
        "pagedown" => Ok(Code::PageDown),
        "comma" => Ok(Code::Comma),
        "period" | "dot" => Ok(Code::Period),
        "slash" => Ok(Code::Slash),
        "semicolon" => Ok(Code::Semicolon),
        "minus" => Ok(Code::Minus),
        "equal" | "equals" => Ok(Code::Equal),
        "backslash" => Ok(Code::Backslash),
        "backquote" | "backtick" | "grave" => Ok(Code::Backquote),
        other => Err(format!("unknown key: {other}")),
    }
}

fn letter_code(name: &str) -> Option<Code> {
    let bytes = name.as_bytes();
    if bytes.len() != 1 {
        return None;
    }
    let c = bytes[0];
    if !c.is_ascii_alphabetic() {
        return None;
    }
    let upper = c.to_ascii_uppercase();
    match upper {
        b'A' => Some(Code::KeyA), b'B' => Some(Code::KeyB), b'C' => Some(Code::KeyC),
        b'D' => Some(Code::KeyD), b'E' => Some(Code::KeyE), b'F' => Some(Code::KeyF),
        b'G' => Some(Code::KeyG), b'H' => Some(Code::KeyH), b'I' => Some(Code::KeyI),
        b'J' => Some(Code::KeyJ), b'K' => Some(Code::KeyK), b'L' => Some(Code::KeyL),
        b'M' => Some(Code::KeyM), b'N' => Some(Code::KeyN), b'O' => Some(Code::KeyO),
        b'P' => Some(Code::KeyP), b'Q' => Some(Code::KeyQ), b'R' => Some(Code::KeyR),
        b'S' => Some(Code::KeyS), b'T' => Some(Code::KeyT), b'U' => Some(Code::KeyU),
        b'V' => Some(Code::KeyV), b'W' => Some(Code::KeyW), b'X' => Some(Code::KeyX),
        b'Y' => Some(Code::KeyY), b'Z' => Some(Code::KeyZ),
        _ => None,
    }
}

fn digit_code(name: &str) -> Option<Code> {
    if name.len() != 1 {
        return None;
    }
    match name.as_bytes()[0] {
        b'0' => Some(Code::Digit0), b'1' => Some(Code::Digit1), b'2' => Some(Code::Digit2),
        b'3' => Some(Code::Digit3), b'4' => Some(Code::Digit4), b'5' => Some(Code::Digit5),
        b'6' => Some(Code::Digit6), b'7' => Some(Code::Digit7), b'8' => Some(Code::Digit8),
        b'9' => Some(Code::Digit9),
        _ => None,
    }
}

fn function_code(name: &str) -> Option<Code> {
    let n = name.strip_prefix('f')?.parse::<u8>().ok()?;
    match n {
        1 => Some(Code::F1), 2 => Some(Code::F2), 3 => Some(Code::F3), 4 => Some(Code::F4),
        5 => Some(Code::F5), 6 => Some(Code::F6), 7 => Some(Code::F7), 8 => Some(Code::F8),
        9 => Some(Code::F9), 10 => Some(Code::F10), 11 => Some(Code::F11), 12 => Some(Code::F12),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_combo() {
        let s = parse_hotkey("ctrl+alt+space").unwrap();
        assert!(s.mods.contains(Modifiers::CONTROL));
        assert!(s.mods.contains(Modifiers::ALT));
        assert!(!s.mods.contains(Modifiers::SHIFT));
        assert_eq!(s.key, Code::Space);
    }

    #[test]
    fn parses_with_titlecase_and_aliases() {
        let s = parse_hotkey("Cmd+Shift+R").unwrap();
        assert!(s.mods.contains(Modifiers::SUPER));
        assert!(s.mods.contains(Modifiers::SHIFT));
        assert_eq!(s.key, Code::KeyR);
    }

    #[test]
    fn parses_function_key() {
        let s = parse_hotkey("alt+f12").unwrap();
        assert!(s.mods.contains(Modifiers::ALT));
        assert_eq!(s.key, Code::F12);
    }

    #[test]
    fn rejects_unknown_modifier() {
        assert!(parse_hotkey("hyper+a").is_err());
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(parse_hotkey("ctrl+banana").is_err());
    }

    #[test]
    fn allows_no_modifier() {
        let s = parse_hotkey("space").unwrap();
        assert!(s.mods.is_empty());
        assert_eq!(s.key, Code::Space);
    }
}
