//! Tappable data in message bodies — web links, email addresses and phone
//! numbers — found in the text AS DRAWN: the web client's port of
//! ios/FamilyConnect/Models/MessageLinks.swift.
//!
//! The same message has to read the same on an iPhone, a Mac, an Android
//! phone and in a browser, and Apple is the reference: its tests are this
//! file's tests, name for name. But Apple has no grammar to port. It asks
//! NSDataDetector — the platform's own detector — and puts a few rules of
//! its own on top. So this file holds two different kinds of rule:
//!
//! - **The rules Swift wrote itself are mirrored exactly**, quirks and all:
//!   how a phone match becomes a `tel:` URL (`tel_url`, `dialable_digits`),
//!   how a markdown destination gains a scheme ([`normalize_destination`]),
//!   that the author's markdown destination beats anything detected over its
//!   label ([`merge`]), and that the preview card describes the first
//!   `https` link and nothing else ([`first_web_link`]).
//! - **NSDataDetector is approximated, from measurement.** It is documented
//!   nowhere worth the name, so every grammar rule below was read off the
//!   detector itself: a Swift script on macOS 26 running MessageLinks' own
//!   code around NSDataDetector, fed every code point in every position and
//!   a few thousand hand-written messages. Where the two still disagree the
//!   difference is a known one, listed under "Where this is not Apple".
//!
//! Detection runs over ONE laid-out text run — markdown already rendered,
//! one block of a table-split body — for the reason Apple's `build` gives:
//! markdown deletes characters, so offsets taken from the raw body land on
//! the wrong glyphs. Ranges are byte offsets into that run and always fall
//! on char boundaries, because this project has shipped a panic from a
//! slice that did not ("Привет" in a server request path).
//!
//! # Where this is not Apple
//!
//! On purpose:
//!
//! - **Only `http`, `https`, `mailto` and `tel` are ever links.**
//!   NSDataDetector links ANY `scheme://` — `ftp://`, `file://`, a custom
//!   app scheme, and `javascript://…%0Aalert(1)` — plus `sms:`, `news:`,
//!   `sip:` and a few other opaque schemes. On iOS an odd scheme simply
//!   fails to open; in a browser `javascript:` runs in this client's own
//!   origin, next to the session token. A `scheme://` URL that is not web
//!   is swallowed whole — no link, and no link over the host inside it
//!   either, which would open a different protocol than the one written.
//! - **A phone number never grows a wrong number.** Apple reads
//!   `555-123-4567,89` as one number and Swift's `dialableDigits` drops the
//!   comma, dialling 555123456789; here the number ends at the comma. Apple
//!   reads `*67 555-123-4567` as `67 555-123-4567` and dials 675551234567,
//!   reads `555-1234 555-4321` as one 14-digit number, and drops the last
//!   group of a number followed by `»` or a flag (`«+44 20 7946 0958»`
//!   dials +44207946); here the first is no link, the second two numbers,
//!   the last the whole number. A fullwidth extension (`x８９`) dials 89
//!   here, and percent-escaped fullwidth digits on Apple.
//! - **A percent escape is never escaped again.** Apple re-escapes every
//!   `%` in a URL once anything non-ASCII follows it (an emoji, `»`, a
//!   bidi mark), so `…/%D0%9C…😀` opens `…/%25D0%259C…`, which does not
//!   exist.
//! - **An internationalised host stays in Unicode.** Apple's target spells
//!   `https://пример.рф` as `https://xn--e1afmkfd.xn--p1ai`. Half an IDNA
//!   implementation is worse than none — punycode over a label UTS 46 would
//!   have mapped first opens a different, registrable host — and a browser
//!   applies the whole of UTS 46 to an `href` itself, so the tap lands on
//!   the same host by the only implementation that is actually correct. The
//!   path and query ARE percent-encoded exactly as Apple does, so for an
//!   ASCII host the target is byte-identical.
//!
//! Because the grammar is measured, not specified (all found by the oracle,
//! none covered by a Swift test):
//!
//! - **Phone numbers are a heuristic on both sides**, and this one is
//!   smaller. It agrees on the shapes people type — international `+CC`
//!   groups, national formats with brackets and trunk prefixes, 7-digit
//!   locals, extensions, vanity 1-800 numbers — and on the non-numbers that
//!   look like them (dates, times, decimals, money, ZIP+4, ISBNs, years).
//!   It does not have Apple's context words (`order 123-456-789` is not a
//!   phone there, `call 911` is, `phone 911` is not) beyond `call`/`dial`
//!   before a short number, and it rejects some long runs Apple accepts
//!   (`4111 1111 1111 1111`) and accepts some it rejects (an IBAN's digit
//!   tail).
//! - **Which letters join a host name, a scheme or an email address** is
//!   Apple's per-script table; this file uses Unicode's own properties (a
//!   letter or decimal digit, cased or not) plus Apple's measured lists of
//!   the scripts written without spaces and of the scripts an email local
//!   part may use. They agree on the letters of Latin, Greek, Cyrillic,
//!   Armenian, Georgian, Hebrew, Arabic, the Indic scripts, Thai, Han, kana
//!   and Hangul, and differ on rarer scripts (Ethiopic, Yi, Tifinagh, …), on
//!   letter-like symbols (circled letters, modifier letters) and on a
//!   combining mark right before a link — a Devanagari or Thai vowel sign
//!   glued to `https://` is a break to Apple and part of the word here.
//! - **Apple reads words, and this file does not.** A URL glued to Japanese
//!   is cut before the particle that follows it (`…/aです` links `…/a`,
//!   `…/a日本語` all of it), a Thai word glued to a bare host sometimes
//!   voids it and sometimes not, and a URL in German quotes that ends
//!   `/“` or `)“` loses the slash or the brackets. None of it is ported.
//! - **Where a URL ends in the middle of non-ASCII text** follows Apple's
//!   own table exactly (`URL_STOP`, `URL_POISON`, `HOST_BAD` — measured
//!   over every assigned code point), including Apple's quirks: an emoji,
//!   `»` or `’` glued to a host becomes part of it — a dead link on both
//!   sides — and a digit from another script in a path cuts the link back
//!   to its host. Code points unassigned on macOS 26 may differ.

use std::cmp::Ordering;
use std::ops::Range;

/// One tappable range in a laid-out text run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    /// Byte range into the run it was found in. Always on char boundaries,
    /// so slicing the run with it cannot panic.
    pub range: Range<usize>,
    /// The characters the span covers, exactly as drawn.
    pub text: String,
    /// What a tap opens. A detected span is always `http://`, `https://`,
    /// `mailto:` or `tel:` (in the case the author typed the scheme in); a
    /// markdown span is whatever the author wrote, normalized — gate it
    /// with [`is_openable`] before it becomes an `href`.
    pub target: String,
}

/// Every web link, email address and phone number in `text`, in order and
/// never overlapping — what NSDataDetector finds, as far as this file can
/// tell (see the module docs for where it cannot).
pub fn detect(text: &str) -> Vec<LinkSpan> {
    let mut found = Vec::new();
    scheme_links(text, &mut found);
    tel_links(text, &mut found);
    email_links(text, &mut found);
    bare_links(text, &mut found);
    phone_links(text, &mut found);
    // Leftmost first, and the longest of those: an address wins over the
    // host inside it, a URL over the phone number in its path.
    found.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut spans = Vec::new();
    let mut taken = 0;
    for f in found {
        if f.start < taken {
            continue;
        }
        taken = f.end;
        if let Some(target) = f.target {
            spans.push(LinkSpan {
                range: f.start..f.end,
                text: text[f.start..f.end].to_owned(),
                target,
            });
        }
    }
    spans
}

/// The markdown's own links, plus every detected link that overlaps none
/// of them, in order.
///
/// The overlap rule is a SAFETY rule, not tidiness (Swift's `decorated`).
/// `[https://www.paypal.com](https://evil.example)` renders as that first
/// URL's text, which the detector then matches as a link to the place it
/// NAMES. Whichever of the two won, a tap could open somewhere the reader
/// had every reason to think was what they were looking at. The author's
/// own destination is what the message declares, so it stands and the
/// detector's duplicate is dropped. Android's `mergeSpans` is the same
/// rule.
pub fn merge(declared: Vec<LinkSpan>, detected: Vec<LinkSpan>) -> Vec<LinkSpan> {
    let mut out = declared;
    for span in detected {
        // Against everything linked so far, as Swift checks the runs of an
        // attributed string that already carries the earlier links.
        if out.iter().any(|kept| overlaps(&kept.range, &span.range)) {
            continue;
        }
        out.push(span);
    }
    out.sort_by_key(|span| span.range.start);
    out
}

/// A markdown link's destination, made openable: `https://` in front when
/// there is no scheme, and anything the author wrote in full left exactly
/// as written (Swift's `normalized`).
///
/// `https` rather than `http`, matching Android's `normalize`. "Has a
/// scheme" is Foundation's reading, not a sensible one, and it is kept so
/// that the same message opens the same thing: `localhost:8080` and
/// `example.com:8080/x` have a scheme (`localhost`, `example.com`) and stay
/// the dead links they are on Apple, where Android makes them `https://`.
/// The destination is taken as the markdown parser hands it over;
/// Foundation's percent-encoding of it is the parser's business, and a
/// browser applies the same encoding to an `href`.
pub fn normalize_destination(destination: &str) -> String {
    // `URL(string: ":x")?.scheme` is "" — present, merely empty.
    if destination.starts_with(':') || scheme_of(destination).is_some() {
        destination.to_owned()
    } else {
        format!("https://{destination}")
    }
}

/// The first https link among `spans` — what the preview card under the
/// balloon describes (Swift's `firstWebLinkAsDrawn`, given the merged
/// spans in document order).
///
/// Only https: phone numbers and addresses have nothing to preview,
/// previewing every link would bury the message itself, and plain http is
/// excluded because ATS blocks the fetch on Apple, so a card here would be
/// a card iOS never shows. (Android's `firstWebLinkUrl` takes http too.)
pub fn first_web_link(spans: &[LinkSpan]) -> Option<&LinkSpan> {
    spans
        .iter()
        .find(|span| scheme_of(&span.target).is_some_and(|s| s.eq_ignore_ascii_case("https")))
}

/// Whether `target` may become an `href`: http, https, mailto or tel.
///
/// Apple opens any URL a link carries, and on iOS a strange scheme is
/// merely a tap that does nothing. In a browser a `javascript:` URL runs in
/// this client's origin, so a markdown destination — which is whatever the
/// author typed — must pass this before it is drawn as a link. Detected
/// spans always pass.
pub fn is_openable(target: &str) -> bool {
    scheme_of(target).is_some_and(|scheme| {
        ["http", "https", "mailto", "tel"]
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
    })
}

/// A match before the overlaps are settled. `target: None` is a region that
/// is not a link and may not hold one — a `ftp://` URL, whose host would
/// otherwise come back as a web link to a different protocol.
struct Found {
    start: usize,
    end: usize,
    target: Option<String>,
}

/// Whether two ranges share a character. An empty range shares none — in
/// Swift it holds no characters, so it carries no link run to object with.
fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    !a.is_empty() && !b.is_empty() && a.start < b.end && b.start < a.end
}

/// RFC 3986's scheme — a letter, then letters, digits, `+`, `-` or `.` —
/// before the first colon. The same reading as Foundation's `URL.scheme`.
fn scheme_of(target: &str) -> Option<&str> {
    let scheme = &target[..target.find(':')?];
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some(scheme)
}

// MARK: - Characters

/// The char that starts at byte `at`, or None past the end or off a char
/// boundary — never a panic.
fn char_at(text: &str, at: usize) -> Option<char> {
    text.get(at..)?.chars().next()
}

/// The char that ends at byte `at`.
fn char_before(text: &str, at: usize) -> Option<char> {
    text.get(..at)?.chars().next_back()
}

/// A letter or digit of a script written with spaces between its words.
///
/// This is what glues: a scheme right after one is not a scheme
/// (`смотриhttps://` is not a link to https), a host name continues
/// through one. Unicode's letters, minus the scripts written without
/// spaces (`is_cjk`), which Apple treats as punctuation for this purpose;
/// and DECIMAL digits only — `²` and `①` are not word characters to
/// Apple, `٣` is. Rust has no general category, but the non-ASCII decimal
/// digits are exactly the numeric characters in Apple's poison table.
fn is_word(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric();
    }
    (c.is_alphabetic() && !is_cjk(c)) || (c.is_numeric() && is_poison(c))
}

/// What may not come right after a host name or an email address: more of
/// a word, a mark, or a character no host can hold. Measured — Apple's
/// list is the same for both — and approximated where Apple's reaches
/// into marks Unicode does not call alphabetic.
fn continues_name(c: char) -> bool {
    !c.is_ascii()
        && (is_word(c)
            || is_combining(c)
            || (is_poison(c) && !is_soft_poison(c))
            || in_table(c, HOST_BAD))
}

/// Han, kana, Hangul syllables and their iteration marks — the scripts
/// written without spaces, where a URL is routinely glued to the sentence
/// around it, so Apple lets them END a word rather than be part of one:
/// `日本語https://example.com` links and so does `日本語example.com`.
/// Measured: exactly the letters after which a glued `https://` still
/// counted.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x02B9..=0x02BA | 0x02C6..=0x02CF | 0x02EC | 0x0374 | 0x2E2F
        | 0x3005..=0x3007 | 0x3021..=0x3029 | 0x3031..=0x3035 | 0x3038..=0x303B
        | 0x3041..=0x3096 | 0x309D..=0x309F | 0x30A1..=0x30FA | 0x30FC..=0x30FF
        | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xA67F | 0xA717..=0xA71F | 0xA788 | 0xAC00..=0xD7A3
        | 0xF900..=0xFAD9 | 0xFF66..=0xFF9F
        | 0x16FE3 | 0x17000..=0x1B2FB | 0x20000..=0x323AF)
}

/// `is_cjk` without the Hangul syllables: Apple takes Hangul in an
/// email's local part, and Han and kana not.
fn is_han_or_kana(c: char) -> bool {
    is_cjk(c) && !(0xAC00..=0xD7A3).contains(&(c as u32))
}

/// The combining-diacritic blocks — the marks of decomposed Latin, Greek
/// and Cyrillic, which Unicode does not count as alphabetic (the marks of
/// Indic and other scripts mostly are, and need no help).
fn is_combining(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

/// An ASCII digit, or a fullwidth one — Apple reads `５５５-１２３-４５６７`
/// and hands it back in ASCII.
fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || ('\u{FF10}'..='\u{FF19}').contains(&c)
}

/// A fullwidth digit in ASCII; anything else as it is.
fn ascii_digit(c: char) -> char {
    match c {
        '\u{FF10}'..='\u{FF19}' => char::from(b'0' + (c as u32 - 0xFF10) as u8),
        c => c,
    }
}

fn in_table(c: char, table: &[(u32, u32)]) -> bool {
    let v = c as u32;
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < v {
                Ordering::Less
            } else if lo > v {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
        .is_ok()
}

/// A character that ends a URL: whitespace, controls and `"` in ASCII, and
/// Apple's own list beyond it (`URL_STOP`) — CJK and fullwidth
/// punctuation, the bullet, most maths, other scripts' full stops.
fn is_url_stop(c: char) -> bool {
    if c.is_ascii() {
        c <= ' ' || c == '"' || c == '\u{7F}'
    } else {
        in_table(c, URL_STOP)
    }
}

/// A character that a URL holds but cannot survive: in a path it cuts the
/// link back to its host, in a host it voids the link (`URL_POISON` —
/// digits of other scripts, format characters, letter numbers).
fn is_poison(c: char) -> bool {
    !c.is_ascii() && in_table(c, URL_POISON)
}

/// Poison that only ENDS a link when nothing but more of it follows: the
/// bidi embedding and isolate controls right-to-left text is full of, and a
/// few dots. `https://example.com/a\u{202A}` links `https://example.com/a`,
/// where any other poison there would cut it back to the host.
fn is_soft_poison(c: char) -> bool {
    matches!(c as u32,
        0x05F4 | 0x061C | 0x2024 | 0x2027 | 0x202A..=0x202E | 0x2066..=0x2069
        | 0xFE13 | 0xFE52 | 0xFF07)
}

/// Punctuation that ends a sentence rather than a URL, dropped from the
/// end of one. Measured, and narrower than it looks: `?`, `:`, `'` and `#`
/// at the end of a URL are kept. (A trailing zero-width non-joiner goes
/// too: Apple leaves it out of the target.)
fn is_trailing(c: char) -> bool {
    matches!(
        c,
        '!' | ',' | '.' | ';' | '\u{201C}' | '\u{201D}' | '\u{2026}' | '\u{30FB}' | '\u{200C}'
    )
}

/// A character a path may hold and a host may not; one in the host voids
/// the whole link.
fn is_host_bad(c: char) -> bool {
    if c.is_ascii() {
        matches!(c, '%' | '[' | '\\' | ']' | '^' | '`' | '|')
    } else {
        in_table(c, HOST_BAD) || is_poison(c)
    }
}

// MARK: - URLs with a scheme

/// A URL's extent and where its authority (userinfo, host, port) sits.
struct Url {
    end: usize,
    authority: Range<usize>,
}

/// Every `scheme://…`: a link when the scheme is http or https, a region
/// nothing else may claim when it is anything else.
fn scheme_links(text: &str, found: &mut Vec<Found>) {
    let bytes = text.as_bytes();
    for (colon, _) in text.match_indices("://") {
        // The scheme is the longest run before `://` that starts with a
        // letter; `.` is left out on purpose, because Apple reads
        // `x.https://` as https.
        let mut run = colon;
        while run > 0
            && matches!(bytes[run - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'+' | b'-')
        {
            run -= 1;
        }
        let Some(letter) = bytes[run..colon].iter().position(u8::is_ascii_alphabetic) else {
            continue;
        };
        let start = run + letter;
        // Glued to a word, it is not a scheme: `1https://`, `смотриhttps://`.
        if char_before(text, start).is_some_and(is_word) {
            continue;
        }
        let body = colon + 3;
        let raw = scan_body(text, body, Brackets::Balanced);
        let scheme = &text[start..colon];
        let web = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
        match scan_url(text, start, body, raw) {
            Some(url) => found.push(Found {
                start,
                end: url.end,
                target: web.then(|| url_target(text, start, &url)),
            }),
            // A URL Apple refuses is refused whole: `https://example.com:abc`
            // does not become a link to the `example.com` inside it.
            None if raw > body => found.push(Found {
                start,
                end: raw,
                target: None,
            }),
            None => {}
        }
    }
}

/// The URL whose body starts at `body` (just past `://`) and whose
/// characters run to `raw`, or None when that is not a URL.
fn scan_url(text: &str, start: usize, body: usize, raw: usize) -> Option<Url> {
    let mut end = cut_unclosed(text, body, raw);
    let authority_end = text[body..end]
        .find(['/', '?', '#'])
        .map_or(end, |at| body + at);
    if let Some(at) = first_poison(text, body, end) {
        let soft_to = |limit: usize| text[at..limit].chars().all(is_soft_poison);
        if at < authority_end {
            // `https://example.com\u{202A}/x` links `https://example.com`.
            if !soft_to(authority_end) {
                return None;
            }
            end = at;
        } else {
            end = if soft_to(end) { at } else { authority_end };
        }
    }
    end = trim_tail(text, start, body, end, Brackets::Balanced);
    let authority = body..authority_end.min(end);
    if end == body || !valid_authority(&text[authority.clone()]) {
        return None;
    }
    Some(Url { end, authority })
}

/// Where the first poison in `text[from..end]` sits. A zero-width joiner
/// holds an emoji sequence together anywhere in a URL, and is poison only
/// as its very last character.
fn first_poison(text: &str, from: usize, end: usize) -> Option<usize> {
    text[from..end]
        .char_indices()
        .find(|&(at, c)| is_poison(c) || (c == '\u{200D}' && from + at + c.len_utf8() == end))
        .map(|(at, _)| from + at)
}

/// Whether `(`, `{` and `<` pair with their closers inside a URL.
#[derive(Clone, Copy, PartialEq)]
enum Brackets {
    /// After a scheme, Apple keeps a closer once any opener has been seen.
    Balanced,
    /// In the path of a host with no scheme, every one of them ends the
    /// link — `en.wikipedia.org/wiki/Foo_(bar)` links `…/Foo_` on Apple.
    Stop,
}

/// How far the characters a URL can hold go from `from`.
///
/// After a scheme, a closing `)`, `}` or `>` ends the URL unless an opener
/// has been seen in it — any opener, which is Apple's rule, not a balance
/// count: `(see https://example.com/a)` stops before the `)`, and
/// `https://example.com/wiki/Foo_(bar)` keeps its own.
fn scan_body(text: &str, from: usize, brackets: Brackets) -> usize {
    let mut end = from;
    let mut opened = false;
    for (at, c) in text[from..].char_indices() {
        if is_url_stop(c) {
            break;
        }
        match c {
            '(' | '{' | '<' | ')' | '}' | '>' if brackets == Brackets::Stop => break,
            '(' | '{' | '<' => opened = true,
            ')' | '}' | '>' if !opened => break,
            _ => {}
        }
        end = from + at + c.len_utf8();
    }
    end
}

/// Cut the URL back to an opener that nothing closes after it:
/// `https://example.com/a(b` links `https://example.com/a`, while
/// `https://example.com/a((b)` keeps everything, because the LAST opener
/// has a closer after it. Apple's rule, measured; an opener before the
/// last closer never matters.
fn cut_unclosed(text: &str, from: usize, end: usize) -> usize {
    let body = &text[from..end];
    let after_close = body.rfind([')', '}', '>']).map_or(0, |at| at + 1);
    match body[after_close..].find(['(', '{', '<']) {
        Some(at) => from + after_close + at,
        None => end,
    }
}

/// Drop sentence punctuation from the end of a URL, and ONE closing `'` or
/// `]` when the URL starts right after its opener — `'https://example.com'`
/// and `[https://example.com]` link without the closer, while a URL that
/// merely ends in one keeps it.
fn trim_tail(text: &str, start: usize, floor: usize, mut end: usize, brackets: Brackets) -> usize {
    let opener = char_before(text, start);
    let mut paired = false;
    while end > floor {
        let Some(last) = char_before(text, end) else {
            break;
        };
        // Without a scheme a closing `]` goes as well: `example.com/a]`.
        if is_trailing(last) || (brackets == Brackets::Stop && last == ']') {
            end -= last.len_utf8();
        } else if !paired && matches!((opener, last), (Some('\''), '\'') | (Some('['), ']')) {
            paired = true;
            end -= 1;
        } else {
            break;
        }
    }
    end
}

/// `userinfo@host:port`, with the host holding nothing a host cannot and
/// the port nothing but digits (possibly none: Apple links
/// `https://example.com:`).
fn valid_authority(authority: &str) -> bool {
    let host_port = match authority.rfind('@') {
        Some(at) if at + 1 == authority.len() => return false,
        Some(at) => &authority[at + 1..],
        None => authority,
    };
    let (host, port) = match host_port.strip_prefix('[') {
        // An IP literal: whatever is inside the brackets, as Apple takes it.
        Some(literal) => match literal.find(']') {
            Some(close) => ("", &literal[close + 1..]),
            None => return false,
        },
        None => match host_port.find(':') {
            Some(at) => (&host_port[..at], &host_port[at..]),
            None => (host_port, ""),
        },
    };
    let port_ok = port.is_empty()
        || port
            .strip_prefix(':')
            .is_some_and(|digits| digits.bytes().all(|b| b.is_ascii_digit()));
    port_ok && !host.chars().any(is_host_bad)
}

/// The URL as a tap opens it: scheme and authority as typed (an `@` inside
/// the userinfo escaped, as Apple does), the rest percent-encoded by
/// `encode_rest`.
fn url_target(text: &str, start: usize, url: &Url) -> String {
    let mut out = String::with_capacity(url.end - start + 8);
    out.push_str(&text[start..url.authority.start]);
    let authority = &text[url.authority.clone()];
    match authority.rfind('@') {
        Some(at) => {
            out.push_str(&authority[..at].replace('@', "%40"));
            out.push_str(&authority[at..]);
        }
        None => out.push_str(authority),
    }
    encode_rest(&text[url.authority.end..url.end], &mut out);
    out
}

/// Percent-encode a path, query and fragment the way Apple's URL does:
/// every non-ASCII character as its UTF-8 bytes, a `%` that does not start
/// an escape, a second `#`, and the ASCII characters RFC 3986 does not
/// allow there. Matching it byte for byte matters beyond tidiness: left
/// raw, a browser turns `\` into `/` in a path and asks for a different
/// resource than the phone does.
fn encode_rest(rest: &str, out: &mut String) {
    let bytes = rest.as_bytes();
    let mut fragment = false;
    for (at, c) in rest.char_indices() {
        let escape_start = bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(at + 2).is_some_and(u8::is_ascii_hexdigit);
        match c {
            '%' if escape_start => out.push('%'),
            '#' if !fragment => {
                fragment = true;
                out.push('#');
            }
            '%' | '#' | '[' | '\\' | ']' | '^' | '`' | '{' | '|' | '}' | '<' | '>' => {
                push_escaped(c, out)
            }
            c if c.is_ascii() => out.push(c),
            c => push_escaped(c, out),
        }
    }
}

fn push_escaped(c: char, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut buf = [0u8; 4];
    for &b in c.encode_utf8(&mut buf).as_bytes() {
        out.push('%');
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0x0F)]));
    }
}

// MARK: - Host names without a scheme

/// The top-level domains Apple links a bare host name under — `example.com`
/// is a link, `example.dev` is not, `readme.md` is not. Measured against
/// IANA's whole list: 113 of 1439, plus `fw`, which is not a domain at
/// all. Only `com`, `edu`, `gov`, `net` and `org` match in any case (and
/// `BO` in capitals); every other entry is lowercase only, so `EXAMPLE.RU`
/// is not a link.
const BARE_TLDS: &[&str] = &[
    "ae", "ai", "ar", "at", "au", "be", "bg", "bid", "biz", "bo", "br", "bs", "ca", "cc", "ch",
    "cl", "club", "cm", "cn", "co", "com", "cx", "cz", "de", "dk", "edu", "ee", "es", "eu", "fi",
    "fm", "fr", "fw", "gdn", "gl", "google", "gov", "gr", "hk", "hn", "hr", "hu", "id", "ie", "il",
    "in", "info", "io", "ir", "is", "it", "jo", "jp", "kr", "la", "lc", "li", "link", "lk", "loan",
    "lt", "lu", "lv", "ly", "ma", "me", "ms", "mx", "my", "net", "nl", "no", "nz", "online", "org",
    "ph", "pk", "pl", "pn", "pt", "pw", "ro", "rs", "ru", "sa", "se", "sg", "si", "site", "sk",
    "so", "social", "st", "tech", "th", "tm", "tn", "to", "tr", "tt", "tv", "tw", "ua", "uk", "us",
    "uy", "va", "vip", "vn", "wang", "win", "xin", "xyz", "za",
];

/// The internationalised TLDs Apple links a bare (or `www.`) host under.
const UNICODE_TLDS: &[&str] = &[
    "рус",
    "рф",
    "укр",
    "السعودية",
    "امارات",
    "مصر",
    "مليسيا",
    "भारत",
    "ਭਾਰਤ",
    "ભારત",
    "ไทย",
    "한국",
];

/// …and the ones an email domain may end in on top of those.
const EMAIL_UNICODE_TLDS: &[&str] = &["中国", "中國", "台湾", "台灣", "澳門", "香港"];

/// Every internationalised TLD an email domain may end in.
const EMAIL_TLDS: &[&[&str]] = &[UNICODE_TLDS, EMAIL_UNICODE_TLDS];

fn email_unicode_tld(label: &str) -> bool {
    EMAIL_TLDS.iter().any(|list| list.contains(&label))
}

fn bare_tld(label: &str) -> bool {
    BARE_TLDS.contains(&label)
        || UNICODE_TLDS.contains(&label)
        || label == "BO"
        || ["com", "edu", "gov", "net", "org"]
            .iter()
            .any(|tld| label.eq_ignore_ascii_case(tld))
}

/// Every `example.com`, `www.example.zz/path` or `пример.рф` with no scheme.
fn bare_links(text: &str, found: &mut Vec<Found>) {
    let mut at = 0;
    while let Some(c) = char_at(text, at) {
        // Only at the start of a word: a host does not begin mid-word, and
        // a mark on the letter before still belongs to that letter's word
        // (`e\u{301}xample.com` is not `xample.com`).
        let starts_word = match char_before(text, at) {
            None => true,
            Some(b) if is_combining(b) => !char_before(text, at - b.len_utf8())
                .is_some_and(|base| is_word(base) || is_combining(base)),
            Some(b) => !is_word(b),
        };
        if is_word(c) && starts_word {
            if let Some(f) = bare_link_at(text, at) {
                at = f.end;
                found.push(f);
                continue;
            }
        }
        at += c.len_utf8();
    }
}

fn bare_link_at(text: &str, start: usize) -> Option<Found> {
    // Right after `@` is an address's domain, not a link — unless Apple
    // refused the local part, when the domain is linked on its own:
    // `user.@example.com` links `example.com`, `@example.com` nothing.
    if char_before(text, start) == Some('@')
        && local_part(text, start - 1).is_none_or(|local| local.valid)
    {
        return None;
    }
    let first = label_end(text, start, Labels::Bare)?;
    let www = text[start..first].eq_ignore_ascii_case("www");
    let mut labels = host_labels(
        text,
        start,
        if www { Labels::Www } else { Labels::Bare },
        &[UNICODE_TLDS],
    );
    if labels.len() < 2 {
        return None;
    }
    let mut last = labels[labels.len() - 1].clone();
    // After `www.` a name may be CJK (`www.例子.com`), but the TLD is not:
    // text resuming right after it is the sentence, not more of the host —
    // `www.example.com日本語` links `www.example.com`.
    if www {
        if let Some((at, _)) = text[last.clone()].char_indices().find(|&(_, c)| is_cjk(c)) {
            let cut = &text[last.start..last.start + at];
            if at > 0
                && (cut.bytes().all(|b| b.is_ascii_alphabetic()) || UNICODE_TLDS.contains(&cut))
            {
                last.end = last.start + at;
                let n = labels.len();
                labels[n - 1] = last.clone();
            }
        }
    }
    let tld = &text[last.clone()];
    let known = bare_tld(tld);
    // `www.` lifts the list: any ASCII-letter TLD will do, `www.example.zz`
    // included, as long as there is a name between the two.
    let tld_ok =
        known || (www && labels.len() >= 3 && tld.bytes().all(|b| b.is_ascii_alphabetic()));
    if !tld_ok {
        return None;
    }
    // A letter of a script Apple will not read in a bare host name — an
    // uncased one that is not CJK, or a loose combining mark — voids the
    // whole name rather than ending it: `ไexample.com` is not a link.
    let checked = if UNICODE_TLDS.contains(&tld) {
        &labels[..labels.len() - 1]
    } else {
        &labels[..]
    };
    if checked
        .iter()
        .any(|label| text[label.clone()].chars().any(voids_bare_host))
    {
        return None;
    }
    let host_end = last.end;
    // A local part, or a TLD that was really longer than it looks:
    // `example.com_` and `example.comx` are not links at all.
    if char_at(text, host_end).is_some_and(|c| matches!(c, '@' | '_') || continues_name(c)) {
        return None;
    }
    let authority_end = bare_port(text, host_end).unwrap_or(host_end);
    let mut end = authority_end;
    // A query or fragment counts only after a path: `example.com?q=1`
    // links `example.com`.
    if char_at(text, end) == Some('/') {
        let mut path_end = scan_body(text, end, Brackets::Stop);
        if let Some(at) = first_poison(text, end, path_end) {
            let soft = text[at..path_end].chars().all(is_soft_poison);
            path_end = if soft { at } else { authority_end };
        }
        end = trim_tail(text, start, authority_end, path_end, Brackets::Stop);
    }
    let mut target = String::from("http://");
    target.push_str(&text[start..authority_end]);
    encode_rest(&text[authority_end..end], &mut target);
    Some(Found {
        start,
        end,
        target: Some(target),
    })
}

/// A letter Apple refuses in a bare host name (see `bare_link_at`).
fn voids_bare_host(c: char) -> bool {
    !c.is_ascii()
        && ((c.is_alphabetic() && !c.is_lowercase() && !c.is_uppercase() && !is_cjk(c))
            || is_combining(c))
}

/// `:8080` after a bare host — two to five digits, and then not a letter.
fn bare_port(text: &str, at: usize) -> Option<usize> {
    let rest = text[at..].strip_prefix(':')?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let end = at + 1 + digits;
    let glued = char_at(text, end).is_some_and(is_word);
    ((2..=5).contains(&digits) && !glued).then_some(end)
}

/// Which characters a host-name label may hold.
#[derive(Clone, Copy, PartialEq)]
enum Labels {
    /// A bare host: letters and digits of the spaced scripts.
    Bare,
    /// A bare host after `www.`, where CJK names count too.
    Www,
    /// An email domain: any letter or decimal digit, CJK included.
    Email,
}

fn is_label_char(c: char, kind: Labels) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric();
    }
    match kind {
        Labels::Bare => is_word(c) || is_combining(c),
        Labels::Www | Labels::Email => is_word(c) || is_cjk(c) || is_combining(c),
    }
}

/// Where the label starting at `at` ends: letters and digits, with single
/// `-` or `_` between them, and `xn--` allowed to start one. Two
/// separators in a row end the label — `ab--cd.com` links `cd.com`.
fn label_end(text: &str, at: usize, kind: Labels) -> Option<usize> {
    let mut pos = if text[at..].starts_with("xn--") {
        at + 4
    } else {
        at
    };
    let mut end = at;
    loop {
        let run = pos;
        while let Some(c) = char_at(text, pos) {
            if !is_label_char(c, kind) {
                break;
            }
            pos += c.len_utf8();
        }
        if pos == run {
            break;
        }
        end = pos;
        match char_at(text, pos) {
            Some('-' | '_') if char_at(text, pos + 1).is_some_and(|c| is_label_char(c, kind)) => {
                pos += 1
            }
            _ => break,
        }
    }
    (end > at).then_some(end)
}

/// The labels of the host name starting at `start`, as far as they go. A
/// known internationalised TLD counts as a label even in a script the
/// label rules would not take (`example.한국`).
fn host_labels(
    text: &str,
    start: usize,
    kind: Labels,
    unicode_tlds: &[&[&str]],
) -> Vec<Range<usize>> {
    let mut labels = Vec::new();
    let mut at = start;
    loop {
        let unicode_tld = (at > start)
            .then(|| {
                unicode_tlds
                    .iter()
                    .flat_map(|list| list.iter())
                    .find(|tld| {
                        text[at..].starts_with(*tld)
                            && !char_at(text, at + tld.len())
                                .is_some_and(|c| is_label_char(c, kind))
                    })
            })
            .flatten();
        let end = match unicode_tld {
            Some(tld) => at + tld.len(),
            None => match label_end(text, at, kind) {
                Some(end) => end,
                None => break,
            },
        };
        labels.push(at..end);
        if unicode_tld.is_some() || char_at(text, end) != Some('.') {
            break;
        }
        at = end + 1;
    }
    labels
}

// MARK: - Email addresses

fn email_links(text: &str, found: &mut Vec<Found>) {
    for (at, _) in text.match_indices('@') {
        let Some(local) = local_part(text, at)
            .filter(|local| local.valid)
            .map(|local| local.start)
        else {
            continue;
        };
        let Some(domain_end) = email_domain(text, at + 1) else {
            continue;
        };
        let (start, mut target) = match mailto_prefix(text, local) {
            Some(prefix) => (prefix, text[prefix..local].to_owned()),
            None => (local, String::from("mailto:")),
        };
        let mut end = domain_end;
        // `mailto:` brings its query with it: `?subject=…`.
        if start < local && char_at(text, domain_end) == Some('?') {
            let raw = scan_body(text, domain_end, Brackets::Balanced);
            let query_end = cut_unclosed(text, domain_end, raw);
            end = trim_tail(text, start, domain_end, query_end, Brackets::Balanced);
        }
        encode_local(&text[local..at], &mut target);
        target.push('@');
        target.push_str(&text[at + 1..domain_end]);
        encode_rest(&text[domain_end..end], &mut target);
        found.push(Found {
            start,
            end,
            target: Some(target),
        });
    }
}

/// An address's local part, ending at an `@`.
struct Local {
    start: usize,
    /// Whether Apple takes it. A refused one — ending in a dot, or holding
    /// a letter Apple does not take there — is not an address, and its
    /// domain is then linked on its own as a web host.
    valid: bool,
}

fn local_part(text: &str, at: usize) -> Option<Local> {
    let mut start = at;
    let mut spoiled = false;
    while let Some(c) = char_before(text, start) {
        if spoils_local(c) {
            spoiled = true;
        } else if !is_local_char(c) {
            break;
        }
        start -= c.len_utf8();
    }
    // What a local part may not start with is dropped rather than refused:
    // `.a@example.com` and `+user@example.com` link without it.
    while let Some(c) = char_at(text, start) {
        if start >= at || starts_local(c) {
            break;
        }
        start += c.len_utf8();
    }
    (start < at).then(|| Local {
        start,
        valid: !spoiled && !text[..at].ends_with('.'),
    })
}

/// RFC 5322's atext, and beyond ASCII what Apple takes: cased letters
/// (Latin, Greek, Cyrillic, Armenian, Georgian…), numbers, and the
/// letters of the handful of uncased scripts in `in_email_script`.
fn is_local_char(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || "!#$%&'*+-./=?^_`{|}~".contains(c)
    } else {
        c.is_lowercase()
            || c.is_uppercase()
            || c.is_numeric()
            || (in_email_script(c) && !is_url_stop(c))
    }
}

/// A letter or mark Apple reads as PART of a local part and then refuses
/// the whole address over — the other scripts, and loose combining
/// diacritics (`us\u{301}er@example.com` is not an address, and links
/// `example.com`). Han and kana are not: they end a local part instead.
fn spoils_local(c: char) -> bool {
    !c.is_ascii()
        && !is_local_char(c)
        && !is_han_or_kana(c)
        && (c.is_alphabetic() || is_combining(c) || c == '\u{AD}')
}

/// Hebrew, Arabic, Devanagari, Gurmukhi, Gujarati, Thai and Hangul — the
/// uncased scripts whose letters Apple takes in a local part. Measured.
fn in_email_script(c: char) -> bool {
    matches!(c as u32,
        0x0591..=0x05F4 | 0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF
        | 0x0900..=0x097F | 0x0A00..=0x0A7F | 0x0A80..=0x0AFF | 0x0E01..=0x0E5B
        | 0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF
        | 0xFB1D..=0xFB4F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF | 0xFFA0..=0xFFDC)
}

/// What Apple lets a local part start with — measured, and odd: `_`, `#`
/// and `?` stay, every other symbol is dropped.
fn starts_local(c: char) -> bool {
    !c.is_ascii() || c.is_ascii_alphanumeric() || matches!(c, '_' | '#' | '?')
}

/// The start of a `mailto:` (any case) right before `local`, when it is
/// not glued to a word: `xmailto:a@b.c` links only the address.
fn mailto_prefix(text: &str, local: usize) -> Option<usize> {
    let prefix = local.checked_sub(7)?;
    let written = text.get(prefix..local)?;
    (written.eq_ignore_ascii_case("mailto:") && !char_before(text, prefix).is_some_and(is_word))
        .then_some(prefix)
}

/// Where the domain starting at `from` ends, or None when it is not one.
fn email_domain(text: &str, from: usize) -> Option<usize> {
    let labels = host_labels(text, from, Labels::Email, EMAIL_TLDS);
    if labels.len() < 2 {
        return None;
    }
    let mut last = labels[labels.len() - 1].clone();
    // A domain may be CJK (`user@例子.中国`), but CJK straight after a
    // Latin TLD is the sentence resuming: `user@example.com日本語`.
    if let Some((at, _)) = text[last.clone()].char_indices().find(|&(_, c)| is_cjk(c)) {
        let cut = &text[last.start..last.start + at];
        if at > 0 && (cut.bytes().all(|b| b.is_ascii_alphabetic()) || email_unicode_tld(cut)) {
            last.end = last.start + at;
        }
    }
    let tld = &text[last.clone()];
    let octet = |label: &Range<usize>| {
        let digits = &text[label.clone()];
        digits.bytes().all(|b| b.is_ascii_digit()) && digits.parse::<u16>().is_ok_and(|n| n <= 255)
    };
    let dotted_quad = labels.len() == 4 && labels[..3].iter().all(octet) && octet(&last);
    let end =
        if dotted_quad || tld.bytes().all(|b| b.is_ascii_alphabetic()) || email_unicode_tld(tld) {
            last.end
        } else {
            // `user@example.com-x` is `user@example.com`: a last label whose
            // letters run into a hyphen is cut at the hyphen.
            let letters = tld.bytes().take_while(u8::is_ascii_alphabetic).count();
            if letters > 0 && tld.as_bytes().get(letters) == Some(&b'-') {
                last.start + letters
            } else {
                return None;
            }
        };
    // A digit, `_` or a letter right after means the TLD was not where the
    // labels said it ended: `user@example.com5` is nothing at all.
    let glued =
        char_at(text, end).is_some_and(|c| c.is_ascii_digit() || c == '_' || continues_name(c));
    (!glued).then_some(end)
}

/// A local part as Apple's `mailto:` URL spells it.
fn encode_local(local: &str, out: &mut String) {
    for c in local.chars() {
        match c {
            '%' | '#' | '^' | '`' | '{' | '}' | '|' => push_escaped(c, out),
            c if c.is_ascii() => out.push(c),
            c => push_escaped(c, out),
        }
    }
}

// MARK: - Phone numbers

fn phone_links(text: &str, found: &mut Vec<Found>) {
    let mut at = 0;
    while let Some(c) = char_at(text, at) {
        if c == '+' || c == '(' || is_digit(c) {
            match phone_start(text, at) {
                Start::Here => match phone_at(text, at) {
                    Ok(f) => {
                        at = f.end;
                        found.push(f);
                        continue;
                    }
                    Err(Refused::Run) => {
                        at = run_end(text, at);
                        continue;
                    }
                    Err(Refused::Here) => {}
                },
                Start::NotThisRun => {
                    at = run_end(text, at);
                    continue;
                }
                Start::NotHere => {}
            }
        }
        at += c.len_utf8();
    }
}

/// Why no number starts where one was tried.
enum Refused {
    /// Not at this start; a later group may still begin one —
    /// `555)123-4567` links `123-4567`.
    Here,
    /// More digits than any number has: none anywhere in this run, rather
    /// than a wrong one made of part of it.
    Run,
}

enum Start {
    Here,
    /// Not here, and not anywhere in the run of digits that follows either.
    NotThisRun,
    /// Not here, but maybe at the next group: `$555-123-4567` is money
    /// followed by `123-4567`, which Apple links.
    NotHere,
}

/// Whether a phone number may start at `at`, judging by what is before it.
fn phone_start(text: &str, at: usize) -> Start {
    let Some(before) = char_before(text, at) else {
        return Start::Here;
    };
    let before_that = char_before(text, at - before.len_utf8());
    match before {
        // `a555-123-4567` is a code, not a number; `/555` a path; and a
        // number after `*` is a service code Apple mis-dials (see module
        // docs) — no link is better than a wrong number.
        c if is_digit(c) => Start::NotHere,
        c if c.is_ascii_alphabetic() => Start::NotThisRun,
        // After `+` the `+` was the start, and it was refused.
        '/' | '*' | '_' | '+' => Start::NotThisRun,
        // Decimals, thousands and clock times: `3.141592653`, `12:30:45`.
        '.' | ',' | ':' if before_that.is_some_and(|c| c.is_ascii_digit()) => Start::NotThisRun,
        '$' | '€' | '£' | '¥' | '₽' | '₴' | '₹' | '₩' | '¢' => Start::NotHere,
        _ => Start::Here,
    }
}

/// The end of the run of digits and separators at `at`.
fn run_end(text: &str, at: usize) -> usize {
    let mut end = at;
    while let Some(c) = char_at(text, end) {
        let spaced_digit =
            matches!(c, ' ' | '\u{A0}') && char_at(text, end + c.len_utf8()).is_some_and(is_digit);
        if !(is_digit(c) || matches!(c, '-' | '.' | '/' | '(' | ')' | '+') || spaced_digit) {
            break;
        }
        end += c.len_utf8();
    }
    // Always past the character it was asked about, and on a boundary.
    if end > at {
        end
    } else {
        at + char_at(text, at).map_or(1, char::len_utf8)
    }
}

/// The phone number starting at `at`, if one does.
fn phone_at(text: &str, at: usize) -> Result<Found, Refused> {
    let (end, phone) = if let Some(end) = vanity_at(text, at) {
        (end, text[at..end].to_owned())
    } else if let Some(end) = called_at(text, at) {
        (end, text[at..end].to_owned())
    } else {
        let number = number_at(text, at)?;
        match extension_at(text, number.end) {
            Some((end, digits)) => (end, format!("{};{digits}", number.dialled)),
            None => {
                // What follows must not carry on the number:
                // `555-123-4567руб`, `555-123-4567-`, `555-123-4567x` and
                // `555-123-4567#` are none of them numbers to Apple.
                if char_at(text, number.end).is_some_and(|c| {
                    is_word(c)
                        || is_combining(c)
                        || matches!(
                            c,
                            '-' | '%'
                                | '°'
                                | '_'
                                | '#'
                                | '+'
                                | '='
                                | '&'
                                | '@'
                                | '$'
                                | '~'
                                | '^'
                                | '`'
                                | '\\'
                        )
                }) {
                    return Err(Refused::Here);
                }
                (number.end, number.dialled)
            }
        }
    };
    let target = tel_url(&phone).ok_or(Refused::Here)?;
    Ok(Found {
        start: at,
        end,
        target: Some(target),
    })
}

/// `tel:+15551234567` written out: linked whole, to exactly what was
/// written — Apple's `.link` result, which Swift passes through untouched
/// rather than through `tel_url`. Lowercase `tel:` only, as Apple reads it,
/// and never when the number reads on past what the literal holds:
/// `tel:+1 555 123 4567` is the phone number `+1 555 123 4567`.
fn tel_links(text: &str, found: &mut Vec<Found>) {
    for (at, _) in text.match_indices("tel:") {
        if char_before(text, at).is_some_and(is_word) {
            continue;
        }
        let body = at + 4;
        let raw = scan_body(text, body, Brackets::Balanced);
        let end = trim_tail(
            text,
            at,
            body,
            cut_unclosed(text, body, raw),
            Brackets::Balanced,
        );
        let written = &text[body..end];
        // Apple links `tel:abc` too; a literal with no digit dials nothing,
        // so it is not a link here.
        if !written.bytes().any(|b| b.is_ascii_digit()) {
            continue;
        }
        if phone_at(text, body).is_ok_and(|phone| phone.end > end) {
            continue;
        }
        let mut target = String::from("tel:");
        encode_rest(written, &mut target);
        found.push(Found {
            start: at,
            end,
            target: Some(target),
        });
    }
}

/// A run of digit groups, as the tokenizer found it.
struct Number {
    end: usize,
    /// The number as Apple hands it to Swift's `telURL`: the matched text
    /// with fullwidth digits in ASCII and a `(0)` trunk after a country
    /// code dropped.
    dialled: String,
}

/// One group of digits: `555`, or `(555)`.
struct Group {
    start: usize,
    end: usize,
    digits: usize,
    paren: bool,
}

impl Group {
    /// Its first digit, in ASCII — past the bracket of a `(555)`.
    fn lead(&self, text: &str) -> Option<char> {
        char_at(text, self.start + usize::from(self.paren)).map(ascii_digit)
    }
}

/// A group of digits starting at `at`.
fn group_at(text: &str, at: usize) -> Option<Group> {
    let paren = char_at(text, at) == Some('(');
    let mut pos = if paren { at + 1 } else { at };
    let mut digits = 0;
    while let Some(c) = char_at(text, pos) {
        if !is_digit(c) {
            break;
        }
        digits += 1;
        pos += c.len_utf8();
    }
    if digits == 0 {
        return None;
    }
    if paren {
        if digits > 5 || char_at(text, pos) != Some(')') {
            return None;
        }
        pos += 1;
    }
    Some(Group {
        start: at,
        end: pos,
        digits,
        paren,
    })
}

/// The separators Apple reads between the groups of a number.
fn is_separator(c: char) -> bool {
    matches!(
        c,
        ' ' | '-' | '.' | '/' | '\u{A0}' | '\u{2013}' | '\u{2014}'
    )
}

/// The number starting at `at`: `+CC …` international, or a national one
/// in groups, checked against the shapes numbers have and the shapes
/// dates, years and postcodes have instead.
fn number_at(text: &str, at: usize) -> Result<Number, Refused> {
    let plus = char_at(text, at) == Some('+');
    let mut pos = if plus { at + 1 } else { at };
    let mut groups: Vec<Group> = Vec::new();
    let mut separators: Vec<char> = Vec::new();
    while let Some(group) = group_at(text, pos) {
        pos = group.end;
        let paren = group.paren;
        let leading_zero = groups.is_empty() && !group.paren && group.lead(text) == Some('0');
        groups.push(group);
        // Reading stops long after any number would have: what was read is
        // judged below, and a run this long is refused whole.
        if groups.len() > 12 || groups.iter().map(|g| g.digits).sum::<usize>() > 30 {
            break;
        }
        match char_at(text, pos) {
            // `/` only as a German-style `030/123456` area-code break.
            Some('/') if !(groups.len() == 1 && leading_zero) => break,
            // Hyphenated groups and then a space are a number and then
            // another: `555-123-4567 555-765-4321` is two.
            Some(' ' | '\u{A0}')
                if separators
                    .iter()
                    .any(|s| matches!(s, '-' | '.' | '\u{2013}' | '\u{2014}')) =>
            {
                break
            }
            Some(sep) if is_separator(sep) && group_at(text, pos + sep.len_utf8()).is_some() => {
                separators.push(sep);
                pos += sep.len_utf8();
            }
            // `(555)1234567`, `8(800)5553535`: brackets need no separator.
            Some(c)
                if (paren && is_digit(c))
                    || (c == '(' && group_at(text, pos).is_some_and(|g| g.paren)) =>
            {
                separators.push('\0');
            }
            _ => break,
        }
    }
    if groups.is_empty() {
        return Err(Refused::Here);
    }
    if groups.iter().map(|g| g.digits).sum::<usize>() > 15 {
        // `5551234567 5557654321`: a whole unseparated number, then more.
        // Anything else this long is no number at all — Apple reads some
        // such runs as one long wrong number, which is worse than none.
        let whole = &groups[..1];
        if plus || whole[0].digits < 9 || !national_shape(text, whole, &[], whole[0].digits) {
            return Err(Refused::Run);
        }
        pos = groups[0].end;
        groups.truncate(1);
        separators.clear();
    }
    let first = &groups[0];
    // `+44 (0)20 …`: the trunk zero is how the number is said at home, not
    // dialled from abroad, and Apple drops it.
    let trunk = plus
        && groups.len() > 1
        && groups[1].paren
        && text[groups[1].start..groups[1].end] == *"(0)";
    let total: usize = groups.iter().map(|g| g.digits).sum::<usize>() - usize::from(trunk);
    let valid = if plus {
        !first.paren
            && if groups.len() == 1 {
                (8..=13).contains(&total)
            } else {
                (1..=3).contains(&first.digits) && (8..=15).contains(&total)
            }
    } else {
        national_shape(text, &groups, &separators, total)
    };
    if !valid {
        return Err(Refused::Here);
    }
    let mut dialled = String::new();
    for (i, c) in text[at..pos].char_indices() {
        let absolute = at + i;
        if trunk && (groups[1].start..groups[1].end).contains(&absolute) {
            continue;
        }
        dialled.push(if is_digit(c) { ascii_digit(c) } else { c });
    }
    Ok(Number { end: pos, dialled })
}

/// Whether groups with no country code have a phone number's shape.
fn national_shape(text: &str, groups: &[Group], separators: &[char], total: usize) -> bool {
    let lengths: Vec<usize> = groups.iter().map(|g| g.digits).collect();
    if groups.len() == 1 {
        // Unseparated, Apple takes 9, 10, 11, 13 and 14 digits — not 12.
        return !groups[0].paren && matches!(total, 9 | 10 | 11 | 13 | 14);
    }
    if !(7..=15).contains(&total) {
        return false;
    }
    // A single digit is a group only as a leading trunk or country code
    // before ten more: `1 555 123 4567`, `8 800 555 35 35`. Anywhere else
    // it is a date, a version or a list: `1-2-3`, `978-0-306-40615-7`.
    for (i, g) in groups.iter().enumerate() {
        if g.digits == 1
            && !(i == 0 && !g.paren && matches!(g.lead(text), Some('1' | '8')) && total == 11)
        {
            return false;
        }
    }
    let years =
        |g: &Group| g.digits == 4 && matches!(text.get(g.start..g.start + 2), Some("19" | "20"));
    match lengths.as_slice() {
        // Dates and postcodes.
        [4, 2, 2] | [2, 2, 4] | [5, 4] => false,
        [4, 4] if years(&groups[0]) && years(&groups[1]) => false,
        // Seven digits come as 555-1234, or in pairs only with hyphens.
        _ if total == 7 => match lengths.as_slice() {
            [3, 4] => true,
            [3, 2, 2] | [2, 2, 3] => separators.iter().all(|&s| s == '-'),
            _ => false,
        },
        _ => true,
    }
}

/// ` x89`, ` ext. 89`, `, ext 89`, ` (ext 89)`, `;89`, `;ext=89` after a
/// number: where it ends, and its digits.
fn extension_at(text: &str, at: usize) -> Option<(usize, String)> {
    let rest = &text[at..];
    let (skip, min_digits, closing) = extension_key(rest)?;
    let digits_start = at + skip;
    let mut pos = digits_start;
    let mut digits = String::new();
    while let Some(c) = char_at(text, pos) {
        if !is_digit(c) || digits.len() == 7 {
            break;
        }
        digits.push(ascii_digit(c));
        pos += c.len_utf8();
    }
    if digits.len() < min_digits {
        return None;
    }
    if closing {
        if char_at(text, pos) != Some(')') {
            return None;
        }
        pos += 1;
    }
    if char_at(text, pos).is_some_and(|c| is_word(c) || c.is_ascii_digit()) {
        return None;
    }
    Some((pos, digits))
}

/// The words that introduce an extension, with what they need: how many
/// bytes to skip, the fewest digits, and whether a `)` closes it.
fn extension_key(rest: &str) -> Option<(usize, usize, bool)> {
    let bytes = rest.as_bytes();
    let mut i = 0;
    if let Some(after) = rest.strip_prefix(" (") {
        let key = key_len(after)?;
        let space = usize::from(after.as_bytes().get(key) == Some(&b' '));
        return Some((2 + key + space, 1, true));
    }
    if bytes.first() == Some(&b',') && bytes.get(1) == Some(&b' ') {
        i = 2;
    } else if bytes.first() == Some(&b' ') {
        i = 1;
    }
    if bytes.get(i) == Some(&b';') {
        let key = if rest[i + 1..].starts_with("ext=") {
            4
        } else {
            0
        };
        return Some((i + 1 + key, 1, false));
    }
    let key = key_len(&rest[i..])?;
    let x = key == 1;
    // A bare `x` needs two digits (`x1` is not an extension), and a comma
    // only ever comes before a spelled-out one.
    if x && i == 2 {
        return None;
    }
    i += key;
    if bytes.get(i) == Some(&b'.') {
        i += 1;
    }
    if bytes.get(i) == Some(&b':') {
        i += 1;
    }
    if bytes.get(i) == Some(&b' ') {
        i += 1;
    }
    Some((i, if x { 2 } else { 1 }, false))
}

fn key_len(rest: &str) -> Option<usize> {
    for key in ["extension", "ext", "x"] {
        if rest
            .get(..key.len())
            .is_some_and(|w| w.eq_ignore_ascii_case(key))
        {
            return Some(key.len());
        }
    }
    None
}

/// `1-800-GOT-JUNK`: a North American number with its last seven to nine
/// characters spelled in capitals. Without the leading 1 the area code has
/// to be toll-free — Apple links `800-FLOWERS`, not `212-GOT-JUNK`.
fn vanity_at(text: &str, at: usize) -> Option<usize> {
    let mut pos = at;
    let mut country = false;
    if text[pos..].starts_with("+1") || text[pos..].starts_with('1') {
        country = true;
        pos += if text[pos..].starts_with('+') { 2 } else { 1 };
        if matches!(char_at(text, pos), Some(' ' | '-' | '.')) {
            pos += 1;
        }
    }
    let paren = char_at(text, pos) == Some('(');
    let area_start = pos + usize::from(paren);
    let area = text.get(area_start..area_start + 3)?;
    if !area.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    pos = area_start + 3;
    if paren {
        if char_at(text, pos) != Some(')') {
            return None;
        }
        pos += 1;
    }
    if !country && !matches!(area, "800" | "888" | "877" | "866" | "855" | "844" | "833") {
        return None;
    }
    if matches!(char_at(text, pos), Some(' ' | '-' | '.')) {
        pos += 1;
    }
    if !char_at(text, pos).is_some_and(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let mut count = 0;
    let mut end = pos;
    loop {
        let chunk = text[pos..]
            .bytes()
            .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            .count();
        if chunk == 0 || count + chunk > 9 {
            break;
        }
        count += chunk;
        pos += chunk;
        end = pos;
        match text.as_bytes().get(pos) {
            Some(b'-' | b' ')
                if text
                    .as_bytes()
                    .get(pos + 1)
                    .is_some_and(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) =>
            {
                pos += 1
            }
            _ => break,
        }
    }
    // Carrying on past nine — `1-800-FLOWERS-NOW` — is not a number.
    let glued = char_at(text, end).is_some_and(|c| c.is_alphanumeric())
        || (text.as_bytes().get(end) == Some(&b'-')
            && text
                .as_bytes()
                .get(end + 1)
                .is_some_and(|b| b.is_ascii_uppercase() || b.is_ascii_digit()));
    ((7..=9).contains(&count) && !glued).then_some(end)
}

/// `call 911`: three to six digits right after the word "call" or "dial"
/// (any case, then spaces or a colon) are a number Apple links, although
/// on their own they would not be.
fn called_at(text: &str, at: usize) -> Option<usize> {
    let before = text[..at].trim_end_matches([' ', ':']);
    let word = before.len().checked_sub(4)?;
    let spoken = before.get(word..)?;
    if before.len() == at
        || !(spoken.eq_ignore_ascii_case("call") || spoken.eq_ignore_ascii_case("dial"))
        || char_before(text, word).is_some_and(is_word)
    {
        return None;
    }
    let digits = text[at..].bytes().take_while(u8::is_ascii_digit).count();
    let end = at + digits;
    let glued = char_at(text, end).is_some_and(|c| {
        is_word(c)
            || c == '-'
            || (is_separator(c) && char_at(text, end + c.len_utf8()).is_some_and(is_digit))
    });
    ((3..=6).contains(&digits) && !glued).then_some(end)
}

/// Swift's `telURL(for:)`: a detector phone match as a dialable `tel:` URL.
///
/// Two traps a naive digits-only filter falls into: an extension comes
/// back normalized behind a `;` ("555-123-4567 x89" → "555-123-4567;89"),
/// and concatenating those digits dials a wrong, longer number — so the
/// extension rides separately in RFC 3966 `;ext=` form. And vanity letters
/// (1-800-GOT-JUNK) are part of the number, mapped to their keypad digits
/// rather than stripped.
fn tel_url(phone: &str) -> Option<String> {
    let (main, extension) = split_extension(phone);
    let number = dialable_digits(main?);
    if number.is_empty() {
        return None;
    }
    let mut tel = format!("tel:{number}");
    if let Some(extension) = extension {
        // Swift filters with `Character.isNumber` — Unicode's Numeric_Type,
        // far wider than ASCII (`٣`, `½`, `一` are all numbers to it). The
        // only extension this file ever builds is ASCII digits, and on
        // ASCII that predicate is exactly 0-9 (checked against Swift), so
        // the narrower one is right for every input that can reach here.
        // Widen the grammar and this has to widen with it.
        let digits: String = extension.chars().filter(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            tel.push_str(";ext=");
            tel.push_str(&digits);
        }
    }
    Some(tel)
}

/// Swift's `phone.split(separator: ";", maxSplits: 1,
/// omittingEmptySubsequences: true)`, which is not `split_once`: leading
/// `;`s are dropped without counting as the split (";89" is one part,
/// "89"), and an empty remainder is no part at all ("a;" is just "a").
fn split_extension(phone: &str) -> (Option<&str>, Option<&str>) {
    let phone = phone.trim_start_matches(';');
    if phone.is_empty() {
        return (None, None);
    }
    match phone.split_once(';') {
        Some((main, rest)) => (Some(main), (!rest.is_empty()).then_some(rest)),
        None => (Some(phone), None),
    }
}

/// Swift's `dialableDigits`: digits, `+` and `*` pass through, vanity
/// letters become their keypad digit, and `#` is percent-encoded — bare,
/// it would start a fragment and silently truncate the dial string.
///
/// Swift upper-cases the whole string and walks it by Character, so a
/// letter with a combining mark after it is a different Character and not
/// that letter, and `"0"..."9"` even takes `1` + U+0301. Neither can reach
/// here — the numbers this file builds hold ASCII digits and capitals,
/// separators and nothing combining — and on that domain `to_uppercase`
/// and walking by char are Swift's rules exactly.
fn dialable_digits(text: &str) -> String {
    let mut out = String::new();
    for c in text.to_uppercase().chars() {
        match c {
            '0'..='9' | '+' | '*' => out.push(c),
            '#' => out.push_str("%23"),
            'A' | 'B' | 'C' => out.push('2'),
            'D' | 'E' | 'F' => out.push('3'),
            'G' | 'H' | 'I' => out.push('4'),
            'J' | 'K' | 'L' => out.push('5'),
            'M' | 'N' | 'O' => out.push('6'),
            'P' | 'Q' | 'R' | 'S' => out.push('7'),
            'T' | 'U' | 'V' => out.push('8'),
            'W' | 'X' | 'Y' | 'Z' => out.push('9'),
            _ => {}
        }
    }
    out
}

// MARK: - Apple's character tables
//
// Measured, not derived: for every code point assigned on macOS 26, where
// NSDataDetector left a URL that had that character in its path, at its
// end, and at the end of its host (ASCII is handled in code above).
// Unassigned code points fall wherever their neighbours do.

/// Non-ASCII characters that end a URL (`is_url_stop`).
#[rustfmt::skip]
const URL_STOP: &[(u32, u32)] = &[
    (0x0080, 0x00A0), (0x05C0, 0x05C0), (0x05C3, 0x05C3), (0x05C6, 0x05C6), (0x0606, 0x060A),
    (0x060C, 0x060D), (0x061B, 0x061B), (0x061D, 0x061F), (0x066A, 0x066A), (0x066C, 0x066D),
    (0x06D4, 0x06D4), (0x0700, 0x070D), (0x07F7, 0x07F9), (0x0830, 0x083E), (0x085E, 0x085E),
    (0x0964, 0x0965), (0x0970, 0x0970), (0x09F4, 0x09F9), (0x09FD, 0x09FD), (0x0A76, 0x0A76),
    (0x0AF0, 0x0AF0), (0x0B72, 0x0B77), (0x0BF0, 0x0BF2), (0x0C77, 0x0C7E), (0x0C84, 0x0C84),
    (0x0D58, 0x0D5E), (0x0D70, 0x0D78), (0x0DF4, 0x0DF4), (0x0E4F, 0x0E4F), (0x0E5A, 0x0E5B),
    (0x0F04, 0x0F12), (0x0F14, 0x0F14), (0x0F2A, 0x0F33), (0x0F3A, 0x0F3D), (0x0F85, 0x0F85),
    (0x0FD0, 0x0FD4), (0x0FD9, 0x0FDA), (0x104A, 0x104F), (0x10FB, 0x10FB), (0x1360, 0x137C),
    (0x166E, 0x166E), (0x1680, 0x1680), (0x169B, 0x169C), (0x16EB, 0x16ED), (0x1735, 0x1736),
    (0x17D4, 0x17D6), (0x17D8, 0x17DA), (0x17F0, 0x1805), (0x1807, 0x180A), (0x1944, 0x1945),
    (0x1A1E, 0x1A1F), (0x1B5A, 0x1B60), (0x1B7D, 0x1B7E), (0x1BFC, 0x1BFF), (0x1C3B, 0x1C3F),
    (0x1C7E, 0x1C7F), (0x1CC0, 0x1CC7), (0x1CD3, 0x1CD3), (0x2000, 0x200B), (0x2016, 0x2017),
    (0x201A, 0x201B), (0x201E, 0x2023), (0x2025, 0x2025), (0x2028, 0x2029), (0x202F, 0x202F),
    (0x205F, 0x205F), (0x2070, 0x2070), (0x2074, 0x207E), (0x2080, 0x208E), (0x2118, 0x2118),
    (0x2140, 0x2144), (0x214B, 0x214B), (0x2150, 0x215F), (0x2189, 0x2189), (0x2200, 0x2211),
    (0x2213, 0x22FF), (0x2308, 0x230B), (0x2320, 0x2321), (0x2329, 0x232A), (0x237C, 0x237C),
    (0x239B, 0x23B3), (0x23DC, 0x23E1), (0x2460, 0x249B), (0x24EA, 0x24FF), (0x25B7, 0x25B7),
    (0x25C1, 0x25C1), (0x25F8, 0x25FF), (0x266F, 0x266F), (0x27C0, 0x27FF), (0x2900, 0x2AFF),
    (0x2CF9, 0x2CFF), (0x2D70, 0x2D70), (0x2E00, 0x2E16), (0x2E18, 0x2E19), (0x2E1B, 0x2E2E),
    (0x2E30, 0x2E39), (0x2E3C, 0x2E3F), (0x2E41, 0x2E4F), (0x2E52, 0x2E5C), (0x2FFC, 0x3003),
    (0x3007, 0x3011), (0x3014, 0x301B), (0x301D, 0x301F), (0x3021, 0x3029), (0x3038, 0x303A),
    (0x303D, 0x303D), (0x3192, 0x3195), (0x31EF, 0x31EF), (0x3220, 0x3229), (0x3248, 0x324F),
    (0x3251, 0x325F), (0x3280, 0x3289), (0x32B1, 0x32BF), (0xA4FE, 0xA4FF), (0xA60D, 0xA60F),
    (0xA673, 0xA673), (0xA67E, 0xA67E), (0xA6F2, 0xA6F7), (0xA830, 0xA835), (0xA874, 0xA877),
    (0xA8CE, 0xA8CF), (0xA8F8, 0xA8FA), (0xA8FC, 0xA8FC), (0xA92E, 0xA92F), (0xA95F, 0xA95F),
    (0xA9C1, 0xA9CD), (0xA9DE, 0xA9DF), (0xAA5C, 0xAA5F), (0xAAF0, 0xAAF1), (0xABEB, 0xABEB),
    (0xE000, 0xF7F2), (0xF8A1, 0xF8A7), (0xF8B4, 0xF8B7), (0xF8BB, 0xF8C0), (0xFB29, 0xFB29),
    (0xFD3E, 0xFD3F), (0xFE10, 0xFE12), (0xFE14, 0xFE19), (0xFE30, 0xFE30), (0xFE35, 0xFE4C),
    (0xFE50, 0xFE51), (0xFE54, 0xFE57), (0xFE59, 0xFE62), (0xFE64, 0xFE68), (0xFE6A, 0xFE6B),
    (0xFF01, 0xFF03), (0xFF05, 0xFF06), (0xFF08, 0xFF0C), (0xFF0E, 0xFF0F), (0xFF1A, 0xFF20),
    (0xFF3B, 0xFF3D), (0xFF5B, 0xFF65), (0xFFE2, 0xFFE2), (0xFFE9, 0xFFEC), (0xFFFC, 0xFFFD),
    (0x10100, 0x10133), (0x10175, 0x10178), (0x1018A, 0x1018B), (0x102E1, 0x102FB),
    (0x10320, 0x10323), (0x1039F, 0x1039F), (0x103D0, 0x103D0), (0x1056F, 0x1056F),
    (0x10857, 0x1085F), (0x10879, 0x1087F), (0x108A7, 0x108AF), (0x108FB, 0x108FF),
    (0x10916, 0x1091F), (0x1093F, 0x1093F), (0x109BC, 0x109BD), (0x109C0, 0x109FF),
    (0x10A40, 0x10A58), (0x10A7D, 0x10A7F), (0x10A9D, 0x10A9F), (0x10AEB, 0x10AF6),
    (0x10B39, 0x10B3F), (0x10B58, 0x10B5F), (0x10B78, 0x10B7F), (0x10B99, 0x10BAF),
    (0x10CFA, 0x10CFF), (0x10E60, 0x10E7E), (0x10F1D, 0x10F26), (0x10F51, 0x10F59),
    (0x10F86, 0x10F89), (0x10FC5, 0x10FCB), (0x11047, 0x11065), (0x110BB, 0x110BC),
    (0x110BE, 0x110C1), (0x11140, 0x11143), (0x11174, 0x11175), (0x111C5, 0x111C8),
    (0x111CD, 0x111CD), (0x111DB, 0x111DB), (0x111DD, 0x111F4), (0x11238, 0x1123D),
    (0x112A9, 0x112A9), (0x1144B, 0x1144F), (0x1145A, 0x1145D), (0x114C6, 0x114C6),
    (0x115C1, 0x115D7), (0x11641, 0x11643), (0x11660, 0x1166C), (0x116B9, 0x116B9),
    (0x1173C, 0x1173E), (0x1183B, 0x1183B), (0x118EA, 0x118F2), (0x11944, 0x11946),
    (0x119E2, 0x119E2), (0x11A3F, 0x11A46), (0x11A9A, 0x11A9C), (0x11A9E, 0x11AA2),
    (0x11B00, 0x11B09), (0x11C41, 0x11C45), (0x11C5A, 0x11C71), (0x11EF7, 0x11EF8),
    (0x11F43, 0x11F4F), (0x11FC0, 0x11FD4), (0x11FFF, 0x11FFF), (0x12470, 0x12474),
    (0x12FF1, 0x12FF2), (0x16A6E, 0x16A6F), (0x16AF5, 0x16AF5), (0x16B37, 0x16B3B),
    (0x16B44, 0x16B44), (0x16B5B, 0x16B61), (0x16E80, 0x16E9A), (0x16FE2, 0x16FE2),
    (0x1BC9F, 0x1BC9F), (0x1D2C0, 0x1D2F3), (0x1D360, 0x1D378), (0x1D6C1, 0x1D6C1),
    (0x1D6DB, 0x1D6DB), (0x1D6FB, 0x1D6FB), (0x1D715, 0x1D715), (0x1D735, 0x1D735),
    (0x1D74F, 0x1D74F), (0x1D76F, 0x1D76F), (0x1D789, 0x1D789), (0x1D7A9, 0x1D7A9),
    (0x1D7C3, 0x1D7C3), (0x1DA87, 0x1DA8B), (0x1E8C7, 0x1E8CF), (0x1E95E, 0x1ECAB),
    (0x1ECAD, 0x1ECAF), (0x1ECB1, 0x1ED2D), (0x1ED2F, 0x1ED3D), (0x1EEF0, 0x1EEF1),
    (0x1F100, 0x1F10C), (0x2EBF0, 0x2EE5D), (0xF0000, 0x10FFFD),
];

/// Non-ASCII characters that cut a URL's path back to its host, or void
/// the URL when they are in the host (`is_poison`).
#[rustfmt::skip]
const URL_POISON: &[(u32, u32)] = &[
    (0x05F3, 0x0605), (0x061C, 0x061C), (0x0660, 0x0669), (0x066B, 0x066B), (0x06DD, 0x06DD),
    (0x06F0, 0x06F9), (0x070F, 0x070F), (0x07C0, 0x07C9), (0x0890, 0x0891), (0x08E2, 0x08E2),
    (0x0966, 0x096F), (0x09E6, 0x09EF), (0x0A66, 0x0A6F), (0x0AE6, 0x0AEF), (0x0B66, 0x0B6F),
    (0x0BE6, 0x0BEF), (0x0C66, 0x0C6F), (0x0CE6, 0x0CEF), (0x0D66, 0x0D6F), (0x0DE6, 0x0DEF),
    (0x0E50, 0x0E59), (0x0ED0, 0x0ED9), (0x0F20, 0x0F29), (0x1040, 0x1049), (0x1090, 0x1099),
    (0x16EE, 0x16F0), (0x17E0, 0x17E9), (0x180E, 0x180E), (0x1810, 0x1819), (0x1946, 0x194F),
    (0x19D0, 0x19DA), (0x1A80, 0x1AA6), (0x1AA8, 0x1AAD), (0x1B50, 0x1B59), (0x1BB0, 0x1BB9),
    (0x1C40, 0x1C49), (0x1C50, 0x1C59), (0x2024, 0x2024), (0x2027, 0x2027), (0x202A, 0x202E),
    (0x2060, 0x206F), (0x2160, 0x2182), (0x2185, 0x2188), (0xA620, 0xA629), (0xA6E6, 0xA6EF),
    (0xA8D0, 0xA8D9), (0xA900, 0xA909), (0xA9D0, 0xA9D9), (0xA9F0, 0xA9F9), (0xAA50, 0xAA59),
    (0xAADE, 0xAADF), (0xABF0, 0xABF9), (0xFE13, 0xFE13), (0xFE33, 0xFE34), (0xFE4D, 0xFE4F),
    (0xFE52, 0xFE52), (0xFEFF, 0xFEFF), (0xFF07, 0xFF07), (0xFF3F, 0xFF3F), (0xFFF9, 0xFFFB),
    (0x10140, 0x10174), (0x10341, 0x10341), (0x1034A, 0x1034A), (0x103D1, 0x103D5),
    (0x104A0, 0x104A9), (0x10D30, 0x10D39), (0x11066, 0x1106F), (0x110BD, 0x110BD),
    (0x110CD, 0x110CD), (0x110F0, 0x110F9), (0x11136, 0x1113F), (0x111D0, 0x111D9),
    (0x112F0, 0x112F9), (0x11450, 0x11459), (0x114D0, 0x114D9), (0x11650, 0x11659),
    (0x116C0, 0x116C9), (0x11730, 0x1173B), (0x118E0, 0x118E9), (0x11950, 0x11959),
    (0x11C50, 0x11C59), (0x11D50, 0x11D59), (0x11DA0, 0x11DA9), (0x11F50, 0x11F59),
    (0x12400, 0x1246E), (0x13430, 0x1343F), (0x16A60, 0x16A69), (0x16AC0, 0x16AC9),
    (0x16B50, 0x16B59), (0x1BCA0, 0x1BCA3), (0x1D173, 0x1D17A), (0x1D7CE, 0x1D7FF),
    (0x1E140, 0x1E149), (0x1E2F0, 0x1E2F9), (0x1E4F0, 0x1E4F9), (0x1E950, 0x1E959),
    (0x1FBF0, 0x1FBF9), (0xE0001, 0xE007F),
];

/// Non-ASCII characters a URL's path may hold and its host may not
/// (`is_host_bad`).
#[rustfmt::skip]
const HOST_BAD: &[(u32, u32)] = &[
    (0x00A8, 0x00A8), (0x00AF, 0x00AF), (0x00B4, 0x00B4), (0x00B8, 0x00B8), (0x02D8, 0x02DD),
    (0x037A, 0x037A), (0x0384, 0x0385), (0x1FBD, 0x1FBD), (0x1FBF, 0x1FC1), (0x1FCD, 0x1FCF),
    (0x1FDD, 0x1FDF), (0x1FED, 0x1FEF), (0x1FFD, 0x1FFE), (0x200E, 0x200F), (0x203E, 0x203E),
    (0x2FF0, 0x2FFB), (0x309B, 0x309C), (0x33C2, 0x33C2), (0x33C7, 0x33C7), (0x33D8, 0x33D8),
    (0xF7F3, 0xF8A0), (0xF8A8, 0xF8B3), (0xF8B8, 0xF8BA), (0xF8C1, 0xF8FF), (0xFC5E, 0xFC63),
    (0xFDFA, 0xFDFB), (0xFE70, 0xFE70), (0xFE72, 0xFE72), (0xFE74, 0xFE76), (0xFE78, 0xFE78),
    (0xFE7A, 0xFE7A), (0xFE7C, 0xFE7C), (0xFE7E, 0xFE7E), (0xFF3E, 0xFF3E), (0xFF40, 0xFF40),
    (0xFFE3, 0xFFE3),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// What a body links to, in order — Swift's `linkURLs(in:)`. The bodies
    /// these tests use hold no markdown, so what is drawn is what was typed
    /// and the detected spans are the bubble's links.
    fn targets(text: &str) -> Vec<String> {
        spans(text).into_iter().map(|(_, target)| target).collect()
    }

    /// The detected spans as (drawn text, target), after checking every
    /// span against the text it came from: ordered, apart, on char
    /// boundaries, holding exactly its own characters, openable.
    fn spans(text: &str) -> Vec<(String, String)> {
        let found = detect(text);
        let mut last_end = 0;
        for span in &found {
            assert!(
                span.range.start >= last_end && span.range.start < span.range.end,
                "{text:?}: {found:?}"
            );
            assert!(
                text.is_char_boundary(span.range.start) && text.is_char_boundary(span.range.end)
            );
            assert_eq!(text[span.range.clone()], span.text);
            assert!(is_openable(&span.target), "{:?} in {text:?}", span.target);
            last_end = span.range.end;
        }
        found
            .into_iter()
            .map(|span| (span.text, span.target))
            .collect()
    }

    fn host(target: &str) -> Option<&str> {
        let rest = &target[target.find("://")? + 3..];
        let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
        let host_port = authority.rsplit('@').next()?;
        Some(host_port.split(':').next().unwrap_or(host_port))
    }

    fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|&(text, target)| (text.to_owned(), target.to_owned()))
            .collect()
    }

    /// A markdown link over `label` in `rendered`, as the markdown port will
    /// hand it over — destination normalized, as Swift's `decorated` does
    /// before the detector runs.
    fn declared(rendered: &str, label: &str, destination: &str) -> LinkSpan {
        let start = rendered.find(label).expect("the label is in the text");
        LinkSpan {
            range: start..start + label.len(),
            text: label.to_owned(),
            target: normalize_destination(destination),
        }
    }

    fn targets_of(spans: &[LinkSpan]) -> Vec<&str> {
        spans.iter().map(|span| span.target.as_str()).collect()
    }

    // MARK: - MessageLinksTests.swift, test for test

    /// "web URLs become tappable links"
    #[test]
    fn web_urls() {
        assert_eq!(
            targets("release notes at https://example.com/notes?v=1 today"),
            ["https://example.com/notes?v=1"]
        );
    }

    /// "schemeless www hosts gain a scheme"
    #[test]
    fn schemeless_web() {
        let urls = targets("see www.example.com");
        assert_eq!(urls.len(), 1);
        assert_eq!(host(&urls[0]), Some("www.example.com"));
        assert!(scheme_of(&urls[0]).is_some_and(|scheme| !scheme.is_empty()));
        // Swift leaves the scheme open; the detector answers http, and so
        // does Android's own test of the same body.
        assert_eq!(urls[0], "http://www.example.com");
    }

    /// "email addresses become mailto links"
    #[test]
    fn emails() {
        assert_eq!(
            targets("write to nettrash@nettrash.me please"),
            ["mailto:nettrash@nettrash.me"]
        );
    }

    /// "phone numbers become sanitized tel links"
    #[test]
    fn phone_numbers() {
        assert_eq!(
            targets("call +1 (555) 123-4567 tonight"),
            ["tel:+15551234567"]
        );
    }

    /// "vanity letters dial as their keypad digits"
    #[test]
    fn vanity_numbers() {
        assert_eq!(targets("call 1-800-GOT-JUNK now"), ["tel:18004685865"]);
    }

    /// "extensions ride separately instead of corrupting the number"
    #[test]
    fn phone_extensions() {
        // The extension is normalized behind a `;`, and naive digit
        // concatenation would dial 555123456789.
        assert_eq!(targets("call 555-123-4567 x89"), ["tel:5551234567;ext=89"]);
    }

    /// "plain text yields no links"
    #[test]
    fn plain_text() {
        assert!(targets("just words, nothing else. really").is_empty());
        assert!(targets("😀😀").is_empty());
        assert!(targets("").is_empty());
    }

    /// "multiple matches keep their order"
    #[test]
    fn multiple_matches() {
        let urls = targets("docs: https://example.com and mail nettrash@nettrash.me");
        let schemes: Vec<&str> = urls.iter().filter_map(|url| scheme_of(url)).collect();
        assert_eq!(schemes, ["https", "mailto"]);
    }

    /// "link runs are underlined; own bubbles force white" — the half that
    /// is detection: exactly one link run, over exactly the URL. Detection
    /// has no side, so it is the same run in your bubble and theirs.
    #[test]
    fn link_styling() {
        let found = detect("see https://example.com");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].range, 4..23);
        assert_eq!(found[0].text, "https://example.com");
    }

    /// "non-link text keeps no link attribute" — the half that is
    /// detection: the words either side are outside the span.
    #[test]
    fn surrounding_text_unaffected() {
        let text = "before https://example.com after";
        let found = detect(text);
        assert_eq!(found.len(), 1);
        assert_eq!(text[found[0].range.clone()], *"https://example.com");
        assert_eq!(text[..found[0].range.start], *"before ");
        assert_eq!(text[found[0].range.end..], *" after");
    }

    /// The bodies of Swift's `drawMarkComesFromTheRawBody`, with the marks
    /// it expects.
    const DRAW_MARK_BODIES: &[(&str, &[&str])] = &[
        ("**/draw** a cat", &[]),
        ("*/draw* a cat", &[]),
        ("`/draw` a cat", &[]),
        ("~~/draw~~ a cat", &[]),
        ("# /draw a cat", &[]),
        ("- /draw a cat", &[]),
        ("/draw a cat", &["/draw"]),
        ("/draw a **fluffy** cat", &["/draw"]),
        ("@ai /draw a cat", &["@ai", "/draw"]),
    ];

    /// The half of `drawMarkComesFromTheRawBody` that IS this file's: the
    /// assistant's tokens are never links. Swift applies the mark as a
    /// style only, which is safe because nothing here claims `@ai` or
    /// `/draw` for a tap.
    #[test]
    fn draw_mark_bodies_hold_no_links() {
        for (body, _) in DRAW_MARK_BODIES {
            assert!(detect(body).is_empty(), "{body:?}");
        }
    }

    /// The links half of `onlyTheFirstBlockCarriesThePictureMark`: nothing
    /// in that body, whole or block by block, is a link.
    #[test]
    fn table_split_body_holds_no_links() {
        for text in [
            "| a |\n| --- |\n| 1 |\n/draw a cat",
            "/draw a cat",
            "a",
            "1",
        ] {
            assert!(detect(text).is_empty(), "{text:?}");
        }
    }

    // MARK: - MessageMarkdownTests.swift, where it pins MessageLinks' rules

    /// `detectedLinkOffsetsSurviveMarkdown`: detected over the RENDERED
    /// text, a link lands on the glyphs the bubble draws.
    #[test]
    fn detected_link_offsets_survive_markdown() {
        // `**important** go to https://example.com now`, as it is drawn.
        let rendered = "important go to https://example.com now";
        let found = detect(rendered);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "https://example.com");
        assert_eq!(found[0].range, 16..35);
    }

    /// `labelThatLooksLikeAURL`: the phishing shape keeps the author's
    /// destination, and only that one.
    #[test]
    fn label_that_looks_like_a_url() {
        // `[https://www.paypal.com](https://evil.example)`, as it is drawn.
        let rendered = "https://www.paypal.com";
        let links = merge(
            vec![declared(rendered, rendered, "https://evil.example")],
            detect(rendered),
        );
        assert_eq!(targets_of(&links), ["https://evil.example"]);
    }

    /// `schemelessDestination`: `[here](example.com/x)` opens https.
    #[test]
    fn schemeless_destination() {
        let links = merge(
            vec![declared("here", "here", "example.com/x")],
            detect("here"),
        );
        assert_eq!(targets_of(&links), ["https://example.com/x"]);
    }

    /// `previewUsesRenderedText`: the card describes the link the bubble
    /// draws — the first https one, after the markdown rules.
    #[test]
    fn preview_uses_rendered_text() {
        let card = |rendered: &str, declared: Vec<LinkSpan>| {
            first_web_link(&merge(declared, detect(rendered))).map(|span| span.target.clone())
        };
        // `**https://example.com**`: the asterisks are not drawn.
        assert_eq!(
            card("https://example.com", vec![]).as_deref(),
            Some("https://example.com")
        );
        // `see [the menu](https://example.com/menu)`: no URL in the glyphs.
        let menu = "see the menu";
        assert_eq!(
            card(
                menu,
                vec![declared(menu, "the menu", "https://example.com/menu")]
            )
            .as_deref(),
            Some("https://example.com/menu")
        );
        // `[here](example.com/x)`: normalized on the way, as for the tap.
        assert_eq!(
            card("here", vec![declared("here", "here", "example.com/x")]).as_deref(),
            Some("https://example.com/x")
        );
        // The phishing shape: where the tap GOES.
        let label = "https://www.paypal.com";
        assert_eq!(
            card(label, vec![declared(label, label, "https://evil.example")]).as_deref(),
            Some("https://evil.example")
        );
        // Plain http previews nowhere.
        assert_eq!(card("see http://example.com", vec![]), None);
        assert_eq!(card("no links here at all", vec![]), None);
    }

    /// `contractTableCellURLStillPreviews`: Swift previews over the flat
    /// render, where a table is the rows as typed.
    #[test]
    fn contract_table_cell_url_still_previews() {
        let flat = "| what | where |\n| --- | --- |\n| site | https://example.com/a |";
        let found = detect(flat);
        assert_eq!(
            first_web_link(&found).map(|span| span.target.as_str()),
            Some("https://example.com/a")
        );
    }

    /// `blocksKeepTheirOwnOffsets`, the links half: each block is its own
    /// offset space, and the block after a table is detected on its own.
    #[test]
    fn blocks_keep_their_own_offsets() {
        let last = "then see https://example.com now";
        assert_eq!(
            spans(last),
            owned(&[("https://example.com", "https://example.com")])
        );
        assert_eq!(detect(last)[0].range, 9..28);
        assert!(detect("ask @ai").is_empty());
    }

    // MARK: - Android's MessageLinksTest, where it pins the same rules

    #[test]
    fn android_web_urls_become_tappable_links() {
        let text = "release notes at https://example.com/notes?v=1 today";
        assert_eq!(
            spans(text),
            owned(&[(
                "https://example.com/notes?v=1",
                "https://example.com/notes?v=1"
            )])
        );
    }

    #[test]
    fn android_schemeless_www_hosts_gain_a_scheme() {
        assert_eq!(targets("see www.example.com"), ["http://www.example.com"]);
    }

    #[test]
    fn android_phone_numbers_become_tel_links() {
        let urls = targets("call +1 555-123-4567 tonight");
        assert_eq!(urls.len(), 1);
        assert!(urls[0].starts_with("tel:"));
        assert_eq!(
            urls[0]
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>(),
            "15551234567"
        );
    }

    #[test]
    fn android_merge_spans_keeps_detected_links_outside_markdown_ones() {
        // `[menu](https://a.example) and https://b.example`, as it is drawn.
        let rendered = "menu and https://b.example";
        let links = merge(
            vec![declared(rendered, "menu", "https://a.example")],
            detect(rendered),
        );
        assert_eq!(
            targets_of(&links),
            ["https://a.example", "https://b.example"]
        );
    }

    #[test]
    fn android_a_body_with_no_web_link_has_nothing_to_preview() {
        let found = detect("call 555-1234");
        assert_eq!(found.len(), 1);
        assert!(first_web_link(&found).is_none());
    }

    // MARK: - Apple, measured

    /// NSDataDetector's own answers — each run through MessageLinks' code on
    /// macOS 26 — which this file gives too. A host Apple spells in punycode
    /// is compared by the host a browser resolves (see the module docs).
    #[rustfmt::skip]
    const APPLE: &[(&str, &[(&str, &str)])] = &[
        // Web links with a scheme, and where they end.
        ("release notes at https://example.com/notes?v=1 today", &[("https://example.com/notes?v=1", "https://example.com/notes?v=1")]),
        ("https://example.com/a-b_c~d", &[("https://example.com/a-b_c~d", "https://example.com/a-b_c~d")]),
        ("https://example.com/?q=hello+world&lang=en", &[("https://example.com/?q=hello+world&lang=en", "https://example.com/?q=hello+world&lang=en")]),
        ("https://example.com/#section-2", &[("https://example.com/#section-2", "https://example.com/#section-2")]),
        ("https://user:pw@example.com:8443/admin", &[("https://user:pw@example.com:8443/admin", "https://user:pw@example.com:8443/admin")]),
        ("http://localhost:3000", &[("http://localhost:3000", "http://localhost:3000")]),
        ("https://example.com:8080", &[("https://example.com:8080", "https://example.com:8080")]),
        ("HTTPS://EXAMPLE.COM/UP", &[("HTTPS://EXAMPLE.COM/UP", "HTTPS://EXAMPLE.COM/UP")]),
        ("https://github.com/nettrash/family.connect/pull/65#issuecomment-123", &[("https://github.com/nettrash/family.connect/pull/65#issuecomment-123", "https://github.com/nettrash/family.connect/pull/65#issuecomment-123")]),
        ("https://[::1]:8080/", &[("https://[::1]:8080/", "https://[::1]:8080/")]),
        ("https://example.com.", &[("https://example.com", "https://example.com")]),
        ("https://example.com,", &[("https://example.com", "https://example.com")]),
        ("https://example.com!", &[("https://example.com", "https://example.com")]),
        ("https://example.com;", &[("https://example.com", "https://example.com")]),
        ("https://example.com/a...", &[("https://example.com/a", "https://example.com/a")]),
        ("https://example.com?", &[("https://example.com?", "https://example.com?")]),
        ("https://example.com:", &[("https://example.com:", "https://example.com:")]),
        ("https://example.com/a'", &[("https://example.com/a'", "https://example.com/a'")]),
        ("https://example.com/a#", &[("https://example.com/a#", "https://example.com/a#")]),
        ("https://example.com/a?!", &[("https://example.com/a?", "https://example.com/a?")]),
        ("https://example.com/a.)", &[("https://example.com/a", "https://example.com/a")]),
        ("https://example.com/a!!", &[("https://example.com/a", "https://example.com/a")]),
        ("https://example.com/a*", &[("https://example.com/a*", "https://example.com/a*")]),
        ("https://example.com/a-", &[("https://example.com/a-", "https://example.com/a-")]),
        ("https://example.com/a~", &[("https://example.com/a~", "https://example.com/a~")]),
        ("is it https://example.com?", &[("https://example.com?", "https://example.com?")]),
        ("see https://example.com/a, ok", &[("https://example.com/a", "https://example.com/a")]),
        ("https://example.com/a b", &[("https://example.com/a", "https://example.com/a")]),
        // Brackets and quotes.
        ("(https://example.com)", &[("https://example.com", "https://example.com")]),
        ("(see https://example.com/a)", &[("https://example.com/a", "https://example.com/a")]),
        ("https://en.wikipedia.org/wiki/Rust_(programming_language)", &[("https://en.wikipedia.org/wiki/Rust_(programming_language)", "https://en.wikipedia.org/wiki/Rust_(programming_language)")]),
        ("(https://example.com/a_(b)).", &[("https://example.com/a_(b))", "https://example.com/a_(b))")]),
        ("https://example.com/a(b", &[("https://example.com/a", "https://example.com/a")]),
        ("https://example.com/a((b)", &[("https://example.com/a((b)", "https://example.com/a((b)")]),
        ("https://example.com/a(b)c(d", &[("https://example.com/a(b)c", "https://example.com/a(b)c")]),
        ("https://example.com/a)b", &[("https://example.com/a", "https://example.com/a")]),
        ("https://example.com/(a", &[("https://example.com/", "https://example.com/")]),
        ("https://example.com/a{b}c}", &[("https://example.com/a{b}c}", "https://example.com/a%7Bb%7Dc%7D")]),
        ("https://example.com/a<b>", &[("https://example.com/a<b>", "https://example.com/a%3Cb%3E")]),
        ("<https://example.com>", &[("https://example.com", "https://example.com")]),
        ("{https://example.com}", &[("https://example.com", "https://example.com")]),
        ("'https://example.com'", &[("https://example.com", "https://example.com")]),
        ("'https://example.com/a'.", &[("https://example.com/a", "https://example.com/a")]),
        ("''https://example.com/a''", &[("https://example.com/a'", "https://example.com/a'")]),
        ("[https://example.com/a]", &[("https://example.com/a", "https://example.com/a")]),
        ("[https://example.com]", &[("https://example.com", "https://example.com")]),
        ("[see https://example.com/a]", &[("https://example.com/a]", "https://example.com/a%5D")]),
        ("\"https://example.com\"", &[("https://example.com", "https://example.com")]),
        ("https://example.com/a\"b\"", &[("https://example.com/a", "https://example.com/a")]),
        ("“https://example.com/a”", &[("https://example.com/a", "https://example.com/a")]),
        ("https://example.com/a“b”", &[("https://example.com/a“b", "https://example.com/a%E2%80%9Cb")]),
        ("https://example.com/it's", &[("https://example.com/it's", "https://example.com/it's")]),
        // Schemes Apple reads, and where it does not see one.
        ("https:// example.com", &[("example.com", "http://example.com")]),
        ("https:example.com", &[("example.com", "http://example.com")]),
        ("1http://x.com", &[("x.com", "http://x.com")]),
        ("смотриhttps://example.com", &[("example.com", "http://example.com")]),
        ("日本語https://example.com", &[("https://example.com", "https://example.com")]),
        ("x.https://example.com", &[("https://example.com", "https://example.com")]),
        ("+https://example.com", &[("https://example.com", "https://example.com")]),
        ("https://ex ample.com", &[("https://ex", "https://ex"), ("ample.com", "http://ample.com")]),
        // Refused URLs link nothing inside them.
        ("https://example.com:abc", &[]),
        ("https://example.com]", &[]),
        ("[see https://example.com]", &[]),
        ("https://example.com%", &[]),
        ("https://example.com|", &[]),
        ("https://example.com::80", &[]),
        ("https://example.com:8080:90", &[]),
        ("https://", &[]),
        ("https://.", &[]),
        // Percent-encoding matches Apple's byte for byte.
        ("https://example.com/a|b", &[("https://example.com/a|b", "https://example.com/a%7Cb")]),
        ("https://example.com/a\\b", &[("https://example.com/a\\b", "https://example.com/a%5Cb")]),
        ("https://example.com/a^b", &[("https://example.com/a^b", "https://example.com/a%5Eb")]),
        ("https://example.com/a`b", &[("https://example.com/a`b", "https://example.com/a%60b")]),
        ("https://example.com/[a]", &[("https://example.com/[a]", "https://example.com/%5Ba%5D")]),
        ("https://example.com/{}", &[("https://example.com/{}", "https://example.com/%7B%7D")]),
        ("https://example.com/a#b#c", &[("https://example.com/a#b#c", "https://example.com/a#b%23c")]),
        ("https://example.com/%", &[("https://example.com/%", "https://example.com/%25")]),
        ("https://example.com/%zz", &[("https://example.com/%zz", "https://example.com/%25zz")]),
        ("https://example.com/a%2", &[("https://example.com/a%2", "https://example.com/a%252")]),
        ("https://example.com/a%20b", &[("https://example.com/a%20b", "https://example.com/a%20b")]),
        ("https://example.com/путь?q=да", &[("https://example.com/путь?q=да", "https://example.com/%D0%BF%D1%83%D1%82%D1%8C?q=%D0%B4%D0%B0")]),
        ("https://example.com/日本語", &[("https://example.com/日本語", "https://example.com/%E6%97%A5%E6%9C%AC%E8%AA%9E")]),
        ("https://example.com/😀", &[("https://example.com/😀", "https://example.com/%F0%9F%98%80")]),
        ("https://a@b@example.com/", &[("https://a@b@example.com/", "https://a%40b@example.com/")]),
        // Hosts without a scheme.
        ("see www.example.com", &[("www.example.com", "http://www.example.com")]),
        ("example.com", &[("example.com", "http://example.com")]),
        ("example.co.uk", &[("example.co.uk", "http://example.co.uk")]),
        ("nettrash.me", &[("nettrash.me", "http://nettrash.me")]),
        ("t.me/family", &[("t.me/family", "http://t.me/family")]),
        ("vk.com", &[("vk.com", "http://vk.com")]),
        ("GOOGLE.COM", &[("GOOGLE.COM", "http://GOOGLE.COM")]),
        ("example.Net", &[("example.Net", "http://example.Net")]),
        ("example.xyz", &[("example.xyz", "http://example.xyz")]),
        ("www.example.zz", &[("www.example.zz", "http://www.example.zz")]),
        ("www.example.c", &[("www.example.c", "http://www.example.c")]),
        ("WwW.example.zz", &[("WwW.example.zz", "http://WwW.example.zz")]),
        ("www.www.example.zz", &[("www.www.example.zz", "http://www.www.example.zz")]),
        ("sub.example.com/path?x=1#frag", &[("sub.example.com/path?x=1#frag", "http://sub.example.com/path?x=1#frag")]),
        ("example.com:8080/path", &[("example.com:8080/path", "http://example.com:8080/path")]),
        ("example.com:80a", &[("example.com", "http://example.com")]),
        ("example.com:0", &[("example.com", "http://example.com")]),
        ("example.com?q=1", &[("example.com", "http://example.com")]),
        ("example.com/?q=1", &[("example.com/?q=1", "http://example.com/?q=1")]),
        ("example.com.", &[("example.com", "http://example.com")]),
        ("see google.com.", &[("google.com", "http://google.com")]),
        ("go to example.com, then", &[("example.com", "http://example.com")]),
        ("example.com's page", &[("example.com", "http://example.com")]),
        ("(www.example.com)", &[("www.example.com", "http://www.example.com")]),
        ("my_site.com", &[("my_site.com", "http://my_site.com")]),
        ("my-site.com", &[("my-site.com", "http://my-site.com")]),
        ("-site.com", &[("site.com", "http://site.com")]),
        ("a--b.com", &[("b.com", "http://b.com")]),
        ("xn--p1ai.com", &[("xn--p1ai.com", "http://xn--p1ai.com")]),
        ("1.2.3.4.com", &[("1.2.3.4.com", "http://1.2.3.4.com")]),
        ("en.wikipedia.org/wiki/Foo_(bar)", &[("en.wikipedia.org/wiki/Foo_", "http://en.wikipedia.org/wiki/Foo_")]),
        ("example.com/a(b)c", &[("example.com/a", "http://example.com/a")]),
        ("example.com/a]", &[("example.com/a", "http://example.com/a")]),
        ("example.com/a[b]c", &[("example.com/a[b]c", "http://example.com/a%5Bb%5Dc")]),
        ("'example.com/a'", &[("example.com/a", "http://example.com/a")]),
        ("example.com/путь.", &[("example.com/путь", "http://example.com/%D0%BF%D1%83%D1%82%D1%8C")]),
        ("www.example.com日本語", &[("www.example.com", "http://www.example.com")]),
        ("日本語www.example.com日本語", &[("www.example.com", "http://www.example.com")]),
        ("www.例子.com日本語", &[("www.例子.com", "http://www.例子.com")]),  // Apple spells the host in punycode
        ("example.рф日本", &[("example.рф", "http://example.рф")]),  // Apple spells the host in punycode
        // Not hosts.
        ("example.dev", &[]),
        ("example.app", &[]),
        ("readme.md", &[]),
        ("file.txt", &[]),
        ("hello.world", &[]),
        ("e.g. this", &[]),
        ("Mr.Smith", &[]),
        ("v1.2.3", &[]),
        ("3.14159", &[]),
        ("192.168.1.1", &[]),
        ("example.IO", &[]),
        ("EXAMPLE.RU", &[]),
        ("www.example", &[]),
        ("www2.example.zz", &[]),
        ("www.a.b1", &[]),
        ("example.com_", &[]),
        ("example.comx", &[]),
        ("site-.com", &[]),
        ("a.b", &[]),
        ("just words, nothing else. really", &[]),
        ("hello@", &[]),
        ("@example.com", &[]),
        // Email addresses.
        ("write to nettrash@nettrash.me please", &[("nettrash@nettrash.me", "mailto:nettrash@nettrash.me")]),
        ("first.last+tag@example.co.uk", &[("first.last+tag@example.co.uk", "mailto:first.last+tag@example.co.uk")]),
        ("UPPER@EXAMPLE.COM", &[("UPPER@EXAMPLE.COM", "mailto:UPPER@EXAMPLE.COM")]),
        ("user@example.zz", &[("user@example.zz", "mailto:user@example.zz")]),
        ("user@a.b.c", &[("user@a.b.c", "mailto:user@a.b.c")]),
        ("user@192.168.1.1", &[("user@192.168.1.1", "mailto:user@192.168.1.1")]),
        ("user@1.2.3", &[]),
        ("user@300.2.3.4", &[]),
        ("user@example.c0m", &[]),
        ("user@example.com5", &[]),
        ("user@example.com-x", &[("user@example.com", "mailto:user@example.com")]),
        ("user@example.com.x", &[("user@example.com.x", "mailto:user@example.com.x")]),
        ("user@example.com_", &[]),
        ("(user@example.com)", &[("user@example.com", "mailto:user@example.com")]),
        ("<user@example.com>", &[("user@example.com", "mailto:user@example.com")]),
        ("user@example.com.", &[("user@example.com", "mailto:user@example.com")]),
        ("user@example.com's", &[("user@example.com", "mailto:user@example.com")]),
        ("user@example.com/path", &[("user@example.com", "mailto:user@example.com")]),
        (".a@example.com", &[("a@example.com", "mailto:a@example.com")]),
        ("+user@example.com", &[("user@example.com", "mailto:user@example.com")]),
        ("_user@example.com", &[("_user@example.com", "mailto:_user@example.com")]),
        ("#user@example.com", &[("#user@example.com", "mailto:%23user@example.com")]),
        ("?user@example.com", &[("?user@example.com", "mailto:?user@example.com")]),
        ("a@b@example.com", &[("b@example.com", "mailto:b@example.com")]),
        ("a b@example.com", &[("b@example.com", "mailto:b@example.com")]),
        ("user.@example.com", &[("example.com", "http://example.com")]),
        ("user..@example.com", &[("example.com", "http://example.com")]),
        ("user@-example.com", &[("example.com", "http://example.com")]),
        ("user@e--x.com", &[("x.com", "http://x.com")]),
        ("a..b@example.com", &[("a..b@example.com", "mailto:a..b@example.com")]),
        ("user%x@example.com", &[("user%x@example.com", "mailto:user%25x@example.com")]),
        ("user#x@example.com", &[("user#x@example.com", "mailto:user%23x@example.com")]),
        ("user{x}@example.com", &[("user{x}@example.com", "mailto:user%7Bx%7D@example.com")]),
        ("user|x@example.com", &[("user|x@example.com", "mailto:user%7Cx@example.com")]),
        ("user'x@example.com", &[("user'x@example.com", "mailto:user'x@example.com")]),
        ("john.me@example.com", &[("john.me@example.com", "mailto:john.me@example.com")]),
        ("nettrash@nettrash.meпока", &[]),
        ("user@example.com日本", &[("user@example.com", "mailto:user@example.com")]),
        ("user@example.com²", &[("user@example.com", "mailto:user@example.com")]),
        ("mailto:x@y.com", &[("mailto:x@y.com", "mailto:x@y.com")]),
        ("MAILTO:x@y.com", &[("MAILTO:x@y.com", "MAILTO:x@y.com")]),
        ("mailto:x@y.com?subject=hi&body=there", &[("mailto:x@y.com?subject=hi&body=there", "mailto:x@y.com?subject=hi&body=there")]),
        ("mailto:x@y.com?subject=hi.", &[("mailto:x@y.com?subject=hi", "mailto:x@y.com?subject=hi")]),
        ("mailto:x@y.com,z@w.com", &[("mailto:x@y.com", "mailto:x@y.com"), ("z@w.com", "mailto:z@w.com")]),
        ("xmailto:x@y.com", &[("x@y.com", "mailto:x@y.com")]),
        ("mailto:x.@y.com", &[("y.com", "http://y.com")]),
        ("mailto:@y.com", &[]),
        ("e-mail: user@example.com", &[("user@example.com", "mailto:user@example.com")]),
        ("почта:nettrash@nettrash.me", &[("nettrash@nettrash.me", "mailto:nettrash@nettrash.me")]),
        // Phone numbers.
        ("call +1 (555) 123-4567 tonight", &[("+1 (555) 123-4567", "tel:+15551234567")]),
        ("call 1-800-GOT-JUNK now", &[("1-800-GOT-JUNK", "tel:18004685865")]),
        ("call 555-123-4567 x89", &[("555-123-4567 x89", "tel:5551234567;ext=89")]),
        ("555-1234", &[("555-1234", "tel:5551234")]),
        ("555 1234", &[("555 1234", "tel:5551234")]),
        ("555.1234", &[("555.1234", "tel:5551234")]),
        ("(555) 123-4567", &[("(555) 123-4567", "tel:5551234567")]),
        ("555.123.4567", &[("555.123.4567", "tel:5551234567")]),
        ("5551234567", &[("5551234567", "tel:5551234567")]),
        ("+15551234567", &[("+15551234567", "tel:+15551234567")]),
        ("1-555-123-4567", &[("1-555-123-4567", "tel:15551234567")]),
        ("1 (555) 123-4567", &[("1 (555) 123-4567", "tel:15551234567")]),
        ("+1.555.123.4567", &[("+1.555.123.4567", "tel:+15551234567")]),
        ("+44 20 7946 0958", &[("+44 20 7946 0958", "tel:+442079460958")]),
        ("+44 (0)20 7946 0958", &[("+44 (0)20 7946 0958", "tel:+442079460958")]),
        ("020 7946 0958", &[("020 7946 0958", "tel:02079460958")]),
        ("07700 900123", &[("07700 900123", "tel:07700900123")]),
        ("+7 (495) 123-45-67", &[("+7 (495) 123-45-67", "tel:+74951234567")]),
        ("8 (800) 555-35-35", &[("8 (800) 555-35-35", "tel:88005553535")]),
        ("+381 64 123 456", &[("+381 64 123 456", "tel:+38164123456")]),
        ("064 123 456", &[("064 123 456", "tel:064123456")]),
        ("064/123-456", &[("064/123-456", "tel:064123456")]),
        ("030/123456", &[("030/123456", "tel:030123456")]),
        ("+33 1 23 45 67 89", &[("+33 1 23 45 67 89", "tel:+33123456789")]),
        ("01 23 45 67 89", &[("01 23 45 67 89", "tel:0123456789")]),
        ("+86 10 1234 5678", &[("+86 10 1234 5678", "tel:+861012345678")]),
        ("+81 3-1234-5678", &[("+81 3-1234-5678", "tel:+81312345678")]),
        ("03-1234-5678", &[("03-1234-5678", "tel:0312345678")]),
        ("+972-50-123-4567", &[("+972-50-123-4567", "tel:+972501234567")]),
        ("+91 98765 43210", &[("+91 98765 43210", "tel:+919876543210")]),
        ("+55 11 91234-5678", &[("+55 11 91234-5678", "tel:+5511912345678")]),
        ("123-45-67", &[("123-45-67", "tel:1234567")]),
        ("55-51-234", &[("55-51-234", "tel:5551234")]),
        ("12 34 56 78", &[("12 34 56 78", "tel:12345678")]),
        ("1234-5678", &[("1234-5678", "tel:12345678")]),
        ("123-45-6789", &[("123-45-6789", "tel:123456789")]),
        ("１２３-４５６７", &[("１２３-４５６７", "tel:1234567")]),
        ("５５５-１２３-４５６７", &[("５５５-１２３-４５６７", "tel:5551234567")]),
        ("+1\u{A0}555\u{A0}123\u{A0}4567", &[("+1\u{A0}555\u{A0}123\u{A0}4567", "tel:+15551234567")]),
        ("555–123–4567", &[("555–123–4567", "tel:5551234567")]),
        ("Звони +7 (495) 123-45-67!", &[("+7 (495) 123-45-67", "tel:+74951234567")]),
        ("电话：+86 10 1234 5678", &[("+86 10 1234 5678", "tel:+861012345678")]),
        ("tel: +44 7700 900123", &[("+44 7700 900123", "tel:+447700900123")]),
        ("Phone: (020) 7946-0958", &[("(020) 7946-0958", "tel:02079460958")]),
        ("555-123-4567 ext. 89", &[("555-123-4567 ext. 89", "tel:5551234567;ext=89")]),
        ("555-123-4567 extension 89", &[("555-123-4567 extension 89", "tel:5551234567;ext=89")]),
        ("555-123-4567x89", &[("555-123-4567x89", "tel:5551234567;ext=89")]),
        ("+1 (555) 123-4567 ext 12", &[("+1 (555) 123-4567 ext 12", "tel:+15551234567;ext=12")]),
        ("555-123-4567 ;89", &[("555-123-4567 ;89", "tel:5551234567;ext=89")]),
        ("555-123-4567;ext=89", &[("555-123-4567;ext=89", "tel:5551234567;ext=89")]),
        ("555-123-4567, ext 89", &[("555-123-4567, ext 89", "tel:5551234567;ext=89")]),
        ("555-123-4567 (ext 89)", &[("555-123-4567 (ext 89)", "tel:5551234567;ext=89")]),
        ("555-123-4567 x1", &[("555-123-4567", "tel:5551234567")]),
        ("555-123-4567 x", &[("555-123-4567", "tel:5551234567")]),
        ("555-123-4567 x89x", &[("555-123-4567", "tel:5551234567")]),
        ("555 1234 ext 5", &[("555 1234 ext 5", "tel:5551234;ext=5")]),
        ("1-800-FLOWERS", &[("1-800-FLOWERS", "tel:18003569377")]),
        ("+1-800-FLOWERS", &[("+1-800-FLOWERS", "tel:+18003569377")]),
        ("(800) GOT-JUNK", &[("(800) GOT-JUNK", "tel:8004685865")]),
        ("1 800 GOT JUNK", &[("1 800 GOT JUNK", "tel:18004685865")]),
        ("1800FLOWERS", &[("1800FLOWERS", "tel:18003569377")]),
        ("1-877-KARS-4-KIDS", &[("1-877-KARS-4-KIDS", "tel:1877527745437")]),
        ("1-800-G0T-JUNK", &[("1-800-G0T-JUNK", "tel:18004085865")]),
        ("1-800-GOT-JUNK AND MORE", &[("1-800-GOT-JUNK", "tel:18004685865")]),
        ("1-800-FLOWERS-NOW", &[]),
        ("212-GOT-JUNK", &[]),
        ("1-800-flowers", &[]),
        ("1-800-FLOWER", &[]),
        ("call 911", &[("911", "tel:911")]),
        ("Call 112", &[("112", "tel:112")]),
        ("CALL 999", &[("999", "tel:999")]),
        ("dial 911", &[("911", "tel:911")]),
        ("call:911", &[("911", "tel:911")]),
        ("call 1234.", &[("1234", "tel:1234")]),
        ("recall 911", &[]),
        ("phone 911", &[]),
        ("call 1234567", &[]),
        ("tel:+15551234567", &[("tel:+15551234567", "tel:+15551234567")]),
        ("tel:555-123-4567", &[("tel:555-123-4567", "tel:555-123-4567")]),
        ("tel:+15551234567.", &[("tel:+15551234567", "tel:+15551234567")]),
        ("tel:5551234567x89", &[("tel:5551234567x89", "tel:5551234567x89")]),
        ("tel:+7916123-45-67", &[("tel:+7916123-45-67", "tel:+7916123-45-67")]),
        ("tel:+1 555 123 4567", &[("+1 555 123 4567", "tel:+15551234567")]),
        ("TEL:+15551234567", &[("+15551234567", "tel:+15551234567")]),
        ("xtel:+15551234567", &[("+15551234567", "tel:+15551234567")]),
        ("555-123-4567 555-765-4321", &[("555-123-4567", "tel:5551234567"), ("555-765-4321", "tel:5557654321")]),
        ("5551234567 5557654321", &[("5551234567", "tel:5551234567"), ("5557654321", "tel:5557654321")]),
        ("call 555-1234 or 555-4321", &[("555-1234", "tel:5551234"), ("555-4321", "tel:5554321")]),
        ("$555-123-4567", &[("123-4567", "tel:1234567")]),
        ("555)123-4567", &[("123-4567", "tel:1234567")]),
        ("1 234 5678", &[("234 5678", "tel:2345678")]),
        ("#1 555-123-4567", &[("1 555-123-4567", "tel:15551234567")]),
        ("#31#555-123-4567", &[("555-123-4567", "tel:5551234567")]),
        ("555-123-4567/555-765-4321", &[("555-123-4567", "tel:5551234567")]),
        ("No.555-123-4567", &[("555-123-4567", "tel:5551234567")]),
        ("тел555-123-4567", &[("555-123-4567", "tel:5551234567")]),
        ("日本555-123-4567", &[("555-123-4567", "tel:5551234567")]),
        ("555-123-4567日本", &[("555-123-4567", "tel:5551234567")]),
        ("555-123-4567👍", &[("555-123-4567", "tel:5551234567")]),
        // Not phone numbers.
        ("112", &[]),
        ("12345", &[]),
        ("1234567", &[]),
        ("12345678", &[]),
        ("123456789012", &[]),
        ("2026-09-10", &[]),
        ("10/09/2026", &[]),
        ("09.10.2026", &[]),
        ("2026.09.10", &[]),
        ("12:30:45", &[]),
        ("1,000,000", &[]),
        ("1.000.000", &[]),
        ("3.141592653", &[]),
        ("1 000 000", &[]),
        ("100 000", &[]),
        ("$1,234.56", &[]),
        ("1-2-3", &[]),
        ("12-34", &[]),
        ("123-45", &[]),
        ("1234 567", &[]),
        ("555 12 34", &[]),
        ("12 34 56", &[]),
        ("1990-2000", &[]),
        ("12345-6789", &[]),
        ("978-0-306-40615-7", &[]),
        ("10.0.0.1", &[]),
        ("v2.0.1", &[]),
        ("a555-123-4567", &[]),
        ("555-123-4567a", &[]),
        ("/555-123-4567", &[]),
        ("555-123-4567-", &[]),
        ("555-123-4567%", &[]),
        ("555-123-4567#", &[]),
        ("555-123-4567руб", &[]),
        ("555-123-4567x", &[]),
        ("555/1234", &[]),
        ("555 - 1234", &[]),
        ("4111-1111-1111-1111", &[]),
        ("1234 5678 9012 3456 7890", &[]),
        ("+1234567", &[]),
        ("+12345678901234", &[]),
        ("٥٥٥-١٢٣-٤٥٦٧", &[]),
        ("tracking 1Z999AA10123456784", &[]),
        ("9am-5pm", &[]),
        ("pages 100-200", &[]),
        // Right-to-left text and bidi controls.
        ("שלום \u{202B}https://example.com/path\u{202B} שלום", &[("https://example.com/path", "https://example.com/path")]),
        ("https://example.com\u{202A}/x", &[("https://example.com", "https://example.com")]),
        ("https://example.com\u{202A}x", &[]),
        ("\u{2067}example.com\u{2067}", &[("example.com", "http://example.com")]),
        ("\u{2066}user@example.com\u{2066}", &[("user@example.com", "mailto:user@example.com")]),
        ("\u{202B}+1 555 123 4567\u{202B}", &[("+1 555 123 4567", "tel:+15551234567")]),
        ("see https://example.com\u{200E} ok", &[]),
        ("https://example.com/a\u{200E}", &[("https://example.com/a\u{200E}", "https://example.com/a%E2%80%8E")]),
        ("\u{200E}example.com\u{200E}", &[]),
        ("https://example.com/a\u{2060}b", &[("https://example.com", "https://example.com")]),
    ];

    #[test]
    fn matches_apple() {
        for (text, want) in APPLE {
            assert_eq!(spans(text), owned(want), "{text:?}");
        }
    }

    /// Cyrillic, CJK, Hangul, RTL, Indic and Thai text, emoji sequences,
    /// flags, internationalised hosts and non-ASCII punctuation right up
    /// against a link — Apple's answers again, every one.
    #[rustfmt::skip]
    const UNICODE: &[(&str, &[(&str, &str)])] = &[
        // Unicode around links.
        ("Привет, смотри https://example.com/путь — круто", &[("https://example.com/путь", "https://example.com/%D0%BF%D1%83%D1%82%D1%8C")]),
        ("Смотри: https://example.com, звони +7 (495) 123-45-67 или пиши nettrash@nettrash.me.", &[("https://example.com", "https://example.com"), ("+7 (495) 123-45-67", "tel:+74951234567"), ("nettrash@nettrash.me", "mailto:nettrash@nettrash.me")]),
        ("请看https://example.com。谢谢", &[("https://example.com", "https://example.com")]),
        ("网站example.com很好", &[("example.com", "http://example.com")]),
        ("サイトはhttps://example.com/テスト。", &[("https://example.com/テスト", "https://example.com/%E3%83%86%E3%82%B9%E3%83%88")]),
        ("리뷰 https://example.com/한국 보세요", &[("https://example.com/한국", "https://example.com/%ED%95%9C%EA%B5%AD")]),
        ("👨\u{200D}👩\u{200D}👧 https://example.com 👨\u{200D}👩\u{200D}👧", &[("https://example.com", "https://example.com")]),
        ("🇷🇸🇬🇧 nettrash@nettrash.me 🇷🇺", &[("nettrash@nettrash.me", "mailto:nettrash@nettrash.me")]),
        ("https://example.com/👨\u{200D}👩\u{200D}👧x", &[("https://example.com/👨\u{200D}👩\u{200D}👧x", "https://example.com/%F0%9F%91%A8%E2%80%8D%F0%9F%91%A9%E2%80%8D%F0%9F%91%A7x")]),
        ("https://example.com/a🇬🇧", &[("https://example.com/a🇬🇧", "https://example.com/a%F0%9F%87%AC%F0%9F%87%A7")]),
        ("https://example.com/a😀", &[("https://example.com/a😀", "https://example.com/a%F0%9F%98%80")]),
        ("https://пример.рф/путь", &[("https://пример.рф/путь", "https://пример.рф/%D0%BF%D1%83%D1%82%D1%8C")]),  // Apple spells the host in punycode
        ("münchen.de", &[("münchen.de", "http://münchen.de")]),  // Apple spells the host in punycode
        ("www.пример.рф", &[("www.пример.рф", "http://www.пример.рф")]),  // Apple spells the host in punycode
        ("user@пример.рф", &[("user@пример.рф", "mailto:user@пример.рф")]),  // Apple spells the host in punycode
        ("имя@пример.рф", &[("имя@пример.рф", "mailto:%D0%B8%D0%BC%D1%8F@пример.рф")]),  // Apple spells the host in punycode
        ("user@例子.中国", &[("user@例子.中国", "mailto:user@例子.中国")]),  // Apple spells the host in punycode
        ("«https://example.com/a»", &[("https://example.com/a»", "https://example.com/a%C2%BB")]),
        ("（https://example.com）", &[("https://example.com", "https://example.com")]),
        ("https://example.com、次", &[("https://example.com", "https://example.com")]),
        ("https://example.com，然后", &[("https://example.com", "https://example.com")]),
        ("“https://example.com”", &[("https://example.com", "https://example.com")]),
        ("‹https://example.com/a›", &[("https://example.com/a›", "https://example.com/a%E2%80%BA")]),
        ("¿Viste example.com?", &[("example.com", "http://example.com")]),
        ("¡Llámame al +34 912 34 56 78!", &[("+34 912 34 56 78", "tel:+34912345678")]),
        ("email:nettrash@nettrash.me😀", &[("nettrash@nettrash.me", "mailto:nettrash@nettrash.me")]),
        ("תסתכל על https://example.com", &[("https://example.com", "https://example.com")]),
        ("اتصل بي على +971 50 123 4567", &[("+971 50 123 4567", "tel:+971501234567")]),
        ("यह देखो https://example.com।", &[("https://example.com", "https://example.com")]),
        ("ดูนี่ https://example.com", &[("https://example.com", "https://example.com")]),
        ("https://example.com/é", &[("https://example.com/é", "https://example.com/%C3%A9")]),
        ("https://example.com/e\u{301}", &[("https://example.com/e\u{301}", "https://example.com/e%CC%81")]),
        ("https://éxample.com", &[("https://éxample.com", "https://éxample.com")]),  // Apple spells the host in punycode
        ("Ǆemail@example.com", &[("Ǆemail@example.com", "mailto:%C7%84email@example.com")]),
        ("ßtraße.de", &[("ßtraße.de", "http://ßtraße.de")]),  // Apple spells the host in punycode
        ("café.fr", &[("café.fr", "http://café.fr")]),  // Apple spells the host in punycode
        ("👍https://example.com/x👍", &[("https://example.com/x👍", "https://example.com/x%F0%9F%91%8D")]),
        ("🇬🇧example.com", &[("example.com", "http://example.com")]),
        ("example.com🇬🇧", &[("example.com", "http://example.com")]),
        ("user@example.com🇬🇧", &[("user@example.com", "mailto:user@example.com")]),
    ];

    #[test]
    fn unicode_around_links() {
        for (text, want) in UNICODE {
            assert_eq!(spans(text), owned(want), "{text:?}");
        }
    }

    // MARK: - Where this is not Apple, on purpose

    #[test]
    fn only_web_mail_and_phone_schemes_link() {
        // Apple links every one of these, to exactly what is written — and
        // in a browser the first three are script.
        for text in [
            "javascript://%0Aalert(1)",
            "javascript://example.com/%0Aalert(document.cookie)",
            "JAVASCRIPT://x%0Aalert(1)",
            "data://text/html,x",
            "file:///etc/passwd",
            "ftp://example.com/file",
            "myapp://open/thing",
            "hxxp://example.com",
            "ahttps://example.com",
        ] {
            assert!(detect(text).is_empty(), "{text:?}");
        }
        // Nor is the host inside one a web link, or its userinfo an address.
        assert!(detect("ftp://user@example.com/file").is_empty());
        // Opaque schemes Apple links whole leave the number or address in
        // them to be linked the way this file links those.
        assert_eq!(
            spans("sms:+15551234567"),
            owned(&[("+15551234567", "tel:+15551234567")])
        );
        assert_eq!(
            spans("sip:alice@example.com"),
            owned(&[("alice@example.com", "mailto:alice@example.com")])
        );
        // A literal `tel:` with no digit in it dials nothing.
        assert!(detect("tel:abc").is_empty());
    }

    #[test]
    fn a_phone_number_never_grows_a_wrong_number() {
        // Apple: `555-123-4567,89` is one number, and Swift's
        // `dialableDigits` drops the comma — tel:555123456789.
        assert_eq!(targets("call 555-123-4567,89"), ["tel:5551234567"]);
        // Apple: `67 555-123-4567` — tel:675551234567.
        assert!(detect("*67 555-123-4567").is_empty());
        // Apple: one number, tel:55512345554321.
        assert_eq!(targets("555-1234 555-4321"), ["tel:5551234", "tel:5554321"]);
        // Apple: tel:5551234567555 and tel:7654321.
        assert!(detect("555 123 4567 555 765 4321").is_empty());
        // Apple drops the last group before a closing guillemet or a flag:
        // tel:+44207946, tel:+1555123.
        assert_eq!(targets("«+44 20 7946 0958»"), ["tel:+442079460958"]);
        assert_eq!(targets("+1 555 123 4567🇺🇸"), ["tel:+15551234567"]);
        // Apple keeps a fullwidth extension fullwidth, and the dialler gets
        // tel:5551234567;ext=%EF%BC%98%EF%BC%99.
        assert_eq!(targets("555-123-4567 x８９"), ["tel:5551234567;ext=89"]);
        // Apple: a card number in fours is a phone number.
        assert!(detect("4111 1111 1111 1111").is_empty());
    }

    #[test]
    fn percent_escapes_survive_whatever_follows() {
        // Apple re-escapes every `%` once anything non-ASCII follows the URL
        // — %D0%9C becomes %25D0%259C, a page that does not exist.
        assert_eq!(
            targets("https://ru.wikipedia.org/wiki/%D0%9C%D0%BE%D1%81%D0%BA%D0%B2%D0%B0😀"),
            ["https://ru.wikipedia.org/wiki/%D0%9C%D0%BE%D1%81%D0%BA%D0%B2%D0%B0%F0%9F%98%80"]
        );
    }

    #[test]
    fn an_internationalised_host_stays_in_unicode() {
        assert_eq!(targets("https://пример.рф"), ["https://пример.рф"]); // Apple: https://xn--e1afmkfd.xn--p1ai
        assert_eq!(targets("münchen.de"), ["http://münchen.de"]); // Apple: http://xn--mnchen-3ya.de
        assert_eq!(targets("user@例子.中国"), ["mailto:user@例子.中国"]); // Apple: …@xn--fsqu00a.xn--fiqs8s
                                                                          // The path is percent-encoded exactly as Apple does it.
        assert_eq!(
            targets("https://münchen.de/straße"),
            ["https://münchen.de/stra%C3%9Fe"]
        );
    }

    // MARK: - Swift's own rules, exactly

    /// `telURL(for:)`, over every shape this file can hand it — each
    /// expectation is Swift's own output for the same string.
    #[test]
    fn tel_url_is_swifts() {
        for (phone, tel) in [
            ("555-123-4567;89", Some("tel:5551234567;ext=89")),
            (";89", Some("tel:89")),
            ("555;;89", Some("tel:555;ext=89")),
            ("a;", Some("tel:2")),
            (";", None),
            ("", None),
            ("555;89;7", Some("tel:555;ext=897")),
            (";;89", Some("tel:89")),
            ("1-800-GOT-JUNK", Some("tel:18004685865")),
            ("1-800-got-junk", Some("tel:18004685865")),
            ("+1 (555) 123-4567", Some("tel:+15551234567")),
            ("*67 555-123-4567", Some("tel:*675551234567")),
            ("#31#555", Some("tel:%2331%23555")),
            ("555#", Some("tel:555%23")),
            ("ß", Some("tel:77")),
            ("\u{FB00}", Some("tel:33")),
            ("555;x89", Some("tel:555;ext=89")),
            ("555;8a9", Some("tel:555;ext=89")),
            ("٥٥٥", None),
            ("５５５", None),
            ("tel", Some("tel:835")),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
                Some("tel:22233344455566677778889999"),
            ),
            (
                "abcdefghijklmnopqrstuvwxyz",
                Some("tel:22233344455566677778889999"),
            ),
            ("555 1234;ext", Some("tel:5551234")),
            ("0123456789+*#", Some("tel:0123456789+*%23")),
            ("555-123-4567;;", Some("tel:5551234567")),
            ("555;1;2", Some("tel:555;ext=12")),
            ("555\u{A0}123", Some("tel:555123")),
            ("555\u{2013}123", Some("tel:555123")),
        ] {
            assert_eq!(tel_url(phone).as_deref(), tel, "{phone:?}");
        }
    }

    /// Where Swift's predicates reach wider than any string built here.
    /// `Character.isNumber` takes Arabic-Indic digits — Swift gives
    /// `555;٨٩` an extension of `%D9%A8%D9%A9` — `"0"..."9"` takes a `1`
    /// with a combining mark on it (`tel:1%CC%81`), and a letter with one
    /// is not that letter (`ǰ` dials nothing). This file's
    /// grammar folds or refuses such digits long before `tel_url`, so its
    /// narrower predicates never see a difference; pinned so that widening
    /// the grammar has to face these.
    #[test]
    fn tel_url_is_narrower_than_swift_off_its_domain() {
        assert_eq!(tel_url("555;٨٩").as_deref(), Some("tel:555"));
        assert_eq!(tel_url("1\u{301}").as_deref(), Some("tel:1"));
        // And Swift walks by Character, so `ǰ` — upper-cased to J and a
        // combining caron, one Character — is not the J that dials 5.
        assert_eq!(tel_url("\u{1F0}").as_deref(), Some("tel:5"));
        assert_eq!(targets("555-123-4567 x٨٩"), ["tel:5551234567"]);
        assert!(detect("٥٥٥-١٢٣-٤٥٦٧").is_empty());
    }

    /// Swift's `split(separator: ";", maxSplits: 1,
    /// omittingEmptySubsequences: true)`, vector for vector.
    #[test]
    fn split_extension_is_swifts_split() {
        for (phone, main, extension) in [
            (";89", Some("89"), None),
            ("555;;89", Some("555"), Some(";89")),
            ("a;", Some("a"), None),
            (";", None, None),
            ("", None, None),
            ("555;89;7", Some("555"), Some("89;7")),
            (";;89", Some("89"), None),
            ("555-123-4567;89", Some("555-123-4567"), Some("89")),
        ] {
            assert_eq!(split_extension(phone), (main, extension), "{phone:?}");
        }
    }

    /// Swift's `normalized`, over Foundation's own reading of each string.
    #[test]
    fn normalize_destination_is_swifts() {
        for (destination, opens) in [
            ("example.com/x", "https://example.com/x"),
            ("example.com", "https://example.com"),
            ("www.example.com", "https://www.example.com"),
            ("user@example.com", "https://user@example.com"),
            ("//example.com", "https:////example.com"),
            ("/path", "https:///path"),
            ("#frag", "https://#frag"),
            ("?q=1", "https://?q=1"),
            ("пример.рф", "https://пример.рф"),
            ("https://example.com", "https://example.com"),
            ("HTTPS://example.com", "HTTPS://example.com"),
            ("mailto:a@b.c", "mailto:a@b.c"),
            ("tel:+15551234567", "tel:+15551234567"),
            ("javascript:alert(1)", "javascript:alert(1)"),
            ("a+b://x", "a+b://x"),
            ("Z9+-.:x", "Z9+-.:x"),
            ("C:\\path", "C:\\path"),
            (":x", ":x"),
            // Foundation reads the host as the scheme: dead links on Apple,
            // and so here (Android makes these https://).
            ("localhost:8080", "localhost:8080"),
            ("example.com:8080/path", "example.com:8080/path"),
            ("www.example.com:443", "www.example.com:443"),
            ("user:pass@example.com", "user:pass@example.com"),
        ] {
            assert_eq!(normalize_destination(destination), opens, "{destination:?}");
        }
    }

    #[test]
    fn merge_keeps_every_declared_link_and_adds_the_rest() {
        let rendered = "see menu, https://b.example and a@b.co";
        let links = merge(
            vec![declared(rendered, "menu", "https://a.example")],
            detect(rendered),
        );
        assert_eq!(
            targets_of(&links),
            ["https://a.example", "https://b.example", "mailto:a@b.co"]
        );
        // A detected link that only TOUCHES a declared one is kept; one that
        // overlaps it by a single character is not.
        let text = "ab https://example.com";
        let touching = LinkSpan {
            range: 0..3,
            text: "ab ".into(),
            target: "https://x.example".into(),
        };
        assert_eq!(merge(vec![touching], detect(text)).len(), 2);
        let overlapping = LinkSpan {
            range: 0..4,
            text: "ab h".into(),
            target: "https://x.example".into(),
        };
        assert_eq!(
            targets_of(&merge(vec![overlapping], detect(text))),
            ["https://x.example"]
        );
        // An empty declared range overlaps nothing, as an empty attributed
        // range has no runs to object with.
        let empty = LinkSpan {
            range: 5..5,
            text: String::new(),
            target: "https://x.example".into(),
        };
        assert_eq!(merge(vec![empty], detect(text)).len(), 2);
    }

    #[test]
    fn first_web_link_is_https_only() {
        let found = detect("call 555-1234, mail a@b.co, see http://a.example and HTTPS://B.EXAMPLE then https://c.example");
        assert_eq!(
            first_web_link(&found).map(|span| span.target.as_str()),
            Some("HTTPS://B.EXAMPLE")
        );
        assert!(first_web_link(&detect("http://example.com and www.example.com")).is_none());
        assert!(first_web_link(&[]).is_none());
    }

    #[test]
    fn is_openable_allows_only_web_mail_and_phone() {
        for target in [
            "https://x",
            "HTTP://x",
            "mailto:a@b.c",
            "MAILTO:a@b.c",
            "tel:+1",
            "Tel:1",
        ] {
            assert!(is_openable(target), "{target}");
        }
        for target in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            " javascript:alert(1)",
            "data:text/html,x",
            "vbscript:x",
            "file:///etc",
            "ftp://x",
            "sms:1",
            "example.com",
            "",
            ":x",
            "//example.com",
            "localhost:8080",
            "https",
            "java\u{0}script:x",
        ] {
            assert!(!is_openable(target), "{target:?}");
        }
    }

    // MARK: - Never a panic, whatever the text

    /// Every char-boundary slice of text built to sit links against
    /// multi-byte characters: nothing panics, and every span holds.
    #[test]
    fn every_slice_of_awkward_text() {
        let seeds = [
            "Привет,https://example.com/путь—тут user@пример.рф +7 (495) 123-45-67",
            "请看https://example.com。电话：+86 10 1234 5678，邮箱：a@例子.中国",
            "👨\u{200D}👩\u{200D}👧https://example.com/👨\u{200D}👩\u{200D}👧🇷🇸🇬🇧 nettrash@nettrash.me🇷🇺",
            "«https://example.com/a»„example.com“‹a@b.co›（555-123-4567）",
            "e\u{301}xample.com https://éxample.com/e\u{301} ü@example.com x\u{308}",
            "\u{200F}https://example.com\u{200E}/x \u{202B}www.example.com\u{202C} ٥٥٥-١٢٣-٤٥٦٧",
            "１２３-４５６７ x８９ ５５５-１２３-４５６７ ｗｗｗ.example.com",
            "a@@b..c https://:80 https://[::1 tel: mailto: www. .com @ + ( ) 1-800-",
        ];
        for seed in seeds {
            let boundaries: Vec<usize> = seed
                .char_indices()
                .map(|(at, _)| at)
                .chain([seed.len()])
                .collect();
            for &from in &boundaries {
                for &to in boundaries.iter().filter(|&&to| to >= from) {
                    spans(&seed[from..to]);
                }
            }
        }
    }

    /// Thousands of messages stitched from the pieces links are made of
    /// and the characters that have broken parsers before.
    #[test]
    fn generated_messages() {
        const PIECES: &[&str] = &[
            "https://",
            "http://",
            "HTTPS://",
            "www.",
            "example",
            ".",
            "com",
            "рф",
            "/",
            "?",
            "#",
            "=",
            "&",
            "%",
            "%2",
            "(",
            ")",
            "[",
            "]",
            "{",
            "}",
            "<",
            ">",
            "'",
            "\"",
            "«",
            "»",
            "„",
            "“",
            "”",
            "@",
            "user",
            "mailto:",
            "tel:",
            "+",
            "1",
            "555",
            "-",
            "123",
            "4567",
            " ",
            "x",
            "ext",
            ";",
            ",",
            ":",
            "!",
            "…",
            "。",
            "、",
            "日本語",
            "Привет",
            "😀",
            "👨\u{200D}👩\u{200D}👧",
            "🇬🇧",
            "\u{301}",
            "\u{200D}",
            "\u{200E}",
            "\u{202B}",
            "\u{A0}",
            "\n",
            "é",
            "ß",
            "٣",
            "５",
            "_",
            "~",
            "|",
            "\\",
            "^",
            "`",
            "$",
            "*",
            "xn--",
            "1-800-",
            "GOT",
            "JUNK",
            "call ",
            "٥",
            "ไทย",
            "한국",
        ];
        let mut seed: u64 = 0x5EED_2026_0910;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        for _ in 0..20_000 {
            let mut text = String::new();
            for _ in 0..1 + next() % 14 {
                text.push_str(PIECES[next() % PIECES.len()]);
            }
            spans(&text);
        }
    }

    /// Long runs of what the scanners look back and forward over stay
    /// cheap: a 4000-character message is the protocol's limit.
    #[test]
    fn long_runs_stay_linear_enough() {
        for piece in [
            "(",
            "a.",
            "1 ",
            "@",
            "://",
            "a-",
            "x@y.",
            "555-",
            "%",
            "https://a",
            "ü.",
        ] {
            let text = piece.repeat(4000 / piece.len());
            let started = std::time::Instant::now();
            spans(&text);
            assert!(
                started.elapsed().as_secs() < 2,
                "{piece:?} took {:?}",
                started.elapsed()
            );
        }
    }
}
