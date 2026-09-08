#!/usr/bin/env bash
set -e

BUILD_DIR="$HOME/.cache/azterm-build"
REPO_URL="https://github.com/AZBrandCanada/azTerm.git"

echo "[1/5] Checking build dependencies..."
if command -v pacman &>/dev/null; then
    sudo pacman -S --needed --noconfirm base-devel git libxkbcommon openssl libxcb libx11 wayland mesa
elif command -v apt-get &>/dev/null; then
    sudo apt-get update -qq
    sudo apt-get install -y -qq build-essential git pkg-config libxkbcommon-dev libssl-dev libxcb1-dev libx11-dev libwayland-dev libgl1-mesa-dev
elif command -v dnf &>/dev/null; then
    sudo dnf install -y git gcc gcc-c++ make pkgconf-pkg-config libxkbcommon-devel openssl-devel libxcb-devel libX11-devel wayland-devel mesa-libGL-devel
fi

if ! command -v cargo &>/dev/null; then
    echo "Installing Rust toolchain..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

echo "[2/5] Preparing source repository in persistent build cache ($BUILD_DIR)..."
mkdir -p "$HOME/.cache"
if [ -d "$BUILD_DIR/.git" ]; then
    echo "Found existing build cache, fetching latest updates..."
    cd "$BUILD_DIR"
    git fetch --all --tags -q
    git reset --hard origin/main -q || git reset --hard origin/master -q
    git pull -q
else
    echo "Cloning repository..."
    git clone "$REPO_URL" "$BUILD_DIR"
    cd "$BUILD_DIR"
fi

echo "[3/5] Compiling AZTerm in release mode..."
cargo build --release --locked

echo "[4/5] Installing binary and desktop integration..."
sudo mkdir -p /usr/local/bin /usr/share/applications /usr/share/icons/hicolor/scalable/apps
sudo install -Dm755 target/release/azterm /usr/local/bin/azterm

# If previously installed via .deb to /usr/bin, update it too so Ubuntu never runs the old version
if [ -f "/usr/bin/azterm" ] || [ -L "/usr/bin/azterm" ]; then
    sudo install -Dm755 target/release/azterm /usr/bin/azterm
fi

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

echo "AZTerm successfully installed and updated!"
