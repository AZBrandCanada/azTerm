#!/usr/bin/env bash
set -e

# If executed via 'curl | bash', clone repository to a temporary workspace
if [ ! -f "Cargo.toml" ]; then
    echo "=========================================="
    echo "Fetching AZTerm from GitHub..."
    echo "=========================================="
    TEMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TEMP_DIR"' EXIT
    git clone --depth=1 https://github.com/AZBrandCanada/azTerm.git "$TEMP_DIR"
    cd "$TEMP_DIR"
fi

# Detect distribution and execute appropriate installer
if [ -f /etc/arch-release ] || command -v pacman &> /dev/null; then
    ./install-arch.sh
elif [ -f /etc/debian_version ] || command -v apt-get &> /dev/null; then
    ./install-debian.sh
else
    echo "Unsupported distribution. Falling back to generic user installation..."
    cargo build --release
    mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/scalable/apps"
    cp target/release/azterm "$HOME/.local/bin/azterm"
    cp assets/azterm.desktop "$HOME/.local/share/applications/azterm.desktop"
    cp assets/azterm.svg "$HOME/.local/share/icons/hicolor/scalable/apps/azterm.svg"
    chmod +x "$HOME/.local/bin/azterm"
fi
