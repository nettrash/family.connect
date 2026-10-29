//! Animations sent in a chat (shell-design §8.4, #96; fc-logic's `animation_preparing` — the
//! traits' rustdoc is the contract): an animated GIF, WebP or PNG made into the protocol's silent
//! looping MP4. fc-logic cuts and composites the frames; the shell decodes a still
//! ([`GdkStillDecoder`], gdk-pixbuf/glycin, straight BGRA) and encodes ([`GstAnimationEncoder`]):
//! NV12 pushed into `appsrc` exactly as made (BT.709 limited — no `videoconvert`), `x264enc` High
//! then `openh264enc` Main, NO audio, and `mp4mux` **WITHOUT `faststart`** — `moov` must come LAST
//! so `animation_mp4::exact_ticks` can rewrite the timing; fc-logic moves the index to the front
//! afterwards.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::{Cancel, Clock, LocalBoxFuture};
use fc_logic::animated_pictures::animation_prep;
use fc_logic::animation_preparing::{
    self, AnimationEncoder, DecodedStill, FrameFeed, H264Profile, StillDecoder,
};
use fc_logic::animation_timing::AnimationPlan;
use fc_logic::media_outbox::StagedMedia;
use fc_logic::transcode_attempts::{AttemptFailure, Attempted, OperationCanceled};
use futures_util::StreamExt;
use futures_util::future::{Either, select};
use gst::prelude::*;
use gtk::gdk_pixbuf::prelude::*;

use super::gst_util::{self, PipelineError};
use super::pipelines;
use super::voice_recorder::{Stopper, TempGuard};
use crate::shell::Shell;

/// How long one frame may wait to be taken by the encoder before the attempt is called failed.
const PUSH_LIMIT: Duration = Duration::from_secs(30);

/// fc-logic's `StillDecoder` over gdk-pixbuf (glycin): the first frame, its own size, straight
/// BGRA, no colour management, EXIF ignored.
#[derive(Debug, Default)]
pub struct GdkStillDecoder;

impl StillDecoder for GdkStillDecoder {
    fn decode_still(&self, still: Vec<u8>) -> LocalBoxFuture<'static, Option<DecodedStill>> {
        Box::pin(async move { decode(&still) })
    }
}

fn decode(still: &[u8]) -> Option<DecodedStill> {
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.write(still).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    let width = pixbuf.width();
    let height = pixbuf.height();
    let channels = usize::try_from(pixbuf.n_channels()).ok()?;
    let stride = usize::try_from(pixbuf.rowstride()).ok()?;
    let alpha = pixbuf.has_alpha();
    let bytes = pixbuf.read_pixel_bytes();
    let pixels: &[u8] = &bytes;
    let (w, h) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
    let mut bgra = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let at = y * stride + x * channels;
            let pixel = pixels.get(at..at + channels)?;
            bgra.extend_from_slice(&[
                pixel[2],
                pixel[1],
                pixel[0],
                if alpha { pixel[3] } else { 255 },
            ]);
        }
    }
    Some(DecodedStill {
        width,
        height,
        bgra,
    })
}

/// fc-logic's `AnimationEncoder` on GStreamer (see the module's comment).
#[derive(Debug, Clone)]
pub struct GstAnimationEncoder {
    tmp: PathBuf,
}

impl GstAnimationEncoder {
    pub fn new(tmp: PathBuf) -> Self {
        GstAnimationEncoder { tmp }
    }

    /// The raw encode (before fc-logic's timing and faststart): the bytes `mp4mux` wrote, or the
    /// attempt's end. What the self-test checks starts `ftyp`, then `mdat` — no faststart.
    pub async fn encode_raw(
        &self,
        plan: &AnimationPlan,
        profile: H264Profile,
        feed: &mut FrameFeed,
        cancel: Cancel,
    ) -> Result<Attempted<Vec<u8>>, AttemptFailure> {
        if !gst_util::has_element(pipelines::h264_factory(profile)) {
            return Ok(Attempted::Refused);
        }
        let Ok(file) = gst_util::temp_file(&self.tmp, "animation", "mp4") else {
            return Err(AttemptFailure::new("temporary", 0));
        };
        let _guard = TempGuard(file.clone());
        let bitrate = u32::try_from(plan.video_bitrate.max(1)).unwrap_or(u32::MAX);
        let Ok(pipeline) = gst_util::launch(&pipelines::animation_encode(
            plan.width,
            plan.height,
            bitrate,
            profile,
        )) else {
            return Ok(Attempted::Refused);
        };
        let _stopper = Stopper(pipeline.clone());
        gst_util::set_location(&pipeline, "out", &file);
        let Some(source) = pipeline
            .by_name("frames")
            .and_then(|element| element.downcast::<gst_app::AppSrc>().ok())
        else {
            return Ok(Attempted::Refused);
        };
        source.set_max_bytes(0);
        source.set_block(false);
        let Some(bus) = pipeline.bus() else {
            return Err(AttemptFailure::new("bus", 0));
        };
        let mut messages = bus.stream();
        if pipeline.set_state(gst::State::Playing).is_err() {
            return Ok(Attempted::Refused);
        }
        // Feed every frame, then EOS; a bus error, the cancel, or a stall ends it.
        while let Some(frame) = feed.next().await {
            if cancel.is_cancelled() {
                return Ok(Attempted::Refused);
            }
            let mut buffer = gst::Buffer::from_mut_slice(frame.nv12);
            if let Some(buffer) = buffer.get_mut() {
                buffer.set_pts(gst::ClockTime::from_nseconds(
                    u64::try_from(frame.pts_ns).unwrap_or(0),
                ));
                buffer.set_duration(gst::ClockTime::from_nseconds(
                    u64::try_from(frame.duration_ns).unwrap_or(0),
                ));
            }
            if source.push_buffer(buffer).is_err() {
                return Err(error_on(&bus)
                    .unwrap_or_else(|| PipelineError::new("push", 0))
                    .attempt_failure());
            }
            if let Some(error) = error_on_stream(&mut messages) {
                return Err(error.attempt_failure());
            }
        }
        let _ = source.end_of_stream();
        let waiting = async {
            while let Some(message) = messages.next().await {
                match message.view() {
                    gst::MessageView::Eos(_) => return Ok(()),
                    gst::MessageView::Error(error) => {
                        return Err(PipelineError::from_message(error));
                    }
                    _ => {}
                }
            }
            Err(PipelineError::new("bus", 1))
        };
        let stop = async {
            let cancelled = cancel.cancelled();
            match select(
                Box::pin(cancelled),
                Box::pin(glib::timeout_future(PUSH_LIMIT)),
            )
            .await
            {
                Either::Left(_) => PipelineError::new("cancelled", 0),
                Either::Right(_) => PipelineError::new("timeout", 0),
            }
        };
        match select(Box::pin(waiting), Box::pin(stop)).await {
            Either::Left((Ok(()), _)) => {}
            Either::Left((Err(error), _)) => return Err(error.attempt_failure()),
            Either::Right((error, _)) if error.domain == "cancelled" => {
                return Ok(Attempted::Refused);
            }
            Either::Right((error, _)) => return Err(error.attempt_failure()),
        }
        let _ = pipeline.set_state(gst::State::Null);
        // Read back: H.264, the plan's size, no sound.
        let Some(info) = gst_util::discover(&file, Duration::from_secs(10)).await else {
            return Ok(Attempted::Wrong);
        };
        let video = info.video_streams().into_iter().next();
        let right = info.audio_streams().is_empty()
            && video.is_some_and(|video| {
                let codec = gst_pbutils::prelude::DiscovererStreamInfoExt::caps(&video)
                    .and_then(|caps| caps.structure(0).map(super::preparing::video_codec));
                codec.as_deref() == Some("h264")
                    && i64::from(video.width()) == i64::from(plan.width)
                    && i64::from(video.height()) == i64::from(plan.height)
            });
        if !right {
            return Ok(Attempted::Wrong);
        }
        match std::fs::read(&file) {
            Ok(bytes) => Ok(Attempted::Taken(bytes)),
            Err(_) => Err(AttemptFailure::new("read", 0)),
        }
    }
}

/// An error already on the bus stream (without waiting).
fn error_on_stream(messages: &mut gst::bus::BusStream) -> Option<PipelineError> {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    while let std::task::Poll::Ready(Some(message)) = messages.poll_next_unpin(&mut context) {
        if let gst::MessageView::Error(error) = message.view() {
            return Some(PipelineError::from_message(error));
        }
    }
    None
}

fn error_on(bus: &gst::Bus) -> Option<PipelineError> {
    gst_util::first_error(bus)
}

impl AnimationEncoder for GstAnimationEncoder {
    fn encode<'a>(
        &'a self,
        plan: &'a AnimationPlan,
        profile: H264Profile,
        feed: &'a mut FrameFeed,
        cancel: Cancel,
    ) -> LocalBoxFuture<'a, Result<Attempted<Vec<u8>>, AttemptFailure>> {
        Box::pin(self.encode_raw(plan, profile, feed, cancel))
    }

    fn poster_jpeg(&self, bgra: &[u8], width: i32, height: i32, quality: f64) -> Option<Vec<u8>> {
        let (w, h) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
        if bgra.len() != w * h * 4 {
            return None;
        }
        let mut rgb = Vec::with_capacity(w * h * 3);
        for pixel in bgra.chunks_exact(4) {
            rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
        let grabbed = super::preparing::Grabbed {
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
            stride: w * 3,
            bytes: rgb,
        };
        grabbed.jpeg(quality)
    }
}

/// Whether a picked picture is animated (worth making into the animation).
pub fn is_animated(head: &[u8], file: &Path) -> bool {
    if !animation_prep::may_be(head) {
        return false;
    }
    std::fs::read(file)
        .ok()
        .and_then(|bytes| animation_prep::animated(&bytes))
        .is_some()
}

/// An animated picture made into the animation — `Ok(None)` for rule C (sent as before), `Err`
/// for the person's cancel.
pub async fn prepare(
    bytes: Rc<[u8]>,
    clock: &dyn Clock,
    log: &mut dyn FnMut(&str),
    cancel: &Cancel,
) -> Result<Option<StagedMedia>, OperationCanceled> {
    let encoder = GstAnimationEncoder::new(Shell::get().paths.tmp_dir());
    animation_preparing::prepare(
        bytes,
        Rc::new(GdkStillDecoder),
        &encoder,
        clock,
        log,
        cancel,
    )
    .await
}
