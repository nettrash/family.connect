using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

public sealed record ReactionSearchResult(string Emoji, string Name);

/// <summary>The shared #116 ranking and names for one display language.</summary>
public sealed class ReactionSearch
{
    private sealed record Annotation(string Name, string NormalisedName, string Terms);

    private const string ResourcePrefix = "FamilyConnect.App.Logic.ReactionSearch.";
    private static readonly string[] Catalogue = EmojiCatalog.Categories
        .SelectMany(category => category.Emoji)
        .Distinct(StringComparer.Ordinal)
        .ToArray();
    // The Rust index takes the last spelling when catalogue entries share a key.
    private static readonly Dictionary<string, string> EmojiByKey = Catalogue
        .GroupBy(Key, StringComparer.Ordinal)
        .ToDictionary(group => group.Key, group => group.Last(), StringComparer.Ordinal);
    private static readonly Lazy<ILookup<string, string>> Aliases = new(() => EmojiTyping.Shortcodes
        .Select(pair => (Emoji: CanonicalEmoji(pair.Value), Term: Normalise(pair.Key)))
        .Where(alias => alias.Emoji is not null)
        .ToLookup(alias => alias.Emoji!, alias => alias.Term, StringComparer.Ordinal));
    private static readonly Dictionary<string, Lazy<Dictionary<string, Annotation>?>> ByLanguage = Languages.All
        .ToDictionary(tag => tag, tag => new Lazy<Dictionary<string, Annotation>?>(() => ReadAnnotations(tag)),
            StringComparer.Ordinal);

    private readonly Dictionary<string, Annotation>? english;
    private readonly Dictionary<string, Annotation>? local;

    public ReactionSearch(string displayLanguage)
    {
        english = ByLanguage[Languages.English].Value;
        local = ByLanguage[Languages.ForDisplay(displayLanguage)].Value ?? english;
    }

    public ReactionSearchResult Describe(string emoji) =>
        new(emoji, local?.GetValueOrDefault(emoji)?.Name ?? emoji);

    public IReadOnlyList<ReactionSearchResult> Search(string text)
    {
        var query = Normalise(text);
        if (query.Length == 0)
        {
            return [];
        }

        var exactEmoji = CanonicalEmoji(query);
        var words = query.Split(' ');
        var exact = new List<ReactionSearchResult>();
        var prefixes = new List<ReactionSearchResult>();
        var keywords = new List<ReactionSearchResult>();
        // Walking the catalogue once keeps the original order within each rank.
        foreach (var emoji in Catalogue)
        {
            var localEntry = local?.GetValueOrDefault(emoji);
            var englishEntry = english?.GetValueOrDefault(emoji);
            var aliases = Aliases.Value[emoji];
            var result = new ReactionSearchResult(emoji, localEntry?.Name ?? emoji);
            if (emoji == exactEmoji || localEntry?.NormalisedName == query || englishEntry?.NormalisedName == query
                || aliases.Contains(query, StringComparer.Ordinal))
            {
                exact.Add(result);
            }
            else if (localEntry?.NormalisedName.StartsWith(query, StringComparison.Ordinal) == true
                || englishEntry?.NormalisedName.StartsWith(query, StringComparison.Ordinal) == true)
            {
                prefixes.Add(result);
            }
            else if (words.All(word =>
                localEntry?.Terms.Contains(word, StringComparison.Ordinal) == true
                || englishEntry?.Terms.Contains(word, StringComparison.Ordinal) == true
                || aliases.Any(alias => alias.Contains(word, StringComparison.Ordinal))))
            {
                keywords.Add(result);
            }
        }

        exact.AddRange(prefixes);
        exact.AddRange(keywords);
        return exact;
    }

    internal static string? CanonicalEmoji(string emoji) => EmojiByKey.GetValueOrDefault(Key(emoji));

    private static string Key(string emoji) => emoji.Replace("\uFE0F", "", StringComparison.Ordinal);

    private static string Normalise(string text) => string.Join(' ', text.ToLowerInvariant()
        .Replace("ß", "ss", StringComparison.Ordinal)
        .Replace('_', ' ').Replace('-', ' ').Replace(':', ' ').Replace('|', ' ')
        .Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries));

    private static Dictionary<string, Annotation>? ReadAnnotations(string tag)
    {
        using var stream = typeof(ReactionSearch).Assembly.GetManifestResourceStream($"{ResourcePrefix}{tag}.tsv");
        if (stream is null)
        {
            return null;
        }

        using var reader = new StreamReader(stream);
        var annotations = new Dictionary<string, Annotation>(StringComparer.Ordinal);
        while (reader.ReadLine() is { } line)
        {
            if (line.StartsWith('#'))
            {
                continue;
            }
            var fields = line.Split('\t');
            if (fields.Length != 4 || fields[0] != tag || CanonicalEmoji(fields[1]) != fields[1]
                || string.IsNullOrWhiteSpace(fields[2]) || string.IsNullOrWhiteSpace(fields[3])
                || !annotations.TryAdd(fields[1], new(fields[2], Normalise(fields[2]), Normalise($"{fields[2]} {fields[3]}"))))
            {
                return null;
            }
        }
        return annotations.Count == EmojiByKey.Count ? annotations : null;
    }
}
