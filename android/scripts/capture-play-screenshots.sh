#!/bin/bash
#
# Capture Google Play store screenshots from an EMULATOR.
#
#   android/scripts/capture-play-screenshots.sh start        # boot + install
#   android/scripts/capture-play-screenshots.sh screens      # drive + capture all 6
#   android/scripts/capture-play-screenshots.sh shot <name>  # capture one screen
#   android/scripts/capture-play-screenshots.sh stop         # leave demo mode
#
# WHY A DEDICATED AVD. The Pixel_8_Pro AVD is 1344x2992, a 2.226:1 ratio.
# Google Play rejects any screenshot whose longer side is more than TWICE
# the shorter one, so every shot taken at that AVD's own resolution is
# refused at upload — which is why the three screenshots committed in
# August cannot be used. This script creates and uses its own AVD at
# 1080x1920 (16:9), inside the limit and the exact ratio Play wants for a
# listing to be eligible for its promotional surfaces.
#
# It is a separate AVD rather than a `wm size` override on an existing one
# for two reasons: `wm size`/`wm density` are STICKY and survive a reboot,
# so a crashed run leaves somebody's emulator permanently mis-sized; and a
# long-lived personal AVD fills up (the Pixel_8_Pro one had 300 MB free,
# too little for the installer) and carries other apps whose notifications
# land in the status bar of a store screenshot. The resize is still applied
# after boot, so pointing FC_AVD at another AVD also produces legal shots.
#
# WHY IT REFUSES A PHYSICAL DEVICE. `adb -e` addresses the emulator, but a
# typo or a missing emulator must never fall through to a phone that
# happens to be plugged in: a framebuffer grab from somebody's own phone
# captures whatever is on it. Every command here is pinned to -e, and the
# script exits if no emulator is running.
#
set -euo pipefail

SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
ADB="$SDK/platform-tools/adb"
EMULATOR="$SDK/emulator/emulator"
AVD="${FC_AVD:-FC_Play_Shots}"
APP_ID="me.nettrash.familyconnect"
# Resolved from the script's own location, not the cwd, so `shot` writes to
# the same place whether it is run from the repo root or from android/.
REPO_ANDROID="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${FC_SHOT_DIR:-$REPO_ANDROID/fastlane/metadata/android/en-US/images/phoneScreenshots}"
# 16:9 — comfortably inside Play's 2:1 ceiling.
WIDTH=1080
HEIGHT=1920
DENSITY=420

# The chosen emulator's serial. Resolved once and used with `adb -s` for
# every command: plain `-e` fails with "more than one emulator" the moment
# a second one is running, and picking by serial is just as safe because
# only serials starting with `emulator-` are ever considered.
SERIAL=""

require_emulator() {
  local devices
  devices="$("$ADB" devices | awk 'NR>1 && $2=="device" {print $1}')"
  SERIAL="$(printf '%s\n' "$devices" | grep '^emulator-' | head -1)"
  if [ -z "$SERIAL" ]; then
    echo "No emulator is running." >&2
    echo "Devices adb can see right now:" >&2
    printf '  %s\n' $devices >&2
    echo >&2
    echo "REFUSING to continue. This script only ever talks to an emulator" >&2
    echo "(-e). It will not capture the framebuffer of a physical phone." >&2
    exit 1
  fi
}

# Create the screenshot AVD if it is not there. Written by hand because
# the SDK here has no cmdline-tools/avdmanager; the emulator generates the
# userdata image itself on first boot from just these two files.
ensure_avd() {
  local dir="$HOME/.android/avd/${AVD}.avd"
  [ -f "$dir/config.ini" ] && return 0
  [ "$AVD" = "FC_Play_Shots" ] || {
    echo "AVD '$AVD' does not exist and only FC_Play_Shots can be auto-created." >&2
    exit 1
  }
  local img="$SDK/system-images/android-36/google_apis_playstore/arm64-v8a"
  [ -d "$img" ] || { echo "missing system image: $img" >&2; exit 1; }
  echo "creating AVD ${AVD} (${WIDTH}x${HEIGHT} @${DENSITY})…"
  mkdir -p "$dir"
  printf 'avd.ini.encoding=UTF-8\npath=%s\npath.rel=avd/%s.avd\ntarget=android-36\n' \
    "$dir" "$AVD" > "$HOME/.android/avd/${AVD}.ini"
  cat > "$dir/config.ini" <<EOF
AvdId=${AVD}
PlayStore.enabled=false
abi.type=arm64-v8a
avd.ini.displayname=${AVD}
avd.ini.encoding=UTF-8
disk.dataPartition.size=8G
fastboot.forceColdBoot=no
fastboot.forceFastBoot=yes
hw.accelerometer=yes
hw.audioInput=yes
hw.battery=yes
hw.camera.back=virtualscene
hw.camera.front=emulated
hw.cpu.arch=arm64
hw.cpu.ncore=4
hw.dPad=no
hw.gps=yes
hw.gpu.enabled=yes
hw.gpu.mode=auto
hw.gyroscope=yes
hw.initialOrientation=portrait
hw.keyboard=yes
hw.lcd.density=${DENSITY}
hw.lcd.height=${HEIGHT}
hw.lcd.width=${WIDTH}
hw.mainKeys=no
hw.ramSize=2048
hw.sdCard=no
hw.sensors.light=yes
hw.sensors.orientation=yes
hw.sensors.proximity=yes
hw.trackBall=no
image.sysdir.1=system-images/android-36/google_apis_playstore/arm64-v8a/
runtime.network.latency=none
runtime.network.speed=full
showDeviceFrame=no
skin.dynamic=yes
skin.name=${WIDTH}x${HEIGHT}
tag.display=Google Play
tag.id=google_apis_playstore
target=android-36
vm.heapSize=256
EOF
}

# SystemUI demo mode: a fixed 9:30 clock, a full battery and Wi-Fi bar, and
# no notification icons — the Android counterpart of the simulator status
# bar override the iOS shots use. Without it every screenshot carries the
# emulator's real clock and whatever Play Protect happens to be showing,
# and the set looks inconsistent because each was taken at a different
# minute. `-e fully true` is what removes the "no internet" (!) badge from
# the Wi-Fi glyph; the emulator has no real uplink.
demo_on() {
  "$ADB" -s "$SERIAL" shell settings put global sysui_demo_allowed 1
  local d=("$ADB" -s "$SERIAL" shell am broadcast -a com.android.systemui.demo)
  "${d[@]}" -e command enter >/dev/null
  "${d[@]}" -e command clock -e hhmm 0930 >/dev/null
  "${d[@]}" -e command battery -e level 100 -e plugged false >/dev/null
  "${d[@]}" -e command network -e wifi show -e level 4 -e fully true >/dev/null
  "${d[@]}" -e command network -e mobile hide >/dev/null
  "${d[@]}" -e command notifications -e visible false >/dev/null
}

demo_off() {
  "$ADB" -s "$SERIAL" shell am broadcast -a com.android.systemui.demo \
    -e command exit >/dev/null || true
}


# ---------------------------------------------------------------- UI driver
#
# `screens` drives the app from adb rather than by hand. Taps resolve
# through uiautomator bounds instead of hard-coded coordinates: the one
# time this was driven by coordinates the Connect tap landed on the
# on-screen keyboard and typed a stray character into the address field.
# Only the scroll offsets are numeric, and those are safe because the AVD
# is pinned to a fixed 1080x1920.
#
ui_dump() {
  "$ADB" -s "$SERIAL" shell uiautomator dump /data/local/tmp/ui.xml >/dev/null 2>&1
  "$ADB" -s "$SERIAL" shell cat /data/local/tmp/ui.xml
}

# Prints "<x> <y>" for the best match. An exact text/desc/id match always
# beats a substring one — "Connect" must not resolve to the "Family
# Connect" title, and it did until this was ranked.
ui_find() {
  ui_dump | python3 -c "
import sys, re
q = sys.argv[1]
hits = []
for m in re.finditer(r'<node[^>]*>', sys.stdin.read()):
    n = m.group(0)
    def at(k):
        mm = re.search(k + r'=\"([^\"]*)\"', n)
        return mm.group(1) if mm else ''
    t, d, i = at('text'), at('content-desc'), at('resource-id').split('/')[-1]
    exact = q in (t, d, i)
    if not (exact or q.lower() in t.lower() or q.lower() in d.lower()):
        continue
    b = re.search(r'bounds=\"\[(\d+),(\d+)\]\[(\d+),(\d+)\]\"', n)
    if b:
        x1, y1, x2, y2 = map(int, b.groups())
        hits.append((0 if exact else 1, (x1 + x2) // 2, (y1 + y2) // 2))
if not hits:
    sys.exit(1)
_, x, y = sorted(hits)[0]
print(x, y)
" "$1"
}

# Wait for a node to appear, then tap it.
ui_tap() {
  local coords="" i=0
  while [ $i -lt 12 ]; do
    coords="$(ui_find "$1" 2>/dev/null || true)"
    [ -n "$coords" ] && break
    sleep 1
    i=$((i + 1))
  done
  if [ -z "$coords" ]; then
    echo "ui_tap: never found '$1'. What is on screen instead:" >&2
    ui_dump | grep -o 'text="[^"]\{1,40\}"' | sort -u | head -20 >&2
    exit 1
  fi
  # shellcheck disable=SC2086
  "$ADB" -s "$SERIAL" shell input tap $coords
  sleep "${2:-2}"
}

# Tap only if the node happens to be there. "Scroll to newest" exists only
# when the thread opened above the newest message, and the notification
# prompt only on a cleared install; waiting 20 rounds for either and then
# failing under `set -e` is what a required ui_tap would do.
ui_tap_if() {
  local coords
  coords="$(ui_find "$1" 2>/dev/null || true)"
  [ -n "$coords" ] || return 0
  # shellcheck disable=SC2086
  "$ADB" -s "$SERIAL" shell input tap $coords
  sleep "${2:-2}"
}

ui_type() {
  "$ADB" -s "$SERIAL" shell input text "$1"
  # BACK closes the IME. Without it the next tap lands on a key. The 3s
  # is the IME close animation plus whatever the field reveals on commit —
  # the address field grows a plain-http warning banner that pushes the
  # Connect button down, and a tap resolved before that settles lands
  # above the button and does nothing.
  "$ADB" -s "$SERIAL" shell input keyevent 4
  sleep 3
}

# The "scroll to newest" chevron is not a timed toast — it is a
# far-from-newest affordance that stays up until the list is close to the
# bottom again, and it lands right on the poll's vote avatars. Waiting it
# out does not work (measured: still up after 85s). How far a swipe travels
# depends on fling momentum, which varies with machine load, so the same
# swipe sequence sometimes ends past the threshold and sometimes not.
# Nudging down until the chevron is gone converges on a well-defined
# position instead of hoping the flings land the same way twice.
hide_chevron() {
  local i=0
  while [ $i -lt 6 ] && ui_find "Scroll to newest" >/dev/null 2>&1; do
    "$ADB" -s "$SERIAL" shell input swipe 540 1200 540 1020 600
    sleep 2
    i=$((i + 1))
  done
  sleep 2
}

ui_swipe() { "$ADB" -s "$SERIAL" shell input swipe "$1" "$2" "$3" "$4" 400; sleep "${5:-2}"; }

# Set the Map Previews switch to on/off, reading its current state rather
# than blind-toggling so a re-run is idempotent.
#
# WHY IT IS TURNED OFF FOR THE THREAD SHOTS. The shared-location card in
# the seeded thread renders as an empty grey grid here: the MAPS_API_KEY in
# local.properties is restricted to a signing certificate the debug build
# does not use, and the SDK says so —
#   E/Google Android Maps SDK: Authorization failure.
# Photographing that is photographing a bug. With previews off the same
# location draws as a pin card carrying its place name, which is a state
# the app genuinely offers and describes in its own settings copy. It is
# turned back on before the settings shot so the listing does not imply
# the app ships with maps disabled. Once the key authorises this build,
# drop the two set_map_previews calls and the grid becomes a real map.
set_map_previews() {
  local want="$1" state
  ui_tap "Settings" 3
  ui_swipe 540 1400 540 900 3
  state="$(ui_dump | python3 -c "
import sys, re
xml = sys.stdin.read()
# The switch is the checkable node nearest below the Map Previews label.
m = re.search(r'text=\"Map [Pp]reviews\"', xml)
if not m:
    print('unknown'); raise SystemExit
tail = xml[m.end():]
c = re.search(r'checkable=\"true\" checked=\"(true|false)\"', tail)
print(c.group(1) if c else 'unknown')
")"
  if [ "$state" = "unknown" ]; then
    echo "  (could not read the Map Previews switch — leaving it alone)"
  elif { [ "$want" = "off" ] && [ "$state" = "true" ]; } ||
       { [ "$want" = "on" ] && [ "$state" = "false" ]; }; then
    ui_tap "Map Previews" 2
  fi
  "$ADB" -s "$SERIAL" shell input keyevent 4
  sleep 3
}

case "${1:-}" in
start)
  ensure_avd
  if ! "$ADB" devices | awk 'NR>1 && $2=="device"' | grep -q '^emulator-'; then
    echo "booting ${AVD}…"
    "$EMULATOR" -avd "$AVD" -no-boot-anim -no-snapshot-save >/tmp/fc-emulator.log 2>&1 &
    # Poll for the emulator reaching `device` state rather than using
    # `adb -e wait-for-device`: with a physical phone also attached that
    # returns while the emulator is still `offline`, and the next command
    # then fails against a device that is not ready.
    for _ in $(seq 1 150); do
      SERIAL="$("$ADB" devices | awk '/^emulator-/ && $2=="device" {print $1; exit}')"
      [ -n "$SERIAL" ] && break
      sleep 2
    done
    [ -n "$SERIAL" ] || { echo "the emulator never reached 'device' state" >&2; exit 1; }
    until [ "$("$ADB" -s "$SERIAL" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ]; do
      sleep 2
    done
  fi
  require_emulator
  echo "sizing to ${WIDTH}x${HEIGHT} @${DENSITY} (Play needs <= 2:1)…"
  "$ADB" -s "$SERIAL" shell wm size "${WIDTH}x${HEIGHT}"
  "$ADB" -s "$SERIAL" shell wm density "$DENSITY"
  echo "installing…"
  # ANDROID_SERIAL pins Gradle to the emulator. Without it the Android
  # plugin installs to EVERY connected device, so a phone attached over
  # wireless debugging gets a debug build pushed at it — here that was
  # refused only because the phone carries a release-signed copy
  # (INSTALL_FAILED_UPDATE_INCOMPATIBLE), which is luck, not a safeguard.
  ( cd "$(dirname "$0")/.." && ANDROID_SERIAL="$SERIAL" ./gradlew --quiet :app:installStandardDebug )
  demo_on
  "$ADB" -s "$SERIAL" shell am start -n "$APP_ID/.MainActivity" >/dev/null
  cat <<'MSG'

  Ready. In the emulator:
    1. Server address:  http://10.0.2.2:8091
       (10.0.2.2 is the host as seen from inside the emulator —
        127.0.0.1 there is the emulator itself and will not connect.)
    2. Sign in:         nora / password123
    3. Navigate to a screen, then:
         android/scripts/capture-play-screenshots.sh shot 01-chats

  When finished:
         android/scripts/capture-play-screenshots.sh stop
MSG
  ;;

shot)
  NAME="${2:?usage: capture-play-screenshots.sh shot <name>}"
  require_emulator
  mkdir -p "$OUT"
  "$ADB" -s "$SERIAL" exec-out screencap -p > "$OUT/$NAME.png"
  # Play's spec is a 24-bit PNG with NO alpha, and `screencap -p` writes
  # RGBA — which is why the three screenshots committed in August carry an
  # alpha channel. Flatten onto white, then check the result against the
  # two rules Play enforces at upload, so a bad shot fails HERE rather than
  # in the console with 20 of them already taken.
  python3 - "$OUT/$NAME.png" <<'PY'
import sys
from PIL import Image

path = sys.argv[1]
im = Image.open(path)
if im.mode in ("RGBA", "LA", "P"):
    im = im.convert("RGBA")
    flat = Image.new("RGB", im.size, (255, 255, 255))
    flat.paste(im, mask=im.split()[-1])
    im = flat
else:
    im = im.convert("RGB")
im.save(path, "PNG", optimize=True)

w, h = im.size
long_side, short_side = max(w, h), min(w, h)
problems = []
if long_side > short_side * 2:
    problems.append(f"ratio {long_side / short_side:.3f}:1 exceeds Play's 2:1 ceiling")
if not (320 <= short_side and long_side <= 3840):
    problems.append(f"{w}x{h} is outside Play's 320..3840 px range")
if problems:
    print(f"  REJECTED {path}")
    for p in problems:
        print(f"    - {p}")
    sys.exit(1)
print(f"  {path}  ({w}x{h}, no alpha)")
PY
  ;;

screens)
  # Capture the whole six-shot set unattended, from a freshly cleared app.
  require_emulator
  SERVER="${FC_SERVER:-http://10.0.2.2:8091}"
  USER_NAME="${FC_USER:-nora}"
  USER_PASS="${FC_PASS:-password123}"
  mkdir -p "$OUT"

  echo "resetting the app…"
  # A downward swipe that lands on the launcher instead of the app opens
  # the notification shade, which then covers everything and makes every
  # subsequent ui_find fail with a misleading "never found <field>".
  # Collapse it before starting rather than debugging that twice.
  "$ADB" -s "$SERIAL" shell cmd statusbar collapse >/dev/null 2>&1 || true
  "$ADB" -s "$SERIAL" shell pm clear "$APP_ID" >/dev/null
  demo_on
  "$ADB" -s "$SERIAL" shell am start -n "$APP_ID/.MainActivity" >/dev/null
  sleep 8

  echo "connecting to ${SERVER}…"
  ui_tap "Server address"
  ui_type "$SERVER"
  ui_tap "Connect" 4
  # Verify rather than assume: a missed Connect otherwise surfaces 60s
  # later as "never found Username", which points at the wrong step.
  if ! ui_find "Username" >/dev/null 2>&1; then
    echo "  (first Connect did not take — retrying)"
    ui_tap "Connect" 6
  fi

  echo "signing in as ${USER_NAME}…"
  ui_tap "Username"
  ui_type "$USER_NAME"
  ui_tap "Password"
  ui_type "$USER_PASS"
  # Both the tab and the submit button read "Log in"; the button is the
  # lower of the two, so pick by position rather than by text.
  LOGIN_Y="$(ui_dump | python3 -c "
import sys, re
ys = []
for m in re.finditer(r'<node[^>]*>', sys.stdin.read()):
    n = m.group(0)
    if 'text=\"Log in\"' not in n:
        continue
    b = re.search(r'bounds=\"\[(\d+),(\d+)\]\[(\d+),(\d+)\]\"', n)
    if b:
        x1, y1, x2, y2 = map(int, b.groups())
        ys.append(((y1 + y2) // 2, (x1 + x2) // 2))
print('%d %d' % max(ys)[::-1] if ys else '')
")"
  # shellcheck disable=SC2086
  "$ADB" -s "$SERIAL" shell input tap $LOGIN_Y
  sleep 7
  # The runtime notification prompt only appears on a cleared install.
  ui_tap_if "Allow" 6

  set_map_previews off

  echo "capturing…"
  "$0" shot 01-chats

  ui_tap "The Harpers" 5
  ui_tap_if "Scroll to newest" 3
  ui_swipe 540 700 540 1500 3
  ui_swipe 540 700 540 1500 3
  hide_chevron
  "$0" shot 02-family-chat

  ui_swipe 540 1400 540 850 2
  ui_swipe 540 1300 540 950 3
  hide_chevron
  "$0" shot 03-photos-and-poll

  "$ADB" -s "$SERIAL" shell input keyevent 4; sleep 3
  ui_tap "Board" 4
  "$0" shot 04-board

  "$ADB" -s "$SERIAL" shell input keyevent 4; sleep 3
  ui_tap "Settings" 4
  ui_tap "Manage family" 4
  "$0" shot 05-family

  # Back out to the chats list: set_map_previews walks in from there.
  "$ADB" -s "$SERIAL" shell input keyevent 4; sleep 2
  "$ADB" -s "$SERIAL" shell input keyevent 4; sleep 3
  set_map_previews on
  ui_tap "Settings" 4
  # Scrolled past the profile header on purpose: on Android that row
  # renders "@nora - <server address>", which in this harness is the
  # emulator's 10.0.2.2 loopback alias and has no business in a listing.
  ui_swipe 540 1400 540 900 3
  "$0" shot 06-settings

  echo
  echo "done — six shots in $OUT"
  ;;

stop)
  require_emulator
  demo_off
  echo "restoring the emulator's own resolution…"
  "$ADB" -s "$SERIAL" shell wm size reset
  "$ADB" -s "$SERIAL" shell wm density reset
  ;;

*)
  echo "usage: capture-play-screenshots.sh {start|screens|shot <name>|stop}" >&2
  exit 2
  ;;
esac
