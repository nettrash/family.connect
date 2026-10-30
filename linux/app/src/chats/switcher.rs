//! The family switcher in the sidebar's header (shell-design §3.1; docs/protocol.md, "Several
//! families"): the family's NAME as a plain title for one family with nothing pending — the Mac's
//! rule (`family_rules::shows_switcher`) — and otherwise a menu button whose popover lists
//! `family_switcher::rows`: every family with its unread count (a click shows it), the join
//! requests waiting, the invitations with Join and Decline, and the two doors to a further family —
//! "New Family…" (`family_rules::offers_new_family`: the operator lets members start families, the
//! account owns none and is under its cap) and "Join Another Family…" (under the cap) — each an
//! `adw::Dialog` holding the family door's own form (`windows::gate::family_forms`).

use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::ChatRowDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::app_session::SessionState;
use fc_logic::families::{SwitcherRowKind, family_rules, family_switcher};

use super::banners;
use crate::services::log;
use crate::shell::{BRAND, Shell};
use crate::windows::gate::{FamilyForms, family_forms};

/// The header's title widget for this state.
pub fn title(shell: &Rc<Shell>, state: &SessionState, chats: &[ChatRowDto]) -> gtk::Widget {
    let say = &shell.say;
    let name = state
        .family
        .as_ref()
        .map_or_else(|| BRAND.to_string(), |family| family.name.clone());
    if !family_rules::shows_switcher(state) {
        return adw::WindowTitle::new(&name, "").upcast();
    }
    let unread: i32 = state
        .all_families()
        .iter()
        .filter(|family| Some(family.id) != state.current_family_id)
        .map(|family| family_switcher::unread(family.id, chats, state.home_family_id()))
        .sum();
    let label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    label.append(
        &gtk::Label::builder()
            .label(&name)
            .css_classes(["heading"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    if unread > 0 {
        // Unread in the OTHER families: a reason to look in the menu.
        label.append(
            &gtk::Label::builder()
                .label(unread.to_string())
                .css_classes(["fc-badge"])
                .build(),
        );
    }
    label.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
    let popover = gtk::Popover::new();
    popover.set_child(Some(&rows(shell, state, chats, &popover)));
    let button = gtk::MenuButton::builder()
        .child(&label)
        .popover(&popover)
        .css_classes(["flat"])
        .tooltip_text(say.get("Switch Family"))
        .build();
    button.update_property(&[gtk::accessible::Property::Label(say.get("Switch Family"))]);
    button.upcast()
}

/// The popover's rows.
fn rows(
    shell: &Rc<Shell>,
    state: &SessionState,
    chats: &[ChatRowDto],
    popover: &gtk::Popover,
) -> gtk::Widget {
    let say = &shell.say;
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.set_margin_top(4);
    column.set_margin_bottom(4);
    column.set_width_request(240);
    for row in family_switcher::rows(state, chats, say.as_ref()) {
        match row.kind {
            SwitcherRowKind::Family => {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let check = gtk::Image::from_icon_name("object-select-symbolic");
                check.set_opacity(if row.current { 1.0 } else { 0.0 });
                line.append(&check);
                line.append(
                    &gtk::Label::builder()
                        .label(&row.title)
                        .xalign(0.0)
                        .hexpand(true)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .build(),
                );
                if row.unread > 0 {
                    line.append(
                        &gtk::Label::builder()
                            .label(row.unread.to_string())
                            .css_classes(["fc-badge"])
                            .build(),
                    );
                }
                let button = gtk::Button::builder()
                    .child(&line)
                    .css_classes(["flat"])
                    .build();
                if let Some(name) = &row.accessible_name {
                    button.update_property(&[gtk::accessible::Property::Label(name)]);
                }
                let (id, popover) = (row.id, popover.clone());
                button.connect_clicked(move |_| {
                    popover.popdown();
                    if let Some(id) = id {
                        switch_to(id);
                    }
                });
                column.append(&button);
            }
            SwitcherRowKind::Pending => {
                let line = gtk::Box::new(gtk::Orientation::Vertical, 0);
                line.set_margin_start(12);
                line.set_margin_end(12);
                line.set_margin_top(4);
                line.set_margin_bottom(4);
                line.append(&gtk::Label::builder().label(&row.title).xalign(0.0).build());
                if let Some(detail) = &row.detail {
                    line.append(
                        &gtk::Label::builder()
                            .label(detail)
                            .xalign(0.0)
                            .css_classes(["dim-label", "caption"])
                            .build(),
                    );
                }
                column.append(&line);
            }
            SwitcherRowKind::Invitation => {
                let line = gtk::Box::new(gtk::Orientation::Vertical, 4);
                line.set_margin_start(12);
                line.set_margin_end(12);
                line.set_margin_top(4);
                line.set_margin_bottom(4);
                line.append(
                    &gtk::Label::builder()
                        .label(row.detail.as_deref().unwrap_or(&row.title))
                        .xalign(0.0)
                        .wrap(true)
                        .build(),
                );
                let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                buttons.set_halign(gtk::Align::End);
                let decline = gtk::Button::with_label(say.get("Decline"));
                let join = gtk::Button::builder()
                    .label(say.get("Join"))
                    .css_classes(["suggested-action"])
                    .build();
                buttons.append(&decline);
                buttons.append(&join);
                line.append(&buttons);
                if let Some(id) = row.id {
                    let shut = popover.clone();
                    join.connect_clicked(move |button| {
                        banners::answer(button, id, true);
                        shut.popdown();
                    });
                    let popover = popover.clone();
                    decline.connect_clicked(move |button| {
                        banners::answer(button, id, false);
                        popover.popdown();
                    });
                }
                column.append(&line);
            }
            SwitcherRowKind::NewFamily | SwitcherRowKind::JoinAnother => {
                let join = row.kind == SwitcherRowKind::JoinAnother;
                let button = gtk::Button::builder()
                    .label(&row.title)
                    .css_classes(["flat"])
                    .build();
                if let Some(child) = button.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                    child.set_xalign(0.0);
                }
                let (popover, title) = (popover.clone(), row.title.clone());
                button.connect_clicked(move |button| {
                    popover.popdown();
                    another_family(button, &title, join);
                });
                column.append(&button);
            }
        }
    }
    column.upcast()
}

/// Show family `id` (docs/protocol.md, "Switching"): the session switches at once and reads the
/// family's document for its roster; the chat list follows the session's change.
pub fn switch_to(id: i64) {
    let shell = Shell::get();
    let Some(connection) = shell.connection() else {
        return;
    };
    if !connection.session.switch(id) {
        return;
    }
    crate::fc_info!("family: switched to {id}");
    glib::spawn_future_local(async move {
        if let Err(error) = connection.session.refresh_family().await {
            crate::fc_warn!("family: the cache refused ({})", log::store_error(&error));
        }
    });
}

/// "New Family…" / "Join Another Family…": the family door's form in a dialog over the window.
fn another_family(anchor: &gtk::Button, title: &str, join: bool) {
    let shell = Shell::get();
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(420)
        .build();
    let close = {
        let dialog = dialog.clone();
        Rc::new(move || {
            dialog.close();
        })
    };
    let forms = family_forms(
        &shell,
        FamilyForms {
            join,
            create: !join,
            another: true,
        },
        close,
    );
    forms.set_margin_top(12);
    forms.set_margin_bottom(18);
    forms.set_margin_start(18);
    forms.set_margin_end(18);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&forms));
    dialog.set_child(Some(&view));
    let parent = anchor
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok())
        .or_else(|| {
            Shell::get()
                .main_window()
                .map(|main| main.window().clone().upcast())
        });
    dialog.present(parent.as_ref());
}
