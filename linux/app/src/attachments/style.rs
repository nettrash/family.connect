//! P2's part of the stylesheet (`theme::register`): tiles, file and location cards, the map, link
//! cards, the viewer's stage, the sticker panel and the pack. Every class is prefixed `fc-attach-`,
//! `fc-viewer-`, `fc-sticker-` or `fc-pack-`. Inside the reader's own balloon (`fc-own-bubble`,
//! the accent behind it) nothing is drawn in the accent: the cards go translucent white, as
//! Windows' `Palette.Ink` lesson and the Mac's own balloon have it.

use crate::theme::palette::Palette;

/// The CSS for `palette`'s scheme.
pub fn css(palette: &Palette) -> String {
    let _ = palette;
    "\
.fc-attach-tile {
  border-radius: 8px;
  background-color: alpha(currentColor, 0.08);
}
.fc-attach-tile-more {
  background-color: rgba(0, 0, 0, 0.5);
  color: white;
  font-size: 24px;
  font-weight: bold;
}
.fc-attach-play {
  background-color: rgba(0, 0, 0, 0.45);
  color: white;
  border-radius: 9999px;
  min-width: 44px;
  min-height: 44px;
}
.fc-attach-duration {
  background-color: rgba(0, 0, 0, 0.55);
  color: white;
  border-radius: 6px;
  padding: 1px 6px;
  font-size: 11px;
  font-feature-settings: \"tnum\";
}
.fc-attach-card {
  border-radius: 12px;
  padding: 6px 10px 6px 8px;
  background-color: alpha(currentColor, 0.06);
  border: 1px solid alpha(currentColor, 0.12);
}
.fc-own-bubble .fc-attach-card {
  background-color: rgba(255, 255, 255, 0.14);
  border-color: rgba(255, 255, 255, 0.16);
}
.fc-attach-card-line {
  font-size: 11px;
  opacity: 0.75;
  font-feature-settings: \"tnum\";
}
.fc-attach-pin-disc {
  background-color: var(--accent-bg-color);
  color: var(--accent-fg-color);
  border-radius: 9999px;
  min-width: 36px;
  min-height: 36px;
}
.fc-own-bubble .fc-attach-pin-disc {
  background-color: white;
  color: var(--accent-bg-color);
}
.fc-attach-map {
  border-radius: 8px;
  background-color: alpha(currentColor, 0.08);
}
.fc-attach-map-credit {
  background-color: rgba(255, 255, 255, 0.78);
  color: #333333;
  font-size: 9px;
  padding: 1px 5px 2px 5px;
  border-top-left-radius: 4px;
}
.fc-attach-map-ring {
  background-color: white;
  border-radius: 9999px;
  min-width: 18px;
  min-height: 18px;
}
.fc-attach-map-dot {
  background-color: #e53935;
  border-radius: 9999px;
  min-width: 12px;
  min-height: 12px;
}
.fc-attach-link-card {
  border-radius: 12px;
  border: 1px solid alpha(currentColor, 0.15);
  background-color: alpha(currentColor, 0.05);
}
.fc-own-bubble .fc-attach-link-card {
  background-color: rgba(255, 255, 255, 0.14);
  border-color: rgba(255, 255, 255, 0.16);
}
.fc-attach-link-site {
  font-size: 10px;
  opacity: 0.7;
}
.fc-attach-link-title {
  font-size: 12px;
  font-weight: 600;
}
.fc-attach-link-text {
  font-size: 10px;
  opacity: 0.7;
}
.fc-viewer-stage {
  background-color: rgba(0, 0, 0, 0.85);
  color: white;
}
.fc-viewer-controls {
  background-color: rgba(0, 0, 0, 0.55);
  color: white;
  border-radius: 12px;
  padding: 4px 8px;
}
.fc-sticker-cell {
  border-radius: 12px;
  padding: 4px;
}
.fc-sticker-cell:hover {
  background-color: alpha(currentColor, 0.08);
}
.fc-sticker-heading {
  font-size: 11px;
  font-weight: 600;
  opacity: 0.7;
}
.fc-pack-cell {
  background-color: alpha(currentColor, 0.06);
  border-radius: 12px;
  padding: 6px;
}
.fc-pack-notice {
  color: var(--error-color);
}
"
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_is_balanced_and_prefixed() {
        let sheet = css(&Palette::for_dark(false));
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
