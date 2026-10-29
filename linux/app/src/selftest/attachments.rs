//! The `attachments` group (P2, shell-design §27, §8): tiles come back at their final size before
//! their bytes arrive and the picture lands without moving them; the grid of four counts the rest;
//! a file row says its size; the viewer pages with → and ←, stops at the ends and goes with Escape;
//! a sticker is drawn without a balloon; an animated sticker moves while it is on screen and stops
//! when it is scrolled away; a link card appears only while "Link Previews" is on (and nothing is
//! fetched while it is off); a location card draws its map only while "Map Previews" is on; an
//! avatar gets its picture; the sticker panel picks; a file handed to the app reaches "Send to"
//! (never the assistant's chat) and is staged in the chosen chat, its copy gone afterwards.
//!
//! P1's conversation page is not in this package, so the slot widgets are hosted in a small test
//! window of this group's own. Pictures come from this group's routes on the fake server; map tiles
//! from a second loopback server of its own (OpenStreetMap is never asked); link previews from
//! memory through fc-logic's real `LinkPreviews` (its https rule cannot be met on loopback).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::{
    AttachmentDto, ChatDto, ChatRowDto, MessageDto, PackItemDto, PackResponse, ServerUrl,
};
use fc_core::rt::{Http, HttpRequest, HttpResponse, LocalBoxFuture, TransportError};
use fc_logic::app_session::Gate;
use fc_logic::link_previews::LinkPreviews;
use fc_logic::media_album::MediaAlbum;
use gtk::{gdk, gdk_pixbuf};

use super::Harness;
use super::fake_server::FakeResponse;
use crate::attachments::{self, AttachmentSlots, inbox::Inbox, map_tiles::MapTiles, pictures};
use crate::seams::conversation::{ConversationSlots, TileLook};
use crate::seams::routes::ChatRoutes;
use crate::services::settings_store::keys;
use crate::shell::Shell;
use crate::windows::main_window::GatePage;

const WAIT: Duration = Duration::from_secs(15);
const PASSWORD: &str = "password";

/// Ids this group's world uses (far from the `shell` group's).
const FAMILY_CHAT: i64 = 9001;
const DIRECT_CHAT: i64 = 9002;
const ASSISTANT_CHAT: i64 = 9003;

pub async fn run(h: &Harness) {
    let shell = h.shell();
    let fake = h.fake_server();
    let dana = fake.add_user("p2-dana", PASSWORD, "Dana");
    let eli = fake.add_user("p2-eli", PASSWORD, "Eli");
    let family = fake.add_family("The Pictures", dana);
    fake.join(eli, family, "member");
    let mut family_chat = ChatDto::new(FAMILY_CHAT, "family", "The Pictures");
    family_chat.family_ids = Some(vec![family]);
    let mut direct = ChatDto::new(DIRECT_CHAT, "direct", "Eli");
    direct.peer_user_id = Some(eli);
    direct.family_ids = Some(vec![family]);
    let mut assistant = ChatDto::new(ASSISTANT_CHAT, "ai", "Assistant");
    assistant.family_ids = Some(vec![family]);
    fake.set_chats(
        family,
        vec![
            ChatRowDto::new(family_chat),
            ChatRowDto::new(direct),
            ChatRowDto::new(assistant),
        ],
    );

    // ---- the pictures this world serves ------------------------------------------------------
    let blobs: Rc<RefCell<HashMap<String, Vec<u8>>>> = Rc::new(RefCell::new(HashMap::new()));
    let preview_bytes = still(600, 450, 0x3d7a_c6ff, 0xf2c0_44ff, false, "jpeg");
    blobs
        .borrow_mut()
        .insert("7001/preview".into(), preview_bytes.clone());
    for id in 7011..=7016 {
        let colour = 0x2060_a0ff + (id as u32 - 7011) * 0x1810_0000;
        blobs.borrow_mut().insert(
            format!("{id}/preview"),
            still(300, 300, colour, 0xffff_ffff, false, "jpeg"),
        );
        blobs.borrow_mut().insert(
            id.to_string(),
            still(1200, 1200, colour, 0x2020_20ff, false, "jpeg"),
        );
    }
    blobs.borrow_mut().insert(
        "7001".into(),
        still(1600, 1200, 0x3d7a_c6ff, 0xf2c0_44ff, false, "jpeg"),
    );
    blobs.borrow_mut().insert(
        "7101".into(),
        still(300, 300, 0xe539_35ff, 0x0000_0000, true, "png"),
    );
    let animated = animated_gif(h).await;
    if let Some(gif) = &animated {
        blobs.borrow_mut().insert("7102".into(), gif.clone());
    }
    blobs.borrow_mut().insert(
        "7201".into(),
        still(200, 200, 0x8e24_aaff, 0xffff_ffff, true, "png"),
    );
    blobs.borrow_mut().insert(
        "7202".into(),
        still(200, 120, 0x0097_a7ff, 0x0000_0000, true, "png"),
    );
    blobs
        .borrow_mut()
        .insert("7301".into(), b"%PDF-1.4 a test".to_vec());
    let served = Rc::clone(&blobs);
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
    let face = still(256, 256, 0xfb8c_00ff, 0xffe0_b2ff, false, "jpeg");
    fake.route(
        "GET",
        "/api/v1/users/*/avatar",
        Rc::new(move |_, _| FakeResponse {
            status: 200,
            body: face.clone(),
        }),
    );
    let pack_items = vec![
        pack_item(1, 7201, 200, 200, dana),
        pack_item(2, 7202, 200, 120, eli),
    ];
    let pack_answer = pack_items.clone();
    fake.route(
        "GET",
        "/api/v1/families/*/pack",
        Rc::new(move |_, _| {
            FakeResponse::json(
                200,
                &PackResponse {
                    items: Some(pack_answer.clone()),
                    max_pack_seq: 2,
                },
            )
        }),
    );

    // ---- signed in as Dana, on the chats -------------------------------------------------------
    if !sign_in(h, &fake.url(), "p2-dana").await {
        return;
    }
    let Some(connection) = shell.connection() else {
        return;
    };
    // The pack this world holds, as the catch-up would put it (the fake names no pack sequence).
    for item in &pack_items {
        let _ = connection
            .pack
            .apply(item, fc_core::store::SeqRoute::Evidence);
    }
    let slots = shell.slots();

    // The test window that hosts the slot widgets.
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(16)
        .margin_end(16)
        .margin_top(16)
        .margin_bottom(16)
        .build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&column)
        .build();
    let window = gtk::Window::builder()
        .application(&shell.app)
        .title("P2 attachments")
        .default_width(1000)
        .default_height(760)
        .child(&scrolled)
        .build();
    window.present();

    // ---- tiles ---------------------------------------------------------------------------------
    let mut photo = AttachmentDto::new(7001, "photo");
    photo.width = Some(1600);
    photo.height = Some(1200);
    photo.has_preview = true;
    let mut message = MessageDto::new(8001, DIRECT_CHAT, eli, None, "", "2026-10-29T10:00:00Z");
    message.attachments = Some(vec![photo.clone()]);
    let (width, height) = fc_text::media::tile_size(Some(1600), Some(1200));
    let look = TileLook {
        width,
        height,
        own: false,
        more: 0,
    };
    let held_before = pictures::is_kept(&photo, true, None);
    let tile = slots.tile(&message, &photo, &look);
    let requested = tile.size_request();
    h.check(
        "tile_final_size_before_bytes",
        requested == (320, 240) && !held_before,
        format!("asked {requested:?} at once (want (320, 240)); held before: {held_before}"),
    );
    let mut video = AttachmentDto::new(7002, "video");
    video.width = Some(1920);
    video.height = Some(1080);
    video.duration_ms = Some(23_000);
    let (vw, vh) = fc_text::media::tile_size(Some(1920), Some(1080));
    let video_tile = slots.tile(
        &message,
        &video,
        &TileLook {
            width: vw,
            height: vh,
            own: false,
            more: 0,
        },
    );
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.append(&tile);
    row.append(&video_tile);
    column.append(&row);
    let landed = h
        .wait_until(WAIT, || pictures::is_kept(&photo, true, None))
        .await;
    h.sleep(Duration::from_millis(300)).await;
    let requests = fake.requests();
    h.check(
        "picture_lands_without_moving",
        landed && tile.width() == 320 && tile.height() == 240,
        format!(
            "landed {landed}; drawn {} × {} after landing",
            tile.width(),
            tile.height()
        ),
    );
    h.check(
        "tile_asks_the_preview_never_the_original_or_a_whole_video",
        requests
            .iter()
            .any(|r| r == "GET /api/v1/attachments/7001/preview")
            && !requests.iter().any(|r| r == "GET /api/v1/attachments/7001")
            && !requests.iter().any(|r| r.contains("/attachments/7002")),
        format!(
            "{:?}",
            requests
                .iter()
                .filter(|r| r.contains("/attachments/"))
                .collect::<Vec<_>>()
        ),
    );

    // The grid of four, "+2" over the last, and a file row.
    let album: Vec<AttachmentDto> = (7011..=7016)
        .map(|id| {
            let mut item = AttachmentDto::new(id, "photo");
            item.width = Some(1200);
            item.height = Some(1200);
            item.has_preview = true;
            item
        })
        .collect();
    let mut file = AttachmentDto::new(7301, "file");
    file.name = Some("Family trip plan, final version 2026 (signed).pdf".into());
    file.size = Some(1_234_567);
    file.mime = Some("application/pdf".into());
    let mut grid_message =
        MessageDto::new(8002, FAMILY_CHAT, dana, None, "", "2026-10-29T10:01:00Z");
    let mut carried = album.clone();
    carried.push(file.clone());
    grid_message.attachments = Some(carried);
    let own_bubble = gtk::Box::builder()
        .css_classes(["fc-own-bubble", "card"])
        .halign(gtk::Align::Start)
        .build();
    let block = attachments::tiles::media_block(&grid_message, true);
    block.set_margin_start(8);
    block.set_margin_end(8);
    block.set_margin_top(8);
    block.set_margin_bottom(8);
    own_bubble.append(&block);
    column.append(&own_bubble);
    h.check(
        "grid_of_four_counts_the_rest",
        count::<gtk::Grid>(&block) == 1 && has_label(&block, "+2"),
        format!(
            "grids {}, \"+2\" shown {}",
            count::<gtk::Grid>(&block),
            has_label(&block, "+2")
        ),
    );
    h.check(
        "file_row_says_its_size",
        has_label(&block, "1.2 MB") && has_label_starting(&block, "Family trip plan"),
        "a file row with its size in decimal units and its name middle-truncated",
    );
    h.wait_until(WAIT, || pictures::is_kept(&album[3], true, None))
        .await;
    h.screenshot("tiles").await;

    // ---- the viewer ----------------------------------------------------------------------------
    let mut paged = MessageDto::new(8003, FAMILY_CHAT, eli, None, "", "2026-10-29T10:02:00Z");
    paged.attachments = Some(album[..3].to_vec());
    crate::viewer::open_message(&paged, 0);
    let viewer = crate::viewer::find((8003, 0));
    let opened = h
        .wait_until(WAIT, || {
            viewer
                .as_ref()
                .is_some_and(|v| v.window.window.is_visible() && v.showing_picture())
        })
        .await;
    let Some(viewer) = viewer else {
        h.check("viewer_opens", false, "no viewer window");
        return;
    };
    let title = |viewer: &crate::viewer::Viewer| viewer.window.title.title().to_string();
    h.check(
        "viewer_opens_on_the_album",
        opened && title(&viewer) == "1 of 3" && !viewer.window.previous.is_sensitive(),
        format!("visible+picture {opened}, title {:?}", title(&viewer)),
    );
    h.screenshot("viewer").await;
    let viewer_window: gtk::Window = viewer.window.window.clone().upcast();
    let keyed = h.input() && h.focus(&viewer_window).await;
    let press = |key: &'static str| {
        let viewer = Rc::clone(&viewer);
        async move {
            if keyed {
                h.key(key).await;
            } else {
                match key {
                    "Right" => viewer.step(1),
                    "Left" => viewer.step(-1),
                    _ => viewer.window.window.close(),
                }
            }
            h.sleep(Duration::from_millis(250)).await;
        }
    };
    press("Right").await;
    let second = (viewer.album().index(), title(&viewer));
    press("Right").await;
    press("Right").await;
    let at_end = (viewer.album().index(), viewer.window.next.is_sensitive());
    press("Left").await;
    let back = viewer.album().index();
    h.check(
        "viewer_pages_and_stops_at_the_ends",
        second == (1, "2 of 3".to_string()) && at_end == (2, false) && back == 1,
        format!("after → {second:?}; at the end {at_end:?}; after ← {back} (real keys: {keyed})"),
    );
    let zoom_before = viewer.album().zoom();
    viewer.zoom_with(MediaAlbum::zoom_in);
    let zoomed = viewer.album().zoom();
    viewer.step(1);
    h.check(
        "a_page_turn_starts_the_next_picture_at_fit",
        zoom_before == 1.0 && zoomed > 1.0 && viewer.album().zoom() == 1.0,
        format!(
            "{zoom_before} → {zoomed} → {} after the turn",
            viewer.album().zoom()
        ),
    );
    press("Escape").await;
    let closed = h
        .wait_until(Duration::from_secs(3), || {
            crate::viewer::find((8003, 0)).is_none()
        })
        .await;
    h.check(
        "escape_closes_the_viewer",
        closed && !viewer.window.window.is_visible(),
        format!("closed {closed} (real keys: {keyed})"),
    );
    drop(viewer);

    // ---- stickers ------------------------------------------------------------------------------
    let mut sticker = AttachmentDto::new(7101, "photo");
    sticker.sticker = true;
    sticker.width = Some(300);
    sticker.height = Some(300);
    sticker.mime = Some("image/png".into());
    let mut sticker_message =
        MessageDto::new(8004, FAMILY_CHAT, eli, None, "", "2026-10-29T10:03:00Z");
    sticker_message.attachments = Some(vec![sticker.clone()]);
    let sticker_widget = slots.sticker(&sticker_message, &sticker);
    let sticker_size = sticker_widget.size_request();
    let stickers_row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    stickers_row.append(&sticker_widget);
    column.append(&stickers_row);
    let sticker_drawn = h
        .wait_until(WAIT, || {
            sticker_widget
                .downcast_ref::<gtk::Picture>()
                .and_then(|p| p.paintable())
                .is_some_and(|p| p.intrinsic_width() == 160 && !is_empty_paintable(&p))
        })
        .await;
    h.check(
        "sticker_drawn_without_a_balloon_in_its_box",
        sticker_size == (160, 160) && sticker_drawn && !sticker_widget.has_css_class("card"),
        format!("box {sticker_size:?}, drawn {sticker_drawn}"),
    );

    // An animated one, moving while on screen and still when scrolled away.
    let mut moving = AttachmentDto::new(7102, "photo");
    moving.sticker = true;
    moving.width = Some(96);
    moving.height = Some(96);
    moving.mime = Some("image/gif".into());
    let mut moving_message =
        MessageDto::new(8005, FAMILY_CHAT, eli, None, "", "2026-10-29T10:04:00Z");
    moving_message.attachments = Some(vec![moving.clone()]);
    let moving_widget = slots.sticker(&moving_message, &moving);
    let shelf_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shelf_box.append(&moving_widget);
    shelf_box.append(&gtk::Box::builder().height_request(1600).build());
    let small_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(200)
        .max_content_height(200)
        .min_content_width(200)
        .child(&shelf_box)
        .build();
    stickers_row.append(&small_scroll);
    if animated.is_none() {
        h.check(
            "animated_sticker_moves_then_pauses_when_scrolled_away",
            false,
            "no animated GIF could be made with ImageMagick",
        );
    } else {
        let started = h
            .wait_until(WAIT, || {
                attachments::stickers::progress(&moving_widget).is_some_and(|(n, m)| m && n >= 2)
            })
            .await;
        if let Some(point) =
            stickers_row.compute_point(&column, &gtk::graphene::Point::new(0.0, 0.0))
        {
            scrolled
                .vadjustment()
                .set_value(f64::from(point.y()) - 16.0);
        }
        h.screenshot("stickers").await;
        let on_screen = attachments::stickers::progress(&moving_widget).map_or(0, |p| p.0);
        h.sleep(Duration::from_millis(800)).await;
        let later = attachments::stickers::progress(&moving_widget).map_or(0, |p| p.0);
        let adjustment = small_scroll.vadjustment();
        adjustment.set_value(adjustment.upper());
        h.sleep(Duration::from_millis(400)).await;
        let scrolled_away = attachments::stickers::progress(&moving_widget).map_or(0, |p| p.0);
        h.sleep(Duration::from_millis(1000)).await;
        let still_away = attachments::stickers::progress(&moving_widget).map_or(0, |p| p.0);
        adjustment.set_value(0.0);
        h.sleep(Duration::from_millis(800)).await;
        let back = attachments::stickers::progress(&moving_widget).map_or(0, |p| p.0);
        h.check(
            "animated_sticker_moves_then_pauses_when_scrolled_away",
            started && later > on_screen && still_away == scrolled_away && back > still_away,
            format!(
                "frames: {on_screen} → {later} on screen; {scrolled_away} → {still_away} scrolled away; {back} back"
            ),
        );
    }

    // ---- the sticker panel ---------------------------------------------------------------------
    let picked: Rc<Cell<Option<i64>>> = Rc::new(Cell::new(None));
    let panel = {
        let picked = Rc::clone(&picked);
        attachments::sticker_panel::panel(Rc::new(move |item: PackItemDto| {
            picked.set(Some(item.id))
        }))
    };
    let buttons = descendants::<gtk::Button>(&panel);
    let panel_frame = gtk::Frame::builder().child(&panel).build();
    stickers_row.append(&panel_frame);
    if let Some(second) = buttons.get(1) {
        second.emit_clicked();
    }
    h.check(
        "sticker_panel_lists_the_pack_and_picks",
        buttons.len() == pack_items.len() && picked.get() == Some(2),
        format!("{} cells, picked {:?}", buttons.len(), picked.get()),
    );

    // ---- the pack ("Family Stickers", which P4's Family dialog opens) ---------------------------
    let pack = attachments::pack::PackDialog::new(Rc::clone(&connection));
    pack.dialog.present(Some(&window));
    h.sleep(Duration::from_millis(700)).await;
    let removable = descendants::<gtk::Button>(&pack.dialog)
        .iter()
        .filter(|button| button.icon_name().as_deref() == Some("window-close-symbolic"))
        .count();
    let cells = count::<gtk::FlowBoxChild>(&pack.dialog);
    h.screenshot("pack").await;
    h.check(
        "pack_lists_every_sticker_with_its_remove_control",
        cells == pack_items.len() && removable == pack_items.len(),
        format!("{cells} cells, {removable} removable (the reader owns the family)"),
    );
    pack.dialog.close();
    h.sleep(Duration::from_millis(300)).await;

    // ---- avatars -------------------------------------------------------------------------------
    let face = adw::Avatar::new(48, Some("Eli"), true);
    let blank = adw::Avatar::new(48, Some("Dana"), true);
    // Avatars asked before this group (the chat group's people) are not this check's.
    let avatars_before = fake
        .requests()
        .iter()
        .filter(|r| r.ends_with("/avatar"))
        .count();
    attachments::avatars::show(&face, eli, 3);
    attachments::avatars::show(&blank, dana, 0);
    stickers_row.append(&face);
    stickers_row.append(&blank);
    let pictured = h.wait_until(WAIT, || face.custom_image().is_some()).await;
    h.check(
        "avatar_shows_the_picture_of_that_version",
        pictured
            && blank.custom_image().is_none()
            && fake
                .requests()
                .iter()
                .filter(|r| r.ends_with("/avatar"))
                .count()
                == avatars_before + 1,
        format!("picture {pictured}; version 0 asks nothing and draws initials"),
    );

    let large = still(1600, 1200, 0x5e35_b1ff, 0xffff_ffff, true, "png");
    let prepared = attachments::avatars::prepare_upload(large).await;
    let prepared_size = prepared.as_ref().and_then(|(bytes, _)| {
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(bytes.as_slice()));
        gdk_pixbuf::Pixbuf::from_stream(&stream, None::<&gio::Cancellable>)
            .ok()
            .map(|p| (p.width(), p.height()))
    });
    h.check(
        "avatar_upload_is_fitted_into_512_as_jpeg",
        prepared.as_ref().map(|(_, mime)| *mime) == Some("image/jpeg")
            && prepared_size == Some((512, 384)),
        format!(
            "{:?} {prepared_size:?}",
            prepared.as_ref().map(|(_, mime)| *mime)
        ),
    );

    // ---- link cards ----------------------------------------------------------------------------
    let pages = Rc::new(PagesFromMemory::default());
    let link_previews = {
        let settings = Rc::clone(&shell.settings);
        LinkPreviews::new(
            move || settings.flag(keys::LINK_PREVIEWS),
            pages.clone(),
            shell.clock.clone(),
            shell.spawn.clone(),
        )
    };
    let link_slots = AttachmentSlots::with_link_previews(Rc::clone(&link_previews));
    let mut linked = MessageDto::new(
        8006,
        FAMILY_CHAT,
        eli,
        None,
        "Look: https://example.test/trip",
        "2026-10-29T10:05:00Z",
    );
    linked.attachments = None;
    shell.settings.set_bool(keys::LINK_PREVIEWS, false);
    let off = link_slots.link_card(&linked, "https://example.test/trip", false);
    let asked_while_off = pages.asked.get();
    shell.settings.set_bool(keys::LINK_PREVIEWS, true);
    let on = link_slots.link_card(&linked, "https://example.test/trip", false);
    let filled = match &on {
        Some(card) => {
            column.append(card);
            h.wait_until(WAIT, || {
                has_label(card, "Our trip to the lake") && count::<gtk::Picture>(card) == 1
            })
            .await
        }
        None => false,
    };
    h.check(
        "link_card_only_with_the_setting_on",
        off.is_none() && asked_while_off == 0 && filled,
        format!(
            "off: card {} and {asked_while_off} requests; on: card filled {filled}",
            off.is_some()
        ),
    );

    // ---- a location ----------------------------------------------------------------------------
    let tiles = TileServer::start();
    if let Some(tiles) = &tiles {
        let base = tiles.url();
        MapTiles::get().use_source(move |tile| {
            format!("{base}/tiles/{}/{}/{}.png", tile.zoom, tile.x, tile.y)
        });
    }
    let mut place = AttachmentDto::new(7401, "location");
    place.name = Some("Kalemegdan".into());
    place.latitude = Some(44.8231);
    place.longitude = Some(20.4504);
    place.accuracy_m = Some(12.0);
    let mut place_message =
        MessageDto::new(8007, FAMILY_CHAT, eli, None, "", "2026-10-29T10:06:00Z");
    place_message.attachments = Some(vec![place.clone()]);
    shell.settings.set_bool(keys::MAP_PREVIEWS, false);
    let pin_only = slots.location(&place_message, &place);
    let tiles_while_off = tiles.as_ref().map_or(0, |t| t.served.get());
    shell.settings.set_bool(keys::MAP_PREVIEWS, true);
    let mapped = slots.location(&place_message, &place);
    let places = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    places.append(&mapped);
    places.append(&pin_only);
    column.append(&places);
    let drawn_tiles = h
        .wait_until(WAIT, || {
            descendants::<gtk::Picture>(&mapped)
                .iter()
                .filter(|p| {
                    p.paintable()
                        .is_some_and(|p| p.intrinsic_width() == 256 && !is_empty_paintable(&p))
                })
                .count()
                >= 2
        })
        .await;
    h.check(
        "location_card_draws_its_map_only_with_the_setting_on",
        tiles.is_some()
            && drawn_tiles
            && tiles_while_off == 0
            && has_label(&mapped, "Kalemegdan")
            && has_label(&mapped, attachments::location::CREDIT)
            && !has_label(&pin_only, attachments::location::CREDIT)
            && has_label(&pin_only, "44.82310, 20.45040 · ±12 m"),
        format!(
            "tile server {}, tiles drawn {drawn_tiles}, asked while off {tiles_while_off}",
            tiles.is_some()
        ),
    );
    scrolled
        .vadjustment()
        .set_value(scrolled.vadjustment().upper());
    h.screenshot("cards").await;

    // ---- the inbox: "Send to", staged in the chosen chat ---------------------------------------
    let routes = Rc::new(StagingRoutes::default());
    let previous_routes = shell.chat_routes.get();
    shell.chat_routes.install(routes.clone());
    let handed_dir = h.out().join("handed");
    let _ = std::fs::create_dir_all(&handed_dir);
    let handed = handed_dir.join("photo one.jpg");
    let _ = std::fs::write(&handed, &preview_bytes);
    shell.inbox.get().receive(&[gio::File::for_path(&handed)]);
    let inbox = Inbox::current();
    let asked = h
        .wait_until(WAIT, || inbox.as_ref().and_then(|i| i.choosing()).is_some())
        .await;
    let choices: Vec<i64> = inbox
        .as_ref()
        .and_then(|i| i.choosing())
        .map(|c| c.rows.iter().map(|(id, _)| *id).collect())
        .unwrap_or_default();
    h.check(
        "file_handed_over_reaches_send_to",
        asked
            && choices.first() == Some(&FAMILY_CHAT)
            && choices.contains(&DIRECT_CHAT)
            && !choices.contains(&ASSISTANT_CHAT),
        format!(
            "asked {asked}; chats offered {choices:?} (the family first, never the assistant's)"
        ),
    );
    let titled = inbox
        .as_ref()
        .and_then(|i| i.choosing())
        .is_some_and(|c| c.dialog.title() == "Send to");
    h.check("send_to_is_titled", titled, "the dialog says \"Send to\"");
    // The dialog's entrance has settled before it is photographed.
    h.sleep(Duration::from_millis(600)).await;
    h.screenshot("send-to").await;
    if let Some(choosing) = inbox.as_ref().and_then(|i| i.choosing())
        && let Some((_, row)) = choosing.rows.iter().find(|(id, _)| *id == DIRECT_CHAT)
    {
        row.emit_by_name::<()>("activated", &[]);
    }
    let staged = h
        .wait_until(WAIT, || !routes.staged.borrow().is_empty())
        .await;
    h.sleep(Duration::from_millis(200)).await;
    let opened = routes.opened.borrow().clone();
    let staged_rows = routes.staged.borrow().clone();
    let inbox_root = inbox.as_ref().map(|i| i.root().to_path_buf());
    let left = inbox_root
        .as_ref()
        .and_then(|root| std::fs::read_dir(root).ok())
        .map_or(0, |entries| entries.count());
    let copied_in_inbox = staged_rows
        .first()
        .is_some_and(|(_, names, existed, under)| {
            names == &vec!["photo one.jpg".to_string()] && *existed && *under
        });
    h.check(
        "file_staged_in_the_chosen_chat_nothing_sent",
        staged
            && opened == vec![DIRECT_CHAT]
            && staged_rows.first().map(|row| row.0) == Some(DIRECT_CHAT)
            && copied_in_inbox
            && left == 0
            && handed.exists(),
        format!(
            "opened {opened:?}; staged {:?}; copies left in the inbox {left}; the original untouched {}",
            staged_rows.iter().map(|r| (r.0, r.1.clone(), r.2, r.3)).collect::<Vec<_>>(),
            handed.exists()
        ),
    );
    shell.chat_routes.install(previous_routes);

    // ---- the Location portal: absent here, answered as a refusal, never a hang -----------------
    let portal = attachments::location_portal::available().await;
    let found = h
        .with_timeout(
            Duration::from_secs(30),
            attachments::location_portal::find(""),
        )
        .await;
    h.check(
        "location_portal_refuses_without_hanging",
        found.is_some(),
        format!("portal answers {portal}; find answered {found:?} (no GeoClue in Docker)"),
    );
    let _ = connection;
    window.close();
}

/// Opens the fake server and signs in as `username` (signing out whoever was in), then waits for
/// the chats.
async fn sign_in(h: &Harness, url: &str, username: &str) -> bool {
    let shell = h.shell();
    let Some(server) = ServerUrl::normalise(Some(url)) else {
        h.check("signed_in", false, "the fake server's address");
        return false;
    };
    let same_server = shell
        .connection()
        .is_some_and(|connection| connection.server == server);
    if !same_server {
        shell.open_server(server);
        h.wait_until(WAIT, || {
            shell.connection().is_some() && !shell.gate_facts().booting
        })
        .await;
    }
    let Some(connection) = shell.connection() else {
        h.check("signed_in", false, "no connection to the fake server");
        return false;
    };
    let me = connection.session.state().me.map(|me| me.username);
    if me.as_deref() != Some(username) {
        if me.is_some() {
            let _ = connection.session.sign_out().await;
        }
        let answer = connection.session.sign_in(username, PASSWORD).await;
        if !matches!(answer, Ok(None)) {
            h.check("signed_in", false, format!("{answer:?}"));
            return false;
        }
    }
    let Some(main) = shell.main_window() else {
        return false;
    };
    let on_chats = h
        .wait_until(WAIT, || {
            main.page() == GatePage::Chats
                && connection.session.state().gate != Gate::SignedOut
                && connection.chats.chat(FAMILY_CHAT).ok().flatten().is_some()
        })
        .await;
    h.check("signed_in", on_chats, format!("page {:?}", main.page()));
    on_chats
}

/// A still picture: the top half `top`, the bottom half `bottom` (RGBA), as `format`.
fn still(width: i32, height: i32, top: u32, bottom: u32, alpha: bool, format: &str) -> Vec<u8> {
    let Some(pixbuf) =
        gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, alpha, 8, width, height)
    else {
        return Vec::new();
    };
    pixbuf.fill(top);
    let half = height / 2;
    pixbuf
        .new_subpixbuf(0, half, width, height - half)
        .fill(bottom);
    // A square in the middle, so a picture's orientation and crop can be seen.
    let side = width.min(height) / 4;
    pixbuf
        .new_subpixbuf((width - side) / 2, (height - side) / 2, side, side)
        .fill(0xffff_ffff);
    pixbuf.save_to_bufferv(format, &[]).unwrap_or_default()
}

/// An animated GIF (three frames, 200 ms each) made by ImageMagick, or `None`.
async fn animated_gif(h: &Harness) -> Option<Vec<u8>> {
    let out = h.out().join("animated.gif");
    let path = out.to_string_lossy().into_owned();
    for program in ["magick", "convert"] {
        let (ok, _) = h
            .run(&[
                program,
                "-size",
                "96x96",
                "-delay",
                "20",
                "xc:#e53935",
                "xc:#43a047",
                "xc:#1e88e5",
                "-loop",
                "0",
                &path,
            ])
            .await;
        if ok && let Ok(bytes) = std::fs::read(&out) {
            return Some(bytes);
        }
    }
    None
}

fn pack_item(id: i64, attachment: i64, width: i32, height: i32, added_by: i64) -> PackItemDto {
    let mut picture = AttachmentDto::new(attachment, "photo");
    picture.sticker = true;
    picture.width = Some(width);
    picture.height = Some(height);
    picture.mime = Some("image/png".into());
    PackItemDto {
        added_by,
        attachment: Some(picture),
        pack_seq: id,
        ..PackItemDto::new(id)
    }
}

/// Whether a paintable is still the empty stand-in (`gdk::Paintable::new_empty` draws nothing and
/// has no flags of a texture).
fn is_empty_paintable(paintable: &gdk::Paintable) -> bool {
    paintable.type_().name() == "GdkEmptyPaintable"
}

fn descendants<T: IsA<gtk::Widget>>(root: &impl IsA<gtk::Widget>) -> Vec<T> {
    let mut found = Vec::new();
    let mut stack = vec![root.as_ref().clone()];
    while let Some(widget) = stack.pop() {
        if let Some(hit) = widget.downcast_ref::<T>() {
            found.push(hit.clone());
        }
        // In the order they are drawn: the first child is looked at first.
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            children.push(next);
        }
        stack.extend(children.into_iter().rev());
    }
    found
}

fn count<T: IsA<gtk::Widget>>(root: &impl IsA<gtk::Widget>) -> usize {
    descendants::<T>(root).len()
}

fn has_label(root: &impl IsA<gtk::Widget>, text: &str) -> bool {
    descendants::<gtk::Label>(root)
        .iter()
        .any(|label| label.label() == text)
}

fn has_label_starting(root: &impl IsA<gtk::Widget>, text: &str) -> bool {
    descendants::<gtk::Label>(root)
        .iter()
        .any(|label| label.label().starts_with(text))
}

/// Pages from memory for fc-logic's real `LinkPreviews` (the https rule cannot be met on loopback).
#[derive(Default)]
struct PagesFromMemory {
    asked: Cell<u32>,
}

impl Http for PagesFromMemory {
    fn send(
        &self,
        request: HttpRequest,
    ) -> LocalBoxFuture<'_, Result<HttpResponse, TransportError>> {
        self.asked.set(self.asked.get() + 1);
        let response = match request.url.as_str() {
            "https://example.test/trip" => HttpResponse {
                status: 200,
                headers: vec![("Content-Type".into(), "text/html; charset=utf-8".into())],
                body: br#"<html><head>
<meta property="og:title" content="Our trip to the lake">
<meta property="og:site_name" content="Example Travel">
<meta property="og:description" content="Three days by the water, with the whole family and the dog.">
<meta property="og:image" content="https://example.test/lake.png">
</head><body></body></html>"#
                    .to_vec(),
            },
            "https://example.test/lake.png" => HttpResponse {
                status: 200,
                headers: vec![("Content-Type".into(), "image/png".into())],
                body: still(560, 240, 0x4fc3_f7ff, 0x2e7d_32ff, false, "png"),
            },
            _ => HttpResponse {
                status: 404,
                headers: Vec::new(),
                body: Vec::new(),
            },
        };
        Box::pin(std::future::ready(Ok(response)))
    }
}

/// A tile server on loopback (never OpenStreetMap's): any `/tiles/…` is a 256-px PNG.
struct TileServer {
    _server: soup::Server,
    port: u16,
    served: Rc<Cell<u32>>,
}

impl TileServer {
    fn start() -> Option<TileServer> {
        use soup::prelude::*;
        let server = soup::Server::builder().build();
        server
            .listen_local(0, soup::ServerListenOptions::IPV4_ONLY)
            .ok()?;
        let port = server
            .uris()
            .first()
            .and_then(|uri| u16::try_from(uri.port()).ok())?;
        let served = Rc::new(Cell::new(0));
        let counter = Rc::clone(&served);
        let tile = still(256, 256, 0xe8e4_d8ff, 0xc8dc_b4ff, false, "png");
        server.add_handler(Some("/tiles"), move |_, message, _, _| {
            counter.set(counter.get() + 1);
            message.set_status(200, None);
            message.set_response(Some("image/png"), soup::MemoryUse::Copy, &tile);
        });
        Some(TileServer {
            _server: server,
            port,
            served,
        })
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// `ChatRoutes` that remember what the inbox asked: the chats opened, and each staging (chat, the
/// files' names, whether they existed then, whether they were the inbox's own copies).
#[derive(Default)]
struct StagingRoutes {
    opened: RefCell<Vec<i64>>,
    staged: Rc<RefCell<Vec<(i64, Vec<String>, bool, bool)>>>,
}

impl ChatRoutes for StagingRoutes {
    fn open_chat(&self, chat_id: i64) {
        self.opened.borrow_mut().push(chat_id);
    }

    fn stage_files(&self, chat_id: i64, files: Vec<gio::File>) -> LocalBoxFuture<'static, ()> {
        let inbox = Shell::get().paths.inbox_dir();
        let names = files
            .iter()
            .filter_map(|file| file.basename())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        let existed = files
            .iter()
            .all(|file| file.path().is_some_and(|path| path.is_file()));
        let under = files
            .iter()
            .all(|file| file.path().is_some_and(|path| path.starts_with(&inbox)));
        self.staged
            .borrow_mut()
            .push((chat_id, names, existed, under));
        Box::pin(std::future::ready(()))
    }
}
