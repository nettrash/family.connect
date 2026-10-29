//! Helpers of the w2 logic-session package's consent tests: the assistant documents the C# suites
//! build (`new AssistantDto(99, "Assistant", "@ai", Images: …, Processor: …)`), a list of names, and
//! a clock for the requests' deadlines.

use fc_core::protocol::AssistantDto;
use fc_core::testing::FakeClock;
use time::macros::datetime;

/// The processor every C# consent suite names.
pub const PROCESSOR: &str = "Microsoft — Azure OpenAI";

/// `new AssistantDto(userId, displayName, "@ai")`: no processor, draws nothing.
pub fn assistant(user_id: i64, display_name: &str) -> AssistantDto {
    AssistantDto {
        user_id,
        display_name: display_name.to_string(),
        mention: "@ai".to_string(),
        ..AssistantDto::default()
    }
}

/// An assistant that names who answers, and talks.
pub fn talks() -> AssistantDto {
    AssistantDto {
        processor: Some(PROCESSOR.to_string()),
        ..assistant(99, "Assistant")
    }
}

/// An assistant that names who answers, and draws.
pub fn draws() -> AssistantDto {
    AssistantDto {
        images: true,
        ..talks()
    }
}

/// The same server before it said who answers — an assistant this client will not use.
pub fn unnamed() -> AssistantDto {
    AssistantDto {
        images: true,
        ..assistant(99, "Assistant")
    }
}

/// A list of names as the functions take one.
pub fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

/// A clock for the requests' deadlines; nothing here waits on it.
pub fn clock() -> FakeClock {
    FakeClock::at(datetime!(2026-10-03 09:30 UTC))
}
