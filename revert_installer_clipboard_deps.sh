#!/usr/bin/env bash
set -euo pipefail
python3 - << 'PY_EOF'
import re
files = ["install.sh", "install-arch.sh", "install-debian.sh", "install-fedora.sh"]
for path in files:
    try:
        with open(path) as f:
            src = f.read()
    except FileNotFoundError:
        continue
    # Remove trailing " wl-clipboard xclip" (or the apt-style line-wrap
    # variant) that the earlier patch added.
    src = src.replace(" wl-clipboard xclip\n", "\n")
    src = src.replace(" \\\n    wl-clipboard xclip\n", "\n")
    with open(path, "w") as f:
        f.write(src)
    print("reverted", path)
PY_EOF
  