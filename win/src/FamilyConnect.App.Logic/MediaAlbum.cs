using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>What a key does in the viewer.</summary>
public enum ViewerKey
{
    None,
    Close,
    Previous,
    Next,
}

/// <summary>
/// A message's photos and videos at full size, one at a time (web <c>views/viewer.rs</c>, the Mac's
/// <c>MacAttachmentViewer</c>): paging that stops at the ends, a photo's zoom from 1× to 6×, and the words over it.
/// </summary>
/// <remarks>
/// A page turn is also a zoom reset: the next picture is a different shape, and a 3× window into it would show a corner of
/// nothing. Save hands over the ORIGINAL, never the preview a bubble draws — that is the window's business, not this.
/// </remarks>
public sealed class MediaAlbum
{
    /// <summary>The furthest a photo zooms.</summary>
    public const double MaxZoom = 6.0;

    /// <summary>One step of the zoom buttons.</summary>
    public const double ZoomFactor = 1.25;

    public MediaAlbum(IReadOnlyList<AttachmentDto> items, int index)
    {
        if (items.Count == 0)
        {
            throw new ArgumentException("an album needs something in it", nameof(items));
        }
        Items = items;
        Index = Math.Clamp(index, 0, items.Count - 1);
    }

    /// <summary>The photos and videos a message carries, in the order they were sent — what its tiles open onto.</summary>
    public static IReadOnlyList<AttachmentDto> Of(MessageDto message) => Of(message.Media);

    public static IReadOnlyList<AttachmentDto> Of(IEnumerable<AttachmentDto> media) =>
        [.. media.Where(attachment => MediaText.IsMedia(attachment.Kind))];

    public IReadOnlyList<AttachmentDto> Items { get; }

    public int Index { get; private set; }

    public int Count => Items.Count;

    public AttachmentDto Current => Items[Index];

    public bool IsVideo => Current.Kind == "video";

    public bool HasPrevious => Index > 0;

    public bool HasNext => Index + 1 < Count;

    /// <summary>1 = the whole picture fits the window.</summary>
    public double Zoom { get; private set; } = 1;

    /// <summary>One page in <paramref name="direction"/> (-1 or +1), stopping at the ends. Whether it moved.</summary>
    public bool Step(int direction)
    {
        var next = Index + Math.Sign(direction);
        if (next < 0 || next >= Count || next == Index)
        {
            return false;
        }
        Index = next;
        Zoom = 1;
        return true;
    }

    /// <summary>The next zoom after a step in <paramref name="direction"/> (+1 in, -1 out), clamped to 1–6.</summary>
    public static double ZoomStep(double scale, double direction) =>
        Math.Clamp(direction > 0 ? scale * ZoomFactor : scale / ZoomFactor, 1, MaxZoom);

    public void ZoomIn() => Zoom = ZoomStep(Zoom, 1);

    public void ZoomOut() => Zoom = ZoomStep(Zoom, -1);

    /// <summary>A double click: back to the whole picture when zoomed at all, 2× when not.</summary>
    public void ToggleZoom() => Zoom = Zoom > 1 ? 1 : 2;

    /// <summary>A zoom the window arrived at by itself — a pinch, a ctrl-wheel — kept within 1–6.</summary>
    public void ZoomedTo(double scale) => Zoom = Math.Clamp(scale, 1, MaxZoom);

    public bool CanZoomIn => !IsVideo && Zoom < MaxZoom;

    public bool CanZoomOut => !IsVideo && Zoom > 1;

    /// <summary>The zoom as the window shows it: "125%".</summary>
    public string ZoomText(System.Globalization.CultureInfo culture) => string.Create(culture, $"{Math.Round(Zoom * 100):0}%");

    /// <summary>What the item is called — its name, or its kind when it has none.</summary>
    public string Title(IStringCatalog say) => AttachmentText.DisplayName(Current.Kind, Current.Name, say);

    /// <summary>"2 of 5" while there is somewhere to page, and nothing when there is not.</summary>
    public string? Position(IStringCatalog say) => Count > 1 ? say.Format("%lld of %lld", Index + 1, Count) : null;

    /// <summary>Esc closes; ← and → page, and do nothing at the ends.</summary>
    public ViewerKey Key(string key) => key switch
    {
        "Escape" => ViewerKey.Close,
        "Left" when HasPrevious => ViewerKey.Previous,
        "Right" when HasNext => ViewerKey.Next,
        _ => ViewerKey.None,
    };
}
