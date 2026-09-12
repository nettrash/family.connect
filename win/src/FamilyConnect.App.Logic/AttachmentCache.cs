using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// Where downloaded bytes are kept. A seam because WHERE is a Windows question — the app's own
/// cache folder, which the system may clear — and because a test has no business writing files.
/// </summary>
public interface IBlobStore
{
    /// <summary>The bytes under this key, or null when they are not held.</summary>
    byte[]? Read(string key);

    /// <summary>Keep these bytes under this key.</summary>
    void Write(string key, ReadOnlyMemory<byte> bytes);
}

/// <summary>
/// One attachment's bytes, downloaded once and kept — with the one rule about previews that three
/// of this app's clients have each had to learn.
/// </summary>
/// <remarks>
/// <para>
/// <b>A PREVIEW IS ONLY ASKED FOR WHEN THE ATTACHMENT SAYS IT HAS ONE.</b> The server generates
/// no preview for a picture the ASSISTANT drew, and none at all for a file, a piece of audio or a
/// location — so <c>has_preview</c> is a fact, not a hint. Asking anyway answers 404, and a client
/// that treats that as "no picture" draws an empty frame for ever: it is exactly how the Android
/// board's backdrops failed (issue #71's follow-up), and the fix belongs HERE rather than at every
/// call site, where one of them will always forget.
/// </para>
/// <para>
/// <b>A FAILED DOWNLOAD IS NOT CACHED.</b> Nothing is written unless bytes arrived, so the next
/// look asks again — a transient failure must not become a permanently blank picture.
/// </para>
/// </remarks>
public sealed class AttachmentCache(ApiClient api, IBlobStore blobs)
{
    /// <summary>The key bytes are kept under: the id, and whether they are the small copy.</summary>
    public static string KeyFor(long attachmentId, bool preview) =>
        preview ? $"{attachmentId}.preview" : $"{attachmentId}";

    /// <summary>
    /// The bytes for this attachment, from the cache when they are held and from the server
    /// otherwise.
    /// </summary>
    /// <param name="preview">
    /// Whether the small copy will do. It is asked for only when the attachment HAS one; on one
    /// that does not, this reads the original instead of asking for something that is not there.
    /// </param>
    public async Task<(byte[]? Bytes, ApiError? Error)> BytesAsync(
        AttachmentDto attachment, bool preview = false, CancellationToken ct = default)
    {
        // THE ONE RULE: a preview exists only where the attachment says so.
        var small = preview && attachment.HasPreview;
        var key = KeyFor(attachment.Id, small);
        if (blobs.Read(key) is { } held)
        {
            return (held, null);
        }
        var answer = await api.Download(attachment.Id, small, ct).ConfigureAwait(false);
        if (!answer.Ok || answer.Value is null)
        {
            // Not cached: the next look asks again rather than drawing a blank for ever.
            return (null, answer.Error ?? ApiError.Transport("no answer"));
        }
        blobs.Write(key, answer.Value);
        return (answer.Value, null);
    }

    /// <summary>
    /// Whether these bytes are already here — what a view asks before it decides to show a
    /// spinner.
    /// </summary>
    public bool Holds(AttachmentDto attachment, bool preview = false) =>
        blobs.Read(KeyFor(attachment.Id, preview && attachment.HasPreview)) is not null;
}
