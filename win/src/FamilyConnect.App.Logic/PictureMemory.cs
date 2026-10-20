using System.Diagnostics.CodeAnalysis;
using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>
/// How big a photo tile's picture is decoded: near the size it is drawn at, not the size the camera took it at.
/// </summary>
/// <remarks>
/// <para>
/// <b>ONLY A PHOTO WITH NO SMALL COPY IS DECODED WHOLE OTHERWISE.</b> A tile draws an attachment's preview when it has one —
/// already small — and the photo's own bytes when it does not (<see cref="AttachmentFiles.SourceFor"/>), and a twelve
/// megapixel original decoded for a 320-point tile is forty-eight megabytes of pixels nobody sees, kept for as long as the
/// view keeps the picture.
/// </para>
/// <para>
/// <b>NEVER BLURRIER THAN BEFORE, WHICHEVER WAY UP IT IS.</b> A decoder is told one side, and which side is "width" depends on
/// an EXIF turn the attachment's own numbers may or may not include. So the width asked for is the box times the picture's
/// long side over its short one: whichever way the decoder holds it, both sides come out at least the box — the most any
/// tile of it is drawn at, a lone tile's 320 either way or an album cell's 156 — and the tile, which crops to fill, never
/// stretches a picture smaller than itself. Where that is no smaller than the picture, or its size is not known, it is
/// decoded whole, as it always was.
/// </para>
/// </remarks>
public static class PictureDecode
{
    /// <summary>The largest box any tile of a picture is drawn in, in effective pixels: a lone tile's longest side.</summary>
    public const double TileBox = MediaText.TileMax;

    /// <summary>
    /// The width, in effective pixels, to decode a photo of <paramref name="width"/> × <paramref name="height"/> at for a tile
    /// on a screen of <paramref name="rasterScale"/> — or null to decode it whole.
    /// </summary>
    public static int? TileWidth(int? width, int? height, double rasterScale, double box = TileBox)
    {
        if (width is not > 0 || height is not > 0 || box <= 0)
        {
            return null;
        }
        var longSide = Math.Max(width.Value, height.Value);
        var shortSide = Math.Min(width.Value, height.Value);
        var asked = (int)Math.Ceiling(box * longSide / shortSide);
        // Effective pixels become device pixels at the screen's scale; a decode that would not come out smaller than the
        // picture's SHORT side — whichever side the decoder calls width — saves nothing and could scale up.
        return asked * Math.Max(rasterScale, 1) < shortSide ? asked : null;
    }
}

/// <summary>
/// Decoded pictures a view keeps so a redraw does not decode them again — at most so many, the least recently drawn going
/// first. Not thread-safe: the window's thread's.
/// </summary>
/// <remarks>
/// A picture let go here is still drawn wherever an image holds it — an image keeps its own source — and is decoded again
/// only if it is drawn anew. Without a ceiling the view kept every photo, poster and tile it had ever drawn for as long as it
/// lived.
/// </remarks>
/// <param name="most">The most pictures kept.</param>
public sealed class RecentPictures<TPicture>(int most)
    where TPicture : class
{
    private readonly Dictionary<string, LinkedListNode<(string Key, TPicture Picture)>> kept = new(StringComparer.Ordinal);
    private readonly LinkedList<(string Key, TPicture Picture)> order = new();

    public int Count => kept.Count;

    /// <summary>The picture kept under this key, which now counts as the most recently drawn.</summary>
    public bool TryGetValue(string key, [MaybeNullWhen(false)] out TPicture picture)
    {
        if (kept.TryGetValue(key, out var node))
        {
            order.Remove(node);
            order.AddFirst(node);
            picture = node.Value.Picture;
            return true;
        }
        picture = null;
        return false;
    }

    /// <summary>Whether a picture is kept under this key. Asking is not drawing.</summary>
    public bool ContainsKey(string key) => kept.ContainsKey(key);

    /// <summary>Keep a picture, the newest; past the ceiling the least recently drawn goes.</summary>
    public TPicture this[string key]
    {
        set
        {
            if (kept.Remove(key, out var old))
            {
                order.Remove(old);
            }
            kept[key] = order.AddFirst((key, value));
            while (kept.Count > Math.Max(most, 1) && order.Last is { } last)
            {
                order.RemoveLast();
                kept.Remove(last.Value.Key);
            }
        }
    }

    public void Clear()
    {
        kept.Clear();
        order.Clear();
    }
}
