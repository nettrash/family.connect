using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>Which screen the app is on, and there is exactly one answer at a time.</summary>
public enum Gate
{
    /// <summary>No server yet: first run.</summary>
    NoServer,

    /// <summary>A server, and nobody signed in.</summary>
    SignedOut,

    /// <summary>Signed in, in no family and with no request pending: the gate.</summary>
    NoFamily,

    /// <summary>A join request waiting for an owner to answer it.</summary>
    Pending,

    /// <summary>In a family — the one shown, when there are several.</summary>
    Member,

    /// <summary>In a family, and it is theirs — the one shown, when there are several.</summary>
    Owner,
}

/// <summary>Why the app left a signed-in state. Each one is a different sentence to the user.</summary>
public enum SessionEnd
{
    /// <summary>They asked to sign out.</summary>
    SignedOut,

    /// <summary>
    /// The session died server-side — a REST <c>401</c> or the socket's <c>4401</c>. Not the
    /// same thing as a wrong password, which never gets this far.
    /// </summary>
    Expired,

    /// <summary>The owner removed them, or the family dissolved.</summary>
    RemovedFromFamily,

    /// <summary>They were waiting for an owner, and the request is no longer pending.</summary>
    JoinRequestRejected,

    /// <summary>
    /// They deleted the account. Not "signed out" and not "expired": there is nothing to sign
    /// back into, and a screen offering to would be cruel as well as wrong.
    /// </summary>
    AccountDeleted,
}

/// <summary>What the window binds to: one snapshot, replaced whole.</summary>
public sealed record SessionState(
    Gate Gate,
    UserDto? Me = null,
    FamilyDto? Family = null,
    AssistantDto? Assistant = null,
    string? PendingFamilyName = null,
    bool CallsEnabled = false,
    bool VideoCallsEnabled = false,
    int MaxFamilyMembers = 0,
    string? SupportContact = null,
    bool FamilyRegistrationEnabled = true,
    int FamilylessAccountTtlDays = 0,
    bool GreetingsEnabled = false,
    /// <summary>
    /// When this member agreed that their words may go to the model, or null until they have
    /// (docs/protocol.md, "Consenting to the assistant"). The SERVER's answer and not this
    /// PC's: the server is what calls the model, a reinstall must not quietly re-ask and
    /// re-send, and somebody who agreed on their phone has agreed.
    /// </summary>
    string? AssistantConsentAt = null,
    bool JoinDeclined = false,
    /// <summary>
    /// When this member agreed that the assistant may send a query or place name it writes from their words to the
    /// lookup providers, or null until they have (docs/protocol.md, "Consenting to the assistant", amended 2026-10-03).
    /// The server's answer, like <see cref="AssistantConsentAt"/>, and never assumed: null is what an older server's
    /// silence reads as.
    /// </summary>
    string? AssistantLookupConsentAt = null,
    /// <summary>
    /// The video message's limits from <c>GET /families/mine</c>, or null on a server that predates them — which offers
    /// no way of recording one (docs/audio-video-messages-2026-10-04.md, S1.2's <b>round available</b>).
    /// </summary>
    RoundVideoLimits? RoundVideo = null,
    /// <summary>
    /// EVERY family the account is in, home family first, each with this account's role in it (docs/protocol.md, "Several
    /// families"). <see cref="Family"/> is the one of them being SHOWN — everything family-level on screen is its.
    /// </summary>
    IReadOnlyList<FamilyDto>? Families = null,
    /// <summary>The family being shown, by id; null while there is none.</summary>
    long? CurrentFamilyId = null,
    /// <summary>Every join request still waiting — while in a family too, to a further one.</summary>
    IReadOnlyList<PendingJoinDto>? PendingJoins = null,
    /// <summary>This account's own pending invitations into other families.</summary>
    IReadOnlyList<InvitationDto>? Invitations = null,
    /// <summary>The operator's switch for members starting another family in the app.</summary>
    bool MemberFamiliesEnabled = false,
    /// <summary>How many families one account may be in — never below one.</summary>
    int MaxFamiliesPerAccount = 1,
    /// <summary>
    /// Whether this server knows several families at all — its <c>GET /me</c> carried <c>families</c>. An older one knows
    /// only <c>/families/mine</c>, and is never asked for a family by id.
    /// </summary>
    bool NamesFamilies = false)
{
    /// <summary>Whether the socket may connect at all: signed in, and in a family.</summary>
    public bool CanChat => Gate is Gate.Member or Gate.Owner;

    /// <summary>Whether this account owns the family being shown.</summary>
    public bool IsOwner => Gate is Gate.Owner;

    /// <summary>The families, never null.</summary>
    public IReadOnlyList<FamilyDto> AllFamilies => Families ?? [];

    /// <summary>
    /// The HOME family — the first <c>GET /me</c> lists (docs/protocol.md, "Several families"), and the one family of a server
    /// that predates them — where a chat with no <c>family_ids</c> is listed (<see cref="FamilyRules.FamiliesOf"/>); null
    /// with no family.
    /// </summary>
    public long? HomeFamilyId => AllFamilies.FirstOrDefault()?.Id;

    /// <summary>The waiting join requests, never null.</summary>
    public IReadOnlyList<PendingJoinDto> AllPending => PendingJoins ?? [];

    /// <summary>The pending invitations to this account, never null.</summary>
    public IReadOnlyList<InvitationDto> AllInvitations => Invitations ?? [];
}

/// <summary>
/// Which family this device last SHOWED, per account — so a relaunch opens where the reader left off (docs/protocol.md,
/// "Several families": "a CURRENT family, persisted per account on the device").
/// </summary>
public interface ICurrentFamily
{
    long? For(long userId);

    void Remember(long userId, long familyId);
}

/// <summary>For a session that need not outlive its process — the tests.</summary>
public sealed class MemoryCurrentFamily : ICurrentFamily
{
    private readonly Dictionary<long, long> shown = [];

    public long? For(long userId) => shown.TryGetValue(userId, out var family) ? family : null;

    public void Remember(long userId, long familyId) => shown[userId] = familyId;
}

/// <summary>
/// The account, from the app's side: who is signed in, which screen that means, and the three
/// ways a signed-in state ends.
/// </summary>
/// <remarks>
/// <para>
/// <b>A WRONG PASSWORD IS NOT AN EXPIRED SESSION.</b> Both are <c>401</c>, and one of this app's
/// own clients read every one of them as "your session expired" — so a mistyped password told the
/// user they had been signed out of a session they were never in. The code decides:
/// <c>invalid_credentials</c> is an answer to the form in front of them, and
/// <c>session_expired</c> is the session going away underneath them.
/// </para>
/// <para>
/// <b>THE SESSION ENDS ONCE.</b> A dead session is discovered by whatever asks next, and on a
/// reconnect loop that is every few seconds. The guard is the token still being stored: the first
/// discovery clears it, and every later one finds nothing to end.
/// </para>
/// <para>
/// <b>A FLAKY NETWORK IS NOT A SIGN-OUT.</b> A transient failure on <c>GET /me</c> leaves the gate
/// exactly where it is and is reported, because the alternative is an app that drops to the
/// sign-in screen in a lift.
/// </para>
/// </remarks>
public sealed class AppSession(
    ApiClient api, ITokenStore tokens, Database cache, IAwaitingJoin? awaitingJoin = null, ICurrentFamily? currentFamily = null)
{
    private readonly IAwaitingJoin awaiting = awaitingJoin ?? new MemoryAwaitingJoin();
    private readonly ICurrentFamily remembered = currentFamily ?? new MemoryCurrentFamily();

    /// <summary>Whose cache has been told its home family (<see cref="Database.AdoptLegacy"/>) — once per account per launch.</summary>
    private long adoptedFor;

    /// <summary>
    /// A family appeared or went outside a list this session asked for — the router's word, or a <c>family_not_found</c>.
    /// The window drops what it holds of a family gone with its rows.
    /// </summary>
    public event Action<long>? FamilyDropped;

    // Signed out until `GET /me` says otherwise: a token in the locker is not a session, and a
    // window drawn from a stored token alone would show a chat list to somebody whose account
    // was deleted three weeks ago.
    private SessionState state = new(Gate.SignedOut);

    /// <summary>The current snapshot. Replaced whole, never edited in place.</summary>
    public SessionState State => state;

    /// <summary>The snapshot changed.</summary>
    public event Action<SessionState>? Changed;

    /// <summary>A signed-in state ended, and why. Raised once per ending.</summary>
    public event Action<SessionEnd>? Ended;

    /// <summary>Sign in. The answer is the refusal, or null when it worked.</summary>
    public Task<ApiError?> SignInAsync(
        string username, string password, CancellationToken ct = default) =>
        AuthenticateAsync(() => api.LogIn(username, password, ct), ct);

    /// <summary>Register, which signs in as the same act — the answer carries the token.</summary>
    public Task<ApiError?> RegisterAsync(
        string username, string displayName, string password, CancellationToken ct = default) =>
        AuthenticateAsync(() => api.Register(username, displayName, password, ct), ct);

    private async Task<ApiError?> AuthenticateAsync(
        Func<Task<ApiResult<AuthResponse>>> attempt, CancellationToken ct)
    {
        var answer = await attempt().ConfigureAwait(false);
        if (!answer.Ok || answer.Value is null)
        {
            // No wiping, no ending: nobody was signed in, and the user is looking at the form
            // this answers.
            return answer.Error ?? ApiError.Transport("no answer");
        }
        // A NEW ACCOUNT MAY NOT INHERIT THE LAST ONE'S CACHE. Signing in as somebody else on a
        // device that was signed in before must not leave that family's messages on screen —
        // and the outbox least of all (see Database.WipeAll).
        cache.WipeAll();
        tokens.Token = answer.Value.Token;
        return await RefreshAsync(ct).ConfigureAwait(false);
    }

    /// <summary>
    /// Read <c>GET /me</c> and put the app on the screen it says. The one call that decides the
    /// gate; everything else here is a consequence of it.
    /// </summary>
    public async Task<ApiError?> RefreshAsync(CancellationToken ct = default) => await RefreshAsync(ct, prefer: null).ConfigureAwait(false);

    /// <summary>
    /// <see cref="RefreshAsync(CancellationToken)"/>, SHOWING <paramref name="prefer"/> when the answer has it — a family just
    /// created, joined or accepted (docs/protocol.md, "Several families": "add it, switch to it").
    /// </summary>
    public async Task<ApiError?> RefreshAsync(CancellationToken ct, long? prefer)
    {
        if (tokens.Token is null)
        {
            Publish(new SessionState(Gate.SignedOut));
            return null;
        }
        var me = await api.Me(ct).ConfigureAwait(false);
        if (!me.Ok || me.Value is null)
        {
            var error = me.Error ?? ApiError.Transport("no answer");
            // The rule lives on the error: a 401 that is not `invalid_credentials`. A wrong
            // password cannot reach here anyway — this call carries a token, not a form — but
            // asking the same question in both places keeps one answer.
            if (error.SessionGone)
            {
                SessionGone();
                return error;
            }
            // Anything else — a dead network, a proxy, a 500 — changes nothing about who is
            // signed in.
            return error;
        }

        Apply(me.Value, prefer);
        return null;
    }

    /// <summary>
    /// What a <c>GET /me</c> answered, applied: which families there are, which one is shown, and the screen that means.
    /// The resync's step 1 lands here too, so a family that appeared or went while the socket was down is caught at the
    /// next pass — a removal it slept through raises no frame it will ever see.
    /// </summary>
    /// <param name="answered">The answer.</param>
    /// <param name="prefer">A family to SHOW if the answer has it — one just created, joined or accepted.</param>
    public void Apply(MeResponse answered, long? prefer = null)
    {
        ArgumentNullException.ThrowIfNull(answered);
        if (tokens.Token is null)
        {
            // A sign-out landed while the read was in flight: there is nobody to apply it to.
            return;
        }
        var was = state.Gate;
        var sameAccount = state.Me?.Id == answered.User.Id;
        var families = answered.AllFamilies;
        var pending = answered.AllPending;
        var shown = Shown(answered.User.Id, families, prefer);
        var current = shown is { } id ? families.First(family => family.Id == id) : null;
        var gate = (current, pending) switch
        {
            ({ } family, _) when family.IsOwner || (answered.Families is null && answered.IsOwner) => Gate.Owner,
            (not null, _) => Gate.Member,
            (null, { Count: > 0 }) => Gate.Pending,
            _ => Gate.NoFamily,
        };
        if (shown is { } chosen)
        {
            remembered.Remember(answered.User.Id, chosen);
            if (adoptedFor != answered.User.Id)
            {
                // A cache an older build filled was the HOME family's — the only family such a build ever saw.
                cache.AdoptLegacy(families[0].Id);
                adoptedFor = answered.User.Id;
            }
        }
        // `/families/{id}` only where the server knows it; an older one is asked for `mine`, which is all it has.
        api.FamilyScope = answered.Families is not null ? shown : null;
        // Declined is what a remembered wait becomes when `GET /me` shows no request and no family — said on the family door
        // until the next join or family replaces it (ios AppSession.joinDeclined).
        var waited = was is Gate.Pending || awaiting.UserId == answered.User.Id;
        var declined = gate is Gate.NoFamily && (waited || (was is Gate.NoFamily && state.JoinDeclined));
        awaiting.UserId = gate is Gate.Pending ? answered.User.Id : null;
        // A family that is no longer in the list was left or lost while nobody was looking: its rows go, and only its —
        // unless it was the LAST one, which takes everything (below).
        // What this cache holds counts too: a family left while the app was not running is in no list this session has seen.
        // (A cache never holds another account's families — signing in wipes it.)
        var gone = families.Count > 0
            ? (sameAccount ? state.AllFamilies.Select(family => family.Id) : [])
                .Union(cache.HeldFamilies())
                .Except(families.Select(family => family.Id))
                .ToList()
            : [];
        var sameFamily = state.CurrentFamilyId == shown;
        var next = new SessionState(
            gate,
            Me: answered.User,
            // The shown family's fuller document (its invite code, its switches) when this read is about the same one.
            Family: current is null ? null : sameFamily && state.Family?.Id == current.Id ? Merge(state.Family, current) : current,
            // CARRIED, NOT RE-READ. `GET /me` says nothing about the assistant — only
            // `GET /families/mine` does — so a refresh that leaves this out publishes a state
            // with no assistant in it, and this is a positional parameter with a null default,
            // so leaving it out COMPILES. What hung off it: the Assistant card in Settings and
            // the consent line over the composer, both of which vanished after any /me — which
            // includes the refresh that runs right after somebody agrees. Cleared with the
            // family, because an assistant belongs to one.
            Assistant: gate is Gate.Member or Gate.Owner ? state.Assistant : null,
            // The same for the video message's limits: only the family's own document carries them.
            RoundVideo: gate is Gate.Member or Gate.Owner ? state.RoundVideo : null,
            PendingFamilyName: pending.FirstOrDefault()?.FamilyName,
            CallsEnabled: answered.CallsEnabled,
            VideoCallsEnabled: answered.VideoCallsEnabled,
            MaxFamilyMembers: answered.MaxFamilyMembers,
            SupportContact: answered.SupportContact,
            FamilyRegistrationEnabled: answered.FamilyRegistrationEnabled,
            FamilylessAccountTtlDays: answered.FamilylessAccountTtlDays,
            GreetingsEnabled: answered.GreetingsEnabled,
            AssistantConsentAt: answered.AssistantConsentAt,
            JoinDeclined: declined,
            AssistantLookupConsentAt: answered.AssistantLookupConsentAt,
            Families: families,
            CurrentFamilyId: shown,
            PendingJoins: pending,
            Invitations: answered.Invitations ?? [],
            MemberFamiliesEnabled: answered.MemberFamiliesEnabled,
            MaxFamiliesPerAccount: answered.FamilyCap,
            NamesFamilies: answered.Families is not null);
        foreach (var family in gone)
        {
            cache.DropFamily(family);
        }
        // A pass's `GET /me` that changed nothing publishes nothing: every publish redraws every screen bound to this state.
        if (gone.Count == 0 && Wire.Encode(next) == Wire.Encode(state))
        {
            return;
        }
        Publish(next);
        foreach (var family in gone)
        {
            FamilyDropped?.Invoke(family);
        }

        // Two departures nothing else announces. `GET /me` is where a client finds out, because
        // a removal it slept through raises no frame it will ever see.
        if (was is Gate.Member or Gate.Owner && gate is Gate.NoFamily or Gate.Pending)
        {
            // Their family's chats and wall are not theirs to keep.
            cache.WipeAll();
            Ended?.Invoke(SessionEnd.RemovedFromFamily);
        }
        else if (waited && gate is Gate.NoFamily)
        {
            Ended?.Invoke(SessionEnd.JoinRequestRejected);
        }
    }

    /// <summary>
    /// Which family to show: one just asked for, else the one shown now, else the one this device last showed this account,
    /// else the first — the home family. Null with no family at all.
    /// </summary>
    private long? Shown(long userId, IReadOnlyList<FamilyDto> families, long? prefer)
    {
        bool Holds(long? id) => id is { } wanted && families.Any(family => family.Id == wanted);
        if (Holds(prefer))
        {
            return prefer;
        }
        if (state.Me?.Id == userId && Holds(state.CurrentFamilyId))
        {
            return state.CurrentFamilyId;
        }
        var last = remembered.For(userId);
        return Holds(last) ? last : families.FirstOrDefault()?.Id;
    }

    /// <summary>The family's fuller document kept, under what the list now says of it — its name, and the reader's role.</summary>
    private static FamilyDto Merge(FamilyDto held, FamilyDto listed) =>
        held with { Name = listed.Name, Role = listed.Role ?? held.Role };

    /// <summary>
    /// Show another of this account's families. Everything family-level on screen follows — the chat list, the board, the
    /// stickers, the roster, the owner's screens — and the family's own document is read for what only it carries (the
    /// assistant, the video message's limits, the invite code).
    /// </summary>
    public async Task<ApiError?> SwitchFamilyAsync(long familyId, CancellationToken ct = default)
    {
        if (!Switch(familyId))
        {
            return null;
        }
        return await RefreshFamilyAsync(ct).ConfigureAwait(false);
    }

    /// <summary>The switch itself, published at once; false when that family is not this account's or is already shown.</summary>
    public bool Switch(long familyId)
    {
        if (state.Me is not { } me || state.CurrentFamilyId == familyId
            || state.AllFamilies.FirstOrDefault(family => family.Id == familyId) is not { } family)
        {
            return false;
        }
        remembered.Remember(me.Id, familyId);
        if (state.NamesFamilies)
        {
            api.FamilyScope = familyId;
        }
        Publish(state with
        {
            Gate = family.IsOwner ? Gate.Owner : Gate.Member,
            Family = family,
            CurrentFamilyId = familyId,
        });
        return true;
    }

    /// <summary>
    /// The router says this account is in a family — re-read <c>GET /me</c> when it is one this session does not hold
    /// (docs/protocol.md, "Several families"), so the new family is listed; the resync that follows reads its rows.
    /// </summary>
    public async Task<bool> JoinedFamilyAsync(long familyId, CancellationToken ct = default)
    {
        if (state.AllFamilies.Any(family => family.Id == familyId))
        {
            return false;
        }
        await RefreshAsync(ct).ConfigureAwait(false);
        return true;
    }

    /// <summary>
    /// This account is out of a family — the router's <c>member_left</c> naming it, or <c>family_not_found</c> from a read
    /// of it: that family's rows go at once, and <c>GET /me</c> says what is left to show.
    /// </summary>
    public async Task FamilyGoneAsync(long familyId, CancellationToken ct = default)
    {
        if (state.AllFamilies.Count > 1)
        {
            cache.DropFamily(familyId);
            FamilyDropped?.Invoke(familyId);
        }
        await RefreshAsync(ct).ConfigureAwait(false);
    }

    /// <summary>An invitation frame: to this account it is listed (or replaced); one this account's family sent is not its.</summary>
    public void ApplyInvitation(InvitationDto invitation)
    {
        ArgumentNullException.ThrowIfNull(invitation);
        if (state.Me is not { } me || invitation.User.Id != me.Id)
        {
            return;
        }
        Publish(state with
        {
            Invitations = [.. state.AllInvitations.Where(held => held.Id != invitation.Id), invitation],
        });
    }

    /// <summary>An invitation lapsed — accepted, declined, withdrawn — and leaves the banner and the switcher.</summary>
    public void InvitationGone(long invitationId)
    {
        if (state.AllInvitations.All(held => held.Id != invitationId))
        {
            return;
        }
        Publish(state with { Invitations = [.. state.AllInvitations.Where(held => held.Id != invitationId)] });
    }

    /// <summary>
    /// Join: the family is added and SHOWN. <c>invitation_not_pending</c> drops the invitation — it is gone, whatever the
    /// reason — and comes back as the refusal for the window to say; <c>family_full</c> and <c>too_many_families</c> leave
    /// it standing.
    /// </summary>
    public async Task<ApiError?> AcceptInvitationAsync(long invitationId, CancellationToken ct = default)
    {
        var answer = await api.AcceptInvitation(invitationId, ct).ConfigureAwait(false);
        if (!answer.Ok || answer.Value is null)
        {
            var error = answer.Error ?? ApiError.Transport("no answer");
            if (error.Code == ErrorCodes.InvitationNotPending)
            {
                InvitationGone(invitationId);
            }
            return error;
        }
        InvitationGone(invitationId);
        return await RefreshAsync(ct, prefer: answer.Value.Family.Id).ConfigureAwait(false);
    }

    /// <summary>Decline: gone from the list whether the server still had it or not.</summary>
    public async Task<ApiError?> DeclineInvitationAsync(long invitationId, CancellationToken ct = default)
    {
        var answer = await api.DeclineInvitation(invitationId, ct).ConfigureAwait(false);
        if (!answer.Ok && answer.Error?.Code != ErrorCodes.InvitationNotPending)
        {
            return answer.Error ?? ApiError.Transport("no answer");
        }
        InvitationGone(invitationId);
        return null;
    }

    /// <summary>
    /// The family's own document, which is where the roster, the assistant and the board's mark
    /// live. Folded into the snapshot so the window has one thing to read.
    /// </summary>
    public async Task<ApiError?> RefreshFamilyAsync(CancellationToken ct = default)
    {
        var family = await api.Family(ct).ConfigureAwait(false);
        if (!family.Ok || family.Value is null)
        {
            var error = family.Error ?? ApiError.Transport("no answer");
            if (error.Code == ErrorCodes.FamilyNotFound)
            {
                // The family shown is gone — left elsewhere, removed, deleted (docs/protocol.md, "Several families"): `/me`
                // says what is left, and the gone family's rows go with it.
                await RefreshAsync(ct).ConfigureAwait(false);
            }
            return error;
        }
        if (!state.CanChat || (state.CurrentFamilyId is { } shown && family.Value.Family.Id != shown))
        {
            // The gate moved while this read was in flight — a sign-out, or a removal — or another family is shown now.
            // Publishing would hand the window a signed-out state carrying a family, or one family's switches on another.
            return null;
        }
        var role = state.Family?.Role;
        Publish(state with
        {
            Family = family.Value.Family with { Role = family.Value.Family.Role ?? role },
            Families = [.. state.AllFamilies.Select(held => held.Id == family.Value.Family.Id
                ? family.Value.Family with { Role = held.Role }
                : held)],
            Assistant = family.Value.Assistant,
            RoundVideo = RoundVideoLimits.Of(family.Value),
        });
        return null;
    }

    /// <summary>
    /// The family's assistant, as a pass over the wire found it — <see cref="Resync"/> reads the
    /// family's own document on every (re)connect, and this is what puts what it found where the
    /// window can see it.
    /// </summary>
    /// <remarks>
    /// Published only while there is a family to have one, and only when it CHANGES, because
    /// every publish redraws every screen bound to this state.
    /// </remarks>
    public void ApplyAssistant(AssistantDto? assistant)
    {
        if (!state.CanChat || state.Assistant == assistant)
        {
            return;
        }
        Publish(state with { Assistant = assistant });
    }

    /// <summary>
    /// The video message's limits, as a pass over the wire read them from the family's own document — published only
    /// while there is a family, and only when they change.
    /// </summary>
    public void ApplyRoundVideo(RoundVideoLimits? limits)
    {
        if (!state.CanChat || state.RoundVideo == limits)
        {
            return;
        }
        Publish(state with { RoundVideo = limits });
    }

    /// <summary>
    /// Sign out. The server is TOLD — which is what stops this device's notifications — but
    /// being unable to tell it changes nothing here: the token is this device's to forget.
    /// </summary>
    public async Task SignOutAsync(CancellationToken ct = default)
    {
        if (tokens.Token is null)
        {
            return;
        }
        try
        {
            await api.LogOut(ct).ConfigureAwait(false);
        }
        catch (Exception)
        {
            // A sign-out that fails on the network is still a sign-out.
        }
        Clear();
        Ended?.Invoke(SessionEnd.SignedOut);
    }

    /// <summary>
    /// Delete this account. The password is the proof, for the same reason a password CHANGE
    /// needs the current one — and when it is done there is nothing left to sign into.
    /// </summary>
    public async Task<ApiError?> DeleteAccountAsync(
        string password, CancellationToken ct = default)
    {
        var answer = await api.DeleteAccount(password, ct).ConfigureAwait(false);
        if (!answer.Ok)
        {
            // A WRONG PASSWORD HERE IS NOT AN EXPIRED SESSION either: the account is still
            // there, and so is the session.
            return answer.Error;
        }
        Clear();
        Ended?.Invoke(SessionEnd.AccountDeleted);
        return null;
    }

    /// <summary>
    /// Change the password. The current one is required because A LIVE SESSION IS NOT PROOF —
    /// and a refusal here says "that is not your password", never "you have been signed out".
    /// </summary>
    public async Task<ApiError?> ChangePasswordAsync(
        string currentPassword, string newPassword, CancellationToken ct = default) =>
        (await api.ChangePassword(currentPassword, newPassword, ct).ConfigureAwait(false)).Error;

    /// <summary>
    /// The session is gone server-side: a REST <c>401</c> with <c>session_expired</c>, or the
    /// socket closing with <c>4401</c>. Idempotent, because the reconnect loop will find out
    /// again every few seconds and the user needs telling once.
    /// </summary>
    public void SessionGone()
    {
        if (tokens.Token is null)
        {
            return;
        }
        Clear();
        Ended?.Invoke(SessionEnd.Expired);
    }

    private void Clear()
    {
        tokens.Token = null;
        // Nobody's family is shown: the next account names its own.
        api.FamilyScope = null;
        cache.WipeAll();
        Publish(new SessionState(Gate.SignedOut, SupportContact: state.SupportContact));
    }

    private void Publish(SessionState next)
    {
        state = next;
        Changed?.Invoke(next);
    }
}
