#!/usr/bin/env bash
# Usage: bundle-linux.sh <target-triple> <arch-label>
#   dist/panda-reader-<version>-linux-<arch>.tar.gz
set -euo pipefail

TARGET="$1"
ARCH="$2"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
VERSION="$(bash "$ROOT/.github/scripts/version.sh")"
NAME="panda-reader-${VERSION}-linux-${ARCH}"
STAGE="dist/${NAME}"

rm -rf "$STAGE"
mkdir -p "$STAGE"

cp "target/${TARGET}/release/panda-reader" "$STAGE/panda-reader"
cp "target/${TARGET}/release/panda-reader-updater" "$STAGE/panda-reader-updater"
chmod +x "$STAGE/panda-reader"
chmod +x "$STAGE/panda-reader-updater"
strip "$STAGE/panda-reader" || echo "strip unavailable — shipping unstripped binary"
cp LICENSE "$STAGE/LICENSE"
cp README.md "$STAGE/README.md"
cp THIRD_PARTY_NOTICES.md "$STAGE/THIRD_PARTY_NOTICES.md"
cp THIRD_PARTY_LICENSES.txt "$STAGE/THIRD_PARTY_LICENSES.txt"
mkdir -p "$STAGE/THIRD_PARTY_ASSET_LICENSES/fonts"
cp apps/panda-reader/assets/TTY7-LICENSE "$STAGE/THIRD_PARTY_ASSET_LICENSES/TTY7-LICENSE"
cp apps/panda-reader/assets/fonts/*LICENSE* "$STAGE/THIRD_PARTY_ASSET_LICENSES/fonts/"

mkdir -p dist
tar -C dist -czf "dist/${NAME}.tar.gz" "$NAME"
tar -tzf "dist/${NAME}.tar.gz" | grep -Fx "${NAME}/panda-reader" >/dev/null
tar -tzf "dist/${NAME}.tar.gz" | grep -Fx "${NAME}/panda-reader-updater" >/dev/null
test -s "dist/${NAME}.tar.gz"
rm -rf "$STAGE"
echo "OK dist/${NAME}.tar.gz"
