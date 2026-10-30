//! The default output device changing — headphones unplugged, a Bluetooth headset gone or come
//! (S4, "a change of the default output device on desktops"; the Mac's `watchTheDefaultOutput`):
//! what plays pauses where `playback_pauses` says (`PlaybackEvent::OutputChanged`), rather than
//! carrying on out of a speaker in a quiet room.
//!
//! A `gst::DeviceMonitor` over the audio sinks, and which of them says `is-default` (PipeWire's and
//! PulseAudio's providers both set it): the interruption is raised only when THAT changes — a
//! volume change, or a sink coming that does not become the default, pauses nothing.

use std::cell::RefCell;

use gst::prelude::{DeviceExt, DeviceMonitorExt, DeviceMonitorExtManual};

use crate::seams::media::NowPlaying;

/// The monitor and what it last saw, for as long as the app runs.
struct Watch {
    monitor: gst::DeviceMonitor,
    default: RefCell<Option<String>>,
    _bus: gst::bus::BusWatchGuard,
}

thread_local! {
    static WATCH: RefCell<Option<std::rc::Rc<Watch>>> = const { RefCell::new(None) };
}

/// Starts watching (`media::install`). Without a device provider nothing is watched, and nothing
/// pauses for it.
pub fn start() {
    let monitor = gst::DeviceMonitor::new();
    if monitor.add_filter(Some("Audio/Sink"), None).is_none() {
        return;
    }
    let bus = monitor.bus();
    let Ok(guard) = bus.add_watch_local(move |_, message| {
        if matches!(
            message.view(),
            gst::MessageView::DeviceAdded(_)
                | gst::MessageView::DeviceRemoved(_)
                | gst::MessageView::DeviceChanged(_)
        ) {
            changed();
        }
        glib::ControlFlow::Continue
    }) else {
        return;
    };
    if monitor.start().is_err() {
        crate::fc_info!("output: no device provider started");
        return;
    }
    let watch = std::rc::Rc::new(Watch {
        default: RefCell::new(default_of(&monitor)),
        monitor,
        _bus: guard,
    });
    WATCH.with(|held| *held.borrow_mut() = Some(watch));
}

/// Stops it (on the way out).
pub fn stop() {
    if let Some(watch) = WATCH.with(|held| held.borrow_mut().take()) {
        watch.monitor.stop();
    }
}

/// A sink came, went or changed: when the default is now another one, what plays pauses.
fn changed() {
    let Some(watch) = WATCH.with(|held| held.borrow().clone()) else {
        return;
    };
    let now = default_of(&watch.monitor);
    let before = watch.default.replace(now.clone());
    if before.is_some() && now != before {
        crate::fc_info!("output: the default output changed; pausing what plays");
        super::now_playing().interrupt(fc_logic::round_look::PlaybackEvent::OutputChanged);
    }
}

/// The sink marked default, by its node name (its display name where it has none).
fn default_of(monitor: &gst::DeviceMonitor) -> Option<String> {
    monitor.devices().into_iter().find_map(|device| {
        let properties = device.properties()?;
        if !properties.get::<bool>("is-default").unwrap_or(false) {
            return None;
        }
        Some(
            properties
                .get::<String>("node.name")
                .unwrap_or_else(|_| device.display_name().to_string()),
        )
    })
}
