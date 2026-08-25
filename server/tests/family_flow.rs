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

/// The family's main language: the owner names it, every member sees it,
/// and it is ABSENT until somebody chooses — not `null` and not `"en"`
/// (protocol.md, "The family's language").
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_owner_names_the_familys_language_and_every_member_sees_it() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (_family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    let mine: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert!(
        mine["family"].get("language").is_none(),
        "a family that never chose has no language key: {}",
        mine["family"]
    );

    // Sent in a client's own casing; the canonical spelling comes back.
    let patched = ts
        .patch(&owner, "/families/mine", json!({"language": "sr-latn"}))
        .await;
    assert_eq!(patched.status(), 200);
    let body: Value = patched.json().await.expect("patch is JSON");
    assert_eq!(body["family"]["language"], "sr-Latn");
    // The policy was not in the request, so it is exactly as it was.
    assert_eq!(body["family"]["join_policy"], "open");

    // Both places a client bootstraps from agree, which is the point of
    // /me carrying the same object.
    let me: Value = ts.get(&member, "/me").await.json().await.expect("me");
    assert_eq!(me["family"]["language"], "sr-Latn");
    assert!(
        me["family"].get("invite_code").is_none(),
        "the language is shared; the invite code is still owner-only"
    );
    let mine: Value = ts
        .get(&member, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(mine["family"]["language"], "sr-Latn");

    // An explicit null clears it. An absent key would have left it alone,
    // which is the whole reason the two are different requests.
    let cleared: Value = ts
        .patch(&owner, "/families/mine", json!({"language": null}))
        .await
        .json()
        .await
        .expect("patch is JSON");
    assert!(
        cleared["family"].get("language").is_none(),
        "null clears it back to unset: {}",
        cleared["family"]
    );
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_language_outside_the_nine_is_refused_and_changes_nothing() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (_family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.join(&member, &invite_code, "pending").await;

    // Well-formed BCP 47 is not enough: nothing ships Portuguese, and a
    // family that set it would get answers they could not explain.
    for refused in ["klingon", "pt-BR", "en-GB", ""] {
        assert_error(
            ts.patch(&owner, "/families/mine", json!({"language": refused}))
                .await,
            400,
            "invalid_language",
        )
        .await;
    }

    // A good policy alongside a bad language changes NEITHER — the owner is
    // never left guessing which half of a request landed.
    assert_error(
        ts.patch(
            &owner,
            "/families/mine",
            json!({"join_policy": "open", "language": "klingon"}),
        )
        .await,
        400,
        "invalid_language",
    )
    .await;
    let mine: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(
        mine["family"]["join_policy"], "approval",
        "still the default"
    );

    // Only the owner. A pending applicant is not even in the family.
    assert_error(
        ts.patch(&member, "/families/mine", json!({"language": "ru"}))
            .await,
        403,
        "not_family_owner",
    )
    .await;

    // Nothing at all is a valid request that changes nothing.
    let empty = ts.patch(&owner, "/families/mine", json!({})).await;
    assert_eq!(empty.status(), 200);
    let body: Value = empty.json().await.expect("patch is JSON");
    assert_eq!(body["family"]["join_policy"], "approval");
    assert!(body["family"].get("language").is_none());
}

/// The switch that decides what an `@ai` mention may take with it: the
/// owner sets it, every member sees it, and it is ON until somebody says
/// otherwise (protocol.md, "Mentioning the assistant in the family chat").
///
/// ALWAYS present, unlike the language: a boolean with a real default has
/// no "unset" for a missing key to mean, and a client that had to guess one
/// would be guessing about what leaves the server.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_owner_decides_whether_a_mention_sees_the_family_chats_history() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    let mine: Value = ts
        .get(&member, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(
        mine["family"]["ai_history"], true,
        "on by default, and present without anybody having set it: {}",
        mine["family"]
    );

    // The owner turns it off. Nothing else in the request, so nothing else
    // moves.
    let patched = ts
        .patch(&owner, "/families/mine", json!({"ai_history": false}))
        .await;
    assert_eq!(patched.status(), 200);
    let body: Value = patched.json().await.expect("patch is JSON");
    let family = body["family"].clone();
    assert_eq!(
        family,
        json!({
            "id": family_id,
            "name": "The Smiths",
            "join_policy": "open",
            "created_at": family["created_at"],
            "invite_code": invite_code,
            "ai_history": false,
        }),
        "the whole Family object, exactly the shape in protocol.md — no language key, \
         because nobody has chosen one, and ai_history present because it always is"
    );

    // Both places a client bootstraps from agree, which is the point of
    // /me carrying the same object.
    let me: Value = ts.get(&member, "/me").await.json().await.expect("me");
    assert_eq!(me["family"]["ai_history"], false);
    let mine: Value = ts
        .get(&member, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(mine["family"]["ai_history"], false);

    // A member may read it and may not change it: this decides what their
    // own words can be used for, but the family has one owner.
    assert_error(
        ts.patch(&member, "/families/mine", json!({"ai_history": true}))
            .await,
        403,
        "not_family_owner",
    )
    .await;

    // An absent key leaves it alone, exactly as it leaves the policy alone.
    let other: Value = ts
        .patch(&owner, "/families/mine", json!({"language": "ru"}))
        .await
        .json()
        .await
        .expect("patch is JSON");
    assert_eq!(other["family"]["language"], "ru");
    assert_eq!(
        other["family"]["ai_history"], false,
        "a request that did not mention it did not change it"
    );

    // And back on again — there is no third state to get stuck in.
    let back: Value = ts
        .patch(&owner, "/families/mine", json!({"ai_history": true}))
        .await
        .json()
        .await
        .expect("patch is JSON");
    assert_eq!(back["family"]["ai_history"], true);
    assert_eq!(
        back["family"]["language"], "ru",
        "and the language survived"
    );
}
