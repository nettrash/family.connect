//! **The interfaces S0 freezes** (shell-design §W.3). Six packages implement against these in
//! parallel without talking to each other; a change here is a request to S0's owner (W.1), never
//! an edit.
//!
//! | Module | Interface | Implemented by | Used by | S0's stand-in |
//! | --- | --- | --- | --- | --- |
//! | [`actions`] | every action name, scope, target type and accelerator | S0 registers the app's and the main window's; P1 the conversation windows' and `win.message-*` | everyone, P5's notifications and tray | — |
//! | [`routes`] | [`ChatRoutes`](routes::ChatRoutes) | P1 | the actions, notifications | does nothing |
//! | [`routes`] | [`FamilyRoutes`](routes::FamilyRoutes) | P4 | the actions, the tray menu | does nothing |
//! | [`routes`] | [`CallRoutes`](routes::CallRoutes) | P6 | `app.answer-call`/`decline-call`/`return-to-call` | answers/declines on the engine, no window |
//! | [`routes`] | [`WindowPart`](routes::WindowPart) | P1, P2, P3, P4, P6 | the main window's frame | — |
//! | [`conversation`] | [`ConversationSlots`](conversation::ConversationSlots) | P2 (tile, sticker, rich, link card, location), P3 (voice, circle) | P1's bubbles | each block's fallback: an icon, or the source text |
//! | [`composer`] | [`ComposerSlots`](composer::ComposerSlots), [`ComposerRecording`](composer::ComposerRecording) | P3 | P1's composer | records nothing |
//! | [`composer`] | [`ComposerHost`](composer::ComposerHost) | P1 | P3 | — |
//! | [`media`] | [`Players`](media::Players) / [`Player`](media::Player), [`NowPlaying`](media::NowPlaying), [`Camera`](media::Camera) | P3 | P2 (the viewer), P3, P6 (the camera) | plays nothing; no camera |
//! | [`desktop`] | [`Notifier`](desktop::Notifier), [`TrayState`](desktop::TrayState), [`BadgeState`](desktop::BadgeState), [`HotKeyState`](desktop::HotKeyState), [`AutostartState`](desktop::AutostartState) | P5 | S0's composition, P4's Settings rows, P6's ringing | show nothing; no icon (so a close quits) |
//! | [`tokens`] | [`TokenVault`](tokens::TokenVault) under fc-core's `TokenStore` ([`VaultedTokens`](tokens::VaultedTokens)) | P5 (the Secret Service) | S0's composition, P4's Server row | in memory for this run |
//! | [`calls`] | fc-logic's `CallMedia` | P6 (`GstCallMedia`) | the call engine | [`NoCallMedia`](calls::NoCallMedia): no microphone, so a call ends at once |
//!
//! **How a package plugs in.** Each package writes `pub fn install(shell: &Rc<Shell>)` in its own
//! module and, in it, puts its implementations into the [`Slot`]s of [`crate::shell::Shell`]
//! (`shell.notifier.install(Rc::new(…))`), registers its [`WindowPart`](routes::WindowPart)s
//! (`shell.add_window_part(…)`), its CSS (`crate::theme::register`) and its self-test group. The
//! integration adds ONE line per package to `app::install_packages` (the place is marked there) and
//! the package's `mod` line to `main.rs` — nothing else in S0's files changes. `install` runs in
//! `startup` before the main window is built and before any server is opened, so a slot is read
//! after every package has filled it.

pub mod actions;
pub mod calls;
pub mod composer;
pub mod conversation;
pub mod desktop;
pub mod media;
pub mod routes;
pub mod tokens;

use std::cell::RefCell;
use std::rc::Rc;

/// One replaceable service: S0's stand-in until a package [`install`](Slot::install)s its own.
/// Read it with [`get`](Slot::get) every time it is needed — never keep the `Rc` past one use, or
/// an installation made later is missed.
pub struct Slot<T: ?Sized>(RefCell<Rc<T>>);

impl<T: ?Sized> Slot<T> {
    pub fn new(initial: Rc<T>) -> Slot<T> {
        Slot(RefCell::new(initial))
    }

    /// The service now installed.
    pub fn get(&self) -> Rc<T> {
        Rc::clone(&self.0.borrow())
    }

    /// Replaces the service. Packages call this from their `install`.
    pub fn install(&self, service: Rc<T>) {
        *self.0.borrow_mut() = service;
    }
}

impl<T: ?Sized> std::fmt::Debug for Slot<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Slot")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Speaks {
        fn word(&self) -> &'static str;
    }
    struct Quiet;
    struct Loud;
    impl Speaks for Quiet {
        fn word(&self) -> &'static str {
            "…"
        }
    }
    impl Speaks for Loud {
        fn word(&self) -> &'static str {
            "hello"
        }
    }

    #[test]
    fn a_slot_answers_its_stand_in_until_a_package_installs_its_own() {
        let slot: Slot<dyn Speaks> = Slot::new(Rc::new(Quiet));
        assert_eq!(slot.get().word(), "…");
        slot.install(Rc::new(Loud));
        assert_eq!(slot.get().word(), "hello");
    }
}
