//! Reaction search (#116): CLDR names/keywords in the reader's language,
//! English names/keywords and the existing English shortcode aliases.
//! Results are restricted to the reaction catalogue and keep its exact bytes.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::emoji::EMOJI_CATALOG;
use crate::i18n::Lang;

const ENGLISH: &str = include_str!("../data/emoji-search/en.tsv");
const ALIASES: &str = include_str!("../data/emoji-names.tsv");
pub const RECENT_LIMIT: usize = 24;

struct Entry {
    emoji: &'static str,
    aliases: Vec<String>,
}

struct Annotation {
    name: String,
    normalised_name: String,
    terms: String,
}

/// Names and keywords for one language. Web can load this alongside English.
pub struct Annotations(HashMap<&'static str, Annotation>);

impl Annotations {
    /// Accept a complete catalogue for this language. A mismatched, truncated
    /// or invalid response leaves the picker on its built-in English data.
    pub fn parse(lang: Lang, tsv: &str) -> Option<Self> {
        let mut annotations = HashMap::new();
        for line in tsv.lines().filter(|line| !line.starts_with('#')) {
            let mut fields = line.split('\t');
            let locale = fields.next()?;
            let emoji = fields.next()?;
            let name = fields.next()?;
            let keywords = fields.next()?;
            if locale != lang.tag()
                || name.is_empty()
                || keywords.is_empty()
                || fields.next().is_some()
            {
                return None;
            }
            let entry = known(emoji).filter(|entry| entry.emoji == emoji)?;
            let annotation = Annotation {
                name: name.to_owned(),
                normalised_name: normalise(name),
                terms: normalise(&format!("{name} {keywords}")),
            };
            if annotations.insert(entry.emoji, annotation).is_some() {
                return None;
            }
        }
        (annotations.len() == entries().len()).then_some(Self(annotations))
    }
}

fn english() -> &'static Annotations {
    static ENTRIES: OnceLock<Annotations> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        Annotations::parse(Lang::En, ENGLISH).expect("complete English emoji search data")
    })
}

/// A single result, with a localised accessible name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match<'a> {
    pub emoji: &'static str,
    pub name: &'a str,
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
                aliases: Vec::new(),
            })
            .collect();
        let positions: HashMap<_, _> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (key(entry.emoji), index))
            .collect();
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

/// Name used by buttons and screen readers, independent of how it was found.
pub fn name<'a>(emoji: &str, local: Option<&'a Annotations>) -> Option<&'a str> {
    let entry = known(emoji)?;
    let annotations = local.unwrap_or_else(|| english());
    Some(&annotations.0[entry.emoji].name)
}

/// Empty queries return no search results: the UI then draws the catalogue.
/// All query words must match; exact localised/English names and aliases come
/// before name prefixes and keywords. Ties keep catalogue order, with each
/// emoji offered once.
pub fn search<'a>(query: &str, local: Option<&'a Annotations>) -> Vec<Match<'a>> {
    let query = normalise(query);
    if query.is_empty() {
        return Vec::new();
    }
    let words: Vec<_> = query.split_whitespace().collect();
    let query_emoji = key(&query);
    let english = english();
    let local = local.unwrap_or(english);
    let mut found = Vec::new();
    for entry in entries() {
        let local = &local.0[entry.emoji];
        let english = &english.0[entry.emoji];
        let exact_emoji = key(entry.emoji) == query_emoji;
        if !exact_emoji
            && !words.iter().all(|word| {
                local.terms.contains(word)
                    || english.terms.contains(word)
                    || entry.aliases.iter().any(|alias| alias.contains(word))
            })
        {
            continue;
        }
        let rank = if exact_emoji
            || local.normalised_name == query
            || english.normalised_name == query
            || entry.aliases.iter().any(|alias| alias == &query)
        {
            0
        } else if local.normalised_name.starts_with(&query)
            || english.normalised_name.starts_with(&query)
        {
            1
        } else {
            2
        };
        found.push((
            rank,
            Match {
                emoji: entry.emoji,
                name: &local.name,
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

    fn localized(lang: Lang) -> &'static Annotations {
        static LOCALES: OnceLock<HashMap<Lang, Annotations>> = OnceLock::new();
        &LOCALES.get_or_init(|| {
            [
                (Lang::En, ENGLISH),
                (Lang::De, include_str!("../data/emoji-search/de.tsv")),
                (Lang::Es, include_str!("../data/emoji-search/es.tsv")),
                (Lang::Fr, include_str!("../data/emoji-search/fr.tsv")),
                (Lang::Ja, include_str!("../data/emoji-search/ja.tsv")),
                (Lang::Ru, include_str!("../data/emoji-search/ru.tsv")),
                (Lang::Sr, include_str!("../data/emoji-search/sr.tsv")),
                (
                    Lang::SrLatn,
                    include_str!("../data/emoji-search/sr-Latn.tsv"),
                ),
                (
                    Lang::ZhHans,
                    include_str!("../data/emoji-search/zh-Hans.tsv"),
                ),
            ]
            .into_iter()
            .map(|(lang, tsv)| (lang, Annotations::parse(lang, tsv).unwrap()))
            .collect()
        })[&lang]
    }

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
            let annotations = Some(localized(lang));
            let found = search(query, annotations);
            assert!(
                found.iter().any(|result| result.emoji == "🔥"),
                "{lang:?} {query}"
            );
            assert!(found.iter().all(|result| !result.name.is_empty()));
            assert_eq!(name("🔥", annotations), Some(expected_name));
        }
        assert_eq!(name("🔥", None), Some("fire"));
        assert_eq!(name("not emoji", None), None);
    }

    #[test]
    fn case_spaces_aliases_and_unicode_variants_return_catalogue_bytes() {
        assert!(search("FUSSBALL", Some(localized(Lang::De)))
            .iter()
            .any(|result| result.emoji == "⚽"));
        assert_eq!(search("  :FIRE:  ", None)[0].emoji, "🔥");
        assert_eq!(search("like", Some(localized(Lang::Ru)))[0].emoji, "👍");
        assert_eq!(search("heart", None)[0].emoji, "❤️");
        assert_eq!(search("❤", None)[0].emoji, "❤️");
        assert!(search("sad", None)
            .iter()
            .any(|result| result.emoji == "😞"));
        assert!(search("red heart", None)
            .iter()
            .any(|result| result.emoji == "❤️"));
        assert!(search("fire nonexistentkeyword", None).is_empty());
        assert!(search("  ", None).is_empty());
        assert!(search("__::", None).is_empty());
        assert!(search("no-such-emoji-8372", None).is_empty());
        assert!(
            search("👍🏽", None).is_empty(),
            "not in the reaction catalogue"
        );
    }

    #[test]
    fn english_exact_name_ranks_first_in_russian() {
        let found = search("  :ICE:  ", Some(localized(Lang::Ru)));
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
        let found = search("ic", Some(localized(Lang::Ru)));
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
        let found = search("heart", None);
        assert_eq!(found, search("heart", None));
        let unique: HashSet<_> = found.iter().map(|entry| entry.emoji).collect();
        assert_eq!(unique.len(), found.len());
        for entry in entries() {
            for lang in [
                Lang::En,
                Lang::De,
                Lang::Es,
                Lang::Fr,
                Lang::Ja,
                Lang::Ru,
                Lang::Sr,
                Lang::SrLatn,
                Lang::ZhHans,
            ] {
                let annotation = &localized(lang).0[entry.emoji];
                assert!(!annotation.name.is_empty(), "{} {lang:?}", entry.emoji);
                assert!(!annotation.terms.is_empty());
            }
        }
    }

    #[test]
    fn rejects_wrong_language_truncated_and_duplicate_catalogues() {
        let russian = include_str!("../data/emoji-search/ru.tsv");
        let (short, last) = russian.trim_end().rsplit_once('\n').unwrap();
        assert!(Annotations::parse(Lang::Fr, russian).is_none());
        assert!(Annotations::parse(Lang::Ru, short).is_none());
        assert!(Annotations::parse(Lang::Ru, &format!("{russian}{last}\n")).is_none());
        assert!(Annotations::parse(Lang::Ru, "<!doctype html>").is_none());
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
