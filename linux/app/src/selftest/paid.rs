//! The `paid` group (issue #89, §27): paid families on a server that charges for them
//! (docs/protocol.md, "Paid families"). An account in no family meets the family gate with the
//! sentence saying where a family is started in place of "Create Family" ("Join a family" still
//! there), and its create is refused as `subscription_required`; with a subscription bought in the
//! Apple app the form is back. A member of an unpaid family sees the banner over the chat list
//! naming the owner; a `family_plan` frame without `delete_at` clears it and one with it brings it
//! back, live; Settings' "New Family…" says where a family is started. The owner sees their own
//! banner, a member-cap picker whose range is the family's `member_ceiling`, and the allowance in
//! Family Statistics. Screenshots: the gate, both banners, the cleared list, New Family…, the
//! statistics.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::{AllowanceDto, AllowanceUseDto, ServerUrl, SubscriptionDto};
use fc_logic::doorway::FamilyDoor;
use fc_logic::paid_families::{self, Days};
use time::format_description::well_known::Rfc3339;

use super::Harness;
use super::fake_server::FakePlan;
use super::settings::{button, descendants, find, install, person, sign_in_as};
use crate::family;
use crate::shell::Connection;
use crate::windows::main_window::{GatePage, MainWindow};

const WAIT: Duration = Duration::from_secs(15);

pub async fn run(h: &Harness) {
    let shell = h.shell();
    let fake = h.fake_server();
    install(&fake);
    let olga = person(&fake, "olga", "Olga");
    let pavel = person(&fake, "pavel", "Pavel");
    let nora = person(&fake, "nora", "Nora");
    let family = fake.add_family("Olga's Family", olga);
    fake.join(pavel, family, "member");
    // Ten days from now, at nine: the day the banner names.
    let deadline = (time::OffsetDateTime::now_utc() + time::Duration::days(10))
        .replace_time(time::Time::from_hms(9, 0, 0).unwrap_or(time::Time::MIDNIGHT));
    let delete_at = deadline.format(&Rfc3339).unwrap_or_default();
    {
        let mut state = fake.state();
        state.subscriptions = true;
        state.plans.insert(
            family,
            FakePlan {
                plan: "paid".into(),
                member_ceiling: 8,
                delete_at: Some(delete_at.clone()),
            },
        );
        state.allowances.insert(
            family,
            AllowanceDto {
                renews_at: Some(next_month()),
                storage_bytes: Some(AllowanceUseDto {
                    used: 3_221_225_472,
                    limit: 10_737_418_240,
                }),
                ai_tokens: Some(AllowanceUseDto {
                    used: 812_345,
                    limit: 2_000_000,
                }),
                images: Some(AllowanceUseDto { used: 4, limit: 30 }),
                transcript_ms: Some(AllowanceUseDto {
                    used: 600_001,
                    limit: 7_200_000,
                }),
            },
        );
    }
    let say = Rc::clone(&shell.say);
    let days = Days::new(
        &shell.culture,
        shell.zone.as_ref(),
        time::OffsetDateTime::now_utc(),
    );
    let date = days.day(deadline);

    // ---- the gate: no subscription, the sentence where Create was --------------------------------
    let Some(connection) = sign_in_at(h, "nora", GatePage::Family).await else {
        h.check("gate_signed_in", false, "nora could not sign in");
        return;
    };
    let Some(main) = shell.main_window() else {
        h.check("gate_signed_in", false, "no main window");
        return;
    };
    let gate = main.page_slot(GatePage::Family);
    let sentence = paid_families::start_in_apple_app(say.as_ref());
    let said = h
        .wait_until(WAIT, || {
            find::<adw::PreferencesGroup>(&gate, |group| {
                group.description().as_deref() == Some(sentence.as_str())
                    && group.title() == "Create a family"
            })
            .is_some()
        })
        .await;
    let joins = button(&gate, "Join").is_some();
    let no_create = button(&gate, "Create Family").is_none();
    h.sleep(Duration::from_millis(400)).await;
    h.screenshot("gate").await;
    h.check(
        "gate_says_where_a_family_is_started",
        said && joins && no_create,
        format!("the sentence {said}, Join {joins}, no Create {no_create}"),
    );

    // A create from anywhere is refused, and said in the door's own words.
    let door = FamilyDoor::new(
        Rc::clone(&connection.api),
        Rc::clone(&connection.session),
        shell.say.clone(),
    );
    let refused = door.create("Nora's Family").await;
    let expected = paid_families::subscription_required(say.as_ref());
    h.check(
        "create_refused_needs_a_subscription",
        matches!(&refused, Ok(Some(text)) if *text == expected),
        format!("{refused:?}"),
    );

    // Bought in the Apple app: the form is back on this one too.
    fake.state().subscriptions_held.insert(
        nora,
        SubscriptionDto {
            product_id: "me.nettrash.FamilyConnect.family.monthly".into(),
            environment: "Sandbox".into(),
            expires_at: Some(next_month()),
            active: true,
            renews: Some(true),
        },
    );
    let _ = connection.session.sign_out().await;
    let offered = match sign_in_at(h, "nora", GatePage::Family).await {
        Some(_) => {
            let gate = main.page_slot(GatePage::Family);
            h.wait_until(WAIT, || button(&gate, "Create Family").is_some())
                .await
        }
        None => false,
    };
    h.check(
        "gate_offers_create_with_a_subscription",
        offered,
        "Create Family is drawn again",
    );
    fake.state().subscriptions_held.remove(&nora);

    // ---- a member's banner --------------------------------------------------------------------------
    let Some(connection) = sign_in_as(h, "pavel").await else {
        h.check("member_signed_in", false, "pavel could not sign in");
        return;
    };
    let member_says = format!(
        "“Olga's Family” will be deleted on {date} because its subscription has ended. Only Olga, the owner, can renew it."
    );
    let shown = h
        .wait_until(WAIT, || banner_text(&main).as_deref() == Some(&member_says))
        .await;
    h.screenshot("banner-member").await;
    h.check(
        "member_banner_names_the_owner",
        shown,
        format!("{:?}", banner_text(&main)),
    );

    // Covered again: the frame without `delete_at` takes the banner away, with no read.
    if let Some(plan) = fake.state().plans.get_mut(&family) {
        plan.delete_at = None;
    }
    fake.send_frame_to(
        pavel,
        &format!(r#"{{"type": "family_plan", "family_id": {family}, "plan": "paid"}}"#),
    );
    let cleared = h
        .wait_until(WAIT, || {
            banner_text(&main).is_none()
                && connection
                    .session
                    .state()
                    .family
                    .is_some_and(|f| f.delete_at.is_none())
        })
        .await;
    h.sleep(Duration::from_millis(300)).await;
    h.screenshot("banner-cleared").await;
    h.check(
        "family_plan_clears_the_banner",
        cleared,
        format!("{:?}", banner_text(&main)),
    );
    // And unpaid again: back, live.
    if let Some(plan) = fake.state().plans.get_mut(&family) {
        plan.delete_at = Some(delete_at.clone());
    }
    fake.send_frame_to(
        pavel,
        &format!(
            r#"{{"type": "family_plan", "family_id": {family}, "plan": "paid", "delete_at": "{delete_at}"}}"#
        ),
    );
    let back = h
        .wait_until(WAIT, || banner_text(&main).as_deref() == Some(&member_says))
        .await;
    h.check(
        "family_plan_brings_the_banner_back",
        back,
        format!("{:?}", banner_text(&main)),
    );

    // Settings' "New Family…": the entry stays, and says where a family is started.
    crate::settings::dialogs::new_family(main.window());
    let elsewhere = h
        .wait_until(Duration::from_secs(5), || {
            main.window().visible_dialog().is_some_and(|dialog| {
                find::<adw::PreferencesGroup>(&dialog, |group| {
                    group.description().as_deref() == Some(sentence.as_str())
                })
                .is_some()
                    && button(&dialog, "Create Family").is_none()
            })
        })
        .await;
    h.sleep(Duration::from_millis(400)).await;
    h.screenshot("new-family").await;
    if let Some(dialog) = main.window().visible_dialog() {
        dialog.close();
    }
    h.check(
        "new_family_says_where_a_family_is_started",
        elsewhere,
        "the sentence, no Create Family",
    );

    // ---- the owner's banner -------------------------------------------------------------------------
    if sign_in_as(h, "olga").await.is_none() {
        h.check("owner_signed_in", false, "olga could not sign in");
        return;
    }
    let owner_says = format!(
        "“Olga's Family” will be deleted on {date} because your subscription has ended. Renew it in the Family Connect app on iPhone, iPad or Mac."
    );
    let shown = h
        .wait_until(WAIT, || banner_text(&main).as_deref() == Some(&owner_says))
        .await;
    h.screenshot("banner-owner").await;
    h.check(
        "owner_banner_says_where_to_renew",
        shown,
        format!("{:?}", banner_text(&main)),
    );

    // The member-cap picker's range is the family's ceiling (8), not the server's (12).
    shell
        .app
        .activate_action(crate::seams::actions::OPEN_FAMILY.name, None);
    let ranged = h
        .wait_until(WAIT, || {
            family::current().is_some_and(|open| {
                find::<adw::SpinRow>(&open.dialog, |row| row.title() == "Most members")
                    .is_some_and(|row| row.adjustment().upper() == 8.0)
            })
        })
        .await;
    let upper = family::current().and_then(|open| {
        find::<adw::SpinRow>(&open.dialog, |row| row.title() == "Most members")
            .map(|row| row.adjustment().upper())
    });
    if let Some(open) = family::current() {
        open.dialog.close();
    }
    h.check(
        "cap_picker_range_is_the_familys_ceiling",
        ranged,
        format!("upper {upper:?}"),
    );
    h.sleep(Duration::from_millis(400)).await;

    // ---- the allowance in Family Statistics -----------------------------------------------------------
    crate::settings::dialogs::statistics(main.window());
    let title = paid_families::allowance_title(say.as_ref());
    let renews = paid_families::allowance_renews(
        &AllowanceDto {
            renews_at: Some(next_month()),
            ..AllowanceDto::default()
        },
        say.as_ref(),
        &days,
    )
    .unwrap_or_default();
    let rows = |dialog: &adw::Dialog| -> Vec<(String, String)> {
        let Some(group) = find::<adw::PreferencesGroup>(dialog, |g| g.title() == title) else {
            return Vec::new();
        };
        descendants(&group)
            .into_iter()
            .filter_map(|w| w.downcast::<adw::ActionRow>().ok())
            .map(|row| {
                let value = find::<gtk::Label>(&row, |l| l.has_css_class("dim-label"))
                    .map(|l| l.text().to_string())
                    .unwrap_or_default();
                (row.title().to_string(), value)
            })
            .collect()
    };
    let wanted_rows = |found: &[(String, String)]| {
        let labels: Vec<&str> = found.iter().map(|(t, _)| t.as_str()).collect();
        labels == ["Storage", "Assistant", "Pictures", "Transcripts"]
            && found[1].1 == "40% used"
            && found[2].1 == "4 of 30"
            && found[3].1 == "11 of 120 min"
            && found[0].1.contains(" of ")
    };
    let listed = h
        .wait_until(WAIT, || {
            main.window()
                .visible_dialog()
                .is_some_and(|dialog| wanted_rows(&rows(&dialog)))
        })
        .await;
    let footnote = main.window().visible_dialog().is_some_and(|dialog| {
        find::<adw::PreferencesGroup>(&dialog, |g| g.title() == title)
            .is_some_and(|group| find::<gtk::Label>(&group, |l| l.text() == renews).is_some())
    });
    let seen = main
        .window()
        .visible_dialog()
        .map(|dialog| rows(&dialog))
        .unwrap_or_default();
    h.sleep(Duration::from_millis(400)).await;
    h.screenshot("statistics").await;
    // Scrolled to the allowance, so its last row and the renewal line are in the picture too.
    if let Some(dialog) = main.window().visible_dialog()
        && let Some(scroller) = find::<gtk::ScrolledWindow>(&dialog, |_| true)
        && let Some(group) = find::<adw::PreferencesGroup>(&dialog, |g| g.title() == title)
        && let Some(inside) = scroller.child()
        && let Some(top) = group.compute_point(&inside, &gtk::graphene::Point::new(0.0, 0.0))
    {
        scroller.vadjustment().set_value(f64::from(top.y()) - 12.0);
        h.sleep(Duration::from_millis(400)).await;
        h.screenshot("statistics-allowance").await;
    }
    if let Some(dialog) = main.window().visible_dialog() {
        dialog.close();
    }
    h.check(
        "statistics_show_the_allowance",
        listed && footnote,
        format!("rows {seen:?}, \"{renews}\" drawn {footnote}"),
    );

    // The groups after this one meet a server that charges nothing, and the app signed out, as the
    // `settings` group leaves it (the `calls` group signs in from there).
    {
        let mut state = fake.state();
        state.subscriptions = false;
        state.plans.clear();
        state.allowances.clear();
    }
    if let Some(connection) = shell.connection() {
        let _ = connection.session.sign_out().await;
        h.wait_until(WAIT, || main.page() == GatePage::SignIn).await;
    }
    let unhandled = fake.unhandled();
    h.check(
        "fake_server_knew_every_request",
        unhandled.is_empty(),
        format!("{unhandled:?}"),
    );
}

/// The unpaid banner's words over the chat list, or `None` while none is drawn.
fn banner_text(main: &MainWindow) -> Option<String> {
    let slot = main.sidebar_banner_slot();
    let banner = find::<gtk::Box>(&slot, |b| b.has_css_class("fc-unpaid-banner"))?;
    find::<gtk::Label>(&banner, |_| true).map(|label| label.text().to_string())
}

/// The 1st of the next month, UTC, as the wire writes it.
fn next_month() -> String {
    let now = time::OffsetDateTime::now_utc();
    let culture = fc_logic::date_culture::DateCulture::invariant();
    let utc = time::UtcOffset::UTC;
    Days::new(&culture, &utc, now)
        .next_month()
        .format(&Rfc3339)
        .unwrap_or_default()
}

/// Points the app at the fake server and signs in as `username`, waiting for `page` — the gate's
/// own pages included, where `settings::sign_in_as` waits for the chats.
async fn sign_in_at(h: &Harness, username: &str, page: GatePage) -> Option<Rc<Connection>> {
    let shell = h.shell();
    let fake = h.fake_server();
    let server = ServerUrl::normalise(Some(&fake.url()))?;
    let here = shell
        .connection()
        .is_some_and(|connection| connection.server.to_string() == server.to_string());
    if !here {
        shell.open_server(server);
        h.wait_until(WAIT, || {
            shell.connection().is_some() && !shell.gate_facts().booting
        })
        .await;
    }
    let connection = shell.connection()?;
    if connection.session.state().me.is_some() {
        let _ = connection.session.sign_out().await;
    }
    let answer = connection.session.sign_in(username, "password").await;
    if !matches!(answer, Ok(None)) {
        return None;
    }
    let main = shell.main_window()?;
    h.wait_until(WAIT, || main.page() == page).await;
    h.sleep(Duration::from_millis(400)).await;
    Some(connection)
}
