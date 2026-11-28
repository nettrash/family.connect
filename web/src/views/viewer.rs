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
use js_sys::{Array, Object, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{future_to_promise, spawn_local, JsFuture};
use web_sys::{Blob, Clipboard, ClipboardItem, HtmlElement};
use yew::prelude::*;

use crate::media::{download, use_media, MediaLoader, Variant};
use crate::model::Attachment;
use crate::views::animation::AnimationView;
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
    #[prop_or_default]
    pub copy_images: bool,
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
    // An animation is drawn as a picture that moves, and named as one.
    let animation = item.kind == "video" && item.animation;
    let title = if animation {
        t("GIF").to_string()
    } else {
        fc_text::media::display_name(&item.kind, item.name.as_deref()).into_owned()
    };

    html! {
        <div class="viewer" role="dialog" aria-modal="true" aria-label={title.clone()}
             tabindex="0" ref={overlay} onkeydown={on_key}>
            <header class="viewer-bar">
                <span class="viewer-title">{ title }</span>
                if count > 1 {
                    <span class="viewer-position">{ t2("%lld of %lld", &(index + 1).to_string(), &count.to_string()) }</span>
                }
                <FileActions key={item.id} attachment={item.clone()}
                    copy_image={props.copy_images && item.id > 0 && item.kind == "photo" && !item.sticker} />
                <button class="link" onclick={close} aria-label={t("Close")}>{ "✕" }</button>
            </header>
            <div class="viewer-stage">
                if let Some(previous) = previous {
                    <button class="viewer-arrow is-previous" onclick={previous} aria-label={t("Previous")}>{ "‹" }</button>
                }
                if animation {
                    <AnimationView key={item.id} attachment={item.clone()} />
                } else if item.kind == "video" {
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

#[derive(Properties, PartialEq)]
struct FileActionsProps {
    attachment: Attachment,
    copy_image: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CopyImageState {
    Idle,
    Writing,
    Copied,
    Failed,
}

fn image_clipboard() -> Option<Clipboard> {
    let window = web_sys::window()?;
    if !window.is_secure_context()
        || !Reflect::get(window.as_ref(), &"ClipboardItem".into())
            .ok()?
            .is_function()
    {
        return None;
    }
    let clipboard = Reflect::get(window.navigator().as_ref(), &"clipboard".into()).ok()?;
    if !Reflect::get(&clipboard, &"write".into())
        .ok()?
        .is_function()
    {
        return None;
    }
    clipboard.dyn_into().ok()
}

fn write_image(clipboard: &Clipboard, original: Blob) -> Result<Promise, JsValue> {
    // Start the clipboard write in the click handler. Safari can lose the
    // gesture if we await decoding first. The promise owns the bytes even
    // if the viewer closes or the cache revokes its URL.
    let png = future_to_promise(async move {
        crate::prep::full_png(&original)
            .await
            .map(JsValue::from)
            .ok_or_else(|| JsValue::from_str("Could not encode PNG"))
    });
    let formats = Object::new();
    Reflect::set(&formats, &"image/png".into(), &png)?;
    let item = ClipboardItem::new_with_record_from_str_to_blob_promise(&formats)?;
    Ok(clipboard.write(&Array::of1(&item)))
}

#[function_component(FileActions)]
fn file_actions(props: &FileActionsProps) -> Html {
    let id = props.attachment.id;
    let url = use_media(id, Variant::Original, true);
    let loaded = url.is_some();
    let name = file_name(&props.attachment);
    let save = url.map(|url| Callback::from(move |_: MouseEvent| download(&url, &name)));
    html! {
        <>
            if props.copy_image {
                <CopyImageButton {id} {loaded} />
            }
            <button class="secondary" onclick={save.clone().unwrap_or_default()} disabled={save.is_none()}>
                { t("Save…") }
            </button>
        </>
    }
}

#[derive(Properties, PartialEq)]
struct CopyImageProps {
    id: i64,
    loaded: bool,
}

#[function_component(CopyImageButton)]
fn copy_image_button(props: &CopyImageProps) -> Html {
    // Own the feedback in a child with no media-loading hook. Updating this
    // state must not restart Save or the viewer's normal load after eviction.
    let id = props.id;
    let loader = use_context::<MediaLoader>();
    let state = use_state(|| CopyImageState::Idle);
    let clipboard = image_clipboard();
    let available = props.loaded
        && loader
            .as_ref()
            .and_then(|media| media.held(id, Variant::Original))
            .is_some_and(|blob| blob.size() > 0.0);
    let writing = *state == CopyImageState::Writing;

    let copy = clipboard.as_ref().map(|clipboard| {
        let clipboard = clipboard.clone();
        let state = state.clone();
        Callback::from(move |_: MouseEvent| {
            let Some(original) = loader
                .as_ref()
                .and_then(|media| media.held(id, Variant::Original))
                .filter(|blob| blob.size() > 0.0)
            else {
                // An earlier render may still have an enabled button after eviction.
                state.set(CopyImageState::Idle);
                return;
            };
            let Ok(done) = write_image(&clipboard, original) else {
                state.set(CopyImageState::Failed);
                return;
            };
            state.set(CopyImageState::Writing);
            let state = state.clone();
            spawn_local(async move {
                state.set(if JsFuture::from(done).await.is_ok() {
                    CopyImageState::Copied
                } else {
                    CopyImageState::Failed
                });
            });
        })
    });
    let status = match (*state, available) {
        (CopyImageState::Writing, _) => "",
        (_, false) => t("Full image unavailable"),
        (CopyImageState::Copied, _) => t("Image copied"),
        (CopyImageState::Failed, _) => t("Could not copy image"),
        (CopyImageState::Idle, _) => "",
    };

    let Some(copy) = copy else {
        return Html::default();
    };
    html! {
        <>
            <button class="secondary" onclick={copy} disabled={!available || writing}
                aria-describedby="viewer-copy-status">
                { t("Copy Image") }
            </button>
            <span id="viewer-copy-status" class="viewer-copy-status" role="status">{ status }</span>
        </>
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
                <video src={url} poster={poster.unwrap_or_default()} controls=true autoplay=true playsinline=true
                       data-playback="true" />
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

    use std::cell::RefCell;
    use std::rc::Rc;

    use gloo_timers::future::TimeoutFuture;
    use wasm_bindgen::closure::Closure;
    use web_sys::{BlobPropertyBag, CanvasRenderingContext2d, HtmlCanvasElement, ImageBitmap};

    use crate::fake_server::{Answer, FakeServer};
    use crate::live::{AppState, Live};

    #[derive(Properties, PartialEq)]
    struct TestViewerProps {
        loader: MediaLoader,
        items: Vec<Attachment>,
    }

    #[function_component(TestViewer)]
    fn test_viewer(props: &TestViewerProps) -> Html {
        let index = use_state(|| 0);
        let on_step = {
            let index = index.clone();
            Callback::from(move |next| index.set(next))
        };
        html! {
            <ContextProvider<MediaLoader> context={props.loader.clone()}>
                <Viewer items={props.items.clone()} index={*index} {on_step}
                    on_close={Callback::noop()} copy_images={true} />
            </ContextProvider<MediaLoader>>
        }
    }

    struct ClipboardWrite {
        clipboard: Clipboard,
        _write: Closure<dyn FnMut(Array) -> Promise>,
        payloads: Rc<RefCell<Vec<Promise>>>,
    }

    impl ClipboardWrite {
        fn record() -> Self {
            let clipboard = image_clipboard().expect("test browser has an image clipboard");
            let payloads = Rc::new(RefCell::new(Vec::new()));
            let captured = payloads.clone();
            let write = Closure::<dyn FnMut(Array) -> Promise>::new(move |items: Array| {
                let item: ClipboardItem = items.get(0).dyn_into().unwrap();
                let payload = item.get_type("image/png");
                captured.borrow_mut().push(payload.clone());
                payload
            });
            assert!(Reflect::set(clipboard.as_ref(), &"write".into(), write.as_ref()).unwrap());
            Self {
                clipboard,
                _write: write,
                payloads,
            }
        }
    }

    impl Drop for ClipboardWrite {
        fn drop(&mut self) {
            let _ =
                Reflect::delete_property::<Object>(self.clipboard.unchecked_ref(), &"write".into());
        }
    }

    fn mount_viewer(
        photos: &[(i64, bool)],
        loader: &MediaLoader,
    ) -> (HtmlElement, yew::AppHandle<TestViewer>) {
        let document = web_sys::window().unwrap().document().unwrap();
        let root: HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let items = photos
            .iter()
            .map(|&(id, has_preview)| Attachment {
                id,
                has_preview,
                kind: "photo".into(),
                ..Default::default()
            })
            .collect();
        let viewer = yew::Renderer::<TestViewer>::with_root_and_props(
            root.clone().into(),
            TestViewerProps {
                loader: loader.clone(),
                items,
            },
        )
        .render();
        (root, viewer)
    }

    fn loader() -> MediaLoader {
        MediaLoader::new(Live::new(
            AppState {
                token: Some("test".into()),
                ..Default::default()
            },
            Rc::new(|| {}),
        ))
    }

    fn png(width: u32, height: u32) -> Blob {
        let window = web_sys::window().unwrap();
        let canvas: HtmlCanvasElement = window
            .document()
            .unwrap()
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        canvas.set_width(width);
        canvas.set_height(height);
        let context: CanvasRenderingContext2d = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        context.set_fill_style_str("#c32756");
        context.fill_rect(
            1.0,
            1.0,
            f64::from(width.saturating_sub(1)),
            f64::from(height.saturating_sub(1)),
        );
        let url = canvas.to_data_url_with_type("image/png").unwrap();
        let base64 = url.split_once(',').unwrap().1;
        let bytes: Vec<u8> = window
            .atob(base64)
            .unwrap()
            .chars()
            .map(|byte| byte as u8)
            .collect();
        let parts = Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
        let options = BlobPropertyBag::new();
        options.set_type("image/png");
        Blob::new_with_u8_array_sequence_and_options(&parts, &options).unwrap()
    }

    fn copy_button(root: &HtmlElement) -> HtmlElement {
        root.query_selector("button[aria-describedby=viewer-copy-status]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap()
    }

    #[wasm_bindgen_test]
    async fn a_preview_cannot_write_to_the_clipboard() {
        let server = FakeServer::answering(|_| Answer::Hang);
        let clipboard = ClipboardWrite::record();
        let loader = loader();
        loader.seed(9711, Variant::Preview, png(2, 1));
        let (root, viewer) = mount_viewer(&[(9711, true)], &loader);
        TimeoutFuture::new(60).await;

        let button = copy_button(&root);
        assert!(button.has_attribute("disabled"));
        assert!(root
            .text_content()
            .unwrap()
            .contains(t("Full image unavailable")));
        button.click();
        TimeoutFuture::new(50).await;
        assert!(clipboard.payloads.borrow().is_empty());
        assert_eq!(
            server.lines(&["/attachments/9711"]),
            ["GET /attachments/9711"]
        );
        viewer.destroy();
        root.remove();
    }

    #[wasm_bindgen_test]
    async fn an_evicted_original_cannot_write_or_restart_the_download() {
        let server = FakeServer::answering(|_| Answer::Hang);
        let clipboard = ClipboardWrite::record();
        let loader = loader();
        loader.seed(9712, Variant::Original, png(4, 3));
        let (root, viewer) = mount_viewer(&[(9712, false)], &loader);
        TimeoutFuture::new(60).await;

        let button = copy_button(&root);
        assert!(!button.has_attribute("disabled"));
        loader.clear();
        button.click();
        TimeoutFuture::new(60).await;
        assert!(copy_button(&root).has_attribute("disabled"));
        assert!(root
            .text_content()
            .unwrap()
            .contains(t("Full image unavailable")));
        assert!(clipboard.payloads.borrow().is_empty());
        assert!(server.lines(&["/attachments/9712"]).is_empty());
        viewer.destroy();
        root.remove();
    }

    #[wasm_bindgen_test]
    async fn copy_captures_the_current_album_photo_until_the_payload_is_read() {
        let server = FakeServer::answering(|_| Answer::Hang);
        let clipboard = ClipboardWrite::record();
        let loader = loader();
        loader.seed(9713, Variant::Original, png(4, 3));
        loader.seed(9714, Variant::Original, png(9, 5));
        let (root, viewer) = mount_viewer(&[(9713, false), (9714, false)], &loader);
        TimeoutFuture::new(60).await;
        root.query_selector(".viewer-arrow.is-next")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        TimeoutFuture::new(60).await;

        let button = copy_button(&root);
        assert!(!button.has_attribute("disabled"));
        button.click();
        assert_eq!(
            clipboard.payloads.borrow().len(),
            1,
            "write starts in the click"
        );
        let payload = clipboard.payloads.borrow()[0].clone();
        viewer.destroy();
        root.remove();
        loader.clear();

        let blob: Blob = JsFuture::from(payload).await.unwrap().dyn_into().unwrap();
        assert_eq!(blob.type_(), "image/png");
        let bitmap: ImageBitmap = JsFuture::from(
            web_sys::window()
                .unwrap()
                .create_image_bitmap_with_blob(&blob)
                .unwrap(),
        )
        .await
        .unwrap()
        .dyn_into()
        .unwrap();
        assert_eq!((bitmap.width(), bitmap.height()), (9, 5));
        let canvas: HtmlCanvasElement = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        canvas.set_width(bitmap.width());
        canvas.set_height(bitmap.height());
        let context: CanvasRenderingContext2d = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        context
            .draw_image_with_image_bitmap(&bitmap, 0.0, 0.0)
            .unwrap();
        assert_eq!(
            context.get_image_data(0.0, 0.0, 1.0, 1.0).unwrap().data().0[3],
            0
        );
        assert_eq!(
            context.get_image_data(1.0, 1.0, 1.0, 1.0).unwrap().data().0[3],
            255
        );
        bitmap.close();
        assert!(server
            .lines(&["/attachments/9713", "/attachments/9714"])
            .is_empty());
    }

    #[wasm_bindgen_test]
    fn zoom_goes_from_one_to_six_and_no_further() {
        assert_eq!(zoom_step(1.0, -1.0), 1.0, "never smaller than the window");
        assert!((zoom_step(1.0, 1.0) - 1.25).abs() < 1e-9);
        assert_eq!(zoom_step(5.5, 1.0), MAX_ZOOM);
        assert_eq!(zoom_step(MAX_ZOOM, 1.0), MAX_ZOOM);
        assert!((zoom_step(2.0, -1.0) - 1.6).abs() < 1e-9);
    }
}
