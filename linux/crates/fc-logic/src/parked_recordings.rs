//! The voice messages that were not sent (`ParkedRecordings.cs`; docs/audio-video-messages-
//! 2026-10-04.md, S2.8), kept on this device per account — the file with its length, its reply and
//! its caption — so that what an interruption stopped survives the app being closed, and can be
//! seen, sent and deleted.
//!
//! THIS MODULE TOUCHES THE FILE SYSTEM, as `folder_media_store` does and for the same reason: the C#
//! class IS the file-backed store, and porting it over `std::fs` keeps its rules — and its tests, in
//! a temporary directory — here. The shell only chooses the root: `$XDG_STATE_HOME/family-connect/
//! parked/` (never `~/.cache`, which tools purge). Every call is a short synchronous file operation
//! on the GLib main context; the C#'s lock (a wipe after a sign-out ran on whatever thread noticed
//! it) has nothing to guard on one thread.
//!
//! ONE FOLDER PER RECORDING, AND IT LANDS WHOLE OR NOT AT ALL. The bytes and what is known about
//! them are written under a part name and moved into place, so a crash mid-write can never leave an
//! entry that reads as a shorter recording, or as one whose caption is missing; a part left behind
//! is no entry, and is swept at launch with anything else no entry names.
//!
//! PER ACCOUNT, PER SERVER. Each account's recordings are under a folder named for the server and
//! the account together ([`owner_key`]), so nobody else signed in on this device — or the same id on
//! another server — ever sees them. They are WIPED whenever the session ends, as the cache and its
//! outbox are ([`ParkedRecordings::wipe_all`]): a recording belongs to the person who made it.
//!
//! AN ID IS NOT A PATH. Anything but the 32 lowercase hex digits this store makes is not read, and
//! not deleted.
//!
//! WHAT THE DISK REFUSES IS HELD IN MEMORY (S4: the disk fills), for as long as this store lives —
//! the app's run — and listed, played, sent and deleted exactly as a written one
//! ([`ParkedRecordings::hold`]): still its own row with its own reply and caption, never carried by
//! another Send (S2.8). [`ParkedRecordings::write_held`] tries the disk again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use fc_core::protocol::send_pipeline::uuid_v4;
use serde_json::{Map, Value};
use time::OffsetDateTime;

use crate::media_outbox::StagedMedia;
use crate::rich_drawing::sha256;
use crate::voice_notes;

const PART: &str = ".part";
const BYTES_FILE: &str = "voice.m4a";
const META_FILE: &str = "meta.json";

/// The earliest and latest `parked_at_ms` a .NET `DateTimeOffset` can hold (years 1 to 9999) — a
/// time outside them is nonsense, and no entry, as the C#'s `FromUnixTimeMilliseconds` threw.
const EARLIEST_MS: i64 = -62_135_596_800_000;
const LATEST_MS: i64 = 253_402_300_799_999;

/// One voice message that was not sent (S2.8): the chat it belongs to, how long it is, the reply it
/// was recorded under and its caption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkedRecording {
    pub id: String,
    pub chat_id: i64,
    pub duration_ms: i32,
    pub reply_to_message_id: Option<i64>,
    pub caption: Option<String>,
    pub parked_at: OffsetDateTime,
}

/// The voice messages one account on one server has waiting.
#[derive(Debug)]
pub struct ParkedRecordings {
    folder: PathBuf,
    /// The recordings the disk refused, by id, with their bytes.
    held: RefCell<HashMap<String, (ParkedRecording, Vec<u8>)>>,
}

/// The folder name for one account on one server: never the address or the id themselves — the
/// first 16 bytes of SHA-256 over `"<server>\n<account id>"`, in lowercase hex. `server` is the
/// server's address as the session holds it (the C# passed `Uri.AbsoluteUri`, which always ends a
/// bare host with `/`); the shell must pass the same spelling every time, or a recording parked
/// under one spelling is not found under the other.
pub fn owner_key(server: &str, account_id: i64) -> String {
    let owner = format!("{server}\n{account_id}");
    sha256(owner.as_bytes())[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl ParkedRecordings {
    /// The store over `folder` — one account's.
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        ParkedRecordings {
            folder: folder.into(),
            held: RefCell::new(HashMap::new()),
        }
    }

    /// The store for one account on one server, under `root` (the C#'s `For`).
    pub fn for_account(root: &Path, server: &str, account_id: i64) -> Self {
        ParkedRecordings::new(root.join(owner_key(server, account_id)))
    }

    /// The folder this store keeps its recordings in.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Keep a recording that was not sent. Answers the entry, or the error when it could not be
    /// written — the caller keeps the recording some other way rather than lose it
    /// ([`hold`](Self::hold)) — leaving no part behind.
    pub fn park(
        &self,
        chat_id: i64,
        bytes: &[u8],
        duration_ms: i32,
        reply_to: Option<i64>,
        caption: Option<&str>,
        at: OffsetDateTime,
    ) -> io::Result<ParkedRecording> {
        let entry = new_entry(chat_id, duration_ms, reply_to, caption, at);
        self.write(&entry, bytes)?;
        Ok(entry)
    }

    /// Keep a recording the disk refused, in memory (S4: the disk fills): the same entry a park
    /// would have made — its chat, length, reply and caption — listed, played, sent and deleted
    /// like any other, until the session ends or the app does.
    pub fn hold(
        &self,
        chat_id: i64,
        bytes: &[u8],
        duration_ms: i32,
        reply_to: Option<i64>,
        caption: Option<&str>,
        at: OffsetDateTime,
    ) -> ParkedRecording {
        let entry = new_entry(chat_id, duration_ms, reply_to, caption, at);
        self.held
            .borrow_mut()
            .insert(entry.id.clone(), (entry.clone(), bytes.to_vec()));
        entry
    }

    /// Whether a recording here is held only in memory — gone with the app unless the disk takes it
    /// ([`write_held`](Self::write_held)).
    pub fn holds_in_memory(&self) -> bool {
        !self.held.borrow().is_empty()
    }

    /// The disk, tried again for every recording held in memory — what a real close does before it
    /// lets the app go. Each one written moves from memory to disk under the same id, so a row drawn
    /// for it is still its row; one the disk still refuses stays held. Answers how many are still
    /// held.
    pub fn write_held(&self) -> usize {
        let held: Vec<(String, ParkedRecording, Vec<u8>)> = self
            .held
            .borrow()
            .iter()
            .map(|(id, (entry, bytes))| (id.clone(), entry.clone(), bytes.clone()))
            .collect();
        for (id, entry, bytes) in held {
            // Still refused: it stays in memory, its row unchanged.
            if self.write(&entry, &bytes).is_ok() {
                self.held.borrow_mut().remove(&id);
            }
        }
        self.held.borrow().len()
    }

    /// Every recording held in memory, gone: the session ended, and they go with everything else
    /// recorded and not sent (S4).
    pub fn forget_held(&self) {
        self.held.borrow_mut().clear();
    }

    /// Every recording this account has waiting, oldest first — the written ones and those held in
    /// memory.
    pub fn all(&self) -> Vec<ParkedRecording> {
        let mut found: Vec<ParkedRecording> = self
            .held
            .borrow()
            .values()
            .map(|(entry, _)| entry.clone())
            .collect();
        if self.folder.is_dir()
            && let Ok(listing) = std::fs::read_dir(&self.folder)
        {
            for item in listing.flatten() {
                let path = item.path();
                if path.is_dir()
                    && let Some(entry) = entry_at(&path)
                {
                    found.push(entry);
                }
            }
        }
        found.sort_by(|a, b| a.parked_at.cmp(&b.parked_at).then_with(|| a.id.cmp(&b.id)));
        found
    }

    /// One chat's recordings waiting, oldest first.
    pub fn of(&self, chat_id: i64) -> Vec<ParkedRecording> {
        self.all()
            .into_iter()
            .filter(|entry| entry.chat_id == chat_id)
            .collect()
    }

    /// Whether a chat holds a recording that was not sent — the microphone's "send or delete it
    /// first" (S1.3).
    pub fn any(&self, chat_id: i64) -> bool {
        self.all().iter().any(|entry| entry.chat_id == chat_id)
    }

    /// The recording as the voice note it is, ready to stage for a send — its duration its
    /// identity, and no name, as [`voice_notes::staged`] makes one — or `None` when it is gone or
    /// cannot be read.
    pub fn staged(&self, entry: &ParkedRecording) -> Option<StagedMedia> {
        if !is_id(&entry.id) {
            return None;
        }
        if let Some((kept, bytes)) = self.held.borrow().get(&entry.id) {
            return Some(StagedMedia {
                duration_ms: Some(kept.duration_ms),
                ..StagedMedia::new("audio", voice_notes::MIME, bytes.clone())
            });
        }
        let bytes = std::fs::read(self.folder.join(&entry.id).join(BYTES_FILE)).ok()?;
        Some(StagedMedia {
            duration_ms: Some(entry.duration_ms),
            ..StagedMedia::new("audio", voice_notes::MIME, bytes)
        })
    }

    /// Gone — sent from its row, or deleted. What it knows goes FIRST, so a recording whose file is
    /// held open for a moment is no longer an entry, and the sweep takes the rest. Answers whether
    /// it is gone.
    pub fn remove(&self, entry: &ParkedRecording) -> bool {
        if !is_id(&entry.id) {
            return false;
        }
        if self.held.borrow_mut().remove(&entry.id).is_some() {
            return true;
        }
        let path = self.folder.join(&entry.id);
        if path.is_dir() {
            match std::fs::remove_file(path.join(META_FILE)) {
                // .NET's `File.Delete` of a file that is not there is no error.
                Err(error) if error.kind() != io::ErrorKind::NotFound => return false,
                _ => {}
            }
        }
        try_delete(&path);
        true
    }

    /// At launch: anything in this account's folder no entry names — a part a crash left, an entry
    /// missing half of itself — goes.
    pub fn sweep(&self) {
        if !self.folder.is_dir() {
            return;
        }
        let Ok(listing) = std::fs::read_dir(&self.folder) else {
            return;
        };
        for item in listing.flatten() {
            let path = item.path();
            if !path.is_dir() || entry_at(&path).is_none() {
                try_delete(&path);
            }
        }
    }

    /// Every account's recordings, gone: what a session ending does — signed out, expired, the
    /// family left, the account deleted — and what pointing the app at another server does.
    pub fn wipe_all(root: &Path) {
        if !root.is_dir() {
            return;
        }
        if std::fs::remove_dir_all(root).is_err() {
            // Something held open: whatever is left is unmade as an entry, recording by recording,
            // and swept later.
            remove_metas(root);
        }
    }

    /// One recording onto the disk, under a part name moved into place — whole, or not at all and
    /// the error answered.
    fn write(&self, entry: &ParkedRecording, bytes: &[u8]) -> io::Result<()> {
        let final_path = self.folder.join(&entry.id);
        let part = self.folder.join(format!("{}{PART}", entry.id));
        let written = (|| {
            std::fs::create_dir_all(&part)?;
            std::fs::write(part.join(BYTES_FILE), bytes)?;
            std::fs::write(part.join(META_FILE), meta(entry))?;
            if final_path.exists() {
                // `Directory.Move` never replaces; `rename` would replace an empty folder.
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            std::fs::rename(&part, &final_path)
        })();
        if written.is_err() {
            try_delete(&part);
        }
        written
    }
}

fn new_entry(
    chat_id: i64,
    duration_ms: i32,
    reply_to: Option<i64>,
    caption: Option<&str>,
    at: OffsetDateTime,
) -> ParkedRecording {
    ParkedRecording {
        // `Guid.NewGuid().ToString("N")`.
        id: uuid_v4().replace('-', ""),
        chat_id,
        duration_ms: duration_ms.max(0),
        reply_to_message_id: reply_to,
        caption: caption
            .filter(|words| !words.trim().is_empty())
            .map(str::to_string),
        // To the millisecond, as the entry is written down: what the park answers is what every
        // later read finds (the C# kept the finer ticks in the answer only).
        parked_at: whole_milliseconds(at),
    }
}

fn whole_milliseconds(at: OffsetDateTime) -> OffsetDateTime {
    let nanos = at.unix_timestamp_nanos();
    let truncated = nanos.div_euclid(1_000_000) * 1_000_000;
    OffsetDateTime::from_unix_timestamp_nanos(truncated).unwrap_or(at)
}

/// The entry a folder holds, or `None` when it holds none: not an id, no bytes, or what it knows
/// cannot be read or makes no sense.
fn entry_at(path: &Path) -> Option<ParkedRecording> {
    let id = path.file_name()?.to_str()?;
    if !is_id(id) || !path.join(BYTES_FILE).is_file() {
        return None;
    }
    let text = std::fs::read(path.join(META_FILE)).ok()?;
    let root: Value = serde_json::from_slice(&text).ok()?;
    let root = root.as_object()?;
    let chat_id = whole(root, "chat_id").filter(|&chat| chat > 0)?;
    let duration = whole(root, "duration_ms")
        .filter(|&duration| (0..=i64::from(i32::MAX)).contains(&duration))?;
    let at = whole(root, "parked_at_ms").filter(|&at| (EARLIEST_MS..=LATEST_MS).contains(&at))?;
    let reply = whole(root, "reply_to_message_id").filter(|&reply| reply > 0);
    let caption = root
        .get("caption")
        .and_then(Value::as_str)
        .filter(|words| !words.trim().is_empty())
        .map(str::to_string);
    Some(ParkedRecording {
        id: id.to_string(),
        chat_id,
        duration_ms: duration as i32,
        reply_to_message_id: reply,
        caption,
        parked_at: OffsetDateTime::from_unix_timestamp_nanos(i128::from(at) * 1_000_000).ok()?,
    })
}

/// A number written as a whole number (`JsonElement.TryGetInt64`): `2000.0`, `2e3` and a string are
/// not.
fn whole(root: &Map<String, Value>, name: &str) -> Option<i64> {
    root.get(name)?.as_i64()
}

/// What is known about a recording, as JSON: numbers as numbers — a time as milliseconds since
/// 1970, never as text a reader's culture could spell differently.
fn meta(entry: &ParkedRecording) -> Vec<u8> {
    let mut object = Map::new();
    object.insert("chat_id".to_string(), Value::from(entry.chat_id));
    object.insert("duration_ms".to_string(), Value::from(entry.duration_ms));
    if let Some(reply) = entry.reply_to_message_id {
        object.insert("reply_to_message_id".to_string(), Value::from(reply));
    }
    if let Some(caption) = &entry.caption {
        object.insert("caption".to_string(), Value::from(caption.as_str()));
    }
    let at = entry.parked_at.unix_timestamp_nanos().div_euclid(1_000_000);
    object.insert(
        "parked_at_ms".to_string(),
        Value::from(i64::try_from(at).unwrap_or(0)),
    );
    serde_json::to_vec(&Value::Object(object)).unwrap_or_default()
}

fn is_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn try_delete(entry: &Path) {
    // A link is removed as the link it is, never followed into what it points at.
    let folder = std::fs::symlink_metadata(entry).is_ok_and(|meta| meta.file_type().is_dir());
    // Held open for a moment, or gone already: the next sweep comes back to it.
    let _ = if folder {
        std::fs::remove_dir_all(entry)
    } else {
        std::fs::remove_file(entry)
    };
}

/// Every `meta.json` under `folder`, removed — what is left of a wipe that could not delete it all.
fn remove_metas(folder: &Path) {
    let Ok(listing) = std::fs::read_dir(folder) else {
        return;
    };
    for item in listing.flatten() {
        let path = item.path();
        let is_dir = std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_dir());
        if is_dir {
            remove_metas(&path);
        } else if item.file_name() == META_FILE {
            try_delete(&path);
        }
    }
}
