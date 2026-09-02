//! Integration: what the migrations leave behind in the schema.
//!
//! Most of what a migration does is proved by the behaviour built on top of
//! it, and that is where it should be proved — `push_flow.rs` asserts that a
//! revoked device stops being woken, which is the thing anybody cares about.
//! What lands here is the half no request path can reach: the shape of a
//! constraint, and a one-off repair whose whole subject is rows that only
//! ever existed between two versions of this server.

mod common;

use common::{TestServer, spawn_server};
use family_connect::migrate::MIGRATIONS;
use sqlx::Row;
use time::OffsetDateTime;

/// The embedded SQL of one migration, by version — the real text the
/// migrator runs, not a copy of it in a test.
fn migration_sql(version: i32) -> &'static str {
    MIGRATIONS
        .iter()
        .find(|m| m.version == version)
        .unwrap_or_else(|| panic!("migration {version} is in the list"))
        .sql
}

/// When the migrator recorded a version as applied.
async fn applied_at(ts: &TestServer, version: i32) -> OffsetDateTime {
    sqlx::query_scalar("SELECT applied_at FROM schema_migrations WHERE version = $1")
        .bind(version)
        .fetch_one(&ts.state.pool)
        .await
        .expect("the version is recorded")
}

/// Put a device row in directly, with an `updated_at` of the test's
/// choosing — the registration endpoint always writes `now()` and always
/// writes a session, and this test is about rows that have neither.
async fn insert_orphan_device(
    ts: &TestServer,
    user_id: i64,
    push_token: &str,
    updated_at: OffsetDateTime,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO devices (user_id, platform, push_token, session_id, updated_at)
         VALUES ($1, 'ios', $2, NULL, $3) RETURNING id",
    )
    .bind(user_id)
    .bind(push_token)
    .bind(updated_at)
    .fetch_one(&ts.state.pool)
    .await
    .expect("inserting a device row")
}

async fn device_exists(ts: &TestServer, device_id: i64) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM devices WHERE id = $1")
        .bind(device_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("counting the device row")
        == 1
}

/// The security property of 0021, said once at the level it is enforced.
///
/// Everything about a signed-out device not being pushed rests on this one
/// letter: deleting a session must take its device rows — and their push
/// tokens — with it. 0020 shipped `SET NULL` here, which combined with
/// "unknown session ⇒ wake it" made every logged-out phone a guaranteed
/// target for family message bodies on its lock screen.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_device_session_link_cascades_when_a_session_is_deleted() {
    let ts = spawn_server().await;
    // `confdeltype` is the ON DELETE action of a foreign key: 'c' is
    // CASCADE, 'n' is SET NULL, 'a' is NO ACTION.
    let action: String = sqlx::query_scalar(
        "SELECT confdeltype::text FROM pg_constraint
         WHERE conrelid = 'devices'::regclass
           AND confrelid = 'sessions'::regclass
           AND contype = 'f'",
    )
    .fetch_one(&ts.state.pool)
    .await
    .expect("devices has exactly one foreign key to sessions");
    assert_eq!(
        action, "c",
        "devices.session_id must be ON DELETE CASCADE: a device registration \
         belongs to the session it was made from, and a session that is gone \
         must take the push token with it"
    );
}

/// The one-off repair in 0021, which has exactly one job and must not
/// overreach.
///
/// While 0020 was in force, revoking a session blanked `session_id` instead
/// of removing the row, leaving an orphan that is indistinguishable from a
/// device written before the column existed — and both are read as "push
/// it". The one thing that tells them apart is time: after 0020 every
/// registration writes a session id, so a row touched after 0020 landed that
/// still has none can only be an orphan. Rows older than that are the honest
/// unknowns and must survive, or the repair takes alerts away from devices
/// that never did anything wrong.
///
/// Driven by re-running the migration's OWN embedded SQL against a database
/// that already has it — every statement in 0021 is idempotent, and running
/// the real text is what makes this a test of the migration rather than of a
/// copy of it.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_0021_repair_removes_orphans_from_its_own_window_and_nothing_older() {
    let ts = spawn_server().await;
    let (token, _) = ts.register("owner", "Olive").await;
    let user_id = ts.user_id(&token).await;

    let landed = applied_at(&ts, 20).await;
    let before = insert_orphan_device(
        &ts,
        user_id,
        "written-before-0020",
        landed - time::Duration::seconds(1),
    )
    .await;
    let during = insert_orphan_device(
        &ts,
        user_id,
        "orphaned-after-0020",
        landed + time::Duration::seconds(1),
    )
    .await;

    sqlx::raw_sql(migration_sql(21))
        .execute(&ts.state.pool)
        .await
        .expect("re-running migration 21");

    assert!(
        device_exists(&ts, before).await,
        "a device that predates the session column never told us anything \
         and keeps the benefit of the doubt"
    );
    assert!(
        !device_exists(&ts, during).await,
        "a device orphaned by 0020's SET NULL was signed out, and must not \
         be left looking like one that simply never said"
    );
}

/// The migrator's own contract, exercised end to end: every version in the
/// list is recorded exactly once by a real run against a real database.
///
/// The unit tests next to `MIGRATIONS` check the list; this checks that the
/// list was actually applied — a migration appended to the array but whose
/// SQL only works on an empty schema fails here and nowhere else.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn every_migration_in_the_list_is_recorded_as_applied() {
    let ts = spawn_server().await;
    let rows = sqlx::query("SELECT version, name FROM schema_migrations ORDER BY version")
        .fetch_all(&ts.state.pool)
        .await
        .expect("reading schema_migrations");
    let applied: Vec<(i32, String)> = rows
        .iter()
        .map(|row| (row.get("version"), row.get("name")))
        .collect();
    let expected: Vec<(i32, String)> = MIGRATIONS
        .iter()
        .map(|m| (m.version, m.name.to_string()))
        .collect();
    assert_eq!(applied, expected);
}

/// 0031's backfill, run as its own text against a table put back in its
/// pre-0031 shape.
///
/// The column is what a BADGE counts, so what it says about notes that
/// already existed decides whether the first launch after the upgrade
/// badges a wall nobody wrote on. It can only be `board_seq`: nothing in
/// the schema records which of a note's past changes were rewrites, and
/// `board_seq` is the one bound that is certainly not too LOW — the last
/// thing that happened to the note happened at that seq, so "its text was
/// written no later than then" is true of every row. Too high is safe; too
/// low badges a note nobody touched.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_0031_backfill_dates_every_existing_note_by_its_board_seq() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;

    let created: serde_json::Value = ts
        .post(
            &owner,
            "/families/mine/board/notes",
            serde_json::json!({"text": "Milk", "color": "yellow", "x": 0.2, "y": 0.3}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let note_id = created["note"]["id"].as_i64().expect("id");
    let created_seq = created["note"]["board_seq"].as_i64().expect("seq");
    // Dragged afterwards, so board_seq and the creation seq differ — which
    // is the only case where the backfill's choice is visible at all.
    let moved: serde_json::Value = ts
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            serde_json::json!({"x": 0.9}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let moved_seq = moved["note"]["board_seq"].as_i64().expect("seq");
    assert!(moved_seq > created_seq);

    // Back to the shape the table had before 0031, then run the real text.
    sqlx::raw_sql("ALTER TABLE notes DROP COLUMN content_seq")
        .execute(&ts.state.pool)
        .await
        .expect("dropping the column back off");
    sqlx::raw_sql(migration_sql(31))
        .execute(&ts.state.pool)
        .await
        .expect("re-running migration 31");

    let content_seq: i64 = sqlx::query_scalar("SELECT content_seq FROM notes WHERE id = $1")
        .bind(note_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("the backfilled seq");
    assert_eq!(content_seq, moved_seq);

    // …and the column it leaves behind takes no default, so a note written
    // without one is a loud failure rather than a note that never badges.
    let has_default: Option<String> = sqlx::query_scalar(
        "SELECT column_default FROM information_schema.columns
         WHERE table_name = 'notes' AND column_name = 'content_seq'",
    )
    .fetch_one(&ts.state.pool)
    .await
    .expect("the column exists");
    assert_eq!(has_default, None);
}
