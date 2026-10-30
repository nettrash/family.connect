//! The app's own symbolic glyphs, for what icon themes draw alike or not at all. Each is a 16-px
//! SVG in `glyphs/`, drawn by GTK's SVG paintable in the ink of the widget showing it (the accent
//! where that is tinted), at any size and scale — as a theme's `-symbolic` icon would be.

/// A sticker: a face on a square with a peeled corner. Icon themes have no sticker, and their
/// smileys are the emoji chooser's (Yaru's emoji and smiley are one outlined face).
pub fn sticker() -> gtk::Svg {
    symbolic(include_bytes!("glyphs/sticker-symbolic.svg"))
}

/// A video message: a camera in a ring, the Mac's `video.circle` — the bare camera is the
/// header's Video Call.
pub fn video_circle() -> gtk::Svg {
    symbolic(include_bytes!("glyphs/video-circle-symbolic.svg"))
}

/// Every fill in the symbolic foreground. GTK reads it so only with its extensions on as well:
/// `TRADITIONAL_SYMBOLIC` alone drew black on the dark ground (measured, GTK 4.22).
fn symbolic(svg: &'static [u8]) -> gtk::Svg {
    let paintable = gtk::Svg::new();
    paintable.set_features(gtk::SvgFeatures::EXTENSIONS | gtk::SvgFeatures::TRADITIONAL_SYMBOLIC);
    paintable.load_from_bytes(&glib::Bytes::from_static(svg));
    paintable
}
