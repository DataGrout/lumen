#!/usr/bin/env bash
# Build Lumen.app and package it as a DMG for distribution.
#
# Usage:
#   ./scripts/build_dmg.sh
#
# Output:
#   dist/Lumen-<version>.dmg
#
# Sharing unsigned:
#   The DMG is excluded from git (.gitignore). Share it via Slack, Google Drive,
#   or any file-sharing tool. Receivers double-click the DMG, drag Lumen to
#   Applications, and are done.
#
#   First launch note (unsigned builds):
#   macOS Gatekeeper will block the first launch because the app is not notarized.
#   Tell recipients: right-click -> Open (once) to bypass the warning, or go to
#   System Settings -> Privacy & Security -> "Open Anyway" after the first attempt.
#
# Notarized / signed builds:
#   SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)" ./scripts/build_dmg.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Read version from Cargo.toml
VERSION=$(grep '^version' "$PROJECT_ROOT/lumen-core/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')
APP_NAME="Lumen"
DMG_NAME="${APP_NAME}-${VERSION}.dmg"
DIST_DIR="$PROJECT_ROOT/dist"
BUILD_DIR="$DIST_DIR/build"
STAGING_DIR="$DIST_DIR/dmg_staging"
APP_BUNDLE="$BUILD_DIR/${APP_NAME}.app"

echo "▸ Building Lumen v${VERSION}"

# Architecture targets. Build universal (x86_64 + arm64) by default so the DMG
# runs on both Intel and Apple Silicon Macs. Override with:
#   ARCHS="arm64"          # Apple Silicon only (faster build)
#   ARCHS="x86_64 arm64"   # default -- universal
ARCHS="${ARCHS:-x86_64 arm64}"
echo "▸ Target architectures: $ARCHS"

# Map our arch names to Rust target triples.
rust_triple() {
    case "$1" in
        x86_64) echo "x86_64-apple-darwin" ;;
        arm64)  echo "aarch64-apple-darwin" ;;
        *) echo "unknown-arch-$1" ;;
    esac
}

# 1. Build Rust daemon for each arch, then lipo into a universal binary.
echo "▸ Compiling lumen-core (release)..."
cd "$PROJECT_ROOT/lumen-core"
RUST_BIN_PATHS=()
for arch in $ARCHS; do
    triple=$(rust_triple "$arch")
    # Ensure the toolchain target is installed (no-op if already present).
    rustup target add "$triple" >/dev/null 2>&1 || true
    echo "  • cargo build --target $triple"
    cargo build --release --target "$triple"
    RUST_BIN_PATHS+=("$PROJECT_ROOT/lumen-core/target/$triple/release/lumen-core")
done

DAEMON_BIN="$PROJECT_ROOT/lumen-core/target/universal-lumen-core"
if [ "${#RUST_BIN_PATHS[@]}" -gt 1 ]; then
    echo "  • lipo -> universal lumen-core"
    lipo -create -output "$DAEMON_BIN" "${RUST_BIN_PATHS[@]}"
else
    cp "${RUST_BIN_PATHS[0]}" "$DAEMON_BIN"
fi

# 2. Build Swift app. SwiftPM accepts multiple --arch flags and produces a
#    universal binary directly via Xcode's XCBuild backend.
echo "▸ Compiling Lumen.app (release)..."
cd "$PROJECT_ROOT/Lumen"
SWIFT_ARCH_ARGS=()
for arch in $ARCHS; do
    SWIFT_ARCH_ARGS+=(--arch "$arch")
done
# We deliberately do NOT pipe through grep here -- multi-arch builds emit
# "Build succeeded" (XCBuild) while single-arch builds emit "Build complete"
# (SwiftPM), so a regex filter would silently swallow failures from one of
# the two paths. Full output is fine for a release-time script.
swift build -c release "${SWIFT_ARCH_ARGS[@]}"

# Multi-arch builds land under .build/apple/Products/Release (XCBuild's layout);
# single-arch builds land at .build/release. Pick whichever exists.
if [ -f "$PROJECT_ROOT/Lumen/.build/apple/Products/Release/Lumen" ]; then
    APP_BIN="$PROJECT_ROOT/Lumen/.build/apple/Products/Release/Lumen"
elif [ -f "$PROJECT_ROOT/Lumen/.build/release/Lumen" ]; then
    APP_BIN="$PROJECT_ROOT/Lumen/.build/release/Lumen"
else
    echo "❌ Could not locate built Lumen binary under .build/" >&2
    exit 1
fi

# 3. Assemble .app bundle
echo "▸ Assembling ${APP_NAME}.app..."
rm -rf "$BUILD_DIR"
mkdir -p "$APP_BUNDLE/Contents/MacOS"
mkdir -p "$APP_BUNDLE/Contents/Resources"

cp "$APP_BIN"    "$APP_BUNDLE/Contents/MacOS/${APP_NAME}"
cp "$DAEMON_BIN" "$APP_BUNDLE/Contents/MacOS/lumen-core"
cp "$PROJECT_ROOT/Lumen/Sources/Info.plist" "$APP_BUNDLE/Contents/Info.plist"
printf 'APPL????' > "$APP_BUNDLE/Contents/PkgInfo"

# Sanity-check: log the architectures of the bundled binaries so a stray
# single-arch build doesn't ship to Intel users undetected.
echo "  • Lumen      -> $(lipo -archs "$APP_BUNDLE/Contents/MacOS/${APP_NAME}" 2>/dev/null || echo unknown)"
echo "  • lumen-core -> $(lipo -archs "$APP_BUNDLE/Contents/MacOS/lumen-core"   2>/dev/null || echo unknown)"

ICON_SRC="$PROJECT_ROOT/Lumen/Resources/AppIcon.icns"
if [ -f "$ICON_SRC" ]; then
    cp "$ICON_SRC" "$APP_BUNDLE/Contents/Resources/AppIcon.icns"
fi

# 4. Sign (ad-hoc unless SIGNING_IDENTITY is set)
if [ -n "${SIGNING_IDENTITY:-}" ]; then
    echo "▸ Signing with: $SIGNING_IDENTITY"
    codesign --deep --force --options runtime --sign "$SIGNING_IDENTITY" "$APP_BUNDLE"
else
    echo "▸ Ad-hoc signing (not notarized)..."
    codesign --deep --force --sign - "$APP_BUNDLE"
fi

# 5. Create DMG
echo "▸ Creating ${DMG_NAME}..."
rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR"
cp -R "$APP_BUNDLE" "$STAGING_DIR/"
ln -s /Applications "$STAGING_DIR/Applications"

mkdir -p "$DIST_DIR"
rm -f "$DIST_DIR/$DMG_NAME"
WRITABLE_DMG="$DIST_DIR/${APP_NAME}-rw.dmg"
rm -f "$WRITABLE_DMG"

# Create writable image first so we can set the volume icon
hdiutil create \
    -volname "$APP_NAME" \
    -srcfolder "$STAGING_DIR" \
    -ov \
    -format UDRW \
    "$WRITABLE_DMG" \
    > /dev/null

# Mount at a fixed path -- avoids parsing hdiutil's tab-separated output,
# which changed format on macOS 15+.
MOUNT_POINT="$DIST_DIR/.dmg_mount_$$"
mkdir -p "$MOUNT_POINT"
hdiutil attach "$WRITABLE_DMG" -readwrite -noverify -noautoopen \
    -mountpoint "$MOUNT_POINT" > /dev/null

if [ -f "$ICON_SRC" ]; then
    cp "$ICON_SRC" "$MOUNT_POINT/.VolumeIcon.icns"
    # SetFile ships with Xcode command-line tools; skip gracefully if absent
    SetFile -a C "$MOUNT_POINT" 2>/dev/null || true
fi

hdiutil detach "$MOUNT_POINT" -quiet
rmdir "$MOUNT_POINT"

# Convert to final compressed read-only image
hdiutil convert "$WRITABLE_DMG" -format UDZO -imagekey zlib-level=9 \
    -o "$DIST_DIR/$DMG_NAME" > /dev/null
rm -f "$WRITABLE_DMG"

# Cleanup staging (mount point already removed above; guard handles abort cases)
rm -rf "$BUILD_DIR" "$STAGING_DIR" "$MOUNT_POINT"

echo ""
echo "✓ dist/${DMG_NAME}"
echo ""
echo "Install: open dist/${DMG_NAME}, drag ${APP_NAME} -> Applications"
if [ -z "${SIGNING_IDENTITY:-}" ]; then
    echo "Note:    Ad-hoc signed -- recipients must right-click -> Open on first launch"
fi
