//! Caret units at the shell's edge (shell-design §7 "Caret units"; carry-forward, w1 board): every
//! `fc-text`/`fc-logic` rule that takes a caret or a selection counts **UTF-16 code units** — as the
//! Mac and Windows do — while `gtk::TextIter::offset()` and `gtk::Editable::position()` count
//! **characters** (Unicode scalars). This module converts both ways, and nothing else in the shell
//! does arithmetic on offsets. Pure; tested with emoji and Cyrillic.

/// How many UTF-16 units `text` is.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the module's own tests measure with it")
)]
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// How many characters (scalars, GTK's unit) `text` is.
pub fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// The UTF-16 offset of GTK's character offset `chars` in `text`. An offset past the end is the
/// end: GTK never reports one, and the end is where a caret past it would stand.
pub fn chars_to_utf16(text: &str, chars: usize) -> usize {
    text.chars().take(chars).map(char::len_utf16).sum()
}

/// GTK's character offset for the UTF-16 offset `units` in `text`, or `None` when it falls inside
/// a surrogate pair (half an emoji) or past the end — no character boundary is there.
pub fn utf16_to_chars(text: &str, units: usize) -> Option<usize> {
    let mut counted = 0;
    for (index, character) in text.chars().enumerate() {
        if counted == units {
            return Some(index);
        }
        if counted > units {
            return None;
        }
        counted += character.len_utf16();
    }
    (counted == units).then(|| char_len(text))
}

/// A selection as the rules take it — `(start, length)` in UTF-16 units — from GTK's two
/// character offsets (in either order).
pub fn selection_to_utf16(text: &str, a: usize, b: usize) -> (usize, usize) {
    let (start, end) = if a <= b { (a, b) } else { (b, a) };
    let start16 = chars_to_utf16(text, start);
    let end16 = chars_to_utf16(text, end);
    (start16, end16 - start16)
}

/// A UTF-16 range `[at, at + length)` as GTK's character range, or `None` when either end is no
/// character boundary.
pub fn range_to_chars(text: &str, at: usize, length: usize) -> Option<(usize, usize)> {
    let start = utf16_to_chars(text, at)?;
    let end = utf16_to_chars(text, at.checked_add(length)?)?;
    Some((start, end))
}

/// The byte offset of GTK's character offset `chars` in `text` (the end past it).
pub fn chars_to_byte(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(byte, _)| byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_counts_the_same_in_both_units() {
        assert_eq!(utf16_len("hello"), 5);
        assert_eq!(chars_to_utf16("hello", 3), 3);
        assert_eq!(utf16_to_chars("hello", 3), Some(3));
        assert_eq!(utf16_to_chars("hello", 5), Some(5));
        assert_eq!(utf16_to_chars("hello", 6), None);
    }

    #[test]
    fn cyrillic_is_one_unit_and_one_character_but_two_bytes() {
        let text = "Привет";
        assert_eq!(utf16_len(text), 6);
        assert_eq!(char_len(text), 6);
        assert_eq!(chars_to_utf16(text, 4), 4);
        assert_eq!(utf16_to_chars(text, 4), Some(4));
        // The byte offset is what slicing needs, never the character's.
        assert_eq!(chars_to_byte(text, 2), 4);
        assert_eq!(&text[chars_to_byte(text, 2)..], "ивет");
    }

    #[test]
    fn an_emoji_is_two_units_and_one_character() {
        // 😀 is outside the BMP: a surrogate pair in UTF-16.
        let text = "a😀b";
        assert_eq!(utf16_len(text), 4);
        assert_eq!(char_len(text), 3);
        assert_eq!(chars_to_utf16(text, 1), 1);
        assert_eq!(chars_to_utf16(text, 2), 3);
        assert_eq!(chars_to_utf16(text, 3), 4);
        assert_eq!(utf16_to_chars(text, 3), Some(2));
        assert_eq!(utf16_to_chars(text, 4), Some(3));
        // Inside the pair: no boundary.
        assert_eq!(utf16_to_chars(text, 2), None);
    }

    #[test]
    fn every_caret_after_an_emoji_moves_by_one() {
        // The carry-forward's warning: without the conversion the caret is off by one for every
        // emoji before it.
        let text = "😀😀 Ёж";
        for chars in 0..=char_len(text) {
            let units = chars_to_utf16(text, chars);
            assert_eq!(utf16_to_chars(text, units), Some(chars), "{chars}");
        }
        assert_eq!(chars_to_utf16(text, 3), 5);
    }

    #[test]
    fn selections_and_ranges_round_trip() {
        let text = "x😀Ёy";
        assert_eq!(selection_to_utf16(text, 3, 1), (1, 3));
        assert_eq!(range_to_chars(text, 1, 3), Some((1, 3)));
        assert_eq!(range_to_chars(text, 2, 1), None);
        assert_eq!(range_to_chars(text, 0, usize::MAX), None);
    }

    #[test]
    fn past_the_end_stands_at_the_end() {
        assert_eq!(chars_to_utf16("ab", 10), 2);
        assert_eq!(chars_to_byte("аб", 10), 4);
        assert_eq!(utf16_to_chars("", 0), Some(0));
    }
}
