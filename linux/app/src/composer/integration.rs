//! The composer's and the menu's doors into the other packages (W.2), one function each:
//!
//! | Function | Owner | What it does |
//! | --- | --- | --- |
//! | [`sticker_panel_offered`], [`open_sticker_panel`] | P2 | the family's sticker panel (§8.4) from the field's sticker glyph, where the server has a pack |
//! | [`location_offered`], [`find_location`] | P2 | the Location portal, once (§8.5): the paperclip's Location where it answers |
//! | [`open_full_screen`] | P2 | the attachment viewer window (§8.2) |
//! | [`text_item`], [`show_text`] | P3 | a recording's text (`TranscriptModel`, through the consent question; `media::transcript`) |
//! | [`prepare_file`] | P3 | `MediaPreparing` for a video, a sound or an animated picture (§10.7: transcode, poster, read-back, cancel); a still photo re-drawn as JPEG with a preview (gdk-pixbuf) and a file sent as it is |
//!
//! [`next_voice_speed`] is the `voice-speed` setting P3's player applies.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use fc_core::protocol::{MessageDto, PackItemDto};
use fc_core::rt::Cancel;
use fc_core::text::location_fixes::LocationFix;
use fc_logic::composer_staging::{PrepFailure, PrepOutcome};
use fc_logic::media_outbox::StagedMedia;
use fc_logic::voice_look::voice_speed;
use fc_text::media::Route;
use gtk::prelude::*;

use crate::attachments::location_portal::{self, Refusal};
use crate::conversation::ConversationPage;
use crate::media::preparing::Picked;
use crate::services::settings_store::keys;
use crate::shell::Shell;

thread_local! {
    /// Whether the Location portal answered, once asked (`None`: not asked yet).
    static LOCATION_PORTAL: Cell<Option<bool>> = const { Cell::new(None) };
}

/// P2: whether the field draws its sticker glyph at all — the server keeps a pack for the family
/// (`attachments::pack::offered`).
pub fn sticker_panel_offered() -> bool {
    Shell::try_get()
        .and_then(|shell| shell.connection())
        .is_some_and(|connection| crate::attachments::pack::offered(&connection.stickers))
}

/// P2: the sticker panel over `anchor` (the sticker glyph — never a text view, §6, P1's measured
/// warning); a pick closes it and goes to `on_pick`.
pub fn open_sticker_panel(anchor: &gtk::Widget, on_pick: impl Fn(PackItemDto) + 'static) {
    let popover = crate::attachments::sticker_panel::popover(on_pick);
    popover.set_parent(anchor);
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}

/// P2: whether the paperclip offers Location — the portal answered its `version`. Asked once per
/// run, in the background; until it has answered, the line is not offered.
pub fn location_offered() -> bool {
    match LOCATION_PORTAL.with(Cell::get) {
        Some(answered) => answered,
        None => {
            LOCATION_PORTAL.with(|asked| asked.set(Some(false)));
            glib::spawn_future_local(async {
                let answered = location_portal::available().await;
                LOCATION_PORTAL.with(|asked| asked.set(Some(answered)));
            });
            false
        }
    }
}

/// P2: "where I am, once" through the Location portal, its dialog over `window`.
pub async fn find_location(window: Option<gtk::Window>) -> Result<LocationFix, Refusal> {
    // The handle lives until the portal has answered: GNOME parents its dialog with it.
    let parent = crate::desktop::portal_request::parent_window(window.as_ref()).await;
    location_portal::find(parent.identifier()).await
}

/// P2: the viewer window for `message`'s attachment `index`.
pub fn open_full_screen(message: &MessageDto, index: usize) {
    crate::viewer::open_message(message, index);
}

/// P3: the menu's text item for `message` — "Show text" exactly where the words under its recording
/// offer it, "Hide text" while the text is open (`message_menu::text_item_for`).
pub fn text_item(message: &MessageDto) -> fc_logic::message_menu::TextItem {
    crate::media::transcript::menu_item(message)
}

/// P3: a recording's text — folded away when it is open, else asked for as the words under it ask
/// (the consent question over the page's window).
pub fn show_text(page: &ConversationPage, message_id: i64) {
    let Ok(Some(message)) = page.connection().chats.message(message_id) else {
        return;
    };
    crate::media::transcript::toggle(&page.widget(), &message);
}

/// The voice speed's next step (1× → 1.5× → 2× → 1×), kept in `settings.conf` for P3's player.
pub fn next_voice_speed(shell: &Rc<Shell>) {
    let rate = voice_speed::parse(shell.settings.string(keys::VOICE_SPEED).as_deref());
    shell.settings.set_string(
        keys::VOICE_SPEED,
        &voice_speed::store(voice_speed::next(rate)),
    );
}

/// A picked or dropped file prepared for the strip (`ComposerStaging::ingest`'s preparer): P3's
/// `prepare_picked` first — a video or a sound brought to the profile with its poster, an animated
/// picture made into the animation, a refusal said — and P1's own path for what is not P3's (a
/// still photo, any other file) and for an animated picture P3 could not make into one (it goes as
/// it did before #96). `animates`: false in the assistant's chat, whose model is shown a
/// photograph — an animated picture there goes as one (the Mac's `MediaPrep.prepare(animates:)`).
/// `Err` is the person's cancel, or a file that could not be read.
pub async fn prepare_file(
    file: gio::File,
    cancel: Cancel,
    animates: bool,
) -> Result<PrepOutcome, String> {
    let name = file
        .basename()
        .and_then(|name| name.to_str().map(str::to_string))
        .unwrap_or_default();
    let mime = mime_of(&file).await;
    let still = !animates && mime.starts_with("image/");
    if let Some(path) = file.path().filter(|_| !still) {
        match crate::media::preparing::prepare_picked(&path, &mime, Some(&name), &cancel).await {
            Err(_) => return Err("cancelled".to_string()),
            Ok(Picked::Staged(media)) => return Ok(PrepOutcome::staged(*media)),
            Ok(Picked::Refused(failure)) => return Ok(PrepOutcome::refused(failure)),
            Ok(Picked::NotMine | Picked::AsBefore) => {}
        }
    }
    let (bytes, _) = file
        .load_contents_future()
        .await
        .map_err(|error| error.to_string())?;
    if cancel.is_cancelled() {
        return Err("cancelled".to_string());
    }
    Ok(prepare_bytes(bytes.to_vec(), &mime, &name))
}

/// What the desktop says `file` is (`standard::content-type`, sniffed), as a MIME type; empty when
/// it cannot say.
async fn mime_of(file: &gio::File) -> String {
    let info = file
        .query_info_future(
            gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await;
    info.ok()
        .and_then(|info| info.content_type())
        .and_then(|kind| gio::content_type_get_mime_type(&kind))
        .map(|mime| mime.to_string())
        .unwrap_or_default()
}

/// Bytes from the clipboard or a file, as the strip takes them.
pub fn prepare_bytes(bytes: Vec<u8>, mime: &str, name: &str) -> PrepOutcome {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > fc_text::media::SIZE_LIMIT {
        return PrepOutcome::refused(PrepFailure::TooLarge);
    }
    if bytes.is_empty() {
        return PrepOutcome::refused(PrepFailure::Unreadable);
    }
    let head = &bytes[..bytes.len().min(64)];
    let safe = fc_text::media::sanitized_name(name);
    match fc_text::media::route(mime, name, head) {
        Route::Photo => match photo(&bytes) {
            Some(media) => PrepOutcome::staged(media),
            None => PrepOutcome::refused(PrepFailure::Unreadable),
        },
        Route::Video => PrepOutcome::staged(StagedMedia {
            name: safe,
            ..StagedMedia::new("video", &fc_text::media::declared_type(mime, name), bytes)
        }),
        Route::Audio(audio) => PrepOutcome::staged(StagedMedia {
            name: safe,
            ..StagedMedia::new("audio", audio, bytes)
        }),
        Route::File => {
            let declared = fc_text::media::declared_type(mime, name);
            let declared = if declared.is_empty() {
                "application/octet-stream".to_string()
            } else {
                declared
            };
            PrepOutcome::staged(StagedMedia {
                name: safe,
                ..StagedMedia::new("file", &declared, bytes)
            })
        }
    }
}

/// A still photo re-drawn as the apps send one: at most `PHOTO_EDGE` either way, JPEG at
/// `PHOTO_QUALITY`, with a `PREVIEW_EDGE` preview — through gdk-pixbuf (glycin underneath).
fn photo(bytes: &[u8]) -> Option<StagedMedia> {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(bytes));
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream(&stream, gio::Cancellable::NONE).ok()?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    let (width, height) = (
        u32::try_from(pixbuf.width()).ok()?,
        u32::try_from(pixbuf.height()).ok()?,
    );
    let encode = |edge: u32, quality: f64| -> Option<(Vec<u8>, u32, u32)> {
        let (w, h) = fc_text::media::fit_within(width, height, edge);
        let scaled = if (w, h) == (width, height) {
            pixbuf.clone()
        } else {
            pixbuf.scale_simple(
                i32::try_from(w).ok()?,
                i32::try_from(h).ok()?,
                gtk::gdk_pixbuf::InterpType::Bilinear,
            )?
        };
        let quality = format!("{}", (quality * 100.0).round() as i64);
        let jpeg = opaque(&scaled)?
            .save_to_bufferv("jpeg", &[("quality", quality.as_str())])
            .ok()?;
        Some((jpeg, w, h))
    };
    let (jpeg, w, h) = encode(fc_text::media::PHOTO_EDGE, fc_text::media::PHOTO_QUALITY)?;
    let preview = encode(
        fc_text::media::PREVIEW_EDGE,
        fc_text::media::PREVIEW_QUALITY,
    )
    .map(|(bytes, _, _)| bytes);
    Some(StagedMedia {
        width: i32::try_from(w).ok(),
        height: i32::try_from(h).ok(),
        preview,
        ..StagedMedia::new("photo", "image/jpeg", jpeg)
    })
}

/// `pixbuf` without its alpha channel, over white — what a JPEG can hold. MEASURED on Ubuntu
/// 26.04 (gdk-pixbuf 2.44 over glycin): saving an RGBA pixbuf as `jpeg` fails with "The encoder or
/// decoder for Jpeg does not support the color type `Rgba8`", so every picture with alpha (any
/// PNG) must be flattened first.
fn opaque(pixbuf: &gtk::gdk_pixbuf::Pixbuf) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    if !pixbuf.has_alpha() {
        return Some(pixbuf.clone());
    }
    let (width, height) = (pixbuf.width(), pixbuf.height());
    let flat =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, width, height)?;
    flat.fill(0xffff_ffff);
    pixbuf.composite(
        &flat,
        0,
        0,
        width,
        height,
        0.0,
        0.0,
        1.0,
        1.0,
        gtk::gdk_pixbuf::InterpType::Bilinear,
        255,
    );
    Some(flat)
}

/// Whether `path` is a folder (a dropped folder is refused: the drag springs back).
pub fn is_folder(file: &gio::File) -> bool {
    file.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
        == gio::FileType::Directory
        || file.path().is_some_and(|path| Path::new(&path).is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_with_alpha_is_flattened_onto_white_for_jpeg() {
        // gdk-pixbuf's own code only: no glycin loader is started by a unit test (the decode and
        // the JPEG write are proved by the `chat` self-test's drop).
        let rgba = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 4, 3)
            .expect("a pixbuf");
        rgba.fill(0x0000_0000);
        let flat = opaque(&rgba).expect("flattened");
        assert!(!flat.has_alpha());
        assert_eq!((flat.width(), flat.height()), (4, 3));
        // Fully transparent black over white is white.
        let pixels = flat.read_pixel_bytes();
        assert_eq!(&pixels[..3], &[255, 255, 255]);
        let rgb = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 2, 2)
            .expect("a pixbuf");
        assert!(!opaque(&rgb).expect("kept").has_alpha());
    }

    #[test]
    fn anything_else_goes_as_a_file_with_its_name() {
        let outcome = prepare_bytes(b"%PDF-1.7 hello".to_vec(), "", "notes.pdf");
        let media = outcome.media.expect("staged");
        assert_eq!(media.kind, "file");
        assert_eq!(media.name.as_deref(), Some("notes.pdf"));
        assert!(prepare_bytes(Vec::new(), "", "empty.txt").media.is_none());
    }
}
