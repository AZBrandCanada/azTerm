# Changelog

All notable changes to AZTerm are documented here. This project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.3.0] - 2026-09-24

### Added
- **Debug logging mode** with a configurable log path (Settings →
  Application & System). Writes timestamped traces of PTY, SSH, SFTP,
  TUI wheel, resize and selection events. Rotates at 20 MB to `.old`.
  A panic hook appends the panic message and a full backtrace to the
  same file, so crashes are captured even when the terminal never
  returns to the shell.
- **Mouse Wheel Scroll Amount** setting (Settings → Terminal
  Interaction). Choose 1 / 3 / 5 / 10 lines, Half page, or Full page
  per wheel notch. Page sizes adapt live to the terminal height.
  Persisted in SQLite alongside the rest of the settings.
- **Ctrl+Shift+A** in a terminal selects the entire scrollback buffer;
  **Ctrl+Shift+C** copies the selection with a line-count toast.

### Fixed
- **Pasted shell scripts no longer corrupt their formatting.** Ctrl+V
  was sending both egui's `Event::Paste` and a raw `0x16` (SYN /
  readline `quoted-insert`) byte to the PTY, which swallowed the next
  character as a literal. Bracketed paste and normal paste now arrive
  cleanly.
- **Random UI freezes during active SSH sessions.** The pty writer was
  a synchronous `Arc<Mutex<Box<dyn Write>>>` on the UI thread. When
  SSH's stdin buffer backed up (slow network, stalled remote process,
  dead ControlMaster), `write_all` blocked and froze the app. Writes
  now go through a dedicated writer thread with a bounded channel;
  the UI never blocks on the pty.
- **SFTP split-view path field could not receive keyboard input.** The
  terminal was auto-requesting egui focus every frame whenever no
  widget held it, starving the SFTP path `TextEdit`. Focus is now
  handed off exactly once per session change.
- **btop / top rendered ghost rows when opening.** Entering
  alt-screen flipped scrollbar visibility, which changed the pty
  column count and fired SIGWINCH mid-startup. The full-screen paint
  raced the resize and left stale cells. Scrollbar width is now
  always reserved in the column math; entering or leaving alt-screen
  no longer changes the terminal size.
- **Shell startup banners (zsh fastfetch / p10k) were missing on new
  tabs.** An unconditional parser-clear on the initial 40x120 → real
  size resize was wiping the banner before it could render. The clear
  now only runs for alt-screen / detected TUIs.
- **Nano / less / vim / htop could not be scrolled with the mouse
  wheel.** In alt-screen or a detected primary-screen TUI, wheel
  events are translated to PageUp / PageDown. If the app enables
  xterm mouse mode, real SGR mouse events are forwarded instead.
- **Drag-select past the pane edge did not autoscroll in nano.**
  Wayland's `hover_pos` returns `None` while dragging outside the
  widget, so the edge check never fired. Pointer position now falls
  through `latest_pos → hover_pos → interact_pos`.
- **Multi-screen drag-copy from full-screen apps.** When you drag or
  wheel past the edge while a TUI owns the screen, AZTerm now
  snapshots each redraw and stitches the frames together at release
  using the longest suffix/prefix overlap. A single drag from nano
  can copy hundreds of lines even though the visual highlight only
  spans one frame.
- **SSH auth modal could consume unbounded memory.** Server output is
  capped at 64 KB; older text is trimmed on a UTF-8 boundary.
- **Stale-socket cleanup and sudo verification blocked the UI
  thread.** Both now run on worker threads, so a slow or dead SSH
  connection can't freeze the app while probing.
- **Ctrl+Shift+PageUp / Ctrl+Shift+PageDown / Ctrl+Shift+Home /
  Ctrl+Shift+End** now scroll the terminal scrollback by a full page
  and jump to the oldest / newest retained line.

### Changed
- `SftpSudoPrompt` no longer derives `Clone` (it holds a
  `Receiver<bool>` for async password verification).
- `TerminalSession::widget_id` provides a stable egui widget id for
  focus tracking.
- `terminal.rs` render loop uses `self.rows` / `self.cols` instead of
  vt100's reported size, eliminating a class of grid-size mismatch
  panics.

### Removed
- Unused `AtomicBool` / `Ordering` import in `sftp.rs`.
- Unused `safe_parent` / `safe_folder` bindings in the local→remote
  folder upload path.

## [0.2.9] - 2026-09-24

### Fixed
- **Paste no longer corrupts formatting in scripts.** Previously, pasting via
  `Ctrl+V` sent both egui's `Event::Paste` *and* a raw `0x16` (SYN /
  readline `quoted-insert`) byte to the PTY, which caused readline to swallow
  the next character as a literal. Multi-line shell scripts and here-docs now
  paste intact and execute correctly.
- **Random UI freezes during active SSH sessions.** The pty writer was a
  synchronous `Arc<Mutex<Box<dyn Write>>>` held by the UI thread — when SSH's
  stdin buffer backed up (slow network, stalled remote process, dead
  ControlMaster), `write_all` blocked and froze the whole app. Writes now go
  through a dedicated writer thread with a bounded channel; the UI never
  blocks on the pty.
- **SFTP path field in the split-view drawer accepted no keyboard input.**
  The terminal was auto-requesting egui focus every frame whenever no widget
  held it, which starved the SFTP path `TextEdit` in the side-by-side
  layout. Focus is now handed off exactly once per session change and is no
  longer stolen automatically.
- **Nano / less / vim could not be scrolled with the mouse wheel.** In the
  alternate screen with no mouse-tracking mode active, wheel events are now
  translated into Up/Down arrow keystrokes (matching xterm behavior).
- **SSH auth modal could consume unbounded memory.** Server output is now
  capped at 64 KB; older text is trimmed from the front on a UTF-8 boundary.
- **Stale-socket cleanup blocked the UI thread.** `ssh -O check` calls that
  could stall up to 10 s on a dead `ControlMaster` are now dispatched to a
  worker thread when opening a session, closing a session, and opening the
  SSH auth modal.
- **Sudo password verification blocked the UI thread while holding the
  prompt mutex.** Verification now runs on a worker thread; the button
  shows a "Verifying credentials..." state and the modal remains responsive.

### Changed
- Terminal focus is now tracked via a stable widget id
  (`TerminalSession::widget_id`) and requested once per session switch,
  rather than re-asserted every frame.
- The `SftpSudoPrompt` struct no longer derives `Clone` (it now holds a
  `Receiver<bool>` for async verification).

### Removed
- Dead `AtomicBool` / `Ordering` import in `sftp.rs`.
- Unused `safe_parent` / `safe_folder` bindings in the local→remote folder
  upload path.

## [0.2.8] - Earlier

- Initial public preview release.
