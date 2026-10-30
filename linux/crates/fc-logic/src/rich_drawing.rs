//! Formulas, diagrams and graphs drawn as pictures (`RichDrawing.cs`; issue #97;
//! docs/capitals-and-rich-markdown-2026-10-17.md, "The native host"): the job a bubble hands
//! `fc-rich.js`, the key a drawing is kept under, the page's answers, the photograph's scale, how an
//! inline formula sits on the line, and the two caches — everything the shell's `rich::RichRenderer`
//! decides with no WebKit in it (`linux/docs/shell-design.md` §9).
//!
//! THREE THINGS ARE WRITTEN BY HAND HERE that the C# took from .NET, because fc-logic's dependencies
//! are std, `serde_json` and the family's own crates (port-plan "Dependencies"):
//!
//! - SHA-256 for the cache key ([`sha256_hex`]), pinned by FIPS 180's known answers;
//! - the job's JSON ([`RichJob::json`]), escaped exactly as `Utf8JsonWriter`'s default encoder
//!   escapes it — `<`, `>`, `&`, `'`, `+`, `` ` ``, `"` and everything outside printable ASCII as
//!   `\uXXXX` (measured on .NET 10) — because the object goes into a script as a JavaScript literal
//!   and must be the SAME bytes Windows writes;
//! - .NET's round-trip number text (`ToString("R")`, [`round_trip_text`]): the cache key hashes the
//!   text size as .NET writes it, so a key differs from Windows' if a single digit or exponent
//!   differs.
//!
//! And the base64 `RichCapture.Png` reads ([`RichCapture::png`]) — a DevTools answer, which only
//! Windows' capture produces; kept because Appendix B keeps `RichCapture` unchanged.
//!
//! THE PAGE IS LINUX'S OWN (`RichPage`, Appendix B item 5): WebKitGTK serves `web/rich` from the
//! custom scheme `fc-rich`, host `rich` (§9.1), where WebView2 used the virtual host
//! `https://rich.familyconnect.invalid`.
//!
//! Not thread-safe, nothing here: the GLib main context's, as the C# is the window thread's.

use std::collections::HashMap;
use std::io;
use std::time::{Duration, SystemTime};

use fc_text::markdown::RichKind;
use serde_json::{Map, Value};

// ---- the look and the job --------------------------------------------------------------------

/// How one bubble wants its formulas, diagrams and graphs drawn: the theme, the bubble's own text
/// colour — an own bubble's white is not another's near-black — and its body text size, in the CSS
/// pixels `fc-rich.js` measures in (GTK's logical pixels on Linux).
#[derive(Debug, Clone, PartialEq)]
pub struct RichLook {
    pub dark: bool,
    /// A CSS colour ([`RichJob::css_color`]).
    pub text_color: String,
    pub font_size: f64,
}

impl RichLook {
    pub fn new(dark: bool, text_color: &str, font_size: f64) -> Self {
        RichLook {
            dark,
            text_color: text_color.to_string(),
            font_size,
        }
    }
}

/// One job for `fc-rich.js`, exactly the object its header describes, built from a bubble's block
/// and its [`RichLook`] by the design doc's job table — and the key a drawing of it is kept under.
#[derive(Debug, Clone, PartialEq)]
pub struct RichJob {
    /// `math`, `math-inline`, `mermaid`, `graphviz` or `plot`.
    pub kind: String,
    /// Graphviz's layout; empty for every other kind.
    pub engine: String,
    /// What to draw, exactly as written — never the delimiters a bubble shows around it.
    pub source: String,
    pub dark: bool,
    /// The ink: the bubble's text colour for a formula, the card's for a diagram or a graph.
    pub color: String,
    /// `transparent` for a formula; the card's colour for a diagram or a graph.
    pub background: String,
    pub font_size: f64,
}

impl RichJob {
    /// The card a diagram or a graph is drawn on, light and dark — fixed, whoever's bubble it is in
    /// (the job table).
    pub const CARD_LIGHT: &'static str = "#FFFFFF";

    pub const CARD_DARK: &'static str = "#1C1C1E";

    /// The ink on that card.
    pub const CARD_INK_LIGHT: &'static str = "#1C1C1E";

    pub const CARD_INK_DARK: &'static str = "#F2F2F7";

    /// A formula has no ground of its own: it sits on the bubble's.
    pub const TRANSPARENT: &'static str = "transparent";

    /// `new RichJob(kind, engine, source, dark, color, background, fontSize)`.
    pub fn new(
        kind: &str,
        engine: &str,
        source: &str,
        dark: bool,
        color: &str,
        background: &str,
        font_size: f64,
    ) -> Self {
        RichJob {
            kind: kind.to_string(),
            engine: engine.to_string(),
            source: source.to_string(),
            dark,
            color: color.to_string(),
            background: background.to_string(),
            font_size,
        }
    }

    /// The job for a block standing on its own: a display formula, a diagram or a graph.
    pub fn for_block(kind: RichKind, engine: &str, source: &str, look: &RichLook) -> Self {
        if kind == RichKind::Math {
            return RichJob::new(
                kind.word(),
                "",
                source,
                look.dark,
                &look.text_color,
                Self::TRANSPARENT,
                look.font_size,
            );
        }
        RichJob::new(
            kind.word(),
            if kind == RichKind::Graphviz {
                engine
            } else {
                ""
            },
            source,
            look.dark,
            if look.dark {
                Self::CARD_INK_DARK
            } else {
                Self::CARD_INK_LIGHT
            },
            if look.dark {
                Self::CARD_DARK
            } else {
                Self::CARD_LIGHT
            },
            look.font_size,
        )
    }

    /// The job for a formula inside a line of text.
    pub fn for_inline(source: &str, look: &RichLook) -> Self {
        RichJob::new(
            "math-inline",
            "",
            source,
            look.dark,
            &look.text_color,
            Self::TRANSPARENT,
            look.font_size,
        )
    }

    /// A formula, drawn in the bubble's ink on nothing; anything else is a diagram or a graph on a
    /// card.
    pub fn is_formula(&self) -> bool {
        self.kind == "math" || self.kind == "math-inline"
    }

    /// The cache key: SHA-256, in lower-case hex, over `fcRich.version`, the kind, the engine, the
    /// theme, both colours, the text size and the source — the doc's list, nothing more. Each field
    /// goes in with its length in front, so no source can end one field and start the next
    /// (`a`+`bc` is never `ab`+`c`). The version is what retires every drawing an older engine made.
    ///
    /// BYTE FOR BYTE WINDOWS' KEY, so a drawing kept by one client's rules is found by the other's:
    /// the length is counted in UTF-16 units, as a C# `string.Length` counts it (a source with an
    /// emoji in it is one unit longer than its scalars), the size is .NET's round-trip text
    /// ([`round_trip_text`]), and the text hashed is UTF-8.
    pub fn key(&self, version: &str) -> String {
        let size = round_trip_text(self.font_size);
        let mut text = String::new();
        for field in [
            version,
            &self.kind,
            &self.engine,
            if self.dark { "1" } else { "0" },
            &self.color,
            &self.background,
            &size,
            &self.source,
        ] {
            // UTF-16 units: C#'s `string.Length`.
            text.push_str(&field.encode_utf16().count().to_string());
            text.push(':');
            text.push_str(field);
            text.push('\n');
        }
        sha256_hex(text.as_bytes())
    }

    /// The job as the JSON object `fc-rich.js` reads. `engine` only for Graphviz, which is the only
    /// engine that reads it; never `mathml`, which would draw a snapshotted formula twice. Escaped
    /// as `Utf8JsonWriter`'s default encoder escapes — every character that could close a script or
    /// a string, and everything outside printable ASCII — so the object is also a safe JavaScript
    /// literal ([`script`](Self::script)), and the same bytes Windows writes.
    ///
    /// One deliberate difference: a text size that is not a finite number (no caller makes one) is
    /// written `null`, which `fc-rich.js` reads as its default 16; `Utf8JsonWriter` throws there.
    pub fn json(&self) -> String {
        let mut out = String::with_capacity(self.source.len() + 128);
        out.push_str("{\"kind\":");
        push_json_string(&mut out, &self.kind);
        out.push_str(",\"source\":");
        push_json_string(&mut out, &self.source);
        if self.kind == "graphviz" {
            out.push_str(",\"engine\":");
            push_json_string(
                &mut out,
                if self.engine.is_empty() {
                    "dot"
                } else {
                    &self.engine
                },
            );
        }
        out.push_str(",\"dark\":");
        out.push_str(if self.dark { "true" } else { "false" });
        out.push_str(",\"color\":");
        push_json_string(&mut out, &self.color);
        out.push_str(",\"background\":");
        push_json_string(&mut out, &self.background);
        out.push_str(",\"fontSize\":");
        if self.font_size.is_finite() {
            out.push_str(&round_trip_text(self.font_size));
        } else {
            out.push_str("null");
        }
        out.push('}');
        out
    }

    /// The script that starts the job in the page: `fcRich.drawAndPost(id, job)`, whose answer comes
    /// back as a script message ([`RichAnswer::parse`]) — on Linux through
    /// `window.webkit.messageHandlers.fcRich`, which `fc-rich.js` already posts to. The job is JSON,
    /// which is a JavaScript expression, so nothing in a source is ever read as code.
    pub fn script(&self, id: i32) -> String {
        format!("fcRich.drawAndPost({id},{})", self.json())
    }

    /// A colour as CSS: `#RRGGBB`, or `#RRGGBBAA` when it is not opaque — a theme's own text
    /// colours may not be (WinUI's light theme writes black at 0xE4), and the formula must be the
    /// same ink as the words beside it.
    pub fn css_color(alpha: u8, red: u8, green: u8, blue: u8) -> String {
        if alpha == 0xFF {
            format!("#{red:02X}{green:02X}{blue:02X}")
        } else {
            format!("#{red:02X}{green:02X}{blue:02X}{alpha:02X}")
        }
    }
}

// ---- the page's answers ------------------------------------------------------------------------

/// What `fc-rich.js` answered for one job: the rectangle it drew at the page's top left, in CSS
/// pixels, and for a formula the distance from its top to its baseline — or that it could not draw
/// it.
#[derive(Debug, Clone, PartialEq)]
pub struct RichResult {
    pub ok: bool,
    pub width: f64,
    pub height: f64,
    pub baseline: f64,
    /// The engine's own words. NEVER LOGGED: KaTeX quotes the source it choked on, and no log
    /// carries a message's text.
    pub error: Option<String>,
}

impl RichResult {
    /// A drawing: `new RichResult(true, width, height, baseline)`.
    pub fn drawn(width: f64, height: f64, baseline: f64) -> Self {
        RichResult {
            ok: true,
            width,
            height,
            baseline,
            error: None,
        }
    }

    /// `RichResult.Failed(error)`.
    pub fn failed(error: &str) -> Self {
        RichResult {
            ok: false,
            width: 0.0,
            height: 0.0,
            baseline: 0.0,
            error: Some(error.to_string()),
        }
    }
}

/// The answers the page posts back, and the metrics a drawing is kept with on disk.
pub struct RichAnswer;

impl RichAnswer {
    /// The page's `{id, result}` message (`fcRich.drawAndPost`), or `None` for anything else. A
    /// result that says it drew but names no positive, finite size is a failure — there is nothing
    /// to photograph — and a baseline outside the drawing is put back inside it: a formula that sank
    /// below its own box would sit off the line.
    ///
    /// The id is a JSON integer that fits 32 bits (`3.0` and `3e0` are not one, as
    /// `JsonElement.TryGetInt32` says). One measured difference from .NET: a number beyond a
    /// double's range (`1e400`) makes the whole answer unreadable here — `serde_json` refuses it —
    /// where .NET reads it as not finite; the job then fails on its fifteen-second timeout instead.
    pub fn parse(json: &str) -> Option<(i32, RichResult)> {
        let root: Value = serde_json::from_str(json).ok()?;
        let root = root.as_object()?;
        let id = match root.get("id")? {
            Value::Number(number) => i32::try_from(number.as_i64()?).ok()?,
            _ => return None,
        };
        let result = root.get("result")?.as_object()?;
        Some((id, Self::result(result, false)))
    }

    /// A drawing's metrics as they are kept beside its PNG:
    /// `{"width":…,"height":…,"baseline":…}`, CSS pixels, each as .NET's round-trip text.
    pub fn metrics(result: &RichResult) -> String {
        format!(
            "{{\"width\":{},\"height\":{},\"baseline\":{}}}",
            round_trip_text(result.width),
            round_trip_text(result.height),
            round_trip_text(result.baseline)
        )
    }

    /// Those metrics read back, or `None` when the file is not what [`metrics`](Self::metrics)
    /// wrote.
    pub fn read_metrics(json: &str) -> Option<RichResult> {
        let root: Value = serde_json::from_str(json).ok()?;
        let read = Self::result(root.as_object()?, true);
        read.ok.then_some(read)
    }

    fn result(result: &Map<String, Value>, assume_ok: bool) -> RichResult {
        let ok = assume_ok || result.get("ok") == Some(&Value::Bool(true));
        if !ok {
            return RichResult::failed(match result.get("error") {
                Some(Value::String(error)) => error,
                _ => "error",
            });
        }
        let width = Self::number(result, "width");
        let height = Self::number(result, "height");
        if !(width > 0.0 && height > 0.0) {
            return RichResult::failed("empty");
        }
        let baseline = Self::number(result, "baseline");
        let baseline = if baseline.is_nan() {
            height
        } else {
            // `height` is positive and finite here, so the clamp is well formed.
            baseline.clamp(0.0, height)
        };
        RichResult::drawn(width, height, baseline)
    }

    /// A finite number, or NaN for anything else.
    fn number(owner: &Map<String, Value>, name: &str) -> f64 {
        match owner.get(name) {
            Some(Value::Number(number)) => number
                .as_f64()
                .filter(|value| value.is_finite())
                .unwrap_or(f64::NAN),
            _ => f64::NAN,
        }
    }
}

// ---- the page ------------------------------------------------------------------------------------

/// The page the hidden web view draws in, and the one place it may reach (docs/protocol.md, "How
/// it is drawn"; shell-design.md §9.1).
pub struct RichPage;

impl RichPage {
    /// The custom URI scheme the app's copy of `web/rich` is served from. It is registered with
    /// WebKit (secure and CORS-enabled, never `local`) and answered by the app itself from
    /// `/usr/share/family-connect/rich/`, so no request to it ever reaches a network — what the
    /// never-resolving `rich.familyconnect.invalid` host was for on Windows.
    pub const SCHEME: &'static str = "fc-rich";

    /// The one host of that scheme.
    pub const HOST: &'static str = "rich";

    /// The page itself, and the only address the web view is ever allowed to show.
    pub const URL: &'static str = "fc-rich://rich/rich.html";

    /// The layout viewport the page is pinned to, CSS pixels: wider than `fc-rich.js`'s 1200-pixel
    /// diagram cap, so nothing is wrapped by the window.
    pub const VIEWPORT_WIDTH: i32 = 1280;

    pub const VIEWPORT_HEIGHT: i32 = 800;

    /// Fifteen seconds without an answer is a failure (the design doc).
    pub const TIMEOUT: Duration = Duration::from_secs(15);

    /// Whether a navigation is to the page — exactly it; a fragment or a query is another
    /// document's business. ASCII letters match either case, as .NET's `OrdinalIgnoreCase` matches
    /// them against this all-ASCII address (measured: it maps no other letter onto one of these).
    pub fn is_page(uri: Option<&str>) -> bool {
        uri.is_some_and(|uri| uri.eq_ignore_ascii_case(Self::URL))
    }

    /// Whether the page may load `uri`: a file of its own folder — `fc-rich://rich/…`, the host in
    /// either case — or a `data:` URL, which the page's content policy already allows for images and
    /// which never leaves the process. Everything else is refused — no network, ever.
    ///
    /// Stricter than Windows' `Uri.TryCreate` reading in three ways that cannot matter to a URI
    /// WebKit hands over (measured on .NET 10, where all three were allowed): user information
    /// before the host (`fc-rich://x@rich/`), white space around the address, and any port at all —
    /// the scheme has no default one to name.
    pub fn allows(uri: Option<&str>) -> bool {
        let Some((scheme, rest)) = uri.and_then(|uri| uri.split_once(':')) else {
            return false;
        };
        if !is_scheme(scheme) {
            return false;
        }
        if scheme.eq_ignore_ascii_case("data") {
            return true;
        }
        if !scheme.eq_ignore_ascii_case(Self::SCHEME) {
            return false;
        }
        let Some(after) = rest.strip_prefix("//") else {
            return false;
        };
        let authority = after.split(['/', '?', '#']).next().unwrap_or(after);
        authority.eq_ignore_ascii_case(Self::HOST)
    }

    /// `fcRich.version` read out of `fc-rich.js` itself, so a drawing kept on disk can be found again
    /// before the web view — a browser process — has been started at all. `None` when the file does
    /// not say, and nothing is then drawn.
    ///
    /// The C#'s pattern `\bvar\s+VERSION\s*=\s*'([^'\r\n]+)'`, matched by hand: the first place
    /// where `var` starts a word, white space follows it, then `VERSION`, an `=` with optional white
    /// space around it, and a quoted value of at least one character on one line. White space is
    /// Unicode's, as .NET's `\s` is; a word character is a letter, a digit, `_` or a combining
    /// accent (.NET's `\w` would also leave out superscript-like digits, which `char::is_alphanumeric`
    /// counts — no script puts one before `var`).
    pub fn version_of(script: &str) -> Option<&str> {
        let mut from = 0;
        while let Some(found) = script[from..].find("var") {
            let at = from + found;
            // `var` is ASCII: one past its start is a character boundary.
            from = at + 1;
            if script[..at].chars().next_back().is_some_and(is_word) {
                continue;
            }
            let rest = &script[at + 3..];
            let spaced = rest.trim_start_matches(char::is_whitespace);
            if spaced.len() == rest.len() {
                continue;
            }
            let Some(rest) = spaced.strip_prefix("VERSION") else {
                continue;
            };
            let Some(rest) = rest
                .trim_start_matches(char::is_whitespace)
                .strip_prefix('=')
            else {
                continue;
            };
            let Some(value) = rest
                .trim_start_matches(char::is_whitespace)
                .strip_prefix('\'')
            else {
                continue;
            };
            let end = value.find(['\'', '\r', '\n']).unwrap_or(value.len());
            if end > 0 && value[end..].starts_with('\'') {
                return Some(&value[..end]);
            }
        }
        None
    }
}

/// An RFC 3986 scheme: a letter, then letters, digits, `+`, `-` or `.`.
fn is_scheme(scheme: &str) -> bool {
    let mut characters = scheme.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// A regex word character, near enough to .NET's `\w` (`\p{L}\p{Mn}\p{Nd}\p{Pc}`).
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || ('\u{300}'..='\u{36F}').contains(&c)
}

// ---- the photograph ------------------------------------------------------------------------------

/// Turning what the page drew into an image of it. The scale rule is the shell's on Linux too
/// (WebKit's `snapshot` is cropped to the answer's rectangle, §9.2); the DevTools arguments and the
/// base64 answer are Windows' capture (md.win's `CaptureRegionPngAsync`), kept unchanged as
/// Appendix B asks.
pub struct RichCapture;

impl RichCapture {
    /// The longest side, in pixels, a drawing is photographed at (the design doc): past it the scale
    /// gives way.
    pub const LONGEST_SIDE: i32 = 4096;

    /// `Emulation.setDefaultBackgroundColorOverride` to clear: without it Chromium photographs a page
    /// with no background on white, and a formula would carry a white box onto a blue bubble (what
    /// Puppeteer's `omitBackground` does).
    pub const CLEAR_BACKGROUND: &'static str = "{\"color\":{\"r\":0,\"g\":0,\"b\":0,\"a\":0}}";

    /// The scale to photograph a `width` × `height` CSS-pixel drawing at: the screen's own, so it is
    /// crisp at the size it is shown, unless that would put the longest side past
    /// [`LONGEST_SIDE`](Self::LONGEST_SIDE) pixels. A screen that cannot be asked counts as 1.
    pub fn scale_for(screen: f64, width: f64, height: f64) -> f64 {
        Self::scale_for_within(screen, width, height, Self::LONGEST_SIDE)
    }

    /// [`scale_for`](Self::scale_for) with another longest side.
    pub fn scale_for_within(screen: f64, width: f64, height: f64, longest: i32) -> f64 {
        let scale = if screen.is_finite() && screen > 0.0 {
            screen
        } else {
            1.0
        };
        let side = net_max(width, height);
        let longest = f64::from(longest);
        if side * scale > longest && side > 0.0 {
            longest / side
        } else {
            scale
        }
    }

    /// `Page.captureScreenshot`'s arguments: the drawing's rectangle at the page's top left, as PNG
    /// (which keeps the alpha), at `scale` — which re-renders rather than upscales — and
    /// `captureBeyondViewport`, so a drawing taller than the viewport is photographed whole.
    pub fn arguments(width: f64, height: f64, scale: f64) -> String {
        format!(
            "{{\"format\":\"png\",\"captureBeyondViewport\":true,\"clip\":{{\"x\":0,\"y\":0,\"width\":{},\"height\":{},\"scale\":{}}}}}",
            round_trip_text(net_max(width, 1.0)),
            round_trip_text(net_max(height, 1.0)),
            round_trip_text(scale)
        )
    }

    /// `Emulation.setDeviceMetricsOverride`: the viewport pinned in CSS pixels at a device scale of
    /// 1, so the window's own scale never reflows a drawing — the photograph's sharpness comes from
    /// the clip's scale instead.
    pub fn metrics() -> String {
        format!(
            "{{\"width\":{},\"height\":{},\"deviceScaleFactor\":1,\"mobile\":false}}",
            RichPage::VIEWPORT_WIDTH,
            RichPage::VIEWPORT_HEIGHT
        )
    }

    /// The PNG in `Page.captureScreenshot`'s answer, or `None` when it holds none.
    pub fn png(answer: &str) -> Option<Vec<u8>> {
        let root: Value = serde_json::from_str(answer).ok()?;
        match root.as_object()?.get("data")? {
            Value::String(base64) if !base64.is_empty() => base64_decode(base64),
            _ => None,
        }
    }
}

/// How a drawing sits in a line of text.
pub struct RichFit;

impl RichFit {
    /// An inline formula at its natural size — or, when it is wider than `max_width`, scaled down
    /// whole, baseline with it — and how far to drop it so its baseline sits on the line's: a picture
    /// in a run of text stands ON the baseline, and the part of a formula below its own baseline (a
    /// fraction's denominator, a subscript) has to hang under it. Answers (width, height, drop).
    pub fn inline(width: f64, height: f64, baseline: f64, max_width: f64) -> (f64, f64, f64) {
        let scale = if width > max_width && max_width > 0.0 {
            max_width / width
        } else {
            1.0
        };
        let shown_height = height * scale;
        (
            width * scale,
            shown_height,
            net_max(0.0, shown_height - net_clamp(baseline, 0.0, height) * scale),
        )
    }
}

// ---- the caches ------------------------------------------------------------------------------------

/// The drawings a window keeps in memory, most recently used first — and the failures, which are
/// kept only here, so a newer engine gets another try at them (the design doc's cache). Bounded
/// twice: by how many it holds and by their total weight — a drawing's weight is its pixels, and one
/// tall diagram photographed at 4096 pixels is worth a hundred formulas. Not thread-safe: the main
/// context owns it.
///
/// The C#'s `long` weights and budget are `u64` here, so the C#'s clamp of a negative weight to 0
/// and its refusal of a negative budget are in the type.
#[derive(Debug, Clone)]
pub struct RichMemory<T> {
    capacity: usize,
    budget: u64,
    /// Each value, its weight, and when it was last used.
    entries: HashMap<String, (T, u64, u64)>,
    clock: u64,
    weight: u64,
}

impl<T> RichMemory<T> {
    /// At most `capacity` entries, of any weight.
    ///
    /// # Panics
    ///
    /// When `capacity` is 0 — the C#'s `ArgumentOutOfRangeException`: a cache that can keep nothing
    /// is a mistake in the caller.
    pub fn new(capacity: usize) -> Self {
        Self::with_budget(capacity, u64::MAX)
    }

    /// At most `capacity` entries and `budget` of total weight; the newest entry is always kept,
    /// whatever it weighs.
    ///
    /// # Panics
    ///
    /// When `capacity` is 0, as [`new`](Self::new).
    pub fn with_budget(capacity: usize, budget: u64) -> Self {
        assert!(capacity >= 1, "a rich memory holds at least one drawing");
        RichMemory {
            capacity,
            budget,
            entries: HashMap::new(),
            clock: 0,
            weight: 0,
        }
    }

    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// The weight of everything held.
    pub fn weight(&self) -> u64 {
        self.weight
    }

    /// The value kept under `key`, which now counts as the most recently used.
    pub fn try_get(&mut self, key: &str) -> Option<&T> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(key).map(|entry| {
            entry.2 = clock;
            &entry.0
        })
    }

    /// Keep `value` under `key` with a weight of 1.
    pub fn put(&mut self, key: &str, value: T) {
        self.put_weighing(key, value, 1);
    }

    /// Keep `value` under `key`, letting the least recently used go once past either bound.
    pub fn put_weighing(&mut self, key: &str, value: T, weight: u64) {
        if let Some((_, old, _)) = self.entries.remove(key) {
            self.weight -= old;
        }
        self.clock += 1;
        self.entries
            .insert(key.to_string(), (value, weight, self.clock));
        self.weight = self.weight.saturating_add(weight);
        while (self.entries.len() > self.capacity || self.weight > self.budget)
            && self.entries.len() > 1
        {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, _, used))| *used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some((_, gone, _)) = self.entries.remove(&oldest) {
                self.weight -= gone;
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.weight = 0;
    }
}

/// The folder the drawings are kept in — the seam [`RichStore`] reaches the disk through, which the
/// shell implements with `std::fs` over `$XDG_CACHE_HOME/family-connect/rich/` (shell-design.md
/// §9.4). Every name is a plain file name inside that one folder — never a path — so nothing here can
/// name a file anywhere else.
///
/// What the C# did with `File`/`Directory` and how a `std::fs` implementation answers:
///
/// | Method | C# | `std::fs` |
/// | --- | --- | --- |
/// | `exists` | `Directory.Exists` | `metadata(dir).is_dir()` |
/// | `create` | `Directory.CreateDirectory` | `create_dir_all` |
/// | `files` | `Directory.EnumerateFiles` (names) | `read_dir`, regular files, any order |
/// | `is_file` | `File.Exists` (false on any error) | `metadata(path).is_file()` |
/// | `read` / `write` | `File.ReadAllBytes` / `File.WriteAllBytes` | `read` / `write` |
/// | `replace` | `File.Move(from, to, overwrite: true)` | `rename` |
/// | `delete` | `File.Delete` — a missing file is no error | `remove_file`, `NotFound` → `Ok` |
/// | `modified` | `FileInfo.LastWriteTimeUtc` | `metadata.modified()` |
/// | `touch` | `File.SetLastWriteTimeUtc(path, UtcNow)` | `File::options().write(true).open` + `set_modified(SystemTime::now())` |
pub trait RichFolder {
    /// Whether the folder itself exists.
    fn exists(&self) -> bool;
    /// Make the folder, and its parents; no error when it exists.
    fn create(&self) -> io::Result<()>;
    /// The names of the files in the folder, in any order.
    fn files(&self) -> io::Result<Vec<String>>;
    /// Whether `name` is a file in the folder; false on any error.
    fn is_file(&self, name: &str) -> bool;
    fn read(&self, name: &str) -> io::Result<Vec<u8>>;
    /// Write `name` whole, replacing what was there.
    fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()>;
    /// Move `from` onto `to`, replacing `to` when it exists.
    fn replace(&self, from: &str, to: &str) -> io::Result<()>;
    /// Delete `name`; a file that is not there is no error.
    fn delete(&self, name: &str) -> io::Result<()>;
    /// When `name` was last written.
    fn modified(&self, name: &str) -> io::Result<SystemTime>;
    /// Mark `name` as written now.
    fn touch(&self, name: &str) -> io::Result<()>;
}

/// The drawings kept on disk, in the app's own cache folder: `<key>.png` and its metrics beside it
/// in `<key>.json`, written last, so a drawing is there only once both are. A file that cannot be
/// read is a drawing that is not kept — it is drawn again — and every failure here is swallowed: a
/// cache never stops a bubble.
pub struct RichStore<F: RichFolder> {
    folder: F,
}

impl<F: RichFolder> RichStore<F> {
    /// How many drawings stay on disk; the ones read least recently go first
    /// ([`trim`](Self::trim)).
    pub const KEEP: usize = 1000;

    pub fn new(folder: F) -> Self {
        RichStore { folder }
    }

    /// The folder it keeps its drawings in.
    pub fn folder(&self) -> &F {
        &self.folder
    }

    /// The drawing kept under `key`, or `None`. Reading one marks it as used, which is what keeps
    /// it. (As in the C#, a drawing whose mark cannot be written is answered `None` too.)
    pub fn read(&self, key: &str) -> Option<(Vec<u8>, RichResult)> {
        if !is_key(key) {
            return None;
        }
        let metrics_name = format!("{key}.json");
        let png_name = format!("{key}.png");
        if !self.folder.is_file(&metrics_name) || !self.folder.is_file(&png_name) {
            return None;
        }
        let text = text_of(&self.folder.read(&metrics_name).ok()?);
        let Some(metrics) = RichAnswer::read_metrics(&text) else {
            self.forget(key);
            return None;
        };
        let png = self.folder.read(&png_name).ok()?;
        if png.is_empty() {
            self.forget(key);
            return None;
        }
        self.folder.touch(&metrics_name).ok()?;
        Some((png, metrics))
    }

    /// Keep a drawing. Each file goes in under a temporary name and is moved into place, so a reader
    /// never sees half of one. A failure, an empty picture or a key that is not one is never kept.
    pub fn write(&self, key: &str, png: &[u8], metrics: &RichResult) {
        if !is_key(key) || !metrics.ok || png.is_empty() {
            return;
        }
        let written = self
            .folder
            .create()
            .and_then(|()| self.place(&format!("{key}.png"), png))
            .and_then(|()| {
                self.place(
                    &format!("{key}.json"),
                    RichAnswer::metrics(metrics).as_bytes(),
                )
            });
        if written.is_err() {
            self.forget(key);
        }
    }

    /// Let all but the [`KEEP`](Self::KEEP) most recently used drawings go, and anything a crash left
    /// half-written.
    pub fn trim(&self) {
        self.trim_to(Self::KEEP);
    }

    /// Let all but the `keep` most recently used drawings go, and anything a crash left
    /// half-written. Stops at the first failure — trimmed next launch, then.
    pub fn trim_to(&self, keep: usize) {
        let _ = self.trimming(keep);
    }

    fn trimming(&self, keep: usize) -> io::Result<()> {
        if !self.folder.exists() {
            return Ok(());
        }
        for stray in self.folder.files()? {
            if stray.ends_with(".part") {
                self.folder.delete(&stray)?;
            }
        }
        let mut kept: Vec<(String, SystemTime)> = self
            .folder
            .files()?
            .into_iter()
            .filter(|name| name.ends_with(".json"))
            .map(|name| {
                // `FileInfo.LastWriteTimeUtc` of a file gone meanwhile is its oldest date, not an
                // error.
                let modified = self
                    .folder
                    .modified(&name)
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                (name, modified)
            })
            .collect();
        // Newest first; a stable sort, as LINQ's `OrderByDescending` is.
        kept.sort_by_key(|one| std::cmp::Reverse(one.1));
        for (name, _) in kept.iter().skip(keep) {
            self.forget(name.strip_suffix(".json").unwrap_or(name));
        }
        // A PNG whose metrics never landed is not a drawing.
        for png in self.folder.files()? {
            if let Some(stem) = png.strip_suffix(".png")
                && !self.folder.is_file(&format!("{stem}.json"))
            {
                self.folder.delete(&png)?;
            }
        }
        Ok(())
    }

    fn place(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let part = format!("{name}.part");
        self.folder.write(&part, bytes)?;
        self.folder.replace(&part, name)
    }

    /// Both files of a drawing gone; a failure is left for [`trim`](Self::trim).
    fn forget(&self, key: &str) {
        let _ = self
            .folder
            .delete(&format!("{key}.json"))
            .and_then(|()| self.folder.delete(&format!("{key}.png")));
    }
}

/// A key is 64 lower-case hex digits ([`RichJob::key`]) and nothing else can name a file here.
fn is_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A file's text, as `File.ReadAllText` reads UTF-8: a byte-order mark dropped, a byte that is not
/// UTF-8 read as U+FFFD.
fn text_of(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

// ---- what .NET did for the C# ------------------------------------------------------------------------

/// A double as .NET writes it with `ToString("R", CultureInfo.InvariantCulture)` — and as
/// `Utf8JsonWriter.WriteNumber` writes a finite one (measured identical on .NET 10).
///
/// The digits are the shortest that read back as the same double (Rust's `{:e}` and .NET Core 3.0's
/// formatter both give exactly those); they are laid out in fixed notation while the decimal
/// exponent is from −4 to 16 (`0.0001`, `99999999999999980`) and in .NET's scientific notation
/// otherwise — `1E-05`, `1E+17`, `1.2345678901234568E+17`: a capital `E`, a sign, at least two
/// digits. Nought is `0` or `-0`; the rest are `NaN`, `Infinity` and `-Infinity`. Rust's `{}` never
/// writes an exponent, which is why the C#'s formats are re-made here (port-plan "Numbers and time").
pub fn round_trip_text(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value == 0.0 {
        return format!("{sign}0");
    }
    let shortest = format!("{:e}", value.abs());
    let (mantissa, exponent) = shortest.split_once('e').unwrap_or((&shortest, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    if (-4..=16).contains(&exponent) {
        // How many digits stand before the point.
        let point = exponent + 1;
        let count = digits.len() as i32;
        if point <= 0 {
            format!("{sign}0.{}{digits}", "0".repeat((-point) as usize))
        } else if point >= count {
            format!("{sign}{digits}{}", "0".repeat((point - count) as usize))
        } else {
            let (whole, fraction) = digits.split_at(point as usize);
            format!("{sign}{whole}.{fraction}")
        }
    } else {
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { "" } else { "." };
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        format!(
            "{sign}{first}{point}{rest}E{exponent_sign}{:02}",
            exponent.unsigned_abs()
        )
    }
}

/// `text` as a JSON string, escaped as `Utf8JsonWriter`'s default encoder escapes (measured on .NET
/// 10): `\b \t \n \f \r` and `\\` in their short forms; the other control characters, `"`, `&`, `'`,
/// `+`, `<`, `>`, `` ` ``, DEL and every character outside ASCII as `\uXXXX`, capital hex, a
/// character past U+FFFF as its two UTF-16 halves. What is left is printable ASCII no HTML or script
/// parser treats specially — so U+2028, which ends a line in older JavaScript, never appears raw.
fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            c if !(' '..'\u{7F}').contains(&c)
                || matches!(c, '"' | '&' | '\'' | '+' | '<' | '>' | '`') =>
            {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04X}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `Convert.FromBase64String`, as measured on .NET 10: the standard alphabet, white space (space,
/// tab, CR, LF — not other spaces) ignored anywhere, a length that is a multiple of four once it is
/// gone, at most two `=` and only at the end; bits left over in the last character are ignored
/// (`AR==` is `01`, as .NET reads it). `None` where .NET throws `FormatException`.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let symbols: Vec<u8> = text
        .bytes()
        .filter(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .collect();
    if !symbols.len().is_multiple_of(4) {
        return None;
    }
    let padding = symbols.iter().rev().take_while(|&&b| b == b'=').count();
    if padding > 2 {
        return None;
    }
    let body = &symbols[..symbols.len() - padding];
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &symbol in body {
        let value = match symbol {
            b'A'..=b'Z' => symbol - b'A',
            b'a'..=b'z' => symbol - b'a' + 26,
            b'0'..=b'9' => symbol - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// SHA-256 (FIPS 180-4) of `bytes`, in lower-case hex — `Convert.ToHexStringLower(SHA256.HashData(…))`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    sha256(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// SHA-256 (FIPS 180-4) of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    // The message, a 1 bit, zeros to 56 mod 64, and its length in bits as a big-endian u64.
    let mut message = bytes.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((bytes.len() as u64).wrapping_mul(8)).to_be_bytes());
    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (index, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[index] = u32::from_be_bytes(*word);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (held, next) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *held = held.wrapping_add(next);
        }
    }
    let mut digest = [0u8; 32];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// `Math.Max` on doubles: NaN if either is NaN (Rust's `f64::max` would answer the other).
fn net_max(one: f64, other: f64) -> f64 {
    if one.is_nan() || other.is_nan() {
        f64::NAN
    } else {
        one.max(other)
    }
}

/// `Math.Clamp` on doubles without its throw: a NaN value stays NaN, and bounds the wrong way round
/// (which no caller passes; .NET throws) answer whichever bound the value passed first.
fn net_clamp(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}
