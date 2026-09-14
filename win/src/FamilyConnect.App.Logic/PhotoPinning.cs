using FamilyConnect.Core;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>How pinning a photo ended.</summary>
public sealed record PinOutcome(bool Pinned, bool Busy = false, ApiError? Error = null)
{
    public static readonly PinOutcome Done = new(true);

    /// <summary>A photo is already on its way: one at a time, and said so.</summary>
    public static readonly PinOutcome StillPinning = new(false, Busy: true);

    public static PinOutcome Refused(ApiError error) => new(false, Error: error);
}

/// <summary>
/// A picture onto the wall: uploaded as a photo, its preview after it, then a PHOTO NOTE naming it
/// (docs/protocol.md, "Board": a photo note is a picture this caller uploaded and nothing has claimed,
/// named as <c>attachment_id</c>) — the web client's <c>pin_photo</c>.
/// </summary>
/// <remarks>
/// <para>
/// <b>ONE AT A TIME, AND SAID SO.</b> A second photo quietly dropped is a photo its sender thinks is on
/// the wall.
/// </para>
/// <para>
/// <b>THE PREVIEW IS BEST EFFORT, BUT NOT BEST EFFORT ONCE:</b> the sticker draws it, so a failed one is
/// tried twice more beside the pin — when the failure was the network's, since the same bytes refused
/// once would be refused again.
/// </para>
/// </remarks>
public sealed class PhotoPinning(ApiClient api, BoardStore board, Func<TimeSpan, CancellationToken, Task> wait)
{
    /// <summary>The pauses before the preview is tried again.</summary>
    public static IReadOnlyList<TimeSpan> PreviewRetries { get; } = [TimeSpan.FromSeconds(2), TimeSpan.FromSeconds(8)];

    private int pinning;

    public bool Pinning => Volatile.Read(ref pinning) == 1;

    public async Task<PinOutcome> PinAsync(StagedMedia photo, (double X, double Y) at, string color, CancellationToken ct = default)
    {
        if (Interlocked.CompareExchange(ref pinning, 1, 0) == 1)
        {
            return PinOutcome.StillPinning;
        }
        try
        {
            var uploaded = await api.Upload("photo", photo.Mime, photo.Bytes, photo.Width, photo.Height, ct: ct).ConfigureAwait(false);
            if (!uploaded.Ok || uploaded.Value is null)
            {
                return PinOutcome.Refused(uploaded.Error ?? ApiError.Transport("no answer"));
            }
            var attachmentId = uploaded.Value.Attachment.Id;
            if (photo.Preview is { IsEmpty: false } preview)
            {
                var sent = await api.UploadPreview(attachmentId, preview, ct).ConfigureAwait(false);
                if (!sent.Ok && (sent.Error ?? ApiError.Transport("no answer")).Transient)
                {
                    _ = RetryPreviewAsync(attachmentId, preview);
                }
            }
            var created = await api.CreateNote(
                new NoteRequest(
                    string.Empty, color, at.X, at.Y, Notes.NameOf(NoteSize.Medium), Notes.NameOf(NoteFont.Plain),
                    Kind: Notes.NameOf(NoteKind.Photo), AttachmentId: attachmentId),
                ct).ConfigureAwait(false);
            if (!created.Ok || created.Value is null)
            {
                return PinOutcome.Refused(created.Error ?? ApiError.Transport("no answer"));
            }
            // As EVIDENCE, exactly as a note written from the sheet: one note's news, no cursor moved.
            board.Apply(created.Value.Note, SeqRoute.Evidence);
            return PinOutcome.Done;
        }
        finally
        {
            Interlocked.Exchange(ref pinning, 0);
        }
    }

    /// <summary>Where a picked photo lands: near the upper middle, jittered so two do not stack exactly.</summary>
    public static (double X, double Y) Scattered(Func<double> random) =>
        (0.35 + ((random() - 0.5) * 0.1), 0.30 + ((random() - 0.5) * 0.1));

    /// <summary>Where a DROPPED photo lands: a medium card centred under the pointer, held inside the wall.</summary>
    public static (double X, double Y) DroppedAt((double X, double Y) pointer, (double Width, double Height) wall)
    {
        var card = BoardWall.Card(NoteSize.Medium, BoardWall.IsCompact(wall.Width));
        var corner = BoardWall.Clamp(pointer.X - (card.Width / 2), pointer.Y - (card.Height / 2), card, wall);
        return BoardWall.FractionOf(corner, wall);
    }

    public static string StillPinningText(IStringCatalog say) =>
        say.Get("A photo is still being pinned. Add the next one when it is on the board.");

    public static string OneAtATimeText(IStringCatalog say) =>
        say.Get("The board pins one photo at a time — the first is on its way.");

    public static string PhotosOnlyText(IStringCatalog say) => say.Get("The board pins photos only.");

    /// <summary>A refused step of pinning: that it did not go, and why.</summary>
    public static string FailureText(ApiError error, IStringCatalog say) =>
        $"{say.Get("Couldn't pin that photo.")} {NoteSheetText.Failure(error, say)}";

    private async Task RetryPreviewAsync(long attachmentId, ReadOnlyMemory<byte> preview)
    {
        foreach (var pause in PreviewRetries)
        {
            try
            {
                await wait(pause, CancellationToken.None).ConfigureAwait(false);
                if ((await api.UploadPreview(attachmentId, preview).ConfigureAwait(false)).Ok)
                {
                    return;
                }
            }
            catch (Exception)
            {
                return;
            }
        }
    }
}
