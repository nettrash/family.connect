"""Turn the push sentences into the server's table, src/push_words.rs.

A push is read on a lock screen whose app is not running, so its words are the
server's, and they go out in the language each DEVICE registered
(docs/protocol.md, "The words of a push"; issue #82). The words are the apps'
own: the apps' `ios/FamilyConnect/Localizable.xcstrings` holds the nine
localisations of every sentence a push shares with them ("Photo", "3 Photos",
"New message"), and `server/i18n/push.json` holds the few the apps never say
("New report", "%@ asked to join").

    python3 server/i18n/generate.py <repo>           # writes src/push_words.rs
    python3 server/i18n/generate.py <repo> --check   # CI: writes nothing, exits 1 if owed

A key the push code uses that is missing a language — in either source — is an
error, not English for now: these are fifteen short sentences, and a lock
screen is where a half-translated app looks most broken.
"""
import json
import pathlib
import re
import subprocess
import sys

LANGS = ["de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans"]

# Every sentence push_payload.rs says through `Text::say` — and only those.
KEYS = [
    "Photo", "Sticker", "Video", "Video message", "Audio", "Location", "File",
    "%lld Photos", "%lld Videos", "%lld Audio", "%lld Files", "%lld attachments",
    "New message", "New note", "New report",
    "%@ — %@ mentioned you", "%@ asked to join", "You're in — welcome to %@",
]


def placeholders(text: str) -> list[str]:
    return re.findall(r"%(?:\d+\$)?(?:lld|@)", text)


def rust(text: str) -> str:
    return json.dumps(text, ensure_ascii=False)


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    checking = "--check" in sys.argv
    repo = pathlib.Path(args[0] if args else ".").resolve()
    apple = json.load(open(repo / "ios/FamilyConnect/Localizable.xcstrings"))["strings"]
    own = json.load(open(repo / "server/i18n/push.json"))["strings"]

    owed, rows = [], []
    for key in KEYS:
        for lang in LANGS:
            said = (own.get(key) or {}).get(lang)
            if said is None:
                unit = (apple.get(key) or {}).get("localizations", {}).get(lang, {}).get("stringUnit", {})
                said = unit.get("value") if unit.get("state") == "translated" else None
            if not said:
                owed.append(f"{lang}: {key!r} has no translation")
                continue
            if placeholders(said) != placeholders(key):
                owed.append(f"{lang}: {key!r} -> {said!r} does not take the same arguments")
                continue
            rows.append((key, lang, said))

    out = [
        "//! GENERATED — do not edit. `server/i18n/generate.py` writes it from the apps'",
        "//! `Localizable.xcstrings` and `server/i18n/push.json` (docs/protocol.md, \"The words",
        "//! of a push\"). A key is the ENGLISH sentence, which is also what a device with no",
        "//! language, or one this table does not know, is sent.",
        "",
        "/// (key, language, words), sorted by key and then language, so a lookup is a binary search.",
        "pub(crate) static TABLE: &[(&str, &str, &str)] = &[",
    ]
    for key, lang, said in sorted(rows):
        out.append(f"    ({rust(key)}, {rust(lang)}, {rust(said)}),")
    out.append("];")
    out.append("")
    out.append("/// Every language the table speaks, besides English.")
    out.append("pub(crate) static LANGUAGES: &[&str] = &[" + ", ".join(rust(l) for l in LANGS) + "];")
    out.append("")
    body = "\n".join(out)

    target = repo / "server/src/push_words.rs"
    before = target.read_text() if target.exists() else None
    target.write_text(body)
    formatted = subprocess.run(["rustfmt", "--edition", "2024", str(target)], capture_output=True, text=True)
    if formatted.returncode != 0:
        print("!! rustfmt failed:", formatted.stderr.strip()[:200])
    stale = target.read_text() != before
    if checking:
        if before is None:
            target.unlink()
        else:
            target.write_text(before)

    print(f"push sentences: {len(KEYS)}, rows: {len(rows)} of {len(KEYS) * len(LANGS)}")
    for line in owed:
        print("   ", line)
    if owed:
        return 1
    if checking and stale:
        print("server/src/push_words.rs is out of date: run this without --check")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
