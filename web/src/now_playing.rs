//! What the app is playing, and what stops it (the plan for #79,
//! docs/audio-video-messages-2026-10-04.md, S5.3 and S4's last column).
//!
//! One thing plays at a time across the app: a voice note or a video message
//! starting pauses any other. Everything playing pauses when the tab is
//! hidden, when a call rings, starts or is placed, and when an output device
//! goes (headphones pulled — a voice note must not carry on out of the
//! laptop's speakers); it stops when the chat closes.
//!
//! "The app's playback" is every `<audio>` and `<video>` marked
//! [`PLAYBACK`]: a voice note in a bubble, a staged or not-sent note, a
//! circle, the viewer's video. A call's own `<audio>` and `<video>` are NOT
//! marked, and nothing here ever touches them — pausing the far end of a call
//! because a voice note started would be a call that went silent.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlMediaElement;
use yew::prelude::*;

/// The attribute that makes a media element the app's playback.
pub const PLAYBACK: &str = "data-playback";

/// Every element of the app's playback on the page.
fn players() -> Vec<HtmlMediaElement> {
    let Some(found) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector_all(&format!("[{PLAYBACK}]")).ok())
    else {
        return Vec::new();
    };
    (0..found.length())
        .filter_map(|index| found.item(index))
        .filter_map(|node| node.dyn_into::<HtmlMediaElement>().ok())
        .collect()
}

/// Everything of the app's playback paused but `except`.
pub fn pause_playback(except: Option<&HtmlMediaElement>) {
    for player in players() {
        if except.is_some_and(|keep| keep == &player) || player.paused() {
            continue;
        }
        let _ = player.pause();
    }
}

/// Whether a `devicechange` took an OUTPUT away, from the outputs listed
/// before it and after it — each a device id, `None` where the browser
/// would not say (Firefox and Safari list no outputs; a page without media
/// permission gets empty ids). Pauses unless it can SEE that every output
/// it had is still there: a voice note that carried on out of the speakers
/// after the headphones came out is the worse mistake, and a note paused
/// when a pair was plugged in costs a tap (S4: "going: pause; coming: plays
/// on").
pub fn output_went(before: Option<&[String]>, after: Option<&[String]>) -> bool {
    let readable = |list: Option<&[String]>| {
        list.filter(|ids| !ids.is_empty() && ids.iter().all(|id| !id.is_empty()))
            .map(<[String]>::to_vec)
    };
    match (readable(before), readable(after)) {
        (Some(before), Some(after)) => before.iter().any(|id| !after.contains(id)),
        _ => true,
    }
}

/// The audio outputs this browser lists now — `None` where it lists none.
async fn outputs_now() -> Option<Vec<String>> {
    #[cfg(test)]
    if let Some(listed) = testing::OUTPUTS.with(|outputs| outputs.borrow().clone()) {
        return listed;
    }
    let devices = web_sys::window()?.navigator().media_devices().ok()?;
    let listed = wasm_bindgen_futures::JsFuture::from(devices.enumerate_devices().ok()?)
        .await
        .ok()?;
    let ids: Vec<String> = js_sys::Array::from(&listed)
        .iter()
        .filter(|device| {
            js_sys::Reflect::get(device, &JsValue::from_str("kind"))
                .ok()
                .and_then(|kind| kind.as_string())
                .as_deref()
                == Some("audiooutput")
        })
        .map(|device| {
            js_sys::Reflect::get(&device, &JsValue::from_str("deviceId"))
                .ok()
                .and_then(|id| id.as_string())
                .unwrap_or_default()
        })
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// The app's playback, owned by the open chat: one thing at a time, paused
/// for a hidden tab, a call (`on_call`) and an output that went, and
/// stopped when the chat closes.
#[hook]
pub fn use_now_playing(on_call: bool) {
    // ONE AT A TIME: whichever of the app's players starts pauses the rest.
    // Heard on the way down, before the player hears its own `play`.
    use_effect_with((), move |_| {
        let one_at_a_time = Closure::<dyn Fn(web_sys::Event)>::new(|event: web_sys::Event| {
            let Some(player) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlMediaElement>().ok())
                .filter(|player| player.has_attribute(PLAYBACK))
            else {
                return;
            };
            pause_playback(Some(&player));
        });
        let hidden = Closure::<dyn Fn()>::new(|| {
            if !crate::sync::page_visible() {
                pause_playback(None);
            }
        });
        let window = web_sys::window();
        let document = window.as_ref().and_then(|window| window.document());
        if let Some(document) = &document {
            let _ = document.add_event_listener_with_callback_and_bool(
                "play",
                one_at_a_time.as_ref().unchecked_ref(),
                true,
            );
            let _ = document.add_event_listener_with_callback(
                "visibilitychange",
                hidden.as_ref().unchecked_ref(),
            );
        }
        // AN OUTPUT WENT: what the browser listed before the change is kept,
        // and compared with what it lists after.
        let devices = window
            .as_ref()
            .and_then(|window| window.navigator().media_devices().ok());
        let known: Rc<RefCell<Option<Vec<String>>>> = Rc::new(RefCell::new(None));
        let alive = Rc::new(RefCell::new(true));
        if devices.is_some() {
            let known = known.clone();
            let alive = alive.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let listed = outputs_now().await;
                if *alive.borrow() {
                    *known.borrow_mut() = listed;
                }
            });
        }
        let changed = {
            let known = known.clone();
            let alive = alive.clone();
            Closure::<dyn Fn()>::new(move || {
                let known = known.clone();
                let alive = alive.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let after = outputs_now().await;
                    if !*alive.borrow() {
                        return;
                    }
                    let before = known.borrow().clone();
                    if output_went(before.as_deref(), after.as_deref()) {
                        pause_playback(None);
                    }
                    *known.borrow_mut() = after;
                });
            })
        };
        if let Some(devices) = &devices {
            let _ = devices
                .add_event_listener_with_callback("devicechange", changed.as_ref().unchecked_ref());
        }
        move || {
            *alive.borrow_mut() = false;
            if let Some(document) = document {
                let _ = document.remove_event_listener_with_callback_and_bool(
                    "play",
                    one_at_a_time.as_ref().unchecked_ref(),
                    true,
                );
                let _ = document.remove_event_listener_with_callback(
                    "visibilitychange",
                    hidden.as_ref().unchecked_ref(),
                );
            }
            if let Some(devices) = devices {
                let _ = devices.remove_event_listener_with_callback(
                    "devicechange",
                    changed.as_ref().unchecked_ref(),
                );
            }
            // THE CHAT CLOSED: nothing it was playing plays on.
            pause_playback(None);
        }
    });
    // A CALL — ringing, placed or answered — pauses what plays (S4).
    use_effect_with(on_call, move |on_call| {
        if *on_call {
            pause_playback(None);
        }
    });
}

#[cfg(test)]
pub mod testing {
    use std::cell::RefCell;

    thread_local! {
        /// What `outputs_now` answers instead of asking the browser —
        /// `Some(None)` for a browser that lists no outputs.
        pub static OUTPUTS: RefCell<Option<Option<Vec<String>>>> = const { RefCell::new(None) };
    }

    /// The outputs the browser "lists", for as long as this lives.
    pub struct Outputs;

    impl Outputs {
        pub fn listing(ids: Option<&[&str]>) -> Outputs {
            OUTPUTS.with(|outputs| {
                *outputs.borrow_mut() =
                    Some(ids.map(|ids| ids.iter().map(|id| id.to_string()).collect()))
            });
            Outputs
        }

        pub fn now(&self, ids: Option<&[&str]>) {
            OUTPUTS.with(|outputs| {
                *outputs.borrow_mut() =
                    Some(ids.map(|ids| ids.iter().map(|id| id.to_string()).collect()))
            });
        }
    }

    impl Drop for Outputs {
        fn drop(&mut self) {
            OUTPUTS.with(|outputs| *outputs.borrow_mut() = None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| id.to_string()).collect()
    }

    /// AN OUTPUT GOING PAUSES; ONE COMING PLAYS ON — where the browser lets
    /// it be told; where it does not, it pauses (S4).
    #[wasm_bindgen_test]
    fn an_output_that_went_is_told_from_one_that_came() {
        let two = ids(&["speakers", "headphones"]);
        let one = ids(&["speakers"]);
        assert!(output_went(Some(&two), Some(&one)), "headphones out");
        assert!(!output_went(Some(&one), Some(&two)), "headphones in");
        assert!(!output_went(Some(&two), Some(&two)), "a microphone changed");
        assert!(output_went(None, Some(&two)), "nothing known before");
        assert!(output_went(Some(&two), None), "nothing listed after");
        let blank = ids(&[""]);
        assert!(output_went(Some(&blank), Some(&blank)), "ids withheld");
        assert!(output_went(Some(&[]), Some(&[])), "no outputs listed");
    }
}
