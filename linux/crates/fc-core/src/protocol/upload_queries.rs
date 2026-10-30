//! The query strings uploads carry their facts in (docs/protocol.md, "Attachments") — a port of
//! `win/src/FamilyConnect.Core/Protocol/UploadQueries.cs`, with the two .NET formatting
//! primitives it leans on (`ToString("F7", InvariantCulture)` and `Uri.EscapeDataString`) written
//! out and pinned by tests.

/// A place: the one upload with nothing to upload — three numbers in the query (docs/protocol.md,
/// "Locations"), written the way the web client's `upload_query` writes them: seven places, and
/// the accuracy as whole metres, rounded, never negative, and left out when it is not a number
/// at all.
pub fn location(
    latitude: f64,
    longitude: f64,
    accuracy_m: Option<f64>,
    name: Option<&str>,
) -> String {
    let mut query = vec![
        "kind=location".to_string(),
        // Seven places: a centimetre, and more than any fix is good for.
        format!("latitude={}", fixed(latitude, 7)),
        format!("longitude={}", fixed(longitude, 7)),
    ];
    if let Some(accuracy) = accuracy_m.filter(|accuracy| accuracy.is_finite()) {
        // Away from zero on a half, as `MidpointRounding.AwayFromZero` — which is Rust's own
        // `round`. The cast saturates, as .NET 9+'s does.
        query.push(format!("accuracy_m={}", accuracy.round().max(0.0) as i64));
    }
    if let Some(name) = name.filter(|name| !name.is_empty()) {
        query.push(format!("name={}", escape_data_string(name)));
    }
    query.join("&")
}

/// A number with exactly `places` digits after the point, as .NET's `ToString("F<places>",
/// InvariantCulture)` writes it: the exact decimal value rounded half to EVEN on an exact tie
/// (.NET 10 writes 0.00390625 as `0.0039062`, and so does Rust's `{:.7}`), a negative zero or a
/// negative value that rounds to zero keeping its sign (`-0.0000000`), and .NET's names for the
/// three values that are not numbers.
pub fn fixed(value: f64, places: usize) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value == f64::INFINITY {
        "Infinity".to_string()
    } else if value == f64::NEG_INFINITY {
        "-Infinity".to_string()
    } else {
        format!("{value:.places$}")
    }
}

/// `Uri.EscapeDataString`: every byte of the UTF-8 percent-encoded, upper-case hex, except the
/// unreserved characters of RFC 3986 (`A–Z a–z 0–9 - . _ ~`).
pub fn escape_data_string(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            escaped.push(char::from(byte));
        } else {
            escaped.push_str(&format!("%{byte:02X}"));
        }
    }
    escaped
}
