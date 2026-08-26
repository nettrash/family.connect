//! Integration: emoji reactions — set/replace/remove semantics, no-op
//! detection, validation and authorization, embedding in message pages,
//! and the seq-cursor catch-up surface.

mod common;

use common::{TestServer, assert_error, spawn_server};
use serde_json::{Value, json};
use uuid::Uuid;

/// Family of two with one message posted by the owner; returns
/// `(owner_token, owner_id, member_token, member_id, chat_id, message_id)`.
async fn family_with_a_message(ts: &TestServer) -> (String, i64, String, i64, i64, i64) {
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;
    let response = ts
        .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), "Dinner at 7?")
        .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("message response is JSON");
    let message_id = body["message"]["id"].as_i64().expect("message id");
    (owner, owner_id, member, member_id, chat_id, message_id)
}

async fn put_reaction(
    ts: &TestServer,
    token: &str,
    chat_id: i64,
    message_id: i64,
    emoji: &str,
) -> reqwest::Response {
    ts.put(
        token,
        &format!("/chats/{chat_id}/messages/{message_id}/reaction"),
        json!({"emoji": emoji}),
    )
    .await
}

async fn state_of(response: reqwest::Response) -> (i64, Vec<(i64, String)>) {
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("reaction response is JSON");
    let seq = body["reaction_seq"].as_i64().expect("reaction_seq present");
    let reactions = body["reactions"]
        .as_array()
        .expect("reactions is an array")
        .iter()
        .map(|r| {
            (
                r["user_id"].as_i64().expect("user_id"),
                r["emoji"].as_str().expect("emoji").to_string(),
            )
        })
        .collect();
    (seq, reactions)
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn setting_replacing_and_removing_a_reaction_is_an_idempotent_state_set() {
    let ts = spawn_server().await;
    let (owner, owner_id, member, member_id, chat_id, message_id) =
        family_with_a_message(&ts).await;

    // First reaction bumps the seq from zero.
    let (seq1, reactions) =
        state_of(put_reaction(&ts, &member, chat_id, message_id, "❤️").await).await;
    assert!(seq1 > 0);
    assert_eq!(reactions, vec![(member_id, "❤️".to_string())]);

    // Re-PUT of the same emoji is a no-op: same state, same seq.
    let (seq_noop, reactions) =
        state_of(put_reaction(&ts, &member, chat_id, message_id, "❤️").await).await;
    assert_eq!(seq_noop, seq1, "a no-op re-PUT must not burn a seq");
    assert_eq!(reactions, vec![(member_id, "❤️".to_string())]);

    // A different emoji replaces (one reaction per user per message).
    let (seq2, reactions) =
        state_of(put_reaction(&ts, &member, chat_id, message_id, "👍").await).await;
    assert!(seq2 > seq1);
    assert_eq!(reactions, vec![(member_id, "👍".to_string())]);

    // A second user's reaction joins the state.
    let (seq3, reactions) =
        state_of(put_reaction(&ts, &owner, chat_id, message_id, "😂").await).await;
    assert!(seq3 > seq2);
    assert_eq!(reactions.len(), 2);
    assert!(reactions.contains(&(member_id, "👍".to_string())));
    assert!(reactions.contains(&(owner_id, "😂".to_string())));

    // Removing is idempotent too.
    let (seq4, reactions) = state_of(
        ts.delete(
            &member,
            &format!("/chats/{chat_id}/messages/{message_id}/reaction"),
        )
        .await,
    )
    .await;
    assert!(seq4 > seq3);
    assert_eq!(reactions, vec![(owner_id, "😂".to_string())]);

    let (seq5, reactions) = state_of(
        ts.delete(
            &member,
            &format!("/chats/{chat_id}/messages/{message_id}/reaction"),
        )
        .await,
    )
    .await;
    assert_eq!(seq5, seq4, "deleting nothing must not burn a seq");
    assert_eq!(reactions, vec![(owner_id, "😂".to_string())]);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn reaction_validation_authorization_and_message_lookup_are_enforced() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, chat_id, message_id) = family_with_a_message(&ts).await;

    // Emoji validation: empty and oversized are rejected.
    assert_error(
        put_reaction(&ts, &member, chat_id, message_id, "   ").await,
        400,
        "invalid_emoji",
    )
    .await;
    assert_error(
        put_reaction(&ts, &member, chat_id, message_id, &"x".repeat(33)).await,
        400,
        "invalid_emoji",
    )
    .await;

    // A user outside the chat gets 403, an unknown chat 404.
    let (stranger, _) = ts.register("stranger", "Sam").await;
    assert_error(
        put_reaction(&ts, &stranger, chat_id, message_id, "❤️").await,
        403,
        "not_chat_member",
    )
    .await;
    assert_error(
        put_reaction(&ts, &member, 99_999_999, message_id, "❤️").await,
        404,
        "chat_not_found",
    )
    .await;

    // An unknown message in a real chat is message_not_found…
    assert_error(
        put_reaction(&ts, &member, chat_id, 99_999_999, "❤️").await,
        404,
        "message_not_found",
    )
    .await;

    // …and so is a real message addressed through the *wrong* chat: a
    // message in the owner↔member direct chat is not reachable via the
    // family chat path.
    let response = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("direct chat response");
    let direct_chat_id = body["chat"]["id"].as_i64().expect("direct chat id");
    let response = ts
        .post_message(&owner, direct_chat_id, &Uuid::new_v4().to_string(), "psst")
        .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("message response");
    let direct_message_id = body["message"]["id"].as_i64().expect("direct message id");
    assert_error(
        put_reaction(&ts, &member, chat_id, direct_message_id, "❤️").await,
        404,
        "message_not_found",
    )
    .await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn message_pages_embed_reactions_and_distinguish_cleared_from_never_reacted() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, chat_id, message_id) = family_with_a_message(&ts).await;
    let response = ts
        .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), "second")
        .await;
    assert_eq!(response.status(), 201);

    let (seq1, _) = state_of(put_reaction(&ts, &member, chat_id, message_id, "❤️").await).await;

    let page: Value = ts
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("messages page is JSON");
    let messages = page["messages"].as_array().expect("messages array");
    let reacted = messages
        .iter()
        .find(|m| m["id"] == message_id)
        .expect("reacted message in page");
    assert_eq!(reacted["reaction_seq"], seq1);
    assert_eq!(
        reacted["reactions"],
        json!([{"user_id": member_id, "emoji": "❤️"}])
    );
    let untouched = messages
        .iter()
        .find(|m| m["id"] != message_id)
        .expect("second message in page");
    assert!(
        untouched.get("reactions").is_none() && untouched.get("reaction_seq").is_none(),
        "a never-reacted message must omit the reaction fields: {untouched}"
    );

    // After removing the only reaction the fields stay present — an empty
    // list plus a seq — so clients can tell "cleared" from "no data".
    let (seq2, _) = state_of(
        ts.delete(
            &member,
            &format!("/chats/{chat_id}/messages/{message_id}/reaction"),
        )
        .await,
    )
    .await;
    let page: Value = ts
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("messages page is JSON");
    let cleared = page["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .find(|m| m["id"] == message_id)
        .expect("cleared message in page")
        .clone();
    assert_eq!(cleared["reaction_seq"], seq2);
    assert_eq!(cleared["reactions"], json!([]));
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn reaction_catchup_pages_by_seq_and_the_chat_list_exposes_the_cursor() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, chat_id, first_message_id) = family_with_a_message(&ts).await;

    // Before any reaction the chat entry omits max_reaction_seq entirely.
    let chats: Value = ts.get(&owner, "/chats").await.json().await.expect("chats");
    let family_entry = chats["chats"]
        .as_array()
        .expect("chats array")
        .iter()
        .find(|entry| entry["chat"]["id"] == chat_id)
        .expect("family chat listed")
        .clone();
    assert!(
        family_entry.get("max_reaction_seq").is_none(),
        "max_reaction_seq must be absent before any reaction: {family_entry}"
    );

    // React to three messages → three distinct seq values, oldest first.
    let mut message_ids = vec![first_message_id];
    for body in ["two", "three"] {
        let response = ts
            .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), body)
            .await;
        assert_eq!(response.status(), 201);
        let value: Value = response.json().await.expect("message response");
        message_ids.push(value["message"]["id"].as_i64().expect("id"));
    }
    let mut seqs = Vec::new();
    for id in &message_ids {
        let (seq, _) = state_of(put_reaction(&ts, &member, chat_id, *id, "❤️").await).await;
        seqs.push(seq);
    }

    // The chat list now carries the highest seq.
    let chats: Value = ts.get(&owner, "/chats").await.json().await.expect("chats");
    let family_entry = chats["chats"]
        .as_array()
        .expect("chats array")
        .iter()
        .find(|entry| entry["chat"]["id"] == chat_id)
        .expect("family chat listed")
        .clone();
    assert_eq!(family_entry["max_reaction_seq"], seqs[2]);

    // Page 1: two oldest changes, ascending, with full state per message.
    let page: Value = ts
        .get(
            &owner,
            &format!("/chats/{chat_id}/reactions?after_seq=0&limit=2"),
        )
        .await
        .json()
        .await
        .expect("reactions page");
    let entries = page["message_reactions"].as_array().expect("entries");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["message_id"], message_ids[0]);
    assert_eq!(entries[0]["reaction_seq"], seqs[0]);
    assert_eq!(
        entries[0]["reactions"],
        json!([{"user_id": member_id, "emoji": "❤️"}])
    );
    assert_eq!(entries[1]["message_id"], message_ids[1]);

    // Page 2 from the last seen seq: the remaining change, then a short
    // (empty) page ends the loop.
    let page: Value = ts
        .get(
            &owner,
            &format!("/chats/{chat_id}/reactions?after_seq={}&limit=2", seqs[1]),
        )
        .await
        .json()
        .await
        .expect("reactions page");
    let entries = page["message_reactions"].as_array().expect("entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["message_id"], message_ids[2]);

    let page: Value = ts
        .get(
            &owner,
            &format!("/chats/{chat_id}/reactions?after_seq={}&limit=2", seqs[2]),
        )
        .await
        .json()
        .await
        .expect("reactions page");
    assert_eq!(page["message_reactions"], json!([]));

    // Replacing an emoji moves that message to the *end* of the seq order —
    // catch-up re-delivers it with the new state.
    let (replace_seq, _) =
        state_of(put_reaction(&ts, &member, chat_id, message_ids[0], "👍").await).await;
    assert!(replace_seq > seqs[2]);
    let page: Value = ts
        .get(
            &owner,
            &format!("/chats/{chat_id}/reactions?after_seq={}&limit=50", seqs[2]),
        )
        .await
        .json()
        .await
        .expect("reactions page");
    let entries = page["message_reactions"].as_array().expect("entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["message_id"], message_ids[0]);
    assert_eq!(
        entries[0]["reactions"],
        json!([{"user_id": member_id, "emoji": "👍"}])
    );

    // Guard rails: garbage pagination and outsiders.
    assert_error(
        ts.get(&owner, &format!("/chats/{chat_id}/reactions?after_seq=abc"))
            .await,
        400,
        "invalid_pagination",
    )
    .await;
    let (stranger, _) = ts.register("stranger", "Sam").await;
    assert_error(
        ts.get(&stranger, &format!("/chats/{chat_id}/reactions"))
            .await,
        403,
        "not_chat_member",
    )
    .await;
}

/// A retried `send` re-acks the ORIGINAL message, and that message is a
/// `Message` like any other — so if it has been reacted to since, it carries
/// `reactions` AND `reaction_seq`, the pair the Objects block always names
/// together.
///
/// The failure this pins is not theoretical: `client_msg_id` dedup exists
/// precisely so an offline client can replay its outbound queue, and a
/// message sitting in that queue is exactly the one somebody has had time to
/// react to. Emitting `reaction_seq` with no `reactions` key hands that
/// client a state protocol.md does not define — the seq says "reaction data
/// has moved", the absent list says "no data" — and a client applying its
/// `reaction_seq` guard would advance its cursor past reactions it was never
/// shown.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_retried_send_re_acks_a_message_with_the_reactions_it_has_since_gained() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id) = {
        let (owner, owner_id) = ts.register("owner", "Olive").await;
        let (member, member_id) = ts.register("junior", "Junior").await;
        let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
        ts.set_open_policy(&owner).await;
        ts.join(&member, &invite_code, "joined").await;
        (owner, owner_id, member, member_id)
    };
    let chat_id = ts.family_chat_id(&owner).await;

    let client_msg_id = Uuid::new_v4().to_string();
    let first = ts
        .post_message(&owner, chat_id, &client_msg_id, "Dinner at 7?")
        .await;
    assert_eq!(first.status(), 201);
    let body: Value = first.json().await.expect("message response is JSON");
    let message_id = body["message"]["id"].as_i64().expect("message id");
    // An unreacted message carries NEITHER key.
    assert!(body["message"].get("reactions").is_none());
    assert!(body["message"].get("reaction_seq").is_none());

    let (seq, _) = state_of(put_reaction(&ts, &member, chat_id, message_id, "❤️").await).await;

    // The retry: same client_msg_id, so no second message is written.
    let retry = ts
        .post_message(&owner, chat_id, &client_msg_id, "Dinner at 7?")
        .await;
    assert_eq!(
        retry.status(),
        200,
        "a retry re-acks rather than duplicating"
    );
    let body: Value = retry.json().await.expect("retry response is JSON");
    let message = &body["message"];
    assert_eq!(message["id"].as_i64(), Some(message_id));
    assert_eq!(
        message["reaction_seq"].as_i64(),
        Some(seq),
        "the re-ack reports the seq the reaction took"
    );
    assert_eq!(
        message["reactions"],
        json!([{"user_id": member_id, "emoji": "❤️"}]),
        "a re-ack that reports a reaction_seq must carry the reactions with it"
    );
}
