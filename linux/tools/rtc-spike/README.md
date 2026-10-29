# The call spike (issue #112, phase 0)

A GStreamer `webrtcbin` peer in Rust — Opus and VP8, signalling as JSON over one WebSocket it
serves on port 9000 — and a Playwright script that calls it from Chromium with a fake camera and
microphone. It proved that a Rust client on Ubuntu 26.04 can call libwebrtc, in both directions,
before anything else was built on that (results in `docs/linux-client-2026-10-28.md`). Not part of
the workspace and not built by CI.

```sh
docker build -t fc-rtc-spike .
docker run --rm -e CARGO_TARGET_DIR=/src/target -v "$PWD":/src fc-rtc-spike \
  bash -c 'cd /src && cargo build --release'
docker network create fc-rtc-spike
# offer | answer
docker run -d --name rtcpeer --network fc-rtc-spike -v "$PWD":/src:ro fc-rtc-spike \
  /src/target/release/rtc-peer offer
# browser-answers | browser-offers-video | browser-offers-audio
docker run --rm --network fc-rtc-spike -v "$PWD/browser":/b mcr.microsoft.com/playwright:v1.55.0-noble \
  bash -c 'cd /b && npm i -s playwright@1.55.0 && node run.mjs browser-answers'
docker logs rtcpeer
```

The Rust side prints `GST SIDE OK` when it decoded more than 50 audio buffers and 50 video frames;
the browser prints `CHROMIUM RESULT` with what it received. In `browser-offers-audio` — the web
client's audio call — the browser sends no video, so the Rust side's video count is 0 and its
check fails by design.
