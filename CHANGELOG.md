# Changelog

All notable changes to AZTerm are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0] - 2026-10-07

This release replaces the terminal emulation engine and hardens session
reattachment. TUIs that previously rendered incorrectly (btop, helix,
neovim's floating windows, anything using Braille graphs or Nerd Font
icons) now behave correctly, and mouse input works across the full set
of xterm mouse protocols.

### Changed

- **Terminal emulation engine: `vt100` → `azterm-parser`.**
  The parser, grid, scrollback ring, and cell model are now handled by
  the same VT state machine that powers Alacritty. This fixes a long
  list of rendering artifacts that were not fixable on top of `vt100`:

  - **btop** now renders its CPU/GPU/mem/net Braille graphs at correct
    cell width and alignment. Previously the Braille block glyphs were
    dropped or misaligned, and columns drifted out of sync with the
    data cells.
  - **vim / neovim** floating windows, `:terminal` splits, and status
    line rendering are correct.
  - **helix, micro, mc, lazygit** and other TUIs that use newer escape
    sequences (CSI Z backtab, DECSCUSR cursor shape, synchronized
    output mode 2026) now render without cursor drift.
  - Wide-character (CJK) and combining-character cell widths are
    correct.
  - Underline, inverse, and wide-char-spacer flags are preserved
    through the render pipeline.

  The engine is vendored as **`azterm-parser`**, a fork of
  `alacritty_terminal` published under the AZBrand organisation. This
  pins the parser against upstream churn so a future Alacritty release
  cannot silently break AZTerm's rendering.

- **Terminal font is now embedded into the binary.**
  `JetBrainsMono Nerd Font Regular` is compiled in via `include_bytes!`
  and registered as the primary Monospace family. This guarantees that
  Braille glyphs (U+2800–U+28FF), box-drawing characters, Powerline
  separators, and the Nerd Font icon set render identically regardless
  of how AZTerm is installed (AppImage, `.deb`, Flatpak, `cargo
  install`, or manual build). No system font dependency, no `fc-list`
  lookup, no "works on my machine". The `default_mono_font_path()`
  filesystem search has been removed. Terminal font size is still
  user-configurable; terminal font *family* selection remains locked
  to the embedded face, for the same reason as before (arbitrary
  monospace fonts break cell alignment and copy/paste geometry).

### Fixed

- **Mouse input in btop (and any TUI using xterm mode 1002).**
  The terminal previously checked only for `MOUSE_REPORT_CLICK` (mode
  1000) and `MOUSE_MOTION` (mode 1003) when deciding whether to
  forward mouse events to the child process. Mode 1002 (`MOUSE_DRAG`)
  is a superset of 1000 — it reports press, release, *and* motion
  while a button is held — and is what btop, neovim with `mouse=a`,
  and helix enable. Because 1002 wasn't in the check, AZTerm treated
  those sessions as text-selection-only and btop's top-bar buttons
  (including the refresh-interval `+` / `-` control) never received
  clicks. All three call sites — the render-time `has_mouse` check,
  the `send_mouse_event` guard, and the `owns_scrollback` check used
  for keyboard routing — now accept mode 1002.

- **Mouse motion events are now forwarded to TUIs.**
  `btop` and other SGR-mode TUIs need to see pointer motion over a
  widget *before* a click lands, so they can highlight the target
  element and route the click correctly. AZTerm now emits `CSI <35;
  C; R M` (motion, no button) whenever the pointer crosses into a new
  grid cell — throttled by cell, not pixel, to avoid flooding the
  PTY. When a button is held during motion, the held-button motion
  code (`CSI <32; C; R M`) is sent instead, matching what xterm and
  Alacritty emit.

- **Session reattach restores terminal modes.**
  The daemon now observes DEC private mode toggles as raw PTY bytes
  pass through it (alt-screen 1049/1047/47, application cursor 1,
  application keypad 66, mouse 1000/1002/1003/1006, bracketed paste
  2004, cursor visibility 25) and replays a short mode preamble to
  every new client *before* the replay buffer. Previously, a long-
  running TUI like btop would enable alt-screen and mouse reporting
  once at startup, then never re-send them — and once the replay
  buffer's rolling window trimmed those initial bytes off the front,
  reattaching to the session produced a `Term` with the wrong mode
  bits. Reattached btop sessions would render but ignore clicks, and
  arrow keys would escape to the GUI instead of the app. Now the
  daemon sends `\x1b[?1049h \x1b[?1002h \x1b[?1006h` (whatever is
  currently active) as a preamble, so a fresh `Term` starts in the
  correct state regardless of how much of the buffer was trimmed.

  The daemon wire protocol was bumped to `PROTO_VERSION = 4` to
  force a clean restart of any pre-existing daemon on first launch
  after upgrade. Live sessions on the old daemon are terminated once
  and only once; thereafter the new daemon takes over.

- **Parser panic recovery is now safe.**
  If the VT parser panics mid-sequence (which `catch_unwind` catches
  but does not repair), AZTerm now rebuilds the `Term`, resets the
  parser, and sends `Ctrl+L` to the child process so curses
  applications redraw from a clean slate. Previously the parser could
  be left in a corrupt state and every subsequent byte was misparsed,
  producing the classic "random characters appear everywhere"
  symptom.

### Removed

- The `default_mono_font_path()` system-font search and the associated
  fallback chain of hardcoded `/usr/share/fonts/...` paths.
- The `vt100` dependency and the CBT-rewrite shim
  (`process_bytes_with_cbt`), which is no longer needed —
  `azterm-parser` handles CSI Z correctly.

### Notes for packagers

- `Cargo.toml` now depends on `azterm-parser = "0.1"` (published to
  crates.io) instead of `vt100 = "0.15"`.
- Two `.ttf` files (`JetBrainsMonoNerdFont-Regular.ttf`, ~2.5 MB) are
  embedded via `include_bytes!` and will be compiled into the release
  binary. This increases the binary size by roughly the font's
  compressed contribution (~1.3 MB after LTO / strip).
- No new runtime system dependencies. Nerd Font, Braille, and icon
  rendering no longer require any font to be installed on the host.

[0.6.0]: https://github.com/AZBrandCanada/azTerm/releases/tag/v0.6.0
