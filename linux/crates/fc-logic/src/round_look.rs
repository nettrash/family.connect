//! How a VIDEO MESSAGE is drawn on this client (docs/audio-video-messages-2026-10-04.md, S5.2,
//! S5.3, S6) — `RoundLook.cs`: a drawing rule, not a wire one — the circle's size, what is written
//! on it, what a screen reader hears, and the size of the circle the viewer plays it in; how the
//! video-message recorder's one big slot is drawn ([`recorder_look`]); and what is playing and
//! what pauses it ([`Playing`], [`PlaybackEvent`], [`playback_pauses`]).
//!
//! **NO BALLOON, AND ONE SIZE.** The circle alone on the chat background, like a sticker — 240 on
//! Linux, whose desktop is never a compact width (S5.2; `fc_text::record::round_diameter` at
//! `WidthClass::Regular`): larger than a sticker (160), smaller than a video tile. The square
//! poster fills it, and until it lands a neutral disc of the same size holds the row's height.
//!
//! **A CLICK PLAYS IT IN PLACE** (S5.3; 2026-10-06 — [`round_inline`](crate::round_playback::round_inline),
//! [`RoundPlayback`](crate::round_playback::RoundPlayback)): inside its own circle, at the same
//! size, with sound. **OPEN FULL SCREEN PLAYS IT IN THE VIEWER** (S5.4): the square clip with its
//! corners painted in the viewer's solid backdrop, exactly one accent ring OUTSIDE the edge running
//! round as it plays (the approved design of 2026-10-05), and a play disc that fades out while it
//! does.

use fc_core::protocol::AttachmentDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_text::media::time_label;

/// The circle in a conversation, in logical pixels (S5.2: the desktop is never compact).
pub const DIAMETER: f64 = fc_text::record::ROUND_DIAMETER_REGULAR as f64;

/// The play disc in the middle of the circle: the approved design's 48 (the 44 target the circle
/// itself exceeds).
pub const PLAY_DISC: f64 = 48.0;

/// The accent dot beside the duration while this device has not played it.
pub const DOT: f64 = 8.0;

/// The ring outside the edge: the accent as it plays — in place or in the viewer — a neutral one
/// while one of the reader's own uploads.
pub const RING: f64 = 3.0;

/// The largest circle the viewer draws: the recording's own 480 pixels.
pub const VIEWER_LARGEST: f64 = 480.0;

/// The smallest circle the viewer draws, however small the window: the conversation's own size and
/// no less.
pub const VIEWER_SMALLEST: f64 = 200.0;

/// How near the end a clip counts as played through: the viewer clock's own tick, and Play's
/// restart rule.
pub const END_SLACK_SECONDS: f64 = 0.25;

/// "0:23", the capsule at the bottom of the circle — or nothing when the sender did not say how
/// long.
pub fn capsule(video: &AttachmentDto) -> Option<String> {
    match video.duration_ms {
        Some(ms) if ms > 0 => Some(time_label(f64::from(ms) / 1000.0)),
        _ => None,
    }
}

/// What a screen reader hears for the circle: "Video message, 0:23" (S6).
pub fn name(video: &AttachmentDto, say: &dyn StringCatalog) -> String {
    match capsule(video) {
        Some(length) => say.format("Video message, %@", &[&length]),
        None => say.get("Video message").to_string(),
    }
}

/// Whether the accent dot shows (S5.2): only on SOMEONE ELSE'S circle that this device has not
/// played — never on the reader's own, which no other client marks either.
pub fn shows_dot(mine: bool, played: bool) -> bool {
    !mine && !played
}

/// Its value: "Not played" while the dot shows and "Played" once this device has played it — on
/// someone else's circle only; the reader's own say neither (S6), as a voice message's do not
/// ([`voice_look::status`](crate::voice_look::status)).
pub fn value(mine: bool, played: bool, say: &dyn StringCatalog) -> Option<String> {
    if mine {
        None
    } else if played {
        Some(say.get("Played").to_string())
    } else {
        Some(say.get("Not played").to_string())
    }
}

/// Whether this opening has PLAYED IT THROUGH, so its dot goes (S5.3: "at the end it returns to the
/// poster and loses its dot") — it was seen playing here and its position has reached the end.
/// Starting it is not enough, and nor is a seek to the end of a clip never played; a clip of
/// unknown length (`total` 0 or less) is never through. `position` and `total` are in seconds.
pub fn played_through(seen_playing: bool, position: f64, total: f64) -> bool {
    seen_playing
        && total.is_finite()
        && total > 0.0
        && position.is_finite()
        && position >= total - END_SLACK_SECONDS
}

/// How far round the viewer's ONE accent ring is drawn, 0 to 1: nothing before this opening has
/// seen it play, as far as it has played while it plays or is paused part way — and NOTHING once
/// it has finished, when the circle is back to its poster and play disc (the approved design hides
/// the ring at the end rather than leaving a full one round it). `total` 0 or less is unknown.
pub fn viewer_ring(seen_playing: bool, playing: bool, position: f64, total: f64) -> f64 {
    if !seen_playing || !total.is_finite() || total <= 0.0 || !position.is_finite() {
        return 0.0;
    }
    if !playing && position >= total - END_SLACK_SECONDS {
        return 0.0;
    }
    (position / total).clamp(0.0, 1.0)
}

/// The circle the viewer plays it in: as large as the room left by the viewer's own bars allows,
/// never larger than the recording's 480 pixels and never smaller than the conversation's circle —
/// a window too small for even that scrolls nothing and simply clips, as the viewer's video always
/// has. `chrome` is what the bars above and below the circle take of the height.
///
/// A `chrome` that is not a number counts as none (the C# let it through `Math.Max` and drew a
/// circle of NaN).
pub fn viewer_diameter(width: f64, height: f64, chrome: f64) -> f64 {
    const MARGIN: f64 = 24.0;
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return VIEWER_LARGEST;
    }
    // `f64::max` ignores a NaN, which is the fix described above; an infinite chrome leaves no
    // room, as in the C#.
    let room = (width - 2.0 * MARGIN).min(height - chrome.max(0.0) - 2.0 * MARGIN);
    room.clamp(VIEWER_SMALLEST, VIEWER_LARGEST).floor()
}

/// The shape the recorder's one big slot button takes (the approved design: Record → Stop → Send).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotShape {
    /// A plain red disc: Record.
    RecordDisc,
    /// A red disc with a white rounded square on it: Stop.
    StopSquare,
    /// An accent disc with the Send arrow: Send.
    SendArrow,
}

/// How the video-message recorder is drawn (the approved design of 2026-10-05, "Recording a video
/// message"): ONE big slot button where Send is, smaller round buttons with captions beside it, and
/// the ring outside the circle — a thin track all the way round, red filling over the minute while
/// it records, the accent while the clip plays back in REVIEW. Sizes in logical pixels.
pub mod recorder_look {
    use fc_core::text::string_catalog::StringCatalog;

    use super::SlotShape;
    use crate::round_recorder::RecorderStage;

    /// The big slot button: 64 across, with a faint 4-wide halo round it.
    pub const SLOT: f64 = 64.0;

    /// The halo round the slot.
    pub const HALO: f64 = 4.0;

    /// Stop's white rounded square on the red disc.
    pub const STOP_SQUARE: f64 = 22.0;

    /// The corner radius of Stop's square.
    pub const STOP_CORNER: f64 = 5.0;

    /// The smaller round buttons beside the slot — Close or Delete, Switch camera or Retake.
    pub const SIDE: f64 = 44.0;

    /// The thin track ring's stroke, drawn where the progress ring runs.
    pub const TRACK: f64 = 1.0;

    /// The slot's target: the disc and its halo.
    pub const SLOT_TARGET: f64 = SLOT + 2.0 * HALO;

    /// The gap between a round button and its caption.
    pub const CAPTION_GAP: f64 = 2.0;

    /// The slot's shape in each stage: Stop while recording, Send in review, Record otherwise.
    pub fn shape(stage: RecorderStage) -> SlotShape {
        match stage {
            RecorderStage::Recording => SlotShape::StopSquare,
            RecorderStage::Review => SlotShape::SendArrow,
            _ => SlotShape::RecordDisc,
        }
    }

    /// The caption under the slot: "Record", "Stop" or "Send".
    pub fn caption(stage: RecorderStage, say: &dyn StringCatalog) -> String {
        match shape(stage) {
            SlotShape::StopSquare => say.get("Stop").to_string(),
            SlotShape::SendArrow => say.get("Send").to_string(),
            SlotShape::RecordDisc => say.get("Record").to_string(),
        }
    }

    /// Whether the thin track ring is drawn: wherever the circle shows a picture or a clip.
    pub fn shows_track(stage: RecorderStage) -> bool {
        matches!(
            stage,
            RecorderStage::Preview | RecorderStage::Recording | RecorderStage::Review
        )
    }
}

/// What is playing through this client's shared player, or its viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Playing {
    /// A voice note or an audio file in a bubble, or a staged or not-sent note.
    VoiceNote,
    /// A video message — in place, in its own circle (S5.3), or in the viewer's circle (S5.4).
    RoundVideo,
    /// An ordinary video in the viewer.
    Video,
}

/// Something that happens to the app while something plays (S4's last column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaybackEvent {
    /// A call rings, starts or is placed — any phase but idle.
    Call,
    /// The session locks, the screen saver starts, or the computer goes to sleep.
    SessionLocked,
    /// The window is minimised, or hidden to the top bar by its close button with Keep running on.
    WindowHidden,
    /// The default output device changed — headphones unplugged, a Bluetooth headset gone or come.
    OutputChanged,
    /// The window only lost focus and is still visible.
    FocusLost,
}

/// Which interruptions pause what plays (docs/audio-video-messages-2026-10-04.md, S4, "Playing (a
/// voice note or a circle)"; S8.6): a call, a session lock and a change of the default output
/// device pause a voice note and a circle; a hidden window pauses a circle and lets a voice note
/// play on; losing focus pauses nothing.
///
/// An ORDINARY video in the viewer is not in S4's column and is left as it always was. Leaving the
/// chat, a real close and a sign-out STOP everything, which the view already does on its own.
///
/// On Linux the shell hears these as shell-design §10.8 lists them (the session lock, logind's
/// `PrepareForSleep`, the window's `suspended`, a call's phase) and the default output device
/// changing through PipeWire/PulseAudio's default-sink change; it keeps ONE process-wide
/// "now playing" registry (§10.1) and asks [`playback_pauses::pauses`] for whatever is in it.
pub mod playback_pauses {
    use super::{PlaybackEvent, Playing};
    use crate::not_sent::RecordingEnd;

    /// Whether `happened` pauses `playing`.
    pub fn pauses(happened: PlaybackEvent, playing: Playing) -> bool {
        playing != Playing::Video
            && match happened {
                PlaybackEvent::Call
                | PlaybackEvent::SessionLocked
                | PlaybackEvent::OutputChanged => true,
                // A circle is something one looks at; a voice note is heard from anywhere (S4's
                // minimised row).
                PlaybackEvent::WindowHidden => playing == Playing::RoundVideo,
                PlaybackEvent::FocusLost => false,
            }
    }

    /// The playback side of what interrupts a recording — the one place the window hears a call, a
    /// lock or a hidden window — or `None` where the view already STOPS everything (leaving the
    /// chat, closing, signing out) or the end was the person's own.
    pub fn of(why: RecordingEnd) -> Option<PlaybackEvent> {
        match why {
            RecordingEnd::Call => Some(PlaybackEvent::Call),
            RecordingEnd::SessionLocked => Some(PlaybackEvent::SessionLocked),
            RecordingEnd::WindowHidden => Some(PlaybackEvent::WindowHidden),
            _ => None,
        }
    }
}
