//! The version the web client loads the rich-block renderer under (issue #97).
//!
//! Trunk copies `rich/` into `dist/rich/` as it is (index.html's `copy-dir`),
//! so its files keep their names — and nginx serves every `.js` and `.css`
//! with `Cache-Control: immutable` for a year (server/nginx). A browser that
//! once fetched `rich/fc-rich.js` would never ask again, and a new engine
//! would never reach it. So the client asks for `rich/fc-rich.js?v=<this>`,
//! and fc-rich.js carries the same query to every engine it loads beside
//! itself.
//!
//! The version is a hash of EVERY file under `rich/` — names and bytes —
//! rather than fc-rich.js's own `VERSION` string: that string is bumped by
//! hand, and a change somebody forgot to bump for would be a change no
//! browser ever saw. A hash cannot be forgotten. It is FNV-1a, written out
//! here, because it must be the same on every machine and toolchain (std's
//! `DefaultHasher` promises neither) and a build script should not need a
//! crate to say so.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let root = Path::new("rich");
    // A directory here makes Cargo re-run this script whenever anything
    // under it changes, recursively.
    println!("cargo:rerun-if-changed=rich");
    let mut files = Vec::new();
    collect(root, &mut files);
    // Sorted, so the walk order of the file system never changes the hash.
    files.sort();
    let mut hash = Fnv::new();
    for file in &files {
        let name = file.strip_prefix(root).unwrap_or(file);
        // Names with `/` on every system, so Windows hashes as macOS does.
        let name = name
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = fs::read(file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
        // The name, a separator no name holds, the length, then the bytes:
        // no two different trees run together into the same stream.
        hash.write(name.as_bytes());
        hash.write(&[0]);
        hash.write(&(bytes.len() as u64).to_le_bytes());
        hash.write(&bytes);
    }
    assert!(
        files.iter().any(|file| file.ends_with("fc-rich.js")),
        "rich/fc-rich.js is missing — the web client draws no rich block without it"
    );
    println!("cargo:rustc-env=FC_RICH_VERSION={:016x}", hash.finish());
}

/// Every file under `dir`, at any depth.
fn collect(dir: &Path, into: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        // A Finder `.DS_Store` is not one of the files, and must not make a
        // Mac's build differ from CI's.
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if path.is_dir() {
            collect(&path, into);
        } else {
            into.push(path);
        }
    }
}

/// 64-bit FNV-1a.
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}
