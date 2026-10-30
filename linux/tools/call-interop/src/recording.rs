//! A `CallMedia` that is `GstCallMedia` with a notebook: every call the engine makes, every
//! event the media emits, what each SDP negotiated — and any event that came after `close`
//! (the contract allows none).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use fc_core::protocol::{IceCandidate, IceServerDto};
use fc_core::rt::{Event, LocalBoxFuture};
use fc_logic::calls::{CallMedia, MediaGrant, RingTone};

use crate::calls::GstCallMedia;
use crate::sdp_summary::summary;

/// What the notebook holds.
#[derive(Debug, Default)]
pub struct Notes {
    /// `(seconds since start, what)`.
    pub calls: Vec<(f64, String)>,
    pub events: Vec<(f64, String)>,
    pub events_after_close: u32,
    pub closed: bool,
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub grant: Option<MediaGrant>,
}

struct Shared {
    start: Instant,
    notes: RefCell<Notes>,
    gathered: Event<IceCandidate>,
    connected: Event<()>,
    failed: Event<()>,
    changed: Event<()>,
}

impl Shared {
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn call(&self, what: impl Into<String>) {
        let now = self.now();
        self.notes.borrow_mut().calls.push((now, what.into()));
    }

    fn event(&self, what: &str) {
        let now = self.now();
        let mut notes = self.notes.borrow_mut();
        if notes.closed {
            notes.events_after_close += 1;
        }
        notes.events.push((now, what.to_string()));
    }
}

pub struct RecordingMedia {
    media: Rc<GstCallMedia>,
    shared: Rc<Shared>,
    candidates: Cell<u32>,
}

impl RecordingMedia {
    pub fn new(media: Rc<GstCallMedia>, start: Instant) -> Self {
        let shared = Rc::new(Shared {
            start,
            notes: RefCell::new(Notes::default()),
            gathered: Event::new(),
            connected: Event::new(),
            failed: Event::new(),
            changed: Event::new(),
        });
        {
            let shared = shared.clone();
            media.on_candidate_gathered().subscribe(move |candidate| {
                shared.event("candidate");
                shared.gathered.emit(candidate);
            });
        }
        {
            let shared = shared.clone();
            media.on_connected().subscribe(move |_| {
                shared.event("connected");
                shared.connected.emit(&());
            });
        }
        {
            let shared = shared.clone();
            media.on_failed().subscribe(move |_| {
                shared.event("failed");
                shared.failed.emit(&());
            });
        }
        {
            let shared = shared.clone();
            media.on_media_changed().subscribe(move |_| {
                shared.event("media_changed");
                shared.changed.emit(&());
            });
        }
        RecordingMedia {
            media,
            shared,
            candidates: Cell::new(0),
        }
    }

    pub fn notes(&self) -> std::cell::Ref<'_, Notes> {
        self.shared.notes.borrow()
    }

    pub fn candidates_added(&self) -> u32 {
        self.candidates.get()
    }
}

impl CallMedia for RecordingMedia {
    fn open(&self, video: bool) -> LocalBoxFuture<'static, MediaGrant> {
        self.shared.call(format!("open(video={video})"));
        self.shared.notes.borrow_mut().closed = false;
        let opening = self.media.open(video);
        let shared = self.shared.clone();
        Box::pin(async move {
            let grant = opening.await;
            shared.call(format!("open → {grant:?}"));
            shared.notes.borrow_mut().grant = Some(grant);
            grant
        })
    }

    fn connect(
        &self,
        servers: Vec<IceServerDto>,
        receive_video_only: bool,
    ) -> LocalBoxFuture<'static, bool> {
        self.shared.call(format!(
            "connect(servers={}, receive_video_only={receive_video_only})",
            servers.len()
        ));
        let connecting = self.media.connect(servers, receive_video_only);
        let shared = self.shared.clone();
        Box::pin(async move {
            let connected = connecting.await;
            shared.call(format!("connect → {connected}"));
            connected
        })
    }

    fn create_offer(&self) -> LocalBoxFuture<'static, Option<String>> {
        self.shared.call("create_offer");
        let offering = self.media.create_offer();
        let shared = self.shared.clone();
        Box::pin(async move {
            let sdp = offering.await;
            shared.call(format!("create_offer → {}", sdp.is_some()));
            shared.notes.borrow_mut().local_sdp = sdp.as_deref().map(summary);
            sdp
        })
    }

    fn set_remote(&self, is_offer: bool, sdp: String) -> LocalBoxFuture<'static, bool> {
        self.shared.call(format!("set_remote(offer={is_offer})"));
        self.shared.notes.borrow_mut().remote_sdp = Some(summary(&sdp));
        let setting = self.media.set_remote(is_offer, sdp);
        let shared = self.shared.clone();
        Box::pin(async move {
            let set = setting.await;
            shared.call(format!("set_remote → {set}"));
            set
        })
    }

    fn create_answer(&self) -> LocalBoxFuture<'static, Option<String>> {
        self.shared.call("create_answer");
        let answering = self.media.create_answer();
        let shared = self.shared.clone();
        Box::pin(async move {
            let sdp = answering.await;
            shared.call(format!("create_answer → {}", sdp.is_some()));
            shared.notes.borrow_mut().local_sdp = sdp.as_deref().map(summary);
            sdp
        })
    }

    fn add_candidate(&self, candidate: IceCandidate) -> LocalBoxFuture<'static, ()> {
        self.candidates.set(self.candidates.get() + 1);
        self.media.add_candidate(candidate)
    }

    fn is_connected(&self) -> bool {
        self.media.is_connected()
    }

    fn set_muted(&self, muted: bool) {
        self.shared.call(format!("set_muted({muted})"));
        self.media.set_muted(muted);
    }

    fn set_camera(&self, on: bool) {
        self.shared.call(format!("set_camera({on})"));
        self.media.set_camera(on);
    }

    fn ring(&self, tone: RingTone) {
        self.shared.call(format!("ring({tone:?})"));
        self.media.ring(tone);
    }

    fn stop_ringing(&self) {
        self.shared.call("stop_ringing");
        self.media.stop_ringing();
    }

    fn close(&self) {
        self.shared.call("close");
        self.media.close();
        self.shared.notes.borrow_mut().closed = true;
    }

    fn on_candidate_gathered(&self) -> &Event<IceCandidate> {
        &self.shared.gathered
    }

    fn on_connected(&self) -> &Event<()> {
        &self.shared.connected
    }

    fn on_failed(&self) -> &Event<()> {
        &self.shared.failed
    }

    fn on_media_changed(&self) -> &Event<()> {
        &self.shared.changed
    }
}
