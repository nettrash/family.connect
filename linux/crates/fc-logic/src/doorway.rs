//! The first screens: which server, who you are, which family — and the one sentence each refusal
//! becomes; with the two small wordings the chat list and a bubble share (`Doorway.cs`).
//!
//! "CONNECT" ASKS THE SERVER, NOT THE ADDRESS. A typo'd host, a router's admin page and a random
//! website all "answer", so the check is the one request whose refusal only this protocol writes:
//! `GET /me` with no token, answered `401` with a code the protocol names. A web server behind a
//! login page answers 401 too — with HTML, which parses to no code at all, so it is not mistaken for
//! a family's server. The same probe as the Apple client's.
//!
//! PLAIN HTTP IS ALLOWED AND SAID. A family on a home network with no certificate types `http://`
//! and means it; the desktop has no transport policy that would refuse it later, so the only honest
//! thing is the warning the other clients show.

use std::collections::HashSet;
use std::rc::Rc;

use fc_core::protocol::{
    ApiClient, ApiError, MemoryTokenStore, MessageDto, ServerUrl, error_codes,
};
use fc_core::rt::{Clock, Http};
use fc_core::store::{ChatStore, StoreResult, times};
use fc_core::text::string_catalog::StringCatalog;
use time::{OffsetDateTime, PrimitiveDateTime};

use crate::app_session::AppSession;
use crate::chat_list::{ChatListModel, RowTimeKind};
use crate::conversation::Bubble;
use crate::date_culture::{self, DateCulture, TimeZone};
use crate::families::family_sentences;

/// What counts as an address, and what counts as a Family Connect server answering at it
/// (`ServerCheck`).
pub mod server_check {
    use super::*;

    /// What people type, as a base URL — or `None` when there is no host in it.
    pub fn parse(typed: Option<&str>) -> Option<ServerUrl> {
        ServerUrl::normalise(typed)
    }

    /// Whether the traffic to this server can be read by anyone on the same network.
    pub fn is_plain_text(server: &ServerUrl) -> bool {
        server.scheme() == "http"
    }

    /// Whether a Family Connect server answers at `server`: `GET /me` with no token, refused with
    /// `401` in the protocol's own words.
    pub async fn answers(http: Rc<dyn Http>, server: ServerUrl, clock: Rc<dyn Clock>) -> bool {
        let api = ApiClient::new(http, server, Rc::new(MemoryTokenStore::new(None)), clock);
        api.me()
            .await
            .error()
            .is_some_and(|error| error.status == 401 && error.canonical())
    }
}

/// The sentence each refusal at the door becomes (`DoorSentences`). Keyed on the protocol's machine
/// codes so the copy can be specific, and worded exactly as the Apple client words it — the
/// catalogue is shared, and a sentence that differed would be a sentence nobody had translated.
///
/// A SERVER'S OWN MESSAGE IS SHOWN ONLY WHEN IT CAME WITH A CODE. `validation` names the rule that
/// was broken, and that is worth reading. A proxy's reason phrase ("Bad Request") is not, and a body
/// the client could not read carries nothing but that.
pub mod door_sentences {
    use super::*;

    /// Signing in, or registering.
    pub fn sign_in(error: &ApiError, registering: bool, say: &dyn StringCatalog) -> String {
        if error.code == error_codes::USERNAME_TAKEN {
            return say.get("That username is taken.").to_string();
        }
        match error.status {
            // A wrong password is 401 with `invalid_credentials` — an answer to the form in front of
            // them, not a session that ended.
            401 if !registering => say.get("Wrong username or password.").to_string(),
            401 => say
                .get("The server rejected the request. Try again.")
                .to_string(),
            _ if error.code == error_codes::TRANSPORT => say
                .get("Can't reach the server. Check your connection.")
                .to_string(),
            429 => say.get("Try again in a moment.").to_string(),
            500.. => say
                .get("The server had a problem. Try again in a moment.")
                .to_string(),
            400 | 409 | 422 => said(error)
                .unwrap_or_else(|| say.get("The server rejected the request.").to_string()),
            _ => say.get("Something went wrong. Try again.").to_string(),
        }
    }

    /// Joining with an invite code. `cap` is the account's `max_families_per_account`, for
    /// `too_many_families` (docs/protocol.md, "Several families"); `another` is joining from inside
    /// a family ("Join Another Family…"), where `already_in_family` means THAT family. (The C#'s
    /// defaults: `cap` 1, `another` false.)
    pub fn join(error: &ApiError, say: &dyn StringCatalog, cap: i32, another: bool) -> String {
        match error.code.as_str() {
            error_codes::TOO_MANY_FAMILIES => {
                return family_sentences::for_error(error, cap, say).unwrap_or_default();
            }
            error_codes::ALREADY_IN_FAMILY if another => {
                return say.get("You're already in that family.").to_string();
            }
            // A CLOSED family answers this too, byte-identical to a code that never existed — so the
            // sentence is the same, and it is right about both.
            error_codes::INVALID_INVITE_CODE => {
                return say
                    .get("That code doesn't match any family. Check it and try again.")
                    .to_string();
            }
            _ => {}
        }
        if error.status == 404 {
            return say.get("That code doesn't work anymore.").to_string();
        }
        match error.code.as_str() {
            error_codes::ALREADY_IN_FAMILY => say.get("You're already in a family.").to_string(),
            error_codes::JOIN_REQUEST_PENDING => {
                say.get("You already have a pending request.").to_string()
            }
            // The one thing this endpoint admits to a stranger, on purpose: telling an invited member
            // their code is invalid on the day the family filled up costs a real person a real join.
            error_codes::FAMILY_FULL => say
                .get("That family is full right now. Ask them to make room, then try the code again.")
                .to_string(),
            _ if matches!(error.status, 400 | 409) => {
                said(error).unwrap_or_else(|| say.get("The server rejected that code.").to_string())
            }
            _ => say.get("Can't reach the server. Try again.").to_string(),
        }
    }

    /// Starting a family. `cap` is the account's `max_families_per_account`, for
    /// `too_many_families` (the C#'s default: 1).
    pub fn create(error: &ApiError, say: &dyn StringCatalog, cap: i32) -> String {
        match error.code.as_str() {
            error_codes::FAMILY_REGISTRATION_DISABLED => say
                .get("This server doesn't take new families.")
                .to_string(),
            // Starting ANOTHER family from inside one (docs/protocol.md, "Several families"): the
            // three doors it can meet shut.
            error_codes::TOO_MANY_FAMILIES
            | error_codes::ALREADY_OWN_A_FAMILY
            | error_codes::MEMBER_FAMILIES_DISABLED => {
                family_sentences::for_error(error, cap, say).unwrap_or_default()
            }
            error_codes::ALREADY_IN_FAMILY => say.get("You're already in a family.").to_string(),
            // A server that charges for a new family, and no active subscription (docs/protocol.md,
            // "Paid families"): terminal, said in the door's own line.
            error_codes::SUBSCRIPTION_REQUIRED => crate::paid_families::subscription_required(say),
            _ if matches!(error.status, 400 | 409) => {
                said(error).unwrap_or_else(|| say.get("The server rejected that name.").to_string())
            }
            _ => say.get("Can't reach the server. Try again.").to_string(),
        }
    }

    fn said(error: &ApiError) -> Option<String> {
        (error.canonical() && !error.message.is_empty()).then(|| error.message.clone())
    }
}

/// The family door for somebody signed in and in no family — and "Join Another Family…" / "New
/// Family…" for somebody in one: join one with a code, or start one (`FamilyDoor`).
///
/// THE SCREEN IS DECIDED BY `GET /me`, NOT BY THE ANSWER. A join answers `joined` or `pending` and
/// a create answers the family, but which screen that means — the chats, or waiting for an owner —
/// is the one decision [`AppSession::refresh`] makes, and making it twice is how two screens come to
/// disagree.
pub struct FamilyDoor {
    api: Rc<ApiClient>,
    session: Rc<AppSession>,
    say: Rc<dyn StringCatalog>,
}

impl std::fmt::Debug for FamilyDoor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FamilyDoor").finish_non_exhaustive()
    }
}

impl FamilyDoor {
    /// The longest family name the server takes.
    pub const NAME_LIMIT: usize = 64;

    pub fn new(api: Rc<ApiClient>, session: Rc<AppSession>, say: Rc<dyn StringCatalog>) -> Self {
        FamilyDoor { api, session, say }
    }

    pub fn may_join(code: Option<&str>) -> bool {
        code.is_some_and(|code| !code.trim().is_empty())
    }

    /// A name of 1 to [`NAME_LIMIT`](Self::NAME_LIMIT) once trimmed, counted in UTF-16 units as the
    /// C#'s `string.Length` is (the server's own rule is the one that is said when it refuses).
    pub fn may_create(name: Option<&str>) -> bool {
        name.is_some_and(|name| {
            (1..=Self::NAME_LIMIT).contains(&name.trim().encode_utf16().count())
        })
    }

    /// Join. The answer is the sentence to show, or `None` when it worked.
    ///
    /// From inside a family too — "Join Another Family…" (docs/protocol.md, "Several families"):
    /// `joined` adds the family and SHOWS it; `pending` leaves the family shown where it is, with a
    /// waiting row in the switcher.
    pub async fn join(&self, code: &str) -> StoreResult<Option<String>> {
        let state = self.session.state();
        let before: HashSet<i64> = state
            .all_families()
            .iter()
            .map(|family| family.id)
            .collect();
        let answer = match self.api.join_family(code.trim()).await.into_result() {
            Ok(answer) => answer,
            Err(error) => {
                return Ok(Some(door_sentences::join(
                    &error,
                    self.say.as_ref(),
                    state.max_families_per_account,
                    !before.is_empty(),
                )));
            }
        };
        // The answer names the family (2026-10-22): joined, it is the one to show. A server that
        // names none leaves the comparison — the family that was not in `/me` before is it.
        if answer.joined()
            && let Some(named) = answer.family_id
        {
            self.session.refresh_preferring(Some(named)).await?;
            return Ok(None);
        }
        self.session.refresh().await?;
        if answer.joined() && !before.is_empty() {
            let joined = self
                .session
                .state()
                .all_families()
                .iter()
                .find(|family| !before.contains(&family.id))
                .map(|family| family.id);
            if let Some(joined) = joined {
                self.session.switch_family(joined).await?;
            }
        }
        Ok(None)
    }

    /// Start a family. The answer is the sentence to show, or `None` when it worked. From inside a
    /// family too — "New Family…": the creator becomes its owner, and the family is added and SHOWN.
    pub async fn create(&self, name: &str) -> StoreResult<Option<String>> {
        match self.api.create_family(name.trim()).await.into_result() {
            Err(error) => Ok(Some(door_sentences::create(
                &error,
                self.say.as_ref(),
                self.session.state().max_families_per_account,
            ))),
            Ok(created) => {
                self.session
                    .refresh_preferring(Some(created.family.id))
                    .await?;
                Ok(None)
            }
        }
    }
}

/// The sign-in form's one rule of its own: nothing is sent while a field it needs is empty
/// (`SignInForm`).
///
/// The server enforces the real rules (a username of 3–32 letters, digits, dots or underscores; a
/// password of at least 8; a display name of 1–64) and names the one that was broken. Checking them
/// here as well would put a second, drifting copy of them in front of the server's own words.
pub mod sign_in_form {
    pub fn may_submit(
        username: Option<&str>,
        password: Option<&str>,
        display_name: Option<&str>,
        registering: bool,
    ) -> bool {
        let filled = |text: Option<&str>| text.is_some_and(|text| !text.trim().is_empty());
        filled(username)
            && password.is_some_and(|password| !password.is_empty())
            && (!registering || filled(display_name))
    }
}

/// The time on a chat row, in the reader's language and the reader's zone (`RowTimeText`).
pub mod row_time_text {
    use super::*;

    /// `RowTimeText.Format(kind, at, culture, say)`, with the reader's zone the C# read from the
    /// machine.
    pub fn format(
        kind: RowTimeKind,
        at: Option<OffsetDateTime>,
        culture: &DateCulture,
        zone: &dyn TimeZone,
        say: &dyn StringCatalog,
    ) -> String {
        match (kind, at) {
            (RowTimeKind::Clock, Some(at)) => culture.format(wall(at, zone), "t"),
            (RowTimeKind::Yesterday, _) => say.get("Yesterday").to_string(),
            (RowTimeKind::Weekday, Some(at)) => culture.format(wall(at, zone), "dddd"),
            (RowTimeKind::Date, Some(at)) => culture.format(wall(at, zone), "d"),
            _ => String::new(),
        }
    }
}

/// What one bubble says: who, the words, and when (`BubbleText`).
pub mod bubble_text {
    use super::*;

    /// The name over the words. "You" for this reader's own, "Deleted account" for an author whose
    /// account is gone — a name this client supplies, because the server keeps the row and not the
    /// person — and the roster's name otherwise.
    pub fn sender(
        bubble: &Bubble,
        chats: &ChatStore,
        say: &dyn StringCatalog,
    ) -> StoreResult<String> {
        if bubble.mine {
            return Ok(say.get("You").to_string());
        }
        Ok(match chats.member(bubble.message.sender_id)? {
            Some(member) if member.deleted => say.get("Deleted account").to_string(),
            Some(member) if !member.display_name.is_empty() => member.display_name,
            _ => String::new(),
        })
    }

    /// The words. A hidden bubble says that something was said and not what; a call reads as a
    /// call and a caption-less attachment as what it is — the same line the chat list draws — and
    /// everything else is the WHOLE body, where the list shows only its first line.
    pub fn words(bubble: &Bubble, list: &ChatListModel, say: &dyn StringCatalog) -> String {
        if !bubble.reads() {
            return say.get("Hidden — blocked member").to_string();
        }
        let message = &bubble.message;
        if message.call.is_none() && !message.body.is_empty() {
            message.body.clone()
        } else {
            list.preview(message, false)
        }
    }

    /// The clock under the words, and "edited" beside it once the body has changed.
    pub fn when(
        message: &MessageDto,
        culture: &DateCulture,
        zone: &dyn TimeZone,
        say: &dyn StringCatalog,
    ) -> String {
        let clock = times::instant(Some(&message.created_at))
            .and_then(|milliseconds| {
                OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000).ok()
            })
            .map(|at| culture.format(wall(at, zone), "t"))
            .unwrap_or_default();
        if message.edited_at.is_none() {
            clock
        } else {
            format!("{clock} · {}", say.get("edited"))
        }
    }
}

/// An instant as the reader's wall clock shows it.
fn wall(at: OffsetDateTime, zone: &dyn TimeZone) -> PrimitiveDateTime {
    let local = date_culture::local(at, zone);
    PrimitiveDateTime::new(local.date(), local.time())
}
