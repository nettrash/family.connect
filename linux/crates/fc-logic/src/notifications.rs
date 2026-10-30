//! When this client raises a notification of its own, and what it says (`Notifications.cs`).
//!
//! THE BODY IS NEVER THE MESSAGE. It is the one a server with `include_message_body = false` would
//! send — "a Mac is pushed who, never what": a desktop client cannot know whether the operator
//! wanted family text on a lock screen, so it never puts it there, whatever the server's setting.
//! Nothing on [`Toast`] can carry message text: its body is [`fc_text::notify::new_message`] or
//! [`fc_text::notify::new_note`], and its title names who and where.
//!
//! THE BLOCK REACHES ONE STEP FURTHER THAN THE SENDER. A blocked member's message raises nothing,
//! and neither does the ASSISTANT'S ANSWER TO ONE — that would light up a notification for a thread
//! its reader cannot read.
//!
//! A NOTE IS NEWS ONLY WHEN IT SAYS SOMETHING NEW. A note somebody dragged across the wall raises
//! no badge, and notifies nobody either — the badge and the notification answer the same question
//! and must not disagree ([`toast_arguments::is_news`](crate::toast_arguments::is_news)).
//!
//! NOTHING IS SAID ABOUT WHAT THE READER IS LOOKING AT — and only that (the Mac's
//! `DesktopNotificationRules`, #84): a message in the chat being READ (shown in the window in
//! front, at its newest) raises nothing, while a message in any other chat does, the app in front
//! or not; a note raises nothing while the board's window is in front. Nothing at all while the
//! reader has not asked for notifications. (Windows muted everything while its window was in
//! front, so a message in another chat went unannounced; Linux follows the Mac.)
//!
//! PLATFORM-FREE. The shell raises a `GNotification` from a [`Toast`]: `title` and `body` as given,
//! [`Toast::tag`] as the notification's id (`Application::send_notification(Some(tag), …)`, so a
//! second one about the same chat replaces the first and opening the chat can withdraw it), and
//! the default action and target from
//! [`toast_arguments::for_toast`](crate::toast_arguments::for_toast).
//!
//! The wording of the title and the body is `fc_text::notify`'s (the reader's language through
//! `fc_text::i18n`, which the shell sets with `use_lang`); "Someone" is the apps' catalogue's,
//! through the `say` this was built with — both set to the same language by the shell.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_core::protocol::{ChatDto, FamilyDto, MessageDto, NoteDto};
use fc_core::store::{ChatStore, StoreResult};
use fc_core::text::string_catalog::StringCatalog;

use crate::families::family_rules;
use crate::groups::chat_kinds;

/// One notification to raise: what it says, what it is FOR, and the tag that makes a second one
/// about the same thing replace the first rather than stack on it (`Toast`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Toast {
    pub tag: String,
    pub title: String,
    pub body: String,
    pub chat_id: Option<i64>,
    pub family_id: Option<i64>,
}

impl Toast {
    /// `new Toast(tag, title, body, chatId, familyId)`.
    pub fn new(tag: &str, title: &str, body: &str, chat_id: Option<i64>) -> Self {
        Toast {
            tag: tag.to_string(),
            title: title.to_string(),
            body: body.to_string(),
            chat_id,
            family_id: None,
        }
    }

    /// The same toast, for this family's wall.
    pub fn in_family(mut self, family_id: Option<i64>) -> Self {
        self.family_id = family_id;
        self
    }

    /// Whether this one opens the board rather than a chat.
    pub fn is_board(&self) -> bool {
        self.chat_id.is_none()
    }
}

/// When this client raises a notification of its own, and what it says (`NotificationRules`).
///
/// The C#'s settable properties are setters here, on `&self`: the shell holds the rules in an `Rc`
/// and updates them as the window and the session move.
pub struct NotificationRules {
    chats: Rc<ChatStore>,
    say: Rc<dyn StringCatalog>,
    wanted: Cell<bool>,
    reading: Cell<Option<i64>>,
    board_in_front: Cell<bool>,
    family_name: RefCell<Option<String>>,
    families: RefCell<Vec<FamilyDto>>,
    assistant_user_id: Cell<Option<i64>>,
}

impl std::fmt::Debug for NotificationRules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No names: the family's logging rule.
        f.debug_struct("NotificationRules")
            .field("wanted", &self.wanted.get())
            .field("reading", &self.reading.get())
            .field("board_in_front", &self.board_in_front.get())
            .field("families", &self.families.borrow().len())
            .finish_non_exhaustive()
    }
}

impl NotificationRules {
    /// `new NotificationRules(chats, words)`: not wanted, nothing being read, no family named,
    /// until told.
    pub fn new(chats: Rc<ChatStore>, say: Rc<dyn StringCatalog>) -> Self {
        NotificationRules {
            chats,
            say,
            wanted: Cell::new(false),
            reading: Cell::new(None),
            board_in_front: Cell::new(false),
            family_name: RefCell::new(None),
            families: RefCell::new(Vec::new()),
            assistant_user_id: Cell::new(None),
        }
    }

    /// Whether the reader has asked for these at all.
    pub fn wanted(&self) -> bool {
        self.wanted.get()
    }

    pub fn set_wanted(&self, wanted: bool) {
        self.wanted.set(wanted);
    }

    /// The chat the reader is reading now — shown in the window in front, at its newest (the Mac's
    /// `ChatPresence.isReading`) — whose messages need no notification.
    pub fn reading(&self) -> Option<i64> {
        self.reading.get()
    }

    pub fn set_reading(&self, chat_id: Option<i64>) {
        self.reading.set(chat_id);
    }

    /// Whether the board's window is the one in front: a note landing on the wall the reader is
    /// looking at is not news to tell them about.
    pub fn board_in_front(&self) -> bool {
        self.board_in_front.get()
    }

    pub fn set_board_in_front(&self, in_front: bool) {
        self.board_in_front.set(in_front);
    }

    /// The family's name, for the title of anything from the family chat or the wall — the SHOWN
    /// family's: what a title says when the account's list cannot name the family something is
    /// from.
    pub fn family_name(&self) -> Option<String> {
        self.family_name.borrow().clone()
    }

    pub fn set_family_name(&self, name: Option<&str>) {
        *self.family_name.borrow_mut() = name.map(str::to_string);
    }

    /// Every family the account is in (docs/protocol.md, "Several families"): a family chat's
    /// message, or a note, is titled with THAT family's name — the one its chat or note belongs
    /// to — not the shown one's.
    pub fn families(&self) -> Vec<FamilyDto> {
        self.families.borrow().clone()
    }

    pub fn set_families(&self, families: &[FamilyDto]) {
        *self.families.borrow_mut() = families.to_vec();
    }

    /// The assistant's user id, so its answers can be gated with the member they answer.
    pub fn assistant_user_id(&self) -> Option<i64> {
        self.assistant_user_id.get()
    }

    pub fn set_assistant_user_id(&self, user_id: Option<i64>) {
        self.assistant_user_id.set(user_id);
    }

    /// The toast for an arriving message, or `None` when there is nothing to say.
    pub fn for_message(&self, message: &MessageDto) -> StoreResult<Option<Toast>> {
        if !self.speaking() {
            return Ok(None);
        }
        let me = self.chats.reader();
        if message.sender_id == me || self.chats.is_blocked(message.sender_id)? {
            return Ok(None);
        }
        // The block, one step further: the assistant answering somebody this reader cannot see.
        if self.assistant_user_id.get() == Some(message.sender_id)
            && let Some(quote) = &message.reply_to
            && self.chats.is_blocked(quote.sender_id)?
        {
            return Ok(None);
        }
        // Being read: the reader sees it arrive.
        if self.reading.get() == Some(message.chat_id) {
            return Ok(None);
        }
        let Some(chat) = self.chats.chat(message.chat_id)? else {
            // A chat this device does not hold: the list read that names it has not landed, and a
            // notification that cannot be opened is worse than none.
            return Ok(None);
        };
        let mentioned = message
            .mentions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|mention| mention.user_id == me);
        let prefix = self.prefix(&chat.chat);
        let sender = self.name_of(message.sender_id)?;
        Ok(Some(Toast {
            tag: Self::chat_tag(message.chat_id),
            title: fc_text::notify::title(prefix.as_deref(), &sender, mentioned),
            body: fc_text::notify::new_message().to_string(),
            // No family: a click opens the chat in the family shown when it lists it, else the
            // first that does (`family_rules::family_to_open`) — a direct chat may be in several.
            chat_id: Some(message.chat_id),
            family_id: None,
        }))
    }

    /// The toast for a note that has just said something new, or `None`. `is_news` is the board's
    /// own verdict — the badge's — so the two cannot disagree.
    pub fn for_note(&self, note: &NoteDto, is_news: bool) -> StoreResult<Option<Toast>> {
        if !self.speaking() || !is_news {
            return Ok(None);
        }
        if note.author_id == self.chats.reader() || self.chats.is_blocked(note.author_id)? {
            return Ok(None);
        }
        if self.board_in_front.get() {
            return Ok(None);
        }
        let family = {
            let families = self.families.borrow();
            family_rules::name_of(note.family_id, &families)
                .map(str::to_string)
                .or_else(|| self.family_name())
        };
        let author = self.name_of(note.author_id)?;
        Ok(Some(Toast {
            tag: "board".to_string(),
            title: fc_text::notify::title(family.as_deref(), &author, false),
            body: fc_text::notify::new_note().to_string(),
            chat_id: None,
            family_id: note.family_id,
        }))
    }

    /// What the window's own title says: the count, then the name. A hundred and more is "99+" —
    /// the exact number stops being the point (`fc_text::notify::page_title`).
    pub fn window_title(&self, brand: &str) -> StoreResult<String> {
        Ok(fc_text::notify::page_title(brand, self.chats.unread()?))
    }

    /// The tag every notification about one chat carries, so a second replaces the first and
    /// opening the chat can take them all away. An identifier, not a sentence: Rust's integer
    /// formatting has no culture (the C# wrote it invariantly on purpose).
    pub fn chat_tag(chat_id: i64) -> String {
        format!("chat-{chat_id}")
    }

    /// What goes before the sender in a title (docs/protocol.md, "Push notifications"): the
    /// family's name in the family chat, the GROUP's name in a group — "<Group> — <Sender>" — and
    /// nothing in a direct chat, whose title is the sender alone. A group whose name arrived empty
    /// is titled by the sender alone too, rather than " — Anna".
    fn prefix(&self, chat: &ChatDto) -> Option<String> {
        match chat.kind.as_str() {
            chat_kinds::FAMILY => {
                let families = self.families.borrow();
                family_rules::name_of_chat(chat, &families)
                    .map(str::to_string)
                    .or_else(|| self.family_name())
            }
            chat_kinds::GROUP if !chat.title.is_empty() => Some(chat.title.clone()),
            _ => None,
        }
    }

    /// Whether anything is worth saying at all.
    fn speaking(&self) -> bool {
        self.wanted.get()
    }

    /// Whoever this is, as the roster names them — and "Someone" when it cannot, which happens and
    /// is nobody's fault: a member who joined while this device slept is a name it has not read
    /// yet.
    fn name_of(&self, user_id: i64) -> StoreResult<String> {
        Ok(match self.chats.member(user_id)? {
            Some(member) if !member.display_name.is_empty() => member.display_name,
            _ => self.say.get("Someone").to_string(),
        })
    }
}
