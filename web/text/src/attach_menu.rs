//! The paperclip's menu (issue #78): which items it offers, in which order
//! and in which groups — the same on every client
//! (docs/attachment-menu-2026-10-07.md, "The menu, on every client").
//!
//! ```text
//! Photo or Video        — "Show the Assistant a Photo…" INSTEAD, in the assistant chat
//! File
//! Paste
//! ──────────────
//! Record Voice Message  — not in the assistant chat
//! Record Video Message  — where round video is offered
//! ──────────────
//! Location
//! Poll                  — the family chat only
//! ```
//!
//! The web has no "Camera" item: there is no system camera to hand off to
//! (S1.5 of docs/audio-video-messages-2026-10-04.md). A group that would be
//! empty is left out, so no separator is ever drawn around nothing.
use crate::i18n::t;

/// One line of the paperclip's menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachItem {
    /// The photo-and-video picker, staged like any picked file.
    PhotoOrVideo,
    /// "Show the Assistant a Photo…" — the assistant chat's replacement for
    /// [`AttachItem::PhotoOrVideo`], images only.
    AssistantPhoto,
    /// The full file picker, photos and videos included.
    File,
    Paste,
    RecordVoice,
    RecordVideo,
    Location,
    Poll,
}

impl AttachItem {
    /// What the line says, in the reader's language.
    pub fn label(self) -> &'static str {
        match self {
            AttachItem::PhotoOrVideo => t("Photo or Video"),
            AttachItem::AssistantPhoto => t("Show the Assistant a Photo…"),
            AttachItem::File => t("File"),
            AttachItem::Paste => t("Paste"),
            AttachItem::RecordVoice => t("Record Voice Message"),
            AttachItem::RecordVideo => t("Record Video Message"),
            AttachItem::Location => t("Location"),
            AttachItem::Poll => t("Poll"),
        }
    }

    /// The emoji drawn before the label — decoration only, the label is
    /// what a screen reader says (the web's counterpart of the apps' SF
    /// Symbols `photo.on.rectangle` / `photo`, `doc`, `doc.on.clipboard`,
    /// `mic`, `video.circle`, `mappin.and.ellipse`, `chart.bar`).
    pub fn icon(self) -> &'static str {
        match self {
            AttachItem::PhotoOrVideo | AttachItem::AssistantPhoto => "🖼️",
            AttachItem::File => "📄",
            AttachItem::Paste => "📋",
            AttachItem::RecordVoice => "🎙️",
            AttachItem::RecordVideo => "📹",
            AttachItem::Location => "📍",
            AttachItem::Poll => "📊",
        }
    }
}

/// What the chat and the server allow, as the menu reads it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttachOffers {
    /// The assistant's own chat.
    pub assistant_chat: bool,
    /// "Show the Assistant a Photo…" is allowed
    /// ([`crate::assistant_pictures::offers_picture_attach`]).
    pub assistant_pictures: bool,
    /// "Record Voice Message" is offered — never in the assistant chat.
    pub record_voice: bool,
    /// "Record Video Message" is offered (`round_video::offers_video_entry`).
    pub record_video: bool,
    /// "Poll" is offered — the family chat, never a thread.
    pub poll: bool,
}

/// The menu, top to bottom, as its non-empty groups; the caller draws a
/// separator between consecutive groups.
pub fn attach_menu(offers: AttachOffers) -> Vec<Vec<AttachItem>> {
    let mut pick = Vec::new();
    if offers.assistant_pictures {
        pick.push(AttachItem::AssistantPhoto);
    } else if !offers.assistant_chat {
        pick.push(AttachItem::PhotoOrVideo);
    }
    pick.extend([AttachItem::File, AttachItem::Paste]);

    let mut record = Vec::new();
    if offers.record_voice {
        record.push(AttachItem::RecordVoice);
    }
    if offers.record_video {
        record.push(AttachItem::RecordVideo);
    }

    let mut share = vec![AttachItem::Location];
    if offers.poll {
        share.push(AttachItem::Poll);
    }

    [pick, record, share]
        .into_iter()
        .filter(|group| !group.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::AttachItem::*;
    use super::*;

    fn family() -> AttachOffers {
        AttachOffers {
            assistant_chat: false,
            assistant_pictures: false,
            record_voice: true,
            record_video: true,
            poll: true,
        }
    }

    #[test]
    fn the_family_chat_offers_everything_in_three_groups() {
        assert_eq!(
            attach_menu(family()),
            vec![
                vec![PhotoOrVideo, File, Paste],
                vec![RecordVoice, RecordVideo],
                vec![Location, Poll],
            ]
        );
    }

    #[test]
    fn a_direct_chat_has_no_poll() {
        let direct = AttachOffers {
            poll: false,
            ..family()
        };
        assert_eq!(
            attach_menu(direct),
            vec![
                vec![PhotoOrVideo, File, Paste],
                vec![RecordVoice, RecordVideo],
                vec![Location],
            ]
        );
    }

    #[test]
    fn without_round_video_the_voice_item_stands_alone() {
        let plain = AttachOffers {
            record_video: false,
            ..family()
        };
        assert_eq!(attach_menu(plain)[1], vec![RecordVoice]);
    }

    #[test]
    fn the_assistant_photo_replaces_photo_or_video_never_both() {
        let assistant = AttachOffers {
            assistant_chat: true,
            assistant_pictures: true,
            record_voice: false,
            record_video: false,
            poll: false,
        };
        assert_eq!(
            attach_menu(assistant),
            vec![vec![AssistantPhoto, File, Paste], vec![Location]]
        );
    }

    #[test]
    fn the_assistant_chat_without_pictures_offers_no_photo_line() {
        let shut = AttachOffers {
            assistant_chat: true,
            ..AttachOffers::default()
        };
        assert_eq!(attach_menu(shut), vec![vec![File, Paste], vec![Location]]);
    }

    #[test]
    fn no_group_is_ever_empty() {
        for bits in 0..32u8 {
            let offers = AttachOffers {
                assistant_chat: bits & 1 != 0,
                assistant_pictures: bits & 2 != 0,
                record_voice: bits & 4 != 0,
                record_video: bits & 8 != 0,
                poll: bits & 16 != 0,
            };
            let menu = attach_menu(offers);
            assert!(menu.iter().all(|group| !group.is_empty()), "{offers:?}");
            let items: Vec<AttachItem> = menu.into_iter().flatten().collect();
            assert!(
                !(items.contains(&PhotoOrVideo) && items.contains(&AssistantPhoto)),
                "{offers:?}"
            );
            assert_eq!(
                items.first() == Some(&File),
                offers.assistant_chat && !offers.assistant_pictures
            );
        }
    }

    #[test]
    fn every_item_has_a_label_and_an_icon() {
        for item in [
            PhotoOrVideo,
            AssistantPhoto,
            File,
            Paste,
            RecordVoice,
            RecordVideo,
            Location,
            Poll,
        ] {
            assert!(!item.label().is_empty());
            assert!(!item.icon().is_empty());
        }
        assert_eq!(File.label(), "File");
        assert_eq!(PhotoOrVideo.label(), "Photo or Video");
    }
}
