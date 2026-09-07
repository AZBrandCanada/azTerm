#!/usr/bin/env bash
set -e

echo "=========================================="
echo "Installing AZTerm on Arch Linux / CachyOS"
echo "=========================================="
echo ""

# 1. Ensure Dependencies (Avoid rustup vs pacman rust conflicts)
echo "[1/5] Checking Arch system dependencies..."
DEPS=("base-devel" "libxkbcommon" "openssl" "libxcb" "libx11" "wayland" "mesa")

if ! command -v rustc &> /dev/null && ! command -v rustup &> /dev/null; then
    DEPS+=("rust")
fi

sudo pacman -S --needed --noconfirm "${DEPS[@]}"

# 2. Build Release Binary
echo "[2/5] Compiling AZTerm in release mode..."
cargo build --release

# 3. Create Directories
echo "[3/5] Setting up system and user paths..."
mkdir -p "$HOME/.local/bin"
mkdir -p "$HOME/.local/share/applications"
mkdir -p "$HOME/.local/share/icons/hicolor/scalable/apps"
mkdir -p "$HOME/.local/share/kio/servicemenus"
mkdir -p "$HOME/.local/share/kservices5/ServiceMenus"
mkdir -p "$HOME/.local/share/nautilus/scripts"
mkdir -p "$HOME/.local/share/nemo/actions"

# 4. Install Executable, Icon, and Desktop Entry
echo "[4/5] Installing binary and desktop integration..."
cp target/release/azterm "$HOME/.local/bin/azterm"
chmod +x "$HOME/.local/bin/azterm"

# Install globally to /usr/local/bin
sudo cp target/release/azterm /usr/local/bin/azterm
sudo chmod +x /usr/local/bin/azterm

# User Icon & Desktop
cp assets/azterm.desktop "$HOME/.local/share/applications/azterm.desktop"
cp assets/azterm.svg "$HOME/.local/share/icons/hicolor/scalable/apps/azterm.svg"

# System Icon & Desktop
sudo cp assets/azterm.desktop /usr/share/applications/azterm.desktop
sudo mkdir -p /usr/share/icons/hicolor/scalable/apps /usr/share/pixmaps
sudo cp assets/azterm.svg /usr/share/icons/hicolor/scalable/apps/azterm.svg
sudo cp assets/azterm.svg /usr/share/pixmaps/azterm.svg

# 5. Install File Manager Right-Click Menus
echo "[5/5] Configuring Dolphin and file manager context menus..."
cp assets/servicemenus/azterm_open.desktop "$HOME/.local/share/kio/servicemenus/azterm_open.desktop"
cp assets/servicemenus/azterm_open.desktop "$HOME/.local/share/kservices5/ServiceMenus/azterm_open.desktop"
chmod +x "$HOME/.local/share/kio/servicemenus/azterm_open.desktop"
chmod +x "$HOME/.local/share/kservices5/ServiceMenus/azterm_open.desktop"

# System-wide KDE ServiceMenu
sudo mkdir -p /usr/share/kio/servicemenus
sudo cp assets/servicemenus/azterm_open.desktop /usr/share/kio/servicemenus/azterm_open.desktop
sudo chmod +x /usr/share/kio/servicemenus/azterm_open.desktop

# Refresh Caches
if command -v kbuildsycoca6 &> /dev/null; then
    kbuildsycoca6 --noincremental 2>/dev/null || true
elif command -v kbuildsycoca5 &> /dev/null; then
    kbuildsycoca5 --noincremental 2>/dev/null || true
fi

if command -v update-desktop-database &> /dev/null; then
    update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
    sudo update-desktop-database /usr/share/applications 2>/dev/null || true
fi

if command -v xdg-mime &> /dev/null; then
    xdg-mime default azterm.desktop x-scheme-handler/ssh 2>/dev/null || true
    xdg-mime default azterm.desktop x-scheme-handler/sftp 2>/dev/null || true
    xdg-mime default azterm.desktop x-scheme-handler/terminal 2>/dev/null || true
fi

echo ""
echo "=========================================="
echo "Installation Complete on Arch / CachyOS!"
echo "=========================================="
echo "Run 'azterm' from terminal, launcher, or right-click context menu."
