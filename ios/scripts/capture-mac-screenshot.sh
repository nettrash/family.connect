#!/bin/bash
#
# Capture Mac App Store screenshots of the Mac app.
#
#   ios/scripts/capture-mac-screenshot.sh start        # build + launch, isolated
#   ios/scripts/capture-mac-screenshot.sh shots        # guided: the whole 1.1 set
#   ios/scripts/capture-mac-screenshot.sh shot <name> [title]  # one window
#   ios/scripts/capture-mac-screenshot.sh stop         # quit and clean up
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
# WHY THE APP IS ISOLATED, AND HOW FAR THAT NOW GOES. `start` builds with a
# throwaway bundle id, which gives the screenshot build its own preferences
# domain and its own Keychain service, and launches it under a throwaway
# HOME, which gives it its own SwiftData store. That last part is what lets
# this run WHILE THE REAL APP IS OPEN. A local build is not sandboxed, so
# its store is $HOME/Library/Application Support/default.store — one path
# for every bundle id alike — and the old answer was to back that file up
# and restore it on `stop`. That is a rollback of whatever the real app
# wrote in between, and a write race against it if it happens to be
# running: two processes on one SQLite store, one of them being quit with
# SIGKILL. `open --env HOME=…` sidesteps both; `start` VERIFIES the
# redirect took — the store must appear where it was asked for, and the
# process must hold no handle on the real one — and quits rather than
# proceed if it did not, because the failure mode is writing the demo
# family into somebody's real cache. (HOME cannot do this job: LaunchServices
# drops it, and Foundation reads the home directory from the user record
# anyway. The app takes a DEBUG-only `-v1.storeURL` for exactly this.)
# (Measured before: a few minutes of demo sync produced a 1.5 MB WAL.)
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
FAKE_STORE="${FC_MAC_STORE:-$APP_DIR/store/default.store}"
REAL_STORE="$HOME/Library/Application Support/default.store"
PIDFILE="$APP_DIR/shots.pid"
OWNER="Family"          # the DISPLAY name, not "FamilyConnect"
EXPECT="The Harpers"    # the seeded family — the guard

# Launch the throwaway build, optionally asking it to open ONE screen
# (-v1.showScreen, DEBUG-only — see MacScreenshotRoute.swift). Replaces the
# instance this script started, records the new pid, waits for a window.
#
# `open -n`, NOT the binary directly: exec'ing Contents/MacOS/FamilyConnect
# yields a process LaunchServices never registered. Every argument rides the
# NSArgumentDomain, which is volatile — none of it reaches a preferences file.
launch_app() {
  ROUTE="$1"
  WHO="$2"
  TOKEN="$(cat "$APP_DIR/token" 2>/dev/null || echo "")"
  if [ -f "$PIDFILE" ] && [ -s "$PIDFILE" ]; then
    OLD="$(cat "$PIDFILE")"
    case "$OLD" in ''|*[!0-9]*) ;; *) kill "$OLD" 2>/dev/null || true ;; esac
    sleep 2
  fi
  open -n "$APP" --args -v1.serverURL "$SERVER" -v1.currentUserID "$WHO" \
    -v1.storeURL "$FAKE_STORE" ${TOKEN:+-v1.sessionToken "$TOKEN"} \
    ${ROUTE:+-v1.showScreen "$ROUTE"}
  for _ in $(seq 1 25); do
    sleep 1
    NEW="$(pgrep -f "$APP_DIR.*FamilyConnect" | head -1)"
    [ -n "$NEW" ] || continue
    echo "$NEW" > "$PIDFILE"
    swift "$HERE/mac-window.swift" --pid "$NEW" --any >/dev/null 2>&1 && return 0
  done
  return 1
}

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

  # NO WINDOW RESTORATION. macOS reopens whatever was on screen when an app
  # quit, so the Board window from one shot was still open during the next
  # launch and the capture photographed it instead of the chat. The set also
  # names the window it wants, but this removes the trap rather than
  # dodging it. `stop` deletes this domain entirely.
  defaults write "$BUNDLE_ID" NSQuitAlwaysKeepsWindows -bool false

  # A store of its own, so nothing here can reach the real one.
  mkdir -p "$(dirname "$FAKE_STORE")"

  # Sign in without typing, and without the Keychain: mint a token against
  # the demo server and HAND IT TO THE APP (-v1.sessionToken, DEBUG-only).
  #
  # The Keychain route is what this replaces, and it did not work: an
  # unsigned build reading a Keychain item raises the ACL confirmation
  # dialog, `-A` or no `-A`, and every rebuild is a new binary so an earlier
  # "Always Allow" does not carry. When the capture relaunches the app that
  # dialog can open on a Space nobody is looking at, and
  # `SecItemCopyMatching` blocks in `applicationDidFinishLaunching`: the app
  # comes up with no window at all, every shot reports "no window", and
  # nothing says why. Measured with `sample` on 2026-09-17.
  TOKEN="$(curl -s -X POST "$SERVER/api/v1/auth/login" -H 'content-type: application/json' \
    -d "{\"username\":\"$USER_NAME\",\"password\":\"$USER_PASS\"}" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')"
  UID_="$(curl -s -X POST "$SERVER/api/v1/auth/login" -H 'content-type: application/json' \
    -d "{\"username\":\"$USER_NAME\",\"password\":\"$USER_PASS\"}" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["user"]["id"])')"
  echo "$TOKEN" > "$APP_DIR/token"
  chmod 600 "$APP_DIR/token"

  echo "$UID_" > "$APP_DIR/userid"   # `shots` relaunches with it
  echo "launching; waiting for the window…"
  launch_app "" "$UID_" || echo "  (no window yet — 'shots' will wait again)" >&2

  # THE REDIRECT IS LOAD-BEARING: verify it rather than trust it. If the
  # store did not appear under the throwaway home, this build is writing
  # the demo family into the real cache — which may also be open in the
  # real app right now — so stop immediately.
  SHOT_PID="$(cat "$PIDFILE" 2>/dev/null || true)"
  BAD=""
  [ -f "$FAKE_STORE" ] || BAD="the throwaway store never appeared at $FAKE_STORE"
  # The stricter half: whatever it opened, it must not be the real cache.
  if [ -n "$SHOT_PID" ] && lsof -p "$SHOT_PID" 2>/dev/null | grep -qF "$REAL_STORE"; then
    BAD="the build has the REAL store open ($REAL_STORE)"
  fi
  if [ -n "$BAD" ]; then
    echo "REFUSING: $BAD." >&2
    echo "Quitting rather than write demo data into the real cache." >&2
    "$0" stop >&2
    exit 1
  fi
  echo "store isolated at $FAKE_STORE — the real one is neither opened nor touched"

  cat <<MSG

  Ready, signed in as $USER_NAME on $SERVER (the real app may stay open).
  Navigate to a screen, then:
      ios/scripts/capture-mac-screenshot.sh shot 01-family-chat

  Or take the whole 1.1 set unattended — it relaunches once per screen:
      ios/scripts/capture-mac-screenshot.sh shots

  When finished:
      ios/scripts/capture-mac-screenshot.sh stop
MSG
  ;;

shots)
  # THE 1.1 SET, unattended: one relaunch per screen, because from a terminal
  # the screens cannot be reached any other way — clicking needs Accessibility
  # permission, and macOS 14+ refuses cross-app activation from a background
  # process, while `-v1.showScreen` needs neither. Lines are
  # "name|window title|route": an empty route is the chat itself. EVERY line
  # names its window, because "the first window that photographed
  # non-blank" is not a rule: macOS restores windows across launches, so the
  # Board window from a previous run was open during the first shot and
  # `01-family-chat` came out as a picture of the board. Naming the window
  # is what makes each shot the screen its filename claims (#55).
  [ -f "$PIDFILE" ] || { echo "Run 'start' first." >&2; exit 1; }
  WHO="$(cat "$APP_DIR/userid" 2>/dev/null || echo "")"
  SET="01-family-chat|$EXPECT|
02-thread|$EXPECT|thread
03-open-polls|$EXPECT|polls
04-board|Board|board
05-family|$EXPECT|family
06-settings|Settings|settings"
  MISSED=0
  while IFS='|' read -r name title route; do
    [ -n "$name" ] || continue
    printf '  %-16s' "$name"
    if ! launch_app "$route" "$WHO"; then
      echo "no window — skipped" >&2; MISSED=$((MISSED + 1)); continue
    fi
    # Nine seconds, not five: the thread route WAITS for its chain to
    # arrive (MacScreenshotRoute — a cold launch has an empty cache), and a
    # sheet photographed before it exists is the blank frame
    # mac-shot-usable.py now refuses.
    sleep 9
    if "$0" shot "$name" ${title:+"$title"} >/dev/null 2>&1; then
      echo "captured"
    else
      echo "NOT captured" >&2
      MISSED=$((MISSED + 1))
    fi
  done <<SETEOF
$SET
SETEOF
  echo
  if [ "$MISSED" -eq 0 ]; then
    # The 1.0 set used 02-board, 03-family, 04-settings; 1.1 renumbers those
    # to 04, 05, 06 and adds the thread and the open-polls screens. Two
    # numberings in one directory is how the wrong file gets uploaded (#55),
    # so retire the old names — but only with six new ones on disk.
    for stale in 02-board 03-family 04-settings; do
      [ -f "$OUT/$stale.png" ] && rm -f "$OUT/$stale.png" && echo "  retired the 1.0 name $stale.png"
    done
    echo "  six screenshots in $OUT"
  else
    echo "  $MISSED of six did not land — the 1.0 names are kept until they do." >&2
  fi
  echo "  LOOK at them before uploading, then: $0 stop"
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
  # The pid `start` recorded, falling back to a match on the build
  # directory — never a bare name, which the real app also answers to.
  SHOT_PID="$(cat "$PIDFILE" 2>/dev/null || true)"
  if [ -z "$SHOT_PID" ] || ! ps -p "$SHOT_PID" >/dev/null 2>&1; then
    SHOT_PID="$(pgrep -f "$APP_DIR.*FamilyConnect" | head -1)"
  fi
  if [ -z "$SHOT_PID" ]; then
    echo "The screenshot build is not running. Run 'start' first." >&2
    exit 1
  fi
  # RAISE FIRST, THEN LOOK. `--raise` goes through NSRunningApplication
  # (see mac-window.swift): public API, no Accessibility permission, and it
  # takes a pid, which is the only way to tell this build from the real
  # app. Two reasons it comes before the lookup rather than after:
  # a window that is on another Space, or behind a full-screen app, is not
  # "on screen" and the lookup reports the build as windowless; and an
  # inactive window photographs with grey traffic lights and a muted title
  # bar. The old code raised afterwards, with System Events, naming a
  # process "Family" — a name only CoreGraphics uses here — through an API
  # that fails with -25211 without Accessibility, swallowed by `|| true`.
  swift "$HERE/mac-window.swift" --raise "$SHOT_PID" || \
    echo "  (could not raise pid $SHOT_PID — it may photograph inactive)" >&2
  ROWS="$(swift "$HERE/mac-window.swift" --pid "$SHOT_PID" --any ${WANT:+"$WANT"} || true)"
  if [ -z "$ROWS" ]; then
    echo "The screenshot build (pid $SHOT_PID) has no window${WANT:+ titled \"$WANT\"} on screen." >&2
    echo "Bring it forward and try again: the capture needs the window on the ACTIVE Space." >&2
    echo "A terminal cannot do that for you — macOS 14 and later refuse cross-app activation" >&2
    echo "from a background process, and --raise is best-effort for that reason." >&2
    swift "$HERE/mac-window.swift" --pid "$SHOT_PID" --any >&2 || true
    exit 1
  fi
  # Belt and braces: the FIRST capture of a run must be the seeded family,
  # which proves the build really is signed into the demo server and not
  # showing somebody's own data.
  if [ ! -f "$OUT/.verified" ]; then
    swift "$HERE/mac-window.swift" --pid "$SHOT_PID" --any | grep -q "$EXPECT" || {
      echo "No window of the screenshot build mentions '$EXPECT'." >&2
      echo "REFUSING: it does not look signed into the seeded demo family." >&2
      exit 1
    }
    mkdir -p "$OUT"; touch "$OUT/.verified"
  fi

  # EVERY CANDIDATE, NOT THE FIRST. With --any the window list is no longer
  # guaranteed front-to-back, and a window that has never been DISPLAYED has
  # no backing store, so `screencapture` returns a transparent frame that
  # composites into a blank blue rectangle. That is how `02-thread` came out
  # empty while five other screens came out right. So: photograph each
  # candidate in turn and keep the first frame with a picture in it.
  mkdir -p "$OUT"
  sleep 1
  CAPTURED=""
  while IFS=$'\t' read -r WID SIZE TITLE; do
    case "$WID" in
      ''|*[!0-9]*) continue ;;   # a non-numeric id would shift screencapture's argv
    esac
    TMP="$APP_DIR/shot-$NAME.png"
    rm -f "$TMP"
    # -l <id>  one window, by id.  NEVER -R (region) and NEVER -S, which
    #          means "capture the screen instead of the window".
    # -o       no drop shadow, so the PNG is exactly the window bounds.
    # -x       no shutter sound.
    screencapture -l "$WID" -o -x "$TMP" 2>/dev/null || continue
    [ -s "$TMP" ] || continue
    if python3 "$HERE/mac-shot-usable.py" "$TMP"; then
      mv "$TMP" "$OUT/$NAME.png"
      CAPTURED="$WID"
      break
    fi
    rm -f "$TMP"
  done <<ROWSEOF
$ROWS
ROWSEOF
  if [ -z "$CAPTURED" ]; then
    echo "Every window of the screenshot build photographed blank for $NAME." >&2
    echo "A window that has never been displayed has no backing store to copy;" >&2
    echo "give the screen longer to draw, or bring the window forward once." >&2
    exit 1
  fi

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
  # `with timeout`: a quit sent to a wedged app blocks for the default two
  # minutes, which is how `stop` came to look like a hang of its own.
  osascript -e "with timeout of 5 seconds
  tell application id \"$BUNDLE_ID\" to quit
end timeout" >/dev/null 2>&1 || true
  sleep 2
  # By PID, from the file `start` wrote — never a pattern. The real app is
  # very likely running too, and `pkill -f` on a shared machine is how the
  # wrong process gets killed.
  if [ -f "$PIDFILE" ] && [ -s "$PIDFILE" ]; then
    SHOT_PID="$(cat "$PIDFILE")"
    case "$SHOT_PID" in
      ''|*[!0-9]*) ;;
      *) kill -9 "$SHOT_PID" 2>/dev/null || true ;;
    esac
    rm -f "$PIDFILE"
  fi
  sleep 1
  # Nothing to restore: the demo family's rows went to the throwaway
  # store, which goes with the build directory.
  rm -f "$FAKE_STORE" "$FAKE_STORE-wal" "$FAKE_STORE-shm"
  rm -f "$OUT/.verified"
  rm -f "$APP_DIR/token" "$APP_DIR/userid"
  # Older runs of this script DID put a token in the Keychain; clear it if
  # one is still there, so nothing of this is left behind.
  security delete-generic-password -s "$BUNDLE_ID" -a session-token >/dev/null 2>&1 \
    && echo "removed a throwaway Keychain item left by an older run"
  defaults delete "$BUNDLE_ID" >/dev/null 2>&1 || true
  echo "done — the real app's preferences, Keychain and cache are untouched"
  ;;

*)
  echo "usage: capture-mac-screenshot.sh {start|shots|shot <name> [title]|stop}" >&2
  exit 2
  ;;
esac
