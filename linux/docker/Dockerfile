# Family Connect for Linux — the development and build image (adapted from md.linux's).
#
# Ubuntu 26.04's own toolchain and libraries, so what builds here is what builds on the target:
# rustc/cargo 1.93, GTK 4.22, libadwaita 1.9, WebKitGTK 6.0 2.52, GStreamer 1.28, libsoup 3. Xvfb and a D-Bus session bus
# let the app run headless; ImageMagick, xdotool and poppler-utils drive and inspect it.
#
# The source tree is bind-mounted at /src; the cargo registry and the target directory live in
# named volumes (see scripts/dev.sh), so a rebuild only recompiles what changed.
#
# The packages will be built from it too (wave 3), so both the base image and the package set are
# pinned: a rebuild of one commit gets the same
# compiler and the same GTK and WebKit headers, and gives the same package bytes.
#
# The base is the multi-arch index digest of ubuntu:26.04 (Ubuntu 26.04.1, image of 2026-10-24).
FROM ubuntu:26.04@sha256:19fcd1bf268f02b60aca3bd30bd8a6d9a11759a430e32921833e65f16d87348a

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8

# The Ubuntu archive as it was at this moment (snapshot.ubuntu.com). Moving it is a deliberate
# commit, like a dependency bump.
ARG UBUNTU_SNAPSHOT=20261027T000000Z

# Pinning the archive, measured on 2026-10-27:
# - The snapshot service is https only and the base image has no CA bundle, so ca-certificates is
#   installed first from the live archive. It is the one unpinned package; nothing built here
#   reads it.
# - `APT::Snapshot` makes every apt call use the snapshot. apt maps archive.ubuntu.com and
#   security.ubuntu.com (amd64) by itself, but not ports.ubuntu.com (arm64), and maps nothing at
#   all on the first update of an empty list cache — so ports is mapped by hand, to /ubuntu/: the
#   snapshot service serves every architecture there (its /ubuntu-ports/ answers 401).
# - An unreachable snapshot makes apt fall back to the live archive with only an "Ign:" line, so
#   the update is checked: every package index must come from the snapshot.
#
# --no-install-recommends keeps the image lean; everything the build and the headless run need is
# listed. The font list is the set an Ubuntu 26.04 desktop installs (ubuntu-desktop-minimal:
# fonts-dejavu-core Depends, the rest Recommends, fonts-urw-base35 via ghostscript) minus the
# CJK family (fonts-noto-cjk). GStreamer's ugly and libav sets hold the H.264 and AAC encoders the
# media profile needs (docs/protocol.md); nice holds webrtcbin's ICE; gtk4 holds gtk4paintablesink,
# the sink the app draws video with; tools and base-apps run pipelines and the discoverer in tests;
# gnome-keyring is the Secret Service the token self-test talks to. dpkg-dev (dpkg-shlibdeps) comes
# with build-essential.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && printf '%s\n' \
      "APT::Snapshot \"${UBUNTU_SNAPSHOT}\";" \
      'Acquire::Snapshots::URI::Host::ports.ubuntu.com "https://snapshot.ubuntu.com/ubuntu/@SNAPSHOTID@/";' \
      > /etc/apt/apt.conf.d/50fc-snapshot \
 && apt-get update \
 && apt-cache policy | grep -q "snapshot.ubuntu.com/ubuntu/${UBUNTU_SNAPSHOT} " \
 && if apt-cache policy | grep -E '^ +[0-9]+ https?://' | grep -v "snapshot.ubuntu.com/ubuntu/${UBUNTU_SNAPSHOT} "; then \
      echo "apt is not reading the ${UBUNTU_SNAPSHOT} snapshot alone" >&2; exit 1; \
    fi \
 && apt-get install -y --no-install-recommends \
      git build-essential pkg-config \
      rustc cargo rust-clippy rustfmt \
      libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev \
      libsoup-3.0-dev glib-networking libsecret-1-dev libsqlite3-dev \
      libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev \
      gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
      gstreamer1.0-plugins-ugly gstreamer1.0-libav gstreamer1.0-nice gstreamer1.0-pipewire \
      gstreamer1.0-gtk4 gstreamer1.0-tools gstreamer1.0-plugins-base-apps gnome-keyring \
      xvfb xauth dbus-daemon dbus-bin dbus-user-session at-spi2-core \
      libgl1-mesa-dri libegl-mesa0 \
      imagemagick x11-apps x11-utils xdotool poppler-utils \
      fontconfig fonts-dejavu-core fonts-dejavu-mono fonts-liberation fonts-urw-base35 \
      fonts-noto-core fonts-noto-mono fonts-noto-color-emoji fonts-ubuntu \
 && rm -rf /var/lib/apt/lists/*

ENV CARGO_HOME=/cargo \
    CARGO_TARGET_DIR=/target \
    XDG_RUNTIME_DIR=/run/user/0

RUN mkdir -p /cargo /target /run/user/0 /src && chmod 700 /run/user/0

WORKDIR /src
