//! Threads (docs/protocol.md, "Threads"): a chain of replies, rooted at the
//! TOP, that can be read on its own.
//!
//! What is pinned: that every reply carries the root of its chain however
//! deep the quoting goes; that the root — and only the root — counts its
//! chain, and never as 0; that the thread read answers the root first and
//! the chain oldest-first whichever member of it was named, pages with
//! `after_id`, and is a chain of one for a message nobody answered; that it
//! is scoped to the chat; and that retention sweeping the root frees the
//! chain rather than the replies.

mod common;

use common::{TestServer, assert_error, spawn_server};
use serde_json::{Value, json};
use uuid::Uuid;

async fn family_of_two(ts: &TestServer) -> (String, String, i64) {
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;
    (owner, member, chat_id)
}

async fn send(ts: &TestServer, token: &str, chat_id: i64, body: &str) -> Value {
    let response = ts
        .post(
            token,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": body}),
        )
        .await;
    assert_eq!(response.status(), 201, "sending {body:?}");
    let value: Value = response.json().await.expect("JSON");
    value["message"].clone()
}

async fn reply(ts: &TestServer, token: &str, chat_id: i64, body: &str, to: i64) -> Value {
    let response = ts
        .post(
            token,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": body,
                "reply_to_message_id": to,
            }),
        )
        .await;
    assert_eq!(response.status(), 201, "replying {body:?}");
    let value: Value = response.json().await.expect("JSON");
    value["message"].clone()
}

fn id(message: &Value) -> i64 {
    message["id"].as_i64().expect("id")
}

/// The newest page of the chat, as whole messages keyed by id.
async fn page(ts: &TestServer, token: &str, chat_id: i64) -> Vec<Value> {
    let response = ts.get(token, &format!("/chats/{chat_id}/messages")).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("JSON");
    body["messages"].as_array().expect("messages").clone()
}

fn find(messages: &[Value], message_id: i64) -> &Value {
    messages
        .iter()
        .find(|m| id(m) == message_id)
        .unwrap_or_else(|| panic!("message {message_id} on the page"))
}

async fn thread(ts: &TestServer, token: &str, chat_id: i64, message_id: i64, query: &str) -> Value {
    let response = ts
        .get(
            token,
            &format!("/chats/{chat_id}/messages/{message_id}/thread{query}"),
        )
        .await;
    assert_eq!(response.status(), 200, "reading the thread of {message_id}");
    response.json().await.expect("JSON")
}

fn ids(body: &Value) -> Vec<i64> {
    body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(id)
        .collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_reply_carries_the_root_of_its_chain_however_deep() {
    let ts = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&ts).await;
    let root = send(&ts, &owner, chat_id, "Dinner at 7?").await;
    let first = reply(&ts, &member, chat_id, "Works for me", id(&root)).await;
    let second = reply(&ts, &owner, chat_id, "Pizza then?", id(&first)).await;
    let third = reply(&ts, &member, chat_id, "Always", id(&second)).await;

    // Rooted at the TOP, not at the parent: the deeper the quoting goes the
    // more this matters, and a depth of three is where a "parent's parent"
    // implementation would first show.
    assert_eq!(first["thread_root_id"], id(&root), "{first}");
    assert_eq!(second["thread_root_id"], id(&root), "{second}");
    assert_eq!(third["thread_root_id"], id(&root), "{third}");
    // The quote is untouched by the chain: still the message it answered.
    assert_eq!(third["reply_to"]["message_id"], id(&second), "{third}");
    assert_eq!(
        second["reply_to"]["parent"]["message_id"],
        id(&root),
        "{second}"
    );
    // And a root is not a reply, so it names no root.
    assert!(root.get("thread_root_id").is_none(), "{root}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_ordinary_message_carries_neither_field() {
    let ts = spawn_server().await;
    let (owner, _member, chat_id) = family_of_two(&ts).await;
    let lonely = send(&ts, &owner, chat_id, "Anyone up?").await;
    assert!(lonely.get("thread_root_id").is_none(), "{lonely}");
    assert!(
        lonely.get("reply_count").is_none(),
        "absent, never 0: {lonely}"
    );

    // Not just on the send response: on a history page too, which is the
    // read that carries the recomputed count.
    let listed = page(&ts, &owner, chat_id).await;
    let copy = find(&listed, id(&lonely));
    assert!(copy.get("thread_root_id").is_none(), "{copy}");
    assert!(copy.get("reply_count").is_none(), "{copy}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_root_counts_its_chain_and_nothing_else_does() {
    let ts = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&ts).await;
    let root = send(&ts, &owner, chat_id, "Dinner at 7?").await;
    let first = reply(&ts, &member, chat_id, "Works for me", id(&root)).await;
    let nested = reply(&ts, &owner, chat_id, "Pizza then?", id(&first)).await;
    let another = reply(&ts, &member, chat_id, "Can't, sorry", id(&root)).await;
    let lonely = send(&ts, &owner, chat_id, "Anyone seen the cat?").await;

    let listed = page(&ts, &member, chat_id).await;
    // Three messages name the root as theirs — the nested one included,
    // because the chain is the root's and not the parent's.
    assert_eq!(find(&listed, id(&root))["reply_count"], 3, "{listed:?}");
    // A reply in the middle of a chain has no thread of its own to count,
    // even though one message quotes it directly.
    for message in [&first, &nested, &another] {
        let copy = find(&listed, id(message));
        assert!(
            copy.get("reply_count").is_none(),
            "only the root counts: {copy}"
        );
    }
    assert!(find(&listed, id(&lonely)).get("reply_count").is_none());

    // The same count on the thread read's own copy of the root.
    let body = thread(&ts, &owner, chat_id, id(&root), "").await;
    assert_eq!(body["messages"][0]["reply_count"], 3, "{body}");
    assert!(body["messages"][1].get("reply_count").is_none(), "{body}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_thread_read_answers_the_root_first_then_the_chain_oldest_first() {
    let ts = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&ts).await;
    let before = send(&ts, &owner, chat_id, "Morning").await;
    let root = send(&ts, &owner, chat_id, "Dinner at 7?").await;
    let first = reply(&ts, &member, chat_id, "Works for me", id(&root)).await;
    let _between = send(&ts, &member, chat_id, "Unrelated").await;
    let nested = reply(&ts, &owner, chat_id, "Pizza then?", id(&first)).await;
    let another = reply(&ts, &member, chat_id, "Can't, sorry", id(&root)).await;
    let expected = vec![id(&root), id(&first), id(&nested), id(&another)];

    // Named by the root.
    let body = thread(&ts, &owner, chat_id, id(&root), "").await;
    assert_eq!(ids(&body), expected, "{body}");
    // Whole messages: the quote rides, so the surface draws it.
    assert_eq!(body["messages"][2]["reply_to"]["message_id"], id(&first));
    assert_eq!(body["messages"][2]["body"], "Pizza then?");

    // Named by a reply in the middle, and by the last one: the SAME chain.
    let by_nested = thread(&ts, &member, chat_id, id(&nested), "").await;
    assert_eq!(ids(&by_nested), expected, "{by_nested}");
    let by_last = thread(&ts, &member, chat_id, id(&another), "").await;
    assert_eq!(ids(&by_last), expected, "{by_last}");

    // What is NOT in it: the unrelated messages around it.
    assert!(!ids(&body).contains(&id(&before)));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_chain_of_one_is_not_an_error() {
    let ts = spawn_server().await;
    let (owner, _member, chat_id) = family_of_two(&ts).await;
    let lonely = send(&ts, &owner, chat_id, "Anyone up?").await;
    let body = thread(&ts, &owner, chat_id, id(&lonely), "").await;
    assert_eq!(ids(&body), vec![id(&lonely)], "{body}");
    assert!(body["messages"][0].get("reply_count").is_none(), "{body}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_thread_read_pages_with_after_id_and_the_root_rides_only_while_newer() {
    let ts = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&ts).await;
    let root = send(&ts, &owner, chat_id, "Dinner at 7?").await;
    let first = reply(&ts, &member, chat_id, "Works for me", id(&root)).await;
    let second = reply(&ts, &owner, chat_id, "Pizza then?", id(&first)).await;
    let third = reply(&ts, &member, chat_id, "Always", id(&root)).await;

    let one = thread(&ts, &owner, chat_id, id(&root), "?limit=2").await;
    assert_eq!(ids(&one), vec![id(&root), id(&first)], "{one}");
    let two = thread(
        &ts,
        &owner,
        chat_id,
        id(&root),
        &format!("?limit=2&after_id={}", id(&first)),
    )
    .await;
    assert_eq!(ids(&two), vec![id(&second), id(&third)], "{two}");
    let done = thread(
        &ts,
        &owner,
        chat_id,
        id(&root),
        &format!("?limit=2&after_id={}", id(&third)),
    )
    .await;
    assert_eq!(ids(&done), Vec::<i64>::new(), "the short page: {done}");
    // An after_id BELOW the root still carries it: it is simply the oldest.
    let from_zero = thread(&ts, &owner, chat_id, id(&root), "?after_id=0").await;
    assert_eq!(ids(&from_zero)[0], id(&root));
    // The paging is by the chain named, whichever member named it.
    let via_reply = thread(
        &ts,
        &member,
        chat_id,
        id(&second),
        &format!("?after_id={}", id(&root)),
    )
    .await;
    assert_eq!(ids(&via_reply), vec![id(&first), id(&second), id(&third)]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_thread_read_is_scoped_to_the_chat_and_its_members() {
    let ts = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&ts).await;
    let root = send(&ts, &owner, chat_id, "Dinner at 7?").await;
    reply(&ts, &member, chat_id, "Works for me", id(&root)).await;

    // A real message in ANOTHER chat — another family's — is not found
    // through this chat's door, exactly as `reply_to_message_id` treats it.
    let (neighbour, _) = ts.register("neighbour", "Nadia").await;
    ts.create_family(&neighbour, "The Joneses").await;
    let their_chat = ts.family_chat_id(&neighbour).await;
    let elsewhere = send(&ts, &neighbour, their_chat, "Dinner at 8?").await;
    assert_error(
        ts.get(
            &member,
            &format!("/chats/{chat_id}/messages/{}/thread", id(&elsewhere)),
        )
        .await,
        404,
        "message_not_found",
    )
    .await;
    // An id that exists nowhere reads the same.
    assert_error(
        ts.get(
            &member,
            &format!("/chats/{chat_id}/messages/99999999/thread"),
        )
        .await,
        404,
        "message_not_found",
    )
    .await;
    // Access is the chat's.
    let (stranger, _) = ts.register("stranger", "Sam").await;
    assert_error(
        ts.get(
            &stranger,
            &format!("/chats/{chat_id}/messages/{}/thread", id(&root)),
        )
        .await,
        403,
        "not_chat_member",
    )
    .await;
    assert_error(
        ts.get(
            &member,
            &format!("/chats/99999999/messages/{}/thread", id(&root)),
        )
        .await,
        404,
        "chat_not_found",
    )
    .await;
    assert_error(
        ts.get(
            &member,
            &format!("/chats/{chat_id}/messages/{}/thread?limit=abc", id(&root)),
        )
        .await,
        400,
        "invalid_pagination",
    )
    .await;
}

/// Retention sweeping the ROOT frees the chain, not the replies: they stay
/// in the scroll, keep what quotes survive, and stop belonging to a thread
/// — there is no root left to open one from. A reply whose parent was
/// swept but whose root was not stays in the chain, because the chain is
/// the root's and not the parent's.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_swept_root_frees_its_chain_and_a_swept_parent_does_not() {
    let ts = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&ts).await;
    let days = ts.state.cfg.limits.retention_days;

    // Chain A: root → first → nested. The FIRST reply is swept; the root
    // survives, so the nested reply stays in the chain.
    let root_a = send(&ts, &owner, chat_id, "Dinner at 7?").await;
    let first_a = reply(&ts, &member, chat_id, "Works for me", id(&root_a)).await;
    let nested_a = reply(&ts, &owner, chat_id, "Pizza then?", id(&first_a)).await;
    // Chain B: root → reply. The ROOT is swept.
    let root_b = send(&ts, &owner, chat_id, "Film tonight?").await;
    let reply_b = reply(&ts, &member, chat_id, "Which one?", id(&root_b)).await;
    let nested_b = reply(&ts, &owner, chat_id, "The long one", id(&reply_b)).await;

    let pool = &ts.state.pool;
    let age = |message_id: i64| async move {
        sqlx::query(
            "UPDATE messages SET created_at = now() - make_interval(days => $2) WHERE id = $1",
        )
        .bind(message_id)
        .bind((days + 1) as i32)
        .execute(pool)
        .await
        .expect("aging the message");
    };
    age(id(&first_a)).await;
    age(id(&root_b)).await;
    let swept = family_connect::handlers_chat::sweep_expired_messages(&ts.state)
        .await
        .expect("sweep");
    assert_eq!(swept, 2);

    let listed = page(&ts, &owner, chat_id).await;
    // Chain A: the nested reply lost its quote (0012) but not its chain.
    let nested = find(&listed, id(&nested_a));
    assert!(nested.get("reply_to").is_none(), "{nested}");
    assert_eq!(nested["thread_root_id"], id(&root_a), "{nested}");
    assert_eq!(find(&listed, id(&root_a))["reply_count"], 1, "one survivor");
    let body = thread(&ts, &owner, chat_id, id(&nested_a), "").await;
    assert_eq!(ids(&body), vec![id(&root_a), id(&nested_a)], "{body}");

    // Chain B: both replies stay, keep the quote between them, and belong
    // to no chain any more.
    let orphan = find(&listed, id(&reply_b));
    assert!(orphan.get("thread_root_id").is_none(), "{orphan}");
    assert!(orphan.get("reply_to").is_none(), "its parent was the root");
    let deep = find(&listed, id(&nested_b));
    assert!(deep.get("thread_root_id").is_none(), "{deep}");
    assert_eq!(deep["reply_to"]["message_id"], id(&reply_b), "{deep}");
    assert_error(
        ts.get(
            &owner,
            &format!("/chats/{chat_id}/messages/{}/thread", id(&root_b)),
        )
        .await,
        404,
        "message_not_found",
    )
    .await;
    // Each survivor is now a chain of one — not a new chain of two: the
    // link between them is a quote, and a chain is decided at send time.
    let alone = thread(&ts, &owner, chat_id, id(&nested_b), "").await;
    assert_eq!(ids(&alone), vec![id(&nested_b)], "{alone}");
}

/// Owner and member replies share one root, and the root counts both. (The
/// assistant's answer, inserted by a path of its own, is pinned where the
/// mock provider lives: assistant_flow.rs,
/// `the_assistants_answer_is_in_the_mentions_chain`.)
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_reply_to_a_reply_by_the_owner_has_the_same_root_as_a_members() {
    let ts = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&ts).await;
    let root = send(&ts, &member, chat_id, "Who's cooking?").await;
    let by_owner = reply(&ts, &owner, chat_id, "Me", id(&root)).await;
    let by_member = reply(&ts, &member, chat_id, "Thanks!", id(&by_owner)).await;
    assert_eq!(by_owner["thread_root_id"], id(&root));
    assert_eq!(by_member["thread_root_id"], id(&root));
    assert_eq!(
        find(&page(&ts, &owner, chat_id).await, id(&root))["reply_count"],
        2
    );
}

/// A message left without a root is a root again for whatever answers it
/// afterwards: the answer is rooted at the survivor, which then carries
/// BOTH its own surviving quote and a count — the one message that does
/// (protocol.md, "Retention").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_survivor_of_a_swept_root_heads_a_new_chain_when_answered() {
    let ts = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&ts).await;
    let days = ts.state.cfg.limits.retention_days;
    let root = send(&ts, &owner, chat_id, "Dinner at 7?").await;
    let first = reply(&ts, &member, chat_id, "Works for me", id(&root)).await;
    let second = reply(&ts, &owner, chat_id, "Pizza then?", id(&first)).await;

    sqlx::query("UPDATE messages SET created_at = now() - make_interval(days => $2) WHERE id = $1")
        .bind(id(&root))
        .bind((days + 1) as i32)
        .execute(&ts.state.pool)
        .await
        .expect("aging the root");
    assert_eq!(
        family_connect::handlers_chat::sweep_expired_messages(&ts.state)
            .await
            .expect("sweep"),
        1
    );

    // Answering the LAST survivor: rooted at it, not at the swept root and
    // not at the first survivor either — a chain is decided at send time,
    // and `second` never had a root after the sweep.
    let third = reply(&ts, &member, chat_id, "Always", id(&second)).await;
    assert_eq!(third["thread_root_id"], id(&second), "{third}");

    let listed = page(&ts, &owner, chat_id).await;
    let head = find(&listed, id(&second));
    assert_eq!(
        head["reply_to"]["message_id"],
        id(&first),
        "its own quote survives: {head}"
    );
    assert_eq!(
        head["reply_count"], 1,
        "and it counts the new chain: {head}"
    );
    assert!(
        head.get("thread_root_id").is_none(),
        "it belongs to no chain of its own: {head}"
    );
    let untouched = find(&listed, id(&first));
    assert!(untouched.get("reply_count").is_none(), "{untouched}");
    let body = thread(&ts, &member, chat_id, id(&third), "").await;
    assert_eq!(ids(&body), vec![id(&second), id(&third)], "{body}");
}
