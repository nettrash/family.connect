//! Files handed to the app (shell-design §8.6, D12 — Windows' `ShareInbox`, the Mac's share
//! extension and `ShareTargetPicker`): `family-connect FILE…`, "Open With" from Files for pictures,
//! videos, sound and PDF (the `.desktop` entry's `%F`, registered by packaging together with this —
//! nettrash's decision, 2026-10-29), a second launch's files. S0's `open` handler presents the main
//! window and hands them to [`Inbox::receive`].
//!
//! **COPIED BEFORE ANYTHING ELSE HAPPENS** (Windows' rule: what was handed over belongs to the
//! process that received it): into `$XDG_DATA_HOME/family-connect/inbox/<one folder per handing>`,
//! at most ten files (one message's composer), each under its own name made safe and unique, and
//! a `.ready` marker written LAST — only a folder whose copy finished is ever offered.
//!
//! **NOTHING IS SENT.** The window asks "Send to" — every chat a file may land in, in the chat
//! list's own order (the family first) and never the assistant's chat, which takes no attachments
//! — and the files land STAGED in the chosen chat's composer (`ChatRoutes::open_chat`, then
//! `ChatRoutes::stage_files`, P1's): the person still presses Send, because handing a file to a
//! family chat is choosing to say something, not having said it. Cancel discards the copies, as
//! does a staging that read them. Until there are chats to choose from (signed out, the gate not
//! on the chats, the video recorder open) the offer waits, and comes when they are.
//!
//! Logs carry counts only — never a file name.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::attachment_files;
use fc_logic::chat_list::ChatListModel;
use fc_text::media::MAX_PER_MESSAGE;

use crate::seams::routes::InboxRoutes;
use crate::shell::Shell;
use crate::windows::main_window::GatePage;

/// The marker a finished copy carries.
pub const READY: &str = ".ready";

/// P2's `InboxRoutes`.
pub struct Inbox {
    me: Weak<Inbox>,
    root: PathBuf,
    /// "Send to" is up.
    choosing: Cell<bool>,
    /// Waiting for the main window to show the chats.
    watching: Cell<bool>,
    /// The dialog up now and its rows by chat (the self-test chooses through it).
    shown: RefCell<Option<Choosing>>,
}

/// "Send to" as it is up.
#[derive(Clone)]
#[cfg_attr(
    not(feature = "selftest"),
    allow(
        dead_code,
        reason = "the attachments self-test chooses a chat through it"
    )
)]
pub struct Choosing {
    pub dialog: adw::Dialog,
    pub rows: Vec<(i64, adw::ActionRow)>,
}

thread_local! {
    static CURRENT: RefCell<Weak<Inbox>> = const { RefCell::new(Weak::new()) };
}

impl Inbox {
    pub fn new(shell: &Rc<Shell>) -> Rc<Inbox> {
        let inbox = Rc::new_cyclic(|me| Inbox {
            me: me.clone(),
            root: shell.paths.inbox_dir(),
            choosing: Cell::new(false),
            watching: Cell::new(false),
            shown: RefCell::new(None),
        });
        CURRENT.with(|current| *current.borrow_mut() = Rc::downgrade(&inbox));
        inbox
    }

    /// The process's inbox (the one `install` put into `Shell::inbox`).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the attachments and chat self-tests read it")
    )]
    pub fn current() -> Option<Rc<Inbox>> {
        CURRENT.with(|current| current.borrow().upgrade())
    }

    /// The folder handings are copied into.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the attachments self-test reads it")
    )]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The "Send to" dialog while it is up (the self-test chooses a chat through it).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(
            dead_code,
            reason = "the attachments and chat self-tests choose through it"
        )
    )]
    pub fn choosing(&self) -> Option<Choosing> {
        self.shown.borrow().clone()
    }

    /// The oldest finished handing: its folder and its files, in name order. Folders that never
    /// finished are left for the next look; finished ones with nothing in them go.
    pub fn peek(&self) -> Option<(PathBuf, Vec<PathBuf>)> {
        peek(&self.root)
    }

    /// Offers the oldest finished handing, now if the chats are there, else as soon as they are.
    pub fn deliver(&self) {
        let Some(me) = self.me.upgrade() else {
            return;
        };
        if self.choosing.get() || self.peek().is_none() {
            return;
        }
        let shell = Shell::get();
        let Some(window) = shell.main_window() else {
            return;
        };
        let ready = shell
            .connection()
            .is_some_and(|connection| connection.session.state().can_chat())
            && window.page() == GatePage::Chats
            && !window.recorder_open();
        if !ready {
            if !self.watching.replace(true) {
                let weak = Rc::downgrade(&me);
                window.on_page_changed().subscribe(move |page| {
                    if *page == GatePage::Chats
                        && let Some(inbox) = weak.upgrade()
                    {
                        // A beat later: the chat list fills in the same turn.
                        glib::idle_add_local_once(move || inbox.deliver());
                    }
                });
            }
            return;
        }
        me.ask();
    }

    /// "Send to" over the main window.
    fn ask(self: &Rc<Self>) {
        let Some((folder, files)) = self.peek() else {
            return;
        };
        let shell = Shell::get();
        let say = &shell.say;
        let Some(connection) = shell.connection() else {
            return;
        };
        let Some(window) = shell.main_window() else {
            return;
        };
        let me_id = {
            let session = Rc::clone(&connection.session);
            move || session.state().me.map_or(0, |me| me.id)
        };
        let rows = ChatListModel::new(Rc::clone(&connection.chats), me_id, shell.say.clone())
            .with_zone(shell.zone.clone())
            .rows(time::OffsetDateTime::now_utc())
            .unwrap_or_default()
            .into_iter()
            .filter(|row| !row.is_assistant())
            .collect::<Vec<_>>();
        if rows.is_empty() {
            discard(&folder);
            return;
        }
        self.choosing.set(true);
        window.present();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .valign(gtk::Align::Start)
            .css_classes(["boxed-list"])
            .build();
        let mut choices = Vec::new();
        for row in &rows {
            let avatar = adw::Avatar::new(32, Some(&row.title), true);
            if let Some(peer) = row.chat.peer_user_id {
                let version = connection
                    .chats
                    .member(peer)
                    .ok()
                    .flatten()
                    .map_or(0, |member| member.avatar_version);
                super::avatars::show(&avatar, peer, version);
            }
            let line = adw::ActionRow::builder()
                .title(&row.title)
                .title_lines(1)
                .activatable(true)
                .build();
            line.add_prefix(&avatar);
            list.append(&line);
            choices.push((row.chat.id, line));
        }
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(420)
            .child(&list)
            .vexpand(true)
            .build();
        let clamp = adw::Clamp::builder()
            .maximum_size(340)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(12)
            .child(&scrolled)
            .build();
        let cancel = gtk::Button::with_label(say.get("Cancel"));
        let header = adw::HeaderBar::builder()
            .show_start_title_buttons(false)
            .show_end_title_buttons(false)
            .build();
        header.pack_start(&cancel);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&clamp));
        // The Mac's sheet: wide enough for a name, tall enough for a family.
        let dialog = adw::Dialog::builder()
            .title(say.get("Send to"))
            .content_width(360)
            .content_height(460)
            .child(&view)
            .build();

        let chosen: Rc<Cell<Option<i64>>> = Rc::new(Cell::new(None));
        for (chat_id, line) in &choices {
            let (chosen, dialog, chat_id) = (Rc::clone(&chosen), dialog.clone(), *chat_id);
            line.connect_activated(move |_| {
                chosen.set(Some(chat_id));
                dialog.close();
            });
        }
        {
            let dialog = dialog.clone();
            cancel.connect_clicked(move |_| {
                dialog.close();
            });
        }
        let weak = Rc::downgrade(self);
        dialog.connect_closed(move |_| {
            let Some(inbox) = weak.upgrade() else {
                return;
            };
            inbox.shown.borrow_mut().take();
            let (folder, files) = (folder.clone(), files.clone());
            match chosen.get() {
                None => {
                    // Sent nowhere: the copies go.
                    crate::fc_info!("inbox: {} file(s) sent nowhere", files.len());
                    discard(&folder);
                    inbox.choosing.set(false);
                    inbox.deliver();
                }
                Some(chat_id) => {
                    glib::spawn_future_local(async move {
                        inbox.stage(chat_id, &folder, files).await;
                    });
                }
            }
        });
        *self.shown.borrow_mut() = Some(Choosing {
            dialog: dialog.clone(),
            rows: choices,
        });
        dialog.present(Some(window.window()));
    }

    /// The chosen chat comes forward with the files staged in its composer; the copies go once the
    /// staging has read them; the next handing, if any, is offered.
    async fn stage(self: &Rc<Self>, chat_id: i64, folder: &Path, files: Vec<PathBuf>) {
        let shell = Shell::get();
        let routes = shell.chat_routes.get();
        routes.open_chat(chat_id);
        let handed: Vec<gio::File> = files.iter().map(gio::File::for_path).collect();
        crate::fc_info!("inbox: {} file(s) staged in chat {chat_id}", handed.len());
        routes.stage_files(chat_id, handed).await;
        discard(folder);
        self.choosing.set(false);
        self.deliver();
    }

    /// Copies `files` into a new folder of the inbox (see the module's comment). Answers how many
    /// were copied.
    pub async fn take_in(&self, files: &[gio::File]) -> usize {
        let folder = self
            .root
            .join(format!("{}-{:08x}", glib::real_time(), glib::random_int()));
        let mut copied = 0;
        for file in files {
            if copied >= MAX_PER_MESSAGE {
                break;
            }
            let kind = file
                .query_info_future(
                    gio::FILE_ATTRIBUTE_STANDARD_TYPE,
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await
                .map(|info| info.file_type());
            if kind.ok() != Some(gio::FileType::Regular) {
                // A folder is not an attachment.
                continue;
            }
            if crate::services::files::private_dir(&folder).is_err() {
                break;
            }
            let name = file
                .basename()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let target = gio::File::for_path(unique(&folder, &name));
            let (copying, _progress) =
                file.copy_future(&target, gio::FileCopyFlags::NONE, glib::Priority::DEFAULT);
            match copying.await {
                Ok(()) => copied += 1,
                Err(error) => {
                    crate::fc_warn!(
                        "inbox: a file could not be copied ({:?})",
                        error.kind::<gio::IOErrorEnum>()
                    );
                }
            }
        }
        if copied > 0 {
            // LAST: a folder without it is a copy that never finished, and is never offered.
            if let Err(error) = std::fs::write(folder.join(READY), b"") {
                crate::fc_warn!("inbox: the copy could not be marked ({})", error.kind());
            }
        } else {
            let _ = std::fs::remove_dir_all(&folder);
        }
        copied
    }
}

impl InboxRoutes for Inbox {
    fn receive(&self, files: &[gio::File]) {
        let Some(me) = self.me.upgrade() else {
            return;
        };
        let files = files.to_vec();
        glib::spawn_future_local(async move {
            let copied = me.take_in(&files).await;
            crate::fc_info!("inbox: {copied} of {} file(s) taken in", files.len());
            me.deliver();
        });
    }
}

/// The oldest finished folder under `root` and its files (see [`Inbox::peek`]).
pub fn peek(root: &Path) -> Option<(PathBuf, Vec<PathBuf>)> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        if !folder.join(READY).is_file() {
            continue;
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && path.file_name().is_some_and(|n| n != READY))
            .collect();
        files.sort();
        if files.is_empty() {
            discard(&folder);
            continue;
        }
        return Some((folder, files));
    }
    None
}

/// A handing that was staged, or that went nowhere: its copies go.
pub fn discard(folder: &Path) {
    if let Err(error) = std::fs::remove_dir_all(folder) {
        crate::fc_warn!("inbox: a handing could not be cleared ({})", error.kind());
    }
}

/// The file's own name, made safe (`attachment_files::safe_file_name`) and unique in `folder` —
/// two files handed over may both be called `image.png`.
pub fn unique(folder: &Path, name: &str) -> PathBuf {
    let safe = attachment_files::safe_file_name(name);
    let path = folder.join(&safe);
    if !path.exists() {
        return path;
    }
    let (stem, extension) = match safe.rfind('.') {
        Some(dot) if dot > 0 => (&safe[..dot], &safe[dot..]),
        _ => (safe.as_str(), ""),
    };
    (2..)
        .map(|n| folder.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new(tag: &str) -> TempFolder {
            let path = std::env::temp_dir().join(format!("fc-inbox-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempFolder(path)
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn two_files_of_one_name_are_both_kept() {
        let temp = TempFolder::new("unique");
        let first = unique(&temp.0, "image.png");
        assert_eq!(first, temp.0.join("image.png"));
        std::fs::write(&first, b"1").unwrap();
        assert_eq!(unique(&temp.0, "image.png"), temp.0.join("image (2).png"));
        std::fs::write(temp.0.join("image (2).png"), b"2").unwrap();
        assert_eq!(unique(&temp.0, "image.png"), temp.0.join("image (3).png"));
        assert_eq!(unique(&temp.0, "../../x"), temp.0.join("x"));
        assert_eq!(unique(&temp.0, ""), temp.0.join("attachment.bin"));
    }

    #[test]
    fn only_a_finished_handing_is_offered_oldest_first() {
        let temp = TempFolder::new("peek");
        assert_eq!(peek(&temp.0), None);
        let unfinished = temp.0.join("100-a");
        std::fs::create_dir_all(&unfinished).unwrap();
        std::fs::write(unfinished.join("a.jpg"), b"x").unwrap();
        assert_eq!(peek(&temp.0), None, "no marker: never offered");
        let later = temp.0.join("300-c");
        std::fs::create_dir_all(&later).unwrap();
        std::fs::write(later.join("c.pdf"), b"x").unwrap();
        std::fs::write(later.join(READY), b"").unwrap();
        let older = temp.0.join("200-b");
        std::fs::create_dir_all(&older).unwrap();
        std::fs::write(older.join("b2.png"), b"x").unwrap();
        std::fs::write(older.join("b1.png"), b"x").unwrap();
        std::fs::write(older.join(READY), b"").unwrap();
        let (folder, files) = peek(&temp.0).unwrap();
        assert_eq!(folder, older);
        assert_eq!(files, vec![older.join("b1.png"), older.join("b2.png")]);
        discard(&older);
        assert_eq!(peek(&temp.0).unwrap().0, later);
        let empty = temp.0.join("150-e");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join(READY), b"").unwrap();
        assert_eq!(peek(&temp.0).unwrap().0, later, "an empty handing goes");
        assert!(!empty.exists());
    }
}
