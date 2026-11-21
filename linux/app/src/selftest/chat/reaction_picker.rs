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
) -> Option<(gtk::Popover, gtk::Button)> {
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
        find::<gtk::Popover>(&more)
            .into_iter()
            .find(|child| child.is_mapped() && child.widget_name() == "reaction-picker")
    };
    if !h.wait_until(SHORT, || picker().is_some()).await {
        return None;
    }
    Some((picker()?, more))
}

fn result(picker: &gtk::Popover, emoji: &str) -> Option<gtk::Button> {
    find::<gtk::FlowBox>(picker)
        .into_iter()
        .find(|w| w.widget_name() == "reaction-picker-grid")
        .and_then(|flow| {
            find::<gtk::Button>(&flow)
                .into_iter()
                .find(|w| w.label().as_deref() == Some(emoji))
        })
}

fn heading(picker: &gtk::Popover) -> String {
    find::<gtk::Label>(picker)
        .into_iter()
        .find(|w| w.widget_name() == "reaction-picker-heading")
        .map(|label| label.text().to_string())
        .unwrap_or_default()
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
    let Some((picker, _)) = open(h, window, page, incoming).await else {
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
    if let Some(tab) = find::<gtk::ToggleButton>(&picker)
        .into_iter()
        .find(|tab| tab.tooltip_text().as_deref() == Some(say.get("Hearts")))
    {
        tab.emit_clicked();
    }
    h.check(
        "reaction_picker_categories",
        heading(&picker) == say.get("Hearts") && result(&picker, "❤️").is_some(),
        "Hearts shortcut opens its category",
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

    let (query, name) = match say.language() {
        "ru" => ("огонь", "огонь"),
        _ => ("fire", "fire"),
    };
    search.set_text(query);
    let named =
        result(&picker, "🔥").is_some_and(|button| button.tooltip_text().as_deref() == Some(name));
    h.check(
        "reaction_picker_language",
        named,
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

    let Some((picker, _)) = open(h, window, page, incoming).await else {
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
    let search = find::<gtk::SearchEntry>(&picker)
        .into_iter()
        .next()
        .unwrap();
    search.set_text("  :HEART:  ");
    if let Some(heart) = result(&picker, "❤️") {
        heart.emit_clicked();
    }
    let replaced = h.wait_until(WAIT, || reaction_is(Some("❤️"))).await;
    h.check(
        "reaction_picker_replaces_reaction",
        replaced,
        "the English shortcode replaces Fire with the exact red heart",
    );
    h.screenshot("reaction-picker-replaced").await;

    if let Some((picker, _)) = open(h, window, page, incoming).await {
        if let Some(heart) = result(&picker, "❤️") {
            heart.emit_clicked();
        }
        let removed = h.wait_until(WAIT, || reaction_is(None)).await;
        h.check(
            "reaction_picker_removes_reaction",
            removed,
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
    if let Some((picker, anchor)) = open(h, window, page, incoming).await {
        if h.input() {
            h.key("Escape").await;
        } else {
            picker.popdown();
        }
        let row = page
            .timeline()
            .row(&conversation_rows::message_key(incoming.id));
        let restored = h
            .wait_until(SHORT, || {
                !picker.is_mapped()
                    && (anchor.has_focus()
                        || gtk::prelude::RootExt::focus(window).is_some_and(|focus| {
                            row.as_ref()
                                .is_some_and(|row| focus == *row || focus.is_ancestor(row))
                        }))
            })
            .await;
        h.check(
            "reaction_picker_dismisses",
            restored,
            "the picker closes and returns focus to the message or its More reactions button",
        );
        h.screenshot("reaction-picker-dismissed").await;
        if let Some(menu) = anchor
            .ancestor(gtk::PopoverMenu::static_type())
            .and_downcast::<gtk::PopoverMenu>()
        {
            menu.popdown();
        }
    } else {
        h.check(
            "reaction_picker_dismisses",
            false,
            "picker did not reopen to check dismissal",
        );
    }
}
