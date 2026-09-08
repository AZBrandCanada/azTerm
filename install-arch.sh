#!/usr/bin/env bash
set -e

echo "[1/5] Checking Arch system dependencies..."
sudo pacman -S --needed --noconfirm base-devel libxkbcommon openssl libxcb libx11 wayland mesa

if ! command -v cargo &>/dev/null; then
    sudo pacman -S --needed --noconfirm rust
fi

echo "[2/5] Compiling AZTerm in release mode..."
cargo build --release --locked

echo "[3/5] Setting up system and user paths..."
sudo mkdir -p /usr/local/bin /usr/share/applications /usr/share/icons/hicolor/scalable/apps

echo "[4/5] Installing binary and desktop integration..."
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
