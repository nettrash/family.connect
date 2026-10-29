//! `SeveralFamiliesTests.LeavingNamesTheFamily`, whose subject is `FamilyModel`: leaving names the
//! family left — the one shown — and only that family goes.

mod common;

use std::rc::Rc;

use fc_core::store::{ChatStore, Database, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_logic::family::FamilyModel;
use time::macros::datetime;

use common::server::{Server, api};

#[test]
fn leaving_names_the_family() -> StoreResult<()> {
    let server = Server::new();
    server.on_status("/families/5/leave", 204, None);
    let clock = FakeClock::at(datetime!(2026-10-22 10:00 UTC));
    let cache = Database::open_in_memory()?;
    let family = FamilyModel::new(api(&server, &clock), Rc::new(ChatStore::new(&cache)));

    assert_eq!(Ok(None), drive(&clock, family.leave(Some(5))));

    assert_eq!(vec!["/api/v1/families/5/leave"], server.asked());
    Ok(())
}
