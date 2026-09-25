using System.Globalization;
using System.Text;

namespace FamilyConnect.Core;

/// <summary>
/// How an attachment is MEASURED and LABELLED for drawing: a file's size, a tile's shape, a location's
/// line and where "Open in Maps" goes. Ported from <c>fc_text::media</c> — the Rust the web client runs
/// — and held to it by <c>ChatOracleTests</c>.
/// </summary>
/// <remarks>
/// <para>
/// <b>SHAPES COME FROM METADATA, NEVER FROM LOADED BYTES</b>, so a row does not change height when its
/// picture lands.
/// </para>
/// <para>
/// <b>A SIZE IS SHOWN, A PLACE IS NOT.</b> A file's size uses the reader's decimal separator — it is a
/// number for a person — while a location's coordinates are always written with a POINT, because a
/// comma between two coordinates reads as a different place.
/// </para>
/// </remarks>
public static class MediaText
{
    /// <summary>The largest a lone photo or video tile draws, either way.</summary>
    public const double TileMax = 320;

    /// <summary>The width over the height a photo or video is drawn at; 4:3 when the uploader could not say.</summary>
    public static double AspectRatio(int? width, int? height) =>
        width is > 0 && height is > 0 ? (double)width.Value / height.Value : 4.0 / 3.0;

    /// <summary>A lone tile, at the attachment's own shape, capped at 320 either way.</summary>
    public static (double Width, double Height) TileSize(int? width, int? height)
    {
        var ratio = AspectRatio(width, height);
        var w = TileMax;
        var h = w / ratio;
        if (h > TileMax)
        {
            h = TileMax;
            w = h * ratio;
        }
        return (w, h);
    }

    /// <summary>An album's top card: the full width, at the item's shape held between 3:4 and 3:2.</summary>
    public static (double Width, double Height) CardSize(int? width, int? height, double maxWidth)
    {
        var ratio = Math.Clamp(AspectRatio(width, height), 0.75, 1.5);
        return (maxWidth, Math.Round(maxWidth / ratio, MidpointRounding.AwayFromZero));
    }

    /// <summary>Whether a kind is LOOKED at (photos, videos) rather than read (files, audio, a location).</summary>
    public static bool IsMedia(string kind) => kind is not ("file" or "audio" or "location");

    /// <summary>
    /// "3:42" — the elapsed or total time of a recording (<c>fc_text::media::time_label</c>, ios <c>AudioRecorder.timeLabel</c>):
    /// whole seconds, rounded half away from zero as Rust rounds, never below zero, and nothing for a time that is not one.
    /// </summary>
    public static string TimeLabel(double seconds)
    {
        var whole = double.IsFinite(seconds) ? (long)Math.Max(0, Math.Round(seconds, MidpointRounding.AwayFromZero)) : 0;
        return $"{whole / 60}:{whole % 60:00}";
    }

    /// <summary>
    /// "1.2 MB": decimal units, whole kilobytes, one decimal of a megabyte, two of a gigabyte, and no
    /// trailing zero — the way the Apple apps' byte formatter writes a file's size.
    /// </summary>
    public static string DisplaySize(long bytes, IStringCatalog? words = null, CultureInfo? culture = null)
    {
        var say = words ?? EnglishCatalog.Instance;
        var reader = culture ?? CultureInfo.CurrentCulture;
        if (bytes <= 0)
        {
            return say.Get("Zero KB");
        }
        if (bytes < 1_000)
        {
            // The catalogue this port reads has no plural forms, so English's one-and-other is two keys
            // here — both English for now (win/i18n/win.json) — where the web's table has one key with
            // two forms. The oracle holds the English to "1 byte".
            return bytes == 1 ? say.Format("%lld byte", bytes) : say.Format("%lld bytes", bytes);
        }
        string Trimmed(double value, int places)
        {
            var text = value.ToString("F" + places.ToString(CultureInfo.InvariantCulture), reader);
            var point = reader.NumberFormat.NumberDecimalSeparator;
            return text.Contains(point, StringComparison.Ordinal)
                ? text.TrimEnd('0').TrimEnd(point.ToCharArray())
                : text;
        }
        var kb = bytes / 1e3;
        if (Math.Round(kb, MidpointRounding.AwayFromZero) < 1_000)
        {
            return say.Format("%@ KB", ((long)Math.Round(kb, MidpointRounding.AwayFromZero)).ToString(reader));
        }
        var mb = bytes / 1e6;
        if (Math.Round(mb * 10, MidpointRounding.AwayFromZero) / 10 < 1_000)
        {
            return say.Format("%@ MB", Trimmed(mb, 1));
        }
        return say.Format("%@ GB", Trimmed(bytes / 1e9, 2));
    }

    /// <summary>A location's second line: coordinates to five places with a POINT, and the accuracy when known.</summary>
    public static string LocationLine(double latitude, double longitude, double? accuracyM)
    {
        var point = string.Create(CultureInfo.InvariantCulture, $"{latitude:F5}, {longitude:F5}");
        return accuracyM is { } accuracy && double.IsFinite(accuracy)
            ? string.Create(CultureInfo.InvariantCulture,
                $"{point} · ±{(long)Math.Round(accuracy, MidpointRounding.AwayFromZero)} m")
            : point;
    }

    /// <summary>
    /// Where "Open in Maps" goes: Apple Maps on the web, which every client can hand a place to, with the
    /// sender's label — or "Location" — percent-encoded byte by byte.
    /// </summary>
    public static string MapsUrl(double latitude, double longitude, string? name, IStringCatalog? words = null)
    {
        var say = words ?? EnglishCatalog.Instance;
        var url = new StringBuilder(
            string.Create(CultureInfo.InvariantCulture, $"https://maps.apple.com/?ll={latitude:F7},{longitude:F7}"));
        var label = name?.Trim() is { Length: > 0 } trimmed ? trimmed : say.Get("Location");
        url.Append("&q=");
        foreach (var b in Encoding.UTF8.GetBytes(label))
        {
            if (b is >= (byte)'A' and <= (byte)'Z' or >= (byte)'a' and <= (byte)'z' or >= (byte)'0' and <= (byte)'9'
                || b == (byte)'-' || b == (byte)'_' || b == (byte)'.' || b == (byte)'~')
            {
                url.Append((char)b);
            }
            else
            {
                url.Append('%').Append(b.ToString("X2", CultureInfo.InvariantCulture));
            }
        }
        return url.ToString();
    }
}
