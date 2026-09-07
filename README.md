# AZTerm

AZTerm is a fast, lightweight, and cross-platform native terminal emulator, SSH bookmark manager, and dual-session SFTP client written in pure Rust.

Built with hardware-accelerated immediate-mode GPU graphics, AZTerm provides a fluid, responsive interface with zero Electron or Chromium web overhead, maintaining an ultra-low memory footprint (~20 MB to 35 MB RAM) and sub-30ms startup times.

---

## Direct Downloads (Latest Releases)

Download pre-compiled binaries for your operating system:

* **Universal Linux AppImage:** [Download AZTerm-x86_64.AppImage](https://github.com/AZBrandCanada/azTerm/releases/latest/download/AZTerm-x86_64.AppImage)
* **Debian / Ubuntu Package:** [Download azterm_0.1.0_amd64.deb](https://github.com/AZBrandCanada/azTerm/releases/latest/download/azterm_0.1.0_amd64.deb)
* **Generic Linux Tarball:** [Download azterm-linux-x86_64.tar.gz](https://github.com/AZBrandCanada/azTerm/releases/latest/download/azterm-linux-x86_64.tar.gz)
* **Windows 64-bit Archive:** [Download azterm-windows-x86_64.zip](https://github.com/AZBrandCanada/azTerm/releases/latest/download/azterm-windows-x86_64.zip)
* **macOS Universal Package:** [Download azterm-macos-universal.tar.gz](https://github.com/AZBrandCanada/azTerm/releases/latest/download/azterm-macos-universal.tar.gz)

To view all versions and changelogs, visit the [AZTerm Releases Page](https://github.com/AZBrandCanada/azTerm/releases).

---

## One-Line Quick Install

Clone and install AZTerm with full desktop integration in a single command:

```bash
git clone https://github.com/AZBrandCanada/azTerm.git && cd azTerm && ./install.sh
```

---

## Key Features

### 1. High-Performance Native Terminal Engine
* **Pure Rust & GPU-Accelerated:** Rendered via OpenGL/Vulkan with zero web runtime latency.
* **Full VT100 / ANSI Support:** Complete color palette, cursor modes, and escape sequence parsing.
* **Copy on Select:** Dragging to select text visually highlights in cyan and immediately copies to the system clipboard upon release.
* **Right-Click Paste:** Right-click inside the terminal canvas to write clipboard text directly to the active shell.
* **Dynamic Grid Resizing:** Propagates window dimensions (SIGWINCH) to full-screen terminal applications like `htop`, `vim`, and `neovim`.
* **Adaptive Tab Sizing:** Top tab bar dynamically scales tab widths to fit your window, maintaining responsive navigation with overflow protection.

### 2. Advanced SSH & Keypair Manager
* **Built-in Ed25519 Key Generator:** Generate fresh SSH keypairs with one-click public key copying for pasting into remote `~/.ssh/authorized_keys`.
* **Inline Key Pasting:** Paste OpenSSH private keys directly in the UI with automated secure permission enforcement (`chmod 0600`) in `~/.config/azterm/keys/`.
* **Profile Management:** Organize servers with custom ports, usernames, identity files, and group tags.

### 3. Non-Blocking 2FA & Authentication Prompts
* **Smart Prompt Interception:** Automatically identifies remote server verification requests (Google Authenticator, Duo, YubiKey, OTP, passwords, key passphrases).
* **Focused Dialog:** Displays a centered input modal with auto-focus and Enter-to-submit handling.
* **Non-Blocking Architecture:** Interacting with authentication prompts does not lock other tabs or workspace panels.

### 4. Dual-Session SFTP File Explorer
* **Live SSH Multiplexing (`ControlMaster`):** The SFTP engine shares your authenticated terminal connection, eliminating duplicate logins, password re-entry, and permission errors.
* **Dual-Pane Transfer Interface:** Simultaneously browse two targets (Local <-> Remote or Session <-> Session) with one-click Upload and Download actions.
* **Live Split-View Drawer:** Toggle the SFTP drawer directly in the bottom status bar to view your remote server's filesystem side-by-side with your active shell.

### 5. SQLite Workspace & Session Persistence
* **State Preservation:** Open tabs, working directories, active SSH profiles, and preferences are automatically stored in `~/.config/azterm/azterm.db`.
* **Seamless Restoration:** Relaunching AZTerm automatically restores your previous workspace state.

### 6. Native Desktop & OS Integration
* **File Manager Context Menus:** Right-click any folder in KDE Dolphin, GNOME Nautilus, or Nemo to choose **Open in AZTerm**.
* **URI Protocol Handlers:** Registers `ssh://` and `sftp://` URI schemes with the operating system for instant connection launching.
* **Desktop App Identity:** Native launcher and taskbar integration across KDE Plasma, GNOME, XFCE, and tiling window managers (Hyprland, Sway, i3, Rofi, Wofi).

---

## Installation by Distribution

### Arch Linux / CachyOS / Manjaro
```bash
git clone https://github.com/AZBrandCanada/azTerm.git
cd azTerm
./install-arch.sh
```

### Ubuntu / Debian / Linux Mint / Pop!_OS
```bash
git clone https://github.com/AZBrandCanada/azTerm.git
cd azTerm
./install-debian.sh
```

---

## Command Line Usage

AZTerm supports standard terminal CLI parameters:

```bash
# Open default shell / restore previous sessions
azterm

# Open AZTerm directly in a specific directory
azterm /var/log
azterm -d /home/user/projects

# Connect directly to an SSH host
azterm ssh://root@192.168.1.100:22

# Launch directly into the SFTP file manager
azterm --sftp

# Execute a command in a new terminal session
azterm -e htop
```

---

## Multi-Platform Packaging

To generate all distribution formats directly from your machine into the `dist/` directory:

```bash
./package-all.sh
```

Outputs generated:
* `dist/AZTerm-x86_64.AppImage` (Universal Linux standalone executable)
* `dist/azterm-0.1.0-1-x86_64.pkg.tar.zst` (Arch Linux native package)
* `dist/azterm_0.1.0_amd64.deb` (Debian / Ubuntu package)
* `dist/azterm-linux-x86_64.tar.gz` (Generic Linux archive)
* `dist/azterm-windows-x86_64.zip` (Windows 64-bit executable archive)

---

## Uninstallation

To remove AZTerm, its desktop entries, and file manager context menus from your system:

```bash
./uninstall.sh
```

To also remove all saved SQLite configuration history and session data:
```bash
rm -rf ~/.config/azterm
```

---

## License

Dual-licensed under either the MIT License or the Apache License (Version 2.0).
