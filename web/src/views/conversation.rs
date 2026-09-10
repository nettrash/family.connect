//! One chat: its messages, and the box to add to them.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use fc_text::media;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, File};
use yew::prelude::*;

use crate::actions::Action;
use crate::live::Opening;
use crate::location;
use crate::model::{Assistant, ChatListItem, Family, Member, Message};
use crate::prep;
use crate::recorder::Recording;
use crate::staged::Prepared;
use crate::store::Draft;
use crate::time;
use crate::timeline::{self, Context};
use crate::views::attach::{
    drag_carries_something, dropped_files, dropped_links, read_clipboard, AttachMenu, Clip,
    RecordingBar, StagingStrip,
};
use crate::views::bubble::Bubble;
use crate::views::composer::{resolve_mentions, Composer, Editing, Pictures, Replying};
use crate::views::poll::PollComposer;
use crate::views::report::{ReportDialog, ReportTarget};
use fc_text::assistant_pictures::{self, Candidate};

#[derive(Properties, PartialEq)]
pub struct ConversationProps {
    pub item: ChatListItem,
    pub messages: Vec<Message>,
    pub my_user_id: i64,
    /// userId → the name to draw. A sender who is not in the roster reads
    /// as "Someone" rather than as a number.
    pub names: HashMap<i64, String>,
    pub members: Vec<Member>,
    pub assistant: Option<Assistant>,
    pub blocked: HashSet<i64>,
    pub revealed: HashSet<i64>,
    /// Quote levels of a blocked member peeked at: (message, 0 the quote or
    /// 1 its parent).
    pub revealed_quotes: HashSet<(i64, u8)>,
    /// This chat's unread state once its messages were in — None while they
    /// are loading (`AppState::opening`).
    pub opening: Option<Opening>,
    /// This device's messages that will not be sent unless somebody asks,
    /// by `client_msg_id`, with why.
    pub failed: HashMap<String, String>,
    pub ai_failed: HashSet<i64>,
    /// The peer's read marker, in a direct chat.
    pub peer_read: i64,
    /// Who is typing here, already resolved to names.
    pub typing: Vec<String>,
    /// There is older history to fetch.
    pub can_load_more: bool,
    /// Open polls in this chat the reader has not voted in.
    pub unanswered_polls: usize,
    /// What was being typed here when the reader last left.
    pub draft: String,
    /// What is staged to go with the next message.
    #[prop_or_default]
    pub staged: Vec<Prepared>,
    /// The reader's family — whose switches decide what may go to the
    /// assistant.
    #[prop_or_default]
    pub family: Option<Family>,
    pub support_contact: Option<String>,
    pub on_action: Callback<Action>,
    /// Wall-clock now, for the day pills.
    pub now_ms: f64,
}

/// How close to the bottom still counts as "reading the newest", in
/// pixels. A reader a line or two up is still following along; one who has
/// scrolled a screen away is reading history and must not be yanked down.
const PINNED_SLACK_PX: i32 = 48;

/// How long a jumped-to message stays tinted, in milliseconds — the apps'
/// 1.6 s.
const HIGHLIGHT_MS: u32 = 1_600;

/// Whether the list is scrolled to its newest message, give or take the
/// slack.
fn is_at_bottom(element: &Element) -> bool {
    element.scroll_top() + element.client_height() >= element.scroll_height() - PINNED_SLACK_PX
}

/// A row's identity across renders: my own messages by `client_msg_id`, so
/// the bubble drawn before the server numbered it IS the bubble after (the
/// server keeps that id unique per sender and chat), and everybody else's
/// by server id. Keyed, "Earlier messages" inserts rows above the ones on
/// screen instead of rewriting every row in place — which is what lets the
/// browser keep the reader where they were.
pub fn row_key(message: &Message, my_user_id: i64) -> String {
    match &message.client_msg_id {
        Some(client_msg_id) if message.sender_id == my_user_id => format!("c{client_msg_id}"),
        _ => format!("m{}", message.id),
    }
}

#[function_component(Conversation)]
pub fn conversation(props: &ConversationProps) -> Html {
    let chat_id = props.item.chat.id;
    let list = use_node_ref();
    // Whether the reader is at the newest message. Starts true, so the rows
    // arriving while the chat loads stay at the bottom; the scroll handler
    // keeps it honest after that.
    let pinned = use_mut_ref(|| true);
    // Where this chat OPENED: None until decided, then the first unread row
    // and the count to draw over it — or no anchor, for the newest. Decided
    // ONCE, when the messages are in and from the unread state as it stood
    // before anything read it (`AppState::opening`), so the divider does
    // not walk while they read (ios UnreadAnchor). The pane is keyed by
    // chat, so a new chat is a new decision.
    let opened = use_state(|| Option::<Option<(i64, i64)>>::None);
    {
        let opened = opened.clone();
        let messages = props.messages.clone();
        let me = props.my_user_id;
        let opening = props.opening;
        use_effect_with((opening, props.messages.len()), move |_| {
            if opened.is_some() {
                return;
            }
            let Some(opening) = opening else {
                return;
            };
            opened.set(Some(
                timeline::open_anchor(
                    &messages,
                    opening.unread_count,
                    opening.last_read_message_id,
                    me,
                )
                .map(|id| (id, opening.unread_count)),
            ));
        });
    }
    let anchor = (*opened).flatten();

    let replying = use_state(|| Option::<i64>::None);
    let editing = use_state(|| Option::<i64>::None);
    // The message being edited NOW, for an answer that lands after the
    // render that asked: a handle reads what it was when it was made.
    let editing_now = use_mut_ref(|| Option::<i64>::None);
    *editing_now.borrow_mut() = *editing;
    let report = use_state(|| Option::<ReportTarget>::None);
    let poll_open = use_state(|| false);
    let highlight = use_state(|| Option::<i64>::None);
    let at_newest = use_state(|| true);

    // OPENING: at the unread divider when there is one, otherwise at the
    // newest — once, after the rows are in the DOM. Then, and only then,
    // the view says whether its reader is at the newest message, which is
    // what lets the chat be read at all: a divider two screens up leaves
    // the newest below the fold, and the chat unread until they get there.
    {
        let list = list.clone();
        let pinned = pinned.clone();
        let at_newest = at_newest.clone();
        let on_action = props.on_action.clone();
        use_effect_with(*opened, move |opened| {
            let Some(anchor) = opened else {
                return;
            };
            let Some(element) = list.cast::<Element>() else {
                return;
            };
            let divider = anchor.and_then(|_| {
                web_sys::window()?
                    .document()?
                    .get_element_by_id("unread-divider")
            });
            match divider {
                Some(divider) => divider.scroll_into_view(),
                None => element.set_scroll_top(element.scroll_height()),
            }
            let bottom = is_at_bottom(&element);
            *pinned.borrow_mut() = bottom;
            at_newest.set(bottom);
            on_action.emit(Action::AtNewest(bottom));
        });
    }

    // FOLLOW THE CONVERSATION — after the DOM has the new message, and only
    // for a reader who was already at the bottom. A message landing must
    // not scroll somebody reading history, and must not leave somebody
    // following along one message short. An effect rather than a call
    // beside the state change: a call there runs BEFORE the re-render.
    {
        let list = list.clone();
        let pinned = pinned.clone();
        let newest = props.messages.last().map(|message| {
            (
                message.id,
                message.client_msg_id.clone(),
                message.body.len(),
            )
        });
        use_effect_with((props.messages.len(), newest), move |_| {
            if *pinned.borrow() {
                if let Some(element) = list.cast::<Element>() {
                    element.set_scroll_top(element.scroll_height());
                }
            }
        });
    }

    // EARLIER MESSAGES land above the reader, and what they were reading
    // stays where it was on the screen: the scroll moves down by exactly
    // what was added above. Done here rather than left to the browser's
    // scroll anchoring, which Safari does not do (styles.css).
    {
        let list = list.clone();
        let pinned = pinned.clone();
        let before = use_mut_ref(|| (None::<i64>, 0));
        let oldest = props
            .messages
            .iter()
            .map(|message| message.id)
            .find(|id| *id != 0);
        use_effect_with((props.messages.len(), oldest), move |(_, oldest)| {
            let Some(element) = list.cast::<Element>() else {
                return;
            };
            let height = element.scroll_height();
            let (was_oldest, was_height) = *before.borrow();
            if let (Some(now), Some(was)) = (*oldest, was_oldest) {
                if now < was && !*pinned.borrow() {
                    element.set_scroll_top(element.scroll_top() + height - was_height);
                }
            }
            *before.borrow_mut() = (*oldest, height);
        });
    }

    let on_scroll = {
        let list = list.clone();
        let pinned = pinned.clone();
        let at_newest = at_newest.clone();
        let on_action = props.on_action.clone();
        // Until the opening is decided the scroll is the loading rows being
        // followed, not a reader arriving anywhere.
        let decided = opened.is_some();
        Callback::from(move |_: Event| {
            if let Some(element) = list.cast::<Element>() {
                let bottom = is_at_bottom(&element);
                *pinned.borrow_mut() = bottom;
                if *at_newest != bottom {
                    at_newest.set(bottom);
                    if decided {
                        on_action.emit(Action::AtNewest(bottom));
                    }
                }
            }
        })
    };

    let jump_to_newest = {
        let list = list.clone();
        let pinned = pinned.clone();
        Callback::from(move |_: MouseEvent| {
            *pinned.borrow_mut() = true;
            if let Some(element) = list.cast::<Element>() {
                element.set_scroll_top(element.scroll_height());
            }
        })
    };

    // A quote's click: show the quoted message, tinted for a moment — if it
    // is here at all.
    let on_jump = {
        let highlight = highlight.clone();
        let pinned = pinned.clone();
        Callback::from(move |id: i64| {
            let Some(target) = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.get_element_by_id(&format!("m-{id}")))
            else {
                return;
            };
            *pinned.borrow_mut() = false;
            target.scroll_into_view();
            highlight.set(Some(id));
            let highlight = highlight.clone();
            gloo_timers::callback::Timeout::new(HIGHLIGHT_MS, move || highlight.set(None)).forget();
        })
    };

    let is_family = props.item.chat.is_family();
    let is_ai = props.item.chat.is_ai();
    let assistant_user_id = props.assistant.as_ref().map(|assistant| assistant.user_id);
    let context = Context {
        my_user_id: props.my_user_id,
        is_family_chat: is_family,
        is_ai_chat: is_ai,
        assistant_user_id,
        blocked: &props.blocked,
        first_unread_id: anchor.map(|(id, _)| id),
        peer_read_up_to: props.peer_read,
    };
    let rows = timeline::rows(&props.messages, &context);
    // In the assistant's own chat, anything not mine is the assistant's —
    // drawn with the chat's own name, not looked up in a roster it is not in.
    let mut names = props.names.clone();
    if is_ai {
        if let Some(title) = props.item.chat.title.clone() {
            for message in &props.messages {
                if message.sender_id != props.my_user_id {
                    names.insert(message.sender_id, title.clone());
                }
            }
        }
    }

    let on_reply = {
        let replying = replying.clone();
        let editing = editing.clone();
        Callback::from(move |id: i64| {
            editing.set(None);
            replying.set(Some(id));
        })
    };
    let on_edit = {
        let replying = replying.clone();
        let editing = editing.clone();
        Callback::from(move |id: i64| {
            replying.set(None);
            editing.set(Some(id));
        })
    };
    let on_report = {
        let report = report.clone();
        let names = names.clone();
        Callback::from(move |(user_id, message_id): (i64, Option<i64>)| {
            report.set(Some(ReportTarget {
                user_id,
                name: names
                    .get(&user_id)
                    .cloned()
                    .unwrap_or_else(|| "Someone".to_string()),
                message_id,
            }));
        })
    };

    let replying_to = replying.and_then(|id| {
        let message = props.messages.iter().find(|message| message.id == id)?;
        let hidden = timeline::is_hidden_by_block(message, props.my_user_id, &props.blocked)
            && !props.revealed.contains(&id);
        Some(Replying {
            message_id: id,
            name: names
                .get(&message.sender_id)
                .cloned()
                .unwrap_or_else(|| "Someone".to_string()),
            excerpt: if hidden {
                String::new()
            } else {
                crate::store::excerpt(&message.body)
            },
        })
    });
    let edit_banner = editing.and_then(|id| {
        let message = props.messages.iter().find(|message| message.id == id)?;
        Some(Editing {
            message_id: id,
            body: message.body.clone(),
        })
    });

    let on_send = {
        let on_action = props.on_action.clone();
        let replying = replying.clone();
        let pinned = pinned.clone();
        let staged = props.staged.clone();
        Callback::from(move |mut draft: Draft| {
            draft.reply_to_message_id = *replying;
            // Whatever is staged goes with it, the words as its caption.
            draft.attachments = staged.clone();
            replying.set(None);
            // Your own message is always shown, wherever you were: a send
            // that disappeared below the fold reads as a send that did not
            // happen.
            *pinned.borrow_mut() = true;
            on_action.emit(Action::Send { chat_id, draft });
        })
    };
    // Edit mode ends when the server has the edit, not when Save is
    // pressed: a refused or lost edit keeps the words in the box (ios
    // MacConversationView clears only once `edit` answers true).
    let on_save_edit = {
        let on_action = props.on_action.clone();
        let editing = editing.clone();
        let editing_now = editing_now.clone();
        Callback::from(move |(message_id, body): (i64, String)| {
            let done = {
                let editing = editing.clone();
                let editing_now = editing_now.clone();
                Callback::from(move |_: ()| {
                    if *editing_now.borrow() == Some(message_id) {
                        editing.set(None);
                    }
                })
            };
            on_action.emit(Action::SaveEdit {
                chat_id,
                message_id,
                body,
                done,
            });
        })
    };

    // --- The attachment doors (the Mac's attach menu, drop and paste) ----
    let preparing = use_state(|| false);
    let locating = use_state(|| false);
    let media_notice = use_state(|| Option::<String>::None);
    let append = use_state(|| (0u32, String::new()));
    let take = use_state(|| 0u32);
    // The location waiting for the draft to be its caption.
    let pending = use_mut_ref(|| Option::<Prepared>::None);
    let recording: Rc<RefCell<Option<Recording>>> = use_mut_ref(|| None);
    // When the recording on the bar started — Some while one is running.
    let recording_since = use_state(|| Option::<f64>::None);
    // The microphone asked for and not answered yet: busy, so a second
    // start cannot race the first.
    let starting = use_state(|| false);
    // A hunt for a fix — the permission prompt included — so a second click
    // does not start a second one.
    let hunting = use_mut_ref(|| false);
    // Whether this pane is still on screen. Work that outlives it — a batch
    // being prepared, a recording finishing, a microphone being granted —
    // must not land in whatever is on screen next: after a sign-out that
    // is SOMEBODY ELSE'S composer.
    let alive = use_mut_ref(|| true);
    let dropping = use_state(|| false);
    let drop_depth = use_mut_ref(|| 0i32);
    let recording_on = recording_since.is_some() || *starting;

    // Which busy it is, because the two have different ways out
    // (MacConversationView.composerBusyNotice).
    let busy_reason = if editing.is_some() {
        Some("Finish editing before attaching something.".to_string())
    } else if *preparing || *locating || recording_on {
        Some("Wait until the current attachment is done.".to_string())
    } else {
        None
    };
    let notice = {
        let media_notice = media_notice.clone();
        Callback::from(move |text: String| media_notice.set(Some(text)))
    };
    // THE way files come in, whatever door they used: prepared one at a
    // time, in order, and stopped at the cap rather than preparing the rest
    // only to throw them away.
    let ingest = {
        let on_action = props.on_action.clone();
        let preparing = preparing.clone();
        let media_notice = media_notice.clone();
        let staged = props.staged.len();
        let busy = busy_reason.clone();
        let alive = alive.clone();
        Callback::from(move |files: Vec<File>| {
            if files.is_empty() {
                return;
            }
            if let Some(reason) = busy.clone() {
                media_notice.set(Some(reason));
                return;
            }
            preparing.set(true);
            media_notice.set(Some("Preparing…".to_string()));
            let on_action = on_action.clone();
            let preparing = preparing.clone();
            let media_notice = media_notice.clone();
            let alive = alive.clone();
            spawn_local(async move {
                let mut count = staged;
                let mut said = None;
                for file in files {
                    if !*alive.borrow() {
                        return;
                    }
                    if !media::can_stage(count) {
                        said = Some(format!(
                            "You can attach up to {} items.",
                            media::MAX_PER_MESSAGE
                        ));
                        break;
                    }
                    let prepared = prep::prepare(&file).await;
                    if !*alive.borrow() {
                        return;
                    }
                    match prepared {
                        Ok(item) => {
                            on_action.emit(Action::Stage { chat_id, item });
                            count += 1;
                        }
                        Err(error) => said = Some(error.message().to_string()),
                    }
                }
                preparing.set(false);
                media_notice.set(said);
            });
        })
    };
    let append_text = {
        let append = append.clone();
        Callback::from(move |text: String| append.set((append.0 + 1, text)))
    };
    let on_paste_menu = {
        let ingest = ingest.clone();
        let append_text = append_text.clone();
        let notice = notice.clone();
        Callback::from(move |_: ()| {
            let ingest = ingest.clone();
            let append_text = append_text.clone();
            let notice = notice.clone();
            spawn_local(async move {
                match read_clipboard().await {
                    Clip::Files(files) => ingest.emit(files),
                    Clip::Text(text) => append_text.emit(text),
                    Clip::Nothing => notice.emit("There's nothing to paste.".to_string()),
                    Clip::Denied => notice.emit(
                        "This browser didn't let Family read the clipboard. Paste with ⌘V or Ctrl+V instead."
                            .to_string(),
                    ),
                }
            });
        })
    };
    // ⌘V anywhere on the page but the box, which handles its own.
    {
        let doors = use_mut_ref(|| (ingest.clone(), append_text.clone()));
        *doors.borrow_mut() = (ingest.clone(), append_text.clone());
        use_effect_with((), move |_| {
            let listener = Closure::<dyn Fn(web_sys::ClipboardEvent)>::new(
                move |event: web_sys::ClipboardEvent| {
                    let in_a_field = event
                        .target()
                        .and_then(|target| target.dyn_into::<Element>().ok())
                        .is_some_and(|element| {
                            matches!(element.tag_name().as_str(), "TEXTAREA" | "INPUT")
                        });
                    if in_a_field {
                        return;
                    }
                    let Some(data) = event.clipboard_data() else {
                        return;
                    };
                    let files: Vec<File> = data
                        .files()
                        .map(|list| {
                            (0..list.length())
                                .filter_map(|index| list.get(index))
                                .collect()
                        })
                        .unwrap_or_default();
                    let names: Vec<String> = files.iter().map(File::name).collect();
                    let text = data.get_data("text/plain").unwrap_or_default();
                    let (ingest, append_text) = doors.borrow().clone();
                    match media::paste_decision(&names, &text) {
                        media::PasteDecision::Attach => {
                            event.prevent_default();
                            ingest.emit(files);
                        }
                        media::PasteDecision::Type => {
                            event.prevent_default();
                            append_text.emit(text);
                        }
                        media::PasteDecision::Nothing => {}
                    }
                },
            );
            let document = web_sys::window().and_then(|window| window.document());
            if let Some(document) = &document {
                let _ = document
                    .add_event_listener_with_callback("paste", listener.as_ref().unchecked_ref());
            }
            move || {
                if let Some(document) = document {
                    let _ = document.remove_event_listener_with_callback(
                        "paste",
                        listener.as_ref().unchecked_ref(),
                    );
                }
            }
        });
    }
    // A voice note: staged when it stops, so a caption can be added.
    let stop_recording = {
        let recording = recording.clone();
        let recording_since = recording_since.clone();
        let on_action = props.on_action.clone();
        let media_notice = media_notice.clone();
        let staged = props.staged.len();
        let alive = alive.clone();
        Callback::from(move |_: ()| {
            let Some(active) = recording.borrow_mut().take() else {
                return;
            };
            recording_since.set(None);
            let on_action = on_action.clone();
            let media_notice = media_notice.clone();
            let alive = alive.clone();
            spawn_local(async move {
                let recorded = active.stop().await;
                if !*alive.borrow() {
                    return;
                }
                let Some(recorded) = recorded else {
                    media_notice.set(Some("That recording was too short.".to_string()));
                    return;
                };
                if !media::can_stage(staged) {
                    media_notice.set(Some(format!(
                        "You can attach up to {} items.",
                        media::MAX_PER_MESSAGE
                    )));
                    return;
                }
                let prepared =
                    prep::recording(recorded.blob, recorded.mime, recorded.duration_ms).await;
                if !*alive.borrow() {
                    return;
                }
                match prepared {
                    Ok(item) => on_action.emit(Action::Stage { chat_id, item }),
                    Err(error) => media_notice.set(Some(error.message().to_string())),
                }
            });
        })
    };
    let cancel_recording = {
        let recording = recording.clone();
        let recording_since = recording_since.clone();
        Callback::from(move |_: ()| {
            if let Some(active) = recording.borrow_mut().take() {
                active.cancel();
            }
            recording_since.set(None);
        })
    };
    let start_recording = {
        let recording = recording.clone();
        let recording_since = recording_since.clone();
        let starting = starting.clone();
        let notice = notice.clone();
        let alive = alive.clone();
        let busy = busy_reason.clone();
        Callback::from(move |_: ()| {
            if let Some(reason) = busy.clone() {
                notice.emit(reason);
                return;
            }
            starting.set(true);
            let recording = recording.clone();
            let recording_since = recording_since.clone();
            let starting = starting.clone();
            let notice = notice.clone();
            let alive = alive.clone();
            spawn_local(async move {
                let started = Recording::start().await;
                starting.set(false);
                match started {
                    // Granted after the pane went, or beside one already
                    // running: dropped here, and the microphone with it.
                    Ok(active) if !*alive.borrow() || recording.borrow().is_some() => drop(active),
                    Ok(active) => {
                        *recording.borrow_mut() = Some(active);
                        recording_since.set(Some(js_sys::Date::now()));
                    }
                    Err(failure) => notice.emit(failure.message().to_string()),
                }
            });
        })
    };
    // Gone with the pane: nothing it started may land after it, and a
    // microphone left open is a microphone left open.
    {
        let recording = recording.clone();
        let alive = alive.clone();
        use_effect_with((), move |_| {
            move || {
                *alive.borrow_mut() = false;
                if let Some(active) = recording.borrow_mut().take() {
                    active.cancel();
                }
            }
        });
    }
    // Where this browser is, sent at once with the draft as its caption.
    // Busy only once the browser has permission to look: somebody reading
    // the permission prompt must not find the composer bricked while they
    // do (the Mac settles permission first, #41).
    let share_location = {
        let locating = locating.clone();
        let media_notice = media_notice.clone();
        let pending = pending.clone();
        let take = take.clone();
        let counter = use_mut_ref(|| 0u32);
        let hunting = hunting.clone();
        let alive = alive.clone();
        Callback::from(move |_: ()| {
            if *hunting.borrow() {
                return;
            }
            *hunting.borrow_mut() = true;
            let permitted = {
                let locating = locating.clone();
                let media_notice = media_notice.clone();
                Callback::from(move |_: ()| {
                    locating.set(true);
                    media_notice.set(Some("Finding your location…".to_string()));
                })
            };
            let locating = locating.clone();
            let media_notice = media_notice.clone();
            let pending = pending.clone();
            let take = take.clone();
            let counter = counter.clone();
            let hunting = hunting.clone();
            let alive = alive.clone();
            spawn_local(async move {
                let found = location::current_fix(permitted).await;
                *hunting.borrow_mut() = false;
                if !*alive.borrow() {
                    return;
                }
                match found {
                    Ok(fix) => {
                        *pending.borrow_mut() = Some(Prepared::location(
                            fix.latitude,
                            fix.longitude,
                            fix.accuracy_m,
                        ));
                        *counter.borrow_mut() += 1;
                        take.set(*counter.borrow());
                        media_notice.set(None);
                    }
                    Err(error) => media_notice.set(Some(error.message().to_string())),
                }
                locating.set(false);
            });
        })
    };
    let on_take = {
        let on_action = props.on_action.clone();
        let pending = pending.clone();
        let replying = replying.clone();
        let pinned = pinned.clone();
        Callback::from(move |mut draft: Draft| {
            let Some(place) = pending.borrow_mut().take() else {
                return;
            };
            // Alone, as a location always is — whatever is staged stays
            // staged for the next message (MacConversationView.shareLocation).
            draft.reply_to_message_id = *replying;
            draft.attachments = vec![place];
            replying.set(None);
            *pinned.borrow_mut() = true;
            on_action.emit(Action::Send { chat_id, draft });
        })
    };
    // Drag and drop, onto the whole pane.
    let on_drag_enter = {
        let dropping = dropping.clone();
        let drop_depth = drop_depth.clone();
        Callback::from(move |event: DragEvent| {
            if event
                .data_transfer()
                .is_some_and(|data| drag_carries_something(&data))
            {
                event.prevent_default();
                *drop_depth.borrow_mut() += 1;
                dropping.set(true);
            }
        })
    };
    let on_drag_over = Callback::from(|event: DragEvent| {
        if event
            .data_transfer()
            .is_some_and(|data| drag_carries_something(&data))
        {
            event.prevent_default();
        }
    });
    let on_drag_leave = {
        let dropping = dropping.clone();
        let drop_depth = drop_depth.clone();
        Callback::from(move |_: DragEvent| {
            let mut depth = drop_depth.borrow_mut();
            *depth = (*depth - 1).max(0);
            if *depth == 0 {
                dropping.set(false);
            }
        })
    };
    let on_drop = {
        let dropping = dropping.clone();
        let drop_depth = drop_depth.clone();
        let ingest = ingest.clone();
        let append_text = append_text.clone();
        let busy = busy_reason.clone();
        let notice = notice.clone();
        Callback::from(move |event: DragEvent| {
            *drop_depth.borrow_mut() = 0;
            dropping.set(false);
            let Some(data) = event.data_transfer() else {
                return;
            };
            if !drag_carries_something(&data) {
                return;
            }
            event.prevent_default();
            let (files, _folders) = dropped_files(&data);
            if !files.is_empty() {
                if let Some(reason) = busy.clone() {
                    notice.emit(reason);
                    return;
                }
                ingest.emit(files);
                return;
            }
            // A folder is refused, as the Mac refuses one; a link is words.
            let links = dropped_links(&data);
            if !links.is_empty() {
                append_text.emit(links);
            }
        })
    };
    let server_can_see = props
        .assistant
        .as_ref()
        .is_some_and(|assistant| assistant.vision);
    let family_allows = props.family.as_ref().is_some_and(|family| family.ai_vision);
    let pictures = Pictures {
        staged: props
            .staged
            .iter()
            .map(|item| {
                let preview = item.preview.as_ref().map(|blob| blob.size() as u64);
                Candidate::new(
                    &item.kind,
                    &assistant_pictures::wire_mime(&item.mime, preview.is_some()),
                    Some(preview.unwrap_or(item.size.max(0) as u64)),
                )
            })
            .collect(),
        quoted: replying
            .and_then(|id| props.messages.iter().find(|message| message.id == id))
            .map(|quoted| {
                quoted
                    .attachments()
                    .iter()
                    .map(|attachment| {
                        Candidate::of_attachment(
                            &attachment.kind,
                            attachment.mime.as_deref().unwrap_or(""),
                            attachment.size.map(|size| size.max(0) as u64),
                            attachment.has_preview,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        server_can_see,
        server_can_draw: props.assistant.as_ref().is_some_and(|assistant| {
            assistant.images
                && assistant
                    .draw
                    .as_deref()
                    .unwrap_or(fc_text::assistant::DRAW_TOKEN)
                    == fc_text::assistant::DRAW_TOKEN
        }),
        family_allows,
        family_history: props.family.as_ref().is_none_or(|family| family.ai_history),
        family_history_photos: props
            .family
            .as_ref()
            .is_some_and(|family| family.ai_history_photos),
    };
    let attach_menu = html! {
        <AttachMenu
            offers_pictures={assistant_pictures::offers_picture_attach(is_ai, server_can_see, family_allows)}
            on_pictures={ingest.clone()}
            offers_poll={is_family}
            busy={busy_reason.clone()}
            on_files={ingest.clone()}
            on_paste={on_paste_menu}
            on_record={start_recording}
            on_location={share_location}
            on_poll={{
                let poll_open = poll_open.clone();
                Callback::from(move |_: ()| poll_open.set(true))
            }}
            on_busy={notice.clone()}
        />
    };
    let composer_busy = *preparing || *locating || recording_on;

    let poll_dialog = (*poll_open).then(|| {
        let on_submit = {
            let on_action = props.on_action.clone();
            let poll_open = poll_open.clone();
            let replying = replying.clone();
            let pinned = pinned.clone();
            let members = props.members.clone();
            Callback::from(move |(question, options): (String, Vec<String>)| {
                poll_open.set(false);
                *pinned.borrow_mut() = true;
                // A question names members the way a message does, and the
                // list is fixed at send (ios MacConversationView resolves
                // them from the question).
                let mentions = resolve_mentions(&question, &members, is_family);
                on_action.emit(Action::Send {
                    chat_id,
                    draft: Draft {
                        body: question,
                        reply_to_message_id: *replying,
                        mentions,
                        poll: Some(options),
                        attachments: Vec::new(),
                    },
                });
                replying.set(None);
            })
        };
        let on_cancel = {
            let poll_open = poll_open.clone();
            Callback::from(move |_: ()| poll_open.set(false))
        };
        html! { <PollComposer {on_submit} {on_cancel} /> }
    });

    let report_dialog = (*report).clone().map(|target| {
        let on_submit = {
            let on_action = props.on_action.clone();
            let report = report.clone();
            let target = target.clone();
            Callback::from(move |reason: String| {
                report.set(None);
                on_action.emit(Action::Report {
                    user_id: target.user_id,
                    message_id: target.message_id,
                    reason,
                });
            })
        };
        let on_cancel = {
            let report = report.clone();
            Callback::from(move |_: ()| report.set(None))
        };
        html! {
            <ReportDialog
                {target}
                support_contact={props.support_contact.clone()}
                {on_submit}
                {on_cancel}
            />
        }
    });

    let divider_count = anchor.map(|(_, count)| count).unwrap_or(0);
    let member_count = props.members.len();
    let title = props.item.chat.display_title(
        props
            .item
            .chat
            .peer_user_id
            .and_then(|peer| props.names.get(&peer))
            .map(String::as_str),
    );

    html! {
        <section
            class={classes!("conversation", dropping.then_some("is-drop-target"))}
            ondragenter={on_drag_enter}
            ondragover={on_drag_over}
            ondragleave={on_drag_leave}
            ondrop={on_drop}
        >
            <header class="conversation-bar">
                <h2 class="conversation-title">{ title }</h2>
                if is_family {
                    <button class="open-polls" onclick={
                        let on_action = props.on_action.clone();
                        Callback::from(move |_: MouseEvent| on_action.emit(Action::ShowOpenPolls { chat_id }))
                    }>
                        { "Open polls" }
                        if props.unanswered_polls > 0 {
                            <span class="badge">{ props.unanswered_polls }</span>
                        }
                    </button>
                }
            </header>
            <div class="messages" ref={list} onscroll={on_scroll}>
                if props.can_load_more {
                    <button class="load-more" onclick={
                        let on_action = props.on_action.clone();
                        Callback::from(move |_: MouseEvent| on_action.emit(Action::LoadMore { chat_id }))
                    }>{ "Earlier messages" }</button>
                }
                // The rows in a list of their OWN: keys only count in a list
                // where every child has one, and "Earlier messages" above
                // has none — mixed in with it, the rows would be matched up by
                // position after all, and a message landing below would hand
                // each bubble's state (an open menu) to its neighbour.
                <>
                { for rows.iter().map(|row| {
                    let message = row.message;
                    let revealed = props.revealed.contains(&message.id);
                    let quote_revealed = props.revealed_quotes.contains(&(message.id, 0));
                    let parent_revealed = props.revealed_quotes.contains(&(message.id, 1));
                    let failed = message
                        .client_msg_id
                        .as_ref()
                        .and_then(|id| props.failed.get(id))
                        .cloned()
                        .filter(|_| message.id == 0);
                    html! {
                        <key={row_key(message, props.my_user_id)}>
                            if let Some(day) = row.day_above {
                                <div class="day-pill" role="separator">{ time::day_label(day, props.now_ms) }</div>
                            }
                            if row.unread_divider_above {
                                <div id="unread-divider" class="unread-divider" role="separator">
                                    { if divider_count == 1 { "1 new message".to_string() } else { format!("{divider_count} new messages") } }
                                </div>
                            }
                            <div class={classes!("row", (*highlight == Some(message.id)).then_some("is-highlighted"))}>
                                <Bubble
                                    message={message.clone()}
                                    my_user_id={props.my_user_id}
                                    names={names.clone()}
                                    blocked={props.blocked.clone()}
                                    {quote_revealed}
                                    {parent_revealed}
                                    hidden={row.hidden && !revealed}
                                    shows_sender={row.shows_sender || (row.hidden && revealed && is_family)}
                                    run_end={row.run_end}
                                    seen={row.seen}
                                    awaited={row.awaited}
                                    ai_failed={props.ai_failed.contains(&message.id)}
                                    {failed}
                                    is_family_chat={is_family}
                                    is_ai_chat={is_ai}
                                    {assistant_user_id}
                                    in_thread={false}
                                    {member_count}
                                    member_ids={props.members.iter().map(|member| member.id).collect::<HashSet<i64>>()}
                                    on_action={props.on_action.clone()}
                                    on_reply={on_reply.clone()}
                                    on_edit={on_edit.clone()}
                                    on_report={on_report.clone()}
                                    on_jump={on_jump.clone()}
                                />
                            </div>
                        </>
                    }
                }) }
                </>
            </div>
            if !*at_newest {
                <button class="jump-newest" onclick={jump_to_newest} aria-label="Jump to the newest message">{ "↓" }</button>
            }
            if !props.typing.is_empty() {
                <p class="typing" aria-live="polite">{ typing_line(&props.typing) }</p>
            }
            <StagingStrip
                items={props.staged.clone()}
                on_remove={props.on_action.reform(move |index| Action::Unstage { chat_id, index })}
            />
            if let Some(text) = (*media_notice).clone() {
                <p class="composer-notice media-notice" role="status">
                    { text }
                    <button class="link" aria-label="Dismiss"
                            onclick={let media_notice = media_notice.clone(); Callback::from(move |_: MouseEvent| media_notice.set(None))}>
                        { "✕" }
                    </button>
                </p>
            }
            if let Some(since) = *recording_since {
                <RecordingBar started_ms={since} on_cancel={cancel_recording} on_stop={stop_recording} />
            }
            <Composer
                key={format!("composer-{chat_id}")}
                {chat_id}
                is_family_chat={is_family}
                is_ai_chat={is_ai}
                my_user_id={props.my_user_id}
                members={props.members.clone()}
                blocked={props.blocked.clone()}
                assistant={props.assistant.clone()}
                replying={replying_to}
                editing={edit_banner}
                initial={props.draft.clone()}
                on_send={on_send}
                on_save_edit={on_save_edit}
                on_cancel={{
                    let replying = replying.clone();
                    let editing = editing.clone();
                    Callback::from(move |_: ()| {
                        replying.set(None);
                        editing.set(None);
                    })
                }}
                on_typing={props.on_action.reform(move |_: ()| Action::Typing { chat_id })}
                on_draft={props.on_action.reform(move |text: String| Action::SaveDraft { chat_id, text })}
                attach={attach_menu}
                staged={props.staged.len()}
                busy={composer_busy}
                append={(*append).clone()}
                take={*take}
                {on_take}
                on_files={ingest}
                takes_files={true}
                {pictures}
            />
            { poll_dialog.unwrap_or_default() }
            { report_dialog.unwrap_or_default() }
        </section>
    }
}

/// "Anna is typing…", "Anna and Bob are typing…", "Several people are
/// typing…" — the third because a list of five names is a line nobody
/// reads, and the count is the part that matters.
pub fn typing_line(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => format!("{one} is typing…"),
        [one, two] => format!("{one} and {two} are typing…"),
        _ => "Several people are typing…".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn the_typing_line_names_one_or_two_and_counts_the_rest() {
        assert_eq!(typing_line(&[]), "");
        assert_eq!(typing_line(&["Anna".to_string()]), "Anna is typing…");
        assert_eq!(
            typing_line(&["Anna".to_string(), "Bob".to_string()]),
            "Anna and Bob are typing…"
        );
        assert_eq!(
            typing_line(&["Anna".to_string(), "Bob".to_string(), "Gran".to_string()]),
            "Several people are typing…"
        );
    }
}
