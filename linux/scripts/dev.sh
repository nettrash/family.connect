#!/usr/bin/env bash
# Family Connect for Linux — build and test in the Ubuntu 26.04 dev image (adapted from md.linux's).
#
#   scripts/dev.sh image             build the image (docker/Dockerfile) as fc-linux-dev
#   scripts/dev.sh test              fmt --check; clippy -D warnings on the workspace, on
#                                    family-connect and on family-connect --features selftest;
#                                    cargo test (the workspace, and the app with the self-test's own
#                                    unit tests) — on Ubuntu's rustc
#   scripts/dev.sh clippy            the three clippy runs only
#   scripts/dev.sh fmt               cargo fmt --all
#   scripts/dev.sh build [args]      cargo build -p family-connect (debug unless --release is passed)
#   scripts/dev.sh run [app args]    run family-connect under Xvfb and a D-Bus session; a screenshot
#                                    of the screen after FC_SHOT_AFTER seconds (default 6):
#                                    linux/dist/run.png
#   scripts/dev.sh selftest [args]   family-connect --selftest OUT/selftest [args] under Xvfb, a
#                                    private session bus and an unlocked gnome-keyring, built with
#                                    the `selftest` feature, its profile in OUT/selftest/home (wiped
#                                    first); args: --only group,… and --no-input (shell-design §27).
#                                    Exits with the run's verdict; report.json and screenshots/ in
#                                    linux/dist/selftest
#   scripts/dev.sh package [args]    the .deb and the .tar.gz — delegates to scripts/package.sh (on
#                                    the host); with FC_DEV_NATIVE=1, to scripts/package.sh --native.
#                                    The .rpm is scripts/package-rpm.sh (docs/packaging.md)
#   scripts/dev.sh shell             an interactive shell in the image
#
# On the host every subcommand but `image` and `package` runs inside Docker: the WHOLE repository
# is mounted at /repo (the workspace reads ../web/text), linux/dist at /out, the cargo registry and
# the target directory are named volumes (fc-linux-cargo, fc-linux-target-<checkout name>) so
# rebuilds are incremental. On a Linux machine with the -dev packages installed, FC_DEV_NATIVE=1
# runs the same subcommands directly.
#
# Passed into the container when set on the host: FC_SHOT_AFTER, GDK_SCALE (HiDPI under Xvfb) and
# ADW_DEBUG_COLOR_SCHEME=prefer-dark (libadwaita's system preference). Experiment switches for the
# headless run: FC_WEBKIT_SANDBOX=1 keeps WebKit's bubblewrap sandbox (default: disabled, as Docker's
# seccomp profile denies bwrap its user namespace — never outside Docker, shell-design §9.5);
# FC_NO_DBUS=1 runs without a session bus (every launch its own primary).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="$(cd "$ROOT/.." && pwd)"
IMAGE="${FC_IMAGE:-fc-linux-dev}"
OUT_HOST="$ROOT/dist"

# Xvfb on :99 plus a private session bus (GApplication's single instance is a D-Bus name), then
# the command in the arguments.
headless() {
    local display=":99"
    Xvfb "$display" -screen 0 1280x800x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
    local xvfb=$!
    for _ in $(seq 50); do [ -e /tmp/.X11-unix/X99 ] && break; sleep 0.1; done
    export DISPLAY="$display"
    export GDK_BACKEND=x11
    # Docker only: its seccomp profile denies bubblewrap a user namespace, so WebKit's sandbox
    # cannot start there (md.linux facts §9). The packaged app never sees this variable.
    if [ "${FC_WEBKIT_SANDBOX:-0}" != 1 ]; then
        export WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1
    fi
    # Cosmetic: Xvfb has no DRI3, and without this Mesa's EGL prints two warnings per process.
    # Rendering is identical either way (llvmpipe).
    export LIBGL_ALWAYS_SOFTWARE=1
    local status=0
    if [ "${FC_NO_DBUS:-0}" = 1 ]; then
        env -u DBUS_SESSION_BUS_ADDRESS "$@" || status=$?
    else
        dbus-run-session -- "$@" || status=$?
    fi
    kill "$xvfb" 2>/dev/null || true
    wait "$xvfb" 2>/dev/null || true
    return "$status"
}

# The Secret Service for the token's self-test (P5): a keyring daemon on the private session bus,
# unlocked with an empty password, then the command. The newline ends that empty password: with
# nothing at all on stdin the daemon creates no login keyring, and every store then waits on a
# prompt (measured, P5).
with_keyring() {
    if command -v gnome-keyring-daemon >/dev/null 2>&1; then
        eval "$(printf '\n' | gnome-keyring-daemon --unlock --components=secrets 2>/dev/null)" || true
        export GNOME_KEYRING_CONTROL SSH_AUTH_SOCK
    fi
    "$@"
}

clippy_all() {
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy -p family-connect --all-targets -- -D warnings
    cargo clippy -p family-connect --all-targets --features selftest -- -D warnings
}

inner() {
    local cmd="${1:-}"
    shift || true
    case "$cmd" in
        test)
            cargo fmt --all -- --check
            clippy_all
            # The app's unit tests use a main context and libsoup on loopback; no display.
            cargo test --workspace
            cargo test -p family-connect --features selftest
            ;;
        clippy)
            clippy_all
            ;;
        fmt)
            cargo fmt --all
            ;;
        build)
            cargo build -p family-connect "$@"
            ;;
        run)
            cargo build -p family-connect
            local bin="${CARGO_TARGET_DIR:-$ROOT/target}/debug/family-connect"
            local after="${FC_SHOT_AFTER:-6}"
            # shellcheck disable=SC2016 # the inner script expands its own arguments and $OUT
            headless bash -c '
                "$1" "${@:3}" &
                app=$!
                sleep "$2"
                import -window root "$OUT/run.png"
                echo "screenshot: $OUT/run.png"
                kill "$app" 2>/dev/null || true
                wait "$app" 2>/dev/null || true
            ' _ "$bin" "$after" "$@"
            ;;
        selftest)
            cargo build -p family-connect --features selftest
            local bin="${CARGO_TARGET_DIR:-$ROOT/target}/debug/family-connect"
            local out="$OUT/selftest"
            # A fresh profile and no stale report: a run never inherits another's state.
            rm -rf "$out"
            mkdir -p "$out/home"
            # The app refuses to self-test unless all four point into its output folder.
            export XDG_CONFIG_HOME="$out/home/.config"
            export XDG_STATE_HOME="$out/home/.local/state"
            export XDG_DATA_HOME="$out/home/.local/share"
            export XDG_CACHE_HOME="$out/home/.cache"
            # The data folder of the checkout (linux/data): the binary lives in /target.
            export FC_DATA_DIR="$ROOT/data"
            headless bash -c "$(declare -f with_keyring); with_keyring \"\$@\"" _ \
                "$bin" --selftest "$out" "$@"
            ;;
        package)
            exec "$ROOT/scripts/package.sh" --native "$@"
            ;;
        shell)
            exec bash
            ;;
        *)
            echo "usage: scripts/dev.sh {image|test|clippy|fmt|build|run|selftest|package|shell}" >&2
            exit 2
            ;;
    esac
}

if [ "${FC_DEV_NATIVE:-0}" = 1 ] || [ -f /.dockerenv ]; then
    if [ -f /.dockerenv ]; then OUT=/out; else OUT="$OUT_HOST"; fi
    export OUT
    mkdir -p "$OUT"
    cd "$ROOT"
    inner "$@"
    exit
fi

if [ "${1:-}" = image ]; then
    exec docker build -t "$IMAGE" -f "$ROOT/docker/Dockerfile" "$ROOT/docker"
fi

if [ "${1:-}" = package ]; then
    shift
    exec "$ROOT/scripts/package.sh" "$@"
fi

mkdir -p "$OUT_HOST"
tty_flags=(-i)
[ -t 0 ] && [ -t 1 ] && tty_flags=(-it)
exec docker run --rm "${tty_flags[@]}" \
    --init \
    --shm-size=1g \
    -v "$REPO:/repo" \
    -v "$OUT_HOST:/out" \
    -v fc-linux-cargo:/cargo \
    -v "fc-linux-target-$(basename "$REPO"):/target" \
    -e FC_SHOT_AFTER -e FC_WEBKIT_SANDBOX -e FC_NO_DBUS -e GDK_SCALE -e ADW_DEBUG_COLOR_SCHEME \
    -w /repo/linux \
    "$IMAGE" /repo/linux/scripts/dev.sh "$@"
