//! What the Call window shows and what the shell does around it — pure functions of the engine's
//! [`CallState`] and of what the media has (a far-side picture, a camera), so every rule is
//! unit-tested without a display (shell-design §W.1, "Logic placement"). The window
//! (`call_window`) only draws a [`CallView`]; the hub (`hub`) asks the other rules when to present
//! it, when the desktop is told a call rings, and when the machine is kept awake.
//!
//! The rules are the Mac's `CallView` and `CallManager` (ios/FamilyConnect/Views/CallView.swift),
//! with the Linux design's words (shell-design §14.3, §14.4):
//!
//! - **Who**: the peer's name as the roster holds it, or "Someone" (Windows' `CallCardView`, the
//!   notifications' rule) — the window's TITLE too: "The window is the person" (the Mac's
//!   `navigationTitle(calls.peerName)`).
//! - **The picture or the face**: a video call shows the far side's picture once there is one,
//!   and then a compact pill carries the name and the status; until then — and for the whole of a
//!   voice call — the avatar, the name and the status line in the middle (the Mac's
//!   `showsCompactIdentity`: exactly one of the two is on screen).
//! - **The self-view** only while this device's camera is on and exists — "a black tile saying
//!   nothing is worse than no tile".
//! - **The discs**: ringing in, Decline and Accept (a camera on Accept for a video call — "the
//!   glyph says what answering opens"); in a call, Mute/Unmute, the camera (video calls) and Hang
//!   Up; once over, none — the reason is on screen and the window goes by itself.

use fc_core::protocol::MemberDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::calls::{CallStage, CallState, call_text};
use time::OffsetDateTime;

/// What a disc does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Decline,
    Accept,
    Mute,
    Camera,
    HangUp,
}

impl Role {
    pub const ALL: [Role; 5] = [
        Role::Decline,
        Role::Accept,
        Role::Mute,
        Role::Camera,
        Role::HangUp,
    ];
}

/// A disc's colour (the Mac's `CallControlButton`: a toggle at rest takes the scheme's colour, an
/// ACTIVE toggle — muted, camera off — the accent; hang up and decline are red, accept green).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Look {
    Toggle,
    Active,
    HangUp,
    Accept,
}

/// One disc of the control row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disc {
    pub role: Role,
    /// A symbolic icon of the Adwaita/Yaru theme.
    pub icon: &'static str,
    /// What it does — under the disc, its accessible name and its tooltip.
    pub label: String,
    pub look: Look,
    pub sensitive: bool,
}

/// Which row of discs is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Row {
    Incoming,
    Active,
    /// Nothing to do: the reason is on screen. The row keeps its height, so the middle does not
    /// jump the instant a call ends (the Mac's ended placeholder).
    Ended,
}

impl Row {
    /// The page's name in the window's stack.
    pub fn name(self) -> &'static str {
        match self {
            Row::Incoming => "incoming",
            Row::Active => "active",
            Row::Ended => "ended",
        }
    }
}

/// Everything the Call window draws for one moment of a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallView {
    /// The window's title: the person.
    pub title: String,
    pub name: String,
    /// `call_text::status_line`: Calling…, Ringing…, Incoming call, Connecting…, the duration, or
    /// why it ended.
    pub status: String,
    /// A video call: the window is dark, deterministically (the Mac's FaceTime rule — a black
    /// picture surface under light text was black on black for all of connecting).
    pub video: bool,
    /// The far side's picture fills the window (and the pill carries the name).
    pub picture: bool,
    /// This device's camera in the corner.
    pub self_view: bool,
    /// "Camera access is off — the call is voice-only for you." under the status line: a video call
    /// up without a camera of ours (refused, absent, busy) — a footnote, never an end reason.
    pub camera_note: bool,
    pub row: Row,
    /// The discs of [`row`](Self::row), in order.
    pub discs: Vec<Disc>,
    /// The status counts seconds: the window redraws it once a second.
    pub ticking: bool,
}

/// Whoever the call is with, as the roster names them — "Someone" when it cannot (a member who
/// joined while this device slept).
pub fn peer_name(member: Option<&MemberDto>, say: &dyn StringCatalog) -> String {
    match member {
        Some(member) if !member.display_name.trim().is_empty() => member.display_name.clone(),
        _ => say.get("Someone").to_string(),
    }
}

/// The window for `call`: `has_picture` and `has_self_view` are what the media holds now
/// (`GstCallMedia::remote_video` / `self_view`).
pub fn view(
    call: &CallState,
    name: &str,
    has_picture: bool,
    has_self_view: bool,
    now: OffsetDateTime,
    say: &dyn StringCatalog,
) -> CallView {
    let over = call.stage == CallStage::Ended;
    let up = matches!(call.stage, CallStage::Connecting | CallStage::Talking);
    let picture = call.video && has_picture && !over;
    let row = match call.stage {
        CallStage::Incoming => Row::Incoming,
        CallStage::Ended => Row::Ended,
        _ => Row::Active,
    };
    let discs = match row {
        Row::Incoming => vec![
            Disc {
                role: Role::Decline,
                icon: "call-stop-symbolic",
                label: say.get("Decline").to_string(),
                look: Look::HangUp,
                sensitive: true,
            },
            Disc {
                role: Role::Accept,
                icon: if call.video {
                    "camera-video-symbolic"
                } else {
                    "call-start-symbolic"
                },
                label: say.get("Accept").to_string(),
                look: Look::Accept,
                sensitive: true,
            },
        ],
        Row::Active => {
            let mut discs = vec![Disc {
                role: Role::Mute,
                icon: if call.muted {
                    "microphone-disabled-symbolic"
                } else {
                    "audio-input-microphone-symbolic"
                },
                label: if call.muted {
                    say.get("Unmute").to_string()
                } else {
                    say.get("Mute").to_string()
                },
                look: if call.muted {
                    Look::Active
                } else {
                    Look::Toggle
                },
                sensitive: true,
            }];
            if call.video {
                discs.push(Disc {
                    role: Role::Camera,
                    icon: if call.camera {
                        "camera-video-symbolic"
                    } else {
                        "camera-disabled-symbolic"
                    },
                    label: if call.camera {
                        say.get("Turn camera off").to_string()
                    } else {
                        say.get("Turn camera on").to_string()
                    },
                    // ON is the neutral state, OFF the accented one (the mute pair's rule).
                    look: if call.camera {
                        Look::Toggle
                    } else {
                        Look::Active
                    },
                    // Nothing to turn on once the call is up without a camera of ours.
                    sensitive: !up || has_self_view,
                });
            }
            discs.push(Disc {
                role: Role::HangUp,
                icon: "call-stop-symbolic",
                label: say.get("Hang Up").to_string(),
                look: Look::HangUp,
                sensitive: true,
            });
            discs
        }
        Row::Ended => Vec::new(),
    };
    CallView {
        title: name.to_string(),
        name: name.to_string(),
        status: call_text::status_line(call, now, say),
        video: call.video,
        picture,
        self_view: call.video && call.camera && has_self_view && !over,
        camera_note: call.video && up && !has_self_view,
        row,
        discs,
        ticking: call.stage == CallStage::Talking,
    }
}

/// Whether a change of the call brings its window forward. The Mac opens the Call window on
/// every change of its phase (RootView's `onChange(of: calls.phase)`), so a window the person
/// closed comes back when the call moves on; and a NEW call always comes forward. Linux keeps
/// that for every live stage, but an ENDING only updates a window that is up — a closed window is
/// not brought back just to say goodbye (and §14.6: a Decline from the notification shows
/// nothing).
pub fn presents(previous: Option<&(String, CallStage)>, call: &CallState, visible: bool) -> bool {
    let changed = previous.is_none_or(|(id, stage)| *id != call.call_id || *stage != call.stage);
    changed && (call.stage != CallStage::Ended || visible)
}

/// The call the desktop is told rings (the notification with Decline/Answer, the top-bar icon's
/// attention, the dock's urgency): one ringing HERE, and only while it rings.
pub fn ringing(call: Option<&CallState>) -> Option<&str> {
    call.filter(|call| call.stage == CallStage::Incoming)
        .map(|call| call.call_id.as_str())
}

/// The ring's notification: "Incoming call" (or "Incoming video call") over "<Name> is calling" —
/// the Mac's `announceIncomingCall`, word for word. Who calls, never anything else.
pub fn ring_words(call: &CallState, name: &str, say: &dyn StringCatalog) -> (String, String) {
    let title = if call.video {
        say.get("Incoming video call").to_string()
    } else {
        say.get("Incoming call").to_string()
    };
    (title, say.format("%@ is calling", &[&name]))
}

/// Whether the machine is kept from idling and suspending (§14.5, §10.9): from the answer until
/// the call is over — a ring is short, and lasts no longer than the server's 45 s.
pub fn keeps_awake(call: Option<&CallState>) -> bool {
    call.is_some_and(|call| matches!(call.stage, CallStage::Connecting | CallStage::Talking))
}

/// Whether `app.answer-call(call_id)` / `app.decline-call(call_id)` applies: only to the call that
/// rings here NOW — a stale notification's id, a call answered meanwhile, one already over, do
/// nothing.
pub fn applies(call: Option<&CallState>, call_id: &str) -> bool {
    call.is_some_and(|call| call.call_id == call_id && call.stage == CallStage::Incoming)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_core::text::string_catalog::EnglishCatalog;
    use time::macros::datetime;

    fn call(stage: CallStage, video: bool, outgoing: bool) -> CallState {
        CallState {
            camera: video,
            ..CallState::new("call-1", 9, 2, outgoing, video, stage)
        }
    }

    fn labels(view: &CallView) -> Vec<&str> {
        view.discs.iter().map(|disc| disc.label.as_str()).collect()
    }

    const NOW: OffsetDateTime = datetime!(2026-10-29 12:00:00 UTC);

    #[test]
    fn a_ring_offers_decline_and_accept_with_the_kind_on_accept() {
        let say = EnglishCatalog::new();
        let voice = view(
            &call(CallStage::Incoming, false, false),
            "Bob",
            false,
            false,
            NOW,
            &say,
        );
        assert_eq!(voice.row, Row::Incoming);
        assert_eq!(labels(&voice), ["Decline", "Accept"]);
        assert_eq!(voice.discs[0].look, Look::HangUp);
        assert_eq!(voice.discs[1].look, Look::Accept);
        assert_eq!(voice.discs[1].icon, "call-start-symbolic");
        assert_eq!(voice.status, "Incoming call");
        assert_eq!(voice.title, "Bob");
        let video = view(
            &call(CallStage::Incoming, true, false),
            "Bob",
            false,
            false,
            NOW,
            &say,
        );
        assert_eq!(video.discs[1].icon, "camera-video-symbolic");
        assert_eq!(video.status, "Incoming video call");
        assert!(video.video && !video.picture && !video.camera_note);
    }

    #[test]
    fn a_call_offers_mute_the_camera_on_video_and_hang_up() {
        let say = EnglishCatalog::new();
        let voice = view(
            &call(CallStage::Dialling, false, true),
            "Bob",
            false,
            false,
            NOW,
            &say,
        );
        assert_eq!(voice.row, Row::Active);
        assert_eq!(labels(&voice), ["Mute", "Hang Up"]);
        assert_eq!(voice.status, "Calling…");
        let ringing = view(
            &call(CallStage::Ringing, false, true),
            "Bob",
            false,
            false,
            NOW,
            &say,
        );
        assert_eq!(ringing.status, "Ringing…");
        let video = view(
            &call(CallStage::Connecting, true, true),
            "Bob",
            false,
            true,
            NOW,
            &say,
        );
        assert_eq!(labels(&video), ["Mute", "Turn camera off", "Hang Up"]);
        assert_eq!(video.status, "Connecting…");
        assert!(video.self_view && video.discs.iter().all(|disc| disc.sensitive));
    }

    #[test]
    fn mute_and_camera_off_are_the_accented_states_and_say_what_they_do() {
        let say = EnglishCatalog::new();
        let held = CallState {
            muted: true,
            camera: false,
            ..call(CallStage::Talking, true, true)
        };
        let drawn = view(&held, "Bob", true, true, NOW, &say);
        assert_eq!(labels(&drawn), ["Unmute", "Turn camera on", "Hang Up"]);
        assert_eq!(drawn.discs[0].look, Look::Active);
        assert_eq!(drawn.discs[0].icon, "microphone-disabled-symbolic");
        assert_eq!(drawn.discs[1].look, Look::Active);
        assert_eq!(drawn.discs[1].icon, "camera-disabled-symbolic");
        // The camera off hides the self-view; the far side's picture stays.
        assert!(!drawn.self_view && drawn.picture);
    }

    #[test]
    fn the_picture_replaces_the_face_only_on_a_video_call_that_has_one() {
        let say = EnglishCatalog::new();
        let video = call(CallStage::Talking, true, true);
        assert!(!view(&video, "Bob", false, true, NOW, &say).picture);
        assert!(view(&video, "Bob", true, true, NOW, &say).picture);
        let voice = call(CallStage::Talking, false, true);
        let drawn = view(&voice, "Bob", true, true, NOW, &say);
        assert!(!drawn.picture && !drawn.self_view && !drawn.video);
    }

    #[test]
    fn a_video_call_up_without_a_camera_says_so_and_has_no_camera_to_turn_on() {
        let say = EnglishCatalog::new();
        let cameraless = CallState {
            camera: false,
            ..call(CallStage::Talking, true, true)
        };
        let drawn = view(&cameraless, "Bob", true, false, NOW, &say);
        assert!(drawn.camera_note && !drawn.self_view);
        let camera = drawn
            .discs
            .iter()
            .find(|disc| disc.role == Role::Camera)
            .unwrap();
        assert!(!camera.sensitive);
        // Before the media is up nothing is known yet.
        let dialling = view(
            &call(CallStage::Dialling, true, true),
            "Bob",
            false,
            false,
            NOW,
            &say,
        );
        assert!(!dialling.camera_note);
    }

    #[test]
    fn the_duration_ticks_from_the_answer_and_the_end_says_why() {
        let say = EnglishCatalog::new();
        let talking = CallState {
            answered_at: Some(datetime!(2026-10-29 11:58:55 UTC)),
            ..call(CallStage::Talking, false, true)
        };
        let drawn = view(&talking, "Bob", false, false, NOW, &say);
        assert_eq!(drawn.status, "1:05");
        assert!(drawn.ticking);
        let declined = CallState {
            ended_reason: Some("decline".into()),
            ..call(CallStage::Ended, false, true)
        };
        let drawn = view(&declined, "Bob", true, true, NOW, &say);
        assert_eq!(drawn.status, "Declined");
        assert_eq!(drawn.row, Row::Ended);
        assert!(drawn.discs.is_empty() && !drawn.ticking && !drawn.picture && !drawn.self_view);
        let mine = CallState {
            ended_reason: Some("hangup".into()),
            ..call(CallStage::Ended, false, false)
        };
        assert_eq!(
            view(&mine, "Bob", false, false, NOW, &say).status,
            "Call ended"
        );
    }

    #[test]
    fn a_name_the_roster_lacks_is_someone() {
        let say = EnglishCatalog::new();
        let bob = MemberDto {
            id: 2,
            display_name: "Bob".into(),
            ..MemberDto::default()
        };
        assert_eq!(peer_name(Some(&bob), &say), "Bob");
        let blank = MemberDto {
            display_name: "  ".into(),
            ..bob
        };
        assert_eq!(peer_name(Some(&blank), &say), "Someone");
        assert_eq!(peer_name(None, &say), "Someone");
    }

    #[test]
    fn a_new_call_and_every_live_stage_come_forward_but_an_ending_only_updates() {
        let ringing = call(CallStage::Ringing, false, true);
        assert!(presents(None, &ringing, false));
        let before = ("call-1".to_string(), CallStage::Dialling);
        assert!(presents(Some(&before), &ringing, false));
        let same = ("call-1".to_string(), CallStage::Ringing);
        assert!(!presents(Some(&same), &ringing, true));
        let other = ("call-0".to_string(), CallStage::Ringing);
        assert!(presents(Some(&other), &ringing, false));
        let ended = call(CallStage::Ended, false, true);
        let talking = ("call-1".to_string(), CallStage::Talking);
        assert!(!presents(Some(&talking), &ended, false));
        assert!(presents(Some(&talking), &ended, true));
    }

    #[test]
    fn only_a_call_ringing_here_rings_on_the_desktop_and_takes_the_routes() {
        let incoming = call(CallStage::Incoming, false, false);
        assert_eq!(ringing(Some(&incoming)), Some("call-1"));
        assert_eq!(ringing(Some(&call(CallStage::Ringing, false, true))), None);
        assert_eq!(
            ringing(Some(&call(CallStage::Connecting, false, false))),
            None
        );
        assert_eq!(ringing(None), None);
        assert!(applies(Some(&incoming), "call-1"));
        assert!(!applies(Some(&incoming), "call-0"));
        assert!(!applies(
            Some(&call(CallStage::Talking, false, false)),
            "call-1"
        ));
        assert!(!applies(
            Some(&call(CallStage::Ended, false, false)),
            "call-1"
        ));
        assert!(!applies(None, "call-1"));
    }

    #[test]
    fn the_ring_says_who_and_what_kind_never_more() {
        let say = EnglishCatalog::new();
        assert_eq!(
            ring_words(&call(CallStage::Incoming, false, false), "Bob", &say),
            ("Incoming call".to_string(), "Bob is calling".to_string())
        );
        assert_eq!(
            ring_words(&call(CallStage::Incoming, true, false), "Bob", &say).0,
            "Incoming video call"
        );
    }

    #[test]
    fn the_machine_stays_awake_from_the_answer_to_the_end() {
        for (stage, awake) in [
            (CallStage::Dialling, false),
            (CallStage::Ringing, false),
            (CallStage::Incoming, false),
            (CallStage::Connecting, true),
            (CallStage::Talking, true),
            (CallStage::Ended, false),
        ] {
            assert_eq!(
                keeps_awake(Some(&call(stage, false, true))),
                awake,
                "{stage:?}"
            );
        }
        assert!(!keeps_awake(None));
    }
}
