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
