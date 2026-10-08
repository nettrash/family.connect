//! Looking things up (docs/protocol.md, "Looking things up", #72): what a
//! client says and asks before the assistant may send a query it wrote to
//! somebody who is not `processor`, and how it reads the sources footer the
//! server writes under such an answer.
//!
//! Three keys, each held by the person it belongs to: the operator's
//! sources (`assistant.lookups`, ABSENT — never `[]` — on a server with
//! none), the owner's `ai_lookups`, and each member's own
//! `assistant_lookup_consent_at`. This module decides the parts of that a
//! client draws, so the views only lay them out:
//!
//! - which providers are named, and how they are joined in a sentence
//!   ([`providers`], [`names`]);
//! - the line the consent screen adds, picked by `ai_history` the way the
//!   two family-chat lines are ([`disclosure_line`]);
//! - what the consent screen asks, given what this member has already
//!   agreed to ([`ask`]), and what Settings offers ([`settings`]);
//! - where the server's footer starts, so a link preview — decision 7 —
//!   never describes a source ([`split_sources`], [`previewable`]).
//!
//! The footer itself needs nothing from a client: it is plain markdown, and
//! its links are the author's markdown links every renderer here already
//! opens.

use crate::i18n::{t, t1, t2, t3};

/// The providers `assistant.lookups` names, as this client names them: each
/// trimmed, blanks dropped, in the server's order (web search, then
/// Open-Meteo, then Wikipedia). Empty means this server has no source — the
/// same as the key being absent, because a list of nobody names nobody.
pub fn providers(lookups: &[String]) -> Vec<String> {
    lookups
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Whether this server offers lookups at all — what the owner's switch, the
/// consent screen's extra line and the Settings section all hang on.
pub fn offered(lookups: &[String]) -> bool {
    !providers(lookups).is_empty()
}

/// The providers in one phrase, in the reader's language: one on its own,
/// two as "%@ and %@", three as "%@, %@ and %@". The server sends at most
/// three today; a longer list keeps every name, the ones before the last
/// two joined by commas.
pub fn names(providers: &[String]) -> String {
    match providers {
        [] => String::new(),
        [one] => one.clone(),
        [one, two] => t2("%@ and %@", one, two),
        [one, two, three] => t3("%@, %@ and %@", one, two, three),
        [first @ .., two, three] => t3("%@, %@ and %@", &first.join(", "), two, three),
    }
}

/// The consent screen's line about lookups (docs/protocol.md, "Consenting
/// to the assistant", amended 2026-10-03): with `ai_history` on, a query in
/// the family chat may be shaped by recent messages too, and the line says
/// so; with it off, it says only the question.
pub fn disclosure_line(named: &str, family_history: bool) -> String {
    if family_history {
        t1(
            "If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question — in the family chat, possibly from recent messages too — to %@, and its answer then lists its sources.",
            named,
        )
    } else {
        t1(
            "If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question to %@, and its answer then lists its sources.",
            named,
        )
    }
}

/// What the consent screen asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// The assistant alone, as before #72: this server has no source.
    /// "I Agree" / "Not Now".
    Assistant,
    /// The assistant, with lookups offered beside it and never assumed:
    /// "Agree With Lookups" / "Agree Without Lookups" / "Not Now".
    AssistantOrBoth,
    /// Lookups alone, for a member who already agreed to the assistant.
    /// "I Agree" / "Not Now".
    Lookups,
    /// Nothing is left to ask.
    Nothing,
}

/// What the consent screen asks this member. The lookup consent may only
/// stand on the assistant consent (the server answers
/// `assistant_consent_required` otherwise), so it is never asked alone of a
/// member who has not given the first.
pub fn ask(agreed_to_assistant: bool, agreed_to_lookups: bool, lookups: &[String]) -> Ask {
    match (agreed_to_assistant, offered(lookups)) {
        (false, false) => Ask::Assistant,
        (false, true) => Ask::AssistantOrBoth,
        (true, true) if !agreed_to_lookups => Ask::Lookups,
        _ => Ask::Nothing,
    }
}

/// The member's lookup row in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settings {
    /// "Review and Allow Lookups…", and what holds until they do.
    NotAllowed,
    /// The date they agreed, and "Stop Lookups".
    Allowed,
}

/// Whether Settings shows a lookup row, and which. None on a server with no
/// source, and for a member who has not agreed to the assistant: their
/// "Review and Agree…" already opens the screen that offers both, and a
/// second door that could only be refused would be a dead button.
pub fn settings(
    agreed_to_assistant: bool,
    agreed_to_lookups: bool,
    lookups: &[String],
) -> Option<Settings> {
    if !agreed_to_assistant || !offered(lookups) {
        None
    } else if agreed_to_lookups {
        Some(Settings::Allowed)
    } else {
        Some(Settings::NotAllowed)
    }
}

/// The footnote under the member's row in Settings.
pub fn settings_footnote(row: Settings, named: &str) -> String {
    match row {
        Settings::NotAllowed => t1(
            "Until you allow lookups, the assistant answers you from what it already knows, and nothing from your questions is sent to %@.",
            named,
        ),
        Settings::Allowed => t1(
            "The assistant may send a short search query or place name it writes from your questions to %@. Stopping takes effect at once; what has already been sent cannot be taken back.",
            named,
        ),
    }
}

/// The footnote under the owner's `ai_lookups` switch, naming who the
/// queries go to.
pub fn switch_footnote(named: &str) -> String {
    t1(
        "With this on, the assistant can look things up when a question needs it — the weather, the news, a fact it isn't sure of — in %@. Only a short search query or place name the assistant writes from the question is sent to them, never the conversation itself, and only when the member asking has agreed to it. Answers then list their sources. It is off unless you turn it on.",
        named,
    )
}

/// The word the server's footer opens its sources line with, in each of
/// the nine languages it writes (server/src/lookups.rs, `FooterWords`).
/// French says "Sources" as English does.
pub const SOURCES_WORDS: [&str; 8] = [
    "Sources",
    "Quellen",
    "Fuentes",
    "出典",
    "Источники",
    "Извори",
    "Izvori",
    "来源",
];

/// The separator between links, and between credits.
const DOT: &str = " · ";

/// `[title](url) · [title](url)…` — at least one link, each to http(s), and
/// nothing else on the line. A title never holds a bracket (the server
/// strips them), so the first `](` ends it; a URL has its parentheses and
/// spaces percent-encoded, so the first `)` ends it.
fn is_link_list(line: &str) -> bool {
    let mut rest = line;
    loop {
        let Some(after) = rest.strip_prefix('[') else {
            return false;
        };
        let Some(close) = after.find("](") else {
            return false;
        };
        if close == 0 {
            return false;
        }
        let after = &after[close + 2..];
        let Some(end) = after.find(')') else {
            return false;
        };
        let url = &after[..end];
        if !(url.starts_with("https://") || url.starts_with("http://"))
            || url.contains(char::is_whitespace)
        {
            return false;
        }
        rest = &after[end + 1..];
        if rest.is_empty() {
            return true;
        }
        match rest.strip_prefix(DOT) {
            Some(next) => rest = next,
            None => return false,
        }
    }
}

/// The sources line: a word from [`SOURCES_WORDS`], a colon, a space, and
/// the links.
fn is_sources_line(line: &str) -> bool {
    SOURCES_WORDS.iter().any(|word| {
        line.strip_prefix(word)
            .and_then(|rest| rest.strip_prefix(": "))
            .is_some_and(is_link_list)
    })
}

/// One credit, in any of the shapes the server writes: the weather's
/// translated words linked to Open-Meteo, Wikipedia's translated name and
/// the licence link, or Brave's own words.
fn is_credit(item: &str) -> bool {
    const WEATHER: &str = "](https://open-meteo.com/)";
    const LICENCE: &str = ", [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)";
    if item == "Powered by Brave" {
        return true;
    }
    if let Some(label) = item
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(WEATHER))
    {
        return !label.is_empty() && !label.contains(['[', ']']);
    }
    if let Some(name) = item.strip_suffix(LICENCE) {
        return !name.is_empty() && !name.contains(['[', ']']);
    }
    false
}

fn is_credit_line(line: &str) -> bool {
    !line.is_empty() && line.split(DOT).all(is_credit)
}

/// An assistant answer split at the footer the server appended after a
/// lookup: the model's words, and the footer when there is one.
///
/// The footer is the body's last paragraph — after one blank line — of a
/// sources line, a credit line, or the one then the other (a weather-only
/// answer has no sources line; a SearXNG-only one no credit line). Anything
/// else there is the answer's own and stays with it, so a body this does
/// not recognise is all answer, which is what it was before #72.
pub fn split_sources(body: &str) -> (&str, Option<&str>) {
    let Some(at) = body.rfind("\n\n") else {
        return (body, None);
    };
    let tail = &body[at + 2..];
    let lines: Vec<&str> = tail.split('\n').collect();
    let footer = match lines.as_slice() {
        [one] => is_sources_line(one) || is_credit_line(one),
        [sources, credits] => is_sources_line(sources) && is_credit_line(credits),
        _ => false,
    };
    if footer {
        (&body[..at], Some(tail))
    } else {
        (body, None)
    }
}

/// The part of a message a link-preview card may describe (decision 7:
/// source links are never turned into preview cards). For the assistant's
/// own message, its answer without the server's footer; anybody else's
/// message is all theirs, footer-shaped or not — only the assistant's
/// replies carry one the server wrote.
///
/// The web client draws no preview card today (docs/protocol.md, "How
/// sources are shown"), so nothing here fetches a source; this is the rule
/// a card must be built on, written down where the footer is read.
pub fn previewable(body: &str, from_assistant: bool) -> &str {
    if from_assistant {
        split_sources(body).0
    } else {
        body
    }
}

/// A heading the views share: the owner's section, the consent screen's
/// part and the member's row all say it.
pub fn heading() -> &'static str {
    t("Looking things up")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{use_lang, Lang};

    fn list(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// Absent and `[]` read the same — nobody named, nothing offered — and
    /// a blank name is no name.
    #[test]
    fn a_list_of_nobody_offers_nothing() {
        assert!(!offered(&[]));
        assert!(!offered(&list(&["", "  "])));
        assert!(offered(&list(&["SearXNG"])));
        assert_eq!(
            providers(&list(&[" Brave Search ", "", "Open-Meteo", "Wikipedia"])),
            list(&["Brave Search", "Open-Meteo", "Wikipedia"]),
            "trimmed, blanks dropped, the server's order kept"
        );
    }

    #[test]
    fn the_providers_are_named_in_one_phrase() {
        assert_eq!(names(&[]), "");
        assert_eq!(names(&list(&["SearXNG"])), "SearXNG");
        assert_eq!(
            names(&list(&["SearXNG", "Wikipedia"])),
            "SearXNG and Wikipedia"
        );
        assert_eq!(
            names(&list(&["Brave Search", "Open-Meteo", "Wikipedia"])),
            "Brave Search, Open-Meteo and Wikipedia"
        );
        assert_eq!(
            names(&list(&["A", "B", "C", "D"])),
            "A, B, C and D",
            "a longer list loses nobody"
        );
    }

    /// The joins are the catalogue's, so a translation joins its own way.
    #[test]
    fn the_providers_are_joined_the_readers_way() {
        let three = list(&["Brave Search", "Open-Meteo", "Wikipedia"]);
        use_lang(Lang::De);
        let german = names(&three);
        use_lang(Lang::Ja);
        let japanese = names(&three);
        let japanese_two = names(&three[..2]);
        use_lang(Lang::En);
        assert_eq!(german, "Brave Search, Open-Meteo und Wikipedia");
        assert_eq!(japanese, "Brave Search、Open-Meteo、Wikipedia");
        assert_eq!(japanese_two, "Brave SearchとOpen-Meteo");
    }

    /// With history on, the line admits that recent messages may shape the
    /// query; with it off it must not say so — saying the wrong one would
    /// be worse than saying neither.
    #[test]
    fn the_consent_line_follows_the_history_switch() {
        let with = disclosure_line("Brave Search", true);
        let without = disclosure_line("Brave Search", false);
        assert!(with.contains("possibly from recent messages too"), "{with}");
        assert!(!without.contains("recent messages"), "{without}");
        for line in [&with, &without] {
            assert!(line.contains("to Brave Search,"), "{line}");
            assert!(line.contains("lists its sources"), "{line}");
        }
    }

    #[test]
    fn the_screen_asks_what_is_left_to_ask() {
        let none: Vec<String> = Vec::new();
        let some = list(&["Wikipedia"]);
        // (agreed to the assistant, agreed to lookups, sources) → asked
        for (assistant, lookups, sources, asked) in [
            (false, false, &none, Ask::Assistant),
            (false, false, &some, Ask::AssistantOrBoth),
            // A stale lookup stamp without the first consent cannot stand.
            (false, true, &some, Ask::AssistantOrBoth),
            (true, false, &some, Ask::Lookups),
            (true, true, &some, Ask::Nothing),
            // No source here: an agreed member is asked nothing more.
            (true, false, &none, Ask::Nothing),
        ] {
            assert_eq!(
                ask(assistant, lookups, sources),
                asked,
                "{assistant} {lookups} {sources:?}"
            );
        }
    }

    #[test]
    fn settings_offers_a_row_only_where_it_can_work() {
        let some = list(&["Open-Meteo"]);
        assert_eq!(
            settings(false, false, &some),
            None,
            "the first consent first"
        );
        assert_eq!(settings(true, false, &[]), None, "no source, no row");
        assert_eq!(settings(true, false, &some), Some(Settings::NotAllowed));
        assert_eq!(settings(true, true, &some), Some(Settings::Allowed));
        assert!(settings_footnote(Settings::NotAllowed, "Open-Meteo")
            .ends_with("nothing from your questions is sent to Open-Meteo."));
        assert!(settings_footnote(Settings::Allowed, "Open-Meteo").contains("cannot be taken back"));
        assert!(switch_footnote("Open-Meteo").contains(" in Open-Meteo. "));
    }

    /// The server's own example (docs/protocol.md, "How sources are shown").
    const ANSWER: &str = "Tomorrow in Tromsø: snow showers, around −2 °C …";
    const FOOTER: &str = "Sources: [Tromsø – Wikipedia](https://en.wikipedia.org/wiki/Troms%C3%B8) · [Weather in Tromsø](https://example.org/tromso)\n[Weather data by Open-Meteo.com](https://open-meteo.com/) · Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) · Powered by Brave";

    #[test]
    fn the_servers_footer_is_found_under_the_answer() {
        let body = format!("{ANSWER}\n\n{FOOTER}");
        assert_eq!(split_sources(&body), (ANSWER, Some(FOOTER)));
        assert_eq!(previewable(&body, true), ANSWER);
        assert_eq!(
            previewable(&body, false),
            body,
            "only the assistant's replies carry a footer the server wrote"
        );
    }

    /// Each shape the server writes: sources alone (SearXNG, which is not
    /// credited), credits alone (the weather), and every language's words.
    #[test]
    fn every_shape_of_footer_is_found() {
        for footer in [
            "Sources: [One](https://a.example/1)",
            "Sources: [One · two](https://a.example/1) · [Three](http://b.example/(x%29)",
            "[Weather data by Open-Meteo.com](https://open-meteo.com/)",
            "Источники: [Тромсё — Википедия](https://ru.wikipedia.org/wiki/%D0%A2)\nВикипедия, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)",
            "出典: [東京](https://ja.wikipedia.org/wiki/x)\n[気象データ: Open-Meteo.com](https://open-meteo.com/) · ウィキペディア, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)",
            "Izvori: [Beograd](https://sr.wikipedia.org/wiki/B)\nPowered by Brave",
            "来源: [北京](https://zh.wikipedia.org/wiki/B)",
            "Quellen: [A](https://a.example/)",
            "Fuentes: [A](https://a.example/)",
            "Извори: [A](https://a.example/)",
        ] {
            let body = format!("Answer.\n\n{footer}");
            assert_eq!(split_sources(&body), ("Answer.", Some(footer)), "{footer}");
        }
    }

    /// What is not the server's footer stays the answer's: an answer that
    /// looked nothing up, a paragraph that merely mentions sources, a
    /// link to something other than the web, a third line.
    #[test]
    fn anything_else_is_the_answers_own() {
        for body in [
            "Just words.",
            "Sources: [One](https://a.example/1)",
            "Answer.\n\nSources: none, I knew it.",
            "Answer.\n\nSources: [One](javascript:alert(1))",
            "Answer.\n\nSources: [One](https://a.example/1) and more",
            "Answer.\n\nSources: [](https://a.example/1)",
            "Answer.\n\nSee [this](https://a.example/1)",
            "Answer.\n\nPowered by Brave\nSources: [One](https://a.example/1)",
            "Answer.\n\nSources: [One](https://a.example/1)\nPowered by Brave\nPowered by Brave",
            "Answer.\n\nSources: [One](https://a.example/1)\nThanks for asking",
            "Answer.\n\n[Weather](https://evil.example/)",
            "Answer.\n\nSources: [One](https://a.example/1)\n\n",
        ] {
            assert_eq!(split_sources(body), (body, None), "{body:?}");
        }
    }

    /// The footer's own links are markdown links to the URLs the server
    /// wrote — the renderer opens exactly those, and the domain inside the
    /// weather credit's label stays under the credit's own link.
    #[test]
    fn the_footer_renders_as_links_to_the_sources() {
        let rendered = crate::markdown::render(FOOTER);
        let links: Vec<(String, String)> = rendered
            .spans
            .iter()
            .filter_map(|span| {
                span.style
                    .link
                    .as_ref()
                    .map(|link| (span.text.clone(), link.destination.clone()))
            })
            .collect();
        assert_eq!(
            links,
            vec![
                (
                    "Tromsø – Wikipedia".to_string(),
                    "https://en.wikipedia.org/wiki/Troms%C3%B8".to_string()
                ),
                (
                    "Weather in Tromsø".to_string(),
                    "https://example.org/tromso".to_string()
                ),
                (
                    "Weather data by Open-Meteo.com".to_string(),
                    "https://open-meteo.com/".to_string()
                ),
                (
                    "CC BY-SA 4.0".to_string(),
                    "https://creativecommons.org/licenses/by-sa/4.0/".to_string()
                ),
            ]
        );
        assert!(
            rendered.plain().contains("Tromsø\n"),
            "the credit line is its own line: {:?}",
            rendered.plain()
        );
        assert!(rendered.plain().ends_with(" · Powered by Brave"));
    }
}
