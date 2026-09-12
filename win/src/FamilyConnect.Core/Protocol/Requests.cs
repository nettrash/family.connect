using System.Text.Json.Serialization;

namespace FamilyConnect.Core.Protocol;

/// <summary>
/// The bodies this client sends and the envelopes it reads back (docs/protocol.md,
/// "REST endpoints").
/// </summary>
/// <remarks>
/// Every optional field is nullable and omitted when null (<see cref="Wire.Options"/>), because
/// on this wire ABSENT and EMPTY are different answers: a PATCH that leaves a field out changes
/// nothing, and one that sends it empty clears it. A record with a default would send the default.
/// </remarks>
public sealed record SendRequest(
    [property: JsonPropertyName("client_msg_id")] string ClientMsgId,
    string Body,
    [property: JsonPropertyName("reply_to_message_id")] long? ReplyToMessageId = null,
    [property: JsonPropertyName("attachment_ids")] long[]? AttachmentIds = null,
    PollRequest? Poll = null,
    MentionDto[]? Mentions = null);

public sealed record PollRequest(string[] Options);

/// <summary>
/// A new note. `kind` decides which of the rest are allowed: a photo REQUIRES
/// <see cref="AttachmentId"/>, an event requires <see cref="StartsAt"/> and may carry a backdrop,
/// a list carries <see cref="Items"/>, and a text note refuses all of them.
/// </summary>
public sealed record NoteRequest(
    string Text,
    string Color,
    double X,
    double Y,
    string? Size = null,
    string? Font = null,
    string? Kind = null,
    [property: JsonPropertyName("attachment_id")] long? AttachmentId = null,
    [property: JsonPropertyName("starts_at")] string? StartsAt = null,
    [property: JsonPropertyName("ends_at")] string? EndsAt = null,
    string? Place = null,
    MentionDto[]? Mentions = null,
    TaskLineRequest[]? Items = null);

/// <summary>
/// An edit. EVERY field is optional and only what is sent changes — except the ones that REPLACE:
/// `mentions` re-decides a note's names (sending text without them clears them) and `items`
/// replaces a list's lines, where an entry that keeps its id keeps its tick.
/// </summary>
public sealed record NotePatch(
    string? Text = null,
    string? Color = null,
    string? Size = null,
    string? Font = null,
    double? X = null,
    double? Y = null,
    [property: JsonPropertyName("starts_at")] string? StartsAt = null,
    [property: JsonPropertyName("ends_at")] string? EndsAt = null,
    string? Place = null,
    MentionDto[]? Mentions = null,
    TaskLineRequest[]? Items = null);

/// <summary>
/// One line of a list as the author wrote it: the id where the note already holds one — which is
/// what keeps its tick — and the words. An `id` the note does not hold is `validation`, and ids
/// are the server's to invent.
/// </summary>
public sealed record TaskLineRequest(string Text, long? Id = null);

// ---- what comes back ----------------------------------------------------
//
// EVERY LIST HERE IS NULLABLE, and every reader of one takes `?? []`. Not because the server ever
// omits an array — it does not — but because a 200 is not a promise that the body is THIS body. A
// rewriting proxy, a captive portal or a path the server has since moved all answer readable JSON
// that simply has no `messages` key, and a page whose array came back null cost a whole resync a
// NullReferenceException before this was written down. An answer that is not the documented one
// carries nothing to apply, which is exactly what an empty page means.

public sealed record AuthResponse(string Token, UserDto User);

public sealed record MessageResponse(MessageDto Message);

public sealed record NoteResponse(NoteDto Note);

public sealed record AttachmentResponse(AttachmentDto Attachment);

public sealed record BoardResponse(
    NoteDto[]? Notes,
    [property: JsonPropertyName("max_board_seq")] long MaxBoardSeq);

public sealed record BoardChangesResponse(NoteDto[]? Notes);

public sealed record MessagesResponse(MessageDto[]? Messages);

public sealed record ChatsResponse(ChatRowDto[]? Chats);

/// <summary>
/// A page of the reaction catch-up, oldest sequence first.
/// </summary>
/// <remarks>
/// The cursor advances with EVERY page, even for messages this client does not hold: states for
/// unknown messages are dropped, and history paging re-delivers them embedded on the messages
/// themselves.
/// </remarks>
public sealed record ReactionsResponse(
    [property: JsonPropertyName("message_reactions")] MessageReactionsDto[]? MessageReactions);

public sealed record MessageReactionsDto(
    [property: JsonPropertyName("message_id")] long MessageId,
    [property: JsonPropertyName("reaction_seq")] long ReactionSeq,
    ReactionDto[] Reactions);

/// <summary>A page of the poll catch-up, oldest sequence first.</summary>
public sealed record PollsResponse(MessagePollDto[]? Polls);

public sealed record MessagePollDto(
    [property: JsonPropertyName("message_id")] long MessageId,
    PollDto Poll);

/// <summary>One row of the chat list: the chat, its preview, and this caller's own cursors.</summary>
/// <remarks>
/// The three <c>max_*_seq</c> cursors are absent until something of that kind has ever happened in
/// the chat, and they are high-water marks that never go back down; <c>last_read_message_id</c> is
/// always present, and <c>0</c> means "never reported reading anything here", which is a real
/// answer. <c>mentioned</c> rides only when an unread message names the caller — absent, never
/// false.
/// </remarks>
public sealed record ChatRowDto(
    ChatDto Chat,
    [property: JsonPropertyName("last_message")] MessageDto? LastMessage = null,
    [property: JsonPropertyName("unread_count")] int UnreadCount = 0,
    [property: JsonPropertyName("last_read_message_id")] long LastReadMessageId = 0,
    [property: JsonPropertyName("max_reaction_seq")] long? MaxReactionSeq = null,
    [property: JsonPropertyName("max_edit_seq")] long? MaxEditSeq = null,
    [property: JsonPropertyName("max_poll_seq")] long? MaxPollSeq = null,
    bool? Mentioned = null);

/// <summary>
/// Step 1 of the resync: who I am, whether I have a family, and what this SERVER can do.
/// </summary>
/// <remarks>
/// The capability flags are here rather than discovered as errors — a shut door is shown shut
/// rather than met as a 403 after somebody has typed a name. <c>BlockedUserIds</c> is the one read
/// where absence is not allowed to mean "leave what you hold alone": it is complete state, and
/// <c>[]</c> is the answer when nobody is blocked.
/// </remarks>
public sealed record MeResponse(
    UserDto User,
    FamilyDto? Family = null,
    string? Role = null,
    [property: JsonPropertyName("pending_join_request")] PendingJoinDto? PendingJoinRequest = null,
    [property: JsonPropertyName("calls_enabled")] bool CallsEnabled = false,
    [property: JsonPropertyName("video_calls_enabled")] bool VideoCallsEnabled = false,
    [property: JsonPropertyName("max_family_members")] int MaxFamilyMembers = 0,
    [property: JsonPropertyName("blocked_user_ids")] long[]? BlockedUserIds = null,
    [property: JsonPropertyName("support_contact")] string? SupportContact = null,
    [property: JsonPropertyName("family_registration_enabled")] bool FamilyRegistrationEnabled = true,
    [property: JsonPropertyName("familyless_account_ttl_days")] int FamilylessAccountTtlDays = 0,
    [property: JsonPropertyName("greetings_enabled")] bool GreetingsEnabled = false)
{
    public bool IsOwner => Role == "owner";
}

public sealed record PendingJoinDto(
    [property: JsonPropertyName("family_id")] long FamilyId,
    [property: JsonPropertyName("family_name")] string FamilyName,
    [property: JsonPropertyName("created_at")] string? CreatedAt = null);

public sealed record FamilyDto(
    long Id,
    string Name,
    [property: JsonPropertyName("join_policy")] string? JoinPolicy = null,
    [property: JsonPropertyName("created_at")] string? CreatedAt = null,
    [property: JsonPropertyName("invite_code")] string? InviteCode = null,
    /// <summary>
    /// The owner's own cap, ABSENT when they never set one — which is not the same as "equal to
    /// the operator's ceiling", because the ceiling moves. A client holds both numbers and the
    /// door takes the lower.
    /// </summary>
    [property: JsonPropertyName("max_members")] int? MaxMembers = null,
    [property: JsonPropertyName("ai_history")] bool AiHistory = false,
    [property: JsonPropertyName("ai_vision")] bool AiVision = false,
    [property: JsonPropertyName("ai_history_photos")] bool AiHistoryPhotos = false,
    [property: JsonPropertyName("ai_greeting")] bool AiGreeting = false,
    [property: JsonPropertyName("ai_faces")] bool AiFaces = false,
    string? Language = null);

/// <summary>
/// The family as the family screen needs it, with the assistant's capabilities.
/// </summary>
public sealed record FamilyResponse(
    FamilyDto Family,
    MemberDto[]? Members = null,
    [property: JsonPropertyName("former_members")] MemberDto[]? FormerMembers = null,
    [property: JsonPropertyName("blocked_user_ids")] long[]? BlockedUserIds = null,
    AssistantDto? Assistant = null,
    /// <summary>
    /// The wall's high-water mark, and the ONLY place it is published. ABSENT while the board is
    /// empty and untouched, which is what it is for: a client reads it to know whether a board
    /// catch-up is worth a request at all.
    /// </summary>
    [property: JsonPropertyName("max_board_seq")] long? MaxBoardSeq = null,
    /// <summary>
    /// Who would inherit the family if the owner left RIGHT NOW — the owner's answer only, and a
    /// PREDICTION with no frame of its own. Any join or leave changes it, so it is re-read
    /// immediately before the leave dialog and never named from a cached value; absent on a fresh
    /// read means the owner is the last member and leaving DELETES the family.
    /// </summary>
    [property: JsonPropertyName("next_owner_user_id")] long? NextOwnerUserId = null)
{
    /// <summary>
    /// Whether this server can draw at all — the whole of the capability check for the board's
    /// backdrop and the assistant's `/draw` (docs/protocol.md, "Pictures").
    /// </summary>
    public bool CanDrawPictures => Assistant?.Images == true;

    /// <summary>
    /// ABSENT means no assistant: a client with none must not offer `@ai` in the composer,
    /// because typing it would produce nothing.
    /// </summary>
    public bool HasAssistant => Assistant is not null;
}

public sealed record AssistantDto(
    [property: JsonPropertyName("user_id")] long UserId,
    [property: JsonPropertyName("display_name")] string DisplayName,
    string Mention,
    string? Draw = null,
    bool Vision = false,
    bool Images = false);
