namespace FamilyConnect.Core;

/// <summary>
/// Emoji the way the apps count them (<c>fc_text::emoji</c>, Swift <c>EmojiOnly</c>): whether a message is nothing
/// but emoji, and so draws them large, and how large.
/// </summary>
/// <remarks>
/// A pinned cross-platform contract, held to the original by the chat oracle. The whitespace it skips and the
/// ranges an emoji may open with are SPELLED OUT rather than taken from a platform predicate, as all four ports do:
/// the platforms disagree at the edges, and agreement by coincidence is not a contract.
/// </remarks>
public static class Emoji
{
    /// <summary>The size for one, two, three and four emoji — points on Apple, sp on Android — chosen against a 17-point body.</summary>
    public static readonly double[] FontLadder = [96, 80, 68, 56];

    /// <summary>The body text size the ladder was chosen against: an iPhone's.</summary>
    public const double LadderBodySize = 17;

    /// <summary>The size an emoji-only message draws at, or null to draw it as ordinary text — five or more, any text, or nothing.</summary>
    public static double? DisplayFontSize(string text) =>
        EmojiOnlyCount(text) is { } count && count is >= 1 and <= 4 ? FontLadder[count - 1] : null;

    /// <summary>
    /// <see cref="DisplayFontSize"/> scaled for a surface whose body is <paramref name="bodySize"/>, which keeps the
    /// proportion the ladder was designed for — the Mac's rule, generalised.
    /// </summary>
    public static double? DisplayFontSizeForBody(string text, double bodySize) =>
        DisplayFontSize(text) * (bodySize / LadderBodySize);

    /// <summary>
    /// How many emoji an emoji-only message carries, or null when it is not one. Whitespace between them is allowed and
    /// not counted; whitespace alone is no emoji message.
    /// </summary>
    public static int? EmojiOnlyCount(string text)
    {
        int[] scalars = [.. text.EnumerateRunes().Select(rune => rune.Value)];
        var index = 0;
        var count = 0;
        while (index < scalars.Length)
        {
            if (IsWhitespace(scalars[index]))
            {
                index++;
                continue;
            }
            var length = SequenceLength(scalars, index);
            if (length == 0)
            {
                return null;
            }
            index += length;
            count++;
        }
        return count > 0 ? count : null;
    }

    private const int Zwj = 0x200D;
    private const int TextSelector = 0xFE0E;
    private const int EmojiSelector = 0xFE0F;
    private const int CombiningKeycap = 0x20E3;

    private static bool IsWhitespace(int value) =>
        value is (>= 0x09 and <= 0x0D) or 0x20 or 0x85 or 0xA0 or 0x1680 or (>= 0x2000 and <= 0x200A)
            or 0x2028 or 0x2029 or 0x202F or 0x205F or 0x3000;

    /// <summary>The length in scalars of the one emoji sequence starting at <paramref name="start"/>, or 0 when what starts there is not emoji.</summary>
    private static int SequenceLength(int[] scalars, int start)
    {
        var value = scalars[start];

        // A flag is two regional indicators; a lone one still renders as an emoji letter, so it counts on its own.
        if (IsRegionalIndicator(value))
        {
            return start + 1 < scalars.Length && IsRegionalIndicator(scalars[start + 1]) ? 2 : 1;
        }

        // Keycaps: digits, # and * are ordinary text unless an enclosing mark follows.
        if (value is 0x23 or 0x2A or (>= 0x30 and <= 0x39))
        {
            var at = start + 1;
            var marked = false;
            if (at < scalars.Length && scalars[at] == EmojiSelector)
            {
                at++;
                marked = true;
            }
            if (at < scalars.Length && scalars[at] == CombiningKeycap)
            {
                at++;
                marked = true;
            }
            return marked ? at - start : 0;
        }

        if (!IsBase(value))
        {
            return 0;
        }

        var index = start + 1;
        while (index < scalars.Length)
        {
            var next = scalars[index];
            if (next == TextSelector)
            {
                // The author asked for the text glyph: the message is not an emoji message — all of it, not just this sequence.
                return 0;
            }
            if (next == EmojiSelector || next is >= 0x1F3FB and <= 0x1F3FF || next is >= 0xE0020 and <= 0xE007F)
            {
                index++;
                continue;
            }
            if (next == Zwj && index + 1 < scalars.Length && IsBase(scalars[index + 1]))
            {
                index += 2;
                continue;
            }
            break;
        }
        return index - start;
    }

    private static bool IsRegionalIndicator(int value) => value is >= 0x1F1E6 and <= 0x1F1FF;

    /// <summary>What can open an emoji sequence, and follow a joiner: Extended_Pictographic in broad strokes, range for range the Swift <c>baseRanges</c>.</summary>
    private static bool IsBase(int value) =>
        value is 0x00A9 or 0x00AE or 0x203C or 0x2049 or 0x2122 or 0x2139
            or (>= 0x2194 and <= 0x2199) or (>= 0x21A9 and <= 0x21AA) or (>= 0x231A and <= 0x231B) or 0x2328 or 0x23CF
            or (>= 0x23E9 and <= 0x23F3) or (>= 0x23F8 and <= 0x23FA) or 0x24C2 or (>= 0x25AA and <= 0x25AB) or 0x25B6
            or 0x25C0 or (>= 0x25FB and <= 0x25FE) or (>= 0x2600 and <= 0x27BF) or (>= 0x2934 and <= 0x2935)
            or (>= 0x2B05 and <= 0x2B07) or (>= 0x2B1B and <= 0x2B1C) or 0x2B50 or 0x2B55 or 0x3030 or 0x303D or 0x3297
            or 0x3299 or (>= 0x1F000 and <= 0x1F0FF) or (>= 0x1F170 and <= 0x1F171) or (>= 0x1F17E and <= 0x1F17F)
            or 0x1F18E or (>= 0x1F191 and <= 0x1F19A) or (>= 0x1F201 and <= 0x1F202) or 0x1F21A or 0x1F22F
            or (>= 0x1F232 and <= 0x1F23A) or (>= 0x1F250 and <= 0x1F251) or (>= 0x1F300 and <= 0x1FAFF);
}
