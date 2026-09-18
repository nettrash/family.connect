using System.Globalization;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>The full-size viewer: an album that pages and stops at its ends, and a photo's zoom from 1× to 6×.</summary>
public sealed class MediaAlbumTests
{
    private static readonly AttachmentDto[] Three =
    [
        new(1, "photo", Name: "beach.jpg"),
        new(2, "video"),
        new(3, "photo"),
    ];

    [Fact]
    public void AnAlbumIsAMessagesPhotosAndVideosInOrder()
    {
        AttachmentDto[] carried =
        [
            new(1, "photo"), new(2, "file", Name: "a.pdf"), new(3, "video"), new(4, "audio"), new(5, "location"), new(6, "photo"),
        ];
        Assert.Equal([1L, 3L, 6L], MediaAlbum.Of(carried).Select(attachment => attachment.Id));
        Assert.Equal("items", Assert.Throws<ArgumentException>(() => new MediaAlbum([], 0)).ParamName);
        Assert.Equal(2, new MediaAlbum(Three, 7).Index);
        Assert.Equal(0, new MediaAlbum(Three, -3).Index);
    }

    /// <summary>← and → page and stop at the ends; Esc closes; a key with nowhere to go does nothing.</summary>
    [Fact]
    public void PagingStopsAtTheEnds()
    {
        var album = new MediaAlbum(Three, 0);
        Assert.False(album.HasPrevious);
        Assert.Equal(ViewerKey.None, album.Key("Left"));
        Assert.False(album.Step(-1));
        Assert.Equal(ViewerKey.Next, album.Key("Right"));
        Assert.True(album.Step(1));
        Assert.True(album.IsVideo);
        Assert.True(album.Step(1));
        Assert.False(album.HasNext);
        Assert.Equal(ViewerKey.None, album.Key("Right"));
        Assert.False(album.Step(1));
        Assert.Equal(2, album.Index);
        Assert.Equal(ViewerKey.Previous, album.Key("Left"));
        Assert.Equal(ViewerKey.Close, album.Key("Escape"));
        Assert.Equal(ViewerKey.None, album.Key("Up"));
    }

    /// <summary>The web's own vectors: never smaller than the window, 1.25× a step, and no further than 6×.</summary>
    [Fact]
    public void ZoomGoesFromOneToSixAndNoFurther()
    {
        Assert.Equal(1.0, MediaAlbum.ZoomStep(1.0, -1.0));
        Assert.Equal(1.25, MediaAlbum.ZoomStep(1.0, 1.0), 9);
        Assert.Equal(MediaAlbum.MaxZoom, MediaAlbum.ZoomStep(5.5, 1.0));
        Assert.Equal(MediaAlbum.MaxZoom, MediaAlbum.ZoomStep(MediaAlbum.MaxZoom, 1.0));
        Assert.Equal(1.6, MediaAlbum.ZoomStep(2.0, -1.0), 9);
    }

    /// <summary>A double click toggles 1× and 2×; a pinch lands where it lands, within 1–6; a page turn starts at 1× again.</summary>
    [Fact]
    public void ZoomTogglesClampsAndResetsOnAPageTurn()
    {
        var album = new MediaAlbum(Three, 0);
        Assert.False(album.CanZoomOut);
        album.ToggleZoom();
        Assert.Equal(2, album.Zoom);
        album.ToggleZoom();
        Assert.Equal(1, album.Zoom);
        album.ToggleZoom();
        album.ZoomIn();
        Assert.Equal(2.5, album.Zoom, 9);
        album.ToggleZoom();
        Assert.Equal(1, album.Zoom);
        album.ZoomedTo(9);
        Assert.Equal(MediaAlbum.MaxZoom, album.Zoom);
        Assert.False(album.CanZoomIn);
        album.ZoomedTo(0.4);
        Assert.Equal(1, album.Zoom);
        album.ZoomOut();
        Assert.Equal(1, album.Zoom);

        album.ZoomedTo(3);
        album.Step(1);
        Assert.Equal(1, album.Zoom);
        // A video has no zoom to offer — not even one a stray pinch left behind.
        Assert.False(album.CanZoomIn);
        Assert.False(album.CanZoomOut);
        album.ZoomedTo(3);
        Assert.False(album.CanZoomIn);
        Assert.False(album.CanZoomOut);
    }

    [Fact]
    public void TheWordsOverThePicture()
    {
        var album = new MediaAlbum(Three, 0);
        Assert.Equal("beach.jpg", album.Title(EnglishCatalog.Instance));
        Assert.Equal("1 of 3", album.Position(EnglishCatalog.Instance));
        album.Step(1);
        Assert.Equal("2 of 3", album.Position(EnglishCatalog.Instance));
        Assert.Equal(AttachmentText.DisplayName("video", null, EnglishCatalog.Instance), album.Title(EnglishCatalog.Instance));
        Assert.Null(new MediaAlbum([new(9, "photo")], 0).Position(EnglishCatalog.Instance));

        var zoomed = new MediaAlbum(Three, 0);
        Assert.Equal("100%", zoomed.ZoomText(CultureInfo.InvariantCulture));
        zoomed.ZoomIn();
        Assert.Equal("125%", zoomed.ZoomText(CultureInfo.InvariantCulture));
    }
}
