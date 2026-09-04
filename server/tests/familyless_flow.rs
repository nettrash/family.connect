//! Integration: the familyless sweep (protocol.md, "Accounts without a
//! family"). An account that has had no family for longer than
//! `[families] familyless_account_ttl_days` is scrubbed exactly as
//! `POST /me/delete` would scrub it; members, pending joiners and the
//! assistant are left alone; leaving starts the clock again; 0 is off.

mod common;

use common::{TestServer, assert_error, spawn_server, spawn_server_with_config};
use family_connect::handlers_auth::{FamilylessGuard, Scrubbed, scrub_account};
use serde_json::{Value, json};

/// Age an account's familyless clock — the sweep reads `familyless_since`,
/// so that is what a test moves.
async fn age(ts: &TestServer, user_id: i64, days: i64) {
    sqlx::query(
        "UPDATE users SET familyless_since = now() - make_interval(days => $2) WHERE id = $1",
    )
    .bind(user_id)
    .bind(days as i32)
    .execute(&ts.state.pool)
    .await
    .expect("aged");
}

async fn sweep(ts: &TestServer) -> u64 {
    family_connect::handlers_auth::sweep_familyless_accounts(&ts.state)
        .await
        .expect("the sweep runs")
}

/// `familyless_since` as the row holds it: `None` for a member (and for
/// anybody else the sweep never considers).
async fn familyless_since(ts: &TestServer, user_id: i64) -> Option<time::OffsetDateTime> {
    sqlx::query_scalar("SELECT familyless_since FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("row")
}

async fn is_tombstone(ts: &TestServer, user_id: i64) -> bool {
    sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("row")
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_account_that_never_joins_is_removed_once_its_grace_has_passed() {
    let ts = spawn_server().await;
    let (drifter, drifter_id) = ts.register("drifter", "Drifter").await;

    // Fresh: the clock is running, but seven days have not.
    assert!(familyless_since(&ts, drifter_id).await.is_some());
    assert_eq!(
        sweep(&ts).await,
        0,
        "a fresh registrant is inside the grace"
    );
    assert_eq!(ts.get(&drifter, "/me").await.status(), 200);

    // The last hour of the grace is still the grace.
    age(&ts, drifter_id, 6).await;
    assert_eq!(sweep(&ts).await, 0);

    age(&ts, drifter_id, 8).await;
    assert_eq!(sweep(&ts).await, 1, "past the grace, the account goes");
    assert!(is_tombstone(&ts, drifter_id).await);
    // Exactly what a requested deletion does: the session is dead, the
    // password no longer works, and the username is free again.
    assert_eq!(ts.get(&drifter, "/me").await.status(), 401);
    assert_error(
        ts.login_raw("drifter", "password123").await,
        401,
        "invalid_credentials",
    )
    .await;
    let (_again, again_id) = ts.register("drifter", "Drifter again").await;
    assert_ne!(
        again_id, drifter_id,
        "a new account, not the old one revived"
    );

    // And the tombstone is not swept twice.
    assert_eq!(sweep(&ts).await, 0);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn members_pending_joiners_and_the_assistant_are_never_candidates() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Owner").await;
    let (_family_id, code) = ts.create_family(&owner, "The Smiths").await;
    // Creating stopped the owner's clock.
    assert!(familyless_since(&ts, owner_id).await.is_none());

    let (waiter, waiter_id) = ts.register("waiter", "Waiter").await;
    // Default policy is approval: the request parks as pending.
    ts.join(&waiter, &code, "pending").await;
    age(&ts, waiter_id, 30).await;
    assert_eq!(
        sweep(&ts).await,
        0,
        "a pending request is waiting on the owner, not the server"
    );
    assert_eq!(ts.get(&waiter, "/me").await.status(), 200);

    // Rejected, the request no longer shields them — and the clock that was
    // running all along has long run out.
    let request_id: i64 = sqlx::query_scalar(
        "SELECT id FROM join_requests WHERE user_id = $1 AND status = 'pending'",
    )
    .bind(waiter_id)
    .fetch_one(&ts.state.pool)
    .await
    .expect("the pending request");
    let rejected = ts
        .post(
            &owner,
            &format!("/families/join-requests/{request_id}/reject"),
            json!({}),
        )
        .await;
    assert_eq!(
        rejected.status(),
        204,
        "{}",
        rejected.text().await.unwrap_or_default()
    );
    assert_eq!(sweep(&ts).await, 1);
    assert!(is_tombstone(&ts, waiter_id).await);
    assert!(
        !is_tombstone(&ts, owner_id).await,
        "a member is never a candidate"
    );

    // The assistant has no family by design and is never a candidate: its
    // clock is not running, and it is excluded by name regardless.
    let assistant: Option<(i64, Option<time::OffsetDateTime>)> = sqlx::query_as(
        "SELECT id, familyless_since FROM users WHERE lower(username) = 'assistant'",
    )
    .fetch_optional(&ts.state.pool)
    .await
    .expect("query");
    let (assistant_id, clock) = assistant.expect("0015 inserted the assistant");
    assert!(clock.is_none());
    age(&ts, assistant_id, 400).await;
    assert_eq!(sweep(&ts).await, 0);
    assert!(!is_tombstone(&ts, assistant_id).await);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn leaving_starts_the_clock_again_however_old_the_account_is() {
    let ts = spawn_server().await;
    let (owner, _owner_id) = ts.register("owner", "Owner").await;
    let (_family_id, code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    let (member, member_id) = ts.register("member", "Member").await;
    ts.join(&member, &code, "joined").await;
    assert!(
        familyless_since(&ts, member_id).await.is_none(),
        "joining stopped the clock"
    );
    // However old the account: the sweep never reads `created_at`.
    sqlx::query("UPDATE users SET created_at = now() - make_interval(days => 400) WHERE id = $1")
        .bind(member_id)
        .execute(&ts.state.pool)
        .await
        .expect("aged the account itself");

    let left = ts.post(&member, "/families/leave", json!({})).await;
    assert_eq!(
        left.status(),
        204,
        "{}",
        left.text().await.unwrap_or_default()
    );
    let since = familyless_since(&ts, member_id)
        .await
        .expect("leaving started the clock");
    assert!(
        time::OffsetDateTime::now_utc() - since < time::Duration::minutes(5),
        "the clock starts at the leave, not at registration: {since}"
    );
    assert_eq!(
        sweep(&ts).await,
        0,
        "a member who left today has the whole grace to come back"
    );

    // Coming back inside the grace is the ordinary rejoin — the clock stops.
    ts.join(&member, &code, "joined").await;
    assert!(familyless_since(&ts, member_id).await.is_none());

    // Leaving again and staying away past the grace: the account goes. It
    // was NOT in the family when it went, so — exactly as a requested
    // deletion after a leave — it is not among that family's
    // `former_members` either; the roster simply no longer knows it, and a
    // client names its old messages the way it names any departed
    // member's (protocol.md, "Deleting an account").
    let left = ts.post(&member, "/families/leave", json!({})).await;
    assert_eq!(left.status(), 204);
    age(&ts, member_id, 8).await;
    assert_eq!(sweep(&ts).await, 1);
    assert!(is_tombstone(&ts, member_id).await);
    let family: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("family");
    assert!(
        family["members"]
            .as_array()
            .expect("members")
            .iter()
            .all(|m| m["id"].as_i64() != Some(member_id)),
        "a tombstone is never in `members`"
    );
    assert_eq!(family["former_members"], Value::Null);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn zero_turns_the_sweep_off_and_me_says_so() {
    let ts = spawn_server_with_config(|cfg| cfg.families.familyless_account_ttl_days = 0).await;
    let (drifter, drifter_id) = ts.register("drifter", "Drifter").await;
    age(&ts, drifter_id, 400).await;
    assert_eq!(sweep(&ts).await, 0);
    let me: Value = ts.get(&drifter, "/me").await.json().await.expect("me");
    assert_eq!(me["familyless_account_ttl_days"], 0);
    assert_eq!(me["family"], Value::Null);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn me_always_carries_the_grace_so_the_gate_can_say_it() {
    let ts = spawn_server().await;
    let (drifter, _) = ts.register("drifter", "Drifter").await;
    let me: Value = ts.get(&drifter, "/me").await.json().await.expect("me");
    assert_eq!(
        me["familyless_account_ttl_days"], 7,
        "the default, on the wire"
    );

    let ts = spawn_server_with_config(|cfg| cfg.families.familyless_account_ttl_days = 3).await;
    let (drifter, _) = ts.register("drifter", "Drifter").await;
    let me: Value = ts.get(&drifter, "/me").await.json().await.expect("me");
    assert_eq!(me["familyless_account_ttl_days"], 3);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_requested_deletion_still_works_through_the_shared_scrub() {
    // The refactor moved `POST /me/delete`'s body into `scrub_account`; the
    // account_flow suite covers the semantics, this pins the plumbing.
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Owner").await;
    ts.create_family(&owner, "The Smiths").await;
    let deleted = ts
        .post(&owner, "/me/delete", json!({"password": "password123"}))
        .await;
    assert_eq!(
        deleted.status(),
        204,
        "{}",
        deleted.text().await.unwrap_or_default()
    );
    assert!(is_tombstone(&ts, owner_id).await);
    assert!(
        familyless_since(&ts, owner_id).await.is_none(),
        "a tombstone's clock is blank"
    );
    assert_eq!(ts.get(&owner, "/me").await.status(), 401);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn every_way_out_of_a_family_starts_the_clock() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Owner").await;
    let (_family_id, code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    let (removed, removed_id) = ts.register("removed", "Removed").await;
    ts.join(&removed, &code, "joined").await;
    let (heir, heir_id) = ts.register("heir", "Heir").await;
    ts.join(&heir, &code, "joined").await;

    // Removed by the owner.
    let gone = ts
        .delete(&owner, &format!("/families/members/{removed_id}"))
        .await;
    assert_eq!(
        gone.status(),
        204,
        "{}",
        gone.text().await.unwrap_or_default()
    );
    assert!(
        familyless_since(&ts, removed_id).await.is_some(),
        "being removed starts the clock"
    );

    // The owner leaves and hands the family on.
    let left = ts.post(&owner, "/families/leave", json!({})).await;
    assert_eq!(
        left.status(),
        200,
        "{}",
        left.text().await.unwrap_or_default()
    );
    assert!(
        familyless_since(&ts, owner_id).await.is_some(),
        "an owner who hands the family on starts the clock"
    );
    assert!(
        familyless_since(&ts, heir_id).await.is_none(),
        "the heir is a member"
    );

    // The heir, now the sole member, leaves and the family goes with them:
    // the one departure the FK's SET NULL performs, so the clock has to be
    // started by hand there — a row with neither family nor clock would be
    // a live account the sweep could never see.
    let left = ts.post(&heir, "/families/leave", json!({})).await;
    assert_eq!(
        left.status(),
        204,
        "{}",
        left.text().await.unwrap_or_default()
    );
    assert!(
        familyless_since(&ts, heir_id).await.is_some(),
        "the sole owner leaving starts the clock"
    );
    age(&ts, heir_id, 8).await;
    age(&ts, owner_id, 8).await;
    age(&ts, removed_id, 8).await;
    assert_eq!(sweep(&ts).await, 3, "all three are past the grace");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_guard_re_reads_the_row_before_the_scrub_and_spares_a_late_joiner() {
    let ts = spawn_server().await;
    let (owner, _owner_id) = ts.register("owner", "Owner").await;
    let (_family_id, code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    let guard = Some(FamilylessGuard { ttl_days: 7 });

    // A member was never a candidate; the guard, asked directly, says so.
    let (member, member_id) = ts.register("member", "Member").await;
    ts.join(&member, &code, "joined").await;
    assert_eq!(
        scrub_account(&ts.state, member_id, guard)
            .await
            .expect("runs"),
        Scrubbed::Spared
    );
    assert!(!is_tombstone(&ts, member_id).await);

    // The race the guard exists for: the scan named them, and they joined
    // before the scrub reached them.
    let (late, late_id) = ts.register("late", "Late").await;
    age(&ts, late_id, 30).await;
    ts.join(&late, &code, "joined").await;
    assert_eq!(
        scrub_account(&ts.state, late_id, guard)
            .await
            .expect("runs"),
        Scrubbed::Spared
    );
    assert!(!is_tombstone(&ts, late_id).await);
    assert_eq!(ts.get(&late, "/me").await.status(), 200);

    // Or asked to join, which is waiting on the owner and not on the server.
    let ts2 = spawn_server().await;
    let (owner2, _) = ts2.register("owner", "Owner").await;
    let (_, code2) = ts2.create_family(&owner2, "The Joneses").await;
    let (asker, asker_id) = ts2.register("asker", "Asker").await;
    age(&ts2, asker_id, 30).await;
    ts2.join(&asker, &code2, "pending").await;
    assert_eq!(
        scrub_account(&ts2.state, asker_id, guard)
            .await
            .expect("runs"),
        Scrubbed::Spared
    );
    assert!(!is_tombstone(&ts2, asker_id).await);

    // Inside the grace: spared. Past it: done — and a second pass is Spared,
    // because a tombstone is not a candidate either.
    let (_drifter, drifter_id) = ts.register("drifter", "Drifter").await;
    age(&ts, drifter_id, 6).await;
    assert_eq!(
        scrub_account(&ts.state, drifter_id, guard)
            .await
            .expect("runs"),
        Scrubbed::Spared
    );
    age(&ts, drifter_id, 8).await;
    assert_eq!(
        scrub_account(&ts.state, drifter_id, guard)
            .await
            .expect("runs"),
        Scrubbed::Done
    );
    assert!(is_tombstone(&ts, drifter_id).await);
    assert_eq!(
        scrub_account(&ts.state, drifter_id, guard)
            .await
            .expect("runs"),
        Scrubbed::Spared
    );
}
