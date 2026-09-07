#!/usr/bin/env bash
TARGET_DIR="${NAUTILUS_SCRIPT_CURRENT_URI:-$1}"
if [ -n "$TARGET_DIR" ]; then
    # Strip file:// prefix if present
    CLEAN_DIR="${TARGET_DIR#file://}"
    azterm -d "$CLEAN_DIR"
else
    azterm
fi
