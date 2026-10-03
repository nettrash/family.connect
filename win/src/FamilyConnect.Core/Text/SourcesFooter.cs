using System.Text.RegularExpressions;

namespace FamilyConnect.Core;

/// <summary>
/// The footer the SERVER appends to an assistant answer that looked something up (docs/protocol.md, "How sources are
/// shown: a footer the server writes"): one blank line after the answer, then a sources line — the word "Sources" in the
/// reply's language, a colon, and at most three <c>[title](url)</c> links separated by <c> · </c> — and a credit line
/// naming each provider whose result reached the model.
/// </summary>
/// <remarks>
/// <para>
/// It is plain markdown, so a bubble already draws it and its links already open; nothing here draws anything. What a
/// client needs to KNOW is only whether an answer carries one, because such an answer is a lookup answer, and every link
/// in it is either a returned source, a provider's credit, or the host of a returned source — the server removed the rest
/// before it added the footer. Those links are kept out of preview cards (design decision 7), so a cited page is not
/// contacted by every device that shows the answer.
/// </para>
/// <para>
/// Recognised by SHAPE and by the words the server writes, never by "it contains a link": the sources word is one of the
/// nine languages' (eight distinct words — French and English share one), and the credit items are the three fixed ones,
/// matched by what is NOT translated — Open-Meteo's link, Wikipedia's licence link, and Brave's own words. A footer of the
/// credit line alone (a weather-only answer) and of the sources line alone (a SearXNG answer, which credits nobody) are
/// both footers.
/// </para>
/// </remarks>
public static class SourcesFooter
{
    /// <summary>"Sources", as the server writes it in each language it ships (server/src/lookups.rs, <c>FooterWords</c>).</summary>
    public static readonly IReadOnlyList<string> SourcesWords =
        ["Sources", "Quellen", "Fuentes", "出典", "Источники", "Извори", "Izvori", "来源"];

    /// <summary>The most links the sources line carries (docs/protocol.md, "Limits": 3, fixed).</summary>
    public const int MaxSources = 3;

    public const string Separator = " · ";

    private static readonly TimeSpan Budget = TimeSpan.FromMilliseconds(250);

    private const string Link = @"\[[^\[\]\n]+\]\(https?://[^\s()]+\)";

    private static readonly Regex SourcesLine = new(
        "^(?:" + string.Join('|', SourcesWords.Select(Regex.Escape)) + "): " + Link
        + "(?:" + Regex.Escape(Separator) + Link + "){0," + (MaxSources - 1) + "}$",
        RegexOptions.CultureInvariant,
        Budget);

    private static readonly Regex[] Credits =
    [
        // "[Weather data by Open-Meteo.com](https://open-meteo.com/)", the words translated, the link not.
        new(@"^\[[^\[\]\n]+\]\(https://open-meteo\.com/\)$", RegexOptions.CultureInvariant, Budget),
        // "Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)", the name translated.
        new(@"^[^\[\]\n,]+, \[CC BY-SA 4\.0\]\(https://creativecommons\.org/licenses/by-sa/4\.0/\)$", RegexOptions.CultureInvariant, Budget),
        // Brave's own words, never translated.
        new(@"^Powered by Brave$", RegexOptions.CultureInvariant, Budget),
    ];

    /// <summary>Does this body END with the server's sources footer — its last paragraph, and nothing after it?</summary>
    public static bool EndsWithFooter(string? body)
    {
        if (string.IsNullOrEmpty(body))
        {
            return false;
        }
        var text = body.Replace("\r\n", "\n", StringComparison.Ordinal).TrimEnd();
        var gap = text.LastIndexOf("\n\n", StringComparison.Ordinal);
        var paragraph = gap < 0 ? text : text[(gap + 2)..];
        var lines = paragraph.Split('\n');
        try
        {
            return lines switch
            {
                [var only] => IsSourcesLine(only) || IsCreditLine(only),
                [var sources, var credits] => IsSourcesLine(sources) && IsCreditLine(credits),
                _ => false,
            };
        }
        catch (RegexMatchTimeoutException)
        {
            // Not a footer the server wrote, whatever it is.
            return false;
        }
    }

    private static bool IsSourcesLine(string line) => SourcesLine.IsMatch(line);

    private static bool IsCreditLine(string line)
    {
        var items = line.Split(Separator);
        if (items.Length > Credits.Length || items.Distinct(StringComparer.Ordinal).Count() != items.Length)
        {
            return false;
        }
        return items.All(item => Credits.Any(credit => credit.IsMatch(item)));
    }
}
