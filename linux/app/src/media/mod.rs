//! P3 — MEDIA (shell-design §10, §8.4's encoder, §22; W.2): everything that plays, records or
//! transcodes, on GStreamer 1.28.
//!
//! | Module | What | Implements |
//! | --- | --- | --- |
//! | [`player`] | `gst_play` + `gtk4paintablesink`, authorised streams, rate with pitch | `Players`/`Player` (W.3), fc-logic's `RoundPlayers` |
//! | [`now_playing`] | one thing plays at a time; a recording quiets everything | `NowPlaying` (W.3) |
//! | [`voice_bubble`], [`waves`] | the voice note in a bubble: waveform, seek, speed, dot | `ConversationSlots::voice` |
//! | [`circle`] | a received video message in place, clipped round | `ConversationSlots::circle` |
//! | [`voice_recorder`] | the microphone into an M4A with a live meter; the sender's waveform | fc-logic's `VoiceRecorder`, `VoiceRecording`, `VoiceMeter`, `VoiceDecoder` |
//! | [`recording`], `composer::recording_row` | recording inside P1's composer: the row, not-sent rows, parking | `ComposerSlots`, `ComposerRecording` (W.3) |
//! | [`camera`] | the Camera portal + PipeWire, then PipeWire's devices, then `v4l2src` | `Camera` (W.3) — P6 opens it for calls too |
//! | [`video_recorder`], [`recorder_layer`] | the video message recorder over the window | fc-logic's `VideoMessageRecorder`; the overlay layer (§10.6) |
//! | [`preparing`] | picked videos and sounds brought to the profile; read back; cancel | fc-logic's `MediaPreparing` |
//! | [`animation`] | animated pictures into the silent looping MP4 (no faststart) | fc-logic's `StillDecoder`, `AnimationEncoder` |
//! | [`session_watch`] | lock, sleep, minimise, a call → interruptions (the screen is kept on while recording through P5's `desktop::keep_awake`) | §10.8, §10.9 |
//! | [`pipelines`] | every GStreamer description, pure and tested | — |
//!
//! **The self-test's sources** (§27): Docker has no microphone and no camera, so a `selftest` build
//! may swap `pulsesrc`/the camera for `audiotestsrc`/`videotestsrc` — when the self-test asks
//! ([`use_test_sources`]) or `FC_TEST_AUDIO_SOURCE` is set. A build without the feature has no way
//! to: a shipped app never records a test tone ([`test_sources`]).

pub mod animation;
pub mod camera;
pub mod circle;
pub mod gst_util;
pub mod now_playing;
pub mod pipelines;
pub mod player;
pub mod preparing;
pub mod recorder_layer;
pub mod recording;
pub mod session_watch;
pub mod video_recorder;
pub mod voice_bubble;
pub mod voice_recorder;
pub mod waves;

use std::cell::RefCell;
use std::rc::Rc;

use fc_core::protocol::{AttachmentDto, MessageDto};

use crate::seams::conversation::ConversationSlots;
use crate::shell::Shell;

thread_local! {
    static NOW_PLAYING: Rc<now_playing::Registry> = Rc::new(now_playing::Registry::new());
    static CAMERA: RefCell<Option<std::rc::Weak<camera::GstCamera>>> = const { RefCell::new(None) };
}

/// The installed camera, concretely (its portal reading; its `open` reaches itself through here).
pub fn installed_camera() -> Option<Rc<camera::GstCamera>> {
    CAMERA.with(|slot| slot.borrow().as_ref().and_then(std::rc::Weak::upgrade))
}

#[cfg(feature = "selftest")]
thread_local! {
    static TEST_SOURCES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the test sources stand in for the microphone and the camera: only ever in a `selftest`
/// build (see the module's comment).
pub fn test_sources() -> bool {
    #[cfg(feature = "selftest")]
    {
        TEST_SOURCES.with(std::cell::Cell::get)
            || std::env::var_os("FC_TEST_AUDIO_SOURCE").is_some_and(|value| !value.is_empty())
    }
    #[cfg(not(feature = "selftest"))]
    {
        false
    }
}

/// The self-test turns the test sources on (or off).
#[cfg(feature = "selftest")]
pub fn use_test_sources(on: bool) {
    TEST_SOURCES.with(|cell| cell.set(on));
}

/// The microphone's description: `pulsesrc`, or the self-test's tone.
pub fn microphone() -> &'static str {
    if test_sources() {
        pipelines::TEST_TONE
    } else {
        pipelines::MICROPHONE
    }
}

/// The process's one-thing-plays-at-a-time registry (also in `Shell::now_playing`).
pub fn now_playing() -> Rc<now_playing::Registry> {
    NOW_PLAYING.with(Rc::clone)
}

/// `app::install_packages`' line for P3: fills the shell's slots, the recorder layer, the CSS, the
/// session watch and the parking on quit.
pub fn install(shell: &Rc<Shell>) {
    let registry = now_playing();
    shell.players.install(Rc::new(player::GstPlayers));
    shell.now_playing.install(registry);
    let camera = camera::GstCamera::new();
    camera::remember(&camera);
    {
        let camera = Rc::downgrade(&camera);
        shell.on_quitting().subscribe(move |_| {
            if let Some(camera) = camera.upgrade() {
                camera.stop_monitor();
            }
        });
    }
    shell.camera.install(camera);
    let slots = recording::MediaComposerSlots::new(shell);
    shell.composer_slots.install(slots.clone());
    shell.media_slots.install(Rc::new(MediaSlots));
    shell.add_window_part(Rc::new(recorder_layer::RecorderPart));
    // A take of a video message is never lost to a quit without the question (§10.6).
    shell
        .close_guard
        .install(Rc::new(recorder_layer::may_close));
    crate::theme::register(css);
    session_watch::start(shell);
    recording::follow_the_session(shell, &slots);
}

/// P3's half of the bubble's slots: voice notes and circles.
#[derive(Debug, Default)]
pub struct MediaSlots;

impl ConversationSlots for MediaSlots {
    fn voice(&self, message: &MessageDto, attachment: &AttachmentDto, own: bool) -> gtk::Widget {
        voice_bubble::widget(message, attachment, own)
    }

    fn circle(&self, message: &MessageDto, attachment: &AttachmentDto) -> gtk::Widget {
        circle::widget(message, attachment)
    }
}

/// The media package's CSS (prefixed `fc-media-`). Words in a warning or an error take
/// libadwaita's semantic text colours (clamped for each scheme and for high contrast); the red
/// FILLS (the dot, the Record and Stop discs) keep the recording red the meter is drawn in. What
/// lies over a picture speaks the tiles' language: a rimmed disc, pill-shaped chips.
fn css(palette: &crate::theme::palette::Palette) -> String {
    let accent = palette.accent_bg.hex();
    let ink = palette.accent_fg.hex();
    format!(
        "button.fc-media-play {{
  background-color: {accent};
  color: {ink};
  min-width: 40px;
  min-height: 40px;
  padding: 0;
}}
button.fc-media-play.fc-media-on-accent {{
  background-color: {ink};
  color: {accent};
}}
.fc-media-dot {{
  background-color: {accent};
  border-radius: 9999px;
  min-width: 7px;
  min-height: 7px;
}}
.fc-media-time {{
  font-feature-settings: \"tnum\";
}}
button.fc-media-speed {{
  min-height: 20px;
  padding: 0 8px;
}}
.fc-media-recording-dot {{
  background-color: #e01b24;
  border-radius: 9999px;
  min-width: 10px;
  min-height: 10px;
}}
.fc-media-warning {{
  color: var(--warning-color);
}}
.fc-media-chip {{
  border-radius: 12px;
  padding: 6px 10px;
}}
.fc-media-circle-disc {{
  background-color: rgba(0, 0, 0, 0.5);
  color: white;
  border-radius: 9999px;
  min-width: 48px;
  min-height: 48px;
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.22), 0 1px 4px rgba(0, 0, 0, 0.3);
}}
.fc-media-capsule {{
  background-color: rgba(0, 0, 0, 0.55);
  color: white;
  border-radius: 9999px;
  padding: 1px 7px;
  font-feature-settings: \"tnum\";
}}
.fc-media-circle .fc-round {{
  box-shadow: 0 0 0 1px alpha(currentColor, 0.08), 0 1px 3px rgba(0, 0, 0, 0.15);
}}
.fc-media-circle .fc-round:focus-visible {{
  outline: 2px solid color-mix(in srgb, var(--accent-color) 60%, transparent);
  outline-offset: 1px;
}}
.fc-media-circle .fc-media-dot {{
  box-shadow: 0 0 0 2px rgba(255, 255, 255, 0.9);
}}
.fc-media-poster {{
  background-color: alpha(currentColor, 0.12);
}}
.fc-media-layer {{
  background-color: var(--window-bg-color);
}}
.fc-media-status {{
  border-radius: 9999px;
  padding: 4px 14px;
  background-color: alpha(currentColor, 0.08);
}}
.fc-media-status.fc-media-red {{
  color: var(--error-color);
}}
.fc-media-bar {{
  background-color: var(--headerbar-bg-color);
  box-shadow: 0 -1px color-mix(in srgb, var(--headerbar-shade-color) 50%, transparent),
    0 -2px 4px color-mix(in srgb, var(--headerbar-shade-color) 50%, transparent);
  padding: 12px;
}}
@media (prefers-contrast: more) {{
  .fc-media-bar {{
    box-shadow: 0 -1px color-mix(in srgb, currentColor 50%, transparent);
  }}
}}
button.fc-media-slot {{
  min-width: 64px;
  min-height: 64px;
  border-radius: 9999px;
  padding: 0;
}}
button.fc-media-slot.fc-media-record,
button.fc-media-slot.fc-media-stop {{
  background-color: #e01b24;
  color: white;
}}
button.fc-media-slot.fc-media-send {{
  background-color: {accent};
  color: {ink};
}}
"
    )
}
