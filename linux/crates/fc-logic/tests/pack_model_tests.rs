//! `PackModelTests`: the family's sticker pack from the window's side (docs/protocol.md, "Sticker
//! pack") — adding, removing under the author-or-owner rule, deciding whether the pack holds a sent
//! sticker, and what one click in the panel sends.
//!
//! These are about the CHAT sticker. The board's cards are called stickers in this code too
//! (`BoardModelTests`), and nothing here touches them.
//!
//! `ABlockedMembersItemsAreInThePanelLikeAnybodys` reads `ChatListModel` too (`chat_list`). The
//! file's second class, `PackPickingTests`, is `pack_picking_tests.rs`.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use fc_core::protocol::send_rules::FlushTrigger;
use fc_core::protocol::{AttachmentDto, PackItemDto, SendPipeline, error_codes};
use fc_core::store::{ChatStore, PackLimits, PackStore, SeqRoute, StoreResult};
use fc_core::testing::{FakeFrameSender, StillClock, drive};
use fc_logic::attachment_cache::AttachmentCache;
use fc_logic::chat_list::ChatListModel;
use fc_logic::conversation::ConversationModel;
use fc_logic::media_outbox::MediaOutbox;
use fc_logic::pack::{PackAdded, PackModel, PackPanel, pack_label, pack_text};

use common::english;
use common::media::{Blobs, Staging};
use common::pack::{
    CEILING, ME, NOW, PackRig, Wire, claimed, item, item_by, item_sized, item_with, png, refusal,
    sent_sticker, uploaded, webp, webp_default,
};

const WEBP: &str = "image/webp";

fn held(blobs: &Blobs, key: &str) -> Option<Vec<u8>> {
    blobs.0.borrow().get(key).cloned()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("UTF-8")
}

fn ids(items: &[PackItemDto]) -> Vec<i64> {
    items.iter().map(|item| item.id).collect()
}

/// The upload and claim routes most adds take: attachment 71, item 5 at seq 12.
fn adding(wire: &Wire) -> &Wire {
    wire.on("POST", "/attachments", &[(201, Some(&uploaded(71, WEBP)))])
        .on(
            "POST",
            "/families/mine/pack",
            &[(201, Some(&claimed(5, 71, 12, ME)))],
        )
}

// ---- whether there is a pack at all ------------------------------------------------------

/// A server that predates packs names no ceilings, and then nothing is offered — rather than a 404
/// when somebody taps a button.
#[test]
fn the_pack_is_offered_only_where_the_server_named_its_ceilings() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let model = rig.build(&Wire::new()).model;
    assert!(model.offered()?);

    rig.pack.set_limits(None)?;

    assert!(!model.offered()?);
    assert!(!model.is_full()?);
    Ok(())
}

// ---- adding --------------------------------------------------------------------------------

/// THE BYPASS. A sticker goes up as the bytes it is — `kind=photo`, its own type, its own size —
/// and with NO preview: a preview is a JPEG, which has no transparency and one frame.
#[test]
fn adding_uploads_the_original_bytes_unprepared_and_then_claims_them() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let bytes = webp(512, 384, true, 4000, 0xAB);
    let wire = Wire::new();
    adding(&wire);
    let built = rig.build(&wire);
    rig.pack.replace(&[], 0)?;

    let added = drive(&rig.clock, built.model.add(&bytes, Some("  party cat ")))?;

    assert_eq!(Ok(PackAdded::Added), added);
    assert_eq!(
        vec![
            "POST /api/v1/attachments?kind=photo&width=512&height=384",
            "POST /api/v1/families/mine/pack"
        ],
        wire.paths()
    );
    let requests = wire.requests();
    // Byte for byte: not downscaled, not re-encoded, not stripped — and typed by what the bytes ARE.
    assert_eq!(bytes, requests[0].body);
    assert_eq!(Some("image/webp"), requests[0].content_type.as_deref());
    assert_eq!(
        r#"{"attachment_id":71,"label":"party cat"}"#,
        text(&requests[1].body)
    );
    // No `PUT …/preview` was ever sent.
    assert!(!requests.iter().any(|asked| asked.path.contains("preview")));

    assert_eq!(vec![5], ids(&rig.pack.items()?));
    // The answer to this device's own POST moves no cursor.
    assert_eq!(0, rig.pack.cursor()?);
    // And the bytes are kept under the id the pack holds them by, so the panel draws at once.
    assert_eq!(Some(bytes), held(&built.blobs, "71"));
    Ok(())
}

#[test]
fn a_png_is_a_sticker_too() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let bytes = png(200, 100, 64);
    let wire = Wire::new();
    wire.on(
        "POST",
        "/attachments",
        &[(201, Some(&uploaded(71, "image/png")))],
    )
    .on(
        "POST",
        "/families/mine/pack",
        &[(201, Some(&claimed(5, 71, 12, ME)))],
    );
    let built = rig.build(&wire);

    let added = drive(&rig.clock, built.model.add(&bytes, None))?;

    assert_eq!(Ok(PackAdded::Added), added);
    let requests = wire.requests();
    assert_eq!(Some("image/png"), requests[0].content_type.as_deref());
    assert_eq!(
        "/api/v1/attachments?kind=photo&width=200&height=100",
        requests[0].path
    );
    // No label was given, so no key was sent.
    assert_eq!(r#"{"attachment_id":71}"#, text(&requests[1].body));
    Ok(())
}

/// ADDING THE SAME STICKER TWICE IS NOT AN ERROR AND NOT TWO STICKERS. The server answers `200`
/// with the item that was there — whose attachment id is NOT the one just uploaded.
#[test]
fn adding_what_the_pack_already_holds_answers_the_item_that_was_there() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let bytes = webp_default();
    rig.pack.replace(&[item(5, 12)], 12)?;
    let wire = Wire::new();
    wire.on("POST", "/attachments", &[(201, Some(&uploaded(99, WEBP)))])
        // The fresh upload (99) was dropped: the item carries the id the pack already had (75).
        .on(
            "POST",
            "/families/mine/pack",
            &[(200, Some(&claimed(5, 75, 12, ME)))],
        );
    let built = rig.build(&wire);

    let added = drive(&rig.clock, built.model.add(&bytes, None))?;

    assert_eq!(Ok(PackAdded::AlreadyThere), added);
    assert_eq!(1, rig.pack.items()?.len());
    assert_eq!(Some(bytes), held(&built.blobs, "75"));
    assert_eq!(None, held(&built.blobs, "99"));
    Ok(())
}

/// THE FRAME CAN BEAT THE ANSWER. The server fans the actor's own `pack_item` frame out before the
/// POST answers, so a genuinely NEW sticker is very often already in the cache when its claim comes
/// back. That is still "Added": `201` says so, and the cache is not asked.
#[test]
fn a_new_sticker_whose_frame_arrived_first_is_still_reported_as_added() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let bytes = webp_default();
    rig.pack.replace(&[], 0)?;
    rig.pack.caught_up(rig.pack.connection());
    let wire = Wire::new();
    adding(&wire);
    let built = rig.build(&wire);
    let pack = rig.pack.clone();
    wire.on_seen(move |asked| {
        if asked.method == "POST" && asked.path.ends_with("/families/mine/pack") {
            // The socket delivers this device's own add while the claim is still in the air.
            let mut frame = item_sized(5, 12, 64);
            frame.attachment = Some(AttachmentDto {
                mime: Some(WEBP.to_string()),
                size: Some(64),
                width: Some(512),
                height: Some(512),
                ..AttachmentDto::new(71, "photo")
            });
            assert!(
                pack.apply(&frame, SeqRoute::LiveFrame)
                    .expect("the cache takes it")
            );
        }
    });

    let added = drive(&rig.clock, built.model.add(&bytes, None))?;

    assert!(rig.pack.item(5)?.is_some());
    assert_eq!(Ok(PackAdded::Added), added);
    assert_eq!(
        "Added to family stickers",
        pack_text::added(PackAdded::Added, &english())
    );
    assert_eq!(vec![5], ids(&rig.pack.items()?));
    // The frame moved the cursor (this connection had caught up); the answer moved nothing.
    assert_eq!(12, rig.pack.cursor()?);
    assert_eq!(Some(bytes), held(&built.blobs, "71"));
    Ok(())
}

/// The other order: the answer first, and the frame after it — one sticker, added once.
#[test]
fn a_new_sticker_whose_frame_arrives_after_the_answer_is_added_once() -> StoreResult<()> {
    let rig = PackRig::new()?;
    rig.pack.replace(&[], 0)?;
    rig.pack.caught_up(rig.pack.connection());
    let wire = Wire::new();
    adding(&wire);
    let built = rig.build(&wire);

    let added = drive(&rig.clock, built.model.add(&webp_default(), None))?;

    assert_eq!(Ok(PackAdded::Added), added);
    assert_eq!(0, rig.pack.cursor()?);
    // Then the frame: the same item at the same seq changes nothing drawn, and moves the cursor.
    assert!(!rig.pack.apply(&item(5, 12), SeqRoute::LiveFrame)?);
    assert_eq!(vec![5], ids(&rig.pack.items()?));
    assert_eq!(12, rig.pack.cursor()?);
    Ok(())
}

/// And the converse: `200` is "the pack already held it" even when THIS DEVICE had never heard of
/// the item — a pack it has not read yet, or an add by somebody else it has not caught up with. The
/// status says which, never the cache.
#[test]
fn a_sticker_the_pack_already_held_is_said_so_even_when_this_device_did_not_hold_it()
-> StoreResult<()> {
    let rig = PackRig::new()?;
    let wire = Wire::new();
    wire.on("POST", "/attachments", &[(201, Some(&uploaded(99, WEBP)))])
        .on(
            "POST",
            "/families/mine/pack",
            &[(200, Some(&claimed(5, 75, 12, 9)))],
        );
    let built = rig.build(&wire);
    assert!(rig.pack.items()?.is_empty());

    let added = drive(&rig.clock, built.model.add(&webp_default(), None))?;

    assert_eq!(Ok(PackAdded::AlreadyThere), added);
    assert_eq!(
        "The family already has that sticker.",
        pack_text::added(PackAdded::AlreadyThere, &english())
    );
    // The item is news all the same, and is held from here.
    assert_eq!(vec![5], ids(&rig.pack.items()?));
    Ok(())
}

// ---- the label -----------------------------------------------------------------------------

/// AT MOST 64 CHARACTERS, COUNTED THE WAY THE SERVER COUNTS: Unicode scalar values after trimming.
/// Not UTF-16 units — sixty-four emoji are 128 of those and are a fine label — and not what a screen
/// shows: a family of four joined emoji is seven scalar values.
#[test]
fn a_label_is_counted_in_scalar_values_after_trimming() {
    assert_eq!(64, pack_label::MAX_LENGTH);
    assert_eq!(0, pack_label::length(None));
    assert_eq!(0, pack_label::length(Some("   \t ")));
    assert_eq!(9, pack_label::length(Some("  party cat \n")));
    assert_eq!(
        Some("party cat".to_string()),
        pack_label::clean(Some("  party cat \n"))
    );
    assert_eq!(None, pack_label::clean(Some("   ")));
    assert_eq!(None, pack_label::clean(None));

    let sixty_four = "🎉".repeat(64);
    // UTF-16 units, as the C#'s `string.Length` counts them.
    assert_eq!(128, sixty_four.encode_utf16().count());
    assert_eq!(64, pack_label::length(Some(&sixty_four)));
    assert!(!pack_label::too_long(Some(&sixty_four)));
    assert!(pack_label::too_long(Some(&format!("{sixty_four}🎉"))));
    // Spaces at the ends are not counted, because they are not sent.
    assert!(!pack_label::too_long(Some(&format!(
        "  {}  ",
        "a".repeat(64)
    ))));
    assert!(pack_label::too_long(Some(&"a".repeat(65))));
    assert!(!pack_label::too_long(Some("Привет, 家族！")));
    assert_eq!(7, pack_label::length(Some("👨‍👩‍👧‍👦")));
    // A combining accent is a scalar value of its own, as the server counts it.
    assert_eq!(2, pack_label::length(Some("e\u{0301}")));
}

/// An over-long label is refused BEFORE ANY REQUEST, and in words.
#[test]
fn an_over_long_label_is_refused_before_anything_is_sent() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let built = rig.build(&Wire::new());
    let too_long = "a".repeat(65);

    let added = drive(
        &rig.clock,
        built.model.add(&webp_default(), Some(&too_long)),
    )?;
    let from_message = drive(
        &rig.clock,
        built.model.add_from_message(
            &AttachmentDto {
                width: Some(512),
                height: Some(512),
                ..sent_sticker(90, WEBP, Some(64))
            },
            Some(&too_long),
        ),
    )?;

    let error = added.expect_err("refused");
    assert_eq!(pack_label::TOO_LONG_CODE, error.code);
    assert_eq!(
        pack_label::TOO_LONG_CODE,
        from_message.expect_err("refused").code
    );
    assert!(built.wire.requests().is_empty());
    assert_eq!(
        "A sticker's label can be at most 64 characters.",
        pack_text::sentence(&error, &english())
    );
    Ok(())
}

/// The longest label there is goes, trimmed, exactly as it was counted.
#[test]
fn a_label_of_sixty_four_scalar_values_is_sent_trimmed() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let label = "🎉".repeat(64);
    let wire = Wire::new();
    adding(&wire);
    let built = rig.build(&wire);

    let added = drive(
        &rig.clock,
        built
            .model
            .add(&webp_default(), Some(&format!("  {label} "))),
    )?;

    assert_eq!(Ok(PackAdded::Added), added);
    let sent: serde_json::Value =
        serde_json::from_slice(&wire.requests()[1].body).expect("a JSON body");
    assert_eq!(Some(label.as_str()), sent["label"].as_str());
    Ok(())
}

/// "Add to family stickers" on a sent sticker carries the label it was given, too.
#[test]
fn a_sent_sticker_is_added_with_the_label_it_was_given() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let bytes = webp_default();
    let wire = Wire::new();
    wire.bytes(90, &bytes);
    adding(&wire);
    let built = rig.build(&wire);

    let added = drive(
        &rig.clock,
        built.model.add_from_message(
            &AttachmentDto {
                width: Some(512),
                height: Some(512),
                ..sent_sticker(90, WEBP, Some(bytes.len() as i64))
            },
            Some(" party cat "),
        ),
    )?;

    assert_eq!(Ok(PackAdded::Added), added);
    assert_eq!(
        r#"{"attachment_id":71,"label":"party cat"}"#,
        text(&wire.requests().last().expect("a request").body)
    );
    Ok(())
}

/// REFUSED WHERE THE PERSON IS CHOOSING, from the ceilings this device already holds: nothing is
/// uploaded to be told no.
#[test]
fn a_picture_over_the_ceiling_or_of_the_wrong_kind_is_refused_before_anything_is_sent()
-> StoreResult<()> {
    let rig = PackRig::new()?;
    let built = rig.build(&Wire::new());
    let over = usize::try_from(CEILING).expect("a ceiling") + 1;

    let too_big = drive(
        &rig.clock,
        built.model.add(&webp(512, 512, false, over, 0xAB), None),
    )?;
    let jpeg = drive(
        &rig.clock,
        built
            .model
            .add(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0], None),
    )?;
    rig.pack.replace(&[item(1, 1), item(2, 2), item(3, 3)], 3)?;
    let full = drive(&rig.clock, built.model.add(&webp_default(), None))?;

    assert_eq!(
        error_codes::PACK_ITEM_TOO_LARGE,
        too_big.expect_err("refused").code
    );
    assert_eq!(
        error_codes::INVALID_ATTACHMENT,
        jpeg.expect_err("refused").code
    );
    assert_eq!(error_codes::PACK_FULL, full.expect_err("refused").code);
    assert!(built.model.is_full()?);
    assert!(built.wire.requests().is_empty());
    Ok(())
}

/// The server checks again — the pack may have filled since this device last looked — and its
/// answer is passed on.
#[test]
fn the_servers_own_refusal_is_passed_on_and_nothing_is_held() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let wire = Wire::new();
    wire.on("POST", "/attachments", &[(201, Some(&uploaded(71, WEBP)))])
        .on(
            "POST",
            "/families/mine/pack",
            &[(409, Some(&refusal("pack_full")))],
        );
    let built = rig.build(&wire);

    let added = drive(&rig.clock, built.model.add(&webp_default(), None))?;

    assert_eq!(error_codes::PACK_FULL, added.expect_err("refused").code);
    assert!(rig.pack.items()?.is_empty());
    assert!(built.blobs.is_empty());
    Ok(())
}

/// `attachment_expired` means the sweep took the upload: upload it again — ONCE, automatically —
/// and claim again. Nothing is shown; the label goes with the second claim too.
#[test]
fn an_expired_upload_keeps_its_label_on_the_second_claim() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let wire = Wire::new();
    wire.on(
        "POST",
        "/attachments",
        &[
            (201, Some(&uploaded(71, WEBP))),
            (201, Some(&uploaded(72, WEBP))),
        ],
    )
    .on(
        "POST",
        "/families/mine/pack",
        &[
            (404, Some(&refusal("attachment_expired"))),
            (201, Some(&claimed(5, 72, 12, ME))),
        ],
    );
    let built = rig.build(&wire);

    let added = drive(
        &rig.clock,
        built.model.add(&webp_default(), Some("party cat")),
    )?;

    assert_eq!(Ok(PackAdded::Added), added);
    assert_eq!(
        vec![
            "POST /api/v1/attachments?kind=photo&width=512&height=512",
            "POST /api/v1/families/mine/pack",
            "POST /api/v1/attachments?kind=photo&width=512&height=512",
            "POST /api/v1/families/mine/pack"
        ],
        wire.paths()
    );
    let requests = wire.requests();
    // The same bytes again, and the claim names the NEW upload.
    assert_eq!(requests[0].body, requests[2].body);
    assert_eq!(
        r#"{"attachment_id":72,"label":"party cat"}"#,
        text(&requests[3].body)
    );
    Ok(())
}

/// `attachment_expired` means the sweep took the upload: upload it again — once.
#[test]
fn an_upload_the_sweep_took_is_uploaded_again() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let wire = Wire::new();
    wire.on(
        "POST",
        "/attachments",
        &[
            (201, Some(&uploaded(71, WEBP))),
            (201, Some(&uploaded(72, WEBP))),
        ],
    )
    .on(
        "POST",
        "/families/mine/pack",
        &[
            (404, Some(&refusal("attachment_expired"))),
            (201, Some(&claimed(5, 72, 12, ME))),
        ],
    );
    let built = rig.build(&wire);

    let added = drive(&rig.clock, built.model.add(&webp_default(), None))?;

    assert_eq!(Ok(PackAdded::Added), added);
    let requests = wire.requests();
    assert_eq!(4, requests.len());
    assert_eq!(r#"{"attachment_id":72}"#, text(&requests[3].body));
    Ok(())
}

#[test]
fn an_upload_that_keeps_expiring_is_not_retried_for_ever() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let wire = Wire::new();
    wire.on("POST", "/attachments", &[(201, Some(&uploaded(71, WEBP)))])
        .on(
            "POST",
            "/families/mine/pack",
            &[(404, Some(&refusal("attachment_expired")))],
        );
    let built = rig.build(&wire);

    let added = drive(&rig.clock, built.model.add(&webp_default(), None))?;

    // Only the SECOND failure is shown: two uploads, two claims, and no third try.
    assert_eq!(
        error_codes::ATTACHMENT_EXPIRED,
        added.expect_err("refused").code
    );
    assert_eq!(4, wire.requests().len());
    assert!(rig.pack.items()?.is_empty());
    Ok(())
}

#[test]
fn an_upload_that_did_not_arrive_is_a_transient_failure_and_claims_nothing() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let built = rig.build(&Wire::new());

    let added = drive(&rig.clock, built.model.add(&webp_default(), None))?;

    assert!(added.expect_err("not added").transient());
    assert_eq!(1, built.wire.requests().len());
    Ok(())
}

/// "Add to family stickers" on a sticker somebody SENT: the message's own bytes, uploaded again
/// unprepared and claimed — read as the original even where the attachment says it has a preview.
#[test]
fn a_sent_sticker_is_added_from_the_messages_own_bytes() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let bytes = webp(512, 512, false, 64, 0x11);
    let sent = AttachmentDto {
        width: Some(512),
        height: Some(512),
        has_preview: true,
        ..sent_sticker(90, WEBP, Some(bytes.len() as i64))
    };
    let wire = Wire::new();
    wire.bytes(90, &bytes)
        .on("POST", "/attachments", &[(201, Some(&uploaded(91, WEBP)))])
        .on(
            "POST",
            "/families/mine/pack",
            &[(201, Some(&claimed(5, 91, 12, ME)))],
        );
    let built = rig.build(&wire);

    let added = drive(&rig.clock, built.model.add_from_message(&sent, None))?;

    assert_eq!(Ok(PackAdded::Added), added);
    assert_eq!("GET /api/v1/attachments/90", wire.paths()[0]);
    let requests = wire.requests();
    assert_eq!(bytes, requests[1].body);
    assert!(!requests.iter().any(|asked| asked.path.contains("preview")));
    Ok(())
}

// ---- who may remove ------------------------------------------------------------------------

/// WHOEVER ADDED IT, OR THE FAMILY OWNER — a shape the board does not have: a member who left
/// leaves their stickers behind, and under an author-only rule nobody could ever remove those.
#[test]
fn removal_is_for_whoever_added_it_or_the_owner() {
    for (added_by, reader, owner, may) in [
        (7, 7, false, true),  // mine
        (9, 7, false, false), // somebody else's, and I am a member
        (9, 7, true, true),   // somebody else's, and I own the family
        (7, 7, true, true),
        (0, 0, false, false), // nobody is signed in: an unknown adder is not "me"
    ] {
        assert_eq!(
            may,
            PackModel::may_remove_by(&item_by(5, 12, added_by), reader, owner),
            "{added_by} {reader} {owner}"
        );
    }
}

#[test]
fn the_reader_is_whoever_is_signed_in() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let model = rig.build(&Wire::new()).model;

    assert!(model.may_remove(&item_by(5, 12, ME), false));
    assert!(!model.may_remove(&item_by(6, 13, 9), false));
    assert!(model.may_remove(&item_by(6, 13, 9), true));
    Ok(())
}

#[test]
fn removing_takes_the_item_out_at_once_and_moves_no_cursor() -> StoreResult<()> {
    let rig = PackRig::new()?;
    rig.pack.replace(&[item(5, 12), item(6, 13)], 13)?;
    rig.pack.used(5, NOW)?;
    let wire = Wire::new();
    wire.on("DELETE", "/families/mine/pack/5", &[(204, None)]);
    let built = rig.build(&wire);

    let error = drive(&rig.clock, built.model.remove(5))?;

    assert_eq!(None, error);
    assert_eq!(vec![6], ids(&rig.pack.items()?));
    // The seq this removal was given is the server's to announce.
    assert_eq!(13, rig.pack.cursor()?);
    // Remembered as gone: the frame for the ADD, arriving late, cannot put it back.
    assert!(!rig.pack.apply(&item(5, 12), SeqRoute::LiveFrame)?);
    assert!(built.model.panel()?.recent.is_empty());
    Ok(())
}

#[test]
fn a_refused_removal_leaves_the_item_where_it_is() -> StoreResult<()> {
    let rig = PackRig::new()?;
    rig.pack.replace(&[item_by(5, 12, 9)], 12)?;
    let wire = Wire::new();
    wire.on(
        "DELETE",
        "/families/mine/pack/5",
        &[(403, Some(&refusal("not_pack_item_author")))],
    );
    let built = rig.build(&wire);

    let error = drive(&rig.clock, built.model.remove(5))?;

    assert_eq!(
        Some(error_codes::NOT_PACK_ITEM_AUTHOR),
        error.as_ref().map(|error| error.code.as_str())
    );
    assert_eq!(1, rig.pack.items()?.len());
    Ok(())
}

/// `pack_item_not_found` ON A REMOVAL MEANS IT IS ALREADY GONE — which is what was asked for. The
/// item is dropped here, remembered as gone, and NO ERROR comes back to be shown.
#[test]
fn removing_an_item_that_is_already_gone_drops_it_and_says_nothing() -> StoreResult<()> {
    let rig = PackRig::new()?;
    rig.pack.replace(&[item(5, 12), item(6, 13)], 13)?;
    rig.pack.used(5, NOW)?;
    let wire = Wire::new();
    wire.on(
        "DELETE",
        "/families/mine/pack/5",
        &[(404, Some(&refusal("pack_item_not_found")))],
    );
    let built = rig.build(&wire);

    let error = drive(&rig.clock, built.model.remove(5))?;

    assert_eq!(None, error);
    assert_eq!(vec![6], ids(&rig.pack.items()?));
    assert!(built.model.panel()?.recent.is_empty());
    assert_eq!(13, rig.pack.cursor()?);
    // Gone for good: an older copy arriving late cannot put it back.
    assert!(!rig.pack.apply(&item(5, 12), SeqRoute::LiveFrame)?);
    assert_eq!(1, wire.requests().len());
    Ok(())
}

/// Any OTHER 404 is not "already gone", and a failure that is not a refusal leaves the item where
/// it is.
#[test]
fn only_the_packs_own_not_found_is_read_as_gone() -> StoreResult<()> {
    let rig = PackRig::new()?;
    rig.pack.replace(&[item(5, 12)], 12)?;
    let wire = Wire::new();
    wire.on(
        "DELETE",
        "/families/mine/pack/5",
        &[(404, Some(&refusal("not_found"))), (502, None)],
    );
    let built = rig.build(&wire);

    let first = drive(&rig.clock, built.model.remove(5))?.expect("refused");
    let second = drive(&rig.clock, built.model.remove(5))?.expect("failed");

    assert_eq!("not_found", first.code);
    assert!(second.transient());
    assert_eq!(1, rig.pack.items()?.len());
    Ok(())
}

// ---- whether the pack holds a sent sticker ---------------------------------------------------

/// Decided HERE, from bytes this device already has: an item whose size and type match and whose
/// bytes are the same. Nothing on the wire names the item a message was sent from.
#[test]
fn the_pack_holds_a_sticker_whose_size_type_and_bytes_match() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let bytes = webp(512, 512, false, 64, 0x11);
    rig.pack.replace(
        &[
            item_sized(5, 12, bytes.len()),
            item_sized(6, 13, bytes.len()),
        ],
        13,
    )?;
    let wire = Wire::new();
    wire.bytes(90, &bytes)
        .bytes(75, &webp(512, 512, false, 64, 0x22))
        .bytes(76, &bytes);
    let built = rig.build(&wire);

    let held = drive(
        &rig.clock,
        built
            .model
            .holds(&sent_sticker(90, WEBP, Some(bytes.len() as i64))),
    )?;

    assert!(held);
    Ok(())
}

#[test]
fn the_same_size_is_not_the_same_sticker() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let bytes = webp(512, 512, false, 64, 0x11);
    rig.pack.replace(&[item_sized(5, 12, bytes.len())], 12)?;
    let wire = Wire::new();
    wire.bytes(90, &bytes)
        .bytes(75, &webp(512, 512, false, 64, 0x22));
    let built = rig.build(&wire);

    assert!(!drive(
        &rig.clock,
        built
            .model
            .holds(&sent_sticker(90, WEBP, Some(bytes.len() as i64)))
    )?);
    Ok(())
}

/// A size or a type that matches nothing is answered without fetching a byte.
#[test]
fn a_sticker_no_item_could_be_is_not_held_and_nothing_is_fetched() -> StoreResult<()> {
    let rig = PackRig::new()?;
    rig.pack.replace(
        &[
            item_sized(5, 12, 64),
            item_with(6, 13, ME, Some(64), "image/png", false),
        ],
        13,
    )?;
    let built = rig.build(&Wire::new());

    let holds = |picture: AttachmentDto| drive(&rig.clock, built.model.holds(&picture));
    assert!(!holds(sent_sticker(90, WEBP, Some(65)))?);
    assert!(!holds(sent_sticker(90, WEBP, None))?);
    assert!(!holds(sent_sticker(90, "image/jpeg", Some(9999)))?);
    assert!(built.wire.requests().is_empty());
    Ok(())
}

/// Bytes that cannot be fetched read as "not held", which only offers what the pack may already
/// have — and the server answers THAT with the item that was there.
#[test]
fn a_sticker_whose_bytes_cannot_be_read_is_offered_rather_than_guessed_at() -> StoreResult<()> {
    let rig = PackRig::new()?;
    rig.pack.replace(&[item_sized(5, 12, 64)], 12)?;
    let built = rig.build(&Wire::new());

    assert!(!drive(
        &rig.clock,
        built.model.holds(&sent_sticker(90, WEBP, Some(64)))
    )?);
    Ok(())
}

// ---- sending -------------------------------------------------------------------------------

/// What one click sends: the item's bytes AS CACHED, as one photo with NO preview — and read as the
/// original even where `has_preview` is true, which it can be by dedup inheritance.
#[test]
fn a_sticker_is_sent_as_the_items_own_bytes_with_no_preview() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let bytes = webp(512, 384, true, 900, 0xAB);
    rig.pack.replace(
        &[item_with(5, 12, ME, Some(bytes.len() as i64), WEBP, true)],
        12,
    )?;
    let wire = Wire::new();
    wire.bytes(75, &bytes);
    let built = rig.build(&wire);
    let five = rig.pack.item(5)?.expect("held");

    let media = drive(&rig.clock, built.model.to_send(&five))?.expect("staged");

    assert_eq!("photo", media.kind);
    assert_eq!("image/webp", media.mime);
    assert_eq!(bytes, media.bytes);
    assert_eq!(None, media.preview);
    assert_eq!((Some(512), Some(512)), (media.width, media.height));
    // The original, never `/preview`.
    assert_eq!(vec!["GET /api/v1/attachments/75"], wire.paths());

    // Kept: the next click is not a download, and neither is a send with no network at all.
    drive(&rig.clock, built.model.to_send(&five))?.expect("staged");
    assert_eq!(1, wire.requests().len());
    Ok(())
}

/// A sticker is a COPY of bytes this device holds. An item it has never fetched, on a device that
/// is offline, cannot be sent until they are here — and that is a transient failure, said as one.
#[test]
fn an_item_whose_bytes_are_not_here_cannot_be_sent_offline() -> StoreResult<()> {
    let rig = PackRig::new()?;
    rig.pack.replace(&[item(5, 12)], 12)?;
    let built = rig.build(&Wire::new());
    let five = rig.pack.item(5)?.expect("held");

    let media = drive(&rig.clock, built.model.to_send(&five))?;

    assert!(media.expect_err("not here").transient());
    Ok(())
}

/// The per-item ceiling binds a sticker MESSAGE too, and an operator may have lowered it since.
#[test]
fn an_item_over_todays_ceiling_is_refused_here_rather_than_as_a_failed_bubble() -> StoreResult<()> {
    let rig = PackRig::new()?;
    let bytes = webp(512, 512, false, 2000, 0xAB);
    rig.pack.replace(&[item_sized(5, 12, bytes.len())], 12)?;
    rig.pack.set_limits(Some(PackLimits {
        max_items: 200,
        max_item_bytes: 1000,
    }))?;
    let wire = Wire::new();
    wire.bytes(75, &bytes);
    let built = rig.build(&wire);
    let five = rig.pack.item(5)?.expect("held");

    let media = drive(&rig.clock, built.model.to_send(&five))?;

    assert_eq!(
        error_codes::PACK_ITEM_TOO_LARGE,
        media.expect_err("refused").code
    );
    Ok(())
}

/// THE WHOLE SEND, OFFLINE FIRST. One click writes a row down with its staged bytes; the media pump
/// uploads those bytes untouched and with no preview; and the message that is then posted says
/// `sticker: true`, has no body, and may be a reply.
#[test]
fn one_click_queues_a_message_that_lands_as_a_sticker_whenever_the_network_lets_it()
-> StoreResult<()> {
    const DELIVERED: &str = r#"
        {"message": {"id": 1340, "chat_id": 42, "sender_id": 7, "client_msg_id": "c-1", "body": "",
         "created_at": "2026-09-13T10:00:00Z",
         "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "size": 700, "sticker": true}]}}
        "#;
    const HANDLE: &str = "0123456789abcdef0123456789abcdef";
    let rig = PackRig::new()?;
    let bytes = webp(512, 512, true, 700, 0xAB);
    rig.pack.replace(&[item_sized(5, 12, bytes.len())], 12)?;
    let wire = Wire::new();
    wire.bytes(75, &bytes)
        .on("POST", "/attachments", &[(201, Some(&uploaded(90, WEBP)))])
        .on("POST", "/chats/42/messages", &[(201, Some(DELIVERED))]);
    let built = rig.build(&wire);
    let staging = Staging::new();
    let media = Rc::new(MediaOutbox::new(
        rig.outbox.clone(),
        built.api.clone(),
        Rc::new(staging.clone()),
    ));
    let landed: Rc<RefCell<Vec<(bool, i64, Vec<u8>)>>> = Rc::default();
    let heard = landed.clone();
    media.landed.subscribe(move |(owed, attachment, file)| {
        heard
            .borrow_mut()
            .push((owed.sticker, attachment.id, file.bytes.clone()));
    });
    // No socket: every send goes by REST.
    let socket = FakeFrameSender::new();
    socket.set_connected(false);
    socket.set_writes_go_through(false);
    let sending = Rc::new(
        SendPipeline::from_api(
            Rc::new(socket.clone()),
            rig.outbox.clone(),
            rig.chats.clone(),
            built.api.clone(),
            Rc::new(StillClock::at(NOW)),
        )
        .with_next_id(|| "c-1".to_string())
        .with_uploads(media.pump()),
    );
    let chat = ConversationModel::new(
        42,
        rig.chats.clone(),
        built.api.clone(),
        sending.clone(),
        Rc::new(socket),
        Rc::new(rig.clock.clone()),
    )?
    .with_outbox(rig.outbox.clone());

    // The click.
    let five = rig.pack.item(5)?.expect("held");
    let staged = drive(&rig.clock, built.model.to_send(&five))?.expect("staged");
    staging.put(HANDLE, staged);
    let row = chat.send_sticker(HANDLE, Some(1337))?;
    built.model.sent(5)?;

    // Written down BEFORE anything moves: this is the offline send.
    assert!(row.sticker);
    assert_eq!("", row.body);
    let pending = chat.pending()?;
    assert_eq!(1, pending.len());
    assert!(pending[0].owes_uploads());
    assert_eq!(1, wire.requests().len());

    // The network comes back.
    assert_eq!(
        1,
        drive(
            &rig.clock,
            sending.flush(FlushTrigger::ConnectivityRestored)
        )?
    );

    let requests = wire.requests();
    let upload = &requests[1];
    assert_eq!(
        "/api/v1/attachments?kind=photo&width=512&height=512",
        upload.path
    );
    assert_eq!(Some("image/webp"), upload.content_type.as_deref());
    assert_eq!(bytes, upload.body);
    assert_eq!(
        r#"{"client_msg_id":"c-1","body":"","reply_to_message_id":1337,"attachment_ids":[90],"sticker":true}"#,
        text(&requests[2].body)
    );
    assert_eq!(3, requests.len());
    assert!(chat.pending()?.is_empty());
    // The upload is announced with the id the server gave it and the bytes that went — which is
    // how the message it becomes is drawn without downloading the picture back.
    let landed = landed.borrow();
    assert_eq!(1, landed.len());
    assert_eq!((true, 90), (landed[0].0, landed[0].1));
    assert_eq!(bytes, landed[0].2);
    // And it is drawn as what it is.
    assert!(
        rig.chats
            .message(1340)?
            .expect("delivered")
            .sticker_picture()
            .is_some()
    );
    assert_eq!(vec![5], ids(&built.model.panel()?.recent));
    Ok(())
}

// ---- the panel -----------------------------------------------------------------------------

#[test]
fn the_panel_leads_with_what_this_device_sent_last_and_then_shows_the_whole_pack() -> StoreResult<()>
{
    let rig = PackRig::new()?;
    rig.pack.set_limits(Some(PackLimits {
        max_items: 200,
        max_item_bytes: CEILING,
    }))?;
    rig.pack
        .replace(&[item(5, 12), item(6, 13), item(7, 14), item(8, 15)], 15)?;
    let model = rig.build(&Wire::new()).model;
    assert!(model.panel()?.recent.is_empty());

    model.sent(7)?;
    rig.clock.advance(Duration::from_secs(60));
    model.sent(5)?;
    rig.clock.advance(Duration::from_secs(60));
    model.sent(7)?;

    let panel = model.panel()?;
    assert_eq!(vec![7, 5], ids(&panel.recent));
    // The whole pack, in the order it was added, recents included: a sticker never moves under a
    // finger.
    assert_eq!(vec![5, 6, 7, 8], ids(&panel.all));
    assert!(!panel.is_empty());
    Ok(())
}

#[test]
fn recents_are_capped_and_forget_what_the_pack_no_longer_holds() {
    let items: Vec<PackItemDto> = (1..=20).map(|id| item(id, id)).collect();
    let recents: Vec<i64> = std::iter::once(99).chain((1..=20).rev()).collect();

    let panel = PackPanel::of(&items, &recents);

    // The last SIXTEEN, the same on every client.
    assert_eq!(16, PackPanel::RECENT_ROOM);
    assert_eq!(16, panel.recent.len());
    assert_eq!(20, panel.recent[0].id);
    assert_eq!(5, panel.recent[15].id);
    assert!(!panel.recent.iter().any(|item| item.id == 99));
    assert!(PackPanel::of(&[], &[5]).is_empty());
}

/// RECENTLY USED IS THIS DEVICE'S, AND IT SURVIVES A RESTART: it is in the cache file, not in a
/// view or a session, so a model built again over the same cache — which is what a relaunch is —
/// leads the panel with the same sixteen. It is cleared at sign-out, with everything else.
#[test]
fn recents_survive_a_restart_are_sixteen_and_are_cleared_at_sign_out() -> StoreResult<()> {
    let rig = PackRig::new()?;
    rig.pack.set_limits(Some(PackLimits {
        max_items: 200,
        max_item_bytes: CEILING,
    }))?;
    let items: Vec<PackItemDto> = (1..=20).map(|id| item(id, id)).collect();
    rig.pack.replace(&items, 20)?;
    let built = rig.build(&Wire::new());
    for id in 1..=20 {
        built.model.sent(id)?;
        rig.clock.advance(Duration::from_secs(1));
    }

    // A relaunch: new stores and a new model over the same cache file.
    let relaunched = PackModel::new(
        Rc::new(PackStore::new(&rig.cache)),
        Rc::new(ChatStore::new(&rig.cache).with_me(|| ME)),
        built.api.clone(),
        Rc::new(AttachmentCache::new(
            built.api.clone(),
            Rc::new(Blobs::new()),
        )),
        Rc::new(rig.clock.clone()),
    );
    let panel = relaunched.panel()?;

    assert_eq!((5..=20).rev().collect::<Vec<i64>>(), ids(&panel.recent));
    // Shown first, and the whole pack after them in the order it was added.
    assert_eq!(20, panel.all.len());

    // Sign-out (AppSession::sign_out) wipes the cache; nothing of whose habits these were is left.
    rig.cache.wipe_all()?;
    assert!(
        PackStore::new(&rig.cache)
            .recents(PackPanel::RECENT_ROOM)?
            .is_empty()
    );
    assert!(relaunched.panel()?.recent.is_empty());
    Ok(())
}

/// BLOCKS DO NOT REACH THE PACK: a blocked member's items are in the panel like anybody's — an item
/// is a picture the family keeps — and their sticker MESSAGE is hidden, by the conversation's own
/// rule, untouched by any of this.
#[test]
fn a_blocked_members_items_are_in_the_panel_like_anybodys() -> StoreResult<()> {
    let rig = PackRig::new()?;
    rig.pack.replace(&[item_by(5, 12, 9), item(6, 13)], 13)?;
    rig.chats.set_blocked(9, true)?;
    let model = rig.build(&Wire::new()).model;

    assert_eq!(vec![5, 6], ids(&model.items()?));
    assert_eq!(vec![5, 6], ids(&model.panel()?.all));
    // And their sticker MESSAGE is hidden, by the conversation's own rule, untouched by any of this.
    let list = ChatListModel::new(rig.chats.clone(), || ME, Rc::new(english()));
    let sticker = fc_core::protocol::MessageDto {
        attachments: Some(vec![AttachmentDto {
            mime: Some(WEBP.to_string()),
            size: Some(64),
            sticker: true,
            ..AttachmentDto::new(90, "photo")
        }]),
        ..fc_core::protocol::MessageDto::new(1, 42, 9, None, "", "2026-09-13T10:00:00Z")
    };
    assert!(list.is_hidden(&sticker)?);
    assert_eq!("Hidden — blocked member", list.preview(&sticker, true));
    Ok(())
}
