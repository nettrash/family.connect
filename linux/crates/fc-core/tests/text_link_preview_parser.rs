//! `LinkPreviewParserTests`: how a page turns into the card under a link — ios
//! `LinkPreviewParserTests` and `LinkPreviewYouTubeTests`, vector for vector (Android mirrors the
//! same ones): the Open Graph → Twitter → plain HTML ladder, the attribute forms real pages use,
//! entities, whitespace, clamping, relative images, no card without a title, and the head stop.

mod common;

use std::cell::Cell;
use std::convert::Infallible;
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::executor::LocalPool;
use futures::stream;

use fc_core::text::link_preview::{
    self, HeadEndDetector, LinkPreview, MAX_DESCRIPTION_LENGTH, MAX_TITLE_LENGTH, SCAN_LIMIT,
    decode_entities, display_host, page_reader,
};

const PAGE: &str = "https://example.com/articles/1";

fn parse(html: &str) -> Option<LinkPreview> {
    link_preview::parse(html, PAGE)
}

fn title(html: &str) -> Option<String> {
    parse(html).map(|preview| preview.title)
}

fn description(html: &str) -> Option<String> {
    parse(html).and_then(|preview| preview.description)
}

fn image(html: &str) -> Option<String> {
    parse(html).and_then(|preview| preview.image_url)
}

#[test]
fn open_graph_tags_win() {
    let preview = parse(
        r#"<html><head>
<title>Ignored</title>
<meta property="og:title" content="The Real Title">
<meta property="og:site_name" content="Example News">
<meta property="og:description" content="A short summary.">
<meta property="og:image" content="https://cdn.example.com/a.jpg">
</head><body>…</body></html>"#,
    )
    .expect("a card");
    assert_eq!(preview.title, "The Real Title");
    assert_eq!(preview.site_name, "Example News");
    assert_eq!(preview.description.as_deref(), Some("A short summary."));
    assert_eq!(
        preview.image_url.as_deref(),
        Some("https://cdn.example.com/a.jpg")
    );
}

#[test]
fn twitter_cards_are_the_second_choice_plain_html_the_third() {
    let twitter = parse(
        r#"<head><meta name="twitter:title" content="Twitter Title">
<meta name="twitter:description" content="Twitter summary">
<meta name="twitter:image" content="https://cdn.example.com/t.png"></head>"#,
    )
    .expect("a card");
    assert_eq!(twitter.title, "Twitter Title");
    assert_eq!(twitter.description.as_deref(), Some("Twitter summary"));
    assert_eq!(
        twitter.image_url.as_deref(),
        Some("https://cdn.example.com/t.png")
    );

    let plain = parse(
        r#"<head><title>Plain Title</title>
<meta name="description" content="Plain summary"></head>"#,
    )
    .expect("a card");
    assert_eq!(plain.title, "Plain Title");
    assert_eq!(plain.description.as_deref(), Some("Plain summary"));
    assert_eq!(plain.image_url, None);
}

/// With both on the page, Open Graph wins every field — whichever comes first in the markup.
#[test]
fn open_graph_beats_twitter_field_for_field() {
    let preview = parse(
        r#"<head>
<meta name="twitter:title" content="Twitter Title">
<meta name="twitter:description" content="Twitter summary">
<meta name="twitter:image" content="https://cdn.example.com/t.png">
<meta name="description" content="Plain summary">
<meta property="og:title" content="OG Title">
<meta property="og:description" content="OG summary">
<meta property="og:image" content="https://cdn.example.com/o.png">
</head>"#,
    )
    .expect("a card");
    assert_eq!(preview.title, "OG Title");
    assert_eq!(preview.description.as_deref(), Some("OG summary"));
    assert_eq!(
        preview.image_url.as_deref(),
        Some("https://cdn.example.com/o.png")
    );
    assert_eq!(
        description(
            r#"<head><title>T</title><meta name="description" content="Plain"><meta name="twitter:description" content="Twitter summary"></head>"#
        )
        .as_deref(),
        Some("Twitter summary")
    );
}

#[test]
fn no_title_means_no_card() {
    assert_eq!(parse("<head></head>"), None);
    assert_eq!(parse("<head><title>   </title></head>"), None);
    assert_eq!(parse(""), None);
}

#[test]
fn the_site_name_falls_back_to_the_host_without_www() {
    assert_eq!(
        link_preview::parse("<head><title>T</title></head>", "https://www.example.com/x")
            .map(|preview| preview.site_name)
            .as_deref(),
        Some("example.com")
    );
    // Android's own vector: a host a strict parser refuses is still the label.
    assert_eq!(
        display_host("https://my_host.example.com/x"),
        "my_host.example.com"
    );
}

#[test]
fn attribute_forms_real_pages_use() {
    // Single quotes, reversed order, self-closing, extra attributes.
    let preview = parse(
        r#"<head>
<meta content='Reversed Order' property='og:title' />
<meta charset="utf-8">
<meta data-rh="true" property="og:description" content="Desc" />
</head>"#,
    )
    .expect("a card");
    assert_eq!(preview.title, "Reversed Order");
    assert_eq!(preview.description.as_deref(), Some("Desc"));
}

#[test]
fn the_first_occurrence_of_a_key_wins() {
    assert_eq!(
        title(
            r#"<head><meta property="og:title" content="First">
<meta property="og:title" content="Second"></head>"#
        )
        .as_deref(),
        Some("First")
    );
}

#[test]
fn entities_are_decoded_and_whitespace_collapsed() {
    assert_eq!(
        title(
            "<head><meta property=\"og:title\"\ncontent=\"Tom &amp; Jerry&#39;s\n     big   day &hellip;\"></head>"
        )
        .as_deref(),
        Some("Tom & Jerry's big day …")
    );
}

#[test]
fn over_long_text_is_clamped_with_an_ellipsis() {
    let long = title(&format!(
        "<head><meta property=\"og:title\" content=\"{}\"></head>",
        "a".repeat(400)
    ))
    .unwrap_or_default();
    assert_eq!(long.encode_utf16().count(), MAX_TITLE_LENGTH + 1);
    assert!(long.ends_with('…'));
    let long = description(&format!(
        "<head><title>T</title><meta name=\"description\" content=\"{}\"></head>",
        "d".repeat(301)
    ))
    .unwrap_or_default();
    assert_eq!(long.encode_utf16().count(), MAX_DESCRIPTION_LENGTH + 1);
    assert_eq!(
        description(&format!(
            "<head><title>T</title><meta name=\"description\" content=\"{}\"></head>",
            "d".repeat(300)
        )),
        Some("d".repeat(300))
    );
}

#[test]
fn relative_and_protocol_relative_images_resolve_against_the_page() {
    assert_eq!(
        image("<head><title>T</title><meta property=\"og:image\" content=\"/img/a.png\"></head>")
            .as_deref(),
        Some("https://example.com/img/a.png")
    );
    assert_eq!(
        image(
            "<head><title>T</title><meta property=\"og:image\" content=\"//cdn.example.com/b.png\"></head>"
        )
        .as_deref(),
        Some("https://cdn.example.com/b.png")
    );
}

#[test]
fn non_http_image_schemes_are_refused() {
    assert_eq!(
        image(
            "<head><title>T</title><meta property=\"og:image\" content=\"file:///etc/passwd\"></head>"
        ),
        None
    );
    assert_eq!(
        image(
            "<head><title>T</title><meta property=\"og:image\" content=\"javascript:alert(1)\"></head>"
        ),
        None
    );
}

#[test]
fn a_tag_whose_name_merely_starts_with_meta_is_not_a_meta_tag() {
    assert_eq!(
        title("<head><metadata property=\"og:title\" content=\"Nope\"><title>Real</title></head>")
            .as_deref(),
        Some("Real")
    );
}

/// İ lowercases to two scalars, ẞ to "ss", the Kelvin sign to "k": nothing may drift.
#[test]
fn unicode_that_lowercases_to_a_different_length_does_not_corrupt_or_crash() {
    assert_eq!(
        title("<head><title>İstanbul Haber</title></head>").as_deref(),
        Some("İstanbul Haber")
    );
    assert_eq!(
        title(&format!("<head><title>{}</title></head>", "İ".repeat(10))),
        Some("İ".repeat(10))
    );
    assert_eq!(
        title("<head><title>GROẞE STRAẞE</title></head>").as_deref(),
        Some("GROẞE STRAẞE")
    );
    assert_eq!(
        title("<head><p>KKK</p><title>Real</title></head>").as_deref(),
        Some("Real")
    );
    assert_eq!(
        title(&format!(
            "<head><p>{}</p><meta property=\"og:title\" content=\"Found\"></head>",
            "İ".repeat(8)
        ))
        .as_deref(),
        Some("Found")
    );
    assert_eq!(
        title(&format!(
            "<head><meta property=\"og:description\" content=\"{}\"><title>Real</title></head>",
            "İ".repeat(16)
        ))
        .as_deref(),
        Some("Real")
    );
}

#[test]
fn a_stray_ampersand_page_stays_linear() {
    let clock = Instant::now();
    let _ = parse(&format!(
        "<head><title>{}x;</title></head>",
        "&".repeat(30_000)
    ));
    assert!(clock.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_quoted_angle_bracket_does_not_truncate_the_tag() {
    assert_eq!(
        title("<head><meta property=\"og:title\" content=\"A > B\"><title>Fallback</title></head>")
            .as_deref(),
        Some("A > B")
    );
}

#[test]
fn clamping_counts_scalars_and_never_splits_a_pair() {
    let clamped = title(&format!(
        "<head><meta property=\"og:title\" content=\"{}\"></head>",
        "😀".repeat(200)
    ))
    .unwrap_or_default();
    assert_eq!(clamped.chars().count(), MAX_TITLE_LENGTH + 1);
    assert!(clamped.ends_with('…'));
    assert_eq!(clamped.trim_end_matches('…'), "😀".repeat(140));
}

/// The entities the scanner knows, and the numeric ones that are not characters.
#[test]
fn only_real_characters_decode() {
    assert_eq!(
        decode_entities(
            "&#8217; &ldquo;x&rdquo; &mdash; &ndash; &lsquo; &LT; &gt; &quot; &apos; &nbsp; &#xE9; &#128512;"
        ),
        "’ “x” — – ‘ < > \" ' \u{A0} é 😀"
    );
    assert_eq!(
        decode_entities("&#xD800; &#1114112; &#; &#x; &bogus; &#-1;"),
        "&#xD800; &#1114112; &#; &#x; &bogus; &#-1;"
    );
    // The lookahead is ten characters, and that is a rule, not only a speed: an eleven-character
    // entity stays as typed.
    assert_eq!(decode_entities("&#000000065;"), "A");
    assert_eq!(decode_entities("&#0000000065;"), "&#0000000065;");
}

// ---- the YouTube page (#50) --------------------------------------------------------------------

const WATCH: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

const TITLE_TAG_OFFSET: usize = 704_923;
const OG_TITLE_OFFSET: usize = 706_842;
const HEAD_END_OFFSET: usize = 715_108;

/// The captured page with its elided player script put back at its exact byte count, so every
/// tag sits where it did.
fn youtube_watch_page() -> String {
    const MARKER: &str = "<!--FAMILY-CONNECT-FIXTURE: ";
    let raw = common::fixture_text("youtube-watch.html");
    let open = raw.find(MARKER).expect("the marker");
    let close = open + raw[open..].find("-->").expect("the marker's end");
    let count: usize = raw[open + MARKER.len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .expect("a count");
    let script = format!(
        "<script>var elided=\"{}\";</script>",
        "a".repeat(count - 31)
    );
    assert_eq!(script.len(), count);
    format!("{}{}{}", &raw[..open], script, &raw[close + 3..])
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The C#'s `html[..n]`: the first `n` UTF-16 units, here never half a character.
fn utf16_prefix(text: &str, units: usize) -> &str {
    let mut counted = 0;
    for (at, character) in text.char_indices() {
        counted += character.len_utf16();
        if counted > units {
            return &text[..at];
        }
    }
    text
}

#[test]
fn the_captured_page_hides_its_tags_past_a_quarter_megabyte() {
    let page = youtube_watch_page();
    let bytes = page.as_bytes();
    assert_eq!(find(bytes, b"<title"), Some(TITLE_TAG_OFFSET));
    assert_eq!(find(bytes, b"og:title"), Some(OG_TITLE_OFFSET));
    assert_eq!(find(bytes, b"</head"), Some(HEAD_END_OFFSET));
    const { assert!(OG_TITLE_OFFSET > 256 * 1024) };
}

#[test]
fn a_watch_page_yields_the_full_card() {
    let preview = link_preview::parse(&youtube_watch_page(), WATCH).expect("a card");
    assert_eq!(
        preview.title,
        "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)"
    );
    assert_eq!(preview.site_name, "YouTube");
    assert_eq!(
        preview.image_url.as_deref(),
        Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg")
    );
    assert!(
        preview
            .description
            .unwrap_or_default()
            .starts_with("The official video for")
    );
}

#[test]
fn the_old_scan_limit_is_what_made_the_card_vanish() {
    let html = youtube_watch_page();
    assert_eq!(
        link_preview::parse(utf16_prefix(&html, 200_000), WATCH),
        None
    );
    assert!(link_preview::parse(utf16_prefix(&html, SCAN_LIMIT), WATCH).is_some());
    const { assert!(SCAN_LIMIT >= page_reader::MAX_PAGE_BYTES) };
}

fn stop_index(bytes: &[u8]) -> Option<usize> {
    let mut detector = HeadEndDetector::new();
    bytes.iter().position(|&byte| detector.consume(byte))
}

#[test]
fn the_head_stop_lands_on_the_real_head_end() {
    assert_eq!(
        stop_index(youtube_watch_page().as_bytes()),
        Some(HEAD_END_OFFSET + 6)
    );
}

#[test]
fn the_head_stop_ignores_lookalikes_folds_case_and_falls_back_to_body() {
    assert_eq!(stop_index(b"<header><bodyguard>"), None);
    assert_eq!(stop_index(b"<head><meta><bodyx"), None);
    assert_eq!(
        stop_index(b"<head><title>x</title><body class=\"a\">"),
        Some(27)
    );
    assert_eq!(stop_index(b"<HEAD><TITLE>x</TITLE></HEAD>"), Some(28));
    assert_eq!(stop_index(b"<html><meta name=\"a\" content=\"b\">"), None);
}

/// A stream that hands over a few bytes at a time, so a head end can straddle two reads, and
/// counts what it handed over.
fn trickle(
    bytes: &[u8],
    chunk: usize,
    handed: Rc<Cell<usize>>,
) -> impl futures::Stream<Item = Result<Vec<u8>, Infallible>> + Unpin {
    let chunks: Vec<Vec<u8>> = bytes.chunks(chunk).map(<[u8]>::to_vec).collect();
    stream::iter(chunks.into_iter().map(move |chunk| {
        handed.set(handed.get() + chunk.len());
        Ok(chunk)
    }))
}

fn read_page(bytes: &[u8], chunk: usize, handed: Rc<Cell<usize>>) -> Vec<u8> {
    let mut pool = LocalPool::new();
    match pool.run_until(page_reader::read(
        trickle(bytes, chunk, handed),
        page_reader::MAX_PAGE_BYTES,
        true,
    )) {
        Ok(read) => read,
        Err(never) => match never {},
    }
}

#[test]
fn the_read_stops_at_the_head_and_tags_below_it_are_not_read() {
    let page = b"<html><head><meta name=\"nothing\" content=\"x\"></head><body><meta property=\"og:title\" content=\"Too Late\"></body></html>";
    let handed = Rc::new(Cell::new(0));
    let read = read_page(page, 5, Rc::clone(&handed));
    let text = String::from_utf8(read).expect("UTF-8");
    assert!(text.ends_with("</head>"));
    assert!(
        handed.get() < page.len(),
        "the rest of the page was never taken"
    );
    assert_eq!(parse(&text), None);
}

#[test]
fn the_ceiling_holds_when_a_page_never_closes_its_head() {
    let page = format!(
        "<html><head><script>{}</script><meta property=\"og:title\" content=\"Past The Cap\">",
        "a".repeat(1_100_000)
    );
    let read = read_page(page.as_bytes(), 16 * 1024, Rc::new(Cell::new(0)));
    assert_eq!(read.len(), page_reader::MAX_PAGE_BYTES);
    assert_eq!(parse(&String::from_utf8_lossy(&read)), None);
}

#[test]
fn the_watch_page_read_in_chunks_still_builds_the_card() {
    let page = youtube_watch_page();
    let read = read_page(page.as_bytes(), 7_919, Rc::new(Cell::new(0)));
    assert_eq!(read.len(), HEAD_END_OFFSET + 7);
    assert_eq!(
        link_preview::parse(&String::from_utf8_lossy(&read), WATCH)
            .map(|preview| preview.site_name)
            .as_deref(),
        Some("YouTube")
    );
}

#[test]
fn a_page_decodes_in_its_declared_charset_or_utf8() {
    assert_eq!(
        page_reader::decode(&[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2], Some("windows-1251")),
        "Привет"
    );
    assert_eq!(
        page_reader::decode("Привет".as_bytes(), Some("\"utf-8\"")),
        "Привет"
    );
    assert_eq!(
        page_reader::decode("Привет".as_bytes(), Some("x-no-such-charset")),
        "Привет"
    );
    assert_eq!(page_reader::decode("Привет".as_bytes(), None), "Привет");
}
