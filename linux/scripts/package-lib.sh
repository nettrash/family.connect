# shellcheck shell=bash
# Family Connect for Linux — what scripts/package.sh (.deb, .tar.gz) and scripts/package-rpm.sh
# (.rpm) share, adapted from md.linux's. Sourced, on the host and inside the containers; it only
# defines functions and constants. The callers define `say` and `die`, and check_gst_elements'
# callers `ok` and `fail`.

# What a package is built from, as paths of the repository: the Rust workspace, the shared text
# crate it reads by path (`fc-text = { path = "../web/text" }`), and the family's CHANGELOG, which
# the packages carry as documentation. linux/tools/rtc-spike (the phase-0 call spike, not in the
# workspace) is left out; linux/tools/call-interop stays, because it IS a workspace member and cargo
# will not load a workspace with a member missing (it is never built for a package: `-p
# family-connect`). A file outside these that the build starts to read must be added here — and to
# the `linux` path filter of .github/workflows/ci.yml.
SOURCE_PATHS=(linux web/text CHANGELOG ':(exclude)linux/tools/rtc-spike')

# The workspace's version: [workspace.package] in $1/Cargo.toml.
tree_version() {
    sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' "$1/Cargo.toml"
}

# Step 2 of both scripts: SOURCE_PATHS of the checkout $1 from git into the empty folder $2 — HEAD
# itself when those paths are clean, else ($3 = 1, --allow-dirty) their tracked and
# untracked-but-not-ignored files — so nothing stray (a .DS_Store, an old dist/) can reach a
# package. Changes elsewhere in the repository (ios/, server/, …) do not make the tree dirty: they
# cannot change a Linux package.
export_source() {
    local root="$1" dest="$2" allow_dirty="$3"
    if [ -z "$(git -C "$root" status --porcelain -- "${SOURCE_PATHS[@]}")" ]; then
        git -C "$root" archive --format=tar HEAD -- "${SOURCE_PATHS[@]}" | tar -x -C "$dest"
        say "source: HEAD $(git -C "$root" rev-parse --short HEAD), SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH"
    elif [ "$allow_dirty" = 1 ]; then
        (cd "$root" && git ls-files -z --cached --others --exclude-standard -- "${SOURCE_PATHS[@]}" |
            while IFS= read -r -d '' f; do [ -e "$f" ] && printf '%s\0' "$f"; done |
            tar --null -T - -cf -) | tar -x -C "$dest"
        say "source: the working tree with uncommitted changes (--allow-dirty), SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH of HEAD"
    else
        die "linux/, web/text/ or CHANGELOG has uncommitted changes: commit them, or pass --allow-dirty"
    fi
    [ -f "$dest/linux/Cargo.toml" ] && [ -f "$dest/web/text/Cargo.toml" ] ||
        die "the exported source has no linux/Cargo.toml or web/text/Cargo.toml"
}

# The release build's flags: the containers' paths in the binary wherever it is built — source
# root $1 (the folder holding linux/ and web/), cargo home $2, target directory $3. Encoded, one
# per \x1f, so a path with a space survives, and replacing any RUSTFLAGS or build.rustflags of the
# machine's.
export_rustflags() {
    local sep=$'\x1f'
    export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$1=/build/family-connect$sep--remap-path-prefix=$2=/cargo$sep--remap-path-prefix=$3=/build/target"
}

# Step 4's fresh system: the pinned base image $1 (name@sha256:<index>) for linux/$2, tagged $3,
# made by BuildKit as the build images are. `docker run` of the digest itself works only once per
# digest in Docker's classic image store (GitHub's runners): the store keeps one image per digest
# reference, so the same digest for a second architecture fails with "cannot overwrite digest". A
# tag per architecture has no such limit.
base_image() {
    printf 'FROM %s\n' "$1" | docker buildx build --quiet --platform "linux/$2" --load -t "$3" - >/dev/null
}

# After installing the package: every GStreamer element named in the arguments exists, which is
# what the plugin packages in Depends / Requires are there for (docs/packaging.md §8.1). Needs
# gst-inspect-1.0; its registry goes to a scratch cache, not the user's.
check_gst_elements() {
    local element missing=""
    export XDG_CACHE_HOME=/tmp/fc-gst-cache
    for element in "$@"; do
        gst-inspect-1.0 --exists "$element" || missing="$missing $element"
    done
    if [ -z "$missing" ]; then ok "GStreamer elements present: $*"
    else fail "GStreamer elements missing:$missing"; fi
}
