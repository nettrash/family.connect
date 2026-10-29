//! P2's part of the stylesheet (`theme::register`): tiles, file and location cards, the map, link
//! cards, the viewer's stage, the sticker panel and the pack. Every class is prefixed `fc-attach-`,
//! `fc-viewer-`, `fc-sticker-` or `fc-pack-`. Inside the reader's own balloon (`fc-own-bubble`,
//! the accent behind it) nothing is drawn in the accent: the cards go translucent white, as
//! Windows' `Palette.Ink` lesson and the Mac's own balloon have it.
//!
//! One anatomy for every card in a bubble — an accent disc leading the row, a picture flush or on
//! an even mat above it — and one language for what lies over a picture: a dark screen, a rimmed
//! play disc, pill-shaped chips. Small type is sized relative to the body, so it follows the
//! desktop's text size.

use crate::theme::palette::{ACCENT_DARK, Palette};

/// The CSS for `palette`'s scheme.
pub fn css(palette: &Palette) -> String {
    let accent = palette.accent.hex();
    // The viewer's header sits on its dark stage in both schemes: the dark scheme's accent there.
    let dark_accent = ACCENT_DARK.hex();
    format!(
        "\
.fc-attach-tile {{
  border-radius: 8px;
  background-color: alpha(currentColor, 0.08);
}}
.fc-attach-tile.fc-attach-video {{
  background-color: #1e1e20;
  background-image: linear-gradient(to bottom, rgba(255, 255, 255, 0.06), transparent);
  color: rgba(255, 255, 255, 0.9);
}}
.fc-attach-tile:focus-visible,
.fc-attach-card:focus-visible,
.fc-attach-link-card:focus-visible {{
  outline: 2px solid {accent};
  outline-offset: 1px;
}}
/* White on the reader's own accent balloon; a bare one has no accent behind it. */
.fc-own-bubble:not(.fc-chat-bare) .fc-attach-tile:focus-visible,
.fc-own-bubble:not(.fc-chat-bare) .fc-attach-card:focus-visible,
.fc-own-bubble:not(.fc-chat-bare) .fc-attach-link-card:focus-visible {{
  outline-color: white;
}}
.fc-attach-tile-more {{
  background-color: rgba(0, 0, 0, 0.5);
  color: white;
  font-size: 24px;
  font-weight: bold;
}}
.fc-attach-play {{
  background-color: rgba(0, 0, 0, 0.5);
  color: white;
  border-radius: 9999px;
  min-width: 44px;
  min-height: 44px;
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.22), 0 1px 4px rgba(0, 0, 0, 0.3);
  -gtk-icon-transform: translate(1px, 0);
}}
.fc-attach-play:dir(rtl) {{
  -gtk-icon-transform: translate(-1px, 0);
}}
.fc-attach-duration {{
  background-color: rgba(0, 0, 0, 0.55);
  color: white;
  border-radius: 9999px;
  padding: 1px 7px;
  font-size: 82%;
  font-feature-settings: \"tnum\";
}}
.fc-attach-card {{
  border-radius: 12px;
  padding: 6px 10px 6px 8px;
  background-color: alpha(currentColor, 0.06);
  border: 1px solid alpha(currentColor, 0.12);
}}
.fc-attach-card.fc-attach-place-map {{
  padding: 9px 9px 6px 9px;
}}
.fc-attach-card.fc-attach-place:hover {{
  background-color: alpha(currentColor, 0.09);
}}
.fc-own-bubble .fc-attach-card {{
  background-color: rgba(255, 255, 255, 0.14);
  border-color: rgba(255, 255, 255, 0.16);
}}
.fc-own-bubble .fc-attach-card.fc-attach-place:hover {{
  background-color: rgba(255, 255, 255, 0.2);
}}
.fc-attach-card-line {{
  font-size: 82%;
  opacity: 0.75;
  font-feature-settings: \"tnum\";
}}
.fc-attach-pin-disc,
.fc-attach-file-disc {{
  background-color: var(--accent-bg-color);
  color: var(--accent-fg-color);
  border-radius: 9999px;
  min-width: 36px;
  min-height: 36px;
}}
.fc-own-bubble .fc-attach-pin-disc,
.fc-own-bubble .fc-attach-file-disc {{
  background-color: white;
  color: var(--accent-bg-color);
}}
.fc-attach-map {{
  border-radius: 8px;
  background-color: alpha(currentColor, 0.08);
}}
.fc-attach-place-map .fc-attach-map {{
  border-radius: 4px;
}}
.fc-attach-map-credit {{
  background-color: rgba(255, 255, 255, 0.78);
  color: #333333;
  font-size: 9px;
  padding: 1px 5px 2px 5px;
  border-top-left-radius: 4px;
}}
.fc-attach-map-ring {{
  background-color: white;
  border-radius: 9999px;
  min-width: 18px;
  min-height: 18px;
}}
.fc-attach-map-dot {{
  background-color: #e53935;
  border-radius: 9999px;
  min-width: 12px;
  min-height: 12px;
}}
.fc-attach-link-card {{
  border-radius: 12px;
  border: 1px solid alpha(currentColor, 0.15);
  background-color: alpha(currentColor, 0.05);
}}
.fc-attach-link-card:hover {{
  background-color: alpha(currentColor, 0.09);
}}
.fc-own-bubble .fc-attach-link-card {{
  background-color: rgba(255, 255, 255, 0.14);
  border-color: rgba(255, 255, 255, 0.16);
}}
.fc-own-bubble .fc-attach-link-card:hover {{
  background-color: rgba(255, 255, 255, 0.2);
}}
.fc-attach-link-site {{
  font-size: 82%;
  font-weight: 600;
  opacity: 0.75;
}}
.fc-attach-link-title {{
  font-weight: bold;
}}
.fc-attach-link-text {{
  font-size: 82%;
  opacity: 0.75;
}}
.fc-viewer-stage {{
  background-color: rgba(0, 0, 0, 0.85);
  color: white;
}}
toolbarview.fc-viewer-toolbar > .top-bar {{
  background-color: color-mix(in srgb, black 85%, var(--window-bg-color));
  color: #eeeeec;
  --accent-color: {dark_accent};
}}
.fc-viewer-controls {{
  background-color: rgba(0, 0, 0, 0.55);
  color: white;
  border-radius: 12px;
  padding: 4px 8px;
}}
.fc-viewer-controls scale > trough > highlight {{
  background-color: {dark_accent};
}}
.fc-sticker-cell {{
  border-radius: 12px;
  padding: 4px;
}}
.fc-sticker-cell:hover {{
  background-color: alpha(currentColor, 0.08);
}}
.fc-sticker-heading {{
  font-size: 82%;
  font-weight: bold;
  opacity: 0.7;
}}
.fc-pack-cell {{
  background-color: alpha(currentColor, 0.06);
  border-radius: 12px;
  padding: 6px;
}}
.fc-pack-notice {{
  color: var(--error-color);
}}
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_is_balanced_and_prefixed() {
        for dark in [false, true] {
            let sheet = css(&Palette::for_dark(dark));
            assert_eq!(sheet.matches('{').count(), sheet.matches('}').count());
            for line in sheet.lines().filter(|l| l.ends_with('{')) {
                assert!(
                    line.contains(".fc-attach-")
                        || line.contains(".fc-viewer-")
                        || line.contains(".fc-sticker-")
                        || line.contains(".fc-pack-"),
                    "{line}"
                );
            }
        }
    }

    #[test]
    fn a_focused_tile_rings_in_the_accent_in_both_schemes() {
        assert!(css(&Palette::for_dark(false)).contains("outline: 2px solid #0d47a1;"));
        assert!(css(&Palette::for_dark(true)).contains("outline: 2px solid #4d7dfc;"));
    }
}
