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

    /// <summary>
    /// A text cut into its plain stretches and the names it says, as UTF-16 strings — what a bubble or a
    /// sticker draws a name bold from. A text naming nobody is one plain run.
    /// </summary>
    public static IReadOnlyList<(string Text, long? UserId)> Runs(string text, IReadOnlyList<Named> mentions)
    {
        var tokens = Tokens(text, mentions);
        if (tokens.Count == 0)
        {
            return [(text, null)];
        }
        // Byte offsets onto character offsets: every token boundary is a cluster boundary, so it is on a rune.
        var charAt = new Dictionary<int, int>();
        var bytes = 0;
        var chars = 0;
        foreach (var rune in text.EnumerateRunes())
        {
            charAt[bytes] = chars;
            bytes += rune.Utf8SequenceLength;
            chars += rune.Utf16SequenceLength;
        }
        charAt[bytes] = chars;
        var runs = new List<(string Text, long? UserId)>();
        var at = 0;
        foreach (var token in tokens)
        {
            var (start, end) = (charAt[token.Start], charAt[token.End]);
            if (start > at)
            {
                runs.Add((text[at..start], null));
            }
            runs.Add((text[start..end], token.Member.UserId));
            at = end;
        }
        if (at < text.Length)
        {
            runs.Add((text[at..], null));
        }
        return runs;
    }

    // ---- the composer's @ strip (fc_text::mentions query / candidates / accept) ------------------------

    /// <summary>
    /// The prefix being typed after a trailing <c>@</c>, or null when the composer is not mid-mention: no
    /// <c>@</c>, one that follows an ASCII letter, digit or <c>_</c> (an address), or a line break after it.
    /// Empty when the <c>@</c> was just typed — every candidate is offered then. The <c>@</c> is the last
    /// CHARACTER that is exactly <c>@</c>, and a line break is a character equal to a lone LF, so a CR LF pair
    /// does not end it — the Swift answers, ported.
    /// </summary>
    public static string? Query(string draft)
    {
        if (LastAt(draft) is not { } at)
        {
            return null;
        }
        if (at > 0 && draft[at - 1] is (>= 'a' and <= 'z') or (>= 'A' and <= 'Z') or (>= '0' and <= '9') or '_')
        {
            return null;
        }
        var tail = draft[(at + 1)..];
        return Graphemes(tail).Any(character => character == "\n") ? null : tail;
    }

    /// <summary>
    /// The roster narrowed to what <paramref name="query"/> could be the start of, minus
    /// <paramref name="excluding"/>, in roster order — each name lowercased scalar by scalar with no context,
    /// and compared character by character.
    /// </summary>
    public static IReadOnlyList<Named> Candidates(IReadOnlyList<Named> roster, string query, IReadOnlyCollection<long> excluding)
    {
        var needle = Lowercased(query);
        return [.. roster.Where(member =>
            !excluding.Contains(member.UserId) && (needle.Length == 0 || HasPrefix(Lowercased(member.Name), needle)))];
    }

    /// <summary>The draft with its trailing <c>@prefix</c> replaced by <c>@Name </c> — or, with no <c>@</c> at all, <c>@Name </c> appended.</summary>
    public static string Accept(string draft, string name) =>
        LastAt(draft) is { } at ? $"{draft[..at]}@{name} " : $"{draft}@{name} ";

    private static int? LastAt(string text)
    {
        int? found = null;
        var elements = StringInfo.GetTextElementEnumerator(text);
        while (elements.MoveNext())
        {
            if (elements.GetTextElement() == "@")
            {
                found = elements.ElementIndex;
            }
        }
        return found;
    }

    private static IEnumerable<string> Graphemes(string text)
    {
        var elements = StringInfo.GetTextElementEnumerator(text);
        while (elements.MoveNext())
        {
            yield return elements.GetTextElement();
        }
    }

    /// <summary>Every scalar's full lowercase mapping with no context — the one multi-scalar mapping, U+0130, spelled out.</summary>
    private static string Lowercased(string text)
    {
        var lower = new StringBuilder(text.Length);
        foreach (var rune in text.EnumerateRunes())
        {
            if (rune.Value == 0x130)
            {
                lower.Append('i').Append((char)0x307);
            }
            else
            {
                lower.Append(Rune.ToLowerInvariant(rune).ToString());
            }
        }
        return lower.ToString();
    }

    private static bool HasPrefix(string text, string prefix)
    {
        using var characters = Graphemes(text).GetEnumerator();
        foreach (var expected in Graphemes(prefix))
        {
            if (!characters.MoveNext() || characters.Current != expected)
            {
                return false;
            }
        }
        return true;
    }

    /// <summary>Longest name first — UTF-8 bytes — then the lower id; stable, so exact duplicates keep their order.</summary>
    private static IEnumerable<Named> LongestFirst(IReadOnlyList<Named> members) =>
        members.OrderByDescending(member => Encoding.UTF8.GetByteCount(member.Name)).ThenBy(member => member.UserId);

    private static bool Overlaps((int Start, int End) a, (int Start, int End) b) => a.Start < b.End && b.Start < a.End;

    /// <summary>The UTF-8 byte offset of every grapheme-cluster boundary in the text, its end included.</summary>
    internal static int[] ClusterBoundaries(string text)
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
    internal static (int Start, int End) Widen(int[] clusters, int start, int end)
    {
        var from = clusters.LastOrDefault(boundary => boundary <= start);
        var to = clusters.FirstOrDefault(boundary => boundary >= end, clusters[^1]);
        return (from, to);
    }
}
