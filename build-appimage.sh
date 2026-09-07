#!/usr/bin/env bash
set -e

echo "[1/4] Compiling release binary..."
cargo build --release

echo "[2/4] Setting up AppDir directory structure..."
APPDIR="target/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"

cp target/release/azterm "$APPDIR/usr/bin/azterm"
cp assets/azterm.desktop "$APPDIR/azterm.desktop"
cp assets/azterm.desktop "$APPDIR/usr/share/applications/azterm.desktop"
cp assets/azterm.svg "$APPDIR/azterm.svg"
cp assets/azterm.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/azterm.svg"

cat << 'APPRUN' > "$APPDIR/AppRun"
#!/bin/sh
SELF=$(readlink -f "$0")
HERE=${SELF%/*}
export PATH="${HERE}/usr/bin:${PATH}"
exec "${HERE}/usr/bin/azterm" "$@"
APPRUN
chmod +x "$APPDIR/AppRun"

echo "[3/4] Downloading appimagetool if not present..."
if [ ! -f "target/appimagetool" ]; then
    curl -Lo target/appimagetool https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage
    chmod +x target/appimagetool
fi

echo "[4/4] Generating AZTerm-x86_64.AppImage..."
ARCH=x86_64 ./target/appimagetool --appimage-extract-and-run "$APPDIR" target/AZTerm-x86_64.AppImage

echo "Build successful: target/AZTerm-x86_64.AppImage"
