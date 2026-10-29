//! `ParkedRecordingsTests`: the store of voice messages that were not sent (S2.8) — per account, on
//! disk, whole or not at all, swept at launch and wiped when the session ends. Each test works in a
//! scratch folder of its own (`common::scratch_dir`, the C#'s temp path + a GUID), removed after.
//! The theory `NonsenseIsNoEntry` loops over its eight rows.

mod common;

use std::path::{Path, PathBuf};

use fc_core::protocol::send_pipeline::uuid_v4;
use fc_logic::parked_recordings::{ParkedRecording, ParkedRecordings};
use fc_logic::voice_notes;
use time::macros::datetime;
use time::{Duration, OffsetDateTime};

const SERVER: &str = "https://family.example/";
const AT: OffsetDateTime = datetime!(2026-10-05 12:00:00 UTC);

/// The test's root, removed when it ends.
struct Root(PathBuf);

impl Root {
    fn new() -> Self {
        Root(common::scratch_dir("parked"))
    }

    fn store(&self) -> ParkedRecordings {
        self.store_for(5, SERVER)
    }

    fn store_for(&self, account: i64, server: &str) -> ParkedRecordings {
        ParkedRecordings::for_account(&self.0, server, account)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn voice(fill: u8) -> Vec<u8> {
    vec![fill; 4096]
}

/// `Guid.NewGuid().ToString("N")`.
fn guid() -> String {
    uuid_v4().replace('-', "")
}

fn entries(folder: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
        .unwrap()
        .map(|item| item.unwrap().path())
        .collect();
    found.sort();
    found
}

#[test]
fn a_parked_recording_reads_back_as_it_was_parked() {
    let root = Root::new();
    let store = root.store();
    let parked = store
        .park(
            42,
            &voice(3),
            12_345,
            Some(9),
            Some("Happy birthday 🎂\nfrom all of us"),
            AT,
        )
        .unwrap();

    let read = store.of(42);
    assert_eq!(1, read.len());
    let read = &read[0];
    assert_eq!(&parked, read);
    assert_eq!(
        (
            42,
            12_345,
            Some(9),
            Some("Happy birthday 🎂\nfrom all of us"),
            AT
        ),
        (
            read.chat_id,
            read.duration_ms,
            read.reply_to_message_id,
            read.caption.as_deref(),
            read.parked_at
        )
    );

    let staged = store.staged(read).unwrap();
    assert_eq!(
        ("audio", "audio/mp4", Some(12_345)),
        (
            staged.kind.as_str(),
            staged.mime.as_str(),
            staged.duration_ms
        )
    );
    assert_eq!(None, staged.name);
    assert_eq!(voice(3), staged.bytes);
    // Exactly what a recording staged at Stop is, so it travels as one.
    assert!(voice_notes::is_recorded(&staged));
}

#[test]
fn without_a_reply_or_a_caption_it_has_neither() {
    let root = Root::new();
    let store = root.store();
    let parked = store
        .park(42, &voice(7), 3_000, None, Some("   "), AT)
        .unwrap();

    let read = store.of(42);
    assert_eq!(1, read.len());
    assert_eq!(None, read[0].reply_to_message_id);
    assert_eq!(None, read[0].caption);
    // What the park answers is what any later read finds: the caller holds the same entry the row
    // is drawn from.
    assert_eq!(read[0], parked);
}

/// The app closing is not the end of it: another store over the same folder — the next launch —
/// finds it.
#[test]
fn it_survives_the_app_being_closed() {
    let root = Root::new();
    root.store()
        .park(42, &voice(7), 3_000, None, Some("later"), AT)
        .unwrap();

    let read = root.store().of(42);
    assert_eq!(1, read.len());
    assert_eq!(Some("later".to_string()), read[0].caption);
}

#[test]
fn each_chat_has_its_own_oldest_first() {
    let root = Root::new();
    let store = root.store();
    let second = store
        .park(1, &voice(7), 2_000, None, None, AT + Duration::minutes(1))
        .unwrap();
    let other = store.park(2, &voice(7), 3_000, None, None, AT).unwrap();
    let first = store.park(1, &voice(7), 1_000, None, None, AT).unwrap();

    assert_eq!(vec![first, second], store.of(1));
    assert_eq!(vec![other], store.of(2));
    assert!(store.of(3).is_empty());
    assert!(store.any(1));
    assert!(!store.any(3));
    assert_eq!(3, store.all().len());
}

#[test]
fn removed_is_gone_file_and_all() {
    let root = Root::new();
    let store = root.store();
    let kept = store.park(1, &voice(7), 2_000, None, None, AT).unwrap();
    let sent = store
        .park(1, &voice(7), 3_000, None, None, AT + Duration::seconds(1))
        .unwrap();

    assert!(store.remove(&sent));

    assert_eq!(vec![kept], store.of(1));
    assert!(!store.folder().join(&sent.id).exists());
    assert_eq!(None, store.staged(&sent));
}

/// Nobody else signed in on this device sees them — nor the same id on another server.
#[test]
fn each_account_on_each_server_has_its_own() {
    let root = Root::new();
    root.store_for(5, SERVER)
        .park(1, &voice(7), 2_000, None, Some("mine"), AT)
        .unwrap();

    assert!(root.store_for(6, SERVER).of(1).is_empty());
    assert!(root.store_for(5, "https://other.example/").of(1).is_empty());
    assert!(
        root.store_for(5, "https://family.example/elsewhere/")
            .of(1)
            .is_empty()
    );
    assert_eq!(1, root.store_for(5, SERVER).of(1).len());
    // The folder is named for neither the address nor the id.
    let store = root.store_for(5, SERVER);
    let name = store.folder().file_name().unwrap().to_str().unwrap();
    assert!(!name.contains("family"));
    assert_eq!(32, name.len());
}

/// A session ending takes every account's recordings with it, as it takes the cache and its outbox.
#[test]
fn a_wipe_takes_every_accounts_recordings() {
    let root = Root::new();
    root.store_for(5, SERVER)
        .park(1, &voice(7), 2_000, None, None, AT)
        .unwrap();
    root.store_for(6, SERVER)
        .park(1, &voice(7), 2_000, None, None, AT)
        .unwrap();

    ParkedRecordings::wipe_all(&root.0);

    assert!(root.store_for(5, SERVER).all().is_empty());
    assert!(root.store_for(6, SERVER).all().is_empty());
    assert!(!root.0.exists());
    // And wiping nothing is nothing.
    ParkedRecordings::wipe_all(&root.0);
}

/// A part a crash left behind is never an entry, and the launch's sweep takes it — with anything
/// else no entry names.
#[test]
fn what_a_crash_left_is_never_read_and_is_swept() {
    let root = Root::new();
    let store = root.store();
    let good = store.park(1, &voice(7), 2_000, None, None, AT).unwrap();
    let folder = store.folder().to_path_buf();
    let part = folder.join(format!("{}.part", guid()));
    std::fs::create_dir_all(&part).unwrap();
    std::fs::write(part.join("voice.m4a"), voice(7)).unwrap();
    let no_meta = folder.join(guid());
    std::fs::create_dir_all(&no_meta).unwrap();
    std::fs::write(no_meta.join("voice.m4a"), voice(7)).unwrap();
    let no_bytes = folder.join(guid());
    std::fs::create_dir_all(&no_bytes).unwrap();
    std::fs::write(
        no_bytes.join("meta.json"),
        r#"{"chat_id":1,"duration_ms":2000,"parked_at_ms":0}"#,
    )
    .unwrap();
    let broken_meta = folder.join(guid());
    std::fs::create_dir_all(&broken_meta).unwrap();
    std::fs::write(broken_meta.join("voice.m4a"), voice(7)).unwrap();
    std::fs::write(
        broken_meta.join("meta.json"),
        r#"{"chat_id":"one","duration_ms":2000"#,
    )
    .unwrap();
    let not_an_id = folder.join("notes");
    std::fs::create_dir_all(&not_an_id).unwrap();
    std::fs::write(folder.join("stray.txt"), "x").unwrap();

    assert_eq!(vec![good.clone()], store.all());

    store.sweep();

    assert_eq!(vec![good.clone()], store.all());
    assert_eq!(vec![folder.join(&good.id)], entries(&folder));
}

/// An entry whose numbers make no sense is no entry: a chat id must be a chat, a length cannot be
/// negative. (A theory over eight metas.)
#[test]
fn nonsense_is_no_entry() {
    for meta in [
        r#"{"chat_id":0,"duration_ms":2000,"parked_at_ms":0}"#,
        r#"{"chat_id":-4,"duration_ms":2000,"parked_at_ms":0}"#,
        r#"{"chat_id":1,"duration_ms":-1,"parked_at_ms":0}"#,
        r#"{"chat_id":1,"duration_ms":4294967296,"parked_at_ms":0}"#,
        r#"{"chat_id":1,"parked_at_ms":0}"#,
        r#"{"chat_id":1,"duration_ms":2000}"#,
        r#"{"chat_id":1,"duration_ms":2000,"parked_at_ms":999999999999999999}"#,
        r#"[1,2,3]"#,
    ] {
        let root = Root::new();
        let folder = root.store().folder().join(guid());
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("voice.m4a"), voice(7)).unwrap();
        std::fs::write(folder.join("meta.json"), meta).unwrap();

        assert!(root.store().all().is_empty(), "{meta}");
    }
}

/// A reply id that is no message is no reply; the recording itself is still kept.
#[test]
fn a_reply_that_is_no_message_is_dropped() {
    let root = Root::new();
    let folder = root.store().folder().join(guid());
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("voice.m4a"), voice(7)).unwrap();
    std::fs::write(
        folder.join("meta.json"),
        r#"{"chat_id":1,"duration_ms":2000,"reply_to_message_id":0,"parked_at_ms":0}"#,
    )
    .unwrap();

    let all = root.store().all();
    assert_eq!(1, all.len());
    assert_eq!(None, all[0].reply_to_message_id);
}

/// An id that comes back from anywhere but this store is not a path: nothing outside the folder is
/// read or deleted.
#[test]
fn an_id_is_not_a_path() {
    let root = Root::new();
    let store = root.store();
    let parked = store.park(1, &voice(7), 2_000, None, None, AT).unwrap();
    let outside = root.0.join("precious");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("voice.m4a"), voice(7)).unwrap();

    let forged = ParkedRecording {
        id: Path::new("..")
            .join("precious")
            .to_str()
            .unwrap()
            .to_string(),
        ..parked.clone()
    };
    assert!(!store.remove(&forged));
    assert_eq!(None, store.staged(&forged));
    assert!(!store.remove(&ParkedRecording {
        id: parked.id.to_uppercase(),
        ..parked.clone()
    }));

    assert!(outside.join("voice.m4a").exists());
    assert_eq!(1, store.all().len());
}

/// A park that cannot be written answers its error, so the caller keeps the recording some other
/// way — and leaves no part behind.
#[test]
fn a_part_that_cannot_land_throws_and_leaves_nothing() {
    let root = Root::new();
    let store = root.store();
    // A FILE where the account's folder should be: nothing can be written under it.
    std::fs::create_dir_all(&root.0).unwrap();
    std::fs::write(store.folder(), "in the way").unwrap();

    assert!(store.park(1, &voice(7), 2_000, None, None, AT).is_err());

    std::fs::remove_file(store.folder()).unwrap();
    assert!(store.all().is_empty());
}

/// WHAT THE DISK REFUSES IS HELD IN MEMORY (S4: the disk fills): the same entry a park would make —
/// its chat, length, reply and caption — listed, played and sent like any other, so it is still its
/// own row, never carried by another Send (S2.8), and its reply and caption are never lost;
/// deleted, it is gone.
#[test]
fn what_the_disk_refuses_is_held_as_the_same_row() {
    let root = Root::new();
    let store = root.store();
    std::fs::create_dir_all(&root.0).unwrap();
    std::fs::write(store.folder(), "in the way").unwrap();
    assert!(
        store
            .park(42, &voice(5), 12_345, Some(9), Some("for you"), AT)
            .is_err()
    );

    let held = store.hold(42, &voice(5), 12_345, Some(9), Some("for you"), AT);

    assert!(store.holds_in_memory());
    assert_eq!(vec![held.clone()], store.of(42));
    assert!(store.any(42));
    assert!(!store.any(43));
    assert_eq!(
        (42, 12_345, Some(9), Some("for you")),
        (
            held.chat_id,
            held.duration_ms,
            held.reply_to_message_id,
            held.caption.as_deref()
        )
    );
    let staged = store.staged(&held).unwrap();
    assert_eq!(voice(5), staged.bytes);
    assert_eq!(Some(12_345), staged.duration_ms);
    assert!(voice_notes::is_recorded(&staged));
    // Its id is an id like any other: nothing forged reaches it.
    assert!(!store.remove(&ParkedRecording {
        id: Path::new("..").join(&held.id).to_str().unwrap().to_string(),
        ..held.clone()
    }));

    assert!(store.remove(&held));
    assert!(store.of(42).is_empty());
    assert!(!store.holds_in_memory());
    assert_eq!(None, store.staged(&held));
}

/// Held and written side by side, oldest first: one chat's rows are one list, wherever each is
/// kept.
#[test]
fn held_and_written_are_one_list_oldest_first() {
    let root = Root::new();
    let store = root.store();
    let written = store
        .park(1, &voice(7), 2_000, None, None, AT + Duration::minutes(1))
        .unwrap();
    let held = store.hold(1, &voice(7), 3_000, None, Some("held"), AT);

    assert_eq!(vec![held, written.clone()], store.of(1));
    // Another store over the same folder — the next launch — finds only what was written.
    assert_eq!(vec![written], root.store().of(1));
}

/// A real close tries the disk again: whatever it takes moves there under the SAME id — the row
/// drawn for it is still its row — and survives the app; whatever it still refuses stays held.
#[test]
fn a_held_recording_is_written_once_the_disk_takes_it() {
    let root = Root::new();
    let store = root.store();
    std::fs::create_dir_all(&root.0).unwrap();
    std::fs::write(store.folder(), "in the way").unwrap();
    let held = store.hold(42, &voice(9), 4_000, Some(3), Some("later"), AT);

    assert_eq!(1, store.write_held());
    assert!(store.holds_in_memory());

    std::fs::remove_file(store.folder()).unwrap();
    assert_eq!(0, store.write_held());

    assert!(!store.holds_in_memory());
    assert_eq!(vec![held.clone()], store.of(42));
    let relaunched = root.store().of(42);
    assert_eq!(1, relaunched.len());
    assert_eq!(held, relaunched[0]);
    assert_eq!(voice(9), root.store().staged(&relaunched[0]).unwrap().bytes);
}

/// A session ending takes what is held in memory with it, as the wipe takes what is written (S4's
/// sign-out row).
#[test]
fn what_is_held_goes_with_the_session() {
    let root = Root::new();
    let store = root.store();
    store.hold(1, &voice(7), 2_000, None, None, AT);

    store.forget_held();

    assert!(!store.holds_in_memory());
    assert!(store.all().is_empty());
    // Nothing held: nothing to write.
    assert_eq!(0, store.write_held());
}
