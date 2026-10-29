//! The Settings window (shell-design §13, §3.2; the Mac's `MacSettingsView`): ONE `adw::Window`
//! (460 × 530, at least 360 × 420) of `adw::PreferencesGroup`s, opened by `app.show-settings`
//! (Ctrl+,, the primary menu, the tray menu) and raised when it is open already — a singleton, as
//! the Mac's Settings scene is, so two copies can never show each other's switches stale.
//!
//! Who you are at the top, then: Profile, Family, Notifications, Top Bar, Privacy, Voice and video,
//! Server, and the version line. Every switch is an `adw::SwitchRow` saved to `settings.conf` at
//! once; the device's rows (notifications, top bar, start at login, hot key, previews, speed,
//! camera) survive a sign-out, the account's do not. Signed out, the window says so instead of
//! drawing a panel of dashes with two destructive buttons under it (the Mac's rule).
//!
//! The rows' looks are `settings::rows`' (pure, tested); the mechanisms behind the Top Bar rows are
//! P5's (`TrayState`, `AutostartState`, `HotKeyState`) — the hot-key row only WRITES the `hot-key`
//! setting and S0 applies it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::families::family_rules;
use fc_logic::settings::birthday_rules;

use crate::family::birthday::{self, Whose};
use crate::seams::media::CameraDevice;
use crate::services::settings_store::keys;
use crate::settings::{dialogs, rows, ui};
use crate::shell::Shell;

/// The window's default size (the Mac's Settings scene).
pub const DEFAULT_SIZE: (i32, i32) = (460, 530);
/// The smallest it may be.
pub const MINIMUM_SIZE: (i32, i32) = (360, 420);
/// The Privacy Policy and Support pages — the Mac's and Windows' own.
const PRIVACY_URL: &str = "https://nettrash.me/appstore/familyconnect/privacy.html";
const SUPPORT_URL: &str = "https://nettrash.me/appstore/familyconnect/support.html";

thread_local! {
    static OPEN: RefCell<Option<Rc<SettingsWindow>>> = const { RefCell::new(None) };
}

/// `app.show-settings`: the Settings window, built once and presented.
pub fn show() -> Rc<SettingsWindow> {
    let window = OPEN.with(|open| open.borrow().clone()).unwrap_or_else(|| {
        let window = SettingsWindow::build(&Shell::get());
        OPEN.with(|open| *open.borrow_mut() = Some(Rc::clone(&window)));
        window
    });
    window.refresh();
    window.window.present();
    window
}

/// The window, when it was ever opened (the self-test reads it).
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "the self-test reads it")
)]
pub fn current() -> Option<Rc<SettingsWindow>> {
    OPEN.with(|open| open.borrow().clone())
}

/// The Settings window (see the module's comment).
pub struct SettingsWindow {
    pub window: adw::Window,
    pub toasts: adw::ToastOverlay,
    /// "settings" or "signed-out".
    pub stack: gtk::Stack,
    name: gtk::Label,
    username: gtk::Label,
    owner: gtk::Label,
    avatar: adw::Avatar,
    /// The profile photo: "Add Photo" / "Change Photo", and "Remove Photo" while there is one.
    pub photo: gtk::Button,
    pub remove_photo: gtk::Button,
    pub birthday: adw::ActionRow,
    birthday_button: gtk::Button,
    pub family_name: adw::ActionRow,
    pub new_family: adw::ButtonRow,
    pub join_another: adw::ButtonRow,
    pub notify: adw::SwitchRow,
    pub keep_running: adw::SwitchRow,
    /// What a close does while an icon holds the app — hidden with no icon, where the switch
    /// already says that a close quits.
    pub top_bar_note: gtk::Label,
    pub autostart: adw::SwitchRow,
    pub hot_key: adw::SwitchRow,
    /// "Change…" beside the shortcut, where the portal offers `ConfigureShortcuts` (§13, §19).
    pub hot_key_change: gtk::Button,
    pub link_previews: adw::SwitchRow,
    pub map_previews: adw::SwitchRow,
    pub speed: adw::ToggleGroup,
    pub camera: adw::ComboRow,
    cameras: RefCell<Vec<CameraDevice>>,
    pub address: adw::ActionRow,
    pub remembered: gtk::Label,
    /// Set while `refresh` writes the rows, so their notify handlers do not write back.
    updating: Cell<bool>,
}

impl std::fmt::Debug for SettingsWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsWindow").finish_non_exhaustive()
    }
}

impl SettingsWindow {
    fn build(shell: &Rc<Shell>) -> Rc<SettingsWindow> {
        let say = Rc::clone(&shell.say);
        let window = adw::Window::builder()
            .title(say.get("Settings"))
            .default_width(DEFAULT_SIZE.0)
            .default_height(DEFAULT_SIZE.1)
            .hide_on_close(true)
            .build();
        // `fc-settings`: a disabled row's reason stays readable (`board::css`). Added, never set:
        // the builder's list would replace the window's own `background`.
        window.add_css_class("fc-settings");
        window.set_size_request(MINIMUM_SIZE.0, MINIMUM_SIZE.1);
        window.set_application(Some(&shell.app));

        // ---- who you are -------------------------------------------------------------------
        // A centred hero, GNOME's identity pattern (Settings ▸ Users, the About dialog): the face,
        // the name under it, the username and the Owner pill below.
        let avatar = ui::avatar("?", 80);
        avatar.set_halign(gtk::Align::Center);
        let name = gtk::Label::builder()
            .xalign(0.5)
            .justify(gtk::Justification::Center)
            .wrap(true)
            .build();
        name.add_css_class("title-2");
        let username = gtk::Label::builder().xalign(0.5).build();
        username.add_css_class("dim-label");
        let owner = ui::owner_pill(say.get("Owner"));
        owner.set_halign(gtk::Align::Center);
        let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
        names.set_halign(gtk::Align::Center);
        names.append(&name);
        names.append(&username);
        names.append(&owner);
        let identity = gtk::Box::new(gtk::Orientation::Vertical, 8);
        identity.set_halign(gtk::Align::Center);
        identity.append(&ui::hero_face(&avatar));
        identity.append(&names);
        let identity_group = adw::PreferencesGroup::new();
        identity_group.add(&identity);

        // ---- Profile -------------------------------------------------------------------------
        let profile = adw::PreferencesGroup::builder()
            .title(say.get("Profile"))
            .build();
        let photo_row = adw::ActionRow::builder().title(say.get("Photo")).build();
        let photo = gtk::Button::builder()
            .label(say.get("Add Photo"))
            .valign(gtk::Align::Center)
            .build();
        // Red words without a fill: the secondary action beside Change Photo, not its rival.
        let remove_photo = gtk::Button::builder()
            .label(say.get("Remove Photo"))
            .valign(gtk::Align::Center)
            .css_classes(["destructive-action", "flat"])
            .visible(false)
            .build();
        photo_row.add_suffix(&remove_photo);
        photo_row.add_suffix(&photo);
        photo.connect_clicked(|_| {
            if let Some(open) = current() {
                open.choose_photo();
            }
        });
        remove_photo.connect_clicked(|_| {
            if let Some(open) = current() {
                open.remove_photo();
            }
        });
        profile.add(&photo_row);
        let birthday = adw::ActionRow::builder().title(say.get("Birthday")).build();
        birthday.add_css_class("property");
        let birthday_button = gtk::Button::builder().valign(gtk::Align::Center).build();
        birthday.add_suffix(&birthday_button);
        profile.add(&birthday);
        profile.add(&ui::button_row(
            say.get("Change Password…"),
            false,
            || {
                if let Some(open) = current() {
                    dialogs::change_password(&open.window);
                }
            },
        ));

        // ---- Family --------------------------------------------------------------------------
        let family = adw::PreferencesGroup::builder()
            .title(say.get("Family"))
            .build();
        let family_name = ui::property_row(say.get("Name"), "—");
        let new_family = ui::button_row(say.get("New Family…"), false, || {
            if let Some(open) = current() {
                dialogs::new_family(&open.window);
            }
        });
        let join_another = ui::button_row(say.get("Join Another Family…"), false, || {
            if let Some(open) = current() {
                dialogs::join_another(&open.window);
            }
        });
        let leave = ui::button_row(say.get("Leave Family"), true, || {
            if let Some(open) = current() {
                dialogs::leave_family(&open.window, &open.toasts);
            }
        });
        let statistics = ui::button_row(say.get("Statistics…"), false, || {
            if let Some(open) = current() {
                dialogs::statistics(&open.window);
            }
        });
        family.add(&family_name);
        family.add(&new_family);
        family.add(&join_another);
        family.add(&statistics);
        family.add(&leave);

        // ---- Notifications -------------------------------------------------------------------
        let notifications = adw::PreferencesGroup::builder()
            .title(say.get("Notifications"))
            .build();
        let notify = adw::SwitchRow::builder()
            .title(say.get("Tell me when a message arrives"))
            .build();
        notifications.add(&notify);
        for note in rows::notification_notes(say.as_ref()) {
            notifications.add(&ui::footnote(&note));
        }

        // ---- Top Bar (Linux's "Menu Bar") --------------------------------------------------------
        let top_bar = adw::PreferencesGroup::builder()
            .title(say.get("Top Bar"))
            .build();
        let keep_running = adw::SwitchRow::new();
        let autostart = adw::SwitchRow::new();
        let hot_key = adw::SwitchRow::new();
        let hot_key_change = gtk::Button::builder()
            .label(say.get("Change…"))
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        hot_key.add_suffix(&hot_key_change);
        // A switch row put its own switch in the suffixes first: Change… goes before it, so every
        // switch of the group ends its row in one column.
        if let Some(suffixes) = hot_key_change.parent().and_downcast::<gtk::Box>() {
            suffixes.reorder_child_after(&hot_key_change, None::<&gtk::Widget>);
        }
        {
            let window = window.clone();
            hot_key_change.connect_clicked(move |_| {
                Shell::get()
                    .hot_key
                    .get()
                    .configure(Some(window.upcast_ref()));
            });
        }
        top_bar.add(&keep_running);
        top_bar.add(&autostart);
        top_bar.add(&hot_key);
        let top_bar_note = ui::footnote(say.get(
            "When you close the window, Family Connect stays in the top bar, so messages and calls still reach you. Quit it from its icon there.",
        ));
        top_bar.add(&top_bar_note);

        // ---- Privacy -------------------------------------------------------------------------
        let privacy = adw::PreferencesGroup::builder()
            .title(say.get("Privacy"))
            .build();
        let link_previews = adw::SwitchRow::builder()
            .title(say.get("Link Previews"))
            .subtitle(say.get("Shows a preview under links in messages. Building one asks the linked website for its title and image, so that site sees a request from this device."))
            .build();
        let map_previews = adw::SwitchRow::builder()
            .title(say.get("Map Previews"))
            .subtitle(say.get("Shows a map on a shared location. Drawing one asks OpenStreetMap for the map around that place, so OpenStreetMap sees a request from this device. With maps off, a shared location still shows its pin and opens in a map when you click it."))
            .build();
        privacy.add(&link_previews);
        privacy.add(&map_previews);
        privacy.add(&link_row(say.get("Privacy Policy"), PRIVACY_URL));
        privacy.add(&link_row(say.get("Support"), SUPPORT_URL));

        // ---- Voice and video -------------------------------------------------------------------
        let voice = adw::PreferencesGroup::builder()
            .title(say.get("Voice and video"))
            .build();
        let speed = adw::ToggleGroup::builder()
            .valign(gtk::Align::Center)
            .build();
        for name in rows::speed_names() {
            let rate = fc_logic::voice_look::voice_speed::parse(Some(&name));
            speed.add(
                adw::Toggle::builder()
                    .name(name.as_str())
                    .label(fc_logic::voice_look::voice_speed::label(rate, say.as_ref()))
                    .tooltip(fc_logic::voice_look::voice_speed::name(rate, say.as_ref()))
                    .build(),
            );
        }
        let speed_row = adw::ActionRow::builder()
            .title(say.get("Playback speed"))
            .build();
        speed_row.add_suffix(&speed);
        let camera = adw::ComboRow::builder().title(say.get("Camera")).build();
        voice.add(&speed_row);
        voice.add(&camera);

        // ---- Server ----------------------------------------------------------------------------
        let server = adw::PreferencesGroup::builder()
            .title(say.get("Server"))
            .build();
        let address = ui::property_row(say.get("Address"), "—");
        server.add(&address);
        let log_out = ui::button_row(say.get("Log Out"), true, || {
            if let Some(open) = current() {
                open.ask_log_out();
            }
        });
        server.add(&log_out);
        server.add(&ui::button_row(say.get("Delete Account…"), true, || {
            if let Some(open) = current() {
                dialogs::delete_account(&open.window);
            }
        }));
        let remembered = ui::footnote("");
        server.add(&remembered);

        // ---- About -------------------------------------------------------------------------------
        let about = adw::PreferencesGroup::new();
        about.add(&ui::button_row(
            say.get("About Family Connect"),
            false,
            || {
                if let Some(shell) = Shell::try_get() {
                    shell
                        .app
                        .activate_action(crate::seams::actions::ABOUT.name, None);
                }
            },
        ));
        let version = ui::footnote(&rows::version_line(say.as_ref()));
        version.set_halign(gtk::Align::Center);
        version.set_xalign(0.5);
        version.set_selectable(true);
        about.add(&version);

        let page = adw::PreferencesPage::new();
        for group in [
            &identity_group,
            &profile,
            &family,
            &notifications,
            &top_bar,
            &privacy,
            &voice,
            &server,
            &about,
        ] {
            page.add(group);
        }
        // A sentence, so the page's body text rather than a headline; compact in a small window.
        let signed_out = adw::StatusPage::builder()
            .icon_name("avatar-default-symbolic")
            .description(say.get("Sign in to see your settings."))
            .build();
        signed_out.add_css_class("compact");
        let stack = gtk::Stack::new();
        stack.add_named(&page, Some("settings"));
        stack.add_named(&signed_out, Some("signed-out"));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&toasts));
        window.set_content(Some(&view));

        let settings = Rc::new(SettingsWindow {
            window,
            toasts,
            stack,
            name,
            username,
            owner,
            avatar,
            photo,
            remove_photo,
            birthday,
            birthday_button,
            family_name,
            new_family,
            join_another,
            notify,
            keep_running,
            top_bar_note,
            autostart,
            hot_key,
            hot_key_change,
            link_previews,
            map_previews,
            speed,
            camera,
            cameras: RefCell::new(Vec::new()),
            address,
            remembered,
            updating: Cell::new(false),
        });
        settings.connect_rows();
        settings.follow(shell);
        settings
    }

    /// Each row writes its key (or asks its service) when the PERSON changes it.
    fn connect_rows(self: &Rc<Self>) {
        let flag = |row: &adw::SwitchRow, key: &'static str| {
            let weak = Rc::downgrade(self);
            row.connect_active_notify(move |row| {
                if let Some(me) = weak.upgrade()
                    && !me.updating.get()
                {
                    Shell::get().settings.set_bool(key, row.is_active());
                }
            });
        };
        flag(&self.notify, keys::NOTIFY);
        flag(&self.keep_running, keys::KEEP_RUNNING);
        flag(&self.hot_key, keys::HOT_KEY);
        flag(&self.link_previews, keys::LINK_PREVIEWS);
        flag(&self.map_previews, keys::MAP_PREVIEWS);

        let weak = Rc::downgrade(self);
        self.autostart.connect_active_notify(move |row| {
            let Some(me) = weak.upgrade() else {
                return;
            };
            if me.updating.get() {
                return;
            }
            let wanted = row.is_active();
            let after = Shell::get().autostart.get().set_enabled(wanted);
            if wanted && !fc_logic::startup_setting::request_succeeded(after) {
                // Refused (the file could not be written, an administrator's entry wins): the row
                // goes back to what is true rather than pretending.
                crate::fc_info!("autostart: the request was not taken ({after:?})");
            }
            me.refresh();
        });

        let weak = Rc::downgrade(self);
        self.speed.connect_active_name_notify(move |group| {
            if let Some(me) = weak.upgrade()
                && !me.updating.get()
                && let Some(name) = group.active_name()
            {
                Shell::get().settings.set_string(keys::VOICE_SPEED, &name);
            }
        });

        let weak = Rc::downgrade(self);
        self.camera.connect_selected_notify(move |row| {
            let Some(me) = weak.upgrade() else {
                return;
            };
            if me.updating.get() {
                return;
            }
            let settings = &Shell::get().settings;
            match rows::camera_choice(&me.cameras.borrow(), row.selected()) {
                Some(id) => settings.set_string(keys::VIDEO_CAMERA, &id),
                None => settings.remove(keys::VIDEO_CAMERA),
            }
        });

        let weak = Rc::downgrade(self);
        self.birthday_button.connect_clicked(move |_| {
            let Some(me) = weak.upgrade() else {
                return;
            };
            let held = Shell::get()
                .connection()
                .and_then(|connection| connection.session.state().me)
                .and_then(|me| me.birthday);
            birthday::show(&me.window, Whose::Mine, held, || {
                if let Some(open) = current() {
                    open.refresh();
                }
            });
        });
    }

    /// Redraws on every change of what the rows read: the session, the connection, the settings
    /// file, the icon, the portal, the cameras.
    fn follow(self: &Rc<Self>, shell: &Rc<Shell>) {
        let refresh = {
            let weak = Rc::downgrade(self);
            move || {
                if let Some(me) = weak.upgrade() {
                    me.refresh();
                }
            }
        };
        let follow_session = {
            let refresh = refresh.clone();
            move |shell: &Shell| {
                if let Some(connection) = shell.connection() {
                    let refresh = refresh.clone();
                    connection
                        .session
                        .on_changed()
                        .subscribe(move |_| refresh());
                }
            }
        };
        follow_session(shell);
        {
            let refresh = refresh.clone();
            shell.on_connection_changed().subscribe(move |_| {
                if let Some(shell) = Shell::try_get() {
                    follow_session(&shell);
                }
                refresh();
            });
        }
        {
            let refresh = refresh.clone();
            shell.settings.changed().subscribe(move |_| refresh());
        }
        {
            let refresh = refresh.clone();
            shell
                .tray
                .get()
                .on_available_changed()
                .subscribe(move |_| refresh());
        }
        {
            let refresh = refresh.clone();
            shell
                .hot_key
                .get()
                .on_state_changed()
                .subscribe(move |_| refresh());
        }
        shell
            .camera
            .get()
            .on_devices_changed()
            .subscribe(move |_| refresh());
    }

    /// "Add Photo" / "Change Photo" (the Mac's `setAvatar`): a picture chosen through the portal's
    /// file chooser, made ready by P2's `attachments::avatars::prepare_upload` (fitted within 512 px,
    /// a JPEG), sent with `PUT /me/avatar`, then `/me` read again for the new version.
    pub fn choose_photo(&self) {
        let shell = Shell::get();
        let say = Rc::clone(&shell.say);
        let filter = gtk::FileFilter::new();
        filter.add_mime_type("image/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .modal(true)
            .filters(&filters)
            .build();
        let window = self.window.clone();
        ui::spawn(async move {
            let Ok(file) = dialog.open_future(Some(&window)).await else {
                return;
            };
            let Ok((bytes, _)) = file.load_contents_future().await else {
                if let Some(open) = current() {
                    ui::toast(&open.toasts, say.get("That file isn't a photo we can use."));
                }
                return;
            };
            upload_photo(bytes.to_vec()).await;
        });
    }

    /// "Remove Photo": `DELETE /me/avatar`, then `/me` again.
    pub fn remove_photo(&self) {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        self.remove_photo.set_sensitive(false);
        ui::spawn(async move {
            let answer = connection.api.delete_avatar().await.into_result();
            finish_photo(&connection, answer.err(), false).await;
        });
    }

    /// Copies what is true now onto every row.
    pub fn refresh(&self) {
        self.updating.set(true);
        let shell = Shell::get();
        let say: &dyn StringCatalog = shell.say.as_ref();
        let connection = shell.connection();
        let state = connection
            .as_ref()
            .map(|connection| connection.session.state());
        let signed_in = state.as_ref().is_some_and(|state| state.can_chat());
        self.stack
            .set_visible_child_name(if signed_in { "settings" } else { "signed-out" });

        if let (Some(connection), Some(state)) = (connection.as_ref(), state.as_ref())
            && signed_in
        {
            let me = state.me.as_ref();
            let shown = me.map_or("—", |me| me.display_name.as_str());
            self.name.set_text(shown);
            self.avatar.set_text(Some(shown));
            match me {
                Some(me) => {
                    crate::attachments::avatars::show(&self.avatar, me.id, me.avatar_version)
                }
                None => crate::attachments::avatars::show(&self.avatar, 0, 0),
            }
            let pictured = me.is_some_and(|me| me.avatar_version > 0);
            self.photo.set_label(if pictured {
                say.get("Change Photo")
            } else {
                say.get("Add Photo")
            });
            self.remove_photo.set_visible(pictured);
            self.username
                .set_text(&me.map(|me| format!("@{}", me.username)).unwrap_or_default());
            self.owner.set_visible(state.is_owner());
            let held = me.and_then(|me| me.birthday);
            self.birthday
                .set_subtitle(&birthday_rules::text(held.as_ref(), say, &shell.culture));
            self.birthday_button.set_label(if held.is_some() {
                say.get("Change Birthday…")
            } else {
                say.get("Add Birthday…")
            });
            self.family_name.set_subtitle(
                state
                    .family
                    .as_ref()
                    .map_or("—", |family| family.name.as_str()),
            );
            self.new_family
                .set_visible(family_rules::offers_new_family(state));
            self.join_another
                .set_visible(family_rules::offers_join_another(state));
            self.address
                .set_subtitle(crate::seams::tokens::origin_of(&connection.server).as_str());
            let remembered = rows::remembered_line(connection.tokens.remembered(), say);
            self.remembered
                .set_text(remembered.as_deref().unwrap_or_default());
            self.remembered.set_visible(remembered.is_some());
        }

        let settings = &shell.settings;
        self.notify.set_active(settings.flag(keys::NOTIFY));
        let tray_available = shell.tray.get().available();
        apply(
            &self.keep_running,
            &rows::keep_running(settings.flag(keys::KEEP_RUNNING), tray_available, say),
        );
        self.top_bar_note.set_visible(tray_available);
        apply(
            &self.autostart,
            &rows::start_at_login(shell.autostart.get().state(), say),
        );
        apply(
            &self.hot_key,
            &rows::hot_key(
                &shell.hot_key.get().state(),
                settings.flag(keys::HOT_KEY),
                say,
            ),
        );
        self.hot_key_change
            .set_visible(shell.hot_key.get().can_configure());
        self.link_previews
            .set_active(settings.flag(keys::LINK_PREVIEWS));
        self.map_previews
            .set_active(settings.flag(keys::MAP_PREVIEWS));
        let speed = rows::speed_selected(settings.string(keys::VOICE_SPEED).as_deref());
        if self.speed.active_name().as_deref() != Some(speed.as_str()) {
            self.speed.set_active_name(Some(&speed));
        }
        let devices = shell.camera.get().devices();
        let (names, selected) = rows::camera_rows(
            &devices,
            settings.string(keys::VIDEO_CAMERA).as_deref(),
            say,
        );
        if *self.cameras.borrow() != devices || self.camera.model().is_none() {
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            self.camera.set_model(Some(&gtk::StringList::new(&names)));
            *self.cameras.borrow_mut() = devices;
        }
        if self.camera.selected() != selected {
            self.camera.set_selected(selected);
        }
        self.updating.set(false);
    }

    /// Log Out asks first (the Mac's question; what is unsent is lost — §13), then signs out: the
    /// gate goes to the sign-in page and this window says to sign in.
    pub fn ask_log_out(&self) {
        let shell = Shell::get();
        let Some(connection) = shell.connection() else {
            return;
        };
        let unsent = connection
            .outbox
            .all()
            .map(|rows| !rows.is_empty())
            .unwrap_or(false);
        let say: &dyn StringCatalog = shell.say.as_ref();
        ui::ask(
            &self.window,
            say.get("Log out?"),
            &rows::logout_message(unsent, say),
            say.get("Cancel"),
            say.get("Log Out"),
            true,
            sign_out,
        );
    }
}

/// Signs out (Log Out's answer). The session publishes the signed-out gate; the server is told,
/// but a network that cannot be reached changes nothing here (`AppSession::sign_out`).
pub fn sign_out() {
    let Some(connection) = Shell::get().connection() else {
        return;
    };
    ui::spawn(async move {
        if let Err(error) = connection.session.sign_out().await {
            crate::fc_warn!(
                "sign out: the cache refused ({})",
                crate::services::log::store_error(&error)
            );
        }
        crate::fc_info!("signed out from Settings");
    });
}

fn apply(row: &adw::SwitchRow, look: &rows::SwitchLook) {
    row.set_visible(look.visible);
    row.set_title(&look.title);
    row.set_active(look.active);
    row.set_sensitive(look.sensitive);
    row.set_subtitle(&look.subtitle);
}

/// A row that opens a page in the browser (`gtk::UriLauncher`).
fn link_row(title: &str, url: &'static str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
    row.connect_activated(move |row| {
        let window = row.root().and_downcast::<gtk::Window>();
        gtk::UriLauncher::new(url).launch(window.as_ref(), None::<&gio::Cancellable>, |_| {});
    });
    row
}

/// A picture's bytes made into a profile photo and sent (`choose_photo`; the self-test hands bytes
/// here directly — Xvfb has no file chooser).
pub async fn upload_photo(bytes: Vec<u8>) {
    let shell = Shell::get();
    let Some(connection) = shell.connection() else {
        return;
    };
    let Some((bytes, mime)) = crate::attachments::avatars::prepare_upload(bytes).await else {
        if let Some(open) = current() {
            ui::toast(
                &open.toasts,
                shell.say.get("That file isn't a photo we can use."),
            );
        }
        return;
    };
    if let Some(open) = current() {
        open.photo.set_sensitive(false);
    }
    let answer = connection.api.put_avatar(bytes, mime).await.into_result();
    finish_photo(&connection, answer.err(), true).await;
}

/// After a photo went up or came down: `/me` read again (the new `avatar_version`), or the refusal
/// said (`settings_text::picture_failure`).
async fn finish_photo(
    connection: &Rc<crate::shell::Connection>,
    error: Option<fc_core::protocol::ApiError>,
    uploading: bool,
) {
    let shell = Shell::get();
    match error {
        None => {
            crate::fc_info!(
                "profile photo: {}",
                if uploading { "set" } else { "removed" }
            );
            if let Err(error) = connection.session.refresh().await {
                crate::fc_warn!(
                    "profile photo: the cache refused ({})",
                    crate::services::log::store_error(&error)
                );
            }
        }
        Some(error) => {
            crate::fc_info!("profile photo: refused ({})", error.code);
            if let Some(open) = current() {
                ui::toast(
                    &open.toasts,
                    &fc_logic::settings::settings_text::picture_failure(
                        &error,
                        uploading,
                        shell.say.as_ref(),
                    ),
                );
            }
        }
    }
    if let Some(open) = current() {
        open.photo.set_sensitive(true);
        open.remove_photo.set_sensitive(true);
        open.refresh();
    }
}
