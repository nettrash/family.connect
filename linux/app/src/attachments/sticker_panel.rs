//! The sticker panel (shell-design §8.4; the Mac's `StickerPanel`): what THIS device sent lately
//! ("Recently used", sixteen, this device's own — `PackPanel::RECENT_ROOM`) and then "All stickers"
//! in the order they were added, five 72-pt cells across (`StickerLook::PANEL_COLUMNS`,
//! `PANEL_CELL`). One click picks: the panel hands the item to the caller's `on_pick` and closes —
//! P1's composer then runs `pack_sending::gate_for` (the consent question in the assistant's chat),
//! `PackModel::to_send`, stages and queues it, and `PackModel::sent`. A self-contained widget:
//! [`popover`] for the composer's sticker button, [`panel`] for anywhere else.
//!
//! Each cell is a sticker drawn as in a bubble (animated where it can be), named for a screen
//! reader by `pack_text::name` — "Sticker", or "Sticker: <label>" — and described "Sends this
//! sticker". Nothing is drawn over the picture.

use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::PackItemDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::pack::pack_text;
use fc_logic::sticker_look::StickerLook;

use super::stickers;
use crate::shell::Shell;

/// Between cells.
const SPACING: u32 = 4;
/// The panel's tallest before it scrolls.
const MOST_HEIGHT: i32 = 360;

/// A popover holding the panel, refreshed each time it opens; a pick closes it. Set its parent
/// (`set_parent`) or give it to a `gtk::MenuButton` (`set_popover`).
pub fn popover(on_pick: impl Fn(PackItemDto) + 'static) -> gtk::Popover {
    let popover = gtk::Popover::builder().has_arrow(true).build();
    let on_pick: Rc<dyn Fn(PackItemDto)> = Rc::new(on_pick);
    popover.connect_show(move |popover| {
        let weak = popover.downgrade();
        let pick = Rc::clone(&on_pick);
        let content = panel(Rc::new(move |item| {
            if let Some(popover) = weak.upgrade() {
                popover.popdown();
            }
            pick(item);
        }));
        popover.set_child(Some(&content));
    });
    popover
}

/// The panel's content, read from the pack now. `on_pick` gets the clicked item.
pub fn panel(on_pick: Rc<dyn Fn(PackItemDto)>) -> gtk::Widget {
    let shell = Shell::get();
    let say = &shell.say;
    let model = shell
        .connection()
        .and_then(|connection| connection.stickers.panel().ok())
        .unwrap_or_default();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_start(6)
        .margin_end(6)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    if model.is_empty() {
        let empty = adw::StatusPage::builder()
            .icon_name("face-smile-symbolic")
            .title(say.get("No stickers yet"))
            .description(
                say.get("Add a picture and everyone in the family can send it as a sticker."),
            )
            .css_classes(["compact"])
            .build();
        column.append(&empty);
        return column.upcast();
    }
    if !model.recent.is_empty() {
        column.append(&heading(say.get("Recently used")));
        column.append(&cells(&model.recent, &on_pick));
    }
    column.append(&heading(say.get("All stickers")));
    column.append(&cells(&model.all, &on_pick));
    let width = cell_side() * StickerLook::PANEL_COLUMNS
        + SPACING as i32 * (StickerLook::PANEL_COLUMNS - 1)
        + 12;
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(MOST_HEIGHT)
        .min_content_width(width)
        .child(&column)
        .build()
        .upcast()
}

fn cell_side() -> i32 {
    super::pictures::logical(StickerLook::PANEL_CELL)
}

fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["fc-sticker-heading"])
        .build()
}

fn cells(items: &[PackItemDto], on_pick: &Rc<dyn Fn(PackItemDto)>) -> gtk::FlowBox {
    let shell = Shell::get();
    let say = &shell.say;
    let columns = u32::try_from(StickerLook::PANEL_COLUMNS).unwrap_or(5);
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(columns)
        .max_children_per_line(columns)
        .row_spacing(SPACING)
        .column_spacing(SPACING)
        .build();
    // The picture inside the cell's padding.
    let inner = StickerLook::PANEL_CELL - 8.0;
    for item in items {
        let Some(picture) = item.attachment.as_ref() else {
            continue;
        };
        let (width, height) = StickerLook::fit_in(picture.width, picture.height, inner);
        let drawn = stickers::picture(picture, width, height);
        drawn.set_halign(gtk::Align::Center);
        drawn.set_valign(gtk::Align::Center);
        let button = gtk::Button::builder()
            .child(&drawn)
            .width_request(cell_side())
            .height_request(cell_side())
            .css_classes(["flat", "fc-sticker-cell"])
            .tooltip_text(pack_text::name(item, say.as_ref()))
            .build();
        button.update_property(&[
            gtk::accessible::Property::Label(&pack_text::name(item, say.as_ref())),
            gtk::accessible::Property::Description(say.get("Sends this sticker")),
        ]);
        let (item, on_pick) = (item.clone(), Rc::clone(on_pick));
        button.connect_clicked(move |_| on_pick(item.clone()));
        flow.insert(&button, -1);
    }
    flow
}
