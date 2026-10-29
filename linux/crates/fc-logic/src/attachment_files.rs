//! What the window asks of an attachment before it draws or saves one (`AttachmentFiles.cs`) —
//! ported from the web client's attachment view (`web/src/views/attachments.rs`), which is where
//! these rules were first written down, and tested with its vectors.

use fc_core::protocol::AttachmentDto;
use fc_text::media;

/// Which bytes a tile draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileSource {
    /// Nothing to fetch: a video with no poster is drawn as a labelled box.
    None,
    /// The small copy the uploader made.
    Preview,
    /// The photo itself, when it has no small copy.
    Original,
}

/// The preview when there is one; a PHOTO's own bytes when there is not; and for a video without a
/// poster, nothing — a tile never downloads a whole video to draw itself.
///
/// A STICKER IS ALWAYS ITS ORIGINAL, whatever `has_preview` says. A preview is a JPEG — no
/// transparency, one frame — and the flag can be true on a sticker nobody made a preview for: an
/// upload whose bytes the family already holds inherits it from the row it deduplicated against,
/// and somebody may once have sent that same PNG as a photograph (docs/protocol.md, "And it has no
/// preview").
pub fn source_for(attachment: &AttachmentDto) -> TileSource {
    if attachment.sticker {
        TileSource::Original
    } else if attachment.has_preview {
        TileSource::Preview
    } else if attachment.kind == "photo" {
        TileSource::Original
    } else {
        TileSource::None
    }
}

/// Win32's device names, which are reserved in EVERY directory and with ANY extension.
const RESERVED_STEMS: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// What Windows refuses in a file name (`Path.GetInvalidFileNameChars()` on Windows): the control
/// characters, and `"<>|:*?\/`.
fn refused(character: char) -> bool {
    character < ' '
        || matches!(
            character,
            '"' | '<' | '>' | '|' | ':' | '*' | '?' | '\\' | '/'
        )
}

/// A file name safe to write: never a path, never a character Windows refuses, and never one of
/// Win32's device names.
///
/// The name comes from whoever sent the attachment, and the server does not vet it: for a file it
/// checks that the name is 1..=MAX_NAME_LEN characters after trimming and nothing else
/// (`handlers_attachment.rs`). So this is not a belt to the server's braces — it is the only guard
/// there is.
///
/// The device names are the part that is easy to miss on Windows: Win32 resolves CON, PRN, AUX,
/// NUL, COM1-9 and LPT1-9 to a DEVICE in every directory and with any extension. A leading
/// underscore is enough to make it an ordinary name again.
///
/// LINUX KEEPS WINDOWS' RULES, deliberately — a Linux file system takes everything but `/` and NUL
/// (what .NET's `GetInvalidFileNameChars` answers there), but a name saved here goes on to USB
/// sticks (FAT, exFAT, NTFS) and SMB shares, which refuse exactly these. The C#'s tests, written on
/// Windows, pin them.
pub fn safe_file_name(name: &str) -> String {
    let unified = name.replace('\\', "/");
    let bare = unified.rsplit('/').next().unwrap_or_default();
    let replaced: String = bare
        .chars()
        .map(|character| if refused(character) { '_' } else { character })
        .collect();
    // `Trim()` — .NET's white space is Rust's — then `TrimEnd('.')`, in that order.
    let cleaned = replaced.trim().trim_end_matches('.');
    if cleaned.is_empty() {
        return "attachment.bin".to_string();
    }
    // `Path.GetFileNameWithoutExtension`: everything before the last dot.
    let stem = cleaned.rfind('.').map_or(cleaned, |dot| &cleaned[..dot]);
    if RESERVED_STEMS
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
    {
        format!("_{cleaned}")
    } else {
        cleaned.to_string()
    }
}

/// What a download is saved as: its own name, or one made from its kind and type.
pub fn file_name(attachment: &AttachmentDto) -> String {
    if let Some(name) = attachment.name.as_deref().filter(|name| !name.is_empty()) {
        return name.to_string();
    }
    let extension = match attachment.mime.as_deref().map(media::essence).as_deref() {
        Some("image/jpeg") => "jpg",
        Some("image/png") => "png",
        Some("image/webp") => "webp",
        Some("image/heic") => "heic",
        Some("video/quicktime") => "mov",
        Some("video/mp4") => "mp4",
        Some("audio/mp4") => "m4a",
        Some("audio/mpeg") => "mp3",
        Some("audio/wav") => "wav",
        Some("audio/ogg") => "ogg",
        _ => "bin",
    };
    format!("{}-{}.{extension}", attachment.kind, attachment.id.max(0))
}

/// A long name with its middle given up, so its start AND its extension stay readable. Counted in
/// scalars (`char`s), so a cut never splits an emoji.
pub fn middle_truncate(name: &str, room: usize) -> String {
    let scalars: Vec<char> = name.chars().collect();
    if scalars.len() <= room || room < 5 {
        return name.to_string();
    }
    let tail = (room - 1) / 3;
    let head = room - 1 - tail;
    let start: String = scalars[..head].iter().collect();
    let end: String = scalars[scalars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}
