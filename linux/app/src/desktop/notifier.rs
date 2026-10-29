//! Notifications (shell-design §15, §14.4, §14.6) — HOW; S0's `shell::Attention` decides WHEN
//! (`NotificationRules` on live frames only).
//!
//! **Who, never what.** A [`Toast`] cannot carry message text (fc-logic): its title names who and
//! where, its body is "New message" / "New note". This module adds nothing to it — no picture, no
//! avatar, the app's own icon (`me.nettrash.FamilyConnect`).
//!
//! **How.** `gio::Application::send_notification` (GNotification): GLib picks the backend — GNOME
//! Shell's `org.gtk.Notifications` on the desktop, the `org.freedesktop.Notifications` server
//! elsewhere (measured in the container, §1.6), the portal inside a sandbox. The tag is the
//! notification's id, so a second about the same chat REPLACES the first and opening the chat can
//! withdraw it. The default action and its target are `toast_arguments::for_toast`'s:
//! `app.open-chat(x)`, `app.open-board(x)` — and a wall whose family is not known is
//! `app.open-board(0)` (the action always takes an `x`; 0 reads back as the wall shown). With the
//! GNOME backend the click comes back as `org.freedesktop.Application.ActivateAction` on the app's
//! bus name, even after a quit through D-Bus activation (§2.2); the targets are untrusted and S0
//! reads them through `notification_target::parse`.
//!
//! **A ringing call** (§14.4): "<Name>" over "Incoming call", urgent, category `call.incoming`,
//! buttons Decline (`app.decline-call(s)`) and Answer (`app.answer-call(s)`) — on Wayland the
//! reliable door, since a call window raised by the socket alone may not take focus (D16). Its
//! default action is `app.return-to-call` (the Call window, which rings with the same two
//! buttons). While any call rings the top-bar icon says `NeedsAttention` and the dock's entry is
//! `urgent`.
//!
//! **When nothing shows** (§15.5): GNOME's per-app switch and Do Not Disturb are not readable by a
//! host app, and GLib logs a failed `Notify` itself; Settings says where the switch is (P4).
//!
//! **Logs** carry the tag (`chat-42`, `board`, `call-<id>`) and what happened — never a title.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use fc_core::text::string_catalog::StringCatalog;
use fc_logic::notification_target::TargetValue;
use fc_logic::notifications::Toast;
use fc_logic::toast_arguments;
use gio::prelude::*;

use crate::app::APP_ID;
use crate::seams::actions;
use crate::seams::desktop::Notifier;
use crate::services::catalogue::Catalogue;
use crate::shell::Shell;

/// A message or a note's category (the freedesktop specification's).
pub const CATEGORY_MESSAGE: &str = "im.received";
/// A ringing call's.
pub const CATEGORY_CALL: &str = "call.incoming";

/// The tag of a call's notification.
pub fn call_tag(call_id: &str) -> String {
    format!("call-{call_id}")
}

/// A toast's default action and target: what `for_toast` writes, with a wall of no known family
/// as `0` (seams/desktop.rs).
pub fn default_action(toast: &Toast) -> (String, glib::Variant) {
    let arguments = toast_arguments::for_toast(toast);
    let target = match arguments.target {
        Some(TargetValue::Int64(id)) => id.to_variant(),
        Some(TargetValue::Str(ref text)) => text.to_variant(),
        Some(TargetValue::Other) | None => 0i64.to_variant(),
    };
    (arguments.detailed_action(), target)
}

/// A ringing call's buttons, in their order: (label, detailed action, target).
pub fn call_buttons(
    say: &dyn StringCatalog,
    call_id: &str,
) -> [(String, String, glib::Variant); 2] {
    [
        (
            say.get("Decline").to_string(),
            actions::DECLINE_CALL.detailed(),
            call_id.to_variant(),
        ),
        (
            say.get("Answer").to_string(),
            actions::ANSWER_CALL.detailed(),
            call_id.to_variant(),
        ),
    ]
}

/// The notifier (see the module's comment).
pub struct DesktopNotifier {
    app: adw::Application,
    say: Rc<Catalogue>,
    /// Calls ringing now.
    ringing: RefCell<BTreeSet<String>>,
}

impl DesktopNotifier {
    pub fn new(app: &adw::Application, say: Rc<Catalogue>) -> DesktopNotifier {
        DesktopNotifier {
            app: app.clone(),
            say,
            ringing: RefCell::new(BTreeSet::new()),
        }
    }

    fn base(title: &str, body: &str) -> gio::Notification {
        let notification = gio::Notification::new(title);
        notification.set_body(Some(body));
        notification.set_icon(&gio::ThemedIcon::new(APP_ID));
        notification
    }

    /// The icon's and the dock's "a call rings", from the set of ringing calls.
    fn tell_ringing(&self) {
        let ringing = !self.ringing.borrow().is_empty();
        if let Some(shell) = Shell::try_get() {
            shell.tray.get().set_ringing(ringing);
            shell.badge.get().set_urgent(ringing);
        }
    }
}

impl Notifier for DesktopNotifier {
    fn available(&self) -> bool {
        // GLib can always try — but with no session bus there is nothing to try on.
        self.app.dbus_connection().is_some()
    }

    fn show(&self, toast: &Toast) {
        let notification = DesktopNotifier::base(&toast.title, &toast.body);
        notification.set_priority(gio::NotificationPriority::High);
        notification.set_category(Some(CATEGORY_MESSAGE));
        let (action, target) = default_action(toast);
        notification.set_default_action_and_target_value(&action, Some(&target));
        self.app.send_notification(Some(&toast.tag), &notification);
        crate::fc_info!("notify: {} shown", toast.tag);
    }

    fn withdraw(&self, tag: &str) {
        self.app.withdraw_notification(tag);
        crate::fc_debug!("notify: {tag} withdrawn");
    }

    fn ring(&self, call_id: &str, title: &str, body: &str) {
        if call_id.is_empty() {
            return;
        }
        let notification = DesktopNotifier::base(title, body);
        notification.set_priority(gio::NotificationPriority::Urgent);
        notification.set_category(Some(CATEGORY_CALL));
        notification.set_default_action(&actions::RETURN_TO_CALL.detailed());
        for (label, action, target) in call_buttons(&*self.say, call_id) {
            notification.add_button_with_target_value(&label, &action, Some(&target));
        }
        let tag = call_tag(call_id);
        self.app.send_notification(Some(&tag), &notification);
        crate::fc_info!("notify: {tag} ringing");
        self.ringing.borrow_mut().insert(call_id.to_string());
        self.tell_ringing();
    }

    fn stop_ringing(&self, call_id: &str) {
        let tag = call_tag(call_id);
        self.app.withdraw_notification(&tag);
        if self.ringing.borrow_mut().remove(call_id) {
            crate::fc_info!("notify: {tag} stopped ringing");
        }
        self.tell_ringing();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_core::text::string_catalog::EnglishCatalog;
    use fc_logic::notifications::NotificationRules;

    #[test]
    fn a_chat_toast_opens_its_chat_and_a_wall_its_family_or_the_wall_shown() {
        let chat = Toast::new(
            &NotificationRules::chat_tag(42),
            "The Testers — Bob",
            "New message",
            Some(42),
        );
        let (action, target) = default_action(&chat);
        assert_eq!(action, "app.open-chat");
        assert_eq!(target.get::<i64>(), Some(42));
        let wall = Toast::new("board", "The Testers — Bob", "New note", None).in_family(Some(7));
        let (action, target) = default_action(&wall);
        assert_eq!(action, "app.open-board");
        assert_eq!(target.get::<i64>(), Some(7));
        // No family known: `app.open-board` still takes an `x`, and 0 is the wall shown.
        let shown = Toast::new("board", "Bob", "New note", None);
        let (action, target) = default_action(&shown);
        assert_eq!(action, "app.open-board");
        assert_eq!(target.get::<i64>(), Some(0));
        // And it reads back the way S0 parses it.
        assert_eq!(
            fc_logic::notification_target::parse(&action, Some(&crate::app::target_value(&target))),
            Some(fc_logic::notification_target::NotificationTarget::Open(
                fc_logic::notification_target::ToastTarget::board(None)
            ))
        );
    }

    #[test]
    fn a_ringing_call_offers_decline_then_answer_for_that_call_only() {
        let say = EnglishCatalog::new();
        let buttons = call_buttons(&say, "c-1");
        assert_eq!(buttons[0].0, "Decline");
        assert_eq!(buttons[0].1, "app.decline-call");
        assert_eq!(buttons[0].2.str(), Some("c-1"));
        assert_eq!(buttons[1].0, "Answer");
        assert_eq!(buttons[1].1, "app.answer-call");
        assert_eq!(buttons[1].2.str(), Some("c-1"));
        assert_eq!(call_tag("c-1"), "call-c-1");
    }
}
