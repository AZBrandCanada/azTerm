#!/usr/bin/env bash
set -e

echo "=========================================="
echo "Installing AZTerm on Ubuntu / Debian"
echo "=========================================="
echo ""

# 1. Ensure Dependencies
echo "[1/5] Checking Debian/Ubuntu dependencies..."
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev libx11-dev libgl1-mesa-dev

# 2. Build Release Binary
echo "[2/5] Compiling AZTerm in release mode..."
cargo build --release

# 3. Create Directories
echo "[3/5] Setting up system and user paths..."
mkdir -p "$HOME/.local/bin"
mkdir -p "$HOME/.local/share/applications"
mkdir -p "$HOME/.local/share/icons/hicolor/scalable/apps"
mkdir -p "$HOME/.local/share/nautilus/scripts"
mkdir -p "$HOME/.local/share/kio/servicemenus"

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

# 5. Install File Manager Right-Click Scripts (GNOME Nautilus)
echo "[5/5] Configuring Nautilus and desktop integration..."
cat << 'NAUTILUS_SCRIPT' > "$HOME/.local/share/nautilus/scripts/Open in AZTerm"
#!/usr/bin/env bash
TARGET_DIR="${NAUTILUS_SCRIPT_CURRENT_URI:-$1}"
if [ -n "$TARGET_DIR" ]; then
    CLEAN_DIR="${TARGET_DIR#file://}"
    azterm -d "$CLEAN_DIR"
else
    azterm
fi
NAUTILUS_SCRIPT
chmod +x "$HOME/.local/share/nautilus/scripts/Open in AZTerm"

# Update Caches
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
echo "Installation Complete on Ubuntu / Debian!"
echo "=========================================="
echo "Run 'azterm' from terminal, launcher, or right-click scripts."
