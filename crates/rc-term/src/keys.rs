//! Keyboard and mouse input turned into the bytes a terminal application expects (xterm).

use egui::{Key, Modifiers};

/// Terminal state that changes how keys are encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyMode {
    /// DECCKM: arrows/Home/End send `ESC O x` instead of `ESC [ x`.
    pub app_cursor: bool,
    /// Alt (Option) acts as Meta: `Alt+x` sends `ESC x`.
    pub alt_meta: bool,
}

/// xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl.
fn modifier_param(m: Modifiers) -> u8 {
    1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8
}

fn has_mods(m: Modifiers) -> bool {
    m.shift || m.alt || m.ctrl
}

/// `CSI 1;m x` with modifiers, else `CSI x` (or `SS3 x` in application cursor mode).
fn cursor_key(c: u8, m: Modifiers, app: bool) -> Vec<u8> {
    if has_mods(m) {
        format!("\x1b[1;{}{}", modifier_param(m), c as char).into_bytes()
    } else if app {
        vec![0x1b, b'O', c]
    } else {
        vec![0x1b, b'[', c]
    }
}

/// `CSI n ~` / `CSI n;m ~`.
fn tilde_key(n: u8, m: Modifiers) -> Vec<u8> {
    if has_mods(m) { format!("\x1b[{n};{}~", modifier_param(m)).into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
}

/// F1–F4: `SS3 P..S`, or `CSI 1;m P..S` with modifiers.
fn pf_key(c: u8, m: Modifiers) -> Vec<u8> {
    if has_mods(m) { format!("\x1b[1;{}{}", modifier_param(m), c as char).into_bytes() } else { vec![0x1b, b'O', c] }
}

/// The ASCII character a printable key produces (lowercase letters, unshifted).
fn key_char(key: Key, shift: bool) -> Option<char> {
    let name = key.name();
    let mut chars = name.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return match key {
            Key::Space => Some(' '),
            Key::OpenBracket => Some('['),
            Key::CloseBracket => Some(']'),
            Key::Backslash => Some('\\'),
            Key::Slash => Some('/'),
            Key::Minus => Some('-'),
            Key::Period => Some('.'),
            Key::Comma => Some(','),
            Key::Semicolon => Some(';'),
            Key::Equals => Some('='),
            Key::Quote => Some('\''),
            Key::Backtick => Some('`'),
            _ => None,
        };
    };
    if c.is_ascii_alphabetic() {
        Some(if shift { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() })
    } else if c.is_ascii_graphic() {
        Some(c)
    } else {
        None
    }
}

/// Control character for `Ctrl+key` (`Ctrl+C` → 0x03, `Ctrl+[` → ESC, `Ctrl+Space` → NUL…).
fn ctrl_byte(key: Key) -> Option<u8> {
    let c = key_char(key, false)?;
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        ' ' | '2' | '@' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '6' | '^' => 0x1e,
        '/' | '-' | '7' | '_' => 0x1f,
        '8' => 0x7f,
        _ => return None,
    })
}

/// Bytes for a key press, or `None` when the key is not for the terminal or its text comes
/// through `Event::Text` (plain printable keys). Command (⌘) shortcuts are never sent.
pub fn key_bytes(key: Key, m: Modifiers, mode: KeyMode) -> Option<Vec<u8>> {
    if m.mac_cmd {
        return None;
    }
    let esc_prefix = |mut v: Vec<u8>| {
        if m.alt && mode.alt_meta {
            v.insert(0, 0x1b);
        }
        v
    };
    let bytes = match key {
        Key::Enter => esc_prefix(vec![b'\r']),
        Key::Tab if m.shift => b"\x1b[Z".to_vec(),
        Key::Tab => vec![b'\t'],
        Key::Backspace if m.ctrl => esc_prefix(vec![0x08]),
        Key::Backspace => esc_prefix(vec![0x7f]),
        Key::Escape => vec![0x1b],
        Key::ArrowUp => cursor_key(b'A', m, mode.app_cursor),
        Key::ArrowDown => cursor_key(b'B', m, mode.app_cursor),
        Key::ArrowRight => cursor_key(b'C', m, mode.app_cursor),
        Key::ArrowLeft => cursor_key(b'D', m, mode.app_cursor),
        Key::Home => cursor_key(b'H', m, mode.app_cursor),
        Key::End => cursor_key(b'F', m, mode.app_cursor),
        Key::Insert => tilde_key(2, m),
        Key::Delete => tilde_key(3, m),
        Key::PageUp => tilde_key(5, m),
        Key::PageDown => tilde_key(6, m),
        Key::F1 => pf_key(b'P', m),
        Key::F2 => pf_key(b'Q', m),
        Key::F3 => pf_key(b'R', m),
        Key::F4 => pf_key(b'S', m),
        Key::F5 => tilde_key(15, m),
        Key::F6 => tilde_key(17, m),
        Key::F7 => tilde_key(18, m),
        Key::F8 => tilde_key(19, m),
        Key::F9 => tilde_key(20, m),
        Key::F10 => tilde_key(21, m),
        Key::F11 => tilde_key(23, m),
        Key::F12 => tilde_key(24, m),
        _ if m.ctrl => {
            // Ctrl+Alt+digit/punctuation is AltGr on Windows: leave it to the text.
            if m.alt && !key_char(key, false).is_some_and(|c| c.is_ascii_alphabetic()) {
                return None;
            }
            let b = ctrl_byte(key)?;
            if m.alt { vec![0x1b, b] } else { vec![b] }
        }
        _ if m.alt && mode.alt_meta => {
            let c = key_char(key, m.shift)?;
            vec![0x1b, c as u8]
        }
        _ => return None,
    };
    Some(bytes)
}

/// Text typed (or committed by an IME) as terminal input: newlines become carriage returns.
pub fn text_bytes(text: &str) -> Vec<u8> {
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

/// Pasted text: line breaks become `\r`, and it is wrapped in `ESC[200~ … ESC[201~` when the
/// application enabled bracketed paste (with the end marker stripped from the text).
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let clean = text.replace("\x1b[201~", "").replace("\x1b[200~", "");
        let mut v = b"\x1b[200~".to_vec();
        v.extend(clean.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
        v.extend(b"\x1b[201~");
        v
    } else {
        text_bytes(text)
    }
}

/// A mouse button as reported to the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    /// Motion without a button (any-event tracking).
    None,
    WheelUp,
    WheelDown,
}

impl MouseButton {
    fn code(self) -> u8 {
        match self {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::None => 3,
            MouseButton::WheelUp => 64,
            MouseButton::WheelDown => 65,
        }
    }
}

/// Encoding of mouse reports the application asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEncoding {
    /// `CSI M Cb Cx Cy` with one byte per value (coordinates up to 223).
    X10,
    /// Like X10 with UTF-8 encoded values (coordinates up to 2015).
    Utf8,
    /// `CSI < b ; x ; y M/m` (SGR 1006).
    Sgr,
}

/// A mouse report for a cell (`col`, `line` from 0). `pressed` is false for releases;
/// `motion` for movement with a button held (or `MouseButton::None`).
pub fn mouse_report(button: MouseButton, pressed: bool, motion: bool, col: usize, line: usize, m: Modifiers, enc: MouseEncoding) -> Option<Vec<u8>> {
    let mods = 4 * m.shift as u8 + 8 * m.alt as u8 + 16 * m.ctrl as u8;
    let motion_bit = if motion { 32 } else { 0 };
    match enc {
        MouseEncoding::Sgr => {
            let cb = button.code() + mods + motion_bit;
            let end = if pressed || motion { 'M' } else { 'm' };
            Some(format!("\x1b[<{cb};{};{}{end}", col + 1, line + 1).into_bytes())
        }
        MouseEncoding::X10 | MouseEncoding::Utf8 => {
            // Releases carry no button in the legacy protocols.
            let base = if !pressed && !motion && !matches!(button, MouseButton::WheelUp | MouseButton::WheelDown) { 3 } else { button.code() };
            let cb = 32 + base + mods + motion_bit;
            let mut v = b"\x1b[M".to_vec();
            v.push(cb);
            for value in [col + 1 + 32, line + 1 + 32] {
                if enc == MouseEncoding::X10 {
                    v.push(u8::try_from(value).ok()?);
                } else {
                    if value > 2047 {
                        return None;
                    }
                    let mut buf = [0u8; 4];
                    v.extend(char::from_u32(value as u32)?.encode_utf8(&mut buf).as_bytes());
                }
            }
            Some(v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers::NONE;
    fn mods(shift: bool, alt: bool, ctrl: bool) -> Modifiers {
        Modifiers { alt, ctrl, shift, mac_cmd: false, command: false }
    }
    fn k(key: Key, m: Modifiers) -> Option<Vec<u8>> {
        key_bytes(key, m, KeyMode::default())
    }

    #[test]
    fn named_keys() {
        assert_eq!(k(Key::Enter, NONE).unwrap(), b"\r");
        assert_eq!(k(Key::Backspace, NONE).unwrap(), b"\x7f");
        assert_eq!(k(Key::Tab, NONE).unwrap(), b"\t");
        assert_eq!(k(Key::Tab, Modifiers::SHIFT).unwrap(), b"\x1b[Z");
        assert_eq!(k(Key::Escape, NONE).unwrap(), b"\x1b");
        assert_eq!(k(Key::Delete, NONE).unwrap(), b"\x1b[3~");
        assert_eq!(k(Key::PageUp, NONE).unwrap(), b"\x1b[5~");
        assert_eq!(k(Key::PageDown, NONE).unwrap(), b"\x1b[6~");
        assert_eq!(k(Key::Insert, NONE).unwrap(), b"\x1b[2~");
    }

    #[test]
    fn arrows_and_application_cursor_mode() {
        assert_eq!(k(Key::ArrowUp, NONE).unwrap(), b"\x1b[A");
        assert_eq!(k(Key::ArrowLeft, NONE).unwrap(), b"\x1b[D");
        assert_eq!(k(Key::Home, NONE).unwrap(), b"\x1b[H");
        assert_eq!(k(Key::End, NONE).unwrap(), b"\x1b[F");
        let app = KeyMode { app_cursor: true, alt_meta: false };
        assert_eq!(key_bytes(Key::ArrowUp, NONE, app).unwrap(), b"\x1bOA");
        assert_eq!(key_bytes(Key::Home, NONE, app).unwrap(), b"\x1bOH");
        // Modifiers always use the CSI form.
        assert_eq!(key_bytes(Key::ArrowRight, Modifiers::CTRL, app).unwrap(), b"\x1b[1;5C");
        assert_eq!(k(Key::ArrowLeft, mods(true, true, false)).unwrap(), b"\x1b[1;4D");
        assert_eq!(k(Key::PageUp, Modifiers::CTRL).unwrap(), b"\x1b[5;5~");
    }

    #[test]
    fn function_keys() {
        assert_eq!(k(Key::F1, NONE).unwrap(), b"\x1bOP");
        assert_eq!(k(Key::F4, NONE).unwrap(), b"\x1bOS");
        assert_eq!(k(Key::F5, NONE).unwrap(), b"\x1b[15~");
        assert_eq!(k(Key::F10, NONE).unwrap(), b"\x1b[21~");
        assert_eq!(k(Key::F12, NONE).unwrap(), b"\x1b[24~");
        assert_eq!(k(Key::F2, Modifiers::SHIFT).unwrap(), b"\x1b[1;2Q");
        assert_eq!(k(Key::F12, Modifiers::CTRL).unwrap(), b"\x1b[24;5~");
    }

    #[test]
    fn ctrl_letters() {
        assert_eq!(k(Key::C, Modifiers::CTRL).unwrap(), [0x03]);
        assert_eq!(k(Key::A, Modifiers::CTRL).unwrap(), [0x01]);
        assert_eq!(k(Key::Z, Modifiers::CTRL).unwrap(), [0x1a]);
        assert_eq!(k(Key::D, mods(true, false, true)).unwrap(), [0x04]);
        assert_eq!(k(Key::Space, Modifiers::CTRL).unwrap(), [0x00]);
        assert_eq!(k(Key::OpenBracket, Modifiers::CTRL).unwrap(), [0x1b]);
        assert_eq!(k(Key::Backslash, Modifiers::CTRL).unwrap(), [0x1c]);
        assert_eq!(k(Key::C, mods(false, true, true)).unwrap(), [0x1b, 0x03]);
        // AltGr (Ctrl+Alt) with digits types text instead.
        assert_eq!(k(Key::Num2, mods(false, true, true)), None);
    }

    #[test]
    fn printable_keys_come_as_text() {
        assert_eq!(k(Key::A, NONE), None);
        assert_eq!(k(Key::Space, NONE), None);
        assert_eq!(k(Key::Num1, Modifiers::SHIFT), None);
        // ⌘ shortcuts belong to the IDE.
        assert_eq!(k(Key::C, Modifiers::MAC_CMD), None);
        assert_eq!(k(Key::ArrowUp, Modifiers::MAC_CMD), None);
    }

    #[test]
    fn alt_as_meta() {
        let meta = KeyMode { app_cursor: false, alt_meta: true };
        assert_eq!(key_bytes(Key::B, Modifiers::ALT, meta).unwrap(), b"\x1bb");
        assert_eq!(key_bytes(Key::F, mods(true, true, false), meta).unwrap(), b"\x1bF");
        assert_eq!(key_bytes(Key::Period, Modifiers::ALT, meta).unwrap(), b"\x1b.");
        assert_eq!(key_bytes(Key::Backspace, Modifiers::ALT, meta).unwrap(), b"\x1b\x7f");
        // Without Meta, Option+key types its character (macOS layouts: Option+2 is @).
        assert_eq!(k(Key::B, Modifiers::ALT), None);
    }

    #[test]
    fn text_and_paste() {
        assert_eq!(text_bytes("a\nb"), b"a\rb");
        assert_eq!(paste_bytes("ls\r\npwd\n", false), b"ls\rpwd\r");
        assert_eq!(paste_bytes("x\ny", true), b"\x1b[200~x\ry\x1b[201~");
        assert_eq!(paste_bytes("a\x1b[201~b", true), b"\x1b[200~ab\x1b[201~");
    }

    #[test]
    fn mouse_reports() {
        let sgr = MouseEncoding::Sgr;
        assert_eq!(mouse_report(MouseButton::Left, true, false, 0, 0, NONE, sgr).unwrap(), b"\x1b[<0;1;1M");
        assert_eq!(mouse_report(MouseButton::Left, false, false, 4, 2, NONE, sgr).unwrap(), b"\x1b[<0;5;3m");
        assert_eq!(mouse_report(MouseButton::Left, true, true, 1, 1, NONE, sgr).unwrap(), b"\x1b[<32;2;2M");
        assert_eq!(mouse_report(MouseButton::WheelUp, true, false, 0, 0, Modifiers::CTRL, sgr).unwrap(), b"\x1b[<80;1;1M");
        assert_eq!(mouse_report(MouseButton::Right, true, false, 9, 4, NONE, MouseEncoding::X10).unwrap(), [0x1b, b'[', b'M', 34, 42, 37]);
        assert_eq!(mouse_report(MouseButton::Left, false, false, 0, 0, NONE, MouseEncoding::X10).unwrap(), [0x1b, b'[', b'M', 35, 33, 33]);
        assert_eq!(mouse_report(MouseButton::Left, true, false, 300, 0, NONE, MouseEncoding::X10), None);
        assert_eq!(mouse_report(MouseButton::Left, true, false, 300, 0, NONE, MouseEncoding::Utf8).unwrap().len(), 3 + 1 + 2 + 1);
    }
}
