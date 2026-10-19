using System.Collections.Frozen;
using System.Text;

namespace FamilyConnect.App.Logic;

/// <summary>
/// Emoji as you type (issue #101; docs/emoji-as-you-type-2026-10-18.md): Telegram's emoticons — <c>:-)</c>, <c>&lt;3</c>,
/// <c>;-P</c> — with their no-nose forms, and <c>:name:</c> shortcodes, become emoji in the field the moment the last character
/// goes in, and a Backspace straight after takes the replacement back. The same rule as the web's
/// <c>fc_text::emoji_typing</c>, the original every port is held to (board-oracle's <c>emoji-typing</c> vectors).
/// </summary>
/// <remarks>
/// <para>
/// Every prose field runs it, single-line ones included, for one typed character at a time: never on a paste, never in code.
/// The text that is sent holds the emoji itself, so nothing about it is on the wire.
/// </para>
/// <para>
/// Offsets are UTF-16 indices into the field's own text, as a <c>TextBox</c> counts them, where the Rust counts UTF-8 bytes.
/// Every guard is ASCII on purpose: the four languages' "is a letter" disagree past it.
/// </para>
/// </remarks>
public static class EmojiTyping
{
    /// <summary>The embedded copy of <c>web/text/data/emoji-names.tsv</c> (tools/emoji/make_names.py writes it; CI compares every copy).</summary>
    internal const string NamesResource = "FamilyConnect.App.Logic.emoji-names.tsv";

    /// <summary>
    /// The emoticons, as typed, and the emoji each becomes — in the original's order. The first 27 are Telegram Desktop's own
    /// table, emoji for emoji; the rest are the owner's addition (2026-10-18): the no-nose forms of the same faces, so <c>:)</c>
    /// works, and the case twins Telegram's case-sensitive table misses. <c>8)</c> and <c>B)</c> are left out on purpose.
    /// An entry ending in a space is replaced when that space is typed, and the space stays: <c>:o</c> and <c>:p</c> would
    /// otherwise eat the start of <c>:ok_hand:</c> and <c>:pizza:</c>, and <c>xD</c> of <c>xDDD</c>.
    /// </summary>
    public static IReadOnlyList<(string Text, string Emoji)> Emoticons { get; } =
    [
        // Telegram's.
        (":-)", "\U0001F60A"),
        ("8-)", "\U0001F60D"),
        ("<3", "❤️"),
        (":-*", "\U0001F61A"),
        ("B-)", "\U0001F60E"),
        (":-D", "\U0001F603"),
        (";-)", "\U0001F609"),
        (";-P", "\U0001F61C"),
        (":-p", "\U0001F60B"),
        ("3(", "\U0001F614"),
        (":-(", "\U0001F61E"),
        (":]", "\U0001F60F"),
        (":'(", "\U0001F622"),
        (":_(", "\U0001F62D"),
        (":((", "\U0001F629"),
        (":|", "\U0001F610"),
        ("3-)", "\U0001F60C"),
        (">(", "\U0001F620"),
        (">((", "\U0001F621"),
        ("O:)", "\U0001F607"),
        (";o", "\U0001F630"),
        ("8|", "\U0001F633"),
        ("8o", "\U0001F632"),
        (":X", "\U0001F637"),
        ("}:)", "\U0001F608"),
        (":o ", "\U0001F628"),
        ("xD ", "\U0001F606"),
        // The no-nose forms and the case twins.
        (":)", "\U0001F60A"),
        (":D", "\U0001F603"),
        (";)", "\U0001F609"),
        (":(", "\U0001F61E"),
        (":*", "\U0001F61A"),
        (":P", "\U0001F60B"),
        (":p ", "\U0001F60B"),
        (":-P", "\U0001F60B"),
        (";P", "\U0001F61C"),
        (";p", "\U0001F61C"),
        (";-p", "\U0001F61C"),
        ("O:-)", "\U0001F607"),
        ("}:-)", "\U0001F608"),
        ("XD ", "\U0001F606"),
    ];

    /// <summary>
    /// The shortcode table, read once from the embedded resource the first time a name is looked up, and never written after:
    /// immutable, so tests running in parallel share it safely. A name listed twice keeps its last line, as the original's
    /// <c>HashMap</c> collect does.
    /// </summary>
    private static readonly Lazy<FrozenDictionary<string, string>> Names = new(LoadNames);

    /// <summary>The emoji <paramref name="name"/> (no colons) stands for, or null. Names are matched exactly: <c>:Fire:</c> is not <c>:fire:</c>.</summary>
    public static string? EmojiNamed(string name) => Names.Value.TryGetValue(name, out var emoji) ? emoji : null;

    /// <summary>How many names the table holds — for the tests, which hold it to the original's size.</summary>
    public static int NameCount => Names.Value.Count;

    /// <summary>
    /// What typing <paramref name="typed"/> turns into, or null when the character goes in as it is. <paramref name="before"/> is
    /// the field's text before the caret, without <paramref name="typed"/>; <paramref name="typed"/> is the ONE character the
    /// keyboard produced — one UTF-16 unit or a surrogate pair, after capitals (#97) decided it; <paramref name="live"/> is the
    /// swap the field still holds, if any. In order:
    /// <list type="number">
    /// <item>nothing in code (the test #97's capitals use, <see cref="Capitals.InCode"/>);</item>
    /// <item>GROWING: the caret is right behind <paramref name="live"/>'s replacement, and what was typed then plus
    /// <paramref name="typed"/> is a longer emoticon starting at the same place — <c>:(</c> then <c>(</c> is <c>:((</c>, one
    /// swap whose Backspace puts back all three characters;</item>
    /// <item>a SHORTCODE, when <paramref name="typed"/> is <c>:</c> — <paramref name="before"/> ends in <c>:name</c> with no
    /// whitespace or colon in the name, the opening colon is at the start or after something that is not an ASCII letter or
    /// digit (so <c>10:30:</c> and <c>a:b:</c> are left alone), and the table knows the name;</item>
    /// <item>an EMOTICON: <paramref name="before"/> plus <paramref name="typed"/> ends in one, the longest whose guard passes —
    /// one starting with <c>:</c> or <c>;</c> must not follow an ASCII digit (<c>1:]</c>), any other must not follow an ASCII
    /// letter or digit (<c>x&lt;3</c>, <c>item3(</c>). <c>HELLO:)</c> is <c>:)</c> after <c>HELL</c>'s <c>O</c>, because the
    /// longer <c>O:)</c> fails its guard.</item>
    /// </list>
    /// </summary>
    public static EmojiSwap? Replace(string before, string typed, EmojiSwap? live)
    {
        // 1. Code is typed as it is.
        if (Capitals.InCode(before))
        {
            return null;
        }
        // 2. Growing the swap just made.
        if (live is not null
            && live.At >= 0
            && live.At + live.Put.Length == before.Length
            && string.CompareOrdinal(before, live.At, live.Put, 0, live.Put.Length) == 0)
        {
            var grown = live.Typed + typed;
            if (EmoticonOf(grown) is { } emoji)
            {
                return new EmojiSwap(live.At, PutFor(grown, emoji), grown);
            }
        }
        // 3. `:name:`.
        if (typed == ":")
        {
            var colon = before.LastIndexOf(':');
            if (colon >= 0)
            {
                var name = before[(colon + 1)..];
                // char.IsWhiteSpace is Unicode's White_Space, Rust's char::is_whitespace: every member is one UTF-16 unit.
                if (name.Length > 0
                    && !name.Any(char.IsWhiteSpace)
                    && !(colon > 0 && char.IsAsciiLetterOrDigit(before[colon - 1]))
                    && EmojiNamed(name) is { } named)
                {
                    return new EmojiSwap(colon, named, $":{name}:");
                }
            }
        }
        // 4. An emoticon, the longest that may stand here. (Two that tie in length and both end the text would be the same
        // emoticon, and the table has none twice, so which of a tie wins never arises.)
        var typedText = before + typed;
        (string Text, string Emoji)? best = null;
        foreach (var entry in Emoticons)
        {
            if (typedText.EndsWith(entry.Text, StringComparison.Ordinal)
                && GuardPasses(typedText[..^entry.Text.Length], entry.Text)
                && (best is null || entry.Text.Length > best.Value.Text.Length))
            {
                best = entry;
            }
        }
        return best is { } found
            ? new EmojiSwap(typedText.Length - found.Text.Length, PutFor(found.Text, found.Emoji), found.Text)
            : null;
    }

    /// <summary>
    /// Types <paramref name="keys"/> into an empty field with the caret always at the end, the way every port's vectors are
    /// checked: <c>⌫</c> is a Backspace (the live swap taken back, or else the last character removed), <c>→</c> is "something
    /// else happened" (a click, a caret move — the live swap is dropped), and every other character is typed.
    /// </summary>
    public static string Typing(string keys)
    {
        var text = string.Empty;
        EmojiSwap? live = null;
        foreach (var rune in keys.EnumerateRunes())
        {
            var key = rune.ToString();
            switch (key)
            {
                case "⌫":
                    if (live?.Backspace(text, text.Length) is { } undone)
                    {
                        text = undone.Text;
                    }
                    else if (text.Length > 0)
                    {
                        // One character: a surrogate pair goes as a whole.
                        var last = text.Length >= 2 && char.IsSurrogatePair(text[^2], text[^1]) ? 2 : 1;
                        text = text[..^last];
                    }
                    live = null;
                    break;
                case "→":
                    live = null;
                    break;
                default:
                    if (Replace(text, key, live) is { } swap)
                    {
                        text = swap.Apply(text, string.Empty).Text;
                        live = swap;
                    }
                    else
                    {
                        text += key;
                        live = null;
                    }
                    break;
            }
        }
        return text;
    }

    private static string? EmoticonOf(string text)
    {
        foreach (var (typed, emoji) in Emoticons)
        {
            if (string.Equals(typed, text, StringComparison.Ordinal))
            {
                return emoji;
            }
        }
        return null;
    }

    /// <summary>The emoji, and the space after it for an emoticon that ends in one.</summary>
    private static string PutFor(string text, string emoji) => text.EndsWith(' ') ? emoji + " " : emoji;

    /// <summary>Rule 4's guard: what may stand before <paramref name="emoticon"/>. A trailing surrogate is never ASCII, so the last UTF-16 unit decides.</summary>
    private static bool GuardPasses(string head, string emoticon)
    {
        if (head.Length == 0)
        {
            return true;
        }
        var previous = head[^1];
        return emoticon[0] is ':' or ';' ? !char.IsAsciiDigit(previous) : !char.IsAsciiLetterOrDigit(previous);
    }

    private static FrozenDictionary<string, string> LoadNames()
    {
        using var stream = typeof(EmojiTyping).Assembly.GetManifestResourceStream(NamesResource)
            ?? throw new InvalidOperationException($"the embedded resource {NamesResource} is missing");
        using var reader = new StreamReader(stream, Encoding.UTF8);
        var names = new Dictionary<string, string>(StringComparer.Ordinal);
        while (reader.ReadLine() is { } line)
        {
            if (line.Length == 0 || line[0] == '#')
            {
                continue;
            }
            var tab = line.IndexOf('\t');
            if (tab >= 0)
            {
                names[line[..tab]] = line[(tab + 1)..];
            }
        }
        return names.ToFrozenDictionary(StringComparer.Ordinal);
    }
}

/// <summary>
/// A replacement the rule just made (<see cref="EmojiTyping.Replace"/>), kept so the Backspace pressed straight after it can take
/// it back — and so the next key can grow it, <c>:(</c> and another <c>(</c> being <c>:((</c>. A field keeps one from the moment
/// the rule replaces until anything else happens — a key the rule does not act on, a click, a paste, a selection — and then
/// drops it, exactly like #97's capital.
/// </summary>
/// <param name="At">Where the replacement starts, in UTF-16 units.</param>
/// <param name="Put">What was put in: the emoji, and the space after it for the emoticons that end in one.</param>
/// <param name="Typed">What was typed, all of it: what a Backspace puts back.</param>
public sealed record EmojiSwap(int At, string Put, string Typed)
{
    /// <summary>
    /// The field's text and caret after the swap, given what was before the caret (without the typed character) and what is
    /// after it.
    /// </summary>
    public (string Text, int Caret) Apply(string before, string after)
    {
        var head = string.Concat(before.AsSpan(0, At), Put);
        return (head + after, head.Length);
    }

    /// <summary>
    /// A Backspace with the caret at <paramref name="caret"/> and nothing selected: the text with what was typed put back, and
    /// the caret right behind it — or null when the caret is not right behind the replacement or the replacement is no longer
    /// there, and the Backspace is an ordinary one.
    /// </summary>
    public (string Text, int Caret)? Backspace(string text, int caret)
    {
        var end = At + Put.Length;
        if (caret != end || At < 0 || end > text.Length || string.CompareOrdinal(text, At, Put, 0, Put.Length) != 0)
        {
            return null;
        }
        return (string.Concat(text.AsSpan(0, At), Typed, text.AsSpan(end)), At + Typed.Length);
    }
}
