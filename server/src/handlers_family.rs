//! Family endpoints: create, join, membership management, invite codes.
//!
//! Concurrency discipline: membership changes never trust a read-then-write.
//! The single source of truth for "is this user in this family" is a row of
//! `family_members` (migration 0057 — an account may be in several families
//! since 2026-10-22, see docs/protocol.md, "Several families"), and every
//! path that grants membership runs the guarded `INSERT ... ON CONFLICT DO
//! NOTHING` under the family row's lock and then the account row's — zero
//! inserted rows means somebody else won the race and the request gets a
//! clean 409 instead of corrupting state. The approve path additionally
//! takes `FOR UPDATE` on the join request so two owner devices cannot
//! approve the same request twice.
//!
//! Every handler here that is about "the family" reads `AuthUser.family_id`,
//! which is the `{family}` of a `/families/{family}/…` route or, on a route
//! without one, the caller's HOME family — so one handler serves both
//! shapes, and the extractor has already refused a family the caller is not
//! in.

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
use crate::models::{Attachment, Birthday, Family, JoinRequest, Member, User, UserBrief};
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

/// `…/{id}` on a route that may also carry `{family}`. NAMED, never a bare
/// `Path<i64>`: since 2026-10-22 every family endpoint has a
/// `/families/{family}/…` form beside the one without (docs/protocol.md,
/// "Several families"), and a positional extractor would refuse the form
/// with two parameters. serde ignores the `family` it does not name — the
/// `AuthUser` extractor has already resolved it.
#[derive(Debug, Deserialize)]
pub struct IdPath {
    pub id: i64,
}

/// `…/members/{user_id}`, by name, for the reason [`IdPath`] is.
#[derive(Debug, Deserialize)]
pub struct UserPath {
    pub user_id: i64,
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
    /// The third switch, the same shape again — and the one with a rule
    /// between it and its neighbour: `true` is refused while `ai_vision`
    /// is off, and turning `ai_vision` off turns this off in the same write
    /// (protocol.md, "Recent photos from the family chat").
    #[serde(default)]
    pub ai_history_photos: Option<bool>,
    /// The fourth switch, and the one with no rule between it and any
    /// neighbour: whether the assistant may greet the family once a day
    /// unprompted (protocol.md, "The daily greeting"). It is about whether
    /// the assistant speaks, not about what it may be shown, so it is set
    /// and cleared entirely on its own.
    #[serde(default)]
    pub ai_greeting: Option<bool>,
    /// The fifth switch, of the third's exact shape: `true` is refused
    /// while `ai_vision` is off, and turning `ai_vision` off turns this off
    /// in the same write (protocol.md, "Profile pictures of members").
    #[serde(default)]
    pub ai_faces: Option<bool>,
    /// The sixth switch, of `ai_greeting`'s shape: bound to no neighbour,
    /// set and cleared entirely on its own — whether a member may ask for
    /// the text of ANOTHER member's recording in the family chat
    /// (protocol.md, "Transcripts on request").
    #[serde(default)]
    pub ai_transcripts: Option<bool>,
    /// The seventh switch, of `ai_greeting`'s shape again: bound to no
    /// neighbour — whether the assistant may look things up for this family
    /// (protocol.md, "Looking things up").
    #[serde(default)]
    pub ai_lookups: Option<bool>,
    /// The places whose forecast the daily greeting mentions (protocol.md,
    /// "Today's weather, for places the owner chose"). A LIST, and it
    /// replaces the stored one: absent leaves it alone, `[]` clears it — and
    /// `null` is refused, which is why this is a double option like the
    /// language: the outer layer is "was the key sent", and a sent `null`
    /// must be told apart from an absent key to be refused at all. There is
    /// no "unset" for a null to mean; a client clears with `[]`.
    #[serde(default, deserialize_with = "present_option")]
    pub greeting_places: Option<Option<Vec<String>>>,
}

/// The most places a family's greeting mentions the weather for.
pub const MAX_GREETING_PLACES: usize = 3;
/// The longest place name, in characters, once whitespace is folded.
pub const MAX_GREETING_PLACE_CHARS: usize = 80;

/// The owner's place list, as the server will keep it — or `validation`.
///
/// Each name is trimmed with every inner run of whitespace folded to one
/// space (a line break in a settings field is never meant, and the name
/// travels as a URL parameter), must not be empty after that, must hold no
/// control character, and must be at most [`MAX_GREETING_PLACE_CHARS`]. A
/// name equal to an EARLIER one once both are lower-cased is dropped in
/// silence, keeping the first spelling — "Moscow" and "moscow" are one place
/// to the geocoder, and refusing the request over it would be a nuisance
/// rather than a protection. The limit is counted after that.
///
/// The messages name the POSITION of a bad name, never the name: a place
/// name is the owner's words and the one thing this list sends somewhere, so
/// it is kept out of every error and every log.
pub(crate) fn validate_greeting_places(requested: &[String]) -> Result<Vec<String>, ApiError> {
    let mut kept: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (index, raw) in requested.iter().enumerate() {
        let position = index + 1;
        let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            return Err(ApiError::validation(format!(
                "greeting place {position} is empty"
            )));
        }
        if name.chars().any(char::is_control) {
            return Err(ApiError::validation(format!(
                "greeting place {position} contains a control character"
            )));
        }
        if name.chars().count() > MAX_GREETING_PLACE_CHARS {
            return Err(ApiError::validation(format!(
                "greeting place {position} is longer than {MAX_GREETING_PLACE_CHARS} characters"
            )));
        }
        let folded = name.to_lowercase();
        if seen.contains(&folded) {
            continue;
        }
        seen.push(folded);
        kept.push(name);
    }
    if kept.len() > MAX_GREETING_PLACES {
        return Err(ApiError::validation(format!(
            "greeting_places may name at most {MAX_GREETING_PLACES} places"
        )));
    }
    Ok(kept)
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
    ai_history_photos: bool,
    ai_greeting: bool,
    ai_faces: bool,
    ai_transcripts: bool,
    ai_lookups: bool,
    greeting_places: Vec<String>,
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
            ai_history_photos: row.get("ai_history_photos"),
            ai_greeting: row.get("ai_greeting"),
            ai_faces: row.get("ai_faces"),
            ai_transcripts: row.get("ai_transcripts"),
            ai_lookups: row.get("ai_lookups"),
            greeting_places: row.get("greeting_places"),
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
            // And the third, for the reason that tops the other three: it
            // decides whether a member's photographs may leave WITHOUT
            // anybody pointing the assistant at them. Every member gets to
            // know that before they send one; only the owner can change it.
            ai_history_photos: self.ai_history_photos,
            // Not owner-gated either. This one is not about a member's own
            // words or pictures at all, so the reasoning that governs the
            // three above does not reach it — but the same principle does:
            // every member gets to know why a message from the assistant
            // appears in their chat each morning, even though only the owner
            // could have asked for it.
            ai_greeting: self.ai_greeting,
            // Not owner-gated, and for the strongest reason on this list: it
            // decides whether a member's own FACE may leave the server.
            // Every member gets to know that; only the owner can change it.
            ai_faces: self.ai_faces,
            // Not owner-gated either: it decides whether a member's own
            // recorded VOICE may be sent to a speech model at somebody
            // else's request. Every member gets to know that before they
            // send a voice note; only the owner can change it.
            ai_transcripts: self.ai_transcripts,
            // Not owner-gated either: it decides whether a query written
            // from a member's words may reach a party that is not
            // `processor`. Every member gets to know that; only the owner
            // can change it.
            ai_lookups: self.ai_lookups,
            // Not owner-gated either: these names are what the greeting
            // sends to the weather provider. Every member gets to know what
            // leaves; only the owner can change it.
            greeting_places: self.greeting_places.clone(),
        }
    }
}

const SELECT_FAMILY: &str = "SELECT id, name, invite_code, join_policy, language, max_members,
                             ai_history, ai_vision, ai_history_photos, ai_greeting, ai_faces,
                             ai_transcripts, ai_lookups, greeting_places, owner_user_id,
                             created_at
                             FROM families";

async fn fetch_family(state: &AppState, family_id: i64) -> Result<FamilyRecord, ApiError> {
    let row = sqlx::query(&format!("{SELECT_FAMILY} WHERE id = $1"))
        .bind(family_id)
        .fetch_one(&state.pool)
        .await?;
    Ok(FamilyRecord::from_row(&row))
}

/// One family as the API object, as `caller` may see it — the invite code
/// for its owner alone. For the endpoints outside this module that answer
/// with a `Family` (accepting an invitation, `GET /me`'s list).
pub(crate) async fn family_api(
    state: &AppState,
    family_id: i64,
    caller: i64,
) -> Result<Family, ApiError> {
    let family = fetch_family(state, family_id).await?;
    let is_owner = family.owner_user_id == caller;
    Ok(family.to_api(is_owner))
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
/// Several states rather than a `bool`, because each failure needs its own
/// words: somebody who is already in THIS family is told so, somebody
/// arriving at a full one is told THAT, and somebody already in as many
/// families as an account may be is told that — and a caller cannot tell
/// them apart from a single `false`.
#[derive(Debug, PartialEq)]
pub(crate) enum Granted {
    /// In. Carries the invitation into this family that the grant LAPSED,
    /// if there was one — the person came in by another door — so the
    /// caller can tell both parties once the transaction has committed
    /// (docs/protocol.md, "Several families").
    Yes(Option<LapsedInvitation>),
    /// Already a member of this family — or a tombstone, which a grant
    /// must never land on (see `grant_membership`).
    AlreadyInAFamily,
    FamilyFull,
    /// At `[families] max_families_per_account` already.
    TooManyFamilies,
}

/// An invitation that went away inside a transaction, for the
/// `invitation_removed` frames sent once it has committed.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct LapsedInvitation {
    pub id: i64,
    pub family_id: i64,
    pub user_id: i64,
}

/// Tell the invited person and the family's owner that these invitations
/// are gone. After the commit, and never failing the request.
pub(crate) async fn announce_lapsed(state: &AppState, lapsed: &[LapsedInvitation]) {
    for invitation in lapsed {
        let owner: Option<i64> =
            sqlx::query_scalar("SELECT owner_user_id FROM families WHERE id = $1")
                .bind(invitation.family_id)
                .fetch_optional(&state.pool)
                .await
                .unwrap_or(None);
        events::log_fanout_error(
            "invitation_removed",
            events::deliver_invitation_removed(
                state,
                invitation.id,
                invitation.family_id,
                invitation.user_id,
                owner,
            )
            .await,
        );
    }
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
            AND u.deleted_at IS NULL
            AND EXISTS (SELECT 1 FROM family_members fm
                         WHERE fm.family_id = $1 AND fm.user_id = cm.user_id)
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
    let head_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM family_members WHERE family_id = $1")
            .bind(family_id)
            .fetch_one(&mut *conn)
            .await?;
    Ok(Some(head_count >= cap))
}

/// Grant `user_id` membership of `family_id` inside `tx`.
///
/// Runs the race-proof guarded insert, joins the family chat, closes the user's pending join request to THIS family (they are in; a
/// later approval would only 409), and lapses an invitation into it.
///
/// Their requests to OTHER families are left alone: since 2026-10-22 an
/// account may be in several, and may have requests pending to several at
/// once (docs/protocol.md, "Several families"). It used to reject them all,
/// because a family was a one-per-account fact.
///
/// This is the ONLY place membership is granted — `create_family`,
/// `join_family`, `approve_join_request` and accepting an invitation all
/// come through here — which is why both caps, the family's and the
/// account's, are enforced here rather than at each door.
pub(crate) async fn grant_membership(
    tx: &mut PgConnection,
    family_id: i64,
    user_id: i64,
    ceiling: i64,
    max_families: i64,
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

    // Then the ACCOUNT's row — the lock order everything in this server
    // observes, family row first, then user row — and it does two jobs.
    //
    // It refuses a TOMBSTONE. The scrub — a deletion the person asked for,
    // or the familyless sweep — removes every membership, and a join that
    // was already past authentication when the scrub committed would
    // otherwise find no membership in the way and make a "Deleted account"
    // with `password_hash = '!'` a member of the family. The session it
    // rode in on is gone, so the caller learns nothing either way;
    // refusing as `AlreadyInAFamily` is the nearest honest answer this
    // helper has.
    //
    // And it serializes the per-account CAP: two joins into two different
    // families by one account hold two different family locks, so only
    // this one row makes "count my families, then add one" a single
    // decision.
    let alive: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if alive.is_none() {
        return Ok(Granted::AlreadyInAFamily);
    }
    if crate::families::is_member(&mut *tx, family_id, user_id).await? {
        return Ok(Granted::AlreadyInAFamily);
    }
    let families: i64 =
        sqlx::query_scalar("SELECT count(*) FROM family_members WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
    if families >= max_families {
        return Ok(Granted::TooManyFamilies);
    }

    let inserted = sqlx::query(
        "INSERT INTO family_members (family_id, user_id) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(family_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(Granted::AlreadyInAFamily);
    }
    // `familyless_since` stops here (0034): a member's clock is not running.
    sqlx::query("UPDATE users SET familyless_since = NULL WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
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
         WHERE user_id = $1 AND family_id = $2 AND status = 'pending'",
    )
    .bind(user_id)
    .bind(family_id)
    .execute(&mut *tx)
    .await?;
    // An invitation into this family lapses — "when the invited person joins
    // the family by another door" — and the accept path deletes its own the
    // same way, so this one statement is every door's.
    let lapsed: Option<i64> = sqlx::query_scalar(
        "DELETE FROM invitations WHERE family_id = $1 AND user_id = $2 RETURNING id",
    )
    .bind(family_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    reopen_direct_chats(&mut *tx, family_id, user_id).await?;
    Ok(Granted::Yes(lapsed.map(|id| LapsedInvitation {
        id,
        family_id,
        user_id,
    })))
}

/// SQL for "may `owner` reset `member`'s password": true exactly when
/// `owner` owns every family `member` is in (docs/protocol.md, "Several
/// families"; D7). Both arguments are SQL expressions, and neither may name
/// an outer `other` or `f` — the subquery's own aliases would shadow it. One
/// boolean, so no other family's id ever leaves the server.
pub(crate) fn may_reset_sql(member: &str, owner: &str) -> String {
    format!(
        "NOT EXISTS (SELECT 1 FROM family_members other
                       JOIN families f ON f.id = other.family_id
                      WHERE other.user_id = {member}
                        AND f.owner_user_id <> {owner})"
    )
}

pub(crate) async fn user_brief(state: &AppState, user_id: i64) -> Result<UserBrief, ApiError> {
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
///
/// Two doors share this endpoint since 2026-10-22 (docs/protocol.md,
/// "Several families"): an account with NO family creates its first one
/// under `[families] registration`, exactly as before, and an account
/// already in one starts ANOTHER under `[families] member_families` — only
/// while it owns none, and under the per-account cap.
pub async fn create_family(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<CreateFamilyRequest>,
) -> Result<Response, ApiError> {
    let first_family = auth.family_ids.is_empty();
    // The operator's door, before the name is even looked at: a server
    // run for one family may be closed to new ones (docs/protocol.md,
    // "Starting a family"). Joining the families already here is not
    // touched by this — that is what the server is for.
    //
    // WHICH door depends on who is knocking. `registration` is about a
    // stranger's first family; a member starting another is the separate
    // `member_families`, so a server closed to strangers can still let its
    // members start theirs — the case issue #88 describes.
    if first_family && !state.cfg.families.registration {
        return Err(ApiError::forbidden(
            codes::FAMILY_REGISTRATION_DISABLED,
            "this server does not take new families",
        ));
    }
    if !first_family && !state.cfg.families.member_families {
        return Err(ApiError::forbidden(
            codes::MEMBER_FAMILIES_DISABLED,
            "this server does not let members start families",
        ));
    }
    let name = req.name.trim().to_string();
    let name_len = name.chars().count();
    if !(1..=64).contains(&name_len) {
        return Err(ApiError::validation("family name must be 1-64 characters"));
    }
    let max_families = state.cfg.families.max_families_per_account;

    // Invite codes have 30^8 combinations, so a unique-index collision is
    // effectively impossible — but "effectively" isn't "actually", and the
    // retry costs three lines.
    for _attempt in 0..5 {
        let invite_code = tokens::gen_invite_code();
        let mut tx = state.pool.begin().await?;

        // The account's own row first: "owns none" and "under the cap" are
        // read under it, so two creates from two devices at once cannot
        // both see "owns none" and both make a family. Taking it before the
        // family insert keeps the lock order (an existing family's row,
        // then a user row) — the family about to be inserted is invisible
        // to everybody else, so it cannot be anybody's first lock.
        let owns: Option<bool> = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM families WHERE owner_user_id = users.id)
               FROM users WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
        )
        .bind(auth.user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(owns) = owns else {
            // Scrubbed while this request was in flight: its session is gone.
            return Err(ApiError::unauthorized());
        };
        let held: i64 =
            sqlx::query_scalar("SELECT count(*) FROM family_members WHERE user_id = $1")
                .bind(auth.user_id)
                .fetch_one(&mut *tx)
                .await?;
        // An account may CREATE a family only while it owns none — so the
        // married son owns his one family, and a second needs the first
        // handed on or deleted. Ownership that came by hand-off counts
        // (decision D3). An account with no family owns none by definition,
        // so this never touches the first door.
        if owns {
            return Err(ApiError::conflict(
                codes::ALREADY_OWN_A_FAMILY,
                "you already own a family",
            ));
        }
        if held >= max_families {
            return Err(ApiError::conflict(
                codes::TOO_MANY_FAMILIES,
                "you are already in as many families as an account may be",
            ));
        }

        let inserted = sqlx::query(
            "INSERT INTO families (name, invite_code, owner_user_id)
             VALUES ($1, $2, $3)
             RETURNING id, name, invite_code, join_policy, language, max_members, ai_history,
                       ai_vision, ai_history_photos, ai_greeting, ai_faces, ai_transcripts,
                       ai_lookups, greeting_places, owner_user_id, created_at",
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
            max_families,
        )
        .await?
        {
            Granted::Yes(_) => {}
            // A family one statement old holds nobody, so "already a
            // member" can only mean the account was scrubbed in between.
            Granted::AlreadyInAFamily => {
                tx.rollback().await?;
                return Err(ApiError::unauthorized());
            }
            Granted::FamilyFull => {
                tx.rollback().await?;
                return Err(ApiError::conflict(
                    codes::FAMILY_FULL,
                    "this family is full",
                ));
            }
            // Counted above under the same lock, so unreachable; answered
            // as the door's own truth rather than a 500.
            Granted::TooManyFamilies => {
                tx.rollback().await?;
                return Err(ApiError::conflict(
                    codes::TOO_MANY_FAMILIES,
                    "you are already in as many families as an account may be",
                ));
            }
        }
        tx.commit().await?;
        refresh_home(&state, auth.user_id).await;

        // A family started from inside the app is news to the creator's
        // OTHER devices — "a family just created on another device" is one
        // of the cases a client reads a `member_joined` naming itself as
        // "you are in a new family" (docs/protocol.md, "Several families").
        // A first family announces itself the same way it always did: not
        // at all, because the creating device is the only one that can
        // hold an account with no family.
        if !first_family {
            let joined = user_brief(&state, auth.user_id).await?;
            events::log_fanout_error(
                "member_joined",
                events::deliver_member_joined(&state, family.id, joined).await,
            );
        }

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

    // "Already in a family" means already in THIS one since 2026-10-22: an
    // account may join another with a code, under that family's own rules
    // (docs/protocol.md, "Several families"; decision D5). Still second in
    // the documented order, and the per-account cap rides beside it — a
    // member who could not get in anyway is told why before a request is
    // queued that no approval could ever satisfy.
    if auth.in_family(family.id) {
        return Err(ApiError::conflict(
            codes::ALREADY_IN_FAMILY,
            "you already belong to this family",
        ));
    }
    let max_families = state.cfg.families.max_families_per_account;
    if auth.family_ids.len() as i64 >= max_families {
        return Err(ApiError::conflict(
            codes::TOO_MANY_FAMILIES,
            "you are already in as many families as an account may be",
        ));
    }

    if family.join_policy == "open" {
        let mut tx = state.pool.begin().await?;
        let lapsed = match grant_membership(
            &mut tx,
            family.id,
            auth.user_id,
            state.cfg.limits.max_family_members,
            max_families,
        )
        .await?
        {
            Granted::Yes(lapsed) => lapsed,
            Granted::AlreadyInAFamily => {
                tx.rollback().await?;
                return Err(ApiError::conflict(
                    codes::ALREADY_IN_FAMILY,
                    "you already belong to this family",
                ));
            }
            Granted::FamilyFull => {
                tx.rollback().await?;
                return Err(ApiError::conflict(
                    codes::FAMILY_FULL,
                    "this family is full",
                ));
            }
            Granted::TooManyFamilies => {
                tx.rollback().await?;
                return Err(ApiError::conflict(
                    codes::TOO_MANY_FAMILIES,
                    "you are already in as many families as an account may be",
                ));
            }
        };
        tx.commit().await?;

        announce_lapsed(&state, lapsed.as_slice()).await;
        announce_joined(&state, family.id, user_brief(&state, auth.user_id).await?).await;
        // `family_id` on both answers since 2026-10-22: a client in several
        // families has to know WHICH one it joined or is waiting on
        // (docs/protocol.md, `POST /families/join`).
        return Ok((
            StatusCode::OK,
            Json(json!({"status": "joined", "family_id": family.id})),
        )
            .into_response());
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
    // Still a live account, read under a share lock on its own row. The
    // familyless sweep scrubs under FOR UPDATE, and a request that was past
    // authentication when the scrub committed would otherwise ride the
    // INSERT's own FK lock — which waits for the scrub and then proceeds —
    // into a `pending` request from "Deleted account" that the owner is
    // pushed about. FOR SHARE waits the same way but is re-evaluated after
    // the wait, so it sees the tombstone. The session is gone with the
    // account, so the caller is answered as one whose session is gone.
    let alive: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR SHARE")
            .bind(auth.user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if alive.is_none() {
        return Err(ApiError::unauthorized());
    }
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
    Ok((
        StatusCode::OK,
        Json(json!({"status": "pending", "family_id": family.id})),
    )
        .into_response())
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
    let is_owner = family.owner_user_id == auth.user_id;
    // `may_reset` rides along for the OWNER's read alone: whether this owner
    // owns every family the member is in, which is what a password reset
    // needs (docs/protocol.md, "Several families"; D7). One boolean per
    // row, computed here so no other family's id ever leaves the server.
    let rows = sqlx::query(&format!(
        "SELECT u.id, u.username, u.display_name, u.avatar_version,
                u.birthday_month, u.birthday_day,
                {} AS may_reset
         FROM family_members fm
         JOIN users u ON u.id = fm.user_id
         WHERE fm.family_id = $1
         ORDER BY u.id",
        may_reset_sql("u.id", "$2")
    ))
    .bind(family_id)
    .bind(auth.user_id)
    .fetch_all(&state.pool)
    .await?;
    let members: Vec<Member> = rows
        .iter()
        .map(|row| {
            let mut member = member_from_row(row, family.owner_user_id);
            // On EVERY row of the owner's read, `false` included: a client
            // reads an absent flag as "a server that predates it, the old
            // rule holds" (docs/protocol.md, `Member`). The owner's own row
            // is `false` — a reset of oneself is `POST /me/password`, and
            // the endpoint refuses it here.
            if is_owner {
                member.password_reset =
                    Some(member.id != auth.user_id && row.get::<bool, _>("may_reset"));
            }
            member
        })
        .collect();
    // The accounts that were deleted while in THIS family, which is what
    // `former_family_members` remembers and its only reader (0057; one row
    // per family the account was in, since an account may have been in
    // several). They are not members — no role, offered to nobody, counted
    // by nothing — and they are here for one reason: their messages, notes
    // and reactions are still in the family chat and a client has to put a
    // name to them (protocol.md, "Deleting an account"). Nothing else in
    // this server needs excluding them, because the scrub removed their
    // memberships and every membership question asks `family_members`.
    let former_rows = sqlx::query(
        "SELECT u.id, u.username, u.display_name, u.avatar_version,
                u.birthday_month, u.birthday_day, u.deleted_at
         FROM former_family_members ffm
         JOIN users u ON u.id = ffm.user_id
         WHERE ffm.family_id = $1 AND u.deleted_at IS NOT NULL
         ORDER BY u.id",
    )
    .bind(family_id)
    .fetch_all(&state.pool)
    .await?;
    let former_members: Vec<Member> = former_rows
        .iter()
        .map(|row| member_from_row(row, family.owner_user_id))
        .collect();
    // The board cursor rides along: this is the call every client already
    // makes on resync, so learning whether a board catch-up is needed
    // costs no extra request. Omitted while the board has never been
    // written to, like max_reaction_seq on an unreacted chat.
    // The sticker pack's cursor rides beside it, for the same reason and
    // under the same rule (protocol.md, "Sticker pack").
    let (last_board_seq, last_pack_seq): (i64, i64) =
        sqlx::query_as("SELECT last_board_seq, last_pack_seq FROM families WHERE id = $1")
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
    if last_pack_seq > 0 {
        body["max_pack_seq"] = json!(last_pack_seq);
    }
    // The pack's two ceilings, ALWAYS present: a client refuses an oversized
    // picture beside the picker rather than after an upload, and — because
    // a server that predates the pack sends neither — reads their absence
    // as "no stickers here" and offers no button that would answer 404
    // (protocol.md, "Sticker pack").
    body["max_pack_items"] = json!(state.cfg.limits.max_pack_items);
    body["max_pack_item_bytes"] = json!(state.cfg.limits.max_pack_item_bytes);
    // A video message's two limits, ALWAYS present on a server that has
    // them, for the pack's reason: a server that predates them sends
    // neither, and ignores `round` on a send — so their absence is how a
    // client knows to offer no way of recording one (protocol.md, "Video
    // messages"). The byte key is the ceiling IN FORCE, the same accessor
    // the claim checks, so the client's refusal and the server's agree.
    body["max_round_video_ms"] = json!(Attachment::ROUND_VIDEO_MAX_MS);
    body["max_round_video_bytes"] = json!(state.cfg.limits.round_video_bytes());
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
            // WHO ANSWERS, in the operator's own words, and the reason a
            // client can ask permission at all: an app that cannot say
            // where a message goes must not send it (protocol.md,
            // "Consenting to the assistant"). `is_usable()` already
            // requires it, so this is never empty when the object exists.
            "processor": state.cfg.ai.processor,
            "vision": state.cfg.ai.vision_usable(),
            "images": state.cfg.ai.images_usable(),
            // The fourth deployment, discovered the same way (protocol.md,
            // "Transcripts on request"): whether this SERVER can turn a
            // recording into text. Whether a member may ask for a given
            // recording is decided per request, not here.
            "transcribe": state.cfg.ai.transcribe_usable(),
        });
        // The sources the assistant may look things up in, by the names a
        // client shows on the consent screen (protocol.md, "Looking things
        // up"). ABSENT when there are none, never `[]`: absence is the
        // capability check, as it is for the whole object.
        if state.cfg.ai.lookups_usable() {
            body["assistant"]["lookups"] = json!(state.cfg.ai.lookups.source_names());
        }
        // Whether the daily greeting can carry the weather for the family's
        // `greeting_places` (protocol.md, "Today's weather, for places the
        // owner chose"). ALWAYS present beside `vision` and `transcribe`, a
        // boolean rather than an absence: a client draws the places field
        // beside the greeting switch only when it is true, and a family's
        // list on a server where it is false is kept and does nothing.
        body["assistant"]["greeting_weather"] =
            json!(crate::greetings::weather_available(&state.cfg));
        // The ceiling rides only beside a `true`: a number for a capability
        // the server does not have would be a promise about nothing.
        if state.cfg.ai.transcribe_usable() {
            body["assistant"]["transcribe_max_bytes"] = json!(state.cfg.ai.transcribe.max_bytes);
        }
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
/// language, the cap, the three assistant switches, or any combination.
///
/// Which fields are PRESENT decides what changes, exactly as on a board
/// note. Everything is validated before anything is written, so a request
/// naming a good policy and a bad language changes neither and the owner is
/// never left guessing which half of it landed.
///
/// One rule binds two of the switches (protocol.md, "Recent photos from
/// the family chat"): `ai_history_photos` may only be on while `ai_vision`
/// is. Turning it on otherwise is `validation`; turning `ai_vision` off
/// turns it off in the same write, whether or not the request said so.
/// The check needs the row, so it is the one validation that runs after
/// `require_owner` — still before anything is written.
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

    // The place list: `null` is refused rather than read as "leave it", and
    // a list is checked whole before anything is written.
    let greeting_places = match &req.greeting_places {
        None => None,
        Some(None) => {
            return Err(ApiError::validation(
                "greeting_places must be an array; send [] to clear it",
            ));
        }
        Some(Some(requested)) => Some(validate_greeting_places(requested)?),
    };

    let family = require_owner(&state, &auth).await?;
    // The third switch may only be on while `ai_vision` is, judged against
    // what `ai_vision` WILL be once this same request has applied: turning
    // both on at once is fine, turning `ai_vision` off and this on is not,
    // and turning this on alone while `ai_vision` is off is the case an
    // owner most needs an answer to rather than a silent no. Refused here
    // with the reason; and enforced AGAIN in the UPDATE below, so a racing
    // request cannot write the state migration 0033's CHECK forbids.
    let vision_after = req.ai_vision.unwrap_or(family.ai_vision);
    if req.ai_history_photos == Some(true) && !vision_after {
        return Err(ApiError::validation(
            "ai_history_photos can only be turned on while ai_vision is on",
        ));
    }
    // The fifth switch, under the same rule as the third and for the same
    // reason (protocol.md, "Profile pictures of members").
    if req.ai_faces == Some(true) && !vision_after {
        return Err(ApiError::validation(
            "ai_faces can only be turned on while ai_vision is on",
        ));
    }
    // COALESCE for the policy and for the switches, because an unsent field
    // binds NULL and the column should keep what it had — which is exactly
    // what COALESCE says. The language cannot use it — NULL is a VALUE
    // there rather than "unchanged" — so a separate flag says whether to
    // write that column at all.
    //
    // `ai_history_photos` is what was asked (or what it was) AND what
    // `ai_vision` is about to be. That one expression is both halves of
    // the rule at once: `ai_vision` going off takes it down in the same
    // write, whether or not the request mentioned it, and nothing this
    // handler can be sent leaves it on over a shut `ai_vision`.
    //
    // `ai_greeting` is a plain COALESCE with no second term, and the
    // difference is the point: it is bound to none of the other three, so
    // "what was asked, or else what it was" is the whole rule. Nothing here
    // may clear it as a side effect (protocol.md, "The daily greeting").
    // `ai_transcripts` is the same plain COALESCE, for the same reason
    // (protocol.md, "Transcripts on request"), and so is `ai_lookups`
    // (protocol.md, "Looking things up"). `greeting_places` is a COALESCE
    // too, over an already-validated list: absent binds NULL and keeps the
    // stored list, and `[]` is a value, which clears it.
    let row = sqlx::query(
        "UPDATE families
         SET join_policy = COALESCE($2, join_policy),
             language = CASE WHEN $3 THEN $4 ELSE language END,
             ai_history = COALESCE($5, ai_history),
             ai_vision = COALESCE($8, ai_vision),
             ai_history_photos = COALESCE($9, ai_history_photos) AND COALESCE($8, ai_vision),
             ai_greeting = COALESCE($10, ai_greeting),
             ai_faces = COALESCE($11, ai_faces) AND COALESCE($8, ai_vision),
             ai_transcripts = COALESCE($12, ai_transcripts),
             ai_lookups = COALESCE($13, ai_lookups),
             greeting_places = COALESCE($14, greeting_places),
             max_members = CASE WHEN $6 THEN $7 ELSE max_members END
         WHERE id = $1
         RETURNING id, name, invite_code, join_policy, language, max_members, ai_history,
                   ai_vision, ai_history_photos, ai_greeting, ai_faces, ai_transcripts,
                   ai_lookups, greeting_places, owner_user_id, created_at",
    )
    .bind(family.id)
    .bind(join_policy)
    .bind(language.is_some())
    .bind(language.flatten())
    .bind(req.ai_history)
    .bind(max_members.is_some())
    .bind(max_members.flatten())
    .bind(req.ai_vision)
    .bind(req.ai_history_photos)
    .bind(req.ai_greeting)
    .bind(req.ai_faces)
    .bind(req.ai_transcripts)
    .bind(req.ai_lookups)
    .bind(greeting_places)
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
    Path(IdPath { id: request_id }): Path<IdPath>,
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

    let lapsed = match grant_membership(
        &mut tx,
        family.id,
        applicant_id,
        state.cfg.limits.max_family_members,
        state.cfg.families.max_families_per_account,
    )
    .await?
    {
        Granted::Yes(lapsed) => lapsed,
        Granted::AlreadyInAFamily => {
            // The applicant got into this family by another door in the
            // meantime — an invitation, an open-policy moment — or the
            // account is gone. The request can never be satisfied, so it is
            // closed as rejected — committed, not rolled back — before
            // reporting the conflict.
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
        // The applicant reached the per-account cap since they asked —
        // another family let them in. The same disposition as `full`, for
        // the same reason: it is a fact about today, somebody can leave a
        // family, and the owner's yes should not be thrown away by the
        // server. The request stays PENDING (docs/protocol.md, "Several
        // families": the cap is read at a join, as `family_full` is).
        Granted::TooManyFamilies => {
            tx.rollback().await?;
            return Err(ApiError::conflict(
                codes::TOO_MANY_FAMILIES,
                "this person is already in as many families as an account may be",
            ));
        }
    };
    // grant_membership closed this request along with any duplicate; this
    // one becomes the approved record.
    sqlx::query(
        "UPDATE join_requests SET status = 'approved', decided_at = now(), decided_by = $2
         WHERE id = $1",
    )
    .bind(request_id)
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // ONE read for the answer and the frame: the brief, the birthday, and
    // whether this owner may reset the newcomer's password — on the answer
    // exactly as on the owner's roster, because a client reads an absent
    // flag as "the old rule holds" and would offer a reset the server then
    // refuses (docs/protocol.md, `Member`). The birthday never travels in
    // the `member_joined` frame, whose payload is the brief alone.
    let row = sqlx::query(&format!(
        "SELECT u.id, u.username, u.display_name, u.avatar_version,
                u.birthday_month, u.birthday_day,
                {} AS may_reset
           FROM users u WHERE u.id = $1",
        may_reset_sql("u.id", "$2")
    ))
    .bind(applicant_id)
    .bind(auth.user_id)
    .fetch_one(&state.pool)
    .await?;
    // An account that has just been approved into a family is live by
    // construction — a tombstone holds no family and makes no requests —
    // which is what `member_from_row` reads off a row without `deleted_at`.
    let mut member = member_from_row(&row, family.owner_user_id);
    member.password_reset = Some(row.get::<bool, _>("may_reset"));
    let joined = UserBrief {
        id: member.id,
        username: member.username.clone(),
        display_name: member.display_name.clone(),
        avatar_version: member.avatar_version,
    };
    announce_lapsed(&state, lapsed.as_slice()).await;
    announce_joined(&state, family.id, joined).await;
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
    Path(IdPath { id: request_id }): Path<IdPath>,
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

/// End the sessions of `user_id` that never said they know about several
/// families, when the departure just written moved the account's HOME family
/// — `was_home`, and another family still left to become the new home. Inside
/// the caller's transaction, after the membership write; answers the session
/// ids, whose sockets the caller closes once it has committed
/// ([`close_ended_sessions`]).
///
/// An app from before several families keeps one family's board, pack, chats
/// and cursors and has exactly two ways to learn they are no longer its own: a
/// live `member_left`, or `GET /me` finding no family at all. An app that was
/// OFFLINE when its home family went, while the account stayed in another,
/// gets neither — `/me` hands it a different family and it goes on using the
/// old one's data and cursors as the new one's. Signing it out is the one
/// thing every such app already answers by wiping and starting clean
/// (docs/protocol.md, "Several families"). Deleting the sessions takes their
/// device rows, and so their push tokens, with them (0021).
async fn end_legacy_sessions_if_home_moved(
    tx: &mut PgConnection,
    user_id: i64,
    was_home: bool,
) -> Result<Vec<i64>, ApiError> {
    if !was_home {
        return Ok(Vec::new());
    }
    if crate::families::family_ids_of(&mut *tx, user_id)
        .await?
        .is_empty()
    {
        // No family left: `/me` says so, and that app wipes on it already.
        return Ok(Vec::new());
    }
    let ended: Vec<i64> = sqlx::query_scalar(
        "DELETE FROM sessions WHERE user_id = $1 AND NOT multi_family RETURNING id",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    Ok(ended)
}

/// Close the sockets of sessions [`end_legacy_sessions_if_home_moved`] ended,
/// with 4401 — "your session no longer exists", which sends the app to its
/// sign-in screen with its local state wiped.
async fn close_ended_sessions(state: &AppState, session_ids: &[i64]) {
    for &session_id in session_ids {
        state.registry.close_session(session_id).await;
    }
}

/// `POST /families/leave`
///
/// Since 2026-10-22 also `POST /families/{family}/leave`: either form leaves
/// ONE family — the named one, or the home family — and the account's other
/// families are untouched (docs/protocol.md, "Several families").
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
    // Decided before the write, because the write is what moves it: the
    // departed account's app from before several families is told it left
    // exactly when this was the family it was showing.
    let was_home = auth.home_family() == Some(family_id);

    // The ownership question, the head count and the delete are ONE
    // decision, so they are one transaction with the family row locked.
    // They used to be two pool round-trips with nothing between them, and
    // under `join_policy = 'open'` somebody could join in that gap and find
    // their new family deleted underneath them. `FOR UPDATE` is what closes
    // it rather than the transaction alone: `grant_membership` takes this
    // same row FOR UPDATE before it inserts a membership, so a join in
    // flight either lands before the count sees it, or waits and then fails
    // against a family that is gone.
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
            let departure = remove_membership(&mut tx, family_id, auth.user_id).await?;
            let ended = end_legacy_sessions_if_home_moved(&mut tx, auth.user_id, was_home).await?;
            tx.commit().await?;
            announce_departure(&state, family_id, auth.user_id, departure, was_home).await;

            // The successor is told FIRST, so no client momentarily holds a
            // family whose `owner_user_id` names nobody in `members`.
            events::log_fanout_error(
                "family_owner",
                events::deliver_family_owner(&state, family_id, new_owner_user_id).await,
            );
            events::log_fanout_error(
                "member_left",
                events::deliver_member_left(&state, family_id, auth.user_id, was_home).await,
            );
            close_ended_sessions(&state, &ended).await;
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
        let remaining: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM family_members WHERE family_id = $1 AND user_id <> $2",
        )
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
                 a membership names the family with no family-chat membership"
            )));
        }

        // Sole member: delete the family. Cascades remove chats, messages,
        // the membership itself, join requests and invitations. Nobody is
        // left to notify but the invited.
        //
        // The cascade removes the membership and nothing else, so the
        // familyless clock is started HERE, by hand — the one departure that
        // does not go through `remove_membership` — and, like there, only
        // when this was the account's LAST family. Without it the row would
        // be {no family, familyless_since NULL}: alive, alone, and invisible
        // to the sweep for good, while `/me` went on promising the grace
        // (docs/protocol.md, "Accounts without a family").
        sqlx::query(
            "UPDATE users SET familyless_since = now()
              WHERE id = $1
                AND NOT EXISTS (SELECT 1 FROM family_members
                                 WHERE user_id = $1 AND family_id <> $2)",
        )
        .bind(auth.user_id)
        .bind(family_id)
        .execute(&mut *tx)
        .await?;
        // The chats the leaver holds after the other side left, whose row
        // names this family: re-pointed before the cascade could take them.
        let held = settle_held_closed_chats(&mut tx, family_id, auth.user_id).await?;
        let deleted = delete_family_in_tx(&mut tx, family_id).await?;
        let ended = end_legacy_sessions_if_home_moved(&mut tx, auth.user_id, was_home).await?;
        tx.commit().await?;
        refresh_home(&state, auth.user_id).await;
        announce_deleted_family_invitations(&state, &deleted.invitations).await;
        announce_direct_chats(&state, &held).await;
        // The account's OTHER devices are told the family is gone the way a
        // departure always tells them — nobody else is left to tell.
        events::log_fanout_error(
            "member_left",
            events::deliver_member_left(&state, family_id, auth.user_id, was_home).await,
        );
        close_ended_sessions(&state, &ended).await;
        let storage_keys = deleted.storage_keys;
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
    let departure = remove_membership(&mut tx, family_id, auth.user_id).await?;
    let ended = end_legacy_sessions_if_home_moved(&mut tx, auth.user_id, was_home).await?;
    tx.commit().await?;
    announce_departure(&state, family_id, auth.user_id, departure, was_home).await;

    events::log_fanout_error(
        "member_left",
        events::deliver_member_left(&state, family_id, auth.user_id, was_home).await,
    );
    close_ended_sessions(&state, &ended).await;
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
    Path(UserPath {
        user_id: target_user_id,
    }): Path<UserPath>,
) -> Result<Response, ApiError> {
    // The same rule with or without a family in the path: a block is a
    // pair, so `/families/{id}/members/{user_id}/block` means exactly what
    // the form without an id does — neither is narrowed to the named or the
    // home family (docs/protocol.md, `PUT /families/members/{user_id}/block`).
    // The extractor has already refused a family the caller is not in, as
    // it does on every `{family}` route.
    if auth.family_ids.is_empty() {
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    }
    if target_user_id == auth.user_id {
        return Err(ApiError::bad_request(
            codes::CANNOT_BLOCK_SELF,
            "you cannot block yourself",
        ));
    }
    // ANY family the two share, not only the one in the path: a block is a
    // pair, it holds in every family, and a member must be able to block
    // somebody from whichever family they meet them in (docs/protocol.md,
    // "Several families"). Same refusal for a stranger and for an id nobody
    // holds, as before.
    if !crate::families::share_a_family(&state.pool, auth.user_id, target_user_id).await? {
        return Err(ApiError::forbidden(
            codes::NOT_SAME_FAMILY,
            "no such member in your family",
        ));
    }

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
    Path(UserPath {
        user_id: target_user_id,
    }): Path<UserPath>,
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
    Path(UserPath {
        user_id: target_user_id,
    }): Path<UserPath>,
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
    if !crate::families::is_member(&mut *tx, family.id, target_user_id).await? {
        return Err(ApiError::not_found(
            codes::USER_NOT_FOUND,
            "no such member in your family",
        ));
    }
    // Their home family, read under the lock before the write moves it —
    // see `events::deliver_member_left` for what an older app needs it for.
    let was_home = crate::families::family_ids_of(&mut *tx, target_user_id)
        .await?
        .first()
        == Some(&family.id);

    // From THIS family only: its chat, its groups, their `ai` chat of it,
    // and direct chats with people they now share nothing with
    // (docs/protocol.md, "Several families").
    let departure = remove_membership(&mut tx, family.id, target_user_id).await?;
    let ended = end_legacy_sessions_if_home_moved(&mut tx, target_user_id, was_home).await?;
    tx.commit().await?;
    announce_departure(&state, family.id, target_user_id, departure, was_home).await;

    events::log_fanout_error(
        "member_left",
        events::deliver_member_left(&state, family.id, target_user_id, was_home).await,
    );
    close_ended_sessions(&state, &ended).await;
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
    Path(UserPath {
        user_id: target_user_id,
    }): Path<UserPath>,
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
    // A reset signs the member out of every device and so hands the
    // resetter the WHOLE account — every family, every direct chat. So an
    // owner may reset only a member ALL of whose families they own
    // (docs/protocol.md, "Several families"; decision D7): the child only
    // in the parents' family, as before, and never the married son who also
    // owns his own. The member changes it themselves with `POST
    // /me/password`.
    if !may_reset_password(&state.pool, auth.user_id, target_user_id).await? {
        return Err(ApiError::forbidden(
            codes::MEMBER_IN_OTHER_FAMILIES,
            "this member is in a family you do not own",
        ));
    }

    let password_hash = crate::auth::hash_password(req.new_password).await?;
    let mut tx = state.pool.begin().await?;
    // Asked AGAIN, under the member's row lock: the check above ran before a
    // hash that takes tens of milliseconds, and an invitation accepted into
    // somebody else's family in that gap would otherwise hand this owner an
    // account that is no longer only theirs to recover. `grant_membership`
    // takes this same row FOR UPDATE before it inserts a membership, so the
    // answer read here cannot move before the commit.
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(target_user_id)
        .execute(&mut *tx)
        .await?;
    if !crate::families::is_member(&mut *tx, family.id, target_user_id).await? {
        return Err(ApiError::forbidden(
            codes::NOT_SAME_FAMILY,
            "no such member in your family",
        ));
    }
    if !may_reset_password(&mut *tx, auth.user_id, target_user_id).await? {
        return Err(ApiError::forbidden(
            codes::MEMBER_IN_OTHER_FAMILIES,
            "this member is in a family you do not own",
        ));
    }
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
    Path(UserPath {
        user_id: target_user_id,
    }): Path<UserPath>,
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
    Path(UserPath {
        user_id: target_user_id,
    }): Path<UserPath>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    require_same_family(&state, &family, target_user_id).await?;

    sqlx::query("UPDATE users SET birthday_month = NULL, birthday_day = NULL WHERE id = $1")
        .bind(target_user_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Does `owner_user_id` own EVERY family `member_user_id` is in — the D7
/// rule for an owner's password reset (docs/protocol.md, "Several
/// families")? False for a member in no family at all, who has no owner.
async fn may_reset_password<'e, E: sqlx::PgExecutor<'e>>(
    executor: E,
    owner_user_id: i64,
    member_user_id: i64,
) -> Result<bool, ApiError> {
    // `may_reset_sql` is the rule the roster and the frames show; the one
    // thing added here is that the member is in SOME family — an account in
    // none is nobody's to reset, where the roster never lists one.
    let allowed = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM family_members WHERE user_id = $1) AND {}",
        may_reset_sql("$1", "$2")
    ))
    .bind(member_user_id)
    .bind(owner_user_id)
    .fetch_one(executor)
    .await?;
    Ok(allowed)
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
    if !crate::families::is_member(&state.pool, family.id, target_user_id).await? {
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
        // Filled in by the one read that carries it — the owner's own
        // roster (`my_family`) — and absent from every other Member.
        password_reset: None,
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
///
/// That second half is also what finds the family's STICKER PACK (0048):
/// a pack picture names no message, its row goes by cascade with the item,
/// and it was uploaded into this family — `handlers_pack` refuses to claim
/// one that was not, precisely so this list stays complete.
///
/// Two things a family with members in several families needs first
/// (docs/protocol.md, "Several families"):
///
/// - a DIRECT chat whose row names this family but whose pair still shares
///   another one is re-pointed there BEFORE the delete, or the cascade would
///   take a conversation two people can still see. Its files are then not
///   this family's, and are not collected;
/// - the family's pending INVITATIONS lapse with it, and are handed back so
///   the invited people can be told once the transaction has committed.
pub(crate) async fn delete_family_in_tx(
    tx: &mut PgConnection,
    family_id: i64,
) -> Result<DeletedFamily, ApiError> {
    sqlx::query(
        "UPDATE chats c
            SET family_id = shared.family_id
           FROM (SELECT c2.id AS chat_id, min(fa.family_id) AS family_id
                   FROM chats c2
                   JOIN family_members fa ON fa.user_id = c2.user_a_id AND fa.family_id <> $1
                   JOIN family_members fb ON fb.user_id = c2.user_b_id
                                         AND fb.family_id = fa.family_id
                  WHERE c2.family_id = $1 AND c2.kind = 'direct'
                  GROUP BY c2.id) AS shared
          WHERE c.id = shared.chat_id",
    )
    .bind(family_id)
    .execute(&mut *tx)
    .await?;
    let invitations: Vec<(i64, i64)> =
        sqlx::query_as("SELECT id, user_id FROM invitations WHERE family_id = $1")
            .bind(family_id)
            .fetch_all(&mut *tx)
            .await?;
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
    Ok(DeletedFamily {
        storage_keys,
        invitations: invitations
            .into_iter()
            .map(|(id, user_id)| LapsedInvitation {
                id,
                family_id,
                user_id,
            })
            .collect(),
    })
}

/// What deleting a family left for after the commit.
#[derive(Debug, Default)]
pub(crate) struct DeletedFamily {
    /// The files its rows named, each to be swept if nothing else does.
    pub storage_keys: Vec<String>,
    /// Its pending invitations, which lapsed with it: each invited person is
    /// told `invitation_removed` (the family has no owner left to tell).
    pub invitations: Vec<LapsedInvitation>,
}

/// After a family was deleted: the invited people told, nobody else — the
/// owner who might have been is the one who deleted it.
pub(crate) async fn announce_deleted_family_invitations(
    state: &AppState,
    invitations: &[LapsedInvitation],
) {
    for invitation in invitations {
        events::log_fanout_error(
            "invitation_removed",
            events::deliver_invitation_removed(
                state,
                invitation.id,
                invitation.family_id,
                invitation.user_id,
                None,
            )
            .await,
        );
    }
}

/// What one departure from one family did, for the frames sent after the
/// transaction commits.
#[derive(Debug, Default)]
pub(crate) struct Departure {
    /// The groups of THIS family the member was taken out of.
    pub group_exits: Vec<crate::handlers_group::GroupExit>,
    /// The direct chats whose `family_ids` the departure changed — shrunk,
    /// or emptied, which closes the chat (docs/protocol.md, "Several
    /// families"). Each side is sent `chat_updated` once it has committed.
    pub direct_changes: Vec<i64>,
    /// The invitations that lapsed because their person and their family's
    /// owner no longer share a family (`lapse_unshared_invitations`).
    pub lapsed: Vec<LapsedInvitation>,
}

/// Shared leave/remove write: take the user out of ONE family — its family
/// chat, its groups, their `ai` chat of it — and settle their direct chats
/// with the people they no longer share it with. Chat and message rows stay:
/// history is retained and resurfaces on rejoin per protocol.md.
///
/// Runs INSIDE the caller's transaction, and both callers open that
/// transaction by taking `FOR UPDATE` on the family row. It used to open a
/// transaction of its own, which meant the leave landed with no lock on the
/// family held at all — and `delete_account`, choosing a successor at that
/// exact moment, still saw the departing member as a member and handed them
/// the family on their way out of it. A family owned by somebody who is not
/// in it has no working owner endpoint and nothing repairs it, so the write
/// and the lock have to be the same transaction.
///
/// It also takes the member out of every GROUP of the family, with each
/// group's admin hand-off, BEFORE the membership rows go; what that did is
/// returned so the caller can announce it once the transaction commits
/// (protocol.md, "Groups": a rejoin does not put them back). Groups of the
/// member's OTHER families are untouched — leaving one family leaves that
/// family only (docs/protocol.md, "Several families").
///
/// DIRECT CHATS belong to the pair, not to a family, and are listed in every
/// family the two share. So the ones this departure can change are exactly
/// those with somebody still in this family: if the two still share another
/// family, the chat stays open and its `family_ids` shrink (and the row's
/// own `family_id` moves off this family, so deleting this family one day
/// cannot take the chat with it by cascade); if they share none, the chat
/// closes exactly as leaving a family always closed it — the departing
/// member's row goes, and it comes back with its history when the two share
/// a family again.
async fn remove_membership(
    tx: &mut PgConnection,
    family_id: i64,
    user_id: i64,
) -> Result<Departure, ApiError> {
    let removed = sqlx::query("DELETE FROM family_members WHERE family_id = $1 AND user_id = $2")
        .bind(family_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        // They already left in a parallel request; nothing to undo.
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "the user no longer belongs to this family",
        ));
    }
    // `familyless_since` starts AGAIN here (0034) — but only when this was
    // the account's LAST family: an account is "without a family" once it
    // has left every one of them, not when it leaves one of several
    // (docs/protocol.md, "Several families"). Whatever the account's age,
    // somebody leaving their last family today has the full grace to be let
    // back in before the sweep takes the account.
    sqlx::query(
        "UPDATE users SET familyless_since = now()
          WHERE id = $1
            AND NOT EXISTS (SELECT 1 FROM family_members WHERE user_id = $1)",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    let group_exits =
        crate::handlers_group::leave_every_group(&mut *tx, user_id, Some(family_id)).await?;
    // The family's own chats: its family chat and the member's `ai` chat of
    // it. NOT its direct chats, which are the pair's and are settled below.
    sqlx::query(
        "DELETE FROM chat_members cm
         USING chats c
         WHERE cm.chat_id = c.id AND c.family_id = $1 AND c.kind <> 'direct'
           AND cm.user_id = $2",
    )
    .bind(family_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    let direct: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT c.id,
                CASE WHEN c.user_a_id = $1 THEN c.user_b_id ELSE c.user_a_id END AS peer
           FROM chats c
           JOIN chat_members cm ON cm.chat_id = c.id AND cm.user_id = $1
          WHERE c.kind = 'direct'
            AND (c.user_a_id = $1 OR c.user_b_id = $1)
            AND EXISTS (SELECT 1 FROM family_members fm
                         WHERE fm.family_id = $2
                           AND fm.user_id =
                               CASE WHEN c.user_a_id = $1 THEN c.user_b_id ELSE c.user_a_id END)
          ORDER BY c.id",
    )
    .bind(user_id)
    .bind(family_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut direct_changes = Vec::with_capacity(direct.len());
    for (chat_id, peer) in direct {
        // The chat row's lock BEFORE the families the two share are read.
        // Each departure holds only its own family's lock, so two of them
        // from two different shared families — one by each side — could
        // otherwise each read the other's membership as still there, and
        // neither would close a chat the pair no longer shares. Locked in
        // ascending id order, and read in a statement of its own after the
        // lock, so the second sees what the first committed.
        sqlx::query("SELECT id FROM chats WHERE id = $1 FOR UPDATE")
            .bind(chat_id)
            .execute(&mut *tx)
            .await?;
        let shared = crate::families::shared_families(&mut *tx, user_id, peer).await?;
        if shared.is_empty() {
            sqlx::query("DELETE FROM chat_members WHERE chat_id = $1 AND user_id = $2")
                .bind(chat_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("UPDATE chats SET family_id = $2 WHERE id = $1 AND family_id <> ALL($3)")
                .bind(chat_id)
                .bind(shared[0])
                .bind(&shared)
                .execute(&mut *tx)
                .await?;
        }
        direct_changes.push(chat_id);
    }
    direct_changes.extend(settle_held_closed_chats(&mut *tx, family_id, user_id).await?);
    let lapsed = lapse_unshared_invitations(&mut *tx, user_id, family_id).await?;
    Ok(Departure {
        group_exits,
        direct_changes,
        lapsed,
    })
}

/// The direct chats `user_id` still holds after the OTHER side left, whose
/// row names `family_id` — the family `user_id` is now leaving. A departure's
/// own pass only sees chats whose peer is in the family; this is the other
/// half. Moved onto the leaver's home family while they have one, so the row
/// always names a family the side that holds it is in: a row left naming a
/// family nobody holding it belongs to would be taken, history and all, by
/// that family's deletion, with nobody told. Without a family left, the
/// leaver lets go of it too, as every departure did before several families.
///
/// Answers the chats it changed, to be announced with the departure.
async fn settle_held_closed_chats(
    tx: &mut PgConnection,
    family_id: i64,
    user_id: i64,
) -> Result<Vec<i64>, ApiError> {
    let held: Vec<i64> = sqlx::query_scalar(
        "SELECT c.id
           FROM chats c
           JOIN chat_members cm ON cm.chat_id = c.id AND cm.user_id = $1
          WHERE c.kind = 'direct'
            AND c.family_id = $2
            AND (c.user_a_id = $1 OR c.user_b_id = $1)
            AND NOT EXISTS (SELECT 1 FROM family_members fm
                             WHERE fm.family_id = $2
                               AND fm.user_id =
                                   CASE WHEN c.user_a_id = $1 THEN c.user_b_id
                                        ELSE c.user_a_id END)
          ORDER BY c.id
            FOR UPDATE OF c",
    )
    .bind(user_id)
    .bind(family_id)
    .fetch_all(&mut *tx)
    .await?;
    if held.is_empty() {
        return Ok(held);
    }
    // Their home family OTHER than this one: when the last member leaves,
    // this runs before the family is deleted and still counts it.
    match crate::families::family_ids_of(&mut *tx, user_id)
        .await?
        .into_iter()
        .find(|id| *id != family_id)
    {
        Some(home) => {
            sqlx::query("UPDATE chats SET family_id = $2 WHERE id = ANY($1)")
                .bind(&held)
                .bind(home)
                .execute(&mut *tx)
                .await?;
        }
        None => {
            sqlx::query("DELETE FROM chat_members WHERE chat_id = ANY($1) AND user_id = $2")
                .bind(&held)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
    }
    Ok(held)
}

/// The invitations that stop being valid when `user_id` leaves `family_id`:
/// an owner invites only somebody they share ANOTHER family with, so an
/// invitation lapses once its person and its family's current owner share
/// none — when either of them leaves the family they shared, or when
/// `family_id` changes hands to an owner who shares none with the invited.
/// Without this the owner went on reading the person's live profile through
/// the invitation, and the person could still come in by a door
/// `create_invitation` would now refuse. Answers the lapsed rows, for the
/// `invitation_removed` frames once the departure has committed.
pub(crate) async fn lapse_unshared_invitations(
    tx: &mut PgConnection,
    user_id: i64,
    family_id: i64,
) -> Result<Vec<LapsedInvitation>, ApiError> {
    let rows: Vec<(i64, i64, i64)> = sqlx::query_as(
        "DELETE FROM invitations i
          USING families f
          WHERE f.id = i.family_id
            AND (i.user_id = $1 OR f.owner_user_id = $1 OR i.family_id = $2)
            AND NOT EXISTS (
                SELECT 1 FROM family_members mine
                  JOIN family_members theirs ON theirs.family_id = mine.family_id
                 WHERE mine.user_id = f.owner_user_id
                   AND theirs.user_id = i.user_id
                   AND mine.family_id <> i.family_id)
          RETURNING i.id, i.family_id, i.user_id",
    )
    .bind(user_id)
    .bind(family_id)
    .fetch_all(&mut *tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, family_id, user_id)| LapsedInvitation {
            id,
            family_id,
            user_id,
        })
        .collect())
}

/// After a departure committed: the groups as they now stand, and each
/// changed direct chat to both of its people.
///
/// `was_home` is whether the family left was the departed account's HOME
/// family before it left (see `events::deliver_member_left`), so an app from
/// before several families hears about the groups it was showing.
pub(crate) async fn announce_departure(
    state: &AppState,
    family_id: i64,
    user_id: i64,
    departure: Departure,
    was_home: bool,
) {
    refresh_home(state, user_id).await;
    crate::handlers_group::announce_exits(
        state,
        user_id,
        departure.group_exits,
        Some((family_id, was_home)),
    )
    .await;
    announce_direct_chats(state, &departure.direct_changes).await;
    announce_lapsed(state, &departure.lapsed).await;
}

/// Tell the registry `user_id`'s home family as it stands now, once a
/// membership change has committed and before its frames go out — what a
/// legacy connection may be sent is decided from it (`Registry::set_home`).
/// Never fails the change it follows; a lost refresh is put right by the
/// app's next connection.
pub(crate) async fn refresh_home(state: &AppState, user_id: i64) {
    match crate::families::family_ids_of(&state.pool, user_id).await {
        Ok(family_ids) => {
            state
                .registry
                .set_home(user_id, family_ids.first().copied())
                .await
        }
        Err(error) => tracing::warn!(?error, user_id, "could not refresh a home family"),
    }
}

/// After a membership committed: the family's `member_joined`, and the
/// direct chats the joiner has with people already in the family — each
/// one's `family_ids` just grew, and one that was closed because the two
/// shared no family comes back with its history (docs/protocol.md, "Several
/// families").
pub(crate) async fn announce_joined(state: &AppState, family_id: i64, joined: UserBrief) {
    let user_id = joined.id;
    refresh_home(state, user_id).await;
    events::log_fanout_error(
        "member_joined",
        events::deliver_member_joined(state, family_id, joined).await,
    );
    match joined_direct_chats(state, family_id, user_id).await {
        Ok(chat_ids) => announce_direct_chats(state, &chat_ids).await,
        Err(error) => tracing::warn!(?error, "could not read direct chats after a join"),
    }
}

/// The joiner's direct chats with members of `family_id`, both sides put
/// back into any that had closed and the row moved onto a family the two
/// share — inside the grant's transaction, as ONE statement, so a leave or a
/// removal that lands after the grant cannot have its closure undone by a
/// reopen running late, and a failure reopens nothing rather than some.
///
/// The row's own `family_id` moves here only when it names a family the two
/// no longer share — `direct_chat`'s upsert rule. A chat that closed kept the
/// family one side left, and everything read off the row (a transcript's
/// switch, language and statistics) would go on naming it. Each chat row is
/// locked, as a departure locks it (`remove_membership`).
async fn reopen_direct_chats(
    tx: &mut PgConnection,
    family_id: i64,
    user_id: i64,
) -> Result<(), ApiError> {
    sqlx::query(
        "WITH pairs AS (
             SELECT c.id, c.user_a_id, c.user_b_id
               FROM chats c
              WHERE c.kind = 'direct'
                AND (c.user_a_id = $1 OR c.user_b_id = $1)
                AND EXISTS (SELECT 1 FROM family_members fm
                             WHERE fm.family_id = $2
                               AND fm.user_id =
                                   CASE WHEN c.user_a_id = $1 THEN c.user_b_id
                                        ELSE c.user_a_id END)
              ORDER BY c.id
                FOR UPDATE OF c
         ), joined AS (
             INSERT INTO chat_members (chat_id, user_id)
             SELECT id, user_a_id FROM pairs
             UNION ALL
             SELECT id, user_b_id FROM pairs
             ON CONFLICT DO NOTHING
         )
         UPDATE chats c SET family_id = $2
           FROM pairs p
          WHERE c.id = p.id
            AND (SELECT count(*) FROM family_members fm
                  WHERE fm.family_id = c.family_id
                    AND fm.user_id IN (c.user_a_id, c.user_b_id)) < 2",
    )
    .bind(user_id)
    .bind(family_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The joiner's direct chats with members of `family_id` — the ones whose
/// `family_ids` the join just grew or reopened, to be announced. A read, after
/// the commit: the writes were the grant's.
async fn joined_direct_chats(
    state: &AppState,
    family_id: i64,
    user_id: i64,
) -> Result<Vec<i64>, ApiError> {
    let chat_ids = sqlx::query_scalar(
        "SELECT c.id
           FROM chats c
          WHERE c.kind = 'direct'
            AND (c.user_a_id = $1 OR c.user_b_id = $1)
            AND EXISTS (SELECT 1 FROM family_members fm
                         WHERE fm.family_id = $2
                           AND fm.user_id =
                               CASE WHEN c.user_a_id = $1 THEN c.user_b_id ELSE c.user_a_id END)
          ORDER BY c.id",
    )
    .bind(user_id)
    .bind(family_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(chat_ids)
}

/// `chat_updated` for each of these direct chats, to each of its two people
/// as it reads to them — the families they share now; for one that has
/// closed, `[]` to the side that left and a family of theirs to the side that
/// stayed (see `viewer_chat_family_ids_sql`). Never fails the request whose
/// commit made the change.
pub(crate) async fn announce_direct_chats(state: &AppState, chat_ids: &[i64]) {
    for &chat_id in chat_ids {
        let row = sqlx::query(&format!(
            "SELECT c.user_a_id, c.user_b_id,
                    ua.display_name AS a_name, ub.display_name AS b_name,
                    {} AS family_ids
               FROM chats c
               JOIN users ua ON ua.id = c.user_a_id
               JOIN users ub ON ub.id = c.user_b_id
              WHERE c.id = $1 AND c.kind = 'direct'",
            crate::families::CHAT_FAMILY_IDS_SQL
        ))
        .bind(chat_id)
        .fetch_optional(&state.pool)
        .await;
        let row = match row {
            Ok(Some(row)) => row,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    ?error,
                    chat_id,
                    "could not read a direct chat to announce it"
                );
                continue;
            }
        };
        let user_a: i64 = row.get("user_a_id");
        let user_b: i64 = row.get("user_b_id");
        let family_ids: Vec<i64> = row.get("family_ids");
        // A pair that shares no family any more: the side that stayed in the
        // chat is sent it under a family of theirs, as `GET /chats` lists it
        // (`viewer_chat_family_ids_sql`); the side that left gets `[]`.
        let (a_families, b_families) = if family_ids.is_empty() {
            match sqlx::query_as::<_, (Vec<i64>, Vec<i64>)>(&format!(
                "SELECT CASE WHEN EXISTS (SELECT 1 FROM chat_members m
                                           WHERE m.chat_id = c.id AND m.user_id = c.user_a_id)
                             THEN {a} ELSE '{{}}'::bigint[] END,
                        CASE WHEN EXISTS (SELECT 1 FROM chat_members m
                                           WHERE m.chat_id = c.id AND m.user_id = c.user_b_id)
                             THEN {b} ELSE '{{}}'::bigint[] END
                   FROM chats c WHERE c.id = $1",
                a = crate::families::viewer_chat_family_ids_sql("c.user_a_id"),
                b = crate::families::viewer_chat_family_ids_sql("c.user_b_id"),
            ))
            .bind(chat_id)
            .fetch_one(&state.pool)
            .await
            {
                Ok(sides) => sides,
                Err(error) => {
                    tracing::warn!(
                        ?error,
                        chat_id,
                        "could not read a closed direct chat's sides"
                    );
                    (Vec::new(), Vec::new())
                }
            }
        } else {
            (family_ids.clone(), family_ids.clone())
        };
        let side = |peer: i64, title: String, family_ids: Vec<i64>| crate::models::Chat {
            id: chat_id,
            kind: "direct".to_string(),
            title,
            peer_user_id: Some(peer),
            member_ids: None,
            admin_user_id: None,
            family_ids,
        };
        events::log_fanout_error(
            "chat_updated",
            events::deliver_direct_chat_updated(
                state,
                [
                    (user_a, side(user_b, row.get("b_name"), a_families)),
                    (user_b, side(user_a, row.get("a_name"), b_families)),
                ],
            )
            .await,
        );
    }
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

    /// The place list's states: absent leaves it alone, a list replaces it,
    /// and `null` arrives as its own state so the handler can refuse it.
    #[test]
    fn the_place_list_parses_as_three_distinct_states() {
        let absent: PatchFamilyRequest = serde_json::from_str("{}").expect("parses");
        assert_eq!(absent.greeting_places, None);
        let null: PatchFamilyRequest =
            serde_json::from_str(r#"{"greeting_places": null}"#).expect("parses");
        assert_eq!(null.greeting_places, Some(None));
        let cleared: PatchFamilyRequest =
            serde_json::from_str(r#"{"greeting_places": []}"#).expect("parses");
        assert_eq!(cleared.greeting_places, Some(Some(Vec::new())));
    }

    fn places(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    /// Kept as the server will keep them: trimmed, whitespace folded, the
    /// first spelling of a case-insensitive repeat, and the three counted
    /// after the repeats are gone.
    #[test]
    fn place_names_are_trimmed_folded_and_deduplicated() {
        assert_eq!(
            validate_greeting_places(&places(&[
                "  Moscow ",
                "Novi\t\n  Sad",
                "MOSCOW",
                "Belgrade"
            ]))
            .expect("valid"),
            places(&["Moscow", "Novi Sad", "Belgrade"])
        );
        // Unicode lower-casing, not ASCII: "МОСКВА" and "Москва" are one name.
        assert_eq!(
            validate_greeting_places(&places(&["Москва", "МОСКВА"])).expect("valid"),
            places(&["Москва"])
        );
        assert_eq!(
            validate_greeting_places(&[]).expect("valid"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_bad_place_list_is_refused_and_the_message_never_names_a_place() {
        let refused = |names: &[&str]| {
            let error = validate_greeting_places(&places(names)).expect_err("refused");
            let text = format!("{error:?}");
            for name in names {
                let name = name.trim();
                if name.len() > 2 {
                    assert!(!text.contains(name), "{text} names a place");
                }
            }
            text
        };
        assert!(refused(&["Moscow", "Belgrade", "Paris", "Tokyo"]).contains("at most 3"));
        assert!(refused(&["Moscow", "   "]).contains("place 2 is empty"));
        assert!(refused(&[""]).contains("place 1 is empty"));
        let long = "x".repeat(MAX_GREETING_PLACE_CHARS + 1);
        assert!(refused(&[long.as_str()]).contains("longer than 80"));
        assert!(refused(&["Bel\u{0}grade"]).contains("control character"));
        // 80 characters exactly is fine, counted in characters, not bytes.
        let exact = "ж".repeat(MAX_GREETING_PLACE_CHARS);
        assert_eq!(
            validate_greeting_places(&places(&[exact.as_str()])).expect("valid"),
            places(&[exact.as_str()])
        );
    }
}
