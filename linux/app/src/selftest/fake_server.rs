//! The self-test's FAKE SERVER (shell-design §27): enough of docs/protocol.md for the gate and the
//! live socket, on 127.0.0.1, built on libsoup's own `soup::Server` — the same library the client
//! speaks through, already linked, no new crate, no second binary to build. Its bodies are fc-core's
//! own DTOs serialised, so what it says is exactly what the client parses.
//!
//! **Two ways to run it.** In process: `Harness::fake_server()` starts one per self-test run on the
//! app's main context (the `shell` group does). As its own process, for runs that need one server
//! for several app instances: `family-connect --fake-server PORT` (feature `selftest`) prints
//! `FAKE-SERVER http://127.0.0.1:PORT` and serves [`FakeServer::seed_default`]'s people until
//! killed.
//!
//! **What it speaks** (S0's routes, `/api/v1` prefixed):
//!
//! | Route | Answer |
//! | --- | --- |
//! | `POST /auth/login` | `200 {token, user}`, or `401 invalid_credentials` |
//! | `POST /auth/register` | `201 {token, user}`, or `409 username_taken` |
//! | `POST /auth/logout` | `204`; the token is revoked |
//! | `GET /me` | `MeResponse`: the user, every family with its role (home first), `blocked_user_ids: []`, `calls_enabled` / `video_calls_enabled` from [`FakeState`], and paid families' keys ([`FakeState::paying`]) |
//! | `GET /families/mine`, `/families/{id}` | `FamilyResponse` with the members (their `avatar_version`), the pack's ceilings from [`FakeState`]; `404 family_not_found` outside them. A family carries its `plan`, `member_ceiling` and `delete_at` from [`FakeState::plans`] while subscriptions are on |
//! | `POST /families` | a new family owned by the caller: `201 {family}` — or `403 subscription_required` while [`FakeState::subscriptions`] is on and the caller holds no active subscription |
//! | `GET /chats` | the family's chat rows ([`FakeServer::set_chats`]), `[]` by default |
//! | `GET /chats/{id}/messages`, `/reactions`, `/edits`, `/polls` | empty lists |
//! | `GET /families/*/board`, `/board/changes`, `/pack`, `/pack/changes` | empty |
//! | `GET /calls/ice` | `{"ice_servers": []}` |
//! | `GET /ws` (WebSocket) | `Authorization: Bearer <live token>` or `401`; frames sent with [`FakeServer::send_frame`] (every socket) or [`FakeServer::send_frame_to`] (one person's) |
//!
//! Every request needs a live token but the three `auth` ones (`401 unauthorized` otherwise — the
//! protocol's canonical code, docs/protocol.md "Error shape"). Each socket remembers whose token
//! opened it, so a group can talk to one person ([`FakeServer::send_frame_to`]) and read what one
//! person's client sent ([`FakeServer::frames_received_from`]).
//! Anything else is `404 not_found` AND is recorded in [`FakeServer::unhandled`] — the `shell`
//! group fails on any, so a client request the fake does not know is never silently ignored.
//!
//! **How a package extends it** — from its self-test group, before it drives the app:
//!
//! ```text
//! let server = h.fake_server();
//! server.route("GET", "/api/v1/chats/*/messages", Rc::new(|server, request| { … FakeResponse::json(200, &body) }));
//! server.set_chats(family, rows);              // seed data
//! server.send_frame(&json);                   // a live frame to every open socket
//! server.send_frame_to(user, &json);          // … or to that person's sockets only
//! server.state().calls = true;                // `/me` says the server carries calls
//! ```
//!
//! A route added later wins over an earlier one for the same request, so a group may replace S0's
//! empty answers with its own. Patterns match path segments; `*` is any one segment, a trailing
//! `**` any rest. Handlers get the caller's user id (from the bearer token), the path's segments,
//! the query and the body, and may read and change [`FakeState`] through [`FakeServer::state`].
//! State lives as long as the server; a group that needs a clean one seeds fresh people.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use fc_core::protocol::{
    AllowanceDto, AuthResponse, BoardResponse, ChatRowDto, ChatsResponse, ErrorBody, ErrorEnvelope,
    FamilyDto, FamilyOnlyResponse, FamilyResponse, IceServersResponse, MeResponse, MemberDto,
    PackResponse, PaidFamilyLimitsDto, SubscriptionDto, UserDto,
};
use serde::Serialize;
use soup::prelude::*;

/// The socket's path.
pub const SOCKET_PATH: &str = "/api/v1/ws";

/// One request as a handler sees it.
#[derive(Debug, Clone)]
pub struct FakeRequest {
    pub method: String,
    pub path: String,
    pub segments: Vec<String>,
    pub query: Option<String>,
    pub body: Vec<u8>,
    /// The caller, when the request carried a live token.
    pub user: Option<i64>,
}

impl FakeRequest {
    /// The body as JSON (`Null` when it is not).
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

/// A handler's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeResponse {
    pub status: u32,
    pub body: Vec<u8>,
}

impl FakeResponse {
    /// `status` with `body` as JSON.
    pub fn json(status: u32, body: &impl Serialize) -> FakeResponse {
        FakeResponse {
            status,
            body: serde_json::to_vec(body).unwrap_or_default(),
        }
    }

    /// The protocol's error shape: `{"error": {"code": …, "message": …}}`.
    pub fn error(status: u32, code: &str) -> FakeResponse {
        FakeResponse::json(
            status,
            &ErrorEnvelope {
                error: ErrorBody {
                    code: code.to_string(),
                    message: code.replace('_', " "),
                },
            },
        )
    }

    /// No body (`204`).
    pub fn empty(status: u32) -> FakeResponse {
        FakeResponse {
            status,
            body: Vec::new(),
        }
    }
}

/// A route's handler.
pub type Handler = Rc<dyn Fn(&FakeServer, &FakeRequest) -> FakeResponse>;

struct Route {
    method: String,
    pattern: Vec<String>,
    handler: Handler,
}

/// A person the fake knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeUser {
    pub id: i64,
    pub username: String,
    pub password: String,
    pub display_name: String,
    /// `(family id, role)`, the home family first.
    pub families: Vec<(i64, String)>,
    /// The profile picture's version (0: none), as `/me` and the family's members say it.
    pub avatar_version: i32,
}

/// A family the fake knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeFamily {
    pub id: i64,
    pub name: String,
}

/// A family's standing on a server that charges (docs/protocol.md, "Paid families").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakePlan {
    /// `"free"` or `"paid"`.
    pub plan: String,
    /// `member_ceiling`.
    pub member_ceiling: i32,
    /// `delete_at` while the family is unpaid.
    pub delete_at: Option<String>,
}

/// Everything the fake remembers.
#[derive(Debug, Default)]
pub struct FakeState {
    pub users: Vec<FakeUser>,
    pub families: Vec<FakeFamily>,
    /// Live token → user id.
    pub tokens: HashMap<String, i64>,
    /// Chat rows per family.
    pub chats: HashMap<i64, Vec<ChatRowDto>>,
    pub next_id: i64,
    /// `/me`'s `calls_enabled`: the server carries voice calls.
    pub calls: bool,
    /// `/me`'s `video_calls_enabled`.
    pub video_calls: bool,
    /// The family's pack ceilings (`max_pack_items`, `max_pack_item_bytes`): a server that keeps
    /// a sticker pack says them; `None` is one that predates packs.
    pub pack_limits: Option<(i32, i64)>,
    /// `/me`'s `subscriptions_enabled`: the server charges for a new family (docs/protocol.md,
    /// "Paid families"). Off, nothing below is said.
    pub subscriptions: bool,
    /// Each account's subscription as `/me` reports it.
    pub subscriptions_held: HashMap<i64, SubscriptionDto>,
    /// Each family's plan, ceiling and deadline, said while subscriptions are on.
    pub plans: HashMap<i64, FakePlan>,
    /// The allowance a family's statistics carry.
    pub allowances: HashMap<i64, AllowanceDto>,
}

impl FakeState {
    fn next(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn user(&self, id: i64) -> Option<&FakeUser> {
        self.users.iter().find(|user| user.id == id)
    }

    /// A family as the caller sees it: with its role.
    pub fn family_for(&self, user: i64, family: i64) -> Option<FamilyDto> {
        let role = self
            .user(user)?
            .families
            .iter()
            .find(|(id, _)| *id == family)?
            .1
            .clone();
        let name = self.families.iter().find(|f| f.id == family)?.name.clone();
        let plan = self.plans.get(&family).filter(|_| self.subscriptions);
        Some(FamilyDto {
            role: Some(role),
            plan: plan.map(|plan| plan.plan.clone()),
            member_ceiling: plan.map(|plan| plan.member_ceiling),
            delete_at: plan.and_then(|plan| plan.delete_at.clone()),
            ..FamilyDto::new(family, &name)
        })
    }

    /// `me` with what a server that charges adds for `user` (docs/protocol.md, "Paid families"):
    /// the switch, their subscription, the limits and the days — nothing while the switch is off.
    pub fn paying(&self, user: i64, me: MeResponse) -> MeResponse {
        if !self.subscriptions {
            return me;
        }
        MeResponse {
            subscriptions_enabled: true,
            subscription: self.subscriptions_held.get(&user).cloned(),
            unpaid_days: Some(14),
            paid_family_limits: Some(PaidFamilyLimitsDto {
                members: Some(25),
                storage_bytes: Some(10 * 1024 * 1024 * 1024),
                ai_tokens: Some(2_000_000),
                images: Some(30),
                transcript_ms: Some(7_200_000),
            }),
            ..me
        }
    }

    /// Whether `user` holds an active subscription.
    pub fn subscribed(&self, user: i64) -> bool {
        self.subscriptions_held
            .get(&user)
            .is_some_and(|held| held.active)
    }

    fn user_dto(&self, id: i64) -> UserDto {
        self.user(id)
            .map(|user| UserDto {
                avatar_version: user.avatar_version,
                ..UserDto::new(user.id, &user.username, &user.display_name)
            })
            .unwrap_or_default()
    }
}

/// The fake server (see the module's comment).
pub struct FakeServer {
    server: soup::Server,
    port: u16,
    state: RefCell<FakeState>,
    routes: RefCell<Vec<Route>>,
    /// Each open socket with the user whose token upgraded it.
    sockets: Rc<RefCell<Vec<(Option<i64>, soup::WebsocketConnection)>>>,
    /// Text frames the clients sent, each with the socket's user.
    received: Rc<RefCell<Vec<(Option<i64>, String)>>>,
    requests: RefCell<Vec<String>>,
    unhandled: RefCell<Vec<String>>,
    down: Cell<bool>,
}

impl std::fmt::Debug for FakeServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeServer")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl FakeServer {
    /// A server on 127.0.0.1 at a free port, S0's routes installed, nobody known yet.
    pub fn start() -> Result<Rc<FakeServer>, String> {
        FakeServer::start_on(0)
    }

    /// The same at `port` (0: any free one).
    pub fn start_on(port: u16) -> Result<Rc<FakeServer>, String> {
        let server = soup::Server::builder().build();
        server
            .listen_local(u32::from(port), soup::ServerListenOptions::IPV4_ONLY)
            .map_err(|error| format!("the fake server could not listen: {error}"))?;
        let port = server
            .uris()
            .first()
            .and_then(|uri| u16::try_from(uri.port()).ok())
            .ok_or("the fake server has no address")?;
        let fake = Rc::new(FakeServer {
            server,
            port,
            state: RefCell::new(FakeState::default()),
            routes: RefCell::new(Vec::new()),
            sockets: Rc::new(RefCell::new(Vec::new())),
            received: Rc::new(RefCell::new(Vec::new())),
            requests: RefCell::new(Vec::new()),
            unhandled: RefCell::new(Vec::new()),
            down: Cell::new(false),
        });
        fake.listen();
        install_routes(&fake);
        Ok(fake)
    }

    /// `http://127.0.0.1:<port>` — what the app is pointed at.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn listen(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.server
            .add_early_handler(Some(SOCKET_PATH), move |_, message, _, _| {
                let Some(fake) = weak.upgrade() else {
                    return;
                };
                if fake.down.get() {
                    message.set_status(503, None);
                    return;
                }
                let user = fake.caller(message);
                fake.requests
                    .borrow_mut()
                    .push(format!("GET {SOCKET_PATH} (upgrade)"));
                if user.is_none() {
                    message.set_status(401, None);
                }
            });
        {
            let (sockets, received) = (Rc::clone(&self.sockets), Rc::clone(&self.received));
            let weak = Rc::downgrade(self);
            self.server.add_websocket_handler(
                Some(SOCKET_PATH),
                None,
                &[],
                move |_, message, _, connection| {
                    let user = weak.upgrade().and_then(|fake| fake.caller(message));
                    let received = Rc::clone(&received);
                    connection.connect_message(move |_, _, bytes| {
                        received
                            .borrow_mut()
                            .push((user, String::from_utf8_lossy(bytes).into_owned()));
                    });
                    sockets.borrow_mut().push((user, connection.clone()));
                },
            );
        }
        let weak = Rc::downgrade(self);
        self.server.add_handler(None, move |_, message, path, _| {
            let Some(fake) = weak.upgrade() else {
                return;
            };
            if path == SOCKET_PATH {
                return;
            }
            fake.serve(message, path);
        });
    }

    /// The user a request's bearer token names, when it is live.
    fn caller(&self, message: &soup::ServerMessage) -> Option<i64> {
        let value = message.request_headers()?.one("Authorization")?;
        let token = value.strip_prefix("Bearer ")?;
        self.state.borrow().tokens.get(token).copied()
    }

    fn serve(&self, message: &soup::ServerMessage, path: &str) {
        let method = message.method().map(|m| m.to_string()).unwrap_or_default();
        let query = message
            .uri()
            .and_then(|uri| uri.query())
            .map(|q| q.to_string());
        let body = message
            .request_body()
            .and_then(|body| body.flatten())
            .map(|bytes| bytes.to_vec())
            .unwrap_or_default();
        self.requests.borrow_mut().push(format!("{method} {path}"));
        let request = FakeRequest {
            segments: segments(path),
            method,
            path: path.to_string(),
            query,
            body,
            user: self.caller(message),
        };
        let response = if self.down.get() {
            FakeResponse::error(503, "unavailable")
        } else {
            self.answer(&request)
        };
        message.set_status(response.status, None);
        if !response.body.is_empty() {
            message.set_response(
                Some(&content_type(&response.body)),
                soup::MemoryUse::Copy,
                &response.body,
            );
        }
    }

    fn answer(&self, request: &FakeRequest) -> FakeResponse {
        let handler = self
            .routes
            .borrow()
            .iter()
            .rev()
            .find(|route| {
                route.method == request.method && matches(&route.pattern, &request.segments)
            })
            .map(|route| Rc::clone(&route.handler));
        let open = request.path.starts_with("/api/v1/auth/");
        match handler {
            Some(_) if !open && request.user.is_none() => FakeResponse::error(401, "unauthorized"),
            Some(handler) => handler(self, request),
            None => {
                let line = format!("{} {}", request.method, request.path);
                eprintln!("fake-server: unhandled {line}");
                self.unhandled.borrow_mut().push(line);
                FakeResponse::error(404, "not_found")
            }
        }
    }

    // ---- what the groups drive it with ---------------------------------------------------------

    /// Adds a route (see the module's comment): `method`, a pattern like
    /// `/api/v1/families/*/board`, and its handler. Later routes win.
    pub fn route(&self, method: &str, pattern: &str, handler: Handler) {
        self.routes.borrow_mut().push(Route {
            method: method.to_string(),
            pattern: segments(pattern),
            handler,
        });
    }

    /// The state, to read or change.
    pub fn state(&self) -> std::cell::RefMut<'_, FakeState> {
        self.state.borrow_mut()
    }

    /// A person with no family. Answers their id.
    pub fn add_user(&self, username: &str, password: &str, display_name: &str) -> i64 {
        let mut state = self.state.borrow_mut();
        let id = state.next();
        state.users.push(FakeUser {
            id,
            username: username.to_string(),
            password: password.to_string(),
            display_name: display_name.to_string(),
            families: Vec::new(),
            avatar_version: 0,
        });
        id
    }

    /// A family owned by `owner`. Answers its id.
    pub fn add_family(&self, name: &str, owner: i64) -> i64 {
        let id = {
            let mut state = self.state.borrow_mut();
            let id = state.next();
            state.families.push(FakeFamily {
                id,
                name: name.to_string(),
            });
            id
        };
        self.join(owner, id, "owner");
        id
    }

    /// `user` joins `family` as `role` (`owner`, `member`).
    pub fn join(&self, user: i64, family: i64, role: &str) {
        let mut state = self.state.borrow_mut();
        if let Some(user) = state.users.iter_mut().find(|u| u.id == user) {
            user.families.retain(|(id, _)| *id != family);
            user.families.push((family, role.to_string()));
        }
    }

    /// The chat rows `GET /chats` answers for `family`.
    pub fn set_chats(&self, family: i64, rows: Vec<ChatRowDto>) {
        self.state.borrow_mut().chats.insert(family, rows);
    }

    /// Every token dies: REST answers `401 unauthorized`, the socket's upgrade `401`.
    pub fn expire_tokens(&self) {
        self.state.borrow_mut().tokens.clear();
    }

    /// While down, every request is `503 unavailable` and the socket's upgrade `503` — a server
    /// the network reaches but that cannot answer.
    pub fn set_down(&self, down: bool) {
        self.down.set(down);
    }

    /// Every request so far: `METHOD /path` (no query, no body).
    pub fn requests(&self) -> Vec<String> {
        self.requests.borrow().clone()
    }

    /// Requests no route answered.
    pub fn unhandled(&self) -> Vec<String> {
        self.unhandled.borrow().clone()
    }

    /// Sockets open now.
    pub fn open_sockets(&self) -> usize {
        self.sockets
            .borrow()
            .iter()
            .filter(|(_, s)| s.state() == soup::WebsocketState::Open)
            .count()
    }

    /// Sends a text frame to every open socket.
    pub fn send_frame(&self, json: &str) {
        for (_, socket) in self.sockets.borrow().iter() {
            if socket.state() == soup::WebsocketState::Open {
                socket.send_text(json);
            }
        }
    }

    /// Sends a text frame to the open sockets `user`'s token upgraded — what the real server does
    /// with a frame meant for one person.
    pub fn send_frame_to(&self, user: i64, json: &str) {
        for (owner, socket) in self.sockets.borrow().iter() {
            if *owner == Some(user) && socket.state() == soup::WebsocketState::Open {
                socket.send_text(json);
            }
        }
    }

    /// Closes every open socket with `code` (4401: the session expired).
    pub fn close_sockets(&self, code: u16) {
        for (_, socket) in self.sockets.borrow().iter() {
            if socket.state() == soup::WebsocketState::Open {
                socket.close(code, None);
            }
        }
    }

    /// Text frames the clients sent, from every socket, in order.
    pub fn frames_received(&self) -> Vec<String> {
        self.received
            .borrow()
            .iter()
            .map(|(_, frame)| frame.clone())
            .collect()
    }

    /// Text frames `user`'s sockets sent, in order.
    pub fn frames_received_from(&self, user: i64) -> Vec<String> {
        self.received
            .borrow()
            .iter()
            .filter(|(owner, _)| *owner == Some(user))
            .map(|(_, frame)| frame.clone())
            .collect()
    }

    /// The people of a ready-made world: `alice` owns "The Testers" with `bob` in it; `carol` is in
    /// no family. Every password is `password`.
    pub fn seed_default(&self) -> (i64, i64, i64, i64) {
        let alice = self.add_user("alice", "password", "Alice");
        let bob = self.add_user("bob", "password", "Bob");
        let carol = self.add_user("carol", "password", "Carol");
        let family = self.add_family("The Testers", alice);
        self.join(bob, family, "member");
        (alice, bob, carol, family)
    }
}

/// What a body is: JSON, or what GIO sniffs (a picture, an MP4 a player streams).
fn content_type(body: &[u8]) -> String {
    if serde_json::from_slice::<serde::de::IgnoredAny>(body).is_ok() {
        return "application/json".to_string();
    }
    let (guessed, _) = gio::content_type_guess(None::<&str>, body);
    gio::content_type_get_mime_type(&guessed).map_or_else(
        || "application/octet-stream".to_string(),
        |mime| mime.to_string(),
    )
}

/// A path's segments (`/api/v1/me` → `["api", "v1", "me"]`).
fn segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether `pattern` (segments with `*` and a trailing `**`) matches `path`.
fn matches(pattern: &[String], path: &[String]) -> bool {
    let mut path = path.iter();
    for (i, want) in pattern.iter().enumerate() {
        if want == "**" && i + 1 == pattern.len() {
            return true;
        }
        match path.next() {
            Some(got) if want == "*" || want == got => {}
            _ => return false,
        }
    }
    path.next().is_none()
}

/// The family a `/families/{x}/…` path names for `user`: `mine` is the home family.
fn family_of(state: &FakeState, user: i64, which: &str) -> Option<i64> {
    let families = &state.user(user)?.families;
    if which == "mine" {
        return families.first().map(|(id, _)| *id);
    }
    let id: i64 = which.parse().ok()?;
    families.iter().any(|(f, _)| *f == id).then_some(id)
}

fn mint(state: &mut FakeState, user: i64) -> String {
    let token = format!("fake-token-{user}-{}", state.next());
    state.tokens.insert(token.clone(), user);
    token
}

/// S0's routes (see the module's comment).
fn install_routes(fake: &FakeServer) {
    fake.route(
        "POST",
        "/api/v1/auth/login",
        Rc::new(|fake, request| {
            let body = request.json();
            let (username, password) = (
                body["username"].as_str().unwrap_or_default(),
                body["password"].as_str().unwrap_or_default(),
            );
            let mut state = fake.state();
            let Some(id) = state
                .users
                .iter()
                .find(|u| u.username == username && u.password == password)
                .map(|u| u.id)
            else {
                return FakeResponse::error(401, "invalid_credentials");
            };
            let token = mint(&mut state, id);
            FakeResponse::json(
                200,
                &AuthResponse {
                    token,
                    user: state.user_dto(id),
                },
            )
        }),
    );
    fake.route(
        "POST",
        "/api/v1/auth/register",
        Rc::new(|fake, request| {
            let body = request.json();
            let username = body["username"].as_str().unwrap_or_default().to_string();
            if fake.state().users.iter().any(|u| u.username == username) {
                return FakeResponse::error(409, "username_taken");
            }
            let id = fake.add_user(
                &username,
                body["password"].as_str().unwrap_or_default(),
                body["display_name"].as_str().unwrap_or_default(),
            );
            let mut state = fake.state();
            let token = mint(&mut state, id);
            FakeResponse::json(
                201,
                &AuthResponse {
                    token,
                    user: state.user_dto(id),
                },
            )
        }),
    );
    fake.route(
        "POST",
        "/api/v1/auth/logout",
        Rc::new(|fake, request| {
            if let Some(user) = request.user {
                fake.state().tokens.retain(|_, held| *held != user);
            }
            FakeResponse::empty(204)
        }),
    );
    fake.route(
        "GET",
        "/api/v1/me",
        Rc::new(|fake, request| {
            let state = fake.state();
            let user = request.user.unwrap_or_default();
            let families: Vec<FamilyDto> = state
                .user(user)
                .map(|u| u.families.clone())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(id, _)| state.family_for(user, id))
                .collect();
            FakeResponse::json(
                200,
                &state.paying(
                    user,
                    MeResponse {
                        user: state.user_dto(user),
                        family: families.first().cloned(),
                        role: families.first().and_then(|f| f.role.clone()),
                        blocked_user_ids: Some(Vec::new()),
                        families: Some(families),
                        pending_join_requests: Some(Vec::new()),
                        invitations: Some(Vec::new()),
                        member_families_enabled: true,
                        max_families_per_account: 3,
                        max_family_members: 12,
                        calls_enabled: state.calls,
                        video_calls_enabled: state.video_calls,
                        ..MeResponse::default()
                    },
                ),
            )
        }),
    );
    let family = |fake: &FakeServer, request: &FakeRequest| {
        let state = fake.state();
        let user = request.user.unwrap_or_default();
        let Some(id) = request
            .segments
            .get(3)
            .and_then(|which| family_of(&state, user, which))
        else {
            return FakeResponse::error(404, "family_not_found");
        };
        let members: Vec<MemberDto> = state
            .users
            .iter()
            .filter_map(|u| {
                let role = u.families.iter().find(|(f, _)| *f == id)?.1.clone();
                Some(MemberDto {
                    id: u.id,
                    username: u.username.clone(),
                    display_name: u.display_name.clone(),
                    avatar_version: u.avatar_version,
                    role: Some(role),
                    ..MemberDto::default()
                })
            })
            .collect();
        FakeResponse::json(
            200,
            &FamilyResponse {
                family: state.family_for(user, id).unwrap_or_default(),
                members: Some(members),
                blocked_user_ids: Some(Vec::new()),
                max_board_seq: Some(0),
                max_pack_seq: Some(0),
                max_pack_items: state.pack_limits.map(|(items, _)| items),
                max_pack_item_bytes: state.pack_limits.map(|(_, bytes)| bytes),
                ..FamilyResponse::default()
            },
        )
    };
    fake.route("GET", "/api/v1/families/*", Rc::new(family));
    fake.route(
        "POST",
        "/api/v1/families",
        Rc::new(|fake, request| {
            let name = request.json()["name"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let user = request.user.unwrap_or_default();
            // Checked after everything else a create can meet (docs/protocol.md, "Paid families").
            let refused = {
                let state = fake.state();
                state.subscriptions && !state.subscribed(user)
            };
            if refused {
                return FakeResponse::error(403, "subscription_required");
            }
            let id = fake.add_family(&name, user);
            let family = fake.state().family_for(user, id).unwrap_or_default();
            FakeResponse::json(201, &FamilyOnlyResponse { family })
        }),
    );
    fake.route(
        "GET",
        "/api/v1/chats",
        Rc::new(|fake, request| {
            let state = fake.state();
            let user = request.user.unwrap_or_default();
            let rows = family_of(&state, user, "mine")
                .and_then(|family| state.chats.get(&family).cloned())
                .unwrap_or_default();
            FakeResponse::json(200, &ChatsResponse { chats: Some(rows) })
        }),
    );
    let empty_messages: Handler =
        Rc::new(|_, _| FakeResponse::json(200, &serde_json::json!({"messages": []})));
    fake.route("GET", "/api/v1/chats/*/messages", empty_messages);
    // A reader's marker (docs/protocol.md: `{last_read_message_id}` → 204): any open conversation
    // reports it, so every group meets it — the chat group records them with its own route.
    fake.route(
        "POST",
        "/api/v1/chats/*/read",
        Rc::new(|_, _| FakeResponse::empty(204)),
    );
    fake.route(
        "GET",
        "/api/v1/chats/*/reactions",
        Rc::new(|_, _| FakeResponse::json(200, &serde_json::json!({"message_reactions": []}))),
    );
    fake.route(
        "GET",
        "/api/v1/chats/*/edits",
        Rc::new(|_, _| FakeResponse::json(200, &serde_json::json!({"messages": []}))),
    );
    fake.route(
        "GET",
        "/api/v1/chats/*/polls",
        Rc::new(|_, _| FakeResponse::json(200, &serde_json::json!({"polls": []}))),
    );
    fake.route(
        "GET",
        "/api/v1/families/*/board",
        Rc::new(|_, _| {
            FakeResponse::json(
                200,
                &BoardResponse {
                    notes: Some(Vec::new()),
                    max_board_seq: 0,
                },
            )
        }),
    );
    fake.route(
        "GET",
        "/api/v1/families/*/board/changes",
        Rc::new(|_, _| FakeResponse::json(200, &serde_json::json!({"notes": []}))),
    );
    fake.route(
        "GET",
        "/api/v1/families/*/pack",
        Rc::new(|_, _| {
            FakeResponse::json(
                200,
                &PackResponse {
                    items: Some(Vec::new()),
                    max_pack_seq: 0,
                },
            )
        }),
    );
    fake.route(
        "GET",
        "/api/v1/families/*/pack/changes",
        Rc::new(|_, _| FakeResponse::json(200, &serde_json::json!({"items": []}))),
    );
    fake.route(
        "GET",
        "/api/v1/calls/ice",
        Rc::new(|_, _| {
            FakeResponse::json(
                200,
                &IceServersResponse {
                    ice_servers: Some(Vec::new()),
                    ttl_secs: Some(3600),
                },
            )
        }),
    );
}

/// `--fake-server PORT` (taken out of `args` before GApplication sees it): `Ok(Some(port))`.
pub fn take(args: &mut Vec<std::ffi::OsString>) -> Result<Option<u16>, String> {
    let Some(at) = args.iter().position(|a| a == "--fake-server") else {
        return Ok(None);
    };
    let port = args
        .get(at + 1)
        .and_then(|p| p.to_str())
        .and_then(|p| p.parse::<u16>().ok())
        .ok_or("usage: family-connect --fake-server PORT (0: any free port)")?;
    args.drain(at..at + 2);
    Ok(Some(port))
}

/// Serves [`FakeServer::seed_default`]'s world on `port` until the process is killed.
pub fn serve_forever(port: u16) -> glib::ExitCode {
    let main_loop = glib::MainLoop::new(None, false);
    let server = match FakeServer::start_on(port) {
        Ok(server) => server,
        Err(message) => {
            eprintln!("family-connect: {message}");
            return glib::ExitCode::FAILURE;
        }
    };
    server.seed_default();
    println!("FAKE-SERVER {}", server.url());
    main_loop.run();
    drop(server);
    glib::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(text: &str) -> Vec<String> {
        segments(text)
    }

    #[test]
    fn patterns_match_by_segment_with_wildcards() {
        let p = pattern("/api/v1/families/*/board");
        assert!(matches(&p, &segments("/api/v1/families/mine/board")));
        assert!(matches(&p, &segments("/api/v1/families/12/board")));
        assert!(!matches(&p, &segments("/api/v1/families/12/board/changes")));
        assert!(!matches(&p, &segments("/api/v1/families/12")));
        let rest = pattern("/api/v1/chats/**");
        assert!(matches(&rest, &segments("/api/v1/chats/1/messages")));
        assert!(matches(&pattern("/api/v1/me"), &segments("/api/v1/me/")));
        assert!(!matches(&pattern("/api/v1/me"), &segments("/api/v1/men")));
    }

    #[test]
    fn the_command_line_option_is_taken_out() {
        let mut args: Vec<std::ffi::OsString> = ["family-connect", "--fake-server", "8099"]
            .iter()
            .map(Into::into)
            .collect();
        assert_eq!(take(&mut args), Ok(Some(8099)));
        assert_eq!(args, vec![std::ffi::OsString::from("family-connect")]);
        let mut bad: Vec<std::ffi::OsString> = ["family-connect", "--fake-server", "x"]
            .iter()
            .map(Into::into)
            .collect();
        assert!(take(&mut bad).is_err());
    }

    #[test]
    fn the_fake_answers_the_gate_over_real_http() {
        use fc_core::rt::{Http, HttpRequest, Method};
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                context.block_on(async {
                    let fake = FakeServer::start().unwrap();
                    fake.seed_default();
                    let http = crate::services::soup_http::SoupHttp::new();
                    let me = HttpRequest::new(Method::Get, format!("{}/api/v1/me", fake.url()));
                    assert_eq!(http.send(me.clone()).await.unwrap().status, 401);
                    let login =
                        HttpRequest::new(Method::Post, format!("{}/api/v1/auth/login", fake.url()))
                            .body(
                                "application/json",
                                br#"{"username":"alice","password":"password"}"#.to_vec(),
                            );
                    let answer = http.send(login).await.unwrap();
                    assert_eq!(answer.status, 200);
                    let auth: AuthResponse = serde_json::from_slice(&answer.body).unwrap();
                    let me = me.header("Authorization", format!("Bearer {}", auth.token));
                    let answer = http.send(me).await.unwrap();
                    let me: MeResponse = serde_json::from_slice(&answer.body).unwrap();
                    assert_eq!(me.user.username, "alice");
                    assert_eq!(me.families.as_ref().map(Vec::len), Some(1));
                    assert_eq!(me.role.as_deref(), Some("owner"));
                    let nowhere =
                        HttpRequest::new(Method::Get, format!("{}/api/v1/nowhere", fake.url()))
                            .header("Authorization", format!("Bearer {}", auth.token));
                    assert_eq!(http.send(nowhere).await.unwrap().status, 404);
                    assert_eq!(fake.unhandled(), vec!["GET /api/v1/nowhere".to_string()]);
                })
            })
            .unwrap();
    }
}
