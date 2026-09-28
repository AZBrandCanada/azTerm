#!/usr/bin/env bash
set -euo pipefail

FILE="src/main.rs"
[ -f "$FILE" ] || { echo "error: run from project root"; exit 1; }

python3 - "$FILE" << 'PY_EOF'
import sys

path = sys.argv[1]
with open(path) as f:
    src = f.read()

old = """#[derive(PartialEq, Eq, Clone, Copy)]
pub enum ActiveView {
    Terminal,
    SshBookmarks,
    SftpBrowser,
    Settings,
}""" 

new = """#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ActiveView {
    Terminal,
    SshBookmarks,
    SftpBrowser,
    Settings,
}"""

assert old in src, "ActiveView enum anchor not found"
src = src.replace(old, new, 1)

with open(path, "w") as f:
    f.write(src)

print("Patched", path)
PY_EOF

echo "Now rebuild:  cargo build --release"
