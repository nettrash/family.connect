//! The cache, the client and the socket the conversation tests open a chat over — what each C#
//! test class builds in its constructor and its `Open`/`Build`: an in-memory cache read by member
//! 7 holding the family chat 42, an outbox on it, and a clock that moves only when told.

use std::rc::Rc;

use fc_core::protocol::{ApiClient, ChatDto, ChatRowDto, MemberDto, SendPipeline};
use fc_core::store::{ChatStore, Database, OutboxStore, StoreResult};
use fc_core::testing::{FakeClock, FakeFrameSender, StillClock, drive};
use fc_logic::conversation::ConversationModel;
use time::OffsetDateTime;
use time::macros::datetime;

use super::server::{Server, api};

pub const ME: i64 = 7;
pub const CHAT: i64 = 42;

/// When every rig's clock starts (`Start` in `ConversationModelTests`).
pub const START: OffsetDateTime = datetime!(2026-09-12 12:00 UTC);

pub struct ChatRig {
    pub cache: Rc<Database>,
    pub chats: Rc<ChatStore>,
    pub outbox: Rc<OutboxStore>,
    pub clock: FakeClock,
}

impl ChatRig {
    /// `chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))])`.
    pub fn new() -> StoreResult<ChatRig> {
        let cache = Database::open_in_memory()?;
        let chats = Rc::new(ChatStore::new(&cache).with_me(|| ME));
        chats.replace(
            &[ChatRowDto::new(ChatDto::new(CHAT, "family", "The Smiths"))],
            None,
        )?;
        Ok(ChatRig {
            outbox: Rc::new(OutboxStore::new(&cache)),
            chats,
            cache,
            clock: FakeClock::at(START),
        })
    }

    pub fn api(&self, server: &Server) -> Rc<ApiClient> {
        api(server, &self.clock)
    }

    /// The pipeline the C# builds as `new SendPipeline(socket, outbox, chats, api, wait: …)`: posting
    /// through the client, its waits ending at once.
    pub fn pipeline(&self, socket: &FakeFrameSender, api: &Rc<ApiClient>) -> Rc<SendPipeline> {
        Rc::new(SendPipeline::from_api(
            Rc::new(socket.clone()),
            self.outbox.clone(),
            self.chats.clone(),
            api.clone(),
            Rc::new(StillClock::at(START)),
        ))
    }

    /// The C# tests' `Open(server)`: a conversation of chat 42 over a socket that is down, with the
    /// outbox its pending rows come from.
    pub fn open(&self, server: &Server) -> StoreResult<ConversationModel> {
        let api = self.api(server);
        let socket = socket(false);
        let sending = self.pipeline(&socket, &api);
        Ok(ConversationModel::new(
            CHAT,
            self.chats.clone(),
            api,
            sending,
            Rc::new(socket),
            Rc::new(self.clock.clone()),
        )?
        .with_outbox(self.outbox.clone()))
    }

    /// Run a future to its end on this rig's clock.
    pub fn drive<F: std::future::Future>(&self, future: F) -> F::Output {
        drive(&self.clock, future)
    }
}

/// `ChatActionsTests`' constructor: Anna (the reader), Bob, Carl and Dora (blocked); Eve gone.
pub fn actions_rig() -> StoreResult<ChatRig> {
    let rig = ChatRig::new()?;
    rig.chats.replace_members(
        &[
            MemberDto::new(7, "anna", "Anna"),
            MemberDto::new(11, "bob", "Bob"),
            MemberDto::new(12, "carl", "Carl"),
            MemberDto::new(13, "dora", "Dora"),
        ],
        &[MemberDto {
            has_left: true,
            deleted: true,
            ..MemberDto::new(14, "gone", "Eve")
        }],
    )?;
    rig.chats.set_blocked(13, true)?;
    Ok(rig)
}

/// The C# tests' `Wire`: up or down, and a write goes exactly when it is up.
pub fn socket(up: bool) -> FakeFrameSender {
    let socket = FakeFrameSender::new();
    set_up(&socket, up);
    socket
}

/// `IsConnected = up`, for a socket whose writes go only while it is.
pub fn set_up(socket: &FakeFrameSender, up: bool) {
    socket.set_connected(up);
    socket.set_writes_go_through(up);
}

/// Whether `text` is a UUID as a dedup key is written: 8-4-4-4-12 hex digits (what
/// `Guid.TryParse` accepts of the keys this client makes).
pub fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, length)| {
            group.len() == length && group.chars().all(|c| c.is_ascii_hexdigit())
        })
}
