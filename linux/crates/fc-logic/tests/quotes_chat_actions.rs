//! `ChatActionsTests`, what a reply quotes (`Quotes`): who it answers unless they are blocked, both
//! levels with the id of each, each level hidden and revealed on its own, and what a click does.

mod common;

use fc_core::protocol::{MessageDto, QuoteParentDto, ReplyToDto};
use fc_core::store::StoreResult;
use fc_logic::quotes::{self, Quote, QuoteClick, QuoteLevel, QuoteLine};

use common::chat::{CHAT, ME, actions_rig};

const SENT: &str = "2026-09-13T10:00:00Z";

fn quoting(id: i64, sender: i64, body: &str, reply_to: ReplyToDto) -> MessageDto {
    MessageDto {
        reply_to: Some(reply_to),
        ..MessageDto::new(id, CHAT, sender, None, body, SENT)
    }
}

fn answers(message_id: i64, sender_id: i64, excerpt: &str) -> ReplyToDto {
    ReplyToDto {
        message_id,
        sender_id,
        excerpt: excerpt.to_string(),
        parent: None,
    }
}

fn under(message_id: i64, sender_id: i64, excerpt: &str) -> Option<QuoteParentDto> {
    Some(QuoteParentDto {
        message_id,
        sender_id,
        excerpt: excerpt.to_string(),
    })
}

#[test]
fn a_quote_names_who_it_answers_unless_they_are_blocked() -> StoreResult<()> {
    let rig = actions_rig()?;
    let say = common::english();
    let reply_to = |sender: i64| quoting(2, 11, "yes", answers(1, sender, "Dinner at 7?"));
    let one = |name: &str| Quote {
        reply: QuoteLine::new(1, name, "Dinner at 7?", false),
        parent: None,
    };

    assert_eq!(
        None,
        quotes::of(
            &MessageDto::new(3, CHAT, 11, None, "plain", SENT),
            &rig.chats,
            &say,
            |_| false
        )?
    );
    assert_eq!(
        Some(one("You")),
        quotes::of(&reply_to(ME), &rig.chats, &say, |_| false)?
    );
    assert_eq!(
        Some(one("Carl")),
        quotes::of(&reply_to(12), &rig.chats, &say, |_| false)?
    );
    assert_eq!(
        Some(one("Deleted account")),
        quotes::of(&reply_to(14), &rig.chats, &say, |_| false)?
    );
    assert_eq!(
        Some(one("Someone")),
        quotes::of(&reply_to(40), &rig.chats, &say, |_| false)?
    );
    assert_eq!(
        Some(Quote {
            reply: QuoteLine::new(1, "", "Replying to a hidden message", true),
            parent: None,
        }),
        quotes::of(&reply_to(13), &rig.chats, &say, |_| false)?
    );

    assert_eq!(
        "Replying to Carl",
        quotes::banner(
            &MessageDto::new(4, CHAT, 12, None, "hi", SENT),
            &rig.chats,
            &say
        )?
    );
    assert_eq!(
        "Replying to a hidden message",
        quotes::banner(
            &MessageDto::new(5, CHAT, 13, None, "hi", SENT),
            &rig.chats,
            &say
        )?
    );
    Ok(())
}

/// THE ID IS DRAWN WITH THE QUOTE, both levels of it: it is what a click on the quote follows back
/// to the message, and the second level is the server's own (docs/protocol.md, "Replies").
#[test]
fn a_quote_carries_both_levels_and_the_id_of_each() -> StoreResult<()> {
    let rig = actions_rig()?;
    let say = common::english();
    let quote = quotes::of(
        &quoting(
            3,
            11,
            "at eight then",
            ReplyToDto {
                parent: under(1, ME, "Dinner at 7?"),
                ..answers(2, 12, "make it eight")
            },
        ),
        &rig.chats,
        &say,
        |_| false,
    )?
    .expect("a quote");

    assert_eq!(
        QuoteLine::new(2, "Carl", "make it eight", false),
        quote.reply
    );
    assert_eq!(
        Some(QuoteLine::new(1, "You", "Dinner at 7?", false)),
        quote.parent
    );
    Ok(())
}

/// Each level is hidden on its own account and revealed on its own — the one-tap rule, per level
/// (docs/protocol.md, "Blocking a member"). The excerpt of a hidden level is never drawn: the server
/// sends it unchanged, and 120 characters of a blocked member is exactly what the rule exists to
/// keep out of somebody else's bubble.
#[test]
fn each_level_of_a_quote_hides_and_reveals_on_its_own() -> StoreResult<()> {
    let rig = actions_rig()?;
    let say = common::english();
    let message = quoting(
        3,
        11,
        "ok",
        ReplyToDto {
            parent: under(1, 13, "blocked too"),
            ..answers(2, 13, "blocked words")
        },
    );

    let hidden = quotes::of(&message, &rig.chats, &say, |_| false)?.unwrap();
    assert_eq!(
        QuoteLine::new(2, "", "Replying to a hidden message", true),
        hidden.reply
    );
    assert_eq!(
        Some(QuoteLine::new(
            1,
            "",
            "which replied to a hidden message",
            true
        )),
        hidden.parent
    );

    let outer = quotes::of(&message, &rig.chats, &say, |level| {
        level == QuoteLevel::Reply
    })?
    .unwrap();
    assert_eq!(
        QuoteLine::new(2, "Dora", "blocked words", false),
        outer.reply
    );
    assert!(outer.parent.unwrap().hidden);

    let both = quotes::of(&message, &rig.chats, &say, |_| true)?.unwrap();
    assert_eq!("blocked words", both.reply.excerpt);
    assert_eq!(
        Some(QuoteLine::new(1, "Dora", "blocked too", false)),
        both.parent
    );
    Ok(())
}

/// WHAT A CLICK ON A QUOTE DOES, and in which order. A masked level is shown before anything moves —
/// outermost first — so a click never carries the reader to a bubble whose words they chose not to
/// see; with nothing masked left it goes to the message (docs/protocol.md, "Blocking a member").
#[test]
fn a_click_on_a_quote_reveals_what_is_masked_before_it_goes_anywhere() -> StoreResult<()> {
    let rig = actions_rig()?;
    let say = common::english();
    let doubled = quoting(
        3,
        11,
        "ok",
        ReplyToDto {
            parent: under(1, 13, "blocked too"),
            ..answers(2, 13, "blocked words")
        },
    );

    assert_eq!(
        QuoteClick::RevealReply,
        quotes::click_on(&quotes::of(&doubled, &rig.chats, &say, |_| false)?.unwrap())
    );
    assert_eq!(
        QuoteClick::RevealParent,
        quotes::click_on(
            &quotes::of(&doubled, &rig.chats, &say, |level| level
                == QuoteLevel::Reply)?
            .unwrap()
        )
    );
    assert_eq!(
        QuoteClick::GoToMessage,
        quotes::click_on(&quotes::of(&doubled, &rig.chats, &say, |_| true)?.unwrap())
    );

    // The ordinary case: one level, nobody blocked, one click and it goes.
    let plain = quoting(4, 11, "yes", answers(1, 12, "Dinner at 7?"));
    let quote = quotes::of(&plain, &rig.chats, &say, |_| false)?.unwrap();
    assert_eq!(QuoteClick::GoToMessage, quotes::click_on(&quote));
    // And it goes to the message the reply answers, which is the id the quote carries.
    assert_eq!(1, quote.reply.message_id);
    Ok(())
}
