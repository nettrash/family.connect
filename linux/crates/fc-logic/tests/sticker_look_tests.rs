//! `StickerLookTests`: how a chat sticker is drawn (docs/protocol.md, "How it is drawn") — one fixed
//! box, fitted whole, larger than an emoji and smaller than a photograph — and the arithmetic of an
//! animated one, and the shelf that keeps them decoded.
//!
//! Elsewhere: the tile's source and the saved file name (`attachment_files_sticker_look.rs`), the
//! viewer's title (`media_album_sticker_look.rs`), "a sticker behaves like any message"
//! (`bubble_rules_sticker_look.rs`, `conversation_sticker_look.rs`) and the chat-list row
//! ("Sticker", `chat_list_media_rows.rs`).
//!
//! After the C#'s tests come the port's own: banker's rounding where .NET's `Math.Round` decides a
//! size, the NaN rules of .NET's `Math.Max`/`Math.Min`, and the one deliberate difference — a
//! decoded size too large to count is not worth animating, where the C#'s arithmetic wrapped round
//! (all measured on .NET 10).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use fc_logic::sticker_look::{StickerAnimation, StickerLook, StickerShelf};

/// NEVER AT THE PICTURE'S OWN PIXEL SIZE: a 96-pixel sticker would be a speck and a 2000-pixel one a
/// poster. Both fill the same box.
#[test]
fn every_sticker_is_fitted_whole_into_the_same_box() {
    let cases: [(i32, i32, f64, f64); 6] = [
        (512, 512, 160.0, 160.0),
        (96, 96, 160.0, 160.0),
        (2000, 2000, 160.0, 160.0),
        (512, 256, 160.0, 80.0),
        (256, 512, 80.0, 160.0),
        (300, 100, 160.0, 53.0),
    ];
    for (width, height, drawn_width, drawn_height) in cases {
        assert_eq!(
            (drawn_width, drawn_height),
            StickerLook::fit(Some(width), Some(height)),
            "{width}×{height}"
        );
    }
}

/// The shape comes from metadata, and a sticker whose uploader could not say gets the whole box.
#[test]
fn a_sticker_of_unknown_shape_gets_the_whole_box() {
    assert_eq!((160.0, 160.0), StickerLook::fit(None, None));
    assert_eq!((160.0, 160.0), StickerLook::fit(Some(512), None));
    assert_eq!((160.0, 160.0), StickerLook::fit(Some(0), Some(512)));
    assert_eq!(
        (72.0, 72.0),
        StickerLook::fit_in(None, None, StickerLook::PANEL_CELL)
    );
    // Never thinner than a pixel.
    assert_eq!((160.0, 1.0), StickerLook::fit(Some(4000), Some(3)));
}

/// The box is 160 effective pixels — the same number on every client, in its own unit.
#[test]
fn the_box_is_one_hundred_and_sixty() {
    assert_eq!(160.0, StickerLook::BOX);
    assert_eq!((160.0, 160.0), StickerLook::fit(Some(512), Some(512)));
    assert_eq!((160.0, 160.0), StickerLook::fit(None, None));
}

/// Larger than an emoji and smaller than a photograph, on this client's own scales.
#[test]
fn the_box_sits_between_an_emoji_and_a_photo() {
    let largest_emoji = fc_text::emoji::display_font_size_for_body("🎉", 14.0).unwrap_or(0.0);

    assert!(largest_emoji > 0.0);
    assert!(StickerLook::BOX > largest_emoji);
    const { assert!(StickerLook::BOX < fc_text::media::TILE_MAX) };
}

// ---- animation ----------------------------------------------------------------------------------

#[test]
fn only_a_picture_with_frames_that_fit_the_budget_is_animated() {
    assert!(StickerAnimation::worth(30, 240, 240));
    // One frame is a still picture.
    assert!(!StickerAnimation::worth(1, 240, 240));
    assert!(!StickerAnimation::worth(0, 240, 240));
    // Too many frames, or too many pixels of them: drawn as frame zero, which is a correct sticker.
    assert!(!StickerAnimation::worth(
        StickerAnimation::MAX_FRAMES + 1,
        16,
        16
    ));
    assert!(!StickerAnimation::worth(200, 512, 512));
    assert!(!StickerAnimation::worth(30, 0, 240));
}

#[test]
fn frames_are_decoded_at_the_size_they_are_drawn_and_no_larger() {
    let cases: [(i32, i32, f64, f64, i32, i32); 5] = [
        (512, 512, 160.0, 1.0, 160, 160),
        (512, 512, 160.0, 1.5, 240, 240),
        (512, 256, 160.0, 2.0, 320, 160),
        // Never decoded larger than it is.
        (96, 96, 160.0, 2.0, 96, 96),
        // A scale nobody could read is taken as 1.
        (512, 512, 160.0, 0.0, 160, 160),
    ];
    for (width, height, side, scale, decoded_width, decoded_height) in cases {
        assert_eq!(
            (decoded_width, decoded_height),
            StickerAnimation::decode_size(width, height, side, scale),
            "{width}×{height} in {side} at {scale}"
        );
    }
}

/// A DURATION OF ZERO IS NOT ZERO: files say 0 or 10 ms for "as fast as you can", and every browser
/// draws those at 100 ms rather than spinning a core.
#[test]
fn a_too_short_frame_stands_for_a_tenth_of_a_second() {
    assert_eq!(
        vec![40, 100, 100, 11, 1000],
        StickerAnimation::clock(&[40, 0, 10, 11, 1000], 5)
    );
}

/// A file whose frame count disagrees with the decoder's cannot be lined up, so every frame gets the
/// floor.
#[test]
fn durations_that_do_not_match_the_frames_are_not_trusted() {
    assert_eq!(vec![100, 100, 100], StickerAnimation::clock(&[40, 40], 3));
    assert_eq!(vec![100, 100], StickerAnimation::clock(&[], 2));
    assert!(StickerAnimation::clock(&[40], 0).is_empty());
}

#[test]
fn the_frame_due_follows_the_clock_and_loops_for_ever() {
    let clock = [40, 60, 100];

    assert_eq!(0, StickerAnimation::frame_at(&clock, 0));
    assert_eq!(0, StickerAnimation::frame_at(&clock, 39));
    assert_eq!(1, StickerAnimation::frame_at(&clock, 40));
    assert_eq!(1, StickerAnimation::frame_at(&clock, 99));
    assert_eq!(2, StickerAnimation::frame_at(&clock, 100));
    assert_eq!(2, StickerAnimation::frame_at(&clock, 199));
    // A sticker is not a film with an end.
    assert_eq!(0, StickerAnimation::frame_at(&clock, 200));
    assert_eq!(1, StickerAnimation::frame_at(&clock, 200 * 1000 + 45));
    assert_eq!(0, StickerAnimation::frame_at(&clock, -5));
    assert_eq!(0, StickerAnimation::frame_at(&[], 500));
    // A clock somebody built with a zero in it still ends.
    assert_eq!(1, StickerAnimation::frame_at(&[0, 0], 1));
}

/// The timer waits for the next frame that is due — not a fixed thirtieth of a second — and never
/// past the end of the frame that is up, so no frame is drawn later than it was.
#[test]
fn the_timer_waits_for_the_next_frame_that_is_due() {
    let clock = [100, 200, 50];
    assert_eq!(100, StickerAnimation::until_next(&clock, 0));
    assert_eq!(40, StickerAnimation::until_next(&clock, 60));
    assert_eq!(200, StickerAnimation::until_next(&clock, 100));
    assert_eq!(1, StickerAnimation::until_next(&clock, 299));
    assert_eq!(50, StickerAnimation::until_next(&clock, 300));
    // It loops, as the frames do.
    assert_eq!(100, StickerAnimation::until_next(&clock, 350));
    assert_eq!(
        StickerAnimation::frame_at(&clock, 350 + 99),
        StickerAnimation::frame_at(&clock, 350)
    );
    assert_ne!(
        StickerAnimation::frame_at(&clock, 350 + 100),
        StickerAnimation::frame_at(&clock, 350)
    );
    assert_eq!(i64::MAX, StickerAnimation::until_next(&[], 10));
    assert_eq!(1, StickerAnimation::until_next(&[0, 0], 0));

    // Never faster than the thirty a second it always moved at, and never so slow an image that left
    // is kept long.
    assert_eq!(
        StickerAnimation::FASTEST_TICK_MS,
        StickerAnimation::tick_after(1)
    );
    assert_eq!(100, StickerAnimation::tick_after(100));
    assert_eq!(
        StickerAnimation::SLOWEST_TICK_MS,
        StickerAnimation::tick_after(i64::MAX)
    );
}

// ---- the shelf: what a view keeps decoded, and what that may cost ------------------------------
//
// No window in any of this: a "picture" here is a name, and whether it is on screen is a set of
// names — as the C#'s `HashSet<string>` compares them, by value.

#[derive(Default)]
struct Wall {
    on_screen: Rc<RefCell<HashSet<String>>>,
    made_still: Rc<RefCell<Vec<String>>>,
    let_go: Rc<RefCell<Vec<String>>>,
}

impl Wall {
    fn shelf(&self, budget: i64, most: usize) -> StickerShelf<String> {
        let on_screen = Rc::clone(&self.on_screen);
        let made_still = Rc::clone(&self.made_still);
        let let_go = Rc::clone(&self.let_go);
        StickerShelf::new(budget, most)
            .with_showing(move |picture: &Rc<String>| on_screen.borrow().contains(&**picture))
            .with_still(move |picture: &Rc<String>| {
                made_still.borrow_mut().push((**picture).clone())
            })
            .with_let_go(move |picture: &Rc<String>| let_go.borrow_mut().push((**picture).clone()))
    }

    fn show(&self, name: &str) {
        self.on_screen.borrow_mut().insert(name.to_string());
    }

    fn made_still(&self) -> Vec<String> {
        self.made_still.borrow().clone()
    }

    fn let_go(&self) -> Vec<String> {
        self.let_go.borrow().clone()
    }
}

fn picture(name: &str) -> Option<Rc<String>> {
    Some(Rc::new(name.to_string()))
}

/// What the shelf answers for a sticker, as the name drawn: `Some(None)` for "nothing decodes it".
fn named(answer: Option<Option<Rc<String>>>) -> Option<Option<String>> {
    answer.map(|picture| picture.map(|name| (*name).clone()))
}

/// THE BUDGET IS FOR THE SHELF, NOT PER STICKER. Forty animated stickers of a legal size each are
/// gigabytes if every one is kept; what is kept never passes the budget, however many go by.
#[test]
fn the_shelf_never_holds_more_than_its_budget_however_many_stickers_go_by() {
    let wall = Wall::default();
    let mut shelf = wall.shelf(StickerAnimation::MAX_HELD_BYTES, 64);

    for id in 1..=40 {
        shelf.put(
            id,
            picture(&format!("s{id}")),
            StickerAnimation::MAX_DECODED_BYTES / 2,
        );
        assert!(shelf.held() <= StickerAnimation::MAX_HELD_BYTES);
    }

    // Four halves fit; the thirty-six drawn longest ago were let go — given back, not left to
    // whoever drops the last reference.
    assert_eq!(4, shelf.count());
    assert_eq!(
        (1..=36).map(|id| format!("s{id}")).collect::<Vec<_>>(),
        wall.let_go()
    );
    assert!(wall.made_still().is_empty());
    assert_eq!(Some(Some("s40".to_string())), named(shelf.try_get(40)));
    assert!(shelf.try_get(1).is_none());
}

/// Asking whether a sticker has an answer — what a redraw asks of every sticker row it might keep —
/// is not drawing it: it must not save that sticker from being the next to go. "Nothing decodes it"
/// is an answer too.
#[test]
fn asking_whether_a_sticker_is_held_is_not_drawing_it() {
    let wall = Wall::default();
    let mut shelf = wall.shelf(100, 2);
    shelf.put(1, picture("a"), 10);
    shelf.put(2, None, 0);
    assert!(shelf.holds(1));
    assert!(shelf.holds(2));
    assert!(!shelf.holds(3));

    // Asked about, not drawn: the oldest DRAWN is still 1, and it is the one that goes.
    assert!(shelf.holds(1));
    shelf.put(3, picture("c"), 10);
    assert!(!shelf.holds(1));
    assert_eq!(vec!["a"], wall.let_go());
    assert!(shelf.holds(2));
    assert!(shelf.holds(3));
}

/// The budget is two of the largest sticker allowed, so the newest always fits.
#[test]
fn the_budget_has_room_for_the_largest_sticker_whatever_else_is_kept() {
    const { assert!(StickerAnimation::MAX_HELD_BYTES >= 2 * StickerAnimation::MAX_DECODED_BYTES) };
    assert_eq!(
        StickerAnimation::MAX_DECODED_BYTES,
        StickerAnimation::decoded_bytes(48, 512, 512)
    );
    assert_eq!(0, StickerAnimation::decoded_bytes(-1, 512, 512));
}

/// A sticker ON SCREEN cannot be let go — an image is drawing its frames — so it is made a STILL
/// instead, oldest first, and stays on the shelf as one: a rebuild finds it there and does not
/// decode it again.
#[test]
fn a_sticker_on_screen_is_made_still_rather_than_let_go() {
    let wall = Wall::default();
    let mut shelf = wall.shelf(100, 64);
    shelf.put(1, picture("old"), 60);
    shelf.put(2, picture("middle"), 30);
    wall.show("old");
    wall.show("middle");

    shelf.put(3, picture("new"), 60);

    // The oldest gave its frames back and that was enough: the middle one keeps moving.
    assert_eq!(vec!["old"], wall.made_still());
    assert!(wall.let_go().is_empty());
    assert_eq!(90, shelf.held());
    assert_eq!(Some(Some("old".to_string())), named(shelf.try_get(1)));

    // Made still once, it has nothing more to give: the next squeeze passes it by.
    wall.show("new");
    shelf.put(4, picture("newer"), 60);
    assert_eq!(vec!["old", "middle", "new"], wall.made_still());
    assert_eq!(60, shelf.held());
    assert_eq!(4, shelf.count());
}

/// What nobody is looking at goes before anything on screen stops moving — in the order it was last
/// drawn.
#[test]
fn what_is_off_screen_is_let_go_in_the_order_it_was_last_drawn() {
    let wall = Wall::default();
    let mut shelf = wall.shelf(100, 64);
    shelf.put(1, picture("a"), 40);
    shelf.put(2, picture("b"), 40);
    // Drawn again: a rebuild asked for it.
    assert!(shelf.try_get(1).is_some());

    shelf.put(3, picture("c"), 40);

    assert_eq!(vec!["b"], wall.let_go());
    assert!(shelf.try_get(1).is_some());
    assert!(shelf.try_get(2).is_none());
}

/// A still picture costs nothing that can be given back, so bytes never push it out; the COUNT does,
/// and only for what no image is showing.
#[test]
fn still_pictures_leave_by_the_count_alone() {
    let wall = Wall::default();
    let mut shelf = wall.shelf(100, 3);
    shelf.put(1, picture("still-1"), 0);
    shelf.put(2, picture("still-2"), 0);
    shelf.put(3, picture("moving"), 90);
    wall.show("moving");

    // Over the bytes: the stills are passed by, and the moving one on screen is what gives.
    shelf.put(4, picture("moving-2"), 90);
    assert_eq!(vec!["moving"], wall.made_still());
    // …and over the count: the oldest still that nothing shows is let go.
    assert_eq!(vec!["still-1"], wall.let_go());
    assert_eq!(3, shelf.count());
    assert_eq!(90, shelf.held());
}

/// "NOTHING HERE DECODES IT" IS AN ANSWER, AND IT IS KEPT: on a machine without the codec a chat of
/// twenty stickers would otherwise read twenty files and fail twenty decodes on every message that
/// arrives.
#[test]
fn a_sticker_nothing_here_decodes_is_remembered_as_that() {
    let wall = Wall::default();
    let mut shelf = wall.shelf(100, 64);

    assert!(shelf.try_get(7).is_none());
    shelf.put(7, None, 50);

    assert_eq!(Some(None), named(shelf.try_get(7)));
    // An answer holds nothing.
    assert_eq!(0, shelf.held());
    shelf.clear();
    assert!(wall.let_go().is_empty());
    assert!(shelf.try_get(7).is_none());
}

/// THE CLOCK IS THE STICKER'S, NOT THE IMAGE'S. The conversation is rebuilt on every change, and each
/// rebuild makes new images: they join the animation where it was, rather than every sticker on
/// screen jumping back to frame zero whenever anybody sends or reacts.
#[test]
fn a_rebuilt_image_joins_the_animation_where_it_was() {
    let wall = Wall::default();
    let mut shelf = wall.shelf(100, 64);
    let clock = [100, 100, 100];
    shelf.put(5, picture("waving"), 10);

    let first = shelf.started(5, 1_000);
    let rebuilt = shelf.started(5, 1_150);

    assert_eq!(1_000, first);
    assert_eq!(1_000, rebuilt);
    assert_eq!(1, StickerAnimation::frame_at(&clock, 1_150 - rebuilt));
    // Decoded again (the screen's scale changed, say), it is still the same sticker.
    shelf.put(5, picture("waving, sharper"), 10);
    assert_eq!(1_000, shelf.started(5, 1_400));
    // One the shelf does not hold — the viewer's — starts when it is shown.
    assert_eq!(2_000, shelf.started(99, 2_000));
}

/// The view is going: every picture is let go, on screen or not, and nothing is held.
#[test]
fn clearing_the_shelf_lets_everything_go() {
    let wall = Wall::default();
    let mut shelf = wall.shelf(100, 64);
    shelf.put(1, picture("a"), 40);
    shelf.put(2, picture("b"), 40);
    wall.show("a");

    shelf.clear();

    let mut let_go = wall.let_go();
    let_go.sort();
    assert_eq!(vec!["a", "b"], let_go);
    assert_eq!(0, shelf.held());
    assert_eq!(0, shelf.count());
}

// ---- added by the port ------------------------------------------------------------------------------

/// .NET's `Math.Round` rounds a half to EVEN (measured: `Fit(64, 5)` is 160 × 12 and
/// `DecodeSize(64, 5, 32, 1)` is 32 × 2 on .NET 10); Rust's `round` would say 13 and 3.
#[test]
fn a_half_rounds_to_even_as_on_windows() {
    assert_eq!((160.0, 12.0), StickerLook::fit(Some(64), Some(5)));
    assert_eq!((12.0, 160.0), StickerLook::fit(Some(5), Some(64)));
    assert_eq!((32, 2), StickerAnimation::decode_size(64, 5, 32.0, 1.0));
    assert_eq!((2, 2), StickerAnimation::decode_size(3, 3, 0.5, 3.0));
}

/// .NET's `Math.Max` and `Math.Min` answer NaN when either side is NaN, and its conversion of NaN to
/// an integer is 0 — so a scale that is not a number decodes at a single pixel, as on Windows
/// (measured), rather than quietly at full size.
#[test]
fn a_scale_that_is_not_a_number_decodes_as_on_windows() {
    assert_eq!(
        (1, 1),
        StickerAnimation::decode_size(512, 512, 160.0, f64::NAN)
    );
    assert_eq!(
        (512, 256),
        StickerAnimation::decode_size(512, 256, 160.0, f64::INFINITY)
    );
    assert_eq!(
        (160, 160),
        StickerAnimation::decode_size(512, 512, 160.0, -1.0)
    );
    assert_eq!(
        (1, 1),
        StickerAnimation::decode_size(512, 512, f64::NAN, 1.0)
    );
    let (width, height) = StickerLook::fit_in(Some(10), Some(10), f64::NAN);
    assert!(width.is_nan() && height.is_nan());
    assert_eq!((1.0, 1.0), StickerLook::fit_in(Some(10), Some(20), 0.0));
    assert_eq!(
        vec![11, 100, 100],
        StickerAnimation::clock(&[11, 10, -5], 3)
    );
    assert_eq!(1, StickerAnimation::until_next(&[-5, 0], 1));
    assert_eq!(1, StickerAnimation::frame_at(&[-5, 0], 1));
    assert_eq!(
        StickerAnimation::FASTEST_TICK_MS,
        StickerAnimation::tick_after(-5)
    );
}

/// DELIBERATELY NOT WINDOWS': a size too large to count is not worth animating. The C# multiplies in
/// `long` and wraps — measured on .NET 10, `DecodedBytes(2, int.MaxValue, int.MaxValue)` is
/// −34 359 738 360 and `DecodedBytes(240, 1 << 30, 1 << 30)` is 0, and `Worth` says yes to both.
#[test]
fn a_size_too_large_to_count_is_not_worth_animating() {
    assert_eq!(
        i64::MAX,
        StickerAnimation::decoded_bytes(2, i32::MAX, i32::MAX)
    );
    assert!(!StickerAnimation::worth(2, i32::MAX, i32::MAX));
    assert!(!StickerAnimation::worth(240, 1 << 30, 1 << 30));
    assert!(!StickerAnimation::worth(16, 1 << 30, 1 << 29));
}

/// The same picture put back is not let go; a new picture for the same sticker lets the old one go
/// unless an image still draws it — the C#'s `ReferenceEquals`, which is `Rc::ptr_eq` here.
#[test]
fn a_picture_put_back_is_kept_and_a_new_one_replaces_it() {
    let wall = Wall::default();
    let mut shelf = wall.shelf(1000, 64);
    let first = picture("waving");
    shelf.put(5, first.clone(), 10);
    shelf.put(5, first, 10);
    assert!(wall.let_go().is_empty());
    // Equal words, another picture: the old one goes.
    shelf.put(5, picture("waving"), 10);
    assert_eq!(vec!["waving"], wall.let_go());
    // Shown, it stays with its image.
    wall.show("waving");
    shelf.put(5, picture("waving, sharper"), 10);
    assert_eq!(vec!["waving"], wall.let_go());
    assert_eq!(10, shelf.held());
    assert_eq!(1, shelf.count());
}
