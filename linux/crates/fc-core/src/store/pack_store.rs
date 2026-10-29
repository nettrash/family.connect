//! The family's sticker pack, as this device holds it (docs/protocol.md, "Sticker pack") — a port
//! of `PackStore.cs`.
//!
//! **THE BOARD'S MACHINERY, UNCHANGED** — the protocol says so in as many words, and this is
//! [`BoardStore`](super::BoardStore) one table over. A FULL READ REPLACES what is held, except an
//! item held above the read's own mark, which arrived after the read was taken; an older full read
//! landing second is ignored outright. A REMOVAL IS REMEMBERED (`pack_gone`), so an older copy of
//! an item the family took out cannot put it back — whether the removal was seen as a tombstone,
//! as a full read that left the item out, or as this device's own `DELETE`. And every write is
//! guarded by `pack_seq`: an item, live or a tombstone, is applied only when the incoming seq is
//! greater than the one held.
//!
//! **THE CURSOR MOVES IN THREE WAYS AND NO OTHERS**: a full read sets it to its mark, a catch-up
//! page to the highest seq on the page, and a `pack_item` frame to its own — the frame ONLY ONCE
//! THIS CONNECTION HAS CAUGHT UP. A frame that moved it sooner would step the feed past every
//! change made while the socket was down: the device would hold the newest sticker and never learn
//! of the three added, or the one removed, before it. The answer to this device's own `POST` is
//! evidence about one item and moves nothing.
//!
//! **"STICKER" HERE IS THE CHAT ONE.** The board's cards are called stickers in this code; these
//! tables and this type say `pack`, as the wire does, so that nobody has to read a name twice to
//! know which is meant.
//!
//! **ONE PACK PER FAMILY** (docs/protocol.md, "Several families"): its own items, seqs, cursor,
//! ceilings and catch-up. A store reads and writes the pack of the family shown, and
//! [`PackStore::in_family`] is the same store held to one family; the connection's count and which
//! families have caught up on it are SHARED between them, because they are one socket's.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;
use std::rc::Rc;

use rusqlite::types::ToSql;
use rusqlite::{Connection, OptionalExtension, Row, named_params};
use time::OffsetDateTime;

use super::board_store::{meta, number, set_meta};
use super::chat_store::SeqRoute;
use super::database::{Database, StoreResult};
use super::{family_keys, times};
use crate::protocol::{AttachmentDto, PackItemDto, wire};

const CURSOR_SEQ: &str = "pack.cursor.pack_seq";

/// The `max_pack_seq` of the newest FULL read applied; an older one landing second changes
/// nothing.
const FULL_MARK: &str = "pack.full.pack_seq";

const LIMIT_ITEMS: &str = "pack.limit.items";
const LIMIT_BYTES: &str = "pack.limit.item_bytes";

/// Every key this store keeps in `meta`, before a family's suffix — what
/// [`Database::adopt_legacy`] moves.
pub(crate) const META_KEYS: [&str; 4] = [CURSOR_SEQ, FULL_MARK, LIMIT_ITEMS, LIMIT_BYTES];

/// How many recently used stickers the panel asks for when it names no number (the C#'s default
/// `limit` of `Recents`).
pub const RECENTS: usize = 16;

/// The pack's two ceilings, as `GET /families/mine` last gave them. Both or neither: a server that
/// has packs always sends both, and one that predates them sends neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PackLimits {
    pub max_items: i32,
    pub max_item_bytes: i64,
}

/// One socket's catch-up, shared by every family's store: which connection it is — counted up each
/// time a socket opens, so a pass can say which one it caught up WITH — and which families have
/// caught up on it. Deliberately not kept in the file: a relaunch is a new connection, and it has
/// not.
#[derive(Debug, Default)]
struct Link {
    connection: i64,
    /// The families caught up on this connection; 0 stands for the unscoped pack of an older
    /// cache.
    caught_up: HashSet<i64>,
}

/// The family's sticker pack, as this device holds it.
pub struct PackStore {
    database: Rc<Database>,
    family: Box<dyn Fn() -> Option<i64>>,
    link: Rc<RefCell<Link>>,
}

impl fmt::Debug for PackStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PackStore")
            .field("family", &self.family())
            .field("link", &self.link)
            .finish_non_exhaustive()
    }
}

impl PackStore {
    /// The one pack of a cache from before several families, on a connection of its own — `new
    /// PackStore(database)`.
    pub fn new(database: &Rc<Database>) -> Self {
        PackStore {
            database: Rc::clone(database),
            family: Box::new(|| None),
            link: Rc::new(RefCell::new(Link::default())),
        }
    }

    /// The same store, reading the pack of the family `current` says is shown (the C#'s
    /// `current`).
    pub fn with_current(mut self, current: impl Fn() -> Option<i64> + 'static) -> Self {
        self.family = Box::new(current);
        self
    }

    /// The family whose pack this is — the one shown, or the one [`in_family`](Self::in_family)
    /// held it to.
    pub fn family(&self) -> Option<i64> {
        (self.family)()
    }

    /// This store, held to ONE family's pack whatever is shown — sharing this connection's
    /// catch-up (the C#'s `In`).
    pub fn in_family(&self, family_id: Option<i64>) -> PackStore {
        PackStore {
            database: Rc::clone(&self.database),
            family: Box::new(move || family_id),
            link: Rc::clone(&self.link),
        }
    }

    fn key(&self, key: &str) -> String {
        family_keys::of(key, self.family())
    }

    fn sqlite(&self) -> &Connection {
        self.database.connection()
    }

    fn caught(&self) -> i64 {
        self.family().unwrap_or(0)
    }

    /// The clause that keeps a read to this store's family — ` WHERE family_id = $family`, or with
    /// `AND` after a clause of its own — and nothing for the unscoped pack of an older cache.
    fn only(&self, join: &str) -> String {
        match self.family() {
            None => String::new(),
            Some(_) => format!(" {join} family_id = $family"),
        }
    }

    /// The pack as it stands, in the order the items were added — the order a panel shows them
    /// in.
    pub fn items(&self) -> StoreResult<Vec<PackItemDto>> {
        let _held = self.database.hold();
        let family = self.family();
        let items = self
            .sqlite()
            .prepare_cached(&format!(
                "SELECT * FROM pack_items{} ORDER BY item_id ASC",
                self.only("WHERE")
            ))?
            .query_map(scoped(&family, &[]).as_slice(), read)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(items)
    }

    /// One item, as this device holds it.
    pub fn item(&self, item_id: i64) -> StoreResult<Option<PackItemDto>> {
        let _held = self.database.hold();
        Ok(self
            .sqlite()
            .prepare_cached("SELECT * FROM pack_items WHERE item_id = $id")?
            .query_row(named_params! { "$id": item_id }, read)
            .optional()?)
    }

    /// How many items are held — what the pack's ceiling is checked against at the picker.
    pub fn count(&self) -> StoreResult<usize> {
        let _held = self.database.hold();
        let family = self.family();
        let count: i64 = self
            .sqlite()
            .prepare_cached(&format!(
                "SELECT COUNT(*) FROM pack_items{}",
                self.only("WHERE")
            ))?
            .query_row(scoped(&family, &[]).as_slice(), |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// A whole-pack read: what arrived IS the pack. Items the answer does not carry are gone —
    /// somebody removed them while this device was not looking — and are remembered as gone.
    /// Answers whether the read was applied; an older one landing second is not.
    pub fn replace(&self, items: &[PackItemDto], max_pack_seq: i64) -> StoreResult<bool> {
        let _held = self.database.hold();
        // TWO FULL READS CAN LAND IN EITHER ORDER, and an older one landing second must change
        // nothing at all: it would drop every item added between the two and set the cursor back.
        // Measured against the CURSOR ALREADY APPLIED, whichever of the three ways moved it — a
        // full read, a catch-up page, or a frame on a caught-up connection: a read taken before
        // any of those knows less than this device does, and would set the cursor back behind
        // changes it has already applied.
        let full = number(meta(&self.database, &self.key(FULL_MARK))?.as_deref());
        if max_pack_seq < full.max(self.cursor()?) {
            return Ok(false);
        }
        let family = self.family();
        let transaction = self.sqlite().unchecked_transaction()?;
        let arrived: HashSet<i64> = items
            .iter()
            .filter(|item| !item.deleted)
            .map(|item| item.id)
            .collect();
        // A FULL READ THAT LEFT AN ITEM OUT HAS SEEN IT REMOVED, and that is remembered like any
        // tombstone — but only for what the read could have known about. An item held ABOVE the
        // read's mark arrived after the read was taken: it is not on that answer, and it stays.
        let left: Vec<i64> = transaction
            .prepare_cached(&format!(
                "SELECT item_id FROM pack_items WHERE pack_seq <= $mark{}",
                self.only("AND")
            ))?
            .query_map(
                scoped(&family, &[("$mark", &max_pack_seq as &dyn ToSql)]).as_slice(),
                |row| row.get(0),
            )?
            .collect::<Result<Vec<i64>, _>>()?
            .into_iter()
            .filter(|id| !arrived.contains(id))
            .collect();
        for id in left {
            remember(&transaction, id)?;
        }
        transaction
            .prepare_cached(&format!(
                "DELETE FROM pack_items WHERE pack_seq <= $mark{}",
                self.only("AND")
            ))?
            .execute(scoped(&family, &[("$mark", &max_pack_seq as &dyn ToSql)]).as_slice())?;
        for item in items.iter().filter(|item| !item.deleted) {
            // Through the same guard as everything else: never over a newer copy, never a
            // resurrection.
            self.apply_one(&transaction, item)?;
        }
        set_meta(&transaction, &self.key(CURSOR_SEQ), max_pack_seq)?;
        set_meta(&transaction, &self.key(FULL_MARK), max_pack_seq)?;
        transaction.commit()?;
        Ok(true)
    }

    /// One item: from a frame, from the change feed, or from the answer to this device's own add
    /// or removal. A TOMBSTONE removes; anything not newer than what is held is dropped. Answers
    /// whether the pack, as drawn, changed. (The C#'s default route: `CatchUpPage`.)
    pub fn apply(&self, item: &PackItemDto, route: SeqRoute) -> StoreResult<bool> {
        let _held = self.database.hold();
        let transaction = self.sqlite().unchecked_transaction()?;
        let changed = self.apply_one(&transaction, item)?;
        let may_move = match route {
            // The item in a POST's answer, or the tombstone this device makes of its own DELETE:
            // one item's news, which says nothing about what else has changed.
            SeqRoute::Evidence => false,
            // ONLY ONCE THIS CONNECTION HAS CAUGHT UP. Before that the cursor is where the catch-up
            // starts from, and a frame that jumped it would leave everything between the two unread
            // for good (docs/protocol.md, "The cursor moves in three ways and no others").
            SeqRoute::LiveFrame => self.link.borrow().caught_up.contains(&self.caught()),
            SeqRoute::CatchUpPage => true,
        };
        if may_move && item.pack_seq > self.cursor()? {
            set_meta(&transaction, &self.key(CURSOR_SEQ), item.pack_seq)?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    /// A page of the change feed, applied in order; the cursor follows the page's highest seq.
    /// Answers how many items changed what is drawn.
    pub fn apply_page(&self, items: &[PackItemDto], route: SeqRoute) -> StoreResult<usize> {
        let _held = self.database.hold();
        let transaction = self.sqlite().unchecked_transaction()?;
        let mut changed = 0;
        let mut highest = self.cursor()?;
        for item in items {
            if self.apply_one(&transaction, item)? {
                changed += 1;
            }
            highest = highest.max(item.pack_seq);
        }
        if route != SeqRoute::Evidence {
            set_meta(&transaction, &self.key(CURSOR_SEQ), highest)?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    fn apply_one(&self, connection: &Connection, item: &PackItemDto) -> StoreResult<bool> {
        // ONE GUARD FOR BOTH STATES: an item — live or a tombstone — is applied only when its
        // `pack_seq` is ABOVE the one held. A removal always takes a newer seq than the add it
        // removes, so this never holds a real tombstone back; what it stops is a copy that is not
        // newer than what this device already has.
        let held: Option<Option<i64>> = connection
            .prepare_cached("SELECT pack_seq FROM pack_items WHERE item_id = $id")?
            .query_row(named_params! { "$id": item.id }, |row| row.get(0))
            .optional()?;
        if held.flatten().is_some_and(|mine| mine >= item.pack_seq) {
            return Ok(false);
        }
        if item.deleted {
            // Whether the PANEL changed, which a tombstone for an item this device never held did
            // not — remembering it is not news.
            return remove(connection, item.id);
        }
        let Some(picture) = item.attachment.as_ref().filter(|_| item.added_by > 0) else {
            // A live item IS its picture, and it always says who added it — that is what decides
            // who may remove it. One missing either is not something this client can draw, send
            // or offer a removal for: it is dropped, not stored as a blank square in the panel.
            return Ok(false);
        };
        if is_gone(connection, item.id)? {
            // A removal is the last word: this is an older copy that crossed it on the wire.
            return Ok(false);
        }
        self.write(connection, item, picture)?;
        Ok(true)
    }

    /// This device's own `DELETE` was answered — `204`, or `pack_item_not_found`, which says the
    /// same thing: the item is not in the pack. It leaves the panel as the click lands and is
    /// remembered as gone, like a tombstone. Answers whether the pack, as drawn, changed.
    ///
    /// NOT THROUGH [`apply`](Self::apply), and NO CURSOR: the seq the removal was given is the
    /// server's to announce, in the frame and on the change feed, and this device does not know
    /// it. A tombstone invented here would either fail the seq guard or move the cursor to a number
    /// nobody issued.
    pub fn removed(&self, item_id: i64) -> StoreResult<bool> {
        let _held = self.database.hold();
        let transaction = self.sqlite().unchecked_transaction()?;
        let changed = remove(&transaction, item_id)?;
        transaction.commit()?;
        Ok(changed)
    }

    /// Where the change feed should carry on from; 0 until this device has read the pack at all.
    pub fn cursor(&self) -> StoreResult<i64> {
        Ok(number(
            meta(&self.database, &self.key(CURSOR_SEQ))?.as_deref(),
        ))
    }

    // ---- one connection's catch-up -------------------------------------------------------------

    /// A connection has OPENED: frames on it may not move the cursor until a pass that BEGAN on it
    /// has read what was missed while the socket was down.
    pub fn reconnected(&self) {
        let _held = self.database.hold();
        let mut link = self.link.borrow_mut();
        link.connection += 1;
        link.caught_up.clear();
    }

    /// The connection a pass is beginning on — taken before its first request and handed back to
    /// [`caught_up`](Self::caught_up), which is how a pass that a reconnect overtook is told apart
    /// from one that was not.
    pub fn connection(&self) -> i64 {
        let _held = self.database.hold();
        self.link.borrow().connection
    }

    /// The pass reached the pack and is level with the server: frames may move the cursor from
    /// here — IF the connection the pass began on (`began`, what
    /// [`connection`](Self::connection) answered then) is still the one that is
    /// open. Answers whether the connection is now caught up; false when a newer one has opened
    /// since.
    ///
    /// **A PASS CATCHES UP THE CONNECTION IT BEGAN ON, AND NO OTHER.** A socket that drops and
    /// reopens while a pass is waiting on a request leaves that pass holding an answer taken before
    /// the new socket was listening: a change committed in between is on neither the answer nor
    /// the wire. Were that pass allowed to say "caught up", the next frame would step the cursor
    /// over the change, the pass the new connection started would find the cursor level with the
    /// server's mark and ask for nothing, and the change would be lost for good — a full read only
    /// ever happens at a cursor of 0. So an overtaken pass's word is dropped, and the pass the new
    /// connection started (every connection starts one) is the one that says it.
    pub fn caught_up(&self, began: i64) -> bool {
        let _held = self.database.hold();
        let mut link = self.link.borrow_mut();
        if began != link.connection {
            return false;
        }
        link.caught_up.insert(self.caught());
        true
    }

    /// Whether this connection has caught up.
    pub fn is_caught_up(&self) -> bool {
        let _held = self.database.hold();
        self.link.borrow().caught_up.contains(&self.caught())
    }

    // ---- the ceilings --------------------------------------------------------------------------

    /// The pack's ceilings as `GET /families/mine` last gave them, or `None` on a server that
    /// predates packs — which is how the window knows to offer no sticker button and no pack
    /// management there (docs/protocol.md, "What old clients and old servers do"). Kept in the
    /// cache so the answer is there before the first read of a launch, and offline.
    pub fn limits(&self) -> StoreResult<Option<PackLimits>> {
        let _held = self.database.hold();
        let items = meta(&self.database, &self.key(LIMIT_ITEMS))?;
        let bytes = meta(&self.database, &self.key(LIMIT_BYTES))?;
        Ok(match (items, bytes) {
            (Some(items), Some(bytes)) => Some(PackLimits {
                // The C#'s `(int)Math.Min(n, int.MaxValue)`: capped above, and — an unchecked cast —
                // wrapped below, which only a damaged cache could ask of it.
                max_items: number(Some(&items)).min(i64::from(i32::MAX)) as i32,
                max_item_bytes: number(Some(&bytes)),
            }),
            _ => None,
        })
    }

    /// What the family's own document said this time — `None` when it said nothing, which is an
    /// answer too.
    pub fn set_limits(&self, limits: Option<PackLimits>) -> StoreResult<()> {
        let _held = self.database.hold();
        let transaction = self.sqlite().unchecked_transaction()?;
        match limits {
            Some(known) => {
                set_meta(
                    &transaction,
                    &self.key(LIMIT_ITEMS),
                    i64::from(known.max_items),
                )?;
                set_meta(&transaction, &self.key(LIMIT_BYTES), known.max_item_bytes)?;
            }
            None => {
                transaction
                    .prepare_cached("DELETE FROM meta WHERE key IN ($items, $bytes)")?
                    .execute(named_params! {
                        "$items": self.key(LIMIT_ITEMS),
                        "$bytes": self.key(LIMIT_BYTES),
                    })?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    // ---- what this device used last --------------------------------------------------------------

    /// This device just sent that item. Never on the wire.
    pub fn used(&self, item_id: i64, at: OffsetDateTime) -> StoreResult<()> {
        let _held = self.database.hold();
        self.sqlite()
            .prepare_cached(
                "INSERT INTO pack_recents (item_id, used_at) VALUES ($id, $at) ON CONFLICT(item_id) DO UPDATE SET used_at = excluded.used_at",
            )?
            .execute(named_params! { "$id": item_id, "$at": times::milliseconds(at) })?;
        Ok(())
    }

    /// The items this device sent most recently, newest first — only ones the pack still holds.
    /// Kept in the cache file, so they are THIS DEVICE's and survive a restart; and gone with
    /// everything else at sign-out ([`Database::wipe_all`]). (The C#'s default `limit` is
    /// [`RECENTS`].)
    pub fn recents(&self, limit: usize) -> StoreResult<Vec<i64>> {
        let _held = self.database.hold();
        let family = self.family();
        let scope = if family.is_none() {
            ""
        } else {
            "WHERE i.family_id = $family "
        };
        let ids = self
            .sqlite()
            .prepare_cached(&format!(
                "SELECT r.item_id FROM pack_recents r JOIN pack_items i ON i.item_id = r.item_id {scope}ORDER BY r.used_at DESC, r.item_id DESC LIMIT $limit"
            ))?
            .query_map(scoped(&family, &[("$limit", &limit as &dyn ToSql)]).as_slice(), |row| {
                row.get(0)
            })?
            .collect::<Result<Vec<i64>, _>>()?;
        Ok(ids)
    }

    // ---- rows ------------------------------------------------------------------------------

    fn write(
        &self,
        connection: &Connection,
        item: &PackItemDto,
        picture: &AttachmentDto,
    ) -> StoreResult<()> {
        connection
            .prepare_cached(
                "INSERT INTO pack_items (item_id, added_by, label, created_at, pack_seq, attachment_json, family_id)
VALUES ($id, $by, $label, $created, $seq, $attachment, $family)
ON CONFLICT(item_id) DO UPDATE SET
    added_by = excluded.added_by, label = excluded.label,
    created_at = excluded.created_at, pack_seq = excluded.pack_seq,
    attachment_json = excluded.attachment_json,
    family_id = COALESCE(excluded.family_id, pack_items.family_id)",
            )?
            .execute(named_params! {
                // Whose pack: the family this store is held to, else what the item says.
                "$family": self.family().or(item.family_id),
                "$id": item.id,
                "$by": item.added_by,
                // Absent and empty are the same answer here: the server never sends an empty
                // label.
                "$label": item.label.as_deref().filter(|label| !label.is_empty()),
                "$created": times::instant(item.created_at.as_deref()).unwrap_or(0),
                "$seq": item.pack_seq,
                "$attachment": wire::encode(picture),
            })?;
        Ok(())
    }
}

/// The parameters a statement built with [`PackStore::only`] needs: `fixed`, and `$family` when
/// the store is held to one.
fn scoped<'a>(
    family: &'a Option<i64>,
    fixed: &[(&'a str, &'a dyn ToSql)],
) -> Vec<(&'a str, &'a dyn ToSql)> {
    let mut parameters = fixed.to_vec();
    if let Some(family) = family {
        parameters.push(("$family", family as &dyn ToSql));
    }
    parameters
}

fn remove(connection: &Connection, item_id: i64) -> StoreResult<bool> {
    remember(connection, item_id)?;
    // A sticker that is gone cannot be "recently used" either.
    connection
        .prepare_cached("DELETE FROM pack_recents WHERE item_id = $id")?
        .execute(named_params! { "$id": item_id })?;
    let deleted = connection
        .prepare_cached("DELETE FROM pack_items WHERE item_id = $id")?
        .execute(named_params! { "$id": item_id })?;
    Ok(deleted > 0)
}

fn remember(connection: &Connection, item_id: i64) -> StoreResult<()> {
    connection
        .prepare_cached("INSERT OR IGNORE INTO pack_gone (item_id) VALUES ($id)")?
        .execute(named_params! { "$id": item_id })?;
    Ok(())
}

fn is_gone(connection: &Connection, item_id: i64) -> StoreResult<bool> {
    Ok(connection
        .prepare_cached("SELECT 1 FROM pack_gone WHERE item_id = $id")?
        .query_row(named_params! { "$id": item_id }, |_| Ok(()))
        .optional()?
        .is_some())
}

fn read(row: &Row<'_>) -> rusqlite::Result<PackItemDto> {
    let created: i64 = row.get("created_at")?;
    Ok(PackItemDto {
        id: row.get("item_id")?,
        added_by: row.get("added_by")?,
        attachment: row
            .get::<_, Option<String>>("attachment_json")?
            .and_then(|json| wire::decode::<AttachmentDto>(&json)),
        created_at: if created == 0 {
            None
        } else {
            times::rfc3339(Some(created))
        },
        pack_seq: row.get("pack_seq")?,
        label: row.get("label")?,
        deleted: false,
        family_id: row.get("family_id")?,
    })
}
