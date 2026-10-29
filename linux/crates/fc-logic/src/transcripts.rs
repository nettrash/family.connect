//! The text of recordings, on request (docs/protocol.md, "Transcripts on request", issue #62) — a
//! port of `Transcripts.cs`: who may ask for the text of which recording and what a failed ask
//! says ([`transcript_rules`]), where one recording's text stands ([`TranscriptPhase`],
//! [`TranscriptLook`]), and the asking itself, kept on this device ([`TranscriptModel`]).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::rc::Rc;
use std::task::Poll;

use fc_core::protocol::{ApiClient, ApiError, TranscriptDto, error_codes};
use fc_core::rt::{Cancel, Clock, Event};
use fc_core::store::{KeptTranscript, StoreResult, TranscriptStore};

use crate::consented_ask::{self, Outcome};
use crate::transcript_sound::{self, SoundSource, SuppliedSound, TranscriptForm};

/// Who may ask for the text of which recording, and what a failed ask says (docs/protocol.md,
/// "Transcripts on request", issue #62). One place, because the bubble that offers "Show text" and
/// the rule the server enforces must not drift apart: an offer the server then refuses is a button
/// that lies.
///
/// **THE RULE, AS THE SERVER WRITES IT:** a member may ask about a recording on a message THEY
/// sent, in any chat they are in, direct chats included; about the OTHER person's in a direct chat
/// (amended 2026-10-21 — no owner's switch reaches a direct chat) or another member's in a group
/// (amended 2026-10-22); about ANOTHER member's in the family chat (its threads are the same chat)
/// only while the family's `ai_transcripts` is on; and never about the assistant's own messages.
/// The server adds one check this client cannot make — that the other SENDER has agreed to the
/// assistant — and answers it with the same `transcript_not_allowed`, which is drawn as "Not
/// available for this message." The rule is `fc_text::transcript::may_ask`, called.
///
/// **TWO FORMS, CHOSEN BY WHAT THE SERVER CAN READ** ([`form`](transcript_rules::form)): a voice
/// note or audio file stored as a type the provider reads, no bigger than
/// `assistant.transcribe_max_bytes`, goes as the server's own copy; a video, an Ogg file and an
/// oversized recording go as sound this device makes from the file it holds
/// ([`transcript_sound`]) — offered on a device that can make it, whatever the attachment's
/// length: a recording too long to fit is told so when the action is pressed, never hidden.
///
/// The stored-copy test and the failure sentences are Windows' and the Mac's
/// (`TranscriptDoor.storedCopyQualifies`, `TranscriptOutcome`), which agree with each other and
/// with the server where `fc_text::transcript` does not: the type compared EXACTLY (the server's
/// `*mime == held.mime`), a size the attachment does not state never the stored form, the ceiling
/// never above 25 MiB, and a 404 or a 409 "not available" rather than "try again".
pub mod transcript_rules {
    use fc_core::protocol::{ApiError, AssistantDto, AttachmentDto, FamilyDto, error_codes};
    use fc_core::text::string_catalog::StringCatalog;

    use crate::assistant_consent;
    use crate::transcript_sound::{self, TranscriptForm};

    /// The provider's ceiling, the server's default, and never more (25 MiB).
    pub const CEILING_BYTES: i64 = fc_text::transcript::DEFAULT_MAX_BYTES;

    /// The stored types the server sends as they are — exactly its list, compared as it compares
    /// them.
    pub const STORED_TYPES: [&str; 4] = fc_text::transcript::STORED_TYPES;

    /// Can this server turn a recording into text at all, and can the consent screen name who it
    /// goes to? A client that cannot name the processor offers no assistant, and a transcript IS
    /// the assistant's provider.
    pub fn is_available(assistant: Option<&AssistantDto>) -> bool {
        assistant.is_some_and(|assistant| {
            assistant.transcribe && assistant_consent::is_available(assistant.processor.as_deref())
        })
    }

    /// The most bytes the stored form may send here: what the server said, held to the ceiling.
    pub fn max_bytes(assistant: &AssistantDto) -> i64 {
        match assistant.transcribe_max_bytes {
            Some(said) if said > 0 => said.min(CEILING_BYTES),
            _ => CEILING_BYTES,
        }
    }

    /// The rule: may this reader ask about a recording on a message this sender sent, in a chat of
    /// this kind? `chat_kind`: a thread is its chat's, so a family thread is `family` and a group's
    /// `group`. `reader`: who is asking; 0 while unknown, which owns nothing and is offered
    /// nothing. `assistant_id`: the assistant's account, whose messages are never asked about.
    /// `family`: the family as the session holds it, for the owner's `ai_transcripts`.
    ///
    /// `fc_text::transcript::may_ask`, and before it the C#'s own guard: a sender that is nobody
    /// (an id of 0 or less) is never asked about. One difference from the C#, which follows
    /// fc-text: with the reader not known yet, another member's recording in the family chat is
    /// not offered even with the owner's switch on (the C# offered it).
    pub fn may_ask(
        chat_kind: Option<&str>,
        sender_id: i64,
        reader: i64,
        assistant_id: Option<i64>,
        family: Option<&FamilyDto>,
    ) -> bool {
        sender_id > 0
            && fc_text::transcript::may_ask(&fc_text::transcript::Asking {
                chat_kind: chat_kind.unwrap_or(""),
                sender_id,
                my_user_id: reader,
                assistant_user_id: assistant_id,
                family_allows: family.is_some_and(|family| family.ai_transcripts),
            })
    }

    /// Is this attachment one the server sends from its own stored copy — a recording of a type
    /// the provider reads, no bigger than the ceiling? A size the attachment does not state is not
    /// offered: the server would decide, and a button that may or may not work is the worse
    /// answer.
    pub fn sends_stored_copy(attachment: &AttachmentDto, assistant: &AssistantDto) -> bool {
        attachment.kind == "audio"
            && attachment
                .mime
                .as_deref()
                .is_some_and(|mime| STORED_TYPES.contains(&mime))
            && attachment
                .size
                .is_some_and(|size| size > 0 && size <= max_bytes(assistant))
    }

    /// Which form a recording's text is asked for in: the server's stored copy where it can send
    /// it; otherwise sound this device makes — a voice note, an audio file or a video, on a device
    /// that `extracts`, WHATEVER its stated length (one too long to fit is told so at the press,
    /// [`transcript_sound::too_long`]); otherwise none.
    pub fn form(
        attachment: &AttachmentDto,
        assistant: &AssistantDto,
        extracts: bool,
    ) -> TranscriptForm {
        if sends_stored_copy(attachment, assistant) {
            TranscriptForm::Stored
        } else if extracts && matches!(attachment.kind.as_str(), "audio" | "video") {
            TranscriptForm::Supplied
        } else {
            TranscriptForm::None
        }
    }

    /// The whole decision for one recording's "Show text": a server that transcribes, the rule,
    /// and a form to ask in ([`form`]). Text this device already HOLDS (`held`) is always shown —
    /// it was given to this member, and showing it sends nothing anywhere. `extracts`: whether
    /// this device can make the sound itself; without it only the stored form is offered.
    pub fn offers(
        assistant: Option<&AssistantDto>,
        family: Option<&FamilyDto>,
        chat_kind: Option<&str>,
        sender_id: i64,
        reader: i64,
        attachment: &AttachmentDto,
        held: bool,
        extracts: bool,
    ) -> bool {
        if held {
            return true;
        }
        match assistant {
            Some(assistant) if is_available(Some(assistant)) => {
                may_ask(
                    chat_kind,
                    sender_id,
                    reader,
                    Some(assistant.user_id),
                    family,
                ) && form(attachment, assistant, extracts) != TranscriptForm::None
            }
            _ => false,
        }
    }

    /// What a failed ask says under the player. The provider's refusal says so, and is final; a
    /// refusal of THIS recording or THIS member is "not available", and is final too; anything
    /// that says nothing about the request — no connection, the deadline, the provider's own
    /// failure — asks to try again.
    pub fn failure_sentence(error: &ApiError, say: &dyn StringCatalog) -> String {
        let sentence = if may_retry(error) {
            say.get("Couldn't get the text. Try again.")
        } else {
            match error.code.as_str() {
                error_codes::TRANSCRIPT_REFUSED => {
                    say.get("The assistant's provider refused this recording.")
                }
                // What this device could not do with the file, in the words the other three
                // clients use.
                transcript_sound::TOO_LONG_CODE => {
                    say.get("This recording is too long to turn into text.")
                }
                transcript_sound::UNREADABLE_CODE => {
                    say.get("Couldn't read the sound in this file.")
                }
                _ => say.get("Not available for this message."),
            }
        };
        sentence.to_string()
    }

    /// Whether asking again could help: a transient failure, and the consent refusal that came
    /// back after a yes (the server had not caught up; the loop guard stopped this click, not the
    /// next one). Every other refusal is final.
    pub fn may_retry(error: &ApiError) -> bool {
        error.code != error_codes::TRANSCRIPT_REFUSED
            && (error.transient() || error.code == error_codes::ASSISTANT_CONSENT_REQUIRED)
    }

    /// The text as drawn: what was said, or "No speech" for silence — an answer, not an error.
    pub fn shown(text: Option<&str>, say: &dyn StringCatalog) -> String {
        match text {
            Some(text) if !text.trim().is_empty() => text.to_string(),
            _ => say.get("No speech").to_string(),
        }
    }
}

/// Where one recording's text stands on this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranscriptPhase {
    /// Folded away, or never asked for: the "Show text" action.
    Closed,
    /// Asked for and not answered yet: "Getting the text…".
    Asking,
    /// Shown under the player, with "Hide text".
    Open,
    /// The last ask failed; [`TranscriptLook::error`] says why.
    Failed,
}

/// One recording's text as the bubble draws it: where it stands, what was said while
/// [`Open`](TranscriptPhase::Open) (`""` is silence), and why the last ask failed while
/// [`Failed`](TranscriptPhase::Failed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptLook {
    pub phase: TranscriptPhase,
    pub text: Option<String>,
    pub error: Option<ApiError>,
}

impl TranscriptLook {
    /// `new TranscriptLook(phase, text, error)`.
    pub fn new(phase: TranscriptPhase, text: Option<&str>, error: Option<ApiError>) -> Self {
        TranscriptLook {
            phase,
            text: text.map(str::to_string),
            error,
        }
    }

    /// `TranscriptLook.Closed`: the "Show text" action.
    pub fn closed() -> Self {
        TranscriptLook::new(TranscriptPhase::Closed, None, None)
    }

    /// "Getting the text…".
    pub fn asking() -> Self {
        TranscriptLook::new(TranscriptPhase::Asking, None, None)
    }

    /// The text, shown.
    pub fn open(text: &str) -> Self {
        TranscriptLook::new(TranscriptPhase::Open, Some(text), None)
    }

    /// The last ask failed, for this reason.
    pub fn failed(error: ApiError) -> Self {
        TranscriptLook::new(TranscriptPhase::Failed, None, Some(error))
    }

    /// The answer was silence, drawn as "No speech".
    pub fn no_speech(&self) -> bool {
        self.phase == TranscriptPhase::Open
            && self
                .text
                .as_deref()
                .is_none_or(|text| text.trim().is_empty())
    }
}

/// The text of recordings, asked for one at a time and kept on this device (docs/protocol.md,
/// "Transcripts on request"): the request with its own deadline (the core's transcript deadline),
/// the consent question in [`consented_ask`]'s order, and the answer in [`TranscriptStore`] so that
/// reopening a chat shows it without asking again.
///
/// **WHAT IS SHOWN IS THIS MODEL'S; WHAT IS KEPT IS THE STORE'S.** A bubble is rebuilt on every
/// redraw, so whether the text is open, folded or still coming cannot live in the widget; it lives
/// here, per attachment, and the view draws [`look`](Self::look) and redraws on
/// [`on_changed`](Self::on_changed). Folding the text away keeps it: "Show text" again reads the
/// store and sends nothing.
///
/// **ONE ASK PER RECORDING AT A TIME.** A second click while the first is still asking — or still
/// waiting on the consent question — does nothing: the server would have made the second wait on
/// the first call anyway.
///
/// **NOTHING HERE IS LOGGED.** The text of somebody's voice goes to the store and the screen and
/// nowhere else.
///
/// **AN ASK BELONGS TO THE SESSION IT STARTED IN.** One model lives as long as the app, across
/// sign-outs, and an ask can take minutes. [`clear`](Self::clear) — the session ended — stops
/// whatever is still out and makes it stale: an answer that lands afterwards is neither kept nor
/// drawn, because the sign-out has wiped the cache and the next account would otherwise be shown
/// the last one's words without asking, and an answer made from supplied sound must never reach
/// anybody but the member who asked. (The C# cancelled a linked token; here the ask races the
/// session's [`Cancel`], and the losing future — the request, the consent question, the sound
/// being made — is dropped.)
///
/// Every method takes `&self`; the shell holds the model in an `Rc` and spawns
/// [`show`](Self::show) on the main context.
pub struct TranscriptModel {
    api: Rc<ApiClient>,
    store: Rc<TranscriptStore>,
    clock: Rc<dyn Clock>,
    looks: RefCell<HashMap<i64, TranscriptLook>>,
    asking: RefCell<HashSet<i64>>,
    /// Which session an ask started in: [`clear`](Self::clear) moves it on, and an ask from before
    /// writes nothing.
    session: Cell<u64>,
    /// Raised by [`clear`](Self::clear): whatever the ended session still had out stops.
    ended: RefCell<Cancel>,
    changed: Event<i64>,
}

impl TranscriptModel {
    /// `new TranscriptModel(api, store, clock)` — the clock dates what is kept.
    pub fn new(api: Rc<ApiClient>, store: Rc<TranscriptStore>, clock: Rc<dyn Clock>) -> Self {
        TranscriptModel {
            api,
            store,
            clock,
            looks: RefCell::default(),
            asking: RefCell::default(),
            session: Cell::new(0),
            ended: RefCell::new(Cancel::new()),
            changed: Event::new(),
        }
    }

    /// A recording's look changed: the attachment id.
    pub fn on_changed(&self) -> &Event<i64> {
        &self.changed
    }

    /// How this recording's text is drawn now.
    pub fn look(&self, attachment_id: i64) -> TranscriptLook {
        self.looks
            .borrow()
            .get(&attachment_id)
            .cloned()
            .unwrap_or_else(TranscriptLook::closed)
    }

    /// Whether this device holds text for this recording — which it may show without asking
    /// anybody.
    pub fn holds(&self, attachment_id: i64) -> StoreResult<bool> {
        Ok(self.store.find(attachment_id)?.is_some())
    }

    /// "Show text" in the stored form (the C#'s shorter `ShowAsync`): the kept text at once when
    /// this device holds it, and otherwise the question (when it is needed) and the request.
    pub async fn show_stored<Asked>(
        &self,
        chat_id: i64,
        message_id: i64,
        attachment_id: i64,
        asks_first: impl FnOnce() -> bool,
        ask: impl FnMut() -> Asked,
    ) -> StoreResult<()>
    where
        Asked: Future<Output = bool>,
    {
        self.show(
            chat_id,
            message_id,
            attachment_id,
            TranscriptForm::Stored,
            None,
            asks_first,
            ask,
        )
        .await
    }

    /// "Show text" in the form [`transcript_rules::form`] chose: the kept text at once when this
    /// device holds it, and otherwise the question (when it is needed — `asks_first` is read when
    /// the click lands, [`crate::backdrop_consent::asks_first`]; `ask` is the consent question,
    /// `true` only on a yes the server has kept) and the request. Ends with the look set — open,
    /// failed, or closed again when the member said Not Now.
    ///
    /// [`TranscriptForm::Supplied`] makes the sound with `sound` AFTER the consent question —
    /// nothing is downloaded or decoded for a member who says Not Now — and at most once per click,
    /// however many times the request goes. [`TranscriptForm::Stored`] asks for the server's copy,
    /// and falls back to the device's sound once if the server answers `not_transcribable` ("a
    /// client that sent no body may send the sound track instead"). `sound` is `None` where this
    /// device cannot make it, which leaves only the stored form.
    ///
    /// An answer made from supplied sound is kept on THIS device only, marked as such
    /// ([`KeptTranscript::supplied`]): the server never kept it, and it never replaces an answer
    /// from the stored copy. `Err` is the cache refusing to be read or written.
    pub async fn show<Asked>(
        &self,
        chat_id: i64,
        message_id: i64,
        attachment_id: i64,
        form: TranscriptForm,
        sound: Option<SoundSource>,
        asks_first: impl FnOnce() -> bool,
        mut ask: impl FnMut() -> Asked,
    ) -> StoreResult<()>
    where
        Asked: Future<Output = bool>,
    {
        if !self.asking.borrow_mut().insert(attachment_id) {
            return Ok(());
        }
        let mine = self.session.get();
        let ended = self.ended.borrow().clone();
        // The C#'s `finally`: gone however this ends, the future dropped included — unless the
        // session that started it has ended, which took its asks with it, and the set may now
        // hold the next session's.
        let _asking = AskingGuard {
            model: self,
            attachment_id,
            session: mine,
        };
        if let Some(kept) = self.store.find(attachment_id)? {
            self.set(attachment_id, TranscriptLook::open(&kept.text), Some(mine));
            return Ok(());
        }
        if form == TranscriptForm::None || (form == TranscriptForm::Supplied && sound.is_none()) {
            // Nothing to ask with — the view offers no button then — so it is said, never sent.
            self.set(
                attachment_id,
                TranscriptLook::failed(transcript_sound::unavailable()),
                Some(mine),
            );
            return Ok(());
        }
        let click = Click {
            model: self,
            chat_id,
            message_id,
            attachment_id,
            form,
            sound: sound.as_ref(),
            session: mine,
            ended: &ended,
            made: RefCell::new(None),
            supplied: Cell::new(false),
        };
        let asked = consented_ask::run(asks_first, &mut ask, || click.request());
        // The session's end wins the race: the request, the question and the sound in the making
        // are dropped with the click, and nothing of it is kept or drawn.
        let mut asked = pin!(asked);
        let mut stopped = ended.cancelled();
        let outcome: Option<Outcome<TranscriptDto>> = poll_fn(|cx| {
            if Pin::new(&mut stopped).poll(cx).is_ready() {
                return Poll::Ready(None);
            }
            asked.as_mut().poll(cx).map(Some)
        })
        .await;
        let Some(outcome) = outcome else {
            return Ok(());
        };
        if outcome.declined {
            self.set(attachment_id, TranscriptLook::closed(), Some(mine));
        } else if let Some(TranscriptDto {
            text: Some(text),
            language,
        }) = outcome.value
        {
            if self.session.get() != mine {
                // Signed out while it was on its way: the cache was wiped, and this is not the next
                // account's.
                return Ok(());
            }
            self.store.keep(
                &KeptTranscript {
                    attachment_id,
                    text: text.clone(),
                    language,
                    supplied: click.supplied.get(),
                },
                self.clock.now(),
            )?;
            self.set(attachment_id, TranscriptLook::open(&text), Some(mine));
        } else {
            let error = outcome
                .error
                .unwrap_or_else(|| ApiError::transport("no answer"));
            self.set(attachment_id, TranscriptLook::failed(error), Some(mine));
        }
        Ok(())
    }

    /// "Hide text": folded away, and still kept — showing it again asks nobody.
    pub fn hide(&self, attachment_id: i64) {
        let open = self
            .looks
            .borrow()
            .get(&attachment_id)
            .is_some_and(|look| look.phase == TranscriptPhase::Open);
        if open {
            self.set(attachment_id, TranscriptLook::closed(), None);
        }
    }

    /// Forget what is on screen — a session that ended — and stop whatever is still being asked:
    /// an answer that lands after this is not kept and not drawn. The store is wiped with the rest
    /// of the cache.
    pub fn clear(&self) {
        self.looks.borrow_mut().clear();
        self.asking.borrow_mut().clear();
        self.session.set(self.session.get() + 1);
        let stopping = self.ended.replace(Cancel::new());
        stopping.cancel();
    }

    /// The look, unless it comes from a session that has since ended (`from`).
    fn set(&self, attachment_id: i64, look: TranscriptLook, from: Option<u64>) {
        if from.is_some_and(|started| started != self.session.get()) {
            return;
        }
        self.looks.borrow_mut().insert(attachment_id, look);
        self.changed.emit(&attachment_id);
    }

    /// The request in one form: no body for the server's stored copy, the supplied sound
    /// otherwise.
    async fn request(
        &self,
        chat_id: i64,
        message_id: i64,
        attachment_id: i64,
        sound: Option<Vec<u8>>,
    ) -> Result<TranscriptDto, ApiError> {
        let answer = match sound {
            None => {
                self.api
                    .transcript(chat_id, message_id, attachment_id)
                    .await
            }
            Some(bytes) => {
                self.api
                    .transcript_of_sound(chat_id, message_id, attachment_id, bytes)
                    .await
            }
        };
        let status = answer.status;
        match answer.outcome {
            // A 2xx whose body could not be decoded at all (a text of the wrong type, say) is the
            // same unreadable answer as one decoded with its text missing, and said the same way;
            // a server's own validation refusal is a 4xx.
            Err(error)
                if error.code == error_codes::VALIDATION && (200..300).contains(&error.status) =>
            {
                Err(unreadable_answer(error.status))
            }
            Err(error) => Err(error),
            // TEXT IS ALWAYS PRESENT, and "" is silence (docs/protocol.md, "Transcripts on
            // request"). The decoder reads a missing "text" and a "text": null alike as None, so a
            // 2xx with no transcript, or no text in it, is checked HERE — never turned into "",
            // which would keep a malformed answer for ever and draw it as "No speech".
            Ok(response) => match response.transcript {
                Some(transcript) if transcript.text.is_some() => Ok(transcript),
                _ => Err(unreadable_answer(status)),
            },
        }
    }
}

/// One click's state, shared by the request the consent question may send twice.
struct Click<'a> {
    model: &'a TranscriptModel,
    chat_id: i64,
    message_id: i64,
    attachment_id: i64,
    form: TranscriptForm,
    sound: Option<&'a SoundSource>,
    session: u64,
    ended: &'a Cancel,
    /// Made once per click: the consent question may send the request twice, and the sound is the
    /// same. Nothing outlives the click: the next one makes the sound again.
    made: RefCell<Option<SuppliedSound>>,
    /// Whether the answer, if any, came from supplied sound.
    supplied: Cell<bool>,
}

impl Click<'_> {
    /// One go of the request: "Getting the text…", then the form's request — and the device's
    /// sound after a stored copy the server cannot send.
    async fn request(&self) -> Result<TranscriptDto, ApiError> {
        self.model.set(
            self.attachment_id,
            TranscriptLook::asking(),
            Some(self.session),
        );
        if self.form == TranscriptForm::Supplied {
            return self.supply().await;
        }
        let stored = self
            .model
            .request(self.chat_id, self.message_id, self.attachment_id, None)
            .await;
        match stored {
            Err(error) if error.code == error_codes::NOT_TRANSCRIBABLE && self.sound.is_some() => {
                self.supply().await
            }
            other => other,
        }
    }

    /// The device's sound, made at most once, and the request that carries it.
    async fn supply(&self) -> Result<TranscriptDto, ApiError> {
        let Some(sound) = self.sound else {
            return Err(transcript_sound::unreadable());
        };
        if self.made.borrow().is_none() {
            let made = make(sound, self.ended.clone()).await;
            *self.made.borrow_mut() = Some(made);
        }
        let bytes = match self.made.borrow().as_ref() {
            Some(SuppliedSound::Made(bytes)) => bytes.clone(),
            Some(SuppliedSound::Failed(error)) => return Err(error.clone()),
            None => return Err(transcript_sound::unreadable()),
        };
        self.supplied.set(true);
        self.model
            .request(
                self.chat_id,
                self.message_id,
                self.attachment_id,
                Some(bytes),
            )
            .await
    }
}

/// Removes the click's attachment from the asking set when the click ends or is dropped — unless
/// its session has ended.
struct AskingGuard<'a> {
    model: &'a TranscriptModel,
    attachment_id: i64,
    session: u64,
}

impl Drop for AskingGuard<'_> {
    fn drop(&mut self) {
        if self.model.session.get() == self.session {
            self.model.asking.borrow_mut().remove(&self.attachment_id);
        }
    }
}

/// The device's sound, held to the ceiling and to MPEG-4 — the server's own checks, made here so
/// that a sound it would refuse is said before it is uploaded: over the ceiling is "too long",
/// empty or not MPEG-4 "couldn't read". A stated length that cannot fit is "too long" before
/// anything is downloaded. A maker that failed outright is "couldn't read".
async fn make(sound: &SoundSource, cancel: Cancel) -> SuppliedSound {
    if !transcript_sound::can_fit(sound.duration_ms, sound.max_bytes) {
        return SuppliedSound::Failed(transcript_sound::too_long());
    }
    let made = match (sound.make)(cancel).await {
        Ok(made) => made,
        Err(_) => return SuppliedSound::Failed(transcript_sound::unreadable()),
    };
    if let SuppliedSound::Made(bytes) = &made {
        if bytes.is_empty() || !fc_text::media::matches_magic(transcript_sound::MIME, bytes) {
            return SuppliedSound::Failed(transcript_sound::unreadable());
        }
        let length = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
        if !transcript_sound::fits(length, sound.max_bytes) {
            return SuppliedSound::Failed(transcript_sound::too_long());
        }
    }
    made
}

/// A 2xx this client cannot read: a failure, not an answer — nothing is kept, so the next click
/// asks again — and said as the generic "Couldn't get the text. Try again.", as the other three
/// clients say an answer they cannot decode. `internal`, the server's own failure, is what that
/// sentence is for.
fn unreadable_answer(status: u16) -> ApiError {
    ApiError::new(
        error_codes::INTERNAL,
        "the answer carried no transcript text",
        status,
    )
}
