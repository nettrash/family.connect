//! The board's shared arithmetic, evaluated by the ORIGINAL (`fc_text::board`, which the web
//! client runs) and printed as JSON, so the Windows port can be pinned to the same numbers rather
//! than to somebody's reading of them.
//!
//! Four implementations of one rule need an oracle, not four readings: this portfolio has been
//! bitten by a byte-versus-character split that panicked on Cyrillic, and by four ports that
//! agreed with each other and were all wrong about `pow(10, n)`.
//!
//! Output goes to `win/tests/FamilyConnect.Core.Tests/Fixtures/board-vectors.json` — see
//! Cargo.toml for the one command.
use fc_text::board as b;
use fc_text::{calendar, call_record, media, notify};

fn q(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

fn main() {
    // No argument: the board vectors, which is the command the docs give. `chat` prints the
    // chat-line vectors instead — the same idea for the words a chat row is drawn with.
    if std::env::args().nth(1).as_deref() == Some("chat") {
        chat();
        return;
    }
    let mut out = String::from("{\n");

    // --- capped -------------------------------------------------------
    let capped_cases: Vec<(&str, usize)> = vec![
        ("Milk and eggs", 4),
        ("Milk and eggs", 280),
        ("Milk", 0),
        ("🎂🎂", 1),
        ("🎂", 0),
        ("👨‍👩‍👧‍👦", 3),
        ("e\u{0301}clair", 2),
        ("é", 1),
        ("Привет", 3),
        ("a̐éö̲", 4),
    ];
    out.push_str("  \"capped\": [\n");
    for (text, max) in &capped_cases {
        out.push_str(&format!(
            "    {{\"text\": {}, \"max\": {}, \"kept\": {}, \"scalars\": {}}},\n",
            q(text),
            max,
            q(b::capped(text, *max)),
            text.chars().count()
        ));
    }
    out.push_str("  ],\n");

    // --- cap_at_caret -------------------------------------------------
    let long_a = "a".repeat(279) + "xyz" + &"b".repeat(20);
    let emoji = "🎂".repeat(200);
    let caret_cases: Vec<(String, usize, usize)> = vec![
        (long_a.clone(), 282, 280),
        (long_a.clone(), 0, 280),
        (long_a.clone(), 302, 280),
        ("Milk".to_string(), 2, 280),
        (emoji.clone(), 400, 280),
        (emoji.clone(), 10, 280),
        ("a".repeat(300), 150, 280),
        ("x".repeat(90) + "🎂", 92, 80),
    ];
    out.push_str("  \"cap_at_caret\": [\n");
    for (value, caret, max) in &caret_cases {
        let (kept, moved) = b::cap_at_caret(value, *caret, *max);
        out.push_str(&format!(
            "    {{\"value\": {}, \"caret\": {}, \"max\": {}, \"kept\": {}, \"moved\": {}}},\n",
            q(value),
            caret,
            max,
            q(&kept),
            moved
        ));
    }
    out.push_str("  ],\n");

    // --- remaining / counter ------------------------------------------
    out.push_str("  \"remaining\": [\n");
    for text in ["", "Milk", &"a".repeat(239), &"a".repeat(240), &"a".repeat(400)] {
        out.push_str(&format!(
            "    {{\"text\": {}, \"remaining\": {}, \"counter\": {}}},\n",
            q(text),
            b::remaining(text),
            b::shows_counter(text)
        ));
    }
    out.push_str("  ],\n");

    // --- fitted_picture ----------------------------------------------
    let picture_cases: Vec<((f64, f64), (f64, f64))> = vec![
        ((150.0, 110.0), (600.0, 1200.0)),
        ((150.0, 110.0), (1600.0, 900.0)),
        ((150.0, 110.0), (300.0, 220.0)),
        ((150.0, 110.0), (15.0, 11.0)),
        ((150.0, 110.0), (0.0, 0.0)),
        ((150.0, 110.0), (600.0, 0.0)),
        ((150.0, 110.0), (-4.0, 8.0)),
        ((150.0, 110.0), (20000.0, 10.0)),
        ((132.0, 132.0), (4032.0, 3024.0)),
        ((280.0, 200.0), (1080.0, 1920.0)),
    ];
    out.push_str("  \"fitted_picture\": [\n");
    for (space, picture) in &picture_cases {
        let (w, h) = b::fitted_picture(*space, *picture);
        out.push_str(&format!(
            "    {{\"space\": [{}, {}], \"picture\": [{}, {}], \"fitted\": [{}, {}]}},\n",
            space.0, space.1, picture.0, picture.1, w, h
        ));
    }
    out.push_str("  ],\n");

    // --- wall / geometry ---------------------------------------------
    out.push_str("  \"wall_height\": [\n");
    for visible in [0.0, 1.0, 500.0, 1000.0] {
        out.push_str(&format!(
            "    {{\"visible\": {}, \"height\": {}}},\n",
            visible,
            b::wall_height(visible)
        ));
    }
    out.push_str("  ],\n");

    out.push_str("  \"task_lines\": [\n");
    for total in [0usize, 3, 5, 6, 20] {
        let (shown, left) = b::wall_task_lines(total);
        out.push_str(&format!(
            "    {{\"total\": {}, \"shown\": {}, \"left\": {}}},\n",
            total, shown, left
        ));
    }
    out.push_str("  ],\n");

    out.push_str("  \"tilt\": [\n");
    for id in -8i64..=15 {
        out.push_str(&format!(
            "    {{\"id\": {}, \"degrees\": {}}},\n",
            id,
            b::tilt_degrees(id)
        ));
    }
    out.push_str("  ],\n");

    let geometry_cases: Vec<((f64, f64), (f64, f64), (f64, f64), (f64, f64))> = vec![
        ((0.5, 0.25), (150.0, 110.0), (900.0, 600.0), (20.0, -20.0)),
        ((0.98, 0.98), (150.0, 110.0), (900.0, 600.0), (0.0, 0.0)),
        ((-1.0, -1.0), (150.0, 110.0), (900.0, 600.0), (5.0, 5.0)),
        ((0.5, 0.5), (150.0, 110.0), (100.0, 80.0), (0.0, 0.0)),
        ((0.1, 0.9), (280.0, 200.0), (1200.0, 1600.0), (-400.0, 400.0)),
    ];
    out.push_str("  \"geometry\": [\n");
    for (fraction, card, board, offset) in &geometry_cases {
        let origin = b::origin(*fraction, *card, *board);
        let dragged = b::dragged(*fraction, *offset, *card, *board);
        let back = b::fraction_of(origin, *board);
        out.push_str(&format!(
            "    {{\"fraction\": [{}, {}], \"card\": [{}, {}], \"board\": [{}, {}], \"offset\": [{}, {}], \"origin\": [{}, {}], \"dragged\": [{}, {}], \"fraction_of_origin\": [{}, {}]}},\n",
            fraction.0, fraction.1, card.0, card.1, board.0, board.1, offset.0, offset.1,
            origin.0, origin.1, dragged.0, dragged.1, back.0, back.1
        ));
    }
    out.push_str("  ],\n");

    // --- names, colours, cards ---------------------------------------
    out.push_str("  \"cards\": [\n");
    for size in b::Size::ALL {
        for compact in [false, true] {
            let (w, h) = size.frame(compact);
            out.push_str(&format!(
                "    {{\"size\": {}, \"compact\": {}, \"card\": [{}, {}], \"type_px\": {}}},\n",
                q(size.name()),
                compact,
                w,
                h,
                size.type_px()
            ));
        }
    }
    out.push_str("  ],\n");

    out.push_str("  \"colors\": [\n");
    for name in b::COLORS.iter().copied().chain(["chartreuse"]) {
        out.push_str(&format!(
            "    {{\"name\": {}, \"hex\": {}}},\n",
            q(name),
            q(b::color_hex(name))
        ));
    }
    out.push_str("  ],\n");

    out.push_str("  \"fallbacks\": [\n");
    for name in [
        Some("small"), Some("medium"), Some("large"), Some("huge"), Some("Large"), None,
    ] {
        out.push_str(&format!(
            "    {{\"given\": {}, \"size\": {}}},\n",
            match name { Some(n) => q(n), None => "null".into() },
            q(b::Size::from_name(name).name())
        ));
    }
    out.push_str("  ],\n");

    out.push_str("  \"kinds\": [\n");
    for name in [Some("text"), Some("photo"), Some("event"), Some("tasks"), Some("video"), None] {
        out.push_str(&format!(
            "    {{\"given\": {}, \"kind\": {}}},\n",
            match name { Some(n) => q(n), None => "null".into() },
            q(b::Kind::from_name(name).name())
        ));
    }
    out.push_str("  ],\n");

    out.push_str("  \"answers\": [\n");
    for name in [Some("going"), Some("maybe"), Some("no"), Some("perhaps"), None] {
        out.push_str(&format!(
            "    {{\"given\": {}, \"answer\": {}}},\n",
            match name { Some(n) => q(n), None => "null".into() },
            match b::Answer::from_name(name) { Some(a) => q(a.name()), None => "null".into() }
        ));
    }
    out.push_str("  ],\n");

    out.push_str("  \"going_line\": [\n");
    for (going, maybe) in [(0usize, 0usize), (2, 0), (0, 1), (2, 1), (7, 3)] {
        out.push_str(&format!(
            "    {{\"going\": {}, \"maybe\": {}, \"line\": {}}},\n",
            going,
            maybe,
            match b::going_line(going, maybe) { Some(line) => q(&line), None => "null".into() }
        ));
    }
    out.push_str("  ],\n");

    out.push_str("  \"lines_that_fit\": [\n");
    for (height, line_height) in [(80.0, 20.0), (10.0, 20.0), (80.0, 0.0), (0.0, 20.0)] {
        out.push_str(&format!(
            "    {{\"height\": {}, \"line_height\": {}, \"lines\": {}}},\n",
            height,
            line_height,
            b::lines_that_fit(height, line_height)
        ));
    }
    out.push_str("  ],\n");

    out.push_str("  \"unread\": [\n");
    let marks = b::Marks { note_id: 10, content_seq: 100 };
    for (note_id, content_seq) in [
        (3i64, Some(101i64)), (99, Some(100)), (11, None), (10, None), (11, Some(0)),
    ] {
        out.push_str(&format!(
            "    {{\"note_id\": {}, \"content_seq\": {}, \"unread\": {}}},\n",
            note_id,
            match content_seq { Some(seq) => seq.to_string(), None => "null".into() },
            b::is_unread(note_id, content_seq, marks)
        ));
    }
    out.push_str("  ],\n");

    out.push_str(&format!(
        "  \"constants\": {{\"max_text\": {}, \"max_place\": {}, \"max_task_item\": {}, \"max_task_items\": {}, \"counter_from\": {}, \"wall_screens\": {}, \"wall_task_lines\": {}, \"compact_below\": {}, \"min_text_scale\": {}, \"fit_steps\": {}}}\n",
        b::MAX_TEXT_CHARS, b::MAX_PLACE_CHARS, b::MAX_TASK_ITEM_CHARS, b::MAX_TASK_ITEMS,
        b::COUNTER_FROM, b::WALL_SCREENS, b::WALL_TASK_LINES, b::COMPACT_BELOW,
        b::MIN_TEXT_SCALE, b::FIT_STEPS
    ));
    out.push_str("}\n");

    // Trailing commas are not JSON: strip the one before each closing bracket.
    let cleaned = out.replace(",\n  ]", "\n  ]");
    print!("{}", cleaned);
}

/// The words a chat ROW is drawn with, from the same crate: a call record's line, what an
/// attachment is called when it has no name, and the two notification sentences. The preview
/// itself is each client's own composition, but every piece it leans on is here.
fn chat() {
    let mut out = String::from("{\n");

    out.push_str("  \"call_record\": [\n");
    for outcome in ["completed", "missed", "declined", "failed", "hologram"] {
        for duration in [None, Some(0i64), Some(61), Some(222), Some(3762), Some(-5)] {
            for video in [false, true] {
                for mine in [false, true] {
                    out.push_str(&format!(
                        "    {{\"outcome\": {}, \"duration_secs\": {}, \"video\": {}, \"mine\": {}, \"said\": {}}},\n",
                        q(outcome),
                        match duration { Some(secs) => secs.to_string(), None => "null".into() },
                        video,
                        mine,
                        q(&call_record::label(outcome, duration, video, mine))
                    ));
                }
            }
        }
    }
    out.push_str("  ],\n");

    out.push_str("  \"duration\": [\n");
    for seconds in [0i64, 9, 59, 60, 61, 599, 3599, 3600, 3762, 86399, -1] {
        out.push_str(&format!(
            "    {{\"seconds\": {}, \"said\": {}}},\n",
            seconds,
            q(&call_record::duration(seconds))
        ));
    }
    out.push_str("  ],\n");

    out.push_str("  \"display_name\": [\n");
    for kind in ["photo", "video", "audio", "file", "location", "hologram"] {
        for name in [None, Some(""), Some("receipts.pdf")] {
            out.push_str(&format!(
                "    {{\"kind\": {}, \"name\": {}, \"said\": {}}},\n",
                q(kind),
                match name { Some(name) => q(name), None => "null".into() },
                q(&media::display_name(kind, name))
            ));
        }
    }
    out.push_str("  ],\n");

    out.push_str("  \"notify_title\": [\n");
    for family in [None, Some("The Smiths")] {
        for mentioned in [false, true] {
            out.push_str(&format!(
                "    {{\"family\": {}, \"sender\": {}, \"mentioned\": {}, \"said\": {}}},\n",
                match family { Some(name) => q(name), None => "null".into() },
                q("Anna"),
                mentioned,
                q(&notify::title(family, "Anna", mentioned))
            ));
        }
    }
    out.push_str("  ],\n");

    out.push_str("  \"page_title\": [\n");
    for unread in [-1i64, 0, 1, 3, 99, 100, 1000] {
        out.push_str(&format!(
            "    {{\"unread\": {}, \"said\": {}}},\n",
            unread,
            q(&notify::page_title("Family Connect", unread))
        ));
    }
    out.push_str("  ],\n");

    // The `.ics` a client writes when it has nowhere else to put an event. Nothing here is on
    // the wire, which is exactly why four clients writing it four ways would diverge in silence.
    let long_a = "a".repeat(200);
    let long_ya = "\u{44f}".repeat(120);
    let long_b = "\u{431}".repeat(90);
    let events: Vec<(&str, &str, &str, Option<&str>, Option<&str>)> = vec![
        ("fc-note-12@nettrash", "Christmas dinner", "20261224T160000Z",
         Some("20261224T200000Z"), Some("Gran's house")),
        ("fc-note-13@nettrash", "Lunch, then a walk; bring boots", "20261225T120000Z",
         None, None),
        ("fc-note-14@nettrash", "Back\\slash and a\nnewline", "20261226T090000Z",
         None, Some("Somewhere, else; really")),
        ("fc-note-15@nettrash",
         "День рождения бабушки и ещё очень длинное название события которое точно не влезает",
         "20261227T100000Z", Some("20261227T140000Z"), Some("У бабушки дома, в саду")),
        ("fc-note-16@nettrash", "A title that is exactly long enough in plain ASCII to fold once",
         "20261228T080000Z", None, None),
        ("fc-note-17@nettrash", "🎂🎂🎂 a cake for every one of the very many guests we invited",
         "20261229T080000Z", None, None),
        // A ZWJ sequence, which is ONE grapheme and seven code points: the fold is per CODE
        // POINT, so a client folding by grapheme cluster would break the file one byte earlier
        // and no oracle-less test would ever notice.
        // Long enough to fold SEVERAL times: every continuation's leading space is itself
        // an octet of the folded line, so an off-by-one in that budget shows up here and
        // nowhere else.
        ("fc-note-19@nettrash", &long_a, "20261231T080000Z", None, None),
        ("fc-note-20@nettrash", &long_ya, "20270101T080000Z", None, Some(&long_b)),
        ("fc-note-18@nettrash",
         "A very long title indeed, long enough to fold right here 👨\u{200d}👩\u{200d}👧\u{200d}👦 and then some more",
         "20261230T080000Z", None, None),
    ];
    out.push_str("  \"ics\": [\n");
    for (uid, title, starts, ends, place) in &events {
        out.push_str(&format!(
            "    {{\"uid\": {}, \"title\": {}, \"starts_at\": {}, \"ends_at\": {}, \"place\": {}, \"file\": {}}},\n",
            q(uid),
            q(title),
            q(starts),
            match ends { Some(ends) => q(ends), None => "null".into() },
            match place { Some(place) => q(place), None => "null".into() },
            q(&calendar::one_event(uid, title, starts, *ends, *place, "20260912T120000Z"))
        ));
    }
    out.push_str("  ],\n");

    out.push_str(&format!(
        "  \"bodies\": {{\"new_message\": {}, \"new_note\": {}}}\n",
        q(notify::new_message()),
        q(notify::new_note())
    ));
    out.push_str("}\n");
    print!("{}", out.replace(",\n  ]", "\n  ]"));
}
