//! The message reaction picker. The composer still uses GTK's general emoji chooser.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use fc_core::store::Database;
use fc_core::text::string_catalog::StringCatalog;
use fc_text::emoji::EMOJI_CATALOG;
use fc_text::emoji_search::{self, Annotations};
use fc_text::i18n::Lang;

use crate::shell::Shell;

const COLUMNS: u32 = 8;
const WIDTH: i32 = 332;
const HEIGHT: i32 = 285;

type Pick = Rc<dyn Fn(&str)>;

/// Open on the message itself so closing the menu cannot take away the picker's focus owner.
pub(super) fn popup(
    anchor: &gtk::Widget,
    point: &gtk::gdk::Rectangle,
    on_pick: impl Fn(&str) -> bool + 'static,
) {
    let shell = Shell::get();
    let account = shell.connection().and_then(|connection| {
        let id = connection.session.state().me?.id;
        let key = recents_key(&connection.server.to_string(), id)?;
        Some((connection, id, key))
    });
    let recent = account
        .as_ref()
        .map_or_else(Vec::new, |(connection, _, key)| {
            recents(&connection.cache, key)
        });
    let picker = gtk::Popover::new();
    picker.set_parent(anchor);
    picker.set_pointing_to(Some(point));
    picker.set_widget_name("reaction-picker");
    picker.add_css_class("fc-reaction-picker");
    let weak_picker = picker.downgrade();
    let pick: Pick = Rc::new(move |emoji| {
        if let Some((connection, id, _)) = &account {
            // A window or connection might have gone away while this menu was open.
            if !Shell::get().connection().is_some_and(|current| {
                Rc::ptr_eq(&current, connection)
                    && current
                        .session
                        .state()
                        .me
                        .is_some_and(|user| user.id == *id)
            }) {
                return;
            }
        }
        let Some(picker) = weak_picker.upgrade().filter(|picker| picker.is_visible()) else {
            return;
        };
        picker.popdown();
        if on_pick(emoji)
            && let Some((connection, _, key)) = &account
        {
            remember(&connection.cache, key, emoji);
        }
    });
    let local = desktop_annotations(shell.paths.data_dir());
    let (content, search) = panel(shell.say.as_ref(), recent, local, pick);
    picker.set_child(Some(&content));
    let weak_picker = picker.downgrade();
    search.connect_stop_search(move |_| {
        if let Some(picker) = weak_picker.upgrade() {
            picker.popdown();
        }
    });
    let row = std::iter::successors(Some(anchor.clone()), |widget| widget.parent())
        .find(|widget| widget.has_css_class("fc-chat-row"))
        .map(|widget| widget.downgrade());
    picker.connect_closed(move |picker| {
        // Give GTK a live focus widget before it processes the popup's focus-out.
        if let Some(row) = row
            .as_ref()
            .and_then(|row| row.upgrade())
            .filter(|row| row.is_mapped())
        {
            row.grab_focus();
        }
        let picker = picker.clone();
        glib::idle_add_local_once(move || {
            if picker.parent().is_some() {
                picker.unparent();
            }
        });
    });
    picker.popup();
    search.grab_focus();
}

fn panel(
    say: &dyn StringCatalog,
    recent: Vec<String>,
    local: Option<&'static Annotations>,
    pick: Pick,
) -> (gtk::Box, gtk::SearchEntry) {
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .width_request(WIDTH)
        .build();
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(say.get("Search emoji")));
    search.update_property(&[gtk::accessible::Property::Label(say.get("Search emoji"))]);
    column.append(&search);

    let heading = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["heading"])
        .build();
    heading.set_widget_name("reaction-picker-heading");
    column.append(&heading);
    let results = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .activate_on_single_click(true)
        .homogeneous(true)
        .min_children_per_line(COLUMNS)
        .max_children_per_line(COLUMNS)
        .valign(gtk::Align::Start)
        .css_classes(["fc-reaction-grid"])
        .build();
    results.set_widget_name("reaction-picker-grid");
    results.connect_child_activated(move |_, cell| {
        if let Some(label) = cell.child().and_downcast::<gtk::Label>() {
            pick(label.text().as_str());
        }
    });
    // GTK navigates and activates FlowBox children with arrows and Return. Give
    // the focused result the same Space activation as a regular button.
    let keys = gtk::EventControllerKey::new();
    let grid = results.downgrade();
    keys.connect_key_pressed(move |_, key, _, state| {
        if matches!(key, gtk::gdk::Key::space | gtk::gdk::Key::KP_Space)
            && !state.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK,
            )
            && let Some(cell) = grid
                .upgrade()
                .and_then(|grid| grid.focus_child())
                .and_downcast::<gtk::FlowBoxChild>()
        {
            cell.activate();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    results.add_controller(keys);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(92)
        .max_content_height(HEIGHT)
        .propagate_natural_height(true)
        .child(&results)
        .build();
    column.append(&scroll);

    // Keep these as literal keys so Linux's translation generator sees them.
    let labels = [
        say.get("Smileys"),
        say.get("Gestures"),
        say.get("Hearts"),
        say.get("Animals & Nature"),
        say.get("Food & Drink"),
        say.get("Activities"),
        say.get("Travel & Places"),
        say.get("Objects & Symbols"),
    ]
    .map(str::to_owned);
    let recent_label = say.get("Recent emoji").to_owned();
    let search_label = say.get("Search results").to_owned();
    let empty_label = say.get("No emoji found").to_owned();
    let tabs = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .homogeneous(true)
        .css_classes(["fc-reaction-categories"])
        .build();
    column.append(&tabs);
    let category = Rc::new(Cell::new(recent.is_empty().then_some(0)));
    let draw = Rc::new({
        let (heading, results, scroll) =
            (heading.downgrade(), results.downgrade(), scroll.downgrade());
        let category = category.clone();
        let labels = labels.clone();
        let recent_label = recent_label.clone();
        let recent = recent.clone();
        move |query: &str| {
            let (Some(heading), Some(results), Some(scroll)) =
                (heading.upgrade(), results.upgrade(), scroll.upgrade())
            else {
                return;
            };
            results.remove_all();
            if query.trim().is_empty() {
                let (title, emoji): (&str, Vec<&str>) = match category.get() {
                    Some(index) => (&labels[index], EMOJI_CATALOG[index].emoji.to_vec()),
                    None => (&recent_label, recent.iter().map(String::as_str).collect()),
                };
                heading.set_label(title);
                for emoji in emoji {
                    add_emoji(
                        &results,
                        emoji,
                        emoji_search::name(emoji, local).unwrap_or(emoji),
                    );
                }
            } else {
                let matches = emoji_search::search(query, local);
                heading.set_label(if matches.is_empty() {
                    &empty_label
                } else {
                    &search_label
                });
                for result in matches {
                    add_emoji(&results, result.emoji, result.name);
                }
            }
            scroll.vadjustment().set_value(0.0);
        }
    });

    let mut first = None;
    let choices = (!recent.is_empty())
        .then_some((None, "◷", recent_label.as_str()))
        .into_iter()
        .chain(
            EMOJI_CATALOG
                .iter()
                .zip(&labels)
                .enumerate()
                .map(|(index, (group, label))| (Some(index), group.emoji[0], label.as_str())),
        );
    for (index, glyph, title) in choices {
        let tab = gtk::ToggleButton::builder()
            .label(glyph)
            .tooltip_text(title)
            .css_classes(["flat"])
            .build();
        tab.update_property(&[gtk::accessible::Property::Label(title)]);
        if let Some(first) = &first {
            tab.set_group(Some(first));
        } else {
            tab.set_active(true);
            first = Some(tab.clone());
        }
        let (category, draw, search, results) = (
            category.clone(),
            draw.clone(),
            search.downgrade(),
            results.downgrade(),
        );
        tab.connect_clicked(move |tab| {
            tab.set_active(true);
            category.set(index);
            if let Some(search) = search.upgrade() {
                if search.text().is_empty() {
                    draw("");
                } else {
                    search.set_text("");
                }
            }
            if tab.has_focus()
                && let Some(cell) = results.upgrade().and_then(|flow| flow.child_at_index(0))
            {
                cell.grab_focus();
            }
        });
        tabs.append(&tab);
    }
    draw("");
    search.connect_changed(move |entry| draw(entry.text().as_str()));
    let results = results.downgrade();
    search.connect_activate(move |entry| {
        if !entry.text().trim().is_empty()
            && let Some(cell) = results.upgrade().and_then(|flow| flow.child_at_index(0))
        {
            cell.activate();
        }
    });
    (column, search)
}

fn add_emoji(flow: &gtk::FlowBox, emoji: &str, name: &str) {
    let glyph = gtk::Label::new(Some(emoji));
    glyph.set_can_target(false);
    let cell = gtk::FlowBoxChild::builder()
        .accessible_role(gtk::AccessibleRole::Button)
        .child(&glyph)
        .tooltip_text(name)
        .build();
    cell.update_property(&[gtk::accessible::Property::Label(name)]);
    flow.append(&cell);
}

fn desktop_annotations(data_dir: &Path) -> Option<&'static Annotations> {
    // Linux reads its desktop language once on startup. Only parse that catalogue.
    static LOCAL: OnceLock<Option<Annotations>> = OnceLock::new();
    LOCAL
        .get_or_init(|| {
            let lang = fc_text::i18n::lang();
            let annotations = localized(data_dir, lang);
            if lang != Lang::En && annotations.is_none() {
                crate::fc_warn!(
                    "no usable {} emoji search table; English instead",
                    lang.tag()
                );
            }
            annotations
        })
        .as_ref()
}

fn localized(data_dir: &Path, lang: Lang) -> Option<Annotations> {
    if lang == Lang::En {
        return None;
    }
    // Shipped copies of the shared CLDR files, in the package's data folder
    // beside i18n. A missing or invalid file leaves the English index available.
    let path = data_dir
        .join("emoji-search")
        .join(format!("{}.tsv", lang.tag()));
    Annotations::parse(lang, &std::fs::read_to_string(path).ok()?)
}

fn recents_key(server: &str, account: i64) -> Option<String> {
    // The cache's meta is wiped on sign-out. Key it per account and server as
    // well, so a stale cache can never give another account those choices.
    (account > 0).then(|| format!("reaction-recents:{account}:{server}"))
}

fn recents(cache: &Database, key: &str) -> Vec<String> {
    let stored = cache.meta_text(key).ok().flatten().unwrap_or_default();
    let stored = serde_json::from_str::<Vec<String>>(&stored).unwrap_or_default();
    emoji_search::recent_with(&stored, "")
}

fn remember(cache: &Database, key: &str, emoji: &str) {
    let values = emoji_search::recent_with(&recents(cache, key), emoji);
    if let Ok(value) = serde_json::to_string(&values)
        && let Err(error) = cache.set_meta_text(key, &value)
    {
        crate::fc_warn!(
            "saving recent reactions: {}",
            crate::services::log::store_error(&error)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_locale_files_match_the_shared_catalogue() {
        let app = Path::new(env!("CARGO_MANIFEST_DIR"));
        let shared_data = app.join("../../web/text/data");
        let shared = shared_data.join("emoji-search");
        let shipped = app.join("../data/emoji-search");
        for file in [
            "de.tsv",
            "en.tsv",
            "es.tsv",
            "fr.tsv",
            "ja.tsv",
            "ru.tsv",
            "sr.tsv",
            "sr-Latn.tsv",
            "zh-Hans.tsv",
        ] {
            assert!(
                std::fs::read(shipped.join(file)).unwrap()
                    == std::fs::read(shared.join(file)).unwrap(),
                "{file} differs from the shared catalogue"
            );
        }
        let notice = "UNICODE-LICENSE.txt";
        assert!(
            std::fs::read(shipped.join(notice)).unwrap()
                == std::fs::read(shared_data.join(notice)).unwrap(),
            "the Linux Unicode notice differs from the shared notice"
        );
    }

    #[test]
    fn linux_catalogues_use_serbian_corrections_and_russian_yo_folding() {
        let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../data");
        let serbian = localized(&data_dir, Lang::Sr).expect("Serbian locale file");
        let russian = localized(&data_dir, Lang::Ru).expect("Russian locale file");
        for (catalogue, query, emoji) in [
            (&serbian, "пламен", "🔥"),
            (&serbian, "сузе", "😂"),
            (&russian, "ёлка", "🎄"),
        ] {
            assert!(
                emoji_search::search(query, Some(catalogue))
                    .iter()
                    .any(|result| result.emoji == emoji),
                "{query} should find {emoji}"
            );
        }
    }

    #[test]
    fn linux_catalogues_search_all_nine_languages_and_keep_english_aliases() {
        let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../data");
        for (lang, query, name) in [
            (Lang::En, "fire", "fire"),
            (Lang::De, "Feuer", "Feuer"),
            (Lang::Es, "fuego", "fuego"),
            (Lang::Fr, "feu", "feu"),
            (Lang::Ja, "火", "火"),
            (Lang::Ru, "огонь", "огонь"),
            (Lang::Sr, "ватра", "ватра"),
            (Lang::SrLatn, "vatra", "vatra"),
            (Lang::ZhHans, "火", "火焰"),
        ] {
            let local = localized(&data_dir, lang);
            assert_eq!(local.is_some(), lang != Lang::En);
            assert!(
                emoji_search::search(query, local.as_ref())
                    .iter()
                    .any(|hit| hit.emoji == "🔥" && hit.name == name)
            );
            assert_eq!(
                emoji_search::search("  :HEART: ", local.as_ref())[0].emoji,
                "❤️"
            );
            assert!(emoji_search::search("fc-no-such-emoji", local.as_ref()).is_empty());
        }
    }

    #[test]
    fn english_search_works_when_the_desktop_catalogue_is_missing() {
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let local = localized(&missing, Lang::Ru);
        assert!(local.is_none());
        let results = emoji_search::search("fire", local.as_ref());
        assert_eq!(results[0].emoji, "🔥");
        assert_eq!(results[0].name, "fire");
    }

    #[test]
    fn recent_reactions_are_device_local_account_scoped_canonical_and_wiped_with_session() {
        let cache = Database::open_in_memory().unwrap();
        let alice = recents_key("https://chat.example/", 1).unwrap();
        let bob = recents_key("https://chat.example/", 2).unwrap();
        let other_server = recents_key("https://elsewhere.example/", 1).unwrap();
        cache.set_meta_text(&alice, "not json").unwrap();
        assert!(recents(&cache, &alice).is_empty());
        remember(&cache, &alice, "❤");
        remember(&cache, &alice, "🔥");
        remember(&cache, &alice, "❤️");
        assert_eq!(recents(&cache, &alice), ["❤️", "🔥"]);
        assert!(recents(&cache, &bob).is_empty());
        assert!(recents(&cache, &other_server).is_empty());
        assert!(recents_key("https://chat.example/", 0).is_none());
        cache.wipe_all().unwrap();
        assert!(recents(&cache, &alice).is_empty());
    }
}
