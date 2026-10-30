//! `FcTextCatalog` has no C# counterpart: it is the Linux port's other source of sentences, the
//! table `fc_text::i18n` compiles in, read through the same `StringCatalog` as a JSON table. These
//! hold that it answers as `JsonCatalog` does — a translation, the key for a sentence it lacks,
//! the language's plural form, and English's form for a counted sentence nobody translated.

use fc_core::text::catalogues::FcTextCatalog;
use fc_core::text::string_catalog::StringCatalog;
use fc_text::i18n::Lang;

#[test]
fn the_compiled_in_table_answers_as_a_json_table_does() {
    let russian = FcTextCatalog::new(Lang::Ru);
    assert_eq!(russian.get("No answer"), "Нет ответа");
    assert_eq!(russian.get("Dinner at 7?"), "Dinner at 7?");
    assert_eq!(
        FcTextCatalog::new(Lang::De).format("%lld of %lld done", &[&3, &7]),
        "3 von 7 erledigt"
    );
    assert_eq!(russian.plural("%lld replies", 3, &[&3]), "3 ответа");
    assert_eq!(russian.plural("%lld replies", 21, &[&21]), "21 ответ");
    assert_eq!(
        FcTextCatalog::new(Lang::En).plural("%lld replies", 1, &[&1]),
        "1 reply"
    );
    // A counted sentence no table has is its key, filled.
    assert_eq!(
        russian.plural("%lld unheard-of things", 5, &[&5]),
        "5 unheard-of things"
    );
}
