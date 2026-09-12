using System.Text.Json;
using System.Text.Json.Nodes;

namespace FamilyConnect.Core.Protocol;

/// <summary>
/// The realtime frames (docs/protocol.md, "WebSocket protocol"): JSON text messages tagged by
/// <c>"type"</c>.
/// </summary>
/// <remarks>
/// <para>
/// The parser answers null for a frame it does not know, and that is the rule rather than
/// laziness: a newer server may add a type, and a client that threw would drop the connection over
/// something it was free to ignore.
/// </para>
/// <para>
/// Web counterpart: <c>web/src/socket.rs</c>. Apple: <c>SocketFrames</c>.
/// </para>
/// </remarks>
public abstract record ServerFrame
{
    /// <summary>A message that is NEW: it bumps unread counts and may raise a notification.</summary>
    public sealed record Message(MessageDto Value) : ServerFrame;

    /// <summary>
    /// The answer to this device's own <c>send</c>. Carries the whole message, so the pending row
    /// is replaced rather than patched.
    /// </summary>
    public sealed record Ack(string ClientMsgId, MessageDto Value) : ServerFrame;

    /// <summary>
    /// An edit. A SEPARATE type from <see cref="Message"/> on purpose: an edit must not bump an
    /// unread count or raise a notification, and it carries the WHOLE message — the assistant's
    /// picture answer arrives as an attachment added by exactly this frame, and a body-only merge
    /// draws nothing.
    /// </summary>
    public sealed record MessageEdited(MessageDto Value) : ServerFrame;

    public sealed record Read(long ChatId, long UserId, long LastReadMessageId) : ServerFrame;

    public sealed record Typing(long ChatId, long UserId) : ServerFrame;

    /// <summary>Complete state, never a delta — <c>reactions: []</c> means cleared.</summary>
    public sealed record Reactions(
        long ChatId, long MessageId, long ReactionSeq, ReactionDto[] Value) : ServerFrame;

    public sealed record Poll(long ChatId, long MessageId, PollDto Value) : ServerFrame;

    public sealed record BoardNote(NoteDto Note) : ServerFrame;

    public sealed record MemberJoined(long? FamilyId, UserDto User) : ServerFrame;

    public sealed record MemberLeft(long? FamilyId, long UserId) : ServerFrame;

    public sealed record MemberDeleted(long? FamilyId, MemberDto Member) : ServerFrame;

    public sealed record FamilyOwner(long? FamilyId, long UserId) : ServerFrame;

    public sealed record MemberBlocked(long UserId, bool Blocked) : ServerFrame;

    /// <summary>The assistant, mid-reply.</summary>
    public sealed record AiDelta(long ChatId, long MessageId, string Text) : ServerFrame;

    /// <summary>The assistant stopped early.</summary>
    public sealed record AiError(long ChatId, long MessageId) : ServerFrame;

    public sealed record CallOffer(
        string CallId, long ChatId, long FromUserId, string Sdp, bool Video) : ServerFrame;

    public sealed record CallRinging(string CallId) : ServerFrame;

    public sealed record CallAnswer(string CallId, string Sdp) : ServerFrame;

    public sealed record CallIce(string CallId, IceCandidate Candidate) : ServerFrame;

    public sealed record CallEnd(string CallId, string Reason) : ServerFrame;

    public sealed record Pong : ServerFrame;

    /// <summary>
    /// A refusal. <c>ClientMsgId</c> is present when it answers a <c>send</c> and <c>CallId</c>
    /// when it answers a call frame — never both.
    /// </summary>
    /// <remarks>
    /// The wire calls the sentence <c>message</c>; here it is <c>Detail</c>, because
    /// <see cref="Message"/> is already a frame. It is English for a log either way — every
    /// client shows its own wording for a code.
    /// </remarks>
    public sealed record Error(string Code, string Detail, string? ClientMsgId, string? CallId)
        : ServerFrame;

    /// <summary>The frame, or null when this client has no use for it.</summary>
    public static ServerFrame? Parse(string json)
    {
        JsonNode? node;
        try
        {
            node = JsonNode.Parse(json);
        }
        catch (JsonException)
        {
            return null;
        }
        if (node is not JsonObject frame)
        {
            return null;
        }
        var type = frame["type"]?.GetValue<string>();
        return type switch
        {
            "message" => Decode<MessageDto>(frame["message"]) is { } message
                ? new Message(message) : null,
            "ack" => Decode<MessageDto>(frame["message"]) is { } acked
                && frame["client_msg_id"]?.GetValue<string>() is { } id
                ? new Ack(id, acked) : null,
            "message_edited" => Decode<MessageDto>(frame["message"]) is { } edited
                ? new MessageEdited(edited) : null,
            "read" => new Read(
                Number(frame["chat_id"]), Number(frame["user_id"]), Number(frame["last_read_message_id"])),
            "typing" => new Typing(Number(frame["chat_id"]), Number(frame["user_id"])),
            "reaction" => new Reactions(
                Number(frame["chat_id"]), Number(frame["message_id"]), Number(frame["reaction_seq"]),
                Decode<ReactionDto[]>(frame["reactions"]) ?? []),
            "poll" => Decode<PollDto>(frame["poll"]) is { } poll
                ? new Poll(Number(frame["chat_id"]), Number(frame["message_id"]), poll) : null,
            "board_note" => Decode<NoteDto>(frame["note"]) is { } note ? new BoardNote(note) : null,
            "member_joined" => Decode<UserDto>(frame["user"]) is { } user
                ? new MemberJoined(Optional(frame["family_id"]), user) : null,
            "member_left" => new MemberLeft(Optional(frame["family_id"]), Number(frame["user_id"])),
            "member_deleted" => Decode<MemberDto>(frame["member"]) is { } member
                ? new MemberDeleted(Optional(frame["family_id"]), member) : null,
            "family_owner" => new FamilyOwner(Optional(frame["family_id"]), Number(frame["user_id"])),
            "member_blocked" => new MemberBlocked(
                Number(frame["user_id"]), frame["blocked"]?.GetValue<bool>() ?? false),
            "ai_delta" => new AiDelta(
                Number(frame["chat_id"]), Number(frame["message_id"]),
                frame["text"]?.GetValue<string>() ?? string.Empty),
            "ai_error" => new AiError(Number(frame["chat_id"]), Number(frame["message_id"])),
            "call_offer" => frame["call_id"]?.GetValue<string>() is { } offered
                ? new CallOffer(
                    offered, Number(frame["chat_id"]), Number(frame["from_user_id"]),
                    frame["sdp"]?.GetValue<string>() ?? string.Empty,
                    frame["video"]?.GetValue<bool>() ?? false)
                : null,
            "call_ringing" => frame["call_id"]?.GetValue<string>() is { } ringing
                ? new CallRinging(ringing) : null,
            "call_answer" => frame["call_id"]?.GetValue<string>() is { } answered
                ? new CallAnswer(answered, frame["sdp"]?.GetValue<string>() ?? string.Empty) : null,
            "call_ice" => frame["call_id"]?.GetValue<string>() is { } iced
                && Decode<IceCandidate>(frame["candidate"]) is { } candidate
                ? new CallIce(iced, candidate) : null,
            "call_end" => frame["call_id"]?.GetValue<string>() is { } ended
                ? new CallEnd(ended, frame["reason"]?.GetValue<string>() ?? "failed") : null,
            "pong" => new Pong(),
            "error" => new Error(
                frame["code"]?.GetValue<string>() ?? ErrorCodes.Internal,
                frame["message"]?.GetValue<string>() ?? string.Empty,
                frame["client_msg_id"]?.GetValue<string>(),
                frame["call_id"]?.GetValue<string>()),
            _ => null,
        };
    }

    private static T? Decode<T>(JsonNode? node) =>
        node is null ? default : Wire.Decode<T>(node.ToJsonString());

    private static long Number(JsonNode? node)
    {
        try
        {
            return node?.GetValue<long>() ?? 0;
        }
        catch (Exception exception) when (exception is FormatException or InvalidOperationException)
        {
            return 0;
        }
    }

    private static long? Optional(JsonNode? node) => node is null ? null : Number(node);
}

/// <summary>
/// One ICE candidate. <c>sdp_mid</c> and <c>sdp_mline_index</c> are each optional: a WebRTC stack
/// supplies one, the other, or both, and the receiving stack accepts whichever it was given.
/// </summary>
public sealed record IceCandidate(string Candidate, string? SdpMid = null, int? SdpMlineIndex = null);

/// <summary>
/// What this client sends. Written as JSON here rather than by the serializer's convention,
/// because the frame names are a document.
/// </summary>
public static class ClientFrames
{
    /// <summary>
    /// A message. The <c>client_msg_id</c> must be a real UUID and is what the <c>ack</c> comes
    /// back with — it is this device's handle on a row it has already drawn.
    /// </summary>
    public static string Send(
        long chatId,
        string clientMsgId,
        string body,
        long? replyToMessageId = null,
        IReadOnlyList<long>? attachmentIds = null,
        IReadOnlyList<string>? pollOptions = null,
        IReadOnlyList<MentionDto>? mentions = null)
    {
        var frame = new JsonObject
        {
            ["type"] = "send",
            ["chat_id"] = chatId,
            ["client_msg_id"] = clientMsgId,
            ["body"] = body,
        };
        if (replyToMessageId is { } reply)
        {
            frame["reply_to_message_id"] = reply;
        }
        if (attachmentIds is { Count: > 0 })
        {
            frame["attachment_ids"] = new JsonArray([.. attachmentIds.Select(id => JsonValue.Create(id))]);
        }
        if (pollOptions is { Count: > 0 })
        {
            frame["poll"] = new JsonObject
            {
                ["options"] = new JsonArray([.. pollOptions.Select(text => JsonValue.Create(text))]),
            };
        }
        if (mentions is { Count: > 0 })
        {
            frame["mentions"] = new JsonArray([.. mentions.Select(mention => (JsonNode)new JsonObject
            {
                ["user_id"] = mention.UserId,
                ["name"] = mention.Name,
            })]);
        }
        return frame.ToJsonString();
    }

    public static string Read(long chatId, long lastReadMessageId) =>
        new JsonObject
        {
            ["type"] = "read",
            ["chat_id"] = chatId,
            ["last_read_message_id"] = lastReadMessageId,
        }.ToJsonString();

    public static string Typing(long chatId) =>
        new JsonObject { ["type"] = "typing", ["chat_id"] = chatId }.ToJsonString();

    public static string Ping() => new JsonObject { ["type"] = "ping" }.ToJsonString();

    public static string CallOffer(string callId, long chatId, string sdp, bool video = false)
    {
        var frame = new JsonObject
        {
            ["type"] = "call_offer",
            ["call_id"] = callId,
            ["chat_id"] = chatId,
            ["sdp"] = sdp,
        };
        if (video)
        {
            // Absent on a voice call, like every optional field on this wire — never false.
            frame["video"] = true;
        }
        return frame.ToJsonString();
    }

    public static string CallAnswer(string callId, string sdp) =>
        new JsonObject
        {
            ["type"] = "call_answer",
            ["call_id"] = callId,
            ["sdp"] = sdp,
        }.ToJsonString();

    public static string CallIce(string callId, IceCandidate candidate)
    {
        var payload = new JsonObject { ["candidate"] = candidate.Candidate };
        if (candidate.SdpMid is { } mid)
        {
            payload["sdp_mid"] = mid;
        }
        if (candidate.SdpMlineIndex is { } index)
        {
            payload["sdp_mline_index"] = index;
        }
        return new JsonObject
        {
            ["type"] = "call_ice",
            ["call_id"] = callId,
            ["candidate"] = payload,
        }.ToJsonString();
    }

    public static string CallEnd(string callId, string reason) =>
        new JsonObject
        {
            ["type"] = "call_end",
            ["call_id"] = callId,
            ["reason"] = reason,
        }.ToJsonString();
}
