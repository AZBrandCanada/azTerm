#!/usr/bin/env bash
set -e

BUILD_DIR="$HOME/.cache/azterm-build"
REPO_URL="https://github.com/AZBrandCanada/azTerm.git"
CURRENT_USER="${SUDO_USER:-$USER}"
USER_HOME=$(eval echo "~$CURRENT_USER")

if [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
fi
export PATH="$HOME/.cargo/bin:$PATH"

echo "[1/5] Checking Debian/Ubuntu system dependencies..."
sudo apt-get update -qq
sudo apt-get install -y -qq \
    build-essential git pkg-config libxkbcommon-dev libssl-dev \
    libxcb1-dev libx11-dev libwayland-dev libgl1-mesa-dev python3-nautilus

if ! command -v cargo &>/dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    export PATH="$HOME/.cargo/bin:$PATH"
fi

echo "[2/5] Preparing source repository in persistent build cache..."
mkdir -p "$HOME/.cache"
if [ -d "$BUILD_DIR/.git" ]; then
    cd "$BUILD_DIR"
    git fetch --all --tags || true
    git checkout -f main 2>/dev/null || git checkout -f master 2>/dev/null || true
    git reset --hard origin/main 2>/dev/null || git reset --hard origin/master 2>/dev/null || true
    git clean -fd || true
else
    rm -rf "$BUILD_DIR"
    git clone "$REPO_URL" "$BUILD_DIR"
    cd "$BUILD_DIR"
fi

if [ ! -f "$BUILD_DIR/Cargo.toml" ]; then
    rm -rf "$BUILD_DIR"
    git clone "$REPO_URL" "$BUILD_DIR"
    cd "$BUILD_DIR"
fi

echo "[3/5] Compiling AZTerm in release mode..."
cargo build --release

echo "[4/5] Installing binary and overriding system terminal hooks..."
sudo mkdir -p /usr/local/bin /usr/share/applications /usr/share/icons/hicolor/scalable/apps \
              /usr/share/kio/servicemenus /usr/share/nemo/actions
sudo install -Dm755 target/release/azterm /usr/local/bin/azterm

# Ensure azterm is updated in all detected locations
declare -A SEEN_LOCS
LOCATIONS=(
    "/usr/local/bin/azterm"
    "/usr/bin/azterm"
    "$USER_HOME/.local/bin/azterm"
    "$USER_HOME/.cargo/bin/azterm"
    "$USER_HOME/bin/azterm"
)

if command -v which &>/dev/null; then
    while IFS= read -r path; do
        if [ -n "$path" ]; then
            LOCATIONS+=("$path")
        fi
    done < <(which -a azterm 2>/dev/null || true)
fi

for loc in "${LOCATIONS[@]}"; do
    if [ -n "$loc" ] && [ -z "${SEEN_LOCS[$loc]}" ]; then
        SEEN_LOCS["$loc"]=1
        if [ -e "$loc" ] || [ -L "$loc" ] || [ "$loc" = "$USER_HOME/.local/bin/azterm" ]; then
            dir_name=$(dirname "$loc")
            if [ -w "$dir_name" ]; then
                install -Dm755 target/release/azterm "$loc" 2>/dev/null || true
            else
                sudo install -Dm755 target/release/azterm "$loc" 2>/dev/null || true
            fi
        fi
    fi
done

# Desktop Entry & Icon
if [ -f "assets/azterm.desktop" ]; then
    sudo install -Dm644 assets/azterm.desktop /usr/share/applications/azterm.desktop
    mkdir -p "$USER_HOME/.local/share/applications"
    install -Dm644 assets/azterm.desktop "$USER_HOME/.local/share/applications/azterm.desktop"
fi
if [ -f "assets/azterm.svg" ]; then
    sudo install -Dm644 assets/azterm.svg /usr/share/icons/hicolor/scalable/apps/azterm.svg
    mkdir -p "$USER_HOME/.local/share/icons/hicolor/scalable/apps"
    install -Dm644 assets/azterm.svg "$USER_HOME/.local/share/icons/hicolor/scalable/apps/azterm.svg"
fi

# -------------------------------------------------------------
# 1. Divert /usr/bin/gnome-terminal to launch azterm
# This intercepts hardcoded GNOME desktop clicks & shortcuts.
# -------------------------------------------------------------
if [ -f /usr/bin/gnome-terminal ] && [ ! -f /usr/bin/gnome-terminal.original ]; then
    sudo dpkg-divert --add --rename --divert /usr/bin/gnome-terminal.original /usr/bin/gnome-terminal 2>/dev/null || true
fi

sudo tee /usr/bin/gnome-terminal > /dev/null << 'EOF'
#!/usr/bin/env bash
target_dir=""
passthrough_args=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --working-directory=*)
            target_dir="${1#*=}"
            shift
            ;;
        --working-directory)
            target_dir="$2"
            shift 2
            ;;
        *)
            passthrough_args+=("$1")
            shift
            ;;
    esac
done

if [ -n "$target_dir" ] && [ -d "$target_dir" ]; then
    cd "$target_dir"
fi

if [ ${#passthrough_args[@]} -gt 0 ]; then
    exec /usr/local/bin/azterm "${passthrough_args[@]}"
else
    exec /usr/local/bin/azterm
fi
EOF
sudo chmod 755 /usr/bin/gnome-terminal
sudo ln -sf /usr/bin/gnome-terminal /usr/local/bin/gnome-terminal

# Also divert gnome-terminal.real if present (Ubuntu 24.04+)
if [ -f /usr/bin/gnome-terminal.real ] && [ ! -f /usr/bin/gnome-terminal.real.original ]; then
    sudo dpkg-divert --add --rename --divert /usr/bin/gnome-terminal.real.original /usr/bin/gnome-terminal.real 2>/dev/null || true
    sudo ln -sf /usr/bin/gnome-terminal /usr/bin/gnome-terminal.real
fi

# -------------------------------------------------------------
# 2. Disable the hardcoded libterminal-nautilus.so C-plugin
# -------------------------------------------------------------
sudo rm -f /usr/lib/x86_64-linux-gnu/nautilus/extensions-*/libterminal-nautilus.so \
           /usr/lib/aarch64-linux-gnu/nautilus/extensions-*/libterminal-nautilus.so \
           /usr/lib/nautilus/extensions-*/libterminal-nautilus.so 2>/dev/null || true

# -------------------------------------------------------------
# 3. Install Native Nautilus Python Extension for azTerm
# -------------------------------------------------------------
NAUTILUS_EXT_DIR="$USER_HOME/.local/share/nautilus-python/extensions"
mkdir -p "$NAUTILUS_EXT_DIR"

cat << 'EOF' > "$NAUTILUS_EXT_DIR/open_azterm.py"
import os
import subprocess
from urllib.parse import unquote, urlparse
import gi

try:
    gi.require_version('Nautilus', '4.0')
except (ValueError, AttributeError):
    try:
        gi.require_version('Nautilus', '3.0')
    except (ValueError, AttributeError):
        pass

from gi.repository import Nautilus, GObject

class AzTermExtension(GObject.GObject, Nautilus.MenuProvider):
    def __init__(self):
        super().__init__()

    def _launch(self, menu, path):
        if path and os.path.isdir(path):
            subprocess.Popen(["/usr/local/bin/azterm"], cwd=path)

    def _get_path(self, item):
        try:
            loc = item.get_location()
            if loc and loc.get_path():
                return loc.get_path()
        except Exception:
            pass
        try:
            uri = item.get_uri()
            if uri and uri.startswith("file://"):
                return unquote(urlparse(uri).path)
        except Exception:
            pass
        return None

    def get_file_items(self, *args):
        files = args[-1]
        if len(files) != 1 or not files[0].is_directory():
            return []
        path = self._get_path(files[0])
        if not path:
            return []
        item = Nautilus.MenuItem(
            name="AzTerm::OpenFolder",
            label="Open in azTerm",
            tip="Open azTerm in selected folder"
        )
        item.connect("activate", self._launch, path)
        return [item]

    def get_background_items(self, *args):
        folder = args[-1]
        path = self._get_path(folder)
        if not path:
            return []
        item = Nautilus.MenuItem(
            name="AzTerm::OpenBackground",
            label="Open in azTerm",
            tip="Open azTerm in current directory"
        )
        item.connect("activate", self._launch, path)
        return [item]
EOF
chown -R "$CURRENT_USER:$CURRENT_USER" "$USER_HOME/.local/share/nautilus-python" 2>/dev/null || true

# -------------------------------------------------------------
# 4. Fallback: Nautilus Scripts Directory
# -------------------------------------------------------------
mkdir -p "$USER_HOME/.local/share/nautilus/scripts"
cat << 'EOF' > "$USER_HOME/.local/share/nautilus/scripts/Open in azTerm"
#!/usr/bin/env bash
target="${NAUTILUS_SCRIPT_SELECTED_FILE_PATHS%%$'\n'*}"
if [ -n "$target" ] && [ -d "$target" ]; then
    cd "$target"
fi
exec /usr/local/bin/azterm
EOF
chmod +x "$USER_HOME/.local/share/nautilus/scripts/Open in azTerm"
chown -R "$CURRENT_USER:$CURRENT_USER" "$USER_HOME/.local/share/nautilus/scripts" 2>/dev/null || true

# -------------------------------------------------------------
# 5. Dolphin, Nemo, and System Default Alternatives
# -------------------------------------------------------------
if [ -f "assets/servicemenus/azterm_open.desktop" ]; then
    sudo install -Dm755 assets/servicemenus/azterm_open.desktop /usr/share/kio/servicemenus/azterm_open.desktop
fi

if [ -f "assets/nemo/azterm.nemo_action" ]; then
    sudo install -Dm644 assets/nemo/azterm.nemo_action /usr/share/nemo/actions/azterm.nemo_action 2>/dev/null || true
fi

if command -v update-alternatives &>/dev/null; then
    sudo update-alternatives --install /usr/bin/x-terminal-emulator x-terminal-emulator /usr/local/bin/azterm 60 2>/dev/null || true
    sudo update-alternatives --set x-terminal-emulator /usr/local/bin/azterm 2>/dev/null || true
fi

# GNOME settings & xdg-terminals preference lists
mkdir -p "$USER_HOME/.config"
echo "azterm.desktop" > "$USER_HOME/.config/ubuntu-xdg-terminals.list" 2>/dev/null || true
echo "azterm.desktop" > "$USER_HOME/.config/xdg-terminals.list" 2>/dev/null || true
gsettings set org.gnome.desktop.default-applications.terminal exec '/usr/local/bin/azterm' 2>/dev/null || true

echo "[5/5] Refreshing system caches and restarting file manager..."
sudo update-desktop-database -q /usr/share/applications 2>/dev/null || true
update-desktop-database -q "$USER_HOME/.local/share/applications" 2>/dev/null || true
sudo gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true

# Terminate running instances so the changes reload immediately
killall -9 nautilus 2>/dev/null || true
killall -9 gnome-terminal-server 2>/dev/null || true
hash -r 2>/dev/null || true

echo "=========================================================="
echo " AZTerm updated successfully on Debian/Ubuntu!"
echo " All terminal launch hooks have been redirected to azterm."
echo "=========================================================="