//! `LookupsTests.TheOwnersSwitchIsTiedToNoOther`, whose subject is `FamilyModel`: vision going off
//! leaves the lookups switch where it was, and it goes out as its own key.

mod common;

use std::rc::Rc;

use fc_core::protocol::{FamilyDto, FamilyPatch};
use fc_core::store::{ChatStore, Database, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_logic::family::FamilyModel;
use time::macros::datetime;

use common::server::{Server, api};

#[test]
fn the_owners_switch_is_tied_to_no_other() -> StoreResult<()> {
    let on = FamilyDto {
        ai_vision: true,
        ai_faces: true,
        ai_lookups: true,
        ..FamilyDto::new(3, "The Smiths")
    };
    let patched = |family: &FamilyDto, patch: FamilyPatch| FamilyModel::as_applied(family, &patch);
    assert!(
        patched(
            &on,
            FamilyPatch {
                ai_vision: Some(false),
                ..FamilyPatch::default()
            }
        )
        .ai_lookups
    );
    assert!(
        !patched(
            &on,
            FamilyPatch {
                ai_lookups: Some(false),
                ..FamilyPatch::default()
            }
        )
        .ai_lookups
    );
    assert!(
        patched(
            &FamilyDto {
                ai_lookups: false,
                ..on.clone()
            },
            FamilyPatch {
                ai_lookups: Some(true),
                ..FamilyPatch::default()
            }
        )
        .ai_lookups
    );
    assert!(FamilyModel::may_turn_on(
        "ai_lookups",
        &FamilyDto::new(3, "The Smiths")
    ));

    let server = Server::new();
    server.on(
        "/families/mine",
        r#"{"family": {"id": 3, "name": "The Smiths", "ai_lookups": true}}"#,
    );
    let clock = FakeClock::at(datetime!(2026-10-03 09:30 UTC));
    let cache = Database::open_in_memory()?;
    let family = FamilyModel::new(
        api(&server, &clock),
        Rc::new(ChatStore::new(&cache).with_me(|| 7)),
    );
    let changed = drive(
        &clock,
        family.change(
            &FamilyDto::new(3, "The Smiths"),
            &FamilyPatch {
                ai_lookups: Some(true),
                ..FamilyPatch::default()
            },
        ),
    )
    .expect("changed");
    assert!(changed.ai_lookups);
    assert_eq!(vec!["{\"ai_lookups\":true}"], server.bodies());
    Ok(())
}
