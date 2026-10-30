//! `ToastArgumentsTests`' parsing cases, for a `GNotification`'s action and `GVariant` target
//! (shell-design §15.3–15.4, §14.6).
//!
//! Ported as parse cases: `AChatNotificationOpensItsChatAndTheWallsOpensTheWall` (the parse half;
//! writing the arguments, `ToastArguments.For`, is the shell's `set_default_action_and_target`),
//! `WhatTheAppDidNotWriteOpensNothing`, and the wall's family from `SeveralFamiliesTests`.
//! `TheIdIsWrittenTheSameWayInEveryLanguage` and `ANoteIsNewsExactlyWhenTheBadgeWouldCountIt` are
//! `toast_arguments_tests.rs`.

use fc_logic::notification_target::{
    ANSWER_CALL, DECLINE_CALL, NotificationTarget, OPEN_BOARD, OPEN_CHAT, TargetValue, ToastTarget,
    parse, positive_id,
};

fn text(value: &str) -> TargetValue {
    TargetValue::Str(value.to_string())
}

fn opens(target: ToastTarget) -> Option<NotificationTarget> {
    Some(NotificationTarget::Open(target))
}

#[test]
fn a_chat_notification_opens_its_chat_and_the_walls_opens_the_wall() {
    assert_eq!(
        parse("app.open-chat", Some(&TargetValue::Int64(42))),
        opens(ToastTarget::chat(42))
    );
    assert_eq!(
        parse(OPEN_CHAT, Some(&TargetValue::Int64(42))),
        opens(ToastTarget::chat(42))
    );
    let wall = parse("app.open-board", Some(&TargetValue::Int64(0)));
    assert!(matches!(wall, Some(NotificationTarget::Open(target)) if target.is_board()));
    assert!(matches!(
        parse(OPEN_BOARD, None),
        Some(NotificationTarget::Open(target)) if target.is_board() && target.family_id.is_none()
    ));
}

/// AN ARGUMENT IS UNTRUSTED INPUT: anything but a positive id opens nothing. The C#'s string cases
/// are a text target (read digits-only, as `NumberStyles.None`); the Linux shapes — a non-positive
/// `x`, another type, no target, an action this app never registered — are added.
#[test]
fn what_the_app_did_not_write_opens_nothing() {
    assert_eq!(parse(OPEN_CHAT, None), None);
    for value in [
        "abc",
        "-5",
        "0",
        " 42",
        "4 2",
        "",
        "+42",
        "42 ",
        "٤٢",
        "99999999999999999999",
    ] {
        assert_eq!(parse(OPEN_CHAT, Some(&text(value))), None, "{value:?}");
    }
    assert_eq!(parse(OPEN_CHAT, Some(&TargetValue::Int64(-5))), None);
    assert_eq!(parse(OPEN_CHAT, Some(&TargetValue::Int64(0))), None);
    assert_eq!(parse(OPEN_CHAT, Some(&TargetValue::Other)), None);
    // `("elsewhere", "42")`: an action that is not one of ours.
    assert_eq!(parse("app.elsewhere", Some(&TargetValue::Int64(42))), None);
    assert_eq!(parse("win.open-chat", Some(&TargetValue::Int64(42))), None);
    // A chat id written as text by another writer still opens that chat — the same meaning.
    assert_eq!(
        parse(OPEN_CHAT, Some(&text("42"))),
        opens(ToastTarget::chat(42))
    );
}

/// `SeveralFamiliesTests`: the wall's notification says whose, and a click reads it back; a family
/// id that is not a positive number is no family, and the wall shown opens.
#[test]
fn the_wall_opens_in_its_family_when_it_says_whose() {
    assert_eq!(
        parse(OPEN_BOARD, Some(&TargetValue::Int64(5))),
        opens(ToastTarget::board(Some(5)))
    );
    assert_eq!(
        parse(OPEN_BOARD, Some(&TargetValue::Int64(-2))),
        opens(ToastTarget::board(None))
    );
    assert_eq!(
        parse(OPEN_BOARD, Some(&text("-2"))),
        opens(ToastTarget::board(None))
    );
    assert_eq!(
        parse(OPEN_BOARD, Some(&text("5"))),
        opens(ToastTarget::board(Some(5)))
    );
    assert_eq!(
        parse(OPEN_BOARD, Some(&TargetValue::Other)),
        opens(ToastTarget::board(None))
    );
}

/// A ringing call's buttons name the call; whether it is the call ringing is the engine's
/// question. A call id is text and there — an `x`, nothing, or an empty string is not one.
#[test]
fn a_calls_buttons_carry_its_id() {
    let id = "6a1f0c3e-0000-4000-8000-000000000000";
    assert_eq!(
        parse("app.answer-call", Some(&text(id))),
        Some(NotificationTarget::Answer(id.to_string()))
    );
    assert_eq!(
        parse(DECLINE_CALL, Some(&text(id))),
        Some(NotificationTarget::Decline(id.to_string()))
    );
    assert_eq!(parse(ANSWER_CALL, Some(&text(""))), None);
    assert_eq!(parse(ANSWER_CALL, Some(&TargetValue::Int64(7))), None);
    assert_eq!(parse(DECLINE_CALL, None), None);
}

#[test]
fn a_positive_id_is_a_positive_number_and_nothing_else() {
    assert_eq!(positive_id(&TargetValue::Int64(1)), Some(1));
    assert_eq!(positive_id(&TargetValue::Int64(i64::MAX)), Some(i64::MAX));
    assert_eq!(positive_id(&text("9223372036854775807")), Some(i64::MAX));
    assert_eq!(positive_id(&text("9223372036854775808")), None);
    assert_eq!(positive_id(&text("007")), Some(7));
    assert_eq!(positive_id(&TargetValue::Int64(i64::MIN)), None);
}
