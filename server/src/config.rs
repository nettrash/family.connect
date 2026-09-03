//! Configuration loading and validation.
//!
//! Every key has a compiled-in default matching `config.example.toml`, so an
//! empty file (or missing sections) is a valid configuration — the example
//! file is documentation, not a requirement. Validation happens once at
//! startup and fails fast with a precise message rather than letting a bad
//! value surface later as a confusing runtime error (e.g. an idle timeout
//! shorter than the ping interval would silently kill every socket).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use jsonwebtoken::EncodingKey;
use serde::Deserialize;

/// Top-level configuration document.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,

    #[serde(default)]
    pub database: DatabaseConfig,

    #[serde(default)]
    pub auth: AuthConfig,

    #[serde(default)]
    pub limits: LimitsConfig,

    #[serde(default)]
    pub push: PushConfig,

    #[serde(default)]
    pub storage: StorageConfig,

    #[serde(default)]
    pub ai: AiConfig,

    #[serde(default)]
    pub calls: CallsConfig,
}

/// `[calls]` — peer-to-peer voice calls (docs/protocol.md, "Voice calls").
///
/// The server only SIGNALS a call: it passes offers, answers and candidates
/// between two members over the sockets they already hold, and the audio
/// goes directly between the two devices. What this section configures is
/// therefore not the calls but the ICE servers the clients are handed —
/// STUN so two phones behind home routers can find each other, and TURN as
/// the relay of last resort for the networks where they cannot.
///
/// ON by default with a public STUN list, because a voice feature that only
/// works inside one Wi-Fi network is not a voice feature. A STUN binding
/// request carries no family data, but an operator who would rather not
/// name Google's servers points `stun_urls` at their own or empties it.
#[derive(Debug, Clone, Deserialize)]
pub struct CallsConfig {
    /// `false` turns signalling off entirely: `call_offer` answers
    /// `calls_disabled`, `GET /calls/ice` does too, and `GET /me` reports
    /// `calls_enabled: false` so clients hide their call button.
    #[serde(default = "default_calls_enabled")]
    pub enabled: bool,

    /// Video calls (protocol.md, "Video"), meaningful only with calls
    /// enabled. `false` refuses a `call_offer` carrying `"video": true`
    /// with `video_calls_disabled` and leaves voice calls untouched;
    /// `GET /me` reports the combined answer as `video_calls_enabled`. On
    /// by default — the switch exists for the operator whose TURN relay is
    /// sized for voice (~10 kB/s per call; video is two orders more).
    #[serde(default = "default_video_calls_enabled")]
    pub video_enabled: bool,

    /// STUN servers, as `stun:` / `stuns:` URLs.
    #[serde(default = "default_stun_urls")]
    pub stun_urls: Vec<String>,

    /// TURN servers, as `turn:` / `turns:` URLs. Empty means no relay: calls
    /// that cannot connect directly simply fail.
    #[serde(default)]
    pub turn_urls: Vec<String>,

    /// coturn's `static-auth-secret`. When set, every `GET /calls/ice` mints
    /// a time-limited credential for the caller: username
    /// `<expiry>:<user_id>`, credential = base64(HMAC-SHA1(secret, username)).
    #[serde(default)]
    pub turn_secret: String,

    /// The static long-term alternative, used when `turn_secret` is empty.
    #[serde(default)]
    pub turn_username: String,
    #[serde(default)]
    pub turn_password: String,

    /// How long a minted TURN credential is good for.
    #[serde(default = "default_turn_credential_ttl_secs")]
    pub turn_credential_ttl_secs: u64,

    /// How long a call rings before the server ends it as `timeout`
    /// (protocol.md fixes the default at 45 s).
    #[serde(default = "default_ring_timeout_secs")]
    pub ring_timeout_secs: u64,
}

impl CallsConfig {
    /// Whether a TURN entry will carry any credentials at all.
    pub fn has_turn_credentials(&self) -> bool {
        !self.turn_secret.is_empty()
            || (!self.turn_username.is_empty() && !self.turn_password.is_empty())
    }
}

impl Default for CallsConfig {
    fn default() -> Self {
        Self {
            enabled: default_calls_enabled(),
            video_enabled: default_video_calls_enabled(),
            stun_urls: default_stun_urls(),
            turn_urls: Vec::new(),
            turn_secret: String::new(),
            turn_username: String::new(),
            turn_password: String::new(),
            turn_credential_ttl_secs: default_turn_credential_ttl_secs(),
            ring_timeout_secs: default_ring_timeout_secs(),
        }
    }
}

/// `[ai]` — the assistant each member can have a private chat with
/// (docs/protocol.md, "The assistant").
///
/// OFF unless configured. A server with no endpoint, deployment or key
/// simply never creates the chat, so the feature costs nothing to leave
/// alone — which is the right default for something that sends text to a
/// third party.
///
/// The key belongs HERE, in the server's own config file, and never on a
/// client: a key shipped to devices is a key that has left your control.
/// The config file is read verbatim — there is no `${VAR}` expansion — so
/// the file itself is the secret: keep it out of version control and
/// readable only by the service user.
#[derive(Debug, Clone, Deserialize)]
pub struct AiConfig {
    /// Master switch. False (the default) means the assistant does not
    /// exist: no chat is created and nothing is ever sent anywhere.
    #[serde(default)]
    pub enabled: bool,

    /// Azure OpenAI resource endpoint, e.g.
    /// `https://my-resource.openai.azure.com`. No trailing slash needed.
    #[serde(default)]
    pub endpoint: String,

    /// The DEPLOYMENT name, which is what the URL is built from — not the
    /// model name. Azure lets these differ and usually they do.
    #[serde(default)]
    pub deployment: String,

    /// The model behind the deployment, e.g. `gpt-oss-120b`. Sent in the
    /// request body and recorded with usage, so statistics can say what
    /// answered.
    #[serde(default)]
    pub model: String,

    /// `api-key` header. Never logged, never sent to a client.
    #[serde(default)]
    pub api_key: String,

    /// Azure's dated API version. Pinned rather than "latest": a silently
    /// changing contract is not something a family server should discover
    /// in production.
    #[serde(default = "default_ai_api_version")]
    pub api_version: String,

    /// What the assistant is told it is, before the member's own words.
    #[serde(default = "default_ai_system_prompt")]
    pub system_prompt: String,

    /// Longest reply, in tokens. A cap the server owns rather than the
    /// model, so one long answer cannot run up a bill.
    #[serde(default = "default_ai_max_tokens")]
    pub max_tokens: u32,

    /// How many earlier messages of that member's OWN assistant chat go
    /// with a question. Nothing else is ever included — not the family
    /// chat, not another member's thread (protocol.md).
    #[serde(default = "default_ai_history_messages")]
    pub history_messages: i64,

    /// What the chat is called in the list.
    #[serde(default = "default_ai_title")]
    pub title: String,

    /// `[ai.vision]` — the deployment that can LOOK at a photograph
    /// (protocol.md, "Pictures"). Absent means the assistant has no eyes:
    /// a picture a member attaches is a `[photo]` marker and nothing more,
    /// which is also the default.
    #[serde(default)]
    pub vision: AiDeployment,

    /// `[ai.images]` — the deployment that MAKES one, reached by `/draw`.
    /// Absent means the assistant has no hands and clients are told not to
    /// offer the affordance.
    #[serde(default)]
    pub images: AiImagesConfig,
}

/// A second deployment on the same provider: only what DIFFERS from `[ai]`.
///
/// Every field falls back to the `[ai]` section when it is empty, because in
/// practice the three deployments a family server talks to are three
/// deployments on ONE resource — the same host, the same key, the same dated
/// api-version, and only the name changes. Making an operator paste the key
/// three times is three chances to paste two of them right.
///
/// `deployment` (or an `endpoint` of its own) is what switches the
/// capability ON. Neither set means the capability does not exist on this
/// server, which is what `GET /families/mine` reports to clients as
/// `assistant.vision` / `assistant.images`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AiDeployment {
    /// Overrides `[ai] endpoint`. Set it when this model answers somewhere
    /// else entirely — a Foundry target URI pasted whole, for instance.
    #[serde(default)]
    pub endpoint: String,
    /// The DEPLOYMENT name, which is what routes the request. Setting this
    /// (or `endpoint`) is what turns the capability on.
    #[serde(default)]
    pub deployment: String,
    /// The model behind it, recorded with usage rather than sent.
    #[serde(default)]
    pub model: String,
    /// Overrides `[ai] api_key`.
    #[serde(default)]
    pub api_key: String,
    /// Overrides `[ai] api_version`.
    #[serde(default)]
    pub api_version: String,
}

/// `[ai.images]` — a deployment plus the two knobs an images endpoint has
/// that a chat one does not.
///
/// Both knobs exist because the images surface is the one place this server
/// cannot verify the contract from here: an operator points it at whatever
/// their portal gave them, and a body that model rejects comes back as an
/// opaque 400. So the two fields that vary between image models are
/// CONFIGURABLE and both are omitted from the request when empty, which is
/// the shape that works everywhere.
#[derive(Debug, Clone, Deserialize)]
pub struct AiImagesConfig {
    #[serde(flatten)]
    pub deployment: AiDeployment,

    /// `size` in the request body — `"1024x1024"` and so on. Empty omits
    /// the field, which lets the deployment choose its own default.
    #[serde(default = "default_ai_image_size")]
    pub size: String,

    /// `response_format`. `"b64_json"` asks for the bytes inline;
    /// `"url"` asks for a link the server then fetches. EMPTY BY DEFAULT and
    /// omitted from the request when empty, because some image deployments
    /// answer 400 to a field they do not implement — and both answers are
    /// parsed whichever way they arrive, so asking for neither is safe.
    #[serde(default)]
    pub response_format: String,

    /// The most bytes one generated picture may be. A ceiling the SERVER
    /// owns, like `max_tokens`: it bounds what a provider can write onto
    /// the family's disk, and it bounds the download when a deployment
    /// answers with a URL instead of bytes.
    #[serde(default = "default_ai_image_max_bytes")]
    pub max_bytes: usize,
}

/// One resolved provider call: where to POST it, which key opens it, what to
/// name in the body, and the cap the server owns.
///
/// Its own type because there are now three of them and they differ in every
/// field. Building it is the only place the endpoint/deployment/key of a
/// request are decided, so "which model saw this?" has exactly one answer to
/// read (protocol.md, "The three deployments").
#[derive(Debug, Clone)]
pub struct ModelRoute {
    pub url: String,
    pub api_key: String,
    /// The `model` field of the request body — on Azure the DEPLOYMENT name,
    /// which is what the v1 surface routes on and what the classic surface
    /// ignores.
    pub model: String,
    pub max_tokens: u32,
}

/// Hand-written rather than derived, so `AiConfig::default()` agrees with
/// what serde fills in for a half-written section.
///
/// A derived `Default` gives `max_tokens: 0`, an empty `api_version` and an
/// empty prompt — values the serde attributes above would never produce.
/// Nothing reads them today (an absent section is `enabled: false`, and
/// `is_usable()` gates every use), but a struct whose two default paths
/// disagree is a trap waiting for the first caller who writes
/// `..Default::default()` and gets a URL with no api-version in it.
impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            deployment: String::new(),
            model: String::new(),
            api_key: String::new(),
            api_version: default_ai_api_version(),
            system_prompt: default_ai_system_prompt(),
            max_tokens: default_ai_max_tokens(),
            history_messages: default_ai_history_messages(),
            title: default_ai_title(),
            vision: AiDeployment::default(),
            images: AiImagesConfig::default(),
        }
    }
}

/// Hand-written for the same reason [`AiConfig`]'s is: a derived `Default`
/// would give an empty `size` and a zero `max_bytes`, neither of which serde
/// would ever produce for a half-written `[ai.images]` section.
impl Default for AiImagesConfig {
    fn default() -> Self {
        Self {
            deployment: AiDeployment::default(),
            size: default_ai_image_size(),
            response_format: String::new(),
            max_bytes: default_ai_image_max_bytes(),
        }
    }
}

fn default_ai_image_size() -> String {
    "1024x1024".to_string()
}

/// 16 MiB. Far above any square PNG an image model returns and far below
/// `limits.max_attachment_bytes`, so it is a guard against a runaway
/// response rather than a limit a family will ever meet.
fn default_ai_image_max_bytes() -> usize {
    16 * 1024 * 1024
}

fn default_ai_api_version() -> String {
    "2024-10-21".to_string()
}

fn default_ai_system_prompt() -> String {
    "You are a helpful assistant in a family's private chat app. Answer briefly \
     and plainly. You can only see this one conversation."
        .to_string()
}

fn default_ai_max_tokens() -> u32 {
    1024
}

fn default_ai_history_messages() -> i64 {
    20
}

fn default_ai_title() -> String {
    "Assistant".to_string()
}

impl AiConfig {
    /// Configured well enough to actually call. `enabled` alone is not
    /// enough — a half-filled section should behave like "off" rather than
    /// failing every question at runtime.
    pub fn is_usable(&self) -> bool {
        self.enabled
            && !self.endpoint.trim().is_empty()
            && !self.deployment.trim().is_empty()
            && !self.api_key.trim().is_empty()
    }

    /// The chat-completions URL for the configured deployment.
    /// The chat-completions URL for the configured deployment.
    ///
    /// Azure has more than one shape. Classic Azure OpenAI is
    /// `{endpoint}/openai/deployments/{deployment}/chat/completions`, which
    /// is what this builds — but an AI Foundry / serverless deployment
    /// answers on a different path entirely, and asking the wrong one gives
    /// a bare `404 Resource not found` with nothing to say which part was
    /// wrong.
    ///
    /// So an `endpoint` that ALREADY names a completions path is used
    /// verbatim: paste the target URI the portal shows and nothing has to be
    /// guessed. `api-version` is still appended when the pasted URL has no
    /// query of its own.
    pub fn completions_url(&self) -> String {
        azure_url(
            &self.endpoint,
            &self.deployment,
            &self.api_version,
            "chat/completions",
        )
    }

    /// What to put in the request body's `model` field.
    ///
    /// On Azure this is the DEPLOYMENT name, not the model name — the v1
    /// surface routes on it, and the classic surface ignores the field
    /// because the deployment is already in the path. `model` in the config
    /// exists to record what answered, for statistics, and is only used
    /// here when no deployment is configured at all (a pasted full URL that
    /// needs no routing).
    pub fn request_model(&self) -> &str {
        if self.deployment.trim().is_empty() {
            self.model.trim()
        } else {
            self.deployment.trim()
        }
    }

    /// The TEXT deployment: `[ai]` itself, unchanged from before there were
    /// three of them. A server that configures nothing else behaves exactly
    /// as it always did, which is what makes pictures additive.
    pub fn text_route(&self) -> ModelRoute {
        ModelRoute {
            url: self.completions_url(),
            api_key: self.api_key.trim().to_string(),
            model: self.request_model().to_string(),
            max_tokens: self.max_tokens,
        }
    }

    /// The deployment that can look at a photograph, or `None` when the
    /// operator configured none.
    ///
    /// `None` is not an error anywhere: it means a picture attached to a
    /// question stays a `[photo]` marker, which is what the assistant is
    /// then told (protocol.md, "Pictures").
    pub fn vision_route(&self) -> Option<ModelRoute> {
        if !self.is_usable() || !self.vision.is_configured() {
            return None;
        }
        Some(ModelRoute {
            url: azure_url(
                self.vision.endpoint_or(&self.endpoint),
                self.vision.deployment_or(&self.deployment),
                self.vision.api_version_or(&self.api_version),
                "chat/completions",
            ),
            api_key: self.vision.api_key_or(&self.api_key).trim().to_string(),
            model: self.vision.request_model_or(&self.deployment).to_string(),
            // The same cap the text deployment obeys: a description of a
            // photograph is still an answer in a family chat.
            max_tokens: self.max_tokens,
        })
    }

    /// The deployment that MAKES a picture, or `None`.
    ///
    /// `max_tokens` is meaningless to an images endpoint and is carried
    /// anyway rather than being made an `Option`: one route type with a
    /// field an image call ignores is simpler to read than two types, and
    /// [`AiImagesConfig::max_bytes`] is the cap that actually binds here.
    pub fn images_route(&self) -> Option<ModelRoute> {
        let images = &self.images;
        if !self.is_usable() || !images.deployment.is_configured() {
            return None;
        }
        Some(ModelRoute {
            url: azure_url(
                images.deployment.endpoint_or(&self.endpoint),
                images.deployment.deployment_or(&self.deployment),
                images.deployment.api_version_or(&self.api_version),
                "images/generations",
            ),
            api_key: images
                .deployment
                .api_key_or(&self.api_key)
                .trim()
                .to_string(),
            model: images
                .deployment
                .request_model_or(&self.deployment)
                .to_string(),
            max_tokens: self.max_tokens,
        })
    }

    /// Whether this server can look at a picture at all — the answer sent to
    /// clients as `assistant.vision`.
    pub fn vision_usable(&self) -> bool {
        self.vision_route().is_some()
    }

    /// Whether it can make one — sent as `assistant.images`.
    pub fn images_usable(&self) -> bool {
        self.images_route().is_some()
    }
}

impl AiDeployment {
    /// Configured at all: the operator named a deployment, or an endpoint of
    /// its own. Either is enough — a pasted Foundry target URI needs no
    /// deployment name, and a deployment on the same resource needs no
    /// endpoint.
    pub fn is_configured(&self) -> bool {
        !self.deployment.trim().is_empty() || !self.endpoint.trim().is_empty()
    }

    fn endpoint_or<'a>(&'a self, parent: &'a str) -> &'a str {
        pick(&self.endpoint, parent)
    }

    fn deployment_or<'a>(&'a self, parent: &'a str) -> &'a str {
        pick(&self.deployment, parent)
    }

    fn api_key_or<'a>(&'a self, parent: &'a str) -> &'a str {
        pick(&self.api_key, parent)
    }

    fn api_version_or<'a>(&'a self, parent: &'a str) -> &'a str {
        pick(&self.api_version, parent)
    }

    /// What goes in the body's `model` field for this deployment, under the
    /// rule [`AiConfig::request_model`] states: the DEPLOYMENT name routes,
    /// and `model` is only a record of what answered.
    fn request_model_or<'a>(&'a self, parent_deployment: &'a str) -> &'a str {
        let deployment = self.deployment_or(parent_deployment).trim();
        if deployment.is_empty() {
            self.model.trim()
        } else {
            deployment
        }
    }
}

/// The sub-section's value when it has one, else the `[ai]` section's.
fn pick<'a>(own: &'a str, parent: &'a str) -> &'a str {
    if own.trim().is_empty() { parent } else { own }
}

/// Build a provider URL for one of Azure's several endpoint shapes.
///
/// Shared by chat completions and image generations because the shapes are
/// the SAME three shapes and only the last path segment differs — and
/// because getting one of them right and the other wrong is the failure this
/// whole function exists to prevent: a bare `404 Resource not found` that
/// cannot say which part was wrong.
///
/// - an endpoint that ALREADY names this path is used verbatim (paste the
///   target URI the portal shows), with `api-version` appended only when it
///   carries no query of its own;
/// - an endpoint ending in `/openai/v1` is Azure's OpenAI-COMPATIBLE
///   surface: the deployment goes in the BODY, not the path, and there is no
///   `api-version` at all. Splicing `/openai/deployments/…` onto one of
///   these gives a doubled `/openai` and that same bare 404 — which is
///   exactly what happened in production once already;
/// - anything else is classic Azure OpenAI: deployment in the path, dated
///   api-version in the query.
fn azure_url(endpoint: &str, deployment: &str, api_version: &str, path: &str) -> String {
    let base = endpoint.trim().trim_end_matches('/');

    if base.contains(&format!("/{path}")) {
        if base.contains('?') || api_version.trim().is_empty() {
            return base.to_string();
        }
        return format!("{base}?api-version={api_version}");
    }

    if base.ends_with("/openai/v1") {
        return format!("{base}/{path}");
    }

    format!("{base}/openai/deployments/{deployment}/{path}?api-version={api_version}")
}

/// `[storage]` — where attachment bytes live.
///
/// On disk, not in PostgreSQL: see migration 0009. The default matches the
/// systemd unit's `StateDirectory=family-connect`, which has been reserved
/// for exactly this since the first release.
#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_attachments_dir")]
    pub attachments_dir: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            attachments_dir: default_attachments_dir(),
        }
    }
}

fn default_attachments_dir() -> PathBuf {
    PathBuf::from("/var/lib/family-connect/attachments")
}

/// `[server]` — the HTTP + WebSocket listener.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Bind address, e.g. `127.0.0.1:8080`. Loopback in production; nginx
    /// terminates TLS and proxies to this address.
    #[serde(default = "default_bind")]
    pub bind: String,

    /// How a member reaches the operator — an email address, a URL, a
    /// sentence. Served to clients on `GET /me` as `support_contact` and
    /// shown on the report screen.
    ///
    /// It exists because the family owner is the moderator, and the owner
    /// is sometimes the problem: a report naming them never reaches them
    /// (protocol.md, "Reporting a member"), so there has to be somewhere
    /// else to go. `None` when unset, and the key is then absent from
    /// `GET /me` rather than empty — a client draws no escalation line at
    /// all rather than an address nobody reads.
    #[serde(default)]
    pub support_contact: Option<String>,
}

/// `[database]` — PostgreSQL connection parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_host")]
    pub host: String,

    #[serde(default = "default_db_port")]
    pub port: u16,

    #[serde(default = "default_db_user")]
    pub user: String,

    /// Empty by default: local trust/peer auth setups need no password.
    #[serde(default)]
    pub password: String,

    #[serde(default = "default_db_database")]
    pub database: String,

    #[serde(default = "default_db_max_connections")]
    pub max_connections: u32,

    #[serde(default = "default_db_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
}

/// `[auth]` — session lifetime policy.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// Sliding session TTL. Every "touch" pushes expiry out this far again.
    #[serde(default = "default_session_ttl_days")]
    pub session_ttl_days: i64,

    /// Minimum age of `last_used_at` before a request triggers a renewal
    /// write — batches the sliding-expiry UPDATEs so chatty clients don't
    /// rewrite their session row on every request.
    #[serde(default = "default_session_touch_interval_mins")]
    pub session_touch_interval_mins: i64,
}

/// `[limits]` — protocol limits (protocol.md "Limits" table).
#[derive(Debug, Clone, Deserialize)]
pub struct LimitsConfig {
    /// Maximum message body length in characters (not bytes).
    #[serde(default = "default_max_message_chars")]
    pub max_message_chars: usize,

    /// Most options one poll may offer. The FEWEST is not configurable —
    /// it is [`crate::models::Poll::MIN_OPTIONS`], because a poll with one
    /// option is a statement rather than a question, and no operator wants
    /// that to be a setting.
    #[serde(default = "default_max_poll_options")]
    pub max_poll_options: usize,

    /// Longest poll option, in characters (not bytes). A button label, not
    /// a message — the QUESTION is the message body and keeps
    /// `max_message_chars`.
    #[serde(default = "default_max_poll_option_chars")]
    pub max_poll_option_chars: usize,

    /// Largest photo or video accepted, in bytes. 100 MB by default: a
    /// minute of phone video, and the clients re-encode past that rather
    /// than refusing outright.
    #[serde(default = "default_max_attachment_bytes")]
    pub max_attachment_bytes: usize,

    /// Most attachments one message may claim (protocol.md, "Photos,
    /// videos, audio, files and locations"). Ten by default. The FEWEST is
    /// not configurable — it is 1, because a message that carries no
    /// attachment is simply an ordinary message, not a state an operator
    /// could allow or forbid.
    #[serde(default = "default_max_attachments_per_message")]
    pub max_attachments_per_message: usize,

    /// Largest PREVIEW accepted (the downscaled photo or poster frame).
    /// Small on purpose — it is drawn in a chat bubble.
    #[serde(default = "default_max_preview_bytes")]
    pub max_preview_bytes: usize,

    /// How long an UNCLAIMED upload survives before the sweeper deletes
    /// it. A send the user abandoned must not leave 100 MB behind.
    #[serde(default = "default_attachment_grace_hours")]
    pub attachment_grace_hours: i64,

    /// How long a message is kept before the retention sweep deletes it,
    /// along with any attachment it owns. 100 days by default.
    ///
    /// **0 disables the sweep entirely** — a family that wants to keep
    /// everything sets 0 rather than a very large number, so "off" is a
    /// state rather than an arithmetic accident.
    ///
    /// This deletes MESSAGES and their attachments. Board notes are not
    /// touched: a sticker note is a live thing on a wall, not history, and
    /// having one vanish because nobody moved it for a season is not what
    /// anybody means by clearing old content.
    #[serde(default = "default_retention_days")]
    pub retention_days: i64,

    /// Most notes one family board may hold at once (tombstones excluded).
    /// A runaway guard, not a budget: a wall nobody could read is not a
    /// feature anyone asked for.
    #[serde(default = "default_max_board_notes")]
    pub max_board_notes: i64,

    /// The CEILING on what a family owner may set as their own
    /// `max_members`, and the cap that binds at the join door for a family
    /// that has set none. It is an operator's runaway guard, in the sense
    /// `max_board_notes` is — NOT the number the apps show, which is the
    /// family's own `max_members` (protocol.md, "Families").
    ///
    /// Lowering it never invalidates a family that is already larger, and
    /// stored caps are deliberately NOT re-validated against it at boot: a
    /// family whose cap was legal when it was set must stay patchable, or
    /// an operator editing one line of config locks owners out of their own
    /// settings screen.
    #[serde(default = "default_max_family_members")]
    pub max_family_members: i64,

    /// Page size for GET /chats/{id}/messages when the client sends none.
    #[serde(default = "default_default_page_size")]
    pub default_page_size: i64,

    /// Ceiling a client-supplied limit is clamped to.
    #[serde(default = "default_max_page_size")]
    pub max_page_size: i64,

    /// Maximum accepted HTTP request body in bytes.
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,

    /// Maximum accepted profile picture in bytes. Applies to
    /// `PUT /me/avatar` ONLY, which overrides `max_body_bytes` — that
    /// limit stays small for every JSON route.
    #[serde(default = "default_max_avatar_bytes")]
    pub max_avatar_bytes: usize,

    /// Free space on the attachments filesystem that uploads may not eat
    /// into, in bytes. 0 disables the check.
    ///
    /// This is a floor for the DATABASE, not a budget for attachments.
    /// PostgreSQL usually shares the filesystem and handles a full disk far
    /// less gracefully than a refused upload does — so uploads stop while
    /// there is still room, and the family is told `storage_full` (507)
    /// rather than the server falling over.
    #[serde(default = "default_min_free_disk_bytes")]
    pub min_free_disk_bytes: u64,

    /// How many password hashes may run at once.
    ///
    /// `Argon2::default()` allocates 19 MiB per hash, and a bare
    /// `#[tokio::main]` gives the blocking pool 512 threads — so an
    /// unbounded login flood can ask for ~9.7 GiB and take the process out.
    /// Both unauthenticated auth endpoints reach this (login deliberately
    /// hashes even for an unknown username, to close a timing oracle), so
    /// the bound is what stops a stranger choosing how much memory the
    /// server allocates. Waiting requests queue rather than being refused
    /// — arrival RATE is nginx's `limit_req` to control, not this.
    #[serde(default = "default_max_password_hashes_in_flight")]
    pub max_password_hashes_in_flight: usize,

    /// Outbound frames buffered per WebSocket before the connection is
    /// declared too slow and dropped.
    #[serde(default = "default_ws_send_queue")]
    pub ws_send_queue: usize,

    /// Protocol-level ping cadence.
    #[serde(default = "default_ws_ping_interval_secs")]
    pub ws_ping_interval_secs: u64,

    /// Idle cutoff after which a silent socket is closed.
    #[serde(default = "default_ws_idle_timeout_secs")]
    pub ws_idle_timeout_secs: u64,
}

/// `[push]` — push notification delivery.
///
/// Each platform's transport is enabled by the *presence* of its section:
/// `[push.apns]` for iOS devices, `[push.fcm]` for Android. A platform with
/// no section falls back to logging the would-be notification (the v1
/// behavior), so a server without push credentials keeps working unchanged.
#[derive(Debug, Clone, Deserialize)]
pub struct PushConfig {
    /// Deprecated v1 knob. Only `"log"` was ever valid and it is now the
    /// implicit fallback; the key is accepted (and ignored) so existing
    /// config files keep loading, but any other value still fails fast.
    #[serde(default)]
    pub driver: Option<String>,

    /// `true` (default): notification bodies carry the message text.
    /// `false`: the body is always `"New message"` — for households that
    /// consider lock screens too public for message content (protocol.md).
    #[serde(default = "default_include_message_body")]
    pub include_message_body: bool,

    /// APNs transport for iOS devices; absent = log fallback.
    #[serde(default)]
    pub apns: Option<ApnsConfig>,

    /// FCM transport for Android devices; absent = log fallback.
    #[serde(default)]
    pub fcm: Option<FcmConfig>,
}

/// `[push.apns]` — token-based APNs authentication (an Apple Developer
/// `.p8` auth key, not certificates: one key serves every app and never
/// expires).
#[derive(Debug, Clone, Deserialize)]
pub struct ApnsConfig {
    /// Apple Developer Team ID — the `iss` claim of the provider JWT.
    pub team_id: String,

    /// Key ID of the APNs auth key — the JWT header's `kid`.
    pub key_id: String,

    /// Path to the downloaded `AuthKey_<KEYID>.p8` (a PEM-encoded PKCS#8
    /// P-256 private key).
    pub key_file: PathBuf,

    /// The app's bundle id — sent as the `apns-topic` header.
    pub bundle_id: String,

    /// `"production"` (App Store / TestFlight builds) or `"sandbox"` (Xcode
    /// development builds). Tokens are environment-specific: the wrong
    /// environment answers `BadDeviceToken`.
    #[serde(default = "default_apns_environment")]
    pub environment: String,

    /// Explicit endpoint override for tests; when absent the endpoint is
    /// derived from `environment`.
    #[serde(default)]
    pub endpoint: Option<String>,
}

impl ApnsConfig {
    /// The base URL notifications are POSTed to, without a trailing slash.
    pub fn endpoint(&self) -> String {
        self.endpoint.clone().unwrap_or_else(|| {
            if self.environment == "sandbox" {
                "https://api.sandbox.push.apple.com".to_string()
            } else {
                "https://api.push.apple.com".to_string()
            }
        })
    }

    /// Read and parse the `.p8` signing key. Called by `validate()` so a
    /// missing or malformed key fails at startup, and again by the APNs
    /// transport when it is built.
    pub fn read_signing_key(&self) -> Result<EncodingKey> {
        let pem = std::fs::read(&self.key_file)
            .with_context(|| format!("reading push.apns.key_file {}", self.key_file.display()))?;
        EncodingKey::from_ec_pem(&pem).with_context(|| {
            format!(
                "push.apns.key_file {} is not a PEM-encoded P-256 private key (.p8)",
                self.key_file.display()
            )
        })
    }
}

/// `[push.fcm]` — FCM HTTP v1 with OAuth2 service-account authentication.
#[derive(Debug, Clone, Deserialize)]
pub struct FcmConfig {
    /// Path to the Firebase service-account JSON (Project settings →
    /// Service accounts → "Generate new private key").
    pub credentials_file: PathBuf,

    /// Explicit endpoint override for tests; defaults to the real FCM host.
    #[serde(default)]
    pub endpoint: Option<String>,
}

impl FcmConfig {
    /// The base URL `…/v1/projects/{project}/messages:send` is appended to,
    /// without a trailing slash.
    pub fn endpoint(&self) -> String {
        self.endpoint
            .clone()
            .unwrap_or_else(|| "https://fcm.googleapis.com".to_string())
    }

    /// Read and parse the service-account file, verifying every field the
    /// transport needs is present and the private key actually signs.
    /// Called by `validate()` and again when the FCM transport is built.
    pub fn read_service_account(&self) -> Result<ServiceAccount> {
        let raw = std::fs::read_to_string(&self.credentials_file).with_context(|| {
            format!(
                "reading push.fcm.credentials_file {}",
                self.credentials_file.display()
            )
        })?;
        let account: ServiceAccount = serde_json::from_str(&raw).with_context(|| {
            format!(
                "parsing service-account JSON {}",
                self.credentials_file.display()
            )
        })?;
        for (name, value) in [
            ("project_id", &account.project_id),
            ("client_email", &account.client_email),
            ("private_key", &account.private_key),
            ("token_uri", &account.token_uri),
        ] {
            if value.is_empty() {
                anyhow::bail!(
                    "service account {} has an empty {name}",
                    self.credentials_file.display()
                );
            }
        }
        account.signing_key().with_context(|| {
            format!(
                "service account {} private_key is not a PEM-encoded RSA key",
                self.credentials_file.display()
            )
        })?;
        Ok(account)
    }
}

/// The subset of a Google service-account JSON the FCM transport uses.
/// Unknown fields (there are many) are ignored. Deliberately no `Debug`:
/// `private_key` is a credential and must never reach a log.
#[derive(Clone, Deserialize)]
pub struct ServiceAccount {
    pub project_id: String,
    pub client_email: String,
    pub private_key: String,
    pub token_uri: String,
}

impl ServiceAccount {
    /// The RS256 signing key for the OAuth2 JWT-bearer assertion.
    pub fn signing_key(&self) -> Result<EncodingKey> {
        EncodingKey::from_rsa_pem(self.private_key.as_bytes())
            .context("parsing the service-account private_key as an RSA PEM")
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            support_contact: None,
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            host: default_db_host(),
            port: default_db_port(),
            user: default_db_user(),
            password: String::new(),
            database: default_db_database(),
            max_connections: default_db_max_connections(),
            connect_timeout_secs: default_db_connect_timeout_secs(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            session_ttl_days: default_session_ttl_days(),
            session_touch_interval_mins: default_session_touch_interval_mins(),
        }
    }
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_message_chars: default_max_message_chars(),
            max_poll_options: default_max_poll_options(),
            max_poll_option_chars: default_max_poll_option_chars(),
            max_board_notes: default_max_board_notes(),
            max_family_members: default_max_family_members(),
            max_attachment_bytes: default_max_attachment_bytes(),
            max_attachments_per_message: default_max_attachments_per_message(),
            max_preview_bytes: default_max_preview_bytes(),
            attachment_grace_hours: default_attachment_grace_hours(),
            retention_days: default_retention_days(),
            default_page_size: default_default_page_size(),
            max_page_size: default_max_page_size(),
            max_body_bytes: default_max_body_bytes(),
            max_avatar_bytes: default_max_avatar_bytes(),
            min_free_disk_bytes: default_min_free_disk_bytes(),
            max_password_hashes_in_flight: default_max_password_hashes_in_flight(),
            ws_send_queue: default_ws_send_queue(),
            ws_ping_interval_secs: default_ws_ping_interval_secs(),
            ws_idle_timeout_secs: default_ws_idle_timeout_secs(),
        }
    }
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            driver: None,
            include_message_body: default_include_message_body(),
            apns: None,
            fcm: None,
        }
    }
}

impl Config {
    /// Read, parse, and validate a config file.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        Self::from_toml_str(&raw).with_context(|| format!("in config file {}", path.display()))
    }

    /// Parse and validate from a TOML string (shared by `load` and tests).
    pub fn from_toml_str(raw: &str) -> Result<Self> {
        let cfg: Config = toml::from_str(raw).context("parsing TOML config")?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject configurations that would misbehave at runtime.
    pub fn validate(&self) -> Result<()> {
        self.server.bind.parse::<SocketAddr>().with_context(|| {
            format!("server.bind is not a socket address: {}", self.server.bind)
        })?;
        if self.database.host.is_empty() {
            anyhow::bail!("database.host must not be empty");
        }
        if self.database.user.is_empty() {
            anyhow::bail!("database.user must not be empty");
        }
        if self.database.database.is_empty() {
            anyhow::bail!("database.database must not be empty");
        }
        if self.database.max_connections == 0 {
            anyhow::bail!("database.max_connections must be at least 1");
        }
        if self.auth.session_ttl_days < 1 {
            anyhow::bail!("auth.session_ttl_days must be at least 1");
        }
        if self.auth.session_touch_interval_mins < 1 {
            anyhow::bail!("auth.session_touch_interval_mins must be at least 1");
        }
        if self.limits.max_message_chars == 0 {
            anyhow::bail!("limits.max_message_chars must be at least 1");
        }
        // Not "at least 1": a poll needs two options to be a poll at all,
        // and the floor is the model's constant so the two cannot drift.
        if self.limits.max_poll_options < crate::models::Poll::MIN_OPTIONS {
            anyhow::bail!(
                "limits.max_poll_options must be at least {} — a poll with fewer \
                 options than that is not a question",
                crate::models::Poll::MIN_OPTIONS
            );
        }
        if self.limits.max_poll_option_chars == 0 {
            anyhow::bail!("limits.max_poll_option_chars must be at least 1");
        }
        // The fewest is 1 and fixed (protocol.md's Limits table): zero
        // would make every attachment send invalid, which no operator
        // means by lowering a ceiling.
        if self.limits.max_attachments_per_message < 1 {
            anyhow::bail!("limits.max_attachments_per_message must be at least 1");
        }
        // An upper bound as well: the claim stamps each attachment's
        // `position` into a SMALLINT (migration 0025), and `as i16` on an
        // unbounded index would wrap negative past 32767 and reverse the
        // album's read order. 100 is already ten times the protocol's
        // default; nothing about a chat bubble wants more.
        if self.limits.max_attachments_per_message > 100 {
            anyhow::bail!(
                "limits.max_attachments_per_message must be at most 100 — attachment \
                 positions are stored as SMALLINT and clients lay albums out for ten"
            );
        }
        // One is the fewest a family can have and still have an owner:
        // `create_family` makes its creator member #1. A ceiling of 0 is
        // not a locked-down server, it is a server where `POST /families`
        // answers `family_full` to everybody — a code that endpoint does
        // not document — and where every `PATCH` of a cap is refused with
        // "between 1 and 0".
        if self.limits.max_family_members < 1 {
            anyhow::bail!("limits.max_family_members must be at least 1");
        }
        if self.limits.default_page_size < 1 {
            anyhow::bail!("limits.default_page_size must be at least 1");
        }
        if self.limits.max_page_size < self.limits.default_page_size {
            anyhow::bail!(
                "limits.max_page_size ({}) must be >= limits.default_page_size ({})",
                self.limits.max_page_size,
                self.limits.default_page_size
            );
        }
        // Below 1 KiB even a login request wouldn't fit; treat as a typo.
        if self.limits.max_body_bytes < 1024 {
            anyhow::bail!("limits.max_body_bytes must be at least 1024");
        }
        if self.limits.ws_send_queue == 0 {
            anyhow::bail!("limits.ws_send_queue must be at least 1");
        }
        if self.limits.ws_ping_interval_secs == 0 {
            anyhow::bail!("limits.ws_ping_interval_secs must be at least 1");
        }
        if self.limits.ws_idle_timeout_secs <= self.limits.ws_ping_interval_secs {
            anyhow::bail!(
                "limits.ws_idle_timeout_secs ({}) must exceed ws_ping_interval_secs ({}) — \
                 otherwise a healthy client can be dropped before it sees a single ping",
                self.limits.ws_idle_timeout_secs,
                self.limits.ws_ping_interval_secs
            );
        }
        if let Some(driver) = &self.push.driver
            && driver != "log"
        {
            anyhow::bail!(
                "push.driver is deprecated and only accepts \"log\" (got {driver:?}); \
                 enable real delivery with [push.apns] / [push.fcm] sections instead"
            );
        }
        if let Some(apns) = &self.push.apns {
            if apns.environment != "production" && apns.environment != "sandbox" {
                anyhow::bail!(
                    "push.apns.environment must be \"production\" or \"sandbox\", got {:?}",
                    apns.environment
                );
            }
            for (name, value) in [
                ("team_id", &apns.team_id),
                ("key_id", &apns.key_id),
                ("bundle_id", &apns.bundle_id),
            ] {
                if value.is_empty() {
                    anyhow::bail!("push.apns.{name} must not be empty");
                }
            }
            // Fail at startup, not on the first notification: a push key
            // problem discovered days later would silently drop messages.
            apns.read_signing_key()?;
        }
        if let Some(fcm) = &self.push.fcm {
            fcm.read_service_account()?;
        }
        // Calls. Validated whether or not they are enabled: a section that
        // is wrong is wrong before somebody flips the switch.
        if self.calls.ring_timeout_secs < 5 {
            anyhow::bail!(
                "calls.ring_timeout_secs must be at least 5 — a phone cannot be picked up faster"
            );
        }
        if self.calls.turn_credential_ttl_secs < 60 {
            anyhow::bail!("calls.turn_credential_ttl_secs must be at least 60");
        }
        for url in &self.calls.stun_urls {
            if !(url.starts_with("stun:") || url.starts_with("stuns:")) {
                anyhow::bail!("calls.stun_urls entry {url:?} must start with stun: or stuns:");
            }
        }
        for url in &self.calls.turn_urls {
            if !(url.starts_with("turn:") || url.starts_with("turns:")) {
                anyhow::bail!("calls.turn_urls entry {url:?} must start with turn: or turns:");
            }
        }
        if !self.calls.turn_urls.is_empty() && !self.calls.has_turn_credentials() {
            anyhow::bail!(
                "calls.turn_urls is set but no credentials are: set calls.turn_secret (coturn's \
                 static-auth-secret) or both calls.turn_username and calls.turn_password"
            );
        }
        Ok(())
    }
}

fn default_bind() -> String {
    "127.0.0.1:8080".to_string()
}

fn default_db_host() -> String {
    "127.0.0.1".to_string()
}

fn default_db_port() -> u16 {
    5432
}

fn default_db_user() -> String {
    "family_connect".to_string()
}

fn default_db_database() -> String {
    "family_connect".to_string()
}

fn default_db_max_connections() -> u32 {
    10
}

fn default_db_connect_timeout_secs() -> u64 {
    10
}

fn default_session_ttl_days() -> i64 {
    180
}

fn default_session_touch_interval_mins() -> i64 {
    15
}

fn default_max_message_chars() -> usize {
    4000
}

/// protocol.md's Limits table: 2 minimum, 10 maximum.
fn default_max_poll_options() -> usize {
    10
}

fn default_max_poll_option_chars() -> usize {
    100
}

fn default_max_attachment_bytes() -> usize {
    100 * 1024 * 1024
}

/// protocol.md's Limits table: 10 per message, the fewest is 1 (fixed).
fn default_max_attachments_per_message() -> usize {
    10
}

fn default_max_preview_bytes() -> usize {
    2 * 1024 * 1024
}

fn default_attachment_grace_hours() -> i64 {
    24
}

fn default_retention_days() -> i64 {
    100
}

fn default_max_board_notes() -> i64 {
    500
}

fn default_max_family_members() -> i64 {
    50
}

fn default_default_page_size() -> i64 {
    50
}

fn default_max_page_size() -> i64 {
    200
}

fn default_max_body_bytes() -> usize {
    16384
}

/// 256 KiB — comfortably above the ~512px square JPEG both clients
/// upload, and far below anything worth storing in a family database.
/// 2 GiB. Room for PostgreSQL to keep working — WAL, autovacuum, a base
/// backup — after uploads have stopped. An operator on a large volume can
/// raise it; 0 turns the check off.
fn default_min_free_disk_bytes() -> u64 {
    2 * 1024 * 1024 * 1024
}

/// Eight concurrent argon2 hashes is ~152 MiB at 19 MiB each — a bound a
/// small VPS survives, and still enough concurrency that a family of
/// twenty all signing in at once notices nothing.
fn default_max_password_hashes_in_flight() -> usize {
    8
}

fn default_max_avatar_bytes() -> usize {
    262_144
}

fn default_ws_send_queue() -> usize {
    64
}

fn default_ws_ping_interval_secs() -> u64 {
    30
}

fn default_ws_idle_timeout_secs() -> u64 {
    75
}

fn default_include_message_body() -> bool {
    true
}

fn default_calls_enabled() -> bool {
    true
}

fn default_video_calls_enabled() -> bool {
    true
}

fn default_stun_urls() -> Vec<String> {
    vec!["stun:stun.l.google.com:19302".to_string()]
}

/// A day: long enough that a credential fetched at the start of a call
/// outlives any call, short enough that a leaked one is not a relay forever.
fn default_turn_credential_ttl_secs() -> u64 {
    86_400
}

/// protocol.md's Limits table: 45 s.
fn default_ring_timeout_secs() -> u64 {
    45
}

fn default_apns_environment() -> String {
    "production".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn the_checked_in_example_config_parses_and_validates() {
        let cfg = Config::from_toml_str(include_str!("../config.example.toml"))
            .expect("config.example.toml must always parse and validate");
        // The example documents the defaults — hold it to that.
        let defaults = Config::default();
        assert_eq!(cfg.server.bind, defaults.server.bind);
        assert_eq!(
            cfg.limits.max_message_chars,
            defaults.limits.max_message_chars
        );
        assert_eq!(cfg.auth.session_ttl_days, defaults.auth.session_ttl_days);
        assert_eq!(
            cfg.push.include_message_body,
            defaults.push.include_message_body
        );
        assert!(
            cfg.push.apns.is_none() && cfg.push.fcm.is_none(),
            "the example must ship with real transports commented out"
        );
        assert!(
            !cfg.ai.enabled,
            "the example must ship with the assistant off"
        );
    }

    /// The example's commented-out `[ai]` block, uncommented.
    ///
    /// The test above only proves the file parses WITH that block commented
    /// out — which is exactly the state in which a stale example survives
    /// unnoticed. This one does what a reader would do, and holds the
    /// documented keys to the real deserializer, so renaming a field without
    /// touching the example fails here rather than in somebody's server.
    #[test]
    fn the_examples_assistant_section_is_valid_when_uncommented() {
        let example = include_str!("../config.example.toml");
        let mut out = String::new();
        let mut in_ai = false;
        for line in example.lines() {
            if line.trim() == "# [ai]" {
                in_ai = true;
                out.push_str("[ai]\n");
                continue;
            }
            if in_ai {
                if let Some(rest) = line.strip_prefix("# ") {
                    out.push_str(rest);
                    out.push('\n');
                    continue;
                }
                if line.trim() == "#" {
                    out.push('\n');
                    continue;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        assert!(in_ai, "the example must document an [ai] section");

        let cfg = Config::from_toml_str(&out)
            .expect("the example's [ai] section must parse once uncommented");
        assert!(cfg.ai.enabled);
        assert!(
            cfg.ai.is_usable(),
            "the documented keys must be enough to actually call"
        );
        // The URL is built from the DEPLOYMENT, which is the mistake the
        // example exists to prevent.
        assert!(
            cfg.ai.completions_url().contains("YOUR-DEPLOYMENT-NAME"),
            "the deployment, not the model, names the endpoint"
        );
        assert!(!cfg.ai.system_prompt.trim().is_empty());
        assert_eq!(cfg.ai.max_tokens, Config::default().ai.max_tokens);
    }

    #[test]
    fn an_empty_config_uses_all_defaults_and_validates() {
        let cfg = Config::from_toml_str("").expect("empty config is valid");
        assert_eq!(cfg.server.bind, "127.0.0.1:8080");
        assert_eq!(cfg.database.port, 5432);
        assert_eq!(cfg.database.max_connections, 10);
        assert_eq!(cfg.auth.session_ttl_days, 180);
        assert_eq!(cfg.auth.session_touch_interval_mins, 15);
        assert_eq!(cfg.limits.max_message_chars, 4000);
        assert_eq!(cfg.limits.default_page_size, 50);
        assert_eq!(cfg.limits.max_page_size, 200);
        assert_eq!(cfg.limits.max_body_bytes, 16384);
        assert_eq!(cfg.limits.ws_send_queue, 64);
        assert_eq!(cfg.limits.ws_ping_interval_secs, 30);
        assert_eq!(cfg.limits.ws_idle_timeout_secs, 75);
        assert!(cfg.push.include_message_body);
        assert!(cfg.push.apns.is_none());
        assert!(cfg.push.fcm.is_none());
    }

    #[test]
    fn load_reads_and_validates_a_file_on_disk() {
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(b"[server]\nbind = \"0.0.0.0:9999\"\n")
            .expect("write");
        f.flush().expect("flush");
        let cfg = Config::load(f.path()).expect("load");
        assert_eq!(cfg.server.bind, "0.0.0.0:9999");
    }

    /// The whole `[ai]` shape as an operator would actually write it, read
    /// off disk rather than built in Rust — because the two sub-sections use
    /// `#[serde(flatten)]` and a flattened struct that TOML cannot fill is a
    /// config file that silently loses its deployment name.
    #[test]
    fn the_three_deployments_load_from_one_ai_section() {
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(
            br#"
[ai]
enabled = true
endpoint = "https://nettrash.openai.azure.com"
deployment = "nettrash-gpt-oss-120b"
model = "gpt-oss-120b"
api_key = "secret"
api_version = "2024-10-21"

[ai.vision]
deployment = "nettrash-gpt-4o"
model = "gpt-4o"

[ai.images]
deployment = "nettrash-FLUX.2-pro"
model = "FLUX.2-pro"
size = "1024x1024"
"#,
        )
        .expect("write");
        f.flush().expect("flush");
        let cfg = Config::load(f.path()).expect("load");

        assert!(cfg.ai.is_usable());
        assert!(cfg.ai.vision_usable(), "a named vision deployment is on");
        assert!(cfg.ai.images_usable(), "a named images deployment is on");
        assert_eq!(cfg.ai.images.size, "1024x1024");
        assert_eq!(
            cfg.ai.images.response_format, "",
            "omitted unless the operator asked for one: some deployments \
             answer 400 to a field they do not implement"
        );

        // Each route goes to its OWN deployment, and inherits the host, the
        // key and the api-version from [ai] — which is what makes the two
        // sub-sections three lines rather than fifteen.
        let text = cfg.ai.text_route();
        let vision = cfg.ai.vision_route().expect("vision route");
        let images = cfg.ai.images_route().expect("images route");
        assert_eq!(
            text.url,
            "https://nettrash.openai.azure.com/openai/deployments/nettrash-gpt-oss-120b\
             /chat/completions?api-version=2024-10-21"
        );
        assert_eq!(
            vision.url,
            "https://nettrash.openai.azure.com/openai/deployments/nettrash-gpt-4o\
             /chat/completions?api-version=2024-10-21"
        );
        assert_eq!(
            images.url,
            "https://nettrash.openai.azure.com/openai/deployments/nettrash-FLUX.2-pro\
             /images/generations?api-version=2024-10-21"
        );
        assert_eq!(
            vision.api_key, "secret",
            "the key is inherited, not retyped"
        );
        assert_eq!(images.api_key, "secret");
        assert_eq!(vision.model, "nettrash-gpt-4o", "the DEPLOYMENT routes");
        assert_eq!(images.model, "nettrash-FLUX.2-pro");
    }

    /// The default, and the one that matters most: a server that configured
    /// only the text deployment has no eyes and no hands, and says so.
    #[test]
    fn pictures_are_off_until_a_deployment_is_named() {
        let cfg = AiConfig {
            enabled: true,
            endpoint: "https://example.openai.azure.com".to_string(),
            deployment: "text".to_string(),
            api_key: "k".to_string(),
            ..Default::default()
        };
        assert!(cfg.is_usable(), "the assistant itself is on");
        assert!(!cfg.vision_usable());
        assert!(!cfg.images_usable());
        assert!(cfg.vision_route().is_none());
        assert!(cfg.images_route().is_none());
    }

    /// And neither half can outlive the assistant: turning `[ai]` off turns
    /// both of them off, whatever the sub-sections say.
    #[test]
    fn pictures_follow_the_master_switch() {
        let mut cfg = AiConfig {
            enabled: true,
            endpoint: "https://example.openai.azure.com".to_string(),
            deployment: "text".to_string(),
            api_key: "k".to_string(),
            ..Default::default()
        };
        cfg.vision.deployment = "sees".to_string();
        cfg.images.deployment.deployment = "draws".to_string();
        assert!(cfg.vision_usable() && cfg.images_usable());
        cfg.enabled = false;
        assert!(!cfg.vision_usable());
        assert!(!cfg.images_usable());
    }

    /// A sub-section may live somewhere else entirely — an image model on a
    /// Foundry resource, a chat model on an Azure OpenAI one — and a pasted
    /// target URI is used verbatim there exactly as it is for `[ai]`.
    #[test]
    fn a_sub_section_may_override_the_endpoint_and_the_key() {
        let mut cfg = AiConfig {
            enabled: true,
            endpoint: "https://text.openai.azure.com".to_string(),
            deployment: "text".to_string(),
            api_key: "text-key".to_string(),
            api_version: "2024-10-21".to_string(),
            ..Default::default()
        };
        cfg.images.deployment.endpoint =
            "https://pictures.services.ai.azure.com/openai/deployments/flux/images/generations\
             ?api-version=2025-04-01-preview"
                .to_string();
        cfg.images.deployment.api_key = "picture-key".to_string();

        let images = cfg
            .images_route()
            .expect("configured by its endpoint alone");
        assert_eq!(
            images.url,
            "https://pictures.services.ai.azure.com/openai/deployments/flux/images/generations\
             ?api-version=2025-04-01-preview",
            "a pasted target URI is used as given, query and all"
        );
        assert_eq!(images.api_key, "picture-key");
        // And the text route is untouched by any of it.
        assert_eq!(cfg.text_route().api_key, "text-key");
    }

    /// The v1 (OpenAI-compatible) surface, for images as well as for chat:
    /// no `/openai/deployments/…` in the path and no api-version query. The
    /// same trap, one path segment along.
    #[test]
    fn the_v1_surface_shapes_the_images_url_too() {
        let mut cfg = AiConfig {
            enabled: true,
            endpoint: "https://nettrash.openai.azure.com/openai/v1".to_string(),
            deployment: "nettrash-gpt-oss-120b".to_string(),
            api_key: "k".to_string(),
            ..Default::default()
        };
        cfg.images.deployment.deployment = "nettrash-FLUX.2-pro".to_string();
        let images = cfg.images_route().expect("images route");
        assert_eq!(
            images.url,
            "https://nettrash.openai.azure.com/openai/v1/images/generations"
        );
        assert_eq!(
            images.model, "nettrash-FLUX.2-pro",
            "the deployment routes from the BODY on this surface"
        );
    }

    #[test]
    fn load_reports_a_missing_file_with_its_path() {
        let err = Config::load(Path::new("/nonexistent/family-connect/cfg.toml")).unwrap_err();
        assert!(format!("{err:#}").contains("reading config file"));
    }

    #[test]
    fn validate_rejects_a_family_ceiling_below_one() {
        // A ceiling of 0 boots happily and then answers `family_full` to
        // every attempt to CREATE a family — an error that endpoint does
        // not document, on a server nobody can use.
        let mut cfg = Config::default();
        cfg.limits.max_family_members = 0;
        assert!(
            cfg.validate().is_err(),
            "a ceiling of zero must be refused at boot"
        );
        cfg.limits.max_family_members = 1;
        assert!(cfg.validate().is_ok(), "one is a legal, if lonely, ceiling");
    }

    #[test]
    fn validate_rejects_an_unparseable_bind_address() {
        let err = Config::from_toml_str("[server]\nbind = \"not-an-addr\"\n").unwrap_err();
        assert!(format!("{err:#}").contains("server.bind"));
    }

    #[test]
    fn validate_rejects_an_idle_timeout_not_exceeding_the_ping_interval() {
        let err = Config::from_toml_str(
            "[limits]\nws_ping_interval_secs = 30\nws_idle_timeout_secs = 30\n",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("ws_idle_timeout_secs"));
    }

    #[test]
    fn validate_rejects_a_max_page_size_below_the_default_page_size() {
        let err = Config::from_toml_str("[limits]\ndefault_page_size = 50\nmax_page_size = 10\n")
            .unwrap_err();
        assert!(format!("{err:#}").contains("max_page_size"));
    }

    #[test]
    fn the_deprecated_log_driver_is_still_accepted_but_others_are_not() {
        let cfg = Config::from_toml_str("[push]\ndriver = \"log\"\n")
            .expect("existing configs with driver = \"log\" must keep loading");
        assert!(cfg.push.apns.is_none() && cfg.push.fcm.is_none());
        let err = Config::from_toml_str("[push]\ndriver = \"apns\"\n").unwrap_err();
        let rendered = format!("{err:#}");
        assert!(rendered.contains("push.driver"), "{rendered}");
        assert!(
            rendered.contains("[push.apns]"),
            "the error must point at the replacement sections: {rendered}"
        );
    }

    /// TOML for a `[push.apns]` section pointing at `key_file`.
    fn apns_toml(key_file: &Path, environment: &str) -> String {
        format!(
            "[push.apns]\nteam_id = \"TEAMID9999\"\nkey_id = \"KEYID12345\"\n\
             key_file = \"{}\"\nbundle_id = \"me.nettrash.familyconnect\"\n\
             environment = \"{environment}\"\n",
            key_file.display()
        )
    }

    #[test]
    fn a_missing_apns_key_file_is_rejected_at_validation_time() {
        let toml = apns_toml(Path::new("/nonexistent/AuthKey_X.p8"), "production");
        let err = Config::from_toml_str(&toml).unwrap_err();
        assert!(format!("{err:#}").contains("push.apns.key_file"));
    }

    #[test]
    fn an_apns_key_file_that_is_not_an_ec_key_is_rejected() {
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(b"this is not a key").expect("write");
        f.flush().expect("flush");
        let err = Config::from_toml_str(&apns_toml(f.path(), "production")).unwrap_err();
        assert!(format!("{err:#}").contains("P-256"));
    }

    #[test]
    fn a_valid_apns_section_validates_and_derives_endpoints_per_environment() {
        let (private_pem, _) = crate::test_keys::ec_p256_key_pair_pems();
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(private_pem.as_bytes()).expect("write");
        f.flush().expect("flush");

        let cfg = Config::from_toml_str(&apns_toml(f.path(), "sandbox")).expect("valid config");
        let apns = cfg.push.apns.expect("apns section parsed");
        assert_eq!(apns.endpoint(), "https://api.sandbox.push.apple.com");

        let cfg = Config::from_toml_str(&apns_toml(f.path(), "production")).expect("valid config");
        let apns = cfg.push.apns.expect("apns section parsed");
        assert_eq!(apns.endpoint(), "https://api.push.apple.com");

        let err = Config::from_toml_str(&apns_toml(f.path(), "staging")).unwrap_err();
        assert!(format!("{err:#}").contains("push.apns.environment"));
    }

    #[test]
    fn a_service_account_missing_required_fields_is_rejected() {
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(br#"{"project_id": "p"}"#).expect("write");
        f.flush().expect("flush");
        let toml = format!(
            "[push.fcm]\ncredentials_file = \"{}\"\n",
            f.path().display()
        );
        let err = Config::from_toml_str(&toml).unwrap_err();
        assert!(
            format!("{err:#}").contains("client_email"),
            "the missing field must be named: {err:#}"
        );
    }

    #[test]
    fn a_valid_fcm_section_validates_and_defaults_the_endpoint() {
        let json = crate::test_keys::service_account_json(
            "test-project",
            "push@test-project.iam.gserviceaccount.com",
            "https://oauth2.googleapis.com/token",
        );
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(json.as_bytes()).expect("write");
        f.flush().expect("flush");
        let toml = format!(
            "[push.fcm]\ncredentials_file = \"{}\"\n",
            f.path().display()
        );
        let cfg = Config::from_toml_str(&toml).expect("valid config");
        let fcm = cfg.push.fcm.expect("fcm section parsed");
        assert_eq!(fcm.endpoint(), "https://fcm.googleapis.com");
        let account = fcm.read_service_account().expect("account parses");
        assert_eq!(account.project_id, "test-project");
    }

    #[test]
    fn an_empty_calls_section_is_on_with_a_public_stun_list() {
        let cfg = Config::from_toml_str("").expect("valid");
        assert!(cfg.calls.enabled);
        assert!(cfg.calls.video_enabled, "video is on wherever calls are");
        assert_eq!(cfg.calls.stun_urls, vec!["stun:stun.l.google.com:19302"]);
        assert!(cfg.calls.turn_urls.is_empty());
        assert_eq!(cfg.calls.ring_timeout_secs, 45);
        assert_eq!(cfg.calls.turn_credential_ttl_secs, 86_400);
        assert!(!cfg.calls.has_turn_credentials());
    }

    #[test]
    fn a_turn_section_validates_with_either_kind_of_credential() {
        let secret =
            "[calls]\nturn_urls = [\"turn:turn.example.com:3478\"]\nturn_secret = \"s3cret\"\n";
        let cfg = Config::from_toml_str(secret).expect("secret creds validate");
        assert!(cfg.calls.has_turn_credentials());
        let fixed = "[calls]\nturn_urls = [\"turns:turn.example.com:5349\"]\nturn_username = \"family\"\nturn_password = \"pw\"\n";
        let cfg = Config::from_toml_str(fixed).expect("static creds validate");
        assert!(cfg.calls.has_turn_credentials());
    }

    #[test]
    fn validate_rejects_a_bad_calls_section() {
        for body in [
            // A relay with nothing to authenticate as is a relay nobody can use.
            "[calls]\nturn_urls = [\"turn:turn.example.com:3478\"]\n",
            // Half a static credential is no credential.
            "[calls]\nturn_urls = [\"turn:turn.example.com:3478\"]\nturn_username = \"x\"\n",
            "[calls]\nstun_urls = [\"turn:turn.example.com:3478\"]\n",
            "[calls]\nturn_urls = [\"stun:stun.example.com:3478\"]\nturn_secret = \"s\"\n",
            "[calls]\nstun_urls = [\"stun.l.google.com:19302\"]\n",
            "[calls]\nring_timeout_secs = 4\n",
            "[calls]\nturn_credential_ttl_secs = 59\n",
        ] {
            assert!(
                Config::from_toml_str(body).is_err(),
                "expected rejection for {body:?}"
            );
        }
        // Disabled does not exempt the section from being right.
        assert!(
            Config::from_toml_str("[calls]\nenabled = false\nring_timeout_secs = 1\n").is_err()
        );
    }

    #[test]
    fn validate_rejects_zero_valued_knobs() {
        for body in [
            "[database]\nmax_connections = 0\n",
            "[auth]\nsession_ttl_days = 0\n",
            "[limits]\nws_send_queue = 0\n",
            "[limits]\nmax_body_bytes = 100\n",
            // A poll needs two options to be a poll: the floor here is not
            // 1, unlike every other knob on this list.
            "[limits]\nmax_poll_options = 1\n",
            "[limits]\nmax_poll_option_chars = 0\n",
        ] {
            assert!(
                Config::from_toml_str(body).is_err(),
                "expected rejection for {body:?}"
            );
        }
    }
}
