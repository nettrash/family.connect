//! P1's CSS (registered once by `windows::gate::install` through `theme::register`). Classes are
//! prefixed `fc-chat-`; the frozen ones (`fc-own-bubble`, `fc-bubble`, `fc-badge`) come from
//! `theme::css`. Pure: a function of the palette.

use crate::theme::palette::Palette;

/// The stylesheet part for the chats, the conversation and the composer.
pub fn css(palette: &Palette) -> String {
    let accent = palette.accent_bg.hex();
    let accent_soft = palette
        .accent_bg
        .rgba(if palette.dark { 0.35 } else { 0.18 });
    let accent_line = palette.accent_bg.rgba(0.85);
    format!(
        ".fc-chat-bubble {{
  border-radius: 16px;
  padding: 6px 12px;
}}
.fc-chat-bubble.fc-chat-run-mine-start {{ border-bottom-right-radius: 6px; }}
.fc-chat-bubble.fc-chat-run-theirs-start {{ border-bottom-left-radius: 6px; }}
.fc-chat-bare {{
  padding: 0;
  background: none;
}}
.fc-chat-row {{
  margin-top: 1px;
  margin-bottom: 1px;
}}
.fc-chat-row.fc-chat-turn {{
  margin-top: 8px;
}}
.fc-chat-row:focus-visible .fc-chat-bubble {{
  outline: 2px solid {accent};
  outline-offset: 2px;
}}
.fc-chat-day {{
  font-size: smaller;
  font-weight: bold;
  padding: 2px 10px;
  margin: 10px 0 6px 0;
  border-radius: 9999px;
  background-color: alpha(currentColor, 0.08);
}}
.fc-chat-divider {{
  color: {accent};
  font-size: smaller;
  font-weight: bold;
  margin: 8px 0;
}}
.fc-chat-sender {{
  font-size: smaller;
  font-weight: bold;
  opacity: 0.8;
  margin-top: 6px;
}}
.fc-chat-time {{
  font-size: 11px;
  opacity: 0.7;
  font-feature-settings: \"tnum\";
}}
.fc-chat-quote {{
  border-left: 3px solid {accent_line};
  border-radius: 4px;
  padding: 2px 8px;
  background-color: alpha(currentColor, 0.08);
}}
.fc-own-bubble .fc-chat-quote {{
  border-left-color: white;
}}
.fc-chat-quote-name {{
  font-size: smaller;
  font-weight: bold;
}}
.fc-chat-quote-text {{
  font-size: smaller;
}}
.fc-chat-hidden {{
  font-style: italic;
  opacity: 0.8;
}}
.fc-chat-reactions {{
  margin-top: 2px;
}}
.fc-chat-reaction {{
  min-height: 22px;
  padding: 0 8px;
  border-radius: 9999px;
  font-size: smaller;
}}
.fc-chat-reaction.fc-chat-mine {{
  background-color: {accent_soft};
}}
.fc-chat-reaction-row button {{
  min-width: 36px;
  min-height: 36px;
  font-size: 18px;
}}
.fc-chat-system {{
  font-size: smaller;
  opacity: 0.8;
}}
.fc-chat-pending {{
  font-size: smaller;
  opacity: 0.75;
}}
.fc-chat-pending-balloon {{
  opacity: 0.75;
}}
.fc-chat-poll-option {{
  padding: 4px 8px;
  border-radius: 8px;
}}
.fc-chat-poll-option.fc-chat-mine {{
  background-color: {accent_soft};
}}
.fc-chat-composer {{
  margin: 6px 0 10px 0;
}}
.fc-chat-capsule {{
  border-radius: 22px;
  border: 1px solid alpha(currentColor, 0.18);
  background-color: var(--view-bg-color);
  padding: 0 4px;
}}
.fc-chat-capsule:focus-within {{
  border-color: {accent_line};
}}
.fc-chat-field {{
  background: none;
}}
.fc-chat-field text {{
  background: none;
}}
.fc-chat-placeholder {{
  opacity: 0.5;
}}
.fc-chat-slot {{
  min-width: 44px;
  min-height: 44px;
  padding: 0;
  background: none;
  box-shadow: none;
}}
.fc-chat-slot-disc {{
  min-width: 40px;
  min-height: 40px;
  border-radius: 9999px;
  background-color: {accent};
  color: white;
}}
.fc-chat-slot:disabled .fc-chat-slot-disc {{
  background-color: alpha(currentColor, 0.15);
  color: alpha(currentColor, 0.5);
}}
.fc-chat-slot-disc.fc-chat-dimmed {{
  opacity: 0.5;
}}
.fc-chat-banner {{
  padding: 4px 8px;
  border-radius: 10px;
  background-color: alpha(currentColor, 0.06);
}}
.fc-chat-notice {{
  font-size: smaller;
}}
.fc-chat-chip {{
  padding: 2px 4px 2px 10px;
  border-radius: 9999px;
  background-color: alpha(currentColor, 0.08);
}}
.fc-chat-drop {{
  border: 2px solid {accent};
  border-radius: 12px;
  background-color: {accent_soft};
}}
.fc-chat-jump {{
  min-width: 36px;
  min-height: 36px;
  border-radius: 9999px;
}}
.fc-chat-list-row {{
  padding: 4px 2px;
}}
.fc-chat-list-preview {{
  font-size: smaller;
  opacity: 0.7;
}}
.fc-chat-list-time {{
  font-size: 11px;
  opacity: 0.6;
}}
.fc-chat-connection {{
  font-size: smaller;
  padding: 6px;
  opacity: 0.8;
}}
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheet_is_balanced_in_both_schemes() {
        for dark in [false, true] {
            let sheet = css(&Palette::for_dark(dark));
            assert_eq!(sheet.matches('{').count(), sheet.matches('}').count());
            assert!(sheet.contains(".fc-chat-slot-disc {"));
        }
    }
}
