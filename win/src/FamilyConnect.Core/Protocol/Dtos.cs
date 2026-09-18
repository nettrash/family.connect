using System.Text.Json;
using System.Text.Json.Serialization;

namespace FamilyConnect.Core.Protocol;

/// <summary>
/// The objects on the wire (docs/protocol.md, "Objects"), as records this client decodes into.
/// </summary>
/// <remarks>
/// <para>
/// Two habits the whole protocol depends on, and they are the reason every optional field here is
/// nullable rather than defaulted: <b>an absent key is not an empty one</b> — a message with no
/// reactions omits <c>reactions</c>, while one whose last reaction was removed sends
/// <c>"reactions": []</c>, and clients distinguish "cleared" from "no data" — and <b>an unknown
/// value falls back rather than failing</b>, which is why kinds, sizes, colours and fonts are
/// carried as strings and resolved through <see cref="Board.Notes"/>.
/// </para>
/// <para>
/// <see cref="Wire.Options"/> is the one serializer: snake_case is spelled out per property
/// instead of guessed by a policy, because the wire is a document and not a convention.
/// </para>
/// </remarks>
public static class Wire
{
    public static readonly JsonSerializerOptions Options = new(JsonSerializerDefaults.Web)
    {
        // An unknown field is a NEWER SERVER, not an error: the compatibility rule is that a
        // client ignores what it does not know (docs/protocol.md, "Compatibility rules").
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Skip,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        NumberHandling = JsonNumberHandling.AllowReadingFromString,
    };

    public static T? Decode<T>(string json)
    {
        try
        {
            return JsonSerializer.Deserialize<T>(json, Options);
        }
        catch (JsonException)
        {
            return default;
        }
    }

    public static string Encode<T>(T value) => JsonSerializer.Serialize(value, Options);
}

public sealed record UserDto(
    long Id,
    string Username,
    string DisplayName,
    int AvatarVersion = 0,
    bool Deleted = false,
    BirthdayDto? Birthday = null);

/// <summary>A member of the family, with the two flags that outlive them.</summary>
/// <remarks>
/// A member who has LEFT and an account that was DELETED both keep their rows, because their
/// messages and their notes keep their authors: a name has to resolve for an old message, and
/// "Deleted account" is a name this client supplies rather than the server.
/// </remarks>
public sealed record MemberDto(
    long Id,
    string Username,
    string DisplayName,
    int AvatarVersion = 0,
    string? Role = null,
    bool HasLeft = false,
    bool Deleted = false,
    BirthdayDto? Birthday = null)
{
    /// <summary>
    /// Whether this member owns the family. The wire says <c>"role": "owner"|"member"</c> — a
    /// STRING, and there is no <c>owner</c> boolean to decode: a port that declared one got
    /// `false` for everybody, including the owner reading their own roster, and every
    /// owner-only door in the app would have been shut. The same idiom as
    /// <see cref="MeResponse.IsOwner"/>, deliberately.
    /// </summary>
    public bool Owner => Role == "owner";

    /// <summary>
    /// A member who is GONE: they left, or their account was deleted. Both keep their row, and
    /// this is the one question a name-resolving caller asks about it.
    /// </summary>
    public bool IsFormer => HasLeft || Deleted;
}

/// <summary>
/// A day and a month, with NO YEAR — "nobody should have to publish their age to be wished a
/// happy birthday" (docs/protocol.md). 29 February is a birthday here, because there is no year
/// for it to fail to exist in.
/// </summary>
/// <remarks>
/// ONE OBJECT, NOT TWO SIBLING FIELDS, and that shape is load-bearing for a reason worth writing
/// down: this port first declared it a string, which made `GET /families/mine` UNREADABLE for any
/// family where somebody had set one — the decode threw, the answer came back as a terminal
/// "2xx I cannot read", and the whole resync stopped. A field whose type is wrong does not
/// degrade; it takes the document with it.
/// </remarks>
public sealed record BirthdayDto(int Month, int Day);

public sealed record ChatDto(
    long Id,
    string Kind,
    string Title,
    long? PeerUserId = null);

public sealed record MentionDto(long UserId, string Name);

public sealed record ReactionDto(long UserId, string Emoji);

public sealed record ReplyToDto(long MessageId, long SenderId, string Excerpt);

public sealed record AttachmentDto(
    long Id,
    string Kind,
    string? Mime = null,
    long? Size = null,
    int? Width = null,
    int? Height = null,
    int? DurationMs = null,
    bool HasPreview = false,
    string? Name = null,
    double? Latitude = null,
    double? Longitude = null,
    double? AccuracyM = null)
{
    public bool IsPhoto => Kind == "photo";
    public bool IsVideo => Kind == "video";

    /// <summary>
    /// The shape to reserve before the bytes arrive, or null when the uploader could not say.
    /// A tile built from this is a tile that never jumps — and never crops (issue #71).
    /// </summary>
    public double? AspectRatio =>
        Width is > 0 && Height is > 0 ? (double)Width.Value / Height.Value : null;
}

public sealed record PollOptionDto(long Id, string Text, long[] Votes);

public sealed record PollDto(long PollSeq, bool Closed, PollOptionDto[] Options);

public sealed record CallRecordDto(string Outcome, int? DurationSecs = null, bool Video = false);

public sealed record MessageDto(
    long Id,
    long ChatId,
    long SenderId,
    string? ClientMsgId,
    string Body,
    string CreatedAt,
    ReactionDto[]? Reactions = null,
    long? ReactionSeq = null,
    ReplyToDto? ReplyTo = null,
    long? ThreadRootId = null,
    int? ReplyCount = null,
    MentionDto[]? Mentions = null,
    string? EditedAt = null,
    long? EditSeq = null,
    AttachmentDto[]? Attachments = null,
    AttachmentDto? Attachment = null,
    PollDto? Poll = null,
    CallRecordDto? Call = null)
{
    /// <summary>
    /// Everything this message carries, in the sender's order — reading <c>attachments</c> and
    /// falling back to the singular only for a server that predates plurality. A client that reads
    /// the plural ignores the singular; the two are never present without each other.
    /// </summary>
    public IReadOnlyList<AttachmentDto> Media =>
        Attachments ?? (Attachment is null ? [] : [Attachment]);
}

public sealed record TaskItemDto(long Id, string Text, bool Done, long? DoneBy = null);

public sealed record RsvpDto(long UserId, string Answer);

public sealed record NoteDto(
    long Id,
    long AuthorId,
    string? Kind = null,
    string? Text = null,
    string? Color = null,
    string? Size = null,
    string? Font = null,
    double X = 0,
    double Y = 0,
    string? CreatedAt = null,
    string? UpdatedAt = null,
    long BoardSeq = 0,
    long? ContentSeq = null,
    AttachmentDto? Attachment = null,
    string? StartsAt = null,
    string? EndsAt = null,
    string? Place = null,
    RsvpDto[]? Rsvps = null,
    TaskItemDto[]? Items = null,
    MentionDto[]? Mentions = null,
    bool Deleted = false)
{
    /// <summary>How many people gave this answer — the number the sticker shows.</summary>
    public int Count(string answer) => Rsvps?.Count(rsvp => rsvp.Answer == answer) ?? 0;

    /// <summary>This reader's own answer, or null while they have not said.</summary>
    public string? MyAnswer(long userId) =>
        Rsvps?.FirstOrDefault(rsvp => rsvp.UserId == userId)?.Answer;

    /// <summary>
    /// The lines of a list. An EMPTY list sends <c>[]</c> and every other kind sends nothing at
    /// all, which is the difference between "a list with nothing on it yet" and "not a list".
    /// </summary>
    public IReadOnlyList<TaskItemDto> TaskList => Items ?? [];
}

/// <summary>The error body, as it arrives.</summary>
public sealed record ErrorEnvelope(ErrorBody Error);

public sealed record ErrorBody(string Code, string Message);
