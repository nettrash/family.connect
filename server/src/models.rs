//! Response DTOs and row-mapping helpers.
//!
//! DTO field names and shapes mirror protocol.md's "Objects" section exactly
//! — the WS frame unit tests in ws.rs pin them against literal JSON from the
//! document. Row mapping is hand-written `from_row(&PgRow)` (no
//! `query_as!`/offline metadata): queries here are dynamic enough that the
//! macro machinery would cost more than it saves, and `.get("col")` panics
//! loudly on a schema/query mismatch, which is a programmer error we want to
//! hear about in tests, not paper over.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::postgres::PgRow;
use time::OffsetDateTime;
use uuid::Uuid;

/// `User` object: the public identity of an account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Bumped on every profile-picture upload, `0` when there is none —
    /// the clients' cache key for `GET /users/{id}/avatar`.
    pub avatar_version: i64,
    /// Present when (and only when) this member has set one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub birthday: Option<Birthday>,
    /// Present and true ONLY on a scrubbed account, so a live user never
    /// carries `"deleted": false` (protocol.md, "Deleting an account").
    /// Same idiom as [`Note::deleted`].
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub deleted: bool,
}

impl User {
    /// Map a row exposing `id, username, display_name, created_at,
    /// avatar_version, birthday_month, birthday_day`, and optionally
    /// `deleted_at`.
    pub fn from_row(row: &PgRow) -> Self {
        Self {
            id: row.get("id"),
            username: row.get("username"),
            display_name: row.get("display_name"),
            created_at: row.get("created_at"),
            avatar_version: row.get("avatar_version"),
            birthday: Birthday::from_row(row),
            deleted: deleted_from_row(row),
        }
    }
}

/// Whether the row describes a scrubbed account: `users.deleted_at IS NOT
/// NULL`, and nothing else (protocol.md, "Deleting an account").
///
/// `try_get`, deliberately, exactly as the attachment columns are read. The
/// column arrived with migration 0023 and every SELECT written before it
/// simply does not name it; an absent column reads as a LIVE account, which
/// is what all of those queries were already saying. `.get` would panic
/// instead, and a panic inside a handler surfaces as a dropped connection
/// with no error body.
///
/// THE COLUMN NAME IS BARE, AND `notes` HAS A `deleted_at` TOO (0008, where
/// it marks a note's tombstone). sqlx resolves by OUTPUT-column name, not by
/// table, so a SELECT that joined `users` with `notes` and named both would
/// hand this whichever one the planner put first — and a live member would
/// read as deleted the moment their note was. No query in the server joins
/// those two tables today; any future one MUST alias
/// (`u.deleted_at AS deleted_at`, or rename the note's), because nothing
/// here can tell the two apart.
pub fn deleted_from_row(row: &PgRow) -> bool {
    row.try_get::<Option<OffsetDateTime>, _>("deleted_at")
        .ok()
        .flatten()
        .is_some()
}

/// `Birthday` object: a day and a month, and no year (protocol.md,
/// "Birthdays").
///
/// One struct rather than two fields on the owning object because the two
/// halves are a single fact — a birthday is set or it is not, and "month
/// but no day" is not a state any reader should have to handle. The
/// database enforces the same equivalence (migration 0018), so this type
/// is the only shape either side can produce.
///
/// `i16` because the columns are `SMALLINT`; the range is the validator's
/// business and the constraint's, not the type's.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Birthday {
    pub month: i16,
    pub day: i16,
}

impl Birthday {
    /// Read `birthday_month`/`birthday_day` from a row, as the one or the
    /// none they always are together.
    ///
    /// Both columns must be in the SELECT: `.get` panics on a name the
    /// query did not ask for, which is the loud failure this crate wants
    /// when a column list and a DTO drift apart.
    pub fn from_row(row: &PgRow) -> Option<Self> {
        let month: Option<i16> = row.get("birthday_month");
        let day: Option<i16> = row.get("birthday_day");
        match (month, day) {
            (Some(month), Some(day)) => Some(Self { month, day }),
            // Half a birthday cannot be written (migration 0018), so this
            // arm is only ever reached by "nobody set one".
            _ => None,
        }
    }
}

/// Abbreviated user used inside WS membership frames — protocol.md shows
/// `member_joined.user` without `created_at`, and the frame tests pin the
/// exact shape, hence a separate type instead of reusing [`User`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserBrief {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub avatar_version: i64,
}

/// `Member` object: a user in the context of their family, with role.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Member {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    /// `"owner"` or `"member"` — and ABSENT on a former member, who holds no
    /// role at all (protocol.md, "Deleting an account"). A live member always
    /// carries one; the `Option` exists for the tombstone in
    /// `former_members`, which is not a member of anything.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub role: Option<String>,
    pub avatar_version: i64,
    /// Present when (and only when) this member has set one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub birthday: Option<Birthday>,
    /// Present and true ONLY on a former member — an account that was
    /// deleted while in this family. Absent otherwise, so a live member never
    /// carries `"deleted": false`.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub deleted: bool,
}

/// `Family` object. `invite_code` is present when (and only when) the caller
/// is the owner — hence `Option` + skip, never `null`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Family {
    pub id: i64,
    pub name: String,
    /// `"open"` or `"approval"`.
    pub join_policy: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_code: Option<String>,
    /// The family's main language, one of the nine tags in
    /// [`crate::handlers_family::FAMILY_LANGUAGES`], or absent when the
    /// owner has never set one. Absent is NOT `"en"` — see protocol.md,
    /// "The family's language".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Whether an `@ai` mention in the family chat is shown the recent
    /// history of that chat (protocol.md, "Mentioning the assistant in the
    /// family chat"). Owner-set, `true` by default.
    ///
    /// ALWAYS serialized, unlike the two fields above, and NOT an `Option`:
    /// a boolean with a real default has no "unset" state for an absent key
    /// to mean, so hiding it would only make a client invent one. The two
    /// above are options because "never chosen" is a state that means
    /// something there; here it is not.
    pub ai_history: bool,
}

/// `JoinRequest` object as listed for the owner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JoinRequest {
    pub id: i64,
    pub user: User,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// `Chat` object. `title` is denormalized server-side (family name or peer
/// display name) so clients never join locally; `peer_user_id` is `null`
/// for the family chat, per the protocol example.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Chat {
    pub id: i64,
    /// `"family"` or `"direct"`.
    pub kind: String,
    pub title: String,
    pub peer_user_id: Option<i64>,
}

/// One user's reaction on a message (protocol.md `Reaction`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reaction {
    pub user_id: i64,
    pub emoji: String,
}

/// One option of a poll, with the full list of who chose it.
///
/// `votes` is ALWAYS present and is never a delta — an empty list means
/// nobody has chosen this option, which is a different statement from "no
/// data" and the only one this type can make. Attributed votes are a product
/// decision (a family wants to see who has not voted yet) and a transport
/// one: one `poll` frame is serialised once and sent to every connection, so
/// a field whose value depends on who is reading it — "did I vote" — cannot
/// exist. Clients derive that from the list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PollOption {
    pub id: i64,
    pub text: String,
    pub votes: Vec<i64>,
}

/// `Poll` object: what makes a message votable (protocol.md, "Polls").
///
/// The QUESTION is deliberately not here — it is the message body, which is
/// what lets a chat-list preview, a push alert, a reply excerpt and the
/// assistant's transcript all read a poll without one new case between them,
/// and what lets a client that has never heard of polls draw it as an
/// ordinary message and lose only the buttons.
///
/// `poll_seq` and `closed` are ALWAYS serialised, unlike `Message`'s
/// `reaction_seq`: a poll has a sequence from the moment it exists, and
/// `closed` is a boolean with a real default, so there is no "unset" state
/// for an absent key to mean.
///
/// **There is no `multiple_choice` field, and adding one is not a small
/// change.** A poll takes exactly one answer, and that is enforced by the
/// primary key of `poll_votes` (migration 0022), not by a validation rule
/// this struct could relax.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Poll {
    pub poll_seq: i64,
    pub closed: bool,
    pub options: Vec<PollOption>,
}

impl Poll {
    /// Fewest options a poll may be created with. One option is not a
    /// question, it is a statement.
    pub const MIN_OPTIONS: usize = 2;
}

/// `Message` object — identical over REST and WS.
///
/// `reactions`/`reaction_seq` are present iff the message has ever been
/// reacted to (`reaction_seq > 0` in the database); after the last reaction
/// is removed they stay present with an empty list, so clients can tell
/// "cleared" from "not provided". Absent-not-null keeps the pre-reaction
/// frame shapes byte-identical for old clients and old tests alike.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: i64,
    pub chat_id: i64,
    pub sender_id: i64,
    pub client_msg_id: Uuid,
    pub body: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reactions: Option<Vec<Reaction>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reaction_seq: Option<i64>,
    /// Present when (and only when) this message is a reply. Recomputed on
    /// every read from the quoted row, never stored — see `ReplyTo`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reply_to: Option<ReplyTo>,
    /// Both present when (and only when) the body has been edited. Absent —
    /// not null, and not a zero seq — on a message still in its original
    /// form, which is how a client tells "never edited" from "edited".
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "crate::models::opt_rfc3339"
    )]
    pub edited_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub edit_seq: Option<i64>,
    /// Present when the message carries a photo or video.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub attachment: Option<Attachment>,
    /// Present when (and only when) the message is a poll. Hydrated AFTER
    /// the fact by an `attach_*` helper, exactly as `reactions` is — a poll
    /// is three tables and cannot be joined onto a message row without
    /// multiplying it, so `from_row` leaves this `None` and never reads a
    /// column for it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub poll: Option<Poll>,
}

/// RFC3339 for an OPTIONAL timestamp. `time::serde::rfc3339::option` exists
/// but pairs badly with skip_serializing_if, so this thin wrapper keeps the
/// absent case absent rather than null.
pub mod opt_rfc3339 {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    pub fn serialize<S: Serializer>(
        value: &Option<OffsetDateTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(stamp) => stamp
                .format(&Rfc3339)
                .map_err(serde::ser::Error::custom)?
                .serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<OffsetDateTime>, D::Error> {
        let raw = Option::<String>::deserialize(deserializer)?;
        raw.map(|s| OffsetDateTime::parse(&s, &Rfc3339))
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}

/// The quoted message, as much of it as a client needs to draw the quote
/// without having the original in its cache (protocol.md, "Replies").
///
/// Deliberately a snapshot computed AT READ TIME rather than columns stored
/// beside the reply: a quoted message can be edited later, and a snippet
/// frozen at send time would go on showing text its author has since
/// changed.
///
/// It carries ONE more level, through [`QuotedParent`] — answering an answer
/// shows both halves of the exchange. The cap is structural rather than a
/// rule someone has to remember: `QuotedParent` is a different type with no
/// `parent` of its own, so there is nothing to recurse into and a message
/// four deep still renders exactly two.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplyTo {
    pub message_id: i64,
    pub sender_id: i64,
    pub excerpt: String,
    /// What the quoted message was itself answering, when it was answering
    /// anything. Absent is normal: the parent was not a reply, or its own
    /// parent has been swept by retention (the FK is ON DELETE SET NULL).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<QuotedParent>,
}

/// The second and LAST level of a quote (protocol.md, "Replies").
///
/// Same three fields as [`ReplyTo`] and cut the same way, but deliberately
/// NOT the same type: giving it a `parent` would make unbounded nesting a
/// one-line change, and one old thread would drag its whole ancestry into
/// every page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuotedParent {
    pub message_id: i64,
    pub sender_id: i64,
    pub excerpt: String,
}

impl QuotedParent {
    pub fn new(message_id: i64, sender_id: i64, body: &str) -> Self {
        Self {
            message_id,
            sender_id,
            excerpt: ReplyTo::excerpt(body),
        }
    }
}

impl ReplyTo {
    /// Longest excerpt sent, in Unicode scalar values. Enough to recognise
    /// the quoted message; a client that wants the rest has the id.
    pub const MAX_EXCERPT_CHARS: usize = 120;

    pub fn new(message_id: i64, sender_id: i64, body: &str) -> Self {
        Self {
            message_id,
            sender_id,
            excerpt: Self::excerpt(body),
            parent: None,
        }
    }

    /// The same snippet, carrying what the quoted message was answering.
    pub fn with_parent(mut self, parent: Option<QuotedParent>) -> Self {
        self.parent = parent;
        self
    }

    /// Cut to [`MAX_EXCERPT_CHARS`] scalar values — `chars()`, never bytes,
    /// so the cut can never land inside a character and produce invalid
    /// UTF-8 (or, on the clients, tofu).
    pub fn excerpt(body: &str) -> String {
        body.chars().take(Self::MAX_EXCERPT_CHARS).collect()
    }
}

impl Message {
    /// Map a row exposing `id, chat_id, sender_id, client_msg_id, body,
    /// created_at`. Reaction fields start `None`; the paths that expose
    /// them (`get_messages`) enrich afterwards.
    ///
    /// The reply snippet is read with `try_get` from the joined columns
    /// `reply_to_message_id` / `reply_sender_id` / `reply_body`, so the
    /// SAME mapper serves both the narrow SELECTs (chat-list previews,
    /// which carry no quote) and the joined one in `get_messages`. All
    /// three must be present and non-NULL for a snippet to appear.
    pub fn from_row(row: &PgRow) -> Self {
        let reply_to = match (
            row.try_get::<Option<i64>, _>("reply_to_message_id"),
            row.try_get::<Option<i64>, _>("reply_sender_id"),
            row.try_get::<Option<String>, _>("reply_body"),
        ) {
            (Ok(Some(message_id)), Ok(Some(sender_id)), Ok(Some(body))) => {
                // The second level, under the same all-three-or-nothing
                // rule. A narrow SELECT that does not join it simply has no
                // such columns, and `try_get` reports that as absent.
                let parent = match (
                    row.try_get::<Option<i64>, _>("reply_parent_message_id"),
                    row.try_get::<Option<i64>, _>("reply_parent_sender_id"),
                    row.try_get::<Option<String>, _>("reply_parent_body"),
                ) {
                    (Ok(Some(pid)), Ok(Some(psender)), Ok(Some(pbody))) => {
                        Some(QuotedParent::new(pid, psender, &pbody))
                    }
                    _ => None,
                };
                Some(ReplyTo::new(message_id, sender_id, &body).with_parent(parent))
            }
            _ => None,
        };
        // Both sequences: zero in the column means "never happened" and is
        // ABSENT on the wire. Read here rather than by each caller — a read
        // path that forgot the reaction_seq loop silently dropped every
        // reaction from its response, which is exactly how this moved.
        let reaction_seq = match row.try_get::<i64, _>("reaction_seq") {
            Ok(seq) if seq > 0 => Some(seq),
            _ => None,
        };
        let edit_seq = match row.try_get::<i64, _>("edit_seq") {
            Ok(seq) if seq > 0 => Some(seq),
            _ => None,
        };
        let edited_at = row
            .try_get::<Option<OffsetDateTime>, _>("edited_at")
            .unwrap_or_default();
        // Joined attachment columns, absent on the narrow SELECTs.
        let attachment = match row.try_get::<Option<i64>, _>("att_id") {
            Ok(Some(id)) => Some(Attachment {
                id,
                kind: row.try_get("att_kind").unwrap_or_default(),
                mime: row.try_get("att_mime").unwrap_or_default(),
                size: row.try_get("att_size").unwrap_or_default(),
                width: row.try_get("att_width").unwrap_or_default(),
                height: row.try_get("att_height").unwrap_or_default(),
                duration_ms: row.try_get("att_duration_ms").unwrap_or_default(),
                has_preview: row.try_get("att_has_preview").unwrap_or_default(),
                name: row.try_get("att_name").unwrap_or_default(),
                latitude: row.try_get("att_latitude").unwrap_or_default(),
                longitude: row.try_get("att_longitude").unwrap_or_default(),
                accuracy_m: row.try_get("att_accuracy_m").unwrap_or_default(),
            }),
            _ => None,
        };
        Self {
            id: row.get("id"),
            chat_id: row.get("chat_id"),
            sender_id: row.get("sender_id"),
            client_msg_id: row.get("client_msg_id"),
            body: row.get("body"),
            created_at: row.get("created_at"),
            reactions: None,
            reaction_seq,
            reply_to,
            edited_at,
            edit_seq,
            attachment,
            // Not read from a column, by design: see the field's doc. The
            // paths that expose a poll enrich afterwards, as they do for
            // reactions.
            poll: None,
        }
    }
}

/// Entry of the `GET /chats` list: chat + preview + authoritative unread.
/// `max_reaction_seq` is omitted while nothing in the chat was ever reacted
/// to; `last_message` previews never carry reactions.
#[derive(Debug, Clone, Serialize)]
pub struct ChatListEntry {
    pub chat: Chat,
    pub last_message: Option<Message>,
    pub unread_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_reaction_seq: Option<i64>,
    /// Omitted while nothing in the chat has ever been edited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_edit_seq: Option<i64>,
    /// Omitted while the chat holds no poll — the fourth cursor, read
    /// exactly as the two above (protocol.md, "Polls").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_poll_seq: Option<i64>,
}

/// Something a message carries: a photo, a video, a piece of audio, a file,
/// or a location.
///
/// The bytes are NOT here and never travel through this struct — they live
/// on the filesystem and are fetched from `GET /attachments/{id}`. This is
/// what a bubble needs to lay itself out before a single byte arrives:
/// what it is, how big, what shape, and whether a preview exists yet.
///
/// A LOCATION is the exception that proves the rule: it has no bytes at all,
/// so everything it is lives in these fields and the bubble draws it without
/// fetching anything.
///
/// `Eq` is deliberately absent — the coordinates are floats. Equality on an
/// attachment is `PartialEq`, which is what every comparison here wants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    pub id: i64,
    /// "photo" | "video" | "audio" | "file" | "location".
    pub kind: String,
    pub mime: String,
    pub size: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
    /// Videos only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i32>,
    /// False until the client has uploaded the downscaled photo or poster
    /// frame — a message may be sent before its preview arrives. Always
    /// false for a file, which has nothing to draw.
    pub has_preview: bool,
    /// Files only, and required there: a document's name is its whole
    /// identity, where a photo renders itself. Optional on audio and on a
    /// location, where it is a label ("Home", "The restaurant").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Locations only, and always present on one: the pin.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    /// Locations only, and only when the sending device reported one: the
    /// radius in metres it believed the fix to be good to. Absent means
    /// unknown, which a client draws as a plain pin rather than a circle —
    /// not as "perfectly accurate".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accuracy_m: Option<i32>,
}

impl Attachment {
    /// The media types accepted for a photo, video or piece of audio,
    /// paired with the kind they belong to. HEIC/HEIF are here because that
    /// is what an iPhone actually produces.
    ///
    /// A file is not on this list and never will be: `kind=file` accepts any
    /// type and verifies none (protocol.md, "Files").
    pub const ACCEPTED: [(&'static str, &'static str); 11] = [
        ("image/jpeg", "photo"),
        ("image/png", "photo"),
        ("image/heic", "photo"),
        ("image/heif", "photo"),
        ("video/mp4", "video"),
        ("video/quicktime", "video"),
        // Audio. m4a/aac is what both phones record into, and mp3/wav/ogg
        // are what a track picked off a disk usually is.
        ("audio/mp4", "audio"),
        ("audio/m4a", "audio"),
        ("audio/mpeg", "audio"),
        ("audio/wav", "audio"),
        ("audio/ogg", "audio"),
    ];

    pub const KIND_FILE: &'static str = "file";
    pub const KIND_AUDIO: &'static str = "audio";
    pub const KIND_LOCATION: &'static str = "location";
    /// A location's media type. Not `application/json` and not a real
    /// geo type: nothing is served under it, and naming it after this
    /// application says plainly that the bytes do not exist rather than
    /// inviting a client to fetch them.
    pub const LOCATION_MIME: &'static str = "application/vnd.family-connect.location";
    /// What a file's bytes are called when the client declared nothing
    /// usable. Deliberately the least interesting type there is.
    pub const DEFAULT_FILE_MIME: &'static str = "application/octet-stream";
    pub const MAX_NAME_LEN: usize = 255;

    pub fn kind_for(mime: &str) -> Option<&'static str> {
        Self::ACCEPTED
            .iter()
            .find(|(accepted, _)| *accepted == mime)
            .map(|(_, kind)| *kind)
    }

    /// A filename safe to put in a `Content-Disposition` header.
    ///
    /// A header is a LINE: an unescaped CR or LF in a filename ends it and
    /// lets the uploader write headers of their own, and a quote closes the
    /// quoted-string early. Path separators go too — the name is a label,
    /// and a client that joins it onto a directory should never be handed
    /// `../`. What survives is printable ASCII; the caller pairs this with
    /// an RFC 5987 `filename*` that carries the real name.
    pub fn ascii_filename(name: &str) -> String {
        let cleaned: String = name
            .chars()
            .map(|c| match c {
                '/' | '\\' => '_',
                '"' => '\'',
                c if c.is_ascii_graphic() || c == ' ' => c,
                // Control characters and everything non-ASCII.
                _ => '_',
            })
            .take(Self::MAX_NAME_LEN)
            .collect();
        let trimmed = cleaned.trim().trim_matches('.').trim().to_string();
        if trimmed.is_empty() {
            "download".to_string()
        } else {
            trimmed
        }
    }

    pub fn from_row(row: &PgRow) -> Self {
        Self {
            id: row.get("id"),
            kind: row.get("kind"),
            mime: row.get("mime"),
            size: row.get("size_bytes"),
            width: row.get("width"),
            height: row.get("height"),
            duration_ms: row.get("duration_ms"),
            has_preview: row.get("has_preview"),
            name: row.get("name"),
            // `try_get` rather than `get`: these three arrived last and a
            // narrow SELECT that forgets them must degrade to "not a
            // location" rather than panic inside a handler, which surfaces
            // as a dropped connection with no error body.
            latitude: row.try_get("latitude").unwrap_or_default(),
            longitude: row.try_get("longitude").unwrap_or_default(),
            accuracy_m: row.try_get("accuracy_m").unwrap_or_default(),
        }
    }
}

/// One sticker note on the family board.
///
/// A tombstone is the same object with `deleted: true` and no content: the
/// change feed has to be able to say "this note is gone", and an absent row
/// cannot say anything (protocol.md, "Board").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "crate::models::opt_rfc3339"
    )]
    pub created_at: Option<OffsetDateTime>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "crate::models::opt_rfc3339"
    )]
    pub updated_at: Option<OffsetDateTime>,
    pub board_seq: i64,
    /// Present and true ONLY on a tombstone; absent otherwise, so a live
    /// note never carries `"deleted": false`.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub deleted: bool,
}

impl Note {
    /// Colours a note may take. A fixed set so the two clients can render
    /// them identically without negotiating; anything else is rejected
    /// rather than stored and silently drawn as a default.
    pub const COLORS: [&'static str; 6] = ["yellow", "pink", "blue", "green", "orange", "purple"];

    /// Longest note text. A sticker, not a message.
    pub const MAX_TEXT_CHARS: usize = 280;

    /// Fractions of the board, clamped rather than rejected: a drag that
    /// ends past the edge should stick to the edge, not fail.
    pub fn clamp_position(value: f64) -> f64 {
        if value.is_nan() {
            return 0.0;
        }
        value.clamp(0.0, 1.0)
    }

    pub fn from_row(row: &PgRow) -> Self {
        let deleted_at: Option<OffsetDateTime> = row.get("deleted_at");
        if deleted_at.is_some() {
            return Self {
                id: row.get("id"),
                author_id: None,
                text: None,
                color: None,
                x: None,
                y: None,
                created_at: None,
                updated_at: None,
                board_seq: row.get("board_seq"),
                deleted: true,
            };
        }
        Self {
            id: row.get("id"),
            author_id: Some(row.get("author_id")),
            text: Some(row.get("text")),
            color: Some(row.get("color")),
            x: Some(row.get("x")),
            y: Some(row.get("y")),
            created_at: Some(row.get("created_at")),
            updated_at: Some(row.get("updated_at")),
            board_seq: row.get("board_seq"),
            deleted: false,
        }
    }
}

/// `GET /me` — the caller's own pending join request, if any.
#[derive(Debug, Clone, Serialize)]
pub struct PendingJoinRequest {
    pub family_id: i64,
    pub family_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn family_omits_invite_code_entirely_for_non_owners() {
        let family = Family {
            id: 3,
            name: "The Smiths".to_string(),
            join_policy: "approval".to_string(),
            created_at: datetime!(2026-08-19 17:03:12 UTC),
            invite_code: None,
            language: None,
            ai_history: true,
        };
        let json = serde_json::to_value(&family).expect("serialize");
        assert!(
            json.get("invite_code").is_none(),
            "invite_code must be absent, not null: {json}"
        );
    }

    /// A family that has never chosen a language carries no `language` key
    /// at all. Absent is NOT `"en"`: a client that saw a null, or a default,
    /// could not tell "we never chose" from "we chose English".
    #[test]
    fn a_family_with_no_language_matches_the_protocol_shape() {
        let family = Family {
            id: 3,
            name: "The Smiths".to_string(),
            join_policy: "open".to_string(),
            created_at: datetime!(2026-08-19 17:03:12 UTC),
            invite_code: None,
            language: None,
            ai_history: true,
        };
        assert_eq!(
            serde_json::to_value(&family).expect("serialize"),
            serde_json::json!({
                "id": 3, "name": "The Smiths", "join_policy": "open",
                "created_at": "2026-08-19T17:03:12Z", "ai_history": true
            })
        );
    }

    #[test]
    fn a_family_with_a_language_and_a_code_matches_the_protocol_shape() {
        let family = Family {
            id: 3,
            name: "The Smiths".to_string(),
            join_policy: "open".to_string(),
            created_at: datetime!(2026-08-19 17:03:12 UTC),
            invite_code: Some("ABCD2345".to_string()),
            language: Some("ru".to_string()),
            ai_history: true,
        };
        assert_eq!(
            serde_json::to_value(&family).expect("serialize"),
            serde_json::json!({
                "id": 3, "name": "The Smiths", "join_policy": "open",
                "created_at": "2026-08-19T17:03:12Z",
                "invite_code": "ABCD2345", "language": "ru", "ai_history": true
            })
        );
    }

    /// A birthday is a day and a month, one object, present or absent —
    /// never a null and never half of one.
    #[test]
    fn a_member_with_a_birthday_matches_the_protocol_shape() {
        let member = Member {
            id: 7,
            username: "anna".to_string(),
            display_name: "Anna".to_string(),
            role: Some("owner".to_string()),
            avatar_version: 3,
            birthday: Some(Birthday { month: 3, day: 14 }),
            deleted: false,
        };
        assert_eq!(
            serde_json::to_value(&member).expect("serialize"),
            serde_json::json!({
                "id": 7, "username": "anna", "display_name": "Anna", "role": "owner",
                "avatar_version": 3, "birthday": {"month": 3, "day": 14}
            })
        );
    }

    #[test]
    fn a_member_without_one_carries_no_birthday_key() {
        let member = Member {
            id: 7,
            username: "anna".to_string(),
            display_name: "Anna".to_string(),
            role: Some("member".to_string()),
            avatar_version: 0,
            birthday: None,
            deleted: false,
        };
        assert_eq!(
            serde_json::to_value(&member).expect("serialize"),
            serde_json::json!({
                "id": 7, "username": "anna", "display_name": "Anna", "role": "member",
                "avatar_version": 0
            })
        );
    }

    #[test]
    fn a_user_with_a_birthday_matches_the_protocol_shape() {
        let user = User {
            id: 7,
            username: "anna".to_string(),
            display_name: "Anna".to_string(),
            created_at: datetime!(2026-08-19 17:03:12 UTC),
            avatar_version: 3,
            birthday: Some(Birthday { month: 2, day: 29 }),
            deleted: false,
        };
        assert_eq!(
            serde_json::to_value(&user).expect("serialize"),
            serde_json::json!({
                "id": 7, "username": "anna", "display_name": "Anna",
                "created_at": "2026-08-19T17:03:12Z", "avatar_version": 3,
                "birthday": {"month": 2, "day": 29}
            })
        );
    }

    /// A former member holds no role and says so by carrying no `role` key
    /// at all — they are in `former_members` precisely because they are in
    /// nothing else (protocol.md, "Deleting an account").
    #[test]
    fn a_former_member_carries_no_role_and_says_deleted() {
        let tombstone = Member {
            id: 7,
            username: String::new(),
            display_name: "Deleted account".to_string(),
            role: None,
            avatar_version: 0,
            birthday: None,
            deleted: true,
        };
        assert_eq!(
            serde_json::to_value(&tombstone).expect("serialize"),
            serde_json::json!({
                "id": 7, "username": "", "display_name": "Deleted account",
                "avatar_version": 0, "deleted": true
            })
        );
    }

    /// `poll_seq` and `closed` are ALWAYS present, unlike `reaction_seq`: a
    /// poll has a sequence from the moment it exists, and `closed` is a
    /// boolean with a real default, so there is no "unset" for a missing key
    /// to mean. `votes` is always present too — empty means nobody chose it,
    /// which is a different statement from "no data".
    #[test]
    fn a_poll_matches_the_protocol_shape() {
        let poll = Poll {
            poll_seq: 88,
            closed: false,
            options: vec![
                PollOption {
                    id: 5,
                    text: "Pizza".to_string(),
                    votes: vec![7, 9],
                },
                PollOption {
                    id: 6,
                    text: "Pasta".to_string(),
                    votes: vec![],
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(&poll).expect("serialize"),
            serde_json::json!({
                "poll_seq": 88, "closed": false,
                "options": [
                    {"id": 5, "text": "Pizza", "votes": [7, 9]},
                    {"id": 6, "text": "Pasta", "votes": []}
                ]
            })
        );
    }

    #[test]
    fn chat_serializes_a_null_peer_for_the_family_chat() {
        let chat = Chat {
            id: 42,
            kind: "family".to_string(),
            title: "The Smiths".to_string(),
            peer_user_id: None,
        };
        let json = serde_json::to_value(&chat).expect("serialize");
        assert_eq!(json["peer_user_id"], serde_json::Value::Null);
    }

    #[test]
    fn timestamps_serialize_as_rfc3339_utc_with_a_z_suffix() {
        let user = User {
            id: 7,
            username: "anna".to_string(),
            display_name: "Anna".to_string(),
            created_at: datetime!(2026-08-19 17:03:12 UTC),
            avatar_version: 0,
            birthday: None,
            deleted: false,
        };
        let json = serde_json::to_value(&user).expect("serialize");
        assert_eq!(json["created_at"], "2026-08-19T17:03:12Z");
    }

    #[test]
    fn a_short_body_is_quoted_whole() {
        let reply = ReplyTo::new(41, 9, "See you at six");
        assert_eq!(reply.excerpt, "See you at six");
        assert_eq!(reply.message_id, 41);
        assert_eq!(reply.sender_id, 9);
    }

    #[test]
    fn a_long_body_is_cut_to_the_documented_length() {
        let body = "x".repeat(500);
        let excerpt = ReplyTo::excerpt(&body);
        assert_eq!(excerpt.chars().count(), ReplyTo::MAX_EXCERPT_CHARS);
    }

    /// The cut counts SCALAR VALUES, not bytes: cutting a multi-byte
    /// character in half would produce invalid UTF-8 — which serde would
    /// refuse to serialize, taking the whole message page down with it.
    #[test]
    fn the_cut_never_lands_inside_a_character() {
        // 'é' is 2 bytes, '中' 3, '😀' 4 — a byte-based cut at 120 would
        // split one of them.
        let body: String = "é中😀".repeat(100);
        let excerpt = ReplyTo::excerpt(&body);
        assert_eq!(excerpt.chars().count(), ReplyTo::MAX_EXCERPT_CHARS);
        // Round-trips as valid UTF-8 through JSON.
        let json = serde_json::to_string(&ReplyTo::new(1, 2, &body)).expect("serializes");
        assert!(json.contains("\"message_id\":1"));
    }

    #[test]
    fn an_empty_body_quotes_as_empty() {
        assert_eq!(ReplyTo::excerpt(""), "");
    }

    /// A filename ends up in a `Content-Disposition` header, and a header
    /// is a LINE. Everything here is about the uploader not being able to
    /// write the rest of that line themselves.
    #[test]
    fn a_filename_cannot_escape_its_header() {
        let hostile = "a\r\nSet-Cookie: x=1\r\n\r\nb.txt";
        let safe = Attachment::ascii_filename(hostile);
        assert!(!safe.contains('\r') && !safe.contains('\n'), "{safe}");
        // A quote would close the quoted-string early.
        assert!(!Attachment::ascii_filename(r#"in"voice.pdf"#).contains('"'));
        // Path separators are dropped: the name is a label, not a path.
        let traversed = Attachment::ascii_filename("../../etc/passwd");
        assert!(!traversed.contains('/'), "{traversed}");
        assert!(!Attachment::ascii_filename(r"..\..\windows\hosts").contains('\\'));
    }

    #[test]
    fn a_filename_always_survives_as_something() {
        // Non-ASCII is replaced in the ASCII form (the caller pairs this
        // with filename*, which carries the real name).
        assert_eq!(
            Attachment::ascii_filename("Rechnung März.pdf"),
            "Rechnung M_rz.pdf"
        );
        // A name made entirely of characters we drop still has to produce
        // a usable filename rather than an empty one.
        assert_eq!(Attachment::ascii_filename("   "), "download");
        assert_eq!(Attachment::ascii_filename("..."), "download");
        assert_eq!(Attachment::ascii_filename(""), "download");
        // And a long one is cut rather than refused.
        let long = "x".repeat(400);
        assert_eq!(
            Attachment::ascii_filename(&long).len(),
            Attachment::MAX_NAME_LEN
        );
    }
}
