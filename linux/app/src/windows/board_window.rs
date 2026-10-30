//! The Board window (shell-design §11, §3.2; the Mac's `MacBoardView` in its own `Window`): an
//! `adw::ApplicationWindow` (900 × 620, at least 480 × 360 — "a sheet cannot be resized", and a
//! board cut at a fixed size hides notes) holding the wall, with the Mac's four header buttons —
//! Add Note (Ctrl+N), Add an event, Add List, Pin a Photo — and their tooltips.
//!
//! ONE window, showing the family SHOWN: `app.open-board(x)` has switched the session to `x`
//! before it lands here (S0's route), every write goes to the family shown (`ApiClient`'s scope),
//! and a switch while the window is open redraws it for the other family — the Mac's board window
//! follows `currentFamilyKey` the same way.
//!
//! THE BADGE MOVES ONLY IN FRONT OF SOMEBODY: the marks are moved (`BoardModel::shown`) while the
//! window is the active one — shown, or changed while shown — never for a window restored behind
//! everything (the Mac's presence rule).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::photo_pinning::PhotoPinning;
use fc_text::board::Kind;

use crate::board::{self, sheet, wall::Wall};
use crate::services::log;
use crate::settings::ui;
use crate::shell::{Connection, Shell};

/// The window's default size (the Mac's).
pub const DEFAULT_SIZE: (i32, i32) = (900, 620);
/// The smallest it may be.
pub const MINIMUM_SIZE: (i32, i32) = (480, 360);
/// Ctrl+N, the Mac's ⌘N.
const ADD_NOTE_ACCELS: &[&str] = &["<Control>n"];
/// The longest edge a pinned photo is sent at, and its small copy's.
const PHOTO_EDGE: i32 = 2048;
const PREVIEW_EDGE: i32 = 320;

thread_local! {
    static OPEN: RefCell<Option<Rc<BoardWindow>>> = const { RefCell::new(None) };
}

/// `app.open-board`: the Board window, built once and presented (the session already shows the
/// family asked for).
pub fn open(family: Option<i64>) -> Rc<BoardWindow> {
    let window = current().unwrap_or_else(|| {
        let window = BoardWindow::build(&Shell::get());
        OPEN.with(|open| *open.borrow_mut() = Some(Rc::clone(&window)));
        window
    });
    crate::fc_info!("board: opened (family given: {})", family.is_some());
    window.window.present();
    window.refresh();
    window
}

/// The window, when it was ever opened.
pub fn current() -> Option<Rc<BoardWindow>> {
    OPEN.with(|open| open.borrow().clone())
}

/// The Board window (see the module's comment).
pub struct BoardWindow {
    pub window: adw::ApplicationWindow,
    pub title: adw::WindowTitle,
    pub wall: Rc<Wall>,
    pub toasts: adw::ToastOverlay,
    pinning: RefCell<Option<Rc<PhotoPinning>>>,
}

impl std::fmt::Debug for BoardWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoardWindow").finish_non_exhaustive()
    }
}

impl BoardWindow {
    fn build(shell: &Rc<Shell>) -> Rc<BoardWindow> {
        let say = Rc::clone(&shell.say);
        let window = adw::ApplicationWindow::builder()
            .application(&shell.app)
            .title(say.get("Board"))
            .default_width(DEFAULT_SIZE.0)
            .default_height(DEFAULT_SIZE.1)
            .hide_on_close(true)
            .build();
        window.set_size_request(MINIMUM_SIZE.0, MINIMUM_SIZE.1);
        let title = adw::WindowTitle::new(say.get("Board"), "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));
        let toasts = adw::ToastOverlay::new();
        let wall = Wall::new(&toasts);
        toasts.set_child(Some(&wall.root));
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&toasts));
        window.set_content(Some(&view));

        let buttons: [(&str, &str, Kind); 3] = [
            ("add-note", "list-add-symbolic", Kind::Text),
            ("add-event", "x-office-calendar-symbolic", Kind::Event),
            ("add-list", "view-list-bullet-symbolic", Kind::Tasks),
        ];
        let tooltips = [
            say.get("Add a note"),
            say.get("Add an event"),
            say.get("Add a task list"),
        ];
        let labels = [
            say.get("Add Note"),
            say.get("Add an event"),
            say.get("Add List"),
        ];
        for (at, (name, icon, kind)) in buttons.into_iter().enumerate() {
            let action = gio::SimpleAction::new(name, None);
            action.connect_activate(move |_, _| {
                if let Some(board) = current() {
                    board.compose(kind);
                }
            });
            window.add_action(&action);
            let button = gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tooltips[at])
                .action_name(format!("win.{name}"))
                .build();
            button.update_property(&[gtk::accessible::Property::Label(labels[at])]);
            header.pack_start(&button);
        }
        shell
            .app
            .set_accels_for_action("win.add-note", ADD_NOTE_ACCELS);
        let pin = gtk::Button::builder()
            .icon_name("insert-image-symbolic")
            .tooltip_text(say.get("Pin a photo"))
            .build();
        pin.update_property(&[gtk::accessible::Property::Label(say.get("Pin a Photo"))]);
        pin.connect_clicked(|_| {
            if let Some(board) = current() {
                board.pick_photo();
            }
        });
        header.pack_start(&pin);

        let board = Rc::new(BoardWindow {
            window,
            title,
            wall,
            toasts,
            pinning: RefCell::new(None),
        });
        board.wall.set_on_edit(|note| {
            if let Some(board) = current() {
                sheet::edit(&board.window, note, || {
                    if let Some(board) = current() {
                        board.refresh();
                    }
                });
            }
        });
        board.follow(shell);
        board
    }

    /// Redraws on what changes the wall: live frames, a resync, a family switch, a sign-out, the
    /// window coming to the front (which moves the marks).
    fn follow(self: &Rc<Self>, shell: &Rc<Shell>) {
        let follow_connection = |connection: &Rc<Connection>| {
            connection
                .router
                .on_board_changed()
                .subscribe(|_| refresh_open());
            connection.live.on_resynced().subscribe(|_| refresh_open());
            connection
                .session
                .on_changed()
                .subscribe(|_| refresh_open());
        };
        if let Some(connection) = shell.connection() {
            follow_connection(&connection);
        }
        shell.on_connection_changed().subscribe(move |_| {
            if let Some(connection) = Shell::get().connection() {
                follow_connection(&connection);
            }
            refresh_open();
        });
        self.window.connect_is_active_notify(|window| {
            if window.is_active() {
                refresh_open();
            }
        });
    }

    /// Draws the wall again, names the family, and — in front of somebody — moves the marks.
    pub fn refresh(&self) {
        let shell = Shell::get();
        let Some(connection) = shell.connection() else {
            self.window.set_visible(false);
            return;
        };
        let state = connection.session.state();
        if !state.can_chat() {
            // Signed out, or out of every family: no board to show.
            self.window.set_visible(false);
            return;
        }
        let name = state
            .family
            .as_ref()
            .map(|family| family.name.clone())
            .unwrap_or_default();
        self.title.set_subtitle(&name);
        self.window
            .set_title(Some(&format!("{} — {name}", shell.say.get("Board"))));
        self.wall.redraw();
        if self.window.is_visible() && self.window.is_active() {
            if let Err(error) = board::model(&connection).shown() {
                crate::fc_warn!("board: the cache refused ({})", log::store_error(&error));
            }
            board::refresh_badges();
        }
    }

    /// The sheet for a new note of `kind`.
    pub fn compose(&self, kind: Kind) {
        sheet::compose(&self.window, kind, || {
            if let Some(board) = current() {
                board.refresh();
            }
        });
    }

    /// Pin a Photo: a picture chosen with the portal's file chooser (images only), made a JPEG of
    /// at most 2048 px with a 320 px small copy, then `PhotoPinning` uploads and pins it.
    fn pick_photo(self: &Rc<Self>) {
        let shell = Shell::get();
        let say: &dyn StringCatalog = shell.say.as_ref();
        let filter = gtk::FileFilter::new();
        filter.add_mime_type("image/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(say.get("Pin a Photo"))
            .filters(&filters)
            .default_filter(&filter)
            .modal(true)
            .build();
        let weak = Rc::downgrade(self);
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |answer| {
                let (Ok(file), Some(board)) = (answer, weak.upgrade()) else {
                    return;
                };
                board.pin(&file);
            },
        );
    }

    fn pin(self: &Rc<Self>, file: &gio::File) {
        let shell = Shell::get();
        let Some(connection) = shell.connection() else {
            return;
        };
        let say = Rc::clone(&shell.say);
        let Some(photo) = file.path().and_then(|path| prepared_photo(&path)) else {
            ui::toast(&self.toasts, say.get("Couldn't read that photo."));
            return;
        };
        let pinning = self
            .pinning
            .borrow_mut()
            .get_or_insert_with(|| {
                Rc::new(PhotoPinning::new(
                    Rc::clone(&connection.api),
                    Rc::clone(&connection.board),
                    shell.clock.clone(),
                    shell.spawn.clone(),
                ))
            })
            .clone();
        let at = PhotoPinning::scattered(glib::random_double);
        let weak = Rc::downgrade(self);
        ui::spawn(async move {
            let outcome = pinning.pin(&photo, at, "yellow").await;
            let Some(board) = weak.upgrade() else {
                return;
            };
            let shell = Shell::get();
            let say: &dyn StringCatalog = shell.say.as_ref();
            match outcome {
                Ok(outcome) if outcome.busy => {
                    ui::toast(&board.toasts, &PhotoPinning::still_pinning_text(say));
                }
                Ok(outcome) => {
                    if let Some(error) = outcome.error {
                        crate::fc_info!("board: pin refused ({})", error.code);
                        ui::toast(&board.toasts, &PhotoPinning::failure_text(&error, say));
                    }
                    board.refresh();
                }
                Err(error) => {
                    crate::fc_warn!("board: the cache refused ({})", log::store_error(&error))
                }
            }
        });
    }
}

/// Redraws the board window when it is open and shown.
fn refresh_open() {
    if let Some(board) = current()
        && board.window.is_visible()
    {
        board.refresh();
    }
}

/// A picture file as the board pins it: upright (its EXIF orientation applied), a JPEG of at most
/// [`PHOTO_EDGE`] on its longest side, and a [`PREVIEW_EDGE`] small copy.
fn prepared_photo(path: &std::path::Path) -> Option<fc_logic::media_outbox::StagedMedia> {
    use gtk::gdk_pixbuf::Pixbuf;
    let full = Pixbuf::from_file_at_scale(path, PHOTO_EDGE, PHOTO_EDGE, true).ok()?;
    let full = full.apply_embedded_orientation().unwrap_or(full);
    let bytes = full.save_to_bufferv("jpeg", &[("quality", "85")]).ok()?;
    let (width, height) = (full.width(), full.height());
    let scale = f64::from(PREVIEW_EDGE) / f64::from(width.max(height).max(1));
    let preview = if scale < 1.0 {
        full.scale_simple(
            ((f64::from(width) * scale).round() as i32).max(1),
            ((f64::from(height) * scale).round() as i32).max(1),
            gtk::gdk_pixbuf::InterpType::Bilinear,
        )
        .and_then(|small| small.save_to_bufferv("jpeg", &[("quality", "80")]).ok())
    } else {
        Some(bytes.clone())
    };
    Some(fc_logic::media_outbox::StagedMedia {
        kind: "photo".to_string(),
        mime: "image/jpeg".to_string(),
        bytes,
        width: Some(width),
        height: Some(height),
        preview,
        ..fc_logic::media_outbox::StagedMedia::default()
    })
}
