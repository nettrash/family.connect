using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// What the window asks of an attachment before it draws or saves one — ported from the web client's
/// attachment view, which is where these rules were first written down, and tested with its vectors.
/// </summary>
public static class AttachmentFiles
{
    /// <summary>Which bytes a tile draws.</summary>
    public enum TileSource
    {
        /// <summary>Nothing to fetch: a video with no poster is drawn as a labelled box.</summary>
        None,

        /// <summary>The small copy the uploader made.</summary>
        Preview,

        /// <summary>The photo itself, when it has no small copy.</summary>
        Original,
    }

    /// <summary>
    /// The preview when there is one; a PHOTO's own bytes when there is not; and for a video without a
    /// poster, nothing — a tile never downloads a whole video to draw itself.
    /// </summary>
    public static TileSource SourceFor(AttachmentDto attachment) =>
        attachment.HasPreview ? TileSource.Preview
        : attachment.Kind == "photo" ? TileSource.Original
        : TileSource.None;

    /// <summary>What a download is saved as: its own name, or one made from its kind and type.</summary>
    public static string FileName(AttachmentDto attachment)
    {
        if (attachment.Name is { Length: > 0 } name)
        {
            return name;
        }
        var essence = attachment.Mime?.Split(';')[0].Trim().ToLowerInvariant();
        var extension = essence switch
        {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/heic" => "heic",
            "video/quicktime" => "mov",
            "video/mp4" => "mp4",
            "audio/mp4" => "m4a",
            "audio/mpeg" => "mp3",
            "audio/wav" => "wav",
            "audio/ogg" => "ogg",
            _ => "bin",
        };
        return $"{attachment.Kind}-{Math.Max(attachment.Id, 0)}.{extension}";
    }

    /// <summary>
    /// A long name with its middle given up, so its start AND its extension stay readable. Counted in
    /// scalars, so a cut never splits an emoji's surrogate pair.
    /// </summary>
    public static string MiddleTruncate(string name, int room)
    {
        var scalars = name.EnumerateRunes().Select(rune => rune.ToString()).ToList();
        if (scalars.Count <= room || room < 5)
        {
            return name;
        }
        var tail = (room - 1) / 3;
        var head = room - 1 - tail;
        return string.Concat(scalars.Take(head)) + "…" + string.Concat(scalars.Skip(scalars.Count - tail));
    }
}
