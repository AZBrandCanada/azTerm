#!/usr/bin/env bash
set -e

echo "=========================================="
echo "Installing AZTerm with Full OS Integration"
echo "=========================================="
echo ""

# 1. Build release binary
echo "[1/5] Building release binary..."
cargo build --release

# 2. Setup standard user directory structure
echo "[2/5] Creating user system directories..."
mkdir -p "$HOME/.local/bin"
mkdir -p "$HOME/.local/share/applications"
mkdir -p "$HOME/.local/share/icons/hicolor/scalable/apps"
mkdir -p "$HOME/.local/share/kio/servicemenus"
mkdir -p "$HOME/.local/share/kservices5/ServiceMenus"
mkdir -p "$HOME/.local/share/nautilus/scripts"
mkdir -p "$HOME/.local/share/nemo/actions"

# 3. Copy binary and desktop launcher
echo "[3/5] Installing executable and desktop launchers..."
cp target/release/azterm "$HOME/.local/bin/azterm"
cp assets/azterm.desktop "$HOME/.local/share/applications/azterm.desktop"
cp assets/azterm.svg "$HOME/.local/share/icons/hicolor/scalable/apps/azterm.svg"

# 4. Install file manager context menus (Dolphin, Nautilus, Nemo)
echo "[4/5] Installing file manager right-click context menus..."
# KDE Dolphin (Plasma 5 & 6)
cp assets/servicemenus/azterm_open.desktop "$HOME/.local/share/kio/servicemenus/azterm_open.desktop"
cp assets/servicemenus/azterm_open.desktop "$HOME/.local/share/kservices5/ServiceMenus/azterm_open.desktop"

# GNOME Nautilus
cp assets/nautilus/open-in-azterm.sh "$HOME/.local/share/nautilus/scripts/Open in AZTerm"

# Nemo
cp assets/nemo/azterm.nemo_action "$HOME/.local/share/nemo/actions/azterm.nemo_action"

# 5. Register XDG MIME Types & Protocol Handlers
echo "[5/5] Registering URI scheme handlers (ssh:// and sftp://)..."
if command -v update-desktop-database &> /dev/null; then
    update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
fi

if command -v xdg-mime &> /dev/null; then
    xdg-mime default azterm.desktop x-scheme-handler/ssh 2>/dev/null || true
    xdg-mime default azterm.desktop x-scheme-handler/sftp 2>/dev/null || true
    xdg-mime default azterm.desktop x-scheme-handler/terminal 2>/dev/null || true
fi

echo ""
echo "=========================================="
echo "Installation & OS Integration Complete!"
echo "=========================================="
echo "1. Desktop App: Available in your application launcher as 'AZTerm'."
echo "2. Right-Click: 'Open in AZTerm' is now active in Dolphin, Nautilus, and Nemo."
echo "3. URL Schemes: ssh:// and sftp:// links now launch AZTerm automatically."
echo "4. CLI Usage: azterm -d /path/to/folder (or 'azterm /path/to/folder')"
