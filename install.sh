#!/usr/bin/env bash
set -e

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
