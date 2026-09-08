#!/usr/bin/env bash
set -e

BUILD_DIR="$HOME/.cache/azterm-build"
REPO_URL="https://github.com/AZBrandCanada/azTerm.git"

if [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
fi
export PATH="$HOME/.cargo/bin:$PATH"

echo "[1/5] Checking Arch system dependencies..."
sudo pacman -S --needed --noconfirm base-devel git libxkbcommon openssl libxcb libx11 wayland mesa

if ! command -v cargo &>/dev/null; then
    sudo pacman -S --needed --noconfirm rust
fi

echo "[2/5] Preparing source repository in persistent build cache..."
mkdir -p "$HOME/.cache"
if [ -d "$BUILD_DIR/.git" ]; then
    cd "$BUILD_DIR"
    git fetch --all --tags || true
    git checkout -f main 2>/dev/null || git checkout -f master 2>/dev/null || true
    git reset --hard origin/main 2>/dev/null || git reset --hard origin/master 2>/dev/null || true
    git clean -fd || true
else
    rm -rf "$BUILD_DIR"
    git clone "$REPO_URL" "$BUILD_DIR"
    cd "$BUILD_DIR"
fi

if [ ! -f "$BUILD_DIR/Cargo.toml" ]; then
    rm -rf "$BUILD_DIR"
    git clone "$REPO_URL" "$BUILD_DIR"
    cd "$BUILD_DIR"
fi

echo "[3/5] Compiling AZTerm in release mode..."
cargo build --release

echo "[4/5] Installing binary and desktop integration..."
sudo mkdir -p /usr/local/bin /usr/share/applications /usr/share/icons/hicolor/scalable/apps
sudo install -Dm755 target/release/azterm /usr/local/bin/azterm

if [ -f "assets/azterm.desktop" ]; then
    sudo install -Dm644 assets/azterm.desktop /usr/share/applications/azterm.desktop
fi
if [ -f "assets/azterm.svg" ]; then
    sudo install -Dm644 assets/azterm.svg /usr/share/icons/hicolor/scalable/apps/azterm.svg
fi

echo "[5/5] Updating desktop & icon caches..."
sudo update-desktop-database -q /usr/share/applications || true
sudo gtk-update-icon-cache -q /usr/share/icons/hicolor || true

echo "AZTerm installation on Arch Linux complete!"
