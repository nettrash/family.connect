//! How a chat sticker is drawn, and the clock and the shelf of an animated one (`StickerLook.cs`;
//! docs/protocol.md, "How it is drawn").
//!
//! Three parts, as in the C#: [`StickerLook`] is the box a sticker is fitted into, [`StickerAnimation`]
//! the arithmetic of an animated one (which frame is up, when the next is due, whether animating it
//! is worth the memory), and [`StickerShelf`] the decoded stickers a view keeps and what keeping
//! them may cost. No window and no picture type in any of it, so every rule is tested where no
//! window exists.
//!
//! .NET's `Math.Round` is banker's rounding — half to even — and Rust's `round` is half away from
//! zero (port-plan "Numbers and time"), so every rounding here is `f64::round_ties_even`: a
//! 64 × 5 sticker is drawn 160 × 12, as on Windows, not 160 × 13.

use std::collections::HashMap;
use std::rc::Rc;

/// The size a chat sticker is drawn at — a drawing rule and not a wire one, written down in the
/// protocol because five clients must agree.
///
/// NO BUBBLE, AND ONE FIXED BOX. The picture alone, its transparency showing the chat behind it, in a
/// box that is the same for every sticker on this client — LARGER than an emoji and smaller than a
/// photograph — fitted whole and never cropped. Never at the picture's own pixel size: that would
/// make a 96-pixel sticker a speck and a 2000-pixel one a poster.
///
/// THE SHAPE COMES FROM METADATA, as a photo tile's does, so a row does not change height when its
/// picture lands; a sticker whose uploader could not say is given the whole box.
///
/// This is the CHAT sticker. The board's cards are `fc_core::board`'s, and nothing here is about
/// them.
pub struct StickerLook;

impl StickerLook {
    /// The box in a conversation, in logical pixels. A lone photo tile is up to 320 and the largest
    /// emoji-only message about a fifth of that: this sits between the two, where a messenger's
    /// stickers sit.
    pub const BOX: f64 = 160.0;

    /// One cell of the panel over the composer.
    pub const PANEL_CELL: f64 = 72.0;

    /// How many cells across the panel is.
    pub const PANEL_COLUMNS: i32 = 5;

    /// One cell of the pack on the Family screen.
    pub const MANAGE_CELL: f64 = 88.0;

    /// The size a sticker is drawn at inside [`BOX`](Self::BOX): the whole picture, its proportions
    /// kept, scaled UP as readily as down — the box is the size, not a ceiling.
    pub fn fit(width: Option<i32>, height: Option<i32>) -> (f64, f64) {
        Self::fit_in(width, height, Self::BOX)
    }

    /// [`fit`](Self::fit) inside a box of side `side` (the C#'s `box` argument: a panel cell, a pack
    /// cell).
    pub fn fit_in(width: Option<i32>, height: Option<i32>, side: f64) -> (f64, f64) {
        let (Some(width), Some(height)) = (width.filter(|w| *w > 0), height.filter(|h| *h > 0))
        else {
            return (side, side);
        };
        let (width, height) = (f64::from(width), f64::from(height));
        let scale = net_min(side / width, side / height);
        // Never thinner than a pixel: a 4000 × 3 strip is still something to click.
        (
            net_max(1.0, (width * scale).round_ties_even()),
            net_max(1.0, (height * scale).round_ties_even()),
        )
    }
}

/// The clock of an animated sticker: which frame is up, and whether animating it is worth the
/// memory at all.
///
/// ANIMATED WHERE THE PLATFORM CAN, FRAME ZERO WHERE IT CANNOT — AND BOTH ARE CORRECT. Whether the
/// machine gives the frames past the first is a fact about it (a missing decoder), and a sticker
/// drawn still is the same sticker. What is decided HERE is only arithmetic, so it can be tested
/// where no codec runs.
///
/// A DURATION OF ZERO IS NOT ZERO. Files in the wild say 0 or 10 ms for "as fast as you can", and
/// every browser draws those at 100 ms rather than spinning a core; so does this.
pub struct StickerAnimation;

impl StickerAnimation {
    /// The most frames one sticker is animated with; past it, frame zero.
    pub const MAX_FRAMES: i32 = 240;

    /// The most decoded pixels one animated sticker may hold, as RGBA bytes; past it, frame zero.
    pub const MAX_DECODED_BYTES: i64 = 48 * 1024 * 1024;

    /// The most decoded bytes EVERY animated sticker a view keeps may hold between them — two of the
    /// largest one allowed, so the newest always fits whatever else is on screen. What is over it is
    /// drawn still ([`StickerShelf`]).
    pub const MAX_HELD_BYTES: i64 = 2 * Self::MAX_DECODED_BYTES;

    /// How many decoded stickers a view keeps, still ones and unreadable ones included.
    pub const MAX_KEPT: usize = 64;

    /// What a too-short frame stands for, in milliseconds — the browsers' rule.
    pub const FLOOR_MS: i32 = 100;

    /// The shortest the timer waits between frames: thirty a second, the most it ever moved
    /// stickers at.
    pub const FASTEST_TICK_MS: i32 = 33;

    /// The longest it waits even when nothing on screen is due sooner, so an image that left the
    /// window is still let go — and its frames with it — within a quarter of a second.
    pub const SLOWEST_TICK_MS: i32 = 250;

    /// What `frames` frames of that size hold once decoded, as four bytes a pixel.
    ///
    /// Saturating where the C#'s `long` arithmetic wraps: measured on .NET 10,
    /// `DecodedBytes(2, int.MaxValue, int.MaxValue)` is −34 359 738 360 and `Worth` then says yes to
    /// a sticker no machine could hold. Here that is `i64::MAX`, and not worth it.
    pub fn decoded_bytes(frames: i32, pixel_width: i32, pixel_height: i32) -> i64 {
        i64::from(frames.max(0))
            .saturating_mul(i64::from(pixel_width.max(0)))
            .saturating_mul(i64::from(pixel_height.max(0)))
            .saturating_mul(4)
    }

    /// Whether to animate: more than one frame, and frames that fit the budget at the size they will
    /// be decoded. Anything else is drawn as its first frame.
    pub fn worth(frames: i32, pixel_width: i32, pixel_height: i32) -> bool {
        frames > 1
            && frames <= Self::MAX_FRAMES
            && pixel_width > 0
            && pixel_height > 0
            && Self::decoded_bytes(frames, pixel_width, pixel_height) <= Self::MAX_DECODED_BYTES
    }

    /// The pixel size frames are decoded at: fitted into the box at the screen's own scale, and never
    /// larger than the picture is — a 512-pixel sticker in a 160-point box at 150% is decoded at 240,
    /// not 512, which is a fifth of the memory for the same picture.
    pub fn decode_size(width: i32, height: i32, side: f64, raster_scale: f64) -> (i32, i32) {
        if width <= 0 || height <= 0 {
            return (1, 1);
        }
        let (w, h) = (f64::from(width), f64::from(height));
        let room = net_max(1.0, side * net_max(raster_scale, 1.0));
        let scale = net_min(1.0, net_min(room / w, room / h));
        // `as` saturates and reads NaN as 0, as .NET 9's conversions do.
        (
            ((w * scale).round_ties_even() as i32).max(1),
            ((h * scale).round_ties_even() as i32).max(1),
        )
    }

    /// How long each of `frames` frames stands. The file's own durations when it gave one per frame;
    /// the floor for every frame when it did not, because a count that disagrees with the decoder's
    /// is a file this clock cannot line up.
    pub fn clock(file_durations: &[i32], frames: i32) -> Vec<i32> {
        let frames = usize::try_from(frames).unwrap_or(0);
        let trusted = file_durations.len() == frames;
        (0..frames)
            .map(|at| {
                if trusted && file_durations[at] > 10 {
                    file_durations[at]
                } else {
                    Self::FLOOR_MS
                }
            })
            .collect()
    }

    /// Which frame is up after `elapsed_ms`, looping for as long as it is looked at — a sticker is
    /// not a film with an end.
    pub fn frame_at(clock: &[i32], elapsed_ms: i64) -> usize {
        if clock.is_empty() {
            return 0;
        }
        let mut within = elapsed_ms.max(0) % Self::total(clock);
        for (at, duration) in clock.iter().enumerate() {
            within -= i64::from((*duration).max(1));
            if within < 0 {
                return at;
            }
        }
        clock.len() - 1
    }

    /// How long after `elapsed_ms` the frame that is up gives way to the next — what the one timer
    /// that moves every sticker waits for, instead of waking thirty times a second for stickers whose
    /// frames stand a tenth of a second or more. Never less than a millisecond; `i64::MAX` for no
    /// frames at all.
    pub fn until_next(clock: &[i32], elapsed_ms: i64) -> i64 {
        if clock.is_empty() {
            return i64::MAX;
        }
        let mut within = elapsed_ms.max(0) % Self::total(clock);
        for duration in clock {
            within -= i64::from((*duration).max(1));
            if within < 0 {
                return -within;
            }
        }
        1
    }

    /// The timer's next wait, from the soonest any sticker on screen is due: within the fastest and
    /// slowest ticks.
    pub fn tick_after(soonest_ms: i64) -> i32 {
        soonest_ms.clamp(
            i64::from(Self::FASTEST_TICK_MS),
            i64::from(Self::SLOWEST_TICK_MS),
        ) as i32
    }

    /// The loop's length: every frame at least a millisecond, so a clock of zeros still ends.
    fn total(clock: &[i32]) -> i64 {
        clock
            .iter()
            .map(|duration| i64::from((*duration).max(1)))
            .sum()
    }
}

/// One sticker the shelf holds.
struct Kept<T: ?Sized> {
    /// `None`: nothing on this machine decodes it.
    picture: Option<Rc<T>>,
    cost: i64,
    /// When it was last drawn, on the shelf's own turn count.
    drawn: u64,
    started: Option<i64>,
}

/// The decoded stickers one view keeps, so that a conversation redrawn on every change does not
/// decode them again — and the ceiling on what keeping them may cost.
///
/// THE BUDGET IS FOR THE SHELF, NOT PER STICKER. [`StickerAnimation::MAX_DECODED_BYTES`] says what
/// ONE animated sticker may hold; a family that sends them the way messengers' users do puts forty in
/// a chat, and forty times that is gigabytes. So every animated picture is put here with what its
/// frames cost, and when the sum passes the budget the LEAST RECENTLY DRAWN give it back: one no
/// image is showing is let go whole, and one that is on screen is made a STILL — frame zero, which is
/// a correct drawing of a sticker. The newest is the one that keeps moving, because the newest is the
/// one at the bottom of the chat, where the reader is.
///
/// A COST IS WHAT CAN BE GIVEN BACK. A still picture and a picture made still are put at zero: they
/// are one frame each, and they leave by the count alone.
///
/// "NOTHING HERE DECODES IT" IS AN ANSWER, AND IT IS KEPT. A sticker no decoder on this machine reads
/// is remembered as that, so a rebuild does not read the file and fail the decode again for every
/// sticker in the chat, on every message that arrives.
///
/// AND IT KEEPS WHEN EACH ONE STARTED, so the images of a rebuilt conversation join the animation
/// where it was rather than sending every sticker on screen back to frame zero whenever anybody in
/// the chat sends or reacts.
///
/// A PICTURE IS AN `Rc<T>`, AND "THE SAME PICTURE" IS THE SAME `Rc`. The C# compares pictures by
/// reference (`ReferenceEquals`): a sticker decoded again into a NEW picture lets the old one go,
/// while putting back the very picture it holds does not. `Rc::ptr_eq` is that test, and an `Rc` is
/// what a GTK shell holds a decoded animation in anyway. The callbacks get the `Rc` itself, so the
/// shell may compare it with what its images draw (`showing`) or drop its textures (`let_go`).
///
/// What a picture is, whether one is on screen and how one is made still are the view's to say,
/// through the three callbacks: `showing` (an image in the window draws that picture right now),
/// `still` (make it a still one — frame zero kept, the rest given back) and `let_go` (it is leaving
/// the shelf and nothing shows it: give back what it holds). Not thread-safe — the main context's.
pub struct StickerShelf<T: ?Sized> {
    budget: i64,
    most: usize,
    showing: Option<Box<dyn Fn(&Rc<T>) -> bool>>,
    still: Option<Box<dyn Fn(&Rc<T>)>>,
    let_go: Option<Box<dyn Fn(&Rc<T>)>>,
    kept: HashMap<i64, Kept<T>>,
    turn: u64,
    held: i64,
}

impl<T: ?Sized> StickerShelf<T> {
    /// A shelf holding at most `budget` bytes that can be given back, and at most `most` pictures —
    /// past it the least recently drawn that no image shows are let go. With no callbacks nothing is
    /// on screen and nothing needs giving back; add them with the `with_` methods.
    pub fn new(budget: i64, most: usize) -> Self {
        StickerShelf {
            budget,
            most,
            showing: None,
            still: None,
            let_go: None,
            kept: HashMap::new(),
            turn: 0,
            held: 0,
        }
    }

    /// Whether an image in the window is drawing that picture right now.
    pub fn with_showing(mut self, showing: impl Fn(&Rc<T>) -> bool + 'static) -> Self {
        self.showing = Some(Box::new(showing));
        self
    }

    /// Make that picture a still one — frame zero kept, the rest given back.
    pub fn with_still(mut self, still: impl Fn(&Rc<T>) + 'static) -> Self {
        self.still = Some(Box::new(still));
        self
    }

    /// That picture is leaving the shelf and nothing shows it: give back what it holds.
    pub fn with_let_go(mut self, let_go: impl Fn(&Rc<T>) + 'static) -> Self {
        self.let_go = Some(Box::new(let_go));
        self
    }

    /// What the pictures here hold between them, in bytes that could be given back.
    pub fn held(&self) -> i64 {
        self.held
    }

    pub fn count(&self) -> usize {
        self.kept.len()
    }

    /// Whether this sticker has been decoded here: `Some(picture)`, where the picture is `None` when
    /// the answer was that nothing on this machine reads it; `None` when it has not been tried.
    /// Asking is drawing: it counts as use.
    pub fn try_get(&mut self, id: i64) -> Option<Option<Rc<T>>> {
        let entry = self.kept.get_mut(&id)?;
        self.turn += 1;
        entry.drawn = self.turn;
        Some(entry.picture.clone())
    }

    /// Whether this sticker has an answer here — decoded, or known to decode to nothing — WITHOUT
    /// counting as a drawing: a redraw asking whether a row's sticker still needs trying again must
    /// not make that sticker the last one to go.
    pub fn holds(&self, id: i64) -> bool {
        self.kept.contains_key(&id)
    }

    /// Keep what a decode answered: the picture and what its frames cost, or `None` for "nothing here
    /// reads it". Older pictures make room for it, never the other way round.
    pub fn put(&mut self, id: i64, picture: Option<Rc<T>>, cost: i64) {
        let replaced = self.kept.remove(&id);
        if let Some(replaced) = &replaced {
            self.held -= replaced.cost;
            if let Some(old) = &replaced.picture {
                let same = picture.as_ref().is_some_and(|new| Rc::ptr_eq(old, new));
                if !same && !self.is_showing(old) {
                    self.give_back(old);
                }
            }
        }
        self.turn += 1;
        let cost = if picture.is_none() { 0 } else { cost.max(0) };
        self.kept.insert(
            id,
            Kept {
                picture,
                cost,
                drawn: self.turn,
                // A sticker decoded again is the same sticker: its clock carries on.
                started: replaced.and_then(|replaced| replaced.started),
            },
        );
        self.held += cost;
        // Least recently drawn first, and never the one just put: the newest is the one being
        // looked at.
        let mut others: Vec<(i64, u64)> = self
            .kept
            .iter()
            .filter(|(other, _)| **other != id)
            .map(|(other, entry)| (*other, entry.drawn))
            .collect();
        others.sort_by_key(|(_, drawn)| *drawn);
        for (other, _) in others {
            let over_count = self.kept.len() > self.most;
            if self.held <= self.budget && !over_count {
                break;
            }
            let Some(entry) = self.kept.get(&other) else {
                continue;
            };
            let (cost, picture) = (entry.cost, entry.picture.clone());
            // Only a picture with something to give back answers for the bytes.
            let over_bytes = self.held > self.budget && cost > 0;
            if let Some(shown) = picture.as_ref().filter(|shown| self.is_showing(shown)) {
                // On screen: it cannot leave, but it can stop moving — and only the bytes ask that
                // of it.
                if over_bytes {
                    if let Some(still) = &self.still {
                        still(shown);
                    }
                    self.held -= cost;
                    if let Some(entry) = self.kept.get_mut(&other) {
                        entry.cost = 0;
                    }
                }
                continue;
            }
            if !over_bytes && !over_count {
                continue;
            }
            self.kept.remove(&other);
            self.held -= cost;
            if let Some(gone) = &picture {
                self.give_back(gone);
            }
        }
    }

    /// When this sticker's animation began, on the caller's clock: `now` the first time it is asked,
    /// and that same moment for every image that draws it afterwards. A sticker the shelf does not
    /// hold — the viewer's — starts when it is shown. Asking is not drawing.
    pub fn started(&mut self, id: i64, now: i64) -> i64 {
        match self.kept.get_mut(&id) {
            Some(entry) => *entry.started.get_or_insert(now),
            None => now,
        }
    }

    /// Forget one picture — a pack item that was removed: its bytes no longer say anything. Let go
    /// unless an image still draws it, in which case it goes when that image drops its `Rc`.
    /// Answers whether it was here.
    pub fn remove(&mut self, id: i64) -> bool {
        let Some(entry) = self.kept.remove(&id) else {
            return false;
        };
        self.held -= entry.cost;
        if let Some(gone) = &entry.picture
            && !self.is_showing(gone)
        {
            self.give_back(gone);
        }
        true
    }

    /// The view is going: everything is let go, shown or not — least recently drawn first (the C#'s
    /// order is its dictionary's, which no rule depends on).
    pub fn clear(&mut self) {
        let mut all: Vec<Kept<T>> = self.kept.drain().map(|(_, entry)| entry).collect();
        all.sort_by_key(|entry| entry.drawn);
        self.held = 0;
        for entry in all {
            if let Some(picture) = &entry.picture {
                self.give_back(picture);
            }
        }
    }

    fn is_showing(&self, picture: &Rc<T>) -> bool {
        self.showing
            .as_ref()
            .is_some_and(|showing| showing(picture))
    }

    fn give_back(&self, picture: &Rc<T>) {
        if let Some(let_go) = &self.let_go {
            let_go(picture);
        }
    }
}

/// `Math.Max` on doubles: NaN if either is NaN (Rust's `f64::max` would answer the other).
fn net_max(one: f64, other: f64) -> f64 {
    if one.is_nan() || other.is_nan() {
        f64::NAN
    } else {
        one.max(other)
    }
}

/// `Math.Min` on doubles: NaN if either is NaN.
fn net_min(one: f64, other: f64) -> f64 {
    if one.is_nan() || other.is_nan() {
        f64::NAN
    } else {
        one.min(other)
    }
}
