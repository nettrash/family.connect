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

    /// <summary>
    /// Win32's device names, which are reserved in EVERY directory and with ANY extension.
    /// </summary>
    private static readonly HashSet<string> ReservedStems = new(StringComparer.OrdinalIgnoreCase)
    {
        "CON", "PRN", "AUX", "NUL",
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    };

    /// <summary>
    /// A file name safe to write: never a path, never a character Windows refuses, and never one of
    /// Win32's device names.
    /// </summary>
    /// <remarks>
    /// The name comes from whoever sent the attachment, and the server does not vet it: for a file it
    /// checks that the name is 1..=MAX_NAME_LEN characters after trimming and nothing else
    /// (<c>handlers_attachment.rs</c>). So this is not a belt to the server's braces — it is the only
    /// guard there is.
    /// <para>
    /// The device names are the part that is easy to miss. Win32 resolves CON, PRN, AUX, NUL, COM1-9
    /// and LPT1-9 to a DEVICE in every directory and with any extension, so a temp path ending in
    /// "CON.txt" is the console rather than a file: the write goes to the device (or fails) and
    /// opening the saved file then cannot work, leaving the reader with "Something went wrong" and no
    /// way to rename anything. A leading underscore is enough to make it an ordinary name again.
    /// </para>
    /// </remarks>
    public static string SafeFileName(string name)
    {
        var bare = Path.GetFileName(name.Replace('\\', '/').Split('/').Last());
        var invalid = Path.GetInvalidFileNameChars();
        var cleaned = string.Concat(bare.Select(c => invalid.Contains(c) ? '_' : c)).Trim().TrimEnd('.');
        if (cleaned.Length == 0)
        {
            return "attachment.bin";
        }
        var stem = Path.GetFileNameWithoutExtension(cleaned);
        return ReservedStems.Contains(stem) ? "_" + cleaned : cleaned;
    }

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
