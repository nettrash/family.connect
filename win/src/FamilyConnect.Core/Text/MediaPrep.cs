using System.Globalization;
using System.Text;

namespace FamilyConnect.Core;

/// <summary>How a picked file is sent.</summary>
public enum MediaRoute
{
    /// <summary>Decoded, downscaled and re-encoded as JPEG, with a preview.</summary>
    Photo,

    /// <summary>Uploaded untouched, with a poster.</summary>
    Video,

    /// <summary>Uploaded untouched, as the type the server checks (<see cref="Routed.AudioMime"/>).</summary>
    Audio,

    /// <summary>Uploaded untouched; nothing is checked.</summary>
    File,
}

/// <summary>A route, and for audio the type the server will accept it as.</summary>
public readonly record struct Routed(MediaRoute Route, string? AudioMime = null);

/// <summary>
/// What happens to a file between being picked and being uploaded: which kind it goes as, what it is
/// called and typed, and whether the server will accept its bytes. Ported from <c>fc_text::media</c> —
/// itself the Apple client's <c>MediaPrep</c> with the SERVER's own magic-number table — and held to it
/// by <c>ChatOracleTests</c>.
/// </summary>
/// <remarks>
/// <para>
/// <b>A VIDEO OR A RECORDING GOES AS ONE ONLY WHEN THE SERVER WILL TAKE ITS BYTES AS ONE</b>, and as a
/// file otherwise, where nothing is checked. The Mac once sent an <c>.mkv</c> as a video on its type
/// alone and the server refused the message.
/// </para>
/// <para>
/// <b>A PHOTO IS RE-DRAWN</b>, which is also what leaves its EXIF — where it was taken — behind. GIF and
/// WebP animate and a JPEG has nowhere to put the frames, so they go as files with their own bytes.
/// </para>
/// </remarks>
public static class MediaPrep
{
    /// <summary>The protocol's default ceiling for one attachment.</summary>
    public const long SizeLimit = 100L * 1024 * 1024;

    /// <summary>Attachments one message may carry.</summary>
    public const int MaxPerMessage = 10;

    public const int PhotoEdge = 2048;
    public const double PhotoQuality = 0.85;

    /// <summary>A preview — a photo's small copy, a video's poster.</summary>
    public const int PreviewEdge = 600;
    public const double PreviewQuality = 0.7;

    /// <summary>Where a video's poster frame is looked for, in seconds, in order.</summary>
    public static IReadOnlyList<double> PosterSeekSeconds { get; } = [0.5, 0.0, 2.0];

    /// <summary>The protocol's ceiling for an attachment name, in scalars.</summary>
    public const int MaxNameLength = 255;

    /// <summary>A media type, lowercased (ASCII only, as the original lowers it) and without its parameters.</summary>
    public static string Essence(string mime) => AsciiLower(mime.Split(';')[0].Trim());

    /// <summary>A name's extension, lowercased, without the dot; a leading dot is a hidden file, not an extension.</summary>
    public static string Extension(string name)
    {
        var dot = name.LastIndexOf('.');
        return dot > 0 && dot + 1 < name.Length ? AsciiLower(name[(dot + 1)..]) : string.Empty;
    }

    /// <summary>The type a file travels as when nothing named one: the common ones by extension.</summary>
    public static string MimeFor(string name) => Extension(name) switch
    {
        "jpg" or "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "tif" or "tiff" => "image/tiff",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "mp4" or "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "mp3" => "audio/mpeg",
        "wav" or "wave" => "audio/wav",
        "ogg" or "oga" => "audio/ogg",
        "aif" or "aiff" => "audio/aiff",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "rtf" => "application/rtf",
        "zip" => "application/zip",
        "json" => "application/json",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "pages" => "application/vnd.apple.pages",
        "numbers" => "application/vnd.apple.numbers",
        "key" => "application/vnd.apple.keynote",
        _ => "application/octet-stream",
    };

    /// <summary>The type the picker named, or the one the name implies when it named none.</summary>
    public static string DeclaredType(string pickedMime, string name)
    {
        var named = Essence(pickedMime);
        return named.Length == 0 || named == "application/octet-stream" ? MimeFor(name) : named;
    }

    /// <summary>The type the server will accept a recording as, or null when it will not take it as audio.</summary>
    public static string? AudioMime(string mime, string name)
    {
        var byType = mime switch
        {
            "audio/mp4" or "audio/x-m4a" or "audio/m4a" or "audio/aac" or "audio/x-aac" => "audio/mp4",
            "audio/mpeg" or "audio/mp3" => "audio/mpeg",
            "audio/wav" or "audio/x-wav" or "audio/wave" or "audio/vnd.wave" => "audio/wav",
            "audio/ogg" or "application/ogg" => "audio/ogg",
            _ => null,
        };
        return byType ?? Extension(name) switch
        {
            "m4a" or "aac" => "audio/mp4",
            "mp3" => "audio/mpeg",
            "wav" or "wave" => "audio/wav",
            "ogg" or "oga" => "audio/ogg",
            _ => null,
        };
    }

    /// <summary>How a picked file goes, from what it was said to be, its name, and its first bytes.</summary>
    public static Routed Route(string pickedMime, string name, ReadOnlySpan<byte> head)
    {
        var mime = DeclaredType(pickedMime, name);
        if (mime.StartsWith("image/", StringComparison.Ordinal))
        {
            return new Routed(IsPhotoType(mime) ? MediaRoute.Photo : MediaRoute.File);
        }
        var video = mime is "video/mp4" or "video/quicktime" or "video/x-m4v";
        if (video && MatchesMagic("video/mp4", head))
        {
            return new Routed(MediaRoute.Video);
        }
        if (mime.StartsWith("audio/", StringComparison.Ordinal) || mime == "application/ogg"
            || AudioMime(string.Empty, name) is not null)
        {
            if (AudioMime(mime, name) is { } audio && MatchesMagic(audio, head))
            {
                return new Routed(MediaRoute.Audio, audio);
            }
        }
        return new Routed(MediaRoute.File);
    }

    /// <summary>The server's check, byte for byte: the declared type must match what the bytes are.</summary>
    public static bool MatchesMagic(string mime, ReadOnlySpan<byte> head) => mime switch
    {
        "image/jpeg" => head.StartsWith((ReadOnlySpan<byte>)[0xFF, 0xD8, 0xFF]),
        "image/png" => head.StartsWith((ReadOnlySpan<byte>)[0x89, (byte)'P', (byte)'N', (byte)'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "image/heic" or "image/heif" or "video/mp4" or "video/quicktime" or "audio/mp4" or "audio/m4a" =>
            head.Length >= 12 && head[4..8].SequenceEqual("ftyp"u8),
        "audio/mpeg" => head.StartsWith("ID3"u8) || (head.Length >= 2 && head[0] == 0xFF && (head[1] & 0xE0) == 0xE0),
        "audio/wav" => head.Length >= 12 && head.StartsWith("RIFF"u8) && head[8..12].SequenceEqual("WAVE"u8),
        "audio/ogg" => head.StartsWith("OggS"u8),
        _ => false,
    };

    /// <summary>
    /// A name the server will take and a recipient will recognise: <c>/</c> and <c>:</c> become <c>_</c>,
    /// and so does any character with a control or FORMAT code — a name lands on somebody else's disk,
    /// and a right-to-left override is how "invoice[RLO]fdp.exe" reads as a PDF. Trimmed; nothing left
    /// is null. Over the limit the STEM is cut, between characters, so the extension survives; the
    /// limit is counted in scalars, as the server counts it.
    /// </summary>
    public static string? SanitizedName(string raw)
    {
        var stripped = new StringBuilder();
        foreach (var grapheme in Graphemes(raw))
        {
            stripped.Append(grapheme is "/" or ":" || grapheme.EnumerateRunes().Any(rune => IsControl(rune) || IsFormat(rune))
                ? "_"
                : grapheme);
        }
        var name = stripped.ToString().Trim();
        if (name.Length == 0)
        {
            return null;
        }
        if (Scalars(name) <= MaxNameLength)
        {
            return name;
        }
        var extension = Extension(name);
        var extensionLength = Scalars(extension);
        if (extension.Length == 0 || extensionLength + 1 >= MaxNameLength)
        {
            return Fits(name, MaxNameLength);
        }
        var stem = name[..(name.Length - extension.Length - 1)];
        var original = name[^extension.Length..];
        return $"{Fits(stem, MaxNameLength - extensionLength - 1)}.{original}";
    }

    /// <summary>The longest edge scaled to fit <paramref name="maxEdge"/>, never up.</summary>
    public static (uint Width, uint Height) FitWithin(uint width, uint height, uint maxEdge)
    {
        var longest = Math.Max(width, height);
        if (longest <= maxEdge || longest == 0)
        {
            return (Math.Max(width, 1), Math.Max(height, 1));
        }
        var scale = (double)maxEdge / longest;
        uint Fit(uint side) => Math.Max((uint)Math.Round(side * scale, MidpointRounding.AwayFromZero), 1);
        return (Fit(width), Fit(height));
    }

    private static bool IsPhotoType(string mime) =>
        mime is "image/jpeg" or "image/jpg" or "image/png" or "image/heic" or "image/heif" or "image/tiff" or "image/avif";

    private static string AsciiLower(string text) =>
        string.Create(text.Length, text, (span, source) =>
        {
            for (var at = 0; at < source.Length; at++)
            {
                span[at] = source[at] is >= 'A' and <= 'Z' ? (char)(source[at] + 32) : source[at];
            }
        });

    private static int Scalars(string text) => text.EnumerateRunes().Count();

    private static IEnumerable<string> Graphemes(string text)
    {
        var elements = StringInfo.GetTextElementEnumerator(text);
        while (elements.MoveNext())
        {
            yield return elements.GetTextElement();
        }
    }

    private static string Fits(string text, int room)
    {
        var kept = new StringBuilder();
        var count = 0;
        foreach (var grapheme in Graphemes(text))
        {
            var size = Scalars(grapheme);
            if (count + size > room)
            {
                break;
            }
            kept.Append(grapheme);
            count += size;
        }
        return kept.ToString();
    }

    /// <summary>General category Cc, as the original's <c>char::is_control</c> reads it.</summary>
    private static bool IsControl(Rune rune) => Rune.GetUnicodeCategory(rune) == UnicodeCategory.Control;

    /// <summary>
    /// General category Cf — the invisible FORMAT characters, bidi overrides among them — as the
    /// original's own table lists them (a table, not the runtime's category, so the two cannot drift
    /// apart with a Unicode version).
    /// </summary>
    private static bool IsFormat(Rune rune) => rune.Value switch
    {
        0x00AD or 0x061C or 0x06DD or 0x070F or 0x08E2 or 0x180E or 0xFEFF or 0x110BD or 0x110CD or 0xE0001 => true,
        >= 0x0600 and <= 0x0605 => true,
        >= 0x0890 and <= 0x0891 => true,
        >= 0x200B and <= 0x200F => true,
        >= 0x202A and <= 0x202E => true,
        >= 0x2060 and <= 0x2064 => true,
        >= 0x2066 and <= 0x206F => true,
        >= 0xFFF9 and <= 0xFFFB => true,
        >= 0x13430 and <= 0x1343F => true,
        >= 0x1BCA0 and <= 0x1BCA3 => true,
        >= 0x1D173 and <= 0x1D17A => true,
        >= 0xE0020 and <= 0xE007F => true,
        _ => false,
    };
}
