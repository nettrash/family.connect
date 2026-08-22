//! Integration: the family board — the split authorship rule (anyone
//! moves, only the author rewrites or deletes), tombstones, and the third
//! catch-up cursor (protocol.md, "Board").

mod common;

use common::{TestServer, assert_error, spawn_server};
use serde_json::{Value, json};

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
