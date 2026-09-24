# Changelog

All notable changes to AZTerm are documented here. This project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
