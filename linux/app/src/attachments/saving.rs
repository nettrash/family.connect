//! Save… and Open for an attachment (shell-design §8.1, §8.2): the ORIGINAL's bytes through the
//! `AttachmentCache` — never the preview a bubble draws — then `gtk::FileDialog::save_future` with
//! the attachment's name suggested (`attachment_files::file_name`, made safe), or
//! `gtk::FileLauncher` on a copy under its real name in the cache's `opened/` folder.
//!
//! A failure is SAID, never swallowed (the Mac's `saveFailure`): the caller shows the sentence this
//! answers — in the viewer as a toast, in a bubble as an alert. A dismissed dialog is no failure.

use std::path::{Path, PathBuf};

use adw::prelude::*;
use fc_core::protocol::AttachmentDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::attachment_files;

use crate::shell::Shell;

/// Where opened copies are written, under the cache folder: one folder per attachment, so two files
/// called `scan.pdf` never meet.
pub const OPENED_FOLDER: &str = "opened";

/// How a save or an open went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// The person dismissed the dialog: nothing to say.
    Dismissed,
    /// The sentence to show.
    Failed(String),
}

/// The original's bytes, or the sentence for a download that failed.
async fn original(attachment: &AttachmentDto) -> Result<Vec<u8>, String> {
    let shell = Shell::get();
    let say = &shell.say;
    let Some(connection) = shell.connection() else {
        return Err(say.get("The file could not be downloaded.").to_string());
    };
    match connection.attachments.bytes(attachment, false).await {
        Ok(bytes) => Ok(bytes),
        Err(error) => {
            crate::fc_info!(
                "save: attachment {} not fetched ({})",
                attachment.id,
                error.code
            );
            Err(say.get("The file could not be downloaded.").to_string())
        }
    }
}

/// Save… : the original, written where the person chooses.
pub async fn save(parent: Option<gtk::Window>, attachment: &AttachmentDto) -> Outcome {
    let bytes = match original(attachment).await {
        Ok(bytes) => bytes,
        Err(sentence) => return Outcome::Failed(sentence),
    };
    let dialog = gtk::FileDialog::builder()
        .modal(true)
        .initial_name(suggested_name(attachment))
        .build();
    let file = match dialog.save_future(parent.as_ref()).await {
        Ok(file) => file,
        Err(_) => return Outcome::Dismissed,
    };
    write(&file, bytes).await
}

/// Writes `bytes` to `file` whole (`replace_contents`, which renames into place).
pub async fn write(file: &gio::File, bytes: Vec<u8>) -> Outcome {
    let written = file
        .replace_contents_future(
            bytes,
            None,
            false,
            gio::FileCreateFlags::REPLACE_DESTINATION,
        )
        .await;
    match written {
        Ok(_) => Outcome::Done,
        Err((_, error)) => {
            crate::fc_warn!(
                "save: the write failed ({:?})",
                error.kind::<gio::IOErrorEnum>()
            );
            let shell = Shell::get();
            Outcome::Failed(shell.say.get("Couldn't save that file").to_string())
        }
    }
}

/// Open: a copy under its real name, handed to whatever opens that kind of file here.
pub async fn open(parent: Option<gtk::Window>, attachment: &AttachmentDto) -> Outcome {
    let bytes = match original(attachment).await {
        Ok(bytes) => bytes,
        Err(sentence) => return Outcome::Failed(sentence),
    };
    let shell = Shell::get();
    let folder = opened_folder(shell.paths.cache_dir(), attachment.id);
    if let Err(error) = crate::services::files::private_dir(&folder) {
        crate::fc_warn!("open: no folder for the copy ({})", error.kind());
        return Outcome::Failed(shell.say.get("Couldn't save that file").to_string());
    }
    let file = gio::File::for_path(folder.join(suggested_name(attachment)));
    if let Outcome::Failed(sentence) = write(&file, bytes).await {
        return Outcome::Failed(sentence);
    }
    let launcher = gtk::FileLauncher::new(Some(&file));
    match launcher.launch_future(parent.as_ref()).await {
        Ok(()) => Outcome::Done,
        Err(error) => {
            // Nothing opens that kind of file, or the portal said no: the copy stays for Save….
            crate::fc_info!(
                "open: not launched ({:?})",
                error.kind::<gtk::DialogError>()
            );
            Outcome::Dismissed
        }
    }
}

/// The name a download is offered under: its own, or one made from its kind and type — never a
/// path, never a character a USB stick refuses (`attachment_files::safe_file_name`).
pub fn suggested_name(attachment: &AttachmentDto) -> String {
    attachment_files::safe_file_name(&attachment_files::file_name(attachment))
}

/// `<cache>/opened/<id>`.
pub fn opened_folder(cache_dir: &Path, attachment_id: i64) -> PathBuf {
    cache_dir
        .join(OPENED_FOLDER)
        .join(attachment_id.max(0).to_string())
}

/// Shows `sentence` as an alert over `parent` (a bubble's failure: there is no toast there).
pub fn alert(parent: Option<&gtk::Widget>, sentence: &str) {
    let shell = Shell::get();
    let say = &shell.say;
    let dialog = adw::AlertDialog::new(Some(say.get("Couldn't save that file")), Some(sentence));
    dialog.add_response("ok", say.get("OK"));
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("ok");
    dialog.present(parent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_suggested_name_is_never_a_path() {
        let mut file = AttachmentDto::new(9, "file");
        file.name = Some("../../etc/passwd".into());
        assert_eq!(suggested_name(&file), "passwd");
        file.name = Some("CON.txt".into());
        assert_eq!(suggested_name(&file), "_CON.txt");
        file.name = None;
        file.mime = Some("image/png".into());
        assert_eq!(suggested_name(&file), "file-9.png");
    }

    #[test]
    fn opened_copies_live_under_the_cache_one_folder_each() {
        assert_eq!(
            opened_folder(Path::new("/c/family-connect"), 42),
            PathBuf::from("/c/family-connect/opened/42")
        );
        assert_eq!(
            opened_folder(Path::new("/c"), -3),
            PathBuf::from("/c/opened/0")
        );
    }
}
