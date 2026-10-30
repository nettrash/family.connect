//! `LookupsTests.TheSessionCarriesTheServersLookupStamp`, whose subject is `AppSession`: the session
//! carries the server's `assistant_lookup_consent_at`, and a later `/me` without it takes it away —
//! never assumed (docs/protocol.md, "Consenting to the assistant", amended 2026-10-03).

mod common;

use std::rc::Rc;

use fc_core::protocol::MemoryTokenStore;
use fc_core::store::{Database, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_logic::app_session::AppSession;
use fc_logic::lookups;
use time::macros::datetime;

use common::server::Server;
use common::session::api_with;

#[test]
fn the_session_carries_the_servers_lookup_stamp() -> StoreResult<()> {
    let me = r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
             "family": {"id": 3, "name": "The Smiths"}, "role": "member",
             "assistant_consent_at": "2026-09-19T08:12:04Z""#;
    let server = Server::new();
    server.then(
        "/me",
        &[
            (
                200,
                Some(&format!(
                    r#"{me}, "assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}}"#
                )),
            ),
            (200, Some(&format!("{me}}}"))),
        ],
    );
    let clock = FakeClock::at(datetime!(2026-10-03 10:00 UTC));
    let tokens = Rc::new(MemoryTokenStore::new(Some("t0ken")));
    let session = AppSession::new(
        api_with(Rc::new(server), &clock, tokens.clone()),
        tokens,
        Database::open_in_memory()?,
    );

    assert_eq!(None, drive(&clock, session.refresh())?);
    assert_eq!(
        Some("2026-10-03T09:30:00Z".to_string()),
        session.state().assistant_lookup_consent_at
    );
    assert!(lookups::allowed(&session.state()));

    assert_eq!(None, drive(&clock, session.refresh())?);
    assert_eq!(None, session.state().assistant_lookup_consent_at);
    assert!(!lookups::allowed(&session.state()));
    Ok(())
}
