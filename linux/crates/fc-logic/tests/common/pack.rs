//! What `PackModelTests` builds: a server that answers by METHOD and path and keeps the BYTES of
//! every body (a sticker is its bytes) — the C#'s `Wire` — the cache it opens (member 7, the family
//! chat 42, a pack of three under an 8 KiB ceiling), and the sticker bytes it writes by hand: a
//! WebP as its container says it, a PNG's header, a GIF of so many pictures.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use fc_core::protocol::{ApiClient, AttachmentDto, ChatDto, ChatRowDto, PackItemDto};
use fc_core::rt::{Http, HttpRequest, HttpResponse, LocalBoxFuture, TransportError};
use fc_core::store::{ChatStore, Database, OutboxStore, PackLimits, PackStore, StoreResult};
use fc_core::testing::FakeClock;
use fc_logic::attachment_cache::AttachmentCache;
use fc_logic::pack::PackModel;
use time::OffsetDateTime;
use time::macros::datetime;

use super::media::Blobs;
use super::server::api_over;

pub const ME: i64 = 7;
pub const CEILING: i64 = 8 * 1024;
pub const NOW: OffsetDateTime = datetime!(2026-09-13 10:00 UTC);

/// One request as it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
}

type Answers = RefCell<VecDeque<(u16, Option<Vec<u8>>)>>;

#[derive(Default)]
struct WireState {
    routes: RefCell<Vec<(String, String, Answers)>>,
    requests: RefCell<Vec<Asked>>,
    seen: RefCell<Option<Rc<dyn Fn(&Asked)>>>,
}

/// Answers by method and path, and keeps the bytes of every body (`Wire`). Clones share it.
#[derive(Clone, Default)]
pub struct Wire(Rc<WireState>);

impl Wire {
    pub fn new() -> Self {
        Wire::default()
    }

    /// `On(method, endpoint, answers…)`: the answers in turn, the last standing for every later
    /// ask. `"/attachments"` is `/api/v1/attachments`, matched query aside.
    pub fn on(&self, method: &str, endpoint: &str, answers: &[(u16, Option<&str>)]) -> &Self {
        let answers = answers
            .iter()
            .map(|(status, json)| (*status, json.map(|json| json.as_bytes().to_vec())))
            .collect();
        self.0.routes.borrow_mut().push((
            method.to_string(),
            format!("/api/v1{endpoint}"),
            RefCell::new(answers),
        ));
        self
    }

    /// `Bytes(attachmentId, bytes)`: `GET /attachments/{id}` answers these bytes.
    pub fn bytes(&self, attachment_id: i64, bytes: &[u8]) -> &Self {
        self.0.routes.borrow_mut().push((
            "GET".to_string(),
            format!("/api/v1/attachments/{attachment_id}"),
            RefCell::new(VecDeque::from([(200, Some(bytes.to_vec()))])),
        ));
        self
    }

    /// `Seen`: called with each request as it arrives, BEFORE it is answered — where a frame that
    /// beats the answer lands.
    pub fn on_seen(&self, seen: impl Fn(&Asked) + 'static) {
        *self.0.seen.borrow_mut() = Some(Rc::new(seen));
    }

    /// Every request so far, oldest first (`Requests`).
    pub fn requests(&self) -> Vec<Asked> {
        self.0.requests.borrow().clone()
    }

    /// `"POST /api/v1/attachments?kind=photo"` for each request (`Paths`).
    pub fn paths(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|asked| format!("{} {}", asked.method, asked.path))
            .collect()
    }
}

/// The path and query of an absolute URL: everything from the first `/` after `://`.
fn path_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.find('/').map_or("/", |at| &rest[at..])
}

impl Http for Wire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> LocalBoxFuture<'_, Result<HttpResponse, TransportError>> {
        let path = path_of(&request.url).to_string();
        let asked = Asked {
            method: request.method.as_str().to_string(),
            path: path.clone(),
            body: request.body.clone().unwrap_or_default(),
            content_type: request
                .header_values("Content-Type")
                .first()
                .map(|value| value.to_string()),
        };
        self.0.requests.borrow_mut().push(asked.clone());
        let seen = self.0.seen.borrow().clone();
        if let Some(seen) = seen {
            seen(&asked);
        }
        let endpoint = path.split('?').next().unwrap_or(&path).to_string();
        let mut answer = None;
        for (method, wanted, answers) in self.0.routes.borrow().iter() {
            if *method == asked.method && *wanted == endpoint {
                let mut answers = answers.borrow_mut();
                // The last answer stands for every later ask.
                let (status, body) = if answers.len() > 1 {
                    answers.pop_front().expect("an answer")
                } else {
                    answers.front().cloned().expect("an answer")
                };
                answer = Some(HttpResponse {
                    status,
                    headers: Vec::new(),
                    body: body.unwrap_or_default(),
                });
                break;
            }
        }
        let outcome = answer.ok_or_else(|| {
            TransportError::Unreachable(format!("no route for {} {path}", asked.method))
        });
        Box::pin(async move { outcome })
    }
}

/// The C# test class's constructor: an in-memory cache read by member 7 holding the family chat
/// 42, a pack of at most three items under an 8 KiB ceiling, and a clock at `Now`.
pub struct PackRig {
    pub cache: Rc<Database>,
    pub pack: Rc<PackStore>,
    pub chats: Rc<ChatStore>,
    pub outbox: Rc<OutboxStore>,
    pub clock: FakeClock,
}

/// One model built over the rig (`Rig`): the model, the wire, the blobs and the client.
pub struct Built {
    pub model: PackModel,
    pub wire: Wire,
    pub blobs: Blobs,
    pub api: Rc<ApiClient>,
}

impl PackRig {
    pub fn new() -> StoreResult<PackRig> {
        let cache = Database::open_in_memory()?;
        let pack = Rc::new(PackStore::new(&cache));
        let chats = Rc::new(ChatStore::new(&cache).with_me(|| ME));
        let outbox = Rc::new(OutboxStore::new(&cache));
        chats.replace(
            &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
            None,
        )?;
        pack.set_limits(Some(PackLimits {
            max_items: 3,
            max_item_bytes: CEILING,
        }))?;
        Ok(PackRig {
            cache,
            pack,
            chats,
            outbox,
            clock: FakeClock::at(NOW),
        })
    }

    /// `Build(wire)`: a client over the wire, a cache of blobs, and the model.
    pub fn build(&self, wire: &Wire) -> Built {
        let api = api_over(Rc::new(wire.clone()), &self.clock);
        let blobs = Blobs::new();
        let attachments = Rc::new(AttachmentCache::new(api.clone(), Rc::new(blobs.clone())));
        Built {
            model: PackModel::new(
                self.pack.clone(),
                self.chats.clone(),
                api.clone(),
                attachments,
                Rc::new(self.clock.clone()),
            ),
            wire: wire.clone(),
            blobs,
            api,
        }
    }
}

/// A WebP as its container says it: an extended header with a canvas, and padding to a length
/// (`PackModelTests.WebP`: alpha flag set, the body filled with `fill`).
pub fn webp(width: u32, height: u32, animated: bool, length: usize, fill: u8) -> Vec<u8> {
    let mut file = vec![fill; length.max(30)];
    file[0..4].copy_from_slice(b"RIFF");
    let riff = u32::try_from(file.len() - 8).expect("a small file");
    file[4..8].copy_from_slice(&riff.to_le_bytes());
    file[8..12].copy_from_slice(b"WEBP");
    file[12..16].copy_from_slice(b"VP8X");
    file[16..20].copy_from_slice(&10u32.to_le_bytes());
    file[20] = 0x10 | if animated { 0x02 } else { 0 };
    file[21] = 0;
    file[22] = 0;
    file[23] = 0;
    write24(&mut file[24..27], width - 1);
    write24(&mut file[27..30], height - 1);
    file
}

/// `WebP()` with every default: 512 × 512, still, 64 bytes, filled with 0xAB.
pub fn webp_default() -> Vec<u8> {
    webp(512, 512, false, 64, 0xAB)
}

/// `PackModelTests.Png(width, height, length)`: the signature and an `IHDR`.
pub fn png(width: u32, height: u32, length: usize) -> Vec<u8> {
    let mut file = vec![0u8; length.max(33)];
    file[0..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    file[8..12].copy_from_slice(&13u32.to_be_bytes());
    file[12..16].copy_from_slice(b"IHDR");
    file[16..20].copy_from_slice(&width.to_be_bytes());
    file[20..24].copy_from_slice(&height.to_be_bytes());
    file
}

/// A 24-bit little-endian number, as `VP8X` writes a canvas side.
fn write24(at: &mut [u8], value: u32) {
    at.copy_from_slice(&value.to_le_bytes()[..3]);
}

/// `Item(id, seq, by, size, mime, label, hasPreview)`: an item whose picture is attachment
/// `70 + id`, 512 × 512.
pub fn item_with(
    id: i64,
    seq: i64,
    by: i64,
    size: Option<i64>,
    mime: &str,
    has_preview: bool,
) -> PackItemDto {
    PackItemDto {
        id,
        added_by: by,
        attachment: Some(AttachmentDto {
            mime: Some(mime.to_string()),
            size,
            width: Some(512),
            height: Some(512),
            has_preview,
            ..AttachmentDto::new(70 + id, "photo")
        }),
        created_at: Some("2026-09-13T10:00:00Z".to_string()),
        pack_seq: seq,
        ..PackItemDto::new(id)
    }
}

/// `Item(id, seq)`: mine, 64 bytes, a WebP.
pub fn item(id: i64, seq: i64) -> PackItemDto {
    item_with(id, seq, ME, Some(64), "image/webp", false)
}

/// `Item(id, seq, by: by)`.
pub fn item_by(id: i64, seq: i64, by: i64) -> PackItemDto {
    item_with(id, seq, by, Some(64), "image/webp", false)
}

/// `Item(id, seq, size: size)`.
pub fn item_sized(id: i64, seq: i64, size: usize) -> PackItemDto {
    item_with(
        id,
        seq,
        ME,
        Some(i64::try_from(size).expect("a small file")),
        "image/webp",
        false,
    )
}

/// `Uploaded(id, mime)`: the answer to an upload.
pub fn uploaded(id: i64, mime: &str) -> String {
    format!(
        r#"{{"attachment": {{"id": {id}, "kind": "photo", "mime": "{mime}", "size": 64, "width": 512, "height": 512}}}}"#
    )
}

/// `Claimed(id, attachment, seq, by)`: the answer to a claim.
pub fn claimed(id: i64, attachment: i64, seq: i64, by: i64) -> String {
    format!(
        r#"{{"item": {{"id": {id}, "added_by": {by},
                  "attachment": {{"id": {attachment}, "kind": "photo", "mime": "image/webp", "size": 64, "width": 512, "height": 512}},
                  "created_at": "2026-09-13T10:00:00Z", "pack_seq": {seq}}}}}"#
    )
}

/// `Refusal(code)`: an error body.
pub fn refusal(code: &str) -> String {
    format!(r#"{{"error": {{"code": "{code}", "message": "…"}}}}"#)
}

/// A sticker a message carried: `new AttachmentDto(id, "photo", mime, size, …, Sticker: true)`.
pub fn sent_sticker(id: i64, mime: &str, size: Option<i64>) -> AttachmentDto {
    AttachmentDto {
        mime: Some(mime.to_string()),
        size,
        sticker: true,
        ..AttachmentDto::new(id, "photo")
    }
}
