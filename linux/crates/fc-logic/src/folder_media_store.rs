//! A send's bytes on disk while the send is under way: one folder per handle, holding the bytes, the
//! preview and what the server has to be told (`FolderMediaStore.cs`).
//!
//! THIS MODULE TOUCHES THE FILE SYSTEM, which nothing else in `fc-logic` does: the C# class IS the
//! file-backed [`MediaStore`], and porting it over `std::fs` keeps its rules — and its tests, in a
//! temporary directory — here rather than in the shell. The shell only chooses the folder:
//! `$XDG_STATE_HOME/family-connect/media/` (never `~/.cache`, which tools purge — the outbox that
//! names these handles lives in the state directory too). Every call is a short synchronous file
//! operation on the GLib main context, as every store call is.
//!
//! A STAGE LANDS WHOLE OR NOT AT ALL. It is written under a part name and moved into place, so a
//! crash mid-write can never leave a handle that reads as a shorter photo.
//!
//! A HANDLE IS PINNED FROM THE MOMENT IT IS MADE UNTIL ITS ROW IS QUEUED. The sweep keeps what the
//! outbox names, and a flush can sweep between "written" and "queued" — the one moment the outbox
//! names nothing (on one thread, whenever anything is awaited in between). Pinned handles are never
//! swept.
//!
//! A HANDLE IS NOT A PATH. It comes back out of the cache, and anything but the 32 lowercase hex
//! digits this store makes is not read, and not deleted.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::media_outbox::{MediaStore, StagedMedia};

const PART: &str = ".part";

/// The file-backed [`MediaStore`].
#[derive(Debug)]
pub struct FolderMediaStore {
    folder: PathBuf,
    pinned: RefCell<HashSet<String>>,
    /// Made handles, so two in one process can never be the same.
    made: Cell<u64>,
}

impl FolderMediaStore {
    /// A store over `folder`, which is made the first time something is staged.
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        FolderMediaStore {
            folder: folder.into(),
            pinned: RefCell::new(HashSet::new()),
            made: Cell::new(0),
        }
    }

    /// Where the store keeps its folders.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Write one send's media, pinned, and answer its handle. [`release`](Self::release) the
    /// handle once its row is queued.
    ///
    /// A place whose latitude or longitude is not a number is refused (`InvalidInput`) before
    /// anything is written: JSON has no NaN, and the C#'s writer threw on one — here as there the
    /// composer learns at once, rather than queueing a place that would read back without its
    /// numbers and fail as missing media later.
    pub fn stage(&self, media: &StagedMedia) -> io::Result<String> {
        let meta = meta(media)?;
        let handle = self.new_handle();
        self.pinned.borrow_mut().insert(handle.clone());
        let final_path = self.folder.join(&handle);
        let part = self.folder.join(format!("{handle}{PART}"));
        let written = (|| {
            std::fs::create_dir_all(&part)?;
            std::fs::write(part.join("bytes"), &media.bytes)?;
            if let Some(preview) = media.preview.as_deref().filter(|p| !p.is_empty()) {
                std::fs::write(part.join("preview"), preview)?;
            }
            std::fs::write(part.join("meta.json"), &meta)?;
            if final_path.exists() {
                // `Directory.Move` never replaces; `rename` would replace an empty folder.
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            std::fs::rename(&part, &final_path)
        })();
        match written {
            Ok(()) => Ok(handle),
            Err(error) => {
                self.release(std::slice::from_ref(&handle));
                try_delete(&part);
                Err(error)
            }
        }
    }

    /// The rows naming these handles are in the outbox now: the sweep may judge them.
    pub fn release(&self, handles: &[String]) {
        let mut pinned = self.pinned.borrow_mut();
        for handle in handles {
            pinned.remove(handle);
        }
    }

    /// 32 lowercase hex digits nobody else will make: two 64-bit hashes, keyed by the process's own
    /// random keys (`RandomState`, seeded by the system), of a count and the time. The C#'s
    /// `Guid.NewGuid()`.
    fn new_handle(&self) -> String {
        let count = self.made.get().wrapping_add(1);
        self.made.set(count);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default();
        let mut halves = [0u64; 2];
        for (index, half) in halves.iter_mut().enumerate() {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u64(count);
            hasher.write_u128(nanos);
            hasher.write_usize(index);
            hasher.write_usize(std::ptr::from_ref(self) as usize);
            *half = hasher.finish();
        }
        format!("{:016x}{:016x}", halves[0], halves[1])
    }
}

impl MediaStore for FolderMediaStore {
    fn read(&self, handle: &str) -> Option<StagedMedia> {
        if !is_handle(handle) {
            return None;
        }
        let path = self.folder.join(handle);
        if !path.is_dir() {
            return None;
        }
        // Half a folder is no folder: the row is told its media is missing, which is the truth.
        let bytes = std::fs::read(path.join("bytes")).ok()?;
        let preview_path = path.join("preview");
        let preview = if preview_path.is_file() {
            Some(std::fs::read(preview_path).ok()?)
        } else {
            None
        };
        let document = String::from_utf8(std::fs::read(path.join("meta.json")).ok()?).ok()?;
        let meta: Value = serde_json::from_str(&document).ok()?;
        // The C# threw (InvalidOperationException, uncaught) on a document that is not an object;
        // a damaged file is a missing one here too.
        let root = meta.as_object()?;
        let numbers = raw_numbers(&document);
        let real = |name: &str| real(root, &numbers, name);
        Some(StagedMedia {
            kind: text(root, "kind").unwrap_or_else(|| "file".to_string()),
            mime: text(root, "mime").unwrap_or_else(|| "application/octet-stream".to_string()),
            bytes,
            width: number(root, "width"),
            height: number(root, "height"),
            duration_ms: number(root, "duration_ms"),
            name: text(root, "name"),
            preview,
            latitude: real("latitude"),
            longitude: real("longitude"),
            accuracy_m: real("accuracy_m"),
            waveform: text(root, "waveform"),
            animation: false,
        })
    }

    fn sweep(&self, keep: &HashSet<String>) {
        let Ok(entries) = std::fs::read_dir(&self.folder) else {
            return;
        };
        let pinned = self.pinned.borrow();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let handle = name.strip_suffix(PART).unwrap_or(&name);
            if keep.contains(handle) || pinned.contains(handle) {
                continue;
            }
            try_delete(&entry.path());
        }
    }
}

fn is_handle(handle: &str) -> bool {
    handle.len() == 32
        && handle
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn meta(media: &StagedMedia) -> io::Result<Vec<u8>> {
    let refuse = |what: &str| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} is not a number"),
        )
    };
    let mut root = Map::new();
    root.insert("kind".into(), Value::from(media.kind.as_str()));
    root.insert("mime".into(), Value::from(media.mime.as_str()));
    if let Some(width) = media.width {
        root.insert("width".into(), Value::from(width));
    }
    if let Some(height) = media.height {
        root.insert("height".into(), Value::from(height));
    }
    if let Some(duration) = media.duration_ms {
        root.insert("duration_ms".into(), Value::from(duration));
    }
    if let Some(name) = &media.name {
        root.insert("name".into(), Value::from(name.as_str()));
    }
    // Written as the doubles they are: a place read back a centimetre off is a different place.
    if let Some(latitude) = media.latitude {
        if !latitude.is_finite() {
            return Err(refuse("a latitude"));
        }
        root.insert("latitude".into(), Value::from(latitude));
    }
    if let Some(longitude) = media.longitude {
        if !longitude.is_finite() {
            return Err(refuse("a longitude"));
        }
        root.insert("longitude".into(), Value::from(longitude));
    }
    if let Some(accuracy) = media.accuracy_m.filter(|accuracy| accuracy.is_finite()) {
        root.insert("accuracy_m".into(), Value::from(accuracy));
    }
    // A voice note's shape survives a relaunch with its bytes: the note goes up with it whenever it
    // goes.
    if let Some(waveform) = &media.waveform {
        root.insert("waveform".into(), Value::from(waveform.as_str()));
    }
    serde_json::to_vec(&Value::Object(root)).map_err(io::Error::other)
}

fn text(root: &Map<String, Value>, name: &str) -> Option<String> {
    root.get(name)?.as_str().map(str::to_string)
}

/// A JSON number that is a whole `i32` (`TryGetInt32`).
fn number(root: &Map<String, Value>, name: &str) -> Option<i32> {
    i32::try_from(root.get(name)?.as_i64()?).ok()
}

/// A JSON number as a double (`TryGetDouble`) — read from its own digits by Rust's `str::parse`,
/// which rounds correctly as .NET's reader does. serde_json's default reader (the workspace does not
/// turn on its `float_roundtrip`) can land one unit in the last place away, and a place is written
/// "as the double it is" so that it reads back as that double.
fn real(root: &Map<String, Value>, numbers: &HashMap<String, &str>, name: &str) -> Option<f64> {
    let value = root.get(name)?;
    if !value.is_number() {
        return None;
    }
    numbers
        .get(name)
        .and_then(|digits| digits.parse::<f64>().ok())
        .or_else(|| value.as_f64())
}

/// The raw digits of every number that is a member of the top-level object of `json` — a document
/// serde_json has already read whole, so it is well formed. The last of a repeated key wins, as in
/// serde_json's map.
fn raw_numbers(json: &str) -> HashMap<String, &str> {
    let bytes = json.as_bytes();
    let mut numbers = HashMap::new();
    let mut at = skip_space(bytes, 0);
    if bytes.get(at) != Some(&b'{') {
        return numbers;
    }
    at += 1;
    loop {
        at = skip_space(bytes, at);
        if bytes.get(at) != Some(&b'"') {
            return numbers;
        }
        let key_end = past_string(bytes, at);
        let Ok(key) = serde_json::from_str::<String>(&json[at..key_end]) else {
            return numbers;
        };
        at = skip_space(bytes, key_end);
        if bytes.get(at) != Some(&b':') {
            return numbers;
        }
        at = skip_space(bytes, at + 1);
        let start = at;
        at = past_value(bytes, at);
        if matches!(bytes.get(start), Some(b'-' | b'0'..=b'9')) {
            numbers.insert(key, &json[start..at]);
        }
        at = skip_space(bytes, at);
        match bytes.get(at) {
            Some(b',') => at += 1,
            _ => return numbers,
        }
    }
}

fn skip_space(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        at += 1;
    }
    at
}

/// Past the string that opens at `at` (its closing quote included).
fn past_string(bytes: &[u8], mut at: usize) -> usize {
    at += 1;
    while let Some(&byte) = bytes.get(at) {
        at += match byte {
            b'\\' => 2,
            b'"' => return at + 1,
            _ => 1,
        };
    }
    bytes.len()
}

/// Past the value that starts at `at`: a string, a nested object or array, or a bare token.
fn past_value(bytes: &[u8], mut at: usize) -> usize {
    match bytes.get(at) {
        Some(b'"') => past_string(bytes, at),
        Some(b'{' | b'[') => {
            let mut depth = 0usize;
            while let Some(&byte) = bytes.get(at) {
                match byte {
                    b'"' => {
                        at = past_string(bytes, at);
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return at + 1;
                        }
                    }
                    _ => {}
                }
                at += 1;
            }
            bytes.len()
        }
        _ => {
            while matches!(bytes.get(at), Some(byte) if !matches!(byte, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r'))
            {
                at += 1;
            }
            at
        }
    }
}

fn try_delete(entry: &Path) {
    // A link is removed as the link it is, never followed into what it points at.
    let folder = std::fs::symlink_metadata(entry).is_ok_and(|meta| meta.file_type().is_dir());
    // Open elsewhere for a moment, or gone already: the next sweep comes back to it.
    let _ = if folder {
        std::fs::remove_dir_all(entry)
    } else {
        std::fs::remove_file(entry)
    };
}
