//! `LookupsTests.TheConsentScreenNamesTheProvidersInOneMoreLine`, whose subject is
//! `AssistantConsent.Disclosure`: one more line on the consent screen, naming the providers, picked
//! by the owner's `ai_history` the way the two family-chat lines are — and no line at all where the
//! server names none.

mod common;

use fc_core::text::string_catalog::EnglishCatalog;
use fc_logic::assistant_consent::disclosure;

use common::consent::{PROCESSOR, strings};

#[test]
fn the_consent_screen_names_the_providers_in_one_more_line() {
    let say = EnglishCatalog::new();
    let without = disclosure(PROCESSOR, true, false, &say, false, &[]);
    let with = disclosure(
        PROCESSOR,
        true,
        false,
        &say,
        false,
        &strings(&["Brave Search", "Open-Meteo", "Wikipedia"]),
    );
    assert_eq!(without.len() + 1, with.len());
    let found: Vec<&String> = with
        .iter()
        .filter(|line| line.contains("lookups"))
        .collect();
    assert_eq!(1, found.len());
    let line = found[0];
    assert_eq!(
        "If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question — in the family chat or a group, possibly from recent messages too — to Brave Search, Open-Meteo and Wikipedia, and its answer then lists its sources.",
        line
    );
    // Before where the answer lands and how to stop.
    assert_eq!(
        Some(with.len() - 3),
        with.iter().position(|text| text == line)
    );

    let no_history = disclosure(PROCESSOR, false, false, &say, false, &strings(&["SearXNG"]));
    assert!(no_history.contains(&"If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question to SearXNG, and its answer then lists its sources.".to_string()));
    assert!(
        !no_history
            .iter()
            .any(|text| text.contains("recent messages"))
    );

    assert_eq!(
        without,
        disclosure(PROCESSOR, true, false, &say, false, &[])
    );
}
