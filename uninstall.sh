#!/usr/bin/env bash
set -e

echo "=========================================="
echo "Uninstalling AZTerm from System"
echo "=========================================="
echo ""

# 1. Remove Binaries
echo "[1/4] Removing executables..."
rm -f "$HOME/.local/bin/azterm"
sudo rm -f /usr/local/bin/azterm /usr/bin/azterm 2>/dev/null || true

# 2. Remove Desktop Launchers & Icons
echo "[2/4] Removing desktop entries and icons..."
rm -f "$HOME/.local/share/applications/azterm.desktop"
rm -f "$HOME/.local/share/icons/hicolor/scalable/apps/azterm.svg"
sudo rm -f /usr/share/applications/azterm.desktop 2>/dev/null || true
sudo rm -f /usr/share/icons/hicolor/scalable/apps/azterm.svg /usr/share/pixmaps/azterm.svg 2>/dev/null || true

# 3. Remove File Manager Context Menus
echo "[3/4] Removing file manager context menu integration..."
rm -f "$HOME/.local/share/kio/servicemenus/azterm_open.desktop"
rm -f "$HOME/.local/share/kservices5/ServiceMenus/azterm_open.desktop"
rm -f "$HOME/.local/share/nautilus/scripts/Open in AZTerm"
rm -f "$HOME/.local/share/nemo/actions/azterm.nemo_action"
sudo rm -f /usr/share/kio/servicemenus/azterm_open.desktop 2>/dev/null || true

# 4. Refresh Desktop & KDE Caches
echo "[4/4] Refreshing system caches..."
if command -v kbuildsycoca6 &> /dev/null; then
    kbuildsycoca6 --noincremental 2>/dev/null || true
elif command -v kbuildsycoca5 &> /dev/null; then
    kbuildsycoca5 --noincremental 2>/dev/null || true
fi

if command -v update-desktop-database &> /dev/null; then
    update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
    sudo update-desktop-database /usr/share/applications 2>/dev/null || true
fi

echo ""
echo "=========================================="
echo "Uninstallation Complete!"
echo "=========================================="
echo "Note: Your saved configurations in ~/.config/azterm were preserved."
echo "To completely wipe all settings and database history, run:"
echo "rm -rf ~/.config/azterm"
