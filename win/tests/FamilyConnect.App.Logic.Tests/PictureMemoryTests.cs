using FamilyConnect.App.Logic;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>How big a tile's photo is decoded, and how many decoded pictures a view keeps.</summary>
public sealed class PictureMemoryTests
{
    /// <summary>
    /// A camera's photo is decoded so that BOTH sides come out at least the largest tile box, whichever side the decoder
    /// calls its width — so an EXIF turn the attachment's numbers leave out can never make a tile blurrier than before.
    /// </summary>
    [Fact]
    public void ABigPhotoIsDecodedNearTheTileWhicheverWayUpItIs()
    {
        var width = PictureDecode.TileWidth(4000, 3000, rasterScale: 1.5);
        Assert.Equal(427, width);
        // Asked as the width: 427 × 320 one way up, and 427 × 569 the other — at least 320 either way.
        foreach (var (pictureWidth, pictureHeight) in new[] { (4000, 3000), (3000, 4000) })
        {
            var decodedHeight = width!.Value * pictureHeight / (double)pictureWidth;
            Assert.True(width >= PictureDecode.TileBox);
            Assert.True(decodedHeight >= PictureDecode.TileBox - 1);
        }
        Assert.Equal(PictureDecode.TileWidth(3000, 4000, 1.5), width);
    }

    /// <summary>A picture already about the tile's size, or one whose size nobody said, is decoded whole, as it always was.</summary>
    [Fact]
    public void ASmallOrUnknownPhotoIsDecodedWhole()
    {
        Assert.Null(PictureDecode.TileWidth(400, 300, 1));
        // A 2:1 picture is asked for at 640 effective pixels wide, 1280 device pixels at 200 %: more than its 600-pixel short
        // side, so there is nothing to save.
        Assert.Null(PictureDecode.TileWidth(1200, 600, 2));
        Assert.NotNull(PictureDecode.TileWidth(2400, 1200, 1));
        Assert.Null(PictureDecode.TileWidth(null, 3000, 1));
        Assert.Null(PictureDecode.TileWidth(4000, 0, 1));
        // A screen that reports no scale is taken at 100 %.
        Assert.Equal(PictureDecode.TileWidth(4000, 3000, 1), PictureDecode.TileWidth(4000, 3000, 0));
    }

    /// <summary>A panorama keeps its shape: the long side is asked for in proportion, so the short side still fills the box.</summary>
    [Fact]
    public void APanoramaIsDecodedWideEnoughToFillTheBox()
    {
        var width = PictureDecode.TileWidth(12000, 2000, 1);
        Assert.Equal(1920, width);
        Assert.True(width * 2000 / 12000.0 >= PictureDecode.TileBox);
    }

    [Fact]
    public void ThePicturesKeptAreTheMostRecentlyDrawn()
    {
        var kept = new RecentPictures<string>(2);
        kept["a"] = "A";
        kept["b"] = "B";
        Assert.True(kept.TryGetValue("a", out var a));
        Assert.Equal("A", a);
        kept["c"] = "C";

        // "b" was drawn longest ago — "a" was just drawn again — so "b" made the room.
        Assert.False(kept.ContainsKey("b"));
        Assert.True(kept.ContainsKey("a"));
        Assert.True(kept.ContainsKey("c"));
        Assert.Equal(2, kept.Count);
        Assert.False(kept.TryGetValue("b", out _));
    }

    /// <summary>Asking whether a picture is kept is not drawing it: it does not save it from being the next to go.</summary>
    [Fact]
    public void AskingIsNotDrawing()
    {
        var kept = new RecentPictures<string>(2);
        kept["a"] = "A";
        kept["b"] = "B";
        Assert.True(kept.ContainsKey("a"));
        kept["c"] = "C";
        Assert.False(kept.ContainsKey("a"));
    }

    [Fact]
    public void APictureKeptAgainReplacesTheOldOneAndClearForgetsAll()
    {
        var kept = new RecentPictures<string>(3);
        kept["a"] = "old";
        kept["a"] = "new";
        Assert.Equal(1, kept.Count);
        Assert.True(kept.TryGetValue("a", out var a));
        Assert.Equal("new", a);
        kept.Clear();
        Assert.Equal(0, kept.Count);
        Assert.False(kept.ContainsKey("a"));
    }
}
