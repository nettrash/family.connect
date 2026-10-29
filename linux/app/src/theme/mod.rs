//! Dark mode, the accent and HiDPI (shell-design §25).
//!
//! - **Colour scheme**: libadwaita follows the system (the settings portal; `ADW_DEBUG_COLOR_SCHEME`
//!   in a test) — there is no in-app switch, as the Mac follows the system appearance.
//! - **One provider** at `STYLE_PROVIDER_PRIORITY_APPLICATION` holding [`css::css`] for the current
//!   scheme, reloaded when libadwaita's dark flag flips; then [`changed`] is raised for what CSS
//!   cannot reach (a rich drawing's key, a `WebView`'s background).
//! - **High contrast and reduced motion** reach the sheet's `@media (prefers-contrast: more)` and
//!   `(prefers-reduced-motion: reduce)` through the provider's own properties: GTK asks each
//!   provider, and only libadwaita sets its own (measured: under `ADW_DEBUG_HIGH_CONTRAST` the
//!   app's `@media` rules stayed off until told).
//! - **Packages add CSS** with [`register`] (call it from your `install`, before or after
//!   [`install`] — the sheet is rebuilt either way). Prefix your classes with your package's area
//!   (`fc-chat-…`, `fc-media-…`); the frozen classes are listed in [`css`].
//! - **HiDPI**: GTK draws vectors and text at the surface's scale; a bitmap the app makes (avatars,
//!   tiles, rich drawings, posters) is made at [`bitmap_scale`] × its logical size (a rich
//!   drawing's key carries the integer scale).
//! - **The app's own glyphs** ([`glyphs`]) where icon themes draw two things alike or have none:
//!   symbolic paintables, recoloured like a theme's icons.

pub mod css;
pub mod glyphs;
pub mod palette;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use fc_core::rt::Event;

use palette::Palette;

/// A package's CSS, made for a palette.
pub type CssPart = Rc<dyn Fn(&Palette) -> String>;

thread_local! {
    static PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static PARTS: RefCell<Vec<CssPart>> = const { RefCell::new(Vec::new()) };
    static CHANGED: Rc<Event<bool>> = Rc::new(Event::new());
}

/// Raised after the sheet was reloaded for the other scheme, with whether it is now dark.
pub fn changed() -> Rc<Event<bool>> {
    CHANGED.with(Rc::clone)
}

/// libadwaita's answer: the system preference as the application follows it.
pub fn is_dark() -> bool {
    adw::StyleManager::default().is_dark()
}

/// The palette of the scheme now shown.
pub fn palette() -> Palette {
    Palette::for_dark(is_dark())
}

/// `startup`: installs the provider on the default display and follows the dark flag. Idempotent;
/// does nothing without a display (`--version`, a unit test).
pub fn install() {
    if PROVIDER.with(|p| p.borrow().is_some()) {
        return;
    }
    let Some(display) = gtk::gdk::Display::default() else {
        crate::fc_warn!("theme: no display; the stylesheet is not installed");
        return;
    };
    let provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    PROVIDER.with(|p| *p.borrow_mut() = Some(provider));
    follow_preferences();
    reload_sheet();
    let manager = adw::StyleManager::default();
    manager.connect_dark_notify(|_| {
        reload_sheet();
        changed().emit(&is_dark());
    });
    manager.connect_high_contrast_notify(|_| {
        follow_preferences();
        reload_sheet();
    });
    gtk::Settings::for_display(&display).connect_gtk_interface_reduced_motion_notify(|_| {
        follow_preferences();
        reload_sheet();
    });
}

/// Tells the provider what the desktop prefers (see the module's comment).
fn follow_preferences() {
    let Some(provider) = PROVIDER.with(|p| p.borrow().clone()) else {
        return;
    };
    provider.set_prefers_contrast(if adw::StyleManager::default().is_high_contrast() {
        gtk::InterfaceContrast::More
    } else {
        gtk::InterfaceContrast::NoPreference
    });
    if let Some(settings) = gtk::Settings::default() {
        provider.set_prefers_reduced_motion(settings.gtk_interface_reduced_motion());
    }
}

/// Adds a package's CSS to the sheet (rebuilt now if the provider exists).
pub fn register(part: impl Fn(&Palette) -> String + 'static) {
    PARTS.with(|parts| parts.borrow_mut().push(Rc::new(part)));
    reload_sheet();
}

/// The sheet as it is now: the frozen part and every registered one, for the current scheme.
pub fn current_sheet() -> String {
    let palette = palette();
    let parts: Vec<String> =
        PARTS.with(|parts| parts.borrow().iter().map(|p| p(&palette)).collect());
    css::css(&palette, &parts)
}

fn reload_sheet() {
    let Some(provider) = PROVIDER.with(|p| p.borrow().clone()) else {
        return;
    };
    provider.load_from_string(&current_sheet());
}

/// The integer scale a bitmap for `widget` is made at: `ceil` of its surface's scale (fractional
/// on Wayland), at least 1. Before the widget is realised, its scale factor.
pub fn bitmap_scale(widget: &impl IsA<gtk::Widget>) -> i32 {
    let scale = widget
        .native()
        .and_then(|native| native.surface())
        .map(|surface| surface.scale())
        .unwrap_or_else(|| f64::from(widget.scale_factor()));
    bitmap_scale_for(scale)
}

/// [`bitmap_scale`]'s rule. Pure.
pub fn bitmap_scale_for(scale: f64) -> i32 {
    if scale.is_finite() && scale > 1.0 {
        // A scale is a small number; the cast cannot truncate anything real.
        scale.ceil().min(16.0) as i32
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bitmap_is_made_at_the_ceiling_of_the_scale() {
        assert_eq!(bitmap_scale_for(1.0), 1);
        assert_eq!(bitmap_scale_for(1.25), 2);
        assert_eq!(bitmap_scale_for(1.5), 2);
        assert_eq!(bitmap_scale_for(2.0), 2);
        assert_eq!(bitmap_scale_for(2.25), 3);
        assert_eq!(bitmap_scale_for(0.5), 1);
        assert_eq!(bitmap_scale_for(f64::NAN), 1);
        assert_eq!(bitmap_scale_for(f64::INFINITY), 1);
        assert_eq!(bitmap_scale_for(400.0), 16);
    }
}
