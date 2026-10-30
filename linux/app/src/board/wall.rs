//! The wall (§11): a `gtk::Fixed` the size of the WALL (as wide as the window, `WALL_SCREENS`
//! windows tall — so a family can pin something without taking something down) in a vertical
//! `gtk::ScrolledWindow`, each note a focusable card placed at its fraction's corner, tilted by its
//! id, drawn in a stable order with the most recently changed on top (`wall_text::layers`).
//!
//! **Moving is anyone's; rewriting and deleting are the author's.** One `gtk::GestureDrag` on the
//! wall (not on the card, whose coordinates move with it) picks up the card under the pointer and
//! drives its `NoteHand`: the card follows the pointer, clamped inside the wall, and the fraction
//! read back from where it is DRAWN is sent once, on release — one PATCH carrying only x/y. The
//! card is held where it was dropped until the move is answered, so it never snaps back for a
//! frame; moves of one note queue, the latest waiting. The arrow keys move a focused card one step
//! (D21, the keyboard's way to do what the drag does); a press that never travelled opens the note
//! (its author's sheet) or reveals a hidden one.
//!
//! **A note's menu** (secondary click, a long press on a touchscreen, Shift+F10, the Menu key): Reveal (a hidden note — the only
//! item it offers); Edit…, Size and Delete for its author ("Written by someone else" otherwise);
//! the list's lines to tick (anyone); for an event who is coming, Add to Calendar and "Are you
//! coming?" (anyone).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use adw::prelude::*;
use fc_core::board::board_picture;
use fc_core::protocol::{NoteDto, NotePatch};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::board::{BoardModel, Sticker};
use fc_logic::event_text;
use fc_logic::note_draft::note_sheet_text;
use fc_logic::note_hand::{HandResult, Letting, NoteHand};
use fc_logic::wall_text;
use fc_text::board::{Answer, Kind, Size, going_line, wall_task_lines};

use crate::services::log;
use crate::settings::ui;
use crate::shell::Shell;

/// How far one arrow press moves a focused card, in wall pixels (Shift: five steps).
pub const STEP: f64 = 8.0;
/// The mouse's pointer id for `NoteHand`.
const MOUSE: i64 = 0;

/// One card on the wall.
struct Placed {
    sticker: Sticker,
    card: gtk::Box,
    hidden: bool,
}

/// The wall (see the module's comment).
pub struct Wall {
    pub scroller: gtk::ScrolledWindow,
    pub fixed: gtk::Fixed,
    /// "The board is empty" over the wall while it is.
    pub empty: adw::StatusPage,
    pub root: gtk::Overlay,
    placed: RefCell<Vec<Placed>>,
    hands: RefCell<HashMap<i64, NoteHand>>,
    revealed: RefCell<HashSet<i64>>,
    /// The wall's own size now.
    size: Cell<(f64, f64)>,
    /// The card in the mouse's hand.
    in_hand: Cell<Option<i64>>,
    /// The card the keyboard focus is on (kept across redraws).
    focused: Cell<Option<i64>>,
    /// A redraw asked for while a card was in hand: done on the drop.
    stale: Cell<bool>,
    /// Where a failure is said.
    toasts: adw::ToastOverlay,
    /// Opens the author's sheet for a note.
    on_edit: RefCell<Option<Rc<dyn Fn(NoteDto)>>>,
    /// Events whose backdrop is being drawn: one request at a time per note — each costs the
    /// family a picture, and a second answer would only replace the first.
    drawing: RefCell<HashSet<i64>>,
}

impl std::fmt::Debug for Wall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wall")
            .field("notes", &self.placed.borrow().len())
            .finish_non_exhaustive()
    }
}

impl Wall {
    pub fn new(toasts: &adw::ToastOverlay) -> Rc<Wall> {
        let shell = Shell::get();
        let say = &shell.say;
        let fixed = gtk::Fixed::new();
        fixed.add_css_class("fc-board-ground");
        fixed.set_overflow(gtk::Overflow::Hidden);
        // External, not Never: the cards must never hold the window wide — a narrower window lays
        // the wall out again at its own width.
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vexpand(true)
            .hexpand(true)
            .child(&fixed)
            .build();
        scroller.add_css_class("fc-board-ground");
        let empty = adw::StatusPage::builder()
            .icon_name("view-grid-symbolic")
            .title(say.get("The board is empty"))
            .description(say.get("Add a note — everyone in the family sees it."))
            .can_target(false)
            .build();
        let root = gtk::Overlay::new();
        root.set_child(Some(&scroller));
        root.add_overlay(&empty);
        let wall = Rc::new(Wall {
            scroller,
            fixed,
            empty,
            root,
            placed: RefCell::new(Vec::new()),
            hands: RefCell::new(HashMap::new()),
            revealed: RefCell::new(HashSet::new()),
            drawing: RefCell::new(HashSet::new()),
            size: Cell::new((0.0, 0.0)),
            in_hand: Cell::new(None),
            focused: Cell::new(None),
            stale: Cell::new(false),
            toasts: toasts.clone(),
            on_edit: RefCell::new(None),
        });
        wall.listen();
        wall
    }

    /// What opens the author's sheet (the window's).
    pub fn set_on_edit(&self, edit: impl Fn(NoteDto) + 'static) {
        *self.on_edit.borrow_mut() = Some(Rc::new(edit));
    }

    /// The wall's size now, in pixels.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the self-test reads it")
    )]
    pub fn wall_size(&self) -> (f64, f64) {
        self.size.get()
    }

    /// The note ids drawn, with each card's top-left corner on the wall (what the self-test reads).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the self-test reads it")
    )]
    pub fn cards(&self) -> Vec<(i64, gtk::Box, (f64, f64))> {
        self.placed
            .borrow()
            .iter()
            .map(|placed| {
                let id = placed.sticker.note.id;
                let at = self.corner_of(&placed.sticker);
                (id, placed.card.clone(), at)
            })
            .collect()
    }

    fn listen(self: &Rc<Self>) {
        // The window's size decides the wall's.
        for adjustment in [self.scroller.hadjustment(), self.scroller.vadjustment()] {
            let weak = Rc::downgrade(self);
            adjustment.connect_page_size_notify(move |_| {
                if let Some(wall) = weak.upgrade() {
                    wall.redraw();
                }
            });
        }

        let drag = gtk::GestureDrag::new();
        drag.set_button(gtk::gdk::BUTTON_PRIMARY);
        {
            let weak = Rc::downgrade(self);
            drag.connect_drag_begin(move |gesture, x, y| {
                let Some(wall) = weak.upgrade() else {
                    return;
                };
                let Some(id) = wall.note_at(x, y) else {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                };
                let Some((fraction, card)) = wall.geometry(id) else {
                    return;
                };
                wall.in_hand.set(Some(id));
                wall.hands.borrow_mut().entry(id).or_default().down(
                    MOUSE,
                    (x, y),
                    fraction,
                    card,
                    wall.size.get(),
                );
            });
        }
        {
            let weak = Rc::downgrade(self);
            drag.connect_drag_update(move |gesture, dx, dy| {
                let Some(wall) = weak.upgrade() else {
                    return;
                };
                let Some(id) = wall.in_hand.get() else {
                    return;
                };
                let Some((x, y)) = gesture.start_point() else {
                    return;
                };
                let moved = wall
                    .hands
                    .borrow_mut()
                    .entry(id)
                    .or_default()
                    .move_to(MOUSE, (x + dx, y + dy));
                if moved {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    if let Some(card) = wall.card_of(id) {
                        card.set_cursor_from_name(Some("grabbing"));
                    }
                    wall.follow_hand(id, true);
                }
            });
        }
        {
            let weak = Rc::downgrade(self);
            drag.connect_drag_end(move |_, _, _| {
                let Some(wall) = weak.upgrade() else {
                    return;
                };
                let Some(id) = wall.in_hand.take() else {
                    return;
                };
                if let Some(card) = wall.card_of(id) {
                    card.set_cursor_from_name(Some("grab"));
                }
                let Some((fraction, card)) = wall.geometry(id) else {
                    return;
                };
                let letting = wall.hands.borrow_mut().entry(id).or_default().up(
                    MOUSE,
                    fraction,
                    card,
                    wall.size.get(),
                );
                wall.let_go(id, letting);
            });
        }
        {
            let weak = Rc::downgrade(self);
            drag.connect_cancel(move |_, _| {
                if let Some(wall) = weak.upgrade()
                    && let Some(id) = wall.in_hand.take()
                {
                    wall.hands.borrow_mut().entry(id).or_default().cancel(MOUSE);
                    wall.follow_hand(id, false);
                }
            });
        }
        self.fixed.add_controller(drag);

        let menu = gtk::GestureClick::new();
        menu.set_button(gtk::gdk::BUTTON_SECONDARY);
        {
            let weak = Rc::downgrade(self);
            menu.connect_pressed(move |gesture, _, x, y| {
                if let Some(wall) = weak.upgrade()
                    && let Some(id) = wall.note_at(x, y)
                {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    wall.show_menu(id, x, y);
                }
            });
        }
        self.fixed.add_controller(menu);

        // A long press opens the menu on a touchscreen (a mouse has its secondary button).
        let hold = gtk::GestureLongPress::new();
        hold.set_touch_only(true);
        {
            let weak = Rc::downgrade(self);
            hold.connect_pressed(move |gesture, x, y| {
                if let Some(wall) = weak.upgrade()
                    && let Some(id) = wall.note_at(x, y)
                {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    wall.show_menu(id, x, y);
                }
            });
        }
        self.fixed.add_controller(hold);
    }

    /// Reads the notes and draws them all again for the wall's size now. Held while a card is in
    /// the mouse's hand (done on its drop).
    pub fn redraw(self: &Rc<Self>) {
        if self.in_hand.get().is_some() {
            self.stale.set(true);
            return;
        }
        self.stale.set(false);
        let shell = Shell::get();
        let Some(connection) = shell.connection() else {
            return;
        };
        let visible = (
            f64::from(self.scroller.width()),
            f64::from(self.scroller.height()),
        );
        if visible.0 <= 0.0 || visible.1 <= 0.0 {
            return;
        }
        let wall = BoardModel::wall_size(visible.0, visible.1);
        self.size.set(wall);
        // Taller than the window, so it scrolls; as wide as the window by the scroller's own
        // allocation (a width request would hold the window wide).
        self.fixed.set_size_request(-1, wall.1.ceil() as i32);
        let model = super::model(&connection);
        let stickers = match model.stickers(visible.0, visible.1) {
            Ok(stickers) => stickers,
            Err(error) => {
                crate::fc_warn!("board: the cache refused ({})", log::store_error(&error));
                return;
            }
        };
        let blocked: Vec<i64> = connection
            .chats
            .blocked()
            .map(|ids| ids.to_vec())
            .unwrap_or_default();
        let me = connection.chats.reader();
        let layers = wall_text::layers(&stickers);
        let mut ordered = stickers;
        // A stable order (by id) with recency as the z-index: drawn lowest first.
        ordered.sort_by_key(|sticker| {
            (
                layers.get(&sticker.note.id).copied().unwrap_or(0),
                sticker.note.id,
            )
        });

        for placed in self.placed.borrow_mut().drain(..) {
            self.fixed.remove(&placed.card);
        }
        let revealed = self.revealed.borrow().clone();
        let mut placed = Vec::new();
        for sticker in ordered {
            let hidden =
                wall_text::is_hidden(&sticker.note, me, |id| blocked.contains(&id), &revealed);
            let card = self.card(&connection, &sticker, hidden, me);
            self.fixed.put(&card, 0.0, 0.0);
            placed.push(Placed {
                sticker,
                card,
                hidden,
            });
        }
        self.empty.set_visible(placed.is_empty());
        // Hands whose notes are gone go with them.
        let ids: HashSet<i64> = placed.iter().map(|p| p.sticker.note.id).collect();
        self.hands.borrow_mut().retain(|id, _| ids.contains(id));
        *self.placed.borrow_mut() = placed;
        for id in ids {
            self.follow_hand(id, false);
        }
        if let Some(id) = self.focused.get()
            && let Some(card) = self.card_of(id)
        {
            card.grab_focus();
        }
    }

    /// The card of note `id`, when drawn.
    pub fn card_of(&self, id: i64) -> Option<gtk::Box> {
        self.placed
            .borrow()
            .iter()
            .find(|placed| placed.sticker.note.id == id)
            .map(|placed| placed.card.clone())
    }

    /// Where a card is drawn: its hand's answer (in hand, dropped and unanswered, or the server's).
    fn corner_of(&self, sticker: &Sticker) -> (f64, f64) {
        let fraction = (sticker.note.x, sticker.note.y);
        let card = (sticker.width, sticker.height);
        match self.hands.borrow().get(&sticker.note.id) {
            Some(hand) => hand.corner(fraction, card, self.size.get()),
            None => (sticker.x, sticker.y),
        }
    }

    /// A note's stored fraction and its card's size.
    fn geometry(&self, id: i64) -> Option<((f64, f64), (f64, f64))> {
        self.placed
            .borrow()
            .iter()
            .find(|placed| placed.sticker.note.id == id)
            .map(|placed| {
                (
                    (placed.sticker.note.x, placed.sticker.note.y),
                    (placed.sticker.width, placed.sticker.height),
                )
            })
    }

    /// Moves the card of `id` to where its hand draws it, tilted by its id; `lifted` raises it over
    /// the wall with a deeper shadow while it travels.
    fn follow_hand(&self, id: i64, lifted: bool) {
        let placed = self.placed.borrow();
        let Some(placed) = placed.iter().find(|placed| placed.sticker.note.id == id) else {
            return;
        };
        let (x, y) = self.corner_of(&placed.sticker);
        let (w, h) = (placed.sticker.width as f32, placed.sticker.height as f32);
        let tilt = placed.sticker.tilt_degrees as f32;
        let transform = gtk::gsk::Transform::new()
            .translate(&gtk::graphene::Point::new(
                x as f32 + (w / 2.0),
                y as f32 + (h / 2.0),
            ))
            .rotate(if lifted { 0.0 } else { tilt })
            .translate(&gtk::graphene::Point::new(-(w / 2.0), -(h / 2.0)));
        self.fixed
            .set_child_transform(&placed.card, Some(&transform));
        if lifted {
            placed.card.add_css_class("fc-note-lifted");
            // Last child is drawn last: over every other card.
            if placed.card.next_sibling().is_some() {
                placed.card.insert_before(&self.fixed, None::<&gtk::Widget>);
            }
        } else {
            placed.card.remove_css_class("fc-note-lifted");
        }
    }

    /// The note under a point of the wall.
    fn note_at(&self, x: f64, y: f64) -> Option<i64> {
        let mut widget = self.fixed.pick(x, y, gtk::PickFlags::DEFAULT)?;
        loop {
            if let Some(id) = widget
                .widget_name()
                .strip_prefix("note-")
                .and_then(|id| id.parse::<i64>().ok())
            {
                return Some(id);
            }
            widget = widget.parent()?;
            if widget == self.fixed.clone().upcast::<gtk::Widget>() {
                return None;
            }
        }
    }

    /// What letting go meant: open (or reveal) on a click, send the move on a drop.
    fn let_go(self: &Rc<Self>, id: i64, letting: Letting) {
        match letting.result {
            HandResult::None => {}
            HandResult::Click => self.click(id),
            HandResult::Drop => {
                self.follow_hand(id, false);
                if letting.send_now
                    && let Some(target) = letting.target
                {
                    self.send_move(id, target);
                }
            }
        }
        if self.stale.get() {
            self.redraw();
        }
    }

    /// A press that did not travel: a hidden note is revealed (a peek, per note, never stored);
    /// the author's note opens in the sheet.
    pub fn click(self: &Rc<Self>, id: i64) {
        let (hidden, mine, note) = {
            let placed = self.placed.borrow();
            let Some(placed) = placed.iter().find(|placed| placed.sticker.note.id == id) else {
                return;
            };
            (
                placed.hidden,
                placed.sticker.mine,
                placed.sticker.note.clone(),
            )
        };
        if hidden {
            self.revealed.borrow_mut().insert(id);
            self.redraw();
        } else if mine && let Some(edit) = self.on_edit.borrow().clone() {
            edit(note);
        }
    }

    /// One PATCH carrying only x/y; the move that waited behind it goes when it is answered.
    fn send_move(self: &Rc<Self>, id: i64, target: (f64, f64)) {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        let weak = Rc::downgrade(self);
        ui::spawn(async move {
            let patch = NotePatch {
                x: Some(target.0),
                y: Some(target.1),
                ..NotePatch::default()
            };
            let answer = super::model(&connection).patch(id, &patch).await;
            let Some(wall) = weak.upgrade() else {
                return;
            };
            wall.said(answer);
            let next = wall.hands.borrow_mut().entry(id).or_default().answered();
            match next {
                Some(next) => wall.send_move(id, next),
                None => wall.redraw(),
            }
        });
    }

    /// A write's answer: a refusal is SAID, never swallowed.
    fn said(&self, answer: fc_core::store::StoreResult<Option<fc_core::protocol::ApiError>>) {
        match answer {
            Ok(None) => {}
            Ok(Some(error)) => {
                crate::fc_info!("board: refused ({})", error.code);
                let shell = Shell::get();
                ui::toast(
                    &self.toasts,
                    &note_sheet_text::failure(&error, shell.say.as_ref()),
                );
            }
            Err(error) => {
                crate::fc_warn!("board: the cache refused ({})", log::store_error(&error))
            }
        }
    }

    /// One card: its look, its words ([`draw_card`]), and its keys.
    fn card(
        self: &Rc<Self>,
        connection: &Rc<crate::shell::Connection>,
        sticker: &Sticker,
        hidden: bool,
        me: i64,
    ) -> gtk::Box {
        let card = draw_card(connection, sticker, hidden, me);
        // Anyone may move a note: the hand says so (the Mac's open-hand cursor).
        card.set_cursor_from_name(Some("grab"));
        self.keys(&card, sticker.note.id);
        card
    }

    /// The card's keys: arrows move it a step (Shift: five), Enter or Space opens it, the Menu key
    /// and Shift+F10 its menu.
    fn keys(self: &Rc<Self>, card: &gtk::Box, id: i64) {
        let keys = gtk::EventControllerKey::new();
        {
            let weak = Rc::downgrade(self);
            keys.connect_key_pressed(move |_, key, _, modifiers| {
                let Some(wall) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let step = if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                    STEP * 5.0
                } else {
                    STEP
                };
                let delta = match key {
                    gtk::gdk::Key::Left => Some((-step, 0.0)),
                    gtk::gdk::Key::Right => Some((step, 0.0)),
                    gtk::gdk::Key::Up => Some((0.0, -step)),
                    gtk::gdk::Key::Down => Some((0.0, step)),
                    _ => None,
                };
                if let Some((dx, dy)) = delta {
                    wall.nudge(id, dx, dy);
                    return glib::Propagation::Stop;
                }
                let menu_key = key == gtk::gdk::Key::Menu
                    || (key == gtk::gdk::Key::F10
                        && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK));
                if menu_key {
                    if let Some(card) = wall.card_of(id) {
                        let (w, h) = (f64::from(card.width()), f64::from(card.height()));
                        if let Some(point) = card.compute_point(
                            &wall.fixed,
                            &gtk::graphene::Point::new((w / 2.0) as f32, (h / 2.0) as f32),
                        ) {
                            wall.show_menu(id, f64::from(point.x()), f64::from(point.y()));
                        }
                    }
                    return glib::Propagation::Stop;
                }
                if matches!(
                    key,
                    gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::space
                ) {
                    wall.click(id);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        {
            let weak = Rc::downgrade(self);
            keys.connect_key_released(move |_, key, _, _| {
                if matches!(
                    key,
                    gtk::gdk::Key::Left
                        | gtk::gdk::Key::Right
                        | gtk::gdk::Key::Up
                        | gtk::gdk::Key::Down
                ) && let Some(wall) = weak.upgrade()
                {
                    wall.put_down(id);
                }
            });
        }
        card.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        {
            let weak = Rc::downgrade(self);
            focus.connect_enter(move |_| {
                if let Some(wall) = weak.upgrade() {
                    wall.focused.set(Some(id));
                }
            });
        }
        {
            let weak = Rc::downgrade(self);
            focus.connect_leave(move |_| {
                if let Some(wall) = weak.upgrade() {
                    wall.put_down(id);
                    if wall.focused.get() == Some(id) {
                        wall.focused.set(None);
                    }
                }
            });
        }
        card.add_controller(focus);
    }

    /// An arrow: the focused card a step from where it is drawn, held inside the wall.
    pub fn nudge(self: &Rc<Self>, id: i64, dx: f64, dy: f64) {
        let Some((fraction, card)) = self.geometry(id) else {
            return;
        };
        self.hands.borrow_mut().entry(id).or_default().nudge(
            dx,
            dy,
            fraction,
            card,
            self.size.get(),
        );
        self.follow_hand(id, false);
    }

    /// The arrow let go of (or the focus gone mid-move): put down where it is drawn, and sent.
    pub fn put_down(self: &Rc<Self>, id: i64) {
        let Some((fraction, card)) = self.geometry(id) else {
            return;
        };
        let letting = self.hands.borrow_mut().entry(id).or_default().put_down(
            fraction,
            card,
            self.size.get(),
        );
        self.let_go(id, letting);
    }

    /// The note's menu at a point of the wall.
    pub fn show_menu(self: &Rc<Self>, id: i64, x: f64, y: f64) {
        let (note, mine, hidden, size) = {
            let placed = self.placed.borrow();
            let Some(placed) = placed.iter().find(|placed| placed.sticker.note.id == id) else {
                return;
            };
            (
                placed.sticker.note.clone(),
                placed.sticker.mine,
                placed.hidden,
                placed.sticker.size,
            )
        };
        let (menu, actions) = self.menu_for(&note, mine, hidden, size);
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.insert_action_group("note", Some(&actions));
        popover.set_parent(&self.fixed);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.set_has_arrow(true);
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
    }

    fn menu_for(
        self: &Rc<Self>,
        note: &NoteDto,
        mine: bool,
        hidden: bool,
        size: Size,
    ) -> (gio::Menu, gio::SimpleActionGroup) {
        let shell = Shell::get();
        let say: &dyn StringCatalog = shell.say.as_ref();
        let menu = gio::Menu::new();
        let actions = gio::SimpleActionGroup::new();
        let id = note.id;
        let kind = Kind::from_name(note.kind.as_deref());
        let add = |name: &str, run: Box<dyn Fn()>| {
            let action = gio::SimpleAction::new(name, None);
            action.connect_activate(move |_, _| run());
            actions.add_action(&action);
        };
        if hidden {
            let weak = Rc::downgrade(self);
            add(
                "reveal",
                Box::new(move || {
                    if let Some(wall) = weak.upgrade() {
                        wall.revealed.borrow_mut().insert(id);
                        wall.redraw();
                    }
                }),
            );
            menu.append(Some(say.get("Reveal")), Some("note.reveal"));
            return (menu, actions);
        }
        let author = gio::Menu::new();
        if mine {
            let (weak, held) = (Rc::downgrade(self), note.clone());
            add(
                "edit",
                Box::new(move || {
                    if let Some(wall) = weak.upgrade()
                        && let Some(edit) = wall.on_edit.borrow().clone()
                    {
                        edit(held.clone());
                    }
                }),
            );
            author.append(Some(say.get("Edit…")), Some("note.edit"));
            // Each choice PATCHes the size alone — the author's field.
            let sizes = gio::Menu::new();
            let resize = gio::SimpleAction::new_stateful(
                "size",
                Some(glib::VariantTy::STRING),
                &size.name().to_variant(),
            );
            {
                let weak = Rc::downgrade(self);
                resize.connect_activate(move |action, parameter| {
                    let Some(name) = parameter.and_then(|p| p.get::<String>()) else {
                        return;
                    };
                    action.set_state(&name.to_variant());
                    if let Some(wall) = weak.upgrade() {
                        wall.write(
                            id,
                            NotePatch {
                                size: Some(name),
                                ..NotePatch::default()
                            },
                        );
                    }
                });
            }
            actions.add_action(&resize);
            for choice in Size::ALL {
                let item = gio::MenuItem::new(Some(choice.title()), None);
                item.set_action_and_target_value(
                    Some("note.size"),
                    Some(&choice.name().to_variant()),
                );
                sizes.append_item(&item);
            }
            author.append_submenu(Some(say.get("Size")), &sizes);
            let weak = Rc::downgrade(self);
            add(
                "delete",
                Box::new(move || {
                    if let Some(wall) = weak.upgrade() {
                        wall.ask_delete(id);
                    }
                }),
            );
            author.append(Some(say.get("Delete")), Some("note.delete"));
        } else {
            // Anyone may MOVE a note; only its author may change it.
            author.append(Some(say.get("Written by someone else")), Some("note.none"));
        }
        menu.append_section(None, &author);

        if kind == Kind::Tasks && !note.task_list().is_empty() {
            // TICKING IS NOT AUTHORSHIP: anyone may.
            let things = gio::Menu::new();
            for item in note.task_list() {
                let name = format!("tick-{}", item.id);
                let action = gio::SimpleAction::new_stateful(&name, None, &item.done.to_variant());
                let (weak, item_id) = (Rc::downgrade(self), item.id);
                action.connect_activate(move |action, _| {
                    let done = !action
                        .state()
                        .and_then(|s| s.get::<bool>())
                        .unwrap_or(false);
                    action.set_state(&done.to_variant());
                    if let Some(wall) = weak.upgrade() {
                        wall.tick(id, item_id, done);
                    }
                });
                actions.add_action(&action);
                things.append(Some(&item.text), Some(&format!("note.{name}")));
            }
            let section = gio::Menu::new();
            section.append_submenu(Some(say.get("Things to do")), &things);
            menu.append_section(None, &section);
        }
        if kind == Kind::Event {
            let section = gio::Menu::new();
            let guests = self.guests(note);
            if guests.is_empty() {
                section.append(Some(say.get("Nobody has answered yet.")), Some("note.none"));
            } else {
                for line in guests {
                    section.append(Some(&line), Some("note.none"));
                }
            }
            let (weak, held) = (Rc::downgrade(self), note.clone());
            add(
                "calendar",
                Box::new(move || {
                    if let Some(wall) = weak.upgrade() {
                        wall.add_to_calendar(&held);
                    }
                }),
            );
            section.append(Some(say.get("Add to Calendar")), Some("note.calendar"));
            // The picture behind it is the author's to ask for (the Mac's `BackdropDoor`), where
            // the server draws and names who does.
            let assistant = Shell::get()
                .connection()
                .and_then(|connection| connection.session.state().assistant);
            if mine && fc_logic::picture_hint::offers_backdrop(true, assistant.as_ref()) {
                let drawing = self.drawing.borrow().contains(&id);
                let weak = Rc::downgrade(self);
                add(
                    "backdrop",
                    Box::new(move || {
                        if let Some(wall) = weak.upgrade() {
                            wall.draw_backdrop(id);
                        }
                    }),
                );
                let label = if drawing {
                    say.get("Drawing…")
                } else if note.attachment.is_none() {
                    say.get("Draw a backdrop")
                } else {
                    say.get("Draw another backdrop")
                };
                section.append(
                    Some(label),
                    Some(if drawing {
                        "note.none"
                    } else {
                        "note.backdrop"
                    }),
                );
                // Drawn from the TITLE, and the provider's filter refuses most real names and
                // brands in it: the hint beside the control, as advice rather than an action.
                section.append(
                    Some(&fc_logic::picture_hint::sentence(say)),
                    Some("note.none"),
                );
            }
            // ANSWERING IS NOT AUTHORSHIP: anyone may say whether they are coming.
            let mine_now = note
                .my_answer(Shell::get().connection().map_or(0, |c| c.chats.reader()))
                .unwrap_or("")
                .to_string();
            let answer = gio::SimpleAction::new_stateful(
                "answer",
                Some(glib::VariantTy::STRING),
                &mine_now.to_variant(),
            );
            {
                let weak = Rc::downgrade(self);
                answer.connect_activate(move |action, parameter| {
                    let Some(name) = parameter.and_then(|p| p.get::<String>()) else {
                        return;
                    };
                    action.set_state(&name.to_variant());
                    if let Some(wall) = weak.upgrade() {
                        wall.answer(id, Answer::from_name(Some(&name)));
                    }
                });
            }
            actions.add_action(&answer);
            let coming = gio::Menu::new();
            for choice in Answer::ALL {
                let item = gio::MenuItem::new(Some(choice.title()), None);
                item.set_action_and_target_value(
                    Some("note.answer"),
                    Some(&choice.name().to_variant()),
                );
                coming.append_item(&item);
            }
            let none = gio::MenuItem::new(Some(say.get("No answer")), None);
            none.set_action_and_target_value(Some("note.answer"), Some(&"".to_variant()));
            coming.append_item(&none);
            section.append_submenu(Some(say.get("Are you coming?")), &coming);
            menu.append_section(None, &section);
        }
        (menu, actions)
    }

    /// Who is coming, grouped by answer and named — "Going: Ann, Bob".
    fn guests(&self, note: &NoteDto) -> Vec<String> {
        let Some(connection) = Shell::get().connection() else {
            return Vec::new();
        };
        Answer::ALL
            .iter()
            .filter_map(|choice| {
                let named: Vec<String> = note
                    .rsvps
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .filter(|rsvp| rsvp.answer == choice.name())
                    .filter_map(|rsvp| {
                        connection
                            .chats
                            .member(rsvp.user_id)
                            .ok()
                            .flatten()
                            .map(|member| member.display_name)
                    })
                    .filter(|name| !name.is_empty())
                    .collect();
                (!named.is_empty()).then(|| format!("{}: {}", choice.title(), named.join(", ")))
            })
            .collect()
    }

    /// Delete — asked first: one menu click used to take a note off the whole family's wall.
    fn ask_delete(self: &Rc<Self>, id: i64) {
        let say = Rc::clone(&Shell::get().say);
        let weak = Rc::downgrade(self);
        ui::ask(
            &self.fixed,
            say.get("Delete this note?"),
            "",
            say.get("Cancel"),
            say.get("Delete"),
            true,
            move || {
                let Some(connection) = Shell::get().connection() else {
                    return;
                };
                let weak = weak.clone();
                ui::spawn(async move {
                    let answer = super::model(&connection).delete(id).await;
                    if let Some(wall) = weak.upgrade() {
                        wall.said(answer);
                        wall.redraw();
                    }
                });
            },
        );
    }

    fn write(self: &Rc<Self>, id: i64, patch: NotePatch) {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        let weak = Rc::downgrade(self);
        ui::spawn(async move {
            let answer = super::model(&connection).patch(id, &patch).await;
            if let Some(wall) = weak.upgrade() {
                wall.said(answer);
                wall.redraw();
            }
        });
    }

    /// "Draw a backdrop": nothing reaches the model unasked — an author who has not agreed is asked
    /// first and it is drawn on a yes, and the server's own `assistant_consent_required` (a consent
    /// withdrawn on another device) raises the same question once instead of a failure
    /// (`backdrop_consent::run`). A refusal by the provider's filter says so; anything else is
    /// "Couldn't draw that."
    fn draw_backdrop(self: &Rc<Self>, id: i64) {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        if !self.drawing.borrow_mut().insert(id) {
            return;
        }
        let weak = Rc::downgrade(self);
        ui::spawn(async move {
            let session = Rc::clone(&connection.session);
            let asks_first = || {
                let state = session.state();
                fc_logic::backdrop_consent::asks_first(
                    state
                        .assistant
                        .as_ref()
                        .and_then(|assistant| assistant.processor.as_deref()),
                    state.assistant_consent_at.as_deref(),
                )
            };
            let (asking, anchor) = (Rc::clone(&connection), weak.clone());
            let ask = move || {
                let (connection, anchor) = (Rc::clone(&asking), anchor.upgrade());
                async move {
                    match anchor {
                        Some(wall) => {
                            crate::composer::consent::ask(&wall.root, &connection).await
                                == crate::composer::consent::Answer::Agreed
                        }
                        None => false,
                    }
                }
            };
            let drawing = Rc::clone(&connection);
            let draw = move || {
                let connection = Rc::clone(&drawing);
                async move {
                    match super::model(&connection).draw_backdrop(id).await {
                        Ok(Ok(Some(drawn))) => Ok(drawn),
                        Ok(Ok(None)) => Err(fc_core::protocol::ApiError::transport("no picture")),
                        Ok(Err(error)) => Err(error),
                        Err(error) => {
                            crate::fc_warn!(
                                "board: the cache refused ({})",
                                log::store_error(&error)
                            );
                            Err(fc_core::protocol::ApiError::transport("cache"))
                        }
                    }
                }
            };
            let outcome = fc_logic::backdrop_consent::run(asks_first, ask, draw).await;
            let Some(wall) = weak.upgrade() else {
                return;
            };
            wall.drawing.borrow_mut().remove(&id);
            match (&outcome.drawn, &outcome.error) {
                (Some(_), _) => crate::fc_info!("board: backdrop drawn for note {id}"),
                (None, Some(error)) => {
                    crate::fc_info!("board: backdrop refused ({})", error.code);
                    let say = Rc::clone(&Shell::get().say);
                    ui::toast(
                        &wall.toasts,
                        &note_sheet_text::backdrop_failure(error, say.as_ref()),
                    );
                }
                (None, None) => {}
            }
            wall.redraw();
        });
    }

    fn tick(self: &Rc<Self>, id: i64, item: i64, done: bool) {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        let weak = Rc::downgrade(self);
        ui::spawn(async move {
            let answer = super::model(&connection).tick(id, item, done).await;
            if let Some(wall) = weak.upgrade() {
                if let Ok(Some(error)) = &answer {
                    crate::fc_info!("board: tick refused ({})", error.code);
                    let say = Rc::clone(&Shell::get().say);
                    ui::toast(&wall.toasts, say.get("Couldn't tick that off."));
                } else {
                    wall.said(answer);
                }
                wall.redraw();
            }
        });
    }

    fn answer(self: &Rc<Self>, id: i64, answer: Option<Answer>) {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        let weak = Rc::downgrade(self);
        ui::spawn(async move {
            let model = super::model(&connection);
            let result = match answer {
                Some(answer) => model.answer(id, answer).await,
                None => model.retract(id).await,
            };
            if let Some(wall) = weak.upgrade() {
                wall.said(result);
                wall.redraw();
            }
        });
    }

    /// The event's `.ics`, written into the app's temporary folder and opened with the desktop's
    /// calendar (`gtk::FileLauncher`). Answers the file (the self-test reads it).
    pub fn add_to_calendar(&self, note: &NoteDto) -> Option<std::path::PathBuf> {
        let shell = Shell::get();
        let path = super::calendar::write(
            &shell.paths.tmp_dir(),
            note,
            time::OffsetDateTime::now_utc(),
        )?;
        crate::fc_info!("board: note {} written as a calendar file", note.id);
        let window = self.fixed.root().and_downcast::<gtk::Window>();
        gtk::FileLauncher::new(Some(&gio::File::for_path(&path))).launch(
            window.as_ref(),
            None::<&gio::Cancellable>,
            |answer| {
                if let Err(error) = answer {
                    crate::fc_info!("board: no calendar took the file ({})", error.message());
                }
            },
        );
        Some(path)
    }
}

/// A small line of a card.
fn line(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(1);
    label.add_css_class("fc-note-line");
    label
}

/// How many lines of words a card has room for (`fc_text::board::lines_that_fit`): the card's
/// height under its padding, at the size's type and a line height of 1.25 of it.
/// The words FIT the note (docs/protocol.md, "Board"; the Mac's `minimumScaleFactor`, the web's
/// `fitted_scale`): the largest type, from the note's own size down to `MIN_TEXT_SCALE`, at which
/// they fit the room the card leaves them — measured with Pango in the note's own face. Past the
/// floor they are cut, with an ellipsis, at the lines they hold there: the one case a reader has to
/// hover or open the note for.
fn fit_text(label: &gtk::Label, words: &str, sticker: &Sticker) {
    use fc_text::board::{FIT_STEPS, MIN_TEXT_SCALE, fitted_scale, lines_that_fit};
    let room = room_for(sticker);
    let width = (sticker.width - 20.0).max(1.0);
    let base = sticker.size.type_px();
    let layout = label.create_pango_layout(Some(words));
    layout.set_width((width * f64::from(gtk::pango::SCALE)) as i32);
    layout.set_wrap(gtk::pango::WrapMode::WordChar);
    let mut font = gtk::pango::FontDescription::new();
    font.set_family(fc_core::board::note_look::font_family(sticker.font));
    let fits = |scale: f64| {
        font.set_absolute_size(base * scale * f64::from(gtk::pango::SCALE));
        layout.set_font_description(Some(&font));
        f64::from(layout.pixel_size().1) <= room
    };
    let (scale, lines) = match fitted_scale(fits, MIN_TEXT_SCALE, FIT_STEPS) {
        // A backstop for one unbroken word, not a layout rule (the Mac's `fittedLineLimit`).
        Some(scale) => (scale, 20),
        None => (
            MIN_TEXT_SCALE,
            lines_that_fit(room, base * MIN_TEXT_SCALE * 1.25),
        ),
    };
    if scale < 1.0 {
        let attributes = gtk::pango::AttrList::new();
        attributes.insert(gtk::pango::AttrFloat::new_scale(scale));
        label.set_attributes(Some(&attributes));
    }
    label.set_lines(i32::try_from(lines.max(1)).unwrap_or(1));
}

/// The height the card leaves its words: less its padding and author line, and what an event's
/// date, a list's lines or a photo take.
fn room_for(sticker: &Sticker) -> f64 {
    let reserved = match sticker.kind {
        Kind::Event => 52.0,
        Kind::Tasks => {
            18.0 * f64::from(
                u32::try_from(wall_task_lines(sticker.note.task_list().len()).0 + 1).unwrap_or(1),
            )
        }
        Kind::Photo => sticker.height * 0.6,
        Kind::Text => 0.0,
    };
    (sticker.height - 20.0 - 14.0 - reserved).max(0.0)
}

/// A colour name the stylesheet knows (`fc_text::board::COLORS`); an unknown one draws yellow.
fn color_class(name: &str) -> &'static str {
    fc_text::board::COLORS
        .iter()
        .copied()
        .find(|known| *known == name)
        .unwrap_or("yellow")
}

/// `board_picture::CONTENT_FIT` read back into GTK's enum ("contain").
fn content_fit() -> gtk::ContentFit {
    match board_picture::CONTENT_FIT {
        "cover" => gtk::ContentFit::Cover,
        "fill" => gtk::ContentFit::Fill,
        "scale-down" => gtk::ContentFit::ScaleDown,
        _ => gtk::ContentFit::Contain,
    }
}

/// One card as the wall draws it — its look and its words, no keys: the wall's own cards, and the
/// note sheet's preview of the draft (`note_preview::of`), which must be the wall's own drawing.
pub fn draw_card(
    connection: &Rc<crate::shell::Connection>,
    sticker: &Sticker,
    hidden: bool,
    me: i64,
) -> gtk::Box {
    let shell = Shell::get();
    let say: &dyn StringCatalog = shell.say.as_ref();
    let note = &sticker.note;
    let caption = wall_text::caption_of(note);
    let bare = !hidden && sticker.kind == Kind::Photo && caption.is_empty();
    let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
    card.set_widget_name(&format!("note-{}", note.id));
    card.add_css_class("fc-note");
    card.add_css_class(&format!("fc-note-size-{}", sticker.size.name()));
    card.add_css_class(&format!("fc-note-font-{}", sticker.font.name()));
    if bare {
        card.add_css_class("fc-note-bare");
    } else {
        card.add_css_class(&format!("fc-note-{}", color_class(&sticker.color)));
    }
    card.set_size_request(sticker.width as i32, sticker.height as i32);
    card.set_overflow(gtk::Overflow::Hidden);
    card.set_focusable(true);
    card.set_accessible_role(gtk::AccessibleRole::Button);

    if !hidden
        && sticker.kind == Kind::Photo
        && let Some(attachment) = note.attachment.clone()
    {
        let picture = gtk::Picture::new();
        picture.set_content_fit(content_fit());
        picture.set_vexpand(true);
        card.append(&picture);
        let attachments = Rc::clone(&connection.attachments);
        ui::spawn(async move {
            if let Ok(bytes) = attachments.bytes(&attachment, true).await
                && let Ok(texture) = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
            {
                picture.set_paintable(Some(&texture));
            }
        });
    }
    let mut when = None;
    let mut going = None;
    if !hidden
        && sticker.kind == Kind::Event
        && let Some(starts) = note.starts_at.as_deref()
    {
        let zone = shell.zone.as_ref();
        let block = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if let Some((day, month)) = event_text::date_block(starts, &shell.culture, zone) {
            // A small calendar leaf: the day heavy over the month in calendar red.
            let date = gtk::Box::new(gtk::Orientation::Vertical, 0);
            date.add_css_class("fc-note-date");
            date.set_valign(gtk::Align::Start);
            let day = gtk::Label::new(Some(&day));
            day.add_css_class("fc-note-day");
            let month = gtk::Label::new(Some(&month.to_uppercase()));
            month.add_css_class("fc-note-month");
            date.append(&day);
            date.append(&month);
            block.append(&date);
        }
        let facts = gtk::Box::new(gtk::Orientation::Vertical, 0);
        facts.set_hexpand(true);
        let clock = event_text::clock(starts, note.ends_at.as_deref(), &shell.culture, zone);
        facts.append(&line(&clock));
        if let Some(place) = note.place.as_deref().filter(|p| !p.trim().is_empty()) {
            facts.append(&line(place.trim()));
        }
        going = going_line(
            note.count(Answer::Going.name()),
            note.count(Answer::Maybe.name()),
        );
        if let Some(going) = &going {
            facts.append(&line(going));
        }
        block.append(&facts);
        card.append(&block);
        when = Some(event_text::when(
            starts,
            note.ends_at.as_deref(),
            &shell.culture,
            zone,
        ));
    }
    if !bare {
        let text = gtk::Label::new(None);
        text.set_wrap(true);
        text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        text.set_xalign(0.0);
        text.set_yalign(0.0);
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        // A small natural width: the card's own width decides, never the words (a label's
        // natural width would otherwise widen the card past its size).
        text.set_max_width_chars(1);
        if hidden {
            text.set_text(say.get("Hidden — blocked member"));
            text.add_css_class("fc-note-hidden");
        } else {
            text.set_text(&caption);
            fit_text(&text, &caption, sticker);
        }
        card.append(&text);
    }
    if !hidden && sticker.kind == Kind::Tasks {
        let items = note.task_list();
        let (shown, left) = wall_task_lines(items.len());
        // Each line behind the theme's own checkbox, a done one struck through as well as
        // dimmed (the card's spoken name is `wall_text`'s, not these widgets').
        for item in items.iter().take(shown) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            row.add_css_class("fc-note-task");
            if item.done {
                row.add_css_class("fc-note-task-done");
            }
            row.append(
                &gtk::Image::builder()
                    .icon_name(if item.done {
                        "checkbox-checked-symbolic"
                    } else {
                        "checkbox-symbolic"
                    })
                    .valign(gtk::Align::Center)
                    .accessible_role(gtk::AccessibleRole::Presentation)
                    .build(),
            );
            let text = line(&item.text);
            text.set_hexpand(true);
            // The box is a picture to a screen reader: the line says its state itself, as the
            // ballot box did when it was a character.
            let mark = if item.done { "☑" } else { "☐" };
            text.update_property(&[gtk::accessible::Property::Label(&format!(
                "{mark} {}",
                item.text
            ))]);
            row.append(&text);
            card.append(&row);
        }
        if left > 0 {
            card.append(&line(&wall_text::more_line(
                i64::try_from(left).unwrap_or(0),
                say,
            )));
        }
    }
    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    card.append(&spacer);
    let author = author_name(connection, note, me);
    if wall_text::shows_author(sticker.kind, &caption, hidden) {
        let by = gtk::Label::new(Some(&author));
        by.set_xalign(0.0);
        by.set_ellipsize(gtk::pango::EllipsizeMode::End);
        by.set_max_width_chars(1);
        by.add_css_class("fc-note-author");
        card.append(&by);
    }
    let what = wall_text::what(
        sticker.kind,
        &caption,
        when.as_deref(),
        note.place.as_deref(),
        going.as_deref(),
        say,
    );
    let label = wall_text::label(hidden, sticker.mine, &author, &what, say);
    card.update_property(&[gtk::accessible::Property::Label(&label)]);
    card.set_tooltip_text(Some(&label));
    if !hidden
        && sticker.kind == Kind::Event
        && let Some(attachment) = note.attachment.clone()
    {
        backdrop(&card, attachment, &connection.attachments);
    }
    card
}

/// Who wrote a note, as the roster names them ("You" for the reader).
fn author_name(connection: &crate::shell::Connection, note: &NoteDto, me: i64) -> String {
    let shell = Shell::get();
    let chats = Rc::clone(&connection.chats);
    wall_text::author_name(
        note,
        me,
        move |id| chats.member(id).ok().flatten().map(|m| m.display_name),
        shell.say.as_ref(),
    )
}

/// An EVENT's picture is its BACKDROP — the card's ground, not its content (docs/protocol.md,
/// "Board"; the Mac's `NoteBackdrop`): drawn edge to edge under the words, filling the card, with
/// a white veil lighter at the top and heavier under the title so the words stay readable; nothing
/// while it loads (the card keeps its colour). The picture and the words are overlays that are not
/// measured, so a picture never makes the card bigger than its size.
fn backdrop(
    card: &gtk::Box,
    attachment: fc_core::protocol::AttachmentDto,
    attachments: &Rc<fc_logic::attachment_cache::AttachmentCache>,
) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    content.add_css_class("fc-note-backdrop-content");
    while let Some(child) = card.first_child() {
        card.remove(&child);
        content.append(&child);
    }
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    let veil = gtk::Box::new(gtk::Orientation::Vertical, 0);
    veil.add_css_class("fc-note-backdrop-veil");
    veil.set_visible(false);
    let ground = gtk::Overlay::new();
    let room = gtk::Box::new(gtk::Orientation::Vertical, 0);
    room.set_vexpand(true);
    room.set_hexpand(true);
    ground.set_child(Some(&room));
    ground.add_overlay(&picture);
    ground.add_overlay(&veil);
    ground.add_overlay(&content);
    ground.set_vexpand(true);
    card.add_css_class("fc-note-backdropped");
    card.append(&ground);
    let attachments = Rc::clone(attachments);
    ui::spawn(async move {
        if let Ok(bytes) = attachments.bytes(&attachment, true).await
            && let Ok(texture) = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
        {
            picture.set_paintable(Some(&texture));
            veil.set_visible(true);
        }
    });
}
