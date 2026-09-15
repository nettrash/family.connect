using System.Globalization;

namespace FamilyConnect.App.Logic;

/// <summary>One map tile of the picture: which tile, and where its top-left corner falls inside the picture.</summary>
public readonly record struct MapTile(int Zoom, int X, int Y, double Left, double Top)
{
    /// <summary>The tile's name on disk and in memory. Invariant: a key must read the same on every machine.</summary>
    public string Key => string.Create(CultureInfo.InvariantCulture, $"{Zoom}-{X}-{Y}");
}

/// <summary>
/// The picture of a shared place, as map tiles: which ones, and where each goes (the Apple apps' 140-point MapKit map in the
/// bubble, drawn on Windows from OpenStreetMap's tiles).
/// </summary>
/// <remarks>
/// <para>
/// <b>WEB MERCATOR, AS EVERY TILE SERVER DRAWS IT.</b> The world is one square of <c>256 × 2^zoom</c> pixels; a place is a
/// pixel in it, and the picture is a window onto that square centred on the place — so the place is always at the middle,
/// which is where the view draws its dot.
/// </para>
/// <para>
/// <b>ZOOM 16</b> is about 600 m across 240 pixels at the equator and less towards the poles: close enough to recognise the
/// street, wide enough to place it in a neighbourhood — the Mac's 0.005° span, in tiles.
/// </para>
/// </remarks>
public static class MapView
{
    public const int TileSize = 256;

    public const int Zoom = 16;

    /// <summary>Where Web Mercator stops: the square's own edge, beyond which a latitude has no pixel.</summary>
    public const double MaxLatitude = 85.0511287798066;

    /// <summary>The place as a pixel of the whole world's square at <paramref name="zoom"/>.</summary>
    public static (double X, double Y) WorldPixel(double latitude, double longitude, int zoom = Zoom)
    {
        var scale = TileSize * (double)(1L << zoom);
        var radians = Math.Clamp(latitude, -MaxLatitude, MaxLatitude) * Math.PI / 180;
        var x = (longitude + 180) / 360 * scale;
        var y = (1 - (Math.Log(Math.Tan(radians) + (1 / Math.Cos(radians))) / Math.PI)) / 2 * scale;
        return (x, y);
    }

    /// <summary>
    /// Every tile a <paramref name="width"/> × <paramref name="height"/> picture centred on the place needs, each with its
    /// offset inside the picture. A tile across the date line is the one on the other side of the world; a row above the
    /// top of the world or below its bottom is sky nobody draws.
    /// </summary>
    public static IReadOnlyList<MapTile> Tiles(double latitude, double longitude, double width, double height, int zoom = Zoom)
    {
        var (centreX, centreY) = WorldPixel(latitude, longitude, zoom);
        var left = centreX - (width / 2);
        var top = centreY - (height / 2);
        var count = 1 << zoom;
        var tiles = new List<MapTile>();
        for (var row = (int)Math.Floor(top / TileSize); row * (double)TileSize < top + height; row++)
        {
            if (row < 0 || row >= count)
            {
                continue;
            }
            for (var column = (int)Math.Floor(left / TileSize); column * (double)TileSize < left + width; column++)
            {
                var wrapped = ((column % count) + count) % count;
                tiles.Add(new MapTile(zoom, wrapped, row, (column * (double)TileSize) - left, (row * (double)TileSize) - top));
            }
        }
        return tiles;
    }

    /// <summary>Where a tile is fetched from: OpenStreetMap's own tile server, over https.</summary>
    public static Uri TileUrl(MapTile tile) =>
        new(string.Create(CultureInfo.InvariantCulture, $"https://tile.openstreetmap.org/{tile.Zoom}/{tile.X}/{tile.Y}.png"));
}
