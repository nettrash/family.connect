//! The two storage seams the media tests hand in: the C# tests' `Blobs` (an `IBlobStore` in a
//! dictionary) and `Staging` (an `IMediaStore` in a dictionary, keeping every sweep it was asked
//! for).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use fc_logic::attachment_cache::BlobStore;
use fc_logic::media_outbox::{MediaStore, StagedMedia};

/// Downloaded bytes in memory. Clones share them.
#[derive(Clone, Default)]
pub struct Blobs(pub Rc<RefCell<HashMap<String, Vec<u8>>>>);

impl Blobs {
    pub fn new() -> Self {
        Blobs::default()
    }

    /// How many keys are held (`Held.Count`).
    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }
}

impl BlobStore for Blobs {
    fn read(&self, key: &str) -> Option<Vec<u8>> {
        self.0.borrow().get(key).cloned()
    }

    fn write(&self, key: &str, bytes: &[u8]) {
        self.0.borrow_mut().insert(key.to_string(), bytes.to_vec());
    }
}

/// Staged bytes, and what a test says has gone missing. Clones share them.
#[derive(Clone, Default)]
pub struct Staging {
    pub files: Rc<RefCell<HashMap<String, StagedMedia>>>,
    pub sweeps: Rc<RefCell<Vec<HashSet<String>>>>,
    /// A store whose sweep forgets nothing (the C# `Staging` of the round-video tests).
    pub keeps_everything: bool,
}

impl Staging {
    pub fn new() -> Self {
        Staging::default()
    }

    /// `With(handle, kind, mime, preview)`: three bytes, 1600 × 1200.
    pub fn with(self, handle: &str, kind: &str, mime: &str, preview: Option<&[u8]>) -> Self {
        let staged = StagedMedia {
            width: Some(1600),
            height: Some(1200),
            preview: preview.map(<[u8]>::to_vec),
            ..StagedMedia::new(kind, mime, vec![1, 2, 3])
        };
        self.files.borrow_mut().insert(handle.to_string(), staged);
        self
    }

    /// `With(handle)`: a photo.
    pub fn with_photo(self, handle: &str) -> Self {
        self.with(handle, "photo", "image/jpeg", None)
    }

    pub fn put(&self, handle: &str, staged: StagedMedia) {
        self.files.borrow_mut().insert(handle.to_string(), staged);
    }

    pub fn holds(&self, handle: &str) -> bool {
        self.files.borrow().contains_key(handle)
    }
}

impl MediaStore for Staging {
    fn read(&self, handle: &str) -> Option<StagedMedia> {
        self.files.borrow().get(handle).cloned()
    }

    fn sweep(&self, keep: &HashSet<String>) {
        self.sweeps.borrow_mut().push(keep.clone());
        if !self.keeps_everything {
            self.files
                .borrow_mut()
                .retain(|handle, _| keep.contains(handle));
        }
    }
}
