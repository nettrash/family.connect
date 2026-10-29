//! Linux's own tests of `transcript_sound::sound_track` — the window's `SoundTracks.MakeAsync` and
//! `MediaPreparing.SoundTrackAsync` (which the C# suite does not test: they are Windows' platform
//! code), ported over the shell's `SoundTrackMaker`, here a fake: the file from the attachment
//! cache, its track read, the attempts tried in order, each result judged and checked to be
//! MPEG-4, and what the transcript model is handed.

mod common;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::ready;
use std::rc::Rc;

use fc_core::protocol::{AttachmentDto, error_codes};
use fc_core::rt::{Cancel, LocalBoxFuture};
use fc_core::store::{Database, StoreResult, TranscriptStore};
use fc_core::testing::{FakeClock, drive};
use fc_logic::attachment_cache::AttachmentCache;
use fc_logic::transcode_attempts::{AttemptFailure, OperationCanceled};
use fc_logic::transcript_sound::{
    self, MadeSound, SoundAttempt, SoundTrackMaker, SoundWay, SuppliedSound, TrackReading,
    TranscriptForm,
};
use fc_logic::transcripts::{TranscriptLook, TranscriptModel};
use time::macros::datetime;

use common::media::Blobs;
use common::server::{Server, api};

const MIB25: i64 = 26_214_400;
const HELD: &[u8] = b"\0\0\0\x18ftypisom the held video";
const M4A: &[u8] = b"\0\0\0\x18ftypM4A  the sound";

/// A shell's maker that answers what it is told and remembers what it was asked.
struct Fake {
    reading: Result<Option<TrackReading>, AttemptFailure>,
    made: RefCell<VecDeque<Result<MadeSound, AttemptFailure>>>,
    asked: RefCell<Vec<SoundAttempt>>,
    extensions: RefCell<Vec<String>>,
    held: RefCell<Vec<Vec<u8>>>,
}

impl Fake {
    fn reading(reading: Result<Option<TrackReading>, AttemptFailure>) -> Self {
        Fake {
            reading,
            made: RefCell::default(),
            asked: RefCell::default(),
            extensions: RefCell::default(),
            held: RefCell::default(),
        }
    }

    fn track(codec: &str, bitrate: Option<i64>, sample_rate: Option<i64>) -> Self {
        Fake::reading(Ok(Some(TrackReading {
            codec: codec.to_string(),
            bitrate,
            sample_rate,
        })))
    }

    fn then(self, made: Result<MadeSound, AttemptFailure>) -> Self {
        self.made.borrow_mut().push_back(made);
        self
    }

    fn asked(&self) -> Vec<SoundAttempt> {
        self.asked.borrow().clone()
    }
}

impl SoundTrackMaker for Fake {
    fn read<'a>(
        &'a self,
        bytes: &'a Rc<[u8]>,
        extension: &'a str,
    ) -> LocalBoxFuture<'a, Result<Option<TrackReading>, AttemptFailure>> {
        self.extensions.borrow_mut().push(extension.to_string());
        self.held.borrow_mut().push(bytes.to_vec());
        Box::pin(ready(self.reading.clone()))
    }

    fn make<'a>(
        &'a self,
        _bytes: &'a Rc<[u8]>,
        _extension: &'a str,
        attempt: &'a SoundAttempt,
        _cancel: Cancel,
    ) -> LocalBoxFuture<'a, Result<MadeSound, AttemptFailure>> {
        self.asked.borrow_mut().push(*attempt);
        let made = self
            .made
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err(AttemptFailure::new("NotExpected", 0)));
        Box::pin(ready(made))
    }
}

fn aac_out(bytes: &[u8], bitrate: Option<i64>) -> Result<MadeSound, AttemptFailure> {
    Ok(MadeSound {
        bytes: bytes.to_vec(),
        codec: Some("aac".to_string()),
        bitrate,
        has_video: false,
    })
}

/// A video of 40 seconds, its bytes already in the attachment cache.
fn video() -> AttachmentDto {
    AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(HELD.len() as i64),
        duration_ms: Some(40_000),
        ..AttachmentDto::new(34, "video")
    }
}

struct Rig {
    clock: FakeClock,
    server: Server,
    blobs: Blobs,
    cache: Rc<AttachmentCache>,
}

impl Rig {
    fn new() -> Self {
        let clock = FakeClock::at(datetime!(2026-10-29 10:00 UTC));
        let server = Server::new();
        let blobs = Blobs::new();
        let cache = Rc::new(AttachmentCache::new(
            api(&server, &clock),
            Rc::new(blobs.clone()),
        ));
        Rig {
            clock,
            server,
            blobs,
            cache,
        }
    }

    fn holding(self, attachment: &AttachmentDto, bytes: &[u8]) -> Self {
        self.cache.remember(attachment, bytes);
        self
    }

    fn make(
        &self,
        attachment: &AttachmentDto,
        maker: &Fake,
        cancel: &Cancel,
    ) -> (Result<SuppliedSound, OperationCanceled>, Vec<String>) {
        let mut lines = Vec::new();
        let made = drive(
            &self.clock,
            transcript_sound::sound_track(
                &self.cache,
                attachment,
                MIB25,
                maker,
                &self.clock,
                cancel,
                |line| lines.push(line.to_string()),
            ),
        );
        (made, lines)
    }
}

/// A phone's video carries AAC: the track is COPIED, the file handed over as the cache holds it,
/// named for its type, and what came out goes as made.
#[test]
fn an_aac_track_is_copied_and_goes_as_made() {
    let attachment = video();
    let rig = Rig::new().holding(&attachment, HELD);
    let maker = Fake::track("aac", Some(128_000), Some(48_000)).then(aac_out(M4A, Some(128_000)));

    let (made, _) = rig.make(&attachment, &maker, &Cancel::new());

    assert_eq!(Ok(SuppliedSound::Made(M4A.to_vec())), made);
    assert_eq!(vec![SoundAttempt::PASSTHROUGH], maker.asked());
    assert_eq!(vec!["mp4".to_string()], *maker.extensions.borrow());
    assert_eq!(vec![HELD.to_vec()], *maker.held.borrow());
    // Held already: nothing downloaded.
    assert!(rig.server.asked().is_empty());
    assert_eq!(1, rig.blobs.len());
}

/// A copy that came out some other way leaves the encode to try — at a voice note's numbers, mono,
/// the track's own 48 kHz kept.
#[test]
fn a_copy_that_came_out_otherwise_falls_to_the_encode() {
    let attachment = video();
    let rig = Rig::new().holding(&attachment, HELD);
    let maker = Fake::track("aac", Some(128_000), Some(48_000))
        .then(Ok(MadeSound {
            bytes: M4A.to_vec(),
            codec: Some("mp3".to_string()),
            bitrate: None,
            has_video: false,
        }))
        .then(aac_out(M4A, Some(64_000)));

    let (made, _) = rig.make(&attachment, &maker, &Cancel::new());

    assert_eq!(Ok(SuppliedSound::Made(M4A.to_vec())), made);
    let asked = maker.asked();
    assert_eq!(2, asked.len());
    assert_eq!(SoundWay::Passthrough, asked[0].way);
    assert_eq!(SoundWay::Reencode, asked[1].way);
    let aac = asked[1].aac.expect("the encode's numbers");
    assert_eq!(
        (48_000, 1, 64_000),
        (aac.sample_rate, aac.channels, aac.bitrate)
    );
}

/// No sound track, or a file the reader cannot read: "Couldn't read the sound in this file.", and
/// nothing is made.
#[test]
fn no_track_or_an_unreadable_file_is_unreadable() {
    let attachment = video();
    let rig = Rig::new().holding(&attachment, HELD);
    for maker in [
        Fake::reading(Ok(None)),
        Fake::reading(Err(AttemptFailure::new("gst-stream-error-quark", 6))),
    ] {
        let (made, lines) = rig.make(&attachment, &maker, &Cancel::new());
        assert_eq!(
            Ok(SuppliedSound::Failed(transcript_sound::unreadable())),
            made
        );
        assert!(maker.asked().is_empty());
        // Numbers and an error's domain only — never a name, a path or the bytes.
        assert_eq!(1, lines.len());
        assert!(!lines[0].contains("ftyp"));
    }
}

/// A recording no way of making can fit is "too long" without the work.
#[test]
fn what_nothing_can_fit_is_too_long_without_the_work() {
    let long = AttachmentDto {
        duration_ms: Some(60 * 60_000),
        ..video()
    };
    let rig = Rig::new().holding(&long, HELD);
    let maker = Fake::track("aac", Some(64_000), Some(44_100));

    let (made, _) = rig.make(&long, &maker, &Cancel::new());

    assert_eq!(
        Ok(SuppliedSound::Failed(transcript_sound::too_long())),
        made
    );
    assert!(maker.asked().is_empty());
}

/// What came out is JUDGED, not trusted: a picture in it, or bytes that are not MPEG-4, end the
/// asking — "couldn't read"; every way failing outright is "couldn't read" too, and each failure
/// is logged by its domain and code.
#[test]
fn what_came_out_wrong_or_failed_is_unreadable() {
    let attachment = video();
    let rig = Rig::new().holding(&attachment, HELD);

    let pictured = Fake::track("aac", Some(128_000), Some(48_000)).then(Ok(MadeSound {
        bytes: M4A.to_vec(),
        codec: Some("aac".to_string()),
        bitrate: Some(128_000),
        has_video: true,
    }));
    let (made, _) = rig.make(&attachment, &pictured, &Cancel::new());
    assert_eq!(
        Ok(SuppliedSound::Failed(transcript_sound::unreadable())),
        made
    );
    assert_eq!(1, pictured.asked().len());

    let ogg = Fake::track("opus", None, Some(48_000)).then(aac_out(b"OggS\0\0\0\0\0\0\0\0", None));
    let (made, _) = rig.make(&attachment, &ogg, &Cancel::new());
    assert_eq!(
        Ok(SuppliedSound::Failed(transcript_sound::unreadable())),
        made
    );
    assert_eq!(1, ogg.asked().len());

    let failing = Fake::track("aac", Some(128_000), Some(48_000))
        .then(Err(AttemptFailure::new("gst-core-error-quark", 1)))
        .then(Err(AttemptFailure::new("gst-library-error-quark", 3)));
    let (made, lines) = rig.make(&attachment, &failing, &Cancel::new());
    assert_eq!(
        Ok(SuppliedSound::Failed(transcript_sound::unreadable())),
        made
    );
    assert_eq!(2, failing.asked().len());
    assert!(
        lines
            .iter()
            .any(|line| line.contains("gst-core-error-quark 0x00000001"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("gst-library-error-quark 0x00000003"))
    );
}

/// A result over the ceiling is refused, and with nothing else that fits, "couldn't read" — the
/// copy over it falls to the encode first.
#[test]
fn a_result_over_the_ceiling_is_not_taken() {
    let attachment = video();
    let rig = Rig::new().holding(&attachment, HELD);
    let big = vec![0u8; (MIB25 + 1) as usize];
    let mut over = M4A.to_vec();
    over.extend_from_slice(&big);
    let maker = Fake::track("aac", Some(128_000), Some(48_000))
        .then(aac_out(&over, Some(128_000)))
        .then(aac_out(M4A, Some(64_000)));

    let (made, _) = rig.make(&attachment, &maker, &Cancel::new());

    assert_eq!(Ok(SuppliedSound::Made(M4A.to_vec())), made);
    assert_eq!(2, maker.asked().len());
}

/// A download that failed is said as what it was: a gone attachment is the server's own refusal.
#[test]
fn a_failed_download_is_said_as_what_it_was() {
    let attachment = video();
    let rig = Rig::new();
    rig.server.on_status(
        "/attachments/34",
        404,
        Some(r#"{"error": {"code": "attachment_not_found", "message": "gone"}}"#),
    );
    let maker = Fake::track("aac", Some(128_000), Some(48_000));

    let (made, _) = rig.make(&attachment, &maker, &Cancel::new());

    match made {
        Ok(SuppliedSound::Failed(error)) => {
            assert_eq!(error_codes::ATTACHMENT_NOT_FOUND, error.code)
        }
        other => panic!("{other:?}"),
    }
    assert!(maker.asked().is_empty());
}

/// A cancel is a cancel — never a sound, never a failure to say.
#[test]
fn a_cancel_is_a_cancel() {
    let attachment = video();
    let rig = Rig::new().holding(&attachment, HELD);
    let maker = Fake::track("aac", Some(128_000), Some(48_000)).then(aac_out(M4A, Some(128_000)));
    let cancel = Cancel::new();
    cancel.cancel();

    let (made, _) = rig.make(&attachment, &maker, &cancel);

    assert_eq!(Err(OperationCanceled), made);
    assert!(maker.asked().is_empty());
}

/// The whole of it, as the shell wires it (`device_source`): a video's "Show text" in the supplied
/// form, the sound made from the cached file and sent as the one `audio` part.
#[test]
fn the_device_source_hands_the_model_the_sound() -> StoreResult<()> {
    let attachment = video();
    let rig = Rig::new().holding(&attachment, HELD);
    rig.server.on(
        "/chats/42/messages/1338/attachments/34/transcript",
        r#"{"transcript": {"text": "see you at six"}}"#,
    );
    let store = Rc::new(TranscriptStore::new(&Database::open_in_memory()?));
    let model = TranscriptModel::new(
        api(&rig.server, &rig.clock),
        Rc::clone(&store),
        Rc::new(rig.clock.clone()),
    );
    let maker =
        Rc::new(Fake::track("aac", Some(128_000), Some(48_000)).then(aac_out(M4A, Some(128_000))));
    let source = transcript_sound::device_source(
        Rc::clone(&rig.cache),
        attachment,
        MIB25,
        maker.clone(),
        Rc::new(rig.clock.clone()),
        Rc::new(|_line: &str| {}),
    );
    assert_eq!(Some(40_000), source.duration_ms);

    drive(
        &rig.clock,
        model.show(
            42,
            1338,
            34,
            TranscriptForm::Supplied,
            Some(source),
            || false,
            || ready(true),
        ),
    )?;

    assert_eq!(TranscriptLook::open("see you at six"), model.look(34));
    assert!(store.find(34)?.expect("kept").supplied);
    let bodies = rig.server.bodies();
    assert_eq!(1, bodies.len());
    assert!(bodies[0].contains("ftypM4A  the sound"));
    assert_eq!(vec![SoundAttempt::PASSTHROUGH], maker.asked());
    Ok(())
}
