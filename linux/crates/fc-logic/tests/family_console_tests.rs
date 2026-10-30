//! `FamilyConsoleTests`: the family console's rules and words — the roster, what each row offers,
//! a report's sentences, and the failures said for what they were.

mod common;

use fc_core::protocol::{ApiError, AttachmentDto, MemberDto, ReportDto, UserDto, error_codes};
use fc_logic::family_console::{MemberTools, family_text};

fn member(id: i64, name: &str, role: &str, left: bool, deleted: bool) -> MemberDto {
    MemberDto {
        role: Some(role.to_string()),
        has_left: left,
        deleted,
        ..MemberDto::new(id, &name.to_lowercase(), name)
    }
}

fn plain(id: i64, name: &str) -> MemberDto {
    member(id, name, "member", false, false)
}

#[test]
fn the_roster_is_who_is_still_in_sorted_by_name_without_regard_to_case() {
    let members = [
        plain(1, "Zoe"),
        plain(2, "anna"),
        member(3, "Bob", "owner", false, false),
        member(4, "Carl", "member", false, true),
        member(5, "Dan", "member", true, false),
        plain(6, "bea"),
    ];
    let roster = family_text::roster(&members);

    assert_eq!(
        vec!["anna", "bea", "Bob", "Zoe"],
        roster
            .iter()
            .map(|member| member.display_name.as_str())
            .collect::<Vec<_>>()
    );
}

/// No "Message" for yourself or for somebody you blocked (it would only be refused); safety for
/// anybody but yourself; the owner's birthday tool on every row, their own included; and a reset or
/// removal never for yourself or the owner.
#[test]
fn each_row_offers_what_this_reader_may_do() {
    let owner = member(7, "Anna", "owner", false, false);
    let bob = plain(11, "Bob");

    assert_eq!(
        MemberTools::new(false, false, true, false, false),
        family_text::tools_for(&owner, 7, true, false)
    );
    assert_eq!(
        MemberTools::new(true, true, true, true, true),
        family_text::tools_for(&bob, 7, true, false)
    );
    assert_eq!(
        MemberTools::new(false, true, true, true, true),
        family_text::tools_for(&bob, 7, true, true)
    );
    assert_eq!(
        MemberTools::new(true, true, false, false, false),
        family_text::tools_for(&owner, 11, false, false)
    );
    assert_eq!(
        MemberTools::new(false, false, false, false, false),
        family_text::tools_for(&bob, 11, false, false)
    );
    // A roster that still names somebody else as owner — ownership moved and the frame is on its
    // way — offers no removal of them.
    assert!(!family_text::tools_for(&owner, 12, true, false).removable);
}

#[test]
fn the_member_count_agrees_with_its_number() {
    let say = common::english();
    assert_eq!("1 member", family_text::members_line(1, &say));
    assert_eq!("3 members", family_text::members_line(3, &say));
    let russian = common::catalogues().for_language("ru");
    assert_ne!("5 members", family_text::members_line(5, &russian));
}

#[test]
fn a_report_says_why_who_and_what_the_message_carried() {
    let say = common::english();
    assert_eq!("Spam", family_text::reason_label("spam", &say));
    assert_eq!("Something else", family_text::reason_label("threats", &say));
    assert_eq!(family_text::FIRST_REASON, family_text::REASONS[1].0);
    let report = ReportDto {
        id: 1,
        reporter: UserDto::new(7, "anna", "Anna"),
        reported: UserDto::new(11, "bob", "Bob"),
        reason: "spam".to_string(),
        ..ReportDto::default()
    };
    assert_eq!("Anna reported Bob", family_text::reported(&report, &say));

    let photo = AttachmentDto::new(1, "photo");
    assert_eq!(None, family_text::carried(None, &say));
    assert_eq!(None, family_text::carried(Some(&[]), &say));
    assert_eq!(
        Some("Photo".to_string()),
        family_text::carried(Some(std::slice::from_ref(&photo)), &say)
    );
    assert_eq!(
        Some("2 Photos".to_string()),
        family_text::carried(Some(&[photo.clone(), photo.clone()]), &say)
    );
    assert_eq!(
        Some("Video".to_string()),
        family_text::carried(Some(&[AttachmentDto::new(2, "video"), photo.clone()]), &say)
    );
    assert_eq!(
        Some("Voice message".to_string()),
        family_text::carried(Some(&[AttachmentDto::new(3, "audio")]), &say)
    );
    assert_eq!(
        Some("Location".to_string()),
        family_text::carried(Some(&[AttachmentDto::new(4, "location")]), &say)
    );
    assert_eq!(
        Some("minutes.pdf".to_string()),
        family_text::carried(
            Some(&[AttachmentDto {
                name: Some("minutes.pdf".to_string()),
                ..AttachmentDto::new(5, "file")
            }]),
            &say
        )
    );
    assert_eq!(
        Some("File".to_string()),
        family_text::carried(Some(&[AttachmentDto::new(6, "file")]), &say)
    );
}

#[test]
fn the_reporter_is_told_who_will_see_it() {
    let say = common::english();
    assert_eq!(
        "Your family owner will see this message and its text.",
        family_text::report_disclosure(true, &say)
    );
    assert_eq!(
        "Your family owner will be told you reported this member.",
        family_text::report_disclosure(false, &say)
    );
}

/// A server that answered with trouble of its own is not a network to go and check.
#[test]
fn a_failure_is_said_for_what_it_was() {
    let say = common::english();
    let busy = "The server is busy. Try again in a moment.";
    let trouble = "The server had a problem. Try again in a moment.";
    assert_eq!(
        busy,
        family_text::generic_failure(
            &ApiError::new(error_codes::TOO_MANY_REQUESTS, "x", 429),
            &say
        )
    );
    assert_eq!(
        busy,
        family_text::generic_failure(&ApiError::new("rate_limited_by_proxy", "x", 429), &say)
    );
    assert_eq!(
        trouble,
        family_text::generic_failure(&ApiError::new(error_codes::INTERNAL, "x", 500), &say)
    );
    assert_eq!(
        trouble,
        family_text::generic_failure(&ApiError::new("http_502", "bad gateway", 502), &say)
    );
    assert_eq!(
        trouble,
        family_text::generic_failure(&ApiError::new(error_codes::STORAGE_FULL, "disk", 507), &say)
    );
    assert_eq!(
        trouble,
        family_text::generic_failure(&ApiError::new("unreadable", "html", 418), &say)
    );
    assert_eq!(
        "Can't reach the server. Check your connection.",
        family_text::generic_failure(&ApiError::transport("reset"), &say)
    );
    assert_eq!(
        "That didn't work. Try again.",
        family_text::generic_failure(
            &ApiError::new(error_codes::NOT_FAMILY_OWNER, "x", 403),
            &say
        )
    );
}

#[test]
fn a_full_family_leaves_the_request_waiting() {
    let say = common::english();
    assert!(
        family_text::request_failure(&ApiError::new(error_codes::FAMILY_FULL, "x", 409), &say)
            .ends_with("the request is still waiting.")
    );
    assert_eq!(
        "That didn't work. Try again.",
        family_text::request_failure(
            &ApiError::new(error_codes::JOIN_REQUEST_NOT_PENDING, "x", 409),
            &say
        )
    );
}

/// The sentence a member reads before reporting an assistant reply. It has to say the operator and
/// NOT the family owner: a private assistant thread belongs to its member alone, so somebody
/// reporting a reply out of one needs to know who will read it before they send it.
#[test]
fn reporting_the_assistant_names_the_operator_and_not_the_owner() {
    let say = common::english();
    let said = family_text::assistant_report_disclosure(&say);
    assert!(said.contains("run this server"));
    // It names the operator, never the owner, and says the family's part OUT LOUD rather than
    // leaving a reader to assume it.
    assert!(!said.to_lowercase().contains("owner"));
    assert!(said.contains("Your family will not"));
    // And the member one still says the owner, which is the point of having two.
    assert!(family_text::report_disclosure(true, &say).contains("owner"));
}
