//! SCRATCH probe file for review — delete after use.

mod common;

use common::{TestServer, spawn_server};
use serde_json::{Value, json};
use uuid::Uuid;

async fn family_with_a_message(ts: &TestServer) -> (String, i64, String, i64, i64, i64) {
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;
    let response = ts
        .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), "See you at six")
        .await;
    let body: Value = response.json().await.expect("json");
    let message_id = body["message"]["id"].as_i64().expect("id");
    (owner, owner_id, member, member_id, chat_id, message_id)
}

async fn post_raw(ts: &TestServer, token: &str, chat_id: i64, payload: Value) -> reqwest::Response {
    ts.post(token, &format!("/chats/{chat_id}/messages"), payload)
        .await
}

#[tokio::test]
#[ignore]
async fn probe_retry_paths() {
    let server = spawn_server().await;
    let (owner, owner_id, member, member_id, chat_id, quoted_id) =
        family_with_a_message(&server).await;
    let _ = (owner_id, member_id);

    // --- 1. plain retry, identical payload
    let cmid = Uuid::new_v4().to_string();
    let first = post_raw(
        &server,
        &member,
        chat_id,
        json!({"client_msg_id": cmid, "body": "Six works", "reply_to_message_id": quoted_id}),
    )
    .await;
    let first_status = first.status();
    let first_body: Value = first.json().await.expect("json");
    let retry = post_raw(
        &server,
        &member,
        chat_id,
        json!({"client_msg_id": cmid, "body": "Six works", "reply_to_message_id": quoted_id}),
    )
    .await;
    let retry_status = retry.status();
    let retry_body: Value = retry.json().await.expect("json");
    println!("PROBE1 first={first_status} {}", first_body["message"]);
    println!("PROBE1 retry={retry_status} {}", retry_body["message"]);

    // --- 2. retry that DROPS the reply target
    let cmid2 = Uuid::new_v4().to_string();
    let a = post_raw(
        &server,
        &member,
        chat_id,
        json!({"client_msg_id": cmid2, "body": "Yep", "reply_to_message_id": quoted_id}),
    )
    .await;
    println!("PROBE2 first={} ", a.status());
    let b = post_raw(
        &server,
        &member,
        chat_id,
        json!({"client_msg_id": cmid2, "body": "Yep"}),
    )
    .await;
    let bs = b.status();
    let bb: Value = b.json().await.expect("json");
    println!("PROBE2 retry-without-target={bs} {}", bb["message"]);

    // --- 3. retry whose reply target is no longer valid (points elsewhere)
    let cmid3 = Uuid::new_v4().to_string();
    let c = post_raw(
        &server,
        &member,
        chat_id,
        json!({"client_msg_id": cmid3, "body": "Third", "reply_to_message_id": quoted_id}),
    )
    .await;
    println!("PROBE3 first={}", c.status());
    let d = post_raw(
        &server,
        &member,
        chat_id,
        json!({"client_msg_id": cmid3, "body": "Third", "reply_to_message_id": 999_999}),
    )
    .await;
    let ds = d.status();
    let db: Value = d.json().await.expect("json");
    println!("PROBE3 retry-with-bogus-target={ds} {db}");

    // --- 4. chat-list preview of a reply
    let list: Value = server
        .get(&owner, "/chats")
        .await
        .json()
        .await
        .expect("json");
    let fam = list["chats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["chat"]["kind"] == "family")
        .unwrap();
    println!("PROBE4 last_message={}", fam["last_message"]);

    // --- 5. before_id page carries the quote
    let page: Value = server
        .get(
            &owner,
            &format!("/chats/{chat_id}/messages?before_id=999999999"),
        )
        .await
        .json()
        .await
        .expect("json");
    println!("PROBE5 before_id page={}", page["messages"]);

    // --- 6. reply to a reply: is the snippet nested?
    let nested: Value = post_raw(
        &server,
        &owner,
        chat_id,
        json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "Grand",
               "reply_to_message_id": first_body["message"]["id"]}),
    )
    .await
    .json()
    .await
    .expect("json");
    println!("PROBE6 reply-to-reply={}", nested["message"]);

    // --- 7. reply to one's own message / to a message id of a chat member who left
    println!("PROBE7 quoted_id={quoted_id}");
}
