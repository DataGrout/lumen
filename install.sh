#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "$0")" && pwd)"
CORE_DIR="$DIR/lumen-core"
SWIFT_DIR="$DIR/Lumen"
APP_NAME="Lumen.app"
INSTALL_DIR="$HOME/Applications"
APP_PATH="$INSTALL_DIR/$APP_NAME"

echo "[install] Building lumen-core (release)..."
cargo build --release --manifest-path "$CORE_DIR/Cargo.toml"
CORE_BIN="$CORE_DIR/target/release/lumen-core"

echo "[install] Building Lumen Swift binary (release)..."
cd "$SWIFT_DIR"
swift build -c release 2>&1
SWIFT_BIN="$SWIFT_DIR/.build/release/Lumen"
cd "$DIR"

echo "[install] Stopping any running lumen-core instances..."
pkill -f lumen-core 2>/dev/null || true

echo "[install] Assembling $APP_NAME bundle..."

# Remove old bundle if present
rm -rf "$APP_PATH"

CONTENTS="$APP_PATH/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"

mkdir -p "$MACOS"
mkdir -p "$RESOURCES"

# Executables
cp "$SWIFT_BIN"  "$MACOS/Lumen"
cp "$CORE_BIN"   "$MACOS/lumen-core"

# Info.plist
cp "$SWIFT_DIR/Sources/Info.plist" "$CONTENTS/Info.plist"

# App icon
ICNS="$SWIFT_DIR/Resources/AppIcon.icns"
if [ -f "$ICNS" ]; then
    cp "$ICNS" "$RESOURCES/AppIcon.icns"
fi

# Mark as app bundle
touch "$APP_PATH/Contents/PkgInfo"
printf "APPL????" > "$APP_PATH/Contents/PkgInfo"

echo "[install] Installing to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"

echo "[install] Removing quarantine attributes..."
xattr -cr "$APP_PATH" 2>/dev/null || true

echo "[install] Forcing Spotlight reindex..."
mdimport "$APP_PATH" 2>/dev/null || true

echo ""
echo "Done! Lumen.app installed to $INSTALL_DIR"
echo ""
echo "  Open via Spotlight: search 'Lumen'"
echo "  Pin to Dock:        drag $APP_PATH to your Dock in Finder,"
echo "                      or right-click the Spotlight result → 'Open' then"
echo "                      right-click the status-bar icon isn't available,"
echo "                      but you can open Finder → ~/Applications → drag to Dock."
echo ""
echo "  To launch now:  open '$APP_PATH'"
