//! Paid families end to end (docs/protocol.md, "Paid families"; issue #89).
//!
//! Every signed value here is signed by a chain shaped like Apple's — a
//! marked intermediate, a marked leaf — under a root the server is told to
//! trust for the test (`trust_root_for_tests`), so the server runs its real
//! verifier, not a stub. The deadline is driven by calling the sweep the
//! hourly loop calls, with the clock moved in the rows rather than in time.

mod common;

use std::time::Duration;

use common::appstore::{Chain, ms, transaction_payload};
use common::{TestServer, assert_error, spawn_server, spawn_server_with_config};
use family_connect::push_payload::{PlanStage, PushEvent};
use family_connect::subscriptions::{self, SweepReport};
use family_connect::vault::Lookup;
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

const MONTHLY: &str = "me.nettrash.FamilyConnect.family.monthly";
const YEARLY: &str = "me.nettrash.FamilyConnect.family.yearly";

fn days(n: i64) -> time::Duration {
    time::Duration::days(n)
}

/// A server that charges, trusting `chain`.
async fn paid_server(chain: &Chain) -> TestServer {
    paid_server_with(chain, |_| {}).await
}

async fn paid_server_with(
    chain: &Chain,
    tweak: impl FnOnce(&mut family_connect::config::Config),
) -> TestServer {
    let root = chain.root.clone();
    spawn_server_with_config(move |cfg| {
        cfg.subscriptions.enabled = true;
        cfg.subscriptions.trust_root_for_tests = Some(root);
        tweak(cfg);
    })
    .await
}

async fn me(ts: &TestServer, token: &str) -> Value {
    let response = ts.get(token, "/me").await;
    assert_eq!(response.status(), 200);
    response.json().await.expect("me is JSON")
}

async fn token_of(ts: &TestServer, token: &str) -> Uuid {
    let me = me(ts, token).await;
    Uuid::parse_str(me["app_account_token"].as_str().expect("a token")).expect("a UUID")
}

/// A transaction for `original`, expiring at `expires`, bought for `token`.
fn txn(chain: &Chain, original: &str, expires: OffsetDateTime, token: Option<Uuid>) -> String {
    chain.sign(&transaction_payload(original, MONTHLY, expires, token))
}

async fn present(ts: &TestServer, token: &str, transaction: &str) -> reqwest::Response {
    ts.put(
        token,
        "/me/subscription",
        json!({"transaction": transaction}),
    )
    .await
}

/// Register, subscribe for a month and start a family: the owner's token,
/// id, the family and its code.
async fn subscribed_owner(
    ts: &TestServer,
    chain: &Chain,
    name: &str,
) -> (String, i64, i64, String) {
    let (token, user_id) = ts.register(name, name).await;
    let account = token_of(ts, &token).await;
    let response = present(
        ts,
        &token,
        &txn(
            chain,
            &format!("orig-{name}"),
            OffsetDateTime::now_utc() + days(30),
            Some(account),
        ),
    )
    .await;
    assert_eq!(response.status(), 200, "the subscription is stored");
    let (family_id, code) = ts.create_family(&token, &format!("{name}'s family")).await;
    (token, user_id, family_id, code)
}

async fn family_json(ts: &TestServer, token: &str, family_id: i64) -> Value {
    let response = ts.get(token, &format!("/families/{family_id}")).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("family is JSON");
    body["family"].clone()
}

async fn sweep(ts: &TestServer) -> SweepReport {
    subscriptions::sweep(&ts.state)
        .await
        .expect("the sweep runs")
}

/// Every deadline push recorded so far, as (family, stage, English body).
fn plan_pushes(ts: &TestServer) -> Vec<(i64, PlanStage, String)> {
    ts.push
        .calls()
        .into_iter()
        .filter_map(|call| match call.note.event {
            PushEvent::FamilyPlan { family_id, stage } => {
                Some((family_id, stage, call.note.body.render(None)))
            }
            _ => None,
        })
        .collect()
}

async fn wait_for_plan_pushes(ts: &TestServer, count: usize) -> Vec<(i64, PlanStage, String)> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let pushes = plan_pushes(ts);
        if pushes.len() >= count {
            return pushes;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "waited for {count} deadline pushes, saw {pushes:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn register_device(ts: &TestServer, token: &str, push_token: &str) {
    let response = ts
        .post(
            token,
            "/devices",
            json!({"platform": "ios", "push_token": push_token}),
        )
        .await;
    assert_eq!(response.status(), 201);
}

/// Move a family's deadline clock back by `back`.
async fn age_deadline(ts: &TestServer, family_id: i64, back: time::Duration) {
    sqlx::query("UPDATE families SET unpaid_since = now() - $2 WHERE id = $1")
        .bind(family_id)
        .bind(back)
        .execute(&ts.state.pool)
        .await
        .expect("ageing the deadline");
}

/// End every subscription an account holds, as an expiry Apple announced.
async fn expire_subscriptions(ts: &TestServer, user_id: i64) {
    sqlx::query(
        "UPDATE subscriptions SET expires_at = now() - interval '1 minute', grace_until = NULL
          WHERE user_id = $1",
    )
    .bind(user_id)
    .execute(&ts.state.pool)
    .await
    .expect("expiring");
}

#[tokio::test]
async fn a_server_that_does_not_charge_changes_nothing() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("anna", "Anna").await;
    let me = me(&ts, &owner).await;
    assert_eq!(me["subscriptions_enabled"], false);
    for absent in [
        "subscription_products",
        "app_account_token",
        "subscription",
        "paid_family_limits",
        "unpaid_days",
    ] {
        assert!(me.get(absent).is_none(), "{absent} must be absent: {me}");
    }
    let (family_id, _) = ts.create_family(&owner, "Ivanovs").await;
    let family = family_json(&ts, &owner, family_id).await;
    assert_eq!(family["member_ceiling"], 50);
    assert!(family.get("plan").is_none());
    assert!(family.get("delete_at").is_none());
    let plan: String = sqlx::query_scalar("SELECT plan FROM families WHERE id = $1")
        .bind(family_id)
        .fetch_one(&ts.state.pool)
        .await
        .unwrap();
    assert_eq!(
        plan, "free",
        "a family made while the server is free is free"
    );

    let chain = Chain::new();
    let response = present(
        &ts,
        &owner,
        &txn(&chain, "1", OffsetDateTime::now_utc() + days(30), None),
    )
    .await;
    assert_error(response, 403, "subscriptions_disabled").await;
    // Apple is answered, so it stops sending.
    let response = ts
        .post_unauth("/appstore/notifications", json!({"signedPayload": "junk"}))
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(sweep(&ts).await, SweepReport::default());
}

#[tokio::test]
async fn a_new_family_needs_an_active_subscription_and_is_paid() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (owner, _) = ts.register("anna", "Anna").await;

    let me1 = me(&ts, &owner).await;
    assert_eq!(me1["subscriptions_enabled"], true);
    assert_eq!(me1["subscription_products"], json!([MONTHLY, YEARLY]));
    assert_eq!(me1["unpaid_days"], 14);
    assert_eq!(
        me1["paid_family_limits"],
        json!({"members": 25, "storage_bytes": 10_737_418_240_i64, "ai_tokens": 2_000_000,
               "images": 30, "transcript_ms": 7_200_000})
    );
    assert_eq!(me1["subscription"], Value::Null);
    // The token is made once and kept.
    let token = token_of(&ts, &owner).await;
    assert_eq!(token, token_of(&ts, &owner).await);
    let sealed: Vec<u8> =
        sqlx::query_scalar("SELECT app_account_token FROM users WHERE username_key = $1")
            .bind(ts.lookup(Lookup::Username, "anna"))
            .fetch_one(&ts.state.pool)
            .await
            .unwrap();
    assert_eq!(sealed[0], 0x01, "the token is sealed at rest");

    let response = ts
        .post(&owner, "/families", json!({"name": "Ivanovs"}))
        .await;
    assert_error(response, 403, "subscription_required").await;

    let expires = OffsetDateTime::now_utc() + days(30);
    let response = present(&ts, &owner, &txn(&chain, "2000001", expires, Some(token))).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["subscription"]["product_id"], MONTHLY);
    assert_eq!(body["subscription"]["environment"], "Production");
    assert_eq!(body["subscription"]["active"], true);
    assert!(
        body["subscription"].get("renews").is_none(),
        "unknown until renewal info says"
    );

    let (family_id, _) = ts.create_family(&owner, "Ivanovs").await;
    let family = family_json(&ts, &owner, family_id).await;
    assert_eq!(family["plan"], "paid");
    assert_eq!(family["member_ceiling"], 25);
    assert!(family.get("delete_at").is_none());
    assert_eq!(
        me(&ts, &owner).await["families"][0]["plan"],
        "paid",
        "/me agrees"
    );

    // The id is stored only as its lookup and its sealed copy.
    let row =
        sqlx::query("SELECT original_transaction_key, original_transaction_id FROM subscriptions")
            .fetch_one(&ts.state.pool)
            .await
            .unwrap();
    use sqlx::Row;
    assert_eq!(
        row.get::<Vec<u8>, _>("original_transaction_key"),
        ts.lookup(Lookup::AppStoreTransaction, "2000001")
    );
    assert_eq!(
        ts.open_text(&row.get::<Vec<u8>, _>("original_transaction_id")),
        "2000001"
    );
}

#[tokio::test]
async fn joining_stays_free_and_the_paid_ceiling_binds() {
    let chain = Chain::new();
    let ts = paid_server_with(&chain, |cfg| cfg.subscriptions.max_members = 2).await;
    let (owner, _, family_id, code) = subscribed_owner(&ts, &chain, "anna").await;
    ts.set_open_policy(&owner).await;
    let (second, _) = ts.register("boris", "Boris").await;
    ts.join(&second, &code, "joined").await;
    let (third, _) = ts.register("clara", "Clara").await;
    let response = ts
        .post(&third, "/families/join", json!({"invite_code": code}))
        .await;
    assert_error(response, 409, "family_full").await;

    assert_eq!(
        family_json(&ts, &owner, family_id).await["member_ceiling"],
        2
    );
    let response = ts
        .patch(&owner, "/families/mine", json!({"max_members": 3}))
        .await;
    assert_error(response, 400, "validation").await;
    let response = ts
        .patch(&owner, "/families/mine", json!({"max_members": 2}))
        .await;
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn a_purchase_is_believed_only_for_its_own_account_and_chain() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (anna, _) = ts.register("anna", "Anna").await;
    let (boris, _) = ts.register("boris", "Boris").await;
    let anna_token = token_of(&ts, &anna).await;
    let later = OffsetDateTime::now_utc() + days(30);

    // Bought for Anna's account, presented by Boris.
    let response = present(&ts, &boris, &txn(&chain, "7", later, Some(anna_token))).await;
    assert_error(response, 400, "subscription_invalid").await;
    // Signed by a chain the server does not trust.
    let forger = Chain::new();
    let response = present(&ts, &anna, &txn(&forger, "7", later, Some(anna_token))).await;
    assert_error(response, 400, "subscription_invalid").await;
    // Another product.
    let other = chain.sign(&transaction_payload(
        "7",
        "me.nettrash.FamilyConnect.stickers",
        later,
        None,
    ));
    assert_error(
        present(&ts, &anna, &other).await,
        400,
        "subscription_invalid",
    )
    .await;
    // No transaction at all.
    let response = ts.put(&anna, "/me/subscription", json!({})).await;
    assert_error(response, 400, "validation").await;

    // Anna's, presented by Anna, twice: idempotent.
    let jws = txn(&chain, "7", later, Some(anna_token));
    assert_eq!(present(&ts, &anna, &jws).await.status(), 200);
    assert_eq!(present(&ts, &anna, &jws).await.status(), 200);
    // A tokenless purchase (an offer code) belongs to whoever presented it
    // first — and to nobody else after.
    let tokenless = txn(&chain, "8", later, None);
    assert_eq!(present(&ts, &boris, &tokenless).await.status(), 200);
    assert_error(
        present(&ts, &anna, &tokenless).await,
        409,
        "subscription_taken",
    )
    .await;

    // A renewal info for another subscription is refused with its transaction.
    let renewal = chain.sign(&json!({
        "originalTransactionId": "999", "autoRenewStatus": 1,
        "signedDate": ms(OffsetDateTime::now_utc()), "environment": "Production",
    }));
    let response = ts
        .put(
            &anna,
            "/me/subscription",
            json!({"transaction": jws, "renewal": renewal}),
        )
        .await;
    assert_error(response, 400, "subscription_invalid").await;
}

#[tokio::test]
async fn renewal_info_says_whether_it_renews_and_holds_the_grace_period() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (anna, anna_id) = ts.register("anna", "Anna").await;
    let token = token_of(&ts, &anna).await;
    // Expired an hour ago, but Apple is retrying the payment for 16 days.
    let expired = OffsetDateTime::now_utc() - time::Duration::hours(1);
    let renewal = chain.sign(&json!({
        "originalTransactionId": "5", "autoRenewStatus": 1, "isInBillingRetryPeriod": true,
        "gracePeriodExpiresDate": ms(OffsetDateTime::now_utc() + days(16)),
        "signedDate": ms(OffsetDateTime::now_utc()), "environment": "Production",
    }));
    let response = ts
        .put(
            &anna,
            "/me/subscription",
            json!({"transaction": txn(&chain, "5", expired, Some(token)), "renewal": renewal}),
        )
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["subscription"]["active"], true,
        "the grace period covers"
    );
    assert_eq!(body["subscription"]["renews"], true);
    assert!(
        subscriptions::is_active(&ts.state.pool, &ts.state.cfg, anna_id)
            .await
            .unwrap()
    );
    // And it lets a family be started.
    ts.create_family(&anna, "Ivanovs").await;
}

#[tokio::test]
async fn an_unpaid_family_is_warned_and_then_deleted() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (owner, owner_id, family_id, code) = subscribed_owner(&ts, &chain, "anna").await;
    ts.set_open_policy(&owner).await;
    let (member, member_id) = ts.register("boris", "Boris").await;
    ts.join(&member, &code, "joined").await;
    register_device(&ts, &owner, "owner-phone").await;
    register_device(&ts, &member, "member-phone").await;

    // Covered: the sweep does nothing.
    assert_eq!(sweep(&ts).await, SweepReport::default());

    expire_subscriptions(&ts, owner_id).await;
    assert_eq!(sweep(&ts).await.unpaid, 1);
    let family = family_json(&ts, &member, family_id).await;
    let delete_at = OffsetDateTime::parse(
        family["delete_at"].as_str().expect("a deadline"),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let expected = OffsetDateTime::now_utc() + days(14);
    assert!(
        (delete_at - expected).abs() < time::Duration::minutes(1),
        "{delete_at}"
    );
    let pushes = wait_for_plan_pushes(&ts, 2).await;
    assert!(pushes.contains(&(
        family_id,
        PlanStage::Unpaid,
        "Your subscription has ended. Renew it, or this family will be deleted.".to_string()
    )));
    assert!(
        pushes.contains(&(
            family_id,
            PlanStage::Unpaid,
            "This family's subscription has ended. It will be deleted unless it is renewed."
                .to_string()
        ))
    );
    // Marked once, not every hour.
    assert_eq!(sweep(&ts).await, SweepReport::default());

    // The day before.
    age_deadline(&ts, family_id, days(13) + time::Duration::minutes(1)).await;
    assert_eq!(sweep(&ts).await.warned, 1);
    let pushes = wait_for_plan_pushes(&ts, 4).await;
    assert_eq!(
        pushes
            .iter()
            .filter(|(_, stage, _)| *stage == PlanStage::Tomorrow)
            .count(),
        2
    );
    assert_eq!(sweep(&ts).await.warned, 0, "warned once");

    // Everything still works until the day.
    let chat_id = ts.family_chat_id(&member).await;
    let sent = ts
        .post_message(&member, chat_id, &Uuid::new_v4().to_string(), "still here")
        .await;
    assert_eq!(
        sent.status(),
        201,
        "a member still sends during the 14 days"
    );

    // The day.
    age_deadline(&ts, family_id, days(14) + time::Duration::minutes(1)).await;
    assert_eq!(sweep(&ts).await.deleted, 1);
    let gone: Option<i64> = sqlx::query_scalar("SELECT id FROM families WHERE id = $1")
        .bind(family_id)
        .fetch_optional(&ts.state.pool)
        .await
        .unwrap();
    assert_eq!(gone, None);
    for (token, user_id) in [(&owner, owner_id), (&member, member_id)] {
        let me = me(&ts, token).await;
        assert_eq!(me["families"], json!([]), "nobody is in it any more");
        let familyless: Option<OffsetDateTime> =
            sqlx::query_scalar("SELECT familyless_since FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_one(&ts.state.pool)
                .await
                .unwrap();
        assert!(familyless.is_some(), "the familyless clock starts");
    }
    let pushes = wait_for_plan_pushes(&ts, 6).await;
    assert_eq!(
        pushes
            .iter()
            .filter(|(id, stage, body)| *id == family_id
                && *stage == PlanStage::Deleted
                && body == "This family was deleted because its subscription ended.")
            .count(),
        2
    );
}

#[tokio::test]
async fn a_renewal_clears_the_deadline_at_once_and_says_so() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (owner, owner_id, family_id, _) = subscribed_owner(&ts, &chain, "anna").await;
    expire_subscriptions(&ts, owner_id).await;
    sweep(&ts).await;
    assert!(
        family_json(&ts, &owner, family_id)
            .await
            .get("delete_at")
            .is_some()
    );

    let mut ws = ws::connect(&ts, &owner).await;
    let token = token_of(&ts, &owner).await;
    let renewed = txn(
        &chain,
        "orig-anna",
        OffsetDateTime::now_utc() + days(60),
        Some(token),
    );
    assert_eq!(present(&ts, &owner, &renewed).await.status(), 200);
    let frame = ws::next_frame_of_type(&mut ws, "family_plan").await;
    assert_eq!(
        frame,
        json!({"type": "family_plan", "family_id": family_id, "plan": "paid"})
    );
    assert!(
        family_json(&ts, &owner, family_id)
            .await
            .get("delete_at")
            .is_none()
    );
}

#[tokio::test]
async fn the_operator_decides_plans_and_free_families_are_never_swept() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (owner, owner_id, family_id, _) = subscribed_owner(&ts, &chain, "anna").await;
    family_connect::admin::set_plan(&ts.state.pool, family_id, "free")
        .await
        .expect("made free");
    expire_subscriptions(&ts, owner_id).await;
    assert_eq!(
        sweep(&ts).await,
        SweepReport::default(),
        "a free family has no deadline"
    );
    let family = family_json(&ts, &owner, family_id).await;
    assert_eq!(family["plan"], "free");
    assert_eq!(family["member_ceiling"], 50);

    // Made paid again, uncovered: the next sweep starts its days...
    family_connect::admin::set_plan(&ts.state.pool, family_id, "paid")
        .await
        .unwrap();
    assert_eq!(sweep(&ts).await.unpaid, 1);
    // ...and made free, the next sweep drops them.
    family_connect::admin::set_plan(&ts.state.pool, family_id, "free")
        .await
        .unwrap();
    assert_eq!(sweep(&ts).await.covered, 1);
    assert!(
        family_json(&ts, &owner, family_id)
            .await
            .get("delete_at")
            .is_none()
    );

    assert!(
        family_connect::admin::set_plan(&ts.state.pool, family_id, "gold")
            .await
            .is_err()
    );
    assert!(
        family_connect::admin::set_plan(&ts.state.pool, 999_999, "free")
            .await
            .is_err()
    );
    let listed = family_connect::admin::families(&ts.state.pool, &ts.state.keys, &ts.state.cfg)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "anna's family");
    assert_eq!(listed[0].members, 1);
}

#[tokio::test]
async fn a_hand_off_moves_the_duty_with_the_family() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (owner, _, family_id, code) = subscribed_owner(&ts, &chain, "anna").await;
    ts.set_open_policy(&owner).await;
    let (member, _) = ts.register("boris", "Boris").await;
    ts.join(&member, &code, "joined").await;
    // Anna leaves; Boris inherits a family he has not paid for.
    let response = ts.post(&owner, "/families/leave", json!({})).await;
    assert_eq!(response.status(), 200);
    assert_eq!(sweep(&ts).await.unpaid, 1);
    assert!(
        family_json(&ts, &member, family_id)
            .await
            .get("delete_at")
            .is_some()
    );
}

#[tokio::test]
async fn switching_subscriptions_off_drops_every_deadline() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (_, owner_id, family_id, _) = subscribed_owner(&ts, &chain, "anna").await;
    expire_subscriptions(&ts, owner_id).await;
    sweep(&ts).await;
    age_deadline(&ts, family_id, days(30)).await;
    // What boot does with the switch off.
    assert_eq!(
        subscriptions::drop_deadlines(&ts.state.pool).await.unwrap(),
        1
    );
    let since: Option<OffsetDateTime> =
        sqlx::query_scalar("SELECT unpaid_since FROM families WHERE id = $1")
            .bind(family_id)
            .fetch_one(&ts.state.pool)
            .await
            .unwrap();
    assert_eq!(
        since, None,
        "a deadline that passed while off never deletes"
    );
}

/// A notification envelope around `transaction` and `renewal`.
fn notification(
    chain: &Chain,
    kind: &str,
    transaction: Option<&str>,
    renewal: Option<&str>,
    bundle: &str,
) -> Value {
    let mut data = json!({"bundleId": bundle, "environment": "Production"});
    if let Some(transaction) = transaction {
        data["signedTransactionInfo"] = json!(transaction);
    }
    if let Some(renewal) = renewal {
        data["signedRenewalInfo"] = json!(renewal);
    }
    json!({"signedPayload": chain.sign(&json!({
        "notificationType": kind,
        "notificationUUID": Uuid::new_v4().to_string(),
        "version": "2.0",
        "signedDate": ms(OffsetDateTime::now_utc()),
        "data": data,
    }))})
}

async fn subscription_row(
    ts: &TestServer,
) -> (OffsetDateTime, Option<OffsetDateTime>, Option<bool>) {
    sqlx::query_as("SELECT expires_at, revoked_at, renews FROM subscriptions")
        .fetch_one(&ts.state.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn apples_notifications_are_applied_by_state() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (anna, anna_id) = ts.register("anna", "Anna").await;
    let token = token_of(&ts, &anna).await;
    let first_end = OffsetDateTime::now_utc() + days(30);
    let first = transaction_payload("42", MONTHLY, first_end, Some(token));
    assert_eq!(present(&ts, &anna, &chain.sign(&first)).await.status(), 200);

    let post = |body: Value| {
        let ts = &ts;
        async move { ts.post_unauth("/appstore/notifications", body).await }
    };

    // A renewal: a later period replaces the held one.
    let second_end = first_end + days(30);
    let second = chain.sign(&transaction_payload("42", MONTHLY, second_end, Some(token)));
    let response = post(notification(
        &chain,
        "DID_RENEW",
        Some(&second),
        None,
        "me.nettrash.FamilyConnect",
    ))
    .await;
    assert_eq!(response.status(), 200);
    let (expires, revoked, _) = subscription_row(&ts).await;
    assert_eq!(expires.unix_timestamp(), second_end.unix_timestamp());
    assert_eq!(revoked, None);

    // A refund of the OLD period arrives late: it ends nothing.
    let mut refunded_first = first.clone();
    refunded_first["revocationDate"] = json!(ms(OffsetDateTime::now_utc()));
    refunded_first["signedDate"] =
        json!(ms(OffsetDateTime::now_utc() + time::Duration::seconds(5)));
    let response = post(notification(
        &chain,
        "REFUND",
        Some(&chain.sign(&refunded_first)),
        None,
        "me.nettrash.FamilyConnect",
    ))
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        subscription_row(&ts).await.1,
        None,
        "an old period's refund ends nothing"
    );
    assert!(
        subscriptions::is_active(&ts.state.pool, &ts.state.cfg, anna_id)
            .await
            .unwrap()
    );

    // A refund of the CURRENT period ends it.
    let mut refunded_second = transaction_payload("42", MONTHLY, second_end, Some(token));
    refunded_second["revocationDate"] = json!(ms(OffsetDateTime::now_utc()));
    refunded_second["signedDate"] =
        json!(ms(OffsetDateTime::now_utc() + time::Duration::seconds(10)));
    let response = post(notification(
        &chain,
        "REFUND",
        Some(&chain.sign(&refunded_second)),
        None,
        "me.nettrash.FamilyConnect",
    ))
    .await;
    assert_eq!(response.status(), 200);
    assert!(subscription_row(&ts).await.1.is_some());
    assert!(
        !subscriptions::is_active(&ts.state.pool, &ts.state.cfg, anna_id)
            .await
            .unwrap()
    );
    let me = me(&ts, &anna).await;
    assert_eq!(me["subscription"]["active"], false);

    // Renewal info: whether it renews, newest by signedDate only.
    let renewal = |renews: i64, at: OffsetDateTime| {
        chain.sign(
            &json!({"originalTransactionId": "42", "autoRenewStatus": renews,
                           "signedDate": ms(at), "environment": "Production"}),
        )
    };
    let now = OffsetDateTime::now_utc();
    post(notification(
        &chain,
        "DID_CHANGE_RENEWAL_STATUS",
        None,
        Some(&renewal(0, now)),
        "me.nettrash.FamilyConnect",
    ))
    .await;
    assert_eq!(subscription_row(&ts).await.2, Some(false));
    post(notification(
        &chain,
        "DID_CHANGE_RENEWAL_STATUS",
        None,
        Some(&renewal(1, now - days(1))),
        "me.nettrash.FamilyConnect",
    ))
    .await;
    assert_eq!(
        subscription_row(&ts).await.2,
        Some(false),
        "an older copy changes nothing"
    );

    // Never presented: answered and dropped. Another app: the same.
    let stranger = chain.sign(&transaction_payload("77", MONTHLY, first_end, None));
    let response = post(notification(
        &chain,
        "SUBSCRIBED",
        Some(&stranger),
        None,
        "me.nettrash.FamilyConnect",
    ))
    .await;
    assert_eq!(response.status(), 200);
    let response = post(notification(
        &chain,
        "SUBSCRIBED",
        Some(&stranger),
        None,
        "me.nettrash.Scan",
    ))
    .await;
    assert_eq!(response.status(), 200);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM subscriptions")
        .fetch_one(&ts.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "a notification never creates a subscription");

    // Forged, or junk: refused, so nothing is believed.
    let forger = Chain::new();
    let response = post(notification(
        &forger,
        "DID_RENEW",
        Some(&second),
        None,
        "me.nettrash.FamilyConnect",
    ))
    .await;
    assert_eq!(response.status(), 400);
    assert_eq!(post(json!({"signedPayload": "a.b.c"})).await.status(), 400);
    assert_eq!(post(json!({})).await.status(), 400);
}

#[tokio::test]
async fn a_notification_this_size_gets_through() {
    // Three certificates per signed value, two values inside a third: the
    // route's own 64 KiB ceiling, not the ordinary 16 KiB.
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let inner = chain.sign(&transaction_payload(
        "1",
        MONTHLY,
        OffsetDateTime::now_utc() + days(30),
        None,
    ));
    let renewal = chain.sign(&json!({"originalTransactionId": "1", "autoRenewStatus": 1,
        "signedDate": ms(OffsetDateTime::now_utc()), "environment": "Production"}));
    let body = notification(
        &chain,
        "DID_RENEW",
        Some(&inner),
        Some(&renewal),
        "me.nettrash.FamilyConnect",
    );
    assert!(
        body.to_string().len() > 4 * 1024,
        "{}",
        body.to_string().len()
    );
    let padded = json!({"signedPayload": body["signedPayload"], "padding": "x".repeat(20 * 1024)});
    assert_eq!(
        ts.post_unauth("/appstore/notifications", padded)
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn a_paid_family_is_held_to_its_storage_allowance() {
    let chain = Chain::new();
    let ts = paid_server_with(&chain, |cfg| cfg.subscriptions.storage_bytes = 1_000).await;
    let (owner, _, family_id, _) = subscribed_owner(&ts, &chain, "anna").await;
    let upload = |bytes: &'static [u8]| {
        let ts = &ts;
        let owner = owner.clone();
        async move {
            ts.client
                .post(ts.url(&format!(
                    "/attachments?kind=file&name=a.bin&family_id={family_id}"
                )))
                .bearer_auth(&owner)
                .header("content-type", "application/octet-stream")
                .body(bytes.to_vec())
                .send()
                .await
                .expect("upload")
        }
    };
    assert_eq!(
        upload(&[7u8; 600]).await.status(),
        201,
        "under the allowance"
    );
    assert_error(upload(&[8u8; 600]).await, 403, "storage_allowance_used").await;
    // A free family on the same server is not held to it.
    family_connect::admin::set_plan(&ts.state.pool, family_id, "free")
        .await
        .unwrap();
    assert_eq!(upload(&[9u8; 600]).await.status(), 201);
}

#[tokio::test]
async fn statistics_show_a_paid_family_its_allowance() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (owner, owner_id, family_id, _) = subscribed_owner(&ts, &chain, "anna").await;
    sqlx::query(
        "INSERT INTO ai_usage (user_id, family_id, prompt_tokens, completion_tokens, images,
                               transcripts, audio_ms)
         VALUES ($1, $2, 1000, 500, 2, 1, 61000),
                ($1, $2, 99999, 99999, 9, 0, 0)",
    )
    .bind(owner_id)
    .bind(family_id)
    .execute(&ts.state.pool)
    .await
    .unwrap();
    // The second row is last month's: it does not count.
    sqlx::query(
        "UPDATE ai_usage SET created_at = date_trunc('month', now() AT TIME ZONE 'UTC')
                                          AT TIME ZONE 'UTC' - interval '1 hour'
          WHERE prompt_tokens = 99999",
    )
    .execute(&ts.state.pool)
    .await
    .unwrap();
    let response = ts
        .get(&owner, &format!("/families/{family_id}/stats"))
        .await;
    assert_eq!(response.status(), 200);
    let stats: Value = response.json().await.unwrap();
    let allowance = &stats["allowance"];
    assert_eq!(
        allowance["ai_tokens"],
        json!({"used": 1500, "limit": 2_000_000})
    );
    assert_eq!(allowance["images"], json!({"used": 2, "limit": 30}));
    assert_eq!(
        allowance["transcript_ms"],
        json!({"used": 61000, "limit": 7_200_000})
    );
    assert_eq!(allowance["storage_bytes"]["limit"], 10_737_418_240_i64);
    let renews = allowance["renews_at"].as_str().unwrap();
    assert!(renews.ends_with("-01T00:00:00Z"), "{renews}");

    // A free family has none.
    family_connect::admin::set_plan(&ts.state.pool, family_id, "free")
        .await
        .unwrap();
    let stats: Value = ts
        .get(&owner, &format!("/families/{family_id}/stats"))
        .await
        .json()
        .await
        .unwrap();
    assert!(stats.get("allowance").is_none());

    // And the assistant's check reads the same numbers.
    family_connect::admin::set_plan(&ts.state.pool, family_id, "paid")
        .await
        .unwrap();
    use family_connect::subscriptions::Spend;
    assert!(
        subscriptions::may_spend(&ts.state, family_id, Spend::Tokens)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE ai_usage SET created_at = now(), images = 30 WHERE prompt_tokens = 1000")
        .execute(&ts.state.pool)
        .await
        .unwrap();
    assert!(
        !subscriptions::may_spend(&ts.state, family_id, Spend::Images)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn deleting_the_account_forgets_its_subscriptions() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (anna, _) = ts.register("anna", "Anna").await;
    let token = token_of(&ts, &anna).await;
    let jws = txn(
        &chain,
        "9",
        OffsetDateTime::now_utc() + days(30),
        Some(token),
    );
    assert_eq!(present(&ts, &anna, &jws).await.status(), 200);
    let response = ts
        .post(&anna, "/me/delete", json!({"password": "password123"}))
        .await;
    assert_eq!(response.status(), 204);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM subscriptions")
        .fetch_one(&ts.state.pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    let token_left: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT app_account_token FROM users WHERE deleted_at IS NOT NULL")
            .fetch_one(&ts.state.pool)
            .await
            .unwrap();
    assert_eq!(token_left, None);
}

#[tokio::test]
async fn a_key_rotation_carries_the_subscriptions_over() {
    let chain = Chain::new();
    let ts = paid_server(&chain).await;
    let (anna, _) = ts.register("anna", "Anna").await;
    let token = token_of(&ts, &anna).await;
    let jws = txn(
        &chain,
        "31",
        OffsetDateTime::now_utc() + days(30),
        Some(token),
    );
    assert_eq!(present(&ts, &anna, &jws).await.status(), 200);
    let new = family_connect::vault::ServerKey::generate();
    family_connect::admin::rotate(&ts.state.pool, &ts.server_key, &new)
        .await
        .expect("rotates");
    let key: Vec<u8> = sqlx::query_scalar("SELECT original_transaction_key FROM subscriptions")
        .fetch_one(&ts.state.pool)
        .await
        .unwrap();
    assert_eq!(key, new.lookup(Lookup::AppStoreTransaction, "31"));
}

mod ws {
    //! The two socket helpers this suite needs, as the other suites write
    //! them.
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpStream;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;
    use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

    pub type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

    pub async fn connect(ts: &TestServer, token: &str) -> WsClient {
        let mut request = ts.ws_url.as_str().into_client_request().expect("request");
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).expect("header"),
        );
        let (mut ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("upgrade");
        ws.send(Message::text(json!({"type": "ping"}).to_string()))
            .await
            .expect("ping");
        next_frame_of_type(&mut ws, "pong").await;
        ws
    }

    pub async fn next_frame_of_type(ws: &mut WsClient, wanted: &str) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let message = tokio::time::timeout_at(deadline, ws.next())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {wanted:?}"))
                .expect("socket closed")
                .expect("socket errored");
            if let Message::Text(text) = message {
                let value: Value = serde_json::from_str(text.as_str()).expect("JSON");
                if value["type"] == wanted {
                    return value;
                }
            }
        }
    }
}
