//! Fetches the card under a message's first web link (`LinkPreviews.cs`; ios `LinkPreviewLoader`,
//! android `LinkPreviewRepository`). How a page becomes a card, where its head ends and how its
//! bytes become text are `fc_core::text::link_preview`'s.
//!
//! PRIVACY, up front: the one place the app talks to a host the family does not own. Every device
//! that shows a linked message contacts that link — switchable off in Settings, which is why
//! [`LinkPreviews::state`] asks first. Requests are stripped down: https only, no cookies, no
//! credentials, no referrer, and a read that stops at the end of the page's `<head>` or 1 MB. A
//! redirect to plain http is not followed.
//!
//! One fetch per link however many bubbles draw it. Results — failures too, so a dead link is not
//! retried on every redraw — live in memory only: a preview is derived data. The oldest half goes
//! when the table fills, never all of it, or every card on screen would vanish at once. A hidden row
//! asks exactly as a visible one does (docs/protocol.md, "A hidden row still fetches, and draws none
//! of it"), and nothing here knows who sent the link.
//!
//! THE SEAM. The pages come through an [`rt::Http`](Http) the shell makes for this alone: a
//! `soup::Session` with no cookie jar and no credentials, whose messages carry
//! `SOUP_MESSAGE_NO_REDIRECT` — THIS follows redirects (at most [`MAX_REDIRECTS`], https only), so it
//! knows where a page landed: `rt::HttpResponse` does not say, and the card's label and its relative
//! image resolve against the landing (the C# read `response.RequestMessage.RequestUri`). A seam that
//! followed redirects itself would still work, with the asked address standing for the landing.
//! The seam hands back whole bodies, so the 1 MB and end-of-head stop bound what is READ, not what
//! is transferred; [`FETCH_TIMEOUT`] goes in `HttpRequest::timeout` and is raced here as well.
//!
//! ONE DIFFERENCE FROM THE C#, deliberately: a card's image is kept under the address its page
//! landed on and evicted with the link that fetched it. The C# evicted `images` by the ASKED
//! address, so the image of a link that redirected — or was asked without a path, `https://a.b`
//! landing as `https://a.b/` — was never evicted: up to 4 MB a link, for the life of the process.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::rc::{Rc, Weak};
use std::time::Duration;

use fc_core::rt::{Cancel, Clock, Event, Http, HttpRequest, HttpResponse, Method};
use fc_core::text::link_preview::{self, LinkPreview, page_reader};
use futures_util::future::{Either, select};

use crate::calls::Spawn;

/// What a bubble knows about its link's card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewStatus {
    Loading,
    Loaded,
    Unavailable,
}

/// How many settled links are remembered before the oldest half goes.
pub const MAX_ENTRIES: usize = 200;

pub const USER_AGENT: &str = "FamilyConnect/1.0 (+link-preview; like WhatsApp)";

pub const ACCEPT: &str = "text/html,application/xhtml+xml";

/// Connecting may take this long — the shell's to honour, where its HTTP stack can — and a whole
/// fetch, page or image, [`FETCH_TIMEOUT`].
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// The most redirects one fetch follows — .NET's own default, which the C# left in place.
pub const MAX_REDIRECTS: usize = 50;

#[derive(Debug, Clone)]
struct Entry {
    status: PreviewStatus,
    preview: Option<LinkPreview>,
    /// Where this link's card image is kept, when it has one.
    image: Option<String>,
}

#[derive(Debug, Default)]
struct Table {
    states: HashMap<String, Entry>,
    in_flight: HashSet<String>,
    images: HashMap<String, Rc<[u8]>>,
    /// Settled links, oldest first, so eviction drops the oldest rather than everything.
    order: VecDeque<String>,
}

impl Table {
    /// Record a settled link — making room first by dropping the oldest half when the table is
    /// full.
    fn settle(&mut self, url: &str, entry: Entry) {
        if self.states.len() >= MAX_ENTRIES {
            let dropping = (MAX_ENTRIES / 2).min(self.order.len());
            for old in self.order.drain(..dropping).collect::<Vec<_>>() {
                if let Some(Entry {
                    image: Some(image), ..
                }) = self.states.remove(&old)
                {
                    let shared = self
                        .states
                        .values()
                        .any(|entry| entry.image.as_deref() == Some(image.as_str()));
                    if !shared {
                        self.images.remove(&image);
                    }
                }
            }
        }
        self.states.insert(url.to_string(), entry);
        self.order.push_back(url.to_string());
    }
}

/// The card fetcher. Hold it in the `Rc` [`LinkPreviews::new`] gives; dropping it stops what it
/// still has on the way.
pub struct LinkPreviews {
    me: Weak<LinkPreviews>,
    http: Rc<dyn Http>,
    clock: Rc<dyn Clock>,
    spawn: Spawn,
    enabled: Box<dyn Fn() -> bool>,
    table: RefCell<Table>,
    generation: Cell<i32>,
    landed: Event<()>,
    closed: Cancel,
}

impl std::fmt::Debug for LinkPreviews {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkPreviews")
            .field("generation", &self.generation)
            .field("table", &self.table)
            .finish_non_exhaustive()
    }
}

impl LinkPreviews {
    /// `enabled` is the reader's Link Previews switch, asked every time; `http` the stripped-down
    /// transport (see the module's docs); `clock` the deadlines; `spawn` how a fetch is started.
    pub fn new(
        enabled: impl Fn() -> bool + 'static,
        http: Rc<dyn Http>,
        clock: Rc<dyn Clock>,
        spawn: Spawn,
    ) -> Rc<LinkPreviews> {
        Rc::new_cyclic(|me| LinkPreviews {
            me: me.clone(),
            http,
            clock,
            spawn,
            enabled: Box::new(enabled),
            table: RefCell::new(Table::default()),
            generation: Cell::new(0),
            landed: Event::new(),
            closed: Cancel::new(),
        })
    }

    /// A fetch settled — a card may have appeared, and a bubble changed height.
    pub fn landed(&self) -> &Event<()> {
        &self.landed
    }

    /// Ticks every time a fetch lands.
    pub fn generation(&self) -> i32 {
        self.generation.get()
    }

    /// What is known about `url`, starting its one fetch the first time it is asked for — or `None`
    /// while the reader has previews switched off, when nothing is fetched at all.
    pub fn state(&self, url: &str) -> Option<(PreviewStatus, Option<LinkPreview>)> {
        if !(self.enabled)() {
            return None;
        }
        let mut table = self.table.borrow_mut();
        if let Some(known) = table.states.get(url) {
            return Some((known.status, known.preview.clone()));
        }
        // https only, as on the phones: fetching a cleartext address somebody else chose is the
        // wrong default for the one request this app makes off the family's own server.
        let Some(uri) = https(url) else {
            let refused = Entry {
                status: PreviewStatus::Unavailable,
                preview: None,
                image: None,
            };
            table.settle(url, refused);
            return Some((PreviewStatus::Unavailable, None));
        };
        if table.in_flight.insert(url.to_string()) {
            table.states.insert(
                url.to_string(),
                Entry {
                    status: PreviewStatus::Loading,
                    preview: None,
                    image: None,
                },
            );
            drop(table);
            let fetch = Fetch {
                http: self.http.clone(),
                clock: self.clock.clone(),
            };
            let (me, closed, url) = (self.me.clone(), self.closed.clone(), url.to_string());
            (self.spawn)(Box::pin(async move {
                let loading = fetch.load(&uri);
                let (preview, image) = match select(Box::pin(loading), closed.cancelled()).await {
                    Either::Left((loaded, _)) => loaded,
                    Either::Right(_) => return,
                };
                if let Some(me) = me.upgrade() {
                    me.landed_with(&url, preview, image);
                }
            }));
        }
        Some((PreviewStatus::Loading, None))
    }

    /// The card image's bytes once they arrived, kept under the preview's own url — where a
    /// redirect landed.
    pub fn image(&self, preview_url: &str) -> Option<Rc<[u8]>> {
        let key = link_preview::absolute_url(preview_url, preview_url)
            .unwrap_or_else(|| preview_url.to_string());
        self.table.borrow().images.get(&key).cloned()
    }

    fn landed_with(&self, url: &str, preview: Option<LinkPreview>, image: Option<Vec<u8>>) {
        {
            let mut table = self.table.borrow_mut();
            table.in_flight.remove(url);
            let mut kept = None;
            if let (Some(preview), Some(image)) = (&preview, image) {
                table
                    .images
                    .insert(preview.url.clone(), Rc::from(image.into_boxed_slice()));
                kept = Some(preview.url.clone());
            }
            let entry = Entry {
                status: if preview.is_some() {
                    PreviewStatus::Loaded
                } else {
                    PreviewStatus::Unavailable
                },
                preview,
                image: kept,
            };
            table.settle(url, entry);
            // Only a fetch that LANDED ticks: a link refused on sight is answered while its bubble
            // is being drawn, and a tick then would make the very next draw rebuild everything for
            // nothing.
            self.generation.set(self.generation.get().wrapping_add(1));
        }
        self.landed.emit(&());
    }
}

impl Drop for LinkPreviews {
    fn drop(&mut self) {
        self.closed.cancel();
    }
}

/// `url` as an absolute https address (`Uri.TryCreate` and `Scheme == https`), written as
/// `Uri.AbsoluteUri` writes it.
fn https(url: &str) -> Option<String> {
    link_preview::absolute_url(url, url).filter(|absolute| absolute.starts_with("https://"))
}

/// What one fetch needs: the transport and the clock.
struct Fetch {
    http: Rc<dyn Http>,
    clock: Rc<dyn Clock>,
}

impl Fetch {
    /// The card, and its image — fetched BEFORE the card is published, so a card appears in one
    /// step instead of growing its bubble twice. https only. A dead or slow link is simply no
    /// card; the text keeps whatever the page gave before the image failed.
    async fn load(&self, uri: &str) -> (Option<LinkPreview>, Option<Vec<u8>>) {
        let preview = self.within_deadline(self.preview(uri)).await.flatten();
        let image = match preview.as_ref().and_then(|p| p.image_url.as_deref()) {
            Some(image) if image.starts_with("https://") => {
                self.within_deadline(self.image(image)).await.flatten()
            }
            _ => None,
        };
        (preview, image)
    }

    /// `work`, or `None` once [`FETCH_TIMEOUT`] has passed.
    async fn within_deadline<T>(&self, work: impl std::future::Future<Output = T>) -> Option<T> {
        match select(Box::pin(work), self.clock.sleep(FETCH_TIMEOUT)).await {
            Either::Left((done, _)) => Some(done),
            Either::Right(_) => None,
        }
    }

    async fn preview(&self, uri: &str) -> Option<LinkPreview> {
        let (response, landed) = self.get(uri).await?;
        if !(200..300).contains(&response.status) {
            return None;
        }
        let content_type = ContentType::of(&response);
        // Only HTML can carry the tags; a PDF or a download would be bytes burned.
        if !content_type.media_type.is_empty() && !content_type.media_type.contains("html") {
            return None;
        }
        let body = futures_util::stream::iter([Ok::<_, Infallible>(response.body)]);
        let bytes = page_reader::read(body, page_reader::MAX_PAGE_BYTES, true)
            .await
            .unwrap_or_else(|never| match never {});
        let html = page_reader::decode(&bytes, content_type.charset.as_deref());
        // Relative images and the host label resolve against where a redirect landed, not what
        // was asked for.
        link_preview::parse(&html, &landed)
    }

    async fn image(&self, uri: &str) -> Option<Vec<u8>> {
        let (response, _) = self.get(uri).await?;
        if !(200..300).contains(&response.status) {
            return None;
        }
        let mut bytes = response.body;
        bytes.truncate(page_reader::MAX_IMAGE_BYTES);
        (!bytes.is_empty()).then_some(bytes)
    }

    /// A GET of `uri`, its redirects followed while they stay https: the answer, and where it was
    /// read. A redirect that cannot be followed is the answer itself, as .NET's handler answers
    /// one. `None` when nothing answered.
    async fn get(&self, uri: &str) -> Option<(HttpResponse, String)> {
        let mut at = uri.to_string();
        let mut redirects = 0;
        loop {
            let mut request = HttpRequest::new(Method::Get, at.clone())
                .header("Accept", ACCEPT)
                // Bots get the metadata without the consent walls and personalisation a browser's
                // User-Agent invites.
                .header("User-Agent", USER_AGENT);
            request.timeout = Some(FETCH_TIMEOUT);
            let response = self.http.send(request).await.ok()?;
            let next = matches!(response.status, 301 | 302 | 303 | 307 | 308)
                .then(|| response.header("Location"))
                .flatten()
                .and_then(|location| link_preview::absolute_url(location, &at))
                .filter(|next| next.starts_with("https://"));
            match next {
                Some(next) if redirects < MAX_REDIRECTS => {
                    redirects += 1;
                    at = next;
                }
                _ => return Some((response, at)),
            }
        }
    }
}

/// A response's `Content-Type`: its media type, lowercased (empty when none, or one that is not
/// `type/subtype`, as .NET drops a header it cannot parse), and its `charset` parameter as written.
struct ContentType {
    media_type: String,
    charset: Option<String>,
}

impl ContentType {
    fn of(response: &HttpResponse) -> ContentType {
        let Some(header) = response.header("Content-Type") else {
            return ContentType {
                media_type: String::new(),
                charset: None,
            };
        };
        let mut parts = header.split(';');
        let media_type = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
        if !media_type.contains('/') {
            return ContentType {
                media_type: String::new(),
                charset: None,
            };
        }
        let charset = parts.find_map(|parameter| {
            let (name, value) = parameter.split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| value.trim().to_string())
        });
        ContentType {
            media_type,
            charset,
        }
    }
}
