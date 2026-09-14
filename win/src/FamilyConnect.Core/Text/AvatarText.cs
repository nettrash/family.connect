using System.Globalization;
using System.Text;

namespace FamilyConnect.Core;

/// <summary>
/// What a profile circle says when there is no picture: the initials of the name it stands for
/// (<c>fc_text::avatar::initials</c>, ios <c>InitialsAvatar</c>).
/// </summary>
/// <remarks>
/// The first letter of each of the first two words — words split on the SPACE character alone, empty runs dropped
/// — where a letter is a whole grapheme, so a name starting with an emoji or a letter and its combining mark keeps
/// all of it. Uppercased the way Rust uppercases: the FULL mapping, so <c>ß</c> is <c>SS</c> and a ligature is its
/// capitals, which .NET's invariant uppercase (one scalar for one) is not. A name with nothing in it is "?".
/// </remarks>
public static class AvatarText
{
    public static string Initials(string title)
    {
        var letters = new StringBuilder();
        var taken = 0;
        foreach (var word in title.Split(' '))
        {
            if (word.Length == 0)
            {
                continue;
            }
            letters.Append(StringInfo.GetNextTextElement(word));
            if (++taken == 2)
            {
                break;
            }
        }
        return letters.Length == 0 ? "?" : Uppercase(letters.ToString());
    }

    /// <summary>Rust's <c>str::to_uppercase</c>: every scalar's full uppercase mapping, with no context.</summary>
    public static string Uppercase(string text)
    {
        var upper = new StringBuilder(text.Length);
        foreach (var rune in text.EnumerateRunes())
        {
            if (Expanding.TryGetValue(rune.Value, out var many))
            {
                upper.Append(many);
            }
            else
            {
                upper.Append(Rune.ToUpperInvariant(rune).ToString());
            }
        }
        return upper.ToString();
    }

    /// <summary>
    /// SpecialCasing's unconditional uppercase mappings: the scalars whose capital is more than one scalar. Every
    /// other scalar is its simple mapping. The chat oracle holds this table to every scalar Rust uppercases.
    /// </summary>
    private static readonly Dictionary<int, string> Expanding = Build();

    private static Dictionary<int, string> Build()
    {
        static string S(params int[] scalars) => string.Concat(scalars.Select(char.ConvertFromUtf32));
        var table = new Dictionary<int, string>
        {
            [0x00DF] = S(0x53, 0x53),
            // Not an expansion, but .NET's invariant casing leaves the dotless i alone where Unicode — and Rust —
            // capitalise it to I.
            [0x0131] = S(0x49),
            [0x0149] = S(0x02BC, 0x4E),
            [0x01F0] = S(0x4A, 0x030C),
            [0x0390] = S(0x0399, 0x0308, 0x0301),
            [0x03B0] = S(0x03A5, 0x0308, 0x0301),
            [0x0587] = S(0x0535, 0x0552),
            [0x1E96] = S(0x48, 0x0331),
            [0x1E97] = S(0x54, 0x0308),
            [0x1E98] = S(0x57, 0x030A),
            [0x1E99] = S(0x59, 0x030A),
            [0x1E9A] = S(0x41, 0x02BE),
            [0x1F50] = S(0x03A5, 0x0313),
            [0x1F52] = S(0x03A5, 0x0313, 0x0300),
            [0x1F54] = S(0x03A5, 0x0313, 0x0301),
            [0x1F56] = S(0x03A5, 0x0313, 0x0342),
            [0x1FB2] = S(0x1FBA, 0x0399),
            [0x1FB3] = S(0x0391, 0x0399),
            [0x1FB4] = S(0x0386, 0x0399),
            [0x1FB6] = S(0x0391, 0x0342),
            [0x1FB7] = S(0x0391, 0x0342, 0x0399),
            [0x1FBC] = S(0x0391, 0x0399),
            [0x1FC2] = S(0x1FCA, 0x0399),
            [0x1FC3] = S(0x0397, 0x0399),
            [0x1FC4] = S(0x0389, 0x0399),
            [0x1FC6] = S(0x0397, 0x0342),
            [0x1FC7] = S(0x0397, 0x0342, 0x0399),
            [0x1FCC] = S(0x0397, 0x0399),
            [0x1FD2] = S(0x0399, 0x0308, 0x0300),
            [0x1FD3] = S(0x0399, 0x0308, 0x0301),
            [0x1FD6] = S(0x0399, 0x0342),
            [0x1FD7] = S(0x0399, 0x0308, 0x0342),
            [0x1FE2] = S(0x03A5, 0x0308, 0x0300),
            [0x1FE3] = S(0x03A5, 0x0308, 0x0301),
            [0x1FE4] = S(0x03A1, 0x0313),
            [0x1FE6] = S(0x03A5, 0x0342),
            [0x1FE7] = S(0x03A5, 0x0308, 0x0342),
            [0x1FF2] = S(0x1FFA, 0x0399),
            [0x1FF3] = S(0x03A9, 0x0399),
            [0x1FF4] = S(0x038F, 0x0399),
            [0x1FF6] = S(0x03A9, 0x0342),
            [0x1FF7] = S(0x03A9, 0x0342, 0x0399),
            [0x1FFC] = S(0x03A9, 0x0399),
            [0xFB00] = S(0x46, 0x46),
            [0xFB01] = S(0x46, 0x49),
            [0xFB02] = S(0x46, 0x4C),
            [0xFB03] = S(0x46, 0x46, 0x49),
            [0xFB04] = S(0x46, 0x46, 0x4C),
            [0xFB05] = S(0x53, 0x54),
            [0xFB06] = S(0x53, 0x54),
            [0xFB13] = S(0x0544, 0x0546),
            [0xFB14] = S(0x0544, 0x0535),
            [0xFB15] = S(0x0544, 0x053B),
            [0xFB16] = S(0x054E, 0x0546),
            [0xFB17] = S(0x0544, 0x053D),
        };
        // A Greek vowel with a subscript iota, lower or title case, is its capital and a capital iota.
        for (var at = 0; at < 8; at++)
        {
            table[0x1F80 + at] = table[0x1F88 + at] = S(0x1F08 + at, 0x0399);
            table[0x1F90 + at] = table[0x1F98 + at] = S(0x1F28 + at, 0x0399);
            table[0x1FA0 + at] = table[0x1FA8 + at] = S(0x1F68 + at, 0x0399);
        }
        return table;
    }
}
