#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "$0")" && pwd)"
CORE_DIR="$DIR/lumen-core"
SWIFT_DIR="$DIR/Lumen"
SCRIPTS_DIR="$DIR/scripts"

BUILD_MODE="debug"
TRANSPARENT=false
PASSIVE=false
VERBOSE=false
CORE_ARGS=()

usage() {
    cat <<EOF
Usage: $0 [options]

Options:
  --release         Build in release mode
  --verbose         Enable debug logging (sets RUST_LOG=lumen_core=debug)
  --transparent     Enable transparent capture (requires sudo for pf rules)
  --passive         Enable passive packet capture via BPF (requires sudo)
  --help            Show this help

Normal mode (default):
  Starts lumen-core as an HTTP proxy on :9090 and the Swift UI.
  No root required.

Transparent mode:
  Starts lumen-core with --transparent, sets up pf redirect rules for
  LLM API hosts, and tears them down on exit. Prompts for sudo once.

Passive mode:
  Starts lumen-core with --passive for BPF packet capture. Monitors
  traffic to known LLM API hosts by reading packet headers (no proxy,
  no TLS interception, no pf rules). Requires sudo for BPF access.
EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) BUILD_MODE="release"; shift ;;
        --verbose) VERBOSE=true; shift ;;
        --transparent) TRANSPARENT=true; shift ;;
        --passive) PASSIVE=true; shift ;;
        --help|-h) usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

# Kill any lingering lumen-core processes to avoid port conflicts
if pgrep -f lumen-core > /dev/null 2>&1; then
    echo "[lumen] Killing existing lumen-core processes..."
    pkill -f lumen-core 2>/dev/null || true
    sleep 1
fi

cleanup() {
    echo ""
    if [ -n "$CORE_PID" ] && kill -0 "$CORE_PID" 2>/dev/null; then
        echo "[lumen] Stopping core (pid $CORE_PID)..."
        kill "$CORE_PID" 2>/dev/null
        wait "$CORE_PID" 2>/dev/null
    fi
    if $TRANSPARENT; then
        echo "[lumen] Tearing down pf rules..."
        sudo pfctl -a com.datagrout.lumen -F all 2>/dev/null || true
    fi
    exit 0
}
trap cleanup INT TERM EXIT

echo "[lumen] Building core ($BUILD_MODE)..."
if [ "$BUILD_MODE" = "release" ]; then
    cargo build --release --manifest-path "$CORE_DIR/Cargo.toml"
    CORE_BIN="$CORE_DIR/target/release/lumen-core"
else
    cargo build --manifest-path "$CORE_DIR/Cargo.toml"
    CORE_BIN="$CORE_DIR/target/debug/lumen-core"
fi

echo "[lumen] Building Swift client..."
cd "$SWIFT_DIR"
swift build
SWIFT_BIN="$SWIFT_DIR/.build/debug/Lumen"
cd "$DIR"

NEEDS_SUDO=false

if $TRANSPARENT; then
    CORE_ARGS+=("--transparent")
    NEEDS_SUDO=true
fi

if $PASSIVE; then
    CORE_ARGS+=("--passive")
    NEEDS_SUDO=true
fi

if $VERBOSE; then
    export RUST_LOG=lumen_core=debug
fi

if $NEEDS_SUDO; then
    if $TRANSPARENT; then
        echo "[lumen] Transparent + sudo mode — required for /dev/pf and pf rules"
    elif $PASSIVE; then
        echo "[lumen] Passive capture mode — sudo required for BPF access"
    fi
    echo ""

    # Validate sudo access up front (single password prompt)
    sudo -v

    echo "[lumen] Starting core with ${CORE_ARGS[*]} (as root)..."
    sudo "$CORE_BIN" "${CORE_ARGS[@]}" &
    CORE_PID=$!

    sleep 1
    if ! kill -0 "$CORE_PID" 2>/dev/null; then
        echo "[lumen] Core failed to start"
        exit 1
    fi
    echo "[lumen] Core running as root (pid $CORE_PID)"

    if $TRANSPARENT; then
        echo "[lumen] Setting up pf redirect rules..."
        sudo "$SCRIPTS_DIR/pf_setup.sh" --local
    fi
else
    echo "[lumen] Starting core..."
    "$CORE_BIN" "${CORE_ARGS[@]}" &
    CORE_PID=$!

    sleep 1
    if ! kill -0 "$CORE_PID" 2>/dev/null; then
        echo "[lumen] Core failed to start"
        exit 1
    fi
    echo "[lumen] Core running (pid $CORE_PID)"
fi

echo "[lumen] Starting Lumen client..."
"$SWIFT_BIN"
