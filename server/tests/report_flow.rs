//! Integration: reporting a member (docs/protocol.md, "Reporting a
//! member"). The owner is the moderator — except when the report is about
//! them.

mod common;

use common::{assert_error, spawn_server};
use serde_json::{Value, json};
use uuid::Uuid;

fn uuid() -> String {
    Uuid::new_v4().to_string()
}

/// A family of three: owner, and two members.
async fn family_of_three(ts: &common::TestServer) -> (String, String, i64, String, i64) {
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (first, first_id) = ts.register("junior", "Junior").await;
    let (second, second_id) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&first, &invite_code, "joined").await;
    ts.join(&second, &invite_code, "joined").await;
    let _ = owner_id;
    (owner, first, first_id, second, second_id)
}

/// The ordinary path: a member reports another, the owner sees it and
/// resolves it once.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_member_reports_another_and_the_owner_resolves_it() {
    let ts = spawn_server().await;
    let (owner, reporter, _reporter_id, _other, other_id) = family_of_three(&ts).await;

    let filed = ts
        .post(
            &reporter,
            "/families/reports",
            json!({"reported_user_id": other_id, "reason": "harassment"}),
        )
        .await;
    assert_eq!(filed.status(), 201);
    let body: Value = filed.json().await.expect("report JSON");
    let report_id = body["report"]["id"].as_i64().expect("id");
    assert_eq!(body["report"]["reason"], "harassment");
    assert_eq!(body["report"]["reported"]["id"], other_id);
    assert!(
        body["report"].get("message_id").is_none()
            && body["report"].get("message_excerpt").is_none(),
        "a person report carries neither message key: {}",
        body["report"]
    );

    let listed: Value = ts
        .get(&owner, "/families/reports")
        .await
        .json()
        .await
        .expect("reports");
    assert_eq!(listed["reports"].as_array().map(Vec::len), Some(1));

    let resolved = ts
        .post(
            &owner,
            &format!("/families/reports/{report_id}/resolve"),
            json!({}),
        )
        .await;
    assert_eq!(resolved.status(), 204);
    let listed: Value = ts
        .get(&owner, "/families/reports")
        .await
        .json()
        .await
        .expect("reports");
    assert_eq!(
        listed["reports"].as_array().map(Vec::len),
        Some(0),
        "resolved reports leave the list: {listed}"
    );

    // Resolving twice is `report_not_pending`, the same answer an unknown
    // id gets.
    let again = ts
        .post(
            &owner,
            &format!("/families/reports/{report_id}/resolve"),
            json!({}),
        )
        .await;
    assert_error(again, 409, "report_not_pending").await;
    let unknown = ts
        .post(&owner, "/families/reports/999999/resolve", json!({}))
        .await;
    assert_error(unknown, 409, "report_not_pending").await;
}

/// The excerpt is FROZEN: the author editing the body away afterwards, and
/// retention deleting the message outright, both otherwise leave the owner
/// an empty screen. This is the one quotation in the protocol that is
/// stored rather than recomputed.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_message_report_freezes_the_body_against_a_later_edit() {
    let ts = spawn_server().await;
    let (owner, reporter, _reporter_id, other, other_id) = family_of_three(&ts).await;
    let chat_id = ts.family_chat_id(&other).await;

    let posted: Value = ts
        .post_message(&other, chat_id, &uuid(), "something regrettable")
        .await
        .json()
        .await
        .expect("message");
    let message_id = posted["message"]["id"].as_i64().expect("id");

    let filed: Value = ts
        .post(
            &reporter,
            "/families/reports",
            json!({
                "reported_user_id": other_id,
                "reason": "inappropriate",
                "message_id": message_id
            }),
        )
        .await
        .json()
        .await
        .expect("report");
    assert_eq!(filed["report"]["message_id"], message_id);
    assert_eq!(filed["report"]["message_excerpt"], "something regrettable");

    // The author edits the evidence away.
    let edited = ts
        .patch(
            &other,
            &format!("/chats/{chat_id}/messages/{message_id}"),
            json!({"body": "nothing to see"}),
        )
        .await;
    assert_eq!(edited.status(), 200);

    let listed: Value = ts
        .get(&owner, "/families/reports")
        .await
        .json()
        .await
        .expect("reports");
    assert_eq!(
        listed["reports"][0]["message_excerpt"], "something regrettable",
        "the excerpt is frozen at the moment the report was raised: {listed}"
    );
}

/// Identity is `(reporter, reported, message_id)`: a double tap is one row,
/// a second MESSAGE of the same member is a second report.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_same_report_twice_is_one_row_but_a_second_message_is_a_second_report() {
    let ts = spawn_server().await;
    let (owner, reporter, _reporter_id, other, other_id) = family_of_three(&ts).await;
    let chat_id = ts.family_chat_id(&other).await;

    let first = ts
        .post(
            &reporter,
            "/families/reports",
            json!({"reported_user_id": other_id, "reason": "spam"}),
        )
        .await;
    assert_eq!(first.status(), 201);
    let first_body: Value = first.json().await.expect("json");
    let first_id = first_body["report"]["id"].as_i64().expect("id");

    // The same person again, with a DIFFERENT reason: the open row comes
    // back, its stored reason unchanged, and nothing is created.
    let again = ts
        .post(
            &reporter,
            "/families/reports",
            json!({"reported_user_id": other_id, "reason": "harassment"}),
        )
        .await;
    assert_eq!(again.status(), 200, "a duplicate returns the open row");
    let again_body: Value = again.json().await.expect("json");
    assert_eq!(again_body["report"]["id"], first_id);
    assert_eq!(
        again_body["report"]["reason"], "spam",
        "the stored reason is not overwritten: {again_body}"
    );

    // Two different messages of theirs are two reports — a moderator needs
    // to see a pattern.
    let m1: Value = ts
        .post_message(&other, chat_id, &uuid(), "one")
        .await
        .json()
        .await
        .expect("m");
    let m2: Value = ts
        .post_message(&other, chat_id, &uuid(), "two")
        .await
        .json()
        .await
        .expect("m");
    for m in [&m1, &m2] {
        let filed = ts
            .post(
                &reporter,
                "/families/reports",
                json!({
                    "reported_user_id": other_id,
                    "reason": "other",
                    "message_id": m["message"]["id"]
                }),
            )
            .await;
        assert_eq!(filed.status(), 201);
    }

    let listed: Value = ts
        .get(&owner, "/families/reports")
        .await
        .json()
        .await
        .expect("reports");
    assert_eq!(
        listed["reports"].as_array().map(Vec::len),
        Some(3),
        "one person report plus two message reports: {listed}"
    );
}

/// **THE OWNER SHIELD.** A report naming the owner is stored, never listed
/// to them, and never resolvable by them — and the reporter's `201` is
/// identical either way, so the response never says whether the owner was
/// shielded.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_report_about_the_owner_never_reaches_the_owner() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (other, other_id) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    ts.join(&other, &invite_code, "joined").await;

    // An ordinary report, so the list is not trivially empty.
    ts.post(
        &member,
        "/families/reports",
        json!({"reported_user_id": other_id, "reason": "spam"}),
    )
    .await;

    let about_owner = ts
        .post(
            &member,
            "/families/reports",
            json!({"reported_user_id": owner_id, "reason": "harassment"}),
        )
        .await;
    assert_eq!(
        about_owner.status(),
        201,
        "byte-identical to any other report — the response never says the owner was shielded"
    );
    let shielded: Value = about_owner.json().await.expect("json");
    let shielded_id = shielded["report"]["id"].as_i64().expect("id");

    let listed: Value = ts
        .get(&owner, "/families/reports")
        .await
        .json()
        .await
        .expect("reports");
    let ids: Vec<i64> = listed["reports"]
        .as_array()
        .expect("array")
        .iter()
        .map(|r| r["id"].as_i64().expect("id"))
        .collect();
    assert!(
        !ids.contains(&shielded_id),
        "the owner must never read who complained about them: {listed}"
    );
    assert_eq!(ids.len(), 1, "the other report is still listed: {listed}");

    // Nor can they resolve it away by guessing the id.
    let resolve = ts
        .post(
            &owner,
            &format!("/families/reports/{shielded_id}/resolve"),
            json!({}),
        )
        .await;
    assert_error(resolve, 409, "report_not_pending").await;
}

/// The refusals.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn reporting_yourself_a_stranger_a_bad_reason_or_an_unseen_message_is_refused() {
    let ts = spawn_server().await;
    let (owner, reporter, reporter_id, other, other_id) = family_of_three(&ts).await;
    let (outsider, outsider_id) = ts.register("stranger", "Stranger").await;

    let myself = ts
        .post(
            &reporter,
            "/families/reports",
            json!({"reported_user_id": reporter_id, "reason": "spam"}),
        )
        .await;
    assert_error(myself, 400, "cannot_report_self").await;

    let stranger = ts
        .post(
            &reporter,
            "/families/reports",
            json!({"reported_user_id": outsider_id, "reason": "spam"}),
        )
        .await;
    assert_error(stranger, 403, "not_same_family").await;

    let bad_reason = ts
        .post(
            &reporter,
            "/families/reports",
            json!({"reported_user_id": other_id, "reason": "rude"}),
        )
        .await;
    assert_error(bad_reason, 400, "validation").await;

    // A message the reported member did NOT write.
    let chat_id = ts.family_chat_id(&reporter).await;
    let mine: Value = ts
        .post_message(&reporter, chat_id, &uuid(), "my own words")
        .await
        .json()
        .await
        .expect("m");
    let not_theirs = ts
        .post(
            &reporter,
            "/families/reports",
            json!({
                "reported_user_id": other_id,
                "reason": "spam",
                "message_id": mine["message"]["id"]
            }),
        )
        .await;
    assert_error(not_theirs, 404, "message_not_found").await;

    // And a message id that does not exist gets the same answer, so the
    // endpoint never confirms one exists elsewhere.
    let nowhere = ts
        .post(
            &reporter,
            "/families/reports",
            json!({"reported_user_id": other_id, "reason": "spam", "message_id": 999999}),
        )
        .await;
    assert_error(nowhere, 404, "message_not_found").await;

    // Only the owner may read or resolve.
    let forbidden = ts.get(&other, "/families/reports").await;
    assert_error(forbidden, 403, "not_family_owner").await;
    let _ = (owner, outsider);
}

/// An owner reads THEIR family's reports and nobody else's. Deleting the
/// family predicate from the list and the resolve compiles, keeps the bind,
/// and passes every single-family test in this file — while every owner on
/// the box reads every other family's reports and their frozen quotations.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_owner_never_sees_another_familys_reports() {
    let ts = spawn_server().await;
    // Family A.
    let (owner_a, _) = ts.register("olive", "Olive").await;
    let (member_a, _) = ts.register("junior", "Junior").await;
    let (target_a, target_a_id) = ts.register("cousin", "Cousin").await;
    let (_, code_a) = ts.create_family(&owner_a, "The Smiths").await;
    ts.set_open_policy(&owner_a).await;
    ts.join(&member_a, &code_a, "joined").await;
    ts.join(&target_a, &code_a, "joined").await;

    // Family B, with a MESSAGE report so its excerpt is distinctive.
    let (owner_b, _) = ts.register("bianca", "Bianca").await;
    let (member_b, _) = ts.register("bruno", "Bruno").await;
    let (target_b, target_b_id) = ts.register("bella", "Bella").await;
    let (_, code_b) = ts.create_family(&owner_b, "The Joneses").await;
    ts.set_open_policy(&owner_b).await;
    ts.join(&member_b, &code_b, "joined").await;
    ts.join(&target_b, &code_b, "joined").await;
    let chat_b = ts.family_chat_id(&target_b).await;
    let posted: Value = ts
        .post_message(&target_b, chat_b, &uuid(), "a very distinctive utterance")
        .await
        .json()
        .await
        .expect("message");

    let a_report: Value = ts
        .post(
            &member_a,
            "/families/reports",
            json!({"reported_user_id": target_a_id, "reason": "spam"}),
        )
        .await
        .json()
        .await
        .expect("json");
    let a_id = a_report["report"]["id"].as_i64().expect("id");
    let b_report: Value = ts
        .post(
            &member_b,
            "/families/reports",
            json!({
                "reported_user_id": target_b_id,
                "reason": "harassment",
                "message_id": posted["message"]["id"]
            }),
        )
        .await
        .json()
        .await
        .expect("json");
    let b_id = b_report["report"]["id"].as_i64().expect("id");

    // Owner A sees exactly one row, and B's excerpt appears nowhere in the
    // response text at all.
    let listed = ts.get(&owner_a, "/families/reports").await;
    let raw = listed.text().await.expect("body");
    let parsed: Value = serde_json::from_str(&raw).expect("json");
    assert_eq!(parsed["reports"].as_array().map(Vec::len), Some(1));
    assert_eq!(parsed["reports"][0]["id"], a_id);
    assert!(
        !raw.contains("a very distinctive utterance"),
        "another family's frozen quotation must not appear: {raw}"
    );

    // Nor may owner A resolve B's report by guessing its id.
    let poached = ts
        .post(
            &owner_a,
            &format!("/families/reports/{b_id}/resolve"),
            json!({}),
        )
        .await;
    assert_error(poached, 409, "report_not_pending").await;
    let b_list: Value = ts
        .get(&owner_b, "/families/reports")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(
        b_list["reports"].as_array().map(Vec::len),
        Some(1),
        "B's report is untouched: {b_list}"
    );
}

/// Only the OWNER may resolve. The existing suite asserts only the read
/// half of "read or resolve", so narrowing `require_owner_family` to a bare
/// membership check passes every report test.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn only_the_owner_may_resolve_a_report() {
    let ts = spawn_server().await;
    let (owner, reporter, _reporter_id, _other, other_id) = family_of_three(&ts).await;
    let filed: Value = ts
        .post(
            &reporter,
            "/families/reports",
            json!({"reported_user_id": other_id, "reason": "spam"}),
        )
        .await
        .json()
        .await
        .expect("json");
    let report_id = filed["report"]["id"].as_i64().expect("id");

    let by_member = ts
        .post(
            &reporter,
            &format!("/families/reports/{report_id}/resolve"),
            json!({}),
        )
        .await;
    assert_error(by_member, 403, "not_family_owner").await;

    // Still open for the owner, who may.
    let listed: Value = ts
        .get(&owner, "/families/reports")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(listed["reports"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        ts.post(
            &owner,
            &format!("/families/reports/{report_id}/resolve"),
            json!({})
        )
        .await
        .status(),
        204
    );
}
