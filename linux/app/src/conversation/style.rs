//! P1's CSS (registered once by `windows::gate::install` through `theme::register`). Classes are
//! prefixed `fc-chat-`; the frozen ones (`fc-own-bubble`, `fc-bubble`, `fc-badge`) come from
//! `theme::css`. Pure: a function of the palette.
//!
//! The balloons speak the Mac's shape language (`MacMessageRow`, Windows' `BalloonCorners`): round
//! corners, tightened on the sender's side where a balloon meets its run mates
//! (`fc-chat-joined-above` / `-below`, `bubble::joins` over `TimelineRow::run_start` / `run_end`);
//! a few emoji and a message of nothing but pictures stand bare (`fc-chat-bare`), a sticker and a
//! circle have no balloon at all; the time, the ticks and a thread's chip sit under the balloon, on
//! the window's ground.

use crate::theme::palette::Palette;

/// The round corner of a balloon, and the corner where it meets a run mate (the Mac's 14 and 4,
/// at GTK's slightly larger body).
const ROUND: u32 = 16;
const JOINED: u32 = 5;

/// The stylesheet part for the chats, the conversation and the composer.
pub fn css(palette: &Palette) -> String {
    let accent = palette.accent_bg.hex();
    let accent_text = palette.accent_text().hex();
    let accent_soft = palette
        .accent_bg
        .rgba(if palette.dark { 0.35 } else { 0.18 });
    let accent_line = palette.accent_bg.rgba(0.85);
    let hairline = palette.accent_text().rgba(0.35);
    let focus = palette.accent_bg.rgba(0.5);
    let focus_strong = palette.accent_bg.rgba(0.8);
    let lift = if palette.dark { 0.4 } else { 0.12 };
    // Small numbers on the accent: dimmer than the words, never under 3:1 on the bright dark one.
    let own_meta = if palette.dark { 0.9 } else { 0.8 };
    let capsule = if palette.dark { 0.08 } else { 0.06 };
    let mut out = format!(
        ".fc-chat-bubble {{
  border-radius: {ROUND}px;
  padding: 7px 12px;
  transition: box-shadow 120ms ease-out;
}}
.fc-own-bubble.fc-chat-joined-above {{ border-top-right-radius: {JOINED}px; }}
.fc-own-bubble.fc-chat-joined-below {{ border-bottom-right-radius: {JOINED}px; }}
.fc-bubble.fc-chat-joined-above {{ border-top-left-radius: {JOINED}px; }}
.fc-bubble.fc-chat-joined-below {{ border-bottom-left-radius: {JOINED}px; }}
.fc-own-bubble.fc-chat-joined-above:dir(rtl) {{
  border-top-right-radius: {ROUND}px;
  border-top-left-radius: {JOINED}px;
}}
.fc-own-bubble.fc-chat-joined-below:dir(rtl) {{
  border-bottom-right-radius: {ROUND}px;
  border-bottom-left-radius: {JOINED}px;
}}
.fc-bubble.fc-chat-joined-above:dir(rtl) {{
  border-top-left-radius: {ROUND}px;
  border-top-right-radius: {JOINED}px;
}}
.fc-bubble.fc-chat-joined-below:dir(rtl) {{
  border-bottom-left-radius: {ROUND}px;
  border-bottom-right-radius: {JOINED}px;
}}
.fc-chat-bubble:hover {{
  box-shadow: 0 1px 3px rgba(0, 0, 0, {lift});
}}
.fc-chat-bubble.fc-chat-bare {{
  padding: 0;
  background: none;
  color: inherit;
  box-shadow: none;
}}
.fc-own-bubble.fc-chat-bare selection {{
  background-color: {accent_soft};
  color: inherit;
}}
.fc-chat-row {{
  margin-top: 1px;
  margin-bottom: 1px;
}}
.fc-chat-row.fc-chat-turn {{
  margin-top: 10px;
}}
.fc-chat-row:focus-visible .fc-chat-bubble {{
  outline: 2px solid {focus};
  outline-offset: -2px;
}}
.fc-chat-row:focus-visible .fc-own-bubble:not(.fc-chat-bare) {{
  outline-color: rgba(255, 255, 255, 0.8);
}}
.fc-chat-row:focus-visible .fc-chat-bubble.fc-chat-bare {{
  outline-offset: 2px;
}}
.fc-chat-row {{
  border-radius: 12px;
  transition: background-color 600ms ease-out;
}}
.fc-chat-row.fc-chat-jumped {{
  background-color: {accent_soft};
  transition: none;
}}
.fc-chat-bubble > button.flat {{
  padding-left: 8px;
  padding-right: 8px;
  margin-left: -8px;
  margin-right: -8px;
}}
.fc-chat-day {{
  font-size: smaller;
  font-weight: 600;
  padding: 3px 12px;
  margin: 12px 0 8px 0;
  border-radius: 9999px;
  background-color: alpha(currentColor, 0.06);
  color: alpha(currentColor, 0.75);
}}
.fc-chat-divider {{
  margin: 10px 0 6px 0;
}}
.fc-chat-divider label {{
  color: {accent_text};
  font-size: smaller;
  font-weight: bold;
}}
.fc-chat-divider separator {{
  background-color: {hairline};
  min-height: 1px;
}}
.fc-chat-sender {{
  font-size: smaller;
  font-weight: 600;
  color: {accent_text};
  margin: 4px 0 2px 2px;
}}
.fc-chat-time {{
  font-size: 11px;
  opacity: 0.75;
  font-feature-settings: \"tnum\";
}}
.fc-own-bubble .fc-chat-time {{
  opacity: {own_meta};
}}
.fc-chat-meta {{
  margin: 2px 6px 0 6px;
}}
.fc-chat-meta .fc-chat-seen {{
  color: {accent_text};
  opacity: 1;
}}
.fc-chat-thread-chip {{
  color: {accent_text};
  font-size: smaller;
  font-weight: 600;
  min-height: 24px;
  padding: 0 8px;
  margin-top: 2px;
}}
.fc-chat-thread-chip image {{
  -gtk-icon-size: 14px;
}}
.fc-chat-thread-chip image.fc-chat-chevron {{
  -gtk-icon-size: 10px;
}}
.fc-chat-quote {{
  border-left: 3px solid {accent_line};
  border-radius: 4px;
  padding: 2px 8px;
  background-color: alpha(currentColor, 0.06);
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
  margin-top: 3px;
}}
.fc-chat-reaction {{
  min-height: 24px;
  padding: 0 8px;
  border-radius: 9999px;
  font-size: smaller;
}}
.fc-chat-reaction.fc-chat-mine {{
  background-color: {accent_soft};
  box-shadow: inset 0 0 0 1px {accent_line};
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
.fc-chat-pending-status {{
  margin-right: -6px;
}}
.fc-chat-pending-status:dir(rtl) {{
  margin-right: 0;
  margin-left: -6px;
}}
.fc-chat-pending-status > button {{
  padding-left: 6px;
  padding-right: 6px;
}}
.fc-chat-pending-status > image {{
  color: var(--error-color);
}}
.fc-chat-poll-option {{
  padding: 6px 8px;
  border-radius: 10px;
  font-weight: normal;
}}
.fc-chat-poll-option.fc-chat-mine {{
  background-color: {accent_soft};
}}
.fc-own-bubble .fc-chat-poll-option.fc-chat-mine {{
  background-color: rgba(255, 255, 255, 0.15);
}}
.fc-chat-poll-option levelbar block.filled {{
  background-color: {accent};
}}
.fc-chat-poll-option levelbar block.empty {{
  background-color: alpha(currentColor, 0.15);
}}
.fc-own-bubble .fc-chat-poll-option levelbar block.filled {{
  background-color: white;
}}
.fc-own-bubble .fc-chat-poll-option levelbar block.empty {{
  background-color: rgba(255, 255, 255, 0.3);
}}
.fc-chat-composer {{
  margin: 6px 0 10px 0;
}}
.fc-chat-capsule {{
  border-radius: 22px;
  background-color: alpha(currentColor, {capsule});
  padding: 0 4px;
  outline: 2px solid transparent;
  outline-offset: -2px;
  transition: outline-color 150ms ease-out;
}}
.fc-chat-capsule:focus-within {{
  outline-color: {focus};
}}
.fc-chat-capsule scrollbar slider {{
  min-height: 12px;
}}
.fc-chat-capsule button.fc-chat-tinted {{
  color: {accent_text};
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
  transition: filter 120ms ease-out;
}}
.fc-chat-slot:hover .fc-chat-slot-disc {{
  filter: brightness(1.12);
}}
.fc-chat-slot:active .fc-chat-slot-disc {{
  filter: brightness(0.9);
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
  border-radius: 12px;
  background-color: alpha(currentColor, 0.06);
}}
.fc-chat-banner.fc-chat-reply-banner {{
  border-left: 3px solid {accent_line};
  border-top-left-radius: 4px;
  border-bottom-left-radius: 4px;
}}
.fc-chat-notice {{
  font-size: smaller;
}}
.fc-chat-hints {{
  margin: 0 8px;
}}
.fc-chat-hint image {{
  opacity: 0.7;
  margin-top: 1px;
}}
.fc-chat-hint button {{
  min-height: 0;
  padding: 0 6px;
}}
.fc-chat-chip {{
  padding: 2px 4px 2px 12px;
  border-radius: 9999px;
  background-color: alpha(currentColor, 0.08);
}}
.fc-chat-chip.fc-chat-chip-pictured {{
  padding-left: 4px;
  border-radius: 12px;
}}
.fc-chat-chip-thumb {{
  border-radius: 8px;
}}
.fc-chat-chip-play {{
  padding: 2px;
  -gtk-icon-size: 12px;
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
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.25);
}}
.fc-chat-list-row {{
  padding: 5px 2px;
}}
.fc-chat-list-preview {{
  font-size: smaller;
  opacity: 0.7;
}}
.fc-chat-list-time {{
  font-size: 11px;
  opacity: 0.75;
  font-feature-settings: \"tnum\";
}}
.fc-chat-list-time.fc-chat-unread {{
  color: {accent_text};
  opacity: 1;
}}
.fc-chat-connection {{
  font-size: smaller;
  padding: 6px;
  opacity: 0.8;
}}
.fc-chat-brand {{
  margin-bottom: 4px;
}}
.fc-chat-gate-actions > button.pill {{
  padding-left: 20px;
  padding-right: 20px;
}}
@media (prefers-contrast: more) {{
  .fc-chat-capsule {{
    box-shadow: inset 0 0 0 1px alpha(currentColor, 0.5);
  }}
  .fc-chat-row:focus-visible .fc-chat-bubble,
  .fc-chat-capsule:focus-within {{
    outline-color: {focus_strong};
  }}
  .fc-chat-bubble.fc-bubble:not(.fc-chat-bare):hover {{
    box-shadow: inset 0 0 0 1px alpha(currentColor, 0.5);
  }}
  .fc-chat-day {{
    color: currentColor;
  }}
}}
@media (prefers-reduced-motion: reduce) {{
  .fc-chat-bubble,
  .fc-chat-capsule,
  .fc-chat-slot-disc {{
    transition: none;
  }}
}}
"
    );
    // A few emoji and nothing else draw big and bare on every client (`fc_text::emoji`): the
    // ladder's sizes are points against a phone's 17-point body, so here they are that PROPORTION
    // of the body the balloon inherits — the Mac's own rule (`display_font_size_for_body`).
    for (index, size) in fc_text::emoji::EMOJI_FONT_LADDER.iter().enumerate() {
        let em = size / fc_text::emoji::LADDER_BODY_SIZE;
        out.push_str(&format!(
            ".fc-chat-emoji-{} {{ font-size: {em:.3}em; }}\n",
            index + 1
        ));
    }
    out
}

/// The class a one-to-four-emoji body draws with, or `None` for anything else (`fc_text`'s
/// scanner: five or more, any text at all, or nothing but whitespace draws as words).
pub fn emoji_class(body: &str) -> Option<String> {
    let count = fc_text::emoji::emoji_only_count(body)?;
    (1..=fc_text::emoji::EMOJI_FONT_LADDER.len())
        .contains(&count)
        .then(|| format!("fc-chat-emoji-{count}"))
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
            assert!(sheet.contains(".fc-bubble.fc-chat-joined-below {"));
        }
    }

    #[test]
    fn the_emoji_ladder_is_the_apps_in_proportion_to_the_body() {
        let sheet = css(&Palette::for_dark(false));
        assert!(
            sheet.contains(".fc-chat-emoji-1 { font-size: 5.647em; }"),
            "{sheet}"
        );
        assert!(
            sheet.contains(".fc-chat-emoji-4 { font-size: 3.294em; }"),
            "{sheet}"
        );
        assert!(!sheet.contains(".fc-chat-emoji-5"));
    }

    #[test]
    fn only_one_to_four_emoji_draw_big() {
        assert_eq!(emoji_class("😀").as_deref(), Some("fc-chat-emoji-1"));
        assert_eq!(emoji_class("😀 👍").as_deref(), Some("fc-chat-emoji-2"));
        assert_eq!(emoji_class("😀😀😀😀").as_deref(), Some("fc-chat-emoji-4"));
        assert_eq!(emoji_class("😀😀😀😀😀"), None);
        assert_eq!(emoji_class("Hi 😀"), None);
        assert_eq!(emoji_class("   "), None);
        assert_eq!(emoji_class(""), None);
    }

    #[test]
    fn small_accent_text_is_the_lifted_accent_on_dark() {
        let dark = css(&Palette::for_dark(true));
        assert!(dark.contains("color: #afc5fe;"), "{dark}");
        let light = css(&Palette::for_dark(false));
        assert!(light.contains("color: #0d47a1;"), "{light}");
    }
}
