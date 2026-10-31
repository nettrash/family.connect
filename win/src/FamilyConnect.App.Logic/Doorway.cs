using System.Globalization;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>
/// The first screen: which server. What counts as an address, and what counts as a Family Connect
/// server answering at it.
/// </summary>
/// <remarks>
/// <para>
/// <b>"CONNECT" ASKS THE SERVER, NOT THE ADDRESS.</b> A typo'd host, a router's admin page and a
/// random website all "answer", so the check is the one request whose refusal only this protocol
/// writes: <c>GET /me</c> with no token, answered <c>401</c> with a code the protocol names. A web
/// server behind a login page answers 401 too — with HTML, which parses to no code at all, so it
/// is not mistaken for a family's server. The same probe as the Apple client's.
/// </para>
/// <para>
/// <b>PLAIN HTTP IS ALLOWED AND SAID.</b> A family on a home network with no certificate types
/// <c>http://</c> and means it; Windows has no transport policy that would refuse it later, so the
/// only honest thing is the warning the other clients show.
/// </para>
/// </remarks>
public static class ServerCheck
{
    /// <summary>What people type, as a base URL — or null when there is no host in it.</summary>
    public static Uri? Parse(string? typed) => ServerUrl.Normalise(typed);

    /// <summary>Whether the traffic to this server can be read by anyone on the same network.</summary>
    public static bool IsPlainText(Uri server) => server.Scheme == Uri.UriSchemeHttp;

    /// <summary>Whether a Family Connect server answers at <paramref name="server"/>.</summary>
    public static async Task<bool> AnswersAsync(
        HttpClient http, Uri server, CancellationToken ct = default)
    {
        var me = await new ApiClient(http, server, new MemoryTokenStore())
            .Me(ct).ConfigureAwait(false);
        return me.Error is { Status: 401, Canonical: true };
    }
}

/// <summary>
/// The sentence each refusal at the door becomes. Keyed on the protocol's machine codes so the copy
/// can be specific, and worded exactly as the Apple client words it — the catalogue is shared, and
/// a sentence that differed would be a sentence nobody had translated.
/// </summary>
/// <remarks>
/// <b>A SERVER'S OWN MESSAGE IS SHOWN ONLY WHEN IT CAME WITH A CODE.</b> <c>validation</c> names the
/// rule that was broken, and that is worth reading. A proxy's reason phrase ("Bad Request") is not,
/// and a body the client could not read carries nothing but that.
/// </remarks>
public static class DoorSentences
{
    /// <summary>Signing in, or registering.</summary>
    public static string SignIn(ApiError error, bool registering, IStringCatalog say) => error switch
    {
        { Code: ErrorCodes.UsernameTaken } => say.Get("That username is taken."),
        // A wrong password is 401 with `invalid_credentials` — an answer to the form in front of
        // them, not a session that ended.
        { Status: 401 } when !registering => say.Get("Wrong username or password."),
        { Status: 401 } => say.Get("The server rejected the request. Try again."),
        { Code: ErrorCodes.Transport } => say.Get("Can't reach the server. Check your connection."),
        { Status: 429 } => say.Get("Try again in a moment."),
        { Status: >= 500 } => say.Get("The server had a problem. Try again in a moment."),
        { Status: 400 or 409 or 422 } => Said(error) ?? say.Get("The server rejected the request."),
        _ => say.Get("Something went wrong. Try again."),
    };

    /// <summary>Joining with an invite code.</summary>
    /// <param name="cap">The account's <c>max_families_per_account</c>, for <c>too_many_families</c> (docs/protocol.md, "Several families").</param>
    /// <param name="another">Joining from inside a family ("Join Another Family…"), where <c>already_in_family</c> means THAT family.</param>
    public static string Join(ApiError error, IStringCatalog say, int cap = 1, bool another = false) => error switch
    {
        { Code: ErrorCodes.TooManyFamilies } => FamilySentences.For(error, cap, say)!,
        { Code: ErrorCodes.AlreadyInFamily } when another => say.Get("You're already in that family."),
        // A CLOSED family answers this too, byte-identical to a code that never existed — so the
        // sentence is the same, and it is right about both.
        { Code: ErrorCodes.InvalidInviteCode } =>
            say.Get("That code doesn't match any family. Check it and try again."),
        { Status: 404 } => say.Get("That code doesn't work anymore."),
        { Code: ErrorCodes.AlreadyInFamily } => say.Get("You're already in a family."),
        { Code: ErrorCodes.JoinRequestPending } => say.Get("You already have a pending request."),
        // The one thing this endpoint admits to a stranger, on purpose: telling an invited member
        // their code is invalid on the day the family filled up costs a real person a real join.
        { Code: ErrorCodes.FamilyFull } =>
            say.Get("That family is full right now. Ask them to make room, then try the code again."),
        { Status: 400 or 409 } => Said(error) ?? say.Get("The server rejected that code."),
        _ => say.Get("Can't reach the server. Try again."),
    };

    /// <summary>Starting a family.</summary>
    /// <param name="cap">The account's <c>max_families_per_account</c>, for <c>too_many_families</c>.</param>
    public static string Create(ApiError error, IStringCatalog say, int cap = 1) => error switch
    {
        { Code: ErrorCodes.FamilyRegistrationDisabled } => say.Get("This server doesn't take new families."),
        // Starting ANOTHER family from inside one (docs/protocol.md, "Several families"): the three doors it can meet shut.
        { Code: ErrorCodes.TooManyFamilies or ErrorCodes.AlreadyOwnAFamily or ErrorCodes.MemberFamiliesDisabled } =>
            FamilySentences.For(error, cap, say)!,
        { Code: ErrorCodes.AlreadyInFamily } => say.Get("You're already in a family."),
        // A server that charges for a new family, and an account without an active subscription (docs/protocol.md, "Paid
        // families"): checked after every refusal above, so a shut door still says its own sentence.
        { Code: ErrorCodes.SubscriptionRequired } => say.Get("A new family on this server needs a subscription."),
        { Status: 400 or 409 } => Said(error) ?? say.Get("The server rejected that name."),
        _ => say.Get("Can't reach the server. Try again."),
    };

    private static string? Said(ApiError error) =>
        error.Canonical && error.Message is { Length: > 0 } message ? message : null;
}

/// <summary>
/// The family door for somebody signed in and in no family: join one with a code, or start one.
/// </summary>
/// <remarks>
/// <b>THE SCREEN IS DECIDED BY <c>GET /me</c>, NOT BY THE ANSWER.</b> A join answers <c>joined</c>
/// or <c>pending</c> and a create answers the family, but which screen that means — the chats, or
/// waiting for an owner — is the one decision <see cref="AppSession.RefreshAsync"/> makes, and
/// making it twice is how two screens come to disagree.
/// </remarks>
public sealed class FamilyDoor(ApiClient api, AppSession session, IStringCatalog? words = null)
{
    private readonly IStringCatalog say = words ?? EnglishCatalog.Instance;

    /// <summary>The longest family name the server takes.</summary>
    public const int NameLimit = 64;

    public static bool MayJoin(string? code) => !string.IsNullOrWhiteSpace(code);

    public static bool MayCreate(string? name) =>
        name?.Trim().Length is >= 1 and <= NameLimit;

    /// <summary>Join. The answer is the sentence to show, or null when it worked.</summary>
    /// <remarks>
    /// From inside a family too — "Join Another Family…" (docs/protocol.md, "Several families"): <c>joined</c> adds the family
    /// and SHOWS it; <c>pending</c> leaves the family shown where it is, with a waiting row in the switcher.
    /// </remarks>
    public async Task<string?> JoinAsync(string code, CancellationToken ct = default)
    {
        var before = session.State.AllFamilies.Select(family => family.Id).ToHashSet();
        var answer = await api.JoinFamily(code.Trim(), ct).ConfigureAwait(false);
        if (!answer.Ok || answer.Value is null)
        {
            return DoorSentences.Join(
                answer.Error ?? ApiError.Transport("no answer"), say, session.State.MaxFamiliesPerAccount, another: before.Count > 0);
        }
        // The answer names the family (2026-10-22): joined, it is the one to show. A server that names none leaves the
        // comparison — the family that was not in `/me` before is it.
        if (answer.Value is { Joined: true, FamilyId: { } named })
        {
            await session.RefreshAsync(ct, prefer: named).ConfigureAwait(false);
            return null;
        }
        await session.RefreshAsync(ct).ConfigureAwait(false);
        if (answer.Value.Joined && before.Count > 0
            && session.State.AllFamilies.FirstOrDefault(family => !before.Contains(family.Id)) is { } joined)
        {
            await session.SwitchFamilyAsync(joined.Id, ct).ConfigureAwait(false);
        }
        return null;
    }

    /// <summary>Start a family. The answer is the sentence to show, or null when it worked.</summary>
    /// <remarks>From inside a family too — "New Family…": the creator becomes its owner, and the family is added and SHOWN.</remarks>
    public async Task<string?> CreateAsync(string name, CancellationToken ct = default)
    {
        var answer = await api.CreateFamily(name.Trim(), ct).ConfigureAwait(false);
        if (!answer.Ok || answer.Value is null)
        {
            return DoorSentences.Create(answer.Error ?? ApiError.Transport("no answer"), say, session.State.MaxFamiliesPerAccount);
        }
        await session.RefreshAsync(ct, prefer: answer.Value.Family.Id).ConfigureAwait(false);
        return null;
    }
}

/// <summary>The sign-in form's one rule of its own: nothing is sent while a field it needs is empty.</summary>
/// <remarks>
/// The server enforces the real rules (a username of 3–32 letters, digits, dots or underscores; a
/// password of at least 8; a display name of 1–64) and names the one that was broken. Checking
/// them here as well would put a second, drifting copy of them in front of the server's own words.
/// </remarks>
public static class SignInForm
{
    public static bool MaySubmit(string? username, string? password, string? displayName, bool registering) =>
        !string.IsNullOrWhiteSpace(username)
        && !string.IsNullOrEmpty(password)
        && (!registering || !string.IsNullOrWhiteSpace(displayName));
}

/// <summary>The time on a chat row, in the reader's language and the reader's zone.</summary>
public static class RowTimeText
{
    public static string Format(
        RowTimeKind kind, DateTimeOffset? at, CultureInfo culture, IStringCatalog say) =>
        (kind, at) switch
        {
            (RowTimeKind.Clock, { } when) => when.ToLocalTime().ToString("t", culture),
            (RowTimeKind.Yesterday, _) => say.Get("Yesterday"),
            (RowTimeKind.Weekday, { } when) => when.ToLocalTime().ToString("dddd", culture),
            (RowTimeKind.Date, { } when) => when.ToLocalTime().ToString("d", culture),
            _ => string.Empty,
        };
}

/// <summary>What one bubble says: who, the words, and when.</summary>
public static class BubbleText
{
    /// <summary>
    /// The name over the words. "You" for this reader's own, "Deleted account" for an author whose
    /// account is gone — a name this client supplies, because the server keeps the row and not the
    /// person — and the roster's name otherwise.
    /// </summary>
    public static string Sender(Bubble bubble, ChatStore chats, IStringCatalog say)
    {
        if (bubble.Mine)
        {
            return say.Get("You");
        }
        return chats.Member(bubble.Message.SenderId) switch
        {
            { Deleted: true } => say.Get("Deleted account"),
            { DisplayName: { Length: > 0 } name } => name,
            _ => string.Empty,
        };
    }

    /// <summary>
    /// The words. A hidden bubble says that something was said and not what; a call reads as a
    /// call and a caption-less attachment as what it is — the same line the chat list draws — and
    /// everything else is the WHOLE body, where the list shows only its first line.
    /// </summary>
    public static string Words(Bubble bubble, ChatListModel list, IStringCatalog say)
    {
        if (!bubble.Reads)
        {
            return say.Get("Hidden — blocked member");
        }
        var message = bubble.Message;
        return message.Call is null && message.Body.Length > 0
            ? message.Body
            : list.Preview(message, hidden: false);
    }

    /// <summary>The clock under the words, and "edited" beside it once the body has changed.</summary>
    public static string When(MessageDto message, CultureInfo culture, IStringCatalog say)
    {
        var clock = Times.Instant(message.CreatedAt) is { } milliseconds
            ? DateTimeOffset.FromUnixTimeMilliseconds(milliseconds).ToLocalTime().ToString("t", culture)
            : string.Empty;
        return message.EditedAt is null ? clock : $"{clock} · {say.Get("edited")}";
    }
}
