//! What a client does on every (re)connect (docs/protocol.md, "Best-effort delivery") — a port of
//! `win/src/FamilyConnect.Core/Protocol/Resync.cs`.
//!
//! The socket is a live wire, not a queue — a slow client's socket is dropped and REST is the
//! source of truth — so the order below is the protocol's own, and two of its rules are the kind
//! that only look small:
//!
//! **THE FLUSH IS NOT A STEP.** It is written fourth because it reads well fourth. The outbox is an
//! obligation ordered against nothing: a client that could not finish — or could not START — the
//! reads must flush anyway. Both of this app's own ports first implemented the list literally, with
//! the flush as the tail of a function that returned early whenever a read failed, which put the
//! one operation that recovers a stuck message behind the reads most likely to fail on the network
//! that stuck it.
//!
//! **THE MESSAGE CURSOR BELONGS TO THE LOOP.** It is read ONCE, before the first page, and then
//! advanced by the largest id each page actually returned. Re-reading `max(id)` from the store
//! between pages lets a live message arriving mid-loop jump the cursor to its id, and every message
//! between the last page and it is skipped — permanently, because `after_id` can never look back
//! and history paging only ever goes older than the oldest row held.
//!
//! **How the app wires it.** One `Resync` per signed-in server, over the same stores as the router
//! and the same pipeline as the composer. On the socket's
//! [`on_connected`](super::LiveSocket::on_connected) — raised before the first frame of the
//! connection is read — the app calls [`Resync::snapshot`] AT ONCE, synchronously, and then asks
//! for a pass; the passes are ONE future the app spawns on the main context for the life of the
//! session (the C#'s `LiveConnection.PassesAsync`), which awaits a "due" signal and runs
//! [`Resync::run`] once per signal, never two at a time. A pass answers a [`Report`]: what it read,
//! where it stopped (a transient stop is tried again on the next connection), and what the SESSION
//! keeps — `/me` ([`Report::me`]), the shown family's assistant and video-message limits. A pass
//! that answers `Err` — the cache refused a write — is a pass that failed, and the next connection
//! starts another. Cancelling a pass is dropping its future: the cursors it started from are handed
//! to the next pass all the same.
//!
//! The batch [`FLAG_REPAIR_BATCH`] is the store's
//! ([`ChatStore::flag_repair_candidates`]) and was ported with it.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use super::api_client::ApiClient;
use super::api_error::{ApiError, error_codes};
use super::requests::{AssistantDto, ChatRowDto, MeResponse, RoundVideoLimits};
use super::send_pipeline::SendPipeline;
use super::send_rules::FlushTrigger;
use crate::store::{BoardStore, ChatStore, PackLimits, PackStore, SeqRoute, StoreResult};

/// How many possible stickers and circles one pass reads again at most — iOS's and Android's
/// batch (`Resync.FlagRepairBatch`).
pub const FLAG_REPAIR_BATCH: usize = 25;

/// How many rows a catch-up page asks for. The server's own default (`Resync.PageSize`).
pub const PAGE_SIZE: i32 = 50;

/// [`PAGE_SIZE`] as a length, for "a short page is the end of the loop".
const PAGE: usize = PAGE_SIZE as usize;

/// What one pass did, for a log and for a test (`Resync.Report`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub flushed: bool,
    pub signed: bool,
    pub has_family: bool,
    pub chats: usize,
    pub messages: usize,
    pub reactions: usize,
    pub edits: usize,
    pub polls: usize,
    pub notes: usize,
    /// Items of the sticker pack read or caught up on, tombstones included.
    pub pack_items: usize,
    /// The assistant this family advertises, or `None` where it has none. Carried OUT of the pass
    /// because the family's own document is the only read that names it and this is the only thing
    /// that runs that read on a reconnect — a client that dropped it here had no assistant in its
    /// session state at all, and so no way to offer the consent the protocol requires before a
    /// word goes to a model (docs/protocol.md, "Consenting to the assistant"). The store keeps the
    /// roster; the SESSION keeps this, so it leaves here rather than being applied.
    pub assistant: Option<AssistantDto>,
    pub stopped: Option<ApiError>,
    /// Whether this pass read the family's own document — the only read that carries what
    /// follows.
    pub family_read: bool,
    /// The video message's two limits, or `None` on a server that predates them (docs/protocol.md,
    /// "Video messages"). Carried out of the pass as the assistant is, and applied only when
    /// [`family_read`](Self::family_read): a pass that stopped before that read knows nothing
    /// about them, which is not the same as a server without them.
    pub round_video: Option<RoundVideoLimits>,
    /// How many cached sets an older build wrote without the sticker or video-message flag were
    /// read again and changed (schema step 8).
    pub flags_repaired: usize,
    /// What step 1 answered — who I am, and every family I am in (docs/protocol.md, "Several
    /// families"). Carried out so the SESSION can reconcile its family list with it, as it carries
    /// the assistant: the store keeps rosters, the session keeps which families there are and which
    /// one is shown.
    pub me: Option<MeResponse>,
    /// The families whose own document this pass read, by id — every one [`me`](Self::me) names,
    /// until a read stopped.
    pub families_read: Option<Vec<i64>>,
    /// The family [`assistant`](Self::assistant) and [`round_video`](Self::round_video) are about
    /// — the one shown when the pass began. A session showing another by the time the pass ends
    /// does not take them.
    pub shown_family: Option<i64>,
}

impl Report {
    /// Whether every read finished. A flush that ran anyway is not a failure.
    pub fn complete(&self) -> bool {
        self.stopped.is_none()
    }
}

/// What one failed read means to the flag repair (`Resync.FlagRepairOutcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlagRepairOutcome {
    /// The server read the request and refused it, or answered what this build cannot read:
    /// asking again changes nothing. Settled, never asked again.
    Settled,

    /// No answer about this message yet (a 5xx, a timeout): asked again next pass; the pass goes
    /// on.
    AskAgainLater,

    /// Nothing about this message at all — the network, the session, the server's patience: every
    /// other read would fail the same way, so the rest wait for the next pass.
    EndPass,
}

/// iOS's `roundRepairOutcome` and Android's `repairUnknownRoundFlags`, in this client's error
/// shape: a transport failure, a 401 and a 429 end the pass; a 5xx, a 408 and `internal` are asked
/// again; any other 4xx, and a 2xx whose body would not decode ([`ApiClient`] calls that
/// `validation` with the 2xx status), settle the one message (`Resync.RepairOutcome`).
pub fn repair_outcome(error: &ApiError) -> FlagRepairOutcome {
    if error.code == error_codes::TRANSPORT
        || error.status == 401
        || error.status == 429
        || error.code == error_codes::TOO_MANY_REQUESTS
        || (error.status == 0 && !error.canonical())
    {
        FlagRepairOutcome::EndPass
    } else if error.status >= 500 || error.status == 408 || error.code == error_codes::INTERNAL {
        FlagRepairOutcome::AskAgainLater
    } else {
        FlagRepairOutcome::Settled
    }
}

/// Every chat's message catch-up cursor, by chat id.
type Cursors = BTreeMap<i64, i64>;

/// What a client does on every (re)connect.
pub struct Resync {
    api: Rc<ApiClient>,
    chats: Rc<ChatStore>,
    board: Rc<BoardStore>,
    sending: Option<Rc<SendPipeline>>,
    pack: Option<Rc<PackStore>>,
    /// The message cursors taken when a connection opened, waiting for the pass that uses them.
    pending: RefCell<Option<Cursors>>,
}

impl std::fmt::Debug for Resync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resync")
            .field("sending", &self.sending.is_some())
            .field("pack", &self.pack.is_some())
            .field(
                "pending",
                &self.pending.borrow().as_ref().map(BTreeMap::len),
            )
            .finish_non_exhaustive()
    }
}

/// The C#'s `finally` in `RunAsync`: a pass that did not finish — it stopped, the cache failed, or
/// its future was dropped — hands the cursors it started from to the next pass, which would
/// otherwise begin past whatever this one did not reach.
struct HandBack<'a> {
    pending: &'a RefCell<Option<Cursors>>,
    taken: Option<Cursors>,
}

impl Drop for HandBack<'_> {
    fn drop(&mut self) {
        if let Some(taken) = self.taken.take()
            && let Ok(mut pending) = self.pending.try_borrow_mut()
        {
            let waiting = pending.take();
            *pending = earliest(Some(taken), waiting);
        }
    }
}

impl Resync {
    /// `new Resync(api, chats, board)`: no outbox to flush and no pack to keep until
    /// [`with_sending`](Self::with_sending) and [`with_pack`](Self::with_pack) give them.
    pub fn new(api: Rc<ApiClient>, chats: Rc<ChatStore>, board: Rc<BoardStore>) -> Self {
        Resync {
            api,
            chats,
            board,
            sending: None,
            pack: None,
            pending: RefCell::new(None),
        }
    }

    /// The outbox this pass flushes FIRST (the C#'s `sending`).
    pub fn with_sending(mut self, sending: Rc<SendPipeline>) -> Self {
        self.sending = Some(sending);
        self
    }

    /// The sticker pack this pass catches up (the C#'s `pack`).
    pub fn with_pack(mut self, pack: Rc<PackStore>) -> Self {
        self.pack = Some(pack);
        self
    }

    /// Take every chat's message cursor NOW — as a connection OPENS, before any frame on it is
    /// applied (docs/protocol.md, "Best-effort delivery", step 3). The pass reaches the messages
    /// several round trips later, and a live message landing in between would otherwise become the
    /// cursor: everything missed while the socket was down would be skipped, for good. A connection
    /// that opens while a pass is still waiting keeps the EARLIER cursor of the two.
    pub fn snapshot(&self) -> StoreResult<()> {
        // The pack's cursor is held still the other way round: rather than being copied out, it
        // simply may not be moved by a frame until a pass that BEGAN after this has caught up — a
        // pass already in flight read its answers before this connection was listening.
        if let Some(pack) = &self.pack {
            pack.reconnected();
        }
        let now = self.chats.catch_up_cursors()?;
        let mut pending = self.pending.borrow_mut();
        let waiting = pending.take();
        *pending = earliest(waiting, Some(now));
        Ok(())
    }

    /// One pass. Answers what it managed; a read that fails stops the READS and nothing else — and
    /// hands the cursors it started from to the next pass, which would otherwise begin past
    /// whatever this one did not reach.
    pub async fn run(&self) -> StoreResult<Report> {
        // No connection handed its cursors over: this pass takes its own, now — before its round
        // trips give a live frame the time to land. Cursors that were handed over are held to what
        // the cache still holds.
        let now = self.chats.catch_up_cursors()?;
        let handed = self.pending.borrow_mut().take();
        let taken = match handed {
            None => now,
            Some(handed) => held_to(&handed, &now),
        };
        let mut hand_back = HandBack {
            pending: &self.pending,
            taken: Some(taken.clone()),
        };
        let report = self.pass(&taken).await;
        if matches!(&report, Ok(report) if report.complete()) {
            hand_back.taken = None;
        }
        report
    }

    async fn pass(&self, taken: &Cursors) -> StoreResult<Report> {
        let mut report = Report::default();

        // Which connection this pass is beginning on, before its first request: the pack may only
        // be called caught up with THAT one. A socket that reopens while the pass is waiting makes
        // everything the pass has already read older than the new connection, and its word on the
        // pack is then dropped (`PackStore::caught_up`) — the pass that connection started says it.
        let began = self.pack.as_ref().map_or(0, |pack| pack.connection());

        // FIRST, and unconditionally: it costs one request per queued message, it is idempotent,
        // and an early flush is never wrong.
        if let Some(sending) = &self.sending {
            sending.flush(FlushTrigger::SocketConnected).await?;
            report.flushed = true;
        }

        // 1. Who I am, and what this server can do.
        let me = match self.api.me().await.into_result() {
            Ok(me) => me,
            Err(error) => {
                report.stopped = Some(error);
                return Ok(report);
            }
        };
        // EVERY family the account is in (docs/protocol.md, "Several families") — or the one an
        // older server describes, which knows no `/families/{id}` and is asked for `mine`.
        let families = me.all_families();
        let named = me.families.is_some();
        report.signed = true;
        report.has_family = !families.is_empty();
        // The block list is COMPLETE STATE and the one read where an absent list means nobody
        // rather than "leave what you hold alone" — there is no catch-up feed for it and nothing
        // to miss, which is exactly why it is re-read in full on every pass.
        self.chats
            .replace_blocked(me.blocked_user_ids.as_deref().unwrap_or(&[]))?;
        report.me = Some(me);

        // The roster and the board's high-water mark live only on each family's own document, and
        // nothing else replays a roster change missed while offline — a join, a leave, a birthday,
        // the join policy, the cap and the family's language raise no frame at all, or one a
        // sleeping client did not get. One read per family, each into its own.
        let shown = match self.board.family() {
            Some(current) if families.iter().any(|family| family.id == current) => Some(current),
            _ => families.first().map(|family| family.id),
        };
        // A cache whose stores were never told of a family, talking to a server that knows of one
        // family only, is kept the way it always was: one roster, one wall, one pack, unscoped.
        // Anything else is kept per family.
        let legacy = !named && self.board.family().is_none();
        let kept = |family_id: i64| (!legacy).then_some(family_id);
        let mut marks: Vec<Mark> = Vec::new();
        let mut read = Vec::new();
        for held in &families {
            let answer = if named {
                self.api.family(Some(held.id)).await
            } else {
                self.api.family(None).await
            };
            let family = match answer.into_result() {
                Ok(family) => family,
                Err(error) => {
                    if error.code == error_codes::FAMILY_NOT_FOUND {
                        // Gone between `/me` and here — left, removed, deleted. Not this pass's to
                        // stop over: the session reconciles its list from the next `/me`, and
                        // nothing of this family is written meanwhile.
                        continue;
                    }
                    report.stopped = Some(error);
                    report.families_read = Some(read);
                    return Ok(report);
                }
            };
            read.push(held.id);
            self.chats.replace_members_of(
                kept(held.id),
                family.members.as_deref().unwrap_or(&[]),
                family.former_members.as_deref().unwrap_or(&[]),
            )?;
            // The same list again, from the family's own document, and applied only when it is
            // THERE. `/me` is the read that always carries it, so an absence here is an older
            // server rather than an empty list — the one place on this pass where absent means
            // "leave alone", because the fact has already arrived in full from the read whose own
            // document it is.
            if let Some(blocked) = &family.blocked_user_ids {
                self.chats.replace_blocked(blocked)?;
            }
            // The two ceilings are ALWAYS there on a server that has packs, so their absence is an
            // older server — and is written down as that, because a ceiling remembered from before
            // a downgrade would offer a sticker button that leads to a 404.
            let limits = match (family.max_pack_items, family.max_pack_item_bytes) {
                (Some(max_items), Some(max_item_bytes)) => Some(PackLimits {
                    max_items,
                    max_item_bytes,
                }),
                _ => None,
            };
            if let Some(pack) = &self.pack {
                pack.in_family(kept(held.id)).set_limits(limits)?;
            }
            marks.push(Mark {
                family: held.id,
                board: family.max_board_seq,
                pack: family.max_pack_seq,
                has_packs: limits.is_some(),
            });
            if Some(held.id) == shown {
                // The assistant and the video message's limits are the SHOWN family's: what the
                // composer offers.
                report.round_video = RoundVideoLimits::of(&family);
                report.assistant = family.assistant;
                report.family_read = true;
                report.shown_family = Some(held.id);
            }
        }
        report.families_read = Some(read);

        // 2. The list: previews, the authoritative unread counts, and the caller's own marker. The
        // mark is taken BEFORE the request: a group changed while it is in flight is not undone by
        // its answer.
        let mark = self.chats.list_mark();
        let rows = match self.api.chats().await.into_result() {
            Ok(list) => list.chats.unwrap_or_default(),
            Err(error) => {
                report.stopped = Some(error);
                return Ok(report);
            }
        };
        self.chats.replace(&rows, Some(mark))?;
        report.chats = rows.len();

        // 3. Per chat: the messages, then the three sequence catch-ups.
        for row in &rows {
            self.catch_up(row, taken, &mut report).await?;
            if report.stopped.is_some() {
                return Ok(report);
            }
        }

        // Each family's wall, against its own cursor (docs/protocol.md, "Several families").
        for mark in &marks {
            let wall = self.board.in_family(kept(mark.family));
            let (notes, stopped) = self
                .board_catch_up(&wall, named.then_some(mark.family), mark.board)
                .await?;
            if stopped
                .as_ref()
                .is_some_and(|error| error.code == error_codes::FAMILY_NOT_FOUND)
            {
                // The family went while this pass was reading it: nothing more of it to read, and
                // `/me` reconciles.
                continue;
            }
            report.notes += notes;
            if stopped.is_some() {
                report.stopped = stopped;
                return Ok(report);
            }
        }

        // Each family's sticker pack, where this server has one: the board's catch-up, one table
        // over.
        if let Some(pack) = &self.pack {
            for mark in marks.iter().filter(|mark| mark.has_packs) {
                let held = pack.in_family(kept(mark.family));
                let (items, stopped) = self
                    .pack_catch_up(&held, named.then_some(mark.family), mark.pack, began)
                    .await?;
                if stopped
                    .as_ref()
                    .is_some_and(|error| error.code == error_codes::FAMILY_NOT_FOUND)
                {
                    continue;
                }
                report.pack_items += items;
                if stopped.is_some() {
                    report.stopped = stopped;
                    return Ok(report);
                }
            }
        }

        // Last, and only on a pass that read everything: the stickers and circles an older build
        // cached as plain photos and videos (schema step 8). Best effort — what it cannot finish
        // waits for the next pass, and nothing it meets is a reason to call this pass failed.
        if report.stopped.is_none() {
            report.flags_repaired = self.repair_flags().await?;
        }
        Ok(report)
    }

    /// Read again, once, the held messages that may be STICKERS or VIDEO MESSAGES but were cached
    /// by a build that did not know the flag (#58, #79; docs/audio-video-messages-2026-10-04.md,
    /// S5.8) — iOS's `ChatSyncCoordinator.repairUnknownRoundFlags` and Android's
    /// `MessageRepository.repairUnknownRoundFlags`. Answers how many held sets changed.
    ///
    /// A build that predates a flag wrote the set back without it, and a chat held in sequence is
    /// never paged again — so after the upgrade a sticker kept its grey tile and a circle stayed
    /// square, for good, while every other device drew the same message right. Each candidate
    /// ([`ChatStore::flag_repair_candidates`]) is read through `before_id = id + 1, limit = 1` and
    /// its set written back ([`ChatStore::repair_media`]), newest first, at most
    /// [`FLAG_REPAIR_BATCH`] per pass; a message the server no longer has is settled.
    ///
    /// ONE FAILED READ IS ABOUT ONE MESSAGE, and the pass goes on past it ([`repair_outcome`]):
    /// ending on it would ask the same unreadable message FIRST on every pass and never reach the
    /// older ones.
    async fn repair_flags(&self) -> StoreResult<usize> {
        let mut repaired = 0;
        for held in self.chats.flag_repair_candidates(FLAG_REPAIR_BATCH)? {
            let page = self
                .api
                .messages(held.chat_id, Some(held.id + 1), Some(1))
                .await;
            match page.into_result() {
                Err(error) => match repair_outcome(&error) {
                    FlagRepairOutcome::Settled => self.chats.settle_flags(held.id)?,
                    FlagRepairOutcome::AskAgainLater => {}
                    FlagRepairOutcome::EndPass => return Ok(repaired),
                },
                Ok(page) => {
                    let server = page
                        .messages
                        .unwrap_or_default()
                        .into_iter()
                        .find(|message| message.id == held.id);
                    match server {
                        Some(server) => {
                            if self.chats.repair_media(&server)? {
                                repaired += 1;
                            }
                        }
                        None => self.chats.settle_flags(held.id)?,
                    }
                }
            }
        }
        Ok(repaired)
    }

    /// One chat's four loops: the messages, then the three sequence feeds — counted into `report`,
    /// and a read that fails is `report.stopped`.
    async fn catch_up(
        &self,
        row: &ChatRowDto,
        taken: &Cursors,
        report: &mut Report,
    ) -> StoreResult<()> {
        let chats = &self.chats;
        let chat_id = row.chat.id;

        // THE CURSOR IS READ ONCE, and before the pass: taken as the connection opened, or as the
        // pass began — from what was delivered in sequence, never from the list preview step 2 just
        // stored. From here on it is advanced by what the pages return, and never re-read from the
        // store. A chat that held nothing in sequence has no hole to fill: history paging reads it
        // from the newest page when it is opened.
        if let Some(&start) = taken.get(&chat_id) {
            let mut cursor = start;
            loop {
                let messages = match self
                    .api
                    .messages_after(chat_id, cursor, PAGE_SIZE)
                    .await
                    .into_result()
                {
                    Ok(page) => page.messages.unwrap_or_default(),
                    Err(error) => {
                        report.stopped = Some(error);
                        return Ok(());
                    }
                };
                if messages.is_empty() {
                    break;
                }
                chats.apply_page(&messages, SeqRoute::CatchUpPage, true)?;
                report.messages += messages.len();
                // A CURSOR THAT CANNOT MOVE ENDS THE LOOP. Every one of these four feeds answers
                // strictly newer rows, so a full page always advances one — but "always" here is
                // the server's promise, not this client's arithmetic, and a full page that did not
                // move the cursor would be asked for again with the same argument until the process
                // died.
                let next = messages
                    .iter()
                    .map(|message| message.id)
                    .max()
                    .unwrap_or(cursor);
                if next <= cursor {
                    break;
                }
                cursor = next;
                if messages.len() < PAGE {
                    // A short page is the end of the loop.
                    break;
                }
            }
        }

        // Then the reaction catch-up, and only when the chat says there is something to catch up
        // to: `max_reaction_seq` from step 2 against the cursor this device holds.
        let reaction_cursor = chats
            .chat(chat_id)?
            .and_then(|held| held.max_reaction_seq)
            .unwrap_or(0);
        if row.max_reaction_seq.unwrap_or(0) > reaction_cursor {
            let mut seq = reaction_cursor;
            loop {
                let states = match self
                    .api
                    .reactions_after(chat_id, seq, PAGE_SIZE)
                    .await
                    .into_result()
                {
                    Ok(page) => page.message_reactions.unwrap_or_default(),
                    Err(error) => {
                        report.stopped = Some(error);
                        return Ok(());
                    }
                };
                if states.is_empty() {
                    break;
                }
                for state in &states {
                    // A state for a message this device does not hold is DROPPED — history paging
                    // re-delivers it embedded on the message itself — and the cursor advances all
                    // the same, which is why the ROUTE is what says so: the store moves the chat on
                    // for a page and a frame, and never for evidence.
                    chats.apply_reactions(
                        chat_id,
                        state.message_id,
                        state.reaction_seq,
                        &state.reactions,
                        SeqRoute::CatchUpPage,
                    )?;
                }
                report.reactions += states.len();
                let next = states
                    .iter()
                    .map(|state| state.reaction_seq)
                    .max()
                    .unwrap_or(seq);
                if next <= seq {
                    break;
                }
                seq = next;
                if states.len() < PAGE {
                    break;
                }
            }
        }

        // Then the edits. This is the ONLY step that learns of a change to a message this device
        // already holds: `after_id` is `WHERE id > cursor` and cannot look at an older row, so
        // without it a device that slept through an edit shows the old words until something else
        // happens to that message. The feed answers whole messages, so they go through the very
        // path a page of history goes through — guard included.
        let edit_cursor = chats
            .chat(chat_id)?
            .and_then(|held| held.max_edit_seq)
            .unwrap_or(0);
        if row.max_edit_seq.unwrap_or(0) > edit_cursor {
            let mut seq = edit_cursor;
            loop {
                let edited = match self
                    .api
                    .edits_after(chat_id, seq, PAGE_SIZE)
                    .await
                    .into_result()
                {
                    Ok(page) => page.messages.unwrap_or_default(),
                    Err(error) => {
                        report.stopped = Some(error);
                        return Ok(());
                    }
                };
                if edited.is_empty() {
                    break;
                }
                // Out of sequence: an edited message may be one this device never held, and it
                // says nothing about the messages before it.
                chats.apply_page(&edited, SeqRoute::Evidence, false)?;
                report.edits += edited.len();
                // A message in this feed carries the seq that put it here — an absent one is a
                // server breaking its own contract, and reads as 0, which cannot move the cursor
                // and so ends the loop rather than spinning it.
                let next = edited
                    .iter()
                    .map(|message| message.edit_seq.unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                if next <= seq {
                    break;
                }
                seq = next;
                chats.advance(chat_id, None, Some(seq), None)?;
                if edited.len() < PAGE {
                    break;
                }
            }
        }

        let poll_cursor = chats
            .chat(chat_id)?
            .and_then(|held| held.max_poll_seq)
            .unwrap_or(0);
        if row.max_poll_seq.unwrap_or(0) > poll_cursor {
            let mut seq = poll_cursor;
            loop {
                let states = match self
                    .api
                    .polls_after(chat_id, seq, PAGE_SIZE)
                    .await
                    .into_result()
                {
                    Ok(page) => page.polls.unwrap_or_default(),
                    Err(error) => {
                        report.stopped = Some(error);
                        return Ok(());
                    }
                };
                if states.is_empty() {
                    break;
                }
                for state in &states {
                    chats.apply_poll(
                        chat_id,
                        state.message_id,
                        &state.poll,
                        SeqRoute::CatchUpPage,
                    )?;
                }
                report.polls += states.len();
                let next = states
                    .iter()
                    .map(|state| state.poll.poll_seq)
                    .max()
                    .unwrap_or(seq);
                if next <= seq {
                    break;
                }
                seq = next;
                if states.len() < PAGE {
                    break;
                }
            }
        }
        Ok(())
    }

    /// The wall: the CHANGES feed from where this device left off, or the whole board when it has
    /// never read one — a full read REPLACES what is held, which is the only way a note somebody
    /// took down while this device was away actually leaves the wall. Answers how many notes it
    /// read, and the failure that stopped it.
    ///
    /// `wall` is the one family being caught up — a store held to it; `family_id` is that family's
    /// id for the wire, or `None` for a server that knows only `mine`.
    async fn board_catch_up(
        &self,
        wall: &BoardStore,
        family_id: Option<i64>,
        mark: Option<i64>,
    ) -> StoreResult<(usize, Option<ApiError>)> {
        // `max_board_seq` is ABSENT while the wall is empty and untouched, and that is the whole
        // point of the field: there is nothing to read, so nothing is asked for. It is checked
        // against what is HELD as well, because a wall this device holds notes for cannot be one
        // nobody has ever touched — and if the two ever disagree, reading wins.
        if mark.is_none() && wall.notes()?.is_empty() {
            return Ok((0, None));
        }
        let cursor = wall.cursor()?;
        if let Some(mark) = mark
            && cursor != 0
            && mark <= cursor
        {
            // Level with the server: this device has applied every change there has been.
            return Ok((0, None));
        }
        if cursor == 0 {
            return match self.api.board(family_id).await.into_result() {
                Ok(whole) => {
                    let notes = whole.notes.unwrap_or_default();
                    wall.replace(&notes, whole.max_board_seq)?;
                    Ok((notes.len(), None))
                }
                Err(error) => Ok((0, Some(error))),
            };
        }
        let mut read = 0;
        loop {
            let notes = match self
                .api
                .board_changes(family_id, wall.cursor()?, PAGE_SIZE)
                .await
                .into_result()
            {
                Ok(page) => page.notes.unwrap_or_default(),
                Err(error) => return Ok((read, Some(error))),
            };
            if notes.is_empty() {
                break;
            }
            // The board's own cursor is advanced by the store as it applies, because the feed
            // INCLUDES tombstones and a delete moves the seq like anything else.
            let was = wall.cursor()?;
            wall.apply_page(&notes, SeqRoute::CatchUpPage)?;
            read += notes.len();
            if wall.cursor()? <= was || notes.len() < PAGE {
                break;
            }
        }
        Ok((read, None))
    }

    /// The family's sticker pack: the WHOLE of it when this device holds none, the change feed from
    /// where it left off otherwise (docs/protocol.md, "Sticker pack") — the board's rules, which is
    /// the point of the design: no client learns a new sync idea. Answers how many items it read,
    /// and the failure that stopped it.
    ///
    /// The cursor is the STORE's and is read from it on every turn of the loop, which is safe here
    /// for a reason the message loop does not have: a `pack_item` frame may not move it until this
    /// method says the connection has caught up, so nothing can step it past what was missed. And
    /// it says so only for the connection the pass BEGAN on (`began`): the mark and the pages below
    /// were all read after that connection opened, and none of them is known to be newer than a
    /// connection that opened since.
    async fn pack_catch_up(
        &self,
        pack: &PackStore,
        family_id: Option<i64>,
        mark: Option<i64>,
        began: i64,
    ) -> StoreResult<(usize, Option<ApiError>)> {
        // `max_pack_seq` is ABSENT while the pack is empty and untouched: there is nothing to read,
        // so nothing is asked for. Checked against what is HELD as well — a pack this device holds
        // items for cannot be one nobody has ever touched, and if the two disagree, reading wins.
        if mark.is_none() && pack.count()? == 0 {
            pack.caught_up(began);
            return Ok((0, None));
        }
        let cursor = pack.cursor()?;
        if let Some(mark) = mark
            && cursor != 0
            && mark <= cursor
        {
            // Level with the server: every change there has been is applied here.
            pack.caught_up(began);
            return Ok((0, None));
        }
        if cursor == 0 {
            return match self.api.pack(family_id).await.into_result() {
                Ok(whole) => {
                    let items = whole.items.unwrap_or_default();
                    pack.replace(&items, whole.max_pack_seq)?;
                    pack.caught_up(began);
                    Ok((items.len(), None))
                }
                Err(error) => Ok((0, Some(error))),
            };
        }
        let mut read = 0;
        loop {
            let was = pack.cursor()?;
            let items = match self
                .api
                .pack_changes(family_id, was, PAGE_SIZE)
                .await
                .into_result()
            {
                Ok(page) => page.items.unwrap_or_default(),
                Err(error) => return Ok((read, Some(error))),
            };
            if items.is_empty() {
                break;
            }
            // The store advances the cursor as it applies: the feed INCLUDES tombstones, and a
            // removal moves the seq like anything else.
            pack.apply_page(&items, SeqRoute::CatchUpPage)?;
            read += items.len();
            // A cursor that cannot move ends the loop, as it does for every other feed here.
            if pack.cursor()? <= was || items.len() < PAGE {
                break;
            }
        }
        pack.caught_up(began);
        Ok((read, None))
    }
}

/// One family's marks from its own document, for the board and pack catch-ups after the chats.
#[derive(Debug, Clone, Copy)]
struct Mark {
    family: i64,
    board: Option<i64>,
    pack: Option<i64>,
    has_packs: bool,
}

/// Per chat, the lower of two sets of cursors — the one that asks for more, never for less.
fn earliest(a: Option<Cursors>, b: Option<Cursors>) -> Option<Cursors> {
    match (a, b) {
        (Some(mut merged), Some(other)) => {
            for (chat, cursor) in other {
                merged
                    .entry(chat)
                    .and_modify(|held| *held = (*held).min(cursor))
                    .or_insert(cursor);
            }
            Some(merged)
        }
        (a, b) => a.or(b),
    }
}

/// Handed-over cursors, held to what the cache holds in sequence NOW: never above it, and none for
/// a chat it holds nothing of. A cursor only grows, so this changes nothing — except after the
/// cache was wiped (a sign-out, another account, another server), where a stranger's cursor would
/// page a chat from past what its new messages reach.
fn held_to(handed: &Cursors, now: &Cursors) -> Cursors {
    handed
        .iter()
        .filter_map(|(chat, cursor)| now.get(chat).map(|held| (*chat, (*cursor).min(*held))))
        .collect()
}
