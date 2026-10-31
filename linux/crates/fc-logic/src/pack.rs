//! The family's sticker pack from the window's side (docs/protocol.md, "Sticker pack"): what the
//! panel shows, who may remove what, and the three writes — add, remove, send (`Pack.cs`).
//!
//! "STICKER" HERE IS THE CHAT ONE — a small picture sent as its own message, the way a messenger's
//! stickers are. The cards on the board are called stickers in this code too (`board::Sticker`);
//! the two never meet, and everything about THIS one is spelled `pack` so that nobody has to read a
//! name twice.
//!
//! A STICKER IS NEVER PREPARED. Everything this client does to a photograph on its way up —
//! decode, downscale, JPEG re-encode, metadata strip and preview — would destroy exactly what makes
//! a sticker one: a JPEG has no transparency and one frame. So nothing here goes near that path. A
//! pack item, and a sticker message, are built from the ORIGINAL bytes as [`StagedMedia`] with no
//! preview, and that is also what makes the copy free: the message's upload hashes to the pack
//! item's file.
//!
//! A SENT STICKER IS A COPY. The message carries its own attachment and does not name the pack
//! item; the pack does not know about the message. So a send needs the item's BYTES, which this
//! device keeps under the attachment id — and an item whose bytes are not here yet, on a device
//! that is offline, cannot be sent until they are.
//!
//! BLOCKS DO NOT REACH THE PACK. A blocked member's sticker MESSAGE is hidden like any message of
//! theirs — that is the conversation's rule and it is untouched — but their pack ITEMS are drawn
//! for everyone: an item is a picture the family keeps, and a panel one sticker short for one
//! member would be a quantity that moved when they blocked somebody. Nothing in this module asks
//! who is blocked, on purpose.
//!
//! What the bytes ARE is read by `fc_text::pack` (`type_of`, `is_sticker_type`, `is_gif`,
//! `is_animated`, `fit`) and `fc_core::text::sticker_file::size`, the C#'s `StickerFile`; a type's
//! essence is `fc_text::media::essence` (`MediaPrep.Essence`). The rules ABOVE the bytes — the
//! plan for a picked picture, who may remove, whether the pack holds a sent sticker — are the C#'s,
//! ported as it writes them: `fc_text::pack`'s `plan`, `may_remove` and `could_be` are the web's
//! cousins of them and differ at the edges (see each function).
//!
//! **How the shell uses it.** One [`PackModel`] per signed-in session over the same `PackStore`,
//! `ChatStore`, client and [`AttachmentCache`] the conversation uses. The sticker button is drawn
//! where [`pack_sending::offered`] (or [`pack_sending::offered_in_thread`]) says; a click in the
//! panel goes through [`pack_sending::gate_for`] — asking the consent question on
//! [`pack_sending::Gate::Ask`], sending nothing on `Withheld` — then [`PackModel::to_send`], whose
//! [`StagedMedia`] the shell stages and queues with `ConversationModel::send_sticker`, and
//! [`PackModel::sent`] puts it first in the panel. "Add to family stickers" runs
//! [`pack_picking::plan_for`] on the picked bytes (the shell's decoder does a `Make`, then
//! [`PackModel::add`] asks again), and every refusal is said with [`pack_text::sentence`].

use std::rc::Rc;

use fc_core::protocol::{
    ApiClient, ApiError, AttachmentDto, PackItemDto, UploadFacts, error_codes,
};
use fc_core::rt::Clock;
use fc_core::store::{ChatStore, PackLimits, PackStore, SeqRoute, StoreResult};
use fc_core::text::sticker_file;
use fc_text::media::essence;
use fc_text::pack;

use crate::attachment_cache::AttachmentCache;
use crate::media_outbox::StagedMedia;

/// What an add came to, when it was not refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackAdded {
    /// A new item: the pack is one sticker bigger.
    Added,
    /// The pack ALREADY held it — the server's `200`. Not an error and not two stickers: two
    /// members who both liked it do not fill the panel with copies.
    AlreadyThere,
}

/// The sticker panel over the composer: what this device sent lately, and then the whole pack.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PackPanel {
    /// Newest first, and THIS DEVICE's alone — never on the wire, because it says something about a
    /// person's habits and nothing about the family's pack.
    pub recent: Vec<PackItemDto>,
    /// The whole pack in the order the items were added, which never reshuffles under a finger.
    pub all: Vec<PackItemDto>,
}

impl PackPanel {
    /// How many recent stickers lead the panel: the last SIXTEEN, the same on every client
    /// (`fc_text::pack::RECENTS_MAX`). They are this DEVICE's — kept in its cache file, so they are
    /// there after a restart — and they go with everything else at sign-out.
    pub const RECENT_ROOM: usize = pack::RECENTS_MAX;

    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    /// The panel from what is held. A recent id the pack no longer holds is simply not there: the
    /// item was removed, and a removed sticker is not offered for sending.
    pub fn of(items: &[PackItemDto], recents: &[i64]) -> PackPanel {
        let mut seen: Vec<i64> = Vec::new();
        let mut recent = Vec::new();
        for id in recents {
            if seen.contains(id) {
                continue;
            }
            seen.push(*id);
            if recent.len() == PackPanel::RECENT_ROOM {
                break;
            }
            if let Some(item) = items.iter().find(|item| item.id == *id) {
                recent.push(item.clone());
            }
        }
        let mut all = items.to_vec();
        all.sort_by_key(|item| item.id);
        PackPanel { recent, all }
    }
}

/// The family's sticker pack from the window's side (docs/protocol.md, "Sticker pack"): what the
/// panel shows, who may remove what, and the three writes — add, remove, send.
pub struct PackModel {
    pack: Rc<PackStore>,
    chats: Rc<ChatStore>,
    api: Rc<ApiClient>,
    attachments: Rc<AttachmentCache>,
    clock: Rc<dyn Clock>,
}

impl std::fmt::Debug for PackModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PackModel").finish_non_exhaustive()
    }
}

impl PackModel {
    /// `new PackModel(pack, chats, api, attachments, clock)`: the clock says when an item was
    /// [`sent`](Self::sent), which is all it is asked.
    pub fn new(
        pack: Rc<PackStore>,
        chats: Rc<ChatStore>,
        api: Rc<ApiClient>,
        attachments: Rc<AttachmentCache>,
        clock: Rc<dyn Clock>,
    ) -> Self {
        PackModel {
            pack,
            chats,
            api,
            attachments,
            clock,
        }
    }

    /// Whether this server has packs at all. One that predates them names no ceilings on `GET
    /// /families/mine`, and then there is no sticker button and no pack management — rather than a
    /// 404 when somebody taps one.
    pub fn offered(&self) -> StoreResult<bool> {
        Ok(self.pack.limits()?.is_some())
    }

    pub fn limits(&self) -> StoreResult<Option<PackLimits>> {
        self.pack.limits()
    }

    /// The pack in the order its items were added.
    pub fn items(&self) -> StoreResult<Vec<PackItemDto>> {
        self.pack.items()
    }

    /// What the panel over the composer shows.
    pub fn panel(&self) -> StoreResult<PackPanel> {
        Ok(PackPanel::of(
            &self.pack.items()?,
            &self.pack.recents(PackPanel::RECENT_ROOM)?,
        ))
    }

    /// Whether the pack is at its ceiling — said beside the Add button, before a picture is even
    /// chosen.
    pub fn is_full(&self) -> StoreResult<bool> {
        Ok(match self.pack.limits()? {
            Some(limits) => !pack::has_room(self.pack.count()?, i64::from(limits.max_items)),
            None => false,
        })
    }

    /// Whoever added it, or the family's owner — a permission shape the board does not have, and
    /// the reason is the pack's other difference: a member who has left, or deleted their account,
    /// leaves their stickers behind, and under an author-only rule nobody could ever remove those.
    ///
    /// The C#'s static `MayRemove(item, reader, readerOwnsFamily)`. Nobody signed in (reader `0`) is
    /// not the adder of an item whose adder is unknown (`0`) — where `fc_text::pack::may_remove`,
    /// which reads `added_by` as optional, would say yes.
    pub fn may_remove_by(item: &PackItemDto, reader: i64, reader_owns_family: bool) -> bool {
        reader_owns_family || (reader != 0 && item.added_by == reader)
    }

    /// [`may_remove_by`](Self::may_remove_by) for whoever is signed in.
    pub fn may_remove(&self, item: &PackItemDto, reader_owns_family: bool) -> bool {
        PackModel::may_remove_by(item, self.chats.reader(), reader_owns_family)
    }

    /// A pack item's bytes — the ORIGINAL, always, whatever `has_preview` says.
    pub async fn bytes(&self, item: &PackItemDto) -> Result<Vec<u8>, ApiError> {
        match &item.attachment {
            Some(picture) => self.attachments.bytes(picture, false).await,
            None => Err(ApiError::coded(
                error_codes::PACK_ITEM_NOT_FOUND,
                "the item has no picture",
            )),
        }
    }

    // ---- adding --------------------------------------------------------------------------------

    /// Add a picture to the pack: upload the bytes AS THEY ARE, then claim them. Any member may.
    ///
    /// REFUSED HERE FIRST, with the server's own codes, where the limits are already known — so
    /// "that one is too big" arrives beside the picker rather than as a rejected request
    /// (docs/protocol.md, "Limits"). The server checks again, because the pack may have filled
    /// since this device last looked.
    ///
    /// `attachment_expired` at the claim means the unclaimed sweep took the upload, and the answer
    /// to that is to upload it again — ONCE, here, since the bytes are in hand — and claim again.
    /// Only a second failure is shown.
    ///
    /// NEW OR ALREADY THERE IS THE HTTP STATUS, AND NOTHING ELSE. `201` is a new item and `200` is
    /// the pack already holding it. Whether THIS DEVICE already holds the item says nothing: the
    /// server fans the actor's own `pack_item` frame out before the POST answers, so a genuinely new
    /// sticker is very often in the cache by the time its claim comes back — and was then reported
    /// as "The family already has that sticker."
    pub async fn add(
        &self,
        bytes: &[u8],
        label: Option<&str>,
    ) -> StoreResult<Result<PackAdded, ApiError>> {
        let Some(limits) = self.pack.limits()? else {
            // A server with no packs. The window offers no way here; this is the backstop.
            return Ok(Err(ApiError::coded(
                error_codes::PACK_ITEM_NOT_FOUND,
                "this server has no sticker pack",
            )));
        };
        // The window refuses an over-long label with words, beside the box it was typed in; this
        // is the backstop, and it too is before any request.
        if pack_label::too_long(label) {
            return Ok(Err(pack_label::refused()));
        }
        let label = pack_label::clean(label);
        if let Some(refused) = pack_picking::refusal(bytes, limits, self.pack.count()?) {
            return Ok(Err(refused));
        }
        let mime = pack::type_of(bytes).unwrap_or(pack::PNG);
        let size = sticker_file::size(bytes);
        let facts = UploadFacts {
            width: size.and_then(|(width, _)| i32::try_from(width).ok()),
            height: size.and_then(|(_, height)| i32::try_from(height).ok()),
            ..UploadFacts::default()
        };
        let mut last = ApiError::transport("no answer");
        for _attempt in 0..2 {
            // kind=photo, the sticker's own bytes, and NO preview: a preview is a JPEG, which is
            // the same destruction by another door.
            let upload = match self
                .api
                .upload("photo", mime, bytes.to_vec(), &facts)
                .await
                .into_result()
            {
                Ok(upload) => upload,
                Err(error) => return Ok(Err(error)),
            };
            let claim = self
                .api
                .add_to_pack(upload.attachment.id, label.as_deref())
                .await;
            // 201 is a new item, 200 is one the pack already held. Read from the STATUS: the cache
            // cannot say, because this device's own frame may have put the item there before this
            // answer arrived.
            let already = claim.status == 200;
            match claim.into_result() {
                Ok(answer) => {
                    let item = answer.item;
                    let Some(kept) = item.attachment.clone() else {
                        last = ApiError::transport("no answer");
                        break;
                    };
                    // Under the per-item guard, as EVIDENCE: the answer to this device's own POST
                    // moves no cursor, for the reason the board gives.
                    self.pack.apply(&item, SeqRoute::Evidence)?;
                    // The bytes are kept under the id the PACK holds them by — which is not the id
                    // that was just uploaded when the pack already held these bytes. A disk that
                    // will not take them costs a download later (the blob store logs and forgets
                    // the write), and never turns an add that HAPPENED into one reported as failed.
                    self.attachments.remember(&kept, bytes);
                    return Ok(Ok(if already {
                        PackAdded::AlreadyThere
                    } else {
                        PackAdded::Added
                    }));
                }
                Err(error) => {
                    let expired = error.code == error_codes::ATTACHMENT_EXPIRED;
                    last = error;
                    if !expired {
                        break;
                    }
                }
            }
        }
        Ok(Err(last))
    }

    /// "Add to family stickers" on a sticker somebody SENT: the pack's own flow, with the message's
    /// bytes — uploaded again, unprepared, and claimed.
    pub async fn add_from_message(
        &self,
        picture: &AttachmentDto,
        label: Option<&str>,
    ) -> StoreResult<Result<PackAdded, ApiError>> {
        if pack_label::too_long(label) {
            return Ok(Err(pack_label::refused()));
        }
        match self.attachments.bytes(picture, false).await {
            Ok(bytes) => self.add(&bytes, label).await,
            Err(error) => Ok(Err(error)),
        }
    }

    /// Whether the pack holds this sticker — decided HERE, from bytes this device already has,
    /// because nothing on the wire names the item a message was sent from: an item whose `size` and
    /// `mime` match, and whose bytes are the same.
    ///
    /// A wrong "no" costs nothing. It offers "Add to family stickers" for something the pack
    /// already has, and the server answers `200` with the item that was there.
    ///
    /// Not `fc_text::pack::could_be`, which lets an unknown size on either side through as a
    /// candidate: here a sticker with no size, or an item with none, is no match, as in the C#.
    pub async fn holds(&self, picture: &AttachmentDto) -> StoreResult<bool> {
        let Some(size) = picture.size else {
            return Ok(false);
        };
        let mime = essence(picture.mime.as_deref().unwrap_or(""));
        let candidates: Vec<PackItemDto> = self
            .pack
            .items()?
            .into_iter()
            .filter(|item| {
                item.attachment.as_ref().is_some_and(|kept| {
                    kept.size == Some(size) && essence(kept.mime.as_deref().unwrap_or("")) == mime
                })
            })
            .collect();
        if candidates.is_empty() {
            return Ok(false);
        }
        let Ok(sent) = self.attachments.bytes(picture, false).await else {
            return Ok(false);
        };
        for candidate in &candidates {
            if let Ok(kept) = self.bytes(candidate).await
                && kept == sent
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // ---- removing ------------------------------------------------------------------------------

    /// Take an item out of the pack. Every message ever sent with it keeps its own copy and goes on
    /// drawing.
    ///
    /// `pack_item_not_found` MEANS IT IS ALREADY GONE — removed by somebody else, or from a family
    /// this reader has left — and "gone" is what was asked for. The item is dropped here and NO
    /// ERROR is shown: a sentence about a sticker that is not there would be telling the person that
    /// what they wanted has happened.
    pub async fn remove(&self, item_id: i64) -> StoreResult<Option<ApiError>> {
        if let Err(error) = self.api.remove_from_pack(item_id).await.into_result()
            && error.code != error_codes::PACK_ITEM_NOT_FOUND
        {
            return Ok(Some(error));
        }
        // The frame and the catch-up both carry the tombstone; dropping the item here as well is
        // what makes the sticker leave the panel as the click lands. No cursor moves: the seq this
        // removal was given is the server's to announce, and a cursor moved to a number nobody
        // issued would step the feed past whatever really holds it.
        self.pack.removed(item_id)?;
        Ok(None)
    }

    // ---- sending -------------------------------------------------------------------------------

    /// What a tap on a pack item sends: the item's bytes AS CACHED, as one `kind=photo` upload with
    /// no preview. The caller stages it and queues the row with `sticker` set
    /// (`ConversationModel::send_sticker`) — the outbox owns it from there, exactly as it owns any
    /// message.
    ///
    /// The server does not check that the bytes are still in the pack, and deliberately: an item
    /// removed between the tap and the send must still go, and a send must survive an outbox that
    /// waited a day. So nothing here looks the item up again either.
    pub async fn to_send(&self, item: &PackItemDto) -> StoreResult<Result<StagedMedia, ApiError>> {
        let Some(picture) = &item.attachment else {
            return Ok(Err(ApiError::coded(
                error_codes::PACK_ITEM_NOT_FOUND,
                "the item has no picture",
            )));
        };
        let bytes = match self.bytes(item).await {
            Ok(bytes) => bytes,
            Err(error) => return Ok(Err(error)),
        };
        // What the bytes ARE, not what a row said of them: the upload is checked by magic number.
        let mime = match pack::type_of(&bytes) {
            Some(mime) => mime.to_string(),
            None => essence(picture.mime.as_deref().unwrap_or("")),
        };
        if !pack::is_sticker_type(&mime) {
            return Ok(Err(ApiError::coded(
                error_codes::INVALID_ATTACHMENT,
                "a sticker is a WebP or a PNG",
            )));
        }
        // The per-item ceiling binds a sticker MESSAGE too, and an operator may have lowered it
        // since this item was added: refused here rather than as a failed bubble.
        if let Some(limits) = self.pack.limits()?
            && over(bytes.len(), limits.max_item_bytes)
        {
            return Ok(Err(ApiError::coded(
                error_codes::PACK_ITEM_TOO_LARGE,
                "over the pack's per-item ceiling",
            )));
        }
        let size = sticker_file::size(&bytes);
        Ok(Ok(StagedMedia {
            width: picture
                .width
                .or_else(|| size.and_then(|(width, _)| i32::try_from(width).ok())),
            height: picture
                .height
                .or_else(|| size.and_then(|(_, height)| i32::try_from(height).ok())),
            ..StagedMedia::new("photo", &mime, bytes)
        }))
    }

    /// This device just sent that item: it leads the panel next time.
    pub fn sent(&self, item_id: i64) -> StoreResult<()> {
        self.pack.used(item_id, self.clock.now())
    }
}

/// Whether `length` bytes are over a ceiling of `max` bytes.
fn over(length: usize, max: i64) -> bool {
    i64::try_from(length).unwrap_or(i64::MAX) > max
}

/// The few words whoever adds a sticker may give it (docs/protocol.md, "label"): optional, for a
/// screen reader, never drawn over the picture — and fixed when the item is added (`PackLabel`).
///
/// COUNTED THE WAY THE SERVER COUNTS: Unicode scalar values, after trimming — not UTF-16 units,
/// which would call thirty-three emoji too long, and not what a screen shows, which would let a
/// family of joined emoji through as one. What is counted is exactly what is sent. (The same rule
/// as `fc_text::pack::label`, in the C#'s three questions.)
pub mod pack_label {
    use fc_core::protocol::ApiError;

    /// The most scalar values a label may be, after trimming.
    pub const MAX_LENGTH: usize = fc_text::pack::LABEL_MAX;

    /// The code of a label refused HERE, before any request. Not a server code: the server's is
    /// `validation`.
    pub const TOO_LONG_CODE: &str = "pack_label_too_long";

    /// The label as it is sent: trimmed, and `None` when nothing is left — an empty label is no
    /// label. (`str::trim` and .NET's `Trim` agree on what white space is.)
    pub fn clean(typed: Option<&str>) -> Option<String> {
        typed
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(str::to_string)
    }

    /// How long a label is, by the server's count: scalar values after trimming.
    pub fn length(typed: Option<&str>) -> usize {
        clean(typed).map_or(0, |label| label.chars().count())
    }

    pub fn too_long(typed: Option<&str>) -> bool {
        length(typed) > MAX_LENGTH
    }

    /// The refusal, for the one table of sentences ([`super::pack_text::sentence`]) — given the
    /// status the server would give it, so it reads as the refusal it is and never as a failure
    /// worth retrying.
    pub fn refused() -> ApiError {
        ApiError::new(
            TOO_LONG_CODE,
            "a sticker's label is at most 64 characters",
            400,
        )
    }
}

/// Where the sticker button is offered, and what stands between a click in the panel and the send
/// (`PackSending`).
///
/// IN EVERY CHAT A MESSAGE CAN BE SENT IN: the family chat, a one-to-one chat, a group, the
/// ASSISTANT's chat, and the THREAD composer. A sticker is a message, and a chat that takes
/// messages takes stickers.
///
/// IN THE ASSISTANT'S CHAT IT IS ASKED ABOUT LIKE ANY MESSAGE THERE (docs/protocol.md, "Consenting
/// to the assistant"). Everything sent in that chat reaches the model, and to the model a sticker
/// is a photo — so one click in the panel goes through the same question the Send button does,
/// never around it.
pub mod pack_sending {
    use crate::assistant_consent;

    /// What a click in the panel may do.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Gate {
        /// Send it.
        Send,
        /// It would reach the model and this member has not agreed: ask first, and send only on a
        /// yes.
        Ask,
        /// It would reach a model whose owner this server will not name: nothing is sent, as for
        /// any message there.
        Withheld,
    }

    /// The button by the conversation's composer: on a server that has packs, in any chat that is
    /// open, and not while a message is being edited — a sticker is its own message and an edit is
    /// somebody else's.
    pub fn offered(chat_kind: Option<&str>, editing: bool, pack_offered: bool) -> bool {
        pack_offered && !editing && chat_kind.is_some()
    }

    /// The button by the THREAD's composer: wherever that composer can send, which needs a root to
    /// answer.
    pub fn offered_in_thread(has_root: bool, pack_offered: bool) -> bool {
        pack_offered && has_root
    }

    /// What stands between the click and the send (the C#'s `PackSending.For`). A sticker has no
    /// words, so in the family chat or a group it cannot mention the assistant and is never asked
    /// about; in the assistant's own chat everything is. `agreed_at` is `/me`'s
    /// `assistant_consent_at`.
    pub fn gate_for(
        chat_kind: Option<&str>,
        has_assistant: bool,
        processor: Option<&str>,
        agreed_at: Option<&str>,
    ) -> Gate {
        if assistant_consent::is_required(chat_kind, Some(""), processor, agreed_at) {
            Gate::Ask
        } else if assistant_consent::is_withheld_from_an_unnamed_assistant(
            chat_kind,
            Some(""),
            has_assistant,
            processor,
        ) {
            Gate::Withheld
        } else {
            Gate::Send
        }
    }
}

/// What is decided about a picture somebody chose for the pack, BEFORE anything is uploaded
/// (`PackPicking`).
///
/// ANY STILL PICTURE THIS MACHINE CAN DECODE MAY BE CHOSEN. What a sticker IS on the wire is a WebP
/// or a PNG; what somebody may pick to make one from is wider than that.
///
/// A FINISHED STICKER IS TAKEN AS GIVEN. A WebP or a PNG within the byte ceiling goes up untouched,
/// whatever its pixel size and whether or not it moves: re-encoding somebody's finished sticker
/// buys nothing, and for an animated one is not possible.
///
/// ANYTHING ELSE IS MADE INTO ONE — a JPEG, a HEIC, a still GIF, or a still WebP or PNG over the
/// byte ceiling and larger than the box: fitted WHOLE into 512 × 512, its proportions and its
/// transparency kept, never scaled up, and written as PNG. NEVER through the photo path, which draws
/// on white and writes a JPEG. A WebP or PNG over the ceiling that already fits the box is refused
/// instead: redrawing the same pixels would not make them fewer bytes. (The decoding and the
/// writing are the shell's — GdkPixbuf/GdkTexture; what to do is decided here.)
///
/// AN ANIMATED PICTURE THAT IS NOT ALREADY AN ACCEPTABLE WEBP IS REFUSED, IN WORDS — an animated
/// GIF, or an animated PNG this client would have to redraw. Making a sticker of it would keep one
/// frame and quietly throw the rest away, and the person who picked a moving picture would get a
/// still one with no word about why. An animated WebP over the ceiling is simply too big: it is the
/// right kind of file, and nothing here can make it smaller.
///
/// Not `fc_text::pack::plan`, which has no "full" (the web checks the room elsewhere), refuses a
/// still WebP or PNG over the ceiling that already fits the box only after a remake, and names no
/// target size.
pub mod pack_picking {
    use fc_core::protocol::{ApiError, error_codes};
    use fc_core::store::PackLimits;
    use fc_core::text::sticker_file;
    use fc_text::pack;

    use super::over;

    /// The code of the "animated stickers must be WebP" refusal. Made here, never by the server.
    pub const ANIMATED_CODE: &str = "pack_animated_not_webp";

    /// What to do with a chosen picture.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Step {
        /// Upload these bytes as they are.
        Take,
        /// A still picture that is not a finished sticker: fit it into 512 × 512 as PNG, then ask
        /// again ([`refusal`]).
        Make,
        /// Animated and not WebP, too big, or the pack is full. [`Plan::refused`] says which.
        Refuse,
    }

    /// What [`plan_for`] decided.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Plan {
        pub what: Step,
        pub refused: Option<ApiError>,
        /// The pixel size to draw a [`Step::Make`] at; `None` when the header did not say — the
        /// decoder's own size, fitted ([`fit`]), then.
        pub target: Option<(i32, i32)>,
    }

    impl Plan {
        fn step(what: Step) -> Plan {
            Plan {
                what,
                refused: None,
                target: None,
            }
        }

        fn refuse(refused: ApiError) -> Plan {
            Plan {
                what: Step::Refuse,
                refused: Some(refused),
                target: None,
            }
        }
    }

    /// What to do with a picture as it was chosen (the C#'s `PackPicking.For`). `held` is how many
    /// items the pack holds now.
    pub fn plan_for(bytes: &[u8], limits: PackLimits, held: usize) -> Plan {
        // The most basic thing wrong first: what the picture IS, then its size, and only then
        // whether the pack is full.
        let full = !pack::has_room(held, i64::from(limits.max_items));
        let Some(mime) = pack::type_of(bytes) else {
            // Not a WebP and not a PNG. A moving one cannot be made a sticker without losing the
            // movement; a still one is for the decoder to read — and to refuse, if it cannot.
            if pack::is_gif(bytes) && pack::is_animated(bytes) {
                return Plan::refuse(animated());
            }
            return if full {
                Plan::refuse(full_pack())
            } else {
                Plan::step(Step::Make)
            };
        };
        if !over(bytes.len(), limits.max_item_bytes) {
            return if full {
                Plan::refuse(full_pack())
            } else {
                Plan::step(Step::Take)
            };
        }
        if pack::is_animated(bytes) {
            // Never re-encoded. A WebP is the right kind and too big; an animated PNG would come
            // out of the redraw as its first frame.
            return Plan::refuse(if mime == pack::WEBP {
                too_large()
            } else {
                animated()
            });
        }
        let size = sticker_file::size(bytes);
        if let Some((width, height)) = size
            && width <= pack::EDGE
            && height <= pack::EDGE
        {
            return Plan::refuse(too_large());
        }
        if full {
            return Plan::refuse(full_pack());
        }
        Plan {
            what: Step::Make,
            refused: None,
            target: size.map(|(width, height)| {
                fit(
                    i32::try_from(width).unwrap_or(i32::MAX),
                    i32::try_from(height).unwrap_or(i32::MAX),
                )
            }),
        }
    }

    /// The refusal for bytes that are about to be uploaded AS THEY ARE, or `None` when they may
    /// go: what [`super::PackModel::add`] asks, and what a picture made into a sticker is asked
    /// again.
    pub fn refusal(bytes: &[u8], limits: PackLimits, held: usize) -> Option<ApiError> {
        if pack::type_of(bytes).is_none() {
            Some(not_a_sticker())
        } else if over(bytes.len(), limits.max_item_bytes) {
            Some(too_large())
        } else if !pack::has_room(held, i64::from(limits.max_items)) {
            Some(full_pack())
        } else {
            None
        }
    }

    /// A picture fitted whole into the 512 × 512 box: proportions kept, never scaled up, never
    /// below one pixel (`fc_text::pack::fit`).
    pub fn fit(width: i32, height: i32) -> (i32, i32) {
        let side = |value: i32| u32::try_from(value.max(1)).unwrap_or(1);
        let (width, height) = pack::fit(side(width), side(height));
        (
            i32::try_from(width).unwrap_or(i32::MAX),
            i32::try_from(height).unwrap_or(i32::MAX),
        )
    }

    /// The refusal of a moving picture that is not a WebP — also what the decoder answers when IT
    /// finds more than one frame. Given a refusal's status: a code the server never wrote would
    /// otherwise read as a failure worth retrying.
    pub fn animated() -> ApiError {
        ApiError::new(ANIMATED_CODE, "an animated sticker must be a WebP", 400)
    }

    // The server's own codes, so one table of sentences answers a refusal wherever it was made.
    fn not_a_sticker() -> ApiError {
        ApiError::coded(
            error_codes::INVALID_ATTACHMENT,
            "a sticker is a WebP or a PNG",
        )
    }

    fn too_large() -> ApiError {
        ApiError::coded(
            error_codes::PACK_ITEM_TOO_LARGE,
            "over the pack's per-item ceiling",
        )
    }

    fn full_pack() -> ApiError {
        ApiError::coded(error_codes::PACK_FULL, "the pack is at its ceiling")
    }
}

/// What the window says about the pack (`PackText`).
pub mod pack_text {
    use fc_core::protocol::{ApiError, PackItemDto, error_codes};
    use fc_core::store::PackLimits;
    use fc_core::text::string_catalog::StringCatalog;

    use super::{PackAdded, pack_label, pack_picking};

    /// A refusal in the reader's words. The ceilings are SAID, like `board_full`: the limit is the
    /// family's, and a sticker that silently did not appear is one its adder will go looking for.
    pub fn sentence(error: &ApiError, say: &dyn StringCatalog) -> String {
        match error.code.as_str() {
            error_codes::PACK_ITEM_TOO_LARGE | error_codes::ATTACHMENT_TOO_LARGE => say
                .get("That picture is too big to be a sticker.")
                .to_string(),
            error_codes::INVALID_ATTACHMENT => say
                .get("A sticker has to be a WebP or PNG picture.")
                .to_string(),
            // Said, never silently flattened to one frame.
            pack_picking::ANIMATED_CODE => say.get("Animated stickers must be WebP.").to_string(),
            pack_label::TOO_LONG_CODE => label_too_long(say),
            error_codes::PACK_FULL => say
                .get("The family's stickers are full. Remove one to make room.")
                .to_string(),
            error_codes::NOT_PACK_ITEM_AUTHOR => say
                .get("Only whoever added a sticker, or the family owner, can remove it.")
                .to_string(),
            // A sticker is an upload like any other (docs/protocol.md, "Paid families").
            error_codes::STORAGE_ALLOWANCE_USED => {
                crate::paid_families::upload_refusal(&error.code, say).unwrap_or_default()
            }
            _ if error.transient() => say
                .get("The sticker didn't reach the server. Check your connection and try again.")
                .to_string(),
            _ => say.get("Something went wrong. Try again.").to_string(),
        }
    }

    /// The over-long label, in words — said beside the box it was typed in, before any request.
    pub fn label_too_long(say: &dyn StringCatalog) -> String {
        say.format(
            "A sticker's label can be at most %lld characters.",
            &[&pack_label::MAX_LENGTH],
        )
    }

    /// What an add that was NOT refused says (the C#'s `Sentence(PackAdded, …)`).
    pub fn added(added: PackAdded, say: &dyn StringCatalog) -> String {
        match added {
            PackAdded::AlreadyThere => say.get("The family already has that sticker.").to_string(),
            PackAdded::Added => say.get("Added to family stickers").to_string(),
        }
    }

    /// What a sticker is called to a screen reader: "Sticker", and the few words whoever added it
    /// gave when they gave any. Never drawn over the picture.
    pub fn name(item: &PackItemDto, say: &dyn StringCatalog) -> String {
        match item.label.as_deref() {
            Some(label) if !label.is_empty() => say.format("Sticker: %@", &[&label]),
            _ => say.get("Sticker").to_string(),
        }
    }

    /// "12 of 200 stickers" — how full the pack is, against the ceiling the server named.
    pub fn fullness(held: usize, limits: PackLimits, say: &dyn StringCatalog) -> String {
        say.format("%lld of %lld stickers", &[&held, &limits.max_items])
    }
}
