using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>
/// How many people may still join, and why. The two numbers are kept apart on purpose: they can
/// legitimately disagree, and a screen that showed only one of them would be wrong about the
/// other.
/// </summary>
/// <param name="Cap">
/// The family's OWN cap, or null when the owner never set one. Absent is NOT the ceiling: "we
/// never set one" is not "we set one equal to whatever the ceiling happens to be today", and the
/// ceiling moves between server restarts.
/// </param>
/// <param name="Ceiling">The operator's, from <c>GET /me</c>.</param>
/// <param name="Members">How many are in the family now — the owner included.</param>
public readonly record struct Seats(int? Cap, int Ceiling, int Members)
{
    /// <summary>
    /// The door takes the LOWER of the two. A family that set 40 under a ceiling of 50 goes on
    /// reporting 40 after the operator drops the ceiling to 10 — a stored cap is never
    /// re-validated when the ceiling moves — so what is actually left is the lower one.
    /// </summary>
    public int Limit => Cap is { } cap ? Math.Min(cap, Ceiling) : Ceiling;

    /// <summary>How many may still come in. Never negative: a cap below the room is a freeze.</summary>
    public int Left => Math.Max(0, Limit - Members);

    /// <summary>Whether anybody else can join at all.</summary>
    public bool Full => Left == 0;
}

/// <summary>
/// The family's own screens: the roster, the door, the owner's console and the numbers everybody
/// may see.
/// </summary>
/// <remarks>
/// <para>
/// <b>A SUCCESSOR IS A PREDICTION AND NEVER A CACHED VALUE.</b> <c>next_owner_user_id</c> changes
/// with any join or leave and takes no frame of its own, so the leave dialog is drawn from a FRESH
/// <c>GET /families/mine</c> every time — and its ABSENCE on that fresh read means the owner is
/// the last member and leaving DELETES the family, which is a different dialog and a different
/// confirmation.
/// </para>
/// <para>
/// <b>THE ASSISTANT'S SWITCHES HAVE A DEPENDENCY, AND THIS SIDE KEEPS IT TOO.</b>
/// <c>ai_history_photos</c> and <c>ai_faces</c> may only be true while <c>ai_vision</c> is: the
/// server refuses the combination and turns both off whenever vision goes off, whether or not the
/// request mentioned them. A client that let a person tick a box the server would refuse — or
/// that drew the two as still on after vision went off — would be lying about what the family had
/// agreed to.
/// </para>
/// <para>
/// <b>THE STATISTICS' ROWS DO NOT ADD UP TO THE TOTALS, AND THE GAP IS THE BLOCK.</b> The totals
/// are the family's numbers; the rows are what this caller may see of them. Deriving a total by
/// summing the rows, or a share by dividing into that sum, is how a client tells two members
/// different things about one family.
/// </para>
/// </remarks>
public sealed class FamilyModel(ApiClient api, ChatStore chats)
{
    /// <summary>The roster as this device holds it — including the members who are gone.</summary>
    public IReadOnlyList<MemberDto> Members() => chats.Members();

    /// <summary>Who is still IN the family, which is who the console acts on.</summary>
    public IReadOnlyList<MemberDto> Present() =>
        [.. chats.Members().Where(member => !member.IsFormer)];

    /// <summary>Everyone this reader has chosen not to see, roster or no roster.</summary>
    public IReadOnlyList<long> Blocked() => chats.Blocked();

    /// <summary>
    /// How many seats are left, from the family's own cap and the operator's ceiling — which is
    /// the only honest answer, because either alone can be the binding one.
    /// </summary>
    public Seats SeatsFor(FamilyDto family, int ceiling) =>
        new(family.MaxMembers, ceiling, Present().Count);

    /// <summary>
    /// Whether a person may tick this switch as things stand. The two that depend on vision are
    /// not offered while vision is off — the server would refuse them, and an offer that cannot
    /// be taken is a worse answer than a disabled one.
    /// </summary>
    public static bool MayTurnOn(string switchName, FamilyDto family) => switchName switch
    {
        "ai_history_photos" or "ai_faces" => family.AiVision,
        _ => true,
    };

    /// <summary>
    /// What the family will look like after this patch, as the SERVER will apply it — so the
    /// screen does not claim the two dependent switches are still on after vision goes off.
    /// </summary>
    public static FamilyDto AsApplied(FamilyDto family, FamilyPatch patch)
    {
        var vision = patch.AiVision ?? family.AiVision;
        var photos = patch.AiHistoryPhotos ?? family.AiHistoryPhotos;
        var faces = patch.AiFaces ?? family.AiFaces;
        return family with
        {
            JoinPolicy = patch.JoinPolicy ?? family.JoinPolicy,
            MaxMembers = patch.ClearsCap ? null : patch.MaxMembers ?? family.MaxMembers,
            Language = patch.ClearsLanguage ? null : patch.Language ?? family.Language,
            AiHistory = patch.AiHistory ?? family.AiHistory,
            AiVision = vision,
            // Turning vision off turns both of these off in the same write, whether or not the
            // request mentioned them.
            AiHistoryPhotos = vision && photos,
            AiFaces = vision && faces,
            AiGreeting = patch.AiGreeting ?? family.AiGreeting,
        };
    }

    /// <summary>
    /// Send a change, having first made it one the server can accept: a dependent switch is never
    /// sent true alongside vision off, because that answer is <c>validation</c> and the person
    /// would be told their own screen was wrong.
    /// </summary>
    public async Task<(FamilyDto? Family, ApiError? Error)> ChangeAsync(
        FamilyDto family, FamilyPatch patch, CancellationToken ct = default)
    {
        var vision = patch.AiVision ?? family.AiVision;
        var sending = vision
            ? patch
            : patch with { AiHistoryPhotos = null, AiFaces = null };
        var answer = await api.PatchFamily(sending, ct).ConfigureAwait(false);
        return answer.Ok && answer.Value is not null
            ? (answer.Value.Family, null)
            : (null, answer.Error ?? ApiError.Transport("no answer"));
    }

    /// <summary>
    /// What the leave dialog must say, read FRESH. Answers the successor's name where there is
    /// one; a null successor means leaving deletes the family.
    /// </summary>
    public async Task<(bool DeletesFamily, MemberDto? Successor, ApiError? Error)> LeaveWouldAsync(
        CancellationToken ct = default)
    {
        var family = await api.Family(ct).ConfigureAwait(false);
        if (!family.Ok || family.Value is null)
        {
            return (false, null, family.Error ?? ApiError.Transport("no answer"));
        }
        // The roster is refreshed by the same read, because the successor has to resolve to a
        // name and a member who joined since would not be in the cache.
        chats.Replace(family.Value.Members ?? [], family.Value.FormerMembers);
        if (family.Value.NextOwnerUserId is not { } successor)
        {
            return (true, null, null);
        }
        return (false, chats.Member(successor), null);
    }

    /// <summary>Leave. The answer names the successor the server actually chose.</summary>
    public async Task<(long? Successor, ApiError? Error)> LeaveAsync(CancellationToken ct = default)
    {
        var answer = await api.LeaveFamily(ct).ConfigureAwait(false);
        return answer.Ok
            ? (answer.Value.NewOwnerUserId, null)
            : (null, answer.Error);
    }

    /// <summary>The requests waiting for an answer, oldest first.</summary>
    public async Task<(IReadOnlyList<JoinRequestDto> Requests, ApiError? Error)> RequestsAsync(
        CancellationToken ct = default)
    {
        var answer = await api.JoinRequests(ct).ConfigureAwait(false);
        return answer.Ok && answer.Value is not null
            ? (answer.Value.Requests ?? [], null)
            : ([], answer.Error ?? ApiError.Transport("no answer"));
    }

    /// <summary>
    /// Approve one. A <c>family_full</c> refusal leaves the request PENDING and says so, because
    /// a full family is a temporary condition rather than a decision — the owner may come back to
    /// it once a seat frees, and a client that struck the row off its list would have thrown the
    /// decision away.
    /// </summary>
    public async Task<(MemberDto? Joined, bool StillPending, ApiError? Error)> ApproveAsync(
        long requestId, CancellationToken ct = default)
    {
        var answer = await api.ApproveJoinRequest(requestId, ct).ConfigureAwait(false);
        if (answer.Ok && answer.Value is not null)
        {
            chats.Joined(new UserDto(
                answer.Value.Member.Id,
                answer.Value.Member.Username,
                answer.Value.Member.DisplayName,
                answer.Value.Member.AvatarVersion,
                Birthday: answer.Value.Member.Birthday));
            return (answer.Value.Member, false, null);
        }
        var error = answer.Error ?? ApiError.Transport("no answer");
        return (null, error.Code == ErrorCodes.FamilyFull, error);
    }

    /// <summary>Refuse one. It leaves the list either way.</summary>
    public async Task<ApiError?> RejectAsync(long requestId, CancellationToken ct = default) =>
        (await api.RejectJoinRequest(requestId, ct).ConfigureAwait(false)).Error;

    /// <summary>
    /// Stop seeing a member, and apply it here at once: the frame that confirms it reaches this
    /// device too, and applying full state twice is applying it once.
    /// </summary>
    public async Task<ApiError?> BlockAsync(
        long userId, bool blocked, CancellationToken ct = default)
    {
        var answer = blocked
            ? await api.Block(userId, ct).ConfigureAwait(false)
            : await api.Unblock(userId, ct).ConfigureAwait(false);
        if (!answer.Ok)
        {
            return answer.Error;
        }
        chats.SetBlocked(userId, blocked);
        return null;
    }

    /// <summary>
    /// The numbers, exactly as the server answered them. The rows are NOT summed into the totals
    /// and never will be: the gap between them is this reader's own block list.
    /// </summary>
    public async Task<(StatsResponse? Stats, ApiError? Error)> StatsAsync(
        CancellationToken ct = default)
    {
        var answer = await api.Stats(ct).ConfigureAwait(false);
        return answer.Ok && answer.Value is not null
            ? (answer.Value, null)
            : (null, answer.Error ?? ApiError.Transport("no answer"));
    }

    /// <summary>
    /// How much of the family's messages this reader can see rows for — the honest way to say
    /// "and some members are hidden", rather than pretending the rows are the whole.
    /// </summary>
    public static bool RowsAreIncomplete(StatsResponse stats) =>
        (stats.Members ?? []).Length < stats.Totals.Members;
}
