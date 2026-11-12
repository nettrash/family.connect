# Reaction search in Web

First delivery for [nettrash/family.connect#116](https://github.com/nettrash/family.connect/issues/116): Web and shared search data. Native client pickers are follow-up work under that issue.

The extended reaction picker now has a fixed search field, recent choices, a scrollable categorised grid and category shortcuts. On narrow screens it is a bottom panel. Searching covers the whole catalogue, not the section currently in view. Clearing restores the catalogue; Escape closes the picker and returns focus to the message's actions button. The existing modal supplies the focus loop.

Search uses CLDR names and keywords in the browser's interface language, English names/keywords and the existing English shortcode aliases. Case, surrounding whitespace, colons, underscores and hyphens are normalised. Multiword queries require every word to match. Exact names and aliases rank first, then name prefixes, then keyword matches, with catalogue order breaking ties. Each result is offered once and sends the exact existing catalogue Unicode sequence. This is keyword search, not an AI service.

The catalogue has 771 positions and 769 unique emoji. Search data covers every unique emoji in all nine interface languages. `tools/emoji/make_search.py` generates `web/text/data/emoji-search.tsv` from Unicode CLDR 49 commit `96e54b273d2da4ec6fbad350ba19ff23fa24a588`; every input is checked against `tools/emoji/cldr-search-sources.sha256`. Serbian Latin has its own CLDR annotations; `zh-Hans` uses CLDR's Simplified Chinese `zh` annotations. Unicode-3.0 attribution is shipped beside the data. Shortcode replacement while typing is unchanged.

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

Local environment: macOS ARM64, Rust 1.95.0, Trunk 0.22.1, wasm-pack 0.15.1, Chrome 157.0.8103.39 and ChromeDriver 157.0.8103.50. The server uses an isolated PostgreSQL 16 Docker container.

| Requirement | Evidence |
| --- | --- |
| Nine languages and exact localised accessible names | `localised_keywords_find_fire_in_all_nine_languages`; nine browser locale contexts in `reaction_search_two_accounts_e2e` |
| Aliases, case/spacing, multiword queries, Unicode variants and no matches | `case_spaces_aliases_and_unicode_variants_return_catalogue_bytes` |
| Stable, unique results and complete catalogue coverage | `results_are_stable_unique_and_every_locale_covers_the_catalogue` |
| Recent order, validation, deduplication and limit | `recents_move_to_front_deduplicate_validate_and_bound` |
| Search, empty state, clear and selected callback | `search_empty_state_clear_and_choice_reach_the_callback` |
| Category navigation does not choose a reaction | `category_navigation_clears_query_without_choosing_a_reaction` |
| Localised field/button names, focus and Escape | `picker_uses_localised_names_and_keyboard_dismissal`; e2e checks return to the message button |
| Account isolation, reopening, malformed storage and logout | `recents_are_per_account_survive_reopening_and_clear_on_logout`; e2e with two actual accounts |
| Real set, replace and remove delivery | `reaction_search_two_accounts_e2e`: three `reaction` frames received by the second client and matching UI changes |
| Desktop, dark and short mobile layout | E2e screenshots and bounds assertions, including category navigation and Cancel inside a 390 × 430 viewport |

The shared native suite passed **568 tests**. The picker browser group passed **5 tests**. The full browser suite's final local run passed **726** and failed **1**: the same pre-existing fake-camera live-encoder test observed before the feature (`round_record_tests::the_live_encoder_makes_a_moov_first_480_square_h264_and_aac_clip`, no first frame within 2 seconds). Nothing is skipped or suppressed. An earlier concurrent run also hit the board text-fitting timing assertion; that test passed separately and did not fail in the final full run. The full local suite is not green.

The standalone e2e passed with real REST/WebSocket traffic, exact names checked in nine browser language contexts, zero API requests caused by searching, local-recents isolation and set/replace/remove propagation. The shorter viewport checks layout when space is reduced; it does not prove the behaviour of a physical iOS/Android software keyboard. No real camera or microphone was opened.

## E2e reproduction

Run the server and serve Web from the same origin with HTTP `/api/v1/` and WebSocket `/api/v1/ws` proxies. Use Node 26.13.0 / npm 11.20.0, as recorded in `web/e2e/.node-version` and `package.json`:

```bash
cd web/e2e
npm ci --ignore-scripts --no-audit --no-fund
./node_modules/.bin/playwright install chromium
npm test
npm run check-results
```

This uses Playwright Test 1.65.0 directly in JavaScript, with the browser revision selected by that exact package. It creates disposable local accounts and a family. `FC_E2E_URL` defaults to `http://127.0.0.1:58081` and is restricted to localhost; `FC_E2E_OUTPUT` overrides the default `output/playwright/reaction-search/`. Reports include standard JSON, JUnit and HTML, a domain `summary.json`, and explicit desktop/mobile screenshots. Failures retain a screenshot, video and trace; no webcam or microphone is opened. There is no Python wrapper, Playwright CLI, or parsing of human-readable command output. Retries are disabled, focused-only tests are forbidden, and CI checks that the named scenario actually completed without skips or retries.

CI uses the repository's existing `.github/workflows/ci.yml`. After review and agreement, a `Web (Playwright scenarios)` job adds the first hosted browser scenario rather than a second workflow. Web, server and emoji-tool changes trigger it; shared protocol/CI changes and an unresolved diff base retain the existing run-all behaviour. The server job shares its already-built binary as a one-day artifact, so Web does not build the server again. The Web job uses a digest-pinned Playwright Linux image (including its Chromium and OS libraries), exact Node/npm/Rust versions, digest-pinned PostgreSQL, a digest-pinned Rust/C build environment and checksum-verified Trunk/wasm-bindgen tools, and locked npm/Cargo dependencies. Shared native search tests remain in the existing Windows/oracle step; translations and shared resources remain in the existing strings/rich checks. Dependency pins apply only to the new Web job and its added server-artifact handoff; existing jobs retain their previous dependency policies.

The job checks Web formatting/Clippy, serves the release Web build against a fresh local server/database, and runs `reaction_search_two_accounts_e2e` in the pinned headless Chromium build. A separate result guard requires the named passing scenario, all nine locales, at least three reaction events, no search API requests and nonempty desktop/mobile screenshots. Screenshots, JSON/JUnit/HTML results and server/Trunk/build-check logs are retained as `web-evidence` for 14 days on success or failure. Failed tests also retain trace and video evidence. Dependency updates in this scope are manual; see [Web CI dependency policy](web-ci-dependencies.md). This is the first real-browser scenario, not a claim of complete browser/platform coverage.

An initial standalone Web workflow was tried and removed after review. Its [recorded run](https://github.com/iBarBuba/family.connect/actions/runs/43059598028) passed the new picker tests and the nine-locale two-account e2e on Ubuntu. The full browser suite there finished with 670 passed and 57 failures, including media capability/encoding failures and other browser-dependent assertions; this is not a green full-Web result. The new job runs the agreed Playwright scenario; it does not run or claim to fix that full wasm-bindgen suite. The failed trial run remains available for investigation. Broader browser/media coverage requires separate work.

## Review evidence

[Before](evidence/reaction-search-116/00-before.png) · [Search](evidence/reaction-search-116/03-search-heart.png) · [Recent choices](evidence/reaction-search-116/05-recents.png) · [Russian](evidence/reaction-search-116/06-search-russian.png) · [Dark](evidence/reaction-search-116/07-search-dark.png) · [Mobile](evidence/reaction-search-116/08-search-mobile.png) · [Short viewport](evidence/reaction-search-116/09-short-mobile-viewport.png) · [Recorded e2e](evidence/reaction-search-116/reaction-search.mp4) · [Machine-readable e2e results](evidence/reaction-search-116/summary.json).

The before image is the original Web picker. The remaining images and video are the fork's implemented Web picker with local test data; they are not mockups or evidence of native client changes.
