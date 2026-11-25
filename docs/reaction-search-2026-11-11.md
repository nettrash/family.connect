# #116 Reaction search in Web

Written 2026-11-11 against `119-web-emoji-reaction-search`. Updated 2026-11-25.

Web picker for [#119](https://github.com/nettrash/family.connect/issues/119), part of the cross-client reaction search in [#116](https://github.com/nettrash/family.connect/issues/116). The canonical data and reference search are reviewed in [#129](https://github.com/nettrash/family.connect/pull/129).

The extended reaction picker now has a fixed search field, recent choices, a scrollable categorised grid and category shortcuts. On narrow screens it is a bottom panel, and the shortcuts wrap when space runs out. Changing the query resets the results to the top, and clearing it restores the catalogue. Category shortcuts scroll to their section and focus its first emoji. Escape closes the picker and returns focus to the message's actions button.

Search uses CLDR names and keywords in the browser's interface language, English names/keywords and the existing English shortcode aliases. Case, surrounding whitespace, colons, underscores and hyphens are normalised. Multiword queries require every word to match. Exact names and aliases rank first, then name prefixes, then keyword matches, with catalogue order breaking ties. Each result is offered once and sends the exact existing catalogue Unicode sequence. This is keyword search, not an AI service.

The catalogue has 771 positions and 769 unique emoji. `tools/emoji/make_search.py` generates search data for all nine interface languages from Unicode CLDR 49 commit `96e54b273d2da4ec6fbad350ba19ff23fa24a588`. Every input is checked against `tools/emoji/cldr-search-sources.sha256`. Serbian Latin has its own CLDR annotations, and `zh-Hans` uses the CLDR `zh` annotations. The Web build includes the Unicode-3.0 license.

All nine locale tables live in `web/text/data/emoji-search/`. Wasm embeds the 53,752-byte English table and the existing shortcode aliases. Trunk copies the eight non-English tables to `dist/emoji-search/`. They total 535,314 bytes on disk, ranging from 50,386 bytes (Spanish) to 103,225 bytes (Russian) before any HTTP compression. The separate Unicode license is 3,193 bytes. The [maintainer measured a 128,038-byte increase](https://github.com/nettrash/family.connect/pull/125#pullrequestreview-6035722696) in raw release Wasm with rustc 1.100 and Trunk 0.22.1, including the English table.

Opening the picker in a non-English locale requests only that locale's versioned TSV. English search stays available while the file loads. When it arrives, the picker updates the current results and accessible names. The page reuses successfully loaded data on reopening without a second render. Reopening can retry a failed load. The picker keeps a status region mounted and updates its text while waiting for results or when no emoji match. Shortcode replacement while typing is unchanged.

Recent choices are local to an account and browser origin, newest first, deduplicated and limited to 24 catalogue entries. A deliberate selection is remembered immediately, including an optimistic selection whose server request might later fail. Opening or searching the picker and other members' reactions do not change recents. Malformed stored data is ignored, invalid entries are removed, and logout clears reaction-recents keys along with the existing personal sticker/played state. No new API or database migration is needed.

## Regeneration

```bash
python3 tools/emoji/make_search.py
python3 tools/emoji/make_search.py --check
```

Both commands download the ten pinned inputs. For offline regeneration/checking, provide a CLDR source directory containing `LICENSE` and `common/annotations/{en,de,es,fr,ja,ru,sr,sr_Latn,zh}.xml`:

```bash
python3 tools/emoji/make_search.py --source-dir /path/to/cldr-49 --check
```

## Verification

Use the repository's Rust and browser test commands:

```bash
(cd web/text && cargo test)
(cd web && wasm-pack test --headless --chrome)
```

The shared tests cover locale and English fallback ranking, aliases, catalogue coverage, Unicode sequences, invalid data, and recent choices. The browser tests also verify scroll reset, category focus, narrow picker layout, the language request, English fallback, retry, and reuse after reopening the picker.
