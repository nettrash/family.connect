//! Auth endpoints: register, login, logout, and `GET /me`.
//!
//! Registration relies on the database (the `users_username_lower_uq` index)
//! for username uniqueness rather than a check-then-insert — the unique
//! violation is mapped to `409 username_taken`, so two concurrent
//! registrations of the same name can never both succeed.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use time::OffsetDateTime;

use crate::auth::{self, AuthUser};
use crate::error::{ApiError, AppJson, codes};
use crate::events;
use crate::models::{Family, PendingJoinRequest, User};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub display_name: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordRequest {
    pub new_password: String,
}

/// `{password}` — the account password, required to delete the account for
/// the same reason `POST /me/password` requires the current one: a live
/// session is not proof of who is holding the phone.
#[derive(Debug, Deserialize)]
pub struct DeleteAccountRequest {
    pub password: String,
}

/// `{month, day}` — the body BOTH birthday endpoints take, the member's own
/// and the owner's.
///
/// Required together rather than optional, because clearing a birthday is
/// `DELETE` and not a half-empty `PUT`: the database refuses a month with
/// no day (migration 0018), so there is no shape here that could express
/// one anyway.
#[derive(Debug, Deserialize)]
pub struct BirthdayRequest {
    pub month: i16,
    pub day: i16,
}

/// The one place the rule lives, so register, change and reset cannot
/// drift apart on what counts as a password.
pub fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(ApiError::validation(format!(
            "password must be at least {MIN_PASSWORD_CHARS} characters"
        )));
    }
    Ok(())
}

pub const MIN_PASSWORD_CHARS: usize = 8;

/// The one place the birthday rule lives, so the member's own endpoint and
/// the owner's cannot drift apart on which dates exist.
///
/// The day is checked against ITS month, not against a flat 1–31: 31 April
/// is not a date and neither is 30 February, and a server that stored them
/// would be handing three clients a value none of them can draw. 29
/// February IS accepted — with no year stored there is no year for it to
/// fail to exist in, which is the one place dropping the year makes the
/// rule looser rather than stricter (docs/protocol.md, "Birthdays").
pub fn validate_birthday(month: i16, day: i16) -> Result<(), ApiError> {
    if !(1..=12).contains(&month) {
        return Err(ApiError::validation("birthday month must be 1-12"));
    }
    let last = days_in_month(month);
    if !(1..=last).contains(&day) {
        return Err(ApiError::validation(format!(
            "birthday day must be 1-{last} for month {month}"
        )));
    }
    Ok(())
}

/// How long a month is when there is no year to ask about. February is 29
/// for exactly that reason: the short February is a property of the year,
/// and no year is stored.
fn days_in_month(month: i16) -> i16 {
    match month {
        2 => 29,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `POST /auth/register`
pub async fn register(
    State(state): State<AppState>,
    AppJson(req): AppJson<RegisterRequest>,
) -> Result<Response, ApiError> {
    validate_username(&req.username)?;
    let display_name = validate_display_name(&req.display_name)?;
    validate_password(&req.password)?;

    let password_hash = auth::hash_password(req.password).await?;

    let inserted = sqlx::query(
        "INSERT INTO users (username, display_name, password_hash)
         VALUES ($1, $2, $3)
         RETURNING id, username, display_name, created_at, avatar_version,
                   birthday_month, birthday_day",
    )
    .bind(&req.username)
    .bind(&display_name)
    .bind(&password_hash)
    .fetch_one(&state.pool)
    .await;

    let row = match inserted {
        Ok(row) => row,
        Err(sqlx::Error::Database(db_err))
            if db_err.constraint() == Some("users_username_lower_uq") =>
        {
            return Err(ApiError::conflict(
                codes::USERNAME_TAKEN,
                "username is already in use",
            ));
        }
        Err(err) => return Err(err.into()),
    };

    let user = User::from_row(&row);
    let token = auth::create_session(&state.pool, &state.cfg, user.id).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"token": token, "user": user})),
    )
        .into_response())
}

/// `POST /auth/login`
pub async fn login(
    State(state): State<AppState>,
    AppJson(req): AppJson<LoginRequest>,
) -> Result<Response, ApiError> {
    // `deleted_at IS NULL` is belt and braces, and deliberately kept: a
    // scrubbed row already holds the never-verifiable `'!'` hash, so it
    // could not be authenticated anyway, and its username was rewritten to
    // `deleted-<id>` — a name the register charset cannot even spell. This
    // line means none of that has to hold for the refusal to: a tombstone
    // is not an account, whatever is in its columns.
    let row = sqlx::query(
        "SELECT id, username, display_name, password_hash, created_at, avatar_version,
                birthday_month, birthday_day
         FROM users WHERE lower(username) = lower($1) AND deleted_at IS NULL",
    )
    .bind(&req.username)
    .fetch_optional(&state.pool)
    .await?;

    // Verify even when the user is unknown (dummy hash inside) so timing
    // does not reveal which usernames exist.
    let stored_hash: Option<String> = row.as_ref().map(|r| r.get("password_hash"));
    if !auth::verify_login_password(stored_hash, req.password).await? {
        return Err(ApiError::invalid_credentials());
    }
    let row = row.expect("password verified, so the user row exists");

    let user = User::from_row(&row);
    let token = auth::create_session(&state.pool, &state.cfg, user.id).await?;
    Ok((StatusCode::OK, Json(json!({"token": token, "user": user}))).into_response())
}

/// `POST /me/password` — change your own password.
///
/// The current password is required even though the caller is holding a
/// live session, and that is the point: the case this protects against is
/// an unattended unlocked phone, where a session is exactly what the
/// attacker already has.
pub async fn change_password(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<ChangePasswordRequest>,
) -> Result<Response, ApiError> {
    validate_password(&req.new_password)?;

    let stored: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
            .bind(auth.user_id)
            .fetch_optional(&state.pool)
            .await?;
    if !auth::verify_login_password(stored, req.current_password).await? {
        return Err(ApiError::invalid_credentials());
    }

    let password_hash = auth::hash_password(req.new_password).await?;
    // The new hash and the revocation go together: a change that stored the
    // password but left the old sessions alive would look done and protect
    // nothing.
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
        .bind(auth.user_id)
        .bind(&password_hash)
        .execute(&mut *tx)
        .await?;
    let revoked: Vec<i64> =
        sqlx::query_scalar("DELETE FROM sessions WHERE user_id = $1 AND id <> $2 RETURNING id")
            .bind(auth.user_id)
            .bind(auth.session_id)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;

    // Sockets close only after the rows are gone, so a racing reconnect
    // cannot re-authenticate on a session that is about to be deleted.
    for session_id in revoked {
        state.registry.close_session(session_id).await;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /me/delete` — permanently delete the calling account
/// (docs/protocol.md, "Deleting an account").
///
/// A deleted account is SCRUBBED, not deleted. The `users` row survives,
/// emptied of everything that identifies the person, so that
/// `messages.sender_id` (ON DELETE RESTRICT), `notes.author_id` and
/// `message_reactions.user_id` all go on resolving: the person is erased
/// and the words stay. What does go outright is their direct chats, both
/// halves, and their private assistant thread — a one-to-one conversation
/// has no meaning with one side removed, and it is the only history the
/// departing member can take without taking somebody else's.
///
/// It never refuses an owner. Ownership passes to the longest-standing
/// remaining member, or the family goes with them if there is nobody left:
/// an account somebody cannot delete because of who else is in their family
/// is exactly the dead end this endpoint exists to forbid.
///
/// `POST` rather than `DELETE /me` because the request carries a body and
/// RFC 9110 gives content on a DELETE no defined semantics.
pub async fn delete_account(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<DeleteAccountRequest>,
) -> Result<Response, ApiError> {
    // An empty password is a malformed request, not a wrong password: the
    // hash comparison would refuse it anyway, but `validation` says which
    // of the two it was.
    if req.password.is_empty() {
        return Err(ApiError::validation("password is required"));
    }

    // 1. Prove it is really them. The same helper the password change uses,
    //    for the same reason: the case this protects against is an
    //    unattended unlocked phone, where a session is what the attacker
    //    already has.
    let row = sqlx::query(
        "SELECT password_hash, family_id FROM users WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(auth.user_id)
    .fetch_optional(&state.pool)
    .await?;
    let stored_hash: Option<String> = row.as_ref().map(|row| row.get("password_hash"));
    if !auth::verify_login_password(stored_hash, req.password).await? {
        return Err(ApiError::invalid_credentials());
    }
    let row = row.expect("password verified, so the user row exists");
    let family_id: Option<i64> = row.get("family_id");

    // 2. Everything that has to be known BEFORE the write, because the
    //    write is what makes it unknowable.

    // Their sessions: the rows are deleted below, and a socket cannot be
    // closed by an id nothing remembers.
    let session_ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM sessions WHERE user_id = $1")
        .bind(auth.user_id)
        .fetch_all(&state.pool)
        .await?;

    // Who has to be told. The family's members, plus everyone who shared a
    // direct chat — a peer can be OUTSIDE the family, because the direct
    // pair is unique globally and a chat follows a pair across families —
    // and after the write neither group can be found: the roster no longer
    // names the caller and the chats that named the peers are gone.
    let recipients: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM users WHERE family_id = $1
         UNION
         SELECT cm.user_id
           FROM chat_members cm
           JOIN chats c ON c.id = cm.chat_id
          WHERE c.user_a_id = $2 OR c.user_b_id = $2",
    )
    .bind(family_id)
    .bind(auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    // The files whose rows are about to go. Collected before the delete and
    // swept after the commit, each key checked rather than removed: since
    // 0011 several rows may name one file, and a photo this member sent to
    // the FAMILY chat still has a row — its message stays, so its file
    // stays with it. That is the rule doing the work in "attachments the
    // member uploaded are removed, subject to the one rule attachment
    // deletion always obeys".
    let mut storage_keys: Vec<String> = sqlx::query_scalar(
        "SELECT a.storage_key FROM attachments a WHERE a.uploader_id = $1
         UNION
         SELECT a.storage_key
           FROM attachments a
           JOIN messages m ON m.id = a.message_id
           JOIN chats c ON c.id = m.chat_id
          WHERE c.user_a_id = $1 OR c.user_b_id = $1",
    )
    .bind(auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    // 3. One transaction. Everything below either happens or none of it
    //    does — a half-deleted account is an account nobody can finish
    //    deleting.
    let mut tx = state.pool.begin().await?;

    // (a) Ownership passes on, or the family goes.
    //
    // The lock is taken on the family this account BELONGS to, by id, and
    // whether it also OWNS it is then read out of the locked row. Locking
    // "the family I own" instead (`WHERE owner_user_id = $1`) locks nothing
    // at all when the answer is no rows — so two members of one family
    // deleting at the same moment took no common lock, and the one that
    // was not yet the owner never learned that the other had just made it
    // one. It handed the family to a member that was, by the time both
    // commits landed, a tombstone; `require_owner` then refused every
    // remaining member and nothing in this server ever repairs
    // `families.owner_user_id`. Every membership change in a family takes
    // this same row's lock — `leave_family`, `remove_member` and this
    // handler — so the roster read below can only see committed answers.
    //
    // It is also what it always was: `grant_membership`'s `UPDATE users SET
    // family_id` takes a FOR KEY SHARE lock on this row for its
    // foreign-key check, and FOR UPDATE conflicts with it, so somebody
    // joining under `join_policy = 'open'` either lands before the
    // successor is chosen or waits for a family that may no longer be
    // there.
    let locked_owner_id: Option<i64> = match family_id {
        Some(family_id) => {
            sqlx::query_scalar("SELECT owner_user_id FROM families WHERE id = $1 FOR UPDATE")
                .bind(family_id)
                .fetch_optional(&mut *tx)
                .await?
        }
        None => None,
    };
    let owned_family_id: Option<i64> = match (family_id, locked_owner_id) {
        (Some(family_id), Some(owner_user_id)) if owner_user_id == auth.user_id => Some(family_id),
        _ => None,
    };
    let mut successor: Option<i64> = None;
    let mut family_to_delete: Option<i64> = None;
    if let Some(owned_family_id) = owned_family_id {
        // Longest-standing means the earliest to join the family CHAT, ties
        // broken by the lower user id, so every read of this question gives
        // the same answer. A tombstone can never be chosen: it is neither
        // in the family nor alive.
        //
        // Asked twice, and the second time under a lock on the row it is
        // about. The ordering read is a plain SELECT and proves nothing
        // about a `users` row a transaction that has not committed yet is
        // in the middle of changing — a member half way through leaving is
        // still `family_id = F` to this snapshot. So the pick is re-checked
        // with `FOR UPDATE` held on that one row: either it was never being
        // changed, or this waits for the change and then sees it. A pick
        // that fails the re-check is set aside and the next-longest-standing
        // member tried, which is the same answer the ordering would have
        // given had the leave landed a moment earlier. Nobody left to try
        // means the family goes, exactly as if they had been alone.
        let mut rejected: Vec<i64> = Vec::new();
        loop {
            let candidate: Option<i64> = sqlx::query_scalar(
                "SELECT cm.user_id
                   FROM chat_members cm
                   JOIN chats c ON c.id = cm.chat_id
                   JOIN users u ON u.id = cm.user_id
                  WHERE c.family_id = $1 AND c.kind = 'family' AND cm.user_id <> $2
                    AND u.family_id = $1 AND u.deleted_at IS NULL
                    AND cm.user_id <> ALL($3)
                  ORDER BY cm.joined_at ASC, cm.user_id ASC
                  LIMIT 1",
            )
            .bind(owned_family_id)
            .bind(auth.user_id)
            .bind(&rejected)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(candidate) = candidate else {
                // Nobody else is here: the family is deleted in (f), once
                // the rows that point into it have gone.
                family_to_delete = Some(owned_family_id);
                break;
            };
            let locked = sqlx::query(
                "SELECT family_id, deleted_at IS NULL AS alive
                   FROM users WHERE id = $1 FOR UPDATE",
            )
            .bind(candidate)
            .fetch_optional(&mut *tx)
            .await?;
            let still_here = locked.as_ref().is_some_and(|row| {
                row.get::<Option<i64>, _>("family_id") == Some(owned_family_id)
                    && row.get::<bool, _>("alive")
            });
            if still_here {
                sqlx::query("UPDATE families SET owner_user_id = $2 WHERE id = $1")
                    .bind(owned_family_id)
                    .bind(candidate)
                    .execute(&mut *tx)
                    .await?;
                successor = Some(candidate);
                break;
            }
            rejected.push(candidate);
        }
    }

    // (b) Their direct chats, both halves, and their private assistant
    //     thread — `kind = 'ai'` hangs off `user_a_id` too. The cascade
    //     takes chat_members, chat_reads, messages and with them
    //     message_reactions, the attachment rows, polls, and the SET NULLs
    //     on ai_usage.message_id and messages.reply_to_message_id.
    sqlx::query("DELETE FROM chats WHERE user_a_id = $1 OR user_b_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await?;

    // (c) Their votes come off every poll that is still OPEN — a live tally
    //     must not go on counting somebody who no longer exists. Votes on
    //     CLOSED polls stay: a finished poll is a record, exactly like the
    //     messages and reactions that stay. After (b), so the only polls
    //     left are the ones in chats that survive.
    let open_poll_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT pv.poll_id
           FROM poll_votes pv
           JOIN polls p ON p.message_id = pv.poll_id
          WHERE pv.user_id = $1 AND p.closed_at IS NULL",
    )
    .bind(auth.user_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut retracted: Vec<crate::handlers_poll::PollState> =
        Vec::with_capacity(open_poll_ids.len());
    for poll_id in open_poll_ids {
        // FOR UPDATE on the poll row, in the same order every voter takes
        // its locks (poll row, then chat row): it serializes this retraction
        // against a concurrent vote, so neither ships a frame missing the
        // other's change. The `closed_at IS NULL` is re-checked under the
        // lock — the poll may have been closed since the list was read, and
        // a closed poll keeps its votes.
        let chat_id: Option<i64> = sqlx::query_scalar(
            "SELECT chat_id FROM polls WHERE message_id = $1 AND closed_at IS NULL FOR UPDATE",
        )
        .bind(poll_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(chat_id) = chat_id else {
            continue;
        };
        sqlx::query("DELETE FROM poll_votes WHERE poll_id = $1 AND user_id = $2")
            .bind(poll_id)
            .bind(auth.user_id)
            .execute(&mut *tx)
            .await?;
        // A retraction is a change to the poll like any other, so it takes
        // the next seq and advances the chat's cursor — otherwise no
        // catch-up would ever carry it.
        crate::handlers_poll::stamp_new_seq(&mut tx, chat_id, poll_id).await?;
        // Read from inside the transaction, so what is broadcast is what
        // was committed.
        let poll = crate::handlers_poll::fetch_poll(&mut *tx, poll_id).await?;
        retracted.push(crate::handlers_poll::PollState {
            chat_id,
            message_id: poll_id,
            poll,
            changed: true,
        });
    }

    // (d) The personal rows. Every one of these is about the ACCOUNT rather
    //     than about anything the family said, so none of it survives.
    //     Deleting the sessions takes the device rows with them (0021's
    //     CASCADE — this is the fourth call site that header predicted, and
    //     the constraint is what makes sure no push token outlives the
    //     account nobody remembered to think about it).
    sqlx::query("DELETE FROM user_avatars WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await?;
    // And any device row the cascade above could not reach. Every
    // registration since 0020 binds a session, so on a freshly installed
    // server this deletes nothing — but 0021 deliberately LEFT the pre-0020
    // orphans alone (`session_id IS NULL` on a row older than migration 20
    // is an honest unknown, not an artefact), and on an upgraded server
    // those rows are still here. protocol.md says "every session and every
    // device row, and with them every push token" without an exception for
    // how the row got there, and a live push token is exactly the kind of
    // thing a deletion is promised to destroy. `devices.user_id` is ON
    // DELETE CASCADE, which never fires, because the users row survives the
    // scrub on purpose.
    sqlx::query("DELETE FROM devices WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM chat_members WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM chat_reads WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM join_requests WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await?;
    // An upload NO message ever claimed is a half-finished action of an
    // account that no longer exists, and nothing else would remove it for
    // hours. A CLAIMED one is not touched: its message is part of the
    // shared record and keeps its picture.
    sqlx::query("DELETE FROM attachments WHERE uploader_id = $1 AND message_id IS NULL")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await?;

    // (e) The scrub itself.
    //
    //   * `'deleted-' || id` frees the real username for somebody else to
    //     register AND can never collide with a name anybody could take:
    //     the register charset is [a-zA-Z0-9_.] and has no '-';
    //   * `'!'` is the same never-verifiable sentinel migration 0015 gives
    //     the assistant account — it is not a hash, so no password can ever
    //     match it;
    //   * `avatar_seq` is deliberately NOT reset. It is the never-reused
    //     counter from 0004, and reusing a version would hand every client
    //     that cached this user's picture a stale one under a URL it
    //     already believes is immutable. `avatar_version` going back to 0
    //     is the wire saying "no picture", which is a different fact.
    let scrubbed = sqlx::query(
        "UPDATE users
            SET username = 'deleted-' || id,
                display_name = 'Deleted account',
                password_hash = '!',
                family_id = NULL,
                deleted_at = now(),
                deleted_family_id = $2,
                avatar_version = 0,
                birthday_month = NULL,
                birthday_day = NULL
          WHERE id = $1
          RETURNING id, username, display_name, avatar_version,
                    birthday_month, birthday_day, deleted_at",
    )
    .bind(auth.user_id)
    .bind(family_id)
    .fetch_one(&mut *tx)
    .await?;
    // The frame carries exactly the row that was written. The owner id
    // passed here decides nothing: the row is a tombstone, and a tombstone
    // is given no role at all.
    let member = crate::handlers_family::member_from_row(&scrubbed, auth.user_id);

    // (f) Sole owner: the family goes with them — its chat, its board, its
    //     attachments and its invite code. Last, because everything above
    //     still had rows pointing into it, and the scrub's
    //     `deleted_family_id` is blanked again by the FK, which is right:
    //     a family that no longer exists has no former members to name.
    if let Some(family_to_delete) = family_to_delete {
        let family_keys =
            crate::handlers_family::delete_family_in_tx(&mut tx, family_to_delete).await?;
        storage_keys.extend(family_keys);
    }

    tx.commit().await?;

    // 4. After the commit, never inside it — and so NOTHING below may fail
    //    the request. The account is already deleted: a `?` here would
    //    answer a deletion that HAS happened with a 500, and the client's
    //    retry would then get a 401, because the sessions it would retry
    //    with are among the rows that went. The sweep is logged and
    //    swallowed for exactly the reason fan-out is (`log_fanout_error`):
    //    the worst a lost sweep costs is a file nothing names, which the
    //    unclaimed-attachment sweeper and the next deletion both walk past
    //    harmlessly; the worst a propagated error costs is an account the
    //    user believes is still there.
    if let Err(err) =
        crate::handlers_attachment::remove_all_if_unreferenced(&state, &storage_keys).await
    {
        tracing::warn!(error = ?err, "sweeping a deleted account's attachments failed");
    }
    // Sockets close only once the rows are gone, so a racing reconnect
    // cannot re-authenticate on a session that is about to be deleted.
    for session_id in session_ids {
        state.registry.close_session(session_id).await;
    }
    // Fan-out cannot fail a write that has already committed.
    //
    // `member_left` and `family_owner` are about a ROSTER, so they are
    // family-scoped and are sent only when there was a family. Between
    // them they say nothing to a peer in another family: `member_left`
    // reaches the family's members, `family_owner` the same.
    if let Some(family_id) = family_id {
        events::log_fanout_error(
            "member_left",
            events::deliver_member_left(&state, family_id, auth.user_id).await,
        );
    }
    // `member_deleted` is NOT family-scoped, and this is the whole reason
    // `recipients` was collected before the write: it goes to the family's
    // members AND to every direct-chat peer, including one in another
    // family entirely or none at all. An account that left its family and
    // kept its direct chats used to delete in SILENCE — the peer's chat was
    // destroyed and their live client went on drawing it until its next
    // `GET /chats` (protocol.md, "Deleting an account").
    events::log_fanout_error(
        "member_deleted",
        events::deliver_member_deleted(&state, family_id, member, &recipients).await,
    );
    if let (Some(family_id), Some(new_owner_id)) = (family_id, successor) {
        events::log_fanout_error(
            "family_owner",
            events::deliver_family_owner(&state, family_id, new_owner_id).await,
        );
    }
    for poll_state in &retracted {
        events::log_fanout_error("poll", events::deliver_poll(&state, poll_state).await);
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `PUT /me/birthday` — your own birthday, a day and a month.
///
/// No current-password proof, unlike the password change: a birthday is not
/// a credential, and the worst an unattended phone can do here is wish
/// somebody happy birthday on the wrong day.
pub async fn set_birthday(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<BirthdayRequest>,
) -> Result<Response, ApiError> {
    validate_birthday(req.month, req.day)?;
    // Both columns in one statement, always: they are one fact, and the
    // equivalence constraint would refuse them separately anyway.
    let row = sqlx::query(
        "UPDATE users SET birthday_month = $2, birthday_day = $3 WHERE id = $1
         RETURNING id, username, display_name, created_at, avatar_version,
                   birthday_month, birthday_day",
    )
    .bind(auth.user_id)
    .bind(req.month)
    .bind(req.day)
    .fetch_one(&state.pool)
    .await?;
    Ok((StatusCode::OK, Json(json!({"user": User::from_row(&row)}))).into_response())
}

/// `DELETE /me/birthday` — clear it. Idempotent, like the avatar delete:
/// clearing a birthday nobody set is still a `204`.
pub async fn delete_birthday(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    sqlx::query("UPDATE users SET birthday_month = NULL, birthday_day = NULL WHERE id = $1")
        .bind(auth.user_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /auth/logout` — revokes the calling session and closes its sockets.
pub async fn logout(auth: AuthUser, State(state): State<AppState>) -> Result<Response, ApiError> {
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(auth.session_id)
        .execute(&state.pool)
        .await?;
    state.registry.close_session(auth.session_id).await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /me`
pub async fn me(auth: AuthUser, State(state): State<AppState>) -> Result<Response, ApiError> {
    let row = sqlx::query(
        "SELECT u.id, u.username, u.display_name, u.created_at, u.avatar_version,
                u.birthday_month, u.birthday_day,
                f.id AS family_id, f.name AS family_name, f.join_policy,
                f.created_at AS family_created_at, f.owner_user_id, f.invite_code,
                f.language, f.max_members, f.ai_history
         FROM users u
         LEFT JOIN families f ON f.id = u.family_id
         WHERE u.id = $1",
    )
    .bind(auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let user = User::from_row(&row);
    let family_id: Option<i64> = row.get("family_id");
    let (family, role) = match family_id {
        Some(id) => {
            let owner_user_id: i64 = row.get("owner_user_id");
            let is_owner = owner_user_id == auth.user_id;
            let family = Family {
                id,
                name: row.get("family_name"),
                join_policy: row.get("join_policy"),
                created_at: row.get("family_created_at"),
                // The invite code is owner-only information.
                invite_code: is_owner.then(|| row.get("invite_code")),
                // Everyone sees the language, owner or not: it is what the
                // assistant answers the whole family in.
                language: row.get("language"),
                // And everyone sees the history switch, for a stronger
                // reason: it decides what a member's own words may be used
                // for. This literal exists twice — here and in
                // `FamilyRecord::to_api` — and a field added to one and not
                // the other makes /me and /families/mine disagree about the
                // same family, which is worse than either answer alone.
                max_members: row.get("max_members"),
                ai_history: row.get("ai_history"),
            };
            let role = if is_owner { "owner" } else { "member" };
            (Some(family), Some(role))
        }
        None => (None, None),
    };

    // The caller's live join request, if any — a waiting client that sees
    // neither `family` nor `pending_join_request` knows it was rejected.
    let pending = sqlx::query(
        "SELECT jr.family_id, f.name AS family_name, jr.created_at
         FROM join_requests jr
         JOIN families f ON f.id = jr.family_id
         WHERE jr.user_id = $1 AND jr.status = 'pending'
         ORDER BY jr.id DESC
         LIMIT 1",
    )
    .bind(auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .map(|row| PendingJoinRequest {
        family_id: row.get("family_id"),
        family_name: row.get("family_name"),
        created_at: row.get::<OffsetDateTime, _>("created_at"),
    });

    Ok((
        StatusCode::OK,
        Json(json!({
            "user": user,
            "family": family,
            "role": role,
            "pending_join_request": pending,
            // Whether this server signals voice calls at all. Always
            // present, like the assistant/history switches: a client hides
            // its call button on false rather than discovering
            // `calls_disabled` when somebody wants to talk (protocol.md,
            // "Voice calls").
            "calls_enabled": state.cfg.calls.enabled,
            // And whether it signals VIDEO calls, gating the video button
            // alone. ANDed with `enabled` because the flag is meaningful
            // only with calls on — a server with calls off reports video
            // off too. Always present, like `calls_enabled` (protocol.md,
            // "Video").
            "video_calls_enabled": state.cfg.calls.enabled && state.cfg.calls.video_enabled,
        })),
    )
        .into_response())
}

/// Username: 3–32 chars from `[a-zA-Z0-9_.]` (protocol.md).
fn validate_username(username: &str) -> Result<(), ApiError> {
    let len = username.chars().count();
    if !(3..=32).contains(&len) {
        return Err(ApiError::validation("username must be 3-32 characters"));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return Err(ApiError::validation(
            "username may only contain letters, digits, '_' and '.'",
        ));
    }
    // The assistant sends under a reserved account (migration 0015). The
    // unique index would refuse this anyway, but as `username_taken` — and
    // a member being told a name is "taken" when nobody has it is a worse
    // answer than being told it is reserved.
    if RESERVED_USERNAMES
        .iter()
        .any(|reserved| username.eq_ignore_ascii_case(reserved))
    {
        return Err(ApiError::validation("that username is reserved"));
    }
    Ok(())
}

/// Names the server itself uses and nobody may register.
const RESERVED_USERNAMES: [&str; 1] = ["assistant"];

/// Display name: trimmed, 1–64 chars. Returns the trimmed value, which is
/// what gets stored.
fn validate_display_name(display_name: &str) -> Result<String, ApiError> {
    let trimmed = display_name.trim();
    let len = trimmed.chars().count();
    if !(1..=64).contains(&len) {
        return Err(ApiError::validation("display_name must be 1-64 characters"));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_outside_the_allowed_charset_or_length_are_rejected() {
        assert!(validate_username("anna").is_ok());
        assert!(validate_username("An.na_42").is_ok());
        assert!(validate_username("ab").is_err(), "too short");
        assert!(validate_username(&"a".repeat(33)).is_err(), "too long");
        assert!(validate_username("anna smith").is_err(), "space");
        // The assistant's own account, in any casing.
        assert!(validate_username("assistant").is_err(), "reserved");
        assert!(validate_username("Assistant").is_err(), "reserved, cased");
        assert!(validate_username("anna@home").is_err(), "symbol");
    }

    #[test]
    fn a_birthday_day_is_checked_against_its_own_month() {
        assert!(validate_birthday(3, 14).is_ok());
        assert!(validate_birthday(1, 31).is_ok());
        assert!(validate_birthday(4, 30).is_ok());
        // 31 April is not a date, however happily "1-31" would take it.
        assert!(validate_birthday(4, 31).is_err(), "April has 30 days");
        assert!(validate_birthday(2, 30).is_err(), "February never has 30");
        assert!(validate_birthday(0, 14).is_err(), "month below range");
        assert!(validate_birthday(13, 1).is_err(), "month above range");
        assert!(validate_birthday(3, 0).is_err(), "day below range");
    }

    /// The one date that only works because no year is stored.
    #[test]
    fn the_twenty_ninth_of_february_is_a_birthday() {
        assert!(validate_birthday(2, 29).is_ok());
    }

    #[test]
    fn display_names_are_trimmed_and_length_checked() {
        assert_eq!(validate_display_name("  Anna  ").expect("valid"), "Anna");
        assert!(validate_display_name("   ").is_err(), "empty after trim");
        assert!(validate_display_name(&"x".repeat(65)).is_err(), "too long");
    }
}
