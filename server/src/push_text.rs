//! The words of a push, in the language of the device it goes to (docs/protocol.md, "The words of
//! a push"; issue #82). The table itself is `push_words`, generated from the apps' catalogue by
//! `server/i18n/generate.py`; this is how a device's language finds its row and how a sentence's
//! arguments go into it.

use crate::push_words::{LANGUAGES, TABLE};

/// The one of the table's languages a registered `language` means, or `None` for English — no
/// language, English itself, or a tag the table does not speak.
///
/// The apps send one of their nine localisations exactly (`sr-Latn`, `zh-Hans`), so the exact match
/// is the whole of it in practice. The rest is forgiveness for a tag spelled the other common ways:
/// case (`zh-hans`), a region (`de-AT`, `fr-CA`), Serbian's script (`sr-Latn-RS` is Latin, `sr-RS`
/// and `sr-Cyrl` Cyrillic), and Chinese — Simplified for `zh` and `zh-Hans-…`, English for
/// Traditional (`zh-Hant`, `zh-TW`, `zh-HK`), which reads Simplified as a different script.
pub fn language(registered: Option<&str>) -> Option<&'static str> {
    let tag = registered?.trim();
    if tag.is_empty() {
        return None;
    }
    if let Some(exact) = LANGUAGES
        .iter()
        .find(|known| known.eq_ignore_ascii_case(tag))
    {
        return Some(exact);
    }
    let lower = tag.to_ascii_lowercase();
    let mut parts = lower.split(['-', '_']);
    let primary = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    match primary {
        "sr" if rest.contains(&"latn") => Some("sr-Latn"),
        "zh" if rest
            .iter()
            .any(|part| matches!(*part, "hant" | "tw" | "hk" | "mo")) =>
        {
            None
        }
        "zh" => Some("zh-Hans"),
        _ => LANGUAGES.iter().find(|known| **known == primary).copied(),
    }
}

/// `key`'s words in the device's `registered` language, or `None` when that is English.
pub fn words(registered: Option<&str>, key: &str) -> Option<&'static str> {
    let language = language(registered)?;
    TABLE
        .binary_search_by(|(k, l, _)| (*k, *l).cmp(&(key, language)))
        .ok()
        .map(|at| TABLE[at].2)
}

/// A sentence with its arguments in: `%@` and `%lld` take them in order, and `%1$@` / `%2$lld`
/// by number — the catalogue's two spellings, the second for a language that turns the order of
/// a sentence round. A missing argument is left out rather than printed as a placeholder.
pub fn fill(template: &str, args: &[String]) -> String {
    let mut out =
        String::with_capacity(template.len() + args.iter().map(String::len).sum::<usize>());
    let mut next = 0;
    let mut rest = template;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let (index, spec) = match tail[digits..].strip_prefix('$') {
            Some(after) if digits > 0 => (
                tail[..digits]
                    .parse::<usize>()
                    .ok()
                    .map(|n| n.saturating_sub(1)),
                after,
            ),
            _ => (None, tail),
        };
        let consumed = if spec.starts_with("lld") {
            Some(3)
        } else if spec.starts_with('@') {
            Some(1)
        } else {
            None
        };
        match consumed {
            Some(width) => {
                let slot = index.unwrap_or_else(|| {
                    next += 1;
                    next - 1
                });
                if let Some(arg) = args.get(slot) {
                    out.push_str(arg);
                }
                let skipped = if index.is_some() { digits + 1 } else { 0 };
                rest = &tail[skipped + width..];
            }
            None => {
                out.push('%');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_apps_nine_tags_are_spoken_exactly() {
        for tag in LANGUAGES {
            assert_eq!(language(Some(tag)), Some(*tag));
        }
        assert_eq!(language(None), None);
        assert_eq!(language(Some("")), None);
        assert_eq!(language(Some("en")), None);
        assert_eq!(language(Some("en-GB")), None);
    }

    #[test]
    fn a_tag_spelled_another_common_way_finds_its_row() {
        assert_eq!(language(Some("de-AT")), Some("de"));
        assert_eq!(language(Some("fr_CA")), Some("fr"));
        assert_eq!(language(Some("zh-hans")), Some("zh-Hans"));
        assert_eq!(language(Some("zh")), Some("zh-Hans"));
        assert_eq!(language(Some("zh-Hans-CN")), Some("zh-Hans"));
        assert_eq!(language(Some("sr-Latn-RS")), Some("sr-Latn"));
        assert_eq!(language(Some("sr-RS")), Some("sr"));
        assert_eq!(language(Some("sr-Cyrl")), Some("sr"));
    }

    #[test]
    fn traditional_chinese_and_unknown_languages_get_english() {
        for tag in [
            "zh-Hant",
            "zh-TW",
            "zh-HK",
            "zh-Hant-TW",
            "pt-BR",
            "xx",
            "Klingon",
        ] {
            assert_eq!(language(Some(tag)), None, "{tag}");
        }
    }

    #[test]
    fn every_row_is_found_and_english_is_the_key_itself() {
        for (key, lang, said) in TABLE {
            assert_eq!(words(Some(lang), key), Some(*said));
            assert_eq!(words(None, key), None);
        }
        assert_eq!(words(Some("de"), "a sentence nobody wrote"), None);
    }

    #[test]
    fn the_table_is_sorted_for_its_binary_search() {
        assert!(
            TABLE
                .windows(2)
                .all(|pair| (pair[0].0, pair[0].1) < (pair[1].0, pair[1].1))
        );
    }

    #[test]
    fn arguments_go_in_by_order_and_by_number() {
        assert_eq!(
            fill("%@ — %@ mentioned you", &args(&["The Smiths", "Anna"])),
            "The Smiths — Anna mentioned you"
        );
        assert_eq!(fill("Файлов: %lld", &args(&["4"])), "Файлов: 4");
        assert_eq!(fill("%2$@ and %1$@", &args(&["a", "b"])), "b and a");
        assert_eq!(fill("100% sure, %@", &args(&["Anna"])), "100% sure, Anna");
        assert_eq!(fill("%@ and %@", &args(&["only"])), "only and ");
        assert_eq!(fill("no holes", &args(&["x"])), "no holes");
    }
}
