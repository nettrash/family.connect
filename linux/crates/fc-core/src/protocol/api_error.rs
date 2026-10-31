//! The protocol's error shape (docs/protocol.md, "Error shape"):
//! `{"error": {"code": "username_taken", "message": "…"}}` — a port of
//! `win/src/FamilyConnect.Core/Protocol/ApiError.cs`.

/// A failure: a refusal the server read and answered, or a request that never got an answer.
///
/// The [`code`](Self::code) is what a client branches on; the message is an English sentence for
/// a log, never for a user — every client shows its own translated wording.
///
/// A failure with no shape at all is still an error: nginx answers its own rate limit with a
/// `429` and an HTML body, so status alone has to be enough to classify one. That is why
/// [`transient`](Self::transient) takes the status as well as the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    /// The HTTP status, or 0 when there was none — a transport failure, or a socket `error`
    /// frame, which carries a code and nothing else.
    pub status: u16,
    /// The server's `Retry-After`, when it gave one. Signed: a `Retry-After` DATE already in the
    /// past is a negative wait, and the reader clamps it rather than this type hiding it.
    pub retry_after: Option<time::Duration>,
}

impl ApiError {
    /// `new ApiError(code, message, status)`.
    pub fn new(code: &str, message: &str, status: u16) -> Self {
        ApiError {
            code: code.to_string(),
            message: message.to_string(),
            status,
            retry_after: None,
        }
    }

    /// A socket refusal: a code, and no status at all.
    pub fn coded(code: &str, message: &str) -> Self {
        ApiError::new(code, message, 0)
    }

    /// The same failure, with the server's own wait.
    pub fn with_retry_after(mut self, wait: time::Duration) -> Self {
        self.retry_after = Some(wait);
        self
    }

    /// A transport failure: nothing was read, so nothing was refused.
    pub fn transport(message: &str) -> Self {
        ApiError::new(error_codes::TRANSPORT, message, 0)
    }

    /// Whether this failure says nothing about the request except that it did not arrive or was
    /// not answered — the distinction the whole send pipeline hangs on (docs/protocol.md: "a
    /// transient failure is not a refusal, and a client must never present one as one").
    ///
    /// Transient: every transport failure, 408, 429, every 5xx, and the `internal` code.
    /// Terminal: every other 4xx carrying a code.
    ///
    /// THE CODE IS CONSULTED BEFORE THE STATUS, because a refusal can arrive with no status at
    /// all: a socket `error` frame carries a code and nothing else, and reading a missing status
    /// as "it never got there" would have this client retry a message the server has already
    /// refused — six times, and then show a failure six delays late. A status of 0 with no code it
    /// knows is the transport failure it looks like.
    pub fn transient(&self) -> bool {
        self.code == error_codes::TRANSPORT
            || self.code == error_codes::INTERNAL
            || self.code == error_codes::TOO_MANY_REQUESTS
            || self.status == 408
            || self.status == 429
            || self.status >= 500
            || (self.status == 0 && !self.canonical())
    }

    /// Whether this is a code the protocol names (docs/protocol.md, "Error shape").
    pub fn canonical(&self) -> bool {
        error_codes::ALL.contains(&self.code.as_str())
    }

    /// The session is gone and the client returns to login — but see the exception.
    ///
    /// `invalid_credentials` shares the 401 and not the meaning: it is a password that was wrong,
    /// at login or as the proof a password change asks for, and the session that sent it is still
    /// live. A client that signs somebody out for a mistyped current password has read the status
    /// and not the answer (docs/protocol.md, "Authentication").
    pub fn session_gone(&self) -> bool {
        self.status == 401 && self.code != error_codes::INVALID_CREDENTIALS
    }
}

/// The canonical codes, spelled as the protocol spells them (docs/protocol.md, "Error shape").
///
/// The list is kept whole, retired codes included: `owner_cannot_leave` is raised by no endpoint
/// any more, and it stays because a client that predates the hand-off still branches on it — a
/// code that vanishes from the document is a code somebody deletes from a client that is still
/// talking to an old server.
pub mod error_codes {
    /// Not on the wire: this client's own name for "it never got there".
    pub const TRANSPORT: &str = "__transport";

    /// Not on the wire either: this client's own name for "the bytes I was going to send are
    /// gone". A staged copy the system reclaimed, or a reinstall — and there is nothing to retry,
    /// because the picture cannot be recovered from an id alone.
    pub const MEDIA_MISSING: &str = "__media_missing";

    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const INVALID_CREDENTIALS: &str = "invalid_credentials";
    pub const USERNAME_TAKEN: &str = "username_taken";
    pub const VALIDATION: &str = "validation";
    pub const ALREADY_IN_FAMILY: &str = "already_in_family";
    pub const FAMILY_REGISTRATION_DISABLED: &str = "family_registration_disabled";
    pub const FAMILY_FULL: &str = "family_full";
    pub const NOT_IN_FAMILY: &str = "not_in_family";
    pub const NOT_FAMILY_OWNER: &str = "not_family_owner";
    pub const INVALID_INVITE_CODE: &str = "invalid_invite_code";
    pub const JOIN_REQUEST_PENDING: &str = "join_request_pending";
    pub const JOIN_REQUEST_NOT_PENDING: &str = "join_request_not_pending";
    pub const USER_ALREADY_IN_FAMILY: &str = "user_already_in_family";
    pub const OWNER_CANNOT_LEAVE: &str = "owner_cannot_leave";
    pub const CANNOT_REMOVE_OWNER: &str = "cannot_remove_owner";
    pub const CANNOT_DM_SELF: &str = "cannot_dm_self";
    pub const CANNOT_BLOCK_SELF: &str = "cannot_block_self";
    pub const BLOCKED: &str = "blocked";
    pub const CANNOT_REPORT_SELF: &str = "cannot_report_self";
    pub const REPORT_NOT_PENDING: &str = "report_not_pending";
    pub const NOT_SAME_FAMILY: &str = "not_same_family";
    pub const USER_NOT_FOUND: &str = "user_not_found";
    pub const CHAT_NOT_FOUND: &str = "chat_not_found";
    pub const NOT_CHAT_MEMBER: &str = "not_chat_member";
    pub const MESSAGE_EMPTY: &str = "message_empty";
    pub const MESSAGE_TOO_LONG: &str = "message_too_long";
    pub const MESSAGE_NOT_FOUND: &str = "message_not_found";
    pub const NOT_MESSAGE_AUTHOR: &str = "not_message_author";
    pub const INVALID_EMOJI: &str = "invalid_emoji";
    pub const NOTE_NOT_FOUND: &str = "note_not_found";
    pub const NOT_NOTE_AUTHOR: &str = "not_note_author";
    pub const INVALID_NOTE_COLOR: &str = "invalid_note_color";
    pub const INVALID_NOTE_SIZE: &str = "invalid_note_size";
    pub const INVALID_NOTE_FONT: &str = "invalid_note_font";
    pub const INVALID_NOTE_KIND: &str = "invalid_note_kind";
    pub const INVALID_RSVP: &str = "invalid_rsvp";
    pub const INVALID_TASK: &str = "invalid_task";
    pub const INVALID_LANGUAGE: &str = "invalid_language";
    pub const BOARD_FULL: &str = "board_full";
    pub const INVALID_PAGINATION: &str = "invalid_pagination";
    pub const DEVICE_NOT_FOUND: &str = "device_not_found";
    pub const INVALID_POLL: &str = "invalid_poll";
    pub const POLL_CLOSED: &str = "poll_closed";
    pub const PICTURES_UNAVAILABLE: &str = "pictures_unavailable";

    /// The provider's own filter refused to draw a backdrop's title (docs/protocol.md, "Board"):
    /// a 400, so terminal — asking again with the same title gets the same refusal.
    pub const PICTURE_REFUSED: &str = "picture_refused";

    /// The member has not agreed that their words may go to the model, so nothing was sent
    /// (docs/protocol.md, "Consenting to the assistant"): a 403, and the answer to it is the
    /// consent question, then asking again.
    ///
    /// In [`ALL`] since 2026-10-02: the document's list under "Error shape" was missing it until
    /// the transcript endpoint answered it too, and that list is what [`ALL`] is held to.
    pub const ASSISTANT_CONSENT_REQUIRED: &str = "assistant_consent_required";

    /// This server has no transcription deployment (docs/protocol.md, "Transcripts on request"): a
    /// 403 about the SERVER, which `assistant.transcribe` already says before anybody asks.
    pub const TRANSCRIPTS_UNAVAILABLE: &str = "transcripts_unavailable";

    /// The rule says no for this member and this recording — another member's, with the owner's
    /// `ai_transcripts` off or that sender not having agreed to the assistant, or another
    /// member's in a direct chat. Terminal until the switch or that consent changes.
    pub const TRANSCRIPT_NOT_ALLOWED: &str = "transcript_not_allowed";

    /// The recording cannot be sent in the form asked for: wrong kind, a type the provider does
    /// not read, too big.
    pub const NOT_TRANSCRIBABLE: &str = "not_transcribable";

    /// The provider's own content filter refused the recording: a 400, terminal — asking again
    /// gets the same.
    pub const TRANSCRIPT_REFUSED: &str = "transcript_refused";

    pub const CALLS_DISABLED: &str = "calls_disabled";
    pub const VIDEO_CALLS_DISABLED: &str = "video_calls_disabled";
    pub const INVALID_CALL: &str = "invalid_call";
    pub const CALL_NOT_FOUND: &str = "call_not_found";
    pub const CALL_BUSY: &str = "call_busy";
    pub const PEER_BUSY: &str = "peer_busy";
    pub const PEER_UNREACHABLE: &str = "peer_unreachable";
    pub const AVATAR_TOO_LARGE: &str = "avatar_too_large";
    pub const INVALID_IMAGE: &str = "invalid_image";
    pub const ATTACHMENT_TOO_LARGE: &str = "attachment_too_large";
    pub const INVALID_ATTACHMENT: &str = "invalid_attachment";
    pub const ATTACHMENT_NOT_FOUND: &str = "attachment_not_found";
    pub const ATTACHMENT_EXPIRED: &str = "attachment_expired";
    pub const ATTACHMENT_ALREADY_USED: &str = "attachment_already_used";
    pub const STORAGE_FULL: &str = "storage_full";
    pub const PACK_FULL: &str = "pack_full";
    pub const PACK_ITEM_TOO_LARGE: &str = "pack_item_too_large";
    pub const PACK_ITEM_NOT_FOUND: &str = "pack_item_not_found";
    pub const NOT_PACK_ITEM_AUTHOR: &str = "not_pack_item_author";

    /// Renaming a group, adding or removing its members, or deleting it, by a member who is not
    /// its admin (docs/protocol.md, "Groups"): a 403, added 2026-10-22.
    pub const NOT_GROUP_ADMIN: &str = "not_group_admin";

    /// The family already has its 50 groups (docs/protocol.md, "Groups"): a 409, added
    /// 2026-10-22.
    pub const TOO_MANY_GROUPS: &str = "too_many_groups";

    /// A family the caller is not in — or one that never existed, the same answer, so the
    /// families on a server cannot be counted by trying ids (docs/protocol.md, "Several
    /// families"): a 404, added 2026-10-22. A client treats the family as gone and resyncs.
    pub const FAMILY_NOT_FOUND: &str = "family_not_found";

    /// The account is already in `max_families_per_account` families: a 409, added 2026-10-22.
    pub const TOO_MANY_FAMILIES: &str = "too_many_families";

    /// Starting another family while owning one, by hand-off included: a 409, added 2026-10-22.
    pub const ALREADY_OWN_A_FAMILY: &str = "already_own_a_family";

    /// The operator's `[families] member_families` is off: a 403, added 2026-10-22.
    pub const MEMBER_FAMILIES_DISABLED: &str = "member_families_disabled";

    /// A password reset of a member who is also in a family the owner does not own — only they
    /// may change it: a 403, added 2026-10-22.
    pub const MEMBER_IN_OTHER_FAMILIES: &str = "member_in_other_families";

    /// An invitation that is no longer pending: accepted, declined, withdrawn, somebody else's,
    /// or never there. A 409, added 2026-10-22.
    pub const INVITATION_NOT_PENDING: &str = "invitation_not_pending";

    /// A new family on a server that charges for one, from an account without an active App Store
    /// subscription (docs/protocol.md, "Paid families"): a 403, added 2026-10-30, checked after
    /// every other refusal `POST /families` can give. Terminal.
    pub const SUBSCRIPTION_REQUIRED: &str = "subscription_required";

    /// `PUT /me/subscription`: what Apple signed did not verify, or is for another app or product —
    /// a 400, added 2026-10-30. Only Apple's apps send one.
    pub const SUBSCRIPTION_INVALID: &str = "subscription_invalid";

    /// `PUT /me/subscription`: the subscription belongs to another account — a 409, added
    /// 2026-10-30.
    pub const SUBSCRIPTION_TAKEN: &str = "subscription_taken";

    /// `PUT /me/subscription` on a server whose subscriptions are off — a 403, added 2026-10-30.
    pub const SUBSCRIPTIONS_DISABLED: &str = "subscriptions_disabled";

    /// An upload for a PAID family that has used its storage allowance (docs/protocol.md, "Paid
    /// families"): a 403, added 2026-10-30, decided before the body is read. TERMINAL — not the
    /// server's disk (`storage_full`, 507), the family's allowance.
    pub const STORAGE_ALLOWANCE_USED: &str = "storage_allowance_used";

    /// A transcript or a board backdrop for a PAID family past this month's minutes or pictures: a
    /// 403, added 2026-10-30. The allowance starts again on the 1st of the next month, UTC.
    pub const AI_ALLOWANCE_USED: &str = "ai_allowance_used";

    pub const TOO_MANY_REQUESTS: &str = "too_many_requests";
    pub const INTERNAL: &str = "internal";

    /// Every code this document names, for the test that holds the list to it.
    pub const ALL: [&str; 85] = [
        UNAUTHORIZED,
        INVALID_CREDENTIALS,
        USERNAME_TAKEN,
        VALIDATION,
        ALREADY_IN_FAMILY,
        FAMILY_REGISTRATION_DISABLED,
        FAMILY_FULL,
        NOT_IN_FAMILY,
        NOT_FAMILY_OWNER,
        INVALID_INVITE_CODE,
        JOIN_REQUEST_PENDING,
        JOIN_REQUEST_NOT_PENDING,
        USER_ALREADY_IN_FAMILY,
        OWNER_CANNOT_LEAVE,
        CANNOT_REMOVE_OWNER,
        CANNOT_DM_SELF,
        CANNOT_BLOCK_SELF,
        BLOCKED,
        CANNOT_REPORT_SELF,
        REPORT_NOT_PENDING,
        NOT_SAME_FAMILY,
        USER_NOT_FOUND,
        CHAT_NOT_FOUND,
        NOT_CHAT_MEMBER,
        MESSAGE_EMPTY,
        MESSAGE_TOO_LONG,
        MESSAGE_NOT_FOUND,
        NOT_MESSAGE_AUTHOR,
        INVALID_EMOJI,
        NOTE_NOT_FOUND,
        NOT_NOTE_AUTHOR,
        INVALID_NOTE_COLOR,
        INVALID_NOTE_SIZE,
        INVALID_NOTE_FONT,
        INVALID_NOTE_KIND,
        INVALID_RSVP,
        INVALID_TASK,
        INVALID_LANGUAGE,
        BOARD_FULL,
        INVALID_PAGINATION,
        DEVICE_NOT_FOUND,
        INVALID_POLL,
        POLL_CLOSED,
        ASSISTANT_CONSENT_REQUIRED,
        PICTURES_UNAVAILABLE,
        PICTURE_REFUSED,
        TRANSCRIPTS_UNAVAILABLE,
        TRANSCRIPT_NOT_ALLOWED,
        NOT_TRANSCRIBABLE,
        TRANSCRIPT_REFUSED,
        CALLS_DISABLED,
        VIDEO_CALLS_DISABLED,
        INVALID_CALL,
        CALL_NOT_FOUND,
        CALL_BUSY,
        PEER_BUSY,
        PEER_UNREACHABLE,
        AVATAR_TOO_LARGE,
        INVALID_IMAGE,
        ATTACHMENT_TOO_LARGE,
        INVALID_ATTACHMENT,
        ATTACHMENT_NOT_FOUND,
        ATTACHMENT_EXPIRED,
        ATTACHMENT_ALREADY_USED,
        STORAGE_FULL,
        PACK_FULL,
        PACK_ITEM_TOO_LARGE,
        PACK_ITEM_NOT_FOUND,
        NOT_PACK_ITEM_AUTHOR,
        NOT_GROUP_ADMIN,
        TOO_MANY_GROUPS,
        FAMILY_NOT_FOUND,
        TOO_MANY_FAMILIES,
        ALREADY_OWN_A_FAMILY,
        MEMBER_FAMILIES_DISABLED,
        MEMBER_IN_OTHER_FAMILIES,
        INVITATION_NOT_PENDING,
        SUBSCRIPTION_REQUIRED,
        SUBSCRIPTION_INVALID,
        SUBSCRIPTION_TAKEN,
        SUBSCRIPTIONS_DISABLED,
        STORAGE_ALLOWANCE_USED,
        AI_ALLOWANCE_USED,
        TOO_MANY_REQUESTS,
        INTERNAL,
    ];
}
