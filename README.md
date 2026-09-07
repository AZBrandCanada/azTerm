# AZTerm

A fast, modern native cross-platform terminal, SSH bookmark manager, and SFTP client written in Rust.

## Features
- **Local & Remote Terminals:** Native pseudo-terminal handling with full ANSI/VT100 support.
- **SSH Profiles & Key Management:**
  - Built-in Ed25519 key generator with public key copying.
  - Paste inline private keys or specify existing key files with automatic `chmod 0600` permissions.
- **2FA & OTP Auto-Detection:** Automatically triggers an interactive OTP submission bar when keywords match.
- **SFTP Explorer:** Integrated directory explorer with file navigation and terminal split-view.
- **Full Settings Suite:** Customizable cursor blink, backspace sequence, log paths, right-click actions, and UI toggles.

## Prerequisites (Arch Linux / CachyOS)
    sudo pacman -S --needed base-devel rust libxkbcommon openssl libxcb libx11 wayland mesa

## Running AZTerm
    cargo run --release
