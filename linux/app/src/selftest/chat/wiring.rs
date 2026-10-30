//! The INTEGRATION pass's checks (wave 3): what the packages were wired to each other with, proved
//! in P1's real conversation against the fake server — never in a package's own test window.
//!
//! - P1 ↔ P2: a person's picture over the initials in the chat list and in the timeline; the
//!   bubbles' tiles, sticker, place and file row from P2's slots; the composer's sticker panel and
//!   a pick that sends; Location (refused with no portal, and a place sent); "Open Full Screen"
//!   (a photo, a video that plays); Save… through P2's saving; a file handed to the inbox staged in
//!   the chosen chat through `ChatRoutes::stage_files`.
//! - P1 ↔ P3: a picked video prepared by P3 before it is staged; a voice note in REVIEW parked as
//!   "not sent" when the chat is left; P3's refusal said on P1's notice line; a video message sent
//!   with `round: true` and its reply; a circle's expand opening the round viewer; an animation
//!   playing in its tile.
//! - P1 ↔ P4: Group Info from a group's header.
//! - P1 ↔ P6: Call and Video Call in a direct chat's header, insensitive while a voice note
//!   records; a record's "Call back" placing a call.
//!
//! It runs after the chat group's own checks, with two more chats listed (`World::wiring`): a
//! direct chat with Finn carrying every kind of attachment, and a group. What it turned on (calls,
//! the pack's ceilings, the test microphone) is turned off again at the end.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::{
    AttachmentDto, AttachmentResponse, CallRecordDto, MessageDto, PackItemDto,
};
use fc_core::text::location_fixes::LocationFix;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::media_outbox::StagedMedia;
use gtk::gdk_pixbuf;

use super::super::Harness;
use super::super::fake_server::{FakeResponse, FakeServer};
use super::{SHORT, WAIT, World, act, find, now_rfc3339, query_of};
use crate::chats;
use crate::conversation::ConversationPage;
use crate::shell::Shell;
use crate::windows::main_window::{DEFAULT_SIZE, MainWindow};

/// The attachments this pass serves (far from every other group's ids).
const PHOTO: i64 = 990_101;
const STICKER: i64 = 990_102;
const PLACE: i64 = 990_103;
const FILE: i64 = 990_104;
const VIDEO: i64 = 990_105;
const ANIMATION: i64 = 990_106;
const ROUND: i64 = 990_107;
const PACK_PICTURE: i64 = 990_201;
/// Ids the fake gives uploads from.
const FIRST_UPLOAD: i64 = 991_000;

/// The messages of the media chat, by what they carry.
struct Seeded {
    photo: i64,
    file: i64,
    video: i64,
    round: i64,
}

pub(super) async fn run(
    h: &Harness,
    shell: &Rc<Shell>,
    main: &Rc<MainWindow>,
    window: &gtk::Window,
    world: &Rc<World>,
) {
    let Some(chats) = chats::instance() else {
        h.check("wiring", false, "no chats instance");
        return;
    };
    let Some(connection) = shell.connection() else {
        h.check("wiring", false, "no connection");
        return;
    };
    let folder = h.out().join("wiring");
    let _ = std::fs::create_dir_all(&folder);

    // ---- what the fake serves: pictures, videos, avatars; uploads are taken in ----------------------
    let clip = folder.join("clip.mp4");
    let square = folder.join("square.mp4");
    let animation = folder.join("animation.mp4");
    let made = make(
        h,
        &[
            &format!("videotestsrc num-buffers=60 pattern=smpte ! video/x-raw,width=640,height=360,framerate=30/1 ! x264enc ! video/x-h264,profile=high ! h264parse ! mp4mux name=m faststart=true ! filesink location={} audiotestsrc num-buffers=90 ! audio/x-raw,rate=44100,channels=1 ! avenc_aac ! aacparse ! m.", clip.display()),
            &format!("videotestsrc num-buffers=60 pattern=ball ! video/x-raw,width=480,height=480,framerate=30/1 ! x264enc ! video/x-h264,profile=high ! h264parse ! mp4mux name=m faststart=true ! filesink location={} audiotestsrc num-buffers=90 ! audio/x-raw,rate=44100,channels=1 ! avenc_aac ! aacparse ! m.", square.display()),
            &format!("videotestsrc num-buffers=30 pattern=pinwheel ! video/x-raw,width=320,height=240,framerate=15/1 ! x264enc ! video/x-h264,profile=high ! h264parse ! mp4mux ! filesink location={}", animation.display()),
        ],
    )
    .await;
    h.check("wiring_media_made", made, folder.display().to_string());
    let read = |path: &std::path::Path| std::fs::read(path).unwrap_or_default();
    let (clip_bytes, square_bytes, animation_bytes) =
        (read(&clip), read(&square), read(&animation));
    let blobs: Rc<std::cell::RefCell<std::collections::HashMap<String, Vec<u8>>>> = Rc::default();
    {
        let mut blobs = blobs.borrow_mut();
        blobs.insert(
            format!("{PHOTO}/preview"),
            picture(600, 450, 0x2e7d_32ff, "jpeg"),
        );
        blobs.insert(PHOTO.to_string(), picture(1600, 1200, 0x2e7d_32ff, "jpeg"));
        blobs.insert(STICKER.to_string(), picture(300, 300, 0xd81b_60ff, "png"));
        blobs.insert(FILE.to_string(), b"%PDF-1.4 the trip".to_vec());
        blobs.insert(VIDEO.to_string(), clip_bytes.clone());
        blobs.insert(
            format!("{VIDEO}/preview"),
            picture(640, 360, 0x1565_c0ff, "jpeg"),
        );
        blobs.insert(ANIMATION.to_string(), animation_bytes.clone());
        blobs.insert(
            format!("{ANIMATION}/preview"),
            picture(320, 240, 0x6a1b_9aff, "jpeg"),
        );
        blobs.insert(ROUND.to_string(), square_bytes.clone());
        blobs.insert(
            format!("{ROUND}/preview"),
            picture(480, 480, 0xef6c_00ff, "jpeg"),
        );
        blobs.insert(
            PACK_PICTURE.to_string(),
            picture(200, 200, 0x00ac_c1ff, "png"),
        );
    }
    routes(&world.fake, world, &blobs);
    {
        let mut state = world.fake.state();
        state.calls = true;
        state.video_calls = true;
        state.pack_limits = Some((48, 512 * 1024));
        for user in state.users.iter_mut() {
            if user.id == world.eve || user.id == world.finn {
                user.avatar_version = 3;
            }
        }
    }
    let seeded = seed(world, animation_bytes.len());
    world.wiring.set(true);
    act(main, "win.refresh", None);
    let Some(list) = chats.list() else {
        h.check("wiring", false, "no list");
        return;
    };
    let listed = h
        .wait_until(WAIT, || {
            let ids = list.listed();
            ids.contains(&world.media_chat)
                && ids.contains(&world.group_chat)
                && connection.session.state().calls_enabled
                && connection.stickers.offered().unwrap_or(false)
        })
        .await;
    h.check(
        "wiring_world_listed",
        listed,
        format!(
            "{:?}; calls {}; pack {:?}",
            list.listed(),
            connection.session.state().calls_enabled,
            connection.stickers.offered()
        ),
    );
    // The pack this family keeps, as a catch-up would put it (the fake names no pack sequence).
    let mut pack_picture = AttachmentDto::new(PACK_PICTURE, "photo");
    pack_picture.sticker = true;
    pack_picture.width = Some(200);
    pack_picture.height = Some(200);
    pack_picture.mime = Some("image/png".into());
    let item = PackItemDto {
        added_by: world.eve,
        attachment: Some(pack_picture),
        pack_seq: 1,
        ..PackItemDto::new(1)
    };
    let _ = connection
        .pack
        .apply(&item, fc_core::store::SeqRoute::Evidence);

    // ---- A1: a person's picture over the initials, in the list and in the timeline -----------------
    let Some(page) = open(h, shell, &chats, world.media_chat).await else {
        h.check("wiring_open_media_chat", false, "the chat did not open");
        return;
    };
    // Finn's row is new; Eve's was drawn before her picture existed, and follows the new version.
    let pictured = |chat: i64| {
        list.row(chat).is_some_and(|row| {
            find::<adw::Avatar>(&row)
                .iter()
                .any(|avatar| avatar.custom_image().is_some())
        })
    };
    let finn_face = h
        .wait_until(WAIT, || {
            pictured(world.media_chat) && pictured(world.direct)
        })
        .await;
    let family = open(h, shell, &chats, world.family_chat).await;
    let eve_face = match &family {
        Some(family) => {
            h.wait_until(WAIT, || {
                find::<adw::Avatar>(family.timeline().widget())
                    .iter()
                    .any(|avatar| avatar.custom_image().is_some())
            })
            .await
        }
        None => false,
    };
    let asked: Vec<String> = world
        .fake
        .requests()
        .into_iter()
        .filter(|r| r.ends_with("/avatar"))
        .collect();
    h.screenshot("wiring-family-avatars").await;
    h.check(
        "avatar_pictures_in_the_list_and_the_timeline",
        finn_face && eve_face,
        format!("list rows (Finn's new, Eve's redrawn) {finn_face}, timeline {eve_face}; asked {asked:?}"),
    );
    drop(page);

    // ---- A6 + E: the media chat's bubbles from P2's and P3's slots -----------------------------------
    let Some(page) = open(h, shell, &chats, world.media_chat).await else {
        h.check(
            "wiring_open_media_chat",
            false,
            "the chat did not open again",
        );
        return;
    };
    let timeline = page.timeline().widget().clone();
    let mut photo = AttachmentDto::new(PHOTO, "photo");
    photo.width = Some(1600);
    photo.height = Some(1200);
    photo.has_preview = true;
    let drawn = h
        .wait_until(WAIT, || {
            has_class(&timeline, "fc-attach-tile")
                && has_class(&timeline, "fc-media-circle")
                && has_label(&timeline, "Kalemegdan")
                && crate::attachments::pictures::is_kept(&photo, true, None)
        })
        .await;
    let placeholders = find::<gtk::Widget>(&timeline)
        .iter()
        .filter(|w| w.has_css_class("fc-placeholder"))
        .count();
    h.screenshot("wiring-media-chat").await;
    h.check(
        "bubbles_draw_from_the_packages_slots",
        drawn && placeholders == 0,
        format!(
            "tiles {}, circle {}, place {}, photo landed {}; S0 placeholders {placeholders}",
            has_class(&timeline, "fc-attach-tile"),
            has_class(&timeline, "fc-media-circle"),
            has_label(&timeline, "Kalemegdan"),
            crate::attachments::pictures::is_kept(&photo, true, None)
        ),
    );

    // The animation plays in its tile (P3's player in P2's tile): its picture becomes the player's.
    let animated = h
        .wait_until(WAIT, || {
            find::<gtk::Picture>(&timeline).iter().any(|picture| {
                picture.paintable().is_some_and(|p| {
                    let name = p.type_().name();
                    name.contains("Gtk4Paintable") || name.contains("GstPaintable")
                })
            })
        })
        .await;
    h.check(
        "animation_plays_in_its_tile",
        animated,
        "the tile's picture is the player's paintable (gtk4paintablesink)",
    );

    // A circle's expand opens the ROUND viewer (P3 → P2), which plays it.
    if h.input() {
        let clip = find::<gtk::Overlay>(&timeline)
            .into_iter()
            .find(|o| o.has_css_class("fc-round"));
        if let Some(clip) = &clip {
            page.timeline().scroll_to_bottom();
            h.sleep(Duration::from_millis(300)).await;
            h.click_middle(window, clip).await;
        }
        let expand = || {
            find::<gtk::Button>(&timeline)
                .into_iter()
                .find(|b| b.has_css_class("osd") && b.is_visible() && b.is_mapped())
        };
        let shown = h.wait_until(WAIT, || expand().is_some()).await;
        // The pointer away: no "Play" tooltip left over the circle for the later photographs.
        h.xdotool(&["mousemove", "0", "0"]).await;
        if let Some(expand) = expand() {
            expand.emit_clicked();
        }
        let viewer = h
            .wait_until(WAIT, || {
                crate::viewer::find((seeded.round, 0)).is_some_and(|v| {
                    v.album().is_round()
                        && v.playback() == Some(crate::seams::media::PlayerState::Playing)
                })
            })
            .await;
        h.screenshot("wiring-round-viewer").await;
        h.check(
            "circle_expand_opens_the_round_viewer",
            clip.is_some() && shown && viewer,
            format!(
                "circle {}, expand shown {shown}, round viewer playing {viewer}",
                clip.is_some()
            ),
        );
        if let Some(open) = crate::viewer::find((seeded.round, 0)) {
            open.window.window.close();
        }
    } else {
        h.check(
            "circle_expand_opens_the_round_viewer",
            true,
            "skipped: --no-input",
        );
    }

    // ---- A4: "Open Full Screen" — a photo, and a video that plays (P3's real player) -----------------
    act(
        main,
        "win.message-open-full-screen",
        Some(&seeded.photo.to_variant()),
    );
    let photo_shown = h
        .wait_until(WAIT, || {
            crate::viewer::find((seeded.photo, 0)).is_some_and(|v| v.showing_picture())
        })
        .await;
    if let Some(open) = crate::viewer::find((seeded.photo, 0)) {
        open.window.window.close();
    }
    h.check(
        "full_screen_opens_the_viewer_on_a_photo",
        photo_shown,
        format!("message {}", seeded.photo),
    );
    act(
        main,
        "win.message-open-full-screen",
        Some(&seeded.video.to_variant()),
    );
    let playing = h
        .wait_until(WAIT, || {
            crate::viewer::find((seeded.video, 0)).is_some_and(|v| {
                v.showing_video() && v.playback() == Some(crate::seams::media::PlayerState::Playing)
            })
        })
        .await;
    h.sleep(Duration::from_millis(500)).await;
    h.screenshot("wiring-viewer-video").await;
    let streamed = world
        .fake
        .requests()
        .iter()
        .any(|r| r == &format!("GET /api/v1/attachments/{VIDEO}"));
    h.check(
        "full_screen_plays_the_video",
        playing && streamed,
        format!(
            "playing {playing}; streamed from the server {streamed}; state {:?}",
            crate::viewer::find((seeded.video, 0)).and_then(|v| v.playback())
        ),
    );
    if let Some(open) = crate::viewer::find((seeded.video, 0)) {
        open.window.window.close();
    }

    // ---- A4: Save… through P2's saving (the original, where the person chooses) ---------------------
    let before: Vec<gtk::Window> = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Window>().ok())
        .collect();
    act(main, "win.message-save", Some(&seeded.file.to_variant()));
    let chooser = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .find(|w| {
                !before.contains(w) && w.is_visible() && w.type_().name().contains("FileChooser")
            })
    };
    let asked_where = h.wait_until(WAIT, || chooser().is_some()).await;
    let fetched = world
        .fake
        .requests()
        .iter()
        .any(|r| r == &format!("GET /api/v1/attachments/{FILE}"));
    if asked_where {
        h.screenshot("wiring-save").await;
    }
    if let Some(chooser) = chooser() {
        chooser.close();
    }
    h.check(
        "save_asks_where_through_p2",
        asked_where && fetched,
        format!("the original fetched {fetched}; a file chooser came up {asked_where}"),
    );

    // ---- D: Call and Video Call in a direct chat's header -----------------------------------------
    let buttons = page.call_buttons();
    let shown = buttons.len() == 2
        && buttons
            .iter()
            .all(|(button, _)| button.is_visible() && button.is_sensitive());
    let family_buttons = family.as_ref().map_or(0, |f| f.call_buttons().len());
    h.screenshot("wiring-call-buttons").await;
    h.check(
        "call_buttons_in_a_direct_chat",
        shown && family_buttons == 0,
        format!(
            "{} buttons {:?}; the family chat has {family_buttons}",
            buttons.len(),
            buttons
                .iter()
                .map(|(b, video)| (*video, b.is_visible(), b.is_sensitive()))
                .collect::<Vec<_>>()
        ),
    );

    // ---- D + B3 + B4: recording beside words; the buttons follow; parked on leaving; refused -------
    crate::media::use_test_sources(true);
    let composer = Rc::clone(page.composer());
    composer.set_text("for the trip");
    composer.record_voice();
    let recording = || crate::media::recording::installed().is_some_and(|s| s.recording());
    let started = h.wait_until(WAIT, recording).await;
    let insensitive = h
        .wait_until(SHORT, || {
            page.call_buttons()
                .iter()
                .all(|(button, _)| !button.is_sensitive())
        })
        .await;
    h.screenshot("wiring-recording").await;
    h.sleep(Duration::from_millis(1600)).await;
    composer.record_voice();
    let reviewed = h
        .wait_until(WAIT, || {
            !recording()
                && composer
                    .staging()
                    .items()
                    .iter()
                    .any(|item| item.kind == "audio")
        })
        .await;
    let sensitive_again = h
        .wait_until(SHORT, || {
            page.call_buttons()
                .iter()
                .all(|(button, _)| button.is_sensitive())
        })
        .await;
    h.check(
        "call_buttons_insensitive_while_recording",
        started && insensitive && reviewed && sensitive_again,
        format!(
            "recording {started}, insensitive {insensitive}, in review {reviewed}, sensitive after {sensitive_again}"
        ),
    );
    drop(composer);
    let media_chat = world.media_chat;
    let group = open(h, shell, &chats, world.group_chat).await;
    let store = crate::media::recording::installed().and_then(|slots| slots.store());
    let parked = store
        .as_ref()
        .map(|store| store.of(media_chat))
        .unwrap_or_default();
    h.check(
        "note_in_review_parked_when_the_chat_is_left",
        parked.len() == 1 && parked[0].caption.as_deref() == Some("for the trip"),
        format!(
            "{} parked; captions {:?}",
            parked.len(),
            parked.iter().map(|p| p.caption.clone()).collect::<Vec<_>>()
        ),
    );

    // ---- C: Group Info from the group's header (P4's dialog) ----------------------------------------
    let info = find::<gtk::Button>(&main.content_header())
        .into_iter()
        .find(|b| b.has_css_class("fc-chat-group-info") && b.is_visible());
    let clicked = match &info {
        Some(info) if h.input() => h.click_middle(window, info).await,
        Some(info) => {
            info.emit_clicked();
            true
        }
        None => false,
    };
    let dialog = h
        .wait_until(WAIT, || {
            main.window()
                .visible_dialog()
                .is_some_and(|d| d.title() == shell.say.get("Group Info"))
        })
        .await;
    h.sleep(Duration::from_millis(500)).await;
    h.screenshot("wiring-group-info").await;
    h.check(
        "group_info_opens_from_the_header",
        group.is_some() && clicked && dialog,
        format!(
            "button {}, clicked {clicked}, dialog {dialog}",
            info.is_some()
        ),
    );
    if let Some(dialog) = main.window().visible_dialog() {
        dialog.close();
    }
    let _ = h
        .wait_until(SHORT, || main.window().visible_dialog().is_none())
        .await;
    drop(group);

    // Back in the media chat: its not-sent note waits, and P3's refusal is said on P1's line.
    let Some(page) = open(h, shell, &chats, world.media_chat).await else {
        h.check(
            "wiring_open_media_chat",
            false,
            "the chat did not open a third time",
        );
        return;
    };
    let composer = Rc::clone(page.composer());
    let words_taken = composer.draft().is_empty();
    composer.record_voice();
    let expected =
        fc_logic::voice_notes::refusal(false, false, false, true, false, shell.say.as_ref());
    let said = h
        .wait_until(SHORT, || composer.notice_text() == expected)
        .await;
    h.screenshot("wiring-not-sent").await;
    h.check(
        "recording_refusal_said_on_the_composer_line",
        said && words_taken && !recording(),
        format!(
            "notice {:?} (want {expected:?}); the caption left the field {words_taken}",
            composer.notice_text()
        ),
    );
    if let Some(store) = &store {
        for entry in store.of(media_chat) {
            store.remove(&entry);
        }
    }
    if let Some(slots) = crate::media::recording::installed() {
        for voice in slots.composers() {
            voice.refresh_rows();
        }
    }
    composer.refresh();
    crate::media::use_test_sources(false);

    // ---- A2: the sticker panel from the composer; a pick sends a sticker ---------------------------
    let glyph = composer.sticker_glyph().clone();
    let sends_before = world.sends.borrow().len();
    let offered = glyph.is_visible();
    if h.input() && glyph.is_mapped() {
        h.click_middle(window, &glyph).await;
    } else {
        glyph.emit_clicked();
    }
    let popover = || {
        find::<gtk::Popover>(&glyph)
            .into_iter()
            .find(|p| p.is_visible())
    };
    let opened = h.wait_until(SHORT, || popover().is_some()).await;
    h.sleep(Duration::from_millis(400)).await;
    h.screenshot("wiring-sticker-panel").await;
    // The pointer away again: no tooltip left standing over the later photographs.
    h.xdotool(&["mousemove", "0", "0"]).await;
    let cells = popover()
        .map(|p| find::<gtk::Button>(&p))
        .unwrap_or_default();
    if let Some(cell) = cells.first() {
        cell.emit_clicked();
    }
    let sticker_sent = h
        .wait_until(WAIT, || {
            world.sends.borrow().iter().skip(sends_before).any(|s| {
                s["sticker"].as_bool() == Some(true)
                    && s["attachment_ids"].as_array().is_some_and(|a| a.len() == 1)
            })
        })
        .await;
    h.check(
        "sticker_panel_opens_from_the_composer_and_a_pick_sends",
        offered && opened && !cells.is_empty() && sticker_sent,
        format!(
            "glyph {offered}, panel {opened}, {} cell(s), sent {sticker_sent}",
            cells.len()
        ),
    );

    // ---- A3: Location — no portal here says so; a place found is sent ------------------------------
    composer.press_location();
    let refused = h
        .wait_until(Duration::from_secs(30), || {
            composer.notice_text().as_deref()
                == Some(shell.say.get("Could not find your location."))
        })
        .await;
    h.check(
        "location_without_a_portal_says_so",
        refused,
        format!("{:?}", composer.notice_text()),
    );
    let sends_before = world.sends.borrow().len();
    composer.send_place(&LocationFix {
        latitude: 44.8231,
        longitude: 20.4504,
        accuracy_m: Some(12.0),
        taken: time::OffsetDateTime::now_utc(),
    });
    let uploads = world.uploads.clone();
    let place_sent = h
        .wait_until(WAIT, || {
            world.sends.borrow().iter().skip(sends_before).any(|s| {
                s["attachment_ids"].as_array().is_some_and(|ids| {
                    ids.iter().any(|id| {
                        id.as_i64()
                            .and_then(|id| uploads.borrow().get(&id).cloned())
                            .is_some_and(|a| a.kind == "location")
                    })
                })
            })
        })
        .await;
    h.check(
        "location_sent_as_a_message",
        place_sent,
        "staged, uploaded as kind=location, sent with its id",
    );

    // ---- A5: a file handed to the app lands staged in the chosen chat (`stage_files`) ----------------
    let handed_dir = h.out().join("wiring-handed");
    let _ = std::fs::create_dir_all(&handed_dir);
    let handed = handed_dir.join("photo two.jpg");
    let _ = std::fs::write(&handed, picture(640, 480, 0x8d6e_63ff, "jpeg"));
    composer.staging().take_all();
    composer.refresh();
    shell.inbox.get().receive(&[gio::File::for_path(&handed)]);
    let inbox = crate::attachments::inbox::Inbox::current();
    let asked = h
        .wait_until(WAIT, || inbox.as_ref().and_then(|i| i.choosing()).is_some())
        .await;
    if let Some(choosing) = inbox.as_ref().and_then(|i| i.choosing())
        && let Some((_, row)) = choosing.rows.iter().find(|(id, _)| *id == media_chat)
    {
        row.emit_by_name::<()>("activated", &[]);
    }
    let staged = h
        .wait_until(WAIT, || {
            chats
                .page()
                .is_some_and(|p| p.chat_id == media_chat && p.composer().staging().len() == 1)
        })
        .await;
    h.sleep(Duration::from_millis(300)).await;
    let left = inbox
        .as_ref()
        .and_then(|i| std::fs::read_dir(i.root()).ok())
        .map_or(0, |entries| entries.count());
    h.screenshot("wiring-inbox-staged").await;
    h.check(
        "inbox_file_staged_in_the_chosen_chat",
        asked && staged && left == 0 && handed.exists(),
        format!("Send to {asked}; staged {staged}; copies left {left}"),
    );
    let page = chats.page().unwrap_or(page);
    let composer = Rc::clone(page.composer());
    composer.staging().take_all();
    composer.refresh();

    // ---- B1: a picked video prepared by P3 before it is staged --------------------------------------
    composer.stage_files(vec![gio::File::for_path(&clip)]);
    let prepared = h
        .wait_until(Duration::from_secs(60), || composer.staging().len() == 1)
        .await;
    let item = composer.staging().items().first().cloned();
    h.check(
        "picked_video_prepared_by_p3_before_staging",
        prepared
            && item.as_ref().is_some_and(|item| {
                item.kind == "video"
                    && item.mime == "video/mp4"
                    && item.preview.is_some()
                    && fc_logic::faststart::is_moov_first(&item.bytes) == Some(true)
            }),
        format!(
            "{:?}",
            item.as_ref().map(|i| (
                i.kind.clone(),
                i.mime.clone(),
                i.width,
                i.height,
                i.preview.as_ref().map(Vec::len)
            ))
        ),
    );
    composer.staging().take_all();
    composer.refresh();

    // ---- B4: a video message travels round, with the reply primed ----------------------------------
    let host = composer.host();
    composer.set_reply(Some(seeded.photo));
    let reply_seen = crate::seams::composer::ComposerHost::reply_to(host.as_ref());
    let sends_before = world.sends.borrow().len();
    crate::seams::composer::ComposerHost::send_video_message(
        host.as_ref(),
        StagedMedia {
            width: Some(480),
            height: Some(480),
            duration_ms: Some(2000),
            preview: Some(picture(480, 480, 0xef6c_00ff, "jpeg")),
            ..StagedMedia::new("video", "video/mp4", square_bytes.clone())
        },
    );
    let round_sent = h
        .wait_until(WAIT, || {
            world.sends.borrow().iter().skip(sends_before).any(|s| {
                s["round"].as_bool() == Some(true)
                    && s["reply_to_message_id"].as_i64() == Some(seeded.photo)
            })
        })
        .await;
    composer.set_reply(Some(seeded.photo));
    crate::seams::composer::ComposerHost::release_reply(host.as_ref());
    h.check(
        "video_message_sent_round_with_its_reply",
        reply_seen == Some(seeded.photo) && round_sent && composer.reply().is_none(),
        format!("reply seen {reply_seen:?}; sent round {round_sent}"),
    );

    // ---- D: a record's "Call back" places a call (P6's engine) --------------------------------------
    page.timeline().scroll_to_bottom();
    let back = find::<gtk::Button>(&timeline)
        .into_iter()
        .find(|b| b.has_css_class("fc-chat-call-back"));
    let ready = back.as_ref().is_some_and(|b| b.is_sensitive());
    if let Some(back) = &back {
        back.emit_clicked();
    }
    let placed = h
        .wait_until(SHORT, || {
            connection
                .calls
                .call()
                .is_some_and(|call| call.chat_id == media_chat && call.outgoing)
        })
        .await;
    h.check(
        "call_back_places_a_call",
        ready && placed,
        format!(
            "button {}, sensitive {ready}; call {:?}",
            back.is_some(),
            connection.calls.call().map(|c| (c.chat_id, c.stage))
        ),
    );
    // No microphone here: the call ends by itself, says why for a moment, and its window goes.
    let _ = h
        .wait_until(WAIT, || connection.calls.call().is_none())
        .await;
    if let Some(window) = crate::calls::hub::get().and_then(|calls| calls.window()) {
        window.hide();
    }

    // ---- the conversation as integrated: normal, narrow, dark ---------------------------------------
    let Some(page) = open(h, shell, &chats, world.media_chat).await else {
        return;
    };
    page.timeline().scroll_to_bottom();
    h.sleep(Duration::from_millis(800)).await;
    h.screenshot("wiring-conversation").await;
    let placed = page.timeline().placement();
    h.check(
        "conversation_drawn_at_the_newest",
        newest_drawn_at_the_bottom(placed),
        format!("value, upper, page, the newest row drawn at {placed:?}"),
    );
    if h.input() {
        h.resize(window, 560, 700).await;
        shell.chat_routes.get().open_chat(world.media_chat);
        h.sleep(Duration::from_millis(800)).await;
        h.screenshot("wiring-conversation-narrow").await;
        h.resize(window, DEFAULT_SIZE.0, DEFAULT_SIZE.1).await;
        let _ = h
            .wait_until(SHORT, || !main.split_view().is_collapsed())
            .await;
    }
    let styles = adw::StyleManager::default();
    styles.set_color_scheme(adw::ColorScheme::ForceDark);
    let _ = h.wait_until(SHORT, crate::theme::is_dark).await;
    // The theme change draws every row again (`Timeline::clear`): let it settle at the newest.
    h.sleep(Duration::from_millis(2500)).await;
    h.screenshot("wiring-conversation-dark").await;
    let placed = page.timeline().placement();
    h.check(
        "dark_redraw_stays_at_the_newest",
        newest_drawn_at_the_bottom(placed),
        format!("value, upper, page, the newest row drawn at {placed:?}"),
    );
    styles.set_color_scheme(adw::ColorScheme::Default);
    let _ = h.wait_until(SHORT, || !crate::theme::is_dark()).await;

    // What this pass turned on goes off again for the groups after it.
    let mut state = world.fake.state();
    state.calls = false;
    state.video_calls = false;
    state.pack_limits = None;
}

/// The adjustment is at the bottom AND the newest row is DRAWN there (its bottom within the view,
/// at most a few margins above the view's bottom): the adjustment alone once said the newest while
/// the rows sat at an old offset.
fn newest_drawn_at_the_bottom(
    (value, upper, page, newest): (f64, f64, f64, Option<(f32, f32)>),
) -> bool {
    let scrolled = value + page >= upper - 1.0;
    let drawn = newest.is_some_and(|(_, bottom)| {
        let bottom = f64::from(bottom);
        bottom <= page + 1.0 && bottom >= page - 40.0
    });
    scrolled && drawn
}

/// Opens `chat_id` in the main window and waits for its first rows.
async fn open(
    h: &Harness,
    shell: &Rc<Shell>,
    chats: &Rc<chats::Chats>,
    chat_id: i64,
) -> Option<Rc<ConversationPage>> {
    shell.chat_routes.get().open_chat(chat_id);
    let opened = h
        .wait_until(WAIT, || {
            chats.page().is_some_and(|p| {
                p.chat_id == chat_id && p.timeline().keys().iter().any(|k| k.starts_with('m'))
            })
        })
        .await;
    if !opened {
        return None;
    }
    chats.page()
}

/// The media chat's messages (oldest first: a file, a place, a sticker, a photo, a call record, a
/// video, an animation, a video message) and a word in the group.
fn seed(world: &Rc<World>, animation_size: usize) -> Seeded {
    let chat = world.media_chat;
    let finn = world.finn;
    let carrying = |attachment: AttachmentDto| {
        let mut message = MessageDto::new(0, chat, finn, None, "", &now_rfc3339());
        message.attachments = Some(vec![attachment]);
        world.insert(message)
    };
    let mut file = AttachmentDto::new(FILE, "file");
    file.name = Some("Trip plan.pdf".into());
    file.size = Some(17);
    file.mime = Some("application/pdf".into());
    let file = carrying(file);
    let mut place = AttachmentDto::new(PLACE, "location");
    place.name = Some("Kalemegdan".into());
    place.latitude = Some(44.8231);
    place.longitude = Some(20.4504);
    place.accuracy_m = Some(12.0);
    carrying(place);
    let mut sticker = AttachmentDto::new(STICKER, "photo");
    sticker.sticker = true;
    sticker.width = Some(300);
    sticker.height = Some(300);
    sticker.mime = Some("image/png".into());
    carrying(sticker);
    let mut photo = AttachmentDto::new(PHOTO, "photo");
    photo.width = Some(1600);
    photo.height = Some(1200);
    photo.has_preview = true;
    photo.mime = Some("image/jpeg".into());
    let photo = carrying(photo);
    let mut record = MessageDto::new(0, chat, finn, None, "", &now_rfc3339());
    record.call = Some(CallRecordDto {
        outcome: "missed".into(),
        duration_secs: None,
        video: false,
    });
    world.insert(record);
    let mut video = AttachmentDto::new(VIDEO, "video");
    video.width = Some(640);
    video.height = Some(360);
    video.duration_ms = Some(2000);
    video.has_preview = true;
    video.mime = Some("video/mp4".into());
    let video = carrying(video);
    let mut moving = AttachmentDto::new(ANIMATION, "video");
    moving.animation = true;
    moving.width = Some(320);
    moving.height = Some(240);
    moving.duration_ms = Some(2000);
    moving.has_preview = true;
    moving.size = i64::try_from(animation_size).ok();
    moving.mime = Some("video/mp4".into());
    carrying(moving);
    let mut round = AttachmentDto::new(ROUND, "video");
    round.round = true;
    round.width = Some(480);
    round.height = Some(480);
    round.duration_ms = Some(2000);
    round.has_preview = true;
    round.mime = Some("video/mp4".into());
    let round = carrying(round);
    world.insert(MessageDto::new(
        0,
        world.group_chat,
        world.eve,
        None,
        "Who brings the tent?",
        &now_rfc3339(),
    ));
    Seeded {
        photo,
        file,
        video,
        round,
    }
}

/// This pass's routes: the attachments it serves, the avatars, and uploads taken in.
fn routes(
    fake: &Rc<FakeServer>,
    world: &Rc<World>,
    blobs: &Rc<std::cell::RefCell<std::collections::HashMap<String, Vec<u8>>>>,
) {
    let served = Rc::clone(blobs);
    fake.route(
        "GET",
        "/api/v1/attachments/**",
        Rc::new(move |_, request| {
            let key = request.segments[3..].join("/");
            match served.borrow().get(&key) {
                Some(bytes) => FakeResponse {
                    status: 200,
                    body: bytes.clone(),
                },
                None => FakeResponse::error(404, "attachment_not_found"),
            }
        }),
    );
    let face = picture(256, 256, 0xfb8c_00ff, "jpeg");
    fake.route(
        "GET",
        "/api/v1/users/*/avatar",
        Rc::new(move |_, _| FakeResponse {
            status: 200,
            body: face.clone(),
        }),
    );
    let uploads = Rc::clone(&world.uploads);
    let next = std::cell::Cell::new(FIRST_UPLOAD);
    fake.route(
        "POST",
        "/api/v1/attachments",
        Rc::new(move |_, request| {
            let query = query_of(request);
            let id = next.get() + 1;
            next.set(id);
            let kind = query.get("kind").cloned().unwrap_or_default();
            let mut attachment = AttachmentDto::new(id, &kind);
            attachment.width = query.get("width").and_then(|w| w.parse().ok());
            attachment.height = query.get("height").and_then(|h| h.parse().ok());
            attachment.duration_ms = query.get("duration_ms").and_then(|d| d.parse().ok());
            attachment.latitude = query.get("latitude").and_then(|l| l.parse().ok());
            attachment.longitude = query.get("longitude").and_then(|l| l.parse().ok());
            attachment.size = i64::try_from(request.body.len()).ok();
            uploads.borrow_mut().insert(id, attachment.clone());
            FakeResponse::json(201, &AttachmentResponse { attachment })
        }),
    );
    // A photo's small copy and a video's poster, made on the device (docs/protocol.md).
    fake.route(
        "PUT",
        "/api/v1/attachments/*/preview",
        Rc::new(|_, _| FakeResponse::empty(204)),
    );
}

/// Runs each description with `gst-launch-1.0 -q -e`; true when all succeeded.
async fn make(h: &Harness, descriptions: &[&str]) -> bool {
    for description in descriptions {
        let mut argv = vec!["gst-launch-1.0", "-q", "-e"];
        argv.extend(description.split_whitespace());
        let (ok, output) = h.run(&argv).await;
        if !ok {
            eprintln!("selftest: gst-launch failed: {output}");
            return false;
        }
    }
    true
}

/// A picture of one colour with a white square in its middle, as `format` (an opaque pixbuf: a
/// JPEG cannot hold alpha — measured, P1/P2).
fn picture(width: i32, height: i32, colour: u32, format: &str) -> Vec<u8> {
    let Some(pixbuf) =
        gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, width, height)
    else {
        return Vec::new();
    };
    pixbuf.fill(colour);
    let side = width.min(height) / 3;
    pixbuf
        .new_subpixbuf((width - side) / 2, (height - side) / 2, side, side)
        .fill(0xffff_ffff);
    pixbuf.save_to_bufferv(format, &[]).unwrap_or_default()
}

fn has_class(root: &impl IsA<gtk::Widget>, class: &str) -> bool {
    find::<gtk::Widget>(root)
        .iter()
        .any(|widget| widget.has_css_class(class))
}

fn has_label(root: &impl IsA<gtk::Widget>, text: &str) -> bool {
    find::<gtk::Label>(root)
        .iter()
        .any(|label| label.label() == text)
}
