//! The family's sticker pack (docs/protocol.md, "Sticker pack"): the rules a
//! client keeps that have no browser in them.
//!
//! A WORD FIRST, as the protocol says it: everywhere else in this crate
//! "sticker" is a board NOTE ([`crate::board`]). Here it is the other thing —
//! a small picture sent in a chat — and the collection is a PACK, which is
//! the wire's word for it, so that neither has to be read twice.
//!
//! What is here: which bytes are a sticker as they stand and which have to be
//! made into one, the 512 × 512 box a made one is fitted into, whether a
//! picture is animated (and so can never be re-encoded), the label's rule,
//! who may remove an item, whether a sticker in a chat could be one the pack
//! already holds, and the order a panel shows the pack in. What is NOT here
//! is the cursor: that is the board's machinery unchanged, and lives beside
//! the board's in the client.

use crate::media;

/// The box a sticker this client MAKES is fitted into, whole (the
/// protocol's "512 × 512 is the CLIENT's rule"). The server never decodes a
/// picture and cannot measure one.
pub const EDGE: u32 = 512;

/// The most characters a label may have — fixed, not the operator's.
pub const LABEL_MAX: usize = 64;

/// The one box every sticker is drawn in, in CSS pixels: larger than the
/// largest emoji-only message and smaller than a photograph's 320, fitted
/// whole and never at the picture's own pixel size.
pub const BOX: f64 = 160.0;

/// How many recently used stickers a panel puts first. Enough to be the
/// handful somebody actually reaches for; few enough that the rest of the
/// pack stays where it was.
pub const RECENTS_MAX: usize = 16;

/// The two types a sticker may be (`invalid_attachment` otherwise).
pub const WEBP: &str = "image/webp";
pub const PNG: &str = "image/png";

/// Whether a declared type is one a sticker may be — parameters and case
/// aside, as every type comparison here is made.
pub fn is_sticker_type(mime: &str) -> bool {
    matches!(media::essence(mime).as_str(), WEBP | PNG)
}

/// What the BYTES are, by the server's own magic numbers: `RIFF` at 0 and
/// `WEBP` at 8 (the four between are the file's length and are not
/// checked), or PNG's eight. Not what the browser said, and not the name:
/// the upload's `Content-Type` is checked against these very bytes, so a
/// `.png` that is really a JPEG must not go up declared as a PNG.
pub fn type_of(head: &[u8]) -> Option<&'static str> {
    if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        Some(WEBP)
    } else if media::matches_magic(PNG, head) {
        Some(PNG)
    } else {
        None
    }
}

/// Whether these bytes begin a GIF — which is never a sticker as it stands
/// (the pack takes WebP and PNG), and matters here for one reason: it is
/// the one other type somebody picks that MOVES. See [`is_animated`].
pub fn is_gif(head: &[u8]) -> bool {
    head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a")
}

/// Whether a GIF holds more than one picture, as far as `bytes` shows.
///
/// A GIF has no flag for it: the file is a run of blocks, and it is
/// animated when a second image descriptor turns up. So the blocks are
/// walked — extensions skipped by their sub-blocks, each picture by its
/// colour table and its data — until the second picture, the trailer, or
/// the end of what was read. Cut short before either, the looping
/// extension every encoder writes in front of an animation (`NETSCAPE2.0`,
/// or its twin `ANIMEXTS1.0`) is taken at its word; on a file read whole it
/// is not, since a still GIF may carry one and is still a still.
fn gif_is_animated(bytes: &[u8]) -> bool {
    /// Past a run of sub-blocks: a length byte and that many bytes, until
    /// a length of nought. None when the bytes run out first.
    fn past_sub_blocks(bytes: &[u8], mut at: usize) -> Option<usize> {
        loop {
            let length = usize::from(*bytes.get(at)?);
            at += 1;
            if length == 0 {
                return Some(at);
            }
            at += length;
        }
    }
    /// The bytes of a colour table, from the packed field that declares it.
    fn table(packed: u8) -> usize {
        if packed & 0x80 != 0 {
            3 << ((packed & 0x07) + 1)
        } else {
            0
        }
    }
    let Some(&packed) = bytes.get(10) else {
        return false;
    };
    // The signature, the logical screen descriptor, the global table.
    let mut at = 13 + table(packed);
    let mut pictures = 0;
    let mut loops = false;
    loop {
        match bytes.get(at) {
            Some(0x2C) => {
                pictures += 1;
                if pictures > 1 {
                    return true;
                }
                let Some(&packed) = bytes.get(at + 9) else {
                    return loops;
                };
                // The descriptor, its local table, the LZW code size.
                match past_sub_blocks(bytes, at + 10 + table(packed) + 1) {
                    Some(next) => at = next,
                    None => return loops,
                }
            }
            Some(0x21) => {
                if bytes.get(at + 1) == Some(&0xFF) {
                    let name = bytes.get(at + 3..at + 14);
                    loops |= matches!(name, Some(b"NETSCAPE2.0") | Some(b"ANIMEXTS1.0"));
                }
                match past_sub_blocks(bytes, at + 2) {
                    Some(next) => at = next,
                    None => return loops,
                }
            }
            // Read to its end without a second picture.
            Some(0x3B) => return false,
            // Cut short between blocks.
            None => return loops,
            // Nothing a GIF holds: whatever this is, it is not moving.
            Some(_) => return false,
        }
    }
}

/// Whether these bytes are an ANIMATED WebP, PNG or GIF, as far as `head`
/// shows.
///
/// WebP says so in its extended header: a `VP8X` chunk first, whose flags
/// byte carries the animation bit. A PNG is animated when an `acTL` chunk
/// comes before its first `IDAT`, so the chunks are walked until one of the
/// two turns up. A head too short to settle it answers false — the caller
/// reads enough that only a PNG with kilobytes of metadata in front of its
/// pixels could be cut short, and the worst that costs is a still frame.
/// A GIF says so only by holding a second picture, which may be anywhere in
/// the file: the caller reads a GIF WHOLE before asking.
pub fn is_animated(head: &[u8]) -> bool {
    if is_gif(head) {
        return gif_is_animated(head);
    }
    match type_of(head) {
        Some(WEBP) => head.len() > 20 && &head[12..16] == b"VP8X" && head[20] & 0x02 != 0,
        Some(PNG) => {
            let mut at = 8usize;
            while let Some(chunk) = head.get(at..at + 8) {
                let length = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize;
                match &chunk[4..8] {
                    b"acTL" => return true,
                    b"IDAT" | b"IEND" => return false,
                    _ => {}
                }
                // Length, type, the data, and its CRC.
                let Some(next) = at.checked_add(12).and_then(|step| step.checked_add(length))
                else {
                    return false;
                };
                at = next;
            }
            false
        }
        _ => false,
    }
}

/// What a picked picture becomes on its way into the pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Already a sticker: the bytes go up AS THEY ARE, declared as this
    /// type, whatever their pixel size. Re-encoding somebody's finished
    /// sticker buys nothing and, for an animated one, is not possible.
    AsGiven(&'static str),
    /// A still picture that is not one yet — another type, or a WebP or PNG
    /// over the byte ceiling: decoded, fitted whole into 512 × 512 with its
    /// transparency kept, and written again.
    Remake,
}

/// Why a picture cannot be a sticker, said beside the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Over the per-item ceiling, and nothing this client may do makes it
    /// smaller: it is animated, or it is still too big at 512 × 512.
    TooLarge,
    /// The pack already holds as many as the server allows.
    Full,
    /// Nothing this browser can decode.
    Unreadable,
    /// It moves, and it is not a WebP this client may take as it stands: an
    /// animated GIF, or an animated PNG too big to go up as given. A client
    /// makes a sticker only out of a STILL picture — made into one, this
    /// would be one frame of what was picked, and nobody would have been
    /// told. Said in words of its own: an animated sticker must be a WebP.
    Animated,
}

/// What to do with a picked file, from its first bytes and its size.
///
/// `max_bytes` is the server's `max_pack_item_bytes`. A WebP or PNG within
/// it is taken as given — animated or not: the bytes go up untouched, and
/// a browser draws either moving. One over it is remade only if it is a
/// still, since an animated one re-encoded is one frame of what it was: an
/// animated WebP over the ceiling is simply too large, and an animated PNG
/// over it is refused as every other animation that is not a WebP is — the
/// remake would flatten it, and a WebP of the same animation is what would
/// fit. For the same reason an animated picture of any OTHER type is
/// refused rather than remade — the protocol makes an item "out of a
/// larger still picture" — and in words of its own: its size is not what
/// is wrong with it. (`head` is the whole file for a GIF; see
/// [`is_animated`].)
pub fn plan(head: &[u8], size: u64, max_bytes: u64) -> Result<Plan, Refusal> {
    match type_of(head) {
        Some(mime) if size <= max_bytes => Ok(Plan::AsGiven(mime)),
        Some(WEBP) if is_animated(head) => Err(Refusal::TooLarge),
        _ if is_animated(head) => Err(Refusal::Animated),
        _ => Ok(Plan::Remake),
    }
}

/// The size a remade sticker is drawn at: the whole picture inside
/// 512 × 512, its proportions kept, and never scaled UP — a 96-pixel
/// picture stays 96 pixels in the file and is only drawn in the box.
pub fn fit(width: u32, height: u32) -> (u32, u32) {
    media::fit_within(width, height, EDGE)
}

/// The WebP qualities tried, in order, when a remade sticker's PNG is over
/// the byte ceiling and this browser can write WebP at all. PNG comes first
/// because it loses nothing; these lose a little more each step, and past
/// the last the picture is refused rather than turned to mud.
pub const WEBP_QUALITIES: [f64; 3] = [0.9, 0.8, 0.6];

/// A label over the 64 characters the server allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelTooLong;

/// A label as it is sent: trimmed, and None when nothing is left — an empty
/// one is no label. Refused when it is over the 64 characters the server
/// allows (`validation`), counted the way the server counts them.
pub fn label(raw: &str) -> Result<Option<String>, LabelTooLong> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > LABEL_MAX {
        return Err(LabelTooLong);
    }
    Ok(Some(trimmed.to_string()))
}

/// Whether the pack has room for one more. The ceiling is the family's,
/// read from `GET /families/mine`; a pack already over it (an operator
/// lowered it) is frozen, not trimmed.
pub fn has_room(held: usize, max_items: i64) -> bool {
    (held as i64) < max_items
}

/// Who may remove an item: whoever added it, or the family's owner. A
/// permission shape the board does not have — a note is its author's alone
/// — because a member who left, or deleted their account, leaves their
/// stickers behind, and under an author-only rule nobody could ever remove
/// those.
pub fn may_remove(added_by: Option<i64>, me: i64, owner: bool) -> bool {
    owner || added_by == Some(me)
}

/// Whether a sticker in a chat COULD be this pack item: the same size and
/// the same type. Only a candidate — the bytes decide — but it is what lets
/// a client compare one or two pictures instead of two hundred. An unknown
/// size on either side rules nothing out.
pub fn could_be(
    size: Option<i64>,
    mime: Option<&str>,
    item_size: Option<i64>,
    item_mime: Option<&str>,
) -> bool {
    let sizes = match (size, item_size) {
        (Some(one), Some(other)) => one == other,
        _ => true,
    };
    let types = match (mime, item_mime) {
        (Some(one), Some(other)) => media::essence(one) == media::essence(other),
        _ => true,
    };
    sizes && types
}

/// The recents after `id` was used: it first, once, and no more than
/// [`RECENTS_MAX`] kept.
pub fn used(recents: &[i64], id: i64) -> Vec<i64> {
    std::iter::once(id)
        .chain(recents.iter().copied().filter(|held| *held != id))
        .take(RECENTS_MAX)
        .collect()
}

/// The order a panel shows the pack in: what this device used most recently
/// first, then everything else in the order it was added — `id` ascending,
/// which is the order `GET /families/mine/pack` gives. `ids` is the pack as
/// held; a recent that is no longer in it is simply not shown, and is not
/// forgotten either, since nothing is lost by remembering an id that is
/// never reused.
pub fn ordered(ids: &[i64], recents: &[i64]) -> Vec<i64> {
    let mut rest: Vec<i64> = ids.to_vec();
    rest.sort_unstable();
    rest.dedup();
    let first: Vec<i64> = recents
        .iter()
        .copied()
        .take(RECENTS_MAX)
        .filter(|id| rest.binary_search(id).is_ok())
        .fold(Vec::new(), |mut seen, id| {
            if !seen.contains(&id) {
                seen.push(id);
            }
            seen
        });
    rest.retain(|id| !first.contains(id));
    first.into_iter().chain(rest).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    fn webp(fourcc: &[u8; 4], flags: u8) -> Vec<u8> {
        let mut bytes = b"RIFF\x24\x00\x00\x00WEBP".to_vec();
        bytes.extend_from_slice(fourcc);
        bytes.extend_from_slice(&[10, 0, 0, 0, flags, 0, 0, 0]);
        bytes
    }

    fn png(chunks: &[(&[u8; 4], usize)]) -> Vec<u8> {
        let mut bytes = PNG_MAGIC.to_vec();
        for (kind, length) in chunks {
            bytes.extend_from_slice(&(*length as u32).to_be_bytes());
            bytes.extend_from_slice(*kind);
            bytes.extend(std::iter::repeat_n(0u8, *length));
            bytes.extend_from_slice(&[0, 0, 0, 0]);
        }
        bytes
    }

    /// A GIF of `pictures` frames, each a 1 × 1 picture with `data` bytes of
    /// image data behind it, with the looping extension in front when
    /// `loops` — the blocks an encoder writes, in the order it writes them.
    fn gif(pictures: usize, loops: bool, data: usize) -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        // 1 × 1, a global table of two colours, and the table.
        bytes.extend_from_slice(&[1, 0, 1, 0, 0x80, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0xFF, 0xFF, 0xFF]);
        if loops {
            bytes.extend_from_slice(&[0x21, 0xFF, 11]);
            bytes.extend_from_slice(b"NETSCAPE2.0");
            bytes.extend_from_slice(&[3, 1, 0, 0, 0]);
        }
        for _ in 0..pictures {
            // A graphic control extension, as every animated frame has.
            bytes.extend_from_slice(&[0x21, 0xF9, 4, 0, 10, 0, 0, 0]);
            bytes.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
            bytes.push(2);
            let mut left = data;
            while left > 0 {
                let run = left.min(255);
                bytes.push(run as u8);
                bytes.extend(std::iter::repeat_n(0x2C, run));
                left -= run;
            }
            bytes.push(0);
        }
        bytes.push(0x3B);
        bytes
    }

    /// A GIF moves when it holds a second picture, wherever in the file
    /// that is — and a still one is a still whatever else it carries.
    #[test]
    fn a_gif_is_animated_by_its_second_picture() {
        assert!(is_gif(b"GIF89a") && is_gif(b"GIF87a"));
        assert!(!is_gif(b"GIF88a") && !is_gif(&PNG_MAGIC));
        assert!(!is_animated(&gif(1, false, 2)), "one picture");
        assert!(is_animated(&gif(2, false, 2)), "two, and no loop block");
        assert!(is_animated(&gif(3, true, 700)));
        assert!(
            !is_animated(&gif(1, true, 2)),
            "a still that says it loops is still a still"
        );
        // Image data full of bytes that LOOK like a picture's first byte is
        // skipped by its length, not searched.
        assert!(!is_animated(&gif(1, false, 5_000)));
        // Cut short inside the first picture: the loop block is all there
        // is to go by, and it is what an animation carries.
        let moving = gif(2, true, 5_000);
        assert!(is_animated(&moving[..2_000]));
        let still = gif(1, false, 5_000);
        assert!(!is_animated(&still[..2_000]));
        // No local table declared by a frame that has one must not run off
        // the end; nor must a header with nothing behind it.
        assert!(!is_animated(b"GIF89a\x01\x00\x01\x00\xF7"));
        assert!(!is_animated(b"GIF89a\x01\x00\x01\x00\x00\x00\x00\x2C"));
        assert!(!is_animated(b"GIF89a\x01\x00\x01\x00\x00\x00\x00\x99\x2C"));
    }

    /// The server's twelve bytes for a WebP and eight for a PNG — and the
    /// four between `RIFF` and `WEBP` are a length, not checked.
    #[test]
    fn the_bytes_say_what_a_picture_is() {
        assert_eq!(type_of(&webp(b"VP8 ", 0)), Some(WEBP));
        assert_eq!(type_of(b"RIFF\xff\xff\xff\xffWEBPVP8L"), Some(WEBP));
        assert_eq!(type_of(&png(&[(b"IHDR", 13)])), Some(PNG));
        assert_eq!(type_of(b"RIFF\x24\x00\x00\x00WAVEfmt "), None, "a WAV");
        assert_eq!(type_of(b"RIFF\x24\x00\x00\x00WEB"), None, "cut short");
        assert_eq!(type_of(&[0xFF, 0xD8, 0xFF, 0xE0]), None, "a JPEG");
        assert_eq!(type_of(b"GIF89a"), None);
        assert_eq!(type_of(&[]), None);
    }

    #[test]
    fn a_sticker_is_a_webp_or_a_png_and_nothing_else() {
        assert!(is_sticker_type("image/webp"));
        assert!(is_sticker_type("image/png"));
        assert!(is_sticker_type("IMAGE/WebP; charset=binary"));
        assert!(!is_sticker_type("image/jpeg"));
        assert!(!is_sticker_type("image/gif"));
        assert!(!is_sticker_type(""));
    }

    /// WebP: the animation bit of a VP8X header, and only there. PNG: an
    /// `acTL` before the first `IDAT`.
    #[test]
    fn animation_is_read_from_the_header() {
        assert!(is_animated(&webp(b"VP8X", 0x02)));
        assert!(is_animated(&webp(b"VP8X", 0x12)), "with alpha beside it");
        assert!(!is_animated(&webp(b"VP8X", 0x10)), "alpha alone");
        assert!(!is_animated(&webp(b"VP8 ", 0x02)), "no extended header");
        assert!(!is_animated(&webp(b"VP8L", 0xFF)));
        assert!(!is_animated(&webp(b"VP8X", 0x02)[..20]), "cut short");

        assert!(is_animated(&png(&[
            (b"IHDR", 13),
            (b"acTL", 8),
            (b"IDAT", 4)
        ])));
        assert!(is_animated(&png(&[
            (b"IHDR", 13),
            (b"iCCP", 300),
            (b"acTL", 8),
            (b"IDAT", 4)
        ])));
        assert!(!is_animated(&png(&[
            (b"IHDR", 13),
            (b"IDAT", 4),
            (b"acTL", 8)
        ])));
        assert!(!is_animated(&png(&[
            (b"IHDR", 13),
            (b"IDAT", 4),
            (b"IEND", 0)
        ])));
        assert!(!is_animated(&png(&[(b"IHDR", 13)])), "no IDAT in sight");
        assert!(!is_animated(&PNG_MAGIC));
        assert!(!is_animated(b"GIF89a"));
        // A length that would run off the end of the address space is not
        // a panic.
        let mut hostile = PNG_MAGIC.to_vec();
        hostile.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        hostile.extend_from_slice(b"tEXt");
        assert!(!is_animated(&hostile));
    }

    /// A finished sticker goes as it is; anything else is made into one;
    /// and an animated one over the ceiling is refused, never flattened.
    #[test]
    fn a_picked_picture_is_taken_remade_or_refused() {
        let max = 524_288;
        let still = webp(b"VP8L", 0);
        let moving = webp(b"VP8X", 0x02);
        let plain = png(&[(b"IHDR", 13), (b"IDAT", 4)]);
        assert_eq!(plan(&still, 40_000, max), Ok(Plan::AsGiven(WEBP)));
        assert_eq!(
            plan(&plain, max, max),
            Ok(Plan::AsGiven(PNG)),
            "at the ceiling"
        );
        assert_eq!(
            plan(&moving, 300_000, max),
            Ok(Plan::AsGiven(WEBP)),
            "animation is stored, never refused and never stripped"
        );
        assert_eq!(
            plan(&plain, max + 1, max),
            Ok(Plan::Remake),
            "a still over it"
        );
        assert_eq!(plan(&still, 2_000_000, max), Ok(Plan::Remake));
        assert_eq!(plan(&moving, max + 1, max), Err(Refusal::TooLarge));
        assert_eq!(
            plan(&[0xFF, 0xD8, 0xFF, 0xE0], 10_000, max),
            Ok(Plan::Remake),
            "a JPEG is not a sticker until it is made one"
        );
        assert_eq!(plan(b"GIF89a", 10_000, max), Ok(Plan::Remake));
        assert_eq!(
            plan(&gif(1, false, 40), 10_000, max),
            Ok(Plan::Remake),
            "a still GIF is a still picture"
        );
        // AN ANIMATED GIF IS NEVER FLATTENED: refused, whatever its size,
        // and not as "too large" — that is not what is wrong with it.
        assert_eq!(plan(&gif(4, true, 40), 10_000, max), Err(Refusal::Animated));
        assert_eq!(
            plan(&gif(2, false, 40), max + 1, max),
            Err(Refusal::Animated)
        );
        // AN ANIMATED PNG: within the ceiling it goes up as it stands, every
        // frame of it; over the ceiling it would have to be remade, which
        // is one frame — so it is refused as an animation, not flattened.
        let apng = png(&[(b"IHDR", 13), (b"acTL", 8), (b"IDAT", 4)]);
        assert_eq!(plan(&apng, 300_000, max), Ok(Plan::AsGiven(PNG)));
        assert_eq!(plan(&apng, max + 1, max), Err(Refusal::Animated));
    }

    /// Whole, inside 512 × 512, proportions kept, never up.
    #[test]
    fn a_remade_sticker_fits_the_box_whole() {
        assert_eq!(fit(2048, 1024), (512, 256));
        assert_eq!(fit(1024, 2048), (256, 512));
        assert_eq!(fit(4000, 4000), (512, 512));
        assert_eq!(fit(512, 512), (512, 512));
        assert_eq!(fit(96, 64), (96, 64), "never scaled up");
        assert_eq!(fit(5120, 3), (512, 1), "a sliver keeps a pixel");
    }

    #[test]
    fn a_label_is_trimmed_and_at_most_sixty_four_characters() {
        assert_eq!(label("  party cat \n"), Ok(Some("party cat".to_string())));
        assert_eq!(label(""), Ok(None));
        assert_eq!(label("   "), Ok(None), "an empty one is no label");
        let longest = "й".repeat(LABEL_MAX);
        assert_eq!(
            label(&longest),
            Ok(Some(longest.clone())),
            "characters, not bytes"
        );
        assert_eq!(label(&format!("{longest}й")), Err(LabelTooLong));
        // SCALAR VALUES, as the server counts them — not UTF-16 units, which
        // would call sixty-four emoji a hundred and twenty-eight.
        let faces = "😀".repeat(LABEL_MAX);
        assert_eq!(label(&faces), Ok(Some(faces.clone())));
        assert_eq!(label(&format!("{faces}😀")), Err(LabelTooLong));
        assert_eq!(
            label(&format!(" {longest} ")),
            Ok(Some(longest)),
            "trimmed first"
        );
    }

    /// Whoever added it, or the owner — and nobody else, including the
    /// item whose adder the tombstone no longer names.
    #[test]
    fn the_adder_or_the_owner_may_remove() {
        assert!(may_remove(Some(7), 7, false));
        assert!(may_remove(Some(9), 7, true), "the owner, of anybody's");
        assert!(!may_remove(Some(9), 7, false));
        assert!(!may_remove(None, 7, false));
        assert!(may_remove(None, 7, true));
    }

    #[test]
    fn a_full_pack_has_no_room() {
        assert!(has_room(0, 200));
        assert!(has_room(199, 200));
        assert!(!has_room(200, 200));
        assert!(!has_room(230, 200), "frozen when the ceiling was lowered");
        assert!(!has_room(0, 0));
    }

    #[test]
    fn a_candidate_has_the_same_size_and_type() {
        assert!(could_be(
            Some(100),
            Some("image/webp"),
            Some(100),
            Some("image/webp")
        ));
        assert!(could_be(
            Some(100),
            Some("image/WEBP"),
            Some(100),
            Some("image/webp")
        ));
        assert!(!could_be(
            Some(100),
            Some("image/webp"),
            Some(101),
            Some("image/webp")
        ));
        assert!(!could_be(
            Some(100),
            Some("image/png"),
            Some(100),
            Some("image/webp")
        ));
        assert!(could_be(
            None,
            Some("image/png"),
            Some(100),
            Some("image/png")
        ));
        assert!(could_be(Some(100), None, Some(100), None));
    }

    /// The last one used comes first, once, and the list does not grow
    /// past its bound.
    #[test]
    fn using_a_sticker_puts_it_first() {
        assert_eq!(used(&[], 5), vec![5]);
        assert_eq!(used(&[3, 5, 8], 5), vec![5, 3, 8]);
        assert_eq!(used(&[3, 5, 8], 9), vec![9, 3, 5, 8]);
        let full: Vec<i64> = (1..=RECENTS_MAX as i64).collect();
        let after = used(&full, 99);
        assert_eq!(after.len(), RECENTS_MAX);
        assert_eq!(after[0], 99);
        assert!(!after.contains(&(RECENTS_MAX as i64)), "the oldest went");
    }

    /// Recents first, then the rest in the order added; a recent that is
    /// no longer in the pack is not drawn.
    #[test]
    fn a_panel_shows_recents_first_and_then_the_order_added() {
        assert_eq!(ordered(&[4, 1, 3, 2], &[]), vec![1, 2, 3, 4]);
        assert_eq!(ordered(&[1, 2, 3, 4], &[3, 1]), vec![3, 1, 2, 4]);
        assert_eq!(ordered(&[1, 2, 4], &[3, 4]), vec![4, 1, 2], "3 was removed");
        assert_eq!(ordered(&[1, 2], &[2, 2, 1]), vec![2, 1], "never twice");
        assert_eq!(ordered(&[], &[1, 2]), Vec::<i64>::new());
    }
}
