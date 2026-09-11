//! The call on the screen: who it is, where it has got to, and the three or
//! four things a person can do about it (ios CallView; docs/protocol.md,
//! "Voice calls").
//!
//! A tab is not a phone: there is no full-screen ringing, because a browser
//! cannot be woken and the chat behind the call is worth keeping in view. So
//! a call is a panel over the corner of the app — the whole width of a phone
//! screen, a card on a desktop — and on a video call it grows to hold the
//! picture.

use wasm_bindgen::JsCast;
use web_sys::{HtmlAudioElement, HtmlMediaElement, HtmlVideoElement};
use yew::prelude::*;

use crate::actions::Action;
use crate::calls::{CallState, Calls, Stage};
use crate::views::avatar::Avatar;

#[derive(Properties, PartialEq)]
pub struct CallProps {
    pub call: CallState,
    /// Who is at the other end.
    pub name: String,
    pub avatar_version: i64,
    pub on_action: Callback<Action>,
}

#[function_component(CallPanel)]
pub fn call_panel(props: &CallProps) -> Html {
    let calls = use_context::<Calls>();
    let remote = use_node_ref();
    let local = use_node_ref();
    let accept = use_node_ref();
    let call = &props.call;
    // The duration is a clock, so it needs a second's worth of redraw of
    // its own — the app around it has no reason to render meanwhile.
    let redraw = use_force_update();
    {
        let talking = call.stage == Stage::Talking;
        use_effect_with(talking, move |talking| {
            let ticker = talking.then(|| {
                gloo_timers::callback::Interval::new(1_000, move || redraw.force_update())
            });
            move || drop(ticker)
        });
    }

    // The streams, attached whenever one appears — a track arriving bumps
    // `media`, which is what brings this effect round again. Playing is
    // asked for here and not promised: the click that placed or answered
    // the call is the gesture a browser wants before it will sound
    // anything, and a refusal is the browser's to make.
    {
        let remote = remote.clone();
        let local = local.clone();
        let calls = calls.clone();
        use_effect_with((call.media, call.stage, call.video), move |_| {
            let Some(calls) = calls else { return };
            attach(&remote, calls.remote_stream());
            attach(&local, calls.local_stream());
        });
    }

    // A ringing tab says so in its title: with the window behind another
    // one, that and the tone are all there is (docs/protocol.md, "A browser
    // is a client too" — a browser rings only while its tab is open).
    {
        let name = props.name.clone();
        let ringing = call.stage == Stage::Incoming;
        use_effect_with(ringing, move |ringing| {
            let document = web_sys::window().and_then(|window| window.document());
            let held = document.as_ref().map(|document| document.title());
            if *ringing {
                if let Some(document) = document.as_ref() {
                    document.set_title(&format!("☎ {name} is calling"));
                }
            }
            move || {
                if let (Some(document), Some(held)) = (document, held) {
                    document.set_title(&held);
                }
            }
        });
    }

    // A ringing call takes the focus, on Accept: Return answers it, and a
    // screen reader reads the call rather than wherever the page was.
    {
        let accept = accept.clone();
        let ringing = call.stage == Stage::Incoming;
        use_effect_with(ringing, move |ringing| {
            if *ringing {
                if let Some(button) = accept.cast::<web_sys::HtmlElement>() {
                    let _ = button.focus();
                }
            }
        });
    }

    let act = |action: Action| {
        let on_action = props.on_action.clone();
        Callback::from(move |_: MouseEvent| on_action.emit(action.clone()))
    };
    let status = status_line(call, crate::sync::wall_ms());
    let incoming = call.stage == Stage::Incoming;
    let over = call.stage == Stage::Ended;
    let talking = call.stage == Stage::Talking;

    html! {
        <section
            class={classes!("call", call.video.then_some("is-video"), incoming.then_some("is-incoming"))}
            role="dialog"
            aria-label={format!("Call with {}", props.name)}
        >
            <div class="call-who">
                <Avatar
                    title={props.name.clone()}
                    user_id={Some(call.peer_user_id)}
                    version={props.avatar_version}
                    size={44}
                />
                <div class="identity-text">
                    <strong>{ props.name.clone() }</strong>
                    // Announced when it CHANGES — but a running clock is
                    // not news every second, so the duration is left out
                    // of the live region and read on demand.
                    <span class="muted" aria-live={if talking { "off" } else { "polite" }}>
                        { status }
                    </span>
                </div>
            </div>
            if call.video && matches!(call.stage, Stage::Connecting | Stage::Talking) {
                <div class="call-picture">
                    // The far side. A camera turned off simply stops
                    // sending: the element keeps the last frame and the
                    // call goes on, which is what the protocol describes.
                    <video ref={remote.clone()} class="call-remote" autoplay={true} playsinline={true} />
                    if call.camera {
                        <video ref={local} class="call-local" autoplay={true} playsinline={true} muted={true} />
                    }
                </div>
            } else {
                // Voice only: nothing to look at, and the audio has to live
                // somewhere.
                <audio ref={remote.clone()} autoplay={true} />
            }
            <div class="call-actions">
                if over {
                    // Nothing left to do: the panel is saying why, and
                    // goes on its own.
                } else if incoming {
                    <button class="danger-button" onclick={act(Action::DeclineCall)}>{ "Decline" }</button>
                    <button class="primary" ref={accept} onclick={act(Action::AnswerCall)}>{ "Accept" }</button>
                } else {
                    // Each button is named for what it DOES, so it needs no
                    // pressed state to be read correctly.
                    <button
                        class={classes!("secondary", call.muted.then_some("is-chosen"))}
                        onclick={act(Action::ToggleMute)}
                    >{ if call.muted { "Unmute" } else { "Mute" } }</button>
                    if call.video {
                        <button
                            class={classes!("secondary", (!call.camera).then_some("is-chosen"))}
                            onclick={act(Action::ToggleCamera)}
                        >{ if call.camera { "Turn camera off" } else { "Turn camera on" } }</button>
                    }
                    <button class="danger-button" onclick={act(Action::EndCall)}>
                        { if call.answered() { "Hang Up" } else { "Cancel" } }
                    </button>
                }
            </div>
        </section>
    }
}

/// A stream into its element, played. The promise `play` answers with is
/// swallowed on purpose: a call that ends while it is in flight rejects it
/// ("the media was removed from the document"), and that is not a failure
/// anybody needs to read about.
fn attach(node: &NodeRef, stream: Option<web_sys::MediaStream>) {
    let (Some(element), Some(stream)) = (media_element(node), stream) else {
        return;
    };
    element.set_src_object(Some(&stream));
    if let Ok(playing) = element.play() {
        wasm_bindgen_futures::spawn_local(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(playing).await;
        });
    }
}

fn media_element(node: &NodeRef) -> Option<HtmlMediaElement> {
    node.cast::<HtmlVideoElement>()
        .map(|video| video.unchecked_into::<HtmlMediaElement>())
        .or_else(|| {
            node.cast::<HtmlAudioElement>()
                .map(|audio| audio.unchecked_into::<HtmlMediaElement>())
        })
}

/// Where the call is, in words — and once it is up, how long it has been.
pub fn status_line(call: &CallState, now_ms: f64) -> String {
    match call.stage {
        Stage::Dialling => "Calling…".to_string(),
        Stage::Ringing => "Ringing…".to_string(),
        Stage::Incoming if call.video => "Incoming video call".to_string(),
        Stage::Incoming => "Incoming call".to_string(),
        Stage::Connecting => "Connecting…".to_string(),
        Stage::Talking => match call.answered_at {
            Some(answered) => fc_text::call_record::duration(((now_ms - answered) / 1000.0) as i64),
            None => "Connected".to_string(),
        },
        // Over: why, in the apps' words, for the moment the panel stays.
        Stage::Ended => {
            crate::calls::ended_line(call.ended_reason.as_deref().unwrap_or("hangup"), call)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn call(stage: Stage, video: bool) -> CallState {
        CallState {
            call_id: "c".into(),
            chat_id: 42,
            peer_user_id: 9,
            outgoing: true,
            video,
            stage,
            muted: false,
            camera: video,
            taken: stage == Stage::Talking,
            answered_at: (stage == Stage::Talking).then_some(1_000.0),
            media: 0,
            ended_reason: None,
        }
    }

    #[wasm_bindgen_test]
    fn the_status_says_where_the_call_is() {
        assert_eq!(status_line(&call(Stage::Dialling, false), 0.0), "Calling…");
        assert_eq!(status_line(&call(Stage::Ringing, false), 0.0), "Ringing…");
        assert_eq!(
            status_line(&call(Stage::Incoming, true), 0.0),
            "Incoming video call"
        );
        // Once it is up, the status IS the clock.
        assert_eq!(status_line(&call(Stage::Talking, false), 13_000.0), "0:12");
        // And once it is over, why — in the apps' words.
        let mut ended = call(Stage::Ended, false);
        ended.ended_reason = Some("timeout".into());
        assert_eq!(status_line(&ended, 0.0), "No answer");
        ended.ended_reason = Some("busy".into());
        assert_eq!(status_line(&ended, 0.0), "Busy");
        ended.outgoing = false;
        ended.ended_reason = Some("timeout".into());
        assert_eq!(status_line(&ended, 0.0), "Missed voice call");
    }
}
