//! Family endpoints: create, join, membership management, invite codes.
//!
//! Concurrency discipline: membership changes never trust a read-then-write.
//! The single source of truth for "is this user in a family" is
//! `users.family_id`, and every path that grants membership runs the guarded
//! `UPDATE ... WHERE family_id IS NULL` — zero affected rows means somebody
//! else won the race and the request gets a clean 409 instead of corrupting
//! state. The approve path additionally takes `FOR UPDATE` on the join
//! request so two owner devices cannot approve the same request twice.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Deserializer};
use serde_json::json;
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};
use time::OffsetDateTime;

use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::events;
use crate::models::{Birthday, Family, JoinRequest, Member, User, UserBrief};
use crate::state::AppState;
use crate::tokens;

#[derive(Debug, Deserialize)]
pub struct CreateFamilyRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct JoinFamilyRequest {
    pub invite_code: String,
}

/// Every field optional, in the shape `PatchNoteRequest` already uses:
/// which fields are PRESENT is what decides what happens, and sending
/// nothing is a no-op that answers with the family unchanged.
///
/// `language` is a DOUBLE option, and the two layers mean different things:
/// the outer is "was the key sent at all", the inner is the value. That is
/// the only way `{"language": null}` can mean CLEAR IT while leaving the
/// key out means LEAVE IT ALONE — and without the distinction a family
/// could set a language and never get rid of it again.
///
/// `ai_history` is deliberately NOT one of those, for the reason the column
/// is `NOT NULL`: a switch has no third state. Absent leaves it alone, and
/// a `null` would only be a second spelling of one of the two values that
/// every reader would then have to map back.
#[derive(Debug, Deserialize)]
pub struct PatchFamilyRequest {
    #[serde(default)]
    pub join_policy: Option<String>,
    #[serde(default, deserialize_with = "present_option")]
    pub language: Option<Option<String>>,
    /// The family's own member cap. Three states like the language, and
    /// for the same reason: absent leaves it alone, `null` CLEARS it, and
    /// a number sets it. Clearing is not the same as setting it to the
    /// operator's ceiling — see [`crate::models::Family::max_members`].
    #[serde(default, deserialize_with = "present_option")]
    pub max_members: Option<Option<i32>>,
    #[serde(default)]
    pub ai_history: Option<bool>,
    /// The picture switch, the same shape and for the same reason as
    /// `ai_history` — absent leaves it alone, and there is nothing for a
    /// `null` to mean. It differs only in what it defaults to when nobody
    /// has ever sent it (protocol.md, "Pictures").
    #[serde(default)]
    pub ai_vision: Option<bool>,
}

/// Deserialize a present key into `Some(...)`, so that `#[serde(default)]`
/// can keep an ABSENT key as `None`. serde has no attribute for the
/// distinction; this three-line function is how it is spelled.
fn present_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// The nine locales the apps themselves ship in, spelled the way iOS and
/// Android spell them (docs/protocol.md, "The family's language").
///
/// A fixed list rather than any well-formed BCP 47 tag, for the reason the
/// board's colours are a fixed list: the only thing that reads this is the
/// assistant, and a family that typed a tag the server merely accepted
/// would get answers they could not explain and no error to explain them.
///
/// Both script variants are here on purpose. `sr` and `sr-Latn` are one
/// language in two alphabets and a family that reads one cannot read the
/// other, which is also why `language_instruction` must not collapse them.
pub const FAMILY_LANGUAGES: [&str; 9] = [
    "en", "de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans",
];

/// Match a requested language against the list, case-insensitively, and
/// answer with the CANONICAL spelling.
///
/// Case-insensitively because BCP 47 casing is a convention rather than a
/// rule — `sr-latn` is the same tag as `sr-Latn` — and canonical on the way
/// out so a client can compare what it reads back against its own list
/// without normalising first.
fn validate_language(language: &str) -> Result<String, ApiError> {
    let requested = language.trim();
    FAMILY_LANGUAGES
        .iter()
        .find(|known| known.eq_ignore_ascii_case(requested))
        .map(|known| (*known).to_string())
        .ok_or_else(|| {
            ApiError::bad_request(
                codes::INVALID_LANGUAGE,
                format!("language must be one of {}", FAMILY_LANGUAGES.join(", ")),
            )
        })
}

/// Internal view of a `families` row.
struct FamilyRecord {
    id: i64,
    name: String,
    invite_code: String,
    join_policy: String,
    language: Option<String>,
    max_members: Option<i32>,
    ai_history: bool,
    ai_vision: bool,
    owner_user_id: i64,
    created_at: OffsetDateTime,
}

impl FamilyRecord {
    fn from_row(row: &PgRow) -> Self {
        Self {
            id: row.get("id"),
            name: row.get("name"),
            invite_code: row.get("invite_code"),
            join_policy: row.get("join_policy"),
            language: row.get("language"),
            max_members: row.get("max_members"),
            ai_history: row.get("ai_history"),
            ai_vision: row.get("ai_vision"),
            owner_user_id: row.get("owner_user_id"),
            created_at: row.get("created_at"),
        }
    }

    /// Project to the API object; the invite code is owner-only.
    fn to_api(&self, caller_is_owner: bool) -> Family {
        Family {
            id: self.id,
            name: self.name.clone(),
            join_policy: self.join_policy.clone(),
            created_at: self.created_at,
            invite_code: caller_is_owner.then(|| self.invite_code.clone()),
            // Not owner-gated, unlike the invite code: the language is what
            // the assistant answers the whole family in, so the whole
            // family sees it.
            language: self.language.clone(),
            // Not owner-gated either: every member may see how many people
            // their family admits, even though only the owner may set it.
            max_members: self.max_members,
            // Not owner-gated either, and for a stronger reason than the
            // language: this decides what a member's own words may be used
            // for when somebody else mentions the assistant. Only the owner
            // can CHANGE it; everybody gets to know what it is.
            ai_history: self.ai_history,
            // Not owner-gated either, and for the strongest reason of the
            // four: this decides whether a member's own PHOTOGRAPHS may
            // leave the server. Every member gets to know the answer before
            // they attach one; only the owner can change it.
            ai_vision: self.ai_vision,
        }
    }
}

const SELECT_FAMILY: &str = "SELECT id, name, invite_code, join_policy, language, max_members,
                             ai_history, ai_vision, owner_user_id, created_at FROM families";

async fn fetch_family(state: &AppState, family_id: i64) -> Result<FamilyRecord, ApiError> {
    let row = sqlx::query(&format!("{SELECT_FAMILY} WHERE id = $1"))
        .bind(family_id)
        .fetch_one(&state.pool)
        .await?;
    Ok(FamilyRecord::from_row(&row))
}

/// The id of the family the caller owns, or `not_family_owner`.
///
/// A thin wrapper over [`require_owner`] for callers outside this module
/// that need the ownership check and not the whole record.
pub(crate) async fn require_owner_family(
    state: &AppState,
    auth: &AuthUser,
) -> Result<i64, ApiError> {
    Ok(require_owner(state, auth).await?.id)
}

/// Resolve the caller's family and require them to be its owner.
async fn require_owner(state: &AppState, auth: &AuthUser) -> Result<FamilyRecord, ApiError> {
    let Some(family_id) = auth.family_id else {
        // Not being in a family at all also means not being an owner.
        return Err(ApiError::forbidden(
            codes::NOT_FAMILY_OWNER,
            "only the family owner may do this",
        ));
    };
    let family = fetch_family(state, family_id).await?;
    if family.owner_user_id != auth.user_id {
        return Err(ApiError::forbidden(
            codes::NOT_FAMILY_OWNER,
            "only the family owner may do this",
        ));
    }
    Ok(family)
}

/// What a membership grant can answer.
///
/// Three states rather than a `bool`, because the two failures need
/// different words: somebody who is already in a family is told so, and
/// somebody arriving at a full one is told THAT, and a caller cannot tell
/// them apart from a single `false`.
#[derive(Debug, PartialEq)]
pub(crate) enum Granted {
    Yes,
    AlreadyInAFamily,
    FamilyFull,
}

/// Who WOULD inherit this family if `owner_user_id` left right now.
///
/// The read-only twin of `handlers_auth::pass_ownership_on`'s ordering
/// query — same `ORDER BY cm.joined_at ASC, cm.user_id ASC`, same exclusion
/// of tombstones — with no lock, no re-check and no write, because this
/// answers a question rather than settling one. Computed server-side and
/// not by the clients: the roster carries no join times, so no client could
/// work it out, and two that guessed would eventually disagree.
async fn next_owner_user_id(
    pool: &sqlx::PgPool,
    family_id: i64,
    owner_user_id: i64,
) -> Result<Option<i64>, ApiError> {
    let candidate: Option<i64> = sqlx::query_scalar(
        "SELECT cm.user_id
           FROM chat_members cm
           JOIN chats c ON c.id = cm.chat_id
           JOIN users u ON u.id = cm.user_id
          WHERE c.family_id = $1 AND c.kind = 'family' AND cm.user_id <> $2
            AND u.family_id = $1 AND u.deleted_at IS NULL
          ORDER BY cm.joined_at ASC, cm.user_id ASC
          LIMIT 1",
    )
    .bind(family_id)
    .bind(owner_user_id)
    .fetch_optional(pool)
    .await?;
    Ok(candidate)
}

/// Is this family at its cap? Its own `max_members` when it set one, the
/// operator's `ceiling` when it did not, and never more than the ceiling —
/// "a valve that limited only what an owner may TYPE would hold nothing
/// shut" (protocol.md, "Families"). `None` when the family is gone.
///
/// One expression, two doors, so they cannot drift: `grant_membership`
/// calls it under the family row's `FOR UPDATE`, because there the count
/// and the insert are one decision; the approval door calls it without a
/// lock, because a pending request reserves nothing.
async fn family_is_full(
    conn: &mut PgConnection,
    family_id: i64,
    ceiling: i64,
) -> Result<Option<bool>, ApiError> {
    let cap: Option<Option<i32>> =
        sqlx::query_scalar("SELECT max_members FROM families WHERE id = $1")
            .bind(family_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(cap) = cap else {
        return Ok(None);
    };
    let cap = i64::from(cap.unwrap_or(i32::MAX)).min(ceiling);
    let head_count: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE family_id = $1")
        .bind(family_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(Some(head_count >= cap))
}

/// Grant `user_id` membership of `family_id` inside `tx`.
///
/// Runs the race-proof guarded update, joins the family chat, and
/// auto-rejects the user's other pending join requests (they just got a
/// family; a later approval elsewhere would only 409).
///
/// This is the ONLY place membership is granted — `create_family`,
/// `join_family` and `approve_join_request` all come through here — which
/// is why the member cap is enforced here rather than at each door.
async fn grant_membership(
    tx: &mut PgConnection,
    family_id: i64,
    user_id: i64,
    ceiling: i64,
) -> Result<Granted, ApiError> {
    // Serialize on the family row BEFORE counting. Without this the count
    // and the insert are two statements with a gap in the middle, and two
    // concurrent joins into a family with one seat left both read "one
    // fewer than the cap" and both insert. The FK's implicit FOR KEY SHARE
    // does not conflict with itself and so does not close it.
    //
    // This joins an existing lock order rather than inventing one:
    // `leave_family`, `remove_member` and `delete_account` already take
    // this same row FOR UPDATE, so a caller can never hold one of those
    // and wait on this.
    sqlx::query("SELECT id FROM families WHERE id = $1 FOR UPDATE")
        .bind(family_id)
        .fetch_optional(&mut *tx)
        .await?;

    // Read under the lock, so it cannot move between here and the insert.
    // A family that has vanished under us is reported as full rather than
    // as a 500: the caller is not getting in either way, and the only
    // other answer this helper could give is a `RowNotFound` the error
    // layer maps to Internal.
    match family_is_full(&mut *tx, family_id, ceiling).await? {
        Some(false) => {}
        Some(true) | None => return Ok(Granted::FamilyFull),
    }

    let updated =
        sqlx::query("UPDATE users SET family_id = $1 WHERE id = $2 AND family_id IS NULL")
            .bind(family_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if updated == 0 {
        return Ok(Granted::AlreadyInAFamily);
    }
    let chat_id: i64 =
        sqlx::query_scalar("SELECT id FROM chats WHERE family_id = $1 AND kind = 'family'")
            .bind(family_id)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query(
        "INSERT INTO chat_members (chat_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(chat_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE join_requests SET status = 'rejected', decided_at = now()
         WHERE user_id = $1 AND status = 'pending'",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    Ok(Granted::Yes)
}

/// One member's birthday, read on its own.
async fn member_birthday(state: &AppState, user_id: i64) -> Result<Option<Birthday>, ApiError> {
    let row = sqlx::query("SELECT birthday_month, birthday_day FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Birthday::from_row(&row))
}

async fn user_brief(state: &AppState, user_id: i64) -> Result<UserBrief, ApiError> {
    let row =
        sqlx::query("SELECT id, username, display_name, avatar_version FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&state.pool)
            .await?;
    Ok(UserBrief {
        id: row.get("id"),
        username: row.get("username"),
        display_name: row.get("display_name"),
        avatar_version: row.get("avatar_version"),
    })
}

/// `POST /families` — create a family; the caller becomes owner and the
/// family chat comes to life in the same transaction.
pub async fn create_family(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<CreateFamilyRequest>,
) -> Result<Response, ApiError> {
    let name = req.name.trim().to_string();
    let name_len = name.chars().count();
    if !(1..=64).contains(&name_len) {
        return Err(ApiError::validation("family name must be 1-64 characters"));
    }
    if auth.family_id.is_some() {
        return Err(ApiError::conflict(
            codes::ALREADY_IN_FAMILY,
            "you already belong to a family",
        ));
    }

    // Invite codes have 30^8 combinations, so a unique-index collision is
    // effectively impossible — but "effectively" isn't "actually", and the
    // retry costs three lines.
    for _attempt in 0..5 {
        let invite_code = tokens::gen_invite_code();
        let mut tx = state.pool.begin().await?;

        let inserted = sqlx::query(
            "INSERT INTO families (name, invite_code, owner_user_id)
             VALUES ($1, $2, $3)
             RETURNING id, name, invite_code, join_policy, language, max_members, ai_history,
                       ai_vision, owner_user_id, created_at",
        )
        .bind(&name)
        .bind(&invite_code)
        .bind(auth.user_id)
        .fetch_one(&mut *tx)
        .await;

        let family = match inserted {
            Ok(row) => FamilyRecord::from_row(&row),
            Err(sqlx::Error::Database(db_err))
                if db_err.constraint() == Some("families_invite_code_uq") =>
            {
                continue; // regenerate and retry
            }
            Err(err) => return Err(err.into()),
        };

        sqlx::query("INSERT INTO chats (family_id, kind) VALUES ($1, 'family')")
            .bind(family.id)
            .execute(&mut *tx)
            .await?;

        // A family one statement old cannot be full — its creator is the
        // first member and the cap is at least one — so `FamilyFull` here
        // would be a bug rather than a state, and it is reported as the
        // conflict it is rather than silently mapped onto something else.
        match grant_membership(
            &mut tx,
            family.id,
            auth.user_id,
            state.cfg.limits.max_family_members,
        )
        .await?
        {
            Granted::Yes => {}
            // Lost a race against another create/join from the same account.
            Granted::AlreadyInAFamily => {
                tx.rollback().await?;
                return Err(ApiError::conflict(
                    codes::ALREADY_IN_FAMILY,
                    "you already belong to a family",
                ));
            }
            Granted::FamilyFull => {
                tx.rollback().await?;
                return Err(ApiError::conflict(
                    codes::FAMILY_FULL,
                    "this family is full",
                ));
            }
        }
        tx.commit().await?;

        return Ok((
            StatusCode::CREATED,
            Json(json!({"family": family.to_api(true)})),
        )
            .into_response());
    }
    Err(ApiError::Internal(anyhow::anyhow!(
        "could not generate a unique invite code after 5 attempts"
    )))
}

/// `POST /families/join` — immediate membership for `open` families, a join
/// request for `approval` ones.
pub async fn join_family(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<JoinFamilyRequest>,
) -> Result<Response, ApiError> {
    // Codes are generated from an uppercase alphabet; accept lowercase input
    // because humans type them from a screen across the room.
    let invite_code = req.invite_code.trim().to_uppercase();

    let row = sqlx::query(&format!("{SELECT_FAMILY} WHERE invite_code = $1"))
        .bind(&invite_code)
        .fetch_optional(&state.pool)
        .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found(
            codes::INVALID_INVITE_CODE,
            "no family with this invite code",
        ));
    };
    let family = FamilyRecord::from_row(&row);

    // A closed family admits nobody, and says so in the words of a code
    // that never existed: byte-identical to a wrong one, so a shut door
    // tells a stranger nothing — the same non-enumeration reasoning the
    // avatar and password-reset endpoints follow.
    //
    // BEFORE the already-in-a-family check, which is the order protocol.md
    // pins: "closed, then already in a family, then a pending request,
    // then full — so a closed family answers `invalid_invite_code`
    // whatever else is true of it". The other way round, a caller who is
    // in any family at all gets 409 for a real closed code and 404 for a
    // made-up one, and telling those apart is one `POST /families` away
    // for anybody.
    //
    // Without this branch a closed family is worse than an open bug: the
    // `else` below inserts a join request, so the door would read as shut
    // in the app while a queue filled up behind it, and the owner would be
    // asked to decide on people they never invited.
    if family.join_policy == "closed" {
        return Err(ApiError::not_found(
            codes::INVALID_INVITE_CODE,
            "no family with this invite code",
        ));
    }

    if auth.family_id.is_some() {
        return Err(ApiError::conflict(
            codes::ALREADY_IN_FAMILY,
            "you already belong to a family",
        ));
    }

    if family.join_policy == "open" {
        let mut tx = state.pool.begin().await?;
        match grant_membership(
            &mut tx,
            family.id,
            auth.user_id,
            state.cfg.limits.max_family_members,
        )
        .await?
        {
            Granted::Yes => {}
            Granted::AlreadyInAFamily => {
                tx.rollback().await?;
                return Err(ApiError::conflict(
                    codes::ALREADY_IN_FAMILY,
                    "you already belong to a family",
                ));
            }
            Granted::FamilyFull => {
                tx.rollback().await?;
                return Err(ApiError::conflict(
                    codes::FAMILY_FULL,
                    "this family is full",
                ));
            }
        }
        tx.commit().await?;

        let joined = user_brief(&state, auth.user_id).await?;
        events::log_fanout_error(
            "member_joined",
            events::deliver_member_joined(&state, family.id, joined).await,
        );
        return Ok((StatusCode::OK, Json(json!({"status": "joined"}))).into_response());
    }

    // approval policy: create a pending request; the partial unique index
    // (family_id, user_id) WHERE pending turns a duplicate into a 409.
    //
    // In a transaction because the cap is read at THIS door too
    // (protocol.md: "under policy `approval` this door is where the
    // REQUEST is created and the cap is read there too, then read again at
    // approval"). Without it a family at its cap answers `pending` to
    // everybody and the owner collects requests that every approval
    // refuses and leaves pending for ever — the queue-behind-a-shut-door
    // failure the `closed` branch exists to prevent, arriving through the
    // cap instead of the policy.
    //
    // The INSERT runs FIRST so the documented order holds — a pending
    // request is reported as one before the room is counted — and it goes
    // back with the rollback when the family turns out to be full. No
    // `FOR UPDATE`: a pending request reserves nothing, so this read
    // decides nothing that the grant does not decide again under the lock.
    let mut tx = state.pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO join_requests (family_id, user_id, invite_code_used) VALUES ($1, $2, $3)",
    )
    .bind(family.id)
    .bind(auth.user_id)
    .bind(&invite_code)
    .execute(&mut *tx)
    .await;
    match inserted {
        Ok(_) => {}
        Err(sqlx::Error::Database(db_err))
            if db_err.constraint() == Some("join_requests_pending_uq") =>
        {
            tx.rollback().await?;
            return Err(ApiError::conflict(
                codes::JOIN_REQUEST_PENDING,
                "a join request for this family is already pending",
            ));
        }
        Err(err) => return Err(err.into()),
    }
    match family_is_full(&mut tx, family.id, state.cfg.limits.max_family_members).await? {
        Some(false) => {}
        Some(true) => {
            tx.rollback().await?;
            return Err(ApiError::conflict(
                codes::FAMILY_FULL,
                "this family is full",
            ));
        }
        // The FK held the family in place across the INSERT, so this is
        // unreachable; answered as the door's own truth rather than a 500.
        None => {
            tx.rollback().await?;
            return Err(ApiError::not_found(
                codes::INVALID_INVITE_CODE,
                "no family with this invite code",
            ));
        }
    }
    tx.commit().await?;
    // The owner learns about the request by push when offline; the request
    // itself is already committed, so failures here must not fail the
    // response.
    events::log_fanout_error(
        "join_request_push",
        events::push_join_request_created(
            &state,
            family.id,
            &family.name,
            family.owner_user_id,
            auth.user_id,
        )
        .await,
    );
    Ok((StatusCode::OK, Json(json!({"status": "pending"}))).into_response())
}

/// `GET /families/mine`
pub async fn my_family(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let Some(family_id) = auth.family_id else {
        return Err(ApiError::not_found(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    };
    let family = fetch_family(&state, family_id).await?;
    let rows = sqlx::query(
        "SELECT id, username, display_name, avatar_version, birthday_month, birthday_day
         FROM users WHERE family_id = $1 ORDER BY id",
    )
    .bind(family_id)
    .fetch_all(&state.pool)
    .await?;
    let members: Vec<Member> = rows
        .iter()
        .map(|row| member_from_row(row, family.owner_user_id))
        .collect();
    // The accounts that were deleted while in THIS family, which is what
    // `deleted_family_id` remembers and its only reader. They are not
    // members — no role, offered to nobody, counted by nothing — and they
    // are here for one reason: their messages, notes and reactions are
    // still in the family chat and a client has to put a name to them
    // (protocol.md, "Deleting an account"). Nothing else in this server
    // needs excluding them, because their `family_id` is NULL and every
    // membership question is `family_id = $1`.
    let former_rows = sqlx::query(
        "SELECT id, username, display_name, avatar_version, birthday_month, birthday_day,
                deleted_at
         FROM users WHERE deleted_family_id = $1 AND deleted_at IS NOT NULL ORDER BY id",
    )
    .bind(family_id)
    .fetch_all(&state.pool)
    .await?;
    let former_members: Vec<Member> = former_rows
        .iter()
        .map(|row| member_from_row(row, family.owner_user_id))
        .collect();
    let is_owner = family.owner_user_id == auth.user_id;
    // The board cursor rides along: this is the call every client already
    // makes on resync, so learning whether a board catch-up is needed
    // costs no extra request. Omitted while the board has never been
    // written to, like max_reaction_seq on an unreacted chat.
    let last_board_seq: i64 =
        sqlx::query_scalar("SELECT last_board_seq FROM families WHERE id = $1")
            .bind(family.id)
            .fetch_one(&state.pool)
            .await?;
    let mut body = json!({"family": family.to_api(is_owner), "members": members});
    // Absent rather than `[]` for a family nobody has left, the wire rule
    // every optional key here follows.
    if !former_members.is_empty() {
        body["former_members"] = json!(former_members);
    }
    if last_board_seq > 0 {
        body["max_board_seq"] = json!(last_board_seq);
    }
    // The assistant rides along too, and for two reasons at once.
    //
    // NAMING: it sends under a reserved account that is in no roster (it
    // has no family), so a client meeting its `sender_id` in the family
    // chat would draw a nameless bubble. Its own chat could name it from
    // the chat title; the family chat has no such handle.
    //
    // DISCOVERY: `is_usable()` is a server-side switch nothing exposed
    // before, and a composer that offers "@ai" on a server with no
    // assistant configured is an affordance that silently does nothing.
    // Absent means both "there is nobody to name" and "do not offer it".
    //
    // It is deliberately NOT added to `members`: it is not a member, it
    // cannot be messaged, removed, made owner or given a password, and
    // every screen that lists people would have to special-case it.
    // Owner-only, and a PREDICTION rather than a fact: any join or leave
    // can change the answer, so a client re-reads this immediately before
    // it shows the leave dialog and never names a successor from a cached
    // value. Absent means the owner is the last member and leaving DELETES
    // the family — a different dialog and a different confirmation.
    // As on `GET /me`: always present, `[]` when empty, a complete
    // state-set the client replaces what it stores with.
    body["blocked_user_ids"] = json!(crate::blocks::blocked_by(&state.pool, auth.user_id).await?);
    if is_owner && let Some(next) = next_owner_user_id(&state.pool, family.id, auth.user_id).await?
    {
        body["next_owner_user_id"] = json!(next);
    }
    if state.cfg.ai.is_usable()
        && let Some(assistant_id) = crate::handlers_ai::assistant_user_id(&state)
            .await
            .unwrap_or(None)
    {
        body["assistant"] = json!({
            "user_id": assistant_id,
            "display_name": state.cfg.ai.title,
            "mention": crate::mentions::MENTION,
            // The picture half of the same discovery problem the two keys
            // above solve (protocol.md, "Pictures"). `vision` and `images`
            // are what this SERVER can do; whether this FAMILY allows the
            // first of them is `family.ai_vision`, and a client needs both
            // before it offers to attach a picture in an ai chat. An
            // affordance that silently does nothing is worse than one that
            // is not there — which is the whole reason this object exists.
            "draw": crate::mentions::DRAW,
            "vision": state.cfg.ai.vision_usable(),
            "images": state.cfg.ai.images_usable(),
        });
    }
    Ok((StatusCode::OK, Json(body)).into_response())
}

/// `POST /families/invite-code/rotate` (owner)
pub async fn rotate_invite_code(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    for _attempt in 0..5 {
        let invite_code = tokens::gen_invite_code();
        let updated = sqlx::query("UPDATE families SET invite_code = $1 WHERE id = $2")
            .bind(&invite_code)
            .bind(family.id)
            .execute(&state.pool)
            .await;
        match updated {
            Ok(_) => {
                return Ok(
                    (StatusCode::OK, Json(json!({"invite_code": invite_code}))).into_response()
                );
            }
            Err(sqlx::Error::Database(db_err))
                if db_err.constraint() == Some("families_invite_code_uq") =>
            {
                continue;
            }
            Err(err) => return Err(err.into()),
        }
    }
    Err(ApiError::Internal(anyhow::anyhow!(
        "could not generate a unique invite code after 5 attempts"
    )))
}

/// `PATCH /families/mine` (owner) — the join policy, the family's main
/// language, whether a mention sees the chat's recent history, or any
/// combination of the three.
///
/// Which fields are PRESENT decides what changes, exactly as on a board
/// note. Everything is validated before anything is written, so a request
/// naming a good policy and a bad language changes neither and the owner is
/// never left guessing which half of it landed.
pub async fn patch_family(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<PatchFamilyRequest>,
) -> Result<Response, ApiError> {
    let join_policy = match req.join_policy.as_deref() {
        None => None,
        Some(policy @ ("open" | "approval" | "closed")) => Some(policy.to_string()),
        Some(_) => {
            return Err(ApiError::validation(
                "join_policy must be \"open\", \"approval\" or \"closed\"",
            ));
        }
    };
    // Three states, not two: absent leaves the language alone, `null`
    // clears it, and a tag sets it.
    let language = match &req.language {
        None => None,
        Some(None) => Some(None),
        Some(Some(requested)) => Some(Some(validate_language(requested)?)),
    };
    // Same three states, and validated here rather than after
    // `require_owner` because this handler promises all-validation-first.
    //
    // A cap BELOW the family's current head count is accepted and acts as
    // a freeze — nobody new until people leave — rather than being
    // refused: an owner who inherits a large family must still be able to
    // shut the door, and the cap is read AT the door and never enforced
    // over the room.
    let ceiling = state.cfg.limits.max_family_members;
    let max_members = match &req.max_members {
        None => None,
        Some(None) => Some(None),
        Some(Some(requested)) => {
            if i64::from(*requested) < 1 || i64::from(*requested) > ceiling {
                return Err(ApiError::validation(format!(
                    "max_members must be between 1 and {ceiling}"
                )));
            }
            Some(Some(*requested))
        }
    };

    let family = require_owner(&state, &auth).await?;
    // COALESCE for the policy and for the history switch, because an unsent
    // field binds NULL and the column should keep what it had — which is
    // exactly what COALESCE says. The language cannot use it — NULL is a
    // VALUE there rather than "unchanged" — so a separate flag says whether
    // to write that column at all.
    let row = sqlx::query(
        "UPDATE families
         SET join_policy = COALESCE($2, join_policy),
             language = CASE WHEN $3 THEN $4 ELSE language END,
             ai_history = COALESCE($5, ai_history),
             ai_vision = COALESCE($8, ai_vision),
             max_members = CASE WHEN $6 THEN $7 ELSE max_members END
         WHERE id = $1
         RETURNING id, name, invite_code, join_policy, language, max_members, ai_history,
                   ai_vision, owner_user_id, created_at",
    )
    .bind(family.id)
    .bind(join_policy)
    .bind(language.is_some())
    .bind(language.flatten())
    .bind(req.ai_history)
    .bind(max_members.is_some())
    .bind(max_members.flatten())
    .bind(req.ai_vision)
    .fetch_one(&state.pool)
    .await?;
    let family = FamilyRecord::from_row(&row);
    Ok((StatusCode::OK, Json(json!({"family": family.to_api(true)}))).into_response())
}

/// `GET /families/join-requests` (owner) — pending only.
pub async fn list_join_requests(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    let rows = sqlx::query(
        "SELECT jr.id, jr.created_at,
                u.id AS user_id, u.username, u.display_name, u.avatar_version,
                u.birthday_month, u.birthday_day,
                u.created_at AS user_created_at
         FROM join_requests jr
         JOIN users u ON u.id = jr.user_id
         WHERE jr.family_id = $1 AND jr.status = 'pending'
         ORDER BY jr.id",
    )
    .bind(family.id)
    .fetch_all(&state.pool)
    .await?;
    let requests: Vec<JoinRequest> = rows
        .iter()
        .map(|row| JoinRequest {
            id: row.get("id"),
            user: User {
                id: row.get("user_id"),
                username: row.get("username"),
                display_name: row.get("display_name"),
                created_at: row.get("user_created_at"),
                avatar_version: row.get("avatar_version"),
                birthday: Birthday::from_row(row),
                // A scrubbed account keeps no family and no sessions, so it
                // cannot have a live join request. Read from the row all the
                // same rather than hard-coded: this is the one place a
                // `User` is assembled by hand, and a literal `false` here is
                // a claim the row is entitled to make instead.
                deleted: crate::models::deleted_from_row(row),
            },
            created_at: row.get("created_at"),
        })
        .collect();
    Ok((StatusCode::OK, Json(json!({"requests": requests}))).into_response())
}

/// `POST /families/join-requests/{id}/approve` (owner)
pub async fn approve_join_request(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(request_id): Path<i64>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;

    let mut tx = state.pool.begin().await?;
    // FOR UPDATE serializes concurrent decisions on the same request.
    let row = sqlx::query(
        "SELECT user_id, status FROM join_requests
         WHERE id = $1 AND family_id = $2
         FOR UPDATE",
    )
    .bind(request_id)
    .bind(family.id)
    .fetch_optional(&mut *tx)
    .await?;
    // A request that does not exist (for this family) is indistinguishable
    // from one already decided — same code either way.
    let not_pending = || {
        ApiError::conflict(
            codes::JOIN_REQUEST_NOT_PENDING,
            "join request is not pending",
        )
    };
    let Some(row) = row else {
        return Err(not_pending());
    };
    let status: String = row.get("status");
    if status != "pending" {
        return Err(not_pending());
    }
    let applicant_id: i64 = row.get("user_id");

    match grant_membership(
        &mut tx,
        family.id,
        applicant_id,
        state.cfg.limits.max_family_members,
    )
    .await?
    {
        Granted::Yes => {}
        Granted::AlreadyInAFamily => {
            // The applicant found a family elsewhere in the meantime. The
            // request can never be satisfied, so it is closed as rejected —
            // committed, not rolled back — before reporting the conflict.
            sqlx::query(
                "UPDATE join_requests SET status = 'rejected', decided_at = now(), decided_by = $2
                 WHERE id = $1",
            )
            .bind(request_id)
            .bind(auth.user_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Err(ApiError::conflict(
                codes::USER_ALREADY_IN_FAMILY,
                "this user already belongs to a family",
            ));
        }
        Granted::FamilyFull => {
            // The OPPOSITE disposition to the case above, and the
            // difference is whether the refusal can ever stop being true.
            // "Already in a family" is permanent from this request's point
            // of view, so the row is closed. "Full" is a fact about today:
            // somebody leaves and the same approval works. So the request
            // is left PENDING and the transaction rolled back — the owner
            // keeps the row on their list and decides again later, rather
            // than having the server quietly reject somebody they said yes
            // to.
            tx.rollback().await?;
            return Err(ApiError::conflict(
                codes::FAMILY_FULL,
                "this family is full",
            ));
        }
    }
    // grant_membership auto-rejected the applicant's *other* pending
    // requests; this one becomes the approved record.
    sqlx::query(
        "UPDATE join_requests SET status = 'approved', decided_at = now(), decided_by = $2
         WHERE id = $1",
    )
    .bind(request_id)
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let joined = user_brief(&state, applicant_id).await?;
    let member = Member {
        id: joined.id,
        username: joined.username.clone(),
        display_name: joined.display_name.clone(),
        role: Some("member".to_string()),
        avatar_version: joined.avatar_version,
        // Read on its own rather than carried by `user_brief`: that brief
        // IS the `member_joined` frame payload, whose shape is pinned, and
        // a birthday has no business travelling in a WS frame.
        birthday: member_birthday(&state, applicant_id).await?,
        // An account that has just been approved into a family is live by
        // construction — a tombstone holds no family and makes no requests.
        deleted: false,
    };
    events::log_fanout_error(
        "member_joined",
        events::deliver_member_joined(&state, family.id, joined).await,
    );
    // An offline requester learns they're in by push ("joined").
    events::log_fanout_error(
        "join_approved_push",
        events::push_join_approved(&state, family.id, &family.name, applicant_id).await,
    );
    Ok((StatusCode::OK, Json(json!({"member": member}))).into_response())
}

/// `POST /families/join-requests/{id}/reject` (owner)
pub async fn reject_join_request(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(request_id): Path<i64>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    let mut tx = state.pool.begin().await?;
    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM join_requests WHERE id = $1 AND family_id = $2 FOR UPDATE",
    )
    .bind(request_id)
    .bind(family.id)
    .fetch_optional(&mut *tx)
    .await?;
    if status.as_deref() != Some("pending") {
        return Err(ApiError::conflict(
            codes::JOIN_REQUEST_NOT_PENDING,
            "join request is not pending",
        ));
    }
    sqlx::query(
        "UPDATE join_requests SET status = 'rejected', decided_at = now(), decided_by = $2
         WHERE id = $1",
    )
    .bind(request_id)
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /families/leave`
pub async fn leave_family(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let Some(family_id) = auth.family_id else {
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    };

    // The ownership question, the head count and the delete are ONE
    // decision, so they are one transaction with the family row locked.
    // They used to be two pool round-trips with nothing between them, and
    // under `join_policy = 'open'` somebody could join in that gap and find
    // their new family deleted underneath them. `FOR UPDATE` is what closes
    // it rather than the transaction alone: `grant_membership`'s
    // `UPDATE users SET family_id` takes a FOR KEY SHARE lock on this same
    // families row to check its foreign key, and FOR UPDATE conflicts with
    // it — so a join in flight either lands before the count sees it, or
    // waits and then fails against a family that is gone.
    let mut tx = state.pool.begin().await?;
    let owner_user_id: Option<i64> =
        sqlx::query_scalar("SELECT owner_user_id FROM families WHERE id = $1 FOR UPDATE")
            .bind(family_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(owner_user_id) = owner_user_id else {
        // The family was deleted while this request was in flight.
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    };

    if owner_user_id == auth.user_id {
        // An owner who leaves HANDS THE FAMILY ON; they are never refused.
        // This used to answer `owner_cannot_leave` to anybody who was not
        // the sole member, which left a family whose owner had lost
        // interest with no way to change hands at all.
        //
        // The same rule as "Deleting an account", reached now from two
        // doors rather than one — see `handlers_auth::pass_ownership_on`,
        // which is called under the lock this function already holds.
        let successor =
            crate::handlers_auth::pass_ownership_on(&mut tx, family_id, auth.user_id).await?;
        if let Some(new_owner_user_id) = successor {
            // Ownership is committed together with the departure, so no
            // reader ever sees a family owned by somebody who has left.
            remove_membership(&mut tx, family_id, auth.user_id).await?;
            tx.commit().await?;

            // The successor is told FIRST, so no client momentarily holds a
            // family whose `owner_user_id` names nobody in `members`.
            events::log_fanout_error(
                "family_owner",
                events::deliver_family_owner(&state, family_id, new_owner_user_id).await,
            );
            events::log_fanout_error(
                "member_left",
                events::deliver_member_left(&state, family_id, auth.user_id).await,
            );
            return Ok((
                StatusCode::OK,
                Json(json!({"new_owner_user_id": new_owner_user_id})),
            )
                .into_response());
        }

        // Nobody inherits. Before believing that, check the head count:
        // `pass_ownership_on` can only see members who hold a
        // `chat_members` row for the family chat, and a `users` row that
        // names the family without one is invisible there while still
        // counting everywhere else. Falling through would then DELETE A
        // FAMILY WITH A LIVE MEMBER IN IT. This is a guard against a
        // divergence that should not exist, so it is logged loudly rather
        // than handled quietly.
        let remaining: i64 =
            sqlx::query_scalar("SELECT count(*) FROM users WHERE family_id = $1 AND id <> $2")
                .bind(family_id)
                .bind(auth.user_id)
                .fetch_one(&mut *tx)
                .await?;
        if remaining > 0 {
            tx.rollback().await?;
            // Reported as `internal`, NOT as `owner_cannot_leave`. That
            // code is retired and protocol.md says no endpoint raises it;
            // reviving it here would make the doc false and would also
            // tell the owner they may not leave, which is not true — the
            // server simply cannot work out to whom. This is a database in
            // a state it should not be able to reach, which is what a 500
            // is for, and the log line is the half that matters.
            return Err(ApiError::Internal(anyhow::anyhow!(
                "family {family_id} has {remaining} member(s) but no successor could be found — \
                 a users row names the family with no family-chat membership"
            )));
        }

        // Sole member: delete the family. Cascades remove chats, messages,
        // members, and join requests; users.family_id resets via
        // ON DELETE SET NULL. Nobody is left to notify.
        let storage_keys = delete_family_in_tx(&mut tx, family_id).await?;
        tx.commit().await?;
        // After the commit, never inside it — and so this may not fail the
        // request. The family is already gone: a `?` here would answer a
        // leave that HAS happened with a 500, and the client's retry would
        // then get a 409 `not_in_family`, because the membership it would
        // retry against is among the rows that went. The sweep is logged
        // and swallowed for exactly the reason fan-out is
        // (`log_fanout_error`): the worst a lost sweep costs is a file
        // nothing names, which the unclaimed-attachment sweeper and the
        // next deletion both walk past harmlessly; the worst a propagated
        // error costs is a family the user believes is still there.
        if let Err(err) =
            crate::handlers_attachment::remove_all_if_unreferenced(&state, &storage_keys).await
        {
            tracing::warn!(error = ?err, "sweeping a deleted family's attachments failed");
        }
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    // The membership write happens under the lock taken above, in this same
    // transaction. It used to roll back here and let `remove_membership`
    // open a transaction of its own, which left a window with no lock on
    // the family at all: `delete_account` could take the family row in it,
    // read a roster that still listed this member, and hand them the family
    // they were in the middle of leaving. See `remove_membership`.
    remove_membership(&mut tx, family_id, auth.user_id).await?;
    tx.commit().await?;

    events::log_fanout_error(
        "member_left",
        events::deliver_member_left(&state, family_id, auth.user_id).await,
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `PUT /families/members/{user_id}/block`
///
/// ANY member may block ANY other member of their own family, **the owner
/// included** — there is no `require_owner` here and that is the point.
/// A block affordance is weakest exactly where the person you want to stop
/// reading is the one with power (protocol.md, "Blocking a member").
///
/// Idempotent: blocking somebody already blocked is still `204`, which is
/// what the primary key on the pair buys.
pub async fn block_member(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(target_user_id): Path<i64>,
) -> Result<Response, ApiError> {
    let Some(family_id) = auth.family_id else {
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    };
    if target_user_id == auth.user_id {
        return Err(ApiError::bad_request(
            codes::CANNOT_BLOCK_SELF,
            "you cannot block yourself",
        ));
    }
    let family = fetch_family(&state, family_id).await?;
    require_same_family(&state, &family, target_user_id).await?;

    sqlx::query(
        "INSERT INTO member_blocks (blocker_user_id, blocked_user_id) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(auth.user_id)
    .bind(target_user_id)
    .execute(&state.pool)
    .await?;

    // Only the blocker's own devices, ever.
    events::log_fanout_error(
        "member_blocked",
        events::deliver_member_blocked(&state, auth.user_id, target_user_id, true).await,
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `DELETE /families/members/{user_id}/block`
///
/// Unblocks. Idempotent — clearing a block nobody set is still `204`.
///
/// Scoped to the CALLER'S OWN LIST and not to the roster: any id in
/// `blocked_user_ids` may be cleared, including one that has since left the
/// family or deleted its account, so `not_same_family` is NOT raised here
/// and neither is `not_in_family`. A block is a pair and not a membership;
/// a block the blocker could not lift would be a permanent entry on their
/// own screen and one that came back to life on a rejoin.
pub async fn unblock_member(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(target_user_id): Path<i64>,
) -> Result<Response, ApiError> {
    if target_user_id == auth.user_id {
        return Err(ApiError::bad_request(
            codes::CANNOT_BLOCK_SELF,
            "you cannot block yourself",
        ));
    }
    sqlx::query("DELETE FROM member_blocks WHERE blocker_user_id = $1 AND blocked_user_id = $2")
        .bind(auth.user_id)
        .bind(target_user_id)
        .execute(&state.pool)
        .await?;

    events::log_fanout_error(
        "member_blocked",
        events::deliver_member_blocked(&state, auth.user_id, target_user_id, false).await,
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `DELETE /families/members/{user_id}` (owner)
pub async fn remove_member(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(target_user_id): Path<i64>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    // The caller is the owner, so "target is the owner" == "target is me".
    if target_user_id == auth.user_id {
        return Err(ApiError::conflict(
            codes::CANNOT_REMOVE_OWNER,
            "the owner cannot be removed from the family",
        ));
    }
    // The family row's lock, then the check, then the write — one
    // transaction, the same shape `leave_family` has. Every membership
    // change in a family is serialized on this row so that
    // `delete_account`, which holds it while it chooses a successor, can
    // never read a roster somebody is half way out of.
    let mut tx = state.pool.begin().await?;
    sqlx::query_scalar::<_, i64>("SELECT id FROM families WHERE id = $1 FOR UPDATE")
        .bind(family.id)
        .fetch_optional(&mut *tx)
        .await?;
    let target_family: Option<i64> =
        sqlx::query_scalar("SELECT family_id FROM users WHERE id = $1")
            .bind(target_user_id)
            .fetch_optional(&mut *tx)
            .await?
            .flatten();
    if target_family != Some(family.id) {
        return Err(ApiError::not_found(
            codes::USER_NOT_FOUND,
            "no such member in your family",
        ));
    }

    remove_membership(&mut tx, family.id, target_user_id).await?;
    tx.commit().await?;

    events::log_fanout_error(
        "member_left",
        events::deliver_member_left(&state, family.id, target_user_id).await,
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /families/members/{id}/password` — the owner resets a member's
/// password.
///
/// No current password: the whole point is that the member has forgotten
/// theirs. What makes that safe to expose is that it is owner-only and
/// family-scoped — and what makes it a RECOVERY rather than a convenience
/// is that every session the member has is revoked, so a device somebody
/// else is holding stops working the moment the reset lands.
pub async fn reset_member_password(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(target_user_id): Path<i64>,
    AppJson(req): AppJson<crate::handlers_auth::ResetPasswordRequest>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    crate::handlers_auth::validate_password(&req.new_password)?;

    // The owner changes their OWN password the ordinary way, with the
    // current one — this endpoint would be a way around that check.
    if target_user_id == auth.user_id {
        return Err(ApiError::validation(
            "use POST /me/password to change your own password",
        ));
    }

    require_same_family(&state, &family, target_user_id).await?;

    let password_hash = crate::auth::hash_password(req.new_password).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
        .bind(target_user_id)
        .bind(&password_hash)
        .execute(&mut *tx)
        .await?;
    // ALL of them, unlike a self-change: the member is not the one asking,
    // and every device signed in as them must come back through login.
    let revoked: Vec<i64> =
        sqlx::query_scalar("DELETE FROM sessions WHERE user_id = $1 RETURNING id")
            .bind(target_user_id)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;

    for session_id in revoked {
        state.registry.close_session(session_id).await;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `PUT /families/members/{user_id}/birthday` — the owner sets a member's
/// birthday.
///
/// The case this exists for is a parent and a child: the parent knows the
/// date, and the child is never going to open a settings screen to type it.
///
/// The owner MAY name themselves here, unlike `reset_member_password`. That
/// endpoint refuses a self-target because it would be a way around proving
/// you know the current password; there is no such proof to skip here, the
/// owner can already set their own with `PUT /me/birthday`, and both paths
/// write the same two numbers. Refusing would buy nothing and would make
/// every roster screen carry a special case for exactly one row.
pub async fn set_member_birthday(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(target_user_id): Path<i64>,
    AppJson(req): AppJson<crate::handlers_auth::BirthdayRequest>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    crate::handlers_auth::validate_birthday(req.month, req.day)?;
    require_same_family(&state, &family, target_user_id).await?;

    let row = sqlx::query(
        "UPDATE users SET birthday_month = $2, birthday_day = $3 WHERE id = $1
         RETURNING id, username, display_name, avatar_version,
                   birthday_month, birthday_day",
    )
    .bind(target_user_id)
    .bind(req.month)
    .bind(req.day)
    .fetch_one(&state.pool)
    .await?;
    Ok((
        StatusCode::OK,
        Json(json!({"member": member_from_row(&row, family.owner_user_id)})),
    )
        .into_response())
}

/// `DELETE /families/members/{user_id}/birthday` — the owner clears one.
/// Idempotent, like the member's own delete.
pub async fn delete_member_birthday(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(target_user_id): Path<i64>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    require_same_family(&state, &family, target_user_id).await?;

    sqlx::query("UPDATE users SET birthday_month = NULL, birthday_day = NULL WHERE id = $1")
        .bind(target_user_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Refuse a target who is not in the owner's own family.
///
/// Same answer whether they are in another family or do not exist at all,
/// following the password reset rather than `remove_member`: a birthday is
/// personal information, and an endpoint that answered differently for a
/// real stranger than for an id nobody holds would be a way to find out
/// which accounts exist.
async fn require_same_family(
    state: &AppState,
    family: &FamilyRecord,
    target_user_id: i64,
) -> Result<(), ApiError> {
    let target_family: Option<i64> =
        sqlx::query_scalar("SELECT family_id FROM users WHERE id = $1")
            .bind(target_user_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    if target_family != Some(family.id) {
        return Err(ApiError::forbidden(
            codes::NOT_SAME_FAMILY,
            "no such member in your family",
        ));
    }
    Ok(())
}

/// Build a `Member` from a row exposing `id, username, display_name,
/// avatar_version, birthday_month, birthday_day`, and optionally
/// `deleted_at` — a SELECT that omits the last reads as a live member.
///
/// `pub(crate)` so the account deletion can build the tombstone its
/// `member_deleted` frame carries out of the row it has just scrubbed,
/// rather than assembling the same placeholder by hand in a second place.
pub(crate) fn member_from_row(row: &PgRow, owner_user_id: i64) -> Member {
    let id: i64 = row.get("id");
    let deleted = crate::models::deleted_from_row(row);
    Member {
        id,
        username: row.get("username"),
        display_name: row.get("display_name"),
        // A former member holds no role — they are in `former_members`
        // precisely because they are in nothing else (protocol.md,
        // "Deleting an account"). Every live row gets one.
        role: (!deleted).then(|| member_role(id, owner_user_id).to_string()),
        avatar_version: row.get("avatar_version"),
        birthday: Birthday::from_row(row),
        deleted,
    }
}

/// Delete a family inside the caller's transaction, and hand back the
/// storage keys its rows named so the caller can sweep the files after the
/// commit.
///
/// The two callers are the sole member leaving and the sole member deleting
/// their account, and both need the same three steps in the same order:
/// COLLECT the keys, DELETE, then `remove_if_unreferenced` per key once the
/// transaction has committed.
///
/// The collection is the part that was missing, and its absence was a
/// permanent leak: `DELETE FROM families` cascades to its chats, to their
/// messages and on to the attachment ROWS, so every file the family ever
/// sent stayed on disk with nothing in the database naming it. Neither
/// sweeper can find those bytes again — `sweep_unclaimed` only matches rows
/// with no message, and `sweep_expired_messages` works from messages that
/// are gone.
///
/// The second half of the union is the uploads no message ever claimed:
/// their rows SURVIVE the family (`attachments.family_id` is ON DELETE SET
/// NULL), so `remove_if_unreferenced` deliberately keeps their files and
/// `sweep_unclaimed` takes both together when the grace period is up.
/// Collecting them costs nothing and means this list is "every file this
/// family could name", which is the question worth asking here.
pub(crate) async fn delete_family_in_tx(
    tx: &mut PgConnection,
    family_id: i64,
) -> Result<Vec<String>, ApiError> {
    let storage_keys: Vec<String> = sqlx::query_scalar(
        "SELECT a.storage_key
           FROM attachments a
           JOIN messages m ON m.id = a.message_id
           JOIN chats c ON c.id = m.chat_id
          WHERE c.family_id = $1
         UNION
         SELECT a.storage_key FROM attachments a WHERE a.family_id = $1",
    )
    .bind(family_id)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM families WHERE id = $1")
        .bind(family_id)
        .execute(&mut *tx)
        .await?;
    Ok(storage_keys)
}

/// Shared leave/remove write: detach the user and drop them from every chat
/// of the family (family chat + their direct chats). Chat and message rows
/// stay — history is retained and resurfaces on rejoin per protocol.md.
///
/// Runs INSIDE the caller's transaction, and both callers open that
/// transaction by taking `FOR UPDATE` on the family row. It used to open a
/// transaction of its own, which meant the leave landed with no lock on the
/// family held at all — and `delete_account`, choosing a successor at that
/// exact moment, still saw the departing member as `family_id = F` and
/// handed them the family on their way out of it. A family owned by
/// somebody who is not in it has no working owner endpoint and nothing
/// repairs it, so the write and the lock have to be the same transaction.
async fn remove_membership(
    tx: &mut PgConnection,
    family_id: i64,
    user_id: i64,
) -> Result<(), ApiError> {
    let updated = sqlx::query("UPDATE users SET family_id = NULL WHERE id = $1 AND family_id = $2")
        .bind(user_id)
        .bind(family_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if updated == 0 {
        // They already left in a parallel request; nothing to undo.
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "the user no longer belongs to this family",
        ));
    }
    sqlx::query(
        "DELETE FROM chat_members cm
         USING chats c
         WHERE cm.chat_id = c.id AND c.family_id = $1 AND cm.user_id = $2",
    )
    .bind(family_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

fn member_role(user_id: i64, owner_user_id: i64) -> &'static str {
    if user_id == owner_user_id {
        "owner"
    } else {
        "member"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_role_is_owner_only_for_the_owner_user_id() {
        assert_eq!(member_role(7, 7), "owner");
        assert_eq!(member_role(9, 7), "member");
    }

    #[test]
    fn a_language_outside_the_nine_the_apps_ship_is_refused() {
        for known in FAMILY_LANGUAGES {
            assert_eq!(validate_language(known).expect("known tag"), known);
        }
        assert!(validate_language("klingon").is_err());
        assert!(validate_language("").is_err());
        // Well-formed BCP 47 is not enough: nothing renders it and the
        // family would get answers they could not explain.
        assert!(validate_language("pt-BR").is_err());
        assert!(
            validate_language("en-GB").is_err(),
            "a region is not one of the nine"
        );
    }

    /// BCP 47 casing is a convention rather than a rule, so a client that
    /// spells it its own way is understood — and what comes back is always
    /// the canonical spelling, so a client can compare with `==`.
    #[test]
    fn casing_is_forgiven_on_the_way_in_and_canonical_on_the_way_out() {
        assert_eq!(validate_language("sr-latn").expect("valid"), "sr-Latn");
        assert_eq!(validate_language("SR-LATN").expect("valid"), "sr-Latn");
        assert_eq!(validate_language("ZH-hans").expect("valid"), "zh-Hans");
        assert_eq!(validate_language("  ru  ").expect("valid"), "ru");
    }

    /// The refusal has its own code, like the board's fixed palette, so a
    /// client can tell "not a language we offer" from any other 400.
    #[test]
    fn the_refusal_carries_the_invalid_language_code() {
        let error = validate_language("klingon").expect_err("refused");
        assert!(
            matches!(error, ApiError::BadRequest { code, .. } if code == codes::INVALID_LANGUAGE),
            "a language outside the list is invalid_language, not a bare validation error"
        );
    }

    /// The two layers of the double option are what make "clear it" and
    /// "leave it alone" different requests. Losing the distinction would
    /// leave a family unable to unset a language they had set.
    #[test]
    fn an_absent_language_key_is_not_the_same_request_as_a_null_one() {
        let absent: PatchFamilyRequest =
            serde_json::from_str(r#"{"join_policy": "open"}"#).expect("parses");
        assert_eq!(absent.language, None, "absent means leave it alone");

        let cleared: PatchFamilyRequest =
            serde_json::from_str(r#"{"language": null}"#).expect("parses");
        assert_eq!(cleared.language, Some(None), "null means clear it");
        assert_eq!(cleared.join_policy, None);

        let set: PatchFamilyRequest =
            serde_json::from_str(r#"{"language": "ru"}"#).expect("parses");
        assert_eq!(set.language, Some(Some("ru".to_string())));
    }

    /// The third policy parses like the other two. A unit test on the
    /// deserializer is not enough on its own — nothing here would notice a
    /// missing migration 0030 — but it is what fails first when somebody
    /// narrows the field to an enum of two.
    #[test]
    fn closed_is_a_join_policy_like_any_other() {
        for policy in ["open", "approval", "closed"] {
            let parsed: PatchFamilyRequest =
                serde_json::from_str(&format!(r#"{{"join_policy": "{policy}"}}"#)).expect("parses");
            assert_eq!(parsed.join_policy.as_deref(), Some(policy));
        }
    }

    /// An empty body is a valid no-op rather than a 400: every field is
    /// optional, and "which fields are present decides what happens" has to
    /// survive none of them being present.
    #[test]
    fn an_empty_patch_body_parses() {
        let empty: PatchFamilyRequest = serde_json::from_str("{}").expect("parses");
        assert_eq!(empty.join_policy, None);
        assert_eq!(empty.language, None);
        assert_eq!(empty.max_members, None);
    }

    /// The cap's three states, pinned where a serde regression shows up as
    /// one failing line rather than as a confusing integration failure:
    /// absent leaves it alone, `null` CLEARS it, a number sets it. Absent
    /// and `null` are the pair that a plain `Option<i32>` would collapse.
    #[test]
    fn the_member_cap_parses_as_three_distinct_states() {
        let absent: PatchFamilyRequest = serde_json::from_str("{}").expect("parses");
        assert_eq!(absent.max_members, None, "absent: leave it alone");

        let cleared: PatchFamilyRequest =
            serde_json::from_str(r#"{"max_members": null}"#).expect("parses");
        assert_eq!(cleared.max_members, Some(None), "null: clear it");

        let set: PatchFamilyRequest =
            serde_json::from_str(r#"{"max_members": 12}"#).expect("parses");
        assert_eq!(set.max_members, Some(Some(12)), "a number: set it");
    }
}
