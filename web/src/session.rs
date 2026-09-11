//! Where the token lives, and how the tab remembers who is signed in.
//!
//! `sessionStorage`, NOT `localStorage` (docs/protocol.md, "A browser is a
//! client too"): the token is the whole credential, and one that outlives
//! the tab is one left behind on a shared machine. Closing the tab is a
//! sign-out.
//!
//! Every accessor is total. Storage can be absent or throw — a private
//! window, a browser configured to block site data — and a chat client that
//! panicked there would be a white page rather than a login form.

use serde::{Deserialize, Serialize};
use web_sys::window;

use crate::store::Outgoing;

const TOKEN_KEY: &str = "fc.token";

/// What this tab has written and the server has not confirmed — kept where
/// the token is, so a reload goes on sending it instead of losing it, and
/// closing the tab forgets it with the session (docs/protocol.md, "Sending
/// on an unreliable network": a message the sender believes they sent is
/// the worst failure there is).
const OUTBOX_KEY: &str = "fc.outbox";

/// The outbox as stored: whose it is, so its bubbles draw as theirs before
/// `/me` has answered, and the rows.
#[derive(Debug, Serialize, Deserialize)]
struct SavedOutbox {
    user_id: i64,
    rows: Vec<Outgoing>,
}

fn storage() -> Option<web_sys::Storage> {
    window()?.session_storage().ok()?
}

/// The board's two seen-marks, under the account's id — the ONE thing this
/// client keeps past the tab (docs/protocol.md, "A browser is a client
/// too"). Two numbers: how far this browser has shown that account the wall,
/// and nothing of what is on it. Kept for the tab alone they would count the
/// whole wall as new at every sign-in, and a badge that always cries wolf is
/// a badge nobody reads.
const BOARD_MARKS_KEY: &str = "fc.board.seen.";

fn lasting() -> Option<web_sys::Storage> {
    window()?.local_storage().ok()?
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedMarks {
    note_id: i64,
    content_seq: i64,
}

/// The marks this browser keeps for `user_id` — none at all (zero) for an
/// account it has never shown the board, which then counts the whole wall,
/// as an app does on its first launch.
/// Where `user_id`'s marks are kept — also what a `storage` event from
/// another tab names when it moves them.
pub fn board_marks_key(user_id: i64) -> String {
    format!("{BOARD_MARKS_KEY}{user_id}")
}

pub fn board_marks(user_id: i64) -> fc_text::board::Marks {
    let saved = lasting()
        .and_then(|storage| storage.get_item(&board_marks_key(user_id)).ok()?)
        .and_then(|json| serde_json::from_str::<SavedMarks>(&json).ok());
    saved
        .map(|saved| fc_text::board::Marks {
            note_id: saved.note_id,
            content_seq: saved.content_seq,
        })
        .unwrap_or_default()
}

/// Keep `marks` for `user_id` — never below what is already kept: two tabs
/// of one account both write, and neither may walk the other's back.
pub fn save_board_marks(user_id: i64, marks: fc_text::board::Marks) {
    if user_id == 0 {
        return;
    }
    let Some(storage) = lasting() else { return };
    let kept = fc_text::board::later(board_marks(user_id), marks);
    let saved = SavedMarks {
        note_id: kept.note_id,
        content_seq: kept.content_seq,
    };
    if let Ok(json) = serde_json::to_string(&saved) {
        let _ = storage.set_item(&board_marks_key(user_id), &json);
    }
}

pub fn token() -> Option<String> {
    storage()?
        .get_item(TOKEN_KEY)
        .ok()?
        .filter(|t| !t.is_empty())
}

pub fn set_token(token: &str) {
    if let Some(storage) = storage() {
        let _ = storage.set_item(TOKEN_KEY, token);
    }
}

pub fn clear() {
    if let Some(storage) = storage() {
        let _ = storage.remove_item(TOKEN_KEY);
        let _ = storage.remove_item(OUTBOX_KEY);
        let _ = storage.remove_item(AWAITING_KEY);
    }
}

/// The account whose join request this tab was waiting on. A refusal is
/// never said by the server — the request just vanishes from `/me`
/// (docs/protocol.md, `GET /me`) — so only a client that REMEMBERS waiting
/// can tell "declined" from "never asked", and a reload must not forget it.
const AWAITING_KEY: &str = "fc.join.awaiting";

pub fn awaiting_join() -> Option<i64> {
    storage()?.get_item(AWAITING_KEY).ok()??.parse().ok()
}

pub fn set_awaiting_join(user_id: Option<i64>) {
    let Some(storage) = storage() else { return };
    match user_id {
        Some(user_id) => {
            let _ = storage.set_item(AWAITING_KEY, &user_id.to_string());
        }
        None => {
            let _ = storage.remove_item(AWAITING_KEY);
        }
    }
}

/// Keep the outbox for a reload. An empty one is not kept at all.
pub fn save_outbox(user_id: i64, rows: &[Outgoing]) {
    let Some(storage) = storage() else { return };
    if rows.is_empty() || user_id == 0 {
        let _ = storage.remove_item(OUTBOX_KEY);
        return;
    }
    let saved = SavedOutbox {
        user_id,
        rows: rows.to_vec(),
    };
    if let Ok(json) = serde_json::to_string(&saved) {
        let _ = storage.set_item(OUTBOX_KEY, &json);
    }
}

/// The outbox a reload left: whose, and the rows. Nothing when there is
/// none, or when what is there cannot be read — a row this version cannot
/// read is not one it can send.
pub fn outbox() -> Option<(i64, Vec<Outgoing>)> {
    let json = storage()?.get_item(OUTBOX_KEY).ok()??;
    let saved: SavedOutbox = serde_json::from_str(&json).ok()?;
    (saved.user_id != 0 && !saved.rows.is_empty()).then_some((saved.user_id, saved.rows))
}

/// The origin this page was served from — which IS the server
/// (docs/protocol.md, "A browser is a client too").
///
/// Empty when there is no window at all, which is only the case under a
/// test harness; every caller treats that as "relative", which is what a
/// browser does with a path anyway.
pub fn origin() -> String {
    window()
        .and_then(|window| window.location().origin().ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn row(client_msg_id: &str) -> Outgoing {
        Outgoing {
            chat_id: 42,
            client_msg_id: client_msg_id.into(),
            body: format!("body of {client_msg_id}"),
            reply_to_message_id: Some(7),
            mentions: Vec::new(),
            poll: Some(vec!["Yes".into(), "No".into()]),
            items: Vec::new(),
            attempts: 2,
            failed: None,
        }
    }

    /// The marks outlive a sign-out (that is what they are for), belong to
    /// one account, only ever rise, and read as zero where there are none
    /// or where what is there cannot be read.
    #[wasm_bindgen_test]
    fn the_board_marks_outlive_the_tab_per_account_and_only_rise() {
        use fc_text::board::Marks;
        let key = |id: i64| format!("{BOARD_MARKS_KEY}{id}");
        let local = lasting().expect("local storage in the test browser");
        for id in [9001, 9002] {
            let _ = local.remove_item(&key(id));
        }
        assert_eq!(board_marks(9001), Marks::default());
        let high = Marks {
            note_id: 12,
            content_seq: 90,
        };
        save_board_marks(9001, high);
        assert_eq!(board_marks(9001), high);
        save_board_marks(
            9001,
            Marks {
                note_id: 20,
                content_seq: 40,
            },
        );
        assert_eq!(
            board_marks(9001),
            Marks {
                note_id: 20,
                content_seq: 90
            },
            "each only rises"
        );
        assert_eq!(
            board_marks(9002),
            Marks::default(),
            "another account has its own"
        );
        clear();
        assert_eq!(board_marks(9001).content_seq, 90, "a sign-out keeps them");
        let _ = local.set_item(&key(9002), "not json");
        assert_eq!(board_marks(9002), Marks::default());
        save_board_marks(0, high);
        assert_eq!(
            board_marks(0),
            Marks::default(),
            "nobody's marks are not kept"
        );
        for id in [9001, 9002] {
            let _ = local.remove_item(&key(id));
        }
    }

    /// What a reload finds is what was there — whose, and every row whole —
    /// and an empty outbox, or a sign-out, leaves nothing behind.
    #[wasm_bindgen_test]
    fn the_outbox_survives_a_reload_and_not_a_sign_out() {
        clear();
        save_outbox(7, &[row("a"), row("b")]);
        assert_eq!(outbox(), Some((7, vec![row("a"), row("b")])));

        save_outbox(7, &[]);
        assert_eq!(outbox(), None, "an empty outbox is not kept");
        assert_eq!(
            storage().and_then(|storage| storage.get_item(OUTBOX_KEY).ok().flatten()),
            None,
            "not even as an empty list"
        );

        save_outbox(0, &[row("a")]);
        assert_eq!(outbox(), None, "nobody's outbox is not kept");

        set_token("t");
        save_outbox(7, &[row("a")]);
        clear();
        assert_eq!(outbox(), None, "signing out forgets it");
        assert_eq!(token(), None);
    }
}
