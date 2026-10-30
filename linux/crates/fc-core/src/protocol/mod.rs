//! The wire: a port of `win/src/FamilyConnect.Core/Protocol` (docs/protocol.md).
//!
//! The data model is here — the objects ([`dtos`]), the request and answer bodies ([`requests`]),
//! the socket's frames ([`frames`]), the error shape ([`api_error`], [`api_result`]), the server's
//! two URLs ([`server_url`]), the upload queries ([`upload_queries`]) and the outbox's rules
//! ([`send_rules`], [`reconnect_backoff`]). The API client ([`api_client`]) and the live socket
//! ([`chat_socket`]) speak it; the frame router ([`frame_router`]), the send pipeline
//! ([`send_pipeline`]) and the resync ([`resync`]) join the wire to the cache.

pub mod api_client;
pub mod api_error;
pub mod api_result;
pub mod chat_socket;
pub mod dtos;
pub mod frame_router;
pub mod frames;
pub mod reconnect_backoff;
pub mod requests;
pub mod resync;
pub mod send_pipeline;
pub mod send_rules;
pub mod server_url;
pub mod upload_queries;

pub use api_client::{ApiClient, FAMILIES_HEADER, UploadFacts};
pub use api_error::{ApiError, error_codes};
pub use api_result::{ApiResult, MemoryTokenStore, Nothing, TokenStore};
pub use chat_socket::{ChatSocket, FrameSender, LiveSocket};
pub use dtos::{
    AttachmentDto, BirthdayDto, CallRecordDto, ChatDto, ErrorBody, ErrorEnvelope, MemberDto,
    MentionDto, MessageDto, NoteDto, PackItemDto, PollDto, PollOptionDto, QuoteParentDto,
    ReactionDto, ReplyToDto, RsvpDto, TaskItemDto, UserDto, wire,
};
pub use frame_router::FrameRouter;
pub use frames::{AiErrorReason, IceCandidate, ServerFrame, client_frames};
pub use reconnect_backoff::ReconnectBackoff;
pub use requests::*;
pub use resync::{FlagRepairOutcome, Resync, repair_outcome};
pub use send_pipeline::{QueueOptions, SendPipeline};
pub use server_url::ServerUrl;
