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

    // And the owner's own powers are untouched by being blocked.
    let mine: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(mine["family"]["join_policy"], "open");
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

    // And the third member, whom the leaver had blocked, is untouched —
    // the row that went was the departed account's own preference.
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
    assert_eq!(
        after["members"].as_array().map(Vec::len),
        Some(1),
        "the blocked member's row is gone: {after}"
    );
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
    assert_eq!(theirs["members"].as_array().map(Vec::len), Some(2));
}

fn uuid() -> String {
    // A fresh v4 without pulling the crate into this file's imports.
    uuid::Uuid::new_v4().to_string()
}
