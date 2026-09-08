#!/usr/bin/env bash
set -e

BUILD_DIR="$HOME/.cache/azterm-build"
REPO_URL="https://github.com/AZBrandCanada/azTerm.git"

if [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
fi
export PATH="$HOME/.cargo/bin:$PATH"

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
    export PATH="$HOME/.cargo/bin:$PATH"
fi

echo "[2/5] Preparing source repository in persistent build cache ($BUILD_DIR)..."
mkdir -p "$HOME/.cache"
if [ -d "$BUILD_DIR/.git" ]; then
    echo "Updating existing build cache..."
    cd "$BUILD_DIR"
    git fetch --all --tags || true
    git checkout -f main 2>/dev/null || git checkout -f master 2>/dev/null || true
    git reset --hard origin/main 2>/dev/null || git reset --hard origin/master 2>/dev/null || true
    git clean -fd || true
else
    echo "Cloning repository..."
    rm -rf "$BUILD_DIR"
    git clone "$REPO_URL" "$BUILD_DIR"
    cd "$BUILD_DIR"
fi

if [ ! -f "$BUILD_DIR/Cargo.toml" ]; then
    echo "Refreshing build repository..."
    rm -rf "$BUILD_DIR"
    git clone "$REPO_URL" "$BUILD_DIR"
    cd "$BUILD_DIR"
fi

echo "[3/5] Compiling AZTerm in release mode..."
cargo build --release

echo "[4/5] Installing binary to all system and user PATH locations..."
# 1. System wide
sudo mkdir -p /usr/local/bin /usr/share/applications /usr/share/icons/hicolor/scalable/apps
sudo install -Dm755 target/release/azterm /usr/local/bin/azterm

if [ -f "/usr/bin/azterm" ] || [ -L "/usr/bin/azterm" ]; then
    sudo install -Dm755 target/release/azterm /usr/bin/azterm
fi

# 2. User level (handles ~/.cargo/bin and ~/.local/bin PATH precedence on Ubuntu)
mkdir -p "$HOME/.local/bin"
install -Dm755 target/release/azterm "$HOME/.local/bin/azterm"

if [ -f "$HOME/.cargo/bin/azterm" ]; then
    install -Dm755 target/release/azterm "$HOME/.cargo/bin/azterm"
fi

# Desktop & icon entries (system + user)
if [ -f "assets/azterm.desktop" ]; then
    sudo install -Dm644 assets/azterm.desktop /usr/share/applications/azterm.desktop
    mkdir -p "$HOME/.local/share/applications"
    install -Dm644 assets/azterm.desktop "$HOME/.local/share/applications/azterm.desktop"
fi
if [ -f "assets/azterm.svg" ]; then
    sudo install -Dm644 assets/azterm.svg /usr/share/icons/hicolor/scalable/apps/azterm.svg
    mkdir -p "$HOME/.local/share/icons/hicolor/scalable/apps"
    install -Dm644 assets/azterm.svg "$HOME/.local/share/icons/hicolor/scalable/apps/azterm.svg"
fi

echo "[5/5] Updating desktop & icon caches..."
if command -v update-desktop-database &>/dev/null; then
    sudo update-desktop-database -q /usr/share/applications || true
    update-desktop-database -q "$HOME/.local/share/applications" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache &>/dev/null; then
    sudo gtk-update-icon-cache -q /usr/share/icons/hicolor || true
fi

echo "=========================================================="
echo " AZTerm updated successfully!"
echo " Binary path: $(which azterm)"
echo "=========================================================="
