using System.Text;

namespace FamilyConnect.Core;

/// <summary>
/// Rust's <c>char</c> as <c>fc_text</c> asks about it, over UTF-8 bytes: the standard library's own properties (generated
/// into <see cref="UnicodeProperties"/>), the one-scalar decoding the ported parsers walk by, and whether an ASCII markup
/// character is a grapheme cluster of its own.
/// </summary>
/// <remarks>
/// .NET's Unicode is a different version with different definitions — <c>Rune.IsLetter</c> is not the Alphabetic property
/// and <c>StringInfo</c> segments by its own tables — so the port never asks .NET. Scalars are <see cref="int"/>s, and
/// <c>'\n'</c> stands in wherever Rust's <c>unwrap_or('\n')</c> does.
/// </remarks>
internal static class RustChar
{
    public static bool IsAlphabetic(int c) => c < 0x80 ? IsAsciiLetter(c) : InRanges(UnicodeProperties.Alphabetic, c);

    public static bool IsNumeric(int c) => c < 0x80 ? c is >= '0' and <= '9' : InRanges(UnicodeProperties.Numeric, c);

    public static bool IsAlphanumeric(int c) => IsAlphabetic(c) || IsNumeric(c);

    public static bool IsLowercase(int c) => c < 0x80 ? c is >= 'a' and <= 'z' : InRanges(UnicodeProperties.Lowercase, c);

    public static bool IsUppercase(int c) => c < 0x80 ? c is >= 'A' and <= 'Z' : InRanges(UnicodeProperties.Uppercase, c);

    public static bool IsWhitespace(int c) => InRanges(UnicodeProperties.Whitespace, c);

    public static bool IsControl(int c) => InRanges(UnicodeProperties.Control, c);

    public static bool IsAsciiLetter(int c) => c is >= 'a' and <= 'z' or >= 'A' and <= 'Z';

    public static bool IsAsciiDigit(int c) => c is >= '0' and <= '9';

    public static bool IsAsciiAlphanumeric(int c) => IsAsciiLetter(c) || IsAsciiDigit(c);

    public static bool IsAsciiHexDigit(int c) => IsAsciiDigit(c) || c is >= 'a' and <= 'f' or >= 'A' and <= 'F';

    /// <summary><c>u8::is_ascii_punctuation</c>: <c>!"#$%&amp;'()*+,-./:;&lt;=&gt;?@[\]^_`{|}~</c>.</summary>
    public static bool IsAsciiPunctuation(int c) => c is >= 0x21 and <= 0x2F or >= 0x3A and <= 0x40 or >= 0x5B and <= 0x60 or >= 0x7B and <= 0x7E;

    /// <summary><c>char::to_lowercase</c>, as UTF-8.</summary>
    public static void AppendLowercase(List<byte> into, int c)
    {
        if (Find(UnicodeProperties.ToLowercase, c) is { } lower)
        {
            into.AddRange(Encoding.UTF8.GetBytes(lower));
        }
        else
        {
            AppendScalar(into, c);
        }
    }

    /// <summary><c>char::to_uppercase</c>, as the scalars it makes.</summary>
    public static IEnumerable<int> Uppercase(int c)
    {
        if (Find(UnicodeProperties.ToUppercase, c) is not { } upper)
        {
            return [c];
        }
        var scalars = new List<int>();
        foreach (var rune in upper.EnumerateRunes())
        {
            scalars.Add(rune.Value);
        }
        return scalars;
    }

    public static bool InRanges((int Low, int High)[] ranges, int value)
    {
        var (low, high) = (0, ranges.Length - 1);
        while (low <= high)
        {
            var middle = (low + high) >>> 1;
            if (ranges[middle].High < value)
            {
                low = middle + 1;
            }
            else if (ranges[middle].Low > value)
            {
                high = middle - 1;
            }
            else
            {
                return true;
            }
        }
        return false;
    }

    private static string? Find((int Scalar, string Mapped)[] table, int value)
    {
        var (low, high) = (0, table.Length - 1);
        while (low <= high)
        {
            var middle = (low + high) >>> 1;
            if (table[middle].Scalar < value)
            {
                low = middle + 1;
            }
            else if (table[middle].Scalar > value)
            {
                high = middle - 1;
            }
            else
            {
                return table[middle].Mapped;
            }
        }
        return null;
    }

    // ---- UTF-8 -------------------------------------------------------------------------------------------------

    public static bool IsContinuation(byte value) => value >> 6 == 2;

    /// <summary>The width of the (valid) sequence a lead byte starts.</summary>
    public static int Width(byte lead) => lead < 0x80 ? 1 : lead < 0xE0 ? 2 : lead < 0xF0 ? 3 : 4;

    /// <summary>The scalar a valid sequence starts at <paramref name="at"/>.</summary>
    public static int At(ReadOnlySpan<byte> text, int at)
    {
        Rune.DecodeFromUtf8(text[at..], out var rune, out _);
        return rune.Value;
    }

    /// <summary>The last scalar before <paramref name="at"/> in valid UTF-8, and where it starts.</summary>
    public static (int Scalar, int Start) Before(ReadOnlySpan<byte> text, int at)
    {
        var start = at - 1;
        while (start > 0 && IsContinuation(text[start]))
        {
            start--;
        }
        return (At(text, start), start);
    }

    /// <summary>
    /// fc_text's <c>decode_at</c>: the one sequence starting at <paramref name="at"/>, or null when <paramref name="at"/> is
    /// not the start of a valid one — cmark walks bytes, and a continuation byte does not decode.
    /// </summary>
    public static int? DecodeAt(ReadOnlySpan<byte> data, int at)
    {
        if (at >= data.Length)
        {
            return null;
        }
        var width = data[at] switch
        {
            <= 0x7F => 1,
            >= 0xC2 and <= 0xDF => 2,
            >= 0xE0 and <= 0xEF => 3,
            >= 0xF0 and <= 0xF4 => 4,
            _ => 0,
        };
        if (width == 0 || at + width > data.Length)
        {
            return null;
        }
        return Rune.DecodeFromUtf8(data.Slice(at, width), out var rune, out var consumed) == System.Buffers.OperationStatus.Done && consumed == width
            ? rune.Value
            : null;
    }

    public static void AppendScalar(List<byte> into, int c)
    {
        Span<byte> buffer = stackalloc byte[4];
        var length = new Rune(c).EncodeToUtf8(buffer);
        into.AddRange(buffer[..length]);
    }

    // ---- grapheme clusters -------------------------------------------------------------------------------------

    /// <summary>
    /// Whether the one-scalar ASCII character (or U+1FEF) at <paramref name="at"/> is a grapheme cluster of its own within
    /// <paramref name="text"/> — unicode-segmentation's answer. Nothing but these two decide it: no scalar before joins it
    /// unless it is a Prepend, and none after unless it is an Extend, a ZWJ or a SpacingMark. A tab is a control, which
    /// nothing joins.
    /// </summary>
    public static bool StandsAlone(ReadOnlySpan<byte> text, int at, int width)
    {
        if (text[at] == (byte)'\t')
        {
            return true;
        }
        if (at > 0 && InRanges(UnicodeProperties.GraphemePrepends, Before(text, at).Scalar))
        {
            return false;
        }
        return at + width >= text.Length || !InRanges(UnicodeProperties.GraphemeExtends, At(text, at + width));
    }

    /// <summary>Whether a scalar joins the cluster of an ASCII letter in front of it — a combining mark, in IDNA's words.</summary>
    public static bool JoinsTheCharacterBefore(int c) => InRanges(UnicodeProperties.GraphemeExtends, c);
}
