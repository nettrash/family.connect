//! Linux's own (no C# counterpart): the camera seam `round_recorder::VideoMessageRecorder` driven
//! the way its documentation tells the shell to — Record, the take's start, Stop and keep, the read
//! back, `round_video_rules::plan_take`, then `clip_finished` — over a fake camera, so the contract's
//! types fit together and a finished square lands in REVIEW ready to send.

mod common;

use std::cell::{Cell, RefCell};
use std::future::ready;
use std::path::PathBuf;
use std::rc::Rc;

use fc_core::protocol::RoundVideoLimits;
use fc_core::rt::{Event, LocalBoxFuture};
use fc_logic::round_recorder::{
    CameraEvent, CameraOpenFailure, RecorderAction, RecorderStage, RoundRecorder,
    VideoMessageRecorder, VideoTake,
};
use fc_logic::round_video_rules::{self as rules, CameraChoice, CameraPanel, TakeReading};
use fc_logic::voice_notes::CapabilityAccess;
use futures::executor::block_on;

const LIMITS: RoundVideoLimits = RoundVideoLimits {
    max_ms: 60_000,
    max_bytes: 12_582_912,
};

/// A camera that opens, records and reads back the square it was asked for.
struct FakeCamera {
    events: Event<CameraEvent>,
    open: Cell<bool>,
    recording: Cell<bool>,
    deleted: RefCell<Vec<PathBuf>>,
}

impl FakeCamera {
    fn new() -> Self {
        FakeCamera {
            events: Event::new(),
            open: Cell::new(false),
            recording: Cell::new(false),
            deleted: RefCell::default(),
        }
    }
}

impl VideoMessageRecorder for FakeCamera {
    fn has_camera(&self) -> bool {
        true
    }

    fn camera_access(&self) -> CapabilityAccess {
        CapabilityAccess::UserPromptRequired
    }

    fn microphone_access(&self) -> CapabilityAccess {
        CapabilityAccess::Allowed
    }

    fn cameras(&self) -> LocalBoxFuture<'static, Vec<CameraChoice>> {
        Box::pin(ready(vec![
            CameraChoice::new("v4l2_input.usb", "USB Camera", CameraPanel::Unknown),
            CameraChoice::new("libcamera.front", "Front Camera", CameraPanel::Front),
        ]))
    }

    fn open(&self, _camera_id: &str) -> LocalBoxFuture<'static, Result<(), CameraOpenFailure>> {
        self.open.set(true);
        Box::pin(ready(Ok(())))
    }

    fn events(&self) -> &Event<CameraEvent> {
        &self.events
    }

    fn start(&self) -> LocalBoxFuture<'static, bool> {
        let started = self.open.get();
        self.recording.set(started);
        Box::pin(ready(started))
    }

    fn stop(&self) -> LocalBoxFuture<'static, Option<VideoTake>> {
        let take = self.recording.replace(false).then(|| VideoTake {
            path: PathBuf::from("round-take.mp4"),
            elapsed_ms: 23_400,
            asked: rules::encodes(30, 1, true)[0],
        });
        Box::pin(ready(take))
    }

    fn discard(&self) -> LocalBoxFuture<'static, ()> {
        self.recording.set(false);
        Box::pin(ready(()))
    }

    fn close(&self) -> LocalBoxFuture<'static, ()> {
        self.recording.set(false);
        self.open.set(false);
        Box::pin(ready(()))
    }

    fn read_back(&self, take: &VideoTake) -> LocalBoxFuture<'static, Option<TakeReading>> {
        let asked = take.asked;
        Box::pin(ready(Some(TakeReading {
            bytes: 1_649_700,
            video_codec: "h264".to_string(),
            width: asked.width,
            height: asked.height,
            rotation: 0,
            audio_codec: Some("aac".to_string()),
            frame_rate: Some(30.0),
            duration_ms: Some(23_450),
            moov_first: true,
        })))
    }

    fn poster(&self, _take: &VideoTake) -> LocalBoxFuture<'static, Option<Vec<u8>>> {
        Box::pin(ready(Some(vec![0xFF, 0xD8, 0xFF])))
    }

    fn delete(&self, take: &VideoTake) {
        self.deleted.borrow_mut().push(take.path.clone());
    }
}

#[test]
fn a_take_through_the_camera_seam_lands_in_review_ready_to_send() {
    let camera: Rc<dyn VideoMessageRecorder> = Rc::new(FakeCamera::new());
    let mut recorder = RoundRecorder::new(LIMITS, Rc::new(common::english()));
    let refused = rules::refused_before(camera.camera_access(), camera.microphone_access());
    let asks = rules::will_ask(camera.camera_access(), camera.microphone_access());
    assert_eq!(None, refused);
    let opening = recorder.open(0, refused, asks);
    assert_eq!(RecorderAction::OpenCamera, opening[0].action);

    let cameras = block_on(camera.cameras());
    let chosen = rules::pick_camera(&cameras, None).expect("a camera");
    assert_eq!("libcamera.front", chosen.id);
    block_on(camera.open(&chosen.id)).expect("it opens");
    recorder.camera_on(10);
    let first = Rc::new(Cell::new(false));
    {
        let first = Rc::clone(&first);
        camera.events().subscribe(move |event| {
            if *event == CameraEvent::FirstFrame {
                first.set(true);
            }
        });
    }
    camera.events().emit(&CameraEvent::FirstFrame);
    assert!(first.get());
    recorder.first_frame();

    let record = recorder.slot(1_000);
    assert_eq!(RecorderAction::StartRecording, record[0].action);
    assert!(block_on(camera.start()));
    recorder.take_started(1_000);

    let stop = recorder.slot(24_400);
    assert_eq!(RecorderAction::StopAndKeep, stop[0].action);
    let take = block_on(camera.stop()).expect("a take");
    let reading = block_on(camera.read_back(&take));
    let say = common::english();
    let (plan, length) = rules::plan_take(
        &take.asked,
        reading.as_ref(),
        take.elapsed_ms,
        &LIMITS,
        &say,
    );
    assert!(plan.round && plan.from_square);
    let poster = block_on(camera.poster(&take));
    let staged = rules::staged(vec![0; 16], length as i32, poster);
    assert_eq!(Some(23_450), staged.duration_ms);
    recorder.clip_finished(25_000, Some(plan), Some(length));
    assert_eq!(RecorderStage::Review, recorder.stage());
    assert!(recorder.clip_ready());
    let sent = recorder.slot(25_000);
    assert_eq!(RecorderAction::Send, sent[0].action);
    block_on(camera.close());
}
