using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using Windows.Storage;

namespace FamilyConnect.App.Services;

/// <summary>
/// The sound a "Show text" sends when the server cannot send its own copy — a video, an Ogg file, a recording over the
/// ceiling (docs/protocol.md, "Transcripts on request"): the file this device already holds, or downloads through the
/// attachment cache as it would to play it, made into an M4A of AAC by <see cref="MediaPreparing.SoundTrackAsync"/>.
/// </summary>
/// <remarks>
/// <para>
/// <b>A COPY WITH A NAME MEDIA FOUNDATION READS.</b> The cache keeps bytes under bare ids, and Media Foundation picks its
/// reader by the extension, so the bytes are written out once more — beside the transcoder's own files, where the
/// transcoder's sweep removes anything a crash left behind — and deleted when this returns.
/// </para>
/// <para>
/// <b>NOTHING HERE IS LOGGED BUT THE EXCEPTION'S TYPE.</b> The bytes are somebody's voice.
/// </para>
/// </remarks>
internal static class SoundTracks
{
    public static async Task<SuppliedSound> MakeAsync(
        AttachmentCache attachments, AttachmentDto attachment, long maxBytes, CancellationToken cancel)
    {
        var (held, error) = await attachments.BytesAsync(attachment, preview: false, cancel);
        if (held is null)
        {
            // The download's own failure: a lost connection is still "try again", a gone attachment is not.
            return SuppliedSound.Failed(error ?? ApiError.Transport("no answer"));
        }
        var folder = Path.Combine(Path.GetTempPath(), "FamilyConnect", "prepared");
        var path = Path.Combine(folder, $"{Guid.NewGuid():N}.{TranscriptSound.TempExtension(attachment.Mime, attachment.Name)}");
        try
        {
            Directory.CreateDirectory(folder);
            await File.WriteAllBytesAsync(path, held, cancel);
            var file = await StorageFile.GetFileFromPathAsync(path);
            // Off the window's thread, as a send's transcode is.
            return await Task.Run(
                () => MediaPreparing.SoundTrackAsync(file, attachment.DurationMs, maxBytes, cancel), cancel);
        }
        catch (OperationCanceledException) when (cancel.IsCancellationRequested)
        {
            throw;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"making a recording's sound: {e.GetType().Name}");
            return SuppliedSound.Failed(TranscriptSound.Unreadable);
        }
        finally
        {
            try
            {
                File.Delete(path);
            }
            catch (Exception e)
            {
                Diagnostics.Write($"removing a recording's copy: {e.GetType().Name}");
            }
        }
    }
}
