#!/usr/bin/env bash
# Cross-compile the lumen-core daemon for Windows and drop the .exe in dist/,
# alongside the macOS DMG produced by build_dmg.sh.
#
# Usage:
#   ./scripts/build_exe.sh
#
# Output:
#   dist/lumen-core-<version>-x86_64-windows.exe
#
# Why just the daemon (no app):
#   The macOS build ships a Swift menu-bar app; Windows has no such app. The
#   lumen-core daemon is pure Rust and self-sufficient — Windows users run the
#   .exe and open http://127.0.0.1:9091/dashboard for the same live gauges,
#   event feed, and lap history. So the Windows artifact is the single binary.
#
# Toolchain:
#   Cross-compiles from macOS via the MinGW GNU toolchain (matches the README's
#   documented path). One-time setup:
#     brew install mingw-w64
#   The Rust target and default `ring` crypto backend cross-compile cleanly with
#   no extra C dependencies.
#
# Sharing unsigned:
#   dist/ is git-ignored — share the .exe via Slack, Drive, etc. The binary is
#   unsigned, so on first run Windows SmartScreen shows "Windows protected your
#   PC". Tell recipients: click "More info" -> "Run anyway".
#
# Overrides:
#   WINDOWS_TARGET   Rust target triple      (default x86_64-pc-windows-gnu)
#   MINGW_CC         cross C compiler/linker (default x86_64-w64-mingw32-gcc)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Read version from Cargo.toml (same source of truth as build_dmg.sh).
VERSION=$(grep '^version' "$PROJECT_ROOT/lumen-core/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')

TARGET="${WINDOWS_TARGET:-x86_64-pc-windows-gnu}"
MINGW_CC="${MINGW_CC:-x86_64-w64-mingw32-gcc}"
DIST_DIR="$PROJECT_ROOT/dist"

# Arch label for the filename (first field of the triple): x86_64, aarch64, …
ARCH_LABEL="${TARGET%%-*}"
EXE_NAME="lumen-core-${VERSION}-${ARCH_LABEL}-windows.exe"

echo "▸ Building lumen-core v${VERSION} for Windows ($TARGET)"

# 1. Preflight — GNU targets need the MinGW cross compiler/linker on PATH.
if [[ "$TARGET" == *-gnu* ]] && ! command -v "$MINGW_CC" >/dev/null 2>&1; then
    echo "❌ MinGW cross compiler '$MINGW_CC' not found on PATH." >&2
    echo "   Install it once with:  brew install mingw-w64" >&2
    exit 1
fi

# Ensure the Rust target is installed (no-op if already present).
rustup target add "$TARGET" >/dev/null 2>&1 || true

# 2. Wire the cross compiler for cargo/cc. The env-var names are derived from
#    the triple: CC_<triple-with-underscores> and
#    CARGO_TARGET_<TRIPLE_UPPER>_LINKER.
CC_VAR="CC_$(echo "$TARGET" | tr '-' '_')"
LINKER_VAR="CARGO_TARGET_$(echo "$TARGET" | tr '[:lower:]-' '[:upper:]_')_LINKER"

echo "▸ Compiling (release, ring crypto backend)..."
cd "$PROJECT_ROOT/lumen-core"
env "${CC_VAR}=$MINGW_CC" "${LINKER_VAR}=$MINGW_CC" \
    cargo build --release --target "$TARGET"

BUILT_EXE="$PROJECT_ROOT/lumen-core/target/$TARGET/release/lumen-core.exe"
if [ ! -f "$BUILT_EXE" ]; then
    echo "❌ Expected binary not found at $BUILT_EXE" >&2
    exit 1
fi

# 3. Publish into dist/ next to the DMG.
mkdir -p "$DIST_DIR"
cp "$BUILT_EXE" "$DIST_DIR/$EXE_NAME"

SIZE=$(du -h "$DIST_DIR/$EXE_NAME" | cut -f1 | tr -d ' ')

echo ""
echo "✓ dist/${EXE_NAME} (${SIZE})"
echo ""
echo "Next:            ./scripts/bundle_windows.sh   (package the shareable zip)"
echo "Run on Windows:  double-click the .exe (or: Start-Process .\\${EXE_NAME})"
echo "Dashboard:       http://127.0.0.1:9091/dashboard"
echo "Note:            Unsigned — first run, SmartScreen: 'More info' -> 'Run anyway'"
