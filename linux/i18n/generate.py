"""Turn the apps' string catalogue into the Linux client's tables.

The apps' `ios/FamilyConnect/Localizable.xcstrings` is the source of truth for the nine languages,
and its KEYS ARE THE ENGLISH SOURCE STRINGS — which is how this port's call sites name them
(`say.get("No answer")`), so a sentence the apps already say is a sentence Linux says in nine
languages for free (shell-design §23; `win/i18n/generate.py` is the model).

Three sources, in this order for a sentence:

1. `Localizable.xcstrings` — a value marked `translated`;
2. `linux/i18n/linux.json` — the Linux port's OWN sentences (the top bar, launch at login, the
   portal's shortcut, the keyring …), in `win/i18n/win.json`'s shape;
3. `win/i18n/win.json` — the sentences the Windows port already wrote for the same desktop words
   (file sizes, "Couldn't load statistics" …), REUSED VERBATIM rather than copied.

For a plural the order is linux.json's forms, then win.json's, then the xcstrings' (a port's own
forms are where it says a key BETTER — English "1 question" for a key whose English is plural).

Only the sentences this port actually draws are emitted. They are found exactly as the catalogue
test finds them (`crates/fc-core/tests/text_catalogue.rs`, `port_sentences()`: `common::sentences()`
over `linux/crates` plus the same scan over `linux/app/src`), so the generator and the test agree on
one list: every `.rs` file under `linux/crates` and `linux/app` outside a `tests` or `target` folder,
lines that start with `//` left out, and a sentence is the string literal right after `say.get(`,
`say.format(` or `say.plural(` — a catalogue named `say` (the convention of
`fc_core::text::string_catalog`).

A sentence nobody has translated and linux.json names with an EMPTY object reads in English, and
must also be on `ENGLISH_FOR_NOW` in `crates/fc-core/tests/text_catalogue.rs`; any other gap fails
— that is the CI string gate. So does a linux.json key no source draws (a stale entry is a lie in the
other direction).

English gets no file: the key IS the English (`EnglishCatalog`).

    python3 linux/i18n/generate.py <repo>           # writes linux/data/i18n/<lang>.json + plurals.json
    python3 linux/i18n/generate.py <repo> --check   # writes nothing; exits 1 if anything is stale

`linux/data/` mirrors what the packages install under `/usr/share/family-connect/` (the app's
`services::paths` looks beside the executable first, then there).
"""
import json
import pathlib
import sys

LANGS = ["de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans"]
CALLS = (".get(", ".format(", ".plural(")


def string_literal(text: str) -> str | None:
    """The ordinary string literal `text` starts with, unescaped — `string_literal` in the Rust
    scan, escape for escape: a raw string or anything else is not a sentence."""
    if not text.startswith('"'):
        return None
    out = []
    i = 1
    while i < len(text):
        c = text[i]
        i += 1
        if c == '"':
            return "".join(out)
        if c != "\\":
            out.append(c)
            continue
        if i >= len(text):
            return None
        e = text[i]
        i += 1
        if e == "n":
            out.append("\n")
        elif e == "t":
            out.append("\t")
        elif e == "r":
            out.append("\r")
        elif e == "0":
            out.append("\0")
        elif e == "u":
            if not text[i:].startswith("{") or "}" not in text[i:]:
                return None
            digits, _, _ = text[i + 1:].partition("}")
            try:
                out.append(chr(int(digits, 16)))
            except ValueError:
                return None
            i += len(digits) + 2
        elif e == "\n":
            # A line continuation: the newline and the next line's indent are not text.
            while i < len(text) and text[i].isspace():
                i += 1
        else:
            out.append(e)
    return None


def is_ident(c: str) -> bool:
    return c.isalnum() or c == "_"


ROOTS = ("crates", "app")


def sentences(linux: pathlib.Path) -> set[str]:
    """Every sentence the Linux port asks a catalogue for (see the module's comment)."""
    found: set[str] = set()
    pending = [linux / root for root in ROOTS if (linux / root).is_dir()]
    while pending:
        directory = pending.pop()
        for path in sorted(directory.iterdir()):
            if path.is_dir():
                if path.name not in ("tests", "target"):
                    pending.append(path)
            elif path.suffix == ".rs":
                source = "".join(
                    line + "\n"
                    for line in path.read_text().split("\n")
                    if not line.lstrip().startswith("//")
                )
                for call in CALLS:
                    start = 0
                    while (at := source.find(call, start)) >= 0:
                        start = at + len(call)
                        receiver = source[:at].rstrip()
                        if not receiver.endswith("say"):
                            continue
                        before = receiver[:-3]
                        if before and is_ident(before[-1]):
                            continue
                        literal = string_literal(source[start:].lstrip())
                        if literal is not None:
                            found.add(literal)
    return found


def translated(entry: dict, lang: str) -> str | None:
    """One language's value in the xcstrings, or None when nobody has written it yet."""
    unit = entry.get("localizations", {}).get(lang, {}).get("stringUnit", {})
    value = unit.get("value")
    # A string Xcode wrote back but nobody touched is not a translation; an outer sentence around a
    # plural substitution ("%1$@. %#@arg2@") is not a sentence on its own — its forms are plurals.
    return value if value and unit.get("state") == "translated" and "%#@" not in value else None


def apple_plural(loc) -> dict:
    """A localization's forms by CLDR category — `win/i18n/generate.py`'s flattening."""
    if not loc:
        return {}
    if "substitutions" in loc:
        outer = loc.get("stringUnit", {}).get("value", "")
        subs = loc["substitutions"]
        if not outer or len(subs) != 1:
            return {}
        name, sub = next(iter(subs.items()))
        place = f"%{sub['argNum']}${sub.get('formatSpecifier', 'lld')}"
        return {category: outer.replace(f"%#@{name}@", body["stringUnit"]["value"].replace("%arg", place))
                for category, body in sub.get("variations", {}).get("plural", {}).items()
                if body.get("stringUnit", {}).get("value")}
    if "variations" in loc:
        return {category: body["stringUnit"]["value"]
                for category, body in loc["variations"].get("plural", {}).items()
                if body.get("stringUnit", {}).get("value")}
    return {}


def own_forms(table: dict, key: str, lang: str) -> dict:
    value = (table.get(key) or {}).get(lang)
    if isinstance(value, dict):
        return {category: text for category, text in value.items() if text}
    return {}


def plural_forms(keys, catalogue, linux_own, windows_own, lang) -> dict:
    found = {}
    for key in sorted(keys):
        forms = (own_forms(linux_own, key, lang)
                 or own_forms(windows_own, key, lang)
                 or apple_plural((catalogue.get(key) or {}).get("localizations", {}).get(lang)))
        if forms:
            found[key] = dict(sorted(forms.items()))
    return found


def said(key: str, lang: str, catalogue: dict, linux_own: dict, windows_own: dict) -> str | None:
    entry = catalogue.get(key)
    value = translated(entry, lang) if entry else None
    for own in (linux_own, windows_own):
        if value is None and key in own and isinstance(own[key].get(lang), str):
            value = own[key][lang]
    return value


def write(path: pathlib.Path, body: str, checking: bool) -> bool:
    """Whether the file was stale."""
    if path.exists() and path.read_text() == body:
        return False
    if not checking:
        path.write_text(body)
    return True


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    checking = "--check" in sys.argv
    repo = pathlib.Path(args[0] if args else ".").resolve()
    linux = repo / "linux"
    keys = sentences(linux)
    catalogue = json.loads((repo / "ios/FamilyConnect/Localizable.xcstrings").read_text())["strings"]
    linux_own = json.loads((linux / "i18n/linux.json").read_text())["strings"]
    windows_own = json.loads((repo / "win/i18n/win.json").read_text())["strings"]
    out = linux / "data/i18n"
    out.mkdir(parents=True, exist_ok=True)

    stale = False
    english_only: dict[str, list[str]] = {}
    plurals = {lang: plural_forms(keys, catalogue, linux_own, windows_own, lang) for lang in ["en", *LANGS]}
    for lang in LANGS:
        table, missing = {}, []
        for key in sorted(keys):
            value = said(key, lang, catalogue, linux_own, windows_own)
            if value is None:
                if key in plurals[lang]:
                    # Said only in forms, one per count: `plural` reads them, `get` never needs to.
                    continue
                missing.append(key)
                continue
            table[key] = value
        english_only[lang] = missing
        body = json.dumps(table, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
        stale |= write(out / f"{lang}.json", body, checking)
        print(f"  {lang}: {len(table)} of {len(keys)}"
              + (f", English for now: {len(missing)}" if missing else ""))
    body = json.dumps(plurals, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    stale |= write(out / "plurals.json", body, checking)
    print("  plurals: " + ", ".join(f"{lang} {len(plurals[lang])}" for lang in plurals))

    failed = False
    unnamed = sorted(set().union(*english_only.values()) - set(linux_own))
    if unnamed:
        print("\nSENTENCES NOBODY HAS TRANSLATED AND linux.json DOES NOT NAME:")
        for key in unnamed:
            langs = [lang for lang in LANGS if key in english_only[lang]]
            print(f"    {key!r} ({', '.join(langs)})")
        print("Translate them in linux/i18n/linux.json (all eight languages), or name one with {} "
              "AND on ENGLISH_FOR_NOW in crates/fc-core/tests/text_catalogue.rs.")
        failed = True
    unused = sorted(set(linux_own) - keys)
    if unused:
        print("\nlinux.json NAMES SENTENCES NO SOURCE DRAWS (say.get/format/plural):")
        for key in unused:
            print(f"    {key!r}")
        failed = True
    if checking and stale:
        print("\nthe tables are out of date: run python3 linux/i18n/generate.py . without --check")
        failed = True
    return 1 if failed else 0


sys.exit(main())
