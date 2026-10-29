//! The footer the SERVER appends to an assistant answer that looked something up
//! (`SourcesFooter.cs`; docs/protocol.md, "How sources are shown: a footer the server writes"):
//! one blank line after the answer, then a sources line — the word "Sources" in the reply's
//! language, a colon, and `[title](url)` links separated by ` · ` — and a credit line naming each
//! provider whose result reached the model.
//!
//! It is plain markdown, so a bubble already draws it and its links already open; what a client
//! needs to KNOW is only whether an answer carries one, because such an answer's links are kept
//! out of preview cards (design decision 7), so a cited page is not contacted by every device that
//! shows the answer.
//!
//! The shape is `fc_text::lookups::split_sources` (and its `SOURCES_WORDS`), called. What the C#
//! adds is the reading of the body before the shape is asked: Windows line ends are line ends, and
//! whitespace after the footer is not "anything else after it" — as the Mac's `AssistantSources`
//! reads it too.
//!
//! Where `win/` is stricter than `fc_text` (and the Mac): it takes at most THREE sources links and
//! at most three DIFFERENT credits, as the server writes them; `fc_text` takes any number of
//! either. `SourcesFooterTests::TheShapeIsTheServersAndNoLooser` pins the difference (ignored here,
//! and reported).

/// Does this body END with the server's sources footer — its last paragraph, and nothing after it?
pub fn ends_with_footer(body: Option<&str>) -> bool {
    let Some(body) = body.filter(|body| !body.is_empty()) else {
        return false;
    };
    let text = body.replace("\r\n", "\n");
    fc_text::lookups::split_sources(text.trim_end()).1.is_some()
}
