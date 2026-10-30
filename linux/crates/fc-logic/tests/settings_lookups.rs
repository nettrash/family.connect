//! `LookupsTests`, the statistics' half: a member's web searches, and the Assistant group that
//! searches alone call for.

mod common;

use fc_core::protocol::requests::{StatsAiDto, StatsMemberDto};
use fc_logic::settings::settings_text;

const POINT: &str = ".";

fn searches(questions: i32, searches: i32) -> Option<StatsAiDto> {
    Some(StatsAiDto {
        questions,
        searches,
        ..StatsAiDto::default()
    })
}

#[test]
fn a_members_line_counts_their_web_searches() {
    let say = common::english();
    let anna = StatsMemberDto {
        user_id: 1,
        display_name: "Anna".to_string(),
        messages: 3,
        ai: searches(2, 1),
        ..StatsMemberDto::default()
    };
    assert_eq!(
        settings_text::member_line(&anna, &say, POINT),
        "2 questions to the assistant · 1 web search"
    );
    let anna = StatsMemberDto {
        ai: searches(0, 5),
        ..anna
    };
    assert_eq!(
        settings_text::member_line(&anna, &say, POINT),
        "5 web searches"
    );
    // Russian counts in three forms.
    let russian = common::catalogues().for_language("ru");
    let line = |count: i32| {
        settings_text::member_line(
            &StatsMemberDto {
                ai: searches(0, count),
                ..anna.clone()
            },
            &russian,
            POINT,
        )
    };
    let (one, few, many) = (line(1), line(2), line(5));
    assert!(
        one != few && few != many && one != many,
        "{one} / {few} / {many}"
    );
    assert_eq!(line(11).replace("11", "5"), many);
    // None is not said.
    let none = StatsMemberDto {
        ai: searches(0, 0),
        ..anna.clone()
    };
    assert_eq!(settings_text::member_line(&none, &say, POINT), "Words only");
}

#[test]
fn the_assistant_group_shows_for_searches_alone() {
    let used = |ai: Option<StatsAiDto>| settings_text::assistant_used(ai.as_ref());
    assert!(!used(None));
    assert!(!used(Some(StatsAiDto::default())));
    assert!(used(searches(0, 1)));
    assert!(used(searches(1, 0)));
    assert!(used(Some(StatsAiDto {
        images: 1,
        ..StatsAiDto::default()
    })));
    assert!(used(Some(StatsAiDto {
        transcripts: 1,
        ..StatsAiDto::default()
    })));
}
