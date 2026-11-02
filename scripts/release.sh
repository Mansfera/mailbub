#!/bin/sh
# Builds a release DMG and the matching Homebrew cask.
#
#   scripts/release.sh            build into dist/release/ (nothing is uploaded)
#   scripts/release.sh --publish  also create the GitHub release and update the tap
#
# RELEASE_REPO  where the DMG is downloaded from (default Mansfera/mailbub)
# TAP_REPO      Homebrew tap repository (default Mansfera/homebrew-tap)
set -eu

cd "$(dirname "$0")/.."
RELEASE_REPO="${RELEASE_REPO:-Mansfera/mailbub}"
TAP_REPO="${TAP_REPO:-Mansfera/homebrew-tap}"
VERSION=$(sed -n 's/^ *"version": *"\([^"]*\)".*/\1/p' src-tauri/tauri.conf.json | head -1)
DMG_NAME="Mailbub_${VERSION}_aarch64.dmg"
OUT=dist/release

# Sign with the local "Mailbub Code Signing" identity when it exists, else ad-hoc.
CONFIG=""
if ! security find-identity -p codesigning 2>/dev/null | grep -q '"Mailbub Code Signing"'; then
  echo "release: 'Mailbub Code Signing' not found, signing ad-hoc" >&2
  CONFIG='{"bundle":{"macOS":{"signingIdentity":"-"}}}'
fi

if [ -n "$CONFIG" ]; then
  bun tauri build --bundles dmg --config "$CONFIG"
else
  bun tauri build --bundles dmg
fi

mkdir -p "$OUT"
cp "src-tauri/target/release/bundle/dmg/$DMG_NAME" "$OUT/$DMG_NAME"
SHA=$(shasum -a 256 "$OUT/$DMG_NAME" | cut -d' ' -f1)
sed -e "s|@VERSION@|$VERSION|g" -e "s|@SHA256@|$SHA|g" -e "s|@RELEASE_REPO@|$RELEASE_REPO|g" \
  packaging/homebrew/mailbub.rb.in > "$OUT/mailbub.rb"
echo "release: $OUT/$DMG_NAME ($SHA)"
echo "release: $OUT/mailbub.rb"

if [ "${1:-}" != "--publish" ]; then
  echo "release: not published (run with --publish to upload)"
  exit 0
fi

gh release create "v$VERSION" "$OUT/$DMG_NAME" --repo "$RELEASE_REPO" \
  --title "Mailbub $VERSION" --notes "Install: brew trust ${TAP_REPO%%/*}/tap && brew install --cask ${TAP_REPO%%/*}/tap/mailbub"

# Update Casks/mailbub.rb in the tap through the GitHub API (no local clone or commit).
CASK_PATH=Casks/mailbub.rb
CASK_SHA=$(gh api "repos/$TAP_REPO/contents/$CASK_PATH" --jq .sha 2>/dev/null || true)
gh api -X PUT "repos/$TAP_REPO/contents/$CASK_PATH" \
  -f message="mailbub $VERSION" \
  -f content="$(base64 < "$OUT/mailbub.rb" | tr -d '\n')" \
  ${CASK_SHA:+-f sha="$CASK_SHA"} > /dev/null
echo "release: published v$VERSION to $RELEASE_REPO and $TAP_REPO"
