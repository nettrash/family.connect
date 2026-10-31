//! The two doors the App Store's word comes in by (docs/protocol.md, "Paid
//! families"): the app handing over what Apple signed for its account, and
//! Apple telling the server itself.
//!
//! Both believe a value only after `crate::appstore` has checked it, and both
//! log by CHECK and never by content: no transaction id, no token, no JWS.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use tracing::{info, warn};

use crate::appstore::Refusal;
use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::state::AppState;
use crate::subscriptions::{self, Applied};

/// `PUT /me/subscription` body.
#[derive(Debug, Deserialize)]
pub struct PutSubscriptionRequest {
    /// `Transaction.jwsRepresentation`.
    pub transaction: Option<String>,
    /// The subscription status's `RenewalInfo.jwsRepresentation`.
    #[serde(default)]
    pub renewal: Option<String>,
}

fn invalid(refusal: Refusal) -> ApiError {
    warn!(reason = refusal.reason(), "subscription refused");
    ApiError::bad_request(
        codes::SUBSCRIPTION_INVALID,
        "the App Store purchase could not be confirmed",
    )
}

/// `PUT /me/subscription` — the app hands over what the App Store signed
/// for this account: after a purchase or a restore, on every launch, and for
/// every update StoreKit delivers. Idempotent; an older value than the one
/// held changes nothing.
pub async fn put_subscription(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<PutSubscriptionRequest>,
) -> Result<Response, ApiError> {
    let subs = &state.cfg.subscriptions;
    if !subs.enabled {
        return Err(ApiError::forbidden(
            codes::SUBSCRIPTIONS_DISABLED,
            "this server does not charge for families",
        ));
    }
    let Some(transaction) = req.transaction.as_deref() else {
        return Err(ApiError::validation("transaction is required"));
    };
    let policy = subs.policy();
    let txn = policy.transaction(transaction).map_err(invalid)?;
    // A purchase made for another account is not this one's to claim. One
    // with no token at all — bought outside the app, an offer code — belongs
    // to whoever presents it first.
    if let Some(token) = txn.app_account_token
        && token != subscriptions::app_account_token(&state, auth.user_id).await?
    {
        return Err(invalid(Refusal::Token));
    }
    let renewal = match req.renewal.as_deref() {
        None => None,
        Some(jws) => {
            let renewal = policy.renewal(jws).map_err(invalid)?;
            if renewal.original_transaction_id != txn.original_transaction_id {
                return Err(invalid(Refusal::Mismatch));
            }
            Some(renewal)
        }
    };

    let mut tx = state.pool.begin().await?;
    match subscriptions::apply_transaction(&mut tx, &state.keys, Some(auth.user_id), &txn).await? {
        Applied::Stored => {}
        Applied::Taken => {
            tx.rollback().await?;
            warn!("subscription refused (taken)");
            return Err(ApiError::conflict(
                codes::SUBSCRIPTION_TAKEN,
                "this subscription belongs to another account",
            ));
        }
        // A presenter always stores or is refused; this would be a bug in
        // `apply_transaction`, answered as one rather than a panic.
        Applied::Unknown => {
            tx.rollback().await?;
            return Err(ApiError::Internal(anyhow::anyhow!(
                "a presented subscription was neither stored nor taken"
            )));
        }
    }
    if let Some(renewal) = &renewal {
        subscriptions::apply_renewal(&mut tx, &state.keys, renewal).await?;
    }
    tx.commit().await?;
    info!(environment = %txn.environment, "subscription stored");

    // A family this account owns that was waiting on it is covered again,
    // now rather than at the next sweep.
    if let Err(error) = subscriptions::settle_owner(&state, auth.user_id).await {
        warn!(?error, "could not settle the owner's families");
    }
    let subscription =
        subscriptions::subscription_of(&state.pool, &state.cfg, auth.user_id).await?;
    Ok((StatusCode::OK, Json(json!({"subscription": subscription}))).into_response())
}

/// `POST /appstore/notifications` body.
#[derive(Debug, Deserialize)]
pub struct NotificationRequest {
    #[serde(rename = "signedPayload")]
    pub signed_payload: Option<String>,
}

/// `POST /appstore/notifications` — App Store Server Notifications, version
/// 2. No session: the sender is Apple, and Apple's signature is the
/// authentication. `200` for everything that verifies, even when it changes
/// nothing; `400` for what does not, which Apple retries harmlessly.
pub async fn appstore_notification(
    State(state): State<AppState>,
    AppJson(req): AppJson<NotificationRequest>,
) -> Result<Response, ApiError> {
    let subs = &state.cfg.subscriptions;
    if !subs.enabled {
        // Answered, so Apple stops: this server takes no subscriptions.
        return Ok(StatusCode::OK.into_response());
    }
    let refused = |refusal: Refusal| {
        warn!(reason = refusal.reason(), "app store notification refused");
        ApiError::validation("the notification could not be verified")
    };
    let Some(signed_payload) = req.signed_payload.as_deref() else {
        return Err(refused(Refusal::Malformed));
    };
    let policy = subs.policy();
    let note = policy.notification(signed_payload).map_err(refused)?;
    let ignored = |why: &'static str| {
        info!(kind = %note.notification_type, why, "app store notification ignored");
        Ok(StatusCode::OK.into_response())
    };
    if note.bundle_id.as_deref() != Some(subs.bundle_id.as_str()) {
        return ignored("another app");
    }
    if !note
        .environment
        .as_deref()
        .is_some_and(|env| subs.environments.iter().any(|e| e == env))
    {
        return ignored("environment");
    }

    // The values inside are Apple's too, and verified on their own. One for
    // a product this server does not sell is not a refusal: Apple signed it.
    let txn = match note
        .signed_transaction
        .as_deref()
        .map(|jws| policy.transaction(jws))
    {
        None => None,
        Some(Ok(txn)) => Some(txn),
        Some(Err(Refusal::Product | Refusal::Kind | Refusal::Bundle | Refusal::Environment)) => {
            return ignored("product");
        }
        Some(Err(refusal)) => return Err(refused(refusal)),
    };
    let renewal = match note
        .signed_renewal
        .as_deref()
        .map(|jws| policy.renewal(jws))
    {
        None => None,
        Some(Ok(renewal)) => Some(renewal),
        Some(Err(Refusal::Environment)) => return ignored("environment"),
        Some(Err(refusal)) => return Err(refused(refusal)),
    };

    let mut tx = state.pool.begin().await?;
    let mut owner = None;
    if let Some(txn) = &txn {
        if subscriptions::apply_transaction(&mut tx, &state.keys, None, txn).await?
            == Applied::Unknown
        {
            tx.rollback().await?;
            return ignored("not presented yet");
        }
        owner =
            subscriptions::owner_of_in(&mut tx, &state.keys, &txn.original_transaction_id).await?;
    }
    if let Some(renewal) = &renewal {
        owner = owner.or(subscriptions::apply_renewal(&mut tx, &state.keys, renewal).await?);
    }
    tx.commit().await?;
    let Some(owner) = owner else {
        return ignored("not presented yet");
    };
    info!(
        kind = %note.notification_type,
        subtype = note.subtype.as_deref().unwrap_or("-"),
        "app store notification applied"
    );
    if let Err(error) = subscriptions::settle_owner(&state, owner).await {
        warn!(?error, "could not settle the owner's families");
    }
    Ok(StatusCode::OK.into_response())
}
