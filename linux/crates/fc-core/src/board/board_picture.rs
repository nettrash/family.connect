//! How a PICTURE is drawn on the wall (docs/protocol.md, "Board") — a port of `BoardPicture.cs`.
//!
//! A PHOTO IS DRAWN WHOLE: fitted in both dimensions and never cropped to fill its box. Issue #71
//! is what filling costs — a portrait photograph from a phone lost more than half its height on
//! every client that fitted the width alone, faces and all.
//!
//! The arithmetic is `fc_text::board::fitted_picture`'s. Apple: `BoardPicture`. Android:
//! `BoardPicture`.

use fc_text::board::fitted_picture;

/// The size a picture of `picture_width` × `picture_height` pixels takes inside a space, fitted
/// in both dimensions: a tall photograph on a wide card comes back narrow, a wide one short, and
/// neither comes back cropped.
///
/// Also the size of a BARE photo's card, which is the picture itself: the note's box hugs this,
/// so the pin sits on the photograph rather than over bare wall.
///
/// A picture the server never gave dimensions for (`None`, as an attachment's `width`/`height`
/// may be) takes the whole space, which costs a margin at worst — the picture is still drawn
/// fitted inside it and never cropped. Never below a hairline: a panorama 20 000 pixels wide
/// would round its height to nothing, and a card of no height is a note nobody can click.
pub fn fitted(
    space_width: f64,
    space_height: f64,
    picture_width: Option<i32>,
    picture_height: Option<i32>,
) -> (f64, f64) {
    fitted_picture(
        (space_width, space_height),
        (
            f64::from(picture_width.unwrap_or(0)),
            f64::from(picture_height.unwrap_or(0)),
        ),
    )
}

/// FITTED, never filled — the one word issue #71 was. Named rather than written into the view so
/// a test can hold it, exactly as `NotePicture.contentMode` and `BoardPicture.scale` are named on
/// Apple and Android, and `BoardPicture.Stretch` (`"Uniform"`, XAML's word) on Windows.
///
/// `"contain"` is the nick of GTK's `gtk::ContentFit::Contain` — the shell reads it back into the
/// enum the way the Windows view parses `Stretch` — and also CSS's `object-fit: contain`, should a
/// sticker be drawn in a web view.
pub const CONTENT_FIT: &str = "contain";
