//! `ChatActionsTests`, who is typing (`TypingRoster`): momentary, in member order, and never the
//! reader's own or a blocked member's.

mod common;

use std::rc::Rc;
use std::time::Duration;

use fc_core::protocol::MessageDto;
use fc_core::store::StoreResult;
use fc_core::testing::FakeClock;
use fc_logic::typing::TypingRoster;
use time::macros::datetime;

use common::chat::{CHAT, ME, actions_rig};

#[test]
fn typing_is_momentary_in_member_order_and_never_mine_or_a_blocked_members() -> StoreResult<()> {
    let rig = actions_rig()?;
    let now = FakeClock::at(datetime!(2026-09-13 10:00 UTC));
    let roster = TypingRoster::new(
        rig.chats.clone(),
        Rc::new(now.clone()),
        Rc::new(common::english()),
    );

    roster.heard(CHAT, 12)?;
    roster.heard(CHAT, 11)?;
    roster.heard(CHAT, ME)?;
    roster.heard(CHAT, 13)?;
    assert_eq!("Bob, Carl are typing…", roster.line(CHAT)?);
    assert_eq!("", roster.line(99)?);

    roster.spoke(&MessageDto::new(
        5,
        CHAT,
        12,
        None,
        "here",
        "2026-09-13T10:00:00Z",
    ));
    assert_eq!("Bob is typing…", roster.line(CHAT)?);

    now.advance(TypingRoster::LASTS - Duration::from_millis(1));
    assert_eq!("Bob is typing…", roster.line(CHAT)?);
    now.advance(Duration::from_millis(1));
    assert_eq!("", roster.line(CHAT)?);

    roster.heard(CHAT, 40)?;
    assert_eq!("Someone is typing…", roster.line(CHAT)?);
    Ok(())
}
