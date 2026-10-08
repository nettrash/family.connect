namespace FamilyConnect.App.Logic;

/// <summary>One line of the paperclip's menu (<see cref="AttachMenu.Groups"/>).</summary>
public enum AttachItem
{
    /// <summary>"Photo or Video": a picture-and-video picker, the Pictures library first.</summary>
    PhotoOrVideo,

    /// <summary>"Show the Assistant a Photo…": pictures only — in the assistant's chat, in Photo or Video's place.</summary>
    AssistantPhoto,

    /// <summary>"File": every file, photos and videos included.</summary>
    File,

    /// <summary>"Paste": the clipboard.</summary>
    Paste,

    /// <summary>"Record Voice Message": hands-free.</summary>
    RecordVoice,

    /// <summary>"Record Video Message": the round-video recorder.</summary>
    RecordVideo,

    /// <summary>"Location": where I am, once.</summary>
    Location,

    /// <summary>"Poll".</summary>
    Poll,
}

/// <summary>
/// The paperclip's menu, the same on every client (docs/attachment-menu-2026-10-07.md, issue #78): three groups read top
/// to bottom — what to pick (a photo or video, a file, the clipboard), what to record, and the rest (a place, a poll) —
/// with a separator between groups and none around a group that is empty. Windows has no Camera line (S1.5 of
/// docs/audio-video-messages-2026-10-04.md). Only WHICH lines show is decided here; whether a line is enabled (a call in
/// progress, an unsent voice message, a busy composer) stays with the view.
/// </summary>
public static class AttachMenu
{
    /// <summary>
    /// What the "Photo or Video" picker offers: every picture and video type <see cref="FamilyConnect.Core.MediaPrep.MimeFor"/> knows —
    /// the photos it re-draws, the GIF and WebP that go with their own bytes, and the videos the server takes as videos.
    /// SVG is a drawing, not a photo, and stays behind "File".
    /// </summary>
    public static IReadOnlyList<string> PhotoOrVideoTypes { get; } =
    [
        ".jpg", ".jpeg", ".png", ".heic", ".heif", ".webp", ".gif", ".bmp", ".tif", ".tiff", ".avif",
        ".mp4", ".m4v", ".mov",
    ];

    /// <summary>
    /// The menu's lines, group by group, top to bottom. In the assistant's chat, "Show the Assistant a Photo…" stands where
    /// "Photo or Video" does — and only where <paramref name="offersPictureAttach"/> (the server can see and the family
    /// allows it); never both lines. Nothing is recorded for the assistant, where every message is a consented model call;
    /// a video message only where one can be recorded; a poll only in the family chat.
    /// </summary>
    public static IReadOnlyList<IReadOnlyList<AttachItem>> Groups(
        bool assistantChat, bool offersPictureAttach, bool familyChat, bool roundAvailable)
    {
        List<AttachItem> pick = [];
        if (!assistantChat)
        {
            pick.Add(AttachItem.PhotoOrVideo);
        }
        else if (offersPictureAttach)
        {
            pick.Add(AttachItem.AssistantPhoto);
        }
        pick.Add(AttachItem.File);
        pick.Add(AttachItem.Paste);

        List<AttachItem> record = [];
        if (!assistantChat)
        {
            record.Add(AttachItem.RecordVoice);
            if (roundAvailable)
            {
                record.Add(AttachItem.RecordVideo);
            }
        }

        List<AttachItem> rest = [AttachItem.Location];
        if (familyChat)
        {
            rest.Add(AttachItem.Poll);
        }

        return [.. new IReadOnlyList<AttachItem>[] { pick, record, rest }.Where(group => group.Count > 0)];
    }
}
