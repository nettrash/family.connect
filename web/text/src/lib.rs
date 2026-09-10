//! Family Connect's message-text rules, ported from the Apple app so that a
//! message reads the same on iPhone, Mac, Android and the web.
//!
//! Each module is a port of one Swift file, checked against that file's own
//! Swift tests (and, where noted, against the Swift code itself run over
//! generated inputs). They are independent of each other: the web client
//! composes them, and the order it composes them in is the Apple app's.
//!
//! - [`markdown`] — the markdown subset people type, as styled runs (only a
//!   table splits a body), exactly as Apple's Foundation renders it.
//! - [`emoji`] — which messages are "emoji only" and how big they draw, the
//!   quick reaction set, and the "More reactions…" catalogue.
//! - [`reactions`] — a message's reactions as chips and as "See who reacted".
//! - [`mentions`] — `@Name`: the composer's trigger and suggestions, who a
//!   text names at send, and which tokens an incoming body highlights.
//! - [`assistant`] — `@ai` and `/draw`, by the server's own grammar.
//! - [`composer`] — the 4,000-character body limit and where a cut may land.
//! - [`call_record`] — how a call record reads.
//! - [`excerpt`] — the server's 120-character quote cut.
//! - [`links`] — web links, email addresses and phone numbers in a text run,
//!   and the markdown label/destination precedence. Only http, https,
//!   `mailto:` and `tel:` ever link — in a browser, anything else is a way
//!   to run script.

pub mod assistant;
pub mod call_record;
pub mod composer;
pub mod emoji;
pub mod excerpt;
pub mod links;
pub mod markdown;
pub mod mentions;
pub mod reactions;
