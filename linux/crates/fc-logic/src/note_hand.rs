//! One sticker in hand — the web client's `Hand` (`NoteHand.cs`). A drag moves the sticker at once
//! and reports the fraction on RELEASE, read back from where the sticker is DRAWN: one intent is
//! one write, not sixty a second.
//!
//! HELD WHERE IT WAS DROPPED UNTIL THE MOVE IS ANSWERED, or it would jump back for the round trip.
//!
//! ONE MOVE OF A NOTE AT A TIME, THE LATEST WAITING. Moves sent side by side commit in whatever
//! order the server locks them, and the note could end at the second of three places on every
//! device.
//!
//! PICKED UP FROM WHERE IT IS DRAWN — picked up again before its last move landed, that is where it
//! was dropped, never an offset the pointer would first have to travel back.
//!
//! Every position is in wall pixels and every size a `(width, height)`; the arithmetic is
//! `fc_text::board`'s (`clamp_corner`, `origin`, `fraction_of`).

use fc_text::board::{clamp_corner, fraction_of, origin};

/// How far a pointer may wander before a press is a drag rather than a click.
pub const CLICK_SLOP: f64 = 4.0;

/// The arrow keys' "pointer".
pub const KEYBOARD: i64 = -1;

/// What letting go of a note meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandResult {
    /// Nothing of this hand's was in progress.
    None,
    /// A press that did not travel: open the note (or reveal it).
    Click,
    /// A move: `target` is the fraction of the wall it was dropped at.
    Drop,
}

/// What letting go asks for: the result, where it was dropped, and whether to send that move now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Letting {
    pub result: HandResult,
    pub target: Option<(f64, f64)>,
    pub send_now: bool,
}

impl Letting {
    /// `new Letting(result)`: no target, nothing to send.
    pub fn new(result: HandResult) -> Self {
        Letting {
            result,
            target: None,
            send_now: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    pointer: i64,
    down: (f64, f64),
    from: (f64, f64),
    delta: (f64, f64),
    moved: bool,
}

/// One sticker in hand.
#[derive(Debug, Default)]
pub struct NoteHand {
    drag: Option<Drag>,
    held: Option<(i64, (f64, f64))>,
    queued: Option<(i64, (f64, f64))>,
    drops: i64,
    sending_drop: i64,
    sending: bool,
}

impl NoteHand {
    pub fn new() -> Self {
        NoteHand::default()
    }

    /// A move of this note is on its way.
    pub fn sending(&self) -> bool {
        self.sending
    }

    /// In hand and travelling — lifted above everything on the wall.
    pub fn dragging(&self) -> bool {
        self.drag.is_some_and(|drag| drag.moved)
    }

    /// Put down, and not yet answered.
    pub fn held(&self) -> bool {
        self.held.is_some()
    }

    /// Where the card is drawn right now, in wall pixels: in hand, where the drag has it; dropped,
    /// where it was dropped; otherwise where the server has it.
    pub fn corner(&self, fraction: (f64, f64), card: (f64, f64), wall: (f64, f64)) -> (f64, f64) {
        match (self.drag, self.held) {
            (Some(moving), _) if moving.moved => clamp_corner(
                moving.from.0 + moving.delta.0,
                moving.from.1 + moving.delta.1,
                card,
                wall,
            ),
            (_, Some((_, target))) => origin(target, card, wall),
            _ => origin(fraction, card, wall),
        }
    }

    pub fn down(
        &mut self,
        pointer: i64,
        at: (f64, f64),
        fraction: (f64, f64),
        card: (f64, f64),
        wall: (f64, f64),
    ) {
        self.drag = Some(Drag {
            pointer,
            down: at,
            from: self.corner(fraction, card, wall),
            delta: (0.0, 0.0),
            moved: false,
        });
    }

    /// The pointer travelled. Answers whether the card is now being dragged, and so has to be drawn
    /// again.
    pub fn move_to(&mut self, pointer: i64, at: (f64, f64)) -> bool {
        let Some(current) = self.drag.filter(|drag| drag.pointer == pointer) else {
            return false;
        };
        let delta = (at.0 - current.down.0, at.1 - current.down.1);
        let moved = current.moved || (delta.0 * delta.0 + delta.1 * delta.1).sqrt() > CLICK_SLOP;
        self.drag = Some(Drag {
            moved,
            delta: if moved { delta } else { current.delta },
            ..current
        });
        moved
    }

    pub fn up(
        &mut self,
        pointer: i64,
        fraction: (f64, f64),
        card: (f64, f64),
        wall: (f64, f64),
    ) -> Letting {
        let Some(current) = self.drag.filter(|drag| drag.pointer == pointer) else {
            return Letting::new(HandResult::None);
        };
        let drawn = self.corner(fraction, card, wall);
        self.drag = None;
        if current.moved {
            self.commit(drawn, wall)
        } else {
            Letting::new(HandResult::Click)
        }
    }

    /// A drag the system took away — cancelled, or its capture lost — is put back where it was,
    /// not dropped.
    pub fn cancel(&mut self, pointer: i64) {
        if self.drag.is_some_and(|drag| drag.pointer == pointer) {
            self.drag = None;
        }
    }

    /// An arrow key: a step of the wall from where it is drawn, held to where it CAN be drawn.
    pub fn nudge(
        &mut self,
        dx: f64,
        dy: f64,
        fraction: (f64, f64),
        card: (f64, f64),
        wall: (f64, f64),
    ) {
        let now = self.corner(fraction, card, wall);
        self.drag = Some(Drag {
            pointer: KEYBOARD,
            down: (0.0, 0.0),
            from: clamp_corner(now.0 + dx, now.1 + dy, card, wall),
            delta: (0.0, 0.0),
            moved: true,
        });
    }

    /// The arrow let go of, or the focus gone mid-move: put down where it is drawn.
    pub fn put_down(
        &mut self,
        fraction: (f64, f64),
        card: (f64, f64),
        wall: (f64, f64),
    ) -> Letting {
        if self.drag.is_none_or(|drag| drag.pointer != KEYBOARD) {
            return Letting::new(HandResult::None);
        }
        let drawn = self.corner(fraction, card, wall);
        self.drag = None;
        self.commit(drawn, wall)
    }

    /// The move on its way was answered. Answers the drop that waited behind it — send that now —
    /// or `None`, and then lets go of where the note was held, so it is drawn where the server has
    /// it.
    pub fn answered(&mut self) -> Option<(f64, f64)> {
        self.sending = false;
        if let Some((drop, target)) = self.queued.take() {
            self.sending = true;
            self.sending_drop = drop;
            return Some(target);
        }
        if self.held.is_some_and(|(drop, _)| drop == self.sending_drop) {
            self.held = None;
        }
        None
    }

    fn commit(&mut self, drawn: (f64, f64), wall: (f64, f64)) -> Letting {
        let target = fraction_of(drawn, wall);
        self.drops += 1;
        self.held = Some((self.drops, target));
        if self.sending {
            self.queued = Some((self.drops, target));
            return Letting {
                result: HandResult::Drop,
                target: Some(target),
                send_now: false,
            };
        }
        self.sending = true;
        self.sending_drop = self.drops;
        Letting {
            result: HandResult::Drop,
            target: Some(target),
            send_now: true,
        }
    }
}
