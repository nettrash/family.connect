using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Store;

/// <summary>
/// The text of recordings this member asked for, kept on this device (docs/protocol.md, "Transcripts on request").
/// </summary>
public sealed class TranscriptStoreTests : IDisposable
{
    private readonly Database cache = Database.OpenInMemory();
    private readonly TranscriptStore store;

    public TranscriptStoreTests() => store = new TranscriptStore(cache);

    public void Dispose() => cache.Dispose();

    [Fact]
    public void NothingIsHeldUntilSomebodyAsked()
    {
        Assert.Null(store.Find(34));
    }

    [Fact]
    public void AnAnswerIsKeptByItsAttachmentWithItsLanguage()
    {
        store.Keep(new KeptTranscript(34, "Мы будем в шесть", "ru"), DateTimeOffset.UnixEpoch);
        store.Keep(new KeptTranscript(35, "Back at six"), DateTimeOffset.UnixEpoch);

        Assert.Equal(new KeptTranscript(34, "Мы будем в шесть", "ru"), store.Find(34));
        Assert.Equal(new KeptTranscript(35, "Back at six"), store.Find(35));
        Assert.Null(store.Find(36));
    }

    /// <summary>SILENCE IS AN ANSWER: kept as the empty text it is, never as "not asked".</summary>
    [Fact]
    public void SilenceIsKeptAsAnAnswer()
    {
        store.Keep(new KeptTranscript(34, ""), DateTimeOffset.UnixEpoch);
        var kept = store.Find(34);
        Assert.NotNull(kept);
        Assert.Equal(string.Empty, kept.Text);
    }

    /// <summary>
    /// The stored-bytes answer is the one every member gets, and the server returns it instead of reading supplied
    /// sound once it exists — so a supplied answer never replaces it here either; the other way round, it does.
    /// </summary>
    [Fact]
    public void AStoredAnswerIsNeverReplacedByASuppliedOne()
    {
        store.Keep(new KeptTranscript(34, "from the server's copy"), DateTimeOffset.UnixEpoch);
        store.Keep(new KeptTranscript(34, "from this device's sound", Supplied: true), DateTimeOffset.UnixEpoch);
        Assert.Equal(new KeptTranscript(34, "from the server's copy"), store.Find(34));

        store.Keep(new KeptTranscript(35, "from this device's sound", Supplied: true), DateTimeOffset.UnixEpoch);
        Assert.True(store.Find(35)!.Supplied);
        store.Keep(new KeptTranscript(35, "from the server's copy", "en"), DateTimeOffset.UnixEpoch);
        Assert.Equal(new KeptTranscript(35, "from the server's copy", "en"), store.Find(35));
    }

    /// <summary>The text of somebody's voice belongs to the account that asked: a sign-out takes it.</summary>
    [Fact]
    public void ASignOutTakesTheTextWithIt()
    {
        store.Keep(new KeptTranscript(34, "See you at six"), DateTimeOffset.UnixEpoch);
        cache.WipeAll();
        Assert.Null(store.Find(34));
    }
}
