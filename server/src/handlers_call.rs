//! Voice-call signalling and the record a call leaves behind
//! (docs/protocol.md, "Voice calls").
//!
//! The server never carries a call's audio. What lives here is the small
//! amount of relaying that gets two members connected — offer, answer,
//! candidates — plus the one message written into the direct chat when the
//! call ends. The in-memory state machine that decides who is busy and what
//! a call ended as is `crate::calls`; this module is the seam between it and
//! the database, the connection registry and the push transports.
//!
//! Every signalling frame names a `call_id` the CALLER minted, and a client
//! applies a frame only to a call it holds — so the server addresses call
//! frames to USERS (all their connections) and lets each device sort out
//! which call a frame is about. That is what makes the multi-device story
//! (ring every phone, first to answer wins, the rest stop) need no
//! per-device bookkeeping on the server.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::json;
use sha1::Sha1;
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::calls::{Busy, CallFault, EndReason, Ended};
use crate::config::Config;
use crate::error::{ApiError, codes};
use crate::events;
use crate::handlers_chat::ensure_chat_access;
use crate::models::{CallRecord, IceCandidate, IceServer, Message};
use crate::state::AppState;
use crate::ws::ServerFrame;

/// Largest offer or answer SDP accepted, in bytes. A real audio-only SDP is
/// a few kilobytes; an offer is held in memory for as long as it rings, and
/// this ceiling is what stops a client parking megabytes in the registry.
pub const MAX_SDP_BYTES: usize = 64 * 1024;

/// Largest single ICE candidate accepted, in bytes. The caller's candidates
/// are buffered while the call rings (protocol.md caps the count at 64), so
/// each one is bounded too.
pub const MAX_CANDIDATE_BYTES: usize = 2 * 1024;

/// An offer or answer worth relaying: non-empty and under the ceiling.
fn validate_sdp(sdp: &str) -> Result<(), ApiError> {
    if sdp.trim().is_empty() {
        return Err(ApiError::bad_request(
            codes::INVALID_CALL,
            "sdp must not be empty",
        ));
    }
    if sdp.len() > MAX_SDP_BYTES {
        return Err(ApiError::bad_request(
            codes::INVALID_CALL,
            format!("sdp exceeds {MAX_SDP_BYTES} bytes"),
        ));
    }
    Ok(())
}

/// A candidate worth relaying. Oversized or empty ones are dropped in
/// silence, like every other candidate that goes nowhere.
pub fn candidate_is_sane(candidate: &IceCandidate) -> bool {
    !candidate.candidate.is_empty()
        && candidate.candidate.len() <= MAX_CANDIDATE_BYTES
        && candidate.sdp_mid.as_ref().is_none_or(|mid| mid.len() <= 64)
}

/// `GET /calls/ice` — the STUN/TURN servers to hand a peer connection, with
/// time-limited TURN credentials minted for the caller when the operator
/// configured a shared secret (protocol.md, "Voice calls").
pub async fn get_ice_servers(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    if !state.cfg.calls.enabled {
        return Err(ApiError::forbidden(
            codes::CALLS_DISABLED,
            "voice calls are disabled on this server",
        ));
    }
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let (ice_servers, ttl) = ice_servers(&state.cfg, auth.user_id, now);
    Ok((
        StatusCode::OK,
        Json(json!({"ice_servers": ice_servers, "ttl_secs": ttl})),
    )
        .into_response())
}

/// Build the ICE-server list from config. Pure: `now` is the current unix
/// time so the TURN-credential expiry is testable without a clock.
///
/// coturn's `use-auth-secret` scheme: the username is `<expiry>:<user_id>`
/// and the credential is base64(HMAC-SHA1(secret, username)). When no secret
/// is set but a static username/password is, those are used verbatim; when
/// neither is, TURN servers go out bare (which only works if the coturn
/// itself is open, but that is the operator's choice to make).
pub fn ice_servers(cfg: &Config, user_id: i64, now: i64) -> (Vec<IceServer>, u64) {
    let calls = &cfg.calls;
    let mut servers: Vec<IceServer> = Vec::new();
    if !calls.stun_urls.is_empty() {
        servers.push(IceServer {
            urls: calls.stun_urls.clone(),
            username: None,
            credential: None,
        });
    }
    if !calls.turn_urls.is_empty() {
        let (username, credential) = if !calls.turn_secret.is_empty() {
            let expiry = now + calls.turn_credential_ttl_secs as i64;
            let username = format!("{expiry}:{user_id}");
            let credential = turn_credential(&calls.turn_secret, &username);
            (Some(username), Some(credential))
        } else if !calls.turn_username.is_empty() {
            (
                Some(calls.turn_username.clone()),
                Some(calls.turn_password.clone()),
            )
        } else {
            (None, None)
        };
        servers.push(IceServer {
            urls: calls.turn_urls.clone(),
            username,
            credential,
        });
    }
    (servers, calls.turn_credential_ttl_secs)
}

/// base64(HMAC-SHA1(secret, username)) — coturn's REST-API credential.
fn turn_credential(secret: &str, username: &str) -> String {
    let mut mac =
        Hmac::<Sha1>::new_from_slice(secret.as_bytes()).expect("HMAC accepts a key of any length");
    mac.update(username.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// Place a call: check it may happen, ring the callee, and answer the
/// caller's connection with `call_ringing` (via the return of `Ok(())`,
/// after which ws.rs is silent — `call_ringing` is delivered here).
pub async fn offer(
    state: &AppState,
    auth: &AuthUser,
    conn_id: u64,
    call_id: Uuid,
    chat_id: i64,
    sdp: String,
    video: bool,
) -> Result<(), ApiError> {
    if !state.cfg.calls.enabled {
        return Err(ApiError::forbidden(
            codes::CALLS_DISABLED,
            "voice calls are disabled on this server",
        ));
    }
    // A video offer against a voice-only server is refused BEFORE anything
    // begins: nothing is rung, nothing is recorded, and the caller is left
    // free for a follow-up voice offer — the same shape as `calls_disabled`
    // (protocol.md, "Video").
    if video && !state.cfg.calls.video_enabled {
        return Err(ApiError::forbidden(
            codes::VIDEO_CALLS_DISABLED,
            "video calls are disabled on this server",
        ));
    }
    validate_sdp(&sdp)?;
    ensure_chat_access(state, chat_id, auth.user_id).await?;
    // The call must be one to one: a family chat has no single person to
    // ring, and the assistant has no ears.
    let row = sqlx::query("SELECT kind, user_a_id, user_b_id FROM chats WHERE id = $1")
        .bind(chat_id)
        .fetch_one(&state.pool)
        .await?;
    let kind: String = row.get("kind");
    if kind != "direct" {
        return Err(ApiError::bad_request(
            codes::INVALID_CALL,
            "voice calls are only allowed in a direct chat",
        ));
    }
    let user_a: i64 = row.get("user_a_id");
    let user_b: i64 = row.get("user_b_id");
    let callee_id = if user_a == auth.user_id {
        user_b
    } else {
        user_a
    };

    // THE BLOCKER'S OWN DIRECTION: refused outright, and this is the one
    // call error that only ever reaches the person who set the block.
    if crate::blocks::blocks(&state.pool, auth.user_id, callee_id).await? {
        return Err(ApiError::conflict(
            codes::BLOCKED,
            "you have blocked this member",
        ));
    }

    // THE OTHER DIRECTION: the callee has blocked the caller. NOT refused —
    // every cheap refusal is a tell the caller reads in their own call
    // history. An API error writes no record where one has always
    // appeared; an auto-decline writes `declined` instantly, at four in the
    // morning, for ever; `peer_unreachable` is satisfiable only when the
    // callee has no pushable device at all, which the caller can rule out
    // by watching them post.
    //
    // So the call is parked and rings out for the full timeout, ending in
    // the ordinary `missed` record — the only outcome indistinguishable
    // from being ignored (protocol.md, "Blocking a member"). What the
    // `suppressed` flag buys is that the CALLEE is not a party to it, so a
    // loop of blocked offers cannot make them uncallable by the rest of
    // the family.
    let suppressed = crate::blocks::blocks(&state.pool, callee_id, auth.user_id).await?;

    // One call per person, on either side, before anything is rung.
    state
        .calls
        .begin(
            call_id,
            chat_id,
            auth.user_id,
            callee_id,
            conn_id,
            sdp.clone(),
            video,
            suppressed,
        )
        .map_err(|busy| match busy {
            Busy::Caller => ApiError::conflict(codes::CALL_BUSY, "you are already on a call"),
            Busy::Callee => {
                ApiError::conflict(codes::PEER_BUSY, "the person you called is on another call")
            }
        })?;

    // Everything past `begin` can fail — the database while it reads the
    // callee's devices or the caller's name, the push seam — and a failure
    // must not leave the callee ringing and both parties busy until the
    // sweep notices. Undo it the way a caller giving up is undone: end the
    // call as a cancel, which also tells any callee connection that already
    // saw the offer, and writes the missed record the callee can act on.
    match ring(
        state, auth, conn_id, call_id, chat_id, callee_id, sdp, video, suppressed,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(err) => {
            if let Some(ended) = state.calls.end(call_id, None, EndReason::Cancel) {
                finish_call(state, ended).await;
            }
            Err(err)
        }
    }
}

/// The half of an offer that runs with the call already in the registry.
#[allow(clippy::too_many_arguments)] // the flag is part of the offer, not a config
async fn ring(
    state: &AppState,
    auth: &AuthUser,
    conn_id: u64,
    call_id: Uuid,
    chat_id: i64,
    callee_id: i64,
    sdp: String,
    video: bool,
    suppressed: bool,
) -> Result<(), ApiError> {
    // Is the callee reachable at all — a socket, or a device that can be
    // woken? If neither, the offer cannot land: the caller is told, and the
    // unwinding in `offer` records it as the missed call it was (the callee
    // was nowhere to be reached).
    let callee_live = !state.registry.live_sessions(&[callee_id]).await.is_empty();
    let wakeable = events::has_wakeable_device(&state.pool, callee_id).await?;
    if !callee_live && !wakeable {
        return Err(ApiError::not_found(
            codes::PEER_UNREACHABLE,
            "the person you called has no device that can be reached",
        ));
    }

    // Ring: deliver the offer to every connection the callee has, and wake
    // the devices that have none.
    let caller_name = display_name(&state.pool, auth.user_id).await?;
    let offer_frame = ServerFrame::CallOffer {
        call_id,
        chat_id,
        from_user_id: auth.user_id,
        sdp,
        video,
    };
    //
    // THE BLOCK IS APPLIED HERE, AT DELIVERY, AND NEVER AT THE DECISION.
    // Everything above ran against the callee's REAL, unblocked state — so
    // a blocked caller is told `peer_unreachable` exactly when the callee
    // genuinely has nothing to reach, in the same second an unblocked
    // caller would have been, and `peer_busy` exactly when they are
    // genuinely on a call. Deciding earlier is a one-frame oracle: take
    // the blocker's devices out of the candidate list first and the offer
    // answers `peer_unreachable` instantly, while a control call to a
    // third member rings for the full timeout.
    //
    // What is dropped here is total, and it has to be: no offer frame, no
    // VoIP push, and — see `ws::call_replays` — no replay to a device that
    // connects while this is still ringing.
    if !suppressed {
        state
            .registry
            .send_to_users(&[callee_id], &offer_frame)
            .await;
        events::push_incoming_call(
            state,
            callee_id,
            crate::push_payload::CallPush {
                call_id: call_id.to_string(),
                chat_id,
                from_user_id: auth.user_id,
                caller_name,
                video,
                ring_timeout_secs: state.cfg.calls.ring_timeout_secs,
            },
        )
        .await?;
    }

    // Tell the caller it is ringing — on the connection the offer arrived
    // on, the only one that placed it.
    state
        .registry
        .send_to_conn(auth.user_id, conn_id, &ServerFrame::CallRinging { call_id })
        .await;
    Ok(())
}

/// The callee answers: relay the answer to the caller, and tell the
/// callee's OTHER devices to stop ringing (`answered_elsewhere`).
pub async fn answer(
    state: &AppState,
    auth: &AuthUser,
    conn_id: u64,
    call_id: Uuid,
    sdp: String,
) -> Result<(), ApiError> {
    validate_sdp(&sdp)?;
    let answered = state
        .calls
        .answer(call_id, auth.user_id)
        .map_err(|fault| match fault {
            CallFault::NotFound => ApiError::not_found(codes::CALL_NOT_FOUND, "no such call"),
            CallFault::NotCallee => {
                ApiError::forbidden(codes::INVALID_CALL, "you are not the callee of this call")
            }
            CallFault::NotRinging => {
                ApiError::conflict(codes::INVALID_CALL, "this call is no longer ringing")
            }
        })?;

    // The caller learns the call was taken.
    state
        .registry
        .send_to_users(
            &[answered.caller_id],
            &ServerFrame::CallAnswer { call_id, sdp },
        )
        .await;
    // The callee's other devices stop ringing — every connection of the
    // callee except the one that answered.
    state
        .registry
        .fan_out(
            &[answered.callee_id],
            &ServerFrame::CallEnd {
                call_id,
                reason: EndReason::AnsweredElsewhere.as_str().to_string(),
            },
            Some(conn_id),
        )
        .await;
    Ok(())
}

/// Relay one ICE candidate to the other party's connections. Silent on an
/// unknown call or a stranger — the state machine decides.
pub async fn ice(state: &AppState, from_user: i64, call_id: Uuid, candidate: IceCandidate) {
    if !candidate_is_sane(&candidate) {
        tracing::debug!(%call_id, "dropping an empty or oversized ICE candidate");
        return;
    }
    if let Some(relay) = state
        .calls
        .add_candidate(call_id, from_user, candidate.clone())
    {
        state
            .registry
            .send_to_users(
                &[relay.to_user],
                &ServerFrame::CallIce { call_id, candidate },
            )
            .await;
    }
}

/// End a call from a client's `call_end`. Relays the end to both parties
/// except the connection it came from, and writes the record.
pub async fn end(
    state: &AppState,
    auth: &AuthUser,
    conn_id: u64,
    call_id: Uuid,
    reason: String,
) -> Result<(), ApiError> {
    let reason = EndReason::from_client(&reason).ok_or_else(|| {
        ApiError::bad_request(
            codes::INVALID_CALL,
            "reason must be hangup, decline, cancel or failed",
        )
    })?;
    // An end for a call this user is not a party to — or that no longer
    // exists — is dropped in silence (protocol.md, "Semantics: Calls"): a
    // client tidying up after a restart must not get an error per frame.
    let Some(ended) = state.calls.end(call_id, Some(auth.user_id), reason) else {
        return Ok(());
    };
    let end_frame = ServerFrame::CallEnd {
        call_id,
        reason: ended.reason.as_str().to_string(),
    };
    state
        .registry
        .fan_out(
            &[ended.caller_id, ended.callee_id],
            &end_frame,
            Some(conn_id),
        )
        .await;
    if let Err(err) = record_call(state, &ended).await {
        events::log_fanout_error("recording a call ended by a client", Err(err));
    }
    Ok(())
}

/// Finish a call the SERVER ended (a ring-out, a caller vanishing, the
/// detached backstop): relay `call_end` to both parties — there is no
/// origin connection to skip — and write the record.
pub async fn finish_call(state: &AppState, ended: Ended) {
    let end_frame = ServerFrame::CallEnd {
        call_id: ended.call_id,
        reason: ended.reason.as_str().to_string(),
    };
    // A suppressed call ends for the CALLER only. The callee was never
    // told it rang, so a `call_end` would be a frame arriving out of
    // nowhere — the loudest possible tell. The RECORD below is written
    // either way: the call was accepted, it rang out, and the caller gets
    // the ordinary `missed` entry (protocol.md, "Blocking a member").
    let end_recipients: &[i64] = if ended.suppressed {
        &[ended.caller_id]
    } else {
        &[ended.caller_id, ended.callee_id]
    };
    state
        .registry
        .send_to_users(end_recipients, &end_frame)
        .await;
    if let Err(err) = record_call(state, &ended).await {
        events::log_fanout_error("recording a server-ended call", Err(err));
    }
}

/// Write the one message a call leaves in the direct chat (protocol.md,
/// "The record"). The record's `client_msg_id` is the call id, so a call
/// that ends twice — a client hang-up racing the sweeper's timeout — inserts
/// once and the second attempt is a dedup no-op.
///
/// A `missed` call pushes as the message it is; the other outcomes are
/// things both parties were present for and do not.
async fn record_call(state: &AppState, ended: &Ended) -> Result<Message, ApiError> {
    let body = ended.outcome.placeholder_body(ended.video);
    let mut tx = state.pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO messages (chat_id, sender_id, client_msg_id, body)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (chat_id, sender_id, client_msg_id) DO NOTHING
         RETURNING id, chat_id, sender_id, client_msg_id, body, created_at",
    )
    .bind(ended.chat_id)
    .bind(ended.caller_id)
    .bind(ended.call_id)
    .bind(body)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = inserted else {
        // The record already exists — this call ended twice. Roll the empty
        // transaction back and hand back the existing message without
        // delivering it a second time.
        tx.rollback().await?;
        let existing = crate::handlers_chat::fetch_message_by_client_id(
            state,
            ended.chat_id,
            ended.caller_id,
            ended.call_id,
        )
        .await?
        .ok_or_else(|| ApiError::not_found(codes::MESSAGE_NOT_FOUND, "call record vanished"))?;
        return Ok(existing);
    };

    let message_id: i64 = row.get("id");
    sqlx::query(
        "INSERT INTO calls (message_id, outcome, duration_secs, video) VALUES ($1, $2, $3, $4)",
    )
    .bind(message_id)
    .bind(ended.outcome.as_str())
    .bind(ended.duration_secs)
    .bind(ended.video)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let mut message = Message::from_row(&row);
    message.call = Some(CallRecord {
        outcome: ended.outcome.as_str().to_string(),
        duration_secs: ended.duration_secs,
        video: ended.video,
    });

    // A missed call is mail; the rest are not. `deliver_new_message` fans
    // the message out AND pushes; `deliver_message_without_push` only fans
    // out (protocol.md, "The record").
    let result = if matches!(ended.outcome, crate::calls::Outcome::Missed) {
        events::deliver_new_message(state, &message, None).await
    } else {
        events::deliver_message_without_push(state, &message, None).await
    };
    events::log_fanout_error("delivering a call record", result);
    Ok(message)
}

/// Fill the `call` object in on every message of a page that has one — one
/// query for the whole page, exactly as `attach_polls` does it. Messages
/// that are not call records are left alone, and the key stays absent.
pub async fn attach_calls(pool: &PgPool, messages: &mut [Message]) -> Result<(), ApiError> {
    if messages.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = messages.iter().map(|m| m.id).collect();
    let rows = sqlx::query(
        "SELECT message_id, outcome, duration_secs, video FROM calls WHERE message_id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;
    let mut by_id: std::collections::HashMap<i64, CallRecord> = std::collections::HashMap::new();
    for row in &rows {
        by_id.insert(
            row.get("message_id"),
            CallRecord {
                outcome: row.get("outcome"),
                duration_secs: row.get("duration_secs"),
                video: row.get("video"),
            },
        );
    }
    for message in messages.iter_mut() {
        if let Some(call) = by_id.remove(&message.id) {
            message.call = Some(call);
        }
    }
    Ok(())
}

async fn display_name(pool: &PgPool, user_id: i64) -> Result<String, ApiError> {
    let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CallsConfig;

    fn cfg_with(calls: CallsConfig) -> Config {
        Config {
            calls,
            ..Config::default()
        }
    }

    #[test]
    fn a_stun_only_config_carries_one_bare_server() {
        let cfg = cfg_with(CallsConfig::default());
        let (servers, ttl) = ice_servers(&cfg, 7, 1_756_300_000);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].urls, vec!["stun:stun.l.google.com:19302"]);
        assert!(servers[0].username.is_none() && servers[0].credential.is_none());
        assert_eq!(ttl, 86_400);
    }

    #[test]
    fn a_static_turn_credential_is_passed_through_verbatim() {
        let calls = CallsConfig {
            turn_urls: vec!["turn:turn.example.com:3478".to_string()],
            turn_username: "family".to_string(),
            turn_password: "secret".to_string(),
            ..CallsConfig::default()
        };
        let (servers, _) = ice_servers(&cfg_with(calls), 7, 1_756_300_000);
        let turn = servers
            .iter()
            .find(|s| s.urls[0].starts_with("turn:"))
            .expect("turn");
        assert_eq!(turn.username.as_deref(), Some("family"));
        assert_eq!(turn.credential.as_deref(), Some("secret"));
    }

    /// The golden: for a fixed secret, expiry and user, the coturn REST
    /// credential is base64(HMAC-SHA1(secret, "<expiry>:<user>")). The
    /// expected value was computed independently with Python's hmac.
    #[test]
    fn a_secret_turn_credential_matches_the_coturn_scheme() {
        let calls = CallsConfig {
            turn_urls: vec!["turn:turn.example.com:3478".to_string()],
            turn_secret: "s3cr3t".to_string(),
            turn_credential_ttl_secs: 86_400,
            ..CallsConfig::default()
        };
        // now + ttl = 1_756_300_000 + 86_400 = 1_756_386_400
        let (servers, ttl) = ice_servers(&cfg_with(calls), 7, 1_756_300_000);
        assert_eq!(ttl, 86_400);
        let turn = servers
            .iter()
            .find(|s| s.urls[0].starts_with("turn:"))
            .expect("turn");
        assert_eq!(turn.username.as_deref(), Some("1756386400:7"));
        // python3: base64.b64encode(hmac.new(b"s3cr3t", b"1756386400:7", hashlib.sha1).digest())
        assert_eq!(
            turn.credential.as_deref(),
            Some("EpHHFnNdWq8PKcjqeyJkQqyemkY=")
        );
    }

    #[test]
    fn an_sdp_is_refused_when_empty_or_past_the_ceiling() {
        assert!(validate_sdp("v=0\r\n").is_ok());
        assert!(validate_sdp("   ").is_err());
        assert!(validate_sdp(&"a".repeat(MAX_SDP_BYTES)).is_ok());
        assert!(validate_sdp(&"a".repeat(MAX_SDP_BYTES + 1)).is_err());
    }

    #[test]
    fn a_candidate_is_sane_only_within_its_ceilings() {
        let ok = IceCandidate {
            candidate: "candidate:1 1 udp 2130706431 192.168.1.10 51234 typ host".to_string(),
            sdp_mid: Some("0".to_string()),
            sdp_mline_index: Some(0),
        };
        assert!(candidate_is_sane(&ok));
        let empty = IceCandidate {
            candidate: String::new(),
            ..ok.clone()
        };
        assert!(!candidate_is_sane(&empty));
        let huge = IceCandidate {
            candidate: "c".repeat(MAX_CANDIDATE_BYTES + 1),
            ..ok.clone()
        };
        assert!(!candidate_is_sane(&huge));
        let long_mid = IceCandidate {
            sdp_mid: Some("m".repeat(65)),
            ..ok
        };
        assert!(!candidate_is_sane(&long_mid));
    }

    #[test]
    fn turn_without_urls_yields_no_turn_server() {
        let calls = CallsConfig {
            turn_secret: "s3cr3t".to_string(),
            ..CallsConfig::default()
        };
        let (servers, _) = ice_servers(&cfg_with(calls), 7, 1_756_300_000);
        assert!(servers.iter().all(|s| s.urls[0].starts_with("stun:")));
    }
}
