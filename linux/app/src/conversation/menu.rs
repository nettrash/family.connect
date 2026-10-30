//! A message's menu (shell-design §5.2 "Menu"; `fc_logic::message_menu`): a
//! `gtk::PopoverMenu::from_model` whose first item is a custom child — the reaction row (the quick
//! set, the reader's own reaction appended, "More reactions…" opening GTK's emoji chooser) — and
//! whose items are the `win.message-*` actions with the message id as target (`seams::actions`), in
//! `message_menu::actions`' order, then the Safety group (Report…, Block / Unblock; an assistant's
//! reply: Report this reply…). A row hidden from a blocked member offers the Safety group and
//! nothing else — Copy would put the words it hides on the clipboard (the Mac's `rowMenu`). The
//! thread dialog uses the same menu under its own prefix
//! (`thread.`), so its rows act on the thread.
//!
//! [`install`] registers the twelve `win.message-*` actions on a window (or the thread's action
//! group), each handed to whatever [`MessageActions`] the window shows now.

use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::MessageDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::message_menu::{self, MessageAction};
use fc_text::emoji::capsule_emojis;

use crate::seams::actions::{self, ActionSpec};
use crate::shell::Shell;

/// What the message actions do on one surface.
pub trait MessageActions {
    fn reply(&self, message_id: i64);
    fn react(&self, message_id: i64, emoji: &str);
    fn show_text(&self, message_id: i64);
    fn playback_speed(&self, message_id: i64);
    fn save(&self, message_id: i64);
    fn open_full_screen(&self, message_id: i64);
    fn copy(&self, message_id: i64);
    fn view_thread(&self, message_id: i64);
    fn edit(&self, message_id: i64);
    /// A pending send's token (negative): discard it. The protocol deletes no message.
    fn delete(&self, token: i64);
    fn report(&self, message_id: i64);
    fn block(&self, message_id: i64);
    /// "See who reacted": every emoji, who chose it, the reader's own the way to take it off.
    fn reactors(&self, message_id: i64);
}

/// Registers `MESSAGE_ACTIONS` on `map`, each sent to `target()` at the time it is activated.
pub fn install(
    map: &impl IsA<gio::ActionMap>,
    target: Rc<dyn Fn() -> Option<Rc<dyn MessageActions>>>,
) {
    for spec in actions::MESSAGE_ACTIONS {
        let action = gio::SimpleAction::new(spec.name, spec.variant_type());
        let target = Rc::clone(&target);
        action.connect_activate(move |_, parameter| {
            let Some(surface) = target() else {
                return;
            };
            dispatch(spec, surface.as_ref(), parameter);
        });
        map.add_action(&action);
    }
}

fn dispatch(spec: ActionSpec, surface: &dyn MessageActions, parameter: Option<&glib::Variant>) {
    if spec == actions::MESSAGE_REACT {
        if let Some((id, emoji)) = parameter.and_then(|p| p.get::<(i64, String)>()) {
            surface.react(id, &emoji);
        }
        return;
    }
    let Some(id) = parameter.and_then(|p| p.get::<i64>()) else {
        return;
    };
    match spec.name {
        n if n == actions::MESSAGE_REPLY.name => surface.reply(id),
        n if n == actions::MESSAGE_SHOW_TEXT.name => surface.show_text(id),
        n if n == actions::MESSAGE_PLAYBACK_SPEED.name => surface.playback_speed(id),
        n if n == actions::MESSAGE_SAVE.name => surface.save(id),
        n if n == actions::MESSAGE_OPEN_FULL_SCREEN.name => surface.open_full_screen(id),
        n if n == actions::MESSAGE_COPY.name => surface.copy(id),
        n if n == actions::MESSAGE_VIEW_THREAD.name => surface.view_thread(id),
        n if n == actions::MESSAGE_EDIT.name => surface.edit(id),
        n if n == actions::MESSAGE_DELETE.name => surface.delete(id),
        n if n == actions::MESSAGE_REPORT.name => surface.report(id),
        n if n == actions::MESSAGE_BLOCK.name => surface.block(id),
        n if n == actions::MESSAGE_REACTORS.name => surface.reactors(id),
        _ => {}
    }
}

/// Who the Safety group is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Safety {
    /// The reader's own, or nothing to say.
    None,
    /// Another member: Report… (when it may be reported) and Block or Unblock.
    Member { may_report: bool, blocked: bool },
    /// The assistant's reply: Report this reply… only.
    Assistant,
}

/// Everything the menu is drawn from.
pub struct MenuFacts {
    pub message: MessageDto,
    pub may_edit: bool,
    pub in_thread: bool,
    /// "Show text", "Hide text" or neither, for a recording.
    pub text_item: message_menu::TextItem,
    pub my_reaction: Option<String>,
    /// Drawn as "Hidden — blocked member" (from a blocked sender, not revealed here): Safety only.
    pub hidden: bool,
    pub safety: Safety,
}

fn item(label: &str, prefix: &str, spec: ActionSpec, id: i64) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), None);
    item.set_action_and_target_value(
        Some(&format!("{prefix}.{}", spec.name)),
        Some(&id.to_variant()),
    );
    item
}

/// The menu for `facts` at (`x`, `y`) of `anchor`; `prefix` is `win` or `thread`.
pub fn popup(facts: &MenuFacts, prefix: &str, anchor: &gtk::Widget, x: f64, y: f64) {
    let shell = Shell::get();
    let say = &shell.say;
    let message = &facts.message;
    let id = message.id;
    let menu = gio::Menu::new();
    let numbered = id != 0;
    let items = if facts.hidden {
        Vec::new()
    } else {
        message_menu::actions(
            message,
            facts.may_edit,
            facts.in_thread,
            facts.text_item != message_menu::TextItem::None,
        )
    };
    let reacts = numbered && items.contains(&MessageAction::React);
    if reacts {
        let section = gio::Menu::new();
        let custom = gio::MenuItem::new(None, None);
        custom.set_attribute_value("custom", Some(&"reactions".to_variant()));
        section.append_item(&custom);
        menu.append_section(None, &section);
    }
    let main = gio::Menu::new();
    // Right under the reactions, where the Mac lists it: who chose them.
    let reacted = message.reactions.as_deref().is_some_and(|r| !r.is_empty());
    if reacts && reacted {
        main.append_item(&item(
            say.get("See who reacted"),
            prefix,
            actions::MESSAGE_REACTORS,
            id,
        ));
    }
    for action in items.iter().filter(|_| numbered) {
        let entry = match action {
            MessageAction::React => None,
            MessageAction::Reply => {
                Some(item(say.get("Reply"), prefix, actions::MESSAGE_REPLY, id))
            }
            MessageAction::ShowText => Some(item(
                if facts.text_item == message_menu::TextItem::Hide {
                    say.get("Hide text")
                } else {
                    say.get("Show text")
                },
                prefix,
                actions::MESSAGE_SHOW_TEXT,
                id,
            )),
            MessageAction::PlaybackSpeed => {
                let rate = fc_logic::voice_look::voice_speed::parse(
                    shell
                        .settings
                        .string(crate::services::settings_store::keys::VOICE_SPEED)
                        .as_deref(),
                );
                Some(item(
                    &fc_logic::voice_look::voice_speed::name(rate, say.as_ref()),
                    prefix,
                    actions::MESSAGE_PLAYBACK_SPEED,
                    id,
                ))
            }
            MessageAction::Save => Some(item(say.get("Save…"), prefix, actions::MESSAGE_SAVE, id)),
            MessageAction::OpenFullScreen => Some(item(
                say.get("Open Full Screen"),
                prefix,
                actions::MESSAGE_OPEN_FULL_SCREEN,
                id,
            )),
            MessageAction::Copy => Some(item(say.get("Copy"), prefix, actions::MESSAGE_COPY, id)),
            MessageAction::ViewThread => Some(item(
                say.get("View thread"),
                prefix,
                actions::MESSAGE_VIEW_THREAD,
                id,
            )),
            MessageAction::Edit => Some(item(say.get("Edit"), prefix, actions::MESSAGE_EDIT, id)),
        };
        if let Some(entry) = entry {
            main.append_item(&entry);
        }
    }
    // Copy is there for any words — a pending row too (no id).
    if !numbered && !facts.hidden && !message.body.is_empty() {
        main.append_item(&item(say.get("Copy"), prefix, actions::MESSAGE_COPY, id));
    }
    menu.append_section(None, &main);
    match facts.safety {
        Safety::None => {}
        Safety::Member {
            may_report,
            blocked,
        } => {
            let safety = gio::Menu::new();
            if may_report {
                safety.append_item(&item(
                    say.get("Report…"),
                    prefix,
                    actions::MESSAGE_REPORT,
                    id,
                ));
            }
            safety.append_item(&item(
                if blocked {
                    say.get("Unblock")
                } else {
                    say.get("Block")
                },
                prefix,
                actions::MESSAGE_BLOCK,
                id,
            ));
            let section = gio::Menu::new();
            section.append_submenu(Some(say.get("Safety")), &safety);
            menu.append_section(None, &section);
        }
        Safety::Assistant => {
            let safety = gio::Menu::new();
            safety.append_item(&item(
                say.get("Report this reply…"),
                prefix,
                actions::MESSAGE_REPORT,
                id,
            ));
            let section = gio::Menu::new();
            section.append_submenu(Some(say.get("Safety")), &safety);
            menu.append_section(None, &section);
        }
    }
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    if reacts {
        popover.add_child(&reaction_row(facts, prefix, &popover), "reactions");
    }
    popover.set_parent(anchor);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}

/// The quick reactions, the reader's own appended, and "More reactions…".
fn reaction_row(facts: &MenuFacts, prefix: &str, popover: &gtk::PopoverMenu) -> gtk::Widget {
    let shell = Shell::get();
    let say = &shell.say;
    let id = facts.message.id;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    row.add_css_class("fc-chat-reaction-row");
    let react = format!("{prefix}.{}", actions::MESSAGE_REACT.name);
    for emoji in capsule_emojis(facts.my_reaction.as_deref()) {
        let button = gtk::Button::builder()
            .label(emoji)
            .css_classes(["flat"])
            .action_name(&react)
            .action_target(&(id, emoji.to_string()).to_variant())
            .build();
        button.update_property(&[gtk::accessible::Property::Label(emoji)]);
        if facts.my_reaction.as_deref() == Some(emoji) {
            button.add_css_class("fc-chat-mine");
        }
        let popover = popover.clone();
        button.connect_clicked(move |_| popover.popdown());
        row.append(&button);
    }
    let more = gtk::Button::builder()
        .icon_name("view-more-horizontal-symbolic")
        .css_classes(["flat"])
        .tooltip_text(say.get("More reactions…"))
        .build();
    more.update_property(&[gtk::accessible::Property::Label(say.get("More reactions…"))]);
    {
        let (popover, react) = (popover.clone(), react.clone());
        more.connect_clicked(move |button| {
            let chooser = gtk::EmojiChooser::new();
            chooser.set_parent(button);
            let (popover, react, button_ref) = (popover.clone(), react.clone(), button.clone());
            chooser.connect_emoji_picked(move |_, emoji| {
                let _ =
                    button_ref.activate_action(&react, Some(&(id, emoji.to_string()).to_variant()));
                popover.popdown();
            });
            chooser.connect_closed(|chooser| {
                let chooser = chooser.clone();
                glib::idle_add_local_once(move || chooser.unparent());
            });
            chooser.popup();
        });
    }
    row.append(&more);
    row.upcast()
}
