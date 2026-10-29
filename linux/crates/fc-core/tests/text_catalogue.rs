//! `CatalogueTests`: the nine languages — that they ship, that they are complete, and that a
//! translation may put its placeholders wherever its grammar wants them.
//!
//! The KEY IS THE ENGLISH SENTENCE, which is what makes a missing translation read in English
//! rather than as a slug. The C# suite reads the tables its assembly embeds. Here the MECHANISM
//! (placeholders, plurals, fallbacks) is held on `win/`'s tables, copied byte for byte into
//! `tests/fixtures/i18n`; COMPLETENESS is held on the tables the Linux client ships —
//! `linux/data/i18n`, written by `linux/i18n/generate.py` from the apps' xcstrings,
//! `linux/i18n/linux.json` and `win/i18n/win.json` (shell-design §23).

mod common;

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use fc_core::text::catalogues::{Catalogue, Catalogues, JsonCatalog, Languages};
use fc_core::text::plurals::{PluralRules, PluralTables};
use fc_core::text::string_catalog::{EnglishCatalog, StringCatalog};

/// The sentences nobody has translated yet, named rather than pretended — none in the C# suite
/// since #82, and none on Linux since `linux/i18n/linux.json` (wave 3, S0). A sentence named here
/// must also be in `linux.json` as `{}` (the generator's English-for-now), and the other way round.
const ENGLISH_FOR_NOW: [&str; 0] = [];

fn json(language: &str) -> JsonCatalog {
    common::catalogues()
        .json(language)
        .unwrap_or_else(|| panic!("{language} ships"))
}

/// The tables the Linux client ships (`linux/data/i18n`, what `services::catalogue` loads).
fn linux_catalogues() -> Catalogues {
    let tables = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/i18n");
    Catalogues::from_dir(&tables).expect("the Linux tables load (python3 linux/i18n/generate.py .)")
}

fn linux_json(language: &str) -> JsonCatalog {
    linux_catalogues()
        .json(language)
        .unwrap_or_else(|| panic!("{language} ships on Linux"))
}

/// Every sentence the Linux client draws: `common::sentences()` (the crates) and the GTK shell's
/// own (`linux/app/src`), found by the same rule — a string literal asked of a value named `say`.
/// `linux/i18n/generate.py` scans exactly these two roots, so the tables and this list agree.
fn port_sentences() -> Vec<String> {
    let mut found: BTreeSet<String> = common::sentences().into_iter().collect();
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../app/src");
    let mut directories = vec![app];
    while let Some(directory) = directories.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                if name != "tests" && name != "target" {
                    directories.push(path);
                }
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let source: String = std::fs::read_to_string(&path)
                .expect("a source file")
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .map(|line| format!("{line}\n"))
                .collect();
            for call in [".get(", ".format(", ".plural("] {
                let mut from = 0;
                while let Some(at) = source[from..].find(call).map(|at| at + from) {
                    from = at + call.len();
                    let receiver = source[..at].trim_end();
                    let named_say = receiver.ends_with("say")
                        && !receiver[..receiver.len() - 3]
                            .ends_with(|c: char| c.is_alphanumeric() || c == '_');
                    if named_say && let Some(literal) = plain_literal(source[from..].trim_start()) {
                        found.insert(literal);
                    }
                }
            }
        }
    }
    found.into_iter().collect()
}

/// The ordinary string literal `text` starts with, unescaped (`common`'s rule; the shell's
/// sentences use no escapes but `\"`, `\\` and `\n`).
fn plain_literal(text: &str) -> Option<String> {
    let mut characters = text.strip_prefix('"')?.chars();
    let mut out = String::new();
    loop {
        match characters.next()? {
            '"' => return Some(out),
            '\\' => match characters.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                // A `\u{…}` or a line continuation never appears in the shell's sentences; such a
                // literal is left out rather than misread.
                'u' | '\n' => return None,
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
}

#[test]
fn this_port_actually_draws_something() {
    let sentences = common::sentences();
    // A scan that found nothing would make every test below vacuously true.
    assert!(
        sentences.len() >= 25,
        "only {} sentences found",
        sentences.len()
    );
    assert!(sentences.iter().any(|sentence| sentence == "No answer"));
}

#[test]
fn all_nine_languages_ship() {
    assert_eq!(common::catalogues().shipped(), Languages::ALL);
    assert_eq!(linux_catalogues().shipped(), Languages::ALL);
}

/// EVERY SENTENCE IS IN EVERY LANGUAGE, or on the short list of ones nobody has translated —
/// which is the point of the list: a new key cannot go quietly untranslated in nine catalogues
/// that look complete.
#[test]
fn every_sentence_is_translated_or_known_to_be_english_for_now() {
    let sentences = port_sentences();
    let catalogues = linux_catalogues();
    for language in Languages::ALL.iter().skip(1) {
        let catalogue = catalogues.json(language).expect("ships");
        for sentence in &sentences {
            // Asked of the TABLE and not of the answer: "Video" in German and "Photo" in French
            // are real translations identical to the English, and comparing values would call
            // them missing.
            if catalogue.holds(sentence) || catalogues.plurals().forms(language, sentence).is_some()
            {
                continue;
            }
            assert!(
                ENGLISH_FOR_NOW.contains(&sentence.as_str()),
                "{language} lacks {sentence:?}"
            );
        }
    }
}

#[test]
fn the_english_only_list_names_nothing_that_is_actually_translated() {
    // A stale list is a lie in the other direction: it would hide a translation that arrived.
    let russian = linux_json("ru");
    for sentence in ENGLISH_FOR_NOW {
        assert!(!russian.holds(sentence), "{sentence}");
        assert_eq!(russian.get(sentence), sentence);
    }
    let sentences = port_sentences();
    for sentence in ENGLISH_FOR_NOW {
        assert!(sentences.iter().any(|said| said == sentence));
    }
}

#[test]
fn a_translated_sentence_reads_in_its_own_language() {
    let catalogues = common::catalogues();
    assert_eq!(catalogues.for_language("ru").get("No answer"), "Нет ответа");
    // A translation identical to the English is still a translation: German says "Video".
    assert!(json("de").holds("Video"));
    assert_eq!(catalogues.for_language("de").get("Video"), "Video");
    assert_eq!(
        catalogues.for_language("de").get("No answer"),
        "Keine Antwort"
    );
    assert_eq!(catalogues.for_language("ja").get("No answer"), "応答なし");
    // The two Serbians are the same language in two alphabets, and a reader of one cannot read
    // the other.
    assert_ne!(
        catalogues.for_language("sr").get("No answer"),
        catalogues.for_language("sr-Latn").get("No answer")
    );
}

/// A TRANSLATION MAY REORDER ITS PLACEHOLDERS — German and Russian both do, in the shipped values
/// — so the formatter reads positional forms as well as sequential ones.
#[test]
fn a_translation_may_put_its_placeholders_where_its_grammar_wants_them() {
    let catalogues = common::catalogues();
    assert_eq!(
        catalogues
            .for_language("de")
            .format("%lld of %lld done", &[&3, &7]),
        "3 von 7 erledigt"
    );
    assert_eq!(
        catalogues
            .for_language("ru")
            .format("%lld of %lld done", &[&3, &7]),
        "Сделано 3 из 7"
    );
    // And English, whose value IS the key, fills them in order.
    assert_eq!(
        EnglishCatalog::new().format("%lld of %lld done", &[&3, &7]),
        "3 of 7 done"
    );
}

#[test]
fn a_missing_key_answers_itself_because_the_key_is_the_english() {
    let russian =
        JsonCatalog::parse(r#"{"No answer": "Нет ответа"}"#, "ru", common::plurals()).unwrap();

    assert_eq!(russian.get("No answer"), "Нет ответа");
    // Not a slug, not a panic, not an empty label: the English sentence.
    assert_eq!(russian.get("Dinner at 7?"), "Dinner at 7?");
    assert_eq!(russian.format("%lld Photos", &[&2]), "2 Photos");
}

#[test]
fn a_language_is_matched_as_nearly_as_the_catalogue_can() {
    assert_eq!(Languages::nearest(Some("de-AT")), "de");
    assert_eq!(Languages::nearest(Some("ru_RU")), "ru");
    assert_eq!(Languages::nearest(Some("sr-Cyrl-RS")), "sr");
    assert_eq!(Languages::nearest(Some("sr-Latn-RS")), "sr-Latn");
    // The catalogue holds Simplified only, which is the nearer of the two wrong answers for a
    // Traditional reader.
    assert_eq!(Languages::nearest(Some("zh-Hant-TW")), "zh-Hans");
    assert_eq!(Languages::nearest(Some("is")), "en");
    assert_eq!(Languages::nearest(None), "en");
    assert_eq!(Languages::nearest(Some("   ")), "en");
}

/// THE FAMILY'S LANGUAGE IS NOT THE DISPLAY LANGUAGE. It is what the assistant answers in; a
/// family setting that silently re-languaged somebody's computer would be a surprise nobody asked
/// for, so the window draws in the DEVICE's language and nothing else.
#[test]
fn a_familys_language_does_not_relanguage_anybodys_window() {
    // A German member of a Russian-speaking family reads a German window.
    assert_eq!(Languages::for_display(Some("de-DE")), "de");
    assert_eq!(
        common::catalogues()
            .for_language(Languages::for_display(Some("de-DE")))
            .get("No answer"),
        "Keine Antwort"
    );

    // And there is no way to ask for the family's: it takes one argument, and it is the device's.
    assert_eq!(Languages::for_display(None), "en");
}

#[test]
fn english_needs_no_table_at_all() {
    let english = common::catalogues().for_language("en");

    assert!(matches!(english, Catalogue::English(_)));
    assert_eq!(english.get("No answer"), "No answer");
}

/// The tables that ship are the ones the generator wrote: a hand-edited catalogue, or one left
/// behind by a key that changed, is a Russian app drawing English at the moment somebody notices.
#[test]
fn the_shipped_tables_hold_only_sentences_this_port_draws() {
    let sentences: BTreeSet<String> = port_sentences().into_iter().collect();
    for language in Languages::ALL.iter().skip(1) {
        let catalogue = linux_json(language);
        assert!(catalogue.count() > 0, "{language}");
        assert!(
            catalogue.count() <= sentences.len(),
            "{language} holds {} of {} sentences",
            catalogue.count(),
            sentences.len()
        );
    }
}

/// The generator (`linux/i18n/generate.py`) and this test's scan (`common::sentences`) find the
/// same sentences: every key of every Linux table is one the port draws, and every sentence drawn
/// is in every table or said in plural forms only.
#[test]
fn the_linux_tables_and_the_source_agree_sentence_for_sentence() {
    let sentences: BTreeSet<String> = port_sentences().into_iter().collect();
    let catalogues = linux_catalogues();
    let table_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/i18n");
    for language in Languages::ALL.iter().skip(1) {
        let text = std::fs::read_to_string(table_dir.join(format!("{language}.json"))).unwrap();
        let table: HashMap<String, String> = serde_json::from_str(&text).unwrap();
        for key in table.keys() {
            assert!(
                sentences.contains(key),
                "{language} holds {key:?}, which nothing draws"
            );
        }
        for sentence in &sentences {
            assert!(
                table.contains_key(sentence)
                    || catalogues.plurals().forms(language, sentence).is_some(),
                "{language} lacks {sentence:?}"
            );
        }
    }
}

/// Three forms in Russian and Serbian, zero as one in French, one form in Japanese.
#[test]
fn a_count_takes_its_languages_plural_category() {
    let russian: Vec<&str> = [1, 3, 5, 11, 21, 22]
        .into_iter()
        .map(|count| PluralRules::category("ru", count))
        .collect();
    assert_eq!(russian, ["one", "few", "many", "many", "one", "few"]);
    assert_eq!(PluralRules::category("fr", 0), "one");
    assert_eq!(PluralRules::category("en", 0), "other");
    assert_eq!(PluralRules::category("ja", 1), "other");
    assert_eq!(PluralRules::category("sr-Latn-RS", 2), "few");
}

#[test]
fn a_key_with_no_forms_is_said_by_its_one_sentence() {
    let russian = JsonCatalog::parse(
        r#"{"%lld unheard-of things": "невиданных вещей: %lld"}"#,
        "ru",
        common::plurals(),
    )
    .unwrap();

    assert_eq!(
        russian.plural("%lld unheard-of things", 5, &[&5]),
        "невиданных вещей: 5"
    );
    assert_eq!(
        EnglishCatalog::with_plurals(common::plurals()).plural("%lld unheard-of things", 7, &[&7]),
        "7 unheard-of things"
    );
}

/// A key nobody translated reads as English — and English that agrees with its number.
#[test]
fn an_untranslated_plural_is_english_that_agrees_with_its_number() {
    let bare = JsonCatalog::parse("{}", "ru", common::plurals()).unwrap();

    assert_eq!(
        bare.plural("%lld questions to the assistant", 1, &[&1]),
        "1 question to the assistant"
    );
    assert_eq!(
        bare.plural("%lld questions to the assistant", 5, &[&5]),
        "5 questions to the assistant"
    );
}

#[test]
fn a_form_the_language_lacks_falls_back_to_other() {
    let two: HashMap<String, String> = [("one", "один"), ("other", "много")]
        .into_iter()
        .map(|(category, form)| (category.to_string(), form.to_string()))
        .collect();

    assert_eq!(PluralTables::pick(&two, "ru", 21), Some("один"));
    assert_eq!(PluralTables::pick(&two, "ru", 5), Some("много"));
    let one: HashMap<String, String> = [("one".to_string(), "один".to_string())].into();
    assert_eq!(PluralTables::pick(&one, "ru", 5), None);
}
