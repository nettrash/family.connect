"""Turn the apps' string catalogue into a Rust table for the web client.

The apps' `Localizable.xcstrings` is the source of truth for the nine
languages: its KEYS are the English source strings, which is exactly how the
web's call sites name them, so a string the apps already say is a string the
web says in nine languages for free.

Only the keys the WEB actually uses are emitted — the file holds 597 and the
web shows a subset — and the web's own strings live beside it in
`web/i18n/web.json`: its `strings` are what the web says and the apps do not
(an empty value means "English for now"), and its `indirect` names the keys
no scan can see, the ones a `const` table holds and a call site says by
variable.

Run: python3 web/i18n/generate.py <repo> ; writes web/text/src/i18n/catalogue.rs
and runs `cargo fmt` over it, because the table is emitted one row per line
and the file is checked in formatted like every other.

Pass --sync to write every English-only key into web/i18n/web.json, so that
file names exactly the strings a translator has left to do.
"""
import json
import pathlib
import re
import subprocess
import sys

LANGS = ["de", "en", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans"]
IDENT = {"de": "DE", "en": "EN", "es": "ES", "fr": "FR", "ja": "JA",
         "ru": "RU", "sr": "SR", "sr-Latn": "SR_LATN", "zh-Hans": "ZH_HANS"}


def used_keys(repo: pathlib.Path) -> set[str]:
    """Every key the web asks for: the first argument of t/t1/t2/t3/tn/tn1/tp."""
    pats = [re.compile(r'\bt[np]?[123]?\(\s*"((?:[^"\\]|\\.)+)"')]
    keys = set()
    for path in list((repo / "web/src").rglob("*.rs")) + list((repo / "web/text/src").rglob("*.rs")):
        # A test asserts on the ENGLISH, and its keys are not the app's.
        text = without_tests(path.read_text())
        for pat in pats:
            for m in pat.finditer(text):
                keys.add(unescape(m.group(1)))
    return keys


def without_tests(text: str) -> str:
    """The file with every `#[cfg(test)]` item cut out.

    Not "everything after the first one": a crate root declares its test
    module near the TOP (`#[cfg(test)] mod layout_tests;`), and cutting
    there hides the whole program — which is exactly how the web's bar kept
    its English through a sweep that reported nothing left to do.
    """
    out = []
    rest = text
    while True:
        at = rest.find("#[cfg(test)]")
        if at < 0:
            out.append(rest)
            return "".join(out)
        out.append(rest[:at])
        after = rest[at + len("#[cfg(test)]"):]
        brace = after.find("{")
        semi = after.find(";")
        if semi >= 0 and (brace < 0 or semi < brace):
            rest = after[semi + 1:]
            continue
        if brace < 0:
            return "".join(out)
        depth, index = 0, brace
        while index < len(after):
            if after[index] == "{":
                depth += 1
            elif after[index] == "}":
                depth -= 1
                if depth == 0:
                    break
            index += 1
        rest = after[index + 1:]


def placeholders(text: str) -> tuple:
    """How many arguments a string takes, and of which kinds — the shape a
    translation has to keep. A positional `%2$@` counts as one `%@`."""
    kinds = re.findall(r"%(?:\d+\$)?(@|lld)", text.replace("%%", ""))
    return tuple(sorted(kinds))


def unescape(s: str) -> str:
    return s.encode().decode("unicode_escape") if "\\" in s else s


def rust(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n") + '"'


def web_forms(value) -> list:
    """The web's own shape: one string, or a string per CLDR category."""
    if isinstance(value, str):
        return [("one form", value)] if value else []
    return [(category, text) for category, text in sorted(value.items()) if text]


def apple_forms(key: str, lang: str, loc) -> list:
    """Apple's shape, in the web's: a unit, a plural, or a plural inside a
    sentence (`substitutions`), all as a list of (category, text)."""
    if not loc:
        return []
    if "substitutions" in loc:
        return expand(key, lang, loc)
    if "stringUnit" in loc:
        value = loc["stringUnit"].get("value", "")
        return [("one form", value)] if value else []
    if "variations" in loc:
        return [(category, body["stringUnit"]["value"])
                for category, body in sorted(loc["variations"].get("plural", {}).items())
                if body.get("stringUnit", {}).get("value")]
    return []


def expand(key: str, lang: str, loc: dict) -> list:
    """A localization whose plural is INSIDE the sentence, flattened.

    Apple writes "%1$@. %#@arg2@" plus a table of forms for `arg2`, each
    spelling its own argument `%arg`. One form per category, with the
    sentence around it, says the same thing without a second mechanism —
    and `%arg` becomes the positional placeholder the argument actually is,
    so `fill` puts the right one there.
    """
    outer = loc.get("stringUnit", {}).get("value", "")
    subs = loc["substitutions"]
    if not outer or len(subs) != 1:
        print(f"  !! {lang} {key[:50]!r}: {len(subs)} substitutions, left English")
        return []
    name, sub = next(iter(subs.items()))
    spec = sub.get("formatSpecifier", "lld")
    place = f"%{sub['argNum']}${spec}"
    forms = []
    for category, body in sorted(sub.get("variations", {}).get("plural", {}).items()):
        text = body.get("stringUnit", {}).get("value")
        if not text:
            continue
        forms.append((category, outer.replace(f"%#@{name}@", text.replace("%arg", place))))
    return forms


def main() -> int:
    repo = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    apple = json.load(open(repo / "ios/FamilyConnect/Localizable.xcstrings"))["strings"]
    own_path = repo / "web/i18n/web.json"
    own_file = json.load(open(own_path)) if own_path.exists() else {}
    own = own_file.get("strings", {})
    wanted = used_keys(repo) | set(own_file.get("indirect", []))

    singular = {lang: [] for lang in LANGS}
    plural = {lang: [] for lang in LANGS}
    missing = []
    for key in sorted(wanted):
        apple_locs = (apple.get(key) or {}).get("localizations", {})
        own_locs = own.get(key) or {}
        if not apple_locs and not own_locs:
            missing.append(key)
            continue
        for lang in LANGS:
            # The web's own file wins: it is where the web says a string the
            # apps do not, and where it says one BETTER than the apps do —
            # English "1 attachment" for a key whose English is the plural.
            if lang in own_locs:
                forms = web_forms(own_locs[lang])
            else:
                forms = apple_forms(key, lang, apple_locs.get(lang))
            if not forms:
                continue
            if len(forms) == 1 and forms[0][0] == "one form":
                singular[lang].append((key, forms[0][1]))
            else:
                plural[lang].append((key, forms))

    out = ['//! GENERATED — do not edit. `web/i18n/generate.py` writes it',
           '//! from the apps\' `Localizable.xcstrings` (the nine languages\' source of',
           '//! truth) plus `web/i18n/web.json` (the web\'s own strings). A key is the',
           '//! ENGLISH source string, so a key with no entry here IS its English.',
           '',
           '/// One language\'s strings, sorted by key so a lookup is a binary search.',
           'pub(super) type Table = &\'static [(&\'static str, &\'static str)];',
           '',
           '/// One language\'s plural strings: the CLDR categories it uses, each with',
           '/// its text, sorted by key.',
           'pub(super) type Plurals = &\'static [(&\'static str, &\'static [(&\'static str, &\'static str)])];',
           '']
    for lang in LANGS:
        rows = sorted(singular[lang])
        out.append(f'pub(super) static {IDENT[lang]}: Table = &[')
        for key, value in rows:
            out.append(f'    ({rust(key)}, {rust(value)}),')
        out.append('];')
        out.append('')
        rows = sorted(plural[lang])
        out.append(f'pub(super) static {IDENT[lang]}_PLURAL: Plurals = &[')
        for key, forms in rows:
            forms_src = ", ".join(f'({rust(cat)}, {rust(text)})' for cat, text in forms)
            out.append(f'    ({rust(key)}, &[{forms_src}]),')
        out.append('];')
        out.append('')
    target = repo / "web/text/src/i18n/catalogue.rs"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text("\n".join(out))
    # One row per line is how this is written and not how it is kept: the
    # file is checked in rustfmt's shape like every other.
    formatted = subprocess.run(
        ["cargo", "fmt"], cwd=repo / "web/text", capture_output=True, text=True)
    if formatted.returncode != 0:
        print("!! cargo fmt failed; run it by hand:", formatted.stderr.strip()[:200])

    if "--sync" in sys.argv:
        # Every key nothing says yet is written down as the web's own, so
        # the file IS the list a translator works from.
        for key in missing:
            own.setdefault(key, {})
        own_file["strings"] = dict(sorted(own.items()))
        own_path.write_text(json.dumps(own_file, ensure_ascii=False, indent=2) + "\n")
        print(f"web.json now names {len(own)} strings of the web's own")

    # A translation that lost a placeholder drops an argument SILENTLY —
    # the sentence still reads, with a name or a number missing from it.
    shapes = []
    for lang in LANGS:
        for key, value in singular[lang] + [(k, f[0][1]) for k, f in plural[lang]]:
            if placeholders(value) != placeholders(key):
                shapes.append((lang, key, value))
    if shapes:
        print(f"{len(shapes)} translations do not take the same arguments as their key:")
        for lang, key, value in shapes:
            print(f"    {lang}: {key[:60]!r}\n       -> {value[:60]!r}")

    stale = sorted(set(own) - wanted)
    if stale:
        print(f"web.json names {len(stale)} strings the web no longer asks for:")
        for key in stale:
            print("   ", key[:90])

    print(f"keys the web asks for: {len(wanted)}")
    print(f"translated (Apple or web.json): {len(wanted) - len(missing)}")
    print(f"English only, no entry anywhere: {len(missing)}")
    for key in missing[:20]:
        print("   ", key[:90])
    for lang in LANGS:
        print(f"  {lang}: {len(singular[lang])} strings, {len(plural[lang])} plural")
    return 0


if __name__ == "__main__":
    sys.exit(main())
