//! `ComposerRowTests`: the message row and the wide window (docs/composer-and-wide-layout-2026-10-20.md,
//! issue #106) — which glyphs sit inside the field for each chat, the column and bubble widths, and
//! the least window that keeps the field a run of text. The C#'s `…Epx` names are logical pixels
//! here, without the suffix.

use fc_core::protocol::AssistantDto;
use fc_logic::composer_row::{self, FieldGlyph, PaneArrangement, conversation_column};

use FieldGlyph::*;

/// `new AssistantDto(99, "Assistant", "@ai", Processor: "Microsoft — Azure OpenAI")`.
fn talks() -> AssistantDto {
    AssistantDto {
        user_id: 99,
        display_name: "Assistant".to_string(),
        mention: "@ai".to_string(),
        processor: Some("Microsoft — Azure OpenAI".to_string()),
        ..AssistantDto::default()
    }
}

/// `new AssistantDto(99, "Assistant", "@ai", Images: true, Processor: "Microsoft — Azure OpenAI")`.
fn draws() -> AssistantDto {
    AssistantDto {
        images: true,
        ..talks()
    }
}

/// The family and direct chats: the stickers always — empty or typing — and the video button only
/// where S1.4 shows it.
#[test]
fn the_stickers_sit_in_the_field_in_every_chat_and_the_video_button_beside_them() {
    let talks = talks();
    assert_eq!(
        vec![Sticker],
        composer_row::in_field(Some("family"), false, true, Some(&talks), false)
    );
    assert_eq!(
        vec![Sticker, VideoDoor],
        composer_row::in_field(Some("family"), false, true, Some(&talks), true)
    );
    assert_eq!(
        vec![Sticker, VideoDoor],
        composer_row::in_field(Some("direct"), false, true, None, true)
    );
    assert_eq!(
        vec![VideoDoor],
        composer_row::in_field(Some("direct"), false, false, None, true)
    );
    assert!(composer_row::in_field(Some("direct"), false, false, None, false).is_empty());
}

/// The assistant's chat: the paintbrush stays a button, inside the field beside the stickers, where
/// it draws.
#[test]
fn the_assistants_chat_carries_the_paintbrush_beside_the_stickers() {
    assert_eq!(
        vec![Sticker, Picture],
        composer_row::in_field(Some("ai"), false, true, Some(&draws()), false)
    );
    assert_eq!(
        vec![Sticker],
        composer_row::in_field(Some("ai"), false, true, Some(&talks()), false)
    );
    // A server that names no processor offers no paintbrush (docs/protocol.md, "Consenting to the
    // assistant").
    let nameless = AssistantDto {
        processor: None,
        ..draws()
    };
    assert_eq!(
        vec![Sticker],
        composer_row::in_field(Some("ai"), false, true, Some(&nameless), false)
    );
    // No @ai button in the family chat any more: the paperclip's menu and the @ strip reach it.
    assert!(
        !composer_row::in_field(Some("family"), false, true, Some(&draws()), false)
            .contains(&Picture)
    );
}

/// An edit carries no sticker and no paintbrush — a sticker is its own message, and an edit cannot
/// become a request.
#[test]
fn an_edit_carries_nothing_in_the_field() {
    assert!(composer_row::in_field(Some("family"), true, true, Some(&talks()), false).is_empty());
    assert!(composer_row::in_field(Some("ai"), true, true, Some(&draws()), false).is_empty());
    assert!(composer_row::in_field(None, false, true, Some(&talks()), false).is_empty());
}

/// The thread's field: the stickers where it can send; its Send disc lit only with a root and
/// something to say.
#[test]
fn the_threads_field_is_the_same_field_and_its_send_needs_words() {
    assert_eq!(vec![Sticker], composer_row::in_thread_field(true, true));
    assert!(composer_row::in_thread_field(false, true).is_empty());
    assert!(composer_row::in_thread_field(true, false).is_empty());
    assert!(composer_row::thread_send_enabled(
        true,
        Some("Sunday works")
    ));
    assert!(!composer_row::thread_send_enabled(true, Some("  \n ")));
    assert!(!composer_row::thread_send_enabled(true, None));
    assert!(!composer_row::thread_send_enabled(
        false,
        Some("Sunday works")
    ));
}

/// The field makes one 44-px target of room per glyph it carries, so text never runs under a
/// button.
#[test]
fn the_field_makes_room_for_every_glyph() {
    assert_eq!(44, composer_row::TARGET);
    assert_eq!(0, composer_row::trailing_room(&[]));
    assert_eq!(44, composer_row::trailing_room(&[Sticker]));
    assert_eq!(88, composer_row::trailing_room(&[Sticker, Picture]));
    let most = conversation_column::MOST_FIELD_GLYPHS * composer_row::TARGET;
    assert!(
        composer_row::trailing_room(&composer_row::in_field(
            Some("ai"),
            false,
            true,
            Some(&draws()),
            false
        )) <= most
    );
    assert!(
        composer_row::trailing_room(&composer_row::in_field(
            Some("family"),
            false,
            true,
            Some(&talks()),
            true
        )) <= most
    );
}

/// The column: min(pane − 32, 1024), centred — the composer exactly as wide as the messages above
/// it.
#[test]
fn the_column_is_the_pane_less_its_insets_up_to_1024() {
    assert_eq!(1024.0, conversation_column::width(1620.0));
    assert_eq!(1024.0, conversation_column::width(1056.0));
    assert_eq!(968.0, conversation_column::width(1000.0));
    assert_eq!(420.0, conversation_column::width(452.0));
    assert_eq!(0.0, conversation_column::width(20.0));
    assert_eq!(0.0, conversation_column::width(0.0));
}

/// The jump button: 12 inside the column's trailing edge — 28 from the pane's while the column
/// fills it, then further in.
#[test]
fn the_jump_button_stands_inside_the_column() {
    assert_eq!(28.0, conversation_column::jump_inset(452.0));
    assert_eq!(28.0, conversation_column::jump_inset(1056.0));
    assert_eq!(310.0, conversation_column::jump_inset(1620.0));
    assert_eq!(22.0, conversation_column::jump_inset(20.0));
    assert_eq!(12.0, conversation_column::jump_inset(0.0));
    for pane in [452.0, 700.0, 1056.0, 1300.0, 1620.0, 2400.0] {
        let width = conversation_column::width(pane);
        let column_right = ((pane - width) / 2.0) + width;
        assert_eq!(
            column_right - conversation_column::JUMP_INSIDE,
            pane - conversation_column::jump_inset(pane)
        );
    }
}

/// A bubble: min(column − 48, 560) — and the margin-plus-cap the view draws it with is exactly
/// that.
#[test]
fn a_bubble_is_the_column_less_48_up_to_560() {
    assert_eq!(560.0, conversation_column::bubble_max(1024.0));
    assert_eq!(560.0, conversation_column::bubble_max(608.0));
    assert_eq!(500.0, conversation_column::bubble_max(548.0));
    assert_eq!(0.0, conversation_column::bubble_max(30.0));
    for column in [0.0, 30.0, 200.0, 420.0, 560.0, 608.0, 700.0, 968.0, 1024.0] {
        // `Math.Max(0, Math.Min(column − spacer, max))`, as the view draws it — written out, not
        // as the `clamp` the rule itself uses, so the two are compared.
        #[allow(clippy::manual_clamp)]
        let drawn = f64::max(
            0.0,
            f64::min(
                column - conversation_column::BUBBLE_SPACER,
                conversation_column::BUBBLE_MAX,
            ),
        );
        assert_eq!(conversation_column::bubble_max(column), drawn);
    }
}

/// The field's floor is arithmetic: at the least pane the run of text is exactly 200, with two
/// glyphs in the field.
#[test]
fn the_least_pane_leaves_the_field_its_run_of_text() {
    let target = f64::from(composer_row::TARGET);
    assert_eq!(452.0, conversation_column::MIN_PANE);
    let column = conversation_column::width(conversation_column::MIN_PANE);
    let field = column
        - (2.0 * conversation_column::CARD_CHROME)
        - (2.0 * target)
        - (2.0 * conversation_column::ROW_SPACING);
    let run = field
        - conversation_column::FIELD_CHROME
        - (f64::from(conversation_column::MOST_FIELD_GLYPHS) * target);
    assert_eq!(conversation_column::MIN_TEXT_RUN, run);
    assert_eq!(777.0, conversation_column::MINIMUM_WINDOW_WIDTH);
}

/// Wide: the list as drawn, the thread beside. Narrower: the list gives first, down to its least;
/// then the thread goes over the conversation instead of beside it — and the pane never drops
/// under its floor at the least window.
#[test]
fn the_list_narrows_then_the_thread_goes_over_the_conversation() {
    assert_eq!(
        PaneArrangement {
            list_width: 300.0,
            thread_beside: false,
            pane_width: 1619.0,
        },
        conversation_column::arrange(1920.0, false)
    );
    assert_eq!(
        PaneArrangement {
            list_width: 300.0,
            thread_beside: true,
            pane_width: 1219.0,
        },
        conversation_column::arrange(1920.0, true)
    );
    // 1150 wide: the list narrows to keep the pane its floor beside the thread.
    let tight = conversation_column::arrange(1150.0, true);
    assert!(tight.thread_beside);
    assert_eq!(1150.0 - 1.0 - 400.0 - 452.0, tight.list_width);
    assert_eq!(452.0, tight.pane_width);
    // Too narrow for both: the thread goes over the conversation, and the list keeps what it can.
    let over = conversation_column::arrange(1000.0, true);
    assert!(!over.thread_beside);
    assert_eq!(300.0, over.list_width);
    assert_eq!(699.0, over.pane_width);
    // The least window's content: the rail and the frame aside, the list at its least and the pane
    // at its floor.
    let least = conversation_column::MINIMUM_WINDOW_WIDTH
        - conversation_column::RAIL
        - conversation_column::FRAME;
    for thread in [false, true] {
        let arranged = conversation_column::arrange(least, thread);
        assert_eq!(conversation_column::LIST_MIN, arranged.list_width);
        assert_eq!(conversation_column::MIN_PANE, arranged.pane_width);
        assert!(!arranged.thread_beside);
    }
    // Every width from the least window up keeps the pane at its floor or above.
    let mut view = least;
    while view <= 2400.0 {
        assert!(
            conversation_column::arrange(view, true).pane_width >= conversation_column::MIN_PANE
        );
        assert!(
            conversation_column::arrange(view, false).pane_width >= conversation_column::MIN_PANE
        );
        view += 7.0;
    }
}
