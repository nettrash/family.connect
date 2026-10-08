using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// How a chat sticker is drawn (docs/protocol.md, "How it is drawn"): one fixed box, fitted whole,
/// larger than an emoji and smaller than a photograph — and the arithmetic of an animated one.
/// </summary>
public class StickerLookTests
{
    /// <summary>
    /// NEVER AT THE PICTURE'S OWN PIXEL SIZE: a 96-pixel sticker would be a speck and a 2000-pixel
    /// one a poster. Both fill the same box.
    /// </summary>
    [Theory]
    [InlineData(512, 512, 160, 160)]
    [InlineData(96, 96, 160, 160)]
    [InlineData(2000, 2000, 160, 160)]
    [InlineData(512, 256, 160, 80)]
    [InlineData(256, 512, 80, 160)]
    [InlineData(300, 100, 160, 53)]
    public void EveryStickerIsFittedWholeIntoTheSameBox(int width, int height, double drawnWidth, double drawnHeight)
    {
        Assert.Equal((drawnWidth, drawnHeight), StickerLook.Fit(width, height));
    }

    /// <summary>The shape comes from metadata, and a sticker whose uploader could not say gets the whole box.</summary>
    [Fact]
    public void AStickerOfUnknownShapeGetsTheWholeBox()
    {
        Assert.Equal((160, 160), StickerLook.Fit(null, null));
        Assert.Equal((160, 160), StickerLook.Fit(512, null));
        Assert.Equal((160, 160), StickerLook.Fit(0, 512));
        Assert.Equal((72, 72), StickerLook.Fit(null, null, StickerLook.PanelCell));
        // Never thinner than a pixel.
        Assert.Equal((160, 1), StickerLook.Fit(4000, 3));
    }

    /// <summary>The box is 160 effective pixels — the same number on every client, in its own unit.</summary>
    [Fact]
    public void TheBoxIsOneHundredAndSixty()
    {
        Assert.Equal(160, StickerLook.Box);
        Assert.Equal((160, 160), StickerLook.Fit(512, 512));
        Assert.Equal((160, 160), StickerLook.Fit(null, null));
    }

    /// <summary>Larger than an emoji and smaller than a photograph, on this client's own scales.</summary>
    [Fact]
    public void TheBoxSitsBetweenAnEmojiAndAPhoto()
    {
        var largestEmoji = Emoji.DisplayFontSizeForBody("🎉", 14) ?? 0;

        Assert.True(largestEmoji > 0);
        Assert.True(StickerLook.Box > largestEmoji);
        Assert.True(StickerLook.Box < MediaText.TileMax);
    }

    // ---- animation -----------------------------------------------------------------------------

    [Fact]
    public void OnlyAPictureWithFramesThatFitTheBudgetIsAnimated()
    {
        Assert.True(StickerAnimation.Worth(30, 240, 240));
        // One frame is a still picture.
        Assert.False(StickerAnimation.Worth(1, 240, 240));
        Assert.False(StickerAnimation.Worth(0, 240, 240));
        // Too many frames, or too many pixels of them: drawn as frame zero, which is a correct sticker.
        Assert.False(StickerAnimation.Worth(StickerAnimation.MaxFrames + 1, 16, 16));
        Assert.False(StickerAnimation.Worth(200, 512, 512));
        Assert.False(StickerAnimation.Worth(30, 0, 240));
    }

    [Theory]
    [InlineData(512, 512, 160, 1.0, 160, 160)]
    [InlineData(512, 512, 160, 1.5, 240, 240)]
    [InlineData(512, 256, 160, 2.0, 320, 160)]
    [InlineData(96, 96, 160, 2.0, 96, 96)]      // never decoded larger than it is
    [InlineData(512, 512, 160, 0.0, 160, 160)]  // a scale nobody could read is taken as 1
    public void FramesAreDecodedAtTheSizeTheyAreDrawnAndNoLarger(
        int width, int height, double box, double scale, int decodedWidth, int decodedHeight)
    {
        Assert.Equal((decodedWidth, decodedHeight), StickerAnimation.DecodeSize(width, height, box, scale));
    }

    /// <summary>
    /// A DURATION OF ZERO IS NOT ZERO: files say 0 or 10 ms for "as fast as you can", and every
    /// browser draws those at 100 ms rather than spinning a core.
    /// </summary>
    [Fact]
    public void ATooShortFrameStandsForATenthOfASecond()
    {
        Assert.Equal([40, 100, 100, 11, 1000], StickerAnimation.Clock([40, 0, 10, 11, 1000], 5));
    }

    /// <summary>A file whose frame count disagrees with the decoder's cannot be lined up, so every frame gets the floor.</summary>
    [Fact]
    public void DurationsThatDoNotMatchTheFramesAreNotTrusted()
    {
        Assert.Equal([100, 100, 100], StickerAnimation.Clock([40, 40], 3));
        Assert.Equal([100, 100], StickerAnimation.Clock([], 2));
        Assert.Empty(StickerAnimation.Clock([40], 0));
    }

    [Fact]
    public void TheFrameDueFollowsTheClockAndLoopsForEver()
    {
        int[] clock = [40, 60, 100];

        Assert.Equal(0, StickerAnimation.FrameAt(clock, 0));
        Assert.Equal(0, StickerAnimation.FrameAt(clock, 39));
        Assert.Equal(1, StickerAnimation.FrameAt(clock, 40));
        Assert.Equal(1, StickerAnimation.FrameAt(clock, 99));
        Assert.Equal(2, StickerAnimation.FrameAt(clock, 100));
        Assert.Equal(2, StickerAnimation.FrameAt(clock, 199));
        // A sticker is not a film with an end.
        Assert.Equal(0, StickerAnimation.FrameAt(clock, 200));
        Assert.Equal(1, StickerAnimation.FrameAt(clock, 200 * 1000 + 45));
        Assert.Equal(0, StickerAnimation.FrameAt(clock, -5));
        Assert.Equal(0, StickerAnimation.FrameAt([], 500));
        // A clock somebody built with a zero in it still ends.
        Assert.Equal(1, StickerAnimation.FrameAt([0, 0], 1));
    }

    // ---- the shelf: what a view keeps decoded, and what that may cost ------------------------------
    //
    // No window in any of this: a "picture" here is a name, and whether it is on screen is a set.

    private sealed class Wall
    {
        public HashSet<string> OnScreen { get; } = [];

        public List<string> MadeStill { get; } = [];

        public List<string> LetGo { get; } = [];

        public StickerShelf<string> Shelf(long budget, int most = 64) =>
            new(budget, most, OnScreen.Contains, MadeStill.Add, LetGo.Add);
    }

    /// <summary>
    /// THE BUDGET IS FOR THE SHELF, NOT PER STICKER. Forty animated stickers of a legal size each
    /// are gigabytes if every one is kept; what is kept never passes the budget, however many go by.
    /// </summary>
    [Fact]
    public void TheShelfNeverHoldsMoreThanItsBudgetHoweverManyStickersGoBy()
    {
        var wall = new Wall();
        var shelf = wall.Shelf(StickerAnimation.MaxHeldBytes);

        for (var id = 1; id <= 40; id++)
        {
            shelf.Put(id, $"s{id}", StickerAnimation.MaxDecodedBytes / 2);
            Assert.True(shelf.Held <= StickerAnimation.MaxHeldBytes);
        }

        // Four halves fit; the thirty-six drawn longest ago were let go — disposed, not left to a finalizer.
        Assert.Equal(4, shelf.Count);
        Assert.Equal(Enumerable.Range(1, 36).Select(id => $"s{id}"), wall.LetGo);
        Assert.Empty(wall.MadeStill);
        Assert.True(shelf.TryGet(40, out var newest));
        Assert.Equal("s40", newest);
        Assert.False(shelf.TryGet(1, out _));
    }

    /// <summary>The budget is two of the largest sticker allowed, so the newest always fits.</summary>
    [Fact]
    public void TheBudgetHasRoomForTheLargestStickerWhateverElseIsKept()
    {
        Assert.True(StickerAnimation.MaxHeldBytes >= 2 * StickerAnimation.MaxDecodedBytes);
        Assert.Equal(StickerAnimation.MaxDecodedBytes, StickerAnimation.DecodedBytes(48, 512, 512));
        Assert.Equal(0, StickerAnimation.DecodedBytes(-1, 512, 512));
    }

    /// <summary>
    /// A sticker ON SCREEN cannot be let go — an image is drawing its frames — so it is made a
    /// STILL instead, oldest first, and stays on the shelf as one: a rebuild finds it there and
    /// does not decode it again.
    /// </summary>
    [Fact]
    public void AStickerOnScreenIsMadeStillRatherThanLetGo()
    {
        var wall = new Wall();
        var shelf = wall.Shelf(budget: 100);
        shelf.Put(1, "old", 60);
        shelf.Put(2, "middle", 30);
        wall.OnScreen.UnionWith(["old", "middle"]);

        shelf.Put(3, "new", 60);

        // The oldest gave its frames back and that was enough: the middle one keeps moving.
        Assert.Equal(["old"], wall.MadeStill);
        Assert.Empty(wall.LetGo);
        Assert.Equal(90, shelf.Held);
        Assert.True(shelf.TryGet(1, out var kept));
        Assert.Equal("old", kept);

        // Made still once, it has nothing more to give: the next squeeze passes it by.
        wall.OnScreen.Add("new");
        shelf.Put(4, "newer", 60);
        Assert.Equal(["old", "middle", "new"], wall.MadeStill);
        Assert.Equal(60, shelf.Held);
        Assert.Equal(4, shelf.Count);
    }

    /// <summary>What nobody is looking at goes before anything on screen stops moving — in the order it was last drawn.</summary>
    [Fact]
    public void WhatIsOffScreenIsLetGoInTheOrderItWasLastDrawn()
    {
        var wall = new Wall();
        var shelf = wall.Shelf(budget: 100);
        shelf.Put(1, "a", 40);
        shelf.Put(2, "b", 40);
        // Drawn again: a rebuild asked for it.
        Assert.True(shelf.TryGet(1, out _));

        shelf.Put(3, "c", 40);

        Assert.Equal(["b"], wall.LetGo);
        Assert.True(shelf.TryGet(1, out _));
        Assert.False(shelf.TryGet(2, out _));
    }

    /// <summary>
    /// A still picture costs nothing that can be given back, so bytes never push it out; the COUNT
    /// does, and only for what no image is showing.
    /// </summary>
    [Fact]
    public void StillPicturesLeaveByTheCountAlone()
    {
        var wall = new Wall();
        var shelf = wall.Shelf(budget: 100, most: 3);
        shelf.Put(1, "still-1", 0);
        shelf.Put(2, "still-2", 0);
        shelf.Put(3, "moving", 90);
        wall.OnScreen.Add("moving");

        // Over the bytes: the stills are passed by, and the moving one on screen is what gives.
        shelf.Put(4, "moving-2", 90);
        Assert.Equal(["moving"], wall.MadeStill);
        // …and over the count: the oldest still that nothing shows is let go.
        Assert.Equal(["still-1"], wall.LetGo);
        Assert.Equal(3, shelf.Count);
        Assert.Equal(90, shelf.Held);
    }

    /// <summary>
    /// "NOTHING HERE DECODES IT" IS AN ANSWER, AND IT IS KEPT: on a machine without the WebP codec a
    /// chat of twenty stickers would otherwise read twenty files and fail twenty decodes on every
    /// message that arrives.
    /// </summary>
    [Fact]
    public void AStickerNothingHereDecodesIsRememberedAsThat()
    {
        var wall = new Wall();
        var shelf = wall.Shelf(budget: 100);

        Assert.False(shelf.TryGet(7, out _));
        shelf.Put(7, null, 50);

        Assert.True(shelf.TryGet(7, out var picture));
        Assert.Null(picture);
        // An answer holds nothing.
        Assert.Equal(0, shelf.Held);
        shelf.Clear();
        Assert.Empty(wall.LetGo);
        Assert.False(shelf.TryGet(7, out _));
    }

    /// <summary>
    /// THE CLOCK IS THE STICKER'S, NOT THE IMAGE'S. The conversation is rebuilt on every change, and
    /// each rebuild makes new images: they join the animation where it was, rather than every
    /// sticker on screen jumping back to frame zero whenever anybody sends or reacts.
    /// </summary>
    [Fact]
    public void ARebuiltImageJoinsTheAnimationWhereItWas()
    {
        var shelf = new Wall().Shelf(budget: 100);
        int[] clock = [100, 100, 100];
        shelf.Put(5, "waving", 10);

        var first = shelf.Started(5, now: 1_000);
        var rebuilt = shelf.Started(5, now: 1_150);

        Assert.Equal(1_000, first);
        Assert.Equal(1_000, rebuilt);
        Assert.Equal(1, StickerAnimation.FrameAt(clock, 1_150 - rebuilt));
        // Decoded again (the screen's scale changed, say), it is still the same sticker.
        shelf.Put(5, "waving, sharper", 10);
        Assert.Equal(1_000, shelf.Started(5, now: 1_400));
        // One the shelf does not hold — the viewer's — starts when it is shown.
        Assert.Equal(2_000, shelf.Started(99, now: 2_000));
    }

    /// <summary>The view is going: every picture is let go, on screen or not, and nothing is held.</summary>
    [Fact]
    public void ClearingTheShelfLetsEverythingGo()
    {
        var wall = new Wall();
        var shelf = wall.Shelf(budget: 100);
        shelf.Put(1, "a", 40);
        shelf.Put(2, "b", 40);
        wall.OnScreen.Add("a");

        shelf.Clear();

        Assert.Equal(["a", "b"], wall.LetGo.Order());
        Assert.Equal(0, shelf.Held);
        Assert.Equal(0, shelf.Count);
    }

    // ---- what a sticker is, everywhere else it is drawn -------------------------------------------

    private static MessageDto Message(string body, params AttachmentDto[] media) =>
        new(1, 42, 9, null, body, "2026-09-13T10:00:00Z", Attachments: media);

    private static readonly AttachmentDto Sent = new(90, "photo", "image/webp", 4096, 512, 512, Sticker: true);

    /// <summary>On a chat-list row it is the word, where a photo's row says so of a photo.</summary>
    [Fact]
    public void AChatListRowSaysStickerWhereAPhotosSaysPhoto()
    {
        using var cache = Database.OpenInMemory();
        var list = new ChatListModel(new ChatStore(cache), () => 7);

        Assert.Equal("Sticker", list.Preview(Message(string.Empty, Sent), hidden: false));
        // OLD MESSAGES: the same attachment without the flag is the photo it always was.
        Assert.Equal("Photo", list.Preview(Message(string.Empty, Sent with { Sticker = false }), hidden: false));
        // A blocked member's sticker is the hidden row, like anything else they sent.
        Assert.Equal("Hidden — blocked member", list.Preview(Message(string.Empty, Sent), hidden: true));
    }

    /// <summary>
    /// A STICKER IS ALWAYS ITS ORIGINAL, whatever <c>has_preview</c> says — a preview is a JPEG, and
    /// the flag can be inherited through dedup from somebody who once sent the same PNG as a photo.
    /// </summary>
    [Fact]
    public void AStickerIsDrawnFromItsOriginalEvenWhereItSaysItHasAPreview()
    {
        Assert.Equal(AttachmentFiles.TileSource.Original, AttachmentFiles.SourceFor(Sent with { HasPreview = true }));
        Assert.Equal(AttachmentFiles.TileSource.Original, AttachmentFiles.SourceFor(Sent));
        // And a photo is still a photo.
        Assert.Equal(
            AttachmentFiles.TileSource.Preview,
            AttachmentFiles.SourceFor(Sent with { Sticker = false, HasPreview = true }));
    }

    [Fact]
    public void ASavedStickerKeepsItsOwnType()
    {
        Assert.Equal("photo-90.webp", AttachmentFiles.FileName(Sent));
        Assert.Equal("photo-90.png", AttachmentFiles.FileName(Sent with { Mime = "image/png" }));
    }

    [Fact]
    public void ShownLargerItIsCalledASticker()
    {
        Assert.Equal("Sticker", new MediaAlbum([Sent], 0).Title(EnglishCatalog.Instance));
        Assert.Equal("Photo", new MediaAlbum([Sent with { Sticker = false }], 0).Title(EnglishCatalog.Instance));
    }

    /// <summary>
    /// A sticker has no words, so there is nothing to edit — and it is a message like any other for
    /// the rest: it may be reported, and its sender blocked.
    /// </summary>
    [Fact]
    public void AStickerBehavesLikeAnyMessage()
    {
        var mine = new Bubble(Message(string.Empty, Sent) with { SenderId = 7 }, Hidden: false, Revealed: false, Mine: true);
        var theirs = Message(string.Empty, Sent);

        Assert.False(ConversationModel.MayEdit(mine));
        // "Edit" IS NEVER OFFERED ON A STICKER, and that is its own rule — not a consequence of the empty body. A
        // sticker that somehow carried words would be refused by the server's edit just the same.
        var worded = mine with { Message = mine.Message with { Body = "look" } };
        Assert.NotNull(worded.Message.StickerPicture);
        Assert.False(ConversationModel.MayEdit(worded));
        // The same words on a PHOTO — the flag absent — are the author's to edit, WebP or not.
        var photo = worded with { Message = worded.Message with { Attachments = [Sent with { Sticker = false }] } };
        Assert.Null(photo.Message.StickerPicture);
        Assert.True(ConversationModel.MayEdit(photo));
        Assert.True(BubbleRules.MayReport(theirs, me: 7, assistantChat: false, assistantUserId: null));
        Assert.True(BubbleRules.IsOtherMember(theirs, me: 7, assistantChat: false, assistantUserId: null));
        // Never mistaken for an assistant answer that has not been written yet.
        Assert.False(BubbleRules.Awaited(theirs, string.Empty, me: 7, assistantChat: true, assistantUserId: 9));
    }
}
