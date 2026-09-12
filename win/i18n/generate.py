"""Turn the apps' string catalogue into the Windows client's nine tables.

The apps' `ios/FamilyConnect/Localizable.xcstrings` is the source of truth for the nine
languages, and its KEYS ARE THE ENGLISH SOURCE STRINGS — which is exactly how this port's call
sites name them (`say.Get("No answer")`), so a string the apps already say is a string Windows
says in nine languages for free.

Only the keys Windows actually uses are emitted: the catalogue holds 617 and this client draws a
few dozen. Its own strings — the ones the apps have never needed — live in `win/i18n/win.json`,
where an EMPTY object means "English for now", recorded rather than pretended.

English gets no file: the key IS the English, so `EnglishCatalog` answers by returning it.

    python3 win/i18n/generate.py <repo>

writes `win/src/FamilyConnect.Core/Text/i18n/<lang>.json` and prints what each language still
owes. `--check` writes nothing and exits non-zero if anything is out of date, which is what a
test can run.

WHY JSON AND NOT `.resw`: a `.resw` name cannot BE an English sentence — `%@`, `·`, an em dash and
a full stop are all either invalid or meaningful in a resource name, and WinUI mangles what it
takes (a dot is a hierarchy separator). A `.resw` would therefore need a second table mapping
slugs to sentences, which is a second source of truth and the thing this design exists to avoid.
The app's own XAML chrome can still use `.resw`; every sentence that has to AGREE with the other
three clients comes from here.
"""
import json
import pathlib
import re
import sys

LANGS = ["de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans"]

KEY = re.compile(r'\b(?:Get|Format)\(\s*"((?:[^"\\]|\\.)+)"')


def used_keys(repo: pathlib.Path) -> set[str]:
    """Every sentence this port asks for, from its own source and nowhere else."""
    keys: set[str] = set()
    for path in sorted((repo / "win/src").rglob("*.cs")):
        keys.update(KEY.findall(path.read_text()))
    return keys


def apple(repo: pathlib.Path) -> dict:
    return json.loads((repo / "ios/FamilyConnect/Localizable.xcstrings").read_text())["strings"]


def own(repo: pathlib.Path) -> dict:
    return json.loads((repo / "win/i18n/win.json").read_text())["strings"]


def translated(entry: dict, lang: str) -> str | None:
    """One language's value, or None when nobody has written it yet."""
    unit = entry.get("localizations", {}).get(lang, {}).get("stringUnit", {})
    value = unit.get("value")
    # A string Xcode has written back but nobody has touched is not a translation.
    return value if value and unit.get("state") == "translated" else None


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    checking = "--check" in sys.argv
    repo = pathlib.Path(args[0] if args else ".").resolve()
    keys = used_keys(repo)
    catalogue, mine = apple(repo), own(repo)
    out = repo / "win/src/FamilyConnect.Core/Text/i18n"
    out.mkdir(parents=True, exist_ok=True)
    english_only: dict[str, list[str]] = {}
    stale = False
    for lang in LANGS:
        table = {}
        missing = []
        for key in sorted(keys):
            entry = catalogue.get(key)
            said = translated(entry, lang) if entry else None
            if said is None and key in mine:
                said = mine[key].get(lang)
            if said is None:
                missing.append(key)
                continue
            table[key] = said
        english_only[lang] = missing
        body = json.dumps(table, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
        path = out / f"{lang}.json"
        if not path.exists() or path.read_text() != body:
            stale = True
            if not checking:
                path.write_text(body)
        print(f"  {lang}: {len(table)} of {len(keys)}"
              + (f", English for now: {len(missing)}" if missing else ""))
    unknown = sorted(set().union(*english_only.values()) - set(mine))
    if unknown:
        print("\nKEYS NOBODY HAS TRANSLATED AND win.json DOES NOT NAME:")
        for key in unknown:
            print("   ", key)
        print("Add them to win/i18n/win.json (empty object = English for now) or translate them.")
        return 1
    if checking and stale:
        print("\nthe tables are out of date: run without --check")
        return 1
    return 0


sys.exit(main())
