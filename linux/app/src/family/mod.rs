//! The Family dialog and the owner's console (shell-design §12; the Mac's `MacFamilyView`): an
//! `adw::Dialog` (content 520 × 520) over the main window — who is in the family, and for the
//! owner the invite code, invitations, join requests, reports, the join policy and the member
//! limit. Owner-only controls are HIDDEN for members, not disabled (a greyed-out row invites a
//! click that will never work).
//!
//! The dialog follows the session: a switch to another family (the sidebar's switcher, a
//! notification's route) redraws it for that family. Every rule is fc-logic's — `family_text`
//! (roster order, member tools, refusals), `family_rules` (doors, resets), `FamilyModel` (the
//! roster, blocks, approvals, leaving), `house_rules` and `fc_text::account` (policy, cap).
//!
//! | Module | What |
//! | --- | --- |
//! | [`birthday`] | the birthday editor (Settings' own, the owner's for any member) |
//! | [`groups`] | New Group and Group Info (`GroupModel`, `group_rules`) |
//! | [`member`] | a member row's menu: Message, Report…, Block, Birthday…, Reset Password…, Remove |
//! | [`words`] | the sentences fc-logic looks up through a variable, said literally (pure, tested) |

pub mod assistant;
pub mod birthday;
pub mod groups;
pub mod member;
pub mod words;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{FamilyPatch, InvitationDto, JoinRequestDto, MemberDto, ReportDto};
use fc_core::text::house_rules;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::cap_draft::CapDraft;
use fc_logic::families::{InviteModel, family_rules, family_sentences};
use fc_logic::family::FamilyModel;
use fc_logic::family_console::family_text;
use fc_logic::settings::birthday_rules;

use crate::services::log;
use crate::settings::ui;
use crate::shell::{Connection, Shell};

/// The dialog's content size (the Mac's sheet).
pub const SIZE: (i32, i32) = (520, 520);
thread_local! {
    static OPEN: RefCell<Option<Rc<FamilyDialog>>> = const { RefCell::new(None) };
}

/// `app.open-family`: the Family dialog over the main window (raised again when open).
pub fn open() {
    if let Some(open) = current() {
        open.reload();
        return;
    }
    let Some(parent) = ui::main_parent() else {
        return;
    };
    let dialog = FamilyDialog::build();
    OPEN.with(|open| *open.borrow_mut() = Some(Rc::clone(&dialog)));
    dialog.dialog.present(Some(&parent));
    dialog.rebuild();
    dialog.reload();
}

/// The dialog while it is open.
pub fn current() -> Option<Rc<FamilyDialog>> {
    OPEN.with(|open| open.borrow().clone())
}

/// What the owner's console read from the server (never cached on disk).
#[derive(Debug, Default)]
struct Loaded {
    requests: Vec<JoinRequestDto>,
    reports: Vec<ReportDto>,
    invitations: Vec<InvitationDto>,
}

/// The Family dialog (see the module's comment).
pub struct FamilyDialog {
    pub dialog: adw::Dialog,
    pub toasts: adw::ToastOverlay,
    scroller: gtk::ScrolledWindow,
    loaded: RefCell<Loaded>,
    /// The member rows drawn now, by user id (the self-test finds them here).
    pub member_rows: RefCell<Vec<(i64, adw::ActionRow)>>,
    /// The header's family name, as drawn.
    pub title: gtk::Label,
    /// The member limit's pending change, for the connection it writes through.
    cap: RefCell<Option<(Rc<Connection>, Rc<CapDraft>)>>,
    /// A change to the assistant's settings on its way (`assistant`): drawn as applied, the
    /// controls held still until it is answered.
    assistant_pending: RefCell<Option<FamilyPatch>>,
    /// The greeting's places as they are being edited, kept across redraws.
    places: RefCell<fc_logic::greeting_weather::PlacesDraft>,
    saving_places: Cell<bool>,
    /// Set while `rebuild` replaces the controls, so a field losing its focus to the redraw saves
    /// nothing.
    drawing: Cell<bool>,
}

impl std::fmt::Debug for FamilyDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FamilyDialog").finish_non_exhaustive()
    }
}

impl FamilyDialog {
    fn build() -> Rc<FamilyDialog> {
        let shell = Shell::get();
        let (dialog, view, toasts) = ui::dialog(shell.say.get("Family"), SIZE.0, SIZE.1);
        let header = adw::HeaderBar::new();
        view.add_top_bar(&header);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        toasts.set_child(Some(&scroller));
        let family = Rc::new(FamilyDialog {
            dialog,
            toasts,
            scroller,
            loaded: RefCell::new(Loaded::default()),
            member_rows: RefCell::new(Vec::new()),
            title: gtk::Label::new(None),
            cap: RefCell::new(None),
            assistant_pending: RefCell::new(None),
            places: RefCell::default(),
            saving_places: Cell::new(false),
            drawing: Cell::new(false),
        });
        family.follow(&shell);
        {
            let weak = Rc::downgrade(&family);
            family.dialog.connect_closed(move |_| {
                // Places typed and not yet saved go now (the Mac saves them as its sheet closes).
                if let (Some(family), Some(connection)) =
                    (weak.upgrade(), Shell::get().connection())
                {
                    family.save_places(&connection);
                }
                OPEN.with(|open| open.borrow_mut().take());
            });
        }
        family
    }

    /// Redraws when the session publishes (a family switch, a refresh) and when the connection is
    /// replaced; the subscriptions end with the dialog (they hold it weakly).
    fn follow(self: &Rc<Self>, shell: &Rc<Shell>) {
        let weak = Rc::downgrade(self);
        let redraw = move || {
            if let Some(me) = weak.upgrade()
                && current().is_some_and(|open| Rc::ptr_eq(&open, &me))
            {
                me.rebuild();
            }
        };
        if let Some(connection) = shell.connection() {
            let redraw = redraw.clone();
            let shown = std::cell::Cell::new(connection.session.state().current_family_id);
            let weak = Rc::downgrade(self);
            connection.session.on_changed().subscribe(move |state| {
                redraw();
                // Another family is shown now: its console is read afresh.
                if shown.replace(state.current_family_id) != state.current_family_id
                    && let Some(me) = weak.upgrade()
                {
                    me.reload();
                }
            });
            // An invitation sent, answered or withdrawn — here or on another device — changes
            // the owner's Pending Invitations: read them again (the Mac's
            // `invitationsGeneration`).
            let reread = {
                let weak = Rc::downgrade(self);
                move || {
                    if let Some(me) = weak.upgrade()
                        && current().is_some_and(|open| Rc::ptr_eq(&open, &me))
                    {
                        me.reload();
                    }
                }
            };
            let changed = reread.clone();
            connection
                .router
                .on_invitation_changed()
                .subscribe(move |_| changed());
            connection
                .router
                .on_invitation_removed()
                .subscribe(move |_| reread());
        }
        shell.on_connection_changed().subscribe(move |_| redraw());
    }

    /// Reads what the console shows from the server — the roster for everybody, and for the owner
    /// the family's own document (the code may have been rotated elsewhere), the join requests, the
    /// reports and the pending invitations — then redraws.
    pub fn reload(self: &Rc<Self>) {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        let weak = Rc::downgrade(self);
        ui::spawn(async move {
            let model = family_model(&connection);
            if let Err(error) = model.read().await {
                crate::fc_warn!("family: the cache refused ({})", log::store_error(&error));
            }
            let state = connection.session.state();
            let mut loaded = Loaded::default();
            if state.is_owner() {
                if let Err(error) = connection.session.refresh_family().await {
                    crate::fc_warn!("family: the cache refused ({})", log::store_error(&error));
                }
                loaded.requests = model.requests().await.unwrap_or_default();
                loaded.reports = connection
                    .api
                    .reports()
                    .await
                    .into_result()
                    .map(|answer| answer.reports.unwrap_or_default())
                    .unwrap_or_default();
                if family_rules::offers_invite(&state)
                    && let Some(family) = state.current_family_id
                {
                    loaded.invitations = invite_model(&connection)
                        .pending(family)
                        .await
                        .unwrap_or_default();
                }
            }
            if let Some(me) = weak.upgrade() {
                *me.loaded.borrow_mut() = loaded;
                me.rebuild();
            }
        });
    }

    /// Draws the whole console from what is held now, keeping the scroll position.
    pub fn rebuild(self: &Rc<Self>) {
        if self.drawing.replace(true) {
            return;
        }
        self.draw();
        self.drawing.set(false);
    }

    fn draw(self: &Rc<Self>) {
        let shell = Shell::get();
        let Some(connection) = shell.connection() else {
            return;
        };
        let say = Rc::clone(&shell.say);
        let state = connection.session.state();
        if !state.can_chat() {
            self.dialog.close();
            return;
        }
        let model = family_model(&connection);
        let present = model.present().unwrap_or_default();
        let blocked: Vec<i64> = model.blocked().map(|ids| ids.to_vec()).unwrap_or_default();
        let me = state.me.as_ref().map_or(0, |me| me.id);
        let owner = state.is_owner();
        let family_name = state.family.as_ref().map_or_else(
            || say.get("Family").to_string(),
            |family| family.name.clone(),
        );

        let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
        content.set_margin_top(12);
        content.set_margin_bottom(24);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // The family, named, with a count: a centred hero, the Settings window's own pattern.
        let head = gtk::Box::new(gtk::Orientation::Vertical, 8);
        head.set_halign(gtk::Align::Center);
        let face = ui::avatar(&family_name, 64);
        face.set_halign(gtk::Align::Center);
        head.append(&ui::hero_face(&face));
        let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
        names.set_halign(gtk::Align::Center);
        self.title.set_text(&family_name);
        self.title.set_xalign(0.5);
        self.title.set_justify(gtk::Justification::Center);
        self.title.set_wrap(true);
        self.title.add_css_class("title-2");
        if let Some(parent) = self.title.parent().and_downcast::<gtk::Box>() {
            parent.remove(&self.title);
        }
        names.append(&self.title);
        let count = gtk::Label::new(Some(&family_text::members_line(
            i64::try_from(present.len()).unwrap_or(0),
            say.as_ref(),
        )));
        count.set_xalign(0.5);
        count.add_css_class("dim-label");
        names.append(&count);
        head.append(&names);
        content.append(&head);

        if owner {
            content.append(&self.invite_group(&connection, &state));
            let loaded = self.loaded.borrow();
            if !loaded.invitations.is_empty() {
                content.append(&self.invitations_group(&connection, &loaded.invitations));
            }
            if !loaded.requests.is_empty() {
                content.append(&self.requests_group(&connection, &loaded.requests));
            }
            content.append(&self.reports_group(&connection, &loaded.reports));
            drop(loaded);
            content.append(&self.policy_group(&connection, &state));
            if state.member_ceiling() > 0 {
                content.append(&self.cap_group(&connection, &state, present.len()));
            }
            for group in assistant::groups(self, &connection, &state) {
                content.append(&group);
            }
        }

        let members = adw::PreferencesGroup::builder()
            .title(say.get("Members"))
            .build();
        let mut rows = Vec::new();
        for person in family_text::roster(present.iter()) {
            let tools = family_text::tools_for(&person, me, owner, blocked.contains(&person.id));
            let row = member::row(self, &person, tools, blocked.contains(&person.id));
            members.add(&row);
            rows.push((person.id, row));
        }
        *self.member_rows.borrow_mut() = rows;
        content.append(&members);

        // The family's pack (P2's "Family Stickers" dialog), on a server that keeps one.
        if crate::attachments::pack::offered(&connection.stickers) {
            let stickers = adw::PreferencesGroup::builder()
                .title(say.get("Family Stickers"))
                .description(
                    say.get("Pictures everyone in the family can send as stickers in a chat."),
                )
                .build();
            let weak = Rc::downgrade(self);
            let manage = ui::button_row(say.get("Manage…"), false, move || {
                if let Some(me) = weak.upgrade() {
                    crate::attachments::pack::present(&me.dialog);
                }
            });
            manage.add_css_class("fc-family-stickers");
            stickers.add(&manage);
            content.append(&stickers);
        }

        let numbers = adw::PreferencesGroup::new();
        let weak = Rc::downgrade(self);
        numbers.add(&ui::button_row(
            say.get("Statistics…"),
            false,
            move || {
                if let Some(me) = weak.upgrade() {
                    crate::settings::dialogs::statistics(&me.dialog);
                }
            },
        ));
        content.append(&numbers);

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .child(&content)
            .build();
        let at = self.scroller.vadjustment().value();
        self.scroller.set_child(Some(&clamp));
        let scroller = self.scroller.clone();
        glib::idle_add_local_once(move || scroller.vadjustment().set_value(at));
    }

    fn invite_group(
        self: &Rc<Self>,
        connection: &Rc<Connection>,
        state: &fc_logic::app_session::SessionState,
    ) -> adw::PreferencesGroup {
        let say = Rc::clone(&Shell::get().say);
        let group = adw::PreferencesGroup::builder()
            .title(say.get("Invite code"))
            .build();
        let code = state
            .family
            .as_ref()
            .and_then(|family| family.invite_code.clone());
        let row = adw::ActionRow::builder()
            .title(code.as_deref().unwrap_or("…"))
            .title_selectable(true)
            .build();
        row.add_css_class("fc-invite-code");
        let copy = gtk::Button::builder()
            .label(say.get("Copy"))
            .valign(gtk::Align::Center)
            .sensitive(code.is_some())
            .build();
        {
            let code = code.clone().unwrap_or_default();
            copy.connect_clicked(move |button| button.clipboard().set_text(&code));
        }
        let rotate = gtk::Button::builder()
            .label(say.get("Rotate"))
            .valign(gtk::Align::Center)
            .build();
        {
            let weak = Rc::downgrade(self);
            let connection = Rc::clone(connection);
            rotate.connect_clicked(move |button| {
                let say = Rc::clone(&Shell::get().say);
                let (weak, connection) = (weak.clone(), Rc::clone(&connection));
                // Asks first, as iOS does: one click beside Copy invalidated the code every member
                // had been given.
                ui::ask(
                    button,
                    say.get("Rotate the invite code?"),
                    say.get("Rotating invalidates the current code immediately."),
                    say.get("Cancel"),
                    say.get("Rotate Code"),
                    true,
                    move || {
                        let (weak, connection) = (weak.clone(), Rc::clone(&connection));
                        ui::spawn(async move {
                            let answer = connection.api.rotate_invite_code().await.into_result();
                            let Some(me) = weak.upgrade() else {
                                return;
                            };
                            match answer {
                                Ok(_) => {
                                    crate::fc_info!("family: invite code rotated");
                                    if let Err(error) = connection.session.refresh_family().await {
                                        crate::fc_warn!(
                                            "family: the cache refused ({})",
                                            log::store_error(&error)
                                        );
                                    }
                                    me.rebuild();
                                }
                                Err(error) => me.failed(&error),
                            }
                        });
                    },
                );
            });
        }
        row.add_suffix(&copy);
        row.add_suffix(&rotate);
        group.add(&row);
        // Between the group's title and the code: it explains Rotate, not the Invite… row under it.
        group.set_description(Some(
            say.get("Rotating invalidates the current code immediately."),
        ));
        if family_rules::offers_invite(state) {
            let weak = Rc::downgrade(self);
            group.add(&ui::button_row(say.get("Invite…"), false, move || {
                if let Some(me) = weak.upgrade() {
                    me.invite_picker();
                }
            }));
        }
        group
    }

    fn invitations_group(
        self: &Rc<Self>,
        connection: &Rc<Connection>,
        invitations: &[InvitationDto],
    ) -> adw::PreferencesGroup {
        let say = Rc::clone(&Shell::get().say);
        let group = adw::PreferencesGroup::builder()
            .title(say.get("Pending Invitations"))
            .build();
        for invitation in invitations {
            // A person row like every other in the dialog: the face, the name, the username.
            let row = adw::ActionRow::builder()
                .title(&invitation.user.display_name)
                .subtitle(format!("@{}", invitation.user.username))
                .build();
            row.add_prefix(&ui::face(
                &invitation.user.display_name,
                invitation.user.id,
                32,
            ));
            let withdraw = gtk::Button::builder()
                .label(say.get("Withdraw Invitation"))
                .valign(gtk::Align::Center)
                .css_classes(["destructive-action", "flat"])
                .build();
            let (weak, connection, family, id) = (
                Rc::downgrade(self),
                Rc::clone(connection),
                invitation.family_id,
                invitation.id,
            );
            withdraw.connect_clicked(move |_| {
                let (weak, connection) = (weak.clone(), Rc::clone(&connection));
                ui::spawn(async move {
                    let refused = invite_model(&connection).withdraw(family, id).await;
                    if let Some(me) = weak.upgrade() {
                        match refused {
                            Some(error) => me.failed(&error),
                            None => me.reload(),
                        }
                    }
                });
            });
            row.add_suffix(&withdraw);
            group.add(&row);
        }
        group
    }

    fn requests_group(
        self: &Rc<Self>,
        connection: &Rc<Connection>,
        requests: &[JoinRequestDto],
    ) -> adw::PreferencesGroup {
        let say = Rc::clone(&Shell::get().say);
        let group = adw::PreferencesGroup::builder()
            .title(say.get("Join requests"))
            .build();
        for request in requests {
            let row = adw::ActionRow::builder()
                .title(&request.user.display_name)
                .subtitle(format!("@{}", request.user.username))
                .build();
            row.add_prefix(&ui::face(&request.user.display_name, request.user.id, 32));
            for (label, approve) in [(say.get("Approve"), true), (say.get("Decline"), false)] {
                let button = gtk::Button::builder()
                    .label(label)
                    .valign(gtk::Align::Center)
                    .build();
                // Approving is the console's primary act; declining, red words without a fill.
                if approve {
                    button.add_css_class("suggested-action");
                } else {
                    button.add_css_class("destructive-action");
                    button.add_css_class("flat");
                }
                let (weak, connection, id) =
                    (Rc::downgrade(self), Rc::clone(connection), request.id);
                button.connect_clicked(move |_| {
                    let (weak, connection) = (weak.clone(), Rc::clone(&connection));
                    ui::spawn(async move {
                        let model = family_model(&connection);
                        let refused = if approve {
                            match model.approve(id).await {
                                Ok(Ok(_)) => None,
                                Ok(Err(not)) => Some(not.error),
                                Err(error) => {
                                    crate::fc_warn!(
                                        "family: the cache refused ({})",
                                        log::store_error(&error)
                                    );
                                    None
                                }
                            }
                        } else {
                            model.reject(id).await
                        };
                        if let Some(me) = weak.upgrade() {
                            if let Some(error) = refused {
                                let shell = Shell::get();
                                ui::toast(
                                    &me.toasts,
                                    &family_text::request_failure(&error, shell.say.as_ref()),
                                );
                            }
                            connection.live.refresh();
                            me.reload();
                        }
                    });
                });
                row.add_suffix(&button);
            }
            group.add(&row);
        }
        group
    }

    fn reports_group(
        self: &Rc<Self>,
        connection: &Rc<Connection>,
        reports: &[ReportDto],
    ) -> adw::PreferencesGroup {
        let say = Rc::clone(&Shell::get().say);
        let group = adw::PreferencesGroup::builder()
            .title(say.get("Reports"))
            .build();
        // Drawn even when empty: an owner who never had a report otherwise has no surface naming
        // the inbox at all.
        if reports.is_empty() {
            group.add(&ui::footnote(
                say.get("Members can report a message or a person to you."),
            ));
        }
        for report in reports {
            let row = adw::ExpanderRow::builder()
                .title(words::reason_label(&report.reason, say.as_ref()))
                .subtitle(family_text::reported(report, say.as_ref()))
                .build();
            // The frozen excerpt, whole — an owner judging a message has to see all of it — and
            // what it carried when the words do not say.
            if let Some(excerpt) = report.message_excerpt.as_deref().filter(|e| !e.is_empty()) {
                let text = gtk::Label::builder()
                    .label(excerpt)
                    .wrap(true)
                    .selectable(true)
                    .xalign(0.0)
                    .margin_start(12)
                    .margin_end(12)
                    .margin_top(6)
                    .margin_bottom(6)
                    .build();
                row.add_row(&text);
            }
            if let Some(carried) =
                family_text::carried(report.message_attachments.as_deref(), say.as_ref())
            {
                row.add_row(&adw::ActionRow::builder().title(carried).build());
            }
            let handled = gtk::Button::builder()
                .label(say.get("Mark as handled"))
                .valign(gtk::Align::Center)
                .build();
            let (weak, connection, id) = (Rc::downgrade(self), Rc::clone(connection), report.id);
            handled.connect_clicked(move |button| {
                button.set_sensitive(false);
                let (weak, connection) = (weak.clone(), Rc::clone(&connection));
                ui::spawn(async move {
                    let answer = connection.api.resolve_report(id).await.into_result();
                    if let Some(me) = weak.upgrade() {
                        match answer {
                            Ok(_) => {
                                me.loaded.borrow_mut().reports.retain(|r| r.id != id);
                                me.rebuild();
                            }
                            Err(error) => me.failed(&error),
                        }
                    }
                });
            });
            row.add_suffix(&handled);
            group.add(&row);
        }
        group
    }

    fn policy_group(
        self: &Rc<Self>,
        connection: &Rc<Connection>,
        state: &fc_logic::app_session::SessionState,
    ) -> adw::PreferencesGroup {
        let say = Rc::clone(&Shell::get().say);
        let group = adw::PreferencesGroup::builder()
            .title(say.get("Join policy"))
            .build();
        let policy = state
            .family
            .as_ref()
            .and_then(|family| family.join_policy.clone());
        let labels = words::policy_labels(say.as_ref());
        let combo = adw::ComboRow::builder()
            .title(say.get("New members"))
            .model(&gtk::StringList::new(&labels))
            .build();
        let at = house_rules::POLICIES
            .iter()
            .position(|(code, _)| Some(*code) == policy.as_deref())
            .unwrap_or(0);
        combo.set_selected(u32::try_from(at).unwrap_or(0));
        {
            let (weak, connection) = (Rc::downgrade(self), Rc::clone(connection));
            combo.connect_selected_notify(move |combo| {
                let Some((code, _)) = usize::try_from(combo.selected())
                    .ok()
                    .and_then(|at| house_rules::POLICIES.get(at))
                else {
                    return;
                };
                let held = connection
                    .session
                    .state()
                    .family
                    .and_then(|family| family.join_policy);
                if held.as_deref().unwrap_or("open") == *code {
                    return;
                }
                let patch = FamilyPatch {
                    join_policy: Some((*code).to_string()),
                    ..FamilyPatch::default()
                };
                if let Some(me) = weak.upgrade() {
                    me.change_family(&connection, patch);
                }
            });
        }
        group.add(&combo);
        group.add(&ui::footnote(&house_rules::policy_caption(
            policy.as_deref(),
            say.as_ref(),
        )));
        group
    }

    fn cap_group(
        self: &Rc<Self>,
        connection: &Rc<Connection>,
        state: &fc_logic::app_session::SessionState,
        members: usize,
    ) -> adw::PreferencesGroup {
        let say = Rc::clone(&Shell::get().say);
        let group = adw::PreferencesGroup::builder()
            .title(say.get("Member limit"))
            .build();
        // The family's own ceiling — a paid family's is lower — else the server's
        // (docs/protocol.md, "Paid families": `member_ceiling`).
        let ceiling = state.member_ceiling();
        let count = i32::try_from(members).unwrap_or(i32::MAX);
        let held = state.family.as_ref().and_then(|family| family.max_members);
        let draft = self.cap_draft(connection);
        // What is drawn is the change on its way, when there is one (`CapDraft::drawn`) — the
        // footer and the stepper follow it, not the server's value it is about to replace.
        let drawn = draft.drawn(held);
        let limit = adw::SwitchRow::builder()
            .title(say.get("Limit members"))
            .active(drawn.is_some())
            .build();
        let most = adw::SpinRow::with_range(1.0, f64::from(ceiling.max(1)), 1.0);
        most.set_title(say.get("Most members"));
        most.set_value(f64::from(drawn.unwrap_or_else(|| {
            i32::try_from(fc_text::account::seed_cap(
                i64::from(count),
                i64::from(ceiling),
            ))
            .unwrap_or(1)
        })));
        most.set_visible(drawn.is_some());
        let footer = ui::footnote(&fc_text::account::cap_footer(fc_text::account::cap_state(
            drawn.map(i64::from),
            i64::from(count),
            i64::from(ceiling),
        )));
        let problem = ui::problem_label();
        if draft.failed() {
            ui::show_problem(
                &problem,
                say.get("Couldn't change the member limit. Try again."),
            );
        }
        {
            // On freezes the family where it stands; off clears the cap. One write per run.
            let (draft, most) = (Rc::clone(&draft), most.clone());
            limit.connect_active_notify(move |limit| {
                most.set_visible(limit.is_active());
                draft.toggle(limit.is_active(), count, ceiling);
            });
        }
        {
            let draft = Rc::clone(&draft);
            most.connect_value_notify(move |most| {
                if !most.is_visible() {
                    return;
                }
                let value = most.value().round() as i32;
                if Some(value) != draft.drawn(held) {
                    draft.typed(value, ceiling);
                }
            });
        }
        {
            // The footer and the refusal follow the draft in place: a redraw of the whole group
            // mid-run would take the stepper from under the pointer.
            let (weak_draft, footer, problem, say) = (
                Rc::downgrade(&draft),
                footer.clone(),
                problem.clone(),
                Rc::clone(&say),
            );
            let subscription = draft.changed().subscribe(move |_| {
                let Some(draft) = weak_draft.upgrade() else {
                    return;
                };
                let held = Shell::get()
                    .connection()
                    .and_then(|connection| connection.session.state().family)
                    .and_then(|family| family.max_members);
                footer.set_text(&fc_text::account::cap_footer(fc_text::account::cap_state(
                    draft.drawn(held).map(i64::from),
                    i64::from(count),
                    i64::from(ceiling),
                )));
                if draft.failed() {
                    ui::show_problem(
                        &problem,
                        say.get("Couldn't change the member limit. Try again."),
                    );
                } else {
                    ui::show_problem(&problem, "");
                }
            });
            let draft = Rc::clone(&draft);
            group.connect_destroy(move |_| {
                draft.changed().unsubscribe(subscription);
            });
        }
        group.add(&limit);
        group.add(&most);
        group.add(&footer);
        group.add(&problem);
        group
    }

    /// The member limit's pending change (`CapDraft`): one write at the end of a run of steps,
    /// sent as a family change and the family read back; made once per dialog and connection.
    fn cap_draft(self: &Rc<Self>, connection: &Rc<Connection>) -> Rc<CapDraft> {
        if let Some((held, draft)) = self.cap.borrow().as_ref()
            && Rc::ptr_eq(held, connection)
        {
            return Rc::clone(draft);
        }
        let send: fc_logic::cap_draft::CapSend = {
            let connection = Rc::downgrade(connection);
            Rc::new(move |cap: Option<i32>| {
                let connection = connection.upgrade();
                Box::pin(async move {
                    let connection = connection?;
                    let family = connection.session.state().family?;
                    let patch = match cap {
                        Some(cap) => FamilyPatch {
                            max_members: Some(cap),
                            ..FamilyPatch::default()
                        },
                        None => FamilyPatch {
                            clears_cap: true,
                            ..FamilyPatch::default()
                        },
                    };
                    let refused = family_model(&connection)
                        .change(&family, &patch)
                        .await
                        .err();
                    if let Some(error) = &refused {
                        crate::fc_info!("family: member limit refused ({})", error.code);
                    }
                    if let Err(error) = connection.session.refresh_family().await {
                        crate::fc_warn!("family: the cache refused ({})", log::store_error(&error));
                    }
                    refused
                })
            })
        };
        let clock = Shell::get().clock.clone();
        let wait: fc_logic::cap_draft::Pause =
            Rc::new(move |duration| fc_core::rt::Clock::sleep(clock.as_ref(), duration));
        let spawn: fc_logic::calls::Spawn = Rc::new(|future| {
            glib::spawn_future_local(future);
        });
        let draft = CapDraft::new(send, wait, spawn);
        *self.cap.borrow_mut() = Some((Rc::clone(connection), Rc::clone(&draft)));
        draft
    }

    /// Sends a change of the family (`FamilyModel::change`, which never sends a combination the
    /// server refuses), then reads the family back.
    fn change_family(self: &Rc<Self>, connection: &Rc<Connection>, patch: FamilyPatch) {
        let Some(family) = connection.session.state().family else {
            return;
        };
        let (weak, connection) = (Rc::downgrade(self), Rc::clone(connection));
        ui::spawn(async move {
            let answer = family_model(&connection).change(&family, &patch).await;
            if let Err(error) = &answer {
                crate::fc_info!("family: change refused ({})", error.code);
            }
            if let Err(error) = connection.session.refresh_family().await {
                crate::fc_warn!("family: the cache refused ({})", log::store_error(&error));
            }
            if let Some(me) = weak.upgrade() {
                if let Err(error) = answer {
                    me.failed(&error);
                }
                me.rebuild();
            }
        });
    }

    /// "Invite…": the members of the owner's OTHER families who are not in this one, each invited
    /// with a click and marked "Invited" while the invitation waits.
    pub fn invite_picker(self: &Rc<Self>) -> Option<adw::Dialog> {
        let shell = Shell::get();
        let connection = shell.connection()?;
        let say = Rc::clone(&shell.say);
        let state = connection.session.state();
        let family_name = state
            .family
            .as_ref()
            .map(|family| family.name.clone())
            .unwrap_or_default();
        let (dialog, view, toasts) =
            ui::dialog(&say.format("Invite to “%@”", &[&family_name]), 420, 420);
        let header = adw::HeaderBar::new();
        view.add_top_bar(&header);
        let group = adw::PreferencesGroup::new();
        let page = adw::PreferencesPage::new();
        page.add(&group);
        toasts.set_child(Some(&page));
        fill_invitees(self, &connection, &group, &toasts);
        dialog.present(Some(&self.dialog));
        Some(dialog)
    }

    /// Says a refusal in the reader's words (several-families refusals first).
    fn failed(&self, error: &fc_core::protocol::ApiError) {
        let shell = Shell::get();
        let say: &dyn StringCatalog = shell.say.as_ref();
        let cap = shell.connection().map_or(1, |connection| {
            connection.session.state().max_families_per_account
        });
        let text = family_sentences::for_error(error, cap, say)
            .unwrap_or_else(|| family_text::generic_failure(error, say));
        crate::fc_info!("family: refused ({})", error.code);
        ui::toast(&self.toasts, &text);
    }
}

fn fill_invitees(
    family: &Rc<FamilyDialog>,
    connection: &Rc<Connection>,
    group: &adw::PreferencesGroup,
    toasts: &adw::ToastOverlay,
) {
    let say = Rc::clone(&Shell::get().say);
    while let Some(child) = group_rows(group).pop() {
        group.remove(&child);
    }
    let state = connection.session.state();
    let pending = family.loaded.borrow().invitations.clone();
    let candidates = invite_model(connection)
        .candidates(&state, &pending)
        .unwrap_or_default();
    if candidates.is_empty() {
        group.add(&ui::footnote(say.get(
            "Members of your other families who aren't in this one appear here.",
        )));
        return;
    }
    for candidate in candidates {
        let row = adw::ActionRow::builder()
            .title(&candidate.member.display_name)
            .build();
        row.add_prefix(&ui::face(
            &candidate.member.display_name,
            candidate.member.id,
            32,
        ));
        if candidate.invited {
            let invited = gtk::Label::new(Some(say.get("Invited")));
            invited.add_css_class("dim-label");
            row.add_suffix(&invited);
        } else {
            let button = gtk::Button::builder()
                .icon_name("mail-send-symbolic")
                .tooltip_text(say.get("Invite…"))
                .valign(gtk::Align::Center)
                .build();
            let (weak, connection, group, toasts, user) = (
                Rc::downgrade(family),
                Rc::clone(connection),
                group.clone(),
                toasts.clone(),
                candidate.member.id,
            );
            button.connect_clicked(move |button| {
                button.set_sensitive(false);
                let Some(family_id) = connection.session.state().current_family_id else {
                    return;
                };
                let (weak, connection, group, toasts) = (
                    weak.clone(),
                    Rc::clone(&connection),
                    group.clone(),
                    toasts.clone(),
                );
                ui::spawn(async move {
                    let invites = invite_model(&connection);
                    let answer = invites.invite(family_id, user).await;
                    let Some(me) = weak.upgrade() else {
                        return;
                    };
                    match answer {
                        Ok(_) => {
                            crate::fc_info!("family: invited user {user}");
                            me.loaded.borrow_mut().invitations =
                                invites.pending(family_id).await.unwrap_or_default();
                            fill_invitees(&me, &connection, &group, &toasts);
                            me.rebuild();
                        }
                        Err(error) => {
                            let shell = Shell::get();
                            let say: &dyn StringCatalog = shell.say.as_ref();
                            let cap = connection.session.state().max_families_per_account;
                            let text = family_sentences::for_error(&error, cap, say)
                                .unwrap_or_else(|| family_text::generic_failure(&error, say));
                            ui::toast(&toasts, &text);
                        }
                    }
                });
            });
            row.add_suffix(&button);
        }
        group.add(&row);
    }
}

/// The rows (and footnotes) a preferences group holds, to empty it.
fn group_rows(group: &adw::PreferencesGroup) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut index = 0;
    while let Some(row) = group.row(index) {
        found.push(row.upcast());
        index += 1;
    }
    // Footnotes sit below the list, outside `row`'s reach: find them as the group's labels.
    let mut stack: Vec<gtk::Widget> = group.first_child().into_iter().collect();
    while let Some(widget) = stack.pop() {
        if widget.has_css_class("caption") && widget.is::<gtk::Label>() {
            found.push(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    found
}

/// The roster and the console's calls over this connection.
pub fn family_model(connection: &Connection) -> FamilyModel {
    FamilyModel::new(Rc::clone(&connection.api), Rc::clone(&connection.chats))
}

/// The owner's invitations over this connection, names in the desktop's collation.
fn invite_model(connection: &Connection) -> InviteModel {
    InviteModel::new(Rc::clone(&connection.api), Rc::clone(&connection.chats)).with_collation(
        Rc::new(|a: &str, b: &str| {
            glib::GString::from(a.to_lowercase())
                .as_gstr()
                .collate(b.to_lowercase().as_str())
        }),
    )
}

/// A member's subtitle: "@username", and the birthday beside it when there is one — day and month
/// in the reader's language, never a year.
pub fn member_subtitle(person: &MemberDto, say: &dyn StringCatalog) -> String {
    let shell = Shell::get();
    match person.birthday.as_ref() {
        Some(day) if birthday_rules::ok(day.month, day.day) => format!(
            "@{} · {}",
            person.username,
            birthday_rules::text(Some(day), say, &shell.culture)
        ),
        _ => format!("@{}", person.username),
    }
}
