using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>What is staged under one handle: the bytes, and what the server has to be told.</summary>
public sealed record StagedMedia(
    string Kind,
    string Mime,
    ReadOnlyMemory<byte> Bytes,
    int? Width = null,
    int? Height = null,
    int? DurationMs = null,
    string? Name = null);

/// <summary>
/// Where a send's bytes wait while it is being sent. The seam exists because WHERE that is, is a
/// Windows question — the app's own local folder, somewhere the system will not reclaim — and Core
/// builds anywhere.
/// </summary>
public interface IMediaStore
{
    /// <summary>What is staged under this handle, or null when it is gone.</summary>
    StagedMedia? Read(string handle);

    /// <summary>
    /// Forget everything except these handles. Called with every handle every queued row still
    /// names, so the staging area cannot outlive the sends it is for.
    /// </summary>
    void Sweep(IReadOnlySet<string> keep);
}

/// <summary>
/// The uploads a queued message owes: the pump that moves the bytes, one file at a time, before
/// the message may be posted at all.
/// </summary>
/// <remarks>
/// <para>
/// <b>A MEDIA SEND IS REPRESENTED BEFORE THE FIRST BYTE MOVES.</b> The row and the files it owes
/// are written down first — that is the outbox's job — and this is what then pushes them, so an
/// interrupted send is a bubble that can be finished rather than nothing at all
/// (docs/protocol.md, "Sending on an unreliable network").
/// </para>
/// <para>
/// <b>EACH LANDING IS REMEMBERED AS IT HAPPENS.</b> An id that landed is kept and reused within
/// the server's grace, so a retry pushes only the remainder — and a crash halfway through a
/// four-photo message costs the remaining three, not all four.
/// </para>
/// <para>
/// <b>THE ROW IS NEVER POSTED WHILE IT OWES ANYTHING.</b> That gate is in the send pipeline; what
/// matters here is the other half of it — a row whose bytes this device can no longer find is
/// FAILED rather than retried, because an id cannot recover a picture and a backoff has nothing
/// to wait for.
/// </para>
/// </remarks>
public sealed class MediaOutbox(OutboxStore outbox, ApiClient api, IMediaStore media)
{
    /// <summary>A row whose media cannot be sent, and the reason to show.</summary>
    public event Action<OutboxRow, ApiError>? Refused;

    /// <summary>
    /// Push what is owed. Answers how many files landed; a transient failure leaves the rest
    /// owed, and the next flush comes back to them.
    /// </summary>
    public async Task<int> PushAsync(CancellationToken ct = default)
    {
        var landed = 0;
        foreach (var row in outbox.All())
        {
            if (row.Failed || !row.OwesUploads)
            {
                continue;
            }
            foreach (var handle in row.PendingFiles ?? [])
            {
                if (ct.IsCancellationRequested)
                {
                    return landed;
                }
                var staged = media.Read(handle);
                if (staged is null)
                {
                    // The bytes are gone — a reclaimed temporary, or a reinstall. Nothing else is
                    // coming, and saying so beats a bubble that spins for ever.
                    outbox.Refuse(row.ClientMsgId, ErrorCodes.MediaMissing);
                    Refused?.Invoke(
                        row,
                        new ApiError(ErrorCodes.MediaMissing, "the file is no longer here"));
                    break;
                }
                var answer = await api.Upload(
                    staged.Kind, staged.Mime, staged.Bytes,
                    staged.Width, staged.Height, staged.DurationMs, staged.Name, ct)
                    .ConfigureAwait(false);
                if (answer.Ok && answer.Value is not null)
                {
                    // Remembered AT ONCE: the id is reusable within the grace, and a crash here
                    // costs the remainder rather than the lot.
                    outbox.Uploaded(row.ClientMsgId, answer.Value.Attachment.Id, handle);
                    landed++;
                    continue;
                }
                var error = answer.Error ?? ApiError.Transport("no answer");
                if (error.Transient)
                {
                    // Still owed. The next flush — a returning network, the window coming
                    // forward, the socket connecting — comes back to it.
                    break;
                }
                // A refusal about the FILE ITSELF: too large, not the kind it claimed, an image
                // the server cannot read. Trying again with the same bytes would refuse again.
                outbox.Refuse(row.ClientMsgId, error.Code);
                Refused?.Invoke(row, error);
                break;
            }
        }
        return landed;
    }

    /// <summary>
    /// Forget the staged bytes no queued row names any more — a send that is over, one way or the
    /// other. Called after a flush, so the staging area cannot outlive the sends it is for.
    /// </summary>
    public void Sweep()
    {
        var keep = new HashSet<string>(StringComparer.Ordinal);
        foreach (var row in outbox.All())
        {
            foreach (var handle in row.StagedFiles ?? [])
            {
                keep.Add(handle);
            }
        }
        media.Sweep(keep);
    }

    /// <summary>What a row is waiting for, for a bubble that wants to say "sending 2 of 3".</summary>
    public (int Landed, int Owed) Progress(OutboxRow row) =>
        ((row.AttachmentIds ?? []).Length, (row.PendingFiles ?? []).Length);

}
