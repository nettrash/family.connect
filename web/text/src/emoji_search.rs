//! Reaction search (#116): CLDR names/keywords in the reader's language,
//! English names/keywords and the existing English shortcode aliases.
//! Results are restricted to the reaction catalogue and keep its exact bytes.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::emoji::EMOJI_CATALOG;
use crate::i18n::Lang;

const ANNOTATIONS: &str = include_str!("../data/emoji-search.tsv");
const ALIASES: &str = include_str!("../data/emoji-names.tsv");
pub const RECENT_LIMIT: usize = 24;

struct Entry {
    emoji: &'static str,
    names: HashMap<&'static str, &'static str>,
    terms: HashMap<&'static str, String>,
    aliases: Vec<String>,
}

/// A single result, with a localised accessible name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub emoji: &'static str,
    pub name: &'static str,
}

fn key(emoji: &str) -> String {
    emoji.replace('\u{fe0f}', "")
}

fn normalise(text: &str) -> String {
    text.to_lowercase()
        // German uppercase SS and lowercase ß must find the same keywords.
        .replace('ß', "ss")
        .replace(['_', '-', ':', '|'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn entries() -> &'static [Entry] {
    static INDEX: OnceLock<Vec<Entry>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut seen = HashSet::new();
        let mut entries: Vec<_> = EMOJI_CATALOG
            .iter()
            .flat_map(|category| category.emoji.iter().copied())
            .filter(|emoji| seen.insert(*emoji))
            .map(|emoji| Entry {
                emoji,
                names: HashMap::new(),
                terms: HashMap::new(),
                aliases: Vec::new(),
            })
            .collect();
        let positions: HashMap<_, _> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (key(entry.emoji), index))
            .collect();
        for line in ANNOTATIONS.lines().filter(|line| !line.starts_with('#')) {
            let mut fields = line.splitn(4, '\t');
            if let (Some(locale), Some(emoji), Some(name), Some(words)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            {
                if let Some(&index) = positions.get(&key(emoji)) {
                    entries[index].names.insert(locale, name);
                    entries[index]
                        .terms
                        .insert(locale, normalise(&format!("{name} {words}")));
                }
            }
        }
        for line in ALIASES.lines().filter(|line| !line.starts_with('#')) {
            if let Some((name, emoji)) = line.split_once('\t') {
                if let Some(&index) = positions.get(&key(emoji)) {
                    entries[index].aliases.push(normalise(name));
                }
            }
        }
        entries
    })
}

/// Name used by buttons and screen readers, independent of how it was found.
fn known(emoji: &str) -> Option<&'static Entry> {
    static POSITIONS: OnceLock<HashMap<String, usize>> = OnceLock::new();
    let positions = POSITIONS.get_or_init(|| {
        entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| (key(entry.emoji), index))
            .collect()
    });
    positions.get(&key(emoji)).map(|&index| &entries()[index])
}

pub fn name(lang: Lang, emoji: &str) -> Option<&'static str> {
    known(emoji)
        .and_then(|entry| {
            entry
                .names
                .get(lang.tag())
                .or_else(|| entry.names.get("en"))
        })
        .copied()
}

/// Empty queries return no search results: the UI then draws the catalogue.
/// All query words must match; exact localised/English names and aliases come
/// before name prefixes and keywords. Ties keep catalogue order, with each
/// emoji offered once.
pub fn search(lang: Lang, query: &str) -> Vec<Match> {
    let query = normalise(query);
    if query.is_empty() {
        return Vec::new();
    }
    let words: Vec<_> = query.split_whitespace().collect();
    let mut found = Vec::new();
    for entry in entries() {
        let local = entry.terms.get(lang.tag()).map_or("", String::as_str);
        let english = entry.terms.get("en").map_or("", String::as_str);
        let exact_emoji = key(entry.emoji) == key(&query);
        if !exact_emoji
            && !words.iter().all(|word| {
                local.contains(word)
                    || english.contains(word)
                    || entry.aliases.iter().any(|alias| alias.contains(word))
            })
        {
            continue;
        }
        let name = entry
            .names
            .get(lang.tag())
            .or_else(|| entry.names.get("en"));
        let Some(&name) = name else { continue };
        let local_name = normalise(name);
        let english_name = entry.names.get("en").map(|name| normalise(name));
        let rank = if exact_emoji
            || local_name == query
            || english_name.as_deref() == Some(query.as_str())
            || entry.aliases.iter().any(|alias| alias == &query)
        {
            0
        } else if local_name.starts_with(&query)
            || english_name
                .as_ref()
                .is_some_and(|name| name.starts_with(&query))
        {
            1
        } else {
            2
        };
        found.push((
            rank,
            Match {
                emoji: entry.emoji,
                name,
            },
        ));
    }
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, found)| found).collect()
}

/// Clean stored choices and, if valid, move `picked` to the front. An invalid
/// pick adds nothing. Variants resolve to the exact catalogue bytes.
pub fn recent_with(recent: &[String], picked: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    std::iter::once(picked)
        .chain(recent.iter().map(String::as_str))
        .filter_map(known)
        .filter(|entry| seen.insert(entry.emoji))
        .take(RECENT_LIMIT)
        .map(|entry| entry.emoji.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localised_keywords_find_fire_in_all_nine_languages() {
        for (lang, query, expected_name) in [
            (Lang::En, "fire", "fire"),
            (Lang::De, "Feuer", "Feuer"),
            (Lang::Es, "fuego", "fuego"),
            (Lang::Fr, "feu", "feu"),
            (Lang::Ja, "火", "火"),
            (Lang::Ru, "огонь", "огонь"),
            (Lang::Sr, "ватра", "ватра"),
            (Lang::SrLatn, "vatra", "vatra"),
            (Lang::ZhHans, "火", "火焰"),
        ] {
            let found = search(lang, query);
            assert!(
                found.iter().any(|result| result.emoji == "🔥"),
                "{lang:?} {query}"
            );
            assert!(found.iter().all(|result| !result.name.is_empty()));
            assert_eq!(name(lang, "🔥"), Some(expected_name));
        }
        assert_eq!(name(Lang::Ru, "🔥"), Some("огонь"));
        assert_eq!(name(Lang::SrLatn, "🔥"), Some("vatra"));
        assert_eq!(name(Lang::En, "not emoji"), None);
    }

    #[test]
    fn case_spaces_aliases_and_unicode_variants_return_catalogue_bytes() {
        assert!(search(Lang::De, "FUSSBALL")
            .iter()
            .any(|result| result.emoji == "⚽"));
        assert_eq!(search(Lang::En, "  :FIRE:  ")[0].emoji, "🔥");
        assert_eq!(search(Lang::Ru, "like")[0].emoji, "👍");
        assert_eq!(search(Lang::En, "heart")[0].emoji, "❤️");
        assert_eq!(search(Lang::En, "❤")[0].emoji, "❤️");
        assert!(search(Lang::En, "sad")
            .iter()
            .any(|result| result.emoji == "😞"));
        assert!(search(Lang::En, "red heart")
            .iter()
            .any(|result| result.emoji == "❤️"));
        assert!(search(Lang::En, "fire nonexistentkeyword").is_empty());
        assert!(search(Lang::En, "  ").is_empty());
        assert!(search(Lang::En, "__::").is_empty());
        assert!(search(Lang::En, "no-such-emoji-8372").is_empty());
        assert!(
            search(Lang::En, "👍🏽").is_empty(),
            "not in the reaction catalogue"
        );
    }

    #[test]
    fn english_exact_name_ranks_first_in_russian() {
        let found = search(Lang::Ru, "  :ICE:  ");
        assert_eq!(
            found.first(),
            Some(&Match {
                emoji: "🧊",
                name: "кубик льда",
            })
        );
    }

    #[test]
    fn english_name_prefix_ranks_before_keyword_substrings_in_russian() {
        let found = search(Lang::Ru, "ic");
        let ice = found
            .iter()
            .position(|result| result.emoji == "🧊")
            .unwrap();
        let nice = found
            .iter()
            .position(|result| result.emoji == "😀")
            .unwrap();
        assert!(
            ice < nice,
            "English name prefix must precede the nice keyword"
        );
        assert_eq!(found[ice].name, "кубик льда");
    }

    #[test]
    fn results_are_stable_unique_and_every_locale_covers_the_catalogue() {
        let found = search(Lang::En, "heart");
        assert_eq!(found, search(Lang::En, "heart"));
        let unique: HashSet<_> = found.iter().map(|entry| entry.emoji).collect();
        assert_eq!(unique.len(), found.len());
        for entry in entries() {
            for locale in [
                "en", "de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans",
            ] {
                assert!(!entry.names[locale].is_empty(), "{} {locale}", entry.emoji);
                assert!(!entry.terms[locale].is_empty());
            }
        }
    }

    #[test]
    fn recents_move_to_front_deduplicate_validate_and_bound() {
        assert_eq!(recent_with(&[], "🔥"), vec!["🔥"]);
        assert_eq!(recent_with(&["🔥".into()], "🔥"), vec!["🔥"]);
        let recent = vec!["😀".into(), "❤".into(), "😀".into(), "not emoji".into()];
        assert_eq!(recent_with(&recent, "❤️"), vec!["❤️", "😀"]);
        assert_eq!(recent_with(&recent, ""), vec!["😀", "❤️"]);
        assert_eq!(recent_with(&[], "not emoji"), Vec::<String>::new());
        let many: Vec<_> = entries()
            .iter()
            .take(RECENT_LIMIT + 1)
            .map(|e| e.emoji.into())
            .collect();
        let picked = many.last().unwrap();
        let result = recent_with(&many, picked);
        assert_eq!(result.len(), RECENT_LIMIT);
        assert_eq!(&result[0], picked);
        assert_eq!(result[1], many[0]);
        assert!(!result.contains(&many[RECENT_LIMIT - 1]));
    }
}
