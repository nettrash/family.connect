#!/bin/bash
#
# Capture ONE window of the Mac app for the Mac App Store.
#
#   ios/scripts/capture-mac-screenshot.sh <name> [output dir]
#
# WHY A SCRIPT AND NOT A UI TEST. The iOS and iPad sets are captured by
# StoreScreenshotUITests, but the UI-test target does not run on macOS
# here — with macosx added to SUPPORTED_PLATFORMS the app launches with
# no window and the runner sees it as Disabled. Rather than ship a broken
# harness, the Mac capture is this: you drive the app, this takes the
# picture. Only the navigation is manual; the capture is exact and
# repeatable.
#
# WHY IT IS SAFE. It never photographs the screen or a region — only one
# window, by id, whose backing store is rendered on its own. Another
# application's pixels cannot appear in the output even if that window is
# on top. It also REFUSES unless the window's title is the seeded demo
# family, so a real family's conversation cannot reach a store listing.
#
# Before running:
#   1. server/scripts/seed-store-screenshots.sh
#   2. Open the Mac app, point it at http://127.0.0.1:8091, sign in as
#      nora / password123, and navigate to the screen you want.
#
set -euo pipefail

NAME="${1:?usage: capture-mac-screenshot.sh <name> [output dir]}"
OUT="${2:-ios/docs/screenshots/mac}"
OWNER="Family"          # the DISPLAY name, not "FamilyConnect"
EXPECT="The Harpers"    # the seeded family — the guard

HERE="$(cd "$(dirname "$0")" && pwd)"
ROWS="$(swift "$HERE/mac-window.swift" "$OWNER" "$EXPECT" || true)"

if [ -z "$ROWS" ]; then
  echo "No window of '$OWNER' titled '$EXPECT' is on screen." >&2
  echo "Windows currently open for '$OWNER':" >&2
  swift "$HERE/mac-window.swift" "$OWNER" >&2 || true
  echo >&2
  echo "REFUSING to capture. Sign the Mac app into the seeded server" >&2
  echo "(http://127.0.0.1:8091, nora / password123) and try again." >&2
  exit 1
fi

WID="$(printf '%s\n' "$ROWS" | head -1 | cut -f1)"
SIZE="$(printf '%s\n' "$ROWS" | head -1 | cut -f2)"
# A non-numeric or empty id would shift screencapture's argv and turn
# `-l <id> -o` into `-l -o`, which captures the SCREEN. Refuse first.
case "$WID" in
  ''|*[!0-9]*) echo "window id '$WID' is not a number — refusing" >&2; exit 1 ;;
esac

mkdir -p "$OUT"
# -l <id>  one window, by id.  NEVER -R (region) and NEVER -S, which means
#          "capture the screen instead of the window".
# -o       no drop shadow, so the PNG is exactly the window bounds.
# -x       no shutter sound.
screencapture -l "$WID" -o -x "$OUT/$NAME.png"
echo "  $OUT/$NAME.png  (window $WID, ${SIZE}pt)"
