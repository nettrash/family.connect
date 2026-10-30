//! Profile pictures into P1's `adw::Avatar`s (shell-design §8.3): [`show`] loads the picture of
//! (user, VERSION) from the connection's `AvatarCache` and sets it as the avatar's custom image —
//! initials otherwise, which the avatar draws by itself from its text.
//!
//! The version is part of the key (`AvatarCache`'s rule: no frame carries the picture, only the
//! number), "no picture" is an answer kept for that version, and a transient failure keeps the
//! initials and asks again on the next look. Decoded pictures are kept in memory per (user,
//! version), so a rebuilt list draws its faces at once.
//!
//! An avatar widget may be asked for somebody else before an earlier load lands (a row reused):
//! only the latest ask is drawn — each avatar's latest (user, version) is remembered beside a weak
//! reference to it.

use std::cell::RefCell;
use std::collections::HashMap;

use adw::prelude::*;
use gtk::gdk;

use crate::shell::Shell;

/// The pixel edge a picture is decoded at: the largest avatar drawn (the viewer-free 64-pt rows)
/// at 2×.
const DECODE_EDGE: i32 = 128;

thread_local! {
    /// Decoded faces by (user, version); `None` is "no picture", kept for that version.
    static FACES: RefCell<HashMap<(i64, i32), Option<gdk::Texture>>> = RefCell::new(HashMap::new());
    /// Each avatar's latest ask.
    static ASKED: RefCell<Vec<(glib::WeakRef<adw::Avatar>, (i64, i32))>> = const { RefCell::new(Vec::new()) };
}

/// The most faces kept in memory; past it the table is emptied and refilled as rows are drawn.
const MOST_FACES: usize = 256;

/// Draws the picture of `user_id` at `avatar_version` into `avatar` (P1's rows call this after
/// setting the avatar's text). Version 0 (no picture) clears any custom image at once, without a
/// request. Safe to call again for the same avatar with somebody else.
pub fn show(avatar: &adw::Avatar, user_id: i64, avatar_version: i32) {
    let key = (user_id, avatar_version);
    remember(avatar, key);
    if avatar_version <= 0 || user_id <= 0 {
        avatar.set_custom_image(None::<&gdk::Paintable>);
        return;
    }
    if let Some(known) = FACES.with(|faces| faces.borrow().get(&key).cloned()) {
        avatar.set_custom_image(known.as_ref());
        return;
    }
    // Initials while it loads (a reused row must not show the last person's face).
    avatar.set_custom_image(None::<&gdk::Paintable>);
    let weak = avatar.downgrade();
    glib::spawn_future_local(async move {
        let Some(connection) = Shell::try_get().and_then(|shell| shell.connection()) else {
            return;
        };
        let face = match connection.avatars.bytes(user_id, avatar_version).await {
            Ok(Some(bytes)) => super::pictures::decode_square(bytes, DECODE_EDGE).await,
            Ok(None) => None,
            Err(error) => {
                // Transient: nothing kept, the next look asks again.
                crate::fc_info!("avatar of {user_id}: not fetched ({})", error.code);
                return;
            }
        };
        FACES.with(|faces| {
            let mut faces = faces.borrow_mut();
            if faces.len() >= MOST_FACES {
                faces.clear();
            }
            faces.insert(key, face.clone());
        });
        if let Some(avatar) = weak.upgrade()
            && latest(&avatar) == Some(key)
        {
            avatar.set_custom_image(face.as_ref());
        }
    });
}

/// [`show`] with the version the roster holds for `user_id` — the reader's own from the session,
/// anybody else's from the family's members (`ChatStore::member`): the faces of the chat list, the
/// timeline, the Family and group dialogs, Settings and the Call window.
pub fn show_member(avatar: &adw::Avatar, user_id: i64) {
    let version = Shell::try_get()
        .and_then(|shell| shell.connection())
        .map_or(0, |connection| version_of(&connection, user_id));
    show(avatar, user_id, version);
}

/// The avatar version the connection knows for `user_id` (0: none).
pub fn version_of(connection: &crate::shell::Connection, user_id: i64) -> i32 {
    if let Some(me) = connection.session.state().me.filter(|me| me.id == user_id) {
        return me.avatar_version;
    }
    connection
        .chats
        .member(user_id)
        .ok()
        .flatten()
        .map_or(0, |member| member.avatar_version)
}

/// A profile picture made ready to send (`avatar_rules`, docs/protocol.md "Profile pictures"): the
/// server stores what it is given and never transcodes, so a JPEG or PNG already within 512 px goes
/// as it is, and anything else gdk-pixbuf reads is fitted (`avatar_rules::fit`, never up), its EXIF
/// turn applied, and written as a JPEG at quality 85. Answers the bytes and their type, or `None`
/// for a picture nothing here reads. (P4's Settings row sends what this answers.)
pub async fn prepare_upload(bytes: Vec<u8>) -> Option<(Vec<u8>, &'static str)> {
    use fc_logic::avatars::avatar_rules;
    let bytes = glib::Bytes::from_owned(super::pictures::recognisable(bytes));
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream_future(&stream)
        .await
        .ok()?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    let declared = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else {
        fc_text::pack::type_of(&bytes).filter(|mime| *mime == fc_text::pack::PNG)
    };
    let (width, height) = (pixbuf.width(), pixbuf.height());
    if let Some(mime) = declared
        && avatar_rules::is_allowed(mime)
        && !avatar_rules::needs_downscale(width, height)
    {
        return Some((bytes.to_vec(), mime));
    }
    let (to_width, to_height) = avatar_rules::fit(width, height);
    let fitted = if (to_width, to_height) == (width, height) {
        pixbuf
    } else {
        pixbuf.scale_simple(to_width, to_height, gtk::gdk_pixbuf::InterpType::Hyper)?
    };
    // JPEG has no alpha: the picture is put on white, into a pixbuf without an alpha channel.
    let opaque = if fitted.has_alpha() {
        let white = gtk::gdk_pixbuf::Pixbuf::new(
            gtk::gdk_pixbuf::Colorspace::Rgb,
            false,
            8,
            fitted.width(),
            fitted.height(),
        )?;
        white.fill(0xFFFF_FFFF);
        fitted.composite(
            &white,
            0,
            0,
            fitted.width(),
            fitted.height(),
            0.0,
            0.0,
            1.0,
            1.0,
            gtk::gdk_pixbuf::InterpType::Nearest,
            255,
        );
        white
    } else {
        fitted
    };
    let jpeg = opaque.save_to_bufferv("jpeg", &[("quality", "85")]).ok()?;
    Some((jpeg, avatar_rules::SEND_AS))
}

fn remember(avatar: &adw::Avatar, key: (i64, i32)) {
    ASKED.with(|asked| {
        let mut asked = asked.borrow_mut();
        asked.retain(|(weak, _)| weak.upgrade().is_some());
        match asked
            .iter_mut()
            .find(|(weak, _)| weak.upgrade().as_ref() == Some(avatar))
        {
            Some(entry) => entry.1 = key,
            None => asked.push((avatar.downgrade(), key)),
        }
    });
}

fn latest(avatar: &adw::Avatar) -> Option<(i64, i32)> {
    ASKED.with(|asked| {
        asked
            .borrow()
            .iter()
            .find(|(weak, _)| weak.upgrade().as_ref() == Some(avatar))
            .map(|(_, key)| *key)
    })
}
