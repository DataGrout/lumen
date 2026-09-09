#!/usr/bin/env bash
set -euo pipefail

# Usage: ./tag.sh <version>
# Brings every version surface to <version>, runs tests, then tags and pushes.
#
# There are three surfaces, and for a long time this script owned only the
# first — which is why shipped 0.2.2 and 0.2.3 builds both reported 0.2.1 in
# the app, and why released work stayed filed under "Unreleased" in the
# changelog:
#
#   1. lumen-core/Cargo.toml          — the daemon's version (/health, sync)
#   2. Lumen/Sources/Info.plist       — what the app itself reports
#   3. CHANGELOG.md                   — the [Unreleased] heading
#
# Re-running for a version the tree is already at is fine: each step is a
# no-op and the bump commit is skipped rather than failing on an empty commit.

VERSION="${1:-}"

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 <version>  (e.g. 0.1.2)" >&2
    exit 1
fi

# Require semver-ish: digits and dots only (e.g. 1.2.3 or 1.2.3-rc.1)
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][a-zA-Z0-9.]+)?$ ]]; then
    echo "error: version must be semver (e.g. 1.2.3 or 1.2.3-rc.1)" >&2
    exit 1
fi

TAG="v${VERSION}"
CARGO_TOML="lumen-core/Cargo.toml"
INFO_PLIST="Lumen/Sources/Info.plist"
CHANGELOG="CHANGELOG.md"
RELEASE_DATE=$(date +%Y-%m-%d)

# -- Sanity checks -------------------------------------------------------------

if ! git diff-index --quiet HEAD --; then
    echo "error: uncommitted changes present -- commit or stash before tagging" >&2
    exit 1
fi

if git rev-parse "$TAG" &>/dev/null; then
    echo "error: tag $TAG already exists" >&2
    exit 1
fi

# Everything below is restored by `git checkout --` if the tests fail, which is
# only safe because the tree is known clean at this point.
TOUCHED=("$CARGO_TOML" "$INFO_PLIST" "$CHANGELOG" "lumen-core/Cargo.lock")

# -- 1. Daemon version ---------------------------------------------------------

CURRENT=$(grep '^version' "$CARGO_TOML" | head -1 | sed 's/version = "\(.*\)"/\1/')
if [[ "$CURRENT" == "$VERSION" ]]; then
    echo "$CARGO_TOML already at $VERSION"
else
    echo "Bumping $CARGO_TOML: $CURRENT -> $VERSION"
    sed -i '' "s/^version = \"$CURRENT\"/version = \"$VERSION\"/" "$CARGO_TOML"
fi

# Regenerate Cargo.lock so it reflects the new version
(cd lumen-core && cargo generate-lockfile 2>/dev/null) || true

# -- 2. App version ------------------------------------------------------------
#
# The value sits on the line *after* its <key>, hence `n` before substituting.
# CFBundleShortVersionString is the human version; CFBundleVersion is a build
# counter that must increase for each build macOS sees.

PLIST_CURRENT=$(sed -n "/<key>CFBundleShortVersionString<\/key>/{n;s|.*<string>\(.*\)</string>.*|\1|p;}" "$INFO_PLIST")
if [[ "$PLIST_CURRENT" == "$VERSION" ]]; then
    echo "$INFO_PLIST already at $VERSION"
else
    echo "Bumping $INFO_PLIST: ${PLIST_CURRENT:-unset} -> $VERSION"
    sed -i '' "/<key>CFBundleShortVersionString<\/key>/{n;s|<string>.*</string>|<string>$VERSION</string>|;}" "$INFO_PLIST"

    BUILD_CURRENT=$(sed -n "/<key>CFBundleVersion<\/key>/{n;s|.*<string>\(.*\)</string>.*|\1|p;}" "$INFO_PLIST")
    if [[ "$BUILD_CURRENT" =~ ^[0-9]+$ ]]; then
        BUILD_NEXT=$((BUILD_CURRENT + 1))
        echo "Bumping CFBundleVersion: $BUILD_CURRENT -> $BUILD_NEXT"
        sed -i '' "/<key>CFBundleVersion<\/key>/{n;s|<string>.*</string>|<string>$BUILD_NEXT</string>|;}" "$INFO_PLIST"
    else
        echo "warning: CFBundleVersion is not a plain integer -- left alone" >&2
    fi
fi

# -- 3. Changelog --------------------------------------------------------------

if grep -q '^## \[Unreleased\]' "$CHANGELOG"; then
    echo "Closing [Unreleased] as [$VERSION] — $RELEASE_DATE"
    sed -i '' "s|^## \[Unreleased\]|## [$VERSION] — $RELEASE_DATE|" "$CHANGELOG"
else
    echo "warning: no [Unreleased] section in $CHANGELOG -- nothing to close" >&2
fi

# -- Run tests -----------------------------------------------------------------

echo ""
echo "Running tests..."
if ! (cd lumen-core && cargo test --all 2>&1); then
    echo ""
    echo "error: tests failed -- reverting version changes" >&2
    git checkout -- "${TOUCHED[@]}" 2>/dev/null || true
    exit 1
fi

# -- Commit the version bump ---------------------------------------------------

echo ""
git add "${TOUCHED[@]}"
if git diff --cached --quiet; then
    echo "No version changes to commit -- tree was already at $VERSION."
else
    echo "Committing version bump..."
    git commit -m "chore: bump version to $VERSION"
fi

# -- Tag and push --------------------------------------------------------------

echo "Creating tag $TAG..."
git tag "$TAG"

echo "Pushing commit and tag..."
git push
git push origin "$TAG"

echo ""
echo "Released $TAG"
