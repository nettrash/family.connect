//! Where this browser is, once — the web's LocationProvider.
//!
//! The apps' bar, which the protocol makes everyone's (docs/protocol.md,
//! "Locations"): a fix older than two minutes is never sent, however quickly
//! the browser hands it over; 100 m is good enough to send at once; and
//! after twenty seconds the best FRESH fix seen goes, coarse or not, with
//! its accuracy saying so honestly. Nothing fresh in twenty seconds is "could
//! not find", never a stale answer.
//!
//! The twenty seconds are for LOOKING, not for somebody reading the
//! browser's permission prompt: the clock starts once the browser may look
//! (the Mac settles permission before it starts hunting, #41).

use std::cell::RefCell;
use std::rc::Rc;

use futures::channel::oneshot;
use futures::future::{select, Either};
use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
// The stable names for GeolocationPosition and its kin: web-sys keeps the
// new ones behind its unstable flag, and the objects are the same.
use web_sys::{Position, PositionError, PositionOptions};

/// Good enough to stop looking, in metres.
pub const GOOD_ENOUGH_M: f64 = 100.0;

/// The oldest fix that may be sent, in milliseconds.
pub const FRESH_ENOUGH_MS: f64 = 2.0 * 60.0 * 1000.0;

/// How long to look, in milliseconds.
pub const TIMEOUT_MS: u32 = 20_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_m: Option<f64>,
    /// When the browser took it, in wall-clock milliseconds.
    pub taken_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationError {
    Denied,
    NotFound,
}

impl LocationError {
    pub fn message(self) -> &'static str {
        match self {
            LocationError::Denied => {
                "Family needs permission to use your location. Allow it in your browser's settings for this site."
            }
            LocationError::NotFound => "Could not find your location.",
        }
    }
}

/// What a fix that just arrived means for the wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Fresh and good enough: send it.
    Accept,
    /// Fresh but coarse: keep it, in case nothing better comes.
    Hold,
    /// Too old to send at all.
    Stale,
}

/// The rule, apart from any browser: how old, and how good.
pub fn judge(fix: &Fix, now_ms: f64) -> Verdict {
    if now_ms - fix.taken_ms > FRESH_ENOUGH_MS {
        return Verdict::Stale;
    }
    match fix.accuracy_m {
        Some(accuracy) if accuracy <= GOOD_ENOUGH_M => Verdict::Accept,
        _ => Verdict::Hold,
    }
}

/// The better of two fresh fixes: the smaller circle, and a known one over
/// an unknown one.
pub fn better(held: Option<Fix>, fix: Fix) -> Fix {
    match held {
        None => fix,
        Some(held) => match (held.accuracy_m, fix.accuracy_m) {
            (Some(was), Some(now)) if now < was => fix,
            (None, Some(_)) => fix,
            _ => held,
        },
    }
}

enum Event {
    Fix(Fix),
    Denied,
    /// Some other answer — a timeout, no position — which at least says
    /// the prompt is over.
    Answered,
}

/// What the browser says about asking for a location, before asking.
async fn permission() -> Option<String> {
    let navigator = web_sys::window()?.navigator();
    let permissions = js_sys::Reflect::get(&navigator, &JsValue::from_str("permissions")).ok()?;
    let query = js_sys::Reflect::get(&permissions, &JsValue::from_str("query")).ok()?;
    let query: js_sys::Function = query.dyn_into().ok()?;
    let descriptor = js_sys::Object::new();
    js_sys::Reflect::set(
        &descriptor,
        &JsValue::from_str("name"),
        &JsValue::from_str("geolocation"),
    )
    .ok()?;
    let promise: js_sys::Promise = query
        .call1(&permissions, &descriptor)
        .ok()?
        .dyn_into()
        .ok()?;
    let status = wasm_bindgen_futures::JsFuture::from(promise).await.ok()?;
    js_sys::Reflect::get(&status, &JsValue::from_str("state"))
        .ok()?
        .as_string()
}

/// One reading, by the rules above. `permitted` hears once the browser may
/// look — at once when it already could, or when its first answer arrives —
/// which is when the hunt, and its clock, begin.
pub async fn current_fix(permitted: yew::Callback<()>) -> Result<Fix, LocationError> {
    let geolocation = web_sys::window()
        .ok_or(LocationError::NotFound)?
        .navigator()
        .geolocation()
        .map_err(|_| LocationError::NotFound)?;
    let state = permission().await;
    if state.as_deref() == Some("denied") {
        return Err(LocationError::Denied);
    }
    // Asked already, or a browser that will not say: the clock starts now.
    // Otherwise it starts with the browser's first answer.
    let asked = matches!(state.as_deref(), Some("granted") | None);
    let (sender, mut receiver) = futures::channel::mpsc::unbounded::<Event>();
    let on_fix = {
        let sender = sender.clone();
        Closure::<dyn FnMut(Position)>::new(move |position: Position| {
            let coords = position.coords();
            let accuracy = coords.accuracy();
            let _ = sender.unbounded_send(Event::Fix(Fix {
                latitude: coords.latitude(),
                longitude: coords.longitude(),
                accuracy_m: (accuracy.is_finite() && accuracy >= 0.0).then_some(accuracy),
                taken_ms: position.timestamp(),
            }));
        })
    };
    let on_error = Closure::<dyn FnMut(PositionError)>::new(move |error: PositionError| {
        let _ = sender.unbounded_send(if error.code() == PositionError::PERMISSION_DENIED {
            Event::Denied
        } else {
            Event::Answered
        });
    });
    let options = PositionOptions::new();
    options.set_enable_high_accuracy(true);
    options.set_maximum_age(FRESH_ENOUGH_MS as u32);
    options.set_timeout(TIMEOUT_MS);
    let watch = geolocation
        .watch_position_with_error_callback_and_options(
            on_fix.as_ref().unchecked_ref(),
            Some(on_error.as_ref().unchecked_ref()),
            &options,
        )
        .map_err(|_| LocationError::NotFound)?;

    let (stop, stopped) = oneshot::channel::<()>();
    let stop = Rc::new(RefCell::new(Some(stop)));
    let start_clock = {
        let stop = stop.clone();
        let started = Rc::new(std::cell::Cell::new(false));
        move || {
            if started.replace(true) {
                return;
            }
            permitted.emit(());
            let stop = stop.clone();
            wasm_bindgen_futures::spawn_local(async move {
                TimeoutFuture::new(TIMEOUT_MS).await;
                if let Some(stop) = stop.borrow_mut().take() {
                    let _ = stop.send(());
                }
            });
        }
    };
    if asked {
        start_clock();
    }

    let mut best: Option<Fix> = None;
    let mut stopped = stopped;
    let outcome = loop {
        use futures::StreamExt;
        match select(receiver.next(), stopped).await {
            Either::Left((Some(Event::Fix(fix)), still)) => {
                stopped = still;
                start_clock();
                match judge(&fix, js_sys::Date::now()) {
                    Verdict::Accept => break Ok(fix),
                    Verdict::Hold => best = Some(better(best, fix)),
                    Verdict::Stale => {}
                }
            }
            Either::Left((Some(Event::Denied), _)) => break Err(LocationError::Denied),
            Either::Left((Some(Event::Answered), still)) => {
                stopped = still;
                start_clock();
            }
            Either::Left((None, _)) | Either::Right(_) => {
                break best.ok_or(LocationError::NotFound);
            }
        }
    };
    geolocation.clear_watch(watch);
    drop(on_fix);
    drop(on_error);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn fix(accuracy: Option<f64>, age_ms: f64) -> Fix {
        Fix {
            latitude: 55.75,
            longitude: 37.61,
            accuracy_m: accuracy,
            taken_ms: 1_000_000.0 - age_ms,
        }
    }

    #[wasm_bindgen_test]
    fn a_fix_is_sent_only_while_it_is_fresh() {
        let now = 1_000_000.0;
        assert_eq!(judge(&fix(Some(12.0), 1_000.0), now), Verdict::Accept);
        assert_eq!(judge(&fix(Some(100.0), 0.0), now), Verdict::Accept);
        assert_eq!(
            judge(&fix(Some(101.0), 0.0), now),
            Verdict::Hold,
            "coarse is held, not refused"
        );
        assert_eq!(judge(&fix(None, 0.0), now), Verdict::Hold);
        assert_eq!(
            judge(&fix(Some(5.0), FRESH_ENOUGH_MS + 1.0), now),
            Verdict::Stale,
            "precise but old is precisely wrong"
        );
    }

    #[wasm_bindgen_test]
    fn the_best_fresh_fix_is_the_smallest_circle() {
        let coarse = fix(Some(900.0), 0.0);
        let finer = fix(Some(300.0), 0.0);
        assert_eq!(better(None, coarse), coarse);
        assert_eq!(better(Some(coarse), finer), finer);
        assert_eq!(better(Some(finer), coarse), finer);
        assert_eq!(
            better(Some(fix(None, 0.0)), coarse),
            coarse,
            "known beats unknown"
        );
        assert_eq!(better(Some(coarse), fix(None, 0.0)), coarse);
    }
}
