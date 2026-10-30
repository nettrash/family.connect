//! What the composer says above its row while a draft is written (the Mac's notices over
//! `MacComposerRow`): decided here, with no GTK, so every branch is a unit test. In the Mac's order:
//!
//! 1. **Nothing goes to the model until this member has said so** (docs/protocol.md, "Consenting to
//!    the assistant"): while the draft would reach it unagreed, "This goes to <processor>. You
//!    haven't agreed to that yet." with "Review…" — or, where the server names nobody to agree to,
//!    the sentence that says nothing can be sent to the assistant here. Not while an edit borrows
//!    the composer.
//! 2. **What a photograph will do** — in the assistant's own chat, every staged photo
//!    (`assistant_pictures::private_notice`); in the family chat or a group, a photo an `@ai` draft
//!    is about to carry, or the one it replies to (`MentionNotice`). Never both.
//! 3. **While a picture is asked for** (`/draw`), that real names and brands are often refused
//!    (`picture_hint`).

use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::app_session::SessionState;
use fc_logic::assistant_consent;
use fc_logic::groups::chat_kinds;
use fc_logic::media_outbox::StagedMedia;
use fc_logic::picture_hint;
use fc_text::assistant_pictures::{self, Candidate, MentionNotice, Switches};

/// The lines to draw, each `None` when it has nothing to say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notices {
    /// The consent line: its sentence — drawn with "Review…" — or the unnamed service's.
    pub consent: Option<Consent>,
    /// What a staged or quoted photograph will do.
    pub picture: Option<String>,
    /// The `/draw` hint.
    pub hint: Option<String>,
}

/// The first line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Consent {
    /// Not agreed yet: the sentence, and "Review…" beside it.
    Ask(String),
    /// A server that names no processor: nothing can be sent to the assistant here.
    Unnamed(String),
}

/// What the composer is about to send, as the notices read it.
pub struct Draft<'a> {
    pub chat_kind: &'a str,
    pub words: &'a str,
    pub editing: bool,
    pub staged: &'a [std::rc::Rc<StagedMedia>],
    /// The message the reply is primed on, when this device holds it.
    pub quoted: Option<&'a MessageDto>,
}

pub fn of(draft: &Draft<'_>, state: &SessionState, say: &dyn StringCatalog) -> Notices {
    let processor = state
        .assistant
        .as_ref()
        .and_then(|assistant| assistant.processor.as_deref());
    let consent = if draft.editing {
        None
    } else if assistant_consent::is_required(
        Some(draft.chat_kind),
        Some(draft.words),
        processor,
        state.assistant_consent_at.as_deref(),
    ) {
        processor.map(|processor| {
            Consent::Ask(say.format(
                "This goes to %@. You haven't agreed to that yet.",
                &[&processor],
            ))
        })
    } else if assistant_consent::is_withheld_from_an_unnamed_assistant(
        Some(draft.chat_kind),
        Some(draft.words),
        state.assistant.is_some(),
        processor,
    ) {
        Some(Consent::Unnamed(
            say.get("This server hasn't said which service answers, so nothing can be sent to the assistant here.")
                .to_string(),
        ))
    } else {
        None
    };
    Notices {
        consent,
        picture: picture(draft, state),
        hint: picture_hint::for_composer(
            Some(draft.chat_kind),
            Some(draft.words),
            state.assistant.as_ref(),
            draft.editing,
        )
        .then(|| picture_hint::sentence(say)),
    }
}

/// The photograph line: the assistant's own chat, or an `@ai` draft in the family chat or a group.
fn picture(draft: &Draft<'_>, state: &SessionState) -> Option<String> {
    let server_can_see = state.assistant.as_ref().is_some_and(|a| a.vision);
    let family = state.family.as_ref();
    let family_allows = family.is_some_and(|f| f.ai_vision);
    let staged: Vec<Candidate> = draft.staged.iter().map(|item| staged(item)).collect();
    if draft.chat_kind == chat_kinds::ASSISTANT {
        return assistant_pictures::private_notice(
            &staged,
            assistant_pictures::offers_picture_attach(true, server_can_see, family_allows),
        );
    }
    if !chat_kinds::is_shared(Some(draft.chat_kind)) {
        return None;
    }
    let quoted: Vec<Candidate> = draft
        .quoted
        .map(|message| message.media().iter().map(attached).collect())
        .unwrap_or_default();
    let switches = Switches {
        server_can_see,
        family_allows,
        family_history: family.is_some_and(|f| f.ai_history),
        family_history_photos: family.is_some_and(|f| f.ai_history_photos),
        server_can_draw: state.assistant.as_ref().is_some_and(|a| a.images),
    };
    MentionNotice::of(draft.words, &staged, &quoted, switches).map(|notice| notice.sentence())
}

/// A staged item as it will TRAVEL: its preview, a JPEG, where it has one.
fn staged(item: &StagedMedia) -> Candidate {
    let travels = item.preview.as_ref().map_or(item.bytes.len(), Vec::len);
    Candidate {
        kind: item.kind.clone(),
        mime: assistant_pictures::wire_mime(&item.mime, item.preview.is_some()),
        bytes: u64::try_from(travels).ok(),
    }
}

/// An attachment already sent: the server's preview where it has one, whose size this client
/// cannot know (the Mac's `AssistantPictureCandidate(attachment:)`).
fn attached(attachment: &AttachmentDto) -> Candidate {
    Candidate {
        kind: attachment.kind.clone(),
        mime: assistant_pictures::wire_mime(
            attachment.mime.as_deref().unwrap_or_default(),
            attachment.has_preview,
        ),
        bytes: if attachment.has_preview {
            None
        } else {
            attachment.size.and_then(|size| u64::try_from(size).ok())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_core::protocol::{AssistantDto, FamilyDto};
    use fc_core::text::string_catalog::EnglishCatalog;
    use fc_logic::app_session::Gate;

    fn state(agreed: bool) -> SessionState {
        let mut state = SessionState::new(Gate::Member);
        state.assistant = Some(AssistantDto {
            processor: Some("Example AI".into()),
            images: true,
            vision: true,
            ..Default::default()
        });
        state.assistant_consent_at = agreed.then(|| "2026-10-30T09:00:00Z".to_string());
        let mut family = FamilyDto::new(1, "The Smiths");
        family.ai_vision = true;
        state.family = Some(family);
        state
    }

    fn draft<'a>(kind: &'a str, words: &'a str) -> Draft<'a> {
        Draft {
            chat_kind: kind,
            words,
            editing: false,
            staged: &[],
            quoted: None,
        }
    }

    #[test]
    fn an_unagreed_draft_to_the_assistant_asks_with_review() {
        let say = EnglishCatalog::new();
        let notices = of(&draft("ai", "What's for dinner?"), &state(false), &say);
        assert_eq!(
            notices.consent,
            Some(Consent::Ask(
                "This goes to Example AI. You haven't agreed to that yet.".into()
            ))
        );
        // Agreed, or a draft that does not reach the model, or an edit: nothing.
        assert_eq!(of(&draft("ai", "Hi"), &state(true), &say).consent, None);
        assert_eq!(
            of(&draft("family", "Dinner at 7?"), &state(false), &say).consent,
            None
        );
        let editing = Draft {
            editing: true,
            ..draft("ai", "Hi")
        };
        assert_eq!(of(&editing, &state(false), &say).consent, None);
    }

    #[test]
    fn a_server_that_names_nobody_says_nothing_can_be_sent() {
        let say = EnglishCatalog::new();
        let mut unnamed = state(false);
        if let Some(assistant) = unnamed.assistant.as_mut() {
            assistant.processor = None;
        }
        assert!(matches!(
            of(&draft("ai", "Hi"), &unnamed, &say).consent,
            Some(Consent::Unnamed(_))
        ));
    }

    #[test]
    fn a_picture_request_carries_its_hint() {
        let say = EnglishCatalog::new();
        let asking = of(&draft("ai", "/draw a red bicycle"), &state(true), &say);
        assert_eq!(
            asking.hint.as_deref(),
            Some(
                "Describe people and things in general words — real names and brands are often refused."
            )
        );
        assert_eq!(
            of(&draft("ai", "a red bicycle"), &state(true), &say).hint,
            None
        );
    }

    #[test]
    fn a_photo_in_the_assistants_chat_says_where_it_goes() {
        let say = EnglishCatalog::new();
        let photo = std::rc::Rc::new(StagedMedia::new("photo", "image/jpeg", vec![0; 1000]));
        let staged = [photo];
        let with_photo = Draft {
            staged: &staged,
            ..draft("ai", "")
        };
        assert!(of(&with_photo, &state(true), &say).picture.is_some());
        // No photo, no line; and none in a direct chat.
        assert_eq!(of(&draft("ai", ""), &state(true), &say).picture, None);
        let direct = Draft {
            staged: &staged,
            ..draft("direct", "@ai look")
        };
        assert_eq!(of(&direct, &state(true), &say).picture, None);
    }
}
