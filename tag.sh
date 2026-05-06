#!/usr/bin/env bash
set -euo pipefail

# Usage: ./tag.sh <version>
# Updates Cargo.toml, runs tests, then creates and pushes a git tag.

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

# -- Sanity checks -------------------------------------------------------------

if ! git diff-index --quiet HEAD --; then
    echo "error: uncommitted changes present -- commit or stash before tagging" >&2
    exit 1
fi

if git rev-parse "$TAG" &>/dev/null; then
    echo "error: tag $TAG already exists" >&2
    exit 1
fi

# -- Bump version in Cargo.toml ------------------------------------------------

CURRENT=$(grep '^version' "$CARGO_TOML" | head -1 | sed 's/version = "\(.*\)"/\1/')
echo "Bumping $CARGO_TOML: $CURRENT -> $VERSION"
sed -i '' "s/^version = \"$CURRENT\"/version = \"$VERSION\"/" "$CARGO_TOML"

# Regenerate Cargo.lock so it reflects the new version
(cd lumen-core && cargo generate-lockfile 2>/dev/null) || true

# -- Run tests -----------------------------------------------------------------

echo ""
echo "Running tests..."
if ! (cd lumen-core && cargo test --all 2>&1); then
    echo ""
    echo "error: tests failed -- reverting Cargo.toml" >&2
    sed -i '' "s/^version = \"$VERSION\"/version = \"$CURRENT\"/" "$CARGO_TOML"
    exit 1
fi

# -- Commit the version bump ---------------------------------------------------

echo ""
echo "Committing version bump..."
git add "$CARGO_TOML" lumen-core/Cargo.lock
git commit -m "chore: bump version to $VERSION"

# -- Tag and push --------------------------------------------------------------

echo "Creating tag $TAG..."
git tag "$TAG"

echo "Pushing commit and tag..."
git push
git push origin "$TAG"

echo ""
echo "Released $TAG"
