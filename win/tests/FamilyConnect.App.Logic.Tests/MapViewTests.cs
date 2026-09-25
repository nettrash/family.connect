using FamilyConnect.App.Logic;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>A shared place's map picture: the tiles it needs, where they go, and the place at its middle.</summary>
public sealed class MapViewTests
{
    private const double Width = 240;
    private const double Height = 132;

    [Fact]
    public void TheEquatorAtGreenwichIsTheMiddleOfTheWorld()
    {
        var (x, y) = MapView.WorldPixel(0, 0, 16);
        var half = MapView.TileSize * 65536 / 2.0;
        Assert.Equal(half, x, 6);
        Assert.Equal(half, y, 6);
        Assert.Equal(0, MapView.WorldPixel(0, -180, 16).X, 6);
    }

    /// <summary>North is up the square and south is down it, the same distance for the same latitude.</summary>
    [Theory]
    [InlineData(51.5074)]
    [InlineData(-33.8688)]
    [InlineData(64.1466)]
    public void NorthAndSouthMirrorEachOther(double latitude)
    {
        var world = MapView.TileSize * 65536.0;
        var north = MapView.WorldPixel(Math.Abs(latitude), 10, 16).Y;
        var south = MapView.WorldPixel(-Math.Abs(latitude), 10, 16).Y;
        Assert.True(north < world / 2);
        Assert.Equal(world, north + south, 3);
    }

    /// <summary>A pole has no pixel in Web Mercator: it is drawn at the square's edge rather than as infinity.</summary>
    [Fact]
    public void APoleIsTheEdgeOfTheSquare()
    {
        Assert.Equal(MapView.WorldPixel(MapView.MaxLatitude, 0).Y, MapView.WorldPixel(90, 0).Y, 6);
        Assert.Equal(0, MapView.WorldPixel(90, 0).Y, 3);
        Assert.All(MapView.Tiles(89.9, 0, Width, Height), tile => Assert.InRange(tile.Y, 0, 65535));
    }

    [Theory]
    [InlineData(51.5074, -0.1278)]
    [InlineData(44.8125, 20.4612)]
    [InlineData(35.6762, 139.6503)]
    [InlineData(-22.9068, -43.1729)]
    [InlineData(0, 0)]
    public void TheTilesCoverThePictureAndNothingOutsideIt(double latitude, double longitude)
    {
        var tiles = MapView.Tiles(latitude, longitude, Width, Height);
        Assert.InRange(tiles.Count, 1, 4);
        Assert.True(tiles.Min(tile => tile.Left) <= 0);
        Assert.True(tiles.Min(tile => tile.Top) <= 0);
        Assert.True(tiles.Max(tile => tile.Left) + MapView.TileSize >= Width);
        Assert.True(tiles.Max(tile => tile.Top) + MapView.TileSize >= Height);
        Assert.All(tiles, tile =>
        {
            Assert.True(tile.Left < Width && tile.Left + MapView.TileSize > 0, "a tile drawn nowhere is a request for nothing");
            Assert.True(tile.Top < Height && tile.Top + MapView.TileSize > 0, "a tile drawn nowhere is a request for nothing");
        });
        Assert.Equal(tiles.Count, tiles.Select(tile => tile.Key).Distinct().Count());
    }

    /// <summary>The place is the picture's middle, which is where the view draws its dot.</summary>
    [Theory]
    [InlineData(51.5074, -0.1278)]
    [InlineData(-22.9068, -43.1729)]
    public void ThePlaceIsAtTheMiddleOfThePicture(double latitude, double longitude)
    {
        var (x, y) = MapView.WorldPixel(latitude, longitude);
        var tile = MapView.Tiles(latitude, longitude, Width, Height)
            .Single(tile => tile.Left <= Width / 2 && tile.Left + MapView.TileSize > Width / 2
                && tile.Top <= Height / 2 && tile.Top + MapView.TileSize > Height / 2);
        Assert.Equal(Width / 2, (x - (tile.X * MapView.TileSize)) + tile.Left, 6);
        Assert.Equal(Height / 2, (y - (tile.Y * MapView.TileSize)) + tile.Top, 6);
    }

    /// <summary>A picture across the date line takes its right-hand tile from the other side of the world.</summary>
    [Fact]
    public void AcrossTheDateLineTheTilesWrap()
    {
        var tiles = MapView.Tiles(0, 179.9999, Width, Height);
        Assert.Contains(tiles, tile => tile.X == 65535);
        Assert.Contains(tiles, tile => tile.X == 0);
        Assert.All(tiles, tile => Assert.InRange(tile.X, 0, 65535));
    }

    [Fact]
    public void ATileIsFetchedFromOpenStreetMapOverHttps()
    {
        var tile = new MapTile(16, 32744, 21792, 0, 0);
        Assert.Equal("https://tile.openstreetmap.org/16/32744/21792.png", MapView.TileUrl(tile).AbsoluteUri);
        Assert.Equal("16-32744-21792", tile.Key);
    }
}
