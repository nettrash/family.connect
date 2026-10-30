//! The `chat` group (P1, shell-design §27): through the gate against the fake server — the Server
//! page's address, the Sign In page (a family-less account first: the Family page's forms), then the
//! chats — and the conversation: list rows, opening one, a live `message` frame, Enter sends and
//! Shift+Enter makes a line (real XTEST keys), capitals and emoji typed, a reaction, a reply, an
//! edit, a refused send given up (Delete), a thread and its reply, a poll vote, the typing line, a
//! draft kept across chats, a mention from the `@` list, the conversation window, the narrow
//! layout, dark mode, the list by keyboard alone, `win.paste-attachment` following the field's
//! focus, and a drop (staged programmatically — Xvfb has no drag source).
//!
//! The group adds its own people and routes to the run's fake server (later routes win) and
//! answers the socket's `send` frames itself — an `ack` with the message, or an `error` when told
//! to refuse the next one — by watching `FakeServer::frames_received`.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::{
    ChatDto, ChatRowDto, ChatsResponse, MentionDto, MessageDto, MessagePollDto,
    MessageReactionsDto, MessageResponse, MessagesResponse, PollDto, PollOptionDto, ReactionDto,
    ReplyToDto, ServerUrl,
};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::conversation_rows;

use super::Harness;
use super::fake_server::{FakeRequest, FakeResponse, FakeServer};
use crate::chats;
use crate::shell::Shell;
use crate::windows::conversation_window;
use crate::windows::main_window::{DEFAULT_SIZE, GatePage, MainWindow};

/// The integration pass's checks, run after the conversation and the layout (§W.2's "P2/P3 into
/// P1's slots, P6 onto P3/P5").
mod wiring;

const WAIT: Duration = Duration::from_secs(15);
const SHORT: Duration = Duration::from_secs(5);

/// The group's own people, chats and messages on the fake server.
struct World {
    fake: Rc<FakeServer>,
    dora: i64,
    eve: i64,
    finn: i64,
    family_chat: i64,
    direct: i64,
    /// The integration pass's chats (listed once `wiring` is on): a direct chat with Finn that
    /// carries every kind of attachment, and a group.
    media_chat: i64,
    group_chat: i64,
    wiring: Rc<Cell<bool>>,
    /// What the clients uploaded (`POST /attachments`), by the id the fake gave it.
    uploads: Rc<RefCell<BTreeMap<i64, fc_core::protocol::AttachmentDto>>>,
    messages: Rc<RefCell<BTreeMap<i64, MessageDto>>>,
    next: Rc<Cell<i64>>,
    reads: Rc<RefCell<HashMap<(i64, i64), i64>>>,
    refuse_next: Rc<Cell<bool>>,
    sends: Rc<RefCell<Vec<serde_json::Value>>>,
    seen_frames: Rc<Cell<usize>>,
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

impl World {
    fn new(fake: &Rc<FakeServer>) -> World {
        let dora = fake.add_user("dora", "password", "Dora");
        let eve = fake.add_user("eve", "password", "Eve");
        let finn = fake.add_user("finn", "password", "Finn");
        let family = fake.add_family("The Chatters", dora);
        fake.join(eve, family, "member");
        fake.join(finn, family, "member");
        let world = World {
            fake: Rc::clone(fake),
            dora,
            eve,
            finn,
            family_chat: 900_001,
            direct: 900_002,
            media_chat: 900_003,
            group_chat: 900_004,
            wiring: Rc::new(Cell::new(false)),
            uploads: Rc::new(RefCell::new(BTreeMap::new())),
            messages: Rc::new(RefCell::new(BTreeMap::new())),
            next: Rc::new(Cell::new(5000)),
            reads: Rc::new(RefCell::new(HashMap::new())),
            refuse_next: Rc::new(Cell::new(false)),
            sends: Rc::new(RefCell::new(Vec::new())),
            seen_frames: Rc::new(Cell::new(0)),
        };
        let _ = family;
        world.seed();
        world.routes();
        world
    }

    fn chat_of(&self, id: i64) -> Option<ChatDto> {
        let family = self.family_id();
        if id == self.family_chat {
            return Some(ChatDto {
                family_ids: Some(vec![family]),
                ..ChatDto::new(id, "family", "The Chatters")
            });
        }
        if id == self.direct {
            return Some(ChatDto {
                peer_user_id: Some(self.eve),
                member_ids: Some(vec![self.dora, self.eve]),
                family_ids: Some(vec![family]),
                ..ChatDto::new(id, "direct", "Eve")
            });
        }
        if id == self.media_chat {
            return Some(ChatDto {
                peer_user_id: Some(self.finn),
                member_ids: Some(vec![self.dora, self.finn]),
                family_ids: Some(vec![family]),
                ..ChatDto::new(id, "direct", "Finn")
            });
        }
        if id == self.group_chat {
            return Some(ChatDto {
                member_ids: Some(vec![self.dora, self.eve, self.finn]),
                admin_user_id: Some(self.dora),
                family_ids: Some(vec![family]),
                ..ChatDto::new(id, "group", "Weekend")
            });
        }
        None
    }

    fn family_id(&self) -> i64 {
        self.fake
            .state()
            .user(self.dora)
            .and_then(|user| user.families.first().map(|(id, _)| *id))
            .unwrap_or_default()
    }

    fn seed(&self) {
        let poll = PollDto {
            poll_seq: 1,
            closed: false,
            options: vec![
                PollOptionDto {
                    id: 1,
                    text: "Pizza".into(),
                    votes: vec![self.eve],
                },
                PollOptionDto {
                    id: 2,
                    text: "Pasta".into(),
                    votes: vec![],
                },
            ],
        };
        let root = self.insert(MessageDto::new(
            0,
            self.family_chat,
            self.eve,
            None,
            "Who is coming on **Sunday**? See https://example.com/plan",
            &now_rfc3339(),
        ));
        let mut reply =
            MessageDto::new(0, self.family_chat, self.finn, None, "Me!", &now_rfc3339());
        reply.reply_to = Some(ReplyToDto {
            message_id: root,
            sender_id: self.eve,
            excerpt: "Who is coming on Sunday?".into(),
            parent: None,
        });
        reply.thread_root_id = Some(root);
        self.insert(reply);
        if let Some(held) = self.messages.borrow_mut().get_mut(&root) {
            held.reply_count = Some(1);
        }
        let mut question = MessageDto::new(
            0,
            self.family_chat,
            self.eve,
            None,
            "Dinner?",
            &now_rfc3339(),
        );
        question.poll = Some(poll);
        self.insert(question);
        self.insert(MessageDto::new(
            0,
            self.direct,
            self.eve,
            None,
            "Привет, Dora 😀",
            &now_rfc3339(),
        ));
    }

    fn insert(&self, mut message: MessageDto) -> i64 {
        let id = self.next.get() + 1;
        self.next.set(id);
        message.id = id;
        self.messages.borrow_mut().insert(id, message);
        id
    }

    /// A message made from a client's send (frame or REST body).
    fn create(&self, sender: i64, chat_id: i64, body: &serde_json::Value) -> MessageDto {
        let mut message = MessageDto::new(
            0,
            chat_id,
            sender,
            body["client_msg_id"].as_str(),
            body["body"].as_str().unwrap_or_default(),
            &now_rfc3339(),
        );
        // What it carries: the uploads it names, as a sticker or a video message when it says so.
        if let Some(ids) = body["attachment_ids"].as_array() {
            let uploads = self.uploads.borrow();
            let carried: Vec<fc_core::protocol::AttachmentDto> = ids
                .iter()
                .filter_map(|id| id.as_i64().and_then(|id| uploads.get(&id).cloned()))
                .map(|mut attachment| {
                    attachment.sticker = body["sticker"].as_bool().unwrap_or(false);
                    attachment.round = body["round"].as_bool().unwrap_or(false);
                    attachment
                })
                .collect();
            if !carried.is_empty() {
                message.attachments = Some(carried);
            }
        }
        if let Some(reply_to) = body["reply_to_message_id"].as_i64() {
            let quoted = self.messages.borrow().get(&reply_to).cloned();
            if let Some(quoted) = quoted {
                let root = quoted.thread_root_id.unwrap_or(quoted.id);
                message.reply_to = Some(ReplyToDto {
                    message_id: quoted.id,
                    sender_id: quoted.sender_id,
                    excerpt: quoted.body.chars().take(80).collect(),
                    parent: None,
                });
                message.thread_root_id = Some(root);
                if let Some(held) = self.messages.borrow_mut().get_mut(&root) {
                    held.reply_count = Some(held.reply_count.unwrap_or(0) + 1);
                }
            }
        }
        if let Some(mentions) = body["mentions"].as_array() {
            message.mentions = Some(
                mentions
                    .iter()
                    .map(|m| {
                        MentionDto::new(
                            m["user_id"].as_i64().unwrap_or_default(),
                            m["name"].as_str().unwrap_or_default(),
                        )
                    })
                    .collect(),
            );
        }
        if let Some(options) = body["poll"]["options"].as_array() {
            message.poll = Some(PollDto {
                poll_seq: 1,
                closed: false,
                options: options
                    .iter()
                    .enumerate()
                    .map(|(i, text)| PollOptionDto {
                        id: i64::try_from(i).unwrap_or(0) + 1,
                        text: text.as_str().unwrap_or_default().to_string(),
                        votes: vec![],
                    })
                    .collect(),
            });
        }
        let id = self.insert(message);
        self.messages.borrow().get(&id).cloned().unwrap_or_default()
    }

    /// Answers Dora's send frames (her sockets only — `frames_received_from`): an ack with the
    /// message, or an error when told to, sent to her alone (`send_frame_to`).
    fn answer_frames(&self) {
        let frames = self.fake.frames_received_from(self.dora);
        let from = self.seen_frames.get();
        if frames.len() <= from {
            return;
        }
        self.seen_frames.set(frames.len());
        for frame in &frames[from..] {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(frame) else {
                continue;
            };
            if value["type"] != "send" {
                continue;
            }
            self.sends.borrow_mut().push(value.clone());
            let key = value["client_msg_id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            if self.refuse_next.replace(false) {
                self.fake.send_frame_to(
                    self.dora,
                    &serde_json::json!({
                        "type": "error", "code": "validation", "message": "refused",
                        "client_msg_id": key,
                    })
                    .to_string(),
                );
                continue;
            }
            let chat = value["chat_id"].as_i64().unwrap_or_default();
            let message = self.create(self.dora, chat, &value);
            self.fake.send_frame_to(
                self.dora,
                &serde_json::json!({"type": "ack", "client_msg_id": key, "message": message})
                    .to_string(),
            );
        }
    }

    fn rows_for(&self, user: i64) -> Vec<ChatRowDto> {
        let mut rows = Vec::new();
        let mut listed = vec![self.family_chat, self.direct];
        if self.wiring.get() {
            listed.extend([self.media_chat, self.group_chat]);
        }
        for id in listed {
            if id == self.direct && user != self.dora && user != self.eve {
                continue;
            }
            if id == self.media_chat && user != self.dora && user != self.finn {
                continue;
            }
            let Some(chat) = self.chat_of(id) else {
                continue;
            };
            let messages = self.messages.borrow();
            let last_read = self.reads.borrow().get(&(user, id)).copied().unwrap_or(0);
            let in_chat: Vec<&MessageDto> = messages.values().filter(|m| m.chat_id == id).collect();
            let unread = in_chat
                .iter()
                .filter(|m| m.sender_id != user && m.id > last_read)
                .count();
            rows.push(ChatRowDto {
                last_message: in_chat.last().map(|m| (*m).clone()),
                unread_count: i32::try_from(unread).unwrap_or(0),
                last_read_message_id: last_read,
                ..ChatRowDto::new(chat)
            });
        }
        rows
    }

    fn routes(&self) {
        let fake = &self.fake;
        let (dora, eve, finn) = (self.dora, self.eve, self.finn);
        let mine =
            move |user: Option<i64>| user.is_some_and(|u| u == dora || u == eve || u == finn);
        {
            let world = self.handle();
            fake.route(
                "GET",
                "/api/v1/chats",
                Rc::new(move |fake, request| {
                    let user = request.user.unwrap_or_default();
                    if !mine(request.user) {
                        let state = fake.state();
                        let family = state
                            .user(user)
                            .and_then(|u| u.families.first().map(|f| f.0));
                        let rows = family
                            .and_then(|f| state.chats.get(&f).cloned())
                            .unwrap_or_default();
                        return FakeResponse::json(200, &ChatsResponse { chats: Some(rows) });
                    }
                    FakeResponse::json(
                        200,
                        &ChatsResponse {
                            chats: Some(world.rows_for(user)),
                        },
                    )
                }),
            );
        }
        {
            let world = self.handle();
            fake.route(
                "GET",
                "/api/v1/chats/*/messages",
                Rc::new(move |_, request| {
                    let chat = chat_in(request);
                    let query = query_of(request);
                    let limit = query
                        .get("limit")
                        .and_then(|l| l.parse::<usize>().ok())
                        .unwrap_or(50);
                    let messages = world.messages.borrow();
                    let mut in_chat: Vec<MessageDto> = messages
                        .values()
                        .filter(|m| m.chat_id == chat)
                        .cloned()
                        .collect();
                    if let Some(after) = query.get("after_id").and_then(|a| a.parse::<i64>().ok()) {
                        in_chat.retain(|m| m.id > after);
                        in_chat.truncate(limit);
                    } else {
                        if let Some(before) =
                            query.get("before_id").and_then(|b| b.parse::<i64>().ok())
                        {
                            in_chat.retain(|m| m.id < before);
                        }
                        in_chat.reverse();
                        in_chat.truncate(limit);
                    }
                    FakeResponse::json(
                        200,
                        &MessagesResponse {
                            messages: Some(in_chat),
                        },
                    )
                }),
            );
        }
        {
            let world = self.handle();
            fake.route(
                "POST",
                "/api/v1/chats/*/messages",
                Rc::new(move |_, request| {
                    let message = world.create(
                        request.user.unwrap_or_default(),
                        chat_in(request),
                        &request.json(),
                    );
                    FakeResponse::json(201, &MessageResponse { message })
                }),
            );
        }
        for method in ["PUT", "DELETE"] {
            let world = self.handle();
            fake.route(
                method,
                "/api/v1/chats/*/messages/*/reaction",
                Rc::new(move |_, request| {
                    let user = request.user.unwrap_or_default();
                    let id = message_in(request);
                    let emoji = request.json()["emoji"].as_str().map(str::to_string);
                    let mut messages = world.messages.borrow_mut();
                    let Some(held) = messages.get_mut(&id) else {
                        return FakeResponse::error(404, "message_not_found");
                    };
                    let mut reactions = held.reactions.clone().unwrap_or_default();
                    reactions.retain(|r| r.user_id != user);
                    if request.method == "PUT"
                        && let Some(emoji) = emoji
                    {
                        reactions.push(ReactionDto {
                            user_id: user,
                            emoji,
                        });
                    }
                    let seq = held.reaction_seq.unwrap_or(0) + 1;
                    held.reactions = Some(reactions.clone());
                    held.reaction_seq = Some(seq);
                    FakeResponse::json(
                        200,
                        &MessageReactionsDto {
                            message_id: id,
                            reaction_seq: seq,
                            reactions,
                        },
                    )
                }),
            );
        }
        {
            let world = self.handle();
            fake.route(
                "PATCH",
                "/api/v1/chats/*/messages/*",
                Rc::new(move |_, request| {
                    let id = message_in(request);
                    let mut messages = world.messages.borrow_mut();
                    let Some(held) = messages.get_mut(&id) else {
                        return FakeResponse::error(404, "message_not_found");
                    };
                    held.body = request.json()["body"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    held.edited_at = Some(now_rfc3339());
                    held.edit_seq = Some(held.edit_seq.unwrap_or(0) + 1);
                    FakeResponse::json(
                        200,
                        &MessageResponse {
                            message: held.clone(),
                        },
                    )
                }),
            );
        }
        for method in ["PUT", "DELETE"] {
            let world = self.handle();
            fake.route(
                method,
                "/api/v1/chats/*/messages/*/vote",
                Rc::new(move |_, request| {
                    let user = request.user.unwrap_or_default();
                    let id = message_in(request);
                    let option = request.json()["option_id"].as_i64();
                    let mut messages = world.messages.borrow_mut();
                    let Some(poll) = messages.get_mut(&id).and_then(|m| m.poll.as_mut()) else {
                        return FakeResponse::error(404, "message_not_found");
                    };
                    for each in &mut poll.options {
                        each.votes.retain(|v| *v != user);
                        if request.method == "PUT" && Some(each.id) == option {
                            each.votes.push(user);
                        }
                    }
                    poll.poll_seq += 1;
                    FakeResponse::json(
                        200,
                        &MessagePollDto {
                            message_id: id,
                            poll: poll.clone(),
                        },
                    )
                }),
            );
        }
        {
            let world = self.handle();
            fake.route(
                "GET",
                "/api/v1/chats/*/messages/*/thread",
                Rc::new(move |_, request| {
                    let id = message_in(request);
                    let messages = world.messages.borrow();
                    let root = messages
                        .get(&id)
                        .map_or(id, |m| m.thread_root_id.unwrap_or(m.id));
                    let after = query_of(request)
                        .get("after_id")
                        .and_then(|a| a.parse::<i64>().ok())
                        .unwrap_or(0);
                    let chain: Vec<MessageDto> = messages
                        .values()
                        .filter(|m| {
                            (m.id == root || m.thread_root_id == Some(root)) && m.id > after
                        })
                        .cloned()
                        .collect();
                    FakeResponse::json(
                        200,
                        &MessagesResponse {
                            messages: Some(chain),
                        },
                    )
                }),
            );
        }
        {
            let world = self.handle();
            fake.route(
                "POST",
                "/api/v1/chats/*/read",
                Rc::new(move |_, request| {
                    let user = request.user.unwrap_or_default();
                    let marker = request.json()["last_read_message_id"].as_i64().unwrap_or(0);
                    world
                        .reads
                        .borrow_mut()
                        .insert((user, chat_in(request)), marker);
                    FakeResponse::empty(204)
                }),
            );
        }
        {
            let world = self.handle();
            fake.route(
                "GET",
                "/api/v1/chats/*/polls/open",
                Rc::new(move |_, request| {
                    let chat = chat_in(request);
                    let polls: Vec<MessageDto> = world
                        .messages
                        .borrow()
                        .values()
                        .filter(|m| m.chat_id == chat && m.poll.as_ref().is_some_and(|p| !p.closed))
                        .cloned()
                        .collect();
                    FakeResponse::json(
                        200,
                        &MessagesResponse {
                            messages: Some(polls),
                        },
                    )
                }),
            );
        }
    }

    /// The pieces a route needs, shared.
    fn handle(&self) -> Rc<World> {
        Rc::new(World {
            fake: Rc::clone(&self.fake),
            dora: self.dora,
            eve: self.eve,
            finn: self.finn,
            family_chat: self.family_chat,
            direct: self.direct,
            media_chat: self.media_chat,
            group_chat: self.group_chat,
            wiring: Rc::clone(&self.wiring),
            uploads: Rc::clone(&self.uploads),
            messages: Rc::clone(&self.messages),
            next: Rc::clone(&self.next),
            reads: Rc::clone(&self.reads),
            refuse_next: Rc::clone(&self.refuse_next),
            sends: Rc::clone(&self.sends),
            seen_frames: Rc::clone(&self.seen_frames),
        })
    }

    fn last_send(&self) -> Option<serde_json::Value> {
        self.sends.borrow().last().cloned()
    }
}

fn chat_in(request: &FakeRequest) -> i64 {
    request
        .segments
        .get(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or_default()
}

fn message_in(request: &FakeRequest) -> i64 {
    request
        .segments
        .get(5)
        .and_then(|s| s.parse().ok())
        .unwrap_or_default()
}

fn query_of(request: &FakeRequest) -> HashMap<String, String> {
    request
        .query
        .as_deref()
        .unwrap_or_default()
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Every widget of type `T` under `root`, in tree order.
fn find<T: IsA<gtk::Widget>>(root: &impl IsA<gtk::Widget>) -> Vec<T> {
    let mut found = Vec::new();
    let mut pending: Vec<gtk::Widget> = vec![root.clone().upcast()];
    while let Some(widget) = pending.pop() {
        if let Ok(hit) = widget.clone().downcast::<T>() {
            found.push(hit);
        }
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            children.push(next);
        }
        pending.extend(children.into_iter().rev());
    }
    found
}

/// The button under `root` labelled `label`.
fn button(root: &impl IsA<gtk::Widget>, label: &str) -> Option<gtk::Button> {
    find::<gtk::Button>(root)
        .into_iter()
        .find(|b| b.label().as_deref() == Some(label) && b.is_visible())
}

pub async fn run(h: &Harness) {
    let shell = h.shell();
    let Some(main) = shell.main_window() else {
        h.check("main_window", false, "no main window");
        return;
    };
    let window: gtk::Window = main.window().clone().upcast();
    // P1's routes in their slot: a group before this one may have put a stand-in there.
    if let Some(chats) = chats::instance() {
        shell.chat_routes.install(chats);
    }
    let fake = h.fake_server();
    let world = Rc::new(World::new(&fake));
    // The socket's sends are answered here, every 40 ms.
    let responder = {
        let world = Rc::clone(&world);
        glib::timeout_add_local(Duration::from_millis(40), move || {
            world.answer_frames();
            glib::ControlFlow::Continue
        })
    };
    if h.input() {
        h.focus(&window).await;
    }

    gate(h, &shell, &main, &window, &world).await;
    if main.page() != GatePage::Chats {
        responder.remove();
        return;
    }
    conversation(h, &shell, &main, &window, &world).await;
    layout(h, &shell, &main, &window, &world).await;
    wiring::run(h, &shell, &main, &window, &world).await;

    let unhandled = fake.unhandled();
    h.check(
        "fake_server_knew_every_request",
        unhandled.is_empty(),
        format!("{unhandled:?}"),
    );
    responder.remove();
    h.screenshot("end").await;
}

/// Server → Sign In (a family-less account: the Family page) → Sign In as Dora → the chats.
async fn gate(
    h: &Harness,
    shell: &Rc<Shell>,
    main: &Rc<MainWindow>,
    window: &gtk::Window,
    world: &Rc<World>,
) {
    let url = world.fake.url();
    let probed = main.page() == GatePage::Server;
    if probed {
        let slot = main.page_slot(GatePage::Server);
        h.screenshot("server").await;
        let rows = find::<adw::EntryRow>(&slot);
        let typed = match rows.first() {
            Some(row) if h.input() => {
                h.click_middle(window, row).await;
                h.key("ctrl+a").await;
                h.type_text(&url).await;
                h.key("Return").await
            }
            Some(row) => {
                row.set_text(&url);
                row.emit_by_name::<()>("entry-activated", &[]);
                true
            }
            None => false,
        };
        // The probe asks `GET /me` with no token and takes only the protocol's own 401
        // (`unauthorized`), which the fake now answers: the page opens the server itself.
        let wanted = ServerUrl::normalise(Some(&url));
        let opened = h
            .wait_until(WAIT, || {
                main.page() != GatePage::Server
                    && main.page() != GatePage::Booting
                    && shell
                        .connection()
                        .is_some_and(|c| Some(&c.server) == wanted.as_ref())
            })
            .await;
        h.check(
            "gate_server_probe_opens_the_server",
            typed && opened,
            format!(
                "the address typed; GET /me answered 401 unauthorized → {:?}",
                main.page()
            ),
        );
        h.screenshot("server-opened").await;
    } else {
        // Another group left a server open: the fake as the server (what the probe opens).
        shell.open_server(ServerUrl::normalise(Some(&url)).expect("the fake's address"));
    }
    // A run after another group may find somebody signed in on it: signed out, to come through the
    // Sign In page.
    let _ = h
        .wait_until(WAIT, || {
            shell.connection().is_some() && main.page() != GatePage::Booting
        })
        .await;
    if matches!(
        main.page(),
        GatePage::Chats | GatePage::Family | GatePage::Pending
    ) && let Some(connection) = shell.connection()
    {
        let _ = connection.session.sign_out().await;
    }
    let signed_out = h.wait_until(WAIT, || main.page() == GatePage::SignIn).await;
    h.check(
        "gate_sign_in_page",
        signed_out,
        format!("{:?}", main.page()),
    );
    h.screenshot("sign-in").await;
    if !probed {
        // The Server page's probe through the Sign In page's "Change server…": `GET /me` with no
        // token answers the protocol's own `401 unauthorized`, so the probe takes the fake and the
        // server is opened from the form — no `Shell::open_server` behind the page's back.
        let slot = main.page_slot(GatePage::SignIn);
        let asked_me = |fake: &FakeServer| {
            fake.requests()
                .iter()
                .filter(|r| *r == "GET /api/v1/me")
                .count()
        };
        let me_before = asked_me(&world.fake);
        let change = button(&slot, shell.say.get("Change server…"));
        let pressed = match &change {
            Some(change) if h.input() => h.click_middle(window, change).await,
            Some(change) => {
                change.emit_clicked();
                true
            }
            None => false,
        };
        let row = h
            .wait_until(SHORT, || {
                find::<adw::EntryRow>(&slot)
                    .iter()
                    .any(|row| row.is_visible() && !row.is::<adw::PasswordEntryRow>())
                    && find::<adw::PasswordEntryRow>(&slot).is_empty()
            })
            .await;
        let typed = match find::<adw::EntryRow>(&slot).into_iter().next() {
            Some(row) if h.input() => {
                h.click_middle(window, &row).await;
                h.key("ctrl+a").await;
                h.type_text(&url).await;
                h.key("Return").await
            }
            Some(row) => {
                row.set_text(&url);
                row.emit_by_name::<()>("entry-activated", &[]);
                true
            }
            None => false,
        };
        let opened = h
            .wait_until(WAIT, || {
                main.page() == GatePage::SignIn
                    && !find::<adw::PasswordEntryRow>(&main.page_slot(GatePage::SignIn)).is_empty()
                    && asked_me(&world.fake) > me_before
            })
            .await;
        h.check(
            "gate_server_probe_opens_the_server",
            pressed && row && typed && opened,
            format!(
                "Change server… {pressed}, the form {row}, typed {typed}; GET /me answered 401 unauthorized → {:?}",
                main.page()
            ),
        );
        h.screenshot("server-probed").await;
    }

    // A family-less account sees the Family page's forms.
    let gus = world.fake.add_user("gus", "password", "Gus");
    let _ = gus;
    sign_in(h, main, window, "gus").await;
    let family = h.wait_until(WAIT, || main.page() == GatePage::Family).await;
    let forms = find::<adw::EntryRow>(&main.page_slot(GatePage::Family)).len();
    h.check(
        "gate_family_page",
        family && forms >= 2,
        format!("{:?}, {forms} rows (invite code, family name)", main.page()),
    );
    h.screenshot("gate-family").await;
    if let Some(connection) = shell.connection() {
        let _ = connection.session.sign_out().await;
    }
    let _ = h.wait_until(WAIT, || main.page() == GatePage::SignIn).await;

    sign_in(h, main, window, "dora").await;
    let chats = h.wait_until(WAIT, || main.page() == GatePage::Chats).await;
    h.check("gate_signed_in", chats, format!("{:?}", main.page()));
}

/// Types `username` / "password" into the Sign In page and presses Return.
async fn sign_in(h: &Harness, main: &Rc<MainWindow>, window: &gtk::Window, username: &str) {
    let slot = main.page_slot(GatePage::SignIn);
    let _ = h
        .wait_until(SHORT, || !find::<adw::PasswordEntryRow>(&slot).is_empty())
        .await;
    let user = find::<adw::EntryRow>(&slot)
        .into_iter()
        .find(|row| row.is_visible() && !row.is::<adw::PasswordEntryRow>());
    let secret = find::<adw::PasswordEntryRow>(&slot).into_iter().next();
    let (Some(user), Some(secret)) = (user, secret) else {
        h.check("sign_in_form", false, "no username/password rows");
        return;
    };
    if h.input() {
        h.click_middle(window, &user).await;
        h.key("ctrl+a").await;
        h.type_text(username).await;
        h.click_middle(window, &secret).await;
        h.key("ctrl+a").await;
        h.type_text("password").await;
        h.key("Return").await;
    } else {
        user.set_text(username);
        secret.set_text("password");
        secret.emit_by_name::<()>("entry-activated", &[]);
    }
}

async fn conversation(
    h: &Harness,
    shell: &Rc<Shell>,
    main: &Rc<MainWindow>,
    window: &gtk::Window,
    world: &Rc<World>,
) {
    let Some(chats) = chats::instance() else {
        h.check("chats", false, "no chats instance");
        return;
    };
    let Some(list) = chats.list() else {
        h.check("chats", false, "no list");
        return;
    };
    let listed = h
        .wait_until(WAIT, || {
            let ids = list.listed();
            ids.contains(&world.family_chat) && ids.contains(&world.direct)
        })
        .await;
    h.check(
        "list_rows",
        listed && list.listed().first() == Some(&world.family_chat),
        format!("{:?} (the family chat first)", list.listed()),
    );
    h.screenshot("list").await;

    // Open the direct chat with a click on its row.
    if let Some(row) = list.row(world.direct) {
        if h.input() {
            h.click_middle(window, &row).await;
        } else {
            shell.chat_routes.get().open_chat(world.direct);
        }
    }
    let opened = h
        .wait_until(WAIT, || {
            chats.page().is_some_and(|p| p.chat_id == world.direct)
        })
        .await;
    let Some(page) = chats.page() else {
        h.check("open_chat", false, "no page");
        return;
    };
    let drew = h
        .wait_until(WAIT, || {
            page.timeline().keys().iter().any(|k| k.starts_with('m'))
        })
        .await;
    h.check(
        "open_chat",
        opened && drew,
        format!("keys {:?}", page.timeline().keys()),
    );
    h.screenshot("direct").await;

    // A live frame.
    let incoming = MessageDto::new(
        world.next.get() + 1,
        world.direct,
        world.eve,
        None,
        "Are you there?",
        &now_rfc3339(),
    );
    world.next.set(incoming.id);
    world
        .messages
        .borrow_mut()
        .insert(incoming.id, incoming.clone());
    world.fake.send_frame_to(
        world.dora,
        &serde_json::json!({"type": "message", "message": incoming}).to_string(),
    );
    let key = conversation_rows::message_key(incoming.id);
    let arrived = h
        .wait_until(WAIT, || page.timeline().keys().contains(&key))
        .await;
    h.check(
        "receive_frame",
        arrived,
        format!("{key} in {:?}", page.timeline().keys()),
    );

    let composer = Rc::clone(page.composer());
    if h.input() {
        h.click_middle(window, composer.field()).await;
        // Capitals and emoji as you type, then Enter sends.
        h.type_text("hello there :)").await;
        h.sleep(Duration::from_millis(300)).await;
        let draft = composer.draft();
        let expected_tail = fc_logic::emoji_typing::typing(" :)");
        h.check(
            "capital_typed",
            draft.starts_with('H'),
            format!("{draft:?}"),
        );
        h.check(
            "emoji_typed",
            draft.ends_with(expected_tail.trim_start()),
            format!("{draft:?} ends with {expected_tail:?}"),
        );
        let before = world.sends.borrow().len();
        h.key("Return").await;
        let sent = h
            .wait_until(WAIT, || world.sends.borrow().len() > before)
            .await;
        h.check(
            "send_enter",
            sent && composer.draft().is_empty(),
            format!("{:?}", world.last_send()),
        );
        // Shift+Enter: a new line, nothing sent; then Enter sends both lines.
        let before = world.sends.borrow().len();
        h.type_text("line one").await;
        h.key("shift+Return").await;
        h.type_text("line two").await;
        h.sleep(Duration::from_millis(200)).await;
        let held = composer.draft();
        let none_sent = world.sends.borrow().len() == before;
        h.key("Return").await;
        let both = h
            .wait_until(WAIT, || {
                world
                    .last_send()
                    .is_some_and(|s| s["body"].as_str() == Some("Line one\nLine two"))
            })
            .await;
        h.check(
            "newline_shift_enter",
            held.contains('\n') && none_sent && both,
            format!("{held:?} → {:?}", world.last_send()),
        );
        h.screenshot("sent").await;
    } else {
        for check in [
            "capital_typed",
            "emoji_typed",
            "send_enter",
            "newline_shift_enter",
        ] {
            h.check(check, true, "skipped: --no-input");
        }
    }

    // React (the menu's action), reply, edit.
    act(
        main,
        "win.message-react",
        Some(&(incoming.id, "👍".to_string()).to_variant()),
    );
    let reacted = h
        .wait_until(WAIT, || {
            shell.connection().is_some_and(|c| {
                c.chats
                    .message(incoming.id)
                    .ok()
                    .flatten()
                    .is_some_and(|m| {
                        m.reactions
                            .unwrap_or_default()
                            .iter()
                            .any(|r| r.emoji == "👍")
                    })
            })
        })
        .await;
    h.check("react", reacted, "👍 on the incoming message");

    act(main, "win.message-reply", Some(&incoming.id.to_variant()));
    let primed = composer.reply() == Some(incoming.id);
    composer.set_text("Yes, here");
    composer.send();
    let replied = h
        .wait_until(WAIT, || {
            world
                .last_send()
                .is_some_and(|s| s["reply_to_message_id"].as_i64() == Some(incoming.id))
        })
        .await;
    h.check(
        "reply",
        primed && replied,
        format!("primed {primed}, {:?}", world.last_send()),
    );
    h.screenshot("reply").await;

    let mine = world
        .messages
        .borrow()
        .values()
        .rev()
        .find(|m| m.sender_id == world.dora && m.chat_id == world.direct)
        .map(|m| m.id);
    if let Some(own) = mine {
        let drawn = h
            .wait_until(WAIT, || {
                page.timeline()
                    .keys()
                    .contains(&conversation_rows::message_key(own))
            })
            .await;
        act(main, "win.message-edit", Some(&own.to_variant()));
        let editing = composer.editing() == Some(own);
        composer.set_text("Yes, I'm here");
        composer.press_slot();
        let edited = h
            .wait_until(WAIT, || {
                shell.connection().is_some_and(|c| {
                    c.chats
                        .message(own)
                        .ok()
                        .flatten()
                        .is_some_and(|m| m.body == "Yes, I'm here" && m.edited_at.is_some())
                })
            })
            .await;
        h.check(
            "edit",
            drawn && editing && edited && composer.editing().is_none(),
            format!("drawn {drawn}, editing {editing}, edited {edited}"),
        );
    } else {
        h.check("edit", false, "no own message to edit");
    }

    // A refused send, given up with Delete.
    world.refuse_next.set(true);
    composer.set_text("This one will fail");
    composer.send();
    let failed = h
        .wait_until(WAIT, || {
            page.model()
                .pending()
                .is_ok_and(|rows| rows.iter().any(|r| r.failed()))
        })
        .await;
    h.screenshot("failed").await;
    let delete = button(page.timeline().widget(), shell.say.get("Delete"));
    let clicked = match &delete {
        Some(delete) if h.input() => h.click_middle(window, delete).await,
        Some(delete) => {
            delete.emit_clicked();
            true
        }
        None => false,
    };
    let gone = h
        .wait_until(WAIT, || {
            page.model().pending().is_ok_and(|rows| rows.is_empty())
        })
        .await;
    h.check(
        "delete_failed_send",
        failed && clicked && gone,
        format!("failed {failed}, clicked {clicked}, gone {gone}"),
    );

    // A draft kept across chats.
    composer.set_text("half a thought");
    shell.chat_routes.get().open_chat(world.family_chat);
    let switched = h
        .wait_until(WAIT, || {
            chats.page().is_some_and(|p| p.chat_id == world.family_chat)
        })
        .await;
    let Some(family) = chats.page() else {
        h.check("draft_kept", false, "no family page");
        return;
    };
    let _ = h
        .wait_until(WAIT, || family.timeline().keys().len() >= 3)
        .await;
    h.screenshot("family").await;

    // The typing line.
    world.fake.send_frame(
        &serde_json::json!({"type": "typing", "chat_id": world.family_chat, "user_id": world.eve})
            .to_string(),
    );
    let typing = h
        .wait_until(SHORT, || main.content_title().subtitle().contains("Eve"))
        .await;
    h.check(
        "typing_line",
        typing,
        format!("{:?}", main.content_title().subtitle()),
    );
    h.screenshot("typing").await;

    // A mention from the @ list.
    let family_composer = Rc::clone(family.composer());
    if h.input() {
        h.click_middle(window, family_composer.field()).await;
        h.type_text("@Ev").await;
        let shown = h
            .wait_until(SHORT, || family_composer.mentions_shown())
            .await;
        h.screenshot("mention").await;
        h.key("Tab").await;
        h.type_text("come too").await;
        h.key("Return").await;
        let mentioned = h
            .wait_until(WAIT, || {
                world.last_send().is_some_and(|s| {
                    s["mentions"]
                        .as_array()
                        .is_some_and(|m| m.iter().any(|m| m["user_id"].as_i64() == Some(world.eve)))
                })
            })
            .await;
        h.check(
            "mention",
            shown && mentioned,
            format!("shown {shown}, {:?}", world.last_send()),
        );
    } else {
        h.check("mention", true, "skipped: --no-input");
    }

    // Vote in the poll: a click on its first option's button.
    let poll_id = world
        .messages
        .borrow()
        .values()
        .find(|m| m.poll.is_some() && m.chat_id == world.family_chat)
        .map(|m| m.id);
    let option = find::<gtk::Button>(family.timeline().widget())
        .into_iter()
        .find(|b| b.has_css_class("fc-chat-poll-option"));
    if let (Some(poll_id), Some(option)) = (poll_id, option) {
        if h.input() {
            h.click_middle(window, &option).await;
        } else {
            option.emit_clicked();
        }
        let voted = h
            .wait_until(WAIT, || {
                shell.connection().is_some_and(|c| {
                    c.chats
                        .message(poll_id)
                        .ok()
                        .flatten()
                        .and_then(|m| m.poll)
                        .is_some_and(|p| p.options.iter().any(|o| o.votes.contains(&world.dora)))
                })
            })
            .await;
        h.check("poll_vote", voted, format!("poll {poll_id}"));
    } else {
        h.check("poll_vote", false, "no poll card drawn");
    }

    // A thread: open it from the root, reply in it, close it.
    let root = world
        .messages
        .borrow()
        .values()
        .find(|m| m.reply_count.unwrap_or(0) > 0)
        .map(|m| m.id);
    if let Some(root) = root {
        act(main, "win.message-view-thread", Some(&root.to_variant()));
        let open = h
            .wait_until(SHORT, || main.window().visible_dialog().is_some())
            .await;
        let thread = family.thread();
        let drew = h
            .wait_until(WAIT, || {
                thread
                    .as_ref()
                    .is_some_and(|t| t.timeline().keys().len() >= 2)
            })
            .await;
        h.check("thread_open", open && drew, format!("root {root}"));
        h.screenshot("thread").await;
        if let Some(thread) = thread {
            thread.composer().set_text("Count me in");
            thread.composer().send();
            let answered = h
                .wait_until(WAIT, || {
                    world.last_send().is_some_and(|s| {
                        s["reply_to_message_id"].as_i64() == Some(root)
                            && s["body"].as_str() == Some("Count me in")
                    })
                })
                .await;
            h.check("thread_reply", answered, format!("{:?}", world.last_send()));
            thread.close();
        }
        let _ = h
            .wait_until(SHORT, || main.window().visible_dialog().is_none())
            .await;
    } else {
        h.check("thread_open", false, "no root with replies");
    }

    // Back to the direct chat: its draft is still there.
    shell.chat_routes.get().open_chat(world.direct);
    let back = h
        .wait_until(WAIT, || {
            chats.page().is_some_and(|p| p.chat_id == world.direct)
        })
        .await;
    let kept = chats
        .page()
        .is_some_and(|p| p.composer().draft() == "half a thought");
    h.check(
        "draft_kept",
        switched && back && kept,
        format!("{:?}", chats.page().map(|p| p.composer().draft())),
    );

    // win.paste-attachment follows the field's focus.
    if h.input()
        && let Some(page) = chats.page()
    {
        h.click_middle(window, page.composer().field()).await;
        let focused = h
            .wait_until(SHORT, || page.composer().field().has_focus())
            .await;
        let off = h
            .wait_until(SHORT, || !action_enabled(main, "paste-attachment"))
            .await
            && focused;
        if let Some(row) = page.timeline().rows().first() {
            row.grab_focus();
        }
        let on = h
            .wait_until(SHORT, || action_enabled(main, "paste-attachment"))
            .await;
        h.check(
            "paste_action_follows_focus",
            off && on,
            format!("focused: off {off}; elsewhere: on {on}"),
        );
    }

    // A drop, staged the way a drop stages (Xvfb has no drag source).
    if let Some(page) = chats.page() {
        let file = h.out().join("dropped.png");
        let png = gtk::gdk::MemoryTexture::new(
            2,
            2,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from(&[255u8; 16]),
            8,
        )
        .save_to_png_bytes();
        let written = std::fs::write(&file, &png).is_ok();
        page.composer()
            .stage_files(vec![gio::File::for_path(&file)]);
        let staged = h
            .wait_until(WAIT, || page.composer().staging().len() == 1)
            .await;
        h.screenshot("staged").await;
        h.check(
            "drop_staged",
            written && staged,
            format!(
                "{} staged, notice {:?}",
                page.composer().staging().len(),
                page.composer().notice_text()
            ),
        );
        page.composer().staging().take_all();
        page.composer().refresh();
    }
}

/// Activates a `win.*` action on the main window, as a menu item would.
fn act(main: &Rc<MainWindow>, name: &str, target: Option<&glib::Variant>) {
    if WidgetExt::activate_action(main.window(), name, target).is_err() {
        eprintln!("selftest: no action {name}");
    }
}

fn action_enabled(main: &Rc<MainWindow>, name: &str) -> bool {
    main.window()
        .lookup_action(name)
        .is_some_and(|action| action.is_enabled())
}

async fn layout(
    h: &Harness,
    shell: &Rc<Shell>,
    main: &Rc<MainWindow>,
    window: &gtk::Window,
    world: &Rc<World>,
) {
    let Some(chats) = chats::instance() else {
        return;
    };
    // The conversation in its own window.
    conversation_window::open(world.family_chat);
    let opened = h
        .wait_until(SHORT, || {
            conversation_window::window_for(world.family_chat)
                .is_some_and(|w| w.window().is_visible() && w.page().is_some())
        })
        .await;
    h.check("conversation_window", opened, "720 × 640, the family chat");
    h.screenshot("window").await;
    if let Some(opened) = conversation_window::window_for(world.family_chat) {
        opened.window().close();
    }
    let closed = h
        .wait_until(SHORT, || {
            conversation_window::window_for(world.family_chat).is_none()
        })
        .await;
    h.check("conversation_window_closes", closed, "");

    // The list by keyboard alone: focus a row, Down, Enter.
    if h.input()
        && let Some(list) = chats.list()
    {
        h.focus(window).await;
        if let Some(first) = list.row(world.family_chat) {
            first.grab_focus();
        }
        h.key("Down").await;
        h.key("Return").await;
        let moved = h
            .wait_until(WAIT, || {
                chats.page().is_some_and(|p| p.chat_id == world.direct)
            })
            .await;
        h.key("Up").await;
        h.key("Return").await;
        let back = h
            .wait_until(WAIT, || {
                chats.page().is_some_and(|p| p.chat_id == world.family_chat)
            })
            .await;
        h.check(
            "keyboard_list",
            moved && back,
            format!("{:?}", chats.selected()),
        );
    }

    // The narrow window: one pane at a time; a chat chosen brings the conversation forward.
    if h.input() {
        h.resize(window, 520, 600).await;
        let collapsed = h
            .wait_until(SHORT, || main.split_view().is_collapsed())
            .await;
        shell.chat_routes.get().open_chat(world.direct);
        let forward = h
            .wait_until(SHORT, || main.split_view().shows_content())
            .await;
        h.check(
            "narrow",
            collapsed && forward,
            "collapsed, the content pane forward",
        );
        h.screenshot("narrow").await;
        h.resize(window, DEFAULT_SIZE.0, DEFAULT_SIZE.1).await;
        let _ = h
            .wait_until(SHORT, || !main.split_view().is_collapsed())
            .await;
    }

    // Dark mode.
    let styles = adw::StyleManager::default();
    styles.set_color_scheme(adw::ColorScheme::ForceDark);
    let dark = h.wait_until(SHORT, crate::theme::is_dark).await;
    h.sleep(Duration::from_millis(300)).await;
    h.check("dark", dark, "the conversation redrawn in the dark scheme");
    h.screenshot("dark").await;
    styles.set_color_scheme(adw::ColorScheme::Default);
    let _ = h.wait_until(SHORT, || !crate::theme::is_dark()).await;
}
