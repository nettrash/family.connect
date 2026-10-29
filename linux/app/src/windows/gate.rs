//! P1's way in (W.2) and the session gate's pages (shell-design §3.6).
//!
//! [`install`] is the package's one line in `app::install_packages`: it registers P1's CSS, puts the
//! chats ([`crate::chats::Chats`]) into `Shell::chat_routes` and adds two window parts — the gate's
//! pages (this module) and the chats (the sidebar and the conversation).
//!
//! **The pages** fill `MainWindow::page_slot(GatePage)`; each is rebuilt every time the gate shows
//! it, so it reads the session as it is then (a family name, the server's switches):
//!
//! | Page | Mac | Windows | Here |
//! | --- | --- | --- | --- |
//! | Booting | spinner | — | the app's icon (`hero`) and an `adw::Spinner` in the gate's form |
//! | Server | `ServerSetupView` | `ServerView` | address row; `doorway::server_check` probes `GET /me` with no token before anything is saved |
//! | SignIn | `AuthView` | `SignInView` | Log In / Register on one page (`adw::ToggleGroup`), `sign_in_form::may_submit`, refusals by `door_sentences::sign_in`; "Change server…" |
//! | Offline | — | `OfflineView` | "a flaky network is not a sign-out": Try Again (`Shell::retry`), Change server… |
//! | Family | `FamilyGateView` | `DoorView` | join with a code / create (only where `family_registration_enabled`) through `FamilyDoor`; the declined line; the familyless-account footnote; Log out |
//! | Pending | `PendingApprovalView` | `PendingView` | the family's name, Log out, `GET /me` again every 15 s while shown |
//!
//! "Change server…" swaps the page's own slot to the server form (Cancel brings the page back): the
//! gate has no "choosing a server" state of its own, and none is needed — a server that answers is
//! opened with `Shell::open_server`, which shows Booting at once.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_core::rt::Http;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::doorway::{FamilyDoor, door_sentences, server_check, sign_in_form};

use crate::seams::routes::WindowPart;
use crate::services::log;
use crate::shell::{BRAND, DEFAULT_SERVER, Shell};
use crate::windows::main_window::{GatePage, MainWindow};

/// How often the Pending page asks `GET /me` again (Windows' `PendingView`: an approval raises no
/// frame this device can hear — it has no family socket yet).
const PENDING_ASK_EVERY: Duration = Duration::from_secs(15);

/// P1's `install` (W.3): CSS, chat routes, the gate's pages and the chats.
pub fn install(shell: &Rc<Shell>) {
    crate::theme::register(crate::conversation::style::css);
    let chats = crate::chats::Chats::new();
    shell.chat_routes.install(chats.clone());
    shell.add_window_part(Rc::new(GatePart::default()));
    shell.add_window_part(chats);
}

/// The gate's pages as one window part.
#[derive(Default)]
struct GatePart {
    /// The Pending page's timer while it is shown.
    pending_timer: Rc<RefCell<Option<glib::SourceId>>>,
}

impl WindowPart for GatePart {
    fn attach(&self, window: &Rc<MainWindow>) {
        let shell = Shell::get();
        // Built once now for whatever is shown, then on every page change.
        for page in [
            GatePage::Booting,
            GatePage::Server,
            GatePage::SignIn,
            GatePage::Offline,
            GatePage::Family,
            GatePage::Pending,
        ] {
            fill(&shell, window, page);
        }
        let weak = Rc::downgrade(window);
        let timer = Rc::clone(&self.pending_timer);
        window.on_page_changed().subscribe(move |page| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let shell = Shell::get();
            if let Some(source) = timer.borrow_mut().take() {
                source.remove();
            }
            if *page != GatePage::Chats {
                fill(&shell, &window, *page);
            }
            if *page == GatePage::Pending {
                let source = glib::timeout_add_local(PENDING_ASK_EVERY, || {
                    let shell = Shell::get();
                    if let Some(connection) = shell.connection() {
                        glib::spawn_future_local(async move {
                            if let Err(error) = connection.session.refresh().await {
                                crate::fc_warn!(
                                    "session: the cache refused ({})",
                                    log::store_error(&error)
                                );
                            }
                        });
                    }
                    glib::ControlFlow::Continue
                });
                *timer.borrow_mut() = Some(source);
            }
        });
    }
}

/// (Re)builds one gate page in its slot.
fn fill(shell: &Rc<Shell>, window: &Rc<MainWindow>, page: GatePage) {
    let slot = window.page_slot(page);
    let widget = match page {
        GatePage::Booting => booting_page(),
        GatePage::Server => server_page(shell, &slot, None),
        GatePage::SignIn => sign_in_page(shell, &slot),
        GatePage::Offline => offline_page(shell, &slot),
        GatePage::Family => family_page(shell),
        GatePage::Pending => pending_page(shell),
        GatePage::Chats => return,
    };
    slot.set_child(Some(&widget));
}

/// The app's own icon: the product's face on the pages a person meets before any chat. Embedded,
/// because a binary run from a checkout has no icon theme to find it in; the theme's copy (same
/// name) is the fallback should the bytes ever not decode.
const BRAND_ICON: &[u8] =
    include_bytes!("../../../packaging/icons/hicolor/256x256/apps/me.nettrash.FamilyConnect.png");

/// How big the icon stands over a doorway page's title.
const BRAND_SIZE: i32 = 96;

/// The icon, then `title` and the page's one-line `lede` under it: one centred group, the page's
/// top. Without a title it is the icon alone (Booting's splash, where the next page's icon lands).
fn hero(title: Option<&gtk::Label>, lede: Option<&str>) -> gtk::Box {
    let texture = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(BRAND_ICON)).ok();
    let builder = gtk::Image::builder()
        .pixel_size(BRAND_SIZE)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .css_classes(["icon-dropshadow", "fc-chat-brand"]);
    let mark = match texture {
        Some(texture) => builder.paintable(&texture).build(),
        None => builder.icon_name(crate::app::APP_ID).build(),
    };
    let group = gtk::Box::new(gtk::Orientation::Vertical, 6);
    group.append(&mark);
    if let Some(title) = title {
        title.set_margin_top(6);
        group.append(title);
    }
    if let Some(lede) = lede {
        // At its natural width, so a long sentence wraps into balanced lines under the title.
        group.append(
            &gtk::Label::builder()
                .label(lede)
                .wrap(true)
                .justify(gtk::Justification::Center)
                .halign(gtk::Align::Center)
                .max_width_chars(40)
                .build(),
        );
    }
    group.set_margin_bottom(6);
    group
}

/// A form in the middle of the page, at most 360 wide, scrolling when the window is short.
fn form(content: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let clamp = adw::Clamp::builder()
        .maximum_size(360)
        .tightening_threshold(300)
        .child(content)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(16)
        .margin_end(16)
        .valign(gtk::Align::Center)
        .build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .vexpand(true)
        .build()
        .upcast()
}

fn column() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 18)
}

fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .css_classes(["title-1"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build()
}

fn note(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .css_classes(["dim-label", "caption"])
        .wrap(true)
        .xalign(0.0)
        .build()
}

/// A refusal under a form: red, wrapping, hidden until there is something to say.
fn error_line() -> gtk::Label {
    gtk::Label::builder()
        .css_classes(["error"])
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build()
}

fn say_error(label: &gtk::Label, text: Option<&str>) {
    match text {
        Some(text) => {
            label.set_label(text);
            label.set_visible(true);
            label.announce(text, gtk::AccessibleAnnouncementPriority::Medium);
        }
        None => label.set_visible(false),
    }
}

/// A pill button; `busy` puts a spinner beside its words.
fn action_button(text: &str, suggested: bool) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(text)
        .css_classes(["pill"])
        .halign(gtk::Align::Center)
        .build();
    if suggested {
        button.add_css_class("suggested-action");
    }
    button
}

/// The icon and a spinner, where the next page's icon will stand: a launch opens on the app's
/// face, not on an empty window.
fn booting_page() -> gtk::Widget {
    let content = column();
    content.append(&hero(None, None));
    content.append(
        &adw::Spinner::builder()
            .width_request(24)
            .height_request(24)
            .halign(gtk::Align::Center)
            .build(),
    );
    form(&content)
}

/// The server form. `back`: what Cancel puts back in `slot` (the page "Change server…" was
/// pressed on); `None` on the Server page itself, which has no Cancel.
fn server_page(shell: &Rc<Shell>, slot: &adw::Bin, back: Option<gtk::Widget>) -> gtk::Widget {
    let say = &shell.say;
    let content = column();
    // The product's name, not a sentence: the same in every language.
    content.append(&hero(
        Some(&heading(BRAND)),
        Some(say.get("Ask the family member who runs the server for its address.")),
    ));
    let group = adw::PreferencesGroup::new();
    let address = adw::EntryRow::builder()
        .title(say.get("Server address"))
        .input_purpose(gtk::InputPurpose::Url)
        .build();
    let prefill = shell
        .connection()
        .map(|connection| connection.server.to_string())
        .or_else(|| shell.saved_server().map(|server| server.to_string()))
        .or_else(|| DEFAULT_SERVER.map(str::to_string))
        .unwrap_or_default();
    address.set_text(&prefill);
    group.add(&address);
    content.append(&group);
    let warning = note(say.get(
        "This address uses plain http. That's fine for a server on your home network, but anyone on the same network can read the traffic.",
    ));
    content.append(&warning);
    let error = error_line();
    content.append(&error);
    let connect = action_button(say.get("Connect"), true);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::Center);
    if let Some(back) = back {
        let cancel = action_button(say.get("Cancel"), false);
        let slot = slot.clone();
        cancel.connect_clicked(move |_| slot.set_child(Some(&back)));
        buttons.append(&cancel);
    }
    buttons.append(&connect);
    content.append(&buttons);

    let working = Rc::new(Cell::new(false));
    let assess = {
        let (address, warning, connect, working) = (
            address.clone(),
            warning.clone(),
            connect.clone(),
            Rc::clone(&working),
        );
        Rc::new(move || {
            let text = address.text();
            let url = server_check::parse(Some(text.as_str()));
            warning.set_visible(url.as_ref().is_some_and(server_check::is_plain_text));
            connect.set_sensitive(!working.get() && !text.trim().is_empty());
        })
    };
    assess();
    {
        let assess = Rc::clone(&assess);
        address.connect_changed(move |_| assess());
    }
    let go = {
        let (shell, address, error, connect) = (
            Rc::clone(shell),
            address.clone(),
            error.clone(),
            connect.clone(),
        );
        Rc::new(move || {
            let typed = address.text().to_string();
            if working.get() || typed.trim().is_empty() {
                return;
            }
            say_error(&error, None);
            let url = server_check::parse(Some(&typed));
            let (shell, error, connect, working, assess) = (
                Rc::clone(&shell),
                error.clone(),
                connect.clone(),
                Rc::clone(&working),
                Rc::clone(&assess),
            );
            working.set(true);
            connect.set_label(shell.say.get("Checking…"));
            assess();
            glib::spawn_future_local(async move {
                let answers = match &url {
                    Some(url) => {
                        let http: Rc<dyn Http> = shell.http.clone();
                        server_check::answers(http, url.clone(), shell.clock.clone()).await
                    }
                    None => false,
                };
                working.set(false);
                connect.set_label(shell.say.get("Connect"));
                assess();
                match url {
                    Some(url) if answers => shell.open_server(url),
                    url => {
                        crate::fc_info!("server: no Family Connect server answered");
                        let shown = url.map_or_else(|| typed.trim().to_string(), |u| u.to_string());
                        say_error(
                            &error,
                            Some(&shell.say.format(
                                "No Family Connect server answered at %@. Check the address and your network.",
                                &[&shown],
                            )),
                        );
                    }
                }
            });
        })
    };
    {
        let go = Rc::clone(&go);
        address.connect_entry_activated(move |_| go());
    }
    connect.connect_clicked(move |_| go());
    let page = form(&content);
    let focus = address.clone();
    page.connect_map(move |_| {
        focus.grab_focus();
    });
    page
}

/// "Change server…": the server form in this page's slot, Cancel bringing the page back.
fn change_server(shell: &Rc<Shell>, slot: &adw::Bin) {
    let back = slot.child();
    let page = server_page(shell, slot, back);
    slot.set_child(Some(&page));
}

fn change_server_button(shell: &Rc<Shell>, slot: &adw::Bin) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(shell.say.get("Change server…"))
        .css_classes(["flat"])
        .halign(gtk::Align::Center)
        .build();
    let (shell, slot) = (Rc::clone(shell), slot.clone());
    button.connect_clicked(move |_| change_server(&shell, &slot));
    button
}

/// Log In or Register on one page: a family app has exactly two entry stories, and both stay
/// visible rather than one hiding behind a link (Windows' `SignInView`).
fn sign_in_page(shell: &Rc<Shell>, slot: &adw::Bin) -> gtk::Widget {
    let say = &shell.say;
    let content = column();
    let title = heading(say.get("Welcome Back"));
    content.append(&hero(Some(&title), None));
    // The page's two doors, as wide as the form under them.
    let modes = adw::ToggleGroup::builder().homogeneous(true).build();
    modes.add(
        adw::Toggle::builder()
            .name("log-in")
            .label(say.get("Log In"))
            .build(),
    );
    modes.add(
        adw::Toggle::builder()
            .name("register")
            .label(say.get("Register"))
            .build(),
    );
    modes.set_active_name(Some("log-in"));
    content.append(&modes);
    let group = adw::PreferencesGroup::new();
    let username = adw::EntryRow::builder().title(say.get("Username")).build();
    let display_name = adw::EntryRow::builder()
        .title(say.get("Display name"))
        .visible(false)
        .build();
    let password = adw::PasswordEntryRow::builder()
        .title(say.get("Password"))
        .build();
    group.add(&username);
    group.add(&display_name);
    group.add(&password);
    content.append(&group);
    let rules = note(say.get(
        "Usernames are 3–32 letters, digits, dots or underscores. Passwords need at least 8 characters.",
    ));
    rules.set_visible(false);
    content.append(&rules);
    let error = error_line();
    content.append(&error);
    let submit = action_button(say.get("Log In"), true);
    content.append(&submit);
    // The server and the way to another one: one footer, apart from the form above.
    let footer = gtk::Box::new(gtk::Orientation::Vertical, 2);
    footer.set_margin_top(12);
    if let Some(connection) = shell.connection() {
        let server = note(&say.format("Connected to %@", &[&connection.server.to_string()]));
        server.set_xalign(0.5);
        server.set_selectable(true);
        footer.append(&server);
    }
    footer.append(&change_server_button(shell, slot));
    content.append(&footer);

    let working = Rc::new(Cell::new(false));
    let arrange = {
        let (shell, modes, title, display_name, rules, error, submit, username, password) = (
            Rc::clone(shell),
            modes.clone(),
            title.clone(),
            display_name.clone(),
            rules.clone(),
            error.clone(),
            submit.clone(),
            username.clone(),
            password.clone(),
        );
        let working = Rc::clone(&working);
        Rc::new(move || {
            let say = &shell.say;
            let registering = modes.active_name().as_deref() == Some("register");
            title.set_label(if registering {
                say.get("Join the Family")
            } else {
                say.get("Welcome Back")
            });
            display_name.set_visible(registering);
            rules.set_visible(registering && !error.is_visible());
            submit.set_label(match (working.get(), registering) {
                (true, true) => say.get("Creating account…"),
                (true, false) => say.get("Logging in…"),
                (false, true) => say.get("Create Account"),
                (false, false) => say.get("Log In"),
            });
            submit.set_sensitive(
                !working.get()
                    && sign_in_form::may_submit(
                        Some(username.text().as_str()),
                        Some(password.text().as_str()),
                        Some(display_name.text().as_str()),
                        registering,
                    ),
            );
        })
    };
    arrange();
    {
        let arrange = Rc::clone(&arrange);
        modes.connect_active_name_notify(move |_| arrange());
    }
    for row in [&username, &display_name] {
        let arrange = Rc::clone(&arrange);
        row.connect_changed(move |_| arrange());
    }
    {
        let arrange = Rc::clone(&arrange);
        password.connect_changed(move |_| arrange());
    }
    let submit_now = {
        let (shell, modes, username, display_name, password, error) = (
            Rc::clone(shell),
            modes.clone(),
            username.clone(),
            display_name.clone(),
            password.clone(),
            error.clone(),
        );
        let (working, arrange) = (Rc::clone(&working), Rc::clone(&arrange));
        Rc::new(move || {
            let registering = modes.active_name().as_deref() == Some("register");
            let (name, shown, secret) = (
                username.text().trim().to_string(),
                display_name.text().trim().to_string(),
                password.text().to_string(),
            );
            if working.get()
                || !sign_in_form::may_submit(Some(&name), Some(&secret), Some(&shown), registering)
            {
                return;
            }
            let Some(connection) = shell.connection() else {
                return;
            };
            say_error(&error, None);
            working.set(true);
            arrange();
            let (shell, error, working, arrange) = (
                Rc::clone(&shell),
                error.clone(),
                Rc::clone(&working),
                Rc::clone(&arrange),
            );
            glib::spawn_future_local(async move {
                let answer = if registering {
                    connection.session.register(&name, &shown, &secret).await
                } else {
                    connection.session.sign_in(&name, &secret).await
                };
                working.set(false);
                match answer {
                    // On success the gate moves and this page is replaced; only a refusal stays.
                    Ok(Some(refused)) => {
                        crate::fc_info!("sign-in: refused ({})", refused.code);
                        say_error(
                            &error,
                            Some(&door_sentences::sign_in(
                                &refused,
                                registering,
                                shell.say.as_ref(),
                            )),
                        );
                    }
                    Ok(None) => {}
                    Err(failure) => {
                        crate::fc_warn!(
                            "sign-in: the cache refused ({})",
                            log::store_error(&failure)
                        );
                        say_error(
                            &error,
                            Some(shell.say.get("Something went wrong. Try again.")),
                        );
                    }
                }
                arrange();
            });
        })
    };
    // Enter walks the fields, and the last one submits.
    {
        let (modes, display_name, password) =
            (modes.clone(), display_name.clone(), password.clone());
        username.connect_entry_activated(move |_| {
            if modes.active_name().as_deref() == Some("register") {
                display_name.grab_focus();
            } else {
                password.grab_focus();
            }
        });
    }
    {
        let password = password.clone();
        display_name.connect_entry_activated(move |_| {
            password.grab_focus();
        });
    }
    {
        let submit_now = Rc::clone(&submit_now);
        password.connect_entry_activated(move |_| submit_now());
    }
    submit.connect_clicked(move |_| submit_now());
    let page = form(&content);
    let focus = username.clone();
    page.connect_map(move |_| {
        focus.grab_focus();
    });
    page
}

/// Signed in, and the server cannot be reached at launch: not the sign-in form — a flaky network
/// is not a sign-out (Windows' `OfflineView`).
fn offline_page(shell: &Rc<Shell>, slot: &adw::Bin) -> gtk::Widget {
    let say = &shell.say;
    // One row of two pills, the primary action at the trailing end (GNOME's order for a pair of
    // actions) — and last in the row, where the self-test looks for it.
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::Center);
    buttons.add_css_class("fc-chat-gate-actions");
    let again = action_button(say.get("Try Again"), true);
    let weak = Rc::downgrade(shell);
    again.connect_clicked(move |_| {
        if let Some(shell) = weak.upgrade() {
            shell.retry();
        }
    });
    let change = change_server_button(shell, slot);
    change.remove_css_class("flat");
    change.add_css_class("pill");
    buttons.append(&change);
    buttons.append(&again);
    adw::StatusPage::builder()
        .icon_name("network-offline-symbolic")
        .title(say.get("Offline"))
        .description(say.get("Can't reach the server. Check your connection."))
        .child(&buttons)
        .build()
        .upcast()
}

/// "Log out" — the gate's way back to the sign-in form: quiet, like "Change server…", since
/// nothing is lost by it and it is never the page's purpose.
fn log_out_button(shell: &Rc<Shell>) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(shell.say.get("Log out"))
        .css_classes(["flat"])
        .halign(gtk::Align::Center)
        .build();
    button.connect_clicked(|_| {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        glib::spawn_future_local(async move {
            if let Err(error) = connection.session.sign_out().await {
                crate::fc_warn!("sign-out: the cache refused ({})", log::store_error(&error));
            }
        });
    });
    button
}

/// One of the family door's two forms: a row to type in, a button, a refusal line.
struct DoorForm {
    group: adw::PreferencesGroup,
    entry: adw::EntryRow,
    button: gtk::Button,
    error: gtk::Label,
}

fn door_form(title: &str, description: &str, field: &str, button: &str) -> DoorForm {
    let group = adw::PreferencesGroup::builder()
        .title(title)
        .description(description)
        .build();
    let entry = adw::EntryRow::builder().title(field).build();
    let button = gtk::Button::builder()
        .label(button)
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .build();
    entry.add_suffix(&button);
    group.add(&entry);
    let error = error_line();
    error.set_margin_top(6);
    group.add(&error);
    DoorForm {
        group,
        entry,
        button,
        error,
    }
}

/// Signed in and in no family: join one with a code, or — where the server allows — start one.
/// Also the body of "New Family…" / "Join Another Family…" (`chats::switcher`): `which` picks the
/// forms drawn there.
pub fn family_forms(shell: &Rc<Shell>, which: FamilyForms, done: Rc<dyn Fn()>) -> gtk::Box {
    let say = &shell.say;
    // GNOME's spacing between preference groups: two forms, not one list.
    let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
    let Some(connection) = shell.connection() else {
        return content;
    };
    let say_dyn: Rc<dyn StringCatalog> = shell.say.clone();
    let door = Rc::new(FamilyDoor::new(
        Rc::clone(&connection.api),
        Rc::clone(&connection.session),
        say_dyn,
    ));
    let state = connection.session.state();
    let busy = Rc::new(Cell::new(false));
    if which.join {
        let join = door_form(
            say.get("Join a family"),
            say.get("Enter the invite code a family member shared with you."),
            say.get("Invite code"),
            say.get("Join"),
        );
        join.button.set_sensitive(false);
        {
            let button = join.button.clone();
            join.entry.connect_changed(move |entry| {
                button.set_sensitive(FamilyDoor::may_join(Some(entry.text().as_str())));
            });
        }
        let go = {
            let (shell, door, entry, button, error, busy, done) = (
                Rc::clone(shell),
                Rc::clone(&door),
                join.entry.clone(),
                join.button.clone(),
                join.error.clone(),
                Rc::clone(&busy),
                Rc::clone(&done),
            );
            Rc::new(move || {
                let code = entry.text().to_string();
                if busy.get() || !FamilyDoor::may_join(Some(&code)) {
                    return;
                }
                busy.set(true);
                say_error(&error, None);
                button.set_label(shell.say.get("Joining…"));
                button.set_sensitive(false);
                let (shell, door, button, error, busy, done) = (
                    Rc::clone(&shell),
                    Rc::clone(&door),
                    button.clone(),
                    error.clone(),
                    Rc::clone(&busy),
                    Rc::clone(&done),
                );
                glib::spawn_future_local(async move {
                    let answer = door.join(&code).await;
                    busy.set(false);
                    button.set_label(shell.say.get("Join"));
                    button.set_sensitive(true);
                    match answer {
                        Ok(None) => done(),
                        Ok(Some(refused)) => say_error(&error, Some(&refused)),
                        Err(failure) => crate::fc_warn!(
                            "join: the cache refused ({})",
                            log::store_error(&failure)
                        ),
                    }
                });
            })
        };
        {
            let go = Rc::clone(&go);
            join.entry.connect_entry_activated(move |_| go());
        }
        join.button.connect_clicked(move |_| go());
        content.append(&join.group);
    }
    if which.create {
        if state.family_registration_enabled || which.another {
            let (title, description) = if which.another {
                (
                    say.get("New Family…").to_string(),
                    fc_logic::families::another_family::explanation(say.as_ref()),
                )
            } else {
                (
                    say.get("Create a family").to_string(),
                    say.get("Start fresh — you'll be the owner and can invite everyone else.")
                        .to_string(),
                )
            };
            let create = door_form(
                &title,
                &description,
                say.get("Family name"),
                say.get("Create Family"),
            );
            // One suggested action per view: beside the join form, joining is it.
            if which.join {
                create.button.remove_css_class("suggested-action");
            }
            create.button.set_sensitive(false);
            {
                let button = create.button.clone();
                create.entry.connect_changed(move |entry| {
                    button.set_sensitive(FamilyDoor::may_create(Some(entry.text().as_str())));
                });
            }
            let go = {
                let (shell, door, entry, button, error, busy, done) = (
                    Rc::clone(shell),
                    Rc::clone(&door),
                    create.entry.clone(),
                    create.button.clone(),
                    create.error.clone(),
                    Rc::clone(&busy),
                    Rc::clone(&done),
                );
                Rc::new(move || {
                    let name = entry.text().to_string();
                    if busy.get() || !FamilyDoor::may_create(Some(&name)) {
                        return;
                    }
                    busy.set(true);
                    say_error(&error, None);
                    button.set_label(shell.say.get("Creating…"));
                    button.set_sensitive(false);
                    let (shell, door, button, error, busy, done) = (
                        Rc::clone(&shell),
                        Rc::clone(&door),
                        button.clone(),
                        error.clone(),
                        Rc::clone(&busy),
                        Rc::clone(&done),
                    );
                    glib::spawn_future_local(async move {
                        let answer = door.create(&name).await;
                        busy.set(false);
                        button.set_label(shell.say.get("Create Family"));
                        button.set_sensitive(true);
                        match answer {
                            Ok(None) => done(),
                            Ok(Some(refused)) => say_error(&error, Some(&refused)),
                            Err(failure) => crate::fc_warn!(
                                "create family: the cache refused ({})",
                                log::store_error(&failure)
                            ),
                        }
                    });
                })
            };
            {
                let go = Rc::clone(&go);
                create.entry.connect_entry_activated(move |_| go());
            }
            create.button.connect_clicked(move |_| go());
            content.append(&create.group);
        } else {
            // The door a closed server has shut is not drawn as a door (docs/protocol.md,
            // "Starting a family").
            let closed = adw::PreferencesGroup::builder()
                .title(say.get("This server doesn't take new families."))
                .description(say.get(
                    "Family Connect is built for one family on a server of its own. To start yours, run your own server and invite everyone from there.",
                ))
                .build();
            content.append(&closed);
        }
    }
    content
}

/// Which of the family door's forms [`family_forms`] draws.
#[derive(Debug, Clone, Copy)]
pub struct FamilyForms {
    pub join: bool,
    pub create: bool,
    /// From inside a family ("New Family…", "Join Another Family…").
    pub another: bool,
}

fn family_page(shell: &Rc<Shell>) -> gtk::Widget {
    let say = &shell.say;
    let content = column();
    let Some(connection) = shell.connection() else {
        return form(&content);
    };
    let state = connection.session.state();
    if let Some(me) = state.me.as_ref() {
        content.append(&hero(
            Some(&heading(&say.format("Hi, %@", &[&me.display_name]))),
            None,
        ));
    }
    if state.join_declined {
        let declined = gtk::Label::builder()
            .label(say.get(
                "Your request to join was declined. You can ask for a new invite code and try again.",
            ))
            .css_classes(["warning"])
            .wrap(true)
            .xalign(0.0)
            .build();
        content.append(&declined);
    }
    content.append(&family_forms(
        shell,
        FamilyForms {
            join: true,
            create: true,
            another: false,
        },
        Rc::new(|| {}),
    ));
    if state.familyless_account_ttl_days > 0 {
        let days = i64::from(state.familyless_account_ttl_days);
        content.append(&note(&say.plural(
            "An account that doesn't join a family within %lld days is removed from this server.",
            days,
            &[&days],
        )));
    }
    content.append(&log_out_button(shell));
    form(&content)
}

/// A join request waiting for an owner (the Mac's `PendingApprovalView`).
fn pending_page(shell: &Rc<Shell>) -> gtk::Widget {
    let say = &shell.say;
    let family = shell
        .connection()
        .and_then(|connection| connection.session.state().pending_family_name)
        .unwrap_or_default();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    // The family's own name is data, not a sentence.
    if !family.is_empty() {
        content.append(
            &gtk::Label::builder()
                .label(&family)
                .css_classes(["title-3"])
                .wrap(true)
                .build(),
        );
    }
    content.append(
        &gtk::Label::builder()
            .label(say.get(
                "The family owner needs to approve your request. This screen updates automatically — or pull down to check right now.",
            ))
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .build(),
    );
    content.append(&log_out_button(shell));
    adw::StatusPage::builder()
        // A speech bubble holding its breath: waiting for an answer, in the app's own language.
        .icon_name("user-status-pending-symbolic")
        .title(say.get("Waiting for approval"))
        .child(&content)
        .build()
        .upcast()
}

/// The server the address field holds, as the probe would read it (tests).
#[cfg(test)]
fn parsed(typed: &str) -> Option<fc_core::protocol::ServerUrl> {
    server_check::parse(Some(typed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_field_is_read_by_the_logic() {
        assert!(parsed("").is_none());
        let plain = parsed("http://192.168.1.4:8080").expect("an address");
        assert!(server_check::is_plain_text(&plain));
        let secure = parsed("chat.example.com").expect("an address");
        assert!(!server_check::is_plain_text(&secure));
    }
}
