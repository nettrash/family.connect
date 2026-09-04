#!/bin/bash
#
# Capture Mac App Store screenshots of the Mac app.
#
#   ios/scripts/capture-mac-screenshot.sh start        # build + launch, isolated
#   ios/scripts/capture-mac-screenshot.sh shot <name> [title]  # capture a window
#   ios/scripts/capture-mac-screenshot.sh stop         # quit, restore, clean up
#
# WHY A SCRIPT AND NOT A UI TEST. The iOS and iPad sets come from
# StoreScreenshotUITests, but the UI-test target does not run on macOS
# here — with macosx added to SUPPORTED_PLATFORMS the app launches with no
# window and the runner sees it as Disabled. So the navigation is manual
# and only the capture is automated. Everything else below exists to make
# that manual step safe to perform on a machine that also runs the real app.
#
# WHAT THIS METHOD CANNOT DO. A window-only capture contains the window's
# own backing store and nothing behind it, so any translucent material has
# no backdrop to blur and renders flat grey. That is visible in the Family
# and Settings sheets, whose header and footer bars are materials. The
# board's grey canvas is NOT that — it is Color(nsColor:
# .underPageBackgroundColor), which is genuinely grey. Fixing the sheets
# would need a screen-region capture, which this script deliberately never
# does; retake those two by hand if the Mac listing ever ships.
#
# WHY THE CAPTURE IS SAFE. It never photographs the screen or a region —
# only one window, by id, whose backing store is rendered on its own.
# Another application's pixels cannot appear in the output even if that
# window sits on top. It also REFUSES unless the window's title is the
# seeded demo family, so a real family's conversation cannot reach a store
# listing.
#
# WHY THE APP IS ISOLATED, AND WHAT ISOLATION DOES NOT COVER. `start`
# builds with a throwaway bundle id, which gives the screenshot build its
# own preferences domain and its own Keychain service, so signing it into
# the demo server cannot disturb a real session. It does NOT separate the
# SwiftData store: this app is not sandboxed in a local build, so its store
# is ~/Library/Application Support/default.store for every bundle id alike.
# `start` therefore backs that file up and `stop` restores it — without
# that, syncing the demo family checkpoints into the real cache on quit.
# (Measured: a few minutes of demo sync produced a 1.5 MB WAL against it.)
#
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
APP_DIR="${FC_MAC_BUILD:-/tmp/fc-mac-shots}"
APP="$APP_DIR/Build/Products/Debug/FamilyConnect.app"
BUNDLE_ID="me.nettrash.FamilyConnect.shots"
SERVER="${FC_SERVER:-http://127.0.0.1:8091}"
USER_NAME="${FC_USER:-nora}"
USER_PASS="${FC_PASS:-password123}"
OUT="${FC_SHOT_DIR:-$REPO/ios/docs/screenshots/mac}"
STORE="$HOME/Library/Application Support/default.store"
BACKUP="$APP_DIR/store-backup"
OWNER="Family"          # the DISPLAY name, not "FamilyConnect"
EXPECT="The Harpers"    # the seeded family — the guard

case "${1:-}" in
start)
  command -v swift >/dev/null || { echo "swift is required" >&2; exit 1; }
  curl -sf -m 3 "$SERVER/api/v1/healthz" >/dev/null || {
    echo "The seeded server is not answering at $SERVER." >&2
    echo "Run server/scripts/seed-store-screenshots.sh first." >&2
    exit 1
  }

  echo "building with bundle id $BUNDLE_ID (isolated prefs + Keychain)…"
  ( cd "$REPO/ios" && DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}" \
    xcodebuild build -project FamilyConnect.xcodeproj -scheme FamilyConnect \
      -destination 'platform=macOS' -configuration Debug \
      PRODUCT_BUNDLE_IDENTIFIER="$BUNDLE_ID" \
      CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO \
      -derivedDataPath "$APP_DIR" -quiet )

  # Force the light appearance so the Mac set matches the iPhone and iPad
  # sets, which are captured on a light simulator. Patched into the
  # throwaway product only — the checked-in Info.plist is untouched.
  /usr/libexec/PlistBuddy -c 'Add :NSRequiresAquaSystemAppearance bool true' \
    "$APP/Contents/Info.plist" 2>/dev/null || true

  mkdir -p "$BACKUP"
  cp -p "$STORE" "$BACKUP/default.store"
  echo "backed up the shared SwiftData store"

  # Sign in without typing: mint a token against the demo server and put it
  # in the Keychain under the THROWAWAY service. -A allows the unsigned
  # build to read it without a GUI prompt; `stop` deletes it again.
  TOKEN="$(curl -s -X POST "$SERVER/api/v1/auth/login" -H 'content-type: application/json' \
    -d "{\"username\":\"$USER_NAME\",\"password\":\"$USER_PASS\"}" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')"
  UID_="$(curl -s -X POST "$SERVER/api/v1/auth/login" -H 'content-type: application/json' \
    -d "{\"username\":\"$USER_NAME\",\"password\":\"$USER_PASS\"}" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["user"]["id"])')"
  security add-generic-password -A -U -s "$BUNDLE_ID" -a session-token -w "$TOKEN"

  # `open -n`, NOT the binary directly: exec'ing Contents/MacOS/FamilyConnect
  # yields a process that never opens a window, because it is not started
  # through LaunchServices. The server URL rides the NSArgumentDomain, which
  # is volatile — it is never written to any preferences file.
  open -n "$APP" --args -v1.serverURL "$SERVER" -v1.currentUserID "$UID_"
  echo "launched; waiting for the window…"
  for _ in $(seq 1 20); do
    sleep 2
    swift "$HERE/mac-window.swift" "$OWNER" "$EXPECT" >/dev/null 2>&1 && break
  done
  cat <<MSG

  Ready, signed in as $USER_NAME on $SERVER.
  Navigate to a screen, then:
      ios/scripts/capture-mac-screenshot.sh shot 01-family-chat

  When finished — this also restores the real store:
      ios/scripts/capture-mac-screenshot.sh stop
MSG
  ;;

shot)
  NAME="${2:?usage: capture-mac-screenshot.sh shot <name> [window title]}"
  # Board, Family and Settings open their own windows, so name the one you
  # want; without it the largest window wins, which is the chat.
  WANT="${3:-}"
  # Select by the pid of the build THIS script launched, not by window
  # title. The real app and this one share the display name "Family", so a
  # title guard was the only thing keeping a real family out of a store
  # listing — a pid guard is stricter, and unlike a title it still holds
  # once the app is showing Board, Family or Settings.
  SHOT_PID="$(pgrep -f "$APP_DIR.*FamilyConnect" | head -1)"
  if [ -z "$SHOT_PID" ]; then
    echo "The screenshot build is not running. Run 'start' first." >&2
    exit 1
  fi
  ROWS="$(swift "$HERE/mac-window.swift" --pid "$SHOT_PID" ${WANT:+"$WANT"} || true)"
  if [ -z "$ROWS" ]; then
    echo "The screenshot build (pid $SHOT_PID) has no window${WANT:+ titled \"$WANT\"} on screen." >&2
    swift "$HERE/mac-window.swift" --pid "$SHOT_PID" >&2 || true
    exit 1
  fi
  # Belt and braces: the FIRST capture of a run must be the seeded family,
  # which proves the build really is signed into the demo server and not
  # showing somebody's own data.
  if [ ! -f "$OUT/.verified" ]; then
    swift "$HERE/mac-window.swift" --pid "$SHOT_PID" | grep -q "$EXPECT" || {
      echo "No window of the screenshot build mentions '$EXPECT'." >&2
      echo "REFUSING: it does not look signed into the seeded demo family." >&2
      exit 1
    }
    mkdir -p "$OUT"; touch "$OUT/.verified"
  fi

  WID="$(printf '%s\n' "$ROWS" | head -1 | cut -f1)"
  # A non-numeric or empty id would shift screencapture's argv and turn
  # `-l <id> -o` into `-l -o`, which captures the SCREEN. Refuse first.
  case "$WID" in
    ''|*[!0-9]*) echo "window id '$WID' is not a number — refusing" >&2; exit 1 ;;
  esac

  mkdir -p "$OUT"
  # Raise the app first: an inactive window draws grey traffic lights and a
  # muted title bar, and the whole set then looks like screenshots of an app
  # nobody was using. Running this script from a terminal is exactly what
  # makes the window inactive, so re-raise it here rather than trusting the
  # state left by whatever opened the screen.
  osascript -e 'tell application "System Events" to tell process "'"$OWNER"'" to set frontmost to true' >/dev/null 2>&1 || true
  sleep 2
  # -l <id>  one window, by id.  NEVER -R (region) and NEVER -S, which means
  #          "capture the screen instead of the window".
  # -o       no drop shadow, so the PNG is exactly the window bounds.
  # -x       no shutter sound.
  screencapture -l "$WID" -o -x "$OUT/$NAME.png"

  # The Mac App Store accepts only 1280x800, 1440x900, 2560x1600 or
  # 2880x1800 — a window capture is never one of those, and SwiftUI windows
  # refuse `set bounds` over Apple Events (-10006), so the window cannot
  # simply be sized to suit. Composite the real, unmodified window onto a
  # 2560x1600 backdrop instead, which is what Mac listings do anyway.
  python3 - "$OUT/$NAME.png" <<'PY'
import sys
from PIL import Image, ImageDraw, ImageFilter

path = sys.argv[1]
src = Image.open(path).convert("RGBA")
W, H = 2560, 1600
if src.width > W or src.height > H:
    src.thumbnail((W - 160, H - 160), Image.LANCZOS)

bg = Image.new("RGB", (W, H))
draw = ImageDraw.Draw(bg)
top, bottom = (24, 84, 163), (11, 46, 102)      # the app's brand blue
for y in range(H):
    t = y / (H - 1)
    draw.line([(0, y), (W, y)],
              fill=tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)))

x, y = (W - src.width) // 2, (H - src.height) // 2
shadow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
ImageDraw.Draw(shadow).rounded_rectangle(
    [x + 8, y + 18, x + src.width + 8, y + src.height + 18], radius=24, fill=(0, 0, 0, 120))
shadow = shadow.filter(ImageFilter.GaussianBlur(30))

out = Image.alpha_composite(bg.convert("RGBA"), shadow)
out.alpha_composite(src, (x, y))
out = out.convert("RGB")
out.save(path, "PNG", optimize=True)

if out.size not in [(1280, 800), (1440, 900), (2560, 1600), (2880, 1800)]:
    print(f"  REJECTED {path}: {out.size} is not a Mac App Store size")
    sys.exit(1)
print(f"  {path}  ({out.width}x{out.height}, no alpha)")
PY
  ;;

stop)
  osascript -e "tell application id \"$BUNDLE_ID\" to quit" >/dev/null 2>&1 || true
  sleep 2
  pkill -9 -f "$APP_DIR.*FamilyConnect" 2>/dev/null || true
  sleep 1
  # Discard the demo family's writes and put the real cache back. The WAL
  # must go BEFORE the store is restored, or SQLite replays it over the top.
  if [ -f "$BACKUP/default.store" ]; then
    rm -f "$STORE-wal" "$STORE-shm"
    cp -p "$BACKUP/default.store" "$STORE"
    echo "restored the real SwiftData store"
  fi
  rm -f "$OUT/.verified"
  security delete-generic-password -s "$BUNDLE_ID" -a session-token >/dev/null 2>&1 \
    && echo "removed the throwaway Keychain item"
  defaults delete "$BUNDLE_ID" >/dev/null 2>&1 || true
  echo "done — the real app's preferences, Keychain and cache are untouched"
  ;;

*)
  echo "usage: capture-mac-screenshot.sh {start|shot <name>|stop}" >&2
  exit 2
  ;;
esac
