//! Rich content through WebKit (shell-design §9, P2): formulas, Mermaid and Graphviz diagrams and
//! `plot` graphs in a bubble, drawn as pictures by the same `web/rich` folder every native client
//! ships (copied to `linux/data/rich`, installed as `/usr/share/family-connect/rich`).
//!
//! | Module | What |
//! | --- | --- |
//! | [`renderer`] | `RichRenderer`: one hidden view, one job at a time, the photograph, the two caches |
//! | [`scheme`] | the `fc-rich` URI scheme serving the folder, contained |
//! | [`sandbox_probe`] | the bubblewrap preflight that keeps a missing AppArmor profile from killing the app |
//!
//! [`block`] and [`inline`] are what `ConversationSlots::rich` / `rich_inline` answer; [`install`]
//! makes the renderer and registers its window part: the view's place in the main window's overlay
//! and the banner (under the conversation's header) that says once when formulas and diagrams
//! cannot be drawn on this machine.

pub mod renderer;
pub mod sandbox_probe;
pub mod scheme;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::bubble_body::BodyRichBlock;
use fc_logic::rich_drawing::{RichJob, RichLook};

use crate::seams::conversation::RichPicture;
use crate::seams::routes::WindowPart;
use crate::shell::Shell;
use crate::windows::main_window::MainWindow;
use renderer::RichRenderer;

thread_local! {
    static RENDERER: RefCell<Option<Rc<RichRenderer>>> = const { RefCell::new(None) };
}

/// Makes the process's renderer and registers its window part.
pub fn install(shell: &Rc<Shell>) {
    let renderer = RichRenderer::new(shell.paths.rich_dir(), shell.paths.rich_cache_dir());
    RENDERER.with(|held| *held.borrow_mut() = Some(Rc::clone(&renderer)));
    shell.add_window_part(Rc::new(RendererPart));
}

/// The process's renderer (after [`install`]).
pub fn renderer() -> Option<Rc<RichRenderer>> {
    RENDERER.with(|held| held.borrow().clone())
}

/// A formula, diagram or graph standing on its own (`ConversationSlots::rich`).
pub fn block(block: &BodyRichBlock, look: &RichLook) -> RichPicture {
    let Some(renderer) = renderer() else {
        return RichPicture::source(&block.shown);
    };
    let job = RichJob::for_block(block.kind, &block.engine, &block.source, look);
    renderer.picture(job, &block.shown)
}

/// A formula inside a line of text (`ConversationSlots::rich_inline`): `typed` until drawn.
pub fn inline(source: &str, typed: &str, look: &RichLook) -> RichPicture {
    let Some(renderer) = renderer() else {
        return RichPicture::source(typed);
    };
    renderer.picture(RichJob::for_inline(source, look), typed)
}

/// The renderer's place in the main window, and the sandbox banner.
struct RendererPart;

impl WindowPart for RendererPart {
    fn attach(&self, window: &Rc<MainWindow>) {
        let Some(renderer) = renderer() else {
            return;
        };
        renderer.set_host(window.overlay());
        let banner = sandbox_banner();
        // Under the conversation's header: the content pane's own toolbar view (libadwaita's place
        // for a banner), where the blocks it is about are.
        if let Some(view) = window
            .content_header()
            .parent()
            .and_downcast::<adw::ToolbarView>()
        {
            view.add_top_bar(&banner);
        }
        let weak = banner.downgrade();
        renderer.sandbox_failed.subscribe(move |_| {
            if let Some(banner) = weak.upgrade() {
                banner.set_revealed(true);
            }
        });
    }
}

/// "Formulas and diagrams are shown as text…", hidden until the preflight fails; OK puts it away.
pub fn sandbox_banner() -> adw::Banner {
    let shell = Shell::get();
    let say = &shell.say;
    let banner = adw::Banner::builder()
        .title(say.get(
            "Formulas and diagrams are shown as text because this system won't start their sandbox. Family Connect's AppArmor profile (/etc/apparmor.d/family-connect) allows it.",
        ))
        .button_label(say.get("OK"))
        .revealed(false)
        .build();
    banner.connect_button_clicked(|banner| banner.set_revealed(false));
    banner
}
