//! The app's colours (shell-design §25): the Mac's `AccentColor` — **`#0D47A1` on a light ground,
//! `#4D7DFC` on a dark one** — so the reader's own balloon, the slot and the badges are the same
//! blue as on every other client, whatever accent the desktop has. The own balloon's ink is white
//! in both schemes (Windows' `Palette.Ink` lesson: the dark accent with black ink was wrong).

/// An sRGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// `#rrggbb`, lower case.
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// `rgba(r, g, b, a)` with `alpha` to two decimals.
    pub fn rgba(self, alpha: f64) -> String {
        let alpha = (alpha.clamp(0.0, 1.0) * 100.0).round() / 100.0;
        format!("rgba({}, {}, {}, {alpha})", self.0, self.1, self.2)
    }

    /// WCAG relative luminance.
    #[cfg(test)]
    pub fn luminance(self) -> f64 {
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.0) + 0.7152 * channel(self.1) + 0.0722 * channel(self.2)
    }

    /// WCAG contrast ratio between two colours (1 to 21).
    #[cfg(test)]
    pub fn contrast(self, other: Rgb) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        let (light, dark) = if a > b { (a, b) } else { (b, a) };
        (light + 0.05) / (dark + 0.05)
    }
}

/// The Mac's accent on a light ground.
pub const ACCENT_LIGHT: Rgb = Rgb(0x0D, 0x47, 0xA1);
/// The Mac's accent on a dark ground.
pub const ACCENT_DARK: Rgb = Rgb(0x4D, 0x7D, 0xFC);
/// White: the ink on the accent, both schemes.
pub const WHITE: Rgb = Rgb(0xFF, 0xFF, 0xFF);
/// libadwaita 1.9's window background, light and dark (for contrast checks only).
#[cfg(test)]
pub const ADWAITA_LIGHT_BG: Rgb = Rgb(0xFA, 0xFA, 0xFB);
#[cfg(test)]
pub const ADWAITA_DARK_BG: Rgb = Rgb(0x22, 0x22, 0x26);

/// The colours of one scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub dark: bool,
    /// The accent as a fill: the own balloon, the slot's disc, a badge, a suggested button.
    pub accent_bg: Rgb,
    /// The accent as text or an outline on the window's ground.
    pub accent: Rgb,
    /// What is written on the accent fill: white in both schemes.
    pub accent_fg: Rgb,
}

impl Palette {
    pub fn for_dark(dark: bool) -> Palette {
        let accent = if dark { ACCENT_DARK } else { ACCENT_LIGHT };
        Palette {
            dark,
            accent_bg: accent,
            accent,
            accent_fg: WHITE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accent_is_the_macs_in_both_schemes_with_white_ink() {
        let light = Palette::for_dark(false);
        let dark = Palette::for_dark(true);
        assert_eq!(light.accent_bg.hex(), "#0d47a1");
        assert_eq!(dark.accent_bg.hex(), "#4d7dfc");
        assert_eq!(light.accent_fg, WHITE);
        assert_eq!(dark.accent_fg, WHITE);
    }

    #[test]
    fn the_accent_reads_on_its_ground_and_white_reads_on_the_accent() {
        // Text on the ground: AA for normal text (4.5) on light; on dark the Mac's #4D7DFC measures
        // 4.28 — AA for large text and for controls (WCAG 1.4.11, 3:1). The Mac's colour is kept
        // (the Mac wins); a package drawing small accent text on the dark ground should know it.
        assert!(ACCENT_LIGHT.contrast(ADWAITA_LIGHT_BG) >= 4.5);
        let dark = ACCENT_DARK.contrast(ADWAITA_DARK_BG);
        assert!((4.2..4.5).contains(&dark), "{dark}");
        // White on the light accent is AA; on the bright dark accent it is AA for large text and
        // the bubble's bold-weight UI (the Mac's choice, kept).
        assert!(WHITE.contrast(ACCENT_LIGHT) >= 4.5);
        assert!(WHITE.contrast(ACCENT_DARK) >= 3.0);
    }

    #[test]
    fn colours_are_written_as_css_wants_them() {
        assert_eq!(Rgb(1, 2, 255).hex(), "#0102ff");
        assert_eq!(Rgb(13, 71, 161).rgba(0.355), "rgba(13, 71, 161, 0.36)");
        assert_eq!(WHITE.rgba(2.0), "rgba(255, 255, 255, 1)");
    }
}
