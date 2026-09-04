//! Integration: family lifecycle — create, join (both policies), invite-code
//! rotation, approval/rejection, leaving, and member removal.

mod common;

use common::{assert_error, spawn_server, spawn_server_with_config};
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

    // The owner cannot remove HIMSELF — that is still a refusal, and it is
    // a different question from leaving, which now hands the family on
    // (see the hand-off tests below).
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
/// `max_members` lives in FOUR separate `families` column lists — the shared
/// SELECT, the create RETURNING, the patch RETURNING, and the fourth,
/// hand-written SELECT behind `/me`. `FamilyRecord::from_row` reads them
/// untyped, so a list that forgot the column is a runtime 500 from ONE
/// endpoint and a green compile everywhere, which is why this walks all four
/// rather than trusting the type checker.
///
/// It also pins the shape a cap has before anybody sets one: the key is
/// ABSENT, not null and not the operator's ceiling. Absent means "this family
/// never chose", and a client that saw the ceiling here could not tell that
/// from a family that chose exactly it (protocol.md, "Families").
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_member_cap_is_absent_until_set_and_reads_from_all_four_column_lists() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;

    // 1. the create RETURNING
    let created: Value = ts
        .post(&owner, "/families", json!({"name": "The Smiths"}))
        .await
        .json()
        .await
        .expect("create is JSON");
    assert!(
        created["family"].get("max_members").is_none(),
        "a brand-new family has no cap of its own: {}",
        created["family"]
    );
    let invite_code = created["family"]["invite_code"]
        .as_str()
        .expect("owner sees the code")
        .to_string();
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    // 2. the shared SELECT_FAMILY, read by a non-owner
    let mine: Value = ts
        .get(&member, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert!(
        mine["family"].get("max_members").is_none(),
        "absent for a member too: {}",
        mine["family"]
    );

    // 3. the patch RETURNING — a patch that does not mention the cap
    let patched: Value = ts
        .patch(&owner, "/families/mine", json!({"ai_history": false}))
        .await
        .json()
        .await
        .expect("patch is JSON");
    assert!(
        patched["family"].get("max_members").is_none(),
        "a patch about something else leaves the cap alone and absent: {}",
        patched["family"]
    );

    // 4. the fourth list: /me's own hand-written SELECT and Family literal
    let me: Value = ts.get(&member, "/me").await.json().await.expect("me");
    assert!(
        me["family"].get("max_members").is_none(),
        "/me must agree with /families/mine about the same family: {}",
        me["family"]
    );
}

/// The cap is only real if it survives a race. `grant_membership` counts
/// the room and then inserts, which are two statements with a gap in the
/// middle: without `SELECT ... FOR UPDATE` on the family row, several
/// simultaneous joins into a family with ONE seat left all read "one fewer
/// than the cap" and all insert.
///
/// The FK's implicit `FOR KEY SHARE` does not close this — it does not
/// conflict with itself — so this test is the only thing standing between a
/// cap of 2 and a family of 6.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn simultaneous_joins_cannot_overfill_a_family_by_one_seat() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    // Owner plus exactly one more.
    ts.patch(&owner, "/families/mine", json!({"max_members": 2}))
        .await;

    // Five candidates, all firing at the one remaining seat at once.
    let mut tokens = Vec::new();
    for i in 0..5 {
        let (token, _) = ts
            .register(&format!("cand{i}"), &format!("Candidate {i}"))
            .await;
        tokens.push(token);
    }
    let joins = tokens.iter().map(|token| {
        ts.post(
            token,
            "/families/join",
            json!({"invite_code": invite_code.clone()}),
        )
    });
    let results = futures_util::future::join_all(joins).await;

    let statuses: Vec<u16> = results.iter().map(|r| r.status().as_u16()).collect();
    let admitted = statuses.iter().filter(|s| **s == 200).count();
    assert_eq!(
        admitted, 1,
        "exactly one seat, exactly one winner; got {statuses:?}"
    );
    // Naming the CODE matters: grant_membership has two ways to answer
    // 409, and a test that counted statuses alone would pass if four
    // people were refused as "already in a family" instead.
    let mut full = 0;
    for r in results {
        if r.status() == 409 {
            let body: Value = r.json().await.expect("error is JSON");
            assert_eq!(
                body["error"]["code"], "family_full",
                "refused for the wrong reason: {body}"
            );
            full += 1;
        }
    }
    assert_eq!(full, 4, "the other four are told the family is full");

    // And the roster agrees with the answers, which is the assertion that
    // would catch a lock that serialized the responses but not the writes.
    let mine: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(
        mine["members"].as_array().map(Vec::len),
        Some(2),
        "the room must match the cap, not the number of hopefuls: {}",
        mine["members"]
    );
}

/// The order the refusal runs in is load-bearing, not cosmetic: a caller
/// who is already in a family must get the SAME answer a stranger gets, or
/// `already_in_family` for a real code and `invalid_invite_code` for a
/// made-up one tell the two apart — and a family of one's own is one
/// request away (protocol.md, "Families": closed is checked first).
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_closed_family_answers_the_same_to_somebody_already_in_a_family() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (prober, _) = ts.register("stranger", "Stranger").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.patch(&owner, "/families/mine", json!({"join_policy": "closed"}))
        .await;
    // The prober has a family of their own, which is what makes the
    // ordering matter.
    ts.create_family(&prober, "The Joneses").await;

    let real = ts
        .post(
            &prober,
            "/families/join",
            json!({ "invite_code": invite_code }),
        )
        .await;
    assert_error(real, 404, "invalid_invite_code").await;
    let nonsense = ts
        .post(
            &prober,
            "/families/join",
            json!({"invite_code": "ZZZZ9999"}),
        )
        .await;
    assert_error(nonsense, 404, "invalid_invite_code").await;
}

/// The cap binds at the APPROVAL door as well as the open one. A family
/// that is already full refuses the request rather than collecting one
/// that every approval would refuse and leave pending for ever
/// (protocol.md, "Families").
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_full_family_refuses_at_the_approval_door_and_collects_no_request() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (stranger, _) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;

    // Fill it through the open door, then shut it to approval and cap it.
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    ts.patch(
        &owner,
        "/families/mine",
        json!({"join_policy": "approval", "max_members": 2}),
    )
    .await;

    let refused = ts
        .post(
            &stranger,
            "/families/join",
            json!({ "invite_code": invite_code }),
        )
        .await;
    assert_error(refused, 409, "family_full").await;

    // And nothing was queued: the rollback took the INSERT with it.
    let requests: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("requests");
    assert_eq!(
        requests["requests"].as_array().map(Vec::len),
        Some(0),
        "a full family must not collect a request no approval could ever satisfy: {requests}"
    );
    // The applicant is not left showing "waiting" for ever, either.
    let me: Value = ts.get(&stranger, "/me").await.json().await.expect("me");
    assert!(
        me["pending_join_request"].is_null(),
        "nothing is pending: {me}"
    );
}

/// The whole justification for leaving a refused approval PENDING is that
/// the owner can use it later. Proving it survives is not the same as
/// proving it still works — `grant_membership` rejects the applicant's
/// other pending requests, so re-entry has to be checked, not assumed.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_kept_request_can_be_approved_once_a_seat_frees() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (applicant, _) = ts.register("junior", "Junior").await;
    let (filler, _) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;

    ts.join(&applicant, &invite_code, "pending").await;
    ts.set_open_policy(&owner).await;
    ts.join(&filler, &invite_code, "joined").await;
    ts.patch(
        &owner,
        "/families/mine",
        json!({"join_policy": "approval", "max_members": 2}),
    )
    .await;

    let requests: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("requests");
    let request_id = requests["requests"][0]["id"].as_i64().expect("one request");
    let refused = ts
        .post(
            &owner,
            &format!("/families/join-requests/{request_id}/approve"),
            json!({}),
        )
        .await;
    assert_error(refused, 409, "family_full").await;

    // A seat frees.
    ts.post(&filler, "/families/leave", json!({})).await;

    // THE POINT: the same request, approved on the second attempt.
    let approved = ts
        .post(
            &owner,
            &format!("/families/join-requests/{request_id}/approve"),
            json!({}),
        )
        .await;
    assert_eq!(
        approved.status(),
        200,
        "a kept request must still be usable, or keeping it was theatre"
    );
    let me: Value = ts.get(&applicant, "/me").await.json().await.expect("me");
    assert!(!me["family"].is_null(), "the applicant is in: {me}");
}

/// The third arm of `min(family cap, operator ceiling)`: a stored cap ABOVE
/// the ceiling must not let anybody past it. Without the `.min()` a family
/// that set 8 under a generous ceiling keeps admitting eight after the
/// operator drops the ceiling to two — and every other cap test still
/// passes, because they set caps BELOW the ceiling.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_stored_cap_above_the_operators_ceiling_does_not_beat_it() {
    let ts = spawn_server_with_config(|cfg| cfg.limits.max_family_members = 2).await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (second, _) = ts.register("junior", "Junior").await;
    let (third, _) = ts.register("cousin", "Cousin").await;
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&second, &invite_code, "joined").await;

    // Write a cap above the ceiling the only way an operator ever could:
    // it was legal when it was set, and the ceiling came down afterwards.
    // The PATCH validator refuses it now, which is exactly why this goes
    // in behind it.
    sqlx::query("UPDATE families SET max_members = 8 WHERE id = $1")
        .bind(family_id)
        .execute(&ts.state.pool)
        .await
        .expect("store a cap that predates the lowered ceiling");

    let full = ts
        .post(
            &third,
            "/families/join",
            json!({ "invite_code": invite_code }),
        )
        .await;
    assert_error(full, 409, "family_full").await;
}

/// `/me` carries the operator's ceiling so an owner's cap picker can draw
/// its own range instead of discovering `validation` when somebody saves.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn me_always_carries_the_operators_family_ceiling() {
    let ts = spawn_server_with_config(|cfg| cfg.limits.max_family_members = 12).await;
    let (user, _) = ts.register("solo", "Solo").await;

    // Before there is any family at all — it is a server fact, not a
    // family one.
    let me: Value = ts.get(&user, "/me").await.json().await.expect("me");
    assert_eq!(
        me["max_family_members"], 12,
        "always present, like calls_enabled: {me}"
    );

    ts.create_family(&user, "The Smiths").await;
    let me: Value = ts.get(&user, "/me").await.json().await.expect("me");
    assert_eq!(me["max_family_members"], 12);
    // And the door is reported open on a default server — always present,
    // for the same reason (protocol.md, "Starting a family").
    assert_eq!(me["family_registration_enabled"], true, "{me}");
}

/// A server closed to NEW families (`[families] registration = false`,
/// protocol.md "Starting a family"): `POST /families` is refused with its
/// own code before the name is looked at, `/me` says so up front, and an
/// account can still be registered — joining a family that already lives
/// here is what such a server is for.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_server_closed_to_new_families_refuses_creation_and_says_so_on_me() {
    let ts = spawn_server_with_config(|cfg| cfg.families.registration = false).await;
    let (user, _) = ts.register("late", "Late Comer").await;

    let me: Value = ts.get(&user, "/me").await.json().await.expect("me");
    assert_eq!(me["family_registration_enabled"], false, "{me}");

    let refused = ts
        .post(
            &user,
            "/families",
            serde_json::json!({"name": "The Latecomers"}),
        )
        .await;
    assert_error(refused, 403, "family_registration_disabled").await;

    // Refused BEFORE validation: an empty name gets the door, not a lecture
    // about lengths — the door is the fact that matters.
    let refused = ts
        .post(&user, "/families", serde_json::json!({"name": ""}))
        .await;
    assert_error(refused, 403, "family_registration_disabled").await;

    // Still nobody's member, and still able to ask to join one.
    let me: Value = ts.get(&user, "/me").await.json().await.expect("me");
    assert!(me["family"].is_null(), "{me}");
    let join = ts
        .post(
            &user,
            "/families/join",
            serde_json::json!({"invite_code": "NOPE1234"}),
        )
        .await;
    assert_error(join, 404, "invalid_invite_code").await;
}

/// D4. An owner who leaves HANDS THE FAMILY ON rather than being refused.
/// The successor is the longest-standing remaining member — the same rule
/// account deletion uses, reached now from a second door.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_owner_who_leaves_hands_the_family_to_the_longest_standing_member() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (first, first_id) = ts.register("junior", "Junior").await;
    let (second, _) = ts.register("cousin", "Cousin").await;
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&first, &invite_code, "joined").await;
    ts.join(&second, &invite_code, "joined").await;

    // The owner is told who WOULD inherit before they commit to leaving.
    let mine: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(
        mine["next_owner_user_id"], first_id,
        "the leave dialog can name the successor: {mine}"
    );

    let left = ts.post(&owner, "/families/leave", json!({})).await;
    assert_eq!(left.status(), 200, "the owner is never refused");
    let body: Value = left.json().await.expect("leave is JSON");
    assert_eq!(
        body["new_owner_user_id"], first_id,
        "longest-standing wins: {body}"
    );

    // The family survives, with a new owner and without the old one.
    let mine: Value = ts
        .get(&first, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(mine["family"]["id"], family_id);
    let ids: Vec<i64> = mine["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|m| m["id"].as_i64().expect("id"))
        .collect();
    assert!(!ids.contains(&owner_id), "the leaver is gone: {ids:?}");
    let me: Value = ts.get(&first, "/me").await.json().await.expect("me");
    assert_eq!(me["role"], "owner", "the successor holds the owner screens");
    // And the departed owner is family-less, like any other leaver.
    let me: Value = ts.get(&owner, "/me").await.json().await.expect("me");
    assert_eq!(me["family"], Value::Null);
}

/// `next_owner_user_id` is owner-only and absent when there is nobody to
/// inherit — which is a DIFFERENT dialog, because leaving then deletes the
/// family.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_successor_hint_is_owner_only_and_absent_when_there_is_nobody() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;

    // Sole member: nobody would inherit.
    let mine: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert!(
        mine.get("next_owner_user_id").is_none(),
        "absent means leaving DELETES the family: {mine}"
    );

    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    // A member never sees it — it is the owner's own decision aid.
    let mine: Value = ts
        .get(&member, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert!(
        mine.get("next_owner_user_id").is_none(),
        "owner-only: {mine}"
    );
}

/// THE GUARD. `pass_ownership_on` can only see members who hold a
/// `chat_members` row for the family chat. A `users` row that names the
/// family without one is invisible to it while still counting everywhere
/// else — and the naive path would then DELETE A FAMILY WITH A LIVE MEMBER
/// IN IT. The refusal is deliberately ugly, because the state is.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_owner_cannot_delete_a_family_that_still_has_an_unseen_member() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    // Manufacture the divergence: still a member by `users.family_id`,
    // invisible to a successor search that goes through the family chat.
    sqlx::query(
        "DELETE FROM chat_members
          WHERE user_id = $1
            AND chat_id IN (SELECT id FROM chats WHERE family_id = $2 AND kind = 'family')",
    )
    .bind(member_id)
    .bind(family_id)
    .execute(&ts.state.pool)
    .await
    .expect("manufacture the divergence");

    // A 500, not a refusal: the owner MAY leave, the server just cannot
    // work out to whom. `owner_cannot_leave` is retired and protocol.md
    // says no endpoint raises it, so reviving it here would make the doc
    // false.
    let refused = ts.post(&owner, "/families/leave", json!({})).await;
    assert_eq!(refused.status(), 500, "a database state that cannot happen");

    // The family is STILL THERE, which is the whole point.
    let mine: Value = ts
        .get(&member, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(
        mine["family"]["id"], family_id,
        "the family must survive a hand-off that could not find a successor: {mine}"
    );
}

/// D3. A closed family admits nobody, and the refusal is byte-identical to
/// a code that never existed — a shut door must tell a stranger nothing.
///
/// The second half is the one that matters. Before `closed` was handled
/// explicitly, `join_family` branched "open -> grant, ELSE -> create a join
/// request", so a closed family would have read as shut in the app while a
/// queue of people the owner never invited filled up behind it.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_closed_family_refuses_the_code_and_collects_no_join_requests() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (stranger, _) = ts.register("stranger", "Stranger").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;

    let patched = ts
        .patch(&owner, "/families/mine", json!({"join_policy": "closed"}))
        .await;
    assert_eq!(patched.status(), 200);
    let body: Value = patched.json().await.expect("patch is JSON");
    assert_eq!(body["family"]["join_policy"], "closed");

    // A real code, and the same answer a made-up one gets — byte-identical
    // down to the MESSAGE, which `assert_error` does not check. A closed
    // family answering "this family is closed" would pass a status-and-code
    // assertion while leaking exactly what the design forbids.
    let refused = ts
        .post(
            &stranger,
            "/families/join",
            json!({"invite_code": invite_code}),
        )
        .await;
    assert_eq!(refused.status(), 404);
    let real: Value = refused.json().await.expect("error is JSON");
    let nonsense = ts
        .post(
            &stranger,
            "/families/join",
            json!({"invite_code": "ZZZZ9999"}),
        )
        .await;
    assert_eq!(nonsense.status(), 404);
    let nonsense: Value = nonsense.json().await.expect("error is JSON");
    assert_eq!(
        real, nonsense,
        "a shut door must be indistinguishable from a wrong code, message included"
    );
    assert_eq!(real["error"]["code"], "invalid_invite_code");

    // THE BUG THIS TEST EXISTS FOR: nothing was queued behind the shut door.
    let requests: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("requests");
    assert_eq!(
        requests["requests"].as_array().map(Vec::len),
        Some(0),
        "a closed family must not collect join requests: {requests}"
    );
}

/// Closing the door is about the CODE, not about the owner: requests that
/// were already pending survive it, and the owner may still say yes.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn closing_the_family_leaves_pending_requests_approvable() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (applicant, applicant_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;

    // Default policy is approval, so this queues.
    ts.join(&applicant, &invite_code, "pending").await;
    ts.patch(&owner, "/families/mine", json!({"join_policy": "closed"}))
        .await;

    let requests: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("requests");
    let request_id = requests["requests"][0]["id"].as_i64().expect("one request");

    let approved = ts
        .post(
            &owner,
            &format!("/families/join-requests/{request_id}/approve"),
            json!({}),
        )
        .await;
    assert_eq!(
        approved.status(),
        200,
        "closing stops the code, not the owner"
    );
    let me: Value = ts.get(&applicant, "/me").await.json().await.expect("me");
    assert_eq!(me["user"]["id"], applicant_id);
    assert!(!me["family"].is_null(), "the applicant is in: {me}");
}

/// D2. The cap is the family's own, set by the owner, and it binds at the
/// door rather than over the room.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_owner_sets_a_member_cap_and_the_door_enforces_it() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (second, _) = ts.register("junior", "Junior").await;
    let (third, _) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;

    // Two seats: the owner plus one.
    let patched: Value = ts
        .patch(&owner, "/families/mine", json!({"max_members": 2}))
        .await
        .json()
        .await
        .expect("patch");
    assert_eq!(patched["family"]["max_members"], 2);

    ts.join(&second, &invite_code, "joined").await;

    let full = ts
        .post(
            &third,
            "/families/join",
            json!({"invite_code": invite_code}),
        )
        .await;
    assert_error(full, 409, "family_full").await;

    // A cap BELOW the current size is a freeze, not an error: an owner who
    // inherits a large family must still be able to shut the door.
    let frozen = ts
        .patch(&owner, "/families/mine", json!({"max_members": 1}))
        .await;
    assert_eq!(
        frozen.status(),
        200,
        "a cap under the head count is accepted"
    );
    let frozen: Value = frozen.json().await.expect("patch is JSON");
    assert_eq!(
        frozen["family"]["max_members"], 1,
        "and it is actually WRITTEN — a 200 that stored nothing would pass a status-only assertion: {}",
        frozen["family"]
    );
    // Nobody is thrown out: the cap is read at the door, never enforced
    // over the room.
    let mine: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert_eq!(
        mine["members"].as_array().map(Vec::len),
        Some(2),
        "a freeze removes nobody: {}",
        mine["members"]
    );

    // And clearing it with an explicit null lets people in again.
    let cleared: Value = ts
        .patch(&owner, "/families/mine", json!({"max_members": null}))
        .await
        .json()
        .await
        .expect("patch");
    assert!(
        cleared["family"].get("max_members").is_none(),
        "null clears the cap, and cleared is ABSENT: {}",
        cleared["family"]
    );
    ts.join(&third, &invite_code, "joined").await;
}

/// The cap is re-checked when the OWNER approves, because the roster can
/// fill between a request and the decision — and a full family leaves the
/// request PENDING, because "full" stops being true when somebody leaves.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn approving_into_a_full_family_is_refused_and_keeps_the_request() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (applicant, _) = ts.register("junior", "Junior").await;
    let (filler, _) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;

    // Applicant queues under the approval policy.
    ts.join(&applicant, &invite_code, "pending").await;
    // Then the family fills by another door before the owner decides.
    ts.set_open_policy(&owner).await;
    ts.join(&filler, &invite_code, "joined").await;
    ts.patch(&owner, "/families/mine", json!({"max_members": 2}))
        .await;

    let requests: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("requests");
    let request_id = requests["requests"][0]["id"].as_i64().expect("one request");
    let refused = ts
        .post(
            &owner,
            &format!("/families/join-requests/{request_id}/approve"),
            json!({}),
        )
        .await;
    assert_error(refused, 409, "family_full").await;

    // STILL PENDING. The owner said yes; the server did not quietly reject
    // somebody on their behalf over a condition that can lift.
    let after: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("requests");
    assert_eq!(
        after["requests"].as_array().map(Vec::len),
        Some(1),
        "a full family must not consume the request: {after}"
    );
}

/// Validation, and the ceiling the operator owns.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_cap_outside_one_to_the_operators_ceiling_is_refused() {
    let ts = spawn_server_with_config(|cfg| cfg.limits.max_family_members = 10).await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;

    for bad in [0, -1, 11] {
        let refused = ts
            .patch(&owner, "/families/mine", json!({"max_members": bad}))
            .await;
        assert_error(refused, 400, "validation").await;
    }
    let ok = ts
        .patch(&owner, "/families/mine", json!({"max_members": 10}))
        .await;
    assert_eq!(ok.status(), 200, "the ceiling itself is allowed");
}

/// A family that set no cap of its own is still bound by the operator's
/// ceiling at the door — that is what makes it a runaway guard rather than
/// a suggestion.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_operators_ceiling_binds_a_family_that_set_no_cap() {
    let ts = spawn_server_with_config(|cfg| cfg.limits.max_family_members = 1).await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (second, _) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;

    let mine: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("mine");
    assert!(
        mine["family"].get("max_members").is_none(),
        "this family set no cap of its own: {}",
        mine["family"]
    );

    let full = ts
        .post(
            &second,
            "/families/join",
            json!({"invite_code": invite_code}),
        )
        .await;
    assert_error(full, 409, "family_full").await;
}

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
            // Present for the same reason `ai_history` is, and `false` for
            // the reason migration 0032 argues: a photograph is not more of
            // the same thing, so this switch defaults the other way
            // (protocol.md, "Pictures").
            "ai_vision": false,
            // And the third, present always and false by default, for the
            // reason migration 0033 argues: nobody chose those pictures
            // for this question (protocol.md, "Recent photos from the
            // family chat").
            "ai_history_photos": false,
        }),
        "the whole Family object, exactly the shape in protocol.md — no language key, \
         because nobody has chosen one, and ai_history present because it always is"
    );

    // The SAME pin with a cap set. The case above stays green whether or
    // not `max_members` exists, because an unset cap serialises to no key —
    // which is the strongest practical argument for the nullable design and
    // also the reason it cannot, on its own, pin the field's presence.
    let capped = ts
        .patch(&owner, "/families/mine", json!({"max_members": 6}))
        .await;
    assert_eq!(capped.status(), 200);
    let body: Value = capped.json().await.expect("patch is JSON");
    let family = body["family"].clone();
    assert_eq!(
        family,
        json!({
            "id": family_id,
            "name": "The Smiths",
            "join_policy": "open",
            "created_at": family["created_at"],
            "invite_code": invite_code,
            "max_members": 6,
            "ai_history": false,
            "ai_vision": false,
            "ai_history_photos": false,
        }),
        "and the whole object again with a cap — one key more, nothing else moved"
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

/// The operator's ceiling binds at the APPROVAL doors too, for a family
/// that never set a cap of its own. Both existing ceiling tests go through
/// the OPEN door, so passing `i64::MAX` at either approval-path call site
/// leaves the suite green while a ceiling of two admits everybody.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_operators_ceiling_binds_at_the_approval_doors() {
    let ts = spawn_server_with_config(|cfg| cfg.limits.max_family_members = 2).await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (second, _) = ts.register("junior", "Junior").await;
    let (third, _) = ts.register("cousin", "Cousin").await;
    // DEFAULT approval policy, and no max_members set at all.
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;

    // Fill the second seat through the approval door.
    ts.join(&second, &invite_code, "pending").await;
    let requests: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("requests");
    let id = requests["requests"][0]["id"].as_i64().expect("id");
    assert_eq!(
        ts.post(
            &owner,
            &format!("/families/join-requests/{id}/approve"),
            json!({})
        )
        .await
        .status(),
        200
    );

    // Now full at the ceiling: the REQUEST door refuses and queues nothing.
    let refused = ts
        .post(
            &third,
            "/families/join",
            json!({"invite_code": invite_code}),
        )
        .await;
    assert_error(refused, 409, "family_full").await;
    let after: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("requests");
    assert_eq!(
        after["requests"].as_array().map(Vec::len),
        Some(0),
        "the ceiling collects no request either: {after}"
    );
}
