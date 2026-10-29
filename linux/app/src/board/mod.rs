//! The family's board (shell-design §11; the Mac's `MacBoardView`): sticker notes on a wall taller
//! than the window, placed by FRACTION of the wall (the note's top-left corner, clamped inside),
//! moved by anyone with one PATCH on release, rewritten and taken down by their author.
//!
//! | Module | What |
//! | --- | --- |
//! | [`wall`] | the wall: placing, drawing, dragging and nudging notes, a note's menu |
//! | [`sheet`] | the note sheet (new and edit): words, colour, size, hand, a list's lines, an event's when and where |
//! | [`calendar`] | "Add to Calendar": the `.ics` file (pure, tested) |
//! | `windows::board_window` | the Board window |
//!
//! The arithmetic is `fc_text::board`'s (through fc-core and `BoardModel`), the words
//! `wall_text`'s, `event_text`'s and `note_sheet_text`'s, the drag `NoteHand`'s — the GTK here only
//! draws and listens.

pub mod calendar;
pub mod sheet;
pub mod wall;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use fc_core::board::note_look;
use fc_logic::board::BoardModel;
use fc_text::board::{COLORS, Font, Size, color_hex};

use crate::shell::{Connection, Shell};

/// The wall's notes over this connection: the family SHOWN (the board store follows the session,
/// and every write goes to the family shown — `ApiClient`'s scope).
pub fn model(connection: &Connection) -> BoardModel {
    BoardModel::new(
        Rc::clone(&connection.board),
        Rc::clone(&connection.chats),
        Rc::clone(&connection.api),
    )
}

/// The package's stylesheet part (registered once in `settings::install`): the wall's ground, the
/// six pastels, the hands and sizes, the lifted card, the pill and the swatches — and the Settings
/// window's and the Family dialog's few rules of their own (P4's three surfaces share one part).
pub fn css(dark: bool) -> String {
    let ground = if dark { "#2b2925" } else { "#ece5d8" };
    // Paper on the wall: a hairline contact edge, a short contact shadow and a soft ambient one,
    // stronger on the dark wall (where the light wall's would vanish), and higher when lifted; a
    // sheen of top light on the light wall, a 6 % veil over the pastels on the dark one.
    let (sheen, rest, lifted) = if dark {
        (
            "image(rgba(0, 0, 0, 0.06))",
            "0 0 0 1px rgba(0, 0, 0, 0.3), 0 2px 4px rgba(0, 0, 0, 0.35), \
             0 8px 18px -4px rgba(0, 0, 0, 0.5)",
            "0 0 0 1px rgba(0, 0, 0, 0.3), 0 10px 18px -4px rgba(0, 0, 0, 0.55), \
             0 22px 44px -8px rgba(0, 0, 0, 0.6)",
        )
    } else {
        (
            "linear-gradient(to bottom, rgba(255, 255, 255, 0.22), rgba(255, 255, 255, 0) 45%)",
            "0 0 0 1px rgba(0, 0, 0, 0.05), 0 1px 2px rgba(0, 0, 0, 0.1), \
             0 4px 10px -2px rgba(0, 0, 0, 0.16)",
            "0 0 0 1px rgba(0, 0, 0, 0.05), 0 8px 14px -4px rgba(0, 0, 0, 0.22), \
             0 18px 36px -8px rgba(0, 0, 0, 0.26)",
        )
    };
    // The Owner pill's word: the accent on light; on dark a light tint of it, readable on a card
    // (the accent itself measured 2.2:1 there).
    let (pill_ink, pill_ground) = if dark {
        (
            "color-mix(in srgb, var(--accent-bg-color) 40%, white)",
            0.18,
        )
    } else {
        ("var(--accent-color)", 0.15)
    };
    // The face at the top of Settings and of the Family dialog, lifted off the page.
    let hero = if dark {
        "0 0 0 1px rgba(255, 255, 255, 0.08), 0 3px 10px rgba(0, 0, 0, 0.45)"
    } else {
        "0 0 0 1px rgba(0, 0, 6, 0.06), 0 3px 10px rgba(0, 0, 6, 0.14)"
    };
    let mut out = format!(
        ".fc-board-ground {{ background-color: {ground}; }}
.fc-note {{
  border-radius: 8px;
  padding: 10px;
  color: rgba(0, 0, 0, 0.85);
  background-image: {sheen};
  box-shadow: {rest};
  transition: box-shadow 150ms ease-out;
}}
.fc-note.fc-note-bare {{
  background-color: transparent;
  background-image: none;
  padding: 0;
  border-radius: 4px;
}}
.fc-note.fc-note-lifted {{ box-shadow: {lifted}; }}
.fc-note:focus-visible {{ outline: 2px solid var(--accent-bg-color); outline-offset: 2px; }}
.fc-note-author {{ font-size: 10px; color: rgba(0, 0, 0, 0.62); }}
.fc-note-hidden {{ font-style: italic; color: rgba(0, 0, 0, 0.45); }}
.fc-note-date {{
  background-color: rgba(255, 255, 255, 0.6);
  border-radius: 6px;
  padding: 2px 7px 4px 7px;
  box-shadow: 0 1px 1px rgba(0, 0, 0, 0.06);
}}
.fc-note-day {{ font-size: 18px; font-weight: 800; font-feature-settings: \"tnum\"; }}
.fc-note-month {{ font-size: 10px; font-weight: 700; letter-spacing: 0.6px; color: var(--red-4); }}
.fc-note-line {{ font-size: 11px; }}
.fc-note-task image {{ -gtk-icon-size: 12px; color: rgba(0, 0, 0, 0.65); }}
.fc-note-task-done {{ opacity: 0.55; }}
.fc-note-task-done label {{ text-decoration-line: line-through; }}
.fc-owner-pill {{
  color: {pill_ink};
  background-color: alpha(var(--accent-bg-color), {pill_ground});
  border-radius: 9999px;
  padding: 1px 7px;
  font-size: 10px;
  font-weight: 600;
}}
.fc-invite-code .title {{
  font-family: monospace;
  font-size: 1.35em;
  font-weight: 600;
  letter-spacing: 2px;
  font-feature-settings: \"tnum\", \"zero\";
}}
toggle-group.fc-note-colors > toggle {{
  min-width: 34px;
  min-height: 34px;
  padding: 0;
  border-radius: 9999px;
}}
toggle-group.fc-note-colors > toggle:checked,
toggle-group.fc-note-colors > toggle:checked:hover {{
  background-color: transparent;
  box-shadow: inset 0 0 0 2px var(--accent-bg-color);
}}
.fc-swatch {{
  min-width: 22px;
  min-height: 22px;
  border-radius: 9999px;
  box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.14);
}}
window.fc-settings row:disabled label.subtitle {{ filter: none; }}
.fc-hero-face {{ border-radius: 9999px; box-shadow: {hero}; }}
@media (prefers-reduced-motion: reduce) {{
  .fc-note {{ transition: none; }}
}}
@media (prefers-contrast: more) {{
  .fc-note {{ box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.6), 0 2px 4px rgba(0, 0, 0, 0.25); }}
  .fc-swatch {{ box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.5); }}
}}
"
    );
    for name in COLORS {
        let hex = color_hex(name);
        out.push_str(&format!(
            ".fc-note-{name} {{ background-color: {hex}; }}\n.fc-swatch-{name} {{ background-color: {hex}; }}\n"
        ));
    }
    for font in Font::ALL {
        out.push_str(&format!(
            ".fc-note-font-{} {{ font-family: {}; }}\n",
            font.name(),
            css_family(note_look::font_family(font))
        ));
    }
    for size in Size::ALL {
        out.push_str(&format!(
            ".fc-note-size-{} {{ font-size: {}px; }}\n",
            size.name(),
            size.type_px()
        ));
    }
    out
}

/// A Pango family list as CSS writes it: each named face quoted, the generic last one bare.
fn css_family(list: &str) -> String {
    const GENERIC: [&str; 5] = ["sans-serif", "serif", "monospace", "cursive", "fantasy"];
    list.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            if GENERIC.contains(&name) {
                name.to_string()
            } else {
                format!("\"{name}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

thread_local! {
    static BADGES: RefCell<Vec<gtk::Label>> = const { RefCell::new(Vec::new()) };
}

/// Keeps `badge` (the Board button's pill) at the board's count of notes with something NEW TO
/// READ (`BoardModel::badge` — never the sync cursor): live frames, a resync, a family switch, the
/// window shown in front of somebody.
pub fn follow_badge(badge: gtk::Label) {
    BADGES.with(|badges| badges.borrow_mut().push(badge));
    let shell = Shell::get();
    let follow = |connection: &Rc<Connection>| {
        connection
            .router
            .on_board_changed()
            .subscribe(|_| refresh_badges());
        connection
            .live
            .on_resynced()
            .subscribe(|_| refresh_badges());
        connection
            .session
            .on_changed()
            .subscribe(|_| refresh_badges());
    };
    if let Some(connection) = shell.connection() {
        follow(&connection);
    }
    shell.on_connection_changed().subscribe(move |_| {
        if let Some(connection) = Shell::get().connection() {
            follow(&connection);
        }
        refresh_badges();
    });
    refresh_badges();
}

/// Redraws every badge from the cache.
pub fn refresh_badges() {
    let count = Shell::try_get()
        .and_then(|shell| shell.connection())
        .filter(|connection| connection.session.state().can_chat())
        .and_then(|connection| model(&connection).badge().ok())
        .unwrap_or(0);
    let text = badge_text(count);
    BADGES.with(|badges| {
        for badge in badges.borrow().iter() {
            badge.set_text(&text);
            badge.set_visible(count > 0);
        }
    });
}

/// The pill's text: the count, never wider than "99+" (the tray's label rule).
pub fn badge_text(count: usize) -> String {
    fc_logic::tray_rules::count_text(i64::try_from(count).unwrap_or(i64::MAX)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheet_names_every_pastel_hand_and_size() {
        let sheet = css(false);
        for name in COLORS {
            assert!(sheet.contains(&format!(
                ".fc-note-{name} {{ background-color: {};",
                color_hex(name)
            )));
        }
        assert!(sheet.contains(".fc-note-font-mono { font-family: \"Ubuntu Sans Mono\", \"DejaVu Sans Mono\", \"Liberation Mono\", monospace; }"));
        assert!(sheet.contains(".fc-note-size-large { font-size: 15px; }"));
        assert_ne!(css(true), sheet);
        // Balanced braces: a stray one would make GTK drop the rest of the sheet.
        assert_eq!(sheet.matches('{').count(), sheet.matches('}').count());
    }

    #[test]
    fn the_badge_says_at_most_99_plus() {
        assert_eq!(badge_text(0), "");
        assert_eq!(badge_text(7), "7");
        assert_eq!(badge_text(100), "99+");
    }

    #[test]
    fn a_family_list_is_quoted_for_css() {
        assert_eq!(
            css_family("Comic Neue, URW Gothic, cursive"),
            "\"Comic Neue\", \"URW Gothic\", cursive"
        );
    }
}
