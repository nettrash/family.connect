//! The nine languages, and one language's table read from JSON (`Catalogues.cs`).
//!
//! THE FAMILY'S LANGUAGE IS NOT THE DISPLAY LANGUAGE, and that is deliberate. A family may declare
//! the one language it speaks to each other in, and what it is FOR is the assistant's answers in
//! the family chat — nothing else. Clients go on drawing their interface in whatever the DEVICE is
//! set to, because "a family setting that silently re-languaged somebody's phone would be a
//! surprise nobody asked for" (docs/protocol.md, "The family's language"). So [`Languages`] takes a
//! system language and the family's setting never reaches it. Unset is also not English: a family
//! that never chose is not a family that chose English, which is why the wire's field is nullable
//! and why nothing here defaults it.
//!
//! The tables are JSON in `win/`'s own format — one `<tag>.json` per language, a flat object of
//! English key → sentence, and `plurals.json` beside them (see [`super::plurals`]) — loaded from
//! wherever the caller keeps them ([`Catalogues::load`], [`Catalogues::from_dir`]). WHICH files the
//! Linux client ships is not decided here. [`FcTextCatalog`] is the other source a caller may use:
//! the table `fc_text::i18n` compiles in, which every `fc_text` wording function already reads.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::rc::Rc;

use fc_text::i18n::Lang;

use super::plurals::PluralTables;
use super::string_catalog::{Argument, EnglishCatalog, StringCatalog, apple_format};

/// The nine languages this app speaks, as the apps' own catalogue names them.
pub struct Languages;

impl Languages {
    /// English is not in the list of FILES: its keys are its values.
    pub const ENGLISH: &'static str = "en";

    /// Every tag the catalogue holds, English first (`fc_text::account::LANGUAGES`' order).
    pub const ALL: [&'static str; 9] = [
        Languages::ENGLISH,
        "de",
        "es",
        "fr",
        "ja",
        "ru",
        "sr",
        "sr-Latn",
        "zh-Hans",
    ];

    /// What this WINDOW draws in: the device's language, and never the family's. The family's
    /// setting is a fact about the family that the assistant reads; it is not a request to
    /// re-language anybody's computer.
    pub fn for_display(system_language: Option<&str>) -> &'static str {
        Languages::nearest(system_language)
    }

    /// The tag this app speaks for a system or family language, or English when it speaks none of
    /// it. A region is dropped (`de-AT` is German) and Serbian keeps its SCRIPT, because `sr` and
    /// `sr-Latn` are the same language in two alphabets and a reader of one cannot read the other;
    /// a Traditional Chinese reader gets Simplified, the nearer of the two wrong answers.
    ///
    /// `fc_text::i18n::Lang::for_tag`, called. It matches over ASCII whatever the machine's
    /// language (`SR-LATN` is Latin Serbian in Turkey too). Where it differs from `win/`'s own
    /// `Languages.Nearest`: it also reads `hr`, `bs` and `sh` as Latin Serbian (Windows: English),
    /// and a tag must BE `zh` before its first hyphen to be Chinese (Windows: any tag starting with
    /// `zh`).
    pub fn nearest(tag: Option<&str>) -> &'static str {
        tag.and_then(Lang::for_tag).unwrap_or_default().tag()
    }
}

/// Why a catalogue could not be loaded.
#[derive(Debug)]
pub enum CatalogueError {
    /// A file that exists could not be read.
    Io { file: String, error: std::io::Error },
    /// A file that was read is not a table of strings.
    Json {
        file: String,
        error: serde_json::Error,
    },
}

impl fmt::Display for CatalogueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogueError::Io { file, error } => write!(f, "{file}: {error}"),
            CatalogueError::Json { file, error } => write!(f, "{file}: {error}"),
        }
    }
}

impl std::error::Error for CatalogueError {}

/// One language's table (`JsonCatalog`).
///
/// A MISSING KEY ANSWERS ITSELF, AND THAT IS THE DESIGN. The key IS the English sentence, so a
/// string no translator has reached yet reads in English rather than as `chat_list_preview_hidden`
/// — which is what a client with slug keys shows its user when a table is incomplete.
///
/// A TRANSLATION MAY REORDER ITS PLACEHOLDERS. German writes "%1$lld von %2$lld erledigt" and
/// Russian "Сделано %1$lld из %2$lld", so the formatter reads positional forms as well as
/// sequential ones ([`apple_format`]).
#[derive(Debug, Clone)]
pub struct JsonCatalog {
    language: String,
    table: Rc<HashMap<String, String>>,
    plurals: Rc<PluralTables>,
}

impl JsonCatalog {
    /// Read one language's table out of a JSON object; `plurals` are the forms its counted
    /// sentences are said in (and English's, which an untranslated one falls back to).
    pub fn parse(
        json: &str,
        language: &str,
        plurals: Rc<PluralTables>,
    ) -> Result<JsonCatalog, serde_json::Error> {
        Ok(JsonCatalog {
            language: language.to_string(),
            table: Rc::new(serde_json::from_str(json)?),
            plurals,
        })
    }

    /// The tag this catalogue was loaded for.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// How many sentences it holds.
    pub fn count(&self) -> usize {
        self.table.len()
    }

    /// Whether this language actually HOLDS a sentence — which is not the same question as whether
    /// `get` answers something different from the key: "Video" in German and "Photo" in French are
    /// real translations that happen to be identical to the English, and a completeness check that
    /// compared the values would call them missing.
    pub fn holds(&self, key: &str) -> bool {
        self.table.contains_key(key)
    }
}

impl StringCatalog for JsonCatalog {
    fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.table.get(key).map_or(key, String::as_str)
    }

    fn format(&self, key: &str, arguments: &[Argument<'_>]) -> String {
        apple_format(self.get(key), arguments)
    }

    /// This language's form for the count; else its one sentence for the key, which is what a
    /// language that says it the same way whatever the count holds; else the ENGLISH form — a key
    /// nobody has translated reads as English, and English that agrees with its number.
    fn plural(&self, key: &str, count: i64, arguments: &[Argument<'_>]) -> String {
        let said = self
            .plurals
            .form(&self.language, key, count)
            .or_else(|| self.table.get(key).map(String::as_str))
            .or_else(|| self.plurals.form(Languages::ENGLISH, key, count))
            .unwrap_or(key);
        apple_format(said, arguments)
    }
}

/// What [`Catalogues::for_language`] answers: English, whose keys are its values, or a table.
#[derive(Debug, Clone)]
pub enum Catalogue {
    English(EnglishCatalog),
    Json(JsonCatalog),
}

impl Catalogue {
    /// The table, when this is not English.
    pub fn as_json(&self) -> Option<&JsonCatalog> {
        match self {
            Catalogue::English(_) => None,
            Catalogue::Json(json) => Some(json),
        }
    }
}

impl StringCatalog for Catalogue {
    fn get<'a>(&'a self, key: &'a str) -> &'a str {
        match self {
            Catalogue::English(english) => english.get(key),
            Catalogue::Json(json) => json.get(key),
        }
    }

    fn format(&self, key: &str, arguments: &[Argument<'_>]) -> String {
        match self {
            Catalogue::English(english) => english.format(key, arguments),
            Catalogue::Json(json) => json.format(key, arguments),
        }
    }

    fn plural(&self, key: &str, count: i64, arguments: &[Argument<'_>]) -> String {
        match self {
            Catalogue::English(english) => english.plural(key, count, arguments),
            Catalogue::Json(json) => json.plural(key, count, arguments),
        }
    }
}

/// The tables a build carries (`JsonCatalog.For` / `JsonCatalog.Shipped`, which read the
/// assembly's embedded resources): every language's file that was found, and the plural forms.
#[derive(Debug, Clone)]
pub struct Catalogues {
    tables: Vec<(&'static str, Rc<HashMap<String, String>>)>,
    plurals: Rc<PluralTables>,
}

impl Catalogues {
    /// The tables `read` hands over by file name — `"<tag>.json"` for each of the eight languages
    /// that are not English, and `"plurals.json"`. A file it does not have (`None`) is a language
    /// this build does not ship, or no plural forms; a file that is not a table of strings is an
    /// error, because a catalogue that quietly lost a language would draw it in English.
    pub fn load(
        mut read: impl FnMut(&str) -> Option<String>,
    ) -> Result<Catalogues, CatalogueError> {
        let plurals = match read("plurals.json") {
            Some(json) => PluralTables::parse(&json).map_err(|error| CatalogueError::Json {
                file: "plurals.json".into(),
                error,
            })?,
            None => PluralTables::empty(),
        };
        let mut tables = Vec::new();
        for tag in Languages::ALL.into_iter().skip(1) {
            let file = format!("{tag}.json");
            if let Some(json) = read(&file) {
                let table = serde_json::from_str(&json)
                    .map_err(|error| CatalogueError::Json { file, error })?;
                tables.push((tag, Rc::new(table)));
            }
        }
        Ok(Catalogues {
            tables,
            plurals: Rc::new(plurals),
        })
    }

    /// The tables in a directory laid out as `win/`'s `Text/i18n` is.
    pub fn from_dir(directory: &Path) -> Result<Catalogues, CatalogueError> {
        let mut failed = None;
        let catalogues =
            Catalogues::load(|file| match std::fs::read_to_string(directory.join(file)) {
                Ok(text) => Some(text),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    failed.get_or_insert(CatalogueError::Io {
                        file: file.to_string(),
                        error,
                    });
                    None
                }
            })?;
        match failed {
            Some(error) => Err(error),
            None => Ok(catalogues),
        }
    }

    /// The plural forms every table here says its counted sentences in.
    pub fn plurals(&self) -> Rc<PluralTables> {
        Rc::clone(&self.plurals)
    }

    /// English, with this build's English plural forms.
    pub fn english(&self) -> EnglishCatalog {
        EnglishCatalog::with_plurals(self.plurals())
    }

    /// The table this build ships for a language, or `None` for English (which has no file — its
    /// keys are its values) and for a language the build did not ship.
    pub fn json(&self, language: &str) -> Option<JsonCatalog> {
        let tag = Languages::nearest(Some(language));
        let (_, table) = self.tables.iter().find(|(held, _)| *held == tag)?;
        Some(JsonCatalog {
            language: tag.to_string(),
            table: Rc::clone(table),
            plurals: self.plurals(),
        })
    }

    /// The catalogue for this language: its table, or English — for English itself, and for a
    /// language this build did not ship (English rather than nothing).
    pub fn for_language(&self, language: &str) -> Catalogue {
        match self.json(language) {
            Some(json) => Catalogue::Json(json),
            None => Catalogue::English(self.english()),
        }
    }

    /// Every tag whose table actually ships in this build, English first.
    pub fn shipped(&self) -> Vec<&'static str> {
        Languages::ALL
            .into_iter()
            .filter(|tag| {
                *tag == Languages::ENGLISH || self.tables.iter().any(|(held, _)| held == tag)
            })
            .collect()
    }
}

/// The table `fc_text::i18n` compiles in (the apps' `Localizable.xcstrings` and `web/i18n/web.json`,
/// filtered to the keys the web client and `fc_text` say), read with the same fallbacks as
/// [`JsonCatalog`]. Every `fc_text` wording function — call records, notifications, file sizes —
/// already says its words from this table, in the language `fc_text::i18n::use_lang` set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FcTextCatalog {
    pub lang: Lang,
}

impl FcTextCatalog {
    pub fn new(lang: Lang) -> Self {
        FcTextCatalog { lang }
    }
}

impl StringCatalog for FcTextCatalog {
    fn get<'a>(&'a self, key: &'a str) -> &'a str {
        fc_text::i18n::text(self.lang, key).unwrap_or(key)
    }

    fn format(&self, key: &str, arguments: &[Argument<'_>]) -> String {
        apple_format(self.get(key), arguments)
    }

    fn plural(&self, key: &str, count: i64, arguments: &[Argument<'_>]) -> String {
        let said = fc_text::i18n::plural(self.lang, key, count)
            .or_else(|| fc_text::i18n::text(self.lang, key))
            .or_else(|| fc_text::i18n::plural(Lang::En, key, count))
            .unwrap_or(key);
        apple_format(said, arguments)
    }
}
