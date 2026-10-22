using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>
/// The protocol's four kinds of chat, and the questions the window asks of a kind (docs/protocol.md, "Objects" and
/// "Groups"). Every "is this the family chat?" that a group now shares lives here, once, so a fifth place cannot forget it.
/// </summary>
/// <remarks>
/// <para>
/// <b>A GROUP IS THE FAMILY CHAT WITH FEWER PEOPLE</b> (docs/protocol.md, "Groups"): sender names, threads, polls, member
/// mentions, the assistant by <c>@ai</c>, video messages — and no "seen", because it has many readers. What it does NOT share
/// with the family chat is the transcript rule, which follows the DIRECT chat (<see cref="Transcripts.MayAsk"/>), and calls,
/// which live in a direct chat alone.
/// </para>
/// </remarks>
public static class ChatKinds
{
    public const string Family = "family";
    public const string Direct = "direct";
    public const string Assistant = "ai";
    public const string Group = "group";

    public static bool IsGroup(string? kind) => kind == Group;

    /// <summary>
    /// A room with more than one other person in it — the family chat or a group: names over bubbles, member mentions,
    /// polls, <c>@ai</c>, and no "seen".
    /// </summary>
    public static bool IsShared(string? kind) => kind is Family or Group;

    /// <summary>Where the video button is offered (S1.4): a family, direct or group chat — never the assistant's.</summary>
    public static bool TakesVideoMessages(string? kind) => kind is Family or Direct or Group;
}

/// <summary>What a chat's circle shows: the family's house, a group's people, or a name's initials (and a picture over them).</summary>
public enum ChatFace
{
    /// <summary>The family chat: a house.</summary>
    House,

    /// <summary>A group: the "people" glyph (docs/groups-2026-10-21.md, §6 — "the initials of the name, or a people glyph").</summary>
    People,

    /// <summary>A direct chat or the assistant's: the initials of the title, with the peer's picture over them when there is one.</summary>
    Initials,
}

/// <summary>Something a member may do to a group (docs/protocol.md, "Groups", "Who may do what").</summary>
public enum GroupAction
{
    Rename,
    AddMembers,
    RemoveMember,
    Delete,
    Leave,
}

/// <summary>One member of a group as its info draws them: in the order they joined, the admin marked.</summary>
/// <param name="Removable">The reader is the admin and this is somebody else: "Remove from Group" is offered for them.</param>
public sealed record GroupMember(long UserId, string Name, bool Admin, bool Me, bool Removable);

/// <summary>
/// Who may do what to a group, what a name may be, and whom the lists offer (docs/protocol.md, "Groups") — the one table
/// every client keeps, pinned by <c>GroupRulesTests</c>.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE ADMIN IS ONE MEMBER</b>, the one who created it until they leave it: they rename, add, remove and delete. Anybody
/// in it may leave. Nobody else may do anything, the family owner included — they cannot even see a group they are not in.
/// The server answers <c>not_group_admin</c> to the rest, and a client that offered the button anyway would be offering a
/// refusal.
/// </para>
/// <para>
/// <b>THE NAME IS TRIMMED AND THEN 1 TO 64 CHARACTERS</b>, counted as the server counts — scalar values, the way a pack
/// label is (<see cref="PackLabel"/>) — so thirty-three emoji that the server takes are not refused here.
/// </para>
/// </remarks>
public static class GroupRules
{
    /// <summary>The most scalar values a group's name may be, after trimming.</summary>
    public const int MaxNameLength = 64;

    /// <summary>The most groups one family may have; the 51st is <c>too_many_groups</c>.</summary>
    public const int MaxGroups = 50;

    /// <summary>Whether the reader may do this to this chat. Nothing at all on a chat that is not a group.</summary>
    public static bool May(GroupAction action, ChatDto chat, long me)
    {
        ArgumentNullException.ThrowIfNull(chat);
        if (!ChatKinds.IsGroup(chat.Kind) || me <= 0)
        {
            return false;
        }
        // Holding a group is being in it: the server lists it to its members alone.
        return action == GroupAction.Leave || chat.AdminUserId == me;
    }

    /// <summary>Whether the reader may take this member out: the admin, somebody else, and somebody who is in it.</summary>
    public static bool MayRemove(ChatDto chat, long me, long member) =>
        May(GroupAction.RemoveMember, chat, me) && member != me && chat.MemberIds?.Contains(member) == true;

    /// <summary>The name as it is sent: trimmed, or null when it is not 1 to 64 characters once trimmed.</summary>
    public static string? Name(string? typed) =>
        typed?.Trim() is { Length: > 0 } name && name.EnumerateRunes().Count() <= MaxNameLength ? name : null;

    /// <summary>
    /// The members a NEW group can be made with: the live roster, never the reader (who is added anyway) and never the
    /// assistant — "never anybody outside the family, never somebody who has left, never a deleted account".
    /// </summary>
    public static IReadOnlyList<MemberDto> Choosable(IReadOnlyList<MemberDto> roster, long me, long? assistantId) =>
        [.. ComposerMentions.Live(roster).Where(member => member.Id != me && member.Id != assistantId)];

    /// <summary>
    /// The members "Add Members" offers: everyone <see cref="Choosable"/> who is not in the group already. Blocks change
    /// nothing here, in either direction: refusing an add would tell the adder about a block they are not party to.
    /// </summary>
    public static IReadOnlyList<MemberDto> Addable(ChatDto chat, IReadOnlyList<MemberDto> roster, long me, long? assistantId)
    {
        ArgumentNullException.ThrowIfNull(chat);
        var inIt = (chat.MemberIds ?? []).ToHashSet();
        return [.. Choosable(roster, me, assistantId).Where(member => !inIt.Contains(member.Id))];
    }

    /// <summary>
    /// The member list of the group's info, in the order they joined (the server's order). A member the roster cannot name
    /// — one who joined the family while this device slept — is "Someone" until the roster is read.
    /// </summary>
    public static IReadOnlyList<GroupMember> Members(ChatDto chat, IReadOnlyList<MemberDto> roster, long me, IStringCatalog say)
    {
        ArgumentNullException.ThrowIfNull(chat);
        ArgumentNullException.ThrowIfNull(say);
        var byId = roster.GroupBy(member => member.Id).ToDictionary(group => group.Key, group => group.First());
        return
        [
            .. (chat.MemberIds ?? []).Distinct().Select(id => new GroupMember(
                id,
                byId.GetValueOrDefault(id) switch
                {
                    { Deleted: true } => say.Get("Deleted account"),
                    { DisplayName: { Length: > 0 } name } => name,
                    _ => say.Get("Someone"),
                },
                Admin: chat.AdminUserId == id,
                Me: id == me,
                Removable: MayRemove(chat, me, id))),
        ];
    }

    /// <summary>
    /// What stops a group being created, said before anything is sent: the name first, then nobody chosen. Null when it may
    /// be asked for.
    /// </summary>
    public static string? CreateProblem(string? typedName, IReadOnlyCollection<long> chosen, long me, IStringCatalog say)
    {
        ArgumentNullException.ThrowIfNull(chosen);
        if (Name(typedName) is null)
        {
            return GroupText.NameProblem(say);
        }
        return chosen.Any(id => id != me) ? null : GroupText.NobodyChosen(say);
    }
}

/// <summary>Every sentence about a group, from the apps' catalogue.</summary>
public static class GroupText
{
    public static string NameProblem(IStringCatalog say) => say.Get("A group's name can be 1 to 64 characters.");

    public static string NobodyChosen(IStringCatalog say) => say.Get("Choose at least one member.");

    public static string EveryoneIsIn(IStringCatalog say) => say.Get("Everyone in the family is already in this group.");

    /// <summary>"Members: 3" — the count over the group's info.</summary>
    public static string Count(ChatDto chat, IStringCatalog say) =>
        say.Format("Members: %lld", (chat.MemberIds ?? []).Distinct().Count());

    /// <summary>Asked before the admin deletes it: final, and for everyone (docs/protocol.md, "Groups").</summary>
    public static string DeleteQuestion(string name, IStringCatalog say) =>
        say.Format("Delete “%@”? Its messages are deleted for everyone.", name);

    /// <summary>Asked before anybody leaves: a group is joined by being added, so coming back is the admin's to do.</summary>
    public static string LeaveQuestion(string name, IStringCatalog say) =>
        say.Format("Leave “%@”? You can come back only if its admin adds you again.", name);

    /// <summary>
    /// A refusal in the reader's words. The two codes groups added are SAID — the limit is the family's, and the admin is
    /// somebody the reader can ask — and anything else is the plain failure of what was being done.
    /// </summary>
    /// <param name="creating">The refusal answers "Create Group" rather than a change to a group that exists.</param>
    public static string Failure(ApiError error, bool creating, IStringCatalog say)
    {
        ArgumentNullException.ThrowIfNull(error);
        return error.Code switch
        {
            ErrorCodes.NotGroupAdmin => say.Get("Only the group's admin can do this."),
            ErrorCodes.TooManyGroups => say.Get("Your family already has 50 groups."),
            _ => creating ? say.Get("Couldn't create the group.") : say.Get("Couldn't change the group."),
        };
    }
}

/// <summary>
/// Making and managing a group: each call asks the server and writes what it answered into the cache at once — the
/// <c>chat_updated</c> or <c>chat_removed</c> frame that confirms it reaches this device too, and applying the same group
/// twice is applying it once (docs/protocol.md, "Groups").
/// </summary>
public sealed class GroupModel(ApiClient api, ChatStore chats)
{
    /// <summary>A new group with the reader as its admin; answers its id, or the refusal.</summary>
    public async Task<(long? ChatId, ApiError? Error)> CreateAsync(
        string typedName, IReadOnlyList<long> memberIds, CancellationToken ct = default)
    {
        ArgumentNullException.ThrowIfNull(memberIds);
        if (GroupRules.Name(typedName) is not { } name)
        {
            return (null, new ApiError(ErrorCodes.Validation, "a group's name is 1 to 64 characters", 400));
        }
        var answer = await api.CreateGroup(name, [.. memberIds.Distinct()], ct).ConfigureAwait(false);
        if (!answer.Ok || answer.Value is not { } created)
        {
            return (null, answer.Error ?? ApiError.Transport("no answer"));
        }
        chats.Upsert(created.Chat);
        return (created.Chat.Id, null);
    }

    public async Task<ApiError?> RenameAsync(long chatId, string typedName, CancellationToken ct = default)
    {
        if (GroupRules.Name(typedName) is not { } name)
        {
            return new ApiError(ErrorCodes.Validation, "a group's name is 1 to 64 characters", 400);
        }
        return Kept(await api.RenameGroup(chatId, name, ct).ConfigureAwait(false));
    }

    public async Task<ApiError?> AddAsync(long chatId, IReadOnlyList<long> userIds, CancellationToken ct = default) =>
        Kept(await api.AddGroupMembers(chatId, userIds, ct).ConfigureAwait(false));

    /// <summary>The admin taking somebody else out. The reader's own way out is <see cref="LeaveAsync"/>.</summary>
    public async Task<ApiError?> RemoveAsync(long chatId, long userId, CancellationToken ct = default)
    {
        var answer = await api.RemoveGroupMember(chatId, userId, ct).ConfigureAwait(false);
        if (!answer.Ok)
        {
            return answer.Error ?? ApiError.Transport("no answer");
        }
        if (answer.Value?.Chat is { } chat)
        {
            chats.Upsert(chat);
        }
        return null;
    }

    /// <summary>
    /// The reader leaves: the server answers <c>204</c>, hands the group on if they were its admin, and deletes it if they
    /// were the last one in. Here it goes, its messages with it.
    /// </summary>
    public async Task<ApiError?> LeaveAsync(long chatId, CancellationToken ct = default) =>
        Gone(chatId, await api.RemoveGroupMember(chatId, chats.Reader, ct).ConfigureAwait(false));

    /// <summary>The admin deletes it, for everyone; here it goes at once.</summary>
    public async Task<ApiError?> DeleteAsync(long chatId, CancellationToken ct = default) =>
        Gone(chatId, await api.DeleteGroup(chatId, ct).ConfigureAwait(false));

    private ApiError? Kept(ApiResult<ChatResponse> answer)
    {
        if (!answer.Ok || answer.Value is not { } kept)
        {
            return answer.Error ?? ApiError.Transport("no answer");
        }
        chats.Upsert(kept.Chat);
        return null;
    }

    /// <summary>
    /// A group the reader is out of. <c>chat_not_found</c> is the same outcome reached first by somebody else — the admin
    /// deleted it, or it was left by its last other member — so it is dropped here all the same and nothing is said.
    /// </summary>
    private ApiError? Gone<T>(long chatId, ApiResult<T> answer)
    {
        if (!answer.Ok && answer.Error?.Code != ErrorCodes.ChatNotFound)
        {
            return answer.Error ?? ApiError.Transport("no answer");
        }
        chats.Remove(chatId);
        return null;
    }
}
