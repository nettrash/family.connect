//! The picture of a shared place, as map tiles: which ones, and where each goes (`MapView.cs` —
//! the Apple apps' 140-point MapKit map in the bubble, drawn on Linux, as on Windows, from
//! OpenStreetMap's tiles).
//!
//! WEB MERCATOR, AS EVERY TILE SERVER DRAWS IT. The world is one square of `256 × 2^zoom` pixels; a
//! place is a pixel in it, and the picture is a window onto that square centred on the place — so
//! the place is always at the middle, which is where the view draws its dot.
//!
//! ZOOM 16 is about 600 m across 240 pixels at the equator and less towards the poles: close
//! enough to recognise the street, wide enough to place it in a neighbourhood — the Mac's 0.005°
//! span, in tiles.

pub const TILE_SIZE: i32 = 256;

pub const ZOOM: i32 = 16;

/// Where Web Mercator stops: the square's own edge, beyond which a latitude has no pixel.
pub const MAX_LATITUDE: f64 = 85.0511287798066;

/// One map tile of the picture: which tile, and where its top-left corner falls inside the
/// picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapTile {
    pub zoom: i32,
    pub x: i32,
    pub y: i32,
    pub left: f64,
    pub top: f64,
}

impl MapTile {
    /// The tile's name on disk and in memory: the same on every machine.
    pub fn key(&self) -> String {
        format!("{}-{}-{}", self.zoom, self.x, self.y)
    }
}

/// The place as a pixel of the whole world's square at `zoom` (`ZOOM` in the C#'s default).
pub fn world_pixel(latitude: f64, longitude: f64, zoom: i32) -> (f64, f64) {
    let scale = f64::from(TILE_SIZE) * (1i64 << zoom) as f64;
    let radians = latitude.clamp(-MAX_LATITUDE, MAX_LATITUDE) * std::f64::consts::PI / 180.0;
    let x = (longitude + 180.0) / 360.0 * scale;
    let y =
        (1.0 - ((radians.tan() + (1.0 / radians.cos())).ln() / std::f64::consts::PI)) / 2.0 * scale;
    (x, y)
}

/// Every tile a `width` × `height` picture centred on the place needs, each with its offset
/// inside the picture. A tile across the date line is the one on the other side of the world; a
/// row above the top of the world or below its bottom is sky nobody draws.
pub fn tiles(latitude: f64, longitude: f64, width: f64, height: f64, zoom: i32) -> Vec<MapTile> {
    let (centre_x, centre_y) = world_pixel(latitude, longitude, zoom);
    let left = centre_x - (width / 2.0);
    let top = centre_y - (height / 2.0);
    let count = 1i32 << zoom;
    let size = f64::from(TILE_SIZE);
    let mut tiles = Vec::new();
    let mut row = (top / size).floor() as i32;
    while f64::from(row) * size < top + height {
        if (0..count).contains(&row) {
            let mut column = (left / size).floor() as i32;
            while f64::from(column) * size < left + width {
                let wrapped = column.rem_euclid(count);
                tiles.push(MapTile {
                    zoom,
                    x: wrapped,
                    y: row,
                    left: (f64::from(column) * size) - left,
                    top: (f64::from(row) * size) - top,
                });
                column += 1;
            }
        }
        row += 1;
    }
    tiles
}

/// Where a tile is fetched from: OpenStreetMap's own tile server, over https.
pub fn tile_url(tile: &MapTile) -> String {
    format!(
        "https://tile.openstreetmap.org/{}/{}/{}.png",
        tile.zoom, tile.x, tile.y
    )
}
