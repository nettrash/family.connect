//! Keeping the session awake (shell-design §10.9, §14.5, §22) — the inhibit wrapper P3's
//! recorders and P6's calls hold. `gtk::Application::inhibit` talks to `org.gnome.SessionManager`
//! itself (or the Inhibit portal), so this is GTK's call made balanced: an [`AwakeHold`] uninhibits
//! when it is dropped, exactly once.
//!
//! | While | Flags | The reason GNOME shows (the apps' catalogue) |
//! | --- | --- | --- |
//! | a voice message records | `IDLE` | "Recording a voice message" |
//! | a video message records | `IDLE` | "Recording video" |
//! | a call, from the answer (Connecting) to the end | `IDLE` + `SUSPEND` | "Voice call" / "Video call" |
//!
//! An idle timer that locks the session would stop a recording (§10.8, Windows' `KeepAwake`); a
//! call must outlive the screen saver. A cookie of 0 means no inhibitor answered (measured in the
//! container: no session manager, §1.6) and is logged once per process.

use std::cell::Cell;

use fc_core::text::string_catalog::StringCatalog;
use gtk::prelude::*;

/// Why the session must stay awake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Awake {
    VoiceMessage,
    VideoMessage,
    Call { video: bool },
}

impl Awake {
    /// What GTK is asked to inhibit.
    pub fn flags(self) -> gtk::ApplicationInhibitFlags {
        match self {
            Awake::VoiceMessage | Awake::VideoMessage => gtk::ApplicationInhibitFlags::IDLE,
            Awake::Call { .. } => {
                gtk::ApplicationInhibitFlags::IDLE | gtk::ApplicationInhibitFlags::SUSPEND
            }
        }
    }

    /// The reason the session manager shows, in the reader's language.
    pub fn reason(self, say: &dyn StringCatalog) -> String {
        match self {
            Awake::VoiceMessage => say.get("Recording a voice message"),
            Awake::VideoMessage => say.get("Recording video"),
            Awake::Call { video: false } => say.get("Voice call"),
            Awake::Call { video: true } => say.get("Video call"),
        }
        .to_string()
    }
}

thread_local! {
    /// "No inhibitor answered" has been logged.
    static WARNED: Cell<bool> = const { Cell::new(false) };
}

/// The session kept awake until this is dropped.
#[derive(Debug)]
#[must_use = "the session is kept awake only while the hold lives"]
pub struct AwakeHold {
    app: gtk::Application,
    cookie: u32,
    why: Awake,
}

impl AwakeHold {
    /// Whether an inhibitor took it (a cookie other than 0).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the desktop self-test reads it")
    )]
    pub fn held(&self) -> bool {
        self.cookie != 0
    }
}

/// Keep the session awake for `why`, tied to `window` (the session manager names the window).
pub fn hold(
    app: &impl IsA<gtk::Application>,
    window: Option<&gtk::Window>,
    why: Awake,
    say: &dyn StringCatalog,
) -> AwakeHold {
    let app: gtk::Application = app.clone().upcast();
    let cookie = app.inhibit(window, why.flags(), Some(&why.reason(say)));
    if cookie == 0 {
        if !WARNED.with(|warned| warned.replace(true)) {
            crate::fc_info!("keep-awake: no inhibitor answered ({why:?})");
        }
    } else {
        crate::fc_debug!("keep-awake: held ({why:?})");
    }
    AwakeHold { app, cookie, why }
}

impl Drop for AwakeHold {
    fn drop(&mut self) {
        if self.cookie != 0 {
            self.app.uninhibit(self.cookie);
            crate::fc_debug!("keep-awake: released ({:?})", self.why);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_core::text::string_catalog::EnglishCatalog;

    #[test]
    fn recordings_keep_the_screen_on_and_calls_the_machine_too() {
        let say = EnglishCatalog::new();
        assert_eq!(
            Awake::VoiceMessage.flags(),
            gtk::ApplicationInhibitFlags::IDLE
        );
        assert_eq!(
            Awake::VideoMessage.flags(),
            gtk::ApplicationInhibitFlags::IDLE
        );
        let call = Awake::Call { video: true }.flags();
        assert!(call.contains(gtk::ApplicationInhibitFlags::IDLE));
        assert!(call.contains(gtk::ApplicationInhibitFlags::SUSPEND));
        assert_eq!(
            Awake::VoiceMessage.reason(&say),
            "Recording a voice message"
        );
        assert_eq!(Awake::VideoMessage.reason(&say), "Recording video");
        assert_eq!(Awake::Call { video: false }.reason(&say), "Voice call");
        assert_eq!(Awake::Call { video: true }.reason(&say), "Video call");
    }
}
