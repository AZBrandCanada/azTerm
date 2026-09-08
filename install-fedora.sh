#!/usr/bin/env bash
set -e

BUILD_DIR="$HOME/.cache/azterm-build"
REPO_URL="https://github.com/AZBrandCanada/azTerm.git"

if [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
fi
export PATH="$HOME/.cargo/bin:$PATH"

echo "[1/5] Checking Fedora system dependencies..."
sudo dnf install -y git gcc gcc-c++ make pkgconf-pkg-config libxkbcommon-devel openssl-devel libxcb-devel libX11-devel wayland-devel mesa-libGL-devel

if ! command -v cargo &>/dev/null; then
    sudo dnf install -y rust cargo || {
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
        export PATH="$HOME/.cargo/bin:$PATH"
    }
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

echo "[4/5] Self-healing and installing to all detected PATH locations..."
sudo mkdir -p /usr/local/bin /usr/share/applications /usr/share/icons/hicolor/scalable/apps
sudo install -Dm755 target/release/azterm /usr/local/bin/azterm

declare -A SEEN_LOCS
LOCATIONS=(
    "/usr/local/bin/azterm"
    "/usr/bin/azterm"
    "$HOME/.local/bin/azterm"
    "$HOME/.cargo/bin/azterm"
    "$HOME/bin/azterm"
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
        if [ -e "$loc" ] || [ -L "$loc" ] || [ "$loc" = "$HOME/.local/bin/azterm" ]; then
            dir_name=$(dirname "$loc")
            if [ -w "$dir_name" ]; then
                install -Dm755 target/release/azterm "$loc" 2>/dev/null || true
            else
                sudo install -Dm755 target/release/azterm "$loc" 2>/dev/null || true
            fi
        fi
    fi
done

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
sudo update-desktop-database -q /usr/share/applications || true
update-desktop-database -q "$HOME/.local/share/applications" 2>/dev/null || true
sudo gtk-update-icon-cache -q /usr/share/icons/hicolor || true

hash -r 2>/dev/null || true

echo "=========================================================="
echo " AZTerm installation on Fedora complete!"
echo " Active binary: $(which azterm)"
echo "=========================================================="
