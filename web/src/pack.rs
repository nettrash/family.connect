//! The family's sticker pack as this tab holds it, and every rule about
//! changing it (docs/protocol.md, "Sticker pack"). Pure, like board.rs:
//! nothing here talks to the server or draws anything.
//!
//! "Sticker" here is the CHAT kind — a small picture sent as its own
//! message — and never a board note, which is what the word means in
//! board.rs. The wire calls the collection a PACK so that the two are never
//! confused, and so does this file.
//!
//! The cursor is the board's machinery unchanged, and the three rules
//! board.rs keeps on purpose are kept here for the same reasons:
//! - A FULL READ REPLACES what is held. It never returns tombstones, so an
//!   item it leaves out is an item that is gone — except one held at a
//!   `pack_seq` above the read's mark, which a frame brought after the read
//!   was taken.
//! - ONLY A FULL READ, A CATCH-UP PAGE AND A FRAME MOVE THE CURSOR, and a
//!   frame only once this connection has caught up. The item in the answer
//!   to this device's own add is evidence about that one item.
//! - AN ITEM SEEN REMOVED STAYS REMOVED. Ids are never reused, so an older
//!   copy arriving late is never allowed to bring one back.
//!
//! Where the pack differs from the board: it is drawn in the order its
//! items were ADDED (`id` ascending), not the order they last changed, and
//! it has nothing to count as unread — adding and removing notify nobody.

use std::collections::{HashMap, HashSet};

use fc_text::assistant_consent;
use fc_text::pack as rules;

use crate::model::{Assistant, Attachment, PackItem};

/// What a click on a sticker in the panel does, in the chat it was made in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Sent, at once: its own message.
    Send,
    /// The assistant's chat, and this member has not yet agreed that what
    /// they send there may go to the model: the consent question is asked
    /// INSTEAD of sending, exactly as for words typed there.
    Ask,
    /// The assistant's chat on a server that will not say who answers:
    /// nothing is sent there at all, a sticker no more than a word.
    Withheld,
}

/// Whether a sticker may be sent in this chat now (docs/protocol.md,
/// "Consenting to the assistant"). A sticker is offered in EVERY chat a
/// message can be sent in — the assistant's included, where it is a
/// photograph shown to the model — and there it goes through the question
/// any message there does, never around it: the same two tests the
/// composer makes of a typed body (views/composer.rs), asked of a message
/// with no words. Anywhere else a sticker reaches no model: it has no body
/// to say `@ai` in.
pub fn send_gate(is_ai_chat: bool, assistant: Option<&Assistant>, agreed: bool) -> Gate {
    let kind = if is_ai_chat { "ai" } else { "direct" };
    let processor = assistant.and_then(|assistant| assistant.processor.as_deref());
    if assistant_consent::is_required(kind, "", processor, agreed) {
        Gate::Ask
    } else if assistant_consent::is_withheld_from_an_unnamed_assistant(
        kind,
        "",
        assistant.is_some(),
        processor,
    ) {
        Gate::Withheld
    } else {
        Gate::Send
    }
}

/// The family's two ceilings, as `GET /families/mine` gives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// `max_pack_items`.
    pub items: i64,
    /// `max_pack_item_bytes` — which binds a sticker MESSAGE too.
    pub bytes: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pack {
    /// id → the live item as last applied.
    pub items: HashMap<i64, PackItem>,
    /// Every id this tab has seen removed.
    pub gone: HashSet<i64>,
    /// The sync cursor: every change at or below it has been applied.
    pub cursor: i64,
    /// Whether this session has read the whole pack. Until it has there is
    /// nothing to catch up FROM.
    pub loaded: bool,
    /// Whether the pack has caught up on the CURRENT connection. Until it
    /// has, a frame applies its item and moves no cursor.
    pub caught_up: bool,
    /// The `max_pack_seq` of the newest full read applied. Kept for the
    /// record; what a later read is tested against is the cursor, which is
    /// never below it (`apply_full`).
    pub full_mark: Option<i64>,
    /// The server's limits, and None on a server that predates packs —
    /// which is the whole capability check: no limits, no sticker button
    /// and no pack on the Family screen.
    pub limits: Option<Limits>,
    /// Which items this person used last on this DEVICE, newest first.
    /// Theirs and this browser's: never on the wire, kept across a restart
    /// and dropped at sign-out (session.rs).
    pub recents: Vec<i64>,
    /// Whose recents those are: 0 until `/me` has said who this is.
    pub recents_for: i64,
    /// A sticker on its way into the pack: one at a time, as a photo is
    /// pinned to the board one at a time.
    pub adding: bool,
}

impl Pack {
    /// Whether this server has packs at all.
    pub fn is_offered(&self) -> bool {
        self.limits.is_some()
    }

    /// One copy of one item, under the per-item guard: applied only when
    /// its `pack_seq` is above the one held — LIVE OR TOMBSTONE, the same
    /// test for both, as every client makes it. A removal takes a new seq,
    /// so the tombstone of an item held is always above it; one that is
    /// not is older than what is held, and changes nothing. Answers
    /// whether the pack changed.
    pub fn apply(&mut self, item: PackItem) -> bool {
        if self.gone.contains(&item.id) {
            return false;
        }
        if self
            .items
            .get(&item.id)
            .is_some_and(|held| item.pack_seq <= held.pack_seq)
        {
            return false;
        }
        if item.deleted {
            self.gone.insert(item.id);
            return self.items.remove(&item.id).is_some();
        }
        // A live item with nobody who added it, or no picture, is a server
        // fault; dropping it beats drawing an empty square nobody can send.
        if !item.is_usable() {
            return false;
        }
        self.items.insert(item.id, item);
        true
    }

    /// A live `pack_item` frame: applied, and the cursor moved — once this
    /// connection has caught up.
    pub fn apply_frame(&mut self, item: PackItem) {
        let seq = item.pack_seq;
        self.apply(item);
        if self.caught_up {
            self.cursor = self.cursor.max(seq);
        }
    }

    /// One page of the change feed: every item applied, tombstones
    /// included, and the cursor moved to the highest seq on it.
    pub fn apply_page(&mut self, page: Vec<PackItem>) {
        let high = page.iter().map(|item| item.pack_seq).max();
        for item in page {
            self.apply(item);
        }
        if let Some(high) = high {
            self.cursor = self.cursor.max(high);
        }
    }

    /// The whole pack, which REPLACES what is held: every held item the
    /// read left out is gone, unless it is held at a seq above the read's
    /// mark — a frame that landed after the read was taken. The cursor goes
    /// to the mark, which the server reads BEFORE the items. A read whose
    /// mark is below the CURSOR already applied changes nothing: it is
    /// older than what this tab holds — the older of two reads in flight,
    /// or one that a catch-up page or a caught-up frame has since passed —
    /// and replacing with it would bring back what was removed after it
    /// was taken. (The cursor is never below the newest full read's mark,
    /// so this is that test and more.)
    pub fn apply_full(&mut self, items: Vec<PackItem>, max_pack_seq: i64) {
        if max_pack_seq < self.cursor {
            return;
        }
        self.full_mark = Some(max_pack_seq);
        let listed: HashSet<i64> = items.iter().map(|item| item.id).collect();
        let left_out: Vec<i64> = self
            .items
            .values()
            .filter(|held| !listed.contains(&held.id) && held.pack_seq <= max_pack_seq)
            .map(|held| held.id)
            .collect();
        for id in left_out {
            self.forget(id);
        }
        for item in items {
            self.apply(item);
        }
        self.cursor = self.cursor.max(max_pack_seq);
        self.loaded = true;
    }

    /// An item known to be gone — this device's own removal was answered,
    /// or the server said there is no such item.
    pub fn forget(&mut self, id: i64) {
        self.items.remove(&id);
        self.gone.insert(id);
    }

    /// The pack in the order it was added to — `id` ascending, the order
    /// the Family screen lists it in.
    pub fn listed(&self) -> Vec<PackItem> {
        let mut items: Vec<PackItem> = self.items.values().cloned().collect();
        items.sort_by_key(|item| item.id);
        items
    }

    /// The pack as a PANEL shows it: what this device used last first, then
    /// the rest in the order added (fc_text::pack::ordered).
    pub fn panel(&self) -> Vec<PackItem> {
        let ids: Vec<i64> = self.items.keys().copied().collect();
        rules::ordered(&ids, &self.recents)
            .into_iter()
            .filter_map(|id| self.items.get(&id).cloned())
            .collect()
    }

    /// Whether one more may be added, as far as this tab knows — refused
    /// beside the picker rather than by a rejected request. The server's
    /// `pack_full` is still the rule.
    pub fn has_room(&self) -> bool {
        self.limits
            .is_some_and(|limits| rules::has_room(self.items.len(), limits.items))
    }

    /// The items a sticker in a chat COULD be a copy of: the same size and
    /// type. The bytes decide among them, and they are whoever draws it to
    /// compare (docs/protocol.md, "Tapping one shows it larger").
    pub fn candidates(&self, sticker: &Attachment) -> Vec<PackItem> {
        let mut found: Vec<PackItem> = self
            .items
            .values()
            .filter(|item| {
                item.attachment.as_ref().is_some_and(|held| {
                    rules::could_be(
                        sticker.size,
                        sticker.mime.as_deref(),
                        held.size,
                        held.mime.as_deref(),
                    )
                })
            })
            .cloned()
            .collect();
        found.sort_by_key(|item| item.id);
        found
    }

    /// The recents kept for `user_id`, taken on once `/me` has said who
    /// this is — which is how a reload gets its order back — and again at
    /// every resync. Replaced rather than merged, with one exception:
    /// NOTHING kept, for the account already held, is a browser that keeps
    /// nothing (storage blocked), and there the order this tab has built
    /// up in memory is all there is.
    pub fn take_recents(&mut self, user_id: i64, kept: Vec<i64>) {
        if self.recents_for == user_id && kept.is_empty() {
            return;
        }
        self.recents = kept;
        self.recents_for = user_id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn picture(id: i64, size: i64, mime: &str) -> Attachment {
        Attachment {
            id,
            kind: "photo".into(),
            mime: Some(mime.into()),
            size: Some(size),
            ..Attachment::default()
        }
    }

    fn item(id: i64, pack_seq: i64) -> PackItem {
        PackItem {
            id,
            pack_seq,
            added_by: Some(7),
            attachment: Some(picture(700 + id, 1_000 + id, "image/webp")),
            ..PackItem::default()
        }
    }

    fn tombstone(id: i64, pack_seq: i64) -> PackItem {
        PackItem {
            id,
            pack_seq,
            deleted: true,
            ..PackItem::default()
        }
    }

    fn ids(pack: &Pack) -> Vec<i64> {
        pack.listed().iter().map(|item| item.id).collect()
    }

    fn assistant(processor: Option<&str>) -> Assistant {
        Assistant {
            user_id: 2,
            display_name: "Assistant".into(),
            mention: Some("@ai".into()),
            draw: Some("/draw".into()),
            vision: true,
            images: false,
            processor: processor.map(str::to_string),
        }
    }

    /// A STICKER IN THE ASSISTANT'S CHAT GOES THROUGH THE CONSENT QUESTION,
    /// never around it: asked while this member has not agreed, sent once
    /// they have, and held back entirely where the server names nobody.
    /// Everywhere else — the family chat, a one-to-one chat, a thread of
    /// either — it is sent at once: a sticker has no words to say `@ai` in.
    #[wasm_bindgen_test]
    fn a_sticker_to_the_assistant_is_asked_about_first() {
        let named = assistant(Some("Microsoft — Azure OpenAI"));
        assert_eq!(send_gate(true, Some(&named), false), Gate::Ask);
        assert_eq!(send_gate(true, Some(&named), true), Gate::Send);
        let unnamed = assistant(None);
        for agreed in [false, true] {
            assert_eq!(send_gate(true, Some(&unnamed), agreed), Gate::Withheld);
            assert_eq!(
                send_gate(true, Some(&assistant(Some("  "))), agreed),
                Gate::Withheld
            );
            // Between people nothing is asked, whatever the server has.
            for assistant in [None, Some(&named), Some(&unnamed)] {
                assert_eq!(send_gate(false, assistant, agreed), Gate::Send);
            }
        }
    }

    /// An out-of-order copy changes nothing: the newer one is held.
    #[wasm_bindgen_test]
    fn an_item_is_written_only_by_a_higher_seq() {
        let mut pack = Pack::default();
        assert!(pack.apply(item(1, 10)));
        assert!(!pack.apply(item(1, 10)), "the same one again");
        assert!(!pack.apply(item(1, 9)), "older");
        let mut relabelled = item(1, 12);
        relabelled.label = Some("party cat".into());
        assert!(pack.apply(relabelled));
        assert_eq!(pack.items[&1].label.as_deref(), Some("party cat"));
    }

    /// Removed is removed: a tombstone takes the item out, and no older
    /// copy — a page, a frame, an answer already in flight — brings it back.
    #[wasm_bindgen_test]
    fn an_item_seen_removed_never_comes_back() {
        let mut pack = Pack::default();
        pack.apply(item(1, 10));
        assert!(pack.apply(tombstone(1, 20)));
        assert!(pack.items.is_empty());
        assert!(!pack.apply(item(1, 15)), "an older live copy");
        assert!(
            !pack.apply(item(1, 25)),
            "any live copy: ids are never reused"
        );
        // A tombstone for an item never held is remembered all the same.
        assert!(!pack.apply(tombstone(2, 30)));
        assert!(!pack.apply(item(2, 5)));
        // And this device's own removal is as final.
        pack.apply(item(3, 40));
        pack.forget(3);
        assert!(!pack.apply(item(3, 41)));
        assert!(pack.items.is_empty());
    }

    /// THE SAME GUARD FOR A TOMBSTONE: one at or below the seq held is
    /// older than the item it names, and takes nothing out. (A removal
    /// takes a NEW seq, so the real tombstone is always above.)
    #[wasm_bindgen_test]
    fn a_tombstone_is_applied_only_above_the_seq_held() {
        let mut pack = Pack::default();
        pack.apply(item(1, 10));
        assert!(!pack.apply(tombstone(1, 10)), "at the seq held");
        assert!(!pack.apply(tombstone(1, 9)), "below it");
        assert!(pack.items.contains_key(&1), "still in the pack");
        assert!(!pack.gone.contains(&1), "and not remembered as removed");
        // A frame carrying it is no different, and moves only the cursor.
        pack.caught_up = true;
        pack.apply_frame(tombstone(1, 9));
        assert!(pack.items.contains_key(&1));
        assert!(pack.apply(tombstone(1, 11)), "above it: removed");
        assert!(pack.items.is_empty() && pack.gone.contains(&1));
    }

    /// A live copy missing what every live item has is a server fault, and
    /// is refused rather than drawn as an empty square.
    #[wasm_bindgen_test]
    fn a_broken_live_item_is_refused() {
        let mut pack = Pack::default();
        let mut nobody = item(1, 10);
        nobody.added_by = None;
        assert!(!pack.apply(nobody));
        let mut nothing = item(2, 10);
        nothing.attachment = None;
        assert!(!pack.apply(nothing));
        assert!(pack.items.is_empty());
        assert!(pack.gone.is_empty(), "refused is not removed");
        // Wherever it arrives: in a full read, on a catch-up page, in a
        // frame — dropped, not drawn, and the items beside it are kept.
        let broken = |id: i64, seq: i64| {
            let mut broken = item(id, seq);
            broken.attachment = None;
            broken
        };
        pack.apply_full(vec![item(3, 11), broken(4, 12)], 12);
        pack.apply_page(vec![broken(5, 13), item(6, 14)]);
        pack.caught_up = true;
        pack.apply_frame(broken(7, 15));
        assert_eq!(ids(&pack), vec![3, 6]);
        assert!(pack.panel().iter().all(PackItem::is_usable));
        assert_eq!(pack.cursor, 15, "the cursor moves all the same");
        // A broken copy of an item HELD does not take its picture away.
        pack.apply_frame(broken(3, 16));
        assert!(pack.items[&3].attachment.is_some());
    }

    /// A full read REPLACES: an item it leaves out is gone — and an item a
    /// frame brought after the read was taken is not.
    #[wasm_bindgen_test]
    fn a_full_read_replaces_what_is_held() {
        let mut pack = Pack::default();
        pack.apply(item(1, 10));
        pack.apply(item(2, 11));
        pack.apply(item(3, 60)); // a frame, after the read was taken
        pack.apply_full(vec![item(2, 11), item(4, 30)], 50);
        assert_eq!(ids(&pack), vec![2, 3, 4]);
        assert!(pack.gone.contains(&1), "left out is gone");
        assert!(!pack.apply(item(1, 10)), "and stays gone");
        assert_eq!(pack.cursor, 50);
        assert!(pack.loaded);
        // An untouched pack reads as nothing at mark 0 — and is loaded.
        let mut fresh = Pack::default();
        fresh.apply_full(Vec::new(), 0);
        assert!(fresh.loaded && fresh.items.is_empty());
        assert_eq!(fresh.cursor, 0);
    }

    /// An older full read landing after a newer one must not bring back an
    /// item removed between them — even one this tab never held.
    #[wasm_bindgen_test]
    fn an_older_full_read_landing_second_changes_nothing() {
        let mut pack = Pack::default();
        pack.apply_full(vec![item(1, 10)], 60);
        pack.apply_full(vec![item(1, 10), item(2, 20)], 50);
        assert_eq!(ids(&pack), vec![1], "the removed item did not come back");
        assert_eq!(pack.cursor, 60, "the cursor never walks back");
        // A read at the same mark is the same pack, and applies.
        pack.apply_full(vec![item(1, 10), item(3, 55)], 60);
        assert_eq!(ids(&pack), vec![1, 3]);
    }

    /// A FULL READ BELOW THE CURSOR IS IGNORED, however the cursor got
    /// there: a catch-up page or a caught-up frame that passed the read
    /// while it was in flight is newer than it, and the read would bring
    /// back what they removed — an item this tab never held, which the
    /// gone set cannot know about.
    #[wasm_bindgen_test]
    fn a_full_read_below_the_cursor_is_ignored() {
        let mut pack = Pack::default();
        pack.apply_full(vec![item(1, 10)], 40);
        pack.apply_page(vec![item(2, 45), tombstone(9, 70)]);
        assert_eq!(pack.cursor, 70);
        pack.apply_full(vec![item(1, 10), item(8, 30)], 50);
        assert_eq!(
            ids(&pack),
            vec![1, 2],
            "the page's item stayed, 8 never came"
        );
        assert_eq!(pack.cursor, 70);
        assert_eq!(pack.full_mark, Some(40), "not applied, so not recorded");
        // By a frame, once this connection has caught up.
        pack.caught_up = true;
        pack.apply_frame(item(3, 80));
        pack.apply_full(vec![item(1, 10)], 79);
        assert_eq!(ids(&pack), vec![1, 2, 3]);
        // A frame BEFORE the catch-up moves no cursor, so the read that is
        // that catch-up still applies — and keeps the frame's item, which
        // is above its mark.
        let mut fresh = Pack::default();
        fresh.apply_frame(item(5, 90));
        fresh.apply_full(vec![item(4, 20)], 85);
        assert_eq!(ids(&fresh), vec![4, 5]);
        assert_eq!(fresh.cursor, 85);
        // At the cursor exactly is not below it.
        fresh.apply_full(vec![item(4, 20), item(5, 90)], 85);
        assert_eq!(ids(&fresh), vec![4, 5]);
    }

    /// The cursor moves three ways and no others: a full read, a catch-up
    /// page, and a frame once this connection has caught up.
    #[wasm_bindgen_test]
    fn the_cursor_moves_three_ways_and_no_others() {
        let mut pack = Pack::default();
        pack.apply_frame(item(1, 130));
        assert_eq!(pack.cursor, 0, "before the catch-up: applied, cursor still");
        assert!(pack.items.contains_key(&1));
        pack.apply_page(vec![item(2, 101), tombstone(3, 104)]);
        assert_eq!(pack.cursor, 104, "the highest seq on the page");
        assert!(pack.gone.contains(&3), "a tombstone on a page is applied");
        pack.caught_up = true;
        pack.apply_frame(item(4, 140));
        assert_eq!(pack.cursor, 140);
        pack.apply_frame(tombstone(4, 141));
        assert_eq!(pack.cursor, 141, "a removal's frame moves it too");
        assert!(!pack.items.contains_key(&4));
        pack.apply_page(vec![item(5, 120)]);
        assert_eq!(pack.cursor, 141, "never back");
        pack.apply_page(Vec::new());
        assert_eq!(pack.cursor, 141);
        // The item in the answer to this device's own POST is `apply`
        // alone: under the per-item guard, moving nothing.
        assert!(pack.apply(item(6, 200)));
        assert_eq!(pack.cursor, 141);
    }

    /// The Family screen lists the pack in the order it was added to; a
    /// panel puts what this device used last first.
    #[wasm_bindgen_test]
    fn the_pack_is_listed_as_added_and_panelled_recents_first() {
        let mut pack = Pack::default();
        for (id, seq) in [(3, 30), (1, 40), (2, 20)] {
            pack.apply(item(id, seq));
        }
        assert_eq!(ids(&pack), vec![1, 2, 3], "by id, not by last change");
        let panel = |pack: &Pack| -> Vec<i64> { pack.panel().iter().map(|item| item.id).collect() };
        assert_eq!(panel(&pack), vec![1, 2, 3]);
        pack.take_recents(7, vec![3, 9, 2]);
        assert_eq!(panel(&pack), vec![3, 2, 1], "9 is not in the pack");
        pack.apply(tombstone(3, 50));
        assert_eq!(panel(&pack), vec![2, 1]);
        // A browser that keeps nothing does not wipe what this tab knows…
        pack.take_recents(7, Vec::new());
        assert_eq!(pack.recents, vec![3, 9, 2]);
        // …what storage does hold replaces it, and another account's
        // nothing is nothing.
        pack.take_recents(7, vec![1]);
        assert_eq!(panel(&pack), vec![1, 2]);
        pack.take_recents(8, Vec::new());
        assert!(pack.recents.is_empty());
        assert_eq!(pack.recents_for, 8);
    }

    /// No limits is a server from before packs, and offers nothing; with
    /// them, the ceiling is refused at the picker.
    #[wasm_bindgen_test]
    fn the_limits_are_the_capability_and_the_ceiling() {
        let mut pack = Pack::default();
        assert!(!pack.is_offered());
        assert!(!pack.has_room(), "nothing to add to on an older server");
        pack.limits = Some(Limits {
            items: 2,
            bytes: 524_288,
        });
        assert!(pack.is_offered() && pack.has_room());
        pack.apply(item(1, 10));
        pack.apply(item(2, 11));
        assert!(!pack.has_room(), "full");
        pack.apply(tombstone(1, 12));
        assert!(pack.has_room(), "a removal makes room");
    }

    /// A sticker in a chat could be a pack item only of the same size and
    /// type; which of them, the bytes say.
    #[wasm_bindgen_test]
    fn only_same_sized_same_typed_items_are_candidates() {
        let mut pack = Pack::default();
        pack.apply(item(1, 10)); // 1001 bytes, webp
        pack.apply(item(2, 11)); // 1002 bytes, webp
        let mut png = item(3, 12);
        png.attachment = Some(picture(703, 1_001, "image/png"));
        pack.apply(png);
        let found = |sticker: &Attachment| -> Vec<i64> {
            pack.candidates(sticker)
                .iter()
                .map(|item| item.id)
                .collect()
        };
        assert_eq!(found(&picture(90, 1_001, "image/webp")), vec![1]);
        assert_eq!(found(&picture(90, 1_001, "image/png")), vec![3]);
        assert!(found(&picture(90, 9_999, "image/webp")).is_empty());
    }
}
