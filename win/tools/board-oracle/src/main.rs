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

fn q(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

fn main() {
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
