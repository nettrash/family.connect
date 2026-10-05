using System.Globalization;
using FamilyConnect.Core;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

public sealed class ComposerStagingTests
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    private static StagedMedia Photo() => new("photo", "image/jpeg", new byte[] { 1 }, 10, 10);

    private static Func<string, Task<PrepOutcome>> Preparer(List<string> asked, params string[] unreadable) =>
        name =>
        {
            asked.Add(name);
            return Task.FromResult(unreadable.Contains(name)
                ? PrepOutcome.Refused(PrepFailure.Unreadable)
                : PrepOutcome.Staged(new StagedMedia("file", "text/plain", new byte[] { 1, 2 }, Name: name)));
        };

    [Fact]
    public async Task FilesAreStagedInTheOrderTheyWerePicked()
    {
        var staging = new ComposerStaging();
        var asked = new List<string>();

        var said = await staging.IngestAsync(["a.txt", "b.txt", "c.txt"], Preparer(asked), () => true, Say);

        Assert.Null(said);
        Assert.Equal(["a.txt", "b.txt", "c.txt"], staging.Items.Select(item => item.Name));
        Assert.False(staging.Preparing);
    }

    /// <summary>STOPPED AT THE CAP, not preparing the rest only to throw them away — and saying so.</summary>
    [Fact]
    public async Task ABatchStopsAtTheCapAndSaysSo()
    {
        var staging = new ComposerStaging();
        for (var i = 0; i < MediaPrep.MaxPerMessage - 2; i++)
        {
            Assert.True(staging.Add(Photo()));
        }
        var asked = new List<string>();

        var said = await staging.IngestAsync(["a", "b", "c", "d"], Preparer(asked), () => true, Say);

        Assert.Equal(MediaPrep.MaxPerMessage, staging.Items.Count);
        Assert.Equal(["a", "b"], asked);
        Assert.Equal("You can attach up to 10 items.", said);
        Assert.False(staging.Add(Photo()));
    }

    [Fact]
    public async Task ARefusedFileIsSaidAndTheRestStillStaged()
    {
        var staging = new ComposerStaging();

        var said = await staging.IngestAsync(["a", "broken", "c"], Preparer([], "broken"), () => true, Say);

        Assert.Equal(["a", "c"], staging.Items.Select(item => item.Name));
        Assert.Equal("Couldn't read that file.", said);
    }

    [Fact]
    public async Task APreparerThatThrowsIsAnUnreadableFileNotACrash()
    {
        var staging = new ComposerStaging();

        var said = await staging.IngestAsync<string>(
            ["x"], _ => throw new IOException("locked"), () => true, Say);

        Assert.Empty(staging.Items);
        Assert.Equal("Couldn't read that file.", said);
        Assert.False(staging.Preparing);
    }

    /// <summary>A pane that went away mid-batch: nothing more lands, and nothing is said.</summary>
    [Fact]
    public async Task APaneThatWentAwayStopsTheBatch()
    {
        var staging = new ComposerStaging();
        var here = true;
        var asked = new List<string>();
        Task<PrepOutcome> Prepare(string name)
        {
            asked.Add(name);
            here = false;
            return Task.FromResult(PrepOutcome.Staged(Photo()));
        }

        var said = await staging.IngestAsync(["a", "b"], Prepare, () => here, Say);

        Assert.Null(said);
        Assert.Empty(staging.Items);
        Assert.Equal(["a"], asked);
        Assert.False(staging.Preparing);
    }

    /// <summary>A pane already gone when the files arrive prepares none of them.</summary>
    [Fact]
    public async Task APaneAlreadyGonePreparesNothing()
    {
        var staging = new ComposerStaging();
        var asked = new List<string>();

        var said = await staging.IngestAsync(["a", "b"], Preparer(asked), () => false, Say);

        Assert.Null(said);
        Assert.Empty(asked);
        Assert.Empty(staging.Items);
    }

    [Fact]
    public async Task PreparingIsTheOtherBusy()
    {
        var staging = new ComposerStaging();
        var gate = new TaskCompletionSource<PrepOutcome>();
        var batch = staging.IngestAsync(["a"], _ => gate.Task, () => true, Say);

        Assert.True(staging.Preparing);
        Assert.Equal("Wait until the current attachment is done.", staging.BusyReason(editing: false, Say));
        Assert.Equal("Finish editing before attaching something.", staging.BusyReason(editing: true, Say));

        gate.SetResult(PrepOutcome.Refused(PrepFailure.TooLarge));
        Assert.Equal("That file is over the 100 MB limit.", await batch);
        Assert.Null(staging.BusyReason(editing: false, Say));
    }

    /// <summary>
    /// A ten-minute clip dropped by mistake: Cancel reaches the preparer through its token, nothing of the rest is
    /// started, nothing is said, and the composer is free again at once — not after the transcode.
    /// </summary>
    [Fact]
    public async Task ABatchCalledOffStopsThePreparerAndStartsNothingMore()
    {
        var staging = new ComposerStaging();
        var asked = new List<string>();
        var started = new TaskCompletionSource();
        async Task<PrepOutcome> Prepare(string name, CancellationToken cancel)
        {
            asked.Add(name);
            if (name == "clip.mov")
            {
                started.SetResult();
                await Task.Delay(Timeout.Infinite, cancel);
            }
            return PrepOutcome.Staged(new StagedMedia("file", "text/plain", new byte[] { 1 }, Name: name));
        }

        var batch = staging.IngestAsync(["a.txt", "clip.mov", "c.txt"], Prepare, () => true, Say);
        await started.Task;
        Assert.True(staging.Preparing);
        staging.CancelPreparing();

        // Bounded: a cancel that never reached the preparer must FAIL here, not hang the suite on its endless clip.
        Assert.Null(await batch.WaitAsync(TimeSpan.FromSeconds(30)));
        Assert.Equal(["a.txt", "clip.mov"], asked);
        // What was staged before the cancel stays: it has a chip of its own to remove it by.
        Assert.Equal(["a.txt"], staging.Items.Select(item => item.Name));
        Assert.False(staging.Preparing);
        Assert.Null(staging.BusyReason(editing: false, Say));
    }

    /// <summary>A preparer that did not notice the cancel and came back with the file anyway: it is still not staged.</summary>
    [Fact]
    public async Task ABatchCalledOffStagesNothingThatCameBackAnyway()
    {
        var staging = new ComposerStaging();
        var gate = new TaskCompletionSource<PrepOutcome>();

        var batch = staging.IngestAsync(["a", "b"], (_, _) => gate.Task, () => true, Say);
        staging.CancelPreparing();
        gate.SetResult(PrepOutcome.Staged(Photo()));

        Assert.Null(await batch);
        Assert.Empty(staging.Items);
    }

    /// <summary>Cancel is for the batch in hand: with none running it does nothing, and the next batch is untouched by it.</summary>
    [Fact]
    public async Task CallingOffNothingLeavesTheNextBatchAlone()
    {
        var staging = new ComposerStaging();
        staging.CancelPreparing();

        var said = await staging.IngestAsync(["a"], Preparer([]), () => true, Say);
        staging.CancelPreparing();

        Assert.Null(said);
        Assert.Equal(["a"], staging.Items.Select(item => item.Name));
        Assert.Null(await staging.IngestAsync(["b"], Preparer([]), () => true, Say));
        Assert.Equal(["a", "b"], staging.Items.Select(item => item.Name));
    }

    [Fact]
    public void TakingEmptiesTheStripAndRestoringPutsItBackInFront()
    {
        var staging = new ComposerStaging();
        staging.Add(new StagedMedia("file", "text/plain", new byte[1], Name: "first"));
        var taken = staging.TakeAll();
        Assert.Empty(staging.Items);

        staging.Add(new StagedMedia("file", "text/plain", new byte[1], Name: "later"));
        staging.Restore(taken);

        Assert.Equal(["first", "later"], staging.Items.Select(item => item.Name));
        staging.Remove(0);
        staging.Remove(7);
        Assert.Equal(["later"], staging.Items.Select(item => item.Name));
    }

    /// <summary>
    /// Leaving the chat takes the voice notes recorded here out of the strip, in their order, to wait as "not sent"
    /// (S2.8) — a sound file picked from disk and everything else staged stay where they are.
    /// </summary>
    [Fact]
    public void LeavingTakesOnlyTheRecordingsMadeHere()
    {
        var staging = new ComposerStaging();
        var first = VoiceNotes.Staged(new byte[2048], TimeSpan.FromSeconds(3))!;
        var second = VoiceNotes.Staged(new byte[4096], TimeSpan.FromSeconds(12))!;
        var song = new StagedMedia("audio", "audio/mp4", new byte[2048], DurationMs: 3000, Name: "song.m4a");
        staging.Add(Photo());
        staging.Add(first);
        staging.Add(song);
        staging.Add(second);
        Assert.True(staging.HoldsRecordings);

        var taken = staging.TakeRecordings();

        Assert.Equal([first, second], taken);
        Assert.Equal(["photo", "audio"], staging.Items.Select(item => item.Kind));
        Assert.Equal("song.m4a", staging.Items[1].Name);
        Assert.False(staging.HoldsRecordings);
        Assert.Empty(staging.TakeRecordings());
    }

    [Fact]
    public void AChipNamesAPictureByItsKindAndAFileByItsNameAndSize()
    {
        var culture = CultureInfo.InvariantCulture;
        Assert.Equal("Photo", ComposerStaging.Label(Photo(), Say, culture));
        Assert.Equal("Video", ComposerStaging.Label(new StagedMedia("video", "video/mp4", new byte[3]), Say, culture));
        Assert.Equal(
            $"report.pdf · {MediaText.DisplaySize(2048, Say, culture)}",
            ComposerStaging.Label(new StagedMedia("file", "application/pdf", new byte[2048], Name: "report.pdf"), Say, culture));
        Assert.Equal(
            $"File · {MediaText.DisplaySize(1, Say, culture)}",
            ComposerStaging.Label(new StagedMedia("file", "application/pdf", new byte[1]), Say, culture));
    }

    /// <summary>
    /// A voice note recorded here is named by its LENGTH on its chip, "Voice message · 0:42" (S2.7, S10) — its size says
    /// nothing to somebody deciding whether to send it — while a sound file picked from disk keeps its name and size.
    /// </summary>
    [Fact]
    public void ARecordedNoteIsNamedByItsLength()
    {
        var culture = CultureInfo.InvariantCulture;
        var note = VoiceNotes.Staged(new byte[64 * 1024], TimeSpan.FromSeconds(42))!;
        Assert.Equal("Voice message · 0:42", ComposerStaging.Label(note, Say, culture));
        Assert.Equal("Voice message · 5:00", ComposerStaging.Label(VoiceNotes.Staged(new byte[4096], VoiceNotes.Longest)!, Say, culture));
        Assert.Equal(
            $"song.m4a · {MediaText.DisplaySize(2048, Say, culture)}",
            ComposerStaging.Label(new StagedMedia("audio", "audio/mp4", new byte[2048], DurationMs: 3000, Name: "song.m4a"), Say, culture));
        Assert.NotEqual("Voice message · 0:42", ComposerStaging.Label(note, JsonCatalog.For("ru"), culture));
        Assert.EndsWith("0:42", ComposerStaging.Label(note, JsonCatalog.For("ru"), culture));
    }

    /// <summary>
    /// "Delete this recording?" may be answered after other items came or went: the note is taken out by REFERENCE — not by
    /// its old place, and not an equal copy of it — and a note already gone is reported gone.
    /// </summary>
    [Fact]
    public void AnItemIsRemovedByReference()
    {
        var staging = new ComposerStaging();
        var bytes = new byte[2048];
        var first = new StagedMedia("audio", "audio/mp4", bytes, DurationMs: 12_000);
        var twin = new StagedMedia("audio", "audio/mp4", bytes, DurationMs: 12_000);
        Assert.Equal(first, twin);
        staging.Add(Photo());
        staging.Add(twin);
        staging.Add(first);
        staging.Remove(0);

        Assert.True(staging.Remove(first));

        Assert.Single(staging.Items);
        Assert.Same(twin, staging.Items[0]);
        Assert.False(staging.Remove(first));
        Assert.Single(staging.Items);
    }
}
