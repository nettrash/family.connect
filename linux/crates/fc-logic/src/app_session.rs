//! The account, from the app's side: who is signed in, which screen that means, and the ways a
//! signed-in state ends (`AppSession.cs`).
//!
//! A WRONG PASSWORD IS NOT AN EXPIRED SESSION. Both are `401`, and one of this app's own clients
//! read every one of them as "your session expired" — so a mistyped password told the user they had
//! been signed out of a session they were never in. The code decides: `invalid_credentials` is an
//! answer to the form in front of them, and `session_expired` is the session going away underneath
//! them.
//!
//! THE SESSION ENDS ONCE. A dead session is discovered by whatever asks next, and on a reconnect
//! loop that is every few seconds. The guard is the token still being stored: the first discovery
//! clears it, and every later one finds nothing to end.
//!
//! A FLAKY NETWORK IS NOT A SIGN-OUT. A transient failure on `GET /me` leaves the gate exactly where
//! it is and is reported, because the alternative is an app that drops to the sign-in screen in a
//! lift.
//!
//! **How the app holds it.** One `Rc<AppSession>` per signed-in server, over the same
//! `Rc<ApiClient>`, token store and `Rc<Database>` as everything else:
//! `AppSession::new(api, tokens, cache).with_awaiting_join(…).with_current_family(…)` — the two
//! memories are the shell's settings file (a relaunch must remember a wait, and the family last
//! shown). The window binds to [`AppSession::state`] and [`AppSession::on_changed`]; the
//! [`LiveConnection`](crate::live_connection::LiveConnection) starts and stops the socket from the
//! same event. Every method that writes the cache answers a `StoreResult` (lib.rs); the C#'s
//! `Task<ApiError?>` is the `Option<ApiError>` inside it.
//!
//! **Linux, unlike `win/`:** an ending is said even when the cache refuses to wipe. The C#'s
//! `Clear()` cleared the token and then let `WipeAll`'s exception out before it published — leaving
//! the window on a signed-in screen with no token behind it. Here the signed-out state is published
//! and the ending raised first, and the wipe's error is answered after (the next sign-in wipes
//! again before it writes anything).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use fc_core::protocol::{
    ApiClient, ApiError, ApiResult, AssistantDto, AuthResponse, FamilyDto, InvitationDto,
    MeResponse, PaidFamilyLimitsDto, PendingJoinDto, RoundVideoLimits, SubscriptionDto, TokenStore,
    UserDto, error_codes,
};
use fc_core::rt::Event;
use fc_core::store::{Database, StoreResult};

use crate::join_memory::{AwaitingJoin, MemoryAwaitingJoin};

/// Where the last `/me` is kept in the account's cache (`meta`).
const LAST_ME: &str = "session.last_me";

/// Which screen the app is on, and there is exactly one answer at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Gate {
    /// No server yet: first run.
    NoServer,
    /// A server, and nobody signed in.
    SignedOut,
    /// Signed in, in no family and with no request pending: the gate.
    NoFamily,
    /// A join request waiting for an owner to answer it.
    Pending,
    /// In a family — the one shown, when there are several.
    Member,
    /// In a family, and it is theirs — the one shown, when there are several.
    Owner,
}

/// Why the app left a signed-in state. Each one is a different sentence to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionEnd {
    /// They asked to sign out.
    SignedOut,
    /// The session died server-side — a REST `401`, the socket's `4401`, or an upgrade refused
    /// with `401`. Not the same thing as a wrong password, which never gets this far.
    Expired,
    /// The owner removed them, or the family dissolved.
    RemovedFromFamily,
    /// They were waiting for an owner, and the request is no longer pending.
    JoinRequestRejected,
    /// They deleted the account. Not "signed out" and not "expired": there is nothing to sign back
    /// into, and a screen offering to would be cruel as well as wrong.
    AccountDeleted,
}

/// What the window binds to: one snapshot, replaced whole (`SessionState`).
///
/// Built as `SessionState { gate: Gate::Member, assistant: Some(…), ..SessionState::new(Gate::Member) }`
/// where the C# names its optional parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionState {
    pub gate: Gate,
    pub me: Option<UserDto>,
    pub family: Option<FamilyDto>,
    pub assistant: Option<AssistantDto>,
    pub pending_family_name: Option<String>,
    pub calls_enabled: bool,
    pub video_calls_enabled: bool,
    pub max_family_members: i32,
    pub support_contact: Option<String>,
    pub family_registration_enabled: bool,
    pub familyless_account_ttl_days: i32,
    pub greetings_enabled: bool,
    /// When this member agreed that their words may go to the model, or `None` until they have
    /// (docs/protocol.md, "Consenting to the assistant"). The SERVER's answer and not this
    /// computer's: the server is what calls the model, a reinstall must not quietly re-ask and
    /// re-send, and somebody who agreed on their phone has agreed.
    pub assistant_consent_at: Option<String>,
    pub join_declined: bool,
    /// When this member agreed that the assistant may send a query or place name it writes from
    /// their words to the lookup providers, or `None` until they have (docs/protocol.md,
    /// "Consenting to the assistant", amended 2026-10-03). The server's answer, like
    /// [`assistant_consent_at`](Self::assistant_consent_at), and never assumed: `None` is what an
    /// older server's silence reads as.
    pub assistant_lookup_consent_at: Option<String>,
    /// The video message's limits from `GET /families/mine`, or `None` on a server that predates
    /// them — which offers no way of recording one (docs/audio-video-messages-2026-10-04.md, S1.2's
    /// "round available").
    pub round_video: Option<RoundVideoLimits>,
    /// EVERY family the account is in, home family first, each with this account's role in it
    /// (docs/protocol.md, "Several families"). [`family`](Self::family) is the one of them being
    /// SHOWN — everything family-level on screen is its.
    pub families: Option<Vec<FamilyDto>>,
    /// The family being shown, by id; `None` while there is none.
    pub current_family_id: Option<i64>,
    /// Every join request still waiting — while in a family too, to a further one.
    pub pending_joins: Option<Vec<PendingJoinDto>>,
    /// This account's own pending invitations into other families.
    pub invitations: Option<Vec<InvitationDto>>,
    /// The operator's switch for members starting another family in the app.
    pub member_families_enabled: bool,
    /// How many families one account may be in — never below one.
    pub max_families_per_account: i32,
    /// Whether this server knows several families at all — its `GET /me` carried `families`. An
    /// older one knows only `/families/mine`, and is never asked for a family by id.
    pub names_families: bool,
    /// Whether this server charges for a new family (docs/protocol.md, "Paid families"): with it
    /// false — and on every server from before — nothing about paying applies.
    pub subscriptions_enabled: bool,
    /// The account's own App Store subscription as the server judged it, or `None`.
    pub subscription: Option<SubscriptionDto>,
    /// What a paid family gets on this server.
    pub paid_family_limits: Option<PaidFamilyLimitsDto>,
    /// How long an unpaid family is kept.
    pub unpaid_days: Option<i32>,
}

impl SessionState {
    /// `new SessionState(gate)`: everything else at the C#'s defaults.
    pub fn new(gate: Gate) -> Self {
        SessionState {
            gate,
            me: None,
            family: None,
            assistant: None,
            pending_family_name: None,
            calls_enabled: false,
            video_calls_enabled: false,
            max_family_members: 0,
            support_contact: None,
            family_registration_enabled: true,
            familyless_account_ttl_days: 0,
            greetings_enabled: false,
            assistant_consent_at: None,
            join_declined: false,
            assistant_lookup_consent_at: None,
            round_video: None,
            families: None,
            current_family_id: None,
            pending_joins: None,
            invitations: None,
            member_families_enabled: false,
            max_families_per_account: 1,
            names_families: false,
            subscriptions_enabled: false,
            subscription: None,
            paid_family_limits: None,
            unpaid_days: None,
        }
    }

    /// Whether the socket may connect at all: signed in, and in a family.
    pub fn can_chat(&self) -> bool {
        matches!(self.gate, Gate::Member | Gate::Owner)
    }

    /// Whether this account owns the family being shown.
    pub fn is_owner(&self) -> bool {
        self.gate == Gate::Owner
    }

    /// The families, never `None`.
    pub fn all_families(&self) -> &[FamilyDto] {
        self.families.as_deref().unwrap_or_default()
    }

    /// The HOME family — the first `GET /me` lists (docs/protocol.md, "Several families"), and the
    /// one family of a server that predates them — where a chat with no `family_ids` is listed
    /// (`family_rules::families_of`); `None` with no family.
    pub fn home_family_id(&self) -> Option<i64> {
        self.all_families().first().map(|family| family.id)
    }

    /// The waiting join requests, never `None`.
    pub fn all_pending(&self) -> &[PendingJoinDto] {
        self.pending_joins.as_deref().unwrap_or_default()
    }

    /// The pending invitations to this account, never `None`.
    pub fn all_invitations(&self) -> &[InvitationDto] {
        self.invitations.as_deref().unwrap_or_default()
    }

    /// Whether this account holds an ACTIVE subscription, as the server judged it.
    pub fn subscription_active(&self) -> bool {
        self.subscription.as_ref().is_some_and(|held| held.active)
    }

    /// The top of the owner's member-cap picker: the family's own `member_ceiling`
    /// (docs/protocol.md, "Paid families" — a paid family's is lower), else the operator's
    /// `max_family_members` from `GET /me`, which is every server that predates it. 0 when neither
    /// says, and then there is no picker.
    pub fn member_ceiling(&self) -> i32 {
        self.family
            .as_ref()
            .and_then(|family| family.member_ceiling)
            .filter(|ceiling| *ceiling > 0)
            .unwrap_or(self.max_family_members)
    }
}

impl Default for SessionState {
    /// Signed out — what a session is until `GET /me` says otherwise.
    fn default() -> Self {
        SessionState::new(Gate::SignedOut)
    }
}

/// Which family this device last SHOWED, per account — so a relaunch opens where the reader left
/// off (docs/protocol.md, "Several families": "a CURRENT family, persisted per account on the
/// device"). `ICurrentFamily`; the shell keeps it in its settings file.
pub trait CurrentFamily {
    /// `For(userId)`.
    fn family_for(&self, user_id: i64) -> Option<i64>;

    fn remember(&self, user_id: i64, family_id: i64);
}

/// For a session that need not outlive its process — the tests (`MemoryCurrentFamily`).
#[derive(Debug, Default)]
pub struct MemoryCurrentFamily(RefCell<HashMap<i64, i64>>);

impl MemoryCurrentFamily {
    pub fn new() -> Self {
        MemoryCurrentFamily::default()
    }
}

impl CurrentFamily for MemoryCurrentFamily {
    fn family_for(&self, user_id: i64) -> Option<i64> {
        self.0.borrow().get(&user_id).copied()
    }

    fn remember(&self, user_id: i64, family_id: i64) {
        self.0.borrow_mut().insert(user_id, family_id);
    }
}

/// The account, from the app's side: who is signed in, which screen that means, and the ways a
/// signed-in state ends (`AppSession`). See the module's comment.
pub struct AppSession {
    api: Rc<ApiClient>,
    tokens: Rc<dyn TokenStore>,
    cache: Rc<Database>,
    awaiting: Rc<dyn AwaitingJoin>,
    remembered: Rc<dyn CurrentFamily>,
    /// Whose cache has been told its home family ([`Database::adopt_legacy`]) — once per account
    /// per launch.
    adopted_for: Cell<i64>,
    // Signed out until `GET /me` says otherwise: a token in the locker is not a session, and a
    // window drawn from a stored token alone would show a chat list to somebody whose account was
    // deleted three weeks ago.
    state: RefCell<SessionState>,
    family_dropped: Event<i64>,
    changed: Event<SessionState>,
    ended: Event<SessionEnd>,
}

impl std::fmt::Debug for AppSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The gate and ids only: no names, no token (the family's logging rule).
        let state = self.state.borrow();
        f.debug_struct("AppSession")
            .field("gate", &state.gate)
            .field("current_family_id", &state.current_family_id)
            .finish_non_exhaustive()
    }
}

impl AppSession {
    /// `new AppSession(api, tokens, cache)`: the wait and the family last shown remembered for this
    /// process only, until [`with_awaiting_join`](Self::with_awaiting_join) and
    /// [`with_current_family`](Self::with_current_family) give the shell's own.
    pub fn new(api: Rc<ApiClient>, tokens: Rc<dyn TokenStore>, cache: Rc<Database>) -> Self {
        AppSession {
            api,
            tokens,
            cache,
            awaiting: Rc::new(MemoryAwaitingJoin::new()),
            remembered: Rc::new(MemoryCurrentFamily::new()),
            adopted_for: Cell::new(0),
            state: RefCell::new(SessionState::default()),
            family_dropped: Event::new(),
            changed: Event::new(),
            ended: Event::new(),
        }
    }

    /// Whose join request this device was waiting on, kept across launches (`IAwaitingJoin`).
    pub fn with_awaiting_join(mut self, awaiting: Rc<dyn AwaitingJoin>) -> Self {
        self.awaiting = awaiting;
        self
    }

    /// Which family this device last showed, per account (`ICurrentFamily`).
    pub fn with_current_family(mut self, remembered: Rc<dyn CurrentFamily>) -> Self {
        self.remembered = remembered;
        self
    }

    /// The current snapshot. Replaced whole, never edited in place — this is a copy of it.
    pub fn state(&self) -> SessionState {
        self.state.borrow().clone()
    }

    /// The snapshot changed (`Changed`).
    pub fn on_changed(&self) -> &Event<SessionState> {
        &self.changed
    }

    /// A signed-in state ended, and why. Raised once per ending (`Ended`).
    pub fn on_ended(&self) -> &Event<SessionEnd> {
        &self.ended
    }

    /// A family appeared or went outside a list this session asked for — the router's word, or a
    /// `family_not_found` (`FamilyDropped`). The window drops what it holds of a family gone with
    /// its rows.
    pub fn on_family_dropped(&self) -> &Event<i64> {
        &self.family_dropped
    }

    /// Sign in. The answer is the refusal, or `None` when it worked.
    pub async fn sign_in(&self, username: &str, password: &str) -> StoreResult<Option<ApiError>> {
        let answer = self.api.log_in(username, password).await;
        self.authenticate(answer).await
    }

    /// Register, which signs in as the same act — the answer carries the token.
    pub async fn register(
        &self,
        username: &str,
        display_name: &str,
        password: &str,
    ) -> StoreResult<Option<ApiError>> {
        let answer = self.api.register(username, display_name, password).await;
        self.authenticate(answer).await
    }

    async fn authenticate(&self, answer: ApiResult<AuthResponse>) -> StoreResult<Option<ApiError>> {
        let answer = match answer.into_result() {
            Ok(answer) => answer,
            // No wiping, no ending: nobody was signed in, and the user is looking at the form this
            // answers.
            Err(error) => return Ok(Some(error)),
        };
        // A NEW ACCOUNT MAY NOT INHERIT THE LAST ONE'S CACHE. Signing in as somebody else on a
        // device that was signed in before must not leave that family's messages on screen — and
        // the outbox least of all (see `Database::wipe_all`).
        self.cache.wipe_all()?;
        self.tokens.set_token(Some(answer.token));
        self.refresh().await
    }

    /// Read `GET /me` and put the app on the screen it says. The one call that decides the gate;
    /// everything else here is a consequence of it.
    pub async fn refresh(&self) -> StoreResult<Option<ApiError>> {
        self.refresh_preferring(None).await
    }

    /// [`refresh`](Self::refresh), SHOWING `prefer` when the answer has it — a family just created,
    /// joined or accepted (docs/protocol.md, "Several families": "add it, switch to it").
    pub async fn refresh_preferring(&self, prefer: Option<i64>) -> StoreResult<Option<ApiError>> {
        if self.tokens.token().is_none() {
            self.publish(SessionState::new(Gate::SignedOut));
            return Ok(None);
        }
        match self.api.me().await.into_result() {
            Ok(me) => {
                self.apply(&me, prefer)?;
                self.remember(&me)?;
                Ok(None)
            }
            Err(error) => {
                // The rule lives on the error: a 401 that is not `invalid_credentials`. A wrong
                // password cannot reach here anyway — this call carries a token, not a form — but
                // asking the same question in both places keeps one answer.
                if error.session_gone() {
                    self.session_gone()?;
                } else {
                    // Anything else — a dead network, a proxy, a 500 — changes nothing about who
                    // is signed in; a launch that has nothing open yet opens what it holds.
                    self.open_offline(prefer)?;
                }
                Ok(Some(error))
            }
        }
    }

    /// The last `/me` this account was answered, kept in its cache (wiped with everything else at a
    /// sign-out or when the session is gone): what an offline launch opens.
    fn remember(&self, me: &MeResponse) -> StoreResult<()> {
        match serde_json::to_string(me) {
            Ok(json) => self.cache.set_meta_text(LAST_ME, &json),
            Err(_) => Ok(()),
        }
    }

    /// A launch that cannot reach the server, with a token and chats in the cache: the chats open
    /// from the last `/me` this account was answered, as it was then (the Mac's `openOffline`) — the
    /// socket keeps trying, and the resync's own `/me` replaces it all once it is answered. Only
    /// while nothing is open yet: a session already on screen stays exactly where it is. Whether it
    /// opened.
    fn open_offline(&self, prefer: Option<i64>) -> StoreResult<bool> {
        if self.state.borrow().gate != Gate::SignedOut || !self.cache.holds_chats()? {
            return Ok(false);
        }
        let Some(me) = self
            .cache
            .meta_text(LAST_ME)?
            .and_then(|json| serde_json::from_str::<MeResponse>(&json).ok())
        else {
            return Ok(false);
        };
        self.apply(&me, prefer)?;
        Ok(self.state.borrow().gate != Gate::SignedOut)
    }

    /// What a `GET /me` answered, applied: which families there are, which one is shown, and the
    /// screen that means. The resync's step 1 lands here too, so a family that appeared or went
    /// while the socket was down is caught at the next pass — a removal it slept through raises no
    /// frame it will ever see. `prefer` is a family to SHOW if the answer has it — one just
    /// created, joined or accepted.
    pub fn apply(&self, answered: &MeResponse, prefer: Option<i64>) -> StoreResult<()> {
        if self.tokens.token().is_none() {
            // A sign-out landed while the read was in flight: there is nobody to apply it to.
            return Ok(());
        }
        let state = self.state();
        let was = state.gate;
        let user_id = answered.user.id;
        let same_account = state.me.as_ref().map(|me| me.id) == Some(user_id);
        let families = answered.all_families();
        let pending = answered.all_pending().to_vec();
        let shown = self.shown(&state, user_id, &families, prefer);
        let current = shown.and_then(|id| families.iter().find(|family| family.id == id));
        let gate = match current {
            Some(family)
                if family.is_owner() || (answered.families.is_none() && answered.is_owner()) =>
            {
                Gate::Owner
            }
            Some(_) => Gate::Member,
            None if !pending.is_empty() => Gate::Pending,
            None => Gate::NoFamily,
        };
        if let Some(chosen) = shown {
            self.remembered.remember(user_id, chosen);
            if self.adopted_for.get() != user_id {
                // A cache an older build filled was the HOME family's — the only family such a
                // build ever saw.
                self.cache.adopt_legacy(families[0].id)?;
                self.adopted_for.set(user_id);
            }
        }
        // `/families/{id}` only where the server knows it; an older one is asked for `mine`, which
        // is all it has.
        self.api.set_family_scope(if answered.families.is_some() {
            shown
        } else {
            None
        });
        // Declined is what a remembered wait becomes when `GET /me` shows no request and no family
        // — said on the family door until the next join or family replaces it (ios
        // AppSession.joinDeclined).
        let waited = was == Gate::Pending || self.awaiting.user_id() == Some(user_id);
        let declined =
            gate == Gate::NoFamily && (waited || (was == Gate::NoFamily && state.join_declined));
        self.awaiting.set_user_id(if gate == Gate::Pending {
            Some(user_id)
        } else {
            None
        });
        // A family that is no longer in the list was left or lost while nobody was looking: its
        // rows go, and only its — unless it was the LAST one, which takes everything (below). What
        // this cache holds counts too: a family left while the app was not running is in no list
        // this session has seen. (A cache never holds another account's families — signing in
        // wipes it.)
        let mut gone: Vec<i64> = Vec::new();
        if !families.is_empty() {
            let held: Vec<i64> = if same_account {
                state
                    .all_families()
                    .iter()
                    .map(|family| family.id)
                    .collect()
            } else {
                Vec::new()
            };
            for id in held.into_iter().chain(self.cache.held_families()?) {
                if !gone.contains(&id) && !families.iter().any(|family| family.id == id) {
                    gone.push(id);
                }
            }
        }
        let same_family = state.current_family_id == shown;
        let next = SessionState {
            gate,
            me: Some(answered.user.clone()),
            // The shown family's fuller document (its invite code, its switches) when this read is
            // about the same one.
            family: current.map(|current| match &state.family {
                Some(held) if same_family && held.id == current.id => merge(held, current),
                _ => current.clone(),
            }),
            // CARRIED, NOT RE-READ. `GET /me` says nothing about the assistant — only `GET
            // /families/mine` does — so a refresh that leaves this out publishes a state with no
            // assistant in it. What hung off it: the Assistant card in Settings and the consent
            // line over the composer, both of which vanished after any /me — which includes the
            // refresh that runs right after somebody agrees. Cleared with the family, because an
            // assistant belongs to one.
            assistant: if matches!(gate, Gate::Member | Gate::Owner) {
                state.assistant.clone()
            } else {
                None
            },
            // The same for the video message's limits: only the family's own document carries them.
            round_video: if matches!(gate, Gate::Member | Gate::Owner) {
                state.round_video
            } else {
                None
            },
            pending_family_name: pending.first().map(|join| join.family_name.clone()),
            calls_enabled: answered.calls_enabled,
            video_calls_enabled: answered.video_calls_enabled,
            max_family_members: answered.max_family_members,
            support_contact: answered.support_contact.clone(),
            family_registration_enabled: answered.family_registration_enabled,
            familyless_account_ttl_days: answered.familyless_account_ttl_days,
            greetings_enabled: answered.greetings_enabled,
            assistant_consent_at: answered.assistant_consent_at.clone(),
            join_declined: declined,
            assistant_lookup_consent_at: answered.assistant_lookup_consent_at.clone(),
            current_family_id: shown,
            pending_joins: Some(pending),
            invitations: Some(answered.invitations.clone().unwrap_or_default()),
            member_families_enabled: answered.member_families_enabled,
            max_families_per_account: answered.family_cap(),
            names_families: answered.families.is_some(),
            families: Some(families),
            subscriptions_enabled: answered.subscriptions_enabled,
            subscription: answered.subscription.clone(),
            paid_family_limits: answered.paid_family_limits.clone(),
            unpaid_days: answered.unpaid_days,
        };
        for family in &gone {
            self.cache.drop_family(*family)?;
        }
        // A pass's `GET /me` that changed nothing publishes nothing: every publish redraws every
        // screen bound to this state.
        if gone.is_empty() && next == state {
            return Ok(());
        }
        self.publish(next);
        for family in &gone {
            self.family_dropped.emit(family);
        }

        // Two departures nothing else announces. `GET /me` is where a client finds out, because a
        // removal it slept through raises no frame it will ever see.
        if matches!(was, Gate::Member | Gate::Owner)
            && matches!(gate, Gate::NoFamily | Gate::Pending)
        {
            // Their family's chats and wall are not theirs to keep — and the ending is said even
            // when the cache refuses (the module's comment).
            let wiped = self.cache.wipe_all();
            self.ended.emit(&SessionEnd::RemovedFromFamily);
            wiped?;
        } else if waited && gate == Gate::NoFamily {
            self.ended.emit(&SessionEnd::JoinRequestRejected);
        }
        Ok(())
    }

    /// Which family to show: one just asked for, else the one shown now, else the one this device
    /// last showed this account, else the first — the home family. `None` with no family at all.
    fn shown(
        &self,
        state: &SessionState,
        user_id: i64,
        families: &[FamilyDto],
        prefer: Option<i64>,
    ) -> Option<i64> {
        let holds = |id: Option<i64>| {
            id.is_some_and(|wanted| families.iter().any(|family| family.id == wanted))
        };
        if holds(prefer) {
            return prefer;
        }
        if state.me.as_ref().map(|me| me.id) == Some(user_id) && holds(state.current_family_id) {
            return state.current_family_id;
        }
        let last = self.remembered.family_for(user_id);
        if holds(last) {
            last
        } else {
            families.first().map(|family| family.id)
        }
    }

    /// Show another of this account's families. Everything family-level on screen follows — the
    /// chat list, the board, the stickers, the roster, the owner's screens — and the family's own
    /// document is read for what only it carries (the assistant, the video message's limits, the
    /// invite code).
    pub async fn switch_family(&self, family_id: i64) -> StoreResult<Option<ApiError>> {
        if !self.switch(family_id) {
            return Ok(None);
        }
        self.refresh_family().await
    }

    /// The switch itself, published at once; `false` when that family is not this account's or is
    /// already shown.
    pub fn switch(&self, family_id: i64) -> bool {
        let state = self.state();
        let Some(me) = &state.me else {
            return false;
        };
        if state.current_family_id == Some(family_id) {
            return false;
        }
        let Some(family) = state
            .all_families()
            .iter()
            .find(|family| family.id == family_id)
            .cloned()
        else {
            return false;
        };
        self.remembered.remember(me.id, family_id);
        if state.names_families {
            self.api.set_family_scope(Some(family_id));
        }
        self.publish(SessionState {
            gate: if family.is_owner() {
                Gate::Owner
            } else {
                Gate::Member
            },
            family: Some(family),
            current_family_id: Some(family_id),
            ..state
        });
        true
    }

    /// The router says this account is in a family — re-read `GET /me` when it is one this session
    /// does not hold (docs/protocol.md, "Several families"), so the new family is listed; the
    /// resync that follows reads its rows. Answers whether it re-read.
    pub async fn joined_family(&self, family_id: i64) -> StoreResult<bool> {
        if self
            .state
            .borrow()
            .all_families()
            .iter()
            .any(|family| family.id == family_id)
        {
            return Ok(false);
        }
        self.refresh().await?;
        Ok(true)
    }

    /// This account is out of a family — the router's `member_left` naming it, or
    /// `family_not_found` from a read of it: that family's rows go at once, and `GET /me` says what
    /// is left to show.
    pub async fn family_gone(&self, family_id: i64) -> StoreResult<()> {
        let several = self.state.borrow().all_families().len() > 1;
        if several {
            self.cache.drop_family(family_id)?;
            self.family_dropped.emit(&family_id);
        }
        self.refresh().await?;
        Ok(())
    }

    /// An invitation frame: to this account it is listed (or replaced); one this account's family
    /// sent is not its.
    pub fn apply_invitation(&self, invitation: &InvitationDto) {
        let state = self.state();
        if state.me.as_ref().map(|me| me.id) != Some(invitation.user.id) {
            return;
        }
        let mut invitations: Vec<InvitationDto> = state
            .all_invitations()
            .iter()
            .filter(|held| held.id != invitation.id)
            .cloned()
            .collect();
        invitations.push(invitation.clone());
        self.publish(SessionState {
            invitations: Some(invitations),
            ..state
        });
    }

    /// A `family_plan` frame (docs/protocol.md, "Paid families"): the family it names takes the
    /// frame's plan and deadline — in the list, and on screen when it is the one shown — so the
    /// unpaid banner comes and goes live. An absent `delete_at` CLEARS the one held (the family is
    /// covered again); an absent `plan` leaves the held one alone. A family this account is not in
    /// changes nothing, and a frame that changes nothing publishes nothing.
    pub fn apply_family_plan(&self, family_id: i64, plan: Option<&str>, delete_at: Option<&str>) {
        let state = self.state();
        let planned = |family: &FamilyDto| FamilyDto {
            plan: plan.map(str::to_string).or_else(|| family.plan.clone()),
            delete_at: delete_at.map(str::to_string),
            ..family.clone()
        };
        if !state.all_families().iter().any(|held| held.id == family_id) {
            return;
        }
        let families: Vec<FamilyDto> = state
            .all_families()
            .iter()
            .map(|held| {
                if held.id == family_id {
                    planned(held)
                } else {
                    held.clone()
                }
            })
            .collect();
        let family = state.family.as_ref().map(|shown| {
            if shown.id == family_id {
                planned(shown)
            } else {
                shown.clone()
            }
        });
        let next = SessionState {
            families: Some(families),
            family,
            ..state.clone()
        };
        if next != state {
            self.publish(next);
        }
    }

    /// An invitation lapsed — accepted, declined, withdrawn — and leaves the banner and the
    /// switcher.
    pub fn invitation_gone(&self, invitation_id: i64) {
        let state = self.state();
        if state
            .all_invitations()
            .iter()
            .all(|held| held.id != invitation_id)
        {
            return;
        }
        let invitations = state
            .all_invitations()
            .iter()
            .filter(|held| held.id != invitation_id)
            .cloned()
            .collect();
        self.publish(SessionState {
            invitations: Some(invitations),
            ..state
        });
    }

    /// Join: the family is added and SHOWN. `invitation_not_pending` drops the invitation — it is
    /// gone, whatever the reason — and comes back as the refusal for the window to say;
    /// `family_full` and `too_many_families` leave it standing.
    pub async fn accept_invitation(&self, invitation_id: i64) -> StoreResult<Option<ApiError>> {
        match self
            .api
            .accept_invitation(invitation_id)
            .await
            .into_result()
        {
            Err(error) => {
                if error.code == error_codes::INVITATION_NOT_PENDING {
                    self.invitation_gone(invitation_id);
                }
                Ok(Some(error))
            }
            Ok(accepted) => {
                self.invitation_gone(invitation_id);
                self.refresh_preferring(Some(accepted.family.id)).await
            }
        }
    }

    /// Decline: gone from the list whether the server still had it or not.
    pub async fn decline_invitation(&self, invitation_id: i64) -> Option<ApiError> {
        if let Err(error) = self
            .api
            .decline_invitation(invitation_id)
            .await
            .into_result()
            && error.code != error_codes::INVITATION_NOT_PENDING
        {
            return Some(error);
        }
        self.invitation_gone(invitation_id);
        None
    }

    /// The family's own document, which is where the roster, the assistant and the board's mark
    /// live. Folded into the snapshot so the window has one thing to read.
    pub async fn refresh_family(&self) -> StoreResult<Option<ApiError>> {
        let answer = match self.api.family(None).await.into_result() {
            Ok(answer) => answer,
            Err(error) => {
                if error.code == error_codes::FAMILY_NOT_FOUND {
                    // The family shown is gone — left elsewhere, removed, deleted (docs/protocol.md,
                    // "Several families"): `/me` says what is left, and the gone family's rows go
                    // with it.
                    self.refresh().await?;
                }
                return Ok(Some(error));
            }
        };
        let state = self.state();
        if !state.can_chat()
            || state
                .current_family_id
                .is_some_and(|shown| answer.family.id != shown)
        {
            // The gate moved while this read was in flight — a sign-out, or a removal — or another
            // family is shown now. Publishing would hand the window a signed-out state carrying a
            // family, or one family's switches on another.
            return Ok(None);
        }
        let role = state.family.as_ref().and_then(|family| family.role.clone());
        let mut family = answer.family.clone();
        if family.role.is_none() {
            family.role = role;
        }
        let families = state
            .all_families()
            .iter()
            .map(|held| {
                if held.id == answer.family.id {
                    FamilyDto {
                        role: held.role.clone(),
                        ..answer.family.clone()
                    }
                } else {
                    held.clone()
                }
            })
            .collect();
        self.publish(SessionState {
            family: Some(family),
            families: Some(families),
            assistant: answer.assistant.clone(),
            round_video: RoundVideoLimits::of(&answer),
            ..state
        });
        Ok(None)
    }

    /// The family's assistant, as a pass over the wire found it — the resync reads the family's
    /// own document on every (re)connect, and this is what puts what it found where the window can
    /// see it. Published only while there is a family to have one, and only when it CHANGES,
    /// because every publish redraws every screen bound to this state.
    pub fn apply_assistant(&self, assistant: Option<AssistantDto>) {
        let state = self.state();
        if !state.can_chat() || state.assistant == assistant {
            return;
        }
        self.publish(SessionState { assistant, ..state });
    }

    /// The video message's limits, as a pass over the wire read them from the family's own
    /// document — published only while there is a family, and only when they change.
    pub fn apply_round_video(&self, limits: Option<RoundVideoLimits>) {
        let state = self.state();
        if !state.can_chat() || state.round_video == limits {
            return;
        }
        self.publish(SessionState {
            round_video: limits,
            ..state
        });
    }

    /// Sign out. The server is TOLD — which is what stops this device's notifications — but being
    /// unable to tell it changes nothing here: the token is this device's to forget.
    pub async fn sign_out(&self) -> StoreResult<()> {
        if self.tokens.token().is_none() {
            return Ok(());
        }
        // A sign-out that fails on the network is still a sign-out.
        let _ = self.api.log_out().await;
        self.clear(SessionEnd::SignedOut)
    }

    /// Delete this account. The password is the proof, for the same reason a password CHANGE needs
    /// the current one — and when it is done there is nothing left to sign into.
    pub async fn delete_account(&self, password: &str) -> StoreResult<Option<ApiError>> {
        if let Err(error) = self.api.delete_account(password).await.into_result() {
            // A WRONG PASSWORD HERE IS NOT AN EXPIRED SESSION either: the account is still there,
            // and so is the session.
            return Ok(Some(error));
        }
        self.clear(SessionEnd::AccountDeleted)?;
        Ok(None)
    }

    /// Change the password. The current one is required because A LIVE SESSION IS NOT PROOF — and
    /// a refusal here says "that is not your password", never "you have been signed out".
    pub async fn change_password(
        &self,
        current_password: &str,
        new_password: &str,
    ) -> Option<ApiError> {
        self.api
            .change_password(current_password, new_password)
            .await
            .into_result()
            .err()
    }

    /// The session is gone server-side: a REST `401` with `session_expired`, the socket closing with
    /// `4401`, or its upgrade refused with `401`. Idempotent, because the reconnect loop will find
    /// out again every few seconds and the user needs telling once.
    pub fn session_gone(&self) -> StoreResult<()> {
        if self.tokens.token().is_none() {
            return Ok(());
        }
        self.clear(SessionEnd::Expired)
    }

    /// Everything this device knew of the account, forgotten — and the ending said even when the
    /// cache refuses its wipe (the module's comment), with that refusal answered after.
    fn clear(&self, ending: SessionEnd) -> StoreResult<()> {
        self.tokens.set_token(None);
        // Nobody's family is shown: the next account names its own.
        self.api.set_family_scope(None);
        let wiped = self.cache.wipe_all();
        let support_contact = self.state.borrow().support_contact.clone();
        self.publish(SessionState {
            support_contact,
            ..SessionState::new(Gate::SignedOut)
        });
        self.ended.emit(&ending);
        wiped
    }

    fn publish(&self, next: SessionState) {
        *self.state.borrow_mut() = next.clone();
        self.changed.emit(&next);
    }
}

/// The family's fuller document kept, under what the list now says of it — its name, the reader's
/// role, and its plan, ceiling and deadline.
fn merge(held: &FamilyDto, listed: &FamilyDto) -> FamilyDto {
    FamilyDto {
        name: listed.name.clone(),
        role: listed.role.clone().or_else(|| held.role.clone()),
        // A paid family's standing is the list's too (docs/protocol.md, "Paid families"): a
        // deadline that appeared — or went — while no frame reached this device is learnt here, and
        // its absence clears the one held.
        plan: listed.plan.clone(),
        member_ceiling: listed.member_ceiling,
        delete_at: listed.delete_at.clone(),
        ..held.clone()
    }
}
