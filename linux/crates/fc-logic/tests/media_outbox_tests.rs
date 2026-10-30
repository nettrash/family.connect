//! `MediaOutboxTests`: the uploads a queued message owes, and the four ways they end.

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_core::protocol::send_rules::FlushTrigger;
use fc_core::protocol::{
    ApiError, ApiResult, ChatDto, ChatRowDto, MessageDto, MessageResponse, SendPipeline,
    error_codes,
};
use fc_core::rt::LocalBoxFuture;
use fc_core::store::{ChatStore, Database, OutboxRow, OutboxStore, StoreResult};
use fc_core::testing::{FakeClock, FakeFrameSender, StillClock, drive};
use fc_logic::media_outbox::{MediaOutbox, StagedMedia};
use time::OffsetDateTime;
use time::macros::datetime;

use common::media::Staging;
use common::server::{Server, api};

const NOW: OffsetDateTime = datetime!(2026-09-12 12:00 UTC);
const POSTER: [u8; 4] = [0xFF, 0xD8, 0xFF, 0xE0];

struct Rig {
    cache: Rc<Database>,
    outbox: Rc<OutboxStore>,
    clock: FakeClock,
    rows: Cell<u32>,
}

impl Rig {
    fn new() -> StoreResult<Rig> {
        let cache = Database::open_in_memory()?;
        ChatStore::new(&cache).with_me(|| 7).replace(
            &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
            None,
        )?;
        Ok(Rig {
            outbox: Rc::new(OutboxStore::new(&cache)),
            cache,
            clock: FakeClock::at(NOW),
            rows: Cell::new(0),
        })
    }

    fn build(&self, server: &Server, staging: &Staging) -> Rc<MediaOutbox> {
        Rc::new(MediaOutbox::new(
            self.outbox.clone(),
            api(server, &self.clock),
            Rc::new(staging.clone()),
        ))
    }

    /// `Queue(files…)`: a row that owes these files, under a key of its own.
    fn queue(&self, files: &[&str]) -> StoreResult<OutboxRow> {
        self.rows.set(self.rows.get() + 1);
        let files: Vec<String> = files.iter().map(|file| file.to_string()).collect();
        let row = OutboxRow {
            pending_files: Some(files.clone()),
            staged_files: Some(files),
            ..OutboxRow::new(&format!("row-{}", self.rows.get()), 42, "look at this")
        };
        self.outbox.queue(&row)?;
        Ok(row)
    }

    fn held(&self, row: &OutboxRow) -> StoreResult<OutboxRow> {
        Ok(self.outbox.find(&row.client_msg_id)?.expect("still queued"))
    }

    fn push(&self, media: &MediaOutbox) -> StoreResult<usize> {
        drive(&self.clock, media.push())
    }

    /// A pipeline over a socket that is never connected (`NeverConnected`) and a clock that never
    /// waits, posting through `post`.
    fn pipeline(
        &self,
        post: impl Fn(OutboxRow) -> ApiResult<MessageResponse> + 'static,
    ) -> SendPipeline {
        let socket = FakeFrameSender::new();
        socket.set_connected(false);
        let post = Rc::new(post);
        SendPipeline::new(
            Rc::new(socket),
            self.outbox.clone(),
            Rc::new(ChatStore::new(&self.cache).with_me(|| 7)),
            Rc::new(
                move |row: OutboxRow| -> LocalBoxFuture<'static, ApiResult<MessageResponse>> {
                    let answer = post(row);
                    Box::pin(async move { answer })
                },
            ),
            Rc::new(StillClock::at(NOW)),
        )
    }
}

fn uploaded(id: i64) -> String {
    format!(r#"{{"attachment": {{"id": {id}, "kind": "photo", "width": 1600, "height": 1200}}}}"#)
}

fn previews_asked(server: &Server, id: i64) -> usize {
    let preview = format!("/api/v1/attachments/{id}/preview");
    server
        .asked()
        .iter()
        .filter(|path| **path == preview)
        .count()
}

#[test]
fn every_owed_file_goes_and_each_landing_is_remembered_as_it_happens() -> StoreResult<()> {
    let rig = Rig::new()?;
    let row = rig.queue(&["a.jpg", "b.jpg"])?;
    let server = Server::new();
    server.then(
        "/attachments",
        &[(200, Some(&uploaded(34))), (200, Some(&uploaded(35)))],
    );
    let media = rig.build(
        &server,
        &Staging::new().with_photo("a.jpg").with_photo("b.jpg"),
    );

    assert_eq!(2, rig.push(&media)?);

    let held = rig.held(&row)?;
    assert_eq!(Some(vec![34, 35]), held.attachment_ids);
    assert!(!held.owes_uploads());
    // The facts ride in the query, the bytes in the body.
    assert!(
        server
            .asked()
            .iter()
            .all(|path| path.contains("kind=photo"))
    );
    assert!(
        server
            .asked()
            .iter()
            .all(|path| path.contains("width=1600"))
    );
    Ok(())
}

/// A place is its three numbers: they go in the query, and nothing goes in the body.
#[test]
fn a_place_is_uploaded_as_its_numbers_with_no_body() -> StoreResult<()> {
    let rig = Rig::new()?;
    let row = rig.queue(&["here"])?;
    let staging = Staging::new();
    staging.put(
        "here",
        StagedMedia {
            latitude: Some(55.7558),
            longitude: Some(37.6173),
            accuracy_m: Some(12.4),
            ..StagedMedia::new("location", "", Vec::new())
        },
    );
    let server = Server::new();
    server.on("/attachments", &uploaded(61));
    let media = rig.build(&server, &staging);

    assert_eq!(1, rig.push(&media)?);

    assert_eq!(Some(vec![61]), rig.held(&row)?.attachment_ids);
    assert_eq!(
        vec![
            "/api/v1/attachments?kind=location&latitude=55.7558000&longitude=37.6173000&accuracy_m=12"
        ],
        server.asked()
    );
    assert!(server.bodies().is_empty());
    Ok(())
}

/// A voice note goes up with the shape measured from it (docs/protocol.md, "A voice note's
/// waveform"), in the query beside its length; one with no shape goes up exactly as before.
#[test]
fn a_voice_notes_waveform_goes_up_with_it() -> StoreResult<()> {
    const WIRE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";
    let rig = Rig::new()?;
    rig.queue(&["note"])?;
    rig.queue(&["plain"])?;
    let staging = Staging::new();
    staging.put(
        "note",
        StagedMedia {
            duration_ms: Some(4200),
            waveform: Some(WIRE.to_string()),
            ..StagedMedia::new("audio", "audio/mp4", vec![1, 2, 3])
        },
    );
    staging.put(
        "plain",
        StagedMedia {
            duration_ms: Some(4200),
            ..StagedMedia::new("audio", "audio/mp4", vec![1, 2, 3])
        },
    );
    let server = Server::new();
    server.on("/attachments", &uploaded(70));
    let media = rig.build(&server, &staging);

    assert_eq!(2, rig.push(&media)?);

    assert_eq!(
        vec![
            format!("/api/v1/attachments?kind=audio&duration_ms=4200&waveform={WIRE}"),
            "/api/v1/attachments?kind=audio&duration_ms=4200".to_string(),
        ],
        server.asked()
    );
    Ok(())
}

/// A place whose numbers did not survive is as lost as a photo whose bytes did not: the row fails.
#[test]
fn a_place_without_its_numbers_fails_the_row() -> StoreResult<()> {
    let rig = Rig::new()?;
    let row = rig.queue(&["here"])?;
    let staging = Staging::new();
    staging.put(
        "here",
        StagedMedia {
            latitude: Some(55.7558),
            ..StagedMedia::new("location", "", Vec::new())
        },
    );
    let server = Server::new();
    let media = rig.build(&server, &staging);

    assert_eq!(0, rig.push(&media)?);

    assert!(rig.held(&row)?.failed());
    assert!(server.asked().is_empty());
    Ok(())
}

/// A crash halfway through a four-photo message must cost the remaining three, not all four: an id
/// that landed is kept and reused within the server's grace.
#[test]
fn a_file_that_failed_transiently_leaves_the_rest_owed_and_the_landed_ones_kept() -> StoreResult<()>
{
    let rig = Rig::new()?;
    let row = rig.queue(&["a.jpg", "b.jpg", "c.jpg"])?;
    let server = Server::new();
    server.then(
        "/attachments",
        &[(200, Some(&uploaded(34))), (502, Some("<html>502</html>"))],
    );
    let media = rig.build(
        &server,
        &Staging::new()
            .with_photo("a.jpg")
            .with_photo("b.jpg")
            .with_photo("c.jpg"),
    );

    assert_eq!(1, rig.push(&media)?);

    let held = rig.held(&row)?;
    assert_eq!(Some(vec![34]), held.attachment_ids);
    assert_eq!(
        Some(vec!["b.jpg".to_string(), "c.jpg".to_string()]),
        held.pending_files
    );
    // Still queued, not failed: a transient upload failure is not a refusal.
    assert!(!held.failed());
    Ok(())
}

/// A row whose bytes this device can no longer find is FAILED rather than retried: an id cannot
/// recover a picture, and a backoff has nothing to wait for.
#[test]
fn media_this_device_can_no_longer_find_fails_the_row_outright() -> StoreResult<()> {
    let rig = Rig::new()?;
    let row = rig.queue(&["gone.jpg"])?;
    let server = Server::new();
    let media = rig.build(&server, &Staging::new());
    let refusals: Rc<RefCell<Vec<ApiError>>> = Rc::default();
    let heard = refusals.clone();
    media
        .refused
        .subscribe(move |(_, error)| heard.borrow_mut().push(error.clone()));

    assert_eq!(0, rig.push(&media)?);

    assert_eq!(
        Some(error_codes::MEDIA_MISSING),
        rig.held(&row)?.failed_code.as_deref()
    );
    let refusals = refusals.borrow();
    assert_eq!(1, refusals.len());
    assert_eq!(error_codes::MEDIA_MISSING, refusals[0].code);
    // Nothing was even attempted: there was nothing to send.
    assert!(server.asked().is_empty());
    Ok(())
}

/// A refusal about the FILE ITSELF — too large, not the kind it claimed — would refuse again with
/// the same bytes, so the row fails and the person is told.
#[test]
fn a_file_the_server_refuses_fails_the_row_and_says_why() -> StoreResult<()> {
    let rig = Rig::new()?;
    let row = rig.queue(&["huge.jpg"])?;
    let server = Server::new();
    server.on_status(
        "/attachments",
        413,
        Some(r#"{"error": {"code": "attachment_too_large", "message": "no"}}"#),
    );
    let media = rig.build(&server, &Staging::new().with_photo("huge.jpg"));
    let refusals: Rc<RefCell<Vec<ApiError>>> = Rc::default();
    let heard = refusals.clone();
    media
        .refused
        .subscribe(move |(_, error)| heard.borrow_mut().push(error.clone()));

    assert_eq!(0, rig.push(&media)?);

    assert_eq!(
        Some(error_codes::ATTACHMENT_TOO_LARGE),
        rig.held(&row)?.failed_code.as_deref()
    );
    let refusals = refusals.borrow();
    assert_eq!(1, refusals.len());
    assert_eq!(error_codes::ATTACHMENT_TOO_LARGE, refusals[0].code);
    Ok(())
}

#[test]
fn a_row_that_owes_nothing_and_one_already_failed_are_passed_over() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.outbox
        .queue(&OutboxRow::new("just-words", 42, "just words"))?;
    let failed = rig.queue(&["a.jpg"])?;
    rig.outbox
        .refuse(&failed.client_msg_id, error_codes::ATTACHMENT_TOO_LARGE)?;
    let server = Server::new();
    let media = rig.build(&server, &Staging::new().with_photo("a.jpg"));

    assert_eq!(0, rig.push(&media)?);

    assert!(server.asked().is_empty());
    Ok(())
}

/// The staging area cannot outlive the sends it is for: what no queued row names any more is
/// forgotten, and what a row still owes is kept.
#[test]
fn sweeping_forgets_the_bytes_of_sends_that_are_over() -> StoreResult<()> {
    let rig = Rig::new()?;
    let row = rig.queue(&["a.jpg"])?;
    let staging = Staging::new().with_photo("a.jpg").with_photo("orphan.jpg");
    let media = rig.build(&Server::new(), &staging);

    media.sweep()?;

    assert!(staging.holds("a.jpg"));
    assert!(!staging.holds("orphan.jpg"));

    // The send lands, the row goes, and its bytes go with it on the next sweep.
    rig.outbox.delivered(&row.client_msg_id)?;
    media.sweep()?;
    assert!(staging.files.borrow().is_empty());
    Ok(())
}

/// A SWEEP KEEPS WHAT IS STAGED, NOT WHAT IS OWED. A row two of whose three uploads have landed
/// still needs those two files: the server can sweep an unclaimed upload, and `attachment_expired`
/// means push it again.
#[test]
fn sweeping_keeps_the_files_of_uploads_that_have_already_landed() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.queue(&["a.jpg", "b.jpg"])?;
    let staging = Staging::new().with_photo("a.jpg").with_photo("b.jpg");
    let server = Server::new();
    server.then("/attachments", &[(200, Some(&uploaded(34)))]);
    let media = rig.build(&server, &staging);
    rig.push(&media)?;

    media.sweep()?;

    assert!(staging.holds("a.jpg"));
    assert!(staging.holds("b.jpg"));
    Ok(())
}

/// A FLUSH PUSHES WHAT IS OWED BEFORE IT POSTS ANYTHING. A row that owes a byte is passed over by
/// the pipeline, and nothing else is going to move those bytes — so a flush that skipped the
/// uploads would pass over every media message for ever.
#[test]
fn a_flush_pushes_the_uploads_before_it_posts_the_message() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.queue(&["a.jpg"])?;
    let server = Server::new();
    server.on("/attachments", &uploaded(34));
    let media = rig.build(&server, &Staging::new().with_photo("a.jpg"));
    let posted: Rc<RefCell<Vec<Option<Vec<i64>>>>> = Rc::default();
    let seen = posted.clone();
    let pipeline = rig
        .pipeline(move |sending| {
            seen.borrow_mut().push(sending.attachment_ids.clone());
            ApiResult::success(
                MessageResponse {
                    message: MessageDto::new(
                        1338,
                        42,
                        7,
                        Some(&sending.client_msg_id),
                        &sending.body,
                        "2026-09-12T12:00:00Z",
                    ),
                },
                201,
            )
        })
        .with_uploads(media.pump());

    assert_eq!(
        1,
        drive(&rig.clock, pipeline.flush(FlushTrigger::SocketConnected))?
    );

    // One flush: the bytes went, and then the message went WITH the id they became.
    assert_eq!(vec![Some(vec![34])], *posted.borrow());
    assert!(rig.outbox.all()?.is_empty());
    Ok(())
}

/// `attachment_expired` means UPLOAD IT AGAIN. The dead ids go, every staged file is owed again,
/// and the same `client_msg_id` is re-sent — which is what makes the repeat safe.
#[test]
fn an_expired_upload_is_pushed_again_rather_than_given_up_on() -> StoreResult<()> {
    let rig = Rig::new()?;
    let row = rig.queue(&["a.jpg"])?;
    let server = Server::new();
    server.then(
        "/attachments",
        &[(200, Some(&uploaded(34))), (200, Some(&uploaded(77)))],
    );
    let media = rig.build(&server, &Staging::new().with_photo("a.jpg"));
    rig.push(&media)?;
    assert_eq!(Some(vec![34]), rig.held(&row)?.attachment_ids);

    // The send is refused because the server swept the upload before the message named it.
    let pipeline = rig.pipeline(|_| {
        ApiResult::failure(ApiError::new(error_codes::ATTACHMENT_EXPIRED, "gone", 409))
    });
    drive(&rig.clock, pipeline.flush(FlushTrigger::UserRetried))?;

    let owing = rig.held(&row)?;
    assert!(!owing.failed());
    assert_eq!(None, owing.attachment_ids);
    assert_eq!(Some(vec!["a.jpg".to_string()]), owing.pending_files);

    // And the pump pushes it again, to a new id.
    assert_eq!(1, rig.push(&media)?);
    assert_eq!(Some(vec![77]), rig.held(&row)?.attachment_ids);
    Ok(())
}

/// …unless the bytes are gone, and then it is the one refusal with no way out.
#[test]
fn an_expired_upload_whose_bytes_are_gone_fails_instead() -> StoreResult<()> {
    let rig = Rig::new()?;
    let row = OutboxRow {
        attachment_ids: Some(vec![34]),
        ..OutboxRow::new("expired", 42, "look at this")
    };
    rig.outbox.queue(&row)?;
    let pipeline = rig.pipeline(|_| {
        ApiResult::failure(ApiError::new(error_codes::ATTACHMENT_EXPIRED, "gone", 409))
    });

    drive(&rig.clock, pipeline.flush(FlushTrigger::UserRetried))?;

    assert_eq!(
        Some(error_codes::ATTACHMENT_EXPIRED),
        rig.held(&row)?.failed_code.as_deref()
    );
    Ok(())
}

/// A preview follows its upload, to the id that upload became.
#[test]
fn a_preview_follows_its_upload() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.queue(&["clip.mp4"])?;
    let server = Server::new();
    server
        .on("/attachments", &uploaded(34))
        .on_status("/attachments/34/preview", 204, None);
    let media = rig.build(
        &server,
        &Staging::new().with("clip.mp4", "video", "video/mp4", Some(&POSTER)),
    );

    assert_eq!(1, rig.push(&media)?);

    assert_eq!(
        Some("/api/v1/attachments/34/preview"),
        server.asked().last().map(String::as_str)
    );
    assert_eq!(0, media.owed_previews());
    Ok(())
}

#[test]
fn media_without_a_preview_sends_none() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.queue(&["notes.pdf"])?;
    let server = Server::new();
    server.on("/attachments", &uploaded(34));
    let media = rig.build(
        &server,
        &Staging::new().with("notes.pdf", "file", "application/pdf", None),
    );

    rig.push(&media)?;

    assert_eq!(0, previews_asked(&server, 34));
    Ok(())
}

/// NOT BEST EFFORT ONCE: a poster that failed is sent again on later flushes — from the bytes held
/// here, since the staged file goes with its send — and a bounded number of times, no more.
#[test]
fn a_failed_preview_is_sent_again_a_bounded_number_of_times() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.queue(&["clip.mp4"])?;
    let staging = Staging::new().with("clip.mp4", "video", "video/mp4", Some(&POSTER));
    let failing = (
        503,
        Some(r#"{"error": {"code": "internal", "message": "later"}}"#),
    );
    let server = Server::new();
    server.on("/attachments", &uploaded(34)).then(
        "/attachments/34/preview",
        &[failing, failing, failing, failing],
    );
    let media = rig.build(&server, &staging);

    rig.push(&media)?;
    staging.files.borrow_mut().clear();
    assert_eq!(1, media.owed_previews());

    for _ in 0..5 {
        rig.push(&media)?;
    }

    assert_eq!(
        usize::try_from(MediaOutbox::PREVIEW_ATTEMPTS).unwrap(),
        previews_asked(&server, 34)
    );
    assert_eq!(0, media.owed_previews());
    Ok(())
}

#[test]
fn a_preview_that_lands_later_is_no_longer_owed() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.queue(&["clip.mp4"])?;
    let server = Server::new();
    server.on("/attachments", &uploaded(34)).then(
        "/attachments/34/preview",
        &[
            (
                503,
                Some(r#"{"error": {"code": "internal", "message": "later"}}"#),
            ),
            (204, None),
        ],
    );
    let media = rig.build(
        &server,
        &Staging::new().with("clip.mp4", "video", "video/mp4", Some(&POSTER)),
    );

    rig.push(&media)?;
    rig.push(&media)?;
    rig.push(&media)?;

    assert_eq!(2, previews_asked(&server, 34));
    assert_eq!(0, media.owed_previews());
    Ok(())
}

/// A REFUSED preview is not tried again: the same bytes would be refused the same way.
#[test]
fn a_refused_preview_is_not_sent_again() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.queue(&["clip.mp4"])?;
    let server = Server::new();
    server.on("/attachments", &uploaded(34)).on_status(
        "/attachments/34/preview",
        400,
        Some(r#"{"error": {"code": "invalid_attachment", "message": "not a jpeg"}}"#),
    );
    let media = rig.build(
        &server,
        &Staging::new().with("clip.mp4", "video", "video/mp4", Some(&POSTER)),
    );

    rig.push(&media)?;
    rig.push(&media)?;

    assert_eq!(1, previews_asked(&server, 34));
    assert_eq!(0, media.owed_previews());
    Ok(())
}

/// A transport that leaves a preview unanswered while it hangs, and hands everything else — and
/// every preview once it stops hanging — to a server.
struct PreviewsHang {
    server: Server,
    hanging: Rc<Cell<bool>>,
}

impl fc_core::rt::Http for PreviewsHang {
    fn send(
        &self,
        request: fc_core::rt::HttpRequest,
    ) -> LocalBoxFuture<'_, Result<fc_core::rt::HttpResponse, fc_core::rt::TransportError>> {
        if self.hanging.get() && request.url.ends_with("/preview") {
            return Box::pin(std::future::pending());
        }
        self.server.send(request)
    }
}

/// Not a C# test: a push DROPPED while a poster is on its way — the C#'s cancel — leaves that poster
/// owed with the tries it had, and the next push sends it. (The C#'s cancel is an exception out of
/// the upload, which loses it.)
#[test]
fn a_push_dropped_while_a_preview_is_on_its_way_leaves_it_owed() -> StoreResult<()> {
    use futures::FutureExt;

    let rig = Rig::new()?;
    rig.queue(&["clip.mp4"])?;
    let server = Server::new();
    server
        .on("/attachments", &uploaded(34))
        .on_status("/attachments/34/preview", 204, None);
    let hanging = Rc::new(Cell::new(true));
    let transport = PreviewsHang {
        server: server.clone(),
        hanging: hanging.clone(),
    };
    let media = MediaOutbox::new(
        rig.outbox.clone(),
        common::server::api_over(Rc::new(transport), &rig.clock),
        Rc::new(Staging::new().with("clip.mp4", "video", "video/mp4", Some(&POSTER))),
    );

    // One turn: the upload lands, the poster goes, and the push is dropped while it waits.
    assert!(media.push().now_or_never().is_none());
    assert!(rig.outbox.all()?[0].attachment_ids.is_some());
    assert_eq!(1, media.owed_previews());

    // The next push sends it, and it is owed no more.
    hanging.set(false);
    assert_eq!(0, rig.push(&media)?);
    assert_eq!(1, previews_asked(&server, 34));
    assert_eq!(0, media.owed_previews());
    Ok(())
}
