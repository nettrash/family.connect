using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>
/// The rules of one account in several families (docs/protocol.md, "Several families"; docs/families-2026-10-22.md) that
/// are not a screen: which chats a family shows, which family an upload or a notification is for, and which doors to a
/// further family are open.
/// </summary>
public static class FamilyRules
{
    /// <summary>
    /// The families <paramref name="chat"/> is listed in: its <c>family_ids</c> — or, for a chat with none (cached before them,
    /// or from a server that predates them, where absent "means the one family the account has": docs/protocol.md,
    /// <c>Chat</c>), the HOME family <paramref name="home"/> alone, as Android's <c>listedIn</c> and the web's
    /// <c>listed_in</c> read it. An explicit <c>[]</c> stays empty: a direct chat closed for the one who left is listed nowhere.
    /// </summary>
    public static long[] FamiliesOf(ChatDto chat, long? home) =>
        chat.FamilyIds ?? (home is { } only ? [only] : []);

    /// <summary>
    /// Whether <paramref name="chat"/> is shown in <paramref name="family"/>: the chats whose families (<see cref="FamiliesOf"/>)
    /// name it. A direct chat names every family the two share, so it is shown in each; a chat with no <c>family_ids</c> is
    /// shown in the home family only. A session with no family shown shows it.
    /// </summary>
    public static bool Lists(ChatDto chat, long? family, long? home) =>
        family is not { } shown || FamiliesOf(chat, home).Contains(shown);

    /// <summary>
    /// The family an upload for <paramref name="chat"/> is FOR: the one shown when the chat is listed there, else the first
    /// the chat is listed in — a message may use an attachment only in a chat its family lists, and a chat with no
    /// <c>family_ids</c> is the home family's — else the one shown.
    /// </summary>
    public static long? UploadFamily(ChatDto? chat, long? shown, long? home) =>
        chat is not null && FamiliesOf(chat, home) is { Length: > 0 } families && !(shown is { } current && families.Contains(current))
            ? families[0]
            : shown;

    /// <summary>
    /// The family to SHOW to open <paramref name="chat"/> — from a notification, say: the one shown when it lists the chat,
    /// else the first that does (docs/protocol.md, "Pushes": "in the family the app is showing when the chat is listed there,
    /// else in the first that lists it"). Null when nothing needs to change — and for a chat listed in NO family
    /// (<c>family_ids: []</c>, a direct chat closed when the last family the two shared went), which has no family to move
    /// to: it opens where the reader is, as Android's <c>familyForChat</c> leaves the app where it is.
    /// </summary>
    public static long? FamilyToOpen(ChatDto chat, long? shown, long? home)
    {
        if (Lists(chat, shown, home))
        {
            return null;
        }
        var families = FamiliesOf(chat, home);
        return families.Length > 0 ? families[0] : null;
    }

    /// <summary>
    /// The name of the family a chat belongs to — the first it is listed in, the home family (the account's first) for a chat
    /// with no <c>family_ids</c> — from the account's list, for a notification's title; null when the list does not name it.
    /// </summary>
    public static string? NameOf(ChatDto chat, IReadOnlyList<FamilyDto> families) =>
        FamiliesOf(chat, families.FirstOrDefault()?.Id) is { Length: > 0 } ids ? NameOf(ids[0], families) : null;

    /// <summary>One family's name from the account's list, or null.</summary>
    public static string? NameOf(long? familyId, IReadOnlyList<FamilyDto> families) =>
        familyId is { } id ? families.FirstOrDefault(family => family.Id == id)?.Name : null;

    /// <summary>
    /// Whether the switcher is drawn at all: two families or more, or a pending invitation or join request to show. An account
    /// in ONE family with nothing pending sees nothing of several families.
    /// </summary>
    public static bool ShowsSwitcher(SessionState state) =>
        state.AllFamilies.Count >= 2 || (state.AllFamilies.Count == 1 && (state.AllInvitations.Count > 0 || state.AllPending.Count > 0));

    /// <summary>
    /// "New Family…" (docs/protocol.md, <c>GET /me</c>): the operator lets members start families, the account owns none of
    /// its families — ownership that came by hand-off counts — and it is under the cap.
    /// </summary>
    public static bool OffersNewFamily(SessionState state) =>
        state.MemberFamiliesEnabled
        && state.AllFamilies.Count > 0
        && !state.AllFamilies.Any(family => family.IsOwner)
        && state.AllFamilies.Count < state.MaxFamiliesPerAccount;

    /// <summary>"Join Another Family…": while under the cap.</summary>
    public static bool OffersJoinAnother(SessionState state) =>
        state.AllFamilies.Count > 0 && state.AllFamilies.Count < state.MaxFamiliesPerAccount;

    /// <summary>
    /// Whether an owner is offered "Reset Password…" for a member (D7): only where the server said this owner owns every family
    /// the member is in (<c>password_reset: true</c>). A roster that does not say — a server from before several families,
    /// where an owner always could — keeps offering it; never for oneself, a former member or a deleted account.
    /// </summary>
    public static bool MayResetPassword(MemberDto member, long me, bool owner) =>
        owner && member.Id != me && !member.IsFormer && member.PasswordReset != false;

    /// <summary>"Invite…" is the owner's, and only while they have ANOTHER family to invite from.</summary>
    public static bool OffersInvite(SessionState state) => state.IsOwner && state.AllFamilies.Count >= 2;
}

/// <summary>What one row of the family switcher is.</summary>
public enum SwitcherRowKind
{
    /// <summary>A family the account is in: shows it.</summary>
    Family,

    /// <summary>A join request waiting for an owner: says so, and does nothing.</summary>
    Pending,

    /// <summary>An invitation to the account: Join or Decline.</summary>
    Invitation,

    /// <summary>"New Family…".</summary>
    NewFamily,

    /// <summary>"Join Another Family…".</summary>
    JoinAnother,
}

/// <summary>
/// One row of the switcher. <see cref="Id"/> is the family's for a family and a pending request, the invitation's for an
/// invitation, and null for the two doors.
/// </summary>
public sealed record SwitcherRow(
    SwitcherRowKind Kind,
    long? Id,
    string Title,
    string? Detail = null,
    int Unread = 0,
    bool Current = false,
    string? AccessibleName = null);

/// <summary>
/// The family switcher (docs/protocol.md, "Switching"; the brief's item 5): every family with its unread count, the pending
/// join requests and invitations as rows of their own, and the two doors to a further family when they are open.
/// </summary>
public static class FamilySwitcher
{
    /// <summary>
    /// A family's unread count: the unread of the chats LISTED in it. A direct chat counts in each family that lists it —
    /// what the reader would find unread there.
    /// </summary>
    public static int Unread(long familyId, IEnumerable<ChatRowDto> chats, long? home) =>
        chats.Where(row => FamilyRules.Lists(row.Chat, familyId, home)).Sum(row => row.UnreadCount);

    public static IReadOnlyList<SwitcherRow> Rows(SessionState state, IReadOnlyList<ChatRowDto> chats, IStringCatalog say)
    {
        var rows = new List<SwitcherRow>();
        foreach (var family in state.AllFamilies)
        {
            var unread = Unread(family.Id, chats, state.HomeFamilyId);
            rows.Add(new SwitcherRow(
                SwitcherRowKind.Family, family.Id, family.Name,
                Unread: unread,
                Current: family.Id == state.CurrentFamilyId,
                AccessibleName: say.Format("%@, %lld unread", family.Name, unread)));
        }
        foreach (var pending in state.AllPending)
        {
            rows.Add(new SwitcherRow(SwitcherRowKind.Pending, pending.FamilyId, pending.FamilyName, say.Get("Waiting for approval")));
        }
        foreach (var invitation in state.AllInvitations)
        {
            rows.Add(new SwitcherRow(
                SwitcherRowKind.Invitation, invitation.Id, invitation.FamilyName, InvitationText.Banner(invitation, say)));
        }
        if (FamilyRules.OffersNewFamily(state))
        {
            rows.Add(new SwitcherRow(SwitcherRowKind.NewFamily, null, say.Get("New Family…")));
        }
        if (FamilyRules.OffersJoinAnother(state))
        {
            rows.Add(new SwitcherRow(SwitcherRowKind.JoinAnother, null, say.Get("Join Another Family…")));
        }
        return rows;
    }
}

/// <summary>What the invitation banner and its rows say.</summary>
public static class InvitationText
{
    /// <summary>"Anna invited you to “The Smiths”" — the inviter's name, the family's.</summary>
    public static string Banner(InvitationDto invitation, IStringCatalog say) =>
        say.Format("%@ invited you to “%@”", Inviter(invitation, say), invitation.FamilyName);

    private static string Inviter(InvitationDto invitation, IStringCatalog say) =>
        invitation.InvitedBy switch
        {
            { Deleted: true } => say.Get("Deleted account"),
            { DisplayName: { Length: > 0 } name } => name,
            _ => say.Get("Someone"),
        };
}

/// <summary>The sentences several families add to the refusals a client has to say (docs/protocol.md, the error list).</summary>
public static class FamilySentences
{
    /// <summary>
    /// The several-families refusal, or null when the error is not one of them — the caller's own sentence then.
    /// <paramref name="cap"/> is <c>max_families_per_account</c>.
    /// </summary>
    public static string? For(ApiError error, int cap, IStringCatalog say) => error.Code switch
    {
        ErrorCodes.TooManyFamilies => say.Format("You've reached the most families one account can be in (%lld).", cap),
        ErrorCodes.AlreadyOwnAFamily => say.Get("You already own a family, so you can't start another."),
        ErrorCodes.MemberFamiliesDisabled => say.Get("This server doesn't let members start new families."),
        ErrorCodes.MemberInOtherFamilies =>
            say.Get("This member is also in a family you don't own, so only they can change their password."),
        ErrorCodes.InvitationNotPending => say.Get("This invitation is no longer available."),
        _ => null,
    };
}

/// <summary>
/// The doors to a FURTHER family from inside the app (docs/protocol.md, "Getting another family"): "New Family…" and
/// "Join Another Family…" — the existing create and join, which add the family and SHOW it.
/// </summary>
public static class AnotherFamily
{
    /// <summary>What "New Family…" says above the existing create screen's field and button.</summary>
    public static string Explanation(IStringCatalog say) =>
        say.Get("Start your own family. You'll be its owner and can invite members of your other families.");
}

/// <summary>One person an owner may invite into the family shown, and whether they already are.</summary>
public sealed record InviteCandidate(MemberDto Member, bool Invited);

/// <summary>
/// The owner's "Invite…" (docs/protocol.md, "Several families", invitations): the members of the owner's OTHER families who
/// are not in this one — from the rosters this device holds, and nobody else on the server — and the pending invitations.
/// </summary>
public sealed class InviteModel(ApiClient api, ChatStore chats)
{
    /// <summary>
    /// Who may be invited into <paramref name="state"/>'s shown family: present members of the account's other families,
    /// not the reader, not in the shown family now, each once, in name order — marked when an invitation is pending.
    /// </summary>
    public IReadOnlyList<InviteCandidate> Candidates(SessionState state, IReadOnlyList<InvitationDto> pending)
    {
        if (state.CurrentFamilyId is not { } shown || state.Me is not { } me)
        {
            return [];
        }
        var here = chats.MembersOf(shown).Where(member => !member.IsFormer).Select(member => member.Id).ToHashSet();
        var invited = pending.Select(invitation => invitation.User.Id).ToHashSet();
        return [.. state.AllFamilies
            .Where(family => family.Id != shown)
            .SelectMany(family => chats.MembersOf(family.Id))
            .Where(member => !member.IsFormer && member.Id != me.Id && !here.Contains(member.Id))
            .GroupBy(member => member.Id)
            .Select(same => same.First())
            .OrderBy(member => member.DisplayName, StringComparer.CurrentCultureIgnoreCase)
            .ThenBy(member => member.Id)
            .Select(member => new InviteCandidate(member, invited.Contains(member.Id)))];
    }

    /// <summary>The shown family's pending invitations, oldest first.</summary>
    public async Task<(IReadOnlyList<InvitationDto> Invitations, ApiError? Error)> PendingAsync(
        long familyId, CancellationToken ct = default)
    {
        var answer = await api.Invitations(familyId, ct).ConfigureAwait(false);
        return answer.Ok && answer.Value is not null
            ? (answer.Value.Invitations ?? [], null)
            : ([], answer.Error ?? ApiError.Transport("no answer"));
    }

    /// <summary>Invite one person; a <c>200</c> with the invitation already pending is success too.</summary>
    public async Task<(InvitationDto? Invitation, ApiError? Error)> InviteAsync(
        long familyId, long userId, CancellationToken ct = default)
    {
        var answer = await api.Invite(familyId, userId, ct).ConfigureAwait(false);
        return answer.Ok && answer.Value is not null
            ? (answer.Value.Invitation, null)
            : (null, answer.Error ?? ApiError.Transport("no answer"));
    }

    /// <summary>Withdraw one; idempotent.</summary>
    public async Task<ApiError?> WithdrawAsync(long familyId, long invitationId, CancellationToken ct = default) =>
        (await api.WithdrawInvitation(familyId, invitationId, ct).ConfigureAwait(false)).Error;
}
