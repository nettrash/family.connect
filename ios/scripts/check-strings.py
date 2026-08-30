#!/usr/bin/env python3
"""Every user-facing literal is in the string catalogue, and translated.

WHY THIS EXISTS. `xcodebuild` extracts localizable strings — the
.stringsdata files land in DerivedData and the per-language .strings are
compiled from them — but it NEVER writes new keys back into the source
`Localizable.xcstrings`. That write-back is an Xcode.app behaviour. So a
string added from the command line compiles, runs, and ships in English
for all nine languages, with nothing failing anywhere to say so.

This is the check that says so. Run it after adding any user-facing text:

    python3 ios/scripts/check-strings.py        # from the repo root

Exit status is 1 when something is missing, so it can gate a commit.

WHAT IT CANNOT SEE. It reads source text, not types, so for an
interpolated literal it accepts any specifier combination that resolves —
`"\\(a) of \\(b)"` matches `%@ of %lld` as readily as `%lld of %lld`. It
also cannot see a key built at runtime. Both are rare here and neither is
worth a type checker.
"""

import itertools
import json
import os
import re
import sys

CATALOGUE = "ios/FamilyConnect/Localizable.xcstrings"
SOURCES = "ios/FamilyConnect"
LANGUAGES = ["de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans"]

# Keys that are deliberately English-only: pure format strings with nothing
# to translate, and the sample data placeholders in text fields.
NEVER_TRANSLATED = {
    "%@ %lld", "%@, %@", "%@: %@", "%lld", "+%lld", "@%@",
    "ABCD2345", "Family Connect", "The Smiths", "chat.example.com",
}

# Positions Swift treats as a LocalizedStringKey.
CALLS = [
    "Text", "Button", "Label", "Toggle", "Picker", "Section", "Stepper",
    "LabeledContent", "TextField", "ContentUnavailableView",
]
MODIFIERS = ["navigationTitle", "accessibilityLabel", "alert", "confirmationDialog"]
PATTERNS = (
    [re.compile(rf'\b{name}\(\s*"((?:[^"\\]|\\.)*)"') for name in CALLS]
    + [re.compile(rf'\.{name}\(\s*"((?:[^"\\]|\\.)*)"') for name in MODIFIERS]
    + [re.compile(r'String\(\s*localized:\s*"((?:[^"\\]|\\.)*)"')]
)


def candidate_keys(literal):
    """The catalogue keys a Swift literal could have been extracted as."""
    parts, buffer, i = [], "", 0
    while i < len(literal):
        # A balanced \( ... ), so \(f(x)) is one hole and not two.
        if literal.startswith("\\(", i):
            depth, j = 1, i + 2
            while j < len(literal) and depth:
                depth += (literal[j] == "(") - (literal[j] == ")")
                j += 1
            parts.append(buffer)
            buffer, i = "", j
        else:
            buffer += literal[i]
            i += 1
    parts.append(buffer)
    return [
        "".join(head + spec for head, spec in zip(parts, combo)) + parts[-1]
        for combo in itertools.product(("%@", "%lld"), repeat=len(parts) - 1)
    ]


def main():
    catalogue = json.load(open(CATALOGUE))["strings"]
    missing, untranslated = [], []

    for root, _, files in os.walk(SOURCES):
        for name in sorted(files):
            if not name.endswith(".swift"):
                continue
            path = os.path.join(root, name)
            for number, line in enumerate(open(path).read().splitlines(), 1):
                # Doc comments quote code, and `verbatim:` opts out.
                if line.lstrip().startswith(("//", "*", "/*")) or "verbatim:" in line:
                    continue
                for pattern in PATTERNS:
                    for literal in pattern.findall(line):
                        if not literal.strip():
                            continue
                        keys = candidate_keys(literal)
                        key = next((k for k in keys if k in catalogue), None)
                        if key is None:
                            missing.append((path, number, literal))
                        elif key not in NEVER_TRANSLATED and any(
                            language not in catalogue[key].get("localizations", {})
                            for language in LANGUAGES
                        ):
                            untranslated.append((path, number, key))

    for title, rows in (("not in the catalogue", missing),
                        ("in the catalogue, not translated", untranslated)):
        print(f"=== {len(rows)} {title} ===")
        for path, number, text in rows:
            print(f"  {path}:{number}  {text[:100]}")

    return 1 if missing or untranslated else 0


if __name__ == "__main__":
    sys.exit(main())
