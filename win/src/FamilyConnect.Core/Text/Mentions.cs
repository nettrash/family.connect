using System.Globalization;
using System.Text;

namespace FamilyConnect.Core;

/// <summary>A member a text may name: the id, and the display name AS TYPED after the <c>@</c>.</summary>
public readonly record struct Named(long UserId, string Name);

/// <summary>One <c>@Name</c> a text carries, as a UTF-8 byte range on grapheme boundaries, and whose it is.</summary>
public readonly record struct NamedToken(int Start, int End, Named Member);

/// <summary>
/// Member mentions (docs/protocol.md, "Mentioning a member") — <c>fc_text::mentions</c>, held to it by
/// <c>ChatOracleTests</c>.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE GRAMMAR IS THE SERVER'S, BYTE FOR BYTE:</b> <c>@</c> followed by exactly the name, with a
/// boundary on both sides — an ASCII letter, digit or <c>_</c> next to it means it is a longer word, so
/// <c>@Ann</c> is not found inside <c>@Anna</c> and <c>mail@Anna</c> is an address. The server refuses
/// a mention the body does not carry, so the two must agree or a note is refused for a name its writer
/// can see. The scan is over UTF-8 BYTES, as the server's is; a match then grows outward to the grapheme
/// clusters it touches, which is the only span a highlight can be drawn over.
/// </para>
/// <para>
/// <b>THE RESOLUTION IS FROM THE TEXT:</b> every member whose <c>@Name</c> it carries, each once, in
/// order of first appearance — names tried LONGEST FIRST (UTF-8 bytes), ties to the lower id, a token
/// once claimed not offered again, so <c>@Anna Lee</c> names Anna Lee and not also Anna.
/// </para>
/// </remarks>
public static class Mentions
{
    /// <summary>The most names one message or note carries (the web composer's <c>MAX_MENTIONS</c>).</summary>
    public const int MaxPerMessage = 20;

    /// <summary>A byte that cannot continue a name: anything but an ASCII letter, digit or <c>_</c>.</summary>
    public static bool IsBoundary(byte value) =>
        !(value is >= (byte)'a' and <= (byte)'z' or >= (byte)'A' and <= (byte)'Z' or >= (byte)'0' and <= (byte)'9' or (byte)'_');

    /// <summary>Every <c>@name</c> in <paramref name="body"/>, the <c>@</c> included, as disjoint byte ranges, in order.</summary>
    public static IReadOnlyList<(int Start, int End)> Ranges(string body, string name)
    {
        if (name.Length == 0)
        {
            return [];
        }
        var bytes = Encoding.UTF8.GetBytes(body);
        var token = Encoding.UTF8.GetBytes(name);
        var length = token.Length + 1;
        var found = new List<(int Start, int End)>();
        int[]? clusters = null;
        var index = 0;
        while (index + length <= bytes.Length)
        {
            if (bytes[index] == (byte)'@'
                && bytes.AsSpan(index + 1, token.Length).SequenceEqual(token)
                && (index == 0 || IsBoundary(bytes[index - 1]))
                && (index + length == bytes.Length || IsBoundary(bytes[index + length])))
            {
                clusters ??= ClusterBoundaries(body);
                var range = Widen(clusters, index, index + length);
                if (found.Count == 0 || found[^1].End <= range.Start)
                {
                    found.Add(range);
                }
                index += length;
            }
            else
            {
                index++;
            }
        }
        return found;
    }

    /// <summary>Whether <paramref name="body"/> names this member — the server's <c>names_member</c>.</summary>
    public static bool Names(string body, string name) => Ranges(body, name).Count > 0;

    /// <summary>The members <paramref name="body"/> names, resolved against the roster. Empty when it names nobody.</summary>
    public static IReadOnlyList<Named> Resolve(string body, IReadOnlyList<Named> roster)
    {
        if (!body.Contains('@'))
        {
            return [];
        }
        var seen = new HashSet<long>();
        var claimed = new List<(int Start, int End)>();
        var found = new List<(int Offset, Named Member)>();
        foreach (var member in LongestFirst(roster))
        {
            if (seen.Contains(member.UserId))
            {
                continue;
            }
            (int Start, int End)? first = null;
            foreach (var range in Ranges(body, member.Name))
            {
                if (!claimed.Any(taken => Overlaps(taken, range)))
                {
                    first = range;
                    break;
                }
            }
            if (first is not { } kept)
            {
                continue;
            }
            seen.Add(member.UserId);
            found.Add((kept.Start, member));
            claimed.Add(kept);
        }
        return [.. found.OrderBy(entry => entry.Offset).Select(entry => entry.Member)];
    }

    /// <summary>
    /// Every <c>@Name</c> token a text draws, one owner per token — its own mentions list, longest name
    /// first, a token claimed once — in order of position.
    /// </summary>
    public static IReadOnlyList<NamedToken> Tokens(string text, IReadOnlyList<Named> mentions)
    {
        var found = new List<NamedToken>();
        foreach (var mention in LongestFirst(mentions))
        {
            foreach (var range in Ranges(text, mention.Name))
            {
                if (!found.Any(token => Overlaps((token.Start, token.End), range)))
                {
                    found.Add(new NamedToken(range.Start, range.End, mention));
                }
            }
        }
        return [.. found.OrderBy(token => token.Start)];
    }

    /// <summary>Longest name first — UTF-8 bytes — then the lower id; stable, so exact duplicates keep their order.</summary>
    private static IEnumerable<Named> LongestFirst(IReadOnlyList<Named> members) =>
        members.OrderByDescending(member => Encoding.UTF8.GetByteCount(member.Name)).ThenBy(member => member.UserId);

    private static bool Overlaps((int Start, int End) a, (int Start, int End) b) => a.Start < b.End && b.Start < a.End;

    /// <summary>The UTF-8 byte offset of every grapheme-cluster boundary in the text, its end included.</summary>
    private static int[] ClusterBoundaries(string text)
    {
        var boundaries = new List<int>();
        var elements = StringInfo.GetTextElementEnumerator(text);
        var chars = 0;
        var bytes = 0;
        while (elements.MoveNext())
        {
            var at = elements.ElementIndex;
            bytes += Encoding.UTF8.GetByteCount(text.AsSpan(chars, at - chars));
            chars = at;
            boundaries.Add(bytes);
        }
        boundaries.Add(Encoding.UTF8.GetByteCount(text));
        return [.. boundaries];
    }

    /// <summary>A byte range grown outward to the nearest cluster boundaries.</summary>
    private static (int Start, int End) Widen(int[] clusters, int start, int end)
    {
        var from = clusters.LastOrDefault(boundary => boundary <= start);
        var to = clusters.FirstOrDefault(boundary => boundary >= end, clusters[^1]);
        return (from, to);
    }
}
