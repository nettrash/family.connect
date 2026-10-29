//! The call media's stand-in (W.3: "`CallEngine` constructed with a NO-OP `CallMedia` (P6 replaces
//! it)"). P6's `calls::GstCallMedia` implements fc-logic's `CallMedia` over `webrtcbin` — its
//! contract is `fc_logic::calls`'s module documentation, "The webrtcbin `CallMedia`" — and installs
//! it in `Shell::call_media` before any server is opened; the window's ONE media object is shared
//! by every connection's `CallEngine` (Windows keeps its page across servers).
//!
//! [`NoCallMedia`] grants nothing, so the engine refuses to place or answer (the microphone is
//! "denied") and an incoming call can still be declined — the protocol stays whole without media.

use fc_core::protocol::{IceCandidate, IceServerDto};
use fc_core::rt::{Event, LocalBoxFuture};
use fc_logic::calls::{CallMedia, MediaGrant, RingTone};

/// fc-logic's `CallMedia` with no media at all.
#[derive(Debug, Default)]
pub struct NoCallMedia {
    candidate: Event<IceCandidate>,
    connected: Event<()>,
    failed: Event<()>,
    media_changed: Event<()>,
}

impl CallMedia for NoCallMedia {
    fn open(&self, _video: bool) -> LocalBoxFuture<'static, MediaGrant> {
        Box::pin(std::future::ready(MediaGrant::None))
    }

    fn connect(
        &self,
        _servers: Vec<IceServerDto>,
        _receive_video_only: bool,
    ) -> LocalBoxFuture<'static, bool> {
        Box::pin(std::future::ready(false))
    }

    fn create_offer(&self) -> LocalBoxFuture<'static, Option<String>> {
        Box::pin(std::future::ready(None))
    }

    fn set_remote(&self, _is_offer: bool, _sdp: String) -> LocalBoxFuture<'static, bool> {
        Box::pin(std::future::ready(false))
    }

    fn create_answer(&self) -> LocalBoxFuture<'static, Option<String>> {
        Box::pin(std::future::ready(None))
    }

    fn add_candidate(&self, _candidate: IceCandidate) -> LocalBoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }

    fn is_connected(&self) -> bool {
        false
    }

    fn set_muted(&self, _muted: bool) {}

    fn set_camera(&self, _on: bool) {}

    fn ring(&self, _tone: RingTone) {}

    fn stop_ringing(&self) {}

    fn close(&self) {}

    fn on_candidate_gathered(&self) -> &Event<IceCandidate> {
        &self.candidate
    }

    fn on_connected(&self) -> &Event<()> {
        &self.connected
    }

    fn on_failed(&self) -> &Event<()> {
        &self.failed
    }

    fn on_media_changed(&self) -> &Event<()> {
        &self.media_changed
    }
}
