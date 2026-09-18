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

    /// <summary>
    /// Rust's <c>str::to_uppercase</c>: every scalar's full uppercase mapping, with no context — read from the table the
    /// Rust standard library itself printed (<see cref="UnicodeProperties.ToUppercase"/>), never from this platform's casing,
    /// which is a different Unicode version on macOS, Linux and Windows and failed CI for the letters Unicode 16 added.
    /// </summary>
    public static string Uppercase(string text)
    {
        var upper = new StringBuilder(text.Length);
        foreach (var rune in text.EnumerateRunes())
        {
            foreach (var scalar in RustChar.Uppercase(rune.Value))
            {
                upper.Append(char.ConvertFromUtf32(scalar));
            }
        }
        return upper.ToString();
    }
}
