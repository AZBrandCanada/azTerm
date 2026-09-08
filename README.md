# AZTerm

AZTerm is a fast, lightweight, and cross-platform native terminal emulator, SSH bookmark manager, dual-session SFTP client, and tiling workspace manager written in pure Rust.

Built with hardware-accelerated immediate-mode GPU graphics, AZTerm provides a fluid, responsive interface with zero Electron or Chromium web overhead, maintaining an ultra-low memory footprint (~20 MB to 45 MB RAM) and sub-30ms startup times.

---

## One-Line Install (Pipe to Bash)

Run this single command in your terminal to automatically download, compile, and install AZTerm with full desktop integration:

```bash
curl -sSL https://raw.githubusercontent.com/AZBrandCanada/azTerm/main/install.sh | bash
```

Or using `wget`:

```bash
wget -qO- https://raw.githubusercontent.com/AZBrandCanada/azTerm/main/install.sh | bash
```

---

## Direct Downloads (Precompiled Releases)

Download pre-compiled standalone release binaries:

* **Universal Linux AppImage:** [Download AZTerm-x86_64.AppImage](https://github.com/AZBrandCanada/azTerm/releases/latest/download/AZTerm-x86_64.AppImage)
* **Debian / Ubuntu Package:** [Download azterm_0.1.6_amd64.deb](https://github.com/AZBrandCanada/azTerm/releases/latest/download/azterm_0.1.6_amd64.deb)
* **Generic Linux Tarball:** [Download azterm-linux-x86_64.tar.gz](https://github.com/AZBrandCanada/azTerm/releases/latest/download/azterm-linux-x86_64.tar.gz)
* **Windows 64-bit Archive:** [Download azterm-windows-x86_64.zip](https://github.com/AZBrandCanada/azTerm/releases/latest/download/azterm-windows-x86_64.zip)
* **macOS Universal Package:** [Download azterm-macos-universal.tar.gz](https://github.com/AZBrandCanada/azTerm/releases/latest/download/azterm-macos-universal.tar.gz)

To view all versions and changelogs, visit the [AZTerm Releases Page](https://github.com/AZBrandCanada/azTerm/releases).

---

## Key Features

### 1. High-Performance Native Terminal Engine
* **Pure Rust & GPU-Accelerated:** Rendered with direct hardware acceleration and zero web runtime latency.
* **Deep Scrollback History:** Up to 10,000 lines of scrollback history per session with smooth mouse wheel scrolling, interactive scrollbar, and Shift+PageUp/PageDown navigation.
* **Smart Progress Bar & Unicode Handling:** Correct handling of wide characters and carriage returns (`\r`) prevents system update logs (`pacman`, `apt`, `cargo`, `dnf`) from squishing onto a single line.
* **Copy on Select & Right-Click Paste:** Highlighting text automatically copies it to the system clipboard upon release; right-clicking writes clipboard contents directly into the active prompt.
* **Persistent Selection Across Scrollback:** Highlighting text anchors to absolute buffer line coordinates, allowing selections to persist and follow the text as you scroll.
* **Persistent Zoom Level:** Scale the entire UI and terminal font dynamically with `Ctrl + +`, `Ctrl + -`, `Ctrl + 0`, or `Ctrl + MouseWheel`. Your chosen zoom level is saved and restored on startup.

### 2. Interactive Tiling & Split Panes
* **Instant Splits:** Split any active pane side-by-side (`Split |`) or stacked (`Split -`) using top bar controls or hotkeys (`Ctrl+Shift+D` / `Ctrl+Shift+E`).
* **Visual Drag-and-Drop Docking:** Drag any pane by its title bar (`::`) or any tab header onto another pane's dock zones (Left, Right, Top, Bottom) with live snap-preview highlights.
* **Draggable Dividers:** Freely resize width and height ratios between tiled panes by dragging the divider with the mouse.
* **Slim In-Pane Control Bar:** Each tiled pane features an in-pane strip showing its title, active focus indicator, split shortcuts, full-pane maximize (`Max`), pop-out to separate tab (`Pop`), and close (`X`).
* **Auto-Hiding Tab Line:** When working in a single-pane tab, the second-row tab bar auto-hides to maximize vertical screen space, reappearing as soon as multiple tabs or splits exist.
* **Quad Grid Layout:** Arrange multiple tabs into an even 2x2 grid with a single click.

### 3. Comprehensive Theme Engine & Transparency
* **8 Built-in Theme Presets:** Cyber Cyan (Default), Dracula, Nord, Tokyo Night, One Dark, Monokai Pro, Matrix Green, and Solarized Dark.
* **Custom Theme Creator:** Duplicate any preset, edit all UI elements with live color pickers, and create your own themes.
* **Full 16-Color ANSI Terminal Palette:** Customize standard and bright ANSI colors directly in Settings so command line utilities (`ls`, `htop`, syntax highlighters) match your theme.
* **Adjustable Window Transparency:** Control background opacity from 20% to 100% with a real-time slider.
* **OS Native vs. Custom Window Bar:** Switch between native OS window manager decorations and AZTerm's integrated title bar featuring draggable top areas, double-click maximize, and 8-zone edge/corner resizing.

### 4. Advanced SSH & Keypair Manager
* **Built-in Ed25519 Key Generator:** Generate SSH keypairs with one-click public key copying for quick addition to remote `~/.ssh/authorized_keys`.
* **Inline Key Pasting & Secure Permissions:** Paste OpenSSH private keys directly into profile dialogs with automatic `chmod 0600` enforcement in `~/.config/azterm/keys/`.
* **Profile Management:** Organize servers with custom ports, usernames, identity files, and group tags.

### 5. Dual-Session SFTP File Explorer
* **SSH Connection Multiplexing (`ControlMaster`):** The SFTP engine shares your authenticated terminal connection, eliminating redundant logins, password re-entry, and permission errors.
* **Dual-Pane Transfer Interface:** Simultaneously browse two targets (Local <-> Remote or Session <-> Session) with one-click Upload and Download actions.
* **Live Split-View Drawer:** Toggle the SFTP drawer in the bottom status bar to view your remote server's filesystem side-by-side with your live shell.

### 6. SQLite Workspace & Session Persistence
* **State Preservation:** Open tabs, tiled layouts, split ratios, active profiles, themes, zoom levels, and settings are saved automatically to `~/.config/azterm/azterm.db`.
* **Seamless Restoration:** Relaunching AZTerm restores your previous workspace state and tabs.

### 7. Native Desktop & OS Integration
* **File Manager Context Menus:** Right-click any folder or background in **KDE Dolphin**, **GNOME Nautilus**, or **Nemo** to select **Open in AZTerm Here**.
* **URI Protocol Handlers:** Registers `ssh://` and `sftp://` URI schemes with your desktop environment.
* **Desktop Launcher:** Full `.desktop` and scalable vector icon integration across KDE Plasma 6/5, GNOME, XFCE, Hyprland, Sway, and i3.

---

## Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| **`Ctrl + Shift + D`** | Split active pane horizontally (side-by-side) |
| **`Ctrl + Shift + E`** | Split active pane vertically (stacked) |
| **`Ctrl + Shift + M`** | Toggle maximize/restore active pane |
| **`Ctrl + Shift + W`** | Close focused pane |
| **`Alt + Arrow Keys`** | Switch focus between tiled panes |
| **`Ctrl` + `+` / `Ctrl` + `=`** | Zoom in (+10%) |
| **`Ctrl` + `-`** | Zoom out (-10%) |
| **`Ctrl` + `0`** | Reset zoom to 100% |
| **`Ctrl` + Mouse Wheel** | Zoom in / Zoom out |
| **`Shift + PageUp`** | Scroll terminal history up |
| **`Shift + PageDown`** | Scroll terminal history down |
| **`Shift + Home`** | Jump to oldest scrollback history |
| **`Shift + End`** | Snap back to live prompt |
| **`Ctrl + Shift + C`** | Copy selected text |
| **`Ctrl + Shift + V`** | Paste from clipboard |

---

## Manual Installation by Distribution

### Arch Linux / CachyOS / Manjaro
```bash
git clone https://github.com/AZBrandCanada/azTerm.git
cd azTerm
./install-arch.sh
```

### Fedora
```bash
git clone https://github.com/AZBrandCanada/azTerm.git
cd azTerm
./install-fedora.sh
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
# Open default shell / restore previous workspace
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

To generate all distribution packages into the `dist/` directory:

```bash
./package-all.sh
```

Outputs generated:
* `dist/AZTerm-x86_64.AppImage` (Universal Linux standalone binary)
* `dist/azterm-0.1.6-1-x86_64.pkg.tar.zst` (Arch Linux native package)
* `dist/azterm_0.1.6_amd64.deb` (Debian / Ubuntu package)
* `dist/azterm-linux-x86_64.tar.gz` (Generic Linux archive)
* `dist/azterm-windows-x86_64.zip` (Windows 64-bit executable archive)

---

## Uninstallation

To remove AZTerm, its desktop launchers, and file manager context menus from your system:

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
