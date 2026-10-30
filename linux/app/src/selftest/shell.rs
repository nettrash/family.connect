//! The `shell` group (S0, §27): the window opens with its frame, sizes and title; every action and
//! accelerator of W.3 exists; the gate's pages switch — Server → SignIn → (a wrong password stays) →
//! Family → Chats with the live socket up → SignIn when the socket closes with 4401 → Offline at a
//! launch that cannot be answered → Chats again from its Try Again; a route waits for the chats and
//! for the recorder; Ctrl+R resyncs, Ctrl+? opens the shortcuts; a narrow window collapses; dark
//! mode re-tints; hide and activate; no icon means a close quits; the fake server knew every
//! request the client made.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::ServerUrl;
use fc_core::text::catalogues::Languages;
use fc_logic::live_connection::Link;
use fc_logic::tray_rules;

use super::Harness;
use crate::seams::actions;
use crate::seams::routes::ChatRoutes;
use crate::services::catalogue;
use crate::services::settings_store::keys;
use crate::shell::BRAND;
use crate::theme;
use crate::windows::main_window::{DEFAULT_SIZE, GatePage, MINIMUM_SIZE, MainWindow};

const WAIT: Duration = Duration::from_secs(15);

/// A `ChatRoutes` that remembers what it was asked to open.
#[derive(Default)]
struct RecordingRoutes(RefCell<Vec<i64>>);

impl ChatRoutes for RecordingRoutes {
    fn open_chat(&self, chat_id: i64) {
        self.0.borrow_mut().push(chat_id);
    }
}

pub async fn run(h: &Harness) {
    let shell = h.shell();
    let Some(main) = shell.main_window() else {
        h.check("main_window", false, "no main window was built");
        return;
    };
    let window: gtk::Window = main.window().clone().upcast();

    // ---- the window and its frame ----------------------------------------------------------------
    let title = window.title().map(|t| t.to_string()).unwrap_or_default();
    h.check(
        "app_starts",
        window.is_visible() && title == BRAND,
        format!("visible {}, title {title:?}", window.is_visible()),
    );
    h.check(
        "default_size",
        window.default_size() == DEFAULT_SIZE,
        format!("{:?}", window.default_size()),
    );
    h.check(
        "minimum_size",
        window.size_request() == MINIMUM_SIZE,
        format!("{:?}", window.size_request()),
    );
    h.check(
        "catalogue_loaded",
        catalogue::shipped(&shell.paths.i18n_dir()) == Languages::ALL,
        format!("data folder {}", shell.paths.data_dir().display()),
    );

    // ---- actions and accelerators ------------------------------------------------------------------
    let missing: Vec<String> = actions::APP_ACTIONS
        .iter()
        .filter(|spec| {
            shell
                .app
                .lookup_action(spec.name)
                .is_none_or(|action| action.parameter_type().as_deref() != spec.variant_type())
        })
        .map(|spec| spec.detailed())
        .chain(
            actions::WINDOW_ACTIONS
                .iter()
                .filter(|spec| main.window().lookup_action(spec.name).is_none())
                .map(|spec| spec.detailed()),
        )
        .collect();
    h.check(
        "actions_exist",
        missing.is_empty(),
        format!("missing {missing:?}"),
    );
    let paste_off = main
        .window()
        .lookup_action(actions::PASTE_ATTACHMENT.name)
        .is_some_and(|action| !action.is_enabled());
    h.check(
        "paste_attachment_off_by_default",
        paste_off,
        "win.paste-attachment must wait for P1",
    );
    let mut wrong = Vec::new();
    for (name, accels) in actions::accelerators() {
        let bound: Vec<String> = shell
            .app
            .accels_for_action(&name)
            .iter()
            .map(|a| a.to_string())
            .collect();
        // GTK writes a chord back in its own order (`<Shift><Control>r`): compare what they mean.
        let meaning =
            |chords: Vec<String>| -> Option<Vec<(gtk::gdk::Key, gtk::gdk::ModifierType)>> {
                chords
                    .iter()
                    .map(|a| gtk::accelerator_parse(a.as_str()))
                    .collect()
            };
        let wanted = meaning(accels.iter().map(|a| a.to_string()).collect());
        if wanted.is_none() || meaning(bound.clone()) != wanted {
            wrong.push(format!("{name}: {bound:?}"));
        }
    }
    h.check("accels_bound", wrong.is_empty(), format!("{wrong:?}"));

    // ---- the gate ------------------------------------------------------------------------------------
    h.check(
        "page_server",
        main.page() == GatePage::Server,
        format!("{:?}", main.page()),
    );
    h.screenshot("server").await;

    let fake = h.fake_server();
    let (_alice, _bob, _carol, _family) = fake.seed_default();
    let server = ServerUrl::normalise(Some(&fake.url())).expect("the fake's address");
    shell.open_server(server.clone());
    let signed_out = h.wait_until(WAIT, || main.page() == GatePage::SignIn).await;
    h.check("page_sign_in", signed_out, format!("{:?}", main.page()));
    h.check(
        "server_saved",
        shell.settings.string(keys::SERVER) == Some(server.to_string()),
        format!("{:?}", shell.settings.string(keys::SERVER)),
    );
    h.check(
        "no_token_no_me",
        !fake.requests().iter().any(|r| r == "GET /api/v1/me"),
        "without a token the gate is signed out without asking",
    );
    h.screenshot("sign-in").await;

    let Some(connection) = shell.connection() else {
        h.check("connection", false, "no connection after open_server");
        return;
    };
    let refused = connection.session.sign_in("carol", "wrong").await;
    let refused_code = refused
        .as_ref()
        .ok()
        .and_then(|error| error.as_ref().map(|e| e.code.clone()));
    h.check(
        "wrong_password_is_an_answer_not_a_sign_out",
        refused_code.as_deref() == Some("invalid_credentials") && main.page() == GatePage::SignIn,
        format!("{refused_code:?}, page {:?}", main.page()),
    );

    let carol = connection.session.sign_in("carol", "password").await;
    let family_page = h.wait_until(WAIT, || main.page() == GatePage::Family).await;
    h.check(
        "page_family",
        matches!(carol, Ok(None)) && family_page,
        format!(
            "{:?}, page {:?}",
            carol.map(|e| e.map(|e| e.code)),
            main.page()
        ),
    );
    h.screenshot("family").await;

    let created = connection
        .api
        .create_family("Carol's Family")
        .await
        .into_result()
        .map(|answer| answer.family.id);
    if let Ok(family) = created {
        let _ = connection.session.refresh_preferring(Some(family)).await;
    }
    let chats = h.wait_until(WAIT, || main.page() == GatePage::Chats).await;
    h.check(
        "page_chats",
        chats,
        format!("{created:?}, page {:?}", main.page()),
    );
    let socket_up = h
        .wait_until(WAIT, || {
            connection.live.link() == Link::Up && fake.open_sockets() == 1
        })
        .await;
    h.check(
        "socket_up_with_bearer",
        socket_up,
        format!(
            "link {:?}, open sockets {}",
            connection.live.link(),
            fake.open_sockets()
        ),
    );
    let resynced = h
        .wait_until(WAIT, || {
            fake.requests().iter().any(|r| r == "GET /api/v1/chats")
        })
        .await;
    h.check("resync_ran", resynced, format!("{:?}", fake.requests()));
    h.check(
        "title_counts_nothing_unread",
        window.title().as_deref() == Some(BRAND),
        format!("{:?}", window.title()),
    );
    h.check(
        "family_remembered",
        shell
            .settings
            .string(keys::CURRENT_FAMILY)
            .is_some_and(|text| {
                created
                    .as_ref()
                    .is_ok_and(|id| text.contains(&format!("\t{id}")))
            }),
        format!("{:?}", shell.settings.string(keys::CURRENT_FAMILY)),
    );
    h.screenshot("chats").await;

    // ---- keys -------------------------------------------------------------------------------------
    if h.input() {
        h.focus(&window).await;
        let before = count(&fake.requests(), "GET /api/v1/chats");
        h.key("ctrl+r").await;
        let refreshed = h
            .wait_until(WAIT, || {
                count(&fake.requests(), "GET /api/v1/chats") > before
            })
            .await;
        h.check(
            "ctrl_r_resyncs",
            refreshed,
            format!("{before} passes before"),
        );

        h.key("ctrl+question").await;
        let open = h
            .wait_until(Duration::from_secs(5), || {
                main.window().visible_dialog().is_some()
            })
            .await;
        h.check(
            "ctrl_question_opens_shortcuts",
            open,
            "adw::ShortcutsDialog",
        );
        h.screenshot("shortcuts").await;
        h.key("Escape").await;
        let closed = h
            .wait_until(Duration::from_secs(5), || {
                main.window().visible_dialog().is_none()
            })
            .await;
        h.check("escape_closes_shortcuts", closed, "");

        h.resize(&window, 520, 600).await;
        let collapsed = h
            .wait_until(Duration::from_secs(5), || main.split_view().is_collapsed())
            .await;
        h.check(
            "narrow_window_collapses",
            collapsed,
            "below 680 sp: one pane",
        );
        h.screenshot("narrow").await;
        h.resize(&window, DEFAULT_SIZE.0, DEFAULT_SIZE.1).await;
        let wide = h
            .wait_until(Duration::from_secs(5), || !main.split_view().is_collapsed())
            .await;
        h.check("wide_window_splits", wide, "");
    }

    // ---- dark mode and the accent ---------------------------------------------------------------
    let styles = adw::StyleManager::default();
    styles.set_color_scheme(adw::ColorScheme::ForceDark);
    let dark = h
        .wait_until(Duration::from_secs(5), || {
            theme::current_sheet().contains("--accent-bg-color: #4d7dfc;")
        })
        .await;
    h.check("dark_accent", dark && theme::is_dark(), "#4D7DFC on dark");
    h.screenshot("chats-dark").await;
    styles.set_color_scheme(adw::ColorScheme::Default);
    let light = h
        .wait_until(Duration::from_secs(5), || {
            theme::current_sheet().contains("--accent-bg-color: #0d47a1;")
        })
        .await;
    h.check("light_accent", light, "#0D47A1 on light");

    // ---- a route waits for the chats, and for the recorder -----------------------------------------
    let routes = Rc::new(RecordingRoutes::default());
    // The routes found here (P1's) go back at the end, for every later group.
    let found_routes = shell.chat_routes.get();
    shell.chat_routes.install(routes.clone());

    // The session dies under the socket: 4401 lands where a REST 401 lands, once.
    fake.expire_tokens();
    fake.close_sockets(4401);
    let expired = h.wait_until(WAIT, || main.page() == GatePage::SignIn).await;
    h.check(
        "socket_4401_signs_out",
        expired,
        format!("{:?}", main.page()),
    );
    let upgrades = count(&fake.requests(), "GET /api/v1/ws (upgrade)");
    h.sleep(Duration::from_secs(3)).await;
    h.check(
        "no_reconnect_after_expiry",
        count(&fake.requests(), "GET /api/v1/ws (upgrade)") == upgrades,
        format!("{upgrades} upgrades"),
    );

    shell
        .app
        .activate_action(actions::OPEN_CHAT.name, Some(&7i64.to_variant()));
    h.sleep(Duration::from_millis(300)).await;
    h.check(
        "route_held_until_chats",
        routes.0.borrow().is_empty(),
        format!("{:?}", routes.0.borrow()),
    );
    let Some(connection) = shell.connection() else {
        h.check("connection", false, "lost the connection");
        return;
    };
    let _ = connection.session.sign_in("alice", "password").await;
    let applied = h
        .wait_until(WAIT, || routes.0.borrow().as_slice() == [7])
        .await;
    h.check(
        "route_applied_at_chats",
        applied,
        format!("{:?}", routes.0.borrow()),
    );
    main.set_recorder_open(true);
    shell
        .app
        .activate_action(actions::OPEN_CHAT.name, Some(&8i64.to_variant()));
    h.sleep(Duration::from_millis(300)).await;
    let held = routes.0.borrow().as_slice() == [7];
    main.set_recorder_open(false);
    h.check(
        "route_held_while_recording",
        held && routes.0.borrow().as_slice() == [7, 8],
        format!("{:?}", routes.0.borrow()),
    );
    // Untrusted targets open nothing.
    shell
        .app
        .activate_action(actions::OPEN_CHAT.name, Some(&(-3i64).to_variant()));
    h.sleep(Duration::from_millis(200)).await;
    h.check(
        "a_bad_target_opens_nothing",
        routes.0.borrow().as_slice() == [7, 8],
        format!("{:?}", routes.0.borrow()),
    );
    shell.chat_routes.install(found_routes);

    // ---- offline at launch, with chats held: they open (the Mac's `openOffline`) ----------------
    // The fake family has none of its own: one is put in the cache, as a family chat read earlier.
    if let Some(connection) = shell.connection()
        && let Err(error) =
            connection
                .chats
                .upsert(&fc_core::protocol::ChatDto::new(42, "family", "The Smiths"))
    {
        crate::fc_warn!(
            "selftest: the cache refused ({})",
            crate::services::log::store_error(&error)
        );
    }
    fake.set_down(true);
    shell.open_server(server.clone());
    let left = h.wait_until(WAIT, || main.page() != GatePage::Chats).await;
    let reopened = h.wait_until(WAIT, || main.page() == GatePage::Chats).await;
    h.check(
        "offline_launch_opens_the_cached_chats",
        left && reopened,
        format!("left {left}, page {:?}", main.page()),
    );

    // ---- offline at launch with nothing held: not a sign-out ----------------------------------------
    if let Some(connection) = shell.connection()
        && let Err(error) = connection.cache.wipe_all()
    {
        crate::fc_warn!(
            "selftest: the cache refused ({})",
            crate::services::log::store_error(&error)
        );
    }
    shell.open_server(server.clone());
    let offline = h
        .wait_until(WAIT, || main.page() == GatePage::Offline)
        .await;
    h.check("page_offline", offline, format!("{:?}", main.page()));
    h.screenshot("offline").await;
    fake.set_down(false);
    let again = try_again_button(&main);
    let clicked = match &again {
        Some(button) if h.input() => h.click_middle(&window, button).await,
        _ => false,
    };
    if !clicked {
        shell.retry();
    }
    let back = h.wait_until(WAIT, || main.page() == GatePage::Chats).await;
    h.check(
        "try_again_reaches_the_chats",
        back,
        format!("clicked {clicked}, page {:?}", main.page()),
    );

    // ---- hiding, the close rule, the size --------------------------------------------------------
    h.check(
        "close_quits_without_an_icon",
        !tray_rules::close_hides(shell.keeps_running(), shell.tray.get().available(), false)
            && !shell.can_hide_to_tray(),
        "S0 has no top-bar icon: a close is a Quit",
    );
    main.hide_to_tray();
    let hidden = !window.is_visible();
    shell.app.activate();
    let shown = h
        .wait_until(Duration::from_secs(5), || window.is_visible())
        .await;
    h.check(
        "hide_and_activate",
        hidden && shown,
        format!("hidden {hidden}, shown {shown}"),
    );
    if h.input() {
        h.focus(&window).await;
    }
    main.toggle();
    h.check(
        "toggle_without_an_icon_never_hides",
        window.is_visible(),
        "",
    );
    main.remember_size();
    h.check(
        "size_remembered",
        shell.settings.window_size() == Some(window.default_size()),
        format!("{:?}", shell.settings.window_size()),
    );

    // ---- where the files went ---------------------------------------------------------------------
    h.check(
        "cache_in_state_home",
        shell.paths.cache_db().is_file()
            && shell.paths.cache_db().starts_with(shell.paths.state_dir())
            && shell.paths.settings_file().is_file(),
        format!("{}", shell.paths.cache_db().display()),
    );
    let unhandled = fake.unhandled();
    h.check(
        "fake_server_knew_every_request",
        unhandled.is_empty(),
        format!("{unhandled:?}"),
    );
    h.screenshot("end").await;
}

fn count(requests: &[String], line: &str) -> usize {
    requests.iter().filter(|r| r.as_str() == line).count()
}

/// The Offline page's Try Again button, found inside its slot.
fn try_again_button(main: &Rc<MainWindow>) -> Option<gtk::Button> {
    let slot = main.page_slot(GatePage::Offline);
    let mut pending: Vec<gtk::Widget> = slot.child().into_iter().collect();
    while let Some(widget) = pending.pop() {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
            return Some(button);
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            pending.push(next);
        }
    }
    None
}
