using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>The words a poll is drawn with, in the apps' own sentences.</summary>
public static class PollText
{
    /// <summary>
    /// "3 of 5 voted" — or "3 voted" before the roster is known. The count that chooses the plural form
    /// is the one that VARIES; the roster's size is only the second number.
    /// </summary>
    public static string Footer(PollDto poll, int memberCount, IStringCatalog say)
    {
        var voted = Polls.VoterCount(poll);
        return memberCount > 0
            ? say.Plural("%lld of %lld voted", voted, voted, memberCount)
            : say.Plural("%lld voted", voted, voted);
    }

    /// <summary>What an option says to a screen reader: its words, its votes, and whether it is the reader's.</summary>
    public static string OptionLabel(PollOptionDto option, bool chosen, IStringCatalog say)
    {
        var votes = say.Plural("%@. %lld votes", option.Votes.Length, option.Text, option.Votes.Length);
        return chosen ? $"{votes}. {say.Get("Your choice")}" : votes;
    }

    /// <summary>"You" for the reader, "Deleted account" for an account that is gone, the roster's name, or "Someone".</summary>
    public static string Name(long userId, long reader, Func<long, MemberDto?> member, IStringCatalog say)
    {
        if (userId == reader)
        {
            return say.Get("You");
        }
        return member(userId) switch
        {
            { Deleted: true } => say.Get("Deleted account"),
            { DisplayName: { Length: > 0 } name } => name,
            _ => say.Get("Someone"),
        };
    }

    /// <summary>
    /// Who chose an option, named up to <see cref="Polls.MaxFaces"/> and "+N" past that. The "+N" is the
    /// overflow of the DRAWABLE names, never a tally: counted from the raw list it would print how many
    /// blocked people chose that option.
    /// </summary>
    public static string Voters(IReadOnlyList<long> drawable, Func<long, string> name, IStringCatalog say)
    {
        var shown = string.Join(", ", drawable.Take(Polls.MaxFaces).Select(name));
        var more = drawable.Count - Polls.MaxFaces;
        return more > 0 ? $"{shown} {say.Format("+%lld", more)}" : shown;
    }

    /// <summary>
    /// The M in "N of M voted": the live roster — neither the people who have left nor accounts that were
    /// deleted, because a tally must not go on counting somebody who no longer exists.
    /// </summary>
    public static int MemberCount(IEnumerable<MemberDto> members) =>
        members.Count(member => !member.HasLeft && !member.Deleted);
}
