//! Integration: family lifecycle — create, join (both policies), invite-code
//! rotation, approval/rejection, leaving, and member removal.

mod common;

use common::{assert_error, spawn_server};
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn creating_a_family_makes_the_caller_owner_with_a_visible_invite_code() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive Owner").await;

    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    assert_eq!(invite_code.len(), 8);

    // A second create conflicts.
    let again = ts
        .post(&owner, "/families", json!({"name": "Another"}))
        .await;
    assert_error(again, 409, "already_in_family").await;

    // /me shows the family, the owner role, and (owner-only) the code.
    let me: Value = ts
        .get(&owner, "/me")
        .await
        .json()
        .await
        .expect("me is JSON");
    assert_eq!(me["family"]["id"], family_id);
    assert_eq!(me["family"]["name"], "The Smiths");
    assert_eq!(me["family"]["join_policy"], "approval", "default policy");
    assert_eq!(me["family"]["invite_code"], invite_code.as_str());
    assert_eq!(me["role"], "owner");

    // /families/mine lists the sole member as owner.
    let mine: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("mine is JSON");
    let members = mine["members"].as_array().expect("members array");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["id"], owner_id);
    assert_eq!(members[0]["role"], "owner");

    // The family chat exists immediately, titled after the family.
    let chats: Value = ts.get(&owner, "/chats").await.json().await.expect("chats");
    let family_chat = &chats["chats"][0];
    assert_eq!(family_chat["chat"]["kind"], "family");
    assert_eq!(family_chat["chat"]["title"], "The Smiths");
    assert_eq!(family_chat["chat"]["peer_user_id"], Value::Null);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_approval_policy_routes_joins_through_owner_decisions() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;

    // Default policy is approval: joining leaves a pending request.
    ts.join(&member, &invite_code, "pending").await;
    let me: Value = ts.get(&member, "/me").await.json().await.expect("me");
    assert_eq!(me["family"], Value::Null);
    assert_eq!(me["pending_join_request"]["family_name"], "The Smiths");

    // Re-joining while pending conflicts; /families/mine is still a 404.
    let again = ts
        .post(
            &member,
            "/families/join",
            json!({"invite_code": invite_code}),
        )
        .await;
    assert_error(again, 409, "join_request_pending").await;
    let mine = ts.get(&member, "/families/mine").await;
    assert_error(mine, 404, "not_in_family").await;

    // Only the owner sees the queue.
    let forbidden = ts.get(&member, "/families/join-requests").await;
    assert_error(forbidden, 403, "not_family_owner").await;
    let requests: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("requests");
    let list = requests["requests"].as_array().expect("array");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["user"]["id"], member_id);
    let request_id = list[0]["id"].as_i64().expect("request id");

    // Approve: the member lands in the family with the member role.
    let approved = ts
        .post(
            &owner,
            &format!("/families/join-requests/{request_id}/approve"),
            json!({}),
        )
        .await;
    assert_eq!(approved.status(), 200);
    let body: Value = approved.json().await.expect("approve body");
    assert_eq!(body["member"]["id"], member_id);
    assert_eq!(body["member"]["role"], "member");

    let me: Value = ts.get(&member, "/me").await.json().await.expect("me");
    assert_eq!(me["family"]["name"], "The Smiths");
    assert_eq!(me["role"], "member");
    assert_eq!(
        me["family"].get("invite_code"),
        None,
        "invite code is owner-only"
    );
    assert_eq!(me["pending_join_request"], Value::Null);

    // The member is in the family chat now.
    ts.family_chat_id(&member).await;

    // Approving the same request twice conflicts.
    let again = ts
        .post(
            &owner,
            &format!("/families/join-requests/{request_id}/approve"),
            json!({}),
        )
        .await;
    assert_error(again, 409, "join_request_not_pending").await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn rejecting_a_request_leaves_the_applicant_family_less_with_no_pending_marker() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (applicant, _) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.join(&applicant, &invite_code, "pending").await;

    let requests: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("requests");
    let request_id = requests["requests"][0]["id"].as_i64().expect("id");

    let rejected = ts
        .post(
            &owner,
            &format!("/families/join-requests/{request_id}/reject"),
            json!({}),
        )
        .await;
    assert_eq!(rejected.status(), 204);

    // Neither family nor pending request: the client infers rejection.
    let me: Value = ts.get(&applicant, "/me").await.json().await.expect("me");
    assert_eq!(me["family"], Value::Null);
    assert_eq!(me["pending_join_request"], Value::Null);

    let again = ts
        .post(
            &owner,
            &format!("/families/join-requests/{request_id}/reject"),
            json!({}),
        )
        .await;
    assert_error(again, 409, "join_request_not_pending").await;

    // Rejected applicants may ask again.
    ts.join(&applicant, &invite_code, "pending").await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_open_policy_admits_members_immediately_and_codes_rotate() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;

    // Policy changes are owner-only.
    let forbidden = ts
        .patch(&member, "/families/mine", json!({"join_policy": "open"}))
        .await;
    assert_error(forbidden, 403, "not_family_owner").await;
    let bad_policy = ts
        .patch(
            &owner,
            "/families/mine",
            json!({"join_policy": "invite-only"}),
        )
        .await;
    assert_eq!(bad_policy.status(), 400);
    ts.set_open_policy(&owner).await;

    // Lowercase input is accepted — codes are read aloud and retyped.
    ts.join(&member, &invite_code.to_lowercase(), "joined")
        .await;

    // Garbage codes are a 404.
    let unknown = ts
        .post(
            &member,
            "/families/join",
            json!({"invite_code": "XXXXXXXX"}),
        )
        .await;
    assert_error(unknown, 404, "invalid_invite_code").await;

    // Rotation: the old code dies, the new one works; owner-only.
    let forbidden = ts
        .post(&member, "/families/invite-code/rotate", json!({}))
        .await;
    assert_error(forbidden, 403, "not_family_owner").await;
    let rotated: Value = ts
        .post(&owner, "/families/invite-code/rotate", json!({}))
        .await
        .json()
        .await
        .expect("rotate body");
    let new_code = rotated["invite_code"]
        .as_str()
        .expect("new code")
        .to_string();
    assert_ne!(new_code, invite_code);

    let (third, _) = ts.register("third", "Third").await;
    let stale = ts
        .post(
            &third,
            "/families/join",
            json!({"invite_code": invite_code}),
        )
        .await;
    assert_error(stale, 404, "invalid_invite_code").await;
    ts.join(&third, &new_code, "joined").await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn leaving_and_removal_respect_the_owner_guards() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    // The owner cannot leave while others remain, and cannot remove himself.
    let blocked = ts.post(&owner, "/families/leave", json!({})).await;
    assert_error(blocked, 409, "owner_cannot_leave").await;
    let self_removal = ts
        .delete(&owner, &format!("/families/members/{owner_id}"))
        .await;
    assert_error(self_removal, 409, "cannot_remove_owner").await;

    // Members cannot remove anyone.
    let forbidden = ts
        .delete(&member, &format!("/families/members/{owner_id}"))
        .await;
    assert_error(forbidden, 403, "not_family_owner").await;

    // A member leaves; their membership and chat access are gone.
    let left = ts.post(&member, "/families/leave", json!({})).await;
    assert_eq!(left.status(), 204);
    let me: Value = ts.get(&member, "/me").await.json().await.expect("me");
    assert_eq!(me["family"], Value::Null);
    let chats: Value = ts.get(&member, "/chats").await.json().await.expect("chats");
    assert_eq!(chats["chats"].as_array().expect("array").len(), 0);
    let not_in = ts.post(&member, "/families/leave", json!({})).await;
    assert_error(not_in, 409, "not_in_family").await;

    // Rejoin, then get removed by the owner.
    ts.join(&member, &invite_code, "joined").await;
    let removed = ts
        .delete(&owner, &format!("/families/members/{member_id}"))
        .await;
    assert_eq!(removed.status(), 204);
    let gone = ts
        .delete(&owner, &format!("/families/members/{member_id}"))
        .await;
    assert_error(gone, 404, "user_not_found").await;

    // Now sole, the owner may leave — which deletes the family entirely.
    let dissolved = ts.post(&owner, "/families/leave", json!({})).await;
    assert_eq!(dissolved.status(), 204);
    let me: Value = ts.get(&owner, "/me").await.json().await.expect("me");
    assert_eq!(me["family"], Value::Null);
    let dead_code = ts
        .post(
            &member,
            "/families/join",
            json!({"invite_code": invite_code}),
        )
        .await;
    assert_error(dead_code, 404, "invalid_invite_code").await;
}
