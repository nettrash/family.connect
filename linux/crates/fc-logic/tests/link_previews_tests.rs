//! `LinkPreviewsTests`: the card fetch — https only, once per link, failures kept, nothing while
//! switched off, and the table's oldest half evicted.
//!
//! The C# tests ran the fetches on the thread pool and polled with `Task.Delay`; here they run on a
//! `LocalPool` the test turns, over a fake `rt::Http` that answers by absolute address and keeps
//! every request (the C#'s `Pages` handler), and a `FakeClock` that never moves unless told — so no
//! deadline passes by itself. The C# faked a redirect's landing with `RequestMessage`; here it is a
//! real `301`, which `LinkPreviews` follows itself (see its docs).

mod common;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::{Http, HttpRequest, HttpResponse, LocalBoxFuture, Method, TransportError};
use fc_core::testing::FakeClock;
use fc_core::text::link_preview::LinkPreview;
use fc_logic::link_previews::{self, LinkPreviews, PreviewStatus};
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use time::macros::datetime;

use common::Gate;

type Answer = Rc<dyn Fn() -> LocalBoxFuture<'static, Result<HttpResponse, TransportError>>>;

/// Linked pages by absolute address, every request recorded.
#[derive(Clone, Default)]
struct Pages {
    answers: Rc<RefCell<HashMap<String, Answer>>>,
    asked: Rc<RefCell<Vec<HttpRequest>>>,
}

impl Pages {
    fn count(&self, url: &str) -> usize {
        self.asked
            .borrow()
            .iter()
            .filter(|request| request.url == url)
            .count()
    }

    fn on(
        &self,
        url: &str,
        answer: impl Fn() -> LocalBoxFuture<'static, Result<HttpResponse, TransportError>> + 'static,
    ) -> &Self {
        self.answers
            .borrow_mut()
            .insert(url.to_string(), Rc::new(answer));
        self
    }

    fn respond(&self, url: &str, response: HttpResponse) -> &Self {
        self.on(url, move || {
            let response = response.clone();
            Box::pin(async move { Ok(response) })
        })
    }

    fn html(&self, url: &str, html: &str) -> &Self {
        self.respond(
            url,
            answer(200, html.as_bytes(), Some("text/html; charset=utf-8")),
        )
    }
}

fn answer(status: u16, body: &[u8], content_type: Option<&str>) -> HttpResponse {
    HttpResponse {
        status,
        headers: content_type
            .map(|kind| vec![("Content-Type".to_string(), kind.to_string())])
            .unwrap_or_default(),
        body: body.to_vec(),
    }
}

impl Http for Pages {
    fn send(
        &self,
        request: HttpRequest,
    ) -> LocalBoxFuture<'_, Result<HttpResponse, TransportError>> {
        let url = request.url.clone();
        self.asked.borrow_mut().push(request);
        let found = self.answers.borrow().get(&url).cloned();
        match found {
            Some(answer) => answer(),
            None => {
                Box::pin(
                    async move { Err(TransportError::Unreachable(format!("no such page {url}"))) },
                )
            }
        }
    }
}

struct Rig {
    pool: LocalPool,
    pages: Pages,
    clock: FakeClock,
    previews: Rc<LinkPreviews>,
}

impl Rig {
    fn new(pages: &Pages) -> Rig {
        Rig::switched(pages, Rc::new(Cell::new(true)))
    }

    fn switched(pages: &Pages, on: Rc<Cell<bool>>) -> Rig {
        let pool = LocalPool::new();
        let spawner = pool.spawner();
        let clock = FakeClock::at(datetime!(2026-10-29 10:00 UTC));
        let previews = LinkPreviews::new(
            move || on.get(),
            Rc::new(pages.clone()),
            Rc::new(clock.clone()),
            Rc::new(move |future| spawner.spawn_local(future).expect("the pool takes work")),
        );
        Rig {
            pool,
            pages: pages.clone(),
            clock,
            previews,
        }
    }

    /// Asks until the answer is not "loading", turning the pool between asks.
    fn settled(&mut self, url: &str) -> (PreviewStatus, Option<LinkPreview>) {
        for _ in 0..50 {
            match self.previews.state(url) {
                Some((PreviewStatus::Loading, _)) => self.pool.run_until_stalled(),
                Some(settled) => return settled,
                None => panic!("{url}: switched off"),
            }
        }
        panic!("{url} never settled")
    }
}

const CARD: &str = r#"<html><head><title>Ignored</title>
<meta property="og:title" content="The Real Title">
<meta property="og:image" content="/img/a.png">
</head><body><p>and megabytes more</p></body></html>"#;

#[test]
fn a_page_becomes_a_card_with_its_image_in_one_step() {
    let picture = [0x89, 0x50, 0x4E, 0x47, 1, 2, 3];
    let pages = Pages::default();
    pages.html("https://example.com/a", CARD).respond(
        "https://example.com/img/a.png",
        answer(200, &picture, Some("image/png")),
    );
    let mut rig = Rig::new(&pages);
    let landed = Rc::new(Cell::new(0));
    let counted = landed.clone();
    rig.previews
        .landed()
        .subscribe(move |()| counted.set(counted.get() + 1));

    assert_eq!(
        rig.previews.state("https://example.com/a"),
        Some((PreviewStatus::Loading, None))
    );
    let (status, preview) = rig.settled("https://example.com/a");

    assert_eq!(status, PreviewStatus::Loaded);
    let preview = preview.unwrap();
    assert_eq!(preview.title, "The Real Title");
    assert_eq!(preview.site_name, "example.com");
    assert_eq!(
        rig.previews.image(&preview.url).as_deref(),
        Some(&picture[..])
    );
    assert_eq!(landed.get(), 1);
    assert_eq!(rig.previews.generation(), 1);
}

#[test]
fn requests_say_who_they_are_and_carry_nothing_else() {
    let pages = Pages::default();
    pages.html("https://example.com/a", CARD);
    let mut rig = Rig::new(&pages);
    let _ = rig.previews.state("https://example.com/a");
    let _ = rig.settled("https://example.com/a");

    let request = rig.pages.asked.borrow()[0].clone();
    assert_eq!(request.method, Method::Get);
    assert_eq!(
        request.header_values("User-Agent"),
        vec!["FamilyConnect/1.0 (+link-preview; like WhatsApp)"]
    );
    assert_eq!(
        request.header_values("Accept"),
        vec!["text/html,application/xhtml+xml"]
    );
    assert!(request.header_values("Cookie").is_empty());
    assert!(request.header_values("Referer").is_empty());
    assert!(request.header_values("Authorization").is_empty());
    assert!(request.body.is_none());
    assert_eq!(request.timeout, Some(link_previews::FETCH_TIMEOUT));
}

/// A cleartext page is refused without a request, and so is a cleartext card image.
#[test]
fn only_https_is_ever_fetched() {
    let pages = Pages::default();
    pages
        .html(
            "https://example.com/a",
            r#"<head><title>T</title><meta property="og:image" content="http://cdn.example.com/a.png"></head>"#,
        )
        .html("http://example.com/plain", CARD);
    let mut rig = Rig::new(&pages);

    assert_eq!(
        rig.previews.state("http://example.com/plain").map(|s| s.0),
        Some(PreviewStatus::Unavailable)
    );
    assert_eq!(
        rig.previews.state("not a url").map(|s| s.0),
        Some(PreviewStatus::Unavailable)
    );
    // Refused on sight, while a bubble is drawn: nothing landed, so nothing ticks.
    assert_eq!(rig.previews.generation(), 0);
    let (status, preview) = rig.settled("https://example.com/a");

    assert_eq!(status, PreviewStatus::Loaded);
    assert!(rig.previews.image(&preview.unwrap().url).is_none());
    assert_eq!(rig.pages.asked.borrow().len(), 1);
}

/// Every bubble drawing a link shares one fetch, and a settled answer — a failure included — is
/// never fetched again.
#[test]
fn a_link_is_fetched_once_and_a_failure_is_kept() {
    let release = Gate::new();
    let pages = Pages::default();
    let waiting = release.clone();
    pages
        .on("https://example.com/slow", move || {
            let gate = waiting.clone();
            Box::pin(async move {
                gate.wait().await;
                Ok(answer(200, CARD.as_bytes(), Some("text/html")))
            })
        })
        .respond(
            "https://example.com/gone",
            answer(404, &[], Some("text/html")),
        );
    let mut rig = Rig::new(&pages);

    for _ in 0..3 {
        assert_eq!(
            rig.previews.state("https://example.com/slow").map(|s| s.0),
            Some(PreviewStatus::Loading)
        );
        rig.pool.run_until_stalled();
    }
    release.open();
    assert_eq!(
        rig.settled("https://example.com/slow").0,
        PreviewStatus::Loaded
    );
    let _ = rig.previews.state("https://example.com/slow");
    assert_eq!(rig.pages.count("https://example.com/slow"), 1);

    assert_eq!(
        rig.settled("https://example.com/gone").0,
        PreviewStatus::Unavailable
    );
    let _ = rig.previews.state("https://example.com/gone");
    rig.pool.run_until_stalled();
    assert_eq!(rig.pages.count("https://example.com/gone"), 1);
}

/// Not OK, not HTML, or not reachable: no card. A page that declares no type is still read.
#[test]
fn only_an_ok_html_page_makes_a_card() {
    let html = CARD.as_bytes();
    let pages = Pages::default();
    pages
        .respond(
            "https://example.com/error",
            answer(500, html, Some("text/html")),
        )
        .respond(
            "https://example.com/moved",
            answer(302, html, Some("text/html")),
        )
        .respond(
            "https://example.com/file.pdf",
            answer(200, html, Some("application/pdf")),
        )
        .respond("https://example.com/untyped", answer(200, html, None))
        .respond(
            "https://example.com/xhtml",
            answer(200, html, Some("application/xhtml+xml")),
        )
        .on("https://example.com/throws", || {
            Box::pin(async { Err(TransportError::Unreachable("reset".to_string())) })
        });
    let mut rig = Rig::new(&pages);

    for url in [
        "https://example.com/error",
        "https://example.com/moved",
        "https://example.com/file.pdf",
        "https://example.com/unreachable",
        "https://example.com/throws",
    ] {
        assert_eq!(rig.settled(url).0, PreviewStatus::Unavailable, "{url}");
    }
    assert_eq!(
        rig.settled("https://example.com/untyped").0,
        PreviewStatus::Loaded
    );
    assert_eq!(
        rig.settled("https://example.com/xhtml").0,
        PreviewStatus::Loaded
    );
}

#[test]
fn switched_off_nothing_is_asked_for_at_all() {
    let on = Rc::new(Cell::new(false));
    let pages = Pages::default();
    pages.html("https://example.com/a", CARD);
    let mut rig = Rig::switched(&pages, on.clone());

    assert_eq!(rig.previews.state("https://example.com/a"), None);
    rig.pool.run_until_stalled();
    assert!(rig.pages.asked.borrow().is_empty());

    on.set(true);
    assert_eq!(
        rig.settled("https://example.com/a").0,
        PreviewStatus::Loaded
    );
    on.set(false);
    assert_eq!(rig.previews.state("https://example.com/a"), None);
}

/// The label and a relative image resolve against where a redirect landed, and the image is kept
/// under that address.
#[test]
fn a_redirected_page_names_where_it_landed() {
    let picture = [1, 2, 3];
    let mut moved = answer(301, &[], None);
    moved.headers.push((
        "Location".to_string(),
        "https://www.landed.example/final/page".to_string(),
    ));
    let pages = Pages::default();
    pages
        .respond("https://short.example/x", moved)
        .respond(
            "https://www.landed.example/final/page",
            answer(200, CARD.as_bytes(), Some("text/html")),
        )
        .respond(
            "https://www.landed.example/img/a.png",
            answer(200, &picture, Some("image/png")),
        );
    let mut rig = Rig::new(&pages);

    let preview = rig.settled("https://short.example/x").1.unwrap();

    assert_eq!(preview.site_name, "landed.example");
    assert_eq!(preview.url, "https://www.landed.example/final/page");
    assert_eq!(
        rig.previews.image(&preview.url).as_deref(),
        Some(&picture[..])
    );
    assert!(rig.previews.image("https://short.example/x").is_none());
}

#[test]
fn a_read_stops_at_the_head_and_a_page_speaks_its_own_charset() {
    // "Привет" in windows-1251.
    let mut cyrillic = b"<head><title>".to_vec();
    cyrillic.extend_from_slice(&[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]);
    cyrillic.extend_from_slice(b"</title></head><body>");
    let pages = Pages::default();
    pages
        .html(
            "https://example.com/late",
            r#"<html><head><meta name="x" content="y"></head><body><meta property="og:title" content="Too Late"></body>"#,
        )
        .respond(
            "https://example.com/ru",
            answer(200, &cyrillic, Some("text/html; charset=windows-1251")),
        );
    let mut rig = Rig::new(&pages);

    assert_eq!(
        rig.settled("https://example.com/late").0,
        PreviewStatus::Unavailable
    );
    assert_eq!(
        rig.settled("https://example.com/ru")
            .1
            .map(|preview| preview.title),
        Some("Привет".to_string())
    );
}

/// A full table drops its oldest half, not everything: the newest cards stay, the oldest are
/// fetched again when asked.
#[test]
fn a_full_table_forgets_its_oldest_half() {
    const MAX: usize = link_previews::MAX_ENTRIES;
    let url = |index: usize| format!("https://example.com/{index}");
    let pages = Pages::default();
    for index in 0..=MAX {
        pages.html(&url(index), &format!("<head><title>{index}</title></head>"));
    }
    let mut rig = Rig::new(&pages);
    for index in 0..=MAX {
        assert_eq!(rig.settled(&url(index)).0, PreviewStatus::Loaded);
    }

    // The newest half is still known — answered at once, not fetched again.
    for index in [MAX / 2, MAX - 1, MAX] {
        assert_eq!(
            rig.previews.state(&url(index)).map(|s| s.0),
            Some(PreviewStatus::Loaded)
        );
    }
    assert_eq!(rig.pages.count(&url(MAX / 2)), 1);
    assert_eq!(rig.pages.count(&url(MAX - 1)), 1);
    // The oldest half was forgotten, and asking again fetches again.
    assert_eq!(rig.settled(&url(0)).0, PreviewStatus::Loaded);
    assert_eq!(rig.pages.count(&url(0)), 2);
    assert_eq!(rig.settled(&url(MAX / 2 - 1)).0, PreviewStatus::Loaded);
    assert_eq!(rig.pages.count(&url(MAX / 2 - 1)), 2);
}

// ---- the port's own ---------------------------------------------------------------------------

/// A redirect to plain http is not followed: the redirect is the answer, and no card.
#[test]
fn a_redirect_to_plain_http_is_not_followed() {
    let mut moved = answer(302, &[], None);
    moved.headers.push((
        "Location".to_string(),
        "http://example.com/plain".to_string(),
    ));
    let pages = Pages::default();
    pages
        .respond("https://example.com/a", moved)
        .html("http://example.com/plain", CARD);
    let mut rig = Rig::new(&pages);

    assert_eq!(
        rig.settled("https://example.com/a").0,
        PreviewStatus::Unavailable
    );
    assert_eq!(rig.pages.count("http://example.com/plain"), 0);
}

/// A page that does not answer within twenty seconds is no card — and the image after a page has
/// its own twenty.
#[test]
fn a_page_slower_than_the_deadline_is_no_card() {
    let never = Gate::new();
    let pages = Pages::default();
    let waiting = never.clone();
    pages.on("https://example.com/slow", move || {
        let gate = waiting.clone();
        Box::pin(async move {
            gate.wait().await;
            Ok(answer(200, CARD.as_bytes(), Some("text/html")))
        })
    });
    let mut rig = Rig::new(&pages);
    let _ = rig.previews.state("https://example.com/slow");
    rig.pool.run_until_stalled();
    rig.clock.advance(Duration::from_secs(19));
    rig.pool.run_until_stalled();
    assert_eq!(
        rig.previews.state("https://example.com/slow").map(|s| s.0),
        Some(PreviewStatus::Loading)
    );
    rig.clock.advance(Duration::from_secs(1));
    assert_eq!(
        rig.settled("https://example.com/slow").0,
        PreviewStatus::Unavailable
    );
}

/// A card's image is evicted with the link that fetched it — even when the page landed somewhere
/// else, where the C# kept it for good.
#[test]
fn a_redirected_cards_image_leaves_with_its_link() {
    const MAX: usize = link_previews::MAX_ENTRIES;
    let mut moved = answer(301, &[], None);
    moved.headers.push((
        "Location".to_string(),
        "https://landed.example/page".to_string(),
    ));
    let pages = Pages::default();
    pages
        .respond("https://short.example/0", moved)
        .respond(
            "https://landed.example/page",
            answer(200, CARD.as_bytes(), Some("text/html")),
        )
        .respond(
            "https://landed.example/img/a.png",
            answer(200, &[7, 7, 7], Some("image/png")),
        );
    for index in 1..=MAX {
        pages.html(
            &format!("https://example.com/{index}"),
            &format!("<head><title>{index}</title></head>"),
        );
    }
    let mut rig = Rig::new(&pages);
    let preview = rig.settled("https://short.example/0").1.unwrap();
    assert!(rig.previews.image(&preview.url).is_some());
    for index in 1..=MAX {
        let _ = rig.settled(&format!("https://example.com/{index}"));
    }
    assert!(rig.previews.image("https://landed.example/page").is_none());
}

/// Dropping the fetcher stops what it had on the way: nothing lands afterwards.
#[test]
fn a_fetch_on_its_way_when_the_fetcher_goes_lands_nowhere() {
    let release = Gate::new();
    let pages = Pages::default();
    let waiting = release.clone();
    pages.on("https://example.com/slow", move || {
        let gate = waiting.clone();
        Box::pin(async move {
            gate.wait().await;
            Ok(answer(200, CARD.as_bytes(), Some("text/html")))
        })
    });
    let mut rig = Rig::new(&pages);
    let landed = Rc::new(Cell::new(0));
    let counted = landed.clone();
    rig.previews
        .landed()
        .subscribe(move |()| counted.set(counted.get() + 1));
    let _ = rig.previews.state("https://example.com/slow");
    rig.pool.run_until_stalled();
    let Rig {
        mut pool, previews, ..
    } = rig;
    drop(previews);
    release.open();
    pool.run_until_stalled();
    assert_eq!(landed.get(), 0);
}
