//! `MapViewTests`: a shared place's map picture — the tiles it needs, where they go, and the place
//! at its middle.

use std::collections::HashSet;

use fc_logic::map_view::{MAX_LATITUDE, MapTile, TILE_SIZE, ZOOM, tile_url, tiles, world_pixel};

const WIDTH: f64 = 240.0;
const HEIGHT: f64 = 132.0;

/// xUnit's `Assert.Equal(expected, actual, precision)`: equal once both are rounded to that many
/// decimal places (.NET's `Math.Round`, to even).
fn assert_close(expected: f64, actual: f64, digits: i32) {
    let scale = 10f64.powi(digits);
    let round = |value: f64| (value * scale).round_ties_even() / scale;
    assert_eq!(
        round(expected),
        round(actual),
        "{expected} vs {actual} to {digits} places"
    );
}

fn size() -> f64 {
    f64::from(TILE_SIZE)
}

#[test]
fn the_equator_at_greenwich_is_the_middle_of_the_world() {
    let (x, y) = world_pixel(0.0, 0.0, 16);
    let half = size() * 65536.0 / 2.0;
    assert_close(half, x, 6);
    assert_close(half, y, 6);
    assert_close(0.0, world_pixel(0.0, -180.0, 16).0, 6);
}

/// North is up the square and south is down it, the same distance for the same latitude.
#[test]
fn north_and_south_mirror_each_other() {
    for latitude in [51.5074, -33.8688, 64.1466] {
        let world = size() * 65536.0;
        let north = world_pixel(f64::abs(latitude), 10.0, 16).1;
        let south = world_pixel(-f64::abs(latitude), 10.0, 16).1;
        assert!(north < world / 2.0);
        assert_close(world, north + south, 3);
    }
}

/// A pole has no pixel in Web Mercator: it is drawn at the square's edge rather than as infinity.
#[test]
fn a_pole_is_the_edge_of_the_square() {
    assert_close(
        world_pixel(MAX_LATITUDE, 0.0, ZOOM).1,
        world_pixel(90.0, 0.0, ZOOM).1,
        6,
    );
    assert_close(0.0, world_pixel(90.0, 0.0, ZOOM).1, 3);
    for tile in tiles(89.9, 0.0, WIDTH, HEIGHT, ZOOM) {
        assert!((0..=65535).contains(&tile.y));
    }
}

#[test]
fn the_tiles_cover_the_picture_and_nothing_outside_it() {
    for (latitude, longitude) in [
        (51.5074, -0.1278),
        (44.8125, 20.4612),
        (35.6762, 139.6503),
        (-22.9068, -43.1729),
        (0.0, 0.0),
    ] {
        let tiles = tiles(latitude, longitude, WIDTH, HEIGHT, ZOOM);
        assert!((1..=4).contains(&tiles.len()));
        let min = |pick: fn(&MapTile) -> f64| tiles.iter().map(pick).fold(f64::INFINITY, f64::min);
        let max =
            |pick: fn(&MapTile) -> f64| tiles.iter().map(pick).fold(f64::NEG_INFINITY, f64::max);
        assert!(min(|tile| tile.left) <= 0.0);
        assert!(min(|tile| tile.top) <= 0.0);
        assert!(max(|tile| tile.left) + size() >= WIDTH);
        assert!(max(|tile| tile.top) + size() >= HEIGHT);
        for tile in &tiles {
            assert!(
                tile.left < WIDTH && tile.left + size() > 0.0,
                "a tile drawn nowhere is a request for nothing"
            );
            assert!(
                tile.top < HEIGHT && tile.top + size() > 0.0,
                "a tile drawn nowhere is a request for nothing"
            );
        }
        let keys: HashSet<String> = tiles.iter().map(MapTile::key).collect();
        assert_eq!(keys.len(), tiles.len());
    }
}

/// The place is the picture's middle, which is where the view draws its dot.
#[test]
fn the_place_is_at_the_middle_of_the_picture() {
    for (latitude, longitude) in [(51.5074, -0.1278), (-22.9068, -43.1729)] {
        let (x, y) = world_pixel(latitude, longitude, ZOOM);
        let middle: Vec<MapTile> = tiles(latitude, longitude, WIDTH, HEIGHT, ZOOM)
            .into_iter()
            .filter(|tile| {
                tile.left <= WIDTH / 2.0
                    && tile.left + size() > WIDTH / 2.0
                    && tile.top <= HEIGHT / 2.0
                    && tile.top + size() > HEIGHT / 2.0
            })
            .collect();
        assert_eq!(middle.len(), 1, "Single");
        let tile = middle[0];
        assert_close(WIDTH / 2.0, (x - f64::from(tile.x) * size()) + tile.left, 6);
        assert_close(HEIGHT / 2.0, (y - f64::from(tile.y) * size()) + tile.top, 6);
    }
}

/// A picture across the date line takes its right-hand tile from the other side of the world.
#[test]
fn across_the_date_line_the_tiles_wrap() {
    let tiles = tiles(0.0, 179.9999, WIDTH, HEIGHT, ZOOM);
    assert!(tiles.iter().any(|tile| tile.x == 65535));
    assert!(tiles.iter().any(|tile| tile.x == 0));
    assert!(tiles.iter().all(|tile| (0..=65535).contains(&tile.x)));
}

#[test]
fn a_tile_is_fetched_from_open_street_map_over_https() {
    let tile = MapTile {
        zoom: 16,
        x: 32744,
        y: 21792,
        left: 0.0,
        top: 0.0,
    };
    assert_eq!(
        tile_url(&tile),
        "https://tile.openstreetmap.org/16/32744/21792.png"
    );
    assert_eq!(tile.key(), "16-32744-21792");
}
