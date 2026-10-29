//! The text rules `fc-text` does not have: the rest of `win/src/FamilyConnect.Core/Text`.
//!
//! `fc_text` is a dependency, not a source to copy (`linux/docs/port-plan.md`): where a C# file
//! has a counterpart there, the client calls `fc_text` and nothing is ported; where a file adds a
//! little on top, only the addition is here. The 40 files of `Text/`:
//!
//! | C# file | Here | `fc_text` |
//! | --- | --- | --- |
//! | `AssistantPictures.cs` | — | `assistant_pictures` (`HouseRules.MaxPicturesPerQuestion` is its `MAX_PER_QUESTION`) |
//! | `AssistantText.cs` | — | `assistant` (`BeginsPictureRequest` = `starts_picture_request`) |
//! | `AttachmentText.cs` | — | `media::display_name` |
//! | `AvatarPrep.cs` | — | `avatar::{square, EDGE, QUALITIES, MAX_BYTES}` |
//! | `AvatarText.cs` | — | `avatar::initials`; `Uppercase` is `str::to_uppercase` |
//! | `Calendar.cs` | [`calendar`]: `stamp`, `FOLD_AT` | `calendar::one_event` |
//! | `CallRecordText.cs` | — | `call_record` (`Decide`/`Say` = `CallRecordLine::new`/`said`) |
//! | `Catalogues.cs` | [`catalogues`]: `JsonCatalog`, `Catalogues`, `Languages` | `i18n::Lang::for_tag` (`Nearest`) |
//! | `Emoji.cs` | — | `emoji` |
//! | `EmojiCatalog.cs` | — | `emoji::EMOJI_CATALOG` |
//! | `Excerpt.cs` | — | `excerpt` |
//! | `GreetingPlaces.cs` | [`greeting_places`]: `length`, `judge` | `greeting_places::{fold, typed, places}` (`Check` ≈ `places`) |
//! | `HouseRules.cs` | [`house_rules`]: policies, captions, notes | `account::{cap_state, cap_footer, clamp_cap, seed_cap, LANGUAGES}` |
//! | `LinkPreview.cs` | [`link_preview`]: all of it | — |
//! | `Links.cs`, `LinksNumbers.cs`, `LinkTables.g.cs` | — | `links` |
//! | `LocationFixes.cs` | [`location_fixes`]: all of it | — (web `location.rs`) |
//! | `Markdown.cs`, `MarkdownInline.cs`, `MarkdownModel.cs`, `MarkdownScanners.cs`, `MarkdownTables.g.cs` | — | `markdown` |
//! | `MediaPlan.cs` | — | `media_plan` |
//! | `MediaPrep.cs` | — | `media` |
//! | `MediaText.cs` | [`media_text`]: `display_size` in the reader's separator | `media` |
//! | `Mentions.cs` | [`mentions`]: `runs`, `MAX_PER_MESSAGE` | `mentions` (`IsBoundary` = `assistant::is_boundary`) |
//! | `MessageBody.cs` | [`message_body`]: all of it | — (web `views/body.rs`) |
//! | `NotifyText.cs` | — | `notify` (`WindowTitle` = `page_title`) |
//! | `PasteRules.cs` | — | `media::{paste_decision, chosen_paste_type, pasted_name, PASTE_PREFERENCE}` |
//! | `Plurals.cs` | [`plurals`]: `PluralTables` | `i18n::Lang::plural_category` |
//! | `Polls.cs` | [`polls`]: all of it | — (web `views/poll.rs`) |
//! | `Reactions.cs` | — | `reactions`, `emoji::{QUICK_REACTIONS, DOUBLE_TAP_REACTION, capsule_emojis}` |
//! | `RustCase.cs`, `RustChar.cs`, `UnicodeProperties.g.cs` | not ported: Rust's own `char`/`str` | — |
//! | `SourcesFooter.cs` | [`sources_footer`]: line ends and trailing space | `lookups::{split_sources, SOURCES_WORDS}` |
//! | `StickerFile.cs` | [`sticker_file`]: `size` | `pack::{type_of, is_sticker_type, is_gif, is_animated}`, `animation_source::webp` |
//! | `StringCatalog.cs` | [`string_catalog`]: the trait, English, `apple_format` | `i18n::fill` |
//! | `Waveform.cs` | — | `waveform` |
//!
//! `Text/i18n/*.json` are data, not code: [`catalogues::Catalogues`] reads files in that format,
//! and which ones the Linux client ships is not decided here.

pub mod calendar;
pub mod catalogues;
pub mod greeting_places;
pub mod house_rules;
pub mod link_preview;
pub mod location_fixes;
pub mod media_text;
pub mod mentions;
pub mod message_body;
pub mod plurals;
pub mod polls;
pub mod sources_footer;
pub mod sticker_file;
pub mod string_catalog;
