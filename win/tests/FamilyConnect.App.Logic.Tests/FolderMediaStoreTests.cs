using Xunit;

namespace FamilyConnect.App.Logic.Tests;

public sealed class FolderMediaStoreTests : IDisposable
{
    private readonly string folder = Path.Combine(Path.GetTempPath(), "fc-staging-" + Guid.NewGuid().ToString("N"));

    public void Dispose()
    {
        if (Directory.Exists(folder))
        {
            Directory.Delete(folder, recursive: true);
        }
    }

    [Fact]
    public void WhatIsStagedReadsBackAsItWasWritten()
    {
        var store = new FolderMediaStore(folder);
        var media = new StagedMedia("video", "video/mp4", new byte[] { 1, 2, 3 }, 1920, 1080, 4200, "clip ✨.mp4", new byte[] { 9, 8 });

        var handle = store.Stage(media);
        var read = store.Read(handle);

        Assert.NotNull(read);
        Assert.Equal(("video", "video/mp4", 1920, 1080, 4200, "clip ✨.mp4"),
            (read.Kind, read.Mime, read.Width, read.Height, read.DurationMs, read.Name));
        Assert.Equal(new byte[] { 1, 2, 3 }, read.Bytes.ToArray());
        Assert.Equal(new byte[] { 9, 8 }, read.Preview!.Value.ToArray());
    }

    [Fact]
    public void MediaWithoutOptionalFactsReadsBackWithoutThem()
    {
        var store = new FolderMediaStore(folder);

        var read = store.Read(store.Stage(new StagedMedia("file", "application/pdf", new byte[] { 7 })));

        Assert.NotNull(read);
        Assert.Null(read.Width);
        Assert.Null(read.DurationMs);
        Assert.Null(read.Name);
        Assert.Null(read.Preview);
        Assert.Null(read.Latitude);
        Assert.Null(read.AccuracyM);
    }

    /// <summary>A place has no bytes, and reads back with its numbers exactly — a centimetre off is a different place.</summary>
    [Fact]
    public void APlaceReadsBackWithItsNumbersAndNoBytes()
    {
        var store = new FolderMediaStore(folder);

        var read = store.Read(store.Stage(new StagedMedia("location", string.Empty, ReadOnlyMemory<byte>.Empty,
            Latitude: 55.00390625, Longitude: -0.1, AccuracyM: 12.5)));

        Assert.NotNull(read);
        Assert.Equal("location", read.Kind);
        Assert.Equal(55.00390625, read.Latitude);
        Assert.Equal(-0.1, read.Longitude);
        Assert.Equal(12.5, read.AccuracyM);
        Assert.True(read.Bytes.IsEmpty);
        Assert.Null(read.Preview);
    }

    /// <summary>An accuracy that is not a number is not written — JSON has no NaN — and the place still stages.</summary>
    [Fact]
    public void APlaceWhoseAccuracyIsNotANumberStagesWithoutOne()
    {
        var store = new FolderMediaStore(folder);

        var read = store.Read(store.Stage(new StagedMedia("location", string.Empty, ReadOnlyMemory<byte>.Empty,
            Latitude: 1, Longitude: 2, AccuracyM: double.NaN)));

        Assert.NotNull(read);
        Assert.Equal(2, read.Longitude);
        Assert.Null(read.AccuracyM);
    }

    /// <summary>
    /// PINNED UNTIL QUEUED: a flush on another thread sweeps between "written" and "queued", when the
    /// outbox names nothing — and must not take the files with it.
    /// </summary>
    [Fact]
    public void ASweepLeavesAPinnedHandleAndTakesItOnceReleased()
    {
        var store = new FolderMediaStore(folder);
        var handle = store.Stage(new StagedMedia("photo", "image/jpeg", new byte[] { 1 }));

        store.Sweep(new HashSet<string>());
        Assert.NotNull(store.Read(handle));

        store.Release([handle]);
        store.Sweep(new HashSet<string> { handle });
        Assert.NotNull(store.Read(handle));

        store.Sweep(new HashSet<string>());
        Assert.Null(store.Read(handle));
    }

    [Fact]
    public void ASweepTakesLeftoverPartsAndStrangers()
    {
        var store = new FolderMediaStore(folder);
        Directory.CreateDirectory(Path.Combine(folder, "0123456789abcdef0123456789abcdef.part"));
        File.WriteAllText(Path.Combine(folder, "stray.txt"), "x");
        var kept = store.Stage(new StagedMedia("photo", "image/jpeg", new byte[] { 1 }));
        store.Release([kept]);

        store.Sweep(new HashSet<string> { kept });

        Assert.Equal([kept], Directory.EnumerateFileSystemEntries(folder).Select(Path.GetFileName));
    }

    /// <summary>A handle comes back out of the cache: anything but this store's own is not a path.</summary>
    [Theory]
    [InlineData("../../etc")]
    [InlineData("0123456789ABCDEF0123456789ABCDEF")]
    [InlineData("")]
    [InlineData("0123456789abcdef0123456789abcde")]
    public void AHandleThatIsNotOneReadsAsGone(string handle)
    {
        Assert.Null(new FolderMediaStore(folder).Read(handle));
    }

    [Fact]
    public void HalfAFolderIsNoFolder()
    {
        var store = new FolderMediaStore(folder);
        var handle = store.Stage(new StagedMedia("photo", "image/jpeg", new byte[] { 1 }));
        File.Delete(Path.Combine(folder, handle, "meta.json"));

        Assert.Null(store.Read(handle));
    }

    [Fact]
    public void ASweepOfAFolderNeverMadeIsNothing()
    {
        new FolderMediaStore(folder).Sweep(new HashSet<string>());
        Assert.False(Directory.Exists(folder));
    }

    /// <summary>
    /// The same rule against REAL folders: a handle that climbs into a neighbour, or spells a real
    /// handle in capitals — which NTFS and APFS would both find — is not read.
    /// </summary>
    [Fact]
    public void AHandleThatReachesARealFolderSomeOtherWayIsNotRead()
    {
        var neighbour = new FolderMediaStore(Path.Combine(folder, "a"));
        var handle = neighbour.Stage(new StagedMedia("photo", "image/jpeg", new byte[] { 1 }));
        Directory.CreateDirectory(Path.Combine(folder, "b"));
        var store = new FolderMediaStore(Path.Combine(folder, "b"));

        Assert.NotNull(neighbour.Read(handle));
        Assert.Null(store.Read($"../a/{handle}"));
        Assert.Null(neighbour.Read(handle.ToUpperInvariant()));
    }

    /// <summary>A part named for a handle the sweep must keep is that handle's write in progress, and stays.</summary>
    [Fact]
    public void APartForAHandleThatIsKeptStays()
    {
        const string handle = "0123456789abcdef0123456789abcdef";
        Directory.CreateDirectory(Path.Combine(folder, handle + ".part"));

        new FolderMediaStore(folder).Sweep(new HashSet<string> { handle });

        Assert.True(Directory.Exists(Path.Combine(folder, handle + ".part")));
    }
}
