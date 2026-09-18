//! The nine languages, and how a string is found in them
//! (docs/protocol.md, "The family's language" names the same nine for the
//! assistant; these are the ones the APPS are translated into, and their
//! catalogue is where these come from).
//!
//! A KEY is the English source string, exactly as it is in the apps'
//! `Localizable.xcstrings`: `t("Sign in")` finds "Anmelden" for a German
//! reader and answers "Sign in" for anybody the catalogue has nothing for.
//! That is what lets the web reuse thousands of translated strings without
//! a second set of names for them, and it makes an untranslated string a
//! readable one rather than a blank or a shouty `SIGN_IN`.
//!
//! The language is the READER's, from the browser — not the family's. The
//! family's language (`Family.language`) is what the assistant answers in,
//! and the two are deliberately separate: a Serbian grandmother reading a
//! Russian family's chat reads the app in Serbian.

mod catalogue;

/// The nine, as the protocol spells them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    #[default]
    En,
    De,
    Es,
    Fr,
    Ja,
    Ru,
    /// Serbian in Cyrillic.
    Sr,
    /// Serbian in Latin script.
    SrLatn,
    /// Simplified Chinese.
    ZhHans,
}

impl Lang {
    /// The tag the protocol uses for this language.
    pub fn tag(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
            Lang::Es => "es",
            Lang::Fr => "fr",
            Lang::Ja => "ja",
            Lang::Ru => "ru",
            Lang::Sr => "sr",
            Lang::SrLatn => "sr-Latn",
            Lang::ZhHans => "zh-Hans",
        }
    }

    /// The language a BCP 47 tag asks for, if it is one of the nine.
    ///
    /// A tag is matched on its own terms and then on its base language, the
    /// way an operating system matches one: `de-AT` reads German, `sr-Latn-RS`
    /// reads Serbian in Latin script, and `sr-RS` reads it in Cyrillic. The
    /// one place this is more generous than Apple's own matching is Chinese:
    /// `zh-Hant` has no catalogue of its own here, and Simplified is closer
    /// to it than English is.
    pub fn for_tag(tag: &str) -> Option<Lang> {
        let tag = tag.trim().to_lowercase().replace('_', "-");
        let mut parts = tag.split('-');
        let base = parts.next().unwrap_or_default();
        let script = tag.contains("-latn");
        match base {
            "en" => Some(Lang::En),
            "de" => Some(Lang::De),
            "es" => Some(Lang::Es),
            "fr" => Some(Lang::Fr),
            "ja" => Some(Lang::Ja),
            "ru" => Some(Lang::Ru),
            "sr" | "sh" => Some(if script { Lang::SrLatn } else { Lang::Sr }),
            // Croatian and Bosnian are written in Latin and are close
            // enough to be read: the alternative for those readers is
            // English, which is further away.
            "hr" | "bs" => Some(Lang::SrLatn),
            "zh" => Some(Lang::ZhHans),
            _ => None,
        }
    }

    /// The first of `tags` this client has a catalogue for — a browser's
    /// language list, in the reader's own order of preference.
    pub fn best(tags: &[String]) -> Lang {
        tags.iter()
            .find_map(|tag| Lang::for_tag(tag))
            .unwrap_or(Lang::En)
    }

    fn table(self) -> catalogue::Table {
        match self {
            Lang::En => catalogue::EN,
            Lang::De => catalogue::DE,
            Lang::Es => catalogue::ES,
            Lang::Fr => catalogue::FR,
            Lang::Ja => catalogue::JA,
            Lang::Ru => catalogue::RU,
            Lang::Sr => catalogue::SR,
            Lang::SrLatn => catalogue::SR_LATN,
            Lang::ZhHans => catalogue::ZH_HANS,
        }
    }

    fn plurals(self) -> catalogue::Plurals {
        match self {
            Lang::En => catalogue::EN_PLURAL,
            Lang::De => catalogue::DE_PLURAL,
            Lang::Es => catalogue::ES_PLURAL,
            Lang::Fr => catalogue::FR_PLURAL,
            Lang::Ja => catalogue::JA_PLURAL,
            Lang::Ru => catalogue::RU_PLURAL,
            Lang::Sr => catalogue::SR_PLURAL,
            Lang::SrLatn => catalogue::SR_LATN_PLURAL,
            Lang::ZhHans => catalogue::ZH_HANS_PLURAL,
        }
    }

    /// Which plural form `count` takes in this language, by the CLDR
    /// categories the catalogue is written in. German, English, Spanish and
    /// French split one from the rest (French counts 0 as one); Japanese and
    /// Chinese have one form for everything; Russian and Serbian have three,
    /// and the arithmetic is the same for both.
    pub fn plural_category(self, count: i64) -> &'static str {
        let n = count.unsigned_abs();
        match self {
            Lang::Ja | Lang::ZhHans => "other",
            Lang::Fr => {
                if n <= 1 {
                    "one"
                } else {
                    "other"
                }
            }
            Lang::En | Lang::De | Lang::Es => {
                if n == 1 {
                    "one"
                } else {
                    "other"
                }
            }
            Lang::Ru | Lang::Sr | Lang::SrLatn => {
                let (ten, hundred) = (n % 10, n % 100);
                if ten == 1 && hundred != 11 {
                    "one"
                } else if (2..=4).contains(&ten) && !(12..=14).contains(&hundred) {
                    "few"
                } else {
                    "many"
                }
            }
        }
    }
}

std::thread_local! {
    /// The reader's language, set once as the client starts. A wasm client
    /// is one thread and one reader; this is what lets the wording
    /// functions in this crate stay the shape they were — `label(outcome)`
    /// rather than `label(lang, outcome)` at two hundred call sites — and
    /// still answer in nine languages.
    static READER: std::cell::Cell<Lang> = const { std::cell::Cell::new(Lang::En) };
}

/// Read the app in this language from here on.
pub fn use_lang(lang: Lang) {
    READER.with(|held| held.set(lang));
}

pub fn lang() -> Lang {
    READER.with(|held| held.get())
}

/// What the reader's language says for `key` — and the key itself, which is
/// its English, when it says nothing.
pub fn t(key: &'static str) -> &'static str {
    text(lang(), key).unwrap_or(key)
}

/// The same, with the apps' placeholders filled in order.
pub fn t1(key: &'static str, one: &str) -> String {
    fill(t(key), &[one])
}

pub fn t2(key: &'static str, one: &str, two: &str) -> String {
    fill(t(key), &[one, two])
}

pub fn t3(key: &'static str, one: &str, two: &str, three: &str) -> String {
    fill(t(key), &[one, two, three])
}

/// A string about `count` things, in the form this language counts in — and
/// the count itself for its first `%lld`.
pub fn tn(key: &'static str, count: i64) -> String {
    tp(key, count, &[&count.to_string()])
}

/// A string about `count` things that also names one.
pub fn tn1(key: &'static str, count: i64, one: &str) -> String {
    tp(key, count, &[&count.to_string(), one])
}

/// A string about `count` things whose arguments are not in that order:
/// `args` are the KEY's own, and `count` only chooses the form. "%@. %lld
/// votes" is filled `&[option, votes]` — a translation is free to say them
/// the other way round.
pub fn tp(key: &'static str, count: i64, args: &[&str]) -> String {
    fill(form(key, count), args)
}

/// The form this language uses for `count` — and, for a language that says
/// the string the same way whatever the count, the string it says.
fn form(key: &'static str, count: i64) -> &'static str {
    let lang = lang();
    plural(lang, key, count)
        .or_else(|| text(lang, key))
        .unwrap_or(key)
}

/// What this language says for `key`, or None when it says nothing — which
/// for English is almost always, because the key IS the English.
pub fn text(lang: Lang, key: &str) -> Option<&'static str> {
    let table = lang.table();
    table
        .binary_search_by(|(held, _)| (*held).cmp(key))
        .ok()
        .map(|at| table[at].1)
}

/// What this language says for `key` about `count` things.
pub fn plural(lang: Lang, key: &str, count: i64) -> Option<&'static str> {
    let plurals = lang.plurals();
    let at = plurals
        .binary_search_by(|(held, _)| (*held).cmp(key))
        .ok()?;
    let forms = plurals[at].1;
    let wanted = lang.plural_category(count);
    forms
        .iter()
        .find(|(category, _)| *category == wanted)
        .or_else(|| forms.iter().find(|(category, _)| *category == "other"))
        .map(|(_, text)| *text)
}

/// The apps' placeholders, filled in order: `%@` takes a word, `%lld` a
/// number. They are the catalogue's own spelling, so a translator sees the
/// same string the apps' translators saw — including the one spelling only
/// a translation needs: `%2$@` names WHICH argument goes there, because
/// "%1$@ reported %2$@" is not the same sentence in every language, and the
/// words move even when the arguments cannot.
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    let mut next = 0usize;
    while let Some(at) = rest.find('%') {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        let after = &from[1..];
        if let Some(rest_after) = after.strip_prefix('%') {
            out.push('%');
            rest = rest_after;
            continue;
        }
        let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let (which, conversion) = match after[..digits].parse::<usize>() {
            Ok(place) if place >= 1 && after[digits..].starts_with('$') => {
                (Some(place - 1), &after[digits + 1..])
            }
            _ => (None, after),
        };
        let width = if conversion.starts_with('@') {
            1
        } else if conversion.starts_with("lld") {
            3
        } else {
            // Not a placeholder at all: a per-cent sign in the sentence.
            out.push('%');
            rest = after;
            continue;
        };
        let taken = which.unwrap_or(next);
        if which.is_none() {
            next += 1;
        }
        out.push_str(args.get(taken).copied().unwrap_or_default());
        rest = &conversion[width..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_finds_its_language_and_its_script() {
        assert_eq!(Lang::for_tag("de"), Some(Lang::De));
        assert_eq!(Lang::for_tag("de-AT"), Some(Lang::De));
        assert_eq!(Lang::for_tag("DE_at"), Some(Lang::De));
        assert_eq!(Lang::for_tag("sr-RS"), Some(Lang::Sr));
        assert_eq!(Lang::for_tag("sr-Latn-RS"), Some(Lang::SrLatn));
        assert_eq!(Lang::for_tag("hr"), Some(Lang::SrLatn), "read in Latin");
        assert_eq!(
            Lang::for_tag("zh-TW"),
            Some(Lang::ZhHans),
            "closer than English"
        );
        assert_eq!(Lang::for_tag("it"), None);
        assert_eq!(Lang::for_tag(""), None);
    }

    #[test]
    fn the_readers_own_order_is_kept() {
        let tags = |list: &[&str]| list.iter().map(|tag| tag.to_string()).collect::<Vec<_>>();
        assert_eq!(Lang::best(&tags(&["it", "fr-CA", "de"])), Lang::Fr);
        assert_eq!(
            Lang::best(&tags(&["it"])),
            Lang::En,
            "English is the fallback"
        );
        assert_eq!(Lang::best(&[]), Lang::En);
    }

    /// The three-form languages are the reason a plural cannot be an `if`:
    /// 1, 21 and 101 are one; 2, 3, 24 are few; 5, 11, 14, 25 are many.
    #[test]
    fn russian_and_serbian_count_in_three_forms() {
        for lang in [Lang::Ru, Lang::Sr, Lang::SrLatn] {
            assert_eq!(lang.plural_category(1), "one");
            assert_eq!(lang.plural_category(21), "one");
            assert_eq!(lang.plural_category(101), "one");
            assert_eq!(lang.plural_category(11), "many");
            assert_eq!(lang.plural_category(2), "few");
            assert_eq!(lang.plural_category(24), "few");
            assert_eq!(lang.plural_category(12), "many");
            assert_eq!(lang.plural_category(5), "many");
            assert_eq!(lang.plural_category(0), "many");
        }
        assert_eq!(Lang::En.plural_category(1), "one");
        assert_eq!(Lang::En.plural_category(0), "other");
        assert_eq!(Lang::Fr.plural_category(0), "one", "French counts 0 as one");
        assert_eq!(
            Lang::Ja.plural_category(1),
            "other",
            "Japanese has one form"
        );
    }

    /// The reader's language is the one thing this crate holds globally,
    /// and everything else here is a function of it.
    #[test]
    fn the_reader_s_language_is_used_until_it_is_changed() {
        use_lang(Lang::De);
        assert_eq!(lang(), Lang::De);
        // Nothing is translated in a test build's catalogue for this key,
        // so it answers its own English — which is the fallback that makes
        // an untranslated string readable.
        assert_eq!(
            t("Something nobody has translated"),
            "Something nobody has translated"
        );
        use_lang(Lang::En);
        assert_eq!(lang(), Lang::En);
    }

    #[test]
    fn the_apps_placeholders_are_filled_in_order() {
        assert_eq!(fill("Hi, %@", &["Anna"]), "Hi, Anna");
        assert_eq!(
            fill("%@ — %@", &["The Smiths", "Anna"]),
            "The Smiths — Anna"
        );
        assert_eq!(
            fill("%lld of %lld seats used.", &["3", "8"]),
            "3 of 8 seats used."
        );
        assert_eq!(fill("100%% sure", &[]), "100% sure");
        assert_eq!(fill("Nothing to fill", &["unused"]), "Nothing to fill");
        assert_eq!(
            fill("%@ and %@", &["one"]),
            "one and ",
            "a missing argument is empty"
        );
        assert_eq!(
            fill("50% off %@", &["today"]),
            "50% off today",
            "a lone % is kept"
        );
    }

    /// The reader's language is per thread, and a test that changes it puts
    /// it back — libtest may hand two tests the same thread.
    fn reading_in<T>(lang: Lang, read: impl FnOnce() -> T) -> T {
        let held = super::lang();
        use_lang(lang);
        let answer = read();
        use_lang(held);
        answer
    }

    #[test]
    fn a_reader_of_another_language_gets_the_apps_own_translation() {
        assert_eq!(reading_in(Lang::De, || t("Settings")), "Einstellungen");
        assert_eq!(reading_in(Lang::Ru, || t("Settings")), "Настройки");
        assert_eq!(
            reading_in(Lang::De, || t("Something nobody has translated")),
            "Something nobody has translated",
            "a key with no entry IS its English"
        );
        assert_eq!(
            t("Settings"),
            "Settings",
            "and the language is back where it was"
        );
    }

    #[test]
    fn a_count_takes_the_form_its_language_counts_in() {
        // English has one and the rest; Russian has three, and the
        // arithmetic is the catalogue's, not a guess at the ending.
        assert_eq!(tn("%lld replies", 1), "1 reply");
        assert_eq!(tn("%lld replies", 2), "2 replies");
        assert_eq!(reading_in(Lang::Ru, || tn("%lld replies", 1)), "1 ответ");
        assert_eq!(reading_in(Lang::Ru, || tn("%lld replies", 3)), "3 ответа");
        assert_eq!(reading_in(Lang::Ru, || tn("%lld replies", 7)), "7 ответов");
        assert_eq!(
            reading_in(Lang::Ru, || tn("%lld replies", 21)),
            "21 ответ",
            "twenty-one counts like one"
        );
        assert_eq!(
            reading_in(Lang::Ja, || tn("%lld replies", 1)),
            "1件の返信",
            "one form for every count"
        );
    }

    #[test]
    fn each_language_counts_the_way_it_counts() {
        let category = |lang: Lang, count: i64| lang.plural_category(count);
        // French puts zero with the singular; English does not.
        assert_eq!(category(Lang::Fr, 0), "one");
        assert_eq!(category(Lang::Fr, 1), "one");
        assert_eq!(category(Lang::Fr, 2), "other");
        assert_eq!(category(Lang::En, 0), "other");
        // Japanese and Chinese have one form for everything.
        assert_eq!(category(Lang::Ja, 1), "other");
        assert_eq!(category(Lang::ZhHans, 1), "other");
        // Russian and Serbian: the tens and the hundreds both matter.
        for lang in [Lang::Ru, Lang::Sr, Lang::SrLatn] {
            assert_eq!(category(lang, 1), "one");
            assert_eq!(category(lang, 21), "one");
            assert_eq!(category(lang, 11), "many", "eleven is not one");
            assert_eq!(category(lang, 111), "many");
            assert_eq!(category(lang, 2), "few");
            assert_eq!(category(lang, 24), "few");
            assert_eq!(category(lang, 12), "many", "twelve is not few");
            assert_eq!(category(lang, 13), "many");
            assert_eq!(category(lang, 14), "many");
            assert_eq!(category(lang, 5), "many");
            assert_eq!(category(lang, 0), "many");
            assert_eq!(category(lang, -2), "few", "a count below zero still counts");
        }
    }

    #[test]
    fn a_translation_that_reorders_the_arguments_still_gets_them_right() {
        // Chinese says the roster's size first: "5 人中 3 人已投票".
        let of = |lang| reading_in(lang, || tp("%lld of %lld voted", 3, &["3", "5"]));
        assert_eq!(of(Lang::En), "3 of 5 voted");
        assert_eq!(of(Lang::ZhHans), "5 人中 3 人已投票");
        assert_eq!(
            reading_in(Lang::Ru, || tp("%lld of %lld voted", 3, &["3", "5"])),
            "3 голоса из 5"
        );
    }

    #[test]
    fn a_plural_inside_a_sentence_is_one_form_per_count() {
        // Apple writes "%1$@. %#@arg2@" with a table for the count; the
        // catalogue holds it flattened, and the option keeps its place.
        assert_eq!(
            reading_in(Lang::De, || tp("%@. %lld votes", 1, &["Pizza", "1"])),
            "Pizza. 1 Stimme"
        );
        assert_eq!(
            reading_in(Lang::De, || tp("%@. %lld votes", 4, &["Pizza", "4"])),
            "Pizza. 4 Stimmen"
        );
    }

    #[test]
    fn a_language_that_says_a_counted_string_one_way_says_it_that_way() {
        // Russian has no plural table for this key, only a translation —
        // which must not be passed over in favour of the English key.
        assert_eq!(
            reading_in(Lang::Ru, || tp("%lld attachments, %@", 2, &["2", "1,2 МБ"])),
            "вложений: 2, 1,2 МБ"
        );
    }

    #[test]
    fn a_translation_may_say_the_arguments_in_its_own_order() {
        // German for "%@ reported %@"; the arguments cannot move, so the
        // translation names which one it wants where.
        assert_eq!(
            fill("%1$@ hat %2$@ gemeldet", &["Anna", "Bob"]),
            "Anna hat Bob gemeldet"
        );
        assert_eq!(
            fill("%2$@ was reported by %1$@", &["Anna", "Bob"]),
            "Bob was reported by Anna",
            "and may say them the other way round"
        );
        assert_eq!(
            fill("%1$@. %2$lld Stimmen", &["Pizza", "3"]),
            "Pizza. 3 Stimmen",
            "a number names its place the same way"
        );
        assert_eq!(
            fill("%1$@ twice: %1$@", &["Anna"]),
            "Anna twice: Anna",
            "and may say one of them twice"
        );
        assert_eq!(
            fill("%9$@ is nobody", &["Anna"]),
            " is nobody",
            "an argument that was never passed is empty, not a panic"
        );
        assert_eq!(
            fill("%0$@ is not a place", &["Anna"]),
            "%0$@ is not a place",
            "numbering starts at one: this is not a placeholder, so it is kept"
        );
    }
}
