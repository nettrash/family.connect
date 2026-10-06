//! Voice and video messages from the Send button (issue #79): the rules every
//! composer follows, written once (docs/audio-video-messages-2026-10-04.md —
//! "the plan" below — under "Where it plugs in", "Shared rules").
//!
//! Five codebases put a microphone in the Send slot, and the plan's whole
//! difficulty is that they must behave the same for a finger, a pen, a mouse,
//! a keyboard and a screen reader. Three decisions live here:
//!
//! - [`composer_slot`] — which control the composer's trailing slot is, every
//!   row of the plan's S1.3, first match wins.
//! - [`video_door`] — whether the video button inside the empty field is
//!   hidden, dimmed or shown (S1.4).
//! - [`hold_step`] — what an activation of the slot, the menu or the shortcut,
//!   Stop, Delete, the length limit or an interruption does to a voice
//!   recording (S2.1, S2.2, S2.5), as a reducer: a state and an event go in,
//!   the next state and what to do come out.
//!
//! and the round video's arithmetic ([`round_cap_ms`], [`round_warning_ms`],
//! [`round_diameter`], [`is_round`]) and the S1.1 constants.
//!
//! **Revised 2026-10-06: there is no hold.** The first draft also made the
//! microphone a walkie-talkie on touch — hold to talk, slide to cancel or to
//! lock, let go to send after a five-second Undo window. The owner removed it
//! after testing it on his iPhone, with everything that existed only for it.
//! The microphone does ONE thing: its activation starts a hands-free
//! recording, which the slot's Send arrow sends, Stop keeps for review and
//! Delete deletes. A long press on it is not a gesture: nothing records and
//! nothing opens while a finger is down — no menu, no callout, no context
//! menu — and the press is the platform button's ordinary tap when it lifts
//! inside, however long it was held, so a slow or unsteady press is never a
//! dead button. The reducer keeps its names (`hold_step`, [`HoldState`],
//! [`HoldEvent`], [`HoldEffect`], [`HoldConstants`]) so that the ports' types
//! keep theirs; only the hold's own parts are gone.
//!
//! It is only the DECISION. What the composer is, as a client saw it, goes in;
//! recording, playing, drawing, staging and parking are each platform's own
//! business. The web client runs this module as it is. The Apple and Android
//! ports are held to every rule here, and the Windows port to the slot, the
//! door and the round helpers (S8.6), by the vectors `win/tools/board-oracle`
//! prints from it (`cargo run -- record`), committed as `record-vectors.json`
//! beside each port's tests — "an oracle, not four readings". So the names are
//! plain on purpose, and every field is one a Swift struct, a Kotlin data class
//! and a C# record can carry under the same name.
//!
//! Words are the apps' English source strings — the catalogue's keys (S10) —
//! and each client says them in its reader's language. Nothing here
//! translates, so the vectors read the same in every language.
//!
//! Every function is total. Times are milliseconds on one monotonic clock.

// --- S1.1, the constants ------------------------------------------------------------------------

/// The slot ignores activation for this long only after its OWN activation
/// changed it — a send that empties the composer, a tap that starts a
/// recording, a Send that ends one or finds it too short, a Stop in row 3
/// that stages the note. An activation while it runs is ignored whole. A
/// change made by typing, pasting or staging is never guarded — it lifts the
/// guard ([`HoldEvent::OtherAction`]) — so "ok" followed at once by Send still
/// sends. The video button ignores activation for the same 600 ms after it
/// appears.
pub const ACTIVATION_GUARD_MS: u64 = 600;

/// Nothing shorter is ever sent or staged: "That recording was too short."
/// It replaces the recorders' 1024-byte rule as the floor people see.
pub const SHORTEST_RECORDING_MS: u64 = 1_000;

/// A voice note's length, unchanged (docs/protocol.md, "A browser is a client
/// too"), and the moment "30 seconds left" is shown and announced. At the cap
/// the recording stops into review — a length limit never sends.
pub const VOICE_CAP_MS: u64 = 300_000;
pub const VOICE_WARNING_MS: u64 = 270_000;

/// `max_round_video_ms` when a server has round video (it always says so —
/// the key's absence is how a client knows to offer no video at all).
pub const DEFAULT_MAX_ROUND_VIDEO_MS: u64 = 60_000;

/// A round video stops this much before `max_round_video_ms` — room for
/// AAC's padding — and warns this much before it.
pub const ROUND_CAP_MARGIN_MS: u64 = 500;
pub const ROUND_WARNING_LEAD_MS: u64 = 10_000;

/// Digital silence — a muted microphone, not a quiet room: no PEAK above
/// −60 dBFS. Each platform reads its own measure of the same level: iOS and
/// the Mac `peakPower` at or below −60 dB, Android `getMaxAmplitude()` at or
/// below 32, the web a sample magnitude at or below 0.001.
pub const SILENCE_PEAK_DBFS: f64 = -60.0;
pub const SILENCE_MAX_AMPLITUDE: u32 = 32;
pub const SILENCE_SAMPLE_MAGNITUDE: f64 = 0.001;

/// "We can't hear anything. Is the microphone muted?" when nothing has risen
/// above the silence level this long after a recording started.
pub const SILENCE_WARNING_AFTER_MS: u64 = 3_000;

/// Deleting a recording this long or longer asks first.
pub const DELETE_ASKS_FROM_MS: u64 = 10_000;

/// The video recorder's PREVIEW closes after this long with no control used.
pub const PREVIEW_IDLE_CLOSE_MS: u64 = 60_000;

/// The slot's Send ↔ microphone cross-fade and the recorder's fade; none
/// under Reduce Motion, Remove animations or `prefers-reduced-motion`.
pub const SLOT_CROSSFADE_MS: u64 = 150;
pub const RECORDER_FADE_MS: u64 = 200;

/// The least hit area on a coarse pointer, in each platform's unit: the hit
/// area grows, the visual and the bar do not.
pub const MIN_TARGET_APPLE_PT: u32 = 44;
pub const MIN_TARGET_ANDROID_DP: u32 = 48;
pub const MIN_TARGET_WINDOWS_EPX: u32 = 44;
pub const MIN_TARGET_WEB_PX: u32 = 44;

/// A received circle's diameter (S5.2): compact widths, and everything else.
pub const ROUND_DIAMETER_COMPACT: u32 = 200;
pub const ROUND_DIAMETER_REGULAR: u32 = 240;

// --- S1.3, the trailing slot --------------------------------------------------------------------

/// The voice recording the composer is showing, as far as the slot is
/// concerned ([`HoldState::recording`] says it of a reducer's state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Recording {
    /// No voice recording runs.
    #[default]
    None,
    /// Hands-free, started with the composer empty: row 2, the Send arrow.
    HandsFree,
    /// Hands-free, started from the paperclip or the shortcut with words typed
    /// or items staged: row 3, the Stop square — the words are hidden behind
    /// the row and must not leave unseen.
    HandsFreeBesideDraft,
}

/// Why the microphone is dimmed — rows 7, 8 and 9, in that order of
/// precedence. Dimmed is not disabled: the control stays focusable and
/// hittable and says why when activated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimmed {
    /// A call in any phase but idle or ended.
    Call,
    /// The composer's attachment guard (iOS `mediaState.blocksComposer`,
    /// Android `mediaState.isBusy`, Windows `strip.Preparing || sendingMedia`,
    /// the web `preparing || locating`).
    Busy,
    /// The chat holds a "Voice message not sent" row (S2.8).
    NotSent,
}

impl Dimmed {
    /// The sentence the composer's notice line says, and a screen reader hears
    /// with the control.
    pub fn notice(self) -> &'static str {
        match self {
            Dimmed::Call => "You can record a message after the call.",
            Dimmed::Busy => "Wait until the current attachment is done.",
            Dimmed::NotSent => "Send or delete the voice message that wasn't sent first.",
        }
    }
}

/// What the composer is, for the slot (S1.2). Thread composers are not
/// changed by this plan and do not ask: no microphone, no recording, today's
/// Send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotInputs {
    /// The video recorder is open (S3) — it owns the row.
    pub recorder_open: bool,
    pub recording: Recording,
    /// An edit is open.
    pub editing: bool,
    /// The draft is blank after trimming whitespace — the client's own trim;
    /// the rule only asks the answer.
    pub draft_blank: bool,
    /// Anything is staged. A primed reply is not: whatever is recorded
    /// carries it.
    pub staged: bool,
    /// The assistant's chat (`kind = ai`).
    pub assistant_chat: bool,
    /// The platform can record sound at all — on the web, a secure context
    /// with `navigator.mediaDevices`.
    pub can_record: bool,
    /// A call in any phase but idle or ended.
    pub call: bool,
    /// The composer's attachment guard ([`Dimmed::Busy`]).
    pub busy: bool,
    /// The chat holds a not-sent voice message.
    pub not_sent: bool,
}

impl SlotInputs {
    /// S1.2's **empty**: the draft is blank AND nothing is staged.
    pub fn empty(&self) -> bool {
        self.draft_blank && !self.staged
    }
}

/// What the slot shows and does. Each is one row of S1.3 ([`Slot::row`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Row 1: the video recorder owns the row.
    Recorder,
    /// Row 2: the Send arrow — stops and sends (S2.5).
    SendVoice,
    /// Row 3: the Stop square — stops; the note is staged beside the words.
    StopRecording,
    /// Row 4: today's Save, disabled while the field is blank. Never a
    /// microphone, even with the field cleared.
    Save { enabled: bool },
    /// Row 5: Send, by today's rules.
    Send,
    /// Row 6: Send, disabled — today's look, in the assistant's chat or where
    /// nothing can record.
    SendDisabled,
    /// Rows 7–9: the microphone, dimmed; activating it says why.
    Dimmed(Dimmed),
    /// Row 10: the microphone — a hands-free recording on activation. A long
    /// press is not a gesture: it records nothing and opens nothing while the
    /// finger is down, and it is a tap when it lifts inside.
    Microphone,
}

impl Slot {
    /// The S1.3 row this is.
    pub fn row(self) -> u8 {
        match self {
            Slot::Recorder => 1,
            Slot::SendVoice => 2,
            Slot::StopRecording => 3,
            Slot::Save { .. } => 4,
            Slot::Send => 5,
            Slot::SendDisabled => 6,
            Slot::Dimmed(Dimmed::Call) => 7,
            Slot::Dimmed(Dimmed::Busy) => 8,
            Slot::Dimmed(Dimmed::NotSent) => 9,
            Slot::Microphone => 10,
        }
    }

    /// Its accessibility label (S6), or None where the recorder owns the row.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Slot::Recorder => None,
            Slot::SendVoice => Some("Send voice message"),
            Slot::StopRecording => Some("Stop recording"),
            Slot::Save { .. } => Some("Save"),
            Slot::Send | Slot::SendDisabled => Some("Send"),
            Slot::Dimmed(_) | Slot::Microphone => Some("Record voice message"),
        }
    }

    /// What activating it says instead of acting: a dimmed microphone's reason.
    pub fn notice(self) -> Option<&'static str> {
        match self {
            Slot::Dimmed(reason) => Some(reason.notice()),
            _ => None,
        }
    }

    /// "The slot is a microphone" — rows 7 to 10, which is where the video
    /// button may show (S1.4).
    pub fn is_microphone(self) -> bool {
        matches!(self, Slot::Dimmed(_) | Slot::Microphone)
    }
}

/// The composer's trailing slot (S1.3): the first matching row wins.
pub fn composer_slot(inputs: &SlotInputs) -> Slot {
    if inputs.recorder_open {
        return Slot::Recorder;
    }
    match inputs.recording {
        Recording::HandsFree => return Slot::SendVoice,
        Recording::HandsFreeBesideDraft => return Slot::StopRecording,
        Recording::None => {}
    }
    if inputs.editing {
        return Slot::Save {
            enabled: !inputs.draft_blank,
        };
    }
    if !inputs.empty() {
        return Slot::Send;
    }
    if inputs.assistant_chat || !inputs.can_record {
        return Slot::SendDisabled;
    }
    if inputs.call {
        return Slot::Dimmed(Dimmed::Call);
    }
    if inputs.busy {
        return Slot::Dimmed(Dimmed::Busy);
    }
    if inputs.not_sent {
        return Slot::Dimmed(Dimmed::NotSent);
    }
    Slot::Microphone
}

// --- S1.4, the video button ---------------------------------------------------------------------

/// The video button's label, and its desktop and pointer tooltip.
pub const VIDEO_DOOR_LABEL: &str = "Record video message";
pub const VIDEO_DOOR_TOOLTIP: &str = "Record a video message";

/// What the video button needs to know besides the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorInputs {
    pub slot: SlotInputs,
    /// The chat's main composer, in a family or a direct chat — never the
    /// assistant's chat, never a thread.
    pub family_or_direct_chat: bool,
    /// The server sends `max_round_video_ms` on `GET /families/mine`.
    pub server_offers_round: bool,
    /// The device has a camera.
    pub has_camera: bool,
    /// On the web, the encoder probe passes (S8.7); `true` everywhere else.
    pub encoder_probe_passes: bool,
    /// THIS build records round video on this platform — its Phase 3 has
    /// shipped (on Windows, 3d). A build that can only receive circles shows
    /// no video entry at all (Decision 40).
    pub records_round_video: bool,
}

impl DoorInputs {
    /// S1.2's **round available**: all four.
    pub fn round_available(&self) -> bool {
        self.server_offers_round
            && self.has_camera
            && self.encoder_probe_passes
            && self.records_round_video
    }
}

/// The video button inside the empty field (S1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Door {
    /// Not drawn — the field gets its width back.
    Hidden,
    /// Drawn dimmed, and saying the slot's own sentence: rows 7 and 8 only.
    Dimmed(Dimmed),
    /// Drawn, and activating it opens the recorder (S3).
    Shown,
}

impl Door {
    /// Its label, where it is drawn.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Door::Hidden => None,
            Door::Dimmed(_) | Door::Shown => Some(VIDEO_DOOR_LABEL),
        }
    }

    /// What activating a dimmed button says.
    pub fn notice(self) -> Option<&'static str> {
        match self {
            Door::Dimmed(reason) => Some(reason.notice()),
            _ => None,
        }
    }
}

/// Whether the video button is hidden, dimmed or shown (S1.4): shown when the
/// slot is a microphone (rows 7–10) in a family or direct chat with round
/// video available; dimmed, with the same sentence, in rows 7 and 8; usable in
/// row 9, because the not-sent rule is about voice. Hidden otherwise — as soon
/// as a character is typed, while anything is staged, while editing or
/// recording, in the assistant's chat and in threads,
/// against a server without the keys, on a device without a camera, in a
/// browser whose probe fails, and in every build that does not record round
/// video.
pub fn video_door(inputs: &DoorInputs) -> Door {
    if !inputs.family_or_direct_chat || !inputs.round_available() {
        return Door::Hidden;
    }
    match composer_slot(&inputs.slot) {
        Slot::Dimmed(reason @ (Dimmed::Call | Dimmed::Busy)) => Door::Dimmed(reason),
        Slot::Dimmed(Dimmed::NotSent) | Slot::Microphone => Door::Shown,
        _ => Door::Hidden,
    }
}

// --- The round video's arithmetic ---------------------------------------------------------------

/// Where a round video recording stops: `max_round_video_ms` − 500 ms, 59.5 s
/// at the server's 60 000. A limit shorter than the margin is 0, never a
/// wrapped-around eternity.
pub fn round_cap_ms(max_round_video_ms: u64) -> u64 {
    max_round_video_ms.saturating_sub(ROUND_CAP_MARGIN_MS)
}

/// Where "10 seconds left" is shown and announced and the ring turns orange:
/// `max_round_video_ms` − 10 000 ms, 50 s at 60 000. A limit shorter than ten
/// seconds warns from the start (0).
pub fn round_warning_ms(max_round_video_ms: u64) -> u64 {
    max_round_video_ms.saturating_sub(ROUND_WARNING_LEAD_MS)
}

/// How wide a window draws a received circle (S5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthClass {
    /// A compact horizontal size class on Apple (every iPhone in portrait, an
    /// iPad in Slide Over or a narrow Split View), an Android window under
    /// 600 dp, the web under 720 px.
    Compact,
    /// Everything else: iPad regular, the Mac, Android at 600 dp and wider,
    /// Windows, the web at 720 px and wider.
    Regular,
}

/// A received circle's diameter, in the platform's unit — a RECOMMENDATION
/// nothing on the wire carries: larger than a sticker (160), smaller than a
/// video tile, and 480 pixels fill a 240-unit circle at 2×.
pub fn round_diameter(width: WidthClass) -> u32 {
    match width {
        WidthClass::Compact => ROUND_DIAMETER_COMPACT,
        WidthClass::Regular => ROUND_DIAMETER_REGULAR,
    }
}

/// What [`is_round`] reads of one attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentFlags<'a> {
    /// The attachment's `kind`, as the wire spells it (`"video"`).
    pub kind: &'a str,
    /// Its `round` — absent on the wire is false.
    pub round: bool,
}

/// The drawing test (S5.1), one test on every client, mirroring the
/// sticker's: exactly one attachment, `kind = video`, carrying `round: true`,
/// and no body. Anything else — two attachments, the flag on a photo, a body —
/// is drawn as the ordinary message it otherwise is. The body is compared
/// EXACTLY: the server stores an attachment message whose body trims to
/// nothing as `""`, so a round message it accepted always has the empty body,
/// and no port's idea of whitespace can make two clients disagree.
pub fn is_round(body: &str, attachments: &[AttachmentFlags]) -> bool {
    body.is_empty() && matches!(attachments, [only] if only.kind == "video" && only.round)
}

// --- S2.1, S2.2 and S2.5, the recording, as a reducer ------------------------------------------

/// The numbers [`hold_step`] decides by: S1.1's. They are constants; they
/// travel as a value so that the vectors can say which ones a case used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldConstants {
    pub shortest_recording_ms: u64,
    pub activation_guard_ms: u64,
    pub delete_asks_from_ms: u64,
}

impl Default for HoldConstants {
    fn default() -> Self {
        HoldConstants {
            shortest_recording_ms: SHORTEST_RECORDING_MS,
            activation_guard_ms: ACTIVATION_GUARD_MS,
            delete_asks_from_ms: DELETE_ASKS_FROM_MS,
        }
    }
}

/// The microphone permission, as the platform reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Permission {
    #[default]
    Granted,
    /// Never asked: activation raises the system's prompt.
    NotAsked,
    /// Refused: the denial notice, with Open Settings where the platform can.
    Denied,
}

/// The facts a decision reads at the moment it is made. A port fills it from
/// what it knows; the defaults are a granted microphone and nothing in the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Situation {
    pub permission: Permission,
    /// The dimmed row the microphone is in (rows 7–9), if any — on
    /// activation nothing records and its sentence is said.
    pub blocked: Option<Dimmed>,
}

/// Where a recording that is waiting on the permission prompt came from.
/// Either way Allow starts recording: the activation meant "record".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The slot's microphone: a tap, a click, Enter or Space, a screen
    /// reader's activation. Its start is guarded.
    Tap,
    /// The paperclip's or the microphone menu's "Record Voice Message", or the
    /// shortcut: beside the draft if there is one, and never guarded.
    Menu,
}

/// Where the slot's voice recording is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Nothing recording.
    Idle,
    /// Recording, hands-free: the recording row.
    HandsFree {
        /// Started with words typed or items staged (row 3).
        beside_draft: bool,
    },
    /// Stopped by Delete at 10 s or more; "Delete this recording?" is asked.
    AskingDelete { recorded_ms: u64 },
    /// The system's microphone prompt is up.
    AwaitingPermission { source: Source, beside_draft: bool },
}

/// The reducer's whole state: the phase and the activation guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldState {
    pub phase: Phase,
    /// Activation of the slot before this moment is ignored whole. 0: none.
    pub guard_until_ms: u64,
}

impl Default for HoldState {
    fn default() -> Self {
        HoldState {
            phase: Phase::Idle,
            guard_until_ms: 0,
        }
    }
}

impl HoldState {
    /// What [`composer_slot`] is told about this recording.
    pub fn recording(&self) -> Recording {
        match self.phase {
            Phase::HandsFree {
                beside_draft: false,
            } => Recording::HandsFree,
            Phase::HandsFree { beside_draft: true } => Recording::HandsFreeBesideDraft,
            _ => Recording::None,
        }
    }

    /// The slot ignores activation at `at_ms` — including a row-5 Send, which
    /// a port asks here before sending: a double tap on the Stop square must
    /// not send what it staged.
    pub fn guarded(&self, at_ms: u64) -> bool {
        at_ms < self.guard_until_ms
    }
}

/// What happened. Every event carries `at_ms`, on the same monotonic clock;
/// a recording's own length is `recorded_ms`, by the recorder's clock — the
/// one the person sees, which with a screen reader starts only once
/// "Recording" has been spoken (S6).
///
/// Every activation of the slot — the microphone, the Send arrow, the Stop
/// square — is `Activate`, whatever activated it: the platform button's own
/// completed tap (a press that lifts inside, however long it was held), a
/// click, Enter or Space, a screen reader's. A port never sends anything for
/// a press going down or being held: a long press is not a gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldEvent {
    /// The recorder stopped itself at the five-minute limit.
    Cap { at_ms: u64 },
    /// Anything but the person stopped it (S4): a call in any phase, the app
    /// going to the BACKGROUND (never the inactive flicker an alert causes) or
    /// the screen locking, a desktop lock or sleep, a window hidden or closed,
    /// leaving the chat, another recording starting anywhere in the app, Siri
    /// or another app taking the microphone, the recorder failing.
    Interruption { at_ms: u64, recorded_ms: u64 },
    /// The slot was activated — see the note above.
    Activate {
        at_ms: u64,
        situation: Situation,
        recorded_ms: u64,
    },
    /// "Record Voice Message" from the paperclip or the microphone's menu, or
    /// the shortcut (⌥⌘R, Ctrl+Shift+R). `beside_draft`: words are typed or
    /// items staged. Pressed during a recording — only the shortcut can be —
    /// it STOPS it into review: a shortcut never sends.
    Record {
        at_ms: u64,
        beside_draft: bool,
        situation: Situation,
        recorded_ms: u64,
    },
    /// Stop, Esc, VoiceOver's escape, Android's Back, Magic Tap, "Stop and
    /// listen first". Never a start: Magic Tap with nothing recording does
    /// nothing here.
    Stop { at_ms: u64, recorded_ms: u64 },
    /// The recording row's Delete.
    Delete { at_ms: u64, recorded_ms: u64 },
    /// "Delete this recording?" answered: Delete, or Keep.
    Answer { at_ms: u64, delete: bool },
    /// The system's microphone prompt answered.
    PermissionAnswer { at_ms: u64, granted: bool },
    /// Any other action of the person's in the composer: a character typed
    /// or deleted, a paste, a suggestion taken, an item staged or taken off
    /// the strip, the paperclip, a sticker. While nothing records it lifts the
    /// activation guard: a change the person made is never guarded (S1.1), so
    /// "ok" sent and "x" typed and deleted at once leaves a microphone that
    /// records, and a photo pasted straight after a Send leaves a Send that
    /// sends. A port says it for every such change, never for its own: a Send
    /// emptying the box is `Emptied`, and the note a Stop in row 3 stages is
    /// the Stop's.
    OtherAction { at_ms: u64 },
    /// The slot's own Send or Save just emptied the composer: the microphone
    /// it turns into is guarded.
    Emptied { at_ms: u64 },
}

impl HoldEvent {
    pub fn at_ms(&self) -> u64 {
        match *self {
            HoldEvent::Cap { at_ms }
            | HoldEvent::Interruption { at_ms, .. }
            | HoldEvent::Activate { at_ms, .. }
            | HoldEvent::Record { at_ms, .. }
            | HoldEvent::Stop { at_ms, .. }
            | HoldEvent::Delete { at_ms, .. }
            | HoldEvent::Answer { at_ms, .. }
            | HoldEvent::PermissionAnswer { at_ms, .. }
            | HoldEvent::OtherAction { at_ms }
            | HoldEvent::Emptied { at_ms } => at_ms,
        }
    }
}

/// S2.9's haptics, phones only — a port drops them on an iPad, the Mac,
/// Windows and the web.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Haptic {
    /// Recording starts: iPhone `.impact(weight: .light)`, Android `ToggleOn`.
    Light,
    /// Sent: `.success`, `Confirm`.
    Success,
    /// Too short; deleted: `.warning`, `Reject`.
    Warning,
}

/// A line the composer SHOWS — in the row or its notice line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    StoppedAtFiveMinutes,
    TooShort,
}

impl Hint {
    pub fn text(self) -> &'static str {
        match self {
            Hint::StoppedAtFiveMinutes => "Recording stopped at five minutes.",
            Hint::TooShort => "That recording was too short.",
        }
    }
}

/// What is SPOKEN, politely, to a screen reader (S6) — state changes only,
/// never the ticking clock. "Not sent" is never said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Announcement {
    Recording,
    RecordingDeleted,
    VoiceMessageSent,
    /// "Ready to review, 0:42" — the length as m:ss.
    ReadyToReview {
        recorded_ms: u64,
    },
    TooShort,
    StoppedAtFiveMinutes,
}

impl Announcement {
    /// The catalogue key; `%@` is [`Announcement::ReadyToReview`]'s length.
    pub fn text(self) -> &'static str {
        match self {
            Announcement::Recording => "Recording",
            Announcement::RecordingDeleted => "Recording deleted",
            Announcement::VoiceMessageSent => "Voice message sent",
            Announcement::ReadyToReview { .. } => "Ready to review, %@",
            Announcement::TooShort => "That recording was too short.",
            Announcement::StoppedAtFiveMinutes => "Recording stopped at five minutes.",
        }
    }
}

/// What a port does, in the order given: the thing first, then what is shown,
/// said and felt. A port that must speak before it records — a screen reader
/// running (S2.2, S6) — holds `Start`'s microphone until its "Recording" has
/// been spoken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldEffect {
    /// Open the microphone and record: the recording row. Keep the screen
    /// awake, pause anything playing, move focus to the slot.
    Start,
    /// Stop the recording if it runs, and delete it.
    Delete,
    /// Stop it, and hand it to the outbox now, with the primed reply.
    Send,
    /// Stop it, and stage it in the chip above the field (S2.7).
    Review,
    /// Stop it, and keep it as the chat's "Voice message not sent" row (S2.8).
    Park,
    /// Stop it, and ask "Delete this recording?" [Delete] [Keep].
    AskDelete,
    /// Raise the system's microphone prompt.
    AskPermission,
    /// The denial notice, with Open Settings where the platform can open it.
    Denied,
    /// Say the dimmed row's sentence instead of acting.
    Explain(Dimmed),
    Hint(Hint),
    Announce(Announcement),
    Haptic(Haptic),
}

/// One step of the slot's voice recording (S2.1, S2.2, S2.5): the state and
/// an event in, the next state and what to do out.
///
/// The rules, in the order they are asked:
///
/// - **The activation guard** swallows an `Activate` while it runs — whole.
///   Nothing else is guarded, and the person's own change to the composer
///   (`OtherAction`) lifts it while nothing records.
/// - **The microphone's activation** — `Activate` from idle, `Record` — says a
///   dimmed row's sentence, raises the prompt, gives the denial notice, or
///   starts a hands-free recording. Allow, after a prompt, starts it.
/// - **Ending a recording**: the slot's Send arrow sends (under a second it is
///   too short); the slot's Stop square (row 3), Stop and the shortcut review
///   (too short likewise); Delete deletes under ten seconds and asks from ten;
///   the five-minute limit reviews; an interruption parks it as not sent
///   (under a second it is deleted) — an interruption never sends.
pub fn hold_step(
    state: HoldState,
    event: HoldEvent,
    constants: &HoldConstants,
) -> (HoldState, Vec<HoldEffect>) {
    let mut s = state;
    let mut fx = Vec::new();
    let at = event.at_ms();
    let c = constants;

    match event {
        HoldEvent::Cap { .. } => {
            if recording(&s) {
                s.phase = Phase::Idle;
                fx.push(HoldEffect::Review);
                fx.push(HoldEffect::Hint(Hint::StoppedAtFiveMinutes));
                fx.push(HoldEffect::Announce(Announcement::StoppedAtFiveMinutes));
            }
        }
        HoldEvent::Interruption { recorded_ms, .. } => match s.phase {
            Phase::HandsFree { .. } => {
                s.phase = Phase::Idle;
                interrupted(&mut fx, recorded_ms, c);
            }
            // The question's answer never came: the recording is kept, never
            // lost and never sent, and the question goes with it.
            Phase::AskingDelete { .. } => {
                s.phase = Phase::Idle;
                fx.push(HoldEffect::Park);
            }
            Phase::AwaitingPermission { .. } => s.phase = Phase::Idle,
            Phase::Idle => {}
        },
        HoldEvent::Activate {
            situation,
            recorded_ms,
            ..
        } => {
            if !s.guarded(at) {
                match s.phase {
                    Phase::Idle => {
                        activate_microphone(&mut s, &mut fx, at, &situation, Source::Tap, false, c)
                    }
                    Phase::HandsFree { beside_draft } => {
                        s.phase = Phase::Idle;
                        s.guard_until_ms = at.saturating_add(c.activation_guard_ms);
                        if beside_draft {
                            stop_into_review(&mut fx, recorded_ms, c);
                        } else {
                            send_now(&mut fx, recorded_ms, c);
                        }
                    }
                    Phase::AskingDelete { .. } | Phase::AwaitingPermission { .. } => {}
                }
            }
        }
        HoldEvent::Record {
            beside_draft,
            situation,
            recorded_ms,
            ..
        } => match s.phase {
            Phase::Idle => activate_microphone(
                &mut s,
                &mut fx,
                at,
                &situation,
                Source::Menu,
                beside_draft,
                c,
            ),
            Phase::HandsFree { .. } => {
                s.phase = Phase::Idle;
                stop_into_review(&mut fx, recorded_ms, c);
            }
            Phase::AskingDelete { .. } | Phase::AwaitingPermission { .. } => {}
        },
        HoldEvent::Stop { recorded_ms, .. } => {
            if recording(&s) {
                s.phase = Phase::Idle;
                stop_into_review(&mut fx, recorded_ms, c);
            }
        }
        HoldEvent::Delete { recorded_ms, .. } => {
            if recording(&s) {
                if recorded_ms < c.delete_asks_from_ms {
                    s.phase = Phase::Idle;
                    deleted(&mut fx);
                } else {
                    s.phase = Phase::AskingDelete { recorded_ms };
                    fx.push(HoldEffect::AskDelete);
                }
            }
        }
        HoldEvent::Answer { delete, .. } => {
            if let Phase::AskingDelete { recorded_ms } = s.phase {
                s.phase = Phase::Idle;
                if delete {
                    deleted(&mut fx);
                } else {
                    fx.push(HoldEffect::Review);
                    fx.push(HoldEffect::Announce(Announcement::ReadyToReview {
                        recorded_ms,
                    }));
                }
            }
        }
        HoldEvent::PermissionAnswer { granted, .. } => {
            if let Phase::AwaitingPermission {
                source,
                beside_draft,
            } = s.phase
            {
                s.phase = Phase::Idle;
                if granted {
                    start_hands_free(&mut s, &mut fx, at, source, beside_draft, c);
                } else {
                    fx.push(HoldEffect::Denied);
                }
            }
        }
        HoldEvent::OtherAction { .. } => {
            // The person changed the composer: the next activation is a
            // decision of its own, not the second half of a double tap. While
            // a recording runs the box is behind the row and nothing in it is
            // the person's to change, so the guard that keeps a double tap on
            // the microphone from sending stays.
            if s.phase == Phase::Idle {
                s.guard_until_ms = 0;
            }
        }
        HoldEvent::Emptied { .. } => {
            s.guard_until_ms = at.saturating_add(c.activation_guard_ms);
        }
    }
    (s, fx)
}

/// A voice recording runs.
fn recording(s: &HoldState) -> bool {
    matches!(s.phase, Phase::HandsFree { .. })
}

/// What stands between the microphone's activation and a recording, in this
/// order: a dimmed row's sentence, the denial notice, or the prompt. None:
/// start.
fn refusal(situation: &Situation) -> Option<HoldEffect> {
    if let Some(reason) = situation.blocked {
        return Some(HoldEffect::Explain(reason));
    }
    match situation.permission {
        Permission::Granted => None,
        Permission::Denied => Some(HoldEffect::Denied),
        Permission::NotAsked => Some(HoldEffect::AskPermission),
    }
}

/// The microphone's completed activation, from the slot or a menu.
fn activate_microphone(
    s: &mut HoldState,
    fx: &mut Vec<HoldEffect>,
    at: u64,
    situation: &Situation,
    source: Source,
    beside_draft: bool,
    c: &HoldConstants,
) {
    match refusal(situation) {
        Some(HoldEffect::AskPermission) => {
            s.phase = Phase::AwaitingPermission {
                source,
                beside_draft,
            };
            fx.push(HoldEffect::AskPermission);
        }
        Some(effect) => {
            s.phase = Phase::Idle;
            fx.push(effect);
        }
        None => start_hands_free(s, fx, at, source, beside_draft, c),
    }
}

/// A hands-free recording starts. Only the slot's OWN activation guards it.
fn start_hands_free(
    s: &mut HoldState,
    fx: &mut Vec<HoldEffect>,
    at: u64,
    source: Source,
    beside_draft: bool,
    c: &HoldConstants,
) {
    s.phase = Phase::HandsFree { beside_draft };
    if source == Source::Tap {
        s.guard_until_ms = at.saturating_add(c.activation_guard_ms);
    }
    fx.push(HoldEffect::Start);
    fx.push(HoldEffect::Announce(Announcement::Recording));
    fx.push(HoldEffect::Haptic(Haptic::Light));
}

/// The person deleted it: said and felt.
fn deleted(fx: &mut Vec<HoldEffect>) {
    fx.push(HoldEffect::Delete);
    fx.push(HoldEffect::Announce(Announcement::RecordingDeleted));
    fx.push(HoldEffect::Haptic(Haptic::Warning));
}

/// Under a second: discarded, with the sentence shown and said.
fn too_short(fx: &mut Vec<HoldEffect>) {
    fx.push(HoldEffect::Delete);
    fx.push(HoldEffect::Hint(Hint::TooShort));
    fx.push(HoldEffect::Announce(Announcement::TooShort));
    fx.push(HoldEffect::Haptic(Haptic::Warning));
}

/// The Send arrow (S2.5).
fn send_now(fx: &mut Vec<HoldEffect>, recorded_ms: u64, c: &HoldConstants) {
    if recorded_ms < c.shortest_recording_ms {
        too_short(fx);
    } else {
        fx.push(HoldEffect::Send);
        fx.push(HoldEffect::Announce(Announcement::VoiceMessageSent));
        fx.push(HoldEffect::Haptic(Haptic::Success));
    }
}

/// Stop, the Stop square, the shortcut (S2.5).
fn stop_into_review(fx: &mut Vec<HoldEffect>, recorded_ms: u64, c: &HoldConstants) {
    if recorded_ms < c.shortest_recording_ms {
        too_short(fx);
    } else {
        fx.push(HoldEffect::Review);
        fx.push(HoldEffect::Announce(Announcement::ReadyToReview {
            recorded_ms,
        }));
    }
}

/// Stopped by something other than the person (S2.8, S4): kept as not sent,
/// or, with nothing worth keeping, deleted without a word.
fn interrupted(fx: &mut Vec<HoldEffect>, recorded_ms: u64, c: &HoldConstants) {
    if recorded_ms < c.shortest_recording_ms {
        fx.push(HoldEffect::Delete);
    } else {
        fx.push(HoldEffect::Park);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Announcement as A;
    use HoldEffect as E;

    // --- the composer, as a row-10 microphone sees it --------------------------------------------

    /// An empty composer in a family chat that can record: row 10.
    fn inputs() -> SlotInputs {
        SlotInputs {
            recorder_open: false,
            recording: Recording::None,
            editing: false,
            draft_blank: true,
            staged: false,
            assistant_chat: false,
            can_record: true,
            call: false,
            busy: false,
            not_sent: false,
        }
    }

    /// The composer with row `row`'s condition made true and nothing else
    /// changed. Rows 2 and 3 share the recording field.
    fn with_row(mut i: SlotInputs, row: u8) -> SlotInputs {
        match row {
            1 => i.recorder_open = true,
            2 => i.recording = Recording::HandsFree,
            3 => {
                i.recording = Recording::HandsFreeBesideDraft;
                i.draft_blank = false;
            }
            4 => i.editing = true,
            5 => i.draft_blank = false,
            6 => i.assistant_chat = true,
            7 => i.call = true,
            8 => i.busy = true,
            9 => i.not_sent = true,
            _ => {}
        }
        i
    }

    #[test]
    fn the_constants_are_the_plans() {
        assert_eq!(ACTIVATION_GUARD_MS, 600);
        assert_eq!(SHORTEST_RECORDING_MS, 1_000);
        assert_eq!((VOICE_CAP_MS, VOICE_WARNING_MS), (300_000, 270_000));
        assert_eq!(DEFAULT_MAX_ROUND_VIDEO_MS, 60_000);
        assert_eq!((ROUND_CAP_MARGIN_MS, ROUND_WARNING_LEAD_MS), (500, 10_000));
        assert_eq!(SILENCE_PEAK_DBFS, -60.0);
        assert_eq!(SILENCE_MAX_AMPLITUDE, 32);
        assert_eq!(SILENCE_SAMPLE_MAGNITUDE, 0.001);
        assert_eq!(SILENCE_WARNING_AFTER_MS, 3_000);
        assert_eq!(DELETE_ASKS_FROM_MS, 10_000);
        assert_eq!(PREVIEW_IDLE_CLOSE_MS, 60_000);
        assert_eq!((SLOT_CROSSFADE_MS, RECORDER_FADE_MS), (150, 200));
        assert_eq!(
            (
                MIN_TARGET_APPLE_PT,
                MIN_TARGET_ANDROID_DP,
                MIN_TARGET_WINDOWS_EPX,
                MIN_TARGET_WEB_PX
            ),
            (44, 48, 44, 44)
        );
        // Android's 32 of 32 767 and the web's 0.001 are the same −60 dBFS.
        let android = 20.0 * (f64::from(SILENCE_MAX_AMPLITUDE) / 32_767.0).log10();
        let web = 20.0 * SILENCE_SAMPLE_MAGNITUDE.log10();
        assert!((android - SILENCE_PEAK_DBFS).abs() < 0.25, "{android}");
        assert!((web - SILENCE_PEAK_DBFS).abs() < 1e-9, "{web}");
        assert_eq!(
            HoldConstants::default(),
            HoldConstants {
                shortest_recording_ms: 1_000,
                activation_guard_ms: 600,
                delete_asks_from_ms: 10_000,
            }
        );
    }

    // --- S1.3 --------------------------------------------------------------------------------------

    #[test]
    fn every_row_on_its_own() {
        let expected = [
            (1, Slot::Recorder, None, None),
            (2, Slot::SendVoice, Some("Send voice message"), None),
            (3, Slot::StopRecording, Some("Stop recording"), None),
            (4, Slot::Save { enabled: false }, Some("Save"), None),
            (5, Slot::Send, Some("Send"), None),
            (6, Slot::SendDisabled, Some("Send"), None),
            (
                7,
                Slot::Dimmed(Dimmed::Call),
                Some("Record voice message"),
                Some("You can record a message after the call."),
            ),
            (
                8,
                Slot::Dimmed(Dimmed::Busy),
                Some("Record voice message"),
                Some("Wait until the current attachment is done."),
            ),
            (
                9,
                Slot::Dimmed(Dimmed::NotSent),
                Some("Record voice message"),
                Some("Send or delete the voice message that wasn't sent first."),
            ),
            (10, Slot::Microphone, Some("Record voice message"), None),
        ];
        for (row, slot, label, notice) in expected {
            let got = composer_slot(&with_row(inputs(), row));
            assert_eq!(got, slot, "row {row}");
            assert_eq!(got.row(), row);
            assert_eq!(got.label(), label, "row {row}");
            assert_eq!(got.notice(), notice, "row {row}");
            assert_eq!(got.is_microphone(), row >= 7, "row {row}");
        }
    }

    #[test]
    fn the_first_matching_row_wins_for_every_pair() {
        for higher in 1..=9u8 {
            for lower in higher + 1..=9u8 {
                if (higher, lower) == (2, 3) {
                    continue; // one field: a recording is one or the other
                }
                let both = with_row(with_row(inputs(), lower), higher);
                assert_eq!(
                    composer_slot(&both).row(),
                    higher,
                    "rows {higher} and {lower}"
                );
            }
        }
        let mut everything = inputs();
        for row in [1, 3, 4, 5, 6, 7, 8, 9] {
            everything = with_row(everything, row);
        }
        assert_eq!(composer_slot(&everything), Slot::Recorder);
    }

    #[test]
    fn empty_is_a_blank_draft_and_nothing_staged() {
        let staged = SlotInputs {
            staged: true,
            ..inputs()
        };
        assert_eq!(composer_slot(&staged), Slot::Send);
        assert!(!staged.empty());
        let typed = SlotInputs {
            draft_blank: false,
            ..inputs()
        };
        assert_eq!(composer_slot(&typed), Slot::Send);
        assert!(inputs().empty());
        // Text typed in the assistant's chat, or where nothing records, still sends.
        for blocked in [
            SlotInputs {
                assistant_chat: true,
                ..typed
            },
            SlotInputs {
                can_record: false,
                ..typed
            },
            SlotInputs {
                call: true,
                busy: true,
                not_sent: true,
                ..staged
            },
        ] {
            assert_eq!(composer_slot(&blocked), Slot::Send);
        }
    }

    #[test]
    fn save_is_disabled_while_the_field_is_blank_and_is_never_a_microphone() {
        let editing = with_row(inputs(), 4);
        assert_eq!(composer_slot(&editing), Slot::Save { enabled: false });
        let typed = SlotInputs {
            draft_blank: false,
            ..editing
        };
        assert_eq!(composer_slot(&typed), Slot::Save { enabled: true });
        let cleared_in_a_call = SlotInputs {
            call: true,
            not_sent: true,
            ..editing
        };
        assert_eq!(
            composer_slot(&cleared_in_a_call),
            Slot::Save { enabled: false }
        );
    }

    #[test]
    fn the_assistant_chat_and_a_device_that_cannot_record_keep_todays_disabled_send() {
        for i in [
            with_row(inputs(), 6),
            SlotInputs {
                can_record: false,
                ..inputs()
            },
            SlotInputs {
                can_record: false,
                call: true,
                ..inputs()
            },
        ] {
            assert_eq!(composer_slot(&i), Slot::SendDisabled);
            assert!(!composer_slot(&i).is_microphone());
        }
    }

    #[test]
    fn a_recording_beside_a_draft_is_stop_never_send() {
        // Row 3's words are behind the row: its slot is Stop, never Send —
        // with words, with items staged, and with every lower row true.
        for i in [
            with_row(inputs(), 3),
            SlotInputs {
                recording: Recording::HandsFreeBesideDraft,
                staged: true,
                ..inputs()
            },
            SlotInputs {
                recording: Recording::HandsFreeBesideDraft,
                editing: true,
                draft_blank: false,
                call: true,
                busy: true,
                not_sent: true,
                ..inputs()
            },
        ] {
            assert_eq!(composer_slot(&i), Slot::StopRecording);
        }
    }

    // --- S1.4 --------------------------------------------------------------------------------------

    fn door(slot: SlotInputs) -> DoorInputs {
        DoorInputs {
            slot,
            family_or_direct_chat: true,
            server_offers_round: true,
            has_camera: true,
            encoder_probe_passes: true,
            records_round_video: true,
        }
    }

    #[test]
    fn the_door_beside_each_row() {
        for row in 1..=10u8 {
            let expected = match row {
                7 => Door::Dimmed(Dimmed::Call),
                8 => Door::Dimmed(Dimmed::Busy),
                9 | 10 => Door::Shown,
                _ => Door::Hidden,
            };
            let got = video_door(&door(with_row(inputs(), row)));
            assert_eq!(got, expected, "row {row}");
        }
        assert_eq!(Door::Shown.label(), Some("Record video message"));
        assert_eq!(Door::Hidden.label(), None);
        assert_eq!(
            Door::Dimmed(Dimmed::Busy).notice(),
            Some("Wait until the current attachment is done.")
        );
        assert_eq!(Door::Shown.notice(), None);
        assert_eq!(VIDEO_DOOR_TOOLTIP, "Record a video message");
    }

    #[test]
    fn the_door_needs_every_part_of_round_available() {
        let open = door(inputs());
        assert!(open.round_available());
        for shut in [
            DoorInputs {
                server_offers_round: false,
                ..open
            },
            DoorInputs {
                has_camera: false,
                ..open
            },
            DoorInputs {
                encoder_probe_passes: false,
                ..open
            },
            DoorInputs {
                records_round_video: false,
                ..open
            },
        ] {
            assert!(!shut.round_available());
            assert_eq!(video_door(&shut), Door::Hidden);
            // Not even dimmed in a call: a door that cannot open is not drawn.
            let in_a_call = DoorInputs {
                slot: with_row(inputs(), 7),
                ..shut
            };
            assert_eq!(video_door(&in_a_call), Door::Hidden);
        }
    }

    #[test]
    fn the_door_is_hidden_outside_a_family_or_direct_chat_and_while_staged_or_recording() {
        let elsewhere = DoorInputs {
            family_or_direct_chat: false,
            ..door(inputs())
        };
        assert_eq!(video_door(&elsewhere), Door::Hidden);
        let staged = door(SlotInputs {
            staged: true,
            ..inputs()
        });
        assert_eq!(video_door(&staged), Door::Hidden);
        for recording in [Recording::HandsFree, Recording::HandsFreeBesideDraft] {
            let recording = door(SlotInputs {
                recording,
                ..inputs()
            });
            assert_eq!(video_door(&recording), Door::Hidden);
        }
    }

    // --- the round video ---------------------------------------------------------------------------

    #[test]
    fn the_round_cap_and_warning_and_their_clamps() {
        assert_eq!(round_cap_ms(DEFAULT_MAX_ROUND_VIDEO_MS), 59_500);
        assert_eq!(round_warning_ms(DEFAULT_MAX_ROUND_VIDEO_MS), 50_000);
        for (max, cap, warning) in [
            (120_000, 119_500, 110_000),
            (10_500, 10_000, 500),
            (10_000, 9_500, 0),
            (9_999, 9_499, 0),
            (501, 1, 0),
            (500, 0, 0),
            (499, 0, 0),
            (0, 0, 0),
        ] {
            assert_eq!(
                (round_cap_ms(max), round_warning_ms(max)),
                (cap, warning),
                "{max}"
            );
            assert!(round_warning_ms(max) <= round_cap_ms(max));
        }
    }

    #[test]
    fn round_diameters() {
        assert_eq!(round_diameter(WidthClass::Compact), 200);
        assert_eq!(round_diameter(WidthClass::Regular), 240);
    }

    #[test]
    fn a_round_message_is_one_video_with_the_flag_and_no_body() {
        let video = |round| AttachmentFlags {
            kind: "video",
            round,
        };
        assert!(is_round("", &[video(true)]));
        assert!(!is_round("", &[video(false)]));
        assert!(!is_round("", &[video(true), video(true)]));
        assert!(!is_round("", &[]));
        assert!(!is_round("hello", &[video(true)]));
        // Exactly empty: the server stores a blank attachment body as "".
        assert!(!is_round(" ", &[video(true)]));
        for kind in ["photo", "audio", "file", "location", "Video", ""] {
            assert!(
                !is_round("", &[AttachmentFlags { kind, round: true }]),
                "{kind}"
            );
        }
    }

    // --- S2.1, S2.2 and S2.5 -----------------------------------------------------------------------

    fn c() -> HoldConstants {
        HoldConstants::default()
    }

    fn step(s: HoldState, e: HoldEvent) -> (HoldState, Vec<HoldEffect>) {
        hold_step(s, e, &c())
    }

    /// Runs the events from `s` and answers the last state and every step's effects.
    fn run(mut s: HoldState, events: &[HoldEvent]) -> (HoldState, Vec<Vec<HoldEffect>>) {
        let mut all = Vec::new();
        for e in events {
            let (next, fx) = step(s, *e);
            s = next;
            all.push(fx);
        }
        (s, all)
    }

    fn sit() -> Situation {
        Situation::default()
    }

    fn activate(at: u64, recorded_ms: u64) -> HoldEvent {
        HoldEvent::Activate {
            at_ms: at,
            situation: sit(),
            recorded_ms,
        }
    }

    fn activate_with(at: u64, situation: Situation) -> HoldEvent {
        HoldEvent::Activate {
            at_ms: at,
            situation,
            recorded_ms: 0,
        }
    }

    fn idle() -> HoldState {
        HoldState::default()
    }

    fn hands_free(beside_draft: bool) -> HoldState {
        HoldState {
            phase: Phase::HandsFree { beside_draft },
            guard_until_ms: 0,
        }
    }

    fn started() -> Vec<HoldEffect> {
        vec![
            E::Start,
            E::Announce(A::Recording),
            E::Haptic(Haptic::Light),
        ]
    }

    fn sent() -> Vec<HoldEffect> {
        vec![
            E::Send,
            E::Announce(A::VoiceMessageSent),
            E::Haptic(Haptic::Success),
        ]
    }

    fn too_short_fx() -> Vec<HoldEffect> {
        vec![
            E::Delete,
            E::Hint(Hint::TooShort),
            E::Announce(A::TooShort),
            E::Haptic(Haptic::Warning),
        ]
    }

    fn deleted_fx() -> Vec<HoldEffect> {
        vec![
            E::Delete,
            E::Announce(A::RecordingDeleted),
            E::Haptic(Haptic::Warning),
        ]
    }

    fn reviewed(recorded_ms: u64) -> Vec<HoldEffect> {
        vec![E::Review, E::Announce(A::ReadyToReview { recorded_ms })]
    }

    #[test]
    fn a_tap_records_hands_free_and_the_same_slot_sends() {
        let (s, fx) = step(idle(), activate(120, 0));
        assert_eq!(fx, started());
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        assert_eq!(s.guard_until_ms, 720);
        assert_eq!(s.recording(), Recording::HandsFree);
        assert_eq!(
            composer_slot(&SlotInputs {
                recording: s.recording(),
                ..inputs()
            }),
            Slot::SendVoice
        );
        let (s, fx) = step(s, activate(5_000, 4_880));
        assert_eq!(fx, sent());
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(
            s.guard_until_ms, 5_600,
            "a Send that ends a recording guards the microphone"
        );
    }

    #[test]
    fn a_double_tap_on_the_microphone_cannot_send_and_one_on_send_cannot_record() {
        let (s, _) = step(idle(), activate(100, 0));
        let (s, fx) = step(s, activate(699, 590));
        assert!(fx.is_empty());
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        assert!(s.guarded(699) && !s.guarded(700));
        let (s, fx) = step(s, activate(700, 600));
        assert_eq!(fx, too_short_fx());
        assert_eq!(s.guard_until_ms, 1_300);
        // The microphone the Send leaves behind is guarded too.
        let (s, fx) = step(s, activate(1_299, 0));
        assert!(fx.is_empty());
        assert_eq!(s.phase, Phase::Idle);

        // A text Send empties the composer: the microphone it becomes is guarded.
        let (s, fx) = step(idle(), HoldEvent::Emptied { at_ms: 1_000 });
        assert!(fx.is_empty());
        let (s, fx) = step(s, activate(1_599, 0));
        assert!(fx.is_empty());
        assert_eq!(s.phase, Phase::Idle);
        let (_, fx) = step(s, activate(1_600, 0));
        assert_eq!(fx, started());
        let (_, fx) = step(idle(), activate(1_599, 0));
        assert_eq!(fx, started(), "nothing guards a composer nobody emptied");
    }

    #[test]
    fn send_at_exactly_a_second_sends_and_a_millisecond_less_is_too_short() {
        let (s, fx) = step(hands_free(false), activate(3_000, 1_000));
        assert_eq!(fx, sent());
        assert_eq!(s.phase, Phase::Idle);
        let (s, fx) = step(hands_free(false), activate(3_000, 999));
        assert_eq!(fx, too_short_fx());
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(
            s.guard_until_ms, 3_600,
            "a Send that finds it too short guards the slot too"
        );
    }

    /// S1.1: "A change caused by typing, pasting, a suggestion, deleting text
    /// or staging is NEVER guarded". The guard is there for the second half
    /// of a double tap; the person's own change to the composer between two
    /// activations means the next one is a new decision.
    #[test]
    fn the_persons_own_change_to_the_composer_lifts_the_guard() {
        // "ok" sent, then "x" typed and deleted inside the guard: the
        // microphone the deletion brought back records.
        let (s, _) = step(idle(), HoldEvent::Emptied { at_ms: 1_000 });
        assert!(s.guarded(1_300));
        let (s, fx) = step(s, HoldEvent::OtherAction { at_ms: 1_100 });
        assert!(fx.is_empty(), "nothing to say: {fx:?}");
        assert!(!s.guarded(1_300), "typed: not guarded");
        assert_eq!(s.guard_until_ms, 0);
        let (_, fx) = step(s, activate(1_300, 0));
        assert_eq!(fx, started());

        // The Stop square's own staging guards a second activation; the
        // person typing after it does not leave the row-5 Send they then
        // press guarded (a port asks `guarded` before such a Send).
        let record = HoldEvent::Record {
            at_ms: 1_000,
            beside_draft: true,
            situation: sit(),
            recorded_ms: 0,
        };
        let (s, _) = run(idle(), &[record, activate(4_000, 3_000)]);
        assert!(s.guarded(4_200), "Stop staged the note: guarded");
        let (s, _) = step(s, HoldEvent::OtherAction { at_ms: 4_100 });
        assert!(!s.guarded(4_200));

        // While a recording runs the box is behind the row and cannot be the
        // person's to change: nothing it says lifts the guard that keeps a
        // double tap on the microphone from sending.
        let (s, _) = step(idle(), activate(100, 0));
        let (s, fx) = step(s, HoldEvent::OtherAction { at_ms: 300 });
        assert!(fx.is_empty());
        assert!(s.guarded(400));
        let (s, fx) = step(s, activate(400, 300));
        assert!(fx.is_empty(), "still guarded: {fx:?}");
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
    }

    #[test]
    fn an_interruption_parks_a_recording_deletes_one_under_a_second_and_never_sends() {
        let interruption = |recorded_ms| HoldEvent::Interruption {
            at_ms: 3_000,
            recorded_ms,
        };
        for recording in [hands_free(false), hands_free(true)] {
            let (s, fx) = step(recording, interruption(1_000));
            assert_eq!(fx, vec![E::Park], "{recording:?}");
            assert_eq!(s.phase, Phase::Idle);
            let (_, fx) = step(recording, interruption(999));
            assert_eq!(fx, vec![E::Delete]);
            let (_, fx) = step(recording, interruption(250_000));
            assert!(!fx.contains(&E::Send), "an interruption never sends");
        }
        let asking = HoldState {
            phase: Phase::AskingDelete {
                recorded_ms: 12_000,
            },
            ..idle()
        };
        let (s, fx) = step(asking, interruption(12_000));
        assert_eq!(fx, vec![E::Park]);
        assert_eq!(s.phase, Phase::Idle);
        let (s, fx) = step(idle(), interruption(0));
        assert!(fx.is_empty());
        assert_eq!(s, idle());
        let prompting = HoldState {
            phase: Phase::AwaitingPermission {
                source: Source::Tap,
                beside_draft: false,
            },
            ..idle()
        };
        let (s, fx) = step(prompting, interruption(0));
        assert!(fx.is_empty());
        let (_, fx) = step(
            s,
            HoldEvent::PermissionAnswer {
                at_ms: 4_000,
                granted: true,
            },
        );
        assert!(fx.is_empty(), "nothing starts after an interruption");
    }

    #[test]
    fn five_minutes_reviews_and_never_sends() {
        for recording in [hands_free(false), hands_free(true)] {
            let (s, fx) = step(recording, HoldEvent::Cap { at_ms: 301_000 });
            assert_eq!(
                fx,
                vec![
                    E::Review,
                    E::Hint(Hint::StoppedAtFiveMinutes),
                    E::Announce(A::StoppedAtFiveMinutes)
                ]
            );
            assert_eq!(s.phase, Phase::Idle);
            assert_eq!(s.guard_until_ms, recording.guard_until_ms);
        }
        let (_, fx) = step(idle(), HoldEvent::Cap { at_ms: 1 });
        assert!(fx.is_empty());
    }

    #[test]
    fn stop_reviews_and_under_a_second_is_too_short() {
        for recording in [hands_free(false), hands_free(true)] {
            let (s, fx) = step(
                recording,
                HoldEvent::Stop {
                    at_ms: 9_000,
                    recorded_ms: 1_000,
                },
            );
            assert_eq!(fx, reviewed(1_000));
            assert_eq!(s.phase, Phase::Idle);
            assert_eq!(
                s.guard_until_ms, recording.guard_until_ms,
                "Stop is not the slot"
            );
            let (_, fx) = step(
                recording,
                HoldEvent::Stop {
                    at_ms: 9_000,
                    recorded_ms: 999,
                },
            );
            assert_eq!(fx, too_short_fx());
        }
        // Magic Tap never starts a recording.
        let (s, fx) = step(
            idle(),
            HoldEvent::Stop {
                at_ms: 9_000,
                recorded_ms: 0,
            },
        );
        assert!(fx.is_empty());
        assert_eq!(s, idle());
    }

    #[test]
    fn delete_under_ten_seconds_deletes_and_from_ten_stops_and_asks() {
        let delete = |recorded_ms| HoldEvent::Delete {
            at_ms: 20_000,
            recorded_ms,
        };
        let (s, fx) = step(hands_free(false), delete(9_999));
        assert_eq!(fx, deleted_fx());
        assert_eq!(s.phase, Phase::Idle);
        let (s, fx) = step(hands_free(false), delete(10_000));
        assert_eq!(fx, vec![E::AskDelete]);
        assert_eq!(
            s.phase,
            Phase::AskingDelete {
                recorded_ms: 10_000
            }
        );
        assert_eq!(s.recording(), Recording::None);
        let (s2, fx) = step(
            s,
            HoldEvent::Answer {
                at_ms: 21_000,
                delete: true,
            },
        );
        assert_eq!(fx, deleted_fx());
        assert_eq!(s2.phase, Phase::Idle);
        let (s3, fx) = step(
            s,
            HoldEvent::Answer {
                at_ms: 21_000,
                delete: false,
            },
        );
        assert_eq!(fx, reviewed(10_000));
        assert_eq!(s3.phase, Phase::Idle);
        let (_, fx) = step(hands_free(true), delete(400));
        assert_eq!(fx, deleted_fx());
    }

    #[test]
    fn a_recording_beside_a_draft_stops_into_review_from_the_slot() {
        let record = HoldEvent::Record {
            at_ms: 1_000,
            beside_draft: true,
            situation: sit(),
            recorded_ms: 0,
        };
        let (s, fx) = step(idle(), record);
        assert_eq!(fx, started());
        assert_eq!(s.phase, Phase::HandsFree { beside_draft: true });
        assert_eq!(s.recording(), Recording::HandsFreeBesideDraft);
        assert_eq!(s.guard_until_ms, 0, "the paperclip is not the slot");
        let (s, fx) = step(s, activate(1_100, 1_000));
        assert_eq!(
            fx,
            reviewed(1_000),
            "the slot is Stop: the words must not leave unseen"
        );
        assert_eq!(
            s.guard_until_ms, 1_700,
            "a Stop in row 3 that stages the note guards the slot"
        );
        let (_, fx) = step(hands_free(true), activate(1_100, 999));
        assert_eq!(fx, too_short_fx());
    }

    #[test]
    fn the_shortcut_starts_and_pressed_again_stops_into_review_never_sending() {
        let shortcut = |at_ms, recorded_ms| HoldEvent::Record {
            at_ms,
            beside_draft: false,
            situation: sit(),
            recorded_ms,
        };
        let (s, fx) = step(idle(), shortcut(0, 0));
        assert_eq!(fx, started());
        assert_eq!(s.guard_until_ms, 0, "the shortcut is not the slot");
        let (s, fx) = step(s, shortcut(100, 100));
        assert_eq!(fx, too_short_fx());
        assert_eq!(s.phase, Phase::Idle);
        let (_, fx) = step(hands_free(false), shortcut(9_000, 8_000));
        assert_eq!(fx, reviewed(8_000));
    }

    #[test]
    fn a_tap_without_permission_asks_and_allow_records() {
        let not_asked = Situation {
            permission: Permission::NotAsked,
            ..sit()
        };
        let (s, fx) = step(idle(), activate_with(100, not_asked));
        assert_eq!(fx, vec![E::AskPermission]);
        assert_eq!(
            s.phase,
            Phase::AwaitingPermission {
                source: Source::Tap,
                beside_draft: false
            }
        );
        let (s2, fx) = step(
            s,
            HoldEvent::PermissionAnswer {
                at_ms: 4_000,
                granted: true,
            },
        );
        assert_eq!(fx, started(), "the tap meant record");
        assert_eq!(
            s2.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        assert_eq!(s2.guard_until_ms, 4_600);
        let (s3, fx) = step(
            s,
            HoldEvent::PermissionAnswer {
                at_ms: 4_000,
                granted: false,
            },
        );
        assert_eq!(fx, vec![E::Denied]);
        assert_eq!(s3.phase, Phase::Idle);
        // From the paperclip, beside a draft, and never guarded.
        let (s, _) = step(
            idle(),
            HoldEvent::Record {
                at_ms: 0,
                beside_draft: true,
                situation: not_asked,
                recorded_ms: 0,
            },
        );
        let (s, fx) = step(
            s,
            HoldEvent::PermissionAnswer {
                at_ms: 4_000,
                granted: true,
            },
        );
        assert_eq!(fx, started());
        assert_eq!(s.phase, Phase::HandsFree { beside_draft: true });
        assert_eq!(s.guard_until_ms, 0);
    }

    #[test]
    fn a_denied_microphone_gives_the_notice() {
        let denied = Situation {
            permission: Permission::Denied,
            ..sit()
        };
        let (s, fx) = step(idle(), activate_with(0, denied));
        assert_eq!(fx, vec![E::Denied]);
        assert_eq!(s, idle(), "and nothing is guarded");
        let (s, fx) = step(
            idle(),
            HoldEvent::Record {
                at_ms: 0,
                beside_draft: false,
                situation: denied,
                recorded_ms: 0,
            },
        );
        assert_eq!(fx, vec![E::Denied]);
        assert_eq!(s, idle());
    }

    #[test]
    fn a_dimmed_microphone_explains_from_a_tap_and_from_the_menu() {
        for reason in [Dimmed::Call, Dimmed::Busy, Dimmed::NotSent] {
            let blocked = Situation {
                blocked: Some(reason),
                // A dimmed row's sentence comes before any prompt.
                permission: Permission::NotAsked,
            };
            let (s, fx) = step(idle(), activate_with(100, blocked));
            assert_eq!(fx, vec![E::Explain(reason)]);
            assert_eq!(s, idle());
            let (s, fx) = step(
                idle(),
                HoldEvent::Record {
                    at_ms: 0,
                    beside_draft: false,
                    situation: blocked,
                    recorded_ms: 0,
                },
            );
            assert_eq!(fx, vec![E::Explain(reason)]);
            assert_eq!(s, idle());
        }
    }

    #[test]
    fn events_out_of_place_change_nothing() {
        let asking = HoldState {
            phase: Phase::AskingDelete {
                recorded_ms: 12_000,
            },
            ..idle()
        };
        let prompting = HoldState {
            phase: Phase::AwaitingPermission {
                source: Source::Menu,
                beside_draft: true,
            },
            ..idle()
        };
        let record = |at_ms| HoldEvent::Record {
            at_ms,
            beside_draft: false,
            situation: sit(),
            recorded_ms: 0,
        };
        let cases: Vec<(HoldState, HoldEvent)> = vec![
            (
                idle(),
                HoldEvent::Delete {
                    at_ms: 10,
                    recorded_ms: 5_000,
                },
            ),
            (
                idle(),
                HoldEvent::Answer {
                    at_ms: 10,
                    delete: true,
                },
            ),
            (
                idle(),
                HoldEvent::PermissionAnswer {
                    at_ms: 10,
                    granted: true,
                },
            ),
            (idle(), HoldEvent::OtherAction { at_ms: 10 }),
            (
                hands_free(false),
                HoldEvent::Answer {
                    at_ms: 2_000,
                    delete: true,
                },
            ),
            (
                hands_free(false),
                HoldEvent::PermissionAnswer {
                    at_ms: 2_000,
                    granted: true,
                },
            ),
            (hands_free(false), HoldEvent::OtherAction { at_ms: 2_000 }),
            (asking, activate(20_000, 0)),
            (asking, record(20_000)),
            (
                asking,
                HoldEvent::Stop {
                    at_ms: 20_000,
                    recorded_ms: 12_000,
                },
            ),
            (
                asking,
                HoldEvent::Delete {
                    at_ms: 20_000,
                    recorded_ms: 12_000,
                },
            ),
            (asking, HoldEvent::Cap { at_ms: 20_000 }),
            (prompting, activate(20_000, 0)),
            (prompting, record(20_000)),
            (
                prompting,
                HoldEvent::Answer {
                    at_ms: 20_000,
                    delete: true,
                },
            ),
            (prompting, HoldEvent::Cap { at_ms: 20_000 }),
        ];
        for (s, e) in cases {
            let (after, fx) = step(s, e);
            assert!(fx.is_empty(), "{s:?} {e:?} -> {fx:?}");
            assert_eq!(after, s, "{e:?}");
        }
    }

    #[test]
    fn the_slot_is_told_what_records_and_the_guard_is_strict() {
        assert_eq!(idle().recording(), Recording::None);
        assert_eq!(hands_free(false).recording(), Recording::HandsFree);
        assert_eq!(
            hands_free(true).recording(),
            Recording::HandsFreeBesideDraft
        );
        let guarded = HoldState {
            guard_until_ms: 600,
            ..idle()
        };
        assert!(guarded.guarded(599));
        assert!(!guarded.guarded(600));
        assert!(!idle().guarded(0));
    }

    /// Nothing but the slot's Send arrow sends: walk every phase through every
    /// event and look for a Send anywhere else.
    #[test]
    fn only_the_slots_send_arrow_ever_sends() {
        let phases = [
            idle(),
            hands_free(false),
            hands_free(true),
            HoldState {
                phase: Phase::AskingDelete {
                    recorded_ms: 12_000,
                },
                ..idle()
            },
            HoldState {
                phase: Phase::AwaitingPermission {
                    source: Source::Tap,
                    beside_draft: false,
                },
                ..idle()
            },
        ];
        let events = |at_ms: u64| {
            [
                HoldEvent::Cap { at_ms },
                HoldEvent::Interruption {
                    at_ms,
                    recorded_ms: 9_000,
                },
                HoldEvent::Activate {
                    at_ms,
                    situation: sit(),
                    recorded_ms: 9_000,
                },
                HoldEvent::Record {
                    at_ms,
                    beside_draft: false,
                    situation: sit(),
                    recorded_ms: 9_000,
                },
                HoldEvent::Stop {
                    at_ms,
                    recorded_ms: 9_000,
                },
                HoldEvent::Delete {
                    at_ms,
                    recorded_ms: 9_000,
                },
                HoldEvent::Answer {
                    at_ms,
                    delete: false,
                },
                HoldEvent::PermissionAnswer {
                    at_ms,
                    granted: true,
                },
                HoldEvent::OtherAction { at_ms },
                HoldEvent::Emptied { at_ms },
            ]
        };
        for s in phases {
            for e in events(50_000) {
                let (_, fx) = step(s, e);
                let is_send_arrow =
                    s == hands_free(false) && matches!(e, HoldEvent::Activate { .. });
                assert_eq!(fx.contains(&E::Send), is_send_arrow, "{s:?} {e:?}");
            }
        }
    }

    #[test]
    fn the_clocks_never_overflow() {
        let late = u64::MAX - 10;
        let (s, fx) = step(idle(), activate(late, 0));
        assert_eq!(fx, started());
        assert_eq!(s.guard_until_ms, u64::MAX);
        let (s, _) = step(hands_free(false), activate(late, 2_000));
        assert_eq!(s.guard_until_ms, u64::MAX);
        let (s, _) = step(idle(), HoldEvent::Emptied { at_ms: late });
        assert_eq!(s.guard_until_ms, u64::MAX);
    }
}
