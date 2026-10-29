//! The nine languages (shell-design §23): which one the desktop asks for, and the tables that say
//! it.
//!
//! **The tables** are `linux/i18n/generate.py`'s output — `<data>/i18n/<lang>.json` for the eight
//! that are not English, and `plurals.json` — read by fc-core's `Catalogues::from_dir` from
//! [`Paths::i18n_dir`] (beside the executable first, then `/usr/share/family-connect/i18n`). They
//! hold exactly the sentences this port draws: the apps' `Localizable.xcstrings`, the port's own
//! `linux/i18n/linux.json`, and the Windows port's `win/i18n/win.json` reused verbatim.
//!
//! **The language is the desktop's, never the family's** (`win/README.md`): the first of
//! `glib::language_names()` (LANGUAGE, LC_ALL, LC_MESSAGES, LANG, in that order) that names one of
//! the nine — `sr@latin`/`sr_RS@latin` → `sr-Latn`, `sr*` → `sr`, `zh*` → `zh-Hans` (Simplified is
//! the nearer wrong answer for a Traditional reader), otherwise the base language; a language the
//! app does not speak gives way to the next one asked for; `C` and nothing at all are English.
//!
//! **One language for everything said.** [`Catalogue::for_desktop`] also calls
//! `fc_text::i18n::use_lang`, so the wordings `fc-text` produces (call records, notifications, file
//! sizes) are in the same language as the catalogue's (carry-forward, "Languages"). GNOME changes
//! the language at the next login, so the app reads it once, at start-up.
//!
//! **The calling convention** the catalogue tests and the generator rely on: a sentence is asked of
//! a value named `say` — `say.get("…")`, `say.format("…", &[…])`, `say.plural("…", n, &[…])` —
//! with the English sentence as a plain string literal. `shell.say.get("…")` counts; a catalogue
//! under any other name is invisible to the string gate, and its sentence is never translated.

use std::path::Path;

use fc_core::text::catalogues::{self, Catalogues, Languages};
use fc_core::text::string_catalog::{Argument, StringCatalog};
use fc_text::i18n::Lang;

use super::paths::Paths;

/// The app's catalogue: one language's table (or English), and the language it is in.
#[derive(Debug, Clone)]
pub struct Catalogue {
    lang: Lang,
    inner: catalogues::Catalogue,
}

impl Catalogue {
    /// The desktop's language from the tables under `paths` — and `fc_text` set to the same. A
    /// missing or unreadable table set is English, with one warning: the app must start even when
    /// an install lost its data folder.
    pub fn for_desktop(paths: &Paths) -> Catalogue {
        let catalogue = Catalogue::load(&paths.i18n_dir(), desktop_language());
        fc_text::i18n::use_lang(catalogue.lang);
        catalogue
    }

    /// `language` (one of [`Languages::ALL`]) from the tables in `dir`; English when they cannot
    /// be read. Does not touch `fc_text`'s language.
    pub fn load(dir: &Path, language: &str) -> Catalogue {
        let lang = Lang::for_tag(language).unwrap_or_default();
        match Catalogues::from_dir(dir) {
            Ok(tables) => {
                let inner = tables.for_language(lang.tag());
                // A language without its table is English through and through — `fc_text` too,
                // so one window never mixes two languages.
                let lang = match inner.as_json() {
                    Some(_) => lang,
                    None if lang == Lang::En => lang,
                    None => {
                        crate::fc_warn!(
                            "no {} table in the data folder; English instead",
                            lang.tag()
                        );
                        Lang::En
                    }
                };
                Catalogue { lang, inner }
            }
            Err(error) => {
                // The error names a file of the app's own data folder — no user data.
                crate::fc_warn!("the string tables could not be read ({error}); English instead");
                Catalogue::english()
            }
        }
    }

    /// English, whose keys are its values (no tables needed).
    pub fn english() -> Catalogue {
        Catalogue {
            lang: Lang::En,
            inner: catalogues::Catalogue::English(Default::default()),
        }
    }

    /// Its tag: `en`, `de`, …, `sr-Latn`, `zh-Hans`.
    pub fn language(&self) -> &'static str {
        self.lang.tag()
    }
}

impl StringCatalog for Catalogue {
    fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.inner.get(key)
    }

    fn format(&self, key: &str, arguments: &[Argument<'_>]) -> String {
        self.inner.format(key, arguments)
    }

    fn plural(&self, key: &str, count: i64, arguments: &[Argument<'_>]) -> String {
        self.inner.plural(key, count, arguments)
    }
}

/// The desktop's language as one of the nine.
pub fn desktop_language() -> &'static str {
    let names: Vec<String> = glib::language_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    language_for(&names)
}

/// The locale dates and times are written in (`LC_TIME`, else `LC_ALL`/`LANG` — GLib's order), for
/// `fc_logic::date_culture::DateCulture::for_locale`. `C` when nothing is set.
pub fn time_locale() -> String {
    glib::language_names_with_category("LC_TIME")
        .first()
        .map(|name| name.to_string())
        .unwrap_or_else(|| "C".to_string())
}

/// The locale the reader's numbers are written in (`LC_NUMERIC`, then `LC_ALL`/`LANG` as GLib
/// orders them), as a POSIX name; `C` when none is set.
pub fn numeric_locale() -> String {
    glib::language_names_with_category("LC_NUMERIC")
        .first()
        .map(|name| name.to_string())
        .unwrap_or_else(|| "C".to_string())
}

/// How the reader writes a number (`Shell::numbers`): the decimal separator and the thousands
/// group of the `LC_NUMERIC` locale. GLib exposes no `localeconv`, so they are the numeric
/// conventions of the locale's LANGUAGE (CLDR's, for the nine): one table for the whole shell —
/// P2's file sizes, P1's staged chips, P4's statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Numbers {
    /// Between the whole and the fraction: `.` or `,`.
    pub decimal: &'static str,
    /// Between groups of three digits: `,`, `.`, a narrow no-break space (French), a no-break
    /// space (Russian).
    pub group: &'static str,
    /// The fewest digits a number has before it is grouped at all (CLDR's minimum grouping digits
    /// + 3: Spanish writes 1234 but 12.345).
    pub group_from: usize,
}

impl Numbers {
    /// The conventions of the POSIX locale `name` (`de_DE.UTF-8`, `sr_RS@latin`, `C`). Pure.
    pub fn for_locale(name: &str) -> Numbers {
        let tag = posix_tag(name).unwrap_or_default();
        let language = tag.split('-').next().unwrap_or_default();
        let (decimal, group, group_from) = match language {
            "de" | "sr" => (",", ".", 4),
            "es" => (",", ".", 5),
            "fr" => (",", "\u{202F}", 4),
            "ru" => (",", "\u{00A0}", 4),
            _ => (".", ",", 4),
        };
        Numbers {
            decimal,
            group,
            group_from,
        }
    }

    /// `value` with its digits grouped by three (`12,345`, `12 345`; `-1,000`).
    pub fn group(&self, value: i64) -> String {
        let digits = value.unsigned_abs().to_string();
        let mut out = String::with_capacity(digits.len() + 8);
        if value < 0 {
            out.push('-');
        }
        if digits.len() < self.group_from {
            out.push_str(&digits);
            return out;
        }
        for (at, digit) in digits.chars().enumerate() {
            if at > 0 && (digits.len() - at).is_multiple_of(3) {
                out.push_str(self.group);
            }
            out.push(digit);
        }
        out
    }
}

/// The first of `names` (POSIX locale names in preference order, as `glib::language_names` gives
/// them) that names one of the nine; English when none does. Pure.
pub fn language_for(names: &[&str]) -> &'static str {
    names
        .iter()
        .filter_map(|name| posix_tag(name))
        .find_map(|tag| Lang::for_tag(&tag))
        .unwrap_or_default()
        .tag()
}

/// A POSIX locale name (`language[_territory][.codeset][@modifier]`) as a BCP 47 tag
/// (`sr_RS.UTF-8@latin` → `sr-Latn-RS`); `None` for `C`, `POSIX` and an empty name.
fn posix_tag(name: &str) -> Option<String> {
    let (base, modifier) = match name.split_once('@') {
        Some((base, modifier)) => (base, Some(modifier)),
        None => (name, None),
    };
    let base = base.split('.').next().unwrap_or(base).trim();
    if base.is_empty() || base.eq_ignore_ascii_case("C") || base.eq_ignore_ascii_case("POSIX") {
        return None;
    }
    let mut parts = base.split(['_', '-']);
    let language = parts.next()?.to_ascii_lowercase();
    let rest: Vec<&str> = parts.collect();
    let script = match modifier {
        Some(m) if m.eq_ignore_ascii_case("latin") => Some("Latn"),
        Some(m) if m.eq_ignore_ascii_case("cyrillic") => Some("Cyrl"),
        _ => None,
    };
    let mut tag = language;
    if let Some(script) = script {
        tag.push('-');
        tag.push_str(script);
    }
    for part in rest {
        tag.push('-');
        tag.push_str(part);
    }
    Some(tag)
}

/// Every language the tables in `dir` ship, English first (the self-test's check).
#[cfg_attr(
    not(any(test, feature = "selftest")),
    allow(dead_code, reason = "the self-test's check")
)]
pub fn shipped(dir: &Path) -> Vec<&'static str> {
    Catalogues::from_dir(dir)
        .map(|tables| tables.shipped())
        .unwrap_or_else(|_| vec![Languages::ENGLISH])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn numbers_follow_the_numeric_locales_conventions() {
        let english = Numbers::for_locale("C");
        assert_eq!(
            (english.decimal, english.group(1_234_567)),
            (".", "1,234,567".to_string())
        );
        assert_eq!(english.group(-1000), "-1,000");
        assert_eq!(english.group(999), "999");
        let german = Numbers::for_locale("de_DE.UTF-8");
        assert_eq!(
            (german.decimal, german.group(12_345)),
            (",", "12.345".to_string())
        );
        let spanish = Numbers::for_locale("es_ES.UTF-8");
        assert_eq!(spanish.group(1234), "1234");
        assert_eq!(spanish.group(12_345), "12.345");
        assert_eq!(
            Numbers::for_locale("fr_FR.UTF-8").group(12_345),
            "12\u{202F}345"
        );
        assert_eq!(
            Numbers::for_locale("ru_RU.UTF-8").group(12_345),
            "12\u{00A0}345"
        );
        assert_eq!(Numbers::for_locale("sr_RS@latin").decimal, ",");
        assert_eq!(Numbers::for_locale("ja_JP.UTF-8").decimal, ".");
        assert_eq!(Numbers::for_locale("zh_CN.UTF-8").group(10_000), "10,000");
    }

    fn tables() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../data/i18n")
    }

    #[test]
    fn the_desktops_first_spoken_language_wins() {
        assert_eq!(
            language_for(&["de_DE.UTF-8", "de_DE", "de.UTF-8", "de", "C"]),
            "de"
        );
        assert_eq!(language_for(&["en_GB.UTF-8", "en_GB", "en", "C"]), "en");
        assert_eq!(language_for(&["fr_CA", "fr"]), "fr");
        assert_eq!(language_for(&["ja_JP.UTF-8"]), "ja");
        assert_eq!(language_for(&["ru_RU.UTF-8", "ru"]), "ru");
        assert_eq!(language_for(&["es_MX"]), "es");
    }

    #[test]
    fn serbian_keeps_its_script() {
        assert_eq!(language_for(&["sr_RS@latin", "sr@latin", "sr"]), "sr-Latn");
        assert_eq!(language_for(&["sr_RS.UTF-8@latin"]), "sr-Latn");
        assert_eq!(language_for(&["sr@latin"]), "sr-Latn");
        assert_eq!(language_for(&["sr_RS.UTF-8", "sr"]), "sr");
        assert_eq!(language_for(&["sr_ME"]), "sr");
        assert_eq!(language_for(&["sr_RS@cyrillic"]), "sr");
    }

    #[test]
    fn chinese_is_simplified() {
        assert_eq!(language_for(&["zh_CN.UTF-8"]), "zh-Hans");
        assert_eq!(language_for(&["zh_SG"]), "zh-Hans");
        assert_eq!(language_for(&["zh_TW.UTF-8"]), "zh-Hans");
    }

    #[test]
    fn a_language_the_app_does_not_speak_gives_way_to_the_next() {
        assert_eq!(language_for(&["pt_BR.UTF-8", "pt_BR", "pt", "C"]), "en");
        assert_eq!(language_for(&["pt_BR", "pt", "fr_FR", "fr"]), "fr");
        assert_eq!(language_for(&["C.UTF-8", "C"]), "en");
        assert_eq!(language_for(&["POSIX"]), "en");
        assert_eq!(language_for(&[]), "en");
        assert_eq!(language_for(&[""]), "en");
    }

    #[test]
    fn the_generated_tables_ship_all_nine_and_say_the_ports_own_sentences() {
        assert_eq!(shipped(&tables()), Languages::ALL);
        let german = Catalogue::load(&tables(), "de");
        assert_eq!(german.language(), "de");
        assert_eq!(german.get("Start when I log in"), "Beim Anmelden starten");
        // A sentence of the apps' own catalogue, through the same table.
        assert_eq!(german.get("Settings…"), "Einstellungen …");
        let serbian = Catalogue::load(&tables(), "sr-Latn");
        assert_eq!(serbian.get("Main Menu"), "Glavni meni");
        // A key no table holds reads as itself: the key is the English.
        assert_eq!(
            german.get("Not a sentence anybody draws"),
            "Not a sentence anybody draws"
        );
    }

    #[test]
    fn missing_tables_are_english_not_a_failure() {
        let nowhere = std::env::temp_dir().join(format!("fc-no-tables-{}", std::process::id()));
        let catalogue = Catalogue::load(&nowhere, "ru");
        // No `plurals.json` and no `ru.json`: the loader finds no files, which is English.
        assert_eq!(catalogue.language(), "en");
        assert_eq!(catalogue.get("Start when I log in"), "Start when I log in");
        let english = Catalogue::english();
        assert_eq!(english.format("%lld of %lld", &[&1, &2]), "1 of 2");
    }
}
