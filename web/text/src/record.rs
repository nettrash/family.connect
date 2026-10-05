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
//!   hidden, dimmed or shown (S1.4). No build records round video yet, so
//!   every client passes `records_round_video: false` and the door stays shut
//!   until its platform's Phase 3; the rule is here so that phase only wires it.
//! - [`hold_step`] — what a press, a hold, a slide, a release, a timer, the
//!   length limit or an interruption does to a voice recording (S2.1, S2.3,
//!   S2.5, S2.6), as a reducer: a state and an event go in, the next state and
//!   what to do come out.
//!
//! and the round video's arithmetic ([`round_cap_ms`], [`round_warning_ms`],
//! [`round_diameter`], [`is_round`]) and the S1.1 constants.
//!
//! It is only the DECISION. What the composer is, as a client saw it, goes in;
//! recording, playing, drawing, staging and parking are each platform's own
//! business. The web client runs this module as it is. The Apple and Android
//! ports are held to every rule here, the hold included, and the Windows port
//! to the slot, the door and the round helpers (it has no hold, S8.6), by the
//! vectors `win/tools/board-oracle` prints from it (`cargo run -- record`),
//! committed as `record-vectors.json` beside each port's tests — "an oracle,
//! not four readings". So the names are plain on purpose, and every field is
//! one a Swift struct, a Kotlin data class and a C# record can carry under the
//! same name.
//!
//! Words are the apps' English source strings — the catalogue's keys (S10) —
//! and each client says them in its reader's language. Nothing here
//! translates, so the vectors read the same in every language.
//!
//! Every function is total. Times are milliseconds on one monotonic clock;
//! distances are the platform's own unit (pt, dp, epx, CSS px) in WINDOW
//! coordinates, y growing downward as it does on every platform here.

// --- S1.1, the constants ------------------------------------------------------------------------

/// The slot ignores activation for this long only after its OWN activation
/// changed it — a send that empties the composer, a tap or hold that starts a
/// recording, a Send or a release that ends one or finds it too short, a Stop
/// in row 3 that stages the note. A press that goes down while it runs is
/// ignored whole. A change made by typing, pasting or staging is never
/// guarded — it lifts the guard ([`HoldEvent::OtherAction`]) — so "ok"
/// followed at once by Send still sends. The video button
/// ignores activation for the same 600 ms after it appears.
pub const ACTIVATION_GUARD_MS: u64 = 600;

/// The hold threshold's floor: H = max(500 ms, the system long-press
/// duration) — see [`hold_threshold_ms`].
pub const MIN_HOLD_THRESHOLD_MS: u64 = 500;

/// A press that moves farther than this from where it went down, before H,
/// can no longer become a hold. It still taps if it lifts inside the button.
pub const TAP_SLOP: f64 = 20.0;

/// Upward from where the press went down: the hold locks into a hands-free
/// recording.
pub const LOCK_DISTANCE: f64 = 60.0;

/// Toward the leading edge — left in a left-to-right layout, right in a
/// right-to-left one — from where the press went down: cancel is armed at
/// [`CANCEL_ARM_DISTANCE`] and disarmed again below [`CANCEL_DISARM_DISTANCE`].
pub const CANCEL_ARM_DISTANCE: f64 = 100.0;
pub const CANCEL_DISARM_DISTANCE: f64 = 80.0;

/// Nothing shorter is ever sent; a hold released sooner keeps recording.
/// It replaces the recorders' 1024-byte rule as the floor people see.
pub const SHORTEST_RECORDING_MS: u64 = 1_000;

/// After a release that sends: the grace before anything leaves the device.
pub const UNDO_WINDOW_MS: u64 = 5_000;

/// A voice note's length, unchanged (docs/protocol.md, "A browser is a client
/// too"), and the moment "30 seconds left" is shown and announced.
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

/// "Still recording. Tap Send when you're done." stays this long.
pub const STILL_RECORDING_HINT_MS: u64 = 3_000;

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

/// The hold threshold H for a system whose own long press takes
/// `system_long_press_ms` — Android's `ViewConfiguration.getLongPressTimeout()`,
/// which follows the person's "Touch & hold delay"; iOS passes 500. Never
/// shorter than [`MIN_HOLD_THRESHOLD_MS`], so a brush does not open the
/// microphone, and never shorter than the person asked their system for.
pub fn hold_threshold_ms(system_long_press_ms: u64) -> u64 {
    system_long_press_ms.max(MIN_HOLD_THRESHOLD_MS)
}

// --- S1.3, the trailing slot --------------------------------------------------------------------

/// The voice recording the composer is showing, as far as the slot is
/// concerned ([`HoldState::recording`] says it of a reducer's state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Recording {
    /// No voice recording runs.
    #[default]
    None,
    /// A finger or pen is holding the microphone and it records (S2.3). A hold
    /// only ever begins on the microphone, so the composer was empty.
    Held,
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
    /// Row 2 while a finger holds it: the pressed microphone stays under the
    /// finger until the recording turns hands-free.
    HeldMicrophone,
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
    /// Row 10: the microphone — a hands-free recording on activation, and on a
    /// touch screen the walkie-talkie when held (S2.3).
    Microphone,
}

impl Slot {
    /// The S1.3 row this is.
    pub fn row(self) -> u8 {
        match self {
            Slot::Recorder => 1,
            Slot::HeldMicrophone | Slot::SendVoice => 2,
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
            Slot::HeldMicrophone | Slot::SendVoice => Some("Send voice message"),
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
        Recording::Held => return Slot::HeldMicrophone,
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
    /// A released voice message is waiting out its Undo window: its row takes
    /// the FIELD's place, and the button inside the field goes with it (S2.6).
    pub undo_window: bool,
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
/// recording, during the Undo window, in the assistant's chat and in threads,
/// against a server without the keys, on a device without a camera, in a
/// browser whose probe fails, and in every build that does not record round
/// video.
pub fn video_door(inputs: &DoorInputs) -> Door {
    if !inputs.family_or_direct_chat || inputs.undo_window || !inputs.round_available() {
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

// --- S2.1 and S2.3, the hold and the recording, as a reducer ------------------------------------

/// The numbers [`hold_step`] decides by: S1.1's, with H for this system. They
/// are constants, tuned after one device session (the plan's Blocked 3); they
/// travel as a value so that the vectors can say which ones a case used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoldConstants {
    /// H — [`hold_threshold_ms`].
    pub hold_threshold_ms: u64,
    pub tap_slop: f64,
    pub lock_distance: f64,
    pub cancel_arm_distance: f64,
    pub cancel_disarm_distance: f64,
    pub shortest_recording_ms: u64,
    pub undo_window_ms: u64,
    pub activation_guard_ms: u64,
    pub delete_asks_from_ms: u64,
}

impl Default for HoldConstants {
    fn default() -> Self {
        HoldConstants {
            hold_threshold_ms: MIN_HOLD_THRESHOLD_MS,
            tap_slop: TAP_SLOP,
            lock_distance: LOCK_DISTANCE,
            cancel_arm_distance: CANCEL_ARM_DISTANCE,
            cancel_disarm_distance: CANCEL_DISARM_DISTANCE,
            shortest_recording_ms: SHORTEST_RECORDING_MS,
            undo_window_ms: UNDO_WINDOW_MS,
            activation_guard_ms: ACTIVATION_GUARD_MS,
            delete_asks_from_ms: DELETE_ASKS_FROM_MS,
        }
    }
}

impl HoldConstants {
    /// S1.1's numbers, with H for a system whose long press takes
    /// `system_long_press_ms`.
    pub fn for_system(system_long_press_ms: u64) -> Self {
        HoldConstants {
            hold_threshold_ms: hold_threshold_ms(system_long_press_ms),
            ..HoldConstants::default()
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
/// what it knows; the defaults are a granted microphone, nothing in the way,
/// no screen reader, a device that has released before, and Review Before
/// Sending off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Situation {
    pub permission: Permission,
    /// The dimmed row the microphone is in (rows 7–9), if any — at activation
    /// and at H, nothing records and its sentence is said.
    pub blocked: Option<Dimmed>,
    /// A screen reader or Switch Control is running: a held release goes to
    /// review, never to the Undo window (S2.3, S2.6).
    pub assistive: bool,
    /// This device has not yet had its first held release taught ("Next
    /// time, letting go will send it."); [`HoldEffect::FirstReleaseDone`] is
    /// the port's cue to remember that it has.
    pub first_release: bool,
    /// The per-device setting (S9): a held release goes to review.
    pub review_before_sending: bool,
}

/// Where a recording that is waiting on the permission prompt came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A completed tap on the microphone, a click, Enter or Space, a screen
    /// reader's activation: Allow starts recording — the tap meant "record".
    Tap,
    /// The hold threshold: a prompt raised by a hold never records, whatever
    /// the answer; Allow says "You can record now."
    Hold,
    /// The paperclip's or the microphone menu's "Record Voice Message", or the
    /// shortcut: Allow starts recording, beside the draft if there is one.
    Menu,
}

/// Where the slot's voice recording is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// Nothing pressed, nothing recording. A released note may still be
    /// waiting out its Undo window ([`HoldState::undo`]).
    Idle,
    /// A finger or pen is down on the microphone, before H; nothing records.
    Pressed {
        down_at_ms: u64,
        /// Where it went down, in window coordinates.
        down_x: f64,
        down_y: f64,
        /// The layout is right-to-left: the leading edge is on the right.
        rtl: bool,
        /// It can still become a hold: a touch or pen press on iPhone, iPad
        /// or Android that has not moved beyond the slop.
        may_hold: bool,
    },
    /// Recording, the finger still down: the hold row.
    Holding {
        down_x: f64,
        down_y: f64,
        rtl: bool,
        /// Slide-to-cancel is armed: the row reads "Release to cancel".
        armed: bool,
    },
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

/// A released note waiting out its Undo window: nothing has left the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndoNote {
    /// When the window runs out and the note goes to the outbox.
    pub until_ms: u64,
    pub recorded_ms: u64,
}

/// The reducer's whole state: the phase, the activation guard, and a note in
/// its Undo window — which is not a phase, because the microphone is usable
/// while it waits and a new press must not end the window before it is a tap
/// or a hold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoldState {
    pub phase: Phase,
    /// Activation of the slot before this moment is ignored whole. 0: none.
    pub guard_until_ms: u64,
    pub undo: Option<UndoNote>,
}

impl Default for HoldState {
    fn default() -> Self {
        HoldState {
            phase: Phase::Idle,
            guard_until_ms: 0,
            undo: None,
        }
    }
}

impl HoldState {
    /// What [`composer_slot`] is told about this recording.
    pub fn recording(&self) -> Recording {
        match self.phase {
            Phase::Holding { .. } => Recording::Held,
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
/// `Down`, `Move`, `Up` and `SystemCancel` are the MICROPHONE's touch — a
/// press that may become a hold — and matter only from `Idle`. Every other
/// activation of the slot — the Send arrow, the Stop square, a click, Enter or
/// Space, a screen reader's — is `Activate`. The lift of a press that went
/// down on the microphone is always `Up`, whatever the slot shows by then:
/// after a lock it does nothing (S2.3), so a port must never turn it into
/// `Activate`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HoldEvent {
    /// A press went down on the microphone. `can_hold`: a finger or pen on
    /// iPhone, iPad or Android; false for a mouse, a trackpad, Windows and a
    /// browser, where a press of any length is a click.
    Down {
        at_ms: u64,
        x: f64,
        y: f64,
        can_hold: bool,
        rtl: bool,
    },
    /// It moved, in window coordinates.
    Move { at_ms: u64, x: f64, y: f64 },
    /// It lifted; `inside` is the button's own hit test. Its position counts
    /// as a last move first.
    Up {
        at_ms: u64,
        x: f64,
        y: f64,
        inside: bool,
        situation: Situation,
        recorded_ms: u64,
        /// Some peak since the recording started rose above the silence level.
        heard: bool,
    },
    /// The system cancelled the touch: an alert, Control Centre, the
    /// notification shade, a rotation. `background`: the app has gone there.
    SystemCancel {
        at_ms: u64,
        background: bool,
        recorded_ms: u64,
    },
    /// Time passed. A port sends one at H — its long-press timer — and whenever
    /// it likes otherwise; the Undo window runs out on every event's clock.
    Tick { at_ms: u64, situation: Situation },
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
    /// The Undo row's Undo.
    Undo { at_ms: u64 },
    /// Any other action of the person's in the composer: a character typed
    /// or deleted, a paste, a suggestion taken, an item staged or taken off
    /// the strip, the paperclip, a sticker. It ends the Undo window early,
    /// and — while nothing records — it lifts the activation guard: a change
    /// the person made is never guarded (S1.1), so "ok" sent and "x" typed
    /// and deleted at once leaves a microphone that records, and a photo
    /// pasted straight after a Send leaves a Send that sends. A port says it
    /// for every such change, never for its own: a Send emptying the box is
    /// `Emptied`, and the note a Stop in row 3 stages is the Stop's.
    OtherAction { at_ms: u64 },
    /// The slot's own Send or Save just emptied the composer: the microphone
    /// it turns into is guarded.
    Emptied { at_ms: u64 },
}

impl HoldEvent {
    pub fn at_ms(&self) -> u64 {
        match *self {
            HoldEvent::Down { at_ms, .. }
            | HoldEvent::Move { at_ms, .. }
            | HoldEvent::Up { at_ms, .. }
            | HoldEvent::SystemCancel { at_ms, .. }
            | HoldEvent::Tick { at_ms, .. }
            | HoldEvent::Cap { at_ms }
            | HoldEvent::Interruption { at_ms, .. }
            | HoldEvent::Activate { at_ms, .. }
            | HoldEvent::Record { at_ms, .. }
            | HoldEvent::Stop { at_ms, .. }
            | HoldEvent::Delete { at_ms, .. }
            | HoldEvent::Answer { at_ms, .. }
            | HoldEvent::PermissionAnswer { at_ms, .. }
            | HoldEvent::Undo { at_ms }
            | HoldEvent::OtherAction { at_ms }
            | HoldEvent::Emptied { at_ms } => at_ms,
        }
    }
}

/// S2.9's haptics, phones only — a port drops them on an iPad, the Mac,
/// Windows and the web.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Haptic {
    /// Recording starts from a tap: iPhone `.impact(weight: .light)`, Android
    /// `ToggleOn`.
    Light,
    /// Recording starts at H: `.impact(weight: .medium)`, `LongPress`.
    Medium,
    /// Lock; cancel armed: `.selection`, `GestureThresholdActivate`.
    Selection,
    /// Sent: `.success`, `Confirm`.
    Success,
    /// Too short; deleted: `.warning`, `Reject`.
    Warning,
}

/// A line the composer SHOWS — in the row or its notice line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    /// A hold released under a second keeps recording; shown for
    /// [`STILL_RECORDING_HINT_MS`].
    StillRecording,
    /// The first held release on the device goes to review with this.
    NextTimeSends,
    /// A held release that never rose above the silence level.
    NothingHeard,
    StoppedAtFiveMinutes,
    TooShort,
    /// Allow, after a prompt a hold raised.
    CanRecordNow,
}

impl Hint {
    pub fn text(self) -> &'static str {
        match self {
            Hint::StillRecording => "Still recording. Tap Send when you're done.",
            Hint::NextTimeSends => "Next time, letting go will send it.",
            Hint::NothingHeard => "We didn't hear anything.",
            Hint::StoppedAtFiveMinutes => "Recording stopped at five minutes.",
            Hint::TooShort => "That recording was too short.",
            Hint::CanRecordNow => "You can record now.",
        }
    }
}

/// What is SPOKEN, politely, to a screen reader (S6) — state changes only,
/// never the ticking clock. "Not sent" and "Undo available" are never said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Announcement {
    Recording,
    RecordingLocked,
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
            Announcement::RecordingLocked => "Recording locked",
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
    /// Open the microphone and record: the hold row when `held`, otherwise
    /// the recording row. Keep the screen awake, pause anything playing; a
    /// hands-free start moves focus to the slot.
    Start {
        held: bool,
    },
    /// Hands-free from here: the hold row becomes the recording row, the
    /// keyboard goes down, and the finger's later lift does nothing.
    Lock,
    /// Cancel armed: the row turns red and reads "Release to cancel".
    Arm,
    /// "‹ Slide to cancel" again.
    Disarm,
    /// Stop the recording if it runs, and delete it.
    Delete,
    /// Stop it, and hand it to the outbox now, with the primed reply.
    Send,
    /// Stop it, and stage it in the chip above the field (S2.7).
    Review,
    /// Stop it, and keep it as the chat's "Voice message not sent" row (S2.8).
    Park,
    /// Stop it, write it to the parked store marked "sending", and show the
    /// Undo row: nothing has left the device yet (S2.6).
    UndoWindow,
    /// The note in its Undo window goes to the outbox now; its "sending"
    /// entry goes with the hand-off.
    UndoSend,
    /// The note in its Undo window goes to review instead; nothing is sent.
    UndoReview,
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
    /// Remember, on this device, that its first held release has been taught.
    FirstReleaseDone,
}

/// One step of the slot's voice recording (S2.1, S2.3, S2.5, S2.6): the state
/// and an event in, the next state and what to do out.
///
/// The rules, in the order they are asked:
///
/// - **The Undo window runs out on every event's clock.** A note whose five
///   seconds have passed is sent before the event itself is looked at.
/// - **The activation guard** swallows a press that goes down, or an
///   `Activate`, while it runs — whole: it can become neither a tap nor a
///   hold. Nothing else is guarded, and the person's own change to the
///   composer (`OtherAction`) lifts it.
/// - **A press** becomes a tap when it lifts inside the button, wherever it
///   wandered, and nothing when it lifts outside; it becomes a hold only on a
///   `Tick` at H while it has not moved beyond the slop.
/// - **The microphone's activation** — a tap, H, `Activate`, `Record` — sends
///   a note still in its Undo window first, then says a dimmed row's sentence,
///   raises the prompt, gives the denial notice, or starts.
/// - **A hold** arms cancel at 100 toward the leading edge and disarms it
///   below 80, and locks at 60 up while cancel is not armed. Its release
///   deletes when armed; keeps recording hands-free under a second; reviews a
///   silent recording; reviews when it is the device's first release, when
///   Review Before Sending is on, or while a screen reader or Switch Control
///   runs; and otherwise opens the Undo window.
/// - **Ending a recording**: Send sends (under a second it is too short);
///   Stop and the shortcut review (too short likewise); Delete deletes under
///   ten seconds and asks from ten; the five-minute limit reviews; an
///   interruption parks it as not sent (under a second it is deleted); an
///   interruption during the Undo window sends.
pub fn hold_step(
    state: HoldState,
    event: HoldEvent,
    constants: &HoldConstants,
) -> (HoldState, Vec<HoldEffect>) {
    let mut s = state;
    let mut fx = Vec::new();
    let at = event.at_ms();
    let c = constants;

    // The Undo window is a grace period after a decision already made, and it
    // runs out on its own clock: a late timer must not keep a note waiting,
    // nor let an Undo pressed after the five seconds take it back.
    if let Some(note) = s.undo {
        if at >= note.until_ms {
            undo_send(&mut s, &mut fx);
        }
    }

    match event {
        HoldEvent::Down {
            x,
            y,
            can_hold,
            rtl,
            ..
        } => {
            if s.phase == Phase::Idle && !s.guarded(at) {
                s.phase = Phase::Pressed {
                    down_at_ms: at,
                    down_x: x,
                    down_y: y,
                    rtl,
                    may_hold: can_hold,
                };
            }
        }
        HoldEvent::Move { x, y, .. } => match s.phase {
            Phase::Pressed {
                down_at_ms,
                down_x,
                down_y,
                rtl,
                may_hold: true,
            } => {
                let (dx, dy) = (x - down_x, y - down_y);
                if dx * dx + dy * dy > c.tap_slop * c.tap_slop {
                    s.phase = Phase::Pressed {
                        down_at_ms,
                        down_x,
                        down_y,
                        rtl,
                        may_hold: false,
                    };
                }
            }
            Phase::Holding { .. } => slide(&mut s, &mut fx, x, y, c),
            _ => {}
        },
        HoldEvent::Up {
            x,
            y,
            inside,
            situation,
            recorded_ms,
            heard,
            ..
        } => match s.phase {
            Phase::Pressed { .. } => {
                if inside {
                    activate_microphone(&mut s, &mut fx, at, &situation, Source::Tap, false, c);
                } else {
                    s.phase = Phase::Idle;
                }
            }
            Phase::Holding { .. } => {
                slide(&mut s, &mut fx, x, y, c);
                if let Phase::Holding { armed, .. } = s.phase {
                    s.guard_until_ms = at.saturating_add(c.activation_guard_ms);
                    if armed {
                        s.phase = Phase::Idle;
                        deleted(&mut fx);
                    } else {
                        release(&mut s, &mut fx, at, &situation, recorded_ms, heard, c);
                    }
                }
            }
            _ => {}
        },
        HoldEvent::SystemCancel {
            background,
            recorded_ms,
            ..
        } => match s.phase {
            Phase::Pressed { .. } => s.phase = Phase::Idle,
            Phase::Holding { armed: true, .. } => {
                s.phase = Phase::Idle;
                deleted(&mut fx);
            }
            Phase::Holding { .. } if background => {
                s.phase = Phase::Idle;
                interrupted(&mut fx, recorded_ms, c);
            }
            Phase::Holding { .. } => {
                s.phase = Phase::HandsFree {
                    beside_draft: false,
                };
                fx.push(HoldEffect::Lock);
                fx.push(HoldEffect::Announce(Announcement::RecordingLocked));
            }
            _ => {}
        },
        HoldEvent::Tick { situation, .. } => {
            if let Phase::Pressed {
                down_at_ms,
                down_x,
                down_y,
                rtl,
                may_hold: true,
            } = s.phase
            {
                if at.saturating_sub(down_at_ms) >= c.hold_threshold_ms {
                    send_waiting(&mut s, &mut fx);
                    match refusal(&situation) {
                        Some(HoldEffect::AskPermission) => {
                            s.phase = Phase::AwaitingPermission {
                                source: Source::Hold,
                                beside_draft: false,
                            };
                            fx.push(HoldEffect::AskPermission);
                        }
                        Some(effect) => {
                            s.phase = Phase::Idle;
                            fx.push(effect);
                        }
                        None => {
                            s.phase = Phase::Holding {
                                down_x,
                                down_y,
                                rtl,
                                armed: false,
                            };
                            s.guard_until_ms = at.saturating_add(c.activation_guard_ms);
                            fx.push(HoldEffect::Start { held: true });
                            fx.push(HoldEffect::Announce(Announcement::Recording));
                            fx.push(HoldEffect::Haptic(Haptic::Medium));
                        }
                    }
                }
            }
        }
        HoldEvent::Cap { .. } => {
            if recording(&s) {
                s.phase = Phase::Idle;
                fx.push(HoldEffect::Review);
                fx.push(HoldEffect::Hint(Hint::StoppedAtFiveMinutes));
                fx.push(HoldEffect::Announce(Announcement::StoppedAtFiveMinutes));
            }
        }
        HoldEvent::Interruption { recorded_ms, .. } => {
            if s.undo.is_some() {
                undo_send(&mut s, &mut fx);
            }
            match s.phase {
                Phase::Holding { .. } | Phase::HandsFree { .. } => {
                    s.phase = Phase::Idle;
                    interrupted(&mut fx, recorded_ms, c);
                }
                // The question's answer never came: the recording is kept,
                // never lost and never sent, and the question goes with it.
                Phase::AskingDelete { .. } => {
                    s.phase = Phase::Idle;
                    fx.push(HoldEffect::Park);
                }
                Phase::Pressed { .. } | Phase::AwaitingPermission { .. } => s.phase = Phase::Idle,
                Phase::Idle => {}
            }
        }
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
                    _ => {}
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
            Phase::Holding { .. } | Phase::HandsFree { .. } => {
                s.phase = Phase::Idle;
                stop_into_review(&mut fx, recorded_ms, c);
            }
            _ => {}
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
                match (granted, source) {
                    (false, _) => fx.push(HoldEffect::Denied),
                    (true, Source::Hold) => fx.push(HoldEffect::Hint(Hint::CanRecordNow)),
                    (true, source) => {
                        start_hands_free(&mut s, &mut fx, at, source, beside_draft, c)
                    }
                }
            }
        }
        HoldEvent::Undo { .. } => {
            if let Some(note) = s.undo.take() {
                fx.push(HoldEffect::UndoReview);
                fx.push(HoldEffect::Announce(Announcement::ReadyToReview {
                    recorded_ms: note.recorded_ms,
                }));
            }
        }
        HoldEvent::OtherAction { .. } => {
            if s.undo.is_some() {
                undo_send(&mut s, &mut fx);
            }
            // The person changed the composer: the next press is a decision
            // of its own, not the second half of a double tap. While a
            // recording runs the box is behind the row and nothing in it is
            // the person's to change, so the guard that keeps a double tap
            // on the microphone from sending stays.
            if s.phase == Phase::Idle {
                s.guard_until_ms = 0;
            }
        }
        HoldEvent::Emptied { .. } => {
            // A text Send is an action like any other: a released note that
            // is still waiting goes first. (A port says OtherAction when the
            // first character is typed, so this is a backstop.)
            if s.undo.is_some() {
                undo_send(&mut s, &mut fx);
            }
            s.guard_until_ms = at.saturating_add(c.activation_guard_ms);
        }
    }
    (s, fx)
}

/// A voice recording runs.
fn recording(s: &HoldState) -> bool {
    matches!(s.phase, Phase::Holding { .. } | Phase::HandsFree { .. })
}

/// What stands between the microphone's activation and a recording, in this
/// order: a dimmed row's sentence, the denial notice, or the prompt. None:
/// start. (The prompt's phase is the caller's to name — it knows where the
/// start came from.)
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

/// The microphone's completed activation from a tap or a menu (H has its own
/// path, because it starts the HOLD row).
fn activate_microphone(
    s: &mut HoldState,
    fx: &mut Vec<HoldEffect>,
    at: u64,
    situation: &Situation,
    source: Source,
    beside_draft: bool,
    c: &HoldConstants,
) {
    send_waiting(s, fx);
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
    fx.push(HoldEffect::Start { held: false });
    fx.push(HoldEffect::Announce(Announcement::Recording));
    fx.push(HoldEffect::Haptic(Haptic::Light));
}

/// A held finger moved (or lifted, its position counting as a last move):
/// arm or disarm cancel, then lock — never while cancel is armed.
fn slide(s: &mut HoldState, fx: &mut Vec<HoldEffect>, x: f64, y: f64, c: &HoldConstants) {
    let Phase::Holding {
        down_x,
        down_y,
        rtl,
        armed,
    } = s.phase
    else {
        return;
    };
    let toward_leading = if rtl { x - down_x } else { down_x - x };
    let up = down_y - y;
    let mut now_armed = armed;
    if !armed && toward_leading >= c.cancel_arm_distance {
        now_armed = true;
        fx.push(HoldEffect::Arm);
        fx.push(HoldEffect::Haptic(Haptic::Selection));
    } else if armed && toward_leading < c.cancel_disarm_distance {
        now_armed = false;
        fx.push(HoldEffect::Disarm);
    }
    if !now_armed && up >= c.lock_distance {
        s.phase = Phase::HandsFree {
            beside_draft: false,
        };
        fx.push(HoldEffect::Lock);
        fx.push(HoldEffect::Announce(Announcement::RecordingLocked));
        fx.push(HoldEffect::Haptic(Haptic::Selection));
    } else {
        s.phase = Phase::Holding {
            down_x,
            down_y,
            rtl,
            armed: now_armed,
        };
    }
}

/// A hold let go with cancel not armed (S2.3), in the plan's order.
fn release(
    s: &mut HoldState,
    fx: &mut Vec<HoldEffect>,
    at: u64,
    situation: &Situation,
    recorded_ms: u64,
    heard: bool,
    c: &HoldConstants,
) {
    if recorded_ms < c.shortest_recording_ms {
        s.phase = Phase::HandsFree {
            beside_draft: false,
        };
        fx.push(HoldEffect::Lock);
        fx.push(HoldEffect::Hint(Hint::StillRecording));
        return;
    }
    s.phase = Phase::Idle;
    if !heard {
        fx.push(HoldEffect::Review);
        fx.push(HoldEffect::Hint(Hint::NothingHeard));
        fx.push(HoldEffect::Announce(Announcement::ReadyToReview {
            recorded_ms,
        }));
        return;
    }
    if situation.first_release || situation.review_before_sending || situation.assistive {
        // "Next time, letting go will send it" is said only when it is true:
        // not to somebody whose releases always review.
        let teach =
            situation.first_release && !situation.review_before_sending && !situation.assistive;
        fx.push(HoldEffect::Review);
        if teach {
            fx.push(HoldEffect::Hint(Hint::NextTimeSends));
        }
        fx.push(HoldEffect::Announce(Announcement::ReadyToReview {
            recorded_ms,
        }));
        if teach {
            fx.push(HoldEffect::FirstReleaseDone);
        }
        return;
    }
    s.undo = Some(UndoNote {
        until_ms: at.saturating_add(c.undo_window_ms),
        recorded_ms,
    });
    fx.push(HoldEffect::UndoWindow);
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

/// The microphone's activation ends the Undo window by sending: letting go
/// was the person's decision.
fn send_waiting(s: &mut HoldState, fx: &mut Vec<HoldEffect>) {
    if s.undo.is_some() {
        undo_send(s, fx);
    }
}

fn undo_send(s: &mut HoldState, fx: &mut Vec<HoldEffect>) {
    s.undo = None;
    fx.push(HoldEffect::UndoSend);
    fx.push(HoldEffect::Announce(Announcement::VoiceMessageSent));
    fx.push(HoldEffect::Haptic(Haptic::Success));
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
        assert_eq!(MIN_HOLD_THRESHOLD_MS, 500);
        assert_eq!(TAP_SLOP, 20.0);
        assert_eq!(LOCK_DISTANCE, 60.0);
        assert_eq!((CANCEL_ARM_DISTANCE, CANCEL_DISARM_DISTANCE), (100.0, 80.0));
        assert_eq!(SHORTEST_RECORDING_MS, 1_000);
        assert_eq!(UNDO_WINDOW_MS, 5_000);
        assert_eq!((VOICE_CAP_MS, VOICE_WARNING_MS), (300_000, 270_000));
        assert_eq!(DEFAULT_MAX_ROUND_VIDEO_MS, 60_000);
        assert_eq!((ROUND_CAP_MARGIN_MS, ROUND_WARNING_LEAD_MS), (500, 10_000));
        assert_eq!(SILENCE_PEAK_DBFS, -60.0);
        assert_eq!(SILENCE_MAX_AMPLITUDE, 32);
        assert_eq!(SILENCE_SAMPLE_MAGNITUDE, 0.001);
        assert_eq!(SILENCE_WARNING_AFTER_MS, 3_000);
        assert_eq!(DELETE_ASKS_FROM_MS, 10_000);
        assert_eq!(STILL_RECORDING_HINT_MS, 3_000);
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
        let defaults = HoldConstants::default();
        assert_eq!(defaults.hold_threshold_ms, 500);
        assert_eq!(defaults.activation_guard_ms, ACTIVATION_GUARD_MS);
        assert_eq!(defaults.undo_window_ms, UNDO_WINDOW_MS);
        assert_eq!(defaults.delete_asks_from_ms, DELETE_ASKS_FROM_MS);
    }

    #[test]
    fn the_hold_threshold_follows_a_longer_system_setting_and_never_a_shorter_one() {
        for (system, h) in [
            (0, 500),
            (400, 500),
            (500, 500),
            (501, 501),
            (1_000, 1_000),
            (1_500, 1_500),
        ] {
            assert_eq!(hold_threshold_ms(system), h, "{system}");
            assert_eq!(HoldConstants::for_system(system).hold_threshold_ms, h);
        }
        assert_eq!(
            HoldConstants::for_system(1_000),
            HoldConstants {
                hold_threshold_ms: 1_000,
                ..HoldConstants::default()
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
    fn a_hold_keeps_the_pressed_microphone_under_the_finger() {
        let held = SlotInputs {
            recording: Recording::Held,
            ..inputs()
        };
        let slot = composer_slot(&held);
        assert_eq!(slot, Slot::HeldMicrophone);
        assert_eq!(slot.row(), 2);
        assert_eq!(slot.label(), Some("Send voice message"));
        assert!(!slot.is_microphone());
        // Row 3's words are behind the row: its slot is Stop, never Send.
        let beside = with_row(inputs(), 3);
        assert_eq!(composer_slot(&beside), Slot::StopRecording);
    }

    // --- S1.4 --------------------------------------------------------------------------------------

    fn door(slot: SlotInputs) -> DoorInputs {
        DoorInputs {
            slot,
            family_or_direct_chat: true,
            undo_window: false,
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
    fn the_door_is_hidden_outside_a_family_or_direct_chat_and_during_the_undo_window() {
        let elsewhere = DoorInputs {
            family_or_direct_chat: false,
            ..door(inputs())
        };
        assert_eq!(video_door(&elsewhere), Door::Hidden);
        let waiting = DoorInputs {
            undo_window: true,
            ..door(inputs())
        };
        assert_eq!(video_door(&waiting), Door::Hidden);
        let staged = door(SlotInputs {
            staged: true,
            ..inputs()
        });
        assert_eq!(video_door(&staged), Door::Hidden);
        let held = door(SlotInputs {
            recording: Recording::Held,
            ..inputs()
        });
        assert_eq!(video_door(&held), Door::Hidden);
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

    // --- S2.1 and S2.3 -----------------------------------------------------------------------------

    const X: f64 = 340.0;
    const Y: f64 = 780.0;

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

    fn down(at: u64) -> HoldEvent {
        HoldEvent::Down {
            at_ms: at,
            x: X,
            y: Y,
            can_hold: true,
            rtl: false,
        }
    }

    fn mv(at: u64, dx: f64, dy: f64) -> HoldEvent {
        HoldEvent::Move {
            at_ms: at,
            x: X + dx,
            y: Y + dy,
        }
    }

    fn up_with(
        at: u64,
        dx: f64,
        dy: f64,
        recorded_ms: u64,
        heard: bool,
        situation: Situation,
    ) -> HoldEvent {
        HoldEvent::Up {
            at_ms: at,
            x: X + dx,
            y: Y + dy,
            inside: true,
            situation,
            recorded_ms,
            heard,
        }
    }

    fn up(at: u64, recorded_ms: u64) -> HoldEvent {
        up_with(at, 0.0, 0.0, recorded_ms, true, sit())
    }

    fn up_outside(at: u64) -> HoldEvent {
        HoldEvent::Up {
            at_ms: at,
            x: X + 90.0,
            y: Y,
            inside: false,
            situation: sit(),
            recorded_ms: 0,
            heard: false,
        }
    }

    fn tick(at: u64) -> HoldEvent {
        HoldEvent::Tick {
            at_ms: at,
            situation: sit(),
        }
    }

    fn tick_with(at: u64, situation: Situation) -> HoldEvent {
        HoldEvent::Tick {
            at_ms: at,
            situation,
        }
    }

    fn activate(at: u64, recorded_ms: u64) -> HoldEvent {
        HoldEvent::Activate {
            at_ms: at,
            situation: sit(),
            recorded_ms,
        }
    }

    fn idle() -> HoldState {
        HoldState::default()
    }

    fn holding(armed: bool) -> HoldState {
        HoldState {
            phase: Phase::Holding {
                down_x: X,
                down_y: Y,
                rtl: false,
                armed,
            },
            guard_until_ms: 1_100,
            undo: None,
        }
    }

    fn hands_free(beside_draft: bool) -> HoldState {
        HoldState {
            phase: Phase::HandsFree { beside_draft },
            guard_until_ms: 0,
            undo: None,
        }
    }

    /// Idle with a released note of `recorded_ms` waiting until `until_ms`, the
    /// release's own guard long over.
    fn waiting(until_ms: u64, recorded_ms: u64) -> HoldState {
        HoldState {
            phase: Phase::Idle,
            guard_until_ms: 0,
            undo: Some(UndoNote {
                until_ms,
                recorded_ms,
            }),
        }
    }

    fn started(held: bool) -> Vec<HoldEffect> {
        vec![
            E::Start { held },
            E::Announce(A::Recording),
            E::Haptic(if held { Haptic::Medium } else { Haptic::Light }),
        ]
    }

    fn sent() -> Vec<HoldEffect> {
        vec![
            E::Send,
            E::Announce(A::VoiceMessageSent),
            E::Haptic(Haptic::Success),
        ]
    }

    fn undo_sent() -> Vec<HoldEffect> {
        vec![
            E::UndoSend,
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
    fn a_tap_records_hands_free_when_it_lifts_and_the_same_slot_sends() {
        let (s, fx) = step(idle(), down(0));
        assert!(fx.is_empty(), "nothing records on touch-down");
        assert!(matches!(s.phase, Phase::Pressed { may_hold: true, .. }));
        let (s, fx) = step(s, up(120, 0));
        assert_eq!(fx, started(false));
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        assert_eq!(s.guard_until_ms, 720);
        assert_eq!(s.recording(), Recording::HandsFree);
        let (s, fx) = step(s, activate(5_000, 4_800));
        assert_eq!(fx, sent());
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(
            s.guard_until_ms, 5_600,
            "a Send that ends a recording guards the microphone"
        );
    }

    #[test]
    fn a_double_tap_on_the_microphone_cannot_send_and_one_on_send_cannot_record() {
        let (s, _) = run(idle(), &[down(0), up(100, 0)]);
        let (s, fx) = step(s, activate(699, 590));
        assert!(fx.is_empty());
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        assert!(s.guarded(699) && !s.guarded(700));
        let (_, fx) = step(s, activate(700, 600));
        assert_eq!(fx, too_short_fx());

        // A text Send empties the composer: the microphone it becomes is guarded,
        // and a press that goes down inside the guard is ignored WHOLE.
        let (s, fx) = step(idle(), HoldEvent::Emptied { at_ms: 1_000 });
        assert!(fx.is_empty());
        let (s, fx) = run(s, &[down(1_599), tick(2_200), up(2_300, 0)]);
        assert!(fx.iter().all(Vec::is_empty), "{fx:?}");
        assert_eq!(s.phase, Phase::Idle);
        let (s, _) = step(s, down(1_600));
        assert!(matches!(s.phase, Phase::Pressed { .. }));
        let (_, fx) = step(idle(), activate(1_599, 0));
        assert_eq!(
            fx,
            started(false),
            "nothing guards a composer nobody emptied"
        );
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

    #[test]
    fn a_press_that_lifts_outside_does_nothing() {
        let (s, fx) = run(idle(), &[down(0), mv(50, 60.0, 0.0), up_outside(200)]);
        assert!(fx.iter().all(Vec::is_empty));
        assert_eq!(s, idle());
    }

    #[test]
    fn a_press_that_wanders_past_the_slop_still_taps_but_never_holds() {
        let (s, fx) = run(idle(), &[down(0), mv(40, 12.0, 16.5), tick(500), tick(900)]);
        assert!(fx.iter().all(Vec::is_empty));
        assert!(matches!(
            s.phase,
            Phase::Pressed {
                may_hold: false,
                ..
            }
        ));
        let (s, fx) = step(s, up(1_200, 0));
        assert_eq!(
            fx,
            started(false),
            "an unsteady press is never a dead button"
        );
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        // Exactly twenty is not "farther than" twenty.
        let (s, _) = run(idle(), &[down(0), mv(40, 12.0, 16.0)]);
        assert!(matches!(s.phase, Phase::Pressed { may_hold: true, .. }));
        let (s, fx) = step(s, tick(500));
        assert_eq!(fx, started(true));
        assert!(matches!(s.phase, Phase::Holding { armed: false, .. }));
    }

    #[test]
    fn the_hold_starts_at_h_and_not_a_millisecond_before() {
        let (s, fx) = run(idle(), &[down(1_000), tick(1_499)]);
        assert!(fx.iter().all(Vec::is_empty));
        let (s, fx) = step(s, tick(1_500));
        assert_eq!(fx, started(true));
        assert_eq!(s.guard_until_ms, 2_100);
        assert_eq!(s.recording(), Recording::Held);
        // A longer system setting moves H with it.
        let slow = HoldConstants::for_system(1_000);
        let (s, _) = hold_step(idle(), down(0), &slow);
        let (s, fx) = hold_step(s, tick(999), &slow);
        assert!(fx.is_empty());
        let (_, fx) = hold_step(s, tick(1_000), &slow);
        assert_eq!(fx, started(true));
        // An Up before any Tick reached H is a tap, however late.
        let (_, fx) = run(idle(), &[down(0), up(2_000, 0)]);
        assert_eq!(fx[1], started(false));
    }

    #[test]
    fn a_mouse_press_of_any_length_is_a_click() {
        let mouse = HoldEvent::Down {
            at_ms: 0,
            x: X,
            y: Y,
            can_hold: false,
            rtl: false,
        };
        let (s, fx) = run(idle(), &[mouse, tick(500), tick(3_000)]);
        assert!(fx.iter().all(Vec::is_empty));
        let (s, fx) = step(s, up(3_100, 0));
        assert_eq!(fx, started(false));
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
    }

    #[test]
    fn slide_to_cancel_arms_at_100_and_disarms_below_80() {
        let (s, fx) = step(holding(false), mv(600, -99.5, 0.0));
        assert!(fx.is_empty());
        let (s, fx) = step(s, mv(620, -100.0, 0.0));
        assert_eq!(fx, vec![E::Arm, E::Haptic(Haptic::Selection)]);
        let (s, fx) = run(
            s,
            &[
                mv(640, -130.0, 0.0),
                mv(660, -85.0, 0.0),
                mv(680, -80.0, 0.0),
            ],
        );
        assert!(fx.iter().all(Vec::is_empty), "80 is not below 80");
        assert!(matches!(s.phase, Phase::Holding { armed: true, .. }));
        let (s, fx) = step(s, mv(700, -79.5, 0.0));
        assert_eq!(fx, vec![E::Disarm]);
        assert!(matches!(s.phase, Phase::Holding { armed: false, .. }));
        let (s, _) = step(s, mv(720, -101.0, 0.0));
        let (s, fx) = step(s, up_with(2_000, -101.0, 0.0, 1_500, true, sit()));
        assert_eq!(fx, deleted_fx());
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(s.guard_until_ms, 2_600);
        assert!(s.undo.is_none());
    }

    #[test]
    fn in_a_right_to_left_layout_cancel_slides_right() {
        let rtl = HoldState {
            phase: Phase::Holding {
                down_x: X,
                down_y: Y,
                rtl: true,
                armed: false,
            },
            ..holding(false)
        };
        let (s, fx) = step(rtl, mv(600, -150.0, 0.0));
        assert!(fx.is_empty(), "left is the trailing edge here");
        let (_, fx) = step(s, mv(620, 100.0, 0.0));
        assert_eq!(fx, vec![E::Arm, E::Haptic(Haptic::Selection)]);
    }

    #[test]
    fn sliding_up_locks_at_60_and_the_lift_then_does_nothing() {
        let (s, fx) = step(holding(false), mv(600, 0.0, -59.5));
        assert!(fx.is_empty());
        let (s, fx) = step(s, mv(620, -10.0, -60.0));
        assert_eq!(
            fx,
            vec![
                E::Lock,
                E::Announce(A::RecordingLocked),
                E::Haptic(Haptic::Selection)
            ]
        );
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        let (s, fx) = step(s, up(3_000, 2_400));
        assert!(fx.is_empty(), "lifting a locked hold does nothing");
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        let (_, fx) = step(s, activate(9_000, 8_400));
        assert_eq!(fx, sent());
    }

    #[test]
    fn a_hold_never_locks_while_cancel_is_armed() {
        let (s, _) = step(holding(false), mv(600, -100.0, 0.0));
        let (s, fx) = step(s, mv(620, -110.0, -70.0));
        assert!(fx.is_empty());
        assert!(matches!(s.phase, Phase::Holding { armed: true, .. }));
        // Back below 80 while still up: disarmed, then locked, in one move.
        let (s, fx) = step(s, mv(640, -50.0, -70.0));
        assert_eq!(
            fx,
            vec![
                E::Disarm,
                E::Lock,
                E::Announce(A::RecordingLocked),
                E::Haptic(Haptic::Selection)
            ]
        );
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        // A diagonal that arms and rises at once arms and does not lock.
        let (s, fx) = step(holding(false), mv(600, -100.0, -60.0));
        assert_eq!(fx, vec![E::Arm, E::Haptic(Haptic::Selection)]);
        assert!(matches!(s.phase, Phase::Holding { armed: true, .. }));
    }

    #[test]
    fn a_lift_counts_its_position_as_a_last_move() {
        let (s, fx) = step(
            holding(false),
            up_with(2_000, 0.0, -65.0, 1_500, true, sit()),
        );
        assert_eq!(
            fx,
            vec![
                E::Lock,
                E::Announce(A::RecordingLocked),
                E::Haptic(Haptic::Selection)
            ]
        );
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        assert_eq!(s.guard_until_ms, 1_100, "a locked lift is no release");
        let (_, fx) = step(
            holding(false),
            up_with(2_000, -120.0, 0.0, 1_500, true, sit()),
        );
        let mut expected = vec![E::Arm, E::Haptic(Haptic::Selection)];
        expected.extend(deleted_fx());
        assert_eq!(fx, expected);
    }

    #[test]
    fn a_hold_let_go_under_a_second_keeps_recording_hands_free() {
        let (s, fx) = step(holding(false), up(1_400, 999));
        assert_eq!(fx, vec![E::Lock, E::Hint(Hint::StillRecording)]);
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        assert_eq!(
            s.guard_until_ms, 2_000,
            "a release that finds it too short guards the slot"
        );
        let (_, fx) = step(holding(false), up(1_400, 1_000));
        assert_eq!(fx, vec![E::UndoWindow]);
        // Under a second wins over silence: it is still recording.
        let (_, fx) = step(holding(false), up_with(1_400, 0.0, 0.0, 400, false, sit()));
        assert_eq!(fx, vec![E::Lock, E::Hint(Hint::StillRecording)]);
    }

    #[test]
    fn a_silent_hold_is_never_sent() {
        let (s, fx) = step(
            holding(false),
            up_with(4_000, 0.0, 0.0, 3_000, false, sit()),
        );
        assert_eq!(
            fx,
            vec![
                E::Review,
                E::Hint(Hint::NothingHeard),
                E::Announce(A::ReadyToReview { recorded_ms: 3_000 })
            ]
        );
        assert_eq!(s.phase, Phase::Idle);
        assert!(s.undo.is_none());
        // Silence comes before the first release: the lesson waits for a real one.
        let first = Situation {
            first_release: true,
            ..sit()
        };
        let (_, fx) = step(
            holding(false),
            up_with(4_000, 0.0, 0.0, 3_000, false, first),
        );
        assert!(!fx.contains(&E::FirstReleaseDone));
    }

    #[test]
    fn the_first_release_on_a_device_reviews_and_teaches() {
        let first = Situation {
            first_release: true,
            ..sit()
        };
        let (s, fx) = step(holding(false), up_with(4_000, 0.0, 0.0, 2_500, true, first));
        assert_eq!(
            fx,
            vec![
                E::Review,
                E::Hint(Hint::NextTimeSends),
                E::Announce(A::ReadyToReview { recorded_ms: 2_500 }),
                E::FirstReleaseDone
            ]
        );
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(s.guard_until_ms, 4_600);
    }

    #[test]
    fn review_before_sending_and_a_screen_reader_review_without_a_lesson_that_would_be_untrue() {
        for situation in [
            Situation {
                review_before_sending: true,
                ..sit()
            },
            Situation {
                assistive: true,
                ..sit()
            },
            Situation {
                first_release: true,
                review_before_sending: true,
                ..sit()
            },
            Situation {
                first_release: true,
                assistive: true,
                ..sit()
            },
        ] {
            let (s, fx) = step(
                holding(false),
                up_with(4_000, 0.0, 0.0, 2_500, true, situation),
            );
            assert_eq!(fx, reviewed(2_500), "{situation:?}");
            assert!(s.undo.is_none());
        }
    }

    #[test]
    fn otherwise_the_undo_window_opens_and_sends_after_five_seconds() {
        let (s, fx) = step(holding(false), up(4_000, 3_000));
        assert_eq!(fx, vec![E::UndoWindow]);
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(
            s.undo,
            Some(UndoNote {
                until_ms: 9_000,
                recorded_ms: 3_000
            })
        );
        assert_eq!(
            s.recording(),
            Recording::None,
            "the slot shows the microphone"
        );
        let (s, fx) = step(s, tick(8_999));
        assert!(fx.is_empty());
        let (s, fx) = step(s, tick(9_000));
        assert_eq!(fx, undo_sent());
        assert!(s.undo.is_none());
    }

    #[test]
    fn undo_takes_the_note_to_review_until_the_window_has_run_out() {
        let (s, fx) = step(waiting(9_000, 3_000), HoldEvent::Undo { at_ms: 8_999 });
        assert_eq!(
            fx,
            vec![
                E::UndoReview,
                E::Announce(A::ReadyToReview { recorded_ms: 3_000 })
            ]
        );
        assert!(s.undo.is_none());
        // Late: the window ran out on its own clock, and the Undo finds nothing.
        let (s, fx) = step(waiting(9_000, 3_000), HoldEvent::Undo { at_ms: 9_000 });
        assert_eq!(fx, undo_sent());
        assert!(s.undo.is_none());
    }

    #[test]
    fn any_other_action_or_an_interruption_ends_the_window_by_sending() {
        for e in [
            HoldEvent::OtherAction { at_ms: 6_000 },
            HoldEvent::Interruption {
                at_ms: 6_000,
                recorded_ms: 0,
            },
            HoldEvent::Emptied { at_ms: 6_000 },
        ] {
            let (s, fx) = step(waiting(9_000, 3_000), e);
            assert_eq!(fx, undo_sent(), "{e:?}");
            assert!(s.undo.is_none());
        }
        let (s, _) = step(waiting(9_000, 3_000), HoldEvent::Emptied { at_ms: 6_000 });
        assert_eq!(s.guard_until_ms, 6_600);
    }

    /// S1.1: "A change caused by typing, pasting, a suggestion, deleting text
    /// or staging is NEVER guarded". The guard is there for the second half
    /// of a double tap; the person's own change to the composer between two
    /// presses means the next one is a new decision.
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
        assert_eq!(fx, started(false));
        // The same for a touch that goes down after the change.
        let (s, _) = run(
            idle(),
            &[
                HoldEvent::Emptied { at_ms: 1_000 },
                HoldEvent::OtherAction { at_ms: 1_100 },
            ],
        );
        let (_, fx) = run(s, &[down(1_200), up(1_300, 0)]);
        assert_eq!(fx[1], started(false));

        // The Stop square's own staging guards a second press; the person
        // typing after it does not leave the row-5 Send they then press
        // guarded (a port asks `guarded` before such a Send).
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
        let (s, _) = run(idle(), &[down(0), up(100, 0)]);
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
        // Nor while held.
        let (s, _) = step(holding(false), HoldEvent::OtherAction { at_ms: 700 });
        assert_eq!(s.guard_until_ms, 1_100);
    }

    #[test]
    fn the_microphone_during_the_window_sends_the_note_and_records_again() {
        let (s, fx) = step(waiting(9_000, 3_000), activate(6_000, 0));
        let mut expected = undo_sent();
        expected.extend(started(false));
        assert_eq!(fx, expected);
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        assert!(s.undo.is_none());
        // A touch ends the window only once it IS a tap or a hold.
        let (s, fx) = step(waiting(9_000, 3_000), down(6_000));
        assert!(fx.is_empty());
        assert!(s.undo.is_some());
        let (s, fx) = step(s, up_outside(6_200));
        assert!(
            fx.is_empty(),
            "a press lifted outside is nothing, and the window runs on"
        );
        assert!(s.undo.is_some());
        let (_, fx) = run(s, &[down(6_400), up(6_500, 0)]);
        assert_eq!(fx[1], expected);
        let (s, fx) = run(waiting(9_000, 3_000), &[down(6_000), tick(6_500)]);
        let mut held = undo_sent();
        held.extend(started(true));
        assert_eq!(fx[1], held);
        assert!(matches!(s.phase, Phase::Holding { .. }));
    }

    #[test]
    fn the_release_guards_the_microphone_but_not_the_window() {
        let (s, _) = step(holding(false), up(4_000, 3_000));
        let (s, fx) = run(s, &[down(4_300), up(4_400, 0), activate(4_599, 0)]);
        assert!(fx.iter().all(Vec::is_empty), "{fx:?}");
        assert!(s.undo.is_some());
        let (s, fx) = step(s, activate(4_600, 0));
        let mut expected = undo_sent();
        expected.extend(started(false));
        assert_eq!(fx, expected);
        assert_eq!(s.guard_until_ms, 5_200);
    }

    #[test]
    fn the_window_runs_out_in_the_middle_of_a_press() {
        let (s, fx) = run(waiting(9_000, 3_000), &[down(8_800), tick(9_000)]);
        assert_eq!(fx[1], undo_sent());
        assert!(matches!(s.phase, Phase::Pressed { .. }));
        let (_, fx) = step(s, tick(9_300));
        assert_eq!(fx, started(true));
    }

    #[test]
    fn a_system_cancel_locks_a_hold_deletes_an_armed_one_and_parks_in_the_background() {
        let cancel = |background, recorded_ms| HoldEvent::SystemCancel {
            at_ms: 3_000,
            background,
            recorded_ms,
        };
        let (s, fx) = step(holding(false), cancel(false, 2_000));
        assert_eq!(fx, vec![E::Lock, E::Announce(A::RecordingLocked)]);
        assert_eq!(
            s.phase,
            Phase::HandsFree {
                beside_draft: false
            }
        );
        let (s, fx) = step(holding(true), cancel(false, 2_000));
        assert_eq!(fx, deleted_fx());
        assert_eq!(s.phase, Phase::Idle);
        let (_, fx) = step(holding(true), cancel(true, 2_000));
        assert_eq!(fx, deleted_fx(), "armed is a decision, foreground or not");
        let (s, fx) = step(holding(false), cancel(true, 2_000));
        assert_eq!(fx, vec![E::Park]);
        assert_eq!(s.phase, Phase::Idle);
        let (_, fx) = step(holding(false), cancel(true, 999));
        assert_eq!(fx, vec![E::Delete]);
        let (s, fx) = run(idle(), &[down(0), cancel(false, 0)]);
        assert!(fx.iter().all(Vec::is_empty));
        assert_eq!(s.phase, Phase::Idle);
        let (s, fx) = step(hands_free(false), cancel(true, 9_000));
        assert!(fx.is_empty(), "a hands-free recording has no touch to lose");
        assert_eq!(s, hands_free(false));
    }

    #[test]
    fn an_interruption_parks_a_recording_and_deletes_one_under_a_second() {
        let interruption = |recorded_ms| HoldEvent::Interruption {
            at_ms: 3_000,
            recorded_ms,
        };
        for recording in [
            holding(false),
            holding(true),
            hands_free(false),
            hands_free(true),
        ] {
            let (s, fx) = step(recording, interruption(1_000));
            assert_eq!(fx, vec![E::Park], "{recording:?}");
            assert_eq!(s.phase, Phase::Idle);
            let (_, fx) = step(recording, interruption(999));
            assert_eq!(fx, vec![E::Delete]);
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
        let (s, fx) = run(idle(), &[down(0), interruption(0)]);
        assert!(fx.iter().all(Vec::is_empty));
        assert_eq!(s.phase, Phase::Idle);
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
    fn five_minutes_reviews_and_the_later_lift_does_nothing() {
        for recording in [holding(false), hands_free(false), hands_free(true)] {
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
            let (_, fx) = step(s, up(302_000, 300_000));
            assert!(fx.is_empty());
        }
        let (_, fx) = step(idle(), HoldEvent::Cap { at_ms: 1 });
        assert!(fx.is_empty());
    }

    #[test]
    fn stop_reviews_and_under_a_second_is_too_short() {
        for recording in [hands_free(false), hands_free(true), holding(false)] {
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
        let (_, fx) = step(holding(false), delete(400));
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
        assert_eq!(fx, started(false));
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
        assert_eq!(fx, started(false));
        let (s, fx) = step(s, shortcut(100, 100));
        assert_eq!(fx, too_short_fx());
        assert_eq!(s.phase, Phase::Idle);
        let (_, fx) = step(hands_free(false), shortcut(9_000, 8_000));
        assert_eq!(fx, reviewed(8_000));
        let (_, fx) = step(holding(false), shortcut(9_000, 8_000));
        assert_eq!(fx, reviewed(8_000));
    }

    #[test]
    fn a_tap_without_permission_asks_and_allow_records() {
        let not_asked = Situation {
            permission: Permission::NotAsked,
            ..sit()
        };
        let (s, fx) = run(
            idle(),
            &[down(0), up_with(100, 0.0, 0.0, 0, false, not_asked)],
        );
        assert_eq!(fx[1], vec![E::AskPermission]);
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
        assert_eq!(fx, started(false), "the tap meant record");
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
        assert_eq!(fx, started(false));
        assert_eq!(s.phase, Phase::HandsFree { beside_draft: true });
        assert_eq!(s.guard_until_ms, 0);
    }

    #[test]
    fn a_prompt_raised_by_a_hold_never_records() {
        let not_asked = Situation {
            permission: Permission::NotAsked,
            ..sit()
        };
        let (s, fx) = run(idle(), &[down(0), tick_with(500, not_asked)]);
        assert_eq!(fx[1], vec![E::AskPermission]);
        assert_eq!(
            s.phase,
            Phase::AwaitingPermission {
                source: Source::Hold,
                beside_draft: false
            }
        );
        let (s2, fx) = run(
            s,
            &[
                up(900, 0),
                HoldEvent::PermissionAnswer {
                    at_ms: 3_000,
                    granted: true,
                },
            ],
        );
        assert!(fx[0].is_empty(), "the finger's lift is not an answer");
        assert_eq!(fx[1], vec![E::Hint(Hint::CanRecordNow)]);
        assert_eq!(s2.phase, Phase::Idle);
        let (_, fx) = step(
            s,
            HoldEvent::PermissionAnswer {
                at_ms: 3_000,
                granted: false,
            },
        );
        assert_eq!(fx, vec![E::Denied]);
    }

    #[test]
    fn a_denied_microphone_gives_the_notice_from_a_tap_and_at_h() {
        let denied = Situation {
            permission: Permission::Denied,
            ..sit()
        };
        let (s, fx) = step(
            idle(),
            HoldEvent::Activate {
                at_ms: 0,
                situation: denied,
                recorded_ms: 0,
            },
        );
        assert_eq!(fx, vec![E::Denied]);
        assert_eq!(s, idle(), "and nothing is guarded");
        let (s, fx) = run(idle(), &[down(0), tick_with(500, denied), up(700, 0)]);
        assert_eq!(fx[1], vec![E::Denied]);
        assert!(fx[2].is_empty());
        assert_eq!(s.phase, Phase::Idle);
    }

    #[test]
    fn a_dimmed_microphone_explains_from_a_tap_at_h_and_from_the_menu() {
        for reason in [Dimmed::Call, Dimmed::Busy, Dimmed::NotSent] {
            let blocked = Situation {
                blocked: Some(reason),
                // A dimmed row's sentence comes before any prompt.
                permission: Permission::NotAsked,
                ..sit()
            };
            let (s, fx) = run(
                idle(),
                &[down(0), up_with(100, 0.0, 0.0, 0, false, blocked)],
            );
            assert_eq!(fx[1], vec![E::Explain(reason)]);
            assert_eq!(s, idle());
            let (s, fx) = run(idle(), &[down(0), tick_with(500, blocked), up(900, 0)]);
            assert_eq!(fx[1], vec![E::Explain(reason)]);
            assert!(fx[2].is_empty());
            assert_eq!(s.phase, Phase::Idle);
            let (_, fx) = step(
                idle(),
                HoldEvent::Record {
                    at_ms: 0,
                    beside_draft: false,
                    situation: blocked,
                    recorded_ms: 0,
                },
            );
            assert_eq!(fx, vec![E::Explain(reason)]);
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
        let pressed = step(idle(), down(0)).0;
        let cases: Vec<(HoldState, HoldEvent)> = vec![
            (idle(), up(10, 0)),
            (idle(), mv(10, -200.0, -200.0)),
            (
                idle(),
                HoldEvent::SystemCancel {
                    at_ms: 10,
                    background: true,
                    recorded_ms: 5_000,
                },
            ),
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
            (idle(), HoldEvent::Undo { at_ms: 10 }),
            (idle(), HoldEvent::OtherAction { at_ms: 10 }),
            (idle(), tick(10_000)),
            (pressed, activate(100, 0)),
            (pressed, down(100)),
            (
                pressed,
                HoldEvent::Record {
                    at_ms: 100,
                    beside_draft: false,
                    situation: sit(),
                    recorded_ms: 0,
                },
            ),
            (holding(false), down(2_000)),
            (holding(false), activate(2_000, 1_500)),
            (holding(false), tick(9_000)),
            (hands_free(false), down(2_000)),
            (hands_free(false), mv(2_000, 0.0, -300.0)),
            (hands_free(false), up(2_000, 1_500)),
            (hands_free(false), tick(9_000)),
            (asking, activate(20_000, 0)),
            (asking, down(20_000)),
            (
                asking,
                HoldEvent::Stop {
                    at_ms: 20_000,
                    recorded_ms: 12_000,
                },
            ),
            (prompting, activate(20_000, 0)),
            (prompting, up(20_000, 0)),
            (
                prompting,
                HoldEvent::Answer {
                    at_ms: 20_000,
                    delete: true,
                },
            ),
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
        assert_eq!(step(idle(), down(0)).0.recording(), Recording::None);
        assert_eq!(holding(true).recording(), Recording::Held);
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

    #[test]
    fn the_clocks_never_overflow() {
        let late = u64::MAX - 10;
        let (s, _) = run(idle(), &[down(late - 600), tick(late)]);
        assert_eq!(s.guard_until_ms, u64::MAX);
        let (s, fx) = step(s, up(late, 2_000));
        assert_eq!(fx, vec![E::UndoWindow]);
        assert_eq!(s.undo.map(|note| note.until_ms), Some(u64::MAX));
    }
}
