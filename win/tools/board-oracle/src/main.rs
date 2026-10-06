//! The board's shared arithmetic, evaluated by the ORIGINAL (`fc_text::board`, which the web
//! client runs) and printed as JSON, so the Windows port can be pinned to the same numbers rather
//! than to somebody's reading of them.
//!
//! Four implementations of one rule need an oracle, not four readings: this portfolio has been
//! bitten by a byte-versus-character split that panicked on Cyrillic, and by four ports that
//! agreed with each other and were all wrong about `pow(10, n)`.
//!
//! Output goes to `win/tests/FamilyConnect.Core.Tests/Fixtures/board-vectors.json` — see
//! Cargo.toml for the one command. `media-plan` is the exception to "the Windows port": its file
//! is copied to the iOS and Android test resources too, because all three ports implement it.
//! `record` (issue #79) is the second such exception: the composer's slot, the video button, the
//! voice recording's reducer and the round video's arithmetic, which the Apple and Android ports
//! implement whole and the Windows port in part (it has no reducer). `waveform` (issue #79) is the third: a voice note's 48
//! levels — computed from metered peaks, parsed off the wire, drawn as bars — which every port
//! implements whole.
use fc_text::board as b;
use fc_text::{calendar, call_record, media, notify};

fn q(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

fn main() {
    // No argument: the board vectors, which is the command the docs give. `chat` prints the
    // chat-line vectors instead — the same idea for the words a chat row is drawn with.
    // `markdown <corpus.json>` prints what fc_text::markdown and fc_text::links make of every body in the corpus;
    // `unicode` prints the Rust standard library's own character properties, which those two modules decide by.
    // `media-plan` prints what fc_text::media_plan decides for a picked video or sound file — the one file of
    // vectors the Apple, Android and Windows ports are ALL held to, so it is copied beside each port's tests.
    // `record` prints fc_text::record — the Send slot, the video button and the voice recording (issue #79) — copied likewise.
    if std::env::args().nth(1).as_deref() == Some("media-plan") {
        media_plan_vectors();
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("record") {
        record_vectors();
        return;
    }
    // `waveform` prints fc_text::waveform — a voice note's shape (issue #79) — copied likewise.
    if std::env::args().nth(1).as_deref() == Some("waveform") {
        waveform_vectors();
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("markdown") {
        markdown_vectors(std::env::args().nth(2).expect("the corpus path"));
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("unicode") {
        unicode_tables();
        return;
    }
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

    // --- the composer's @ strip ----------------------------------------------------------------------
    {
        use fc_text::mentions;
        let ch = |c: u32| char::from_u32(c).unwrap().to_string();
        let drafts: Vec<String> = vec![
            "@".into(), "hi @An".into(), "mail@An".into(), format!("@An{}x", ch(10)), "no at".into(),
            "@Anna @Bo".into(), format!("a{}@x", ch(0x301)), "x_@y".into(), format!("@x{}{}y", ch(13), ch(10)),
            format!("@{} x", ch(0x301)), "(@Ann".into(), "Ж@Ann".into(), "9@x".into(), "@@".into(), String::new(),
        ];
        let query_rows: Vec<serde_json::Value> = drafts.iter()
            .map(|draft| serde_json::json!({"draft": draft, "query": mentions::query(draft)})).collect();
        let names: Vec<(i64, String)> = vec![(1, "Anna".into()), (2, "anna lee".into()), (3, format!("Zo{}", ch(0xEB))),
            (4, format!("Zoe{}", ch(0x308))), (5, format!("{}lker", ch(0x130))), (6, "Σοφία".into()), (7, "Bob".into()),
            (8, "ẞtraße".into())];
        let roster: Vec<mentions::Member> = names.iter().map(|(id, name)| mentions::Member { user_id: *id, name }).collect();
        let cases: Vec<(String, Vec<i64>)> = vec![(String::new(), vec![]), ("an".into(), vec![]), ("AN".into(), vec![]),
            ("zoe".into(), vec![]), ("zo".into(), vec![]), ("i".into(), vec![]), (format!("i{}", ch(0x307)), vec![]),
            ("σο".into(), vec![]), ("ΣΟ".into(), vec![]), ("b".into(), vec![]), ("x".into(), vec![]),
            ("anna l".into(), vec![]), ("a".into(), vec![1, 7]), ("ß".into(), vec![]), (String::new(), vec![2, 3, 4, 5, 6])];
        let candidate_rows: Vec<serde_json::Value> = cases.iter().map(|(query, excluding)| serde_json::json!({
            "query": query, "excluding": excluding,
            "ids": mentions::candidates(&roster, query, excluding).iter().map(|member| member.user_id).collect::<Vec<_>>()
        })).collect();
        let roster_rows: Vec<serde_json::Value> = names.iter().map(|(id, name)| serde_json::json!({"id": id, "name": name})).collect();
        let accepts: Vec<(String, String)> = vec![("hi @An".into(), "Anna".into()), ("no at".into(), "Bob".into()),
            ("@Anna and @Bo".into(), "Bob".into()), (String::new(), format!("Zo{}", ch(0xEB))),
            (format!("@{} x", ch(0x301)), "Anna".into())];
        let accept_rows: Vec<serde_json::Value> = accepts.iter()
            .map(|(draft, name)| serde_json::json!({"draft": draft, "name": name, "accepted": mentions::accept(draft, name)})).collect();
        for (name, rows) in [("strip_query", query_rows), ("strip_candidates", candidate_rows),
                             ("strip_roster", roster_rows), ("strip_accept", accept_rows)] {
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

    // --- @ai and /draw: what a body asks the assistant for ---------------------------------------------
    {
        use fc_text::assistant;
        let ch = |c: u32| char::from_u32(c).unwrap().to_string();
        let bodies: Vec<String> = vec![
            "@ai hello".into(), "@AI".into(), "hello @Ai!".into(), "anna@ai.example".into(), "@aiden".into(), "(@ai)".into(),
            "@ai_".into(), "Я@ai".into(), format!("@ai{}", ch(0x301)), format!("{}@ai", ch(0x600)), "@ai /draw a cat".into(),
            "/draw a cat".into(), "/DRAW  a cat ".into(), "/drawer".into(), "/draw".into(), "/draw ".into(), "hey @ai /draw a cat".into(),
            "  @ai   /draw   dogs  ".into(), format!("/draw{} a cat", ch(0x301)), "@ai/draw a cat".into(),
            format!("/draw{}moon", ch(0x3000)), format!("/draw{}moon", ch(0x200B)), String::new(), "@a".into(), "x @ai".into(),
            "look @ai /draw a cat".into(), "@ai @ai /draw a cat".into(), "/draw,a cat".into(), format!("/draw\t{}", ch(0x85)),
            format!("{}/draw sun", ch(0xA0)), "@ai\n/draw\nsun".into(), "@ai9".into(), "9@ai".into(), "@ai.".into(), "ai".into(),
        ];
        let body_rows: Vec<serde_json::Value> = bodies.iter().map(|body| serde_json::json!({
            "body": body, "mentions": assistant::mentions(body), "prompt": assistant::draw_prompt(body),
            "asks": assistant::asks_for_picture(body),
        })).collect();
        let drafts: Vec<String> = vec![
            String::new(), "hi".into(), "hi ".into(), "hi @ai".into(), "a  ".into(), format!("a {}", ch(0x301)), "hello\n".into(),
            format!("{} ", ch(0x600)), "@AI please".into(),
        ];
        let draft_rows: Vec<serde_json::Value> = drafts.iter()
            .map(|draft| serde_json::json!({"draft": draft, "with": assistant::with_assistant_mention(draft)})).collect();
        // The ai chat's picture button: `/draw ` in front of the words, once (fc_text::assistant::with_draw_token).
        // Its own cases on top: a request already there, `/draw` alone, the trims only Foundation's set takes, a CRLF.
        let mut draw_drafts = drafts.clone();
        draw_drafts.extend([
            "/draw a cat".to_string(), "/draw".into(), "/DRAW  a cat".into(), "  a cat \n".into(), ch(0x200B).to_string(),
            format!("{}cat{}", ch(0x200B), ch(0x3000)), format!("{}cat", ch(0x85)), "@ai /draw a cat".into(), "@ai a cat".into(),
            "кот".into(), "a\r\nb\r\n".into(), format!("/draw{}a", ch(0x301)), format!("{}a", ch(0xFEFF)),
        ]);
        let draw_rows: Vec<serde_json::Value> = draw_drafts.iter()
            .map(|draft| serde_json::json!({"draft": draft, "with": assistant::with_draw_token(draft)})).collect();
        for (name, rows) in [("assistant_bodies", body_rows), ("assistant_mention_drafts", draft_rows), ("draw_token_drafts", draw_rows)] {
            out.push_str(&format!("  \"{}\": [\n{}\n  ],\n", name,
                rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        }
    }

    // --- the assistant and a photograph: what a composer says before the pixels leave --------------------
    {
        use fc_text::assistant_pictures::{self as pictures, Candidate, MentionNotice, Switches};
        let jpeg = |bytes: Option<u64>| Candidate::new("photo", "image/jpeg", bytes);
        let png = Candidate::new("photo", "image/png", Some(10));
        let heic = Candidate::new("photo", "image/heic", Some(10));
        let big = jpeg(Some(5 * 1024 * 1024 + 1));
        let video = Candidate::new("video", "video/mp4", Some(10));
        let as_json = |list: &[Candidate]| list.iter()
            .map(|c| serde_json::json!({"kind": c.kind, "mime": c.mime, "bytes": c.bytes})).collect::<Vec<_>>();
        let stagings: Vec<Vec<Candidate>> = vec![
            vec![], vec![jpeg(Some(10))], vec![video.clone()], vec![heic.clone()], vec![big.clone()], vec![jpeg(None); 5],
            vec![jpeg(Some(10)), video.clone()], vec![png.clone(), heic.clone()], vec![jpeg(Some(10)); 4],
        ];
        let mut private_rows: Vec<serde_json::Value> = Vec::new();
        for staged in &stagings {
            for can_see in [false, true] {
                private_rows.push(serde_json::json!({
                    "staged": as_json(staged), "can_see": can_see, "said": pictures::private_notice(staged, can_see),
                }));
            }
        }
        let drafts = ["@ai look at this", "hello", "@ai /draw a cat", "look @ai"];
        let staged_sets: Vec<Vec<Candidate>> = vec![
            vec![], vec![jpeg(Some(10))], vec![jpeg(Some(10)); 3], vec![heic.clone(), jpeg(Some(10))],
            // Unreadable AND past the budget at once: which sentence wins is a rule.
            vec![heic.clone(), jpeg(Some(10)), jpeg(Some(10)), jpeg(Some(10))],
        ];
        let quoted_sets: Vec<Vec<Candidate>> = vec![vec![], vec![jpeg(None)], vec![jpeg(None); 3], vec![big.clone()]];
        let switch_sets = [
            (true, true, true, false, false), (true, true, true, true, false), (true, true, false, true, false),
            (false, true, true, true, false), (true, false, true, true, false), (true, true, true, true, true),
        ];
        let mut mention_rows: Vec<serde_json::Value> = Vec::new();
        for draft in drafts {
            for staged in &staged_sets {
                for quoted in &quoted_sets {
                    for (see, allows, history, photos, draw) in switch_sets {
                        let switches = Switches { server_can_see: see, family_allows: allows, family_history: history,
                            family_history_photos: photos, server_can_draw: draw };
                        let notice = MentionNotice::of(draft, staged, quoted, switches);
                        mention_rows.push(serde_json::json!({
                            "draft": draft, "staged": as_json(staged), "quoted": as_json(quoted),
                            "switches": [see, allows, history, photos, draw],
                            "said": notice.as_ref().map(|n| n.sentence()),
                            "counts": notice.as_ref().map(|n| [n.shown_on_mention, n.shown_on_quote, n.extra, n.unreadable]),
                            "recent": notice.as_ref().and_then(|n| n.recent_up_to),
                        }));
                    }
                }
            }
        }
        let shown_cases: Vec<(&str, &str, Option<u64>)> = vec![
            ("photo", "image/jpeg", Some(5 * 1024 * 1024)), ("photo", "image/jpeg", Some(5 * 1024 * 1024 + 1)), ("photo", "image/jpeg", None),
            ("photo", "IMAGE/PNG", Some(1)), ("photo", "image/webp", Some(1)), ("file", "image/jpeg", Some(1)), ("video", "video/mp4", Some(1)),
        ];
        let shown_rows: Vec<serde_json::Value> = shown_cases.iter().map(|(kind, mime, bytes)| serde_json::json!({
            "kind": kind, "mime": mime, "bytes": bytes, "shown": pictures::is_shown_to_model(kind, mime, *bytes),
        })).collect();
        let attachment_cases: Vec<(&str, &str, Option<u64>, bool)> = vec![
            ("photo", "image/heic", Some(9_000_000), true), ("photo", "image/heic", Some(9_000_000), false), ("photo", "image/png", None, false),
        ];
        let attachment_rows: Vec<serde_json::Value> = attachment_cases.iter().map(|(kind, mime, size, preview)| {
            let candidate = Candidate::of_attachment(kind, mime, *size, *preview);
            serde_json::json!({"kind": kind, "mime": mime, "size": size, "preview": preview,
                "as": {"kind": candidate.kind, "mime": candidate.mime, "bytes": candidate.bytes}})
        }).collect();
        let offer_rows: Vec<serde_json::Value> = [(true, true, true), (false, true, true), (true, false, true), (true, true, false)].iter()
            .map(|(chat, see, allows)| serde_json::json!({"chat": chat, "see": see, "allows": allows,
                "offers": pictures::offers_picture_attach(*chat, *see, *allows)})).collect();
        for (name, rows) in [("pictures_private", private_rows), ("pictures_mention", mention_rows), ("pictures_shown", shown_rows),
                             ("pictures_attachment", attachment_rows), ("pictures_offer", offer_rows)] {
            out.push_str(&format!("  \"{}\": [\n{}\n  ],\n", name,
                rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        }
    }

    // --- a paste: attach its files, type its words, or nothing ---------------------------------------
    {
        use fc_text::media;
        let ch = |c: u32| char::from_u32(c).unwrap().to_string();
        let v = |list: &[&str]| list.iter().map(|item| item.to_string()).collect::<Vec<String>>();
        let decisions: Vec<(Vec<String>, String)> = vec![
            (v(&[]), "hello".into()), (v(&[]), "   ".into()), (v(&[]), String::new()),
            (v(&["a.png"]), String::new()), (v(&["a.png"]), "a.png".into()), (v(&["a.png", "b.pdf"]), "a.png\nb.pdf\n".into()),
            (v(&["a.png", "b.pdf"]), "a.png\r\nb.pdf".into()), (v(&["a.png"]), "a.png\nsomething else".into()),
            (v(&["a.png"]), "  a.png  ".into()), (v(&["a.png"]), "A.PNG".into()), (v(&["a.png"]), "\n\n a.png \n\n".into()),
            (v(&["a.png"]), "caption words".into()), (v(&[]), format!("{}x{}", ch(0xA0), ch(0x3000))),
            (v(&["a.png"]), format!("a.png{}", ch(0x2028))), (v(&["a.png"]), ch(0x85)), (v(&["a b.png"]), "a b.png".into()),
            (v(&["a.png"]), "a.png\rb.png".into()),
            // Each LINE is trimmed, and only a line feed ends one: a carriage return alone is inside a name.
            (v(&["a.png", "b.pdf"]), "a.png  \n  b.pdf".into()), (v(&["a.png", "b.png"]), "a.png\rb.png".into()),
        ];
        let decision_rows: Vec<serde_json::Value> = decisions.iter().map(|(names, text)| serde_json::json!({
            "names": names, "text": text, "said": format!("{:?}", media::paste_decision(names, text)),
        })).collect();
        let offers: Vec<Vec<String>> = vec![
            v(&["image/png", "image/gif"]), v(&["text/plain", "image/png"]), v(&["text/plain", "text/html"]), v(&["application/zip"]),
            v(&["weird"]), v(&[]), v(&["video/quicktime", "application/pdf"]), v(&["text/uri-list", "application/x-thing"]),
            v(&["IMAGE/PNG"]),
        ];
        let type_rows: Vec<serde_json::Value> = offers.iter()
            .map(|offered| serde_json::json!({"offered": offered, "chosen": media::chosen_paste_type(offered)})).collect();
        let mimes = ["image/gif", "image/webp", "image/heic", "image/heif", "image/png", "image/jpeg", "image/bmp", "image/tiff",
            "video/mp4", "video/quicktime", "audio/mp4", "audio/mpeg", "audio/wav", "application/pdf", "application/zip",
            "image/svg+xml", "", "text/plain", "video/x-matroska", "audio"];
        let name_rows: Vec<serde_json::Value> = mimes.iter()
            .map(|mime| serde_json::json!({"mime": mime, "name": media::pasted_name(mime)})).collect();
        // A recording's elapsed or total time.
        let seconds = [0.0, 0.4, 0.5, 4.4, 4.5, 59.5, 60.0, 222.0, 300.0, 3599.5, 3600.0, 36000.0, -3.0, f64::INFINITY, f64::NAN, 1.5, 2.5];
        let time_rows: Vec<serde_json::Value> = seconds.iter()
            .map(|s| serde_json::json!({"seconds": if s.is_finite() { serde_json::json!(s) } else { serde_json::json!(s.to_string()) },
                "label": media::time_label(*s)})).collect();
        for (name, rows) in [("paste_decision", decision_rows), ("paste_type", type_rows), ("pasted_name", name_rows), ("time_label", time_rows)] {
            out.push_str(&format!("  \"{}\": [\n{}\n  ],\n", name,
                rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        }
    }

    // --- location: the query a shared place is uploaded with (web api.rs upload_query) -----------------
    {
        // The formatting IS the rule here: Rust's `{:.7}` and `round() as i64`, which a port must match on the ties.
        let places: Vec<(f64, f64, Option<f64>)> = vec![
            (55.7558, 37.6173, Some(12.0)), (55.7558, 37.6173, None), (0.00390625, -0.00390625, Some(0.5)),
            (55.00390625, 37.12890625, Some(2.5)), (-33.8688, 151.2093, Some(-3.0)), (90.0, 180.0, Some(0.49)),
            (-90.0, -180.0, Some(1.5)), (-0.0, 0.0, Some(99.5)), (12.3456789, -98.76543215, Some(f64::NAN)),
            (1e-8, -4e-8, Some(f64::INFINITY)), (51.47782, -0.00148, Some(65.0)), (35.6895, 139.69171, Some(1234.5678)),
            (-0.00000005, 0.00000015, Some(-0.4)), (40.7128, -74.006, Some(3_000_000_000.0)), (45.12345675, -45.12345665, Some(100.5)),
        ];
        let rows: Vec<serde_json::Value> = places.iter().map(|(latitude, longitude, accuracy)| {
            let mut query = vec!["kind=location".to_string(), format!("latitude={latitude:.7}"), format!("longitude={longitude:.7}")];
            if let Some(accuracy) = accuracy.filter(|accuracy| accuracy.is_finite()) {
                query.push(format!("accuracy_m={}", accuracy.round().max(0.0) as i64));
            }
            serde_json::json!({"latitude": latitude, "longitude": longitude,
                "accuracy_m": match accuracy {
                    None => serde_json::Value::Null,
                    Some(a) if a.is_finite() => serde_json::json!(a),
                    Some(a) => serde_json::json!(a.to_string()),
                },
                "query": query.join("&")})
        }).collect();
        out.push_str(&format!("  \"location_query\": [\n{}\n  ],\n",
            rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
    }

    // --- emoji: an emoji-only message's size, and the "More reactions…" catalogue ----------------------
    {
        use fc_text::emoji;
        let ch = |c: u32| char::from_u32(c).unwrap().to_string();
        let s = |scalars: &[u32]| scalars.iter().map(|c| ch(*c)).collect::<String>();
        let texts: Vec<String> = vec![
            s(&[0x1F600]), s(&[0x1F600, 0x1F600]), s(&[0x1F600, 0x20, 0x1F600, 0x20, 0x1F600]), s(&[0x1F600; 4]), s(&[0x1F600; 5]),
            format!("hi {}", ch(0x1F600)), String::new(), "   ".into(), s(&[0x2764, 0xFE0F]), s(&[0x2764, 0xFE0E]), s(&[0x2764]),
            s(&[0x1F44D, 0x1F3FD]), s(&[0x1F468, 0x200D, 0x1F469, 0x200D, 0x1F467, 0x200D, 0x1F466]), s(&[0x1F1FA, 0x1F1E6]),
            s(&[0x1F1FA]), s(&[0x1F1FA, 0x1F1E6, 0x1F1FA]), s(&[0x35, 0xFE0F, 0x20E3]), "5".into(), s(&[0x23, 0x20E3]), s(&[0x2A, 0xFE0F]),
            s(&[0x1F3F4, 0xE0067, 0xE0062, 0xE0073, 0xE0063, 0xE0074, 0xE007F]), s(&[0xA9]), s(&[0x2122, 0xFE0F]), s(&[0x2602]),
            s(&[0x1F600, 0x200D]), format!("{}a", s(&[0x1F600, 0x200D])), s(&[0x85, 0x1F600]), s(&[0x1C, 0x1F600]), s(&[0x1F90C]),
            s(&[0x1FAE0]), s(&[0x1FB00]), s(&[0x3030, 0x303D]), s(&[0x2B50, 0x2B55, 0x2B1B, 0x2B1C]), s(&[0x3000, 0x1F600, 0x2028]),
            s(&[0x1F600, 0xFE0E, 0x1F600]), s(&[0x200D, 0x1F600]), s(&[0x1F3FB]), s(&[0xE0067]), s(&[0x1F004, 0x1F0CF]),
            // A lone regional indicator is one emoji, and what follows it is its own.
            s(&[0x1F1FA, 0x1F600]), s(&[0x1F1FA, 0x20, 0x1F1E6]),
        ];
        let only_rows: Vec<serde_json::Value> = texts.iter().map(|text| serde_json::json!({
            "text": text,
            "count": emoji::emoji_only_count(text),
            "size": emoji::display_font_size(text),
            "at13": emoji::display_font_size_for_body(text, 13.0),
        })).collect();
        let catalog_rows: Vec<serde_json::Value> = emoji::EMOJI_CATALOG.iter()
            .map(|category| serde_json::json!({"name": category.name, "emoji": category.emoji})).collect();
        for (name, rows) in [("emoji_only", only_rows), ("emoji_catalog", catalog_rows)] {
            out.push_str(&format!("  \"{}\": [\n{}\n  ],\n", name,
                rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        }
    }

    // --- a profile circle's initials ----------------------------------------------------------------
    // `fc_text::avatar::initials` uppercases with `str::to_uppercase`, the FULL mapping (ß → SS, ﬁ → FI, a Greek
    // letter with a subscript iota → two capitals), so every scalar Rust uppercases is a vector.
    {
        use fc_text::avatar;
        let ch = |c: u32| char::from_u32(c).unwrap().to_string();
        let family = [0x1F468u32, 0x200D, 0x1F469, 0x200D, 0x1F467].iter().map(|c| ch(*c)).collect::<String>();
        let titles: Vec<String> = vec![
            "Anna Smith".into(), "anna maria smith".into(), "Anna".into(), "  Anna   Smith  ".into(), String::new(),
            "   ".into(), format!("{} Smiths", family), format!("e{}mile zola", ch(0x301)),
            format!("{}en stra{}e", ch(0xDF), ch(0xDF)), format!("{} fo", ch(0xFB01)), format!("Anna{}Smith", ch(0xA0)),
            "Anna\tSmith".into(), format!("{}lker {}zmir", ch(0x130), ch(0x131)), format!("{}emal {}uro", ch(0x1C6), ch(0x1C5)),
            format!("{}{} {}", ch(0x3B6), ch(0x3C9), ch(0x1F50)), "александр пушкин".into(), format!("{} {}", ch(0x674E), ch(0x5C0F)),
            format!("{}x", ch(0x1FB3)), "a".into(), format!("{} {}", ch(0x1F3), ch(0x587)),
        ];
        let initial_rows: Vec<serde_json::Value> = titles.iter()
            .map(|title| serde_json::json!({"title": title, "said": avatar::initials(title)})).collect();
        let upper_rows: Vec<serde_json::Value> = (0u32..0x110000)
            .filter_map(char::from_u32)
            .filter(|c| c.to_uppercase().collect::<String>() != c.to_string())
            .map(|c| serde_json::json!({"c": c as u32, "upper": c.to_uppercase().collect::<String>()}))
            .collect();
        for (name, rows) in [("avatar_initials", initial_rows), ("avatar_upper", upper_rows)] {
            out.push_str(&format!("  \"{}\": [\n{}\n  ],\n", name,
                rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
        }
    }

    // --- a poll's options, checked the server's way -------------------------------------------------
    // Not fc_text: the rule lives in server/src/handlers_chat.rs (`validate_poll_options`) and the web
    // composer's `validate`, and what both lean on is Rust's own std — `str::trim`, `chars().count()`
    // and `str::to_lowercase`. Those three are what these vectors pin.
    {
        let ch = |c: u32| char::from_u32(c).unwrap().to_string();
        fn sanitized(options: &[String]) -> Option<Vec<String>> {
            let trimmed: Vec<String> = options
                .iter()
                .map(|option| option.trim().to_string())
                .filter(|option| !option.is_empty())
                .collect();
            if trimmed.len() < 2 || trimmed.len() > 10 {
                return None;
            }
            if trimmed.iter().any(|option| option.chars().count() > 100) {
                return None;
            }
            let mut seen = std::collections::HashSet::new();
            if !trimmed.iter().all(|option| seen.insert(option.to_lowercase())) {
                return None;
            }
            Some(trimmed)
        }
        let trims: Vec<String> = vec![
            " a ".into(), "\t\n a\r".into(), format!("{}a{}", ch(0xA0), ch(0x3000)), format!("{}a{}", ch(0x2028), ch(0x2029)),
            format!("{}a", ch(0x85)), format!("{}a{}", ch(0x180E), ch(0x180E)), format!("{}a{}", ch(0x200B), ch(0x200B)),
            format!("{}a", ch(0xFEFF)), format!("{}a{}", ch(0x1680), ch(0x202F)), format!("{}a{}", ch(0x205F), ch(0x2000)),
            format!("{}a{}", ch(0x1C), ch(0x1F)), String::new(), "   ".into(),
        ];
        let trim_rows: Vec<serde_json::Value> = trims.iter()
            .map(|text| serde_json::json!({"text": text, "trimmed": text.trim()})).collect();
        let sigma = ch(0x3A3);
        let lowers: Vec<String> = vec![
            "ΟΔΟΣ".into(), "ΣΑΣ".into(), sigma.clone(), format!("Α{}.", sigma), format!("Α'{}", sigma),
            format!("Α{}'Α", sigma), format!("Α{}{}", sigma, ch(0x301)), format!("{}{}", ch(0x301), sigma),
            format!("{}{}", ch(0x24B6), sigma), format!("{}{}", ch(0x2160), sigma), format!("{}{}", ch(0xAA), sigma),
            format!("A{}{}B", sigma, ch(0x200D)), format!("{}stanbul", ch(0x130)), ch(0x1E9E), ch(0x1C4), ch(0x1C5),
            format!("Α{} Α", sigma), format!("Α{}{}", sigma, ch(0xAD)), format!("1{}", sigma), format!("Α{}1", sigma),
            format!("{}{}Α", sigma, ch(0x301)), format!("{}{}", ch(0x2B0), sigma), format!("{}{}", ch(0x1F130), sigma),
            format!("{}{}", ch(0x5D0), sigma), "ПРИВЕТ".into(), format!("{}{}", ch(0x10A0), ch(0x1C90)),
            // One of each thing Case_Ignorable skips, and each kind of cased letter, BEFORE the sigma —
            // where skipping it or not changes the answer.
            format!("Α.{}", sigma), format!("Α{}{}", ch(0x301), sigma), format!("{}{}", ch(0x1C5), sigma),
            format!("Α{}{}", ch(0xAD), sigma), format!("Α{}{}", ch(0x2B0), sigma), format!("Α{}{}", ch(0x20DD), sigma),
            format!("Α^{}", sigma), format!("Α:{}", sigma), format!("Α{}{}", ch(0x2019), sigma), format!("α{}", sigma),
            // What is NEAREST the sigma decides, not what comes first in the string.
            format!("1Α{}", sigma), format!("Α1{}", sigma),
        ];
        // And every scalar, not only the ones picked out above: each one's own full mapping, and each one Final_Sigma could
        // skip or count, both before a capital sigma and after it — so no casing table of the machine running the port
        // can stand in for Rust's (CI's Linux ICU and Windows' NLS lacked the letters Unicode 16 added).
        let mut lowers = lowers;
        for c in (0u32..0x110000).filter_map(char::from_u32) {
            let lone = c.to_string();
            if lone.to_lowercase() != lone {
                lowers.push(lone);
            }
            let skipped = format!("A{c}Σ").to_lowercase().ends_with('ς') && !format!("{c}Σ").to_lowercase().ends_with('ς');
            let cased = format!("{c}Σ").to_lowercase().ends_with('ς');
            if skipped || cased {
                lowers.push(format!("A{c}Σ"));
                lowers.push(format!("AΣ{c}"));
            }
        }
        let lower_rows: Vec<serde_json::Value> = lowers.iter()
            .map(|text| serde_json::json!({"text": text, "lower": text.to_lowercase()})).collect();
        let family = [0x1F468u32, 0x200D, 0x1F469, 0x200D, 0x1F467].iter().map(|c| ch(*c)).collect::<String>();
        let s = |list: &[&str]| list.iter().map(|option| option.to_string()).collect::<Vec<String>>();
        let option_cases: Vec<Vec<String>> = vec![
            s(&["  Pizza ", "Pasta"]), s(&["a", "  "]), s(&["Pizza", "PIZZA"]), s(&["Ärger", "ärger"]),
            s(&["ΟΔΟΣ", "οδοσ"]), s(&["ΟΔΟΣ", "οδος"]), vec![ch(0x130), "i".into()], vec![ch(0x130), format!("i{}", ch(0x307))],
            vec!["я".repeat(100), "b".into()], vec!["я".repeat(101), "b".into()], vec![family.repeat(20), "b".into()],
            vec![family.repeat(21), "b".into()], (0..11).map(|n| n.to_string()).collect(), (0..10).map(|n| n.to_string()).collect(),
            s(&["a", "b", "", "  "]), vec![], vec![format!("{}x{}", ch(0xA0), ch(0xA0)), "x".into()],
            vec![ch(0x1C5), ch(0x1C6)], s(&["ß", "SS"]), s(&["ß", "ẞ"]), vec![format!("{}a", ch(0x200B)), "a".into()],
            (0..12).map(|n| if n < 2 { String::from("  ") } else { n.to_string() }).collect(),
        ];
        let option_rows: Vec<serde_json::Value> = option_cases.iter()
            .map(|options| serde_json::json!({"options": options, "sent": sanitized(options)})).collect();
        for (name, rows) in [("poll_trim", trim_rows), ("poll_lower", lower_rows), ("poll_options", option_rows)] {
            out.push_str(&format!("  \"{}\": [\n{}\n  ],\n", name,
                rows.iter().map(|r| format!("    {}", r)).collect::<Vec<_>>().join(",\n")));
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

// --- markdown and links: the body a bubble draws (fc_text::markdown, fc_text::links) -----------------------

fn md_link(link: &Option<fc_text::markdown::Link>) -> serde_json::Value {
    match link {
        Some(link) => serde_json::json!({"destination": link.destination, "title": link.title}),
        None => serde_json::Value::Null,
    }
}

fn md_text(text: &fc_text::markdown::Text) -> serde_json::Value {
    use fc_text::markdown::Font;
    serde_json::Value::Array(
        text.spans
            .iter()
            .map(|span| {
                let style = &span.style;
                let font = match style.font {
                    Font::Body => "body".to_string(),
                    Font::Heading(level) => format!("h{level}"),
                    Font::Monospaced => "mono".to_string(),
                };
                serde_json::json!({"text": span.text, "emphasis": style.emphasis, "strong": style.strong,
                    "code": style.code, "strikethrough": style.strikethrough, "line_break": style.line_break,
                    "html": style.html, "link": md_link(&style.link), "image": md_link(&style.image), "font": font})
            })
            .collect(),
    )
}

fn md_blocks(body: &str) -> serde_json::Value {
    use fc_text::markdown::{Block, ColumnAlignment};
    serde_json::Value::Array(
        fc_text::markdown::blocks(body)
            .iter()
            .map(|block| match block {
                Block::Text(text) => serde_json::json!({"text": md_text(text)}),
                Block::Table(table) => serde_json::json!({"table": {
                    "alignments": table.alignments.iter().map(|alignment| match alignment {
                        ColumnAlignment::Leading => "leading",
                        ColumnAlignment::Center => "center",
                        ColumnAlignment::Trailing => "trailing",
                    }).collect::<Vec<_>>(),
                    "header": table.header.iter().map(md_text).collect::<Vec<_>>(),
                    "rows": table.rows.iter().map(|row| row.iter().map(md_text).collect::<Vec<_>>()).collect::<Vec<_>>(),
                }}),
            })
            .collect(),
    )
}

fn link_spans(spans: &[fc_text::links::LinkSpan]) -> serde_json::Value {
    serde_json::Value::Array(
        spans
            .iter()
            .map(|span| serde_json::json!({"start": span.range.start, "end": span.range.end, "text": span.text, "target": span.target}))
            .collect(),
    )
}

/// The web body's declared links (web/src/views/body.rs `pieces`, step 2): every markdown destination that can be
/// opened once normalised, a label split into runs still one link.
fn declared_links(text: &fc_text::markdown::Text) -> Vec<fc_text::links::LinkSpan> {
    let plain = text.plain();
    let mut declared: Vec<fc_text::links::LinkSpan> = Vec::new();
    let mut at = 0;
    for span in &text.spans {
        let range = at..at + span.text.len();
        at += span.text.len();
        let Some(link) = &span.style.link else { continue };
        let target = fc_text::links::normalize_destination(&link.destination);
        if !fc_text::links::is_openable(&target) {
            continue;
        }
        match declared.last_mut() {
            Some(previous) if previous.range.end == range.start && previous.target == target => {
                previous.range.end = range.end;
                previous.text = plain[previous.range.clone()].to_string();
            }
            _ => declared.push(fc_text::links::LinkSpan {
                range: range.clone(),
                text: plain[range].to_string(),
                target,
            }),
        }
    }
    declared
}

fn markdown_vectors(corpus: String) {
    let bodies: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(corpus).expect("the corpus")).expect("a list of bodies");
    let cases: Vec<serde_json::Value> = bodies
        .iter()
        .map(|body| {
            let rendered = fc_text::markdown::render(body);
            let plain = rendered.plain();
            let detected = fc_text::links::detect(&plain);
            let merged = fc_text::links::merge(declared_links(&rendered), detected.clone());
            let normalized: Vec<serde_json::Value> = rendered
                .spans
                .iter()
                .filter_map(|span| span.style.link.as_ref())
                .map(|link| {
                    let target = fc_text::links::normalize_destination(&link.destination);
                    serde_json::json!({"destination": link.destination, "normalized": target,
                        "openable": fc_text::links::is_openable(&target)})
                })
                .collect();
            serde_json::json!({
                "body": body,
                "blocks": md_blocks(body),
                "render": md_text(&rendered),
                "detect_body": link_spans(&fc_text::links::detect(body)),
                "detect": link_spans(&detected),
                "merged": link_spans(&merged),
                "first_web": fc_text::links::first_web_link(&merged).map(|span| span.target.clone()),
                "assistant_ranges": fc_text::assistant::ranges(&plain).iter().map(|range| [range.start, range.end]).collect::<Vec<_>>(),
                "draw_range": fc_text::assistant::draw_token_range(&plain).map(|range| [range.start, range.end]),
                "draw_asked": fc_text::assistant::draw_token_range(body).is_some(),
                "normalized": normalized,
            })
        })
        .collect();
    println!("{}", serde_json::to_string(&serde_json::json!({"cases": cases})).unwrap());
}

/// The standard library's character properties over every scalar, as ranges — what fc_text decides by, so the
/// port decides by exactly the same Unicode version rather than by .NET's.
fn unicode_tables() {
    use unicode_segmentation::UnicodeSegmentation;
    fn ranges(predicate: impl Fn(char) -> bool) -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = Vec::new();
        for value in 0u32..=0x10FFFF {
            let Some(c) = char::from_u32(value) else { continue };
            if !predicate(c) {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.1 + 1 == value => last.1 = value,
                _ => out.push((value, value)),
            }
        }
        out
    }
    let lower: Vec<serde_json::Value> = (0u32..=0x10FFFF)
        .filter_map(char::from_u32)
        .filter(|&c| c.to_lowercase().collect::<String>() != c.to_string())
        .map(|c| serde_json::json!([c as u32, c.to_lowercase().collect::<String>()]))
        .collect();
    let upper: Vec<serde_json::Value> = (0u32..=0x10FFFF)
        .filter_map(char::from_u32)
        .filter(|&c| c.to_uppercase().collect::<String>() != c.to_string())
        .map(|c| serde_json::json!([c as u32, c.to_uppercase().collect::<String>()]))
        .collect();
    let tables = serde_json::json!({
        "alphabetic": ranges(char::is_alphabetic),
        "numeric": ranges(char::is_numeric),
        "lowercase": ranges(char::is_lowercase),
        "uppercase": ranges(char::is_uppercase),
        "whitespace": ranges(char::is_whitespace),
        "control": ranges(char::is_control),
        "to_lowercase": lower,
        "to_uppercase": upper,
        // str::to_lowercase's Final_Sigma context, read off the standard library itself rather than a Unicode table of this
        // machine's: a scalar the context SKIPS makes "AxΣ" end in ς while "xΣ" does not; one it counts as cased makes "xΣ"
        // end in ς. (Rust's own Case_Ignorable and Cased tables are private.)
        "case_ignorable": ranges(|c| format!("A{c}Σ").to_lowercase().ends_with('ς') && !format!("{c}Σ").to_lowercase().ends_with('ς')),
        "cased_not_ignorable": ranges(|c| format!("{c}Σ").to_lowercase().ends_with('ς')),
        // unicode-segmentation's clusters, as far as fc_text::markdown asks about them: whether a scalar
        // after an ASCII character joins its cluster (Extend, ZWJ, SpacingMark), and whether one before joins it
        // (Prepend). Those two decide whether a markup character is a cluster of its own.
        "grapheme_extends": ranges(|c| format!("a{c}").graphemes(true).count() == 1),
        "grapheme_prepends": ranges(|c| format!("{c}a").graphemes(true).count() == 1),
    });
    println!("{}", serde_json::to_string(&tables).unwrap());
}

// --- media plan: what a picked video or sound file becomes before upload (fc_text::media_plan) ------------

/// A frame rate as JSON. A whole one is written as an integer (`30`, not `30.0`), so that every
/// number in the file that IS an integer reads as one in all four languages; anything else is the
/// shortest decimal that round-trips, which every port's parser turns back into the same `f64`.
fn rate(value: f64) -> serde_json::Value {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 9.0e15 {
        serde_json::json!(value as i64)
    } else {
        serde_json::json!(value)
    }
}

fn video_input(source: &fc_text::media_plan::VideoSource) -> serde_json::Value {
    serde_json::json!({
        "width": source.width,
        "height": source.height,
        "frame_rate": source.frame_rate.map(rate),
        "container": source.container,
        "video_codec": source.video_codec,
        "audio_codec": source.audio_codec,
        "audio_channels": source.audio_channels,
        "video_bitrate": source.video_bitrate,
        "audio_bitrate": source.audio_bitrate,
        "size_bytes": source.size_bytes,
        "duration_ms": source.duration_ms,
    })
}

/// Every intermediate as well as the plan, so a port that disagrees finds out WHICH step it
/// disagrees on: V, the target (computed for a kept source too — rule A is judged against it) and
/// rule A's verdict.
fn video_expected(source: &fc_text::media_plan::VideoSource) -> serde_json::Value {
    use fc_text::media_plan::{self as mp, VideoPlan};
    let target = mp::video_target(source);
    let plan = match mp::plan_video(source) {
        VideoPlan::Keep => "keep",
        VideoPlan::Transcode(transcode) => {
            assert_eq!(Some(transcode), target, "a transcode is to the target");
            "transcode"
        }
        VideoPlan::Fallback => "fallback",
    };
    serde_json::json!({
        "plan": plan,
        "within_profile": mp::within_profile(source),
        "source_video_bitrate": mp::source_video_bitrate(source),
        "target": target.map(|target| serde_json::json!({
            "width": target.width,
            "height": target.height,
            "frame_rate": rate(target.frame_rate),
            "video_bitrate": target.video_bitrate,
            "audio_bitrate": target.audio_bitrate,
        })),
    })
}

fn audio_input(source: &fc_text::media_plan::AudioSource) -> serde_json::Value {
    serde_json::json!({
        "container": source.container,
        "codec": source.codec,
        "channels": source.channels,
        "bitrate": source.bitrate,
        "size_bytes": source.size_bytes,
        "duration_ms": source.duration_ms,
    })
}

fn audio_expected(source: &fc_text::media_plan::AudioSource) -> serde_json::Value {
    use fc_text::media_plan::{self as mp, AudioPlan};
    let bitrate = mp::source_audio_bitrate(source);
    let target = mp::target_audio_bitrate(source.channels, bitrate);
    let plan = match mp::plan_audio(source) {
        AudioPlan::Keep => "keep",
        AudioPlan::Transcode { bitrate } => {
            assert_eq!(bitrate, target, "a transcode is to the target");
            "transcode"
        }
    };
    serde_json::json!({"plan": plan, "source_bitrate": bitrate, "target_bitrate": target})
}

/// The vectors for `fc_text::media_plan` (docs/protocol.md, "Preparing media before upload"): one
/// case per line, each `{"name", "function", "input", "expected"}`, where `function` names the
/// module's function the case is for and the input and expected fields are its arguments and
/// answers under the module's own names. Hand-picked cases for every rule and edge first, then a
/// sweep across sizes, turns, frame rates, bitrate situations and containers that nobody picked.
fn media_plan_vectors() {
    use fc_text::media_plan::{self as mp, AudioSource, OnFailure, Upload, VideoSource};
    use serde_json::json;

    let mut cases: Vec<serde_json::Value> = Vec::new();
    let mut case = |name: &str, function: &str, input: serde_json::Value, expected: serde_json::Value| {
        cases.push(json!({"name": name, "function": function, "input": input, "expected": expected}));
    };

    // --- the steps on their own ------------------------------------------------------------
    for (width, height) in [
        (1920, 1080), (1080, 1920), (3840, 2160), (2160, 3840), (1440, 1080), (1080, 1440),
        (1080, 1080), (2560, 1080), (854, 480), (480, 854), (640, 360), (176, 144),
        (1280, 720), (720, 1280), (1282, 721), (721, 1282), (1281, 721), (853, 481),
        (1279, 719), (641, 361), (360, 641), (1000, 800), (1001, 800), (2025, 1080),
        (1443, 1440), (1441, 1440), (720, 2560), (1, 1080), (1080, 1), (1, 1), (2, 2),
        (0, 1080), (1920, 0), (0, 0),
    ] {
        let (target_width, target_height) = mp::target_size(width, height);
        case(
            &format!("target size {width}x{height}"),
            "target_size",
            json!({"width": width, "height": height}),
            json!({"width": target_width, "height": target_height}),
        );
    }
    for frame_rate in [
        Some(24.0), Some(24_000.0 / 1001.0), Some(25.0), Some(29.97), Some(30_000.0 / 1001.0),
        Some(30.0), Some(30.5), Some(30.51), Some(50.0), Some(59.94), Some(60_000.0 / 1001.0),
        Some(60.0), Some(120.0), Some(240.0), Some(15.0), Some(0.0), Some(-1.0), None,
    ] {
        case(
            &format!("target frame rate {}", frame_rate.map_or("unknown".to_string(), |rate| rate.to_string())),
            "target_frame_rate",
            json!({"frame_rate": frame_rate.map(rate)}),
            json!({"frame_rate": rate(mp::target_frame_rate(frame_rate))}),
        );
    }
    for (width, height, frame_rate, why) in [
        (1280, 720, 30.0, "the reference point"),
        (720, 1280, 30.0, "turned"),
        (1280, 720, 25.0, "1666.67 thousand"),
        (1280, 720, 24.0, ""),
        (1280, 720, 29.97, ""),
        (1280, 720, 30_000.0 / 1001.0, ""),
        (1280, 720, 30.5, "2033.33 thousand, over the ceiling"),
        (960, 720, 30.0, ""),
        (854, 480, 30.0, "889.58 thousand"),
        (852, 480, 24_000.0 / 1001.0, ""),
        (640, 360, 30.0, ""),
        (720, 720, 30.0, ""),
        (320, 240, 15.0, "under the floor"),
        (176, 144, 30.0, "under the floor"),
        (720, 2560, 30.0, "a panorama, over the ceiling"),
        (0, 1080, 30.0, "no pixels at all"),
        (404, 288, 30.0, "exactly 252.5 thousand: half up, where half-even says 252"),
        (484, 360, 24.0, "exactly 302.5 thousand: half up, where half-even says 302"),
        (960, 540, 25.0, "exactly 937.5 thousand"),
        (3618, 128, 25.0, "exactly 837.5 thousand; left to right as written it is 837.4999"),
        (644, 360, 24.0, "exactly 402.5 thousand; left to right as written it is 402.50000000000006"),
    ] {
        case(
            &format!("profile bitrate {width}x{height} at {frame_rate}{}{why}", if why.is_empty() { "" } else { ": " }),
            "profile_video_bitrate",
            json!({"width": width, "height": height, "frame_rate": rate(frame_rate)}),
            json!({"bitrate": mp::profile_video_bitrate(width, height, frame_rate)}),
        );
    }
    for (source_bitrate, why) in [
        (None, "no V: the profile's"),
        (Some(0), "a stated 0 is no V"),
        (Some(9_000_000), "V above: the profile's"),
        (Some(2_000_000), "V equal"),
        (Some(800_000), "V below: V"),
        (Some(1_234_567), "the cap comes after the rounding"),
        (Some(100_000), "below the floor, still V"),
    ] {
        case(
            &format!("target bitrate 1280x720 at 30, {why}"),
            "target_video_bitrate",
            json!({"width": 1280, "height": 720, "frame_rate": 30, "source_bitrate": source_bitrate}),
            json!({"bitrate": mp::target_video_bitrate(1280, 720, 30.0, source_bitrate)}),
        );
    }
    for (channels, source_bitrate, why) in [
        (Some(2), None, "stereo"),
        (Some(1), None, "mono"),
        (Some(6), None, "5.1 takes the stereo rate"),
        (None, None, "a count nobody gave is not mono"),
        (Some(0), None, "0 channels is not mono either"),
        (Some(2), Some(96_000), "stereo, never above the source's"),
        (Some(1), Some(48_000), "mono, never above the source's"),
        (Some(1), Some(96_000), "mono under a higher source"),
        (Some(2), Some(0), "a stated 0 is no rate"),
        (Some(2), Some(1_411_200), "CD audio"),
    ] {
        case(
            &format!("target audio bitrate: {why}"),
            "target_audio_bitrate",
            json!({"channels": channels, "source_bitrate": source_bitrate}),
            json!({"bitrate": mp::target_audio_bitrate(channels, source_bitrate)}),
        );
    }
    for (size_bytes, duration_ms, audio_bitrate, why) in [
        (1_000_000u64, Some(3_000u64), 128_000u64, "2 666 666.67 truncates, less the audio"),
        (1_000_000, Some(3_000), 0, "nothing to take off"),
        (1_000_000, None, 0, "no duration"),
        (1_000_000, Some(0), 0, "a duration of 0 is none"),
        (0, Some(1_000), 0, "an empty file"),
        (16_000, Some(1_000), 128_000, "the audio is the whole file"),
        (16_000, Some(1_000), 127_999, "one bit a second left"),
        (16_000, Some(1_000), 200_000, "more audio than file"),
        (3_285_000, Some(10_000), 128_000, "exactly 2 500 000"),
        (3_285_001, Some(10_000), 128_000, "truncates back to 2 500 000"),
        (3_285_002, Some(10_000), 128_000, "2 500 001"),
        (7_200_000, Some(180_000), 0, "a three-minute MP3 at 320k"),
        (31_752_000, Some(180_000), 0, "three minutes of CD audio"),
        (95_000_000, Some(95_000), 128_000, "a 95 MB phone clip"),
    ] {
        case(
            &format!("estimated bitrate: {why}"),
            "estimated_bitrate",
            json!({"size_bytes": size_bytes, "duration_ms": duration_ms, "audio_bitrate": audio_bitrate}),
            json!({"bitrate": mp::estimated_bitrate(size_bytes, duration_ms, audio_bitrate)}),
        );
    }

    // --- a video, by hand --------------------------------------------------------------------
    // A 1080p H.264 phone clip with stereo AAC, every number stated; each case changes it.
    let clip = VideoSource {
        width: 1920,
        height: 1080,
        frame_rate: Some(30.0),
        container: "video/mp4".into(),
        video_codec: "h264".into(),
        audio_codec: Some("aac".into()),
        audio_channels: Some(2),
        video_bitrate: Some(8_000_000),
        audio_bitrate: Some(128_000),
        size_bytes: 20_000_000,
        duration_ms: Some(19_000),
    };
    // 720p30 H.264 at 1.5 Mbit/s: within the profile (the design's test fixture 2).
    let within = VideoSource {
        width: 1280,
        height: 720,
        video_bitrate: Some(1_500_000),
        size_bytes: 2_035_000,
        duration_ms: Some(10_000),
        ..clip.clone()
    };
    let hevc = |source: &VideoSource| VideoSource { video_codec: "hevc".into(), ..source.clone() };
    let sized = |width: u32, height: u32, base: &VideoSource| VideoSource { width, height, ..base.clone() };
    let silent = |source: &VideoSource| VideoSource {
        audio_codec: None,
        audio_channels: None,
        audio_bitrate: None,
        ..source.clone()
    };

    let mut videos: Vec<(String, VideoSource)> = vec![
        ("1080p landscape".into(), clip.clone()),
        ("1080p portrait".into(), sized(1080, 1920, &clip)),
        (
            "4K60 HEVC portrait from a phone (the design's test fixture 1)".into(),
            VideoSource {
                width: 2160,
                height: 3840,
                frame_rate: Some(59.94),
                container: "video/quicktime".into(),
                video_codec: "hevc".into(),
                video_bitrate: Some(40_000_000),
                size_bytes: 150_000_000,
                duration_ms: Some(29_000),
                ..clip.clone()
            },
        ),
        ("4K landscape".into(), sized(3840, 2160, &clip)),
        ("within the profile: kept".into(), within.clone()),
        ("within the profile, portrait: kept".into(), sized(720, 1280, &within)),
        ("within the profile, no audio: kept".into(), silent(&within)),
        (
            "within the profile, 320k audio: kept (rule A does not ask the audio's rate)".into(),
            VideoSource { audio_bitrate: Some(320_000), ..within.clone() },
        ),
        (
            "within the profile, V estimated: kept".into(),
            VideoSource { video_bitrate: None, ..within.clone() },
        ),
        ("short side exactly 720, V too high".into(), VideoSource { video_bitrate: Some(8_000_000), ..within.clone() }),
        ("short side 721: scaled".into(), sized(1282, 721, &within)),
        ("short side 721, portrait".into(), sized(721, 1282, &within)),
        ("short side 721, odd long side".into(), sized(1281, 721, &within)),
        ("odd sides under the cap, HEVC".into(), hevc(&sized(853, 481, &within))),
        (
            "odd sides under the cap, H.264: kept (even sides are not in rule A)".into(),
            VideoSource { video_bitrate: Some(1_000_000), ..sized(1279, 719, &within) },
        ),
        ("odd sides under the cap, HEVC, transcoded".into(), hevc(&sized(1279, 719, &within))),
        ("480p stays 480p".into(), hevc(&sized(854, 480, &within))),
        ("480p portrait stays 480p".into(), hevc(&sized(480, 854, &within))),
        ("360p stays 360p".into(), hevc(&sized(640, 360, &within))),
        ("QCIF stays QCIF, bitrate at the floor".into(), hevc(&sized(176, 144, &within))),
        ("square".into(), sized(1080, 1080, &clip)),
        (
            "square 720 at 1M: kept".into(),
            VideoSource { video_bitrate: Some(1_000_000), ..sized(720, 720, &within) },
        ),
        ("square 720 at 1.5M, 1.33 times its smaller target: transcoded".into(), sized(720, 720, &within)),
        ("QuickTime holding H.264 is never within the profile".into(), VideoSource {
            container: "video/quicktime".into(),
            ..within.clone()
        }),
        ("HEVC at 900k: transcoded, never above V".into(), VideoSource { video_bitrate: Some(900_000), ..hevc(&within) }),
        ("an unnamed video codec".into(), VideoSource { video_codec: "unknown".into(), ..within.clone() }),
        ("AV1".into(), VideoSource { video_codec: "av1".into(), ..within.clone() }),
        ("MP3 audio in MP4".into(), VideoSource { audio_codec: Some("mp3".into()), ..within.clone() }),
        ("an unnamed audio codec is not absent".into(), VideoSource {
            audio_codec: Some("unknown".into()),
            ..within.clone()
        }),
        ("no audio, transcoded: no audio target".into(), silent(&clip)),
        ("mono audio".into(), VideoSource { audio_channels: Some(1), audio_bitrate: Some(64_000), ..clip.clone() }),
        ("mono audio under 64k".into(), VideoSource { audio_channels: Some(1), audio_bitrate: Some(48_000), ..clip.clone() }),
        ("stereo audio under 128k".into(), VideoSource { audio_bitrate: Some(96_000), ..clip.clone() }),
        ("5.1 audio".into(), VideoSource { audio_channels: Some(6), audio_bitrate: Some(384_000), ..clip.clone() }),
        ("channels unknown".into(), VideoSource { audio_channels: None, ..clip.clone() }),
        ("channels 0".into(), VideoSource { audio_channels: Some(0), ..clip.clone() }),
        ("audio rate not stated, V stated".into(), VideoSource { audio_bitrate: None, ..clip.clone() }),
        (
            "audio rate not stated, V not stated: V unknown".into(),
            VideoSource { audio_bitrate: None, video_bitrate: None, ..within.clone() },
        ),
        (
            "no audio track, V estimated from the whole file".into(),
            VideoSource { video_bitrate: None, ..silent(&within) },
        ),
        ("V stated as 0 is estimated".into(), VideoSource { video_bitrate: Some(0), ..within.clone() }),
        (
            "V unknown: no duration".into(),
            VideoSource { video_bitrate: None, duration_ms: None, ..within.clone() },
        ),
        (
            "V unknown: duration 0".into(),
            VideoSource { video_bitrate: None, duration_ms: Some(0), ..within.clone() },
        ),
        (
            "V unknown: the stated audio is more than the file".into(),
            VideoSource {
                video_bitrate: None,
                size_bytes: 100_000,
                audio_bitrate: Some(128_000),
                ..within.clone()
            },
        ),
        ("rule A: exactly 1.25 times".into(), VideoSource { video_bitrate: Some(2_500_000), ..within.clone() }),
        ("rule A: one bit over".into(), VideoSource { video_bitrate: Some(2_500_001), ..within.clone() }),
        ("rule A at 360p: exactly 1.25 times".into(), VideoSource { video_bitrate: Some(625_000), ..sized(640, 360, &within) }),
        ("rule A at 360p: one bit over".into(), VideoSource { video_bitrate: Some(625_001), ..sized(640, 360, &within) }),
        (
            "rule A at 24 fps: exactly 1.25 times".into(),
            VideoSource { frame_rate: Some(24.0), video_bitrate: Some(2_000_000), ..within.clone() },
        ),
        (
            "rule A at 24 fps: one bit over".into(),
            VideoSource { frame_rate: Some(24.0), video_bitrate: Some(2_000_001), ..within.clone() },
        ),
        (
            "rule A, V estimated exactly 1.25 times".into(),
            VideoSource { video_bitrate: None, size_bytes: 3_285_000, ..within.clone() },
        ),
        (
            "rule A, V estimated: the truncation keeps it at 1.25 times".into(),
            VideoSource { video_bitrate: None, size_bytes: 3_285_001, ..within.clone() },
        ),
        (
            "rule A, V estimated one bit over".into(),
            VideoSource { video_bitrate: None, size_bytes: 3_285_002, ..within.clone() },
        ),
        ("the floor".into(), VideoSource { video_bitrate: Some(500_000), frame_rate: Some(15.0), ..hevc(&sized(320, 240, &within)) }),
        (
            "the floor, then capped by V".into(),
            VideoSource { video_bitrate: Some(200_000), frame_rate: Some(15.0), ..hevc(&sized(320, 240, &within)) },
        ),
        (
            "the ceiling: 720p at 30.5".into(),
            VideoSource { frame_rate: Some(30.5), video_bitrate: Some(8_000_000), ..hevc(&within) },
        ),
        ("the ceiling: a panorama".into(), hevc(&sized(2560, 720, &clip))),
        ("the ceiling: a portrait panorama".into(), hevc(&sized(720, 2560, &clip))),
        ("never above V".into(), VideoSource { video_bitrate: Some(1_200_000), ..hevc(&clip) }),
        ("never above V, to the bit".into(), VideoSource { video_bitrate: Some(1_234_567), ..hevc(&clip) }),
        ("a tie at 404x288, 30 fps".into(), VideoSource { video_bitrate: None, duration_ms: None, ..hevc(&sized(404, 288, &within)) }),
        ("a tie at 484x360, 24 fps".into(), VideoSource {
            frame_rate: Some(24.0),
            video_bitrate: None,
            duration_ms: None,
            ..hevc(&sized(484, 360, &within))
        }),
        ("a tie at 960x540, 25 fps".into(), VideoSource {
            frame_rate: Some(25.0),
            video_bitrate: None,
            duration_ms: None,
            ..hevc(&sized(960, 540, &within))
        }),
        ("a tie at 3618x128, 25 fps".into(), VideoSource {
            frame_rate: Some(25.0),
            video_bitrate: None,
            duration_ms: None,
            ..hevc(&sized(3618, 128, &within))
        }),
        (
            "within the profile but over the 100 MB ceiling: kept (rule A names no ceiling)".into(),
            VideoSource {
                video_bitrate: Some(2_000_000),
                size_bytes: 160_000_000,
                duration_ms: Some(600_000),
                ..within.clone()
            },
        ),
        ("no width: the fallback".into(), sized(0, 1080, &clip)),
        ("no height: the fallback".into(), sized(1920, 0, &clip)),
        ("no size at all: the fallback".into(), sized(0, 0, &clip)),
        ("one pixel wide: no even width, the fallback".into(), sized(1, 1080, &clip)),
        ("one pixel high: the fallback".into(), sized(1080, 1, &clip)),
        (
            "one pixel wide but within the profile: kept, the original goes".into(),
            VideoSource { video_bitrate: Some(200_000), ..sized(1, 1080, &within) },
        ),
        ("two by two".into(), hevc(&sized(2, 2, &within))),
    ];
    // Every frame rate the task names, on a clip that is otherwise within the profile (so rule A
    // turns on the rate alone) and on a 1080p HEVC clip (so the target's rate and bitrate show).
    for (label, frame_rate) in [
        ("24", Some(24.0)),
        ("23.976", Some(24_000.0 / 1001.0)),
        ("25", Some(25.0)),
        ("29.97", Some(29.97)),
        ("30000/1001", Some(30_000.0 / 1001.0)),
        ("30", Some(30.0)),
        ("30.5", Some(30.5)),
        ("30.51", Some(30.51)),
        ("50", Some(50.0)),
        ("59.94", Some(59.94)),
        ("60000/1001", Some(60_000.0 / 1001.0)),
        ("60", Some(60.0)),
        ("120", Some(120.0)),
        ("240", Some(240.0)),
        ("unknown", None),
        ("0, which is unknown", Some(0.0)),
        ("-1, which is unknown", Some(-1.0)),
    ] {
        videos.push((format!("{label} fps, otherwise within the profile"), VideoSource { frame_rate, ..within.clone() }));
        videos.push((format!("{label} fps, 1080p HEVC"), VideoSource { frame_rate, ..hevc(&clip) }));
    }
    for (name, source) in &videos {
        case(name, "plan_video", video_input(source), video_expected(source));
    }

    // --- a video, swept ----------------------------------------------------------------------
    // Sizes × turns × frame rates, with the bitrate situation and the container/codecs rotating
    // through them so that every pairing turns up without a cross product nobody could read: the
    // situation steps with k + group and the variant with 2k + group, which between them meet
    // every situation with every variant, and every frame rate with both.
    let resolutions: [(u32, u32); 12] = [
        (3840, 2160), (2560, 1440), (1920, 1080), (1440, 1080), (1280, 720), (1080, 1080),
        (960, 540), (854, 480), (640, 480), (640, 360), (426, 240), (1281, 721),
    ];
    let rates: [(&str, Option<f64>); 8] = [
        ("24", Some(24.0)),
        ("25", Some(25.0)),
        ("30000/1001", Some(30_000.0 / 1001.0)),
        ("30", Some(30.0)),
        ("50", Some(50.0)),
        ("60000/1001", Some(60_000.0 / 1001.0)),
        ("60", Some(60.0)),
        ("unknown", None),
    ];
    // (container, video codec, audio as (codec, channels, stated bitrate) or none)
    type Variant = (&'static str, &'static str, Option<(&'static str, u32, u64)>);
    let variants: [Variant; 6] = [
        ("video/mp4", "h264", Some(("aac", 2, 128_000))),
        ("video/quicktime", "hevc", Some(("aac", 2, 128_000))),
        ("video/mp4", "h264", None),
        ("video/mp4", "hevc", Some(("aac", 1, 64_000))),
        ("video/mp4", "h264", Some(("aac", 1, 48_000))),
        ("video/quicktime", "h264", Some(("aac", 2, 160_000))),
    ];
    let situations = ["V stated high", "V stated low", "V estimated", "V unknown"];
    let mut group = 0usize;
    for (width, height) in resolutions {
        for turned in [false, true] {
            if turned && width == height {
                continue;
            }
            let (width, height) = if turned { (height, width) } else { (width, height) };
            let pixels = u64::from(width) * u64::from(height);
            for (k, (rate_label, frame_rate)) in rates.iter().enumerate() {
                let situation = (k + group) % situations.len();
                let (container, video_codec, audio) = variants[(2 * k + group) % variants.len()];
                let audio_rate = audio.map_or(0, |(_, _, bitrate)| bitrate);
                // Consistent files: the size is what the rates make over the duration.
                let (video_bitrate, size_bytes, duration_ms) = match situation {
                    0 => (Some(pixels * 6), (pixels * 6 + audio_rate) * 5 / 4, Some(10_000)),
                    1 => (Some(pixels * 2), (pixels * 2 + audio_rate) * 5 / 4, Some(10_000)),
                    2 => (None, (pixels * 5 / 2 + audio_rate) * 9 / 8, Some(9_000)),
                    _ => (None, (pixels * 3 + audio_rate) * 5 / 4, None),
                };
                let source = VideoSource {
                    width,
                    height,
                    frame_rate: *frame_rate,
                    container: container.into(),
                    video_codec: video_codec.into(),
                    audio_codec: audio.map(|(codec, _, _)| codec.to_string()),
                    audio_channels: audio.map(|(_, channels, _)| channels),
                    video_bitrate,
                    audio_bitrate: audio.map(|(_, _, bitrate)| bitrate),
                    size_bytes,
                    duration_ms,
                };
                let audio_label = match audio {
                    Some((codec, channels, bitrate)) => {
                        format!("{codec} {} {}k", if channels == 1 { "mono" } else { "stereo" }, bitrate / 1000)
                    }
                    None => "no audio".into(),
                };
                let name = format!(
                    "sweep {width}x{height}{} at {rate_label} fps, {}, {container} {video_codec} + {audio_label}",
                    if turned { " (turned 90°)" } else { "" },
                    situations[situation],
                );
                case(&name, "plan_video", video_input(&source), video_expected(&source));
            }
            group += 1;
        }
    }

    // --- audio alone ---------------------------------------------------------------------------
    // Three minutes, stereo, with a size that estimates to 222 222 bit/s unless a case says otherwise.
    let song = |container: &str, codec: &str, bitrate: Option<u64>| AudioSource {
        container: container.into(),
        codec: codec.into(),
        channels: Some(2),
        bitrate,
        size_bytes: 5_000_000,
        duration_ms: Some(180_000),
    };
    let mono = |source: AudioSource| AudioSource { channels: Some(1), ..source };
    let by_size = |source: AudioSource, size_bytes: u64| AudioSource { bitrate: None, size_bytes, ..source };
    let audios: Vec<(&str, AudioSource)> = vec![
        ("MP3 128k: kept", song("audio/mpeg", "mp3", Some(128_000))),
        ("MP3 at exactly 192 000: kept", song("audio/mpeg", "mp3", Some(192_000))),
        ("MP3 at 192 001: re-encoded", song("audio/mpeg", "mp3", Some(192_001))),
        ("MP3 320k: re-encoded", song("audio/mpeg", "mp3", Some(320_000))),
        ("MP3 256k mono: re-encoded to 64k", mono(song("audio/mpeg", "mp3", Some(256_000)))),
        ("MP3 192 001 mono", mono(song("audio/mpeg", "mp3", Some(192_001)))),
        ("AAC 128k: kept", song("audio/mp4", "aac", Some(128_000))),
        ("AAC at exactly 192 000: kept", song("audio/mp4", "aac", Some(192_000))),
        ("AAC at 192 001: re-encoded", song("audio/mp4", "aac", Some(192_001))),
        ("AAC 256k as audio/m4a", song("audio/m4a", "aac", Some(256_000))),
        ("HE-AAC 48k: kept", song("audio/mp4", "aac", Some(48_000))),
        (
            "MP3 of unknown bitrate: kept",
            AudioSource { duration_ms: None, ..song("audio/mpeg", "mp3", None) },
        ),
        (
            "AAC of unknown bitrate: kept",
            AudioSource { duration_ms: Some(0), ..song("audio/mp4", "aac", Some(0)) },
        ),
        ("MP3 estimated at 320k: re-encoded", by_size(song("audio/mpeg", "mp3", None), 7_200_000)),
        ("MP3 estimated at exactly 192 000: kept", by_size(song("audio/mpeg", "mp3", None), 4_320_000)),
        (
            "MP3 estimated at 192 000.49, truncated to 192 000: kept",
            by_size(song("audio/mpeg", "mp3", None), 4_320_011),
        ),
        (
            "MP3 estimated at 192 001.02, truncated to 192 001: re-encoded",
            by_size(song("audio/mpeg", "mp3", None), 4_320_023),
        ),
        (
            "MP3 estimated at 192 001.96, truncated to 192 001: re-encoded",
            by_size(song("audio/mpeg", "mp3", None), 4_320_044),
        ),
        (
            "MP3 stated 128k in a file that estimates at 320k: the stated rate wins",
            AudioSource { size_bytes: 7_200_000, ..song("audio/mpeg", "mp3", Some(128_000)) },
        ),
        ("WAV, estimated at CD rate", by_size(song("audio/wav", "pcm", None), 31_752_000)),
        ("WAV mono", mono(by_size(song("audio/wav", "pcm", None), 15_876_000))),
        ("WAV mono 8 kHz 8-bit: exactly 64k", mono(by_size(song("audio/wav", "pcm", None), 1_440_000))),
        ("WAV mono at 32k: never raised", mono(by_size(song("audio/wav", "pcm", None), 720_000))),
        (
            "WAV of unknown channels",
            AudioSource { channels: None, ..by_size(song("audio/wav", "pcm", None), 31_752_000) },
        ),
        (
            "WAV of 0 channels",
            AudioSource { channels: Some(0), ..by_size(song("audio/wav", "pcm", None), 31_752_000) },
        ),
        ("WAV holding ADPCM: no rule names it, kept", song("audio/wav", "adpcm", None)),
        ("AIFF", song("audio/aiff", "pcm", Some(1_411_200))),
        ("FLAC, estimated", song("audio/flac", "flac", None)),
        ("FLAC mono", mono(song("audio/flac", "flac", Some(700_000)))),
        (
            "FLAC of unknown bitrate: re-encoded at the profile's",
            AudioSource { duration_ms: None, ..song("audio/flac", "flac", None) },
        ),
        ("ALAC in M4A", song("audio/mp4", "alac", Some(900_000))),
        ("5.1 FLAC", AudioSource { channels: Some(6), ..song("audio/flac", "flac", Some(2_000_000)) }),
        ("Ogg Vorbis 160k", song("audio/ogg", "vorbis", Some(160_000))),
        ("Ogg Vorbis, unknown bitrate", AudioSource { duration_ms: None, ..song("audio/ogg", "vorbis", None) }),
        ("Ogg Opus 96k: never raised", song("audio/ogg", "opus", Some(96_000))),
        ("Ogg Opus mono 32k", mono(song("audio/ogg", "opus", Some(32_000)))),
        ("Ogg Opus mono estimated at 24k", mono(by_size(song("audio/ogg", "opus", None), 540_000))),
        ("Ogg of an unnamed codec", song("audio/ogg", "unknown", None)),
        ("FLAC in Ogg", song("audio/ogg", "flac", None)),
        ("Opus in MP4: no rule names it, kept", song("audio/mp4", "opus", Some(256_000))),
        ("AC-3: no rule names it, kept", song("audio/mp4", "ac3", Some(448_000))),
        ("an unnamed codec in MP4: kept", song("audio/mp4", "unknown", Some(900_000))),
        ("Vorbis outside Ogg: kept", song("audio/mpeg", "vorbis", None)),
    ];
    for (name, source) in &audios {
        case(name, "plan_audio", audio_input(source), audio_expected(source));
    }

    // --- sendable, rule C, rule D -------------------------------------------------------------
    let ceiling = media::SIZE_LIMIT;
    for (kind, container, honest, size_bytes, why) in [
        ("video", "video/mp4", true, 1_000u64, "an MP4"),
        ("video", "video/quicktime", true, 1_000, "a QuickTime movie"),
        ("video", "video/webm", true, 1_000, "not an accepted video type"),
        ("video", "audio/mp4", true, 1_000, "a type of the other kind"),
        ("video", "video/mp4", false, 1_000, "the bytes say otherwise"),
        ("video", "video/mp4", true, ceiling, "at the ceiling is within it"),
        ("video", "video/mp4", true, ceiling + 1, "a byte over the ceiling"),
        ("audio", "audio/mp4", true, 1_000, "M4A"),
        ("audio", "audio/m4a", true, 1_000, "audio/m4a"),
        ("audio", "audio/mpeg", true, 1_000, "MP3"),
        ("audio", "audio/wav", true, 1_000, "WAV"),
        ("audio", "audio/ogg", true, 1_000, "Ogg"),
        ("audio", "audio/aiff", true, 1_000, "AIFF is not accepted"),
        ("audio", "audio/flac", true, 1_000, "FLAC is not accepted"),
        ("audio", "audio/wav", false, 1_000, "dishonest WAV"),
        ("audio", "audio/wav", true, ceiling + 1, "a WAV over the ceiling"),
        ("file", "video/mp4", true, 1_000, "a file is not a kind these rules are about"),
        ("photo", "image/jpeg", true, 1_000, "nor is a photo"),
    ] {
        case(
            &format!("sendable: {why}"),
            "sendable",
            json!({"kind": kind, "container": container, "honest": honest, "size_bytes": size_bytes, "ceiling_bytes": ceiling}),
            json!({"sendable": mp::sendable(kind, container, honest, size_bytes, ceiling)}),
        );
    }
    // A server configured lower than the default: the ceiling is an input, not a constant.
    case(
        "sendable: under a 50 MB server ceiling",
        "sendable",
        json!({"kind": "video", "container": "video/mp4", "honest": true, "size_bytes": 60_000_000, "ceiling_bytes": 50_000_000}),
        json!({"sendable": mp::sendable("video", "video/mp4", true, 60_000_000, 50_000_000)}),
    );
    for source_sendable in [true, false] {
        let send = match mp::on_failure(source_sendable) {
            OnFailure::Original => "original",
            OnFailure::TodaysPath => "todays_path",
        };
        case(
            &format!("rule C: the source is {}sendable", if source_sendable { "" } else { "not " }),
            "on_failure",
            json!({"source_sendable": source_sendable}),
            json!({"send": send}),
        );
    }
    for (source_bytes, source_sendable, result_bytes, why) in [
        (10_000_000u64, true, 12_000_000u64, "bigger, and the source can go: the source"),
        (10_000_000, false, 12_000_000, "bigger, but the source cannot go: the result"),
        (10_000_000, true, 4_000_000, "smaller: the result"),
        (10_000_000, false, 4_000_000, "smaller, source unsendable: the result"),
        (10_000_000, true, 10_000_000, "equal is not bigger: the result"),
        (10_000_000, true, 10_000_001, "one byte bigger: the source"),
        (150_000_000, false, 90_000_000, "a source over the ceiling, made to fit: the result"),
    ] {
        let upload = match mp::keep_smaller(source_bytes, source_sendable, result_bytes) {
            Upload::Source => "source",
            Upload::Result => "result",
        };
        case(
            &format!("rule D: {why}"),
            "keep_smaller",
            json!({"source_bytes": source_bytes, "source_sendable": source_sendable, "result_bytes": result_bytes}),
            json!({"upload": upload}),
        );
    }

    let lines: Vec<String> = cases.iter().map(|case| serde_json::to_string(case).unwrap()).collect();
    println!("[\n  {}\n]", lines.join(",\n  "));
}

// --- record (issue #79) -----------------------------------------------------------------------------
//
// One case per line, `{"name", "function", "input", "expected"}`, keys sorted — the media-plan file's
// shape. The functions: `constants`, `composer_slot`, `video_door`, `round_cap_ms`, `round_warning_ms`,
// `round_diameter`, `is_round` and `hold_step` (the voice recording's reducer, which keeps its first name:
// since 2026-10-06 there is no hold — the microphone's activation is the one way in). The Windows port has
// no reducer, so it reads every function but `hold_step`. A `hold_step` case is ONE step — a state,
// an event and the constants in; the next state, the effects and what the slot is told out — taken from a
// named scenario run through the reducer, so every port meets each transition in a state it can really
// be in, and checks it without replaying anything.

fn record_dimmed(reason: fc_text::record::Dimmed) -> &'static str {
    use fc_text::record::Dimmed;
    match reason {
        Dimmed::Call => "call",
        Dimmed::Busy => "busy",
        Dimmed::NotSent => "not_sent",
    }
}

fn record_recording(recording: fc_text::record::Recording) -> &'static str {
    use fc_text::record::Recording;
    match recording {
        Recording::None => "none",
        Recording::HandsFree => "hands_free",
        Recording::HandsFreeBesideDraft => "hands_free_beside_draft",
    }
}

fn record_slot_inputs(i: &fc_text::record::SlotInputs) -> serde_json::Value {
    serde_json::json!({
        "recorder_open": i.recorder_open,
        "recording": record_recording(i.recording),
        "editing": i.editing,
        "draft_blank": i.draft_blank,
        "staged": i.staged,
        "assistant_chat": i.assistant_chat,
        "can_record": i.can_record,
        "call": i.call,
        "busy": i.busy,
        "not_sent": i.not_sent,
    })
}

fn record_slot(slot: fc_text::record::Slot) -> serde_json::Value {
    use fc_text::record::Slot;
    let (name, enabled, reason) = match slot {
        Slot::Recorder => ("recorder", None, None),
        Slot::SendVoice => ("send_voice", None, None),
        Slot::StopRecording => ("stop_recording", None, None),
        Slot::Save { enabled } => ("save", Some(enabled), None),
        Slot::Send => ("send", None, None),
        Slot::SendDisabled => ("send_disabled", None, None),
        Slot::Dimmed(reason) => ("dimmed", None, Some(record_dimmed(reason))),
        Slot::Microphone => ("microphone", None, None),
    };
    serde_json::json!({
        "row": slot.row(),
        "slot": name,
        "enabled": enabled,
        "reason": reason,
        "label": slot.label(),
        "notice": slot.notice(),
    })
}

fn record_door_inputs(i: &fc_text::record::DoorInputs) -> serde_json::Value {
    serde_json::json!({
        "slot": record_slot_inputs(&i.slot),
        "family_or_direct_chat": i.family_or_direct_chat,
        "server_offers_round": i.server_offers_round,
        "has_camera": i.has_camera,
        "encoder_probe_passes": i.encoder_probe_passes,
        "records_round_video": i.records_round_video,
    })
}

fn record_door(door: fc_text::record::Door) -> serde_json::Value {
    use fc_text::record::Door;
    let (name, reason) = match door {
        Door::Hidden => ("hidden", None),
        Door::Dimmed(reason) => ("dimmed", Some(record_dimmed(reason))),
        Door::Shown => ("shown", None),
    };
    serde_json::json!({"door": name, "reason": reason, "label": door.label(), "notice": door.notice()})
}

fn record_constants(c: &fc_text::record::HoldConstants) -> serde_json::Value {
    serde_json::json!({
        "shortest_recording_ms": c.shortest_recording_ms,
        "activation_guard_ms": c.activation_guard_ms,
        "delete_asks_from_ms": c.delete_asks_from_ms,
    })
}

fn record_situation(s: &fc_text::record::Situation) -> serde_json::Value {
    use fc_text::record::Permission;
    serde_json::json!({
        "permission": match s.permission {
            Permission::Granted => "granted",
            Permission::NotAsked => "not_asked",
            Permission::Denied => "denied",
        },
        "blocked": s.blocked.map(record_dimmed),
    })
}

fn record_state(s: &fc_text::record::HoldState) -> serde_json::Value {
    use fc_text::record::{Phase, Source};
    use serde_json::json;
    let mut value = match s.phase {
        Phase::Idle => json!({"phase": "idle"}),
        Phase::HandsFree { beside_draft } => json!({"phase": "hands_free", "beside_draft": beside_draft}),
        Phase::AskingDelete { recorded_ms } => json!({"phase": "asking_delete", "recorded_ms": recorded_ms}),
        Phase::AwaitingPermission { source, beside_draft } => json!({
            "phase": "awaiting_permission",
            "source": match source { Source::Tap => "tap", Source::Menu => "menu" },
            "beside_draft": beside_draft,
        }),
    };
    value["guard_until_ms"] = json!(s.guard_until_ms);
    value
}

fn record_event(e: &fc_text::record::HoldEvent) -> serde_json::Value {
    use fc_text::record::HoldEvent as E;
    use serde_json::json;
    match *e {
        E::Cap { at_ms } => json!({"event": "cap", "at_ms": at_ms}),
        E::Interruption { at_ms, recorded_ms } => json!({"event": "interruption", "at_ms": at_ms, "recorded_ms": recorded_ms}),
        E::Activate { at_ms, situation, recorded_ms } => json!({
            "event": "activate", "at_ms": at_ms, "situation": record_situation(&situation), "recorded_ms": recorded_ms,
        }),
        E::Record { at_ms, beside_draft, situation, recorded_ms } => json!({
            "event": "record", "at_ms": at_ms, "beside_draft": beside_draft,
            "situation": record_situation(&situation), "recorded_ms": recorded_ms,
        }),
        E::Stop { at_ms, recorded_ms } => json!({"event": "stop", "at_ms": at_ms, "recorded_ms": recorded_ms}),
        E::Delete { at_ms, recorded_ms } => json!({"event": "delete", "at_ms": at_ms, "recorded_ms": recorded_ms}),
        E::Answer { at_ms, delete } => json!({"event": "answer", "at_ms": at_ms, "delete": delete}),
        E::PermissionAnswer { at_ms, granted } => json!({"event": "permission_answer", "at_ms": at_ms, "granted": granted}),
        E::OtherAction { at_ms } => json!({"event": "other_action", "at_ms": at_ms}),
        E::Emptied { at_ms } => json!({"event": "emptied", "at_ms": at_ms}),
    }
}

fn record_effect(e: &fc_text::record::HoldEffect) -> serde_json::Value {
    use fc_text::record::{Announcement, Haptic, Hint, HoldEffect as F};
    use serde_json::json;
    let plain = |name: &str| json!({"effect": name});
    match *e {
        F::Start => plain("start"),
        F::Delete => plain("delete"),
        F::Send => plain("send"),
        F::Review => plain("review"),
        F::Park => plain("park"),
        F::AskDelete => plain("ask_delete"),
        F::AskPermission => plain("ask_permission"),
        F::Denied => plain("denied"),
        F::Explain(reason) => json!({"effect": "explain", "reason": record_dimmed(reason), "text": reason.notice()}),
        F::Hint(hint) => {
            let name = match hint {
                Hint::StoppedAtFiveMinutes => "stopped_at_five_minutes",
                Hint::TooShort => "too_short",
            };
            json!({"effect": "hint", "hint": name, "text": hint.text()})
        }
        F::Announce(announcement) => {
            let name = match announcement {
                Announcement::Recording => "recording",
                Announcement::RecordingDeleted => "recording_deleted",
                Announcement::VoiceMessageSent => "voice_message_sent",
                Announcement::ReadyToReview { .. } => "ready_to_review",
                Announcement::TooShort => "too_short",
                Announcement::StoppedAtFiveMinutes => "stopped_at_five_minutes",
            };
            let mut value = json!({"effect": "announce", "announcement": name, "text": announcement.text()});
            if let Announcement::ReadyToReview { recorded_ms } = announcement {
                value["recorded_ms"] = json!(recorded_ms);
            }
            value
        }
        F::Haptic(haptic) => json!({"effect": "haptic", "haptic": match haptic {
            Haptic::Light => "light",
            Haptic::Success => "success",
            Haptic::Warning => "warning",
        }}),
    }
}

fn record_vectors() {
    use fc_text::record::{
        self as r, AttachmentFlags, Dimmed, DoorInputs, HoldConstants, HoldEvent, HoldState, Permission, Phase,
        Recording, Situation, SlotInputs, Source, WidthClass,
    };
    use serde_json::json;

    let mut cases: Vec<serde_json::Value> = Vec::new();
    let mut case = |name: &str, function: &str, input: serde_json::Value, expected: serde_json::Value| {
        cases.push(json!({"name": name, "function": function, "input": input, "expected": expected}));
    };

    // --- the constants -----------------------------------------------------------------------------
    case(
        "the S1.1 constants, and S5.2's diameters",
        "constants",
        json!({}),
        json!({
            "activation_guard_ms": r::ACTIVATION_GUARD_MS,
            "shortest_recording_ms": r::SHORTEST_RECORDING_MS,
            "voice_cap_ms": r::VOICE_CAP_MS,
            "voice_warning_ms": r::VOICE_WARNING_MS,
            "default_max_round_video_ms": r::DEFAULT_MAX_ROUND_VIDEO_MS,
            "round_cap_margin_ms": r::ROUND_CAP_MARGIN_MS,
            "round_warning_lead_ms": r::ROUND_WARNING_LEAD_MS,
            "silence_peak_dbfs": rate(r::SILENCE_PEAK_DBFS),
            "silence_max_amplitude": r::SILENCE_MAX_AMPLITUDE,
            "silence_sample_magnitude": r::SILENCE_SAMPLE_MAGNITUDE,
            "silence_warning_after_ms": r::SILENCE_WARNING_AFTER_MS,
            "delete_asks_from_ms": r::DELETE_ASKS_FROM_MS,
            "preview_idle_close_ms": r::PREVIEW_IDLE_CLOSE_MS,
            "slot_crossfade_ms": r::SLOT_CROSSFADE_MS,
            "recorder_fade_ms": r::RECORDER_FADE_MS,
            "min_target_apple_pt": r::MIN_TARGET_APPLE_PT,
            "min_target_android_dp": r::MIN_TARGET_ANDROID_DP,
            "min_target_windows_epx": r::MIN_TARGET_WINDOWS_EPX,
            "min_target_web_px": r::MIN_TARGET_WEB_PX,
            "round_diameter_compact": r::ROUND_DIAMETER_COMPACT,
            "round_diameter_regular": r::ROUND_DIAMETER_REGULAR,
            "video_door_label": r::VIDEO_DOOR_LABEL,
            "video_door_tooltip": r::VIDEO_DOOR_TOOLTIP,
            "default_hold_constants": record_constants(&HoldConstants::default()),
        }),
    );
    // --- S1.3, the trailing slot -------------------------------------------------------------------
    // Every row on its own, every pair of rows (the higher must win), and the inputs inside a row.
    let mic = SlotInputs {
        recorder_open: false,
        recording: Recording::None,
        editing: false,
        draft_blank: true,
        staged: false,
        assistant_chat: false,
        can_record: true,
        call: false,
        busy: false,
        not_sent: false,
    };
    let with_row = |mut i: SlotInputs, row: u8| -> SlotInputs {
        match row {
            1 => i.recorder_open = true,
            2 => i.recording = Recording::HandsFree,
            3 => {
                i.recording = Recording::HandsFreeBesideDraft;
                i.draft_blank = false;
            }
            4 => i.editing = true,
            5 => i.draft_blank = false,
            6 => i.assistant_chat = true,
            7 => i.call = true,
            8 => i.busy = true,
            9 => i.not_sent = true,
            _ => {}
        }
        i
    };
    let row_names = [
        "",
        "the video recorder is open",
        "a hands-free recording that started empty",
        "a recording beside a draft",
        "editing",
        "words typed",
        "the assistant's chat",
        "a call",
        "busy with an attachment",
        "a not-sent voice message",
        "otherwise",
    ];
    let mut slot_cases: Vec<(String, SlotInputs)> = Vec::new();
    for row in 1..=10u8 {
        slot_cases.push((format!("row {row} alone: {}", row_names[usize::from(row)]), with_row(mic, row)));
    }
    for higher in 1..=9u8 {
        for lower in higher + 1..=9u8 {
            if (higher, lower) == (2, 3) {
                continue;
            }
            slot_cases.push((
                format!("row {higher} over row {lower}: {} and {}", row_names[usize::from(higher)], row_names[usize::from(lower)]),
                with_row(with_row(mic, lower), higher),
            ));
        }
    }
    let typed = with_row(mic, 5);
    let staged = SlotInputs { staged: true, ..mic };
    slot_cases.extend([
        ("row 2 with every lower row true".to_string(), SlotInputs {
            recording: Recording::HandsFree,
            editing: true,
            draft_blank: false,
            assistant_chat: true,
            call: true,
            busy: true,
            not_sent: true,
            ..mic
        }),
        ("row 3 with items staged, not words".to_string(), SlotInputs { recording: Recording::HandsFreeBesideDraft, staged: true, ..mic }),
        ("row 4 with words in the field: Save enabled".to_string(), SlotInputs { editing: true, draft_blank: false, ..mic }),
        ("row 4 with the field cleared during a call: Save, never a microphone".to_string(), SlotInputs {
            editing: true,
            call: true,
            not_sent: true,
            ..mic
        }),
        ("row 5 by staging alone".to_string(), staged),
        ("row 5 by staging, in a call, busy, with a not-sent message".to_string(), SlotInputs { call: true, busy: true, not_sent: true, ..staged }),
        ("row 5 in the assistant's chat".to_string(), SlotInputs { assistant_chat: true, ..typed }),
        ("row 5 where nothing can record".to_string(), SlotInputs { can_record: false, ..typed }),
        ("row 6 where nothing can record".to_string(), SlotInputs { can_record: false, ..mic }),
        ("row 6 where nothing can record, in a call".to_string(), SlotInputs { can_record: false, call: true, ..mic }),
        ("row 6 in the assistant's chat, busy".to_string(), SlotInputs { assistant_chat: true, busy: true, ..mic }),
        ("every row true".to_string(), {
            let mut all = mic;
            for row in [1, 3, 4, 5, 6, 7, 8, 9] {
                all = with_row(all, row);
            }
            all
        }),
    ]);
    for (name, inputs) in &slot_cases {
        case(&format!("slot: {name}"), "composer_slot", record_slot_inputs(inputs), record_slot(r::composer_slot(inputs)));
    }

    // --- S1.4, the video button --------------------------------------------------------------------
    let open = DoorInputs {
        slot: mic,
        family_or_direct_chat: true,
        server_offers_round: true,
        has_camera: true,
        encoder_probe_passes: true,
        records_round_video: true,
    };
    let mut door_cases: Vec<(String, DoorInputs)> = Vec::new();
    for row in 1..=10u8 {
        door_cases.push((
            format!("beside row {row}: {}", row_names[usize::from(row)]),
            DoorInputs { slot: with_row(mic, row), ..open },
        ));
    }
    for (row, why) in [(10, "a microphone"), (7, "a call"), (9, "a not-sent message")] {
        let at = DoorInputs { slot: with_row(mic, row), ..open };
        door_cases.extend([
            (format!("{why}, against a server without the keys"), DoorInputs { server_offers_round: false, ..at }),
            (format!("{why}, on a device without a camera"), DoorInputs { has_camera: false, ..at }),
            (format!("{why}, in a browser whose probe fails"), DoorInputs { encoder_probe_passes: false, ..at }),
            (format!("{why}, in a build that does not record round video"), DoorInputs { records_round_video: false, ..at }),
            (format!("{why}, in a thread or the assistant's chat"), DoorInputs { family_or_direct_chat: false, ..at }),
        ]);
    }
    door_cases.extend([
        ("items staged".to_string(), DoorInputs { slot: staged, ..open }),
        ("recording hands-free".to_string(), DoorInputs { slot: SlotInputs { recording: Recording::HandsFree, ..mic }, ..open }),
        ("recording beside a draft".to_string(), DoorInputs { slot: SlotInputs { recording: Recording::HandsFreeBesideDraft, ..mic }, ..open }),
        ("editing with the field cleared".to_string(), DoorInputs { slot: with_row(mic, 4), ..open }),
        ("a call and busy: the call's sentence".to_string(), DoorInputs { slot: SlotInputs { call: true, busy: true, ..mic }, ..open }),
        ("busy with a not-sent message: dimmed for busy".to_string(), DoorInputs { slot: SlotInputs { busy: true, not_sent: true, ..mic }, ..open }),
        ("where nothing can record".to_string(), DoorInputs { slot: SlotInputs { can_record: false, ..mic }, ..open }),
        ("Phase 1, everywhere: everything but a build that records".to_string(), DoorInputs { records_round_video: false, ..open }),
    ]);
    for (name, inputs) in &door_cases {
        case(&format!("door: {name}"), "video_door", record_door_inputs(inputs), record_door(r::video_door(inputs)));
    }

    // --- the round video's arithmetic --------------------------------------------------------------
    for (max, why) in [
        (60_000, "the server's"),
        (120_000, "a longer one"),
        (30_000, "a shorter one"),
        (10_500, ""),
        (10_000, "exactly the warning's lead"),
        (9_999, "shorter than the lead: warned from the start"),
        (1_000, ""),
        (501, ""),
        (500, "exactly the margin"),
        (499, "shorter than the margin: clamped, never wrapped"),
        (0, "none at all"),
    ] {
        let suffix = if why.is_empty() { String::new() } else { format!(": {why}") };
        case(
            &format!("round cap for {max} ms{suffix}"),
            "round_cap_ms",
            json!({"max_round_video_ms": max}),
            json!({"cap_ms": r::round_cap_ms(max)}),
        );
        case(
            &format!("round warning for {max} ms{suffix}"),
            "round_warning_ms",
            json!({"max_round_video_ms": max}),
            json!({"warning_ms": r::round_warning_ms(max)}),
        );
    }
    for (width, name) in [(WidthClass::Compact, "compact"), (WidthClass::Regular, "regular")] {
        case(
            &format!("round diameter, {name}"),
            "round_diameter",
            json!({"width_class": name}),
            json!({"diameter": r::round_diameter(width)}),
        );
    }
    let flags = |kind: &'static str, round: bool| AttachmentFlags { kind, round };
    let shapes: Vec<(&str, &str, Vec<AttachmentFlags>)> = vec![
        ("one video with the flag", "", vec![flags("video", true)]),
        ("one video without it", "", vec![flags("video", false)]),
        ("two flagged videos", "", vec![flags("video", true), flags("video", true)]),
        ("a flagged video and a photo", "", vec![flags("video", true), flags("photo", false)]),
        ("no attachment", "", vec![]),
        ("a body beside it", "Look!", vec![flags("video", true)]),
        ("a body of one space: compared exactly", " ", vec![flags("video", true)]),
        ("the flag on a photo", "", vec![flags("photo", true)]),
        ("the flag on an audio", "", vec![flags("audio", true)]),
        ("the flag on a file", "", vec![flags("file", true)]),
        ("the flag on a location", "", vec![flags("location", true)]),
        ("a kind spelled otherwise", "", vec![flags("Video", true)]),
    ];
    for (name, body, attachments) in &shapes {
        let listed: Vec<serde_json::Value> =
            attachments.iter().map(|a| json!({"kind": a.kind, "round": a.round})).collect();
        case(
            &format!("is round: {name}"),
            "is_round",
            json!({"body": body, "attachments": listed}),
            json!({"round": r::is_round(body, attachments)}),
        );
    }

    // --- S2.1, S2.2 and S2.5, the voice recording, as scenarios -------------------------------------
    // Every activation of the slot is `activate` — the platform button's own completed tap, however long
    // the press was held, a click, Enter or Space, a screen reader's. There is no press, hold or slide.
    let sit = Situation::default();
    let not_asked = Situation { permission: Permission::NotAsked, ..sit };
    let denied = Situation { permission: Permission::Denied, ..sit };
    let activate = |at_ms: u64, recorded_ms: u64| HoldEvent::Activate { at_ms, situation: sit, recorded_ms };
    let activate_with = |at_ms: u64, situation: Situation| HoldEvent::Activate { at_ms, situation, recorded_ms: 0 };
    let record = |at_ms: u64, beside_draft: bool, situation: Situation, recorded_ms: u64| HoldEvent::Record {
        at_ms,
        beside_draft,
        situation,
        recorded_ms,
    };
    let interruption = |at_ms: u64, recorded_ms: u64| HoldEvent::Interruption { at_ms, recorded_ms };
    let stop = |at_ms: u64, recorded_ms: u64| HoldEvent::Stop { at_ms, recorded_ms };
    let delete = |at_ms: u64, recorded_ms: u64| HoldEvent::Delete { at_ms, recorded_ms };
    let answer = |at_ms: u64, granted: bool| HoldEvent::PermissionAnswer { at_ms, granted };
    let other = |at_ms: u64| HoldEvent::OtherAction { at_ms };
    let idle = HoldState::default();
    let defaults = HoldConstants::default();

    type Scenario = (&'static str, HoldState, HoldConstants, Vec<HoldEvent>);
    let scenarios: Vec<Scenario> = vec![
        (
            "a tap records hands-free, a second tap inside the guard is ignored, the same slot sends, and the microphone it leaves is guarded",
            idle,
            defaults,
            vec![activate(120, 0), activate(719, 599), activate(5_120, 5_000), activate(5_719, 0), activate(5_720, 0)],
        ),
        ("a double tap on the microphone finds the recording too short", idle, defaults, vec![activate(100, 0), activate(700, 600)]),
        ("Send at a millisecond under a second is too short", idle, defaults, vec![activate(100, 0), activate(1_200, 999)]),
        ("Send at exactly a second sends", idle, defaults, vec![activate(100, 0), activate(1_200, 1_000)]),
        (
            "a text Send emptied the composer: an activation inside the guard is ignored whole",
            idle,
            defaults,
            vec![HoldEvent::Emptied { at_ms: 1_000 }, activate(1_599, 0), activate(1_600, 0)],
        ),
        (
            "a text Send emptied the composer, then words typed and deleted: never guarded, the microphone records",
            idle,
            defaults,
            vec![HoldEvent::Emptied { at_ms: 1_000 }, other(1_100), other(1_200), activate(1_300, 0)],
        ),
        (
            "Record Voice Message beside a draft, Stop, then a character typed: the row-5 Send is no longer guarded",
            idle,
            defaults,
            vec![record(1_000, true, sit, 0), activate(4_000, 3_000), other(4_100)],
        ),
        (
            "while a recording runs nothing lifts the guard on its Send",
            idle,
            defaults,
            vec![activate(100, 0), other(300), activate(400, 300)],
        ),
        (
            "a tap with the microphone never asked: the prompt, then Allow records",
            idle,
            defaults,
            vec![activate_with(100, not_asked), answer(4_000, true)],
        ),
        ("a tap with the microphone never asked: Don't Allow", idle, defaults, vec![activate_with(100, not_asked), answer(4_000, false)]),
        (
            "Record Voice Message with the microphone never asked, beside a draft",
            idle,
            defaults,
            vec![record(0, true, not_asked, 0), answer(4_000, true)],
        ),
        ("a denied microphone, from a tap", idle, defaults, vec![activate_with(0, denied)]),
        ("a denied microphone, from the paperclip", idle, defaults, vec![record(0, false, denied, 0)]),
        (
            "dimmed by a call: a tap explains, before any prompt",
            idle,
            defaults,
            vec![activate_with(100, Situation { blocked: Some(Dimmed::Call), permission: Permission::NotAsked })],
        ),
        (
            "dimmed while busy: the shortcut explains",
            idle,
            defaults,
            vec![record(0, false, Situation { blocked: Some(Dimmed::Busy), ..sit }, 0)],
        ),
        (
            "dimmed by a not-sent message: the paperclip explains",
            idle,
            defaults,
            vec![record(0, false, Situation { blocked: Some(Dimmed::NotSent), ..sit }, 0)],
        ),
        ("an interruption hands-free parks it", idle, defaults, vec![activate(100, 0), interruption(9_000, 8_900)]),
        ("an interruption hands-free under a second deletes it", idle, defaults, vec![activate(100, 0), interruption(900, 800)]),
        ("an interruption beside a draft parks it", idle, defaults, vec![record(0, true, sit, 0), interruption(5_000, 5_000)]),
        (
            "an interruption while the prompt is up abandons it",
            idle,
            defaults,
            vec![activate_with(0, not_asked), interruption(1_000, 0), answer(2_000, true)],
        ),
        ("five minutes, hands-free: review", idle, defaults, vec![activate(100, 0), HoldEvent::Cap { at_ms: 300_100 }]),
        ("five minutes beside a draft: review", idle, defaults, vec![record(0, true, sit, 0), HoldEvent::Cap { at_ms: 300_000 }]),
        ("Stop reviews", idle, defaults, vec![activate(100, 0), stop(9_000, 8_900)]),
        ("Stop under a second is too short", idle, defaults, vec![activate(100, 0), stop(1_000, 900)]),
        ("Magic Tap with nothing recording never starts one", idle, defaults, vec![stop(0, 0)]),
        ("Delete under ten seconds", idle, defaults, vec![activate(100, 0), delete(9_000, 9_999)]),
        (
            "Delete at ten seconds stops and asks; Delete",
            idle,
            defaults,
            vec![activate(100, 0), delete(10_200, 10_000), HoldEvent::Answer { at_ms: 11_000, delete: true }],
        ),
        (
            "Delete at ten seconds stops and asks; Keep",
            idle,
            defaults,
            vec![activate(100, 0), delete(12_200, 12_000), HoldEvent::Answer { at_ms: 13_000, delete: false }],
        ),
        ("an interruption while asking parks it", idle, defaults, vec![activate(100, 0), delete(12_200, 12_000), interruption(13_000, 12_000)]),
        (
            "Record Voice Message beside a draft: the slot is Stop, it stages the note, and a double tap cannot send it",
            idle,
            defaults,
            vec![record(1_000, true, sit, 0), activate(4_000, 3_000), activate(4_599, 0)],
        ),
        ("Record Voice Message beside a draft, stopped too soon", idle, defaults, vec![record(1_000, true, sit, 0), activate(1_600, 600)]),
        (
            "the shortcut starts, and pressed again stops into review",
            idle,
            defaults,
            vec![record(0, false, sit, 0), record(9_000, false, sit, 9_000)],
        ),
        ("the shortcut during a tapped recording stops into review, never sending", idle, defaults, vec![activate(0, 0), record(4_000, false, sit, 3_500)]),
    ];

    for (name, start, constants, events) in &scenarios {
        let mut state = *start;
        for (index, event) in events.iter().enumerate() {
            let (next, effects) = r::hold_step(state, *event, constants);
            let label = record_event(event)["event"].as_str().unwrap().to_string();
            case(
                &format!("voice: {name} — step {}: {label}", index + 1),
                "hold_step",
                json!({"state": record_state(&state), "event": record_event(event), "constants": record_constants(constants)}),
                json!({
                    "state": record_state(&next),
                    "effects": effects.iter().map(record_effect).collect::<Vec<_>>(),
                    "recording": record_recording(next.recording()),
                }),
            );
            state = next;
        }
    }

    // --- events out of place: nothing changes ---------------------------------------------------------
    let hands_free = HoldState { phase: Phase::HandsFree { beside_draft: false }, guard_until_ms: 700 };
    let asking = HoldState { phase: Phase::AskingDelete { recorded_ms: 12_000 }, ..idle };
    let prompting = HoldState { phase: Phase::AwaitingPermission { source: Source::Menu, beside_draft: true }, ..idle };
    let still: Vec<(&str, HoldState, HoldEvent)> = vec![
        ("Delete with nothing recording", idle, delete(10, 5_000)),
        ("an answer nobody was asked for", idle, HoldEvent::Answer { at_ms: 10, delete: true }),
        ("a permission answer with no prompt", idle, answer(10, true)),
        ("another action with nothing guarded", idle, other(10)),
        ("five minutes with nothing recording", idle, HoldEvent::Cap { at_ms: 10 }),
        ("an interruption with nothing recording", idle, interruption(10, 0)),
        ("another action while recording keeps the guard", hands_free, other(300)),
        ("an answer nobody was asked for, while recording", hands_free, HoldEvent::Answer { at_ms: 300, delete: true }),
        ("a permission answer while recording", hands_free, answer(300, true)),
        ("an Activate while asking", asking, activate(20_000, 0)),
        ("the shortcut while asking", asking, record(20_000, false, sit, 12_000)),
        ("Stop while asking", asking, stop(20_000, 12_000)),
        ("Delete while asking", asking, delete(20_000, 12_000)),
        ("five minutes while asking", asking, HoldEvent::Cap { at_ms: 20_000 }),
        ("an Activate while the prompt is up", prompting, activate(20_000, 0)),
        ("the paperclip while the prompt is up", prompting, record(20_000, false, sit, 0)),
        ("Stop while the prompt is up", prompting, stop(20_000, 0)),
        ("a delete answer while the prompt is up", prompting, HoldEvent::Answer { at_ms: 20_000, delete: true }),
    ];
    for (name, state, event) in &still {
        let (next, effects) = r::hold_step(*state, *event, &defaults);
        case(
            &format!("voice, out of place: {name}"),
            "hold_step",
            json!({"state": record_state(state), "event": record_event(event), "constants": record_constants(&defaults)}),
            json!({
                "state": record_state(&next),
                "effects": effects.iter().map(record_effect).collect::<Vec<_>>(),
                "recording": record_recording(next.recording()),
            }),
        );
    }

    let lines: Vec<String> = cases.iter().map(|case| serde_json::to_string(case).unwrap()).collect();
    println!("[\n  {}\n]", lines.join(",\n  "));
}

// --- waveform: a voice note's shape (fc_text::waveform) ---------------------------------------------

/// A peak as JSON. JSON has no NaN or infinity, so those three are the strings `"NaN"`,
/// `"Infinity"` and `"-Infinity"` — the spellings Swift's `Double(_:)`, Kotlin's `toDouble()` and
/// C#'s `double.Parse` (invariant culture) all read back. Everything else is the number, written as
/// the shortest decimal that round-trips; [`rate`] writes a whole one as an integer.
fn peak(value: f64) -> serde_json::Value {
    if value.is_nan() {
        serde_json::json!("NaN")
    } else if value == f64::INFINITY {
        serde_json::json!("Infinity")
    } else if value == f64::NEG_INFINITY {
        serde_json::json!("-Infinity")
    } else {
        rate(value)
    }
}

/// A deterministic "recording" of `n` peaks: an envelope that rises and falls with a ripple,
/// every value a multiple of 1/8 dB — exact in binary, so the input every port parses is the
/// input this printed — and some below the floor and above full scale.
fn synthetic_peaks(n: usize, seed: u64) -> Vec<f64> {
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..n)
        .map(|i| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let noise = ((state >> 33) % 121) as i64; // 0..=120 eighths: 0..15 dB
            let envelope = if n <= 1 {
                0
            } else {
                (i as i64 * (n as i64 - 1 - i as i64) * 4 * 8 * 30)
                    / ((n as i64 - 1) * (n as i64 - 1))
            };
            // −70 dB … +5 dB in eighths: below the floor at the ends, above full scale at the peak.
            let eighths = -560 + 2 * envelope + noise;
            eighths as f64 / 8.0
        })
        .collect()
}

fn waveform_vectors() {
    use fc_text::waveform as w;
    use serde_json::json;

    let mut cases: Vec<serde_json::Value> = Vec::new();
    let mut case =
        |name: &str, function: &str, input: serde_json::Value, expected: serde_json::Value| {
            cases.push(
                json!({"name": name, "function": function, "input": input, "expected": expected}),
            );
        };

    case(
        "the constants and the placeholder",
        "constants",
        json!({}),
        json!({
            "levels": w::LEVELS,
            "max_level": w::MAX_LEVEL,
            "floor_dbfs": rate(w::FLOOR_DBFS),
            "db_per_level": rate(w::DB_PER_LEVEL),
            "placeholder_level": w::PLACEHOLDER_LEVEL,
            "placeholder": w::encode(&w::PLACEHOLDER),
        }),
    );

    // --- level: one peak ------------------------------------------------------------------------
    let below = |value: f64| f64::from_bits(value.to_bits() + 1); // one ulp further from zero
    let above = |value: f64| f64::from_bits(value.to_bits() - 1); // one ulp nearer zero
    let mut peaks: Vec<(String, f64)> = vec![
        ("NaN is silence".into(), f64::NAN),
        ("+infinity is full scale".into(), f64::INFINITY),
        ("-infinity is silence".into(), f64::NEG_INFINITY),
        ("-160, a meter's floor".into(), -160.0),
        ("-120".into(), -120.0),
        ("-60.5, below the floor".into(), -60.5),
        ("above full scale: +3.5".into(), 3.5),
        ("+120".into(), 120.0),
        ("0".into(), 0.0),
        ("one ulp below -58, the first tie".into(), below(-58.0)),
        ("one ulp above -58".into(), above(-58.0)),
        ("2^-16 below -58".into(), -58.0 - 1.0 / 65_536.0),
        (
            "one ulp below -2: the addition rounds it onto the tie".into(),
            below(-2.0),
        ),
        ("2^-16 below -2".into(), -2.0 - 1.0 / 65_536.0),
        ("one ulp above -60".into(), above(-60.0)),
        ("one ulp below 0".into(), below(0.0)),
        (
            "one ulp above -0 (the least positive)".into(),
            f64::from_bits(1),
        ),
        ("-12.3, not exact in binary".into(), -12.3),
        ("-33.333333333333336".into(), -33.333333333333336),
        (
            "-45.1, a Float meter widened (f32 -> f64)".into(),
            f64::from(-45.1_f32),
        ),
    ];
    for k in 0..=15 {
        let centre = -60.0 + 4.0 * f64::from(k);
        peaks.push((format!("level {k}'s centre"), centre));
        peaks.push((
            format!("level {k}'s centre - 2 (a tie, rounds up)"),
            centre - 2.0,
        ));
        peaks.push((format!("level {k}'s centre + 1.875"), centre + 1.875));
        peaks.push((format!("level {k}'s centre - 1.875"), centre - 1.875));
    }
    for eighths in (-500..=16).step_by(3) {
        let value = f64::from(eighths) / 8.0;
        peaks.push((format!("{value} dBFS"), value));
    }
    for (name, value) in &peaks {
        case(
            &format!("level: {name}"),
            "level",
            json!({"dbfs": peak(*value)}),
            json!({"level": w::level(*value)}),
        );
    }

    // --- from_peaks: a recording's peaks to the wire ---------------------------------------------
    let mut recordings: Vec<(String, Vec<f64>, usize)> = vec![
        ("no peaks at all: silence".into(), vec![], 48),
        ("one peak covers every slice".into(), vec![-30.0], 48),
        ("two peaks, half each".into(), vec![-60.0, 0.0], 48),
        (
            "three peaks, a third each".into(),
            vec![-60.0, -30.0, 0.0],
            48,
        ),
        (
            "NaN and infinities among peaks".into(),
            vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -20.0],
            48,
        ),
        (
            "every peak a tie".into(),
            (0..48).map(|i| -58.0 + 4.0 * f64::from(i % 15)).collect(),
            48,
        ),
        (
            "a slice takes its loudest".into(),
            {
                let mut v = vec![-60.0; 96];
                v[1] = 0.0;
                v[94] = -30.0;
                v
            },
            48,
        ),
        ("count 0 is empty".into(), vec![-30.0, -20.0], 0),
        (
            "count 1 is the loudest".into(),
            vec![-50.0, -10.0, -40.0],
            1,
        ),
        ("a live meter's 22 bars".into(), synthetic_peaks(300, 7), 22),
    ];
    for n in [
        5usize, 47, 48, 49, 95, 96, 97, 100, 143, 144, 145, 480, 1_000, 3_001,
    ] {
        recordings.push((
            format!("{n} synthetic peaks"),
            synthetic_peaks(n, n as u64),
            48,
        ));
    }
    for (name, samples, count) in &recordings {
        case(
            &format!("from_peaks: {name}"),
            "from_peaks",
            json!({"samples_dbfs": samples.iter().map(|value| peak(*value)).collect::<Vec<_>>(), "levels": count}),
            json!({"waveform": w::from_peaks(samples, *count)}),
        );
    }

    // --- parse and the placeholder ---------------------------------------------------------------
    let wire = "0123456789abcdef0123456789abcdef0123456789abcdef";
    let parses: Vec<(&str, Option<String>)> = vec![
        ("every digit", Some(wire.to_string())),
        ("all zeros", Some("0".repeat(48))),
        ("all f", Some("f".repeat(48))),
        (
            "protocol.md's example",
            Some("0124689abcddeeedcba987654321001245678aabbba98642".to_string()),
        ),
        ("the placeholder", Some("4".repeat(48))),
        ("empty", Some(String::new())),
        ("47", Some("0".repeat(47))),
        ("49", Some("0".repeat(49))),
        ("96", Some("0".repeat(96))),
        ("uppercase", Some(wire.to_uppercase())),
        ("one uppercase F", Some(format!("{}F", &wire[..47]))),
        ("g", Some(format!("{}g", &wire[..47]))),
        ("a trailing space", Some(format!("{} ", &wire[..47]))),
        ("a leading space", Some(format!(" {}", &wire[..47]))),
        ("a newline", Some(format!("{}\n", &wire[..47]))),
        ("a comma", Some(format!("{},", &wire[..47]))),
        ("a minus", Some(format!("{}-", &wire[..47]))),
        ("48 bytes, 47 characters", Some(format!("{}é", &wire[..46]))),
        ("48 characters, 49 bytes", Some(format!("{}٣", &wire[..47]))),
        ("a fullwidth digit", Some(format!("{}０", &wire[..45]))),
        ("absent", None),
    ];
    for (name, value) in &parses {
        if let Some(value) = value {
            case(
                &format!("parse: {name}"),
                "parse",
                json!({"waveform": value}),
                json!({"levels": w::parse(value).map(|levels| levels.to_vec())}),
            );
        }
        case(
            &format!("levels_or_placeholder: {name}"),
            "levels_or_placeholder",
            json!({"waveform": value}),
            json!({"levels": w::levels_or_placeholder(value.as_deref()).to_vec()}),
        );
    }

    // --- drawing: bars, heights, the played part -------------------------------------------------
    let shapes: Vec<(&str, Vec<u8>)> = vec![
        ("every digit", w::parse(wire).unwrap().to_vec()),
        (
            "protocol.md's example",
            w::parse("0124689abcddeeedcba987654321001245678aabbba98642")
                .unwrap()
                .to_vec(),
        ),
        ("the placeholder", w::PLACEHOLDER.to_vec()),
    ];
    for (name, levels) in &shapes {
        for count in [
            0usize, 1, 2, 3, 10, 16, 22, 24, 34, 44, 47, 48, 49, 64, 96, 100,
        ] {
            case(
                &format!("bars: {name} as {count}"),
                "bars",
                json!({"levels": levels, "count": count}),
                json!({"bars": w::bars(levels, count)}),
            );
        }
    }
    case(
        "bars: no levels is silence",
        "bars",
        json!({"levels": [], "count": 5}),
        json!({"bars": w::bars(&[], 5)}),
    );
    case(
        "bars: a level above 15 reads as 15",
        "bars",
        json!({"levels": [200, 3, 16], "count": 3}),
        json!({"bars": w::bars(&[200, 3, 16], 3)}),
    );
    for level in [0u8, 1, 2, 3, 4, 5, 7, 8, 10, 14, 15, 16, 255] {
        case(
            &format!("bar_fraction: {level}"),
            "bar_fraction",
            json!({"level": level}),
            json!({"fraction": rate(w::bar_fraction(level))}),
        );
    }
    let positions: Vec<(u64, u64, usize)> = vec![
        (0, 14_200, 44),
        (1, 14_200, 44),
        (322, 14_200, 44),
        (323, 14_200, 44),
        (7_100, 14_200, 44),
        (14_199, 14_200, 44),
        (14_200, 14_200, 44),
        (99_999, 14_200, 44),
        (5_000, 0, 44),
        (0, 0, 48),
        (1_000, 3_000, 0),
        (2_999, 3_000, 48),
        (300_000, 300_000, 48),
        (149_999, 300_000, 48),
        (u64::MAX / 2, u64::MAX / 2 + 1, 48),
    ];
    for (position_ms, duration_ms, bars) in positions {
        case(
            &format!("played_bars: {position_ms} of {duration_ms} ms, {bars} bars"),
            "played_bars",
            json!({"position_ms": position_ms, "duration_ms": duration_ms, "bars": bars}),
            json!({"played": w::played_bars(position_ms, duration_ms, bars)}),
        );
    }

    let lines: Vec<String> = cases
        .iter()
        .map(|case| serde_json::to_string(case).unwrap())
        .collect();
    println!("[\n  {}\n]", lines.join(",\n  "));
}
