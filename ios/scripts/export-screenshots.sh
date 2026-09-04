#!/bin/bash
#
# Export the store screenshots from an .xcresult into a named folder.
#
#   ios/scripts/export-screenshots.sh <bundle>.xcresult <output dir>
#
# xcresulttool exports every attachment under an opaque UUID filename and
# a manifest that maps it back; this keeps only the ones the screenshot
# test named (01-…, 02-…) and restores those names.
#
set -euo pipefail
BUNDLE="${1:?usage: export-screenshots.sh <bundle>.xcresult <output dir>}"
OUT="${2:?usage: export-screenshots.sh <bundle>.xcresult <output dir>}"
RAW="$(mktemp -d)"
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
xcrun xcresulttool export attachments --path "$BUNDLE" --output-path "$RAW" >/dev/null
mkdir -p "$OUT"
RAW="$RAW" OUT="$OUT" python3 - <<'EOF'
import json, os, re, shutil
raw, out = os.environ["RAW"], os.environ["OUT"]
manifest = json.load(open(os.path.join(raw, "manifest.json")))
kept = 0
for test in manifest:
    for a in test.get("attachments", []):
        name = a.get("suggestedHumanReadableName", "")
        # The test names its own captures "01-chats", "02-family-chat", …
        # Everything else in the bundle is XCUITest's own diagnostics.
        if not re.match(r"^\d\d-", name):
            continue
        clean = re.sub(r"_\d+_[0-9A-F-]{36}\.png$", "", name)
        shutil.copy(os.path.join(raw, a["exportedFileName"]),
                    os.path.join(out, clean + ".png"))
        kept += 1
print(f"  {kept} screenshots → {out}")
EOF
rm -rf "$RAW"
