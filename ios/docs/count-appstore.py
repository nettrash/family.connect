#!/usr/bin/env python3
"""Measure every App Store Connect field in appstore.md — twice.

    python3 ios/docs/count-appstore.py [--fill]

Once as the file is written, and once with every `[PLACEHOLDER]` carrying a value
of the length it will really have. THE SECOND MEASUREMENT IS THE ONE THAT MATTERS
and the reason this exists: the review notes were written to 3,988 of the 4,000
characters App Store Connect allows, and `[AI_PROCESSOR]` alone is 27 characters
longer than its placeholder — so the field that "fitted" was 4,004 when filled,
and the Mac's was 4,027. App Store Connect truncates notes over the limit
without saying so, and what falls off the end of these notes is the AI-consent
paragraph and the privacy-policy URL: the two things guideline 5.1.1(i) asks for.

Exits non-zero when a field is over, so it can guard a change to the copy. The
placeholder VALUES are lengths, not secrets — the real demo password goes in
App Store Connect and never in this repository (see the DEMO ACCOUNTS block).
"""
import pathlib
import re
import sys

# Only the length of each stands in for the real value; see the docstring.
STAND_INS = {
    "[AI_PROCESSOR]": "Microsoft — Azure OpenAI (Sweden Central)",
    "[DEMO_USER]": "TEST_1",
    "[DEMO_USER_2]": "TEST_2",
    "[DEMO_PASS]": "x" * 16,
    "[DEMO_PASS_2]": "x" * 16,
    "[INVITE_CODE]": "XXXXXXXX",
    "[SUPPORT_EMAIL]": "nettrash@nettrash.me",
    "[DEMO_SERVER_URL]": "https://fc.nettrash.me",
}

# Every field with a limit App Store Connect enforces, in the order the console asks.
FIELDS = [
    ("Subtitle", 30),
    ("Promotional Text", 170),
    ("What's New in 1.1", 4000),
    ("Description", 4000),
    ("Keywords", 100),
    ("Notes for App Review", 4000),
    ("Beta App Description", 4000),
    ("What to Test", 4000),
    ("macOS Promotional Text", 170),
    ("macOS What's New in 1.1", 4000),
    ("macOS — Notes for App Review", 4000),
    ("macOS Beta App Description", 4000),
    ("macOS What to Test", 4000),
]


def body(text: str, heading: str) -> str | None:
    """The pasteable text under one heading.

    The editor's own notes are not copy: an italic block (possibly several lines
    of one), a quoted block, a checklist row and a table are all dropped. A line
    that opens an italic block it never closes is kept, because `*Minimum*` at
    the start of a sentence is prose and swallowing the rest would measure a
    field as nothing.
    """
    found = re.search(
        r"(?ms)^#{2,3} " + re.escape(heading) + r"[^\n]*\n(.*?)(?=^#{2,3} |\Z)", text)
    if not found:
        return None
    lines = found[1].split("\n")
    kept: list[str] = []
    at = 0
    while at < len(lines):
        line = lines[at].strip()
        if line.startswith((">", "- [", "|")):
            at += 1
            continue
        if line.startswith("*") and not line.startswith("**"):
            closes = next(
                (a for a in range(at, min(len(lines), at + 12))
                 if lines[a].strip().endswith("*") and not lines[a].strip().endswith("**")),
                None)
            if closes is not None:
                at = closes + 1
                continue
        kept.append(lines[at])
        at += 1
    return "\n".join(kept).strip()


def main() -> int:
    path = pathlib.Path(__file__).with_name("appstore.md")
    text = path.read_text(encoding="utf-8")
    failed = False
    print(f"{'field':34}{'written':>9}{'filled':>8}{'limit':>7}  ")
    for heading, limit in FIELDS:
        raw = body(text, heading)
        if raw is None:
            print(f"{heading:34}{'MISSING':>9}")
            failed = True
            continue
        filled = raw
        for placeholder, stand_in in STAND_INS.items():
            filled = filled.replace(placeholder, stand_in)
        state = "ok"
        if len(filled) > limit:
            state = f"OVER by {len(filled) - limit} once filled"
            failed = True
        elif not raw:
            state = "EMPTY"
            failed = True
        print(f"{heading:34}{len(raw):>9}{len(filled):>8}{limit:>7}  {state}")
    # `[PLACEHOLDER]` is the word the header uses for all of them, not one of them.
    left = sorted(set(re.findall(r"\[[A-Z_0-9]+\]", text)) - {"[PLACEHOLDER]"})
    print(f"\nplaceholders still to fill in App Store Connect: {', '.join(left) if left else 'none'}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
