using System.Globalization;
using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>Why a picked file could not be staged.</summary>
public enum PrepFailure
{
    /// <summary>Over the protocol's ceiling for one attachment.</summary>
    TooLarge,

    /// <summary>Gone, locked, or a photo nothing on this device can decode.</summary>
    Unreadable,
}

/// <summary>One picked file, prepared — or the reason it was not.</summary>
public sealed record PrepOutcome(StagedMedia? Media, PrepFailure? Failure = null)
{
    public static PrepOutcome Staged(StagedMedia media) => new(media);

    public static PrepOutcome Refused(PrepFailure failure) => new(null, failure);
}

/// <summary>
/// What is staged to go with one chat's next message — held in MEMORY until the send, as the web
/// client holds it.
/// </summary>
/// <remarks>
/// <para>
/// <b>STAGED IS NOT QUEUED.</b> Nothing reaches the media store before the person presses Send: the
/// store is swept after every flush down to what the OUTBOX names, and a chip that lived on disk
/// before its row existed would be swept out from under the composer.
/// </para>
/// <para>
/// <b>PREPARED ONE AT A TIME, IN ORDER, AND STOPPED AT THE CAP</b> rather than preparing the rest only
/// to throw them away (the web client's <c>ingest</c>, and MacConversationView's before it).
/// </para>
/// <para>
/// <b>A BATCH CAN BE CALLED OFF</b> (<see cref="CancelPreparing"/>). Preparing was once a file read that took seconds;
/// since a video is transcoded first (issue #74) it can take minutes, and somebody who dropped the wrong clip — or
/// closed the window — must not have to sit it out behind "Wait until the current attachment is done."
/// </para>
/// </remarks>
public sealed class ComposerStaging
{
    private readonly List<StagedMedia> items = [];

    /// <summary>The batch being prepared, so it can be called off; null between batches.</summary>
    private CancellationTokenSource? preparing;

    public IReadOnlyList<StagedMedia> Items => items;

    /// <summary>Whether a batch is being prepared — the composer's other busy.</summary>
    public bool Preparing { get; private set; }

    /// <summary>Whether one more fits in this message.</summary>
    public bool CanStage => items.Count < MediaPrep.MaxPerMessage;

    /// <summary>Stage one, if it fits. Answers whether it did.</summary>
    public bool Add(StagedMedia media)
    {
        if (!CanStage)
        {
            return false;
        }
        items.Add(media);
        return true;
    }

    public void Remove(int index)
    {
        if (index >= 0 && index < items.Count)
        {
            items.RemoveAt(index);
        }
    }

    /// <summary>Everything staged, handed to a send, and the strip left empty.</summary>
    public IReadOnlyList<StagedMedia> TakeAll()
    {
        var taken = items.ToList();
        items.Clear();
        return taken;
    }

    /// <summary>Put a send's items back in front of anything staged since — the send could not be written down.</summary>
    public void Restore(IReadOnlyList<StagedMedia> taken)
    {
        items.InsertRange(0, taken);
    }

    /// <summary>
    /// Why nothing may be attached right now, or null. Which busy it is, because the two have
    /// different ways out (MacConversationView.composerBusyNotice).
    /// </summary>
    public string? BusyReason(bool editing, IStringCatalog say) =>
        editing ? say.Get("Finish editing before attaching something.")
        : Preparing ? say.Get("Wait until the current attachment is done.")
        : null;

    /// <summary>
    /// Call off the batch being prepared: the file in hand is told to stop, the rest are never started, and nothing is
    /// said — the person asked for this. What the batch had already staged stays; each has its own chip to remove it
    /// by. Nothing happens when no batch is running.
    /// </summary>
    public void CancelPreparing()
    {
        try
        {
            preparing?.Cancel();
        }
        catch (ObjectDisposedException)
        {
            // The batch ended between the look and the call: there is nothing left to call off.
        }
    }

    /// <summary>
    /// Prepare <paramref name="files"/> into this strip. Answers what to say afterwards — the cap,
    /// or the last refusal — or null for nothing. A pane that went away meanwhile
    /// (<paramref name="stillHere"/>) stops the batch: its files must not land in somebody else's
    /// composer.
    /// </summary>
    public Task<string?> IngestAsync<T>(
        IReadOnlyList<T> files,
        Func<T, Task<PrepOutcome>> prepare,
        Func<bool> stillHere,
        IStringCatalog say) =>
        IngestAsync(files, (file, _) => prepare(file), stillHere, say);

    /// <summary>
    /// The same, with a preparer that can be stopped: it is handed the token <see cref="CancelPreparing"/> cancels.
    /// A batch called off stages nothing more and says nothing, whether its preparer stopped by throwing or came back
    /// with something anyway.
    /// </summary>
    public async Task<string?> IngestAsync<T>(
        IReadOnlyList<T> files,
        Func<T, CancellationToken, Task<PrepOutcome>> prepare,
        Func<bool> stillHere,
        IStringCatalog say)
    {
        using var batch = new CancellationTokenSource();
        preparing = batch;
        Preparing = true;
        try
        {
            string? said = null;
            foreach (var file in files)
            {
                if (!stillHere() || batch.IsCancellationRequested)
                {
                    return null;
                }
                if (!CanStage)
                {
                    said = CapSentence(say);
                    break;
                }
                PrepOutcome outcome;
                try
                {
                    outcome = await prepare(file, batch.Token).ConfigureAwait(true);
                }
                catch (Exception)
                {
                    outcome = PrepOutcome.Refused(PrepFailure.Unreadable);
                }
                if (!stillHere() || batch.IsCancellationRequested)
                {
                    return null;
                }
                if (outcome.Media is { } media)
                {
                    items.Add(media);
                }
                else
                {
                    said = Sentence(outcome.Failure ?? PrepFailure.Unreadable, say);
                }
            }
            return said;
        }
        finally
        {
            Preparing = false;
            preparing = null;
        }
    }

    public static string CapSentence(IStringCatalog say) =>
        say.Format("You can attach up to %lld items.", MediaPrep.MaxPerMessage);

    public static string Sentence(PrepFailure failure, IStringCatalog say) => failure switch
    {
        PrepFailure.TooLarge => say.Get("That file is over the 100 MB limit."),
        _ => say.Get("Couldn't read that file."),
    };

    /// <summary>What a staged item is called on its chip: a picture by its kind, a file by its name and size.</summary>
    public static string Label(StagedMedia item, IStringCatalog say, CultureInfo? culture = null) => item.Kind switch
    {
        "photo" => say.Get("Photo"),
        "video" => say.Get("Video"),
        "audio" or "file" =>
            $"{item.Name ?? (item.Kind == "audio" ? say.Get("Voice message") : say.Get("File"))} · {MediaText.DisplaySize(item.Bytes.Length, say, culture)}",
        _ => item.Name ?? say.Get("File"),
    };
}
