//! A message's photos and videos, one at a time and full size — the Mac's
//! viewer window (MacAttachmentViewer), as an overlay over the chat.
//!
//! ←/→ page and stop at the ends, Esc closes. A photo zooms from 1× to 6× —
//! double-click toggles 1× and 2×, a pinch (a browser's ctrl-wheel) or the
//! buttons go between — and pans by dragging once it is bigger than the
//! window. A video plays in the browser's own player, which brings full
//! screen and picture-in-picture with it. Save hands over the ORIGINAL,
//! never the preview a bubble draws.

use fc_text::i18n::{t, t2};
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;
use yew::prelude::*;

use crate::media::{download, use_media, Variant};
use crate::model::Attachment;
use crate::views::attachments::file_name;

/// The zoom a photo may go to.
pub const MAX_ZOOM: f64 = 6.0;

/// The next zoom after a step in `direction` (+1 in, -1 out), clamped.
pub fn zoom_step(scale: f64, direction: f64) -> f64 {
    let next = if direction > 0.0 {
        scale * 1.25
    } else {
        scale / 1.25
    };
    next.clamp(1.0, MAX_ZOOM)
}

#[derive(Properties, PartialEq)]
pub struct ViewerProps {
    pub items: Vec<Attachment>,
    pub index: usize,
    pub on_step: Callback<usize>,
    pub on_close: Callback<()>,
}

#[function_component(Viewer)]
pub fn viewer(props: &ViewerProps) -> Html {
    let overlay = use_node_ref();
    let index = props.index.min(props.items.len().saturating_sub(1));
    let item = props.items.get(index).cloned().unwrap_or_default();
    let count = props.items.len();

    // Focus the overlay, so the keys reach it.
    {
        let overlay = overlay.clone();
        use_effect_with((), move |_| {
            if let Some(element) = overlay.cast::<HtmlElement>() {
                let _ = element.focus();
            }
        });
    }

    let on_key = {
        let on_step = props.on_step.clone();
        let on_close = props.on_close.clone();
        Callback::from(move |event: KeyboardEvent| match event.key().as_str() {
            "Escape" => on_close.emit(()),
            "ArrowLeft" if index > 0 => on_step.emit(index - 1),
            "ArrowRight" if index + 1 < count => on_step.emit(index + 1),
            _ => {}
        })
    };
    let close = props.on_close.reform(|_: MouseEvent| ());
    let previous = (index > 0).then(|| props.on_step.reform(move |_: MouseEvent| index - 1));
    let next = (index + 1 < count).then(|| props.on_step.reform(move |_: MouseEvent| index + 1));
    let title = fc_text::media::display_name(&item.kind, item.name.as_deref()).into_owned();

    html! {
        <div class="viewer" role="dialog" aria-modal="true" aria-label={title.clone()}
             tabindex="0" ref={overlay} onkeydown={on_key}>
            <header class="viewer-bar">
                <span class="viewer-title">{ title }</span>
                if count > 1 {
                    <span class="viewer-position">{ t2("%lld of %lld", &(index + 1).to_string(), &count.to_string()) }</span>
                }
                <SaveButton attachment={item.clone()} />
                <button class="link" onclick={close} aria-label={t("Close")}>{ "✕" }</button>
            </header>
            <div class="viewer-stage">
                if let Some(previous) = previous {
                    <button class="viewer-arrow is-previous" onclick={previous} aria-label={t("Previous")}>{ "‹" }</button>
                }
                if item.kind == "video" {
                    <VideoView key={item.id} attachment={item.clone()} />
                } else {
                    <PhotoView key={item.id} attachment={item.clone()} />
                }
                if let Some(next) = next {
                    <button class="viewer-arrow is-next" onclick={next} aria-label={t("Next")}>{ "›" }</button>
                }
            </div>
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct ItemProps {
    attachment: Attachment,
}

/// Save — the original's bytes, once this tab has them.
#[function_component(SaveButton)]
fn save_button(props: &ItemProps) -> Html {
    let url = use_media(props.attachment.id, Variant::Original, true);
    let name = file_name(&props.attachment);
    let save = url
        .clone()
        .map(|url| Callback::from(move |_: MouseEvent| download(&url, &name)));
    html! {
        <button class="secondary" onclick={save.clone().unwrap_or_default()} disabled={save.is_none()}>
            { t("Save…") }
        </button>
    }
}

#[function_component(PhotoView)]
fn photo_view(props: &ItemProps) -> Html {
    let attachment = &props.attachment;
    let full = use_media(attachment.id, Variant::Original, true);
    let preview = use_media(attachment.id, Variant::Preview, attachment.has_preview);
    let scale = use_state(|| 1.0_f64);
    let pan = use_state(|| (0.0_f64, 0.0_f64));
    let drag = use_mut_ref(|| Option::<(f64, f64, f64, f64)>::None);

    let set_scale = {
        let scale = scale.clone();
        let pan = pan.clone();
        Callback::from(move |next: f64| {
            if next <= 1.0 {
                pan.set((0.0, 0.0));
            }
            scale.set(next);
        })
    };
    let on_double = {
        let set_scale = set_scale.clone();
        let current = *scale;
        Callback::from(move |_: MouseEvent| set_scale.emit(if current > 1.0 { 1.0 } else { 2.0 }))
    };
    let on_wheel = {
        let set_scale = set_scale.clone();
        let current = *scale;
        Callback::from(move |event: WheelEvent| {
            // A trackpad pinch reaches a page as a wheel with ctrl held.
            if event.ctrl_key() || event.meta_key() {
                event.prevent_default();
                set_scale.emit(zoom_step(current, -event.delta_y().signum()));
            }
        })
    };
    let on_down = {
        let drag = drag.clone();
        let pan = pan.clone();
        let current = *scale;
        Callback::from(move |event: PointerEvent| {
            if current > 1.0 {
                *drag.borrow_mut() = Some((
                    f64::from(event.client_x()),
                    f64::from(event.client_y()),
                    pan.0,
                    pan.1,
                ));
                if let Some(target) = event
                    .target()
                    .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                {
                    let _ = target.set_pointer_capture(event.pointer_id());
                }
            }
        })
    };
    let on_move = {
        let drag = drag.clone();
        let pan = pan.clone();
        let current = *scale;
        Callback::from(move |event: PointerEvent| {
            if let Some((x, y, from_x, from_y)) = *drag.borrow() {
                let dx = (f64::from(event.client_x()) - x) / current;
                let dy = (f64::from(event.client_y()) - y) / current;
                pan.set((from_x + dx, from_y + dy));
            }
        })
    };
    let on_up = {
        let drag = drag.clone();
        Callback::from(move |_: PointerEvent| {
            *drag.borrow_mut() = None;
        })
    };
    let zoom_in = {
        let set_scale = set_scale.clone();
        let current = *scale;
        Callback::from(move |_: MouseEvent| set_scale.emit(zoom_step(current, 1.0)))
    };
    let zoom_out = {
        let current = *scale;
        Callback::from(move |_: MouseEvent| set_scale.emit(zoom_step(current, -1.0)))
    };
    let shown = full.clone().or(preview);
    html! {
        <div class="viewer-photo" onwheel={on_wheel}>
            if let Some(url) = shown {
                <img
                    src={url}
                    alt={fc_text::media::display_name(&attachment.kind, attachment.name.as_deref()).into_owned()}
                    draggable="false"
                    class={classes!((*scale > 1.0).then_some("is-zoomed"))}
                    style={format!("transform:scale({:.3}) translate({:.1}px,{:.1}px)", *scale, pan.0, pan.1)}
                    ondblclick={on_double}
                    onpointerdown={on_down}
                    onpointermove={on_move}
                    onpointerup={on_up.clone()}
                    onpointercancel={on_up}
                />
            } else {
                <p class="viewer-loading">{ t("Loading…") }</p>
            }
            if full.is_none() {
                <p class="viewer-loading is-over">{ t("Loading…") }</p>
            }
            <div class="viewer-zoom">
                <button class="secondary" onclick={zoom_out} disabled={*scale <= 1.0} aria-label={t("Zoom out")}>{ "−" }</button>
                <span>{ format!("{:.0}%", *scale * 100.0) }</span>
                <button class="secondary" onclick={zoom_in} disabled={*scale >= MAX_ZOOM} aria-label={t("Zoom in")}>{ "+" }</button>
            </div>
        </div>
    }
}

/// A video, in the browser's own player — once its bytes are here, which
/// for a long clip is a wait the poster fills.
#[function_component(VideoView)]
fn video_view(props: &ItemProps) -> Html {
    let attachment = &props.attachment;
    let full = use_media(attachment.id, Variant::Original, true);
    let poster = use_media(attachment.id, Variant::Preview, attachment.has_preview);
    html! {
        <div class="viewer-video">
            if let Some(url) = full {
                <video src={url} poster={poster.unwrap_or_default()} controls=true autoplay=true playsinline=true />
            } else {
                if let Some(poster) = poster {
                    <img src={poster} alt="" draggable="false" />
                }
                <p class="viewer-loading is-over">{ t("Loading video…") }</p>
            }
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn zoom_goes_from_one_to_six_and_no_further() {
        assert_eq!(zoom_step(1.0, -1.0), 1.0, "never smaller than the window");
        assert!((zoom_step(1.0, 1.0) - 1.25).abs() < 1e-9);
        assert_eq!(zoom_step(5.5, 1.0), MAX_ZOOM);
        assert_eq!(zoom_step(MAX_ZOOM, 1.0), MAX_ZOOM);
        assert!((zoom_step(2.0, -1.0) - 1.6).abs() < 1e-9);
    }
}
