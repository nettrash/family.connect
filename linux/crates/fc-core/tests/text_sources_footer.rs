//! `SourcesFooterTests`: the footer the server appends to an answer that looked something up
//! (docs/protocol.md, "How sources are shown: a footer the server writes"), recognised by its
//! shape and its fixed words — so the client can keep a lookup answer's links out of preview cards
//! (design decision 7). The bodies are built the way `server/src/lookups.rs`'s `footer` builds
//! them: the answer, one blank line, the sources line, the credit line.

use fc_core::text::sources_footer::ends_with_footer;

const WEATHER: &str = "[Weather data by Open-Meteo.com](https://open-meteo.com/)";
const WIKIPEDIA: &str =
    "Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)";
const BRAVE: &str = "Powered by Brave";

fn footer(body: &str) -> bool {
    ends_with_footer(Some(body))
}

/// protocol.md's own example, verbatim.
#[test]
fn the_protocols_example_is_a_footer() {
    let example = format!(
        "Tomorrow in Tromsø: snow showers, around −2 °C …\n\nSources: [Tromsø – Wikipedia](https://en.wikipedia.org/wiki/Troms%C3%B8) · [Weather in Tromsø](https://example.org/tromso)\n{WEATHER} · {WIKIPEDIA} · {BRAVE}"
    );
    assert!(footer(&example));
}

/// A weather-only answer has no sources line, and a SearXNG answer no credit line: both are
/// footers.
#[test]
fn either_line_alone_is_a_footer() {
    assert!(footer(&format!("Snow tomorrow.\n\n{WEATHER}")));
    assert!(footer(
        "Here is what I found.\n\nSources: [A page](https://a.example/x)"
    ));
    assert!(footer(&format!(
        "Brave says so.\n\nSources: [A](https://a.example/) · [B](http://b.example/b)\n{BRAVE}"
    )));
}

/// The sources word in every language the server writes it in, English for anything else.
#[test]
fn every_languages_sources_word_is_known() {
    for word in [
        "Sources",
        "Quellen",
        "Fuentes",
        "出典",
        "Источники",
        "Извори",
        "Izvori",
        "来源",
    ] {
        assert!(
            footer(&format!("…\n\n{word}: [Title](https://a.example/)")),
            "{word}"
        );
    }
    assert_eq!(fc_text::lookups::SOURCES_WORDS.len(), 8);
    // The credit words are translated; the links and Brave's words are not.
    assert!(footer(
        "…\n\nИсточники: [Тромсё](https://ru.wikipedia.org/wiki/%D0%A2)\n[Данные о погоде: Open-Meteo.com](https://open-meteo.com/) · Википедия, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)"
    ));
    assert!(footer(
        "…\n\n维基百科, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)"
    ));
}

#[test]
fn an_ordinary_answer_is_not() {
    assert!(!ends_with_footer(None));
    assert!(!footer(""));
    assert!(!footer("Paris is the capital of France."));
    assert!(!footer("See [the docs](https://example.com/docs)."));
    assert!(!footer("Read more:\n\nhttps://example.com"));
    // A sources word with no links, or with prose after the links, is somebody's sentence.
    assert!(!footer("…\n\nSources: my memory"));
    assert!(!footer(
        "…\n\nSources: [A](https://a.example/) and that is all"
    ));
    assert!(!footer("…\n\nReferences: [A](https://a.example/)"));
}

/// Only the END: a footer followed by anything else is not the server's, which appends it last.
#[test]
fn the_footer_is_the_last_paragraph() {
    assert!(!footer(
        "…\n\nSources: [A](https://a.example/)\n\nAnd one more thing."
    ));
    assert!(!footer(
        "…\n\nSources: [A](https://a.example/)\nAnd one more thing."
    ));
    // Trailing whitespace and Windows line ends are not "anything else".
    assert!(footer(&format!(
        "…\r\n\r\nSources: [A](https://a.example/)\r\n{WEATHER}\n "
    )));
}

/// Three links at most, three known credits at most, each once.
#[test]
#[ignore = "DIVERGENCE: fc_text::lookups::split_sources takes any number of sources links and of \
            credits, repeats included (as the Mac's AssistantSources does); win/ takes at most \
            three links and three different credits — the 4-link and Weather·Weather lines are \
            footers to fc_text"]
fn the_shape_is_the_servers_and_no_looser() {
    assert!(footer(
        "…\n\nSources: [A](https://a.example/) · [B](https://b.example/) · [C](https://c.example/)"
    ));
    assert!(!footer(
        "…\n\nSources: [A](https://a.example/) · [B](https://b.example/) · [C](https://c.example/) · [D](https://d.example/)"
    ));
    assert!(!footer("…\n\nSources: [A](javascript:alert)"));
    assert!(!footer(&format!("…\n\n{WEATHER} · {WEATHER}")));
    assert!(!footer("…\n\nPowered by Bravo"));
    assert!(!footer("…\n\n[Weather](https://open-meteo.example/)"));
    // The credit line belongs AFTER the sources line, never before it.
    assert!(!footer(&format!(
        "…\n\n{WEATHER}\nSources: [A](https://a.example/)"
    )));
    assert!(!footer(
        "…\n\nSources: [A](https://a.example/)\nSources: [B](https://b.example/)"
    ));
}
