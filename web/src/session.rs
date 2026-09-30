//! Where the token lives, and how the tab remembers who is signed in.
//!
//! `sessionStorage`, NOT `localStorage` (docs/protocol.md, "A browser is a
//! client too"): the token is the whole credential, and one that outlives
//! the tab is one left behind on a shared machine. Closing the tab is a
//! sign-out. Two small things are kept by the DEVICE instead, in
//! `localStorage`, and neither is a credential or a word anybody wrote: the
//! board's seen-marks, and which stickers were used last — which a sign-out
//! takes.
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

/// The board's two seen-marks, under the account's id — one of the two
/// things this client keeps past the tab (docs/protocol.md, "A browser is a
/// client too"; the other is the sticker recents below, which a sign-out
/// takes). Two numbers: how far this browser has shown that account the
/// wall, and nothing of what is on it. Kept for the tab alone they would count the
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

/// Which stickers this person used last on this DEVICE — the pack items'
/// ids, newest first, and nothing else (docs/protocol.md, "Sticker pack":
/// which stickers somebody used most recently is that device's own
/// business, and is never on the wire).
///
/// Kept in `localStorage`, beside the board's marks and NOT beside the
/// token: the rule every client keeps is that the recents are the
/// device's, survive a restart, and go at sign-out. `sessionStorage` is the
/// tab's — a browser closed and opened again would forget them, and a
/// second tab would never have had them. What stops them being left behind
/// on a shared machine is [`clear`], which takes them with the token; they
/// are one account's besides, and another account reads none of them.
///
/// Every access is guarded like every other here: `localStorage` can be
/// absent, and can THROW — on the accessor, on a read, on a write (a
/// private window, site data blocked, a quota of nought) — and each of
/// those is "nothing kept", never a panic.
const PACK_RECENTS_KEY: &str = "fc.pack.recent";

/// The recents as stored: whose they are, and the ids.
#[derive(Debug, Serialize, Deserialize)]
struct SavedRecents {
    user_id: i64,
    ids: Vec<i64>,
}

/// The recents this device keeps for `user_id` — none when it has sent no
/// sticker as that account, and none where storage is absent, blocked or
/// holds something that cannot be read.
pub fn pack_recents(user_id: i64) -> Vec<i64> {
    lasting()
        .and_then(|storage| storage.get_item(PACK_RECENTS_KEY).ok()?)
        .and_then(|json| serde_json::from_str::<SavedRecents>(&json).ok())
        .filter(|saved| saved.user_id == user_id && user_id != 0)
        .map(|mut saved| {
            saved.ids.truncate(fc_text::pack::RECENTS_MAX);
            saved.ids
        })
        .unwrap_or_default()
}

/// `id` was just sent: it goes first. Answers the list as it now stands,
/// which is what the panel draws — whether or not storage took it.
pub fn use_pack_item(user_id: i64, id: i64) -> Vec<i64> {
    let ids = fc_text::pack::used(&pack_recents(user_id), id);
    if user_id != 0 {
        let saved = SavedRecents {
            user_id,
            ids: ids.clone(),
        };
        if let (Some(storage), Ok(json)) = (lasting(), serde_json::to_string(&saved)) {
            let _ = storage.set_item(PACK_RECENTS_KEY, &json);
        }
    }
    ids
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
    // The one thing a sign-out takes from `localStorage`: which stickers
    // this person reached for. The board's marks stay — that is what they
    // are for.
    if let Some(storage) = lasting() {
        let _ = storage.remove_item(PACK_RECENTS_KEY);
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
            sticker: false,
            attempts: 2,
            failed: None,
        }
    }

    /// THE STICKER RECENTS ARE THE DEVICE'S: the last sixteen, kept where a
    /// restart finds them and gone at sign-out — the same on every client.
    /// They were once in `sessionStorage`, where closing the browser
    /// forgot them and a second tab never had them. So: they are written
    /// to `localStorage` and nothing of them to `sessionStorage`, they are
    /// still there when everything the TAB held is gone (which is what a
    /// restart is), they are one account's, there are never more than
    /// sixteen, and a sign-out takes them.
    #[wasm_bindgen_test]
    fn the_sticker_recents_are_kept_for_the_device_and_go_at_sign_out() {
        let tab = storage().expect("session storage in the test browser");
        let local = lasting().expect("local storage in the test browser");
        let was = local.get_item(PACK_RECENTS_KEY).expect("reads");
        let _ = local.remove_item(PACK_RECENTS_KEY);
        // What other tests of this page had in the tab, to put back.
        let (token, outbox, awaiting) = (
            tab.get_item(TOKEN_KEY).expect("reads"),
            tab.get_item(OUTBOX_KEY).expect("reads"),
            tab.get_item(AWAITING_KEY).expect("reads"),
        );
        let tab_keys = || -> Vec<String> {
            (0..tab.length().expect("a length"))
                .filter_map(|index| tab.key(index).expect("a key"))
                .filter(|key| key.starts_with("fc.pack"))
                .collect()
        };

        assert!(pack_recents(9101).is_empty());
        assert_eq!(use_pack_item(9101, 5), vec![5]);
        assert_eq!(use_pack_item(9101, 8), vec![8, 5]);
        assert_eq!(use_pack_item(9101, 5), vec![5, 8], "once, and first");
        assert!(
            local.get_item(PACK_RECENTS_KEY).expect("reads").is_some(),
            "kept by the device"
        );
        assert_eq!(tab_keys(), Vec::<String>::new(), "and not by the tab");

        // A RESTART: everything the tab held is gone — the token with it —
        // and the recents are read back as they were.
        let held: Vec<String> = (0..tab.length().expect("a length"))
            .filter_map(|index| tab.key(index).expect("a key"))
            .filter(|key| key.starts_with("fc."))
            .collect();
        for key in held {
            tab.remove_item(&key).expect("removes");
        }
        assert!(super::token().is_none(), "the tab's session is over");
        assert_eq!(pack_recents(9101), vec![5, 8], "a restart keeps them");
        assert!(pack_recents(9102).is_empty(), "another account has none");

        // The last sixteen, and no more — also of a list somebody else
        // wrote longer.
        for id in 100..130 {
            use_pack_item(9101, id);
        }
        let kept = pack_recents(9101);
        assert_eq!(kept.len(), fc_text::pack::RECENTS_MAX);
        assert_eq!(kept[0], 129, "newest first");
        assert_eq!(kept[15], 114, "the seventeenth went");
        let long: Vec<i64> = (1..=40).collect();
        local
            .set_item(
                PACK_RECENTS_KEY,
                &serde_json::to_string(&SavedRecents {
                    user_id: 9101,
                    ids: long,
                })
                .expect("writes"),
            )
            .expect("writes");
        assert_eq!(pack_recents(9101).len(), fc_text::pack::RECENTS_MAX);

        // Another account on the same device starts its own, and the first
        // one's are not handed to it.
        assert_eq!(use_pack_item(9102, 3), vec![3]);
        assert!(pack_recents(9101).is_empty());

        local
            .set_item(PACK_RECENTS_KEY, "{not a list")
            .expect("writes");
        assert!(pack_recents(9102).is_empty());

        // Nobody signed in keeps nothing — but the panel still gets its
        // order for this tab.
        let _ = local.remove_item(PACK_RECENTS_KEY);
        assert_eq!(use_pack_item(0, 3), vec![3]);
        assert!(local.get_item(PACK_RECENTS_KEY).expect("reads").is_none());

        // SIGNING OUT takes them with the token.
        use_pack_item(9101, 5);
        clear();
        assert!(pack_recents(9101).is_empty(), "gone at sign-out");
        assert!(local.get_item(PACK_RECENTS_KEY).expect("reads").is_none());

        for (key, value) in [
            (TOKEN_KEY, token),
            (OUTBOX_KEY, outbox),
            (AWAITING_KEY, awaiting),
        ] {
            if let Some(value) = value {
                let _ = tab.set_item(key, &value);
            }
        }
        if let Some(value) = was {
            let _ = local.set_item(PACK_RECENTS_KEY, &value);
        }
    }

    /// STORAGE CAN THROW, and every access to the recents is guarded: a
    /// browser whose `localStorage` refuses to be read, written or removed
    /// from keeps nothing, says nothing, and the panel still gets its order
    /// for as long as the tab is open. Made to throw here the way a blocked
    /// one does — from the methods themselves.
    #[wasm_bindgen_test]
    fn the_sticker_recents_survive_a_storage_that_throws() {
        use js_sys::{Function, Reflect};
        use wasm_bindgen::JsValue;
        let local = lasting().expect("local storage in the test browser");
        let _ = local.remove_item(PACK_RECENTS_KEY);
        assert_eq!(use_pack_item(9103, 4), vec![4]);

        let prototype = Reflect::get(
            &Reflect::get(&window().expect("a window"), &JsValue::from_str("Storage"))
                .expect("Storage"),
            &JsValue::from_str("prototype"),
        )
        .expect("its prototype");
        let throws = Function::new_no_args("throw new DOMException('blocked', 'SecurityError')");
        let methods = ["getItem", "setItem", "removeItem"];
        let originals: Vec<JsValue> = methods
            .iter()
            .map(|name| Reflect::get(&prototype, &JsValue::from_str(name)).expect("a method"))
            .collect();
        for name in methods {
            Reflect::set(&prototype, &JsValue::from_str(name), &throws).expect("replaced");
        }
        let (read, used, again) = (
            pack_recents(9103),
            use_pack_item(9103, 6),
            use_pack_item(9103, 7),
        );
        clear();
        for (name, original) in methods.iter().zip(&originals) {
            Reflect::set(&prototype, &JsValue::from_str(name), original).expect("put back");
        }

        assert!(read.is_empty(), "unreadable reads as nothing");
        assert_eq!(used, vec![6], "and the send still gets its order");
        assert_eq!(again, vec![7]);
        assert_eq!(
            pack_recents(9103),
            vec![4],
            "nothing was written, and nothing removed, while it threw"
        );
        let _ = local.remove_item(PACK_RECENTS_KEY);
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
