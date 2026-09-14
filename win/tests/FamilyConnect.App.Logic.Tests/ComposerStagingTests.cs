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
}
