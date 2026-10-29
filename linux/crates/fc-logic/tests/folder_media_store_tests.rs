//! `FolderMediaStoreTests`: a send's bytes on disk while it is under way — written whole, read back
//! as they were, pinned until queued, swept when no row names them, and never read through a handle
//! that is not this store's own. In a folder of their own under the system's temporary directory.

mod common;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use fc_logic::folder_media_store::FolderMediaStore;
use fc_logic::media_outbox::{MediaStore, StagedMedia};

/// The folder the C# test made per test (`Path.GetTempPath()` + a GUID), removed afterwards.
struct Folder(PathBuf);

impl Folder {
    fn new() -> Folder {
        Folder(common::scratch_dir("staging"))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn store(&self) -> FolderMediaStore {
        FolderMediaStore::new(&self.0)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn keep(handles: &[&str]) -> HashSet<String> {
    handles.iter().map(|handle| handle.to_string()).collect()
}

fn entries(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn what_is_staged_reads_back_as_it_was_written() {
    let folder = Folder::new();
    let store = folder.store();
    let media = StagedMedia {
        width: Some(1920),
        height: Some(1080),
        duration_ms: Some(4200),
        name: Some("clip ✨.mp4".to_string()),
        preview: Some(vec![9, 8]),
        ..StagedMedia::new("video", "video/mp4", vec![1, 2, 3])
    };

    let handle = store.stage(&media).unwrap();
    let read = store.read(&handle).unwrap();

    assert_eq!(
        (
            read.kind.as_str(),
            read.mime.as_str(),
            read.width,
            read.height,
            read.duration_ms,
            read.name.as_deref()
        ),
        (
            "video",
            "video/mp4",
            Some(1920),
            Some(1080),
            Some(4200),
            Some("clip ✨.mp4")
        )
    );
    assert_eq!(read.bytes, vec![1, 2, 3]);
    assert_eq!(read.preview, Some(vec![9, 8]));
}

/// A voice note's shape survives a relaunch with its bytes, and a note without one reads back
/// without one.
#[test]
fn a_voice_notes_waveform_reads_back() {
    let folder = Folder::new();
    let store = folder.store();
    const WIRE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";
    let note = |waveform: Option<&str>| StagedMedia {
        duration_ms: Some(4200),
        waveform: waveform.map(str::to_string),
        ..StagedMedia::new("audio", "audio/mp4", vec![1])
    };

    let read = store
        .read(&store.stage(&note(Some(WIRE))).unwrap())
        .unwrap();
    let plain = store.read(&store.stage(&note(None)).unwrap()).unwrap();

    assert_eq!(read.waveform.as_deref(), Some(WIRE));
    assert_eq!(plain.waveform, None);
}

#[test]
fn media_without_optional_facts_reads_back_without_them() {
    let folder = Folder::new();
    let store = folder.store();

    let read = store
        .read(
            &store
                .stage(&StagedMedia::new("file", "application/pdf", vec![7]))
                .unwrap(),
        )
        .unwrap();

    assert_eq!(read.width, None);
    assert_eq!(read.duration_ms, None);
    assert_eq!(read.name, None);
    assert_eq!(read.preview, None);
    assert_eq!(read.latitude, None);
    assert_eq!(read.accuracy_m, None);
}

/// A place has no bytes, and reads back with its numbers exactly — a centimetre off is a different
/// place.
#[test]
fn a_place_reads_back_with_its_numbers_and_no_bytes() {
    let folder = Folder::new();
    let store = folder.store();

    let place = StagedMedia {
        latitude: Some(55.003_906_25),
        longitude: Some(-0.1),
        accuracy_m: Some(12.5),
        ..StagedMedia::new("location", "", vec![])
    };
    let read = store.read(&store.stage(&place).unwrap()).unwrap();

    assert_eq!(read.kind, "location");
    assert_eq!(read.latitude, Some(55.003_906_25));
    assert_eq!(read.longitude, Some(-0.1));
    assert_eq!(read.accuracy_m, Some(12.5));
    assert!(read.bytes.is_empty());
    assert_eq!(read.preview, None);
}

/// An accuracy that is not a number is not written — JSON has no NaN — and the place still stages.
#[test]
fn a_place_whose_accuracy_is_not_a_number_stages_without_one() {
    let folder = Folder::new();
    let store = folder.store();

    let place = StagedMedia {
        latitude: Some(1.0),
        longitude: Some(2.0),
        accuracy_m: Some(f64::NAN),
        ..StagedMedia::new("location", "", vec![])
    };
    let read = store.read(&store.stage(&place).unwrap()).unwrap();

    assert_eq!(read.longitude, Some(2.0));
    assert_eq!(read.accuracy_m, None);
}

/// PINNED UNTIL QUEUED: a flush sweeps between "written" and "queued", when the outbox names
/// nothing — and must not take the files with it.
#[test]
fn a_sweep_leaves_a_pinned_handle_and_takes_it_once_released() {
    let folder = Folder::new();
    let store = folder.store();
    let handle = store
        .stage(&StagedMedia::new("photo", "image/jpeg", vec![1]))
        .unwrap();

    store.sweep(&keep(&[]));
    assert!(store.read(&handle).is_some());

    store.release(std::slice::from_ref(&handle));
    store.sweep(&keep(&[&handle]));
    assert!(store.read(&handle).is_some());

    store.sweep(&keep(&[]));
    assert!(store.read(&handle).is_none());
}

#[test]
fn a_sweep_takes_leftover_parts_and_strangers() {
    let folder = Folder::new();
    let store = folder.store();
    std::fs::create_dir_all(folder.path().join("0123456789abcdef0123456789abcdef.part")).unwrap();
    std::fs::write(folder.path().join("stray.txt"), "x").unwrap();
    let kept = store
        .stage(&StagedMedia::new("photo", "image/jpeg", vec![1]))
        .unwrap();
    store.release(std::slice::from_ref(&kept));

    store.sweep(&keep(&[&kept]));

    assert_eq!(entries(folder.path()), vec![kept]);
}

/// A handle comes back out of the cache: anything but this store's own is not a path.
#[test]
fn a_handle_that_is_not_one_reads_as_gone() {
    let folder = Folder::new();
    for handle in [
        "../../etc",
        "0123456789ABCDEF0123456789ABCDEF",
        "",
        "0123456789abcdef0123456789abcde",
    ] {
        assert!(folder.store().read(handle).is_none(), "{handle}");
    }
}

#[test]
fn half_a_folder_is_no_folder() {
    let folder = Folder::new();
    let store = folder.store();
    let handle = store
        .stage(&StagedMedia::new("photo", "image/jpeg", vec![1]))
        .unwrap();
    std::fs::remove_file(folder.path().join(&handle).join("meta.json")).unwrap();

    assert!(store.read(&handle).is_none());
}

#[test]
fn a_sweep_of_a_folder_never_made_is_nothing() {
    let folder = Folder::new();
    folder.store().sweep(&keep(&[]));
    assert!(!folder.path().exists());
}

/// The same rule against REAL folders: a handle that climbs into a neighbour, or spells a real
/// handle in capitals — which a case-folding file system would find — is not read.
#[test]
fn a_handle_that_reaches_a_real_folder_some_other_way_is_not_read() {
    let folder = Folder::new();
    let neighbour = FolderMediaStore::new(folder.path().join("a"));
    let handle = neighbour
        .stage(&StagedMedia::new("photo", "image/jpeg", vec![1]))
        .unwrap();
    std::fs::create_dir_all(folder.path().join("b")).unwrap();
    let store = FolderMediaStore::new(folder.path().join("b"));

    assert!(neighbour.read(&handle).is_some());
    assert!(store.read(&format!("../a/{handle}")).is_none());
    assert!(neighbour.read(&handle.to_uppercase()).is_none());
}

/// A part named for a handle the sweep must keep is that handle's write in progress, and stays.
#[test]
fn a_part_for_a_handle_that_is_kept_stays() {
    let folder = Folder::new();
    const HANDLE: &str = "0123456789abcdef0123456789abcdef";
    std::fs::create_dir_all(folder.path().join(format!("{HANDLE}.part"))).unwrap();

    folder.store().sweep(&keep(&[HANDLE]));

    assert!(folder.path().join(format!("{HANDLE}.part")).is_dir());
}

// ---- the port's own ---------------------------------------------------------------------------

/// A place whose latitude or longitude is not a number is refused before anything is written — the
/// C#'s JSON writer threw on it — and leaves nothing pinned or on disk.
#[test]
fn a_place_whose_coordinates_are_not_numbers_is_refused() {
    let folder = Folder::new();
    let store = folder.store();
    let place = |latitude: f64, longitude: f64| StagedMedia {
        latitude: Some(latitude),
        longitude: Some(longitude),
        ..StagedMedia::new("location", "", vec![])
    };
    for (latitude, longitude) in [(f64::NAN, 2.0), (1.0, f64::INFINITY)] {
        let error = store.stage(&place(latitude, longitude)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
    assert!(!folder.path().exists());
}

/// Every handle is 32 lowercase hex digits, and no two are the same.
#[test]
fn handles_are_this_stores_own_and_never_repeat() {
    let folder = Folder::new();
    let store = folder.store();
    let mut seen = HashSet::new();
    for _ in 0..200 {
        let handle = store
            .stage(&StagedMedia::new("photo", "image/jpeg", vec![1]))
            .unwrap();
        assert_eq!(handle.len(), 32);
        assert!(
            handle
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert!(seen.insert(handle));
    }
}

/// A `meta.json` that is not an object — where the C# threw out of `Read` — is a missing file, and
/// one with fields of the wrong type reads as if they were absent.
#[test]
fn a_damaged_meta_reads_as_gone_or_without_the_damaged_facts() {
    let folder = Folder::new();
    let store = folder.store();
    let handle = store
        .stage(&StagedMedia::new("photo", "image/jpeg", vec![1]))
        .unwrap();
    let meta = folder.path().join(&handle).join("meta.json");

    std::fs::write(&meta, "[]").unwrap();
    assert!(store.read(&handle).is_none());

    std::fs::write(
        &meta,
        "{\"kind\": 3, \"width\": 1.5, \"height\": 4294967296, \"name\": null}",
    )
    .unwrap();
    let read = store.read(&handle).unwrap();
    assert_eq!(read.kind, "file");
    assert_eq!(read.mime, "application/octet-stream");
    assert_eq!((read.width, read.height, read.name), (None, None, None));
}

/// A place written as the double it is reads back as that double, whatever its digits.
#[test]
fn every_double_reads_back_exactly() {
    let folder = Folder::new();
    let store = folder.store();
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    for _ in 0..500 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let latitude = (state >> 11) as f64 / (1u64 << 53) as f64 * 180.0 - 90.0;
        let longitude = f64::from_bits(state).abs().fract() * 360.0 - 180.0;
        let place = StagedMedia {
            latitude: Some(latitude),
            longitude: Some(longitude),
            ..StagedMedia::new("location", "", vec![])
        };
        let read = store.read(&store.stage(&place).unwrap()).unwrap();
        assert_eq!(read.latitude.map(f64::to_bits), Some(latitude.to_bits()));
        assert_eq!(read.longitude.map(f64::to_bits), Some(longitude.to_bits()));
    }
}

/// A name that spells a number field is only a name: the doubles are read from the document's own
/// members, never from inside a string.
#[test]
fn a_name_that_spells_a_number_field_is_only_a_name() {
    let folder = Folder::new();
    let store = folder.store();
    let place = StagedMedia {
        name: Some("\"latitude\": 9, \"longitude\": [1, {\"x\": \"}\"}]".to_string()),
        latitude: Some(51.507_351_234_567_89),
        longitude: Some(-0.127_758_765_432_1),
        accuracy_m: Some(3.25),
        ..StagedMedia::new("location", "", vec![])
    };
    let read = store.read(&store.stage(&place).unwrap()).unwrap();
    assert_eq!(read.name, place.name);
    assert_eq!(read.latitude, Some(51.507_351_234_567_89));
    assert_eq!(read.longitude, Some(-0.127_758_765_432_1));
    assert_eq!(read.accuracy_m, Some(3.25));
}
