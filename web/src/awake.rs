//! Keeping the screen on while somebody records (the plan for #79,
//! docs/audio-video-messages-2026-10-04.md, S1.7): a phone that locks itself
//! halfway through a long story hides the tab, a hidden tab stops the
//! recording (S4), and the story becomes a "Voice message not sent" row
//! nobody asked for.
//!
//! `navigator.wakeLock` where the browser has one, nothing where it has not:
//! asked, never assumed, and never a reason for anything else not to work. A
//! browser grants it only to a page that is visible, and takes it back by
//! itself when the page is hidden — the moment a recording stops anyway. It
//! is the same call a phone's browser needs for playback (S1.7), which is
//! why it is a piece of its own.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};

/// The screen, kept on for as long as this lives.
pub struct ScreenAwake(Rc<RefCell<Hold>>);

enum Hold {
    /// Asked for, and not answered yet.
    Asking,
    /// Granted: the browser's `WakeLockSentinel`.
    Held(JsValue),
    /// Let go of — and a grant that arrives after this is let go of at once,
    /// or the screen would stay on for a recording that has ended.
    Done,
}

impl ScreenAwake {
    pub fn hold() -> ScreenAwake {
        let hold = Rc::new(RefCell::new(Hold::Asking));
        if let Some(asked) = request() {
            let hold = hold.clone();
            spawn_local(async move {
                // Refused — a hidden page, a policy that forbids it — is
                // simply not held.
                let Ok(sentinel) = JsFuture::from(asked).await else {
                    return;
                };
                let mut held = hold.borrow_mut();
                if matches!(*held, Hold::Done) {
                    release(&sentinel);
                } else {
                    *held = Hold::Held(sentinel);
                }
            });
        }
        ScreenAwake(hold)
    }
}

impl Drop for ScreenAwake {
    fn drop(&mut self) {
        if let Hold::Held(sentinel) = std::mem::replace(&mut *self.0.borrow_mut(), Hold::Done) {
            release(&sentinel);
        }
    }
}

/// `navigator.wakeLock.request("screen")`, where there is a `wakeLock` to
/// ask — looked up rather than bound, because a browser without one has no
/// such property at all.
fn request() -> Option<js_sys::Promise> {
    let navigator = web_sys::window()?.navigator();
    let lock = js_sys::Reflect::get(&navigator, &JsValue::from_str("wakeLock")).ok()?;
    if lock.is_undefined() || lock.is_null() {
        return None;
    }
    let request: js_sys::Function = js_sys::Reflect::get(&lock, &JsValue::from_str("request"))
        .ok()?
        .dyn_into()
        .ok()?;
    request
        .call1(&lock, &JsValue::from_str("screen"))
        .ok()?
        .dyn_into::<js_sys::Promise>()
        .ok()
}

fn release(sentinel: &JsValue) {
    let Some(release) = js_sys::Reflect::get(sentinel, &JsValue::from_str("release"))
        .ok()
        .and_then(|release| release.dyn_into::<js_sys::Function>().ok())
    else {
        return;
    };
    if let Ok(Ok(done)) = release
        .call0(sentinel)
        .map(|done| done.dyn_into::<js_sys::Promise>())
    {
        // Waited on, so a refusal is not reported as one nobody handled.
        spawn_local(async move {
            let _ = JsFuture::from(done).await;
        });
    }
}

#[cfg(test)]
pub mod testing {
    use wasm_bindgen::JsValue;

    /// A `navigator.wakeLock` that counts what it is asked, standing in for
    /// the browser's for as long as this lives. Its grants are numbered;
    /// `asked()` and `released()` say how many of each there have been.
    pub struct FakeWakeLock(JsValue);

    impl FakeWakeLock {
        pub fn install() -> FakeWakeLock {
            let fake = js_sys::Function::new_no_args(
                "const fake = { asked: 0, released: 0, types: [], \
                   request(type) { \
                     this.asked += 1; this.types.push(type); \
                     return Promise.resolve({ release: () => { fake.released += 1; \
                                                              return Promise.resolve(); } }); \
                   } }; \
                 Object.defineProperty(navigator, 'wakeLock', \
                   { value: fake, configurable: true }); \
                 return fake;",
            )
            .call0(&JsValue::NULL)
            .expect("standing in for navigator.wakeLock");
            FakeWakeLock(fake)
        }

        fn count(&self, name: &str) -> u32 {
            js_sys::Reflect::get(&self.0, &JsValue::from_str(name))
                .ok()
                .and_then(|count| count.as_f64())
                .unwrap_or(0.0) as u32
        }

        pub fn asked(&self) -> u32 {
            self.count("asked")
        }

        pub fn released(&self) -> u32 {
            self.count("released")
        }

        /// What each request asked for — "screen", every time.
        pub fn types(&self) -> Vec<String> {
            js_sys::Reflect::get(&self.0, &JsValue::from_str("types"))
                .ok()
                .map(|types| {
                    js_sys::Array::from(&types)
                        .iter()
                        .filter_map(|kind| kind.as_string())
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    impl Drop for FakeWakeLock {
        fn drop(&mut self) {
            let _ =
                js_sys::Function::new_no_args("delete navigator.wakeLock;").call0(&JsValue::NULL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::FakeWakeLock;
    use super::*;
    use gloo_timers::future::TimeoutFuture;
    use wasm_bindgen_test::*;

    /// Held for as long as the guard lives, for the SCREEN, and let go of
    /// when it goes — including when it goes before the browser has
    /// answered, which must not leave the screen on after all.
    #[wasm_bindgen_test]
    async fn the_screen_stays_on_while_held_and_is_let_go_after() {
        let fake = FakeWakeLock::install();
        let awake = ScreenAwake::hold();
        TimeoutFuture::new(20).await;
        assert_eq!((fake.asked(), fake.released()), (1, 0));
        assert_eq!(fake.types(), vec!["screen".to_string()]);
        drop(awake);
        TimeoutFuture::new(20).await;
        assert_eq!(fake.released(), 1);

        // Gone before the grant arrived: the grant is let go of on arrival.
        drop(ScreenAwake::hold());
        TimeoutFuture::new(20).await;
        assert_eq!((fake.asked(), fake.released()), (2, 2));
    }

    /// A browser with no wake lock is asked nothing, and one that refuses
    /// leaves nothing held — no panic, and nothing to let go of later.
    #[wasm_bindgen_test]
    async fn without_a_wake_lock_or_with_one_refused_nothing_is_held() {
        let stand_in = |value: &str| {
            js_sys::Function::new_no_args(&format!(
                "Object.defineProperty(navigator, 'wakeLock', \
                   {{ value: {value}, configurable: true }});"
            ))
            .call0(&JsValue::NULL)
            .expect("standing in for navigator.wakeLock");
        };
        stand_in("undefined");
        assert!(request().is_none(), "nothing to ask");
        drop(ScreenAwake::hold());

        stand_in("{ request: () => Promise.reject(new DOMException('no', 'NotAllowedError')) }");
        let asked = request().expect("asked");
        assert!(JsFuture::from(asked).await.is_err(), "and refused");
        let refused = ScreenAwake::hold();
        TimeoutFuture::new(20).await;
        assert!(
            matches!(*refused.0.borrow(), Hold::Asking),
            "refused, nothing is held"
        );
        drop(refused);
        let _ = js_sys::Function::new_no_args("delete navigator.wakeLock;").call0(&JsValue::NULL);
    }
}
