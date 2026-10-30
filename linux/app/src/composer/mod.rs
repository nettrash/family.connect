//! The composer (shell-design §6, P1): the approved row of `docs/composer-and-wide-layout` — the
//! paperclip (its menu `AttachMenu`'s), a capsule holding the field and its glyphs, and the Send
//! SLOT, one fixed 40-px accent disc in a 44-px target (`ComposerButton`: Send, Save, the
//! microphone, the recording's arrow or Stop) so the row never jumps. Stacked above it: notices,
//! the reply / edit banner, the staged strip (chips with ✕), P3's "not sent" rows; the `@`
//! suggestions float over the field.
//!
//! - **Keys** (§6.2, [`keys`]): Enter alone activates the slot only where it is Send or an enabled
//!   Save; Shift, Ctrl or Alt + Enter is a new line; Enter is the input method's while it
//!   composes; a held key is one press; the 600-ms guard (`SlotGuard`). Escape (bubble phase):
//!   suggestions, recording, edit, reply.
//! - **Paste** (§6.3): the field's Ctrl+V pastes words; it is taken over only when the clipboard
//!   holds no words but files, a texture or a picture type (`PASTE_PREFERENCE` order). Ctrl+V
//!   elsewhere is `win.paste-attachment`, enabled only while the field is not focused.
//! - **Drop** (§6.4): a `gtk::DropTarget` for file lists on the whole page, an accent outline while
//!   a drag hovers; folders refused (the drag springs back). Files are staged, never sent.
//! - **Staging and sending** (§6.5): `ComposerStaging` (one at a time, the cap, Cancel), the staged
//!   bytes written into the media store only at Send, `ConversationModel::send` (the outbox row
//!   first, then a flush); the assistant's consent asked first where `AssistantConsent` says so.
//! - **Typing aids** (§7): capitals and emoji as you type (`text::typing_hooks`); mentions from
//!   `ComposerMentions` in the family chat and groups; GTK's emoji chooser from the ☺ glyph and
//!   Ctrl+. / Ctrl+;. No spelling (D9).
//! - **Recording** is P3's, through `ComposerSlots` (this composer is the `ComposerHost`).

pub mod consent;
pub mod drafts;
pub mod integration;
pub mod keys;
pub mod notices;
// P3's file in P1's folder (W.2): the row that takes the field's place while a voice note records.
pub mod recording_row;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use fc_core::protocol::{ChatDto, PackItemDto, QueueOptions};
use fc_core::rt::{Clock, LocalBoxFuture};
use fc_core::text::location_fixes::LocationFix;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::assistant_consent;
use fc_logic::attach_menu::{self, AttachItem};
use fc_logic::composer_button::{self, Door, DoorInputs, Recording, Slot, SlotGuard, SlotInputs};
use fc_logic::composer_mentions::{self, MentionChoice};
use fc_logic::composer_row::{self, FieldGlyph};
use fc_logic::composer_staging::{self, ComposerStaging};
use fc_logic::conversation::ConversationModel;
use fc_logic::groups::chat_kinds;
use fc_logic::media_outbox::{StagedMedia, animation_send};
use fc_logic::not_sent::{self, RecordingEnd};
use fc_logic::pack::{pack_sending, pack_text};
use fc_logic::quotes;

use crate::seams::composer::{ComposerHost, ComposerRecording};
use crate::seams::media::{MediaSource, NowPlaying, Player, PlayerState};
use crate::services::catalogue::Catalogue;
use crate::services::log;
use crate::shell::{Connection, Shell};
use crate::text::{offsets, typing_hooks};
use crate::theme;

use keys::{EnterAction, EscapeAction};

/// Which composer: a conversation's own, or a thread's (no paperclip, no microphone; every send
/// answers the root).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerKind {
    Main,
    Thread { root_id: i64 },
}

/// The composer (see the module's comment).
pub struct Composer {
    shell: Rc<Shell>,
    connection: Rc<Connection>,
    model: Rc<ConversationModel>,
    chat: ChatDto,
    kind: Cell<ComposerKind>,
    window: Rc<dyn Fn() -> Option<gtk::Window>>,
    on_sent: Rc<dyn Fn()>,
    set_paste: Rc<dyn Fn(bool)>,
    root: gtk::Box,
    notice_row: gtk::Box,
    notice: gtk::Label,
    cancel_preparing: gtk::Button,
    banner: gtk::Box,
    banner_label: gtk::Label,
    banner_excerpt: gtk::Label,
    /// The assistant's notices over the row (`notices::of`) and what they last said.
    hints: gtk::Box,
    hints_drawn: RefCell<notices::Notices>,
    banner_close: gtk::Button,
    strip: gtk::FlowBox,
    not_sent: adw::Bin,
    paperclip: Option<gtk::MenuButton>,
    stack: gtk::Stack,
    recording_bin: adw::Bin,
    field: gtk::TextView,
    placeholder: gtk::Label,
    emoji_button: gtk::Button,
    sticker_button: gtk::Button,
    picture_button: gtk::Button,
    video_button: gtk::Button,
    /// When the video button last appeared (ms): a click within the activation guard of that is
    /// one aimed at what stood there before, and opens no camera (the Mac's `videoDoorShownAtMS`).
    video_shown_at: Cell<i64>,
    /// Whether a voice recording showed at the last redraw: the keyboard's focus follows the row
    /// (the Mac's `slotFocusRequest`, S2.4).
    was_recording: Cell<bool>,
    slot: gtk::Button,
    slot_icon: gtk::Image,
    slot_disc: gtk::Box,
    hooks: RefCell<Option<Rc<typing_hooks::TypingHooks>>>,
    staging: Rc<ComposerStaging>,
    guard: RefCell<SlotGuard>,
    press: Cell<i64>,
    reply: Cell<Option<i64>>,
    editing: RefCell<Option<(i64, String)>>,
    recording: RefCell<Option<Rc<dyn ComposerRecording>>>,
    mentions: gtk::Popover,
    mention_list: gtk::ListBox,
    choices: RefCell<Vec<MentionChoice>>,
    composing: Cell<bool>,
    enter_down: Cell<bool>,
    setting: Cell<bool>,
    sending: Cell<bool>,
    /// A sticker is being fetched and queued (one at a time, Windows' `sendingSticker`).
    sending_sticker: Cell<bool>,
    /// The place is being found ("Finding your location…").
    locating: Cell<bool>,
    /// The voice notes P3 put into the strip (row 3's Stop): a note in REVIEW, parked as "not sent"
    /// when the chat is left (`not_sent::parks_review`).
    reviewing: RefCell<Vec<Rc<StagedMedia>>>,
    /// A staged voice note playing in its chip: the item (by address), its player, its claim.
    strip_player: RefCell<Option<(usize, Rc<dyn Player>, u64)>>,
    /// The thumbnails of staged pictures and videos, by item address — made once, not per redraw.
    thumbnails: RefCell<std::collections::HashMap<usize, gtk::gdk::Texture>>,
    /// What the strip last drew (the items' addresses and the one playing): it is rebuilt only
    /// when that changes, not on every keystroke's redraw.
    strip_drawn: RefCell<(Vec<usize>, Option<usize>)>,
    /// Told after every redraw (the page re-asks its call buttons: nothing recording?).
    on_refresh: RefCell<Option<Rc<dyn Fn()>>>,
    /// Whether the window's Record Voice Message and Record Video Message can act now
    /// (`composer_button::records_voice` / `records_video`), as of the last redraw.
    record_commands: Cell<(bool, bool)>,
    closed: Cell<bool>,
    drop_outline: RefCell<Option<gtk::Box>>,
    me: Weak<Composer>,
}

impl std::fmt::Debug for Composer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Composer")
            .field("chat", &self.chat.id)
            .finish_non_exhaustive()
    }
}

fn now_ms() -> i64 {
    glib::monotonic_time() / 1000
}

/// A 44-px flat circular button with a symbolic icon, named and tooltipped.
fn glyph(icon: &str, name: &str) -> gtk::Button {
    let button = target(name);
    button.set_icon_name(icon);
    button
}

/// [`glyph`] drawing a paintable instead of a theme's icon — a symbolic one in the button's ink.
fn painted_glyph(paintable: &impl IsA<gtk::gdk::Paintable>, name: &str) -> gtk::Button {
    let button = target(name);
    button.set_child(Some(&gtk::Image::from_paintable(Some(paintable))));
    // The icon buttons' padding, which `set_icon_name` would have given it.
    button.add_css_class("image-button");
    button
}

/// The 44-px flat circular target of [`glyph`], named and tooltipped, still empty.
fn target(name: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .css_classes(["flat", "circular"])
        .tooltip_text(name)
        .valign(gtk::Align::End)
        .width_request(composer_row::TARGET as i32)
        .height_request(composer_row::TARGET as i32)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button
}

impl Composer {
    pub fn new(
        shell: &Rc<Shell>,
        connection: &Rc<Connection>,
        model: Rc<ConversationModel>,
        chat: ChatDto,
        kind: ComposerKind,
        window: Rc<dyn Fn() -> Option<gtk::Window>>,
        on_sent: Rc<dyn Fn()>,
        set_paste: Rc<dyn Fn(bool)>,
    ) -> Rc<Composer> {
        let say = &shell.say;
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.add_css_class("fc-chat-composer");

        // The assistant's notices, first over the row (`notices`): drawn on every redraw.
        let hints = gtk::Box::new(gtk::Orientation::Vertical, 2);
        hints.add_css_class("fc-chat-hints");
        hints.set_visible(false);
        root.append(&hints);

        let notice = gtk::Label::builder()
            .css_classes(["fc-chat-notice"])
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .build();
        let cancel_preparing = gtk::Button::builder()
            .label(say.get("Cancel"))
            .css_classes(["flat"])
            .visible(false)
            .build();
        let dismiss = glyph("window-close-symbolic", say.get("Dismiss"));
        let notice_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        notice_row.add_css_class("fc-chat-banner");
        notice_row.append(&notice);
        notice_row.append(&cancel_preparing);
        notice_row.append(&dismiss);
        notice_row.set_visible(false);
        root.append(&notice_row);

        let banner_label = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        // The reply's words under its title (the Mac's `MacComposerBanner` text).
        let banner_excerpt = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", "dim-label"])
            .visible(false)
            .build();
        let banner_words = gtk::Box::new(gtk::Orientation::Vertical, 0);
        banner_words.set_valign(gtk::Align::Center);
        banner_words.set_hexpand(true);
        banner_words.append(&banner_label);
        banner_words.append(&banner_excerpt);
        let banner_close = glyph("window-close-symbolic", say.get("Cancel reply"));
        let banner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        banner.add_css_class("fc-chat-banner");
        banner.add_css_class("fc-chat-reply-banner");
        banner.append(&gtk::Image::from_icon_name("mail-reply-sender-symbolic"));
        banner.append(&banner_words);
        banner.append(&banner_close);
        banner.set_visible(false);
        root.append(&banner);

        let strip = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(10)
            .column_spacing(6)
            .row_spacing(6)
            .visible(false)
            .build();
        root.append(&strip);
        let not_sent = adw::Bin::new();
        root.append(&not_sent);

        // One line stands as tall as the targets beside it (13 + an 18-px line + 13), centred on
        // them, and the field fills the capsule as it grows.
        let field = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .top_margin(13)
            .bottom_margin(13)
            .left_margin(8)
            .right_margin(4)
            .css_classes(["fc-chat-field"])
            .hexpand(true)
            .build();
        let placeholder_text = match kind {
            ComposerKind::Main => say.get("Message"),
            ComposerKind::Thread { .. } => say.get("Reply in thread"),
        };
        field.update_property(&[gtk::accessible::Property::Label(placeholder_text)]);
        let field_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(8 * 22 + 22)
            .child(&field)
            .hexpand(true)
            .valign(gtk::Align::Fill)
            .build();
        // At the field's own left margin, where the caret stands.
        let placeholder = gtk::Label::builder()
            .label(placeholder_text)
            .css_classes(["fc-chat-placeholder"])
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Center)
            .margin_start(8)
            .can_target(false)
            .build();
        let field_overlay = gtk::Overlay::new();
        field_overlay.set_child(Some(&field_scroller));
        field_overlay.add_overlay(&placeholder);
        let recording_bin = adw::Bin::new();
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(150)
            .hhomogeneous(false)
            .vhomogeneous(false)
            .hexpand(true)
            .build();
        stack.add_named(&field_overlay, Some("field"));
        stack.add_named(&recording_bin, Some("recording"));

        // GTK's own glyph for its emoji chooser (a face), and the app's own for the stickers (a
        // face on a peeled square): icon themes draw every face alike (Yaru's emoji and smiley are
        // one outline), so a second face could not tell the two apart.
        let emoji_button = glyph("emoji-people-symbolic", say.get("Emoji"));
        let sticker_button = painted_glyph(&theme::glyphs::sticker(), say.get("Stickers"));
        let picture_button = glyph(
            "applications-graphics-symbolic",
            say.get("Ask for a picture"),
        );
        // A camera in a ring (the Mac's `video.circle`): the header's bare camera is Video Call.
        let video_button = painted_glyph(
            &theme::glyphs::video_circle(),
            &composer_button::door_tooltip(say.as_ref()),
        );
        // In the accent, as the Mac draws its stickers and video glyphs beside the slot.
        sticker_button.add_css_class("fc-chat-tinted");
        video_button.add_css_class("fc-chat-tinted");
        let capsule = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        capsule.add_css_class("fc-chat-capsule");
        capsule.set_hexpand(true);
        capsule.append(&stack);
        capsule.append(&emoji_button);
        capsule.append(&sticker_button);
        capsule.append(&picture_button);
        capsule.append(&video_button);

        let slot_icon = gtk::Image::from_icon_name("mail-send-symbolic");
        let slot_disc = gtk::Box::builder()
            .css_classes(["fc-chat-slot-disc"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .width_request(composer_button::SLOT_VISUAL_PX as i32)
            .height_request(composer_button::SLOT_VISUAL_PX as i32)
            .build();
        // Expanding, so the box gives it the whole disc to centre in: a box packs a child at its
        // natural width from the start, and `halign` alone left the glyph off to one side.
        slot_icon.set_hexpand(true);
        slot_icon.set_halign(gtk::Align::Center);
        slot_icon.set_valign(gtk::Align::Center);
        slot_disc.append(&slot_icon);
        let slot = gtk::Button::builder()
            .child(&slot_disc)
            .css_classes(["fc-chat-slot"])
            .valign(gtk::Align::End)
            .hexpand(false)
            .width_request(composer_row::TARGET as i32)
            .height_request(composer_row::TARGET as i32)
            .build();

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let paperclip = match kind {
            ComposerKind::Main => {
                let button = gtk::MenuButton::builder()
                    .icon_name("mail-attachment-symbolic")
                    .css_classes(["flat", "circular"])
                    .valign(gtk::Align::End)
                    .tooltip_text(say.get("Attach a photo, video or file"))
                    .width_request(composer_row::TARGET as i32)
                    .height_request(composer_row::TARGET as i32)
                    .build();
                button.update_property(&[gtk::accessible::Property::Label(
                    say.get("Attach a photo, video or file"),
                )]);
                row.append(&button);
                Some(button)
            }
            ComposerKind::Thread { .. } => None,
        };
        row.append(&capsule);
        row.append(&slot);
        root.append(&row);

        let mention_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Browse)
            .css_classes(["navigation-sidebar"])
            .build();
        let mentions = gtk::Popover::builder()
            .child(&mention_list)
            .autohide(false)
            .has_arrow(false)
            .position(gtk::PositionType::Top)
            .build();
        // Parented to the composer's box, never to the text view: a `GtkTextView` takes every
        // child for one of its own and, disposing, loops for ever on one it does not know
        // ("GtkPopover is not a child of GtkTextView" — measured in the self-test).
        mentions.set_parent(&root);

        let composer = Rc::new_cyclic(|me: &Weak<Composer>| Composer {
            shell: Rc::clone(shell),
            connection: Rc::clone(connection),
            model,
            chat,
            kind: Cell::new(kind),
            window,
            on_sent,
            set_paste,
            root,
            notice_row,
            notice,
            cancel_preparing,
            banner,
            banner_label,
            banner_excerpt,
            hints,
            hints_drawn: RefCell::new(notices::Notices::default()),
            banner_close,
            strip,
            not_sent,
            paperclip,
            stack,
            recording_bin,
            field,
            placeholder,
            emoji_button,
            sticker_button,
            picture_button,
            video_button,
            video_shown_at: Cell::new(0),
            was_recording: Cell::new(false),
            slot,
            slot_icon,
            slot_disc,
            hooks: RefCell::new(None),
            staging: Rc::new(ComposerStaging::new()),
            guard: RefCell::new(SlotGuard::new()),
            press: Cell::new(0),
            reply: Cell::new(None),
            editing: RefCell::new(None),
            recording: RefCell::new(None),
            mentions,
            mention_list,
            choices: RefCell::new(Vec::new()),
            composing: Cell::new(false),
            enter_down: Cell::new(false),
            setting: Cell::new(false),
            sending: Cell::new(false),
            sending_sticker: Cell::new(false),
            locating: Cell::new(false),
            reviewing: RefCell::new(Vec::new()),
            strip_player: RefCell::new(None),
            thumbnails: RefCell::default(),
            strip_drawn: RefCell::default(),
            on_refresh: RefCell::new(None),
            record_commands: Cell::new((false, false)),
            closed: Cell::new(false),
            drop_outline: RefCell::new(None),
            me: me.clone(),
        });
        {
            let weak = Rc::downgrade(&composer);
            dismiss.connect_clicked(move |_| {
                if let Some(composer) = weak.upgrade() {
                    composer.notice_row.set_visible(false);
                }
            });
        }
        composer.wire();
        // The recording half is P3's.
        let host: Rc<dyn ComposerHost> = Rc::new(Host(Rc::downgrade(&composer)));
        let recording = shell.composer_slots.get().attach(host);
        composer.recording_bin.set_child(Some(&recording.row()));
        *composer.recording.borrow_mut() = Some(recording);
        if kind == ComposerKind::Main {
            let draft = drafts::shared().borrow().of(composer.chat.id).to_string();
            composer.set_text(&draft);
        }
        composer.refresh();
        composer
    }

    pub fn say(&self) -> Rc<Catalogue> {
        Rc::clone(&self.shell.say)
    }

    /// The composer's widget.
    pub fn widget(&self) -> gtk::Box {
        self.root.clone()
    }

    /// The text field (focus, the self-test's keys).
    pub fn field(&self) -> &gtk::TextView {
        &self.field
    }

    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn staging(&self) -> &Rc<ComposerStaging> {
        &self.staging
    }

    /// The words in the field.
    pub fn draft(&self) -> String {
        typing_hooks::buffer_text(&self.field.buffer())
    }

    /// The reply primed, if any.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn reply(&self) -> Option<i64> {
        self.reply.get()
    }

    /// The message being edited, if any.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn editing(&self) -> Option<i64> {
        self.editing.borrow().as_ref().map(|(id, _)| *id)
    }

    /// The notice line's words while it shows.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn notice_text(&self) -> Option<String> {
        self.notice_row
            .is_visible()
            .then(|| self.notice.label().to_string())
    }

    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn mentions_shown(&self) -> bool {
        self.mentions.is_visible()
    }

    pub fn focus(&self) {
        self.field.grab_focus();
    }

    /// The field's sticker glyph (the self-test presses it).
    #[cfg(feature = "selftest")]
    pub fn sticker_glyph(&self) -> &gtk::Button {
        &self.sticker_button
    }

    /// This composer as P3 sees it (`ComposerHost`) — the self-test hands it a video message.
    #[cfg(feature = "selftest")]
    pub fn host(&self) -> Rc<dyn ComposerHost> {
        Rc::new(Host(self.me.clone()))
    }

    /// A place found, sent as the paperclip's Location sends it (Xvfb has no Location portal).
    #[cfg(feature = "selftest")]
    pub fn send_place(&self, fix: &LocationFix) {
        self.send_location(fix);
    }

    /// The paperclip's Location, pressed (the self-test: no portal answers in Docker).
    #[cfg(feature = "selftest")]
    pub fn press_location(&self) {
        self.share_location();
    }

    /// `then` runs after every redraw of the composer — a recording starting or ending among them.
    /// Whether Record Voice Message and Record Video Message can act now (the Mac greys them out
    /// where they would do nothing; `MacVoiceCommands`).
    pub fn record_commands(&self) -> (bool, bool) {
        self.record_commands.get()
    }

    pub fn set_on_refresh(&self, then: Rc<dyn Fn()>) {
        *self.on_refresh.borrow_mut() = Some(then);
    }

    fn recording(&self) -> Rc<dyn ComposerRecording> {
        self.recording
            .borrow()
            .clone()
            .unwrap_or_else(|| Rc::new(crate::seams::composer::NoRecording))
    }

    /// Sets the field's words as the program (never judged by the typing aids, never the
    /// person's change to the guard).
    pub fn set_text(&self, text: &str) {
        self.setting.set(true);
        self.field.buffer().set_text(text);
        self.setting.set(false);
        self.place_end();
        self.text_changed();
    }

    fn place_end(&self) {
        let buffer = self.field.buffer();
        buffer.place_cursor(&buffer.end_iter());
    }

    /// Inserts `text` at the caret, replacing the selection, as one user action.
    pub fn insert(&self, text: &str) {
        let buffer = self.field.buffer();
        buffer.begin_user_action();
        buffer.delete_selection(true, true);
        buffer.insert_at_cursor(text);
        buffer.end_user_action();
    }

    /// Says `text` under the field (and to a screen reader).
    pub fn notice(&self, text: &str) {
        self.say_notice(text, true);
    }

    /// Says `text` under the field, and to a screen reader when `aloud` (never while a recording
    /// runs: the microphone would take the speech in).
    fn say_notice(&self, text: &str, aloud: bool) {
        self.notice.set_label(text);
        self.notice_row.set_visible(true);
        if aloud {
            self.notice
                .announce(text, gtk::AccessibleAnnouncementPriority::Medium);
        }
    }

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let buffer = self.field.buffer();
        *self.hooks.borrow_mut() = Some(typing_hooks::attach_text_view(&self.field, true));
        {
            let weak = weak.clone();
            buffer.connect_changed(move |_| {
                if let Some(composer) = weak.upgrade() {
                    composer.text_changed();
                }
            });
        }
        {
            let weak = weak.clone();
            buffer.connect_mark_set(move |buffer, _, mark| {
                if mark == &buffer.get_insert()
                    && let Some(composer) = weak.upgrade()
                {
                    composer.update_mentions();
                }
            });
        }
        {
            let weak = weak.clone();
            self.field.connect_preedit_changed(move |_, preedit| {
                if let Some(composer) = weak.upgrade() {
                    composer.composing.set(!preedit.is_empty());
                }
            });
        }
        // Keys, before the field acts on them.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let weak = weak.clone();
            keys.connect_key_pressed(move |_, key, _, state| match weak.upgrade() {
                Some(composer) => composer.key_pressed(key, state),
                None => glib::Propagation::Proceed,
            });
        }
        {
            let weak = weak.clone();
            keys.connect_key_released(move |_, key, _, _| {
                if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
                    && let Some(composer) = weak.upgrade()
                {
                    composer.enter_down.set(false);
                }
            });
        }
        self.field.add_controller(keys);
        // Escape in the bubble phase: it belongs to an input method or a popover first.
        let escape = gtk::EventControllerKey::new();
        {
            let weak = weak.clone();
            escape.connect_key_pressed(move |_, key, _, _| {
                if key != gtk::gdk::Key::Escape {
                    return glib::Propagation::Proceed;
                }
                match weak.upgrade() {
                    Some(composer) => composer.escape(),
                    None => glib::Propagation::Proceed,
                }
            });
        }
        self.root.add_controller(escape);
        // Paste: words are the field's; anything else is an attachment.
        {
            let weak = weak.clone();
            self.field.connect_paste_clipboard(move |field| {
                let Some(composer) = weak.upgrade() else {
                    return;
                };
                let formats = field.clipboard().formats();
                if !has_words(&formats) && has_attachment(&formats) {
                    field.stop_signal_emission_by_name("paste-clipboard");
                    composer.paste_attachment();
                }
            });
        }
        // `win.paste-attachment` only while the field is not focused (§6.3).
        let focus = gtk::EventControllerFocus::new();
        {
            let weak = weak.clone();
            focus.connect_enter(move |_| {
                if let Some(composer) = weak.upgrade() {
                    (composer.set_paste)(false);
                }
            });
        }
        {
            let weak = weak.clone();
            focus.connect_leave(move |_| {
                // A composer going away loses the focus too: it must not switch the action on for
                // whatever page comes next.
                if let Some(composer) = weak.upgrade().filter(|c| !c.closed.get()) {
                    (composer.set_paste)(true);
                    composer.mentions.popdown();
                }
            });
        }
        self.field.add_controller(focus);
        // The slot: its press for the guard, its click, its menu.
        let press = gtk::GestureClick::new();
        {
            let weak = weak.clone();
            press.connect_pressed(move |gesture, _, _, _| {
                if let Some(composer) = weak.upgrade() {
                    let device = slot_device(gesture.current_event_device());
                    let id = composer
                        .guard
                        .borrow_mut()
                        .pointer_down(now_ms(), device, false);
                    composer.press.set(id);
                }
            });
        }
        {
            let weak = weak.clone();
            press.connect_stopped(move |_| {
                if let Some(composer) = weak.upgrade() {
                    let id = composer.press.get();
                    composer.guard.borrow_mut().press_ended(id);
                }
            });
        }
        self.slot.add_controller(press);
        {
            let weak = weak.clone();
            self.slot.connect_clicked(move |_| {
                if let Some(composer) = weak.upgrade() {
                    composer.slot_clicked();
                }
            });
        }
        let secondary = gtk::GestureClick::builder().button(3).build();
        {
            let weak = weak.clone();
            secondary.connect_pressed(move |gesture, _, _, _| {
                if let Some(composer) = weak.upgrade() {
                    let device = slot_device(gesture.current_event_device());
                    let microphone = composer.slot_now().is_microphone();
                    if composer
                        .guard
                        .borrow_mut()
                        .secondary_click(device, microphone)
                        == composer_button::SlotResponse::Menu
                    {
                        composer.slot_menu();
                    }
                }
            });
        }
        self.slot.add_controller(secondary);
        // Shift+F10 and the Menu key on the focused slot: the microphone's menu.
        let slot_keys = gtk::EventControllerKey::new();
        {
            let weak = weak.clone();
            slot_keys.connect_key_pressed(move |_, key, _, state| {
                let context = key == gtk::gdk::Key::Menu
                    || (key == gtk::gdk::Key::F10
                        && state.contains(gtk::gdk::ModifierType::SHIFT_MASK));
                let Some(composer) = weak.upgrade().filter(|_| context) else {
                    return glib::Propagation::Proceed;
                };
                let microphone = composer.slot_now().is_microphone();
                if composer_button::respond(
                    composer_button::SlotPress::ContextKey,
                    composer_button::SlotDevice::None,
                    false,
                    microphone,
                ) == composer_button::SlotResponse::Menu
                {
                    composer.slot_menu();
                }
                glib::Propagation::Stop
            });
        }
        self.slot.add_controller(slot_keys);
        {
            let weak = weak.clone();
            self.banner_close.connect_clicked(move |_| {
                if let Some(composer) = weak.upgrade() {
                    if composer.editing.borrow().is_some() {
                        composer.cancel_edit();
                    } else {
                        composer.set_reply(None);
                    }
                }
            });
        }
        {
            let weak = weak.clone();
            self.cancel_preparing.connect_clicked(move |_| {
                if let Some(composer) = weak.upgrade() {
                    composer.staging.cancel_preparing();
                }
            });
        }
        {
            let weak = weak.clone();
            self.emoji_button.connect_clicked(move |button| {
                let Some(composer) = weak.upgrade() else {
                    return;
                };
                let chooser = gtk::EmojiChooser::new();
                chooser.set_parent(button);
                let weak = Rc::downgrade(&composer);
                chooser.connect_emoji_picked(move |_, emoji| {
                    if let Some(composer) = weak.upgrade() {
                        composer.insert(emoji);
                        composer.focus();
                    }
                });
                chooser.connect_closed(|chooser| {
                    let chooser = chooser.clone();
                    glib::idle_add_local_once(move || chooser.unparent());
                });
                chooser.popup();
            });
        }
        {
            let weak = weak.clone();
            self.sticker_button.connect_clicked(move |button| {
                let weak = weak.clone();
                integration::open_sticker_panel(button.upcast_ref(), move |item| {
                    if let Some(composer) = weak.upgrade() {
                        composer.send_sticker(item);
                    }
                });
            });
        }
        {
            let weak = weak.clone();
            self.picture_button.connect_clicked(move |_| {
                if let Some(composer) = weak.upgrade() {
                    let draft = composer.draft();
                    if !draft.starts_with(fc_text::assistant::DRAW_TOKEN) {
                        composer.set_text(&format!("{} {draft}", fc_text::assistant::DRAW_TOKEN));
                    }
                    composer.focus();
                }
            });
        }
        {
            let weak = weak.clone();
            self.video_button.connect_clicked(move |_| {
                if let Some(composer) = weak.upgrade() {
                    let guard = i64::try_from(composer_button::ACTIVATION_GUARD_MS).unwrap_or(600);
                    if now_ms() < composer.video_shown_at.get() + guard {
                        return;
                    }
                    composer.record_video();
                }
            });
        }
        {
            let weak = weak.clone();
            self.mention_list.connect_row_activated(move |_, row| {
                if let Some(composer) = weak.upgrade()
                    && let Ok(index) = usize::try_from(row.index())
                {
                    composer.accept_mention(index);
                }
            });
        }
        if let Some(paperclip) = &self.paperclip {
            let weak = weak.clone();
            paperclip.set_create_popup_func(move |button| {
                if let Some(composer) = weak.upgrade() {
                    button.set_menu_model(Some(&composer.attach_menu()));
                }
            });
            self.install_attach_actions();
        }
    }

    // ---- the slot ------------------------------------------------------------------------------

    fn inputs(&self) -> SlotInputs {
        let recording = self.recording();
        let main = self.kind.get() == ComposerKind::Main;
        let draft = self.draft();
        SlotInputs {
            recorder_open: recording.recorder_open(),
            recording: recording.recording(),
            editing: self.editing.borrow().is_some(),
            draft_blank: fc_text::composer::trimmed_for_send(&draft).is_none(),
            staged: !self.staging.is_empty(),
            assistant_chat: self.chat.kind == chat_kinds::ASSISTANT,
            can_record: main && recording.can_record(),
            call: self.connection.calls.call().is_some(),
            busy: self.staging.preparing(),
            not_sent: recording.not_sent(),
        }
    }

    /// What the slot is now.
    pub fn slot_now(&self) -> Slot {
        composer_button::composer_slot(&self.inputs())
    }

    /// Redraws the slot, the glyphs, the banner, the strip and the recording row.
    pub fn refresh(&self) {
        if self.closed.get() {
            return;
        }
        let say = &self.shell.say;
        let slot = self.slot_now();
        let face = composer_button::face(slot, self.sending.get(), say.as_ref());
        self.slot_icon.set_icon_name(Some(face.icon.icon_name()));
        self.slot.set_tooltip_text(Some(&face.tooltip));
        self.slot.update_property(&[
            gtk::accessible::Property::Label(&face.name),
            gtk::accessible::Property::Description(&face.help_text),
        ]);
        self.slot.set_sensitive(face.enabled);
        if face.looks_dimmed {
            self.slot_disc.add_css_class("fc-chat-dimmed");
        } else {
            self.slot_disc.remove_css_class("fc-chat-dimmed");
        }
        // The field, or the recording row in its place.
        let recording = self.recording();
        let records = recording.recording() != Recording::None;
        self.stack
            .set_visible_child_name(if records { "recording" } else { "field" });
        // The field goes away under a recording, and the focus with it: to the slot (Stop or the
        // Send arrow) while it runs, and back to the field once it ends — where the focus was in
        // this composer, never taken from somewhere else.
        if self.was_recording.replace(records) != records {
            let window = self.root.root().and_downcast::<gtk::Window>();
            let focus = window.as_ref().and_then(gtk::prelude::GtkWindowExt::focus);
            let here = focus.as_ref().is_none_or(|focus| {
                focus.is_ancestor(&self.root) || focus == self.root.upcast_ref::<gtk::Widget>()
            });
            if here {
                if records {
                    self.slot.grab_focus();
                } else {
                    self.field.grab_focus();
                }
            }
        }
        self.not_sent.set_child(recording.not_sent_rows().as_ref());
        // The glyphs inside the field.
        let state = self.connection.session.state();
        let editing = self.editing.borrow().is_some();
        let inputs = self.inputs();
        let door = composer_button::video_door(&DoorInputs {
            slot: inputs,
            family_or_direct_chat: self.kind.get() == ComposerKind::Main
                && chat_kinds::takes_video_messages(Some(&self.chat.kind)),
            server_offers_round: state.round_video.is_some(),
            has_camera: self.shell.camera.get().present(),
            encoder_probe_passes: true,
            records_round_video: recording.video_offered(),
        });
        let main = self.kind.get() == ComposerKind::Main;
        self.record_commands.set((
            main && composer_button::records_voice(&inputs),
            main && composer_button::records_video(&inputs, door != Door::Hidden),
        ));
        let glyphs = match self.kind.get() {
            ComposerKind::Main => composer_row::in_field(
                Some(&self.chat.kind),
                editing,
                integration::sticker_panel_offered(),
                state.assistant.as_ref(),
                door != Door::Hidden,
            ),
            ComposerKind::Thread { .. } => {
                composer_row::in_thread_field(true, integration::sticker_panel_offered())
            }
        };
        self.sticker_button
            .set_visible(glyphs.contains(&FieldGlyph::Sticker));
        self.picture_button
            .set_visible(glyphs.contains(&FieldGlyph::Picture));
        let door_shown = glyphs.contains(&FieldGlyph::VideoDoor);
        if door_shown && !self.video_button.is_visible() {
            self.video_shown_at.set(now_ms());
        }
        self.video_button.set_visible(door_shown);
        if let Some(label) = composer_button::door_label(door, say.as_ref()) {
            self.video_button
                .update_property(&[gtk::accessible::Property::Label(&label)]);
        }
        // The banner.
        if let Some((id, _)) = self.editing.borrow().as_ref() {
            let _ = id;
            self.banner_label.set_label(say.get("Editing message"));
            self.banner_excerpt.set_visible(false);
            self.banner_close
                .set_tooltip_text(Some(say.get("Cancel editing")));
            self.banner_close
                .update_property(&[gtk::accessible::Property::Label(say.get("Cancel editing"))]);
            self.banner.set_visible(true);
        } else if let Some(reply) = self.reply.get() {
            let held = self.connection.chats.message(reply).ok().flatten();
            let text = held
                .as_ref()
                .and_then(|message| {
                    quotes::banner(message, &self.connection.chats, say.as_ref()).ok()
                })
                .unwrap_or_default();
            self.banner_label.set_label(&text);
            let excerpt = held
                .as_ref()
                .and_then(|message| {
                    quotes::banner_excerpt(message, &self.connection.chats, say.as_ref())
                        .ok()
                        .flatten()
                })
                .unwrap_or_default();
            self.banner_excerpt.set_label(&excerpt);
            self.banner_excerpt.set_visible(!excerpt.is_empty());
            self.banner_close
                .set_tooltip_text(Some(say.get("Cancel reply")));
            self.banner_close
                .update_property(&[gtk::accessible::Property::Label(say.get("Cancel reply"))]);
            self.banner.set_visible(true);
        } else {
            self.banner.set_visible(false);
        }
        self.draw_strip();
        self.draw_hints();
        self.cancel_preparing.set_visible(self.staging.preparing());
        if let Some(paperclip) = &self.paperclip {
            paperclip.set_sensitive(!records);
        }
        let then = self.on_refresh.borrow().clone();
        if let Some(then) = then {
            then();
        }
    }

    /// The assistant's notices over the row (`notices::of`), redrawn only when what they say
    /// changes — a redraw on every keystroke would move the focus out of a "Review…".
    fn draw_hints(&self) {
        let state = self.connection.session.state();
        let draft = self.draft();
        let staged = self.staging.items();
        let quoted = self
            .reply
            .get()
            .and_then(|reply| self.connection.chats.message(reply).ok().flatten());
        let said = notices::of(
            &notices::Draft {
                chat_kind: &self.chat.kind,
                words: &draft,
                editing: self.editing.borrow().is_some(),
                staged: &staged,
                quoted: quoted.as_ref(),
            },
            &state,
            self.shell.say.as_ref(),
        );
        if *self.hints_drawn.borrow() == said {
            return;
        }
        while let Some(child) = self.hints.first_child() {
            self.hints.remove(&child);
        }
        let line = |icon: &str, text: &str| {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            line.add_css_class("fc-chat-hint");
            line.append(
                &gtk::Image::builder()
                    .icon_name(icon)
                    .valign(gtk::Align::Start)
                    .accessible_role(gtk::AccessibleRole::Presentation)
                    .build(),
            );
            line.append(
                &gtk::Label::builder()
                    .label(text)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::WordChar)
                    .xalign(0.0)
                    .hexpand(true)
                    .css_classes(["caption", "dim-label"])
                    .build(),
            );
            line
        };
        match &said.consent {
            Some(notices::Consent::Ask(text)) => {
                let ask = line("security-medium-symbolic", text);
                let review = gtk::Button::builder()
                    .label(self.shell.say.get("Review…"))
                    .css_classes(["flat", "caption"])
                    .valign(gtk::Align::Center)
                    .build();
                let weak = self.me.clone();
                // Agreed: what was written goes, as the Send it stood in front of would have sent it.
                review.connect_clicked(move |_| {
                    if let Some(composer) = weak.upgrade() {
                        composer.ask_consent(Rc::new(|composer: &Composer| composer.send()));
                    }
                });
                ask.append(&review);
                self.hints.append(&ask);
            }
            Some(notices::Consent::Unnamed(text)) => {
                self.hints.append(&line("security-medium-symbolic", text));
            }
            None => {}
        }
        if let Some(text) = &said.picture {
            self.hints.append(&line("view-reveal-symbolic", text));
        }
        if let Some(text) = &said.hint {
            self.hints
                .append(&line("applications-graphics-symbolic", text));
        }
        self.hints.set_visible(self.hints.first_child().is_some());
        *self.hints_drawn.borrow_mut() = said;
    }

    /// The slot's activation as Enter or a click would make it (the guard aside).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn press_slot(&self) {
        self.activate_slot(self.slot_now());
    }

    fn slot_clicked(&self) {
        let slot = self.slot_now();
        let response = self.guard.borrow_mut().click(now_ms(), slot, &self.draft());
        match response {
            composer_button::SlotResponse::Activate => self.activate_slot(slot),
            composer_button::SlotResponse::Menu => self.slot_menu(),
            composer_button::SlotResponse::Ignore => {}
        }
    }

    /// The slot's own activation (S1.3's last column).
    fn activate_slot(&self, slot: Slot) {
        let recording = self.recording();
        match slot {
            Slot::Send => self.send(),
            Slot::Save { enabled: true } => self.save_edit(),
            Slot::SendVoice => recording.send(),
            Slot::StopRecording => recording.stop(),
            Slot::Microphone => recording.start_voice(false),
            Slot::Dimmed(reason) => {
                if let Some(text) = composer_button::notice(Some(reason), self.shell.say.as_ref()) {
                    self.notice(&text);
                }
                return;
            }
            Slot::Recorder | Slot::SendDisabled | Slot::Save { enabled: false } => return,
        }
        self.guard.borrow_mut().arm(now_ms(), &self.draft());
        self.refresh();
    }

    /// The microphone's menu: Record Voice Message (and Record Video Message where offered).
    fn slot_menu(&self) {
        let say = &self.shell.say;
        let menu = gio::Menu::new();
        menu.append(
            Some(say.get("Record Voice Message")),
            Some("composer.record-voice"),
        );
        if self.recording().video_offered() {
            menu.append(
                Some(say.get("Record Video Message")),
                Some("composer.record-video"),
            );
        }
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&self.slot);
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
    }

    // ---- keys ----------------------------------------------------------------------------------

    fn key_pressed(&self, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> glib::Propagation {
        // The suggestions take the arrows, Enter and Tab while they show.
        if self.mentions.is_visible() {
            match key {
                gtk::gdk::Key::Down | gtk::gdk::Key::Up => {
                    let count = i32::try_from(self.choices.borrow().len()).unwrap_or(0);
                    let at = self
                        .mention_list
                        .selected_row()
                        .map_or(-1, |row| row.index());
                    let next = if key == gtk::gdk::Key::Down {
                        (at + 1).min(count - 1)
                    } else {
                        (at - 1).max(0)
                    };
                    if let Some(row) = self.mention_list.row_at_index(next) {
                        self.mention_list.select_row(Some(&row));
                    }
                    return glib::Propagation::Stop;
                }
                gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::Tab => {
                    let at = self
                        .mention_list
                        .selected_row()
                        .map_or(0, |row| row.index());
                    if let Ok(index) = usize::try_from(at) {
                        self.accept_mention(index);
                    }
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }
        if !matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) {
            return glib::Propagation::Proceed;
        }
        let repeat = self.enter_down.replace(true);
        let slot = self.slot_now();
        let action = keys::enter(
            self.composing.get(),
            state.contains(gtk::gdk::ModifierType::SHIFT_MASK),
            state.contains(gtk::gdk::ModifierType::CONTROL_MASK),
            state.contains(gtk::gdk::ModifierType::ALT_MASK),
            slot,
        );
        match action {
            EnterAction::ImeOwns => glib::Propagation::Proceed,
            EnterAction::NewLine => {
                if !repeat {
                    self.insert("\n");
                }
                glib::Propagation::Stop
            }
            EnterAction::ActivateSlot => {
                let ignored =
                    self.guard
                        .borrow()
                        .ignores_field_key(now_ms(), repeat, true, &self.draft());
                if !ignored {
                    self.activate_slot(slot);
                }
                glib::Propagation::Stop
            }
            EnterAction::Nothing => glib::Propagation::Stop,
        }
    }

    fn escape(&self) -> glib::Propagation {
        let recording = self.recording().recording() != Recording::None;
        match keys::escape(
            self.mentions.is_visible(),
            recording,
            self.editing.borrow().is_some(),
            self.reply.get().is_some(),
        ) {
            EscapeAction::CloseMentions => {
                self.mentions.popdown();
                glib::Propagation::Stop
            }
            EscapeAction::StopRecording => {
                self.recording().stop();
                self.refresh();
                glib::Propagation::Stop
            }
            EscapeAction::CancelEdit => {
                self.cancel_edit();
                glib::Propagation::Stop
            }
            EscapeAction::DropReply => {
                self.set_reply(None);
                glib::Propagation::Stop
            }
            EscapeAction::Nothing => glib::Propagation::Proceed,
        }
    }

    // ---- words ---------------------------------------------------------------------------------

    fn text_changed(&self) {
        let draft = self.draft();
        self.placeholder.set_visible(draft.is_empty());
        if let Some(cut) = fc_text::composer::clamping(&draft) {
            let cut = cut.to_string();
            self.set_text(&cut);
            self.notice(&fc_text::composer::Notice::Clamped.said());
            return;
        }
        let recording = self.recording().recording() != Recording::None;
        if !self.setting.get() {
            self.guard.borrow_mut().text_changed(&draft, recording);
            if !draft.trim().is_empty() {
                let model = Rc::clone(&self.model);
                glib::spawn_future_local(async move {
                    let _ = model.typing().await;
                });
            }
        }
        self.update_mentions();
        self.refresh();
    }

    /// The text before the caret — what the `@` strip reads.
    fn before_caret(&self) -> String {
        let buffer = self.field.buffer();
        let caret = buffer.iter_at_mark(&buffer.get_insert());
        buffer.text(&buffer.start_iter(), &caret, false).to_string()
    }

    fn update_mentions(&self) {
        let enabled = composer_mentions::enabled(Some(&self.chat.kind));
        let editing = self.editing.borrow().is_some();
        if !enabled || editing || self.composing.get() || !self.field.has_focus() {
            self.mentions.popdown();
            return;
        }
        let chats = &self.connection.chats;
        let roster = chats.members().unwrap_or_else(|_| Rc::from(Vec::new()));
        let chat = chats
            .chat(self.chat.id)
            .ok()
            .flatten()
            .map_or_else(|| self.chat.clone(), |row| row.chat);
        let candidates = composer_mentions::candidates(Some(&chat), &roster);
        let blocked = chats.blocked().unwrap_or_else(|_| Rc::from(Vec::new()));
        let is_blocked = |id: i64| blocked.contains(&id);
        let assistant = self.connection.session.state().assistant;
        let choices = composer_mentions::choices(
            &self.before_caret(),
            &candidates,
            chats.reader(),
            &is_blocked,
            enabled,
            editing,
            assistant.as_ref(),
        );
        if choices.is_empty() {
            self.mentions.popdown();
            self.choices.borrow_mut().clear();
            return;
        }
        while let Some(child) = self.mention_list.first_child() {
            self.mention_list.remove(&child);
        }
        for choice in &choices {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.append(&crate::chats::avatar::avatar(
                &choice.name,
                fc_logic::groups::ChatFace::Initials,
                choice.member.as_ref().map(|m| m.id),
                24,
            ));
            line.append(&gtk::Label::new(Some(&choice.name)));
            line.append(
                &gtk::Label::builder()
                    .label(&choice.handle)
                    .css_classes(["dim-label"])
                    .build(),
            );
            self.mention_list.append(&line);
        }
        if let Some(first) = self.mention_list.row_at_index(0) {
            self.mention_list.select_row(Some(&first));
        }
        *self.choices.borrow_mut() = choices;
        // Anchored to the caret.
        let buffer = self.field.buffer();
        let caret = buffer.iter_at_mark(&buffer.get_insert());
        let rect = self.field.iter_location(&caret);
        let (x, y) =
            self.field
                .buffer_to_window_coords(gtk::TextWindowType::Widget, rect.x(), rect.y());
        let at = self
            .field
            .compute_point(&self.root, &gtk::graphene::Point::new(x as f32, y as f32))
            .map_or((0, 0), |p| (p.x() as i32, p.y() as i32));
        self.mentions
            .set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                at.0,
                at.1,
                1,
                rect.height().max(1),
            )));
        self.mentions.popup();
    }

    /// The suggestion `index` taken: the `@query` before the caret becomes `@Name `.
    fn accept_mention(&self, index: usize) {
        let Some(choice) = self.choices.borrow().get(index).cloned() else {
            return;
        };
        let buffer = self.field.buffer();
        let before = self.before_caret();
        let whole = self.draft();
        let after = whole[before.len()..].to_string();
        let _ = after;
        let accepted = fc_text::mentions::accept(&before, &choice.writes);
        // Only the text before the caret changes: replace it, as one user action.
        let caret = offsets::char_len(&accepted);
        buffer.begin_user_action();
        let mut start = buffer.start_iter();
        let mut end = buffer.iter_at_mark(&buffer.get_insert());
        buffer.delete(&mut start, &mut end);
        let mut at = buffer.start_iter();
        buffer.insert(&mut at, &accepted);
        if let Ok(caret) = i32::try_from(caret) {
            buffer.place_cursor(&buffer.iter_at_offset(caret));
        }
        buffer.end_user_action();
        self.mentions.popdown();
        self.choices.borrow_mut().clear();
        self.guard.borrow_mut().changed(false);
    }

    // ---- reply, edit -----------------------------------------------------------------------------

    /// Primes (or drops) the reply.
    pub fn set_reply(&self, message_id: Option<i64>) {
        if self.editing.borrow().is_some() && message_id.is_some() {
            self.cancel_edit();
        }
        self.reply.set(message_id);
        self.refresh();
    }

    /// The composer borrows the field for an edit; Escape gives the draft back.
    pub fn start_edit(&self, message_id: i64, body: &str) {
        if self.editing.borrow().is_none() {
            let draft = self.draft();
            *self.editing.borrow_mut() = Some((message_id, draft));
        } else if let Some(editing) = self.editing.borrow_mut().as_mut() {
            editing.0 = message_id;
        }
        self.reply.set(None);
        self.set_text(body);
        self.focus();
        self.refresh();
    }

    fn cancel_edit(&self) {
        let saved = self.editing.borrow_mut().take();
        if let Some((_, draft)) = saved {
            self.set_text(&draft);
        }
        self.refresh();
    }

    fn save_edit(&self) {
        let Some((id, _)) = self.editing.borrow().clone() else {
            return;
        };
        let body = self.draft();
        let (model, weak) = (Rc::clone(&self.model), self.me.clone());
        glib::spawn_future_local(async move {
            let answer = model.edit(id, &body).await;
            let Some(composer) = weak.upgrade() else {
                return;
            };
            match answer {
                Ok(None) => {
                    crate::fc_info!("edit: message {id} saved");
                    composer.cancel_edit();
                    (composer.on_sent)();
                }
                Ok(Some(error)) => {
                    crate::fc_info!("edit: refused ({})", error.code);
                    composer.notice(composer.shell.say.get("Couldn't send that — try again."));
                }
                Err(error) => {
                    crate::fc_warn!("edit: the cache refused ({})", log::store_error(&error));
                }
            }
        });
    }

    /// The thread's chain was re-rooted by its read.
    pub fn set_root(&self, root_id: i64) {
        if let ComposerKind::Thread { .. } = self.kind.get() {
            self.kind.set(ComposerKind::Thread { root_id });
        }
    }

    // ---- sending -------------------------------------------------------------------------------

    /// Send what the composer holds: the words and the staged items, the reply primed — after the
    /// assistant's consent where it must be asked.
    pub fn send(&self) {
        let draft = self.draft();
        let body = fc_text::composer::trimmed_for_send(&draft)
            .unwrap_or("")
            .to_string();
        if body.is_empty() && self.staging.is_empty() {
            return;
        }
        let state = self.connection.session.state();
        let kind = self.chat.kind.as_str();
        let processor = state.assistant.as_ref().and_then(|a| a.processor.clone());
        if assistant_consent::is_withheld_from_an_unnamed_assistant(
            Some(kind),
            Some(&body),
            state.assistant.is_some(),
            processor.as_deref(),
        ) {
            self.notice(self.shell.say.get(
                "This server hasn't said which service answers, so nothing can be sent to the assistant here.",
            ));
            return;
        }
        if assistant_consent::is_required(
            Some(kind),
            Some(&body),
            processor.as_deref(),
            state.assistant_consent_at.as_deref(),
        ) {
            self.ask_consent(Rc::new(|composer: &Composer| composer.send()));
            return;
        }
        self.stop_strip_player();
        let staged = self.staging.take_all();
        let mut handles = Vec::new();
        for item in &staged {
            match self.connection.staging.stage(item) {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    crate::fc_warn!("staging: a file could not be written ({})", error.kind());
                    self.connection.staging.release(&handles);
                    self.staging.restore(staged);
                    self.notice(self.shell.say.get("Couldn't send that — try again."));
                    self.refresh();
                    return;
                }
            }
        }
        let roster = self
            .connection
            .chats
            .members()
            .unwrap_or_else(|_| Rc::from(Vec::new()));
        let chat = self
            .connection
            .chats
            .chat(self.chat.id)
            .ok()
            .flatten()
            .map_or_else(|| self.chat.clone(), |row| row.chat);
        let candidates = composer_mentions::candidates(Some(&chat), &roster);
        let mentions =
            composer_mentions::for_send(&body, &candidates, composer_mentions::enabled(Some(kind)))
                .unwrap_or_default();
        let reply = match self.kind.get() {
            ComposerKind::Main => self.reply.get(),
            ComposerKind::Thread { root_id } => Some(self.reply.get().unwrap_or(root_id)),
        };
        let values: Vec<StagedMedia> = staged.iter().map(|item| (**item).clone()).collect();
        let options = QueueOptions {
            reply_to_message_id: reply,
            pending_files: handles.clone(),
            mentions,
            animation: animation_send::is(&values),
            ..QueueOptions::default()
        };
        match self.model.send(&body, &options) {
            Ok(_) => {
                crate::fc_info!(
                    "send: queued in chat {} ({} file(s))",
                    self.chat.id,
                    handles.len()
                );
                // The row names them now: the sweep may judge them (`FolderMediaStore::stage`).
                self.connection.staging.release(&handles);
                self.reviewing.borrow_mut().clear();
                self.set_text("");
                self.reply.set(None);
                if self.kind.get() == ComposerKind::Main {
                    drafts::shared().borrow_mut().sent(self.chat.id);
                }
                self.guard.borrow_mut().arm(now_ms(), "");
                self.flush();
            }
            Err(error) => {
                crate::fc_warn!("send: the cache refused ({})", log::store_error(&error));
                self.connection.staging.release(&handles);
                self.staging.restore(staged);
                self.notice(self.shell.say.get("Couldn't send that — try again."));
            }
        }
        self.refresh();
    }

    /// A poll from the composer ("Poll" in the paperclip; the Mac's `sendPoll`). Its QUESTION is
    /// the body, so an `@ai` in it asks the assistant exactly as a typed message does — the consent
    /// question first where it must be asked — and it carries the primed reply and the question's
    /// mentions.
    pub fn send_poll(&self, question: String, options: Vec<String>) {
        let state = self.connection.session.state();
        let kind = self.chat.kind.as_str();
        let processor = state.assistant.as_ref().and_then(|a| a.processor.clone());
        if assistant_consent::is_withheld_from_an_unnamed_assistant(
            Some(kind),
            Some(&question),
            state.assistant.is_some(),
            processor.as_deref(),
        ) {
            self.notice(self.shell.say.get(
                "This server hasn't said which service answers, so nothing can be sent to the assistant here.",
            ));
            return;
        }
        if assistant_consent::is_required(
            Some(kind),
            Some(&question),
            processor.as_deref(),
            state.assistant_consent_at.as_deref(),
        ) {
            self.ask_consent(Rc::new(move |composer: &Composer| {
                composer.send_poll(question.clone(), options.clone());
            }));
            return;
        }
        let roster = self
            .connection
            .chats
            .members()
            .unwrap_or_else(|_| Rc::from(Vec::new()));
        let candidates = composer_mentions::candidates(Some(&self.chat), &roster);
        let mentions = composer_mentions::for_send(
            &question,
            &candidates,
            composer_mentions::enabled(Some(kind)),
        )
        .unwrap_or_default();
        let reply = match self.kind.get() {
            ComposerKind::Main => self.reply.get(),
            ComposerKind::Thread { root_id } => Some(self.reply.get().unwrap_or(root_id)),
        };
        let queue = QueueOptions {
            poll_options: options,
            reply_to_message_id: reply,
            mentions,
            ..QueueOptions::default()
        };
        match self.model.send(&question, &queue) {
            Ok(_) => {
                self.reply.set(None);
                self.flush();
                self.refresh();
            }
            Err(error) => {
                crate::fc_warn!("poll: the cache refused ({})", log::store_error(&error));
                self.notice(self.shell.say.get("Couldn't send that — try again."));
            }
        }
    }

    /// Send staged media now — a voice note's Send, or P3's video message (`round`: the outbox row's
    /// `round: true`) — the reply primed. A voice note that cannot be queued goes back into review
    /// rather than away: it cannot be recorded again (the Mac's send of a stopped note).
    fn send_media(&self, media: StagedMedia, round: bool) {
        let handle = match self.connection.staging.stage(&media) {
            Ok(handle) => handle,
            Err(error) => {
                crate::fc_warn!("staging: a file could not be written ({})", error.kind());
                self.notice(self.shell.say.get("Couldn't send that — try again."));
                if !round {
                    self.review(media);
                }
                return;
            }
        };
        let options = QueueOptions {
            reply_to_message_id: self.reply.get(),
            pending_files: vec![handle.clone()],
            round,
            animation: media.animation,
            ..QueueOptions::default()
        };
        match self.model.send("", &options) {
            Ok(_) => {
                self.connection.staging.release(&[handle]);
                self.reply.set(None);
                self.flush();
            }
            Err(error) => {
                crate::fc_warn!("send: the cache refused ({})", log::store_error(&error));
                self.connection.staging.release(&[handle]);
                self.notice(self.shell.say.get("Couldn't send that — try again."));
                if !round {
                    self.review(media);
                }
            }
        }
        self.refresh();
    }

    /// A finished voice note into the strip, in REVIEW until sent or removed (row 3's Stop, and a
    /// send that could not be queued).
    fn review(&self, note: StagedMedia) {
        if self.staging.add(note) {
            // The note just added is the strip's last.
            if let Some(added) = self.staging.items().last() {
                self.reviewing.borrow_mut().push(Rc::clone(added));
            }
        } else {
            self.notice(&composer_staging::cap_sentence(self.shell.say.as_ref()));
        }
        self.refresh();
    }

    /// The "not sent" row's Send (the Mac's `sendParked`): the note with its own caption — the
    /// words a consent question is about — and its own reply, the mentions read from the caption.
    /// `sent` runs once, when it is queued.
    fn send_parked(
        &self,
        media: Rc<StagedMedia>,
        caption: Option<String>,
        reply_to: Option<i64>,
        sent: Rc<RefCell<Option<Box<dyn FnOnce()>>>>,
    ) {
        let body = caption.clone().unwrap_or_default();
        let state = self.connection.session.state();
        let kind = self.chat.kind.as_str();
        let processor = state.assistant.as_ref().and_then(|a| a.processor.clone());
        if assistant_consent::is_withheld_from_an_unnamed_assistant(
            Some(kind),
            Some(&body),
            state.assistant.is_some(),
            processor.as_deref(),
        ) {
            self.notice(self.shell.say.get(
                "This server hasn't said which service answers, so nothing can be sent to the assistant here.",
            ));
            return;
        }
        if assistant_consent::is_required(
            Some(kind),
            Some(&body),
            processor.as_deref(),
            state.assistant_consent_at.as_deref(),
        ) {
            self.ask_consent(Rc::new(move |composer: &Composer| {
                composer.send_parked(
                    Rc::clone(&media),
                    caption.clone(),
                    reply_to,
                    Rc::clone(&sent),
                );
            }));
            return;
        }
        let handle = match self.connection.staging.stage(&media) {
            Ok(handle) => handle,
            Err(error) => {
                crate::fc_warn!("staging: a file could not be written ({})", error.kind());
                self.notice(self.shell.say.get("Couldn't send that."));
                return;
            }
        };
        let roster = self
            .connection
            .chats
            .members()
            .unwrap_or_else(|_| Rc::from(Vec::new()));
        let candidates = composer_mentions::candidates(Some(&self.chat), &roster);
        let mentions =
            composer_mentions::for_send(&body, &candidates, composer_mentions::enabled(Some(kind)))
                .unwrap_or_default();
        let options = QueueOptions {
            reply_to_message_id: reply_to,
            pending_files: vec![handle.clone()],
            mentions,
            ..QueueOptions::default()
        };
        match self.model.send(&body, &options) {
            Ok(_) => {
                self.connection.staging.release(&[handle]);
                self.flush();
                if let Some(sent) = sent.borrow_mut().take() {
                    sent();
                }
            }
            Err(error) => {
                crate::fc_warn!("send: the cache refused ({})", log::store_error(&error));
                self.connection.staging.release(&[handle]);
                self.notice(self.shell.say.get("Couldn't send that."));
            }
        }
        self.refresh();
    }

    /// A sticker picked in the panel (Windows' `SendStickerAsync`): through the same consent
    /// question the Send button asks in the assistant's chat (`pack_sending::gate_for`), then the
    /// item's bytes as cached (`PackModel::to_send`), staged and queued with `sticker` set — a reply
    /// when one is primed, the root in a thread — and remembered as used (`PackModel::sent`).
    pub fn send_sticker(&self, item: PackItemDto) {
        if self.sending_sticker.get() || self.closed.get() {
            return;
        }
        let say = Rc::clone(&self.shell.say);
        let state = self.connection.session.state();
        let processor = state.assistant.as_ref().and_then(|a| a.processor.clone());
        match pack_sending::gate_for(
            Some(&self.chat.kind),
            state.assistant.is_some(),
            processor.as_deref(),
            state.assistant_consent_at.as_deref(),
        ) {
            pack_sending::Gate::Ask => {
                self.ask_consent(Rc::new(move |composer: &Composer| {
                    composer.send_sticker(item.clone());
                }));
                return;
            }
            pack_sending::Gate::Withheld => {
                self.notice(say.get(
                    "This server hasn't said which service answers, so nothing can be sent to the assistant here.",
                ));
                return;
            }
            pack_sending::Gate::Send => {}
        }
        self.sending_sticker.set(true);
        self.notice_row.set_visible(false);
        // Taken now: the banner is the reader's to cancel while the bytes are fetched.
        let reply = match self.kind.get() {
            ComposerKind::Main => self.reply.get(),
            ComposerKind::Thread { root_id } => Some(root_id),
        };
        let weak = self.me.clone();
        let connection = Rc::clone(&self.connection);
        let model = Rc::clone(&self.model);
        let chat_id = self.chat.id;
        glib::spawn_future_local(async move {
            let answer = connection.stickers.to_send(&item).await;
            let composer = weak.upgrade();
            let media = match answer {
                Ok(Ok(media)) => media,
                Ok(Err(error)) => {
                    crate::fc_info!("sticker: not sent ({})", error.code);
                    if let Some(composer) = &composer {
                        composer.sending_sticker.set(false);
                        composer.notice(&if error.transient() {
                            say.get(
                                "Couldn't send that sticker. Check your connection and try again.",
                            )
                            .to_string()
                        } else {
                            pack_text::sentence(&error, say.as_ref())
                        });
                    }
                    return;
                }
                Err(error) => {
                    crate::fc_warn!("sticker: the cache refused ({})", log::store_error(&error));
                    if let Some(composer) = &composer {
                        composer.sending_sticker.set(false);
                        composer.notice(say.get("Couldn't send that — try again."));
                    }
                    return;
                }
            };
            let handle = match connection.staging.stage(&media) {
                Ok(handle) => handle,
                Err(error) => {
                    crate::fc_warn!("staging: a sticker could not be written ({})", error.kind());
                    if let Some(composer) = &composer {
                        composer.sending_sticker.set(false);
                        composer.notice(say.get("Couldn't send that — try again."));
                    }
                    return;
                }
            };
            let queued = model.send_sticker(&handle, reply);
            connection.staging.release(&[handle]);
            match queued {
                Ok(_) => {
                    crate::fc_info!("sticker: queued in chat {chat_id}");
                    if let Err(error) = connection.stickers.sent(item.id) {
                        crate::fc_warn!(
                            "sticker: the cache refused ({})",
                            log::store_error(&error)
                        );
                    }
                }
                Err(error) => {
                    crate::fc_warn!("sticker: the cache refused ({})", log::store_error(&error));
                }
            }
            let Some(composer) = composer else {
                return;
            };
            composer.sending_sticker.set(false);
            if reply.is_some() && composer.reply.get() == reply {
                // The reply has been sent; what is typed in the field stays, for its own message.
                composer.reply.set(None);
            }
            composer.flush();
            composer.refresh();
        });
    }

    /// The paperclip's Location (Windows' `ShareLocationAsync`): where this device is, once,
    /// through the Location portal — then sent alone, the draft as its caption and the reply it was
    /// primed for; whatever is staged stays staged.
    fn share_location(&self) {
        if self.locating.get() || self.closed.get() {
            return;
        }
        let say = Rc::clone(&self.shell.say);
        if let Some(busy) = self
            .staging
            .busy_reason(self.editing.borrow().is_some(), say.as_ref())
        {
            self.notice(&busy);
            return;
        }
        self.locating.set(true);
        self.say_notice(say.get("Finding your location…"), true);
        let window = (self.window)();
        let weak = self.me.clone();
        glib::spawn_future_local(async move {
            let found = integration::find_location(window).await;
            let Some(composer) = weak.upgrade().filter(|composer| !composer.closed.get()) else {
                return;
            };
            composer.locating.set(false);
            composer.notice_row.set_visible(false);
            match found {
                Ok(fix) => composer.send_location(&fix),
                Err(refusal) => {
                    crate::fc_info!("location: none ({refusal:?})");
                    composer.notice(if refusal == crate::attachments::location_portal::Refusal::Denied {
                        say.get("Family needs permission to use your location. Turn it on in Settings.")
                    } else {
                        say.get("Could not find your location.")
                    });
                }
            }
        });
    }

    /// The place, queued like any attachment (Windows' `SendLocationAsync`) — through the staging
    /// folder, so a send interrupted by anything is a bubble that can be finished. An edit begun
    /// meanwhile keeps its words: the place then goes captionless.
    fn send_location(&self, fix: &LocationFix) {
        let editing = self.editing.borrow().is_some();
        let draft = self.draft();
        let body = if editing {
            String::new()
        } else {
            fc_text::composer::trimmed_for_send(&draft)
                .unwrap_or("")
                .to_string()
        };
        let reply = if editing { None } else { self.reply.get() };
        let place = StagedMedia {
            latitude: Some(fix.latitude),
            longitude: Some(fix.longitude),
            accuracy_m: fix.accuracy_m,
            ..StagedMedia::new("location", "", Vec::new())
        };
        let handle = match self.connection.staging.stage(&place) {
            Ok(handle) => handle,
            Err(error) => {
                crate::fc_warn!("staging: a place could not be written ({})", error.kind());
                self.notice(self.shell.say.get("Could not share your location."));
                return;
            }
        };
        let roster = self
            .connection
            .chats
            .members()
            .unwrap_or_else(|_| Rc::from(Vec::new()));
        let candidates = composer_mentions::candidates(Some(&self.chat), &roster);
        let mentions = composer_mentions::for_send(
            &body,
            &candidates,
            composer_mentions::enabled(Some(&self.chat.kind)),
        )
        .unwrap_or_default();
        let options = QueueOptions {
            reply_to_message_id: reply,
            pending_files: vec![handle.clone()],
            mentions,
            ..QueueOptions::default()
        };
        let queued = self.model.send(&body, &options);
        self.connection.staging.release(&[handle]);
        match queued {
            Ok(_) => {
                crate::fc_info!("location: queued in chat {}", self.chat.id);
                if !editing {
                    self.set_text("");
                    self.reply.set(None);
                    if self.kind.get() == ComposerKind::Main {
                        drafts::shared().borrow_mut().sent(self.chat.id);
                    }
                }
                self.flush();
            }
            Err(error) => {
                crate::fc_warn!("location: the cache refused ({})", log::store_error(&error));
                self.notice(self.shell.say.get("Could not share your location."));
            }
        }
        self.refresh();
    }

    fn flush(&self) {
        (self.on_sent)();
        let (connection, on_sent) = (Rc::clone(&self.connection), Rc::clone(&self.on_sent));
        glib::spawn_future_local(async move {
            if let Err(error) = connection
                .live
                .flush(fc_core::protocol::send_rules::FlushTrigger::Queued)
                .await
            {
                crate::fc_warn!("outbox: flush failed ({})", log::store_error(&error));
            }
            on_sent();
        });
    }

    /// The assistant question (Windows' `AssistantConsentAsync`): the disclosure, then I Agree /
    /// Not Now — split into Agree With / Without Lookups where the server can look things up. A
    /// yes is recorded, `/me` re-read, and `then` (the send, the sticker) tried again.
    fn ask_consent(&self, then: Rc<dyn Fn(&Composer)>) {
        let answer = consent::ask(&self.root, &self.connection);
        let weak = self.me.clone();
        glib::spawn_future_local(async move {
            let answer = answer.await;
            let Some(composer) = weak.upgrade() else {
                return;
            };
            match answer {
                consent::Answer::Agreed => then(&composer),
                consent::Answer::NotSaved => composer.notice(
                    composer
                        .shell
                        .say
                        .get("Couldn't save your answer. Try again."),
                ),
                consent::Answer::NotNow => {}
            }
        });
    }

    // ---- the paperclip -----------------------------------------------------------------------------

    fn attach_menu(&self) -> gio::Menu {
        let state = self.connection.session.state();
        let assistant_chat = self.chat.kind == chat_kinds::ASSISTANT;
        let offers_picture = state.assistant.as_ref().is_some_and(|a| a.vision)
            && state.family.as_ref().is_some_and(|f| f.ai_vision);
        let groups = attach_menu::groups(
            assistant_chat,
            offers_picture,
            chat_kinds::is_shared(Some(&self.chat.kind)),
            self.recording().video_offered(),
            attach_menu::assistant_available(state.assistant.as_ref()),
        );
        let menu = gio::Menu::new();
        for group in groups {
            let section = gio::Menu::new();
            for item in group {
                if item == AttachItem::Location && !integration::location_offered() {
                    continue;
                }
                if matches!(item, AttachItem::RecordVoice) && !self.recording().can_record() {
                    continue;
                }
                section.append(
                    Some(item.label()),
                    Some(&format!("composer.{}", attach_action(item))),
                );
            }
            if section.n_items() > 0 {
                menu.append_section(None, &section);
            }
        }
        menu
    }

    fn install_attach_actions(self: &Rc<Self>) {
        let group = gio::SimpleActionGroup::new();
        let items = [
            AttachItem::PhotoOrVideo,
            AttachItem::AssistantPhoto,
            AttachItem::File,
            AttachItem::Paste,
            AttachItem::RecordVoice,
            AttachItem::RecordVideo,
            AttachItem::AskAssistant,
            AttachItem::Location,
            AttachItem::Poll,
        ];
        for item in items {
            let action = gio::SimpleAction::new(attach_action(item), None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(composer) = weak.upgrade() {
                    composer.attach_item(item);
                }
            });
            group.add_action(&action);
        }
        self.root.insert_action_group("composer", Some(&group));
    }

    fn attach_item(&self, item: AttachItem) {
        let picks = matches!(
            item,
            AttachItem::PhotoOrVideo
                | AttachItem::AssistantPhoto
                | AttachItem::File
                | AttachItem::Paste
        );
        if picks
            && let Some(busy) = self
                .staging
                .busy_reason(self.editing.borrow().is_some(), self.shell.say.as_ref())
        {
            self.notice(&busy);
            return;
        }
        match item {
            AttachItem::PhotoOrVideo => self.pick(Some(attach_menu::PHOTO_OR_VIDEO_TYPES)),
            AttachItem::AssistantPhoto => self.pick(Some(&[
                ".jpg", ".jpeg", ".png", ".heic", ".heif", ".webp", ".gif",
            ])),
            AttachItem::File => self.pick(None),
            AttachItem::Paste => self.paste_from_menu(),
            AttachItem::RecordVoice => self.record_voice(),
            AttachItem::RecordVideo => self.record_video(),
            AttachItem::AskAssistant => {
                self.insert(&format!("{} ", fc_text::assistant::TOKEN));
                self.focus();
            }
            AttachItem::Location => self.share_location(),
            AttachItem::Poll => {
                let weak = self.me.clone();
                crate::conversation::polls::compose(
                    &self.root,
                    Rc::new(move |question, options| {
                        if let Some(composer) = weak.upgrade() {
                            composer.send_poll(question, options);
                        }
                    }),
                );
            }
        }
    }

    /// `gtk::FileDialog::open_multiple_future` — "Attach" as the accept label; `types` a filter.
    fn pick(&self, types: Option<&[&str]>) {
        let say = &self.shell.say;
        let dialog = gtk::FileDialog::builder()
            .accept_label(say.get("Attach"))
            .modal(true)
            .build();
        if let Some(types) = types {
            let filter = gtk::FileFilter::new();
            for suffix in types {
                filter.add_suffix(suffix.trim_start_matches('.'));
            }
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));
        }
        let window = (self.window)();
        let weak = self.me.clone();
        glib::spawn_future_local(async move {
            let Ok(files) = dialog.open_multiple_future(window.as_ref()).await else {
                return;
            };
            let picked: Vec<gio::File> = (0..files.n_items())
                .filter_map(|i| files.item(i).and_then(|o| o.downcast::<gio::File>().ok()))
                .collect();
            if let Some(composer) = weak.upgrade() {
                composer.stage_files(picked);
            }
        });
    }

    /// Stages `files` (picked, dropped, handed over), one at a time, stopping at the cap.
    pub fn stage_files(&self, files: Vec<gio::File>) {
        if let Some(staging) = self.staging_of(files) {
            glib::spawn_future_local(staging);
        }
    }

    /// [`stage_files`](Self::stage_files) as a future that resolves once every file has been read
    /// into the strip (or refused) — what P2's inbox waits for before it deletes its copies
    /// (`ChatRoutes::stage_files`). `None` when nothing is staged (no files, or the strip is busy:
    /// said on the notice line).
    pub fn staging_of(&self, files: Vec<gio::File>) -> Option<LocalBoxFuture<'static, ()>> {
        if files.is_empty() {
            return None;
        }
        if let Some(busy) = self
            .staging
            .busy_reason(self.editing.borrow().is_some(), self.shell.say.as_ref())
        {
            self.notice(&busy);
            return None;
        }
        crate::fc_info!("staging: {} file(s) in chat {}", files.len(), self.chat.id);
        self.notice(self.shell.say.get("Preparing…"));
        let staging = Rc::clone(&self.staging);
        let weak = self.me.clone();
        let say = Rc::clone(&self.shell.say);
        let animates = self.chat.kind != chat_kinds::ASSISTANT;
        self.refresh();
        Some(Box::pin(async move {
            let still = weak.clone();
            let said = staging
                .ingest(
                    files,
                    move |file, cancel| integration::prepare_file(file, cancel, animates),
                    move || still.upgrade().is_some_and(|c| !c.closed.get()),
                    say.as_ref(),
                )
                .await;
            if let Some(composer) = weak.upgrade() {
                composer.notice_row.set_visible(false);
                if let Some(said) = said {
                    composer.notice(&said);
                }
                composer.guard.borrow_mut().changed(false);
                composer.refresh();
            }
        }))
    }

    /// Pasted bytes, prepared exactly as a picked file is (`integration::prepare_file`): a video or
    /// a sound brought to the profile, an animated picture made into the animation (the Mac's
    /// paste goes through the same door as its picker). Written to a folder of their own under the
    /// temporaries first — named as the paste names them — and gone once staged; where the disk
    /// refuses, the bytes are staged as they are.
    fn stage_pasted(&self, bytes: Vec<u8>, mime: &str) {
        let name = fc_text::media::pasted_name(mime);
        let folder =
            self.shell
                .paths
                .tmp_dir()
                .join(format!("paste-{}-{}", std::process::id(), now_ms()));
        let path = folder.join(&name);
        let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&path, &bytes));
        if let Err(error) = written {
            crate::fc_warn!("paste: a temporary could not be written ({})", error.kind());
            let _ = std::fs::remove_dir_all(&folder);
            self.stage_bytes(bytes, mime);
            return;
        }
        match self.staging_of(vec![gio::File::for_path(&path)]) {
            Some(staging) => {
                glib::spawn_future_local(async move {
                    staging.await;
                    let _ = std::fs::remove_dir_all(&folder);
                });
            }
            None => {
                let _ = std::fs::remove_dir_all(&folder);
            }
        }
    }

    /// Words added at the end of the draft, within the body's ceiling — what a dropped link is
    /// (`fc_text::composer::appending`; the ceiling's sentence when it bites). Nothing is sent.
    fn append_words(&self, words: &str) {
        let draft = self.draft();
        let joined = if draft.is_empty() || draft.ends_with(char::is_whitespace) {
            words.to_string()
        } else {
            format!(" {words}")
        };
        let outcome = fc_text::composer::appending(&joined, &draft);
        if let Some(notice) = outcome.notice() {
            self.notice(&notice.said());
        }
        match outcome {
            fc_text::composer::Paste::Appended(text)
            | fc_text::composer::Paste::Truncated(text) => {
                self.set_text(&text);
            }
            fc_text::composer::Paste::Full => {}
        }
        self.focus();
    }

    /// One item from bytes (a pasted screenshot, or a paste the disk would not hold).
    fn stage_bytes(&self, bytes: Vec<u8>, mime: &str) {
        let name = fc_text::media::pasted_name(mime);
        let outcome = integration::prepare_bytes(bytes, mime, &name);
        match outcome.media {
            Some(media) => {
                if !self.staging.add(media) {
                    self.notice(&composer_staging::cap_sentence(self.shell.say.as_ref()));
                }
            }
            None => self.notice(&composer_staging::sentence(
                outcome
                    .failure
                    .unwrap_or(fc_logic::composer_staging::PrepFailure::Unreadable),
                self.shell.say.as_ref(),
            )),
        }
        self.guard.borrow_mut().changed(false);
        self.refresh();
    }

    fn draw_strip(&self) {
        let items = self.staging.items();
        let addresses: Vec<usize> = items.iter().map(|item| Rc::as_ptr(item) as usize).collect();
        let playing = self.strip_player.borrow().as_ref().map(|(at, _, _)| *at);
        let drawn = (addresses.clone(), playing);
        if *self.strip_drawn.borrow() == drawn
            && self.strip.first_child().is_some() != items.is_empty()
        {
            return;
        }
        *self.strip_drawn.borrow_mut() = drawn;
        self.thumbnails
            .borrow_mut()
            .retain(|at, _| addresses.contains(at));
        while let Some(child) = self.strip.first_child() {
            self.strip.remove(&child);
        }
        self.strip.set_visible(!items.is_empty());
        let say = &self.shell.say;
        let separator = self.shell.numbers.decimal;
        for item in items {
            let chip = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            chip.add_css_class("fc-chat-chip");
            chip.set_halign(gtk::Align::Start);
            let recorded = fc_logic::voice_notes::is_recorded(&item);
            if recorded {
                self.voice_chip(&chip, &item);
            } else {
                let label = composer_staging::label(&item, say.as_ref(), separator);
                if let Some(thumbnail) = self.thumbnail(&item) {
                    chip.add_css_class("fc-chat-chip-pictured");
                    chip.append(&thumbnail);
                    chip.append(&gtk::Label::new(Some(&label)));
                } else if item.kind == "file" {
                    // A file: its name, and what may go with it (the Mac's `fileChip`).
                    let words = gtk::Box::new(gtk::Orientation::Vertical, 1);
                    words.set_valign(gtk::Align::Center);
                    words.append(
                        &gtk::Label::builder()
                            .label(&label)
                            .xalign(0.0)
                            .ellipsize(gtk::pango::EllipsizeMode::Middle)
                            .max_width_chars(28)
                            .build(),
                    );
                    words.append(
                        &gtk::Label::builder()
                            .label(say.get("Add a message, or send it on its own."))
                            .xalign(0.0)
                            .css_classes(["caption", "dim-label"])
                            .build(),
                    );
                    chip.append(&words);
                } else {
                    chip.append(&gtk::Label::new(Some(&label)));
                }
            }
            let remove = gtk::Button::builder()
                .icon_name("window-close-symbolic")
                .css_classes(["flat", "circular"])
                .valign(gtk::Align::Center)
                .tooltip_text(if recorded {
                    say.get("Delete recording")
                } else {
                    say.get("Remove")
                })
                .build();
            remove.update_property(&[gtk::accessible::Property::Label(if recorded {
                say.get("Delete recording")
            } else {
                say.get("Remove attachment")
            })]);
            let weak = self.me.clone();
            remove.connect_clicked(move |button| {
                if let Some(composer) = weak.upgrade() {
                    composer.unstage(&item, button);
                }
            });
            chip.append(&remove);
            self.strip.append(&chip);
        }
    }

    /// A staged picture's or video's thumbnail (the Mac's `StagedAttachment`): its preview, or a
    /// photo's own bytes, cropped to a square; a video carries a play mark. `None` for anything
    /// else, or what cannot be read.
    fn thumbnail(&self, item: &Rc<StagedMedia>) -> Option<gtk::Widget> {
        if item.kind != "photo" && item.kind != "video" {
            return None;
        }
        let at = Rc::as_ptr(item) as usize;
        let held = self.thumbnails.borrow().get(&at).cloned();
        let texture = match held {
            Some(texture) => texture,
            None => {
                let bytes = item
                    .preview
                    .as_deref()
                    .or((item.kind == "photo").then_some(item.bytes.as_slice()))?;
                let texture = gtk::gdk::Texture::from_bytes(&glib::Bytes::from(bytes)).ok()?;
                self.thumbnails.borrow_mut().insert(at, texture.clone());
                texture
            }
        };
        let picture = gtk::Picture::builder()
            .paintable(&texture)
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .width_request(40)
            .height_request(40)
            .css_classes(["fc-chat-chip-thumb"])
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        picture.set_overflow(gtk::Overflow::Hidden);
        if item.kind != "video" {
            return Some(picture.upcast());
        }
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&picture));
        overlay.add_overlay(
            &gtk::Image::builder()
                .icon_name("media-playback-start-symbolic")
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .css_classes(["osd", "circular", "fc-chat-chip-play"])
                .build(),
        );
        Some(overlay.upcast())
    }

    /// A recorded voice note in review: ▶ / ❚❚, its shape, its length.
    fn voice_chip(&self, chip: &gtk::Box, item: &Rc<StagedMedia>) {
        let say = &self.shell.say;
        let at = Rc::as_ptr(item) as usize;
        let playing = self
            .strip_player
            .borrow()
            .as_ref()
            .is_some_and(|(held, _, _)| *held == at);
        let name = if playing {
            say.get("Pause")
        } else {
            say.get("Play")
        };
        let play = gtk::Button::builder()
            .icon_name(if playing {
                "media-playback-pause-symbolic"
            } else {
                "media-playback-start-symbolic"
            })
            .css_classes(["flat", "circular"])
            .valign(gtk::Align::Center)
            .tooltip_text(name)
            .build();
        play.update_property(&[gtk::accessible::Property::Label(name)]);
        {
            let (weak, item) = (self.me.clone(), Rc::clone(item));
            play.connect_clicked(move |_| {
                if let Some(composer) = weak.upgrade() {
                    composer.play_staged(&item);
                }
            });
        }
        chip.append(&play);
        let width = 96.0;
        let waves = crate::media::waves::Waves::new(
            width,
            fc_logic::voice_look::CHIP_WAVE_HEIGHT,
            crate::media::waves::Ink::Accent,
        );
        waves.set_levels(fc_logic::voice_look::bars(
            item.waveform.as_deref(),
            fc_logic::voice_look::bar_count(width),
        ));
        waves.widget().set_valign(gtk::Align::Center);
        chip.append(waves.widget());
        let seconds = f64::from(item.duration_ms.unwrap_or(0)) / 1000.0;
        chip.append(
            &gtk::Label::builder()
                .label(fc_text::media::time_label(seconds))
                .css_classes(["numeric", "caption"])
                .valign(gtk::Align::Center)
                .build(),
        );
        chip.update_property(&[gtk::accessible::Property::Label(&format!(
            "{} · {}",
            say.get("Voice message"),
            fc_text::media::time_label(seconds)
        ))]);
    }

    /// A staged voice note's ▶: played on this device, one thing at a time (`now_playing`), never
    /// while a recording runs.
    fn play_staged(&self, item: &Rc<StagedMedia>) {
        let at = Rc::as_ptr(item) as usize;
        let was = self
            .strip_player
            .borrow()
            .as_ref()
            .map(|(held, _, _)| *held);
        self.stop_strip_player();
        if was == Some(at) {
            self.draw_strip();
            return;
        }
        let registry = crate::media::now_playing();
        if registry.refuses() {
            self.notice(self.shell.say.get("You can play this after recording."));
            return;
        }
        let Ok(file) =
            crate::media::gst_util::temp_file(&self.shell.paths.tmp_dir(), "review", "m4a")
        else {
            return;
        };
        if std::fs::write(&file, &item.bytes).is_err() {
            crate::media::gst_util::remove_file(&file);
            return;
        }
        let player = self.shell.players.get().create(false);
        let weak = self.me.clone();
        let claim = registry.claim(
            fc_logic::round_look::Playing::VoiceNote,
            Rc::new({
                let weak = weak.clone();
                move || {
                    if let Some(composer) = weak.upgrade() {
                        composer.stop_strip_player();
                        composer.draw_strip();
                    }
                }
            }),
        );
        player.on_state().subscribe(move |state| {
            if matches!(state, PlayerState::Ended | PlayerState::Failed)
                && let Some(composer) = weak.upgrade()
            {
                composer.stop_strip_player();
                composer.draw_strip();
            }
        });
        player.load(MediaSource::File(file.clone()));
        player.set_rate(crate::media::voice_bubble::current_rate(&self.shell));
        player.play();
        *self.strip_player.borrow_mut() = Some((at, player, claim));
        self.draw_strip();
        // The temporary goes once the player has it open; the sweep at launch takes a leftover.
        glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
            crate::media::gst_util::remove_file(&file)
        });
    }

    fn stop_strip_player(&self) {
        let playing = self.strip_player.borrow_mut().take();
        if let Some((_, player, claim)) = playing {
            player.stop();
            crate::media::now_playing().release(claim);
        }
    }

    /// A chip's ✕: a recorded voice note of ten seconds or more asks first — it cannot be made
    /// again — and says "Recording deleted" (the Mac's `StagedAttachment`); anything else goes.
    fn unstage(&self, item: &Rc<StagedMedia>, anchor: &gtk::Button) {
        let recorded = fc_logic::voice_notes::is_recorded(item);
        let gone = {
            let (weak, item) = (self.me.clone(), Rc::clone(item));
            move || {
                let Some(composer) = weak.upgrade() else {
                    return;
                };
                let at = Rc::as_ptr(&item) as usize;
                if composer
                    .strip_player
                    .borrow()
                    .as_ref()
                    .is_some_and(|(held, _, _)| *held == at)
                {
                    composer.stop_strip_player();
                }
                composer.staging.remove(&item);
                composer
                    .reviewing
                    .borrow_mut()
                    .retain(|note| !Rc::ptr_eq(note, &item));
                composer.guard.borrow_mut().changed(false);
                if recorded {
                    let said = composer.shell.say.get("Recording deleted");
                    composer
                        .strip
                        .announce(said, gtk::AccessibleAnnouncementPriority::Medium);
                }
                composer.refresh();
            }
        };
        if !(recorded && fc_logic::voice_notes::asks_before_deleting(item.duration_ms.unwrap_or(0)))
        {
            gone();
            return;
        }
        let say = &self.shell.say;
        let dialog = adw::AlertDialog::new(Some(say.get("Delete this recording?")), None);
        dialog.add_response("cancel", say.get("Cancel"));
        dialog.add_response("delete", say.get("Delete"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        dialog.connect_response(Some("delete"), move |_, _| gone());
        dialog.present(Some(anchor));
    }

    // ---- paste and drop ------------------------------------------------------------------------

    /// What the clipboard holds, attached (§6.3): files, a texture, a picture type — in
    /// `PASTE_PREFERENCE` order. Words, when that is all there is, go into the field.
    pub fn paste_attachment(&self) {
        let clipboard = self.field.clipboard();
        let formats = clipboard.formats();
        let weak = self.me.clone();
        if formats.contains_type(gtk::gdk::FileList::static_type()) {
            glib::spawn_future_local(async move {
                let Ok(value) = clipboard
                    .read_value_future(gtk::gdk::FileList::static_type(), glib::Priority::DEFAULT)
                    .await
                else {
                    return;
                };
                if let (Ok(list), Some(composer)) =
                    (value.get::<gtk::gdk::FileList>(), weak.upgrade())
                {
                    let files: Vec<gio::File> = list
                        .files()
                        .into_iter()
                        .filter(|file| !integration::is_folder(file))
                        .collect();
                    composer.stage_files(files);
                }
            });
            return;
        }
        let offered: Vec<String> = formats
            .mime_types()
            .iter()
            .map(|mime| mime.to_string())
            .collect();
        if let Some(mime) = fc_text::media::chosen_paste_type(&offered) {
            glib::spawn_future_local(async move {
                let Ok((stream, _)) = clipboard
                    .read_future(&[mime.as_str()], glib::Priority::DEFAULT)
                    .await
                else {
                    return;
                };
                let bytes = read_all(&stream).await;
                if let Some(composer) = weak.upgrade() {
                    match bytes {
                        Some(bytes) => composer.stage_pasted(bytes, &mime),
                        None => composer.notice(composer.shell.say.get("Couldn't read that file.")),
                    }
                }
            });
            return;
        }
        if formats.contains_type(gtk::gdk::Texture::static_type()) {
            glib::spawn_future_local(async move {
                let Ok(Some(texture)) = clipboard.read_texture_future().await else {
                    return;
                };
                let png = texture.save_to_png_bytes().to_vec();
                if let Some(composer) = weak.upgrade() {
                    composer.stage_bytes(png, "image/png");
                }
            });
            return;
        }
        if has_words(&formats) {
            self.focus();
            self.field.emit_paste_clipboard();
            return;
        }
        self.notice(self.shell.say.get("There's nothing to paste."));
    }

    /// The paperclip's Paste: the same rule; "There's nothing to paste." only when nothing — not
    /// even words — is there (the Mac's fix).
    fn paste_from_menu(&self) {
        self.paste_attachment();
    }

    /// The whole page as the drop target (§6.4), with the accent outline while a drag hovers.
    pub fn attach_drop(&self, page: &gtk::Overlay) {
        let outline = gtk::Box::builder()
            .css_classes(["fc-chat-drop"])
            .can_target(false)
            .visible(false)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        page.add_overlay(&outline);
        *self.drop_outline.borrow_mut() = Some(outline.clone());
        let target = gtk::DropTarget::new(
            gtk::gdk::FileList::static_type(),
            gtk::gdk::DragAction::COPY,
        );
        {
            let outline = outline.clone();
            target.connect_enter(move |_, _, _| {
                outline.set_visible(true);
                gtk::gdk::DragAction::COPY
            });
        }
        {
            let outline = outline.clone();
            target.connect_leave(move |_| outline.set_visible(false));
        }
        let weak = self.me.clone();
        target.connect_drop(move |_, value, _, _| {
            outline.set_visible(false);
            let Ok(list) = value.get::<gtk::gdk::FileList>() else {
                return false;
            };
            // A link off a web page arrives as a file with a web address: its bytes are on
            // somebody else's server, so it belongs in the draft as words (the Mac's
            // `DroppedAttachment`, `.link`) — files, when there are any, are what is attached.
            let (links, files): (Vec<gio::File>, Vec<gio::File>) = list
                .files()
                .into_iter()
                .partition(|file| matches!(file.uri_scheme().as_deref(), Some("http" | "https")));
            let files: Vec<gio::File> = files
                .into_iter()
                .filter(|file| !integration::is_folder(file))
                .collect();
            let Some(composer) = weak.upgrade() else {
                return false;
            };
            if !files.is_empty() {
                composer.stage_files(files);
                return true;
            }
            if !links.is_empty() {
                let words: Vec<String> = links.iter().map(|file| file.uri().to_string()).collect();
                composer.append_words(&words.join("\n"));
                return true;
            }
            // Only folders (or nothing): refused, and the drag springs back (the Mac's answer).
            false
        });
        page.add_controller(target);
    }

    // ---- recording (P3) ------------------------------------------------------------------------

    /// Ctrl+Shift+R / "Record Voice Message": start (beside words or items, row 3) — or, during
    /// one, stop into review. Never sends.
    pub fn record_voice(&self) {
        let recording = self.recording();
        if self.kind.get() != ComposerKind::Main || !recording.can_record() {
            return;
        }
        if recording.recording() != Recording::None {
            recording.stop();
        } else {
            let beside = !self.draft().trim().is_empty() || !self.staging.is_empty();
            recording.start_voice(beside);
        }
        self.refresh();
    }

    pub fn record_video(&self) {
        let recording = self.recording();
        if self.kind.get() == ComposerKind::Main && recording.video_offered() {
            recording.record_video();
        }
    }

    /// The composer goes away (another chat, a closed window): the draft kept, a recording
    /// interrupted as "left the chat".
    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        if self.kind.get() == ComposerKind::Main {
            let words = match self.editing.borrow().as_ref() {
                Some((_, saved)) => saved.clone(),
                None => self.draft(),
            };
            drafts::shared().borrow_mut().save(self.chat.id, &words);
        }
        self.stop_strip_player();
        self.recording().interrupt(RecordingEnd::LeftChat);
        self.park_review(RecordingEnd::LeftChat);
        self.staging.cancel_preparing();
        self.mentions.popdown();
    }

    /// A voice note still in REVIEW — staged beside the words by row 3's Stop — becomes a voice
    /// message that was not sent when the chat is left or the window really closes
    /// (`not_sent::parks_review`): written to P3's not-sent store with the words as its caption
    /// and the reply with them (`not_sent::for_review`), and taken out of the strip. The draft
    /// saved for the chat keeps the words unless the note took them.
    fn park_review(&self, why: RecordingEnd) {
        if !not_sent::parks_review(why) {
            return;
        }
        let reviewing: Vec<Rc<StagedMedia>> = self.reviewing.borrow_mut().drain(..).collect();
        let items = self.staging.items();
        let notes: Vec<Rc<StagedMedia>> = items
            .into_iter()
            .filter(|item| reviewing.iter().any(|note| Rc::ptr_eq(note, item)))
            .collect();
        if notes.is_empty() {
            return;
        }
        let Some(store) = crate::media::recording::installed().and_then(|slots| slots.store())
        else {
            crate::fc_warn!("not sent: nobody is signed in; a voice note in review was let go");
            return;
        };
        let editing = self.editing.borrow().is_some();
        let take = not_sent::for_review(&self.draft(), self.reply.get(), editing);
        let now = self.shell.clock.now();
        for note in &notes {
            let duration = note.duration_ms.unwrap_or(0);
            let parked = store.park(
                self.chat.id,
                &note.bytes,
                duration,
                take.reply_to,
                take.caption.as_deref(),
                now,
            );
            match parked {
                Ok(entry) => crate::fc_info!(
                    "not sent: a note in review kept in chat {} ({} ms)",
                    entry.chat_id,
                    entry.duration_ms
                ),
                Err(error) => {
                    crate::fc_warn!(
                        "not sent: the disk refused ({}); held in memory",
                        error.kind()
                    );
                    store.hold(
                        self.chat.id,
                        &note.bytes,
                        duration,
                        take.reply_to,
                        take.caption.as_deref(),
                        now,
                    );
                }
            }
            self.staging.remove(note);
        }
        if take.clears_words && self.kind.get() == ComposerKind::Main {
            drafts::shared().borrow_mut().save(self.chat.id, "");
        }
        if take.clears_reply {
            self.reply.set(None);
        }
    }
}

/// The name of a paperclip line's action on the `composer` group.
fn attach_action(item: AttachItem) -> &'static str {
    match item {
        AttachItem::PhotoOrVideo => "photo-or-video",
        AttachItem::AssistantPhoto => "assistant-photo",
        AttachItem::File => "file",
        AttachItem::Paste => "paste",
        AttachItem::RecordVoice => "record-voice",
        AttachItem::RecordVideo => "record-video",
        AttachItem::AskAssistant => "ask-assistant",
        AttachItem::Location => "location",
        AttachItem::Poll => "poll",
    }
}

/// The clipboard offers words.
fn has_words(formats: &gtk::gdk::ContentFormats) -> bool {
    formats.contains_type(glib::GString::static_type())
        || formats.contains_type(String::static_type())
        || formats
            .mime_types()
            .iter()
            .any(|mime| mime.starts_with("text/plain"))
}

/// The clipboard offers something to attach: files, a texture, or a type the paste rule takes.
fn has_attachment(formats: &gtk::gdk::ContentFormats) -> bool {
    if formats.contains_type(gtk::gdk::FileList::static_type())
        || formats.contains_type(gtk::gdk::Texture::static_type())
    {
        return true;
    }
    let offered: Vec<String> = formats.mime_types().iter().map(|m| m.to_string()).collect();
    fc_text::media::chosen_paste_type(&offered).is_some()
}

/// A stream read to its end (at most the protocol's ceiling and a little).
async fn read_all(stream: &gio::InputStream) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    loop {
        let chunk = stream
            .read_bytes_future(64 * 1024, glib::Priority::DEFAULT)
            .await
            .ok()?;
        if chunk.is_empty() {
            break;
        }
        bytes.extend_from_slice(&chunk);
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > fc_text::media::SIZE_LIMIT + 1 {
            break;
        }
    }
    Some(bytes)
}

/// The pointer behind a press, as `SlotGuard` reads it.
fn slot_device(device: Option<gtk::gdk::Device>) -> composer_button::SlotDevice {
    use composer_button::SlotDevice;
    match device.map(|device| device.source()) {
        None | Some(gtk::gdk::InputSource::Keyboard) => SlotDevice::None,
        Some(gtk::gdk::InputSource::Touchscreen) => SlotDevice::Touch,
        Some(gtk::gdk::InputSource::Pen) => SlotDevice::Pen,
        Some(_) => SlotDevice::Mouse,
    }
}

/// This composer as P3's `ComposerHost`.
struct Host(Weak<Composer>);

impl ComposerHost for Host {
    fn chat_id(&self) -> i64 {
        self.0.upgrade().map_or(0, |composer| composer.chat.id)
    }

    fn draft(&self) -> String {
        self.0
            .upgrade()
            .map(|composer| composer.draft())
            .unwrap_or_default()
    }

    fn staged(&self) -> usize {
        self.0
            .upgrade()
            .map_or(0, |composer| composer.staging.len())
    }

    fn window(&self) -> Option<gtk::Window> {
        self.0.upgrade().and_then(|composer| (composer.window)())
    }

    fn stage(&self, note: StagedMedia) {
        if let Some(composer) = self.0.upgrade() {
            composer.review(note);
        }
    }

    fn send(&self, media: StagedMedia) {
        if let Some(composer) = self.0.upgrade() {
            composer.send_media(media, false);
        }
    }

    fn send_video_message(&self, media: StagedMedia) {
        if let Some(composer) = self.0.upgrade() {
            composer.send_media(media, true);
        }
    }

    fn send_parked(
        &self,
        media: StagedMedia,
        caption: Option<String>,
        reply_to: Option<i64>,
        sent: Box<dyn FnOnce()>,
    ) {
        if let Some(composer) = self.0.upgrade() {
            let sent = Rc::new(RefCell::new(Some(sent)));
            composer.send_parked(Rc::new(media), caption, reply_to, sent);
        }
    }

    fn reply_line(&self, message_id: i64) -> Option<String> {
        let composer = self.0.upgrade()?;
        let message = composer.connection.chats.message(message_id).ok()??;
        quotes::banner(
            &message,
            &composer.connection.chats,
            composer.shell.say.as_ref(),
        )
        .ok()
    }

    fn changed(&self) {
        if let Some(composer) = self.0.upgrade() {
            composer.refresh();
        }
    }

    fn reply_to(&self) -> Option<i64> {
        self.0
            .upgrade()
            .filter(|composer| composer.editing.borrow().is_none())
            .and_then(|composer| composer.reply.get())
    }

    fn release_reply(&self) {
        if let Some(composer) = self.0.upgrade() {
            composer.set_reply(None);
        }
    }

    fn notice(&self, sentence: &str) {
        if let Some(composer) = self.0.upgrade() {
            let quiet = composer.recording().recording() != Recording::None;
            composer.say_notice(sentence, !quiet);
        }
    }
}
