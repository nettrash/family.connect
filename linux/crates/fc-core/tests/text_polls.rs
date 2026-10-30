//! `PollsTests`: a poll's arithmetic — who holds what, how many voted, how full a bar is, and which
//! polls still want an answer. The C# builds `PollDto`s and `MessageDto`s; the rules read them
//! through `polls::Poll` and `polls::PollMessage`, which these test records implement with the
//! same fields.

use fc_core::text::polls::{self, Poll, PollMessage};

const ME: i64 = 7;

#[derive(Debug, Clone)]
struct TestOption {
    id: i64,
    votes: Vec<i64>,
}

/// `PollDto(PollSeq, Closed, Options)`.
#[derive(Debug, Clone)]
struct TestPoll {
    poll_seq: i64,
    closed: bool,
    options: Vec<TestOption>,
}

impl Poll for TestPoll {
    fn options(&self) -> Vec<(i64, &[i64])> {
        self.options
            .iter()
            .map(|option| (option.id, option.votes.as_slice()))
            .collect()
    }

    fn closed(&self) -> bool {
        self.closed
    }

    fn poll_seq(&self) -> i64 {
        self.poll_seq
    }
}

/// The fields of `MessageDto` the rules read.
#[derive(Debug, Clone)]
struct TestMessage {
    id: i64,
    sender_id: i64,
    poll: Option<TestPoll>,
}

impl PollMessage for TestMessage {
    type Poll = TestPoll;

    fn id(&self) -> i64 {
        self.id
    }

    fn sender_id(&self) -> i64 {
        self.sender_id
    }

    fn poll(&self) -> Option<&TestPoll> {
        self.poll.as_ref()
    }
}

fn poll(votes: &[&[i64]]) -> TestPoll {
    TestPoll {
        poll_seq: 1,
        closed: false,
        options: votes
            .iter()
            .enumerate()
            .map(|(index, voters)| TestOption {
                id: index as i64 + 1,
                votes: voters.to_vec(),
            })
            .collect(),
    }
}

fn message(id: i64, poll: TestPoll) -> TestMessage {
    from(id, poll, 9)
}

fn from(id: i64, poll: TestPoll, sender: i64) -> TestMessage {
    TestMessage {
        id,
        sender_id: sender,
        poll: Some(poll),
    }
}

fn no_poll(id: i64) -> TestMessage {
    TestMessage {
        id,
        sender_id: 9,
        poll: None,
    }
}

fn closed(mut poll: TestPoll) -> TestPoll {
    poll.closed = true;
    poll
}

fn seq(mut poll: TestPoll, poll_seq: i64) -> TestPoll {
    poll.poll_seq = poll_seq;
    poll
}

#[test]
fn the_arithmetic_counts_everyone_and_the_names_skip_the_blocked() {
    let held = poll(&[&[7, 9], &[11], &[]]);
    assert_eq!(polls::my_option(&held, ME), Some(1));
    assert_eq!(polls::my_option(&held, 4), None);
    assert_eq!(polls::voter_count(&held), 3);
    assert!((polls::fraction(&held, 2) - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(polls::fraction(&poll(&[&[], &[]]), 0), 0.0);
    // A share of the votes CAST: two votes over four options is a half each, not a quarter.
    assert!((polls::fraction(&poll(&[&[7], &[9], &[], &[]]), 1) - 0.5).abs() < 1e-9);

    // Identity only, in the order they voted: the count above goes on counting 9.
    assert_eq!(polls::drawable_voters(&[7, 9], |voter| voter == 9), [7]);
    assert_eq!(polls::drawable_voters(&[9, 7], |_| false), [9, 7]);
}

/// A member holds one option; a state that somehow said otherwise still counts them once.
#[test]
fn a_voter_is_counted_once() {
    assert_eq!(polls::voter_count(&poll(&[&[7], &[7]])), 1);
}

#[test]
fn tapping_the_option_you_hold_retracts_and_any_other_casts() {
    let held = poll(&[&[7], &[9]]);
    assert!(polls::tap_retracts(&held, 1, ME));
    assert!(!polls::tap_retracts(&held, 2, ME));
    assert!(!polls::tap_retracts(&held, 1, 4));
}

#[test]
fn only_a_numbered_open_poll_takes_votes() {
    assert!(polls::votable(&message(5, poll(&[&[], &[]]))));
    assert!(!polls::votable(&message(0, poll(&[&[], &[]]))));
    assert!(!polls::votable(&message(5, closed(poll(&[&[], &[]])))));
    assert!(!polls::votable(&no_poll(5)));
}

#[test]
fn a_question_is_more_than_white_space() {
    assert!(!polls::has_question(" \n\t"));
    assert!(!polls::has_question(""));
    assert!(polls::has_question(" ?"));
}

/// The badge: open, unanswered, not a blocked member's and not unnumbered — each message once, by
/// its NEWEST copy, so a vote the list has seen and the cache has not is not asked for again.
#[test]
fn an_unanswered_poll_is_open_not_mine_to_chase_twice_and_counted_by_its_newest_copy() {
    let open = poll(&[&[], &[]]);
    let held = [
        message(100, open.clone()),
        message(90, poll(&[&[ME], &[]])),
        message(80, closed(open.clone())),
        from(70, open.clone(), 13),
        message(0, open.clone()),
        message(50, closed(seq(open.clone(), 3))),
        from(40, open.clone(), ME),
        no_poll(30),
    ];
    let listed = [
        message(100, seq(poll(&[&[ME], &[]]), 2)),
        message(60, open.clone()),
        message(50, seq(open.clone(), 2)),
    ];
    let blocked = |user: i64| user == 13;

    // 60, and the reader's own 40: 100 was answered in the newer copy, 50 closed in the newer copy.
    assert_eq!(polls::unanswered(&held, &listed, ME, blocked), 2);
    // Without the list, 100 is still waiting.
    assert_eq!(polls::unanswered(&held, &[], ME, blocked), 2);
    assert_eq!(polls::unanswered(&held, &[], ME, |_| false), 3);
}

#[test]
fn the_options_sent_are_the_ones_checked() {
    assert_eq!(
        polls::sanitized(&[" Pizza", "", "Pasta "]),
        Some(vec!["Pizza".to_string(), "Pasta".to_string()])
    );
    assert_eq!(polls::sanitized(&["Pizza"]), None);
}

/// Not a port: the protocol's own records are what the bubble hands these rules, so they must
/// read a poll exactly as the test records do.
#[test]
fn the_protocols_records_read_as_a_poll() {
    let message: fc_core::protocol::MessageDto = fc_core::protocol::dtos::wire::decode(
        r#"{"id":9,"chat_id":1,"sender_id":4,"body":"Lunch?","created_at":"2026-10-28T12:00:00Z",
            "poll":{"poll_seq":3,"closed":false,"options":[{"id":1,"text":"Soup","votes":[4,7]},
            {"id":2,"text":"Salad","votes":[]}]}}"#,
    )
    .expect("a message with a poll decodes");
    let poll = PollMessage::poll(&message).expect("it carries its poll");
    assert_eq!(polls::my_option(poll, 7), Some(1));
    assert_eq!(polls::my_option(poll, 5), None);
    assert_eq!(
        (PollMessage::id(&message), PollMessage::sender_id(&message)),
        (9, 4)
    );
    assert!(!Poll::closed(poll));
    assert_eq!(Poll::poll_seq(poll), 3);
}
