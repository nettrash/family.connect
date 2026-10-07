using FamilyConnect.App.Logic;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The store of voice messages that were not sent (S2.8): per account, on disk, whole or not at all, swept at launch and
/// wiped when the session ends.
/// </summary>
public sealed class ParkedRecordingsTests : IDisposable
{
    private static readonly Uri Server = new("https://family.example/");
    private static readonly DateTimeOffset At = new(2026, 10, 5, 12, 0, 0, TimeSpan.Zero);

    private readonly string root = Path.Combine(Path.GetTempPath(), "fc-parked-" + Guid.NewGuid().ToString("N"));

    public void Dispose()
    {
        if (Directory.Exists(root))
        {
            Directory.Delete(root, recursive: true);
        }
    }

    private ParkedRecordings Store(long account = 5, Uri? server = null) => ParkedRecordings.For(root, server ?? Server, account);

    private static byte[] Voice(int length = 4096, byte fill = 7) => Enumerable.Repeat(fill, length).ToArray();

    [Fact]
    public void AParkedRecordingReadsBackAsItWasParked()
    {
        var store = Store();
        var parked = store.Park(42, Voice(fill: 3), 12_345, replyTo: 9, caption: "Happy birthday 🎂\nfrom all of us", At);

        var read = Assert.Single(store.Of(42));
        Assert.Equal(parked, read);
        Assert.Equal((42L, 12_345, 9L, "Happy birthday 🎂\nfrom all of us", At), (read.ChatId, read.DurationMs, read.ReplyToMessageId, read.Caption, read.ParkedAt));

        var staged = store.Staged(read)!;
        Assert.Equal(("audio", "audio/mp4", 12_345), (staged.Kind, staged.Mime, staged.DurationMs));
        Assert.Null(staged.Name);
        Assert.Equal(Voice(fill: 3), staged.Bytes.ToArray());
        // Exactly what a recording staged at Stop is, so it travels as one.
        Assert.True(VoiceNotes.IsRecorded(staged));
    }

    [Fact]
    public void WithoutAReplyOrACaptionItHasNeither()
    {
        var store = Store();
        var parked = store.Park(42, Voice(), 3_000, replyTo: null, caption: "   ", At);

        var read = Assert.Single(store.Of(42));
        Assert.Null(read.ReplyToMessageId);
        Assert.Null(read.Caption);
        // What the park answers is what any later read finds: the caller holds the same entry the row is drawn from.
        Assert.Equal(read, parked);
    }

    /// <summary>The app closing is not the end of it: another store over the same folder — the next launch — finds it.</summary>
    [Fact]
    public void ItSurvivesTheAppBeingClosed()
    {
        Store().Park(42, Voice(), 3_000, null, "later", At);

        var read = Assert.Single(Store().Of(42));
        Assert.Equal("later", read.Caption);
    }

    [Fact]
    public void EachChatHasItsOwnOldestFirst()
    {
        var store = Store();
        var second = store.Park(1, Voice(), 2_000, null, null, At.AddMinutes(1));
        var other = store.Park(2, Voice(), 3_000, null, null, At);
        var first = store.Park(1, Voice(), 1_000, null, null, At);

        Assert.Equal([first, second], store.Of(1));
        Assert.Equal([other], store.Of(2));
        Assert.Empty(store.Of(3));
        Assert.True(store.Any(1));
        Assert.False(store.Any(3));
        Assert.Equal(3, store.All().Count);
    }

    [Fact]
    public void RemovedIsGoneFileAndAll()
    {
        var store = Store();
        var kept = store.Park(1, Voice(), 2_000, null, null, At);
        var sent = store.Park(1, Voice(), 3_000, null, null, At.AddSeconds(1));

        Assert.True(store.Remove(sent));

        Assert.Equal([kept], store.Of(1));
        Assert.False(Directory.Exists(Path.Combine(store.Folder, sent.Id)));
        Assert.Null(store.Staged(sent));
    }

    /// <summary>Nobody else signed in on this device sees them — nor the same id on another server.</summary>
    [Fact]
    public void EachAccountOnEachServerHasItsOwn()
    {
        Store(account: 5).Park(1, Voice(), 2_000, null, "mine", At);

        Assert.Empty(Store(account: 6).Of(1));
        Assert.Empty(Store(account: 5, server: new Uri("https://other.example/")).Of(1));
        Assert.Empty(Store(account: 5, server: new Uri("https://family.example/elsewhere/")).Of(1));
        Assert.Single(Store(account: 5).Of(1));
        // The folder is named for neither the address nor the id.
        var name = Path.GetFileName(Store(account: 5).Folder);
        Assert.DoesNotContain("family", name);
        Assert.Equal(32, name.Length);
    }

    /// <summary>A session ending takes every account's recordings with it, as it takes the cache and its outbox.</summary>
    [Fact]
    public void AWipeTakesEveryAccountsRecordings()
    {
        Store(account: 5).Park(1, Voice(), 2_000, null, null, At);
        Store(account: 6).Park(1, Voice(), 2_000, null, null, At);

        ParkedRecordings.WipeAll(root);

        Assert.Empty(Store(account: 5).All());
        Assert.Empty(Store(account: 6).All());
        Assert.False(Directory.Exists(root));
        // And wiping nothing is nothing.
        ParkedRecordings.WipeAll(root);
    }

    /// <summary>A part a crash left behind is never an entry, and the launch's sweep takes it — with anything else no entry names.</summary>
    [Fact]
    public void WhatACrashLeftIsNeverReadAndIsSwept()
    {
        var store = Store();
        var good = store.Park(1, Voice(), 2_000, null, null, At);
        var folder = store.Folder;
        var part = Path.Combine(folder, Guid.NewGuid().ToString("N") + ".part");
        Directory.CreateDirectory(part);
        File.WriteAllBytes(Path.Combine(part, "voice.m4a"), Voice());
        var noMeta = Path.Combine(folder, Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(noMeta);
        File.WriteAllBytes(Path.Combine(noMeta, "voice.m4a"), Voice());
        var noBytes = Path.Combine(folder, Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(noBytes);
        File.WriteAllText(Path.Combine(noBytes, "meta.json"), """{"chat_id":1,"duration_ms":2000,"parked_at_ms":0}""");
        var brokenMeta = Path.Combine(folder, Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(brokenMeta);
        File.WriteAllBytes(Path.Combine(brokenMeta, "voice.m4a"), Voice());
        File.WriteAllText(Path.Combine(brokenMeta, "meta.json"), """{"chat_id":"one","duration_ms":2000""");
        var notAnId = Path.Combine(folder, "notes");
        Directory.CreateDirectory(notAnId);
        File.WriteAllText(Path.Combine(folder, "stray.txt"), "x");

        Assert.Equal([good], store.All());

        store.Sweep();

        Assert.Equal([good], store.All());
        Assert.Equal([Path.Combine(folder, good.Id)], Directory.EnumerateFileSystemEntries(folder));
    }

    /// <summary>An entry whose numbers make no sense is no entry: a chat id must be a chat, a length cannot be negative.</summary>
    [Theory]
    [InlineData("""{"chat_id":0,"duration_ms":2000,"parked_at_ms":0}""")]
    [InlineData("""{"chat_id":-4,"duration_ms":2000,"parked_at_ms":0}""")]
    [InlineData("""{"chat_id":1,"duration_ms":-1,"parked_at_ms":0}""")]
    [InlineData("""{"chat_id":1,"duration_ms":4294967296,"parked_at_ms":0}""")]
    [InlineData("""{"chat_id":1,"parked_at_ms":0}""")]
    [InlineData("""{"chat_id":1,"duration_ms":2000}""")]
    [InlineData("""{"chat_id":1,"duration_ms":2000,"parked_at_ms":999999999999999999}""")]
    [InlineData("""[1,2,3]""")]
    public void NonsenseIsNoEntry(string meta)
    {
        var folder = Path.Combine(Store().Folder, Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(folder);
        File.WriteAllBytes(Path.Combine(folder, "voice.m4a"), Voice());
        File.WriteAllText(Path.Combine(folder, "meta.json"), meta);

        Assert.Empty(Store().All());
    }

    /// <summary>A reply id that is no message is no reply; the recording itself is still kept.</summary>
    [Fact]
    public void AReplyThatIsNoMessageIsDropped()
    {
        var folder = Path.Combine(Store().Folder, Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(folder);
        File.WriteAllBytes(Path.Combine(folder, "voice.m4a"), Voice());
        File.WriteAllText(Path.Combine(folder, "meta.json"), """{"chat_id":1,"duration_ms":2000,"reply_to_message_id":0,"parked_at_ms":0}""");

        Assert.Null(Assert.Single(Store().All()).ReplyToMessageId);
    }

    /// <summary>An id that comes back from anywhere but this store is not a path: nothing outside the folder is read or deleted.</summary>
    [Fact]
    public void AnIdIsNotAPath()
    {
        var store = Store();
        var parked = store.Park(1, Voice(), 2_000, null, null, At);
        var outside = Path.Combine(root, "precious");
        Directory.CreateDirectory(outside);
        File.WriteAllBytes(Path.Combine(outside, "voice.m4a"), Voice());

        var forged = parked with { Id = Path.Combine("..", "precious") };
        Assert.False(store.Remove(forged));
        Assert.Null(store.Staged(forged));
        Assert.False(store.Remove(parked with { Id = parked.Id.ToUpperInvariant() }));

        Assert.True(File.Exists(Path.Combine(outside, "voice.m4a")));
        Assert.Single(store.All());
    }

    /// <summary>A park that cannot be written throws, so the caller keeps the recording some other way — and leaves no part behind.</summary>
    [Fact]
    public void APartThatCannotLandThrowsAndLeavesNothing()
    {
        var store = Store();
        // A FILE where the account's folder should be: nothing can be written under it.
        Directory.CreateDirectory(root);
        File.WriteAllText(store.Folder, "in the way");

        Assert.ThrowsAny<IOException>(() => store.Park(1, Voice(), 2_000, null, null, At));

        File.Delete(store.Folder);
        Assert.Empty(store.All());
    }

    /// <summary>
    /// WHAT THE DISK REFUSES IS HELD IN MEMORY (S4: the disk fills): the same entry a park would make — its chat, length, reply
    /// and caption — listed, played and sent like any other, so it is still its own row, never carried by another Send
    /// (S2.8), and its reply and caption are never lost; deleted, it is gone.
    /// </summary>
    [Fact]
    public void WhatTheDiskRefusesIsHeldAsTheSameRow()
    {
        var store = Store();
        Directory.CreateDirectory(root);
        File.WriteAllText(store.Folder, "in the way");
        Assert.ThrowsAny<IOException>(() => store.Park(42, Voice(fill: 5), 12_345, replyTo: 9, caption: "for you", At));

        var held = store.Hold(42, Voice(fill: 5), 12_345, replyTo: 9, caption: "for you", At);

        Assert.True(store.HoldsInMemory);
        Assert.Equal([held], store.Of(42));
        Assert.True(store.Any(42));
        Assert.False(store.Any(43));
        Assert.Equal((42L, 12_345, 9L, "for you"), (held.ChatId, held.DurationMs, held.ReplyToMessageId, held.Caption));
        var staged = store.Staged(held)!;
        Assert.Equal(Voice(fill: 5), staged.Bytes.ToArray());
        Assert.Equal(12_345, staged.DurationMs);
        Assert.True(VoiceNotes.IsRecorded(staged));
        // Its id is an id like any other: nothing forged reaches it.
        Assert.False(store.Remove(held with { Id = Path.Combine("..", held.Id) }));

        Assert.True(store.Remove(held));
        Assert.Empty(store.Of(42));
        Assert.False(store.HoldsInMemory);
        Assert.Null(store.Staged(held));
    }

    /// <summary>Held and written side by side, oldest first: one chat's rows are one list, wherever each is kept.</summary>
    [Fact]
    public void HeldAndWrittenAreOneListOldestFirst()
    {
        var store = Store();
        var written = store.Park(1, Voice(), 2_000, null, null, At.AddMinutes(1));
        var held = store.Hold(1, Voice(), 3_000, null, "held", At);

        Assert.Equal([held, written], store.Of(1));
        // Another store over the same folder — the next launch — finds only what was written.
        Assert.Equal([written], Store().Of(1));
    }

    /// <summary>
    /// A real close tries the disk again: whatever it takes moves there under the SAME id — the row drawn for it is still its
    /// row — and survives the app; whatever it still refuses stays held.
    /// </summary>
    [Fact]
    public void AHeldRecordingIsWrittenOnceTheDiskTakesIt()
    {
        var store = Store();
        Directory.CreateDirectory(root);
        File.WriteAllText(store.Folder, "in the way");
        var held = store.Hold(42, Voice(fill: 9), 4_000, replyTo: 3, caption: "later", At);

        Assert.Equal(1, store.WriteHeld());
        Assert.True(store.HoldsInMemory);

        File.Delete(store.Folder);
        Assert.Equal(0, store.WriteHeld());

        Assert.False(store.HoldsInMemory);
        Assert.Equal([held], store.Of(42));
        var relaunched = Assert.Single(Store().Of(42));
        Assert.Equal(held, relaunched);
        Assert.Equal(Voice(fill: 9), Store().Staged(relaunched)!.Bytes.ToArray());
    }

    /// <summary>A session ending takes what is held in memory with it, as the wipe takes what is written (S4's sign-out row).</summary>
    [Fact]
    public void WhatIsHeldGoesWithTheSession()
    {
        var store = Store();
        store.Hold(1, Voice(), 2_000, null, null, At);

        store.ForgetHeld();

        Assert.False(store.HoldsInMemory);
        Assert.Empty(store.All());
        // Nothing held: nothing to write.
        Assert.Equal(0, store.WriteHeld());
    }
}
