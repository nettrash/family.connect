namespace FamilyConnect.Core;

/// <summary>What a paste means (<c>fc_text::media::paste_decision</c>, ios <c>ClipboardAttachment.decide</c>).</summary>
public enum PasteDecision
{
    /// <summary>Stage what the clipboard holds.</summary>
    Attach,

    /// <summary>Type its words into the draft.</summary>
    Type,

    /// <summary>Nothing worth a message.</summary>
    Nothing,
}

/// <summary>
/// A paste into the composer: whether it is words or files, which of a clipboard's many types a pasted item is taken as,
/// and what it is called — the web client's rules, held to them by the chat oracle.
/// </summary>
public static class PasteRules
{
    /// <summary>
    /// The types a pasted item is taken as, most wanted first — GIF and WebP ahead of the flattened PNG copied beside them,
    /// so an animation is not pasted as a still of itself.
    /// </summary>
    public static readonly IReadOnlyList<string> Preference =
    [
        "image/gif", "image/webp", "image/heic", "image/heif", "image/png", "image/jpeg", "image/bmp", "image/tiff",
        "video/mp4", "video/quicktime", "audio/mp4", "audio/mpeg", "audio/wav", "application/pdf",
    ];

    /// <summary>
    /// THE rule: WORDS WIN — an ordinary text paste must stay one, even from an app that puts a picture of the selection
    /// beside the words — except for copied FILES, which bring their own names along as text, one to a line, and taking the
    /// names instead of the files would make the feature useless exactly where it is most wanted.
    /// </summary>
    public static PasteDecision Decision(IReadOnlyList<string> fileNames, string text)
    {
        var words = text.Trim();
        bool OnlyTheirNames() => Lines(words)
            .Select(line => line.Trim())
            .Where(line => line.Length > 0)
            .All(line => fileNames.Any(name => string.Equals(line, name, StringComparison.Ordinal)));
        if (fileNames.Count > 0 && (words.Length == 0 || OnlyTheirNames()))
        {
            return PasteDecision.Attach;
        }
        return words.Length > 0 ? PasteDecision.Type : PasteDecision.Nothing;
    }

    /// <summary>The type to take out of everything a clipboard item offers: the most wanted one, or any that is not words.</summary>
    public static string? ChosenType(IReadOnlyList<string> offered) =>
        Preference.FirstOrDefault(wanted => offered.Contains(wanted, StringComparer.Ordinal))
        ?? offered.FirstOrDefault(offer => !offer.StartsWith("text/", StringComparison.Ordinal) && offer.Contains('/'));

    /// <summary>What a pasted item is called — never a scratch name. The same English words on every client.</summary>
    public static string PastedName(string mime)
    {
        var extension = mime switch
        {
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/heic" => "heic",
            "image/heif" => "heif",
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/bmp" => "bmp",
            "image/tiff" => "tiff",
            "video/mp4" => "mp4",
            "video/quicktime" => "mov",
            "audio/mp4" => "m4a",
            "audio/mpeg" => "mp3",
            "audio/wav" => "wav",
            "application/pdf" => "pdf",
            _ => "dat",
        };
        var what = mime.Split('/')[0] switch
        {
            "image" => "image",
            "video" => "video",
            "audio" => "audio",
            _ => "item",
        };
        return $"Pasted {what}.{extension}";
    }

    /// <summary>Rust's <c>str::lines</c>: split at each line feed, a carriage return before it dropped.</summary>
    private static IEnumerable<string> Lines(string text) =>
        text.Split('\n').Select(line => line.EndsWith('\r') ? line[..^1] : line);
}
