#!/usr/bin/env bash
set -e

ROOT_DIR="$(pwd)"
DIST_DIR="$ROOT_DIR/dist"
mkdir -p "$DIST_DIR"
mkdir -p target

echo "=========================================="
echo "AZTerm Master Multi-Platform Build Script"
echo "Repository: https://github.com/AZBrandCanada/azTerm"
echo "=========================================="
echo ""

# -----------------------------------------------------------
# 1. Build Native Linux Release Binary
# -----------------------------------------------------------
echo "[1/5] Compiling Linux Release Binary..."
cargo build --release

echo "-> Packaging Linux Tarball..."
STAGE_DIR="target/stage-linux"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"

cp target/release/azterm "$STAGE_DIR/azterm"
cp assets/azterm.desktop "$STAGE_DIR/azterm.desktop"
cp assets/azterm.svg "$STAGE_DIR/azterm.svg"
cp assets/azterm.png "$STAGE_DIR/azterm.png" 2>/dev/null || true

tar -czf "$DIST_DIR/azterm-linux-x86_64.tar.gz" -C "$STAGE_DIR" .
echo "-> Generated: dist/azterm-linux-x86_64.tar.gz"

# -----------------------------------------------------------
# 2. Build Standalone Linux AppImage
# -----------------------------------------------------------
echo ""
echo "[2/5] Building Standalone Linux AppImage..."
APPDIR="target/AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"
mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"

cp target/release/azterm "$APPDIR/usr/bin/azterm"
cp assets/azterm.desktop "$APPDIR/azterm.desktop"
cp assets/azterm.desktop "$APPDIR/usr/share/applications/azterm.desktop"
cp assets/azterm.svg "$APPDIR/azterm.svg"
cp assets/azterm.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/azterm.svg"
cp assets/azterm.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/azterm.png" 2>/dev/null || true

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
# 3. Build Arch Linux Native Package (.pkg.tar.zst)
# -----------------------------------------------------------
echo ""
echo "[3/5] Building Arch Linux Native Package..."
if command -v makepkg &> /dev/null; then
    PKG_STAGE="target/pkgbuild-stage"
    rm -rf "$PKG_STAGE"
    mkdir -p "$PKG_STAGE"
    cp PKGBUILD "$PKG_STAGE/"
    cp target/release/azterm "$PKG_STAGE/"
    mkdir -p "$PKG_STAGE/assets"
    cp assets/azterm.desktop "$PKG_STAGE/assets/"
    cp assets/azterm.svg "$PKG_STAGE/assets/"

    cat << 'LOCAL_PKGBUILD' > "$PKG_STAGE/PKGBUILD"
pkgname=azterm
pkgver=0.1.0
pkgrel=1
pkgdesc="Fast, modern native terminal, SSH bookmark manager, and SFTP client"
arch=('x86_64')
url="https://github.com/AZBrandCanada/azTerm"
license=('MIT' 'Apache-2.0')
depends=('libxkbcommon' 'openssl' 'libxcb' 'libx11' 'mesa' 'wayland')

package() {
    install -Dm755 "$startdir/azterm" "$pkgdir/usr/bin/azterm"
    install -Dm644 "$startdir/assets/azterm.desktop" "$pkgdir/usr/share/applications/azterm.desktop"
    install -Dm644 "$startdir/assets/azterm.svg" "$pkgdir/usr/share/icons/hicolor/scalable/apps/azterm.svg"
}
LOCAL_PKGBUILD

    (cd "$PKG_STAGE" && makepkg -f --nodeps > /dev/null 2>&1)
    cp "$PKG_STAGE"/azterm-*.pkg.tar.zst "$DIST_DIR/" 2>/dev/null || true
    echo "-> Generated: $(ls "$DIST_DIR"/azterm-*.pkg.tar.zst 2>/dev/null | head -n 1)"
else
    echo "-> Skipping Arch package (makepkg not found)."
fi

# -----------------------------------------------------------
# 4. Build Debian / Ubuntu .deb Package
# -----------------------------------------------------------
echo ""
echo "[4/5] Building Debian / Ubuntu .deb Package..."
if ! command -v cargo-deb &> /dev/null; then
    echo "-> Installing cargo-deb..."
    cargo install cargo-deb 2>/dev/null || true
fi

if command -v cargo-deb &> /dev/null; then
    cargo deb -o "$DIST_DIR/azterm_0.1.0_amd64.deb"
    echo "-> Generated: dist/azterm_0.1.0_amd64.deb"
else
    echo "-> Skipping .deb (cargo-deb unavailable)."
fi

# -----------------------------------------------------------
# 5. Cross-Compile Windows 64-bit Binary (.exe / .zip)
# -----------------------------------------------------------
echo ""
echo "[5/5] Cross-Compiling Windows 64-bit Binary..."
if command -v x86_64-w64-mingw32-gcc &> /dev/null; then
    rustup target add x86_64-pc-windows-gnu 2>/dev/null || true
    cargo build --release --target x86_64-pc-windows-gnu
    
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
# Summary Output
# -----------------------------------------------------------
echo ""
echo "=========================================="
echo "Build Complete! All packages ready in dist/:"
echo "=========================================="
ls -lh "$DIST_DIR"
