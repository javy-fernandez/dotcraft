//! Terminal colours: a JetBrains-like ANSI palette for Islands Dark and Islands Light, the
//! 256-colour cube and grays, and truecolor. Default foreground/background come from the
//! editor scheme so the terminal looks like an editor island.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use egui::Color32;

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// The 16 ANSI colours of the dark terminal (IntelliJ New UI dark console).
const ANSI_DARK: [Color32; 16] = [
    hex(0x45484E), // black: still visible on the island background
    hex(0xF0524F), // red
    hex(0x5C962C), // green
    hex(0xA68A0D), // yellow
    hex(0x3993D4), // blue
    hex(0xA771BF), // magenta
    hex(0x00A3A3), // cyan
    hex(0x9FA2A8), // white
    hex(0x595959), // bright black
    hex(0xFF4050), // bright red
    hex(0x4FC414), // bright green
    hex(0xE5BF00), // bright yellow
    hex(0x1FB0FF), // bright blue
    hex(0xED7EED), // bright magenta
    hex(0x00E5E5), // bright cyan
    hex(0xFFFFFF), // bright white
];

/// The 16 ANSI colours of the light terminal (IntelliJ New UI light console).
const ANSI_LIGHT: [Color32; 16] = [
    hex(0x000000),
    hex(0xC91B00),
    hex(0x1F8C2D),
    hex(0x8F6D00),
    hex(0x1E5BD6),
    hex(0x9C3DB8),
    hex(0x00818A),
    hex(0x8C8C8C),
    hex(0x5E5E5E),
    hex(0xE5534B),
    hex(0x3DA03A),
    hex(0xB38600),
    hex(0x3B82F6),
    hex(0xC058D6),
    hex(0x00A3A8),
    hex(0xBDBDBD),
];

/// Colours of one theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TermColors {
    pub ansi: [Color32; 16],
    pub fg: Color32,
    pub bg: Color32,
    pub cursor: Color32,
    pub selection: Color32,
    pub link: Color32,
}

impl TermColors {
    pub fn for_theme(dark: bool) -> Self {
        let s = rc_kit::theme::scheme(dark);
        let p = rc_kit::theme::palette(dark);
        TermColors {
            ansi: if dark { ANSI_DARK } else { ANSI_LIGHT },
            fg: s.text,
            bg: p.editor_bg,
            cursor: s.caret,
            selection: p.editor_selection,
            link: s.hyperlink,
        }
    }

    /// Colour `i` of the 256-colour palette (16 ANSI, 6×6×6 cube, 24 grays).
    pub fn indexed(&self, i: u8) -> Color32 {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                Color32::from_rgb(v, v, v)
            }
        }
    }

    /// Colour for palette slot `index` (0–255 and alacritty's named slots ≥ 256).
    pub fn slot(&self, index: usize) -> Color32 {
        match index {
            0..=255 => self.indexed(index as u8),
            i if i == NamedColor::Foreground as usize || i == NamedColor::BrightForeground as usize => self.fg,
            i if i == NamedColor::Background as usize => self.bg,
            i if i == NamedColor::Cursor as usize => self.cursor,
            i if i == NamedColor::DimForeground as usize => dim(self.fg, self.bg),
            i if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => {
                dim(self.ansi[i - NamedColor::DimBlack as usize], self.bg)
            }
            _ => self.fg,
        }
    }

    /// Resolves a cell colour, honouring the palette changes the application made (OSC 4/10/11).
    pub fn resolve(&self, color: Color, overrides: &Colors) -> Color32 {
        match color {
            Color::Spec(rgb) => to_color32(rgb),
            Color::Indexed(i) => overrides[i as usize].map_or_else(|| self.indexed(i), to_color32),
            Color::Named(n) => overrides[n].map_or_else(|| self.slot(n as usize), to_color32),
        }
    }

    /// As alacritty's `Rgb` (answers to colour queries).
    pub fn rgb(&self, index: usize) -> Rgb {
        let c = self.slot(index);
        Rgb { r: c.r(), g: c.g(), b: c.b() }
    }
}

pub fn to_color32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// Faint text (SGR 2): two thirds of the way from the background to the colour.
pub fn dim(fg: Color32, bg: Color32) -> Color32 {
    let mix = |f: u8, b: u8| ((f as f32) * 0.66 + (b as f32) * 0.34).round() as u8;
    Color32::from_rgb(mix(fg.r(), bg.r()), mix(fg.g(), bg.g()), mix(fg.b(), bg.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_256() {
        let c = TermColors::for_theme(true);
        assert_eq!(c.indexed(1), ANSI_DARK[1]);
        assert_eq!(c.indexed(16), Color32::from_rgb(0, 0, 0));
        assert_eq!(c.indexed(231), Color32::from_rgb(255, 255, 255));
        assert_eq!(c.indexed(196), Color32::from_rgb(255, 0, 0));
        assert_eq!(c.indexed(232), Color32::from_rgb(8, 8, 8));
        assert_eq!(c.indexed(255), Color32::from_rgb(238, 238, 238));
    }

    #[test]
    fn defaults_follow_the_scheme() {
        for dark in [true, false] {
            let c = TermColors::for_theme(dark);
            assert_eq!(c.bg, rc_kit::theme::palette(dark).editor_bg);
            assert_eq!(c.fg, rc_kit::theme::scheme(dark).text);
            let none = Colors::default();
            assert_eq!(c.resolve(Color::Named(NamedColor::Background), &none), c.bg);
            assert_eq!(c.resolve(Color::Spec(Rgb { r: 1, g: 2, b: 3 }), &none), Color32::from_rgb(1, 2, 3));
        }
    }
}
