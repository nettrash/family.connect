using System.Text;

namespace FamilyConnect.Core;

/// <summary>The card under a message's first web link (ios <c>LinkPreview</c>, android <c>LinkPreview</c>).</summary>
/// <param name="Url">Where the page was read — past any redirect — and the key its image is kept under.</param>
/// <param name="SiteName">The publisher (<c>og:site_name</c>), falling back to the host without <c>www.</c>.</param>
public sealed record LinkPreview(Uri Url, string Title, string SiteName, string? Description, Uri? ImageUrl);

/// <summary>
/// How a page's HTML turns into a <see cref="LinkPreview"/> — a port of ios <c>LinkPreviewParser</c>, which Android
/// transcribes; the Swift tests are the spec, and <c>LinkPreviewParserTests</c> carries them.
/// </summary>
/// <remarks>
/// <para>
/// A small, tolerant scanner, not an HTML parser: the Open Graph tags, then Twitter cards, then plain <c>&lt;title&gt;</c>
/// and <c>&lt;meta name="description"&gt;</c>. It works on the string with integer indices and folds case over ASCII only —
/// matching on a Unicode-lowercased COPY and slicing the original with its indices corrupts titles, because lowercasing is
/// not length-preserving (İ, ẞ, the Kelvin sign). Tag and attribute names are ASCII, so ASCII folding is enough.
/// </para>
/// <para>
/// PRIVACY: building one means this device contacts the linked site. That is the trade the feature makes, and why Settings
/// can switch it off.
/// </para>
/// </remarks>
public static class LinkPreviewParser
{
    /// <summary>
    /// Longest prefix scanned — the SAME number as the page byte cap, because a UTF-8 page never decodes to more characters
    /// than it has bytes, so the parser always sees everything that was downloaded. YouTube's og:title sits at ~706K (#50).
    /// </summary>
    public const int ScanLimit = 1_048_576;

    /// <summary>Both limits count UNICODE SCALARS, as on the other platforms, so a surrogate pair is never split.</summary>
    public const int MaxTitleLength = 140;

    public const int MaxDescriptionLength = 300;

    /// <summary>Longest entity decoded, <c>&amp;</c> and <c>;</c> excluded — bounds the lookahead, so stray ampersands stay linear.</summary>
    private const int MaxEntityLength = 10;

    /// <summary>A preview out of <paramref name="html"/> read at <paramref name="pageUrl"/>, or null when the page offers no title.</summary>
    public static LinkPreview? Parse(string html, Uri pageUrl)
    {
        var head = html.Length > ScanLimit ? html[..ScanLimit] : html;
        var metas = MetaTags(head);

        if (FirstNonEmpty(Meta(metas, "og:title"), Meta(metas, "twitter:title"), TitleTag(head)) is not { } title)
        {
            return null;
        }
        var description = FirstNonEmpty(Meta(metas, "og:description"), Meta(metas, "twitter:description"), Meta(metas, "description"));
        var siteName = FirstNonEmpty(Meta(metas, "og:site_name")) ?? DisplayHost(pageUrl);
        var image = FirstNonEmpty(Meta(metas, "og:image"), Meta(metas, "og:image:url"), Meta(metas, "twitter:image")) is { } raw
            ? AbsoluteUrl(raw, pageUrl)
            : null;
        return new LinkPreview(
            pageUrl,
            Clamp(title, MaxTitleLength),
            siteName,
            description is null ? null : Clamp(description, MaxDescriptionLength),
            image);
    }

    /// <summary>The host as a card shows it, without a leading <c>www.</c>.</summary>
    public static string DisplayHost(Uri url)
    {
        var host = url.IsAbsoluteUri && url.Host.Length > 0 ? url.Host : url.OriginalString;
        return host.StartsWith("www.", StringComparison.Ordinal) ? host[4..] : host;
    }

    private static string? Meta(Dictionary<string, string> metas, string key) => metas.GetValueOrDefault(key);

    // ---- scanning ------------------------------------------------------------------------------------------------

    /// <summary>Every <c>&lt;meta&gt;</c>'s key (its <c>property</c> or <c>name</c>, lowercased) → content. The FIRST occurrence wins.</summary>
    internal static Dictionary<string, string> MetaTags(string html)
    {
        var result = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var tag in Tags("meta", html))
        {
            var attributes = Attributes(tag);
            if ((attributes.GetValueOrDefault("property") ?? attributes.GetValueOrDefault("name")) is not { } key
                || attributes.GetValueOrDefault("content") is not { } content)
            {
                continue;
            }
            result.TryAdd(AsciiLowercase(key), CollapseWhitespace(DecodeEntities(content)));
        }
        return result;
    }

    /// <summary>The text of the first <c>&lt;title&gt;</c> element.</summary>
    internal static string? TitleTag(string html)
    {
        var open = IndexOf("<title", html, 0);
        if (open < 0 || !IsTagNameBoundary(html, open + 6))
        {
            return null;
        }
        var contentStart = EndOfTag(html, open + 6);
        if (contentStart < 0)
        {
            return null;
        }
        var close = IndexOf("</title", html, contentStart + 1);
        if (close < 0)
        {
            return null;
        }
        var text = CollapseWhitespace(DecodeEntities(html[(contentStart + 1)..close]));
        return text.Length == 0 ? null : text;
    }

    /// <summary>The bodies of every <c>&lt;name …&gt;</c> tag: after the name, before the closing <c>&gt;</c>.</summary>
    private static List<string> Tags(string name, string html)
    {
        var tags = new List<string>();
        var opening = "<" + name;
        var index = 0;
        while (index < html.Length)
        {
            var open = IndexOf(opening, html, index);
            if (open < 0)
            {
                break;
            }
            var afterName = open + opening.Length;
            if (!IsTagNameBoundary(html, afterName))
            {
                index = afterName;
                continue;
            }
            var end = EndOfTag(html, afterName);
            if (end < 0)
            {
                break;
            }
            tags.Add(html[afterName..end]);
            index = end + 1;
        }
        return tags;
    }

    /// <summary>Whether the tag name really ends here — <c>&lt;metadata</c> is not <c>&lt;meta</c>.</summary>
    private static bool IsTagNameBoundary(string html, int index) => index >= html.Length || !char.IsLetterOrDigit(html[index]);

    /// <summary>The <c>&gt;</c> closing a tag whose body starts at <paramref name="start"/>, skipping one inside a quoted value; -1 when none.</summary>
    private static int EndOfTag(string html, int start)
    {
        char? quote = null;
        for (var index = start; index < html.Length; index++)
        {
            var character = html[index];
            if (quote is { } open)
            {
                if (character == open)
                {
                    quote = null;
                }
            }
            else if (character is '"' or '\'')
            {
                quote = character;
            }
            else if (character == '>')
            {
                return index;
            }
        }
        return -1;
    }

    /// <summary>The first offset at or after <paramref name="from"/> matching an already-lowercase needle, ASCII-case-insensitively.</summary>
    private static int IndexOf(string needle, string html, int from)
    {
        if (needle.Length == 0 || html.Length < needle.Length)
        {
            return -1;
        }
        for (var start = Math.Max(0, from); start <= html.Length - needle.Length; start++)
        {
            var offset = 0;
            while (offset < needle.Length && AsciiLowercase(html[start + offset]) == needle[offset])
            {
                offset++;
            }
            if (offset == needle.Length)
            {
                return start;
            }
        }
        return -1;
    }

    /// <summary>One tag body's attributes, names lowercased: single, double or no quotes, in any order; the first of a name wins.</summary>
    internal static Dictionary<string, string> Attributes(string tag)
    {
        var result = new Dictionary<string, string>(StringComparer.Ordinal);
        var index = 0;

        void SkipWhitespace()
        {
            while (index < tag.Length && char.IsWhiteSpace(tag[index]))
            {
                index++;
            }
        }

        while (index < tag.Length)
        {
            SkipWhitespace();
            var name = new StringBuilder();
            while (index < tag.Length && !char.IsWhiteSpace(tag[index]) && tag[index] != '=' && tag[index] != '/')
            {
                name.Append(tag[index]);
                index++;
            }
            SkipWhitespace();
            if (index >= tag.Length || tag[index] != '=')
            {
                // A valueless attribute: skip a stray "/" and carry on.
                if (index < tag.Length && tag[index] == '/')
                {
                    index++;
                }
                if (name.Length == 0 && index < tag.Length)
                {
                    index++;
                }
                continue;
            }
            index++;
            SkipWhitespace();
            var value = new StringBuilder();
            if (index < tag.Length && tag[index] is '"' or '\'')
            {
                var quote = tag[index];
                index++;
                while (index < tag.Length && tag[index] != quote)
                {
                    value.Append(tag[index]);
                    index++;
                }
                if (index < tag.Length)
                {
                    index++;
                }
            }
            else
            {
                while (index < tag.Length && !char.IsWhiteSpace(tag[index]))
                {
                    value.Append(tag[index]);
                    index++;
                }
            }
            if (name.Length > 0)
            {
                result.TryAdd(AsciiLowercase(name.ToString()), value.ToString());
            }
        }
        return result;
    }

    // ---- text ------------------------------------------------------------------------------------------------------

    /// <summary>ASCII-only lowercasing: length-preserving by construction.</summary>
    internal static string AsciiLowercase(string text) => string.Create(text.Length, text, (span, source) =>
    {
        for (var index = 0; index < source.Length; index++)
        {
            span[index] = AsciiLowercase(source[index]);
        }
    });

    private static char AsciiLowercase(char character) => character is >= 'A' and <= 'Z' ? (char)(character + 32) : character;

    /// <summary>The named and numeric entities that show up in titles and descriptions; the lookahead for <c>;</c> is bounded.</summary>
    public static string DecodeEntities(string text)
    {
        if (!text.Contains('&'))
        {
            return text;
        }
        var output = new StringBuilder(text.Length);
        var index = 0;
        while (index < text.Length)
        {
            if (text[index] != '&')
            {
                output.Append(text[index]);
                index++;
                continue;
            }
            var semicolon = -1;
            var limit = Math.Min(text.Length, index + MaxEntityLength + 2);
            for (var lookahead = index + 1; lookahead < limit; lookahead++)
            {
                if (text[lookahead] == ';')
                {
                    semicolon = lookahead;
                    break;
                }
            }
            if (semicolon < 0 || Replacement(text[(index + 1)..semicolon]) is not { } replacement)
            {
                output.Append(text[index]);
                index++;
                continue;
            }
            output.Append(replacement);
            index = semicolon + 1;
        }
        return output.ToString();
    }

    private static string? Replacement(string entity)
    {
        switch (AsciiLowercase(entity))
        {
            case "amp":
                return "&";
            case "lt":
                return "<";
            case "gt":
                return ">";
            case "quot":
                return "\"";
            case "apos" or "#39":
                return "'";
            case "nbsp":
                return " ";
            case "hellip":
                return "…";
            case "mdash":
                return "—";
            case "ndash":
                return "–";
            case "rsquo" or "#8217":
                return "’";
            case "lsquo":
                return "‘";
            case "ldquo":
                return "“";
            case "rdquo":
                return "”";
        }
        if (!entity.StartsWith('#'))
        {
            return null;
        }
        var digits = entity[1..];
        var hex = digits.StartsWith('x') || digits.StartsWith('X');
        var number = hex ? digits[1..] : digits;
        // Digits only, as Swift's `UInt32(_:radix:)` takes them: no sign, no spaces, nothing empty.
        if (number.Length == 0 || !number.All(c => hex ? char.IsAsciiHexDigit(c) : char.IsAsciiDigit(c))
            || !uint.TryParse(number, hex ? System.Globalization.NumberStyles.AllowHexSpecifier : System.Globalization.NumberStyles.None,
                System.Globalization.CultureInfo.InvariantCulture, out var value)
            || !Rune.IsValid(value))
        {
            return null;
        }
        return new Rune(value).ToString();
    }

    /// <summary>Runs of whitespace — the newlines inside a wrapped tag, a decoded NBSP — collapsed to single spaces, and trimmed.</summary>
    internal static string CollapseWhitespace(string text)
    {
        var output = new StringBuilder(text.Length);
        var pendingSpace = false;
        foreach (var character in text)
        {
            if (char.IsWhiteSpace(character))
            {
                pendingSpace = output.Length > 0;
                continue;
            }
            if (pendingSpace)
            {
                output.Append(' ');
                pendingSpace = false;
            }
            output.Append(character);
        }
        return output.ToString();
    }

    private static string? FirstNonEmpty(params string?[] candidates)
    {
        foreach (var candidate in candidates)
        {
            if (candidate?.Trim() is { Length: > 0 } trimmed)
            {
                return trimmed;
            }
        }
        return null;
    }

    /// <summary>Clamped to <paramref name="limit"/> scalars, an ellipsis after — never half a surrogate pair.</summary>
    private static string Clamp(string text, int limit)
    {
        var scalars = 0;
        for (var index = 0; index < text.Length; index += char.IsSurrogatePair(text, index) ? 2 : 1)
        {
            if (scalars == limit)
            {
                return text[..index].TrimEnd() + "…";
            }
            scalars++;
        }
        return text;
    }

    /// <summary>The absolute form of a possibly-relative URL in the page — http or https only, never file: or a custom scheme.</summary>
    internal static Uri? AbsoluteUrl(string raw, Uri page)
    {
        var trimmed = raw.Trim();
        if (trimmed.Length == 0)
        {
            return null;
        }
        Uri? resolved;
        if (trimmed.StartsWith("//", StringComparison.Ordinal))
        {
            Uri.TryCreate($"{page.Scheme}:{trimmed}", UriKind.Absolute, out resolved);
        }
        else
        {
            Uri.TryCreate(page, trimmed, out resolved);
        }
        return resolved is { IsAbsoluteUri: true } && resolved.Scheme is "http" or "https" ? resolved : null;
    }
}

/// <summary>
/// Spots the end of a page's <c>&lt;head&gt;</c> in a byte stream, one byte at a time, so a fetch can stop there
/// (ios <c>HeadEndDetector</c>, android <c>HeadEndScanner</c>). ASCII folding, and a match only where the tag NAME ends —
/// <c>&lt;bodyguard&gt;</c> is not the body. <c>&lt;body</c> counts too, because the head end tag is optional.
/// </summary>
public sealed class HeadEndDetector
{
    private const int Window = 7;
    private static readonly byte[] Head = "</head"u8.ToArray();
    private static readonly byte[] Body = "<body"u8.ToArray();
    private readonly byte[] window = new byte[Window];
    private int filled;

    /// <summary>True the moment <paramref name="value"/> completes <c>&lt;/head</c> or <c>&lt;body</c> — it is the byte after the name.</summary>
    public bool Consume(byte value)
    {
        var folded = value is >= (byte)'A' and <= (byte)'Z' ? (byte)(value + 32) : value;
        if (filled == Window)
        {
            Array.Copy(window, 1, window, 0, Window - 1);
            window[Window - 1] = folded;
        }
        else
        {
            window[filled++] = folded;
        }
        var boundary = window[filled - 1];
        if (boundary is >= (byte)'a' and <= (byte)'z' or >= (byte)'0' and <= (byte)'9')
        {
            return false;
        }
        return EndsWith(Head) || EndsWith(Body);
    }

    private bool EndsWith(byte[] needle)
    {
        var end = filled - 1;
        return end >= needle.Length && window.AsSpan(end - needle.Length, needle.Length).SequenceEqual(needle);
    }
}

/// <summary>How much of a linked page and its image a preview reads, and how the page's bytes become text.</summary>
public static class PageReader
{
    /// <summary>
    /// The page ceiling. Rarely reached — the read stops at the end of <c>&lt;head&gt;</c> — and 1 MB because YouTube's
    /// og: tags sit ~706K in, behind inline player JSON (#50). Equal to <see cref="LinkPreviewParser.ScanLimit"/>.
    /// </summary>
    public const int MaxPageBytes = 1024 * 1024;

    public const int MaxImageBytes = 4 * 1024 * 1024;

    static PageReader() => Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);

    /// <summary>
    /// Up to <paramref name="cap"/> bytes — and for a page, up to and including the byte that ends its head. The rest is never
    /// read, so the caller's disposal cancels its transfer.
    /// </summary>
    public static async Task<byte[]> ReadAsync(Stream stream, int cap, bool stoppingAtEndOfHead, CancellationToken ct)
    {
        using var output = new MemoryStream(Math.Min(cap, 64 * 1024));
        var detector = stoppingAtEndOfHead ? new HeadEndDetector() : null;
        var buffer = new byte[16 * 1024];
        while (output.Length < cap)
        {
            var read = await stream.ReadAsync(buffer.AsMemory(0, (int)Math.Min(buffer.Length, cap - output.Length)), ct).ConfigureAwait(false);
            if (read <= 0)
            {
                break;
            }
            if (detector is not null)
            {
                for (var index = 0; index < read; index++)
                {
                    if (detector.Consume(buffer[index]))
                    {
                        output.Write(buffer, 0, index + 1);
                        return output.ToArray();
                    }
                }
            }
            output.Write(buffer, 0, read);
        }
        return output.ToArray();
    }

    /// <summary>The page as text in the charset its response declared, or UTF-8 when it declared none or one unknown here.</summary>
    public static string Decode(byte[] bytes, string? charset)
    {
        var encoding = Encoding.UTF8;
        if (charset?.Trim().Trim('"', '\'') is { Length: > 0 } name)
        {
            try
            {
                encoding = Encoding.GetEncoding(name);
            }
            catch (ArgumentException)
            {
                encoding = Encoding.UTF8;
            }
        }
        return encoding.GetString(bytes);
    }
}
