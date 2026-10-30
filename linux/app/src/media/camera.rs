//! The camera (shell-design §10.5, §14.2; W.3 `Camera`): **the XDG Camera portal + PipeWire**, then
//! PipeWire's (or V4L2's) devices through a `gst::DeviceMonitor`, then `v4l2src` — and a refusal
//! by the PERSON is never bypassed: it ends in [`CameraRefusal::Denied`].
//!
//! The portal, over GDBus (§0: no zbus): `org.freedesktop.portal.Camera`'s `IsCameraPresent`
//! property, `AccessCamera` (a Request whose `Response` 0 grants, 1 is the person's no, 2 anything
//! else), then `OpenPipeWireRemote` → a Unix fd for `pipewiresrc fd=… target-object=…`. The fd is
//! owned by the source element (dropped with it), so a camera nobody holds is a remote closed.
//! Portal 1.21 made camera permission per app again for host apps (§1.7); GNOME asks.
//!
//! [`Camera::open`] answers a SOURCE element ready to link (a `Bin` with a ghost `src` pad for the
//! self-test's moving picture), usable outside the recorder: P6 takes it for a call's video
//! (§14.2). Nothing is logged but counts and error kinds — never a device's name, the portal's
//! handle token or a node's path.

use std::cell::{Cell, RefCell};
use std::os::fd::{AsRawFd, OwnedFd};
use std::rc::Rc;

use fc_core::rt::{Event, LocalBoxFuture};
use fc_logic::voice_notes::CapabilityAccess;
use gio::prelude::*;
use gst::prelude::DeviceMonitorExtManual;
use gst::prelude::{DeviceExt, DeviceMonitorExt, ObjectExt};

use crate::desktop::portal_request::{self, Response};
use crate::seams::media::{Camera, CameraDevice, CameraRefusal};

const PORTAL_BUS: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const CAMERA_INTERFACE: &str = "org.freedesktop.portal.Camera";
/// The id the self-test's camera is listed under.
pub const TEST_CAMERA_ID: &str = "fc-test-camera";

/// What the portal said.
#[derive(Debug)]
enum PortalAnswer {
    /// Access granted and the PipeWire remote opened.
    Remote(OwnedFd),
    /// The person (or GNOME for them) said no.
    Denied,
    /// No portal, no Camera interface, "not supported", or no camera behind it: try the fallbacks.
    Unavailable,
    /// The portal answered something else.
    Failed,
}

/// The camera service installed in `Shell::camera`.
pub struct GstCamera {
    changed: Event<()>,
    monitor: RefCell<Option<gst::DeviceMonitor>>,
    watch: RefCell<Option<gst::bus::BusWatchGuard>>,
    /// The portal's `IsCameraPresent`, once read.
    portal_present: Cell<Option<bool>>,
    /// What the portal answered in this session (S3.2's reading, without prompting).
    access: Cell<CapabilityAccess>,
}

impl std::fmt::Debug for GstCamera {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GstCamera")
            .field("portal_present", &self.portal_present.get())
            .field("access", &self.access.get())
            .finish_non_exhaustive()
    }
}

impl GstCamera {
    /// Starts the device monitor and reads the portal's `IsCameraPresent` (both quietly).
    pub fn new() -> Rc<GstCamera> {
        let camera = Rc::new(GstCamera {
            changed: Event::new(),
            monitor: RefCell::new(None),
            watch: RefCell::new(None),
            portal_present: Cell::new(None),
            access: Cell::new(CapabilityAccess::UserPromptRequired),
        });
        camera.start_monitor();
        let weak = Rc::downgrade(&camera);
        glib::spawn_future_local(async move {
            let present = portal_present().await;
            if let Some(camera) = weak.upgrade() {
                camera.portal_present.set(present);
                if present.is_some() {
                    camera.changed.emit(&());
                }
            }
        });
        camera
    }

    fn start_monitor(self: &Rc<Self>) {
        let monitor = gst::DeviceMonitor::new();
        if monitor.add_filter(Some("Video/Source"), None).is_none() {
            return;
        }
        let weak = Rc::downgrade(self);
        if let Ok(guard) = monitor.bus().add_watch_local(move |_, message| {
            let Some(camera) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if matches!(
                message.view(),
                gst::MessageView::DeviceAdded(_) | gst::MessageView::DeviceRemoved(_)
            ) {
                camera.changed.emit(&());
            }
            glib::ControlFlow::Continue
        }) {
            *self.watch.borrow_mut() = Some(guard);
        }
        if monitor.start().is_err() {
            crate::fc_info!("camera: no device provider started");
        }
        *self.monitor.borrow_mut() = Some(monitor);
    }

    /// The devices the monitor sees, with their GStreamer handles.
    fn monitored(&self) -> Vec<(CameraDevice, gst::Device)> {
        let Some(monitor) = self.monitor.borrow().clone() else {
            return Vec::new();
        };
        monitor
            .devices()
            .into_iter()
            .map(|device| (describe(&device), device))
            .collect()
    }

    /// What the portal answered in this session, without asking anyone.
    pub fn access(&self) -> CapabilityAccess {
        if super::test_sources() {
            CapabilityAccess::Allowed
        } else {
            self.access.get()
        }
    }

    async fn open_camera(
        self: Rc<Self>,
        device: Option<String>,
    ) -> Result<gst::Element, CameraRefusal> {
        if super::test_sources() {
            return test_camera().ok_or(CameraRefusal::Failed);
        }
        let chosen = device.filter(|id| !id.is_empty());
        match access_camera().await {
            PortalAnswer::Remote(fd) => {
                self.access.set(CapabilityAccess::Allowed);
                match pipewire_source(fd, chosen.as_deref()) {
                    Some(source) => return Ok(source),
                    None => crate::fc_warn!("camera: pipewiresrc is missing"),
                }
            }
            PortalAnswer::Denied => {
                self.access.set(CapabilityAccess::DeniedByUser);
                return Err(CameraRefusal::Denied);
            }
            PortalAnswer::Failed => {
                self.access.set(CapabilityAccess::Unknown);
            }
            PortalAnswer::Unavailable => {}
        }
        // The fallbacks — only where the person refused nothing: the monitor's devices (PipeWire's
        // own default remote, or V4L2's provider), then a bare `v4l2src`.
        let devices = self.monitored();
        let pick = chosen
            .as_deref()
            .and_then(|id| devices.iter().find(|(found, _)| found.id == id))
            .or_else(|| devices.first());
        if let Some((_, device)) = pick
            && let Ok(element) = device.create_element(None)
        {
            self.access.set(CapabilityAccess::Allowed);
            return Ok(element);
        }
        if let Some(path) = first_video_device()
            && let Ok(element) = gst::ElementFactory::make("v4l2src")
                .property("device", path.to_string_lossy().as_ref())
                .build()
        {
            self.access.set(CapabilityAccess::Allowed);
            return Ok(element);
        }
        if self.portal_present.get() == Some(false) || devices.is_empty() {
            Err(CameraRefusal::Absent)
        } else {
            Err(CameraRefusal::Failed)
        }
    }
}

impl GstCamera {
    /// Stops the device monitor (quitting: a started provider must be stopped before it goes).
    pub fn stop_monitor(&self) {
        self.watch.borrow_mut().take();
        if let Some(monitor) = self.monitor.borrow_mut().take() {
            monitor.stop();
        }
    }
}

impl Drop for GstCamera {
    fn drop(&mut self) {
        self.stop_monitor();
    }
}

impl Camera for GstCamera {
    fn present(&self) -> bool {
        super::test_sources()
            || self.portal_present.get() == Some(true)
            || !self.monitored().is_empty()
            || first_video_device().is_some()
    }

    fn devices(&self) -> Vec<CameraDevice> {
        if super::test_sources() {
            return vec![CameraDevice {
                id: TEST_CAMERA_ID.to_string(),
                name: "videotestsrc".to_string(),
            }];
        }
        self.monitored()
            .into_iter()
            .map(|(device, _)| device)
            .collect()
    }

    fn on_devices_changed(&self) -> &Event<()> {
        &self.changed
    }

    fn open(
        &self,
        device: Option<String>,
    ) -> LocalBoxFuture<'static, Result<gst::Element, CameraRefusal>> {
        // The service is the shell's one camera, alive for the process: reach it again by `Rc`.
        match super::installed_camera() {
            Some(me) => Box::pin(me.open_camera(device)),
            None => Box::pin(std::future::ready(Err(CameraRefusal::Failed))),
        }
    }
}

/// A device as the settings and the recorder list it: a stable id (PipeWire's `node.name`, else a
/// V4L2 path, else its name) and the name it is shown by.
fn describe(device: &gst::Device) -> CameraDevice {
    let name = device.display_name().to_string();
    let id = device
        .properties()
        .and_then(|properties| {
            ["node.name", "api.v4l2.path", "device.path", "object.serial"]
                .iter()
                .find_map(|key| {
                    properties
                        .get::<String>(*key)
                        .ok()
                        .or_else(|| properties.get::<i64>(*key).ok().map(|n| n.to_string()))
                        .or_else(|| properties.get::<u64>(*key).ok().map(|n| n.to_string()))
                })
        })
        .unwrap_or_else(|| name.clone());
    CameraDevice { id, name }
}

/// The first `/dev/video*` this user may open (logind's uaccess), for the last fallback.
fn first_video_device() -> Option<std::path::PathBuf> {
    let mut found: Vec<std::path::PathBuf> = std::fs::read_dir("/dev")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("video"))
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

/// The self-test's camera: a moving picture as a bin with a ghost `src` pad.
fn test_camera() -> Option<gst::Element> {
    match gst::parse::bin_from_description(super::pipelines::TEST_CAMERA, true) {
        Ok(bin) => Some(bin.into()),
        Err(error) => {
            crate::fc_warn!(
                "camera: the test camera would not build ({})",
                super::gst_util::error_line(&error)
            );
            None
        }
    }
}

/// `pipewiresrc fd=… [target-object=…]`, owning the fd for as long as the element lives.
fn pipewire_source(fd: OwnedFd, target: Option<&str>) -> Option<gst::Element> {
    let mut builder = gst::ElementFactory::make("pipewiresrc").property("fd", fd.as_raw_fd());
    if let Some(target) = target {
        builder = builder.property("target-object", target);
    }
    let source = builder.build().ok()?;
    // The remote stays open exactly as long as the source: the closure owning the fd is dropped
    // when the element is finalised (from whichever thread lets go of it last — `OwnedFd` is
    // `Send + Sync`).
    source.connect_notify(Some("name"), move |_, _| {
        let _ = fd.as_raw_fd();
    });
    Some(source)
}

/// The portal's `IsCameraPresent`; `None` when there is no portal or no Camera interface.
async fn portal_present() -> Option<bool> {
    let bus = gio::bus_get_future(gio::BusType::Session).await.ok()?;
    let answer = bus
        .call_future(
            Some(PORTAL_BUS),
            PORTAL_PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(CAMERA_INTERFACE, "IsCameraPresent").to_variant()),
            Some(glib::VariantTy::new("(v)").ok()?),
            gio::DBusCallFlags::NO_AUTO_START,
            2_000,
        )
        .await
        .ok()?;
    answer.child_value(0).as_variant()?.get::<bool>()
}

/// `AccessCamera`, its Request's `Response`, then `OpenPipeWireRemote`.
async fn access_camera() -> PortalAnswer {
    let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else {
        return PortalAnswer::Unavailable;
    };
    match portal_present().await {
        None | Some(false) => return PortalAnswer::Unavailable,
        Some(true) => {}
    }
    // P5's Request/Response (`desktop::portal_request`): subscribed before the call, the request
    // closed if this is dropped while the dialog is up.
    let answer = portal_request::request(
        &bus,
        PORTAL_BUS,
        CAMERA_INTERFACE,
        "AccessCamera",
        |token| {
            let options = glib::VariantDict::new(None);
            options.insert("handle_token", token);
            glib::Variant::tuple_from_iter([options.end()])
        },
    )
    .await;
    match answer {
        Err(error) => {
            crate::fc_info!("camera: the portal refused the request ({})", error.kind());
            return PortalAnswer::Unavailable;
        }
        Ok(Response::Cancelled) => return PortalAnswer::Denied,
        Ok(Response::Ended) => return PortalAnswer::Failed,
        Ok(Response::Success(_)) => {}
    }
    let empty = glib::VariantDict::new(None);
    let opened = bus
        .call_with_unix_fd_list_future(
            Some(PORTAL_BUS),
            PORTAL_PATH,
            CAMERA_INTERFACE,
            "OpenPipeWireRemote",
            Some(&glib::Variant::tuple_from_iter([empty.end()])),
            None,
            gio::DBusCallFlags::NONE,
            -1,
            None::<&gio::UnixFDList>,
        )
        .await;
    let Ok((answer, Some(fds))) = opened else {
        return PortalAnswer::Failed;
    };
    let Some(index) = answer.child_value(0).get::<glib::variant::Handle>() else {
        return PortalAnswer::Failed;
    };
    match fds.get(index.0) {
        Ok(fd) => PortalAnswer::Remote(fd),
        Err(_) => PortalAnswer::Failed,
    }
}

/// The installed camera's `Rc` (kept by `media`).
pub fn remember(camera: &Rc<GstCamera>) {
    super::CAMERA.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(camera)));
}
