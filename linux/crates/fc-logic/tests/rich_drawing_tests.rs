//! `RichDrawingTests`: the pure half of drawing formulas, diagrams and graphs (issue #97;
//! docs/capitals-and-rich-markdown-2026-10-17.md, "The native host"): the job table, the cache key,
//! the page's answers, the photograph's scale, and the two caches.
//!
//! The C# `RichStore` tests write a real temporary folder; here the store reaches the disk through
//! `RichFolder`, so they run against an in-memory folder whose clock the test moves — the same
//! steps and the same assertions. The `RichPage` tests are Windows' cases moved to Linux's origin,
//! `fc-rich://rich/…` (shell-design.md §9.1), each keeping what it pinned. After the C#'s tests
//! come the ones this port adds: the key, the JSON and the number text byte for byte as .NET 10
//! wrote them for the same jobs (measured with the C# itself), and the hand-written SHA-256 and
//! base64 against their standards.

mod common;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, SystemTime};

use fc_logic::rich_drawing::{
    RichAnswer, RichCapture, RichFit, RichFolder, RichJob, RichLook, RichMemory, RichPage,
    RichResult, RichStore, base64_decode, round_trip_text, sha256_hex,
};
use fc_text::markdown::RichKind;
use serde_json::Value;

const VERSION: &str = "1;katex-0.17.0;mermaid-11.16.0;viz-3.24.0;plot-md-1.6";

fn mine() -> RichLook {
    RichLook::new(false, "#FFFFFF", 14.0)
}

fn theirs_dark() -> RichLook {
    RichLook::new(true, "#FFFFFF", 14.0)
}

/// The folder as the store sees it: names, bytes, and when each was last written — on a clock the
/// test owns, which starts in 2026 and moves a second with every write or touch.
#[derive(Default)]
struct MemoryFolder {
    files: RefCell<BTreeMap<String, (Vec<u8>, SystemTime)>>,
    now: RefCell<u64>,
    created: RefCell<bool>,
}

impl MemoryFolder {
    fn tick(&self) -> SystemTime {
        let mut now = self.now.borrow_mut();
        *now += 1;
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000 + *now)
    }

    /// `File.SetLastWriteTimeUtc(path, DateTime.UtcNow.AddMinutes(minutes))`, from the folder's
    /// own now.
    fn set_modified_minutes(&self, name: &str, minutes: i64) {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000 + *self.now.borrow());
        let at = if minutes < 0 {
            now - Duration::from_secs(minutes.unsigned_abs() * 60)
        } else {
            now + Duration::from_secs(minutes as u64 * 60)
        };
        if let Some(file) = self.files.borrow_mut().get_mut(name) {
            file.1 = at;
        }
    }

    /// `File.WriteAllText` / `File.WriteAllBytes` from outside the store.
    fn put(&self, name: &str, bytes: &[u8]) {
        let at = self.tick();
        self.files
            .borrow_mut()
            .insert(name.to_string(), (bytes.to_vec(), at));
    }

    fn names(&self) -> Vec<String> {
        self.files.borrow().keys().cloned().collect()
    }

    fn missing(name: &str) -> io::Error {
        io::Error::new(io::ErrorKind::NotFound, name.to_string())
    }
}

impl RichFolder for MemoryFolder {
    fn exists(&self) -> bool {
        *self.created.borrow()
    }

    fn create(&self) -> io::Result<()> {
        *self.created.borrow_mut() = true;
        Ok(())
    }

    fn files(&self) -> io::Result<Vec<String>> {
        Ok(self.names())
    }

    fn is_file(&self, name: &str) -> bool {
        self.files.borrow().contains_key(name)
    }

    fn read(&self, name: &str) -> io::Result<Vec<u8>> {
        self.files
            .borrow()
            .get(name)
            .map(|file| file.0.clone())
            .ok_or_else(|| Self::missing(name))
    }

    fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        assert!(*self.created.borrow(), "written before the folder was made");
        self.put(name, bytes);
        Ok(())
    }

    fn replace(&self, from: &str, to: &str) -> io::Result<()> {
        let file = self
            .files
            .borrow_mut()
            .remove(from)
            .ok_or_else(|| Self::missing(from))?;
        self.files.borrow_mut().insert(to.to_string(), file);
        Ok(())
    }

    fn delete(&self, name: &str) -> io::Result<()> {
        self.files.borrow_mut().remove(name);
        Ok(())
    }

    fn modified(&self, name: &str) -> io::Result<SystemTime> {
        self.files
            .borrow()
            .get(name)
            .map(|file| file.1)
            .ok_or_else(|| Self::missing(name))
    }

    fn touch(&self, name: &str) -> io::Result<()> {
        let at = self.tick();
        let mut files = self.files.borrow_mut();
        let file = files.get_mut(name).ok_or_else(|| Self::missing(name))?;
        file.1 = at;
        Ok(())
    }
}

fn store() -> RichStore<MemoryFolder> {
    RichStore::new(MemoryFolder::default())
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{text}: {error}"))
}

/// The design doc's job table, row for row: a formula in the bubble's ink on nothing; a diagram on
/// the fixed card.
#[test]
fn jobs_follow_the_table() {
    assert_eq!(
        RichJob::new("math", "", "x^2", false, "#FFFFFF", "transparent", 14.0),
        RichJob::for_block(RichKind::Math, "", "x^2", &mine())
    );
    assert_eq!(
        RichJob::new(
            "math-inline",
            "",
            "x",
            false,
            "#FFFFFF",
            "transparent",
            14.0
        ),
        RichJob::for_inline("x", &mine())
    );
    assert_eq!(
        RichJob::new("mermaid", "", "graph TD", false, "#1C1C1E", "#FFFFFF", 14.0),
        RichJob::for_block(RichKind::Mermaid, "", "graph TD", &mine())
    );
    assert_eq!(
        RichJob::new(
            "graphviz", "neato", "graph{}", true, "#F2F2F7", "#1C1C1E", 14.0
        ),
        RichJob::for_block(RichKind::Graphviz, "neato", "graph{}", &theirs_dark())
    );
    // An engine on anything but Graphviz is nobody's business.
    assert_eq!(
        "",
        RichJob::for_block(RichKind::Plot, "dot", "sin(x)", &mine()).engine
    );
    assert!(RichJob::for_inline("x", &mine()).is_formula());
    assert!(!RichJob::for_block(RichKind::Plot, "", "x", &mine()).is_formula());
}

/// The key moves with every field the doc lists, and with nothing else.
#[test]
fn the_key_covers_every_field() {
    let job = RichJob::for_block(RichKind::Math, "", "x^2", &mine());
    let key = job.key(VERSION);
    assert_eq!(key.len(), 64);
    assert!(key.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    assert_eq!(
        key,
        RichJob::for_block(RichKind::Math, "", "x^2", &mine()).key(VERSION)
    );
    let others = [
        RichJob {
            kind: "math-inline".into(),
            ..job.clone()
        },
        RichJob {
            engine: "dot".into(),
            ..job.clone()
        },
        RichJob {
            dark: true,
            ..job.clone()
        },
        RichJob {
            color: "#000000".into(),
            ..job.clone()
        },
        RichJob {
            background: "#FFFFFF".into(),
            ..job.clone()
        },
        RichJob {
            font_size: 15.0,
            ..job.clone()
        },
        RichJob {
            source: "x^3".into(),
            ..job.clone()
        },
    ];
    for other in &others {
        assert_ne!(key, other.key(VERSION), "{other:?}");
    }
    assert_ne!(key, job.key(&format!("{VERSION}x")));
}

/// Fields go in with their lengths, so moving a character from one field to the next is a different
/// key.
#[test]
fn the_key_cannot_be_shifted_between_fields() {
    let a = RichJob::new("math", "a", "bc", false, "#FFF", "transparent", 14.0);
    let b = RichJob::new("math", "ab", "c", false, "#FFF", "transparent", 14.0);
    assert_ne!(a.key(VERSION), b.key(VERSION));
}

/// The job reaches the page as the object fc-rich.js reads, and a source can never break out of it.
#[test]
fn the_job_is_safe_json() {
    let hostile = "</script><script>alert('x')</script>\"\u{2028}`${x}`";
    let job = RichJob::for_block(RichKind::Graphviz, "", hostile, &mine());
    let text = job.json();
    assert!(!text.contains('<'));
    assert!(!text.contains('\''));
    assert!(!text.contains('\u{2028}'));
    let root = json(&text);
    assert_eq!(root["kind"], "graphviz");
    assert_eq!(root["source"], hostile);
    // Graphviz with no layout named is dot.
    assert_eq!(root["engine"], "dot");
    assert_eq!(root["dark"], false);
    assert_eq!(root["fontSize"].as_f64(), Some(14.0));
    assert!(root.get("mathml").is_none());
    assert!(
        json(&RichJob::for_inline("x", &mine()).json())
            .get("engine")
            .is_none()
    );
    assert!(job.script(7).starts_with("fcRich.drawAndPost(7,{"));
    assert!(job.script(7).ends_with("})"));
}

#[test]
fn colours_are_css() {
    let cases: [(u8, u8, u8, u8, &str); 3] = [
        (0xFF, 0xFF, 0xFF, 0xFF, "#FFFFFF"),
        (0xE4, 0x00, 0x00, 0x00, "#000000E4"),
        (0xFF, 0x1C, 0x1C, 0x1E, "#1C1C1E"),
    ];
    for (a, r, g, b, css) in cases {
        assert_eq!(css, RichJob::css_color(a, r, g, b));
    }
}

/// The page's answers: a drawing, a failure in the engine's words, and the shapes that are neither.
#[test]
fn answers_parse() {
    assert_eq!(
        Some((3, RichResult::drawn(120.0, 40.0, 30.0))),
        RichAnswer::parse(
            "{\"id\":3,\"result\":{\"ok\":true,\"width\":120,\"height\":40,\"baseline\":30}}"
        )
    );
    let failed =
        RichAnswer::parse("{\"id\":4,\"result\":{\"ok\":false,\"error\":\"KaTeX parse error\"}}");
    assert_eq!(Some(4), failed.as_ref().map(|(id, _)| *id));
    assert_eq!(Some(false), failed.as_ref().map(|(_, result)| result.ok));
    assert_eq!(
        Some("KaTeX parse error"),
        failed
            .as_ref()
            .and_then(|(_, result)| result.error.as_deref())
    );
    let error = |text: &str| RichAnswer::parse(text).and_then(|(_, result)| result.error);
    let baseline = |text: &str| RichAnswer::parse(text).map(|(_, result)| result.baseline);
    // Drew, it says, but nothing to photograph.
    assert_eq!(
        Some("empty".to_string()),
        error("{\"id\":5,\"result\":{\"ok\":true,\"width\":0,\"height\":40}}")
    );
    // No baseline (a diagram) is its foot; one outside the box is put back inside it.
    assert_eq!(
        Some(40.0),
        baseline("{\"id\":6,\"result\":{\"ok\":true,\"width\":10,\"height\":40}}")
    );
    assert_eq!(
        Some(40.0),
        baseline("{\"id\":6,\"result\":{\"ok\":true,\"width\":10,\"height\":40,\"baseline\":90}}")
    );
    assert_eq!(
        Some(0.0),
        baseline("{\"id\":6,\"result\":{\"ok\":true,\"width\":10,\"height\":40,\"baseline\":-3}}")
    );
    assert_eq!(None, RichAnswer::parse("not json"));
    assert_eq!(None, RichAnswer::parse("{\"event\":\"ready\"}"));
    assert_eq!(None, RichAnswer::parse("{\"id\":\"3\",\"result\":{}}"));
    assert_eq!(None, RichAnswer::parse("[1,2]"));
}

#[test]
fn metrics_round_trip() {
    let drawn = RichResult::drawn(120.5, 40.0, 31.25);
    assert_eq!(
        Some(drawn.clone()),
        RichAnswer::read_metrics(&RichAnswer::metrics(&drawn))
    );
    assert_eq!(None, RichAnswer::read_metrics("{\"width\":0,\"height\":1}"));
    assert_eq!(None, RichAnswer::read_metrics("garbage"));
}

/// Only the page's own folder, and data: URLs — nothing that leaves the machine. Windows' cases on
/// Linux's origin: its virtual host is the `fc-rich` scheme's `rich` host here, and Windows' own
/// address is now one more place the page may not reach.
#[test]
fn the_page_reaches_only_its_own_folder() {
    let cases: [(Option<&str>, bool); 15] = [
        (Some("fc-rich://rich/rich.html"), true),
        (Some("fc-rich://rich/fonts/KaTeX_Main-Regular.woff2"), true),
        (Some("fc-rich://RICH/katex.min.js"), true),
        (Some("data:image/png;base64,AAAA"), true),
        // Another scheme to the same place (Windows: http for https).
        (Some("https://rich/rich.html"), false),
        (Some("fc-rich://rich:8443/rich.html"), false),
        // Another host of the scheme (Windows: call.familyconnect.invalid).
        (Some("fc-rich://call/call.html"), false),
        (Some("https://fc.nettrash.me/"), false),
        (Some("fc-rich://rich.example.com/x"), false),
        (Some("blob:fc-rich://rich/1"), false),
        (
            Some("file:///usr/share/family-connect/rich/rich.html"),
            false,
        ),
        (Some(""), false),
        (None, false),
        // Windows' page is not Linux's.
        (Some("https://rich.familyconnect.invalid/rich.html"), false),
        // Linux is stricter than `Uri.TryCreate` about who claims to be the host.
        (Some("fc-rich://evil@rich/rich.html"), false),
    ];
    for (uri, allowed) in cases {
        assert_eq!(allowed, RichPage::allows(uri), "{uri:?}");
    }
}

#[test]
fn navigation_is_to_the_page_alone() {
    assert!(RichPage::is_page(Some("fc-rich://rich/rich.html")));
    assert!(!RichPage::is_page(Some("fc-rich://rich/fc-rich.js")));
    assert!(!RichPage::is_page(Some("fc-rich://rich/rich.html?x")));
    assert!(!RichPage::is_page(Some("about:blank")));
    assert!(!RichPage::is_page(None));
    // Windows' page is another document.
    assert!(!RichPage::is_page(Some(
        "https://rich.familyconnect.invalid/rich.html"
    )));
}

/// The version is read out of the very file the app ships — so every copy gets the same key as
/// web/rich.
#[test]
fn the_version_is_read_from_the_shipped_script() {
    assert_eq!(
        Some("1;a"),
        RichPage::version_of("  var VERSION = '1;a';\n")
    );
    assert_eq!(None, RichPage::version_of("var VERSIONS = 'x';"));
    assert_eq!(None, RichPage::version_of("nothing here"));
    let script = common::fixture_text("fc-rich.js");
    assert_eq!(Some(VERSION), RichPage::version_of(&script));
}

/// The screen's scale, until the longest side would pass 4096 pixels.
#[test]
fn the_scale_is_capped() {
    let cases: [(f64, f64, f64, f64); 6] = [
        (1.5, 200.0, 100.0, 1.5),
        (2.0, 1200.0, 3000.0, 4096.0 / 3000.0),
        (1.0, 4096.0, 10.0, 1.0),
        (1.0, 5000.0, 10.0, 4096.0 / 5000.0),
        (0.0, 100.0, 100.0, 1.0),
        (f64::NAN, 100.0, 100.0, 1.0),
    ];
    for (screen, width, height, expected) in cases {
        // xUnit's `Assert.Equal(expected, actual, 9)`: equal to nine decimal places.
        let actual = RichCapture::scale_for(screen, width, height);
        assert!(
            (expected - actual).abs() < 5e-10,
            "{screen} {width}×{height}: {actual}"
        );
    }
}

#[test]
fn the_capture_asks_for_the_drawing_at_the_top_left() {
    let root = json(&RichCapture::arguments(120.0, 40.0, 1.5));
    assert_eq!(root["format"], "png");
    assert_eq!(root["captureBeyondViewport"], true);
    let clip = &root["clip"];
    assert_eq!(
        (Some(0.0), Some(0.0), Some(120.0), Some(40.0), Some(1.5)),
        (
            clip["x"].as_f64(),
            clip["y"].as_f64(),
            clip["width"].as_f64(),
            clip["height"].as_f64(),
            clip["scale"].as_f64()
        )
    );
    assert_eq!(
        Some(1.0),
        json(&RichCapture::metrics())["deviceScaleFactor"].as_f64()
    );
    assert_eq!(
        Some(0.0),
        json(RichCapture::CLEAR_BACKGROUND)["color"]["a"].as_f64()
    );
    assert_eq!(Some(vec![1, 2, 3]), RichCapture::png("{\"data\":\"AQID\"}"));
    assert_eq!(None, RichCapture::png("{\"data\":\"***\"}"));
    assert_eq!(None, RichCapture::png("{}"));
    assert_eq!(None, RichCapture::png("nope"));
}

/// An inline formula drops by what hangs below its baseline, and a too-wide one shrinks whole,
/// baseline with it.
#[test]
fn inline_formulas_sit_on_the_baseline() {
    assert_eq!((40.0, 20.0, 5.0), RichFit::inline(40.0, 20.0, 15.0, 500.0));
    assert_eq!(
        (500.0, 10.0, 2.5),
        RichFit::inline(1000.0, 20.0, 15.0, 500.0)
    );
    assert_eq!((10.0, 20.0, 0.0), RichFit::inline(10.0, 20.0, 20.0, 500.0));
}

#[test]
fn memory_lets_the_least_recently_used_go() {
    let mut memory = RichMemory::<i32>::new(2);
    memory.put("a", 1);
    memory.put("b", 2);
    assert!(memory.try_get("a").is_some());
    memory.put("c", 3);
    assert!(memory.try_get("b").is_none());
    assert_eq!(Some(&1), memory.try_get("a"));
    memory.put("a", 9);
    assert_eq!(2, memory.count());
    assert_eq!(Some(&9), memory.try_get("a"));
    memory.clear();
    assert_eq!(0, memory.count());
    // `Assert.Throws<ArgumentOutOfRangeException>`: a memory that keeps nothing is refused.
    assert!(std::panic::catch_unwind(|| RichMemory::<i32>::new(0)).is_err());
}

/// Pixels are the budget too: a heavy drawing pushes the old ones out, and the newest always stays.
#[test]
fn memory_keeps_to_its_weight() {
    let mut memory = RichMemory::<&str>::with_budget(10, 100);
    memory.put_weighing("a", "a", 40);
    memory.put_weighing("b", "b", 40);
    memory.put_weighing("failed", "failed", 0);
    memory.put_weighing("c", "c", 40);
    assert!(memory.try_get("a").is_none());
    assert!(memory.try_get("b").is_some());
    assert_eq!(80, memory.weight());
    memory.put_weighing("huge", "huge", 500);
    assert_eq!(1, memory.count());
    assert!(memory.try_get("huge").is_some());
    memory.put_weighing("huge", "smaller", 10);
    assert_eq!(10, memory.weight());
}

#[test]
fn the_disk_keeps_drawings_with_their_metrics() {
    let store = store();
    let key = RichJob::for_inline("x", &mine()).key(VERSION);
    assert_eq!(None, store.read(&key));
    store.write(&key, &[1, 2, 3], &RichResult::drawn(10.0, 20.0, 15.0));
    let read = store.read(&key);
    assert_eq!(
        Some(vec![1, 2, 3]),
        read.as_ref().map(|(png, _)| png.clone())
    );
    assert_eq!(
        Some(RichResult::drawn(10.0, 20.0, 15.0)),
        read.map(|(_, metrics)| metrics)
    );
    // A failure is never kept on disk, nor is a key that is not one.
    let other = RichJob::for_inline("y", &mine()).key(VERSION);
    store.write(&other, &[1], &RichResult::failed("bad"));
    assert_eq!(None, store.read(&other));
    store.write("../escape", &[1], &RichResult::drawn(1.0, 1.0, 1.0));
    assert!(
        store
            .folder()
            .names()
            .iter()
            .all(|name| !name.contains("escape"))
    );
    assert_eq!(None, store.read("../escape"));
}

#[test]
fn a_broken_drawing_is_drawn_again() {
    let store = store();
    let key = RichJob::for_inline("x", &mine()).key(VERSION);
    store.write(&key, &[1, 2, 3], &RichResult::drawn(10.0, 20.0, 15.0));
    store.folder().put(&format!("{key}.json"), b"{broken");
    assert_eq!(None, store.read(&key));
    assert!(!store.folder().is_file(&format!("{key}.png")));
}

#[test]
fn trim_keeps_the_most_recently_used() {
    let store = store();
    let keys: Vec<String> = (0..4)
        .map(|index| RichJob::for_inline(&format!("x{index}"), &mine()).key(VERSION))
        .collect();
    for (index, key) in keys.iter().enumerate() {
        store.write(key, &[1], &RichResult::drawn(1.0, 1.0, 1.0));
        store
            .folder()
            .set_modified_minutes(&format!("{key}.json"), index as i64 - 10);
    }
    // Reading the oldest makes it the newest.
    assert!(store.read(&keys[0]).is_some());
    store.folder().put("stray.png.part", &[1]);
    store.folder().put(&format!("{}.png", "a".repeat(64)), &[1]);
    store.trim_to(2);
    assert!(store.read(&keys[0]).is_some());
    assert!(store.read(&keys[3]).is_some());
    assert!(store.read(&keys[1]).is_none());
    assert!(store.read(&keys[2]).is_none());
    assert_eq!(4, store.folder().names().len());
}

// ---- added by the port: Windows' bytes, measured ------------------------------------------------

/// The cache key is Windows' own for the same job — measured by running `RichJob.Key` on .NET 10 —
/// including a source past ASCII, whose length is counted in UTF-16 units, a size with a fraction, a
/// size .NET writes in full (`1E+15` is `1000000000000000`), and a size that is not a number.
#[test]
fn the_key_is_windows_key_byte_for_byte() {
    let dark = RichLook::new(true, "#000000E4", 15.5);
    assert_eq!(
        "6e025927a995cd7f17160a45bbdc42e75f439622e886c409017f17ab5f0cc59a",
        RichJob::for_block(RichKind::Math, "", "x^2", &mine()).key(VERSION)
    );
    assert_eq!(
        "079521485e163d0a5992a20828a747e408fb27db1448b089de5ae27bad77c704",
        RichJob::for_inline("Привет 😀 \\frac{a}{b}", &dark).key(VERSION)
    );
    assert_eq!(
        "c326735205e283ed6609c60c59ce2cbcac0ba8b185c5765bc0f85e5ea3d44dd4",
        RichJob::for_block(RichKind::Graphviz, "neato", "graph { a -- b }", &dark).key(VERSION)
    );
    assert_eq!(
        "05b3b1655904caa57326230c44ba2eacd279c2d630909898ad984ef0b53805fc",
        RichJob::new("plot", "", "y = sin(x)", false, "#1C1C1E", "#FFFFFF", 0.1).key(VERSION)
    );
    assert_eq!(
        "53c233034013d8d5309e4ef35bef8dd3aea3de2b5de9f03ad63abe6ba260b7e4",
        RichJob::new("math", "", "x", true, "#FFF", "transparent", 1e15).key("")
    );
    assert_eq!(
        "49f518e287de333a8b8b860bfd233ac3cdd1f211dff25ada20864e3b7c7f8042",
        RichJob::new("math", "", "x", true, "#FFF", "transparent", f64::NAN).key(VERSION)
    );
}

/// The job's JSON is the bytes `Utf8JsonWriter` writes on .NET 10 — every escape as it escapes it.
#[test]
fn the_json_is_windows_json_byte_for_byte() {
    let hostile = "</script><script>alert('x')</script>\"\u{2028}`${x}`&+\\\n\té😀";
    assert_eq!(
        "{\"kind\":\"graphviz\",\"source\":\"\\u003C/script\\u003E\\u003Cscript\\u003Ealert(\\u0027x\\u0027)\\u003C/script\\u003E\\u0022\\u2028\\u0060${x}\\u0060\\u0026\\u002B\\\\\\n\\t\\u00E9\\uD83D\\uDE00\",\"engine\":\"dot\",\"dark\":false,\"color\":\"#1C1C1E\",\"background\":\"#FFFFFF\",\"fontSize\":14}",
        RichJob::for_block(RichKind::Graphviz, "", hostile, &mine()).json()
    );
    assert_eq!(
        "{\"kind\":\"math-inline\",\"source\":\"x\",\"dark\":false,\"color\":\"#FFFFFF\",\"background\":\"transparent\",\"fontSize\":14.25}",
        RichJob::for_inline("x", &RichLook::new(false, "#FFFFFF", 14.25)).json()
    );
    assert_eq!(
        "fcRich.drawAndPost(7,{\"kind\":\"mermaid\",\"source\":\"graph TD\",\"dark\":true,\"color\":\"#F2F2F7\",\"background\":\"#1C1C1E\",\"fontSize\":15.5})",
        RichJob::for_block(
            RichKind::Mermaid,
            "",
            "graph TD",
            &RichLook::new(true, "#000000E4", 15.5)
        )
        .script(7)
    );
    // Every ASCII character `Utf8JsonWriter` escapes, and how; the rest go as they are.
    let all: String = (0u8..0x80).map(char::from).collect();
    let source = json(&RichJob::for_inline(&all, &mine()).json());
    assert_eq!(source["source"], all.as_str());
    let written = RichJob::for_inline(&all, &mine()).json();
    let escaped = written
        .split("\"source\":\"")
        .nth(1)
        .and_then(|rest| rest.split("\",\"dark\"").next())
        .unwrap();
    let mut expected = String::new();
    for c in all.chars() {
        expected.push_str(&match c {
            '\u{8}' => "\\b".to_string(),
            '\t' => "\\t".to_string(),
            '\n' => "\\n".to_string(),
            '\u{c}' => "\\f".to_string(),
            '\r' => "\\r".to_string(),
            '\\' => "\\\\".to_string(),
            c if c < ' ' || matches!(c, '"' | '&' | '\'' | '+' | '<' | '>' | '`' | '\u{7F}') => {
                format!("\\u{:04X}", c as u32)
            }
            c => c.to_string(),
        });
    }
    assert_eq!(expected, escaped);
    // A size that is not a number is `null` (fc-rich.js then sets 16), where .NET throws.
    assert_eq!(
        json(&RichJob::for_inline("x", &RichLook::new(false, "#FFFFFF", f64::NAN)).json())["fontSize"],
        Value::Null
    );
}

/// The metrics file and the capture's arguments, as .NET 10 writes them.
#[test]
fn the_metrics_and_arguments_are_windows_text() {
    assert_eq!(
        "{\"width\":120.5,\"height\":40,\"baseline\":31.25}",
        RichAnswer::metrics(&RichResult::drawn(120.5, 40.0, 31.25))
    );
    assert_eq!(
        "{\"width\":1000000000000000,\"height\":0.1,\"baseline\":1E-05}",
        RichAnswer::metrics(&RichResult::drawn(1e15, 0.1, 1e-5))
    );
    assert_eq!(
        "{\"format\":\"png\",\"captureBeyondViewport\":true,\"clip\":{\"x\":0,\"y\":0,\"width\":120,\"height\":40,\"scale\":1.5}}",
        RichCapture::arguments(120.0, 40.0, 1.5)
    );
    assert_eq!(
        "{\"format\":\"png\",\"captureBeyondViewport\":true,\"clip\":{\"x\":0,\"y\":0,\"width\":1,\"height\":10000000000000000,\"scale\":1.3653333333333333}}",
        RichCapture::arguments(0.5, 1e16, 4096.0 / 3000.0)
    );
    assert_eq!(
        "{\"width\":1280,\"height\":800,\"deviceScaleFactor\":1,\"mobile\":false}",
        RichCapture::metrics()
    );
}

/// `double.ToString("R", InvariantCulture)` on .NET 10, value by value: fixed from 1E-04 to below
/// 1E+17, scientific outside, at least two exponent digits.
#[test]
fn numbers_are_written_as_dotnet_writes_them() {
    let table: &[(f64, &str)] = &[
        (14.0, "14"),
        (14.5, "14.5"),
        (0.1, "0.1"),
        (1e15, "1000000000000000"),
        (1e14, "100000000000000"),
        (999999999999999.0, "999999999999999"),
        (1e-5, "1E-05"),
        (0.0001, "0.0001"),
        (0.00012345, "0.00012345"),
        (123456789012345.67, "123456789012345.67"),
        (1.2345678901234567e16, "12345678901234568"),
        (-0.0, "-0"),
        (0.0, "0"),
        (1e21, "1E+21"),
        (5e-324, "5E-324"),
        (f64::MAX, "1.7976931348623157E+308"),
        (1.0 / 3.0, "0.3333333333333333"),
        (4096.0 / 3000.0, "1.3653333333333333"),
        (31.25, "31.25"),
        (1e16, "10000000000000000"),
        (1.5e300, "1.5E+300"),
        (2.5e-7, "2.5E-07"),
        (-1e-7, "-1E-07"),
        (100.0, "100"),
        (1e5, "100000"),
        (12345678901234567890.0, "1.2345678901234567E+19"),
        (0.30000000000000004, "0.30000000000000004"),
        (1e17, "1E+17"),
        (9.999999999999998e16, "99999999999999980"),
        (1.2345678901234568e17, "1.2345678901234568E+17"),
        (0.001234, "0.001234"),
        (0.00001234, "1.234E-05"),
        (1.5e17, "1.5E+17"),
        (0.5, "0.5"),
        (1e22, "1E+22"),
        (1e100, "1E+100"),
        (1.7e-308, "1.7E-308"),
        (2.2250738585072014e-308, "2.2250738585072014E-308"),
        (1.1e16, "11000000000000000"),
        (f64::NAN, "NaN"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
    ];
    for &(value, text) in table {
        assert_eq!(text, round_trip_text(value), "{value:e}");
    }
}

/// `Convert.FromBase64String`'s acceptance, case by case as .NET 10 answered (`None` where it threw).
#[test]
fn base64_is_read_as_dotnet_reads_it() {
    let table: &[(&str, Option<&[u8]>)] = &[
        ("AQID", Some(&[1, 2, 3])),
        ("AQI=", Some(&[1, 2])),
        ("AQ==", Some(&[1])),
        ("AQI", None),
        ("A", None),
        ("", Some(&[])),
        (" AQID ", Some(&[1, 2, 3])),
        ("AQ ID", Some(&[1, 2, 3])),
        ("A\tQ\r\nID", Some(&[1, 2, 3])),
        ("AQ=D", None),
        ("AQ==AQID", None),
        ("AR==", Some(&[1])),
        ("AQJ=", Some(&[1, 2])),
        ("***", None),
        ("AQID====", None),
        ("AQ=", None),
        ("=AQI", None),
        ("AQID\u{A0}", None),
        ("AQ\u{3000}ID", None),
        ("-_8=", None),
        ("AQ===", None),
        ("AQ= =", Some(&[1])),
        (" ", Some(&[])),
        ("AQ==\n", Some(&[1])),
        ("A Q = =", Some(&[1])),
        ("\u{B}AQID", None),
        ("AQID\u{C}", None),
    ];
    for &(text, bytes) in table {
        assert_eq!(bytes.map(<[u8]>::to_vec), base64_decode(text), "{text:?}");
    }
    // The capture's answer as .NET read it: empty data, null, a number, an array, two `data`s (the
    // last wins), and text after the object.
    assert_eq!(None, RichCapture::png("{\"data\":\"\"}"));
    assert_eq!(None, RichCapture::png("{\"data\":null}"));
    assert_eq!(None, RichCapture::png("{\"data\":7}"));
    assert_eq!(None, RichCapture::png("[\"AQID\"]"));
    assert_eq!(
        Some(vec![1]),
        RichCapture::png("{\"data\":\"AQID\",\"data\":\"AQ==\"}")
    );
    assert_eq!(None, RichCapture::png("{\"data\":\"AQID\"} x"));
}

/// FIPS 180-2's known answers, and a message of more than one block.
#[test]
fn sha256_gives_the_standards_answers() {
    assert_eq!(
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        sha256_hex(b"")
    );
    assert_eq!(
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        sha256_hex(b"abc")
    );
    assert_eq!(
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")
    );
    assert_eq!(
        "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1",
        sha256_hex(
            b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"
        )
    );
    assert_eq!(
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
        sha256_hex(&vec![b'a'; 1_000_000])
    );
    // 55 and 56 bytes: the last length that fits one padded block, and the first that does not.
    assert_eq!(
        "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
        sha256_hex(&[b'a'; 55])
    );
    assert_eq!(
        "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a",
        sha256_hex(&[b'a'; 56])
    );
}

/// The page's answers at their edges, as .NET 10 read them.
#[test]
fn answers_parse_at_the_edges_as_dotnet_reads_them() {
    let parse = |text: &str| RichAnswer::parse(text);
    // An id is a 32-bit JSON integer.
    assert_eq!(
        None,
        parse("{\"id\":3.0,\"result\":{\"ok\":true,\"width\":10,\"height\":40}}")
    );
    assert_eq!(
        None,
        parse("{\"id\":3e0,\"result\":{\"ok\":true,\"width\":10,\"height\":40}}")
    );
    assert_eq!(
        None,
        parse("{\"id\":2147483648,\"result\":{\"ok\":true,\"width\":10,\"height\":40}}")
    );
    assert_eq!(
        Some(i32::MIN),
        parse("{\"id\":-2147483648,\"result\":{\"ok\":true,\"width\":10,\"height\":40}}")
            .map(|(id, _)| id)
    );
    // A key said twice: the last.
    assert_eq!(
        Some(2),
        parse("{\"id\":1,\"id\":2,\"result\":{\"ok\":true,\"width\":10,\"height\":40}}")
            .map(|(id, _)| id)
    );
    // `ok` must be the literal true; an error that is not a string is "error".
    let error = |text: &str| parse(text).and_then(|(_, result)| result.error);
    assert_eq!(
        Some("error".to_string()),
        error("{\"id\":1,\"result\":{\"ok\":\"true\",\"width\":10,\"height\":40}}")
    );
    assert_eq!(
        Some("error".to_string()),
        error("{\"id\":1,\"result\":{\"ok\":false,\"error\":7}}")
    );
    assert_eq!(
        Some("error".to_string()),
        error("{\"id\":1,\"result\":{\"ok\":false,\"error\":null}}")
    );
    // A size that is a string, or not positive, is nothing to photograph.
    assert_eq!(
        Some("empty".to_string()),
        error("{\"id\":1,\"result\":{\"ok\":true,\"width\":\"10\",\"height\":40}}")
    );
    assert_eq!(
        Some("empty".to_string()),
        error("{\"id\":1,\"result\":{\"ok\":true,\"width\":10,\"height\":-40}}")
    );
    // A baseline that is not a number is the foot.
    assert_eq!(
        Some(40.0),
        parse("{\"id\":1,\"result\":{\"ok\":true,\"width\":10,\"height\":40,\"baseline\":\"5\"}}")
            .map(|(_, result)| result.baseline)
    );
    // Not JSON as .NET's reader takes it: a trailing comma, a comment, text after it, a BOM.
    for text in [
        "{\"id\":1,\"result\":{\"ok\":true,\"width\":10,\"height\":40},}",
        "{\"id\":1,/*c*/\"result\":{\"ok\":true,\"width\":10,\"height\":40}}",
        "{\"id\":1,\"result\":{\"ok\":true,\"width\":10,\"height\":40}}x",
        "\u{FEFF}{\"id\":1,\"result\":{\"ok\":true,\"width\":10,\"height\":40}}",
    ] {
        assert_eq!(None, parse(text), "{text}");
    }
    assert!(parse("{\"id\":1,\"result\":{\"ok\":true,\"width\":10,\"height\":40}} ").is_some());
    // The metrics file: `ok` is not asked, a baseline past the foot is the foot.
    assert_eq!(
        Some(RichResult::drawn(1.0, 2.0, 2.0)),
        RichAnswer::read_metrics("{\"width\":1,\"height\":2,\"ok\":false}")
    );
    assert_eq!(
        Some(RichResult::drawn(1.0, 2.0, 2.0)),
        RichAnswer::read_metrics("{\"width\":1,\"height\":2,\"baseline\":9}")
    );
    assert_eq!(None, RichAnswer::read_metrics("[]"));
    assert_eq!(None, RichAnswer::read_metrics(""));
}

/// The version pattern at its edges, as .NET's `Regex` matched it.
#[test]
fn the_version_pattern_matches_as_dotnet_matched_it() {
    let table: &[(&str, Option<&str>)] = &[
        ("\tvar\u{A0}VERSION='x'", Some("x")),
        ("xvar VERSION = 'a'", None),
        ("_var VERSION='a'", None),
        ("évar VERSION='a'", None),
        (".var VERSION='a'", Some("a")),
        ("var VERSION = ''", None),
        ("var VERSION = 'a\nb' var VERSION='c'", Some("c")),
        ("var  VERSION\t=\t'1;2' ", Some("1;2")),
        ("var\u{2028}VERSION='z'", Some("z")),
        ("varVERSION='q'", None),
        ("var VERSION = \"d\"", None),
        ("var VERSION = 'a'b'", Some("a")),
        ("1var VERSION='n'", None),
        ("\u{301}var VERSION='m'", None),
    ];
    for &(script, version) in table {
        assert_eq!(version, RichPage::version_of(script), "{script:?}");
    }
}

/// A store whose folder fails swallows it: nothing is read, nothing panics.
#[test]
fn a_failing_folder_is_a_cache_that_keeps_nothing() {
    struct Broken;
    impl RichFolder for Broken {
        fn exists(&self) -> bool {
            true
        }
        fn create(&self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
        fn files(&self) -> io::Result<Vec<String>> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
        fn is_file(&self, _: &str) -> bool {
            false
        }
        fn read(&self, _: &str) -> io::Result<Vec<u8>> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
        fn write(&self, _: &str, _: &[u8]) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
        fn replace(&self, _: &str, _: &str) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
        fn delete(&self, _: &str) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
        fn modified(&self, _: &str) -> io::Result<SystemTime> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
        fn touch(&self, _: &str) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
    }
    let store = RichStore::new(Broken);
    let key = RichJob::for_inline("x", &mine()).key(VERSION);
    store.write(&key, &[1], &RichResult::drawn(1.0, 1.0, 1.0));
    assert_eq!(None, store.read(&key));
    store.trim();
}
