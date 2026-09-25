//! What a composer says before a photograph goes to the assistant — and
//! the bounds that decide it (docs/protocol.md, "Pictures", "Showing the
//! assistant a picture from the family chat", "Recent photos from the
//! family chat").
//!
//! Ported from ios `Models/AssistantPictureLimits.swift`,
//! `Models/MentionPictureNotice.swift` and the private thread's sentences in
//! `MacViews/MacConversationView.swift` (`pictureNotice`), with their Swift
//! tests. The numbers are fixed by the protocol, never configured and never
//! on the wire — which is why a client may hold them: they decide a
//! sentence somebody reads before pixels leave their house, and a sentence
//! that disagreed with the server would be worse than none.
use crate::i18n::{t, t1, t2, tn};

use crate::assistant;

/// Photos off one question that reach the model. In the family chat, one
/// budget across the `@ai` message and the one it replies to.
pub const MAX_PER_QUESTION: usize = 4;

/// The largest photo that travels, after the preview is preferred. 5 MiB.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;

/// The only two encodings a chat deployment reads.
pub const ACCEPTED: [&str; 2] = ["image/jpeg", "image/png"];

/// One attachment, reduced to the three facts the rule reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub kind: String,
    /// What it will TRAVEL as — a preview is a JPEG by definition.
    pub mime: String,
    /// What it will travel as, in bytes; None when this client cannot know,
    /// and then the size rule goes unapplied rather than guessed at.
    pub bytes: Option<u64>,
}

impl Candidate {
    pub fn new(kind: &str, mime: &str, bytes: Option<u64>) -> Self {
        Candidate {
            kind: kind.to_string(),
            mime: mime.to_string(),
            bytes,
        }
    }

    /// An attachment on a message in the chat, usually somebody else's:
    /// with a preview it is judged as a JPEG of unknown size (the preview's
    /// length is not on the wire), without one by its own type and size.
    pub fn of_attachment(kind: &str, mime: &str, size: Option<u64>, has_preview: bool) -> Self {
        Candidate {
            kind: kind.to_string(),
            mime: wire_mime(mime, has_preview),
            bytes: if has_preview { None } else { size },
        }
    }
}

/// Would this be SHOWN to the model, or only named to it? Kind first: a
/// video, a file, audio or a place never reach a model, at any size.
pub fn is_shown_to_model(kind: &str, mime: &str, bytes: Option<u64>) -> bool {
    if kind != "photo" || !ACCEPTED.contains(&mime.to_ascii_lowercase().as_str()) {
        return false;
    }
    bytes.is_none_or(|bytes| bytes <= MAX_BYTES)
}

/// The type an attachment travels as: the preview when there is one.
pub fn wire_mime(mime: &str, has_preview: bool) -> String {
    if has_preview {
        "image/jpeg".to_string()
    } else {
        mime.to_string()
    }
}

/// Everything that CAN travel, before the cap.
pub fn carried(candidates: &[Candidate]) -> Vec<&Candidate> {
    candidates
        .iter()
        .filter(|candidate| is_shown_to_model(&candidate.kind, &candidate.mime, candidate.bytes))
        .collect()
}

/// May this composer offer "Show the Assistant a Photo…"? The assistant's
/// own chat, a server that can see, and a family that allows it — all
/// three (AssistantSurfaces.offersPictureAttach).
pub fn offers_picture_attach(
    is_assistant_chat: bool,
    server_can_see: bool,
    family_allows: bool,
) -> bool {
    is_assistant_chat && server_can_see && family_allows
}

/// The strip above the ASSISTANT'S chat composer while photos are staged —
/// or None when none are (MacConversationView.pictureNotice).
pub fn private_notice(staged: &[Candidate], can_see: bool) -> Option<String> {
    let photos: Vec<&Candidate> = staged
        .iter()
        .filter(|candidate| candidate.kind == "photo")
        .collect();
    if photos.is_empty() {
        return None;
    }
    if !can_see {
        return Some(
            t("The assistant on this server can't look at pictures, so it will be told a photo is here but won't be shown it.")
                .to_string(),
        );
    }
    let carried = photos
        .iter()
        .filter(|candidate| is_shown_to_model(&candidate.kind, &candidate.mime, candidate.bytes))
        .count();
    if carried < photos.len() {
        return Some(unreadable().to_string());
    }
    if photos.len() > MAX_PER_QUESTION {
        return Some(tn(
            "The first %lld photos go to the model your server is set up to use. The rest are named to it, not shown.",
            MAX_PER_QUESTION as i64,
        ));
    }
    Some(
        t("This goes to the model your server is set up to use, with your question. Nothing else from this chat does.")
            .to_string(),
    )
}

/// A function, not a const: a translated string is not a constant.
fn unreadable() -> &'static str {
    t("A photo here is too large, or in a format the model can't read, so it will be told it's here but won't be shown it.")
}

/// The strip above the FAMILY composer while an `@ai` draft carries — or
/// replies to — a photograph (MentionPictureNotice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionNotice {
    pub shown_on_mention: usize,
    pub shown_on_quote: usize,
    /// Past the shared budget: named, not shown.
    pub extra: usize,
    /// Cannot travel at all: told, never shown.
    pub unreadable: usize,
    /// How many of the chat's most recent photos may also go — None when
    /// the owner's `ai_history_photos` is not in effect.
    pub recent_up_to: Option<usize>,
}

/// The locks and switches the rule reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switches {
    pub server_can_see: bool,
    pub family_allows: bool,
    pub family_history: bool,
    pub family_history_photos: bool,
    pub server_can_draw: bool,
}

impl MentionNotice {
    pub fn shown(&self) -> usize {
        self.shown_on_mention + self.shown_on_quote
    }

    /// The notice for this draft, or None — for every reason the sentence
    /// would otherwise be a lie: a lock shut, no mention, a `/draw` request
    /// (which sends the words and nothing else), or no photograph at all.
    pub fn of(
        draft: &str,
        staged: &[Candidate],
        quoted: &[Candidate],
        switches: Switches,
    ) -> Option<MentionNotice> {
        if !switches.server_can_see || !switches.family_allows {
            return None;
        }
        if !assistant::mentions(draft) {
            return None;
        }
        if switches.server_can_draw && assistant::asks_for_picture(draft) {
            return None;
        }
        let recent_photos = switches.family_history && switches.family_history_photos;
        let on_mention: Vec<Candidate> = staged
            .iter()
            .filter(|c| c.kind == "photo")
            .cloned()
            .collect();
        let on_quote: Vec<Candidate> = quoted
            .iter()
            .filter(|c| c.kind == "photo")
            .cloned()
            .collect();
        if on_mention.is_empty() && on_quote.is_empty() && !recent_photos {
            return None;
        }
        let carried_on_mention = carried(&on_mention).len();
        let carried_on_quote = carried(&on_quote).len();
        let shown_on_mention = carried_on_mention.min(MAX_PER_QUESTION);
        let shown_on_quote = carried_on_quote.min(MAX_PER_QUESTION - shown_on_mention);
        Some(MentionNotice {
            shown_on_mention,
            shown_on_quote,
            extra: (carried_on_mention - shown_on_mention) + (carried_on_quote - shown_on_quote),
            unreadable: (on_mention.len() - carried_on_mention)
                + (on_quote.len() - carried_on_quote),
            recent_up_to: recent_photos
                .then(|| MAX_PER_QUESTION - shown_on_mention - shown_on_quote),
        })
    }

    /// What the strip says.
    pub fn sentence(&self) -> String {
        let recent = self.recent_up_to.unwrap_or(0);
        let token = assistant::TOKEN;
        if self.unreadable > 0 {
            if recent == 0 {
                return unreadable().to_string();
            }
            // Two sentences, each its own key: the apps say the second on
            // its own too, and a translation must be free to order its
            // words without reaching into the first.
            return format!(
                "{} {}",
                unreadable(),
                tn(
                    "Up to %lld of the most recent photos in this chat may still go.",
                    recent as i64,
                )
            );
        }
        if self.extra > 0 {
            return tn(
                "Only the first %lld photos go to the model your server is set up to use — yours first, then the ones you're replying to. The rest are named to it, not shown.",
                MAX_PER_QUESTION as i64,
            );
        }
        match (self.shown_on_mention > 0, self.shown_on_quote > 0) {
            (true, false) if recent > 0 => t2(
                "This goes to the model your server is set up to use, with your %@ message, and up to %lld of the most recent photos in this chat may go too.",
                token,
                &recent.to_string(),
            ),
            (true, false) => t1(
                "This goes to the model your server is set up to use, with your %@ message. No other photo in this chat does.",
                token,
            ),
            (false, true) if recent > 0 => t2(
                "The photo you're replying to goes to the model your server is set up to use, with your %@ message, and up to %lld of the most recent photos in this chat may go too.",
                token,
                &recent.to_string(),
            ),
            (false, true) => t1(
                "The photo you're replying to goes to the model your server is set up to use, with your %@ message. No other photo in this chat does.",
                token,
            ),
            (true, true) if recent > 0 => t2(
                "This and the photo you're replying to go to the model your server is set up to use, with your %@ message, and up to %lld of the most recent photos in this chat may go too.",
                token,
                &recent.to_string(),
            ),
            (true, true) => t1(
                "This and the photo you're replying to go to the model your server is set up to use, with your %@ message. No other photo in this chat does.",
                token,
            ),
            // The count comes first in the key, so it is the first argument.
            (false, false) => t2(
                "Up to %lld of the most recent photos in this chat go to the model your server is set up to use, with your %@ message — pictures nobody pointed it at, whoever sent them.",
                &recent.to_string(),
                token,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    //! MentionPictureNoticeTests.swift and AssistantPicturesTests.swift,
    //! row for row where they pin a rule.
    use super::*;

    fn photo(bytes: Option<u64>, mime: &str) -> Candidate {
        Candidate::new("photo", mime, bytes)
    }

    fn jpeg() -> Candidate {
        photo(Some(40_000), "image/jpeg")
    }

    fn video() -> Candidate {
        Candidate::new("video", "video/mp4", Some(900_000))
    }

    fn switches() -> Switches {
        Switches {
            server_can_see: true,
            family_allows: true,
            family_history: true,
            family_history_photos: false,
            server_can_draw: true,
        }
    }

    fn notice(draft: &str, staged: &[Candidate], quoted: &[Candidate]) -> Option<MentionNotice> {
        MentionNotice::of(draft, staged, quoted, switches())
    }

    fn recent(draft: &str, staged: &[Candidate], quoted: &[Candidate]) -> Option<MentionNotice> {
        MentionNotice::of(
            draft,
            staged,
            quoted,
            Switches {
                family_history_photos: true,
                ..switches()
            },
        )
    }

    fn counts(mention: usize, quote: usize, extra: usize, unreadable: usize) -> MentionNotice {
        MentionNotice {
            shown_on_mention: mention,
            shown_on_quote: quote,
            extra,
            unreadable,
            recent_up_to: None,
        }
    }

    const ASK: &str = "@ai what is this?";

    #[test]
    fn what_travels() {
        assert!(is_shown_to_model("photo", "image/jpeg", None));
        assert!(is_shown_to_model("photo", "image/png", None));
        assert!(is_shown_to_model("photo", "IMAGE/JPEG", None));
        assert!(!is_shown_to_model("photo", "image/heic", None));
        assert!(!is_shown_to_model("photo", "image/gif", None));
        assert!(!is_shown_to_model("video", "video/mp4", None));
        assert!(!is_shown_to_model("audio", "audio/mp4", None));
        assert!(!is_shown_to_model("file", "application/pdf", None));
        assert!(!is_shown_to_model("location", "application/json", None));
        assert!(is_shown_to_model("photo", "image/jpeg", Some(MAX_BYTES)));
        assert!(!is_shown_to_model(
            "photo",
            "image/jpeg",
            Some(MAX_BYTES + 1)
        ));
        assert_eq!(wire_mime("image/heic", true), "image/jpeg");
        assert_eq!(wire_mime("image/heic", false), "image/heic");
    }

    #[test]
    fn the_door_needs_all_three() {
        assert!(offers_picture_attach(true, true, true));
        assert!(!offers_picture_attach(false, true, true));
        assert!(!offers_picture_attach(true, false, true));
        assert!(!offers_picture_attach(true, true, false));
    }

    #[test]
    fn absent_with_either_lock_shut() {
        let shut = |server_can_see, family_allows| Switches {
            server_can_see,
            family_allows,
            ..switches()
        };
        assert_eq!(
            MentionNotice::of(ASK, &[jpeg()], &[], shut(false, true)),
            None
        );
        assert_eq!(
            MentionNotice::of(ASK, &[jpeg()], &[], shut(true, false)),
            None
        );
        assert_eq!(
            MentionNotice::of(ASK, &[], &[jpeg()], shut(false, false)),
            None
        );
    }

    #[test]
    fn absent_without_a_mention_or_a_photograph() {
        assert_eq!(notice("what is this?", &[jpeg()], &[]), None);
        assert_eq!(notice("", &[jpeg()], &[jpeg()]), None);
        assert_eq!(notice("mail anna@ai.example this", &[], &[jpeg()]), None);
        assert_eq!(notice(ASK, &[], &[]), None);
        assert_eq!(notice(ASK, &[video()], &[]), None);
        assert_eq!(notice(ASK, &[], &[video()]), None);
    }

    #[test]
    fn absent_for_a_draw_request_on_a_server_that_draws() {
        assert_eq!(notice("@ai /draw a cat", &[jpeg()], &[]), None);
        assert_eq!(notice("@ai /draw a cat", &[], &[jpeg()]), None);
        let no_draw = Switches {
            server_can_draw: false,
            ..switches()
        };
        assert!(MentionNotice::of("@ai /draw a cat", &[jpeg()], &[], no_draw).is_some());
        assert!(notice("@ai what does /draw do?", &[jpeg()], &[]).is_some());
    }

    #[test]
    fn present_on_the_mention_and_on_the_quote() {
        assert_eq!(notice(ASK, &[jpeg()], &[]), Some(counts(1, 0, 0, 0)));
        assert_eq!(
            notice("@AI look", &[jpeg(), jpeg()], &[]),
            Some(counts(2, 0, 0, 0))
        );
        assert_eq!(notice(ASK, &[], &[jpeg()]), Some(counts(0, 1, 0, 0)));
        assert_eq!(
            notice(ASK, &[], &[video(), jpeg(), video()]),
            Some(counts(0, 1, 0, 0))
        );
    }

    /// THE SHARED BUDGET: four across both messages, the mention's first.
    #[test]
    fn four_across_mention_and_quote_mention_first() {
        let three = [jpeg(), jpeg(), jpeg()];
        assert_eq!(notice(ASK, &three, &three), Some(counts(3, 1, 2, 0)));
        assert_eq!(
            notice(ASK, &vec![jpeg(); 5], &[jpeg()]),
            Some(counts(4, 0, 2, 0))
        );
        assert_eq!(
            notice(ASK, &[jpeg(), jpeg()], &[jpeg(), jpeg()]),
            Some(counts(2, 2, 0, 0))
        );
        assert_eq!(notice(ASK, &[], &vec![jpeg(); 6]), Some(counts(0, 4, 2, 0)));
        assert_eq!(
            notice(ASK, &three, &three).map(|n| n.shown()),
            Some(MAX_PER_QUESTION)
        );
    }

    #[test]
    fn a_photo_that_will_not_travel_is_counted_as_such() {
        let heic = photo(Some(3_000_000), "image/heic");
        let huge = photo(Some(MAX_BYTES + 1), "image/jpeg");
        assert_eq!(
            notice(ASK, std::slice::from_ref(&heic), &[]),
            Some(counts(0, 0, 0, 1))
        );
        assert_eq!(
            notice(ASK, &[], std::slice::from_ref(&huge)),
            Some(counts(0, 0, 0, 1))
        );
        assert_eq!(
            notice(ASK, &[heic, jpeg(), jpeg()], &[huge, jpeg(), jpeg()]),
            Some(counts(2, 2, 0, 2))
        );
        assert_eq!(
            notice(ASK, &[], &[photo(None, "image/jpeg")]),
            Some(counts(0, 1, 0, 0))
        );
    }

    #[test]
    fn the_sentence_names_which_photo_goes() {
        let mine = notice(ASK, &[jpeg()], &[]).unwrap().sentence();
        assert!(mine.starts_with("This goes to the model"));
        assert!(mine.contains(assistant::TOKEN));
        assert!(mine.contains("No other photo in this chat does."));
        let theirs = notice(ASK, &[], &[jpeg()]).unwrap().sentence();
        assert!(theirs.starts_with("The photo you're replying to goes"));
        let both = notice(ASK, &[jpeg()], &[jpeg()]).unwrap().sentence();
        assert!(both.starts_with("This and the photo you're replying to go"));
        let over = notice(ASK, &[jpeg(), jpeg(), jpeg()], &[jpeg(), jpeg()]).unwrap();
        assert_eq!(over.extra, 1);
        assert!(over.sentence().contains("first 4 photos"));
        assert!(over
            .sentence()
            .contains("yours first, then the ones you're replying to"));
        let unreadable = notice(ASK, &[], &[photo(Some(10), "image/heic")]).unwrap();
        assert_eq!(unreadable.sentence(), super::unreadable());
        let both = notice(ASK, &vec![jpeg(); 5], &[photo(Some(10), "image/heic")]).unwrap();
        assert!(both.extra == 1 && both.unreadable == 1);
        assert!(both.sentence().starts_with("A photo here is too large"));
    }

    #[test]
    fn recent_photos_only_with_the_third_switch_and_its_prerequisites() {
        let plain = notice(ASK, &[jpeg()], &[]).unwrap();
        assert_eq!(plain.recent_up_to, None);
        assert!(!plain.sentence().contains("most recent photos"));
        let off = |server_can_see, family_allows, family_history| Switches {
            server_can_see,
            family_allows,
            family_history,
            family_history_photos: true,
            server_can_draw: true,
        };
        assert_eq!(
            MentionNotice::of(ASK, &[], &[], off(false, true, true)),
            None
        );
        assert_eq!(
            MentionNotice::of(ASK, &[], &[], off(true, false, true)),
            None
        );
        assert_eq!(
            MentionNotice::of(ASK, &[], &[], off(true, true, false)),
            None
        );
        assert_eq!(
            MentionNotice::of(ASK, &[jpeg()], &[], off(true, true, false)),
            Some(counts(1, 0, 0, 0))
        );
    }

    #[test]
    fn a_bare_mention_shows_under_the_switch() {
        let bare = recent(ASK, &[], &[]).unwrap();
        assert_eq!(
            bare,
            MentionNotice {
                recent_up_to: Some(4),
                ..counts(0, 0, 0, 0)
            }
        );
        assert!(bare
            .sentence()
            .starts_with("Up to 4 of the most recent photos in this chat go"));
        assert!(bare.sentence().contains("nobody pointed it at"));
        assert_eq!(recent("what is this?", &[], &[]), None);
        assert_eq!(recent("@ai /draw a cat", &[], &[]), None);
    }

    #[test]
    fn history_takes_what_is_left_of_the_four() {
        for own in 0..=4 {
            let n = recent(ASK, &vec![jpeg(); own], &[]).unwrap();
            assert_eq!(n.shown_on_mention, own);
            assert_eq!(n.recent_up_to, Some(MAX_PER_QUESTION - own));
        }
        assert_eq!(
            recent(ASK, &[jpeg()], &[jpeg()]).unwrap().recent_up_to,
            Some(2)
        );
        assert_eq!(
            recent(ASK, &[], &[jpeg(), jpeg(), jpeg()])
                .unwrap()
                .recent_up_to,
            Some(1)
        );
        let over = recent(ASK, &vec![jpeg(); 5], &[]).unwrap();
        assert!(over.recent_up_to == Some(0) && over.extra == 1);
        assert!(over.sentence().starts_with("Only the first 4 photos go"));
        let heic = recent(ASK, &[photo(Some(10), "image/heic")], &[]).unwrap();
        assert!(heic.recent_up_to == Some(4) && heic.unreadable == 1);
        assert!(heic.sentence().starts_with("A photo here is too large"));
        assert!(heic
            .sentence()
            .ends_with("Up to 4 of the most recent photos in this chat may still go."));
        let spent = recent(ASK, &[jpeg(), jpeg()], &[jpeg(), jpeg()]).unwrap();
        assert_eq!(spent.recent_up_to, Some(0));
        assert_eq!(
            spent.sentence(),
            notice(ASK, &[jpeg(), jpeg()], &[jpeg(), jpeg()])
                .unwrap()
                .sentence()
        );
        let mine = recent(ASK, &[jpeg()], &[]).unwrap().sentence();
        assert!(mine.contains("up to 3 of the most recent photos in this chat may go too"));
    }

    #[test]
    fn an_attachment_is_judged_as_it_will_travel() {
        let previewed = Candidate::of_attachment("photo", "image/heic", Some(9_000_000), true);
        assert_eq!(previewed, Candidate::new("photo", "image/jpeg", None));
        let original = Candidate::of_attachment("photo", "image/heic", Some(9_000_000), false);
        assert_eq!(
            original,
            Candidate::new("photo", "image/heic", Some(9_000_000))
        );
        assert_eq!(notice(ASK, &[], &[original]).unwrap().unreadable, 1);
        assert_eq!(notice(ASK, &[], &[previewed]).unwrap().shown_on_quote, 1);
    }

    /// The private thread's sentences.
    #[test]
    fn the_assistants_own_chat_says_what_goes() {
        assert_eq!(private_notice(&[], true), None);
        assert_eq!(private_notice(&[video()], true), None);
        assert!(private_notice(&[jpeg()], false)
            .unwrap()
            .starts_with("The assistant on this server can't look at pictures"));
        assert_eq!(
            private_notice(&[photo(Some(10), "image/heic")], true).as_deref(),
            Some(super::unreadable())
        );
        assert!(private_notice(&vec![jpeg(); 5], true)
            .unwrap()
            .starts_with("The first 4 photos go"));
        assert_eq!(
            private_notice(&[jpeg()], true).as_deref(),
            Some("This goes to the model your server is set up to use, with your question. Nothing else from this chat does.")
        );
    }
}
