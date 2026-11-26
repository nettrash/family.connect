use fc_text::emoji_search::{self, Annotations};
use fc_text::i18n::Lang;
use serde_json::json;

pub fn print_vectors() {
    macro_rules! data {
        ($locale:literal) => {
            include_str!(concat!(
                "../../../../web/text/data/emoji-search/",
                $locale,
                ".tsv"
            ))
        };
    }

    let catalogues = [
        (Lang::En, data!("en"), "fire"),
        (Lang::De, data!("de"), "Feuer"),
        (Lang::Es, data!("es"), "fuego"),
        (Lang::Fr, data!("fr"), "feu"),
        (Lang::Ja, data!("ja"), "火"),
        (Lang::Ru, data!("ru"), "огонь"),
        (Lang::Sr, data!("sr"), "ватра"),
        (Lang::SrLatn, data!("sr-Latn"), "vatra"),
        (Lang::ZhHans, data!("zh-Hans"), "火"),
    ];
    let mut vectors = Vec::new();
    let mut add = |id, language: Lang, query, annotations: &Annotations| {
        let matches: Vec<_> = emoji_search::search(query, Some(annotations))
            .into_iter()
            .map(|result| json!({"emoji": result.emoji, "name": result.name}))
            .collect();
        vectors.push(json!({
            "id": id,
            "locale": language.tag(),
            "query": query,
            "matches": matches,
        }));
    };
    for (language, tsv, fire) in catalogues {
        let annotations = Annotations::parse(language, tsv).expect("complete reaction search TSV");
        add("localized_fire", language, fire, &annotations);
        let queries: &[(&str, &str)] = match language {
            Lang::En => &[
                ("shortcode_whitespace_case", "  :FIRE:  "),
                ("two_words", "red heart"),
                ("exact_emoji", "❤️"),
                ("emoji_without_selector", "❤"),
                ("text_presentation_selector", "❤\u{fe0e}"),
                ("uncatalogued_skin_tone", "👍🏽"),
                ("empty", ""),
                ("separators_only", "__::"),
                ("no_match", "no-such-emoji-8372"),
            ],
            Lang::De => &[("sharp_s", "FUSSBALL")],
            Lang::Es => &[
                ("accents_preserved", "corazón"),
                ("unaccented_query", "corazon"),
            ],
            Lang::Fr => &[
                ("composed_query", "fête"),
                ("decomposed_query", "fe\u{0302}te"),
                ("ligature_preserved", "cœur"),
            ],
            Lang::Ru => &[
                ("english_shortcode", ":ice:"),
                ("russian_e", "елка"),
                ("russian_yo", "ёлка"),
                ("russian_decomposed_yo", "е\u{0308}лка"),
            ],
            Lang::Sr => &[
                ("serbian_flame", "пламен"),
                ("serbian_tears", "сузе"),
                ("serbian_sweet", "слатко"),
                ("serbian_cake", "торта"),
            ],
            _ => &[],
        };
        for &(id, query) in queries {
            add(id, language, query, &annotations);
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"search": vectors})).unwrap()
    );
}
