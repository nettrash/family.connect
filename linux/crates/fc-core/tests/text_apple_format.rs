//! `AppleFormatTests`: Apple's placeholder grammar, which the shipped catalogue's values are
//! written in — and the trap the web port hit: a TRANSLATION may need the arguments in another
//! order, and then it writes them positionally.

use fc_core::text::string_catalog::{EnglishCatalog, StringCatalog, apple_format};

#[test]
fn placeholders_fill_in_order() {
    assert_eq!(apple_format("%lld of %lld done", &[&2, &5]), "2 of 5 done");
    assert_eq!(
        apple_format("%@ mentioned you", &[&"Anna"]),
        "Anna mentioned you"
    );
    assert_eq!(
        apple_format("%lld going, %lld maybe", &[&2, &1]),
        "2 going, 1 maybe"
    );
}

/// The German and Serbian sentences in the shipped catalogue are written this way.
#[test]
fn a_translation_may_put_them_in_any_order() {
    assert_eq!(
        apple_format("%2$lld von %1$lld erledigt", &[&2, &5]),
        "5 von 2 erledigt"
    );
    assert_eq!(
        apple_format("%1$@: %2$@", &[&"Anna", &"hello"]),
        "Anna: hello"
    );
    // Mixed forms in one sentence: the unpositioned ones keep their own running order.
    assert_eq!(apple_format("%2$@ %@ %2$@", &[&"a", &"b"]), "b a b");
}

#[test]
fn a_stray_per_cent_is_shown_rather_than_thrown() {
    assert_eq!(apple_format("100%% full", &[]), "100% full");
    // Not a placeholder this grammar knows: copied through, because a sentence with an odd per
    // cent in it is a sentence to show, not a panic in somebody reading their family's chat.
    assert_eq!(apple_format("50% off", &[]), "50% off");
    assert_eq!(apple_format("%", &[]), "%");
    assert_eq!(apple_format("% ", &[]), "% ");
}

#[test]
fn a_missing_argument_leaves_a_gap_and_never_throws() {
    assert_eq!(apple_format("%lld of %lld done", &[]), " of  done");
    assert_eq!(apple_format("%lld of %lld done", &[&2]), "2 of  done");
    assert_eq!(apple_format("%99$@", &[&"a"]), "");
}

#[test]
fn the_english_catalogue_answers_its_own_keys() {
    let english = EnglishCatalog::new();
    assert_eq!(english.get("Add to Calendar"), "Add to Calendar");
    assert_eq!(
        english.format("%lld of %lld done", &[&2, &5]),
        "2 of 5 done"
    );
}
