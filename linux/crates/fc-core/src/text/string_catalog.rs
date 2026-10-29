//! Where a sentence comes from (`StringCatalog.cs`). THE KEY IS THE ENGLISH SENTENCE — the apps'
//! `Localizable.xcstrings` is the source of truth for all four clients, and its keys are the
//! English strings themselves ("Add to Calendar", "%lld of %lld done").
//!
//! Two consequences a port keeps hitting (the web client paid for both): a translated string
//! cannot live in a `const`, because it is not known until a language is loaded; and a sentence
//! matched by its beginning has to become a variant of its own rather than a prefix test, because
//! the translation may put the variable part first.
//!
//! The placeholder grammar itself is `fc_text::i18n::fill`; [`apple_format`] only turns the
//! arguments into text first.
//!
//! A CONVENTION THE CATALOGUE TESTS RELY ON: a sentence is asked of a catalogue named `say`
//! (`say.get("…")`, `say.format("…", …)`, `say.plural("…", …)`), so a scan of the source finds every
//! sentence this port draws — as `win/`'s `Get("…")` scan does.

use std::fmt::Display;
use std::rc::Rc;

use super::plurals::PluralTables;

/// One argument of a sentence: anything that can be written as text. The C# formats each with the
/// reader's culture (`Convert.ToString(argument, CultureInfo.CurrentCulture)`); Rust has no
/// culture, so an argument is written by its `Display` — a number a person reads in their own
/// format is formatted by the caller before it gets here.
pub type Argument<'a> = &'a dyn Display;

/// `IStringCatalog`: a language's sentences.
pub trait StringCatalog {
    /// The sentence for this key, or the key itself when nothing is loaded for it.
    fn get<'a>(&'a self, key: &'a str) -> &'a str;

    /// The sentence with its placeholders filled, in Apple's grammar.
    fn format(&self, key: &str, arguments: &[Argument<'_>]) -> String;

    /// A sentence about `count` things, in the form this language uses for that count.
    /// `arguments` are the KEY's own, in its order; the count only chooses the form, so a
    /// translation is free to say them the other way round.
    fn plural(&self, key: &str, count: i64, arguments: &[Argument<'_>]) -> String {
        let _ = count;
        self.format(key, arguments)
    }
}

/// Apple's placeholder grammar, which the catalogue's values are written in (`AppleFormat.Apply`).
///
/// `%@` is a string and `%lld` a number, filled in order — but a TRANSLATION may need them in
/// another order, and then it writes them POSITIONALLY: `%1$@`, `%2$lld`. Both forms appear in the
/// shipped catalogue (the German and Serbian guest lines use the positional one), so both have to
/// work, and mixing them in one sentence has to work too. `%%` is a literal per cent; anything
/// else beginning with `%` is copied through untouched rather than swallowed: a sentence with a
/// stray per cent is a sentence to show, not a panic in somebody's family chat. A placeholder with
/// no argument leaves a gap.
///
/// The grammar is `fc_text::i18n::fill`, called rather than copied. One difference from the C#,
/// which no catalogue exercises: `win/`'s `AppleFormat` also fills `%d`, `%ld`, `%f` and `%s`, and
/// `fill` knows only `%@` and `%lld` (every value in every catalogue uses only those two), so one
/// of the other four is copied through as typed.
pub fn apple_format(pattern: &str, arguments: &[Argument<'_>]) -> String {
    let texts: Vec<String> = arguments
        .iter()
        .map(|argument| argument.to_string())
        .collect();
    let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
    fc_text::i18n::fill(pattern, &borrowed)
}

/// The English catalogue: every key answers itself. This is not a stub — English is the one
/// language whose values ARE its keys, so the app's resource layer only ever has to supply the
/// other eight.
///
/// Its plural forms come from the shipped `plurals.json` ("1 question to the assistant"); without
/// them, a plural is its key, filled.
#[derive(Debug, Clone, Default)]
pub struct EnglishCatalog {
    plurals: Option<Rc<PluralTables>>,
}

impl EnglishCatalog {
    /// English with no plural tables: a counted sentence is its key, filled.
    pub fn new() -> Self {
        EnglishCatalog::default()
    }

    /// English that says a counted sentence in the form its count takes.
    pub fn with_plurals(plurals: Rc<PluralTables>) -> Self {
        EnglishCatalog {
            plurals: Some(plurals),
        }
    }
}

impl StringCatalog for EnglishCatalog {
    fn get<'a>(&'a self, key: &'a str) -> &'a str {
        key
    }

    fn format(&self, key: &str, arguments: &[Argument<'_>]) -> String {
        apple_format(key, arguments)
    }

    fn plural(&self, key: &str, count: i64, arguments: &[Argument<'_>]) -> String {
        let form = self
            .plurals
            .as_ref()
            .and_then(|plurals| plurals.form(super::catalogues::Languages::ENGLISH, key, count));
        apple_format(form.unwrap_or(key), arguments)
    }
}
