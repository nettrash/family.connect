//! The `rich` group (P2, shell-design §27, §9): every engine draws offline under the page's
//! content policy — a display formula (KaTeX), a Mermaid diagram, a Graphviz graph and a `plot` —
//! through the hidden web view; a block the engine refuses falls back to its source; three jobs
//! asked at once run one at a time, in order; a drawing asked again comes back at once from memory,
//! and is kept on disk; the scheme refuses anything outside its folder.
//!
//! The drawings are hosted in a small test window of this group's own (P1's bubbles are not in this
//! package), on a card and inside an own-balloon stand-in, and photographed.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_logic::bubble_body::BodyRichBlock;
use fc_logic::rich_drawing::RichLook;
use fc_text::markdown::RichKind;

use super::Harness;
use crate::rich;
use crate::seams::conversation::{ConversationSlots, RichDrawn, RichPicture};

/// How long a drawing may take (a cold WebKit and Mermaid's 3.5 MB script under Xvfb).
const DRAW_LIMIT: Duration = Duration::from_secs(60);

pub async fn run(h: &Harness) {
    let shell = h.shell();
    let Some(renderer) = rich::renderer() else {
        h.check("renderer_installed", false, "no rich renderer");
        return;
    };
    let slots = shell.slots();
    let dark = crate::theme::is_dark();
    let ink = if dark { "#F2F2F7" } else { "#1C1C1E" };
    let look = RichLook::new(dark, ink, 15.0);
    let own_look = RichLook::new(dark, "#FFFFFF", 15.0);

    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(16)
        .margin_end(16)
        .margin_top(16)
        .margin_bottom(16)
        .build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&column)
        .build();
    let window = gtk::Window::builder()
        .application(&shell.app)
        .title("P2 rich")
        .default_width(900)
        .default_height(760)
        .child(&scrolled)
        .build();
    window.present();

    let place = |picture: &RichPicture, own: bool| {
        let card = gtk::Box::builder()
            .css_classes([if own { "fc-own-bubble" } else { "fc-bubble" }, "card"])
            .halign(gtk::Align::Start)
            .build();
        picture.widget.set_margin_start(10);
        picture.widget.set_margin_end(10);
        picture.widget.set_margin_top(8);
        picture.widget.set_margin_bottom(8);
        card.append(&picture.widget);
        column.append(&card);
    };

    // ---- every engine ----------------------------------------------------------------------
    let math = BodyRichBlock::new(
        RichKind::Math,
        "",
        r"e^{i\pi} + 1 = 0 \qquad \sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}",
    )
    .with_shown("$$e^{i\\pi} + 1 = 0 …$$");
    let mermaid = BodyRichBlock::new(
        RichKind::Mermaid,
        "",
        "graph LR\n  Mum --> Dinner\n  Dad --> Dinner\n  Dinner --> Washing-up",
    );
    let graphviz = BodyRichBlock::new(
        RichKind::Graphviz,
        "dot",
        "digraph { rankdir=LR; Grandma -> Mum; Mum -> Kid; Grandma -> Uncle }",
    );
    let plot = BodyRichBlock::new(RichKind::Plot, "", "x: -3..3\nsin(x)\ncos(x)");
    let pictures: Vec<(&str, RichPicture)> = vec![
        ("math", slots.rich(&math, &own_look)),
        ("mermaid", slots.rich(&mermaid, &look)),
        ("graphviz", slots.rich(&graphviz, &look)),
        ("plot", slots.rich(&plot, &look)),
    ];
    for (name, picture) in &pictures {
        place(picture, *name == "math");
    }
    for (name, picture) in &pictures {
        let drawn = wait_drawn(h, picture).await;
        let shown = picture
            .widget
            .downcast_ref::<gtk::Stack>()
            .and_then(|stack| stack.visible_child_name())
            .map(|n| n.to_string());
        h.check(
            &format!("{name}_drawn"),
            matches!(drawn, Some(RichDrawn::Drawn { width, height, .. }) if width > 4.0 && height > 4.0)
                && shown.as_deref() == Some("drawing"),
            format!(
                "{drawn:?}, showing {shown:?}; last failure {:?}",
                renderer.last_failure()
            ),
        );
    }

    // ---- a failure falls back to the source -------------------------------------------------
    let broken = BodyRichBlock::new(RichKind::Math, "", r"\frac{1}{").with_shown("$$\\frac{1}{$$");
    let failed = slots.rich(&broken, &look);
    place(&failed, false);
    let outcome = wait_drawn(h, &failed).await;
    let source_shown = failed
        .widget
        .downcast_ref::<gtk::Stack>()
        .and_then(|stack| stack.visible_child())
        .and_downcast::<gtk::Label>()
        .is_some_and(|label| label.label() == "$$\\frac{1}{$$");
    h.check(
        "failure_falls_back_to_source",
        outcome == Some(RichDrawn::Failed) && source_shown,
        format!("{outcome:?}, the source shown {source_shown}"),
    );

    // ---- one job at a time, in order ---------------------------------------------------------
    let before = renderer.finished().len();
    let inline: Vec<RichPicture> = ["x^{2}", "y_{1} + y_{2}", r"\sqrt{z}"]
        .iter()
        .map(|source| slots.rich_inline(source, &format!("${source}$"), &look))
        .collect();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    for picture in &inline {
        row.append(&picture.widget);
    }
    column.append(&row);
    let mut all = true;
    for picture in &inline {
        all &= matches!(wait_drawn(h, picture).await, Some(RichDrawn::Drawn { .. }));
    }
    let stats = renderer.stats();
    let finished = renderer.finished();
    let order_kept = finished.len() >= before + 3 && {
        let scale = crate::theme::bitmap_scale(&window);
        let keys: Vec<Option<String>> = ["x^{2}", "y_{1} + y_{2}", r"\sqrt{z}"]
            .iter()
            .map(|source| {
                renderer.key_for(
                    &fc_logic::rich_drawing::RichJob::for_inline(source, &look),
                    scale,
                )
            })
            .collect();
        keys.iter().flatten().count() == 3
            && finished[before..]
                .iter()
                .filter(|key| keys.iter().flatten().any(|k| k == *key))
                .eq(keys.iter().flatten())
    };
    h.check(
        "one_job_at_a_time",
        all && stats.most_in_flight == 1 && order_kept,
        format!(
            "drawn {all}; most in flight {}; finished in order {order_kept}; {stats:?}",
            stats.most_in_flight
        ),
    );

    // ---- kept: at once from memory, and on disk ----------------------------------------------
    let again = slots.rich(&math, &own_look);
    let at_once = matches!(again.drawn.get(), Some(RichDrawn::Drawn { .. }));
    let on_disk = std::fs::read_dir(shell.paths.rich_cache_dir())
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".png"))
                .count()
        })
        .unwrap_or(0);
    h.check(
        "drawing_kept_in_memory_and_on_disk",
        at_once && on_disk >= 4,
        format!("asked again: drawn at once {at_once}; {on_disk} drawings on disk"),
    );
    h.check(
        "sandbox_preflight_answered",
        renderer.preflight().is_some_and(|p| p.allows_webkit()),
        format!(
            "{:?} (Docker disables WebKit's sandbox, so it is skipped here)",
            renderer.preflight()
        ),
    );
    let refused = rich::scheme::answer(&shell.paths.rich_dir(), "fc-rich://rich/../i18n/de.json");
    let served = rich::scheme::answer(&shell.paths.rich_dir(), "fc-rich://rich/rich.html");
    h.check(
        "scheme_serves_only_its_folder",
        refused.status == 404 && served.status == 200 && served.csp,
        format!("../ → {}, rich.html → {}", refused.status, served.status),
    );
    h.screenshot("drawn").await;

    // The banner a failed preflight shows (revealed here by hand, then put away).
    {
        let banner = rich::sandbox_banner();
        let holder = gtk::Box::builder()
            .valign(gtk::Align::Start)
            .halign(gtk::Align::Fill)
            .build();
        holder.append(&banner);
        banner.set_hexpand(true);
        column.prepend(&holder);
        banner.set_revealed(true);
        h.sleep(Duration::from_millis(400)).await;
        scrolled.vadjustment().set_value(0.0);
        h.screenshot("sandbox-banner").await;
        h.check(
            "sandbox_banner_names_the_profile",
            banner.title().contains("/etc/apparmor.d/family-connect"),
            "the banner a failed preflight reveals",
        );
    }
    window.close();
}

/// Waits until `picture` landed (or failed) and answers what it became.
async fn wait_drawn(h: &Harness, picture: &RichPicture) -> Option<RichDrawn> {
    let drawn = Rc::clone(&picture.drawn);
    h.wait_until(DRAW_LIMIT, move || drawn.get().is_some())
        .await;
    picture.drawn.get()
}
