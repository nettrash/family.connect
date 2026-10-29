//! Today's weather in the daily greeting, from a client's side (docs/protocol.md, "Today's weather,
//! for places the owner chose"; issue #72) — `GreetingWeather.cs`: whether the family screen offers
//! the places field, whether it may be edited now, what the server holds ([`offered`], [`shown`],
//! [`editable`], [`saved`]), and the field as it is being edited ([`PlacesDraft`]).
//!
//! THE OWNER'S, AND ONLY WHERE THE SERVER SAYS SO. The field is drawn for the owner alone — every
//! other AI setting of the family is too, and a member's screen has no assistant card at all — and
//! only where `assistant.greeting_weather` is true. An older server sends no such key, which reads
//! as false: no field, rather than a list that does nothing.
//!
//! EDITABLE WHETHER OR NOT THE GREETING IS ON. The list is the family's, kept by the server whatever
//! `ai_greeting` says, and an owner may choose the places first and turn the greeting on afterwards
//! — so the field is editable with the switch off as with it on (protocol.md, "The field is
//! editable whether or not the greeting is on"), as it is on iOS, Android and the web. Only another
//! change of the family's on its way holds it.
//!
//! WHAT IS SHOWN AFTER A SAVE IS THE SERVER'S LIST, from its answer: it may be shorter than what was
//! sent (a repeat, dropped) or spelt with less whitespace, and a server that predates the key keeps
//! nothing at all.
//!
//! The name's rules are `fc_text::greeting_places`' (`typed`, `fold`, `places`, `MAX_PLACES`),
//! called. One difference from the C#, decided for the whole port (linux/docs/carry-forward.md,
//! "Decided while porting": Linux follows `fc-text` where Windows diverged from it): the request is
//! `places`, which drops a repeat ("Moscow", "moscow" → "Moscow", Rust's lower-casing, first
//! spelling kept) before it is sent, where `win/`'s `GreetingPlaces.Check` sent both for the server
//! to drop. The server keeps the same list either way; `GreetingWeatherTests.
//! TheFieldsBecomeWhatTheServerKept` is adapted to it.

use fc_core::protocol::{AssistantDto, FamilyDto};
use fc_text::greeting_places::{MAX_PLACES, Refusal, fold, places, typed};

use crate::app_session::SessionState;

/// Whether this server can put the weather in the greeting at all. A missing key is an older
/// server: no.
pub fn offered(assistant: Option<&AssistantDto>) -> bool {
    assistant.is_some_and(|assistant| assistant.greeting_weather)
}

/// Whether the family screen draws the places field: the owner's screen, on a server that offers
/// it.
pub fn shown(state: &SessionState) -> bool {
    state.is_owner() && offered(state.assistant.as_ref())
}

/// Whether the places may be edited right now: whenever no other change of the family's is on its
/// way — with the greeting on or off.
pub fn editable(idle: bool) -> bool {
    idle
}

/// The places the server holds — none for an absent list (an older server). (A `null` inside one
/// is dropped as the DTO is read.)
pub fn saved(family: Option<&FamilyDto>) -> Vec<String> {
    family
        .and_then(|family| family.greeting_places.clone())
        .unwrap_or_default()
}

/// The places field as it is being edited: up to three text fields, what they would send, and how
/// they follow the server without throwing away what the owner is typing.
///
/// A REDRAW IS NOT A RESET. The family screen redraws on every frame that touches the family, and
/// the server's list only replaces the fields when it CHANGED and the fields hold nothing unsaved;
/// even then, the empty fields at the end — the ones the owner has just added — stay. What is typed
/// while a save is on its way is kept, and is the next save.
///
/// NOTHING IS SENT THAT CHANGES NOTHING, and nothing is sent that the server would refuse: a blank
/// field is no place, and every rule a name must keep is kept as it is typed
/// (`fc_text::greeting_places::typed`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlacesDraft {
    fields: Vec<String>,
    basis: Vec<String>,
}

impl PlacesDraft {
    pub fn new() -> Self {
        PlacesDraft::default()
    }

    /// What each field holds, in order.
    pub fn fields(&self) -> &[String] {
        &self.fields
    }

    /// The list this draft was last measured against: the server's, as far as this screen knows.
    pub fn basis(&self) -> &[String] {
        &self.basis
    }

    /// Whether another field may be added: fewer than three listed.
    pub fn can_add(&self) -> bool {
        self.fields.len() < MAX_PLACES
    }

    /// The fields as a request: folded, the blank ones and the repeats left out — or why the server
    /// would refuse them (`fc_text::greeting_places::places`; with every field typed through
    /// [`set`](Self::set), only a fourth place could be).
    pub fn request(&self) -> Result<Vec<String>, Refusal> {
        places(&self.fields)
    }

    /// Whether the fields ask for something other than what the server holds. A list the server
    /// would refuse is unsaved too.
    pub fn unsaved(&self) -> bool {
        match self.request() {
            Ok(names) => names != self.basis,
            Err(_) => true,
        }
    }

    /// One more, empty, field — while there are fewer than three.
    pub fn add(&mut self) -> bool {
        if !self.can_add() {
            return false;
        }
        self.fields.push(String::new());
        true
    }

    /// Take one field out. An index that is not there changes nothing. (The C#'s `int`, so a
    /// negative index is "not there" too.)
    pub fn remove(&mut self, index: isize) -> bool {
        match usize::try_from(index) {
            Ok(index) if index < self.fields.len() => {
                self.fields.remove(index);
                true
            }
            _ => false,
        }
    }

    /// What a field now holds, as typed — answered as it will be kept
    /// (`fc_text::greeting_places::typed`).
    pub fn set(&mut self, index: isize, text: &str) -> String {
        let kept = typed(text);
        if let Ok(index) = usize::try_from(index)
            && let Some(field) = self.fields.get_mut(index)
        {
            field.clone_from(&kept);
        }
        kept
    }

    /// The server's list, as a redraw found it. A list it has already seen changes nothing; a new
    /// one replaces the fields when they hold nothing unsaved, and otherwise only becomes what the
    /// next save is measured against.
    pub fn sync(&mut self, saved: &[String]) {
        if saved == self.basis.as_slice() {
            return;
        }
        let untouched = !self.unsaved();
        self.basis = saved.to_vec();
        if untouched {
            self.replace(saved);
        }
    }

    /// The list the server KEPT, from its answer to the save of `sent`: what the next save is
    /// measured against, and what the fields become — unless they were changed while the save was on
    /// its way, in which case they hold what the owner typed since, and that is the next save.
    pub fn adopt(&mut self, sent: &[String], kept: &[String]) {
        self.basis = kept.to_vec();
        if self.request().is_ok_and(|names| names == sent) {
            self.replace(kept);
        }
    }

    /// The list to send, or `None` when the fields ask for nothing new — or for something the server
    /// would refuse.
    pub fn pending(&self) -> Option<Vec<String>> {
        self.request().ok().filter(|names| *names != self.basis)
    }

    /// The fields become the server's list — and the empty fields at the end stay, because an empty
    /// field at the end is one the owner has just added and is about to type in.
    fn replace(&mut self, places: &[String]) {
        let trailing = self
            .fields
            .iter()
            .rev()
            .take_while(|field| fold(field).is_empty())
            .count();
        self.fields = places.iter().take(MAX_PLACES).cloned().collect();
        for _ in 0..trailing {
            if !self.add() {
                break;
            }
        }
    }
}
