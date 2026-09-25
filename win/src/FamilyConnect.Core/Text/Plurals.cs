using System.Text.Json;

namespace FamilyConnect.Core;

/// <summary>
/// Which CLDR plural category a count takes in each of the nine languages — the rules
/// <c>fc_text::i18n::Lang::plural_category</c> uses, held to it by <c>ChatOracleTests</c>.
/// </summary>
/// <remarks>
/// Russian and both Serbians have THREE forms (one, few, many), French counts zero as one, and
/// Japanese and Chinese say a sentence the same way whatever the count. English is one and other,
/// and "1 questions to the assistant" is the sentence a catalogue without this says.
/// </remarks>
public static class PluralRules
{
    public static string Category(string language, long count)
    {
        var n = count == long.MinValue ? (ulong)long.MaxValue + 1 : (ulong)Math.Abs(count);
        return Languages.Nearest(language) switch
        {
            "ja" or "zh-Hans" => "other",
            "fr" => n <= 1 ? "one" : "other",
            "ru" or "sr" or "sr-Latn" => Slavic(n),
            _ => n == 1 ? "one" : "other",
        };
    }

    private static string Slavic(ulong n)
    {
        var (ten, hundred) = (n % 10, n % 100);
        if (ten == 1 && hundred != 11)
        {
            return "one";
        }
        return ten is >= 2 and <= 4 && hundred is not (>= 12 and <= 14) ? "few" : "many";
    }
}

/// <summary>
/// The sentences said in FORMS, one per plural category, per language — generated into
/// <c>Text/i18n/plurals.json</c> by <c>win/i18n/generate.py</c> from the apps' plural variations
/// and <c>win/i18n/win.json</c>'s own English forms.
/// </summary>
public static class PluralTables
{
    private static readonly Lazy<Dictionary<string, Dictionary<string, Dictionary<string, string>>>> All =
        new(Load);

    /// <summary>A key's forms in this language, or null when it has none.</summary>
    public static IReadOnlyDictionary<string, string>? Forms(string language, string key) =>
        All.Value.TryGetValue(language, out var table) && table.TryGetValue(key, out var forms)
            ? forms
            : null;

    /// <summary>
    /// The form this language uses for <paramref name="count"/> — its own category, else "other"
    /// — or null when the key has no forms here.
    /// </summary>
    public static string? Form(string language, string key, long count) =>
        Forms(language, key) is { } forms ? Pick(forms, language, count) : null;

    /// <summary>
    /// One form out of a set: the count's own category, else "other" — a translator who wrote only
    /// "one" and "other" for a three-form language still has a sentence for five — else none.
    /// </summary>
    public static string? Pick(IReadOnlyDictionary<string, string> forms, string language, long count) =>
        forms.TryGetValue(PluralRules.Category(language, count), out var form)
            ? form
            : forms.TryGetValue("other", out var other) ? other : null;

    private static Dictionary<string, Dictionary<string, Dictionary<string, string>>> Load()
    {
        using var stream = typeof(PluralTables).Assembly
            .GetManifestResourceStream("FamilyConnect.Core.Text.i18n.plurals.json");
        if (stream is null)
        {
            return [];
        }
        return JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, Dictionary<string, string>>>>(stream)
               ?? [];
    }
}
