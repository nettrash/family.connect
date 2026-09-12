using System.Reflection;
using System.Text.Json;

namespace FamilyConnect.Core;

/// <summary>
/// The nine languages this app speaks, as the apps' own catalogue names them.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE FAMILY'S LANGUAGE IS NOT THE DISPLAY LANGUAGE, and that is deliberate.</b> A family may
/// declare the one language it speaks to each other in, and what it is FOR is the assistant's
/// answers in the family chat — nothing else. Clients go on drawing their interface in whatever
/// the DEVICE is set to, because "a family setting that silently re-languaged somebody's phone
/// would be a surprise nobody asked for" (docs/protocol.md, "The family's language"). So this
/// class takes a system language and the family's setting never reaches it.
/// </para>
/// <para>
/// Unset is also not English: a family that never chose is not a family that chose English, which
/// is why the wire's field is nullable and why nothing here defaults it.
/// </para>
/// </remarks>
public static class Languages
{
    /// <summary>English is not in the list of FILES: its keys are its values.</summary>
    public const string English = "en";

    /// <summary>Every tag the catalogue holds, English first.</summary>
    public static readonly string[] All =
        [English, "de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans"];

    /// <summary>
    /// What this WINDOW draws in: the device's language, and never the family's. The family's
    /// setting is a fact about the family that the assistant reads; it is not a request to
    /// re-language anybody's computer.
    /// </summary>
    public static string ForDisplay(string? systemLanguage) => Nearest(systemLanguage);

    /// <summary>
    /// The tag this app speaks for a system or family language, or English when it speaks none of
    /// it. A region is dropped (<c>de-AT</c> is German) and Serbian keeps its SCRIPT, because
    /// <c>sr</c> and <c>sr-Latn</c> are the same language in two alphabets and a reader of one
    /// cannot read the other.
    /// </summary>
    public static string Nearest(string? tag)
    {
        if (string.IsNullOrWhiteSpace(tag))
        {
            return English;
        }
        var wanted = tag.Trim().Replace('_', '-');
        var exact = All.FirstOrDefault(
            known => known.Equals(wanted, StringComparison.OrdinalIgnoreCase));
        if (exact is not null)
        {
            return exact;
        }
        // `sr-Latn-RS` is Latin Serbian; `sr-RS` and `sr-Cyrl-RS` are not.
        if (wanted.StartsWith("sr-Latn", StringComparison.OrdinalIgnoreCase))
        {
            return "sr-Latn";
        }
        if (wanted.StartsWith("zh", StringComparison.OrdinalIgnoreCase))
        {
            // The catalogue holds Simplified only; a Traditional reader gets it rather than
            // English, which is the nearer of the two wrong answers.
            return "zh-Hans";
        }
        var bare = wanted.Split('-')[0];
        return All.FirstOrDefault(known => known.Equals(bare, StringComparison.OrdinalIgnoreCase))
               ?? English;
    }
}

/// <summary>
/// One language's table, read from the catalogue that ships inside this assembly.
/// </summary>
/// <remarks>
/// <para>
/// <b>A MISSING KEY ANSWERS ITSELF, AND THAT IS THE DESIGN.</b> The key IS the English sentence,
/// so a string no translator has reached yet reads in English rather than as
/// <c>chat_list_preview_hidden</c> — which is what a client with slug keys shows its user when a
/// table is incomplete.
/// </para>
/// <para>
/// <b>A TRANSLATION MAY REORDER ITS PLACEHOLDERS.</b> German writes "%1$lld von %2$lld erledigt"
/// and Russian "Сделано %1$lld из %2$lld", so the formatter has to read positional forms as well
/// as sequential ones — <see cref="AppleFormat"/> does, and this is the layer that proves it with
/// the shipped values rather than with an invented sentence.
/// </para>
/// </remarks>
public sealed class JsonCatalog : IStringCatalog
{
    private readonly Dictionary<string, string> table;

    private JsonCatalog(Dictionary<string, string> table) => this.table = table;

    /// <summary>The tag this catalogue was loaded for.</summary>
    public string Language { get; private init; } = Languages.English;

    /// <summary>How many sentences it holds.</summary>
    public int Count => table.Count;

    /// <summary>Read one language's table out of a JSON object.</summary>
    public static JsonCatalog Parse(string json, string language = Languages.English)
    {
        var table = JsonSerializer.Deserialize<Dictionary<string, string>>(json)
                    ?? throw new ArgumentException("not a table of strings", nameof(json));
        return new JsonCatalog(table) { Language = language };
    }

    /// <summary>
    /// The catalogue that ships for this language. English has no file — its keys are its values —
    /// and so answers the empty table, which is exactly right.
    /// </summary>
    public static IStringCatalog For(string language)
    {
        var tag = Languages.Nearest(language);
        if (tag == Languages.English)
        {
            return EnglishCatalog.Instance;
        }
        var assembly = typeof(JsonCatalog).Assembly;
        var name = $"FamilyConnect.Core.Text.i18n.{tag}.json";
        using var stream = assembly.GetManifestResourceStream(name);
        if (stream is null)
        {
            // A language the build did not ship: English, rather than nothing.
            return EnglishCatalog.Instance;
        }
        using var reading = new StreamReader(stream);
        return Parse(reading.ReadToEnd(), tag);
    }

    /// <summary>Every tag whose table actually ships in this build.</summary>
    public static IReadOnlyList<string> Shipped()
    {
        var assembly = typeof(JsonCatalog).Assembly;
        return
        [
            .. Languages.All.Where(tag =>
                tag == Languages.English
                || assembly.GetManifestResourceStream(
                    $"FamilyConnect.Core.Text.i18n.{tag}.json") is not null),
        ];
    }

    /// <summary>
    /// Whether this language actually HOLDS a sentence — which is not the same question as
    /// whether <see cref="Get"/> answers something different from the key: "Video" in German and
    /// "Photo" in French are real translations that happen to be identical to the English, and a
    /// completeness check that compared the values would call them missing.
    /// </summary>
    public bool Holds(string key) => table.ContainsKey(key);

    public string Get(string key) => table.TryGetValue(key, out var said) ? said : key;

    public string Format(string key, params object[] arguments) =>
        AppleFormat.Apply(Get(key), arguments);
}
