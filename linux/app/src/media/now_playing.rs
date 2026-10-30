//! ONE THING PLAYS AT A TIME (shell-design §10.1; `PlaybackPauses`, `Playing`): the process-wide
//! [`NowPlaying`] registry. A voice note starting pauses a circle and the viewer's video and the
//! other way round; an interruption pauses what `playback_pauses::pauses` says; a recording
//! quiets everything (S1.7) — [`Registry::quiet`] — and while one runs or starts nothing new may
//! begin ([`Registry::refuses`]).
//!
//! Pure: no GTK, no GStreamer — the players hand in their own `pause`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_logic::round_look::{PlaybackEvent, Playing, playback_pauses};

use crate::seams::media::NowPlaying;

/// The claim held now.
struct Held {
    id: u64,
    kind: Playing,
    pause: Rc<dyn Fn()>,
}

/// The registry installed in `Shell::now_playing`.
#[derive(Default)]
pub struct Registry {
    held: RefCell<Option<Held>>,
    next: Cell<u64>,
    /// A voice or video message records, or is being started: no app sound (S1.7).
    recording: Cell<bool>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("playing", &self.playing())
            .field("recording", &self.recording.get())
            .finish()
    }
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    /// A recording begins (or is being started) or ends: while it runs, whatever plays is paused
    /// and nothing new may start — "You can play this after recording."
    pub fn set_recording(&self, recording: bool) {
        self.recording.set(recording);
        if recording {
            self.quiet();
        }
    }

    /// Whether playback is refused now: a recording runs or is being started (S1.7).
    pub fn refuses(&self) -> bool {
        self.recording.get()
    }

    /// Pause whatever holds the claim, whatever it is.
    pub fn quiet(&self) {
        let held = self.held.borrow_mut().take();
        if let Some(held) = held {
            (held.pause)();
        }
    }
}

impl NowPlaying for Registry {
    fn claim(&self, kind: Playing, pause: Rc<dyn Fn()>) -> u64 {
        let id = self.next.get().wrapping_add(1);
        self.next.set(id);
        // Replaced BEFORE the old holder is paused, so its pause (which may release its claim)
        // finds the claim already taken over and does nothing.
        let previous = self.held.borrow_mut().replace(Held { id, kind, pause });
        if let Some(previous) = previous {
            (previous.pause)();
        }
        id
    }

    fn release(&self, claim: u64) {
        let mut held = self.held.borrow_mut();
        if held.as_ref().is_some_and(|held| held.id == claim) {
            *held = None;
        }
    }

    fn interrupt(&self, event: PlaybackEvent) {
        let paused = {
            let mut held = self.held.borrow_mut();
            match held.as_ref() {
                Some(holder) if playback_pauses::pauses(event, holder.kind) => held.take(),
                _ => None,
            }
        };
        if let Some(paused) = paused {
            (paused.pause)();
        }
    }

    fn playing(&self) -> Option<Playing> {
        self.held.borrow().as_ref().map(|held| held.kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> (Rc<Cell<u32>>, Rc<dyn Fn()>) {
        let count = Rc::new(Cell::new(0));
        let seen = Rc::clone(&count);
        (count, Rc::new(move || seen.set(seen.get() + 1)))
    }

    #[test]
    fn a_new_claim_pauses_the_one_before_and_never_itself() {
        let registry = Registry::new();
        let (voice, pause_voice) = counter();
        let (circle, pause_circle) = counter();
        let first = registry.claim(Playing::VoiceNote, pause_voice);
        assert_eq!(registry.playing(), Some(Playing::VoiceNote));
        let second = registry.claim(Playing::RoundVideo, pause_circle);
        assert_ne!(first, second);
        assert_eq!(voice.get(), 1);
        assert_eq!(circle.get(), 0);
        assert_eq!(registry.playing(), Some(Playing::RoundVideo));
        // The old claim's release is stale and changes nothing.
        registry.release(first);
        assert_eq!(registry.playing(), Some(Playing::RoundVideo));
        registry.release(second);
        assert_eq!(registry.playing(), None);
    }

    #[test]
    fn a_pause_that_releases_its_own_claim_does_not_undo_the_new_one() {
        let registry = Rc::new(Registry::new());
        let id = Rc::new(Cell::new(0u64));
        let (me, held) = (Rc::downgrade(&registry), Rc::clone(&id));
        let first = registry.claim(
            Playing::VoiceNote,
            Rc::new(move || {
                if let Some(registry) = me.upgrade() {
                    registry.release(held.get());
                }
            }),
        );
        id.set(first);
        registry.claim(Playing::Video, Rc::new(|| {}));
        assert_eq!(registry.playing(), Some(Playing::Video));
    }

    #[test]
    fn an_interruption_pauses_what_s4_says() {
        let registry = Registry::new();
        let (voice, pause) = counter();
        registry.claim(Playing::VoiceNote, pause);
        // A hidden window lets a voice note play on (S4's minimised row) …
        registry.interrupt(PlaybackEvent::WindowHidden);
        assert_eq!(voice.get(), 0);
        assert_eq!(registry.playing(), Some(Playing::VoiceNote));
        // … a lock does not.
        registry.interrupt(PlaybackEvent::SessionLocked);
        assert_eq!(voice.get(), 1);
        assert_eq!(registry.playing(), None);
        let (circle, pause) = counter();
        registry.claim(Playing::RoundVideo, pause);
        registry.interrupt(PlaybackEvent::WindowHidden);
        assert_eq!(circle.get(), 1);
        // An ordinary video is left as it always was.
        let (video, pause) = counter();
        registry.claim(Playing::Video, pause);
        registry.interrupt(PlaybackEvent::Call);
        assert_eq!(video.get(), 0);
    }

    #[test]
    fn a_recording_quiets_everything_and_refuses_until_it_ends() {
        let registry = Registry::new();
        let (video, pause) = counter();
        registry.claim(Playing::Video, pause);
        registry.set_recording(true);
        assert_eq!(video.get(), 1);
        assert!(registry.refuses());
        assert_eq!(registry.playing(), None);
        registry.set_recording(false);
        assert!(!registry.refuses());
    }
}
