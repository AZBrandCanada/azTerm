#!/usr/bin/env bash
set -euo pipefail

FILE="src/terminal.rs"

if [ ! -f "$FILE" ]; then
    echo "Error: $FILE not found. Run this from the project root."
    exit 1
fi

python3 - "$FILE" << 'PY_EOF'
import sys

path = sys.argv[1]
with open(path, "r") as f:
    src = f.read()

old = """        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                widget_id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: false,
                    vertical_arrows: false,
                    escape: false,
                },
            );
        });"""

new = """        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                widget_id,
                egui::EventFilter {
                    // Lock ALL focus-navigation keys to this terminal widget
                    // so egui never steals them. Previously only `tab` was
                    // locked; arrows and Escape were free, so pressing arrow
                    // keys moved keyboard focus to the navbar / tab bar and
                    // nano, vim, htop, less, etc. never received the escape
                    // sequences they need.
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });"""

if old not in src:
    print("ERROR: could not locate the focus_lock_filter block.")
    print("The file may already be patched, or differs from expected.")
    sys.exit(1)

src = src.replace(old, new, 1)

with open(path, "w") as f:
    f.write(src)

print("Patched", path)
PY_EOF

echo "Done. Now run:  cargo build --release"
