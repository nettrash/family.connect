//! Attachment bytes, in this tab's memory.
//!
//! An `<img>` cannot send the session's token and a token in a URL is
//! refused, so every picture, poster, recording and file is fetched with
//! `fetch` and drawn from an object URL (docs/protocol.md, "A browser is a
//! client too"). This is where those URLs live: one per attachment and
//! variant, fetched once however many bubbles ask, bounded so a long
//! session does not hold every photo it ever scrolled past, and all of them
//! let go at sign-out.
//!
//! Two kinds of id are never fetched. A PROVISIONAL id (negative) is an
//! attachment still in the outbox, drawn from the bytes the store holds for
//! it; and a server id this device uploaded is drawn from the same bytes
//! through `Store::aliases`, rather than fetching back what it just sent.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen_futures::spawn_local;
use web_sys::{Blob, Url};
use yew::prelude::*;

use crate::api;
use crate::live::Live;

/// Which bytes of an attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variant {
    /// The small JPEG: a photo's preview, a video's poster.
    Preview,
    /// The file itself.
    Original,
}

/// How many previews are held. A preview is a few tens of kilobytes, so
/// this is a few megabytes — and far more than one screen of a chat.
pub const PREVIEW_CAP: usize = 400;

/// How many originals are held. An original may be a hundred-megabyte
/// video, so only the handful just looked at.
pub const ORIGINAL_CAP: usize = 16;

struct Entry {
    url: String,
    /// Kept so a second key for the same bytes — an upload's server id —
    /// can have a URL of its own, and so Save has the bytes.
    blob: Blob,
    used: u64,
}

#[derive(Default)]
struct Cache {
    /// The session the entries belong to.
    session: u64,
    entries: HashMap<(i64, Variant), Entry>,
    tick: u64,
    /// Fetches in flight, and who is waiting for each.
    waiting: HashMap<(i64, Variant), Vec<Callback<Option<String>>>>,
}

impl Cache {
    fn revoke_all(&mut self) {
        for (_, entry) in self.entries.drain() {
            let _ = Url::revoke_object_url(&entry.url);
        }
        self.waiting.clear();
    }

    fn touch(&mut self, key: (i64, Variant)) -> Option<String> {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(&key).map(|entry| {
            entry.used = tick;
            entry.url.clone()
        })
    }

    fn insert(&mut self, key: (i64, Variant), blob: Blob) -> Option<String> {
        let url = Url::create_object_url_with_blob(&blob).ok()?;
        self.tick += 1;
        if let Some(old) = self.entries.insert(
            key,
            Entry {
                url: url.clone(),
                blob,
                used: self.tick,
            },
        ) {
            let _ = Url::revoke_object_url(&old.url);
        }
        let cap = match key.1 {
            Variant::Preview => PREVIEW_CAP,
            Variant::Original => ORIGINAL_CAP,
        };
        while self.entries.keys().filter(|held| held.1 == key.1).count() > cap {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(held, _)| held.1 == key.1 && **held != key)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(held, _)| *held)
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                let _ = Url::revoke_object_url(&entry.url);
            }
        }
        Some(url)
    }
}

/// The app's one media cache, handed to the views as a context.
#[derive(Clone)]
pub struct MediaLoader {
    live: Live,
    cache: Rc<RefCell<Cache>>,
}

impl PartialEq for MediaLoader {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.cache, &other.cache)
    }
}

impl MediaLoader {
    pub fn new(live: Live) -> Self {
        let session = live.session();
        MediaLoader {
            live,
            cache: Rc::new(RefCell::new(Cache {
                session,
                ..Cache::default()
            })),
        }
    }

    /// Everything held belongs to the session in front of the reader, or
    /// goes: one person's photos never draw in the next person's tab.
    fn fresh(&self) {
        let session = self.live.session();
        let mut cache = self.cache.borrow_mut();
        if cache.session != session {
            cache.revoke_all();
            cache.session = session;
        }
    }

    /// Let go of everything — at sign-out.
    pub fn clear(&self) {
        self.cache.borrow_mut().revoke_all();
    }

    /// The URL for these bytes, if this tab already has them — from the
    /// cache, from an upload this device made, or from the outbox.
    pub fn get(&self, id: i64, variant: Variant) -> Option<String> {
        self.fresh();
        if let Some(url) = self.cache.borrow_mut().touch((id, variant)) {
            return Some(url);
        }
        // A server id this device uploaded: the bytes its bubble drew from.
        let provisional = self
            .live
            .read(|state| state.store.aliases.get(&id).copied());
        if let Some(provisional) = provisional {
            if let Some(blob) = self.held(provisional, variant) {
                return self.cache.borrow_mut().insert((id, variant), blob);
            }
        }
        if id < 0 {
            let blob = self.live.read(|state| {
                state.store.bytes.get(&id).and_then(|bytes| match variant {
                    Variant::Preview => bytes.preview.clone(),
                    Variant::Original => bytes.file.clone(),
                })
            });
            return blob.and_then(|blob| self.cache.borrow_mut().insert((id, variant), blob));
        }
        None
    }

    /// Bytes this tab already has under a server id — a picture it has just
    /// pinned itself — so drawing it does not fetch back what was sent.
    /// Never over bytes already held: their URL may be on screen.
    pub fn seed(&self, id: i64, variant: Variant, blob: Blob) {
        self.fresh();
        let mut cache = self.cache.borrow_mut();
        if !cache.entries.contains_key(&(id, variant)) {
            let _ = cache.insert((id, variant), blob);
        }
    }

    /// The bytes behind a cached entry, or behind a provisional id.
    fn held(&self, id: i64, variant: Variant) -> Option<Blob> {
        if let Some(entry) = self.cache.borrow().entries.get(&(id, variant)) {
            return Some(entry.blob.clone());
        }
        self.live.read(|state| {
            state.store.bytes.get(&id).and_then(|bytes| match variant {
                Variant::Preview => bytes.preview.clone(),
                Variant::Original => bytes.file.clone(),
            })
        })
    }

    /// The URL for these bytes, fetching them if this tab has none; `done`
    /// hears once, with None when they could not be had. Fetches of the
    /// same bytes are shared.
    pub fn load(&self, id: i64, variant: Variant, done: Callback<Option<String>>) {
        if let Some(url) = self.get(id, variant) {
            done.emit(Some(url));
            return;
        }
        // An outbox id whose bytes are gone — the message was sent, and
        // they went with its row: fetched under the id the server gave it.
        if id < 0 {
            let landed = self.live.read(|state| {
                state
                    .store
                    .aliases
                    .iter()
                    .find(|(_, provisional)| **provisional == id)
                    .map(|(real, _)| *real)
            });
            match landed {
                Some(real) => self.load(real, variant, done),
                None => done.emit(None),
            }
            return;
        }
        if id == 0 {
            done.emit(None);
            return;
        }
        let key = (id, variant);
        let first = {
            let mut cache = self.cache.borrow_mut();
            let waiting = cache.waiting.entry(key).or_default();
            waiting.push(done);
            waiting.len() == 1
        };
        if !first {
            return;
        }
        let (session, token) = self.live.read(|state| (state.session, state.token.clone()));
        let live = self.live.clone();
        let cache = self.cache.clone();
        spawn_local(async move {
            let fetched = match token {
                Some(token) => api::attachment_bytes(&token, id, variant == Variant::Preview)
                    .await
                    .ok(),
                None => None,
            };
            let url = fetched
                .filter(|_| live.is_live(session))
                .and_then(|blob| cache.borrow_mut().insert(key, blob));
            let waiters = cache.borrow_mut().waiting.remove(&key).unwrap_or_default();
            for waiter in waiters {
                waiter.emit(url.clone());
            }
        });
    }
}

/// How long after a failed fetch the next try waits, by try — then no more
/// tries until the bubble is drawn again. A flaky network gets a few
/// chances; a server that has lost the bytes does not get hammered.
pub const RETRY_MS: [u32; 3] = [2_000, 8_000, 30_000];

/// The URL to draw these bytes from — at once when this tab has them,
/// and once they have been fetched otherwise. `wanted` false asks for
/// nothing: a hidden row, a video whose tile has no poster. A failed fetch
/// is tried again, a few times, while the bubble is on screen.
#[hook]
pub fn use_media(id: i64, variant: Variant, wanted: bool) -> Option<String> {
    let loader = use_context::<MediaLoader>();
    let fetched = use_state(|| Option::<(i64, Variant, String)>::None);
    let tries = use_state(|| 0usize);
    let current = (*fetched)
        .clone()
        .filter(|(held, kind, _)| *held == id && *kind == variant)
        .map(|(_, _, url)| url)
        .or_else(|| {
            loader
                .as_ref()
                .filter(|_| wanted)
                .and_then(|loader| loader.get(id, variant))
        });
    {
        let fetched = fetched.clone();
        let tries = tries.clone();
        let have = current.is_some();
        let attempt = *tries;
        use_effect_with(
            (id, variant, wanted, have, attempt),
            move |(id, variant, wanted, have, attempt)| {
                // The try waiting to happen, dropped (and so cancelled) with
                // the bubble.
                let pending: Rc<RefCell<Option<gloo_timers::callback::Timeout>>> =
                    Rc::new(RefCell::new(None));
                if *wanted && !*have {
                    if let Some(loader) = loader {
                        let (id, variant, attempt) = (*id, *variant, *attempt);
                        let later = pending.clone();
                        loader.load(
                            id,
                            variant,
                            Callback::from(move |url: Option<String>| match url {
                                Some(url) => fetched.set(Some((id, variant, url))),
                                None => {
                                    if let Some(wait) = RETRY_MS.get(attempt) {
                                        let tries = tries.clone();
                                        *later.borrow_mut() = Some(
                                            gloo_timers::callback::Timeout::new(*wait, move || {
                                                tries.set(attempt + 1)
                                            }),
                                        );
                                    }
                                }
                            }),
                        );
                    }
                }
                move || {
                    pending.borrow_mut().take();
                }
            },
        );
    }
    current
}

/// Hand the browser a download of `url` under `name` — Save, and a file
/// row's click.
pub fn download(url: &str, name: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(anchor) = document.create_element("a") else {
        return;
    };
    let _ = anchor.set_attribute("href", url);
    let _ = anchor.set_attribute("download", name);
    let _ = anchor.set_attribute("rel", "noopener");
    if let Some(body) = document.body() {
        let _ = body.append_child(&anchor);
        if let Ok(anchor) = wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(anchor) {
            anchor.click();
            anchor.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::AppState;
    use crate::staged::StagedBytes;

    fn blob(text: &str) -> Blob {
        let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(text));
        Blob::new_with_str_sequence(&parts).expect("a blob")
    }

    fn loader() -> MediaLoader {
        MediaLoader::new(Live::new(
            AppState {
                token: Some("t".into()),
                ..AppState::default()
            },
            Rc::new(|| {}),
        ))
    }

    /// An attachment still in the outbox draws from the bytes the store
    /// holds — and after it lands, its server id draws from the same
    /// bytes, under a URL of its own.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn an_upload_draws_from_this_devices_own_bytes() {
        let loader = loader();
        let session = loader.live.session();
        loader.live.update(session, |state| {
            state.store.bytes.insert(
                -1,
                StagedBytes {
                    file: Some(blob("full")),
                    preview: Some(blob("small")),
                },
            );
        });
        let provisional = loader
            .get(-1, Variant::Preview)
            .expect("drawn from the outbox");
        assert!(provisional.starts_with("blob:"));
        assert_eq!(
            loader.get(-2, Variant::Preview),
            None,
            "nothing held, nothing fetched"
        );

        loader.live.update(session, |state| {
            state.store.aliases.insert(501, -1);
            state.store.bytes.clear();
        });
        let real = loader
            .get(501, Variant::Preview)
            .expect("drawn from the same bytes");
        assert_ne!(real, provisional, "a URL of its own");
        assert!(loader.held(501, Variant::Preview).is_some());
    }

    /// Bounded: past the cap the least recently used goes, and never the
    /// one just added.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn the_cache_lets_go_of_the_least_recently_used() {
        let loader = loader();
        for id in 1..=(ORIGINAL_CAP as i64 + 1) {
            loader
                .cache
                .borrow_mut()
                .insert((id, Variant::Original), blob("x"));
            if id == 1 {
                continue;
            }
            // Keep the first one warm.
            loader.get(1, Variant::Original);
        }
        let held: Vec<i64> = {
            let cache = loader.cache.borrow();
            let mut ids: Vec<i64> = cache.entries.keys().map(|key| key.0).collect();
            ids.sort();
            ids
        };
        assert_eq!(held.len(), ORIGINAL_CAP);
        assert!(held.contains(&1), "used recently");
        assert!(!held.contains(&2), "the least recently used went");
        assert!(held.contains(&(ORIGINAL_CAP as i64 + 1)));
    }

    /// A new session starts with nothing of the last one's.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn a_new_session_holds_nothing_of_the_last() {
        let loader = loader();
        loader
            .cache
            .borrow_mut()
            .insert((7, Variant::Preview), blob("theirs"));
        assert!(loader.get(7, Variant::Preview).is_some());
        loader.live.end_session();
        assert_eq!(loader.get(7, Variant::Preview), None);
    }

    /// A pending album's viewer outlives the bytes: once the message is
    /// sent they go with its row, and an item not yet drawn is fetched
    /// under the id the server gave it.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn a_sent_items_provisional_id_is_fetched_by_its_real_one() {
        let loader = loader();
        let session = loader.live.session();
        loader.live.update(session, |state| {
            state.store.aliases.insert(501, -1);
        });
        loader.load(-1, Variant::Original, Callback::noop());
        assert!(
            loader
                .cache
                .borrow()
                .waiting
                .contains_key(&(501, Variant::Original)),
            "fetching 501"
        );
        loader.load(-2, Variant::Original, Callback::noop());
        assert!(
            !loader
                .cache
                .borrow()
                .waiting
                .keys()
                .any(|key| key.0 == -2 || key.0 == 0),
            "nothing on the wire for an id with no server id"
        );
    }
}
