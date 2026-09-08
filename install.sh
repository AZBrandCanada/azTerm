#!/usr/bin/env bash
set -e

echo "[1/5] Checking build dependencies..."
if command -v pacman &>/dev/null; then
    sudo pacman -S --needed --noconfirm base-devel libxkbcommon openssl libxcb libx11 wayland mesa
elif command -v apt-get &>/dev/null; then
    sudo apt-get update -qq
    sudo apt-get install -y -qq build-essential libxkbcommon-dev libssl-dev libxcb1-dev libx11-dev libwayland-dev libgl1-mesa-dev
elif command -v dnf &>/dev/null; then
    sudo dnf install -y libxkbcommon-devel openssl-devel libxcb-devel libX11-devel wayland-devel mesa-libGL-devel
fi

if ! command -v cargo &>/dev/null; then
    echo "Installing Rust toolchain..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

echo "[2/5] Compiling AZTerm in release mode..."
cargo build --release --locked

echo "[3/5] Setting up system and user paths..."
sudo mkdir -p /usr/local/bin /usr/share/applications /usr/share/icons/hicolor/scalable/apps

echo "[4/5] Installing binary and desktop integration..."
# Using install -Dm755 unlinks any running binary to prevent "Text file busy"
sudo install -Dm755 target/release/azterm /usr/local/bin/azterm
if [ -f "assets/azterm.desktop" ]; then
    sudo install -Dm644 assets/azterm.desktop /usr/share/applications/azterm.desktop
fi
if [ -f "assets/azterm.svg" ]; then
    sudo install -Dm644 assets/azterm.svg /usr/share/icons/hicolor/scalable/apps/azterm.svg
fi

echo "[5/5] Updating desktop & icon caches..."
if command -v update-desktop-database &>/dev/null; then
    sudo update-desktop-database -q /usr/share/applications || true
fi
if command -v gtk-update-icon-cache &>/dev/null; then
    sudo gtk-update-icon-cache -q /usr/share/icons/hicolor || true
fi

echo "AZTerm successfully installed/updated!"
