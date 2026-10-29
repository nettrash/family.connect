//! `RichRenderer` (shell-design §9; Windows' `RichRenderer` the model, md's off-screen renderer the
//! mechanism): formulas (KaTeX), Mermaid, Graphviz and `plot` blocks drawn by the shipped
//! `fc-rich.js` in ONE hidden, mapped `webkit::WebView`, photographed, and kept.
//!
//! **The page and its origin** (§9.1): one `WebContext`, one ephemeral `NetworkSession` (nothing
//! of the web is written anywhere), the `fc-rich` scheme ([`super::scheme`]); the view may show
//! `fc-rich://rich/rich.html` and nothing else (`RichPage::is_page`, exact) — every other navigation
//! and every new window is ignored, every permission denied; JavaScript on, every storage and device
//! API off, developer extras in debug builds only.
//!
//! **One job at a time** (§9.2): `fcRich.drawAndPost(id, job)` is evaluated; the answer comes back
//! through the `fcRich` script message handler (`RichAnswer::parse`); after two animation frames the
//! view's visible region is photographed (`snapshot_future`, transparent background) at the
//! surface's scale and cropped to the rectangle the answer reported. The next job waits for the
//! photograph; FIFTEEN SECONDS without an answer fails the job and throws the web view away (an
//! engine stuck in a loop would fail every job after it). A drawing taller than the viewport makes
//! the view taller first.
//!
//! **Where the view lives** (§9.3): an overlay child of the main window's root `gtk::Overlay`,
//! 1280 × 800 (`RichPage`), at opacity 1/255 (md measured: at 0 GTK does not draw it and the page
//! gets no animation frames), not targetable, not focusable, start-aligned — it never moves the
//! window's minimum. While the main window is unmapped jobs wait.
//!
//! **Kept** (§9.4): in memory (`RichMemory`: 256 drawings, 32 Mpx) and on disk
//! (`$XDG_CACHE_HOME/family-connect/rich`, `RichStore`: PNG + metrics) under `RichJob::key`, whose
//! version is `fcRich.version` read from the shipped `fc-rich.js` with the screen's scale folded in
//! (carry-forward, "Rich page") — so a drawing on disk is shown without WebKit ever starting, and a
//! move to a 2× screen draws afresh. A failure is kept in memory only (a newer engine gets another
//! try).
//!
//! **The sandbox** (§9.5): before the first view, [`super::sandbox_probe`]. A failure leaves every
//! block as its source, says so once in a banner, and keeps the app running.
//!
//! Logs carry job ids, kinds, sizes and timings — never a source, never the engine's words (KaTeX
//! quotes the source it choked on).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, SystemTime};

use adw::prelude::*;
use fc_core::rt::Event;
use fc_logic::composer_row::conversation_column;
use fc_logic::rich_drawing::{
    RichAnswer, RichCapture, RichFit, RichFolder, RichJob, RichMemory, RichPage, RichResult,
    RichStore,
};
use futures_channel::oneshot;
use gtk::gdk;
use webkit::prelude::*;

use super::sandbox_probe::{self, Preflight};
use crate::seams::conversation::{RichDrawn, RichPicture};

/// The view's opacity: the least GTK draws (md measured 0 starves the page of frames).
pub const OPACITY: f64 = 1.0 / 255.0;
/// Drawings kept in memory.
pub const MEMORY_ENTRIES: usize = 256;
/// Their pixels between them.
pub const MEMORY_PIXELS: u64 = 32_000_000;
/// The widest a drawing is shown in a bubble: the bubble's widest less its padding.
pub const MOST_WIDTH: f64 = conversation_column::BUBBLE_MAX - 24.0;
/// How long the page may take to load.
const LOAD_LIMIT: Duration = Duration::from_secs(30);
/// How long GTK may take to give the view a new size.
const ALLOCATION_LIMIT: Duration = Duration::from_secs(3);
/// How long the two-frame wait may take before the photograph is taken anyway.
const FRAMES_LIMIT: Duration = Duration::from_millis(500);
/// How long one photograph may take.
const SNAPSHOT_LIMIT: Duration = Duration::from_secs(10);

/// The page's warm-up, once per view: KaTeX's engine and EVERY one of its fonts loaded before the
/// first formula is measured. `fc-rich.js` measures a formula as soon as KaTeX has laid it out, and
/// a font still loading then is measured as the fallback — a first formula came out cut off on the
/// right (measured under Xvfb). The shared script is not changed: the shell warms the page instead.
const WARM_UP: &str = "await fcRich.prepare('math'); \
     await Promise.all(Array.from(document.fonts, face => face.load().catch(() => null))); \
     return true;";
/// How long the warm-up may take before the first job goes anyway.
const WARM_LIMIT: Duration = Duration::from_secs(10);

/// The two-frame wait, so a drawing has been painted before it is photographed.
const TWO_FRAMES: &str =
    "await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));";

/// A drawing, or that it could not be drawn.
#[derive(Clone)]
enum Drawing {
    Drawn(gdk::Texture, RichResult),
    Failed,
}

/// One bubble waiting for a drawing.
struct Waiter {
    stack: glib::WeakRef<gtk::Stack>,
    picture: glib::WeakRef<gtk::Picture>,
    drawn: Rc<Cell<Option<RichDrawn>>>,
}

/// How the renderer has fared (the self-test's questions).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Jobs given to the page and not yet answered.
    pub in_flight: u32,
    /// The most there ever were at once.
    pub most_in_flight: u32,
    pub drawn: u32,
    pub failed: u32,
    /// Web views made (a timeout throws one away).
    pub views: u32,
}

/// The process's one renderer.
pub struct RichRenderer {
    me: Weak<RichRenderer>,
    rich_dir: PathBuf,
    /// `fcRich.version` from the shipped script; `None`: nothing is drawn.
    version: Option<String>,
    host: RefCell<Option<glib::WeakRef<gtk::Overlay>>>,
    view: RefCell<Option<webkit::WebView>>,
    sandbox: RefCell<Option<Preflight>>,
    probing: Cell<bool>,
    queue: RefCell<VecDeque<(String, RichJob)>>,
    waiters: RefCell<HashMap<String, Vec<Waiter>>>,
    busy: Cell<bool>,
    next_id: Cell<i32>,
    answer: RefCell<Option<(i32, oneshot::Sender<RichResult>)>>,
    memory: RefCell<RichMemory<Drawing>>,
    store: RichStore<FolderOnDisk>,
    stats: Cell<Stats>,
    /// Keys in the order their jobs finished (the self-test's "one at a time, in order").
    finished: RefCell<Vec<String>>,
    /// Why the last job failed, in this module's own words (never the engine's).
    last_failure: RefCell<Option<String>>,
    /// Raised when the sandbox preflight failed (the banner listens).
    pub sandbox_failed: Event<()>,
}

impl RichRenderer {
    /// The renderer over the shipped folder `rich_dir`, keeping drawings in `cache_dir`.
    pub fn new(rich_dir: PathBuf, cache_dir: PathBuf) -> Rc<RichRenderer> {
        let version = std::fs::read_to_string(rich_dir.join("fc-rich.js"))
            .ok()
            .and_then(|script| RichPage::version_of(&script).map(str::to_string));
        if version.is_none() {
            crate::fc_warn!(
                "rich: fc-rich.js is missing or names no version; blocks show their source"
            );
        }
        Rc::new_cyclic(|me| RichRenderer {
            me: me.clone(),
            rich_dir,
            version,
            host: RefCell::new(None),
            view: RefCell::new(None),
            sandbox: RefCell::new(None),
            probing: Cell::new(false),
            queue: RefCell::new(VecDeque::new()),
            waiters: RefCell::new(HashMap::new()),
            busy: Cell::new(false),
            next_id: Cell::new(0),
            answer: RefCell::new(None),
            memory: RefCell::new(RichMemory::with_budget(MEMORY_ENTRIES, MEMORY_PIXELS)),
            store: RichStore::new(FolderOnDisk(cache_dir)),
            stats: Cell::new(Stats::default()),
            finished: RefCell::new(Vec::new()),
            last_failure: RefCell::new(None),
            sandbox_failed: Event::new(),
        })
    }

    /// Where the view lives: the main window's root overlay. Jobs waiting for it start once it is
    /// mapped.
    pub fn set_host(&self, overlay: &gtk::Overlay) {
        *self.host.borrow_mut() = Some(overlay.downgrade());
        let me = self.me.clone();
        overlay.connect_map(move |_| {
            if let Some(me) = me.upgrade() {
                me.pump();
            }
        });
        // An old drawing folder is trimmed once, after start-up.
        let me = self.me.clone();
        glib::timeout_add_local_once(Duration::from_secs(5), move || {
            if let Some(me) = me.upgrade() {
                me.store.trim();
            }
        });
    }

    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the rich self-test reads it")
    )]
    pub fn stats(&self) -> Stats {
        self.stats.get()
    }

    /// Why the last job failed (the self-test's detail), in this module's own words.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the rich self-test reads it")
    )]
    pub fn last_failure(&self) -> Option<String> {
        self.last_failure.borrow().clone()
    }

    /// The keys of finished jobs, in the order they finished.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the rich self-test reads it")
    )]
    pub fn finished(&self) -> Vec<String> {
        self.finished.borrow().clone()
    }

    /// The sandbox preflight's answer, once it ran.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the rich self-test reads it")
    )]
    pub fn preflight(&self) -> Option<Preflight> {
        self.sandbox.borrow().clone()
    }

    /// The key `job` is kept under for a host at `scale` — `None` when nothing can be drawn.
    pub fn key_for(&self, job: &RichJob, scale: i32) -> Option<String> {
        let version = self.version.as_deref()?;
        Some(job.key(&format!("{version};scale={scale}")))
    }

    fn scale(&self) -> i32 {
        self.host
            .borrow()
            .as_ref()
            .and_then(|weak| weak.upgrade())
            .map_or(1, |overlay| crate::theme::bitmap_scale(&overlay))
    }

    /// The bubble's picture of `job`: the drawing at once when it is kept, else `shown` in the
    /// monospaced face until the drawing lands — or for good when it cannot be drawn.
    pub fn picture(&self, job: RichJob, shown: &str) -> RichPicture {
        let failed = self
            .sandbox
            .borrow()
            .as_ref()
            .is_some_and(|preflight| !preflight.allows_webkit());
        let Some(key) = self.key_for(&job, self.scale()).filter(|_| !failed) else {
            return RichPicture::source(shown);
        };
        let kept = self.memory.borrow_mut().try_get(&key).cloned();
        match kept {
            Some(Drawing::Drawn(texture, result)) => {
                return drawn_picture(shown, &texture, &result);
            }
            Some(Drawing::Failed) => return RichPicture::source(shown),
            None => {}
        }
        if let Some((png, result)) = self.store.read(&key)
            && let Ok(texture) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(png))
        {
            self.keep(&key, Drawing::Drawn(texture.clone(), result.clone()));
            return drawn_picture(shown, &texture, &result);
        }
        let (stack, picture) = pending_widget(shown);
        let drawn = Rc::new(Cell::new(None));
        let waiter = Waiter {
            stack: stack.downgrade(),
            picture: picture.downgrade(),
            drawn: Rc::clone(&drawn),
        };
        let first = {
            let mut waiters = self.waiters.borrow_mut();
            let list = waiters.entry(key.clone()).or_default();
            list.push(waiter);
            list.len() == 1
        };
        if first {
            self.queue.borrow_mut().push_back((key, job));
            let me = self.me.clone();
            glib::idle_add_local_once(move || {
                if let Some(me) = me.upgrade() {
                    me.pump();
                }
            });
        }
        RichPicture {
            widget: stack.upcast(),
            drawn,
        }
    }

    fn keep(&self, key: &str, drawing: Drawing) {
        let weight = match &drawing {
            Drawing::Drawn(texture, _) => {
                u64::try_from(texture.width()).unwrap_or(0)
                    * u64::try_from(texture.height()).unwrap_or(0)
            }
            Drawing::Failed => 1,
        };
        self.memory.borrow_mut().put_weighing(key, drawing, weight);
    }

    /// Starts the next job if none runs.
    fn pump(&self) {
        if self.busy.get() || self.queue.borrow().is_empty() {
            return;
        }
        let Some(me) = self.me.upgrade() else {
            return;
        };
        match self.sandbox.borrow().clone() {
            None => {
                if !self.probing.replace(true) {
                    glib::spawn_future_local(async move {
                        let preflight = sandbox_probe::run().await;
                        match &preflight {
                            Preflight::Failed(why) => {
                                crate::fc_warn!("rich: the WebKit sandbox cannot start: {why}")
                            }
                            Preflight::Skipped(why) => {
                                crate::fc_info!("rich: preflight skipped: {why}")
                            }
                            Preflight::Passed => crate::fc_info!("rich: preflight passed"),
                        }
                        let failed = !preflight.allows_webkit();
                        *me.sandbox.borrow_mut() = Some(preflight);
                        if failed {
                            me.fail_everything();
                            me.sandbox_failed.emit(&());
                        } else {
                            me.pump();
                        }
                    });
                }
                return;
            }
            Some(preflight) if !preflight.allows_webkit() => {
                self.fail_everything();
                return;
            }
            Some(_) => {}
        }
        let host = self.host.borrow().as_ref().and_then(|weak| weak.upgrade());
        let Some(host) = host.filter(|overlay| overlay.is_mapped()) else {
            // Nothing is shown anywhere: nothing needs drawing yet (§9.3).
            return;
        };
        self.busy.set(true);
        glib::spawn_future_local(async move {
            me.run_next(&host).await;
            me.busy.set(false);
            me.pump();
        });
    }

    /// Every waiting bubble keeps its source for good.
    fn fail_everything(&self) {
        let keys: Vec<String> = self
            .queue
            .borrow_mut()
            .drain(..)
            .map(|(key, _)| key)
            .collect();
        for key in keys {
            self.land(&key, Drawing::Failed, false);
        }
    }

    async fn run_next(&self, host: &gtk::Overlay) {
        let Some((key, job)) = self.queue.borrow_mut().pop_front() else {
            return;
        };
        let drawing = match self.draw(host, &job).await {
            Ok((texture, result)) => {
                let png = texture.save_to_png_bytes();
                self.store.write(&key, &png, &result);
                Drawing::Drawn(texture, result)
            }
            Err(why) => {
                crate::fc_info!("rich: a {} block was not drawn ({why})", job.kind);
                *self.last_failure.borrow_mut() = Some(why);
                Drawing::Failed
            }
        };
        self.land(&key, drawing, true);
    }

    /// Every bubble waiting on `key` gets `drawing`.
    fn land(&self, key: &str, drawing: Drawing, count: bool) {
        self.keep(key, drawing.clone());
        let mut stats = self.stats.get();
        match &drawing {
            Drawing::Drawn(..) => stats.drawn += u32::from(count),
            Drawing::Failed => stats.failed += u32::from(count),
        }
        self.stats.set(stats);
        self.finished.borrow_mut().push(key.to_string());
        let waiters = self.waiters.borrow_mut().remove(key).unwrap_or_default();
        for waiter in waiters {
            let outcome = match &drawing {
                Drawing::Drawn(texture, result) => {
                    let (width, height, baseline) = shown_size(result);
                    if let (Some(stack), Some(picture)) =
                        (waiter.stack.upgrade(), waiter.picture.upgrade())
                    {
                        picture.set_paintable(Some(&crate::attachments::pictures::fit(
                            texture, width, height,
                        )));
                        picture.set_size_request(
                            crate::attachments::pictures::logical(width),
                            crate::attachments::pictures::logical(height),
                        );
                        stack.set_visible_child_name("drawing");
                    }
                    RichDrawn::Drawn {
                        width,
                        height,
                        baseline,
                    }
                }
                Drawing::Failed => RichDrawn::Failed,
            };
            waiter.drawn.set(Some(outcome));
        }
    }

    /// One job through the page: the answer, the photograph, the crop.
    async fn draw(
        &self,
        host: &gtk::Overlay,
        job: &RichJob,
    ) -> Result<(gdk::Texture, RichResult), String> {
        let view = self.view_in(host).await?;
        let id = self.next_id.get().wrapping_add(1);
        self.next_id.set(id);
        let (sender, receiver) = oneshot::channel();
        *self.answer.borrow_mut() = Some((id, sender));
        let mut stats = self.stats.get();
        stats.in_flight += 1;
        stats.most_in_flight = stats.most_in_flight.max(stats.in_flight);
        self.stats.set(stats);
        let started = std::time::Instant::now();
        let evaluated = view
            // `void`: the call answers a promise, which is not a value WebKit hands back — the
            // drawing's answer comes through the script message instead.
            .evaluate_javascript_future(&format!("void {}", job.script(id)), None, None)
            .await;
        let result = match evaluated {
            Err(_) => Err("the script was refused".to_string()),
            Ok(_) => match glib::future_with_timeout(RichPage::TIMEOUT, receiver).await {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(_)) => Err("the page went away".to_string()),
                Err(_) => {
                    // An engine stuck in a loop would fail every job after it: a fresh view.
                    self.throw_away_view();
                    Err(format!("no answer in {} s", RichPage::TIMEOUT.as_secs()))
                }
            },
        };
        self.answer.borrow_mut().take();
        let mut stats = self.stats.get();
        stats.in_flight = stats.in_flight.saturating_sub(1);
        self.stats.set(stats);
        let result = result?;
        if !result.ok {
            return Err("the engine refused it".to_string());
        }
        let texture = self.photograph(&view, &result).await?;
        crate::fc_info!(
            "rich: a {} block drawn, {} × {} px, in {} ms",
            job.kind,
            texture.width(),
            texture.height(),
            started.elapsed().as_millis()
        );
        Ok((texture, result))
    }

    /// The view, made (and its page loaded) when there is none.
    async fn view_in(&self, host: &gtk::Overlay) -> Result<webkit::WebView, String> {
        let held = self.view.borrow().clone();
        if let Some(view) = held {
            if view.parent().as_ref() != Some(host.upcast_ref()) {
                if let Some(old) = view.parent().and_downcast::<gtk::Overlay>() {
                    old.remove_overlay(&view);
                }
                host.add_overlay(&view);
            }
            return Ok(view);
        }
        let view = self.make_view();
        host.add_overlay(&view);
        let (loaded, finished) = oneshot::channel::<bool>();
        let loaded = RefCell::new(Some(loaded));
        let failed = Rc::new(Cell::new(false));
        let failed_flag = Rc::clone(&failed);
        let load_failed = view.connect_load_failed(move |_, _, _, _| {
            failed_flag.set(true);
            false
        });
        let changed = view.connect_load_changed(move |_, event| {
            if event == webkit::LoadEvent::Finished
                && let Some(sender) = loaded.borrow_mut().take()
            {
                let _ = sender.send(true);
            }
        });
        view.load_uri(RichPage::URL);
        let outcome = glib::future_with_timeout(LOAD_LIMIT, finished).await;
        view.disconnect(changed);
        view.disconnect(load_failed);
        match outcome {
            Ok(Ok(true)) if !failed.get() => {
                let warming = view.call_async_javascript_function_future(WARM_UP, None, None, None);
                if !matches!(
                    glib::future_with_timeout(WARM_LIMIT, warming).await,
                    Ok(Ok(_))
                ) {
                    crate::fc_info!("rich: the page's fonts were not all warmed");
                }
                *self.view.borrow_mut() = Some(view.clone());
                let mut stats = self.stats.get();
                stats.views += 1;
                self.stats.set(stats);
                Ok(view)
            }
            _ => {
                host.remove_overlay(&view);
                Err("the page did not load".to_string())
            }
        }
    }

    fn make_view(&self) -> webkit::WebView {
        let context = webkit::WebContext::default().unwrap_or_default();
        super::scheme::register(&context, self.rich_dir.clone());
        let manager = webkit::UserContentManager::new();
        manager.register_script_message_handler("fcRich", None);
        let me = self.me.clone();
        manager.connect_script_message_received(Some("fcRich"), move |_, value| {
            let Some(me) = me.upgrade() else {
                return;
            };
            let json = value.to_str();
            let Some((id, result)) = RichAnswer::parse(json.as_str()) else {
                return;
            };
            let mut answer = me.answer.borrow_mut();
            if answer.as_ref().is_some_and(|(waiting, _)| *waiting == id)
                && let Some((_, sender)) = answer.take()
            {
                let _ = sender.send(result);
            }
        });
        let view = webkit::WebView::builder()
            .web_context(&context)
            .network_session(&network_session())
            .user_content_manager(&manager)
            .settings(&settings())
            .build();
        view.set_background_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
        view.set_opacity(OPACITY);
        view.set_can_target(false);
        view.set_can_focus(false);
        view.set_focusable(false);
        view.set_halign(gtk::Align::Start);
        view.set_valign(gtk::Align::Start);
        view.set_size_request(RichPage::VIEWPORT_WIDTH, RichPage::VIEWPORT_HEIGHT);
        view.connect_decide_policy(|_, decision, kind| {
            if kind == webkit::PolicyDecisionType::Response {
                return false;
            }
            let uri = decision
                .downcast_ref::<webkit::NavigationPolicyDecision>()
                .and_then(|navigation| navigation.navigation_action())
                .and_then(|action| action.request())
                .and_then(|request| request.uri())
                .map(|uri| uri.to_string());
            if kind == webkit::PolicyDecisionType::NavigationAction
                && RichPage::is_page(uri.as_deref())
            {
                decision.use_();
            } else {
                decision.ignore();
            }
            true
        });
        view.connect_create(|_, _| None);
        view.connect_permission_request(|_, request| {
            request.deny();
            true
        });
        let me = self.me.clone();
        view.connect_web_process_terminated(move |_, reason| {
            crate::fc_warn!("rich: the web process ended ({reason:?})");
            if let Some(me) = me.upgrade() {
                me.throw_away_view();
                if let Some((_, sender)) = me.answer.borrow_mut().take() {
                    drop(sender);
                }
            }
        });
        view
    }

    fn throw_away_view(&self) {
        if let Some(view) = self.view.borrow_mut().take() {
            view.stop_loading();
            if let Some(overlay) = view.parent().and_downcast::<gtk::Overlay>() {
                overlay.remove_overlay(&view);
            }
        }
    }

    /// The drawing at the page's top left, photographed at the view's scale and cropped.
    async fn photograph(
        &self,
        view: &webkit::WebView,
        result: &RichResult,
    ) -> Result<gdk::Texture, String> {
        // Taller than the viewport: make the view taller first, so all of it is visible.
        let needed = (result.height.ceil() as i32).max(RichPage::VIEWPORT_HEIGHT);
        if view.height() < needed || view.height_request() < needed {
            view.set_size_request(RichPage::VIEWPORT_WIDTH, needed);
            let start = std::time::Instant::now();
            while view.height() < needed && start.elapsed() < ALLOCATION_LIMIT {
                glib::timeout_future(Duration::from_millis(16)).await;
            }
        }
        let frames = view.call_async_javascript_function_future(TWO_FRAMES, None, None, None);
        let _ = glib::future_with_timeout(FRAMES_LIMIT, frames).await;
        let texture = glib::future_with_timeout(
            SNAPSHOT_LIMIT,
            view.snapshot_future(
                webkit::SnapshotRegion::Visible,
                webkit::SnapshotOptions::TRANSPARENT_BACKGROUND,
            ),
        )
        .await
        .map_err(|_| "no photograph in time".to_string())?
        .map_err(|_| "the photograph failed".to_string())?;
        let css_width = f64::from(view.width().max(1));
        let factor = f64::from(texture.width()) / css_width;
        let (width, height) = (
            ((result.width * factor).ceil() as i32).clamp(1, texture.width()),
            ((result.height * factor).ceil() as i32).clamp(1, texture.height()),
        );
        // The longest side is capped (`RichCapture::LONGEST_SIDE`): past it the drawing is kept at
        // the cap's scale.
        let scale = RichCapture::scale_for(factor, result.width, result.height);
        let cropped = crop(&texture, width, height);
        if scale + f64::EPSILON < factor {
            let (w, h) = (
                ((result.width * scale).round() as i32).max(1),
                ((result.height * scale).round() as i32).max(1),
            );
            return Ok(scaled(&cropped, w, h));
        }
        Ok(cropped)
    }
}

/// The size a drawing is shown at in a bubble: its CSS size, scaled down whole (baseline with it)
/// when it is wider than the bubble allows (`RichFit::inline`).
pub fn shown_size(result: &RichResult) -> (f64, f64, f64) {
    let (width, height, drop) =
        RichFit::inline(result.width, result.height, result.baseline, MOST_WIDTH);
    (width, height, height - drop)
}

/// The waiting widget: the source in the monospaced face, and the empty picture it becomes.
fn pending_widget(shown: &str) -> (gtk::Stack, gtk::Picture) {
    let source = gtk::Label::builder()
        .label(shown)
        .selectable(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .css_classes(["monospace"])
        .build();
    let picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Fill)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .build();
    let stack = gtk::Stack::builder()
        .hhomogeneous(false)
        .vhomogeneous(false)
        .halign(gtk::Align::Start)
        .build();
    stack.add_named(&source, Some("source"));
    stack.add_named(&picture, Some("drawing"));
    stack.set_visible_child_name("source");
    (stack, picture)
}

/// A drawing already kept: its picture at once, already landed.
fn drawn_picture(shown: &str, texture: &gdk::Texture, result: &RichResult) -> RichPicture {
    let (stack, picture) = pending_widget(shown);
    let (width, height, baseline) = shown_size(result);
    picture.set_paintable(Some(&crate::attachments::pictures::fit(
        texture, width, height,
    )));
    picture.set_size_request(
        crate::attachments::pictures::logical(width),
        crate::attachments::pictures::logical(height),
    );
    stack.set_visible_child_name("drawing");
    RichPicture {
        widget: stack.upcast(),
        drawn: Rc::new(Cell::new(Some(RichDrawn::Drawn {
            width,
            height,
            baseline,
        }))),
    }
}

/// `width` × `height` from the top left of `texture`, as a new memory texture.
fn crop(texture: &gdk::Texture, width: i32, height: i32) -> gdk::Texture {
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8Premultiplied);
    let (bytes, stride) = downloader.download_bytes();
    let row = usize::try_from(width).unwrap_or(1) * 4;
    let mut out = Vec::with_capacity(row * usize::try_from(height).unwrap_or(1));
    for y in 0..usize::try_from(height).unwrap_or(0) {
        let start = y * stride;
        out.extend_from_slice(&bytes[start..start + row]);
    }
    gdk::MemoryTexture::new(
        width,
        height,
        gdk::MemoryFormat::R8g8b8a8Premultiplied,
        &glib::Bytes::from_owned(out),
        row,
    )
    .upcast()
}

/// `texture` redrawn at `width` × `height` (the longest-side cap), through gdk-pixbuf.
fn scaled(texture: &gdk::Texture, width: i32, height: i32) -> gdk::Texture {
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_bytes(
        &bytes,
        gtk::gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        texture.width(),
        texture.height(),
        i32::try_from(stride).unwrap_or(i32::MAX),
    );
    match pixbuf.scale_simple(width, height, gtk::gdk_pixbuf::InterpType::Hyper) {
        Some(smaller) => crate::attachments::pictures::texture_of(&smaller),
        None => texture.clone(),
    }
}

/// The one ephemeral network session every view shares (nothing of the web is written to disk).
fn network_session() -> webkit::NetworkSession {
    thread_local! {
        static SESSION: webkit::NetworkSession = webkit::NetworkSession::new_ephemeral();
    }
    SESSION.with(Clone::clone)
}

/// The view's settings (§9.1): JavaScript on; windows, clipboard, storage, databases, WebAudio,
/// WebRTC, media streams and full screen off; developer extras in debug builds only.
fn settings() -> webkit::Settings {
    let settings = webkit::Settings::new();
    settings.set_enable_javascript(true);
    settings.set_javascript_can_open_windows_automatically(false);
    settings.set_javascript_can_access_clipboard(false);
    settings.set_enable_back_forward_navigation_gestures(false);
    settings.set_enable_html5_local_storage(false);
    settings.set_enable_html5_database(false);
    settings.set_enable_webaudio(false);
    settings.set_enable_webrtc(false);
    settings.set_enable_media_stream(false);
    settings.set_enable_fullscreen(false);
    settings.set_default_charset("utf-8");
    settings.set_enable_developer_extras(cfg!(debug_assertions));
    settings
}

/// `RichFolder` over `std::fs`, in `$XDG_CACHE_HOME/family-connect/rich` (fc-logic's table).
pub struct FolderOnDisk(PathBuf);

impl FolderOnDisk {
    fn at(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl RichFolder for FolderOnDisk {
    fn exists(&self) -> bool {
        std::fs::metadata(&self.0).is_ok_and(|m| m.is_dir())
    }

    fn create(&self) -> io::Result<()> {
        crate::services::files::private_dir(&self.0)
    }

    fn files(&self) -> io::Result<Vec<String>> {
        Ok(std::fs::read_dir(&self.0)?
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect())
    }

    fn is_file(&self, name: &str) -> bool {
        std::fs::metadata(self.at(name)).is_ok_and(|m| m.is_file())
    }

    fn read(&self, name: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.at(name))
    }

    fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(self.at(name), bytes)
    }

    fn replace(&self, from: &str, to: &str) -> io::Result<()> {
        std::fs::rename(self.at(from), self.at(to))
    }

    fn delete(&self, name: &str) -> io::Result<()> {
        match std::fs::remove_file(self.at(name)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    fn modified(&self, name: &str) -> io::Result<SystemTime> {
        std::fs::metadata(self.at(name))?.modified()
    }

    fn touch(&self, name: &str) -> io::Result<()> {
        std::fs::File::options()
            .write(true)
            .open(self.at(name))?
            .set_modified(SystemTime::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drawing_wider_than_a_bubble_is_shown_smaller_baseline_with_it() {
        let narrow = RichResult::drawn(120.0, 40.0, 30.0);
        assert_eq!(shown_size(&narrow), (120.0, 40.0, 30.0));
        let wide = RichResult::drawn(MOST_WIDTH * 2.0, 100.0, 80.0);
        let (width, height, baseline) = shown_size(&wide);
        assert!((width - MOST_WIDTH).abs() < 1e-9);
        assert!((height - 50.0).abs() < 1e-9);
        assert!((baseline - 40.0).abs() < 1e-9);
    }

    #[test]
    fn the_disk_folder_answers_as_fc_logic_expects() {
        let dir = std::env::temp_dir().join(format!("fc-rich-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = FolderOnDisk(dir.clone());
        assert!(!folder.exists());
        folder.create().unwrap();
        assert!(folder.exists());
        folder.write("a.part", b"x").unwrap();
        folder.replace("a.part", "a.png").unwrap();
        assert!(folder.is_file("a.png"));
        assert_eq!(folder.read("a.png").unwrap(), b"x");
        folder.touch("a.png").unwrap();
        assert!(folder.modified("a.png").is_ok());
        assert_eq!(folder.files().unwrap(), vec!["a.png".to_string()]);
        folder.delete("a.png").unwrap();
        folder.delete("a.png").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
