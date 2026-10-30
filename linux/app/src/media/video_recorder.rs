//! The camera of a VIDEO MESSAGE (shell-design §10.5; fc-logic's `round_recorder::
//! VideoMessageRecorder` — its rustdoc is the contract): the camera from [`Camera::open`] (the
//! portal, PipeWire, the fallbacks), a MIRRORED preview into `gtk4paintablesink`, and — from Record
//! to Stop — a take of its own: the frames pushed from the camera pipeline's `appsink` into the
//! take's `appsrc`, the centre square cut, 480 × 480 at the camera's rate up to 30, NOT mirrored,
//! upright, H.264 + the microphone's AAC, `mp4mux faststart=true`. The microphone opens only at
//! Record; PREVIEW never records.
//!
//! **However it ends the camera and the microphone are let go of**: every path ends in
//! [`close`](VideoMessageRecorder::close) — the take thrown away unless handed over, both pipelines
//! `Null`, the source (and with it the portal's PipeWire fd) dropped, the preview's paintable
//! dropped.
//!
//! Deviation from the contract's step 2 (`pick_mode` over the camera's caps): the stream is left to
//! negotiate its own mode and the take is built from the caps that came (`videoconvert` takes any
//! raw format, `decodebin` an MJPEG `v4l2src`); the rate is never raised.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fc_core::rt::{Event, LocalBoxFuture};
use fc_logic::faststart;
use fc_logic::media_encoding::VideoEncoding;
use fc_logic::round_recorder::{CameraEvent, CameraOpenFailure, VideoMessageRecorder, VideoTake};
use fc_logic::round_video_rules::{self, CameraChoice, CameraPanel, CameraTrouble, TakeReading};
use fc_logic::voice_notes::CapabilityAccess;
use futures_util::StreamExt;
use futures_util::future::{Either, select};
use gst::prelude::*;
use gst_pbutils::prelude::*;

use super::gst_util::{self, PipelineError};
use super::pipelines::{self, AacEncoder};
use crate::seams::media::CameraRefusal;
use crate::shell::Shell;

/// How long a take's file may take to finish.
const STOP_LIMIT: Duration = Duration::from_secs(10);
/// How long the brightness of the preview is reported (S3.6's covered camera).
const BRIGHTNESS_FOR: Duration =
    Duration::from_millis(round_video_rules::BLACK_CHECK_MS as u64 + 500);
/// How often (at most) a brightness is reported.
const BRIGHTNESS_EVERY: Duration = Duration::from_millis(200);

/// The camera's mode as it came: width, height, frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mode {
    width: u32,
    height: u32,
    numerator: u32,
    denominator: u32,
}

/// What the camera's streaming thread tells the main context.
enum News {
    FirstFrame,
    Brightness(f64),
}

/// The open camera.
struct Open {
    pipeline: gst::Pipeline,
    paintable: Option<gtk::gdk::Paintable>,
    feeding: Arc<Mutex<Option<gst_app::AppSrc>>>,
    mode: Arc<Mutex<Option<Mode>>>,
    _watch: Option<gst::bus::BusWatchGuard>,
}

/// A take being recorded.
struct Take {
    pipeline: gst::Pipeline,
    file: PathBuf,
    asked: VideoEncoding,
    started: Instant,
}

/// fc-logic's `VideoMessageRecorder` on GStreamer (see the module's comment).
pub struct GstVideoRecorder {
    events: Event<CameraEvent>,
    open: RefCell<Option<Open>>,
    take: RefCell<Option<Take>>,
    serial: Cell<u64>,
    tmp: PathBuf,
}

impl std::fmt::Debug for GstVideoRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GstVideoRecorder")
            .field("open", &self.open.borrow().is_some())
            .field("recording", &self.take.borrow().is_some())
            .finish_non_exhaustive()
    }
}

impl GstVideoRecorder {
    pub fn new() -> Rc<GstVideoRecorder> {
        let tmp = Shell::get().paths.tmp_dir();
        // Takes left by an earlier run (a crash mid-take) go when the recorder is made.
        if let Ok(entries) = std::fs::read_dir(&tmp) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy().starts_with("round-") {
                    gst_util::remove_file(&entry.path());
                }
            }
        }
        Rc::new(GstVideoRecorder {
            events: Event::new(),
            open: RefCell::new(None),
            take: RefCell::new(None),
            serial: Cell::new(0),
            tmp,
        })
    }

    /// The live preview's paintable (the recorder's round clip draws it).
    pub fn preview(&self) -> Option<gtk::gdk::Paintable> {
        self.open
            .borrow()
            .as_ref()
            .and_then(|open| open.paintable.clone())
    }

    fn me(&self) -> Option<Rc<GstVideoRecorder>> {
        super::recorder_layer::recorder().filter(|me| std::ptr::eq(Rc::as_ptr(me), self))
    }

    async fn open_camera(self: Rc<Self>, camera_id: String) -> Result<(), CameraOpenFailure> {
        self.close_now();
        let serial = self.serial.get().wrapping_add(1);
        self.serial.set(serial);
        let camera = Shell::get().camera.get();
        let source = match camera.open(Some(camera_id)).await {
            Ok(source) => source,
            Err(CameraRefusal::Denied) => {
                return Err(CameraOpenFailure::Refused(
                    round_video_rules::refused_after(
                        self.camera_access(),
                        self.microphone_access(),
                    ),
                ));
            }
            Err(CameraRefusal::Absent) => {
                return Err(CameraOpenFailure::Troubled(CameraTrouble::Missing));
            }
            Err(CameraRefusal::Failed) => {
                return Err(CameraOpenFailure::Troubled(CameraTrouble::Failed));
            }
        };
        if self.serial.get() != serial {
            return Err(CameraOpenFailure::Troubled(CameraTrouble::Failed));
        }
        let decode = source
            .factory()
            .is_some_and(|factory| factory.name() == "v4l2src");
        let pipeline = gst::Pipeline::new();
        let unsupported = || CameraOpenFailure::Troubled(CameraTrouble::Unsupported);
        let branches = gst::parse::bin_from_description(&pipelines::camera_branches(decode), true)
            .map_err(|_| unsupported())?;
        pipeline
            .add_many([&source, branches.upcast_ref()])
            .map_err(|_| unsupported())?;
        source.link(&branches).map_err(|_| unsupported())?;
        let paintable = branches
            .by_name("preview")
            .map(|sink| sink.property::<gtk::gdk::Paintable>("paintable"));
        let frames = branches
            .by_name("frames")
            .and_then(|sink| sink.downcast::<gst_app::AppSink>().ok())
            .ok_or_else(unsupported)?;
        let feeding: Arc<Mutex<Option<gst_app::AppSrc>>> = Arc::new(Mutex::new(None));
        let mode: Arc<Mutex<Option<Mode>>> = Arc::new(Mutex::new(None));
        let (sender, mut receiver) = futures_channel::mpsc::unbounded::<News>();
        {
            let (feeding, mode) = (Arc::clone(&feeding), Arc::clone(&mode));
            let mut first: Option<Instant> = None;
            let mut last_brightness: Option<Instant> = None;
            frames.set_callbacks(
                gst_app::AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                        let now = Instant::now();
                        if first.is_none() {
                            first = Some(now);
                            if let Some(info) = sample
                                .caps()
                                .and_then(|caps| gst_video::VideoInfo::from_caps(caps).ok())
                                && let Ok(mut held) = mode.lock()
                            {
                                let rate = info.fps();
                                *held = Some(Mode {
                                    width: info.width(),
                                    height: info.height(),
                                    numerator: u32::try_from(rate.numer()).unwrap_or(0),
                                    denominator: u32::try_from(rate.denom()).unwrap_or(1),
                                });
                            }
                            let _ = sender.unbounded_send(News::FirstFrame);
                        }
                        let early =
                            first.is_some_and(|first| now.duration_since(first) < BRIGHTNESS_FOR);
                        let due = last_brightness
                            .is_none_or(|last| now.duration_since(last) >= BRIGHTNESS_EVERY);
                        if early && due {
                            last_brightness = Some(now);
                            if let Some(luma) = sample.buffer().and_then(mean_luma) {
                                let _ = sender.unbounded_send(News::Brightness(luma));
                            }
                        }
                        // While a take records, its appsrc gets every frame — timestamped by the
                        // take's own clock (`do-timestamp`).
                        let source = feeding.lock().ok().and_then(|held| held.clone());
                        if let (Some(source), Some(buffer)) = (source, sample.buffer()) {
                            let mut copy = buffer.copy();
                            if let Some(copy) = copy.get_mut() {
                                copy.set_pts(gst::ClockTime::NONE);
                                copy.set_dts(gst::ClockTime::NONE);
                            }
                            let _ = source.push_buffer(copy);
                        }
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            );
        }
        let watch =
            pipeline.bus().and_then(|bus| {
                let weak = Rc::downgrade(&self);
                bus.add_watch_local(move |_, message| {
                    let Some(recorder) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    if let gst::MessageView::Error(error) = message.view() {
                        let error = PipelineError::from_message(error);
                        crate::fc_warn!("recorder: the camera failed ({error})");
                        if recorder.serial.get() == serial {
                            recorder.events.emit(&CameraEvent::Troubled(
                                round_video_rules::trouble(&error.domain, error.code),
                            ));
                        }
                    }
                    glib::ControlFlow::Continue
                })
                .ok()
            });
        if pipeline.set_state(gst::State::Playing).is_err() {
            let error = pipeline
                .bus()
                .and_then(|bus| gst_util::first_error(&bus))
                .unwrap_or_else(|| PipelineError::new(round_video_rules::CORE_ERROR_DOMAIN, 0));
            crate::fc_warn!("recorder: the camera would not start ({error})");
            let _ = pipeline.set_state(gst::State::Null);
            return Err(CameraOpenFailure::Troubled(round_video_rules::trouble(
                &error.domain,
                error.code,
            )));
        }
        {
            let weak = Rc::downgrade(&self);
            glib::spawn_future_local(async move {
                while let Some(news) = receiver.next().await {
                    let Some(recorder) = weak.upgrade() else {
                        return;
                    };
                    if recorder.serial.get() != serial {
                        return;
                    }
                    recorder.events.emit(&match news {
                        News::FirstFrame => CameraEvent::FirstFrame,
                        News::Brightness(luma) => CameraEvent::Brightness(luma),
                    });
                }
            });
        }
        *self.open.borrow_mut() = Some(Open {
            pipeline,
            paintable,
            feeding,
            mode,
            _watch: watch,
        });
        Ok(())
    }

    /// The take thrown away, the camera off — now.
    fn close_now(&self) {
        self.discard_now();
        if let Some(open) = self.open.borrow_mut().take() {
            let _ = open.pipeline.set_state(gst::State::Null);
        }
        self.serial.set(self.serial.get().wrapping_add(1));
    }

    fn discard_now(&self) {
        if let Some(open) = self.open.borrow().as_ref()
            && let Ok(mut feeding) = open.feeding.lock()
        {
            *feeding = None;
        }
        if let Some(take) = self.take.borrow_mut().take() {
            let _ = take.pipeline.set_state(gst::State::Null);
            gst_util::remove_file(&take.file);
        }
    }

    fn start_take(&self) -> bool {
        let Some((feeding, mode)) = self.open.borrow().as_ref().map(|open| {
            (
                Arc::clone(&open.feeding),
                open.mode.lock().ok().and_then(|held| *held),
            )
        }) else {
            return false;
        };
        let Some(mode) = mode else {
            return false;
        };
        let Some(crop) = round_video_rules::centre_square(mode.width, mode.height) else {
            return false;
        };
        let aac = AacEncoder::ORDER
            .into_iter()
            .find(|encoder| gst_util::has_element(encoder.factory()));
        let microphone = super::test_sources() || gst_util::has_element("pulsesrc");
        let has_audio = aac.is_some() && microphone;
        let asked = round_video_rules::encodes(mode.numerator, mode.denominator, has_audio)
            .into_iter()
            .find(|asked| gst_util::has_element(pipelines::h264_factory(asked.profile)));
        let Some(asked) = asked else {
            crate::fc_warn!("recorder: no H.264 encoder");
            return false;
        };
        let Ok(file) = gst_util::temp_file(&self.tmp, "round", "mp4") else {
            return false;
        };
        let description = pipelines::take_recording(
            crop.videocrop(mode.width, mode.height),
            &asked,
            super::microphone(),
            aac.unwrap_or(AacEncoder::Avenc),
        );
        let Ok(pipeline) = gst_util::launch(&description) else {
            gst_util::remove_file(&file);
            return false;
        };
        gst_util::set_location(&pipeline, "out", &file);
        let Some(source) = pipeline
            .by_name("frames")
            .and_then(|element| element.downcast::<gst_app::AppSrc>().ok())
        else {
            gst_util::remove_file(&file);
            return false;
        };
        source.set_caps(Some(
            &gst::Caps::builder("video/x-raw")
                .field("format", "I420")
                .field("width", i32::try_from(mode.width).unwrap_or(0))
                .field("height", i32::try_from(mode.height).unwrap_or(0))
                .field(
                    "framerate",
                    gst::Fraction::new(
                        i32::try_from(mode.numerator).unwrap_or(30),
                        i32::try_from(mode.denominator.max(1)).unwrap_or(1),
                    ),
                )
                .build(),
        ));
        if pipeline.set_state(gst::State::Playing).is_err() {
            let _ = pipeline.set_state(gst::State::Null);
            gst_util::remove_file(&file);
            return false;
        }
        if let Ok(mut held) = feeding.lock() {
            *held = Some(source);
        }
        *self.take.borrow_mut() = Some(Take {
            pipeline,
            file,
            asked,
            started: Instant::now(),
        });
        true
    }

    async fn stop_take(self: Rc<Self>) -> Option<VideoTake> {
        if let Some(open) = self.open.borrow().as_ref()
            && let Ok(mut feeding) = open.feeding.lock()
        {
            *feeding = None;
        }
        let take = self.take.borrow_mut().take()?;
        let elapsed_ms = i64::try_from(take.started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let mut messages = take.pipeline.bus()?.stream();
        if let Some(source) = take
            .pipeline
            .by_name("frames")
            .and_then(|element| element.downcast::<gst_app::AppSrc>().ok())
        {
            let _ = source.end_of_stream();
        }
        if let Some(microphone) = take.pipeline.by_name("mic") {
            microphone.send_event(gst::event::Eos::new());
        }
        let waiting = async {
            while let Some(message) = messages.next().await {
                match message.view() {
                    gst::MessageView::Eos(_) => return true,
                    gst::MessageView::Error(error) => {
                        crate::fc_warn!(
                            "recorder: the take could not be finished ({})",
                            PipelineError::from_message(error)
                        );
                        return false;
                    }
                    _ => {}
                }
            }
            false
        };
        let finished = match select(
            Box::pin(waiting),
            Box::pin(glib::timeout_future(STOP_LIMIT)),
        )
        .await
        {
            Either::Left((finished, _)) => finished,
            Either::Right(_) => false,
        };
        let _ = take.pipeline.set_state(gst::State::Null);
        let length = std::fs::metadata(&take.file)
            .map(|meta| meta.len())
            .unwrap_or(0);
        if !finished || length == 0 {
            gst_util::remove_file(&take.file);
            return None;
        }
        Some(VideoTake {
            path: take.file,
            elapsed_ms,
            asked: take.asked,
        })
    }
}

/// The mean of every eighth pixel's Y of an I420 frame, 0 to 1 (S3.6).
fn mean_luma(buffer: &gst::BufferRef) -> Option<f64> {
    let map = buffer.map_readable().ok()?;
    let bytes = map.as_slice();
    // The Y plane is the first two thirds of an I420 frame.
    let luma = &bytes[..bytes.len() * 2 / 3];
    let (sum, count) = luma
        .iter()
        .step_by(8)
        .fold((0u64, 0u64), |(sum, count), y| {
            (sum + u64::from(*y), count + 1)
        });
    (count > 0).then(|| sum as f64 / count as f64 / 255.0)
}

impl VideoMessageRecorder for GstVideoRecorder {
    fn has_camera(&self) -> bool {
        Shell::get().camera.get().present()
    }

    fn camera_access(&self) -> CapabilityAccess {
        super::installed_camera().map_or(CapabilityAccess::Unknown, |camera| camera.access())
    }

    fn microphone_access(&self) -> CapabilityAccess {
        CapabilityAccess::Allowed
    }

    fn cameras(&self) -> LocalBoxFuture<'static, Vec<CameraChoice>> {
        let devices = Shell::get().camera.get().devices();
        Box::pin(std::future::ready(
            devices
                .into_iter()
                .map(|device| CameraChoice::new(&device.id, &device.name, CameraPanel::Unknown))
                .collect(),
        ))
    }

    fn open(&self, camera_id: &str) -> LocalBoxFuture<'static, Result<(), CameraOpenFailure>> {
        match self.me() {
            Some(me) => Box::pin(me.open_camera(camera_id.to_string())),
            None => Box::pin(std::future::ready(Err(CameraOpenFailure::Troubled(
                CameraTrouble::Failed,
            )))),
        }
    }

    fn events(&self) -> &Event<CameraEvent> {
        &self.events
    }

    fn start(&self) -> LocalBoxFuture<'static, bool> {
        Box::pin(std::future::ready(self.start_take()))
    }

    fn stop(&self) -> LocalBoxFuture<'static, Option<VideoTake>> {
        match self.me() {
            Some(me) => Box::pin(me.stop_take()),
            None => Box::pin(std::future::ready(None)),
        }
    }

    fn discard(&self) -> LocalBoxFuture<'static, ()> {
        self.discard_now();
        Box::pin(std::future::ready(()))
    }

    fn close(&self) -> LocalBoxFuture<'static, ()> {
        self.close_now();
        Box::pin(std::future::ready(()))
    }

    fn read_back(&self, take: &VideoTake) -> LocalBoxFuture<'static, Option<TakeReading>> {
        let path = take.path.clone();
        Box::pin(async move { read_take(&path).await })
    }

    fn poster(&self, take: &VideoTake) -> LocalBoxFuture<'static, Option<Vec<u8>>> {
        let path = take.path.clone();
        Box::pin(async move {
            let side = round_video_rules::EDGE;
            super::preparing::poster(&path, Some((side, side)), side).await
        })
    }

    fn delete(&self, take: &VideoTake) {
        gst_util::remove_file(&take.path);
    }
}

/// A finished take read back (the discoverer, and `faststart::is_moov_first` on its bytes).
pub async fn read_take(path: &std::path::Path) -> Option<TakeReading> {
    let bytes = std::fs::read(path).ok()?;
    let info = gst_util::discover(path, Duration::from_secs(10)).await?;
    let video = info.video_streams().into_iter().next()?;
    let structure_name = |caps: Option<gst::Caps>| {
        caps.and_then(|caps| {
            caps.structure(0)
                .map(|s| (s.name().to_string(), s.to_owned()))
        })
    };
    let video_codec = structure_name(video.caps())
        .map(|(name, structure)| match name.as_str() {
            "video/x-h264" => "h264".to_string(),
            _ => super::preparing::video_codec(&structure),
        })
        .unwrap_or_else(|| "unknown".to_string());
    let audio_codec = info.audio_streams().into_iter().next().map(|audio| {
        structure_name(audio.caps())
            .map(|(_, structure)| super::preparing::audio_codec(&structure))
            .unwrap_or_else(|| "unknown".to_string())
    });
    let rate = video.framerate();
    let rotation = fc_logic::media_encoding::rotation(
        video.tags().as_ref().and_then(|tags| {
            tags.get::<gst::tags::ImageOrientation>()
                .and_then(|orientation| match orientation.get() {
                    "rotate-90" => Some(90),
                    "rotate-180" => Some(180),
                    "rotate-270" => Some(270),
                    "rotate-0" => Some(0),
                    _ => None,
                })
        }),
        None,
    );
    Some(TakeReading {
        bytes: i64::try_from(bytes.len()).unwrap_or(i64::MAX),
        video_codec,
        width: video.width(),
        height: video.height(),
        rotation,
        audio_codec,
        frame_rate: (rate.numer() > 0 && rate.denom() > 0)
            .then(|| f64::from(rate.numer()) / f64::from(rate.denom())),
        duration_ms: info
            .duration()
            .map(|duration| i64::try_from(duration.mseconds()).unwrap_or(i64::MAX)),
        moov_first: faststart::is_moov_first(&bytes).unwrap_or(false),
    })
}
