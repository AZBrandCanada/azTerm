#!/usr/bin/env bash
set -euo pipefail

VERSION="0.5.4"
TAG="v${VERSION}"
REPO="AZBrandCanada/azTerm"

# --- sanity checks -----------------------------------------------------------
if [ ! -f Cargo.toml ]; then
  echo "error: run this from the project root (Cargo.toml not found)"; exit 1
fi

CARGO_VERSION=$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')
if [ "$CARGO_VERSION" != "$VERSION" ]; then
  echo "error: Cargo.toml says $CARGO_VERSION, script expects $VERSION"
  echo "       edit the script or bump Cargo.toml"; exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
  echo "Committing working tree changes..."
  git add -A
  git commit -m "Release ${TAG}"
fi

if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "error: tag $TAG already exists locally. Delete it first:"
  echo "       git tag -d $TAG && git push origin :refs/tags/$TAG"
  exit 1
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "warning: 'gh' CLI not found — you'll need to create the release manually."
fi

# --- build -------------------------------------------------------------------
echo ">>> cargo build --release"
cargo build --release

DEB_ASSET=""
if command -v cargo-deb >/dev/null 2>&1; then
  echo ">>> cargo deb --no-build"
  cargo deb --no-build || echo "(cargo-deb failed; skipping .deb)"
  DEB_ASSET=$(ls target/debian/*.deb 2>/dev/null | head -1 || true)
fi

# --- tag & push --------------------------------------------------------------
echo ">>> tagging $TAG"
git tag -a "$TAG" -m "AZTerm ${TAG}"
echo ">>> pushing branch and tag"
git push origin HEAD
git push origin "$TAG"

# --- github release ----------------------------------------------------------
if command -v gh >/dev/null 2>&1; then
  ASSETS=("target/release/azterm")
  [ -n "$DEB_ASSET" ] && ASSETS+=("$DEB_ASSET")

  NOTES=$(cat <<'NOTE'
## AZTerm v0.5.2

### Fixed
- **Highlight on select wasnt ratio'd properly

Full changelog: https://github.com/AZBrandCanada/azTerm/compare/v0.5.1...v0.5.2
NOTE
)

  echo ">>> creating GitHub release $TAG"
  gh release create "$TAG" \
    --repo "$REPO" \
    --title "AZTerm ${TAG}" \
    --notes "$NOTES" \
    "${ASSETS[@]}"

  echo
  echo "Done. Release published:"
  gh release view "$TAG" --repo "$REPO" --json url -q .url
else
  echo
  echo "Tag pushed. Create the release here:"
  echo "  https://github.com/${REPO}/releases/new?tag=${TAG}"
fi
 