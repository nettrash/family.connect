//! Integration: blocking a member (docs/protocol.md, "Blocking a member").
//!
//! The REST half — who may block whom, what the block list looks like on
//! the wire, and what survives leaving and deletion. The SILENCE half, that
//! nothing about a block ever reaches the person blocked, is proved over
//! the socket in `ws_flow.rs`.

mod common;

use common::{assert_error, spawn_server, spawn_server_with_config};
use serde_json::{Value, json};

/// Any member may block any other member of their family, and the list is
/// a complete state-set: ALWAYS present, `[]` when empty, so a second
/// device learns about the last unblock.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_member_blocks_and_unblocks_and_the_list_is_always_present() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    // Empty is `[]`, not absent — on both endpoints a client bootstraps
    // from.
    let me: Value = ts.get(&owner, "/me").await.json().await.expect("me");
    assert_eq!(
        me["blocked_user_ids"],
        json!([]),
        "always present, even empty: {me}"
    );
    let mine: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(mine["blocked_user_ids"], json!([]));

    let blocked = ts
        .put(
            &owner,
            &format!("/families/members/{member_id}/block"),
            json!({}),
        )
        .await;
    assert_eq!(blocked.status(), 204);

    let me: Value = ts.get(&owner, "/me").await.json().await.expect("me");
    assert_eq!(me["blocked_user_ids"], json!([member_id]));
    let mine: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(
        mine["blocked_user_ids"],
        json!([member_id]),
        "the two bootstrap reads agree: {mine}"
    );

    // Idempotent both ways.
    let again = ts
        .put(
            &owner,
            &format!("/families/members/{member_id}/block"),
            json!({}),
        )
        .await;
    assert_eq!(again.status(), 204, "blocking twice is still 204");

    let cleared = ts
        .delete(&owner, &format!("/families/members/{member_id}/block"))
        .await;
    assert_eq!(cleared.status(), 204);
    let cleared_again = ts
        .delete(&owner, &format!("/families/members/{member_id}/block"))
        .await;
    assert_eq!(
        cleared_again.status(),
        204,
        "clearing a block nobody set is still 204"
    );

    let me: Value = ts.get(&owner, "/me").await.json().await.expect("me");
    assert_eq!(
        me["blocked_user_ids"],
        json!([]),
        "back to empty, and still PRESENT — a vanishing key would never \
         tell a second device about the last unblock: {me}"
    );
}

/// **The owner is blockable.** There is no `require_owner` on this route
/// and that is the point: the affordance is weakest exactly where the
/// person you want to stop reading is the one with power.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_member_may_block_the_owner() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    let blocked = ts
        .put(
            &member,
            &format!("/families/members/{owner_id}/block"),
            json!({}),
        )
        .await;
    assert_eq!(blocked.status(), 204, "the owner has no special standing");

    let me: Value = ts.get(&member, "/me").await.json().await.expect("me");
    assert_eq!(me["blocked_user_ids"], json!([owner_id]));

    // And the owner's own powers are untouched by being blocked — a block
    // is a rendering preference, never a governance action. Reading the
    // join policy back proved nothing; exercising the powers does.
    let (member_id, _) = (ts.user_id(&member).await, ());
    assert_eq!(
        ts.post(
            &owner,
            &format!("/families/members/{member_id}/password"),
            json!({"new_password": "a-new-password"})
        )
        .await
        .status(),
        204,
        "the owner may still reset the password of somebody who blocked them"
    );
    assert_eq!(
        ts.post(&owner, "/families/invite-code/rotate", json!({}))
            .await
            .status(),
        200,
        "and still rotate the code"
    );
    assert_eq!(
        ts.delete(&owner, &format!("/families/members/{member_id}"))
            .await
            .status(),
        204,
        "and still remove them"
    );
}

/// The refusals, all of them aimed at the caller.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn blocking_yourself_a_stranger_or_from_no_family_is_refused() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (outsider, outsider_id) = ts.register("stranger", "Stranger").await;
    ts.create_family(&owner, "The Smiths").await;

    let myself = ts
        .put(
            &owner,
            &format!("/families/members/{owner_id}/block"),
            json!({}),
        )
        .await;
    assert_error(myself, 400, "cannot_block_self").await;

    // Somebody who exists but is not in this family — the same answer a
    // nonexistent id gets, exactly as the password reset does.
    let stranger = ts
        .put(
            &owner,
            &format!("/families/members/{outsider_id}/block"),
            json!({}),
        )
        .await;
    assert_error(stranger, 403, "not_same_family").await;
    let nobody = ts
        .put(&owner, "/families/members/999999/block", json!({}))
        .await;
    assert_error(nobody, 403, "not_same_family").await;

    // A caller in no family cannot block anybody.
    let family_less = ts
        .put(
            &outsider,
            &format!("/families/members/{owner_id}/block"),
            json!({}),
        )
        .await;
    assert_error(family_less, 409, "not_in_family").await;
}

/// A block is a PAIR, not a membership: it survives one of them leaving,
/// and the blocker can always lift it — even for somebody who is no longer
/// in the family, which is why unblock raises no membership errors at all.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_block_outlives_the_membership_and_can_still_be_lifted() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    ts.put(
        &owner,
        &format!("/families/members/{member_id}/block"),
        json!({}),
    )
    .await;

    ts.post(&member, "/families/leave", json!({})).await;

    // Still blocked: the block was never scoped to the family.
    let me: Value = ts.get(&owner, "/me").await.json().await.expect("me");
    assert_eq!(
        me["blocked_user_ids"],
        json!([member_id]),
        "a block survives the other side leaving: {me}"
    );

    // Blocking them AGAIN now would be `not_same_family` — but UNBLOCKING
    // must still work, or the blocker holds a permanent entry on their own
    // screen that they cannot clear.
    let reblock = ts
        .put(
            &owner,
            &format!("/families/members/{member_id}/block"),
            json!({}),
        )
        .await;
    assert_error(reblock, 403, "not_same_family").await;
    let lifted = ts
        .delete(&owner, &format!("/families/members/{member_id}/block"))
        .await;
    assert_eq!(
        lifted.status(),
        204,
        "unblock is scoped to the caller's own list, not to the roster"
    );
    let me: Value = ts.get(&owner, "/me").await.json().await.expect("me");
    assert_eq!(me["blocked_user_ids"], json!([]));
}

/// Deleting an account destroys the blocks it MADE and NOT the blocks made
/// AGAINST it: their messages stay in the family chat, and somebody else's
/// decision not to read them was never theirs to revoke.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn account_deletion_takes_the_blocks_it_made_and_leaves_the_rest() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (leaver, leaver_id) = ts.register("junior", "Junior").await;
    let (third, third_id) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&leaver, &invite_code, "joined").await;
    ts.join(&third, &invite_code, "joined").await;

    // The leaver blocks somebody, and somebody blocks the leaver.
    ts.put(
        &leaver,
        &format!("/families/members/{third_id}/block"),
        json!({}),
    )
    .await;
    ts.put(
        &owner,
        &format!("/families/members/{leaver_id}/block"),
        json!({}),
    )
    .await;

    let deleted = ts
        .post(&leaver, "/me/delete", json!({"password": "password123"}))
        .await;
    assert_eq!(deleted.status(), 204);

    // The block the owner made against the departed account STANDS.
    let me: Value = ts.get(&owner, "/me").await.json().await.expect("me");
    assert_eq!(
        me["blocked_user_ids"],
        json!([leaver_id]),
        "a block made against a deleted account is not theirs to revoke: {me}"
    );

    // And read the asymmetry straight off the table, because the endpoint
    // cannot see it: `third`'s list is [] whether or not the leaver's block
    // was deleted — they were the BLOCKED party, never the blocker.
    let made: i64 =
        sqlx::query_scalar("SELECT count(*) FROM member_blocks WHERE blocker_user_id = $1")
            .bind(leaver_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("count blocks made");
    let against: i64 =
        sqlx::query_scalar("SELECT count(*) FROM member_blocks WHERE blocked_user_id = $1")
            .bind(leaver_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("count blocks against");
    assert_eq!(made, 0, "the blocks the account MADE went with it");
    assert_eq!(
        against, 1,
        "the block made AGAINST it stands — never theirs to revoke by leaving"
    );
    let me: Value = ts.get(&third, "/me").await.json().await.expect("me");
    assert_eq!(me["blocked_user_ids"], json!([]));
    assert_ne!(owner_id, leaver_id);
}

/// `support_contact` is the operator's published escalation path, served
/// on `/me` and absent when unset.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn me_carries_the_operators_support_contact_when_one_is_configured() {
    let ts = spawn_server().await;
    let (user, _) = ts.register("solo", "Solo").await;
    let me: Value = ts.get(&user, "/me").await.json().await.expect("me");
    assert!(
        me["support_contact"].is_null(),
        "absent when the operator set none: {me}"
    );

    let ts = spawn_server_with_config(|cfg| {
        cfg.server.support_contact = Some("family-admin@example.com".to_string());
    })
    .await;
    let (user, _) = ts.register("solo", "Solo").await;
    let me: Value = ts.get(&user, "/me").await.json().await.expect("me");
    assert_eq!(me["support_contact"], "family-admin@example.com");
}

// ---------------------------------------------------------------- enforcement

use std::time::Duration;

async fn register_device(ts: &common::TestServer, token: &str, push_token: &str) {
    let response = ts
        .post(
            token,
            "/devices",
            json!({"platform": "ios", "push_token": push_token}),
        )
        .await;
    assert_eq!(response.status(), 201, "registering a device");
}

/// Wait until the push seam has been called at least `n` times; dispatch is
/// fire-and-forget, so a bare read races it.
async fn wait_for_push_calls(ts: &common::TestServer, n: usize) -> Vec<common::PushCall> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let calls = ts.push.calls();
        if calls.len() >= n {
            return calls;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the push seam was called {} times, expected {n}",
            calls.len()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// A block removes a candidate BEFORE the per-device rule runs: the blocker
/// is not woken for a blocked member's message, while a bystander still is.
///
/// The frame is NOT suppressed — only the push — but that half is proved
/// over the socket in `ws_flow.rs`; here we watch the push seam.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_blocked_members_message_does_not_wake_the_blocker() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (noisy, noisy_id) = ts.register("junior", "Junior").await;
    let (bystander, _) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&noisy, &invite_code, "joined").await;
    ts.join(&bystander, &invite_code, "joined").await;
    register_device(&ts, &owner, "owner-device").await;
    register_device(&ts, &bystander, "bystander-device").await;

    ts.put(
        &owner,
        &format!("/families/members/{noisy_id}/block"),
        json!({}),
    )
    .await;

    let chat_id = ts.family_chat_id(&noisy).await;
    ts.post_message(&noisy, chat_id, &uuid(), "anybody about?")
        .await;

    let calls = wait_for_push_calls(&ts, 1).await;
    let woken: Vec<String> = calls
        .iter()
        .flat_map(|c| c.devices.iter().map(|d| d.push_token.clone()))
        .collect();
    assert!(
        woken.contains(&"bystander-device".to_string()),
        "a member who blocked nobody is still woken: {woken:?}"
    );
    assert!(
        !woken.contains(&"owner-device".to_string()),
        "the blocker must not be woken by somebody they blocked: {woken:?}"
    );
}

/// THE ASSISTANT HOLE. When a blocked member mentions `@ai`, the answer is
/// a real message whose SENDER is the assistant, quoting the mention. A
/// filter keyed on the sender alone lets that answer light up the blocker's
/// phone for a thread they cannot read — so the quoted sender is consulted
/// too.
///
/// Proved here without an assistant configured, by the same shape: a reply
/// from an UNBLOCKED member quoting a BLOCKED one.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_reply_quoting_a_blocked_member_does_not_wake_the_blocker() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (blocked, blocked_id) = ts.register("junior", "Junior").await;
    let (quoter, _) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&blocked, &invite_code, "joined").await;
    ts.join(&quoter, &invite_code, "joined").await;

    let chat_id = ts.family_chat_id(&blocked).await;
    let quoted: Value = ts
        .post(
            &blocked,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": uuid(), "body": "something reply-worthy"}),
        )
        .await
        .json()
        .await
        .expect("message");
    let quoted_id = quoted["message"]["id"].as_i64().expect("id");

    // Block only AFTER the quoted message exists, and register the device
    // now so the earlier post cannot have woken it.
    ts.put(
        &owner,
        &format!("/families/members/{blocked_id}/block"),
        json!({}),
    )
    .await;
    register_device(&ts, &owner, "owner-device").await;

    // An unblocked member quotes the blocked one.
    let reply = ts
        .post(
            &quoter,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": uuid(),
                "body": "good point",
                "reply_to_message_id": quoted_id
            }),
        )
        .await;
    assert_eq!(reply.status(), 201);

    // Give the fire-and-forget dispatch a moment, then assert nothing woke
    // the blocker.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let woken: Vec<String> = ts
        .push
        .calls()
        .iter()
        .flat_map(|c| c.devices.iter().map(|d| d.push_token.clone()))
        .collect();
    assert!(
        !woken.contains(&"owner-device".to_string()),
        "a reply quoting a blocked member must not wake the blocker — this is \
         the shape an @ai answer takes: {woken:?}"
    );
}

/// The blocker's chat list drops a direct chat with somebody they blocked,
/// and re-opening it is refused — in ONE direction. The blocked member's
/// own list and their ability to open the chat are untouched.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_blocked_direct_chat_leaves_the_blockers_list_only() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    let opened = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await;
    assert_eq!(opened.status(), 200);

    let count_direct = |chats: &Value| -> usize {
        chats["chats"]
            .as_array()
            .expect("array")
            .iter()
            .filter(|c| c["chat"]["kind"] == "direct")
            .count()
    };
    let before: Value = ts.get(&owner, "/chats").await.json().await.expect("chats");
    assert_eq!(count_direct(&before), 1);

    ts.put(
        &owner,
        &format!("/families/members/{member_id}/block"),
        json!({}),
    )
    .await;

    let after: Value = ts.get(&owner, "/chats").await.json().await.expect("chats");
    assert_eq!(
        count_direct(&after),
        0,
        "gone from the BLOCKER's list: {after}"
    );
    let refused = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await;
    assert_error(refused, 409, "blocked").await;

    // ASYMMETRIC. The blocked member sees their chat and may still open it
    // — a same-family DM that suddenly 409'd would have no innocent
    // explanation and would announce the block outright.
    let theirs: Value = ts.get(&member, "/chats").await.json().await.expect("chats");
    assert_eq!(
        count_direct(&theirs),
        1,
        "the blocked member's list is untouched: {theirs}"
    );
    let still_opens = ts
        .post(&member, "/chats/direct", json!({"user_id": owner_id}))
        .await;
    assert_eq!(
        still_opens.status(),
        200,
        "the blocked member may go on opening it, and their messages simply reach nobody"
    );

    // Nothing was deleted: unblocking brings it back whole.
    ts.delete(&owner, &format!("/families/members/{member_id}/block"))
        .await;
    let restored: Value = ts.get(&owner, "/chats").await.json().await.expect("chats");
    assert_eq!(
        count_direct(&restored),
        1,
        "unblock restores it: {restored}"
    );
}

/// Family statistics drop the blocked member's ROW and leave the TOTALS
/// whole — the rows deliberately no longer add up, and the gap is the
/// block. A number two members can compare must be one number.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn stats_drop_the_blocked_members_row_but_not_the_totals() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&member).await;
    ts.post_message(&member, chat_id, &uuid(), "one").await;
    ts.post_message(&member, chat_id, &uuid(), "two").await;

    let before: Value = ts
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("stats");
    assert_eq!(before["totals"]["messages"], 2);
    assert_eq!(before["members"].as_array().map(Vec::len), Some(2));

    ts.put(
        &owner,
        &format!("/families/members/{member_id}/block"),
        json!({}),
    )
    .await;

    let after: Value = ts
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("stats");
    let ids = |v: &Value| -> Vec<i64> {
        v["members"]
            .as_array()
            .expect("array")
            .iter()
            .map(|m| {
                m["user_id"]
                    .as_i64()
                    .or_else(|| m["id"].as_i64())
                    .expect("id")
            })
            .collect()
    };
    assert!(
        !ids(&after).contains(&member_id),
        "the blocked member's row is gone — by identity, not by a count that \
         a `<>` typo would also satisfy: {after}"
    );
    assert_eq!(ids(&after).len(), 1, "and only theirs: {after}");
    assert_eq!(
        after["totals"]["messages"], 2,
        "totals are the FAMILY's numbers and are never projected: {after}"
    );
    assert_eq!(
        after["totals"]["members"], 2,
        "including totals.members — the gap IS the block: {after}"
    );

    // A third party sees the unprojected view, which is what makes the
    // totals comparable at all.
    let theirs: Value = ts
        .get(&member, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("stats");
    assert!(
        ids(&theirs).contains(&member_id),
        "a third party sees the unprojected view: {theirs}"
    );
    assert_eq!(ids(&theirs).len(), 2);
}

fn uuid() -> String {
    // A fresh v4 without pulling the crate into this file's imports.
    uuid::Uuid::new_v4().to_string()
}

/// **The blocker's hidden direct chat is closed on EVERY path they can
/// reach it by** — not merely unlisted (protocol.md, "Blocking a member").
///
/// The family chat is deliberately the opposite: a hidden row there still
/// arrives, still counts and still moves the read marker, because a
/// filtered feed freezes that marker into an oracle. Here there is no
/// hidden row to reveal and no cursor to freeze, so the chat is simply
/// gone — and a badge the blocker could never clear would itself be a
/// quantity that moved when they blocked somebody.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_blockers_hidden_direct_chat_is_closed_on_every_path() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    let opened: Value = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("direct chat");
    let dm = opened["chat"]["id"].as_i64().expect("chat id");
    ts.post_message(&member, dm, &uuid(), "before the block")
        .await;

    ts.put(
        &owner,
        &format!("/families/members/{member_id}/block"),
        json!({}),
    )
    .await;

    // Every REST path the blocker's client might still hold the id for.
    assert_error(
        ts.get(&owner, &format!("/chats/{dm}/messages")).await,
        409,
        "blocked",
    )
    .await;
    assert_error(
        ts.post(
            &owner,
            &format!("/chats/{dm}/messages"),
            json!({"client_msg_id": uuid(), "body": "hello?"}),
        )
        .await,
        409,
        "blocked",
    )
    .await;
    assert_error(
        ts.post(
            &owner,
            &format!("/chats/{dm}/read"),
            json!({"last_read_message_id": 1}),
        )
        .await,
        409,
        "blocked",
    )
    .await;

    // The blocked member is refused NOTHING. A same-family DM that
    // suddenly 409'd for them would announce the block outright.
    assert_eq!(
        ts.get(&member, &format!("/chats/{dm}/messages"))
            .await
            .status(),
        200
    );
    assert_eq!(
        ts.post_message(&member, dm, &uuid(), "still sending")
            .await
            .status(),
        201
    );

    // And the family chat is untouched for the blocker — the contrast that
    // makes this rule a scoped exception rather than a general filter.
    let family_chat = ts.family_chat_id(&owner).await;
    assert_eq!(
        ts.get(&owner, &format!("/chats/{family_chat}/messages"))
            .await
            .status(),
        200,
        "the family chat is never closed by a block"
    );

    // Unblocking restores every path at once, with the history intact.
    ts.delete(&owner, &format!("/families/members/{member_id}/block"))
        .await;
    let restored: Value = ts
        .get(&owner, &format!("/chats/{dm}/messages"))
        .await
        .json()
        .await
        .expect("messages");
    let bodies: Vec<&str> = restored["messages"]
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|m| m["body"].as_str())
        .collect();
    assert!(
        bodies.contains(&"before the block") && bodies.contains(&"still sending"),
        "nothing was destroyed, only the blocker's way back in: {bodies:?}"
    );
    let _ = owner_id;
}

/// A hidden direct chat contributes nothing to the blocker's badge. The
/// family chat still does — the count is the other half of the read marker,
/// and projecting one without the other desynchronises them.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_hidden_direct_chat_counts_towards_no_badge() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (aunt, _) = ts.register("aunt", "Aunt").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    ts.join(&aunt, &invite_code, "joined").await;

    let opened: Value = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("direct chat");
    let dm = opened["chat"]["id"].as_i64().expect("chat id");

    ts.put(
        &owner,
        &format!("/families/members/{member_id}/block"),
        json!({}),
    )
    .await;
    register_device(&ts, &owner, "owner-device").await;

    // Three unread in the hidden DM, then one from an unblocked member in
    // the family chat — whose push carries the badge we read.
    for _ in 0..3 {
        ts.post_message(&member, dm, &uuid(), "unread").await;
    }
    let family_chat = ts.family_chat_id(&aunt).await;
    ts.post_message(&aunt, family_chat, &uuid(), "visible")
        .await;

    let calls = wait_for_push_calls(&ts, 1).await;
    let badge = calls
        .iter()
        .find(|c| c.devices.iter().any(|d| d.push_token == "owner-device"))
        .map(|c| c.note.badge)
        .expect("the owner was woken by the unblocked member");
    assert_eq!(
        badge, 1,
        "only the family-chat message counts — a badge the blocker could \
         never clear would be a quantity that moved when they blocked somebody"
    );
}

/// **Nothing a blocked member reads names their blocker.** Every existing
/// assertion on `blocked_user_ids` is made from the BLOCKER's own session,
/// so a `blocked_by` widened to `blocker_user_id = $1 OR blocked_user_id =
/// $1` — the plausible "a block is a pair" reading — would hand the blocked
/// person the exact list of who blocked them, on the two endpoints every
/// client bootstraps from, with the whole suite still green.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn nothing_a_blocked_member_reads_names_their_blocker() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (bystander, _) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    ts.join(&bystander, &invite_code, "joined").await;

    ts.put(
        &owner,
        &format!("/families/members/{member_id}/block"),
        json!({}),
    )
    .await;

    // Read everything as the BLOCKED member.
    let me: Value = ts.get(&member, "/me").await.json().await.expect("me");
    assert_eq!(
        me["blocked_user_ids"],
        json!([]),
        "present and empty — never the list of who blocked them: {me}"
    );
    let mine: Value = ts
        .get(&member, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(mine["blocked_user_ids"], json!([]));
    // Their roster, chat list and statistics are all untouched.
    assert_eq!(mine["members"].as_array().map(Vec::len), Some(3));
    let stats: Value = ts
        .get(&member, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("stats");
    assert_eq!(stats["members"].as_array().map(Vec::len), Some(3));

    // And a third party learns nothing either.
    let theirs: Value = ts.get(&bystander, "/me").await.json().await.expect("me");
    assert_eq!(theirs["blocked_user_ids"], json!([]));
}

/// Blocking ONE person hides ONE chat. Two independent one-line deletions
/// in the `GET /chats` filter are each severe and each green today: dropping
/// the peer test empties the blocker's entire DM list, and dropping the
/// blocker test hides the chat from the wrong side.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_blocked_direct_chat_leaves_only_the_blockers_own_row() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (first, first_id) = ts.register("junior", "Junior").await;
    let (second, second_id) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&first, &invite_code, "joined").await;
    ts.join(&second, &invite_code, "joined").await;

    // The owner holds TWO direct chats, and blocks only one peer.
    ts.post(&owner, "/chats/direct", json!({"user_id": first_id}))
        .await;
    ts.post(&owner, "/chats/direct", json!({"user_id": second_id}))
        .await;
    ts.put(
        &owner,
        &format!("/families/members/{first_id}/block"),
        json!({}),
    )
    .await;

    let peers = |chats: &Value| -> Vec<i64> {
        chats["chats"]
            .as_array()
            .expect("array")
            .iter()
            .filter(|c| c["chat"]["kind"] == "direct")
            .map(|c| c["chat"]["peer_user_id"].as_i64().expect("peer"))
            .collect()
    };

    let mine: Value = ts.get(&owner, "/chats").await.json().await.expect("chats");
    assert_eq!(
        peers(&mine),
        vec![second_id],
        "exactly the blocked peer's row is gone, and only it: {mine}"
    );

    // The blocked peer's OWN list still holds the chat — the filter is on
    // the blocker's side, not the chat's.
    let theirs: Value = ts.get(&first, "/chats").await.json().await.expect("chats");
    assert!(
        !peers(&theirs).is_empty(),
        "the blocked member's list is untouched: {theirs}"
    );
}

/// A blocked member's BOARD NOTE does not wake the blocker — the seam the
/// board tests never touch, because `board_flow.rs` mentions blocking
/// nowhere.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_blocked_members_board_note_does_not_wake_the_blocker() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (noisy, noisy_id) = ts.register("junior", "Junior").await;
    let (bystander, _) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&noisy, &invite_code, "joined").await;
    ts.join(&bystander, &invite_code, "joined").await;
    register_device(&ts, &owner, "owner-device").await;
    register_device(&ts, &bystander, "bystander-device").await;

    ts.put(
        &owner,
        &format!("/families/members/{noisy_id}/block"),
        json!({}),
    )
    .await;

    let posted = ts
        .post(
            &noisy,
            "/families/mine/board/notes",
            json!({"text": "on the wall", "color": "yellow", "x": 10, "y": 10}),
        )
        .await;
    assert_eq!(posted.status(), 201);

    let calls = wait_for_push_calls(&ts, 1).await;
    let woken: Vec<String> = calls
        .iter()
        .flat_map(|c| c.devices.iter().map(|d| d.push_token.clone()))
        .collect();
    assert!(
        woken.contains(&"bystander-device".to_string()),
        "an unblocked member is still woken by a new note: {woken:?}"
    );
    assert!(
        !woken.contains(&"owner-device".to_string()),
        "the blocker is not woken by a blocked member's note: {woken:?}"
    );
}

/// **A block never narrows the owner's MODERATION pushes.** A join request
/// and a report wake the owner whoever they name, because being blocked by
/// the moderator is not a way to stop being moderated (protocol.md, "Push
/// notifications").
///
/// This is the exception to every other push gate in the system, and it
/// exists because the tempting gate is wrong in exactly one direction: it
/// would make the member an owner has stopped reading the member the owner
/// is never told about — including by third parties.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_block_never_narrows_the_owners_moderation_pushes() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (nuisance, nuisance_id) = ts.register("junior", "Junior").await;
    let (reporter, _) = ts.register("cousin", "Cousin").await;
    let (applicant, _) = ts.register("stranger", "Stranger").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&nuisance, &invite_code, "joined").await;
    ts.join(&reporter, &invite_code, "joined").await;
    register_device(&ts, &owner, "owner-device").await;

    // The owner stops reading the nuisance.
    ts.put(
        &owner,
        &format!("/families/members/{nuisance_id}/block"),
        json!({}),
    )
    .await;

    // A REPORT about the blocked member still wakes the owner.
    ts.post(
        &reporter,
        "/families/reports",
        json!({"reported_user_id": nuisance_id, "reason": "harassment"}),
    )
    .await;
    let calls = wait_for_push_calls(&ts, 1).await;
    assert!(
        calls.iter().any(|c| {
            c.note.event.kind() == "report"
                && c.devices.iter().any(|d| d.push_token == "owner-device")
        }),
        "a report about a blocked member must still wake the owner: {calls:?}"
    );

    // And a JOIN REQUEST does too. Switch to approval so one is created.
    ts.patch(&owner, "/families/mine", json!({"join_policy": "approval"}))
        .await;
    ts.join(&applicant, &invite_code, "pending").await;
    let calls = wait_for_push_calls(&ts, 2).await;
    assert!(
        calls.iter().any(|c| {
            c.note.event.kind() == "join_request"
                && c.devices.iter().any(|d| d.push_token == "owner-device")
        }),
        "a join request must wake the owner whoever it names: {calls:?}"
    );
}

/// The report push reaches the owner, and never the owner it NAMES. One
/// character — `==` to `!=` at the shield — both leaks the shielded report
/// ("somebody in this family reported ME") and silences every ordinary one.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_report_pushes_the_owner_and_never_the_owner_it_names() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (reporter, _) = ts.register("junior", "Junior").await;
    let (other, other_id) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&reporter, &invite_code, "joined").await;
    ts.join(&other, &invite_code, "joined").await;
    register_device(&ts, &owner, "owner-device").await;

    // A report naming the OWNER pushes nobody.
    ts.post(
        &reporter,
        "/families/reports",
        json!({"reported_user_id": owner_id, "reason": "harassment"}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        ts.push.calls().is_empty(),
        "the owner must never be told somebody reported them: {:?}",
        ts.push.calls()
    );

    // An ordinary report does.
    ts.post(
        &reporter,
        "/families/reports",
        json!({"reported_user_id": other_id, "reason": "spam"}),
    )
    .await;
    let calls = wait_for_push_calls(&ts, 1).await;
    let report_pushes: Vec<_> = calls
        .iter()
        .filter(|c| c.note.event.kind() == "report")
        .collect();
    assert_eq!(
        report_pushes.len(),
        1,
        "exactly the ordinary one: {calls:?}"
    );
    assert!(
        report_pushes[0]
            .note
            .body
            .to_lowercase()
            .contains("new report"),
        "and it carries no reported text — the excerpt is the very content \
         somebody asked to have looked at: {:?}",
        report_pushes[0].note
    );
}

/// **The family chat is never projected per caller.** A hidden row still
/// arrives, still counts toward `unread_count`, may still be
/// `last_message`, and the blocker's read marker still advances THROUGH it.
///
/// Filtering any of that would freeze the marker at the id before a blocked
/// message — and a marker that leaps forward the moment a third member
/// posts is a perfect, repeatable oracle for the blocked person watching
/// the other end.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_family_chat_row_and_history_are_never_projected() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (noisy, noisy_id) = ts.register("junior", "Junior").await;
    let (cousin, _) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&noisy, &invite_code, "joined").await;
    ts.join(&cousin, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;

    let mut ids = Vec::new();
    for n in 0..3 {
        let m: Value = ts
            .post_message(&noisy, chat_id, &uuid(), &format!("before {n}"))
            .await
            .json()
            .await
            .expect("m");
        ids.push(m["message"]["id"].as_i64().expect("id"));
    }
    ts.put(
        &owner,
        &format!("/families/members/{noisy_id}/block"),
        json!({}),
    )
    .await;
    for n in 0..3 {
        let m: Value = ts
            .post_message(&noisy, chat_id, &uuid(), &format!("after {n}"))
            .await
            .json()
            .await
            .expect("m");
        ids.push(m["message"]["id"].as_i64().expect("id"));
    }
    ts.post_message(&cousin, chat_id, &uuid(), "from the cousin")
        .await;

    // HISTORY: all seven, the blocked member's ids present and contiguous.
    let history: Value = ts
        .get(&owner, &format!("/chats/{chat_id}/messages?limit=50"))
        .await
        .json()
        .await
        .expect("messages");
    let got: Vec<i64> = history["messages"]
        .as_array()
        .expect("array")
        .iter()
        .map(|m| m["id"].as_i64().expect("id"))
        .collect();
    assert_eq!(
        got.len(),
        7,
        "the server never filters history — a short page reads as the end of \
         the feed and freezes the read cursor: {history}"
    );
    for id in &ids {
        assert!(got.contains(id), "hidden id {id} still delivered: {got:?}");
    }

    // THE CHAT ROW: the count is the other half of the read marker, so it
    // counts hidden rows too, and the preview may be a hidden member's.
    let chats: Value = ts.get(&owner, "/chats").await.json().await.expect("chats");
    let row = chats["chats"]
        .as_array()
        .expect("array")
        .iter()
        .find(|c| c["chat"]["id"] == chat_id)
        .expect("the family row")
        .clone();
    assert_eq!(row["unread_count"], 7, "every message counts: {row}");

    // THE MARKER: it advances through a hidden message, and the count
    // follows it down.
    let marked = ts
        .post(
            &owner,
            &format!("/chats/{chat_id}/read"),
            json!({"last_read_message_id": ids[4]}),
        )
        .await;
    assert_eq!(
        marked.status(),
        204,
        "reading through a hidden row is allowed"
    );
    let chats: Value = ts.get(&owner, "/chats").await.json().await.expect("chats");
    let row = chats["chats"]
        .as_array()
        .expect("array")
        .iter()
        .find(|c| c["chat"]["id"] == chat_id)
        .expect("the family row")
        .clone();
    assert_eq!(
        row["last_read_message_id"], ids[4],
        "the marker landed on a hidden id: {row}"
    );
    assert_eq!(row["unread_count"], 2, "and the count followed it: {row}");
}
