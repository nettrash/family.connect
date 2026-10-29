//! The note sheet (§11; the Mac's `MacNoteEditor`): an `adw::Dialog` that writes a new note, list
//! or event, or rewrites the author's own — the words (a `gtk::TextView`, "n characters left" near
//! the cap), a list's lines, an event's start, end and place, and the look: colour, size, hand.
//!
//! What is being written is a `NoteDraft`, kept apart from the note so a save sends ONLY WHAT
//! CHANGED against the note as it stood when the sheet opened (`NoteDraft::patch`); names are
//! resolved from the words at save, against the live roster. Why it cannot be saved is
//! `NoteDraft::problem`'s answer (empty: Save is simply off); refusals are `note_sheet_text`'s.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{MemberDto, NoteDto};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::note_draft::{DraftLine, NoteDraft, note_sheet_text};
use fc_text::board::{COLORS, Font, Kind, MAX_TEXT_CHARS, Size, capped, remaining, shows_counter};
use time::OffsetDateTime;

use crate::services::log;
use crate::settings::ui;
use crate::shell::Shell;

/// The sheet open now (the self-test drives it).
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "the self-test drives it")
)]
pub struct Sheet {
    pub text: gtk::TextView,
    pub save: gtk::Button,
    pub colors: adw::ToggleGroup,
}

impl std::fmt::Debug for Sheet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sheet").finish_non_exhaustive()
    }
}

thread_local! {
    static OPEN: RefCell<Option<Rc<Sheet>>> = const { RefCell::new(None) };
}

/// The sheet while it is open.
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "the self-test reads it")
)]
pub fn current() -> Option<Rc<Sheet>> {
    OPEN.with(|open| open.borrow().clone())
}

/// A new note of `kind` (Add Note, Add List, Add an event).
pub fn compose(parent: &impl IsA<gtk::Widget>, kind: Kind, done: impl Fn() + 'static) {
    let shell = Shell::get();
    let draft = NoteDraft::blank(
        kind,
        OffsetDateTime::now_utc(),
        shell.zone.as_ref(),
        &|count| {
            usize::try_from(glib::random_int_range(0, i32::try_from(count).unwrap_or(1)))
                .unwrap_or(0)
        },
    );
    open(parent, kind, None, draft, Rc::new(done));
}

/// The author's own note, to rewrite.
pub fn edit(parent: &impl IsA<gtk::Widget>, note: NoteDto, done: impl Fn() + 'static) {
    let kind = Kind::from_name(note.kind.as_deref());
    let draft = NoteDraft::of(&note);
    open(parent, kind, Some(note), draft, Rc::new(done));
}

fn open(
    parent: &impl IsA<gtk::Widget>,
    kind: Kind,
    current: Option<NoteDto>,
    draft: NoteDraft,
    done: Rc<dyn Fn()>,
) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let (dialog, view, _) = ui::dialog(
        &note_sheet_text::title(current.is_none(), kind, say.as_ref()),
        460,
        560,
    );
    let (header, save) = ui::dialog_bar(&dialog, say.get("Cancel"), say.get("Save"), false);
    view.add_top_bar(&header);
    let draft = Rc::new(RefCell::new(draft));
    let page = adw::PreferencesPage::new();
    let problem = ui::problem_label();

    // ---- the words ----------------------------------------------------------------------------
    let words = adw::PreferencesGroup::builder()
        .title(note_sheet_text::field_label(kind, say.as_ref()))
        .build();
    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .accepts_tab(false)
        .height_request(if kind == Kind::Photo { 60 } else { 110 })
        .build();
    text.buffer().set_text(draft.borrow().text());
    text.update_property(&[gtk::accessible::Property::Label(
        &note_sheet_text::field_label(kind, say.as_ref()),
    )]);
    let frame = gtk::Frame::new(None);
    frame.set_child(Some(&text));
    frame.add_css_class("view");
    words.add(&frame);
    let counter = ui::footnote("");
    counter.set_visible(false);
    words.add(&counter);
    page.add(&words);

    // ---- a list's lines -------------------------------------------------------------------------
    let lines_box = gtk::ListBox::new();
    if kind == Kind::Tasks {
        let lines = adw::PreferencesGroup::builder()
            .title(say.get("Things to do"))
            .build();
        lines_box.add_css_class("boxed-list");
        lines_box.set_selection_mode(gtk::SelectionMode::None);
        lines.add(&lines_box);
        let add = ui::button_row(say.get("Add a thing"), false, {
            let (lines_box, draft) = (lines_box.clone(), Rc::clone(&draft));
            move || {
                draft.borrow_mut().lines.push(DraftLine::new(None, ""));
                fill_lines(&lines_box, &draft);
            }
        });
        let adder = gtk::ListBox::new();
        adder.add_css_class("boxed-list");
        adder.set_selection_mode(gtk::SelectionMode::None);
        adder.set_margin_top(6);
        adder.append(&add);
        lines.add(&adder);
        fill_lines(&lines_box, &draft);
        page.add(&lines);
    }

    // ---- an event's when and where ----------------------------------------------------------------
    if kind == Kind::Event {
        let when = adw::PreferencesGroup::builder()
            .title(say.get("When"))
            .build();
        let starts = time_row(say.get("Starts"), Rc::clone(&draft), Moment::Starts);
        when.add(&starts);
        let has_end = adw::SwitchRow::builder()
            .title(say.get("Has an end"))
            .active(draft.borrow().has_end)
            .build();
        let ends = time_row(say.get("Ends"), Rc::clone(&draft), Moment::Ends);
        ends.set_visible(draft.borrow().has_end);
        {
            let (draft, ends) = (Rc::clone(&draft), ends.clone());
            has_end.connect_active_notify(move |row| {
                draft.borrow_mut().has_end = row.is_active();
                ends.set_visible(row.is_active());
            });
        }
        when.add(&has_end);
        when.add(&ends);
        let place = adw::EntryRow::builder()
            .title(say.get("Place"))
            .text(&draft.borrow().place)
            .build();
        {
            let draft = Rc::clone(&draft);
            place.connect_changed(move |row| {
                draft.borrow_mut().place =
                    capped(&row.text(), fc_text::board::MAX_PLACE_CHARS).to_string();
            });
        }
        when.add(&place);
        page.add(&when);
    }

    // ---- the look -------------------------------------------------------------------------------
    let look = adw::PreferencesGroup::new();
    let colors = adw::ToggleGroup::builder()
        .valign(gtk::Align::Center)
        .build();
    for name in COLORS {
        let swatch = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        swatch.add_css_class("fc-swatch");
        swatch.add_css_class(&format!("fc-swatch-{name}"));
        swatch.set_halign(gtk::Align::Center);
        swatch.set_valign(gtk::Align::Center);
        colors.add(adw::Toggle::builder().name(name).child(&swatch).build());
    }
    colors.set_active_name(Some(&draft.borrow().color));
    {
        let draft = Rc::clone(&draft);
        colors.connect_active_name_notify(move |group| {
            if let Some(name) = group.active_name() {
                draft.borrow_mut().color = name.to_string();
            }
        });
    }
    let color_row = adw::ActionRow::builder().title(say.get("Colour")).build();
    color_row.add_suffix(&colors);
    look.add(&color_row);
    let sizes = adw::ToggleGroup::builder()
        .valign(gtk::Align::Center)
        .build();
    for size in Size::ALL {
        sizes.add(
            adw::Toggle::builder()
                .name(size.name())
                .label(size.title())
                .build(),
        );
    }
    sizes.set_active_name(Some(draft.borrow().size.name()));
    {
        let draft = Rc::clone(&draft);
        sizes.connect_active_name_notify(move |group| {
            draft.borrow_mut().size = Size::from_name(group.active_name().as_deref());
        });
    }
    let size_row = adw::ActionRow::builder().title(say.get("Size")).build();
    size_row.add_suffix(&sizes);
    look.add(&size_row);
    let fonts = adw::ToggleGroup::builder()
        .valign(gtk::Align::Center)
        .build();
    for font in Font::ALL {
        fonts.add(
            adw::Toggle::builder()
                .name(font.name())
                .label(font.title())
                .build(),
        );
    }
    fonts.set_active_name(Some(draft.borrow().font.name()));
    {
        let draft = Rc::clone(&draft);
        fonts.connect_active_name_notify(move |group| {
            draft.borrow_mut().font = Font::from_name(group.active_name().as_deref());
        });
    }
    let font_row = adw::ActionRow::builder().title(say.get("Font")).build();
    font_row.add_suffix(&fonts);
    look.add(&font_row);
    look.add(&problem);
    page.add(&look);
    view.set_content(Some(&page));

    // ---- what the words allow ---------------------------------------------------------------------
    let judge: Rc<dyn Fn()> = {
        let (draft, save, problem) = (Rc::clone(&draft), save.clone(), problem.clone());
        Rc::new(move || {
            let shell = Shell::get();
            match draft.borrow().problem(kind, shell.say.as_ref()) {
                None => {
                    save.set_sensitive(true);
                    ui::show_problem(&problem, "");
                }
                Some(text) => {
                    save.set_sensitive(false);
                    ui::show_problem(&problem, &text);
                }
            }
        })
    };
    {
        let (draft, counter, judge) = (Rc::clone(&draft), counter.clone(), Rc::clone(&judge));
        text.buffer().connect_changed(move |buffer| {
            let typed = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let kept = capped(&typed, MAX_TEXT_CHARS);
            if kept.len() != typed.len() {
                // Over the server's cap (counted in scalars): cut back, once.
                let kept = kept.to_string();
                buffer.set_text(&kept);
                return;
            }
            draft.borrow_mut().set_text(kept);
            let shell = Shell::get();
            let left = i64::try_from(remaining(kept)).unwrap_or(0);
            counter.set_text(&shell.say.plural("%lld characters left", left, &[&left]));
            counter.set_visible(shows_counter(kept));
            judge();
        });
    }
    judge();

    let sheet = Rc::new(Sheet {
        text: text.clone(),
        save: save.clone(),
        colors,
    });
    OPEN.with(|open| *open.borrow_mut() = Some(Rc::clone(&sheet)));
    dialog.connect_closed(|_| {
        OPEN.with(|open| open.borrow_mut().take());
    });
    {
        let (weak, problem, current) = (dialog.downgrade(), problem.clone(), current.clone());
        let busy = ui::busy(&save);
        save.connect_clicked(move |_| {
            let Some(connection) = Shell::get().connection() else {
                return;
            };
            let roster: Vec<MemberDto> = connection
                .chats
                .members()
                .map(|members| members.to_vec())
                .unwrap_or_default();
            let model = super::model(&connection);
            let held = sheet_draft(&draft);
            let (weak, problem, done, busy, current) = (
                weak.clone(),
                problem.clone(),
                Rc::clone(&done),
                Rc::clone(&busy),
                current.clone(),
            );
            busy(true);
            ui::spawn(async move {
                let answer = match &current {
                    None => {
                        // Dropped where the Mac drops a new note: somewhere in the middle.
                        let at = (
                            glib::random_double_range(0.25, 0.65),
                            glib::random_double_range(0.25, 0.65),
                        );
                        model.create(&held.new_note(kind, at, &roster)).await
                    }
                    Some(note) => {
                        let patch = held.patch(note, &roster);
                        if NoteDraft::is_empty(&patch) {
                            Ok(None)
                        } else {
                            model.patch(note.id, &patch).await
                        }
                    }
                };
                busy(false);
                match answer {
                    Ok(None) => {
                        crate::fc_info!("board: note saved");
                        if let Some(dialog) = weak.upgrade() {
                            dialog.close();
                        }
                        done();
                    }
                    Ok(Some(error)) => {
                        crate::fc_info!("board: save refused ({})", error.code);
                        let shell = Shell::get();
                        ui::show_problem(
                            &problem,
                            &note_sheet_text::failure(&error, shell.say.as_ref()),
                        );
                    }
                    Err(error) => {
                        crate::fc_warn!("board: the cache refused ({})", log::store_error(&error))
                    }
                }
            });
        });
    }
    dialog.present(Some(parent));
    text.grab_focus();
}

/// The draft as it stands, its event's end kept after its start.
fn sheet_draft(draft: &Rc<RefCell<NoteDraft>>) -> NoteDraft {
    let mut held = draft.borrow().clone();
    held.keep_end_after_start();
    held
}

/// A list's lines, drawn again: one entry per line, each with a remove button.
fn fill_lines(list: &gtk::ListBox, draft: &Rc<RefCell<NoteDraft>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let shell = Shell::get();
    let count = draft.borrow().lines.len();
    for at in 0..count {
        let line = draft.borrow().lines[at].clone();
        let row = adw::EntryRow::builder().text(&line.text).build();
        row.clone()
            .upcast::<gtk::Widget>()
            .update_property(&[gtk::accessible::Property::Label(
                shell.say.get("Things to do"),
            )]);
        {
            let draft = Rc::clone(draft);
            row.connect_changed(move |row| {
                if let Some(line) = draft.borrow_mut().lines.get_mut(at) {
                    line.text =
                        capped(&row.text(), fc_text::board::MAX_TASK_ITEM_CHARS).to_string();
                }
            });
        }
        let remove = gtk::Button::builder()
            .icon_name("list-remove-symbolic")
            .tooltip_text(shell.say.get("Remove"))
            .valign(gtk::Align::Center)
            .build();
        remove.add_css_class("flat");
        {
            let (list, draft) = (list.clone(), Rc::clone(draft));
            remove.connect_clicked(move |_| {
                if at < draft.borrow().lines.len() {
                    draft.borrow_mut().lines.remove(at);
                }
                fill_lines(&list, &draft);
            });
        }
        row.add_suffix(&remove);
        list.append(&row);
    }
}

/// Which instant of an event a row edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Moment {
    Starts,
    Ends,
}

/// A row of a date (a calendar in a popover) and a time (hour and minute), in the reader's zone.
fn time_row(title: &str, draft: Rc<RefCell<NoteDraft>>, moment: Moment) -> adw::ActionRow {
    let shell = Shell::get();
    let row = adw::ActionRow::builder().title(title).build();
    let held = match moment {
        Moment::Starts => draft.borrow().starts,
        Moment::Ends => draft.borrow().ends,
    }
    .unwrap_or_else(OffsetDateTime::now_utc);
    let local = glib::DateTime::from_unix_local(held.unix_timestamp())
        .unwrap_or_else(|_| glib::DateTime::now_local().expect("the local clock"));
    let calendar = gtk::Calendar::new();
    calendar.set_date(&local);
    let date = gtk::MenuButton::builder()
        .valign(gtk::Align::Center)
        .label(date_text(&local, &shell.culture))
        .build();
    let popover = gtk::Popover::new();
    popover.set_child(Some(&calendar));
    date.set_popover(Some(&popover));
    let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    hour.set_value(f64::from(local.hour()));
    hour.set_valign(gtk::Align::Center);
    hour.set_orientation(gtk::Orientation::Vertical);
    let minute = gtk::SpinButton::with_range(0.0, 59.0, 5.0);
    minute.set_value(f64::from(local.minute()));
    minute.set_valign(gtk::Align::Center);
    minute.set_orientation(gtk::Orientation::Vertical);
    let separator = gtk::Label::new(Some(&shell.culture.time_separator));
    let clock = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    clock.append(&hour);
    clock.append(&separator);
    clock.append(&minute);
    row.add_suffix(&date);
    row.add_suffix(&clock);
    let apply: Rc<dyn Fn()> = {
        let (calendar, hour, minute, date, draft) = (
            calendar.clone(),
            hour.clone(),
            minute.clone(),
            date.clone(),
            Rc::clone(&draft),
        );
        Rc::new(move || {
            let day = calendar.date();
            let Ok(at) = glib::DateTime::from_local(
                day.year(),
                day.month(),
                day.day_of_month(),
                hour.value_as_int(),
                minute.value_as_int(),
                0.0,
            ) else {
                return;
            };
            date.set_label(&date_text(&at, &Shell::get().culture));
            let instant = OffsetDateTime::from_unix_timestamp(at.to_unix()).ok();
            let mut held = draft.borrow_mut();
            match moment {
                Moment::Starts => held.starts = instant,
                Moment::Ends => held.ends = instant,
            }
        })
    };
    {
        let (apply, popover) = (Rc::clone(&apply), popover.clone());
        calendar.connect_day_selected(move |_| {
            apply();
            popover.popdown();
        });
    }
    {
        let apply = Rc::clone(&apply);
        hour.connect_value_changed(move |_| apply());
    }
    minute.connect_value_changed(move |_| apply());
    row
}

/// A day as the reader writes it short ("24/12/2026", "24.12.2026").
fn date_text(at: &glib::DateTime, culture: &fc_logic::date_culture::DateCulture) -> String {
    let date = u8::try_from(at.month())
        .ok()
        .and_then(|month| time::Month::try_from(month).ok())
        .and_then(|month| {
            time::Date::from_calendar_date(at.year(), month, u8::try_from(at.day_of_month()).ok()?)
                .ok()
        });
    match date {
        Some(date) => culture.format(
            time::PrimitiveDateTime::new(date, time::Time::MIDNIGHT),
            "d",
        ),
        None => String::new(),
    }
}
