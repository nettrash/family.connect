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

    // --- plural categories ---------------------------------------------------------------------------
    {
        let counts: &[i64] = &[0, 1, 2, 3, 4, 5, 10, 11, 12, 13, 14, 15, 19, 20, 21, 22, 24, 25, 100, 101,
            102, 104, 105, 111, 112, 114, 121, 122, 125, 1000, 1001, 1011, -1, -2, -5, -11, -21, i64::MIN];
        let mut rows = Vec::new();
        for tag in ["en", "de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans"] {
            let lang = fc_text::i18n::Lang::for_tag(tag).expect("a catalogue language");
            for count in counts {
                rows.push(serde_json::json!({"lang": tag, "count": count, "category": lang.plural_category(*count)}));
            }
        }
        out.push_str(&format!("  \"plural_categories\": [\n{}\n  ],\n",
            rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
    }

    // --- a profile picture's square --------------------------------------------------------------------
    {
        let sizes: &[(u32, u32)] = &[(4000, 3000), (300, 401), (512, 512), (513, 100), (0, 10), (10, 0),
            (1, 1), (3024, 4032), (1000, 1001), (7, 5), (2, 1000), (1025, 1024)];
        let rows: Vec<serde_json::Value> = sizes.iter().map(|(w, h)| serde_json::json!({
            "width": w, "height": h,
            "square": fc_text::avatar::square(*w, *h).map(|s| serde_json::json!({"x": s.x, "y": s.y, "side": s.side, "edge": s.edge}))
        })).collect();
        out.push_str(&format!("  \"avatar_square\": [\n{}\n  ],\n",
            rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        out.push_str(&format!("  \"avatar_budget\": {},\n", serde_json::json!({
            "edge": fc_text::avatar::EDGE, "qualities": fc_text::avatar::QUALITIES, "max_bytes": fc_text::avatar::MAX_BYTES})));
    }

    // --- the owner's house rules ----------------------------------------------------------------------
    {
        let caps: &[(Option<i64>, i64, i64)] = &[(None, 3, 10), (Some(4), 3, 10), (Some(3), 3, 10), (Some(2), 3, 10),
            (Some(40), 3, 10), (Some(40), 10, 10), (Some(40), 12, 10), (Some(1), 0, 50), (None, 0, 0), (Some(5), 4, 5)];
        let cap_rows: Vec<serde_json::Value> = caps.iter().map(|(cap, members, ceiling)| {
            let state = match fc_text::account::cap_state(*cap, *members, *ceiling) {
                fc_text::account::CapState::OpenToCeiling { ceiling } => serde_json::json!({"kind": "open", "ceiling": ceiling}),
                fc_text::account::CapState::Frozen { members } => serde_json::json!({"kind": "frozen", "members": members}),
                fc_text::account::CapState::Room { members, seats } => serde_json::json!({"kind": "room", "members": members, "seats": seats}),
            };
            serde_json::json!({"cap": cap, "members": members, "ceiling": ceiling, "state": state})
        }).collect();
        let clamps: &[(i64, i64)] = &[(0, 10), (1, 10), (5, 10), (10, 10), (11, 10), (-3, 10), (5, 0), (0, 0), (7, 1)];
        let clamp_rows: Vec<serde_json::Value> = clamps.iter().map(|(value, ceiling)| serde_json::json!({
            "value": value, "ceiling": ceiling,
            "clamp": fc_text::account::clamp_cap(*value, *ceiling), "seed": fc_text::account::seed_cap(*value, *ceiling)})).collect();
        let language_rows: Vec<serde_json::Value> = fc_text::account::LANGUAGES.iter()
            .map(|(tag, name)| serde_json::json!({"tag": tag, "name": name})).collect();
        for (name, rows) in [("house_caps", cap_rows), ("house_clamps", clamp_rows), ("house_languages", language_rows)] {
            out.push_str(&format!("  \"{}\": [\n{}\n  ],\n", name,
                rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        }
        out.push_str(&format!("  \"house_pictures_per_question\": {},\n", fc_text::assistant_pictures::MAX_PER_QUESTION));
    }

    // --- member mentions ------------------------------------------------------------------------------
    {
        use fc_text::mentions;
        let ch = |c: u32| char::from_u32(c).unwrap().to_string();
        let family = [0x1F468u32, 0x200D, 0x1F469, 0x200D, 0x1F467].iter().map(|c| ch(*c)).collect::<String>();
        let range_cases: Vec<(String, String)> = vec![
            ("@Anna hi".into(), "Anna".into()), ("hi @Anna".into(), "Anna".into()), ("@Annabel".into(), "Anna".into()),
            ("mail@Anna".into(), "Anna".into()), ("@Anna_x".into(), "Anna".into()), ("@Anna.".into(), "Anna".into()),
            ("@Anna @Anna".into(), "Anna".into()), ("@@Anna".into(), "Anna".into()), ("@Anna Lee is here".into(), "Anna Lee".into()),
            ("@Анна привет".into(), "Анна".into()), (format!("@Anna{} x", ch(0x301)), "Anna".into()),
            (format!("{}@Anna", ch(0x600)), "Anna".into()), (format!("@{} hi", family), family.clone()),
            ("@Anna".into(), String::new()), (String::new(), "Anna".into()), ("@anna".into(), "Anna".into()),
            ("@Anna1".into(), "Anna".into()), ("é@Anna".into(), "Anna".into()), ("@Anna,@Anna".into(), "Anna".into()),
        ];
        let range_rows: Vec<serde_json::Value> = range_cases.iter().map(|(body, name)| serde_json::json!({
            "body": body, "name": name,
            "ranges": mentions::ranges(body, name).iter().map(|range| [range.start, range.end]).collect::<Vec<_>>()
        })).collect();
        let roster_names: Vec<(i64, String)> = vec![(1, "Anna".into()), (2, "Anna Lee".into()), (3, "Bob".into()),
            (4, "Anna".into()), (5, "Анна".into()), (6, "Bo".into())];
        let roster: Vec<mentions::Member> = roster_names.iter()
            .map(|(id, name)| mentions::Member { user_id: *id, name: name.as_str() }).collect();
        let bodies = ["@Anna Lee and @Bob", "@Anna", "@Bob @Anna", "no names", "@Anna Lee", "@Bob@Anna",
            "@Анна и @Bo", "@Bo @Bob", "@Anna @Anna Lee", "@Anna Lee @Anna Lee"];
        let resolve_rows: Vec<serde_json::Value> = bodies.iter().map(|body| serde_json::json!({
            "body": body, "ids": mentions::resolve(body, &roster).iter().map(|member| member.user_id).collect::<Vec<_>>()
        })).collect();
        let roster_rows: Vec<serde_json::Value> = roster_names.iter()
            .map(|(id, name)| serde_json::json!({"id": id, "name": name})).collect();
        let token_cases: Vec<(&str, Vec<(i64, &str)>)> = vec![
            ("@Anna Lee met @Anna and @Bob", vec![(1, "Anna"), (2, "Anna Lee"), (3, "Bob")]),
            ("@Bo @Bob @Bo", vec![(6, "Bo"), (3, "Bob")]),
            ("nothing here", vec![(1, "Anna")]),
        ];
        let token_rows: Vec<serde_json::Value> = token_cases.iter().map(|(text, named)| {
            let members: Vec<mentions::Member> = named.iter().map(|(id, name)| mentions::Member { user_id: *id, name }).collect();
            serde_json::json!({
                "text": text,
                "mentions": named.iter().map(|(id, name)| serde_json::json!({"id": id, "name": name})).collect::<Vec<_>>(),
                "tokens": mentions::tokens(text, &members).iter().map(|token| [token.range.start as i64, token.range.end as i64, token.member.user_id]).collect::<Vec<_>>()
            })
        }).collect();
        for (name, rows) in [("mention_ranges", range_rows), ("mention_resolve", resolve_rows),
                             ("mention_roster", roster_rows), ("mention_tokens", token_rows)] {
            out.push_str(&format!("  \"{}\": [\n{}\n  ],\n", name,
                rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        }
    }

    // --- how a picked file is prepared ---------------------------------------------------------------
    {
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let ch = |c: u32| char::from_u32(c).unwrap().to_string();
        let types: Vec<(&str, &str)> = vec![
            ("", "photo.JPG"), ("application/octet-stream", "clip.mov"), ("image/HEIC; foo=bar", "x.bin"),
            ("", ".hidden"), ("", "noext"), ("", "archive.tar.gz"), ("Video/MP4", "a.mp4"), ("", "a."),
            ("", "song.Mp3"), ("  text/Plain ; charset=utf-8", "notes"), ("", "Report.PDF"), ("", "deck.KEY"), ("text/plain", "voice.oga"),
        ];
        let type_rows: Vec<serde_json::Value> = types.iter().map(|(mime, name)| serde_json::json!({
            "mime": mime, "name": name, "essence": media::essence(mime), "extension": media::extension(name),
            "mime_for": media::mime_for(name), "declared": media::declared_type(mime, name),
            "audio": media::audio_mime(&media::essence(mime), name)})).collect();
        let mp4: Vec<u8> = [0u8, 0, 0, 0x18].iter().copied().chain(*b"ftypmp42").chain([0u8; 4]).collect();
        let jpeg = vec![0xFFu8, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1];
        let png = vec![0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        let id3 = b"ID3\x04\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        let sync = vec![0xFFu8, 0xFB, 0x90, 0x64, 0, 0, 0, 0, 0, 0, 0, 0];
        let wav: Vec<u8> = b"RIFF".iter().copied().chain([0x24u8, 0, 0, 0]).chain(*b"WAVE").collect();
        let ogg: Vec<u8> = b"OggS".iter().copied().chain([0u8; 8]).collect();
        let junk = vec![0x12u8, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 1, 2, 3, 4];
        let short = vec![0xFFu8];
        let heads: Vec<(&str, &Vec<u8>)> = vec![("mp4", &mp4), ("jpeg", &jpeg), ("png", &png), ("id3", &id3),
            ("sync", &sync), ("wav", &wav), ("ogg", &ogg), ("junk", &junk), ("short", &short)];
        let picks: Vec<(&str, &str)> = vec![
            ("video/mp4", "a.mp4"), ("video/quicktime", "a.mov"), ("", "a.mkv"), ("audio/aac", "a.aac"),
            ("audio/x-m4a", "a.m4a"), ("", "voice.ogg"), ("image/gif", "a.gif"), ("image/png", "a.png"),
            ("", "IMG.HEIC"), ("application/ogg", "x"), ("", "x.wav"), ("audio/mpeg", "x.mp3"),
            ("", "scan.tiff"), ("image/webp", "a.webp"), ("", "doc.pdf"), ("audio/flac", "a.flac"), ("video/ogg", "voice.ogg"), ("application/x-foo", "a.m4a"), ("text/plain", "take.oga"),
        ];
        let mut route_rows = Vec::new();
        for (mime, name) in &picks {
            for (label, head) in &heads {
                let said = match media::route(mime, name, head) {
                    media::Route::Photo => "photo".to_string(),
                    media::Route::Video => "video".to_string(),
                    media::Route::Audio(audio) => format!("audio:{audio}"),
                    media::Route::File => "file".to_string(),
                };
                route_rows.push(serde_json::json!({"mime": mime, "name": name, "head": label, "route": said}));
            }
        }
        let mimes = ["image/jpeg", "image/png", "image/heic", "image/heif", "video/mp4", "video/quicktime",
            "audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav", "audio/ogg", "image/gif", "application/pdf"];
        let mut magic_rows = Vec::new();
        for mime in mimes {
            for (label, head) in &heads {
                magic_rows.push(serde_json::json!({"mime": mime, "head": label, "matches": media::matches_magic(mime, head)}));
            }
        }
        let head_rows: Vec<serde_json::Value> = heads.iter().map(|(label, head)| serde_json::json!({"label": label, "hex": hex(head)})).collect();
        let family = [0x1F468u32, 0x200D, 0x1F469, 0x200D, 0x1F467].iter().map(|c| ch(*c)).collect::<String>();
        let names: Vec<String> = vec![
            "report.pdf".into(), "  spaced  ".into(), "a/b:c.txt".into(),
            format!("invoice{}fdp.exe", ch(0x202E)), format!("tab{}here.txt", ch(9)), String::new(), "   ".into(),
            format!("caf{}.txt", ch(0x301).replace("", "")).replacen("caf", "cafe", 1),
            format!("{}.pdf", "a".repeat(300)), "x".repeat(260), format!("{}.docx", ch(0x431).repeat(250)),
            ".hiddenfile".into(), format!("{}.png", family.repeat(60)), format!("a.{}", "b".repeat(260)),
            format!("zero{}width.txt", ch(0x200B)), format!("soft{}hyphen.txt", ch(0xAD)),
            format!("{}.tar.gz", "q".repeat(254)), format!("{}.{}", "s".repeat(10), "e".repeat(254)),
        ];
        let name_rows: Vec<serde_json::Value> = names.iter().map(|raw| serde_json::json!({"raw": raw, "clean": media::sanitized_name(raw)})).collect();
        let fits: Vec<(u32, u32, u32)> = vec![(4032, 3024, 2048), (3024, 4032, 600), (100, 50, 2048), (0, 0, 600),
            (2048, 1, 600), (3, 2, 2), (2049, 2049, 2048), (1000, 333, 600), (0, 5000, 600), (1, 3, 2), (8, 5, 4)];
        let fit_rows: Vec<serde_json::Value> = fits.iter().map(|(w, h, e)| { let f = media::fit_within(*w, *h, *e);
            serde_json::json!({"width": w, "height": h, "edge": e, "fit": [f.0, f.1]}) }).collect();
        for (name, rows) in [("prep_types", type_rows), ("prep_heads", head_rows), ("prep_route", route_rows),
                             ("prep_magic", magic_rows), ("prep_names", name_rows), ("prep_fit", fit_rows)] {
            out.push_str(&format!("  \"{}\": [\n{}\n  ],\n", name,
                rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        }
    }

    // --- how an attachment is measured and labelled ----------------------------------------------
    {
        let mut rows = Vec::new();
        for bytes in [0u64, 1, 2, 999, 1_000, 1_499, 1_500, 999_499, 999_500, 1_000_000, 1_049_999,
                      1_050_000, 1_250_000, 999_949_999, 999_950_000, 1_000_000_000, 1_234_567_890,
                      10_000_000_000, 99_999_999_999] {
            rows.push(serde_json::json!({"bytes": bytes, "said": media::display_size(bytes)}));
        }
        let sizes: Vec<(Option<i64>, Option<i64>)> = vec![
            (None, None), (Some(4032), Some(3024)), (Some(3024), Some(4032)), (Some(1920), Some(1080)),
            (Some(1080), Some(1920)), (Some(1000), Some(1000)), (Some(0), Some(100)), (Some(100), None),
            (Some(4000), Some(1000)), (Some(1000), Some(4000)), (Some(-5), Some(10)), (Some(5), Some(4)),
        ];
        let mut shapes = Vec::new();
        for (w, h) in &sizes {
            let tile = media::tile_size(*w, *h);
            let card = media::card_size(*w, *h, 300.0);
            // 250.625 over 5:4 is exactly 200.5: the one width where rounding half away from zero and
            // rounding half to even disagree.
            let odd = media::card_size(*w, *h, 250.625);
            shapes.push(serde_json::json!({"width": w, "height": h, "aspect": media::aspect_ratio(*w, *h),
                "tile": [tile.0, tile.1], "card": [card.0, card.1], "card_odd": [odd.0, odd.1]}));
        }
        let kinds: Vec<serde_json::Value> = ["photo", "video", "audio", "file", "location", "hologram"]
            .iter().map(|k| serde_json::json!({"kind": k, "is_media": media::is_media(k)})).collect();
        let places: Vec<(f64, f64, Option<f64>, Option<&str>)> = vec![
            (55.7558, 37.6173, Some(12.0), Some("Home")),
            (-33.868820, 151.209296, None, None),
            (51.5, -0.12, Some(4.5), Some("  ")),
            (0.000005, -0.000005, Some(0.49), Some("Gran's house, 2nd floor")),
            (40.4406248, -3.7153898, Some(f64::NAN), Some("Plaza & Mayor")),
            (89.9999999, -179.9999999, Some(1500.5), Some("  Caf\u{e9}  ")),
            (48.8584, 2.2945, Some(3.0), Some("Tower~Top_2.0-west")),
        ];
        let locations: Vec<serde_json::Value> = places.iter().map(|(lat, lon, acc, name)| serde_json::json!({
            "latitude": lat, "longitude": lon,
            "accuracy_m": acc.filter(|a| a.is_finite()),
            "accuracy_nan": acc.map(|a| a.is_nan()).unwrap_or(false),
            "name": name,
            "line": media::location_line(*lat, *lon, *acc),
            "maps": media::maps_url(*lat, *lon, *name)})).collect();
        out.push_str(&format!("  \"display_size\": [\n{}\n  ],\n",
            rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        out.push_str(&format!("  \"shapes\": [\n{}\n  ],\n",
            shapes.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        out.push_str(&format!("  \"is_media\": [\n{}\n  ],\n",
            kinds.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        out.push_str(&format!("  \"locations\": [\n{}\n  ],\n",
            locations.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
    }

    // --- reactions and the reply excerpt ------------------------------------------------------
    // The chips under a bubble, "See who reacted", the optimistic toggle, the capsule and the
    // 120-scalar cut. Emoji are built from code points so this file stays plain ASCII to the eye.
    {
        use fc_text::{emoji, excerpt, reactions as r};
        use std::collections::{HashMap, HashSet};
        let quick = emoji::QUICK_REACTIONS;
        let (heart, up, joy) = (quick[0].to_string(), quick[1].to_string(), quick[3].to_string());
        let ring = char::from_u32(0xC5).unwrap().to_string();
        let ring_combining = format!("A{}", char::from_u32(0x30A).unwrap());
        let robot = char::from_u32(0x1F916).unwrap().to_string();
        let lists: Vec<Vec<(i64, String)>> = vec![
            vec![],
            vec![(7, heart.clone())],
            vec![(9, up.clone()), (7, heart.clone()), (11, up.clone())],
            vec![(9, heart.clone()), (12, heart.clone()), (11, joy.clone()), (7, joy.clone())],
            vec![(12, ring.clone()), (9, ring_combining.clone())],
            vec![(11, heart.clone()), (12, heart.clone())],
        ];
        let names: HashMap<i64, String> =
            [(9, "Anna".to_string()), (11, "Bob".to_string())].into_iter().collect();
        let blocked_sets: Vec<Vec<i64>> = vec![vec![], vec![11], vec![11, 12]];
        let as_json = |list: &[r::Reaction]| {
            serde_json::Value::Array(
                list.iter()
                    .map(|x| serde_json::json!({"user_id": x.user_id, "emoji": x.emoji}))
                    .collect(),
            )
        };
        let (mut chip_rows, mut detail_rows, mut toggle_rows) = (Vec::new(), Vec::new(), Vec::new());
        for list in &lists {
            let reactions: Vec<r::Reaction> =
                list.iter().map(|(user, emoji)| r::Reaction::new(*user, emoji)).collect();
            for me in [7i64, 9] {
                let chips: Vec<serde_json::Value> = r::reaction_chips(&reactions, me)
                    .into_iter()
                    .map(|c| serde_json::json!({"emoji": c.emoji, "count": c.count, "includes_me": c.includes_me}))
                    .collect();
                chip_rows.push(serde_json::json!({"reactions": as_json(&reactions), "me": me, "chips": chips}));
                for blocked in &blocked_sets {
                    let set: HashSet<i64> = blocked.iter().copied().collect();
                    let details: Vec<serde_json::Value> = r::reaction_details(&reactions, &names, me, &set)
                        .into_iter()
                        .map(|d| serde_json::json!({"emoji": d.emoji, "names": d.names, "lead_user_id": d.lead_user_id}))
                        .collect();
                    detail_rows.push(serde_json::json!({
                        "reactions": as_json(&reactions), "me": me, "blocked": blocked, "details": details}));
                }
                for choice in [heart.clone(), up.clone(), joy.clone(), robot.clone()] {
                    let toggled = r::toggle_reaction(&reactions, me, &choice);
                    toggle_rows.push(serde_json::json!({
                        "reactions": as_json(&reactions), "me": me, "emoji": choice,
                        "removing": toggled.removing, "after": as_json(&toggled.reactions)}));
                }
            }
        }
        let capsule_rows: Vec<serde_json::Value> = [None, Some(heart.clone()), Some(robot.clone())]
            .iter()
            .map(|mine| serde_json::json!({"mine": mine, "emojis": emoji::capsule_emojis(mine.as_deref())}))
            .collect();
        let family: String = [0x1F468u32, 0x200D, 0x1F469, 0x200D, 0x1F467, 0x200D, 0x1F466]
            .iter()
            .map(|c| char::from_u32(*c).unwrap())
            .collect();
        let cyrillic = char::from_u32(0x44F).unwrap().to_string();
        let bodies = vec![
            String::new(),
            "short".to_string(),
            "a".repeat(120),
            "a".repeat(121),
            family.repeat(20),
            cyrillic.repeat(130),
            format!("{}{}", "b".repeat(119), family),
        ];
        let excerpt_rows: Vec<serde_json::Value> = bodies
            .iter()
            .map(|body| serde_json::json!({"body": body, "excerpt": excerpt::excerpt(body)}))
            .collect();
        for (name, rows) in [
            ("reaction_chips", chip_rows),
            ("reaction_details", detail_rows),
            ("reaction_toggle", toggle_rows),
            ("capsule", capsule_rows),
            ("excerpt", excerpt_rows),
        ] {
            out.push_str(&format!("  \"{}\": [\n", name));
            for row in rows {
                out.push_str(&format!("    {},\n", row));
            }
            out.push_str("  ],\n");
        }
    }

    out.push_str(&format!(
        "  \"bodies\": {{\"new_message\": {}, \"new_note\": {}}}\n",
        q(notify::new_message()),
        q(notify::new_note())
    ));
    out.push_str("}\n");
    print!("{}", out.replace(",\n  ]", "\n  ]"));
}
