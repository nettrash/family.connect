//! What the user types, turned into the two URLs this client speaks (docs/protocol.md,
//! "Transport"): REST under `{base}/api/v1` and the socket at `{base}/api/v1/ws`, `wss://` for an
//! https server and `ws://` for an http one — a port of
//! `win/src/FamilyConnect.Core/Protocol/ServerUrl.cs`.
//!
//! People type `chat.example.com`, `https://chat.example.com/` and `192.168.1.10:8080`, and every
//! client normalises the same way: a bare host gets `https://`, a trailing slash goes, and an
//! explicit scheme is kept — a family running on a LAN with no certificate types `http://` and
//! means it.
//!
//! There is no URL crate here, so the parse is written out, held to what .NET's
//! `Uri.TryCreate` + `UriBuilder` made of the same text (checked against .NET 10 itself; the
//! cases are in `tests/protocol_server_url.rs`): the scheme and an ASCII host are lowercased, a
//! default port is dropped, user info, path, query and fragment are dropped, an IPv6 literal is
//! written in its canonical form without its zone, an IPv4 address in any of the forms
//! `inet_aton` reads (`127.1`, `0x7f.0.0.1`, `010.0.0.1`) is written as a dotted quad, and a
//! host with an ASCII character a DNS name cannot hold, or an empty label, is no host. A
//! non-ASCII host is kept as typed, as .NET kept it: turning it into its `xn--` form is the HTTP
//! transport's job.

use std::fmt;
use std::net::Ipv6Addr;
use std::str::FromStr;

/// A normalised server base: scheme, host and a non-default port, and nothing else — `{base}` is
/// an origin. Displayed as the base URL, `https://chat.example.com/`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerUrl {
    secure: bool,
    host: String,
    port: Option<u16>,
}

impl ServerUrl {
    /// The normalised base, or `None` when there is no host in it at all.
    pub fn normalise(typed: Option<&str>) -> Option<ServerUrl> {
        let text = typed?.trim();
        if text.is_empty() {
            return None;
        }
        let text = if text.contains("://") {
            text.to_string()
        } else {
            // A bare host is https: the one default that is safe to guess wrong in the strict
            // direction — a plain-text server has to be asked for by name.
            format!("https://{text}")
        };
        let (scheme, rest) = text.split_once("://")?;
        let secure = match scheme_of(scheme)?.as_str() {
            "https" => true,
            "http" => false,
            _ => return None,
        };
        // The path is dropped on purpose: `{base}` is an origin, and a base with a path would
        // put `/api/v1` under whatever the user happened to paste.
        let authority = match rest.find(['/', '?', '#']) {
            Some(end) => &rest[..end],
            None => rest,
        };
        let host_and_port = match authority.matches('@').count() {
            0 => authority,
            1 => authority.split_once('@').map(|(_, after)| after)?,
            _ => return None,
        };
        let (host, port) = host_and_port_of(host_and_port)?;
        let default = if secure { 443 } else { 80 };
        Some(ServerUrl {
            secure,
            host,
            port: port.filter(|port| *port != default),
        })
    }

    /// `https` or `http`.
    pub fn scheme(&self) -> &'static str {
        if self.secure { "https" } else { "http" }
    }

    /// The host as it goes in a URL: lowercased when ASCII, an IPv6 literal in brackets.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port, when it is not the scheme's default.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    fn origin(&self, scheme: &str) -> String {
        match self.port {
            Some(port) => format!("{scheme}://{}:{port}", self.host),
            None => format!("{scheme}://{}", self.host),
        }
    }

    /// The REST root: `{base}/api/v1`, with no trailing slash.
    pub fn rest(&self) -> String {
        format!("{}/api/v1", self.origin(self.scheme()))
    }

    /// The socket URL. The token never travels in it: a native client sends
    /// `Authorization: Bearer` on the upgrade, which is what this one does — the two subprotocols
    /// are the browser's workaround, because a browser cannot set headers on an upgrade. "A token
    /// in the query string is not a token at all" (docs/protocol.md, "WebSocket protocol"), and a
    /// URL is written into every proxy log there is.
    ///
    /// The one thing the query does carry is `families=1`: the socket's counterpart of the
    /// `X-FC-Families: 1` header, without which the server sends only what concerns the home
    /// family (docs/protocol.md, "Several families"). `wss`'s default port is `https`'s and `ws`'s
    /// is `http`'s, so the port carries over as it is.
    pub fn socket(&self) -> String {
        let scheme = if self.secure { "wss" } else { "ws" };
        format!("{}/api/v1/ws?families=1", self.origin(scheme))
    }
}

impl fmt::Display for ServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/", self.origin(self.scheme()))
    }
}

/// A scheme as RFC 3986 spells one, lowercased; `None` for anything else.
fn scheme_of(scheme: &str) -> Option<String> {
    let mut chars = scheme.chars();
    let first = chars.next()?;
    let valid = first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then(|| scheme.to_ascii_lowercase())
}

/// The host (normalised) and the port written after it, if any.
fn host_and_port_of(text: &str) -> Option<(String, Option<u16>)> {
    if let Some(literal) = text.strip_prefix('[') {
        let (inside, after) = literal.split_once(']')?;
        // A zone (`fe80::1%en0`, `%25en0` in a URL) means nothing off this machine: .NET drops it.
        let address = inside.split('%').next().unwrap_or_default();
        let address = Ipv6Addr::from_str(address).ok()?;
        // .NET reads a port after the bracket and ignores anything else there.
        let port = match after.strip_prefix(':') {
            Some(port) => port_of(port)?,
            None => None,
        };
        return Some((format!("[{address}]"), port));
    }
    let (host, port) = match text.split_once(':') {
        Some((host, port)) => (host, port_of(port)?),
        None => (text, None),
    };
    Some((host_of(host)?, port))
}

/// A port: digits, at most 65535; empty is the scheme's default.
fn port_of(text: &str) -> Option<Option<u16>> {
    if text.is_empty() {
        return Some(None);
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Leading zeros are allowed (`065` is 65); the digits are bounded before they are summed.
    let digits = text.trim_start_matches('0');
    if digits.len() > 5 {
        return None;
    }
    let value: u32 = digits.parse().unwrap_or(0);
    u16::try_from(value).ok().map(Some)
}

/// A host that is not an IPv6 literal: an IPv4 address in any form `inet_aton` reads, written as
/// a dotted quad, or a DNS name.
fn host_of(host: &str) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    if let Some([a, b, c, d]) = ipv4_of(host) {
        return Some(format!("{a}.{b}.{c}.{d}"));
    }
    let labels: Vec<&str> = host.split('.').collect();
    // One empty label at the end is the DNS root (`example.com.`); any other is no name.
    let last = labels.len() - 1;
    let named = labels
        .iter()
        .enumerate()
        .all(|(i, label)| !label.is_empty() || (i == last && i > 0));
    let lettered = host
        .chars()
        .all(|c| !c.is_ascii() || c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (named && lettered).then(|| host.to_ascii_lowercase())
}

/// An IPv4 address as `inet_aton` (and .NET) reads one: one to four parts, each decimal, octal
/// with a leading `0` or hexadecimal with `0x`; the last part fills the bytes the others left.
fn ipv4_of(host: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() > 4 {
        return None;
    }
    let mut values = Vec::with_capacity(parts.len());
    for part in &parts {
        values.push(ipv4_part(part)?);
    }
    let (last, leading) = values.split_last()?;
    if leading.iter().any(|value| *value > 255) {
        return None;
    }
    let room = 8 * (4 - leading.len() as u32);
    if room < 32 && *last >= 1u64 << room {
        return None;
    }
    if *last > u64::from(u32::MAX) {
        return None;
    }
    let mut address: u32 = 0;
    for (i, value) in leading.iter().enumerate() {
        address |= (*value as u32) << (24 - 8 * i as u32);
    }
    address |= *last as u32;
    Some(address.to_be_bytes())
}

/// One part of an IPv4 address, bounded so that no part can overflow.
fn ipv4_part(part: &str) -> Option<u64> {
    let (digits, radix) =
        if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
            (hex, 16)
        } else if part.len() > 1 && part.starts_with('0') {
            (&part[1..], 8)
        } else {
            (part, 10)
        };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let digits = digits.trim_start_matches('0');
    if digits.len() > 12 {
        return None;
    }
    if digits.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(digits, radix).ok()
}
