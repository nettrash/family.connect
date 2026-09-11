//! Attachments, as far as they need no browser: which kind a picked file is
//! sent as, what it is called and typed, whether the server will accept its
//! bytes, and how a bubble lays it out.
//!
//! Ported from ios `Core/MediaPrep.swift` (the routes, the names, the
//! types), `Models/AttachmentAlbum.swift` (the pile), `MacViews/
//! MacMessageRow.swift` (the tile), `Core/APIModels.swift` (sizes and
//! labels), `Views/LocationAttachmentView.swift` and `Core/AudioRecorder.
//! swift` — and, for the magic numbers, from the SERVER's own
//! `handlers_attachment.rs::matches_magic`, so that what this client decides
//! to send as a video or as audio is exactly what the server will take as
//! one rather than a guess that comes back `invalid_attachment`.
use crate::i18n::{t, t1, tn};

use unicode_segmentation::UnicodeSegmentation;

/// The protocol's default ceiling for one attachment (docs/protocol.md,
/// "Limits"). A server may be configured lower; it then answers
/// `attachment_too_large`, and the send fails saying so.
pub const SIZE_LIMIT: u64 = 100 * 1024 * 1024;

/// Attachments one message may carry.
pub const MAX_PER_MESSAGE: usize = 10;

/// The longest edge of an uploaded photo, and its JPEG quality.
pub const PHOTO_EDGE: u32 = 2048;
pub const PHOTO_QUALITY: f64 = 0.85;

/// The longest edge of a preview — a photo's small copy, a video's poster —
/// and its JPEG quality.
pub const PREVIEW_EDGE: u32 = 600;
pub const PREVIEW_QUALITY: f64 = 0.7;

/// Where a video's poster frame is looked for, in seconds, in order: past a
/// fade-in first, then the very start, then a little later.
pub const POSTER_SEEK_SECONDS: [f64; 3] = [0.5, 0.0, 2.0];

/// The protocol's ceiling for an attachment name, in characters.
pub const MAX_NAME_LEN: usize = 255;

/// A voice note's longest recording, in seconds.
pub const VOICE_MAX_SECONDS: u32 = 5 * 60;

/// A recording this small or smaller has nothing in it worth sending.
pub const VOICE_MIN_BYTES: u64 = 1024;

/// How a picked file goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Decoded, downscaled and re-encoded as JPEG, with a preview. A photo
    /// the browser cannot decode falls back to [`Route::File`] at the
    /// decoding step, where that is known.
    Photo,
    /// Uploaded untouched, with a poster.
    Video,
    /// Uploaded untouched, as the type the server checks.
    Audio(&'static str),
    /// Uploaded untouched; nothing is checked.
    File,
}

/// A media type, lowercased and without its parameters.
pub fn essence(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// A name's extension, lowercased, without the dot — "" when it has none.
/// A leading dot is a hidden file's name, not an extension.
pub fn extension(name: &str) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => name[dot + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// The type a file travels as when the browser gave none: the common ones
/// by extension, and the least interesting type there is otherwise.
pub fn mime_for(name: &str) -> &'static str {
    match extension(name).as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "tif" | "tiff" => "image/tiff",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "mp3" => "audio/mpeg",
        "wav" | "wave" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "aif" | "aiff" => "audio/aiff",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "rtf" => "application/rtf",
        "zip" => "application/zip",
        "json" => "application/json",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "pages" => "application/vnd.apple.pages",
        "numbers" => "application/vnd.apple.numbers",
        "key" => "application/vnd.apple.keynote",
        _ => "application/octet-stream",
    }
}

/// The type the browser named, or the one the name implies when it named
/// none — which is what Chrome does for a HEIC photo.
pub fn declared_type(browser_mime: &str, name: &str) -> String {
    let named = essence(browser_mime);
    if named.is_empty() || named == "application/octet-stream" {
        mime_for(name).to_string()
    } else {
        named
    }
}

/// The types the photo path re-encodes. GIF and WebP animate, and a JPEG
/// has nowhere to put the frames, so they go as files with their ORIGINAL
/// bytes; BMP, SVG and anything else image-shaped goes as a file too
/// (ios MediaPrep.sendsAsFile). Everything here is decoded and re-drawn,
/// which is also what leaves its EXIF — the place it was taken — behind; a
/// photo the browser cannot decode is refused, never sent as it was.
fn is_photo_type(mime: &str) -> bool {
    matches!(
        mime,
        "image/jpeg"
            | "image/jpg"
            | "image/png"
            | "image/heic"
            | "image/heif"
            | "image/tiff"
            | "image/avif"
    )
}

/// The type the SERVER will accept a piece of audio as, from the type a
/// browser named or the name's extension (ios MediaPrep.audioMIME) — or
/// None, when it will not take it as audio at all.
pub fn audio_mime(mime: &str, name: &str) -> Option<&'static str> {
    let by_type = match mime {
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" | "audio/aac" | "audio/x-aac" => {
            Some("audio/mp4")
        }
        "audio/mpeg" | "audio/mp3" => Some("audio/mpeg"),
        "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => Some("audio/wav"),
        "audio/ogg" | "application/ogg" => Some("audio/ogg"),
        _ => None,
    };
    by_type.or(match extension(name).as_str() {
        "m4a" | "aac" => Some("audio/mp4"),
        "mp3" => Some("audio/mpeg"),
        "wav" | "wave" => Some("audio/wav"),
        "ogg" | "oga" => Some("audio/ogg"),
        _ => None,
    })
}

/// How a picked file goes, from what the browser said it is, its name, and
/// its first bytes.
///
/// The bytes are the SERVER's own test (see [`matches_magic`]): a video or
/// a piece of audio goes as one only when the server will accept its bytes
/// as one, and otherwise as a file, where nothing is checked. The Mac asks
/// its file type and sends a `.aac` stream or an `.mkv` as audio or video
/// all the same, and the server refuses the message.
pub fn route(browser_mime: &str, name: &str, head: &[u8]) -> Route {
    let mime = declared_type(browser_mime, name);
    if mime.starts_with("image/") {
        return if is_photo_type(&mime) {
            Route::Photo
        } else {
            Route::File
        };
    }
    let video = matches!(
        mime.as_str(),
        "video/mp4" | "video/quicktime" | "video/x-m4v"
    );
    if video && matches_magic("video/mp4", head) {
        return Route::Video;
    }
    if mime.starts_with("audio/") || mime == "application/ogg" || audio_mime("", name).is_some() {
        if let Some(audio) = audio_mime(&mime, name) {
            if matches_magic(audio, head) {
                return Route::Audio(audio);
            }
        }
    }
    Route::File
}

/// The server's check, byte for byte: the declared type must match what
/// the bytes are (server `handlers_attachment.rs::matches_magic`).
pub fn matches_magic(mime: &str, head: &[u8]) -> bool {
    match mime {
        "image/jpeg" => head.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/png" => head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "image/heic" | "image/heif" | "video/mp4" | "video/quicktime" | "audio/mp4"
        | "audio/m4a" => head.len() >= 12 && &head[4..8] == b"ftyp",
        "audio/mpeg" => {
            head.starts_with(b"ID3")
                || (head.len() >= 2 && head[0] == 0xFF && (head[1] & 0xE0) == 0xE0)
        }
        "audio/wav" => head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WAVE",
        "audio/ogg" => head.starts_with(b"OggS"),
        _ => false,
    }
}

/// Unicode general category Cf — the invisible FORMAT characters, bidi
/// overrides among them — which Apple's `CharacterSet.controlCharacters`
/// counts as control characters alongside Cc.
fn is_format(c: char) -> bool {
    matches!(c as u32,
        0x00AD
        | 0x0600..=0x0605
        | 0x061C
        | 0x06DD
        | 0x070F
        | 0x0890..=0x0891
        | 0x08E2
        | 0x180E
        | 0x200B..=0x200F
        | 0x202A..=0x202E
        | 0x2060..=0x2064
        | 0x2066..=0x206F
        | 0xFEFF
        | 0xFFF9..=0xFFFB
        | 0x110BD
        | 0x110CD
        | 0x13430..=0x1343F
        | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A
        | 0xE0001
        | 0xE0020..=0xE007F)
}

/// A name the server will take and a recipient will recognise (ios
/// MediaPrep.sanitizedName): `/` and `:` become `_`, and so does any
/// character carrying a control or format code — a name lands on somebody
/// else's disk, and a right-to-left override is how "invoice<U+202E>fdp.exe"
/// reads as a PDF. Trimmed, and nothing left is None.
///
/// Over the limit the STEM is cut, so ".pdf" survives. The limit is counted
/// the way the SERVER counts it — Unicode scalars — and the cut falls
/// between characters: Apple counts graphemes, so a long name in a script
/// of combining marks passes its check and fails the server's.
pub fn sanitized_name(raw: &str) -> Option<String> {
    let stripped: String = raw
        .graphemes(true)
        .map(|grapheme| {
            if grapheme == "/"
                || grapheme == ":"
                || grapheme.chars().any(|c| c.is_control() || is_format(c))
            {
                "_"
            } else {
                grapheme
            }
        })
        .collect();
    let name = stripped.trim();
    if name.is_empty() {
        return None;
    }
    if name.chars().count() <= MAX_NAME_LEN {
        return Some(name.to_string());
    }
    let ext = extension(name);
    let fits = |text: &str, room: usize| -> String {
        let mut kept = String::new();
        for grapheme in text.graphemes(true) {
            if kept.chars().count() + grapheme.chars().count() > room {
                break;
            }
            kept.push_str(grapheme);
        }
        kept
    };
    let ext_len = ext.chars().count();
    if ext.is_empty() || ext_len + 1 >= MAX_NAME_LEN {
        return Some(fits(name, MAX_NAME_LEN));
    }
    let stem = &name[..name.len() - ext.len() - 1];
    let original_ext = &name[name.len() - ext.len()..];
    Some(format!(
        "{}.{}",
        fits(stem, MAX_NAME_LEN - ext_len - 1),
        original_ext
    ))
}

/// "1.2 MB" — a file row's size, the way Apple's ByteCountFormatter writes
/// it for files: decimal units, whole kilobytes, one decimal of a megabyte,
/// two of a gigabyte, and no trailing zero.
pub fn display_size(bytes: u64) -> String {
    if bytes == 0 {
        return t("Zero KB").to_string();
    }
    if bytes < 1_000 {
        // The unit is a word here, so it is said in the reader's language;
        // above a kilobyte it is a symbol every one of the nine uses.
        return tn("%lld bytes", bytes as i64);
    }
    let trimmed = |value: f64, places: usize| -> String {
        let text = format!("{value:.places$}");
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            text
        }
    };
    let kb = bytes as f64 / 1e3;
    if kb.round() < 1_000.0 {
        return t1("%@ KB", &(kb.round() as u64).to_string());
    }
    let mb = bytes as f64 / 1e6;
    if (mb * 10.0).round() / 10.0 < 1_000.0 {
        return t1("%@ MB", &trimmed(mb, 1));
    }
    t1("%@ GB", &trimmed(bytes as f64 / 1e9, 2))
}

/// "3:42" — elapsed or total time of a recording (ios AudioRecorder.timeLabel).
pub fn time_label(seconds: f64) -> String {
    let whole = if seconds.is_finite() {
        seconds.round().max(0.0) as u64
    } else {
        0
    };
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// The width-over-height a bubble draws a photo or video at — 4:3 when the
/// uploader could not say (ios AttachmentDTO.aspectRatio).
pub fn aspect_ratio(width: Option<i64>, height: Option<i64>) -> f64 {
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => width as f64 / height as f64,
        _ => 4.0 / 3.0,
    }
}

/// The largest a lone photo or video tile draws, either way.
pub const TILE_MAX: f64 = 320.0;

/// A lone tile, at the attachment's own shape, capped at 320 either way
/// (MacAttachmentBlock.tileSize). From METADATA, never from loaded bytes, so
/// a row does not change height when its picture lands.
pub fn tile_size(width: Option<i64>, height: Option<i64>) -> (f64, f64) {
    let ratio = aspect_ratio(width, height);
    let mut w = TILE_MAX;
    let mut h = w / ratio;
    if h > TILE_MAX {
        h = TILE_MAX;
        w = h * ratio;
    }
    (w, h)
}

/// The card an album's top item is drawn in: the full media width, at the
/// item's own shape held between 3:4 and 3:2 (AttachmentAlbum.cardSize).
pub fn card_size(width: Option<i64>, height: Option<i64>, max_width: f64) -> (f64, f64) {
    let ratio = aspect_ratio(width, height).clamp(0.75, 1.5);
    (max_width, (max_width / ratio).round())
}

/// A card behind an album's top one: how far its highest corner peeks above
/// the top card, how much smaller it is drawn (about its TOP edge), and its
/// tilt in degrees (AttachmentAlbum.Layer).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub lift: f64,
    pub scale: f64,
    pub tilt: f64,
}

impl Layer {
    pub const SECOND: Layer = Layer {
        lift: 6.0,
        scale: 0.94,
        tilt: -2.5,
    };
    pub const THIRD: Layer = Layer {
        lift: 12.0,
        scale: 0.88,
        tilt: 2.5,
    };

    /// How far up this card moves so that its highest corner, shrunk about
    /// the top and tilted about the centre, sits exactly `lift` above the
    /// top card.
    pub fn offset(&self, card: (f64, f64)) -> f64 {
        let radians = self.tilt.abs().to_radians();
        let corner_rise = card.0 * self.scale / 2.0 * radians.sin();
        let centre_drop = card.1 / 2.0 * (1.0 - radians.cos());
        self.lift - corner_rise + centre_drop
    }
}

/// The room an album reserves above its card for the cards behind it.
pub const PEEK: f64 = Layer::THIRD.lift;

/// Whether an attachment kind is LOOKED at (photos and videos, which pile)
/// rather than read (files, audio, a location, which are rows).
pub fn is_media(kind: &str) -> bool {
    kind != "file" && kind != "audio" && kind != "location"
}

/// A location's second line: the coordinates to five places with a POINT,
/// whatever the reader's locale — a comma would read as a different place —
/// and the accuracy when the sender's device knew one.
pub fn location_line(latitude: f64, longitude: f64, accuracy_m: Option<f64>) -> String {
    let point = format!("{latitude:.5}, {longitude:.5}");
    match accuracy_m {
        Some(accuracy) if accuracy.is_finite() => {
            format!("{point} · ±{} m", accuracy.round() as i64)
        }
        _ => point,
    }
}

/// Where "Open in Maps" goes: Apple Maps on the web, which every browser
/// reaches and which the apps hand a location to, with the label when the
/// sender typed one.
pub fn maps_url(latitude: f64, longitude: f64, name: Option<&str>) -> String {
    let mut url = format!("https://maps.apple.com/?ll={latitude:.7},{longitude:.7}");
    let label = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| t("Location"));
    url.push_str("&q=");
    for byte in label.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                url.push(byte as char)
            }
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}

/// What an attachment is called when it has no name of its own
/// (ios AttachmentDTO.displayName).
pub fn display_name<'a>(kind: &str, name: Option<&'a str>) -> std::borrow::Cow<'a, str> {
    match name.filter(|name| !name.is_empty()) {
        Some(name) => std::borrow::Cow::Borrowed(name),
        None => std::borrow::Cow::Borrowed(match kind {
            "video" => t("Video"),
            "audio" => t("Audio"),
            "location" => t("Location"),
            "file" => t("File"),
            _ => t("Photo"),
        }),
    }
}

/// The longest edge scaled to fit `max_edge`, never up (the photo path's
/// downsample, for a browser's canvas).
pub fn fit_within(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max_edge || longest == 0 {
        return (width.max(1), height.max(1));
    }
    let scale = max_edge as f64 / longest as f64;
    let fit = |side: u32| ((side as f64 * scale).round() as u32).max(1);
    (fit(width), fit(height))
}

/// What a paste means (ios ClipboardAttachment.decide, as a browser sees
/// a clipboard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteDecision {
    /// Stage what the clipboard holds.
    Attach,
    /// Type its words into the draft.
    Type,
    /// Nothing worth a message.
    Nothing,
}

/// THE rule: WORDS WIN — an ordinary text paste must stay one, even from an
/// app that puts a picture of the selection beside the words — except for
/// copied FILES, which bring their own names along as text, one to a line,
/// and taking the names instead of the files would make the feature useless
/// exactly where it is most wanted.
pub fn paste_decision(file_names: &[String], text: &str) -> PasteDecision {
    let words = text.trim();
    let only_their_names = || {
        words
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .all(|line| file_names.iter().any(|name| line == name.as_str()))
    };
    if !file_names.is_empty() && (words.is_empty() || only_their_names()) {
        return PasteDecision::Attach;
    }
    if !words.is_empty() {
        return PasteDecision::Type;
    }
    PasteDecision::Nothing
}

/// The types a pasted item is taken as, most wanted first — GIF and WebP
/// ahead of the flattened PNG copied beside them, so an animation is not
/// pasted as a still of itself (ClipboardAttachment.preferredTypes).
pub const PASTE_PREFERENCE: [&str; 14] = [
    "image/gif",
    "image/webp",
    "image/heic",
    "image/heif",
    "image/png",
    "image/jpeg",
    "image/bmp",
    "image/tiff",
    "video/mp4",
    "video/quicktime",
    "audio/mp4",
    "audio/mpeg",
    "audio/wav",
    "application/pdf",
];

/// The type to take out of everything a clipboard item offers: the most
/// wanted one, or any that is not words.
pub fn chosen_paste_type(offered: &[String]) -> Option<String> {
    PASTE_PREFERENCE
        .iter()
        .find(|wanted| offered.iter().any(|offer| offer == *wanted))
        .map(|wanted| wanted.to_string())
        .or_else(|| {
            offered
                .iter()
                .find(|offer| !offer.starts_with("text/") && offer.contains('/'))
                .cloned()
        })
}

/// What a pasted item is called — never a scratch name.
pub fn pasted_name(mime: &str) -> String {
    let extension = match mime {
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/heic" => "heic",
        "image/heif" => "heif",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "audio/mp4" => "m4a",
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        "application/pdf" => "pdf",
        _ => "dat",
    };
    let what = match mime.split('/').next() {
        Some("image") => "image",
        Some("video") => "video",
        Some("audio") => "audio",
        _ => "item",
    };
    format!("Pasted {what}.{extension}")
}

/// Whether one more item may be staged (ios StagedAttachment.canAdd).
pub fn can_stage(already: usize) -> bool {
    already < MAX_PER_MESSAGE
}

#[cfg(test)]
mod tests {
    use super::*;

    const JPEG: &[u8] = &[
        0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1,
    ];
    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D,
    ];
    const MP4: &[u8] = &[
        0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm',
    ];
    const MKV: &[u8] = &[
        0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x86, 0x81, 1, 0x42, 0xF7, 0x81,
    ];
    const ADTS: &[u8] = &[0xFF, 0xF1, 0x50, 0x80, 0x02, 0x1F, 0xFC, 0x21, 0, 0, 0, 0];
    const MP3: &[u8] = &[b'I', b'D', b'3', 4, 0, 0, 0, 0, 0, 0, 0, 0];
    const WAV: &[u8] = &[
        b'R', b'I', b'F', b'F', 0x24, 0, 0, 0, b'W', b'A', b'V', b'E',
    ];
    const OGG: &[u8] = &[b'O', b'g', b'g', b'S', 0, 2, 0, 0, 0, 0, 0, 0];
    const WEBM: &[u8] = &[0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0, 0, 0, 0, 0];

    /// The routes the Mac takes, by type — and the one place this client is
    /// stricter than the Mac: a video or audio claim the server's own byte
    /// check would refuse goes as a file instead of failing the message.
    #[test]
    fn a_picked_file_goes_the_way_the_apps_send_it() {
        assert_eq!(route("image/jpeg", "a.jpg", JPEG), Route::Photo);
        assert_eq!(route("image/png", "a.png", PNG), Route::Photo);
        // Chrome names no type for a HEIC; the name does.
        assert_eq!(route("", "IMG_0001.HEIC", MP4), Route::Photo);
        assert_eq!(route("image/avif", "a.avif", MP4), Route::Photo);
        // Animated, or not one of the photo types: the original bytes.
        assert_eq!(route("image/gif", "a.gif", b"GIF89a"), Route::File);
        assert_eq!(route("image/webp", "a.webp", b"RIFF"), Route::File);
        assert_eq!(route("image/bmp", "a.bmp", b"BM"), Route::File);
        assert_eq!(route("image/svg+xml", "a.svg", b"<svg"), Route::File);
        assert_eq!(route("video/mp4", "clip.mp4", MP4), Route::Video);
        assert_eq!(route("video/quicktime", "IMG_0002.MOV", MP4), Route::Video);
        assert_eq!(route("", "clip.mov", MP4), Route::Video);
        assert_eq!(route("video/x-matroska", "film.mkv", MKV), Route::File);
        assert_eq!(
            route("video/mp4", "lies.mp4", MKV),
            Route::File,
            "the bytes say otherwise"
        );
        assert_eq!(route("video/webm", "a.webm", WEBM), Route::File);
        assert_eq!(
            route("audio/x-m4a", "note.m4a", MP4),
            Route::Audio("audio/mp4")
        );
        assert_eq!(
            route("audio/mpeg", "song.mp3", MP3),
            Route::Audio("audio/mpeg")
        );
        assert_eq!(
            route("audio/wav", "take.wav", WAV),
            Route::Audio("audio/wav")
        );
        assert_eq!(
            route("audio/ogg", "take.ogg", OGG),
            Route::Audio("audio/ogg")
        );
        assert_eq!(route("", "song.mp3", MP3), Route::Audio("audio/mpeg"));
        // Raw ADTS is not the MP4 the server checks for.
        assert_eq!(route("audio/aac", "raw.aac", ADTS), Route::File);
        assert_eq!(route("audio/webm", "rec.webm", WEBM), Route::File);
        assert_eq!(
            route("application/pdf", "receipts.pdf", b"%PDF"),
            Route::File
        );
        assert_eq!(route("", "notes", b"hello"), Route::File);
    }

    #[test]
    fn the_magic_numbers_are_the_servers() {
        assert!(matches_magic("image/jpeg", JPEG));
        assert!(!matches_magic("image/jpeg", PNG));
        assert!(matches_magic("image/png", PNG));
        assert!(matches_magic("video/mp4", MP4) && matches_magic("audio/mp4", MP4));
        assert!(
            !matches_magic("video/mp4", &MP4[..11]),
            "twelve bytes at least"
        );
        assert!(matches_magic("audio/mpeg", MP3));
        assert!(
            matches_magic("audio/mpeg", &[0xFF, 0xFB, 0x90, 0x64]),
            "a bare frame sync"
        );
        assert!(
            !matches_magic("audio/mpeg", &[0xFF, 0x1F, 0x90, 0x64]),
            "not a frame sync"
        );
        assert!(!matches_magic("audio/mpeg", WAV));
        assert!(matches_magic("audio/wav", WAV));
        assert!(matches_magic("audio/ogg", OGG));
        assert!(!matches_magic("audio/webm", WEBM));
        assert!(
            !matches_magic("application/pdf", b"%PDF-1.7"),
            "files are never checked"
        );
    }

    #[test]
    fn a_type_comes_from_the_browser_or_else_the_name() {
        assert_eq!(declared_type("image/PNG; charset=binary", "x"), "image/png");
        assert_eq!(declared_type("", "Scan.PDF"), "application/pdf");
        assert_eq!(
            declared_type("application/octet-stream", "a.heic"),
            "image/heic"
        );
        assert_eq!(declared_type("", "README"), "application/octet-stream");
        assert_eq!(extension(".bashrc"), "");
        assert_eq!(extension("archive.tar.gz"), "gz");
        assert_eq!(extension("trailing."), "");
    }

    /// ios MediaPrep.sanitizedName, and its tests' cases.
    #[test]
    fn a_name_is_cleaned_on_the_way_out() {
        assert_eq!(sanitized_name("report.pdf").as_deref(), Some("report.pdf"));
        assert_eq!(sanitized_name("a/b:c.txt").as_deref(), Some("a_b_c.txt"));
        assert_eq!(
            sanitized_name("line\nbreak.txt").as_deref(),
            Some("line_break.txt")
        );
        assert_eq!(
            sanitized_name("  padded.txt  ").as_deref(),
            Some("padded.txt")
        );
        // A tab is a control character before it is whitespace — Apple's
        // order, kept so a name reads the same from every device.
        assert_eq!(
            sanitized_name("tabbed.txt\t").as_deref(),
            Some("tabbed.txt_")
        );
        assert_eq!(sanitized_name("   "), None);
        assert_eq!(sanitized_name(""), None);
        // The right-to-left override that makes an .exe read as a PDF.
        assert_eq!(
            sanitized_name("invoice\u{202E}fdp.exe").as_deref(),
            Some("invoice_fdp.exe")
        );
        assert_eq!(sanitized_name("Привет.txt").as_deref(), Some("Привет.txt"));
    }

    #[test]
    fn a_long_name_keeps_its_extension_and_fits_the_servers_count() {
        let long = format!("{}.pdf", "a".repeat(300));
        let cut = sanitized_name(&long).expect("a name");
        assert_eq!(cut.chars().count(), MAX_NAME_LEN);
        assert!(cut.ends_with(".pdf"));
        // Combining marks: 200 graphemes of two scalars each is 400 scalars,
        // under Apple's count and over the server's. Cut between graphemes.
        let marked = format!("{}.txt", "e\u{0301}".repeat(200));
        let cut = sanitized_name(&marked).expect("a name");
        assert!(
            cut.chars().count() <= MAX_NAME_LEN,
            "{}",
            cut.chars().count()
        );
        assert!(cut.ends_with(".txt"));
        assert!(
            cut.trim_end_matches(".txt")
                .chars()
                .collect::<Vec<_>>()
                .chunks(2)
                .all(|pair| pair == ['e', '\u{0301}']),
            "no mark left without its letter"
        );
        let no_ext = "b".repeat(400);
        assert_eq!(
            sanitized_name(&no_ext).map(|n| n.chars().count()),
            Some(MAX_NAME_LEN)
        );
    }

    #[test]
    fn sizes_read_like_the_apps() {
        assert_eq!(display_size(0), "Zero KB");
        assert_eq!(display_size(1), "1 byte");
        assert_eq!(display_size(999), "999 bytes");
        assert_eq!(display_size(1_000), "1 KB");
        assert_eq!(display_size(182_734), "183 KB");
        assert_eq!(display_size(999_499), "999 KB");
        assert_eq!(display_size(999_500), "1 MB");
        assert_eq!(display_size(1_234_567), "1.2 MB");
        assert_eq!(display_size(2_000_000), "2 MB");
        assert_eq!(display_size(104_857_600), "104.9 MB");
        assert_eq!(display_size(1_500_000_000), "1.5 GB");
        assert_eq!(display_size(2_345_678_901), "2.35 GB");
    }

    #[test]
    fn times_are_minutes_and_seconds() {
        assert_eq!(time_label(0.0), "0:00");
        assert_eq!(time_label(4.4), "0:04");
        assert_eq!(time_label(4.5), "0:05");
        assert_eq!(time_label(222.0), "3:42");
        assert_eq!(time_label(300.0), "5:00");
        assert_eq!(time_label(-3.0), "0:00");
        assert_eq!(time_label(f64::INFINITY), "0:00");
    }

    /// The Mac's tile: 320 wide at the photo's shape, or 320 tall when
    /// that would be taller; 4:3 when nobody knew the size.
    #[test]
    fn a_tile_is_the_photos_shape_capped_at_320() {
        assert_eq!(tile_size(Some(1600), Some(1200)), (320.0, 240.0));
        assert_eq!(tile_size(Some(1080), Some(1920)), (180.0, 320.0));
        assert_eq!(tile_size(Some(1000), Some(1000)), (320.0, 320.0));
        assert_eq!(tile_size(None, Some(10)), (320.0, 240.0));
        assert_eq!(tile_size(Some(0), Some(0)), (320.0, 240.0));
    }

    /// AttachmentAlbum: the card between 3:4 and 3:2, whole points; the
    /// cards behind lifted so their highest corner shows exactly their lift.
    #[test]
    fn an_album_piles_the_way_the_phone_does() {
        assert_eq!(card_size(Some(4000), Some(1000), 320.0), (320.0, 213.0));
        assert_eq!(card_size(Some(1000), Some(4000), 320.0), (320.0, 427.0));
        assert_eq!(card_size(Some(1600), Some(1200), 320.0), (320.0, 240.0));
        let card = (320.0, 240.0);
        for layer in [Layer::SECOND, Layer::THIRD] {
            // Where the highest corner ends up, recomputed from scratch.
            let radians = layer.tilt.abs().to_radians();
            let top_after_shrink = 0.0; // shrunk about the top edge
            let corner = top_after_shrink - card.0 * layer.scale / 2.0 * radians.sin()
                + card.1 / 2.0 * (1.0 - radians.cos());
            let highest = -(corner - layer.offset(card));
            assert!((highest - layer.lift).abs() < 1e-9, "{layer:?}");
        }
        assert_eq!(PEEK, 12.0);
        let (second, third) = (Layer::SECOND, Layer::THIRD);
        assert!(
            second.tilt.signum() != third.tilt.signum(),
            "they lean opposite ways"
        );
    }

    #[test]
    fn media_pile_and_the_rest_are_rows() {
        assert!(is_media("photo") && is_media("video"));
        assert!(!is_media("file") && !is_media("audio") && !is_media("location"));
    }

    #[test]
    fn a_location_reads_as_numbers_with_a_point() {
        assert_eq!(
            location_line(55.7558, 37.6173, Some(12.0)),
            "55.75580, 37.61730 · ±12 m"
        );
        assert_eq!(
            location_line(-33.8688, 151.2093, None),
            "-33.86880, 151.20930"
        );
        assert_eq!(
            location_line(1.0, 2.0, Some(8.6)),
            "1.00000, 2.00000 · ±9 m"
        );
        assert_eq!(
            maps_url(55.7558, 37.6173, Some("Our café")),
            "https://maps.apple.com/?ll=55.7558000,37.6173000&q=Our%20caf%C3%A9"
        );
        assert_eq!(
            maps_url(1.5, -2.25, None),
            "https://maps.apple.com/?ll=1.5000000,-2.2500000&q=Location"
        );
    }

    #[test]
    fn an_attachment_without_a_name_is_called_what_it_is() {
        assert_eq!(display_name("file", Some("receipts.pdf")), "receipts.pdf");
        assert_eq!(display_name("audio", None), "Audio");
        assert_eq!(display_name("audio", Some("")), "Audio");
        assert_eq!(display_name("video", None), "Video");
        assert_eq!(display_name("location", None), "Location");
        assert_eq!(display_name("file", None), "File");
        assert_eq!(display_name("photo", None), "Photo");
    }

    #[test]
    fn a_photo_is_shrunk_to_fit_and_never_grown() {
        assert_eq!(fit_within(4032, 3024, PHOTO_EDGE), (2048, 1536));
        assert_eq!(fit_within(3024, 4032, PHOTO_EDGE), (1536, 2048));
        assert_eq!(fit_within(800, 600, PHOTO_EDGE), (800, 600));
        assert_eq!(fit_within(10000, 1, PREVIEW_EDGE), (600, 1));
        assert_eq!(fit_within(0, 0, PREVIEW_EDGE), (1, 1));
    }

    #[test]
    fn words_win_a_paste_unless_they_are_a_copied_files_name() {
        let names = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        assert_eq!(
            paste_decision(&names(&["image.png"]), ""),
            PasteDecision::Attach,
            "a screenshot"
        );
        assert_eq!(
            paste_decision(&names(&["report.pdf"]), "report.pdf"),
            PasteDecision::Attach,
            "a file copied in a file manager brings its name as text"
        );
        assert_eq!(
            paste_decision(&names(&["image.png"]), "Dinner at seven"),
            PasteDecision::Type,
            "an app that puts a picture of the words beside them"
        );
        assert_eq!(
            paste_decision(&names(&["a.pdf", "b.pdf"]), "a.pdf\nb.pdf"),
            PasteDecision::Attach,
            "several files copied at once, one name to a line"
        );
        assert_eq!(
            paste_decision(&names(&["a.pdf"]), "a.pdf\nand a sentence"),
            PasteDecision::Type
        );
        assert_eq!(paste_decision(&[], "hello"), PasteDecision::Type);
        assert_eq!(paste_decision(&[], "   "), PasteDecision::Nothing);
    }

    #[test]
    fn a_paste_takes_the_type_worth_most() {
        let offer = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        assert_eq!(
            chosen_paste_type(&offer(&["text/html", "image/png", "image/gif"])).as_deref(),
            Some("image/gif"),
            "the animation, not the still of it"
        );
        assert_eq!(
            chosen_paste_type(&offer(&["image/png", "text/plain"])).as_deref(),
            Some("image/png")
        );
        assert_eq!(
            chosen_paste_type(&offer(&["text/plain", "text/html"])),
            None
        );
        assert_eq!(
            chosen_paste_type(&offer(&["text/plain", "application/x-thing"])).as_deref(),
            Some("application/x-thing")
        );
        assert_eq!(pasted_name("image/png"), "Pasted image.png");
        assert_eq!(pasted_name("application/pdf"), "Pasted item.pdf");
    }

    #[test]
    fn ten_items_and_no_more() {
        assert!(can_stage(0) && can_stage(9));
        assert!(!can_stage(10) && !can_stage(11));
    }
}
