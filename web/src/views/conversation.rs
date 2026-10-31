//! One chat: its messages, and the box to add to them.

use std::collections::{HashMap, HashSet};

use fc_text::i18n::{t, t1, t2, tn};
use fc_text::media;
use fc_text::record::Dimmed;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, File};
use yew::prelude::*;

use crate::actions::Action;
use crate::live::Opening;
use crate::location;
use crate::model::{AiFailure, Assistant, ChatListItem, Family, Member, Message, PackItem};
use crate::pack::Gate;
use crate::prep;
use crate::staged::Prepared;
use crate::store::{Draft, Messages, NotSent};
use crate::time;
use crate::timeline::{self, Context, Window};
use crate::views::attach::{
    drag_carries_something, dropped_files, dropped_links, read_clipboard, AttachMenu, Clip,
    NotSentRow, RecordingRow, StagingStrip, PLAY_AFTER,
};
use crate::views::bubble::Bubble;
use crate::views::composer::{
    cannot_record_video, resolve_mentions, Composer, Editing, Pictures, Records, Replying,
    VideoEntry,
};
use crate::views::consent::AssistantConsentDialog;
use crate::views::dialog::Confirm;
use crate::views::group::GroupInfoDialog;
use crate::views::poll::PollComposer;
use crate::views::quiet::{use_quiet_reason, use_quiet_while};
use crate::views::report::{AssistantReportDialog, ReportDialog, ReportTarget};
use crate::views::round_recorder::RoundRecorder;
use crate::views::stickers::StickerMenu;
use crate::views::voice::{self, use_voice};
use fc_text::assistant_pictures::{self, Candidate};

#[derive(Properties, PartialEq)]
pub struct ConversationProps {
    pub item: ChatListItem,
    /// The chat's messages, oldest first — the store's own list, shared
    /// (`store::Messages`): handing it over copies nothing, and a draw of
    /// the app that changed nothing here is told so by the pointer.
    pub messages: Messages,
    pub my_user_id: i64,
    /// userId → the name to draw. A sender who is not in the roster reads
    /// as "Someone" rather than as a number.
    pub names: HashMap<i64, String>,
    pub members: Vec<Member>,
    pub assistant: Option<Assistant>,
    /// What this server signals: no button at all where it carries no
    /// calls, and no camera where it carries no video (docs/protocol.md,
    /// "Where the servers come from").
    #[prop_or_default]
    pub calls_enabled: bool,
    #[prop_or_default]
    pub video_calls_enabled: bool,
    /// A call is already on: there is only ever one, so the buttons wait.
    #[prop_or_default]
    pub on_call: bool,
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
    pub ai_failed: HashMap<i64, AiFailure>,
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
    /// This chat's voice messages that were not sent — stopped by something
    /// other than the person, or left in review when they left the chat
    /// (the plan for #79, docs/audio-video-messages-2026-10-04.md, S2.8) —
    /// each in a row of its own above the box, sent or deleted by that row
    /// and nothing else. While one waits, no new recording starts here.
    #[prop_or_default]
    pub not_sent: Vec<NotSent>,
    /// Which sign-in this pane belongs to: work it started that lands after
    /// the pane has gone lands only in this one (`AppState::session`).
    #[prop_or_default]
    pub session: u64,
    /// The reader's family — whose switches decide what may go to the
    /// assistant.
    #[prop_or_default]
    pub family: Option<Family>,
    pub support_contact: Option<String>,
    /// The family's sticker pack, in the order the panel shows it — and
    /// None where there is no sticker button at all: a server that predates
    /// packs, or an account in no family (docs/protocol.md, "What old
    /// clients and old servers do").
    #[prop_or_default]
    pub stickers: Option<Vec<PackItem>>,
    /// Whether this member has agreed that their words may go to the model
    /// (docs/protocol.md, "Consenting to the assistant").
    #[prop_or_default]
    pub agreed_to_assistant: bool,
    /// The text of recordings this member asked for, as this tab holds it
    /// (docs/protocol.md, "Transcripts on request").
    #[prop_or_default]
    pub transcripts: fc_text::transcript::Transcripts,
    pub on_action: Callback<Action>,
    /// Wall-clock now, for the day pills — to the MINUTE (main.rs floors
    /// it). A day starts on a minute, so the pills read exactly what they
    /// would at the millisecond; and a prop that changed on every draw of
    /// the app was a pane, and every bubble in it, drawn again on every
    /// draw of the app.
    pub now_ms: f64,
    /// How long and how big a video message may be on this server — None on
    /// a server without video messages, which is offered no video entry at
    /// all (docs/protocol.md, "Video messages").
    #[prop_or_default]
    pub round: Option<crate::round_video::RoundLimits>,
}

/// How close to the bottom still counts as "reading the newest", in
/// pixels. A reader a line or two up is still following along; one who has
/// scrolled a screen away is reading history and must not be yanked down.
pub(crate) const PINNED_SLACK_PX: i32 = 48;

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

/// Whether `message` answers a video message among `held` — its quote then
/// says "Video message", having no words of its own to show (the plan for
/// #79, S5.7). A quoted message this client does not hold says what the
/// server's excerpt says.
///
/// The rule as written, one reply at a time: what `QuotedKinds` — which a
/// draw asks instead, once for every row — is held to.
#[cfg(test)]
pub fn quotes_round(message: &Message, held: &[Message]) -> bool {
    message.reply_to.as_ref().is_some_and(|quote| {
        held.iter().any(|other| {
            other.id == quote.message_id && other.id != 0 && other.round_video().is_some()
        })
    })
}

/// Whether `message` answers an ANIMATION among `held` — its quote then
/// says "GIF" where it has no words of its own (docs/protocol.md,
/// "Animations"). A quoted message this client does not hold says what the
/// server's excerpt says.
///
/// Held to like `quotes_round`.
#[cfg(test)]
pub fn quotes_animation(message: &Message, held: &[Message]) -> bool {
    message.reply_to.as_ref().is_some_and(|quote| {
        held.iter().any(|other| {
            other.id == quote.message_id && other.id != 0 && other.animation().is_some()
        })
    })
}

/// Which of the messages held are video messages and which are animations,
/// by id — `quotes_round` and `quotes_animation` for every row of a draw,
/// from ONE pass over the messages instead of one per reply.
#[derive(Debug, Default, PartialEq)]
pub struct QuotedKinds {
    rounds: HashSet<i64>,
    animations: HashSet<i64>,
}

impl QuotedKinds {
    pub fn of(held: &[Message]) -> Self {
        let mut kinds = QuotedKinds::default();
        for message in held.iter().filter(|message| message.id != 0) {
            if message.round_video().is_some() {
                kinds.rounds.insert(message.id);
            }
            if message.animation().is_some() {
                kinds.animations.insert(message.id);
            }
        }
        kinds
    }

    /// `quotes_round(message, held)`.
    pub fn round(&self, message: &Message) -> bool {
        message
            .reply_to
            .as_ref()
            .is_some_and(|quote| self.rounds.contains(&quote.message_id))
    }

    /// `quotes_animation(message, held)`.
    pub fn animation(&self, message: &Message) -> bool {
        message
            .reply_to
            .as_ref()
            .is_some_and(|quote| self.animations.contains(&quote.message_id))
    }
}

/// The names a chat's bubbles are drawn with: the roster's — and in the
/// assistant's own chat, anything not mine is the assistant's, drawn with
/// the chat's own name rather than looked up in a roster it is not in.
/// `senders` are the chat's senders other than me, which is all the
/// messages have to say about it.
pub fn drawn_names(
    names: &HashMap<i64, String>,
    ai_title: Option<&str>,
    senders: &[i64],
) -> HashMap<i64, String> {
    let mut names = names.clone();
    if let Some(title) = ai_title {
        for sender in senders {
            names.insert(*sender, title.to_string());
        }
    }
    names
}

/// The oldest numbered row drawn from `start` on.
fn drawn_oldest(messages: &[Message], start: usize) -> Option<i64> {
    messages
        .get(start..)?
        .iter()
        .map(|message| message.id)
        .find(|id| *id != 0)
}

/// The bubble of message `id`, if it is drawn.
fn drawn_row(id: i64) -> Option<Element> {
    web_sys::window()?
        .document()?
        .get_element_by_id(&format!("m-{id}"))
}

/// Scroll a drawn bubble into view and tint it for a moment.
fn show_highlighted(target: &Element, id: i64, highlight: &UseStateHandle<Option<i64>>) {
    target.scroll_into_view();
    highlight.set(Some(id));
    let highlight = highlight.clone();
    gloo_timers::callback::Timeout::new(HIGHLIGHT_MS, move || highlight.set(None)).forget();
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
    // The reply the box is answering NOW, for what outlives the render that
    // made it: a recording kept as not sent — by a hidden tab, a call, the
    // pane going — keeps the reply it was recorded under (S2.8).
    let replying_now = use_mut_ref(|| Option::<i64>::None);
    *replying_now.borrow_mut() = *replying;
    let report = use_state(|| Option::<ReportTarget>::None);
    // The assistant reply being reported, by server id — its own state,
    // because it goes to its own endpoint and its own reader
    // (docs/protocol.md, "Reporting the assistant").
    let assistant_report = use_state(|| Option::<i64>::None);
    // The consent screen, raised by a send that would reach the model
    // before this member has agreed (docs/protocol.md, "Consenting to the
    // assistant"). The draft stays in the box either way: agreeing leaves
    // it there to send, and "Not Now" is "not this one", never "lose what
    // I typed".
    let consent_open = use_state(|| false);
    // "Group Info", opened from a group's title (docs/protocol.md,
    // "Groups").
    let group_info = use_state(|| false);
    let poll_open = use_state(|| false);
    let highlight = use_state(|| Option::<i64>::None);
    let at_newest = use_state(|| true);
    // THE VIDEO MESSAGE (the plan for #79, S3): what this browser can do,
    // asked once a page, and whether the recorder is open.
    let probe = use_state(|| Option::<crate::round_video::Probe>::None);
    {
        let probe = probe.clone();
        use_effect_with((), move |_| {
            spawn_local(async move { probe.set(Some(crate::round_video::probe().await)) });
        });
    }
    let recorder_open = use_state(|| false);
    // A roster from a server that no longer offers video messages takes the
    // recorder away mid-way — it is gone, not waiting: closed, so that the
    // composer it covered is drawn again and it does not spring back open
    // when the offer returns (decision 41: the composer is hidden while the
    // recorder is open, which must mean while it is drawn).
    {
        let recorder_open = recorder_open.clone();
        use_effect_with(props.round.is_some(), move |offered| {
            if !*offered && *recorder_open {
                recorder_open.set(false);
            }
        });
    }
    let round_said = use_state(|| (0u32, String::new()));
    let pane = use_node_ref();

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

    // THE WINDOW: which rows are drawn (timeline::Window) — the newest
    // `WINDOW` while the reader follows along, more as they scroll up, and
    // from the divider down when the chat opens at one. Taken here, in the
    // draw, because what it says is what this draw puts in the DOM.
    let window = use_mut_ref(Window::default);
    // The messages as of the latest draw, for the callbacks below — made
    // once and handed to every bubble, so they cannot hold the draw's own.
    let held_now = use_mut_ref(Messages::default);
    *held_now.borrow_mut() = props.messages.clone();
    // Whether the opening's anchor has been taken into the window: once,
    // in the draw that first knows it, so the divider the opening scrolls
    // to is there to scroll to.
    let anchored = use_mut_ref(|| false);
    let redraw = use_force_update();
    // What the list was before rows were added above — the oldest row
    // drawn and the list's height — for the effect below.
    let before = use_mut_ref(|| (None::<i64>, 0));
    let start = {
        let mut window = window.borrow_mut();
        let mut start = window.start(&props.messages, *pinned.borrow());
        if let Some((id, _)) = anchor.filter(|_| !*anchored.borrow()) {
            *anchored.borrow_mut() = true;
            if let Some(index) = props.messages.iter().position(|message| message.id == id) {
                window.include(&props.messages, index);
                start = window.held_start(&props.messages);
                // The opening puts the reader at the divider itself: the
                // rows drawn above it are not rows to hold the reader
                // still against.
                *before.borrow_mut() = (None, 0);
            }
        }
        start
    };

    // EARLIER MESSAGES land above the reader, and what they were reading
    // stays where it was on the screen: the scroll moves down by exactly
    // what was added above. Done here rather than left to the browser's
    // scroll anchoring, which Safari does not do (styles.css). The same for
    // rows the window draws above as the reader scrolls up to them — which
    // is why this follows the oldest row DRAWN, the oldest held whenever the
    // window reaches the top.
    {
        let list = list.clone();
        let pinned = pinned.clone();
        let before = before.clone();
        let oldest = drawn_oldest(&props.messages, start);
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
        let window = window.clone();
        let held_now = held_now.clone();
        let before = before.clone();
        let redraw = redraw.clone();
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
                // Within a screen of the top of the window, with rows held
                // above it: draw some of them, before the reader gets there.
                if !bottom && element.scroll_top() < element.client_height() {
                    let held = held_now.borrow();
                    let oldest = drawn_oldest(&held, window.borrow().held_start(&held));
                    if window.borrow_mut().grow(&held) {
                        *before.borrow_mut() = (oldest, element.scroll_height());
                        redraw.force_update();
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
    // is here at all. Held but above the window, it is drawn first (the
    // window taken up to it) and shown once it is: `jump` carries it to the
    // effect after that draw. One callback for the pane's life, handed to
    // every bubble: a new one each draw was every bubble drawn again.
    let jump = use_state(|| Option::<i64>::None);
    let on_jump = {
        let highlight = highlight.clone();
        let pinned = pinned.clone();
        let window = window.clone();
        let held_now = held_now.clone();
        let before = before.clone();
        let list = list.clone();
        let jump = jump.clone();
        use_callback((), move |id: i64, _| {
            if let Some(target) = drawn_row(id) {
                *pinned.borrow_mut() = false;
                show_highlighted(&target, id, &highlight);
                return;
            }
            let held = held_now.borrow();
            let Some(index) = held
                .iter()
                .position(|message| message.id == id && message.id != 0)
            else {
                return;
            };
            *pinned.borrow_mut() = false;
            let oldest = drawn_oldest(&held, window.borrow().held_start(&held));
            window.borrow_mut().include(&held, index);
            if let Some(element) = list.cast::<Element>() {
                *before.borrow_mut() = (oldest, element.scroll_height());
            }
            jump.set(Some(id));
        })
    };
    {
        let highlight = highlight.clone();
        let jump = jump.clone();
        use_effect_with(*jump, move |target| {
            if let Some(id) = *target {
                if let Some(target) = drawn_row(id) {
                    show_highlighted(&target, id, &highlight);
                }
                jump.set(None);
            }
        });
    }

    // What this chat offers, by its kind as sent (fc_text::groups::offers):
    // a group is the family chat less calls and "seen" (docs/protocol.md,
    // "Groups"). `chat_kind` is the kind the shared rules are asked about —
    // a group as a group, and nothing guessed for a kind this client does
    // not know.
    let chat_kind = fc_text::groups::known(&props.item.chat.kind);
    let offers = props.item.chat.offers();
    let takes_mentions = offers.mentions;
    let is_ai = props.item.chat.is_ai();
    // Whom a mention here may name and a poll here counts: the family, or
    // in a group the group's members.
    let chat_members = props.item.chat.members_of(&props.members);
    let blocked_peer = props
        .item
        .chat
        .peer_user_id
        .is_some_and(|peer| props.blocked.contains(&peer));
    let place = |video: bool| {
        let on_action = props.on_action.clone();
        Callback::from(move |_: MouseEvent| on_action.emit(Action::PlaceCall { chat_id, video }))
    };
    let assistant_user_id = props.assistant.as_ref().map(|assistant| assistant.user_id);
    let context = Context {
        my_user_id: props.my_user_id,
        chat_kind,
        assistant_user_id,
        blocked: &props.blocked,
        first_unread_id: anchor.map(|(id, _)| id),
        peer_read_up_to: props.peer_read,
    };
    let rows = timeline::rows_from(&props.messages, start, &context);

    // WHAT EVERY BUBBLE IS HANDED, made once and kept while it holds: a
    // bubble is drawn again only when something of ITS changed. Handed a
    // fresh copy of the roster's names, a fresh set and fresh callbacks on
    // every draw — every typing frame, every catch-up page — every bubble in
    // the chat was drawn again, its markdown parsed and its links found
    // anew, to say exactly what it said before.
    let ai_title = props.item.chat.title.clone().filter(|_| is_ai);
    let ai_senders: Vec<i64> = match ai_title {
        Some(_) => {
            let mut senders: Vec<i64> = props
                .messages
                .iter()
                .map(|message| message.sender_id)
                .filter(|sender| *sender != props.my_user_id)
                .collect();
            senders.sort_unstable();
            senders.dedup();
            senders
        }
        None => Vec::new(),
    };
    let names = use_memo(
        (props.names.clone(), ai_title, ai_senders),
        |(names, ai_title, senders)| drawn_names(names, ai_title.as_deref(), senders),
    );
    let blocked = use_memo(props.blocked.clone(), |blocked| blocked.clone());
    let member_ids = use_memo(
        chat_members
            .iter()
            .map(|member| member.id)
            .collect::<Vec<i64>>(),
        |ids| ids.iter().copied().collect::<HashSet<i64>>(),
    );
    let quoted = use_memo(props.messages.clone(), |messages| QuotedKinds::of(messages));

    let on_reply = {
        let replying = replying.clone();
        let editing = editing.clone();
        use_callback((), move |id: i64, _| {
            editing.set(None);
            replying.set(Some(id));
        })
    };
    let on_edit = {
        let replying = replying.clone();
        let editing = editing.clone();
        use_callback((), move |id: i64, _| {
            replying.set(None);
            editing.set(Some(id));
        })
    };
    let on_report = {
        let report = report.clone();
        use_callback(
            names.clone(),
            move |(user_id, message_id): (i64, Option<i64>), names| {
                report.set(Some(ReportTarget {
                    user_id,
                    name: names
                        .get(&user_id)
                        .cloned()
                        .unwrap_or_else(|| t("Someone").to_string()),
                    message_id,
                }));
            },
        )
    };

    let on_report_assistant = {
        let assistant_report = assistant_report.clone();
        use_callback((), move |message_id: i64, _| {
            assistant_report.set(Some(message_id))
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
                .unwrap_or_else(|| t("Someone").to_string()),
            excerpt: if hidden {
                String::new()
            } else if message.round_video().is_some() {
                // No words to quote: what it is (S5.7).
                t("Video message").to_string()
            } else if message.body.is_empty() && message.animation().is_some() {
                // An animation with no caption: what it is (#96).
                t("GIF").to_string()
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
    // The batch being prepared, for as long as it is: how it is told to
    // stop. A video outside the profile is transcoded as it is staged, and
    // for a long clip that is minutes — during which the composer is busy.
    // So it can be called off, and it is called off when the pane goes.
    let job = use_mut_ref(|| Option::<prep::Job>::None);
    let locating = use_state(|| false);
    let media_notice = use_state(|| Option::<String>::None);
    let append = use_state(|| (0u32, String::new()));
    let take = use_state(|| 0u32);
    // Moves each time "Ask the Assistant" is chosen from the paperclip's
    // menu: the composer puts `@ai ` into its draft (#106).
    let ask = use_state(|| 0u32);
    // The location waiting for the draft to be its caption.
    let pending = use_mut_ref(|| Option::<Prepared>::None);
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
    // Whether the words in the box are blank — the composer says so as it
    // changes — for a recording started beside them (S1.3 row 3).
    let draft_blank = use_mut_ref(|| props.draft.trim().is_empty());

    // THE VOICE MESSAGE IN THE SEND SLOT (the plan for #79, S1–S2): the
    // shared rules (fc_text::record) driven, the microphone with them.
    let voice = use_voice(
        voice::Setup {
            chat_id,
            session: props.session,
            on_action: props.on_action.clone(),
            staged: props.staged.len(),
            blocked: voice::blocked(
                props.on_call,
                *preparing || *locating,
                !props.not_sent.is_empty(),
            ),
            on_call: props.on_call,
            notice_now: (*media_notice).clone(),
        },
        alive.clone(),
        replying.clone(),
        replying_now.clone(),
        media_notice.clone(),
    );
    let recording_on = voice.active();
    // While it records, the app's other live regions are quiet too (S6).
    use_quiet_while(recording_on);
    // What plays — one thing at a time, paused for a hidden tab, a call and
    // headphones pulled, stopped when this chat closes (S4, S5.3).
    crate::now_playing::use_now_playing(props.on_call);

    // Which busy it is, because the two have different ways out
    // (MacConversationView.composerBusyNotice).
    let busy_reason = if editing.is_some() {
        Some(t("Finish editing before attaching something.").to_string())
    } else if *preparing || *locating || recording_on || voice.finishing {
        Some(t("Wait until the current attachment is done.").to_string())
    } else {
        None
    };
    let notice = {
        let media_notice = media_notice.clone();
        Callback::from(move |text: String| media_notice.set(Some(text)))
    };
    // A bubble's ▶, dimmed while a recording runs, says why on this line
    // (S1.3, S1.7).
    use_quiet_reason(notice.clone());
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
        let job = job.clone();
        let changed = voice.changed.clone();
        Callback::from(move |files: Vec<File>| {
            if files.is_empty() {
                return;
            }
            // An animated picture becomes a looping video in every chat but
            // the assistant's, whose model is shown a photograph (#96).
            let assistant = is_ai;
            if let Some(reason) = busy.clone() {
                media_notice.set(Some(reason));
                return;
            }
            preparing.set(true);
            media_notice.set(Some(t("Preparing…").to_string()));
            // How far a transcode has got, said beside the word: minutes of
            // "Preparing…" with nothing moving reads as a page that hung.
            let batch = {
                let media_notice = media_notice.clone();
                prep::Job::watched(move |percent| {
                    media_notice.set(Some(format!("{} {percent}%", t("Preparing…"))));
                })
            };
            *job.borrow_mut() = Some(batch.clone());
            let on_action = on_action.clone();
            let preparing = preparing.clone();
            let media_notice = media_notice.clone();
            let alive = alive.clone();
            let job = job.clone();
            let changed = changed.clone();
            spawn_local(async move {
                let mut count = staged;
                let mut said = None;
                for file in files {
                    if !*alive.borrow() || batch.stopped() {
                        return;
                    }
                    if !media::can_stage(count) {
                        said = Some(tn(
                            "You can attach up to %lld items.",
                            media::MAX_PER_MESSAGE as i64,
                        ));
                        break;
                    }
                    let prepared = if assistant {
                        prep::prepare_for_assistant(&file, &batch).await
                    } else {
                        prep::prepare(&file, &batch).await
                    };
                    // Cancelled, like gone: whoever cancelled has already
                    // put the composer back, and may have started another
                    // batch since — this one touches nothing more.
                    if !*alive.borrow() || batch.stopped() {
                        return;
                    }
                    match prepared {
                        Ok(item) => {
                            on_action.emit(Action::Stage { chat_id, item });
                            // Staged by the person: never guarded (S1.1).
                            changed.emit(());
                            count += 1;
                        }
                        Err(error) => said = Some(error.message().to_string()),
                    }
                }
                *job.borrow_mut() = None;
                preparing.set(false);
                media_notice.set(said);
            });
        })
    };
    // Calling a preparation off. The composer is given back at once, not
    // when the transcode notices: what is left of it lets go of its codecs
    // the next time its loop comes round, and stages nothing.
    let cancel_preparing = {
        let job = job.clone();
        let preparing = preparing.clone();
        let media_notice = media_notice.clone();
        Callback::from(move |_: MouseEvent| {
            if let Some(batch) = job.borrow_mut().take() {
                batch.stop();
            }
            preparing.set(false);
            media_notice.set(None);
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
                    Clip::Nothing => notice.emit(t("There's nothing to paste.").to_string()),
                    Clip::Denied => notice.emit(
                        t("This browser didn't let Family read the clipboard. Paste with ⌘V or Ctrl+V instead.")
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
    // A HIDDEN TAB stops a recording and keeps it (S4): a tab switched away
    // from, a window minimised, a phone locked or gone to another app — and
    // a desktop that locked, where the browser says so through the same
    // event. A window that only lost focus is still on screen, and goes on
    // recording; so does one showing a permission prompt.
    {
        let interrupt = voice.interrupt.clone();
        use_effect_with((), move |_| {
            let hidden = Closure::<dyn Fn()>::new(move || {
                if !crate::sync::page_visible() {
                    interrupt.emit(());
                }
            });
            let document = web_sys::window().and_then(|window| window.document());
            if let Some(document) = &document {
                let _ = document.add_event_listener_with_callback(
                    "visibilitychange",
                    hidden.as_ref().unchecked_ref(),
                );
            }
            move || {
                if let Some(document) = document {
                    let _ = document.remove_event_listener_with_callback(
                        "visibilitychange",
                        hidden.as_ref().unchecked_ref(),
                    );
                }
            }
        });
    }
    // A CALL — ringing, placed or answered — stops a recording and keeps it
    // (S4); and none starts while one is on: the microphone is dimmed.
    {
        let interrupt = voice.interrupt.clone();
        use_effect_with(props.on_call, move |on_call| {
            if *on_call {
                interrupt.emit(());
            }
        });
    }
    // NOTHING OF THE APP'S PLAYS INTO A NOTE (S1.7): whatever starts playing
    // while a recording runs — a voice note in a bubble, here or in a
    // thread, a video — is paused at once, and says why. Heard on the way
    // down, before any player hears its own `play`.
    {
        let notice = notice.clone();
        use_effect_with(recording_on, move |recording| {
            let document = web_sys::window()
                .and_then(|window| window.document())
                .filter(|_| *recording);
            let guard = Closure::<dyn Fn(web_sys::Event)>::new(move |event: web_sys::Event| {
                if let Some(player) = event
                    .target()
                    .and_then(|target| target.dyn_into::<web_sys::HtmlMediaElement>().ok())
                {
                    let _ = player.pause();
                    notice.emit(t(PLAY_AFTER).to_string());
                }
            });
            if let Some(document) = &document {
                let _ = document.add_event_listener_with_callback_and_bool(
                    "play",
                    guard.as_ref().unchecked_ref(),
                    true,
                );
            }
            move || {
                if let Some(document) = document {
                    let _ = document.remove_event_listener_with_callback_and_bool(
                        "play",
                        guard.as_ref().unchecked_ref(),
                        true,
                    );
                }
            }
        });
    }
    // Gone with the pane: nothing it started may land in whatever is on
    // screen next, a microphone left open is a microphone left open — and a
    // transcode nobody will see the end of is stopped, not left to run for
    // minutes and be thrown away. A recording is not cancelled: leaving the
    // chat keeps it as not sent (S2.8, S4).
    {
        let alive = alive.clone();
        let job = job.clone();
        let interrupt = voice.interrupt.clone();
        use_effect_with((), move |_| {
            move || {
                *alive.borrow_mut() = false;
                interrupt.emit(());
                if let Some(batch) = job.borrow_mut().take() {
                    batch.stop();
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
                    media_notice.set(Some(t("Finding your location…").to_string()));
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
    // "Record Voice Message" says why it cannot record instead of
    // recording: during a call, and while a voice message that was not sent
    // waits (S1.5). With words typed or items staged it records beside them,
    // and the slot is Stop (S1.3 row 3): they must not leave unseen.
    let record_dimmed = if props.on_call {
        Some(t(Dimmed::Call.notice()).to_string())
    } else if !props.not_sent.is_empty() {
        Some(t(Dimmed::NotSent.notice()).to_string())
    } else {
        None
    };
    let record_from_menu = {
        let record = voice.record.clone();
        let draft_blank = draft_blank.clone();
        let staged = props.staged.len();
        Callback::from(move |_: ()| record.emit(!*draft_blank.borrow() || staged > 0))
    };
    // THE WAYS INTO THE VIDEO RECORDER (S1.4–S1.6): offered in the family
    // chat, a direct chat or a group, on a server with video messages, on a page that can
    // capture at all — in a browser that cannot record one ALWAYS, the menus
    // saying so instead of opening (S1.5, S8.7), whatever cameras it lists;
    // where it can, on a device with a camera
    // (`round_video::offers_video_entry`). None of
    // them opens it during a call or while the composer is busy; a voice
    // message that was not sent does not stop it — that rule is about voice.
    let probed = *probe;
    let offers_video = crate::round_video::offers_video_entry(
        props.round.is_some(),
        offers.video_messages,
        probed,
    );
    let records_video = probed.is_some_and(|probe| probe.records);
    let open_recorder = {
        let recorder_open = recorder_open.clone();
        let notice = notice.clone();
        let on_call = props.on_call;
        let busy = busy_reason.clone();
        Callback::from(move |_: ()| {
            if on_call {
                notice.emit(t(Dimmed::Call.notice()).to_string());
            } else if let Some(reason) = busy.clone() {
                notice.emit(reason);
            } else if !records_video {
                notice.emit(cannot_record_video().to_string());
            } else {
                recorder_open.set(true);
            }
        })
    };
    let video_dimmed = if props.on_call {
        Some(t(Dimmed::Call.notice()).to_string())
    } else if !records_video {
        Some(cannot_record_video().to_string())
    } else {
        None
    };
    // "Ask the Assistant" — the row's old `@ai` button, now a line of the
    // paperclip's menu — where `@ai` may be written at all: the family
    // chat or a group, on a server with an assistant that names who answers
    // (docs/protocol.md, "Mentioning a member", amended 2026-10-20).
    let offers_assistant = offers.assistant_mention
        && props
            .assistant
            .as_ref()
            .and_then(|assistant| assistant.processor.as_deref())
            .is_some_and(|processor| !processor.trim().is_empty());
    let attach_menu = html! {
        <AttachMenu
            {offers_assistant}
            on_assistant={{
                let ask = ask.clone();
                Callback::from(move |_: ()| ask.set(*ask + 1))
            }}
            offers_video={offers_video}
            video_dimmed={video_dimmed}
            on_video={open_recorder.clone()}
            assistant_chat={is_ai}
            offers_pictures={assistant_pictures::offers_picture_attach(is_ai, server_can_see, family_allows)}
            on_pictures={ingest.clone()}
            offers_poll={offers.polls}
            busy={busy_reason.clone()}
            on_files={ingest.clone()}
            on_paste={on_paste_menu}
            on_record={record_from_menu}
            offers_record={!is_ai}
            record_dimmed={record_dimmed}
            on_location={share_location}
            on_poll={{
                let poll_open = poll_open.clone();
                Callback::from(move |_: ()| poll_open.set(true))
            }}
            on_busy={notice.clone()}
        />
    };
    // THE STICKER BUTTON, inside the field at its trailing edge (#106), in
    // EVERY chat a message can be sent in, wherever this server has packs:
    // the family chat, a one-to-one chat, and the assistant's own. One
    // click sends: its own message, no caption, answering whatever the box
    // was answering.
    //
    // In the assistant's chat a sticker is a photograph shown to the model
    // (docs/protocol.md, "Sending one"), so the click goes through the
    // question every message there does and never round it ("Consenting
    // to the assistant"): while this member has not agreed, it raises the
    // consent screen INSTEAD of sending — picking again once the answer is
    // recorded sends it, as pressing Send again does for words — and on a
    // server that names nobody it says so and sends nothing.
    let sticker_gate = crate::pack::send_gate(
        chat_kind,
        props.assistant.as_ref(),
        props.agreed_to_assistant,
    );
    let sticker_menu = props.stickers.clone().map(|items| {
        let on_pick = {
            let on_action = props.on_action.clone();
            let replying = replying.clone();
            let pinned = pinned.clone();
            let consent_open = consent_open.clone();
            let notice = notice.clone();
            Callback::from(move |item_id: i64| {
                match sticker_gate {
                    Gate::Send => {}
                    Gate::Ask => {
                        consent_open.set(true);
                        return;
                    }
                    Gate::Withheld => {
                        notice.emit(
                            fc_text::assistant_consent::unnamed_processor_notice().to_string(),
                        );
                        return;
                    }
                }
                let reply_to_message_id = *replying;
                replying.set(None);
                // Your own message is always shown, wherever you were.
                *pinned.borrow_mut() = true;
                on_action.emit(Action::SendSticker {
                    chat_id,
                    item_id,
                    reply_to_message_id,
                });
            })
        };
        html! {
            <StickerMenu
                {items}
                // An edit in progress is the one thing a sticker waits for:
                // the box is somebody else's words until it is done.
                busy={editing.is_some().then(|| t("Finish editing before attaching something.").to_string())}
                {on_pick}
                on_busy={notice.clone()}
            />
        }
    });
    // The composer's attachment guard — S1.2's **busy**, which dims the
    // microphone (row 8) and holds a Send back. A recording is not it: the
    // recording row has the field's place while one runs. A note being
    // finished once it stopped is: Stop beside words stages it beside them
    // (S2.4), and a Send before it lands would leave without it.
    let composer_busy = *preparing || *locating || voice.finishing;

    let poll_dialog = (*poll_open).then(|| {
        let on_submit = {
            let on_action = props.on_action.clone();
            let poll_open = poll_open.clone();
            let replying = replying.clone();
            let pinned = pinned.clone();
            let members = chat_members.clone();
            Callback::from(move |(question, options): (String, Vec<String>)| {
                poll_open.set(false);
                *pinned.borrow_mut() = true;
                // A question names members the way a message does, and the
                // list is fixed at send (ios MacConversationView resolves
                // them from the question).
                let mentions = resolve_mentions(&question, &members, takes_mentions);
                on_action.emit(Action::Send {
                    chat_id,
                    draft: Draft {
                        body: question,
                        reply_to_message_id: *replying,
                        mentions,
                        poll: Some(options),
                        ..Draft::default()
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

    let assistant_report_dialog = (*assistant_report).map(|message_id| {
        let on_submit = {
            let on_action = props.on_action.clone();
            let assistant_report = assistant_report.clone();
            Callback::from(move |(reason, note): (String, Option<String>)| {
                assistant_report.set(None);
                on_action.emit(Action::ReportAssistant {
                    message_id,
                    reason,
                    note,
                });
            })
        };
        let on_cancel = {
            let assistant_report = assistant_report.clone();
            Callback::from(move |_: ()| assistant_report.set(None))
        };
        html! {
            <AssistantReportDialog
                {message_id}
                support_contact={props.support_contact.clone()}
                {on_submit}
                {on_cancel}
            />
        }
    });

    // "Show text" under a recording, which raises this chat's consent
    // screen when this member has not agreed yet.
    let transcription = {
        let consent_open = consent_open.clone();
        let on_review_consent = use_callback((), move |(), _| consent_open.set(true));
        use_memo(
            (
                props.assistant.clone(),
                props.family.clone(),
                on_review_consent,
            ),
            |(assistant, family, on_review_consent)| {
                crate::views::bubble::Transcription::of(
                    assistant.as_ref(),
                    family.as_ref(),
                    on_review_consent.clone(),
                )
            },
        )
    };
    let consent_dialog = consent_open
        .then(|| {
            props
                .assistant
                .as_ref()
                .and_then(|assistant| assistant.processor.clone())
        })
        .flatten()
        .map(|processor| {
            let on_agree = {
                let on_action = props.on_action.clone();
                let consent_open = consent_open.clone();
                Callback::from(move |with_lookups: bool| {
                    consent_open.set(false);
                    on_action.emit(Action::agreement(with_lookups));
                })
            };
            let on_cancel = {
                let consent_open = consent_open.clone();
                Callback::from(move |_: ()| consent_open.set(false))
            };
            let family = props.family.as_ref();
            html! {
                <AssistantConsentDialog
                    {processor}
                    family_history={family.is_some_and(|family| family.ai_history)}
                    family_vision={family.is_some_and(|family| family.ai_vision)}
                    transcribe={props.assistant.as_ref().is_some_and(|assistant| assistant.transcribe)}
                    lookups={props.assistant.as_ref().map(|assistant| assistant.lookups.clone()).unwrap_or_default()}
                    {on_agree}
                    {on_cancel}
                />
            }
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

    // THE VOICE MESSAGES THAT WERE NOT SENT (S2.8): each quotes the reply it
    // was recorded under, the way the box's banner quotes one…
    let quote_of = |reply: Option<i64>| -> Option<String> {
        let id = reply?;
        let message = props.messages.iter().find(|message| message.id == id)?;
        let hidden = timeline::is_hidden_by_block(message, props.my_user_id, &props.blocked)
            && !props.revealed.contains(&id);
        let name = names
            .get(&message.sender_id)
            .cloned()
            .unwrap_or_else(|| t("Someone").to_string());
        let excerpt = if hidden {
            String::new()
        } else {
            crate::store::excerpt(&message.body)
        };
        let quote = t1("Replying to %@", &name);
        Some(if excerpt.is_empty() {
            quote
        } else {
            format!("{quote}: {excerpt}")
        })
    };
    // …and leaves only by its own Send: with ITS reply and caption, and
    // nothing else — through the question every message to the model asks
    // first, as the box's own Send goes (docs/protocol.md, "Consenting to
    // the assistant").
    let send_not_sent = {
        let on_action = props.on_action.clone();
        let consent_open = consent_open.clone();
        let notice = notice.clone();
        let pinned = pinned.clone();
        let members = chat_members.clone();
        let processor = props
            .assistant
            .as_ref()
            .and_then(|assistant| assistant.processor.clone());
        let has_assistant = props.assistant.is_some();
        let agreed = props.agreed_to_assistant;
        Callback::from(move |row: NotSent| {
            use fc_text::assistant_consent;
            if assistant_consent::is_required(chat_kind, &row.caption, processor.as_deref(), agreed)
            {
                consent_open.set(true);
                return;
            }
            if assistant_consent::is_withheld_from_an_unnamed_assistant(
                chat_kind,
                &row.caption,
                has_assistant,
                processor.as_deref(),
            ) {
                notice.emit(assistant_consent::unnamed_processor_notice().to_string());
                return;
            }
            // Your own message is always shown, wherever you were.
            *pinned.borrow_mut() = true;
            on_action.emit(Action::SendNotSent {
                chat_id,
                id: row.id,
                mentions: resolve_mentions(&row.caption, &members, takes_mentions),
            });
        })
    };
    let delete_not_sent = props
        .on_action
        .reform(move |row: NotSent| Action::DeleteNotSent {
            chat_id,
            id: row.id,
        });

    // THE SEND SLOT (S1.3): the composer's trailing button, and the row
    // that takes the field's place while a recording runs (S2.4).
    let records = Records {
        recording: voice.recording,
        row: html! {
            <RecordingRow
                started_ms={voice.started_ms}
                meter={voice.meter.clone()}
                beside_draft={voice.recording == fc_text::record::Recording::HandsFreeBesideDraft}
                on_delete={voice.delete.clone()}
                on_stop={voice.stop.clone()}
                on_cap={voice.cap.clone()}
                on_warning={voice.warn.clone()}
                on_silence={voice.silence.clone()}
            />
        },
        call: props.on_call,
        not_sent: !props.not_sent.is_empty(),
        can_record: crate::recorder::can_record(),
        starting: voice.starting,
        on_slot: voice.activate.clone(),
        on_record: voice.record.reform(|_: ()| false),
        on_stop: voice.stop.clone(),
        on_emptied: voice.emptied.clone(),
        on_changed: voice.changed.clone(),
        guard_until: voice.guard_until.clone(),
        on_blank: {
            let draft_blank = draft_blank.clone();
            Callback::from(move |blank: bool| *draft_blank.borrow_mut() = blank)
        },
        refocus: voice.refocus,
        recorder_open: *recorder_open,
        video: offers_video.then(|| VideoEntry {
            records: records_video,
            on_open: open_recorder.clone(),
            on_explain: notice.clone(),
        }),
    };
    // THE RECORDER (S3), over the whole window while it is open.
    let recorder = match (props.round, *recorder_open) {
        (Some(limits), true) => {
            let on_send = {
                let on_action = props.on_action.clone();
                let replying = replying.clone();
                let replying_now = replying_now.clone();
                let pinned = pinned.clone();
                Callback::from(move |(clip, round): (Prepared, bool)| {
                    // Alone, as a video message always travels: the words
                    // and whatever is staged stay in the composer, and the
                    // primed reply goes with it (S1.5).
                    let reply_to_message_id = *replying_now.borrow();
                    replying.set(None);
                    *pinned.borrow_mut() = true;
                    on_action.emit(Action::Send {
                        chat_id,
                        draft: Draft {
                            reply_to_message_id,
                            attachments: vec![clip],
                            round,
                            ..Draft::default()
                        },
                    });
                })
            };
            let on_close = {
                let recorder_open = recorder_open.clone();
                let round_said = round_said.clone();
                Callback::from(move |said: Option<String>| {
                    recorder_open.set(false);
                    if let Some(said) = said {
                        round_said.set((round_said.0 + 1, said));
                    }
                })
            };
            let on_voice = {
                // "Record a voice message instead" (S3.4): hands-free, beside
                // whatever the box holds — in the click that asked for it.
                let record = voice.record.clone();
                let draft_blank = draft_blank.clone();
                let staged = props.staged.len();
                Callback::from(move |_: ()| record.emit(!*draft_blank.borrow() || staged > 0))
            };
            html! {
                <RoundRecorder
                    {limits}
                    on_call={props.on_call}
                    replying={replying_to.clone()}
                    on_drop_reply={{
                        let replying = replying.clone();
                        Callback::from(move |_: ()| replying.set(None))
                    }}
                    not_sent={!props.not_sent.is_empty()}
                    {on_voice}
                    {on_send}
                    {on_close}
                    pane={pane.clone()}
                />
            }
        }
        _ => Html::default(),
    };
    // Words the announcement node is saying already — "That recording was
    // too short.", "Recording stopped at five minutes." — are shown on the
    // notice line and hidden there from screen readers: said once, by the
    // node that is always there to say them (S6), where a line drawn with
    // its words already in it may not be read at all. The line stays the
    // live region it is, for whatever it says next.
    let notice_said = (*media_notice)
        .as_ref()
        .is_some_and(|text| voice.announcement.0 > 0 && voice.announcement.1 == *text);

    // What stacks above the row, handed to the composer to place (#106, doc
    // 2.4): one order on every client, the nearest to the field being what
    // is being written — notices · voice messages not sent · reply or edit
    // banner · staged · mention suggestions · the busy line · THE ROW. They
    // were drawn here, above the whole composer, which put the staged strip
    // and the voice messages not sent above the notices and the reply.
    let not_sent_rows = html! {
        <>
        { for props.not_sent.iter().map(|row| html! {
            <NotSentRow
                key={row.id}
                row={row.clone()}
                quote={quote_of(row.reply_to_message_id)}
                on_send={send_not_sent.clone()}
                on_delete={delete_not_sent.clone()}
                recording={recording_on}
                on_explain={notice.clone()}
            />
        }) }
        </>
    };
    let staging_strip = html! {
        <StagingStrip
            items={props.staged.clone()}
            on_remove={{
                let on_action = props.on_action.clone();
                let changed = voice.changed.clone();
                Callback::from(move |index| {
                    on_action.emit(Action::Unstage { chat_id, index });
                    // Taken off by the person: never guarded (S1.1).
                    changed.emit(());
                })
            }}
            recording={recording_on}
            on_explain={notice.clone()}
        />
    };
    let busy_line = match (*media_notice).clone() {
        // Quiet while a recording runs, as every live region here is
        // but the one that says what the recording does (S6).
        Some(text) => html! {
            <p class="composer-notice media-notice"
               role={(!recording_on).then_some("status")}
               aria-live={recording_on.then_some("off")}>
                if notice_said {
                    <span aria-hidden="true">{ text }</span>
                } else {
                    { text }
                }
                if *preparing {
                    // Not a notice to dismiss but work to call off.
                    <button class="link" onclick={cancel_preparing}>{ t("Cancel") }</button>
                } else {
                    <button class="link" aria-label={t("Dismiss")}
                            onclick={let media_notice = media_notice.clone(); Callback::from(move |_: MouseEvent| media_notice.set(None))}>
                        { "✕" }
                    </button>
                }
            </p>
        },
        None => Html::default(),
    };

    let divider_count = anchor.map(|(_, count)| count).unwrap_or(0);
    let chat_kind_value = AttrValue::from(props.item.chat.kind.clone());
    let member_count = chat_members.len();
    let avatar_versions: HashMap<i64, i64> = props
        .members
        .iter()
        .map(|member| (member.id, member.avatar_version))
        .collect();
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
            ref={pane.clone()}
            class={classes!("conversation", dropping.then_some("is-drop-target"))}
            ondragenter={on_drag_enter}
            ondragover={on_drag_over}
            ondragleave={on_drag_leave}
            ondrop={on_drop}
        >
            <header class="conversation-bar">
                // Below the two panes the conversation is the whole window,
                // and this is the way back to the list of chats (#106,
                // section 3). The stylesheet hides it where the list is
                // beside the chat.
                <button type="button" class="back-to-chats"
                        onclick={props.on_action.reform(|_: MouseEvent| Action::CloseChat)}>
                    <span aria-hidden="true">{ "‹ " }</span>{ t("Chats") }
                </button>
                // A group's title opens Group Info — its members, and the
                // admin's tools and everybody's Leave (docs/protocol.md,
                // "Groups").
                if props.item.chat.is_group() {
                    <h2 class="conversation-title">
                        <button type="button" class="title-button" aria-haspopup="dialog" onclick={{
                            let group_info = group_info.clone();
                            Callback::from(move |_: MouseEvent| group_info.set(true))
                        }}>
                            <span class="title-name">{ title.clone() }</span>
                            <span class="title-sub">{ tn("Members: %lld", props.item.chat.group_member_ids().len() as i64) }</span>
                        </button>
                    </h2>
                } else {
                    <h2 class="conversation-title">{ title }</h2>
                }
                // A call lives in a direct chat: the family chat has nobody
                // in particular to ring, and the assistant has no ears
                // (docs/protocol.md, "Voice calls"). A member this reader
                // has blocked is not somebody to call either.
                if props.calls_enabled && props.item.chat.peer_user_id.is_some() && props.item.chat.is_direct() && !blocked_peer {
                    <span class="call-buttons">
                        <button
                            class="link"
                            disabled={props.on_call || recording_on}
                            onclick={place(false)}
                        >{ t("Call") }</button>
                        if props.video_calls_enabled {
                            <button
                                class="link"
                                disabled={props.on_call || recording_on}
                                onclick={place(true)}
                            >{ t("Video") }</button>
                        }
                    </span>
                }
                if offers.polls {
                    <button class="open-polls" onclick={
                        let on_action = props.on_action.clone();
                        Callback::from(move |_: MouseEvent| on_action.emit(Action::ShowOpenPolls { chat_id }))
                    }>
                        { t("Open polls") }
                        if props.unanswered_polls > 0 {
                            <span class="badge">{ props.unanswered_polls }</span>
                        }
                    </button>
                }
            </header>
            <div class="messages" ref={list} onscroll={on_scroll}>
                // At the top of what is HELD only: with rows held above the
                // window, scrolling up draws those first.
                if props.can_load_more && start == 0 {
                    <button class="load-more" onclick={
                        let on_action = props.on_action.clone();
                        Callback::from(move |_: MouseEvent| on_action.emit(Action::LoadMore { chat_id }))
                    }>{ t("Earlier messages") }</button>
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
                                    { tn("%lld new messages", divider_count as i64) }
                                </div>
                            }
                            <div class={classes!("row", (*highlight == Some(message.id)).then_some("is-highlighted"))}>
                                <Bubble
                                    message={message.clone()}
                                    my_user_id={props.my_user_id}
                                    names={names.clone()}
                                    blocked={blocked.clone()}
                                    {quote_revealed}
                                    {parent_revealed}
                                    quote_round={quoted.round(message)}
                                    quote_animation={quoted.animation(message)}
                                    hidden={row.hidden && !revealed}
                                    shows_sender={row.shows_sender || (row.hidden && revealed && offers.sender_names)}
                                    sender_avatar_version={avatar_versions.get(&message.sender_id).copied().unwrap_or(0)}
                                    calls_enabled={props.calls_enabled && !props.on_call && !recording_on}
                                    video_calls_enabled={props.video_calls_enabled}
                                    run_end={row.run_end}
                                    seen={row.seen}
                                    awaited={row.awaited}
                                    ai_failed={props.ai_failed.get(&message.id).cloned()}
                                    {failed}
                                    chat_kind={chat_kind_value.clone()}
                                    {assistant_user_id}
                                    in_thread={false}
                                    {member_count}
                                    member_ids={member_ids.clone()}
                                    on_action={props.on_action.clone()}
                                    on_reply={on_reply.clone()}
                                    on_edit={on_edit.clone()}
                                    on_report={on_report.clone()}
                                    on_report_assistant={on_report_assistant.clone()}
                                    on_jump={on_jump.clone()}
                                    transcription={(*transcription).clone()}
                                    transcripts={props.transcripts.of(message.attachments().iter().map(|attachment| attachment.id))}
                                />
                            </div>
                        </>
                    }
                }) }
                </>
            </div>
            // On a strip of its own under the list, not floating over it: a
            // disc over the list covered the end of the last bubble — my own,
            // which ends where it stood (#106). The list is shorter by the
            // strip while it shows; what the reader is looking at stays put.
            if !*at_newest {
                <div class="jump-strip">
                    <button class="jump-newest" onclick={jump_to_newest} aria-label={t("Jump to the newest message")}>{ "↓" }</button>
                </div>
            }
            if !props.typing.is_empty() {
                // Quiet while a recording runs: "… is typing" must never be
                // spoken into a note (S6).
                <p class="typing" aria-live={if recording_on { "off" } else { "polite" }}>{ typing_line(&props.typing) }</p>
            }
            // The voice messages not sent, the staged strip and the busy line
            // are drawn BY the composer, in the one order of #106 (doc 2.4).
            <Composer
                key={format!("composer-{chat_id}")}
                {chat_id}
                chat_kind={chat_kind_value.clone()}
                my_user_id={props.my_user_id}
                members={chat_members.clone()}
                blocked={props.blocked.clone()}
                assistant={props.assistant.clone()}
                agreed_to_assistant={props.agreed_to_assistant}
                on_review_consent={{
                    let consent_open = consent_open.clone();
                    Callback::from(move |_: ()| consent_open.set(true))
                }}
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
                on_draft={{
                    // Read when the composer GOES, which is long after this
                    // render: the reply as it is then.
                    let on_action = props.on_action.clone();
                    let replying_now = replying_now.clone();
                    Callback::from(move |text: String| on_action.emit(Action::SaveDraft {
                        chat_id,
                        text,
                        reply_to_message_id: *replying_now.borrow(),
                    }))
                }}
                attach={attach_menu}
                stickers={sticker_menu.unwrap_or_default()}
                ask_assistant={*ask}
                staged={props.staged.len()}
                {not_sent_rows}
                {staging_strip}
                {busy_line}
                busy={composer_busy}
                append={(*append).clone()}
                take={*take}
                {on_take}
                on_files={ingest}
                takes_files={true}
                {pictures}
                focus={voice.focus}
                records={Some(records)}
            />
            if voice.asking {
                // Delete at ten seconds or more: the recording stopped
                // first, and this asks (S2.5). Keep — and Esc — review it.
                <Confirm
                    title={t("Delete this recording?")}
                    confirm={t("Delete")}
                    cancel={AttrValue::from(t("Keep"))}
                    on_confirm={voice.answer.reform(|_: ()| true)}
                    on_cancel={voice.answer.reform(|_: ()| false)}
                />
            }
            // What the recording does, said (S6): a node of its own, not the
            // typing line — "Recording", "Voice message sent", "Ready to
            // review, 0:42"… Polite, and never the ticking clock.
            <div class="visually-hidden" aria-live="polite" aria-atomic="true">
                if voice.announcement.0 > 0 {
                    <span key={voice.announcement.0}>{ voice.announcement.1.clone() }</span>
                }
            </div>
            // What the video recorder says as it closes — "Video message
            // sent", "Camera turned off" — once the page is no longer inert
            // under it; while it is open it speaks through its own (S6).
            <div class="visually-hidden round-said" aria-live="polite" aria-atomic="true">
                if round_said.0 > 0 {
                    <span key={round_said.0}>{ round_said.1.clone() }</span>
                }
            </div>
            { recorder }
            { poll_dialog.unwrap_or_default() }
            { report_dialog.unwrap_or_default() }
            { assistant_report_dialog.unwrap_or_default() }
            { consent_dialog.unwrap_or_default() }
            if *group_info && props.item.chat.is_group() {
                <GroupInfoDialog
                    chat={props.item.chat.clone()}
                    members={props.members.clone()}
                    my_user_id={props.my_user_id}
                    on_action={props.on_action.clone()}
                    on_close={{
                        let group_info = group_info.clone();
                        Callback::from(move |_: ()| group_info.set(false))
                    }}
                />
            }
        </section>
    }
}

/// "Anna is typing…", "Anna and Bob are typing…", "Several people are
/// typing…" — the third because a list of five names is a line nobody
/// reads, and the count is the part that matters.
pub fn typing_line(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => t1("%@ is typing…", one),
        [one, two] => t2("%@ and %@ are typing…", one, two),
        _ => t("Several people are typing…").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn message(id: i64, sender_id: i64) -> Message {
        Message {
            id,
            chat_id: 42,
            sender_id,
            created_at: "2026-08-19T17:03:12Z".into(),
            ..Message::default()
        }
    }

    fn carrying(id: i64, round: bool, animation: bool) -> Message {
        Message {
            attachments: Some(vec![crate::model::Attachment {
                id: id * 10,
                kind: "video".into(),
                round,
                animation,
                ..crate::model::Attachment::default()
            }]),
            ..message(id, 9)
        }
    }

    fn quoting(id: i64, quoted: i64) -> Message {
        Message {
            reply_to: Some(crate::model::ReplyTo {
                message_id: quoted,
                sender_id: 9,
                excerpt: String::new(),
                parent: None,
            }),
            ..message(id, 7)
        }
    }

    /// One pass for a whole draw says, for every reply, what a pass per
    /// reply said: a video message's quote, an animation's, and neither for
    /// a plain video, a message not held, or a pending row.
    #[wasm_bindgen_test]
    fn the_quoted_kinds_of_a_draw_are_the_rule_for_every_reply() {
        let mut pending_round = carrying(0, true, false);
        pending_round.client_msg_id = Some("c".into());
        let held = vec![
            carrying(1, true, false),
            carrying(2, false, true),
            carrying(3, false, false),
            message(4, 9),
            pending_round,
        ];
        let kinds = QuotedKinds::of(&held);
        for quoted in [0, 1, 2, 3, 4, 99] {
            let reply = quoting(100 + quoted, quoted);
            assert_eq!(
                kinds.round(&reply),
                quotes_round(&reply, &held),
                "quoting {quoted}"
            );
            assert_eq!(
                kinds.animation(&reply),
                quotes_animation(&reply, &held),
                "quoting {quoted}"
            );
        }
        assert!(kinds.round(&quoting(200, 1)));
        assert!(kinds.animation(&quoting(201, 2)));
        assert!(!kinds.round(&message(5, 7)), "not a reply at all");
    }

    /// In the assistant's own chat every sender but me is drawn with the
    /// chat's name; anywhere else the roster's names are the names.
    #[wasm_bindgen_test]
    fn the_assistants_chat_names_every_other_sender_after_itself() {
        let roster = HashMap::from([(7, "Me".to_string()), (9, "Anna".to_string())]);
        assert_eq!(drawn_names(&roster, None, &[9, 2]), roster);
        let drawn = drawn_names(&roster, Some("Assistant"), &[2, 9]);
        assert_eq!(drawn.get(&2).map(String::as_str), Some("Assistant"));
        assert_eq!(drawn.get(&9).map(String::as_str), Some("Assistant"));
        assert_eq!(drawn.get(&7).map(String::as_str), Some("Me"));
    }

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
