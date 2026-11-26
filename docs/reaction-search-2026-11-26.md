# Reaction search (issue #116)

Written 26 November 2026 against `v1.4`.

The picker searches the 769 emoji in the reaction catalogue. It uses CLDR 49 names and keywords
in the app's language, English annotations, and the existing English shortcodes. Results keep
the catalogue's exact emoji sequences. Exact matches rank first, then name prefixes, then
keyword matches, with catalogue order breaking ties. The client uses the existing reaction API
when an emoji is selected.

The picker shows recents and categories when search is empty. Selecting a category clears the
query and opens that section. Choosing a result acts on the message that opened the picker.
Recents are local to the account and server. The app PRs cover keyboard and focus behavior on
desktop and keeping results usable with the keyboard open on mobile.

## Catalogue and license

`tools/emoji/make_search.py` checks the pinned CLDR inputs and generates nine UTF-8 TSVs and
the Unicode license notice. The generator replaces six Latin lookalikes inside Cyrillic words
in the Serbian Cyrillic table. The other eight locale tables retain their original bytes.
The checked-in package copies come from the same generator.

| Client | How the app PR ships the catalogue and notice | Uncompressed data |
| --- | --- | ---: |
| Web | English is compiled into WebAssembly. The other languages are versioned static assets loaded on demand. `UNICODE-LICENSE.txt` is a static asset. | 53,752 bytes built in, 536,703 bytes of locale assets, 3,193 bytes for the notice |
| Linux | English is compiled into the shared Rust crate. Packaging uses the locale files in `linux/data/emoji-search/` and records Unicode-3.0 attribution in the package copyright. | 53,752 bytes built in, 536,703 bytes for the eight additional locale tables, 3,193 bytes for the full notice |
| iOS and macOS | The Xcode targets use the generated `Emoji/emoji-search` resources in the app bundle, including the notice. | 593,648 bytes per app |
| Android | The app packages the generated `emoji-search` resources on the classpath, including the notice. | 593,648 bytes |
| Windows | App.Logic embeds the TSVs. The MSIX packages the notice as an asset. | 593,648 bytes before MSIX compression |

The eight optional Web tables are 16,110 to 25,112 bytes each when compressed independently
with gzip at level 9. The table gives data sizes. The app PRs measure the built artifacts.
Catalogue copies and the notice can be checked locally with `make_search.py --check`. Extend the
shared file comparison in a separate CI change agreed under CONTRIBUTING §7.

## Search parity

Search ignores case, normalizes shortcode separators and folds German ß to `ss`. It folds
Russian ё to е in both composed and decomposed input. Other accents and ligatures are preserved.
The reference leaves other code-point sequences as entered. For exact emoji matching it ignores
emoji presentation selector U+FE0F and retains text presentation selector U+FE0E.

`cargo run --quiet -- reaction-search` in `win/tools/board-oracle` prints the reference fixture
for the app ports. Each case specifies a locale, a query and every result in order with its name.
The cases cover all nine languages, shortcode and keyword ranking, the corrected Serbian terms,
accented and decomposed input, selector variants, and empty or unmatched input. Rust is the
reference for Web and Linux. The platform PRs wire the fixture into the C#, Kotlin and Swift
test suites when they adopt the shared catalogue and reference behavior.
