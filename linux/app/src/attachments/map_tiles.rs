//! OpenStreetMap's tiles for the maps under shared locations (shell-design §5.2, D11 — Windows'
//! `MapTiles`): fetched once, kept on disk under `$XDG_CACHE_HOME/family-connect/tiles`, and asked
//! for only while the reader has "Map Previews" on (the caller's rule; [`super::location`]).
//!
//! **PRIVACY, UP FRONT.** A tile is a square of map around a place a family member shared, so
//! asking for one tells OpenStreetMap roughly where that place is — the trade the Apple apps make
//! with Apple for MapKit, and the reason for the switch. Requests go through their OWN libsoup
//! session (never the API's: no token, no cookies, no redirects followed, no referrer), and nothing
//! about a tile — not its coordinates, not its URL — is ever written to the log.
//!
//! **OPENSTREETMAP'S TILE POLICY, KEPT** (<https://operations.osmfoundation.org/policies/tiles/>):
//! the app names itself in its User-Agent, every tile is kept on disk and not fetched again for a
//! month, a tile that failed is not asked for again for five minutes, one fetch per tile however
//! many maps ask, and every map drawn carries the attribution. Out of date but on disk beats
//! nothing: a month-old street is still the street.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use fc_logic::map_view::{self, MapTile};
use futures_util::FutureExt;
use futures_util::future::{LocalBoxFuture, Shared};
use soup::prelude::*;

/// What every tile request names itself as (the policy asks for a contact).
pub fn user_agent() -> String {
    format!(
        "FamilyConnect-Linux/{} (+https://github.com/nettrash/family.connect)",
        env!("CARGO_PKG_VERSION")
    )
}

/// A tile on disk is fresh for a month.
pub const KEEP: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// A failed tile is not asked for again for five minutes.
pub const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
/// One fetch may take this long.
pub const FETCH_LIMIT: Duration = Duration::from_secs(20);
/// The most a tile may weigh (a PNG tile is tens of kilobytes).
pub const MOST_BYTES: usize = 1_000_000;

type Loading = Shared<LocalBoxFuture<'static, Option<Rc<[u8]>>>>;

/// The tile fetcher (one per process).
pub struct MapTiles {
    session: soup::Session,
    folder: PathBuf,
    url_for: RefCell<Rc<dyn Fn(&MapTile) -> String>>,
    loading: RefCell<HashMap<String, Loading>>,
    failed: RefCell<HashMap<String, Instant>>,
}

thread_local! {
    static TILES: RefCell<Option<Rc<MapTiles>>> = const { RefCell::new(None) };
}

impl MapTiles {
    /// The process's fetcher, over `$XDG_CACHE_HOME/family-connect/tiles`.
    pub fn get() -> Rc<MapTiles> {
        TILES.with(|tiles| {
            if let Some(held) = tiles.borrow().as_ref() {
                return Rc::clone(held);
            }
            let folder = crate::shell::Shell::get().paths.tiles_dir();
            let made = Rc::new(MapTiles::new(folder));
            *tiles.borrow_mut() = Some(Rc::clone(&made));
            made
        })
    }

    fn new(folder: PathBuf) -> MapTiles {
        let session = soup::Session::builder()
            .user_agent(user_agent())
            .timeout(0)
            .build();
        MapTiles {
            session,
            folder,
            url_for: RefCell::new(Rc::new(map_view::tile_url)),
            loading: RefCell::new(HashMap::new()),
            failed: RefCell::new(HashMap::new()),
        }
    }

    /// The self-test's tile server instead of OpenStreetMap's (never in a shipped build).
    #[cfg(feature = "selftest")]
    pub fn use_source(&self, url_for: impl Fn(&MapTile) -> String + 'static) {
        *self.url_for.borrow_mut() = Rc::new(url_for);
        self.failed.borrow_mut().clear();
    }

    /// A tile's bytes, or `None` when there are none to be had right now.
    pub async fn bytes(self: &Rc<Self>, tile: &MapTile) -> Option<Rc<[u8]>> {
        let key = tile.key();
        let joined = self.loading.borrow().get(&key).cloned();
        let loading = match joined {
            Some(loading) => loading,
            None => {
                let failed_lately = self
                    .failed
                    .borrow()
                    .get(&key)
                    .is_some_and(|at| at.elapsed() < RETRY_AFTER);
                if failed_lately {
                    return self.on_disk(&key, false);
                }
                let (me, tile_held, key_held) = (Rc::clone(self), *tile, key.clone());
                let task: LocalBoxFuture<'static, Option<Rc<[u8]>>> =
                    Box::pin(async move { me.load(&key_held, &tile_held).await });
                let shared = task.shared();
                self.loading
                    .borrow_mut()
                    .insert(key.clone(), shared.clone());
                shared
            }
        };
        let bytes = loading.await;
        self.loading.borrow_mut().remove(&key);
        bytes
    }

    async fn load(&self, key: &str, tile: &MapTile) -> Option<Rc<[u8]>> {
        if let Some(fresh) = self.on_disk(key, true) {
            return Some(fresh);
        }
        let url = (self.url_for.borrow().clone())(tile);
        let fetched = self.fetch(&url).await;
        match fetched {
            Some(bytes) => {
                self.failed.borrow_mut().remove(key);
                self.keep(key, &bytes);
                Some(Rc::from(bytes.into_boxed_slice()))
            }
            None => {
                self.failed
                    .borrow_mut()
                    .insert(key.to_string(), Instant::now());
                // Out of date but on disk beats nothing.
                self.on_disk(key, false)
            }
        }
    }

    async fn fetch(&self, url: &str) -> Option<Vec<u8>> {
        let message = soup::Message::new("GET", url).ok()?;
        message.add_flags(soup::MessageFlags::NO_REDIRECT);
        let sending = self
            .session
            .send_and_read_future(&message, glib::Priority::DEFAULT_IDLE);
        let body = match glib::future_with_timeout(FETCH_LIMIT, sending).await {
            Ok(Ok(body)) => body,
            Ok(Err(error)) => {
                crate::fc_info!(
                    "map tile: unreachable ({:?})",
                    error.kind::<gio::IOErrorEnum>()
                );
                return None;
            }
            Err(_) => {
                crate::fc_info!("map tile: timed out");
                return None;
            }
        };
        let status = glib::translate::IntoGlib::into_glib(message.status());
        if !(200..300).contains(&status) {
            crate::fc_info!("map tile refused: {status}");
            return None;
        }
        (!body.is_empty() && body.len() < MOST_BYTES).then(|| body.to_vec())
    }

    fn path(&self, key: &str) -> PathBuf {
        self.folder.join(format!("{key}.png"))
    }

    /// The tile on disk: only when fresh, unless `fresh_only` is false.
    fn on_disk(&self, key: &str, fresh_only: bool) -> Option<Rc<[u8]>> {
        let path = self.path(key);
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        let age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or(Duration::ZERO);
        if fresh_only && age >= KEEP {
            return None;
        }
        std::fs::read(&path)
            .ok()
            .filter(|bytes| !bytes.is_empty())
            .map(|bytes| Rc::from(bytes.into_boxed_slice()))
    }

    /// Writes a tile whole (a part file renamed into place).
    fn keep(&self, key: &str, bytes: &[u8]) {
        if crate::services::files::private_dir(&self.folder).is_err() {
            return;
        }
        let path = self.path(key);
        let part = self
            .folder
            .join(format!(".{key}.{}.part", std::process::id()));
        let written = std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, &path));
        if let Err(error) = written {
            let _ = std::fs::remove_file(&part);
            crate::fc_info!("map tile: not kept ({})", error.kind());
        }
    }
}

/// Where "open the place" goes on Linux: OpenStreetMap's own site, with a marker, at the map's zoom
/// (§5.2, D11 — no system map app is assumed). The coordinates with a POINT and seven places,
/// whatever the reader's locale.
pub fn place_url(latitude: f64, longitude: f64) -> String {
    format!(
        "https://www.openstreetmap.org/?mlat={latitude:.7}&mlon={longitude:.7}#map={}/{latitude:.7}/{longitude:.7}",
        map_view::ZOOM
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_place_opens_on_openstreetmap_with_a_point_whatever_the_locale() {
        assert_eq!(
            place_url(44.8125, 20.4612),
            "https://www.openstreetmap.org/?mlat=44.8125000&mlon=20.4612000#map=16/44.8125000/20.4612000"
        );
        assert!(place_url(-33.5, -70.25).contains("mlat=-33.5000000&mlon=-70.2500000"));
    }

    #[test]
    fn the_app_names_itself_with_a_contact() {
        let agent = user_agent();
        assert!(agent.starts_with("FamilyConnect-Linux/"), "{agent}");
        assert!(agent.contains("+https://"), "{agent}");
    }
}
