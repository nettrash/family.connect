//! The family's sticker pack (shell-design §8.4, §3.2; the Mac's `StickerPackView`, which its
//! Family screen opens): an `adw::Dialog` "Family Stickers" with every sticker in the order it was
//! added (88-pt cells, `StickerLook::MANAGE_CELL`), a remove control on the ones this reader may
//! remove (whoever added it, or the family's owner — `PackModel::may_remove`), "Add a sticker", and
//! the footer: how full the pack is and what may be added. P4's Family dialog opens it with
//! [`present`]; nothing here is offered on a server with no packs (`PackModel::offered`).
//!
//! **Adding** (`PackPicking`, decided by `pack_picking::plan_for` BEFORE anything is uploaded): a
//! WebP or a PNG within the ceiling goes up as it is; any other still picture gdk-pixbuf decodes is
//! fitted whole into 512 × 512 and written as PNG with its transparency (never the photo path's
//! JPEG); an animated picture that is not already an acceptable WebP is refused in words. The
//! person may give a few words for a screen reader (`pack_label`), never drawn over the picture.
//! Every refusal is said with `pack_text::sentence`, beside the grid.
//!
//! **Removing** asks first ("Remove this sticker?"): every message ever sent with it keeps its own
//! copy. Already gone is what was asked for (`PackModel::remove` drops it without a word).

use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{ApiError, PackItemDto};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::pack::{PackModel, pack_label, pack_picking, pack_text};
use fc_logic::sticker_look::StickerLook;
use gtk::gdk_pixbuf;

use super::stickers;
use crate::shell::{Connection, Shell};

/// The dialog's content size (the Mac's 520 × 520 sheet).
const SIZE: (i32, i32) = (520, 520);

/// Opens "Family Stickers" over `parent`.
pub fn present(parent: &impl IsA<gtk::Widget>) {
    let Some(connection) = Shell::get().connection() else {
        return;
    };
    let dialog = PackDialog::new(connection);
    dialog.dialog.present(Some(parent));
}

/// The dialog and what it redraws.
pub struct PackDialog {
    pub dialog: adw::Dialog,
    connection: Rc<Connection>,
    grid: adw::Bin,
    footer: gtk::Label,
    notice: gtk::Label,
    toasts: adw::ToastOverlay,
    add: gtk::Button,
}

impl PackDialog {
    pub fn new(connection: Rc<Connection>) -> Rc<PackDialog> {
        let shell = Shell::get();
        let say = &shell.say;
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text(say.get("Add a sticker"))
            .build();
        add.update_property(&[gtk::accessible::Property::Label(say.get("Add a sticker"))]);
        let header = adw::HeaderBar::new();
        header.pack_start(&add);
        let notice = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .visible(false)
            .css_classes(["fc-pack-notice"])
            .build();
        let grid = adw::Bin::new();
        let footer = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label", "caption"])
            .build();
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_start(16)
            .margin_end(16)
            .margin_top(12)
            .margin_bottom(16)
            .build();
        column.append(&notice);
        column.append(&grid);
        column.append(&footer);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&column)
            .build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&scrolled));
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&toasts));
        let dialog = adw::Dialog::builder()
            .title(say.get("Family Stickers"))
            .content_width(SIZE.0)
            .content_height(SIZE.1)
            .child(&view)
            .build();
        let me = Rc::new(PackDialog {
            dialog,
            connection,
            grid,
            footer,
            notice,
            toasts,
            add,
        });
        let weak = Rc::downgrade(&me);
        me.add.connect_clicked(move |_| {
            if let Some(me) = weak.upgrade() {
                me.pick();
            }
        });
        me.redraw();
        me
    }

    /// The grid and the footer, from what the pack holds now.
    pub fn redraw(self: &Rc<Self>) {
        let shell = Shell::get();
        let say = &shell.say;
        let items = self.connection.stickers.items().unwrap_or_default();
        let limits = self.connection.stickers.limits().ok().flatten();
        let full = self.connection.stickers.is_full().unwrap_or(false);
        self.add.set_sensitive(!full && limits.is_some());
        let mut footer = Vec::new();
        if let Some(limits) = limits {
            footer.push(pack_text::fullness(items.len(), limits, say.as_ref()));
        }
        footer.push(
            say.get(
                "WebP and PNG pictures are added as they are. Anything else is fitted into 512 × 512.",
            )
            .to_string(),
        );
        footer.push(
            say.get("An animated sticker has to be a WebP picture.")
                .to_string(),
        );
        self.footer.set_label(&footer.join("\n"));
        if items.is_empty() {
            let empty = adw::StatusPage::builder()
                .paintable(&crate::theme::glyphs::sticker())
                .title(say.get("No stickers yet"))
                .description(
                    say.get("Add a picture and everyone in the family can send it as a sticker."),
                )
                .css_classes(["compact"])
                .build();
            self.grid.set_child(Some(&empty));
            return;
        }
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(3)
            .max_children_per_line(5)
            .row_spacing(10)
            .column_spacing(10)
            .build();
        let owner = self.connection.session.state().is_owner();
        for item in &items {
            flow.insert(&self.cell(item, owner), -1);
        }
        self.grid.set_child(Some(&flow));
    }

    fn cell(self: &Rc<Self>, item: &PackItemDto, owner: bool) -> gtk::Widget {
        let shell = Shell::get();
        let say = &shell.say;
        let side = super::pictures::logical(StickerLook::MANAGE_CELL);
        let frame = gtk::Overlay::builder()
            .width_request(side)
            .height_request(side)
            .css_classes(["fc-pack-cell"])
            .build();
        let inner = StickerLook::MANAGE_CELL - 12.0;
        let picture = match item.attachment.as_ref() {
            Some(attachment) => {
                let (width, height) =
                    StickerLook::fit_in(attachment.width, attachment.height, inner);
                stickers::picture(attachment, width, height)
            }
            None => gtk::Image::from_icon_name("image-missing-symbolic").upcast(),
        };
        picture.set_halign(gtk::Align::Center);
        picture.set_valign(gtk::Align::Center);
        frame.set_child(Some(&picture));
        frame.update_property(&[gtk::accessible::Property::Label(&pack_text::name(
            item,
            say.as_ref(),
        ))]);
        if self.connection.stickers.may_remove(item, owner) {
            let remove = gtk::Button::builder()
                .icon_name("window-close-symbolic")
                .halign(gtk::Align::End)
                .valign(gtk::Align::Start)
                .tooltip_text(say.get("Remove sticker"))
                .css_classes(["circular", "osd"])
                .build();
            remove.update_property(&[gtk::accessible::Property::Label(say.get("Remove sticker"))]);
            let (weak, id) = (Rc::downgrade(self), item.id);
            remove.connect_clicked(move |_| {
                if let Some(me) = weak.upgrade() {
                    me.ask_to_remove(id);
                }
            });
            frame.add_overlay(&remove);
        }
        frame.upcast()
    }

    fn say_notice(&self, error: Option<&ApiError>) {
        let shell = Shell::get();
        match error {
            Some(error) => {
                self.notice
                    .set_label(&pack_text::sentence(error, shell.say.as_ref()));
                self.notice.set_visible(true);
            }
            None => self.notice.set_visible(false),
        }
    }

    fn ask_to_remove(self: &Rc<Self>, item_id: i64) {
        let shell = Shell::get();
        let say = &shell.say;
        let alert = adw::AlertDialog::new(
            Some(say.get("Remove this sticker?")),
            Some(say.get(
                "It leaves everyone's sticker panel. Stickers already sent stay in the chat.",
            )),
        );
        alert.add_response("cancel", say.get("Cancel"));
        alert.add_response("remove", say.get("Remove"));
        alert.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        alert.connect_response(None, move |_, response| {
            if response != "remove" {
                return;
            }
            let Some(me) = weak.upgrade() else {
                return;
            };
            glib::spawn_future_local(async move {
                match me.connection.stickers.remove(item_id).await {
                    Ok(error) => me.say_notice(error.as_ref()),
                    Err(error) => crate::fc_warn!(
                        "pack: the cache refused ({})",
                        crate::services::log::store_error(&error)
                    ),
                }
                me.redraw();
            });
        });
        alert.present(Some(&self.dialog));
    }

    /// "Add a sticker": a picture chosen, planned, made if it must be, labelled, added.
    fn pick(self: &Rc<Self>) {
        let me = Rc::clone(self);
        glib::spawn_future_local(async move {
            let filter = gtk::FileFilter::new();
            filter.add_mime_type("image/*");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let chooser = gtk::FileDialog::builder()
                .modal(true)
                .filters(&filters)
                .default_filter(&filter)
                .build();
            let parent = me.dialog.root().and_downcast::<gtk::Window>();
            let Ok(file) = chooser.open_future(parent.as_ref()).await else {
                return;
            };
            let Ok((bytes, _)) = file.load_contents_future().await else {
                me.say_notice(Some(&ApiError::coded(
                    fc_core::protocol::error_codes::INVALID_ATTACHMENT,
                    "unreadable",
                )));
                return;
            };
            me.add_bytes(bytes.to_vec()).await;
        });
    }

    /// The rest of adding, once the bytes are in hand (also the self-test's door).
    pub async fn add_bytes(self: &Rc<Self>, bytes: Vec<u8>) {
        let Some(limits) = self.connection.stickers.limits().ok().flatten() else {
            return;
        };
        let held = self
            .connection
            .stickers
            .items()
            .map(|i| i.len())
            .unwrap_or(0);
        let plan = pack_picking::plan_for(&bytes, limits, held);
        let bytes = match plan.what {
            pack_picking::Step::Refuse => {
                self.say_notice(plan.refused.as_ref());
                return;
            }
            pack_picking::Step::Take => bytes,
            pack_picking::Step::Make => match make_sticker(bytes, plan.target).await {
                Ok(png) => png,
                Err(error) => {
                    self.say_notice(Some(&error));
                    return;
                }
            },
        };
        let Some(label) = self.ask_label().await else {
            return;
        };
        if pack_label::too_long(Some(&label)) {
            let shell = Shell::get();
            self.notice
                .set_label(&pack_text::label_too_long(shell.say.as_ref()));
            self.notice.set_visible(true);
            return;
        }
        let added = self.connection.stickers.add(&bytes, Some(&label)).await;
        match added {
            Ok(Ok(added)) => {
                self.say_notice(None);
                let shell = Shell::get();
                self.toasts.add_toast(adw::Toast::new(&pack_text::added(
                    added,
                    shell.say.as_ref(),
                )));
            }
            Ok(Err(error)) => self.say_notice(Some(&error)),
            Err(error) => crate::fc_warn!(
                "pack: the cache refused ({})",
                crate::services::log::store_error(&error)
            ),
        }
        self.redraw();
    }

    /// The few words for a screen reader (optional). `None` when the person cancelled.
    async fn ask_label(&self) -> Option<String> {
        let shell = Shell::get();
        let say = &shell.say;
        let entry = gtk::Entry::builder()
            .placeholder_text(
                say.get("A few words for a screen reader. They are never drawn over the picture."),
            )
            .max_length(i32::try_from(pack_label::MAX_LENGTH * 2).unwrap_or(64))
            .activates_default(true)
            .build();
        let alert = adw::AlertDialog::new(Some(say.get("Add a sticker")), None);
        alert.set_extra_child(Some(&entry));
        alert.add_response("cancel", say.get("Cancel"));
        alert.add_response("add", say.get("Add"));
        alert.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        alert.set_default_response(Some("add"));
        alert.set_close_response("cancel");
        let (sender, receiver) = futures_channel::oneshot::channel::<bool>();
        let sender = std::cell::RefCell::new(Some(sender));
        alert.connect_response(None, move |_, response| {
            if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(response == "add");
            }
        });
        alert.present(Some(&self.dialog));
        match receiver.await {
            Ok(true) => Some(entry.text().to_string()),
            _ => None,
        }
    }
}

/// A still picture made into a sticker: decoded, fitted whole into 512 × 512 (never up), written
/// as PNG with its alpha. A picture with more than one frame is refused ("Animated stickers must
/// be WebP.") rather than flattened.
async fn make_sticker(bytes: Vec<u8>, target: Option<(i32, i32)>) -> Result<Vec<u8>, ApiError> {
    let unreadable = || {
        ApiError::coded(
            fc_core::protocol::error_codes::INVALID_ATTACHMENT,
            "not a picture this machine reads",
        )
    };
    let gbytes = glib::Bytes::from_owned(bytes);
    let stream = gio::MemoryInputStream::from_bytes(&gbytes);
    let pixbuf = gdk_pixbuf::Pixbuf::from_stream_future(&stream)
        .await
        .map_err(|_| unreadable())?;
    if fc_logic::animated_pictures::animation_prep::animated(&gbytes).is_some() {
        return Err(pack_picking::animated());
    }
    let (width, height) =
        target.unwrap_or_else(|| pack_picking::fit(pixbuf.width(), pixbuf.height()));
    let fitted = if (width, height) == (pixbuf.width(), pixbuf.height()) {
        pixbuf
    } else {
        pixbuf
            .scale_simple(width, height, gdk_pixbuf::InterpType::Hyper)
            .ok_or_else(unreadable)?
    };
    fitted.save_to_bufferv("png", &[]).map_err(|_| unreadable())
}

/// Whether this server keeps a pack at all (P4 shows the Family dialog's row only then).
pub fn offered(model: &PackModel) -> bool {
    model.offered().unwrap_or(false)
}
