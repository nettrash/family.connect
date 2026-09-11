//! What a profile circle says when there is no picture: the initials of the
//! name it stands for (ios Views/InitialsAvatar.swift).
//!
//! The first letter of each of the first two words, uppercased — words split
//! on the SPACE character alone, as Swift's `split(separator: " ")` splits,
//! empty runs dropped — and a letter is a whole grapheme, as Swift's
//! `Character` is, so a name that starts with an emoji or a letter written
//! with a combining mark keeps all of it. A name with nothing in it is "?".

use unicode_segmentation::UnicodeSegmentation;

/// The longest side of a profile picture as uploaded, in pixels — sharp on
/// every circle this client draws (ios Core/AvatarImage.swift).
pub const EDGE: u32 = 512;

/// The JPEG qualities tried in turn: the first that fits `MAX_BYTES` is
/// sent, and the last is sent anyway when none does.
pub const QUALITIES: [f64; 4] = [0.8, 0.65, 0.5, 0.4];

/// The byte budget for an upload. The server takes 256 KiB, but a family
/// server sits behind a proxy whose own body limit is small, and a proxy's
/// 413 carries none of the protocol's explanation — so the apps stay far
/// under it, and so does this client (ios Core/AvatarImage.swift).
pub const MAX_BYTES: usize = 56 * 1024;

/// Where the square comes from and how big it ends up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Square {
    /// The top-left corner of the largest centred square of the source.
    pub x: u32,
    pub y: u32,
    /// Its side, in source pixels.
    pub side: u32,
    /// The side it is drawn at: never larger than `EDGE`, never scaled UP.
    pub edge: u32,
}

/// The centre-cropped square of a `width`×`height` picture, or None for a
/// picture with no pixels.
pub fn square(width: u32, height: u32) -> Option<Square> {
    let side = width.min(height);
    (side > 0).then(|| Square {
        x: (width - side) / 2,
        y: (height - side) / 2,
        side,
        edge: side.min(EDGE),
    })
}

pub fn initials(title: &str) -> String {
    let letters: String = title
        .split(' ')
        .filter(|word| !word.is_empty())
        .take(2)
        .filter_map(|word| word.graphemes(true).next())
        .collect();
    if letters.is_empty() {
        "?".to_string()
    } else {
        letters.to_uppercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_its_largest_centred_square_at_most_512_across() {
        assert_eq!(
            square(4000, 3000),
            Some(Square {
                x: 500,
                y: 0,
                side: 3000,
                edge: 512
            })
        );
        assert_eq!(
            square(300, 401),
            Some(Square {
                x: 0,
                y: 50,
                side: 300,
                edge: 300
            }),
            "a small picture is never scaled up"
        );
        assert_eq!(
            square(512, 512),
            Some(Square {
                x: 0,
                y: 0,
                side: 512,
                edge: 512
            })
        );
        assert_eq!(square(0, 10), None);
    }

    #[test]
    fn two_words_give_two_letters_and_more_give_no_more() {
        assert_eq!(initials("Anna Smith"), "AS");
        assert_eq!(initials("anna maria smith"), "AM");
        assert_eq!(initials("Anna"), "A");
        assert_eq!(initials("  Anna   Smith  "), "AS", "empty runs are dropped");
    }

    #[test]
    fn nothing_to_draw_is_a_question_mark() {
        assert_eq!(initials(""), "?");
        assert_eq!(initials("   "), "?");
    }

    /// A letter is a GRAPHEME, as Swift's Character is: an emoji family or
    /// an e with a combining accent is kept whole, never cut to a scalar.
    #[test]
    fn a_letter_is_a_whole_grapheme() {
        assert_eq!(initials("👨‍👩‍👧 Smiths"), "👨‍👩‍👧S");
        assert_eq!(initials("e\u{301}mile zola"), "E\u{301}Z");
        assert_eq!(initials("Юлия Иванова"), "ЮИ");
        assert_eq!(initials("łukasz"), "Ł");
    }

    /// Only the SPACE separates words, as Swift's split(separator: " ")
    /// does: a tab or a newline is part of a word.
    #[test]
    fn only_a_space_separates_words() {
        assert_eq!(initials("Anna\tSmith"), "A");
        assert_eq!(
            initials("Anna\u{a0}Smith"),
            "A",
            "a no-break space is not a space here"
        );
    }

    /// Uppercasing is Unicode's, not a locale's — the same answer Swift's
    /// `uppercased()` gives.
    #[test]
    fn uppercasing_is_unicodes() {
        assert_eq!(initials("ßig"), "SS");
        assert_eq!(initials("istanbul"), "I");
    }
}
