//! Integration: the account lifecycle — register, login, `/me`, logout —
//! and the auth error codes along the way.

mod common;

use common::{assert_error, spawn_server};
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn registering_returns_a_token_and_me_reflects_the_new_account() {
    let ts = spawn_server().await;

    let (token, user_id) = ts.register("anna", "Anna").await;
    assert_eq!(token.len(), 43, "opaque session token shape");

    let response = ts.get(&token, "/me").await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("me is JSON");
    assert_eq!(body["user"]["id"], user_id);
    assert_eq!(body["user"]["username"], "anna");
    assert_eq!(body["user"]["display_name"], "Anna");
    assert!(body["user"]["created_at"].is_string(), "rfc3339 timestamp");
    assert_eq!(body["family"], Value::Null);
    assert_eq!(body["role"], Value::Null);
    assert_eq!(body["pending_join_request"], Value::Null);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn usernames_are_case_insensitively_unique() {
    let ts = spawn_server().await;
    ts.register("anna", "Anna").await;
    let response = ts
        .post_unauth(
            "/auth/register",
            json!({"username": "ANNA", "display_name": "Impostor", "password": "password123"}),
        )
        .await;
    assert_error(response, 409, "username_taken").await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn registration_validates_username_password_and_display_name() {
    let ts = spawn_server().await;
    for (body, why) in [
        (
            json!({"username": "ab", "display_name": "X", "password": "password123"}),
            "username too short",
        ),
        (
            json!({"username": "has space", "display_name": "X", "password": "password123"}),
            "username charset",
        ),
        (
            json!({"username": "valid.name", "display_name": "X", "password": "short"}),
            "password too short",
        ),
        (
            json!({"username": "valid.name", "display_name": "   ", "password": "password123"}),
            "display name empty after trim",
        ),
    ] {
        let response = ts.post_unauth("/auth/register", body).await;
        assert_eq!(response.status(), 400, "{why}");
        let json: Value = response.json().await.expect("error body");
        assert_eq!(json["error"]["code"], "validation", "{why}");
    }
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn login_works_with_correct_credentials_and_401s_otherwise() {
    let ts = spawn_server().await;
    ts.register("anna", "Anna").await;

    // Correct credentials — username case-insensitive.
    let response = ts
        .post_unauth(
            "/auth/login",
            json!({"username": "Anna", "password": "password123"}),
        )
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("login is JSON");
    assert!(body["token"].as_str().expect("token").len() == 43);
    assert_eq!(body["user"]["username"], "anna");

    // Wrong password and unknown user produce the same error.
    let wrong_pw = ts
        .post_unauth(
            "/auth/login",
            json!({"username": "anna", "password": "wrong-password"}),
        )
        .await;
    assert_error(wrong_pw, 401, "invalid_credentials").await;
    let unknown = ts
        .post_unauth(
            "/auth/login",
            json!({"username": "nobody", "password": "password123"}),
        )
        .await;
    assert_error(unknown, 401, "invalid_credentials").await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn logout_revokes_the_session_but_only_that_session() {
    let ts = spawn_server().await;
    let (token_a, _) = ts.register("anna", "Anna").await;

    // A second session from a "second device".
    let response = ts
        .post_unauth(
            "/auth/login",
            json!({"username": "anna", "password": "password123"}),
        )
        .await;
    let token_b = response.json::<Value>().await.expect("json")["token"]
        .as_str()
        .expect("token")
        .to_string();

    let response = ts.post(&token_a, "/auth/logout", json!({})).await;
    assert_eq!(response.status(), 204);

    let dead = ts.get(&token_a, "/me").await;
    assert_error(dead, 401, "unauthorized").await;
    let alive = ts.get(&token_b, "/me").await;
    assert_eq!(alive.status(), 200, "other sessions survive a logout");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn requests_without_or_with_garbage_tokens_are_unauthorized() {
    let ts = spawn_server().await;
    let no_auth = ts
        .client
        .get(ts.url("/me"))
        .send()
        .await
        .expect("request sends");
    assert_error(no_auth, 401, "unauthorized").await;
    let bad_token = ts.get("not-a-real-token", "/me").await;
    assert_error(bad_token, 401, "unauthorized").await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn healthz_answers_ok_without_authentication() {
    let ts = spawn_server().await;
    let response = ts
        .client
        .get(ts.url("/healthz"))
        .send()
        .await
        .expect("request sends");
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("health is JSON");
    assert_eq!(body, json!({"status": "ok"}));
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn malformed_json_bodies_get_the_protocol_error_shape() {
    let ts = spawn_server().await;
    let response = ts
        .client
        .post(ts.url("/auth/register"))
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .expect("request sends");
    assert_error(response, 400, "validation").await;
}
