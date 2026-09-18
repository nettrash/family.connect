//! Emoji-only messages, the quick reaction set, and the "More reactions…"
//! catalogue.
//!
//! Ported from the Apple app — ios/FamilyConnect/Models/EmojiOnly.swift,
//! Models/EmojiCatalog.swift and `MessagePresentation` in
//! Models/Snapshots.swift. Android embeds the same table, grammar and lists
//! (ui/chat/EmojiOnly.kt, EmojiCatalog.kt, ChatItems.kt). All of them and
//! this file move in lockstep: a body has to draw big and bare on every
//! device or on none, and the picker has to offer the same emoji everywhere.
//!
//! # Scalars, not grapheme clusters, and no Unicode property data
//!
//! The Swift scanner deliberately does NOT use the platform's emoji
//! knowledge — no `Unicode.Scalar.Properties.isEmoji`, no
//! `isEmojiPresentation`, no `Character` (grapheme cluster) counting.
//! Android has no JVM-testable equivalent, and the apps must agree to the
//! scalar, so both walk `String.unicodeScalars` against a hand-rolled table.
//! A Rust `char` is a Unicode scalar value, so `str::chars()` is exactly
//! that sequence and this port walks it with the same table and grammar.
//! That is why the crate has no dependencies: `unicode-segmentation` would
//! not be a faster route to the same answer but a DIFFERENT rule — "🇩🇪🏽"
//! is one grapheme cluster (a skin tone is Grapheme_Cluster_Break=Extend)
//! and two emoji to the scanner, on both apps.
//!
//! The scanner indexes a `Vec` of scalars and never slices the `&str`, so
//! no byte offset can land inside a UTF-8 sequence.

use std::ops::RangeInclusive;

// MARK: - The font ladder

/// Bubble font size for an emoji-only message, by how many emoji it carries:
/// one 96, two 80, three 68, four 56 (points on Apple, sp on Android).
///
/// A pinned cross-platform contract. The Mac does not edit it — it scales
/// it at the call site (see [`display_font_size_for_body`]). `f64` because
/// that is what `CGFloat` is on every platform the Apple app runs on and
/// what a JavaScript number is, so the Mac's arithmetic reproduces bit for
/// bit.
pub const EMOJI_FONT_LADDER: [f64; 4] = [96.0, 80.0, 68.0, 56.0];

/// The body text size the ladder was chosen against: an iPhone's ~17 pt.
pub const LADDER_BODY_SIZE: f64 = 17.0;

/// The Mac's scale for the ladder: its ~13 pt body over the phone's 17
/// (`MacMessageRow.macBodyRatio`).
pub const MAC_BODY_RATIO: f64 = 13.0 / 17.0;

/// The size an emoji-only message draws at, from [`EMOJI_FONT_LADDER`];
/// `None` means "draw it as an ordinary text message" — five or more emoji,
/// any text at all, or nothing but whitespace.
///
/// Swift: `EmojiOnly.displayFontSize(for:)`.
pub fn display_font_size(text: &str) -> Option<f64> {
    let count = emoji_only_count(text)?;
    if (1..=4).contains(&count) {
        Some(EMOJI_FONT_LADDER[count - 1])
    } else {
        None
    }
}

/// [`display_font_size`] scaled for a surface whose body text is
/// `body_size` — the Mac's rule, generalized.
///
/// The ladder's points were chosen against a phone's 17 pt body, so a
/// literal 96 pt glyph beside a smaller body sits far larger, relative to
/// everything around it, than the same message on a phone. Scaling by the
/// ratio of the two body sizes keeps the PROPORTION the ladder was designed
/// for, which is what "the same as iOS" means on a different-sized surface.
/// With `body_size` 13 this is exactly the Mac's `size * macBodyRatio`.
pub fn display_font_size_for_body(text: &str, body_size: f64) -> Option<f64> {
    display_font_size(text).map(|size| size * (body_size / LADDER_BODY_SIZE))
}

// MARK: - The scanner

/// How many emoji an emoji-only message carries; `None` when the text is
/// not emoji-only.
///
/// Whitespace between emoji is allowed and not counted, but whitespace
/// alone is not an emoji message. Five or more is still emoji-only — it
/// just draws at text size, which is [`display_font_size`]'s business, not
/// this count's.
///
/// Swift: `EmojiOnly.count(in:)`, which walks `Array(text.unicodeScalars)`.
pub fn emoji_only_count(text: &str) -> Option<usize> {
    let scalars: Vec<u32> = text.chars().map(u32::from).collect();
    let mut index = 0;
    let mut count = 0;
    while index < scalars.len() {
        if is_whitespace(scalars[index]) {
            index += 1;
            continue;
        }
        let length = emoji_sequence_length(&scalars, index);
        if length == 0 {
            return None;
        }
        index += length;
        count += 1;
    }
    if count > 0 {
        Some(count)
    } else {
        None
    }
}

/// The whitespace the scanner skips: Unicode White_Space, spelled out here
/// rather than delegated to a platform predicate. Swift's scalar properties
/// and java.lang disagree at the edges (U+0085, U+001C–U+001F), so both
/// apps encode this exact list. Rust's `char::is_whitespace` agrees with it
/// today, but agreement by coincidence is not a contract, so the port
/// carries the list too.
fn is_whitespace(value: u32) -> bool {
    matches!(
        value,
        0x09..=0x0D
            | 0x20
            | 0x85
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
    )
}

const ZWJ: u32 = 0x200D;
const TEXT_SELECTOR: u32 = 0xFE0E;
const EMOJI_SELECTOR: u32 = 0xFE0F;
const COMBINING_KEYCAP: u32 = 0x20E3;

/// Length in scalars of the one emoji sequence starting at `start`, or 0
/// when what starts there is not emoji.
fn emoji_sequence_length(scalars: &[u32], start: usize) -> usize {
    let value = scalars[start];

    // A flag is two regional indicators; a lone indicator still renders as
    // an emoji letter symbol, so it counts on its own.
    if is_regional_indicator(value) {
        let paired = scalars
            .get(start + 1)
            .is_some_and(|&next| is_regional_indicator(next));
        return if paired { 2 } else { 1 };
    }

    // Keycaps (5️⃣, #️⃣): digits, # and * are ordinary text unless at least
    // one of the enclosing marks follows.
    if is_keycap_base(value) {
        let mut index = start + 1;
        let mut marked = false;
        if scalars.get(index) == Some(&EMOJI_SELECTOR) {
            index += 1;
            marked = true;
        }
        if scalars.get(index) == Some(&COMBINING_KEYCAP) {
            index += 1;
            marked = true;
        }
        return if marked { index - start } else { 0 };
    }

    if !is_emoji_base(value) {
        return 0;
    }

    let mut index = start + 1;
    while let Some(&next) = scalars.get(index) {
        if next == TEXT_SELECTOR {
            // The author explicitly asked for the text glyph — the message
            // is not an emoji message. Not just this sequence: all of it.
            return 0;
        }
        if next == EMOJI_SELECTOR || is_skin_tone(next) || is_tag(next) {
            index += 1;
            continue;
        }
        if next == ZWJ
            && scalars
                .get(index + 1)
                .is_some_and(|&joined| is_emoji_base(joined))
        {
            index += 2;
            continue;
        }
        break;
    }
    index - start
}

fn is_regional_indicator(value: u32) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&value)
}

fn is_keycap_base(value: u32) -> bool {
    value == 0x23 || value == 0x2A || (0x30..=0x39).contains(&value)
}

fn is_skin_tone(value: u32) -> bool {
    (0x1F3FB..=0x1F3FF).contains(&value)
}

/// Tag characters — the payload of subdivision flags (🏴󠁧󠁢󠁳󠁣󠁴󠁿).
fn is_tag(value: u32) -> bool {
    (0xE0020..=0xE007F).contains(&value)
}

/// Code points that can open an emoji sequence (and follow a ZWJ).
///
/// Extended_Pictographic in broad strokes, NOT generated from Unicode data
/// and so not tied to a Unicode version: bare text-presentation pictographs
/// (☂, ™) count as emoji on purpose — Android draws most of them in colour
/// anyway, and a symbol-only message reads as an emoji message either way.
/// Regional indicators and keycap bases have their own branches above.
/// Copied range for range from the Swift `baseRanges`; do not tidy it.
const BASE_RANGES: [RangeInclusive<u32>; 39] = [
    0x00A9..=0x00A9,
    0x00AE..=0x00AE,
    0x203C..=0x203C,
    0x2049..=0x2049,
    0x2122..=0x2122,
    0x2139..=0x2139,
    0x2194..=0x2199,
    0x21A9..=0x21AA,
    0x231A..=0x231B,
    0x2328..=0x2328,
    0x23CF..=0x23CF,
    0x23E9..=0x23F3,
    0x23F8..=0x23FA,
    0x24C2..=0x24C2,
    0x25AA..=0x25AB,
    0x25B6..=0x25B6,
    0x25C0..=0x25C0,
    0x25FB..=0x25FE,
    0x2600..=0x27BF,
    0x2934..=0x2935,
    0x2B05..=0x2B07,
    0x2B1B..=0x2B1C,
    0x2B50..=0x2B50,
    0x2B55..=0x2B55,
    0x3030..=0x3030,
    0x303D..=0x303D,
    0x3297..=0x3297,
    0x3299..=0x3299,
    0x1F000..=0x1F0FF,
    0x1F170..=0x1F171,
    0x1F17E..=0x1F17F,
    0x1F18E..=0x1F18E,
    0x1F191..=0x1F19A,
    0x1F201..=0x1F202,
    0x1F21A..=0x1F21A,
    0x1F22F..=0x1F22F,
    0x1F232..=0x1F23A,
    0x1F250..=0x1F251,
    0x1F300..=0x1FAFF,
];

fn is_emoji_base(value: u32) -> bool {
    BASE_RANGES.iter().any(|range| range.contains(&value))
}

// MARK: - What we offer to send

/// Upper bound on a reaction emoji, in UTF-8 bytes (protocol.md, "Limits";
/// the server's `MAX_EMOJI_BYTES`). The server trims and then refuses
/// anything longer with `invalid_emoji`, so nothing a picker offers may
/// exceed it.
pub const MAX_REACTION_EMOJI_BYTES: usize = 32;

/// The quick set the long-press capsule and the Mac's React menu offer, in
/// order: ❤️ 👍 👎 😂 😮 😢.
///
/// Client UI only — the server takes any emoji within
/// [`MAX_REACTION_EMOJI_BYTES`], so chips draw whatever arrives; this is
/// just what WE offer to send. Spelled as escapes because the heart carries
/// an invisible U+FE0F: without it, it is a different string, so a
/// different chip from everybody else's heart.
pub const QUICK_REACTIONS: [&str; 6] = [
    "\u{2764}\u{FE0F}",
    "\u{1F44D}",
    "\u{1F44E}",
    "\u{1F602}",
    "\u{1F62E}",
    "\u{1F622}",
];

/// The reaction a double-tap (double-click on the Mac) toggles — the
/// Tapback-heart idiom. Inside [`QUICK_REACTIONS`] so the capsule shows it
/// selected afterwards. Same value on every client.
pub const DOUBLE_TAP_REACTION: &str = "\u{2764}\u{FE0F}";

/// The capsule's items: the quick set, plus the reader's current reaction
/// appended when it is not one of them — so their own reaction is always on
/// show and can be tapped off.
///
/// Swift: `ConversationView.capsuleEmojis(mine:)`; Android builds the same
/// list in `ReactionCapsule`.
pub fn capsule_emojis(mine: Option<&str>) -> Vec<&str> {
    let mut emojis: Vec<&str> = QUICK_REACTIONS.to_vec();
    if let Some(mine) = mine {
        if !emojis.contains(&mine) {
            emojis.push(mine);
        }
    }
    emojis
}

// MARK: - The catalogue

/// One section of the "More reactions…" picker: a canonical name and its
/// emoji, in canonical order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmojiCategory {
    /// The canonical cross-platform name — an identifier, shared verbatim
    /// with iOS and Android. The apps draw a localized header for it; until
    /// the web localizes, this English name is also the header.
    pub name: &'static str,
    /// The section's emoji. Unique within the section (a grid keyed by
    /// emoji needs that), though 💫 and 🎂 each appear in two sections.
    pub emoji: &'static [&'static str],
}

/// The full picker behind "More reactions…": eight sections, 771 emoji.
///
/// Transcribed by script from ios/FamilyConnect/Models/EmojiCatalog.swift,
/// which is byte-identical to Android's EmojiCatalog.kt — same sections,
/// same order, same bytes (31 entries carry an invisible U+FE0F). Do not
/// add, remove or reorder anything here without changing both apps too;
/// `catalog_matches_swift_bytes` fails on any byte that drifts. Every entry
/// is skin-tone-neutral and at most 7 bytes of UTF-8.
///
/// The source JSON listed 💢 twice in Hearts; the apps drop the second, and
/// so does this.
/// The eight, in the apps' order. A `name` is the KEY it is said by (a
/// static cannot look a translation up).
pub static EMOJI_CATALOG: [EmojiCategory; 8] = [
    EmojiCategory {
        name: "Smileys",
        emoji: &[
            "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🤣", "🙂", "😉", "😊", "😇", "🥰", "😍",
            "🤩", "😘", "😗", "😚", "😙", "😋", "😛", "😜", "🤪", "😝", "🤑", "🤗", "🤭", "🤫",
            "🤔", "🤐", "🤨", "😐", "😑", "😶", "😏", "😒", "🙄", "😬", "🤥", "😌", "😔", "😪",
            "🤤", "😴", "😷", "🤒", "🤕", "🤢", "🤮", "🤧", "🥵", "🥶", "🥴", "😵", "🤯", "🤠",
            "🥳", "😎", "🤓", "🧐", "😕", "😟", "🙁", "😮", "😯", "😲", "😳", "🥺", "😦", "😧",
            "😨", "😰", "😥", "😢", "😭", "😱", "😖", "😣", "😞", "😓", "😩", "😫", "🥱", "😤",
            "😡", "😠", "🤬", "😈", "👿", "💀", "🤡", "👻", "👽", "🤖", "💩",
        ],
    },
    EmojiCategory {
        name: "Gestures",
        emoji: &[
            "👋", "🤚", "🖐", "✋", "🖖", "👌", "🤏", "✌️", "🤞", "🤟", "🤘", "🤙", "👈", "👉",
            "👆", "🖕", "👇", "☝️", "👍", "👎", "✊", "👊", "🤛", "🤜", "👏", "🙌", "👐", "🤲",
            "🤝", "🙏", "✍️", "💅", "🤳", "💪", "🦾", "👂", "👃", "🧠", "👀", "👁", "👅", "👄",
            "💋",
        ],
    },
    EmojiCategory {
        name: "Hearts",
        emoji: &[
            "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💔", "❣️", "💕", "💞", "💓",
            "💗", "💖", "💘", "💝", "💟", "♥️", "💌", "💤", "💢", "💥", "💦", "💨", "💫", "⭐",
            "🌟", "✨", "⚡", "🔥", "💯", "🎉", "🎊", "🎈", "🎂", "🎁", "🏆", "🥇", "🥈", "🥉",
            "🏅",
        ],
    },
    EmojiCategory {
        name: "Animals & Nature",
        emoji: &[
            "🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮", "🐷", "🐸",
            "🐵", "🙈", "🙉", "🙊", "🐔", "🐧", "🐦", "🐤", "🦆", "🦅", "🦉", "🦇", "🐺", "🐗",
            "🐴", "🦄", "🐝", "🐛", "🦋", "🐌", "🐞", "🐜", "🕷", "🐢", "🐍", "🦎", "🐙", "🦑",
            "🦀", "🐡", "🐠", "🐟", "🐬", "🐳", "🐋", "🦈", "🐊", "🐘", "🦏", "🐪", "🦒", "🦘",
            "🐃", "🐎", "🐖", "🐏", "🐑", "🦙", "🐐", "🦌", "🐕", "🐩", "🐈", "🐇", "🐿", "🦔",
            "🌵", "🎄", "🌲", "🌳", "🌴", "🌱", "🌿", "☘️", "🍀", "🎍", "🍁", "🍄", "🌾", "💐",
            "🌷", "🌹", "🥀", "🌺", "🌸", "🌼", "🌻", "🌞", "🌝", "🌚", "🌙", "🌎", "🪐", "💫",
            "🌈", "☀️", "⛅", "☁️", "🌧", "⛈", "❄️", "⛄", "🌊",
        ],
    },
    EmojiCategory {
        name: "Food & Drink",
        emoji: &[
            "🍏", "🍎", "🍐", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🍈", "🍒", "🍑", "🥭", "🍍",
            "🥥", "🥝", "🍅", "🍆", "🥑", "🥦", "🥬", "🥒", "🌶", "🌽", "🥕", "🧄", "🧅", "🥔",
            "🍠", "🥐", "🥯", "🍞", "🥖", "🥨", "🧀", "🥚", "🍳", "🧈", "🥞", "🧇", "🥓", "🥩",
            "🍗", "🍖", "🌭", "🍔", "🍟", "🍕", "🥪", "🥙", "🧆", "🌮", "🌯", "🥗", "🥘", "🍝",
            "🍜", "🍲", "🍛", "🍣", "🍱", "🥟", "🦪", "🍤", "🍙", "🍚", "🍘", "🍥", "🥮", "🍢",
            "🍡", "🍧", "🍨", "🍦", "🥧", "🧁", "🍰", "🎂", "🍮", "🍭", "🍬", "🍫", "🍿", "🍩",
            "🍪", "🌰", "🥜", "🍯", "🥛", "🍼", "☕", "🍵", "🧃", "🥤", "🍶", "🍺", "🍻", "🥂",
            "🍷", "🥃", "🍸", "🍹", "🧉", "🍾", "🧊",
        ],
    },
    EmojiCategory {
        name: "Activities",
        emoji: &[
            "⚽", "🏀", "🏈", "⚾", "🥎", "🎾", "🏐", "🏉", "🥏", "🎱", "🪀", "🏓", "🏸", "🏒",
            "🏑", "🥍", "🏏", "🥅", "⛳", "🪁", "🏹", "🎣", "🤿", "🥊", "🥋", "🎽", "🛹", "🛷",
            "⛸", "🥌", "🎿", "⛷", "🏂", "🏋️", "🤸", "⛹️", "🤺", "🤾", "🏌️", "🏇", "🧘", "🏄", "🏊",
            "🤽", "🚣", "🧗", "🚴", "🚵", "🎪", "🎭", "🎨", "🎬", "🎤", "🎧", "🎼", "🎹", "🥁",
            "🎷", "🎺", "🎸", "🪕", "🎻", "🎲", "♟", "🎯", "🎳", "🎮", "🎰", "🧩",
        ],
    },
    EmojiCategory {
        name: "Travel & Places",
        emoji: &[
            "🚗", "🚕", "🚙", "🚌", "🚎", "🏎", "🚓", "🚑", "🚒", "🚐", "🚚", "🚛", "🚜", "🛴",
            "🚲", "🛵", "🏍", "🚨", "🚔", "🚍", "🚘", "🚖", "🚡", "🚠", "🚟", "🚃", "🚋", "🚞",
            "🚝", "🚄", "🚅", "🚈", "🚂", "🚆", "🚇", "🚊", "🚉", "✈️", "🛫", "🛬", "🛩", "💺", "🛰",
            "🚀", "🛸", "🚁", "🛶", "⛵", "🚤", "🛥", "🛳", "⛴", "🚢", "⚓", "⛽", "🚧", "🚦", "🚥",
            "🚏", "🗺", "🗿", "🗽", "🗼", "🏰", "🏯", "🏟", "🎡", "🎢", "🎠", "⛲", "⛱", "🏖", "🏝",
            "🏜", "🌋", "⛰", "🏔", "🗻", "🏕", "⛺", "🏠", "🏡", "🏘", "🏚", "🏗", "🏭", "🏢", "🏬",
            "🏣", "🏤", "🏥", "🏦", "🏨", "🏪", "🏫", "🏩", "💒", "🏛", "⛪", "🕌", "🕍", "🛕",
            "🕋", "⛩", "🌅", "🌄", "🌠", "🎇", "🎆", "🌇", "🌆", "🏙", "🌃", "🌌", "🌉", "🌁",
        ],
    },
    EmojiCategory {
        name: "Objects & Symbols",
        emoji: &[
            "⌚", "📱", "💻", "⌨️", "🖥", "🖨", "🖱", "🕹", "🗜", "💽", "💾", "💿", "📀", "📼", "📷",
            "📸", "📹", "🎥", "📽", "🎞", "📞", "☎️", "📟", "📠", "📺", "📻", "🎙", "⏰", "⌛", "⏳",
            "📡", "🔋", "🔌", "💡", "🔦", "🕯", "🧯", "🛢", "💸", "💵", "💰", "💳", "💎", "⚖️", "🧰",
            "🔧", "🔨", "⚒", "🛠", "⛏", "🔩", "⚙️", "🧲", "🔫", "💣", "🧨", "🪓", "🔪", "🗡", "🛡",
            "🚬", "⚰️", "⚱️", "🏺", "🔮", "📿", "🧿", "💈", "⚗️", "🔭", "🔬", "🕳", "💊", "💉",
            "🧬", "🦠", "🧫", "🧪", "🌡", "🧹", "🧺", "🧻", "🚽", "🚰", "🚿", "🛁", "🧼", "🪒",
            "🧽", "🧴", "🛎", "🔑", "🗝", "🚪", "🪑", "🛋", "🛏", "🧸", "🖼", "🛍", "🛒", "🎀", "🎏",
            "🎗", "📯", "📦", "📫", "📮", "📜", "📃", "📄", "📊", "📈", "📉", "🗒", "📆", "📅", "📇",
            "🗃", "🗳", "🗄", "📋", "📁", "📂", "🗞", "📰", "📓", "📔", "📒", "📕", "📗", "📘", "📙",
            "📚", "📖", "🔖", "🧷", "🔗", "📎", "📐", "📏", "🧮", "📌", "📍", "✂️", "🖊", "🖋", "✒️",
            "🖌", "🖍", "📝", "✏️", "🔍", "🔎", "🔒", "🔓", "❗", "❓", "‼️", "⁉️", "✅", "❌", "⭕",
            "🚫", "💬", "💭", "🗯", "♻️", "🔱", "📣", "📢", "🔔", "🔕", "🎵", "🎶", "➕", "➖",
            "➗", "✖️", "♾", "💲", "™️", "©️", "®️", "🔴", "🟠", "🟡", "🟢", "🔵", "🟣", "⚫",
            "⚪", "🟤",
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - Ported from EmojiOnlyTests.swift, vector for vector
    //
    // Inputs are spelled as escapes wherever the Swift literal carries an
    // invisible scalar (ZWJ, U+FE0F, tags), so what is tested is visible.

    /// Swift: fontLadder — "the font ladder: one emoji biggest, five back to text".
    #[test]
    fn font_ladder() {
        assert_eq!(display_font_size("😀"), Some(96.0));
        assert_eq!(display_font_size("😀😂"), Some(80.0));
        assert_eq!(display_font_size("😀😂🥳"), Some(68.0));
        assert_eq!(display_font_size("😀😂🥳😎"), Some(56.0));
        assert_eq!(display_font_size("😀😂🥳😎😍"), None);
        // Five IS still emoji-only — it just renders at text size.
        assert_eq!(emoji_only_count("😀😂🥳😎😍"), Some(5));
    }

    /// Swift: notEmojiOnly — "text, mixed content and empty input are not emoji-only".
    #[test]
    fn not_emoji_only() {
        assert_eq!(emoji_only_count("hi"), None);
        assert_eq!(emoji_only_count("hi 😀"), None);
        assert_eq!(emoji_only_count("😀 hi"), None);
        assert_eq!(emoji_only_count("😀!"), None);
        assert_eq!(emoji_only_count("a😀"), None);
        assert_eq!(emoji_only_count(""), None);
        assert_eq!(emoji_only_count("  \n "), None);
        // Bare digits, # and * are text; only the keycap marks make them
        // emoji.
        assert_eq!(emoji_only_count("1"), None);
        assert_eq!(emoji_only_count("123"), None);
        assert_eq!(emoji_only_count("#"), None);
    }

    /// Swift: whitespaceBetween — "whitespace between emoji is allowed and not counted".
    #[test]
    fn whitespace_between() {
        assert_eq!(emoji_only_count("😀 😀"), Some(2));
        assert_eq!(emoji_only_count("😀\n😀"), Some(2));
        assert_eq!(emoji_only_count(" 😀 "), Some(1));
        // The scanner's own White_Space table, not a platform predicate:
        // these two are where Swift and java.lang disagree, so they pin
        // cross-platform agreement — NEL is whitespace, the C0 separators
        // are not.
        assert_eq!(emoji_only_count("😀\u{85}😀"), Some(2));
        assert_eq!(emoji_only_count("😀\u{1C}😀"), None);
    }

    /// Swift: multiScalarSequences — "multi-scalar sequences count as one emoji".
    #[test]
    fn multi_scalar_sequences() {
        assert_eq!(emoji_only_count("\u{2764}\u{FE0F}"), Some(1)); // ❤️ VS16
        assert_eq!(emoji_only_count("\u{2600}\u{FE0F}"), Some(1)); // ☀️ BMP + VS16
        assert_eq!(emoji_only_count("\u{2600}"), Some(1)); // ☀ bare BMP pictograph, by design
        assert_eq!(emoji_only_count("\u{1F44D}\u{1F3FD}"), Some(1)); // 👍🏽 skin tone
        assert_eq!(
            emoji_only_count("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}"),
            Some(1)
        ); // 👨‍👩‍👧‍👦 ZWJ family
        assert_eq!(
            emoji_only_count("\u{1F46E}\u{200D}\u{2640}\u{FE0F}"),
            Some(1)
        ); // 👮‍♀️ ZWJ + BMP + VS16
        assert_eq!(
            emoji_only_count("\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}"),
            Some(1)
        ); // 🏳️‍🌈 flag + ZWJ
        assert_eq!(emoji_only_count("\u{1F1E9}\u{1F1EA}"), Some(1)); // 🇩🇪 regional pair
        assert_eq!(
            emoji_only_count("\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}"),
            Some(1)
        ); // 🏴󠁧󠁢󠁳󠁣󠁴󠁿 tag sequence
        assert_eq!(emoji_only_count("1\u{FE0F}\u{20E3}"), Some(1)); // 1️⃣ keycap
        assert_eq!(
            emoji_only_count("\u{2764}\u{FE0F}\u{2764}\u{FE0F}"),
            Some(2)
        ); // ❤️❤️
        assert_eq!(
            emoji_only_count("\u{1F1E9}\u{1F1EA}\u{1F1EB}\u{1F1F7}"),
            Some(2)
        ); // 🇩🇪🇫🇷
        assert_eq!(
            emoji_only_count("\u{1F44D}\u{1F3FD}\u{1F44D}\u{1F3FF}"),
            Some(2)
        ); // 👍🏽👍🏿
    }

    /// Swift: textSelectorOptsOut — "an explicit text selector opts the message out".
    #[test]
    fn text_selector_opts_out() {
        // U+2602 U+FE0E — the author asked for the text glyph.
        assert_eq!(emoji_only_count("\u{2602}\u{FE0E}"), None);
    }

    /// Swift: catalogSweep — "every catalog entry counts as exactly one emoji".
    #[test]
    fn catalog_sweep() {
        for category in &EMOJI_CATALOG {
            for emoji in category.emoji {
                assert_eq!(
                    emoji_only_count(emoji),
                    Some(1),
                    "{}: {emoji} → {:?}",
                    category.name,
                    emoji_only_count(emoji)
                );
            }
        }
    }

    /// Swift: quickReactionsSweep — "the quick reactions count as exactly one emoji".
    #[test]
    fn quick_reactions_sweep() {
        for emoji in QUICK_REACTIONS {
            assert_eq!(emoji_only_count(emoji), Some(1), "{emoji}");
        }
    }

    // MARK: - Ported from EmojiCatalogTests.swift

    /// Swift: categoriesNonEmpty — "the catalog has categories and none is empty".
    #[test]
    fn categories_non_empty() {
        assert!(!EMOJI_CATALOG.is_empty());
        for category in &EMOJI_CATALOG {
            assert!(!category.emoji.is_empty(), "{} is empty", category.name);
        }
    }

    /// Swift: entriesNonEmpty — "every entry is a non-empty string".
    #[test]
    fn entries_non_empty() {
        for category in &EMOJI_CATALOG {
            assert!(
                category.emoji.iter().all(|emoji| !emoji.is_empty()),
                "{} has an empty entry",
                category.name
            );
        }
    }

    /// Swift: entriesUniqueWithinCategory — "entries are unique within their
    /// category (ForEach identity)".
    ///
    /// Swift's `Set<String>` compares by canonical equivalence and this by
    /// bytes. They agree here because no entry changes under NFC or NFD
    /// (checked against Unicode 15.1 when this was ported), so no two
    /// entries can be canonically equivalent without being identical.
    #[test]
    fn entries_unique_within_category() {
        for category in &EMOJI_CATALOG {
            let mut seen = std::collections::HashSet::new();
            let duplicates: Vec<&str> = category
                .emoji
                .iter()
                .copied()
                .filter(|emoji| !seen.insert(*emoji))
                .collect();
            assert!(
                duplicates.is_empty(),
                "{} repeats {duplicates:?}",
                category.name
            );
        }
    }

    /// Swift: categoryNamesUnique — "category names are unique (section identity)".
    #[test]
    fn category_names_unique() {
        let names: Vec<&str> = EMOJI_CATALOG.iter().map(|category| category.name).collect();
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len());
    }

    /// Swift: entriesFitServerLimit — "every entry fits the server's 32-byte UTF-8 limit".
    #[test]
    fn entries_fit_server_limit() {
        for category in &EMOJI_CATALOG {
            for emoji in category.emoji {
                assert!(
                    emoji.len() <= MAX_REACTION_EMOJI_BYTES,
                    "{}: {emoji} is {} bytes",
                    category.name,
                    emoji.len()
                );
            }
        }
    }

    /// Swift: quickReactionsFitServerLimit — "the quick set is also within the server limit".
    #[test]
    fn quick_reactions_fit_server_limit() {
        for emoji in QUICK_REACTIONS {
            assert!(emoji.len() <= MAX_REACTION_EMOJI_BYTES);
        }
    }

    /// Swift: doubleTapReactionValid — "the double-tap reaction is a quick
    /// reaction within the limit".
    #[test]
    fn double_tap_reaction_valid() {
        assert!(DOUBLE_TAP_REACTION.len() <= MAX_REACTION_EMOJI_BYTES);
        // Inside the quick set so the capsule shows it selected after a
        // double-tap. Android pins the same value.
        assert!(QUICK_REACTIONS.contains(&DOUBLE_TAP_REACTION));
    }

    // MARK: - Beyond the Swift suite: the catalogue and the quick set

    /// Android's catalogHasTheCanonicalCategoriesInOrder: the names are an
    /// identifier shared by three apps, so their order is pinned too.
    #[test]
    fn catalog_has_the_canonical_categories_in_order() {
        let names: Vec<&str> = EMOJI_CATALOG.iter().map(|category| category.name).collect();
        assert_eq!(
            names,
            [
                "Smileys",
                "Gestures",
                "Hearts",
                "Animals & Nature",
                "Food & Drink",
                "Activities",
                "Travel & Places",
                "Objects & Symbols",
            ]
        );
    }

    /// The catalogue is byte-for-byte the Swift one. The fingerprint was
    /// computed from EmojiCatalog.swift's literals (FNV-1a 64 over each name,
    /// NUL, each entry followed by NUL, then 0x01 per section) — so an
    /// editor that quietly drops a U+FE0F, or a hand edit on one side only,
    /// fails here rather than as a different picker on the web.
    #[test]
    fn catalog_matches_swift_bytes() {
        let counts: Vec<usize> = EMOJI_CATALOG
            .iter()
            .map(|category| category.emoji.len())
            .collect();
        assert_eq!(counts, [95, 43, 43, 107, 105, 69, 116, 193]);

        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for &byte in bytes {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        for category in &EMOJI_CATALOG {
            feed(category.name.as_bytes());
            feed(&[0]);
            for emoji in category.emoji {
                feed(emoji.as_bytes());
                feed(&[0]);
            }
            feed(&[1]);
        }
        assert_eq!(hash, 0xaa64_dcd7_12cf_9db0);
    }

    /// EmojiCatalog.swift promises skin-tone-neutral entries; its own tests
    /// never check that. A toned entry would make "your" 👍🏽 a different
    /// chip from everybody's 👍.
    #[test]
    fn catalog_entries_carry_no_skin_tone() {
        for category in &EMOJI_CATALOG {
            for emoji in category.emoji {
                assert!(
                    !emoji.chars().any(|scalar| is_skin_tone(u32::from(scalar))),
                    "{}: {emoji}",
                    category.name
                );
            }
        }
    }

    /// The quick set to the scalar — the heart is U+2764 U+FE0F — as in
    /// Snapshots.swift and ChatItems.kt.
    #[test]
    fn quick_reactions_are_the_apps_scalars() {
        let scalars: Vec<Vec<u32>> = QUICK_REACTIONS
            .iter()
            .map(|emoji| emoji.chars().map(u32::from).collect())
            .collect();
        assert_eq!(
            scalars,
            [
                vec![0x2764, 0xFE0F],
                vec![0x1F44D],
                vec![0x1F44E],
                vec![0x1F602],
                vec![0x1F62E],
                vec![0x1F622],
            ]
        );
        assert_eq!(DOUBLE_TAP_REACTION, "\u{2764}\u{FE0F}");
    }

    #[test]
    fn capsule_is_the_quick_set_plus_my_other_reaction() {
        assert_eq!(capsule_emojis(None), QUICK_REACTIONS.to_vec());
        // Mine is already offered: not twice.
        assert_eq!(capsule_emojis(Some("\u{1F44D}")), QUICK_REACTIONS.to_vec());
        // Mine is not: appended LAST, so the quick set never shifts.
        let with_party = capsule_emojis(Some("🎉"));
        assert_eq!(with_party.len(), 7);
        assert_eq!(&with_party[..6], &QUICK_REACTIONS[..]);
        assert_eq!(with_party[6], "🎉");
        // A bare U+2764 is not the quick heart: it is its own reaction, so
        // it gets its own slot to be tapped off from.
        assert_eq!(capsule_emojis(Some("\u{2764}")).last(), Some(&"\u{2764}"));
    }

    // MARK: - Beyond the Swift suite: the grammar

    #[test]
    fn zwj_sequences_are_one_emoji() {
        assert_eq!(
            emoji_only_count("\u{1F9D1}\u{200D}\u{1F9D1}\u{200D}\u{1F9D2}"),
            Some(1)
        ); // 🧑‍🧑‍🧒
        assert_eq!(
            emoji_only_count("\u{1F469}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F48B}\u{200D}\u{1F468}"),
            Some(1)
        ); // 👩‍❤️‍💋‍👨
        assert_eq!(
            emoji_only_count("\u{1FAF1}\u{1F3FB}\u{200D}\u{1FAF2}\u{1F3FF}"),
            Some(1)
        ); // 🫱🏻‍🫲🏿
        assert_eq!(
            emoji_only_count("\u{2764}\u{FE0F}\u{200D}\u{1F525}"),
            Some(1)
        ); // ❤️‍🔥
        assert_eq!(
            emoji_only_count("\u{1F3F4}\u{200D}\u{2620}\u{FE0F}"),
            Some(1)
        ); // 🏴‍☠️
        assert_eq!(
            emoji_only_count("\u{1F3F3}\u{FE0F}\u{200D}\u{26A7}\u{FE0F}"),
            Some(1)
        ); // 🏳️‍⚧️
           // Two families side by side are two.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(emoji_only_count(&format!("{family}{family}")), Some(2));
        // A joiner that joins nothing emoji is not part of the emoji — and
        // is not whitespace either, so the message is not emoji-only.
        assert_eq!(emoji_only_count("😀\u{200D}"), None);
        assert_eq!(emoji_only_count("😀\u{200D}a"), None);
        assert_eq!(emoji_only_count("\u{200D}😀"), None);
    }

    #[test]
    fn skin_tones() {
        assert_eq!(emoji_only_count("\u{1F44B}\u{1F3FF}"), Some(1)); // 👋🏿
                                                                     // A bare modifier is a swatch, and a swatch is an emoji.
        assert_eq!(emoji_only_count("\u{1F3FD}"), Some(1));
        // Modifiers after a base are consumed, however many.
        assert_eq!(emoji_only_count("\u{1F44D}\u{1F3FD}\u{1F3FF}"), Some(1));
        // After a flag they are not (the flag branch takes exactly two), so
        // this is two emoji — though it is ONE grapheme cluster, which is
        // why the port does not count clusters.
        assert_eq!(emoji_only_count("\u{1F1E9}\u{1F1EA}\u{1F3FD}"), Some(2));
    }

    #[test]
    fn flags() {
        assert_eq!(emoji_only_count("\u{1F1FA}\u{1F1E6}"), Some(1)); // 🇺🇦
                                                                     // Indicators pair left to right; the odd one out counts alone.
        assert_eq!(emoji_only_count("\u{1F1E9}\u{1F1EA}\u{1F1EB}"), Some(2));
        assert_eq!(emoji_only_count("\u{1F1EB}"), Some(1));
        // England, Wales.
        assert_eq!(
            emoji_only_count("\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}"),
            Some(1)
        );
        assert_eq!(
            emoji_only_count("\u{1F3F4}\u{E0067}\u{E0062}\u{E0077}\u{E006C}\u{E0073}\u{E007F}"),
            Some(1)
        );
    }

    #[test]
    fn keycaps() {
        assert_eq!(emoji_only_count("#\u{FE0F}\u{20E3}"), Some(1)); // #️⃣
        assert_eq!(emoji_only_count("*\u{FE0F}\u{20E3}"), Some(1)); // *️⃣
                                                                    // Either mark alone is enough.
        assert_eq!(emoji_only_count("1\u{20E3}"), Some(1));
        assert_eq!(emoji_only_count("1\u{FE0F}"), Some(1));
        assert_eq!(emoji_only_count("\u{1F51F}"), Some(1)); // 🔟 is a pictograph, not a keycap
                                                            // A text selector is not a mark, and the keycap after it is then
                                                            // orphaned.
        assert_eq!(emoji_only_count("#\u{FE0E}\u{20E3}"), None);
        // One keycap each; the digit after the last one is plain text.
        assert_eq!(
            emoji_only_count("1\u{FE0F}\u{20E3}2\u{FE0F}\u{20E3}"),
            Some(2)
        );
        assert_eq!(emoji_only_count("1\u{FE0F}\u{20E3}2"), None);
        assert_eq!(emoji_only_count("10"), None);
        // A keycap takes no modifiers after it.
        assert_eq!(emoji_only_count("1\u{FE0F}\u{20E3}\u{FE0F}"), None);
    }

    #[test]
    fn text_versus_emoji_presentation() {
        // Bare text-presentation pictographs count, by design.
        assert_eq!(emoji_only_count("\u{2764}"), Some(1)); // ❤ without VS16
        assert_eq!(emoji_only_count("\u{2764}\u{FE0F}"), Some(1)); // ❤️
        assert_eq!(emoji_only_count("\u{2122}"), Some(1)); // ™
        assert_eq!(emoji_only_count("\u{A9}"), Some(1)); // ©
        assert_eq!(emoji_only_count("\u{2602}"), Some(1)); // ☂
                                                           // An explicit text selector opts out the WHOLE message, not just its
                                                           // own emoji.
        assert_eq!(emoji_only_count("\u{2764}\u{FE0E}"), None);
        assert_eq!(emoji_only_count("\u{2764}\u{FE0F} \u{2602}\u{FE0E}"), None);
        assert_eq!(emoji_only_count("\u{1F468}\u{200D}\u{1F469}\u{FE0E}"), None);
        // Swift's `displayFontSize` gives no separate treatment to either.
        assert_eq!(
            display_font_size("\u{2764}"),
            display_font_size("\u{2764}\u{FE0F}")
        );
    }

    #[test]
    fn mixed_emoji_and_text() {
        assert_eq!(emoji_only_count("Привет 😀"), None);
        assert_eq!(emoji_only_count("😀 Привет"), None);
        assert_eq!(emoji_only_count("日本😀"), None);
        assert_eq!(emoji_only_count("😀é"), None);
        assert_eq!(emoji_only_count("😀e\u{301}"), None);
        // A combining mark does not extend an emoji — only the selectors,
        // tones, tags and joins do.
        assert_eq!(emoji_only_count("😀\u{301}"), None);
        assert_eq!(emoji_only_count("\u{301}"), None);
        // Nor is a zero-width space whitespace: it is not White_Space.
        assert_eq!(emoji_only_count("😀\u{200B}😀"), None);
        assert_eq!(emoji_only_count("😀\u{3000}😀\u{2028}"), Some(2));
    }

    /// Every scalar there is, alone and between multi-byte neighbours: the
    /// scanner must answer, never panic. This is the class of bug the
    /// project has shipped before (a `&str` cut inside "Привет"); walking a
    /// `Vec` of scalars makes it impossible, and this keeps it that way.
    #[test]
    fn every_scalar_is_scanned_without_panicking() {
        for value in 0..=0x10FFFF_u32 {
            let Some(scalar) = char::from_u32(value) else {
                continue;
            };
            let alone = scalar.to_string();
            let count = emoji_only_count(&alone);
            assert!(
                count.is_none() || count == Some(1),
                "U+{value:04X} → {count:?}"
            );
            if value % 97 == 0 {
                emoji_only_count(&format!("Пр{scalar}日😀\u{200D}{scalar}\u{E007F}"));
            }
        }
    }

    /// The whitespace table is exactly Unicode White_Space, as Rust's
    /// standard library knows it today — the same list both apps encode.
    #[test]
    fn whitespace_table_is_unicode_white_space() {
        for value in 0..=0x10FFFF_u32 {
            if let Some(scalar) = char::from_u32(value) {
                assert_eq!(
                    is_whitespace(value),
                    scalar.is_whitespace(),
                    "U+{value:04X}"
                );
            }
        }
    }

    /// The Mac draws the ladder at 13/17 of its size; the web will use its
    /// own body size the same way. At 13 the arithmetic is the Mac's to the
    /// last bit.
    #[test]
    fn ladder_scales_with_the_body_size() {
        for text in ["😀", "😀😂", "😀😂🥳", "😀😂🥳😎"] {
            let mac = display_font_size(text).map(|size| size * MAC_BODY_RATIO);
            assert_eq!(display_font_size_for_body(text, 13.0), mac);
            assert_eq!(
                display_font_size_for_body(text, LADDER_BODY_SIZE),
                display_font_size(text)
            );
        }
        assert_eq!(display_font_size_for_body("hi", 13.0), None);
        assert_eq!(display_font_size_for_body("😀😂🥳😎😍", 13.0), None);
    }

    // MARK: - Answers from the Swift code itself

    /// Every boundary of every table range — one below, first, last, one
    /// above — as (scalar, count alone, count followed by U+FE0F). The
    /// expected values were not written by hand: they are what the app's own
    /// EmojiOnly.swift answered when compiled and run on 2026-09-10.
    const SWIFT_BOUNDARIES: &[(u32, Option<usize>, Option<usize>)] = &[
        (0x22, None, None),
        (0x23, None, Some(1)),
        (0x24, None, None),
        (0x29, None, None),
        (0x2A, None, Some(1)),
        (0x2B, None, None),
        (0x2F, None, None),
        (0x30, None, Some(1)),
        (0x39, None, Some(1)),
        (0x3A, None, None),
        (0xA8, None, None),
        (0xA9, Some(1), Some(1)),
        (0xAA, None, None),
        (0xAD, None, None),
        (0xAE, Some(1), Some(1)),
        (0xAF, None, None),
        (0x203B, None, None),
        (0x203C, Some(1), Some(1)),
        (0x203D, None, None),
        (0x2048, None, None),
        (0x2049, Some(1), Some(1)),
        (0x204A, None, None),
        (0x2121, None, None),
        (0x2122, Some(1), Some(1)),
        (0x2123, None, None),
        (0x2138, None, None),
        (0x2139, Some(1), Some(1)),
        (0x213A, None, None),
        (0x2193, None, None),
        (0x2194, Some(1), Some(1)),
        (0x2199, Some(1), Some(1)),
        (0x219A, None, None),
        (0x21A8, None, None),
        (0x21A9, Some(1), Some(1)),
        (0x21AA, Some(1), Some(1)),
        (0x21AB, None, None),
        (0x2319, None, None),
        (0x231A, Some(1), Some(1)),
        (0x231B, Some(1), Some(1)),
        (0x231C, None, None),
        (0x2327, None, None),
        (0x2328, Some(1), Some(1)),
        (0x2329, None, None),
        (0x23CE, None, None),
        (0x23CF, Some(1), Some(1)),
        (0x23D0, None, None),
        (0x23E8, None, None),
        (0x23E9, Some(1), Some(1)),
        (0x23F3, Some(1), Some(1)),
        (0x23F4, None, None),
        (0x23F7, None, None),
        (0x23F8, Some(1), Some(1)),
        (0x23FA, Some(1), Some(1)),
        (0x23FB, None, None),
        (0x24C1, None, None),
        (0x24C2, Some(1), Some(1)),
        (0x24C3, None, None),
        (0x25A9, None, None),
        (0x25AA, Some(1), Some(1)),
        (0x25AB, Some(1), Some(1)),
        (0x25AC, None, None),
        (0x25B5, None, None),
        (0x25B6, Some(1), Some(1)),
        (0x25B7, None, None),
        (0x25BF, None, None),
        (0x25C0, Some(1), Some(1)),
        (0x25C1, None, None),
        (0x25FA, None, None),
        (0x25FB, Some(1), Some(1)),
        (0x25FE, Some(1), Some(1)),
        (0x25FF, None, None),
        (0x2600, Some(1), Some(1)),
        (0x27BF, Some(1), Some(1)),
        (0x27C0, None, None),
        (0x2933, None, None),
        (0x2934, Some(1), Some(1)),
        (0x2935, Some(1), Some(1)),
        (0x2936, None, None),
        (0x2B04, None, None),
        (0x2B05, Some(1), Some(1)),
        (0x2B07, Some(1), Some(1)),
        (0x2B08, None, None),
        (0x2B1A, None, None),
        (0x2B1B, Some(1), Some(1)),
        (0x2B1C, Some(1), Some(1)),
        (0x2B1D, None, None),
        (0x2B4F, None, None),
        (0x2B50, Some(1), Some(1)),
        (0x2B51, None, None),
        (0x2B54, None, None),
        (0x2B55, Some(1), Some(1)),
        (0x2B56, None, None),
        (0x302F, None, None),
        (0x3030, Some(1), Some(1)),
        (0x3031, None, None),
        (0x303C, None, None),
        (0x303D, Some(1), Some(1)),
        (0x303E, None, None),
        (0x3296, None, None),
        (0x3297, Some(1), Some(1)),
        (0x3298, None, None),
        (0x3299, Some(1), Some(1)),
        (0x329A, None, None),
        (0x1EFFF, None, None),
        (0x1F000, Some(1), Some(1)),
        (0x1F0FF, Some(1), Some(1)),
        (0x1F100, None, None),
        (0x1F16F, None, None),
        (0x1F170, Some(1), Some(1)),
        (0x1F171, Some(1), Some(1)),
        (0x1F172, None, None),
        (0x1F17D, None, None),
        (0x1F17E, Some(1), Some(1)),
        (0x1F17F, Some(1), Some(1)),
        (0x1F180, None, None),
        (0x1F18D, None, None),
        (0x1F18E, Some(1), Some(1)),
        (0x1F18F, None, None),
        (0x1F190, None, None),
        (0x1F191, Some(1), Some(1)),
        (0x1F19A, Some(1), Some(1)),
        (0x1F19B, None, None),
        (0x1F1E5, None, None),
        (0x1F1E6, Some(1), None),
        (0x1F1FF, Some(1), None),
        (0x1F200, None, None),
        (0x1F201, Some(1), Some(1)),
        (0x1F202, Some(1), Some(1)),
        (0x1F203, None, None),
        (0x1F219, None, None),
        (0x1F21A, Some(1), Some(1)),
        (0x1F21B, None, None),
        (0x1F22E, None, None),
        (0x1F22F, Some(1), Some(1)),
        (0x1F230, None, None),
        (0x1F231, None, None),
        (0x1F232, Some(1), Some(1)),
        (0x1F23A, Some(1), Some(1)),
        (0x1F23B, None, None),
        (0x1F24F, None, None),
        (0x1F250, Some(1), Some(1)),
        (0x1F251, Some(1), Some(1)),
        (0x1F252, None, None),
        (0x1F2FF, None, None),
        (0x1F300, Some(1), Some(1)),
        (0x1F3FA, Some(1), Some(1)),
        (0x1F3FB, Some(1), Some(1)),
        (0x1F3FF, Some(1), Some(1)),
        (0x1F400, Some(1), Some(1)),
        (0x1FAFF, Some(1), Some(1)),
        (0x1FB00, None, None),
        (0xE001F, None, None),
        (0xE0020, None, None),
        (0xE007F, None, None),
        (0xE0080, None, None),
    ];

    /// More of the Swift code's own answers: every White_Space edge, a
    /// sample of Unicode's emoji-test.txt (Emoji 17.0), the grammar's two
    /// oddities, and random strings from a differential run in which this
    /// port and the Swift code agreed on all 8,215,798 inputs.
    const SWIFT_STRINGS: &[(&str, Option<usize>)] = &[
        ("\u{1F600}\u{9}\u{1F600}", Some(2)), // U+0009 between two emoji
        ("\u{1F600}\u{D}\u{1F600}", Some(2)), // U+000D between two emoji
        ("\u{1F600}\u{1C}\u{1F600}", None),   // U+001C between two emoji
        ("\u{1F600}\u{1F}\u{1F600}", None),   // U+001F between two emoji
        ("\u{1F600}\u{85}\u{1F600}", Some(2)), // U+0085 between two emoji
        ("\u{1F600}\u{A0}\u{1F600}", Some(2)), // U+00A0 between two emoji
        ("\u{1F600}\u{1680}\u{1F600}", Some(2)), // U+1680 between two emoji
        ("\u{1F600}\u{2000}\u{1F600}", Some(2)), // U+2000 between two emoji
        ("\u{1F600}\u{200A}\u{1F600}", Some(2)), // U+200A between two emoji
        ("\u{1F600}\u{200B}\u{1F600}", None), // U+200B between two emoji
        ("\u{1F600}\u{2028}\u{1F600}", Some(2)), // U+2028 between two emoji
        ("\u{1F600}\u{2029}\u{1F600}", Some(2)), // U+2029 between two emoji
        ("\u{1F600}\u{202F}\u{1F600}", Some(2)), // U+202F between two emoji
        ("\u{1F600}\u{205F}\u{1F600}", Some(2)), // U+205F between two emoji
        ("\u{1F600}\u{2060}\u{1F600}", None), // U+2060 between two emoji
        ("\u{1F600}\u{3000}\u{1F600}", Some(2)), // U+3000 between two emoji
        ("\u{1F600}\u{FEFF}\u{1F600}", None), // U+FEFF between two emoji
        ("\u{1F93C}\u{1F3FE}\u{200D}\u{2640}\u{FE0F}", Some(1)), // women wrestling: medium-dark skin tone (E17.0, fully-qualified)
        ("\u{1F46F}\u{1F3FE}\u{200D}\u{2640}\u{FE0F}", Some(1)), // women with bunny ears: medium-dark skin tone (E17.0, fully-qualified)
        (
            "\u{1F468}\u{1F3FE}\u{200D}\u{1FAEF}\u{200D}\u{1F468}\u{1F3FC}",
            Some(1),
        ), // men wrestling: medium-dark skin tone, medium-light skin tone (E17.0, fully-qualified)
        ("\u{1F6D0}", Some(1)), // place of worship (E1.0, fully-qualified)
        ("\u{1F441}\u{FE0F}\u{200D}\u{1F5E8}", Some(1)), // eye in speech bubble (E2.0, minimally-qualified)
        ("\u{1F476}", Some(1)),                          // baby (E0.6, fully-qualified)
        ("\u{2693}", Some(1)),                           // anchor (E0.6, fully-qualified)
        ("\u{1F486}\u{200D}\u{2642}", Some(1)), // man getting massage (E4.0, minimally-qualified)
        (
            "\u{1F9D1}\u{1F3FF}\u{200D}\u{1FAEF}\u{200D}\u{1F9D1}\u{1F3FD}",
            Some(1),
        ), // people wrestling: dark skin tone, medium skin tone (E17.0, fully-qualified)
        ("\u{1F399}", Some(1)),                 // studio microphone (E0.7, unqualified)
        ("\u{1F448}", Some(1)), // backhand index pointing left (E0.6, fully-qualified)
        ("\u{1F962}", Some(1)), // chopsticks (E5.0, fully-qualified)
        (
            "\u{1F468}\u{1F3FD}\u{200D}\u{1F430}\u{200D}\u{1F468}\u{1F3FB}",
            Some(1),
        ), // men with bunny ears: medium skin tone, light skin tone (E17.0, fully-qualified)
        ("\u{1F48C}", Some(1)), // love letter (E0.6, fully-qualified)
        ("\u{1F47C}", Some(1)), // baby angel (E0.6, fully-qualified)
        (
            "\u{1F469}\u{1F3FE}\u{200D}\u{1FAEF}\u{200D}\u{1F469}\u{1F3FC}",
            Some(1),
        ), // women wrestling: medium-dark skin tone, medium-light skin tone (E17.0, fully-qualified)
        (
            "\u{1F469}\u{1F3FC}\u{200D}\u{1FAEF}\u{200D}\u{1F469}\u{1F3FD}",
            Some(1),
        ), // women wrestling: medium-light skin tone, medium skin tone (E17.0, fully-qualified)
        ("\u{1F4AA}", Some(1)), // flexed biceps (E0.6, fully-qualified)
        (
            "\u{1F469}\u{1F3FB}\u{200D}\u{1F430}\u{200D}\u{1F469}\u{1F3FE}",
            Some(1),
        ), // women with bunny ears: light skin tone, medium-dark skin tone (E17.0, fully-qualified)
        ("\u{1FAC8}", Some(1)), // hairy creature (E17.0, fully-qualified)
        ("\u{1F311}", Some(1)), // new moon (E0.6, fully-qualified)
        (
            "\u{1F469}\u{1F3FD}\u{200D}\u{1FAEF}\u{200D}\u{1F469}\u{1F3FB}",
            Some(1),
        ), // women wrestling: medium skin tone, light skin tone (E17.0, fully-qualified)
        ("\u{261D}", Some(1)),  // index pointing up (E0.6, unqualified)
        ("\u{26F8}", Some(1)),  // ice skate (E0.7, unqualified)
        ("\u{1F46F}\u{1F3FC}\u{200D}\u{2642}\u{FE0F}", Some(1)), // men with bunny ears: medium-light skin tone (E17.0, fully-qualified)
        (
            "\u{1F468}\u{1F3FE}\u{200D}\u{1F430}\u{200D}\u{1F468}\u{1F3FC}",
            Some(1),
        ), // men with bunny ears: medium-dark skin tone, medium-light skin tone (E17.0, fully-qualified)
        ("\u{2697}", Some(1)),         // alembic (E1.0, unqualified)
        ("\u{2697}\u{FE0F}", Some(1)), // alembic (E1.0, fully-qualified)
        ("\u{1F44D}", Some(1)),        // thumbs up (E0.6, fully-qualified)
        ("\u{1F576}", Some(1)),        // sunglasses (E0.7, unqualified)
        ("\u{1F3B7}", Some(1)),        // saxophone (E0.6, fully-qualified)
        (
            "\u{1F468}\u{1F3FE}\u{200D}\u{1FAEF}\u{200D}\u{1F468}\u{1F3FD}",
            Some(1),
        ), // men wrestling: medium-dark skin tone, medium skin tone (E17.0, fully-qualified)
        ("\u{1F44B}", Some(1)),        // waving hand (E0.6, fully-qualified)
        (
            "\u{1F468}\u{1F3FE}\u{200D}\u{1F430}\u{200D}\u{1F468}\u{1F3FB}",
            Some(1),
        ), // men with bunny ears: medium-dark skin tone, light skin tone (E17.0, fully-qualified)
        ("\u{1F573}", Some(1)),        // hole (E0.7, unqualified)
        ("\u{1F397}", Some(1)),        // reminder ribbon (E0.7, unqualified)
        ("\u{1F46F}\u{1F3FE}\u{200D}\u{2642}", Some(1)), // men with bunny ears: medium-dark skin tone (E17.0, minimally-qualified)
        ("\u{1F93C}\u{1F3FF}", Some(1)), // people wrestling: dark skin tone (E17.0, fully-qualified)
        ("\u{1F46F}\u{1F3FC}\u{200D}\u{2640}\u{FE0F}", Some(1)), // women with bunny ears: medium-light skin tone (E17.0, fully-qualified)
        ("\u{2708}\u{FE0F}", Some(1)),                           // airplane (E0.6, fully-qualified)
        ("\u{1F3FB}\u{1F3FC}\u{1F3FD}\u{1F3FE}\u{1F3FF}", Some(1)), // the five skin-tone swatches: ONE to the scanner
        ("\u{1F1E6}\u{FE0F}", None), // a regional indicator does not take U+FE0F
        ("\u{1F251}", Some(1)),
        ("\u{1F3FB}\u{1F3FF}\u{2028}\u{C}", Some(1)),
        ("\u{2640}", Some(1)),
        ("\u{2934}", Some(1)),
        ("\u{3030}", Some(1)),
        ("\u{2B07}", Some(1)),
        ("\u{1F21A}", Some(1)),
        ("\u{21AA}\u{1F3FB}\u{FE0F}", Some(1)),
        ("\u{1F251}", Some(1)),
        ("\u{2049}", Some(1)),
        ("\u{2328}", Some(1)),
        ("\u{1F300}", Some(1)),
        (
            "\u{2028}\u{1F1FA}\u{1F18E}\u{2028}\u{2B55}\u{2640}",
            Some(4),
        ),
        ("\u{1F3FF}\u{E007F}\u{303D}", Some(2)),
        ("\u{2122}\u{24C2}", Some(2)),
        ("\u{1F9D1}\u{2139}\u{E007F} \u{A}", Some(2)),
        ("\u{1F22F}\u{1F170}", Some(2)),
        ("\u{2B50}\u{3299}", Some(2)),
        ("\u{1F1EA}\u{2000}\u{27BF}", Some(2)),
        ("\u{2122}\u{2049}\u{E0020}", Some(2)),
        ("\u{1F468}\u{2602}", Some(2)),
        ("\u{2328}\u{2328}", Some(2)),
        ("\u{23FA}\u{205F}\u{1F22F}", Some(2)),
        ("\u{25AA}\u{2764}\u{E0020}", Some(2)),
        (
            "\u{1F3F4}\u{FE0F}\u{1F23A}\u{2B50}\u{1F308}\u{2600}\u{1F17F}\u{2029}",
            Some(6),
        ),
        ("\u{23F3}\u{A9}\u{27BF}\u{1F1FF}\u{1F3FA}\u{2000}", Some(5)),
        (
            "\u{1F468}\u{1F1EA}\u{25FE}\u{AE}\u{C}\u{1F18E}\u{1F600}\u{2049}",
            Some(7),
        ),
        (
            "\u{2049}\u{FE0F}\u{2934}\u{1F17F}\u{3030}\u{1F1FF}",
            Some(5),
        ),
        ("\u{203C}\u{23FA}\u{1F468}\u{203C}\u{1F201}", Some(5)),
        (
            "\u{1F300}\u{1F300}\u{1F3FF}\u{A9}\u{1F22F}\u{1F308}",
            Some(5),
        ),
        ("\u{1F203}\u{1F9D1}\u{301}\u{FE0E}\u{1F3FF}", None),
        ("Z\u{1F1FF}\u{2000}\u{219A}/\u{20E3}", None),
        ("\u{2B50}\u{E007F}\u{202F}a", None),
        ("\u{E0067}\u{1F3FB}", None),
        ("\u{25FF}\u{1F18E}\u{303D}\u{E001F}", None),
        ("", None),
        ("", None),
        ("\u{FE0F}", None),
        ("\u{1F3FB}\u{8}\u{200D}", None),
        ("\u{301}\u{2640}\u{FE00}$\u{303D}", None),
        ("\u{1F9D1}\u{200D}\u{D}\u{2319}", None),
        (
            "a\u{2060}\u{440}/\u{1F201}\u{2933}\u{3299}\u{2B08}\u{23F4}",
            None,
        ),
        ("\u{FE0E}\u{200D}\u{2B04}\u{2029}\u{20E3}\u{1F3FF}", None),
        ("\u{C}\u{2060}\u{E001F}\u{24C2}\u{200B}", None),
        ("\u{25FA}\u{AE}\u{200A}\u{2936}\u{200D}", None),
        ("\u{FE0F}\u{200C}\u{1F3F4}\u{25FE}a\u{231A}", None),
        ("", None),
        ("\u{41F}*\u{E0067}\u{2139}\u{1F000}\u{1680}\u{FE0F}", None),
        ("\u{FE0E}\u{3000}\u{1F19B}\u{20E3}", None),
        ("\u{301}\u{1F18E}\u{1F9D1}", None),
    ];

    #[test]
    fn swift_oracle_vectors() {
        for &(value, alone, with_selector) in SWIFT_BOUNDARIES {
            let scalar = char::from_u32(value).unwrap();
            assert_eq!(
                emoji_only_count(&scalar.to_string()),
                alone,
                "U+{value:04X}"
            );
            assert_eq!(
                emoji_only_count(&format!("{scalar}\u{FE0F}")),
                with_selector,
                "U+{value:04X} U+FE0F"
            );
        }
        for &(text, expected) in SWIFT_STRINGS {
            assert_eq!(
                emoji_only_count(text),
                expected,
                "{:?}",
                text.escape_unicode().to_string()
            );
        }
    }
}
