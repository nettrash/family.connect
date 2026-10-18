//! Formulas, diagrams and graphs, drawn in the bubble (issue #97;
//! docs/protocol.md, "Rich blocks: formulas, diagrams and graphs";
//! docs/capitals-and-rich-markdown-2026-10-17.md, "In the bubble").
//!
//! The web draws IN THE PAGE with the same renderer every native client
//! snapshots: `rich/fc-rich.js` and the engines beside it, which Trunk copies
//! into `dist/rich/` as they are. Nothing of it loads at startup — it is
//! 5.4 MB, and most families never type a formula. The first bubble that has
//! something to draw injects ONE `<script>` for fc-rich.js, which then loads
//! only the engine that bubble needs (KaTeX for a formula, Mermaid for a
//! diagram, …) the first time it is needed.
//!
//! WHY `?v=`: nginx serves every `.js` and `.css` as immutable for a year,
//! and these files keep their names. The query is a hash of every file in
//! `rich/` made by build.rs, so any change to any of them is a new URL; and
//! fc-rich.js carries its own query to every engine it loads.
//!
//! Until a block is drawn, and for good when it cannot be, the bubble shows
//! what was typed in the monospaced face, exactly as a code block shows today
//! — the drawing never leaves a blank. A drawing is made OFF to one side, in
//! an invisible stage the same width as the bubble, and swapped in only once
//! it is whole: so a re-draw for a theme change never flashes empty, and the
//! bubble changes height exactly once.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_text::i18n::t;
use fc_text::markdown::{Rich, RichKind};
use gloo_timers::callback::Timeout;
use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{Element, HtmlElement};
use yew::prelude::*;

use crate::views::conversation::PINNED_SLACK_PX;

/// The hash of `rich/`, made by build.rs: the `?v=` every rich file is
/// fetched under.
pub const VERSION: &str = env!("FC_RICH_VERSION");

/// A diagram's or a graph's card and its ink (the design doc's job table) —
/// the same on every client, so a drawing reads the same everywhere.
const CARD_LIGHT: &str = "#FFFFFF";
const CARD_DARK: &str = "#1C1C1E";
const CARD_INK_LIGHT: &str = "#1C1C1E";
const CARD_INK_DARK: &str = "#F2F2F7";

/// How big fc-rich.js may draw a diagram or a graph, in CSS pixels — the
/// native clients' snapshot cap. The bubble scales it down to its own width
/// in CSS; the full-size view shows this.
const MAX_SIDE: f64 = 4096.0;

/// The body text size, should the page not say (styles.css `body`).
const BODY_PX: f64 = 15.0;

/// How long a click on a diagram waits before it opens full size, so the
/// second click of a double-click can be the heart instead — the same beat
/// a link waits (views/body.rs).
const OPEN_DELAY_MS: u32 = 350;

/// Where fc-rich.js is, relative to the page's `<base href="/">`.
pub fn script_url() -> String {
    format!("rich/fc-rich.js?v={VERSION}")
}

// MARK: - Loading the renderer, once

thread_local! {
    /// The one load of fc-rich.js, shared by every bubble that waits for it.
    /// Emptied again when it fails, so a later bubble tries again (a page
    /// opened offline is not without formulas for the rest of its life).
    static LOADING: RefCell<Option<Promise>> = const { RefCell::new(None) };
}

/// `window.fcRich`, once fc-rich.js has run.
fn renderer() -> Option<JsValue> {
    let window = web_sys::window()?;
    let renderer = Reflect::get(&window, &JsValue::from_str("fcRich")).ok()?;
    let draw = Reflect::get(&renderer, &JsValue::from_str("renderInto")).ok()?;
    draw.is_function().then_some(renderer)
}

/// Whether fc-rich.js is ready, loading it first if nothing has asked yet.
async fn ready() -> bool {
    if renderer().is_some() {
        return true;
    }
    let promise = LOADING.with(|loading| {
        loading
            .borrow_mut()
            .get_or_insert_with(inject_script)
            .clone()
    });
    if JsFuture::from(promise.clone()).await.is_err() {
        LOADING.with(|loading| {
            let mut loading = loading.borrow_mut();
            // Only the load that failed: another may have started since.
            if loading
                .as_ref()
                .is_some_and(|stored| JsValue::from(stored) == JsValue::from(&promise))
            {
                *loading = None;
            }
        });
        return false;
    }
    renderer().is_some()
}

/// A `<script>` for fc-rich.js in the head, and a promise for its load.
fn inject_script() -> Promise {
    Promise::new(&mut |resolve: Function, reject: Function| {
        let document = web_sys::window().and_then(|window| window.document());
        let Some((document, head)) = document.and_then(|document| {
            let head = document.head()?;
            Some((document, head))
        }) else {
            let _ = reject.call0(&JsValue::NULL);
            return;
        };
        let Ok(script) = document.create_element("script") else {
            let _ = reject.call0(&JsValue::NULL);
            return;
        };
        let loaded = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        let failed = {
            let script = script.clone();
            let reject = reject.clone();
            Closure::once_into_js(move || {
                // Gone, so the next try is a fresh element, not a second
                // copy beside a dead one.
                script.remove();
                let _ = reject.call0(&JsValue::NULL);
            })
        };
        let _ = script.add_event_listener_with_callback("load", loaded.unchecked_ref());
        let _ = script.add_event_listener_with_callback("error", failed.unchecked_ref());
        let _ = script.set_attribute("src", &script_url());
        if head.append_child(&script).is_err() {
            let _ = reject.call0(&JsValue::NULL);
        }
    })
}

/// Draw `job` into `stage` with `fcRich.renderInto`: whether it drew.
///
/// The engine's own message on a failure is never logged: it can quote the
/// formula, and the console never holds what a family wrote.
async fn render_into(stage: &Element, job: &JsValue) -> bool {
    let Some(renderer) = renderer() else {
        return false;
    };
    let Some(draw) = Reflect::get(&renderer, &JsValue::from_str("renderInto"))
        .ok()
        .and_then(|draw| draw.dyn_into::<Function>().ok())
    else {
        return false;
    };
    let Some(promise) = draw
        .call2(&renderer, stage, job)
        .ok()
        .and_then(|promise| promise.dyn_into::<Promise>().ok())
    else {
        return false;
    };
    let Ok(result) = JsFuture::from(promise).await else {
        return false;
    };
    Reflect::get(&result, &JsValue::from_str("ok"))
        .ok()
        .and_then(|ok| ok.as_bool())
        .unwrap_or(false)
}

// MARK: - The job

/// What one drawing is asked for — fc-rich.js's JOB (its header is the
/// contract), as the design doc's table fills it in for the web.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    /// `math`, `math-inline`, `mermaid`, `graphviz` or `plot`.
    pub kind: &'static str,
    /// Graphviz's layout; empty for every other kind.
    pub engine: String,
    pub source: String,
    pub dark: bool,
    pub color: String,
    pub background: String,
    /// CSS pixels.
    pub font_size: f64,
}

impl Job {
    /// A formula is drawn in the text colour around it (`text_color`, which
    /// is mine or theirs) on nothing; a diagram or a graph on the theme's
    /// card in the card's ink. Both at the size the words around them are.
    pub fn new(
        kind: &'static str,
        engine: &str,
        source: &str,
        dark: bool,
        text_color: &str,
        font_size: f64,
    ) -> Job {
        let formula = kind == "math" || kind == "math-inline";
        let (color, background) = match (formula, dark) {
            (true, _) => (text_color, "transparent"),
            (false, false) => (CARD_INK_LIGHT, CARD_LIGHT),
            (false, true) => (CARD_INK_DARK, CARD_DARK),
        };
        Job {
            kind,
            engine: engine.to_string(),
            source: source.to_string(),
            dark,
            color: color.to_string(),
            background: background.to_string(),
            font_size,
        }
    }

    /// As the object fc-rich.js takes. `mathml` because a formula drawn in
    /// the page is read by a screen reader from its MathML — the web's own
    /// accessibility, which a native snapshot cannot have.
    fn to_js(&self) -> JsValue {
        let job = Object::new();
        let set = |key: &str, value: JsValue| {
            let _ = Reflect::set(&job, &JsValue::from_str(key), &value);
        };
        set("kind", JsValue::from_str(self.kind));
        set("source", JsValue::from_str(&self.source));
        if !self.engine.is_empty() {
            set("engine", JsValue::from_str(&self.engine));
        }
        set("dark", JsValue::from_bool(self.dark));
        set("color", JsValue::from_str(&self.color));
        set("background", JsValue::from_str(&self.background));
        set("fontSize", JsValue::from_f64(self.font_size));
        set("maxWidth", JsValue::from_f64(MAX_SIDE));
        set("maxHeight", JsValue::from_f64(MAX_SIDE));
        set("mathml", JsValue::TRUE);
        job.into()
    }
}

// MARK: - The theme

/// Whether the page is dark: it follows the system's setting and nothing
/// else (styles.css), so this is the same question.
fn prefers_dark() -> bool {
    web_sys::window()
        .and_then(|window| window.match_media("(prefers-color-scheme: dark)").ok())
        .flatten()
        .is_some_and(|list| list.matches())
}

/// The theme, re-read whenever the system's changes — a drawing's colours
/// are baked into it, so a theme change is a re-draw.
#[hook]
fn use_dark() -> bool {
    let dark = use_state(prefers_dark);
    {
        let dark = dark.clone();
        use_effect_with((), move |_| {
            let listening = web_sys::window()
                .and_then(|window| window.match_media("(prefers-color-scheme: dark)").ok())
                .flatten()
                .map(|list| {
                    let heard = Closure::<dyn Fn()>::new(move || dark.set(prefers_dark()));
                    let _ = list
                        .add_event_listener_with_callback("change", heard.as_ref().unchecked_ref());
                    (list, heard)
                });
            move || {
                if let Some((list, heard)) = listening {
                    let _ = list.remove_event_listener_with_callback(
                        "change",
                        heard.as_ref().unchecked_ref(),
                    );
                }
            }
        });
    }
    *dark
}

/// The text colour and size where the drawing goes — the bubble's own,
/// heading and all — as the page computed them.
fn words_around(wrap: &Element) -> (String, f64) {
    let style = web_sys::window().and_then(|window| window.get_computed_style(wrap).ok().flatten());
    let color = style
        .as_ref()
        .and_then(|style| style.get_property_value("color").ok())
        .filter(|color| !color.is_empty())
        .unwrap_or_else(|| "currentColor".to_string());
    let size = style
        .and_then(|style| style.get_property_value("font-size").ok())
        .and_then(|size| size.trim_end_matches("px").parse::<f64>().ok())
        .filter(|size| *size > 0.0)
        .unwrap_or(BODY_PX);
    (color, size)
}

// MARK: - Keeping the reader's place

/// Make `change` — a drawing landing, which changes the bubble's height —
/// without moving what the reader is looking at, the way the chat keeps its
/// place for everything else: a reader at the newest message stays at the
/// newest (the drawing may be in the message they are reading), and a
/// drawing that lands ABOVE the screen moves the scroll down by what it
/// added, so the words on screen stay put. One on screen or below just
/// grows. The list sets `overflow-anchor: none` (styles.css), so nothing
/// else adjusts for it.
fn keep_place(wrap: &Element, change: impl FnOnce()) {
    let Some(list) = wrap.closest(".messages, .thread-messages").ok().flatten() else {
        change();
        return;
    };
    let top = list.scroll_top();
    let height = list.scroll_height();
    let at_newest = top + list.client_height() >= height - PINNED_SLACK_PX;
    let above = wrap.get_bounding_client_rect().bottom() <= list.get_bounding_client_rect().top();
    change();
    let grown = list.scroll_height() - height;
    if grown == 0 {
        return;
    }
    if at_newest {
        list.set_scroll_top(list.scroll_height());
    } else if above {
        list.set_scroll_top(top + grown);
    }
}

// MARK: - One drawing's life

/// The elements a drawing lives in. `wrap` holds the fallback, the `stage`
/// a drawing is made on, and the `host` it is moved into once whole; the
/// `drawn` attribute on `wrap` is what hides the fallback (styles.css).
///
/// The attribute and the host's children are set by hand, not by Yew: the
/// swap and the height measurement around it (`keep_place`) must happen in
/// one step, and Yew leaves alone what its own markup never mentions.
struct Drawing {
    wrap: NodeRef,
    stage: NodeRef,
    host: NodeRef,
}

#[hook]
fn use_drawing(kind: &'static str, engine: String, source: String, inline: bool) -> Drawing {
    let wrap = use_node_ref();
    let stage = use_node_ref();
    let host = use_node_ref();
    let dark = use_dark();
    {
        let (wrap, stage, host) = (wrap.clone(), stage.clone(), host.clone());
        use_effect_with(
            (kind, engine, source, dark),
            move |(kind, engine, source, dark)| {
                // False once this drawing is superseded (a new theme, an edited
                // body) or its bubble gone: what it then makes is thrown away.
                let alive = Rc::new(Cell::new(true));
                {
                    let alive = alive.clone();
                    let (kind, engine, source, dark) =
                        (*kind, engine.clone(), source.clone(), *dark);
                    spawn_local(async move {
                        if !ready().await || !alive.get() {
                            return;
                        }
                        let (Some(wrap), Some(stage_box), Some(host)) = (
                            wrap.cast::<Element>(),
                            stage.cast::<Element>(),
                            host.cast::<Element>(),
                        ) else {
                            return;
                        };
                        let Some(document) = wrap.owner_document() else {
                            return;
                        };
                        let Ok(sheet) =
                            document.create_element(if inline { "span" } else { "div" })
                        else {
                            return;
                        };
                        if stage_box.append_child(&sheet).is_err() {
                            return;
                        }
                        let (color, size) = words_around(&wrap);
                        let job = Job::new(kind, &engine, &source, dark, &color, size);
                        let drew = render_into(&sheet, &job.to_js()).await;
                        if !alive.get() {
                            sheet.remove();
                            return;
                        }
                        keep_place(&wrap, || {
                            host.set_text_content(None);
                            if drew && host.append_child(&sheet).is_ok() {
                                let _ = wrap.set_attribute("data-drawn", "");
                            } else {
                                // The source again, for good — or until the
                                // next theme gives the engine another try.
                                sheet.remove();
                                let _ = wrap.remove_attribute("data-drawn");
                            }
                        });
                    });
                }
                move || alive.set(false)
            },
        );
    }
    Drawing { wrap, stage, host }
}

/// Whether `wrap` holds a finished drawing.
fn is_drawn(wrap: &NodeRef) -> bool {
    wrap.cast::<Element>()
        .is_some_and(|wrap| wrap.has_attribute("data-drawn"))
}

// MARK: - A block

#[derive(Properties, PartialEq)]
pub struct RichBlockProps {
    pub rich: Rich,
}

/// A rich block in a bubble: drawn, scaled down to the bubble's width; the
/// source as typed until then. A diagram or a graph opens full size on a
/// click — a formula does not, it is already at the size of the words.
#[function_component(RichBlock)]
pub fn rich_block(props: &RichBlockProps) -> Html {
    let rich = &props.rich;
    let kind = rich.kind.word();
    let drawing = use_drawing(kind, rich.engine.clone(), rich.source.clone(), false);
    let opens = rich.kind != RichKind::Math;
    let open = use_state(|| Option::<Element>::None);
    let pending = use_mut_ref(|| Option::<Timeout>::None);

    let show = {
        let open = open.clone();
        let host = drawing.host.clone();
        let wrap = drawing.wrap.clone();
        move || {
            if !is_drawn(&wrap) {
                return;
            }
            if let Some(drawn) = host
                .cast::<Element>()
                .and_then(|host| host.first_element_child())
            {
                open.set(Some(drawn));
            }
        }
    };
    let on_click = {
        let show = show.clone();
        let pending = pending.clone();
        Callback::from(move |_: MouseEvent| {
            let show = show.clone();
            *pending.borrow_mut() = Some(Timeout::new(OPEN_DELAY_MS, show));
        })
    };
    // The second click of a double-click is the bubble's heart.
    let on_double = {
        let pending = pending.clone();
        Callback::from(move |_: MouseEvent| {
            pending.borrow_mut().take();
        })
    };
    let on_key = Callback::from(move |event: KeyboardEvent| {
        if matches!(event.key().as_str(), "Enter" | " ") {
            event.prevent_default();
            show();
        }
    });
    let close = {
        let open = open.clone();
        Callback::from(move |_: ()| open.set(None))
    };

    // The drawing's name for a screen reader is its source — no new words
    // to translate (the design doc's "Accessibility"). A formula has its
    // MathML instead.
    let label = opens.then(|| rich.source.clone());
    html! {
        <div class={classes!("rich-block", format!("rich-{kind}"))} ref={drawing.wrap}>
            <p class="rich-fallback"><span class="mono">{ rich.shown.clone() }</span></p>
            <div class="rich-stage" ref={drawing.stage} aria-hidden="true"></div>
            if opens {
                <div class="rich-host is-openable" ref={drawing.host}
                     role="button" tabindex="0" aria-label={label.clone()}
                     onclick={on_click} ondblclick={on_double} onkeydown={on_key}></div>
            } else {
                <div class="rich-host" ref={drawing.host}></div>
            }
            if let Some(drawn) = (*open).clone() {
                <FullSize {drawn} label={label.unwrap_or_default()} on_close={close} />
            }
        </div>
    }
}

// MARK: - A formula in a line

#[derive(Properties, PartialEq)]
pub struct MathInlineProps {
    /// The formula, without its delimiters (`Style::math`).
    pub source: String,
    /// What was typed, delimiters and all — shown until it is drawn.
    pub typed: String,
}

/// An inline formula: drawn on the line's baseline at the size of the words
/// around it; what was typed, monospaced, until then.
#[function_component(MathInline)]
pub fn math_inline(props: &MathInlineProps) -> Html {
    let drawing = use_drawing("math-inline", String::new(), props.source.clone(), true);
    html! {
        <span class="rich-inline" ref={drawing.wrap}>
            <span class="mono rich-fallback">{ props.typed.clone() }</span>
            <span class="rich-stage" ref={drawing.stage} aria-hidden="true"></span>
            <span class="rich-host" ref={drawing.host}></span>
        </span>
    }
}

// MARK: - Full size

#[derive(Properties, PartialEq)]
struct FullSizeProps {
    /// The drawing in the bubble; shown here as a copy.
    drawn: Element,
    label: String,
    on_close: Callback<()>,
}

/// A diagram or a graph over the whole window: fitted to it, and at its own
/// size — scrolled where it is bigger — after a click on it. The photo
/// viewer's frame (views/viewer.rs), which cannot show it: that one shows a
/// message's attachments, fetched from the server. Esc or ✕ closes.
///
/// It is a portal to the body: inside the bubble a pending message's
/// opacity, or any ancestor's stacking, would hold it.
#[function_component(FullSize)]
fn full_size(props: &FullSizeProps) -> Html {
    let overlay = use_node_ref();
    let stage = use_node_ref();
    let natural = use_state(|| false);
    {
        let stage = stage.clone();
        let overlay = overlay.clone();
        use_effect_with(props.drawn.clone(), move |drawn| {
            if let Some(stage) = stage.cast::<Element>() {
                stage.set_text_content(None);
                // A copy: the bubble keeps its own. (A diagram's ids repeat
                // in it, and both copies are the same drawing, so what they
                // point at is the same.)
                if let Ok(copy) = drawn.clone_node_with_deep(true) {
                    let _ = stage.append_child(&copy);
                }
            }
            if let Some(overlay) = overlay.cast::<HtmlElement>() {
                let _ = overlay.focus();
            }
        });
    }
    let on_key = {
        let on_close = props.on_close.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() == "Escape" {
                event.stop_propagation();
                on_close.emit(());
            }
        })
    };
    let toggle = {
        let natural = natural.clone();
        Callback::from(move |_: MouseEvent| natural.set(!*natural))
    };
    let close = props.on_close.reform(|_: MouseEvent| ());
    let Some(body) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.body())
    else {
        return Html::default();
    };
    create_portal(
        html! {
            <div class="viewer rich-viewer" role="dialog" aria-modal="true"
                 aria-label={props.label.clone()} tabindex="0" ref={overlay} onkeydown={on_key}>
                <header class="viewer-bar">
                    <span class="viewer-title"></span>
                    <button class="link" onclick={close} aria-label={t("Close")}>{ "✕" }</button>
                </header>
                <div class={classes!("viewer-stage", "rich-viewer-stage", natural.then_some("is-natural"))}>
                    <div class="rich-full" ref={stage} onclick={toggle}></div>
                </div>
            </div>
        },
        body.into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The design doc's job table: a formula takes the words' colour on
    /// nothing; a diagram or a graph the theme's card and its ink.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn the_job_follows_the_design_table() {
        let formula = Job::new("math", "", "x^2", true, "rgb(1, 2, 3)", 15.0);
        assert_eq!(formula.color, "rgb(1, 2, 3)");
        assert_eq!(formula.background, "transparent");
        let inline = Job::new("math-inline", "", "x", false, "rgb(9, 9, 9)", 19.35);
        assert_eq!(
            (inline.color.as_str(), inline.font_size),
            ("rgb(9, 9, 9)", 19.35)
        );
        for kind in ["mermaid", "graphviz", "plot"] {
            let light = Job::new(kind, "dot", "a", false, "rgb(1, 2, 3)", 15.0);
            assert_eq!(
                (light.color.as_str(), light.background.as_str()),
                ("#1C1C1E", "#FFFFFF")
            );
            let dark = Job::new(kind, "dot", "a", true, "rgb(1, 2, 3)", 15.0);
            assert_eq!(
                (dark.color.as_str(), dark.background.as_str()),
                ("#F2F2F7", "#1C1C1E")
            );
        }
        let job = Job::new("graphviz", "neato", "graph{a--b}", false, "red", 15.0).to_js();
        let get = |key: &str| Reflect::get(&job, &JsValue::from_str(key)).unwrap();
        assert_eq!(get("kind").as_string().as_deref(), Some("graphviz"));
        assert_eq!(get("engine").as_string().as_deref(), Some("neato"));
        assert_eq!(get("mathml"), JsValue::TRUE);
        assert_eq!(get("fontSize").as_f64(), Some(15.0));
        let math = Job::new("math", "", "x", false, "red", 15.0).to_js();
        assert!(Reflect::get(&math, &JsValue::from_str("engine"))
            .unwrap()
            .is_undefined());
    }

    /// The renderer is asked for under the hash of its files.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn the_script_is_fetched_under_the_hash_of_rich() {
        assert_eq!(VERSION.len(), 16);
        assert!(VERSION.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(script_url(), format!("rich/fc-rich.js?v={VERSION}"));
    }
}
