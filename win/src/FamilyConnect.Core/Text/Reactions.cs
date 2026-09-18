using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core;

/// <summary>One chip under a bubble: an emoji, how many chose it, and whether the reader did.</summary>
public sealed record ReactionChip(string Emoji, int Count, bool IncludesMe)
{
    /// <summary>The count is drawn only from two up: a lone reaction's chip is just its emoji.</summary>
    public bool ShowsCount => Count > 1;

    /// <summary>
    /// A tap JOINS a chip and never removes: on a chip the reader is part of, it shows who reacted,
    /// where their own row is the remove control. Undoing something you never meant to do is a worse
    /// failure than one extra tap — every client does this, and the Mac once did not.
    /// </summary>
    public bool TapJoins => !IncludesMe;
}

/// <summary>One emoji's row in "See who reacted".</summary>
public sealed record ReactionDetail(string Emoji, IReadOnlyList<string> Names, long? LeadUserId);

/// <summary>What a toggle does before the server answers.</summary>
public sealed record ReactionToggle(bool Removing, IReadOnlyList<ReactionDto> Reactions);

/// <summary>
/// A message's reactions as the bubble draws them — ported from <c>fc_text::reactions</c> and
/// <c>fc_text::emoji</c>, the Rust the web client runs, and held to it by <c>ChatOracleTests</c>.
/// </summary>
/// <remarks>
/// The server holds ONE reaction per user per message and sends the full list in the order the
/// reactions were made, so everything here is a pure function of that list. Emoji are compared
/// ORDINALLY, as the server and Kotlin compare them.
/// </remarks>
public static class Reactions
{
    /// <summary>
    /// The quick set every client offers, in the same order. The heart carries an invisible
    /// variation selector: without it, it is a different string and a different chip.
    /// </summary>
    public static IReadOnlyList<string> Quick { get; } =
    [
        char.ConvertFromUtf32(0x2764) + char.ConvertFromUtf32(0xFE0F),
        char.ConvertFromUtf32(0x1F44D),
        char.ConvertFromUtf32(0x1F44E),
        char.ConvertFromUtf32(0x1F602),
        char.ConvertFromUtf32(0x1F62E),
        char.ConvertFromUtf32(0x1F622),
    ];

    /// <summary>What a double-click toggles: the heart, the same on every client.</summary>
    public static string DoubleTap => Quick[0];

    /// <summary>
    /// One chip per distinct emoji, in the order each emoji FIRST appears — never by popularity,
    /// so a chip does not jump when others pile onto a later one.
    /// </summary>
    public static IReadOnlyList<ReactionChip> Chips(IReadOnlyList<ReactionDto> reactions, long me)
    {
        var chips = new List<ReactionChip>();
        foreach (var reaction in reactions)
        {
            var mine = reaction.UserId == me;
            var at = chips.FindIndex(chip => string.Equals(chip.Emoji, reaction.Emoji, StringComparison.Ordinal));
            if (at < 0)
            {
                chips.Add(new ReactionChip(reaction.Emoji, 1, mine));
            }
            else
            {
                chips[at] = chips[at] with { Count = chips[at].Count + 1, IncludesMe = chips[at].IncludesMe || mine };
            }
        }
        return chips;
    }

    /// <summary>The reader's own reaction, or null. One per user, so the first is the only.</summary>
    public static string? Mine(IReadOnlyList<ReactionDto> reactions, long me) =>
        reactions.FirstOrDefault(reaction => reaction.UserId == me)?.Emoji;

    /// <summary>
    /// The optimistic rewrite of a tap: the reader's reaction comes off, and goes back on APPENDED —
    /// as the server does — unless they tapped the one they already had, which is a removal.
    /// The input is left alone: it is what a failed request restores.
    /// </summary>
    public static ReactionToggle Toggle(IReadOnlyList<ReactionDto> reactions, long me, string emoji)
    {
        var removing = string.Equals(Mine(reactions, me), emoji, StringComparison.Ordinal);
        var rewritten = reactions.Where(reaction => reaction.UserId != me).ToList();
        if (!removing)
        {
            rewritten.Add(new ReactionDto(me, emoji));
        }
        return new ReactionToggle(removing, rewritten);
    }

    /// <summary>
    /// The capsule's items: the quick set, plus the reader's own reaction when it is not one of
    /// them — so their reaction is always on show and can be taken off.
    /// </summary>
    public static IReadOnlyList<string> Capsule(string? mine)
    {
        var emojis = Quick.ToList();
        if (mine is not null && !emojis.Contains(mine, StringComparer.Ordinal))
        {
            emojis.Add(mine);
        }
        return emojis;
    }

    /// <summary>
    /// The rows of "See who reacted", in the SAME order as the chips. The reader is "You" and comes
    /// first; everybody else keeps reaction order, by name or "Someone".
    /// </summary>
    /// <remarks>
    /// A BLOCKED reactor is dropped from these rows and from nowhere else: the chip keeps their
    /// count, so a chip may read 3 while its row names two. A count that moved would tell the
    /// blocked person they had been (docs/protocol.md, "Blocking a member").
    /// </remarks>
    public static IReadOnlyList<ReactionDetail> Details(
        IReadOnlyList<ReactionDto> reactions,
        Func<long, string?> nameOf,
        long me,
        IReadOnlySet<long> blocked,
        IStringCatalog? words = null)
    {
        var say = words ?? EnglishCatalog.Instance;
        var rows = new List<(string Emoji, bool Mine, List<string> Others, List<long> OtherIds)>();
        foreach (var reaction in reactions)
        {
            var at = rows.FindIndex(row => string.Equals(row.Emoji, reaction.Emoji, StringComparison.Ordinal));
            if (at < 0)
            {
                rows.Add((reaction.Emoji, false, [], []));
                at = rows.Count - 1;
            }
            if (reaction.UserId == me)
            {
                rows[at] = rows[at] with { Mine = true };
            }
            else if (!blocked.Contains(reaction.UserId))
            {
                rows[at].Others.Add(nameOf(reaction.UserId) ?? say.Get("Someone"));
                rows[at].OtherIds.Add(reaction.UserId);
            }
        }
        return rows
            .Select(row => row.Mine
                ? new ReactionDetail(row.Emoji, [say.Get("You"), .. row.Others], me)
                : new ReactionDetail(row.Emoji, row.Others, row.OtherIds.Count > 0 ? row.OtherIds[0] : null))
            .ToList();
    }
}
