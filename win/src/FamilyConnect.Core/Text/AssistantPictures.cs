namespace FamilyConnect.Core;

/// <summary>One attachment, reduced to the three facts the picture rule reads.</summary>
/// <param name="Mime">What it will TRAVEL as — a preview is a JPEG by definition.</param>
/// <param name="Bytes">What it will travel as, in bytes; null when this client cannot know, and then the size rule goes unapplied rather than guessed at.</param>
public sealed record PictureCandidate(string Kind, string Mime, long? Bytes)
{
    /// <summary>
    /// An attachment on a message in the chat, usually somebody else's: with a preview it is judged as a JPEG of unknown size
    /// (the preview's length is not on the wire), without one by its own type and size.
    /// </summary>
    public static PictureCandidate OfAttachment(string kind, string mime, long? size, bool hasPreview) =>
        new(kind, AssistantPictures.WireMime(mime, hasPreview), hasPreview ? null : size);
}

/// <summary>The locks and switches the family chat's picture rule reads.</summary>
public sealed record PictureSwitches(
    bool ServerCanSee, bool FamilyAllows, bool FamilyHistory, bool FamilyHistoryPhotos, bool ServerCanDraw);

/// <summary>
/// The strip above the FAMILY composer while an <c>@ai</c> draft carries — or replies to — a photograph
/// (<c>fc_text::assistant_pictures::MentionNotice</c>, ios <c>MentionPictureNotice</c>).
/// </summary>
/// <param name="Extra">Past the shared budget: named, not shown.</param>
/// <param name="Unreadable">Cannot travel at all: told, never shown.</param>
/// <param name="RecentUpTo">How many of the chat's most recent photos may also go — null when the owner's recent photos switch is not in effect.</param>
public sealed record MentionPictureNotice(int ShownOnMention, int ShownOnQuote, int Extra, int Unreadable, int? RecentUpTo)
{
    public int Shown => ShownOnMention + ShownOnQuote;

    /// <summary>
    /// The notice for this draft, or null — for every reason the sentence would otherwise be a lie: a lock shut, no mention, a
    /// <c>/draw</c> request (which sends the words and nothing else), or no photograph at all.
    /// </summary>
    public static MentionPictureNotice? Of(
        string draft, IReadOnlyList<PictureCandidate> staged, IReadOnlyList<PictureCandidate> quoted, PictureSwitches switches)
    {
        if (!switches.ServerCanSee || !switches.FamilyAllows || !AssistantText.Mentions(draft))
        {
            return null;
        }
        if (switches.ServerCanDraw && AssistantText.AsksForPicture(draft))
        {
            return null;
        }
        var recentPhotos = switches.FamilyHistory && switches.FamilyHistoryPhotos;
        var onMention = staged.Where(candidate => candidate.Kind == "photo").ToList();
        var onQuote = quoted.Where(candidate => candidate.Kind == "photo").ToList();
        if (onMention.Count == 0 && onQuote.Count == 0 && !recentPhotos)
        {
            return null;
        }
        var carriedOnMention = AssistantPictures.Carried(onMention).Count;
        var carriedOnQuote = AssistantPictures.Carried(onQuote).Count;
        var shownOnMention = Math.Min(carriedOnMention, AssistantPictures.MaxPerQuestion);
        var shownOnQuote = Math.Min(carriedOnQuote, AssistantPictures.MaxPerQuestion - shownOnMention);
        return new MentionPictureNotice(
            shownOnMention,
            shownOnQuote,
            carriedOnMention - shownOnMention + (carriedOnQuote - shownOnQuote),
            onMention.Count - carriedOnMention + (onQuote.Count - carriedOnQuote),
            recentPhotos ? AssistantPictures.MaxPerQuestion - shownOnMention - shownOnQuote : null);
    }

    /// <summary>What the strip says.</summary>
    public string Sentence(IStringCatalog say)
    {
        var recent = RecentUpTo ?? 0;
        var token = AssistantText.Token;
        if (Unreadable > 0)
        {
            // Two sentences, each its own key: a translation must be free to order its words without reaching into the first.
            return recent == 0
                ? AssistantPictures.UnreadableSentence(say)
                : $"{AssistantPictures.UnreadableSentence(say)} {say.Plural("Up to %lld of the most recent photos in this chat may still go.", recent, recent)}";
        }
        if (Extra > 0)
        {
            return say.Plural(
                "Only the first %lld photos go to the model your server is set up to use — yours first, then the ones you're replying to. The rest are named to it, not shown.",
                AssistantPictures.MaxPerQuestion, AssistantPictures.MaxPerQuestion);
        }
        return (ShownOnMention > 0, ShownOnQuote > 0) switch
        {
            (true, false) when recent > 0 => say.Format(
                "This goes to the model your server is set up to use, with your %@ message, and up to %lld of the most recent photos in this chat may go too.",
                token, recent),
            (true, false) => say.Format(
                "This goes to the model your server is set up to use, with your %@ message. No other photo in this chat does.", token),
            (false, true) when recent > 0 => say.Format(
                "The photo you're replying to goes to the model your server is set up to use, with your %@ message, and up to %lld of the most recent photos in this chat may go too.",
                token, recent),
            (false, true) => say.Format(
                "The photo you're replying to goes to the model your server is set up to use, with your %@ message. No other photo in this chat does.",
                token),
            (true, true) when recent > 0 => say.Format(
                "This and the photo you're replying to go to the model your server is set up to use, with your %@ message, and up to %lld of the most recent photos in this chat may go too.",
                token, recent),
            (true, true) => say.Format(
                "This and the photo you're replying to go to the model your server is set up to use, with your %@ message. No other photo in this chat does.",
                token),
            // The count comes first in the key, so it is the first argument.
            (false, false) => say.Format(
                "Up to %lld of the most recent photos in this chat go to the model your server is set up to use, with your %@ message — pictures nobody pointed it at, whoever sent them.",
                recent, token),
        };
    }
}

/// <summary>
/// What a composer says before a photograph goes to the assistant, and the bounds that decide it
/// (<c>fc_text::assistant_pictures</c>, docs/protocol.md "Pictures"). The numbers are fixed by the protocol, never
/// configured and never on the wire — which is why a client may hold them: they decide a sentence somebody reads before
/// pixels leave their house.
/// </summary>
public static class AssistantPictures
{
    /// <summary>Photos off one question that reach the model — in the family chat, one budget across the mention and what it replies to.</summary>
    public const int MaxPerQuestion = HouseRules.MaxPicturesPerQuestion;

    /// <summary>The largest photo that travels, after the preview is preferred: 5 MiB.</summary>
    public const long MaxBytes = 5 * 1024 * 1024;

    /// <summary>The only two encodings a chat deployment reads.</summary>
    public static readonly IReadOnlyList<string> Accepted = ["image/jpeg", "image/png"];

    /// <summary>Would this be SHOWN to the model, or only named to it? Kind first: a video, a file, audio or a place never reach a model.</summary>
    public static bool IsShownToModel(string kind, string mime, long? bytes) =>
        kind == "photo"
        && Accepted.Contains(AsciiLower(mime), StringComparer.Ordinal)
        && (bytes is null || bytes <= MaxBytes);

    /// <summary>The type an attachment travels as: the preview when there is one.</summary>
    public static string WireMime(string mime, bool hasPreview) => hasPreview ? "image/jpeg" : mime;

    /// <summary>Everything that CAN travel, before the cap.</summary>
    public static IReadOnlyList<PictureCandidate> Carried(IEnumerable<PictureCandidate> candidates) =>
        [.. candidates.Where(candidate => IsShownToModel(candidate.Kind, candidate.Mime, candidate.Bytes))];

    /// <summary>May this composer offer "Show the Assistant a Photo…"? The assistant's own chat, a server that can see, and a family that allows it.</summary>
    public static bool OffersPictureAttach(bool assistantChat, bool serverCanSee, bool familyAllows) =>
        assistantChat && serverCanSee && familyAllows;

    /// <summary>The strip above the ASSISTANT'S chat composer while photos are staged — or null when none are.</summary>
    public static string? PrivateNotice(IReadOnlyList<PictureCandidate> staged, bool canSee, IStringCatalog say)
    {
        var photos = staged.Where(candidate => candidate.Kind == "photo").ToList();
        if (photos.Count == 0)
        {
            return null;
        }
        if (!canSee)
        {
            return say.Get("The assistant on this server can't look at pictures, so it will be told a photo is here but won't be shown it.");
        }
        if (Carried(photos).Count < photos.Count)
        {
            return UnreadableSentence(say);
        }
        if (photos.Count > MaxPerQuestion)
        {
            return say.Plural(
                "The first %lld photos go to the model your server is set up to use. The rest are named to it, not shown.",
                MaxPerQuestion, MaxPerQuestion);
        }
        return say.Get("This goes to the model your server is set up to use, with your question. Nothing else from this chat does.");
    }

    internal static string UnreadableSentence(IStringCatalog say) =>
        say.Get("A photo here is too large, or in a format the model can't read, so it will be told it's here but won't be shown it.");

    /// <summary>Rust's <c>to_ascii_lowercase</c>: A–Z and nothing else.</summary>
    private static string AsciiLower(string text) =>
        string.Create(text.Length, text, (span, source) =>
        {
            for (var at = 0; at < source.Length; at++)
            {
                span[at] = source[at] is >= 'A' and <= 'Z' ? (char)(source[at] + 32) : source[at];
            }
        });
}
