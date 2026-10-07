// src/terminal.rs
use crate::settings::{AppSettings, BackspaceSequence};
use crate::theme::*;
use eframe::egui;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtyPair, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

// -- Alacritty terminal integration -----------------------------------------
//
// The terminal emulation engine is provided by `alacritty_terminal`,
// the same VT state machine that powers Alacritty. It handles all
// escape sequences, grid management, scrollback, and selection.
//
// We feed raw PTY bytes through a `vte::ansi::Processor` which drives
// the `Term` handler. Rendering reads `renderable_content()` and maps
// each cell to egui primitives.

use azterm_parser::event::{Event as AlacTermEvent, EventListener};
use azterm_parser::grid::{Dimensions, Scroll};
use azterm_parser::term::cell::Flags as CellFlags;
use azterm_parser::term::test::TermSize;
use azterm_parser::term::{Config as TermConfig, Term, TermMode};
use azterm_parser::vte::ansi::{
    Color as AlacColor, CursorShape, NamedColor, Processor,
};

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
    // Fire-and-forget.
    let _ = clipboard_tx().send(ClipboardMsg::Write(text.to_string()));
}

/// Maximum raw PTY bytes retained per session for restore-on-reopen.
/// When exceeded, the front of the buffer is trimmed up to the next ESC
/// so we never start replaying mid-escape-sequence.
const HISTORY_MAX: usize = 1024 * 1024;

// -- Event listener for alacritty_terminal ----------------------------------

/// Minimal event listener. We forward repaint requests and title changes.
#[derive(Clone)]
pub struct TermEventListener {
    ctx: egui::Context,
    title_tx: std::sync::mpsc::Sender<String>,
}

impl TermEventListener {
    fn new(ctx: egui::Context, title_tx: std::sync::mpsc::Sender<String>) -> Self {
        Self { ctx, title_tx }
    }
}

impl EventListener for TermEventListener {
    fn send_event(&self, event: AlacTermEvent) {
        match event {
            AlacTermEvent::Wakeup => {
                self.ctx.request_repaint();
            }
            AlacTermEvent::Title(title) => {
                let _ = self.title_tx.send(title);
            }
            AlacTermEvent::ResetTitle => {
                let _ = self.title_tx.send(String::new());
            }
            _ => {}
        }
    }
}

// -- Helper: convert alacritty color to egui color --------------------------

fn alac_to_egui_color(color: AlacColor, theme: &ThemeConfig, is_bg: bool) -> egui::Color32 {
    match color {
        AlacColor::Named(named) => match named {
            NamedColor::Black => egui::Color32::from_rgb(0, 0, 0),
            NamedColor::Red => egui::Color32::from_rgb(205, 49, 49),
            NamedColor::Green => egui::Color32::from_rgb(13, 188, 121),
            NamedColor::Yellow => egui::Color32::from_rgb(229, 229, 16),
            NamedColor::Blue => egui::Color32::from_rgb(36, 114, 200),
            NamedColor::Magenta => egui::Color32::from_rgb(188, 63, 188),
            NamedColor::Cyan => egui::Color32::from_rgb(17, 168, 205),
            NamedColor::White => egui::Color32::from_rgb(229, 229, 229),
            NamedColor::BrightBlack => egui::Color32::from_rgb(102, 102, 102),
            NamedColor::BrightRed => egui::Color32::from_rgb(241, 76, 76),
            NamedColor::BrightGreen => egui::Color32::from_rgb(35, 209, 139),
            NamedColor::BrightYellow => egui::Color32::from_rgb(245, 245, 67),
            NamedColor::BrightBlue => egui::Color32::from_rgb(59, 142, 234),
            NamedColor::BrightMagenta => egui::Color32::from_rgb(214, 112, 214),
            NamedColor::BrightCyan => egui::Color32::from_rgb(41, 184, 219),
            NamedColor::BrightWhite => egui::Color32::from_rgb(255, 255, 255),
            NamedColor::Foreground => theme.text_primary_color(),
            NamedColor::Background => theme.bg_main_color(),
            _ => {
                if is_bg {
                    theme.bg_main_color()
                } else {
                    theme.text_primary_color()
                }
            }
        },
        AlacColor::Spec(rgb) => egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b),
        AlacColor::Indexed(idx) => theme.ansi_color(idx),
    }
}

// -- TerminalSession --------------------------------------------------------

#[derive(Debug, Clone)]
pub enum SessionType {
    Local { working_dir: String },
    Ssh { profile_id: String },
}

pub struct TerminalSession {
    pub id: usize,
    pub title: String,
    pub session_type: SessionType,
    pub term: Term<TermEventListener>,
    pub parser: Processor,
    pub rx: Receiver<Vec<u8>>,
    /// Channel to the dedicated pty-writer thread.
    pub writer_tx: SyncSender<WriterMsg>,
    /// Local PTY master handle. None in daemon mode.
    pub master_pty: Option<Arc<Mutex<Box<dyn MasterPty + Send>>>>,
    /// Daemon-mode resize sink.
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
    pub alt_drag_page_cooldown: Option<std::time::Instant>,
    pub tui_drag_frames: Vec<Vec<String>>,
    pub tui_drag_last_snapshot: Vec<String>,
    pub tui_drag_direction: Option<bool>,

    pub recovery_note: Option<(String, (u8, u8, u8))>,
    pub history_buf: Vec<u8>,
    pub history_dirty: bool,

    pub is_dead: std::sync::Arc<AtomicBool>,
    pub reconnect_requested: bool,

    /// Retained context for re-creating the Term on panic recovery.
    ctx: egui::Context,
    /// Receiver for title updates from the event listener.
    title_rx: Receiver<String>,
}

fn make_term(
    ctx: &egui::Context,
    rows: u16,
    cols: u16,
    scrollback: usize,
) -> (Term<TermEventListener>, Receiver<String>) {
    let (title_tx, title_rx) = std::sync::mpsc::channel();
    let listener = TermEventListener::new(ctx.clone(), title_tx);
    let term_config = TermConfig {
        scrolling_history: scrollback.max(1000),
        ..Default::default()
    };
    let term_size = TermSize {
        columns: cols as usize,
        screen_lines: rows as usize,
    };
    let term = Term::new(term_config, &term_size, listener);
    (term, title_rx)
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
        let (writer_tx, writer_rx): (SyncSender<WriterMsg>, Receiver<WriterMsg>) =
            sync_channel(65536);

        let is_dead = std::sync::Arc::new(AtomicBool::new(false));

        // Dedicated writer thread.
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
            let ctx = ctx.clone();
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

        let (term, title_rx) = make_term(&ctx, rows, cols, scrollback_len);

        Self {
            id,
            title,
            session_type,
            term,
            parser: Processor::new(),
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
            ctx,
            title_rx,
        }
    }

    /// True if this session's PTY lives in the azterm-daemon process.
    pub fn is_daemon(&self) -> bool {
        self.daemon_resize_tx.is_some()
    }

    /// Build a TerminalSession that reads from and writes to the daemon.
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

        let (term, title_rx) = make_term(&ctx, rows, cols, scrollback_len);

        Some(Self {
            id,
            title,
            session_type,
            term,
            parser: Processor::new(),
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
            ctx,
            title_rx,
        })
    }

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

        // Try to extract a path from the current title (set via OSC 0/2).
        if !self.title.is_empty() {
            if let Some(dir) = Self::parse_dir_from_str(&self.title, ssh_user) {
                return Some(dir);
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

    pub fn feed_restore_history(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.parser.advance(&mut self.term, bytes);
        }));

        // Push visible content into scrollback, then clear and home.
        let mut push = Vec::with_capacity(self.rows as usize * 2 + 8);
        for _ in 0..(self.rows as usize * 2) {
            push.push(b'\n');
        }
        push.extend_from_slice(b"\x1b[2J\x1b[H");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.parser.advance(&mut self.term, &push);
        }));

        self.scroll_offset = 0;
        self.max_scroll = 0;
        self.term.scroll_display(Scroll::Bottom);
    }

    pub fn clear_screen_and_scrollback(&mut self) {
        self.parser.advance(&mut self.term, b"\x1b[2J\x1b[H");
        self.scroll_offset = 0;
        self.max_scroll = 0;
        self.selection_start = None;
        self.selection_end = None;
    }

    pub fn query_max_scrollback(&mut self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn set_view_scroll(&mut self, target: usize) {
        if self.term.mode().contains(TermMode::ALT_SCREEN) {
            self.scroll_offset = 0;
            self.term.scroll_display(Scroll::Bottom);
            return;
        }

        self.max_scroll = self.term.history_size();
        let clamped = target.min(self.max_scroll);
        let current = self.term.grid().display_offset() as i32;
        let delta = clamped as i32 - current;
        if delta != 0 {
            self.term.scroll_display(Scroll::Delta(delta));
        }
        self.scroll_offset = clamped;
    }

    pub fn send_input(&mut self, text: &str) {
        if self.recovery_note.is_some() {
            self.recovery_note = None;
        }
        if self.scroll_offset > 0 && !self.term.mode().contains(TermMode::ALT_SCREEN) {
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
        let mode = self.term.mode();
        if !mode.intersects(TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_MOTION) {
            return;
        }

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

        let sgr = mode.contains(TermMode::SGR_MOUSE);
        if sgr {
            let term = if is_release { 'm' } else { 'M' };
            let seq = format!("\x1b[<{};{};{}{}", btn, c, r, term);
            self.send_input(&seq);
        } else {
            let code = if is_release { 3 } else { btn };
            let cb = 32u8.saturating_add(code);
            let cx = (32u16.saturating_add(c)).min(255) as u8;
            let cy = (32u16.saturating_add(r)).min(255) as u8;
            let _ = self.writer_tx.try_send(WriterMsg::Data(vec![
                b'\x1b', b'[', b'M', cb, cx, cy,
            ]));
        }
    }

    pub fn send_paste(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.scroll_offset > 0 {
            self.set_view_scroll(0);
        }

        let bracketed = self.term.mode().contains(TermMode::BRACKETED_PASTE);
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
        let in_alt = self.term.mode().contains(TermMode::ALT_SCREEN);

        let old_max = if !in_alt && self.scroll_offset > 0 {
            self.term.history_size()
        } else {
            0
        };

        while let Ok(bytes) = self.rx.try_recv() {
            total_bytes += bytes.len();

            crate::dbg_log!(
                "pty_recv id={} n={} hex={:02x?}",
                self.id,
                bytes.len(),
                &bytes[..bytes.len().min(64)]
            );

            let parse_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.parser.advance(&mut self.term, &bytes);
            }));
            if parse_result.is_err() {
                crate::dbg_log!(
                    "PARSER_PANIC id={} bytes={:02x?} — resetting term",
                    self.id,
                    &bytes[..bytes.len().min(128)]
                );
                let (term, title_rx) =
                    make_term(&self.ctx, self.rows, self.cols, self.scrollback_limit);
                self.term = term;
                self.title_rx = title_rx;
                self.parser = Processor::new();
                let _ = self.writer_tx.try_send(WriterMsg::Data(vec![0x0c]));
            }

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

        // Pull any title updates.
        while let Ok(title) = self.title_rx.try_recv() {
            if !title.is_empty() {
                self.title = title;
            }
        }

        if total_bytes > 0 {
            crate::dbg_log!("pty_poll id={} bytes={}", self.id, total_bytes);
        }

        if in_alt {
            self.scroll_offset = 0;
        } else if self.scroll_offset > 0 {
            let new_max = self.term.history_size();
            if new_max > old_max {
                let added = new_max - old_max;
                let new_offset = (self.scroll_offset + added).min(new_max);
                let current = self.term.grid().display_offset() as i32;
                let delta = new_offset as i32 - current;
                if delta != 0 {
                    self.term.scroll_display(Scroll::Delta(delta));
                }
                self.scroll_offset = new_offset;
            }
            self.max_scroll = new_max;
        }
    }

    // -- Selection helpers --------------------------------------------------

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

    /// Snapshot visible cells keyed by (viewport_row, col).
    fn visible_cells(&self) -> std::collections::HashMap<(i32, usize), (char, AlacColor, AlacColor, CellFlags)> {
        let mut map = std::collections::HashMap::new();
        let content = self.term.renderable_content();
        for indexed in content.display_iter {
            let row = indexed.point.line.0;
            let col = indexed.point.column.0;
            let cell = indexed.cell;
            map.insert((row, col), (cell.c, cell.fg, cell.bg, cell.flags));
        }
        map
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

            let cells = self.visible_cells();
            let rows = self.rows as i64;
            let cols = self.cols;

            for age in (bot_age..=top_age).rev() {
                let screen_r = rows - 1 - age;
                let start_c = if age == top_age { top_col } else { 0 };
                let end_c = if age == bot_age {
                    bot_col
                } else {
                    cols.saturating_sub(1)
                };

                let mut line = String::new();
                for c in start_c..=end_c {
                    if let Some((ch, _, _, flags)) = cells.get(&(screen_r as i32, c as usize)) {
                        if flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                            continue;
                        }
                        if *ch == '\0' || *ch == ' ' {
                            line.push(' ');
                        } else {
                            line.push(*ch);
                        }
                    } else {
                        line.push(' ');
                    }
                }
                result.push_str(line.trim_end());
                if age != bot_age {
                    result.push('\n');
                }
            }
        }
        result
    }

    // -- Keyboard handling --------------------------------------------------

    fn handle_keyboard_events(
        &mut self,
        ctx: &egui::Context,
        settings: &AppSettings,
        toast: &mut Option<(String, std::time::Instant)>,
    ) {
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

        let has_paste_event = ctx.input(|i| {
            i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))
        });

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
                    if !self.term.mode().contains(TermMode::ALT_SCREEN) {
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

                        if *key == egui::Key::Tab {
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

                        let owns_scrollback = !self.term.mode().contains(TermMode::ALT_SCREEN)
                            && !self.term.mode().intersects(
                                TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_MOTION,
                            );
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
                                self.send_input(&String::from_utf8_lossy(seq));
                                continue;
                            }
                        }

                        if modifiers.ctrl && modifiers.shift && *key == egui::Key::A {
                            let max = self.term.history_size();
                            let top_age = (max + self.rows as usize - 1) as i64;
                            self.selection_start = Some((top_age, 0));
                            self.selection_end = Some((0i64, self.cols.saturating_sub(1)));
                            self.is_dragging_selection = false;
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

        if let Some(text) = pending_clipboard {
            set_system_clipboard_text(Some(ctx), &text);
        }
    }

    // -- Render -------------------------------------------------------------

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

        let in_alt = self.term.mode().contains(TermMode::ALT_SCREEN);
        let has_mouse = self
            .term
            .mode()
            .intersects(TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_MOTION);
        let app_wants_mouse = has_mouse;
        let tui_in_primary = !has_mouse && !in_alt && self.looks_like_primary_screen_tui();
        let show_scrollback_bar = !app_wants_mouse && !in_alt && !tui_in_primary;
        const SCROLLBAR_RESERVE: f32 = 12.0;
        let scrollbar_width = if show_scrollback_bar { SCROLLBAR_RESERVE } else { 0.0 };

        let avail = ui.available_size();
        let usable_w = (avail.x - SCROLLBAR_RESERVE).max(80.0);
        let usable_h = avail.y.max(40.0);
        let new_cols = ((usable_w / char_width).floor() as u16).max(4);
        let new_rows = ((usable_h / row_height).floor() as u16).max(2);

        if new_cols != self.cols || new_rows != self.rows {
            let old_rows = self.rows;
            let old_cols = self.cols;
            self.cols = new_cols;
            self.rows = new_rows;

            self.term.resize(TermSize {
                columns: new_cols as usize,
                screen_lines: new_rows as usize,
            });

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
                in_alt,
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

        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                widget_id,
                egui::EventFilter {
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

        let has_egui_focus = response.has_focus();
        let nothing_else_has_focus = ui.memory(|m| m.focused().is_none());
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
        }

        let has_egui_focus = response.has_focus();
        if is_active_session && (has_egui_focus || nothing_else_has_focus || terminal_key_stolen) {
            if !has_egui_focus && nothing_else_has_focus {
                response.request_focus();
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
            if grid_rect.contains(pointer_pos) && !is_ctrl && (in_alt || tui_in_primary) {
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
                self.selection_start = Some((age, 0));
                self.selection_end = Some((age, self.cols.saturating_sub(1)));
                self.is_dragging_selection = false;
                if settings.copy_on_select {
                    let selected = self.extract_selected_text();
                    if !selected.trim().is_empty() {
                        set_system_clipboard_text(Some(ui.ctx()), &selected);
                        *toast = Some((
                            format!("Copied: {}", selected.trim()),
                            std::time::Instant::now(),
                        ));
                    }
                }
            } else if is_primary_pressed
                && grid_rect.contains(pointer_pos)
                && (!sb_track.contains(pointer_pos) || in_alt)
            {
                let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - cell_r as i64);
                self.selection_start = Some((age, cell_c));
                self.selection_end = Some((age, cell_c));
                self.is_dragging_selection = true;
                self.alt_drag_page_cooldown = None;
                self.tui_drag_frames.clear();
                self.tui_drag_direction = None;
                if in_alt || tui_in_primary {
                    self.tui_drag_last_snapshot = self.snapshot_visible_lines();
                } else {
                    self.tui_drag_last_snapshot.clear();
                }
            }

            if self.is_dragging_selection && is_primary_down {
                let dragging_above = pointer_pos.y < grid_rect.min.y;
                let dragging_below = pointer_pos.y > grid_rect.max.y;
                let tui_owns_screen = in_alt || tui_in_primary;

                if dragging_above && !tui_owns_screen {
                    let dist = (grid_rect.min.y - pointer_pos.y).max(0.0);
                    let auto_scroll_lines = ((dist / 8.0).clamp(1.0, 30.0)) as usize;
                    self.set_view_scroll(self.scroll_offset + auto_scroll_lines);
                    let age = self.scroll_offset as i64 + (self.rows as i64 - 1);
                    self.selection_end = Some((age, cell_c));
                    ui.ctx().request_repaint();
                } else if dragging_below && !tui_owns_screen {
                    let dist = (pointer_pos.y - grid_rect.max.y).max(0.0);
                    let auto_scroll_lines = ((dist / 8.0).clamp(1.0, 30.0)) as usize;
                    self.set_view_scroll(self.scroll_offset.saturating_sub(auto_scroll_lines));
                    let age = self.scroll_offset as i64;
                    self.selection_end = Some((age, cell_c));
                    ui.ctx().request_repaint();
                } else if (dragging_above || dragging_below) && tui_owns_screen {
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

                let used_tui_accumulator = (in_alt || tui_in_primary)
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
                        if !combined.trim().is_empty() {
                            set_system_clipboard_text(Some(ui.ctx()), &combined);
                            *toast = Some((
                                format!("Copied {} line(s) (multi-screen)", line_count),
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
                            set_system_clipboard_text(Some(ui.ctx()), &selected);
                            let line_count = selected.lines().count().max(1);
                            *toast = Some((
                                format!("Copied {} line(s)", line_count),
                                std::time::Instant::now(),
                            ));
                        }
                    }
                }
            }

            if settings.paste_on_right_click && response.secondary_clicked() {
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

        // -- Scrollbar ------------------------------------------------------
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

        // -- Grid rendering -------------------------------------------------
        ui.painter().rect_filled(grid_rect, 0.0, theme.bg_main_color());

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let content = self.term.renderable_content();
            let rows = self.rows;
            let cols = self.cols;

            // Build the cell map. The keys are VIEWPORT-relative rows
            // (0 = top of the visible area), which is what display_iter
            // yields.
            let mut cell_map: std::collections::HashMap<(i32, usize), (char, AlacColor, AlacColor, CellFlags)> =
                std::collections::HashMap::new();
            for indexed in content.display_iter {
                let cell = indexed.cell;
                let row = indexed.point.line.0;
                let col = indexed.point.column.0;
                cell_map.insert((row, col), (cell.c, cell.fg, cell.bg, cell.flags));
            }

            let cursor = content.cursor;
            let cursor_hidden = cursor.shape == CursorShape::Hidden;
            let cursor_row = cursor.point.line.0;
            let cursor_col = cursor.point.column.0;

            let show_cursor = is_active_session
                && !cursor_hidden
                && self.scroll_offset == 0
                && (!settings.cursor_blink || (ui.input(|i| (i.time * 2.0).fract() < 0.5)));

            for r in 0..rows {
                let row_y = grid_rect.min.y + r as f32 * row_height;
                let line_age = self.scroll_offset as i64 + (self.rows as i64 - 1 - r as i64);

                let mut job = egui::text::LayoutJob::default();
                job.wrap.max_width = f32::INFINITY;

                for c in 0..cols {
                    let default_cell: (char, AlacColor, AlacColor, CellFlags) = (
                        ' ',
                        AlacColor::Named(NamedColor::Foreground),
                        AlacColor::Named(NamedColor::Background),
                        CellFlags::empty(),
                    );
                    let (ch, fg_col, bg_col, flags) = cell_map
                        .get(&(r as i32, c as usize))
                        .copied()
                        .unwrap_or(default_cell);

                    let is_cursor = show_cursor
                        && cursor_row == r as i32
                        && cursor_col == c as usize;
                    let is_selected = self.is_cell_selected(line_age, c);

                    let mut fg = alac_to_egui_color(fg_col, theme, false);
                    let mut bg = alac_to_egui_color(bg_col, theme, true);

                    if is_selected {
                        fg = theme.bg_main_color();
                        bg = theme.accent_color();
                    } else if flags.contains(CellFlags::INVERSE) || is_cursor {
                        std::mem::swap(&mut fg, &mut bg);
                        if is_cursor && bg == fg {
                            fg = theme.bg_main_color();
                            bg = theme.text_primary_color();
                        }
                    }

                    let cell_text: String = if ch == '\0' {
                        " ".to_string()
                    } else {
                        ch.to_string()
                    };

                    job.append(
                        &cell_text,
                        0.0,
                        egui::TextFormat {
                            font_id: font_id.clone(),
                            color: fg,
                            background: if bg != theme.bg_main_color() {
                                bg
                            } else {
                                egui::Color32::TRANSPARENT
                            },
                            underline: if flags.contains(CellFlags::UNDERLINE) {
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

        // Dead-session overlay.
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

        // Scroll chip.
        if self.scroll_offset > 0 && !in_alt {
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

    /// Heuristic: does this look like a full-screen TUI running in the
    /// primary screen?
    fn looks_like_primary_screen_tui(&self) -> bool {
        let content = self.term.renderable_content();
        let cursor = content.cursor;
        let cursor_row = cursor.point.line.0;
        if cursor_row >= (self.rows as i32 - 1) {
            return false;
        }
        for indexed in content.display_iter {
            if indexed.point.line.0 > cursor_row {
                let cell = indexed.cell;
                if cell.c != ' ' && cell.c != '\0' {
                    return true;
                }
            }
        }
        false
    }

    pub fn snapshot_visible_lines(&self) -> Vec<String> {
        let content = self.term.renderable_content();
        let mut lines: Vec<Vec<char>> = vec![vec![' '; self.cols as usize]; self.rows as usize];

        for indexed in content.display_iter {
            let row = indexed.point.line.0;
            let col = indexed.point.column.0;
            if row >= 0 && row < self.rows as i32 && col < self.cols as usize {
                let ch = indexed.cell.c;
                lines[row as usize][col] = if ch == '\0' { ' ' } else { ch };
            }
        }

        lines
            .into_iter()
            .map(|row| {
                let s: String = row.into_iter().collect();
                s.trim_end().to_string()
            })
            .collect()
    }
}

/// Stitch captured TUI frames into a single string.
fn combine_tui_frames(frames: Vec<Vec<String>>, drag_up: bool) -> String {
    if frames.is_empty() {
        return String::new();
    }
    if frames.len() == 1 {
        return frames.into_iter().next().unwrap().join("\n");
    }
    let mut ordered = frames;
    if drag_up {
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
