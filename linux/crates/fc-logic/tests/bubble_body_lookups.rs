//! `LookupsTests.TheFootersLinksAreTappable`, whose subject is `BubbleBody`: the footer is plain
//! markdown, and every link in it opens — the sources, Open-Meteo's credit and the licence — and
//! nothing the server defanged becomes a link again.

use fc_logic::bubble_body::{BodyBlock, BodyRun, lay, openable};

const ME: i64 = 7;

const FOOTER: &str = "Snow showers tomorrow, around −2 °C.\n\n\
    Sources: [Tromsø – Wikipedia](https://en.wikipedia.org/wiki/Troms%C3%B8) · [Weather in Tromsø](https://example.org/tromso)\n\
    [Weather data by Open-Meteo.com](https://open-meteo.com/) · Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) · Powered by Brave";

fn text_runs(blocks: &[BodyBlock]) -> Vec<&BodyRun> {
    blocks
        .iter()
        .filter_map(BodyBlock::runs)
        .flatten()
        .collect()
}

#[test]
fn the_footers_links_are_tappable() {
    let blocks = lay(FOOTER, None, &[], ME, &|_| false);
    let mut links: Vec<&str> = Vec::new();
    for link in text_runs(&blocks)
        .iter()
        .filter_map(|run| run.link.as_deref())
    {
        if !links.contains(&link) {
            links.push(link);
        }
    }
    assert_eq!(
        vec![
            "https://en.wikipedia.org/wiki/Troms%C3%B8",
            "https://example.org/tromso",
            "https://open-meteo.com/",
            "https://creativecommons.org/licenses/by-sa/4.0/",
        ],
        links
    );
    assert!(links.iter().all(|link| openable(Some(link)).is_some()));
    // The two lines stay two lines: the credit does not run on from the last source.
    let plain: String = text_runs(&blocks)
        .iter()
        .map(|run| run.text.as_str())
        .collect();
    assert!(plain.contains("Weather in Tromsø\n"), "{plain:?}");

    let defanged = lay("Look at example[.]com for more.", None, &[], ME, &|_| false);
    assert!(text_runs(&defanged).iter().all(|run| run.link.is_none()));
}
