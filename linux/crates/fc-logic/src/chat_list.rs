//! The chat list: the rows, in the order something last happened in them, each with the one line
//! under its name (`ChatList.cs`).
//!
//! THE ORDER AND THE COUNTS ARE THE SERVER'S. The list read carries the authoritative unread counts
//! and the caller's own read marker, and this model reads them back out of the cache rather than
//! counting anything itself — two devices that each counted would eventually disagree about the same
//! family.
//!
//! A BLOCKED MEMBER'S MESSAGE IS THE HIDDEN ROW HERE TOO, with no name and no reveal. In the family
//! chat their message still arrives, still counts and may still BE the preview, because the count is
//! the other half of the read marker and projecting one without the other desynchronises them — so
//! the row shows that something was said and not what (docs/protocol.md, "Blocking a member").
//!
//! A CALL READS AS A CALL and never as the English placeholder the server writes into a record's
//! body for clients that predate calls; a caption-less attachment reads as what it is, because a
//! preview with nothing in it is a row that looks like nothing happened. (Both wordings are
//! `fc_text`'s — `call_record::label`, `media::display_name` — in the reader's language as the shell
//! set it with `fc_text::i18n::use_lang`.)

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use fc_core::protocol::{ChatDto, ChatRowDto, MessageDto};
use fc_core::store::{ChatStore, StoreResult, times};
use fc_core::text::string_catalog::StringCatalog;
use time::{Duration, OffsetDateTime, UtcOffset};

use crate::date_culture::{self, TimeZone};
use crate::families::family_rules;
use crate::groups::{ChatFace, chat_kinds};

/// WHICH time a row shows, decided here; HOW it reads is the window's (`doorway::row_time_text`),
/// because a date is drawn in the reader's own zone, calendar and language and none of those
/// belong in a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowTimeKind {
    /// Nothing to show: a chat with no messages.
    None,
    /// Today: the clock.
    Clock,
    /// Yesterday: the word.
    Yesterday,
    /// Within the last week: the weekday.
    Weekday,
    /// Older: the date.
    Date,
}

/// One row of the chat list, as the window draws it and with nothing left to decide.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRow {
    pub chat: ChatDto,
    pub title: String,
    pub preview: String,
    pub unread: i32,
    pub mentioned: bool,
    /// The newest message is from somebody this reader has blocked, so the line is the hidden row
    /// and there is nothing to reveal — a list is not where a person peeks.
    pub hidden: bool,
    pub when: RowTimeKind,
    pub at: Option<OffsetDateTime>,
}

impl ChatRow {
    pub fn is_family(&self) -> bool {
        self.chat.kind == chat_kinds::FAMILY
    }

    pub fn is_direct(&self) -> bool {
        self.chat.kind == chat_kinds::DIRECT
    }

    pub fn is_assistant(&self) -> bool {
        self.chat.kind == chat_kinds::ASSISTANT
    }

    /// A group (docs/protocol.md, "Groups"): listed below the family chat, by its last message like
    /// any chat.
    pub fn is_group(&self) -> bool {
        chat_kinds::is_group(Some(&self.chat.kind))
    }

    /// What its circle shows.
    pub fn face(&self) -> ChatFace {
        chat_faces::for_kind(Some(&self.chat.kind))
    }
}

/// Which circle a chat is drawn with, by its kind — the list's rows and the conversation's header
/// alike (`ChatFaces`).
pub mod chat_faces {
    use super::*;

    /// `ChatFaces.For(kind)`.
    pub fn for_kind(kind: Option<&str>) -> ChatFace {
        match kind {
            Some(chat_kinds::FAMILY) => ChatFace::House,
            Some(chat_kinds::GROUP) => ChatFace::People,
            _ => ChatFace::Initials,
        }
    }
}

/// The chat list (`ChatListModel`). See the module's comment.
pub struct ChatListModel {
    chats: Rc<ChatStore>,
    me: Box<dyn Fn() -> i64>,
    say: Rc<dyn StringCatalog>,
    zone: Rc<dyn TimeZone>,
    held: RefCell<Vec<ChatRowDto>>,
}

impl std::fmt::Debug for ChatListModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatListModel")
            .field("held", &self.held.borrow().len())
            .finish_non_exhaustive()
    }
}

impl ChatListModel {
    /// `new ChatListModel(chats, me, words)`: days compared in UTC until
    /// [`with_zone`](Self::with_zone) gives the reader's zone (the shell's `glib::TimeZone::local()`;
    /// the C# read the machine's).
    pub fn new(
        chats: Rc<ChatStore>,
        me: impl Fn() -> i64 + 'static,
        say: Rc<dyn StringCatalog>,
    ) -> Self {
        ChatListModel {
            chats,
            me: Box::new(me),
            say,
            zone: Rc::new(UtcOffset::UTC),
            held: RefCell::new(Vec::new()),
        }
    }

    /// The reader's zone, which "today" and "yesterday" are days of.
    pub fn with_zone(mut self, zone: Rc<dyn TimeZone>) -> Self {
        self.zone = zone;
        self
    }

    /// Every chat on the list as the last [`rows`](Self::rows) read it — of EVERY family, before the
    /// shown one was picked out — so the family switcher can count each family's unread without
    /// reading the list a second time (`Held`).
    pub fn held(&self) -> Vec<ChatRowDto> {
        self.held.borrow().clone()
    }

    /// The rows, as of `now` — which only the times depend on.
    ///
    /// THE FAMILY CHAT IS FIRST, always, and everything else — groups included (docs/protocol.md,
    /// "Groups") — follows it newest conversation first. It is the room everybody is in and the one
    /// a reader looks for — Apple pins it, the web pins it, and a list that let it slide under a busy
    /// direct chat would be a different app here. The rest go by the id of their newest message
    /// rather than by its time: ids are monotonic, where a stamp can be moved by a server's clock and
    /// is deliberately NOT moved by an edit.
    pub fn rows(&self, now: OffsetDateTime) -> StoreResult<Vec<ChatRow>> {
        let mut rows = Vec::new();
        let mut newest_id: HashMap<i64, i64> = HashMap::new();
        // The family shown: its chats and no other family's (docs/protocol.md, "Several families")
        // — so "the family chat" pinned first below is the shown family's, and so is the
        // assistant's. A chat with no family_ids is the home family's.
        let shown = self.chats.family();
        let home = self.chats.home();
        let held = self.chats.chats()?;
        *self.held.borrow_mut() = held.clone();
        for row in held
            .iter()
            .filter(|row| family_rules::lists(&row.chat, shown, home))
        {
            let newest = self.chats.newest(row.chat.id)?;
            newest_id.insert(row.chat.id, newest.as_ref().map_or(0, |message| message.id));
            let hidden = match &newest {
                Some(message) => self.is_hidden(message)?,
                None => false,
            };
            let at = newest
                .as_ref()
                .and_then(|message| instant(&message.created_at));
            rows.push(ChatRow {
                chat: row.chat.clone(),
                title: self.title(&row.chat)?,
                preview: match &newest {
                    None => self.say.get("No messages yet").to_string(),
                    Some(message) => self.preview(message, hidden),
                },
                unread: row.unread_count,
                mentioned: row.mentioned == Some(true),
                hidden,
                when: at.map_or(RowTimeKind::None, |at| {
                    ChatListModel::when(at, now, self.zone.as_ref())
                }),
                at,
            });
        }
        rows.sort_by(|a, b| {
            b.is_family()
                .cmp(&a.is_family())
                .then(newest_id[&b.chat.id].cmp(&newest_id[&a.chat.id]))
                .then(a.chat.id.cmp(&b.chat.id))
        });
        Ok(rows)
    }

    /// The name on the row. The wire's title is the family's name, the peer's display name, the
    /// assistant's or the group's, recomputed by the server on every read — so it wins; the roster
    /// is the fallback for a direct chat whose title arrived empty, and "Chat" is the floor, because
    /// a row with no name at all is a row nobody can tap with intent.
    pub fn title(&self, chat: &ChatDto) -> StoreResult<String> {
        if !chat.title.is_empty() {
            return Ok(chat.title.clone());
        }
        if let Some(peer) = chat.peer_user_id
            && let Some(member) = self.chats.member(peer)?
            && !member.display_name.is_empty()
        {
            return Ok(member.display_name);
        }
        Ok(self.say.get("Chat").to_string())
    }

    /// The one line under the name.
    pub fn preview(&self, message: &MessageDto, hidden: bool) -> String {
        if hidden {
            return self.say.get("Hidden — blocked member").to_string();
        }
        if let Some(call) = &message.call {
            return fc_text::call_record::label(
                &call.outcome,
                call.duration_secs.map(i64::from),
                call.video,
                message.sender_id == (self.me)(),
            );
        }
        if !message.body.is_empty() {
            // The FIRST LINE only: a row is one line high, and a body with a newline in it would
            // otherwise push every row below it down the list.
            return message
                .body
                .split(['\n', '\r'])
                .next()
                .unwrap_or_default()
                .to_string();
        }
        let media = message.media();
        let Some(first) = media.first() else {
            return String::new();
        };
        if message.sticker_picture().is_some() {
            // The word, in the reader's language, where a photo's row says so of a photo.
            return self.say.get("Sticker").to_string();
        }
        if message.round_video().is_some() {
            // Asked BEFORE "Video" (S5.7): a circle is a video on the wire and a video message to
            // the reader.
            return self.say.get("Video message").to_string();
        }
        if message.animation_video().is_some() {
            // An animation says "GIF" where a video says "Video" (docs/protocol.md, "Animations") —
            // and its caption first, as any video's row does.
            return self.say.get("GIF").to_string();
        }
        match first.kind.as_str() {
            "photo" if media.len() > 1 => self.say.format(
                "%lld Photos",
                &[&i64::try_from(media.len()).unwrap_or(i64::MAX)],
            ),
            "photo" => self.say.get("Photo").to_string(),
            "video" => self.say.get("Video").to_string(),
            // The preview says "Voice message" where an attachment's own name would say "Audio":
            // this is the row for a message that IS the recording.
            "audio" => self.say.get("Voice message").to_string(),
            "location" => self.say.get("Location").to_string(),
            // A file — and a kind this build never heard of still says something happened.
            kind => fc_text::media::display_name(kind, first.name.as_deref()).into_owned(),
        }
    }

    /// Whether this message is drawn hidden: somebody else's, and blocked.
    pub fn is_hidden(&self, message: &MessageDto) -> StoreResult<bool> {
        Ok(message.sender_id != (self.me)() && self.chats.is_blocked(message.sender_id)?)
    }

    /// Which time a row shows. Days are compared in the READER's zone, not in UTC: a message sent at
    /// 23:30 last night is "Yesterday" to the person reading it, whatever the clock says in
    /// Greenwich.
    pub fn when(at: OffsetDateTime, now: OffsetDateTime, zone: &dyn TimeZone) -> RowTimeKind {
        let day = date_culture::local(at, zone).date();
        let today = date_culture::local(now, zone).date();
        if day == today {
            return RowTimeKind::Clock;
        }
        if today.previous_day() == Some(day) {
            return RowTimeKind::Yesterday;
        }
        let age = now - at;
        if age >= Duration::ZERO && age < Duration::days(7) {
            RowTimeKind::Weekday
        } else {
            RowTimeKind::Date
        }
    }
}

/// An RFC 3339 instant as the store reads one, or `None`.
fn instant(rfc3339: &str) -> Option<OffsetDateTime> {
    let milliseconds = times::instant(Some(rfc3339))?;
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000).ok()
}
