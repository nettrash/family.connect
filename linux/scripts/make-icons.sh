#!/usr/bin/env bash
# Family Connect for Linux — the app icon in the hicolor layout (docs/packaging.md §4), adapted from
# md.linux's. Run once by hand; the output under packaging/icons/ is committed, so the package
# build never needs ImageMagick.
#
#   scripts/make-icons.sh [source.png]
#
# The source is the Mac's 1024 px icon, ios/FamilyConnect/Assets.xcassets/AppIcon.appiconset/
# AppIcon-macOS-512x512@2x.png: the rounded square on a transparent margin that the Mac's Dock
# shows, which is also the shape GNOME draws app icons in. Every size is made from that one file —
# 512x512@2 is a byte copy of it, the other twelve are resized with ImageMagick 7 (`magick`, from
# the host if it has one, else from the fc-linux-dev image) with the date and time chunks left out,
# so running the script again gives the same bytes.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:-$ROOT/../ios/FamilyConnect/Assets.xcassets/AppIcon.appiconset/AppIcon-macOS-512x512@2x.png}"
OUT="$ROOT/packaging/icons/hicolor"
NAME="me.nettrash.FamilyConnect.png"
IMAGE="${FC_IMAGE:-fc-linux-dev}"

if [ ! -f "$SRC" ]; then
    echo "make-icons: no icon at $SRC (pass the 1024 px PNG)" >&2
    exit 2
fi
SRC_DIR="$(cd "$(dirname "$SRC")" && pwd)"
SRC_FILE="$(basename "$SRC")"

# magick ARGS…, with /in = the source's folder and /out = the hicolor folder.
magick_run() {
    if command -v magick >/dev/null 2>&1; then
        local args=() a
        for a in "$@"; do a="${a//\/in\//$SRC_DIR/}"; args+=("${a//\/out\//$OUT/}"); done
        magick "${args[@]}"
    else
        docker run --rm -v "$SRC_DIR:/in:ro" -v "$OUT:/out" "$IMAGE" magick "$@"
    fi
}

[ "$(magick_run identify -format '%wx%h' "/in/$SRC_FILE")" = 1024x1024 ] ||
    { echo "make-icons: $SRC is not 1024x1024" >&2; exit 2; }

# The hicolor directories of hicolor-icon-theme's index on Ubuntu 26.04 that an app icon uses;
# @2 is the same size at scale 2 (twice the pixels).
DIRS=(16x16 16x16@2 24x24 32x32 32x32@2 48x48 64x64 128x128 128x128@2 256x256 256x256@2 512x512 512x512@2)

pixels() {
    local dir="$1" base want
    base="${dir%@2}"
    want="${base%%x*}"
    [ "$dir" != "$base" ] && want=$((want * 2))
    printf '%s' "$want"
}

rm -rf "$OUT"
for dir in "${DIRS[@]}"; do
    px="$(pixels "$dir")"
    mkdir -p "$OUT/$dir/apps"
    if [ "$px" = 1024 ]; then
        cp "$SRC" "$OUT/$dir/apps/$NAME"
    else
        magick_run "/in/$SRC_FILE" -filter Lanczos -resize "${px}x${px}" -strip \
            -define png:exclude-chunks=date,time "/out/$dir/apps/$NAME"
    fi
    chmod 0644 "$OUT/$dir/apps/$NAME"
done

# Every file must be the size its directory names, and square.
status=0
for dir in "${DIRS[@]}"; do
    want="$(pixels "$dir")"
    got="$(magick_run identify -format '%wx%h' "/out/$dir/apps/$NAME")"
    if [ "$got" != "${want}x${want}" ]; then
        echo "make-icons: $dir/apps/$NAME is $got, expected ${want}x${want}" >&2
        status=1
    else
        echo "$dir/apps/$NAME  $got"
    fi
done
exit "$status"
