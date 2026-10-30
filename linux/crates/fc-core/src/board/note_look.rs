//! The names a note's look travels under, and the fallbacks that keep an unknown one readable — a
//! port of `NoteLook.cs`.
//!
//! EVERY name that comes off the wire falls back rather than failing: a note from a newer server
//! with a fourth size or a fifth colour still has to be readable, and a hole in the family's
//! shared layout would be worse than a sticker drawn in the default. The fallback is for DRAWING
//! only — `Size::patch_name` is why an edit must not write it back.
//!
//! All of it is `fc_text::board`'s — `Size`, `Kind`, `Font`, `Answer`, `COLORS`, `color_hex`,
//! `going_line` and the titles — except the one answer that is each platform's own: which face a
//! hand is drawn in.

use fc_text::board::Font;

/// The face each hand asks for on Ubuntu, as a family list (Pango's: comma-separated, tried in
/// order, which is what `pango::FontDescription` takes).
///
/// The hand is an INTENT, so this is the Linux answer to it and nobody else's — Windows names
/// Segoe UI, Georgia, Cascadia Mono and Segoe Print; the web, `Font::css_family`. Nothing is
/// bundled and nothing is synthesised: the faces are Ubuntu 26.04 desktop's own default set (the
/// `desktop-common` seed: `fonts-ubuntu`, `fonts-noto-core`, `fonts-dejavu-core`,
/// `fonts-liberation`), and every list ends in a fontconfig generic family, which always resolves
/// — a face that is missing falls through the list the way the web's does.
///
/// `casual` is the one exception the Mac's own note allows for ("not the plain one", not a promise
/// of the same curves): no informal face is in Ubuntu's default set, so it names the common
/// optional ones (`fonts-comic-neue`; URW Gothic from `fonts-urw-base35`, which Ghostscript brings)
/// and then fontconfig's `cursive`.
pub fn font_family(font: Font) -> &'static str {
    match font {
        Font::Plain => "Ubuntu Sans, Noto Sans, DejaVu Sans, sans-serif",
        Font::Serif => "Noto Serif, DejaVu Serif, Liberation Serif, serif",
        Font::Mono => "Ubuntu Sans Mono, DejaVu Sans Mono, Liberation Mono, monospace",
        Font::Casual => "Comic Neue, URW Gothic, cursive",
    }
}
