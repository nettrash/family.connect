//! While a voice message is being recorded, the app's live regions are quiet
//! (the plan for #79, docs/audio-video-messages-2026-10-04.md, S6): a screen
//! reader would otherwise speak "Connecting…", a send that failed or a
//! transcript's progress into the note, and nobody can take a sent note
//! back. Only what the recording does is said — by the conversation's own
//! announcement node — and the conversation's typing and notice lines go
//! quiet by themselves.
//!
//! [`QuietRoot`] holds the fact for the whole app, the conversation tells it
//! ([`use_quiet_while`]), and every other live region is a [`LiveRegion`],
//! which drops its role and says `aria-live="off"` while it holds. The call
//! band is not one: a call stops the recording (S4), and what it says about
//! itself is the call's to say.

use std::cell::RefCell;
use std::rc::Rc;

use yew::prelude::*;

/// Whether a voice message is being recorded, as the app's live regions read
/// it — and how the conversation says so; and where a play control, dimmed
/// while it records, says why (S1.7: "You can play this after recording.").
#[derive(Clone)]
pub struct Quiet {
    pub on: bool,
    set: Callback<bool>,
    /// The recording pane's notice line — written by the pane at every
    /// render, read only when a dimmed control is activated, so it draws
    /// nothing again.
    reason: Rc<RefCell<Callback<String>>>,
}

impl PartialEq for Quiet {
    fn eq(&self, other: &Self) -> bool {
        self.on == other.on && self.set == other.set && Rc::ptr_eq(&self.reason, &other.reason)
    }
}

#[derive(Properties, PartialEq)]
pub struct QuietRootProps {
    #[prop_or_default]
    pub children: Html,
}

/// The app's quiet, for everything drawn inside it.
#[function_component(QuietRoot)]
pub fn quiet_root(props: &QuietRootProps) -> Html {
    let on = use_state(|| false);
    let set = {
        let on = on.clone();
        use_callback((), move |value: bool, _| on.set(value))
    };
    let reason = use_mut_ref(Callback::noop);
    html! {
        <ContextProvider<Quiet> context={Quiet { on: *on, set, reason }}>
            { props.children.clone() }
        </ContextProvider<Quiet>>
    }
}

/// Whether the app is quiet now. Loud where nothing holds the fact — a view
/// drawn on its own.
#[hook]
pub fn use_quiet() -> bool {
    use_context::<Quiet>().is_some_and(|quiet| quiet.on)
}

/// The app is quiet while `recording` — said to the root as it changes, and
/// taken back when it ends or the pane that said it goes.
#[hook]
pub fn use_quiet_while(recording: bool) {
    let set = use_context::<Quiet>().map(|quiet| quiet.set);
    use_effect_with(recording, move |recording| {
        let recording = *recording;
        if let Some(set) = &set {
            set.emit(recording);
        }
        move || {
            if recording {
                if let Some(set) = set {
                    set.emit(false);
                }
            }
        }
    });
}

/// Where a play control dimmed by a recording says why: `say`, the
/// recording pane's own notice line (S1.3: "says WHY when activated, in the
/// composer's existing notice line").
#[hook]
pub fn use_quiet_reason(say: Callback<String>) {
    if let Some(quiet) = use_context::<Quiet>() {
        *quiet.reason.borrow_mut() = say;
    }
}

/// Say why a control is dimmed, on the recording pane's notice line —
/// nowhere, where nothing holds the fact.
#[hook]
pub fn use_quiet_explain() -> Callback<String> {
    let reason = use_context::<Quiet>().map(|quiet| quiet.reason);
    Callback::from(move |text: String| {
        if let Some(reason) = &reason {
            let say = reason.borrow().clone();
            say.emit(text);
        }
    })
}

#[derive(Properties, PartialEq)]
pub struct LiveRegionProps {
    /// The element: a paragraph unless said otherwise.
    #[prop_or("p")]
    pub tag: &'static str,
    #[prop_or_default]
    pub class: Classes,
    /// "status" or "alert": the role that makes it a live region.
    pub role: &'static str,
    /// What it says is still being fetched (`aria-busy`).
    #[prop_or_default]
    pub busy: bool,
    #[prop_or_default]
    pub children: Html,
}

/// A live region of the app's — with no role, and `aria-live="off"`, while
/// a voice message is being recorded.
#[function_component(LiveRegion)]
pub fn live_region(props: &LiveRegionProps) -> Html {
    let quiet = use_quiet();
    html! {
        <@{props.tag}
            class={props.class.clone()}
            role={(!quiet).then_some(props.role)}
            aria-live={quiet.then_some("off")}
            aria-busy={props.busy.then_some("true")}
        >
            { props.children.clone() }
        </@>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gloo_timers::future::TimeoutFuture;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    #[derive(Properties, PartialEq)]
    struct HostProps {
        recording: bool,
    }

    /// A recording that runs, the way the conversation says so, beside a
    /// live region of the app's.
    #[function_component(Host)]
    fn host(props: &HostProps) -> Html {
        html! {
            <QuietRoot>
                <Recorder recording={props.recording} />
                <LiveRegion tag="span" class="said" role="alert">{ "Not sent." }</LiveRegion>
                <LiveRegion class="busy" role="status" busy=true>{ "Getting the text…" }</LiveRegion>
            </QuietRoot>
        }
    }

    #[function_component(Recorder)]
    fn recorder(props: &HostProps) -> Html {
        use_quiet_while(props.recording);
        Html::default()
    }

    #[derive(Properties, PartialEq)]
    struct ShowProps {
        shown: bool,
    }

    /// The same, with the recording's pane there or gone.
    #[function_component(Shown)]
    fn shown(props: &ShowProps) -> Html {
        html! {
            <QuietRoot>
                if props.shown {
                    <Recorder recording=true />
                }
                <LiveRegion class="said" role="status">{ "Connecting…" }</LiveRegion>
            </QuietRoot>
        }
    }

    fn root() -> web_sys::Element {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        root
    }

    fn attributes(root: &web_sys::Element, selector: &str) -> (Option<String>, Option<String>) {
        let element = root.query_selector(selector).unwrap().expect("drawn");
        (
            element.get_attribute("role"),
            element.get_attribute("aria-live"),
        )
    }

    /// S6: while a recording runs, the app's other live regions are quiet —
    /// no role, `aria-live="off"` — and loud again once it ends.
    #[wasm_bindgen_test]
    async fn the_apps_live_regions_are_quiet_while_a_recording_runs() {
        let root = root();
        let mut handle = yew::Renderer::<Host>::with_root_and_props(
            root.clone(),
            HostProps { recording: false },
        )
        .render();
        TimeoutFuture::new(20).await;
        assert_eq!(
            attributes(&root, "span.said"),
            (Some("alert".into()), None),
            "a live region as it was"
        );
        assert_eq!(attributes(&root, "p.busy"), (Some("status".into()), None));
        assert_eq!(
            root.query_selector("p.busy")
                .unwrap()
                .unwrap()
                .get_attribute("aria-busy")
                .as_deref(),
            Some("true")
        );

        handle.update(HostProps { recording: true });
        TimeoutFuture::new(20).await;
        assert_eq!(
            attributes(&root, "span.said"),
            (None, Some("off".into())),
            "quiet while it records"
        );
        assert_eq!(attributes(&root, "p.busy"), (None, Some("off".into())));
        assert_eq!(
            root.query_selector("span.said")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Not sent."),
            "still shown"
        );

        handle.update(HostProps { recording: false });
        TimeoutFuture::new(20).await;
        assert_eq!(attributes(&root, "span.said"), (Some("alert".into()), None));
        handle.destroy();
        root.remove();
    }

    /// The pane that was recording going away takes the quiet with it.
    #[wasm_bindgen_test]
    async fn the_quiet_goes_with_the_pane_that_asked_for_it() {
        let root = root();
        let mut handle =
            yew::Renderer::<Shown>::with_root_and_props(root.clone(), ShowProps { shown: true })
                .render();
        TimeoutFuture::new(20).await;
        assert_eq!(attributes(&root, "p.said"), (None, Some("off".into())));
        handle.update(ShowProps { shown: false });
        TimeoutFuture::new(20).await;
        assert_eq!(attributes(&root, "p.said"), (Some("status".into()), None));
        handle.destroy();
        root.remove();
    }

    /// Drawn on its own, with nothing holding the fact, a region is loud.
    #[wasm_bindgen_test]
    async fn a_region_with_no_root_is_loud() {
        let root = root();
        #[function_component(Alone)]
        fn alone() -> Html {
            html! { <LiveRegion class="said" role="status">{ "Loading…" }</LiveRegion> }
        }
        let handle = yew::Renderer::<Alone>::with_root(root.clone()).render();
        TimeoutFuture::new(20).await;
        let said = root
            .query_selector("p.said")
            .unwrap()
            .unwrap()
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap();
        assert_eq!(said.get_attribute("role").as_deref(), Some("status"));
        assert!(said.get_attribute("aria-live").is_none());
        handle.destroy();
        root.remove();
    }
}
