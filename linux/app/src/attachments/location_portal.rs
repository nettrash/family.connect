//! "Where I am, once" (shell-design §8.5): the XDG Location portal over GDBus —
//! `org.freedesktop.portal.Location.CreateSession` (accuracy EXACT), `Start`, the first
//! `LocationUpdated`, then `Session.Close`. P1's paperclip offers "Location" only when
//! [`available`] says the portal answers its `version`, and sends what [`find`] answers
//! (`AttachItem::Location`).
//!
//! What a fix is good enough for is `fc_core::text::location_fixes` (the C#'s `LocationFixes`): a
//! fix within 100 m is taken at once, a worse one is kept while a better is awaited, and after 20
//! seconds the best so far is the answer. The person (or GNOME for them) may refuse: that is an
//! answer too, never a retry.
//!
//! Nothing here is logged but the portal's response code: never a coordinate.
//! (Unverified — real desktop, §C 39: whether GeoClue on a stock desktop without GPS gives a fix.)

use std::cell::RefCell;
use std::rc::Rc;

use fc_core::text::location_fixes::{self, LocationFix, LocationHunt};
use futures_channel::mpsc;
use futures_util::StreamExt;
use futures_util::future::{Either, select};
use glib::prelude::*;

const BUS_NAME: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const LOCATION: &str = "org.freedesktop.portal.Location";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
/// The portal's `accuracy` for "exact".
const ACCURACY_EXACT: u32 = 5;
/// A D-Bus call's own limit, ms.
const CALL_LIMIT_MS: i32 = 10_000;

/// Why no place came back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No portal, or it has no Location interface.
    Unavailable,
    /// The person, or the desktop for them, said no.
    Denied,
    /// No fix in time, or the portal failed.
    NoFix,
}

/// Whether the Location portal answers (its `version` property).
pub async fn available() -> bool {
    let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else {
        return false;
    };
    let answer = bus
        .call_future(
            Some(BUS_NAME),
            PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(LOCATION, "version").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            CALL_LIMIT_MS,
        )
        .await;
    answer.is_ok()
}

/// A token for a request or a session handle: letters and digits only (the portal's rule).
fn token(kind: &str) -> String {
    format!("fc_{kind}_{}", glib::random_int())
}

/// The object path a handle `token` will have for this connection.
fn handle_path(bus: &gio::DBusConnection, kind: &str, token: &str) -> Option<String> {
    let sender = bus.unique_name()?;
    let sender = sender.trim_start_matches(':').replace('.', "_");
    Some(format!("{PATH}/{kind}/{sender}/{token}"))
}

/// Asks the portal where this device is, once (see the module's comment). `parent` is the
/// portal's parent-window string (empty when none).
pub async fn find(parent: &str) -> Result<LocationFix, Refusal> {
    let bus = gio::bus_get_future(gio::BusType::Session)
        .await
        .map_err(|_| Refusal::Unavailable)?;
    let session_token = token("session");
    let session_path = handle_path(&bus, "session", &session_token).ok_or(Refusal::Unavailable)?;

    let (fixes, mut heard) = mpsc::unbounded::<LocationFix>();
    let fixes = Rc::new(RefCell::new(Some(fixes)));
    let _updates = {
        let fixes = Rc::clone(&fixes);
        bus.subscribe_to_signal(
            Some(BUS_NAME),
            Some(LOCATION),
            Some("LocationUpdated"),
            Some(PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if let Some(fix) = read_fix(signal.parameters)
                    && let Some(sender) = fixes.borrow().as_ref()
                {
                    let _ = sender.unbounded_send(fix);
                }
            },
        )
    };

    let options = glib::VariantDict::new(None);
    options.insert("session_handle_token", &session_token);
    options.insert("distance-threshold", 0u32);
    options.insert("time-threshold", 0u32);
    options.insert("accuracy", ACCURACY_EXACT);
    bus.call_future(
        Some(BUS_NAME),
        PATH,
        LOCATION,
        "CreateSession",
        Some(&(options.end(),).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        CALL_LIMIT_MS,
    )
    .await
    .map_err(|_| Refusal::Unavailable)?;

    let result = start_and_wait(&bus, &session_path, parent, &mut heard).await;
    // Done either way: the session goes, and the GPS with it.
    let _ = bus
        .call_future(
            Some(BUS_NAME),
            &session_path,
            SESSION,
            "Close",
            None,
            None,
            gio::DBusCallFlags::NONE,
            CALL_LIMIT_MS,
        )
        .await;
    fixes.borrow_mut().take();
    result
}

async fn start_and_wait(
    bus: &gio::DBusConnection,
    session_path: &str,
    parent: &str,
    heard: &mut mpsc::UnboundedReceiver<LocationFix>,
) -> Result<LocationFix, Refusal> {
    let request_token = token("request");
    let request_path = handle_path(bus, "request", &request_token).ok_or(Refusal::Unavailable)?;
    let (responses, mut answered) = mpsc::unbounded::<u32>();
    let _response = bus.subscribe_to_signal(
        Some(BUS_NAME),
        Some(REQUEST),
        Some("Response"),
        Some(&request_path),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            if let Some((code, _)) = signal
                .parameters
                .get::<(u32, std::collections::HashMap<String, glib::Variant>)>()
            {
                let _ = responses.unbounded_send(code);
            }
        },
    );
    let options = glib::VariantDict::new(None);
    options.insert("handle_token", &request_token);
    let session = glib::variant::ObjectPath::try_from(session_path.to_string())
        .map_err(|_| Refusal::Unavailable)?;
    bus.call_future(
        Some(BUS_NAME),
        PATH,
        LOCATION,
        "Start",
        Some(&(session, parent, options.end()).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        CALL_LIMIT_MS,
    )
    .await
    .map_err(|_| Refusal::NoFix)?;

    let hunt = LocationHunt::new();
    let waiting = async {
        loop {
            // Either the request's answer or a fix; a dropped `next()` loses nothing.
            match select(answered.next(), heard.next()).await {
                Either::Left((Some(0), _)) => {}
                Either::Left((Some(code), _)) => {
                    crate::fc_info!("location: the portal answered {code}");
                    return Err(if code == 1 {
                        Refusal::Denied
                    } else {
                        Refusal::NoFix
                    });
                }
                Either::Left((None, _)) | Either::Right((None, _)) => return Err(Refusal::NoFix),
                Either::Right((Some(fix), _)) => {
                    let now = time::OffsetDateTime::now_utc();
                    if let Some(taken) = hunt.offer(fix, now) {
                        return Ok(taken);
                    }
                }
            }
        }
    };
    match glib::future_with_timeout(location_fixes::TIMEOUT, Box::pin(waiting)).await {
        Ok(answer) => answer,
        // Out of time: the best fix so far, if there was one.
        Err(_) => hunt.best().ok_or(Refusal::NoFix),
    }
}

/// `LocationUpdated(o session, a{sv} location)` read into a fix: `Latitude`, `Longitude` and
/// `Accuracy` (metres), stamped now.
fn read_fix(parameters: &glib::Variant) -> Option<LocationFix> {
    let (_, location) = parameters.get::<(
        glib::variant::ObjectPath,
        std::collections::HashMap<String, glib::Variant>,
    )>()?;
    let number = |name: &str| location.get(name).and_then(|value| value.get::<f64>());
    Some(LocationFix {
        latitude: number("Latitude")?,
        longitude: number("Longitude")?,
        accuracy_m: number("Accuracy").and_then(location_fixes::usable_accuracy),
        taken: time::OffsetDateTime::now_utc(),
    })
}
