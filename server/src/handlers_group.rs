//! Groups inside a family (docs/protocol.md, "Groups"; issue #107).
//!
//! A group is a chat of kind `group` with a name, an admin and an explicit
//! member list in `chat_members`. Everything a message does — replies,
//! threads, reactions, polls, edits, retention, unread counts, catch-up —
//! already works in it, because it is a chat. What lives here is what is a
//! group's own: creating one, renaming it, adding and removing members,
//! leaving, deleting, and the admin hand-off when the admin goes.
//!
//! The family chat is never touched by anything in this module: every
//! endpoint refuses a chat that is not a group with `validation`.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use sqlx::{PgConnection, Row};

use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::events;
use crate::models::Chat;
use crate::state::AppState;

/// A group's name, trimmed, is 1 to this many characters (protocol.md,
/// "Limits").
pub const MAX_GROUP_TITLE_CHARS: usize = 64;

/// A family has at most this many groups.
pub const MAX_GROUPS_PER_FAMILY: i64 = 50;

#[derive(Debug, Deserialize)]
pub struct CreateGroupRequest {
    pub title: String,
    #[serde(default)]
    pub member_ids: Vec<i64>,
    /// The family the group is made in (docs/protocol.md, "Several
    /// families"): absent means the home family, and a family the caller is
    /// not in is `family_not_found`.
    #[serde(default)]
    pub family_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RenameGroupRequest {
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct AddMembersRequest {
    #[serde(default)]
    pub user_ids: Vec<i64>,
}

/// What a member leaving a group did to it, for the frames sent after the
/// transaction commits.
#[derive(Debug)]
pub struct GroupExit {
    pub chat_id: i64,
    /// The family the group is in.
    pub family_id: i64,
    /// Nobody was left, so the group went with the last member.
    pub deleted: bool,
    /// The files only the deleted group's messages named — swept, each one
    /// checked, after the commit.
    pub storage_keys: Vec<String>,
}

/// The name as stored: trimmed, and 1 to 64 characters.
pub fn clean_title(raw: &str) -> Result<String, ApiError> {
    let title = raw.trim();
    let count = title.chars().count();
    if count == 0 || count > MAX_GROUP_TITLE_CHARS {
        return Err(ApiError::bad_request(
            codes::VALIDATION,
            "a group's name must be 1 to 64 characters",
        ));
    }
    Ok(title.to_string())
}

/// The ids to add, without duplicates and without the caller.
fn others(ids: &[i64], caller: i64) -> Vec<i64> {
    let mut seen = Vec::with_capacity(ids.len());
    for &id in ids {
        if id != caller && !seen.contains(&id) {
            seen.push(id);
        }
    }
    seen
}

/// Every id is a current member of this family: never somebody who has
/// left, a deleted account (it has no family) or the assistant (it belongs
/// to none) — and never a member of the caller's OTHER family, since a
/// group is inside one family (docs/protocol.md, "Groups").
async fn ensure_family_members(
    conn: &mut PgConnection,
    family_id: i64,
    ids: &[i64],
) -> Result<(), ApiError> {
    let found: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM family_members fm
           JOIN users u ON u.id = fm.user_id
          WHERE fm.user_id = ANY($1) AND fm.family_id = $2 AND u.deleted_at IS NULL",
    )
    .bind(ids)
    .bind(family_id)
    .fetch_one(&mut *conn)
    .await?;
    if found != ids.len() as i64 {
        return Err(ApiError::conflict(
            codes::NOT_SAME_FAMILY,
            "everybody in a group must be a member of your family",
        ));
    }
    Ok(())
}

/// The group as `Chat`: its name, its members in the order they joined,
/// and its admin.
pub async fn group_chat(conn: &mut PgConnection, chat_id: i64) -> Result<Chat, ApiError> {
    let row = sqlx::query(
        "SELECT c.title, c.admin_user_id, c.family_id,
                COALESCE((SELECT array_agg(gm.user_id ORDER BY gm.joined_at, gm.user_id)
                            FROM chat_members gm WHERE gm.chat_id = c.id), '{}') AS member_ids
           FROM chats c
          WHERE c.id = $1 AND c.kind = 'group'",
    )
    .bind(chat_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found(codes::CHAT_NOT_FOUND, "no such chat"));
    };
    Ok(Chat {
        id: chat_id,
        kind: "group".to_string(),
        title: row.get::<Option<String>, _>("title").unwrap_or_default(),
        peer_user_id: None,
        member_ids: Some(row.get("member_ids")),
        admin_user_id: row.get("admin_user_id"),
        // A group is inside exactly one family.
        family_ids: vec![row.get::<i64, _>("family_id")],
    })
}

/// The group, locked for the rest of the transaction, with the caller in it.
///
/// A chat the caller is not in answers as `ensure_chat_access` does — a
/// group they are not in does not exist for them — and a chat they ARE in
/// that is not a group is `validation`: nothing here renames, empties or
/// deletes the family chat, a direct chat or an `ai` chat.
struct LockedGroup {
    family_id: i64,
    admin_user_id: Option<i64>,
}

async fn lock_group(
    conn: &mut PgConnection,
    chat_id: i64,
    caller: i64,
) -> Result<LockedGroup, ApiError> {
    let row = sqlx::query(
        "SELECT c.kind, c.family_id, c.admin_user_id,
                EXISTS (SELECT 1 FROM chat_members cm
                         WHERE cm.chat_id = c.id AND cm.user_id = $2) AS is_member
           FROM chats c
          WHERE c.id = $1
          FOR UPDATE OF c",
    )
    .bind(chat_id)
    .bind(caller)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found(codes::CHAT_NOT_FOUND, "no such chat"));
    };
    let kind: String = row.get("kind");
    let is_member: bool = row.get("is_member");
    if !is_member {
        return Err(if kind == "group" {
            ApiError::not_found(codes::CHAT_NOT_FOUND, "no such chat")
        } else {
            ApiError::forbidden(codes::NOT_CHAT_MEMBER, "you are not a member of this chat")
        });
    }
    if kind != "group" {
        return Err(ApiError::bad_request(
            codes::VALIDATION,
            "only a group can be renamed, changed, left or deleted",
        ));
    }
    Ok(LockedGroup {
        family_id: row.get("family_id"),
        admin_user_id: row.get("admin_user_id"),
    })
}

fn require_admin(group: &LockedGroup, caller: i64) -> Result<(), ApiError> {
    if group.admin_user_id != Some(caller) {
        return Err(ApiError::forbidden(
            codes::NOT_GROUP_ADMIN,
            "only the group's admin may do this",
        ));
    }
    Ok(())
}

/// The files the group's messages name, collected before the rows go.
async fn group_storage_keys(
    conn: &mut PgConnection,
    chat_id: i64,
) -> Result<Vec<String>, ApiError> {
    let keys = sqlx::query_scalar(
        "SELECT DISTINCT a.storage_key
           FROM attachments a
           JOIN messages m ON m.id = a.message_id
          WHERE m.chat_id = $1",
    )
    .bind(chat_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(keys)
}

/// One member out of one group, inside the caller's transaction, with the
/// group's own consequences: the last one out deletes it, and an admin who
/// goes hands it to the member who joined it earliest (ties by lower id).
async fn exit_group(
    conn: &mut PgConnection,
    chat_id: i64,
    family_id: i64,
    user_id: i64,
) -> Result<GroupExit, ApiError> {
    sqlx::query("DELETE FROM chat_members WHERE chat_id = $1 AND user_id = $2")
        .bind(chat_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    let next: Option<i64> = sqlx::query_scalar(
        "SELECT user_id FROM chat_members WHERE chat_id = $1
          ORDER BY joined_at, user_id LIMIT 1",
    )
    .bind(chat_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(next) = next else {
        let storage_keys = group_storage_keys(conn, chat_id).await?;
        sqlx::query("DELETE FROM chats WHERE id = $1")
            .bind(chat_id)
            .execute(&mut *conn)
            .await?;
        return Ok(GroupExit {
            chat_id,
            family_id,
            deleted: true,
            storage_keys,
        });
    };
    sqlx::query(
        "UPDATE chats SET admin_user_id = $2
          WHERE id = $1 AND (admin_user_id = $3 OR admin_user_id IS NULL)",
    )
    .bind(chat_id)
    .bind(next)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(GroupExit {
        chat_id,
        family_id,
        deleted: false,
        storage_keys: Vec::new(),
    })
}

/// A member out of every group of ONE family — leaving it or being removed
/// from it — or, with `None`, out of every group they are in anywhere, which
/// is what deleting their account means. Inside that transaction, BEFORE
/// their `chat_members` rows go. Each group is locked in id order, the order
/// every group write takes, so two of these never deadlock.
///
/// Per family since 2026-10-22: leaving one family leaves its groups and no
/// other's (docs/protocol.md, "Several families").
pub async fn leave_every_group(
    conn: &mut PgConnection,
    user_id: i64,
    family_id: Option<i64>,
) -> Result<Vec<GroupExit>, ApiError> {
    let groups: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT c.id, c.family_id FROM chats c
           JOIN chat_members cm ON cm.chat_id = c.id
          WHERE c.kind = 'group' AND cm.user_id = $1
            AND ($2::BIGINT IS NULL OR c.family_id = $2)
          ORDER BY c.id
          FOR UPDATE OF c",
    )
    .bind(user_id)
    .bind(family_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut exits = Vec::with_capacity(groups.len());
    for (chat_id, group_family) in groups {
        exits.push(exit_group(conn, chat_id, group_family, user_id).await?);
    }
    Ok(exits)
}

/// After the commit: the member who left is told each group is gone from
/// their list, the members who stay get the group as it now is, and the
/// files of a group that went with its last member are swept.
///
/// `departed` is the family a member left and whether it was their HOME
/// family before they left: an app from before several families is told
/// about a group of its home family and of no other (docs/protocol.md,
/// "Several families"). `None` — a member leaving one group, or deleting
/// their account — scopes the leaver's frame to the group's family as it
/// stands, which for a deleted account reaches nobody's socket anyway.
pub async fn announce_exits(
    state: &AppState,
    user_id: i64,
    exits: Vec<GroupExit>,
    departed: Option<(i64, bool)>,
) {
    for exit in exits {
        let scope = match departed {
            Some((family_id, true)) if family_id == exit.family_id => events::Scope::Everywhere,
            Some(_) => events::Scope::MultiOnly,
            None => events::Scope::Family(exit.family_id),
        };
        events::log_fanout_error(
            "chat_removed",
            events::deliver_chat_removed(state, exit.chat_id, &[user_id], scope).await,
        );
        if exit.deleted {
            if let Err(error) =
                crate::handlers_attachment::remove_all_if_unreferenced(state, &exit.storage_keys)
                    .await
            {
                tracing::warn!(?error, "sweeping an emptied group's attachments failed");
            }
            continue;
        }
        announce_group(state, exit.chat_id).await;
    }
}

/// `chat_updated` with the group as it now stands, to every member.
async fn announce_group(state: &AppState, chat_id: i64) {
    let chat = match state.pool.acquire().await {
        Ok(mut conn) => group_chat(&mut conn, chat_id).await,
        Err(error) => Err(error.into()),
    };
    match chat {
        Ok(chat) => events::log_fanout_error(
            "chat_updated",
            events::deliver_chat_updated(state, &chat).await,
        ),
        Err(error) => tracing::warn!(?error, chat_id, "could not read a group to announce it"),
    }
}

/// `POST /chats/groups` — a new group, its creator its admin.
pub async fn create_group(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<CreateGroupRequest>,
) -> Result<Response, ApiError> {
    // The family the body names, or the home family — NOT `auth.family_id`
    // blindly: this route has no `{family}`, so the body is where a
    // multi-family client says which family it is showing.
    let Some(family_id) = auth.family_or_home(req.family_id)? else {
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    };
    let title = clean_title(&req.title)?;
    let members = others(&req.member_ids, auth.user_id);
    if members.is_empty() {
        return Err(ApiError::bad_request(
            codes::VALIDATION,
            "a group needs at least one other member",
        ));
    }

    let mut tx = state.pool.begin().await?;
    // The family row, locked: the count below and the insert after it are
    // one decision, and two creators racing for the fiftieth group must not
    // both get it.
    let still_here: Option<i64> =
        sqlx::query_scalar("SELECT id FROM families WHERE id = $1 FOR NO KEY UPDATE")
            .bind(family_id)
            .fetch_optional(&mut *tx)
            .await?;
    if still_here.is_none() {
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    }
    ensure_family_members(&mut tx, family_id, &members).await?;
    let groups: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chats WHERE family_id = $1 AND kind = 'group'")
            .bind(family_id)
            .fetch_one(&mut *tx)
            .await?;
    if groups >= MAX_GROUPS_PER_FAMILY {
        return Err(ApiError::conflict(
            codes::TOO_MANY_GROUPS,
            "this family already has the most groups it may have",
        ));
    }
    let chat_id: i64 = sqlx::query_scalar(
        "INSERT INTO chats (family_id, kind, title, admin_user_id)
         VALUES ($1, 'group', $2, $3)
         RETURNING id",
    )
    .bind(family_id)
    .bind(&title)
    .bind(auth.user_id)
    .fetch_one(&mut *tx)
    .await?;
    // The creator first, then the others: `clock_timestamp()` moves on row
    // by row where `now()` is the transaction's one instant, so "the order
    // they joined" starts with the person who made the group.
    sqlx::query(
        "INSERT INTO chat_members (chat_id, user_id, joined_at)
         VALUES ($1, $2, clock_timestamp())",
    )
    .bind(chat_id)
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO chat_members (chat_id, user_id, joined_at)
         SELECT $1, member, clock_timestamp() FROM unnest($2::BIGINT[]) AS member",
    )
    .bind(chat_id)
    .bind(&members)
    .execute(&mut *tx)
    .await?;
    let chat = group_chat(&mut tx, chat_id).await?;
    tx.commit().await?;

    events::log_fanout_error(
        "chat_updated",
        events::deliver_chat_updated(&state, &chat).await,
    );
    Ok((StatusCode::CREATED, Json(json!({"chat": chat}))).into_response())
}

/// `PATCH /chats/{id}` — the admin renames a group.
pub async fn rename_group(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    AppJson(req): AppJson<RenameGroupRequest>,
) -> Result<Response, ApiError> {
    let mut tx = state.pool.begin().await?;
    let group = lock_group(&mut tx, chat_id, auth.user_id).await?;
    let title = clean_title(&req.title)?;
    require_admin(&group, auth.user_id)?;
    sqlx::query("UPDATE chats SET title = $2 WHERE id = $1")
        .bind(chat_id)
        .bind(&title)
        .execute(&mut *tx)
        .await?;
    let chat = group_chat(&mut tx, chat_id).await?;
    tx.commit().await?;

    events::log_fanout_error(
        "chat_updated",
        events::deliver_chat_updated(&state, &chat).await,
    );
    Ok((StatusCode::OK, Json(json!({"chat": chat}))).into_response())
}

/// `POST /chats/{id}/members` — the admin adds members. Each one added
/// starts with the group's newest message read: the whole history is
/// theirs to scroll, none of it is news.
pub async fn add_members(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    AppJson(req): AppJson<AddMembersRequest>,
) -> Result<Response, ApiError> {
    let mut tx = state.pool.begin().await?;
    let group = lock_group(&mut tx, chat_id, auth.user_id).await?;
    let members = others(&req.user_ids, auth.user_id);
    if members.is_empty() {
        return Err(ApiError::bad_request(
            codes::VALIDATION,
            "name at least one member to add",
        ));
    }
    require_admin(&group, auth.user_id)?;
    ensure_family_members(&mut tx, group.family_id, &members).await?;
    let added: Vec<i64> = sqlx::query_scalar(
        "INSERT INTO chat_members (chat_id, user_id, joined_at)
         SELECT $1, member, clock_timestamp() FROM unnest($2::BIGINT[]) AS member
         ON CONFLICT DO NOTHING
         RETURNING user_id",
    )
    .bind(chat_id)
    .bind(&members)
    .fetch_all(&mut *tx)
    .await?;
    if !added.is_empty() {
        sqlx::query(
            "INSERT INTO chat_reads (chat_id, user_id, last_read_message_id)
             SELECT $1, member,
                    COALESCE((SELECT max(id) FROM messages WHERE chat_id = $1), 0)
               FROM unnest($2::BIGINT[]) AS member
             ON CONFLICT (chat_id, user_id)
             DO UPDATE SET last_read_message_id = EXCLUDED.last_read_message_id",
        )
        .bind(chat_id)
        .bind(&added)
        .execute(&mut *tx)
        .await?;
    }
    let chat = group_chat(&mut tx, chat_id).await?;
    tx.commit().await?;

    if !added.is_empty() {
        events::log_fanout_error(
            "chat_updated",
            events::deliver_chat_updated(&state, &chat).await,
        );
    }
    Ok((StatusCode::OK, Json(json!({"chat": chat}))).into_response())
}

/// `DELETE /chats/{id}/members/{user_id}` — the admin removes somebody, or
/// a member with their own id leaves.
pub async fn remove_member(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((chat_id, user_id)): Path<(i64, i64)>,
) -> Result<Response, ApiError> {
    let mut tx = state.pool.begin().await?;
    let group = lock_group(&mut tx, chat_id, auth.user_id).await?;

    if user_id == auth.user_id {
        let exit = exit_group(&mut tx, chat_id, group.family_id, user_id).await?;
        tx.commit().await?;
        announce_exits(&state, user_id, vec![exit], None).await;
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    require_admin(&group, auth.user_id)?;
    let removed = sqlx::query("DELETE FROM chat_members WHERE chat_id = $1 AND user_id = $2")
        .bind(chat_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(ApiError::bad_request(
            codes::VALIDATION,
            "that member is not in this group",
        ));
    }
    let chat = group_chat(&mut tx, chat_id).await?;
    tx.commit().await?;

    events::log_fanout_error(
        "chat_removed",
        events::deliver_chat_removed(
            &state,
            chat_id,
            &[user_id],
            events::Scope::Family(group.family_id),
        )
        .await,
    );
    events::log_fanout_error(
        "chat_updated",
        events::deliver_chat_updated(&state, &chat).await,
    );
    Ok((StatusCode::OK, Json(json!({"chat": chat}))).into_response())
}

/// `DELETE /chats/{id}` — the admin deletes a group and everything in it.
pub async fn delete_group(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
) -> Result<Response, ApiError> {
    let mut tx = state.pool.begin().await?;
    let group = lock_group(&mut tx, chat_id, auth.user_id).await?;
    require_admin(&group, auth.user_id)?;
    let members: Vec<i64> =
        sqlx::query_scalar("SELECT user_id FROM chat_members WHERE chat_id = $1")
            .bind(chat_id)
            .fetch_all(&mut *tx)
            .await?;
    let storage_keys = group_storage_keys(&mut tx, chat_id).await?;
    // The cascade takes chat_members, chat_reads, messages and with them
    // reactions, polls and votes, mentions and the attachment rows.
    sqlx::query("DELETE FROM chats WHERE id = $1")
        .bind(chat_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    events::log_fanout_error(
        "chat_removed",
        events::deliver_chat_removed(
            &state,
            chat_id,
            &members,
            events::Scope::Family(group.family_id),
        )
        .await,
    );
    if let Err(error) =
        crate::handlers_attachment::remove_all_if_unreferenced(&state, &storage_keys).await
    {
        tracing::warn!(?error, "sweeping a deleted group's attachments failed");
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed_and_one_to_sixty_four_characters() {
        assert_eq!(clean_title("  Cousins  ").unwrap(), "Cousins");
        assert_eq!(clean_title(&"я".repeat(64)).unwrap().chars().count(), 64);
        assert!(clean_title(&"я".repeat(65)).is_err());
        assert!(clean_title("   ").is_err());
        assert!(clean_title("").is_err());
    }

    #[test]
    fn the_others_lose_duplicates_and_the_caller() {
        assert_eq!(others(&[9, 7, 9, 11, 7], 7), vec![9, 11]);
        assert!(others(&[7, 7], 7).is_empty());
    }
}
