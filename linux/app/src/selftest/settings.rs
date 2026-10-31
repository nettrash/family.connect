//! The `settings` group (P4, §27) — and the fake server's half that P4's three groups share
//! ([`World`], installed once with [`install`]): the owner's console, birthdays, the board's notes,
//! groups, and a `GET /me` whose `member_families_enabled` a check can turn off.
//!
//! The group: the Settings window opens on Ctrl+, at its size; every device row reads and writes
//! its key (notify, keep-running, hot-key, link and map previews, voice speed, video camera); the
//! Top Bar rows show the stand-in states of P5's services (no icon; the portal's Bound / NotBound /
//! NoPortal; autostart Disabled → Enabled, DisabledByUser, Unavailable); "New Family…" is shown
//! for a member the server lets start one and hidden when the server does not or the account owns
//! a family; the profile's birthday is saved; the "isn't remembered" line; Log Out asks and returns
//! the window to the sign-in gate. Screenshots: light, dark, narrow.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::{
    BirthdayDto, BoardResponse, ChatDto, ChatResponse, FamilyDto, FamilyOnlyResponse,
    FamilyResponse, InvitationDto, InvitationResponse, InvitationsResponse, InviteCodeResponse,
    JoinRequestDto, JoinRequestsResponse, LeftAnswer, MeResponse, MemberDto, MemberResponse,
    NoteDto, NoteResponse, ReportDto, ReportResponse, ReportsResponse, RsvpDto, ServerUrl,
    TaskItemDto, UserDto, UserResponse,
};
use fc_core::rt::Event;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::global_hot_key_rules::PortalState;
use fc_logic::startup_setting::StartupState;

use super::Harness;
use super::fake_server::{FakeRequest, FakeResponse, FakeServer, FakeState};
use crate::seams::desktop::{AutostartState, HotKeyState, TrayState};
use crate::seams::media::{Camera, CameraDevice, CameraRefusal};
use crate::services::settings_store::keys;
use crate::shell::{Connection, Shell};
use crate::windows::main_window::GatePage;
use crate::windows::settings_window;

const WAIT: Duration = Duration::from_secs(15);

// ---- the shared fake world ------------------------------------------------------------------------

/// What P4's routes remember beside S0's `FakeState`.
#[derive(Debug)]
pub struct World {
    installed: bool,
    pub birthdays: HashMap<i64, BirthdayDto>,
    /// `GET /me`'s `member_families_enabled`.
    pub member_families: bool,
    pub max_families: i32,
    pub codes: HashMap<i64, String>,
    pub policies: HashMap<i64, String>,
    pub caps: HashMap<i64, i32>,
    /// (request id, family, user)
    pub requests: Vec<(i64, i64, i64)>,
    /// (family, report)
    pub reports: Vec<(i64, ReportDto)>,
    pub invitations: Vec<InvitationDto>,
    pub notes: Vec<NoteDto>,
    pub groups: Vec<ChatDto>,
    seq: i64,
    next: i64,
}

impl Default for World {
    fn default() -> Self {
        World {
            installed: false,
            birthdays: HashMap::new(),
            member_families: true,
            max_families: 3,
            codes: HashMap::new(),
            policies: HashMap::new(),
            caps: HashMap::new(),
            requests: Vec::new(),
            reports: Vec::new(),
            invitations: Vec::new(),
            notes: Vec::new(),
            groups: Vec::new(),
            seq: 0,
            next: 50_000,
        }
    }
}

impl World {
    pub fn next_id(&mut self) -> i64 {
        self.next += 1;
        self.next
    }

    fn next_seq(&mut self) -> i64 {
        self.seq += 1;
        self.seq
    }

    /// A note on `family`'s wall, written by `author` — as the server would store it.
    pub fn add_note(&mut self, family: i64, author: i64, mut note: NoteDto) -> NoteDto {
        note.id = self.next_id();
        note.author_id = author;
        note.family_id = Some(family);
        let seq = self.next_seq();
        note.board_seq = seq;
        note.content_seq = Some(seq);
        self.notes.push(note.clone());
        note
    }
}

thread_local! {
    static WORLD: Rc<RefCell<World>> = Rc::new(RefCell::new(World::default()));
}

/// P4's fake world.
pub fn world() -> Rc<RefCell<World>> {
    WORLD.with(Rc::clone)
}

/// The family a `/families/{which}` path names for `user` (`mine`: the home family).
fn family_of(state: &FakeState, user: i64, which: &str) -> Option<i64> {
    let families = &state.user(user)?.families;
    if which == "mine" {
        return families.first().map(|(id, _)| *id);
    }
    let id: i64 = which.parse().ok()?;
    families.iter().any(|(f, _)| *f == id).then_some(id)
}

fn user_dto(state: &FakeState, world: &World, id: i64) -> UserDto {
    let mut user = state
        .user(id)
        .map(|user| UserDto {
            avatar_version: user.avatar_version,
            ..UserDto::new(user.id, &user.username, &user.display_name)
        })
        .unwrap_or_default();
    user.birthday = world.birthdays.get(&id).copied();
    user
}

fn role_in(state: &FakeState, user: i64, family: i64) -> Option<String> {
    state
        .user(user)?
        .families
        .iter()
        .find(|(id, _)| *id == family)
        .map(|(_, role)| role.clone())
}

fn family_dto(state: &FakeState, world: &World, user: i64, family: i64) -> Option<FamilyDto> {
    let mut dto = state.family_for(user, family)?;
    dto.join_policy = Some(
        world
            .policies
            .get(&family)
            .cloned()
            .unwrap_or_else(|| "open".to_string()),
    );
    dto.max_members = world.caps.get(&family).copied();
    if dto.role.as_deref() == Some("owner") {
        dto.invite_code = Some(
            world
                .codes
                .get(&family)
                .cloned()
                .unwrap_or_else(|| format!("CODE{family}")),
        );
    }
    Some(dto)
}

fn members_of(state: &FakeState, world: &World, family: i64) -> Vec<MemberDto> {
    state
        .users
        .iter()
        .filter_map(|u| {
            let role = u.families.iter().find(|(f, _)| *f == family)?.1.clone();
            Some(MemberDto {
                id: u.id,
                username: u.username.clone(),
                display_name: u.display_name.clone(),
                avatar_version: u.avatar_version,
                role: Some(role),
                birthday: world.birthdays.get(&u.id).copied(),
                ..MemberDto::default()
            })
        })
        .collect()
}

/// The family a request's path names (segment 3), for its caller.
fn path_family(fake: &FakeServer, request: &FakeRequest) -> Option<i64> {
    let state = fake.state();
    let user = request.user?;
    family_of(&state, user, request.segments.get(3)?)
}

fn segment_id(request: &FakeRequest, at: usize) -> Option<i64> {
    request.segments.get(at).and_then(|s| s.parse().ok())
}

/// Installs P4's routes on the run's fake server, once.
pub fn install(fake: &Rc<FakeServer>) {
    let world = world();
    if world.borrow().installed {
        return;
    }
    world.borrow_mut().installed = true;

    {
        let world = Rc::clone(&world);
        fake.route(
            "GET",
            "/api/v1/me",
            Rc::new(move |fake, request| {
                let state = fake.state();
                let world = world.borrow();
                let user = request.user.unwrap_or_default();
                let families: Vec<FamilyDto> = state
                    .user(user)
                    .map(|u| u.families.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|(id, _)| family_dto(&state, &world, user, id))
                    .collect();
                let invitations: Vec<InvitationDto> = world
                    .invitations
                    .iter()
                    .filter(|invitation| invitation.user.id == user)
                    .cloned()
                    .collect();
                FakeResponse::json(
                    200,
                    // And for paid families: S0's `FakeState` says what a charging server adds.
                    &state.paying(
                        user,
                        MeResponse {
                            user: user_dto(&state, &world, user),
                            family: families.first().cloned(),
                            role: families.first().and_then(|f| f.role.clone()),
                            blocked_user_ids: Some(Vec::new()),
                            families: Some(families),
                            pending_join_requests: Some(Vec::new()),
                            invitations: Some(invitations),
                            member_families_enabled: world.member_families,
                            max_families_per_account: world.max_families,
                            max_family_members: 12,
                            // The run's one truth for calls (S0's `FakeState`), whoever answers
                            // `/me`.
                            calls_enabled: state.calls,
                            video_calls_enabled: state.video_calls,
                            ..MeResponse::default()
                        },
                    ),
                )
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "GET",
            "/api/v1/families/*",
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let state = fake.state();
                let world = world.borrow();
                let user = request.user.unwrap_or_default();
                let members = members_of(&state, &world, family);
                let next_owner = members
                    .iter()
                    .filter(|member| !member.owner())
                    .map(|member| member.id)
                    .min();
                FakeResponse::json(
                    200,
                    &FamilyResponse {
                        family: family_dto(&state, &world, user, family).unwrap_or_default(),
                        members: Some(members),
                        blocked_user_ids: Some(Vec::new()),
                        max_board_seq: Some(world.seq),
                        max_pack_seq: Some(0),
                        max_pack_items: state.pack_limits.map(|(items, _)| items),
                        max_pack_item_bytes: state.pack_limits.map(|(_, bytes)| bytes),
                        next_owner_user_id: next_owner,
                        ..FamilyResponse::default()
                    },
                )
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "PATCH",
            "/api/v1/families/*",
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let body = request.json();
                {
                    let mut world = world.borrow_mut();
                    if let Some(policy) = body["join_policy"].as_str() {
                        world.policies.insert(family, policy.to_string());
                    }
                    match &body["max_members"] {
                        serde_json::Value::Null if body.get("max_members").is_some() => {
                            world.caps.remove(&family);
                        }
                        value => {
                            if let Some(cap) = value.as_i64().and_then(|c| i32::try_from(c).ok()) {
                                world.caps.insert(family, cap);
                            }
                        }
                    }
                }
                let state = fake.state();
                let world = world.borrow();
                let user = request.user.unwrap_or_default();
                FakeResponse::json(
                    200,
                    &FamilyOnlyResponse {
                        family: family_dto(&state, &world, user, family).unwrap_or_default(),
                    },
                )
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "POST",
            "/api/v1/families/*/invite-code/rotate",
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let mut world = world.borrow_mut();
                let code = format!("ROT{}", world.next_id());
                world.codes.insert(family, code.clone());
                FakeResponse::json(200, &InviteCodeResponse { invite_code: code })
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "GET",
            "/api/v1/families/*/join-requests",
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let state = fake.state();
                let world = world.borrow();
                let requests = world
                    .requests
                    .iter()
                    .filter(|(_, f, _)| *f == family)
                    .map(|(id, _, user)| JoinRequestDto {
                        id: *id,
                        user: user_dto(&state, &world, *user),
                        created_at: None,
                    })
                    .collect();
                FakeResponse::json(
                    200,
                    &JoinRequestsResponse {
                        requests: Some(requests),
                    },
                )
            }),
        );
    }
    for verb in ["approve", "reject"] {
        let world = Rc::clone(&world);
        fake.route(
            "POST",
            &format!("/api/v1/families/*/join-requests/*/{verb}"),
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let id = segment_id(request, 5).unwrap_or_default();
                let taken = {
                    let mut world = world.borrow_mut();
                    let at = world
                        .requests
                        .iter()
                        .position(|(r, f, _)| *r == id && *f == family);
                    at.map(|at| world.requests.remove(at))
                };
                let Some((_, _, user)) = taken else {
                    return FakeResponse::error(404, "not_found");
                };
                if verb == "reject" {
                    return FakeResponse::empty(204);
                }
                fake.join(user, family, "member");
                let state = fake.state();
                let world = world.borrow();
                let member = members_of(&state, &world, family)
                    .into_iter()
                    .find(|member| member.id == user)
                    .unwrap_or_default();
                FakeResponse::json(200, &MemberResponse { member })
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "GET",
            "/api/v1/families/*/reports",
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let reports = world
                    .borrow()
                    .reports
                    .iter()
                    .filter(|(f, _)| *f == family)
                    .map(|(_, report)| report.clone())
                    .collect();
                FakeResponse::json(
                    200,
                    &ReportsResponse {
                        reports: Some(reports),
                    },
                )
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "POST",
            "/api/v1/families/*/reports",
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let body = request.json();
                let reported = body["reported_user_id"].as_i64().unwrap_or_default();
                let state = fake.state();
                let mut world = world.borrow_mut();
                let report = ReportDto {
                    id: world.next_id(),
                    reporter: user_dto(&state, &world, request.user.unwrap_or_default()),
                    reported: user_dto(&state, &world, reported),
                    reason: body["reason"].as_str().unwrap_or("other").to_string(),
                    ..ReportDto::default()
                };
                world.reports.push((family, report.clone()));
                FakeResponse::json(201, &ReportResponse { report })
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "POST",
            "/api/v1/families/*/reports/*/resolve",
            Rc::new(move |_, request| {
                let id = segment_id(request, 5).unwrap_or_default();
                world
                    .borrow_mut()
                    .reports
                    .retain(|(_, report)| report.id != id);
                FakeResponse::empty(204)
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "GET",
            "/api/v1/families/*/invitations",
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let invitations = world
                    .borrow()
                    .invitations
                    .iter()
                    .filter(|invitation| invitation.family_id == family)
                    .cloned()
                    .collect();
                FakeResponse::json(
                    200,
                    &InvitationsResponse {
                        invitations: Some(invitations),
                    },
                )
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "POST",
            "/api/v1/families/*/invitations",
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let user = request.json()["user_id"].as_i64().unwrap_or_default();
                let state = fake.state();
                let mut world = world.borrow_mut();
                let invitation = InvitationDto {
                    id: world.next_id(),
                    family_id: family,
                    family_name: state
                        .families
                        .iter()
                        .find(|f| f.id == family)
                        .map(|f| f.name.clone())
                        .unwrap_or_default(),
                    invited_by: user_dto(&state, &world, request.user.unwrap_or_default()),
                    user: user_dto(&state, &world, user),
                    created_at: None,
                };
                world.invitations.push(invitation.clone());
                FakeResponse::json(201, &InvitationResponse { invitation })
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "DELETE",
            "/api/v1/families/*/invitations/*",
            Rc::new(move |_, request| {
                let id = segment_id(request, 5).unwrap_or_default();
                world.borrow_mut().invitations.retain(|i| i.id != id);
                FakeResponse::empty(204)
            }),
        );
    }
    fake.route(
        "DELETE",
        "/api/v1/families/*/members/*",
        Rc::new(|fake, request| {
            let Some(family) = path_family(fake, request) else {
                return FakeResponse::error(404, "family_not_found");
            };
            let user = segment_id(request, 5).unwrap_or_default();
            if let Some(person) = fake.state().users.iter_mut().find(|u| u.id == user) {
                person.families.retain(|(f, _)| *f != family);
            }
            FakeResponse::empty(204)
        }),
    );
    fake.route(
        "POST",
        "/api/v1/families/*/leave",
        Rc::new(|fake, request| {
            let Some(family) = path_family(fake, request) else {
                return FakeResponse::error(404, "family_not_found");
            };
            let me = request.user.unwrap_or_default();
            let mut state = fake.state();
            let owner = role_in(&state, me, family).as_deref() == Some("owner");
            if let Some(person) = state.users.iter_mut().find(|u| u.id == me) {
                person.families.retain(|(f, _)| *f != family);
            }
            let heir = if owner {
                state
                    .users
                    .iter()
                    .filter(|u| u.families.iter().any(|(f, _)| *f == family))
                    .map(|u| u.id)
                    .min()
            } else {
                None
            };
            if let Some(heir) = heir
                && let Some(person) = state.users.iter_mut().find(|u| u.id == heir)
            {
                for entry in person.families.iter_mut().filter(|(f, _)| *f == family) {
                    entry.1 = "owner".to_string();
                }
            }
            FakeResponse::json(
                200,
                &LeftAnswer {
                    new_owner_user_id: heir,
                },
            )
        }),
    );
    {
        let world = Rc::clone(&world);
        fake.route(
            "PUT",
            "/api/v1/me/birthday",
            Rc::new(move |fake, request| {
                let me = request.user.unwrap_or_default();
                let body = request.json();
                let day = BirthdayDto {
                    month: i32::try_from(body["month"].as_i64().unwrap_or(0)).unwrap_or(0),
                    day: i32::try_from(body["day"].as_i64().unwrap_or(0)).unwrap_or(0),
                };
                world.borrow_mut().birthdays.insert(me, day);
                let state = fake.state();
                FakeResponse::json(
                    200,
                    &UserResponse {
                        user: user_dto(&state, &world.borrow(), me),
                    },
                )
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "DELETE",
            "/api/v1/me/birthday",
            Rc::new(move |_, request| {
                world
                    .borrow_mut()
                    .birthdays
                    .remove(&request.user.unwrap_or_default());
                FakeResponse::empty(204)
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "PUT",
            "/api/v1/families/*/members/*/birthday",
            Rc::new(move |fake, request| {
                let user = segment_id(request, 5).unwrap_or_default();
                let body = request.json();
                let day = BirthdayDto {
                    month: i32::try_from(body["month"].as_i64().unwrap_or(0)).unwrap_or(0),
                    day: i32::try_from(body["day"].as_i64().unwrap_or(0)).unwrap_or(0),
                };
                world.borrow_mut().birthdays.insert(user, day);
                let family = path_family(fake, request).unwrap_or_default();
                let state = fake.state();
                let member = members_of(&state, &world.borrow(), family)
                    .into_iter()
                    .find(|member| member.id == user)
                    .unwrap_or_default();
                FakeResponse::json(200, &MemberResponse { member })
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "POST",
            "/api/v1/chats/groups",
            Rc::new(move |_, request| {
                let body = request.json();
                let me = request.user.unwrap_or_default();
                let mut ids = vec![me];
                ids.extend(
                    body["member_ids"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(serde_json::Value::as_i64),
                );
                let mut world = world.borrow_mut();
                let chat = ChatDto {
                    member_ids: Some(ids),
                    admin_user_id: Some(me),
                    family_ids: body["family_id"].as_i64().map(|f| vec![f]),
                    ..ChatDto::new(
                        world.next_id(),
                        "group",
                        body["title"].as_str().unwrap_or(""),
                    )
                };
                world.groups.push(chat.clone());
                FakeResponse::json(201, &ChatResponse { chat })
            }),
        );
    }
    {
        let world = Rc::clone(&world);
        fake.route(
            "POST",
            "/api/v1/chats/direct",
            Rc::new(move |_, request| {
                let peer = request.json()["user_id"].as_i64().unwrap_or_default();
                let chat = ChatDto {
                    peer_user_id: Some(peer),
                    ..ChatDto::new(world.borrow_mut().next_id(), "direct", "")
                };
                FakeResponse::json(200, &ChatResponse { chat })
            }),
        );
    }
    fake.route(
        "GET",
        "/api/v1/families/*/stats",
        Rc::new(|fake, request| {
            let Some(family) = path_family(fake, request) else {
                return FakeResponse::error(404, "family_not_found");
            };
            let state = fake.state();
            let members: Vec<serde_json::Value> = state
                .users
                .iter()
                .filter(|u| u.families.iter().any(|(f, _)| *f == family))
                .map(|u| serde_json::json!({"user_id": u.id, "display_name": u.display_name, "messages": 3}))
                .collect();
            let mut stats = serde_json::json!({
                "totals": {"members": members.len(), "messages": 3 * members.len(), "board_notes": 2,
                           "attachments": {"count": 4, "photo": 3, "video": 1, "audio": 0, "file": 0,
                                           "location": 0, "bytes": 2_500_000, "stored_bytes": 1_500_000}},
                "members": members
            });
            // A paid family's allowance (docs/protocol.md, "Family statistics", 2026-10-30).
            if let Some(allowance) = state.allowances.get(&family).filter(|_| state.subscriptions) {
                stats["allowance"] = serde_json::to_value(allowance).unwrap_or_default();
            }
            FakeResponse::json(200, &stats)
        }),
    );
    install_board(fake, &world);
}

fn install_board(fake: &Rc<FakeServer>, world: &Rc<RefCell<World>>) {
    {
        let world = Rc::clone(world);
        fake.route(
            "GET",
            "/api/v1/families/*/board",
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let world = world.borrow();
                let notes = world
                    .notes
                    .iter()
                    .filter(|note| note.family_id == Some(family) && !note.deleted)
                    .cloned()
                    .collect();
                FakeResponse::json(
                    200,
                    &BoardResponse {
                        notes: Some(notes),
                        max_board_seq: world.seq,
                    },
                )
            }),
        );
    }
    {
        let world = Rc::clone(world);
        fake.route(
            "GET",
            "/api/v1/families/*/board/changes",
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let after = request
                    .query
                    .as_deref()
                    .and_then(|q| {
                        q.split('&')
                            .find_map(|pair| pair.strip_prefix("after_seq="))
                            .and_then(|v| v.parse::<i64>().ok())
                    })
                    .unwrap_or(0);
                let notes: Vec<NoteDto> = world
                    .borrow()
                    .notes
                    .iter()
                    .filter(|note| note.family_id == Some(family) && note.board_seq > after)
                    .cloned()
                    .collect();
                FakeResponse::json(200, &serde_json::json!({ "notes": notes }))
            }),
        );
    }
    {
        let world = Rc::clone(world);
        fake.route(
            "POST",
            "/api/v1/families/*/board/notes",
            Rc::new(move |fake, request| {
                let Some(family) = path_family(fake, request) else {
                    return FakeResponse::error(404, "family_not_found");
                };
                let body: fc_core::protocol::NoteRequest =
                    serde_json::from_slice(&request.body).unwrap_or_default();
                let mut world = world.borrow_mut();
                let items = body.items.map(|items| {
                    items
                        .into_iter()
                        .enumerate()
                        .map(|(at, line)| TaskItemDto {
                            id: i64::try_from(at).unwrap_or(0) + 1,
                            text: line.text,
                            ..TaskItemDto::default()
                        })
                        .collect()
                });
                let note = NoteDto {
                    kind: body.kind,
                    text: Some(body.text),
                    color: Some(body.color),
                    size: body.size,
                    font: body.font,
                    x: body.x,
                    y: body.y,
                    starts_at: body.starts_at,
                    ends_at: body.ends_at,
                    place: body.place,
                    items,
                    ..NoteDto::default()
                };
                let note = world.add_note(family, request.user.unwrap_or_default(), note);
                FakeResponse::json(201, &NoteResponse { note })
            }),
        );
    }
    {
        let world = Rc::clone(world);
        fake.route(
            "PATCH",
            "/api/v1/families/*/board/notes/*",
            Rc::new(move |_, request| {
                let id = segment_id(request, 6).unwrap_or_default();
                let body = request.json();
                let mut world = world.borrow_mut();
                let seq = world.next_seq();
                let Some(note) = world.notes.iter_mut().find(|note| note.id == id) else {
                    return FakeResponse::error(404, "note_not_found");
                };
                if note.author_id != request.user.unwrap_or_default()
                    && (body.get("text").is_some() || body.get("color").is_some())
                {
                    return FakeResponse::error(403, "not_note_author");
                }
                if let Some(x) = body["x"].as_f64() {
                    note.x = x;
                }
                if let Some(y) = body["y"].as_f64() {
                    note.y = y;
                }
                if let Some(text) = body["text"].as_str() {
                    note.text = Some(text.to_string());
                    note.content_seq = Some(seq);
                }
                if let Some(color) = body["color"].as_str() {
                    note.color = Some(color.to_string());
                }
                if let Some(size) = body["size"].as_str() {
                    note.size = Some(size.to_string());
                }
                if let Some(place) = body["place"].as_str() {
                    note.place = Some(place.to_string());
                }
                note.board_seq = seq;
                FakeResponse::json(200, &NoteResponse { note: note.clone() })
            }),
        );
    }
    {
        let world = Rc::clone(world);
        fake.route(
            "DELETE",
            "/api/v1/families/*/board/notes/*",
            Rc::new(move |_, request| {
                let id = segment_id(request, 6).unwrap_or_default();
                let mut world = world.borrow_mut();
                let seq = world.next_seq();
                if let Some(note) = world.notes.iter_mut().find(|note| note.id == id) {
                    note.deleted = true;
                    note.board_seq = seq;
                }
                FakeResponse::empty(204)
            }),
        );
    }
    {
        let world = Rc::clone(world);
        fake.route(
            "PUT",
            "/api/v1/families/*/board/notes/*/rsvp",
            Rc::new(move |_, request| {
                let id = segment_id(request, 6).unwrap_or_default();
                let me = request.user.unwrap_or_default();
                let answer = request.json()["answer"].as_str().unwrap_or("").to_string();
                let mut world = world.borrow_mut();
                let seq = world.next_seq();
                let Some(note) = world.notes.iter_mut().find(|note| note.id == id) else {
                    return FakeResponse::error(404, "note_not_found");
                };
                let mut rsvps = note.rsvps.clone().unwrap_or_default();
                rsvps.retain(|rsvp| rsvp.user_id != me);
                rsvps.push(RsvpDto {
                    user_id: me,
                    answer,
                });
                note.rsvps = Some(rsvps);
                note.board_seq = seq;
                FakeResponse::json(200, &NoteResponse { note: note.clone() })
            }),
        );
    }
}

// ---- signing in as somebody of the world ----------------------------------------------------------

/// The user called `username`, made (with password `password`) when the fake does not know them.
pub fn person(fake: &FakeServer, username: &str, display: &str) -> i64 {
    let known = fake
        .state()
        .users
        .iter()
        .find(|user| user.username == username)
        .map(|user| user.id);
    known.unwrap_or_else(|| fake.add_user(username, "password", display))
}

/// Points the app at the fake server (when it is not already) and signs in as `username`, waiting
/// for the chats. Answers the connection.
pub async fn sign_in_as(h: &Harness, username: &str) -> Option<Rc<Connection>> {
    let shell = h.shell();
    let fake = h.fake_server();
    install(&fake);
    let server = ServerUrl::normalise(Some(&fake.url()))?;
    let here = shell
        .connection()
        .is_some_and(|connection| connection.server.to_string() == server.to_string());
    if !here {
        shell.open_server(server.clone());
        h.wait_until(WAIT, || {
            shell.connection().is_some() && !shell.gate_facts().booting
        })
        .await;
    }
    let connection = shell.connection()?;
    let who = connection.session.state().me.map(|me| me.username);
    if who.as_deref() != Some(username) {
        if who.is_some() {
            let _ = connection.session.sign_out().await;
        }
        let answer = connection.session.sign_in(username, "password").await;
        if !matches!(answer, Ok(None)) {
            return None;
        }
    }
    let main = shell.main_window()?;
    h.wait_until(WAIT, || main.page() == GatePage::Chats).await;
    // The first resync reads the rosters and the walls.
    connection.live.refresh();
    h.sleep(Duration::from_millis(600)).await;
    Some(connection)
}

// ---- finding widgets ----------------------------------------------------------------------------

/// Every widget under `root`, depth first.
pub fn descendants(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut pending: Vec<gtk::Widget> = vec![root.clone().upcast()];
    while let Some(widget) = pending.pop() {
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            pending.push(next.clone());
            found.push(next);
        }
    }
    found
}

/// The first `T` under `root` that `wanted` accepts.
pub fn find<T: IsA<gtk::Widget>>(
    root: &impl IsA<gtk::Widget>,
    wanted: impl Fn(&T) -> bool,
) -> Option<T> {
    descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<T>().ok())
        .find(|widget| wanted(widget))
}

/// A button by its label under `root`.
pub fn button(root: &impl IsA<gtk::Widget>, label: &str) -> Option<gtk::Button> {
    find::<gtk::Button>(root, |b| b.label().as_deref() == Some(label))
}

// ---- stand-ins for P5's services ------------------------------------------------------------------

#[derive(Default)]
struct StandInTray {
    available: Cell<bool>,
    changed: Event<bool>,
}

impl TrayState for StandInTray {
    fn available(&self) -> bool {
        self.available.get()
    }
    fn on_available_changed(&self) -> &Event<bool> {
        &self.changed
    }
    fn set_unread(&self, _unread: i64, _title: &str) {}
}

struct StandInHotKey {
    state: RefCell<PortalState>,
    changed: Event<()>,
    applied: RefCell<Vec<bool>>,
    /// "Change…" asked the desktop's dialog (`configure`), with a parent window or not.
    configured: RefCell<Vec<bool>>,
}

impl HotKeyState for StandInHotKey {
    fn state(&self) -> PortalState {
        self.state.borrow().clone()
    }
    fn on_state_changed(&self) -> &Event<()> {
        &self.changed
    }
    fn apply(&self, enabled: bool) {
        self.applied.borrow_mut().push(enabled);
    }
    fn configure(&self, parent: Option<&gtk::Window>) {
        self.configured.borrow_mut().push(parent.is_some());
    }
    fn can_configure(&self) -> bool {
        matches!(*self.state.borrow(), PortalState::Bound(_))
    }
}

struct StandInAutostart {
    state: Cell<StartupState>,
    asked: RefCell<Vec<bool>>,
}

impl AutostartState for StandInAutostart {
    fn state(&self) -> StartupState {
        self.state.get()
    }
    fn set_enabled(&self, enabled: bool) -> StartupState {
        self.asked.borrow_mut().push(enabled);
        if matches!(
            self.state.get(),
            StartupState::Disabled | StartupState::Enabled
        ) {
            self.state.set(if enabled {
                StartupState::Enabled
            } else {
                StartupState::Disabled
            });
        }
        self.state.get()
    }
}

#[derive(Default)]
struct StandInCamera {
    changed: Event<()>,
}

impl Camera for StandInCamera {
    fn present(&self) -> bool {
        true
    }
    fn devices(&self) -> Vec<CameraDevice> {
        vec![
            CameraDevice {
                id: "pw-41".into(),
                name: "Integrated Camera".into(),
            },
            CameraDevice {
                id: "/dev/video2".into(),
                name: "USB Camera".into(),
            },
        ]
    }
    fn on_devices_changed(&self) -> &Event<()> {
        &self.changed
    }
    fn open(
        &self,
        _device: Option<String>,
    ) -> fc_core::rt::LocalBoxFuture<'static, Result<gst::Element, CameraRefusal>> {
        Box::pin(std::future::ready(Err(CameraRefusal::Absent)))
    }
}

// ---- the group ------------------------------------------------------------------------------------

pub async fn run(h: &Harness) {
    let shell = h.shell();
    let fake = h.fake_server();
    install(&fake);
    let dana = person(&fake, "dana", "Dana");
    let erin = person(&fake, "erin", "Erin");
    let family = fake.add_family("Dana's Family", dana);
    fake.join(erin, family, "member");

    // P5's services are stand-ins here, installed before the window first reads them.
    let tray = Rc::new(StandInTray::default());
    let hot_key = Rc::new(StandInHotKey {
        state: RefCell::new(PortalState::Bound("Shift+Ctrl+Alt+F".into())),
        changed: Event::new(),
        applied: RefCell::new(Vec::new()),
        configured: RefCell::new(Vec::new()),
    });
    let autostart = Rc::new(StandInAutostart {
        state: Cell::new(StartupState::Disabled),
        asked: RefCell::new(Vec::new()),
    });
    let (old_tray, old_hot_key, old_autostart, old_camera) = (
        shell.tray.get(),
        shell.hot_key.get(),
        shell.autostart.get(),
        shell.camera.get(),
    );
    shell.tray.install(tray.clone());
    shell.hot_key.install(hot_key.clone());
    shell.autostart.install(autostart.clone());
    shell.camera.install(Rc::new(StandInCamera::default()));
    // S0 calls `apply` for every change of the setting; re-wire it to the stand-in.
    {
        let hot_key = Rc::clone(&hot_key);
        shell.settings.changed().subscribe(move |key: &str| {
            if key == keys::HOT_KEY {
                hot_key.apply(Shell::get().settings.flag(keys::HOT_KEY));
            }
        });
    }

    let Some(connection) = sign_in_as(h, "erin").await else {
        h.check("signed_in", false, "erin could not sign in");
        return;
    };
    h.check("signed_in", true, "erin, a member of Dana's Family");

    // ---- the window ------------------------------------------------------------------------------
    let main = shell.main_window().expect("the main window");
    let mut opened_by_key = false;
    if h.input() {
        h.focus(&main.window().clone().upcast()).await;
        h.key("ctrl+comma").await;
        opened_by_key = h
            .wait_until(Duration::from_secs(5), || {
                settings_window::current().is_some_and(|open| open.window.is_visible())
            })
            .await;
    }
    if !opened_by_key {
        shell
            .app
            .activate_action(crate::seams::actions::SHOW_SETTINGS.name, None);
    }
    let Some(open) = settings_window::current() else {
        h.check("window_opens", false, "no Settings window");
        return;
    };
    h.wait_until(Duration::from_secs(5), || open.window.is_visible())
        .await;
    h.check(
        "window_opens",
        open.window.is_visible() && open.window.default_size() == settings_window::DEFAULT_SIZE,
        format!(
            "by Ctrl+, {opened_by_key}; size {:?}",
            open.window.default_size()
        ),
    );
    h.check(
        "signed_in_page",
        open.stack.visible_child_name().as_deref() == Some("settings"),
        format!("{:?}", open.stack.visible_child_name()),
    );
    h.screenshot("window").await;

    // ---- the profile photo: Add → P2's `prepare_upload` (within 512 px, a JPEG) → `PUT /me/avatar`,
    // the new version read back; Remove → `DELETE /me/avatar` ------------------------------------------
    let uploaded: Rc<RefCell<Vec<u8>>> = Rc::default();
    {
        let uploaded = Rc::clone(&uploaded);
        fake.route(
            "PUT",
            "/api/v1/me/avatar",
            Rc::new(move |fake, request| {
                *uploaded.borrow_mut() = request.body.clone();
                let user = request.user.unwrap_or_default();
                let mut state = fake.state();
                let Some(found) = state.users.iter_mut().find(|u| u.id == user) else {
                    return FakeResponse::error(404, "user_not_found");
                };
                found.avatar_version += 1;
                let dto = fc_core::protocol::UserDto {
                    avatar_version: found.avatar_version,
                    ..fc_core::protocol::UserDto::new(
                        found.id,
                        &found.username,
                        &found.display_name,
                    )
                };
                FakeResponse::json(200, &fc_core::protocol::UserResponse { user: dto })
            }),
        );
    }
    fake.route(
        "DELETE",
        "/api/v1/me/avatar",
        Rc::new(|fake, request| {
            let user = request.user.unwrap_or_default();
            if let Some(found) = fake.state().users.iter_mut().find(|u| u.id == user) {
                found.avatar_version = 0;
            }
            FakeResponse::empty(204)
        }),
    );
    let say = Rc::clone(&shell.say);
    let label = |open: &settings_window::SettingsWindow| open.photo.label().map(|l| l.to_string());
    let starts =
        label(&open).as_deref() == Some(say.get("Add Photo")) && !open.remove_photo.is_visible();
    let big = {
        let pixbuf =
            gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 1600, 1200)
                .expect("a pixbuf");
        pixbuf.fill(0x3949_abff);
        pixbuf.save_to_bufferv("jpeg", &[]).unwrap_or_default()
    };
    settings_window::upload_photo(big).await;
    let set = h
        .wait_until(WAIT, || {
            label(&open).as_deref() == Some(say.get("Change Photo"))
                && open.remove_photo.is_visible()
        })
        .await;
    let sent_size = {
        let bytes = uploaded.borrow().clone();
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes.clone()));
        gtk::gdk_pixbuf::Pixbuf::from_stream(&stream, None::<&gio::Cancellable>)
            .ok()
            .map(|p| (p.width(), p.height(), bytes.starts_with(&[0xFF, 0xD8])))
    };
    h.screenshot("profile-photo").await;
    open.remove_photo();
    let removed = h
        .wait_until(WAIT, || {
            label(&open).as_deref() == Some(say.get("Add Photo")) && !open.remove_photo.is_visible()
        })
        .await;
    h.check(
        "profile_photo_added_and_removed",
        starts && set && sent_size == Some((512, 384, true)) && removed,
        format!(
            "Add Photo at first {starts}; after the upload Change Photo + Remove {set}, sent {sent_size:?} (want 512 × 384 JPEG); removed {removed}"
        ),
    );

    // ---- every device row reads and writes its key ---------------------------------------------------
    let settings = &shell.settings;
    let mut wrong = Vec::new();
    for (row, key) in [
        (&open.notify, keys::NOTIFY),
        (&open.link_previews, keys::LINK_PREVIEWS),
        (&open.map_previews, keys::MAP_PREVIEWS),
    ] {
        let before = settings.flag(key);
        row.set_active(!before);
        if settings.flag(key) == before {
            wrong.push(format!("{key}: the row did not write"));
        }
        settings.set_bool(key, before);
        if row.is_active() != before {
            wrong.push(format!("{key}: the row did not read"));
        }
    }
    h.check(
        "switch_rows_read_and_write",
        wrong.is_empty(),
        format!("{wrong:?}"),
    );

    // Keep running: no icon → off, insensitive, says why, and the group's "stays in the top bar"
    // note is hidden; an icon → the setting, and the note shows.
    let no_icon = !open.keep_running.is_active()
        && !open.keep_running.is_sensitive()
        && open
            .keep_running
            .subtitle()
            .is_some_and(|s| s.contains("quits"))
        && !open.top_bar_note.is_visible();
    tray.available.set(true);
    tray.changed.emit(&true);
    let with_icon = open.keep_running.is_active()
        && open.keep_running.is_sensitive()
        && open.top_bar_note.is_visible();
    open.keep_running.set_active(false);
    let wrote = !settings.flag(keys::KEEP_RUNNING);
    settings.set_bool(keys::KEEP_RUNNING, true);
    h.check(
        "keep_running_row",
        no_icon && with_icon && wrote && open.keep_running.is_active(),
        format!("no icon {no_icon}, with icon {with_icon}, wrote {wrote}"),
    );
    tray.available.set(false);
    tray.changed.emit(&false);

    // The hot key: the trigger GNOME bound, with "Change…" (the desktop's own dialog, over this
    // window); NotBound; NoPortal with the --toggle line.
    let bound_title = open.hot_key.title().to_string();
    let change_offered = open.hot_key_change.is_visible();
    open.hot_key_change.emit_clicked();
    let change_asked = hot_key.configured.borrow().as_slice() == [true];
    open.hot_key.set_active(false);
    let off_written =
        !settings.flag(keys::HOT_KEY) && hot_key.applied.borrow().last() == Some(&false);
    open.hot_key.set_active(true);
    *hot_key.state.borrow_mut() = PortalState::NotBound;
    hot_key.changed.emit(&());
    let not_bound = open
        .hot_key
        .subtitle()
        .is_some_and(|s| s.contains("didn't set"));
    *hot_key.state.borrow_mut() = PortalState::NoPortal;
    hot_key.changed.emit(&());
    let no_portal = !open.hot_key.is_sensitive()
        && open
            .hot_key
            .subtitle()
            .is_some_and(|s| s.contains("family-connect --toggle"));
    let change_hidden = !open.hot_key_change.is_visible();
    h.screenshot("no-portal").await;
    h.check(
        "hot_key_change_button",
        change_offered && change_asked && change_hidden,
        format!(
            "shown while bound {change_offered}, pressed → configure(window) {change_asked}, hidden with no portal {change_hidden}"
        ),
    );
    h.check(
        "hot_key_row_states",
        bound_title == "Open with Shift+Ctrl+Alt+F" && off_written && not_bound && no_portal,
        format!(
            "title {bound_title:?}, off {off_written}, not bound {not_bound}, no portal {no_portal}"
        ),
    );
    *hot_key.state.borrow_mut() = PortalState::Bound("Shift+Ctrl+Alt+F".into());
    hot_key.changed.emit(&());

    // Start when I log in: Disabled → on asks for it; DisabledByUser; Unavailable hides the row.
    open.refresh();
    let offered =
        open.autostart.is_visible() && !open.autostart.is_active() && open.autostart.is_sensitive();
    open.autostart.set_active(true);
    let enabled = autostart.asked.borrow().as_slice() == [true]
        && autostart.state.get() == StartupState::Enabled
        && open.autostart.is_active();
    autostart.state.set(StartupState::DisabledByUser);
    open.refresh();
    let by_user = !open.autostart.is_active()
        && !open.autostart.is_sensitive()
        && open
            .autostart
            .subtitle()
            .is_some_and(|s| s.contains("startup settings"));
    autostart.state.set(StartupState::Unavailable);
    open.refresh();
    let hidden = !open.autostart.is_visible();
    h.check(
        "autostart_row_states",
        offered && enabled && by_user && hidden,
        format!(
            "offered {offered}, enabled {enabled}, by user {by_user}, unavailable hides {hidden}"
        ),
    );
    autostart.state.set(StartupState::Disabled);
    open.refresh();

    // Voice speed and camera.
    open.speed.set_active_name(Some("1.5"));
    let speed_written = settings.string(keys::VOICE_SPEED).as_deref() == Some("1.5");
    settings.set_string(keys::VOICE_SPEED, "2");
    let speed_read = open.speed.active_name().as_deref() == Some("2");
    let cameras = open.camera.model().map_or(0, |model| model.n_items());
    open.camera.set_selected(2);
    let camera_written = settings.string(keys::VIDEO_CAMERA).as_deref() == Some("/dev/video2");
    open.camera.set_selected(0);
    let automatic = settings.string(keys::VIDEO_CAMERA).is_none();
    h.check(
        "voice_and_camera_rows",
        speed_written && speed_read && cameras == 3 && camera_written && automatic,
        format!("speed {speed_written}/{speed_read}, {cameras} cameras, chosen {camera_written}, automatic {automatic}"),
    );

    // ---- New Family…: the three cases -------------------------------------------------------------
    let refresh = || async {
        let _ = connection.session.refresh().await;
        h.sleep(Duration::from_millis(200)).await;
    };
    refresh().await;
    let member_allowed = open.new_family.is_visible();
    world().borrow_mut().member_families = false;
    refresh().await;
    let server_closed = !open.new_family.is_visible();
    h.screenshot("no-new-family").await;
    world().borrow_mut().member_families = true;
    let own = fake.add_family("Erin's Own", erin);
    refresh().await;
    let owns_one = !open.new_family.is_visible();
    h.check(
        "new_family_three_cases",
        member_allowed && server_closed && owns_one,
        format!(
            "member allowed {member_allowed}, server closed {server_closed}, owns one {owns_one}"
        ),
    );
    // Back to a member of one family.
    if let Some(user) = fake.state().users.iter_mut().find(|u| u.id == erin) {
        user.families.retain(|(f, _)| *f != own);
    }
    refresh().await;

    // ---- the profile's birthday, the keyring line ------------------------------------------------------
    let saved = connection
        .api
        .set_my_birthday(3, 12)
        .await
        .into_result()
        .is_ok();
    refresh().await;
    let shown = open
        .birthday
        .subtitle()
        .map(|s| s.to_string())
        .unwrap_or_default();
    h.check(
        "birthday_row",
        saved && !shown.is_empty() && shown != "Not set",
        format!("shown {shown:?}"),
    );
    // The line follows what the vault did: P5's Secret Service keeps the sign-in (no line); with
    // no keyring, or S0's memory vault, it says the sign-in isn't remembered.
    let remembered = shell
        .connection()
        .is_some_and(|connection| connection.tokens.remembered());
    let line_right = if remembered {
        !open.remembered.is_visible()
    } else {
        open.remembered.is_visible() && open.remembered.text().contains("keyring")
    };
    h.check(
        "keyring_line",
        line_right,
        format!(
            "remembered {remembered}, line shown {}",
            open.remembered.is_visible()
        ),
    );

    // ---- the lower groups, scrolled to --------------------------------------------------------------
    // The page's own scroller (the signed-out page has one too).
    let page = open.stack.child_by_name("settings");
    if let Some(scroller) = page
        .as_ref()
        .and_then(|page| find::<gtk::ScrolledWindow>(page, |_| true))
    {
        let adjustment = scroller.vadjustment();
        let top_bar = open
            .keep_running
            .compute_point(&scroller, &gtk::graphene::Point::new(0.0, 0.0))
            .map_or(600.0, |p| f64::from(p.y()) + adjustment.value() - 120.0);
        adjustment.set_value(top_bar);
        h.screenshot("top-bar").await;
        adjustment.set_value(adjustment.upper());
        h.screenshot("server").await;
        adjustment.set_value(0.0);
    }

    // ---- dark and narrow ---------------------------------------------------------------------------
    let styles = adw::StyleManager::default();
    styles.set_color_scheme(adw::ColorScheme::ForceDark);
    h.sleep(Duration::from_millis(300)).await;
    h.screenshot("dark").await;
    styles.set_color_scheme(adw::ColorScheme::Default);
    if h.input() {
        h.resize(&open.window.clone().upcast(), 360, 520).await;
        h.screenshot("narrow").await;
        h.resize(
            &open.window.clone().upcast(),
            settings_window::DEFAULT_SIZE.0,
            settings_window::DEFAULT_SIZE.1,
        )
        .await;
    }

    // ---- Log Out asks, then the gate ------------------------------------------------------------------
    open.ask_log_out();
    h.sleep(Duration::from_millis(300)).await;
    let asked = open
        .window
        .visible_dialog()
        .and_downcast::<adw::AlertDialog>();
    let question = asked
        .as_ref()
        .map(|alert| alert.heading().map(|t| t.to_string()));
    h.screenshot("log-out").await;
    if let Some(alert) = &asked {
        // What a click on its button does: the response, and the alert goes.
        alert.emit_by_name::<()>("response", &[&crate::settings::ui::CONFIRM]);
        alert.close();
    }
    let gate = h
        .wait_until(WAIT, || {
            main.page() == GatePage::SignIn
                && open.stack.visible_child_name().as_deref() == Some("signed-out")
        })
        .await;
    h.check(
        "log_out_returns_to_the_gate",
        asked.is_some() && gate,
        format!("asked {question:?}, page {:?}", main.page()),
    );
    h.screenshot("signed-out").await;
    open.window.close();

    shell.tray.install(old_tray);
    shell.hot_key.install(old_hot_key);
    shell.autostart.install(old_autostart);
    shell.camera.install(old_camera);
    let unhandled = fake.unhandled();
    h.check(
        "fake_server_knew_every_request",
        unhandled.is_empty(),
        format!("{unhandled:?}"),
    );
}
