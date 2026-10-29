//! The camera for a video call: P3's [`Camera`](crate::seams::media::Camera) (the Camera portal,
//! then PipeWire, then `v4l2src` — shell-design §10.5) behind the media's [`CallCamera`] seam.
//!
//! The `video-camera` setting picks the device (Settings' "Camera for video messages" — one camera
//! choice for the whole app: a PipeWire `node.name`, else a v4l2 path); none chosen asks for the
//! first. A refusal answers `None`, and the call goes on camera-off: a camera is never a reason to
//! miss a call (docs/protocol.md, "Video"). `Denied` is the person's no and is never answered by
//! trying another device; every open asks the portal again (GNOME prompts the first time only).
//!
//! **What comes back is decoded here** (P3's facts): a `pipewiresrc` (it owns the portal's fd,
//! closed when the element is finalised — so [`CameraFeed::keep`] is empty), a device monitor's
//! element, or a `v4l2src` that MAY DELIVER MJPEG, its caps not fixed. The media's camera branch
//! starts with `videoconvert`, which takes raw video only, so the source is handed over inside a
//! bin — `source ! decodebin ! videoconvert`, the decoder linked when its pad appears — whose one
//! `src` pad is raw whatever the camera sends (raw passes through `decodebin` untouched).
//!
//! The slot is read at the moment of the call, never kept (`seams::Slot`): P3 installs its camera
//! after this was made.

use fc_core::rt::LocalBoxFuture;

use super::{CallCamera, CameraFeed};
use crate::seams::media::CameraRefusal;
use crate::services::settings_store::keys;
use crate::shell::Shell;

/// The shell's camera as the call media asks for it.
#[derive(Debug, Default)]
pub struct ShellCamera;

impl CallCamera for ShellCamera {
    fn open(&self) -> LocalBoxFuture<'static, Option<CameraFeed>> {
        let Some(shell) = Shell::try_get() else {
            return Box::pin(async { None });
        };
        let device = shell
            .settings
            .string(keys::VIDEO_CAMERA)
            .filter(|device| !device.trim().is_empty());
        let opening = shell.camera.get().open(device);
        Box::pin(async move {
            match opening.await {
                Ok(source) => match decoded(source) {
                    Some(source) => Some(CameraFeed {
                        source,
                        keep: Box::new(()),
                    }),
                    None => {
                        crate::fc_warn!("call: the camera's decoder could not be made");
                        None
                    }
                },
                Err(refusal) => {
                    let why = match refusal {
                        CameraRefusal::Denied => "denied",
                        CameraRefusal::Absent => "absent",
                        CameraRefusal::Failed => "failed",
                    };
                    crate::fc_info!("call: no camera ({why}); the call goes on without one");
                    None
                }
            }
        })
    }
}

/// `source ! decodebin ! videoconvert` in a bin with one `src` pad (raw video), the decoder linked
/// when its pad appears — on a streaming thread, where linking two pads is what GStreamer expects.
fn decoded(source: gst::Element) -> Option<gst::Element> {
    use gst::prelude::*;

    let bin = gst::Bin::new();
    let decode = gst::ElementFactory::make("decodebin").build().ok()?;
    let convert = gst::ElementFactory::make("videoconvert").build().ok()?;
    bin.add_many([&source, &decode, &convert]).ok()?;
    source.link(&decode).ok()?;
    let sink = convert.static_pad("sink")?;
    decode.connect_pad_added(move |_, pad| {
        let video = pad
            .current_caps()
            .unwrap_or_else(|| pad.query_caps(None))
            .structure(0)
            .is_some_and(|structure| structure.name().starts_with("video/"));
        if video && !sink.is_linked() {
            let _ = pad.link(&sink);
        }
    });
    let ghost = gst::GhostPad::with_target(&convert.static_pad("src")?).ok()?;
    bin.add_pad(&ghost).ok()?;
    Some(bin.upcast())
}
