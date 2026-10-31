//! Paid families on the wire (docs/protocol.md, "Paid families", 2026-10-30): what `GET /me`, a
//! `Family`, the statistics and two frames carry, read as the protocol writes them — and, above
//! all, an absent key read as the old behaviour, because every server from before says nothing.

use std::cell::RefCell;
use std::rc::Rc;

use fc_core::protocol::{
    AiErrorReason, AllowanceUseDto, FamilyDto, FrameRouter, MeResponse, ServerFrame, StatsResponse,
    wire,
};
use fc_core::store::{BoardStore, ChatStore, Database, StoreResult};
use time::macros::datetime;

#[test]
fn a_server_from_before_charges_for_nothing() {
    let me: MeResponse =
        wire::decode(r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#)
            .expect("GET /me");
    assert!(!me.subscriptions_enabled);
    assert_eq!(me.subscription, None);
    assert_eq!(me.paid_family_limits, None);
    assert_eq!(me.unpaid_days, None);
    assert!(!me.subscription_active());
    assert!(!MeResponse::default().subscriptions_enabled);
    let family: FamilyDto = wire::decode(r#"{"id": 3, "name": "The Smiths"}"#).expect("a family");
    assert_eq!(
        (family.member_ceiling, family.plan, family.delete_at),
        (None, None, None)
    );
}

#[test]
fn me_reads_the_switch_the_subscription_and_the_limits() {
    let me: MeResponse = wire::decode(
        r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
            "subscriptions_enabled": true,
            "subscription_products": ["me.nettrash.FamilyConnect.family.monthly"],
            "app_account_token": "6f1c0000-0000-4000-8000-000000000000",
            "unpaid_days": 14,
            "paid_family_limits": {"members": 25, "storage_bytes": 10737418240,
                                   "ai_tokens": 2000000, "images": 30},
            "subscription": {"product_id": "me.nettrash.FamilyConnect.family.yearly",
                             "environment": "Sandbox", "expires_at": "2026-12-01T00:00:00Z",
                             "active": true, "renews": false}}"#,
    )
    .expect("GET /me");
    assert!(me.subscriptions_enabled);
    assert!(me.subscription_active());
    assert_eq!(me.unpaid_days, Some(14));
    let held = me.subscription.as_ref().expect("a subscription");
    assert_eq!(held.environment, "Sandbox");
    assert_eq!(held.renews, Some(false));
    assert_eq!(held.expires_at.as_deref(), Some("2026-12-01T00:00:00Z"));
    let limits = me.paid_family_limits.expect("the limits");
    assert_eq!(limits.members, Some(25));
    assert_eq!(limits.images, Some(30));
    // A limit that is off is absent, and stays absent.
    assert_eq!(limits.transcript_ms, None);
}

#[test]
fn an_ended_or_absent_subscription_is_not_active() {
    let ended: MeResponse = wire::decode(
        r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
            "subscriptions_enabled": true,
            "subscription": {"product_id": "x", "environment": "Production",
                             "expires_at": "2026-10-01T00:00:00Z", "active": false}}"#,
    )
    .expect("GET /me");
    assert!(ended.subscriptions_enabled);
    assert!(!ended.subscription_active());
    assert_eq!(ended.subscription.and_then(|s| s.renews), None);
    let none: MeResponse = wire::decode(
        r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
            "subscriptions_enabled": true, "subscription": null}"#,
    )
    .expect("GET /me");
    assert!(!none.subscription_active());
}

#[test]
fn a_family_reads_its_plan_its_ceiling_and_its_deadline() {
    let family: FamilyDto = wire::decode(
        r#"{"id": 3, "name": "The Smiths", "plan": "paid", "member_ceiling": 25,
            "delete_at": "2026-11-14T09:00:00Z"}"#,
    )
    .expect("a family");
    assert_eq!(family.plan.as_deref(), Some("paid"));
    assert_eq!(family.member_ceiling, Some(25));
    assert_eq!(family.delete_at.as_deref(), Some("2026-11-14T09:00:00Z"));
    // And writes them back only when they are there (the cache keeps `GET /me` as JSON).
    let written = serde_json::to_string(&FamilyDto::new(3, "The Smiths")).expect("JSON");
    assert!(!written.contains("delete_at") && !written.contains("plan"));
}

#[test]
fn the_statistics_read_a_paid_familys_allowance() {
    let stats: StatsResponse = wire::decode(
        r#"{"totals": {"members": 2, "messages": 10, "board_notes": 0},
            "allowance": {"renews_at": "2026-12-01T00:00:00Z",
                          "storage_bytes": {"used": 3221225472, "limit": 10737418240},
                          "ai_tokens": {"used": 812345, "limit": 2000000},
                          "images": {"used": 4, "limit": 30},
                          "transcript_ms": {"used": 600000, "limit": 7200000}}}"#,
    )
    .expect("the statistics");
    let allowance = stats.allowance.expect("an allowance");
    assert_eq!(allowance.renews_at.as_deref(), Some("2026-12-01T00:00:00Z"));
    assert_eq!(
        allowance.images,
        Some(AllowanceUseDto { used: 4, limit: 30 })
    );
    assert_eq!(
        allowance.storage_bytes.map(|s| s.limit),
        Some(10_737_418_240)
    );
    let free: StatsResponse =
        wire::decode(r#"{"totals": {"members": 2, "messages": 10, "board_notes": 0}}"#)
            .expect("the statistics");
    assert_eq!(free.allowance, None);
}

#[test]
fn a_family_plan_frame_names_its_family_and_may_clear_the_deadline() {
    assert_eq!(
        ServerFrame::parse(
            r#"{"type": "family_plan", "family_id": 3, "plan": "paid",
                "delete_at": "2026-11-14T09:00:00Z"}"#
        ),
        Some(ServerFrame::FamilyPlan {
            family_id: 3,
            plan: Some("paid".into()),
            delete_at: Some("2026-11-14T09:00:00Z".into()),
        })
    );
    // Covered again: no `delete_at`, which is what clears it.
    assert_eq!(
        ServerFrame::parse(r#"{"type": "family_plan", "family_id": 3, "plan": "paid"}"#),
        Some(ServerFrame::FamilyPlan {
            family_id: 3,
            plan: Some("paid".into()),
            delete_at: None,
        })
    );
    // A null, or a wrong type, reads as absent too.
    assert_eq!(
        ServerFrame::parse(r#"{"type": "family_plan", "family_id": 3, "delete_at": null}"#),
        Some(ServerFrame::FamilyPlan {
            family_id: 3,
            plan: None,
            delete_at: None,
        })
    );
    // About no family: nothing to apply it to.
    assert_eq!(
        ServerFrame::parse(r#"{"type": "family_plan", "plan": "paid"}"#),
        None
    );
}

#[test]
fn an_allowance_used_failure_says_when_it_renews() {
    assert_eq!(
        ServerFrame::parse(
            r#"{"type": "ai_error", "chat_id": 42, "message_id": 1339,
                "reason": "allowance_used", "renews_at": "2026-12-01T00:00:00Z"}"#
        ),
        Some(ServerFrame::AiError {
            chat_id: 42,
            message_id: 1339,
            reason: Some(AiErrorReason::AllowanceUsed {
                renews_at: Some(datetime!(2026-12-01 00:00 UTC)),
            }),
        })
    );
    // A stamp that cannot be read keeps the reason, without its day.
    assert_eq!(
        ServerFrame::parse(
            r#"{"type": "ai_error", "chat_id": 42, "message_id": 1339,
                "reason": "allowance_used", "renews_at": "soon"}"#
        ),
        Some(ServerFrame::AiError {
            chat_id: 42,
            message_id: 1339,
            reason: Some(AiErrorReason::AllowanceUsed { renews_at: None }),
        })
    );
    // An unknown reason is still absent — `renews_at` beside it changes nothing.
    assert_eq!(
        ServerFrame::parse(
            r#"{"type": "ai_error", "chat_id": 42, "message_id": 1339,
                "reason": "allowance_spent", "renews_at": "2026-12-01T00:00:00Z"}"#
        ),
        Some(ServerFrame::AiError {
            chat_id: 42,
            message_id: 1339,
            reason: None,
        })
    );
}

#[test]
fn the_router_hands_a_family_plan_to_the_session() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let chats = Rc::new(ChatStore::new(&database).with_me(|| 7));
    let board = Rc::new(BoardStore::new(&database));
    let router = FrameRouter::new(chats, board, None);
    let heard = Rc::new(RefCell::new(Vec::new()));
    let into = Rc::clone(&heard);
    router
        .on_family_plan()
        .subscribe(move |plan| into.borrow_mut().push(plan.clone()));
    for frame in [
        r#"{"type": "family_plan", "family_id": 3, "plan": "paid", "delete_at": "2026-11-14T09:00:00Z"}"#,
        r#"{"type": "family_plan", "family_id": 3, "plan": "paid"}"#,
    ] {
        router.hear(&ServerFrame::parse(frame).expect("a frame"))?;
    }
    assert_eq!(
        *heard.borrow(),
        [
            (
                3,
                Some("paid".to_string()),
                Some("2026-11-14T09:00:00Z".to_string())
            ),
            (3, Some("paid".to_string()), None),
        ]
    );
    Ok(())
}
