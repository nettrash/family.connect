using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text.Json;

namespace FamilyConnect.Core.Protocol;

/// <summary>
/// The REST half of the protocol: JSON over HTTP under <c>{base}/api/v1</c> (docs/protocol.md,
/// "Transport").
/// </summary>
/// <remarks>
/// <para>
/// The transport rules are the interesting part, and they are all here rather than at each call
/// site: the bearer token on every request, the error shape decoded into <see cref="ApiError"/>
/// with its status and <c>Retry-After</c>, and ONE retry on a GET that failed transiently — a
/// 429, a 5xx, or a request that never arrived at all. A read is safe to repeat, and the
/// alternative is a chat that empties itself because a proxy hiccuped or a pooled connection went
/// stale while the laptop slept. A write is never retried here; the send outbox owns that, because only it knows
/// whether the message has a dedup key (<see cref="SendRules"/>).
/// </para>
/// <para>
/// Apple counterpart: <c>APIClient</c>. Android: <c>ApiClient</c> + the per-area Api interfaces.
/// </para>
/// </remarks>
public sealed class ApiClient(HttpClient http, Uri baseUrl, ITokenStore tokens)
{
    private readonly Uri rest = ServerUrl.Rest(baseUrl);

    /// <summary>The server this client talks to, as the user gave it.</summary>
    public Uri BaseUrl => baseUrl;

    /// <summary>The socket URL for the same server (docs/protocol.md, "WebSocket protocol").</summary>
    public Uri SocketUrl => ServerUrl.Socket(baseUrl);

    // ---- auth ------------------------------------------------------------

    public Task<ApiResult<AuthResponse>> Register(
        string username, string displayName, string password, CancellationToken ct = default) =>
        Send<AuthResponse>(HttpMethod.Post, "/auth/register",
            new { username, display_name = displayName, password }, ct: ct);

    public Task<ApiResult<AuthResponse>> LogIn(
        string username, string password, CancellationToken ct = default) =>
        Send<AuthResponse>(HttpMethod.Post, "/auth/login", new { username, password }, ct: ct);

    public Task<ApiResult<Nothing>> LogOut(CancellationToken ct = default) =>
        Send<Nothing>(HttpMethod.Post, "/auth/logout", ct: ct);

    public Task<ApiResult<MeResponse>> Me(CancellationToken ct = default) =>
        Send<MeResponse>(HttpMethod.Get, "/me", ct: ct);

    public Task<ApiResult<FamilyResponse>> Family(CancellationToken ct = default) =>
        Send<FamilyResponse>(HttpMethod.Get, "/families/mine", ct: ct);

    // ---- chats and messages ---------------------------------------------

    public Task<ApiResult<ChatsResponse>> Chats(CancellationToken ct = default) =>
        Send<ChatsResponse>(HttpMethod.Get, "/chats", ct: ct);

    /// <summary>
    /// One page of a chat, newest first — the paging the chat view walks backwards.
    /// </summary>
    public Task<ApiResult<MessagesResponse>> Messages(
        long chatId, long? beforeId = null, int? limit = null, CancellationToken ct = default)
    {
        var query = new List<string>();
        if (beforeId is { } before)
        {
            query.Add($"before_id={before}");
        }
        if (limit is { } many)
        {
            query.Add($"limit={many}");
        }
        var path = $"/chats/{chatId}/messages" + (query.Count > 0 ? "?" + string.Join("&", query) : "");
        return Send<MessagesResponse>(HttpMethod.Get, path, ct: ct);
    }

    /// <summary>
    /// Send over REST — what the outbox falls back to when a frame went unanswered. The
    /// <c>client_msg_id</c> is the DEDUP KEY: the same id sent twice is one message, which is
    /// what makes a repeat safe (docs/protocol.md, "Sending on an unreliable network").
    /// </summary>
    public Task<ApiResult<MessageResponse>> SendMessage(
        long chatId,
        string clientMsgId,
        string body,
        long? replyToMessageId = null,
        IReadOnlyList<long>? attachmentIds = null,
        IReadOnlyList<string>? pollOptions = null,
        IReadOnlyList<MentionDto>? mentions = null,
        CancellationToken ct = default) =>
        Send<MessageResponse>(HttpMethod.Post, $"/chats/{chatId}/messages", new SendRequest(
            clientMsgId, body, replyToMessageId,
            attachmentIds is { Count: > 0 } ? [.. attachmentIds] : null,
            pollOptions is { Count: > 0 } ? new PollRequest([.. pollOptions]) : null,
            mentions is { Count: > 0 } ? [.. mentions] : null), ct: ct);

    /// <summary>
    /// The reconnect catch-up: strictly newer, OLDEST FIRST — the opposite direction to a history
    /// page, and looped until a short one (docs/protocol.md, "Best-effort delivery").
    /// </summary>
    public Task<ApiResult<MessagesResponse>> MessagesAfter(
        long chatId, long afterId, int limit = 50, CancellationToken ct = default) =>
        Send<MessagesResponse>(
            HttpMethod.Get, $"/chats/{chatId}/messages?after_id={afterId}&limit={limit}", ct: ct);

    /// <summary>
    /// The reaction catch-up, by its own sequence. `after_id` is `WHERE id > cursor` and can never
    /// see a change to an OLDER row, which is why reactions have a sequence of their own.
    /// </summary>
    public Task<ApiResult<ReactionsResponse>> ReactionsAfter(
        long chatId, long afterSeq, int limit = 50, CancellationToken ct = default) =>
        Send<ReactionsResponse>(
            HttpMethod.Get, $"/chats/{chatId}/reactions?after_seq={afterSeq}&limit={limit}", ct: ct);

    /// <summary>The poll catch-up, the same shape one sequence over.</summary>
    public Task<ApiResult<PollsResponse>> PollsAfter(
        long chatId, long afterSeq, int limit = 50, CancellationToken ct = default) =>
        Send<PollsResponse>(
            HttpMethod.Get, $"/chats/{chatId}/polls?after_seq={afterSeq}&limit={limit}", ct: ct);

    /// <summary>
    /// The edit catch-up, which answers whole messages rather than a bespoke patch — so a client
    /// applies them through exactly the same path as a page of history.
    /// </summary>
    public Task<ApiResult<MessagesResponse>> EditsAfter(
        long chatId, long afterSeq, int limit = 50, CancellationToken ct = default) =>
        Send<MessagesResponse>(
            HttpMethod.Get, $"/chats/{chatId}/edits?after_seq={afterSeq}&limit={limit}", ct: ct);

    public Task<ApiResult<Nothing>> MarkRead(
        long chatId, long lastReadMessageId, CancellationToken ct = default) =>
        Send<Nothing>(HttpMethod.Post, $"/chats/{chatId}/read",
            new { last_read_message_id = lastReadMessageId }, ct: ct);

    // ---- the board -------------------------------------------------------

    public Task<ApiResult<BoardResponse>> Board(CancellationToken ct = default) =>
        Send<BoardResponse>(HttpMethod.Get, "/families/mine/board", ct: ct);

    /// <summary>The catch-up, looped until a short page (docs/protocol.md, "Board").</summary>
    public Task<ApiResult<BoardChangesResponse>> BoardChanges(
        long afterSeq, int limit = 50, CancellationToken ct = default) =>
        Send<BoardChangesResponse>(
            HttpMethod.Get, $"/families/mine/board/changes?after_seq={afterSeq}&limit={limit}", ct: ct);

    public Task<ApiResult<NoteResponse>> CreateNote(NoteRequest note, CancellationToken ct = default) =>
        Send<NoteResponse>(HttpMethod.Post, "/families/mine/board/notes", note, ct: ct);

    public Task<ApiResult<NoteResponse>> PatchNote(
        long noteId, NotePatch patch, CancellationToken ct = default) =>
        Send<NoteResponse>(HttpMethod.Patch, $"/families/mine/board/notes/{noteId}", patch, ct: ct);

    /// <summary>An idempotent state-set, not a toggle; ANY member may send it.</summary>
    public Task<ApiResult<NoteResponse>> Answer(
        long noteId, string answer, CancellationToken ct = default) =>
        Send<NoteResponse>(HttpMethod.Put, $"/families/mine/board/notes/{noteId}/rsvp",
            new { answer }, ct: ct);

    public Task<ApiResult<NoteResponse>> RetractAnswer(long noteId, CancellationToken ct = default) =>
        Send<NoteResponse>(HttpMethod.Delete, $"/families/mine/board/notes/{noteId}/rsvp", ct: ct);

    /// <summary>The same shape one field over: a state, and the server records who.</summary>
    public Task<ApiResult<NoteResponse>> TickTask(
        long noteId, long itemId, bool done, CancellationToken ct = default) =>
        Send<NoteResponse>(HttpMethod.Put, $"/families/mine/board/notes/{noteId}/tasks/{itemId}",
            new { done }, ct: ct);

    /// <summary>
    /// Ask the assistant for an event's backdrop. NO REQUEST BODY: the prompt is the note's title
    /// and nothing else (docs/protocol.md, "Board").
    /// </summary>
    public Task<ApiResult<NoteResponse>> DrawBackdrop(long noteId, CancellationToken ct = default) =>
        Send<NoteResponse>(
            HttpMethod.Post, $"/families/mine/board/notes/{noteId}/backdrop", ct: ct);

    public Task<ApiResult<Nothing>> DeleteNote(long noteId, CancellationToken ct = default) =>
        Send<Nothing>(HttpMethod.Delete, $"/families/mine/board/notes/{noteId}", ct: ct);

    // ---- the family's own console ----------------------------------------

    /// <summary>Start a family. The caller becomes its owner and its chat is made with it.</summary>
    public Task<ApiResult<FamilyOnlyResponse>> CreateFamily(
        string name, CancellationToken ct = default) =>
        Send<FamilyOnlyResponse>(HttpMethod.Post, "/families", new { name }, ct: ct);

    /// <summary>
    /// Ask to join one. Under policy <c>open</c> the answer is <c>joined</c> and membership is
    /// immediate; under <c>approval</c> it is <c>pending</c> and an owner has to answer.
    /// </summary>
    /// <remarks>
    /// A CLOSED family answers <c>invalid_invite_code</c>, byte-identical to a code that never
    /// existed: a shut door tells a stranger nothing. A FULL one answers <c>family_full</c>,
    /// which does admit the code is real — the alternative is telling an invited member their
    /// code is invalid on the day the family filled up.
    /// </remarks>
    public Task<ApiResult<JoinAnswer>> JoinFamily(
        string inviteCode, CancellationToken ct = default) =>
        Send<JoinAnswer>(
            HttpMethod.Post, "/families/join", new { invite_code = inviteCode }, ct: ct);

    /// <summary>
    /// Change the family (owner only). Only the fields present change — and the two that can be
    /// CLEARED send an explicit null (see <see cref="FamilyPatch"/>).
    /// </summary>
    public Task<ApiResult<FamilyOnlyResponse>> PatchFamily(
        FamilyPatch patch, CancellationToken ct = default)
    {
        var body = new Dictionary<string, object?>();
        if (patch.JoinPolicy is { } policy)
        {
            body["join_policy"] = policy;
        }
        if (patch.ClearsCap)
        {
            body["max_members"] = null;
        }
        else if (patch.MaxMembers is { } cap)
        {
            body["max_members"] = cap;
        }
        if (patch.ClearsLanguage)
        {
            body["language"] = null;
        }
        else if (patch.Language is { } language)
        {
            body["language"] = language;
        }
        foreach (var (key, value) in new (string, bool?)[]
                 {
                     ("ai_history", patch.AiHistory),
                     ("ai_vision", patch.AiVision),
                     ("ai_history_photos", patch.AiHistoryPhotos),
                     ("ai_greeting", patch.AiGreeting),
                     ("ai_faces", patch.AiFaces),
                 })
        {
            if (value is { } flag)
            {
                body[key] = flag;
            }
        }
        return Send<FamilyOnlyResponse>(HttpMethod.Patch, "/families/mine", body, ct: ct);
    }

    /// <summary>A new invite code (owner). The old one stops working; pending requests survive.</summary>
    public Task<ApiResult<InviteCodeResponse>> RotateInviteCode(CancellationToken ct = default) =>
        Send<InviteCodeResponse>(HttpMethod.Post, "/families/invite-code/rotate", ct: ct);

    public Task<ApiResult<JoinRequestsResponse>> JoinRequests(CancellationToken ct = default) =>
        Send<JoinRequestsResponse>(HttpMethod.Get, "/families/join-requests", ct: ct);

    /// <summary>
    /// Let them in. The cap is re-checked HERE, because the roster can fill between a request and
    /// the decision: <c>family_full</c> leaves the request PENDING — a full family is a temporary
    /// condition and not a decision, and the owner may approve it again once a seat frees.
    /// </summary>
    public Task<ApiResult<MemberResponse>> ApproveJoinRequest(
        long requestId, CancellationToken ct = default) =>
        Send<MemberResponse>(
            HttpMethod.Post, $"/families/join-requests/{requestId}/approve", ct: ct);

    public Task<ApiResult<Nothing>> RejectJoinRequest(
        long requestId, CancellationToken ct = default) =>
        Send<Nothing>(HttpMethod.Post, $"/families/join-requests/{requestId}/reject", ct: ct);

    /// <summary>
    /// Leave. AN OWNER WHO LEAVES HANDS THE FAMILY ON and is never refused; the answer names the
    /// successor, or carries nobody when the family went with them.
    /// </summary>
    public Task<ApiResult<LeftAnswer>> LeaveFamily(CancellationToken ct = default) =>
        Send<LeftAnswer>(HttpMethod.Post, "/families/leave", ct: ct);

    public Task<ApiResult<Nothing>> RemoveMember(long userId, CancellationToken ct = default) =>
        Send<Nothing>(HttpMethod.Delete, $"/families/members/{userId}", ct: ct);

    /// <summary>Your own birthday: a day and a month, and no year at all.</summary>
    public Task<ApiResult<UserResponse>> SetMyBirthday(
        int month, int day, CancellationToken ct = default) =>
        Send<UserResponse>(HttpMethod.Put, "/me/birthday", new { month, day }, ct: ct);

    public Task<ApiResult<Nothing>> ClearMyBirthday(CancellationToken ct = default) =>
        Send<Nothing>(HttpMethod.Delete, "/me/birthday", ct: ct);

    /// <summary>
    /// The owner filling one in for somebody else — a parent for a child, typically, which is
    /// what makes the family calendar usable at all. The owner MAY name themselves here.
    /// </summary>
    public Task<ApiResult<MemberResponse>> SetMemberBirthday(
        long userId, int month, int day, CancellationToken ct = default) =>
        Send<MemberResponse>(
            HttpMethod.Put, $"/families/members/{userId}/birthday", new { month, day }, ct: ct);

    public Task<ApiResult<Nothing>> ClearMemberBirthday(
        long userId, CancellationToken ct = default) =>
        Send<Nothing>(HttpMethod.Delete, $"/families/members/{userId}/birthday", ct: ct);

    /// <summary>
    /// Stop seeing a member. ANY member may block any other, the owner included — and the blocked
    /// member is never told (docs/protocol.md, "Blocking a member").
    /// </summary>
    public Task<ApiResult<Nothing>> Block(long userId, CancellationToken ct = default) =>
        Send<Nothing>(HttpMethod.Put, $"/families/members/{userId}/block", ct: ct);

    /// <summary>
    /// Unblock. Scoped to the CALLER'S OWN list and not to the roster: any id on it may be
    /// cleared, including somebody who has since left or deleted their account.
    /// </summary>
    public Task<ApiResult<Nothing>> Unblock(long userId, CancellationToken ct = default) =>
        Send<Nothing>(HttpMethod.Delete, $"/families/members/{userId}/block", ct: ct);

    /// <summary>
    /// Report a member, or one of their messages. An OPEN report by this caller against this
    /// member is returned as it stands and creates nothing — a double tap is not two rows in the
    /// owner's list, while reporting a second message IS a second report.
    /// </summary>
    public Task<ApiResult<ReportResponse>> Report(
        long reportedUserId, string reason, long? messageId = null, CancellationToken ct = default) =>
        Send<ReportResponse>(
            HttpMethod.Post, "/families/reports",
            messageId is { } named
                ? new { reported_user_id = reportedUserId, reason, message_id = named }
                : (object)new { reported_user_id = reportedUserId, reason },
            ct: ct);

    /// <summary>The owner's moderation list: open only, oldest first.</summary>
    public Task<ApiResult<ReportsResponse>> Reports(CancellationToken ct = default) =>
        Send<ReportsResponse>(HttpMethod.Get, "/families/reports", ct: ct);

    /// <summary>Dealt with — what that MEANS is the owner's business.</summary>
    public Task<ApiResult<Nothing>> ResolveReport(
        long reportId, CancellationToken ct = default) =>
        Send<Nothing>(HttpMethod.Post, $"/families/reports/{reportId}/resolve", ct: ct);

    /// <summary>
    /// What the family has sent. NOT owner-only: it is a shared curiosity, and the same numbers go
    /// to everyone — except that a member the caller has blocked is left out of the rows.
    /// </summary>
    public Task<ApiResult<StatsResponse>> Stats(CancellationToken ct = default) =>
        Send<StatsResponse>(HttpMethod.Get, "/families/mine/stats", ct: ct);

    // ---- attachments -----------------------------------------------------

    /// <summary>
    /// The bytes, raw, with their type; the metadata in the query (docs/protocol.md,
    /// "Photos, videos, audio, files and locations"). Unclaimed until a message or a note names
    /// it, and swept after the grace if nothing ever does.
    /// </summary>
    public async Task<ApiResult<AttachmentResponse>> Upload(
        string kind,
        string mime,
        ReadOnlyMemory<byte> bytes,
        int? width = null,
        int? height = null,
        int? durationMs = null,
        string? name = null,
        CancellationToken ct = default)
    {
        var query = new List<string> { $"kind={Uri.EscapeDataString(kind)}" };
        if (width is { } w)
        {
            query.Add($"width={w}");
        }
        if (height is { } h)
        {
            query.Add($"height={h}");
        }
        if (durationMs is { } ms)
        {
            query.Add($"duration_ms={ms}");
        }
        if (!string.IsNullOrEmpty(name))
        {
            query.Add($"name={Uri.EscapeDataString(name)}");
        }
        using var content = new ReadOnlyMemoryContent(bytes);
        content.Headers.ContentType = new MediaTypeHeaderValue(mime);
        return await Send<AttachmentResponse>(
            HttpMethod.Post, "/attachments?" + string.Join("&", query), content: content, ct: ct);
    }

    /// <summary>One attachment's bytes, or its preview.</summary>
    public async Task<ApiResult<byte[]>> Download(
        long attachmentId, bool preview = false, CancellationToken ct = default)
    {
        var path = preview ? $"/attachments/{attachmentId}/preview" : $"/attachments/{attachmentId}";
        using var request = Request(HttpMethod.Get, path);
        try
        {
            using var response = await http.SendAsync(request, ct).ConfigureAwait(false);
            if (!response.IsSuccessStatusCode)
            {
                return ApiResult<byte[]>.Failure(await Failure(response, ct).ConfigureAwait(false));
            }
            return ApiResult<byte[]>.Success(
                await response.Content.ReadAsByteArrayAsync(ct).ConfigureAwait(false));
        }
        catch (Exception exception) when (Unreached(exception, ct))
        {
            return ApiResult<byte[]>.Failure(ApiError.Transport(exception.Message));
        }
    }

    // ---- the transport ---------------------------------------------------

    private HttpRequestMessage Request(HttpMethod method, string path)
    {
        var request = new HttpRequestMessage(method, new Uri(rest + path));
        if (tokens.Token is { Length: > 0 } token)
        {
            request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token);
        }
        return request;
    }

    private async Task<ApiResult<T>> Send<T>(
        HttpMethod method,
        string path,
        object? body = null,
        HttpContent? content = null,
        CancellationToken ct = default)
    {
        // A read is safe to repeat and a write is not: only the outbox knows whether a send
        // carries a dedup key, so nothing here retries a POST, PATCH, PUT or DELETE.
        var mayRetry = method == HttpMethod.Get;
        for (var attempt = 0; ; attempt++)
        {
            using var request = Request(method, path);
            if (content is not null)
            {
                request.Content = content;
            }
            else if (body is not null)
            {
                request.Content = JsonContent.Create(body, options: Wire.Options);
            }
            HttpResponseMessage response;
            try
            {
                response = await http.SendAsync(request, ct).ConfigureAwait(false);
            }
            catch (Exception exception) when (Unreached(exception, ct))
            {
                // A read that never arrived is retried once, like a read that hit a 502: a
                // pooled connection dropped while idle fails exactly this way, and the first
                // call after a laptop wakes up is the common case.
                if (mayRetry && attempt == 0)
                {
                    try
                    {
                        await Task.Delay(TimeSpan.FromMilliseconds(250), ct).ConfigureAwait(false);
                    }
                    catch (OperationCanceledException)
                    {
                        return ApiResult<T>.Failure(ApiError.Transport(exception.Message));
                    }
                    continue;
                }
                return ApiResult<T>.Failure(ApiError.Transport(exception.Message));
            }
            using (response)
            {
                if (response.IsSuccessStatusCode)
                {
                    return await Body<T>(response, ct).ConfigureAwait(false);
                }
                var error = await Failure(response, ct).ConfigureAwait(false);
                if (mayRetry && attempt == 0 && error.Transient)
                {
                    // Once, and only on a read. Honour the server's own wait where it gave one.
                    var wait = error.RetryAfter ?? TimeSpan.FromMilliseconds(250);
                    if (wait > SendRules.RetryAfterCap)
                    {
                        wait = SendRules.RetryAfterCap;
                    }
                    if (wait > TimeSpan.Zero)
                    {
                        try
                        {
                            await Task.Delay(wait, ct).ConfigureAwait(false);
                        }
                        catch (OperationCanceledException)
                        {
                            return ApiResult<T>.Failure(error);
                        }
                    }
                    continue;
                }
                return ApiResult<T>.Failure(error);
            }
        }
    }

    private static async Task<ApiResult<T>> Body<T>(HttpResponseMessage response, CancellationToken ct)
    {
        if (typeof(T) == typeof(Nothing))
        {
            return ApiResult<T>.Success((T)(object)Nothing.Value);
        }
        var text = await response.Content.ReadAsStringAsync(ct).ConfigureAwait(false);
        var value = Wire.Decode<T>(text);
        // A 2xx whose body this client cannot read is not a success it can act on — and it is
        // TERMINAL, not transient: repeating the call will produce the same body.
        return value is null
            ? ApiResult<T>.Failure(new ApiError(
                ErrorCodes.Validation, "the answer could not be read", (int)response.StatusCode))
            : ApiResult<T>.Success(value);
    }

    /// <summary>
    /// The failure, read as the protocol writes it — and classified without it where nginx
    /// answered its own rate limit with an HTML body.
    /// </summary>
    private static async Task<ApiError> Failure(HttpResponseMessage response, CancellationToken ct)
    {
        var status = (int)response.StatusCode;
        var retryAfter = response.Headers.RetryAfter?.Delta
            ?? (response.Headers.RetryAfter?.Date is { } when
                ? when - DateTimeOffset.UtcNow
                : null);
        string code = status == 429 ? ErrorCodes.TooManyRequests : "";
        var message = response.ReasonPhrase ?? string.Empty;
        try
        {
            var text = await response.Content.ReadAsStringAsync(ct).ConfigureAwait(false);
            if (Wire.Decode<ErrorEnvelope>(text) is { } envelope)
            {
                code = envelope.Error.Code;
                message = envelope.Error.Message;
            }
        }
        catch (Exception exception) when (exception is HttpRequestException or IOException or JsonException)
        {
            // A body that cannot be read changes nothing: the status still classifies it.
        }
        return new ApiError(code, message, status, retryAfter);
    }

    /// <summary>
    /// Whether this exception means the request never got an answer — a transport failure, which
    /// says nothing about the request. A cancellation the CALLER asked for is not one of those and
    /// is left to propagate.
    /// </summary>
    private static bool Unreached(Exception exception, CancellationToken ct) =>
        exception switch
        {
            OperationCanceledException => !ct.IsCancellationRequested,
            HttpRequestException or IOException or TimeoutException => true,
            _ => false,
        };
}
