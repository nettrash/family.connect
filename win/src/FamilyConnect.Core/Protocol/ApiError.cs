namespace FamilyConnect.Core.Protocol;

/// <summary>
/// The protocol's error shape (docs/protocol.md, "Error shape"):
/// <c>{"error": {"code": "username_taken", "message": "…"}}</c>.
/// </summary>
/// <remarks>
/// <para>
/// The <see cref="Code"/> is what a client branches on; the message is an English sentence for a
/// log, never for a user — every client shows its own translated wording.
/// </para>
/// <para>
/// A failure with no shape at all is still an error: nginx answers its own rate limit with a
/// <c>429</c> and an HTML body, so status alone has to be enough to classify one. That is why
/// <see cref="Transient"/> takes the status as well as the code.
/// </para>
/// </remarks>
public sealed record ApiError(string Code, string Message, int Status = 0, TimeSpan? RetryAfter = null)
{
    /// <summary>A transport failure: nothing was read, so nothing was refused.</summary>
    public static ApiError Transport(string message) => new(ErrorCodes.Transport, message);

    /// <summary>
    /// Whether this failure says nothing about the request except that it did not arrive or was
    /// not answered — the distinction the whole send pipeline hangs on (docs/protocol.md: "a
    /// transient failure is not a refusal, and a client must never present one as one").
    /// </summary>
    /// <remarks>
    /// <para>
    /// Transient: every transport failure, 408, 429, every 5xx, and the <c>internal</c> code.
    /// Terminal: every other 4xx carrying a code.
    /// </para>
    /// <para>
    /// THE CODE IS CONSULTED BEFORE THE STATUS, because a refusal can arrive with no status at
    /// all: a socket <c>error</c> frame carries a code and nothing else, and reading a missing
    /// status as "it never got there" would have this client retry a message the server has
    /// already refused — six times, and then show a failure six delays late. A status of 0 with
    /// no code it knows is the transport failure it looks like.
    /// </para>
    /// </remarks>
    public bool Transient =>
        Code == ErrorCodes.Transport
        || Code == ErrorCodes.Internal
        || Code == ErrorCodes.TooManyRequests
        || Status == 408
        || Status == 429
        || Status >= 500
        || (Status == 0 && !Canonical);

    /// <summary>Whether this is a code the protocol names (docs/protocol.md, "Error shape").</summary>
    public bool Canonical => ErrorCodes.All.Contains(Code);

    /// <summary>The session is gone and the client returns to login — but see the exception.</summary>
    /// <remarks>
    /// <c>invalid_credentials</c> shares the 401 and not the meaning: it is a password that was
    /// wrong, at login or as the proof a password change asks for, and the session that sent it is
    /// still live. A client that signs somebody out for a mistyped current password has read the
    /// status and not the answer (docs/protocol.md, "Authentication").
    /// </remarks>
    public bool SessionGone => Status == 401 && Code != ErrorCodes.InvalidCredentials;
}

/// <summary>
/// The canonical codes, spelled as the protocol spells them (docs/protocol.md, "Error shape").
/// </summary>
/// <remarks>
/// The list is kept whole, retired codes included: <c>owner_cannot_leave</c> is raised by no
/// endpoint any more, and it stays because a client that predates the hand-off still branches on
/// it — a code that vanishes from the document is a code somebody deletes from a client that is
/// still talking to an old server.
/// </remarks>
public static class ErrorCodes
{
    /// <summary>Not on the wire: this client's own name for "it never got there".</summary>
    public const string Transport = "__transport";

    public const string Unauthorized = "unauthorized";
    public const string InvalidCredentials = "invalid_credentials";
    public const string UsernameTaken = "username_taken";
    public const string Validation = "validation";
    public const string AlreadyInFamily = "already_in_family";
    public const string FamilyRegistrationDisabled = "family_registration_disabled";
    public const string FamilyFull = "family_full";
    public const string NotInFamily = "not_in_family";
    public const string NotFamilyOwner = "not_family_owner";
    public const string InvalidInviteCode = "invalid_invite_code";
    public const string JoinRequestPending = "join_request_pending";
    public const string JoinRequestNotPending = "join_request_not_pending";
    public const string UserAlreadyInFamily = "user_already_in_family";
    public const string OwnerCannotLeave = "owner_cannot_leave";
    public const string CannotRemoveOwner = "cannot_remove_owner";
    public const string CannotDmSelf = "cannot_dm_self";
    public const string CannotBlockSelf = "cannot_block_self";
    public const string Blocked = "blocked";
    public const string CannotReportSelf = "cannot_report_self";
    public const string ReportNotPending = "report_not_pending";
    public const string NotSameFamily = "not_same_family";
    public const string UserNotFound = "user_not_found";
    public const string ChatNotFound = "chat_not_found";
    public const string NotChatMember = "not_chat_member";
    public const string MessageEmpty = "message_empty";
    public const string MessageTooLong = "message_too_long";
    public const string MessageNotFound = "message_not_found";
    public const string NotMessageAuthor = "not_message_author";
    public const string InvalidEmoji = "invalid_emoji";
    public const string NoteNotFound = "note_not_found";
    public const string NotNoteAuthor = "not_note_author";
    public const string InvalidNoteColor = "invalid_note_color";
    public const string InvalidNoteSize = "invalid_note_size";
    public const string InvalidNoteFont = "invalid_note_font";
    public const string InvalidNoteKind = "invalid_note_kind";
    public const string InvalidRsvp = "invalid_rsvp";
    public const string InvalidTask = "invalid_task";
    public const string InvalidLanguage = "invalid_language";
    public const string BoardFull = "board_full";
    public const string InvalidPagination = "invalid_pagination";
    public const string DeviceNotFound = "device_not_found";
    public const string InvalidPoll = "invalid_poll";
    public const string PollClosed = "poll_closed";
    public const string PicturesUnavailable = "pictures_unavailable";
    public const string CallsDisabled = "calls_disabled";
    public const string VideoCallsDisabled = "video_calls_disabled";
    public const string InvalidCall = "invalid_call";
    public const string CallNotFound = "call_not_found";
    public const string CallBusy = "call_busy";
    public const string PeerBusy = "peer_busy";
    public const string PeerUnreachable = "peer_unreachable";
    public const string AvatarTooLarge = "avatar_too_large";
    public const string InvalidImage = "invalid_image";
    public const string AttachmentTooLarge = "attachment_too_large";
    public const string InvalidAttachment = "invalid_attachment";
    public const string AttachmentNotFound = "attachment_not_found";
    public const string AttachmentExpired = "attachment_expired";
    public const string AttachmentAlreadyUsed = "attachment_already_used";
    public const string StorageFull = "storage_full";
    public const string TooManyRequests = "too_many_requests";
    public const string Internal = "internal";

    /// <summary>Every code this document names, for the test that holds the list to it.</summary>
    public static readonly string[] All =
    [
        Unauthorized, InvalidCredentials, UsernameTaken, Validation, AlreadyInFamily,
        FamilyRegistrationDisabled, FamilyFull, NotInFamily, NotFamilyOwner, InvalidInviteCode,
        JoinRequestPending, JoinRequestNotPending, UserAlreadyInFamily, OwnerCannotLeave,
        CannotRemoveOwner, CannotDmSelf, CannotBlockSelf, Blocked, CannotReportSelf,
        ReportNotPending, NotSameFamily, UserNotFound, ChatNotFound, NotChatMember, MessageEmpty,
        MessageTooLong, MessageNotFound, NotMessageAuthor, InvalidEmoji, NoteNotFound,
        NotNoteAuthor, InvalidNoteColor, InvalidNoteSize, InvalidNoteFont, InvalidNoteKind,
        InvalidRsvp, InvalidTask, InvalidLanguage, BoardFull, InvalidPagination, DeviceNotFound,
        InvalidPoll, PollClosed, PicturesUnavailable, CallsDisabled, VideoCallsDisabled,
        InvalidCall, CallNotFound, CallBusy, PeerBusy, PeerUnreachable, AvatarTooLarge,
        InvalidImage, AttachmentTooLarge, InvalidAttachment, AttachmentNotFound,
        AttachmentExpired, AttachmentAlreadyUsed, StorageFull, TooManyRequests, Internal,
    ];
}
