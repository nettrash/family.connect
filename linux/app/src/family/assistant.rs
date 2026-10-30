//! The owner's settings for the assistant (the Mac's `FamilyAssistantSettings` and
//! `GreetingPlacesSection`; Windows' `DrawAssistant`): the language it answers in, how much of the
//! chat it is shown, whether it may be shown photographs — and the two switches that ride on that —
//! whether members may ask for the text of each other's recordings, whether it may look things up,
//! the daily greeting, and the places whose weather the greeting mentions. Owner endpoints on the
//! server (`PATCH /families/mine`); the dialog draws these for the owner alone.
//!
//! WHAT IS DRAWN IS WHAT THE SERVER WILL HOLD: while a change is on its way the switches show the
//! family as that change will leave it (`FamilyModel::as_applied` — turning photos off turns the
//! two that ride on them off in the same write) and hold still, one change at a time. A switch the
//! server cannot honour is drawn off and insensitive with the reason under it; one that would name
//! nobody (photos on a server that cannot see, lookups with no providers) is not drawn at all.

use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{FamilyDto, FamilyPatch};
use fc_core::text::house_rules;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::app_session::SessionState;
use fc_logic::assistant_consent;
use fc_logic::family::FamilyModel;
use fc_logic::{greeting_weather, lookups};
use fc_text::account::LANGUAGES;
use fc_text::assistant_pictures::MAX_PER_QUESTION;

use super::FamilyDialog;
use crate::settings::ui;
use crate::shell::{Connection, Shell};

/// The groups, in the Mac's order; empty where the server has no assistant.
pub fn groups(
    family: &Rc<FamilyDialog>,
    connection: &Rc<Connection>,
    state: &SessionState,
) -> Vec<adw::PreferencesGroup> {
    let (Some(assistant), Some(held)) = (state.assistant.as_ref(), state.family.as_ref()) else {
        return Vec::new();
    };
    let shell = Shell::get();
    let say: &dyn StringCatalog = shell.say.as_ref();
    let pending = family.assistant_pending.borrow().clone();
    let idle = pending.is_none();
    let shown = match &pending {
        Some(patch) => FamilyModel::as_applied(held, patch),
        None => held.clone(),
    };
    let token = if assistant.mention.is_empty() {
        "@ai".to_string()
    } else {
        assistant.mention.clone()
    };
    let switch = |title: &str, on: bool, sensitive: bool, patch: fn(bool) -> FamilyPatch| {
        let row = adw::SwitchRow::builder()
            .title(title)
            .active(on)
            .sensitive(idle && sensitive)
            .build();
        let (weak, connection) = (Rc::downgrade(family), Rc::clone(connection));
        row.connect_active_notify(move |row| {
            if let Some(family) = weak.upgrade() {
                family.change_assistant(&connection, patch(row.is_active()));
            }
        });
        row
    };
    let mut groups = Vec::new();

    // ---- the language ------------------------------------------------------------------------
    let language = adw::PreferencesGroup::builder()
        .title(say.get("Assistant language"))
        .build();
    let mut names = vec![say.get("Not set").to_string()];
    names.extend(LANGUAGES.iter().map(|(_, name)| (*name).to_string()));
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let answers = adw::ComboRow::builder()
        .title(say.get("Answers in"))
        .model(&gtk::StringList::new(&names))
        .sensitive(idle)
        .build();
    let at = shown
        .language
        .as_deref()
        .and_then(|tag| {
            LANGUAGES
                .iter()
                .position(|(known, _)| known.eq_ignore_ascii_case(tag))
        })
        .map_or(0, |at| at + 1);
    answers.set_selected(u32::try_from(at).unwrap_or(0));
    {
        let (weak, connection) = (Rc::downgrade(family), Rc::clone(connection));
        answers.connect_selected_notify(move |row| {
            let chosen = usize::try_from(row.selected())
                .ok()
                .and_then(|at| at.checked_sub(1))
                .and_then(|at| LANGUAGES.get(at))
                .map(|(tag, _)| (*tag).to_string());
            let patch = match chosen {
                Some(tag) => FamilyPatch {
                    language: Some(tag),
                    ..FamilyPatch::default()
                },
                // "Not set" is a real choice and not English: it clears the family's language.
                None => FamilyPatch {
                    clears_language: true,
                    ..FamilyPatch::default()
                },
            };
            if let Some(family) = weak.upgrade() {
                family.change_assistant(&connection, patch);
            }
        });
    }
    language.add(&answers);
    language.add(&ui::footnote(&say.format(
        "The language %@ answers in when it is asked in the family chat or a group. It is not this app's language — that follows the device. With none chosen, it answers in the language of whoever asked.",
        &[&token],
    )));
    groups.push(language);

    // ---- what it is shown ----------------------------------------------------------------------
    let history = adw::PreferencesGroup::builder()
        .title(say.get("Assistant"))
        .build();
    history.add(&switch(
        say.get("Sees recent history"),
        shown.ai_history,
        true,
        |on| FamilyPatch {
            ai_history: Some(on),
            ..FamilyPatch::default()
        },
    ));
    history.add(&ui::footnote(&say.format(
        "With this on, mentioning %@ in the family chat or a group sends the last month of that chat to the assistant, so it can answer questions about what was said earlier. With it off, only the message that mentions it is sent.",
        &[&token],
    )));
    groups.push(history);

    // ---- pictures ------------------------------------------------------------------------------
    let pictures = adw::PreferencesGroup::builder()
        .title(say.get("Pictures"))
        .build();
    // ABSENT on a server that cannot see: a switch that could never do anything would promise that
    // photographs are one click from leaving.
    if assistant.vision {
        pictures.add(&switch(
            say.get("Can be shown photos"),
            shown.ai_vision,
            true,
            |on| FamilyPatch {
                ai_vision: Some(on),
                ..FamilyPatch::default()
            },
        ));
        pictures.add(&ui::footnote(&say.format(
            "With this on, a photo is sent to the model your server is set up to use when a member attaches it to a question in their own chat with the assistant, attaches it to an %@ message in the family chat or a group, or replies to a photo with %@ — never a photo the assistant was not pointed at, never from an earlier message unless Recent photos is on, and never a video, file or place. With it off, no photo is ever sent.",
            &[&token, &token],
        )));
    }
    // The two that ride on vision: present, and inert-but-explained until it is on.
    let pictures_on = assistant.vision && shown.ai_vision;
    let max = MAX_PER_QUESTION as i64;
    pictures.add(&switch(
        say.get("Recent photos"),
        shown.ai_history_photos,
        pictures_on && FamilyModel::may_turn_on("ai_history_photos", &shown),
        |on| FamilyPatch {
            ai_history_photos: Some(on),
            ..FamilyPatch::default()
        },
    ));
    pictures.add(&ui::footnote(&with_note(
        say.format(
            "With this on, whenever anyone mentions %@ in the family chat or a group, the most recent photos in that chat — up to %lld, from anyone, that nobody pointed the assistant at — also go to the model your server is set up to use, after any photo on the message itself or on the one it replies to. Nearly every mention then sends pictures, which costs more. It is off unless you turn it on.",
            &[&token, &max],
        ),
        house_rules::pictures_note(
            assistant.vision,
            shown.ai_vision,
            shown.ai_history,
            say.get("While Sees recent history is off this does nothing: the chat's history isn't sent, so no photo from it is either."),
            say,
        ),
    )));
    pictures.add(&switch(
        say.get("Member faces"),
        shown.ai_faces,
        pictures_on && FamilyModel::may_turn_on("ai_faces", &shown),
        |on| FamilyPatch {
            ai_faces: Some(on),
            ..FamilyPatch::default()
        },
    ));
    pictures.add(&ui::footnote(&with_note(
        say.format(
            "With this on, whenever anyone mentions %@ in the family chat or a group, the profile pictures of the members named in that chat's recent history — up to %lld — also go to the model your server is set up to use, so it can tell who is who. They are the pictures members chose for themselves, not photos anyone attached; never a member who has left, and never anyone outside this family. Most mentions then send pictures, which costs more. It is off unless you turn it on; with it off, no face is ever sent.",
            &[&token, &max],
        ),
        house_rules::pictures_note(
            assistant.vision,
            shown.ai_vision,
            shown.ai_history,
            say.get("While Sees recent history is off this does nothing: no names are sent, so no faces are either."),
            say,
        ),
    )));
    groups.push(pictures);

    // ---- recordings as text ----------------------------------------------------------------------
    // It names who receives the sound, so only where the server names a processor.
    if let Some(processor) = assistant
        .processor
        .as_deref()
        .filter(|processor| assistant_consent::is_available(Some(processor)))
    {
        let recordings = adw::PreferencesGroup::new();
        recordings.add(&switch(
            say.get("Voice and video as text"),
            shown.ai_transcripts,
            assistant.transcribe,
            |on| FamilyPatch {
                ai_transcripts: Some(on),
                ..FamilyPatch::default()
            },
        ));
        recordings.add(&ui::footnote(&with_note(
            say.format(
                "With this on, members can ask for the text of other members' voice notes, audio and videos in the family chat, and that recording's sound is then sent to %@. It is sent only when someone asks, and only if the member who sent it has agreed to the assistant. Everyone can get the text of their own recordings without this. It is off unless you turn it on.",
                &[&processor],
            ),
            (!assistant.transcribe).then(|| {
                say.get("Not available here: this server can't turn recordings into text.")
                    .to_string()
            }),
        )));
        groups.push(recordings);
    }

    // ---- looking things up -------------------------------------------------------------------------
    // Only where the server names its providers: the footnote has to name who receives a query.
    if let Some(providers) =
        lookups::providers(Some(assistant)).filter(|_| lookups::offered(Some(assistant)))
    {
        let looking = adw::PreferencesGroup::builder()
            .title(say.get("Looking things up"))
            .build();
        looking.add(&switch(
            say.get("Can look things up"),
            shown.ai_lookups,
            true,
            |on| FamilyPatch {
                ai_lookups: Some(on),
                ..FamilyPatch::default()
            },
        ));
        looking.add(&ui::footnote(&lookups::switch_footnote(&providers, say)));
        groups.push(looking);
    }

    // ---- the daily greeting ---------------------------------------------------------------------
    let greeting = adw::PreferencesGroup::builder()
        .title(say.get("Daily greeting"))
        .build();
    greeting.add(&switch(
        say.get("Good morning message"),
        shown.ai_greeting,
        state.greetings_enabled,
        |on| FamilyPatch {
            ai_greeting: Some(on),
            ..FamilyPatch::default()
        },
    ));
    greeting.add(&ui::footnote(&with_note(
        say.get("With this on, the assistant posts one short good-morning message into the family chat each day, mentioning the star signs of the birthdays your family has set. It never sends anyone's name or birth date, only the signs; it makes no claims about the date; and it never sounds a notification — it is simply there when you next open the chat.")
            .to_string(),
        (!state.greetings_enabled).then(|| {
            say.get("Not available here: this server doesn't post daily greetings.")
                .to_string()
        }),
    )));
    groups.push(greeting);

    if greeting_weather::shown(state) {
        groups.push(places(family, connection, held, idle));
    }
    groups
}

/// "Weather in the greeting": up to three place names, saved on Enter, on leaving a field, on a
/// removal and when the dialog closes — the whole list, which REPLACES the stored one; what is
/// drawn after is the list the server kept.
fn places(
    family: &Rc<FamilyDialog>,
    connection: &Rc<Connection>,
    held: &FamilyDto,
    idle: bool,
) -> adw::PreferencesGroup {
    let shell = Shell::get();
    let say: &dyn StringCatalog = shell.say.as_ref();
    family
        .places
        .borrow_mut()
        .sync(&greeting_weather::saved(Some(held)));
    let editable = greeting_weather::editable(idle || family.saving_places.get());
    let group = adw::PreferencesGroup::builder()
        .title(say.get("Weather in the greeting"))
        .build();
    let fields: Vec<String> = family.places.borrow().fields().to_vec();
    for (index, text) in fields.iter().enumerate() {
        let at = isize::try_from(index).unwrap_or(-1);
        let row = adw::EntryRow::builder()
            .title(say.get("City or town"))
            .text(text)
            .sensitive(editable)
            .show_apply_button(false)
            .build();
        {
            let weak = Rc::downgrade(family);
            row.connect_changed(move |row| {
                let Some(family) = weak.upgrade() else {
                    return;
                };
                let typed = row.text().to_string();
                let kept = family.places.borrow_mut().set(at, &typed);
                if kept != typed {
                    // A rule the name must keep (`greeting_places::typed`): the field says it at once.
                    row.set_text(&kept);
                    row.set_position(-1);
                }
            });
        }
        {
            let (weak, connection) = (Rc::downgrade(family), Rc::clone(connection));
            row.connect_entry_activated(move |_| {
                if let Some(family) = weak.upgrade() {
                    family.save_places(&connection);
                }
            });
        }
        {
            let focus = gtk::EventControllerFocus::new();
            let (weak, connection) = (Rc::downgrade(family), Rc::clone(connection));
            focus.connect_leave(move |_| {
                if let Some(family) = weak.upgrade() {
                    family.save_places(&connection);
                }
            });
            row.add_controller(focus);
        }
        let remove = gtk::Button::builder()
            .icon_name("list-remove-symbolic")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .tooltip_text(say.get("Remove place"))
            .sensitive(editable)
            .build();
        remove.update_property(&[gtk::accessible::Property::Label(say.get("Remove place"))]);
        {
            let (weak, connection) = (Rc::downgrade(family), Rc::clone(connection));
            remove.connect_clicked(move |_| {
                if let Some(family) = weak.upgrade() {
                    family.places.borrow_mut().remove(at);
                    family.save_places(&connection);
                }
            });
        }
        row.add_suffix(&remove);
        group.add(&row);
    }
    if family.places.borrow().can_add() {
        let weak = Rc::downgrade(family);
        let add = ui::button_row(say.get("Add place"), false, move || {
            if let Some(family) = weak.upgrade() {
                family.places.borrow_mut().add();
                family.rebuild();
            }
        });
        add.set_sensitive(editable);
        group.add(&add);
    } else {
        group.add(&ui::footnote(say.get("Up to 3 places.")));
    }
    group.add(&ui::footnote(say.get(
        "The daily greeting will also mention today's weather in these places. Only the place names are sent to Open-Meteo to fetch the forecast. Nothing else is sent.",
    )));
    group
}

/// A switch's footnote with its reason after it, where it has one.
fn with_note(text: String, note: Option<String>) -> String {
    match note {
        Some(note) => format!("{text}\n\n{note}"),
        None => text,
    }
}

impl FamilyDialog {
    /// One change to the assistant's settings: drawn as the server will apply it while it is on its
    /// way, every control held still, then the family read back and drawn as the server holds it.
    /// A refusal is said (`house_rules::assistant_failure`). One at a time: a change while another
    /// is on its way is dropped and the controls snap back to the truth.
    pub(super) fn change_assistant(
        self: &Rc<Self>,
        connection: &Rc<Connection>,
        patch: FamilyPatch,
    ) {
        if self.assistant_pending.borrow().is_some() || self.drawing.get() {
            return;
        }
        let Some(held) = connection.session.state().family else {
            return;
        };
        *self.assistant_pending.borrow_mut() = Some(patch.clone());
        self.redraw_soon();
        let (weak, connection) = (Rc::downgrade(self), Rc::clone(connection));
        ui::spawn(async move {
            let answer = super::family_model(&connection).change(&held, &patch).await;
            if let Err(error) = &answer {
                crate::fc_info!("family: assistant change refused ({})", error.code);
            }
            if let Err(error) = connection.session.refresh_family().await {
                crate::fc_warn!(
                    "family: the cache refused ({})",
                    crate::services::log::store_error(&error)
                );
            }
            if let Some(me) = weak.upgrade() {
                me.assistant_pending.borrow_mut().take();
                if let Err(error) = answer {
                    let say = Shell::get().say.clone();
                    ui::toast(
                        &me.toasts,
                        &house_rules::assistant_failure(&error.code, say.as_ref()),
                    );
                }
                me.rebuild();
            }
        });
    }

    /// Sends the greeting's places when they ask for something new — and only then; what the
    /// server KEPT is what the fields become (`PlacesDraft::adopt`). Never logged: a place name is
    /// the owner's words.
    pub(super) fn save_places(self: &Rc<Self>, connection: &Rc<Connection>) {
        if self.drawing.get() {
            return;
        }
        if self.assistant_pending.borrow().is_some() || self.saving_places.get() {
            return;
        }
        let Some(held) = connection.session.state().family else {
            return;
        };
        let Some(request) = self.places.borrow().pending() else {
            self.redraw_soon();
            return;
        };
        let patch = FamilyPatch {
            greeting_places: Some(request.clone()),
            ..FamilyPatch::default()
        };
        self.saving_places.set(true);
        *self.assistant_pending.borrow_mut() = Some(patch.clone());
        let (weak, connection) = (Rc::downgrade(self), Rc::clone(connection));
        ui::spawn(async move {
            let answer = super::family_model(&connection).change(&held, &patch).await;
            if let Err(error) = connection.session.refresh_family().await {
                crate::fc_warn!(
                    "family: the cache refused ({})",
                    crate::services::log::store_error(&error)
                );
            }
            let Some(me) = weak.upgrade() else {
                return;
            };
            me.saving_places.set(false);
            me.assistant_pending.borrow_mut().take();
            match answer {
                Ok(family) => {
                    me.places
                        .borrow_mut()
                        .adopt(&request, &greeting_weather::saved(Some(&family)));
                    crate::fc_info!("family: greeting places saved");
                }
                Err(error) => {
                    crate::fc_info!("family: greeting places refused ({})", error.code);
                    let say = Shell::get().say.clone();
                    ui::toast(
                        &me.toasts,
                        &house_rules::assistant_failure(&error.code, say.as_ref()),
                    );
                }
            }
            me.rebuild();
        });
    }

    /// A redraw after the handler that asked for it has returned — a control is never torn down
    /// from inside its own signal.
    fn redraw_soon(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(me) = weak.upgrade() {
                me.rebuild();
            }
        });
    }
}
