using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Store;

/// <summary>
/// The family's sticker pack as this device holds it — the board's machinery one table over
/// (docs/protocol.md, "Sticker pack"): a full read replaces, a removal is remembered, every write
/// is guarded by <c>pack_seq</c>, and the cursor moves in three ways and no others.
/// </summary>
public class PackStoreTests : IDisposable
{
    private readonly Database database = Database.OpenInMemory();

    public void Dispose() => database.Dispose();

    private PackStore Store() => new(database);

    private static PackItemDto Item(long id, long seq, long by = 7, string? label = null, long size = 4096) =>
        new(id, by, new AttachmentDto(70 + id, "photo", "image/webp", size, 512, 512), "2026-09-13T10:00:00Z", seq, label);

    private static PackItemDto Tombstone(long id, long seq) => new(id, Deleted: true, PackSeq: seq);

    [Fact]
    public void AFullReadIsThePackInTheOrderItWasAdded()
    {
        var store = Store();
        // The wire's order is id ascending, and the store answers it whatever order it was told in.
        Assert.True(store.Replace([Item(9, 14), Item(5, 12, label: "party cat"), Item(7, 13)], 14));

        Assert.Equal([5L, 7, 9], store.Items().Select(item => item.Id));
        Assert.Equal(3, store.Count());
        Assert.Equal(14, store.Cursor);
        var first = store.Items()[0];
        Assert.Equal("party cat", first.Label);
        Assert.Equal(7, first.AddedBy);
        Assert.Equal(12, first.PackSeq);
        Assert.Equal("image/webp", first.Attachment!.Mime);
        Assert.Equal(4096, first.Attachment.Size);
        // No label is no label — never an empty one.
        Assert.Null(store.Item(7)!.Label);
    }

    /// <summary>
    /// A FULL READ REPLACES: an item somebody removed while this device was away is not on the
    /// answer, and must leave the panel.
    /// </summary>
    [Fact]
    public void AFullReadDropsWhatItDoesNotCarry()
    {
        var store = Store();
        store.Replace([Item(5, 12), Item(6, 13)], 13);

        store.Replace([Item(6, 13)], 15);

        Assert.Equal([6L], store.Items().Select(item => item.Id));
        Assert.Equal(15, store.Cursor);
    }

    /// <summary>
    /// …EXCEPT an item held above the read's own mark: it arrived after the read was taken, is
    /// not on that answer, and stays.
    /// </summary>
    [Fact]
    public void AFullReadKeepsAnItemThatArrivedAfterItWasTaken()
    {
        var store = Store();
        store.Replace([Item(5, 12)], 12);
        store.Apply(Item(8, 20), SeqRoute.LiveFrame);

        // A read taken at 14 cannot know about seq 20.
        store.Replace([Item(5, 12), Item(6, 14)], 14);

        Assert.Equal([5L, 6, 8], store.Items().Select(item => item.Id));
    }

    /// <summary>Two full reads can land in either order; the older one landing second changes nothing.</summary>
    [Fact]
    public void AnOlderFullReadLandingSecondIsIgnored()
    {
        var store = Store();
        Assert.True(store.Replace([Item(5, 12), Item(6, 14)], 14));

        Assert.False(store.Replace([Item(5, 12)], 12));

        Assert.Equal([5L, 6], store.Items().Select(item => item.Id));
        Assert.Equal(14, store.Cursor);
    }

    [Fact]
    public void ATombstoneRemovesAndIsRemembered()
    {
        var store = Store();
        store.Replace([Item(5, 12), Item(6, 13)], 13);

        Assert.True(store.Apply(Tombstone(5, 14)));
        Assert.Equal([6L], store.Items().Select(item => item.Id));

        // THE GONE SET: ids are never reused, so an older copy arriving late — a frame that
        // crossed the removal, a slower page — never brings the item back.
        Assert.False(store.Apply(Item(5, 12), SeqRoute.LiveFrame));
        Assert.False(store.Apply(Item(5, 99)));
        store.Replace([Item(5, 12), Item(6, 13)], 20);
        Assert.Equal([6L], store.Items().Select(item => item.Id));
    }

    /// <summary>A tombstone for an item this device never held is remembered, and is not news.</summary>
    [Fact]
    public void ATombstoneForAnItemNeverHeldChangesNothingDrawn()
    {
        var store = Store();

        Assert.False(store.Apply(Tombstone(5, 14)));

        Assert.False(store.Apply(Item(5, 12)));
        Assert.Empty(store.Items());
    }

    /// <summary>
    /// A full read that LEFT AN ITEM OUT has seen it removed, exactly as a tombstone would have
    /// said — so a late frame carrying the old copy cannot resurrect it.
    /// </summary>
    [Fact]
    public void AFullReadThatLeftAnItemOutRemembersItAsGone()
    {
        var store = Store();
        store.Replace([Item(5, 12), Item(6, 13)], 13);

        store.Replace([Item(6, 13)], 15);

        Assert.False(store.Apply(Item(5, 12), SeqRoute.LiveFrame));
        Assert.Equal([6L], store.Items().Select(item => item.Id));
    }

    /// <summary>An item is written only when the incoming seq is GREATER than the one held.</summary>
    [Fact]
    public void AnItemIsWrittenOnlyWhenItsSeqIsNewer()
    {
        var store = Store();
        store.Apply(Item(5, 12, label: "party cat"));

        Assert.False(store.Apply(Item(5, 12, label: "same seq")));
        Assert.False(store.Apply(Item(5, 11, label: "older")));

        Assert.Equal("party cat", store.Item(5)!.Label);
    }

    /// <summary>A live item is its picture; one without it is nothing this client can draw or send.</summary>
    [Fact]
    public void ALiveItemWithNoPictureIsNotStored()
    {
        var store = Store();

        Assert.False(store.Apply(new PackItemDto(5, 7, PackSeq: 12)));

        Assert.Empty(store.Items());
    }

    // ---- the cursor: three ways and no others --------------------------------

    [Fact]
    public void ACatchUpPageMovesTheCursorToItsHighestSeq()
    {
        var store = Store();
        store.Replace([Item(5, 12)], 12);

        // Tombstones are in the feed and move the seq like anything else. Three things changed what is drawn: two
        // stickers arrived and one left.
        Assert.Equal(3, store.Apply([Item(6, 13), Tombstone(5, 17), Item(7, 15)]));

        Assert.Equal(17, store.Cursor);
        Assert.Equal([6L, 7], store.Items().Select(item => item.Id));
    }

    /// <summary>
    /// A FRAME MOVES THE CURSOR ONLY ONCE THIS CONNECTION HAS CAUGHT UP. Before that the cursor is
    /// where the catch-up starts from, and a frame that jumped it would leave every change made
    /// while the socket was down unread for good.
    /// </summary>
    [Fact]
    public void AFrameDoesNotMoveTheCursorUntilTheConnectionHasCaughtUp()
    {
        var store = Store();
        store.Replace([Item(5, 12)], 12);
        store.Reconnected();

        // The item is applied — it is drawn at once — and the cursor stays where the feed needs it.
        Assert.True(store.Apply(Item(9, 20), SeqRoute.LiveFrame));
        Assert.Equal(12, store.Cursor);
        Assert.False(store.IsCaughtUp);

        // The catch-up then reads 13..19 from 12, which is the whole point.
        store.Apply([Item(6, 13), Item(9, 20)]);
        store.CaughtUp(store.Connection);
        Assert.Equal(20, store.Cursor);

        store.Apply(Item(10, 21), SeqRoute.LiveFrame);
        Assert.Equal(21, store.Cursor);
    }

    /// <summary>
    /// "Caught up" is said OF A CONNECTION. A pass that began on one and finishes after another
    /// has opened is holding answers older than the socket now listening, and its word is dropped:
    /// a frame on the new connection still may not move the cursor.
    /// </summary>
    [Fact]
    public void CatchingUpAnEarlierConnectionDoesNotCatchUpThisOne()
    {
        var store = Store();
        store.Replace([Item(5, 12)], 12);
        store.Reconnected();
        var began = store.Connection;
        store.Reconnected();

        Assert.False(store.CaughtUp(began));
        Assert.False(store.IsCaughtUp);
        store.Apply(Item(9, 20), SeqRoute.LiveFrame);
        Assert.Equal(12, store.Cursor);

        // The pass the newer connection started is the one that may say it.
        Assert.True(store.CaughtUp(store.Connection));
        store.Apply(Item(10, 21), SeqRoute.LiveFrame);
        Assert.Equal(21, store.Cursor);
    }

    /// <summary>A relaunch is a new connection: nothing has been caught up on yet.</summary>
    [Fact]
    public void AFreshStoreHasNotCaughtUp()
    {
        var first = Store();
        first.Replace([Item(5, 12)], 12);
        first.CaughtUp(first.Connection);

        var relaunched = Store();
        relaunched.Apply(Item(9, 20), SeqRoute.LiveFrame);

        Assert.False(relaunched.IsCaughtUp);
        Assert.Equal(12, relaunched.Cursor);
    }

    /// <summary>
    /// The item in the answer to this device's own POST — and what this device makes of its own
    /// DELETE — is applied under the per-item guard and MOVES NO CURSOR.
    /// </summary>
    [Fact]
    public void EvidenceMovesNoCursor()
    {
        var store = Store();
        store.Replace([Item(5, 12)], 12);
        store.CaughtUp(store.Connection);

        Assert.True(store.Apply(Item(9, 30), SeqRoute.Evidence));
        Assert.True(store.Removed(5));
        Assert.Equal(1, store.Apply([Item(10, 31)], SeqRoute.Evidence));

        Assert.Equal(12, store.Cursor);
        Assert.Equal([9L, 10], store.Items().Select(item => item.Id));
    }

    /// <summary>
    /// This device's own DELETE, answered: the item leaves at once, its "recently used" goes with
    /// it, and it is remembered as gone — so the frame for the ADD, arriving late, cannot put it
    /// back. Said of an item this device never held, it changes nothing drawn and is still remembered.
    /// </summary>
    [Fact]
    public void ThisDevicesOwnRemovalIsRememberedAndMovesNothing()
    {
        var store = Store();
        store.Replace([Item(5, 12), Item(6, 13)], 13);
        store.Used(5, DateTimeOffset.UnixEpoch);

        Assert.True(store.Removed(5));
        Assert.False(store.Removed(5));
        Assert.False(store.Removed(44));

        Assert.Equal([6L], store.Items().Select(item => item.Id));
        Assert.Empty(store.Recents());
        Assert.Equal(13, store.Cursor);
        Assert.False(store.Apply(Item(5, 12), SeqRoute.LiveFrame));
        Assert.False(store.Apply(Item(44, 40), SeqRoute.LiveFrame));
    }

    // ---- staleness: the same guard for a live item and a tombstone -----------

    /// <summary>
    /// AN ITEM — LIVE OR A TOMBSTONE — IS APPLIED ONLY WHEN ITS SEQ IS ABOVE THE ONE HELD. A
    /// tombstone that is not newer than the copy held is an older state of the item, and changes
    /// nothing; the real one, which always takes a newer seq than the add, removes.
    /// </summary>
    [Fact]
    public void ATombstoneIsAppliedOnlyWhenItsSeqIsAboveTheOneHeld()
    {
        var store = Store();
        store.Replace([Item(5, 12)], 12);

        Assert.False(store.Apply(Tombstone(5, 12), SeqRoute.LiveFrame));
        Assert.False(store.Apply(Tombstone(5, 11), SeqRoute.LiveFrame));
        Assert.Equal(0, store.Apply([Tombstone(5, 9)], SeqRoute.Evidence));
        Assert.Equal(5, Assert.Single(store.Items()).Id);
        // Not applied is not remembered either: the item is still one a newer copy may replace.
        Assert.True(store.Apply(Item(5, 13, label: "newer")));

        Assert.True(store.Apply(Tombstone(5, 14), SeqRoute.LiveFrame));
        Assert.Empty(store.Items());
    }

    /// <summary>
    /// A FULL READ IS IGNORED WHEN ITS MARK IS BELOW THE CURSOR ALREADY APPLIED — whichever of
    /// the three ways moved the cursor there. Here a catch-up page and then a frame on a caught-up
    /// connection did; a full read taken before either knows less than this device does, and
    /// applying it would drop what arrived since and set the cursor back behind it.
    /// </summary>
    [Fact]
    public void AFullReadBelowTheCursorAlreadyAppliedIsIgnored()
    {
        var store = Store();
        Assert.True(store.Replace([Item(5, 12)], 12));
        store.Apply([Item(6, 15)]);
        Assert.Equal(15, store.Cursor);

        Assert.False(store.Replace([Item(5, 12)], 14));
        Assert.Equal([5L, 6], store.Items().Select(item => item.Id));
        Assert.Equal(15, store.Cursor);

        store.CaughtUp(store.Connection);
        store.Apply(Item(7, 18), SeqRoute.LiveFrame);
        Assert.False(store.Replace([Item(5, 12), Item(6, 15)], 17));
        Assert.Equal([5L, 6, 7], store.Items().Select(item => item.Id));
        Assert.Equal(18, store.Cursor);

        // Level with the cursor is not below it, and one above is simply newer.
        Assert.True(store.Replace([Item(5, 12), Item(6, 15), Item(7, 18)], 18));
        Assert.True(store.Replace([Item(6, 15), Item(7, 18)], 19));
        Assert.Equal([6L, 7], store.Items().Select(item => item.Id));
        Assert.Equal(19, store.Cursor);
    }

    /// <summary>
    /// A frame on a connection that has NOT caught up moves no cursor, so it cannot make a full
    /// read look stale: the read this pass is about to take must still be applied.
    /// </summary>
    [Fact]
    public void AFrameBeforeCatchUpDoesNotMakeTheFullReadStale()
    {
        var store = Store();
        store.Reconnected();
        var began = store.Connection;
        Assert.True(store.Apply(Item(9, 20), SeqRoute.LiveFrame));
        Assert.Equal(0, store.Cursor);

        // Taken before the frame's change committed: its mark is below the item held, which stays.
        Assert.True(store.Replace([Item(5, 12)], 15));

        Assert.Equal([5L, 9], store.Items().Select(item => item.Id));
        Assert.Equal(15, store.Cursor);
        Assert.True(store.CaughtUp(began));
    }

    /// <summary>
    /// A LIVE ITEM MISSING <c>added_by</c> OR <c>attachment</c> IS DROPPED, NOT DRAWN — from a
    /// frame, on a page and in a full read alike. The cursor still follows the page: the item was
    /// read, it is simply not something this client can draw, send or offer a removal for.
    /// </summary>
    [Fact]
    public void ALiveItemMissingWhoAddedItOrItsPictureIsDropped()
    {
        var store = Store();
        var picture = new AttachmentDto(75, "photo", "image/webp", 4096, 512, 512);
        var nobody = new PackItemDto(5, Attachment: picture, PackSeq: 12);
        var noPicture = new PackItemDto(6, 7, PackSeq: 13);

        Assert.False(store.Apply(nobody, SeqRoute.LiveFrame));
        Assert.False(store.Apply(noPicture, SeqRoute.LiveFrame));
        Assert.Equal(0, store.Apply([nobody, noPicture]));
        Assert.Equal(13, store.Cursor);
        Assert.True(store.Replace([nobody, noPicture, Item(7, 14)], 14));

        Assert.Equal(7, Assert.Single(store.Items()).Id);
        // As the wire writes them, too: the fields absent, not zero.
        var decoded = Wire.Decode<PackItemDto>(
            """{"id": 8, "attachment": {"id": 78, "kind": "photo", "mime": "image/png"}, "pack_seq": 15}""")!;
        Assert.False(store.Apply(decoded, SeqRoute.LiveFrame));
        Assert.Equal(7, Assert.Single(store.Items()).Id);
    }

    [Fact]
    public void AnUntouchedPackReadsAsNothingHeldAndNoCursor()
    {
        var store = Store();

        Assert.True(store.Replace([], 0));

        Assert.Empty(store.Items());
        Assert.Equal(0, store.Cursor);
    }

    // ---- the ceilings ---------------------------------------------------------

    /// <summary>
    /// The two ceilings are how the window knows the server has packs at all, so "the server said
    /// nothing" is written down as that rather than left as whatever it said last time.
    /// </summary>
    [Fact]
    public void TheCeilingsAreKeptAndTheirAbsenceIsKeptToo()
    {
        var store = Store();
        Assert.Null(store.Limits);

        store.SetLimits(new PackLimits(200, 524_288));
        Assert.Equal(new PackLimits(200, 524_288), store.Limits);
        // Across a relaunch, and offline: the answer is there before the first read of a launch.
        Assert.Equal(new PackLimits(200, 524_288), Store().Limits);

        store.SetLimits(null);
        Assert.Null(store.Limits);
    }

    // ---- recents ---------------------------------------------------------------

    [Fact]
    public void RecentsAreNewestFirstAndOnlyWhatThePackStillHolds()
    {
        var store = Store();
        var at = new DateTimeOffset(2026, 9, 13, 10, 0, 0, TimeSpan.Zero);
        store.Replace([Item(5, 12), Item(6, 13), Item(7, 14)], 14);

        store.Used(5, at);
        store.Used(7, at.AddMinutes(1));
        store.Used(5, at.AddMinutes(2));
        // An id the pack does not hold is not a sticker that can be sent.
        store.Used(99, at.AddMinutes(3));

        Assert.Equal([5L, 7], store.Recents());
        Assert.Equal([5L], store.Recents(limit: 1));

        // A removed sticker cannot be "recently used" either.
        store.Apply(Tombstone(5, 15));
        Assert.Equal([7L], store.Recents());
    }

    /// <summary>A sign-out wipes every table, and the pack — recents and ceilings included — goes with it.</summary>
    [Fact]
    public void WipingTheCacheTakesThePackWithIt()
    {
        var store = Store();
        store.Replace([Item(5, 12)], 12);
        store.Apply(Tombstone(5, 13));
        store.Replace([Item(6, 14)], 14);
        store.Used(6, DateTimeOffset.UnixEpoch);
        store.SetLimits(new PackLimits(200, 524_288));

        database.WipeAll();

        Assert.Empty(store.Items());
        Assert.Empty(store.Recents());
        Assert.Equal(0, store.Cursor);
        Assert.Null(store.Limits);
        // And the gone set with it: the next account's pack is another family's, with its own ids.
        Assert.True(store.Apply(Item(5, 12)));
    }
}
