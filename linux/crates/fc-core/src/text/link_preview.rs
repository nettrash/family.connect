//! The card under a message's first web link (`LinkPreview.cs`; ios `LinkPreview`, android
//! `LinkPreview`): how a page's HTML becomes one ([`parse`], a port of ios `LinkPreviewParser`,
//! whose Swift tests are the spec), where a page's head ends ([`HeadEndDetector`]), and how much of
//! a page is read and how its bytes become text ([`page_reader`]). No `fc_text` module has any of
//! it: the web client draws no preview card.
//!
//! A small, tolerant scanner, not an HTML parser: the Open Graph tags, then Twitter cards, then
//! plain `<title>` and `<meta name="description">`. It folds case over ASCII only — matching on a
//! Unicode-lowercased COPY and slicing the original with its offsets corrupts titles, because
//! lowercasing is not length-preserving (İ, ẞ, the Kelvin sign). Tag and attribute names are
//! ASCII, so ASCII folding is enough, and every offset it slices at is an ASCII byte's, so a cut
//! is always on a character boundary.
//!
//! PRIVACY: building one means this device contacts the linked site. That is the trade the feature
//! makes, and why Settings can switch it off.
//!
//! URLs are strings here; the C# carries `System.Uri`. A relative image is resolved against the
//! page by RFC 3986 ([`absolute_url`]), and the result is written as `Uri.AbsoluteUri` writes it
//! for what pages put in `og:image`: scheme and host lower-cased, a default port dropped, an empty
//! path `/`, dot segments removed, and a space, a control character, a non-ASCII character or one
//! of ``"<>\^`{|}`` percent-encoded as UTF-8.

use std::collections::HashMap;

/// A preview: the page, its title, its publisher, and what it says of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPreview {
    /// Where the page was read — past any redirect — and the key its image is kept under.
    pub url: String,
    pub title: String,
    /// The publisher (`og:site_name`), falling back to the host without `www.`.
    pub site_name: String,
    pub description: Option<String>,
    pub image_url: Option<String>,
}

/// Longest prefix scanned, in UTF-16 units as the C# counts it — the SAME number as the page byte
/// cap, because a page never decodes to more UTF-16 units than it has bytes, so the parser always
/// sees everything that was downloaded. YouTube's og:title sits at ~706K (#50).
pub const SCAN_LIMIT: usize = 1_048_576;

/// Both limits count UNICODE SCALARS, as on the other platforms.
pub const MAX_TITLE_LENGTH: usize = 140;
pub const MAX_DESCRIPTION_LENGTH: usize = 300;

/// Longest entity decoded, `&` and `;` excluded — bounds the lookahead, so stray ampersands stay
/// linear.
const MAX_ENTITY_LENGTH: usize = 10;

/// A preview out of `html` read at `page_url`, or `None` when the page offers no title.
pub fn parse(html: &str, page_url: &str) -> Option<LinkPreview> {
    let head = utf16_prefix(html, SCAN_LIMIT);
    let metas = meta_tags(head);
    let meta = |key: &str| metas.get(key).map(String::as_str);

    let title = first_non_empty(&[
        meta("og:title"),
        meta("twitter:title"),
        title_tag(head).as_deref(),
    ])?;
    let description = first_non_empty(&[
        meta("og:description"),
        meta("twitter:description"),
        meta("description"),
    ]);
    let site_name =
        first_non_empty(&[meta("og:site_name")]).unwrap_or_else(|| display_host(page_url));
    let image = first_non_empty(&[
        meta("og:image"),
        meta("og:image:url"),
        meta("twitter:image"),
    ])
    .and_then(|raw| absolute_url(&raw, page_url));
    Some(LinkPreview {
        url: page_url.to_string(),
        title: clamp(&title, MAX_TITLE_LENGTH),
        site_name,
        description: description.map(|description| clamp(&description, MAX_DESCRIPTION_LENGTH)),
        image_url: image,
    })
}

/// The longest prefix of `text` that is at most `limit` UTF-16 units, cut between characters.
fn utf16_prefix(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        // Never more UTF-16 units than UTF-8 bytes.
        return text;
    }
    let mut units = 0;
    for (at, character) in text.char_indices() {
        units += character.len_utf16();
        if units > limit {
            return &text[..at];
        }
    }
    text
}

/// The host as a card shows it, without a leading `www.` — or the URL as it was given, when it
/// has no host.
pub fn display_host(url: &str) -> String {
    let host = Url::parse(url)
        .filter(|parsed| parsed.scheme.is_some())
        .and_then(|parsed| parsed.host().map(str::to_ascii_lowercase))
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| url.to_string());
    match host.strip_prefix("www.") {
        Some(rest) => rest.to_string(),
        None => host,
    }
}

// ---- scanning -----------------------------------------------------------------------------------

/// Every `<meta>`'s key (its `property` or `name`, lowercased) → content. The FIRST occurrence
/// wins.
pub fn meta_tags(html: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for tag in tags("meta", html) {
        let attributes = attributes(tag);
        let (Some(key), Some(content)) = (
            attributes
                .get("property")
                .or_else(|| attributes.get("name")),
            attributes.get("content"),
        ) else {
            continue;
        };
        result
            .entry(key.to_ascii_lowercase())
            .or_insert_with(|| collapse_whitespace(&decode_entities(content)));
    }
    result
}

/// The text of the first `<title>` element.
pub fn title_tag(html: &str) -> Option<String> {
    let open = index_of("<title", html, 0)?;
    if !is_tag_name_boundary(html, open + 6) {
        return None;
    }
    let content_start = end_of_tag(html, open + 6)?;
    let close = index_of("</title", html, content_start + 1)?;
    let text = collapse_whitespace(&decode_entities(&html[content_start + 1..close]));
    (!text.is_empty()).then_some(text)
}

/// The bodies of every `<name …>` tag: after the name, before the closing `>`.
fn tags<'a>(name: &str, html: &'a str) -> Vec<&'a str> {
    let mut tags = Vec::new();
    let opening = format!("<{name}");
    let mut index = 0;
    while index < html.len() {
        let Some(open) = index_of(&opening, html, index) else {
            break;
        };
        let after_name = open + opening.len();
        if !is_tag_name_boundary(html, after_name) {
            index = after_name;
            continue;
        }
        let Some(end) = end_of_tag(html, after_name) else {
            break;
        };
        tags.push(&html[after_name..end]);
        index = end + 1;
    }
    tags
}

/// Whether the tag name really ends here — `<metadata` is not `<meta`.
fn is_tag_name_boundary(html: &str, index: usize) -> bool {
    html[index..]
        .chars()
        .next()
        .is_none_or(|next| !next.is_alphanumeric())
}

/// The `>` closing a tag whose body starts at `start`, skipping one inside a quoted value.
fn end_of_tag(html: &str, start: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (offset, &byte) in html.as_bytes()[start..].iter().enumerate() {
        match quote {
            Some(open) => {
                if byte == open {
                    quote = None;
                }
            }
            None if byte == b'"' || byte == b'\'' => quote = Some(byte),
            None if byte == b'>' => return Some(start + offset),
            None => {}
        }
    }
    None
}

/// The first offset at or after `from` matching an already-lowercase ASCII needle,
/// ASCII-case-insensitively.
fn index_of(needle: &str, html: &str, from: usize) -> Option<usize> {
    let (needle, haystack) = (needle.as_bytes(), html.as_bytes());
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&start| {
        haystack[start..start + needle.len()]
            .iter()
            .zip(needle)
            .all(|(have, want)| have.to_ascii_lowercase() == *want)
    })
}

/// One tag body's attributes, names lowercased: single, double or no quotes, in any order; the
/// first of a name wins.
pub fn attributes(tag: &str) -> HashMap<String, String> {
    let characters: Vec<char> = tag.chars().collect();
    let length = characters.len();
    let mut result = HashMap::new();
    let mut index = 0;
    let skip_whitespace = |index: &mut usize| {
        while *index < length && characters[*index].is_whitespace() {
            *index += 1;
        }
    };
    while index < length {
        skip_whitespace(&mut index);
        let mut name = String::new();
        while index < length
            && !characters[index].is_whitespace()
            && characters[index] != '='
            && characters[index] != '/'
        {
            name.push(characters[index]);
            index += 1;
        }
        skip_whitespace(&mut index);
        if index >= length || characters[index] != '=' {
            // A valueless attribute: skip a stray "/" and carry on.
            if index < length && characters[index] == '/' {
                index += 1;
            }
            if name.is_empty() && index < length {
                index += 1;
            }
            continue;
        }
        index += 1;
        skip_whitespace(&mut index);
        let mut value = String::new();
        if index < length && (characters[index] == '"' || characters[index] == '\'') {
            let quote = characters[index];
            index += 1;
            while index < length && characters[index] != quote {
                value.push(characters[index]);
                index += 1;
            }
            if index < length {
                index += 1;
            }
        } else {
            while index < length && !characters[index].is_whitespace() {
                value.push(characters[index]);
                index += 1;
            }
        }
        if !name.is_empty() {
            result.entry(name.to_ascii_lowercase()).or_insert(value);
        }
    }
    result
}

// ---- text ---------------------------------------------------------------------------------------

/// The named and numeric entities that show up in titles and descriptions; the lookahead for `;`
/// is bounded at ten UTF-16 units of entity, as the C# counts them.
pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        output.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        // `;` within MAX_ENTITY_LENGTH + 1 UTF-16 units of the `&`.
        let mut units = 1;
        let mut semicolon = None;
        for (at, character) in after.char_indices() {
            if units >= MAX_ENTITY_LENGTH + 2 {
                break;
            }
            if character == ';' {
                semicolon = Some(at);
                break;
            }
            units += character.len_utf16();
        }
        match semicolon.and_then(|at| replacement(&after[..at]).map(|said| (at, said))) {
            Some((at, said)) => {
                output.push_str(&said);
                rest = &after[at + 1..];
            }
            None => {
                output.push('&');
                rest = after;
            }
        }
    }
    output.push_str(rest);
    output
}

fn replacement(entity: &str) -> Option<String> {
    let named = match entity.to_ascii_lowercase().as_str() {
        "amp" => Some("&"),
        "lt" => Some("<"),
        "gt" => Some(">"),
        "quot" => Some("\""),
        "apos" | "#39" => Some("'"),
        "nbsp" => Some("\u{A0}"),
        "hellip" => Some("…"),
        "mdash" => Some("—"),
        "ndash" => Some("–"),
        "rsquo" | "#8217" => Some("’"),
        "lsquo" => Some("‘"),
        "ldquo" => Some("“"),
        "rdquo" => Some("”"),
        _ => None,
    };
    if let Some(named) = named {
        return Some(named.to_string());
    }
    let digits = entity.strip_prefix('#')?;
    let (number, radix) = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => (hex, 16),
        None => (digits, 10),
    };
    // Digits only, as Swift's `UInt32(_:radix:)` takes them: no sign, no spaces, nothing empty.
    if number.is_empty() || !number.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let value = u32::from_str_radix(number, radix).ok()?;
    char::from_u32(value).map(String::from)
}

/// Runs of whitespace — the newlines inside a wrapped tag, a decoded NBSP — collapsed to single
/// spaces, and trimmed.
pub fn collapse_whitespace(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            pending_space = !output.is_empty();
            continue;
        }
        if pending_space {
            output.push(' ');
            pending_space = false;
        }
        output.push(character);
    }
    output
}

fn first_non_empty(candidates: &[Option<&str>]) -> Option<String> {
    candidates
        .iter()
        .flatten()
        .map(|candidate| candidate.trim())
        .find(|trimmed| !trimmed.is_empty())
        .map(str::to_string)
}

/// Clamped to `limit` scalars, an ellipsis after.
fn clamp(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((at, _)) => format!("{}…", text[..at].trim_end()),
        None => text.to_string(),
    }
}

// ---- URLs ---------------------------------------------------------------------------------------

/// The absolute form of a possibly-relative URL in the page — http or https only, never `file:`
/// or a custom scheme.
pub fn absolute_url(raw: &str, page: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let base = Url::parse(page).filter(|base| base.scheme.is_some());
    let resolved = if trimmed.starts_with("//") {
        let scheme = base.as_ref()?.scheme.clone()?;
        Url::parse(&format!("{scheme}:{trimmed}"))?.with_dots_removed()
    } else {
        let reference = Url::parse(trimmed)?;
        if reference.scheme.is_some() {
            reference.with_dots_removed()
        } else {
            base?.resolve(&reference)
        }
    };
    let scheme = resolved.scheme.as_deref()?.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    resolved.written(&scheme)
}

/// A URL reference split as RFC 3986 §3 splits one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Url {
    scheme: Option<String>,
    authority: Option<String>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl Url {
    /// Appendix B's split. `None` only for nothing at all.
    fn parse(text: &str) -> Option<Url> {
        let (rest, fragment) = match text.split_once('#') {
            Some((rest, fragment)) => (rest, Some(fragment.to_string())),
            None => (text, None),
        };
        let (rest, query) = match rest.split_once('?') {
            Some((rest, query)) => (rest, Some(query.to_string())),
            None => (rest, None),
        };
        let scheme_end = rest.find(':').filter(|&colon| {
            let scheme = &rest[..colon];
            scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        });
        let (scheme, rest) = match scheme_end {
            Some(colon) => (Some(rest[..colon].to_string()), &rest[colon + 1..]),
            None => (None, rest),
        };
        let (authority, path) = match rest.strip_prefix("//") {
            Some(after) => {
                let end = after.find('/').unwrap_or(after.len());
                (Some(after[..end].to_string()), after[end..].to_string())
            }
            None => (None, rest.to_string()),
        };
        Some(Url {
            scheme,
            authority,
            path,
            query,
            fragment,
        })
    }

    /// The host of the authority, without user information or port.
    fn host(&self) -> Option<&str> {
        let authority = self.authority.as_deref()?;
        let host_port = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if host_port.starts_with('[') {
            return host_port.find(']').map(|end| &host_port[..=end]);
        }
        Some(
            host_port
                .split_once(':')
                .map_or(host_port, |(host, _)| host),
        )
    }

    fn with_dots_removed(mut self) -> Url {
        self.path = remove_dot_segments(&self.path);
        self
    }

    /// RFC 3986 §5.2.2: `reference` (which has no scheme) resolved against this base.
    fn resolve(&self, reference: &Url) -> Url {
        let (authority, path, query) = if reference.authority.is_some() {
            (
                reference.authority.clone(),
                remove_dot_segments(&reference.path),
                reference.query.clone(),
            )
        } else if reference.path.is_empty() {
            (
                self.authority.clone(),
                self.path.clone(),
                reference.query.clone().or_else(|| self.query.clone()),
            )
        } else if reference.path.starts_with('/') {
            (
                self.authority.clone(),
                remove_dot_segments(&reference.path),
                reference.query.clone(),
            )
        } else {
            // §5.2.3: merge with the base's directory.
            let merged = if self.authority.is_some() && self.path.is_empty() {
                format!("/{}", reference.path)
            } else {
                match self.path.rfind('/') {
                    Some(slash) => format!("{}{}", &self.path[..=slash], reference.path),
                    None => reference.path.clone(),
                }
            };
            (
                self.authority.clone(),
                remove_dot_segments(&merged),
                reference.query.clone(),
            )
        };
        Url {
            scheme: self.scheme.clone(),
            authority,
            path,
            query,
            fragment: reference.fragment.clone(),
        }
    }

    /// The URL as `Uri.AbsoluteUri` writes an http(s) one, or `None` when it has no usable host.
    fn written(&self, scheme: &str) -> Option<String> {
        let authority = self.authority.as_deref()?;
        let (user, host_port) = match authority.rsplit_once('@') {
            Some((user, host_port)) => (Some(user), host_port),
            None => (None, authority),
        };
        let (host, port) = if host_port.starts_with('[') {
            let end = host_port.find(']')?;
            (&host_port[..=end], host_port[end + 1..].strip_prefix(':'))
        } else {
            match host_port.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (host_port, None),
            }
        };
        if host.is_empty()
            || host
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "<>\"{}|\\^`/?#".contains(c))
        {
            return None;
        }
        let port = match port {
            None | Some("") => None,
            Some(port) => {
                let number: u16 = port.parse().ok()?;
                let default = if scheme == "https" { 443 } else { 80 };
                (number != default).then_some(number)
            }
        };
        let mut out = format!("{scheme}://");
        if let Some(user) = user {
            out.push_str(&escaped(user));
            out.push('@');
        }
        out.push_str(&host.to_ascii_lowercase());
        if let Some(port) = port {
            out.push_str(&format!(":{port}"));
        }
        // `Uri` reads a backslash in an http(s) path as a slash.
        let path = self.path.replace('\\', "/");
        if path.is_empty() {
            out.push('/');
        } else {
            out.push_str(&escaped(&path));
        }
        if let Some(query) = &self.query {
            out.push('?');
            out.push_str(&escaped(query));
        }
        if let Some(fragment) = &self.fragment {
            out.push('#');
            out.push_str(&escaped(fragment));
        }
        Some(out)
    }
}

/// RFC 3986 §5.2.4.
fn remove_dot_segments(path: &str) -> String {
    let mut input = path;
    let mut output: Vec<&str> = Vec::new();
    while !input.is_empty() {
        if let Some(rest) = input
            .strip_prefix("../")
            .or_else(|| input.strip_prefix("./"))
        {
            input = rest;
        } else if input.starts_with("/./") {
            input = &input[2..];
        } else if input == "/." {
            input = "/";
        } else if input.starts_with("/../") {
            input = &input[3..];
            output.pop();
        } else if input == "/.." {
            input = "/";
            output.pop();
        } else if input == "." || input == ".." {
            input = "";
        } else {
            let start = usize::from(input.starts_with('/'));
            let end = input[start..]
                .find('/')
                .map_or(input.len(), |at| at + start);
            output.push(&input[..end]);
            input = &input[end..];
        }
    }
    output.concat()
}

/// What `Uri` percent-encodes in what a page writes: a space, a control character, a non-ASCII
/// character and ``"<>\^`{|}``, as UTF-8; everything else, an existing escape included, as it is.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_ascii_graphic() && !"\"<>\\^`{|}".contains(character) {
            out.push(character);
        } else {
            let mut buffer = [0u8; 4];
            for byte in character.encode_utf8(&mut buffer).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

// ---- the head stop ------------------------------------------------------------------------------

/// Spots the end of a page's `<head>` in a byte stream, one byte at a time, so a fetch can stop
/// there (ios `HeadEndDetector`, android `HeadEndScanner`). ASCII folding, and a match only where
/// the tag NAME ends — `<bodyguard>` is not the body. `<body` counts too, because the head end tag
/// is optional.
#[derive(Debug, Clone, Default)]
pub struct HeadEndDetector {
    window: [u8; HeadEndDetector::WINDOW],
    filled: usize,
}

impl HeadEndDetector {
    const WINDOW: usize = 7;
    const HEAD: &'static [u8] = b"</head";
    const BODY: &'static [u8] = b"<body";

    pub fn new() -> Self {
        HeadEndDetector::default()
    }

    /// True the moment `value` completes `</head` or `<body` — it is the byte after the name.
    pub fn consume(&mut self, value: u8) -> bool {
        let folded = value.to_ascii_lowercase();
        if self.filled == HeadEndDetector::WINDOW {
            self.window.copy_within(1.., 0);
            self.window[HeadEndDetector::WINDOW - 1] = folded;
        } else {
            self.window[self.filled] = folded;
            self.filled += 1;
        }
        let boundary = self.window[self.filled - 1];
        if boundary.is_ascii_lowercase() || boundary.is_ascii_digit() {
            return false;
        }
        self.ends_with(HeadEndDetector::HEAD) || self.ends_with(HeadEndDetector::BODY)
    }

    fn ends_with(&self, needle: &[u8]) -> bool {
        let end = self.filled - 1;
        end >= needle.len() && &self.window[end - needle.len()..end] == needle
    }
}

/// How much of a linked page and its image a preview reads, and how the page's bytes become text
/// (`PageReader`).
pub mod page_reader {
    use futures_util::{Stream, StreamExt};

    use super::HeadEndDetector;

    /// The page ceiling. Rarely reached — the read stops at the end of `<head>` — and 1 MB because
    /// YouTube's og: tags sit ~706K in, behind inline player JSON (#50). Equal to
    /// [`super::SCAN_LIMIT`].
    pub const MAX_PAGE_BYTES: usize = 1024 * 1024;

    pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;

    /// Up to `cap` bytes of `stream` — and for a page, up to and including the byte that ends its
    /// head. The rest is never asked for, so dropping the stream cancels its transfer. A chunk the
    /// transport fails with ends the read with that error.
    pub async fn read<S, E>(
        mut stream: S,
        cap: usize,
        stopping_at_end_of_head: bool,
    ) -> Result<Vec<u8>, E>
    where
        S: Stream<Item = Result<Vec<u8>, E>> + Unpin,
    {
        let mut output = Vec::with_capacity(cap.min(64 * 1024));
        let mut detector = stopping_at_end_of_head.then(HeadEndDetector::new);
        while output.len() < cap {
            let Some(chunk) = stream.next().await else {
                break;
            };
            let chunk = chunk?;
            let take = chunk.len().min(cap - output.len());
            if let Some(detector) = detector.as_mut()
                && let Some(end) = chunk[..take]
                    .iter()
                    .position(|&byte| detector.consume(byte))
            {
                output.extend_from_slice(&chunk[..=end]);
                return Ok(output);
            }
            output.extend_from_slice(&chunk[..take]);
        }
        Ok(output)
    }

    /// The page as text in the charset its response declared, or UTF-8 when it declared none or
    /// one unknown here. Invalid UTF-8 becomes U+FFFD, as .NET's decoder makes it.
    ///
    /// The C# has every code page .NET knows (`CodePagesEncodingProvider`). Here, with no encoding
    /// library among this crate's dependencies: UTF-8, UTF-16 (either order), US-ASCII,
    /// ISO-8859-1, windows-1252 and windows-1251; any other charset is read as UTF-8, as an
    /// unknown one is in the C#.
    pub fn decode(bytes: &[u8], charset: Option<&str>) -> String {
        let name = charset
            .map(|charset| charset.trim().trim_matches(['"', '\'']))
            .unwrap_or_default()
            .to_ascii_lowercase();
        match name.as_str() {
            "us-ascii" | "ascii" | "us" | "iso646-us" => bytes
                .iter()
                .map(|&byte| if byte < 0x80 { char::from(byte) } else { '?' })
                .collect(),
            "iso-8859-1" | "iso_8859-1" | "latin1" | "l1" | "cp819" | "ibm819" => {
                bytes.iter().map(|&byte| char::from(byte)).collect()
            }
            "windows-1252" | "cp1252" | "x-cp1252" => single_byte(bytes, &WINDOWS_1252_HIGH),
            "windows-1251" | "cp1251" | "x-cp1251" => single_byte(bytes, &WINDOWS_1251_HIGH),
            "utf-16" | "utf-16le" | "unicode" => utf16(bytes, u16::from_le_bytes),
            "utf-16be" | "unicodefffe" => utf16(bytes, u16::from_be_bytes),
            _ => String::from_utf8_lossy(bytes).into_owned(),
        }
    }

    fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|pair| match pair {
                [a, b] => unit([*a, *b]),
                // A trailing odd byte is no character.
                _ => 0xFFFD,
            })
            .collect();
        String::from_utf16_lossy(&units)
    }

    fn single_byte(bytes: &[u8], high: &[u16; 128]) -> String {
        bytes
            .iter()
            .map(|&byte| {
                if byte < 0x80 {
                    char::from(byte)
                } else {
                    char::from_u32(u32::from(high[usize::from(byte - 0x80)])).unwrap_or('\u{FFFD}')
                }
            })
            .collect()
    }

    /// windows-1252's 0x80–0xFF; the five bytes it leaves undefined read as the C1 control of the
    /// same number, as .NET reads them.
    const WINDOWS_1252_HIGH: [u16; 128] = {
        let mut table = [0u16; 128];
        let c1: [u16; 32] = [
            0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
            0x2039, 0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022,
            0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
        ];
        let mut at = 0;
        while at < 128 {
            table[at] = if at < 32 { c1[at] } else { 0x80 + at as u16 };
            at += 1;
        }
        table
    };

    /// windows-1251's 0x80–0xFF: 0xC0–0xFF are А–я in order; the one undefined byte (0x98) reads
    /// as U+0098.
    const WINDOWS_1251_HIGH: [u16; 128] = {
        let mut table = [0u16; 128];
        let low: [u16; 64] = [
            0x0402, 0x0403, 0x201A, 0x0453, 0x201E, 0x2026, 0x2020, 0x2021, 0x20AC, 0x2030, 0x0409,
            0x2039, 0x040A, 0x040C, 0x040B, 0x040F, 0x0452, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022,
            0x2013, 0x2014, 0x0098, 0x2122, 0x0459, 0x203A, 0x045A, 0x045C, 0x045B, 0x045F, 0x00A0,
            0x040E, 0x045E, 0x0408, 0x00A4, 0x0490, 0x00A6, 0x00A7, 0x0401, 0x00A9, 0x0404, 0x00AB,
            0x00AC, 0x00AD, 0x00AE, 0x0407, 0x00B0, 0x00B1, 0x0406, 0x0456, 0x0491, 0x00B5, 0x00B6,
            0x00B7, 0x0451, 0x2116, 0x0454, 0x00BB, 0x0458, 0x0405, 0x0455, 0x0457,
        ];
        let mut at = 0;
        while at < 128 {
            table[at] = if at < 64 {
                low[at]
            } else {
                0x0410 + (at as u16 - 64)
            };
            at += 1;
        }
        table
    };
}
