//! Check the real message menu and picker against the chat group's local server.

use adw::prelude::*;
use fc_core::protocol::MessageDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::conversation_rows;

use crate::conversation::ConversationPage;
use crate::conversation::menu::{self, MenuFacts, Safety};
use crate::shell::Shell;

use super::super::Harness;
use super::{SHORT, WAIT, World, find};

async fn open(
    h: &Harness,
    window: &gtk::Window,
    page: &ConversationPage,
    message: &MessageDto,
) -> Option<gtk::Popover> {
    let key = conversation_rows::message_key(message.id);
    if !page.timeline().scroll_to(&key, false) {
        return None;
    }
    h.sleep(std::time::Duration::from_millis(250)).await;
    let row = page.timeline().row(&key)?;
    let bubble = find::<gtk::Widget>(&row)
        .into_iter()
        .find(|widget| widget.has_css_class("fc-chat-bubble"))?;
    let (x, y) = (
        f64::from(bubble.width()) / 2.0,
        f64::from(bubble.height()) / 2.0,
    );
    if h.input() {
        let (x, y) = h.screen_point(window, &bubble, x, y).await?;
        let (ok, _) = h
            .xdotool(&["mousemove", &x.to_string(), &y.to_string(), "click", "3"])
            .await;
        if !ok {
            return None;
        }
    } else {
        menu::popup(
            &MenuFacts {
                message: message.clone(),
                may_edit: false,
                in_thread: false,
                text_item: fc_logic::message_menu::TextItem::None,
                my_reaction: None,
                hidden: false,
                safety: Safety::None,
            },
            "win",
            &bubble,
            x,
            y,
        );
    }
    let more = || {
        find::<gtk::Button>(&bubble).into_iter().find(|button| {
            button.is_mapped()
                && button.icon_name().as_deref() == Some("view-more-horizontal-symbolic")
        })
    };
    if !h.wait_until(SHORT, || more().is_some()).await {
        return None;
    }
    let more = more()?;
    more.emit_clicked();
    let picker = || {
        find::<gtk::Popover>(&bubble)
            .into_iter()
            .find(|child| child.is_mapped() && child.widget_name() == "reaction-picker")
    };
    if !h.wait_until(SHORT, || picker().is_some()).await {
        return None;
    }
    picker()
}

fn grid(picker: &gtk::Popover) -> Option<gtk::FlowBox> {
    find::<gtk::FlowBox>(picker)
        .into_iter()
        .find(|w| w.widget_name() == "reaction-picker-grid")
}

fn result(picker: &gtk::Popover, emoji: &str) -> Option<gtk::FlowBoxChild> {
    find::<gtk::FlowBoxChild>(&grid(picker)?)
        .into_iter()
        .find(|cell| {
            cell.child()
                .and_downcast::<gtk::Label>()
                .is_some_and(|label| label.text() == emoji)
        })
}

fn heading(picker: &gtk::Popover) -> String {
    find::<gtk::Label>(picker)
        .into_iter()
        .find(|w| w.widget_name() == "reaction-picker-heading")
        .map(|label| label.text().to_string())
        .unwrap_or_default()
}

fn message_has_focus(page: &ConversationPage, window: &gtk::Window, id: i64) -> bool {
    page.timeline()
        .row(&conversation_rows::message_key(id))
        .is_some_and(|row| {
            gtk::prelude::RootExt::focus(window)
                .is_some_and(|focus| focus == row || focus.is_ancestor(&row))
        })
}

pub(super) async fn run(
    h: &Harness,
    shell: &Shell,
    window: &gtk::Window,
    page: &ConversationPage,
    world: &World,
    incoming: &MessageDto,
) {
    let say = &shell.say;
    let Some(picker) = open(h, window, page, incoming).await else {
        h.check(
            "reaction_picker_open",
            false,
            "the message's More reactions menu did not open the picker",
        );
        return;
    };
    let Some(search) = find::<gtk::SearchEntry>(&picker).into_iter().next() else {
        h.check(
            "reaction_picker_open",
            false,
            "the picker has no search entry",
        );
        picker.popdown();
        return;
    };
    let focused = gtk::prelude::RootExt::focus(window).is_some_and(|widget| {
        widget == *search.upcast_ref::<gtk::Widget>() || widget.is_ancestor(&search)
    });
    h.check(
        "reaction_picker_open",
        focused,
        "More reactions opened the picker and focused search",
    );
    let start = heading(&picker);
    search.set_text("fc-no-such-emoji");
    h.check(
        "reaction_picker_empty_and_clear",
        heading(&picker) == say.get("No emoji found") && {
            search.set_text("");
            heading(&picker) == start
        },
        "empty search gives the localized status and clearing restores the category",
    );
    search.set_text("fire");
    if let Some(tab) = find::<gtk::ToggleButton>(&picker)
        .into_iter()
        .find(|tab| tab.tooltip_text().as_deref() == Some(say.get("Hearts")))
    {
        tab.emit_clicked();
    }
    h.check(
        "reaction_picker_categories",
        search.text().is_empty()
            && heading(&picker) == say.get("Hearts")
            && result(&picker, "❤️").is_some(),
        "Hearts shortcut clears the active query and opens its category",
    );
    h.screenshot("reaction-picker-category").await;

    search.set_text("a");
    if let Some(scroll) = find::<gtk::ScrolledWindow>(&picker).into_iter().next() {
        let adjustment = scroll.vadjustment();
        let can_scroll = h
            .wait_until(SHORT, || adjustment.upper() > adjustment.page_size() + 20.0)
            .await;
        adjustment.set_value(100.0);
        let scrolled = adjustment.value() > 0.0;
        search.set_text("e");
        h.check(
            "reaction_picker_new_query_starts_at_top",
            can_scroll && scrolled && adjustment.value() == 0.0,
            "changing a query returns to the highest-ranked results",
        );
    } else {
        h.check(
            "reaction_picker_new_query_starts_at_top",
            false,
            "picker results have no scroll area",
        );
    }

    let name = match say.language() {
        "de" => "Feuer",
        "es" => "fuego",
        "fr" => "feu",
        "ja" => "火",
        "ru" => "огонь",
        "sr" => "ватра",
        "sr-Latn" => "vatra",
        "zh-Hans" => "火焰",
        _ => "fire",
    };
    search.set_text("fire");
    let english = result(&picker, "🔥").is_some();
    search.set_text(name);
    let named =
        result(&picker, "🔥").is_some_and(|cell| cell.tooltip_text().as_deref() == Some(name));
    h.check(
        "reaction_picker_language",
        english && named,
        format!(
            "{} desktop finds {name} from the Hearts category",
            say.language()
        ),
    );
    h.screenshot("reaction-picker-search").await;
    // Enter picks the top result. Exercise the default action, not a duplicate dispatch in the test.
    if h.input() {
        h.key("Return").await;
    } else {
        search.emit_by_name::<()>("activate", &[]);
    }
    let reaction_is = |expected: Option<&str>| {
        world
            .messages
            .borrow()
            .get(&incoming.id)
            .and_then(|message| message.reactions.as_ref())
            .is_some_and(|reactions| {
                reactions
                    .iter()
                    .find(|reaction| reaction.user_id == world.dora)
                    .map(|reaction| reaction.emoji.as_str())
                    == expected
            })
    };
    let added = h.wait_until(WAIT, || reaction_is(Some("🔥"))).await;
    h.check(
        "reaction_picker_sets_reaction",
        added,
        "Enter sends the exact Fire emoji to the existing reaction endpoint",
    );
    h.screenshot("reaction-picker-selected").await;

    let Some(picker) = open(h, window, page, incoming).await else {
        h.check(
            "reaction_picker_reopen",
            false,
            "picker did not reopen after selection",
        );
        return;
    };
    h.check(
        "reaction_picker_reopen",
        heading(&picker) == say.get("Recent emoji") && result(&picker, "🔥").is_some(),
        "reopening shows Fire in the account's recent choices",
    );
    h.screenshot("reaction-picker-recent").await;
    let Some(search) = find::<gtk::SearchEntry>(&picker).into_iter().next() else {
        h.check(
            "reaction_picker_replaces_reaction",
            false,
            "no search entry after reopening",
        );
        picker.popdown();
        return;
    };
    let route = format!("/messages/{}/reaction", incoming.id);
    let reaction_requests = || {
        world
            .fake
            .requests()
            .iter()
            .filter(|request| request.ends_with(&route))
            .count()
    };
    let before = reaction_requests();
    let mut stayed_open = true;
    for text in ["", "   "] {
        search.set_text(text);
        if h.input() {
            h.key("Return").await;
        } else {
            search.emit_by_name::<()>("activate", &[]);
        }
        stayed_open &= picker.is_mapped();
    }
    h.check(
        "reaction_picker_blank_enter",
        stayed_open && reaction_is(Some("🔥")) && reaction_requests() == before,
        "Enter on empty and whitespace queries keeps the picker, the existing Fire reaction and the request count",
    );
    search.set_text("  :HEART:  ");
    if let Some(heart) = result(&picker, "❤️") {
        if h.input() {
            h.click_middle(window, &heart).await;
        } else {
            heart.activate();
        }
    }
    let replaced = h.wait_until(WAIT, || reaction_is(Some("❤️"))).await;
    h.check(
        "reaction_picker_replaces_reaction",
        replaced && reaction_requests() == before + 1,
        "one activation of the English shortcode replaces Fire with the exact red heart",
    );
    h.screenshot("reaction-picker-replaced").await;

    if let Some(picker) = open(h, window, page, incoming).await {
        let before = reaction_requests();
        if let Some(heart) = result(&picker, "❤️") {
            if h.input() {
                h.click_middle(window, &heart).await;
            } else {
                heart.activate();
            }
        }
        let removed = h.wait_until(WAIT, || reaction_is(None)).await;
        h.check(
            "reaction_picker_removes_reaction",
            removed && reaction_requests() == before + 1,
            "picking the current reaction again removes it",
        );
        h.screenshot("reaction-picker-removed").await;
    } else {
        h.check(
            "reaction_picker_removes_reaction",
            false,
            "picker did not reopen for removal",
        );
    }
    if h.input() {
        let mut selected = None;
        if let Some(picker) = open(h, window, page, incoming).await
            && let Some(search) = find::<gtk::SearchEntry>(&picker).into_iter().next()
        {
            search.set_text("fire");
            if let Some(cell) = grid(&picker).and_then(|grid| grid.child_at_index(1))
                && let Some(label) = cell.child().and_downcast::<gtk::Label>()
            {
                let before = reaction_requests();
                let navigated = h.key("Tab").await && h.key("Right").await && cell.has_focus();
                h.screenshot("reaction-picker-second-focused").await;
                let emoji = label.text();
                if navigated
                    && h.key("Return").await
                    && h.wait_until(WAIT, || reaction_is(Some(emoji.as_str())))
                        .await
                    && reaction_requests() == before + 1
                    && !picker.is_mapped()
                    && message_has_focus(page, window, incoming.id)
                {
                    selected = Some(emoji);
                }
            }
            if picker.is_mapped() {
                picker.popdown();
            }
        }
        h.check(
            "reaction_picker_focused_result_enter",
            selected.is_some(),
            format!("Tab then Right then Enter applies the focused second result to message {} and restores its focus (picked {selected:?})", incoming.id),
        );
        h.screenshot("reaction-picker-second-applied").await;

        let mut removed = false;
        if let Some(emoji) = selected
            && let Some(picker) = open(h, window, page, incoming).await
        {
            let before = reaction_requests();
            // The picked result heads Recents. Tab should focus that cell.
            removed = h.key("Tab").await
                && result(&picker, emoji.as_str()).is_some_and(|cell| cell.has_focus())
                && h.key("space").await
                && h.wait_until(WAIT, || reaction_is(None)).await
                && reaction_requests() == before + 1
                && !picker.is_mapped();
            if picker.is_mapped() {
                picker.popdown();
            }
        }
        h.check(
            "reaction_picker_focused_result_space",
            removed,
            "Space on the focused recent result removes that reaction",
        );
    }
    if let Some(picker) = open(h, window, page, incoming).await {
        if h.input() {
            h.key("Escape").await;
        } else {
            picker.popdown();
        }
        let restored = h
            .wait_until(SHORT, || {
                !picker.is_mapped() && message_has_focus(page, window, incoming.id)
            })
            .await;
        h.check(
            "reaction_picker_dismisses",
            restored,
            "the picker closes and returns focus to the same message row",
        );
        h.screenshot("reaction-picker-dismissed").await;
    } else {
        h.check(
            "reaction_picker_dismisses",
            false,
            "picker did not reopen to check dismissal",
        );
    }
}
