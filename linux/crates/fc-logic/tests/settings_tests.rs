//! `SettingsTests`: what leaving means, the statistics line, refusals said for what they were, a
//! new password, and a birthday written the reader's way.
//!
//! The C#'s `CultureInfo.InvariantCulture` for a size is the decimal point `"."`; its en-US and
//! ru-RU for a birthday are `DateCulture::named` (measured from those cultures).

mod common;

use fc_core::protocol::requests::{StatsAiDto, StatsMediaDto, StatsMemberDto};
use fc_core::protocol::{ApiError, BirthdayDto, MemberDto, error_codes};
use fc_core::text::media_text;
use fc_logic::date_culture::DateCulture;
use fc_logic::settings::{LeaveContext, LeaveKind, birthday_rules, settings_text};

const POINT: &str = ".";

fn member(id: i64, name: &str) -> MemberDto {
    MemberDto::new(id, &name.to_lowercase(), name)
}

fn stats(user_id: i64, name: &str, messages: i64) -> StatsMemberDto {
    StatsMemberDto {
        user_id,
        display_name: name.to_string(),
        messages,
        ..StatsMemberDto::default()
    }
}

fn media(count: i32, bytes: i64, stored_bytes: Option<i64>) -> StatsMediaDto {
    StatsMediaDto {
        count,
        bytes,
        stored_bytes,
        ..StatsMediaDto::default()
    }
}

fn ai(questions: i32) -> StatsAiDto {
    StatsAiDto {
        questions,
        ..StatsAiDto::default()
    }
}

#[test]
fn leaving_says_what_it_means_for_who_is_leaving() {
    let say = common::english();
    let bob = LeaveContext::for_leaving(true, false, Some(&member(2, "Bob"))).unwrap();
    assert_eq!(
        settings_text::leave_message(&bob, &say),
        "Bob becomes the owner. You'll lose access to the family chat and your direct chats; your history returns if you rejoin."
    );
    let last = LeaveContext::for_leaving(true, true, None).unwrap();
    assert!(settings_text::leave_message(&last, &say).contains("deletes the family"));
    let one = LeaveContext::for_leaving(false, true, None).unwrap();
    assert!(!settings_text::leave_message(&one, &say).contains("deletes"));
    // Deleting the family is its own question, and its own button.
    assert_eq!(
        settings_text::leave_title(&last, &say),
        "Delete the family?"
    );
    assert_eq!(settings_text::leave_button(&last, &say), "Leave and Delete");
    assert_eq!(settings_text::leave_button(&bob, &say), "Leave Family");
    assert_eq!(settings_text::leave_title(&one, &say), "Leave the family?");
}

/// A MEMBER never gets the owner's dialogs, whatever the roster says; and an owner whose successor
/// the fresh roster does not name gets NO dialog rather than a guess.
#[test]
fn which_dialog_is_the_owners_question_and_never_a_guess() {
    assert_eq!(
        LeaveContext::for_leaving(false, false, Some(&member(2, "Bob")))
            .unwrap()
            .kind,
        LeaveKind::Member
    );
    assert_eq!(
        LeaveContext::for_leaving(false, true, None).unwrap().kind,
        LeaveKind::Member
    );
    assert_eq!(LeaveContext::for_leaving(true, false, None), None);
}

#[test]
fn a_members_line_counts_what_they_send_besides_words() {
    let say = common::english();
    let anna = stats(1, "Anna", 3);
    assert_eq!(settings_text::member_line(&anna, &say, POINT), "Words only");

    let anna = StatsMemberDto {
        attachments: Some(media(1, 1_234_567, None)),
        ai: Some(StatsAiDto { images: 1, ..ai(2) }),
        ..anna
    };
    assert_eq!(
        settings_text::member_line(&anna, &say, POINT),
        format!(
            "1 attachment, {} · 2 questions to the assistant · 1 picture from the assistant",
            media_text::display_size(1_234_567, POINT)
        )
    );

    let anna = StatsMemberDto {
        attachments: Some(media(4, 10, None)),
        ai: Some(StatsAiDto { images: 3, ..ai(1) }),
        ..anna
    };
    assert!(settings_text::member_line(&anna, &say, POINT).starts_with("4 attachments, "));
    assert!(
        settings_text::member_line(&anna, &say, POINT)
            .ends_with("1 question to the assistant · 3 pictures from the assistant")
    );
}

/// The recordings a member had turned into text, beside their questions — charged to whoever
/// ASKED, and counted in English as one and many.
#[test]
fn a_members_line_counts_the_recordings_they_had_as_text() {
    let say = common::english();
    let anna = StatsMemberDto {
        ai: Some(StatsAiDto {
            transcripts: 1,
            ..ai(2)
        }),
        ..stats(1, "Anna", 3)
    };
    assert_eq!(
        settings_text::member_line(&anna, &say, POINT),
        "2 questions to the assistant · 1 recording as text"
    );
    let anna = StatsMemberDto {
        ai: Some(StatsAiDto {
            transcripts: 4,
            ..ai(0)
        }),
        ..anna
    };
    assert_eq!(
        settings_text::member_line(&anna, &say, POINT),
        "4 recordings as text"
    );
    let russian = common::catalogues().for_language("ru");
    assert_eq!(
        settings_text::member_line(&anna, &russian, POINT),
        "записей в текст: 4"
    );
}

/// A translation the apps say one way whatever the count is said that way, with its own order.
#[test]
fn a_members_line_reads_in_the_readers_language() {
    let russian = common::catalogues().for_language("ru");
    let anna = StatsMemberDto {
        ai: Some(ai(1)),
        ..stats(1, "Анна", 3)
    };
    assert_eq!(
        settings_text::member_line(&anna, &russian, POINT),
        "вопросов ассистенту: 1"
    );
}

#[test]
fn a_picture_refused_is_said_for_what_it_was() {
    let say = common::english();
    let failure =
        |error: ApiError, uploading: bool| settings_text::picture_failure(&error, uploading, &say);
    assert_eq!(
        failure(ApiError::new(error_codes::AVATAR_TOO_LARGE, "x", 413), true),
        "That photo is too large for this server."
    );
    assert_eq!(
        failure(ApiError::new("payload_too_large", "proxy", 413), true),
        "That photo is too large for this server."
    );
    assert_eq!(
        failure(ApiError::new(error_codes::INVALID_IMAGE, "x", 400), true),
        "That file isn't a photo we can use."
    );
    assert_eq!(
        failure(ApiError::transport("reset"), false),
        "Can't reach the server. Check your connection."
    );
    assert_eq!(
        failure(
            ApiError::new(error_codes::TOO_MANY_REQUESTS, "slow", 429),
            true
        ),
        "The server is busy. Try again in a moment."
    );
    assert_eq!(
        failure(ApiError::new(error_codes::INTERNAL, "x", 500), true),
        "Couldn't upload the photo."
    );
    assert_eq!(
        failure(ApiError::new(error_codes::INTERNAL, "x", 500), false),
        "Couldn't remove the photo."
    );
}

/// Eight SCALARS, typed twice: seven Cyrillic letters are seven, and four family emoji are four
/// graphemes the apps refuse but twenty-eight scalars the server takes.
#[test]
fn a_new_password_is_eight_scalars_and_typed_twice() {
    let say = common::english();
    let problem = |new: &str, again: &str| settings_text::password_problem(new, again, &say);
    assert_eq!(
        problem("1234567", "1234567").as_deref(),
        Some("Use at least 8 characters.")
    );
    assert_eq!(
        problem("12345678", "12345679").as_deref(),
        Some("Those two do not match.")
    );
    assert_eq!(problem("12345678", "12345678"), None);
    // A password is its exact characters: typed twice means typed the same, capitals included.
    assert_eq!(
        problem("Password1", "password1").as_deref(),
        Some("Those two do not match.")
    );
    assert_eq!(
        problem("пароль1", "пароль1").as_deref(),
        Some("Use at least 8 characters.")
    );
    let family = "👨‍👩‍👧‍👦".repeat(4);
    assert_eq!(problem(&family, &family), None);
    assert_eq!(
        problem("😀😀😀😀", "😀😀😀😀").as_deref(),
        Some("Use at least 8 characters.")
    );
}

#[test]
fn account_refusals_are_said_as_the_web_says_them() {
    let say = common::english();
    assert_eq!(
        settings_text::delete_failure(
            &ApiError::new(error_codes::INVALID_CREDENTIALS, "x", 401),
            &say
        ),
        "That password is not right."
    );
    assert_eq!(
        settings_text::delete_failure(&ApiError::new(error_codes::VALIDATION, "x", 400), &say),
        "Type your password to confirm."
    );
    assert_eq!(
        settings_text::delete_failure(&ApiError::transport("x"), &say),
        "Couldn't delete your account. Try again."
    );
    assert_eq!(
        settings_text::password_change_failure(
            &ApiError::new(error_codes::INVALID_CREDENTIALS, "x", 401),
            &say
        ),
        "That current password is not right."
    );
    assert_eq!(
        settings_text::password_change_failure(
            &ApiError::new(error_codes::INTERNAL, "x", 500),
            &say
        ),
        "Couldn't change your password. Try again."
    );
    assert_eq!(
        settings_text::birthday_failure(&ApiError::new(error_codes::VALIDATION, "x", 400), &say),
        "That date doesn't exist."
    );
    assert_eq!(
        settings_text::birthday_failure(
            &ApiError::new(error_codes::NOT_FAMILY_OWNER, "x", 403),
            &say
        ),
        "Only the family owner can do that."
    );
    assert_eq!(
        settings_text::birthday_failure(&ApiError::transport("x"), &say),
        "Couldn't save that birthday. Try again."
    );
}

/// February has 29, and a picker opens on the birthday held — inside the calendar.
#[test]
fn a_birthday_has_no_year_for_the_twenty_ninth_to_fail_in() {
    let days: Vec<i32> = (1..=12).map(birthday_rules::days_in).collect();
    assert_eq!(days, [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]);
    assert!(birthday_rules::ok(2, 29));
    assert!(!birthday_rules::ok(4, 31));
    assert!(!birthday_rules::ok(13, 1));
    assert!(!birthday_rules::ok(1, 0));
    assert_eq!(birthday_rules::start(None), (1, 1));
    assert_eq!(
        birthday_rules::start(Some(&BirthdayDto { month: 4, day: 31 })),
        (4, 30)
    );
    assert_eq!(
        birthday_rules::start(Some(&BirthdayDto { month: 14, day: -3 })),
        (12, 1)
    );
}

#[test]
fn a_birthday_is_written_the_readers_way() {
    let say = common::english();
    let american = DateCulture::named("en-US").unwrap();
    let russian = DateCulture::named("ru-RU").unwrap();
    let invariant = DateCulture::invariant();
    let text = |birthday: Option<BirthdayDto>, culture: &DateCulture| {
        birthday_rules::text(birthday.as_ref(), &say, culture)
    };
    assert_eq!(
        text(Some(BirthdayDto { month: 3, day: 12 }), &american),
        "March 12"
    );
    assert_eq!(
        text(Some(BirthdayDto { month: 3, day: 12 }), &russian),
        "12 марта"
    );
    assert_eq!(
        text(Some(BirthdayDto { month: 2, day: 29 }), &american),
        "February 29"
    );
    assert_eq!(text(None, &invariant), "Not set");
    assert_eq!(
        text(Some(BirthdayDto { month: 2, day: 30 }), &invariant),
        "Not set"
    );
    // The month standing alone, as a picker lists it — nominative, not "марта".
    assert_eq!(birthday_rules::month_name(3, &russian), "март");
    assert_eq!(birthday_rules::month_name(3, &american), "March");
    assert_eq!(birthday_rules::month_name(13, &american), "");
}

#[test]
fn what_deduplication_saved_is_the_familys_totals_and_only_when_it_saved_anything() {
    assert_eq!(settings_text::saved(None), None);
    assert_eq!(settings_text::saved(Some(&media(3, 100, None))), None);
    assert_eq!(settings_text::saved(Some(&media(3, 100, Some(100)))), None);
    assert_eq!(settings_text::saved(Some(&media(3, 100, Some(140)))), None);
    assert_eq!(
        settings_text::saved(Some(&media(3, 100, Some(40)))),
        Some(60)
    );
}
