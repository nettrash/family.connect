//! Push notification transports: APNs (token-based provider auth) and FCM
//! (HTTP v1 with an OAuth2 service account), behind the `PushSender` seam.
//!
//! Delivery is best-effort by design in v1: a transport error is `warn!`ed
//! and dropped — never retried, never surfaced to the message write path.
//! The one piece of feedback that *is* acted on is "this token will never
//! work again" (APNs `410`/`BadDeviceToken`, FCM `404`/`UNREGISTERED`):
//! `notify` returns those device ids so events.rs can delete the rows, per
//! protocol.md's token-invalidation rule.
//!
//! The trait stays the seam it was in v1 — the integration tests still
//! substitute a recording impl — but the shipped implementation is now a
//! [`Dispatcher`] that routes each device by platform to its configured
//! transport, falling back to the log sender when a platform's config
//! section is absent.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::{ApnsConfig, FcmConfig, PushConfig};
use crate::push_payload::{Notification, apns_payload, fcm_message};

/// A device that should be notified: a row of `devices` with a non-null
/// token, which the per-device gate in `events` decided is not already
/// being fed by a live socket of its own.
#[derive(Debug, Clone)]
pub struct DevicePush {
    pub device_id: i64,
    pub user_id: i64,
    pub platform: String,
    pub push_token: String,
}

/// Sink for push notifications. `async_trait` keeps the trait object-safe so
/// `AppState` can hold `Arc<dyn PushSender>`.
#[async_trait]
pub trait PushSender: Send + Sync {
    /// Deliver `note` to every listed device. The caller batches per
    /// recipient user (the badge inside the note is per-user). Returns the
    /// ids of devices the platform reported as permanently unregistered so
    /// the caller can delete their rows; transport failures are logged and
    /// swallowed — push never blocks and never retries in v1.
    async fn notify(&self, devices: &[DevicePush], note: &Notification) -> Vec<i64>;
}

/// Fallback sender: records the notification in the server log and does
/// nothing else — the entire v1 behavior, now scoped to platforms without a
/// configured transport. Tokens themselves are deliberately not logged —
/// they are credentials for a third-party push service.
pub struct LogPushSender;

#[async_trait]
impl PushSender for LogPushSender {
    async fn notify(&self, devices: &[DevicePush], note: &Notification) -> Vec<i64> {
        for device in devices {
            info!(
                device_id = device.device_id,
                user_id = device.user_id,
                platform = %device.platform,
                kind = note.event.kind(),
                "push (log fallback): would deliver notification"
            );
        }
        Vec::new()
    }
}

/// The shipped `PushSender`: routes `ios` devices to APNs and `android`
/// devices to FCM, each falling back to [`LogPushSender`] when its config
/// section is absent — so a server without credentials behaves exactly like
/// v1 did.
pub struct Dispatcher {
    apns: Option<ApnsSender>,
    fcm: Option<FcmSender>,
    log: LogPushSender,
}

#[async_trait]
impl PushSender for Dispatcher {
    async fn notify(&self, devices: &[DevicePush], note: &Notification) -> Vec<i64> {
        // The devices table CHECK-constrains platform to these three
        // values, so the filters partition the slice.
        //
        // A Mac goes out over APNs with the iPhones: the macOS build shares
        // the iOS bundle id, so it shares the APNs topic, and nothing about
        // the payload differs. It is a separate platform in the DATA
        // because knowing which devices are Macs is worth having; it is not
        // a separate transport.
        let ios: Vec<DevicePush> = devices
            .iter()
            .filter(|d| d.platform == "ios" || d.platform == "macos")
            .cloned()
            .collect();
        let android: Vec<DevicePush> = devices
            .iter()
            .filter(|d| d.platform == "android")
            .cloned()
            .collect();

        let mut dead = Vec::new();
        if !ios.is_empty() {
            match &self.apns {
                Some(apns) => dead.extend(apns.send_all(&ios, note).await),
                None => {
                    self.log.notify(&ios, note).await;
                }
            }
        }
        if !android.is_empty() {
            match &self.fcm {
                Some(fcm) => dead.extend(fcm.send_all(&android, note).await),
                None => {
                    self.log.notify(&android, note).await;
                }
            }
        }
        dead
    }
}

/// Build the shipped sender from `[push]`. Fails only on unreadable
/// credentials — which `Config::validate()` already rejected, so a failure
/// here means the files changed between validation and startup.
pub fn build(cfg: &PushConfig) -> Result<Arc<dyn PushSender>> {
    // One client shared by both transports. HTTP/2 (which APNs requires) is
    // negotiated via TLS ALPN thanks to reqwest's "http2" feature; the
    // client is deliberately NOT pinned to HTTP/2-only so a plain-HTTP
    // endpoint override in the integration tests still works over HTTP/1.1.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("building the push HTTP client")?;
    let apns = cfg
        .apns
        .as_ref()
        .map(|apns_cfg| ApnsSender::new(apns_cfg, client.clone()))
        .transpose()
        .context("configuring the APNs transport")?;
    let fcm = cfg
        .fcm
        .as_ref()
        .map(|fcm_cfg| FcmSender::new(fcm_cfg, client))
        .transpose()
        .context("configuring the FCM transport")?;
    Ok(Arc::new(Dispatcher {
        apns,
        fcm,
        log: LogPushSender,
    }))
}

/// Provider JWTs are valid for 60 minutes; re-mint after 45 so a token is
/// never presented near its expiry edge. Apple also rejects tokens refreshed
/// more often than every ~20 minutes, so the cache is mandatory, not an
/// optimization.
const APNS_JWT_REFRESH: Duration = Duration::from_secs(45 * 60);

/// Claims of the APNs provider JWT — Apple wants exactly `iss` (team id) and
/// `iat`; there is no `exp`.
#[derive(Debug, Serialize, Deserialize)]
struct ApnsClaims {
    iss: String,
    iat: i64,
}

/// APNs transport: token-based provider authentication over HTTP/2.
pub struct ApnsSender {
    client: reqwest::Client,
    endpoint: String,
    bundle_id: String,
    team_id: String,
    key_id: String,
    key: EncodingKey,
    /// Cached provider JWT and when it was minted.
    jwt: Mutex<Option<(String, Instant)>>,
}

impl ApnsSender {
    pub fn new(cfg: &ApnsConfig, client: reqwest::Client) -> Result<Self> {
        Ok(Self {
            client,
            endpoint: cfg.endpoint(),
            bundle_id: cfg.bundle_id.clone(),
            team_id: cfg.team_id.clone(),
            key_id: cfg.key_id.clone(),
            key: cfg.read_signing_key()?,
            jwt: Mutex::new(None),
        })
    }

    /// Sign a fresh provider JWT: ES256, `kid` = the auth key's id in the
    /// header, `iss` = team id and `iat` = now in the claims.
    fn mint_jwt(&self) -> Result<String> {
        let header = Header {
            alg: Algorithm::ES256,
            kid: Some(self.key_id.clone()),
            ..Header::default()
        };
        let claims = ApnsClaims {
            iss: self.team_id.clone(),
            iat: time::OffsetDateTime::now_utc().unix_timestamp(),
        };
        jsonwebtoken::encode(&header, &claims, &self.key).context("signing the APNs provider JWT")
    }

    /// The cached provider JWT, re-minted after [`APNS_JWT_REFRESH`].
    async fn provider_jwt(&self) -> Result<String> {
        let mut cached = self.jwt.lock().await;
        if let Some((token, minted_at)) = cached.as_ref()
            && minted_at.elapsed() < APNS_JWT_REFRESH
        {
            return Ok(token.clone());
        }
        let token = self.mint_jwt()?;
        *cached = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    /// Send `note` to every device; returns the device ids APNs reported as
    /// unregistered.
    async fn send_all(&self, devices: &[DevicePush], note: &Notification) -> Vec<i64> {
        let jwt = match self.provider_jwt().await {
            Ok(jwt) => jwt,
            Err(err) => {
                warn!(error = ?err, "APNs provider JWT unavailable; dropping notifications");
                return Vec::new();
            }
        };
        let payload = apns_payload(note);
        let mut dead = Vec::new();
        for device in devices {
            if self.send_one(&jwt, &payload, device).await {
                dead.push(device.device_id);
            }
        }
        dead
    }

    /// One notification to one device. Returns `true` when APNs declared the
    /// token permanently unregistered.
    async fn send_one(&self, jwt: &str, payload: &Value, device: &DevicePush) -> bool {
        let url = format!("{}/3/device/{}", self.endpoint, device.push_token);
        let response = self
            .client
            .post(&url)
            .header("authorization", format!("bearer {jwt}"))
            .header("apns-topic", &self.bundle_id)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .json(payload)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(err) => {
                warn!(
                    device_id = device.device_id,
                    error = %err,
                    "APNs request failed; dropping notification"
                );
                return false;
            }
        };
        let status = response.status();
        if status.is_success() {
            return false;
        }
        // APNs failure bodies are {"reason": "..."}.
        let reason: Option<String> = response.json::<Value>().await.ok().and_then(|body| {
            body.get("reason")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        if apns_unregistered(status, reason.as_deref()) {
            info!(
                device_id = device.device_id,
                "APNs reports the token unregistered; the device row will be deleted"
            );
            return true;
        }
        warn!(
            device_id = device.device_id,
            status = %status,
            reason = reason.as_deref().unwrap_or("<none>"),
            "APNs rejected the notification; dropping it"
        );
        false
    }
}

/// APNs "this token is permanently dead": `410 Gone` (the device no longer
/// has the app), or a `400` whose reason is `BadDeviceToken` (a token for
/// the wrong environment or app — retrying can never succeed either).
fn apns_unregistered(status: reqwest::StatusCode, reason: Option<&str>) -> bool {
    status == reqwest::StatusCode::GONE
        || (status == reqwest::StatusCode::BAD_REQUEST && reason == Some("BadDeviceToken"))
}

/// Google access tokens live one hour; refresh when fewer than five minutes
/// remain so an in-flight batch never straddles the expiry.
const FCM_TOKEN_SLACK: Duration = Duration::from_secs(5 * 60);

const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const OAUTH_JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

/// Claims of the OAuth2 JWT-bearer assertion (RFC 7523 as Google profiles
/// it): the service account asserts its own identity and asks for a
/// messaging-scoped access token.
#[derive(Debug, Serialize, Deserialize)]
struct FcmAssertionClaims {
    iss: String,
    scope: String,
    aud: String,
    iat: i64,
    exp: i64,
}

#[derive(Debug, Deserialize)]
struct OauthTokenResponse {
    access_token: String,
    expires_in: u64,
}

/// FCM HTTP v1 transport: OAuth2 service-account tokens, one `messages:send`
/// POST per device.
pub struct FcmSender {
    client: reqwest::Client,
    endpoint: String,
    project_id: String,
    client_email: String,
    token_uri: String,
    key: EncodingKey,
    /// Cached access token and its absolute expiry.
    token: Mutex<Option<(String, Instant)>>,
}

impl FcmSender {
    pub fn new(cfg: &FcmConfig, client: reqwest::Client) -> Result<Self> {
        let account = cfg.read_service_account()?;
        let key = account.signing_key()?;
        Ok(Self {
            client,
            endpoint: cfg.endpoint(),
            project_id: account.project_id,
            client_email: account.client_email,
            token_uri: account.token_uri,
            key,
            token: Mutex::new(None),
        })
    }

    /// Sign the RS256 assertion: `iss` = the service-account email, `aud` =
    /// the token endpoint itself, one-hour validity (Google's maximum).
    fn mint_assertion(&self) -> Result<String> {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let claims = FcmAssertionClaims {
            iss: self.client_email.clone(),
            scope: FCM_SCOPE.to_string(),
            aud: self.token_uri.clone(),
            iat: now,
            exp: now + 3600,
        };
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &self.key)
            .context("signing the FCM OAuth assertion")
    }

    /// The cached access token, exchanged anew when it is within
    /// [`FCM_TOKEN_SLACK`] of expiry.
    async fn access_token(&self) -> Result<String> {
        let mut cached = self.token.lock().await;
        if let Some((token, expires_at)) = cached.as_ref()
            && Instant::now() + FCM_TOKEN_SLACK < *expires_at
        {
            return Ok(token.clone());
        }
        let assertion = self.mint_assertion()?;
        let response = self
            .client
            .post(&self.token_uri)
            .form(&[
                ("grant_type", OAUTH_JWT_BEARER_GRANT),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .context("requesting an FCM access token")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("FCM token endpoint answered {status}: {body}");
        }
        let token_response: OauthTokenResponse = response
            .json()
            .await
            .context("parsing the FCM token response")?;
        let expires_at = Instant::now() + Duration::from_secs(token_response.expires_in);
        *cached = Some((token_response.access_token.clone(), expires_at));
        Ok(token_response.access_token)
    }

    /// Send `note` to every device; returns the device ids FCM reported as
    /// unregistered.
    async fn send_all(&self, devices: &[DevicePush], note: &Notification) -> Vec<i64> {
        let token = match self.access_token().await {
            Ok(token) => token,
            Err(err) => {
                warn!(error = ?err, "FCM access token unavailable; dropping notifications");
                return Vec::new();
            }
        };
        let url = format!(
            "{}/v1/projects/{}/messages:send",
            self.endpoint, self.project_id
        );
        let mut dead = Vec::new();
        for device in devices {
            if self.send_one(&url, &token, note, device).await {
                dead.push(device.device_id);
            }
        }
        dead
    }

    /// One notification to one device. Returns `true` when FCM declared the
    /// token permanently unregistered.
    async fn send_one(
        &self,
        url: &str,
        token: &str,
        note: &Notification,
        device: &DevicePush,
    ) -> bool {
        let response = self
            .client
            .post(url)
            .bearer_auth(token)
            .json(&fcm_message(note, &device.push_token))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(err) => {
                warn!(
                    device_id = device.device_id,
                    error = %err,
                    "FCM request failed; dropping notification"
                );
                return false;
            }
        };
        let status = response.status();
        if status.is_success() {
            return false;
        }
        let body = response.json::<Value>().await.unwrap_or(Value::Null);
        if fcm_unregistered(status, &body) {
            info!(
                device_id = device.device_id,
                "FCM reports the token unregistered; the device row will be deleted"
            );
            return true;
        }
        warn!(
            device_id = device.device_id,
            status = %status,
            body = %body,
            "FCM rejected the notification; dropping it"
        );
        false
    }
}

/// FCM "this token is permanently dead": HTTP 404 (the v1 API answers
/// NOT_FOUND for unregistered tokens), or an error detail carrying
/// `errorCode: UNREGISTERED`.
fn fcm_unregistered(status: reqwest::StatusCode, body: &Value) -> bool {
    if status == reqwest::StatusCode::NOT_FOUND {
        return true;
    }
    body["error"]["details"]
        .as_array()
        .is_some_and(|details| details.iter().any(|d| d["errorCode"] == "UNREGISTERED"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation, decode, decode_header};
    use serde_json::json;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

    use crate::test_keys;

    /// A test APNs sender plus the public key PEM its JWTs verify against.
    /// The `TempDir` keeps the key file alive for the sender's lifetime.
    fn test_apns_sender() -> (ApnsSender, String, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let (private_pem, public_pem) = test_keys::ec_p256_key_pair_pems();
        let key_file = dir.path().join("AuthKey_KEYID12345.p8");
        std::fs::write(&key_file, private_pem).expect("writing the test .p8");
        let cfg = ApnsConfig {
            team_id: "TEAMID9999".to_string(),
            key_id: "KEYID12345".to_string(),
            key_file,
            bundle_id: "me.nettrash.familyconnect".to_string(),
            environment: "production".to_string(),
            endpoint: None,
        };
        let sender = ApnsSender::new(&cfg, reqwest::Client::new()).expect("building ApnsSender");
        (sender, public_pem, dir)
    }

    fn test_fcm_sender() -> (FcmSender, &'static str) {
        let (_, public_pem) = test_keys::rsa_2048_key_pair_pems();
        let json = test_keys::service_account_json(
            "test-project",
            "push@test-project.iam.gserviceaccount.com",
            "https://oauth2.googleapis.com/token",
        );
        let mut f = tempfile::NamedTempFile::new().expect("tempfile");
        f.write_all(json.as_bytes()).expect("write");
        f.flush().expect("flush");
        let cfg = FcmConfig {
            credentials_file: PathBuf::from(f.path()),
            endpoint: None,
        };
        let sender = FcmSender::new(&cfg, reqwest::Client::new()).expect("building FcmSender");
        (sender, public_pem)
    }

    #[tokio::test]
    async fn the_apns_provider_jwt_carries_the_key_id_and_team_id() {
        let (sender, public_pem, _dir) = test_apns_sender();
        let before = time::OffsetDateTime::now_utc().unix_timestamp();
        let jwt = sender.provider_jwt().await.expect("provider JWT");

        let header = decode_header(&jwt).expect("JWT header decodes");
        assert_eq!(header.alg, Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("KEYID12345"));

        // APNs provider JWTs have no exp/aud — disable those checks.
        let mut validation = Validation::new(Algorithm::ES256);
        validation.validate_exp = false;
        validation.validate_aud = false;
        validation.required_spec_claims.clear();
        let decoding_key = DecodingKey::from_ec_pem(public_pem.as_bytes()).expect("public key");
        let data =
            decode::<ApnsClaims>(&jwt, &decoding_key, &validation).expect("signature verifies");
        assert_eq!(data.claims.iss, "TEAMID9999");
        let after = time::OffsetDateTime::now_utc().unix_timestamp();
        assert!(
            (before..=after).contains(&data.claims.iat),
            "iat must be the mint time: {}",
            data.claims.iat
        );
    }

    #[tokio::test]
    async fn the_apns_provider_jwt_is_cached_between_calls() {
        let (sender, _public_pem, _dir) = test_apns_sender();
        let first = sender.provider_jwt().await.expect("first JWT");
        let second = sender.provider_jwt().await.expect("second JWT");
        assert_eq!(
            first, second,
            "a fresh JWT within the refresh window would mean the cache is broken \
             (Apple throttles refreshes to ~every 20 minutes)"
        );
    }

    #[test]
    fn the_fcm_assertion_claims_follow_the_jwt_bearer_profile() {
        let (sender, public_pem) = test_fcm_sender();
        let assertion = sender.mint_assertion().expect("assertion signs");

        let header = decode_header(&assertion).expect("JWT header decodes");
        assert_eq!(header.alg, Algorithm::RS256);

        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_aud = false; // asserted against Google's endpoint, checked below
        let decoding_key = DecodingKey::from_rsa_pem(public_pem.as_bytes()).expect("public key");
        let data = decode::<FcmAssertionClaims>(&assertion, &decoding_key, &validation)
            .expect("signature verifies");
        assert_eq!(data.claims.iss, "push@test-project.iam.gserviceaccount.com");
        assert_eq!(
            data.claims.scope,
            "https://www.googleapis.com/auth/firebase.messaging"
        );
        assert_eq!(data.claims.aud, "https://oauth2.googleapis.com/token");
        assert_eq!(
            data.claims.exp - data.claims.iat,
            3600,
            "Google caps assertions at one hour"
        );
    }

    #[test]
    fn apns_unregistered_matches_410_and_bad_device_token_only() {
        use reqwest::StatusCode;
        assert!(apns_unregistered(StatusCode::GONE, None));
        assert!(apns_unregistered(StatusCode::GONE, Some("Unregistered")));
        assert!(apns_unregistered(
            StatusCode::BAD_REQUEST,
            Some("BadDeviceToken")
        ));
        assert!(!apns_unregistered(
            StatusCode::BAD_REQUEST,
            Some("PayloadTooLarge")
        ));
        assert!(!apns_unregistered(StatusCode::FORBIDDEN, None));
    }

    #[test]
    fn fcm_unregistered_matches_404_and_the_unregistered_error_code_only() {
        use reqwest::StatusCode;
        assert!(fcm_unregistered(StatusCode::NOT_FOUND, &Value::Null));
        let unregistered = json!({"error": {"status": "INVALID_ARGUMENT",
            "details": [{"errorCode": "UNREGISTERED"}]}});
        assert!(fcm_unregistered(StatusCode::BAD_REQUEST, &unregistered));
        let other = json!({"error": {"status": "INVALID_ARGUMENT",
            "details": [{"errorCode": "INVALID_ARGUMENT"}]}});
        assert!(!fcm_unregistered(StatusCode::BAD_REQUEST, &other));
        assert!(!fcm_unregistered(
            StatusCode::INTERNAL_SERVER_ERROR,
            &Value::Null
        ));
    }

    #[tokio::test]
    async fn the_log_fallback_never_reports_dead_devices() {
        let devices = vec![DevicePush {
            device_id: 1,
            user_id: 2,
            platform: "ios".to_string(),
            push_token: "tok".to_string(),
        }];
        let note = crate::push_payload::joined_notification("The Smiths", 7, 0);
        assert!(LogPushSender.notify(&devices, &note).await.is_empty());
    }
}
