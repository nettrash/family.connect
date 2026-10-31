//! What the fc-logic tests share: the fixtures (copied byte for byte — `win/`'s embedded catalogues
//! under `fixtures/i18n`, `emoji-typing-vectors.json` from `win/tests/FamilyConnect.Core.Tests/
//! Fixtures`, and `fc-rich.js` from `win/src/FamilyConnect.App/Assets/Rich`, which is `web/rich`'s),
//! and the catalogues the C# suite gets from its assembly — plus the C# suite's two helpers,
//! `Server.cs` ([`server`], with the `ApiClient` the tests build over it) and `TypingBox.cs`
//! ([`typing_box`]); the in-memory blob and staging stores the media tests share ([`media`]); and
//! the cache, client and socket the conversation tests open a chat over ([`chat`]), with the
//! roster and poll documents of `PollsModelTests` ([`polls`]); the call engine's rig ([`calls`]);
//! and the Base64, SHA-256 and zlib the animated-picture tests take from .NET ([`codecs`]), with a
//! scratch folder for the file-store tests ([`scratch_dir`]) and a gate a fake answer waits on
//! ([`Gate`]).
//!
//! Each test binary uses a different part of this, so the rest is unused there.
#![allow(dead_code)]

use std::path::PathBuf;
use std::rc::Rc;

use serde_json::Value;

use fc_core::text::catalogues::Catalogues;
use fc_core::text::plurals::PluralTables;
use fc_core::text::string_catalog::EnglishCatalog;

pub mod calls;
pub mod chat;
pub mod codecs;
pub mod consent;
pub mod media;
pub mod notes;
pub mod pack;
pub mod polls;
pub mod server;
pub mod session;
pub mod typing_box;

/// A fixture's path.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// A fixture's text.
pub fn fixture_text(name: &str) -> String {
    std::fs::read_to_string(fixture(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// A fixture read as JSON.
pub fn fixture_json(name: &str) -> Value {
    serde_json::from_str(&fixture_text(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// The catalogues `win/` embeds (`Text/i18n/*.json`), standing in for whatever the Linux client
/// ships: the C# suite reads the ones in its assembly (`JsonCatalog.For(language)`).
pub fn catalogues() -> Catalogues {
    Catalogues::from_dir(&fixture("i18n")).expect("the fixture catalogues load")
}

/// `win/`'s `plurals.json`.
pub fn plurals() -> Rc<PluralTables> {
    catalogues().plurals()
}

/// `EnglishCatalog.Instance`: English, saying a counted sentence in the form its count takes.
pub fn english() -> EnglishCatalog {
    EnglishCatalog::with_plurals(plurals())
}

/// How a day is written where a test does not care: en-US, UTC, on 30 October 2026 — so a paid
/// family's allowance renews "November 1". Leaked once per call: a test's lifetime.
pub fn days() -> fc_logic::paid_families::Days<'static> {
    let culture: &'static fc_logic::date_culture::DateCulture = Box::leak(Box::new(
        fc_logic::date_culture::DateCulture::named("en-US").expect("en-US is measured"),
    ));
    fc_logic::paid_families::Days::new(
        culture,
        &time::UtcOffset::UTC,
        time::macros::datetime!(2026-10-30 12:00 UTC),
    )
}

/// A new, empty folder of its own under the system's temporary directory (`TMPDIR`), named for
/// `label` — the C# tests' `Path.GetTempPath()` + a GUID. The caller removes it.
pub fn scratch_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    let folder = std::env::temp_dir().join(format!(
        "fc-{label}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    assert!(!folder.exists(), "a scratch folder is new");
    folder
}

/// A gate a fake answer waits on — the C# tests' `TaskCompletionSource` — opened once, for every
/// waiter, by [`Gate::open`]. Clones share it.
#[derive(Clone, Default)]
pub struct Gate(Rc<std::cell::RefCell<GateState>>);

#[derive(Default)]
struct GateState {
    open: bool,
    waiting: Vec<std::task::Waker>,
}

impl Gate {
    pub fn new() -> Self {
        Gate::default()
    }

    pub fn open(&self) {
        let waiting = {
            let mut state = self.0.borrow_mut();
            state.open = true;
            std::mem::take(&mut state.waiting)
        };
        for waker in waiting {
            waker.wake();
        }
    }

    pub fn is_open(&self) -> bool {
        self.0.borrow().open
    }

    /// A future that ends once the gate is open.
    pub fn wait(&self) -> impl std::future::Future<Output = ()> + 'static {
        let gate = self.clone();
        std::future::poll_fn(move |cx| {
            let mut state = gate.0.borrow_mut();
            if state.open {
                std::task::Poll::Ready(())
            } else {
                state.waiting.push(cx.waker().clone());
                std::task::Poll::Pending
            }
        })
    }
}
