// src/terminal.rs
use crate::settings::{AppSettings, BackspaceSequence};
use crate::theme::*;
use eframe::egui;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtyPair, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

// -- Clipboard worker thread ------------------------------------------------
//
// All arboard operations are serialised on this single background thread.
// This guarantees:
//   * only one arboard::Clipboard object exists per process (the Wayland
//     data-control backend refuses multiple simultaneous owners);
//   * the object is kept alive after a write so the selection is retained;
//   * a hung arboard call can never freeze the UI thread — reads use a
//     hard timeout, writes are fire-and-forget.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Hard upper bound on a single clipboard read. If arboard does not
/// respond within this window (e.g. the compositor is unresponsive), the
/// call returns `None` and the UI thread moves on.
const CLIPBOARD_TIMEOUT: Duration = Duration::from_millis(600);

pub enum ClipboardMsg {
    Write(String),
    Read(std::sync::mpsc::Sender<Option<String>>),
}

fn clipboard_tx() -> &'static std::sync::mpsc::Sender<ClipboardMsg> {
    static TX: OnceLock<std::sync::mpsc::Sender<ClipboardMsg>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<ClipboardMsg>();
        std::thread::Builder::new()
            .name("azterm-clipboard".to_string())
            .spawn(move || {
                let mut cb: Option<arboard::Clipboard> = None;
                while let Ok(msg) = rx.recv() {
                    match msg {
                        ClipboardMsg::Write(text) => {
                            if cb.is_none() {
                                cb = arboard::Clipboard::new().ok();
                            }
                            let mut ok = false;
                            if let Some(ref mut c) = cb {
                                if c.set_text(&text).is_ok() {
                                    ok = true;
                                }
                            }
                            if !ok {
                                // Handle went bad; recreate once and retry.
                                cb = arboard::Clipboard::new().ok();
                                if let Some(ref mut c) = cb {
                                    let _ = c.set_text(&text);
                                }
                            }
                        }
                        ClipboardMsg::Read(reply) => {
                            if cb.is_none() {
                                cb = arboard::Clipboard::new().ok();
                            }
                            let text = cb.as_mut().and_then(|c| c.get_text().ok());
                            let _ = reply.send(text);
                        }
                    }
                }
            })
            .expect("spawn clipboard worker");
        tx
    })
}
// ---------------------------------------------------------------------------


/// Messages sent to the dedicated pty-writer thread. Kept pub because it
/// appears in a pub field on TerminalSession.
pub enum WriterMsg {
    Data(Vec<u8>),
}

pub fn get_system_clipboard_text() -> Option<String> {
    // All arboard work is serialised on a single background thread with a
    // hard timeout. If the Wayland compositor is unresponsive and arboard
    // blocks inside get_text(), the worker thread stalls but the UI thread
    // returns None after CLIPBOARD_TIMEOUT and carries on.
    let (tx, rx) = std::sync::mpsc::channel();
    if clipboard_tx().send(ClipboardMsg::Read(tx)).is_err() {
        return None;
    }
    match rx.recv_timeout(CLIPBOARD_TIMEOUT) {
        Ok(t) => t.filter(|s| !s.is_empty()),
        Err(_) => {
            crate::dbg_log!(
                "clipboard_read timeout after {:?} — arboard did not respond",
                CLIPBOARD_TIMEOUT
            );
            None
        }
    }
}

pub fn set_system_clipboard_text(ctx: Option<&egui::Context>, text: &str) {
    if text.is_empty() {
        return;
    }
    // Mirror into egui's in-app clipboard so TextEdit widgets can paste
    // even if the OS clipboard write below fails.
    if let Some(c) = ctx {
        c.copy_text(text.to_string());
    }
    // Fire-and-forget. The worker thread holds the Clipboard object so the
    // OS selection stays owned for as long as AZTerm runs. Callers don't
    // wait for a result — a Ctrl+C should feel instant.
    let _ = clipboard_tx().send(ClipboardMsg::Write(text.to_string()));
}

/// Maximum raw PTY bytes retained per session for restore-on-reopen.
/// When exceeded, the front of the buffer is trimmed up to the next ESC
/// so we never start replaying mid-escape-sequence.
const HISTORY_MAX: usize = 1024 * 1024;

fn vt_to_egui_color(color: vt100::Color, is_bg: bool, theme: &ThemeConfig) -> egui::Color32 {
    match color {
        vt100::Color::Default => {
            if is_bg {
                theme.bg_main_color()
            } else {
                theme.text_primary_color()
            }
        }
        vt100::Color::Idx(idx) => theme.ansi_color(idx),
        vt100::Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
    }
}

#[derive(Debug, Clone)]
pub enum SessionType {
    Local { working_dir: String },
    Ssh { profile_id: String },
}

/// Stitch captured TUI frames into a single string. Adjacent frames
/// overlap (nano pages ~half-screen), so we find the longest suffix/prefix
/// match between consecutive frames and append only the new tail.
fn combine_tui_frames(frames: Vec<Vec<String>>, drag_up: bool) -> String {
    if frames.is_empty() {
        return String::new();
    }
    if frames.len() == 1 {
        return frames.into_iter().next().unwrap().join("\n");
    }
    let mut ordered = frames;
    if drag_up {
        // Frames were captured newest-first (we paged upward); reverse so
        // older content lands at the top.
        ordered.reverse();
    }
    let mut result: Vec<String> = Vec::new();
    for frame in ordered {
        if result.is_empty() {
            result = frame;
            continue;
        }
        let max_check = result.len().min(frame.len()).min(120);
        let mut overlap = 0usize;
        for k in (1..=max_check).rev() {
            if result[result.len() - k..] == frame[..k] {
                overlap = k;
                break;
            }
        }
        result.extend_from_slice(&frame[overlap..]);
    }
    result.join("\n")
}

pub struct TerminalSession {
    pub id: usize,
    pub title: String,
    pub session_type: SessionType,
    pub parser: vt100::Parser,
    pub rx: Receiver<Vec<u8>>,
    /// Channel to the dedicated pty-writer thread. Kept pub for parity with
    /// the old `writer` field; callers should use send_input/send_paste.
    pub writer_tx: SyncSender<WriterMsg>,
    /// Local PTY master handle. None in daemon mode (the daemon owns the
    /// real PTY; the GUI only sees a byte stream).
    pub master_pty: Option<Arc<Mutex<Box<dyn MasterPty + Send>>>>,
    /// Daemon-mode resize sink. Some when this session is daemon-backed.
    /// Resize events are forwarded over IPC instead of hitting a local PTY.
    pub daemon_resize_tx: Option<SyncSender<(u16, u16)>>,
    pub child_pid: Option<u32>,
    pub current_dir: Option<String>,
    pub last_detected_dir: Option<String>,
    pub rows: u16,
    pub cols: u16,
    pub scroll_offset: usize,
    pub max_scroll: usize,
    pub scrollback_limit: usize,

    pub selection_start: Option<(i64, u16)>,
    pub selection_end: Option<(i64, u16)>,
    pub is_dragging_selection: bool,
    /// Throttles page-key emission when drag-selecting past the edge of
    /// a full-screen program (nano, less, vim, htop, ...). Prevents
    /// flooding the PTY with dozens of PageUp/PageDown per second.
    pub alt_drag_page_cooldown: Option<std::time::Instant>,
    /// Screens snapshotted during a TUI drag where the user paged the app
    /// (by edge-drag OR wheel-while-holding). At release, these frames are
    /// stitched together so a single drag can copy multiple screens.
    pub tui_drag_frames: Vec<Vec<String>>,
    /// Last snapshot; used to detect when the app actually redrew.
    pub tui_drag_last_snapshot: Vec<String>,
    /// None until the user pages; true = paged up (older content), false = down.
    pub tui_drag_direction: Option<bool>,

    /// Raw PTY output bytes captured for restore-on-reopen. Bounded ring
    /// trimmed at HISTORY_MAX, aligned to the next ESC.
    pub recovery_note: Option<(String, (u8, u8, u8))>,

    pub history_buf: Vec<u8>,
    /// Set true whenever history_buf grows; the caller clears it after
    /// writing to disk. Lets persist_sessions skip unchanged sessions.
    pub history_dirty: bool,

    /// Set true by the PTY reader thread when it observes EOF — i.e.
    /// the shell, SSH connection, or child process has exited. The UI
    /// uses this to offer a one-click reconnect instead of forcing the
    /// user to close and re-open the whole tab.
    pub is_dead: std::sync::Arc<AtomicBool>,
    /// Flipped to true when the user interacts with a dead session
    /// (types any key, or clicks the Reconnect overlay button).
    /// `render_single_pane` checks this after `render()` returns and
    /// emits `PaneAction::Reconnect` for the app layer to handle.
    pub reconnect_requested: bool,
}

/// Backtab (CBT, CSI Z) is silently ignored by vt100 0.15.
///
/// Nano, vim, htop, less and friends use CBT to jump the cursor to the
/// previous tab stop, which can be several columns away. When the parser
/// drops it, its cursor state diverges from the program's and every later
/// *relative* cursor move (BS, CSI C, Tab, another CBT) inherits the
/// error. That is the mechanism behind the "cursor jumps then writes land
/// on the wrong cell" corruption in nano.
///
/// This wrapper scans each chunk for CBT and rewrites every occurrence
/// into the equivalent absolute cursor-left move computed from the
/// parser's live column. Non-CBT bytes are forwarded verbatim, so escape
/// sequences that legitimately span chunk boundaries still work.
fn process_bytes_with_cbt(parser: &mut vt100::Parser, bytes: &[u8]) {
    // Fast path: no CBT anywhere in this chunk.
    if bytes.len() < 3 || !bytes.windows(3).any(|w| w == b"\x1b[Z") {
        parser.process(bytes);
        return;
    }

    // Standard xterm-256color tab stops: every 8 columns.
    const TAB_SIZE: u16 = 8;

    let mut i = 0usize;
    while i < bytes.len() {
        let rel = match bytes[i..].windows(3).position(|w| w == b"\x1b[Z") {
            Some(r) => r,
            None => {
                if i < bytes.len() {
                    parser.process(&bytes[i..]);
                }
                return;
            }
        };
        let cbt_at = i + rel;

        // Feed everything before the CBT normally.
        if cbt_at > i {
            parser.process(&bytes[i..cbt_at]);
        }

        // Translate CBT -> CSI {n} D. From column N, CBT moves to the
        // previous tab stop strictly less than N.
        let (_, col) = parser.screen().cursor_position();
        let prev_tab = if col == 0 {
            0
        } else {
            ((col - 1) / TAB_SIZE) * TAB_SIZE
        };
        let distance = col.saturating_sub(prev_tab);
        if distance > 0 {
            let seq = format!("\x1b[{}D", distance);
            parser.process(seq.as_bytes());
        }

        i = cbt_at + 3;
    }
}

impl TerminalSession {
    /// Stable widget id used for keyboard focus tracking.
    pub fn widget_id(session_id: usize) -> egui::Id {
        egui::Id::new(("azterm_terminal_pane", session_id))
    }

    pub fn new(
        id: usize,
        title: String,
        session_type: SessionType,
        cmd: CommandBuilder,
        ctx: egui::Context,
        scrollback_len: usize,
    ) -> Self {
        let rows = 40;
        let cols = 120;

        let pty_system = native_pty_system();
        let pair: PtyPair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("Failed to open PTY");

        let child = pair.slave.spawn_command(cmd).expect("Failed to spawn shell");
        let child_pid = child.process_id();

        let mut reader = pair
            .master
            .try_clone_reader()
            .expect("Failed to clone PTY reader");

        let mut writer = pair
            .master
            .take_writer()
            .expect("Failed to take PTY writer");

        let master_pty = Arc::new(Mutex::new(pair.master));

        let (tx, rx): (SyncSender<Vec<u8>>, Receiver<Vec<u8>>) = sync_channel(512);
        // Writer channel: deliberately large. send_input uses
        // try_send from the UI thread (must never block), so a full
        // channel would silently drop keystrokes. 64K pending
        // messages is far beyond any realistic backpressure; even a
        // completely wedged downstream PTY would need hundreds of
        // thousands of keystrokes to fill it.
        let (writer_tx, writer_rx): (SyncSender<WriterMsg>, Receiver<WriterMsg>) =
            sync_channel(65536);

        let is_dead = std::sync::Arc::new(AtomicBool::new(false));

        // Dedicated writer thread. The UI never blocks on pty writes — it
        // just hands bytes to this thread. Prevents UI freezes when SSH's
        // stdin buffer backs up on a stalled connection.
        thread::spawn(move || {
            while let Ok(msg) = writer_rx.recv() {
                match msg {
                    WriterMsg::Data(bytes) => {
                        if writer.write_all(&bytes).is_err() {
                            break;
                        }
                        let _ = writer.flush();
                    }
                }
            }
        });

        {
            let is_dead = is_dead.clone();
            thread::spawn(move || {
                let mut buf = [0u8; 16384];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
                is_dead.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            });
        }

        let initial_dir = match &session_type {
            SessionType::Local { working_dir } => Some(working_dir.clone()),
            SessionType::Ssh { .. } => None,
        };

        Self {
            id,
            title,
            session_type,
            parser: vt100::Parser::new(rows, cols, scrollback_len.max(1000)),
            rx,
            writer_tx,
            master_pty: Some(master_pty),
            daemon_resize_tx: None,
            child_pid,
            current_dir: initial_dir.clone(),
            last_detected_dir: initial_dir,
            rows,
            cols,
            scroll_offset: 0,
            max_scroll: 0,
            scrollback_limit: scrollback_len.max(1000),
            selection_start: None,
            selection_end: None,
            is_dragging_selection: false,
            alt_drag_page_cooldown: None,
            tui_drag_frames: Vec::new(),
            tui_drag_last_snapshot: Vec::new(),
            tui_drag_direction: None,
            history_buf: Vec::new(),
            history_dirty: false,
            recovery_note: None,
            is_dead,
            reconnect_requested: false,
        }
    }

    /// True if this session's PTY lives in the azterm-daemon process.
    pub fn is_daemon(&self) -> bool {
        self.daemon_resize_tx.is_some()
    }

    /// Build a TerminalSession that reads from and writes to the daemon
    /// instead of owning a local PTY. Returns None if the attach handshake
    /// fails (daemon died, session was killed between List and Attach).
    pub fn new_daemon(
        id: usize,
        title: String,
        session_type: SessionType,
        daemon: Arc<crate::daemon_client::DaemonClient>,
        ctx: egui::Context,
        scrollback_len: usize,
        initial_cols: u16,
        initial_rows: u16,
    ) -> Option<Self> {
        let attach = match daemon.attach(id as u64, initial_cols, initial_rows) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("[terminal] daemon attach failed for id={}: {}", id, e);
                return None;
            }
        };

        let rows = initial_rows;
        let cols = initial_cols;

        let (tx, rx): (SyncSender<Vec<u8>>, Receiver<Vec<u8>>) = sync_channel(512);

        let is_dead = std::sync::Arc::new(AtomicBool::new(false));

        {
            let tx = tx.clone();
            let out_rx = attach.out_rx;
            let ctx2 = ctx.clone();
            let is_dead = is_dead.clone();
            thread::spawn(move || {
                while let Ok(bytes) = out_rx.recv() {
                    if bytes.is_empty() {
                        // EOF sentinel from client reader.
                        break;
                    }
                    if tx.send(bytes).is_err() {
                        break;
                    }
                    ctx2.request_repaint();
                }
                is_dead.store(true, Ordering::Relaxed);
                ctx2.request_repaint();
            });
        }

        // Same rationale as the local-writer channel above: large
        // enough that UI-thread try_send never realistically drops.
        let (writer_tx, writer_rx): (SyncSender<WriterMsg>, Receiver<WriterMsg>) =
            sync_channel(65536);
        {
            let in_tx = attach.in_tx;
            thread::spawn(move || {
                while let Ok(msg) = writer_rx.recv() {
                    match msg {
                        WriterMsg::Data(bytes) => {
                            if in_tx.send(bytes).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }

        let initial_dir = match &session_type {
            SessionType::Local { working_dir } => Some(working_dir.clone()),
            SessionType::Ssh { .. } => None,
        };

        Some(Self {
            id,
            title,
            session_type,
            parser: vt100::Parser::new(rows, cols, scrollback_len.max(1000)),
            rx,
            writer_tx,
            master_pty: None,
            daemon_resize_tx: Some(attach.resize_tx),
            child_pid: None,
            current_dir: initial_dir.clone(),
            last_detected_dir: initial_dir,
            rows,
            cols,
            scroll_offset: 0,
            max_scroll: 0,
            scrollback_limit: scrollback_len.max(1000),
            selection_start: None,
            selection_end: None,
            is_dragging_selection: false,
            alt_drag_page_cooldown: None,
            tui_drag_frames: Vec::new(),
            tui_drag_last_snapshot: Vec::new(),
            tui_drag_direction: None,
            history_buf: Vec::new(),
            history_dirty: false,
            recovery_note: None,
            is_dead,
            reconnect_requested: false,
        })
    }

    /// True once the child process on the other end of this PTY has
    /// exited and the reader thread has seen EOF. The UI uses this to
    /// show a Reconnect overlay and to short-circuit keyboard input
    /// into a reconnect request.
    pub fn session_ended(&self) -> bool {
        self.is_dead.load(Ordering::Relaxed)
    }

    pub fn detect_current_working_dir(&self, ssh_user: Option<&str>) -> Option<String> {
        if let Some(ref d) = self.current_dir {
            if d.starts_with('/') && !d.contains("file:") {
                return Some(d.clone());
            }
        }

        #[cfg(target_os = "linux")]
        {
            if matches!(self.session_type, SessionType::Local { .. }) {
                if let Some(pid) = self.child_pid {
                    if let Ok(link) = std::fs::read_link(format!("/proc/{}/cwd", pid)) {
                        return Some(link.to_string_lossy().to_string());
                    }
                }
            }
        }

        let screen = self.parser.screen();
        let title = screen.title();
        if !title.is_empty() {
            if let Some(dir) = Self::parse_dir_from_str(title, ssh_user) {
                return Some(dir);
            }
        }

        let (cursor_r, _) = screen.cursor_position();
        let (rows, cols) = screen.size();
        let start_r = cursor_r.min(rows.saturating_sub(1));
        let min_r = start_r.saturating_sub(4);

        for r in (min_r..=start_r).rev() {
            let mut line_text = String::with_capacity(cols as usize);
            for c in 0..cols {
                if let Some(cell) = screen.cell(r, c) {
                    let contents = cell.contents();
                    if contents.is_empty() {
                        line_text.push(' ');
                    } else {
                        line_text.push_str(&contents);
                    }
                }
            }
            let trimmed = line_text.trim();
            if !trimmed.is_empty() {
                if let Some(dir) = Self::parse_dir_from_str(trimmed, ssh_user) {
                    return Some(dir);
                }
            }
        }

        None
    }

    fn parse_dir_from_str(text: &str, ssh_user: Option<&str>) -> Option<String> {
        let clean = text.trim();
        if clean.is_empty() {
            return None;
        }

        let candidate_path = if let Some(idx) = clean.find(':') {
            let after_colon = &clean[idx + 1..];
            let path_part = after_colon.trim_start();
            let end_idx = path_part
                .find(|c| c == '$' || c == '#' || c == '%' || c == ' ' || c == '\n')
                .unwrap_or(path_part.len());
            path_part[..end_idx].trim()
        } else if let Some(idx) = clean.find("] ") {
            let after = &clean[idx + 2..];
            let end_idx = after
                .find(|c| c == '$' || c == '#' || c == '%')
                .unwrap_or(after.len());
            after[..end_idx].trim()
        } else {
            return None;
        };

        if candidate_path.is_empty() {
            return None;
        }

        let username = ssh_user.unwrap_or("root");
        let home_dir = if username == "root" {
            "/root".to_string()
        } else {
            format!("/home/{}", username)
        };

        let resolved = if candidate_path == "~" {
            home_dir
        } else if let Some(stripped) = candidate_path.strip_prefix("~/") {
            format!(
                "{}/{}",
                home_dir.trim_end_matches('/'),
                stripped.trim_matches('/')
            )
        } else if candidate_path.starts_with('/') {
            candidate_path.to_string()
        } else {
            return None;
        };

        if resolved.contains(' ') || resolved.contains("&&") || resolved.contains('|') {
            return None;
        }

        Some(resolved)
    }

    /// Replay persisted raw PTY bytes into the parser, then roll every
    /// visible row into scrollback and clear the visible screen so the
    /// new shell's prompt lands at the top-left of a clean pane. All
    /// historical output stays reachable via the scroll wheel.
    pub fn feed_restore_history(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.parser.process(bytes);
        }));

        // Emit enough line-feeds to push every visible row into the
        // scrollback ring, then clear the visible screen and home the
        // cursor. 2*rows is safely more than the cursor can ever need.
        let mut push = Vec::with_capacity(self.rows as usize * 2 + 8);
        for _ in 0..(self.rows as usize * 2) {
            push.push(b'\n');
        }
        push.extend_from_slice(b"\x1b[2J\x1b[H");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.parser.process(&push);
        }));

        self.scroll_offset = 0;
        self.max_scroll = 0;
        self.parser.set_scrollback(0);
    }

    pub fn clear_screen_and_scrollback(&mut self) {
        self.parser = vt100::Parser::new(self.rows, self.cols, self.scrollback_limit);
        self.scroll_offset = 0;
        self.max_scroll = 0;
        self.selection_start = None;
        self.selection_end = None;
    }

    pub fn query_max_scrollback(&mut self) -> usize {
        let current = self.parser.screen().scrollback();
        self.parser.set_scrollback(usize::MAX);
        let max = self.parser.screen().scrollback();
        self.parser.set_scrollback(current);
        max
    }

    pub fn set_view_scroll(&mut self, target: usize) {
        if self.parser.screen().alternate_screen() {
            self.scroll_offset = 0;
            self.parser.set_scrollback(0);
            return;
        }

        self.max_scroll = self.query_max_scrollback();
        let clamped = target.min(self.max_scroll);
        self.scroll_offset = clamped;
        self.parser.set_scrollback(clamped);
    }

    /// Heuristic: does this look like a full-screen TUI (nano, htop, mc,
    /// emacs -nw, ...) running in the *primary* screen?
    ///
    /// Signal: does the app write content BELOW the cursor? Shells write
    /// sequentially, so the cursor is always at the last written cell and
    /// everything below is blank. TUIs position the cursor and draw status
    /// bars / panels below it, so there's usually non-blank content further
    /// down the screen.
    fn looks_like_primary_screen_tui(&self) -> bool {
        let screen = self.parser.screen();
        if screen.alternate_screen() {
            return false;
        }
        let (cursor_r, _) = screen.cursor_position();
        let (rows, cols) = screen.size();
        if cursor_r >= rows.saturating_sub(1) {
            return false;
        }
        for r in (cursor_r + 1)..rows {
            for c in 0..cols {
                if let Some(cell) = screen.cell(r, c) {
                    let contents = cell.contents();
                    if !contents.is_empty() && contents != " " {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Snapshot the current visible rows as trimmed strings.
    /// Used to capture TUI frames while drag-scrolling nano / htop / etc.
    pub fn snapshot_visible_lines(&self) -> Vec<String> {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let mut out = Vec::with_capacity(rows as usize);
        for r in 0..rows {
            let mut line = String::with_capacity(cols as usize);
            for c in 0..cols {
                if let Some(cell) = screen.cell(r, c) {
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let t = cell.contents();
                    if t.is_empty() {
                        line.push(' ');
                    } else {
                        line.push_str(&t);
                    }
                }
            }
            out.push(line.trim_end().to_string());
        }
        out
    }

    pub fn send_input(&mut self, text: &str) {
        if self.recovery_note.is_some() { self.recovery_note = None; }
        if self.scroll_offset > 0 && !self.parser.screen().alternate_screen() {
            self.set_view_scroll(0);
        }
        crate::dbg_log!(
            "pty_send_input id={} bytes={} hex={:02x?} preview={:?}",
            self.id,
            text.len(),
            text.as_bytes(),
            &text.chars().take(48).collect::<String>()
        );
        let _ = self
            .writer_tx
            .try_send(WriterMsg::Data(text.as_bytes().to_vec()));
    }

    pub fn send_mouse_event(
        &mut self,
        button: u8,
        is_release: bool,
        col: u16,
        row: u16,
        modifiers: egui::Modifiers,
    ) {
        let mode = self.parser.screen().mouse_protocol_mode();
        if mode == vt100::MouseProtocolMode::None {
            return;
        }

        let encoding = self.parser.screen().mouse_protocol_encoding();
        let c = col.saturating_add(1).min(self.cols);
        let r = row.saturating_add(1).min(self.rows);

        let mut btn = button;
        if modifiers.shift {
            btn = btn.saturating_add(4);
        }
        if modifiers.alt {
            btn = btn.saturating_add(8);
        }
        if modifiers.ctrl {
            btn = btn.saturating_add(16);
        }

        match encoding {
            vt100::MouseProtocolEncoding::Sgr => {
                let term = if is_release { 'm' } else { 'M' };
                let seq = format!("\x1b[<{};{};{}{}", btn, c, r, term);
                self.send_input(&seq);
            }
            _ => {
                let code = if is_release { 3 } else { btn };
                let cb = 32u8.saturating_add(code);
                let cx = (32u16.saturating_add(c)).min(255) as u8;
                let cy = (32u16.saturating_add(r)).min(255) as u8;
                let _ = self.writer_tx.try_send(WriterMsg::Data(vec![
                    b'\x1b', b'[', b'M', cb, cx, cy,
                ]));
            }
        }
    }

    pub fn send_paste(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.scroll_offset > 0 {
            self.set_view_scroll(0);
        }

        let bracketed = self.parser.screen().bracketed_paste();
        let payload = if bracketed {
            let sanitized = text.replace('\x1b', "");
            let normalized = sanitized.replace("\r\n", "\n").replace('\r', "\n");
            format!("\x1b[200~{}\x1b[201~", normalized)
        } else {
            text.replace("\r\n", "\n").replace('\r', "\n")
        };

        crate::dbg_log!(
            "pty_send_paste id={} bytes={} bracketed={}",
            self.id,
            payload.len(),
            bracketed
        );
        let _ = self
            .writer_tx
            .try_send(WriterMsg::Data(payload.into_bytes()));
    }

    pub fn poll_updates(&mut self) {
        let mut total_bytes = 0;
        let in_alt = self.parser.screen().alternate_screen();

        let old_max = if !in_alt && self.scroll_offset > 0 {
            self.query_max_scrollback()
        } else {
            0
        };

        while let Ok(bytes) = self.rx.try_recv() {
            total_bytes += bytes.len();

            // Log the head of every PTY chunk. We cap the hex dump at
            // 64 bytes so a full-screen redraw doesn't flood the log;
            // for a single arrow-key tap the whole chunk fits.
            crate::dbg_log!(
                "pty_recv id={} n={} hex={:02x?}",
                self.id,
                bytes.len(),
                &bytes[..bytes.len().min(64)]
            );

            if !in_alt && bytes.windows(4).any(|w| w == b"\x1b[3J") {
                self.clear_screen_and_scrollback();
            }

            if let Some(idx) = bytes.windows(9).position(|w| w == b"\x1b]7;file:") {
                let rest = &bytes[idx + 9..];
                let end_idx = rest.iter().position(|&b| b == 0x07 || b == 0x1b);
                if let Some(end) = end_idx {
                    if let Ok(s) = std::str::from_utf8(&rest[..end]) {
                        let path_candidate = if let Some(stripped) = s.strip_prefix("//") {
                            if let Some(slash_idx) = stripped.find('/') {
                                &stripped[slash_idx..]
                            } else {
                                stripped
                            }
                        } else {
                            s
                        };
                        if !path_candidate.is_empty() {
                            self.current_dir = Some(path_candidate.replace("%20", " "));
                        }
                    }
                }
            }

            // Parser panic handling. `catch_unwind` catches but does NOT
            // restore the parser's internal state — if vt100 panics
            // halfway through a byte sequence, the parser is left in a
            // corrupt state and every subsequent byte is misparsed.
            // That is the classic "random characters appear everywhere"
            // symptom. Log the offending bytes AND reset the parser so
            // the terminal is usable again instead of stuck corrupt.
            let parse_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                process_bytes_with_cbt(&mut self.parser, &bytes);
            }));
            {
                let screen = self.parser.screen();
                let (cr, cc) = screen.cursor_position();
                let at = screen
                    .cell(cr, cc)
                    .map(|c| c.contents().to_string())
                    .unwrap_or_else(|| "<none>".into());
                crate::dbg_log!(
                    "cursor_after id={} pos=(r{},c{}) at={:?}",
                    self.id, cr, cc, at
                );
            }
            if parse_result.is_err() {
                crate::dbg_log!(
                    "PARSER_PANIC id={} bytes={:02x?} — resetting parser",
                    self.id,
                    &bytes[..bytes.len().min(128)]
                );
                self.parser = vt100::Parser::new(
                    self.rows,
                    self.cols,
                    self.scrollback_limit,
                );
                // Ask the app to repaint. The freshly-created parser is
                // blank, but nano/vim/htop still think their previous
                // screen state is live — without a full repaint every
                // later incremental write lands in the wrong cell.
                // Ctrl+L (0x0C) is the universal "redraw screen" key
                // for curses programs.
                let _ = self.writer_tx.try_send(WriterMsg::Data(vec![0x0c]));
            }

            // Mirror into the restore-history buffer. Same loop iteration
            // as parser.process, so the two stay in sync even if the
            // 256 KB break below kicks in (bytes leftover stay in the
            // channel for the next frame).
            self.history_buf.extend_from_slice(&bytes);
            if self.history_buf.len() > HISTORY_MAX {
                let target = self.history_buf.len() - HISTORY_MAX;
                let mut cut = target;
                while cut < self.history_buf.len() && self.history_buf[cut] != 0x1b {
                    cut += 1;
                }
                if cut < self.history_buf.len() {
                    self.history_buf.drain(..cut);
                }
            }
            self.history_dirty = true;

            if total_bytes > 262_144 {
                break;
            }
        }

        if total_bytes > 0 {
            crate::dbg_log!("pty_poll id={} bytes={}", self.id, total_bytes);
        }

        if in_alt {
            self.scroll_offset = 0;
            self.parser.set_scrollback(0);
        } else if self.scroll_offset > 0 {
            let new_max = self.query_max_scrollback();
            if new_max > old_max {
                let added = new_max - old_max;
                self.scroll_offset = (self.scroll_offset + added).min(new_max);
                self.parser.set_scrollback(self.scroll_offset);
            }
            self.max_scroll = new_max;
        }
    }

    fn is_cell_selected(&self, line_age: i64, c: u16) -> bool {
        if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
            if start == end {
                return false;
            }
            let (top_age, top_col, bot_age, bot_col) =
                if start.0 > end.0 || (start.0 == end.0 && start.1 <= end.1) {
                    (start.0, start.1, end.0, end.1)
                } else {
                    (end.0, end.1, start.0, start.1)
                };

            if line_age > top_age || line_age < bot_age {
                return false;
            }
            if line_age == top_age && line_age == bot_age {
                let (c1, c2) = if top_col <= bot_col {
                    (top_col, bot_col)
                } else {
                    (bot_col, top_col)
                };
                return c >= c1 && c <= c2;
            }
            if line_age == top_age {
                return c >= top_col;
            }
            if line_age == bot_age {
                return c <= bot_col;
            }
            true
        } else {
            false
        }
    }

    fn find_word_bounds(&self, r: u16, c: u16) -> Option<(u16, u16)> {
        let screen = self.parser.screen();
        let cols = self.cols;
        if c >= cols {
            return None;
        }

        let is_word_char = |col: u16| -> bool {
            if let Some(cell) = screen.cell(r, col) {
                let text = cell.contents();
                if let Some(ch) = text.chars().next() {
                    return ch.is_alphanumeric()
                        || ch == '_'
                        || ch == '-'
                        || ch == '.'
                        || ch == '/'
                        || ch == ':';
                }
            }
            false
        };

        if !is_word_char(c) {
            return Some((c, c));
        }

        let mut start_c = c;
        while start_c > 0 && is_word_char(start_c - 1) {
            start_c -= 1;
        }

        let mut end_c = c;
        while end_c + 1 < cols && is_word_char(end_c + 1) {
            end_c += 1;
        }

        Some((start_c, end_c))
    }

    fn extract_selected_text(&mut self) -> String {
        let mut result = String::new();
        if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
            let (top_age, top_col, bot_age, bot_col) =
                if start.0 > end.0 || (start.0 == end.0 && start.1 <= end.1) {
                    (start.0, start.1, end.0, end.1)
                } else {
                    (end.0, end.1, start.0, start.1)
                };

            let saved_offset = self.scroll_offset;

            for age in (bot_age..=top_age).rev() {
                let target_r = (self.rows as i64 - 1) + self.scroll_offset as i64 - age;

                let (screen_r, temp_offset): (u16, usize) =
                    if target_r >= 0 && target_r < self.rows as i64 {
                        (target_r as u16, self.scroll_offset)
                    } else if target_r < 0 {
                        let needed_offset =
                            (self.scroll_offset as i64 - target_r).max(0) as usize;
                        (0u16, needed_offset)
                    } else {
                        let diff = target_r - (self.rows as i64 - 1);
                        let needed_offset =
                            (self.scroll_offset as i64 - diff).max(0) as usize;
                        (self.rows.saturating_sub(1), needed_offset)
                    };

                self.parser.set_scrollback(temp_offset);
                let screen = self.parser.screen();

                let start_c = if age == top_age { top_col } else { 0 };
                let end_c = if age == bot_age {
                    bot_col
                } else {
                    self.cols.saturating_sub(1)
                };

                let mut line = String::new();
                for c in start_c..=end_c {
                    if let Some(cell) = screen.cell(screen_r, c) {
                        if cell.is_wide_continuation() {
                            continue;
                        }
                        let text = cell.contents();
                        if text.is_empty() {
                            line.push(' ');
                        } else {
                            line.push_str(&text);
                        }
                    }
                }
                result.push_str(line.trim_end());
                if age != bot_age {
                    result.push('\n');
                }
            }

            self.parser.set_scrollback(saved_offset);
        }
        result
    }

    fn handle_keyboard_events(
        &mut self,
        ctx: &egui::Context,
        settings: &AppSettings,
        toast: &mut Option<(String, std::time::Instant)>,
    ) {
        // Dead-session short circuit: any key press requests a
        // reconnect. Matches the muscle memory of just hitting Enter
        // to retry, without the user having to close and re-open the
        // whole tab.
        if self.session_ended() {
            let any_key = ctx.input(|i| {
                i.events.iter().any(|e| matches!(
                    e,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::Text(_)
                        | egui::Event::Paste(_)
                ))
            });
            if any_key {
                self.reconnect_requested = true;
            }
            return;
        }

        // If egui already produced an Event::Paste this frame, do NOT also send
        // the raw 0x16 (Ctrl+V / readline quoted-insert) — that double-input
        // corrupts pasted scripts and leaves readline in a weird state.
        let has_paste_event = ctx.input(|i| {
            i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))
        });

        // Any text that needs to end up on the system clipboard is queued
        // here and flushed AFTER the ctx.input(...) closure below returns.
        //
        // Why: egui::Context::input() holds egui's internal parking_lot
        // RwLock for the duration of the closure. ctx.copy_text() tries
        // to acquire the SAME write lock on the SAME thread, which
        // parking_lot does not permit. The process deadlocks and is
        // aborted (SIGABRT). Collecting the payload here and calling
        // set_system_clipboard_text once outside the closure avoids the
        // reentrant lock entirely.
        let mut pending_clipboard: Option<String> = None;

        ctx.input(|i| {
            if i.modifiers.ctrl && !i.modifiers.shift && !i.modifiers.alt {
                if i.key_pressed(egui::Key::C) {
                    self.send_input("\x03");
                    return;
                }
                if i.key_pressed(egui::Key::X) {
                    self.send_input("\x18");
                    return;
                }
                if i.key_pressed(egui::Key::U) {
                    self.send_input("\x15");
                    return;
                }
                if i.key_pressed(egui::Key::K) {
                    self.send_input("\x0b");
                    return;
                }
                if i.key_pressed(egui::Key::L) {
                    if !self.parser.screen().alternate_screen() {
                        self.clear_screen_and_scrollback();
                    } else {
                        self.scroll_offset = 0;
                    }
                    self.send_input("\x0c");
                    return;
                }
                if i.key_pressed(egui::Key::D) {
                    self.send_input("\x04");
                    return;
                }
                if i.key_pressed(egui::Key::Z) {
                    self.send_input("\x1a");
                    return;
                }
                if i.key_pressed(egui::Key::A) {
                    self.send_input("\x01");
                    return;
                }
                if i.key_pressed(egui::Key::E) {
                    self.send_input("\x05");
                    return;
                }
                if i.key_pressed(egui::Key::W) {
                    self.send_input("\x17");
                    return;
                }
            }

            for event in &i.events {
                match event {
                    egui::Event::Copy => {
                        let selected = self.extract_selected_text();
                        if !selected.is_empty() {
                            // Deferred — see pending_clipboard above.
                            pending_clipboard = Some(selected);
                        } else {
                            self.send_input("\x03");
                        }
                    }
                    egui::Event::Cut => {
                        self.send_input("\x18");
                    }
                    egui::Event::Paste(text) => {
                        self.send_paste(text);
                    }
                    egui::Event::Text(text) => {
                        if !i.modifiers.ctrl && !i.modifiers.command && !i.modifiers.alt {
                            self.send_input(text);
                        }
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        repeat,
                        ..
                    } => {
                        // Log EVERY key event, not just the "interesting"
                        // ones. If a single tap is producing two events
                        // (one from keydown, one from a bogus
                        // Text-with-modifier path), this is where we'll
                        // see it.
                        crate::dbg_log!(
                            "kbd id={} key={:?} ctrl={} shift={} alt={} cmd={} repeat={}",
                            self.id,
                            key,
                            modifiers.ctrl,
                            modifiers.shift,
                            modifiers.alt,
                            modifiers.command,
                            repeat
                        );
                        if *key == egui::Key::Enter {
                            crate::dbg_log!(
                                "kbd_enter id={} ctrl={} shift={} alt={} cmd={}",
                                self.id,
                                modifiers.ctrl,
                                modifiers.shift,
                                modifiers.alt,
                                modifiers.command
                            );
                        }
                        if *key == egui::Key::Tab {
                            // Shell autocomplete. Shift+Tab sends the
                            // xterm "backtab" sequence (CSI Z), which zsh
                            // and readline map to reverse-menu-complete.
                            if modifiers.shift {
                                self.send_input("\x1b[Z");
                            } else {
                                self.send_input("\t");
                            }
                            continue;
                        }
                        if *key == egui::Key::Escape {
                            self.send_input("\x1b");
                            continue;
                        }

                        // Shift+PageUp/PageDown/Home/End scroll the
                        // terminal scrollback (not the child app) —
                        // but ONLY when we actually own the
                        // scrollback (i.e. not in alt-screen and not
                        // a primary-screen TUI that has its own
                        // scrolling). In nano/less/vim we must
                        // forward the raw key so the app handles it.
                        let owns_scrollback = !self.parser.screen().alternate_screen()
                            && self.parser.screen().mouse_protocol_mode()
                                == vt100::MouseProtocolMode::None;
                        if modifiers.shift && owns_scrollback {
                            if *key == egui::Key::PageUp {
                                let jump = (self.rows.saturating_sub(2) as usize).max(1);
                                self.set_view_scroll(self.scroll_offset + jump);
                                continue;
                            }
                            if *key == egui::Key::PageDown {
                                let jump = (self.rows.saturating_sub(2) as usize).max(1);
                                self.set_view_scroll(self.scroll_offset.saturating_sub(jump));
                                continue;
                            }
                            if *key == egui::Key::Home {
                                self.set_view_scroll(usize::MAX);
                                continue;
                            }
                            if *key == egui::Key::End {
                                self.set_view_scroll(0);
                                continue;
                            }
                        }

                        // ---- Terminal control keys, modifier-independent ----
                        //
                        // Arrow keys, Home/End, PageUp/Down, and Delete
                        // are terminal keys. egui can report stale
                        // modifier state on the frame a key event
                        // arrives (especially on Wayland), and the old
                        // code gated these sequences behind
                        // `!modifiers.ctrl`, which silently discarded
                        // the keypress when a stale ctrl was reported.
                        // Send the base sequence unconditionally; TUI
                        // apps interpret shift/ctrl variants the same
                        // way as the base for these keys anyway.
                        //
                        // The only exception is Ctrl+Shift+A/C/V,
                        // which the terminal widget uses for its own
                        // operations — those run first below.
                        let ctrl_shift_reserved = modifiers.ctrl && modifiers.shift
                            && matches!(key, egui::Key::A | egui::Key::C | egui::Key::V);
                        if !ctrl_shift_reserved {
                            let control_seq: Option<&[u8]> = match key {
                                egui::Key::ArrowUp => Some(b"\x1b[A"),
                                egui::Key::ArrowDown => Some(b"\x1b[B"),
                                egui::Key::ArrowRight => Some(b"\x1b[C"),
                                egui::Key::ArrowLeft => Some(b"\x1b[D"),
                                egui::Key::Home => Some(b"\x1b[H"),
                                egui::Key::End => Some(b"\x1b[F"),
                                egui::Key::PageUp => Some(b"\x1b[5~"),
                                egui::Key::PageDown => Some(b"\x1b[6~"),
                                egui::Key::Delete => Some(b"\x1b[3~"),
                                _ => None,
                            };
                            if let Some(seq) = control_seq {
                                crate::dbg_log!(
                                    "kbd_ctrl_seq id={} key={:?} ctrl={} shift={} alt={} bytes={:?}",
                                    self.id,
                                    key,
                                    modifiers.ctrl,
                                    modifiers.shift,
                                    modifiers.alt,
                                    seq
                                );
                                self.send_input(&String::from_utf8_lossy(seq));
                                continue;
                            }
                        }

                        if modifiers.ctrl && modifiers.shift && *key == egui::Key::A {
                            // Select the entire scrollback buffer + current screen.
                            let max = self.query_max_scrollback();
                            let top_age = (max + self.rows as usize - 1) as i64;
                            self.selection_start = Some((top_age, 0));
                            self.selection_end = Some((0i64, self.cols.saturating_sub(1)));
                            self.is_dragging_selection = false;
                            crate::dbg_log!(
                                "sel_all id={} scrollback={} rows={} top_age={}",
                                self.id,
                                max,
                                self.rows,
                                top_age
                            );
                            *toast = Some((
                                format!(
                                    "Selected {} line(s) of scrollback (Ctrl+Shift+C to copy)",
                                    max + self.rows as usize
                                ),
                                std::time::Instant::now(),
                            ));
                            continue;
                        }

                        if modifiers.ctrl && modifiers.shift && *key == egui::Key::C {
                            let selected = self.extract_selected_text();
                            if !selected.is_empty() {
                                // Deferred — see pending_clipboard above.
                                let line_count = selected.lines().count().max(1);
                                *toast = Some((
                                    format!("Copied {} line(s)", line_count),
                                    std::time::Instant::now(),
                                ));
                                pending_clipboard = Some(selected);
                            }
                            continue;
                        }

                        if (modifiers.shift && *key == egui::Key::Insert)
                            || (modifiers.ctrl
                                && modifiers.shift
                                && *key == egui::Key::V)
                        {
                            if let Some(clip) = get_system_clipboard_text() {
                                self.send_paste(&clip);
                            }
                            continue;
                        }

                        let mut bytes: Option<Vec<u8>> = None;
                        if modifiers.ctrl && !modifiers.shift {
                            let ctrl_byte = match key {
                                egui::Key::A => Some(1),
                                egui::Key::B => Some(2),
                                egui::Key::C => Some(3),
                                egui::Key::D => Some(4),
                                egui::Key::E => Some(5),
                                egui::Key::F => Some(6),
                                egui::Key::G => Some(7),
                                egui::Key::H => Some(8),
                                egui::Key::I => Some(9),
                                egui::Key::J => Some(10),
                                egui::Key::K => Some(11),
                                egui::Key::L => Some(12),
                                egui::Key::M => Some(13),
                                egui::Key::N => Some(14),
                                egui::Key::O => Some(15),
                                egui::Key::P => Some(16),
                                egui::Key::Q => Some(17),
                                egui::Key::R => Some(18),
                                egui::Key::S => Some(19),
                                egui::Key::T => Some(20),
                                egui::Key::U => Some(21),
                                egui::Key::V if !has_paste_event => Some(22),
                                egui::Key::W => Some(23),
                                egui::Key::X => Some(24),
                                egui::Key::Y => Some(25),
                                egui::Key::Z => Some(26),
                                _ => None,
                            };
                            if let Some(b) = ctrl_byte {
                                bytes = Some(vec![b]);
                            }
                        } else if !modifiers.ctrl {
                            bytes = match key {
                                egui::Key::Enter => Some(b"\r".to_vec()),
                                egui::Key::Backspace => match settings.backspace_sequence {
                                    BackspaceSequence::Delete127 => Some(b"\x7f".to_vec()),
                                    BackspaceSequence::Backspace8 => Some(b"\x08".to_vec()),
                                },
                                egui::Key::ArrowUp => Some(b"\x1b[A".to_vec()),
                                egui::Key::ArrowDown => Some(b"\x1b[B".to_vec()),
                                egui::Key::ArrowRight => Some(b"\x1b[C".to_vec()),
                                egui::Key::ArrowLeft => Some(b"\x1b[D".to_vec()),
                                egui::Key::Home => Some(b"\x1b[H".to_vec()),
                                egui::Key::End => Some(b"\x1b[F".to_vec()),
                                egui::Key::PageUp => Some(b"\x1b[5~".to_vec()),
                                egui::Key::PageDown => Some(b"\x1b[6~".to_vec()),
                                egui::Key::Delete => Some(b"\x1b[3~".to_vec()),
                                _ => None,
                            };
                        }

                        if let Some(b) = bytes {
                            self.send_input(&String::from_utf8_lossy(&b));
                        }
                    }
                    _ => {}
                }
            }
        });

        // Flush any clipboard copy queued while inside the ctx.input
        // closure above. Running it here — outside the closure — is what
        // avoids the reentrant lock on egui's Context that was
        // triggering SIGABRT crashes on Ctrl+C / Ctrl+Shift+C.
        if let Some(text) = pending_clipboard {
            set_system_clipboard_text(Some(ctx), &text);
        }
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        settings: &AppSettings,
        theme: &ThemeConfig,
        has_focus: bool,
        toast: &mut Option<(String, std::time::Instant)>,
    ) -> bool {
        let font_size = settings.terminal_font_size.clamp(6.0, 32.0);
        let font_id = egui::FontId::monospace(font_size);

        // Measure the per-cell advance width by laying out a sample row
        // using the exact same structure the render loop below uses: one
        // TextFormat run per cell, no wrapping. This is what egui's text
        // layout will actually do for the real row, so the mouse-to-cell
        // mapping stays aligned with the glyphs the user sees.
        //
        // The previous approach ("WWWWWWWWWW" laid out and divided by 10)
        // produced a per-cell width that disagreed with the real row
        // layout: the two paths round glyph advances differently, and the
        // error accumulated across the width of the terminal. That's why
        // the selection highlight drifted away from the cursor, and why
        // the drift got larger or smaller when font size or app zoom
        // changed.
        const SAMPLE_CELLS: usize = 100;
        let mut sample_job = egui::text::LayoutJob::default();
        sample_job.wrap.max_width = f32::INFINITY;
        for _ in 0..SAMPLE_CELLS {
            sample_job.append(
                "M",
                0.0,
                egui::TextFormat {
                    font_id: font_id.clone(),
                    color: egui::Color32::WHITE,
                    ..Default::default()
                },
            );
        }
        let sample_galley = ui.painter().layout_job(sample_job);
        let char_width = (sample_galley.size().x / SAMPLE_CELLS as f32).max(1.0);
        let row_height = (sample_galley.size().y * 1.05).max(1.0);

        let in_alternate = self.parser.screen().alternate_screen();
        let has_mouse =
            self.parser.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None;
        // Do NOT require alternate screen: nano, htop and emacs -nw sometimes
        // run in the primary screen but still enable xterm mouse reporting.
        let app_wants_mouse = has_mouse;
        // Detect full-screen TUIs in the primary screen (no mouse mode, not
        // in alternate screen) so we can route the wheel to the app instead
        // of the terminal scrollback.
        let tui_in_primary = !has_mouse && !in_alternate && self.looks_like_primary_screen_tui();
        // Scrollback bar is only drawn when we actually own the wheel.
        let show_scrollback_bar = !app_wants_mouse && !in_alternate && !tui_in_primary;
        // Always reserve scrollbar width for the *column math*, so entering
        // or leaving alt-screen (which toggles scrollbar visibility) does not
        // change the pty column count and trigger a spurious resize. btop,
        // htop, top and friends don't repaint cleanly when SIGWINCH arrives
        // mid-startup, which is what caused the double-render / ghost rows.
        // When the bar is hidden, the reserved strip is simply unused space.
        const SCROLLBAR_RESERVE: f32 = 12.0;
        let scrollbar_width = if show_scrollback_bar { SCROLLBAR_RESERVE } else { 0.0 };

        let avail = ui.available_size();
        let usable_w = (avail.x - SCROLLBAR_RESERVE).max(80.0);
        let usable_h = avail.y.max(40.0);
        // Minimums are deliberately low. If a pane is very narrow,
        // forcing the terminal to 20 cols makes its grid wider than
        // the pane itself, which used to overflow into neighbours
        // (clipping in tiling.rs now prevents that, but a smaller
        // minimum means the terminal actually fits its pane).
        let new_cols = ((usable_w / char_width).floor() as u16).max(4);
        let new_rows = ((usable_h / row_height).floor() as u16).max(2);

        if new_cols != self.cols || new_rows != self.rows {
            let old_rows = self.rows;
            let old_cols = self.cols;
            self.cols = new_cols;
            self.rows = new_rows;

            self.parser.set_size(new_rows, new_cols);

            // NOTE: we deliberately do NOT wipe the parser's visible grid
            // here. Wiping silently desyncs the running app's model of the
            // screen from ours: programs like nano, vim, htop keep their own
            // idea of what is on screen, and blanking the parser behind
            // their back means every later incremental write lands in the
            // wrong cell — the "random characters from elsewhere on the
            // line" bug. Modern full-screen apps redraw on SIGWINCH
            // themselves.

            if let Some(tx) = &self.daemon_resize_tx {
                let _ = tx.try_send((new_cols, new_rows));
            } else if let Some(master) = &self.master_pty {
                if let Ok(m) = master.lock() {
                    let _ = m.resize(PtySize {
                        rows: new_rows,
                        cols: new_cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
            }

            crate::dbg_log!(
                "term_resize id={} old={}x{} new={}x{} alt={} tui_primary={}",
                self.id,
                old_rows,
                old_cols,
                new_rows,
                new_cols,
                in_alternate,
                tui_in_primary
            );
        }

        let term_grid_size = egui::vec2(
            self.cols as f32 * char_width,
            self.rows as f32 * row_height,
        );
        let total_size = egui::vec2(term_grid_size.x + scrollbar_width, term_grid_size.y);

        let mut user_clicked_pane = false;

        let widget_id = Self::widget_id(self.id);
        let (full_rect, _) = ui.allocate_exact_size(total_size, egui::Sense::hover());
        let response = ui.interact(full_rect, widget_id, egui::Sense::click_and_drag());

        // While this terminal widget has focus, tell egui NOT to use Tab
        // for widget-focus navigation. Without this, pressing Tab for shell
        // autocomplete makes egui jump focus to the next widget in its tab
        // order, and the terminal stops receiving keystrokes until the user
        // clicks it again.
        ui.memory_mut(|m| {
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
        });

        let grid_rect = egui::Rect::from_min_size(full_rect.min, term_grid_size);
        let sb_track = egui::Rect::from_min_max(
            egui::pos2(grid_rect.max.x + 2.0, grid_rect.min.y),
            egui::pos2(full_rect.max.x, grid_rect.max.y),
        );

        // Pointer position during a drag is unreliable on Wayland:
        // hover_pos returns None when the cursor leaves the widget, and
        // egui's interact_pos can freeze at the press origin. `latest_pos`
        // tracks the live cursor each frame; fall back through the others.
        let pointer_pos = ui
            .input(|i| {
                i.pointer
                    .latest_pos()
                    .or_else(|| i.pointer.hover_pos())
                    .or_else(|| i.pointer.interact_pos())
            })
            .or_else(|| response.interact_pointer_pos())
            .unwrap_or(egui::Pos2::ZERO);

        let is_primary_down = ui.input(|i| i.pointer.primary_down());
        let is_primary_pressed =
            ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let is_primary_released =
            ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
        let is_ctrl = ui.input(|i| i.modifiers.ctrl);
        let is_shift = ui.input(|i| i.modifiers.shift);

        let rel_x = (pointer_pos.x - grid_rect.min.x).max(0.0);
        let rel_y = (pointer_pos.y - grid_rect.min.y).max(0.0);
        let cell_c = ((rel_x / char_width).floor() as u16).min(self.cols.saturating_sub(1));
        let cell_r = ((rel_y / row_height).floor() as u16).min(self.rows.saturating_sub(1));

        if grid_rect.contains(pointer_pos) {
            if app_wants_mouse && !is_shift {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            } else {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
        }

        if response.clicked() || response.secondary_clicked() || response.drag_started() {
            user_clicked_pane = true;
            response.request_focus();
        }

        let is_active_session = has_focus || user_clicked_pane;

        // Focus is requested by the caller (terminal_view) when the active
        // session changes, or right here when the pane is clicked. We never
        // steal focus automatically — that breaks TextEdits in split views.
        //
        // Recovery case: egui can lose widget focus on Wayland (pointer
        // leaves the window for a frame, WM hint, etc.). When that happens,
        // NO widget reports focus and `response.has_focus()` returns false
        // — so keystrokes are silently dropped and the user has to mash
        // Enter. If nothing at all has focus and we're the active session,
        // process keys here and re-request focus.
        //
        // We must NOT do this when another widget owns focus (SFTP path
        // TextEdit, settings field, ...) — otherwise keys would type both
        // there and here.
        let has_egui_focus = response.has_focus();
        let nothing_else_has_focus = ui.memory(|m| m.focused().is_none());
        // Recovery path: if the pane is the active one but a TextEdit
        // elsewhere grabbed focus and the user taps an arrow key, we
        // still want the terminal to receive it. Steal focus back in
        // that case. We only do this when a modifier is NOT held, so
        // deliberate Ctrl+... shortcuts elsewhere aren't hijacked.
        let terminal_key_stolen = if is_active_session && !has_egui_focus && !nothing_else_has_focus {
            ui.input(|i| {
                !i.modifiers.ctrl && !i.modifiers.command && !i.modifiers.alt
                    && i.events.iter().any(|e| matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::ArrowUp
                                | egui::Key::ArrowDown
                                | egui::Key::ArrowLeft
                                | egui::Key::ArrowRight
                                | egui::Key::Home
                                | egui::Key::End
                                | egui::Key::PageUp
                                | egui::Key::PageDown,
                            pressed: true,
                            ..
                        }
                    ))
            })
        } else {
            false
        };
        if terminal_key_stolen {
            response.request_focus();
            crate::dbg_log!("kbd_focus_steal_arrows id={}", self.id);
        }

        let has_egui_focus = response.has_focus();
        if is_active_session && (has_egui_focus || nothing_else_has_focus || terminal_key_stolen) {
            if !has_egui_focus && nothing_else_has_focus {
                response.request_focus();
                crate::dbg_log!("kbd_focus_recover id={}", self.id);
            }
            self.handle_keyboard_events(ui.ctx(), settings, toast);
        }

        if app_wants_mouse && !is_shift && is_active_session {
            if grid_rect.contains(pointer_pos) {
                let scroll_y = ui.input(|i| {
                    if i.raw_scroll_delta.y != 0.0 {
                        i.raw_scroll_delta.y
                    } else {
                        i.smooth_scroll_delta.y
                    }
                });
                if scroll_y != 0.0 {
                    let btn = if scroll_y > 0.0 { 64 } else { 65 };
                    self.send_mouse_event(
                        btn,
                        false,
                        cell_c,
                        cell_r,
                        ui.input(|i| i.modifiers),
                    );
                    ui.ctx().request_repaint();
                }
            }

            if is_primary_pressed && grid_rect.contains(pointer_pos) {
                self.send_mouse_event(0, false, cell_c, cell_r, ui.input(|i| i.modifiers));
            }
            if is_primary_released && grid_rect.contains(pointer_pos) {
                self.send_mouse_event(0, true, cell_c, cell_r, ui.input(|i| i.modifiers));
            }
            if response.secondary_clicked() && grid_rect.contains(pointer_pos) {
                self.send_mouse_event(2, false, cell_c, cell_r, ui.input(|i| i.modifiers));
                self.send_mouse_event(2, true, cell_c, cell_r, ui.input(|i| i.modifiers));
            }
        } else {
            // Full-screen program with no mouse mode (nano, less, vim, htop,
            // emacs -nw, mc, ...): translate wheel into PageUp/PageDown. Arrow
            // keys would just move the cursor within the visible buffer; PageUp/
            // PageDown scroll the view, which is what users expect from a wheel.
            if grid_rect.contains(pointer_pos) && !is_ctrl && (in_alternate || tui_in_primary) {
                let scroll_y = ui.input(|i| {
                    if i.raw_scroll_delta.y != 0.0 {
                        i.raw_scroll_delta.y
                    } else {
                        i.smooth_scroll_delta.y
                    }
                });
                if scroll_y != 0.0 {
                    let count = ((scroll_y.abs() / 50.0).round() as usize).clamp(1, 3);
                    let seq = if scroll_y > 0.0 { "\x1b[5~" } else { "\x1b[6~" };
                    if self.is_dragging_selection && self.tui_drag_direction.is_none() {
                        self.tui_drag_direction = Some(scroll_y > 0.0);
                    }
                    crate::dbg_log!(
                        "tui_wheel id={} alt={} tui_primary={} delta={:.1} count={} seq={:?} drag={}",
                        self.id,
                        in_alternate,
                        tui_in_primary,
                        scroll_y,
                        count,
                        seq,
                        self.is_dragging_selection
                    );
                    for _ in 0..count {
                        self.send_input(seq);
                    }
                    ui.ctx().request_repaint();
                }
            }

            if grid_rect.contains(pointer_pos) && !is_ctrl && show_scrollback_bar {
                let scroll_y = ui.input(|i| {
                    if i.raw_scroll_delta.y != 0.0 {
                        i.raw_scroll_delta.y
                    } else {
                        i.smooth_scroll_delta.y
                    }
                });

                if scroll_y != 0.0 {
                    // Convert egui's pixel delta into whole wheel notches
                    // (~18 px each) and multiply by the user's preferred
                    // lines-per-notch setting.
                    let notches = ((scroll_y.abs() / 18.0).round() as usize).max(1);
                    let per_notch = settings.mouse_wheel_scroll.lines(self.rows);
                    let lines = notches * per_notch;
                    if scroll_y > 0.0 {
                        self.set_view_scroll(self.scroll_offset + lines);
                    } else {
                        self.set_view_scroll(self.scroll_offset.saturating_sub(lines));
                    }

                    if self.is_dragging_selection && is_primary_down {
                        let age =
                            self.scroll_offset as i64 + (self.rows as i64 - 1 - cell_r as i64);
                        self.selection_end = Some((age, cell_c));
                    }
                    ui.ctx().request_repaint();
                }
            }

            if response.triple_clicked() && grid_rect.contains(pointer_pos) {
                let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - cell_r as i64);
                self.selection_start = Some((age, 0));
                self.selection_end = Some((age, self.cols.saturating_sub(1)));
                self.is_dragging_selection = false;
                if settings.copy_on_select {
                    let selected = self.extract_selected_text();
                    if !selected.trim().is_empty() {
                        set_system_clipboard_text(Some(ui.ctx()), &selected);
                        *toast = Some((
                            "Copied selected line".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                }
            } else if response.double_clicked() && grid_rect.contains(pointer_pos) {
                let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - cell_r as i64);

                if let Some((start_c, end_c)) = self.find_word_bounds(cell_r, cell_c) {
                    self.selection_start = Some((age, start_c));
                    self.selection_end = Some((age, end_c));
                    self.is_dragging_selection = false;
                    if settings.copy_on_select {
                        let selected = self.extract_selected_text();
                        if !selected.trim().is_empty() {
                            crate::dbg_log!(
                                "clipboard_word_copy begin id={} bytes={}",
                                self.id,
                                selected.len()
                            );
                            set_system_clipboard_text(Some(ui.ctx()), &selected);
                            crate::dbg_log!("clipboard_word_copy end id={}", self.id);
                            let preview = if selected.len() > 24 {
                                format!("{}...", &selected[..21].replace('\n', " "))
                            } else {
                                selected.replace('\n', " ")
                            };
                            *toast = Some((
                                format!("Copied: {}", preview),
                                std::time::Instant::now(),
                            ));
                        }
                    }
                }
            } else if is_primary_pressed
                && grid_rect.contains(pointer_pos)
                && (!sb_track.contains(pointer_pos) || in_alternate)
            {
                let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - cell_r as i64);
                self.selection_start = Some((age, cell_c));
                self.selection_end = Some((age, cell_c));
                self.is_dragging_selection = true;
                self.alt_drag_page_cooldown = None;
                self.tui_drag_frames.clear();
                self.tui_drag_direction = None;
                if in_alternate || tui_in_primary {
                    self.tui_drag_last_snapshot = self.snapshot_visible_lines();
                } else {
                    self.tui_drag_last_snapshot.clear();
                }
                crate::dbg_log!(
                    "sel_start id={} age={} col={} scroll_offset={}",
                    self.id,
                    age,
                    cell_c,
                    self.scroll_offset
                );
            }

            if self.is_dragging_selection && is_primary_down {
                let dragging_above = pointer_pos.y < grid_rect.min.y;
                let dragging_below = pointer_pos.y > grid_rect.max.y;
                // A full-screen app owns the screen if we're in the alternate
                // screen OR it's a primary-screen TUI like nano. In that case
                // there is no terminal scrollback to walk during a drag.
                let tui_owns_screen = in_alternate || tui_in_primary;

                if dragging_above && !tui_owns_screen {
                    // Shell scrollback: extend selection into history.
                    let dist = (grid_rect.min.y - pointer_pos.y).max(0.0);
                    let auto_scroll_lines = ((dist / 8.0).clamp(1.0, 30.0)) as usize;
                    self.set_view_scroll(self.scroll_offset + auto_scroll_lines);
                    let age = self.scroll_offset as i64 + (self.rows as i64 - 1);
                    self.selection_end = Some((age, cell_c));
                    crate::dbg_log!(
                        "sel_autoscroll_up id={} scroll_offset={} end=({},{})",
                        self.id,
                        self.scroll_offset,
                        age,
                        cell_c
                    );
                    ui.ctx().request_repaint();
                } else if dragging_below && !tui_owns_screen {
                    let dist = (pointer_pos.y - grid_rect.max.y).max(0.0);
                    let auto_scroll_lines = ((dist / 8.0).clamp(1.0, 30.0)) as usize;
                    self.set_view_scroll(self.scroll_offset.saturating_sub(auto_scroll_lines));
                    let age = self.scroll_offset as i64;
                    self.selection_end = Some((age, cell_c));
                    crate::dbg_log!(
                        "sel_autoscroll_down id={} scroll_offset={} end=({},{})",
                        self.id,
                        self.scroll_offset,
                        age,
                        cell_c
                    );
                    ui.ctx().request_repaint();
                } else if (dragging_above || dragging_below) && tui_owns_screen {
                    // Full-screen app (nano, less, vim, htop, emacs -nw):
                    // page the app itself while the user holds the drag past
                    // the edge. Throttle to ~8 Hz so we don't flood the PTY.
                    let now = std::time::Instant::now();
                    let ready = match self.alt_drag_page_cooldown {
                        Some(t) => now.duration_since(t).as_millis() >= 120,
                        None => true,
                    };
                    if ready {
                        let seq = if dragging_above { "\x1b[5~" } else { "\x1b[6~" };
                        self.send_input(seq);
                        self.alt_drag_page_cooldown = Some(now);
                        if self.tui_drag_direction.is_none() {
                            self.tui_drag_direction = Some(dragging_above);
                        }
                        crate::dbg_log!(
                            "sel_tui_page id={} alt={} tui_primary={} dir={}",
                            self.id,
                            in_alternate,
                            tui_in_primary,
                            if dragging_above { "up" } else { "down" }
                        );
                    }
                    let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - cell_r as i64);
                    self.selection_end = Some((age, cell_c));
                    ui.ctx().request_repaint();
                } else {
                    let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - cell_r as i64);
                    self.selection_end = Some((age, cell_c));
                    self.alt_drag_page_cooldown = None;
                }
            }

            if self.is_dragging_selection && is_primary_released {
                self.is_dragging_selection = false;
                self.alt_drag_page_cooldown = None;

                // TUI multi-frame path: user paged during the drag, so stitch
                // the captured screens together. Include the final frame.
                let used_tui_accumulator = (in_alternate || tui_in_primary)
                    && self.tui_drag_direction.is_some()
                    && !self.tui_drag_frames.is_empty();

                if used_tui_accumulator {
                    if settings.copy_on_select {
                        let final_snap = self.snapshot_visible_lines();
                        if final_snap != self.tui_drag_last_snapshot {
                            self.tui_drag_frames.push(final_snap);
                        }
                        let drag_up = self.tui_drag_direction.unwrap_or(true);
                        let frames = std::mem::take(&mut self.tui_drag_frames);
                        let combined = combine_tui_frames(frames, drag_up);
                        let line_count = combined.lines().count().max(1);
                        let preview = if combined.len() > 30 {
                            format!("{}...", &combined[..27].replace('\n', " "))
                        } else {
                            combined.replace('\n', " ")
                        };
                        crate::dbg_log!(
                            "tui_copy id={} bytes={} lines={} drag_up={}",
                            self.id,
                            combined.len(),
                            line_count,
                            drag_up
                        );
                        if !combined.trim().is_empty() {
                            set_system_clipboard_text(Some(ui.ctx()), &combined);
                            *toast = Some((
                                format!(
                                    "Copied {} line(s) (multi-screen): {}",
                                    line_count, preview
                                ),
                                std::time::Instant::now(),
                            ));
                        }
                    }
                    self.tui_drag_last_snapshot.clear();
                    self.tui_drag_direction = None;
                    self.selection_start = None;
                    self.selection_end = None;
                } else if let (Some(start), Some(end)) =
                    (self.selection_start, self.selection_end)
                {
                    self.tui_drag_frames.clear();
                    self.tui_drag_last_snapshot.clear();
                    self.tui_drag_direction = None;
                    if start == end {
                        self.selection_start = None;
                        self.selection_end = None;
                    } else if settings.copy_on_select {
                        let selected = self.extract_selected_text();
                        if !selected.trim().is_empty() {
                            crate::dbg_log!(
                                "clipboard_copy begin id={} bytes={}",
                                self.id,
                                selected.len()
                            );
                            set_system_clipboard_text(Some(ui.ctx()), &selected);
                            crate::dbg_log!("clipboard_copy end id={}", self.id);
                            let line_count = selected.lines().count().max(1);
                            let preview = if selected.len() > 30 {
                                format!("{}...", &selected[..27].replace('\n', " "))
                            } else {
                                selected.replace('\n', " ")
                            };
                            crate::dbg_log!(
                                "sel_copy id={} bytes={} lines={} start=({},{}) end=({},{})",
                                self.id,
                                selected.len(),
                                line_count,
                                start.0,
                                start.1,
                                end.0,
                                end.1
                            );
                            *toast = Some((
                                format!("Copied {} line(s): {}", line_count, preview),
                                std::time::Instant::now(),
                            ));
                        }
                    }
                }
            }

            if settings.paste_on_right_click && response.secondary_clicked() {
                crate::dbg_log!("clipboard_paste begin id={} (right-click)", self.id);
                if let Some(clip) = get_system_clipboard_text() {
                    if !clip.is_empty() {
                        self.send_paste(&clip);
                        *toast = Some((
                            "Pasted from clipboard".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                }
            }
        }

        if show_scrollback_bar {
            ui.painter().rect_filled(
                sb_track,
                3.0,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 6),
            );

            let sb_id = ui.id().with(self.id).with("term_sb");
            let sb_resp = ui.interact(sb_track, sb_id, egui::Sense::click_and_drag());

            let total_lines = (self.max_scroll + self.rows as usize).max(1) as f32;
            let visible_ratio = (self.rows as f32 / total_lines).clamp(0.04, 1.0);
            let max_thumb = sb_track.height().max(1.0);
            let min_thumb = 18.0_f32.min(max_thumb);
            let thumb_height = (sb_track.height() * visible_ratio).clamp(min_thumb, max_thumb);
            let scroll_ratio = if self.max_scroll > 0 {
                (self.scroll_offset as f32 / self.max_scroll as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let thumb_y = sb_track.bottom()
                - thumb_height
                - scroll_ratio * (sb_track.height() - thumb_height);

            let sb_thumb = egui::Rect::from_min_size(
                egui::pos2(sb_track.left() + 1.0, thumb_y),
                egui::vec2(sb_track.width() - 2.0, thumb_height),
            );

            if sb_resp.clicked() || sb_resp.dragged() {
                if let Some(ptr) = sb_resp.interact_pointer_pos() {
                    let rel_y = (sb_track.bottom() - ptr.y) / sb_track.height();
                    let target_offset =
                        (rel_y.clamp(0.0, 1.0) * self.max_scroll as f32).round() as usize;
                    self.set_view_scroll(target_offset);
                    ui.ctx().request_repaint();
                }
            }

            let thumb_color = if sb_resp.dragged() {
                theme.accent_color()
            } else if sb_resp.hovered() {
                theme.accent_hover_color()
            } else if self.scroll_offset > 0 {
                theme.accent_color().linear_multiply(0.7)
            } else {
                egui::Color32::from_rgb(55, 65, 81)
            };
            ui.painter().rect_filled(sb_thumb, 3.0, thumb_color);
        }

        ui.painter().rect_filled(grid_rect, 0.0, theme.bg_main_color());

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let screen = self.parser.screen();
            // Use self.rows/self.cols so the loop always matches term_grid_size
            // (which was allocated from the same values).
            let rows = self.rows;
            let cols = self.cols;
            let (cursor_r, cursor_c) = screen.cursor_position();
            let hide_cursor = screen.hide_cursor();

            let show_cursor = is_active_session
                && !hide_cursor
                && self.scroll_offset == 0
                && (!settings.cursor_blink || (ui.input(|i| (i.time * 2.0).fract() < 0.5)));

            for r in 0..rows {
                let row_y = grid_rect.min.y + r as f32 * row_height;
                let mut job = egui::text::LayoutJob::default();
                job.wrap.max_width = f32::INFINITY;

                let line_age = self.scroll_offset as i64 + (self.rows as i64 - 1 - r as i64);

                for c in 0..cols {
                    let default_cell = vt100::Cell::default();
                    let cell = screen.cell(r, c).unwrap_or(&default_cell);

                    if cell.is_wide_continuation() {
                        continue;
                    }

                    let is_cursor = show_cursor && (r == cursor_r && c == cursor_c);
                    let is_selected = self.is_cell_selected(line_age, c);
                    let cell_text = cell.contents();
                    let display_char: &str = if cell_text.is_empty() { " " } else { &cell_text };

                    let mut fg = vt_to_egui_color(cell.fgcolor(), false, theme);
                    let mut bg = vt_to_egui_color(cell.bgcolor(), true, theme);

                    if is_selected {
                        fg = theme.bg_main_color();
                        bg = theme.accent_color();
                    } else if cell.inverse() || is_cursor {
                        std::mem::swap(&mut fg, &mut bg);
                        if is_cursor && bg == fg {
                            fg = theme.bg_main_color();
                            bg = theme.text_primary_color();
                        }
                    }

                    job.append(
                        display_char,
                        0.0,
                        egui::TextFormat {
                            font_id: font_id.clone(),
                            color: fg,
                            background: if bg != theme.bg_main_color() {
                                bg
                            } else {
                                egui::Color32::TRANSPARENT
                            },
                            underline: if cell.underline() {
                                egui::Stroke::new(1.0_f32, fg)
                            } else {
                                egui::Stroke::NONE
                            },
                            ..Default::default()
                        },
                    );
                }

                let galley = ui.painter().layout_job(job);
                ui.painter().galley(
                    egui::pos2(grid_rect.min.x, row_y),
                    galley,
                    egui::Color32::WHITE,
                );
            }
        }));

        // TUI drag-accumulator: if the user is dragging inside a full-screen
        // app and the screen content changed (they paged via wheel or edge),
        // push the frame so we can stitch at release.
        if self.is_dragging_selection && (in_alternate || tui_in_primary) {
            let snap = self.snapshot_visible_lines();
            if snap != self.tui_drag_last_snapshot {
                if self.tui_drag_frames.is_empty() && !self.tui_drag_last_snapshot.is_empty() {
                    self.tui_drag_frames.push(self.tui_drag_last_snapshot.clone());
                }
                if !self.tui_drag_last_snapshot.is_empty() {
                    self.tui_drag_frames.push(snap.clone());
                }
                self.tui_drag_last_snapshot = snap;
                crate::dbg_log!(
                    "tui_frame_captured id={} frames={}",
                    self.id,
                    self.tui_drag_frames.len()
                );
            }
        }

        // Dead-session overlay: a big centered "Reconnect" button plus
        // a "press any key" hint. Painted on top of the frozen terminal
        // contents so the user can revive the shell / SSH connection
        // without losing the tile position or scrollback.
        if self.session_ended() {
            let overlay_rect = grid_rect;
            ui.painter().rect_filled(
                overlay_rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(
                    theme.bg_main[0],
                    theme.bg_main[1],
                    theme.bg_main[2],
                    205,
                ),
            );

            let btn_w = 230.0_f32;
            let btn_h = 46.0_f32;
            let btn_rect = egui::Rect::from_center_size(
                overlay_rect.center(),
                egui::vec2(btn_w, btn_h),
            );
            let btn_id = ui.id().with(self.id).with("term_reconnect_btn");
            let btn_resp = ui.interact(btn_rect, btn_id, egui::Sense::click());
            let hovered = btn_resp.hovered();
            let fill = if hovered {
                theme.accent_hover_color()
            } else {
                theme.accent_color()
            };
            ui.painter().rect_filled(btn_rect, 6.0, fill);
            ui.painter().rect_stroke(
                btn_rect,
                6.0,
                egui::Stroke::new(1.5_f32, theme.on_accent_color()),
            );
            ui.painter().text(
                btn_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Reconnect Session",
                egui::FontId::proportional(15.0),
                theme.on_accent_color(),
            );

            ui.painter().text(
                btn_rect.center() + egui::vec2(0.0, 34.0),
                egui::Align2::CENTER_CENTER,
                "or press any key",
                egui::FontId::proportional(11.5),
                theme.text_muted_color(),
            );

            if btn_resp.clicked() {
                self.reconnect_requested = true;
                user_clicked_pane = true;
            }
            if hovered {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }

        if self.scroll_offset > 0 && !in_alternate {
            let chip_w = 150.0;
            let chip_h = 22.0;
            let chip_rect = egui::Rect::from_min_size(
                egui::pos2(
                    grid_rect.max.x - chip_w - 6.0,
                    grid_rect.max.y - chip_h - 6.0,
                ),
                egui::vec2(chip_w, chip_h),
            );
            let chip_resp = ui.interact(
                chip_rect,
                ui.id().with(self.id).with("jump_chip"),
                egui::Sense::click(),
            );
            let is_chip_hov = chip_resp.hovered();

            ui.painter().rect(
                chip_rect,
                3.0,
                if is_chip_hov {
                    theme.bg_card_color()
                } else {
                    theme.bg_panel_color()
                },
                egui::Stroke::new(1.0_f32, theme.accent_color()),
            );
            ui.painter().text(
                chip_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("[Scrolled -{}] View Live", self.scroll_offset),
                egui::FontId::proportional(11.0),
                if is_chip_hov {
                    theme.accent_hover_color()
                } else {
                    theme.accent_color()
                },
            );

            if chip_resp.clicked() {
                self.set_view_scroll(0);
                ui.ctx().request_repaint();
            }
        }

        user_clicked_pane
    }
}
