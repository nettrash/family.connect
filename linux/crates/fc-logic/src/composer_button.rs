//! Voice and video messages from the Send button (`ComposerButton.cs`;
//! docs/audio-video-messages-2026-10-04.md — "the plan" — issue #79): the composer's trailing slot,
//! the video button inside the empty field, and the round video's arithmetic.
//!
//! THE RULES ARE `fc_text::record`'S, CALLED RATHER THAN COPIED — the same Rust the web client runs,
//! held case for case to `record-vectors.json` — and re-exported here: which row of S1.3 the slot is
//! ([`composer_slot`], [`Slot`]: the C#'s record of a kind, Save's switch and a dimmed microphone's
//! reason is fc-text's enum carrying the last two), the video button ([`video_door`], [`Door`]), the
//! round video's numbers and the S1.1 constants. THE FIRST MATCHING ROW WINS: the recorder, a
//! recording running, an edit (never a microphone, even with the field cleared), something to send,
//! the assistant's chat, and only then the microphone — dimmed for a call, an attachment on its way
//! or a voice message not sent, in that order.
//!
//! What is the desktop's own, and so here: how each row is drawn and named ([`face`] — an icon the
//! shell maps to a symbolic icon name, where Windows had a Segoe glyph), which press opens the
//! microphone's menu ([`respond`], adapted to GTK's gestures), the 600 ms guard and the press
//! reaching the slot ([`SlotGuard`]), and Ctrl+Shift+R held down ([`ShortcutPresses`]).
//!
//! EVERY INPUT CLICKS (S8.6, Decision 28; amended 2026-10-06, #79 — the hold is gone from every
//! client): a press of any length with a mouse, a finger or a pen records hands-free. The menu opens
//! for a secondary click (a mouse's right button, a pen's barrel button), Shift+F10 or the Menu key —
//! never for a long press, which simply clicks (shell-design §6.1).

use fc_core::text::string_catalog::StringCatalog;

pub use fc_text::record::{
    ACTIVATION_GUARD_MS, AttachmentFlags, DEFAULT_MAX_ROUND_VIDEO_MS, DELETE_ASKS_FROM_MS, Dimmed,
    Door, DoorInputs, MIN_TARGET_WINDOWS_EPX, ROUND_CAP_MARGIN_MS, ROUND_DIAMETER_COMPACT,
    ROUND_DIAMETER_REGULAR, ROUND_WARNING_LEAD_MS, Recording, SHORTEST_RECORDING_MS,
    SLOT_CROSSFADE_MS, Slot, SlotInputs, VIDEO_DOOR_LABEL, VIDEO_DOOR_TOOLTIP, VOICE_CAP_MS,
    VOICE_WARNING_MS, WidthClass, composer_slot, is_round, round_cap_ms, round_diameter,
    round_warning_ms, video_door,
};

/// The slot's visual, inside its [`MIN_TARGET_WINDOWS_EPX`] target: a 40-px accent disc in a 44-px
/// target (shell-design §6.1 — the desktop's unit is the logical pixel, Windows' the effective one;
/// the target is the same 44).
pub const SLOT_VISUAL_PX: u32 = 40;

/// The shortcut that records (S1.6): Ctrl+Shift+R — the app's first keyboard accelerator — as the
/// tooltip and menus write it (GTK's accelerator `<Control><Shift>r`).
pub const RECORD_SHORTCUT: &str = "Ctrl+Shift+R";

/// Save's own switch (row 4) — the C# record's `Enabled`; `None` on every other row.
pub fn enabled(slot: Slot) -> Option<bool> {
    match slot {
        Slot::Save { enabled } => Some(enabled),
        _ => None,
    }
}

/// Why a dimmed microphone is dimmed (rows 7–9) — the C# record's `Reason`; `None` on every other
/// row.
pub fn reason(slot: Slot) -> Option<Dimmed> {
    match slot {
        Slot::Dimmed(reason) => Some(reason),
        _ => None,
    }
}

/// The slot's accessibility label (S6), in the reader's language, or `None` where the recorder owns
/// the row (`Slot::label`, said).
pub fn label(slot: Slot, say: &dyn StringCatalog) -> Option<String> {
    let said = match slot {
        Slot::Recorder => return None,
        Slot::SendVoice => say.get("Send voice message"),
        Slot::StopRecording => say.get("Stop recording"),
        Slot::Save { .. } => say.get("Save"),
        Slot::Send | Slot::SendDisabled => say.get("Send"),
        Slot::Dimmed(_) | Slot::Microphone => say.get("Record voice message"),
    };
    Some(said.to_string())
}

/// What activating a dimmed slot — or a dimmed video button — says instead of acting
/// (`Dimmed::notice`, said); `None` where nothing is dimmed.
pub fn notice(reason: Option<Dimmed>, say: &dyn StringCatalog) -> Option<String> {
    let said = match reason? {
        Dimmed::Call => say.get("You can record a message after the call."),
        Dimmed::Busy => say.get("Wait until the current attachment is done."),
        Dimmed::NotSent => say.get("Send or delete the voice message that wasn't sent first."),
    };
    Some(said.to_string())
}

/// Why a dimmed video button is dimmed (rows 7 and 8); `None` where it is hidden or shown.
pub fn door_reason(door: Door) -> Option<Dimmed> {
    match door {
        Door::Dimmed(reason) => Some(reason),
        _ => None,
    }
}

/// The video button's label, where it is drawn, in the reader's language. (Windows kept the English
/// key, because no Windows build drew the button before Phase 3d; the catalogue has the sentence,
/// and Linux says it.)
pub fn door_label(door: Door, say: &dyn StringCatalog) -> Option<String> {
    match door {
        Door::Hidden => None,
        Door::Dimmed(_) | Door::Shown => Some(say.get("Record video message").to_string()),
    }
}

/// The video button's tooltip, in the reader's language.
pub fn door_tooltip(say: &dyn StringCatalog) -> String {
    say.get("Record a video message").to_string()
}

/// The pointer behind a press, from the press's `gdk::InputSource` — [`None`](Self::None) for the
/// keyboard and an assistive technology's activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotDevice {
    /// No pointer: Enter or Space on the focused slot, a screen reader's activate action.
    None,
    /// `InputSource::Mouse`, `Touchpad`, `Trackpoint`.
    Mouse,
    /// `InputSource::Touchscreen`.
    Touch,
    /// `InputSource::Pen` (a stylus tip or eraser).
    Pen,
}

/// Which press reached the slot (S8.6), as GTK raises it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotPress {
    /// The button's `clicked`: a primary press of any length with any pointer, released inside
    /// (`gtk::GestureClick` on `GDK_BUTTON_PRIMARY`), Enter or Space, an accessible activation.
    Click,
    /// A secondary click: a `gtk::GestureClick` set to `GDK_BUTTON_SECONDARY` — a mouse's right
    /// button, and a pen's barrel button, which GDK reports as a secondary (or middle) press of the
    /// stylus. (Windows' `RightTapped`.)
    SecondaryClick,
    /// Shift+F10 or the Menu key on the focused slot (a `gtk::ShortcutController` the shell adds).
    ContextKey,
    /// A context request at a pointer from any other gesture — GTK raises none unless a
    /// `gtk::GestureLongPress` is attached, which the slot never has; one that arrives is ignored.
    ContextAtPointer,
}

/// What a press on the slot does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotResponse {
    /// The slot's own activation (S1.3's last column).
    Activate,
    /// The microphone's secondary menu (S1.6).
    Menu,
    /// Nothing.
    Ignore,
}

/// Which picture the slot shows — the shell maps it to an icon ([`icon_name`](Self::icon_name)).
/// Windows' Segoe Fluent glyphs were Send E724, the microphone E720, Stop E71A, Save E73E.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotIcon {
    Send,
    Microphone,
    Stop,
    Save,
}

impl SlotIcon {
    /// The freedesktop icon-naming name of its symbolic icon, which Adwaita and Yaru draw; the shell
    /// may bundle its own under the same names.
    pub fn icon_name(self) -> &'static str {
        match self {
            SlotIcon::Send => "mail-send-symbolic",
            SlotIcon::Microphone => "audio-input-microphone-symbolic",
            SlotIcon::Stop => "media-playback-stop-symbolic",
            SlotIcon::Save => "document-save-symbolic",
        }
    }
}

/// How the slot is drawn and named in one state.
///
/// `name` is its accessible label — S6's label; `tooltip` what a pointer resting on it is told;
/// `help_text` its accessible description (`gtk::accessible::Property::Description`): a dimmed
/// microphone's reason (S6), empty otherwise; `enabled` whether the button takes activation at all
/// (`set_sensitive`) — false only where S1.3 says "disabled"; `looks_dimmed` drawn in the disabled
/// colours although it is enabled (rows 7–9: dimmed is not disabled — a `.dim-label`-like style
/// class, never `set_sensitive(false)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotFace {
    pub icon: SlotIcon,
    pub name: String,
    pub tooltip: String,
    pub help_text: String,
    pub enabled: bool,
    pub looks_dimmed: bool,
}

/// Whether the slot's tooltip may open while a press of `pressing` is down (2026-10-06): never under
/// a finger or a pen — a callout on a long press, which no client shows on the microphone any more —
/// and as ever under a mouse, whose tooltip is a hover's, and for a key or a screen reader. (The
/// shell clears `has-tooltip` while such a press is down.)
pub fn tooltip_during(pressing: SlotDevice) -> bool {
    !matches!(pressing, SlotDevice::Touch | SlotDevice::Pen)
}

/// What a press on the slot does (S8.6). A click — any pointer, any length, the keyboard, a screen
/// reader — activates, except a pen tap made with the barrel button down, which is the pen's
/// secondary click. The menu is the MICROPHONE's (rows 7–10): a mouse's secondary click, that barrel
/// tap, Shift+F10 or the Menu key. Any other secondary click — a finger's, a pen's without its
/// barrel — and a context request at a pointer are ignored, so a long press simply clicks.
///
/// `barrel_at_press`: the pen's barrel button was down when the press began (the press event's
/// `ModifierType::BUTTON2_MASK | BUTTON3_MASK` as the tip went down). `microphone`: the slot is a
/// microphone (rows 7–10).
pub fn respond(
    press: SlotPress,
    device: SlotDevice,
    barrel_at_press: bool,
    microphone: bool,
) -> SlotResponse {
    let barrel = device == SlotDevice::Pen && barrel_at_press;
    let wants_menu = match press {
        SlotPress::Click => barrel,
        SlotPress::SecondaryClick => device == SlotDevice::Mouse || barrel,
        SlotPress::ContextKey => true,
        SlotPress::ContextAtPointer => false,
    };
    if !wants_menu {
        return if press == SlotPress::Click {
            SlotResponse::Activate
        } else {
            SlotResponse::Ignore
        };
    }
    if microphone {
        SlotResponse::Menu
    } else {
        SlotResponse::Ignore
    }
}

/// The slot as drawn in one state (S8.6): Send, the microphone, Stop, Save — each with its name and
/// tooltip. A dimmed microphone stays enabled and gives its reason as its description; only row 6
/// (and Save with nothing to save, and Send while a send is being written down) is disabled.
///
/// `send_held`: a send's files are being written down or a place is being found — today's Send
/// waits for them.
pub fn face(slot: Slot, send_held: bool, say: &dyn StringCatalog) -> SlotFace {
    let drawn = |icon, name: &str, tooltip: &str, help: &str, enabled, looks_dimmed| SlotFace {
        icon,
        name: name.to_string(),
        tooltip: tooltip.to_string(),
        help_text: help.to_string(),
        enabled,
        looks_dimmed,
    };
    let record = say.format("Record a voice message (%@)", &[&RECORD_SHORTCUT]);
    match slot {
        Slot::SendVoice => {
            let name = say.get("Send voice message");
            drawn(SlotIcon::Send, name, name, "", true, false)
        }
        Slot::StopRecording => {
            let name = say.get("Stop recording");
            drawn(SlotIcon::Stop, name, name, "", true, false)
        }
        Slot::Save { enabled } => {
            let name = say.get("Save");
            drawn(SlotIcon::Save, name, name, "", enabled, false)
        }
        Slot::Send => {
            let name = say.get("Send");
            drawn(SlotIcon::Send, name, name, "", !send_held, false)
        }
        Slot::Dimmed(reason) => {
            let help = notice(Some(reason), say).unwrap_or_default();
            drawn(
                SlotIcon::Microphone,
                say.get("Record voice message"),
                &record,
                &help,
                true,
                true,
            )
        }
        Slot::Microphone => drawn(
            SlotIcon::Microphone,
            say.get("Record voice message"),
            &record,
            "",
            true,
            false,
        ),
        Slot::Recorder | Slot::SendDisabled => {
            // Row 6 — and row 1, whose recorder draws its own controls: today's Send, disabled.
            let name = say.get("Send");
            drawn(SlotIcon::Send, name, name, "", false, false)
        }
    }
}

/// One press on the slot: its pointer (none for a key), a pen's barrel button as it went down, and
/// whether the guard ran then.
#[derive(Debug, Clone, Copy)]
struct Press {
    id: i64,
    device: SlotDevice,
    barrel: bool,
    began_guarded: bool,
}

/// The slot's activation guard (S1.1) and the press reaching the slot (S8.6).
///
/// For 600 ms after the slot's OWN activation changed it — a send that emptied the composer, a click
/// that started a recording, a Send that ended one or found it too short, a Stop in row 3 — the slot
/// ignores activation, and a press that went down while the guard ran is ignored WHOLE, however late
/// it lifts and however often the guard is armed again before it does. A double click on Send cannot
/// start a recording; a double click on the microphone cannot send one. Typing, pasting and staging
/// never arm it, so "ok" followed at once by Send still sends. Any change the PERSON makes to the
/// composer — a character typed or deleted, a paste, a suggestion taken, an item staged or taken off
/// the strip ([`changed`](Self::changed), [`text_changed`](Self::text_changed)) — LIFTS it, for the
/// microphone and for Send alike, exactly as `fc_text::record`'s `OtherAction` does on every other
/// client. A Send or a Save after the words have changed since it was armed is never guarded either,
/// even before the field has said so.
///
/// ENTER IN THE FIELD IS THE SLOT'S ACTIVATION in rows 2 to 5 (S1.3; `fc_text::record`'s `guarded`),
/// so it waits out the same guard: a second Enter after the slot's Stop in row 3 must not send the
/// words the note was staged beside. A KEY HELD DOWN IS ONE PRESS: its first may have been the
/// slot's, or the recording row's, and handed focus to the field (S2.4) — its repeats there never
/// act ([`ignores_field_key`](Self::ignores_field_key); GTK's `key-pressed` repeats).
///
/// A PRESS IS USED ONCE, AND ENDS. The press under way — a pointer's from its going down (the
/// `GestureClick`'s `pressed`), Enter's or Space's on the focused slot — is what a click reads its
/// pointer, a pen's barrel button and the guard from; the click uses it up, and its end (`released`,
/// `cancel`, `stopped`, the key up, focus gone) forgets it, so an activation no press began — an
/// accessible activation — reads none of a press that came before it.
///
/// Times are milliseconds on one monotonic clock (`glib::monotonic_time() / 1000` in the shell).
#[derive(Debug, Clone)]
pub struct SlotGuard {
    until: i64,
    armed_over: Option<String>,
    changed_since_armed: bool,
    live: Option<Press>,
    last_pointer: Option<Press>,
    last_id: i64,
    menu_for: i64,
}

impl Default for SlotGuard {
    fn default() -> Self {
        SlotGuard {
            until: i64::MIN,
            armed_over: None,
            changed_since_armed: false,
            live: None,
            last_pointer: None,
            last_id: 0,
            menu_for: -1,
        }
    }
}

impl SlotGuard {
    pub fn new() -> Self {
        SlotGuard::default()
    }

    /// The slot's own activation changed it, now, leaving `draft` in the field. Armed again while it
    /// runs, it runs on to 600 ms after the later activation — never shorter.
    pub fn arm(&mut self, now_ms: i64, draft: &str) {
        self.until = self
            .until
            .max(now_ms.saturating_add(ACTIVATION_GUARD_MS as i64));
        self.armed_over = Some(draft.to_string());
        self.changed_since_armed = false;
    }

    /// The person changed the composer — a picture pasted, dropped or picked landed in the strip, an
    /// item was taken off it with its ✕, or the words changed ([`text_changed`](Self::text_changed)):
    /// `fc_text::record`'s `OtherAction`. A change the person made is never guarded (S1.1), so the
    /// guard is lifted — the microphone records at the next click, and a Send or a Save goes, even
    /// one whose press went down while the guard still ran (its Send is the person's). A press on
    /// the microphone that went down under the guard stays ignored whole. While a recording runs
    /// (`recording`) nothing in the box is the person's to change, and the guard that keeps a double
    /// click from sending stays. The slot's own staging — the note a Stop in row 3 stages — never
    /// tells it.
    pub fn changed(&mut self, recording: bool) {
        if recording {
            return;
        }
        self.until = i64::MIN;
        self.changed_since_armed = true;
    }

    /// The field's words are now `draft`. Words that differ from those the guard was armed over are
    /// the person's change ([`changed`](Self::changed)) — typed, deleted, pasted or a suggestion
    /// taken: the box's own changes are made before the slot arms the guard over what they leave (a
    /// send's clear), so they never differ from it.
    pub fn text_changed(&mut self, draft: &str, recording: bool) {
        if self
            .armed_over
            .as_deref()
            .is_some_and(|armed| armed != draft)
        {
            self.changed(recording);
        }
    }

    /// Whether the guard is running now.
    pub fn running(&self, now_ms: i64) -> bool {
        now_ms < self.until
    }

    /// A pointer goes down on the slot, now — a pen with its barrel button held, or not. Answers the
    /// press, for its end.
    pub fn pointer_down(&mut self, now_ms: i64, device: SlotDevice, barrel: bool) -> i64 {
        self.last_id += 1;
        let press = Press {
            id: self.last_id,
            device,
            barrel: device == SlotDevice::Pen && barrel,
            began_guarded: self.running(now_ms),
        };
        self.live = Some(press);
        self.last_pointer = Some(press);
        press.id
    }

    /// Enter or Space goes down on the focused slot, now — not a repeat, which is the same press.
    /// Answers the press, for its end.
    pub fn key_down(&mut self, now_ms: i64) -> i64 {
        self.last_id += 1;
        let press = Press {
            id: self.last_id,
            device: SlotDevice::None,
            barrel: false,
            began_guarded: self.running(now_ms),
        };
        self.live = Some(press);
        press.id
    }

    /// A press ended — lifted, cancelled, its grab lost, its key up, focus gone — after whatever
    /// click it made.
    pub fn press_ended(&mut self, press: i64) {
        if self.live.is_some_and(|live| live.id == press) {
            self.live = None;
        }
    }

    /// The slot's click (S8.6): the press under way is used up, and what the click does is answered
    /// — the slot's activation; the microphone's menu, for a pen tap made with the barrel button
    /// down (once, though it may also arrive as a secondary click); or nothing, because the guard
    /// runs, the press went down while it ran, or the barrel's menu is open.
    ///
    /// `slot`: what the slot is as the click arrives. `draft`: the words in the field now — a Send or
    /// a Save after they changed, or something was staged, since the guard was armed is not
    /// guarded.
    pub fn click(&mut self, now_ms: i64, slot: Slot, draft: &str) -> SlotResponse {
        let press = self.live.take();
        let response = respond(
            SlotPress::Click,
            press.map_or(SlotDevice::None, |press| press.device),
            press.is_some_and(|press| press.barrel),
            slot.is_microphone(),
        );
        match response {
            SlotResponse::Menu => self.menu_once(press),
            SlotResponse::Activate
                if self.ignores(
                    press.is_some_and(|press| press.began_guarded),
                    now_ms,
                    matches!(slot, Slot::Send | Slot::Save { .. }),
                    draft,
                ) =>
            {
                SlotResponse::Ignore
            }
            _ => response,
        }
    }

    /// A secondary click on the slot (S8.6), which follows the press that made it: a mouse's, or
    /// that press's barrel tap, opens the microphone's menu — the barrel's once — and anything else,
    /// a finger's or a pen's without its barrel above all, nothing. (Windows' `RightTapped`.)
    pub fn secondary_click(&mut self, device: SlotDevice, microphone: bool) -> SlotResponse {
        let press = self.last_pointer;
        let response = respond(
            SlotPress::SecondaryClick,
            device,
            press.is_some_and(|press| press.barrel),
            microphone,
        );
        if response == SlotResponse::Menu && device == SlotDevice::Pen {
            self.menu_once(press)
        } else {
            response
        }
    }

    /// Whether a key in the composer's field is ignored. A repeat always is: a key held down is one
    /// press, whose first went wherever focus was. Enter — a send or a save (`sends`), the slot's
    /// activation in rows 2 to 5 — also waits out the guard, unless the words changed, or something
    /// was staged, since it was armed; Esc and the rest are not the slot's, and only repeat.
    pub fn ignores_field_key(&self, now_ms: i64, repeat: bool, sends: bool, draft: &str) -> bool {
        repeat || (sends && self.ignores(false, now_ms, sends, draft))
    }

    fn ignores(&self, began_guarded: bool, now_ms: i64, sends: bool, draft: &str) -> bool {
        let persons_change = sends
            && self
                .armed_over
                .as_deref()
                .is_some_and(|armed| self.changed_since_armed || armed != draft);
        !persons_change && (began_guarded || self.running(now_ms))
    }

    /// A barrel tap's menu opens once for its press, which may reach the slot as its click and as a
    /// secondary click both.
    fn menu_once(&mut self, press: Option<Press>) -> SlotResponse {
        if let Some(barrel) = press.filter(|press| press.barrel) {
            if self.menu_for == barrel.id {
                return SlotResponse::Ignore;
            }
            self.menu_for = barrel.id;
        }
        SlotResponse::Menu
    }
}

/// Ctrl+Shift+R held down (S1.6, S8.6). A held key auto-repeats, and GTK delivers each repeat to
/// the shortcut as it does to `key-pressed`, so a held chord would start a recording and then stop
/// it, again and again. A key held down is ONE press: the shell notes each of the chord's key-downs
/// in a capture-phase `gtk::EventControllerKey` (before the shortcut runs) and swallows the repeats
/// there; an invocation that still follows a repeat is ignored here.
#[derive(Debug, Clone, Default)]
pub struct ShortcutPresses {
    seen_at: i64,
    seen_repeat: bool,
}

impl ShortcutPresses {
    /// An invocation this soon after a key-down is that key-down's: the same input, give or take a
    /// tick of the clock.
    pub const SAME_KEY_MS: i64 = 50;

    pub fn new() -> Self {
        ShortcutPresses::default()
    }

    /// The chord's key went down, now — a repeat or not. Answers whether to swallow it: a repeat
    /// is.
    pub fn key_down(&mut self, now_ms: i64, repeat: bool) -> bool {
        self.seen_at = now_ms;
        self.seen_repeat = repeat;
        repeat
    }

    /// Whether the shortcut, invoked now, is a fresh press — anything but the repeat just seen.
    pub fn fresh(&self, now_ms: i64) -> bool {
        !(self.seen_repeat
            && now_ms >= self.seen_at
            && now_ms
                .checked_sub(self.seen_at)
                .is_some_and(|since| since <= Self::SAME_KEY_MS))
    }
}
