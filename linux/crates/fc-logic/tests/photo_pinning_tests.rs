//! `PhotoPinningTests`: a picture onto the wall — uploaded, previewed, pinned as a photo note — one
//! at a time, with the preview tried again beside the pin when the network lost it.
//!
//! The C# polled with `Task.Delay` for the fire-and-forget retries to land; here they are spawned on
//! the test's `LocalPool` and the pool is run until nothing more can move, and the C#'s `wait` is a
//! `StillClock` (its waits end at once and are recorded).

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;

use fc_core::protocol::error_codes;
use fc_core::rt::Http;
use fc_core::store::{BoardStore, Database, StoreResult};
use fc_core::testing::{FakeClock, StillClock};
use fc_logic::media_outbox::StagedMedia;
use fc_logic::photo_pinning::{PREVIEW_RETRIES, PhotoPinning, PinOutcome};
use time::macros::datetime;

use common::notes::Gated;
use common::server::{Server, api_over};

const UPLOADED: &str =
    r#"{"attachment": {"id": 34, "kind": "photo", "width": 1600, "height": 1200}}"#;

const PINNED: &str = r#"{"note": {"id": 70, "author_id": 7, "kind": "photo", "text": "", "board_seq": 5, "attachment": {"id": 34, "kind": "photo"}}}"#;

fn photo() -> StagedMedia {
    StagedMedia {
        width: Some(1600),
        height: Some(1200),
        preview: Some(vec![9]),
        ..StagedMedia::new("photo", "image/jpeg", vec![1, 2, 3])
    }
}

struct Rig {
    board: Rc<BoardStore>,
    pool: LocalPool,
    pauses: StillClock,
    clock: FakeClock,
}

impl Rig {
    fn new() -> StoreResult<Rig> {
        let cache = Database::open_in_memory()?;
        Ok(Rig {
            board: Rc::new(BoardStore::new(&cache)),
            pool: LocalPool::new(),
            pauses: StillClock::at(datetime!(2026-09-12 12:00 UTC)),
            clock: FakeClock::at(datetime!(2026-09-12 12:00 UTC)),
        })
    }

    fn build(&self, http: Rc<dyn Http>) -> Rc<PhotoPinning> {
        let spawner = self.pool.spawner();
        Rc::new(PhotoPinning::new(
            api_over(http, &self.clock),
            self.board.clone(),
            Rc::new(self.pauses.clone()),
            Rc::new(move |future| spawner.spawn_local(future).expect("spawned")),
        ))
    }

    /// `await pinning.PinAsync(photo, at, color)`, and then whatever it spawned, until nothing more
    /// can move.
    fn pin(
        &mut self,
        pinning: &PhotoPinning,
        photo: &StagedMedia,
        at: (f64, f64),
        color: &str,
    ) -> StoreResult<PinOutcome> {
        let outcome = self.pool.run_until(pinning.pin(photo, at, color));
        self.pool.run_until_stalled();
        outcome
    }
}

fn previews(server: &Server) -> usize {
    server
        .asked()
        .iter()
        .filter(|path| path.ends_with("/preview"))
        .count()
}

#[test]
fn a_photo_is_uploaded_previewed_then_pinned_as_a_photo_note() -> StoreResult<()> {
    let mut rig = Rig::new()?;
    let server = Server::new();
    server
        .on("/attachments", UPLOADED)
        .on_status("/attachments/34/preview", 204, None)
        .on("/families/mine/board/notes", PINNED);
    let pinning = rig.build(Rc::new(server.clone()));

    let outcome = rig.pin(&pinning, &photo(), (0.3, 0.4), "pink")?;

    assert!(outcome.pinned);
    assert_eq!(
        vec![
            "/api/v1/attachments?kind=photo&width=1600&height=1200".to_string(),
            "/api/v1/attachments/34/preview".to_string(),
            "/api/v1/families/mine/board/notes".to_string(),
        ],
        server.asked()
    );
    let note: serde_json::Value =
        serde_json::from_str(server.bodies().last().expect("a body")).expect("JSON");
    assert_eq!(Some(34), note["attachment_id"].as_i64());
    assert_eq!(Some("photo"), note["kind"].as_str());
    assert_eq!(Some(""), note["text"].as_str());
    assert_eq!(Some("pink"), note["color"].as_str());
    assert_eq!(Some("medium"), note["size"].as_str());
    assert!(rig.board.note(70)?.is_some());
    Ok(())
}

/// Best effort, but not best effort once: a preview the network lost is tried twice more.
#[test]
fn a_preview_the_network_lost_is_tried_twice_more_beside_the_pin() -> StoreResult<()> {
    let mut rig = Rig::new()?;
    let down = (
        503,
        Some(r#"{"error": {"code": "internal", "message": "later"}}"#),
    );
    let server = Server::new();
    server
        .on("/attachments", UPLOADED)
        .then("/attachments/34/preview", &[down, down, down, down])
        .on("/families/mine/board/notes", PINNED);
    let pinning = rig.build(Rc::new(server.clone()));

    assert!(rig.pin(&pinning, &photo(), (0.3, 0.4), "pink")?.pinned);

    assert_eq!(3, previews(&server));
    assert_eq!(PREVIEW_RETRIES.to_vec(), rig.pauses.slept());
    Ok(())
}

#[test]
fn a_refused_preview_is_not_tried_again() -> StoreResult<()> {
    let mut rig = Rig::new()?;
    let server = Server::new();
    server
        .on("/attachments", UPLOADED)
        .on_status(
            "/attachments/34/preview",
            400,
            Some(r#"{"error": {"code": "invalid_attachment", "message": "no"}}"#),
        )
        .on("/families/mine/board/notes", PINNED);
    let pinning = rig.build(Rc::new(server.clone()));

    assert!(rig.pin(&pinning, &photo(), (0.3, 0.4), "pink")?.pinned);

    assert_eq!(1, previews(&server));
    assert!(rig.pauses.slept().is_empty());
    Ok(())
}

#[test]
fn a_refused_upload_pins_nothing() -> StoreResult<()> {
    let mut rig = Rig::new()?;
    let server = Server::new();
    server.on_status(
        "/attachments",
        413,
        Some(r#"{"error": {"code": "attachment_too_large", "message": "big"}}"#),
    );
    let pinning = rig.build(Rc::new(server.clone()));

    let outcome = rig.pin(&pinning, &photo(), (0.3, 0.4), "pink")?;

    assert!(!outcome.pinned);
    let error = outcome.error.expect("refused");
    assert_eq!(error_codes::ATTACHMENT_TOO_LARGE, error.code);
    assert_eq!(1, server.asked().len());
    assert_eq!(
        "Couldn't pin that photo. That photo is too large to pin.",
        PhotoPinning::failure_text(&error, &common::english())
    );
    Ok(())
}

#[test]
fn a_full_board_refuses_the_pin_and_nothing_is_applied() -> StoreResult<()> {
    let mut rig = Rig::new()?;
    let server = Server::new();
    server
        .on("/attachments", UPLOADED)
        .on_status("/attachments/34/preview", 204, None)
        .on_status(
            "/families/mine/board/notes",
            409,
            Some(r#"{"error": {"code": "board_full", "message": "full"}}"#),
        );
    let pinning = rig.build(Rc::new(server.clone()));

    let outcome = rig.pin(&pinning, &photo(), (0.3, 0.4), "pink")?;

    assert_eq!(
        Some(error_codes::BOARD_FULL.to_string()),
        outcome.error.map(|error| error.code)
    );
    assert!(rig.board.notes()?.is_empty());
    Ok(())
}

/// One at a time, and said so: a second photo quietly dropped is a photo its sender thinks is on
/// the wall.
#[test]
fn one_photo_is_pinned_at_a_time() -> StoreResult<()> {
    let mut rig = Rig::new()?;
    let server = Server::new();
    server
        .on("/attachments", UPLOADED)
        .on("/families/mine/board/notes", PINNED);
    let gated = Gated::new(server, "/attachments");
    let pinning = rig.build(Rc::new(gated.clone()));

    let first: Rc<RefCell<Option<StoreResult<PinOutcome>>>> = Rc::default();
    {
        let (pinning, first) = (pinning.clone(), first.clone());
        let unpreviewed = StagedMedia {
            preview: None,
            ..photo()
        };
        rig.pool
            .spawner()
            .spawn_local(async move {
                let outcome = pinning.pin(&unpreviewed, (0.3, 0.4), "pink").await;
                *first.borrow_mut() = Some(outcome);
            })
            .expect("spawned");
    }
    rig.pool.run_until_stalled();
    assert!(pinning.pinning());
    assert!(rig.pin(&pinning, &photo(), (0.1, 0.1), "blue")?.busy);

    gated.release();
    rig.pool.run_until_stalled();
    let outcome = first.borrow_mut().take().expect("answered")?;
    assert!(outcome.pinned);
    assert!(!pinning.pinning());
    Ok(())
}

#[test]
fn a_photo_lands_near_the_middle_or_under_the_pointer() {
    let (x, y) = PhotoPinning::scattered(|| 0.5);
    assert!((0.35 - x).abs() < 1e-9 && (0.30 - y).abs() < 1e-9);
    assert!((0.40 - PhotoPinning::scattered(|| 1.0).0).abs() < 1e-9);

    // A medium card centred under the pointer, on a wide wall: 150 x 110.
    let dropped = PhotoPinning::dropped_at((575.0, 855.0), (1000.0, 1600.0));
    assert!((0.5 - dropped.0).abs() < 1e-9 && (0.5 - dropped.1).abs() < 1e-9);
    // Held inside the wall at the corner.
    assert_eq!(
        (0.0, 0.0),
        PhotoPinning::dropped_at((5.0, 5.0), (1000.0, 1600.0))
    );
}
