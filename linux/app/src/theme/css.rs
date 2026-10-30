//! The stylesheet, generated from a [`Palette`] (shell-design §25: "One CSS file, generated from a
//! palette"). Pure: [`css`] is a function of the palette and the parts the packages registered.
//!
//! **libadwaita's variables** at `STYLE_PROVIDER_PRIORITY_APPLICATION`, overriding the desktop's
//! accent (`--accent-bg-color`, `--accent-color`, `--accent-fg-color`, and Yaru's
//! `--yaru-accent-bg-color`, which Ubuntu's libadwaita reads) — and the links' colours, which
//! Ubuntu's libadwaita writes out rather than taking from the accent.
//!
//! **The app's style classes** (frozen, W.3 — a package uses them, and adds its own through
//! `theme::register`):
//!
//! | Class | Look | Used by |
//! | --- | --- | --- |
//! | `fc-own-bubble` | the accent fill, white ink, links and selection legible on it | P1's bubbles |
//! | `fc-bubble` | a quiet neutral fill over the ground, the ink of the window (the Mac's `appSecondaryFill` balloon); outlined in high contrast | P1's bubbles |
//! | `fc-badge` | a pill: accent fill, white bold 11 px, a 16 px disc for one digit (GTK's `min-width` is the content box's), round; over a header button it sits on the glyph's corner, ringed in the header's colour | the header's Board badge (S0/P1/P4), unread pills |
//! | `fc-round` | `border-radius: 9999px` — with `set_overflow(Hidden)` it clips a child to a disc | P3's circles (§10.4) |
//! | `fc-placeholder` | dimmed, the gate's empty pages until P1 fills them | S0 |

use super::palette::Palette;

/// The whole stylesheet: the variables, the frozen classes, then each registered part in
/// registration order.
pub fn css(palette: &Palette, parts: &[String]) -> String {
    let accent_bg = palette.accent_bg.hex();
    let accent = palette.accent.hex();
    let accent_fg = palette.accent_fg.hex();
    let selection = palette.accent_fg.rgba(0.3);
    // Ubuntu's libadwaita writes its links' colours out (#ac3911 light, #f08762 dark, hover and
    // active derived from them) where stock libadwaita uses the accent, so no variable reaches
    // them: the app's blue, said for every state (small text, so the text tint).
    let link = palette.accent_text().hex();
    // The other person's balloon: the ink of the window at a tenth or less — grey on either
    // ground, never a white card on a near-white window.
    let incoming = if palette.dark { 0.1 } else { 0.08 };
    let mut out = format!(
        ":root {{
  --accent-bg-color: {accent_bg};
  --accent-color: {accent};
  --accent-fg-color: {accent_fg};
  --yaru-accent-bg-color: {accent_bg};
}}
link,
button.link,
link:visited,
button.link:visited,
link:active,
button.link:active {{
  color: {link};
}}
link:hover,
button.link:hover {{
  color: HSL(from {link} h calc(s * 1.1) calc(l * 1.1));
}}
.fc-own-bubble {{
  background-color: {accent_bg};
  color: {accent_fg};
}}
.fc-own-bubble link,
.fc-own-bubble label link,
.fc-own-bubble link:visited,
.fc-own-bubble link:hover,
.fc-own-bubble link:active {{
  color: {accent_fg};
}}
.fc-own-bubble selection {{
  background-color: {selection};
  color: {accent_fg};
}}
.fc-bubble {{
  background-color: alpha(currentColor, {incoming});
}}
@media (prefers-contrast: more) {{
  .fc-bubble {{
    box-shadow: inset 0 0 0 1px alpha(currentColor, 0.5);
  }}
}}
.fc-badge {{
  background-color: {accent_bg};
  color: {accent_fg};
  border-radius: 9999px;
  min-width: 8px;
  min-height: 16px;
  padding: 0 4px;
  font-size: 11px;
  font-weight: bold;
  font-feature-settings: \"tnum\";
}}
headerbar overlay > .fc-badge {{
  margin-top: -3px;
  margin-right: -4px;
  box-shadow: 0 0 0 2px var(--window-bg-color);
}}
headerbar overlay > .fc-badge:dir(rtl) {{
  margin-right: 0;
  margin-left: -4px;
}}
.fc-round {{
  border-radius: 9999px;
}}
.fc-placeholder {{
  opacity: 0.55;
}}
"
    );
    for part in parts {
        out.push_str(part);
        if !part.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variables_carry_the_macs_accent_for_each_scheme() {
        let light = css(&Palette::for_dark(false), &[]);
        assert!(light.contains("--accent-bg-color: #0d47a1;"), "{light}");
        assert!(light.contains("--accent-fg-color: #ffffff;"), "{light}");
        assert!(
            light.contains("--yaru-accent-bg-color: #0d47a1;"),
            "{light}"
        );
        assert!(
            light.contains("button.link:active {\n  color: #0d47a1;"),
            "{light}"
        );
        let dark = css(&Palette::for_dark(true), &[]);
        assert!(dark.contains("--accent-bg-color: #4d7dfc;"), "{dark}");
        assert!(dark.contains("--accent-color: #4d7dfc;"), "{dark}");
        // Links are small text: the lifted tint on the dark ground.
        assert!(
            dark.contains("button.link:active {\n  color: #afc5fe;"),
            "{dark}"
        );
    }

    #[test]
    fn every_frozen_class_is_defined_and_registered_parts_follow_in_order() {
        let sheet = css(
            &Palette::for_dark(false),
            &[
                ".p1-a { margin: 1px; }".to_string(),
                ".p2-b {}\n".to_string(),
            ],
        );
        for class in [
            "fc-own-bubble",
            "fc-bubble",
            "fc-badge",
            "fc-round",
            "fc-placeholder",
        ] {
            assert!(sheet.contains(&format!(".{class} {{")), "{class}");
        }
        let a = sheet.find(".p1-a").unwrap();
        let b = sheet.find(".p2-b").unwrap();
        assert!(a < b);
        assert!(sheet.ends_with(".p2-b {}\n"));
        // Balanced braces: a stray one would make GTK drop the rest of the sheet.
        assert_eq!(sheet.matches('{').count(), sheet.matches('}').count());
    }
}
