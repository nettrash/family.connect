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

    /// <summary>In a family.</summary>
    Member,

    /// <summary>In a family, and it is theirs.</summary>
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
    bool GreetingsEnabled = false)
{
    /// <summary>Whether the socket may connect at all: signed in, and in a family.</summary>
    public bool CanChat => Gate is Gate.Member or Gate.Owner;

    public bool IsOwner => Gate is Gate.Owner;
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
public sealed class AppSession(ApiClient api, ITokenStore tokens, Database cache)
{
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
    public async Task<ApiError?> RefreshAsync(CancellationToken ct = default)
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

        var was = state.Gate;
        var answered = me.Value;
        var gate = answered switch
        {
            { Family: not null } when answered.IsOwner => Gate.Owner,
            { Family: not null } => Gate.Member,
            { PendingJoinRequest: not null } => Gate.Pending,
            _ => Gate.NoFamily,
        };
        Publish(new SessionState(
            gate,
            Me: answered.User,
            Family: answered.Family,
            PendingFamilyName: answered.PendingJoinRequest?.FamilyName,
            CallsEnabled: answered.CallsEnabled,
            VideoCallsEnabled: answered.VideoCallsEnabled,
            MaxFamilyMembers: answered.MaxFamilyMembers,
            SupportContact: answered.SupportContact,
            FamilyRegistrationEnabled: answered.FamilyRegistrationEnabled,
            FamilylessAccountTtlDays: answered.FamilylessAccountTtlDays,
            GreetingsEnabled: answered.GreetingsEnabled));

        // Two departures nothing else announces. `GET /me` is where a client finds out, because
        // a removal it slept through raises no frame it will ever see.
        if (was is Gate.Member or Gate.Owner && gate is Gate.NoFamily or Gate.Pending)
        {
            // Their family's chats and wall are not theirs to keep.
            cache.WipeAll();
            Ended?.Invoke(SessionEnd.RemovedFromFamily);
        }
        else if (was is Gate.Pending && gate is Gate.NoFamily)
        {
            Ended?.Invoke(SessionEnd.JoinRequestRejected);
        }
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
            return family.Error ?? ApiError.Transport("no answer");
        }
        Publish(state with
        {
            Family = family.Value.Family,
            Assistant = family.Value.Assistant,
        });
        return null;
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
        cache.WipeAll();
        Publish(new SessionState(Gate.SignedOut, SupportContact: state.SupportContact));
    }

    private void Publish(SessionState next)
    {
        state = next;
        Changed?.Invoke(next);
    }
}
