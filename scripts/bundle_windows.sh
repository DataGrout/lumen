#!/usr/bin/env bash
# Assemble the shareable Windows release zip from the tracked bundle files
# (packaging/windows/) plus the daemon .exe produced by build_exe.sh.
#
# This replaces the previous hand-assembly of the zip — the step that shipped a
# BOM-less UTF-8 lumen.ps1 in 0.2.1 (PowerShell 5.1 mis-parsed it) and let the
# zip drift out of sync with the tracked launcher. Everything that ships is now
# copied from version control, and a guard rejects a non-ASCII launcher.
#
# Usage:
#   ./scripts/build_exe.sh          # 1. build the daemon .exe
#   ./scripts/bundle_windows.sh     # 2. package the zip (this script)
#
#   ./scripts/bundle_windows.sh <version>   # override the version (default: Cargo.toml)
#
# Output:
#   dist/lumen-windows-<version>/        (staging dir mirroring packaging/windows/ + the .exe)
#   dist/lumen-windows-<version>.zip     (the artifact to share)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BUNDLE_SRC="$PROJECT_ROOT/packaging/windows"
DIST_DIR="$PROJECT_ROOT/dist"

# Version: CLI arg wins, else Cargo.toml (same source of truth as build_exe.sh).
if [ "${1:-}" != "" ]; then
    VERSION="$1"
else
    VERSION=$(grep '^version' "$PROJECT_ROOT/lumen-core/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
fi

# Arch label matches build_exe.sh's EXE_NAME (first field of the target triple).
ARCH_LABEL="${WINDOWS_ARCH:-x86_64}"
EXE_NAME="lumen-core-${VERSION}-${ARCH_LABEL}-windows.exe"
EXE_PATH="$DIST_DIR/$EXE_NAME"
STAGE="$DIST_DIR/lumen-windows-${VERSION}"
ZIP_NAME="lumen-windows-${VERSION}.zip"

echo "▸ Bundling Lumen for Windows v${VERSION}"

# 1. Preconditions — the .exe must already be built.
if [ ! -f "$EXE_PATH" ]; then
    echo "❌ $EXE_NAME not found in dist/ — run ./scripts/build_exe.sh first." >&2
    exit 1
fi
for f in lumen.ps1 run.bat README.txt; do
    [ -f "$BUNDLE_SRC/$f" ] || { echo "❌ missing bundle file $BUNDLE_SRC/$f" >&2; exit 1; }
done

# 2. ASCII guard on the launcher. Windows PowerShell 5.1 reads a BOM-less .ps1
#    as the system ANSI code page, so any non-ASCII byte corrupts parsing. Keep
#    lumen.ps1 pure ASCII. (This is the exact 0.2.1 failure, now caught here.)
if ! LC_ALL=C perl -ne 'exit 1 if /[^\x00-\x7F]/' "$BUNDLE_SRC/lumen.ps1"; then
    echo "❌ packaging/windows/lumen.ps1 contains non-ASCII bytes." >&2
    echo "   Windows PowerShell 5.1 will fail to parse it. Keep it pure ASCII." >&2
    exit 1
fi

# 3. Stage the bundle contents (everything from version control + the .exe).
rm -rf "$STAGE" "$DIST_DIR/$ZIP_NAME"
mkdir -p "$STAGE"
cp "$BUNDLE_SRC/lumen.ps1" "$BUNDLE_SRC/run.bat" "$BUNDLE_SRC/README.txt" "$STAGE/"
cp "$EXE_PATH" "$STAGE/"

# 4. Zip it (deterministic-ish: no extra attrs, no macOS cruft).
( cd "$DIST_DIR" && zip -r -X "$ZIP_NAME" "lumen-windows-${VERSION}" -x '*.DS_Store' >/dev/null )

echo ""
echo "✓ dist/${ZIP_NAME}"
( cd "$DIST_DIR" && unzip -l "$ZIP_NAME" )
