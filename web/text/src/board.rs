//! The family board's rules that are arithmetic rather than drawing
//! (docs/protocol.md, "Board"): what a note may say, the names its size,
//! colour, face and kind travel under, where a note sits on the wall, how
//! its text is fitted, and what the badge counts.
//!
//! Ported from the Apple app — NoteText, NoteSize, NoteColor, NoteFont,
//! NoteKind, RsvpAnswer and BoardBadge, with the Mac's card metrics and
//! tilt from MacBoardView — so the web draws the wall the family already
//! knows, and counts the same badge the apps count.
//!
//! Every name that comes off the wire falls back rather than failing: a
//! note from a newer server with a fourth size or a fifth colour still has
//! to be readable, and a hole in the family's shared layout would be worse
//! than a sticker drawn in the default.
use crate::i18n::{t, t1, t2};

/// The longest a note may be: "text is trimmed, non-empty and at most 280
/// characters" — counted as the server counts them, in Unicode scalars.
pub const MAX_TEXT_CHARS: usize = 280;

/// The longest an event's place may be, counted the same way.
pub const MAX_PLACE_CHARS: usize = 200;

/// The longest one line of a task list may be, counted the same way — a
/// thing to do, not a paragraph about it.
pub const MAX_TASK_ITEM_CHARS: usize = 100;

/// The most lines one list may hold. The server's own `max_task_items`
/// default: a client that lets somebody type a twenty-first line is a
/// client whose save fails for a reason nobody can see.
pub const MAX_TASK_ITEMS: usize = 20;

/// The counter under the editor shows only in the last 40, so an ordinary
/// note is written without a number counting down at it (NoteText).
pub const COUNTER_FROM: usize = 40;

/// The first `max` Unicode scalars of `text`, which is what may be kept of
/// it.
///
/// Scalars, because the server counts `chars()`: a browser's `length` is
/// UTF-16 units and would give an emoji note half its allowance, and a
/// grapheme count is SMALLER than the server's for a family emoji or a
/// letter typed with a combining mark — a note that looked under the cap
/// would come back refused.
pub fn capped(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

/// Cut what went past `max` out of what was just typed or pasted — the
/// scalars immediately before the caret — rather than off the END of the
/// note, which ate the words after the place somebody was typing in, and
/// moved their caret to the end besides. Only when the caret is too near
/// the start to take it all from there is the end cut, as before.
///
/// `caret` and the caret answered are in UTF-16 units, as a browser's
/// `selectionStart` counts them. Answers the text kept and where the caret
/// goes.
pub fn cap_at_caret(value: &str, caret: usize, max: usize) -> (String, usize) {
    let count = value.chars().count();
    if count <= max {
        return (value.to_string(), caret);
    }
    let mut units = 0;
    let mut caret_byte = value.len();
    for (index, character) in value.char_indices() {
        if units >= caret {
            caret_byte = index;
            break;
        }
        units += character.len_utf16();
    }
    let before: Vec<usize> = value[..caret_byte]
        .char_indices()
        .map(|(index, _)| index)
        .collect();
    let removable = (count - max).min(before.len());
    let cut_from = if removable == 0 {
        caret_byte
    } else {
        before[before.len() - removable]
    };
    let mut kept = String::with_capacity(value.len());
    kept.push_str(&value[..cut_from]);
    kept.push_str(&value[caret_byte..]);
    let kept = capped(&kept, max).to_string();
    (kept, value[..cut_from].encode_utf16().count())
}

/// How many more characters the author may type. Never negative.
pub fn remaining(text: &str) -> usize {
    MAX_TEXT_CHARS.saturating_sub(text.chars().count())
}

/// Whether the counter is worth showing yet.
pub fn shows_counter(text: &str) -> bool {
    remaining(text) <= COUNTER_FROM
}

/// A note's size: a STEP the author chooses, drawn at each client's own
/// idiom — never a measurement on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Size {
    Small,
    Medium,
    Large,
}

impl Size {
    /// Small to large, the order a picker offers them in.
    pub const ALL: [Size; 3] = [Size::Small, Size::Medium, Size::Large];

    /// An unknown or absent name reads as medium — what every note was
    /// before sizes existed.
    pub fn from_name(name: Option<&str>) -> Size {
        match name {
            Some("small") => Size::Small,
            Some("large") => Size::Large,
            _ => Size::Medium,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Size::Small => "small",
            Size::Medium => "medium",
            Size::Large => "large",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Size::Small => t("Small"),
            Size::Medium => t("Medium"),
            Size::Large => t("Large"),
        }
    }

    /// What an edit sends for size: the chosen name when the author changed
    /// it, nothing when they did not. A size this client does not know
    /// DRAWS as medium but must not be WRITTEN BACK as medium because the
    /// author fixed a typo (NoteSize.patchName).
    pub fn patch_name(self, stored: Option<&str>) -> Option<&'static str> {
        (self != Size::from_name(stored)).then_some(self.name())
    }

    /// The card, in CSS pixels: the Mac's landscape card on a wall wide
    /// enough for it, and the phone's square sticker on a narrow one — the
    /// same step drawn at the idiom of the screen it is on.
    pub fn frame(self, compact: bool) -> (f64, f64) {
        match (self, compact) {
            (Size::Small, false) => (120.0, 88.0),
            (Size::Medium, false) => (150.0, 110.0),
            (Size::Large, false) => (280.0, 200.0),
            (Size::Small, true) => (100.0, 100.0),
            (Size::Medium, true) => (132.0, 132.0),
            (Size::Large, true) => (220.0, 220.0),
        }
    }

    /// The type size the text starts from — the CEILING, which fitting
    /// scales down from until the whole note is inside the card. It climbs
    /// with the step: a large note is meant to be read from across the room.
    ///
    /// The Mac's own steps — footnote, callout and body, 10, 12 and 13 pt —
    /// scaled by this page's 15 px to the Mac's 13 pt, so the same note on
    /// the same card reads the same size on both, and reaches the fitting
    /// floor at the same length.
    pub fn type_px(self) -> f64 {
        match self {
            Size::Small => 12.0,
            Size::Medium => 14.0,
            Size::Large => 15.0,
        }
    }
}

/// How far the type may shrink before the text is cut instead — the same
/// fraction at every step, as on the apps. Below it the note truncates,
/// which is the one case a reader has to open it for.
pub const MIN_TEXT_SCALE: f64 = 0.6;

/// How many halvings the fitting search takes: a hundredth of the range
/// from the floor to the ceiling is well under a pixel at every size.
pub const FIT_STEPS: u32 = 7;

/// The largest type scale, from `floor` up to 1, at which the text fits —
/// or None when it does not fit even at the floor, and must be cut.
///
/// `fits` answers for one scale. Smaller type never fits worse, which is
/// what lets this halve the range rather than walk it.
pub fn fitted_scale(mut fits: impl FnMut(f64) -> bool, floor: f64, steps: u32) -> Option<f64> {
    if fits(1.0) {
        return Some(1.0);
    }
    if !fits(floor) {
        return None;
    }
    // `low` always fits and `high` never does.
    let (mut low, mut high) = (floor, 1.0);
    for _ in 0..steps {
        let middle = (low + high) / 2.0;
        if fits(middle) {
            low = middle;
        } else {
            high = middle;
        }
    }
    Some(low)
}

/// The lines a box `height` tall holds at `line_height`, at least one — how
/// far text cut at the floor may run before its ellipsis.
pub fn lines_that_fit(height: f64, line_height: f64) -> u32 {
    if line_height <= 0.0 || !height.is_finite() {
        return 1;
    }
    ((height / line_height).floor() as u32).max(1)
}

/// The six colours the protocol allows, in the picker's order.
pub const COLORS: [&str; 6] = ["yellow", "pink", "blue", "green", "orange", "purple"];

/// A colour name's pastel — the apps' own values. An unknown name draws
/// yellow, the first and the most note-like, rather than failing.
pub fn color_hex(name: &str) -> &'static str {
    match name {
        "pink" => "#fcc7d9",
        "blue" => "#c2e0fc",
        "green" => "#c9f0c9",
        "orange" => "#ffd9b3",
        "purple" => "#e0d1fa",
        _ => "#fff2b3",
    }
}

/// A note's hand: an INTENT, which each client resolves to a face of its
/// own (docs/protocol.md, "Board") — never a family name on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Font {
    Plain,
    Serif,
    Mono,
    Casual,
}

impl Font {
    /// Plainest first, as the picker shows them.
    pub const ALL: [Font; 4] = [Font::Plain, Font::Serif, Font::Mono, Font::Casual];

    /// An unknown or absent name reads as plain — the face every note had
    /// before fonts existed.
    pub fn from_name(name: Option<&str>) -> Font {
        match name {
            Some("serif") => Font::Serif,
            Some("mono") => Font::Mono,
            Some("casual") => Font::Casual,
            _ => Font::Plain,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Font::Plain => "plain",
            Font::Serif => "serif",
            Font::Mono => "mono",
            Font::Casual => "casual",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Font::Plain => t("Plain"),
            Font::Serif => t("Serif"),
            Font::Mono => t("Mono"),
            Font::Casual => t("Casual"),
        }
    }

    /// The same rule `Size::patch_name` follows, one field over.
    pub fn patch_name(self, stored: Option<&str>) -> Option<&'static str> {
        (self != Font::from_name(stored)).then_some(self.name())
    }

    /// The CSS family stack each intent resolves to — system faces only,
    /// nothing downloaded and nothing bundled, the same promise the apps
    /// keep. `casual` is whichever friendly face the system has to hand:
    /// the rounded one where there is one, otherwise the informal one.
    pub fn css_family(self) -> &'static str {
        match self {
            Font::Plain => r#"system-ui, -apple-system, "Segoe UI", Roboto, sans-serif"#,
            Font::Serif => r#"ui-serif, "New York", Georgia, "Times New Roman", serif"#,
            Font::Mono => {
                r#"ui-monospace, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace"#
            }
            Font::Casual => {
                r#"ui-rounded, "SF Pro Rounded", "Comic Sans MS", "Chalkboard SE", "Segoe Print", cursive"#
            }
        }
    }
}

/// What a note IS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Text,
    Photo,
    Event,
    Tasks,
}

impl Kind {
    /// An unknown kind DRAWS AS TEXT rather than being dropped: the note
    /// still has a slot on a shared wall.
    pub fn from_name(name: Option<&str>) -> Kind {
        match name {
            Some("photo") => Kind::Photo,
            Some("event") => Kind::Event,
            Some("tasks") => Kind::Tasks,
            _ => Kind::Text,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Photo => "photo",
            Kind::Event => "event",
            Kind::Tasks => "tasks",
        }
    }
}

/// Whether somebody is coming to an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answer {
    Going,
    Maybe,
    No,
}

impl Answer {
    pub const ALL: [Answer; 3] = [Answer::Going, Answer::Maybe, Answer::No];

    /// An answer from a newer server is not drawn as one of these — better
    /// no button lit than a claim that somebody said something else.
    pub fn from_name(name: Option<&str>) -> Option<Answer> {
        match name {
            Some("going") => Some(Answer::Going),
            Some("maybe") => Some(Answer::Maybe),
            Some("no") => Some(Answer::No),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Answer::Going => "going",
            Answer::Maybe => "maybe",
            Answer::No => "no",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Answer::Going => t("Going"),
            Answer::Maybe => t("Maybe"),
            Answer::No => t("Can't"),
        }
    }
}

/// Who is coming, as the sticker says it: the counts, not the names — a
/// sticker has room for the news, and the note that opens has room for the
/// people. Nothing at all while nobody is going or thinking about it.
pub fn going_line(going: usize, maybe: usize) -> Option<String> {
    match (going, maybe) {
        (0, 0) => None,
        (going, 0) => Some(t1("%lld going", &going.to_string())),
        (0, maybe) => Some(t1("%lld maybe", &maybe.to_string())),
        (going, maybe) => Some(t2(
            "%lld going, %lld maybe",
            &going.to_string(),
            &maybe.to_string(),
        )),
    }
}


/// A few degrees of tilt, derived from the id so a note keeps the same
/// angle for everyone and across reloads — a wall of perfectly square notes
/// reads as a table, not a pinboard. The Mac's -3…3.
pub fn tilt_degrees(note_id: i64) -> i64 {
    note_id.rem_euclid(7) - 3
}

/// How many windows tall the wall is (docs/protocol.md, "Board").
///
/// A wall the size of the window is a wall that fills up, and then a family
/// has to take something down before it can say anything. `x` and `y` are
/// fractions of the WALL, so making it taller moves nothing relative to
/// anything else — it only means the bottom of the wall is below the
/// bottom of the window, and the wall scrolls.
///
/// The number is shared by all four clients on purpose, even though the
/// wire says nothing about it: a note two thirds of the way down should be
/// two thirds of the way down on the phone and on the Mac.
pub const WALL_SCREENS: f64 = 1.6;

/// The wall's height for a window of `visible` height.
pub fn wall_height(visible: f64) -> f64 {
    (visible * WALL_SCREENS).max(visible)
}

/// How many lines of a task list a STICKER draws before it says how many
/// more there are (docs/protocol.md, "Board").
///
/// One number for all four clients, for the reason `WALL_SCREENS` is one:
/// a list that ran to a different point on the phone and on the Mac would
/// be a different list. The note itself always has them all.
pub const WALL_TASK_LINES: usize = 5;

/// The lines a sticker draws, and how many it had to leave — `(shown,
/// left)`, where `left` is 0 on a list that fits.
pub fn wall_task_lines(total: usize) -> (usize, usize) {
    let shown = total.min(WALL_TASK_LINES);
    (shown, total - shown)
}

/// Below this width a wall takes the phone's square stickers: the Mac's
/// landscape cards would cover most of a narrow board between them.
pub const COMPACT_BELOW: f64 = 640.0;

pub fn is_compact(board_width: f64) -> bool {
    board_width < COMPACT_BELOW
}

/// A card's top-left corner held inside the board, so no part of it is off
/// screen — which depends on the card, a large one running out of room
/// sooner. A board smaller than the card pins it to the top-left.
pub fn clamp_corner(x: f64, y: f64, card: (f64, f64), board: (f64, f64)) -> (f64, f64) {
    (
        x.min(board.0 - card.0).max(0.0),
        y.min(board.1 - card.1).max(0.0),
    )
}

/// Where a stored position puts a card: the fraction is its TOP-LEFT
/// corner, as the protocol says and the apps draw it — drawn clamped, so a
/// stored 0.98 hugs the edge rather than hanging off it.
pub fn origin(fraction: (f64, f64), card: (f64, f64), board: (f64, f64)) -> (f64, f64) {
    clamp_corner(fraction.0 * board.0, fraction.1 * board.1, card, board)
}

/// Where a card sits while it is in hand: its origin moved by the drag, and
/// held inside again — clamping only on release would let a note be
/// dragged off the edge and then snap back.
pub fn dragged(
    fraction: (f64, f64),
    offset: (f64, f64),
    card: (f64, f64),
    board: (f64, f64),
) -> (f64, f64) {
    let (x, y) = origin(fraction, card, board);
    clamp_corner(x + offset.0, y + offset.1, card, board)
}

/// The fraction to store for a card DRAWN at `corner`: read back from
/// where it is, so what was dropped is what gets stored, clamped to the
/// wall as the server would clamp it anyway.
pub fn fraction_of(corner: (f64, f64), board: (f64, f64)) -> (f64, f64) {
    let unit = |value: f64, extent: f64| (value / extent.max(1.0)).clamp(0.0, 1.0);
    (unit(corner.0, board.0), unit(corner.1, board.1))
}

/// How much of the board a device has actually SHOWN its user — the badge's
/// marks, which are not the sync cursor (docs/protocol.md, "Board").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Marks {
    /// The highest note id shown: the rule for a note that carries no
    /// `content_seq`, from a server that predates it.
    pub note_id: i64,
    /// The highest `content_seq` shown: the rule.
    pub content_seq: i64,
}

/// One note's verdict: judged by its content seq when it has one, and by
/// its id when it does not.
pub fn is_unread(note_id: i64, content_seq: Option<i64>, marks: Marks) -> bool {
    match content_seq.filter(|seq| *seq > 0) {
        Some(seq) => seq > marks.content_seq,
        None => note_id > marks.note_id,
    }
}

/// The marks after the board has been on screen: everything on it has been
/// shown. Monotonic in both — a board that has just lost its newest note to
/// somebody's delete must not bring a cleared badge back.
pub fn marks_after_showing(
    notes: impl IntoIterator<Item = (i64, Option<i64>)>,
    marks: Marks,
) -> Marks {
    notes
        .into_iter()
        .fold(marks, |marks, (note_id, content_seq)| Marks {
            note_id: marks.note_id.max(note_id),
            content_seq: marks.content_seq.max(content_seq.unwrap_or(0)),
        })
}

/// The later of two sets of marks, field by field — two tabs of one account
/// both write, and neither may walk the other's back.
pub fn later(a: Marks, b: Marks) -> Marks {
    Marks {
        note_id: a.note_id.max(b.note_id),
        content_seq: a.content_seq.max(b.content_seq),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_sticker_draws_the_first_lines_of_a_list_and_says_how_many_are_left() {
        assert_eq!(WALL_TASK_LINES, 5);
        assert_eq!(wall_task_lines(0), (0, 0));
        assert_eq!(wall_task_lines(3), (3, 0));
        // A list of exactly the cap says nothing extra.
        assert_eq!(wall_task_lines(5), (5, 0));
        assert_eq!(wall_task_lines(6), (5, 1));
        assert_eq!(wall_task_lines(20), (5, 15));
    }

    #[test]
    fn the_wall_is_taller_than_the_window_and_never_shorter() {
        // Taller, so there is room to pin something without taking
        // something down (docs/protocol.md, "Board").
        assert_eq!(wall_height(500.0), 800.0);
        assert!(wall_height(1000.0) > 1000.0);
        // And never SHORTER, whatever a window does: a wall smaller than
        // the window would put fractions of it behind the edges.
        assert_eq!(wall_height(0.0), 0.0);
        assert!(wall_height(1.0) >= 1.0);
    }

    use super::*;

    #[test]
    fn the_cap_counts_scalars_the_way_the_server_does() {
        let short = "Milk and eggs";
        assert_eq!(capped(short, MAX_TEXT_CHARS), short);
        let long = "a".repeat(300);
        assert_eq!(capped(&long, MAX_TEXT_CHARS).chars().count(), 280);
        // A family emoji is ONE grapheme and SEVEN scalars: the server counts
        // seven, so the cap must too.
        let family = "👨‍👩‍👧‍👦";
        assert_eq!(family.chars().count(), 7);
        let many = family.repeat(41);
        assert_eq!(capped(&many, MAX_TEXT_CHARS).chars().count(), 280);
        // Cut on a scalar boundary, never inside one.
        let cyrillic = "Привет".repeat(50);
        let kept = capped(&cyrillic, MAX_TEXT_CHARS);
        assert_eq!(kept.chars().count(), 280);
        assert!(cyrillic.starts_with(kept));
        // Exactly at the cap is kept whole.
        let exact = "б".repeat(280);
        assert_eq!(capped(&exact, MAX_TEXT_CHARS), exact);
        assert_eq!(capped("", MAX_TEXT_CHARS), "");
    }

    /// Typing into a full note takes the new character back, not the end
    /// of the note; a paste keeps what fits of itself; and the caret stays
    /// where the typing was.
    #[test]
    fn a_full_note_refuses_what_was_just_typed_not_its_own_end() {
        let full = format!("{}END", "a".repeat(277));
        assert_eq!(full.chars().count(), 280);
        // One character typed at position 10.
        let typed = format!("{}x{}", &full[..10], &full[10..]);
        let (kept, caret) = cap_at_caret(&typed, 11, MAX_TEXT_CHARS);
        assert_eq!(kept, full, "the x went, and the END stayed");
        assert_eq!(caret, 10);
        // A paste of five into 278: two of it fit.
        let base = format!("{}END", "b".repeat(275));
        let pasted = format!("{}12345{}", &base[..100], &base[100..]);
        let (kept, caret) = cap_at_caret(&pasted, 105, MAX_TEXT_CHARS);
        assert_eq!(kept, format!("{}12{}", &base[..100], &base[100..]));
        assert_eq!(caret, 102);
        assert!(kept.ends_with("END"));
        // Under the cap, nothing changes.
        assert_eq!(
            cap_at_caret("hello", 2, MAX_TEXT_CHARS),
            ("hello".to_string(), 2)
        );
        // Carets count UTF-16: an emoji before the caret is two units.
        let emoji = format!("😀{}", "c".repeat(280));
        let (kept, caret) = cap_at_caret(&emoji, 2, MAX_TEXT_CHARS);
        assert_eq!(kept, "c".repeat(280), "the emoji just typed went");
        assert_eq!(caret, 0);
        // …and the caret answered counts them the same way: two emoji before
        // a typed x are four units, so the caret lands at 14, not at 12.
        let wide = format!("😀😀{}x{}", "a".repeat(10), "a".repeat(268));
        assert_eq!(wide.chars().count(), 281);
        let (kept, caret) = cap_at_caret(&wide, 15, MAX_TEXT_CHARS);
        assert_eq!(kept, format!("😀😀{}", "a".repeat(278)));
        assert_eq!(caret, 14);
        // A caret at the very start has nothing before it to take: the end
        // is cut, as it always was.
        let (kept, caret) = cap_at_caret(&"d".repeat(290), 0, MAX_TEXT_CHARS);
        assert_eq!(kept.chars().count(), 280);
        assert_eq!(caret, 0);
    }

    #[test]
    fn the_counter_shows_in_the_last_forty() {
        assert_eq!(remaining(""), 280);
        assert!(!shows_counter(&"a".repeat(239)));
        assert!(shows_counter(&"a".repeat(240)));
        assert_eq!(remaining(&"a".repeat(240)), 40);
        assert_eq!(remaining(&"a".repeat(400)), 0, "never negative");
        assert_eq!(remaining("é"), 279, "one scalar");
        assert_eq!(
            remaining("e\u{301}"),
            278,
            "a base and a combining mark are two"
        );
    }

    #[test]
    fn unknown_names_fall_back_to_what_every_note_was_before() {
        assert_eq!(Size::from_name(None), Size::Medium);
        assert_eq!(Size::from_name(Some("huge")), Size::Medium);
        assert_eq!(Size::from_name(Some("small")), Size::Small);
        assert_eq!(Size::from_name(Some("large")), Size::Large);
        assert_eq!(Font::from_name(None), Font::Plain);
        assert_eq!(Font::from_name(Some("gothic")), Font::Plain);
        assert_eq!(Font::from_name(Some("casual")), Font::Casual);
        assert_eq!(Font::from_name(Some("serif")), Font::Serif);
        assert_eq!(Font::from_name(Some("mono")), Font::Mono);
        assert_eq!(Kind::from_name(None), Kind::Text);
        assert_eq!(
            Kind::from_name(Some("poll")),
            Kind::Text,
            "an unknown kind draws as text"
        );
        assert_eq!(Kind::from_name(Some("photo")), Kind::Photo);
        assert_eq!(Kind::from_name(Some("event")), Kind::Event);
        assert_eq!(color_hex("teal"), color_hex("yellow"));
        assert_eq!(Answer::from_name(Some("perhaps")), None, "no button lit");
        assert_eq!(Answer::from_name(None), None);
        assert_eq!(Answer::from_name(Some("no")), Some(Answer::No));
    }

    #[test]
    fn names_round_trip_as_the_wire_spells_them() {
        for size in Size::ALL {
            assert_eq!(Size::from_name(Some(size.name())), size);
        }
        for font in Font::ALL {
            assert_eq!(Font::from_name(Some(font.name())), font);
        }
        for kind in [Kind::Text, Kind::Photo, Kind::Event] {
            assert_eq!(Kind::from_name(Some(kind.name())), kind);
        }
        for answer in Answer::ALL {
            assert_eq!(Answer::from_name(Some(answer.name())), Some(answer));
        }
        assert_eq!(
            Size::ALL.map(Size::name),
            ["small", "medium", "large"],
            "the picker's order"
        );
        assert_eq!(
            Font::ALL.map(Font::name),
            ["plain", "serif", "mono", "casual"]
        );
        assert_eq!(
            COLORS,
            ["yellow", "pink", "blue", "green", "orange", "purple"]
        );
        // Every colour has its own pastel; only the unknown one borrows.
        let hexes: std::collections::HashSet<_> =
            COLORS.iter().map(|name| color_hex(name)).collect();
        assert_eq!(hexes.len(), 6);
    }

    /// A size or a face this client does not know DRAWS as the default and
    /// is never WRITTEN BACK as it because the author fixed the text.
    #[test]
    fn an_untouched_size_or_face_is_not_sent() {
        assert_eq!(Size::Medium.patch_name(Some("huge")), None);
        assert_eq!(Size::Medium.patch_name(Some("medium")), None);
        assert_eq!(Size::Medium.patch_name(None), None);
        assert_eq!(Size::Large.patch_name(Some("medium")), Some("large"));
        assert_eq!(Size::Large.patch_name(Some("huge")), Some("large"));
        assert_eq!(Font::Plain.patch_name(Some("gothic")), None);
        assert_eq!(Font::Serif.patch_name(Some("plain")), Some("serif"));
        assert_eq!(Font::Plain.patch_name(Some("serif")), Some("plain"));
    }

    #[test]
    fn cards_and_type_climb_with_the_step() {
        for compact in [false, true] {
            let frames = Size::ALL.map(|size| size.frame(compact));
            assert!(frames[0].0 < frames[1].0 && frames[1].0 < frames[2].0);
            assert!(frames[0].1 < frames[1].1 && frames[1].1 < frames[2].1);
        }
        assert_eq!(
            Size::Medium.frame(false),
            (150.0, 110.0),
            "the Mac's medium card"
        );
        assert_eq!(
            Size::Medium.frame(true),
            (132.0, 132.0),
            "the phone's medium sticker"
        );
        assert!(Size::Small.type_px() < Size::Medium.type_px());
        assert!(Size::Medium.type_px() < Size::Large.type_px());
        assert!(is_compact(639.0) && !is_compact(640.0));
    }

    /// The largest scale that fits — found, not walked — and None when even
    /// the floor is too big.
    #[test]
    fn fitting_finds_the_largest_scale_that_fits() {
        // Fits at 1: no search at all.
        let mut asked = Vec::new();
        assert_eq!(
            fitted_scale(
                |scale| {
                    asked.push(scale);
                    true
                },
                MIN_TEXT_SCALE,
                FIT_STEPS
            ),
            Some(1.0)
        );
        assert_eq!(asked, vec![1.0]);

        // Fits only at or below 0.8.
        let answer = fitted_scale(|scale| scale <= 0.8, MIN_TEXT_SCALE, FIT_STEPS).expect("fits");
        assert!(answer <= 0.8, "{answer}");
        assert!(
            0.8 - answer < 0.4 / 2f64.powi(FIT_STEPS as i32) + 1e-9,
            "{answer}"
        );

        // Fits only at the floor itself.
        assert_eq!(
            fitted_scale(|scale| scale <= MIN_TEXT_SCALE, MIN_TEXT_SCALE, FIT_STEPS),
            Some(MIN_TEXT_SCALE)
        );

        // Never fits: cut at the floor instead.
        assert_eq!(fitted_scale(|_| false, MIN_TEXT_SCALE, FIT_STEPS), None);
    }

    #[test]
    fn text_cut_at_the_floor_keeps_the_lines_it_has_room_for() {
        assert_eq!(lines_that_fit(60.0, 10.0), 6);
        assert_eq!(lines_that_fit(59.9, 10.0), 5);
        assert_eq!(lines_that_fit(4.0, 10.0), 1, "at least one");
        assert_eq!(lines_that_fit(40.0, 0.0), 1);
        assert_eq!(lines_that_fit(f64::NAN, 10.0), 1);
    }

    #[test]
    fn the_counts_say_who_is_coming_and_nothing_while_nobody_is() {
        assert_eq!(going_line(0, 0), None);
        assert_eq!(going_line(3, 0).as_deref(), Some("3 going"));
        assert_eq!(going_line(0, 2).as_deref(), Some("2 maybe"));
        assert_eq!(going_line(3, 2).as_deref(), Some("3 going, 2 maybe"));
        assert_eq!(Answer::No.title(), "Can't");
    }

    #[test]
    fn the_tilt_is_the_macs_and_never_changes() {
        assert_eq!(tilt_degrees(0), -3);
        assert_eq!(tilt_degrees(3), 0);
        assert_eq!(tilt_degrees(6), 3);
        assert_eq!(tilt_degrees(7), -3);
        assert_eq!(tilt_degrees(12), 2);
        for id in -20..40 {
            assert!((-3..=3).contains(&tilt_degrees(id)), "{id}");
            assert_eq!(tilt_degrees(id), tilt_degrees(id + 7));
        }
    }

    /// The corner, not the centre, and drawn inside the board — a stored
    /// 0.98 hugs the right edge rather than hanging off it.
    #[test]
    fn a_note_is_drawn_by_its_corner_and_inside_the_wall() {
        let board = (1000.0, 600.0);
        let card = (150.0, 110.0);
        assert_eq!(origin((0.5, 0.5), card, board), (500.0, 300.0));
        assert_eq!(origin((0.98, 0.99), card, board), (850.0, 490.0));
        assert_eq!(origin((0.0, 0.0), card, board), (0.0, 0.0));
        // A board smaller than the card pins it to the top-left.
        assert_eq!(origin((0.5, 0.5), card, (100.0, 50.0)), (0.0, 0.0));
    }

    /// Held inside the wall WHILE dragging, and stored as the fraction of
    /// where it was dropped — so a note pushed past the edge sticks there.
    #[test]
    fn a_drag_stays_on_the_wall_and_stores_where_it_was_dropped() {
        let board = (1000.0, 600.0);
        let card = (150.0, 110.0);
        let at = dragged((0.5, 0.5), (1000.0, -1000.0), card, board);
        assert_eq!(at, (850.0, 0.0), "held inside while in hand");
        assert_eq!(fraction_of(at, board), (0.85, 0.0));
        // A note on the edge pushed further out stores the same place.
        let edge = dragged((0.98, 0.5), (30.0, 0.0), card, board);
        assert_eq!(edge.0, 850.0);
        // A plain drop moves by exactly the drag.
        assert_eq!(
            dragged((0.1, 0.1), (50.0, 30.0), card, board),
            (150.0, 90.0)
        );
        assert_eq!(fraction_of((150.0, 90.0), board), (0.15, 0.15));
        // A wall with no size yet stores a real fraction, never NaN.
        let (x, y) = fraction_of((10.0, 10.0), (0.0, 0.0));
        assert!(x.is_finite() && y.is_finite());
        assert_eq!((x, y), (1.0, 1.0));
        assert_eq!(fraction_of((-5.0, -5.0), board), (0.0, 0.0));
    }

    /// The badge counts what there is to READ: content seq when a note
    /// carries one, its id when it does not.
    #[test]
    fn the_badge_counts_new_words_not_tidying() {
        let marks = Marks {
            note_id: 10,
            content_seq: 80,
        };
        assert!(!is_unread(5, Some(80), marks), "shown already");
        assert!(
            is_unread(5, Some(81), marks),
            "an OLD note rewritten counts"
        );
        assert!(
            !is_unread(12, Some(79), marks),
            "a new id with old words: a move into an empty cache"
        );
        assert!(is_unread(11, None, marks), "no content seq: the id rule");
        assert!(!is_unread(10, None, marks));
        assert!(is_unread(11, Some(0), marks), "zero is no seq at all");
    }

    #[test]
    fn showing_the_board_raises_both_marks_and_never_lowers_them() {
        let marks = Marks {
            note_id: 10,
            content_seq: 80,
        };
        assert_eq!(
            marks_after_showing([(4, Some(90)), (12, None), (7, Some(60))], marks),
            Marks {
                note_id: 12,
                content_seq: 90
            }
        );
        assert_eq!(
            marks_after_showing([(3, Some(20))], marks),
            marks,
            "a board that lost its newest note keeps the marks it had"
        );
        assert_eq!(marks_after_showing(std::iter::empty(), marks), marks);
        assert_eq!(
            later(
                Marks {
                    note_id: 3,
                    content_seq: 99
                },
                marks
            ),
            Marks {
                note_id: 10,
                content_seq: 99
            }
        );
    }
}
