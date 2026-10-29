//! The chat list's widget (shell-design §4): a `gtk::ListBox` with `.navigation-sidebar` in a
//! `gtk::ScrolledWindow` — tens of rows, every row a real widget — drawing exactly the rows
//! `fc_logic::chat_list::ChatListModel` answers and nothing else.
//!
//! **Updated in place** by chat id: a row whose chat is still listed keeps its widget and only its
//! words change; the order is a sort function over the model's order (`invalidate_sort`), never
//! rows removed and added again — Windows learnt that emptying a list inside its own selection
//! handler crashes it, and a row re-added under the pointer loses the click.
//!
//! **Opening in a window** is the context menu's (secondary click, long press, Shift+F10, the Menu
//! key) — never a double click, which stole the Mac list's own clicks.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::chat_list::ChatRow;
use fc_logic::groups::ChatFace;

use super::avatar;

/// What the list tells its owner.
pub struct ListHandlers {
    /// A row was chosen (clicked, Enter, the arrows): open its chat.
    pub select: Box<dyn Fn(i64)>,
    /// A row was activated where the content pane is hidden (a narrow window): bring it forward.
    pub activate: Box<dyn Fn(i64)>,
    /// "Open in New Window".
    pub open_window: Box<dyn Fn(i64)>,
}

struct RowParts {
    row: gtk::ListBoxRow,
    avatar: adw::Avatar,
    title: gtk::Label,
    time: gtk::Label,
    preview: gtk::Label,
    mention: gtk::Label,
    unread: gtk::Label,
    drawn: Option<(ChatRow, String)>,
}

/// The chat list (see the module's comment).
pub struct ChatList {
    scroller: gtk::ScrolledWindow,
    list: gtk::ListBox,
    rows: RefCell<HashMap<i64, RowParts>>,
    order: Rc<RefCell<HashMap<i64, usize>>>,
    /// The program is moving the selection: not a reader's choice.
    selecting: Rc<Cell<bool>>,
    say: Rc<dyn StringCatalog>,
}

impl std::fmt::Debug for ChatList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatList")
            .field("rows", &self.rows.borrow().len())
            .finish_non_exhaustive()
    }
}

/// The chat id a row stands for (its widget name `chat-<id>`).
pub fn row_chat(row: &gtk::ListBoxRow) -> Option<i64> {
    row.widget_name().strip_prefix("chat-")?.parse().ok()
}

impl ChatList {
    pub fn new(say: Rc<dyn StringCatalog>, handlers: ListHandlers) -> Rc<ChatList> {
        let list = gtk::ListBox::builder()
            .css_classes(["navigation-sidebar"])
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        list.update_property(&[gtk::accessible::Property::Label(say.get("Chats"))]);
        let order: Rc<RefCell<HashMap<i64, usize>>> = Rc::new(RefCell::new(HashMap::new()));
        {
            let order = Rc::clone(&order);
            list.set_sort_func(move |a, b| {
                let order = order.borrow();
                let place = |row: &gtk::ListBoxRow| {
                    row_chat(row)
                        .and_then(|id| order.get(&id).copied())
                        .unwrap_or(usize::MAX)
                };
                place(a).cmp(&place(b)).into()
            });
        }
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        let selecting = Rc::new(Cell::new(false));
        let handlers = Rc::new(handlers);
        {
            let (selecting, handlers) = (Rc::clone(&selecting), Rc::clone(&handlers));
            list.connect_row_selected(move |_, row| {
                if selecting.get() {
                    return;
                }
                if let Some(id) = row.and_then(row_chat) {
                    // A beat later: a chat opened from inside the list's own selection handler
                    // must not rebuild what the handler is walking (Windows' lesson).
                    let handlers = Rc::clone(&handlers);
                    glib::idle_add_local_once(move || (handlers.select)(id));
                }
            });
        }
        {
            let handlers = Rc::clone(&handlers);
            list.connect_row_activated(move |_, row| {
                if let Some(id) = row_chat(row) {
                    (handlers.activate)(id);
                }
            });
        }
        // "Open in New Window": an action on the list, offered by every row's context menu.
        let group = gio::SimpleActionGroup::new();
        let open = gio::SimpleAction::new("open-window", Some(glib::VariantTy::INT64));
        {
            let handlers = Rc::clone(&handlers);
            open.connect_activate(move |_, target| {
                if let Some(id) = target.and_then(|t| t.get::<i64>()) {
                    (handlers.open_window)(id);
                }
            });
        }
        group.add_action(&open);
        list.insert_action_group("chats", Some(&group));
        // Shift+F10 and the Menu key open the focused row's menu.
        let keys = gtk::EventControllerKey::new();
        {
            let (list_ref, say) = (list.clone(), Rc::clone(&say));
            keys.connect_key_pressed(move |_, key, _, state| {
                let menu_key = key == gtk::gdk::Key::Menu
                    || (key == gtk::gdk::Key::F10
                        && state.contains(gtk::gdk::ModifierType::SHIFT_MASK));
                if !menu_key {
                    return glib::Propagation::Proceed;
                }
                let focused = list_ref
                    .focus_child()
                    .and_then(|w| w.downcast::<gtk::ListBoxRow>().ok())
                    .or_else(|| list_ref.selected_row());
                if let Some(row) = focused {
                    let (w, h) = (f64::from(row.width()), f64::from(row.height()));
                    context_menu(&row, w / 2.0, h / 2.0, say.as_ref());
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        list.add_controller(keys);
        Rc::new(ChatList {
            scroller,
            list,
            rows: RefCell::new(HashMap::new()),
            order,
            selecting,
            say,
        })
    }

    /// The widget to place (the sidebar's slot).
    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.scroller
    }

    /// Draws `rows` in their order; `times[i]` is row i's time as the reader reads it.
    pub fn update(&self, rows: &[ChatRow], times: &[String]) {
        let mut held = self.rows.borrow_mut();
        let listed: Vec<i64> = rows.iter().map(|row| row.chat.id).collect();
        let gone: Vec<i64> = held
            .keys()
            .copied()
            .filter(|id| !listed.contains(id))
            .collect();
        for id in gone {
            if let Some(parts) = held.remove(&id) {
                self.selecting.set(true);
                self.list.remove(&parts.row);
                self.selecting.set(false);
            }
        }
        {
            let mut order = self.order.borrow_mut();
            order.clear();
            for (index, id) in listed.iter().enumerate() {
                order.insert(*id, index);
            }
        }
        for (row, time) in rows.iter().zip(times.iter()) {
            let id = row.chat.id;
            let parts = held.entry(id).or_insert_with(|| {
                let parts = self.build_row(row);
                self.list.append(&parts.row);
                parts
            });
            {
                let changed = parts
                    .drawn
                    .as_ref()
                    .is_none_or(|(drawn, at)| drawn != row || at != time);
                if changed {
                    fill_row(parts, row, time, self.say.as_ref());
                    parts.drawn = Some((row.clone(), time.clone()));
                }
            }
            // A person's picture follows the roster's avatar version, which the row itself does
            // not carry (a new picture changes no chat row): asked again on every redraw — the
            // decoded face is kept per (user, version), so an unchanged one costs a lookup.
            if row.face() == ChatFace::Initials
                && let Some(peer) = row.chat.peer_user_id
            {
                crate::attachments::avatars::show_member(&parts.avatar, peer);
            }
        }
        drop(held);
        self.list.invalidate_sort();
    }

    fn build_row(&self, row: &ChatRow) -> RowParts {
        let face = avatar::avatar(&row.title, row.face(), row.chat.peer_user_id, 34);
        face.set_valign(gtk::Align::Center);
        let title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let time = gtk::Label::builder()
            .css_classes(["fc-chat-list-time"])
            .valign(gtk::Align::Start)
            .build();
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        top.append(&title);
        top.append(&time);
        let preview = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .single_line_mode(true)
            .css_classes(["fc-chat-list-preview"])
            .build();
        let mention = gtk::Label::builder()
            .label("@")
            .css_classes(["fc-badge"])
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        mention.update_property(&[gtk::accessible::Property::Label(
            self.say.get("Mentions you"),
        )]);
        let unread = gtk::Label::builder()
            .css_classes(["fc-badge"])
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bottom.append(&preview);
        bottom.append(&mention);
        bottom.append(&unread);
        let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
        words.set_valign(gtk::Align::Center);
        words.append(&top);
        words.append(&bottom);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        line.add_css_class("fc-chat-list-row");
        line.append(&face);
        line.append(&words);
        let list_row = gtk::ListBoxRow::builder().child(&line).build();
        list_row.set_widget_name(&format!("chat-{}", row.chat.id));
        // The context menu: a secondary click, or a long press on a touch screen.
        let click = gtk::GestureClick::builder().button(3).build();
        {
            let (list_row, say) = (list_row.clone(), Rc::clone(&self.say));
            click.connect_pressed(move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                context_menu(&list_row, x, y, say.as_ref());
            });
        }
        list_row.add_controller(click);
        let long = gtk::GestureLongPress::new();
        long.set_touch_only(true);
        {
            let (list_row, say) = (list_row.clone(), Rc::clone(&self.say));
            long.connect_pressed(move |_, x, y| context_menu(&list_row, x, y, say.as_ref()));
        }
        list_row.add_controller(long);
        RowParts {
            row: list_row,
            avatar: face,
            title,
            time,
            preview,
            mention,
            unread,
            drawn: None,
        }
    }

    /// Selects `chat_id`'s row without it counting as the reader's choice. False when the list
    /// does not hold it.
    pub fn select_quietly(&self, chat_id: i64) -> bool {
        let row = self
            .rows
            .borrow()
            .get(&chat_id)
            .map(|parts| parts.row.clone());
        match row {
            Some(row) => {
                if self.list.selected_row().as_ref() != Some(&row) {
                    self.selecting.set(true);
                    self.list.select_row(Some(&row));
                    self.selecting.set(false);
                }
                true
            }
            None => false,
        }
    }

    /// Clears the selection quietly.
    pub fn unselect(&self) {
        self.selecting.set(true);
        self.list.unselect_all();
        self.selecting.set(false);
    }

    /// The chats listed, in order.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn listed(&self) -> Vec<i64> {
        let order = self.order.borrow();
        let mut ids: Vec<(usize, i64)> = order.iter().map(|(id, at)| (*at, *id)).collect();
        ids.sort_unstable();
        ids.into_iter().map(|(_, id)| id).collect()
    }

    /// Row `chat_id`'s widget (the self-test clicks it).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn row(&self, chat_id: i64) -> Option<gtk::ListBoxRow> {
        self.rows
            .borrow()
            .get(&chat_id)
            .map(|parts| parts.row.clone())
    }
}

/// One row's words, drawn from the model's row.
fn fill_row(parts: &RowParts, row: &ChatRow, time: &str, say: &dyn StringCatalog) {
    parts.avatar.set_text(Some(&row.title));
    parts.title.set_label(&row.title);
    // The family chat's title is the family's NAME and a group's the group's: they get a second
    // line (the Mac's rule); a person's name stays on one.
    let lines = if row.is_family() || row.is_group() {
        2
    } else {
        1
    };
    parts.title.set_lines(lines);
    parts.title.set_wrap(lines > 1);
    parts.title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    if row.unread > 0 {
        parts.title.add_css_class("heading");
    } else {
        parts.title.remove_css_class("heading");
    }
    parts.time.set_label(time);
    parts.time.set_visible(!time.is_empty());
    parts.preview.set_label(&row.preview);
    parts.mention.set_visible(row.mentioned);
    parts.unread.set_label(&row.unread.to_string());
    parts.unread.set_visible(row.unread > 0);
    let mut name = format!("{}, {}", row.title, row.preview);
    if row.unread > 0 {
        name.push_str(", ");
        name.push_str(&say.plural(
            "%lld unread",
            i64::from(row.unread),
            &[&i64::from(row.unread)],
        ));
    }
    if row.mentioned {
        name.push_str(", ");
        name.push_str(say.get("Mentions you"));
    }
    parts
        .row
        .update_property(&[gtk::accessible::Property::Label(&name)]);
}

/// "Open in New Window" at (`x`, `y`) of `row`.
fn context_menu(row: &gtk::ListBoxRow, x: f64, y: f64, say: &dyn StringCatalog) {
    let Some(id) = row_chat(row) else {
        return;
    };
    let menu = gio::Menu::new();
    let item = gio::MenuItem::new(Some(say.get("Open in New Window")), None);
    item.set_action_and_target_value(Some("chats.open-window"), Some(&id.to_variant()));
    menu.append_item(&item);
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(row);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}
