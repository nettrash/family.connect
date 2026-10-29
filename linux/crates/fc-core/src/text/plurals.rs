//! Plural forms (`Plurals.cs`): which CLDR category a count takes, and the sentences said in
//! FORMS, one per category, per language.
//!
//! The categories are `fc_text::i18n::Lang::plural_category`, called: Russian and both Serbians
//! have THREE forms (one, few, many), French counts zero as one, and Japanese and Chinese say a
//! sentence the same way whatever the count. English is one and other, and "1 questions to the
//! assistant" is the sentence a catalogue without this says.
//!
//! The tables are read from JSON in `win/`'s format (`Text/i18n/plurals.json`, which
//! `win/i18n/generate.py` writes): `{ "<tag>": { "<key>": { "<category>": "<form>" } } }`. Which
//! file the Linux client ships is not this module's decision; it reads whatever it is given.

use std::collections::HashMap;

use fc_text::i18n::Lang;

/// `PluralRules`: a count's category in a language.
pub struct PluralRules;

impl PluralRules {
    /// The CLDR category `count` takes in `language` (any tag; matched as
    /// [`super::catalogues::Languages::nearest`] matches it).
    pub fn category(language: &str, count: i64) -> &'static str {
        Lang::for_tag(language)
            .unwrap_or_default()
            .plural_category(count)
    }
}

/// One key's forms: category → sentence.
pub type Forms = HashMap<String, String>;

/// `PluralTables`: every language's forms, by key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluralTables {
    languages: HashMap<String, HashMap<String, Forms>>,
}

impl PluralTables {
    /// No forms at all: every counted sentence falls back to its one sentence, or its key.
    pub fn empty() -> Self {
        PluralTables::default()
    }

    /// Read `plurals.json`.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        Ok(PluralTables {
            languages: serde_json::from_str(json)?,
        })
    }

    /// A key's forms in this language (an exact tag, as the file spells it), or `None` when it has
    /// none.
    pub fn forms(&self, language: &str, key: &str) -> Option<&Forms> {
        self.languages.get(language)?.get(key)
    }

    /// The form this language uses for `count` — its own category, else "other" — or `None` when
    /// the key has no forms here.
    pub fn form(&self, language: &str, key: &str, count: i64) -> Option<&str> {
        PluralTables::pick(self.forms(language, key)?, language, count)
    }

    /// One form out of a set: the count's own category, else "other" — a translator who wrote only
    /// "one" and "other" for a three-form language still has a sentence for five — else none.
    pub fn pick<'a>(forms: &'a Forms, language: &str, count: i64) -> Option<&'a str> {
        forms
            .get(PluralRules::category(language, count))
            .or_else(|| forms.get("other"))
            .map(String::as_str)
    }
}
