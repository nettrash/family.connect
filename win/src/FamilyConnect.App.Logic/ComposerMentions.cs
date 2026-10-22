using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>One stretch of a text: plain words, or a name — and whether that name opens a chat.</summary>
public sealed record NameRun(string Text, long? UserId, bool Opens);

/// <summary>
/// One line of the strip over the composer: a member, or — <see cref="Member"/> null — the assistant. <see cref="Name"/> is
/// what it is drawn as, <see cref="Handle"/> the quieter words after it, and <see cref="Writes"/> what a pick puts after the
/// <c>@</c>: a member's display name, and the assistant's <c>ai</c>, never its display name.
/// </summary>
public sealed record MentionChoice(MemberDto? Member, string Name, string Handle, string Writes);

/// <summary>
/// Mentioning a member from the composer (docs/protocol.md, "Mentioning a member") — the web client's
/// composer rules: who the strip offers, what a send carries, and whom a name in a bubble opens.
/// </summary>
/// <remarks>
/// <para>
/// <b>ONLY IN THE FAMILY CHAT OR A GROUP</b>, where a mention means anything, and never while a message is being edited.
/// In a group the names are THAT GROUP's members alone (docs/protocol.md, "Mentioning a member", amended 2026-10-22): the
/// server refuses anybody else, who would not even receive the message — so every rule here takes the chat's
/// <see cref="Candidates"/> rather than the roster.
/// </para>
/// <para>
/// <b>THE NAMES ARE RESOLVED FROM THE TEXT AT SEND</b>: a name typed by hand mentions too, a name deleted after
/// being picked does not. The strip only helps write the whole name.
/// </para>
/// </remarks>
public static class ComposerMentions
{
    /// <summary>The members a name may name: the live roster — never a member who has left or been deleted.</summary>
    public static IReadOnlyList<MemberDto> Live(IReadOnlyList<MemberDto> members) =>
        [.. members.Where(member => !member.Deleted && !member.IsFormer)];

    /// <summary>Whether a mention means anything in a chat of this kind: the family chat or a group.</summary>
    public static bool Enabled(string? chatKind) => ChatKinds.IsShared(chatKind);

    /// <summary>
    /// Who a name in this chat may name — and who its polls count: the live roster in the family chat; in a group, the live
    /// members of THAT GROUP, in the order they joined; nobody anywhere else. A group whose members this device has not
    /// read yet (one an older build cached) offers nobody rather than the whole family.
    /// </summary>
    public static IReadOnlyList<MemberDto> Candidates(ChatDto? chat, IReadOnlyList<MemberDto> roster)
    {
        ArgumentNullException.ThrowIfNull(roster);
        switch (chat?.Kind)
        {
            case ChatKinds.Family:
                return Live(roster);
            case ChatKinds.Group:
                var live = Live(roster).GroupBy(member => member.Id).ToDictionary(group => group.Key, group => group.First());
                return [.. (chat.MemberIds ?? []).Distinct().Where(live.ContainsKey).Select(id => live[id])];
            default:
                return [];
        }
    }

    /// <summary>
    /// The names a half-typed <c>@</c> could mean, in the order of <paramref name="candidates"/> — never the reader, never
    /// somebody they blocked. Empty where mentions are not <paramref name="enabled"/>, while editing, and when the draft is
    /// not mid-mention.
    /// </summary>
    /// <param name="candidates">The chat's <see cref="Candidates"/>: the roster in the family chat, the group's members in a group.</param>
    public static IReadOnlyList<MemberDto> Offered(
        string draft, IReadOnlyList<MemberDto> candidates, long me, Func<long, bool> blocked, bool enabled, bool editing)
    {
        if (!enabled || editing || Mentions.Query(draft) is not { } query)
        {
            return [];
        }
        var roster = Live(candidates);
        var excluding = roster.Where(member => member.Id == me || blocked(member.Id)).Select(member => member.Id).ToHashSet();
        var offered = Mentions.Candidates([.. roster.Select(member => new Named(member.Id, member.DisplayName))], query, excluding)
            .Select(member => member.UserId)
            .ToHashSet();
        return [.. roster.Where(member => offered.Contains(member.Id))];
    }

    /// <summary>
    /// The strip's lines for a half-typed <c>@</c>: <b>THE ASSISTANT FIRST</b>, above the members (docs/protocol.md,
    /// "Mentioning a member", amended 2026-10-20, #106) — exactly where <c>@ai</c> may be written at all: the family chat or
    /// a group (amended 2026-10-22), not while editing, on a server whose assistant names who answers — narrowed by its display name and by <c>ai</c> as a
    /// member is by theirs; then <see cref="Offered"/>. The assistant is never a member here: if the roster carries its
    /// account, that row is left out rather than offered as <c>@&lt;display_name&gt;</c>.
    /// </summary>
    public static IReadOnlyList<MentionChoice> Choices(
        string draft,
        IReadOnlyList<MemberDto> candidates,
        long me,
        Func<long, bool> blocked,
        bool enabled,
        bool editing,
        AssistantDto? assistant)
    {
        var people = Offered(draft, candidates, me, blocked, enabled, editing)
            .Where(member => assistant is null || member.Id != assistant.UserId)
            .Select(member => new MentionChoice(member, member.DisplayName, member.Username.Length > 0 ? $"@{member.Username}" : string.Empty, member.DisplayName));
        if (!enabled || editing || !AttachMenu.AssistantAvailable(assistant) || Mentions.Query(draft) is not { } query)
        {
            return [.. people];
        }
        var token = AssistantText.Token[1..];
        var name = assistant!.DisplayName.Length > 0 ? assistant.DisplayName : token;
        var asked = Mentions.Candidates([new Named(assistant.UserId, name), new Named(assistant.UserId, token)], query, []).Count > 0;
        return asked ? [new MentionChoice(null, name, AssistantText.Token, token), .. people] : [.. people];
    }

    /// <summary>The names a text carries, resolved against the live members given, at most <see cref="Mentions.MaxPerMessage"/>.</summary>
    public static MentionDto[] Resolve(string text, IReadOnlyList<MemberDto> members) =>
    [
        .. Mentions.Resolve(text, [.. Live(members).Select(member => new Named(member.Id, member.DisplayName))])
            .Take(Mentions.MaxPerMessage)
            .Select(member => new MentionDto(member.UserId, member.Name)),
    ];

    /// <summary>
    /// What a send carries: the names of <paramref name="candidates"/> it says, where mentions are <paramref name="enabled"/>
    /// — the family chat, or a group with its own members as the candidates — and nothing anywhere else or when it names
    /// nobody. Resolving against the group's members is what keeps a send from naming somebody the server would refuse.
    /// </summary>
    public static MentionDto[]? ForSend(string body, IReadOnlyList<MemberDto> candidates, bool enabled)
    {
        if (!enabled)
        {
            return null;
        }
        var named = Resolve(body, candidates);
        return named.Length > 0 ? named : null;
    }

    /// <summary>
    /// Whether a name in a bubble is a DOOR: a live member who is not the reader and not somebody they blocked. A
    /// name outside that is drawn like any other and simply does not open.
    /// </summary>
    public static bool OpensChat(long userId, IReadOnlyList<MemberDto> members, long me, Func<long, bool> blocked) =>
        userId != me && !blocked(userId) && Live(members).Any(member => member.Id == userId);

    /// <summary>
    /// A text cut into its plain stretches and the names it says, each name marked with whether it is a door
    /// (<see cref="OpensChat"/>) — the same answer for a bubble and for a note somebody has opened. A text that
    /// names nobody is one plain stretch.
    /// </summary>
    public static IReadOnlyList<NameRun> Runs(
        string text, MentionDto[]? named, IReadOnlyList<MemberDto> members, long me, Func<long, bool> blocked)
    {
        if (named is not { Length: > 0 })
        {
            return [new NameRun(text, null, false)];
        }
        var runs = new List<NameRun>();
        foreach (var (part, userId) in Mentions.Runs(text, [.. named.Select(mention => new Named(mention.UserId, mention.Name))]))
        {
            runs.Add(new NameRun(part, userId, userId is { } id && OpensChat(id, members, me, blocked)));
        }
        return runs;
    }
}
