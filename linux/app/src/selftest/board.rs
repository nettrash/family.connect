//! The `board` group (P4, §27): the Board window opens on `app.open-board`, its notes laid out at
//! their fractions (the corner, clamped, tilted by id) on a wall taller than the window; a note is
//! dragged with real pointer events (XTEST) and the drop sends ONE move carrying only x/y; a focused
//! note moves with the arrow keys (D21); a note is written in the sheet and then rewritten (only
//! what changed is sent); an event is written as an `.ics` file for the calendar; a live note from
//! somebody else raises the Board button's badge, which showing the board clears. Screenshots:
//! the wall, the sheet, a note's menu, dark, narrow (compact cards).
//!
//! Birthdays: the Mac's board draws none (`MacBoardView`), so neither does this one — a birthday is
//! drawn where the Mac draws it, in the roster (the `family` group's `member_birthday`) and the
//! profile (the `settings` group's `birthday_row`).

use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::{NoteDto, TaskItemDto};
use fc_text::board::{Kind, Size, is_compact, origin};

use super::Harness;
use super::settings::{install, person, sign_in_as, world};
use crate::board::{sheet, wall};
use crate::windows::board_window;

const WAIT: Duration = Duration::from_secs(15);

pub async fn run(h: &Harness) {
    let shell = h.shell();
    let fake = h.fake_server();
    install(&fake);
    let lena = person(&fake, "lena", "Lena");
    let milo = person(&fake, "milo", "Milo");
    let family = fake.add_family("Lena's Family", lena);
    fake.join(milo, family, "member");
    let tomorrow = time::OffsetDateTime::now_utc() + time::Duration::days(1);
    let starts = format!(
        "{:04}-{:02}-{:02}T16:00:00Z",
        tomorrow.year(),
        u8::from(tomorrow.month()),
        tomorrow.day()
    );
    let (text_note, event_note) = {
        let world = world();
        let mut world = world.borrow_mut();
        let text = world.add_note(
            family,
            milo,
            NoteDto {
                text: Some("Dentist on Friday — who takes Ana?".into()),
                color: Some("pink".into()),
                size: Some("medium".into()),
                x: 0.08,
                y: 0.06,
                ..NoteDto::default()
            },
        );
        let event = world.add_note(
            family,
            lena,
            NoteDto {
                kind: Some("event".into()),
                text: Some("Gran's birthday dinner".into()),
                color: Some("blue".into()),
                size: Some("large".into()),
                starts_at: Some(starts.clone()),
                place: Some("Gran's".into()),
                x: 0.45,
                y: 0.08,
                ..NoteDto::default()
            },
        );
        world.add_note(
            family,
            lena,
            NoteDto {
                kind: Some("tasks".into()),
                text: Some("Shopping".into()),
                color: Some("green".into()),
                x: 0.1,
                y: 0.4,
                items: Some(vec![
                    TaskItemDto {
                        id: 1,
                        text: "Milk".into(),
                        done: true,
                        ..TaskItemDto::default()
                    },
                    TaskItemDto {
                        id: 2,
                        text: "Bread".into(),
                        ..TaskItemDto::default()
                    },
                ]),
                ..NoteDto::default()
            },
        );
        (text, event)
    };

    let Some(connection) = sign_in_as(h, "lena").await else {
        h.check("signed_in", false, "lena could not sign in");
        return;
    };
    let _ = connection.session.switch_family(family).await;
    connection.live.refresh();
    h.sleep(Duration::from_millis(800)).await;

    // ---- the window and the wall -------------------------------------------------------------------
    shell.app.activate_action(
        crate::seams::actions::OPEN_BOARD.name,
        Some(&family.to_variant()),
    );
    let laid = h
        .wait_until(WAIT, || {
            board_window::current().is_some_and(|board| board.wall.cards().len() == 3)
        })
        .await;
    let Some(board) = board_window::current() else {
        h.check("window_opens", false, "no Board window");
        return;
    };
    let window: gtk::Window = board.window.clone().upcast();
    h.check(
        "window_opens",
        board.window.is_visible()
            && board.window.default_size() == board_window::DEFAULT_SIZE
            && board.title.subtitle() == "Lena's Family",
        format!(
            "size {:?}, subtitle {:?}",
            board.window.default_size(),
            board.title.subtitle()
        ),
    );
    h.sleep(Duration::from_millis(400)).await;
    let wall_size = board.wall.wall_size();
    let mut misplaced = Vec::new();
    for (id, card, corner) in board.wall.cards() {
        let note = world()
            .borrow()
            .notes
            .iter()
            .find(|note| note.id == id)
            .cloned()
            .unwrap_or_default();
        // The card at the Mac's metrics for its named size (fc_text::board), and the card drawn.
        let size = Size::from_name(note.size.as_deref()).frame(is_compact(wall_size.0));
        // The card's own box (its border box, padding included) and its centre on the wall —
        // the centre of its bounds whatever the tilt.
        let drawn = card.compute_bounds(&card).map_or((0.0, 0.0), |b| {
            (f64::from(b.width()), f64::from(b.height()))
        });
        let expected = origin((note.x, note.y), size, wall_size);
        let centre = card.compute_bounds(&board.wall.fixed).map(|b| {
            (
                f64::from(b.x() + b.width() / 2.0),
                f64::from(b.y() + b.height() / 2.0),
            )
        });
        let ok = centre.is_some_and(|c| {
            (c.0 - (expected.0 + size.0 / 2.0)).abs() < 1.5
                && (c.1 - (expected.1 + size.1 / 2.0)).abs() < 1.5
        }) && (corner.0 - expected.0).abs() < 0.5
            && (drawn.0 - size.0).abs() < 1.0
            && (drawn.1 - size.1).abs() < 1.0;
        if !ok {
            misplaced.push(format!(
                "{id}: {corner:?} vs {expected:?}, card {size:?} drawn {drawn:?}, centre {:?}",
                centre
            ));
        }
    }
    h.check(
        "notes_laid_out",
        laid && misplaced.is_empty() && wall_size.1 > f64::from(board.wall.scroller.height()),
        format!("wall {wall_size:?}, misplaced {misplaced:?}"),
    );
    h.screenshot("wall").await;

    // ---- drag a note: one move, on release -----------------------------------------------------------
    let patches_before = count(&fake.requests(), "PATCH");
    let mut dragged = false;
    if h.input()
        && let Some(card) = board.wall.card_of(text_note.id)
    {
        h.focus(&window).await;
        let (w, hgt) = (f64::from(card.width()), f64::from(card.height()));
        if let Some((sx, sy)) = h.screen_point(&window, &card, w / 2.0, hgt / 2.0).await {
            let to = (sx + 160, sy + 90);
            let mut steps = vec![
                vec![
                    "mousemove".to_string(),
                    "--sync".into(),
                    sx.to_string(),
                    sy.to_string(),
                ],
                vec!["mousedown".into(), "1".into()],
            ];
            for step in 1..=8 {
                steps.push(vec![
                    "mousemove".into(),
                    (sx + (to.0 - sx) * step / 8).to_string(),
                    (sy + (to.1 - sy) * step / 8).to_string(),
                ]);
            }
            steps.push(vec!["mouseup".into(), "1".into()]);
            for step in steps {
                let args: Vec<&str> = step.iter().map(String::as_str).collect();
                h.xdotool(&args).await;
                h.sleep(Duration::from_millis(40)).await;
            }
            dragged = true;
        }
    }
    if !dragged {
        // Without input: the keyboard's way, which is the same hand.
        board.wall.nudge(text_note.id, 160.0, 90.0);
        board.wall.put_down(text_note.id);
    }
    let moved = h
        .wait_until(WAIT, || {
            world()
                .borrow()
                .notes
                .iter()
                .any(|note| note.id == text_note.id && (note.x - text_note.x).abs() > 0.05)
        })
        .await;
    h.sleep(Duration::from_millis(500)).await;
    let patches = count(&fake.requests(), "PATCH") - patches_before;
    let stored = world()
        .borrow()
        .notes
        .iter()
        .find(|note| note.id == text_note.id)
        .map(|note| (note.x, note.y));
    h.check(
        "drag_a_note",
        moved && patches == 1,
        format!("real pointer {dragged}, {patches} PATCH, stored {stored:?}"),
    );
    h.screenshot("dragged").await;

    // ---- the arrow keys move a focused note (D21) -----------------------------------------------------
    let before = stored.map_or(0.0, |(x, _)| x);
    let mut nudged = false;
    if h.input()
        && let Some(card) = board.wall.card_of(text_note.id)
    {
        h.focus(&window).await;
        card.grab_focus();
        h.key("Right").await;
        nudged = h
            .wait_until(WAIT, || {
                world().borrow().notes.iter().any(|note| {
                    note.id == text_note.id
                        && ((note.x - before) * wall_size.0 - wall::STEP).abs() < 1.0
                })
            })
            .await;
    }
    h.check(
        "arrow_keys_move_a_note",
        nudged || !h.input(),
        format!("one step of {} px", wall::STEP),
    );

    // ---- write a note in the sheet, then rewrite it ---------------------------------------------------
    board.compose(Kind::Text);
    h.sleep(Duration::from_millis(300)).await;
    let composed = sheet::current();
    if let Some(sheet) = &composed {
        sheet.text.buffer().set_text("Milk and bread");
        h.screenshot("sheet").await;
        sheet.save.emit_clicked();
    }
    let created = h
        .wait_until(WAIT, || {
            world()
                .borrow()
                .notes
                .iter()
                .any(|note| note.text.as_deref() == Some("Milk and bread"))
                && board.wall.cards().len() == 4
        })
        .await;
    h.check(
        "create_a_note",
        composed.is_some() && created,
        format!("{} cards", board.wall.cards().len()),
    );
    let new_note = world()
        .borrow()
        .notes
        .iter()
        .find(|note| note.text.as_deref() == Some("Milk and bread"))
        .cloned();
    let patches_before = count(&fake.requests(), "PATCH");
    if let Some(note) = new_note.clone() {
        sheet::edit(&board.window, note, || {});
        h.sleep(Duration::from_millis(300)).await;
        if let Some(sheet) = sheet::current() {
            sheet.text.buffer().set_text("Milk, bread and eggs");
            sheet.colors.set_active_name(Some("green"));
            sheet.save.emit_clicked();
        }
    }
    let rewritten = h
        .wait_until(WAIT, || {
            world().borrow().notes.iter().any(|note| {
                note.text.as_deref() == Some("Milk, bread and eggs")
                    && note.color.as_deref() == Some("green")
            })
        })
        .await;
    h.check(
        "edit_a_note",
        rewritten && count(&fake.requests(), "PATCH") - patches_before == 1,
        "text and colour in one PATCH",
    );

    // ---- an event on the calendar -----------------------------------------------------------------------
    let file = board.wall.add_to_calendar(&event_note);
    let ics = file
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    let dtstart = format!(
        "DTSTART:{:04}{:02}{:02}T160000Z\r\n",
        tomorrow.year(),
        u8::from(tomorrow.month()),
        tomorrow.day()
    );
    h.check(
        "event_on_the_calendar",
        ics.contains(&dtstart)
            && ics.contains(&format!("UID:fc-note-{}@family.connect", event_note.id))
            && ics.contains("SUMMARY:Gran's birthday dinner\r\n"),
        format!("{:?}", file.as_ref().and_then(|p| p.file_name())),
    );
    if let Some(card) = board.wall.card_of(event_note.id) {
        let (w, hgt) = (f64::from(card.width()), f64::from(card.height()));
        if let Some(point) = card.compute_point(
            &board.wall.fixed,
            &gtk::graphene::Point::new((w / 2.0) as f32, (hgt / 2.0) as f32),
        ) {
            board
                .wall
                .show_menu(event_note.id, f64::from(point.x()), f64::from(point.y()));
            h.screenshot("event-menu").await;
            if h.input() {
                h.key("Escape").await;
            }
        }
    }

    // ---- the badge: a live note from somebody else, cleared by showing the board ------------------------
    board.window.set_visible(false);
    let live = world().borrow_mut().add_note(
        family,
        milo,
        NoteDto {
            text: Some("Back at six".into()),
            color: Some("orange".into()),
            x: 0.6,
            y: 0.5,
            ..NoteDto::default()
        },
    );
    fake.send_frame(&serde_json::json!({ "type": "board_note", "note": live }).to_string());
    let main = shell.main_window().expect("the main window");
    let badge = || {
        super::settings::find::<gtk::Label>(&main.content_header(), |label| {
            label.has_css_class("fc-badge")
        })
    };
    let raised = h
        .wait_until(WAIT, || {
            badge().is_some_and(|label| label.is_visible() && label.text() == "1")
        })
        .await;
    h.screenshot("badge").await;
    board_window::open(None);
    if h.input() {
        h.focus(&window).await;
    }
    let cleared = h
        .wait_until(WAIT, || badge().is_some_and(|label| !label.is_visible()))
        .await;
    h.check(
        "badge_counts_news_until_shown",
        raised && (cleared || !h.input()),
        format!("raised {raised}, cleared when shown in front {cleared}"),
    );

    // ---- dark and narrow -------------------------------------------------------------------------------
    let styles = adw::StyleManager::default();
    styles.set_color_scheme(adw::ColorScheme::ForceDark);
    h.sleep(Duration::from_millis(300)).await;
    h.screenshot("wall-dark").await;
    styles.set_color_scheme(adw::ColorScheme::Default);
    if h.input() {
        h.resize(&window, 520, 420).await;
        h.sleep(Duration::from_millis(300)).await;
        h.screenshot("wall-narrow").await;
        h.resize(
            &window,
            board_window::DEFAULT_SIZE.0,
            board_window::DEFAULT_SIZE.1,
        )
        .await;
    }
    board.window.close();
    let unhandled = fake.unhandled();
    h.check(
        "fake_server_knew_every_request",
        unhandled.is_empty(),
        format!("{unhandled:?}"),
    );
}

fn count(requests: &[String], method: &str) -> usize {
    requests
        .iter()
        .filter(|line| line.starts_with(method) && line.contains("/board/notes/"))
        .count()
}
