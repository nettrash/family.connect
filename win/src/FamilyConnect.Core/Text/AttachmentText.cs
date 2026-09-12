namespace FamilyConnect.Core;

/// <summary>
/// What an attachment is CALLED when it has no name of its own — ported from
/// <c>fc_text::media::display_name</c> (itself <c>AttachmentDTO.displayName</c>) and pinned to it
/// by the oracle.
/// </summary>
/// <remarks>
/// A name is REQUIRED on a file (its whole identity) and optional on audio and a location, where
/// it is a label; a photo or video usually has none. An EMPTY name is no name: a caption-less
/// photo must read as "Photo" rather than as a blank row, because "a preview with nothing in it is
/// a chat row that looks like nothing happened".
/// </remarks>
public static class AttachmentText
{
    public static string DisplayName(string kind, string? name, IStringCatalog? words = null)
    {
        if (!string.IsNullOrEmpty(name))
        {
            return name;
        }
        var say = words ?? EnglishCatalog.Instance;
        return say.Get(kind switch
        {
            "video" => "Video",
            "audio" => "Audio",
            "location" => "Location",
            "file" => "File",
            // A kind this build does not know is drawn as the commonest one, which is what every
            // other client does with it.
            _ => "Photo",
        });
    }
}
