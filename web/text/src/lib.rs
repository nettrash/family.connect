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
//! - [`assistant_pictures`] — what a composer says before a photograph goes
//!   to the assistant, and the four-photo, 5 MiB, JPEG-or-PNG bounds.
//! - [`composer`] — the 4,000-character body limit and where a cut may land.
//! - [`call_record`] — how a call record reads.
//! - [`excerpt`] — the server's 120-character quote cut.
//! - [`media`] — attachments: which kind a picked file goes as, its name and
//!   type, the server's magic-number check, and how a bubble lays one out.
//! - [`media_plan`] — what a picked video or sound file becomes before it is
//!   uploaded: kept, or transcoded to the protocol's profile and to exactly
//!   which numbers — the rules every port is held to by the oracle's vectors.
//! - [`wav`] — a voice note as WAV, for browsers that cannot record MP4.
//! - [`account`] — usernames, passwords, names, invite codes, birthdays, the
//!   family's languages and the owner's member cap, by the server's rules.
//! - [`avatar`] — the initials a profile circle draws with no picture.
//! - [`board`] — the family board: what a note may say, the names its size,
//!   colour, face and kind travel under, where it sits, how its text is
//!   fitted, and what the badge counts.
//! - [`links`] — web links, email addresses and phone numbers in a text run,
//!   and the markdown label/destination precedence. Only http, https,
//!   `mailto:` and `tel:` ever link — in a browser, anything else is a way
//!   to run script.
//! - [`i18n`] — the nine languages, and how a string is found in them: the
//!   KEY is the apps' own English source string, so the modules above say
//!   the same words the phone says. Every other module here reads the
//!   reader's language through it (docs/protocol.md, "A browser is a client
//!   too").
//! - [`notify`] — what a browser's own notification says, and the unread
//!   count in the page's title.
//! - [`pack`] — the family's sticker pack: which bytes are a sticker as they
//!   stand, the 512 × 512 box one is made to fit, who may remove one, and
//!   the order a panel shows them in. (Everywhere else here "sticker" is a
//!   board note; in that module it is the picture sent in a chat.)
//! - [`transcript`] — the text of a recording on request: whether "Show
//!   text" is offered, what each refusal means, and what the device keeps.
//! - [`transcript_sound`] — the sound a device sends for that text when the
//!   server's stored copy will not do (a video, Ogg, an oversized file):
//!   AAC copied where it fits, re-encoded to 64 kbit/s mono where not.
//! - [`lookups`] — the assistant looking things up (#72): which providers a
//!   client names and how, what the consent screen asks, and where the
//!   server's sources footer starts under an answer.
//! - [`greeting_places`] — the places the owner names for today's weather
//!   in the daily greeting (#72): when the field is offered, and what a name
//!   may be, by the server's own rules.

pub mod account;
pub mod assistant;
pub mod assistant_consent;
pub mod assistant_pictures;
pub mod avatar;
pub mod board;
pub mod calendar;
pub mod call_record;
pub mod composer;
pub mod emoji;
pub mod excerpt;
pub mod greeting_places;
pub mod i18n;
pub mod links;
pub mod lookups;
pub mod markdown;
pub mod media;
pub mod media_plan;
pub mod media_probe;
pub mod mentions;
pub mod mp4;
pub mod mp4_read;
pub mod notify;
pub mod pack;
pub mod reactions;
pub mod transcode;
pub mod transcript;
pub mod transcript_sound;
pub mod wav;
