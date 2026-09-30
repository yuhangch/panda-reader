#!/usr/bin/env bash
# Usage: bundle-appimage.sh <target-triple> <arch-label>
#   dist/panda-reader-<version>-linux-<arch>.AppImage
set -euo pipefail

TARGET="$1"
ARCH="$2"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
VERSION="$("$ROOT/.github/scripts/version.sh")"
NAME="panda-reader-${VERSION}-linux-${ARCH}"

export APPIMAGE_EXTRACT_AND_RUN=1

case "$ARCH" in
  x86_64) LD_ARCH=x86_64 ;;
  arm64 | aarch64) LD_ARCH=aarch64 ;;
  *) echo "unsupported arch for AppImage: $ARCH" >&2; exit 1 ;;
esac

TOOLS="$(mktemp -d)"
LINUXDEPLOY="$TOOLS/linuxdeploy-${LD_ARCH}.AppImage"
APPIMAGETOOL="$TOOLS/appimagetool-${LD_ARCH}.AppImage"
curl -fsSL -o "$LINUXDEPLOY" \
  "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${LD_ARCH}.AppImage"
curl -fsSL -o "$APPIMAGETOOL" \
  "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${LD_ARCH}.AppImage"
chmod +x "$LINUXDEPLOY" "$APPIMAGETOOL"

APPDIR="dist/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/doc/panda-reader/THIRD_PARTY_ASSET_LICENSES/fonts" dist

cp "target/${TARGET}/release/panda-reader" "$APPDIR/usr/bin/panda-reader"
chmod +x "$APPDIR/usr/bin/panda-reader"
cp LICENSE "$APPDIR/usr/share/doc/panda-reader/LICENSE"
cp README.md "$APPDIR/usr/share/doc/panda-reader/README.md"
cp THIRD_PARTY_NOTICES.md "$APPDIR/usr/share/doc/panda-reader/THIRD_PARTY_NOTICES.md"
cp THIRD_PARTY_LICENSES.txt "$APPDIR/usr/share/doc/panda-reader/THIRD_PARTY_LICENSES.txt"
cp apps/panda-reader/assets/TTY7-LICENSE "$APPDIR/usr/share/doc/panda-reader/THIRD_PARTY_ASSET_LICENSES/TTY7-LICENSE"
cp apps/panda-reader/assets/fonts/*LICENSE* "$APPDIR/usr/share/doc/panda-reader/THIRD_PARTY_ASSET_LICENSES/fonts/"

cat > "$TOOLS/panda-reader.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Panda Reader
Comment=A desktop RSS reader
Exec=panda-reader
Icon=panda-reader
Categories=Network;News;
Terminal=false
StartupWMClass=panda-reader
X-AppImage-Version=${VERSION}
DESKTOP

convert apps/panda-reader/assets/app-icon.png -resize 256x256 "$TOOLS/panda-reader.png"

"$LINUXDEPLOY" \
  --appdir "$APPDIR" \
  --executable "$APPDIR/usr/bin/panda-reader" \
  --desktop-file "$TOOLS/panda-reader.desktop" \
  --icon-file "$TOOLS/panda-reader.png"

"$APPIMAGETOOL" "$APPDIR" "dist/${NAME}.AppImage"
chmod +x "dist/${NAME}.AppImage"
test -s "dist/${NAME}.AppImage"
"dist/${NAME}.AppImage" --appimage-extract >/dev/null
test -x squashfs-root/usr/bin/panda-reader
rm -rf squashfs-root
rm -rf "$APPDIR" "$TOOLS"
echo "OK dist/${NAME}.AppImage"
