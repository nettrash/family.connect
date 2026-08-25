//! Integration: birthdays — a member setting their own, and the owner
//! setting one for a member of their family (protocol.md, "Birthdays").
//!
//! The interesting half is not the two numbers. It is that a birthday is a
//! day and a month with NO YEAR, which makes 29 February valid and 31 April
//! not; and that the owner's endpoint, unlike the password reset, is happy
//! to be pointed at the owner's own row.

mod common;

use common::{TestServer, assert_error, spawn_server};
use serde_json::{Value, json};

/// The birthday a roster shows for one member, or `Value::Null` when there
/// is no `birthday` key at all — which is the state this feature spends
/// most of its life in.
async fn roster_birthday(ts: &TestServer, token: &str, user_id: i64) -> Value {
    let mine: Value = ts
        .get(token, "/families/mine")
        .await
        .json()
        .await
        .expect("mine is JSON");
    mine["members"]
        .as_array()
        .expect("members array")
        .iter()
        .find(|member| member["id"] == user_id)
        .expect("the member is in the roster")
        .get("birthday")
        .cloned()
        .unwrap_or(Value::Null)
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_member_sets_their_own_birthday_and_the_family_sees_it() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    // Absent until set — the key is not there, rather than a null.
    let me: Value = ts.get(&member, "/me").await.json().await.expect("me");
    assert!(
        me["user"].get("birthday").is_none(),
        "no birthday key until one is set: {}",
        me["user"]
    );
    assert_eq!(roster_birthday(&ts, &owner, member_id).await, Value::Null);

    let response = ts
        .put(&member, "/me/birthday", json!({"month": 3, "day": 14}))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("put is JSON");
    assert_eq!(body["user"]["birthday"], json!({"month": 3, "day": 14}));

    // It reaches everyone the roster reaches, not only the device that set
    // it: this is a family calendar, not a private note.
    let me: Value = ts.get(&member, "/me").await.json().await.expect("me");
    assert_eq!(me["user"]["birthday"], json!({"month": 3, "day": 14}));
    assert_eq!(
        roster_birthday(&ts, &owner, member_id).await,
        json!({"month": 3, "day": 14})
    );

    // A PUT replaces rather than merges — there is no half of a birthday.
    let body: Value = ts
        .put(&member, "/me/birthday", json!({"month": 12, "day": 1}))
        .await
        .json()
        .await
        .expect("put is JSON");
    assert_eq!(body["user"]["birthday"], json!({"month": 12, "day": 1}));

    // Clearing is a DELETE, and it is idempotent like the avatar's.
    assert_eq!(ts.delete(&member, "/me/birthday").await.status(), 204);
    assert_eq!(ts.delete(&member, "/me/birthday").await.status(), 204);
    assert_eq!(roster_birthday(&ts, &owner, member_id).await, Value::Null);
}

/// 31 April is not a birthday; 29 February is. With no year stored there is
/// no year for the twenty-ninth to fail to exist in.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_day_is_checked_against_its_own_month() {
    let ts = spawn_server().await;
    let (member, _) = ts.register("junior", "Junior").await;

    for (month, day) in [(4, 31), (2, 30), (6, 31), (0, 14), (13, 1), (3, 0), (1, 32)] {
        assert_error(
            ts.put(&member, "/me/birthday", json!({"month": month, "day": day}))
                .await,
            400,
            "validation",
        )
        .await;
    }

    let leap = ts
        .put(&member, "/me/birthday", json!({"month": 2, "day": 29}))
        .await;
    assert_eq!(leap.status(), 200, "29 February is a birthday here");

    // A number no smallint can hold is refused by the body extractor, in
    // the same error shape rather than a bare 400.
    assert_error(
        ts.put(&member, "/me/birthday", json!({"month": 70000, "day": 1}))
            .await,
        400,
        "validation",
    )
    .await;
    // So is a missing half: clearing is DELETE, not a partial PUT.
    assert_error(
        ts.put(&member, "/me/birthday", json!({"month": 3})).await,
        400,
        "validation",
    )
    .await;
}

/// The case this exists for: a parent filling in a child's birthday,
/// because the child is never going to open a settings screen to type it.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_owner_sets_a_birthday_for_a_member() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    let response = ts
        .put(
            &owner,
            &format!("/families/members/{member_id}/birthday"),
            json!({"month": 7, "day": 4}),
        )
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("put is JSON");
    assert_eq!(body["member"]["id"], member_id);
    assert_eq!(body["member"]["role"], "member");
    assert_eq!(body["member"]["birthday"], json!({"month": 7, "day": 4}));

    // The member reads it back as their own.
    let me: Value = ts.get(&member, "/me").await.json().await.expect("me");
    assert_eq!(me["user"]["birthday"], json!({"month": 7, "day": 4}));

    // The owner MAY point it at themselves — deliberately unlike the
    // password reset, which refuses because it would skip a proof. There is
    // no proof to skip here, and refusing would make every roster screen
    // special-case exactly one row.
    let mine = ts
        .put(
            &owner,
            &format!("/families/members/{owner_id}/birthday"),
            json!({"month": 2, "day": 29}),
        )
        .await;
    assert_eq!(mine.status(), 200);
    let body: Value = mine.json().await.expect("put is JSON");
    assert_eq!(body["member"]["role"], "owner");
    assert_eq!(body["member"]["birthday"], json!({"month": 2, "day": 29}));

    // And clears them again, idempotently.
    assert_eq!(
        ts.delete(&owner, &format!("/families/members/{member_id}/birthday"))
            .await
            .status(),
        204
    );
    assert_eq!(
        ts.delete(&owner, &format!("/families/members/{member_id}/birthday"))
            .await
            .status(),
        204
    );
    assert_eq!(roster_birthday(&ts, &owner, member_id).await, Value::Null);
}

/// The owner-acts-on-member matrix: a member cannot, a stranger's id is
/// refused, and an id nobody holds is refused the SAME way — a birthday is
/// personal information, so the endpoint never confirms who exists.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn only_the_owner_writes_someone_elses_and_only_inside_the_family() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (stranger, stranger_id) = ts.register("stranger", "Sam").await;
    let (_family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let (_other_family, _other_code) = ts.create_family(&stranger, "The Joneses").await;

    // A member cannot write anyone's but their own, the owner's included.
    for target in [owner_id, member_id] {
        assert_error(
            ts.put(
                &member,
                &format!("/families/members/{target}/birthday"),
                json!({"month": 1, "day": 1}),
            )
            .await,
            403,
            "not_family_owner",
        )
        .await;
    }
    assert_error(
        ts.delete(&member, &format!("/families/members/{owner_id}/birthday"))
            .await,
        403,
        "not_family_owner",
    )
    .await;

    // Somebody in another family, and an id nobody has, answer alike.
    for target in [stranger_id, 999_999] {
        assert_error(
            ts.put(
                &owner,
                &format!("/families/members/{target}/birthday"),
                json!({"month": 1, "day": 1}),
            )
            .await,
            403,
            "not_same_family",
        )
        .await;
        assert_error(
            ts.delete(&owner, &format!("/families/members/{target}/birthday"))
                .await,
            403,
            "not_same_family",
        )
        .await;
    }

    // The stranger's own birthday is untouched by any of that, and they set
    // it themselves through the endpoint everybody has.
    assert_eq!(
        ts.put(&stranger, "/me/birthday", json!({"month": 5, "day": 5}))
            .await
            .status(),
        200
    );
    assert_eq!(roster_birthday(&ts, &owner, member_id).await, Value::Null);
}
