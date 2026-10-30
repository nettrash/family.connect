//! `SeveralFamiliesTests`, the doors' part: "New Family…" and "Join Another Family…" add the family
//! and SHOW it (docs/protocol.md, "Several families"), and the three refusals of a further family
//! say why, in their own words.

mod common;

use std::rc::Rc;

use fc_core::protocol::{ApiError, JoinAnswer, MemoryTokenStore, error_codes};
use fc_core::store::{Database, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_logic::app_session::{AppSession, Gate};
use fc_logic::doorway::{FamilyDoor, door_sentences};
use fc_logic::families::{another_family, family_sentences};
use time::macros::datetime;

use common::server::Server;
use common::session::api_with;

const SMITHS: &str = r#"{"id": 3, "name": "The Smiths", "role": "member"}"#;
const IVANOVS: &str = r#"{"id": 5, "name": "Ivanovs", "role": "owner"}"#;

fn me(families: &str, pending: &str) -> String {
    format!(
        r#"{{"user": {{"id": 7, "username": "anna", "display_name": "Anna"}}, "blocked_user_ids": [],
         "families": [{families}], "pending_join_requests": [{pending}], "invitations": [],
         "member_families_enabled": true, "max_families_per_account": 5}}"#
    )
}

struct Rig {
    session: Rc<AppSession>,
    door: FamilyDoor,
    clock: FakeClock,
}

fn build(server: &Server) -> StoreResult<Rig> {
    let clock = FakeClock::at(datetime!(2026-10-22 10:00 UTC));
    let tokens = Rc::new(MemoryTokenStore::new(Some("t0ken")));
    let api = api_with(Rc::new(server.clone()), &clock, tokens.clone());
    let session = Rc::new(AppSession::new(
        api.clone(),
        tokens,
        Database::open_in_memory()?,
    ));
    Ok(Rig {
        door: FamilyDoor::new(api, session.clone(), Rc::new(common::english())),
        session,
        clock,
    })
}

impl Rig {
    fn refresh(&self) -> StoreResult<()> {
        drive(&self.clock, self.session.refresh())?;
        Ok(())
    }
}

/// "New Family…": the family is created, added and SHOWN — its owner is this account.
#[test]
fn a_new_family_is_shown() -> StoreResult<()> {
    let server = Server::new();
    server
        .then(
            "/me",
            &[
                (200, Some(&me(SMITHS, ""))),
                (200, Some(&me(&format!("{SMITHS}, {IVANOVS}"), ""))),
            ],
        )
        .on_status(
            "/families",
            201,
            Some(r#"{"family": {"id": 5, "name": "Ivanovs"}}"#),
        );
    let rig = build(&server)?;
    rig.refresh()?;
    assert_eq!(None, drive(&rig.clock, rig.door.create("Ivanovs"))?);
    assert_eq!(Some(5), rig.session.state().current_family_id);
    assert_eq!(Gate::Owner, rig.session.state().gate);
    Ok(())
}

/// "Join Another Family…": joined is added and shown; pending leaves the family shown where it is.
#[test]
fn joining_another_family_shows_it_when_joined() -> StoreResult<()> {
    let joined = Server::new();
    joined
        .then(
            "/me",
            &[
                (200, Some(&me(SMITHS, ""))),
                (200, Some(&me(&format!("{SMITHS}, {IVANOVS}"), ""))),
            ],
        )
        .on("/families/join", r#"{"status": "joined"}"#)
        .on("/families/5", r#"{"family": {"id": 5, "name": "Ivanovs"}}"#);
    let rig = build(&joined)?;
    rig.refresh()?;
    assert_eq!(None, drive(&rig.clock, rig.door.join("ABCD2345"))?);
    assert_eq!(Some(5), rig.session.state().current_family_id);

    let pending = Server::new();
    pending
        .then(
            "/me",
            &[
                (200, Some(&me(SMITHS, ""))),
                (
                    200,
                    Some(&me(SMITHS, r#"{"family_id": 8, "family_name": "Petrovs"}"#)),
                ),
            ],
        )
        .on("/families/join", r#"{"status": "pending"}"#);
    let waiting = build(&pending)?;
    waiting.refresh()?;
    assert_eq!(None, drive(&waiting.clock, waiting.door.join("ABCD2345"))?);
    let state = waiting.session.state();
    assert_eq!(Some(3), state.current_family_id);
    assert_eq!(1, state.all_pending().len());
    assert_eq!("Petrovs", state.all_pending()[0].family_name);
    Ok(())
}

/// The join answer names its family (2026-10-22): joined, THAT family is shown — even when `/me`
/// brought another new one along — without comparing lists; waiting, the family shown stays.
#[test]
fn a_join_answer_that_names_its_family_shows_that_family() -> StoreResult<()> {
    const PETROVS: &str = r#"{"id": 8, "name": "Petrovs", "role": "member"}"#;
    let server = Server::new();
    server
        .then(
            "/me",
            &[
                (200, Some(&me(SMITHS, ""))),
                (
                    200,
                    Some(&me(&format!("{SMITHS}, {IVANOVS}, {PETROVS}"), "")),
                ),
            ],
        )
        .on("/families/join", r#"{"status": "joined", "family_id": 8}"#);
    let rig = build(&server)?;
    rig.refresh()?;
    assert_eq!(None, drive(&rig.clock, rig.door.join("ABCD2345"))?);
    assert_eq!(Some(8), rig.session.state().current_family_id);
    // Nothing but `/me` was asked to find it.
    assert!(
        !server
            .asked()
            .iter()
            .any(|path| path.starts_with("/api/v1/families/8"))
    );
    let waiting: JoinAnswer =
        serde_json::from_str(r#"{"status": "pending", "family_id": 8}"#).expect("an answer");
    assert_eq!(Some(8), waiting.family_id);
    let joined: JoinAnswer = serde_json::from_str(r#"{"status": "joined"}"#).expect("an answer");
    assert_eq!(None, joined.family_id);
    Ok(())
}

/// The three refusals of a further family, in their own words — the cap with its number.
#[test]
fn the_doors_say_why_they_are_shut() {
    let say = common::english();
    assert_eq!(
        "You've reached the most families one account can be in (5).",
        door_sentences::join(
            &ApiError::new(error_codes::TOO_MANY_FAMILIES, "cap", 409),
            &say,
            5,
            false
        )
    );
    assert_eq!(
        "You've reached the most families one account can be in (5).",
        door_sentences::create(
            &ApiError::new(error_codes::TOO_MANY_FAMILIES, "cap", 409),
            &say,
            5
        )
    );
    assert_eq!(
        "You already own a family, so you can't start another.",
        door_sentences::create(
            &ApiError::new(error_codes::ALREADY_OWN_A_FAMILY, "own", 409),
            &say,
            1
        )
    );
    assert_eq!(
        "This server doesn't let members start new families.",
        door_sentences::create(
            &ApiError::new(error_codes::MEMBER_FAMILIES_DISABLED, "off", 403),
            &say,
            1
        )
    );
    assert_eq!(
        Some(
            "This member is also in a family you don't own, so only they can change their password."
                .to_string()
        ),
        family_sentences::for_error(
            &ApiError::new(error_codes::MEMBER_IN_OTHER_FAMILIES, "no", 403),
            5,
            &say
        )
    );
    assert_eq!(
        None,
        family_sentences::for_error(&ApiError::new(error_codes::VALIDATION, "no", 400), 5, &say)
    );
    assert_eq!(
        "Start your own family. You'll be its owner and can invite members of your other families.",
        another_family::explanation(&say)
    );
}
