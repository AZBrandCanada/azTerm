// src/terminal.rs
use crate::settings::{AppSettings, BackspaceSequence};
use crate::theme::*;
use eframe::egui;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtyPair, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

#[cfg(target_os = "linux")]
use arboard::{GetExtLinux, SetExtLinux};

enum WriterMsg {
    Data(Vec<u8>),
}

pub fn get_system_clipboard_text() -> Option<String> {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        if let Ok(text) = cb.get_text() {
            if !text.is_empty() {
                return Some(text);
            }
        }
        #[cfg(target_os = "linux")]
        {
            if let Ok(text) = cb.get().clipboard(arboard::LinuxClipboardKind::Primary).text() {
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            if let Ok(output) = std::process::Command::new("wl-paste")
                .arg("--no-newline")
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
        } else {
            if let Ok(output) = std::process::Command::new("xclip")
                .args(["-selection", "clipboard", "-o"])
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
        }
    }

    None
}

pub fn set_system_clipboard_text(ctx: Option<&egui::Context>, text: &str) {
    if text.is_empty() {
        return;
    }

    if let Some(c) = ctx {
        c.copy_text(text.to_string());
    }

    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
        #[cfg(target_os = "linux")]
        {
            let _ = cb.set().clipboard(arboard::LinuxClipboardKind::Primary).text(text.to_string());
        }
    }
}

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

pub struct TerminalSession {
    pub id: usize,
    pub title: String,
    pub session_type: SessionType,
    pub parser: vt100::Parser,
    pub rx: Receiver<Vec<u8>>,
    pub writer_tx: SyncSender<WriterMsg>,
    pub master_pty: Arc<Mutex<Box<dyn MasterPty + Send>>>,
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
        let (writer_tx, writer_rx): (SyncSender<WriterMsg>, Receiver<WriterMsg>) = sync_channel(4096);

        // Dedicated writer thread. The UI never blocks on pty writes — it just
        // hands bytes to this thread. This is what prevents UI freezes when
        // SSH's stdin buffer backs up on a stalled connection.
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
        });

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
            master_pty,
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
        }
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
            let end_idx = path_part.find(|c| c == '$' || c == '#' || c == '%' || c == ' ' || c == '\n')
                .unwrap_or(path_part.len());
            path_part[..end_idx].trim()
        } else if let Some(idx) = clean.find("] ") {
            let after = &clean[idx + 2..];
            let end_idx = after.find(|c| c == '$' || c == '#' || c == '%').unwrap_or(after.len());
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
            format!("{}/{}", home_dir.trim_end_matches('/'), stripped.trim_matches('/'))
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

    pub fn send_input(&mut self, text: &str) {
        if self.scroll_offset > 0 && !self.parser.screen().alternate_screen() {
            self.set_view_scroll(0);
        }
        let _ = self.writer_tx.try_send(WriterMsg::Data(text.as_bytes().to_vec()));
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
        if modifiers.shift { btn = btn.saturating_add(4); }
        if modifiers.alt { btn = btn.saturating_add(8); }
        if modifiers.ctrl { btn = btn.saturating_add(16); }

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

        let _ = self.writer_tx.try_send(WriterMsg::Data(payload.into_bytes()));
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

            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.parser.process(&bytes);
            }));

            if total_bytes > 262_144 {
                break;
            }
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
            let (top_age, top_col, bot_age, bot_col) = if start.0 > end.0 || (start.0 == end.0 && start.1 <= end.1) {
                (start.0, start.1, end.0, end.1)
            } else {
                (end.0, end.1, start.0, start.1)
            };

            if line_age > top_age || line_age < bot_age {
                return false;
            }
            if line_age == top_age && line_age == bot_age {
                let (c1, c2) = if top_col <= bot_col { (top_col, bot_col) } else { (bot_col, top_col) };
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
                    return ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.' || ch == '/' || ch == ':';
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
            let (top_age, top_col, bot_age, bot_col) = if start.0 > end.0 || (start.0 == end.0 && start.1 <= end.1) {
                (start.0, start.1, end.0, end.1)
            } else {
                (end.0, end.1, start.0, start.1)
            };

            let saved_offset = self.scroll_offset;

            for age in (bot_age..=top_age).rev() {
                let target_r = (self.rows as i64 - 1) + self.scroll_offset as i64 - age;

                let (screen_r, temp_offset): (u16, usize) = if target_r >= 0 && target_r < self.rows as i64 {
                    (target_r as u16, self.scroll_offset)
                } else if target_r < 0 {
                    let needed_offset = (self.scroll_offset as i64 - target_r).max(0) as usize;
                    (0u16, needed_offset)
                } else {
                    let diff = target_r - (self.rows as i64 - 1);
                    let needed_offset = (self.scroll_offset as i64 - diff).max(0) as usize;
                    (self.rows.saturating_sub(1), needed_offset)
                };

                self.parser.set_scrollback(temp_offset);
                let screen = self.parser.screen();

                let start_c = if age == top_age { top_col } else { 0 };
                let end_c = if age == bot_age { bot_col } else { self.cols.saturating_sub(1) };

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

    fn handle_keyboard_events(&mut self, ctx: &egui::Context, settings: &AppSettings) {
        // If egui already produced an Event::Paste this frame, do NOT also send
        // the raw 0x16 (Ctrl+V / readline quoted-insert) — that double-input
        // corrupts pasted scripts and leaves readline in a weird state.
        let has_paste_event = ctx.input(|i| {
            i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))
        });

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
                            set_system_clipboard_text(Some(ctx), &selected);
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
                        ..
                    } => {
                        if *key == egui::Key::Escape {
                            self.send_input("\x1b");
                            continue;
                        }

                        if modifiers.shift {
                            if *key == egui::Key::PageUp {
                                let jump = (self.rows.saturating_sub(2) as usize).max(1);
                                self.set_view_scroll(self.scroll_offset + jump);
                                continue;
                            }
                            if *key == egui::Key::PageDown {
                                let jump = (self.rows.saturating_sub(2) as usize).max(1);
                                self.set_view_scroll(self.scroll_offset + jump);
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

                        if modifiers.ctrl && modifiers.shift && *key == egui::Key::C {
                            let selected = self.extract_selected_text();
                            if !selected.is_empty() {
                                set_system_clipboard_text(Some(ctx), &selected);
                            }
                            continue;
                        }

                        if (modifiers.shift && *key == egui::Key::Insert)
                            || (modifiers.ctrl && modifiers.shift && *key == egui::Key::V)
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
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        settings: &AppSettings,
        theme: &ThemeConfig,
        has_focus: bool,
        toast: &mut Option<(String, std::time::Instant)>,
    ) -> bool {
        let font_size = 13.5;
        let font_id = egui::FontId::monospace(font_size);

        let probe = ui.painter().layout_no_wrap(
            "WWWWWWWWWW".to_string(),
            font_id.clone(),
            egui::Color32::WHITE,
        );
        let char_width = (probe.size().x / 10.0).max(1.0);
        let row_height = (probe.size().y * 1.05).max(1.0);

        let in_alternate = self.parser.screen().alternate_screen();
        let app_wants_mouse = in_alternate && (self.parser.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None);
        let scrollbar_width = if in_alternate || app_wants_mouse { 0.0 } else { 12.0 };

        let avail = ui.available_size();
        let usable_w = (avail.x - scrollbar_width).max(80.0);
        let usable_h = avail.y.max(40.0);
        let new_cols = ((usable_w / char_width).floor() as u16).max(20);
        let new_rows = ((usable_h / row_height).floor() as u16).max(4);

        if new_cols != self.cols || new_rows != self.rows {
            self.cols = new_cols;
            self.rows = new_rows;

            self.parser.set_size(new_rows, new_cols);

            if let Ok(master) = self.master_pty.lock() {
                let _ = master.resize(PtySize {
                    rows: new_rows,
                    cols: new_cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
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

        let grid_rect = egui::Rect::from_min_size(full_rect.min, term_grid_size);
        let sb_track = egui::Rect::from_min_max(
            egui::pos2(grid_rect.max.x + 2.0, grid_rect.min.y),
            egui::pos2(full_rect.max.x, grid_rect.max.y),
        );

        let pointer_pos = ui.input(|i| i.pointer.hover_pos().unwrap_or(egui::Pos2::ZERO));
        let is_primary_down = ui.input(|i| i.pointer.primary_down());
        let is_primary_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let is_primary_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
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

        // Only consume keyboard input while this widget actually owns egui
        // focus. Otherwise typing in other widgets leaks into the terminal.
        if response.has_focus() {
            self.handle_keyboard_events(ui.ctx(), settings);
        }

        if app_wants_mouse && !is_shift && is_active_session {
            if grid_rect.contains(pointer_pos) {
                let scroll_y = ui.input(|i| {
                    if i.raw_scroll_delta.y != 0.0 { i.raw_scroll_delta.y } else { i.smooth_scroll_delta.y }
                });
                if scroll_y != 0.0 {
                    let btn = if scroll_y > 0.0 { 64 } else { 65 };
                    self.send_mouse_event(btn, false, cell_c, cell_r, ui.input(|i| i.modifiers));
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
            // Alt-screen program with no mouse mode (nano, less, vim...):
            // translate wheel to arrow keys, like xterm does.
            if grid_rect.contains(pointer_pos) && !is_ctrl && in_alternate {
                let scroll_y = ui.input(|i| {
                    if i.raw_scroll_delta.y != 0.0 {
                        i.raw_scroll_delta.y
                    } else {
                        i.smooth_scroll_delta.y
                    }
                });
                if scroll_y != 0.0 {
                    let count = ((scroll_y.abs() / 40.0).round() as usize).clamp(1, 5);
                    let seq = if scroll_y > 0.0 { "\x1b[A" } else { "\x1b[B" };
                    for _ in 0..count {
                        self.send_input(seq);
                    }
                    ui.ctx().request_repaint();
                }
            }

            if grid_rect.contains(pointer_pos) && !is_ctrl && !in_alternate {
                let scroll_y = ui.input(|i| {
                    if i.raw_scroll_delta.y != 0.0 {
                        i.raw_scroll_delta.y
                    } else {
                        i.smooth_scroll_delta.y
                    }
                });

                if scroll_y != 0.0 {
                    let lines = ((scroll_y.abs() / 18.0).round() as usize).max(1) * 3;
                    if scroll_y > 0.0 {
                        self.set_view_scroll(self.scroll_offset + lines);
                    } else {
                        self.set_view_scroll(self.scroll_offset.saturating_sub(lines));
                    }

                    if self.is_dragging_selection && is_primary_down {
                        let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - cell_r as i64);
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
                        *toast = Some(("Copied selected line".to_string(), std::time::Instant::now()));
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
                            set_system_clipboard_text(Some(ui.ctx()), &selected);
                            let preview = if selected.len() > 24 {
                                format!("{}...", &selected[..21].replace('\n', " "))
                            } else {
                                selected.replace('\n', " ")
                            };
                            *toast = Some((format!("Copied: {}", preview), std::time::Instant::now()));
                        }
                    }
                }
            } else if is_primary_pressed && grid_rect.contains(pointer_pos) && (!sb_track.contains(pointer_pos) || in_alternate) {
                let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - cell_r as i64);
                self.selection_start = Some((age, cell_c));
                self.selection_end = Some((age, cell_c));
                self.is_dragging_selection = true;
            }

            if self.is_dragging_selection && is_primary_down {
                if pointer_pos.y < grid_rect.min.y && !in_alternate {
                    let dist = (grid_rect.min.y - pointer_pos.y).max(0.0);
                    let auto_scroll_lines = ((dist / 14.0).clamp(1.0, 10.0)) as usize;
                    self.set_view_scroll(self.scroll_offset + auto_scroll_lines);
                    let age = self.scroll_offset as i64 + (self.rows as i64 - 1);
                    self.selection_end = Some((age, 0));
                    ui.ctx().request_repaint();
                } else if pointer_pos.y > grid_rect.max.y && !in_alternate {
                    let dist = (pointer_pos.y - grid_rect.max.y).max(0.0);
                    let auto_scroll_lines = ((dist / 14.0).clamp(1.0, 10.0)) as usize;
                    self.set_view_scroll(self.scroll_offset.saturating_sub(auto_scroll_lines));
                    let age = self.scroll_offset as i64;
                    self.selection_end = Some((age, self.cols.saturating_sub(1)));
                    ui.ctx().request_repaint();
                } else {
                    let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - cell_r as i64);
                    self.selection_end = Some((age, cell_c));
                }
            }

            if self.is_dragging_selection && is_primary_released {
                self.is_dragging_selection = false;
                if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
                    if start == end {
                        self.selection_start = None;
                        self.selection_end = None;
                    } else if settings.copy_on_select {
                        let selected = self.extract_selected_text();
                        if !selected.trim().is_empty() {
                            set_system_clipboard_text(Some(ui.ctx()), &selected);
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

        if !in_alternate && !app_wants_mouse {
            ui.painter().rect_filled(sb_track, 3.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 6));

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
            let thumb_y = sb_track.bottom() - thumb_height - scroll_ratio * (sb_track.height() - thumb_height);

            let sb_thumb = egui::Rect::from_min_size(
                egui::pos2(sb_track.left() + 1.0, thumb_y),
                egui::vec2(sb_track.width() - 2.0, thumb_height),
            );

            if sb_resp.clicked() || sb_resp.dragged() {
                if let Some(ptr) = sb_resp.interact_pointer_pos() {
                    let rel_y = (sb_track.bottom() - ptr.y) / sb_track.height();
                    let target_offset = (rel_y.clamp(0.0, 1.0) * self.max_scroll as f32).round() as usize;
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
            let (rows, cols) = screen.size();
            let (cursor_r, cursor_c) = screen.cursor_position();
            let hide_cursor = screen.hide_cursor();

            let show_cursor = is_active_session
                && !hide_cursor
                && self.scroll_offset == 0
                && (!settings.cursor_blink
                    || (ui.input(|i| (i.time * 2.0).fract() < 0.5)));

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
                            underline: if cell.underline() { egui::Stroke::new(1.0_f32, fg) } else { egui::Stroke::NONE },
                            ..Default::default()
                        },
                    );
                }

                let galley = ui.painter().layout_job(job);
                ui.painter().galley(egui::pos2(grid_rect.min.x, row_y), galley, egui::Color32::WHITE);
            }
        }));

        if self.scroll_offset > 0 && !in_alternate {
            let chip_w = 150.0;
            let chip_h = 22.0;
            let chip_rect = egui::Rect::from_min_size(
                egui::pos2(grid_rect.max.x - chip_w - 6.0, grid_rect.max.y - chip_h - 6.0),
                egui::vec2(chip_w, chip_h),
            );
            let chip_resp = ui.interact(chip_rect, ui.id().with(self.id).with("jump_chip"), egui::Sense::click());
            let is_chip_hov = chip_resp.hovered();

            ui.painter().rect(
                chip_rect,
                3.0,
                if is_chip_hov { theme.bg_card_color() } else { theme.bg_panel_color() },
                egui::Stroke::new(1.0_f32, theme.accent_color()),
            );
            ui.painter().text(
                chip_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("[Scrolled -{}] View Live", self.scroll_offset),
                egui::FontId::proportional(11.0),
                if is_chip_hov { theme.accent_hover_color() } else { theme.accent_color() },
            );

            if chip_resp.clicked() {
                self.set_view_scroll(0);
                ui.ctx().request_repaint();
            }
        }

        user_clicked_pane
    }
}
