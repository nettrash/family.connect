//! Integration: the family board — the split authorship rule (anyone
//! moves, only the author rewrites, resizes or deletes), tombstones, note
//! sizes, and the third catch-up cursor (protocol.md, "Board").

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

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

const FRAME_WAIT: Duration = Duration::from_secs(5);

// --- Minimal WebSocket client (the size test wants to see the live frame
// --- carry the field; mirrors the helpers in ws_flow.rs).

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
    // A pong proves the server-side connection task is registered — later
    // fan-outs cannot race past a connection that already answered a frame.
    ws.send(Message::text(json!({"type": "ping"}).to_string()))
        .await
        .expect("sending ping");
    let pong = next_frame_of_type(&mut ws, "pong").await;
    assert_eq!(pong, json!({"type": "pong"}));
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

/// Family of two; returns `(owner_token, member_token)`.
async fn family_of_two(ts: &TestServer) -> (String, String) {
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    (owner, member)
}

async fn add_note(ts: &TestServer, token: &str, text: &str) -> Value {
    let response = ts
        .post(
            token,
            "/families/mine/board/notes",
            json!({"text": text, "color": "yellow", "x": 0.25, "y": 0.5}),
        )
        .await;
    assert_eq!(response.status(), 201);
    response.json().await.expect("JSON")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_note_is_added_and_the_whole_family_can_read_it() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;

    let created = add_note(&server, &owner, "Milk").await;
    let note = &created["note"];
    assert_eq!(note["text"], "Milk");
    assert_eq!(note["color"], "yellow");
    assert_eq!(note["x"].as_f64(), Some(0.25));
    assert!(note["board_seq"].as_i64().expect("seq") > 0);
    // A live note never carries the tombstone flag.
    assert!(note.get("deleted").is_none(), "got {note}");

    let board: Value = server
        .get(&member, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    let notes = board["notes"].as_array().expect("array");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["text"], "Milk");
    assert_eq!(board["max_board_seq"].as_i64(), note["board_seq"].as_i64());
}

/// The split rule, which is the whole point of the board's permissions:
/// tidying the wall is shared, rewriting someone's words is not.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn anyone_may_move_a_note_but_only_the_author_may_rewrite_it() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let note_id = add_note(&server, &owner, "Milk").await["note"]["id"]
        .as_i64()
        .expect("id");

    // A member who did not write it may move it.
    let moved = server
        .patch(
            &member,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"x": 0.9, "y": 0.1}),
        )
        .await;
    assert_eq!(moved.status(), 200);
    let body: Value = moved.json().await.expect("JSON");
    assert_eq!(body["note"]["x"].as_f64(), Some(0.9));
    assert_eq!(body["note"]["text"], "Milk");

    // …but not rewrite it, nor recolour it.
    assert_error(
        server
            .patch(
                &member,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"text": "Beer"}),
            )
            .await,
        403,
        "not_note_author",
    )
    .await;
    assert_error(
        server
            .patch(
                &member,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"color": "pink"}),
            )
            .await,
        403,
        "not_note_author",
    )
    .await;

    // The author may.
    let edited = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"text": "Oat milk", "color": "green"}),
        )
        .await;
    assert_eq!(edited.status(), 200);
    let body: Value = edited.json().await.expect("JSON");
    assert_eq!(body["note"]["text"], "Oat milk");
    assert_eq!(body["note"]["color"], "green");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_the_author_may_delete_and_deleting_is_idempotent() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let note_id = add_note(&server, &owner, "Milk").await["note"]["id"]
        .as_i64()
        .expect("id");

    assert_error(
        server
            .delete(&member, &format!("/families/mine/board/notes/{note_id}"))
            .await,
        403,
        "not_note_author",
    )
    .await;

    assert_eq!(
        server
            .delete(&owner, &format!("/families/mine/board/notes/{note_id}"))
            .await
            .status(),
        204
    );
    // Deleting it again is still a 204 and takes no new seq.
    assert_eq!(
        server
            .delete(&owner, &format!("/families/mine/board/notes/{note_id}"))
            .await
            .status(),
        204
    );

    // Gone from the board…
    let board: Value = server
        .get(&member, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    assert!(board["notes"].as_array().expect("array").is_empty());
}

/// The reason tombstones exist: a client who was offline when a note was
/// removed has no other way to learn it is gone.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_change_feed_reports_deletions_as_tombstones() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let created = add_note(&server, &owner, "Milk").await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    let after_create = created["note"]["board_seq"].as_i64().expect("seq");

    server
        .delete(&owner, &format!("/families/mine/board/notes/{note_id}"))
        .await;

    // Catch up from just after the creation: exactly one entry, and it is
    // the tombstone.
    let changes: Value = server
        .get(
            &member,
            &format!("/families/mine/board/changes?after_seq={after_create}"),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let notes = changes["notes"].as_array().expect("array");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["id"].as_i64(), Some(note_id));
    assert_eq!(notes[0]["deleted"], true);
    // A tombstone carries no content.
    assert!(notes[0].get("text").is_none(), "got {}", notes[0]);
    assert!(notes[0]["board_seq"].as_i64().expect("seq") > after_create);

    // The full-board read never returns tombstones.
    let board: Value = server
        .get(&member, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    assert!(board["notes"].as_array().expect("array").is_empty());
}

/// The feed carries each note ONCE, in the state it is now in — it is a
/// state feed, not an event log. A note created and then moved has a single
/// row carrying a single (latest) board_seq, which is precisely what lets a
/// client apply entries idempotently and in any order.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_change_feed_replays_moves_in_seq_order() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let first = add_note(&server, &owner, "Milk").await["note"]["id"]
        .as_i64()
        .expect("id");
    let second = add_note(&server, &owner, "Bread").await["note"]["id"]
        .as_i64()
        .expect("id");

    server
        .patch(
            &member,
            &format!("/families/mine/board/notes/{first}"),
            json!({"x": 0.75}),
        )
        .await;

    let changes: Value = server
        .get(&member, "/families/mine/board/changes?after_seq=0")
        .await
        .json()
        .await
        .expect("JSON");
    let notes = changes["notes"].as_array().expect("array");
    // TWO entries, not three: `first` was created and then moved, and it
    // carries only its latest seq. Oldest change first, so the moved note
    // sorts last.
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0]["id"].as_i64(), Some(second));
    assert_eq!(notes[1]["id"].as_i64(), Some(first));
    assert_eq!(notes[1]["x"].as_f64(), Some(0.75));
    let seqs: Vec<i64> = notes
        .iter()
        .map(|n| n["board_seq"].as_i64().expect("seq"))
        .collect();
    assert!(
        seqs.windows(2).all(|pair| pair[0] < pair[1]),
        "got {seqs:?}"
    );
}

/// Re-sending what a note already says takes no sequence value — otherwise
/// a client retrying a move would advance everyone's cursor for nothing.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_unchanged_patch_is_a_no_op() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;
    let created = add_note(&server, &owner, "Milk").await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    let seq = created["note"]["board_seq"].as_i64().expect("seq");

    let again: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"x": 0.25, "y": 0.5, "text": "Milk"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(again["note"]["board_seq"].as_i64(), Some(seq));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn positions_are_clamped_not_rejected() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;

    let created: Value = server
        .post(
            &owner,
            "/families/mine/board/notes",
            json!({"text": "Edge", "color": "blue", "x": 1.7, "y": -0.4}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    // A drag that ends past the edge sticks to the edge rather than failing.
    assert_eq!(created["note"]["x"].as_f64(), Some(1.0));
    assert_eq!(created["note"]["y"].as_f64(), Some(0.0));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn bad_notes_are_refused() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;

    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({"text": "   ", "color": "yellow", "x": 0.1, "y": 0.1}),
            )
            .await,
        400,
        "validation",
    )
    .await;
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({"text": "x".repeat(281), "color": "yellow", "x": 0.1, "y": 0.1}),
            )
            .await,
        400,
        "validation",
    )
    .await;
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({"text": "Milk", "color": "chartreuse", "x": 0.1, "y": 0.1}),
            )
            .await,
        400,
        "invalid_note_color",
    )
    .await;
    assert_error(
        server
            .patch(
                &owner,
                "/families/mine/board/notes/999999",
                json!({"x": 0.5}),
            )
            .await,
        404,
        "note_not_found",
    )
    .await;
}

/// A board belongs to ONE family; another family's board is invisible, and
/// its notes are not addressable from outside.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn boards_do_not_leak_between_families() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;
    let note_id = add_note(&server, &owner, "Milk").await["note"]["id"]
        .as_i64()
        .expect("id");

    let (outsider, _) = server.register("stranger", "Stranger").await;
    server.create_family(&outsider, "The Joneses").await;

    let board: Value = server
        .get(&outsider, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    assert!(board["notes"].as_array().expect("array").is_empty());

    // The other family's note is not found rather than forbidden — the
    // endpoint never confirms an id exists elsewhere.
    assert_error(
        server
            .patch(
                &outsider,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"x": 0.5}),
            )
            .await,
        404,
        "note_not_found",
    )
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn families_mine_reports_the_board_cursor() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;

    let before: Value = server
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("JSON");
    assert!(before.get("max_board_seq").is_none(), "got {before}");

    let seq = add_note(&server, &owner, "Milk").await["note"]["board_seq"]
        .as_i64()
        .expect("seq");

    let after: Value = server
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(after["max_board_seq"].as_i64(), Some(seq));
}

/// A note created without a size is `medium` — the size every note had
/// before the field existed — and a chosen size survives every path a
/// client reads notes through: the creation reply, the full board, the
/// change feed and the live frame.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_note_without_a_size_is_medium_and_a_chosen_size_is_kept_everywhere() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let plain = add_note(&server, &owner, "Milk").await;
    assert_eq!(plain["note"]["size"], "medium", "got {plain}");
    let plain_frame = next_frame_of_type(&mut member_ws, "board_note").await;
    assert_eq!(plain_frame["note"]["size"], "medium", "got {plain_frame}");

    let response = server
        .post(
            &owner,
            "/families/mine/board/notes",
            json!({"text": "DENTIST 9AM", "color": "pink", "size": "large", "x": 0.5, "y": 0.5}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let large: Value = response.json().await.expect("JSON");
    assert_eq!(large["note"]["size"], "large", "got {large}");
    let large_id = large["note"]["id"].as_i64().expect("id");
    let large_frame = next_frame_of_type(&mut member_ws, "board_note").await;
    assert_eq!(large_frame["note"]["id"].as_i64(), Some(large_id));
    assert_eq!(large_frame["note"]["size"], "large", "got {large_frame}");

    let size_of = |notes: &Value, id: i64| -> Value {
        notes
            .as_array()
            .expect("array")
            .iter()
            .find(|n| n["id"].as_i64() == Some(id))
            .unwrap_or_else(|| panic!("note {id} missing from {notes}"))["size"]
            .clone()
    };
    let plain_id = plain["note"]["id"].as_i64().expect("id");

    let board: Value = server
        .get(&member, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(size_of(&board["notes"], plain_id), "medium");
    assert_eq!(size_of(&board["notes"], large_id), "large");

    let changes: Value = server
        .get(&member, "/families/mine/board/changes?after_seq=0")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(size_of(&changes["notes"], plain_id), "medium");
    assert_eq!(size_of(&changes["notes"], large_id), "large");
}

/// Size belongs to the author with text and colour: a size anyone could
/// change is a size anyone could shrink to nothing. Moving stays shared,
/// and a resize is a mutation like any other — it takes a new seq.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn anyone_may_move_a_note_but_only_the_author_may_resize_it() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let created = add_note(&server, &owner, "Milk").await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    let created_seq = created["note"]["board_seq"].as_i64().expect("seq");

    assert_error(
        server
            .patch(
                &member,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"size": "large"}),
            )
            .await,
        403,
        "not_note_author",
    )
    .await;

    // The same member may still move it — position is everyone's.
    let moved = server
        .patch(
            &member,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"x": 0.9, "y": 0.1}),
        )
        .await;
    assert_eq!(moved.status(), 200);
    let moved: Value = moved.json().await.expect("JSON");
    assert_eq!(moved["note"]["size"], "medium");
    let moved_seq = moved["note"]["board_seq"].as_i64().expect("seq");
    assert!(moved_seq > created_seq);

    let resized = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"size": "small"}),
        )
        .await;
    assert_eq!(resized.status(), 200);
    let resized: Value = resized.json().await.expect("JSON");
    assert_eq!(resized["note"]["size"], "small");
    // Everything else is left as it was: a resize touches only the size.
    assert_eq!(resized["note"]["text"], "Milk");
    assert_eq!(resized["note"]["x"].as_f64(), Some(0.9));
    assert!(
        resized["note"]["board_seq"].as_i64().expect("seq") > moved_seq,
        "a resize takes a new board_seq"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn bad_sizes_are_refused() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;

    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({"text": "Milk", "color": "yellow", "size": "huge", "x": 0.1, "y": 0.1}),
            )
            .await,
        400,
        "invalid_note_size",
    )
    .await;

    let note_id = add_note(&server, &owner, "Milk").await["note"]["id"]
        .as_i64()
        .expect("id");
    assert_error(
        server
            .patch(
                &owner,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"size": "huge"}),
            )
            .await,
        400,
        "invalid_note_size",
    )
    .await;
}

/// Same rule as re-sending a note's text: the size it already has takes
/// no sequence value, so a client retrying a resize advances nobody's
/// cursor.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn re_sending_the_current_size_is_a_no_op() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;
    let created = add_note(&server, &owner, "Milk").await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    let seq = created["note"]["board_seq"].as_i64().expect("seq");

    let again: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"size": "medium"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(again["note"]["size"], "medium");
    assert_eq!(again["note"]["board_seq"].as_i64(), Some(seq));
}

// --- content_seq: the seq a BADGE counts (protocol.md, "Board") ---------
//
// `board_seq` answers "what has changed?" and therefore MUST move for a
// drag — the change feed carries moves, or a move on one device never
// reaches another. `content_seq` answers "is there anything to READ?", and
// tidying a wall does not put anything on it.

/// The whole of issue #53 in one test: dragging, resizing and recolouring
/// all take a new `board_seq` and all leave `content_seq` exactly where it
/// was, so no client can badge them.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn geometry_and_colour_take_a_board_seq_but_never_a_content_seq() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let created = add_note(&server, &owner, "Milk").await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    let created_seq = created["note"]["board_seq"].as_i64().expect("seq");
    // A new note IS new text: both stamps start together.
    assert_eq!(created["note"]["content_seq"].as_i64(), Some(created_seq));

    // A move — by somebody who is not the author, which is the ordinary
    // case and the one that used to badge a whole family.
    let moved: Value = server
        .patch(
            &member,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"x": 0.9, "y": 0.1}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert!(moved["note"]["board_seq"].as_i64().expect("seq") > created_seq);
    assert_eq!(moved["note"]["content_seq"].as_i64(), Some(created_seq));

    // A resize, by the author (only they may).
    let resized: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"size": "large"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(resized["note"]["size"], "large");
    assert!(
        resized["note"]["board_seq"].as_i64().expect("seq")
            > moved["note"]["board_seq"].as_i64().expect("seq")
    );
    assert_eq!(resized["note"]["content_seq"].as_i64(), Some(created_seq));

    // A recolour: the author chose it, but the note still says what it said.
    let recoloured: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"color": "pink"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(recoloured["note"]["color"], "pink");
    assert!(
        recoloured["note"]["board_seq"].as_i64().expect("seq")
            > resized["note"]["board_seq"].as_i64().expect("seq")
    );
    assert_eq!(
        recoloured["note"]["content_seq"].as_i64(),
        Some(created_seq)
    );

    // And the whole-board read agrees with the PATCH answers.
    let board: Value = server
        .get(&member, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        board["notes"][0]["content_seq"].as_i64(),
        Some(created_seq),
        "got {}",
        board["notes"][0]
    );
}

/// A rewrite moves BOTH stamps: new words are the one board change worth
/// telling somebody about.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_rewrite_moves_the_content_seq() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;
    let created = add_note(&server, &owner, "Milk").await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    let created_seq = created["note"]["board_seq"].as_i64().expect("seq");

    let edited: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"text": "Oat milk"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let board_seq = edited["note"]["board_seq"].as_i64().expect("seq");
    assert!(board_seq > created_seq);
    assert_eq!(edited["note"]["content_seq"].as_i64(), Some(board_seq));

    // A rewrite that lands in the same request as a move still counts as a
    // rewrite: which fields are PRESENT is what the wire means by an edit.
    let both: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"text": "Oat milk, 2 cartons", "x": 0.4}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        both["note"]["content_seq"].as_i64(),
        both["note"]["board_seq"].as_i64()
    );
}

/// Why it is a stamp on the note and not a flag on the change entry: the
/// feed COLLAPSES. A note edited and then dragged five times appears once,
/// and the one entry still has to say that its words changed.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_change_feed_keeps_the_content_seq_through_later_moves() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let created = add_note(&server, &owner, "Milk").await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    // Where a client that has already seen this note stands.
    let seen_through = created["note"]["board_seq"].as_i64().expect("seq");

    let edited: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"text": "Oat milk"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let edit_seq = edited["note"]["content_seq"].as_i64().expect("seq");

    // …and then five drags by anybody, which is what a family does to a
    // wall.
    for step in 1..=5 {
        server
            .patch(
                &member,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"x": 0.1 * f64::from(step)}),
            )
            .await;
    }

    let changes: Value = server
        .get(
            &member,
            &format!("/families/mine/board/changes?after_seq={seen_through}"),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let notes = changes["notes"].as_array().expect("array");
    assert_eq!(notes.len(), 1, "the feed carries a note once: {notes:?}");
    // The single collapsed entry still reports the EDIT, which a
    // "this change was only a move" flag on the last event could not.
    assert_eq!(notes[0]["content_seq"].as_i64(), Some(edit_seq));
    assert!(notes[0]["board_seq"].as_i64().expect("seq") > edit_seq);
}

/// A note that has only ever been dragged is not worth a badge on any
/// client, however far its `board_seq` has travelled.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_dragged_note_stays_below_the_mark_a_reader_already_had() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    // Two notes, and a reader who has seen both: their mark is the highest
    // content_seq on the board.
    let first = add_note(&server, &owner, "Milk").await;
    let second = add_note(&server, &owner, "Bread").await;
    let mark = second["note"]["content_seq"].as_i64().expect("seq");
    let first_id = first["note"]["id"].as_i64().expect("id");

    for step in 1..=3 {
        server
            .patch(
                &member,
                &format!("/families/mine/board/notes/{first_id}"),
                json!({"x": 0.2 * f64::from(step), "y": 0.3}),
            )
            .await;
    }

    let board: Value = server
        .get(&member, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    let unread = board["notes"]
        .as_array()
        .expect("array")
        .iter()
        .filter(|note| note["content_seq"].as_i64().expect("seq") > mark)
        .count();
    assert_eq!(unread, 0, "a tidied wall badges nobody: {board}");
    // …while the cursor a SYNC uses has moved on, which is the whole
    // tension: the move must still travel.
    assert!(board["max_board_seq"].as_i64().expect("seq") > mark);
}

/// A tombstone carries no `content_seq`, for the same reason it carries no
/// text: there is nothing left to read.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_tombstone_carries_no_content_seq() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let created = add_note(&server, &owner, "Milk").await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    let after_create = created["note"]["board_seq"].as_i64().expect("seq");

    server
        .delete(&owner, &format!("/families/mine/board/notes/{note_id}"))
        .await;

    let changes: Value = server
        .get(
            &member,
            &format!("/families/mine/board/changes?after_seq={after_create}"),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let tombstone = &changes["notes"][0];
    assert_eq!(tombstone["deleted"], true);
    assert!(tombstone.get("content_seq").is_none(), "got {tombstone}");
}

/// The live WS frame carries it too — a client applying a frame and a
/// client catching up must reach the same badge.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_board_note_frame_carries_the_content_seq() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let mut ws = connect_ws(&server, &member).await;

    let created = add_note(&server, &owner, "Milk").await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    let created_seq = created["note"]["board_seq"].as_i64().expect("seq");
    let frame = next_frame_of_type(&mut ws, "board_note").await;
    assert_eq!(frame["note"]["content_seq"].as_i64(), Some(created_seq));

    server
        .patch(
            &member,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"x": 0.9}),
        )
        .await;
    let frame = next_frame_of_type(&mut ws, "board_note").await;
    assert_eq!(frame["note"]["x"].as_f64(), Some(0.9));
    assert!(frame["note"]["board_seq"].as_i64().expect("seq") > created_seq);
    assert_eq!(frame["note"]["content_seq"].as_i64(), Some(created_seq));
}

/// A note created without a font is `plain` — the face every note had
/// before the field existed — and a chosen font survives every path a
/// client reads notes through, exactly as a size does.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_note_without_a_font_is_plain_and_a_chosen_font_is_kept_everywhere() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let plain = add_note(&server, &owner, "Milk").await;
    assert_eq!(plain["note"]["font"], "plain", "got {plain}");
    let plain_frame = next_frame_of_type(&mut member_ws, "board_note").await;
    assert_eq!(plain_frame["note"]["font"], "plain", "got {plain_frame}");
    let plain_id = plain["note"]["id"].as_i64().expect("id");

    let response = server
        .post(
            &owner,
            "/families/mine/board/notes",
            json!({"text": "Happy birthday!", "color": "pink", "font": "casual",
                   "x": 0.5, "y": 0.5}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let casual: Value = response.json().await.expect("JSON");
    assert_eq!(casual["note"]["font"], "casual", "got {casual}");
    let casual_id = casual["note"]["id"].as_i64().expect("id");
    let casual_frame = next_frame_of_type(&mut member_ws, "board_note").await;
    assert_eq!(casual_frame["note"]["id"].as_i64(), Some(casual_id));
    assert_eq!(casual_frame["note"]["font"], "casual", "got {casual_frame}");

    let font_of = |notes: &Value, id: i64| -> Value {
        notes
            .as_array()
            .expect("array")
            .iter()
            .find(|n| n["id"].as_i64() == Some(id))
            .unwrap_or_else(|| panic!("note {id} missing from {notes}"))["font"]
            .clone()
    };

    let board: Value = server
        .get(&member, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(font_of(&board["notes"], plain_id), "plain");
    assert_eq!(font_of(&board["notes"], casual_id), "casual");

    let changes: Value = server
        .get(&member, "/families/mine/board/changes?after_seq=0")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(font_of(&changes["notes"], plain_id), "plain");
    assert_eq!(font_of(&changes["notes"], casual_id), "casual");
}

/// The font belongs to the author with text, colour and size — and a change
/// of face is not a change of what the note SAYS, so it takes a new
/// `board_seq` and leaves `content_seq` exactly where it was. A badge that
/// went off because somebody chose a nicer hand would be a lie about there
/// being something to read (protocol.md, "Board").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_the_author_may_change_the_font_and_it_raises_no_badge() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let created = add_note(&server, &owner, "Milk").await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    let created_seq = created["note"]["board_seq"].as_i64().expect("seq");
    let content_seq = created["note"]["content_seq"]
        .as_i64()
        .expect("content seq");

    assert_error(
        server
            .patch(
                &member,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"font": "serif"}),
            )
            .await,
        403,
        "not_note_author",
    )
    .await;

    let restyled = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"font": "serif"}),
        )
        .await;
    assert_eq!(restyled.status(), 200);
    let restyled: Value = restyled.json().await.expect("JSON");
    assert_eq!(restyled["note"]["font"], "serif");
    // Everything else is left as it was.
    assert_eq!(restyled["note"]["text"], "Milk");
    assert_eq!(restyled["note"]["size"], "medium");
    assert_eq!(restyled["note"]["color"], "yellow");
    assert!(
        restyled["note"]["board_seq"].as_i64().expect("seq") > created_seq,
        "a change of font is a mutation and takes a new board_seq"
    );
    assert_eq!(
        restyled["note"]["content_seq"].as_i64(),
        Some(content_seq),
        "the face is not what the note says: no badge"
    );

    // And re-sending the font it already has is a no-op, like every other
    // field: no new seq, and nothing fanned out.
    let again = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"font": "serif"}),
        )
        .await;
    assert_eq!(again.status(), 200);
    let again: Value = again.json().await.expect("JSON");
    assert_eq!(
        again["note"]["board_seq"].as_i64(),
        restyled["note"]["board_seq"].as_i64(),
        "a no-op takes no sequence value"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn bad_fonts_are_refused() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;

    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({"text": "Milk", "color": "yellow", "font": "comic-sans",
                       "x": 0.1, "y": 0.1}),
            )
            .await,
        400,
        "invalid_note_font",
    )
    .await;

    let created = add_note(&server, &owner, "Milk").await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    assert_error(
        server
            .patch(
                &owner,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"font": "Helvetica"}),
            )
            .await,
        400,
        "invalid_note_font",
    )
    .await;
}

// --- Photo notes (protocol.md, "Board") -------------------------------------

fn jpeg_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(len.max(4), 0x00);
    bytes
}

/// ISO base media: "ftyp" at offset 4, which is what the server sniffs an
/// m4a by.
fn mp4_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0x00, 0x00, 0x00, 0x18];
    bytes.extend_from_slice(b"ftypmp42");
    bytes.resize(len.max(12), 0x00);
    bytes
}

/// Upload one picture and answer with its id.
async fn upload_photo(server: &TestServer, token: &str) -> i64 {
    let response = server
        .put_bytes_method(
            "POST",
            token,
            "/attachments?kind=photo&width=1600&height=1200",
            "image/jpeg",
            jpeg_bytes(4096),
        )
        .await;
    assert_eq!(response.status(), 201, "uploading a picture");
    let body: Value = response.json().await.expect("JSON");
    body["attachment"]["id"].as_i64().expect("attachment id")
}

async fn pin_photo(
    server: &TestServer,
    token: &str,
    attachment_id: i64,
    caption: &str,
) -> reqwest::Response {
    server
        .post(
            token,
            "/families/mine/board/notes",
            json!({"text": caption, "color": "yellow", "kind": "photo",
                   "attachment_id": attachment_id, "x": 0.3, "y": 0.4}),
        )
        .await
}

/// A picture pinned to the wall is a NOTE: it takes a slot, it rides the
/// same feed and the same seq, anyone may move it, and every member can
/// FETCH IT — which is the half no chat membership could ever grant.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_photo_note_is_pinned_read_and_fetched_by_the_whole_family() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let attachment_id = upload_photo(&server, &owner).await;
    let response = pin_photo(&server, &owner, attachment_id, "The lake").await;
    assert_eq!(response.status(), 201, "{:?}", response.text().await);
    let pinned: Value = response.json().await.expect("JSON");
    let note = &pinned["note"];
    let note_id = note["id"].as_i64().expect("id");
    assert_eq!(note["kind"], "photo");
    assert_eq!(note["text"], "The lake", "the caption is the note's text");
    assert_eq!(note["attachment"]["id"].as_i64(), Some(attachment_id));
    assert_eq!(note["attachment"]["kind"], "photo");

    // The live frame carries the picture too, or the wall draws a blank.
    let frame = next_frame_of_type(&mut member_ws, "board_note").await;
    assert_eq!(frame["note"]["kind"], "photo", "got {frame}");
    assert_eq!(
        frame["note"]["attachment"]["id"].as_i64(),
        Some(attachment_id),
        "got {frame}"
    );

    // And so do both reads.
    for path in [
        "/families/mine/board",
        "/families/mine/board/changes?after_seq=0",
    ] {
        let body: Value = server.get(&member, path).await.json().await.expect("JSON");
        let found = body["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .find(|n| n["id"].as_i64() == Some(note_id))
            .unwrap_or_else(|| panic!("the note is missing from {path}"));
        assert_eq!(
            found["attachment"]["id"].as_i64(),
            Some(attachment_id),
            "{path}"
        );
    }

    // THE BYTES. A board note belongs to no chat, so chat membership can
    // never grant this — the family does.
    let bytes = server
        .get(&member, &format!("/attachments/{attachment_id}"))
        .await;
    assert_eq!(
        bytes.status(),
        200,
        "every member can fetch a pinned picture"
    );

    // A stranger cannot.
    let (stranger, _) = server.register("stranger", "Sam").await;
    server.create_family(&stranger, "The Joneses").await;
    assert_eq!(
        server
            .get(&stranger, &format!("/attachments/{attachment_id}"))
            .await
            .status(),
        404,
        "another family's wall is not readable"
    );
}

/// A picture may be pinned without a caption, and moving it must not lose
/// it: the frame IS the note, so a move fanned out without the attachment
/// would blank the picture on every other device.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_caption_less_photo_survives_a_move_by_anyone() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;

    let attachment_id = upload_photo(&server, &owner).await;
    let pinned: Value = pin_photo(&server, &owner, attachment_id, "")
        .await
        .json()
        .await
        .expect("JSON");
    let note_id = pinned["note"]["id"].as_i64().expect("id");
    assert_eq!(pinned["note"]["text"], "", "a picture needs no caption");

    // Position is everyone's, even on somebody else's picture.
    let moved = server
        .patch(
            &member,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"x": 0.9, "y": 0.1}),
        )
        .await;
    assert_eq!(moved.status(), 200);
    let moved: Value = moved.json().await.expect("JSON");
    assert_eq!(
        moved["note"]["attachment"]["id"].as_i64(),
        Some(attachment_id),
        "a move must not lose the picture: {moved}"
    );
    assert_eq!(moved["note"]["kind"], "photo");

    // The author may empty a caption that exists — a text note may not.
    let captioned = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"text": "  "}),
        )
        .await;
    assert_eq!(captioned.status(), 200);
    let text_note = add_note(&server, &owner, "Milk").await;
    let text_id = text_note["note"]["id"].as_i64().expect("id");
    assert_error(
        server
            .patch(
                &owner,
                &format!("/families/mine/board/notes/{text_id}"),
                json!({"text": "   "}),
            )
            .await,
        400,
        "validation",
    )
    .await;
}

/// The kind and the picture arrive together or not at all, a wall pins
/// PICTURES, and one picture is pinned by one thing.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_ways_of_pinning_a_picture_wrong() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;

    // A kind with no picture, and a picture with no kind.
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({"text": "hi", "color": "yellow", "kind": "photo", "x": 0.1, "y": 0.1}),
            )
            .await,
        400,
        "validation",
    )
    .await;
    let attachment_id = upload_photo(&server, &owner).await;
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({"text": "hi", "color": "yellow", "attachment_id": attachment_id,
                       "x": 0.1, "y": 0.1}),
            )
            .await,
        400,
        "validation",
    )
    .await;
    // An unknown kind.
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({"text": "hi", "color": "yellow", "kind": "video", "x": 0.1, "y": 0.1}),
            )
            .await,
        400,
        "invalid_note_kind",
    )
    .await;

    // A wall pins pictures: a voice message is a thing to play.
    let audio_response = server
        .put_bytes_method(
            "POST",
            &owner,
            "/attachments?kind=audio&duration_ms=1200",
            "audio/mp4",
            mp4_bytes(2048),
        )
        .await;
    assert_eq!(audio_response.status(), 201, "uploading a voice message");
    let audio: Value = audio_response.json().await.expect("JSON");
    let audio_id = audio["attachment"]["id"].as_i64().expect("id");
    assert_error(
        pin_photo(&server, &owner, audio_id, "listen").await,
        400,
        "invalid_attachment",
    )
    .await;

    // One picture, one note.
    assert_eq!(
        pin_photo(&server, &owner, attachment_id, "once")
            .await
            .status(),
        201
    );
    assert_error(
        pin_photo(&server, &owner, attachment_id, "twice").await,
        409,
        "attachment_already_used",
    )
    .await;

    // And somebody else's upload is not yours to pin.
    let (member, _) = server.register("cousin", "Cousin").await;
    let theirs = upload_photo(&server, &owner).await;
    assert_error(
        pin_photo(&server, &member, theirs, "mine now").await,
        409,
        "not_in_family",
    )
    .await;
}

/// Deleting a photo note takes its picture with it: the tombstone carries
/// no content, so nothing left could ever show it.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn deleting_a_photo_note_removes_its_picture() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;

    let attachment_id = upload_photo(&server, &owner).await;
    let pinned: Value = pin_photo(&server, &owner, attachment_id, "The lake")
        .await
        .json()
        .await
        .expect("JSON");
    let note_id = pinned["note"]["id"].as_i64().expect("id");
    assert_eq!(
        server
            .get(&member, &format!("/attachments/{attachment_id}"))
            .await
            .status(),
        200
    );

    let deleted = server
        .delete(&owner, &format!("/families/mine/board/notes/{note_id}"))
        .await;
    assert_eq!(deleted.status(), 204);

    assert_eq!(
        server
            .get(&member, &format!("/attachments/{attachment_id}"))
            .await
            .status(),
        404,
        "the picture goes with the note"
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM attachments WHERE id = $1")
        .bind(attachment_id)
        .fetch_one(&server.state.pool)
        .await
        .expect("counting the attachment row");
    assert_eq!(rows, 0, "the row goes too, not just the access");
}

/// The sweeper's predicate is the trap: a pinned picture has no
/// `message_id` and is NOT unclaimed. Sweeping on the message alone would
/// eat every photo note's picture hours after it was pinned.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_unclaimed_sweep_leaves_a_pinned_picture_alone() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;

    let pinned_id = upload_photo(&server, &owner).await;
    assert_eq!(
        pin_photo(&server, &owner, pinned_id, "The lake")
            .await
            .status(),
        201
    );
    let loose_id = upload_photo(&server, &owner).await;

    // Age both uploads past the grace period.
    sqlx::query("UPDATE attachments SET created_at = now() - interval '30 days'")
        .execute(&server.state.pool)
        .await
        .expect("ageing the uploads");

    let swept = family_connect::handlers_attachment::sweep_unclaimed(&server.state)
        .await
        .expect("the sweep runs");
    assert_eq!(swept, 1, "the loose upload goes and the pinned one stays");

    let pinned_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM attachments WHERE id = $1")
        .bind(pinned_id)
        .fetch_one(&server.state.pool)
        .await
        .expect("counting");
    assert_eq!(pinned_rows, 1, "a pinned picture is not unclaimed");
    let loose_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM attachments WHERE id = $1")
        .bind(loose_id)
        .fetch_one(&server.state.pool)
        .await
        .expect("counting");
    assert_eq!(loose_rows, 0);
}

// --- Events (protocol.md, "Board") ------------------------------------------

async fn pin_event(
    server: &TestServer,
    token: &str,
    title: &str,
    extra: Value,
) -> reqwest::Response {
    let mut body = json!({
        "text": title, "color": "blue", "kind": "event",
        "starts_at": "2026-12-24T17:00:00Z", "x": 0.3, "y": 0.4,
    });
    if let Some(extra) = extra.as_object() {
        for (key, value) in extra {
            body[key] = value.clone();
        }
    }
    server.post(token, "/families/mine/board/notes", body).await
}

/// An event is a NOTE with a when, a where and a list of who is coming —
/// and it is born with `rsvps: []`, because "nobody has answered yet" is an
/// answer and a missing field is not.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_event_carries_its_times_its_place_and_an_empty_guest_list() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let response = pin_event(
        &server,
        &owner,
        "Christmas dinner",
        json!({"ends_at": "2026-12-24T21:00:00Z", "place": "  Gran's house  "}),
    )
    .await;
    assert_eq!(response.status(), 201, "{:?}", response.text().await);
    let created: Value = response.json().await.expect("JSON");
    let note = &created["note"];
    let note_id = note["id"].as_i64().expect("id");
    assert_eq!(note["kind"], "event");
    assert_eq!(
        note["text"], "Christmas dinner",
        "the title is the note's text"
    );
    assert_eq!(
        note["place"], "Gran's house",
        "trimmed, like every other text"
    );
    assert!(
        note["starts_at"]
            .as_str()
            .is_some_and(|s| s.starts_with("2026-12-24T17:00"))
    );
    assert_eq!(
        note["rsvps"].as_array().map(Vec::len),
        Some(0),
        "born with nobody answering, and that is [] not absent: {note}"
    );

    // The live frame and both reads carry it all.
    let frame = next_frame_of_type(&mut member_ws, "board_note").await;
    assert_eq!(frame["note"]["kind"], "event", "got {frame}");
    assert_eq!(frame["note"]["place"], "Gran's house");
    for path in [
        "/families/mine/board",
        "/families/mine/board/changes?after_seq=0",
    ] {
        let body: Value = server.get(&member, path).await.json().await.expect("JSON");
        let found = body["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .find(|n| n["id"].as_i64() == Some(note_id))
            .unwrap_or_else(|| panic!("the event is missing from {path}"));
        assert_eq!(found["rsvps"].as_array().map(Vec::len), Some(0), "{path}");
        assert!(found["ends_at"].as_str().is_some(), "{path}");
    }

    // A text note carries none of it.
    let plain = add_note(&server, &owner, "Milk").await;
    assert!(plain["note"]["starts_at"].is_null(), "{plain}");
    assert!(
        plain["note"]["rsvps"].is_null(),
        "only an event has a guest list"
    );
}

/// ANSWERING IS NOT AUTHORSHIP: any member may say they are coming, one
/// answer each, replaced rather than added to, and retractable. It takes a
/// `board_seq` so the other devices hear about it, and leaves `content_seq`
/// alone because the event says exactly what it said before.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn anyone_may_answer_an_event_once_and_change_their_mind() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let created: Value = pin_event(&server, &owner, "Christmas dinner", json!({}))
        .await
        .json()
        .await
        .expect("JSON");
    let note_id = created["note"]["id"].as_i64().expect("id");
    let created_seq = created["note"]["board_seq"].as_i64().expect("seq");
    let content_seq = created["note"]["content_seq"]
        .as_i64()
        .expect("content seq");
    let member_id = server.user_id(&member).await;

    // Somebody who did NOT write it answers.
    let answered = server
        .put(
            &member,
            &format!("/families/mine/board/notes/{note_id}/rsvp"),
            json!({"answer": "going"}),
        )
        .await;
    assert_eq!(answered.status(), 200, "{:?}", answered.text().await);
    let answered: Value = answered.json().await.expect("JSON");
    assert_eq!(
        answered["note"]["rsvps"],
        json!([{"user_id": member_id, "answer": "going"}])
    );
    let answered_seq = answered["note"]["board_seq"].as_i64().expect("seq");
    assert!(
        answered_seq > created_seq,
        "an answer reaches the other devices"
    );
    assert_eq!(
        answered["note"]["content_seq"].as_i64(),
        Some(content_seq),
        "an answer is not something new to READ: no badge"
    );

    // The same answer again is a no-op.
    let again: Value = server
        .put(
            &member,
            &format!("/families/mine/board/notes/{note_id}/rsvp"),
            json!({"answer": "going"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(again["note"]["board_seq"].as_i64(), Some(answered_seq));

    // Changing your mind REPLACES rather than adds.
    let changed: Value = server
        .put(
            &member,
            &format!("/families/mine/board/notes/{note_id}/rsvp"),
            json!({"answer": "maybe"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        changed["note"]["rsvps"],
        json!([{"user_id": member_id, "answer": "maybe"}])
    );

    // And retracting leaves nobody — idempotently.
    let retracted: Value = server
        .delete(
            &member,
            &format!("/families/mine/board/notes/{note_id}/rsvp"),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(retracted["note"]["rsvps"].as_array().map(Vec::len), Some(0));
    let retracted_seq = retracted["note"]["board_seq"].as_i64().expect("seq");
    let nothing: Value = server
        .delete(
            &member,
            &format!("/families/mine/board/notes/{note_id}/rsvp"),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        nothing["note"]["board_seq"].as_i64(),
        Some(retracted_seq),
        "retracting nothing burns no seq"
    );
}

/// The author owns WHEN and WHERE, as they own the title — and the three
/// are refused on any other kind, at creation and at edit.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_the_author_may_move_an_event_in_time() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let created: Value = pin_event(
        &server,
        &owner,
        "Christmas dinner",
        json!({"ends_at": "2026-12-24T21:00:00Z", "place": "Gran's house"}),
    )
    .await
    .json()
    .await
    .expect("JSON");
    let note_id = created["note"]["id"].as_i64().expect("id");

    assert_error(
        server
            .patch(
                &member,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"starts_at": "2026-12-25T17:00:00Z"}),
            )
            .await,
        403,
        "not_note_author",
    )
    .await;

    // The author may. An empty place CLEARS it; a null ends_at clears that.
    let edited = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"starts_at": "2026-12-25T18:00:00Z", "place": "", "ends_at": null}),
        )
        .await;
    assert_eq!(edited.status(), 200, "{:?}", edited.text().await);
    let edited: Value = edited.json().await.expect("JSON");
    assert!(
        edited["note"]["starts_at"]
            .as_str()
            .is_some_and(|s| s.starts_with("2026-12-25T18:00"))
    );
    assert!(
        edited["note"]["place"].is_null(),
        "an emptied place is no place"
    );
    assert!(edited["note"]["ends_at"].is_null(), "a null clears it");

    // An end before the start, checked against the STORED start.
    assert_error(
        server
            .patch(
                &owner,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"ends_at": "2026-12-25T17:00:00Z"}),
            )
            .await,
        400,
        "validation",
    )
    .await;

    // None of the three belongs on a text note.
    let plain = add_note(&server, &owner, "Milk").await;
    let plain_id = plain["note"]["id"].as_i64().expect("id");
    assert_error(
        server
            .patch(
                &owner,
                &format!("/families/mine/board/notes/{plain_id}"),
                json!({"place": "nowhere"}),
            )
            .await,
        400,
        "validation",
    )
    .await;
    assert_error(
        server
            .put(
                &owner,
                &format!("/families/mine/board/notes/{plain_id}/rsvp"),
                json!({"answer": "going"}),
            )
            .await,
        400,
        "invalid_rsvp",
    )
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_ways_of_pinning_an_event_wrong() {
    let server = spawn_server().await;
    let (owner, _) = family_of_two(&server).await;

    // No starts_at.
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({"text": "Dinner", "color": "blue", "kind": "event", "x": 0.1, "y": 0.1}),
            )
            .await,
        400,
        "validation",
    )
    .await;
    // Unparseable, and an end before the start.
    assert_error(
        pin_event(
            &server,
            &owner,
            "Dinner",
            json!({"starts_at": "christmas eve"}),
        )
        .await,
        400,
        "validation",
    )
    .await;
    assert_error(
        pin_event(
            &server,
            &owner,
            "Dinner",
            json!({"ends_at": "2026-12-24T16:00:00Z"}),
        )
        .await,
        400,
        "validation",
    )
    .await;
    // A place longer than a line on a card.
    assert_error(
        pin_event(&server, &owner, "Dinner", json!({"place": "x".repeat(201)})).await,
        400,
        "validation",
    )
    .await;
    // An event still needs a title: only a PICTURE may say nothing.
    assert_error(
        pin_event(&server, &owner, "   ", json!({})).await,
        400,
        "validation",
    )
    .await;
    // And the three are refused on a text note at creation too.
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({"text": "Milk", "color": "yellow", "place": "Gran's",
                       "x": 0.1, "y": 0.1}),
            )
            .await,
        400,
        "validation",
    )
    .await;
    // A bad answer on a real event.
    let created: Value = pin_event(&server, &owner, "Dinner", json!({}))
        .await
        .json()
        .await
        .expect("JSON");
    let note_id = created["note"]["id"].as_i64().expect("id");
    assert_error(
        server
            .put(
                &owner,
                &format!("/families/mine/board/notes/{note_id}/rsvp"),
                json!({"answer": "perhaps"}),
            )
            .await,
        400,
        "invalid_rsvp",
    )
    .await;
}

// --- Whose picture it is (protocol.md, "Attachments", "Deleting an account") --

/// An event's backdrop is read back EVERYWHERE, not only in the answer to
/// the create: the board, the change feed, a move and an RSVP. Reading it
/// back for photo notes alone lost it on every device the moment anything
/// but its creation was read.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_events_backdrop_is_read_back_everywhere() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;

    let backdrop = upload_photo(&server, &owner).await;
    let response = pin_event(
        &server,
        &owner,
        "Christmas dinner",
        json!({"attachment_id": backdrop}),
    )
    .await;
    assert_eq!(response.status(), 201, "{:?}", response.text().await);
    let created: Value = response.json().await.expect("JSON");
    let note_id = created["note"]["id"].as_i64().expect("id");
    assert_eq!(created["note"]["attachment"]["id"].as_i64(), Some(backdrop));

    for path in [
        "/families/mine/board",
        "/families/mine/board/changes?after_seq=0",
    ] {
        let body: Value = server.get(&member, path).await.json().await.expect("JSON");
        let found = body["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .find(|n| n["id"].as_i64() == Some(note_id))
            .unwrap_or_else(|| panic!("the event is missing from {path}"));
        assert_eq!(
            found["attachment"]["id"].as_i64(),
            Some(backdrop),
            "{path} lost the backdrop: {found}"
        );
    }

    let moved: Value = server
        .patch(
            &member,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"x": 0.7, "y": 0.2}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        moved["note"]["attachment"]["id"].as_i64(),
        Some(backdrop),
        "{moved}"
    );

    let answered: Value = server
        .put(
            &member,
            &format!("/families/mine/board/notes/{note_id}/rsvp"),
            json!({"answer": "going"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        answered["note"]["attachment"]["id"].as_i64(),
        Some(backdrop),
        "{answered}"
    );
}

/// A picture pinned to the board is TAKEN: a message naming it is refused
/// exactly as one naming another message's picture is, and the note keeps it.
/// Two owners for one upload would let deleting either take the other's.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_pinned_picture_cannot_also_go_on_a_message() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;
    let attachment_id = upload_photo(&server, &owner).await;
    assert_eq!(
        pin_photo(&server, &owner, attachment_id, "The lake")
            .await
            .status(),
        201
    );

    let chat_id = server.family_chat_id(&owner).await;
    assert_error(
        server
            .post(
                &owner,
                &format!("/chats/{chat_id}/messages"),
                json!({"client_msg_id": "7d3e9a61-2b4c-4f1e-9a8d-5c6b7e8f9a01",
                       "body": "", "attachment_ids": [attachment_id]}),
            )
            .await,
        409,
        "attachment_already_used",
    )
    .await;

    let message_id: Option<i64> =
        sqlx::query_scalar("SELECT message_id FROM attachments WHERE id = $1")
            .bind(attachment_id)
            .fetch_one(&server.state.pool)
            .await
            .expect("reading the attachment");
    assert_eq!(message_id, None, "the refused message took nothing");
    assert_eq!(
        server
            .get(&member, &format!("/attachments/{attachment_id}"))
            .await
            .status(),
        200,
        "the note still shows it"
    );
}

/// THE PERSON IS ERASED; THE WALL STAYS. A deleted account's photo notes and
/// event backdrops keep their pictures, like its messages keep theirs — the
/// cleanup of what it left half-finished must not mistake a pinned picture
/// (no message, but a note) for an unused upload.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn deleting_an_account_keeps_the_pictures_on_its_board_notes() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;

    let photo = upload_photo(&server, &member).await;
    let photo_note: Value = pin_photo(&server, &member, photo, "The lake")
        .await
        .json()
        .await
        .expect("JSON");
    let backdrop = upload_photo(&server, &member).await;
    let event: Value = pin_event(
        &server,
        &member,
        "Picnic",
        json!({"attachment_id": backdrop}),
    )
    .await
    .json()
    .await
    .expect("JSON");
    // And one upload the member never used, which DOES go.
    let loose = upload_photo(&server, &member).await;

    let deleted = server
        .post(&member, "/me/delete", json!({"password": "password123"}))
        .await;
    assert!(deleted.status().is_success(), "{:?}", deleted.text().await);

    let board: Value = server
        .get(&owner, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    for (note, picture) in [(&photo_note, photo), (&event, backdrop)] {
        let note_id = note["note"]["id"].as_i64().expect("id");
        let found = board["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .find(|n| n["id"].as_i64() == Some(note_id))
            .unwrap_or_else(|| panic!("note {note_id} left the wall"));
        assert_eq!(found["attachment"]["id"].as_i64(), Some(picture), "{found}");
        assert_eq!(
            server
                .get(&owner, &format!("/attachments/{picture}"))
                .await
                .status(),
            200,
            "the picture is still there to fetch"
        );
    }
    let loose_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM attachments WHERE id = $1")
        .bind(loose)
        .fetch_one(&server.state.pool)
        .await
        .expect("counting");
    assert_eq!(loose_rows, 0, "the upload nothing used is cleaned up");
}

// --- A note names members (protocol.md, "Board") -------------------------

/// Owner "Olive", member "Junior", third member "Gran".
async fn family_of_three(ts: &TestServer) -> (String, String, String, i64, i64) {
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (gran, _) = ts.register("gran", "Gran").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    ts.join(&gran, &invite_code, "joined").await;
    (owner, member, gran, owner_id, member_id)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_note_names_members_and_every_reader_gets_the_list() {
    let server = spawn_server().await;
    let (owner, member, _gran, owner_id, member_id) = family_of_three(&server).await;

    let response = server
        .post(
            &owner,
            "/families/mine/board/notes",
            json!({
                "text": "@Junior your kit is in the hall",
                "color": "yellow", "x": 0.2, "y": 0.3,
                "mentions": [{"user_id": member_id, "name": "Junior"}],
            }),
        )
        .await;
    assert_eq!(response.status(), 201);
    let created: Value = response.json().await.expect("JSON");
    let note_id = created["note"]["id"].as_i64().expect("id");
    assert_eq!(created["note"]["mentions"][0]["user_id"], member_id);
    assert_eq!(created["note"]["mentions"][0]["name"], "Junior");

    // Every reader of the board gets the list, not just the author: the
    // highlight is drawn for everybody.
    let board: Value = server
        .get(&member, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    let notes = board["notes"].as_array().expect("notes");
    let held = notes
        .iter()
        .find(|note| note["id"].as_i64() == Some(note_id))
        .expect("the note");
    assert_eq!(held["mentions"][0]["name"], "Junior");

    // A note that names nobody carries no `mentions` at all — never `[]`,
    // so a client that predates this reads what it always read.
    let plain = add_note(&server, &owner, "Milk").await;
    assert!(plain["note"].get("mentions").is_none());
    let plain_id = plain["note"]["id"].as_i64().expect("id");

    // A WALL of them, read in one go: each note carries its OWN names, and
    // a note that names nobody still carries none. The page reads every
    // note's names in a single query, and getting that wrong would put one
    // note's highlight on another's words.
    let other = server
        .post(
            &member,
            "/families/mine/board/notes",
            json!({
                "text": "@Olive the forms are signed",
                "color": "blue", "x": 0.4, "y": 0.5,
                "mentions": [{"user_id": owner_id, "name": "Olive"}],
            }),
        )
        .await;
    assert_eq!(other.status(), 201);
    let other_id = other.json::<Value>().await.expect("JSON")["note"]["id"]
        .as_i64()
        .expect("id");

    let board: Value = server
        .get(&member, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    let notes = board["notes"].as_array().expect("notes");
    let named: Vec<(i64, Option<String>)> = notes
        .iter()
        .filter_map(|note| {
            let id = note["id"].as_i64()?;
            let name = note["mentions"]
                .get(0)
                .and_then(|first| first["name"].as_str())
                .map(str::to_string);
            Some((id, name))
        })
        .collect();
    assert!(named.contains(&(note_id, Some("Junior".to_string()))));
    assert!(named.contains(&(other_id, Some("Olive".to_string()))));
    assert!(named.contains(&(plain_id, None)));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_notes_names_are_re_decided_on_every_edit() {
    let server = spawn_server().await;
    let (owner, _member, _gran, owner_id, member_id) = family_of_three(&server).await;

    let created: Value = server
        .post(
            &owner,
            "/families/mine/board/notes",
            json!({
                "text": "@Junior your kit is in the hall",
                "color": "yellow", "x": 0.2, "y": 0.3,
                "mentions": [{"user_id": member_id, "name": "Junior"}],
            }),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let note_id = created["note"]["id"].as_i64().expect("id");

    // Rewritten to name somebody else: unlike a message, whose list is
    // fixed at send, a note's is read off the new text — an edit to a note
    // notifies nobody, so it cannot wake anybody twice.
    let patched: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({
                "text": "@Olive is doing the kit",
                "mentions": [{"user_id": owner_id, "name": "Olive"}],
            }),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(patched["note"]["mentions"][0]["user_id"], owner_id);
    assert_eq!(
        patched["note"]["mentions"].as_array().expect("list").len(),
        1,
        "replaced, not added to"
    );

    // A text edit with no `mentions` CLEARS them: the names are part of
    // what the note says.
    let cleared: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"text": "the kit is in the hall"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert!(cleared["note"].get("mentions").is_none());

    // And a MOVE leaves them alone — position is everyone's business,
    // names are the author's.
    let named_again: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({
                "text": "@Junior — the kit",
                "mentions": [{"user_id": member_id, "name": "Junior"}],
            }),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(named_again["note"]["mentions"][0]["user_id"], member_id);
    let moved: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"x": 0.8, "y": 0.1}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(moved["note"]["mentions"][0]["user_id"], member_id);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_refusals_a_named_note_promises() {
    let server = spawn_server().await;
    let (owner, _member, _gran, _owner_id, member_id) = family_of_three(&server).await;
    let (stranger, stranger_id) = server.register("stranger", "Stranger").await;
    let _ = stranger;

    let refused = |body: Value| async {
        let response = server
            .post(&owner, "/families/mine/board/notes", body)
            .await;
        assert_error(response, 400, "validation").await;
    };

    // The text does not say the name.
    refused(json!({
        "text": "the kit is in the hall", "color": "yellow", "x": 0.1, "y": 0.1,
        "mentions": [{"user_id": member_id, "name": "Junior"}],
    }))
    .await;
    // Somebody outside the family.
    refused(json!({
        "text": "@Stranger hello", "color": "yellow", "x": 0.1, "y": 0.1,
        "mentions": [{"user_id": stranger_id, "name": "Stranger"}],
    }))
    .await;
    // The same member twice.
    refused(json!({
        "text": "@Junior @Junior", "color": "yellow", "x": 0.1, "y": 0.1,
        "mentions": [
            {"user_id": member_id, "name": "Junior"},
            {"user_id": member_id, "name": "Junior"},
        ],
    }))
    .await;
    // An empty name.
    refused(json!({
        "text": "@Junior hello", "color": "yellow", "x": 0.1, "y": 0.1,
        "mentions": [{"user_id": member_id, "name": ""}],
    }))
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_the_author_may_change_who_a_note_names() {
    let server = spawn_server().await;
    let (owner, member, _gran, _owner_id, member_id) = family_of_three(&server).await;

    let created: Value = server
        .post(
            &owner,
            "/families/mine/board/notes",
            json!({
                "text": "@Junior your kit is in the hall",
                "color": "yellow", "x": 0.2, "y": 0.3,
                "mentions": [{"user_id": member_id, "name": "Junior"}],
            }),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let note_id = created["note"]["id"].as_i64().expect("id");

    // The names are part of what the note says, so they follow the same
    // split rule as its text: anyone may MOVE it, only the author may
    // change what it says.
    let response = server
        .patch(
            &member,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"mentions": []}),
        )
        .await;
    assert_error(response, 403, "not_note_author").await;
}

/// Pin a task list with these lines and hand back the created note.
async fn pin_list(ts: &TestServer, token: &str, title: &str, items: Value) -> Value {
    let response = ts
        .post(
            token,
            "/families/mine/board/notes",
            json!({
                "text": title, "color": "green", "kind": "tasks",
                "x": 0.2, "y": 0.3, "items": items,
            }),
        )
        .await;
    assert_eq!(response.status(), 201);
    response.json().await.expect("JSON")
}

/// The whole split, in one flow: the AUTHOR writes the list and ANYONE
/// ticks it (docs/protocol.md, "Board").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_task_list_is_written_by_its_author_and_ticked_by_anyone() {
    let server = spawn_server().await;
    let (owner, member, _gran, _owner_id, member_id) = family_of_three(&server).await;

    let created = pin_list(
        &server,
        &owner,
        "Saturday",
        json!([{"text": "Milk"}, {"text": "Bread"}]),
    )
    .await;
    let note_id = created["note"]["id"].as_i64().expect("id");
    let items = created["note"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["text"], "Milk");
    assert_eq!(items[1]["text"], "Bread");
    assert_eq!(items[0]["done"], false);
    // Not done means nobody did it, so there is nobody to name.
    assert!(items[0].get("done_by").is_none());
    let milk = items[0]["id"].as_i64().expect("item id");
    let bread = items[1]["id"].as_i64().expect("item id");
    let written_seq = created["note"]["content_seq"].as_i64().expect("seq");

    // An empty list is a list: pinning the title and filling it in later
    // is how a list gets made.
    let blank = pin_list(&server, &owner, "Sunday", json!([])).await;
    assert_eq!(blank["note"]["items"].as_array().expect("items").len(), 0);

    // ANYONE ticks — and the server records who.
    let ticked: Value = server
        .put(
            &member,
            &format!("/families/mine/board/notes/{note_id}/tasks/{milk}"),
            json!({"done": true}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let items = ticked["note"]["items"].as_array().expect("items");
    assert_eq!(items[0]["done"], true);
    assert_eq!(items[0]["done_by"].as_i64(), Some(member_id));
    assert_eq!(items[1]["done"], false);
    // A tick reaches the other devices (a new board_seq) and raises NO
    // badge: the list says exactly what it said before.
    let ticked_seq = ticked["note"]["board_seq"].as_i64().expect("seq");
    assert!(ticked_seq > created["note"]["board_seq"].as_i64().expect("seq"));
    assert_eq!(ticked["note"]["content_seq"].as_i64(), Some(written_seq));

    // A state, not a toggle: the same state again is a no-op that burns no
    // seq — which is what makes two phones tapping the same line safe.
    let again: Value = server
        .put(
            &member,
            &format!("/families/mine/board/notes/{note_id}/tasks/{milk}"),
            json!({"done": true}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(again["note"]["board_seq"].as_i64(), Some(ticked_seq));
    assert_eq!(again["note"]["items"][0]["done"], true);

    // Unticking forgets who did it: an item nobody has done has nobody
    // who did it.
    let untucked: Value = server
        .put(
            &member,
            &format!("/families/mine/board/notes/{note_id}/tasks/{milk}"),
            json!({"done": false}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(untucked["note"]["items"][0]["done"], false);
    assert!(untucked["note"]["items"][0].get("done_by").is_none());
    // Ticked again, so the rest of the flow has a tick to protect.
    server
        .put(
            &member,
            &format!("/families/mine/board/notes/{note_id}/tasks/{milk}"),
            json!({"done": true}),
        )
        .await;

    // WRITING the list is the author's.
    assert_error(
        server
            .patch(
                &member,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"items": [{"text": "Wine"}]}),
            )
            .await,
        403,
        "not_note_author",
    )
    .await;

    // The author rewrites: a line whose id comes back is that line —
    // rewritten, moved, and STILL TICKED. One without an id is new, and a
    // line left out is gone.
    let rewritten: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"items": [
                {"text": "Eggs"},
                {"id": milk, "text": "Oat milk"},
            ]}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let items = rewritten["note"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["text"], "Eggs");
    assert_eq!(items[0]["done"], false);
    assert_eq!(items[1]["id"].as_i64(), Some(milk));
    assert_eq!(items[1]["text"], "Oat milk");
    assert_eq!(
        items[1]["done"], true,
        "fixing a typo must not untick the line"
    );
    assert_eq!(items[1]["done_by"].as_i64(), Some(member_id));
    // Bread was left out and is gone — and its id is not one of this
    // note's lines any more.
    assert!(items.iter().all(|item| item["id"].as_i64() != Some(bread)));
    // A line the AUTHOR wrote is something to READ, so this one moves the
    // badge — the one thing on a task list that does.
    let rewritten_seq = rewritten["note"]["content_seq"].as_i64().expect("seq");
    assert!(rewritten_seq > written_seq);

    // Sending the list it already holds changes nothing at all.
    let noop: Value = server
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"items": [
                {"id": items[0]["id"].as_i64().expect("id"), "text": "Eggs"},
                {"id": milk, "text": "Oat milk"},
            ]}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        noop["note"]["board_seq"].as_i64(),
        rewritten["note"]["board_seq"].as_i64()
    );

    // Every reader gets the list, and a MOVE leaves it alone.
    let moved: Value = server
        .patch(
            &member,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"x": 0.7, "y": 0.2}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(moved["note"]["items"].as_array().expect("items").len(), 2);
    assert_eq!(moved["note"]["content_seq"].as_i64(), Some(rewritten_seq));

    let board: Value = server
        .get(&member, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    let held = board["notes"]
        .as_array()
        .expect("notes")
        .iter()
        .find(|note| note["id"].as_i64() == Some(note_id))
        .expect("the list");
    assert_eq!(held["items"][1]["text"], "Oat milk");
    assert_eq!(held["items"][1]["done"], true);
    // And a note that is not a list carries no `items` at all, so a client
    // that has never heard of them reads what it always read.
    let plain = add_note(&server, &owner, "Milk").await;
    assert!(plain["note"].get("items").is_none());
}

/// The refusals a task list promises (docs/protocol.md, "Board").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_ways_of_writing_a_task_list_wrong() {
    let server = spawn_server().await;
    let (owner, member) = family_of_two(&server).await;

    // Lines belong to a list and to nothing else.
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({
                    "text": "Milk", "color": "yellow", "x": 0.1, "y": 0.1,
                    "items": [{"text": "Milk"}],
                }),
            )
            .await,
        400,
        "validation",
    )
    .await;
    // A title is required, as an event's is.
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({
                    "text": "   ", "color": "green", "kind": "tasks",
                    "x": 0.1, "y": 0.1, "items": [{"text": "Milk"}],
                }),
            )
            .await,
        400,
        "validation",
    )
    .await;
    // Ids are the SERVER's: a created line cannot already have one.
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({
                    "text": "Saturday", "color": "green", "kind": "tasks",
                    "x": 0.1, "y": 0.1, "items": [{"id": 1, "text": "Milk"}],
                }),
            )
            .await,
        400,
        "validation",
    )
    .await;
    // Empty, too long, and too many.
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({
                    "text": "Saturday", "color": "green", "kind": "tasks",
                    "x": 0.1, "y": 0.1, "items": [{"text": "   "}],
                }),
            )
            .await,
        400,
        "validation",
    )
    .await;
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({
                    "text": "Saturday", "color": "green", "kind": "tasks",
                    "x": 0.1, "y": 0.1, "items": [{"text": "x".repeat(101)}],
                }),
            )
            .await,
        400,
        "validation",
    )
    .await;
    let too_many: Vec<Value> = (0..21).map(|n| json!({"text": format!("t{n}")})).collect();
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({
                    "text": "Saturday", "color": "green", "kind": "tasks",
                    "x": 0.1, "y": 0.1, "items": too_many,
                }),
            )
            .await,
        400,
        "validation",
    )
    .await;

    let list = pin_list(&server, &owner, "Saturday", json!([{"text": "Milk"}])).await;
    let note_id = list["note"]["id"].as_i64().expect("id");
    let item_id = list["note"]["items"][0]["id"].as_i64().expect("item id");

    // An id that is not this note's is a bug in the client, not a new
    // line: treating it as new would silently lose the edit.
    assert_error(
        server
            .patch(
                &owner,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"items": [{"id": item_id + 5_000, "text": "Milk"}]}),
            )
            .await,
        400,
        "validation",
    )
    .await;
    // The same line twice in one list.
    assert_error(
        server
            .patch(
                &owner,
                &format!("/families/mine/board/notes/{note_id}"),
                json!({"items": [
                    {"id": item_id, "text": "Milk"},
                    {"id": item_id, "text": "Milk again"},
                ]}),
            )
            .await,
        400,
        "validation",
    )
    .await;

    // Only a list has anything to tick, and only its own lines.
    let plain = add_note(&server, &owner, "Milk").await;
    let plain_id = plain["note"]["id"].as_i64().expect("id");
    assert_error(
        server
            .put(
                &member,
                &format!("/families/mine/board/notes/{plain_id}/tasks/{item_id}"),
                json!({"done": true}),
            )
            .await,
        400,
        "invalid_task",
    )
    .await;
    assert_error(
        server
            .put(
                &member,
                &format!(
                    "/families/mine/board/notes/{note_id}/tasks/{}",
                    item_id + 5_000
                ),
                json!({"done": true}),
            )
            .await,
        400,
        "invalid_task",
    )
    .await;
    // ANOTHER list's line, which is the case that has to be scoped by the
    // note and not by the item id alone: both exist, and a tick on the
    // wrong list would move a line nobody was looking at.
    let other = pin_list(&server, &owner, "Sunday", json!([{"text": "Wine"}])).await;
    let other_item = other["note"]["items"][0]["id"].as_i64().expect("item id");
    assert_error(
        server
            .put(
                &member,
                &format!("/families/mine/board/notes/{note_id}/tasks/{other_item}"),
                json!({"done": true}),
            )
            .await,
        400,
        "invalid_task",
    )
    .await;
    // And the line it names is untouched.
    let untouched: Value = server
        .get(&member, "/families/mine/board")
        .await
        .json()
        .await
        .expect("JSON");
    let sunday = untouched["notes"]
        .as_array()
        .expect("notes")
        .iter()
        .find(|note| note["id"].as_i64() == other["note"]["id"].as_i64())
        .expect("the other list");
    assert_eq!(sunday["items"][0]["done"], false);

    // Another family's list is no such note — not `invalid_task`, which
    // would say the note is there.
    let (stranger, _) = server.register("stranger", "Stranger").await;
    server.create_family(&stranger, "The Joneses").await;
    assert_error(
        server
            .put(
                &stranger,
                &format!("/families/mine/board/notes/{note_id}/tasks/{item_id}"),
                json!({"done": true}),
            )
            .await,
        404,
        "note_not_found",
    )
    .await;

    // A tombstoned list has nothing to tick either.
    server
        .delete(&owner, &format!("/families/mine/board/notes/{note_id}"))
        .await;
    assert_error(
        server
            .put(
                &member,
                &format!("/families/mine/board/notes/{note_id}/tasks/{item_id}"),
                json!({"done": true}),
            )
            .await,
        404,
        "note_not_found",
    )
    .await;
}
