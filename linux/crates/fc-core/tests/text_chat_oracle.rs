//! `ChatOracleTests`, THE SECOND DIFFERENTIAL ORACLE: the words a chat ROW is drawn with. Every
//! vector in `fixtures/chat-vectors.json` was printed by `win/tools/board-oracle` from `fc_text`
//! (and, for the poll sections, from Rust's own std, as the server reads it), and the C# port is
//! held to it string for string.
//!
//! On Linux most of these rules ARE `fc_text`, so a case here holds the `fc_text` this build links
//! to the file Windows is held to: a vector that no longer matches is a divergence between the two
//! clients (a stale file, or an `fc_text` that moved), never something to "fix" here. The rest —
//! the polls, the decimal separator, the calendar stamp, the plural categories by tag — hold
//! `fc_core::text`.
//!
//! Not here: `APlaceIsUploadedWithTheOriginalsQuery` is `UploadQueries.Location`, the `protocol`
//! port's. The fixture's text is in English: `fc_text::i18n`'s reader language is English unless a
//! test sets it, and none here does.

mod common;

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use time::macros::datetime;

use common::{
    array, boolean, float, int, ints, optional_float, optional_int, optional_string, optional_uint,
    quoted, string, strings, uint,
};
use fc_core::text::calendar::stamp;
use fc_core::text::media_text;
use fc_core::text::plurals::PluralRules;
use fc_core::text::polls;
use fc_text::account::{self, CapState};
use fc_text::assistant;
use fc_text::assistant_pictures::{self, Candidate, MentionNotice, Switches};
use fc_text::avatar;
use fc_text::emoji;
use fc_text::media::{self, Route};
use fc_text::mentions::{self, Member};
use fc_text::reactions::{self, Reaction};

fn vectors() -> &'static Value {
    use std::sync::OnceLock;
    static VECTORS: OnceLock<Value> = OnceLock::new();
    VECTORS.get_or_init(|| common::fixture_json("chat-vectors.json"))
}

fn section(name: &str) -> &'static Vec<Value> {
    vectors()[name]
        .as_array()
        .unwrap_or_else(|| panic!("no section {name}"))
}

fn candidates(list: &Value) -> Vec<Candidate> {
    list.as_array()
        .expect("candidates")
        .iter()
        .map(|candidate| {
            Candidate::new(
                &string(candidate, "kind"),
                &string(candidate, "mime"),
                optional_uint(candidate, "bytes"),
            )
        })
        .collect()
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex"))
        .collect()
}

/// @ai and /draw: whether a body reaches the assistant and whether it asks for a picture — the
/// server's grammar, body for body.
#[test]
fn what_a_body_asks_the_assistant_for_is_the_servers_grammar() {
    assert!(!section("assistant_bodies").is_empty());
    for row in section("assistant_bodies") {
        let body = string(row, "body");
        assert_eq!(
            boolean(row, "mentions"),
            assistant::mentions(&body),
            "{body:?}"
        );
        assert_eq!(
            optional_string(row, "prompt").as_deref(),
            assistant::draw_prompt(&body),
            "{body:?}"
        );
        assert_eq!(
            boolean(row, "asks"),
            assistant::asks_for_picture(&body),
            "{body:?}"
        );
    }
    for row in section("assistant_mention_drafts") {
        assert_eq!(
            string(row, "with"),
            assistant::with_assistant_mention(&string(row, "draft"))
        );
    }
    assert!(section("draw_token_drafts").len() > 20);
    for row in section("draw_token_drafts") {
        let draft = string(row, "draft");
        assert_eq!(
            string(row, "with"),
            assistant::with_draw_token(&draft),
            "draw token for {}",
            quoted(&draft)
        );
    }
}

/// A draft that has BEGUN a picture request is exactly one that would ask for a picture the
/// moment a description is typed after it.
#[test]
fn a_begun_picture_request_is_one_a_description_would_complete() {
    let mut drafts: Vec<String> = Vec::new();
    drafts.extend(
        section("assistant_bodies")
            .iter()
            .map(|row| string(row, "body")),
    );
    drafts.extend(
        section("draw_token_drafts")
            .iter()
            .map(|row| string(row, "draft")),
    );
    drafts.extend(
        section("draw_token_drafts")
            .iter()
            .map(|row| string(row, "with")),
    );
    drafts.extend(
        [
            "/draw ",
            "  /draw ",
            "/DRAW\n",
            "@ai /draw ",
            " /draw ",
            "/draw",
            "/drawer ",
            "x /draw ",
            "hey @ai /draw ",
            "@ai @ai /draw ",
            "",
        ]
        .map(str::to_string),
    );
    for draft in &drafts {
        assert_eq!(
            assistant::asks_for_picture(&format!("{draft}x")),
            assistant::starts_picture_request(draft),
            "begun picture request for {}",
            quoted(draft)
        );
        // A request is always a begun one.
        assert!(
            !assistant::asks_for_picture(draft) || assistant::starts_picture_request(draft),
            "{}",
            quoted(draft)
        );
    }
    assert!(assistant::starts_picture_request("/draw "));
    assert!(assistant::starts_picture_request(" \t@AI  /Draw \n"));
    assert!(!assistant::starts_picture_request("/draw"));
    assert!(!assistant::starts_picture_request("/drawer a cat"));
    assert!(!assistant::starts_picture_request("look /draw a cat"));
}

/// The assistant's own chat: what its composer says of the photos staged.
#[test]
fn the_private_chats_picture_notice_is_the_originals() {
    assert!(!section("pictures_private").is_empty());
    for row in section("pictures_private") {
        assert_eq!(
            optional_string(row, "said"),
            assistant_pictures::private_notice(
                &candidates(&row["staged"]),
                boolean(row, "can_see")
            ),
            "{row}"
        );
    }
}

/// The family chat: whether an @ai draft's notice shows, what it counts, and what it says — every
/// lock and switch.
#[test]
fn the_family_chats_picture_notice_is_the_originals() {
    let rows = section("pictures_mention");
    assert!(rows.len() > 100);
    for row in rows {
        let flags: Vec<bool> = array(row, "switches")
            .iter()
            .map(|flag| flag.as_bool().expect("a flag"))
            .collect();
        let notice = MentionNotice::of(
            &string(row, "draft"),
            &candidates(&row["staged"]),
            &candidates(&row["quoted"]),
            Switches {
                server_can_see: flags[0],
                family_allows: flags[1],
                family_history: flags[2],
                family_history_photos: flags[3],
                server_can_draw: flags[4],
            },
        );
        assert_eq!(
            optional_string(row, "said"),
            notice.as_ref().map(MentionNotice::sentence),
            "{row}"
        );
        if row["counts"].is_null() {
            assert_eq!(notice, None, "{row}");
            continue;
        }
        let notice = notice.expect("a notice");
        let expected: Vec<usize> = ints(row, "counts")
            .into_iter()
            .map(|count| count as usize)
            .collect();
        assert_eq!(
            expected,
            [
                notice.shown_on_mention,
                notice.shown_on_quote,
                notice.extra,
                notice.unreadable
            ],
            "{row}"
        );
        assert_eq!(
            optional_int(row, "recent"),
            notice.recent_up_to.map(|recent| recent as i64),
            "{row}"
        );
    }
}

#[test]
fn what_reaches_the_model_is_decided_as_the_original_decides_it() {
    for row in section("pictures_shown") {
        assert_eq!(
            boolean(row, "shown"),
            assistant_pictures::is_shown_to_model(
                &string(row, "kind"),
                &string(row, "mime"),
                optional_uint(row, "bytes")
            ),
            "{row}"
        );
    }
    for row in section("pictures_attachment") {
        let candidate = Candidate::of_attachment(
            &string(row, "kind"),
            &string(row, "mime"),
            optional_uint(row, "size"),
            boolean(row, "preview"),
        );
        let expected = &row["as"];
        assert_eq!(
            Candidate::new(
                &string(expected, "kind"),
                &string(expected, "mime"),
                optional_uint(expected, "bytes")
            ),
            candidate,
            "{row}"
        );
    }
    for row in section("pictures_offer") {
        assert_eq!(
            boolean(row, "offers"),
            assistant_pictures::offers_picture_attach(
                boolean(row, "chat"),
                boolean(row, "see"),
                boolean(row, "allows")
            ),
            "{row}"
        );
    }
}

/// Words win, except copied files and their names — the original's decision, paste for paste.
#[test]
fn a_paste_attaches_types_or_does_nothing_as_the_original_decides() {
    assert!(!section("paste_decision").is_empty());
    for row in section("paste_decision") {
        assert_eq!(
            string(row, "said"),
            format!(
                "{:?}",
                media::paste_decision(&strings(row, "names"), &string(row, "text"))
            ),
            "{row}"
        );
    }
}

#[test]
fn a_pasted_item_is_taken_and_named_as_the_original_takes_it() {
    for row in section("paste_type") {
        assert_eq!(
            optional_string(row, "chosen"),
            media::chosen_paste_type(&strings(row, "offered")),
            "{row}"
        );
    }
    assert!(!section("pasted_name").is_empty());
    for row in section("pasted_name") {
        assert_eq!(
            string(row, "name"),
            media::pasted_name(&string(row, "mime")),
            "{row}"
        );
    }
}

/// A recording's time, "3:42" — the original's label, including for a time that is not a number.
#[test]
fn a_recordings_time_is_labelled_as_the_original() {
    assert!(!section("time_label").is_empty());
    for row in section("time_label") {
        let seconds = match &row["seconds"] {
            Value::Number(number) => number.as_f64().expect("a number"),
            Value::String(word) => match word.as_str() {
                "inf" => f64::INFINITY,
                "-inf" => f64::NEG_INFINITY,
                _ => f64::NAN,
            },
            other => panic!("seconds {other}"),
        };
        assert_eq!(string(row, "label"), media::time_label(seconds), "{row}");
    }
}

/// Whether a message is nothing but emoji, how many, and how large it draws — the original's
/// answer, text for text.
#[test]
fn an_emoji_only_message_is_counted_and_sized_as_the_original() {
    assert!(!section("emoji_only").is_empty());
    for row in section("emoji_only") {
        let text = string(row, "text");
        assert_eq!(
            optional_int(row, "count").map(|count| count as usize),
            emoji::emoji_only_count(&text),
            "{}",
            quoted(&text)
        );
        assert_eq!(optional_float(row, "size"), emoji::display_font_size(&text));
        assert_eq!(
            optional_float(row, "at13"),
            emoji::display_font_size_for_body(&text, 13.0)
        );
    }
}

/// The "More reactions…" catalogue is the original's: the same sections, the same order, the same
/// bytes.
#[test]
fn the_emoji_catalogue_is_the_originals() {
    let sections = section("emoji_catalog");
    let names: Vec<String> = sections.iter().map(|row| string(row, "name")).collect();
    let catalog_names: Vec<&str> = emoji::EMOJI_CATALOG
        .iter()
        .map(|category| category.name)
        .collect();
    assert_eq!(names, catalog_names);
    for (row, category) in sections.iter().zip(emoji::EMOJI_CATALOG.iter()) {
        assert_eq!(strings(row, "emoji"), category.emoji);
    }
    assert_eq!(
        emoji::EMOJI_CATALOG
            .iter()
            .map(|category| category.emoji.len())
            .sum::<usize>(),
        771
    );
}

/// A profile circle's initials: two words' first graphemes, uppercased — the original's, title for
/// title.
#[test]
fn an_avatars_initials_are_the_originals() {
    assert!(!section("avatar_initials").is_empty());
    for row in section("avatar_initials") {
        assert_eq!(
            string(row, "said"),
            avatar::initials(&string(row, "title")),
            "{row}"
        );
    }
}

/// EVERY scalar Rust uppercases, uppercased the same — the full mapping. On Linux this is
/// `str::to_uppercase` itself: what it holds is that the rustc this builds with has the Unicode
/// tables the oracle printed with. Collected rather than asserted one at a time, so a
/// disagreement shows all of itself at once.
#[test]
fn every_letter_uppercases_as_rust_uppercases_it() {
    let rows = section("avatar_upper");
    assert!(rows.len() > 1000, "only {} scalars", rows.len());
    let wrong: Vec<String> = rows
        .iter()
        .filter_map(|row| {
            let scalar = char::from_u32(int(row, "c") as u32).expect("a scalar");
            (scalar.to_uppercase().collect::<String>() != string(row, "upper"))
                .then(|| format!("U+{:04X}", scalar as u32))
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{} differ: {}",
        wrong.len(),
        wrong.iter().take(60).cloned().collect::<Vec<_>>().join(" ")
    );
}

/// A poll option is trimmed as the server trims it: Rust's White_Space, no more and no less.
#[test]
fn a_poll_option_is_trimmed_the_way_rust_trims() {
    assert!(!section("poll_trim").is_empty());
    for row in section("poll_trim") {
        assert_eq!(
            string(row, "trimmed"),
            polls::trimmed(&string(row, "text")),
            "{row}"
        );
    }
}

/// Two options are the same when Rust's `str::to_lowercase` says so — a word-final capital sigma
/// and a dotted capital I included.
#[test]
fn a_poll_option_is_folded_the_way_rust_lowercases() {
    // Every scalar with a mapping of its own and every one Final_Sigma skips or counts: the
    // casing tables differ by Unicode version, and only a sweep catches that.
    assert!(section("poll_lower").len() > 10_000, "the sweep is missing");
    let wrong: Vec<String> = section("poll_lower")
        .iter()
        .filter(|row| polls::lowercase(&string(row, "text")) != string(row, "lower"))
        .map(|row| {
            string(row, "text")
                .chars()
                .map(|c| format!("U+{:04X}", c as u32))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{} differ: {}",
        wrong.len(),
        wrong
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join(" | ")
    );
}

/// The composer refuses exactly what the server would answer `invalid_poll`, and sends what it
/// checked.
#[test]
fn a_poll_is_refused_exactly_where_the_server_refuses_it() {
    assert!(!section("poll_options").is_empty());
    for row in section("poll_options") {
        let options = strings(row, "options");
        let sent = match &row["sent"] {
            Value::Null => None,
            _ => Some(strings(row, "sent")),
        };
        assert_eq!(sent, polls::sanitized(&options), "{row}");
    }
}

/// A record's line, for every outcome the wire names and one it does not, at every duration and
/// on both sides of the call.
#[test]
fn a_call_record_says_what_the_original_says() {
    assert!(!section("call_record").is_empty());
    for row in section("call_record") {
        assert_eq!(
            string(row, "said"),
            fc_text::call_record::label(
                &string(row, "outcome"),
                optional_int(row, "duration_secs"),
                boolean(row, "video"),
                boolean(row, "mine")
            ),
            "{row}"
        );
    }
}

#[test]
fn a_duration_is_clocked_the_same_way() {
    for row in section("duration") {
        assert_eq!(
            string(row, "said"),
            fc_text::call_record::duration(int(row, "seconds")),
            "{row}"
        );
    }
}

#[test]
fn an_attachment_with_no_name_is_called_what_the_original_calls_it() {
    for row in section("display_name") {
        let name = optional_string(row, "name");
        assert_eq!(
            string(row, "said"),
            media::display_name(&string(row, "kind"), name.as_deref()),
            "{row}"
        );
    }
}

#[test]
fn a_notifications_title_reads_the_same() {
    for row in section("notify_title") {
        let family = optional_string(row, "family");
        assert_eq!(
            string(row, "said"),
            fc_text::notify::title(
                family.as_deref(),
                &string(row, "sender"),
                boolean(row, "mentioned")
            ),
            "{row}"
        );
    }
}

#[test]
fn the_window_title_counts_the_same_way() {
    for row in section("page_title") {
        assert_eq!(
            string(row, "said"),
            fc_text::notify::page_title("Family Connect", int(row, "unread")),
            "{row}"
        );
    }
}

/// THE `.ics` FILE, BYTE FOR BYTE: CRLF, escaping, folding at 75 OCTETS without splitting a UTF-8
/// sequence.
#[test]
fn an_events_calendar_file_is_the_originals_byte_for_byte() {
    assert!(!section("ics").is_empty());
    for row in section("ics") {
        let ends_at = optional_string(row, "ends_at");
        let place = optional_string(row, "place");
        assert_eq!(
            string(row, "file"),
            fc_text::calendar::one_event(
                &string(row, "uid"),
                &string(row, "title"),
                &string(row, "starts_at"),
                ends_at.as_deref(),
                place.as_deref(),
                "20260912T120000Z"
            ),
            "{row}"
        );
    }
}

#[test]
fn an_instant_is_stamped_the_way_i_calendar_wants_it() {
    // The wire carries an instant; a calendar file wants UTC and says so with the Z.
    assert_eq!(
        stamp(datetime!(2026-12-24 19:00:00 +03:00)),
        "20261224T160000Z"
    );
}

#[test]
fn the_notification_bodies_are_the_originals() {
    let bodies = &vectors()["bodies"];
    assert_eq!(
        string(bodies, "new_message"),
        fc_text::notify::new_message()
    );
    assert_eq!(string(bodies, "new_note"), fc_text::notify::new_note());
}

fn reaction_list(list: &Value) -> Vec<Reaction> {
    list.as_array()
        .expect("reactions")
        .iter()
        .map(|item| Reaction::new(int(item, "user_id"), &string(item, "emoji")))
        .collect()
}

/// The chips under a bubble: first-seen order, counting everybody, marking the reader — two
/// spellings of one letter equal only canonically are two chips, as on the server and Android.
#[test]
fn reaction_chips_are_grouped_as_the_original_groups_them() {
    assert!(!section("reaction_chips").is_empty());
    for row in section("reaction_chips") {
        let chips = reactions::reaction_chips(&reaction_list(&row["reactions"]), int(row, "me"));
        let expected = array(row, "chips");
        assert_eq!(expected.len(), chips.len(), "{row}");
        for (want, chip) in expected.iter().zip(&chips) {
            assert_eq!(string(want, "emoji"), chip.emoji, "{row}");
            assert_eq!(int(want, "count") as usize, chip.count, "{row}");
            assert_eq!(boolean(want, "includes_me"), chip.includes_me, "{row}");
        }
    }
}

/// "See who reacted": "You" first, names or "Someone", and a BLOCKED reactor left out of the names
/// while the chip still counts them.
#[test]
fn who_reacted_reads_as_the_original_reads() {
    assert!(!section("reaction_details").is_empty());
    let names: HashMap<i64, String> =
        HashMap::from([(9, "Anna".to_string()), (11, "Bob".to_string())]);
    for row in section("reaction_details") {
        let blocked: HashSet<i64> = ints(row, "blocked").into_iter().collect();
        let details = reactions::reaction_details(
            &reaction_list(&row["reactions"]),
            &names,
            int(row, "me"),
            &blocked,
        );
        let expected = array(row, "details");
        assert_eq!(expected.len(), details.len(), "{row}");
        for (want, detail) in expected.iter().zip(&details) {
            assert_eq!(string(want, "emoji"), detail.emoji, "{row}");
            assert_eq!(strings(want, "names"), detail.names, "{row}");
            assert_eq!(
                optional_int(want, "lead_user_id"),
                detail.lead_user_id,
                "{row}"
            );
        }
    }
}

/// A tap on the emoji already held removes it; any other appends it as the newest.
#[test]
fn a_toggle_rewrites_the_list_as_the_original_does() {
    assert!(!section("reaction_toggle").is_empty());
    for row in section("reaction_toggle") {
        let toggled = reactions::toggle_reaction(
            &reaction_list(&row["reactions"]),
            int(row, "me"),
            &string(row, "emoji"),
        );
        assert_eq!(boolean(row, "removing"), toggled.removing, "{row}");
        assert_eq!(reaction_list(&row["after"]), toggled.reactions, "{row}");
    }
}

#[test]
fn the_capsule_offers_what_the_original_offers() {
    for row in section("capsule") {
        let mine = optional_string(row, "mine");
        assert_eq!(
            strings(row, "emojis"),
            emoji::capsule_emojis(mine.as_deref()),
            "{row}"
        );
    }
}

/// The reply quote is cut per SCALAR, as the server cuts it.
#[test]
fn a_quote_is_cut_where_the_server_cuts_it() {
    assert!(!section("excerpt").is_empty());
    for row in section("excerpt") {
        assert_eq!(
            string(row, "excerpt"),
            fc_text::excerpt::excerpt(&string(row, "body")),
            "{row}"
        );
    }
}

/// A file's size, at every threshold where the unit or the rounding changes (the C# passes the
/// invariant culture: a point).
#[test]
fn a_file_size_reads_as_the_original_reads() {
    assert!(!section("display_size").is_empty());
    for row in section("display_size") {
        assert_eq!(
            string(row, "said"),
            media_text::display_size(int(row, "bytes"), "."),
            "{row}"
        );
    }
}

/// A size is for a person: the reader's decimal separator, the same arithmetic.
#[test]
fn a_file_size_uses_the_readers_decimal_separator() {
    assert_eq!(media_text::display_size(1_250_000, ","), "1,2 MB");
    assert_eq!(media_text::display_size(1_234_567_890, ","), "1,23 GB");
    assert_eq!(media_text::display_size(10_000_000_000, ","), "10 GB");
}

/// Tiles and album cards, from metadata alone — including sizes the uploader could not give.
#[test]
fn a_shape_is_measured_as_the_original_measures_it() {
    assert!(!section("shapes").is_empty());
    for row in section("shapes") {
        let (width, height) = (optional_int(row, "width"), optional_int(row, "height"));
        assert_eq!(
            float(row, "aspect"),
            media::aspect_ratio(width, height),
            "{row}"
        );
        let pair = |name: &str| {
            let values = array(row, name);
            (
                values[0].as_f64().expect("a width"),
                values[1].as_f64().expect("a height"),
            )
        };
        assert_eq!(pair("tile"), media::tile_size(width, height), "{row}");
        assert_eq!(
            pair("card"),
            media::card_size(width, height, 300.0),
            "{row}"
        );
        assert_eq!(
            pair("card_odd"),
            media::card_size(width, height, 250.625),
            "{row}"
        );
    }
}

#[test]
fn what_is_looked_at_is_what_the_original_looks_at() {
    for row in section("is_media") {
        assert_eq!(
            boolean(row, "is_media"),
            media::is_media(&string(row, "kind")),
            "{row}"
        );
    }
}

/// A place: coordinates always with a POINT, the accuracy only when it is a number, and the Maps
/// link percent-encoded byte by byte with the label trimmed — or "Location" when there is none.
#[test]
fn a_place_reads_and_links_as_the_original_does() {
    assert!(!section("locations").is_empty());
    for row in section("locations") {
        let latitude = float(row, "latitude");
        let longitude = float(row, "longitude");
        let accuracy = if boolean(row, "accuracy_nan") {
            Some(f64::NAN)
        } else {
            optional_float(row, "accuracy_m")
        };
        let name = optional_string(row, "name");
        assert_eq!(
            string(row, "line"),
            media::location_line(latitude, longitude, accuracy),
            "{row}"
        );
        assert_eq!(
            string(row, "maps"),
            media::maps_url(latitude, longitude, name.as_deref()),
            "{row}"
        );
    }
}

#[test]
fn a_files_type_is_read_as_the_original_reads_it() {
    assert!(!section("prep_types").is_empty());
    for row in section("prep_types") {
        let mime = string(row, "mime");
        let name = string(row, "name");
        assert_eq!(string(row, "essence"), media::essence(&mime), "{row}");
        assert_eq!(string(row, "extension"), media::extension(&name), "{row}");
        assert_eq!(string(row, "mime_for"), media::mime_for(&name), "{row}");
        assert_eq!(
            string(row, "declared"),
            media::declared_type(&mime, &name),
            "{row}"
        );
        assert_eq!(
            optional_string(row, "audio").as_deref(),
            media::audio_mime(&media::essence(&mime), &name),
            "{row}"
        );
    }
}

fn heads() -> HashMap<String, Vec<u8>> {
    section("prep_heads")
        .iter()
        .map(|row| (string(row, "label"), hex(&string(row, "hex"))))
        .collect()
}

/// Which kind a picked file goes as — a video or a recording only when the server will take its
/// BYTES as one.
#[test]
fn a_picked_file_takes_the_route_the_original_gives_it() {
    let heads = heads();
    assert!(!section("prep_route").is_empty());
    for row in section("prep_route") {
        let routed = media::route(
            &string(row, "mime"),
            &string(row, "name"),
            &heads[&string(row, "head")],
        );
        let said = match routed {
            Route::Photo => "photo".to_string(),
            Route::Video => "video".to_string(),
            Route::Audio(mime) => format!("audio:{mime}"),
            Route::File => "file".to_string(),
        };
        assert_eq!(string(row, "route"), said, "{row}");
    }
}

#[test]
fn the_magic_numbers_are_the_servers_own() {
    let heads = heads();
    for row in section("prep_magic") {
        assert_eq!(
            boolean(row, "matches"),
            media::matches_magic(&string(row, "mime"), &heads[&string(row, "head")]),
            "{row}"
        );
    }
}

/// A name for somebody else's disk: separators and control or format characters replaced,
/// trimmed, and a long one cut in its STEM so the extension survives.
#[test]
fn a_name_is_cleaned_as_the_original_cleans_it() {
    assert!(!section("prep_names").is_empty());
    for row in section("prep_names") {
        assert_eq!(
            optional_string(row, "clean"),
            media::sanitized_name(&string(row, "raw")),
            "{row}"
        );
    }
}

#[test]
fn a_picture_is_fitted_as_the_original_fits_it() {
    for row in section("prep_fit") {
        let fit = media::fit_within(
            uint(row, "width") as u32,
            uint(row, "height") as u32,
            uint(row, "edge") as u32,
        );
        let expected = array(row, "fit");
        assert_eq!(
            (
                expected[0].as_u64().expect("a width") as u32,
                expected[1].as_u64().expect("a height") as u32
            ),
            fit,
            "{row}"
        );
    }
}

#[test]
fn plural_categories_are_the_originals() {
    assert!(!section("plural_categories").is_empty());
    for row in section("plural_categories") {
        assert_eq!(
            string(row, "category"),
            PluralRules::category(&string(row, "lang"), int(row, "count")),
            "{row}"
        );
    }
}

#[test]
fn a_profile_picture_is_the_originals_square() {
    assert!(!section("avatar_square").is_empty());
    for row in section("avatar_square") {
        let square = avatar::square(uint(row, "width") as u32, uint(row, "height") as u32);
        let said = &row["square"];
        if said.is_null() {
            assert_eq!(square, None, "{row}");
            continue;
        }
        assert_eq!(
            Some(avatar::Square {
                x: uint(said, "x") as u32,
                y: uint(said, "y") as u32,
                side: uint(said, "side") as u32,
                edge: uint(said, "edge") as u32,
            }),
            square,
            "{row}"
        );
    }
    let budget = &vectors()["avatar_budget"];
    assert_eq!(uint(budget, "edge") as u32, avatar::EDGE);
    assert_eq!(uint(budget, "max_bytes") as usize, avatar::MAX_BYTES);
    let qualities: Vec<f64> = array(budget, "qualities")
        .iter()
        .map(|quality| quality.as_f64().expect("a quality"))
        .collect();
    assert_eq!(qualities, avatar::QUALITIES);
}

/// The member limit's three states, the clamp and the seed, as the original computes them.
#[test]
fn the_member_limit_is_the_originals() {
    assert!(!section("house_caps").is_empty());
    for row in section("house_caps") {
        let state = account::cap_state(
            optional_int(row, "cap"),
            int(row, "members"),
            int(row, "ceiling"),
        );
        let said = &row["state"];
        match string(said, "kind").as_str() {
            "open" => assert_eq!(
                state,
                CapState::OpenToCeiling {
                    ceiling: int(said, "ceiling")
                },
                "{row}"
            ),
            "frozen" => assert_eq!(
                state,
                CapState::Frozen {
                    members: int(said, "members")
                },
                "{row}"
            ),
            _ => assert_eq!(
                state,
                CapState::Room {
                    members: int(said, "members"),
                    seats: int(said, "seats")
                },
                "{row}"
            ),
        }
    }
    for row in section("house_clamps") {
        let (value, ceiling) = (int(row, "value"), int(row, "ceiling"));
        assert_eq!(
            int(row, "clamp"),
            account::clamp_cap(value, ceiling),
            "{row}"
        );
        assert_eq!(int(row, "seed"), account::seed_cap(value, ceiling), "{row}");
    }
    let languages: Vec<(String, String)> = section("house_languages")
        .iter()
        .map(|row| (string(row, "tag"), string(row, "name")))
        .collect();
    let held: Vec<(String, String)> = account::LANGUAGES
        .iter()
        .map(|(tag, name)| (tag.to_string(), name.to_string()))
        .collect();
    assert_eq!(languages, held);
    assert_eq!(
        assistant_pictures::MAX_PER_QUESTION as u64,
        vectors()["house_pictures_per_question"]
            .as_u64()
            .expect("a count")
    );
}

/// The server's byte grammar and the grapheme widening, as the original finds them.
#[test]
fn a_mention_is_found_where_the_original_finds_it() {
    assert!(!section("mention_ranges").is_empty());
    for row in section("mention_ranges") {
        let body = string(row, "body");
        let name = string(row, "name");
        let expected: Vec<(usize, usize)> = array(row, "ranges")
            .iter()
            .map(|pair| {
                (
                    pair[0].as_u64().expect("a start") as usize,
                    pair[1].as_u64().expect("an end") as usize,
                )
            })
            .collect();
        let found: Vec<(usize, usize)> = mentions::ranges(&body, &name)
            .into_iter()
            .map(|range| (range.start, range.end))
            .collect();
        assert_eq!(expected, found, "{row}");
        assert_eq!(!expected.is_empty(), mentions::names(&body, &name), "{row}");
    }
}

fn roster(rows: &'static [Value]) -> Vec<Member<'static>> {
    rows.iter()
        .map(|row| Member {
            user_id: int(row, "id"),
            name: row["name"].as_str().expect("a name"),
        })
        .collect()
}

#[test]
fn the_names_a_text_carries_are_resolved_as_the_original_resolves_them() {
    let members = roster(section("mention_roster"));
    for row in section("mention_resolve") {
        let ids: Vec<i64> = mentions::resolve(&string(row, "body"), &members)
            .into_iter()
            .map(|member| member.user_id)
            .collect();
        assert_eq!(ints(row, "ids"), ids, "{row}");
    }
    for row in section("mention_tokens") {
        let named = roster(array(row, "mentions"));
        let expected: Vec<(usize, usize, i64)> = array(row, "tokens")
            .iter()
            .map(|token| {
                (
                    token[0].as_u64().expect("a start") as usize,
                    token[1].as_u64().expect("an end") as usize,
                    token[2].as_i64().expect("an id"),
                )
            })
            .collect();
        let found: Vec<(usize, usize, i64)> = mentions::tokens(&string(row, "text"), &named)
            .into_iter()
            .map(|token| (token.range.start, token.range.end, token.member.user_id))
            .collect();
        assert_eq!(expected, found, "{row}");
    }
}

/// The composer's strip — the @ being typed, who it could mean, and the name accepted — as the
/// original has it.
#[test]
fn the_composers_strip_reads_a_draft_as_the_original_reads_it() {
    assert!(!section("strip_query").is_empty());
    for row in section("strip_query") {
        assert_eq!(
            optional_string(row, "query").as_deref(),
            mentions::query(&string(row, "draft")),
            "{row}"
        );
    }
    let members = roster(section("strip_roster"));
    for row in section("strip_candidates") {
        let ids: Vec<i64> =
            mentions::candidates(&members, &string(row, "query"), &ints(row, "excluding"))
                .into_iter()
                .map(|member| member.user_id)
                .collect();
        assert_eq!(ints(row, "ids"), ids, "{row}");
    }
    for row in section("strip_accept") {
        assert_eq!(
            string(row, "accepted"),
            mentions::accept(&string(row, "draft"), &string(row, "name")),
            "{row}"
        );
    }
}
