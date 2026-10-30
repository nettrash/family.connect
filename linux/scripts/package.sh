#!/usr/bin/env bash
# Family Connect for Linux — the .deb for Ubuntu 26.04 and the portable .tar.gz
# (docs/packaging.md), adapted from md.linux's.
#
#   scripts/package.sh [options] [amd64|arm64 …]      default: the host's architecture
#
#   --clean         build in a new, empty target volume (removed afterwards): what proves that two
#                   builds of one commit give one SHA-256, since an incremental build would not
#   --no-check      skip the install / uninstall checks in a fresh ubuntu:26.04 container
#   --allow-dirty   package the working tree although linux/, web/text/ or CHANGELOG has
#                   uncommitted changes (see package-lib.sh's export_source)
#   --native        no Docker: build on this machine, an Ubuntu 26.04 with the build packages
#                   (below), for its own architecture only
#
# Self-contained: it needs only Docker with buildx (amd64 runs under emulation on an arm64 Mac).
# `scripts/dev.sh package [arch…]` delegates here. Per architecture it
#
#   1. builds the image fc-linux-dev:<arch> from docker/Dockerfile (pinned base and archive);
#   2. exports the source from git (linux/, web/text/, CHANGELOG — package-lib.sh) — HEAD itself
#      when those are clean, else (--allow-dirty) their tracked and untracked-but-not-ignored files;
#   3. in fc-linux-dev:<arch>: cargo build --release --locked -p family-connect, stages
#      docs/packaging.md §1 in a volume and computes Depends (§8.1) and the control data; then, in
#      fc-linux-dev of the machine's own architecture, builds
#      dist/family-connect_<version>_<arch>.deb with dpkg-deb and checks its info and contents
#      against §1 (see deb_inside for why dpkg-deb runs natively), and from the same files the
#      portable dist/family-connect-<version>-linux-<x86_64|aarch64>.tar.gz (§12);
#   4. in a fresh ubuntu:26.04 of the same architecture (§9.3; the image fc-linux-ubuntu:<arch>,
#      the pinned base and nothing else): apt installs the .deb from the live archive, runs
#      family-connect --version, checks the GStreamer elements the plugin Depends are for,
#      desktop-file-validate, appstreamcli validate --no-net, an AppArmor parse of the profile,
#      then apt remove and purge, the portable archive unpacked and run, and a reinstall; and
#      lintian, in a throwaway ubuntu:26.04 of the machine's own architecture.
#
# The package's SOURCE_DATE_EPOCH is the commit time of HEAD — with --allow-dirty too, so a build
# of an uncommitted tree carries the time of the commit it sits on.
#
# --native runs step 3 on this machine instead: the build packages are docker/Dockerfile's
#
#   sudo apt install build-essential pkg-config git rustc cargo \
#        libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev libgstreamer1.0-dev \
#        libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev \
#        libsecret-1-dev libsqlite3-dev
#
# and the target directory is linux/target/package (--clean: a new one, removed afterwards). Of
# step 4 it runs lintian only, if installed (sudo apt install lintian): it does not install the
# package on the machine that builds it. The bytes match a Docker build's when this machine's
# compiler and libraries are the snapshot's; a later archive can change them.
#
# Environment: FC_PACKAGE_SKIP_IMAGE=1 reuses the existing images fc-linux-dev:<arch> and
# fc-linux-ubuntu:<arch>.
# `A && ok … || fail …` is meant as if-else: `ok` and `fail` both return 0.
# shellcheck disable=SC2015
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"   # linux/
REPO="$(cd "$ROOT/.." && pwd)"                             # the repository
PKG=family-connect
APP_ID=me.nettrash.FamilyConnect
APP_ID_RE='me\.nettrash\.FamilyConnect'

say() { printf '%s\n' "package: $*" >&2; }
die() { say "FAILED: $*"; exit 1; }
# shellcheck source=scripts/package-lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/package-lib.sh"

# Where the steps find the source (the folder holding linux/ and web/), the staging area, the
# output, the target directory and the cargo home: the containers' mounts, unless --native points
# them at this machine.
SRC=/src STAGE=/stage OUT=/out
TARGET="${CARGO_TARGET_DIR:-/target}" CARGO_DIR="${CARGO_HOME:-/cargo}"

# The packages dpkg-shlibdeps may name for /usr/bin/family-connect: the libraries behind the -sys
# crates of app/Cargo.toml (GStreamer's base, bad and — for libgstplay, the player — Ubuntu
# 26.04's "extra" libraries, libsecret, SQLite), plus libc and libgcc. Anything else is a new
# linkage to review, not to ship.
ALLOWED_DEPENDS="libc6 libgcc-s1 libglib2.0-0t64 libgtk-4-1 libadwaita-1-0 libwebkitgtk-6.0-4
libjavascriptcoregtk-6.0-1 libsoup-3.0-0 libpango-1.0-0 libcairo2 libcairo-gobject2
libgdk-pixbuf-2.0-0 libgraphene-1.0-0 libgstreamer1.0-0 libgstreamer-plugins-base1.0-0
libgstreamer-plugins-bad1.0-0 libgstreamer-plugins-extra1.0-0 libsecret-1-0 libsqlite3-0"

# Floors raised to what the crates are built against (gtk v4_22, adw v1_9, webkit v2_52, glib
# v2_88, gst v1_28, soup v3_6 — the socket's keepalive-pong-timeout) and the client is tested on:
# dpkg-shlibdeps' floors only reflect the symbols the binary happens to reference.
RAISED_FLOORS="libgtk-4-1:4.22 libadwaita-1-0:1.9 libwebkitgtk-6.0-4:2.52 libjavascriptcoregtk-6.0-1:2.52
libglib2.0-0t64:2.88 libgstreamer1.0-0:1.28 libgstreamer-plugins-base1.0-0:1.28
libgstreamer-plugins-bad1.0-0:1.28 libgstreamer-plugins-extra1.0-0:1.28 libsoup-3.0-0:3.6"

# Libraries the client will link and does not yet: fc-core's cache (SQLite, the system's — no
# `bundled`) and the token store (the Secret Service, libsecret). Declared now so the package
# needs what the client will; once dpkg-shlibdeps names one, its entry is used instead.
PLANNED_LIBRARIES="libsqlite3-0 libsecret-1-0"

# What the client loads at run time rather than links (docs/linux-client-2026-10-28.md): the
# GStreamer plugins for calls (webrtcbin, ICE, DTLS/SRTP, Opus, VP8) and the media profile (H.264
# and AAC-LC in MP4: x264enc in ugly, avenc_aac in libav — both in universe), PipeWire capture,
# the sink that draws video in the window (gtk4paintablesink, docs/shell-design.md: gstreamer1.0-gtk4
# is gst-plugins-rs, with its own version — 0.14.4 on 26.04 — so no 1.28 floor), and GIO's TLS for
# the server's https and wss.
RUNTIME_DEPENDS="gstreamer1.0-plugins-base (>= 1.28), gstreamer1.0-plugins-good (>= 1.28),
gstreamer1.0-plugins-bad (>= 1.28), gstreamer1.0-plugins-ugly (>= 1.28), gstreamer1.0-libav (>= 1.28),
gstreamer1.0-nice, gstreamer1.0-pipewire, gstreamer1.0-gtk4, glib-networking"

# packaging/control.in's Recommends, which the layout check expects: the Secret Service that keeps
# the session token (GNOME Keyring) and GNOME's portals (the camera, notifications, the global
# shortcut). Ubuntu Desktop has both; a Recommends, so a minimal system still installs the package.
RECOMMENDS="gnome-keyring, xdg-desktop-portal-gnome"

# The GStreamer elements check_inside expects once the .deb is installed: what RUNTIME_DEPENDS is
# for. Every one is read by the plan; a missing one is a Depends that no longer delivers it.
GST_ELEMENTS="webrtcbin nicesrc nicesink dtlssrtpenc dtlssrtpdec rtpopuspay rtpvp8pay opusenc opusdec
vp8enc vp8dec x264enc avdec_h264 h264parse avenc_aac mp4mux qtdemux pipewiresrc gtk4paintablesink"

# Every crate linked into the binary must be named in packaging/copyright and offer MIT.
ALLOWED_CRATE_LICENSES="MIT|MIT OR Apache-2.0|Apache-2.0 OR MIT|MIT/Apache-2.0|Unlicense OR MIT"
# The crates allowed under Zlib instead, by name — each in packaging/copyright's Zlib paragraph:
# foldhash, the hash rusqlite's statement cache uses (through hashlink and hashbrown). Zlib asks
# nothing of a binary; any OTHER crate without an MIT choice still stops the build.
ZLIB_CRATES="foldhash"

# The file layout of docs/packaging.md §1, without the icons (13, from packaging/icons) and the
# data folder (linux/data, installed as /usr/share/family-connect).
LAYOUT_FILES="etc/apparmor.d/$PKG usr/bin/$PKG
usr/share/applications/$APP_ID.desktop usr/share/metainfo/$APP_ID.metainfo.xml
usr/share/dbus-1/services/$APP_ID.service
usr/share/doc/$PKG/CHANGELOG.gz usr/share/doc/$PKG/changelog.gz usr/share/doc/$PKG/copyright
usr/share/man/man1/$PKG.1.gz"

# ---------------------------------------------------------------------------------------------
# Inside fc-linux-dev:<arch>: build, stage, compute the control data.

build_inside() {
    local arch="$1"
    local version stage
    cd "$SRC/linux"
    [ "$(dpkg --print-architecture)" = "$arch" ] || die "this system is $(dpkg --print-architecture), not $arch"
    [ -n "${SOURCE_DATE_EPOCH:-}" ] || die "SOURCE_DATE_EPOCH is not set"

    version="$(tree_version .)"
    [ -n "$version" ] || die "no [workspace.package] version in linux/Cargo.toml"
    grep -q "^$PKG ($version) " packaging/changelog || die "packaging/changelog does not start at $version"
    grep -q "<release version=\"$version\"" "packaging/$APP_ID.metainfo.xml" ||
        die "the metainfo has no release $version"
    grep -q " - $version-1\$" "packaging/rpm/$PKG.spec" ||
        die "packaging/rpm/$PKG.spec's %changelog does not start at $version"
    grep -q "\"$PKG $version\"" packaging/$PKG.1 || die "packaging/$PKG.1 does not name $version"

    check_crates

    # Reproducible: fixed paths in the binary (the containers' ones, wherever it is built), no debug
    # info, symbols stripped, one codegen unit (the workspace's [profile.release]).
    export_rustflags "$SRC" "$CARGO_DIR" "$TARGET"
    export CARGO_TARGET_DIR="$TARGET" CARGO_PROFILE_RELEASE_STRIP=true CARGO_PROFILE_RELEASE_DEBUG=false
    say "cargo build --release --locked -p $PKG ($arch, $(rustc --version))"
    cargo build --release --locked -p "$PKG"
    local bin="$TARGET/release/$PKG"
    if readelf -S "$bin" | grep -qE '\.(symtab|debug_[a-z]+)'; then die "the binary is not stripped"; fi

    stage="$STAGE/tree"
    rm -rf "$stage"
    mkdir -p "$stage"
    install -D -m 0755 "$bin" "$stage/usr/bin/$PKG"
    # What dh_strip also removes: the compilers' .comment and the .note section.
    strip --remove-section=.comment --remove-section=.note "$stage/usr/bin/$PKG"
    install -D -m 0644 "packaging/$APP_ID.desktop" "$stage/usr/share/applications/$APP_ID.desktop"
    install -D -m 0644 "packaging/$APP_ID.metainfo.xml" "$stage/usr/share/metainfo/$APP_ID.metainfo.xml"
    # DBusActivatable (docs/packaging.md §2): a notification clicked after a quit starts the app.
    install -D -m 0644 "packaging/dbus/$APP_ID.service" "$stage/usr/share/dbus-1/services/$APP_ID.service"
    mkdir -p "$stage/usr/share/icons"
    cp -R packaging/icons/hicolor "$stage/usr/share/icons/"
    install -D -m 0644 "packaging/apparmor/$PKG" "$stage/etc/apparmor.d/$PKG"
    mkdir -p "$stage/usr/share/man/man1" "$stage/usr/share/doc/$PKG"
    gzip -9n -c "packaging/$PKG.1" > "$stage/usr/share/man/man1/$PKG.1.gz"
    gzip -9n -c packaging/changelog > "$stage/usr/share/doc/$PKG/changelog.gz"
    gzip -9n -c ../CHANGELOG > "$stage/usr/share/doc/$PKG/CHANGELOG.gz"
    install -m 0644 packaging/copyright "$stage/usr/share/doc/$PKG/copyright"
    # The data folder (docs/shell-design.md §2.3: the catalogue's tables, the rich page): the binary
    # reads /usr/share/family-connect, and the tarball's bin/ finds ../share/family-connect.
    mkdir -p "$stage/usr/share/$PKG"
    cp -R data/. "$stage/usr/share/$PKG/"

    # Modes and owners: directories 0755, files 0644, the binary and the scripts 0755; dpkg-deb
    # --root-owner-group makes every entry root:root.
    find "$stage" -type d -exec chmod 0755 {} +
    find "$stage" -type f -exec chmod 0644 {} +
    chmod 0755 "$stage/usr/bin/$PKG"

    # DEBIAN/ first: dpkg-shlibdeps finds the package root of the binary by it.
    mkdir -p "$stage/DEBIAN"
    local depends installed_size
    depends="$(compute_depends "$stage/usr/bin/$PKG")"
    installed_size="$(installed_size "$stage")"

    sed -e "s/@VERSION@/$version/" -e "s/@ARCH@/$arch/" -e "s/@INSTALLED_SIZE@/$installed_size/" \
        -e "s/@DEPENDS@/$depends/" packaging/control.in > "$stage/DEBIAN/control"
    if grep -q '@[A-Z_]*@' "$stage/DEBIAN/control"; then die "control.in has an unfilled field"; fi
    printf '%s\n' "/etc/apparmor.d/$PKG" > "$stage/DEBIAN/conffiles"
    install -m 0755 packaging/postinst "$stage/DEBIAN/postinst"
    install -m 0755 packaging/postrm "$stage/DEBIAN/postrm"
    (cd "$stage" && find . -path ./DEBIAN -prune -o -type f -printf '%P\n' | LC_ALL=C sort |
        xargs -d '\n' md5sum) > "$stage/DEBIAN/md5sums"
    chmod 0644 "$stage/DEBIAN/control" "$stage/DEBIAN/conffiles" "$stage/DEBIAN/md5sums"

    find "$stage" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
    printf '%s\n' "$depends" > "$STAGE/depends"
    say "staged for $arch"
}

# In fc-linux-dev of the machine's own architecture: the .deb from the staged tree, and its
# layout check. dpkg-deb runs natively because under Docker Desktop's Rosetta emulation GNU tar
# cannot stat a path given by name (`Cannot stat: Function not implemented`), and dpkg-deb always
# names its files to tar; what dpkg-deb writes depends on its version and its input, not on the
# processor it runs on, and both images carry the same dpkg from the same snapshot.
deb_inside() {
    local arch="$1" version stage="$STAGE/tree" deb depends
    cd "$SRC/linux"
    version="$(tree_version .)"
    [ -f "$stage/DEBIAN/control" ] || die "nothing staged for $arch"
    grep -qx "Architecture: $arch" "$stage/DEBIAN/control" || die "the staged tree is not $arch"
    depends="$(cat "$STAGE/depends")"
    deb="$OUT/${PKG}_${version}_${arch}.deb"
    rm -f "$deb"
    # One xz thread: liblzma's multi-threaded output is laid out in blocks, so the bytes would
    # depend on how many CPUs the building machine has.
    dpkg-deb --root-owner-group --threads-max=1 -Zxz -z9 --build "$stage" "$deb" >/dev/null
    check_layout "$deb" "$version" "$arch" "$depends"
    say "built $(basename "$deb"): $(stat -c %s "$deb") bytes"
    (cd "$OUT" && sha256sum "$(basename "$deb")")
    make_tarball "$stage" "$version" "$arch"
    rm -rf "$stage" "$STAGE/depends"
}

# The portable archive family-connect-<version>-linux-<x86_64|aarch64>.tar.gz (docs/packaging.md
# §12): the staged /usr as family-connect-<version>/, whose bin/family-connect finds share/icons
# beside it, plus packaging/tarball/README and the AppArmor profile as a template. Reproducible
# like the .deb: sorted names, root-owned, SOURCE_DATE_EPOCH, gzip without a name.
make_tarball() {
    local stage="$1" version="$2" arch="$3" machine name tmp tarball status=0 listing want
    case "$arch" in amd64) machine=x86_64 ;; arm64) machine=aarch64 ;; *) die "no tarball name for $arch" ;; esac
    name="$PKG-$version"
    tarball="$OUT/$PKG-$version-linux-$machine.tar.gz"
    tmp="$(mktemp -d /tmp/fc-tarball.XXXXXX)"
    cp -a "$stage/usr" "$tmp/$name"
    sed "s/@VERSION@/$version/g" packaging/tarball/README > "$tmp/$name/README"
    install -m 0644 "packaging/apparmor/$PKG" "$tmp/$name/share/doc/$PKG/apparmor-profile"
    chmod 0644 "$tmp/$name/README"
    find "$tmp" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
    rm -f "$tarball"
    tar --sort=name --format=gnu --owner=root:0 --group=root:0 -C "$tmp" -cf - "$name" |
        gzip -9n > "$tarball"
    rm -rf "$tmp"

    listing="$(tar -tzvf "$tarball")"
    for want in "bin/$PKG" README "share/applications/$APP_ID.desktop" \
        "share/metainfo/$APP_ID.metainfo.xml" "share/man/man1/$PKG.1.gz" \
        "share/doc/$PKG/copyright" "share/doc/$PKG/changelog.gz" "share/doc/$PKG/CHANGELOG.gz" \
        "share/doc/$PKG/apparmor-profile" "share/$PKG/i18n/plurals.json" \
        "share/dbus-1/services/$APP_ID.service" "share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"; do
        printf '%s\n' "$listing" | grep -qF " $name/$want" || { say "tarball: no $name/$want"; status=1; }
    done
    printf '%s\n' "$listing" | grep -qE "^-rwxr-xr-x root/root .* $name/bin/$PKG\$" ||
        { say "tarball: bin/$PKG is not an executable owned by root"; status=1; }
    [ "$(printf '%s\n' "$listing" | grep -c " $name/share/icons/hicolor/.*/apps/$APP_ID_RE\.png\$")" = 13 ] ||
        { say "tarball: not 13 icon sizes"; status=1; }
    printf '%s\n' "$listing" | grep -qE " $name/(share/lintian|DEBIAN|etc)/?" &&
        { say "tarball: holds a Debian-only file"; status=1; }
    [ "$status" = 0 ] || die "$(basename "$tarball") is not as docs/packaging.md §12 describes"
    say "built $(basename "$tarball"): $(stat -c %s "$tarball") bytes"
    (cd "$OUT" && sha256sum "$(basename "$tarball")")
}

# Every crate the binary links (normal edges, no proc macros: their code runs at compile time)
# is named in packaging/copyright, under a licence that offers MIT.
check_crates() {
    local line name license status=0
    while IFS='|' read -r line license; do
        name="${line%% *}"
        case "$name" in "$PKG" | fc-core | fc-logic | fc-text) continue ;; esac
        if ! printf '%s\n' "$license" | grep -qxE "$ALLOWED_CRATE_LICENSES" &&
            ! { [ "$license" = Zlib ] && printf '%s\n' $ZLIB_CRATES | grep -qxF "$name"; }; then
            say "crate $line has licence '$license', which does not offer MIT"; status=1
        fi
        # shellcheck disable=SC2020 # a set of characters is what is meant: blanks become newlines
        if ! tr -s ' \n' '\n\n' < packaging/copyright | tr -d ',.;' | grep -qx "$name"; then
            say "crate $name is linked into $PKG but not named in packaging/copyright"; status=1
        fi
    done < <(cargo tree --locked -p "$PKG" -e normal,no-proc-macro --prefix none --format '{p}|{l}' |
        sed 's/ (\*)$//' | sort -u)
    [ "$status" = 0 ] || die "packaging/copyright does not cover the crates linked into $PKG"
}

# dpkg-shlibdeps' Depends for the binary, every package checked against ALLOWED_DEPENDS, the
# floors of RAISED_FLOORS raised, PLANNED_LIBRARIES and RUNTIME_DEPENDS appended.
compute_depends() {
    local bin="$1" scratch shlibs entry pkg floor raised result="" named=""
    scratch="$(mktemp -d /tmp/fc-shlibs.XXXXXX)"
    mkdir -p "$scratch/debian"
    printf '%s\n' "Source: $PKG" "" "Package: $PKG" "Architecture: any" > "$scratch/debian/control"
    shlibs="$(cd "$scratch" && dpkg-shlibdeps -O -e"$bin" 2>"$scratch/warnings")" ||
        { cat "$scratch/warnings" >&2; die "dpkg-shlibdeps failed"; }
    # On amd64, Ubuntu's usr-merge diverts /lib64/ld-linux-x86-64.so.2, so dpkg-shlibdeps warns
    # that diversions are involved and, unable to attribute symbols through the diverted loader,
    # that linking it is useless. libc6 provides the loader and is in Depends either way; those
    # two messages are dropped, anything else is shown.
    grep -vE 'diversions involved - output may be incorrect|diversion by libc6 (from|to): /lib64/ld-linux-x86-64\.so\.2|could avoid a useless dependency if .* was not linked against ld-linux-x86-64\.so\.2' \
        "$scratch/warnings" | sed 's/^/package: dpkg-shlibdeps: /' >&2 || true
    rm -rf "$scratch"
    shlibs="${shlibs#shlibs:Depends=}"
    while IFS= read -r entry; do
        [ -n "$entry" ] || continue
        case "$entry" in *'|'*) die "an alternative in Depends: $entry" ;; esac
        pkg="${entry%% *}"
        # shellcheck disable=SC2086 # one package per word
        printf '%s\n' $ALLOWED_DEPENDS | grep -qxF "$pkg" ||
            die "dpkg-shlibdeps names $pkg, which ALLOWED_DEPENDS does not expect: review the new linkage"
        # shellcheck disable=SC2086 # one package:floor per word
        raised="$(printf '%s\n' $RAISED_FLOORS | awk -F: -v p="$pkg" '$1 == p { print $2 }')"
        if [ -n "$raised" ]; then
            floor="$(printf '%s' "$entry" | sed -n 's/^.*(>= \(.*\))$/\1/p')"
            if [ -z "$floor" ] || dpkg --compare-versions "$floor" lt "$raised"; then
                entry="$pkg (>= $raised)"
            fi
        fi
        result="${result:+$result, }$entry"
        named="$named $pkg"
    done <<EOF
$(printf '%s\n' "$shlibs" | tr ',' '\n' | sed 's/^ *//; s/ *$//')
EOF
    for pkg in $PLANNED_LIBRARIES; do
        case " $named " in *" $pkg "*) ;; *) result="$result, $pkg" ;; esac
    done
    printf '%s' "$result, $(printf '%s' "$RUNTIME_DEPENDS" | tr '\n' ' ' | sed 's/  */ /g')"
}

# What dpkg-gencontrol computes: each file rounded up to KiB, every other entry 1 KiB.
installed_size() {
    (cd "$1" && find . -mindepth 1 -path ./DEBIAN -prune -o -printf '%y %s\n') |
        awk '{ if ($1 == "f") total += int(($2 + 1023) / 1024); else total += 1 } END { print total }'
}

# dpkg-deb --info / --contents against docs/packaging.md §1.
check_layout() {
    local deb="$1" version="$2" arch="$3" depends="$4" status=0 tmp
    tmp="$(mktemp -d /tmp/fc-layout.XXXXXX)"

    dpkg-deb --info "$deb" > "$tmp/info"
    local field value
    while IFS='|' read -r field value; do
        if ! grep -qxF " $field: $value" "$tmp/info"; then say "control: want '$field: $value'"; status=1; fi
    done <<EOF
Package|$PKG
Version|$version
Architecture|$arch
Maintainer|nettrash <nettrash@nettrash.me>
Depends|$depends
Recommends|$RECOMMENDS
Section|net
Priority|optional
Homepage|https://nettrash.me/linux/familyconnect/support.html
Description|family chat and calls on the family's own server
EOF
    grep -qE '^ Installed-Size: [0-9]+$' "$tmp/info" || { say "control: no Installed-Size"; status=1; }
    local members
    members="$(dpkg-deb --ctrl-tarfile "$deb" | tar -t | sed 's#^\./##' | grep -v '^$' | LC_ALL=C sort | tr '\n' ' ')"
    [ "$members" = "conffiles control md5sums postinst postrm " ] ||
        { say "control members: $members"; status=1; }

    # Every entry root:root, mtime SOURCE_DATE_EPOCH, mode 0755 for directories, the binary and
    # nothing else, 0644 for every other file. tar lists the times in the local zone, so in UTC like
    # the stamp: a desktop's own zone (--native) would shift every one of them.
    TZ=UTC0 dpkg-deb --contents "$deb" > "$tmp/contents"
    local stamp
    stamp="$(date -u -d "@$SOURCE_DATE_EPOCH" '+%Y-%m-%d %H:%M')"
    awk -v stamp="$stamp" -v bin="./usr/bin/$PKG" '
        { path = $6; for (i = 7; i <= NF; i++) path = path " " $i }
        $2 != "root/root" { print "owner " $2 ": " path; bad = 1 }
        ($4 " " $5) != stamp { print "mtime " $4 " " $5 ": " path; bad = 1 }
        /^d/ && $1 != "drwxr-xr-x" { print "mode " $1 ": " path; bad = 1 }
        /^-/ && path == bin && $1 != "-rwxr-xr-x" { print "mode " $1 ": " path; bad = 1 }
        /^-/ && path != bin && $1 != "-rw-r--r--" { print "mode " $1 ": " path; bad = 1 }
        /^[^d-]/ { print "not a file or directory: " path; bad = 1 }
        END { exit bad }' "$tmp/contents" >&2 || status=1

    # The file list is exactly §1: the fixed files, the icons and the data folder.
    awk '/^-/ { path = $6; for (i = 7; i <= NF; i++) path = path " " $i; sub(/^\.\//, "", path); print path }' \
        "$tmp/contents" | LC_ALL=C sort > "$tmp/actual"
    {
        # shellcheck disable=SC2086 # one path per word
        printf '%s\n' $LAYOUT_FILES
        (cd packaging/icons && find hicolor -type f) | sed 's#^#usr/share/icons/#'
        (cd data && find . -type f) | sed "s#^\./#usr/share/$PKG/#"
    } | LC_ALL=C sort > "$tmp/expected"
    if ! cmp -s "$tmp/expected" "$tmp/actual"; then
        say "contents differ from docs/packaging.md §1 (< expected, > in the .deb):"
        diff "$tmp/expected" "$tmp/actual" >&2 || true
        status=1
    fi
    [ "$(grep -c "^usr/share/icons/hicolor/.*/apps/$APP_ID_RE\.png\$" "$tmp/actual")" = 13 ] ||
        { say "not 13 icon sizes"; status=1; }
    rm -rf "$tmp"
    [ "$status" = 0 ] || die "the .deb does not match docs/packaging.md §1"
    say "info and contents match docs/packaging.md §1"
}

# ---------------------------------------------------------------------------------------------
# Inside a fresh ubuntu:26.04: install, check, remove, purge, the archive, reinstall
# (docs/packaging.md §9.3).

check_inside() {
    local arch="$1" deb version status=0 machine
    version="$(tree_version /src/linux)"
    deb="/dist/${PKG}_${version}_${arch}.deb"
    [ -f "$deb" ] || die "no $deb"
    export DEBIAN_FRONTEND=noninteractive
    fail() { say "CHECK FAILED: $*"; status=1; }
    ok() { say "ok: $*"; }

    say "apt: the checking tools"
    apt-get update -qq
    # Installed first so that their triggers (desktop-file-utils) run when the package is. bsdtar
    # unpacks the portable archive: GNU tar 1.35 cannot create a file or folder at all under
    # Docker Desktop's Rosetta (amd64 on an Apple-silicon Mac: "Cannot mkdir: Function not
    # implemented"), while libarchive can.
    apt-get install -y -qq --no-install-recommends \
        desktop-file-utils appstream apparmor gstreamer1.0-tools libarchive-tools ca-certificates >/dev/null

    say "apt install ./$(basename "$deb") — every Depends from the archive"
    cp "$deb" /tmp/
    apt-get install -y --no-install-recommends "/tmp/$(basename "$deb")" > /tmp/apt-install.log 2>&1 ||
        { cat /tmp/apt-install.log >&2; die "apt could not install the package"; }
    grep -E '^(W|E):' /tmp/apt-install.log >&2 && fail "apt printed warnings or errors"
    ok "installed: $(dpkg-query -W -f '${Package} ${Version} ${Architecture} ${db:Status-Status}' "$PKG")"
    dpkg -L "$PKG" | LC_ALL=C sort > /tmp/fc-files

    if ldd "/usr/bin/$PKG" | grep -q 'not found'; then ldd "/usr/bin/$PKG" >&2; fail "unresolved libraries"; else
        ok "ldd: every library resolves"; fi

    local v
    v="$(timeout 30 "$PKG" --version 2>&1 | head -1 || true)"
    if [ "$v" = "$PKG $version" ]; then ok "$PKG --version: $v"
    else fail "$PKG --version printed '$v', not '$PKG $version'"; fi

    # shellcheck disable=SC2086 # one element per word
    check_gst_elements $GST_ELEMENTS

    # The Recommends are not installed (--no-install-recommends keeps this system lean), but they
    # must resolve from the archive.
    local recommends
    recommends="$(dpkg-query -W -f '${Recommends}' "$PKG" | tr -d ',')"
    # shellcheck disable=SC2086 # one package per word
    if apt-get install -s -qq --no-install-recommends $recommends >/dev/null 2>/tmp/apt-rec.log; then
        ok "the Recommends resolve: $recommends"
    else cat /tmp/apt-rec.log >&2; fail "the Recommends do not resolve: $recommends"; fi

    # The AppArmor profile: postinst created its local include; the profile parses (no kernel
    # here, so it is compiled but not loaded — loading it is the first desktop check, §11).
    [ -f "/etc/apparmor.d/local/$PKG" ] && ok "postinst created /etc/apparmor.d/local/$PKG" ||
        fail "no /etc/apparmor.d/local/$PKG"
    if apparmor_parser --skip-kernel-load --skip-cache -Q "/etc/apparmor.d/$PKG" 2>/tmp/aa.log; then
        ok "apparmor_parser compiles /etc/apparmor.d/$PKG"
    else cat /tmp/aa.log >&2; fail "apparmor_parser rejects /etc/apparmor.d/$PKG"; fi

    local out
    out="$(desktop-file-validate "/usr/share/applications/$APP_ID.desktop" 2>&1)" || true
    if [ -z "$out" ]; then ok "desktop-file-validate: clean"; else printf '%s\n' "$out" >&2
        fail "desktop-file-validate"; fi
    # "Open With" (docs/packaging.md §2): listed for pictures, video, sound and PDF — never the
    # default for any of them, and never for "any file".
    for mime in image/jpeg video/mp4 audio/mpeg application/pdf; do
        grep -qE "^$mime=(.*;)?$APP_ID\.desktop;" /usr/share/applications/mimeinfo.cache 2>/dev/null &&
            ok "mimeinfo.cache lists $APP_ID for $mime" || fail "mimeinfo.cache does not list $APP_ID for $mime"
    done
    grep -qE "^(application/octet-stream|inode/directory)=.*$APP_ID" /usr/share/applications/mimeinfo.cache 2>/dev/null &&
        fail "mimeinfo.cache lists $APP_ID for any file"
    grep -qx "Exec=/usr/bin/$PKG --gapplication-service" "/usr/share/dbus-1/services/$APP_ID.service" &&
        ok "the D-Bus service starts /usr/bin/$PKG" || fail "no D-Bus service for $APP_ID"
    if out="$(appstreamcli validate --no-net --pedantic "/usr/share/metainfo/$APP_ID.metainfo.xml" 2>&1)"; then
        printf '%s\n' "$out" | sed 's/^/  /' >&2; ok "appstreamcli validate --no-net --pedantic"
    else printf '%s\n' "$out" >&2; fail "appstreamcli validate --no-net --pedantic"; fi

    # The user's own files, which no step below may touch (where the client will keep its
    # settings, cache and state).
    mkdir -p /root/.config/$PKG /root/.local/state/$PKG /root/.local/share/$PKG
    printf '[fc]\n' > /root/.config/$PKG/settings.conf
    printf '{"version":1}\n' > /root/.local/state/$PKG/session.json
    printf 'cache\n' > /root/.local/share/$PKG/cache.sqlite
    local user_files="/root/.config/$PKG/settings.conf /root/.local/state/$PKG/session.json /root/.local/share/$PKG/cache.sqlite"

    say "apt remove $PKG"
    apt-get remove -y -qq "$PKG" >/dev/null
    [ "$(dpkg-query -W -f '${db:Status-Status}' "$PKG")" = config-files ] && ok "removed: config-files remain" ||
        fail "status after remove is $(dpkg-query -W -f '${db:Status-Status}' "$PKG")"
    [ -f "/etc/apparmor.d/$PKG" ] && ok "/etc/apparmor.d/$PKG kept (a conffile)" || fail "/etc/apparmor.d/$PKG is gone after remove"
    [ -f "/etc/apparmor.d/local/$PKG" ] && ok "/etc/apparmor.d/local/$PKG kept" || fail "/etc/apparmor.d/local/$PKG is gone after remove"
    local path left=""
    while IFS= read -r path; do
        case "$path" in /etc/apparmor.d | "/etc/apparmor.d/$PKG") continue ;; esac
        if [ -f "$path" ] || [ -L "$path" ]; then left="$left $path"; fi
    done < /tmp/fc-files
    [ -z "$left" ] && ok "remove left no packaged file but the conffile" || fail "left after remove:$left"
    for path in $user_files; do [ -f "$path" ] || fail "remove deleted the user's $path"; done

    say "apt purge $PKG"
    apt-get purge -y -qq "$PKG" >/dev/null
    dpkg-query -W "$PKG" >/dev/null 2>&1 && fail "dpkg still knows $PKG after purge" || ok "purged: dpkg no longer knows $PKG"
    if dpkg -L "$PKG" >/dev/null 2>&1; then fail "dpkg -L $PKG still lists files"; else ok "dpkg -L $PKG reports nothing"; fi
    [ -e "/etc/apparmor.d/$PKG" ] && fail "/etc/apparmor.d/$PKG remains after purge"
    [ -e "/etc/apparmor.d/local/$PKG" ] && fail "/etc/apparmor.d/local/$PKG remains after purge"
    [ ! -e "/etc/apparmor.d/$PKG" ] && [ ! -e "/etc/apparmor.d/local/$PKG" ] &&
        ok "purge removed the profile and its local include"
    for path in $user_files; do [ -f "$path" ] || fail "purge deleted the user's $path"; done
    ok "the user's settings, state and cache are untouched"

    # The portable archive, on the libraries the .deb brought (still installed: apt removes no
    # dependency by itself): unpacked anywhere, it runs, and finds its icons beside it.
    case "$arch" in amd64) machine=x86_64 ;; arm64) machine=aarch64 ;; esac
    local tarball="/dist/$PKG-$version-linux-$machine.tar.gz" unpacked=/tmp/fc-unpacked
    mkdir -p "$unpacked"
    bsdtar -xzf "$tarball" -C "$unpacked"
    local exe="$unpacked/$PKG-$version/bin/$PKG"
    if ldd "$exe" | grep -q 'not found'; then ldd "$exe" >&2; fail "the archive's binary has unresolved libraries"; fi
    v="$(timeout 30 "$exe" --version 2>&1 | head -1 || true)"
    if [ "$v" = "$PKG $version" ]; then ok "$(basename "$tarball") unpacked: bin/$PKG --version: $v"
    else fail "the archive's bin/$PKG --version printed '$v'"; fi
    [ -f "$unpacked/$PKG-$version/share/icons/hicolor/48x48/apps/$APP_ID.png" ] &&
        ok "the archive's icons are in share/icons beside bin/" || fail "the archive has no share/icons/hicolor"

    say "apt install again"
    apt-get install -y -qq --no-install-recommends "/tmp/$(basename "$deb")" >/dev/null 2>&1 &&
        [ -x "/usr/bin/$PKG" ] && [ -f "/etc/apparmor.d/local/$PKG" ] && ok "reinstall works" || fail "reinstall"

    [ "$status" = 0 ] || die "checks failed for $(basename "$deb")"
    say "every check passed for $(basename "$deb")"
}

# lintian (docs/packaging.md §8.2), in its own throwaway ubuntu:26.04 of the machine's own
# architecture: lintian inspects the .deb and never runs it, and under Rosetta its unpacking hits
# the tar limitation deb_inside describes.
lintian_inside() {
    local arch="$1" version deb
    version="$(tree_version /src/linux)"
    deb="/dist/${PKG}_${version}_${arch}.deb"
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends lintian >/dev/null
    cp "$deb" /tmp/lintian.deb && chmod 0644 /tmp/lintian.deb
    lintian_check /tmp/lintian.deb "$(basename "$deb")" runuser -u nobody -- env HOME=/tmp
}

# lintian on the .deb $1 (named $2 in the messages), started through the command in the remaining
# arguments (none for --native, which runs as the user already), at every level — errors,
# warnings, info, pedantic, experimental and overrides — and the verdict: no tag but the
# experimental hardening-no-fortify-functions on the binary, and the three of ACCEPTED_TAGS, each on
# its own files only (docs/packaging.md §8.2). The first is also the proof that lintian analysed the
# binary: it is raised for every Rust binary (Rust calls no fortified libc function), and a run that
# read nothing prints nothing either.
#
# ACCEPTED_TAGS, one extended regular expression per line, each a whole tag line:
# - the rich page's KaTeX web fonts, which WebKit loads through the app's own fc-rich scheme from
#   /usr/share/family-connect/rich — page assets, not system fonts (web/rich is shared, byte for
#   byte, by every client);
# - that page itself, rich.html, which lintian takes for documentation;
# - "misspellings" in the binary: the nine languages' sentences compiled into it (fc-text), whose
#   German, French and Spanish words lintian's English dictionary reads as errors.
ACCEPTED_TAGS="I: $PKG: font-(in-non-font-package|outside-font-dir) \[usr/share/$PKG/rich/fonts/KaTeX_[A-Za-z0-9-]+\.(woff2|woff|ttf)\]
I: $PKG: package-contains-documentation-outside-usr-share-doc \[usr/share/$PKG/rich/rich\.html\]
X: $PKG: spelling-error-in-binary [^ ]+ [^\[]+ \[usr/bin/$PKG(:[0-9]+)?\]"
lintian_check() {
    local deb="$1" name="$2" log code=0 status=0
    local expected="X: $PKG: hardening-no-fortify-functions [usr/bin/$PKG]"
    shift 2
    log="$(mktemp /tmp/fc-lintian.XXXXXX)"
    say "lintian $(dpkg-query -W -f '${Version}' lintian 2>/dev/null || lintian --print-version) on $name"
    "$@" lintian --display-info --pedantic --display-experimental --show-overrides --tag-display-limit 0 \
        "$deb" > "$log" 2>&1 || code=$?
    sed 's/^/  /' "$log" >&2
    if [ "$code" -gt 1 ]; then say "CHECK FAILED: lintian could not run (exit $code)"; status=1; fi
    # Only tag lines count ("X: package: tag [...]"): lintian also prints notices, such as "running
    # with root privileges is not recommended!" for a --native build run as root.
    if grep -E '^[A-Z]: ' "$log" | grep -vxF "$expected" | grep -vxE "$ACCEPTED_TAGS" | grep -q .; then
        say "CHECK FAILED: lintian reports a tag other than the expected ones:"
        grep -E '^[A-Z]: ' "$log" | grep -vxF "$expected" | grep -vxE "$ACCEPTED_TAGS" | sed 's/^/  /' >&2
        status=1
    fi
    if ! grep -qxF "$expected" "$log"; then
        say "CHECK FAILED: lintian did not report '$expected': it did not analyse the binary"; status=1
    fi
    rm -f "$log"
    [ "$status" = 0 ] || die "lintian checks failed for $name"
    say "ok: lintian: no tag at any level but the expected experimental one and ACCEPTED_TAGS"
}

# --native: this machine should be the release it packages for (Depends comes from its libraries),
# with docker/Dockerfile's build packages.
check_native_machine() {
    local os tool missing=""
    os="$(. /etc/os-release 2>/dev/null && printf '%s %s' "${ID:-?}" "${VERSION_ID:-?}")" || os="unknown"
    [ "$os" = "ubuntu 26.04" ] || say "warning: this system is '$os', not ubuntu 26.04, which the package is for"
    for tool in git cargo rustc pkg-config dpkg-deb dpkg-shlibdeps readelf strip; do
        command -v "$tool" >/dev/null || missing="$missing $tool"
    done
    if command -v pkg-config >/dev/null && ! pkg-config --exists gtk4 libadwaita-1 webkitgtk-6.0 gstreamer-1.0; then
        missing="$missing gtk4/libadwaita-1/webkitgtk-6.0/gstreamer-1.0 (pkg-config)"
    fi
    [ -z "$missing" ] || die "missing:$missing — install the build packages: sudo apt install build-essential pkg-config git rustc cargo libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev libsecret-1-dev libsqlite3-dev"
}

# ---------------------------------------------------------------------------------------------
# On the host.

case "${1:-}" in
    --build-inside) build_inside "$2"; exit ;;
    --deb-inside) deb_inside "$2"; exit ;;
    --lintian-inside) lintian_inside "$2"; exit ;;
    --check-inside) check_inside "$2"; exit ;;
esac

clean=0 check=1 allow_dirty=0 native_build=0
arches=()
while [ $# -gt 0 ]; do
    case "$1" in
        --clean) clean=1 ;;
        --no-check) check=0 ;;
        --allow-dirty) allow_dirty=1 ;;
        --native) native_build=1 ;;
        amd64 | arm64) arches+=("$1") ;;
        *) echo "usage: scripts/package.sh [--native] [--clean] [--no-check] [--allow-dirty] [amd64|arm64 …]" >&2; exit 2 ;;
    esac
    shift
done
if [ ${#arches[@]} -eq 0 ]; then
    case "$(uname -m)" in arm64 | aarch64) arches=(arm64) ;; *) arches=(amd64) ;; esac
fi

if [ "$native_build" = 1 ]; then
    check_native_machine
else
    command -v docker >/dev/null || die "docker is not installed (on Ubuntu 26.04, --native builds without it)"
    BASE="$(sed -n 's/^FROM \(ubuntu:26\.04@sha256:[0-9a-f]*\).*$/\1/p' "$ROOT/docker/Dockerfile")"
    [ -n "$BASE" ] || die "docker/Dockerfile does not pin ubuntu:26.04 by digest"
fi
git -C "$REPO" rev-parse --verify HEAD >/dev/null 2>&1 || die "$REPO is not a git checkout"
SOURCE_DATE_EPOCH="$(git -C "$REPO" log -1 --format=%ct HEAD)"
export SOURCE_DATE_EPOCH

# The source, from git (step 2).
export_dir="$ROOT/dist/.package-source"
rm -rf "$export_dir"
mkdir -p "$export_dir"
export_source "$REPO" "$export_dir" "$allow_dirty"
mkdir -p "$ROOT/dist"

# --native: step 3 on this machine, then lintian (step 4's other checks install the package).
if [ "$native_build" = 1 ]; then
    host="$(dpkg --print-architecture)"
    for arch in "${arches[@]}"; do
        [ "$arch" = "$host" ] || die "--native builds for this machine's architecture ($host) only, not $arch"
    done
    SRC="$export_dir" OUT="$ROOT/dist" CARGO_DIR="${CARGO_HOME:-$HOME/.cargo}"
    STAGE="$(mktemp -d "${TMPDIR:-/tmp}/fc-package-stage.XXXXXX")"
    if [ "$clean" = 1 ]; then TARGET="$ROOT/target/package-clean-$$"; else TARGET="$ROOT/target/package"; fi
    trap 'rm -rf "$STAGE" "$export_dir"; if [ "$clean" = 1 ]; then rm -rf "$TARGET"; fi' EXIT
    build_inside "$host"
    deb_inside "$host"
    version="$(tree_version "$SRC/linux")"
    deb="$OUT/${PKG}_${version}_${host}.deb"
    if [ "$check" = 0 ]; then :
    elif command -v lintian >/dev/null; then lintian_check "$deb" "$(basename "$deb")"
    else say "lintian is not installed, so it did not run (sudo apt install lintian)"; fi
    say "install it with: sudo apt install $(printf '%q' "$deb")"
    exit 0
fi

case "$(docker version --format '{{.Server.Arch}}')" in
    arm64 | aarch64) native=arm64 ;;
    *) native=amd64 ;;
esac
build_image() {
    if [ "${FC_PACKAGE_SKIP_IMAGE:-0}" != 1 ]; then
        say "image fc-linux-dev:$1 (docker/Dockerfile, linux/$1)"
        docker buildx build --quiet --platform "linux/$1" --load -t "fc-linux-dev:$1" "$ROOT/docker" >/dev/null
    fi
}
built_native=0
checkout="$(basename "$REPO")"

for arch in "${arches[@]}"; do
    image="fc-linux-dev:$arch"
    build_image "$arch"
    if [ "$arch" = "$native" ]; then built_native=1
    elif [ "$built_native" = 0 ]; then build_image "$native"; built_native=1; fi
    stage_volume="fc-linux-package-stage-$checkout-$arch"
    if [ "$clean" = 1 ]; then
        target_volume="fc-linux-package-clean-$arch-$$"
    else
        target_volume="fc-linux-package-target-$checkout-$arch"
    fi
    status=0
    docker run --rm --platform "linux/$arch" \
        -v "$export_dir:/src:ro" \
        -v fc-linux-cargo:/cargo \
        -v "$target_volume:/target" \
        -v "$stage_volume:/stage" \
        -e SOURCE_DATE_EPOCH \
        "$image" bash /src/linux/scripts/package.sh --build-inside "$arch" || status=$?
    if [ "$clean" = 1 ]; then docker volume rm "$target_volume" >/dev/null || true; fi
    if [ "$status" = 0 ]; then
        docker run --rm --platform "linux/$native" \
            -v "$export_dir:/src:ro" \
            -v "$ROOT/dist:/out" \
            -v "$stage_volume:/stage" \
            -e SOURCE_DATE_EPOCH \
            "fc-linux-dev:$native" bash /src/linux/scripts/package.sh --deb-inside "$arch" || status=$?
    fi
    docker volume rm "$stage_volume" >/dev/null 2>&1 || true
    [ "$status" = 0 ] || die "the $arch build failed"

    if [ "$check" = 1 ]; then
        if [ "${FC_PACKAGE_SKIP_IMAGE:-0}" != 1 ]; then
            base_image "$BASE" "$arch" "fc-linux-ubuntu:$arch"
            base_image "$BASE" "$native" "fc-linux-ubuntu:$native"
        fi
        docker run --rm --platform "linux/$arch" \
            -v "$export_dir:/src:ro" \
            -v "$ROOT/dist:/dist:ro" \
            "fc-linux-ubuntu:$arch" bash /src/linux/scripts/package.sh --check-inside "$arch"
        docker run --rm --platform "linux/$native" \
            -v "$export_dir:/src:ro" \
            -v "$ROOT/dist:/dist:ro" \
            "fc-linux-ubuntu:$native" bash /src/linux/scripts/package.sh --lintian-inside "$arch"
    fi
done
rm -rf "$export_dir"
