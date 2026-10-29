#!/usr/bin/env bash
# The call interop matrix (issue #112, package P6). The REAL fc-logic CallEngine over the app's
# calls::GstCallMedia (app/src/calls, compiled into the harness as it is) against another Linux
# peer, Chromium and Firefox (Playwright), over a relay that does what the server does with the
# protocol's call frames — with audiotestsrc/videotestsrc behind the self-test switches, media
# counted on both sides. Everything runs in Docker on private networks: the Linux peers and the
# relay in fc-linux-dev (Ubuntu 26.04, GStreamer 1.28), the browsers in Playwright's image, coturn
# from coturn/Dockerfile (the same Ubuntu snapshot). No camera, no microphone, no real server.
#
#   tools/call-interop/run.sh            every case
#   tools/call-interop/run.sh CASE…      those cases (run.sh list names them)
#
# FC_INTEROP_OUT=DIR keeps each peer's log (with its JSON report) there; a temporary directory
# otherwise. Containers are named fci-*, networks fc-call-interop*; all are removed on exit.
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=$(cd "$HERE/../../.." && pwd)
OUT=${FC_INTEROP_OUT:-$(mktemp -d)}
mkdir -p "$OUT"
IMAGE=${FC_IMAGE:-fc-linux-dev}
PLAYWRIGHT=mcr.microsoft.com/playwright:v1.55.0-noble
COTURN=fc-call-interop-coturn
NET=fc-call-interop
TARGET=fc-call-interop-target
NODE=fc-call-interop-node
BIN=/target/debug/call-interop
SECRET=interop-shared-secret

DEVICES=(
    -e "FC_TEST_AUDIO_SOURCE=audiotestsrc is-live=true wave=ticks"
    -e "FC_TEST_AUDIO_SINK=fakesink sync=true"
    -e "FC_TEST_VIDEO_SOURCE=videotestsrc is-live=true pattern=ball"
    -e "FC_TEST_VIDEO_SINK=fakesink sync=true"
    -e "G_MESSAGES_DEBUG=family-connect"
)

cleanup() {
    local containers
    containers=$(docker ps -aq --filter name='^fci-')
    [ -n "$containers" ] && docker unpause $containers >/dev/null 2>&1
    [ -n "$containers" ] && docker rm -f $containers >/dev/null 2>&1
    for network in "$NET" "$NET-a" "$NET-b"; do docker network rm "$network" >/dev/null 2>&1; done
}
trap cleanup EXIT

build() {
    echo "building the harness in ${IMAGE}…"
    docker run --rm -v "$REPO:/repo" -v fc-linux-cargo:/cargo -v "$TARGET:/target" \
        -w /repo/linux/tools/call-interop "$IMAGE" cargo build -q || exit 2
    docker run --rm -v "$NODE:/work" -w /work "$PLAYWRIGHT" bash -c \
        '[ -d node_modules/playwright ] || { npm init -y >/dev/null && npm i -s playwright@1.55.0; }' \
        >/dev/null || exit 2
    docker image inspect "$COTURN" >/dev/null 2>&1 \
        || docker build -q -t "$COTURN" "$HERE/coturn" >/dev/null || exit 2
}

relay_up() {
    docker network create "$NET" >/dev/null 2>&1
    docker rm -f fci-relay >/dev/null 2>&1
    docker run -d --name fci-relay --network "$NET" -v "$TARGET:/target" "$IMAGE" \
        "$BIN" relay 9000 >/dev/null
}

# linux NAME NETWORK FLAGS… — a Linux peer, in the background.
linux() {
    local name=$1 network=$2
    shift 2
    docker run -d --name "fci-$name" --network "$network" -v "$TARGET:/target" "${DEVICES[@]}" \
        "$IMAGE" "$BIN" peer --label "$name" --relay ws://fci-relay:9000 "$@" >/dev/null
}

# linux_gtk NAME NETWORK FLAGS… — the same under Xvfb, drawing into gtk4paintablesink.
linux_gtk() {
    local name=$1 network=$2
    shift 2
    local devices=("${DEVICES[@]}")
    unset 'devices[7]' 'devices[6]'
    docker run -d --name "fci-$name" --network "$network" -v "$TARGET:/target" "${devices[@]}" \
        "$IMAGE" bash -c 'Xvfb :99 -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 &
            for _ in $(seq 50); do [ -e /tmp/.X11-unix/X99 ] && break; sleep 0.1; done
            export DISPLAY=:99 GDK_BACKEND=x11 LIBGL_ALWAYS_SOFTWARE=1
            exec "$0" peer --gtk "$@"' "$BIN" --label "$name" --relay ws://fci-relay:9000 "$@" \
        >/dev/null
}

# browser NAME NETWORK JSON — a Chromium or Firefox peer, in the background.
browser() {
    docker run -d --name "fci-$1" --network "$2" --ipc=host -v "$NODE:/work" \
        -v "$HERE/browser:/b:ro" -w /work "$PLAYWRIGHT" \
        bash -c 'cp /b/peer.mjs . && exec node peer.mjs "$1"' _ "$3" >/dev/null
}

# web LABEL KIND USER PEER TIMEOUT FIELDS — a browser peer's options.
web() {
    printf '{"label":"%s","browser":"%s","relay":"ws://fci-relay:9000","user":%s,"peer":%s,"chat":7,"placeAfter":1,"answerAfter":2,"timeout":%s%s}' \
        "$1" "$2" "$3" "$4" "$5" "${6:+,$6}"
}

# collect CASE NAME… — wait for each peer, keep its log, show its checks.
collect() {
    local case=$1 status=0
    shift
    for name in "$@"; do
        docker wait "fci-$name" >/dev/null
        docker logs "fci-$name" >"$OUT/$case.$name.log" 2>&1
        docker rm "fci-$name" >/dev/null
        grep -E "\] (CHECK|RESULT)|stage |hanging up|placing|declining|giving up|stalled|connection (connected|failed)|(chromium|firefox) [0-9]" \
            "$OUT/$case.$name.log" | grep -v "REPORT"
        grep -q "RESULT PASS" "$OUT/$case.$name.log" || status=1
    done
    return $status
}

# when_talking NAME — once the peer has logged its Talking stage.
when_talking() {
    for _ in $(seq 300); do
        docker logs "fci-$1" 2>&1 | grep -q "stage Talking" && return 0
        sleep 0.1
    done
    return 1
}

# drop NAME CASE — keep a peer's log and remove it, whatever state it is in.
drop() {
    docker unpause "fci-$1" >/dev/null 2>&1
    docker logs "fci-$1" >"$OUT/$2.$1.log" 2>&1
    docker rm -f "fci-$1" >/dev/null 2>&1
}

L1=(--user 1 --peer 2)
L2=(--user 2 --peer 1)
MUTE_CAMERA=(--mute 4-8 --camera-off 9-13)
PEER_MUTE_CAMERA=(--peer-mute 4-8 --peer-camera-off 9-13)
BROWSER_MUTE_CAMERA='"peerMute":[4,8],"peerCameraOff":[9,13]'

# ---- (a) Linux ↔ Linux, and (e), (f) ------------------------------------------------------------

case_a_voice_out() { # Linux 1 places a voice call; the caller hangs up.
    linux L2 "$NET" "${L2[@]}" --answer-after 2 --expect-audio 300 --expect-no-video \
        --expect-ring Incoming --expect-end hangup --timeout 40
    linux L1 "$NET" "${L1[@]}" --place audio --talk 10 --expect-audio 300 --expect-no-video \
        --expect-ring Ringback --expect-end hangup --timeout 40
    collect a-voice-out L1 L2
}

case_a_voice_in() { # Linux 2 places a voice call; the CALLEE hangs up.
    linux L1 "$NET" "${L1[@]}" --answer-after 2 --talk 10 --expect-audio 300 --expect-no-video \
        --expect-end hangup --timeout 40
    linux L2 "$NET" "${L2[@]}" --place audio --expect-audio 300 --expect-no-video \
        --expect-end hangup --timeout 40
    collect a-voice-in L1 L2
}

case_a_video_out() { # Linux 1 places a video call, mutes 4–8 s and turns its camera off 9–13 s.
    linux L2 "$NET" "${L2[@]}" "${PEER_MUTE_CAMERA[@]}" --expect-audio 500 --expect-video 300 \
        --expect-codec VP8 --expect-end hangup --timeout 45
    linux L1 "$NET" "${L1[@]}" --place video --talk 17 "${MUTE_CAMERA[@]}" --expect-audio 500 \
        --expect-video 300 --expect-codec VP8 --expect-end hangup --timeout 45
    collect a-video-out L1 L2
}

case_a_video_in() { # Linux 2 places a video call; the callee mutes, turns its camera off, hangs up.
    linux L1 "$NET" "${L1[@]}" --talk 17 "${MUTE_CAMERA[@]}" --expect-audio 500 --expect-video 300 \
        --expect-codec VP8 --expect-end hangup --timeout 45
    linux L2 "$NET" "${L2[@]}" --place video "${PEER_MUTE_CAMERA[@]}" --expect-audio 500 \
        --expect-video 300 --expect-codec VP8 --expect-end hangup --timeout 45
    collect a-video-in L1 L2
}

case_a_video_gtk() { # The same through gtk4paintablesink under Xvfb: both pictures get frames.
    linux_gtk L2 "$NET" "${L2[@]}" --expect-audio 300 --expect-video 200 --expect-paintables \
        --expect-end hangup --timeout 40
    linux_gtk L1 "$NET" "${L1[@]}" --place video --talk 10 --expect-audio 300 --expect-video 200 \
        --expect-paintables --expect-end hangup --timeout 40
    collect a-video-gtk L1 L2
}

case_a_cameraless() { # A video call placed with no camera: receive-only video, the far side's shows.
    linux L2 "$NET" "${L2[@]}" --expect-audio 300 --expect-no-video --expect-end hangup --timeout 40
    linux L1 "$NET" "${L1[@]}" --place video --no-camera --talk 10 --expect-audio 300 \
        --expect-video 200 --expect-local-video recvonly --expect-end hangup --timeout 40
    collect a-cameraless L1 L2
}

case_f_cancel() { # The caller gives up while it rings: both end `cancel`, both tones stop.
    linux L2 "$NET" "${L2[@]}" --answer-after 30 --expect-no-connect --expect-ring Incoming \
        --expect-end cancel --timeout 30
    linux L1 "$NET" "${L1[@]}" --place audio --cancel-after 2.5 --expect-no-connect \
        --expect-ring Ringback --expect-end cancel --timeout 30
    collect f-cancel L1 L2
}

case_f_decline() { # The callee declines: both end `decline`.
    linux L2 "$NET" "${L2[@]}" --answer-after 2 --decline --expect-no-connect \
        --expect-ring Incoming --expect-end decline --timeout 30
    linux L1 "$NET" "${L1[@]}" --place video --expect-no-connect --expect-ring Ringback \
        --expect-end decline --timeout 30
    collect f-decline L1 L2
}

# ---- (b) Linux ↔ Chromium, (c) H.264, (d) Firefox ------------------------------------------------

case_b_voice_in() { # Chromium places the web client's voice call (audio only); Chromium hangs up.
    linux L1 "$NET" "${L1[@]}" --expect-audio 300 --expect-no-video --expect-end hangup --timeout 45
    browser W "$NET" "$(web W chromium 2 1 40 '"place":{"video":false},"talk":12,"expectAudio":300,"expectNoVideo":true,"expectEnd":"hangup"')"
    collect b-voice-in L1 W
}

case_b_voice_recvonly_in() { # Chromium's voice call with a recvonly video m-line (the old shape).
    linux L1 "$NET" "${L1[@]}" --expect-audio 300 --expect-no-video --expect-local-video inactive \
        --expect-end hangup --timeout 45
    browser W "$NET" "$(web W chromium 2 1 40 '"place":{"video":false,"recvonlyVideo":true},"talk":12,"expectAudio":300,"expectNoVideo":true,"expectEnd":"hangup"')"
    collect b-voice-recvonly-in L1 W
}

case_b_video_in() { # Chromium places a video call; Linux mutes and turns its camera off; Chromium hangs up.
    linux L1 "$NET" "${L1[@]}" "${MUTE_CAMERA[@]}" --expect-audio 500 --expect-video 200 \
        --expect-codec VP8 --expect-end hangup --timeout 45
    browser W "$NET" "$(web W chromium 2 1 40 "\"place\":{\"video\":true},\"talk\":17,$BROWSER_MUTE_CAMERA,\"expectAudio\":500,\"expectVideo\":200,\"expectCodec\":\"VP8\",\"expectEnd\":\"hangup\"")"
    collect b-video-in L1 W
}

case_b_voice_out() { # Linux places a voice call to Chromium; Linux hangs up.
    browser W "$NET" "$(web W chromium 2 1 40 '"expectAudio":300,"expectNoVideo":true,"expectEnd":"hangup"')"
    sleep 3
    linux L1 "$NET" "${L1[@]}" --place audio --talk 12 --expect-audio 300 --expect-no-video \
        --expect-end hangup --timeout 40
    collect b-voice-out L1 W
}

case_b_video_out() { # Linux places a video call; Chromium mutes 4–8 s (track.enabled), hangs up.
    browser W "$NET" "$(web W chromium 2 1 40 '"mute":[4,8],"cameraOff":[9,13],"talk":17,"expectAudio":500,"expectVideo":200,"expectCodec":"VP8","expectEnd":"hangup"')"
    sleep 3
    linux L1 "$NET" "${L1[@]}" --place video --peer-mute 4-8 --expect-audio 500 --expect-video 200 \
        --expect-codec VP8 --expect-end hangup --timeout 40
    collect b-video-out L1 W
}

case_b_mdns() { # Chromium with its default mDNS host candidates (what real browsers send).
    linux L1 "$NET" "${L1[@]}" --expect-audio 300 --expect-video 200 --expect-end hangup --timeout 45
    browser W "$NET" "$(web W chromium 2 1 40 '"mdns":true,"place":{"video":true},"talk":12,"expectAudio":300,"expectVideo":200,"expectEnd":"hangup"')"
    collect b-mdns L1 W
}

case_b_cameraless_in() { # Chromium places a video call; Linux has no camera and answers receive-only.
    linux L1 "$NET" "${L1[@]}" --no-camera --expect-audio 300 --expect-video 200 \
        --expect-local-video recvonly --expect-end hangup --timeout 45
    browser W "$NET" "$(web W chromium 2 1 40 '"place":{"video":true},"talk":12,"expectAudio":300,"expectNoVideo":true,"expectEnd":"hangup"')"
    collect b-cameraless-in L1 W
}

case_c_h264() { # An H.264-only offer (a Linux peer's: neither Playwright browser has H.264).
    linux L2 "$NET" "${L2[@]}" --expect-audio 300 --expect-video 200 --expect-codec H264 \
        --expect-end hangup --timeout 40
    linux L1 "$NET" "${L1[@]}" --place video --offer-video h264 --talk 10 --expect-audio 300 \
        --expect-video 200 --expect-codec H264 --expect-end hangup --timeout 40
    collect c-h264 L1 L2
}

fixture() { # fixture CASE FILE FLAGS…
    local case=$1 file=$2
    shift 2
    docker run --rm -v "$TARGET:/target" -v "$HERE/fixtures:/f:ro" "${DEVICES[@]}" "$IMAGE" \
        "$BIN" answer-fixture "/f/$file" "$@" >"$OUT/$case.log" 2>&1
    grep "^fixture" "$OUT/$case.log"
    grep -q "RESULT PASS" "$OUT/$case.log"
}

case_c_h264_fixture() { # Chrome's H.264-only offer shape (fixtures/chromium-h264-only-offer.sdp).
    fixture c-h264-fixture chromium-h264-only-offer.sdp --video --expect-codec H264 \
        --expect-video-pt 106 --expect-audio-pt 111 --expect-video sendrecv
}

case_d_firefox_video_in() { # Firefox places a video call (Opus 109, VP8 120).
    linux L1 "$NET" "${L1[@]}" --expect-audio 300 --expect-video 200 --expect-codec VP8 \
        --expect-end hangup --timeout 45
    browser F "$NET" "$(web F firefox 2 1 40 '"place":{"video":true},"talk":12,"expectAudio":300,"expectVideo":200,"expectCodec":"VP8/120","expectEnd":"hangup"')"
    collect d-firefox-video-in L1 F
}

case_d_firefox_video_out() { # Linux places a video call to Firefox.
    browser F "$NET" "$(web F firefox 2 1 40 '"expectAudio":300,"expectVideo":200,"expectCodec":"VP8/96","expectEnd":"hangup"')"
    sleep 4
    linux L1 "$NET" "${L1[@]}" --place video --talk 12 --expect-audio 300 --expect-video 200 \
        --expect-codec VP8 --expect-end hangup --timeout 40
    collect d-firefox-video-out L1 F
}

case_d_firefox_voice_in() { # Firefox places a voice call.
    linux L1 "$NET" "${L1[@]}" --expect-audio 300 --expect-no-video --expect-end hangup --timeout 45
    browser F "$NET" "$(web F firefox 2 1 40 '"place":{"video":false},"talk":12,"expectAudio":300,"expectNoVideo":true,"expectEnd":"hangup"')"
    collect d-firefox-voice-in L1 F
}

case_d_firefox_fixture() { # Firefox 141's own offer (fixtures/firefox-video-offer.sdp).
    fixture d-firefox-fixture firefox-video-offer.sdp --video --expect-codec VP8 \
        --expect-video-pt 120 --expect-audio-pt 109 --expect-video sendrecv
}

# ---- (g) a dead peer --------------------------------------------------------------------------

dead_peer() { # dead_peer CASE HOW — HOW: pause (frozen: lid shut) | kill (gone: app killed)
    local case=$1 how=$2
    linux L2 "$NET" "${L2[@]}" --timeout 120
    linux L1 "$NET" "${L1[@]}" --place audio --expect-audio 100 --expect-end failed --timeout 120
    when_talking L1 && sleep 5
    echo "  $how the callee at $(date -u +%T)"
    docker "$how" fci-L2 >/dev/null
    collect "$case" L1
    local status=$?
    drop L2 "$case"
    return $status
}

case_g_pause() { dead_peer g-pause pause; }
case_g_kill() { dead_peer g-kill kill; }

case_g_browser_gone() { # The browser's container killed mid-call.
    linux L1 "$NET" "${L1[@]}" --expect-audio 100 --expect-end failed --timeout 120
    browser W "$NET" "$(web W chromium 2 1 110 '"place":{"video":false}')"
    when_talking L1 && sleep 5
    echo "  kill the browser at $(date -u +%T)"
    docker kill fci-W >/dev/null
    collect g-browser-gone L1
    local status=$?
    drop W g-browser-gone
    return $status
}

case_g_fallback() { # ICE alive, the far side's audio stalled: the fallback rule (forced) ends it.
    linux L2 "$NET" "${L2[@]}" --stall-audio-at 5 --timeout 90
    linux L1 "$NET" "${L1[@]}" --place audio --silence-rule always --expect-audio 100 \
        --expect-end failed --timeout 90
    collect g-fallback L1
    local status=$?
    drop L2 g-fallback
    return $status
}

case_g_fallback_off() { # The same stall with the default rule: consent is fresh, the call stays up.
    linux L2 "$NET" "${L2[@]}" --stall-audio-at 5 --timeout 90
    linux L1 "$NET" "${L1[@]}" --place audio --talk 45 --expect-audio 100 --expect-end hangup \
        --timeout 90
    collect g-fallback-off L1
    local status=$?
    drop L2 g-fallback-off
    return $status
}

# ---- TURN: coturn's REST credentials, peers on two networks that cannot reach each other ----------

turn_up() {
    docker network create "$NET-a" >/dev/null 2>&1
    docker network create "$NET-b" >/dev/null 2>&1
    docker network connect "$NET-a" fci-relay >/dev/null 2>&1
    docker network connect "$NET-b" fci-relay >/dev/null 2>&1
    docker rm -f fci-turn >/dev/null 2>&1
    docker run -d --name fci-turn --network "$NET-a" "$COTURN" turnserver -n --no-cli --verbose \
        --log-file=stdout --listening-port=3478 --realm=fc-interop --fingerprint \
        --use-auth-secret --static-auth-secret="$SECRET" --no-tls --no-dtls \
        --allowed-peer-ip=172.16.0.0-172.31.255.255 --allowed-peer-ip=192.168.0.0-192.168.255.255 \
        --allowed-peer-ip=10.0.0.0-10.255.255.255 >/dev/null
    docker network connect "$NET-b" fci-turn >/dev/null
    TURN_A=$(docker inspect -f "{{(index .NetworkSettings.Networks \"$NET-a\").IPAddress}}" fci-turn)
    TURN_B=$(docker inspect -f "{{(index .NetworkSettings.Networks \"$NET-b\").IPAddress}}" fci-turn)
    # coturn's REST scheme: the username is <expiry>:<user id>, the credential base64 of the
    # HMAC-SHA1 of it under the shared secret — a colon in the user, `=` (and maybe + or /) in the
    # credential: what fc-logic percent-encodes.
    TURN_USER="$(($(date +%s) + 3600)):42"
    TURN_PASS=$(printf %s "$TURN_USER" | openssl dgst -sha1 -hmac "$SECRET" -binary | base64)
    sleep 1
}

turn_down() {
    docker logs fci-turn >"$OUT/$1.coturn.log" 2>&1
    docker rm -f fci-turn >/dev/null 2>&1
}

case_turn() { # Linux ↔ Linux through coturn only.
    turn_up
    linux L2 "$NET-b" "${L2[@]}" --turn "turn:$TURN_B:3478?transport=udp" --turn-user "$TURN_USER" \
        --turn-pass "$TURN_PASS" --expect-audio 300 --expect-video 200 --expect-end hangup --timeout 45
    linux L1 "$NET-a" "${L1[@]}" --turn "turn:$TURN_A:3478?transport=udp" --turn-user "$TURN_USER" \
        --turn-pass "$TURN_PASS" --place video --talk 10 --expect-audio 300 --expect-video 200 \
        --expect-end hangup --timeout 45
    collect turn L1 L2
    local status=$?
    turn_down turn
    echo "  credential: user '$TURN_USER', password ${#TURN_PASS} characters, ends '${TURN_PASS: -1}'"
    grep -E "ALLOCATE processed, success|CREATE_PERMISSION processed, success|CHANNEL_BIND processed, success" \
        "$OUT/turn.coturn.log" | grep -v -E "^$" | head -8
    return $status
}

case_turn_control() { # The same networks with no TURN: nothing connects (the isolation is real).
    turn_up
    linux L2 "$NET-b" "${L2[@]}" --timeout 60
    linux L1 "$NET-a" "${L1[@]}" --place audio --expect-no-connect --expect-end failed --timeout 60
    collect turn-control L1
    local status=$?
    drop L2 turn-control
    turn_down turn-control
    return $status
}

case_turn_browser() { # Chromium ↔ Linux through coturn only.
    turn_up
    linux L1 "$NET-a" "${L1[@]}" --turn "turn:$TURN_A:3478?transport=udp" --turn-user "$TURN_USER" \
        --turn-pass "$TURN_PASS" --expect-audio 300 --expect-video 200 --expect-end hangup --timeout 45
    browser W "$NET-b" "$(web W chromium 2 1 40 "\"iceServers\":[{\"urls\":\"turn:$TURN_B:3478?transport=udp\",\"username\":\"$TURN_USER\",\"credential\":\"$TURN_PASS\"}],\"place\":{\"video\":true},\"talk\":12,\"expectAudio\":300,\"expectVideo\":200,\"expectEnd\":\"hangup\"")"
    collect turn-browser L1 W
    local status=$?
    turn_down turn-browser
    return $status
}

CASES=(a_voice_out a_voice_in a_video_out a_video_in a_video_gtk a_cameraless f_cancel f_decline
    b_voice_in b_voice_recvonly_in b_video_in b_voice_out b_video_out b_mdns b_cameraless_in
    c_h264 c_h264_fixture d_firefox_video_in d_firefox_video_out d_firefox_voice_in
    d_firefox_fixture g_pause g_kill g_browser_gone g_fallback g_fallback_off
    turn turn_control turn_browser)

if [ "${1:-}" = list ]; then
    printf '%s\n' "${CASES[@]}"
    exit 0
fi
selected=("$@")
[ ${#selected[@]} -eq 0 ] && selected=("${CASES[@]}")

build
relay_up
passed=() failed=()
for case in "${selected[@]}"; do
    echo "=== $case"
    if "case_$case"; then passed+=("$case"); else failed+=("$case"); fi
done
echo
echo "passed (${#passed[@]}): ${passed[*]:-}"
echo "failed (${#failed[@]}): ${failed[*]:-}"
echo "logs: $OUT"
[ ${#failed[@]} -eq 0 ]
