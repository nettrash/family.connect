using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Store;

/// <summary>
/// The video messages this device has played — what the dot beside an unplayed circle is drawn from
/// (docs/audio-video-messages-2026-10-04.md, S5.2).
/// </summary>
public sealed class PlayedRoundStoreTests : IDisposable
{
    private readonly Database cache = Database.OpenInMemory();
    private readonly PlayedRoundStore store;

    public PlayedRoundStoreTests() => store = new PlayedRoundStore(cache);

    public void Dispose() => cache.Dispose();

    [Fact]
    public void NothingIsPlayedUntilItIs()
    {
        Assert.False(store.Played(91));
        Assert.Equal(0, store.Count);
    }

    /// <summary>Marked once, by attachment — and a second mark is not news, so nothing redraws for it.</summary>
    [Fact]
    public void APlayIsRememberedByItsAttachmentAndOnlyOnce()
    {
        Assert.True(store.MarkPlayed(91));
        Assert.False(store.MarkPlayed(91));
        Assert.True(store.Played(91));
        Assert.False(store.Played(92));
        Assert.Equal(1, store.Count);
    }

    /// <summary>
    /// THE NEWEST 5 000 AND NO MORE: ids only grow, so the lowest go first — even when an old circle is the one played
    /// last, it is the oldest that has to make room.
    /// </summary>
    [Fact]
    public void OnlyTheNewestFiveThousandAreKept()
    {
        Assert.Equal(5000, PlayedRoundStore.Kept);
        for (long id = 1; id <= PlayedRoundStore.Kept; id++)
        {
            store.MarkPlayed(id + 100);
        }
        Assert.Equal(PlayedRoundStore.Kept, store.Count);
        Assert.True(store.Played(101));

        store.MarkPlayed(100_000);
        Assert.Equal(PlayedRoundStore.Kept, store.Count);
        Assert.False(store.Played(101));
        Assert.True(store.Played(102));
        Assert.True(store.Played(100_000));

        // An OLDER one played now is below everything kept: it does not push a newer one out.
        store.MarkPlayed(5);
        Assert.Equal(PlayedRoundStore.Kept, store.Count);
        Assert.False(store.Played(5));
        Assert.True(store.Played(102));
    }

    /// <summary>The device's own knowledge, and this account's: a sign-out takes it with the rest of the cache.</summary>
    [Fact]
    public void ASignOutForgetsWhatWasPlayed()
    {
        store.MarkPlayed(91);
        cache.WipeAll();
        Assert.False(store.Played(91));
        Assert.Equal(0, store.Count);
    }
}
