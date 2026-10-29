//! How an attachment is measured and labelled for drawing (`MediaText.cs`) — all of it
//! `fc_text::media` (`aspect_ratio`, `tile_size`, `card_size`, `is_media`, `time_label`,
//! `display_size`, `location_line`, `maps_url`), called, but for one thing the C# adds: A SIZE IS
//! FOR A PERSON, so it is written with the reader's decimal separator ("1,2 MB" in German), where
//! `fc_text` always writes a point. A place is not: a location's coordinates are always written
//! with a POINT, because a comma between two coordinates reads as a different place.

/// "1.2 MB" — `fc_text::media::display_size` (decimal units, whole kilobytes, one decimal of a
/// megabyte, two of a gigabyte, no trailing zero, in the reader's language as `fc_text::i18n` is
/// set) — with its decimal point written as `decimal_separator`, the reader's own (`","` in
/// German). Nothing but the size's own point is touched: the one `.` between two digits. A size of
/// nought or less is "Zero KB", as the C#'s is.
pub fn display_size(bytes: i64, decimal_separator: &str) -> String {
    let said = fc_text::media::display_size(u64::try_from(bytes).unwrap_or(0));
    if decimal_separator == "." {
        return said;
    }
    let raw = said.as_bytes();
    let point = (1..raw.len().saturating_sub(1)).find(|&at| {
        raw[at] == b'.' && raw[at - 1].is_ascii_digit() && raw[at + 1].is_ascii_digit()
    });
    match point {
        // ASCII on both sides, so `at` and `at + 1` are character boundaries.
        Some(at) => format!("{}{}{}", &said[..at], decimal_separator, &said[at + 1..]),
        None => said,
    }
}
