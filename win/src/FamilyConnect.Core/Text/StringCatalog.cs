using System.Globalization;
using System.Text;

namespace FamilyConnect.Core;

/// <summary>
/// Where a sentence comes from. THE KEY IS THE ENGLISH SENTENCE — the apps'
/// <c>Localizable.xcstrings</c> is the source of truth for all four clients, and its keys are the
/// English strings themselves ("Add to Calendar", "%lld of %lld done").
/// </summary>
/// <remarks>
/// Two consequences a port keeps hitting (the web client paid for both):
/// a translated string cannot live in a <c>const</c>, because it is not known until a language is
/// loaded; and a sentence matched by its beginning has to become a variant of its own rather than
/// a prefix test, because the translation may put the variable part first.
/// </remarks>
public interface IStringCatalog
{
    /// <summary>The sentence for this key, or the key itself when nothing is loaded for it.</summary>
    string Get(string key);

    /// <summary>The sentence with its placeholders filled, in Apple's grammar.</summary>
    string Format(string key, params object[] arguments);
}

/// <summary>
/// The English catalogue: every key answers itself. This is not a stub — English is the one
/// language whose values ARE its keys, so the app's resource layer only ever has to supply the
/// other eight.
/// </summary>
public sealed class EnglishCatalog : IStringCatalog
{
    public static readonly EnglishCatalog Instance = new();

    public string Get(string key) => key;

    public string Format(string key, params object[] arguments) =>
        AppleFormat.Apply(key, arguments);
}

/// <summary>
/// Apple's placeholder grammar, which the catalogue's values are written in.
/// </summary>
/// <remarks>
/// <para>
/// <c>%@</c> is a string and <c>%lld</c> a number, filled in order — but a TRANSLATION may need
/// them in another order, and then it writes them POSITIONALLY: <c>%1$@</c>, <c>%2$lld</c>. Both
/// forms appear in the shipped catalogue (the German and Serbian guest lines use the positional
/// one), so both have to work, and mixing them in one sentence has to work too.
/// </para>
/// <para>
/// <c>%%</c> is a literal per cent. Anything else beginning with <c>%</c> is copied through
/// untouched rather than swallowed: a sentence with a stray per cent is a sentence to show, not an
/// exception to throw at somebody reading their family's chat.
/// </para>
/// </remarks>
public static class AppleFormat
{
    public static string Apply(string pattern, params object[] arguments)
    {
        var text = new StringBuilder(pattern.Length + 16);
        var next = 0;
        var at = 0;
        while (at < pattern.Length)
        {
            var character = pattern[at];
            if (character != '%')
            {
                text.Append(character);
                at++;
                continue;
            }
            if (at + 1 < pattern.Length && pattern[at + 1] == '%')
            {
                text.Append('%');
                at += 2;
                continue;
            }
            var (taken, index) = ReadPlaceholder(pattern, at);
            if (taken == 0)
            {
                text.Append(character);
                at++;
                continue;
            }
            var which = index ?? next;
            if (index is null)
            {
                next++;
            }
            text.Append(Value(arguments, which));
            at += taken;
        }
        return text.ToString();
    }

    /// <summary>
    /// How many characters the placeholder at <paramref name="at"/> takes, and which argument it
    /// names when it names one. <c>(0, null)</c> when this is not a placeholder at all.
    /// </summary>
    private static (int Taken, int? Index) ReadPlaceholder(string pattern, int at)
    {
        var cursor = at + 1;
        int? index = null;
        var digits = cursor;
        while (digits < pattern.Length && char.IsAsciiDigit(pattern[digits]))
        {
            digits++;
        }
        if (digits > cursor && digits < pattern.Length && pattern[digits] == '$')
        {
            index = int.Parse(pattern.AsSpan(cursor, digits - cursor), CultureInfo.InvariantCulture) - 1;
            if (index < 0)
            {
                return (0, null);
            }
            cursor = digits + 1;
        }
        foreach (var conversion in Conversions)
        {
            if (pattern.AsSpan(cursor).StartsWith(conversion, StringComparison.Ordinal))
            {
                return (cursor - at + conversion.Length, index);
            }
        }
        return (0, null);
    }

    /// <summary>Longest first: <c>%lld</c> has to win over <c>%ld</c> and <c>%d</c>.</summary>
    private static readonly string[] Conversions = ["lld", "ld", "d", "@", "f", "s"];

    private static string Value(object[] arguments, int which) =>
        which >= 0 && which < arguments.Length
            ? Convert.ToString(arguments[which], CultureInfo.CurrentCulture) ?? string.Empty
            // A placeholder with no argument keeps its own text rather than vanishing: a sentence
            // missing a name reads as a bug, where a sentence with a hole in it reads as nothing.
            : string.Empty;
}
