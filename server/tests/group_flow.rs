//! Integration: groups inside a family (docs/protocol.md, "Groups"; #107).
//!
//! What is under test: a group is visible only to its members, its admin
//! alone renames, adds, removes and deletes, an admin who goes hands it on,
//! the last one out takes it with them — and the family chat is never
//! touched by any of it.

mod common;

use std::time::Duration;

use common::{TestServer, assert_error, spawn_server};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

const FRAME_WAIT: Duration = Duration::from_secs(5);

struct Family {
    olive: String,
    olive_id: i64,
    junior: String,
    junior_id: i64,
    anna: String,
    anna_id: i64,
    max: String,
    max_id: i64,
    invite: String,
}

/// A family of four: Olive owns it; Junior, Anna and Max joined.
async fn family(ts: &TestServer) -> Family {
    let (olive, olive_id) = ts.register("olive", "Olive").await;
    let (junior, junior_id) = ts.register("junior", "Junior").await;
    let (anna, anna_id) = ts.register("anna", "Anna").await;
    let (max, max_id) = ts.register("max", "Max").await;
    let (_, invite) = ts.create_family(&olive, "The Smiths").await;
    ts.set_open_policy(&olive).await;
    for token in [&junior, &anna, &max] {
        ts.join(token, &invite, "joined").await;
    }
    Family {
        olive,
        olive_id,
        junior,
        junior_id,
        anna,
        anna_id,
        max,
        max_id,
        invite,
    }
}

async fn create(ts: &TestServer, token: &str, title: &str, members: &[i64]) -> Value {
    let response = ts
        .post(
            token,
            "/chats/groups",
            json!({"title": title, "member_ids": members}),
        )
        .await;
    assert_eq!(response.status(), 201, "a group is created");
    let body: Value = response.json().await.expect("JSON");
    body["chat"].clone()
}

async fn chats(ts: &TestServer, token: &str) -> Vec<Value> {
    let body: Value = ts.get(token, "/chats").await.json().await.expect("JSON");
    body["chats"].as_array().expect("chats").clone()
}

async fn listed(ts: &TestServer, token: &str, chat_id: i64) -> Option<Value> {
    chats(ts, token)
        .await
        .into_iter()
        .find(|entry| entry["chat"]["id"] == chat_id)
}

async fn send(ts: &TestServer, token: &str, chat_id: i64, body: &str) -> reqwest::Response {
    ts.post_message(token, chat_id, &Uuid::new_v4().to_string(), body)
        .await
}

async fn connect_ws(ts: &TestServer, token: &str) -> WsClient {
    let mut request = ts
        .ws_url
        .as_str()
        .into_client_request()
        .expect("building the ws request");
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).expect("header value"),
    );
    let (mut ws, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("websocket upgrade succeeds");
    ws.send(Message::text(json!({"type": "ping"}).to_string()))
        .await
        .expect("sending a frame");
    next_frame_of_type(&mut ws, "pong").await;
    ws
}

async fn next_frame_of_type(ws: &mut WsClient, wanted: &str) -> Value {
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    loop {
        let message = tokio::time::timeout_at(deadline, ws.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for a {wanted:?} frame"))
            .expect("socket closed while waiting for a frame")
            .expect("socket errored while waiting for a frame");
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(text.as_str()).expect("frames are JSON");
            if value["type"] == wanted {
                return value;
            }
        }
    }
}

/// Created by any member, its creator the admin and first in the list, and
/// listed — with its name, members and admin — for its members alone.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_group_is_created_and_seen_only_by_its_members() {
    let ts = spawn_server().await;
    let f = family(&ts).await;

    let chat = create(
        &ts,
        &f.junior,
        "  Cousins  ",
        &[f.anna_id, f.junior_id, f.anna_id],
    )
    .await;
    assert_eq!(chat["kind"], "group");
    assert_eq!(chat["title"], "Cousins", "the name is trimmed");
    assert_eq!(chat["admin_user_id"], f.junior_id);
    assert_eq!(chat["member_ids"], json!([f.junior_id, f.anna_id]));
    assert_eq!(chat["peer_user_id"], Value::Null);
    let id = chat["id"].as_i64().expect("id");

    let seen = listed(&ts, &f.anna, id).await.expect("a member lists it");
    assert_eq!(seen["chat"], chat);
    assert_eq!(seen["unread_count"], 0);
    assert!(listed(&ts, &f.junior, id).await.is_some());

    // Not to anybody else — the owner included — and to them it does not
    // exist: never `not_chat_member`, which would say that it does.
    for outsider in [&f.max, &f.olive] {
        assert!(listed(&ts, outsider, id).await.is_none());
        assert_error(
            ts.get(outsider, &format!("/chats/{id}/messages")).await,
            404,
            "chat_not_found",
        )
        .await;
        assert_error(
            send(&ts, outsider, id, "hello?").await,
            404,
            "chat_not_found",
        )
        .await;
        assert_error(
            ts.patch(outsider, &format!("/chats/{id}"), json!({"title": "Mine"}))
                .await,
            404,
            "chat_not_found",
        )
        .await;
    }

    // Other chats are listed exactly as before: the family chat has no group
    // fields, and still names the family.
    let family_chat = ts.family_chat_id(&f.anna).await;
    let entry = listed(&ts, &f.anna, family_chat)
        .await
        .expect("family chat");
    assert_eq!(entry["chat"]["title"], "The Smiths");
    assert!(entry["chat"].get("member_ids").is_none());
    assert!(entry["chat"].get("admin_user_id").is_none());
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn creating_a_group_checks_its_name_and_its_members() {
    let ts = spawn_server().await;
    let f = family(&ts).await;
    let groups = |body: Value| {
        let ts = &ts;
        let token = f.junior.clone();
        async move { ts.post(&token, "/chats/groups", body).await }
    };
    for title in ["", "   ", &"x".repeat(65)] {
        assert_error(
            groups(json!({"title": title, "member_ids": [f.anna_id]})).await,
            400,
            "validation",
        )
        .await;
    }
    // Nobody else in it.
    assert_error(
        groups(json!({"title": "Me", "member_ids": [f.junior_id]})).await,
        400,
        "validation",
    )
    .await;
    assert_error(groups(json!({"title": "Me"})).await, 400, "validation").await;
    // Somebody outside the family, and somebody who does not exist.
    let (_, stranger_id) = ts.register("stranger", "Stranger").await;
    for outsider in [stranger_id, 9_999_999] {
        assert_error(
            groups(json!({"title": "Odd", "member_ids": [f.anna_id, outsider]})).await,
            409,
            "not_same_family",
        )
        .await;
    }
    // 64 characters of anything is a name; Cyrillic counts as characters.
    let long = create(&ts, &f.junior, &"я".repeat(64), &[f.anna_id]).await;
    assert_eq!(long["title"].as_str().expect("title").chars().count(), 64);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_family_has_at_most_fifty_groups() {
    let ts = spawn_server().await;
    let f = family(&ts).await;
    for n in 0..50 {
        create(&ts, &f.anna, &format!("Group {n}"), &[f.max_id]).await;
    }
    assert_error(
        ts.post(
            &f.olive,
            "/chats/groups",
            json!({"title": "One more", "member_ids": [f.junior_id]}),
        )
        .await,
        409,
        "too_many_groups",
    )
    .await;
}

/// The admin renames, adds and removes; a member who is not the admin is
/// `not_group_admin` for each.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn only_the_admin_renames_adds_and_removes() {
    let ts = spawn_server().await;
    let f = family(&ts).await;
    let chat = create(&ts, &f.junior, "Cousins", &[f.anna_id]).await;
    let id = chat["id"].as_i64().expect("id");

    assert_error(
        ts.patch(&f.anna, &format!("/chats/{id}"), json!({"title": "Mine"}))
            .await,
        403,
        "not_group_admin",
    )
    .await;
    assert_error(
        ts.post(
            &f.anna,
            &format!("/chats/{id}/members"),
            json!({"user_ids": [f.max_id]}),
        )
        .await,
        403,
        "not_group_admin",
    )
    .await;
    assert_error(
        ts.delete(&f.anna, &format!("/chats/{id}/members/{}", f.junior_id))
            .await,
        403,
        "not_group_admin",
    )
    .await;
    assert_error(
        ts.delete(&f.anna, &format!("/chats/{id}")).await,
        403,
        "not_group_admin",
    )
    .await;

    let renamed: Value = ts
        .patch(
            &f.junior,
            &format!("/chats/{id}"),
            json!({"title": " The cousins "}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(renamed["chat"]["title"], "The cousins");
    assert_error(
        ts.patch(&f.junior, &format!("/chats/{id}"), json!({"title": "  "}))
            .await,
        400,
        "validation",
    )
    .await;

    // A message before Max arrives: he sees it, and none of it is news.
    send(&ts, &f.anna, id, "before Max").await;
    let added: Value = ts
        .post(
            &f.junior,
            &format!("/chats/{id}/members"),
            json!({"user_ids": [f.max_id, f.anna_id]}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        added["chat"]["member_ids"],
        json!([f.junior_id, f.anna_id, f.max_id])
    );
    let max_sees = listed(&ts, &f.max, id).await.expect("Max lists it");
    assert_eq!(
        max_sees["unread_count"], 0,
        "an added member starts with nothing unread"
    );
    assert_eq!(
        max_sees["last_message"]["body"], "before Max",
        "and sees the history"
    );

    let removed: Value = ts
        .delete(&f.junior, &format!("/chats/{id}/members/{}", f.anna_id))
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        removed["chat"]["member_ids"],
        json!([f.junior_id, f.max_id])
    );
    assert!(listed(&ts, &f.anna, id).await.is_none());
    assert_error(
        ts.get(&f.anna, &format!("/chats/{id}/messages")).await,
        404,
        "chat_not_found",
    )
    .await;
    // Removing somebody who is not in it.
    assert_error(
        ts.delete(&f.junior, &format!("/chats/{id}/members/{}", f.olive_id))
            .await,
        400,
        "validation",
    )
    .await;
}

/// An admin who leaves hands the group to the member who joined it
/// earliest; the last one out deletes it.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_admin_leaving_hands_it_on_and_the_last_one_out_deletes_it() {
    let ts = spawn_server().await;
    let f = family(&ts).await;
    let chat = create(&ts, &f.junior, "Cousins", &[f.max_id, f.anna_id]).await;
    let id = chat["id"].as_i64().expect("id");
    let order = chat["member_ids"].as_array().expect("members").clone();
    let next_admin = order[1].as_i64().expect("second member");

    let left = ts
        .delete(&f.junior, &format!("/chats/{id}/members/{}", f.junior_id))
        .await;
    assert_eq!(left.status(), 204);
    let seen = listed(&ts, &f.anna, id).await.expect("still there");
    assert_eq!(seen["chat"]["admin_user_id"], next_admin);
    assert!(listed(&ts, &f.junior, id).await.is_none());

    for token in [&f.anna, &f.max] {
        let me = ts.user_id(token).await;
        let left = ts.delete(token, &format!("/chats/{id}/members/{me}")).await;
        assert_eq!(left.status(), 204);
    }
    // Nobody is left, so nobody can reach it — not even by an id they held.
    assert!(listed(&ts, &f.anna, id).await.is_none());
    assert_error(
        ts.get(&f.max, &format!("/chats/{id}/messages")).await,
        404,
        "chat_not_found",
    )
    .await;
}

/// Deleting is the admin's, final, and never reaches any other kind of
/// chat — least of all the family chat.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn deleting_a_group_removes_it_for_everyone_and_never_touches_the_family_chat() {
    let ts = spawn_server().await;
    let f = family(&ts).await;
    let chat = create(&ts, &f.anna, "Book club", &[f.max_id]).await;
    let id = chat["id"].as_i64().expect("id");
    send(&ts, &f.max, id, "chapter 3?").await;

    let deleted = ts.delete(&f.anna, &format!("/chats/{id}")).await;
    assert_eq!(deleted.status(), 204);
    for token in [&f.anna, &f.max] {
        assert!(listed(&ts, token, id).await.is_none());
    }

    // The family chat: not renamed, not left, not deleted — by the owner
    // either.
    let family_chat = ts.family_chat_id(&f.olive).await;
    for (token, method) in [
        (&f.olive, "patch"),
        (&f.olive, "delete"),
        (&f.anna, "leave"),
    ] {
        let response = match method {
            "patch" => {
                ts.patch(
                    token,
                    &format!("/chats/{family_chat}"),
                    json!({"title": "Ours"}),
                )
                .await
            }
            "delete" => ts.delete(token, &format!("/chats/{family_chat}")).await,
            _ => {
                ts.delete(
                    token,
                    &format!("/chats/{family_chat}/members/{}", f.anna_id),
                )
                .await
            }
        };
        assert_error(response, 400, "validation").await;
    }
    let entry = listed(&ts, &f.anna, family_chat)
        .await
        .expect("still listed");
    assert_eq!(entry["chat"]["title"], "The Smiths");
    // And a direct chat is no group either.
    let direct: Value = ts
        .post(&f.anna, "/chats/direct", json!({"user_id": f.max_id}))
        .await
        .json()
        .await
        .expect("JSON");
    let dm = direct["chat"]["id"].as_i64().expect("id");
    assert_error(
        ts.delete(&f.anna, &format!("/chats/{dm}")).await,
        400,
        "validation",
    )
    .await;
}

/// Mentions name members of the group only; polls may be started in it.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn mentions_name_the_group_s_members_and_polls_are_allowed() {
    let ts = spawn_server().await;
    let f = family(&ts).await;
    let chat = create(&ts, &f.anna, "Cousins", &[f.max_id]).await;
    let id = chat["id"].as_i64().expect("id");
    let mention = |user_id: i64, name: &str| {
        json!({
            "client_msg_id": Uuid::new_v4().to_string(),
            "body": format!("@{name} dinner?"),
            "mentions": [{"user_id": user_id, "name": name}],
        })
    };
    let ok = ts
        .post(
            &f.anna,
            &format!("/chats/{id}/messages"),
            mention(f.max_id, "Max"),
        )
        .await;
    assert_eq!(ok.status(), 201);
    // Junior is in the family but not in the group.
    assert_error(
        ts.post(
            &f.anna,
            &format!("/chats/{id}/messages"),
            mention(f.junior_id, "Junior"),
        )
        .await,
        400,
        "validation",
    )
    .await;

    let poll = ts
        .post(
            &f.anna,
            &format!("/chats/{id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "Pizza or pasta?",
                "poll": {"options": ["Pizza", "Pasta"]},
            }),
        )
        .await;
    assert_eq!(poll.status(), 201, "a poll in a group");
}

/// A group appears on a member's devices the moment they are added, and
/// goes the moment they are removed — no resync needed.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn members_are_told_as_a_group_changes() {
    let ts = spawn_server().await;
    let f = family(&ts).await;
    let mut max_ws = connect_ws(&ts, &f.max).await;
    let mut anna_ws = connect_ws(&ts, &f.anna).await;

    let chat = create(&ts, &f.anna, "Cousins", &[f.max_id]).await;
    let id = chat["id"].as_i64().expect("id");
    let created = next_frame_of_type(&mut max_ws, "chat_updated").await;
    assert_eq!(created["chat"], chat);

    ts.patch(
        &f.anna,
        &format!("/chats/{id}"),
        json!({"title": "Cousins!"}),
    )
    .await;
    let renamed = next_frame_of_type(&mut max_ws, "chat_updated").await;
    assert_eq!(renamed["chat"]["title"], "Cousins!");

    ts.delete(&f.anna, &format!("/chats/{id}/members/{}", f.max_id))
        .await;
    let removed = next_frame_of_type(&mut max_ws, "chat_removed").await;
    assert_eq!(removed["chat_id"], id);
    // Anna has every change on her socket, the creation and the rename
    // first; the one after the removal names her alone.
    let mut updated = next_frame_of_type(&mut anna_ws, "chat_updated").await;
    while updated["chat"]["member_ids"] != json!([f.anna_id]) {
        updated = next_frame_of_type(&mut anna_ws, "chat_updated").await;
    }
    assert_eq!(updated["chat"]["title"], "Cousins!");

    ts.delete(&f.anna, &format!("/chats/{id}")).await;
    let gone = next_frame_of_type(&mut anna_ws, "chat_removed").await;
    assert_eq!(gone["chat_id"], id);
}

/// Leaving the family takes a member out of every group, with the hand-off,
/// and coming back does not put them back.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn leaving_the_family_leaves_every_group_and_a_rejoin_does_not_restore_it() {
    let ts = spawn_server().await;
    let f = family(&ts).await;
    let chat = create(&ts, &f.max, "Cousins", &[f.anna_id, f.junior_id]).await;
    let id = chat["id"].as_i64().expect("id");
    send(&ts, &f.max, id, "my words stay").await;

    let left = ts.post(&f.max, "/families/leave", json!({})).await;
    assert_eq!(left.status(), 204);
    let seen = listed(&ts, &f.anna, id).await.expect("the group stays");
    assert_eq!(seen["chat"]["admin_user_id"], f.anna_id, "handed on");
    assert_eq!(seen["chat"]["member_ids"], json!([f.anna_id, f.junior_id]));
    assert_eq!(seen["last_message"]["body"], "my words stay");

    ts.join(&f.max, &f.invite, "joined").await;
    assert!(
        listed(&ts, &f.max, id).await.is_none(),
        "a group is joined by being added"
    );
}

/// The owner removing a member, and a member deleting their account, do the
/// same — and an account's words stay behind it.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn removal_and_account_deletion_leave_every_group_too() {
    let ts = spawn_server().await;
    let f = family(&ts).await;
    let removed_from = create(&ts, &f.max, "Max's", &[f.anna_id]).await;
    let removed_id = removed_from["id"].as_i64().expect("id");
    let deleted_from = create(&ts, &f.junior, "Junior's", &[f.anna_id]).await;
    let deleted_id = deleted_from["id"].as_i64().expect("id");
    send(&ts, &f.junior, deleted_id, "still here").await;

    let removed = ts
        .delete(&f.olive, &format!("/families/members/{}", f.max_id))
        .await;
    assert_eq!(removed.status(), 204);
    let seen = listed(&ts, &f.anna, removed_id).await.expect("stays");
    assert_eq!(seen["chat"]["admin_user_id"], f.anna_id);
    assert_eq!(seen["chat"]["member_ids"], json!([f.anna_id]));

    let gone = ts
        .post(&f.junior, "/me/delete", json!({"password": "password123"}))
        .await;
    assert!(gone.status().is_success(), "{}", gone.status());
    let seen = listed(&ts, &f.anna, deleted_id).await.expect("stays");
    assert_eq!(seen["chat"]["admin_user_id"], f.anna_id);
    assert_eq!(seen["last_message"]["body"], "still here");
    assert_eq!(seen["last_message"]["sender_id"], f.junior_id);
}
