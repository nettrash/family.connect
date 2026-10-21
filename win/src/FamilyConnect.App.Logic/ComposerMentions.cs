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
/// <b>ONLY IN THE FAMILY CHAT</b>, where a mention means anything, and never while a message is being edited.
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

    /// <summary>
    /// The names a half-typed <c>@</c> could mean, in roster order — never the reader, never somebody they
    /// blocked. Empty outside the family chat, while editing, and when the draft is not mid-mention.
    /// </summary>
    public static IReadOnlyList<MemberDto> Offered(
        string draft, IReadOnlyList<MemberDto> members, long me, Func<long, bool> blocked, bool familyChat, bool editing)
    {
        if (!familyChat || editing || Mentions.Query(draft) is not { } query)
        {
            return [];
        }
        var roster = Live(members);
        var excluding = roster.Where(member => member.Id == me || blocked(member.Id)).Select(member => member.Id).ToHashSet();
        var offered = Mentions.Candidates([.. roster.Select(member => new Named(member.Id, member.DisplayName))], query, excluding)
            .Select(member => member.UserId)
            .ToHashSet();
        return [.. roster.Where(member => offered.Contains(member.Id))];
    }

    /// <summary>
    /// The strip's lines for a half-typed <c>@</c>: <b>THE ASSISTANT FIRST</b>, above the members (docs/protocol.md,
    /// "Mentioning a member", amended 2026-10-20, #106) — exactly where <c>@ai</c> may be written at all: the family chat,
    /// not while editing, on a server whose assistant names who answers — narrowed by its display name and by <c>ai</c> as a
    /// member is by theirs; then <see cref="Offered"/>. The assistant is never a member here: if the roster carries its
    /// account, that row is left out rather than offered as <c>@&lt;display_name&gt;</c>.
    /// </summary>
    public static IReadOnlyList<MentionChoice> Choices(
        string draft,
        IReadOnlyList<MemberDto> members,
        long me,
        Func<long, bool> blocked,
        bool familyChat,
        bool editing,
        AssistantDto? assistant)
    {
        var people = Offered(draft, members, me, blocked, familyChat, editing)
            .Where(member => assistant is null || member.Id != assistant.UserId)
            .Select(member => new MentionChoice(member, member.DisplayName, member.Username.Length > 0 ? $"@{member.Username}" : string.Empty, member.DisplayName));
        if (!familyChat || editing || !AttachMenu.AssistantAvailable(assistant) || Mentions.Query(draft) is not { } query)
        {
            return [.. people];
        }
        var token = AssistantText.Token[1..];
        var name = assistant!.DisplayName.Length > 0 ? assistant.DisplayName : token;
        var asked = Mentions.Candidates([new Named(assistant.UserId, name), new Named(assistant.UserId, token)], query, []).Count > 0;
        return asked ? [new MentionChoice(null, name, AssistantText.Token, token), .. people] : [.. people];
    }

    /// <summary>The names a text carries, resolved against the live roster, at most <see cref="Mentions.MaxPerMessage"/>.</summary>
    public static MentionDto[] Resolve(string text, IReadOnlyList<MemberDto> members) =>
    [
        .. Mentions.Resolve(text, [.. Live(members).Select(member => new Named(member.Id, member.DisplayName))])
            .Take(Mentions.MaxPerMessage)
            .Select(member => new MentionDto(member.UserId, member.Name)),
    ];

    /// <summary>What a send carries: the names in the family chat, and nothing anywhere else or when it names nobody.</summary>
    public static MentionDto[]? ForSend(string body, IReadOnlyList<MemberDto> members, bool familyChat)
    {
        if (!familyChat)
        {
            return null;
        }
        var named = Resolve(body, members);
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
