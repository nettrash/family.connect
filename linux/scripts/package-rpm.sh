#!/usr/bin/env bash
# Family Connect for Linux — the .rpm for Fedora 44 and later (docs/packaging.md §12), adapted from
# md.linux's.
#
#   scripts/package-rpm.sh [options] [amd64|arm64 …]      default: the host's architecture
#
#   --clean         build in a new, empty target volume (removed afterwards)
#   --no-check      skip the install / remove checks in a fresh Fedora container
#   --allow-dirty   package the working tree although linux/, web/text/ or CHANGELOG has
#                   uncommitted changes
#
# Self-contained: it needs only Docker with buildx (the other architecture runs under emulation).
# The architectures are named as scripts/package.sh names them; the .rpm says x86_64 or aarch64.
# Per architecture it
#
#   1. builds the image fc-linux-rpm:<arch> from docker/Dockerfile.fedora (Fedora 44);
#   2. exports the source from git, as scripts/package.sh does;
#   3. in fc-linux-rpm:<arch>: cargo build --release --locked -p family-connect with the .deb's
#      path remapping, stages the files of docs/packaging.md §12, and rpmbuild -bb
#      packaging/rpm/family-connect.spec writes
#      dist/family-connect-<version>-1.fc44.<x86_64|aarch64>.rpm, whose header, Requires and file
#      list are checked; then rpmlint, in fc-linux-rpm of the machine's own architecture (see
#      rpmlint_inside), must report no error and no warning;
#   4. in a fresh Fedora 44 of the same architecture (the image fc-linux-fedora:<arch>, the pinned
#      base and nothing else): dnf installs the .rpm, every Requires from the archive;
#      family-connect --version, ldd, the GStreamer elements the Requires are for (and the
#      Recommends, installed on its own), desktop-file-validate, appstreamcli validate --no-net;
#      then dnf remove, which must leave nothing of the package's behind and nothing of the
#      user's touched.
#
# The package's build time and file times are SOURCE_DATE_EPOCH, the commit time of HEAD.
#
# Environment: FC_PACKAGE_SKIP_IMAGE=1 reuses the existing images fc-linux-rpm:<arch> and
# fc-linux-fedora:<arch>.
# `A && ok … || fail …` is meant as if-else: `ok` and `fail` both return 0.
# shellcheck disable=SC2015
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"   # linux/
REPO="$(cd "$ROOT/.." && pwd)"                             # the repository
PKG=family-connect
APP_ID=me.nettrash.FamilyConnect
APP_ID_RE='me\.nettrash\.FamilyConnect'

say() { printf '%s\n' "package-rpm: $*" >&2; }
die() { say "FAILED: $*"; exit 1; }
# shellcheck source=scripts/package-lib.sh
. "$(dirname "${BASH_SOURCE[0]}")/package-lib.sh"

# The GStreamer elements the .rpm's Requires are for (packaging/rpm/family-connect.spec), and the
# ones its openh264 Recommends is for. Fedora has no x264enc and no avdec_h264: H.264 is openh264's.
GST_ELEMENTS="webrtcbin nicesrc nicesink dtlssrtpenc dtlssrtpdec rtpopuspay rtpvp8pay opusenc opusdec
vp8enc vp8dec h264parse avenc_aac mp4mux qtdemux pipewiresrc gtk4paintablesink"
GST_RECOMMENDED="openh264enc openh264dec"

machine_of() {
    case "$1" in amd64) echo x86_64 ;; arm64) echo aarch64 ;; *) die "unknown architecture $1" ;; esac
}

# ---------------------------------------------------------------------------------------------
# Inside fc-linux-rpm:<arch>: build, stage, rpmbuild, check the package.

build_inside() {
    local arch="$1" machine version stage top bin rpm
    cd /src/linux
    machine="$(machine_of "$arch")"
    [ "$(uname -m)" = "$machine" ] || die "container is $(uname -m), not $machine"
    [ -n "${SOURCE_DATE_EPOCH:-}" ] || die "SOURCE_DATE_EPOCH is not set"
    version="$(tree_version .)"
    [ -n "$version" ] || die "no [workspace.package] version in linux/Cargo.toml"
    grep -q " - $version-1\$" "packaging/rpm/$PKG.spec" ||
        die "packaging/rpm/$PKG.spec's %changelog does not start at $version"

    export_rustflags /src "${CARGO_HOME:-/cargo}" "${CARGO_TARGET_DIR:-/target}"
    export CARGO_PROFILE_RELEASE_STRIP=true CARGO_PROFILE_RELEASE_DEBUG=false
    say "cargo build --release --locked -p $PKG ($machine, $(rustc --version))"
    cargo build --release --locked -p "$PKG"
    bin="${CARGO_TARGET_DIR:-/target}/release/$PKG"
    if readelf -S "$bin" | grep -qE '\.(symtab|debug_[a-z]+)'; then die "the binary is not stripped"; fi

    # §12: the .deb's files (§1) without the AppArmor profile; the man page uncompressed (rpm's
    # brp-compress gzips it), CHANGELOG uncompressed, as Fedora ships documents, and the copyright
    # file under licenses/. The Debian changelog is the .deb's alone: the spec has its own. The
    # binary keeps its build-id note, which rpm reads.
    stage=/tmp/fc-stage
    rm -rf "$stage"
    install -D -m 0755 "$bin" "$stage/usr/bin/$PKG"
    strip --remove-section=.comment "$stage/usr/bin/$PKG"
    install -D -m 0644 "packaging/$APP_ID.desktop" "$stage/usr/share/applications/$APP_ID.desktop"
    install -D -m 0644 "packaging/$APP_ID.metainfo.xml" "$stage/usr/share/metainfo/$APP_ID.metainfo.xml"
    install -D -m 0644 "packaging/dbus/$APP_ID.service" "$stage/usr/share/dbus-1/services/$APP_ID.service"
    mkdir -p "$stage/usr/share/icons"
    cp -R packaging/icons/hicolor "$stage/usr/share/icons/"
    install -D -m 0644 "packaging/$PKG.1" "$stage/usr/share/man/man1/$PKG.1"
    install -D -m 0644 ../CHANGELOG "$stage/usr/share/doc/$PKG/CHANGELOG"
    install -D -m 0644 packaging/copyright "$stage/usr/share/licenses/$PKG/copyright"
    # The data folder, as the .deb's (docs/shell-design.md §2.3): /usr/share/family-connect.
    mkdir -p "$stage/usr/share/$PKG"
    cp -R data/. "$stage/usr/share/$PKG/"
    find "$stage" -type d -exec chmod 0755 {} +
    find "$stage" -type f -exec chmod 0644 {} +
    chmod 0755 "$stage/usr/bin/$PKG"
    find "$stage" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +

    top=/tmp/fc-rpmbuild
    rm -rf "$top"
    say "rpmbuild -bb packaging/rpm/$PKG.spec ($(rpmbuild --version))"
    rpmbuild -bb --quiet \
        --define "_topdir $top" \
        --define "fc_version $version" \
        --define "fc_stage $stage" \
        --define "_buildhost fc-linux" \
        --define "use_source_date_epoch_as_buildtime 1" \
        --define "clamp_mtime_to_source_date_epoch 1" \
        "packaging/rpm/$PKG.spec"
    rpm="$(find "$top/RPMS/$machine" -name "$PKG-$version-1.*.$machine.rpm" -print -quit)"
    [ -n "$rpm" ] || die "rpmbuild wrote no $PKG-$version-1.*.$machine.rpm"
    check_rpm "$rpm" "$version" "$machine" "$stage"
    rm -f /out/"$PKG-$version"-*."$machine".rpm
    cp "$rpm" /out/
    rm -rf "$stage" "$top"
    say "built $(basename "$rpm"): $(stat -c %s "/out/$(basename "$rpm")") bytes"
    (cd /out && sha256sum "$(basename "$rpm")")
}

# The header, Requires and file list against §12.
check_rpm() {
    local rpm="$1" version="$2" machine="$3" stage="$4" status=0 tmp want
    tmp="$(mktemp -d /tmp/fc-rpmcheck.XXXXXX)"

    [ "$(rpm -qp --qf '%{NAME} %{VERSION} %{ARCH}' "$rpm")" = "$PKG $version $machine" ] ||
        { say "header: $(rpm -qp --qf '%{NAME} %{VERSION} %{ARCH}' "$rpm")"; status=1; }
    rpm -qpR "$rpm" | LC_ALL=C sort -u > "$tmp/requires"
    # %{?_isa}: (x86-64) or (aarch-64).
    local isa
    case "$machine" in x86_64) isa=x86-64 ;; aarch64) isa=aarch-64 ;; esac
    # rpm's generators: the libraries of the -sys crates, by soname; the rest by hand (the spec).
    for want in 'libgtk-4.so.1()(64bit)' 'libadwaita-1.so.0()(64bit)' 'libwebkitgtk-6.0.so.4()(64bit)' \
        'libgstreamer-1.0.so.0()(64bit)' 'libglib-2.0.so.0()(64bit)' \
        'libsecret-1.so.0()(64bit)' 'libsqlite3.so.0()(64bit)' hicolor-icon-theme \
        "glib-networking($isa)" \
        'gstreamer1(element-webrtcbin)()(64bit)' 'gstreamer1(element-nicesrc)()(64bit)' \
        'gstreamer1(element-avenc_aac)()(64bit)' 'gstreamer1(element-pipewiresrc)()(64bit)' \
        'gstreamer1(element-gtk4paintablesink)()(64bit)'; do
        grep -qxF "$want" "$tmp/requires" || { say "Requires: no $want"; status=1; }
    done
    say "Requires: $(grep -v '^rpmlib(' "$tmp/requires" | tr '\n' ' ')"
    [ "$(rpm -qp --recommends "$rpm" | LC_ALL=C sort | tr '\n' ' ')" = \
        'gnome-keyring gstreamer1(element-openh264enc)()(64bit) xdg-desktop-portal-gnome ' ] ||
        { say "Recommends: $(rpm -qp --recommends "$rpm" | tr '\n' ' ')"; status=1; }

    # The file list is exactly the staged tree, plus rpm's build-id links and the compressed man page.
    rpm -qpl "$rpm" | grep -v '^/usr/lib/\.build-id' | LC_ALL=C sort > "$tmp/actual"
    (cd "$stage" && find . -type f -printf '/%P\n') | sed "s#^/usr/share/man/man1/$PKG\\.1\$#&.gz#" |
        LC_ALL=C sort > "$tmp/files"
    # rpm also lists the directories it owns: the doc and licence folders, and the data folder
    # with every folder in it.
    {
        cat "$tmp/files"
        printf '%s\n' "/usr/share/doc/$PKG" "/usr/share/licenses/$PKG"
        (cd "$stage" && find "usr/share/$PKG" -type d) | sed 's#^#/#'
    } | LC_ALL=C sort > "$tmp/expected"
    if ! cmp -s "$tmp/expected" "$tmp/actual"; then
        say "contents differ from docs/packaging.md §12 (< expected, > in the .rpm):"
        diff "$tmp/expected" "$tmp/actual" >&2 || true
        status=1
    fi
    [ "$(grep -c "^/usr/share/icons/hicolor/.*/apps/$APP_ID_RE\.png\$" "$tmp/actual")" = 13 ] ||
        { say "not 13 icon sizes"; status=1; }

    rm -rf "$tmp"
    [ "$status" = 0 ] || die "the .rpm does not match docs/packaging.md §12"
    say "header, Requires and contents match docs/packaging.md §12"
}

# rpmlint, in fc-linux-rpm of the machine's own architecture — as scripts/package.sh runs lintian:
# rpmlint inspects the package and never runs it, and it unpacks it with `rpm2archive | tar -xz`,
# which GNU tar cannot do under Docker Desktop's Rosetta (amd64 on an Apple-silicon Mac). On
# GitHub's runners the machine's architecture is the package's. No error and no warning; the
# summary line must say one package was checked, or rpmlint read nothing.
rpmlint_inside() {
    local arch="$1" machine version rpm log code=0 status=0
    machine="$(machine_of "$arch")"
    version="$(tree_version /src/linux)"
    rpm="$(find /dist -maxdepth 1 -name "$PKG-$version-1.*.$machine.rpm" -print -quit)"
    [ -n "$rpm" ] || die "no $PKG-$version-1.*.$machine.rpm in dist/"
    log="$(mktemp /tmp/fc-rpmlint.XXXXXX)"
    say "$(rpmlint --version 2>/dev/null | head -1 || echo rpmlint) on $(basename "$rpm") ($(uname -m))"
    rpmlint "$rpm" > "$log" 2>&1 || code=$?
    sed 's/^/  /' "$log" >&2
    if grep -qE '^[^ ]+: [EW]: ' "$log" || [ "$code" -gt 1 ]; then
        say "CHECK FAILED: rpmlint reports errors or warnings"; status=1
    fi
    grep -qE '(^| )1 packages and 0 specfiles checked; 0 errors, 0 warnings' "$log" ||
        { say "CHECK FAILED: rpmlint did not report checking the package with 0 errors and 0 warnings"; status=1; }
    rm -f "$log"
    [ "$status" = 0 ] || die "rpmlint checks failed for $(basename "$rpm")"
    say "ok: rpmlint: no error or warning"
}

# ---------------------------------------------------------------------------------------------
# Inside a fresh Fedora 44: install, check, remove.

check_inside() {
    local arch="$1" machine version rpm status=0
    machine="$(machine_of "$arch")"
    version="$(tree_version /src/linux)"
    rpm="$(find /dist -maxdepth 1 -name "$PKG-$version-1.*.$machine.rpm" -print -quit)"
    [ -n "$rpm" ] || die "no $PKG-$version-1.*.$machine.rpm in dist/"
    fail() { say "CHECK FAILED: $*"; status=1; }
    ok() { say "ok: $*"; }

    say "dnf: the checking tools"
    # Installed first so that their file triggers run when the package is. gst-inspect-1.0 is in
    # gstreamer1 itself.
    dnf -y -q install --setopt=install_weak_deps=False \
        desktop-file-utils appstream findutils gstreamer1 > /tmp/dnf-tools.log 2>&1 ||
        { cat /tmp/dnf-tools.log >&2; die "dnf could not install the checking tools"; }

    say "dnf install ./$(basename "$rpm") — every Requires from the archive"
    dnf -y install --setopt=install_weak_deps=False "$rpm" > /tmp/dnf-install.log 2>&1 ||
        { cat /tmp/dnf-install.log >&2; die "dnf could not install the package"; }
    ok "installed: $(rpm -q "$PKG")"
    rpm -ql "$PKG" | LC_ALL=C sort > /tmp/fc-files

    if ldd "/usr/bin/$PKG" | grep -q 'not found'; then ldd "/usr/bin/$PKG" >&2; fail "unresolved libraries"; else
        ok "ldd: every library resolves"; fi
    local v
    v="$(timeout 30 "$PKG" --version 2>&1 | head -1 || true)"
    if [ "$v" = "$PKG $version" ]; then ok "$PKG --version: $v"
    else fail "$PKG --version printed '$v', not '$PKG $version'"; fi

    # shellcheck disable=SC2086 # one element per word
    check_gst_elements $GST_ELEMENTS
    # The weak dependencies, which the install above left out: each must be provided by the
    # archive, and openh264's is installed and its elements checked — it resolves where Cisco's
    # repository is on (it is in the image, as on Fedora Workstation).
    local cap h264='gstreamer1(element-openh264enc)()(64bit)'
    while IFS= read -r cap; do
        if [ -n "$(dnf -q repoquery --whatprovides "$cap" 2>/dev/null)" ]; then ok "Recommends $cap: in the archive"
        else fail "Recommends $cap: nothing in the archive provides it"; fi
    done < <(rpm -q --recommends "$PKG")
    if dnf -y -q install --setopt=install_weak_deps=False "$h264" > /tmp/dnf-rec.log 2>&1; then
        # shellcheck disable=SC2086
        check_gst_elements $GST_RECOMMENDED
    else cat /tmp/dnf-rec.log >&2; fail "the Recommends $h264 does not install"; fi

    local out
    out="$(desktop-file-validate "/usr/share/applications/$APP_ID.desktop" 2>&1)" || true
    if [ -z "$out" ]; then ok "desktop-file-validate: clean"; else printf '%s\n' "$out" >&2
        fail "desktop-file-validate"; fi
    if out="$(appstreamcli validate --no-net --pedantic "/usr/share/metainfo/$APP_ID.metainfo.xml" 2>&1)"; then
        printf '%s\n' "$out" | sed 's/^/  /' >&2; ok "appstreamcli validate --no-net --pedantic"
    else printf '%s\n' "$out" >&2; fail "appstreamcli validate --no-net --pedantic"; fi

    # The user's own files, which removing the package must not touch.
    mkdir -p /root/.config/$PKG /root/.local/state/$PKG /root/.local/share/$PKG
    printf '[fc]\n' > /root/.config/$PKG/settings.conf
    printf '{"version":1}\n' > /root/.local/state/$PKG/session.json
    printf 'cache\n' > /root/.local/share/$PKG/cache.sqlite
    local user_files="/root/.config/$PKG/settings.conf /root/.local/state/$PKG/session.json /root/.local/share/$PKG/cache.sqlite"

    say "dnf remove $PKG"
    dnf -y remove "$PKG" > /tmp/dnf-remove.log 2>&1 || { cat /tmp/dnf-remove.log >&2; die "dnf could not remove $PKG"; }
    rpm -q "$PKG" >/dev/null 2>&1 && fail "rpm still knows $PKG after remove" || ok "removed: rpm no longer knows $PKG"
    local path left=""
    while IFS= read -r path; do
        if [ -f "$path" ] || [ -L "$path" ]; then left="$left $path"; fi
    done < /tmp/fc-files
    [ -z "$left" ] && ok "remove left no packaged file" || fail "left after remove:$left"
    for path in $user_files; do [ -f "$path" ] || fail "remove deleted the user's $path"; done

    [ "$status" = 0 ] || die "checks failed for $(basename "$rpm")"
    say "every check passed for $(basename "$rpm")"
}

# ---------------------------------------------------------------------------------------------
# On the host.

case "${1:-}" in
    --build-inside) build_inside "$2"; exit ;;
    --rpmlint-inside) rpmlint_inside "$2"; exit ;;
    --check-inside) check_inside "$2"; exit ;;
esac

clean=0 check=1 allow_dirty=0
arches=()
while [ $# -gt 0 ]; do
    case "$1" in
        --clean) clean=1 ;;
        --no-check) check=0 ;;
        --allow-dirty) allow_dirty=1 ;;
        amd64 | arm64) arches+=("$1") ;;
        *) echo "usage: scripts/package-rpm.sh [--clean] [--no-check] [--allow-dirty] [amd64|arm64 …]" >&2; exit 2 ;;
    esac
    shift
done
if [ ${#arches[@]} -eq 0 ]; then
    case "$(uname -m)" in arm64 | aarch64) arches=(arm64) ;; *) arches=(amd64) ;; esac
fi

command -v docker >/dev/null || die "docker is not installed"
git -C "$REPO" rev-parse --verify HEAD >/dev/null 2>&1 || die "$REPO is not a git checkout"
SOURCE_DATE_EPOCH="$(git -C "$REPO" log -1 --format=%ct HEAD)"
export SOURCE_DATE_EPOCH
BASE="$(sed -n 's/^FROM \(fedora:[0-9]*@sha256:[0-9a-f]*\).*$/\1/p' "$ROOT/docker/Dockerfile.fedora")"
[ -n "$BASE" ] || die "docker/Dockerfile.fedora does not pin fedora by digest"

export_dir="$ROOT/dist/.package-rpm-source"
rm -rf "$export_dir"
mkdir -p "$export_dir"
export_source "$REPO" "$export_dir" "$allow_dirty"
mkdir -p "$ROOT/dist"
checkout="$(basename "$REPO")"
case "$(docker version --format '{{.Server.Arch}}')" in
    arm64 | aarch64) native=arm64 ;;
    *) native=amd64 ;;
esac
build_image() {
    if [ "${FC_PACKAGE_SKIP_IMAGE:-0}" != 1 ]; then
        say "image fc-linux-rpm:$1 (docker/Dockerfile.fedora, linux/$1)"
        docker buildx build --quiet --platform "linux/$1" --load -t "fc-linux-rpm:$1" \
            -f "$ROOT/docker/Dockerfile.fedora" "$ROOT/docker" >/dev/null
    fi
}
built_native=0

for arch in "${arches[@]}"; do
    image="fc-linux-rpm:$arch"
    build_image "$arch"
    if [ "$arch" = "$native" ]; then built_native=1
    elif [ "$built_native" = 0 ]; then build_image "$native"; built_native=1; fi
    if [ "$clean" = 1 ]; then
        target_volume="fc-linux-rpm-clean-$arch-$$"
    else
        target_volume="fc-linux-rpm-target-$checkout-$arch"
    fi
    status=0
    # Its own registry volume: Fedora's cargo is newer than Ubuntu's.
    docker run --rm --platform "linux/$arch" \
        -v "$export_dir:/src:ro" \
        -v fc-linux-rpm-cargo:/cargo \
        -v "$target_volume:/target" \
        -v "$ROOT/dist:/out" \
        -e SOURCE_DATE_EPOCH \
        "$image" bash /src/linux/scripts/package-rpm.sh --build-inside "$arch" || status=$?
    if [ "$clean" = 1 ]; then docker volume rm "$target_volume" >/dev/null || true; fi
    [ "$status" = 0 ] || die "the $arch build failed"
    docker run --rm --platform "linux/$native" \
        -v "$export_dir:/src:ro" \
        -v "$ROOT/dist:/dist:ro" \
        "fc-linux-rpm:$native" bash /src/linux/scripts/package-rpm.sh --rpmlint-inside "$arch"

    if [ "$check" = 1 ]; then
        if [ "${FC_PACKAGE_SKIP_IMAGE:-0}" != 1 ]; then
            base_image "$BASE" "$arch" "fc-linux-fedora:$arch"
        fi
        docker run --rm --platform "linux/$arch" \
            -v "$export_dir:/src:ro" \
            -v "$ROOT/dist:/dist:ro" \
            "fc-linux-fedora:$arch" bash /src/linux/scripts/package-rpm.sh --check-inside "$arch"
    fi
done
rm -rf "$export_dir"
