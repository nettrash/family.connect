# Serbian localization

Serbian ships in **both scripts**: Cyrillic (`sr`) and Latin (`sr-Latn`).

**Cyrillic is the source of truth. Latin is generated, never translated.**
Serbian's two scripts are a strict 1:1 mapping, so transliterating removes any
chance of the two drifting apart as strings change — which is the only real
cost of shipping both.

```
serbian.py                 the Cyrillic translations + the transliterator
serbian_android.py         the strings Android has that iOS does not
regenerate_serbian.py      writes all four resource sets
```

To change a Serbian string, edit the Cyrillic in `serbian.py` and run:

```bash
python3 tools/i18n/regenerate_serbian.py
```

It is idempotent — running it without an edit rewrites the same bytes.

## What it writes

| File | Contents |
| --- | --- |
| `ios/FamilyConnect/Localizable.xcstrings` | `sr` + `sr-Latn` localizations (iOS and macOS share one catalog) |
| `android/app/src/main/res/values-sr/strings.xml` | Cyrillic |
| `android/app/src/main/res/values-b+sr+Latn/strings.xml` | Latin |

Android's Latin folder is `values-b+sr+Latn` — the BCP-47 form. `values-sr-rLatn`
is **not** a thing; the old two-letter syntax has no room for a script subtag.

## Rules the generator enforces

- **Android reuses the iOS wording** wherever the English matches (bar
  capitalisation). Only genuinely Android-only strings live in
  `serbian_android.py`, so the two platforms cannot word one sentence two ways.
  A string with no translation is a hard error, not a silent English fallback.
- **The `.xcstrings` writer is byte-lossless.** It asserts that re-serialising
  the untouched catalog reproduces it exactly before touching anything, so
  adding one string does not reformat the whole file. Xcode's style: two-space
  indent, `"key" : value` with spaces both sides of the colon, an empty object
  written with a blank line inside it, and no trailing newline.
- **Register is informal** (ти), matching the other six languages — this is a
  family chat, not a bank.
- **Product names stay Latin** in Cyrillic text (`Family Connect`, `Mac`), which
  is what Apple's own Serbian does. The transliterator only maps Cyrillic
  characters, so they pass through untouched.
- **Plurals use Serbian's CLDR categories**: `one` / `few` / `other`
  (1, 21, 31… / 2–4, 22–24… / everything else).

## Adding a new string later

`xcodebuild` writes newly extracted literals back into the catalog as
untranslated keys, and `values/strings.xml` is hand-maintained. Either way the
generator will fail loudly on Android and leave a gap on iOS, so after adding
user-facing text, add the Serbian here and re-run. To find gaps across every
language at once:

```bash
python3 - <<'EOF'
import json, io
d = json.load(io.open('ios/FamilyConnect/Localizable.xcstrings', encoding='utf-8'))
need = {'de','es','fr','ja','ru','zh-Hans','sr','sr-Latn'}
for k, v in d['strings'].items():
    if v.get('shouldTranslate') is False:
        continue
    have = {l for l, u in v.get('localizations', {}).items()
            if u.get('stringUnit', {}).get('state') == 'translated' or 'variations' in u}
    if need - have:
        print(sorted(need - have), repr(k))
EOF
```

Android's equivalent is lint's `MissingTranslation`, which is already an error
in this project.
