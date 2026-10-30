//! Where a note sits, and how big it is drawn (docs/protocol.md, "Board") — a port of
//! `BoardWall.cs`.
//!
//! The numbers are shared by all four clients even though the wire says nothing about them: `x`
//! and `y` are fractions of the WALL, so a note two thirds of the way down has to be two thirds of
//! the way down on the phone, on the Mac, in a browser and here.
//!
//! Every rule but one is `fc_text::board`'s: `WALL_SCREENS`, `wall_height`, `COMPACT_BELOW`,
//! `is_compact`, `Size::frame` (`Card`), `Size::type_px` (`TypePx`), `clamp_corner` (`Clamp`),
//! `origin`, `dragged`, `fraction_of` and `tilt_degrees`. Apple: `BoardWall` + `NoteSize.frame`.
//! Android: `BoardWall` + `NoteSizes`.

use fc_text::board::wall_height;

/// The wall's own size for a window of `visible_width` × `visible_height`, for the fractions to be
/// read against: as wide as the window and [`wall_height`] tall — never shorter than the window,
/// or fractions of the wall would sit behind its edges.
pub fn size(visible_width: f64, visible_height: f64) -> (f64, f64) {
    (visible_width, wall_height(visible_height))
}
