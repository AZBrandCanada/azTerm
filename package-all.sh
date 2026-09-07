#!/usr/bin/env bash
set -e

ROOT_DIR="$(pwd)"
DIST_DIR="$ROOT_DIR/dist"
mkdir -p "$DIST_DIR"
mkdir -p target

echo "=========================================="
echo "AZTerm Master Multi-Platform Build Script"
echo "=========================================="
echo ""

# -----------------------------------------------------------
# 1. Build Native Linux Binary, AppImage, and Tarball
# -----------------------------------------------------------
echo "[1/4] Building Native Linux Release Binary..."
cargo build --release

echo "-> Creating Linux Tarball..."
STAGE_DIR="target/stage-linux"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"

cp target/release/azterm "$STAGE_DIR/azterm"
cp assets/azterm.desktop "$STAGE_DIR/azterm.desktop"
cp assets/azterm.svg "$STAGE_DIR/azterm.svg"

tar -czf "$DIST_DIR/azterm-linux-x86_64.tar.gz" -C "$STAGE_DIR" .

echo "-> Building Standalone AppImage..."
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

if [ ! -f "target/appimagetool" ]; then
    echo "-> Downloading appimagetool..."
    curl -sLo target/appimagetool https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage
    chmod +x target/appimagetool
fi

ARCH=x86_64 ./target/appimagetool --appimage-extract-and-run "$APPDIR" "$DIST_DIR/AZTerm-x86_64.AppImage" > /dev/null 2>&1
echo "-> Generated: dist/AZTerm-x86_64.AppImage"

# -----------------------------------------------------------
# 2. Build Debian / Ubuntu .deb Package
# -----------------------------------------------------------
echo ""
echo "[2/4] Building Debian / Ubuntu .deb Package..."
if command -v cargo-deb &> /dev/null; then
    cargo deb -o "$DIST_DIR/azterm_0.1.0_amd64.deb"
    echo "-> Generated: dist/azterm_0.1.0_amd64.deb"
else
    echo "-> Skipping .deb (cargo-deb not installed. Run: cargo install cargo-deb)"
fi

# -----------------------------------------------------------
# 3. Cross-Compile Windows Binary (.exe / .zip)
# -----------------------------------------------------------
echo ""
echo "[3/4] Cross-Compiling Windows 64-bit Binary..."
if command -v x86_64-w64-mingw32-gcc &> /dev/null; then
    rustup target add x86_64-pc-windows-gnu 2>/dev/null || true
    cargo build --release --target x86_64-pc-windows-gnu
    
    echo "-> Packaging Windows zip archive..."
    if command -v zip &> /dev/null; then
        zip -j "$DIST_DIR/azterm-windows-x86_64.zip" target/x86_64-pc-windows-gnu/release/azterm.exe
        echo "-> Generated: dist/azterm-windows-x86_64.zip"
    else
        cp target/x86_64-pc-windows-gnu/release/azterm.exe "$DIST_DIR/azterm.exe"
        echo "-> Generated: dist/azterm.exe"
    fi
else
    echo "-> Skipping Windows build (mingw-w64-gcc missing. Run: sudo pacman -S mingw-w64-gcc)"
fi

# -----------------------------------------------------------
# 4. Summary Output
# -----------------------------------------------------------
echo ""
echo "=========================================="
echo "Build Complete! Generated artifacts in dist/:"
echo "=========================================="
ls -lh "$DIST_DIR"
