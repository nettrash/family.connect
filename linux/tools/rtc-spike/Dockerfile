# The phase-0 call spike's peer (docs/linux-client-2026-10-28.md): Ubuntu 26.04's own rustc and
# GStreamer, nothing newer.
FROM ubuntu:26.04
RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
      rustc cargo pkg-config ca-certificates \
      libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev \
      gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-nice \
      >/dev/null && rm -rf /var/lib/apt/lists/*
