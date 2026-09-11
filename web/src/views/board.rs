//! The family board: a wall of sticker notes anyone in the family can add
//! to and rearrange (docs/protocol.md, "Board") — drawn the way the Mac
//! draws it (ios MacViews/MacBoardView.swift), and opened the way the phone
//! opens a note (ios Views/BoardView.swift).
//!
//! Positions are FRACTIONS of the wall — a note's top-left corner, drawn
//! clamped inside it — so a note sits in the same place on every screen. A
//! drag moves the sticker at once and reports the fraction on RELEASE, read
//! back from where the sticker is drawn: one intent is one write, not sixty
//! a second. The sticker keeps where it was dropped until the server's
//! position lands, or it would jump back for a frame.
//!
//! Two authorship rules, and both are legible here: anyone may drag any
//! note, and a click OPENS it — to edit, for its author; to read, and to
//! answer if it is an event, for everybody else. A note a block hides
//! reveals on the first click and does nothing else: falling through would
//! open the very text it is hiding.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use fc_text::board::{self as rules, Answer, Font, Kind, Size};
use fc_text::i18n::{t, t1, t2, tn};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{DataTransfer, Element, File, HtmlElement, HtmlInputElement, HtmlTextAreaElement};
use yew::prelude::*;

use crate::actions::{random_color, Action};
use crate::api::{NewNote, NotePatch};
use crate::media::use_media;
use crate::model::{Attachment, Note};
use crate::time;

/// What the pane's sheet is showing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sheet {
    /// A blank note of this kind, being written.
    New(Kind),
    /// One that exists.
    Open(i64),
}

#[derive(Properties, PartialEq)]
pub struct BoardProps {
    /// In the order drawn (`Board::drawn`): the last touched on top.
    pub notes: Vec<Note>,
    /// Whether the wall has been read yet this session.
    pub loaded: bool,
    pub my_user_id: i64,
    pub names: HashMap<i64, String>,
    pub blocked: HashSet<i64>,
    /// Hidden notes peeked at.
    pub revealed: HashSet<i64>,
    /// A photo on its way up.
    pub pinning: bool,
    /// Wall-clock now, in minutes — enough to quieten an event that has
    /// passed, and coarse so the wall is not redrawn every render.
    pub now_minute: i64,
    pub on_action: Callback<Action>,
}

/// Whether the reader sees a note's content: not while its author is
/// somebody they blocked, until they peek (docs/protocol.md, "Board").
pub fn is_hidden(
    note: &Note,
    my_user_id: i64,
    blocked: &HashSet<i64>,
    revealed: &HashSet<i64>,
) -> bool {
    let author = note.author_id.unwrap_or_default();
    author != my_user_id && blocked.contains(&author) && !revealed.contains(&note.id)
}

/// Who wrote it, as the sticker signs it.
pub fn author_name(note: &Note, my_user_id: i64, names: &HashMap<i64, String>) -> String {
    let author = note.author_id.unwrap_or_default();
    if author == my_user_id {
        return t("You").to_string();
    }
    names
        .get(&author)
        .cloned()
        .unwrap_or_else(|| t("Someone").to_string())
}

/// Where a new note lands: near the middle, scattered so a run of them
/// does not stack into one pile (MacBoardView).
fn scattered() -> (f64, f64) {
    let spread = |random: f64| 0.25 + random * 0.4;
    (
        spread(js_sys::Math::random()),
        spread(js_sys::Math::random()),
    )
}

/// Where a picked photo lands (MacBoardView.pinPicture).
fn scattered_photo() -> (f64, f64) {
    let jitter = |random: f64| (random - 0.5) * 0.1;
    (
        0.35 + jitter(js_sys::Math::random()),
        0.30 + jitter(js_sys::Math::random()),
    )
}

/// What a second photo offered while one is on its way is told. A function,
/// not a const: a translated string is not a constant.
pub fn still_pinning() -> &'static str {
    t("A photo is still being pinned. Add the next one when it is on the board.")
}

/// Whether a drag carries files — the only thing a wall takes.
fn carries_files(data: &DataTransfer) -> bool {
    let types = data.types();
    (0..types.length()).any(|index| types.get(index).as_string().as_deref() == Some("Files"))
}

/// A box being watched, and the callback that must live as long as it is.
type Watch = (web_sys::ResizeObserver, Closure<dyn FnMut(js_sys::Array)>);

/// Watch `element`'s box, calling `changed` whenever it is laid out anew.
/// Answers what must be kept alive for as long as it watches.
fn watch_size(element: &Element, changed: impl Fn() + 'static) -> Option<Watch> {
    let callback = Closure::<dyn FnMut(js_sys::Array)>::new(move |_: js_sys::Array| changed());
    let observer = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()).ok()?;
    observer.observe(element);
    Some((observer, callback))
}

#[function_component(BoardPane)]
pub fn board_pane(props: &BoardProps) -> Html {
    let wall = use_node_ref();
    let size = use_state_eq(|| (0.0f64, 0.0f64));
    let sheet = use_state_eq(|| Option::<Sheet>::None);
    // A cell, like the sticker's hand: two photos dropped in one moment must
    // both see that the first is already on its way.
    let preparing = use_mut_ref(|| false);
    let redraw = use_force_update();
    let dropping = use_state_eq(|| false);
    let picker = use_node_ref();
    // False once the pane has gone: a photo still being prepared then has
    // nowhere to be pinned from, and is dropped.
    let alive = use_mut_ref(|| true);
    {
        let alive = alive.clone();
        use_effect_with((), move |_| move || *alive.borrow_mut() = false);
    }

    // The wall's size, which every position is a fraction of.
    {
        let wall = wall.clone();
        let size = size.clone();
        use_effect_with((), move |_| {
            let watching = wall.cast::<Element>().and_then(|element| {
                let measure = {
                    let element = element.clone();
                    move || {
                        size.set((
                            f64::from(element.client_width()),
                            f64::from(element.client_height()),
                        ))
                    }
                };
                measure();
                watch_size(&element, measure)
            });
            move || {
                if let Some((observer, _callback)) = watching {
                    observer.disconnect();
                }
            }
        });
    }

    let (width, height) = *size;
    let compact = rules::is_compact(width);

    // One picture onto the wall, at a fraction of it.
    let pin = {
        let on_action = props.on_action.clone();
        let preparing = preparing.clone();
        let redraw = redraw.clone();
        let alive = alive.clone();
        let pinning = props.pinning;
        Callback::from(move |(file, at): (File, (f64, f64))| {
            // One at a time, and said so: a second photo quietly dropped is a
            // photo its sender thinks is on the wall.
            if *preparing.borrow() || pinning {
                on_action.emit(Action::Fail(still_pinning().to_string()));
                return;
            }
            *preparing.borrow_mut() = true;
            redraw.force_update();
            let on_action = on_action.clone();
            let preparing = preparing.clone();
            let redraw = redraw.clone();
            let alive = alive.clone();
            spawn_local(async move {
                let prepared = crate::prep::prepare_photo(&file).await;
                if !*alive.borrow() {
                    return;
                }
                *preparing.borrow_mut() = false;
                redraw.force_update();
                match prepared {
                    Ok(photo) => on_action.emit(Action::PinPhoto { photo, at }),
                    Err(error) => on_action.emit(Action::Fail(format!(
                        "{} {}",
                        t("Couldn't pin that photo."),
                        error.message()
                    ))),
                }
            });
        })
    };

    let on_pick = {
        let pin = pin.clone();
        Callback::from(move |event: Event| {
            let Some(input) = event.target_dyn_into::<HtmlInputElement>() else {
                return;
            };
            let file = input.files().and_then(|files| files.get(0));
            // Emptied, so the same photo picked again is a change again.
            input.set_value("");
            if let Some(file) = file {
                pin.emit((file, scattered_photo()));
            }
        })
    };
    let busy = props.pinning || *preparing.borrow();
    let pick_photo = {
        let picker = picker.clone();
        Callback::from(move |_: MouseEvent| {
            if let Some(input) = picker.cast::<HtmlInputElement>() {
                input.click();
            }
        })
    };
    let open_new = |kind: Kind| {
        let sheet = sheet.clone();
        Callback::from(move |_: MouseEvent| sheet.set(Some(Sheet::New(kind))))
    };

    // Entered and left once per sticker the pointer crosses, so the outline
    // is kept by a count of how deep the drag is, not by the last event.
    let depth = use_mut_ref(|| 0i32);
    let on_drag_enter = {
        let dropping = dropping.clone();
        let depth = depth.clone();
        Callback::from(move |event: DragEvent| {
            if event
                .data_transfer()
                .is_some_and(|data| carries_files(&data))
            {
                event.prevent_default();
                *depth.borrow_mut() += 1;
                dropping.set(true);
            }
        })
    };
    let on_drag_over = Callback::from(move |event: DragEvent| {
        if event
            .data_transfer()
            .is_some_and(|data| carries_files(&data))
        {
            event.prevent_default();
        }
    });
    let on_drag_leave = {
        let dropping = dropping.clone();
        let depth = depth.clone();
        Callback::from(move |_: DragEvent| {
            let mut depth = depth.borrow_mut();
            *depth = (*depth - 1).max(0);
            if *depth == 0 {
                dropping.set(false);
            }
        })
    };
    let on_drop = {
        let dropping = dropping.clone();
        let depth = depth.clone();
        let wall = wall.clone();
        let pin = pin.clone();
        let on_action = props.on_action.clone();
        Callback::from(move |event: DragEvent| {
            *depth.borrow_mut() = 0;
            dropping.set(false);
            let Some(data) = event.data_transfer() else {
                return;
            };
            if !carries_files(&data) {
                return;
            }
            // Taken FIRST, whatever it turns out to hold: a folder dropped
            // here would otherwise be opened by the browser, in this tab,
            // navigating away from the family's board.
            event.prevent_default();
            let (files, _) = crate::views::attach::dropped_files(&data);
            let count = files.len();
            let Some(file) = files.into_iter().next() else {
                return;
            };
            if count > 1 {
                on_action.emit(Action::Fail(
                    t("The board pins one photo at a time — the first is on its way.").to_string(),
                ));
            }
            // Dropped where the pointer is, the card centred under it.
            let Some(element) = wall.cast::<Element>() else {
                return;
            };
            let rect = element.get_bounding_client_rect();
            let board = (rect.width(), rect.height());
            let card = Size::Medium.frame(rules::is_compact(board.0));
            let corner = rules::clamp_corner(
                f64::from(event.client_x()) - rect.left() - card.0 / 2.0,
                f64::from(event.client_y()) - rect.top() - card.1 / 2.0,
                card,
                board,
            );
            pin.emit((file, rules::fraction_of(corner, board)));
        })
    };

    let on_open = {
        let sheet = sheet.clone();
        Callback::from(move |note_id: i64| sheet.set(Some(Sheet::Open(note_id))))
    };
    // Closed: focus goes back to the note it was opened from, so a keyboard
    // reader is where they were rather than at the top of the page.
    let close = {
        let sheet = sheet.clone();
        let wall = wall.clone();
        Callback::from(move |()| {
            if let (Some(Sheet::Open(id)), Some(wall)) = (*sheet, wall.cast::<Element>()) {
                // Out from under the sheet first: nothing inert takes the
                // focus, and the redraw that lifts it comes after this.
                let _ = wall.remove_attribute("inert");
                if let Ok(Some(note)) = wall.query_selector(&format!("[data-note='{id}']")) {
                    if let Ok(note) = note.dyn_into::<HtmlElement>() {
                        let _ = note.focus();
                    }
                }
            }
            sheet.set(None);
        })
    };

    // Drawn in id order, stacked by how recently each was touched.
    let by_id: Vec<(Note, usize)> = {
        let mut by_id: Vec<(Note, usize)> = props
            .notes
            .iter()
            .enumerate()
            .map(|(layer, note)| (note.clone(), layer + 1))
            .collect();
        by_id.sort_by_key(|(note, _)| note.id);
        by_id
    };

    let sheet_view = (*sheet).and_then(|open| {
        let note = match open {
            Sheet::Open(id) => Some(props.notes.iter().find(|note| note.id == id).cloned()),
            Sheet::New(_) => None,
        };
        // A note hidden by a block never opens: the sheet would draw the
        // text the wall is hiding.
        if let Some(Some(note)) = &note {
            if is_hidden(note, props.my_user_id, &props.blocked, &props.revealed) {
                return None;
            }
        }
        let (mine, author) = match &note {
            Some(Some(note)) => (
                note.author_id == Some(props.my_user_id),
                author_name(note, props.my_user_id, &props.names),
            ),
            _ => (true, t("You").to_string()),
        };
        let gone = matches!(note, Some(None));
        Some(html! {
            <NoteSheet
                key={format!("{open:?}")}
                sheet={open}
                note={note.flatten()}
                {gone}
                {mine}
                author={AttrValue::from(author)}
                my_user_id={props.my_user_id}
                {compact}
                now_minute={props.now_minute}
                on_close={close.clone()}
                on_action={props.on_action.clone()}
            />
        })
    });

    // While a sheet is open nothing behind it takes a click or the focus:
    // Tab from its last button used to reach the stickers, where the arrow
    // keys moved them under a dialog.
    let behind = sheet_view.is_some().then_some("");
    html! {
        <section class="board" aria-label={t("Board")}>
            <header class="board-bar" inert={behind}>
                <h2 class="board-title">{ t("Board") }</h2>
                <div class="board-actions">
                    <button class="secondary" onclick={open_new(Kind::Text)} title={t("Add a note")}>{ t("Add Note") }</button>
                    <button class="secondary" onclick={open_new(Kind::Event)} title={t("Add an event")}>{ t("Add Event") }</button>
                    <button class="secondary" onclick={pick_photo} disabled={busy} aria-busy={busy.then_some("true")} title={t("Pin a photo")}>
                        { if busy { t("Pinning…") } else { t("Pin a Photo") } }
                    </button>
                </div>
                <input
                    ref={picker}
                    class="hidden-picker"
                    type="file"
                    accept="image/*"
                    onchange={on_pick}
                    tabindex="-1"
                    aria-hidden="true"
                />
            </header>
            <div
                ref={wall}
                inert={behind}
                class={classes!("board-wall", (*dropping).then_some("is-drop-target"))}
                ondragenter={on_drag_enter}
                ondragover={on_drag_over}
                ondragleave={on_drag_leave}
                ondrop={on_drop}
            >
                if width > 0.0 && height > 0.0 {
                    // Keyed, and in a list of their own (the YEW KEYS TRAP):
                    // one unkeyed sibling would hand a sticker's drag to its
                    // neighbour. And in a STABLE order — by id, so a new note
                    // only ever joins the end — with recency drawn by
                    // z-index: a sticker the browser MOVED in the page would
                    // lose the pointer it had captured, mid-drag, whenever
                    // somebody else touched a note.
                    <>
                        { for by_id.iter().map(|(note, layer)| {
                            let hidden = is_hidden(note, props.my_user_id, &props.blocked, &props.revealed);
                            html! {
                                <Sticker
                                    key={note.id}
                                    layer={*layer}
                                    note={note.clone()}
                                    mine={note.author_id == Some(props.my_user_id)}
                                    author={AttrValue::from(author_name(note, props.my_user_id, &props.names))}
                                    {hidden}
                                    wall={(width, height)}
                                    {compact}
                                    now_minute={props.now_minute}
                                    on_open={on_open.clone()}
                                    on_action={props.on_action.clone()}
                                />
                            }
                        }) }
                    </>
                }
                if !props.loaded {
                    <p class="board-empty">{ t("Loading the board…") }</p>
                } else if props.notes.is_empty() {
                    <div class="board-empty">
                        <p class="board-empty-title">{ t("The board is empty") }</p>
                        <p>{ t("Add a note — everyone in the family sees it.") }</p>
                    </div>
                }
            </div>
            { sheet_view.unwrap_or_default() }
        </section>
    }
}

/// A drag in progress: which pointer, where it went down, where the card
/// was drawn when it began, how far it has come, and whether that is far
/// enough to be a drag rather than a click.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Drag {
    pointer: i32,
    down: (f64, f64),
    /// The card's corner, in wall pixels, as drawn when the drag began — so
    /// the note stays under the pointer even if the wall moves it meanwhile.
    from: (f64, f64),
    delta: (f64, f64),
    moved: bool,
}

/// The arrow keys' "pointer".
const KEYBOARD: i32 = -1;

/// How far a pointer may wander before a press is a drag, not a click.
const CLICK_SLOP: f64 = 4.0;

/// A sticker in hand. Held in a cell, not in `use_state`: pointer events
/// arrive many times between two renders, and each must see the last one's
/// work rather than the render's snapshot of it.
#[derive(Debug, Default)]
struct Hand {
    drag: Option<Drag>,
    /// Where it was dropped — the fraction of the wall that was sent — and
    /// which drop that was, held until that move is answered.
    held: Option<(u64, (f64, f64))>,
    drops: u64,
    /// A move of this note is on its way. One at a time: moves sent side by
    /// side commit in whatever order the server locks them, and the note
    /// could end at the second of three places on every device.
    sending: bool,
    /// The drop waiting behind it — only ever the latest.
    queued: Option<(u64, (f64, f64))>,
}

impl Hand {
    /// Where the card is drawn right now, in wall pixels: in hand, where
    /// the drag has it; dropped, where it was dropped; otherwise where the
    /// server has it.
    fn corner(&self, fraction: (f64, f64), card: (f64, f64), wall: (f64, f64)) -> (f64, f64) {
        match (self.drag.filter(|drag| drag.moved), self.held) {
            (Some(drag), _) => rules::clamp_corner(
                drag.from.0 + drag.delta.0,
                drag.from.1 + drag.delta.1,
                card,
                wall,
            ),
            (None, Some((_, target))) => rules::origin(target, card, wall),
            (None, None) => rules::origin(fraction, card, wall),
        }
    }
}

/// Send one move, and when it is answered either send the drop that waited
/// behind it or, if none did, let go of where the note was held.
fn send_move(
    hand: Rc<std::cell::RefCell<Hand>>,
    redraw: UseForceUpdateHandle,
    on_action: Callback<Action>,
    note_id: i64,
    which: u64,
    target: (f64, f64),
) {
    let done = {
        let hand = hand.clone();
        let redraw = redraw.clone();
        let on_action = on_action.clone();
        Callback::from(move |()| {
            let next = {
                let mut held = hand.borrow_mut();
                held.sending = false;
                match held.queued.take() {
                    Some(queued) => {
                        held.sending = true;
                        Some(queued)
                    }
                    None => {
                        if held.held.is_some_and(|(drop, _)| drop == which) {
                            held.held = None;
                        }
                        None
                    }
                }
            };
            if let Some((next, target)) = next {
                send_move(
                    hand.clone(),
                    redraw.clone(),
                    on_action.clone(),
                    note_id,
                    next,
                    target,
                );
            }
            redraw.force_update();
        })
    };
    on_action.emit(Action::MoveNote {
        note_id,
        x: target.0,
        y: target.1,
        done,
    });
}

#[derive(Properties, PartialEq)]
struct StickerProps {
    /// Where it stacks: the most recently touched note highest.
    layer: usize,
    note: Note,
    mine: bool,
    author: AttrValue,
    hidden: bool,
    wall: (f64, f64),
    compact: bool,
    now_minute: i64,
    on_open: Callback<i64>,
    on_action: Callback<Action>,
}

#[function_component(Sticker)]
fn sticker(props: &StickerProps) -> Html {
    let note = &props.note;
    let size = note.size();
    let card = size.frame(props.compact);
    let fraction = note.position();
    let wall = props.wall;
    let hand = use_mut_ref(Hand::default);
    let redraw = use_force_update();
    let node = use_node_ref();

    // The server's position has landed with nothing of ours on the way: let
    // go of anything still held.
    {
        let hand = hand.clone();
        let redraw = redraw.clone();
        use_effect_with(fraction, move |_| {
            let mut held = hand.borrow_mut();
            if !held.sending && held.held.take().is_some() {
                drop(held);
                redraw.force_update();
            }
        });
    }

    let corner = hand.borrow().corner(fraction, card, wall);
    let dragging = hand.borrow().drag.is_some_and(|drag| drag.moved);
    let held_now = hand.borrow().held.is_some();

    let note_id = note.id;
    // Drop: the fraction of where the card is DRAWN, held there until the
    // move is answered — behind the one on its way, if there is one.
    let commit = {
        let hand = hand.clone();
        let redraw = redraw.clone();
        let on_action = props.on_action.clone();
        Rc::new(move |drawn: (f64, f64)| {
            let target = rules::fraction_of(drawn, wall);
            let send = {
                let mut held = hand.borrow_mut();
                held.drops += 1;
                let which = held.drops;
                held.held = Some((which, target));
                if held.sending {
                    held.queued = Some((which, target));
                    None
                } else {
                    held.sending = true;
                    Some(which)
                }
            };
            redraw.force_update();
            if let Some(which) = send {
                send_move(
                    hand.clone(),
                    redraw.clone(),
                    on_action.clone(),
                    note_id,
                    which,
                    target,
                );
            }
        })
    };
    let click = {
        let on_open = props.on_open.clone();
        let on_action = props.on_action.clone();
        let hidden = props.hidden;
        Rc::new(move || {
            if hidden {
                on_action.emit(Action::RevealNote { note_id });
            } else {
                on_open.emit(note_id);
            }
        })
    };

    let on_down = {
        let hand = hand.clone();
        let node = node.clone();
        Callback::from(move |event: PointerEvent| {
            if event.button() != 0 {
                return;
            }
            // Captured by the STICKER, so the rest of the drag comes to it
            // wherever the pointer goes. Not by `current_target`: Yew listens
            // at the app's root, and a pointer captured there takes every
            // move and the release away from the note for good.
            if let Some(sticker) = node.cast::<Element>() {
                let _ = sticker.set_pointer_capture(event.pointer_id());
            }
            let mut held = hand.borrow_mut();
            // From where it is DRAWN — picked up again before its last move
            // landed, that is where it was dropped, never an offset past the
            // edge the pointer would first have to travel back.
            let from = held.corner(fraction, card, wall);
            held.drag = Some(Drag {
                pointer: event.pointer_id(),
                down: (f64::from(event.client_x()), f64::from(event.client_y())),
                from,
                delta: (0.0, 0.0),
                moved: false,
            });
        })
    };
    let on_move = {
        let hand = hand.clone();
        let redraw = redraw.clone();
        Callback::from(move |event: PointerEvent| {
            let mut held = hand.borrow_mut();
            let Some(drag) = held.drag.as_mut() else {
                return;
            };
            if drag.pointer != event.pointer_id() {
                return;
            }
            let delta = (
                f64::from(event.client_x()) - drag.down.0,
                f64::from(event.client_y()) - drag.down.1,
            );
            drag.moved = drag.moved || delta.0.hypot(delta.1) > CLICK_SLOP;
            if drag.moved {
                drag.delta = delta;
                drop(held);
                redraw.force_update();
            }
        })
    };
    let on_up = {
        let hand = hand.clone();
        let commit = commit.clone();
        let click = click.clone();
        Callback::from(move |event: PointerEvent| {
            let (drag, drawn) = {
                let mut held = hand.borrow_mut();
                let drawn = held.corner(fraction, card, wall);
                match held.drag {
                    Some(drag) if drag.pointer == event.pointer_id() => (held.drag.take(), drawn),
                    _ => (None, drawn),
                }
            };
            match drag {
                Some(drag) if drag.moved => commit(drawn),
                Some(_) => click(),
                None => {}
            }
        })
    };
    // A drag the browser took away — cancelled, or its capture lost — is put
    // back where it was, not dropped where it happened to be. A release has
    // already ended the drag by the time its capture goes, so this does
    // nothing after an ordinary drop; nor to the arrow keys' drag.
    let on_cancel = {
        let hand = hand.clone();
        let redraw = redraw.clone();
        Callback::from(move |event: PointerEvent| {
            let mut held = hand.borrow_mut();
            if held
                .drag
                .is_some_and(|drag| drag.pointer == event.pointer_id())
            {
                held.drag = None;
                drop(held);
                redraw.force_update();
            }
        })
    };
    // The keyboard's way to do all of it: Enter or Space opens (or
    // reveals), the arrows move — a hundredth of the wall a press, a
    // twentieth with Shift — and letting go of an arrow puts it down.
    let on_key_down = {
        let hand = hand.clone();
        let redraw = redraw.clone();
        let click = click.clone();
        Callback::from(move |event: KeyboardEvent| {
            let key = event.key();
            if key == "Enter" || key == " " {
                event.prevent_default();
                click();
                return;
            }
            let step = if event.shift_key() { 0.05 } else { 0.01 };
            let (dx, dy) = match key.as_str() {
                "ArrowLeft" => (-step * wall.0, 0.0),
                "ArrowRight" => (step * wall.0, 0.0),
                "ArrowUp" => (0.0, -step * wall.1),
                "ArrowDown" => (0.0, step * wall.1),
                _ => return,
            };
            event.prevent_default();
            let mut held = hand.borrow_mut();
            // From where it is drawn, kept to where it CAN be drawn, so a
            // press past the edge is not owed back before the next one moves.
            let now = held.corner(fraction, card, wall);
            let from = rules::clamp_corner(now.0 + dx, now.1 + dy, card, wall);
            held.drag = Some(Drag {
                pointer: KEYBOARD,
                down: (0.0, 0.0),
                from,
                delta: (0.0, 0.0),
                moved: true,
            });
            drop(held);
            redraw.force_update();
        })
    };
    // Focus gone mid-move — Tab, or another window — never comes back for
    // its key-up: the move is put down where it was taken, not left lifted.
    let on_blur = {
        let hand = hand.clone();
        let commit = commit.clone();
        Callback::from(move |_: FocusEvent| {
            let drawn = {
                let mut held = hand.borrow_mut();
                let drawn = held.corner(fraction, card, wall);
                match held.drag {
                    Some(drag) if drag.pointer == KEYBOARD => {
                        held.drag = None;
                        Some(drawn)
                    }
                    _ => None,
                }
            };
            if let Some(drawn) = drawn {
                commit(drawn);
            }
        })
    };
    let on_key_up = {
        let hand = hand.clone();
        let commit = commit.clone();
        Callback::from(move |event: KeyboardEvent| {
            if !event.key().starts_with("Arrow") {
                return;
            }
            let drawn = {
                let mut held = hand.borrow_mut();
                let drawn = held.corner(fraction, card, wall);
                match held.drag {
                    Some(drag) if drag.pointer == KEYBOARD => {
                        held.drag = None;
                        Some(drawn)
                    }
                    _ => None,
                }
            };
            if let Some(drawn) = drawn {
                commit(drawn);
            }
        })
    };

    let kind = note.kind();
    let hidden = props.hidden;
    let picture = (!hidden && kind == Kind::Photo)
        .then(|| note.attachment.clone())
        .flatten();
    let caption = note.text().trim().to_string();
    let event_block = (!hidden && kind == Kind::Event)
        .then(|| note.starts_at.clone())
        .flatten()
        .map(|starts_at| {
            html! {
                <EventBlock
                    starts_at={AttrValue::from(starts_at.clone())}
                    ends_at={note.ends_at.clone().map(AttrValue::from)}
                    place={note.place.clone().filter(|place| !place.is_empty()).map(AttrValue::from)}
                    going={note.count(Answer::Going)}
                    maybe={note.count(Answer::Maybe)}
                    past={time::is_past(&starts_at, note.ends_at.as_deref(), props.now_minute as f64 * 60_000.0)}
                />
            }
        });
    let label = if hidden {
        t("Hidden note from a blocked member").to_string()
    } else {
        let mut what = match (kind, caption.is_empty()) {
            (Kind::Photo, true) => t("a photo").to_string(),
            _ => caption.clone(),
        };
        // The label stands in for the card's content, so an event's when,
        // where and who is coming must be in it, or a screen reader hears
        // the title and nothing that makes it an event.
        if let (Kind::Event, Some(starts)) = (kind, note.starts_at.as_deref()) {
            what.push_str(", ");
            what.push_str(&time::event_when(starts, note.ends_at.as_deref()));
            if let Some(place) = note.place.as_deref().filter(|place| !place.is_empty()) {
                what.push_str(", ");
                what.push_str(place);
            }
            if let Some(line) =
                rules::going_line(note.count(Answer::Going), note.count(Answer::Maybe))
            {
                what.push_str(", ");
                what.push_str(&line);
            }
        }
        if props.mine {
            t1("Your note: %@", &what)
        } else {
            t2("Note from %@: %@", &props.author, &what)
        }
    };
    let style = format!(
        "left:{:.1}px;top:{:.1}px;width:{}px;height:{}px;background:{};transform:rotate({}deg){};z-index:{};",
        corner.0,
        corner.1,
        card.0,
        card.1,
        rules::color_hex(note.color.as_deref().unwrap_or_default()),
        rules::tilt_degrees(note.id),
        if dragging { " scale(1.04)" } else { "" },
        // In hand above everything on the wall — and still there once put
        // down, until the move is answered and it is the newest anyway,
        // rather than sinking under later notes for the moment between.
        if dragging {
            100_000
        } else if held_now {
            99_999
        } else {
            props.layer
        },
    );
    html! {
        <div
            ref={node}
            data-note={note.id.to_string()}
            class={classes!(
                "sticker",
                dragging.then_some("is-dragging"),
                hidden.then_some("is-hidden"),
                picture.is_some().then_some("is-photo"),
                (!caption.is_empty()).then_some("has-caption"),
            )}
            {style}
            role="button"
            tabindex="0"
            aria-label={label}
            aria-description={t("Enter opens it; the arrow keys move it.")}
            onpointerdown={on_down}
            onpointermove={on_move}
            onpointerup={on_up}
            onpointercancel={on_cancel.clone()}
            onlostpointercapture={on_cancel}
            onkeydown={on_key_down}
            onkeyup={on_key_up}
            onblur={on_blur}
        >
            if let Some(attachment) = picture {
                <NotePicture {attachment} />
            }
            { event_block.unwrap_or_default() }
            if hidden {
                <FittedText text={t("Hidden — blocked member")} font={note.font()} {size} class={classes!("note-hidden")} />
            } else if kind != Kind::Photo || !caption.is_empty() {
                <FittedText text={AttrValue::from(note.text().to_string())} font={note.font()} {size} />
            }
            // No author line at all while hidden — not an empty one, which
            // would still say a note came from somebody.
            if !hidden {
                <span class="note-author">{ props.author.clone() }</span>
            }
        </div>
    }
}

#[derive(Properties, PartialEq)]
pub struct FittedProps {
    pub text: AttrValue,
    pub font: Font,
    pub size: Size,
    #[prop_or_default]
    pub class: Classes,
}

/// A note's text, FITTED to its card (docs/protocol.md, "Board"): drawn at
/// its size's own type, scaled down until all of it is inside — and only
/// below the floor cut, with an ellipsis, at the lines there is room for.
/// Refitted whenever the text, the face or the step changes, and whenever
/// the box it has to fit is laid out anew.
#[function_component(FittedText)]
pub fn fitted_text(props: &FittedProps) -> Html {
    let node = use_node_ref();
    let base = props.size.type_px();
    {
        let node = node.clone();
        use_effect_with((props.text.clone(), props.font, props.size), move |_| {
            // Fitted from the observer alone: its first callback comes as it
            // starts watching, before the frame is painted, so fitting here
            // as well measured every note twice on a wall of them.
            let watching = node.cast::<HtmlElement>().and_then(|element| {
                let watched = element.clone();
                watch_size(&element, move || fit(&watched, base))
            });
            move || {
                if let Some((observer, _callback)) = watching {
                    observer.disconnect();
                }
            }
        });
    }
    html! {
        <div
            ref={node}
            class={classes!("note-text", props.class.clone())}
            style={format!("font-family:{};font-size:{base}px;", props.font.css_family())}
        >
            { props.text.clone() }
        </div>
    }
}

/// A textarea's value held to `max` scalars, cut at the caret
/// (`fc_text::board::cap_at_caret`), with the caret put back where it was.
fn capped_area(area: &HtmlTextAreaElement, max: usize) -> String {
    let value = area.value();
    let caret = area
        .selection_start()
        .ok()
        .flatten()
        .map_or_else(|| value.encode_utf16().count(), |caret| caret as usize);
    let (kept, caret) = rules::cap_at_caret(&value, caret, max);
    if kept != value {
        area.set_value(&kept);
        let _ = area.set_selection_range(caret as u32, caret as u32);
    }
    kept
}

/// The same, for a one-line input.
fn capped_input(input: &HtmlInputElement, max: usize) -> String {
    let value = input.value();
    let caret = input
        .selection_start()
        .ok()
        .flatten()
        .map_or_else(|| value.encode_utf16().count(), |caret| caret as usize);
    let (kept, caret) = rules::cap_at_caret(&value, caret, max);
    if kept != value {
        input.set_value(&kept);
        let _ = input.set_selection_range(caret as u32, caret as u32);
    }
    kept
}

/// The type line height, as the stylesheet sets it on `.note-text`.
const LINE_HEIGHT: f64 = 1.2;

/// Fit `element`'s text to its box.
pub fn fit(element: &HtmlElement, base: f64) {
    let style = element.style();
    let class = element.class_list();
    let _ = class.remove_1("is-cut");
    let _ = style.remove_property("-webkit-line-clamp");
    let at = |scale: f64| {
        let _ = style.set_property("font-size", &format!("{:.2}px", base * scale));
    };
    let fits = |scale: f64| {
        at(scale);
        element.scroll_height() <= element.client_height() + 1
            && element.scroll_width() <= element.client_width() + 1
    };
    match rules::fitted_scale(fits, rules::MIN_TEXT_SCALE, rules::FIT_STEPS) {
        Some(scale) => at(scale),
        None => {
            // Past the floor: cut, with an ellipsis, at the lines it holds.
            at(rules::MIN_TEXT_SCALE);
            let line = base * rules::MIN_TEXT_SCALE * LINE_HEIGHT;
            let lines = rules::lines_that_fit(f64::from(element.client_height()), line);
            let _ = class.add_1("is-cut");
            let _ = style.set_property("-webkit-line-clamp", &lines.to_string());
        }
    }
}

#[derive(Properties, PartialEq)]
struct PictureProps {
    attachment: Attachment,
}

/// The picture on a photo note — its preview, which is what a card wants,
/// through the same cache a message's photo comes from.
#[function_component(NotePicture)]
fn note_picture(props: &PictureProps) -> Html {
    let source = crate::views::attachments::tile_source(&props.attachment);
    let url = use_media(
        props.attachment.id,
        source.unwrap_or(crate::media::Variant::Preview),
        source.is_some(),
    );
    html! {
        <div class={classes!("note-picture", url.is_none().then_some("is-loading"))}>
            if let Some(url) = url {
                <img src={url} alt="" draggable="false" />
            }
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct EventProps {
    starts_at: AttrValue,
    ends_at: Option<AttrValue>,
    place: Option<AttrValue>,
    going: usize,
    maybe: usize,
    past: bool,
}

/// When, where and who is coming — above an event's title, because the
/// date is the reason it is on the wall (NoteEventBlock).
#[function_component(EventBlock)]
fn event_block(props: &EventProps) -> Html {
    let when = time::event_when(&props.starts_at, props.ends_at.as_deref());
    html! {
        <div class={classes!("note-event", props.past.then_some("is-past"))}>
            <span class="note-when">{ when }</span>
            if let Some(place) = props.place.clone() {
                <span class="note-place">{ place }</span>
            }
            if let Some(line) = rules::going_line(props.going, props.maybe) {
                <span class="note-going">{ line }</span>
            }
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct SheetProps {
    sheet: Sheet,
    /// The note as it stands now, for an open one.
    note: Option<Note>,
    /// It was open, and somebody has taken it down.
    gone: bool,
    mine: bool,
    author: AttrValue,
    my_user_id: i64,
    compact: bool,
    now_minute: i64,
    on_close: Callback<()>,
    on_action: Callback<Action>,
}

/// What the author is writing: the note's words and look, and an event's
/// when and where. Kept apart from the note so a save sends only what
/// changed.
#[derive(Debug, Clone, PartialEq)]
struct Draft {
    text: String,
    color: String,
    size: Size,
    font: Font,
    starts: String,
    has_end: bool,
    ends: String,
    place: String,
}

impl Draft {
    fn blank(kind: Kind, now_ms: f64) -> Draft {
        let starts = time::next_round_hour(now_ms);
        let ends = time::next_round_hour(now_ms + 3_600_000.0);
        Draft {
            text: String::new(),
            // An event is blue, as on the phone; a note any colour at all.
            color: if kind == Kind::Event {
                "blue".to_string()
            } else {
                random_color()
            },
            size: Size::Medium,
            font: Font::Plain,
            starts: time::local_input(&starts),
            has_end: false,
            ends: time::local_input(&ends),
            place: String::new(),
        }
    }

    fn of(note: &Note) -> Draft {
        let starts = note.starts_at.clone().unwrap_or_default();
        // An hour after the start is the shape most family things take,
        // and only a starting point for the picker.
        let ends = note
            .ends_at
            .clone()
            .or_else(|| time::shifted(&starts, HOUR_MS))
            .unwrap_or_default();
        Draft {
            text: note.text().to_string(),
            color: note.color.clone().unwrap_or_else(|| "yellow".to_string()),
            size: note.size(),
            font: note.font(),
            starts: time::local_input(&starts),
            has_end: note.ends_at.is_some(),
            ends: time::local_input(&ends),
            place: note.place.clone().unwrap_or_default(),
        }
    }

    /// A start moved past the end takes the end with it, an hour on — the
    /// way Android's editor keeps the pair, rather than refusing the save.
    fn keep_end_after_start(&mut self) {
        let at = |value: &str| time::from_local_input(value).and_then(|at| time::instant(&at));
        if let (Some(starts), Some(ends)) = (at(&self.starts), at(&self.ends)) {
            if ends < starts {
                if let Some(later) = time::from_local_input(&self.starts)
                    .and_then(|starts| time::shifted(&starts, HOUR_MS))
                {
                    self.ends = time::local_input(&later);
                }
            }
        }
    }

    /// Why this cannot be saved as it stands, if it cannot.
    fn problem(&self, kind: Kind) -> Option<&'static str> {
        // A photo's caption may be empty; a note and an event's title not.
        if kind != Kind::Photo && self.text.trim().is_empty() {
            return Some("");
        }
        if kind == Kind::Event {
            let starts = time::from_local_input(&self.starts).and_then(|at| time::instant(&at));
            let Some(starts) = starts else {
                return Some(t("Pick when it starts."));
            };
            if self.has_end {
                let ends = time::from_local_input(&self.ends).and_then(|at| time::instant(&at));
                match ends {
                    None => return Some(t("Pick when it ends, or turn the end off.")),
                    Some(ends) if ends < starts => {
                        return Some(t("The end can't be before the start."))
                    }
                    Some(_) => {}
                }
            }
        }
        None
    }

    /// A new note of `kind`, dropped at `at`.
    fn new_note(&self, kind: Kind, at: (f64, f64)) -> NewNote {
        let event = kind == Kind::Event;
        let place = self.place.trim();
        NewNote {
            text: self.text.clone(),
            color: self.color.clone(),
            size: self.size.name().to_string(),
            font: self.font.name().to_string(),
            x: at.0,
            y: at.1,
            kind: event.then(|| Kind::Event.name().to_string()),
            attachment_id: None,
            starts_at: event
                .then(|| time::from_local_input(&self.starts))
                .flatten(),
            ends_at: (event && self.has_end)
                .then(|| time::from_local_input(&self.ends))
                .flatten(),
            place: (event && !place.is_empty()).then(|| place.to_string()),
        }
    }

    /// What changed against `note` — and nothing else, so a size or a face
    /// this client does not know is not written back as its default, and a
    /// save that changed nothing sends nothing.
    fn patch(&self, note: &Note) -> NotePatch {
        let mut patch = NotePatch::default();
        if self.text.trim() != note.text().trim() {
            patch.text = Some(self.text.clone());
        }
        if Some(self.color.as_str()) != note.color.as_deref() {
            patch.color = Some(self.color.clone());
        }
        patch.size = self
            .size
            .patch_name(note.size.as_deref())
            .map(str::to_string);
        patch.font = self
            .font
            .patch_name(note.font.as_deref())
            .map(str::to_string);
        if note.kind() == Kind::Event {
            let stored = |at: Option<&str>| at.and_then(time::instant);
            let starts = time::from_local_input(&self.starts);
            if starts.as_deref().and_then(time::instant) != stored(note.starts_at.as_deref()) {
                patch.starts_at = starts;
            }
            let ends = if self.has_end {
                time::from_local_input(&self.ends)
            } else {
                None
            };
            if ends.as_deref().and_then(time::instant) != stored(note.ends_at.as_deref()) {
                patch.ends_at = Some(ends);
            }
            if self.place.trim() != note.place.as_deref().unwrap_or_default().trim() {
                patch.place = Some(self.place.trim().to_string());
            }
        }
        patch
    }
}

const HOUR_MS: f64 = 3_600_000.0;

/// The sheet's own state: the draft, and where saving it has got to.
#[derive(Debug, Clone)]
struct Editing {
    draft: Draft,
    /// The note as it stood when the sheet opened — what a save is a change
    /// FROM. Diffing against the note as it stands now sent the author's
    /// stale copy of anything changed meanwhile on another device: the
    /// colour they had just set on their phone, put back.
    opened: Option<Note>,
    saving: bool,
    error: Option<String>,
    confirming: bool,
    /// An answer to an event on its way, shown before it lands.
    answering: Option<Option<Answer>>,
    answer_sending: bool,
    answer_queued: Option<Option<Answer>>,
}

/// Send one answer; when it is in, send the one that waited behind it, or
/// go back to showing what the note itself says.
fn send_answer(
    cell: Rc<std::cell::RefCell<Editing>>,
    redraw: UseForceUpdateHandle,
    on_action: Callback<Action>,
    note_id: i64,
    choice: Option<Answer>,
) {
    let done = {
        let cell = cell.clone();
        let redraw = redraw.clone();
        let on_action = on_action.clone();
        Callback::from(move |()| {
            let next = {
                let mut editing = cell.borrow_mut();
                editing.answer_sending = false;
                match editing.answer_queued.take() {
                    Some(next) => {
                        editing.answer_sending = true;
                        Some(next)
                    }
                    None => {
                        editing.answering = None;
                        None
                    }
                }
            };
            if let Some(next) = next {
                send_answer(
                    cell.clone(),
                    redraw.clone(),
                    on_action.clone(),
                    note_id,
                    next,
                );
            }
            redraw.force_update();
        })
    };
    on_action.emit(Action::AnswerEvent {
        note_id,
        answer: choice.map(|choice| choice.name().to_string()),
        done,
    });
}

/// The note, opened: written by its author, read — and answered, for an
/// event — by everybody else (ios BoardView.NoteEditor).
#[function_component(NoteSheet)]
fn note_sheet(props: &SheetProps) -> Html {
    let now_ms = props.now_minute as f64 * 60_000.0;
    let kind = match props.sheet {
        Sheet::New(kind) => kind,
        Sheet::Open(_) => props.note.as_ref().map(Note::kind).unwrap_or(Kind::Text),
    };
    // Held in a cell and not in `use_state`: each state handle is the
    // render's SNAPSHOT, so two edits landing before a redraw — a word typed
    // and a box ticked in the same moment — would each write their own copy
    // over the other's, and a double click on Save would send twice.
    let cell = {
        let note = props.note.clone();
        use_mut_ref(move || Editing {
            draft: match &note {
                Some(note) => Draft::of(note),
                None => Draft::blank(kind, now_ms),
            },
            opened: note.clone(),
            saving: false,
            error: None,
            confirming: false,
            answering: None,
            answer_sending: false,
            answer_queued: None,
        })
    };
    let redraw = use_force_update();
    let field = use_node_ref();
    let dialog = use_node_ref();
    let editable = props.mine && !props.gone;

    // Focus INTO the sheet as it opens — the words, for the author, and the
    // sheet itself for everyone else. Left on the note behind it, Escape
    // would never reach the sheet, and the arrow keys would go on moving the
    // note under a dialog nobody could close from the keyboard.
    {
        let field = field.clone();
        let dialog = dialog.clone();
        use_effect_with((), move |_| {
            if let Some(field) = field.cast::<HtmlElement>() {
                let _ = field.focus();
            } else if let Some(dialog) = dialog.cast::<HtmlElement>() {
                let _ = dialog.focus();
            }
        });
    }

    let edit = |change: fn(&mut Draft, String)| {
        let cell = cell.clone();
        let redraw = redraw.clone();
        move |value: String| {
            change(&mut cell.borrow_mut().draft, value);
            redraw.force_update();
        }
    };
    // The cap where the typing is, counted as the server counts — a full
    // field, never a refused save — and taken out of what was just typed or
    // pasted, never off the end of what was already there. Not while an
    // input method is still composing: rewriting the field under it breaks
    // the composition, so it is capped when the composition ends.
    let on_text = {
        let set = Rc::new(edit(|draft, value| draft.text = value));
        let on_input = {
            let set = set.clone();
            Callback::from(move |event: InputEvent| {
                if event.is_composing() {
                    return;
                }
                if let Some(area) = event.target_dyn_into::<HtmlTextAreaElement>() {
                    set(capped_area(&area, rules::MAX_TEXT_CHARS));
                }
            })
        };
        let on_composed = Callback::from(move |event: web_sys::Event| {
            if let Some(area) = event.target_dyn_into::<HtmlTextAreaElement>() {
                set(capped_area(&area, rules::MAX_TEXT_CHARS));
            }
        });
        (on_input, on_composed)
    };
    let on_place = {
        let set = Rc::new(edit(|draft, value| draft.place = value));
        let on_input = {
            let set = set.clone();
            Callback::from(move |event: InputEvent| {
                if event.is_composing() {
                    return;
                }
                if let Some(input) = event.target_dyn_into::<HtmlInputElement>() {
                    set(capped_input(&input, rules::MAX_PLACE_CHARS));
                }
            })
        };
        let on_composed = Callback::from(move |event: web_sys::Event| {
            if let Some(input) = event.target_dyn_into::<HtmlInputElement>() {
                set(capped_input(&input, rules::MAX_PLACE_CHARS));
            }
        });
        (on_input, on_composed)
    };
    // `compositionend` by hand: Yew has no listener attribute for it.
    let place_field = use_node_ref();
    {
        let field = field.clone();
        let place_field = place_field.clone();
        let text_done = on_text.1.clone();
        let place_done = on_place.1.clone();
        use_effect_with((), move |_| {
            let listen = |node: &NodeRef, done: Callback<web_sys::Event>| {
                node.cast::<Element>().map(|target| {
                    let heard =
                        Closure::<dyn Fn(web_sys::Event)>::new(move |event: web_sys::Event| {
                            done.emit(event)
                        });
                    let _ = target.add_event_listener_with_callback(
                        "compositionend",
                        heard.as_ref().unchecked_ref(),
                    );
                    (target, heard)
                })
            };
            let listening = [listen(&field, text_done), listen(&place_field, place_done)];
            move || {
                for (target, heard) in listening.into_iter().flatten() {
                    let _ = target.remove_event_listener_with_callback(
                        "compositionend",
                        heard.as_ref().unchecked_ref(),
                    );
                }
            }
        });
    }
    let on_starts = {
        let set = edit(|draft, value| {
            draft.starts = value;
            draft.keep_end_after_start();
        });
        Callback::from(move |event: Event| {
            if let Some(input) = event.target_dyn_into::<HtmlInputElement>() {
                set(input.value());
            }
        })
    };
    let on_ends = {
        let set = edit(|draft, value| draft.ends = value);
        Callback::from(move |event: Event| {
            if let Some(input) = event.target_dyn_into::<HtmlInputElement>() {
                set(input.value());
            }
        })
    };
    let on_has_end = {
        let set = edit(|draft, value| draft.has_end = value == "on");
        Callback::from(move |event: Event| {
            if let Some(input) = event.target_dyn_into::<HtmlInputElement>() {
                set(if input.checked() { "on" } else { "off" }.to_string());
            }
        })
    };
    let choose_color = |name: &'static str| {
        let set = edit(|draft, value| draft.color = value);
        Callback::from(move |_: Event| set(name.to_string()))
    };
    let choose_size = |size: Size| {
        let set = edit(|draft, value| draft.size = Size::from_name(Some(&value)));
        Callback::from(move |_: Event| set(size.name().to_string()))
    };
    let choose_font = |font: Font| {
        let set = edit(|draft, value| draft.font = Font::from_name(Some(&value)));
        Callback::from(move |_: Event| set(font.name().to_string()))
    };

    // False once the sheet has gone — closed while a save was on its way.
    let open = use_mut_ref(|| true);
    {
        let open = open.clone();
        use_effect_with((), move |_| move || *open.borrow_mut() = false);
    }

    // Heard once the server has it — the sheet closes — or with what went
    // wrong, which it shows and keeps the words beside. A refusal for a
    // sheet already closed goes to the bar instead: `board_full` is said to
    // the person, whatever they pressed while it was on its way.
    let done = {
        let on_close = props.on_close.clone();
        let on_action = props.on_action.clone();
        let cell = cell.clone();
        let redraw = redraw.clone();
        let open = open.clone();
        Callback::from(move |failed: Option<String>| match failed {
            None => {
                if *open.borrow() {
                    on_close.emit(());
                }
            }
            Some(reason) if !*open.borrow() => on_action.emit(Action::Fail(reason)),
            Some(reason) => {
                {
                    let mut editing = cell.borrow_mut();
                    editing.saving = false;
                    editing.error = Some(reason);
                }
                redraw.force_update();
            }
        })
    };
    let now = cell.borrow().clone();
    let problem = now.draft.problem(kind);
    let save = {
        let cell = cell.clone();
        let redraw = redraw.clone();
        let sheet = props.sheet;
        let on_action = props.on_action.clone();
        let done = done.clone();
        Callback::from(move |()| {
            // A reader's sheet has nothing to save — not even by Ctrl+Enter.
            if !editable {
                return;
            }
            let action = {
                let mut editing = cell.borrow_mut();
                if editing.saving {
                    return;
                }
                let note = editing.opened.clone();
                if let Some(reason) = editing.draft.problem(kind) {
                    if !reason.is_empty() {
                        editing.error = Some(reason.to_string());
                    }
                    None
                } else {
                    match (sheet, &note) {
                        (Sheet::New(kind), _) => Some(Action::CreateNote {
                            note: editing.draft.new_note(kind, scattered()),
                            done: done.clone(),
                        }),
                        (Sheet::Open(note_id), Some(note)) => Some(Action::UpdateNote {
                            note_id,
                            patch: editing.draft.patch(note),
                            done: done.clone(),
                        }),
                        (Sheet::Open(_), None) => None,
                    }
                    .inspect(|_| {
                        editing.saving = true;
                        editing.error = None;
                    })
                }
            };
            redraw.force_update();
            if let Some(action) = action {
                on_action.emit(action);
            }
        })
    };
    let save_click = save.reform(|_: MouseEvent| ());
    let close_click = props.on_close.reform(|_: MouseEvent| ());
    let on_key = {
        let on_close = props.on_close.clone();
        let save = save.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() == "Escape" {
                event.stop_propagation();
                on_close.emit(());
            } else if event.key() == "Enter" && (event.meta_key() || event.ctrl_key()) {
                event.prevent_default();
                save.emit(());
            }
        })
    };
    let confirm = |asking: bool| {
        let cell = cell.clone();
        let redraw = redraw.clone();
        Callback::from(move |_: MouseEvent| {
            cell.borrow_mut().confirming = asking;
            redraw.force_update();
        })
    };
    let ask_delete = confirm(true);
    let keep = confirm(false);
    let delete = {
        let on_action = props.on_action.clone();
        let sheet = props.sheet;
        let done = done.clone();
        let cell = cell.clone();
        let redraw = redraw.clone();
        Callback::from(move |_: MouseEvent| {
            let Sheet::Open(note_id) = sheet else {
                return;
            };
            {
                let mut editing = cell.borrow_mut();
                if editing.saving {
                    return;
                }
                editing.saving = true;
            }
            redraw.force_update();
            on_action.emit(Action::DeleteNote {
                note_id,
                done: done.clone(),
            });
        })
    };

    let title = match (props.sheet, kind, editable) {
        (Sheet::New(Kind::Event), _, _) => t("New Event"),
        (Sheet::New(_), _, _) => t("New Note"),
        (_, Kind::Event, _) => t("Event"),
        (_, Kind::Photo, _) => t("Photo"),
        _ => t("Note"),
    };
    let field_label = match kind {
        Kind::Event => t("Title"),
        Kind::Photo => t("Caption"),
        Kind::Text => t("Note"),
    };

    // Answering is its own act: ANY member may, so it is not part of the
    // author's save, and it sits outside every author gate.
    let answering = props
        .note
        .as_ref()
        .filter(|note| note.kind() == Kind::Event && !props.gone)
        .map(|note| {
            // Lit at once, as the phone lights it: the answer being sent,
            // until the server's copy of the note — or its refusal — is in.
            let mine = now
                .answering
                .unwrap_or_else(|| Answer::from_name(note.answer_of(props.my_user_id)));
            let answer = |choice: Option<Answer>| {
                let on_action = props.on_action.clone();
                let cell = cell.clone();
                let redraw = redraw.clone();
                let note_id = note.id;
                Callback::from(move |_: MouseEvent| {
                    let send = {
                        let mut editing = cell.borrow_mut();
                        editing.answering = Some(choice);
                        // One at a time, the latest waiting: answers sent
                        // side by side land in whatever order the server
                        // takes them, and "Going" then "Maybe" could end as
                        // "Going".
                        if editing.answer_sending {
                            editing.answer_queued = Some(choice);
                            false
                        } else {
                            editing.answer_sending = true;
                            true
                        }
                    };
                    redraw.force_update();
                    if send {
                        send_answer(
                            cell.clone(),
                            redraw.clone(),
                            on_action.clone(),
                            note_id,
                            choice,
                        );
                    }
                })
            };
            let guests = (!note.rsvps().is_empty()).then(|| {
                rules::guest_line(
                    note.count(Answer::Going),
                    note.count(Answer::Maybe),
                    note.count(Answer::No),
                )
            });
            html! {
                <fieldset class="rsvp">
                    <legend>{ t("Are you coming?") }</legend>
                    <div class="segmented" role="group" aria-label={t("Are you coming?")}>
                        <button
                            class={classes!(mine.is_none().then_some("is-chosen"))}
                            aria-pressed={if mine.is_none() { "true" } else { "false" }}
                            onclick={answer(None)}
                        >{ t("No answer") }</button>
                        { for Answer::ALL.iter().map(|choice| html! {
                            <button
                                class={classes!((mine == Some(*choice)).then_some("is-chosen"))}
                                aria-pressed={if mine == Some(*choice) { "true" } else { "false" }}
                                onclick={answer(Some(*choice))}
                            >{ choice.title() }</button>
                        }) }
                    </div>
                    if let Some(guests) = guests {
                        <p class="footnote">{ guests }</p>
                    }
                </fieldset>
            }
        });

    let picture = props
        .note
        .as_ref()
        .filter(|note| note.kind() == Kind::Photo)
        .and_then(|note| note.attachment.clone())
        .map(|attachment| {
            let on_action = props.on_action.clone();
            let items = vec![attachment.clone()];
            html! {
                <button
                    class="sheet-picture"
                    aria-label={t("View the photo")}
                    onclick={Callback::from(move |_: MouseEvent| on_action.emit(Action::OpenViewer { items: items.clone(), index: 0 }))}
                >
                    <NotePicture {attachment} />
                </button>
            }
        });

    let body = if props.gone {
        html! { <p class="footnote">{ t("This note has been taken down.") }</p> }
    } else if editable {
        let draft_now = now.draft.clone();
        let when_fields = (kind == Kind::Event).then(|| {
            html! {
                <fieldset>
                    <legend>{ t("When") }</legend>
                    <label class="field">{ t("Starts") }
                        <input type="datetime-local" value={draft_now.starts.clone()} onchange={on_starts} required={true} />
                    </label>
                    <label class="check">
                        <input type="checkbox" checked={draft_now.has_end} onchange={on_has_end} />
                        { t("Has an end") }
                    </label>
                    if draft_now.has_end {
                        <label class="field">{ t("Ends") }
                            <input type="datetime-local" min={draft_now.starts.clone()} value={draft_now.ends.clone()} onchange={on_ends} />
                        </label>
                    }
                    <label class="field">{ t("Place") }
                        <input ref={place_field.clone()} type="text" value={draft_now.place.clone()} oninput={on_place.0} placeholder={t("Where")} />
                    </label>
                </fieldset>
            }
        });
        let preview_event = (kind == Kind::Event).then(|| {
            let starts = time::from_local_input(&draft_now.starts).unwrap_or_default();
            let ends = draft_now
                .has_end
                .then(|| time::from_local_input(&draft_now.ends))
                .flatten();
            let place = draft_now.place.trim().to_string();
            html! {
                <EventBlock
                    starts_at={AttrValue::from(starts)}
                    ends_at={ends.map(AttrValue::from)}
                    place={(!place.is_empty()).then(|| AttrValue::from(place))}
                    going={props.note.as_ref().map(|note| note.count(Answer::Going)).unwrap_or(0)}
                    maybe={props.note.as_ref().map(|note| note.count(Answer::Maybe)).unwrap_or(0)}
                    past={false}
                />
            }
        });
        let card = draft_now.size.frame(props.compact);
        let preview_text = if draft_now.text.trim().is_empty() {
            match kind {
                Kind::Photo => String::new(),
                Kind::Event => t("Your event").to_string(),
                Kind::Text => t("Your note").to_string(),
            }
        } else {
            draft_now.text.clone()
        };
        let preview_picture = props
            .note
            .as_ref()
            .filter(|_| kind == Kind::Photo)
            .and_then(|note| note.attachment.clone());
        html! {
            <>
                { picture.clone().unwrap_or_default() }
                <label class="field">{ field_label }
                    <textarea
                        ref={field.clone()}
                        rows="4"
                        value={draft_now.text.clone()}
                        oninput={on_text.0}
                        placeholder={if kind == Kind::Photo { t("Say something about it (optional)") } else { "" }}
                    />
                </label>
                if rules::shows_counter(&draft_now.text) {
                    <p class={classes!("footnote", (rules::remaining(&draft_now.text) == 0).then_some("danger"))}>
                        { tn("%lld characters left", rules::remaining(&draft_now.text) as i64) }
                    </p>
                }
                { when_fields.unwrap_or_default() }
                <fieldset>
                    <legend>{ t("Colour") }</legend>
                    <div class="swatches" role="radiogroup" aria-label={t("Colour")}>
                        { for rules::COLORS.iter().map(|name| html! {
                            <label class="swatch" style={format!("background:{}", rules::color_hex(name))} title={*name}>
                                <input type="radio" name="note-color" value={*name}
                                    checked={draft_now.color == *name}
                                    onchange={choose_color(name)}
                                    aria-label={*name} />
                            </label>
                        }) }
                    </div>
                </fieldset>
                <fieldset>
                    <legend>{ t("Size") }</legend>
                    <div class="segmented" role="radiogroup" aria-label={t("Size")}>
                        { for Size::ALL.iter().map(|size| html! {
                            <label class={classes!((draft_now.size == *size).then_some("is-chosen"))}>
                                <input type="radio" name="note-size" checked={draft_now.size == *size} onchange={choose_size(*size)} />
                                { size.title() }
                            </label>
                        }) }
                    </div>
                </fieldset>
                <fieldset>
                    <legend>{ t("Font") }</legend>
                    <div class="segmented" role="radiogroup" aria-label={t("Font")}>
                        // Each face's name written IN that face.
                        { for Font::ALL.iter().map(|font| html! {
                            <label class={classes!((draft_now.font == *font).then_some("is-chosen"))} style={format!("font-family:{}", font.css_family())}>
                                <input type="radio" name="note-font" checked={draft_now.font == *font} onchange={choose_font(*font)} />
                                { font.title() }
                            </label>
                        }) }
                    </div>
                </fieldset>
                // The sticker as the wall will draw it, type already
                // fitted: the size is a choice with its result in front of
                // the author, not a name (docs/protocol.md, "Board").
                <div class="sheet-preview" aria-label={t("Preview")} role="img">
                    <div
                        class={classes!("sticker", "is-preview", preview_picture.is_some().then_some("is-photo"), (!preview_text.is_empty()).then_some("has-caption"))}
                        style={format!("width:{}px;height:{}px;background:{};", card.0, card.1, rules::color_hex(&draft_now.color))}
                    >
                        if let Some(attachment) = preview_picture {
                            <NotePicture {attachment} />
                        }
                        { preview_event.unwrap_or_default() }
                        if !preview_text.is_empty() {
                            <FittedText text={AttrValue::from(preview_text)} font={draft_now.font} size={draft_now.size} />
                        }
                        <span class="note-author">{ props.author.clone() }</span>
                    </div>
                </div>
                { answering.clone().unwrap_or_default() }
            </>
        }
    } else {
        let note = props.note.clone().unwrap_or_default();
        let event = (kind == Kind::Event)
            .then(|| note.starts_at.clone())
            .flatten()
            .map(|starts_at| {
                html! {
                    <p class="sheet-when">
                        { time::event_when(&starts_at, note.ends_at.as_deref()) }
                        if let Some(place) = note.place.clone().filter(|place| !place.is_empty()) {
                            <span class="sheet-place">{ place }</span>
                        }
                    </p>
                }
            });
        html! {
            <>
                { picture.unwrap_or_default() }
                { event.unwrap_or_default() }
                if !note.text().trim().is_empty() {
                    <p class="sheet-text" style={format!("font-family:{}", note.font().css_family())}>{ note.text().to_string() }</p>
                }
                <p class="footnote">{ t1("Written by %@", &props.author) }</p>
                { answering.unwrap_or_default() }
            </>
        }
    };

    html! {
        <div class="dialog-backdrop" onkeydown={on_key}>
            <div ref={dialog} class="dialog note-sheet" role="dialog" aria-modal="true" aria-labelledby="note-sheet-title" tabindex="-1">
                <h2 id="note-sheet-title">{ title }</h2>
                { body }
                if let Some(reason) = now.error.clone() {
                    <p class="error" role="alert">{ reason }</p>
                }
                <div class="dialog-actions">
                    if editable && matches!(props.sheet, Sheet::Open(_)) {
                        if now.confirming {
                            <span class="confirm">{ t("Delete this note?") }</span>
                            <button class="secondary" onclick={keep}>{ t("Keep") }</button>
                            <button class="danger-button" onclick={delete} disabled={now.saving}>{ t("Delete") }</button>
                        } else {
                            <button class="secondary danger" onclick={ask_delete} disabled={now.saving}>{ t("Delete Note") }</button>
                        }
                    }
                    <span class="spacer" />
                    if editable {
                        <button class="secondary" onclick={close_click}>{ t("Cancel") }</button>
                        <button class="primary" onclick={save_click} disabled={now.saving || problem == Some("")}>
                            { if now.saving { t("Saving…") } else { t("Save") } }
                        </button>
                    } else {
                        <button class="primary" onclick={close_click}>{ t("Done") }</button>
                    }
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use gloo_timers::future::TimeoutFuture;
    use wasm_bindgen_test::*;
    use web_sys::HtmlElement;

    const ME: i64 = 7;
    const ANNA: i64 = 9;

    fn note(id: i64, author: i64, text: &str, x: f64, y: f64) -> Note {
        Note {
            id,
            board_seq: id * 10,
            author_id: Some(author),
            text: Some(text.into()),
            color: Some("yellow".into()),
            size: Some("medium".into()),
            font: Some("plain".into()),
            kind: Some("text".into()),
            x: Some(x),
            y: Some(y),
            content_seq: Some(id * 10),
            ..Note::default()
        }
    }

    fn event_note(id: i64, author: i64) -> Note {
        Note {
            kind: Some("event".into()),
            starts_at: Some(time::next_round_hour(js_sys::Date::now() + 86_400_000.0)),
            place: Some("The park".into()),
            rsvps: Some(vec![crate::model::Rsvp {
                user_id: ANNA,
                answer: "going".into(),
            }]),
            ..note(id, author, "Picnic", 0.1, 0.1)
        }
    }

    type Log = Rc<RefCell<Vec<Action>>>;

    fn props(notes: Vec<Note>, blocked: &[i64], log: &Log) -> BoardProps {
        let sink = log.clone();
        BoardProps {
            notes,
            loaded: true,
            my_user_id: ME,
            names: HashMap::from([(ANNA, "Anna".to_string())]),
            blocked: blocked.iter().copied().collect(),
            revealed: HashSet::new(),
            pinning: false,
            now_minute: (js_sys::Date::now() / 60_000.0) as i64,
            on_action: Callback::from(move |action: Action| sink.borrow_mut().push(action)),
        }
    }

    fn install_stylesheet() {
        let document = web_sys::window().unwrap().document().unwrap();
        if document.get_element_by_id("fc-board-styles").is_some() {
            return;
        }
        let style = document.create_element("style").unwrap();
        style.set_id("fc-board-styles");
        style.set_text_content(Some(include_str!("../../styles.css")));
        document.head().unwrap().append_child(&style).unwrap();
    }

    /// A pane of a known size, laid out like the main pane of `.split`.
    async fn render(props: BoardProps) -> (HtmlElement, yew::AppHandle<BoardPane>) {
        install_stylesheet();
        let document = web_sys::window().unwrap().document().unwrap();
        let root: HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
        root.set_attribute(
            "style",
            "position:fixed;top:0;left:0;width:900px;height:600px;\
             display:grid;grid-template-rows:minmax(0,1fr);",
        )
        .unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let handle =
            yew::Renderer::<BoardPane>::with_root_and_props(root.clone().into(), props).render();
        TimeoutFuture::new(60).await;
        (root, handle)
    }

    fn all(root: &Element, selector: &str) -> Vec<HtmlElement> {
        let list = root.query_selector_all(selector).unwrap();
        (0..list.length())
            .filter_map(|index| list.item(index))
            .filter_map(|node| node.dyn_into::<HtmlElement>().ok())
            .collect()
    }

    fn one(root: &Element, selector: &str) -> HtmlElement {
        all(root, selector)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{selector} is on the page"))
    }

    fn px(element: &HtmlElement, property: &str) -> f64 {
        element
            .style()
            .get_property_value(property)
            .unwrap()
            .trim_end_matches("px")
            .parse()
            .unwrap()
    }

    fn pointer(target: &Element, kind: &str, x: f64, y: f64) {
        let init = web_sys::PointerEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_pointer_id(1);
        init.set_client_x(x as i32);
        init.set_client_y(y as i32);
        init.set_button(0);
        let event = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
        target.dispatch_event(&event).unwrap();
    }

    fn key(target: &Element, kind: &str, name: &str) {
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(name);
        let event = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict(kind, &init).unwrap();
        target.dispatch_event(&event).unwrap();
    }

    fn click(target: &Element) {
        let init = web_sys::MouseEventInit::new();
        init.set_bubbles(true);
        let event = web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).unwrap();
        target.dispatch_event(&event).unwrap();
    }

    fn change(target: &Element) {
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        let event = web_sys::Event::new_with_event_init_dict("change", &init).unwrap();
        target.dispatch_event(&event).unwrap();
    }

    fn moves(log: &Log) -> Vec<(i64, f64, f64)> {
        log.borrow()
            .iter()
            .filter_map(|action| match action {
                Action::MoveNote { note_id, x, y, .. } => Some((*note_id, *x, *y)),
                _ => None,
            })
            .collect()
    }

    /// Fractions, to the third place.
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.002
    }

    /// Pixels, as the style writes them — to a tenth.
    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() <= 0.051
    }

    /// The fraction is the CORNER, drawn inside the wall: a stored 0.99
    /// hugs the far edge rather than hanging off it.
    #[wasm_bindgen_test]
    async fn a_note_sits_by_its_corner_and_inside_the_wall() {
        let log = Log::default();
        let (root, handle) = render(props(
            vec![
                note(1, ME, "Milk", 0.5, 0.25),
                note(2, ANNA, "Eggs", 0.99, 0.99),
            ],
            &[],
            &log,
        ))
        .await;
        let wall = one(&root, ".board-wall");
        let (width, height) = (
            f64::from(wall.client_width()),
            f64::from(wall.client_height()),
        );
        assert!(width > 640.0, "the wide wall: {width}");
        let stickers = all(&root, ".sticker");
        assert_eq!(stickers.len(), 2);
        assert!(near(px(&stickers[0], "left"), 0.5 * width));
        assert!(near(px(&stickers[0], "top"), 0.25 * height));
        assert_eq!(px(&stickers[0], "width"), 150.0, "the Mac's medium card");
        assert!(
            near(px(&stickers[1], "left"), width - 150.0),
            "held inside the wall"
        );
        assert!(near(px(&stickers[1], "top"), height - 110.0));
        // Signed by who wrote it.
        let authors: Vec<String> = all(&root, ".note-author")
            .iter()
            .map(|author| author.text_content().unwrap_or_default())
            .collect();
        assert_eq!(authors, vec!["You".to_string(), "Anna".to_string()]);
        handle.destroy();
        root.remove();
    }

    /// A blocked author's note keeps its slot and draws the placeholder with
    /// no author line — and the first click REVEALS it and opens nothing.
    #[wasm_bindgen_test]
    async fn a_hidden_note_reveals_on_the_first_click_and_opens_nothing() {
        let log = Log::default();
        let (root, handle) = render(props(
            vec![note(3, ANNA, "a secret", 0.2, 0.2)],
            &[ANNA],
            &log,
        ))
        .await;
        let sticker = one(&root, ".sticker");
        let text = sticker.text_content().unwrap_or_default();
        assert!(text.contains("Hidden — blocked member"), "{text}");
        assert!(!text.contains("a secret"), "the words are not on the page");
        assert!(
            all(&root, ".note-author").is_empty(),
            "no author line at all"
        );
        assert_eq!(
            sticker.get_attribute("aria-label").as_deref(),
            Some("Hidden note from a blocked member")
        );
        pointer(&sticker, "pointerdown", 200.0, 200.0);
        pointer(&sticker, "pointerup", 200.0, 200.0);
        TimeoutFuture::new(30).await;
        assert_eq!(*log.borrow(), vec![Action::RevealNote { note_id: 3 }]);
        assert!(
            all(&root, ".note-sheet").is_empty(),
            "the editor would show the hidden text"
        );
        handle.destroy();
        root.remove();
    }

    /// A press that goes nowhere is a click, and opens the note; a press
    /// that travels is a drag, and reports ONE move — the fraction of where
    /// the sticker was dropped — and holds it there until the answer.
    #[wasm_bindgen_test]
    async fn a_click_opens_the_note_and_a_drag_reports_where_it_was_dropped() {
        let log = Log::default();
        let (root, handle) =
            render(props(vec![note(4, ANNA, "Tidy me", 0.1, 0.1)], &[], &log)).await;
        let wall = one(&root, ".board-wall");
        let (width, height) = (
            f64::from(wall.client_width()),
            f64::from(wall.client_height()),
        );
        let sticker = one(&root, ".sticker");

        pointer(&sticker, "pointerdown", 100.0, 100.0);
        pointer(&sticker, "pointermove", 102.0, 101.0);
        pointer(&sticker, "pointerup", 102.0, 101.0);
        TimeoutFuture::new(30).await;
        assert!(
            moves(&log).is_empty(),
            "a few pixels are a click, not a drag"
        );
        assert_eq!(all(&root, ".note-sheet").len(), 1, "the click opened it");
        let done = one(&root, ".note-sheet .dialog-actions .primary");
        click(&done);
        TimeoutFuture::new(30).await;
        assert!(all(&root, ".note-sheet").is_empty());

        let before = px(&sticker, "left");
        pointer(&sticker, "pointerdown", 100.0, 100.0);
        pointer(&sticker, "pointermove", 160.0, 130.0);
        pointer(&sticker, "pointermove", 190.0, 150.0);
        TimeoutFuture::new(30).await;
        assert!(
            sticker.class_list().contains("is-dragging"),
            "lifted while in hand"
        );
        assert!(
            near(px(&sticker, "left"), before + 90.0),
            "it follows the pointer"
        );
        pointer(&sticker, "pointerup", 190.0, 150.0);
        TimeoutFuture::new(30).await;
        let moved = moves(&log);
        assert_eq!(moved.len(), 1, "one drag, one write");
        let (id, x, y) = moved[0];
        assert_eq!(id, 4);
        assert!(close(x, (0.1 * width + 90.0) / width), "{x}");
        assert!(close(y, (0.1 * height + 50.0) / height), "{y}");
        assert!(all(&root, ".note-sheet").is_empty(), "a drag opens nothing");
        assert!(
            near(px(&sticker, "left"), before + 90.0),
            "held where it was dropped"
        );

        // A move that came to nothing lets go, and the note is where the
        // server has it.
        let done = log.borrow().iter().find_map(|action| match action {
            Action::MoveNote { done, .. } => Some(done.clone()),
            _ => None,
        });
        done.expect("a done").emit(());
        TimeoutFuture::new(30).await;
        assert!(
            near(px(&sticker, "left"), before),
            "back where the server has it"
        );
        handle.destroy();
        root.remove();
    }

    /// One move of a note at a time: a second drop while the first is on its
    /// way waits, only the LATEST waiting one is sent, and the note is drawn
    /// where it was last dropped all the while.
    #[wasm_bindgen_test]
    async fn moves_of_one_note_go_one_at_a_time_and_the_latest_wins() {
        let log = Log::default();
        let (root, handle) = render(props(vec![note(12, ME, "Busy", 0.1, 0.1)], &[], &log)).await;
        let sticker = one(&root, ".sticker");
        for (dx, key_name) in [(1, "ArrowRight"), (2, "ArrowRight"), (3, "ArrowDown")] {
            let _ = dx;
            key(&sticker, "keydown", key_name);
            key(&sticker, "keyup", key_name);
        }
        TimeoutFuture::new(30).await;
        let sent = moves(&log);
        assert_eq!(sent.len(), 1, "the second and third wait: {sent:?}");
        assert!(close(sent[0].1, 0.11));
        let left_now = px(&sticker, "left");
        let wall = one(&root, ".board-wall");
        let width = f64::from(wall.client_width());
        assert!(
            near(left_now, 0.12 * width),
            "drawn where it was LAST put: {left_now}"
        );
        // The first is answered: only the latest waiting drop goes.
        let first_done = log.borrow().iter().find_map(|action| match action {
            Action::MoveNote { done, .. } => Some(done.clone()),
            _ => None,
        });
        first_done.expect("a done").emit(());
        TimeoutFuture::new(30).await;
        let sent = moves(&log);
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert!(close(sent[1].1, 0.12) && close(sent[1].2, 0.11), "{sent:?}");
        handle.destroy();
        root.remove();
    }

    /// A refusal that arrives after its sheet was closed is still said —
    /// on the bar, since the sheet is gone.
    #[wasm_bindgen_test]
    async fn a_refusal_for_a_closed_sheet_is_still_said() {
        let log = Log::default();
        let (root, handle) = render(props(Vec::new(), &[], &log)).await;
        click(&all(&root, ".board-actions button")[0]);
        TimeoutFuture::new(30).await;
        let area = one(&root, ".note-sheet textarea");
        area.dyn_ref::<HtmlTextAreaElement>()
            .unwrap()
            .set_value("One too many");
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        area.dispatch_event(
            &web_sys::InputEvent::new_with_event_init_dict("input", &init.clone().unchecked_into())
                .unwrap(),
        )
        .unwrap();
        TimeoutFuture::new(20).await;
        click(&one(&root, ".note-sheet .dialog-actions .primary"));
        TimeoutFuture::new(20).await;
        key(&one(&root, ".note-sheet"), "keydown", "Escape");
        TimeoutFuture::new(30).await;
        assert!(all(&root, ".note-sheet").is_empty());
        let done = log.borrow().iter().find_map(|action| match action {
            Action::CreateNote { done, .. } => Some(done.clone()),
            _ => None,
        });
        done.expect("a done")
            .emit(Some("The board is full.".into()));
        TimeoutFuture::new(20).await;
        assert!(
            log.borrow()
                .iter()
                .any(|action| *action == Action::Fail("The board is full.".into())),
            "{:?}",
            log.borrow()
        );
        handle.destroy();
        root.remove();
    }

    /// THE REVIEW'S HIGH: a wall of many notes must never paint over the
    /// sheet opened on top of it, or take its clicks.
    #[wasm_bindgen_test]
    async fn a_crowded_wall_stays_under_the_sheet() {
        let log = Log::default();
        let notes: Vec<Note> = (1..=40)
            .map(|id| note(id, ME, "crowd", 0.3 + (id % 5) as f64 * 0.02, 0.2))
            .collect();
        let (root, handle) = render(props(notes, &[], &log)).await;
        click(&all(&root, ".board-actions button")[0]);
        TimeoutFuture::new(40).await;
        let dialog = one(&root, ".note-sheet");
        let rect = dialog.get_bounding_client_rect();
        let document = web_sys::window().unwrap().document().unwrap();
        // The wall is inert under an open sheet, and hit-testing steps over
        // inert elements — which would hide a sticker PAINTED on top of the
        // sheet from this probe. Lifted here, so what is probed is the
        // painting order itself.
        one(&root, ".board-wall").remove_attribute("inert").unwrap();
        for (fx, fy) in [(0.1, 0.1), (0.5, 0.5), (0.9, 0.3), (0.2, 0.8)] {
            let at = document
                .element_from_point(
                    (rect.left() + rect.width() * fx) as f32,
                    (rect.top() + rect.height() * fy) as f32,
                )
                .expect("something is there");
            assert!(
                at.closest(".note-sheet").unwrap().is_some(),
                "the sheet is on top at ({fx}, {fy}), not {:?}",
                at.class_name()
            );
        }
        handle.destroy();
        root.remove();
    }

    /// A reader's sheet has nothing to save, whatever key they press.
    #[wasm_bindgen_test]
    async fn a_readers_sheet_saves_nothing() {
        let log = Log::default();
        let (root, handle) =
            render(props(vec![note(17, ANNA, "Theirs", 0.2, 0.2)], &[], &log)).await;
        key(&one(&root, ".sticker"), "keydown", "Enter");
        TimeoutFuture::new(40).await;
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_key("Enter");
        init.set_ctrl_key(true);
        let event =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        one(&root, ".note-sheet").dispatch_event(&event).unwrap();
        TimeoutFuture::new(20).await;
        assert!(
            !log.borrow().iter().any(|action| matches!(
                action,
                Action::UpdateNote { .. } | Action::CreateNote { .. }
            )),
            "{:?}",
            log.borrow()
        );
        handle.destroy();
        root.remove();
    }

    /// An event's when and where give way before its title does: even a
    /// small card keeps a line of what the event IS.
    #[wasm_bindgen_test]
    async fn an_events_title_keeps_its_line_on_a_small_card() {
        let log = Log::default();
        let mut picnic = event_note(13, ANNA);
        picnic.size = Some("small".into());
        picnic.ends_at = picnic
            .starts_at
            .as_deref()
            .and_then(|starts| time::shifted(starts, 26.0 * 3_600_000.0));
        let (root, handle) = render(props(vec![picnic], &[], &log)).await;
        TimeoutFuture::new(60).await;
        let title = one(&root, ".note-text");
        assert!(
            title.client_height() >= 7,
            "the title has a line: {}",
            title.client_height()
        );
        handle.destroy();
        root.remove();
    }

    /// Typing into a full note takes back what was typed, not the end of the
    /// note, and leaves the caret where the typing was.
    #[wasm_bindgen_test]
    async fn typing_into_a_full_note_takes_back_the_new_character() {
        let log = Log::default();
        let full = format!("{}END", "a".repeat(277));
        let (root, handle) = render(props(vec![note(14, ME, &full, 0.1, 0.1)], &[], &log)).await;
        key(&one(&root, ".sticker"), "keydown", "Enter");
        TimeoutFuture::new(40).await;
        let area_element = one(&root, ".note-sheet textarea");
        let area = area_element.dyn_ref::<HtmlTextAreaElement>().unwrap();
        area.set_value(&format!("{}x{}", &full[..10], &full[10..]));
        area.set_selection_range(11, 11).unwrap();
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        area.dispatch_event(
            &web_sys::InputEvent::new_with_event_init_dict("input", &init.clone().unchecked_into())
                .unwrap(),
        )
        .unwrap();
        TimeoutFuture::new(20).await;
        assert_eq!(area.value(), full, "the END is still there");
        assert_eq!(area.selection_start().unwrap(), Some(10));
        handle.destroy();
        root.remove();
    }

    /// A save is a change FROM the note as it was opened: a colour set on
    /// another device while the sheet was open is not put back.
    #[wasm_bindgen_test]
    async fn a_save_does_not_undo_a_change_made_elsewhere_meanwhile() {
        let log = Log::default();
        let (root, mut handle) =
            render(props(vec![note(15, ME, "Milk", 0.1, 0.1)], &[], &log)).await;
        key(&one(&root, ".sticker"), "keydown", "Enter");
        TimeoutFuture::new(40).await;
        // The phone recolours it while the sheet is open here.
        let mut recoloured = note(15, ME, "Milk", 0.1, 0.1);
        recoloured.color = Some("green".into());
        recoloured.board_seq += 1;
        handle.update(props(vec![recoloured], &[], &log));
        TimeoutFuture::new(30).await;
        let area_element = one(&root, ".note-sheet textarea");
        let area = area_element.dyn_ref::<HtmlTextAreaElement>().unwrap();
        area.set_value("Milk and eggs");
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        area.dispatch_event(
            &web_sys::InputEvent::new_with_event_init_dict("input", &init.clone().unchecked_into())
                .unwrap(),
        )
        .unwrap();
        TimeoutFuture::new(20).await;
        click(&one(&root, ".note-sheet .dialog-actions .primary"));
        TimeoutFuture::new(20).await;
        let patches: Vec<NotePatch> = log
            .borrow()
            .iter()
            .filter_map(|action| match action {
                Action::UpdateNote { patch, .. } => Some(patch.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            patches,
            vec![NotePatch {
                text: Some("Milk and eggs".into()),
                ..NotePatch::default()
            }],
            "only the words — not the yellow the sheet opened with"
        );
        handle.destroy();
        root.remove();
    }

    /// A keyboard move whose focus goes elsewhere is put down there, not
    /// left lifted with no key-up ever coming.
    #[wasm_bindgen_test]
    async fn a_keyboard_move_is_put_down_when_the_focus_goes() {
        let log = Log::default();
        let (root, handle) =
            render(props(vec![note(16, ME, "Tab away", 0.2, 0.2)], &[], &log)).await;
        let sticker = one(&root, ".sticker");
        sticker.focus().unwrap();
        key(&sticker, "keydown", "ArrowRight");
        TimeoutFuture::new(20).await;
        assert!(sticker.class_list().contains("is-dragging"));
        sticker.blur().unwrap();
        TimeoutFuture::new(20).await;
        let sent = moves(&log);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(close(sent[0].1, 0.21), "{sent:?}");
        assert!(!sticker.class_list().contains("is-dragging"), "put down");
        handle.destroy();
        root.remove();
    }

    /// The keyboard does all of it: Enter opens, an arrow moves a hundredth
    /// of the wall a press, and letting go of it puts the note down.
    #[wasm_bindgen_test]
    async fn the_arrow_keys_move_a_note_and_letting_go_puts_it_down() {
        let log = Log::default();
        let (root, handle) = render(props(vec![note(5, ME, "Nudge", 0.2, 0.3)], &[], &log)).await;
        let sticker = one(&root, ".sticker");
        for _ in 0..3 {
            key(&sticker, "keydown", "ArrowRight");
        }
        key(&sticker, "keydown", "ArrowUp");
        TimeoutFuture::new(20).await;
        assert!(
            moves(&log).is_empty(),
            "nothing is written while the key is down"
        );
        key(&sticker, "keyup", "ArrowUp");
        TimeoutFuture::new(20).await;
        let moved = moves(&log);
        assert_eq!(moved.len(), 1);
        assert!(close(moved[0].1, 0.23), "{moved:?}");
        assert!(close(moved[0].2, 0.29), "{moved:?}");
        key(&sticker, "keydown", "Enter");
        TimeoutFuture::new(30).await;
        assert_eq!(all(&root, ".note-sheet").len(), 1);
        handle.destroy();
        root.remove();
    }

    /// A long note is FITTED: smaller type, all of it inside; a short one
    /// stays at its size's own; and one that cannot fit even at the floor is
    /// cut there with an ellipsis rather than shrunk into dust.
    #[wasm_bindgen_test]
    async fn the_text_fits_its_card_down_to_the_floor_and_is_cut_below_it() {
        let log = Log::default();
        let long = "Pick up milk, eggs, bread and the good cheese on the way home. ".repeat(3);
        let mut small = note(8, ME, &"W".repeat(280), 0.5, 0.6);
        small.size = Some("small".into());
        let (root, handle) = render(props(
            vec![
                note(6, ME, "Milk", 0.1, 0.1),
                note(7, ME, long.trim(), 0.4, 0.1),
                small,
            ],
            &[],
            &log,
        ))
        .await;
        TimeoutFuture::new(60).await;
        let texts = all(&root, ".note-text");
        let size = |element: &HtmlElement| px(element, "font-size");
        assert_eq!(
            size(&texts[0]),
            14.0,
            "a short note keeps its size's own type"
        );
        assert!(
            size(&texts[1]) < 14.0,
            "a long one is scaled down: {}",
            size(&texts[1])
        );
        assert!(size(&texts[1]) >= 14.0 * rules::MIN_TEXT_SCALE - 0.01);
        assert!(
            texts[1].scroll_height() <= texts[1].client_height() + 1,
            "and all of it is inside"
        );
        assert!(!texts[1].class_list().contains("is-cut"));
        assert!(
            texts[2].class_list().contains("is-cut"),
            "past the floor: cut"
        );
        assert!(near(size(&texts[2]), 12.0 * rules::MIN_TEXT_SCALE));
        handle.destroy();
        root.remove();
    }

    /// The author's own note opens to edit, and a save sends ONLY what
    /// changed — here the colour.
    #[wasm_bindgen_test]
    async fn the_authors_save_sends_only_what_changed() {
        let log = Log::default();
        let (root, handle) = render(props(vec![note(9, ME, "Milk", 0.1, 0.1)], &[], &log)).await;
        let sticker = one(&root, ".sticker");
        key(&sticker, "keydown", "Enter");
        TimeoutFuture::new(30).await;
        let pink = one(&root, ".note-sheet input[value='pink']");
        pink.dyn_ref::<HtmlInputElement>()
            .unwrap()
            .set_checked(true);
        change(&pink);
        TimeoutFuture::new(20).await;
        click(&one(&root, ".note-sheet .dialog-actions .primary"));
        TimeoutFuture::new(20).await;
        let updates: Vec<(i64, NotePatch)> = log
            .borrow()
            .iter()
            .filter_map(|action| match action {
                Action::UpdateNote { note_id, patch, .. } => Some((*note_id, patch.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            updates,
            vec![(
                9,
                NotePatch {
                    color: Some("pink".into()),
                    ..NotePatch::default()
                }
            )]
        );
        handle.destroy();
        root.remove();
    }

    /// Somebody else's event opens to READ, with no editor — and anybody
    /// may say whether they are coming, or take it back.
    #[wasm_bindgen_test]
    async fn anybody_answers_an_event_and_only_its_author_edits_it() {
        let log = Log::default();
        let (root, handle) = render(props(vec![event_note(10, ANNA)], &[], &log)).await;
        let sticker = one(&root, ".sticker");
        let text = sticker.text_content().unwrap_or_default();
        assert!(
            text.contains("The park") && text.contains("1 going"),
            "{text}"
        );
        key(&sticker, "keydown", "Enter");
        TimeoutFuture::new(30).await;
        let sheet = one(&root, ".note-sheet");
        assert!(
            all(&sheet, "textarea").is_empty(),
            "not the author's: nothing to edit"
        );
        assert!(sheet
            .text_content()
            .unwrap_or_default()
            .contains("Written by Anna"));
        let answers = all(&sheet, ".rsvp button");
        assert_eq!(answers.len(), 4, "no answer, going, maybe, can't");
        assert_eq!(
            answers[0].get_attribute("aria-pressed").as_deref(),
            Some("true"),
            "no answer yet: the author's is not assumed"
        );
        click(&answers[1]);
        TimeoutFuture::new(20).await;
        let answers = all(&sheet, ".rsvp button");
        assert_eq!(
            answers[1].get_attribute("aria-pressed").as_deref(),
            Some("true"),
            "lit at once, before the server has answered"
        );
        // The server refused it (or answered): back to the truth.
        let done = log.borrow().iter().find_map(|action| match action {
            Action::AnswerEvent { done, .. } => Some(done.clone()),
            _ => None,
        });
        done.expect("a done").emit(());
        TimeoutFuture::new(20).await;
        let answers = all(&sheet, ".rsvp button");
        assert_eq!(
            answers[0].get_attribute("aria-pressed").as_deref(),
            Some("true"),
            "the note still says no answer, so the button does"
        );
        click(&answers[0]);
        TimeoutFuture::new(20).await;
        let sent: Vec<(i64, Option<String>)> = log
            .borrow()
            .iter()
            .filter_map(|action| match action {
                Action::AnswerEvent {
                    note_id, answer, ..
                } => Some((*note_id, answer.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(sent, vec![(10, Some("going".into())), (10, None)]);
        handle.destroy();
        root.remove();
    }

    /// Opened from the keyboard, the sheet takes the focus — so Escape
    /// closes it, and the arrow keys no longer reach the note behind it —
    /// and closing gives the focus back to the note.
    #[wasm_bindgen_test]
    async fn the_sheet_takes_the_focus_and_gives_it_back() {
        let log = Log::default();
        let (root, handle) =
            render(props(vec![note(11, ANNA, "Theirs", 0.2, 0.2)], &[], &log)).await;
        let sticker = one(&root, ".sticker");
        sticker.focus().unwrap();
        key(&sticker, "keydown", "Enter");
        TimeoutFuture::new(40).await;
        let document = web_sys::window().unwrap().document().unwrap();
        let focused = document.active_element().expect("something has the focus");
        assert!(
            focused.closest(".note-sheet").unwrap().is_some(),
            "the focus is in the sheet, not on the note behind it"
        );
        key(&focused, "keydown", "ArrowRight");
        key(&focused, "keyup", "ArrowRight");
        key(&focused, "keydown", "Escape");
        TimeoutFuture::new(40).await;
        assert!(all(&root, ".note-sheet").is_empty(), "Escape closed it");
        assert!(
            moves(&log).is_empty(),
            "the note behind the sheet did not move"
        );
        assert_eq!(
            document
                .active_element()
                .and_then(|element| element.get_attribute("data-note")),
            Some("11".to_string()),
            "the focus is back on the note"
        );
        handle.destroy();
        root.remove();
    }

    /// Drawn in id order whatever happens to the wall, and stacked by how
    /// recently each note was touched.
    #[wasm_bindgen_test]
    async fn the_last_touched_note_stacks_on_top_without_moving_in_the_page() {
        let log = Log::default();
        let older = note(1, ME, "first", 0.1, 0.1);
        let mut newer = note(2, ME, "second", 0.2, 0.2);
        newer.board_seq = 5; // touched before note 1 (seq 10)
        let (root, handle) = render(props(vec![newer, older], &[], &log)).await;
        let stickers = all(&root, ".sticker");
        let ids: Vec<Option<String>> = stickers
            .iter()
            .map(|s| s.get_attribute("data-note"))
            .collect();
        assert_eq!(
            ids,
            vec![Some("1".into()), Some("2".into())],
            "id order in the page"
        );
        let layer = |element: &HtmlElement| element.style().get_property_value("z-index").unwrap();
        assert!(
            layer(&stickers[0]).parse::<i32>().unwrap()
                > layer(&stickers[1]).parse::<i32>().unwrap(),
            "the last touched is on top"
        );
        handle.destroy();
        root.remove();
    }

    /// A new note says nothing until something is written, and then goes
    /// as a text note with every part only other kinds have left out.
    #[wasm_bindgen_test]
    async fn a_new_note_is_written_then_pinned() {
        let log = Log::default();
        let (root, handle) = render(props(Vec::new(), &[], &log)).await;
        assert!(one(&root, ".board-empty")
            .text_content()
            .unwrap()
            .contains("The board is empty"));
        click(&all(&root, ".board-actions button")[0]);
        TimeoutFuture::new(30).await;
        let save = one(&root, ".note-sheet .dialog-actions .primary");
        assert!(
            save.has_attribute("disabled"),
            "nothing written, nothing to save"
        );
        let area = one(&root, ".note-sheet textarea");
        area.dyn_ref::<HtmlTextAreaElement>()
            .unwrap()
            .set_value(&"x".repeat(300));
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        area.dispatch_event(
            &web_sys::InputEvent::new_with_event_init_dict("input", &init.clone().unchecked_into())
                .unwrap(),
        )
        .unwrap();
        TimeoutFuture::new(20).await;
        assert_eq!(
            area.dyn_ref::<HtmlTextAreaElement>()
                .unwrap()
                .value()
                .chars()
                .count(),
            280,
            "capped where it is typed"
        );
        click(&one(&root, ".note-sheet .dialog-actions .primary"));
        TimeoutFuture::new(20).await;
        let created: Vec<NewNote> = log
            .borrow()
            .iter()
            .filter_map(|action| match action {
                Action::CreateNote { note, .. } => Some(note.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(created.len(), 1);
        let new = &created[0];
        assert_eq!(new.text.chars().count(), 280);
        assert_eq!((new.size.as_str(), new.font.as_str()), ("medium", "plain"));
        assert_eq!(new.kind, None, "a text note names no kind");
        assert!(new.starts_at.is_none() && new.attachment_id.is_none());
        assert!((0.25..=0.65).contains(&new.x) && (0.25..=0.65).contains(&new.y));
        handle.destroy();
        root.remove();
    }

    /// An event whose end is before its start is caught in the sheet, and
    /// nothing is sent.
    #[wasm_bindgen_test]
    async fn an_event_ending_before_it_starts_is_not_sent() {
        let log = Log::default();
        let (root, handle) = render(props(Vec::new(), &[], &log)).await;
        click(&all(&root, ".board-actions button")[1]);
        TimeoutFuture::new(30).await;
        let area = one(&root, ".note-sheet textarea");
        area.dyn_ref::<HtmlTextAreaElement>()
            .unwrap()
            .set_value("Picnic");
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        area.dispatch_event(
            &web_sys::InputEvent::new_with_event_init_dict("input", &init.clone().unchecked_into())
                .unwrap(),
        )
        .unwrap();
        let has_end = one(&root, ".note-sheet input[type='checkbox']");
        has_end
            .dyn_ref::<HtmlInputElement>()
            .unwrap()
            .set_checked(true);
        change(&has_end);
        TimeoutFuture::new(20).await;
        let times = all(&root, ".note-sheet input[type='datetime-local']");
        assert_eq!(times.len(), 2);
        times[0]
            .dyn_ref::<HtmlInputElement>()
            .unwrap()
            .set_value("2026-09-12T11:00");
        change(&times[0]);
        TimeoutFuture::new(10).await;
        times[1]
            .dyn_ref::<HtmlInputElement>()
            .unwrap()
            .set_value("2026-09-12T10:00");
        change(&times[1]);
        TimeoutFuture::new(20).await;
        click(&one(&root, ".note-sheet .dialog-actions .primary"));
        TimeoutFuture::new(20).await;
        assert!(log.borrow().is_empty(), "{:?}", log.borrow());
        assert!(one(&root, ".note-sheet .error")
            .text_content()
            .unwrap()
            .contains("The end can't be before the start."));
        handle.destroy();
        root.remove();
    }

    /// What a save sends, against the note as it stands — pure.
    #[wasm_bindgen_test]
    fn a_draft_patches_only_what_it_changed() {
        let mut stored = note(1, ME, "Milk", 0.1, 0.1);
        // A size and a face from a newer server draw as the defaults and are
        // NOT written back as them.
        stored.size = Some("huge".into());
        stored.font = Some("gothic".into());
        let draft = Draft::of(&stored);
        assert_eq!(draft.size, Size::Medium);
        assert!(
            draft.patch(&stored).is_empty(),
            "nothing changed, nothing sent"
        );
        let mut edited = draft.clone();
        edited.text = "Milk and eggs".into();
        assert_eq!(
            edited.patch(&stored),
            NotePatch {
                text: Some("Milk and eggs".into()),
                ..NotePatch::default()
            }
        );
        // Trailing space is not a change: the server trims.
        let mut padded = draft.clone();
        padded.text = "Milk  ".into();
        assert!(padded.patch(&stored).is_empty());
        let mut bigger = draft;
        bigger.size = Size::Large;
        assert_eq!(bigger.patch(&stored).size.as_deref(), Some("large"));

        // An event: its end taken off is a null, its place cleared an empty
        // string, and a start that did not move is not sent.
        let mut event = event_note(2, ME);
        event.ends_at = Some(time::next_round_hour(
            js_sys::Date::now() + 2.0 * 86_400_000.0,
        ));
        let draft = Draft::of(&event);
        assert!(draft.has_end);
        assert!(draft.patch(&event).is_empty());
        let mut cleared = draft.clone();
        cleared.has_end = false;
        cleared.place = "  ".into();
        let patch = cleared.patch(&event);
        assert_eq!(patch.ends_at, Some(None));
        assert_eq!(patch.place.as_deref(), Some(""));
        assert_eq!(patch.starts_at, None);
        // A text note never sends an event's fields.
        let mut plain = Draft::of(&stored);
        plain.place = "Somewhere".into();
        assert!(plain.patch(&stored).place.is_none());
    }

    /// A photo's caption may be empty; a note's text and an event's title
    /// may not; and an event needs its start.
    #[wasm_bindgen_test]
    fn what_may_be_saved_depends_on_the_kind() {
        let mut draft = Draft::blank(Kind::Text, js_sys::Date::now());
        assert_eq!(draft.problem(Kind::Text), Some(""));
        assert_eq!(draft.problem(Kind::Photo), None);
        draft.text = "Picnic".into();
        assert_eq!(draft.problem(Kind::Text), None);
        assert_eq!(
            draft.problem(Kind::Event),
            None,
            "a blank event starts on the next hour"
        );
        draft.starts = String::new();
        assert_eq!(draft.problem(Kind::Event), Some("Pick when it starts."));
        let event = Draft::blank(Kind::Event, js_sys::Date::now());
        assert_eq!(event.color, "blue");
        assert!(!event.has_end);
        let new = Draft {
            text: "Picnic".into(),
            place: " The park ".into(),
            ..event
        }
        .new_note(Kind::Event, (0.3, 0.4));
        assert_eq!(new.kind.as_deref(), Some("event"));
        assert!(new.starts_at.is_some());
        assert_eq!(new.ends_at, None, "no end, none sent");
        assert_eq!(new.place.as_deref(), Some("The park"));
    }
}
