//! `FamilyModelTests`: the family's own screens — the door, the owner's console, and the numbers
//! everybody may see.

mod common;

use std::rc::Rc;

use fc_core::protocol::{BirthdayDto, FamilyDto, FamilyPatch, MemberDto, UserDto, error_codes};
use fc_core::store::{ChatStore, Database, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_logic::families::family_rules;
use fc_logic::family::FamilyModel;
use time::macros::datetime;

use common::server::{Server, api};

const ME: i64 = 7;

struct Rig {
    chats: Rc<ChatStore>,
    clock: FakeClock,
}

impl Rig {
    fn new() -> StoreResult<Rig> {
        let cache = Database::open_in_memory()?;
        let chats = Rc::new(ChatStore::new(&cache).with_me(|| ME));
        chats.replace_members(
            &[
                MemberDto {
                    role: Some("owner".to_string()),
                    ..MemberDto::new(7, "anna", "Anna")
                },
                MemberDto {
                    role: Some("member".to_string()),
                    ..MemberDto::new(11, "bob", "Bob")
                },
            ],
            &[],
        )?;
        Ok(Rig {
            chats,
            clock: FakeClock::at(datetime!(2026-09-12 12:00 UTC)),
        })
    }

    fn build(&self, server: &Server) -> FamilyModel {
        FamilyModel::new(api(server, &self.clock), self.chats.clone())
    }
}

/// The C# `Smiths(cap, vision, photos, faces, history, language)`.
#[derive(Default)]
struct Smiths {
    cap: Option<i32>,
    vision: bool,
    photos: bool,
    faces: bool,
    no_history: bool,
    language: Option<&'static str>,
}

impl Smiths {
    fn family(self) -> FamilyDto {
        FamilyDto {
            join_policy: Some("open".to_string()),
            invite_code: Some("ABCD2345".to_string()),
            max_members: self.cap,
            ai_history: !self.no_history,
            ai_vision: self.vision,
            ai_history_photos: self.photos,
            ai_greeting: false,
            ai_faces: self.faces,
            language: self.language.map(str::to_string),
            ..FamilyDto::new(3, "The Smiths")
        }
    }
}

fn smiths() -> FamilyDto {
    Smiths::default().family()
}

fn capped(cap: i32) -> FamilyDto {
    Smiths {
        cap: Some(cap),
        ..Smiths::default()
    }
    .family()
}

fn one(bodies: Vec<String>) -> String {
    assert_eq!(1, bodies.len(), "exactly one body");
    bodies.into_iter().next().unwrap()
}

/// THE DOOR TAKES THE LOWER OF TWO NUMBERS, and the family's own setting is not the same question.
/// A family that set 40 under a ceiling of 50 goes on reporting 40 after the operator drops the
/// ceiling to 10 — a stored cap is never re-validated when the ceiling moves, because one config
/// edit must not lock an owner out of their own settings screen.
#[test]
fn seats_left_is_the_lower_of_the_familys_cap_and_the_operators_ceiling() -> StoreResult<()> {
    let rig = Rig::new()?;
    let family = rig.build(&Server::new());

    // Its own cap binds.
    let own = family.seats_for(&capped(4), 50)?;
    assert_eq!(4, own.limit());
    assert_eq!(2, own.left());
    assert!(!own.full());

    // The ceiling binds, and the family's own setting is still 40 — which is what the settings
    // screen shows.
    let ceiling = family.seats_for(&capped(40), 10)?;
    assert_eq!(Some(40), ceiling.cap);
    assert_eq!(10, ceiling.limit());
    assert_eq!(8, ceiling.left());

    // NO cap of its own is not "a cap equal to the ceiling": it is no cap, and only the operator's
    // ceiling binds.
    let none = family.seats_for(&smiths(), 3)?;
    assert_eq!(None, none.cap);
    assert_eq!(3, none.limit());
    assert_eq!(1, none.left());

    // A cap BELOW the room is a freeze rather than a refusal: nobody new until people leave.
    let frozen = family.seats_for(&capped(1), 50)?;
    assert_eq!(0, frozen.left());
    assert!(frozen.full());
    Ok(())
}

#[test]
fn former_members_do_not_take_seats() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.chats.replace_members(
        &[MemberDto {
            role: Some("owner".to_string()),
            ..MemberDto::new(7, "anna", "Anna")
        }],
        &[MemberDto {
            has_left: true,
            deleted: true,
            ..MemberDto::new(11, "bob", "Bob")
        }],
    )?;
    let family = rig.build(&Server::new());

    assert_eq!(1, family.present()?.len());
    assert_eq!(2, family.members()?.len());
    assert_eq!(3, family.seats_for(&capped(4), 50)?.left());
    Ok(())
}

/// THE ASSISTANT'S TWO DEPENDENT SWITCHES may only be true while `ai_vision` is, and turning vision
/// off turns them off in the same write whether or not the request mentioned them. A screen that
/// drew them as still on would be lying about what the family agreed to.
#[test]
fn turning_vision_off_turns_the_two_that_depend_on_it_off() {
    let on = Smiths {
        vision: true,
        photos: true,
        faces: true,
        ..Smiths::default()
    }
    .family();

    let off = FamilyModel::as_applied(
        &on,
        &FamilyPatch {
            ai_vision: Some(false),
            ..FamilyPatch::default()
        },
    );

    assert!(!off.ai_vision);
    assert!(!off.ai_history_photos);
    assert!(!off.ai_faces);
    // And nothing else moved: absent leaves a field alone.
    assert!(off.ai_history);
    assert_eq!(Some("ABCD2345"), off.invite_code.as_deref());

    // Neither is offered while vision is off, because the server would refuse it.
    assert!(!FamilyModel::may_turn_on("ai_history_photos", &off));
    assert!(!FamilyModel::may_turn_on("ai_faces", &off));
    // The two that depend on nothing are always offered.
    assert!(FamilyModel::may_turn_on("ai_history", &off));
    assert!(FamilyModel::may_turn_on("ai_greeting", &off));
}

/// THE TRANSCRIPT SWITCH DEPENDS ON NOTHING (docs/protocol.md, "Transcripts on request"): vision
/// going off leaves it where it was, it is always offered, and it goes out as its own key — even
/// beside vision off.
#[test]
fn the_transcript_switch_is_tied_to_no_other() -> StoreResult<()> {
    let on = FamilyDto {
        ai_transcripts: true,
        ..Smiths {
            vision: true,
            ..Smiths::default()
        }
        .family()
    };
    assert!(
        FamilyModel::as_applied(
            &on,
            &FamilyPatch {
                ai_vision: Some(false),
                ..FamilyPatch::default()
            }
        )
        .ai_transcripts
    );
    assert!(
        !FamilyModel::as_applied(
            &on,
            &FamilyPatch {
                ai_transcripts: Some(false),
                ..FamilyPatch::default()
            }
        )
        .ai_transcripts
    );
    assert!(
        FamilyModel::as_applied(
            &smiths(),
            &FamilyPatch {
                ai_transcripts: Some(true),
                ..FamilyPatch::default()
            }
        )
        .ai_transcripts
    );
    assert!(FamilyModel::may_turn_on("ai_transcripts", &smiths()));

    let rig = Rig::new()?;
    let server = Server::new();
    server.on(
        "/families/mine",
        r#"
            {"family": {"id": 3, "name": "The Smiths", "ai_transcripts": true}}
            "#,
    );
    let family = rig.build(&server);
    let changed = drive(
        &rig.clock,
        family.change(
            &smiths(),
            &FamilyPatch {
                ai_transcripts: Some(true),
                ..FamilyPatch::default()
            },
        ),
    )
    .expect("changed");
    assert!(changed.ai_transcripts);
    assert_eq!("{\"ai_transcripts\":true}", one(server.bodies()));
    Ok(())
}

/// A dependent switch is never SENT true alongside vision off: that answer is `validation`, and the
/// person would be told their own screen was wrong.
#[test]
fn a_change_the_server_would_refuse_is_not_sent() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(
        "/families/mine",
        r#"
            {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                        "ai_history": true, "ai_vision": false}}
            "#,
    );
    let family = rig.build(&server);

    let _ = drive(
        &rig.clock,
        family.change(
            &smiths(),
            &FamilyPatch {
                ai_history_photos: Some(true),
                ai_faces: Some(true),
                ai_greeting: Some(true),
                ..FamilyPatch::default()
            },
        ),
    );

    let sent = one(server.bodies());
    assert!(!sent.contains("ai_history_photos"));
    assert!(!sent.contains("ai_faces"));
    // What did not depend on vision went.
    assert!(sent.contains("\"ai_greeting\":true"));
    Ok(())
}

/// THE TWO PLACES WHERE `null` MEANS SOMETHING A MISSING KEY DOES NOT: clearing the cap and clearing
/// the language. Everything else absent means "leave it alone".
#[test]
fn clearing_the_cap_and_the_language_sends_null_and_nothing_else_sends_anything() -> StoreResult<()>
{
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(
        "/families/mine",
        r#"
            {"family": {"id": 3, "name": "The Smiths", "ai_history": true}}
            "#,
    );
    let family = rig.build(&server);
    let held = || {
        Smiths {
            cap: Some(8),
            language: Some("ru"),
            ..Smiths::default()
        }
        .family()
    };
    let clear = FamilyPatch {
        clears_cap: true,
        clears_language: true,
        ..FamilyPatch::default()
    };

    let _ = drive(&rig.clock, family.change(&held(), &clear));

    let sent = one(server.bodies());
    assert!(sent.contains("\"max_members\":null"));
    assert!(sent.contains("\"language\":null"));
    assert!(!sent.contains("join_policy"));
    assert!(!sent.contains("ai_history"));

    // And locally the two are cleared rather than left alone.
    let applied = FamilyModel::as_applied(&held(), &clear);
    assert_eq!(None, applied.max_members);
    assert_eq!(None, applied.language);
    Ok(())
}

#[test]
fn a_change_that_names_nothing_is_a_valid_no_op() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(
        "/families/mine",
        r#"
            {"family": {"id": 3, "name": "The Smiths", "ai_history": true}}
            "#,
    );
    let family = rig.build(&server);

    let changed = drive(
        &rig.clock,
        family.change(&smiths(), &FamilyPatch::default()),
    )
    .expect("changed");

    assert_eq!("The Smiths", changed.name);
    assert_eq!("{}", one(server.bodies()));
    Ok(())
}

/// A SUCCESSOR IS A PREDICTION. Any join or leave changes it and it takes no frame of its own, so
/// the dialog is drawn from a FRESH read every time — and its absence on that read means leaving
/// DELETES the family, which is a different dialog.
#[test]
fn the_leave_dialog_is_drawn_from_a_fresh_read_and_never_from_a_cache() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.then(
        "/families/mine",
        &[
            (
                200,
                Some(
                    r#"
                {"family": {"id": 3, "name": "The Smiths", "ai_history": true},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"},
                             {"id": 14, "username": "gran", "display_name": "Gran", "role": "member"}],
                 "blocked_user_ids": [], "next_owner_user_id": 14}
                "#,
                ),
            ),
            (
                200,
                Some(
                    r#"
                {"family": {"id": 3, "name": "The Smiths", "ai_history": true},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
                 "blocked_user_ids": []}
                "#,
                ),
            ),
        ],
    );
    let family = rig.build(&server);

    let would = drive(&rig.clock, family.leave_would())?.expect("an answer");
    assert!(!would.deletes_family);
    // Resolved to a name — from the roster THAT read refreshed, because a member who joined since
    // would not be in the cache.
    assert_eq!("Gran", would.successor.expect("a successor").display_name);

    // Gran leaves first. The next dialog is a different dialog.
    let alone = drive(&rig.clock, family.leave_would())?.expect("an answer");
    assert!(alone.deletes_family);
    assert_eq!(None, alone.successor);
    assert_eq!(2, server.asked().len());
    Ok(())
}

#[test]
fn leaving_answers_the_successor_the_server_actually_chose() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on("/families/leave", r#"{"new_owner_user_id": 11}"#);
    let family = rig.build(&server);

    assert_eq!(Ok(Some(11)), drive(&rig.clock, family.leave(None)));
    Ok(())
}

/// The COMMON leave is a member's — or the last member's, whose family goes with them — and its
/// answer is a `204` with no body (protocol.md, `POST /families/leave`). It names nobody, and it is
/// not an error.
#[test]
fn leaving_as_a_member_is_a_no_content_answer_that_names_nobody() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on_status("/families/leave", 204, None);
    let family = rig.build(&server);

    assert_eq!(Ok(None), drive(&rig.clock, family.leave(None)));
    Ok(())
}

/// And ONLY there. An endpoint whose answer is a body — the join requests, here — does not turn an
/// empty `204` into a success with nothing in it: a 2xx this client cannot read stays a failure it
/// will not act on.
#[test]
fn an_answer_that_promises_a_body_is_not_a_success_without_one() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on_status("/families/join-requests", 204, None);
    let family = rig.build(&server);

    let error = drive(&rig.clock, family.requests()).expect_err("no requests");

    assert_eq!(error_codes::VALIDATION, error.code);
    Ok(())
}

/// A `family_full` refusal of an approval LEAVES THE REQUEST PENDING: a full family is a temporary
/// condition and not a decision, and a client that struck the row off its list would have thrown
/// the owner's decision away.
#[test]
fn approving_a_join_request_into_a_full_family_leaves_it_pending() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on_status(
        "/families/join-requests/12/approve",
        409,
        Some(r#"{"error": {"code": "family_full", "message": "no"}}"#),
    );
    let family = rig.build(&server);

    let refused = drive(&rig.clock, family.approve(12))?.expect_err("not joined");

    assert!(refused.still_pending);
    assert_eq!(error_codes::FAMILY_FULL, refused.error.code);
    Ok(())
}

#[test]
fn an_approved_request_puts_the_member_on_the_roster() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(
        "/families/join-requests/12/approve",
        r#"
            {"member": {"id": 14, "username": "gran", "display_name": "Gran", "role": "member",
                        "birthday": {"month": 3, "day": 14}}}
            "#,
    );
    let family = rig.build(&server);

    let joined = drive(&rig.clock, family.approve(12))?.expect("joined");

    assert_eq!("Gran", joined.display_name);
    let gran = rig.chats.member(14)?.expect("on the roster");
    assert_eq!("Gran", gran.display_name);
    // A join carries what the server knows, birthday included — the calendar is drawn from the
    // roster and nothing else replays it.
    assert_eq!(Some(BirthdayDto { month: 3, day: 14 }), gran.birthday);
    // And arriving is not owning.
    assert!(!gran.owner());
    Ok(())
}

/// The member an approval answers with carries the owner's `password_reset` for them, and the
/// roster keeps it: a newcomer who is also in a family this owner does not own is not offered a
/// reset the server would refuse with `member_in_other_families` — and the `member_joined` frame
/// that follows, should it say nothing, does not put the offer back (docs/protocol.md, "Several
/// families").
#[test]
fn an_approved_newcomer_keeps_the_reset_flag_the_answer_carries() -> StoreResult<()> {
    let held = Database::open_in_memory()?;
    let roster = Rc::new(
        ChatStore::new(&held)
            .with_me(|| ME)
            .with_current(|| Some(3)),
    );
    roster.replace_members_of(
        Some(3),
        &[MemberDto {
            role: Some("owner".to_string()),
            ..MemberDto::new(7, "anna", "Anna")
        }],
        &[],
    )?;
    let server = Server::new();
    server.on(
        "/families/join-requests/12/approve",
        r#"{"member": {"id": 14, "username": "gran", "display_name": "Gran", "role": "member", "password_reset": false}}"#,
    );
    let clock = FakeClock::at(datetime!(2026-09-12 12:00 UTC));
    let family = FamilyModel::new(api(&server, &clock), roster.clone());

    let joined = drive(&clock, family.approve(12))?.expect("joined");

    assert_eq!(Some(false), joined.password_reset);
    let gran = |roster: &ChatStore| -> StoreResult<MemberDto> {
        Ok(roster
            .members()?
            .iter()
            .find(|member| member.id == 14)
            .cloned()
            .expect("on the roster"))
    };
    assert_eq!(Some(false), gran(&roster)?.password_reset);
    assert!(!family_rules::may_reset_password(&gran(&roster)?, ME, true));

    roster.joined(&UserDto::new(14, "gran", "Gran"), Some(3), None)?;
    assert_eq!(Some(false), gran(&roster)?.password_reset);
    Ok(())
}

#[test]
fn requests_that_nobody_has_made_are_an_empty_list_and_not_a_failure() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on("/families/join-requests", "{}");
    let family = rig.build(&server);

    assert_eq!(Ok(Vec::new()), drive(&rig.clock, family.requests()));
    Ok(())
}

/// Blocking is applied HERE as well as asked for: the frame that confirms it reaches this device
/// too, and full state applied twice is applied once.
#[test]
fn blocking_takes_effect_without_waiting_for_the_frame() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on_status("/families/members/11/block", 204, None);
    let family = rig.build(&server);

    assert_eq!(None, drive(&rig.clock, family.block(11, true))?);

    assert!(rig.chats.is_blocked(11)?);
    assert_eq!(vec![11], family.blocked()?.to_vec());
    assert_eq!(1, server.asked().len());

    assert_eq!(None, drive(&rig.clock, family.block(11, false))?);
    assert!(!rig.chats.is_blocked(11)?);
    Ok(())
}

#[test]
fn a_refused_block_changes_nothing_here() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on_status(
        "/families/members/11/block",
        403,
        Some(r#"{"error": {"code": "not_same_family", "message": "no"}}"#),
    );
    let family = rig.build(&server);

    let refused = drive(&rig.clock, family.block(11, true))?;

    assert_eq!("not_same_family", refused.expect("refused").code);
    assert!(!rig.chats.is_blocked(11)?);
    Ok(())
}

/// THE ROWS DO NOT ADD UP TO THE TOTALS, AND THE GAP IS THE BLOCK. A client must never derive a
/// total by summing the rows, nor a share by dividing into that sum — the totals are the family's
/// numbers, and the rows are what this caller may see of them.
#[test]
fn the_statistics_totals_are_the_familys_and_the_rows_are_only_what_this_reader_may_see()
-> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(
        "/families/mine/stats",
        r#"
            {"generated_at": "2026-09-12T12:00:00Z",
             "totals": {"members": 4, "messages": 1284, "board_notes": 7,
                        "attachments": {"count": 96, "photo": 61, "bytes": 734003200,
                                        "stored_bytes": 612368384},
                        "ai": {"questions": 43, "prompt_tokens": 12040,
                               "completion_tokens": 30512, "images": 6}},
             "members": [{"user_id": 7, "display_name": "Anna", "messages": 512,
                          "attachments": {"count": 31, "photo": 22, "bytes": 241172480},
                          "ai": {"questions": 12, "images": 2}}]}
            "#,
    );
    let family = rig.build(&server);

    let stats = drive(&rig.clock, family.stats()).expect("stats");

    assert_eq!(1284, stats.totals.messages);
    assert_eq!(4, stats.totals.members);
    // One row for four members: the gap is who this reader has blocked.
    let members = stats.members.as_ref().expect("rows");
    assert_eq!(1, members.len());
    assert!(FamilyModel::rows_are_incomplete(&stats));
    assert_ne!(
        stats.totals.messages,
        members.iter().map(|member| member.messages).sum::<i64>()
    );

    // `bytes` and `stored_bytes` are different numbers and the gap is what dedup saved — and the
    // stored one is a family total, never a per-member share.
    let totals = stats.totals.attachments.as_ref().expect("attachments");
    assert_eq!(734003200, totals.bytes);
    assert_eq!(Some(612368384), totals.stored_bytes);
    assert_eq!(
        None,
        members[0]
            .attachments
            .as_ref()
            .expect("theirs")
            .stored_bytes
    );

    // A picture answer is one question, no tokens and one image, so a family reading only the
    // token counts would see the expensive half of the assistant as free.
    assert_eq!(6, stats.totals.ai.as_ref().expect("ai").images);
    assert_eq!(2, members[0].ai.as_ref().expect("theirs").images);
    Ok(())
}

#[test]
fn statistics_with_every_row_visible_say_nothing_is_hidden() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(
        "/families/mine/stats",
        r#"
            {"generated_at": "2026-09-12T12:00:00Z",
             "totals": {"members": 1, "messages": 10, "board_notes": 0},
             "members": [{"user_id": 7, "display_name": "Anna", "messages": 10}]}
            "#,
    );
    let family = rig.build(&server);

    let stats = drive(&rig.clock, family.stats()).expect("stats");

    assert!(!FamilyModel::rows_are_incomplete(&stats));
    Ok(())
}

#[test]
fn reading_the_family_again_replaces_the_roster_and_the_block_list() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(
        "/families/mine",
        r#"
            {"family": {"id": 3, "name": "The Smiths"},
             "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"},
                         {"id": 11, "username": "bob", "display_name": "Bob", "role": "member", "birthday": {"month": 3, "day": 12}}],
             "blocked_user_ids": [11]}
            "#,
    );
    let family = rig.build(&server);

    assert_eq!(None, drive(&rig.clock, family.read())?);

    assert_eq!(
        Some(BirthdayDto { month: 3, day: 12 }),
        rig.chats.member(11)?.expect("Bob").birthday
    );
    assert!(rig.chats.is_blocked(11)?);
    Ok(())
}

#[test]
fn a_failed_read_leaves_the_roster_as_it_was() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on_status(
        "/families/mine",
        500,
        Some(r#"{"error": {"code": "internal", "message": "later"}}"#),
    );
    let family = rig.build(&server);

    assert_eq!(
        error_codes::INTERNAL,
        drive(&rig.clock, family.read())?.expect("failed").code
    );

    assert_eq!(
        vec!["Anna", "Bob"],
        rig.chats
            .members()?
            .iter()
            .map(|member| member.display_name.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(None, rig.chats.member(11)?.expect("Bob").birthday);
    Ok(())
}
