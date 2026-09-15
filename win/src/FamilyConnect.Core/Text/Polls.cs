using System.Globalization;
using System.Text;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core;

/// <summary>
/// A poll's arithmetic and the composer's check (docs/protocol.md, "Polls") — the rules the web
/// client keeps in <c>views/poll.rs</c> and the apps in <c>PollPresentation</c>, in one place the
/// bubble and the open-polls sheet both draw from.
/// </summary>
/// <remarks>
/// <para>
/// <b>COUNTS COUNT EVERYONE; NAMES SKIP THE BLOCKED.</b> The voter count, the bars and "N of M voted"
/// go on counting a blocked member's vote, because a tally that moved when you blocked somebody
/// would tell you they had voted. Only <see cref="DrawableVoters"/> filters, and it filters
/// identity alone (docs/protocol.md, "Blocking a member").
/// </para>
/// <para>
/// <b>THE CHECK IS THE SERVER'S, IN RUST'S OWN STD.</b> The server trims with <c>str::trim</c>,
/// counts <c>chars()</c> — scalars, not UTF-16 units — and folds with <c>str::to_lowercase</c>,
/// which maps <c>İ</c> to two scalars and applies the final-sigma rule. .NET's invariant lowercase
/// does neither, so a composer built on it would refuse "ΟΔΟΣ" beside "οδοσ", two options the
/// server takes. The chat oracle pins all three to Rust.
/// </para>
/// </remarks>
public static class Polls
{
    public const int MinOptions = 2;
    public const int MaxOptions = 10;
    public const int MaxOptionChars = 100;

    /// <summary>How many voters a row names before "+N".</summary>
    public const int MaxFaces = 5;

    /// <summary>The option this reader holds, if any — one choice per member.</summary>
    public static long? MyOption(PollDto poll, long me) =>
        poll.Options.FirstOrDefault(option => option.Votes.Contains(me))?.Id;

    /// <summary>Everyone who has voted, counted once.</summary>
    public static int VoterCount(PollDto poll) =>
        poll.Options.SelectMany(option => option.Votes).Distinct().Count();

    /// <summary>
    /// How full one option's bar is: its share of the votes CAST, not of the family — 0 while nobody
    /// has voted, so a poll opens with every bar empty.
    /// </summary>
    public static double Fraction(PollDto poll, int votes)
    {
        var total = poll.Options.Sum(option => option.Votes.Length);
        return total == 0 ? 0 : (double)votes / total;
    }

    /// <summary>The voters whose names may be DRAWN, in the order they voted: everyone but the blocked.</summary>
    public static IReadOnlyList<long> DrawableVoters(IReadOnlyList<long> votes, Func<long, bool> blocked) =>
        [.. votes.Where(voter => !blocked(voter))];

    /// <summary>
    /// What a tap means: the option already held clears the vote, any other casts it. The server's
    /// vote is a state-set and not a toggle, so this is the client's to decide.
    /// </summary>
    public static bool TapRetracts(PollDto poll, long optionId, long me) => MyOption(poll, me) == optionId;

    /// <summary>A poll the server has numbered and nobody has closed.</summary>
    public static bool Votable(MessageDto message) => message.Id != 0 && message.Poll is { Closed: false };

    /// <summary>
    /// Open polls this reader has not answered — the badge on the way to the list. A message held
    /// twice (the chat's copy and the list's) counts once, by its newest state; a blocked member's
    /// poll is not one to chase.
    /// </summary>
    public static int Unanswered(
        IEnumerable<MessageDto> held, IEnumerable<MessageDto> listed, long me, Func<long, bool> blocked) =>
        held.Concat(listed)
            .Where(message => message.Id != 0 && message.Poll is not null)
            .GroupBy(message => message.Id)
            .Select(copies => copies.MaxBy(copy => copy.Poll!.PollSeq)!)
            .Where(message => !blocked(message.SenderId))
            .Count(message => !message.Poll!.Closed && MyOption(message.Poll, me) is null);

    /// <summary>Rust's <c>str::trim</c>: the White_Space property, which is .NET's <c>char.IsWhiteSpace</c> set.</summary>
    public static string Trimmed(string text) => text.Trim();

    /// <summary>A question is there once it is more than white space.</summary>
    public static bool HasQuestion(string question) => Trimmed(question).Length > 0;

    /// <summary>
    /// The options as they will be sent — trimmed, blank rows dropped — or null when the server would
    /// answer <c>invalid_poll</c>: fewer than 2 or more than 10, one over 100 characters, or two the
    /// same ignoring case. The CLEANED list rather than a yes, so what was checked is what leaves.
    /// </summary>
    public static string[]? Sanitized(IEnumerable<string> options)
    {
        string[] trimmed = [.. options.Select(Trimmed).Where(option => option.Length > 0)];
        if (trimmed.Length is < MinOptions or > MaxOptions)
        {
            return null;
        }
        if (trimmed.Any(option => option.EnumerateRunes().Count() > MaxOptionChars))
        {
            return null;
        }
        var seen = new HashSet<string>(StringComparer.Ordinal);
        return trimmed.All(option => seen.Add(Lowercase(option))) ? trimmed : null;
    }

    /// <summary>
    /// Rust's <c>str::to_lowercase</c>: each scalar's full lowercase mapping — <c>İ</c> becomes
    /// <c>i</c> and a combining dot — and a capital sigma that ends a word becomes <c>ς</c>.
    /// </summary>
    public static string Lowercase(string text)
    {
        var runes = text.EnumerateRunes().ToArray();
        var lower = new StringBuilder(text.Length);
        for (var index = 0; index < runes.Length; index++)
        {
            var rune = runes[index];
            switch (rune.Value)
            {
                case 0x3A3:
                    // Final_Sigma, as Rust reads it: cased before (past anything case-ignorable) and
                    // not cased after.
                    var final = CaseIgnorableThenCased(runes.Take(index).Reverse())
                        && !CaseIgnorableThenCased(runes.Skip(index + 1));
                    lower.Append(final ? (char)0x3C2 : (char)0x3C3);
                    break;
                default:
                    // Rust's own full mapping — U+0130 included — and never this platform's casing tables.
                    var mapped = new List<byte>(4);
                    RustChar.AppendLowercase(mapped, rune.Value);
                    lower.Append(Encoding.UTF8.GetString([.. mapped]));
                    break;
            }
        }
        return lower.ToString();
    }

    private static bool CaseIgnorableThenCased(IEnumerable<Rune> runes)
    {
        foreach (var rune in runes)
        {
            if (!CaseIgnorable(rune))
            {
                return Cased(rune);
            }
        }
        return false;
    }

    /// <summary>Case_Ignorable, as Rust's <c>str::to_lowercase</c> reads it — from the table the standard library printed.</summary>
    private static bool CaseIgnorable(Rune rune) => RustChar.InRanges(UnicodeProperties.CaseIgnorable, rune.Value);

    /// <summary>Cased, as the same context reads it (only asked of what it did not skip).</summary>
    private static bool Cased(Rune rune) => RustChar.InRanges(UnicodeProperties.CasedNotIgnorable, rune.Value);
}
