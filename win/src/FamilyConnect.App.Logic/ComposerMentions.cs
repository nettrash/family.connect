using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

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
}
