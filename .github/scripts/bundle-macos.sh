#!/usr/bin/env bash
# Usage: bundle-macos.sh <target-triple> <arch-label>
#   dist/panda-reader-<version>-macos-<arch>.zip
#   dist/panda-reader-<version>-macos-<arch>.dmg
#
# Signs with Developer ID + notarizes when APPLE_* secrets are present;
# otherwise ad-hoc signs (fine for CI artifacts, Gatekeeper will warn).
set -euo pipefail

TARGET="$1"
ARCH="$2"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
VERSION="$("$ROOT/.github/scripts/version.sh")"
NAME="panda-reader-${VERSION}-macos-${ARCH}"
APP="dist/Panda Reader.app"

rm -rf dist
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/Third-Party-Asset-Licenses/fonts"

cp "target/${TARGET}/release/panda-reader" "$APP/Contents/MacOS/panda-reader"
chmod +x "$APP/Contents/MacOS/panda-reader"
cp LICENSE "$APP/Contents/Resources/LICENSE.txt"
cp README.md "$APP/Contents/Resources/README.md"
cp THIRD_PARTY_NOTICES.md "$APP/Contents/Resources/THIRD_PARTY_NOTICES.md"
cp THIRD_PARTY_LICENSES.txt "$APP/Contents/Resources/THIRD_PARTY_LICENSES.txt"
cp apps/panda-reader/assets/TTY7-LICENSE "$APP/Contents/Resources/Third-Party-Asset-Licenses/TTY7-LICENSE"
cp apps/panda-reader/assets/fonts/*LICENSE* "$APP/Contents/Resources/Third-Party-Asset-Licenses/fonts/"

# Build .icns from the PNG for Finder / Dock.
ICONSET="$(mktemp -d)/PandaReader.iconset"
mkdir -p "$ICONSET"
SRC="apps/panda-reader/assets/app-icon.png"
for size in 16 32 64 128 256 512; do
  sips -z "$size" "$size" "$SRC" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  dub=$((size * 2))
  if (( dub <= 1024 )); then
    sips -z "$dub" "$dub" "$SRC" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
  fi
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/PandaReader.icns"
rm -rf "$(dirname "$ICONSET")"

printf 'APPL????' > "$APP/Contents/PkgInfo"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Panda Reader</string>
    <key>CFBundleDisplayName</key><string>Panda Reader</string>
    <key>CFBundleIdentifier</key><string>com.PandaReader.PandaReader</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundleExecutable</key><string>panda-reader</string>
    <key>CFBundleIconFile</key><string>PandaReader</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

SIGN_ID="${APPLE_SIGNING_IDENTITY:-}"
SIGNING_STATUS="unsigned"
if [[ -n "$SIGN_ID" && -n "${APPLE_CERTIFICATE:-}" ]]; then
  KEYCHAIN="panda-reader-signing.keychain-db"
  KEYCHAIN_PASSWORD="${KEYCHAIN_PASSWORD:-panda-reader-ci}"
  security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
  security set-keychain-settings -lut 21600 "$KEYCHAIN"
  security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
  echo "$APPLE_CERTIFICATE" | base64 --decode > /tmp/panda-reader-cert.p12
  security import /tmp/panda-reader-cert.p12 -k "$KEYCHAIN" \
    -P "${APPLE_CERTIFICATE_PASSWORD:-}" -T /usr/bin/codesign -T /usr/bin/security
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
    -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
  security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
  codesign --force --deep --options runtime --sign "$SIGN_ID" "$APP"
  SIGNING_STATUS="Developer ID signed; not notarized"
  if [[ -n "${APPLE_ID:-}" && -n "${APPLE_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]]; then
    ditto -c -k --keepParent "$APP" /tmp/panda-reader-notarize.zip
    xcrun notarytool submit /tmp/panda-reader-notarize.zip \
      --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
    xcrun stapler staple "$APP"
    SIGNING_STATUS="Developer ID signed and notarized"
  fi
else
  if codesign --force --deep --sign - "$APP"; then
    SIGNING_STATUS="ad-hoc signed; not notarized"
  fi
fi

printf 'macOS artifact signing status: %s\n' "$SIGNING_STATUS" > "dist/${NAME}-signing-status.txt"

ditto -c -k --keepParent "$APP" "dist/${NAME}.zip"
test -s "dist/${NAME}.zip"
ZIP_CHECK="$(mktemp -d)"
ditto -x -k "dist/${NAME}.zip" "$ZIP_CHECK"
test -x "$ZIP_CHECK/Panda Reader.app/Contents/MacOS/panda-reader"
rm -rf "$ZIP_CHECK"

DMG_STAGE="$(mktemp -d)"
cp -R "$APP" "$DMG_STAGE/"
ln -s /Applications "$DMG_STAGE/Applications"
hdiutil create -volname "Panda Reader" -srcfolder "$DMG_STAGE" -ov -format UDZO "dist/${NAME}.dmg"
rm -rf "$DMG_STAGE"
test -s "dist/${NAME}.dmg"
hdiutil verify "dist/${NAME}.dmg"

echo "OK dist/${NAME}.zip"
echo "OK dist/${NAME}.dmg"
echo "OK dist/${NAME}-signing-status.txt ($SIGNING_STATUS)"
