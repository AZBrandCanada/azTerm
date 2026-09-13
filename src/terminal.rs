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
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub master_pty: Arc<Mutex<Box<dyn MasterPty + Send>>>,
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

        let _child = pair.slave.spawn_command(cmd).expect("Failed to spawn shell");

        let mut reader = pair
            .master
            .try_clone_reader()
            .expect("Failed to clone PTY reader");
        let writer = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .expect("Failed to take PTY writer"),
        ));
        let master_pty = Arc::new(Mutex::new(pair.master));

        let (tx, rx): (SyncSender<Vec<u8>>, Receiver<Vec<u8>>) = sync_channel(512);

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

        Self {
            id,
            title,
            session_type,
            parser: vt100::Parser::new(rows, cols, scrollback_len.max(1000)),
            rx,
            writer,
            master_pty,
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

    pub fn clear_screen_and_scrollback(&mut self) {
        self.parser = vt100::Parser::new(self.rows, self.cols, self.scrollback_limit);
        self.scroll_offset = 0;
        self.max_scroll = 0;
        self.selection_start = None;
        self.selection_end = None;
    }

    pub fn set_view_scroll(&mut self, target: usize) {
        if self.parser.screen().alternate_screen() {
            self.scroll_offset = 0;
            self.parser.set_scrollback(0);
            return;
        }

        // Measure true current scrollback rows by querying vt100's clamped offset
        self.parser.set_scrollback(usize::MAX);
        self.max_scroll = self.parser.screen().scrollback();

        let clamped = target.min(self.max_scroll);
        self.scroll_offset = clamped;
        self.parser.set_scrollback(clamped);
    }

    pub fn send_input(&mut self, text: &str) {
        if self.scroll_offset > 0 {
            self.set_view_scroll(0);
        }
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(text.as_bytes());
            let _ = w.flush();
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

        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(payload.as_bytes());
            let _ = w.flush();
        }
    }

    pub fn poll_updates(&mut self) {
        let mut total_bytes = 0;

        while let Ok(bytes) = self.rx.try_recv() {
            total_bytes += bytes.len();

            // Intercept CSI 3 J ("Erase in Display 3": Clear scrollback buffer)
            if bytes.windows(4).any(|w| w == b"\x1b[3J") {
                self.clear_screen_and_scrollback();
            }

            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.parser.process(&bytes);
            }));

            if total_bytes > 262_144 {
                break;
            }
        }

        if self.parser.screen().alternate_screen() {
            self.scroll_offset = 0;
            self.parser.set_scrollback(0);
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
                    // Clear the terminal screen and scrollback buffer completely
                    self.clear_screen_and_scrollback();
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
                                egui::Key::V => Some(22),
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
        let scrollbar_width = if in_alternate { 0.0 } else { 12.0 };

        let avail = ui.available_size();
        let usable_w = (avail.x - scrollbar_width - 8.0).max(80.0);
        let usable_h = (avail.y - 8.0).max(40.0);
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
                    pixel_width: (new_cols as f32 * char_width).round() as u16,
                    pixel_height: (new_rows as f32 * row_height).round() as u16,
                });
            }
        }

        let term_grid_size = egui::vec2(
            self.cols as f32 * char_width,
            self.rows as f32 * row_height,
        );
        let total_size = egui::vec2(term_grid_size.x + scrollbar_width + 4.0, term_grid_size.y);

        let mut user_clicked_pane = false;

        egui::Frame::none()
            .fill(theme.bg_main_color())
            .inner_margin(egui::Margin::symmetric(4.0, 4.0))
            .show(ui, |ui| {
                let (full_rect, response) = ui.allocate_exact_size(
                    total_size,
                    egui::Sense::click_and_drag(),
                );

                let grid_rect = egui::Rect::from_min_size(full_rect.min, term_grid_size);
                let sb_track = egui::Rect::from_min_max(
                    egui::pos2(grid_rect.max.x + 2.0, grid_rect.min.y),
                    egui::pos2(full_rect.max.x, grid_rect.max.y),
                );

                let pointer_pos = ui.input(|i| i.pointer.hover_pos().unwrap_or(egui::Pos2::ZERO));
                let is_hovered = full_rect.contains(pointer_pos);
                let is_primary_down = ui.input(|i| i.pointer.primary_down());
                let is_primary_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
                let is_primary_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
                let is_ctrl = ui.input(|i| i.modifiers.ctrl);

                if grid_rect.contains(pointer_pos) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                }

                if response.clicked()
                    || response.secondary_clicked()
                    || response.drag_started()
                    || (is_hovered && ui.input(|i| i.pointer.any_pressed()))
                {
                    user_clicked_pane = true;
                    response.request_focus();
                }

                let active_focus = has_focus || user_clicked_pane;

                if active_focus && !response.has_focus() {
                    response.request_focus();
                }

                if active_focus {
                    self.handle_keyboard_events(ui.ctx(), settings);
                }

                if is_hovered && !is_ctrl && !in_alternate {
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
                            let rel_x = (pointer_pos.x - grid_rect.min.x).max(0.0);
                            let rel_y = (pointer_pos.y - grid_rect.min.y).clamp(0.0, grid_rect.height() - 1.0);
                            let c = ((rel_x / char_width).floor() as u16).min(self.cols.saturating_sub(1));
                            let r = ((rel_y / row_height).floor() as u16).min(self.rows.saturating_sub(1));
                            let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - r as i64);
                            self.selection_end = Some((age, c));
                        }
                        ui.ctx().request_repaint();
                    }
                }

                if response.triple_clicked() && grid_rect.contains(pointer_pos) {
                    let rel_y = (pointer_pos.y - grid_rect.min.y).max(0.0);
                    let r = ((rel_y / row_height).floor() as u16).min(self.rows.saturating_sub(1));
                    let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - r as i64);
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
                    let rel_x = (pointer_pos.x - grid_rect.min.x).max(0.0);
                    let rel_y = (pointer_pos.y - grid_rect.min.y).max(0.0);
                    let c = ((rel_x / char_width).floor() as u16).min(self.cols.saturating_sub(1));
                    let r = ((rel_y / row_height).floor() as u16).min(self.rows.saturating_sub(1));
                    let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - r as i64);

                    if let Some((start_c, end_c)) = self.find_word_bounds(r, c) {
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
                    let rel_x = (pointer_pos.x - grid_rect.min.x).max(0.0);
                    let rel_y = (pointer_pos.y - grid_rect.min.y).max(0.0);
                    let c = ((rel_x / char_width).floor() as u16).min(self.cols.saturating_sub(1));
                    let r = ((rel_y / row_height).floor() as u16).min(self.rows.saturating_sub(1));
                    let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - r as i64);

                    self.selection_start = Some((age, c));
                    self.selection_end = Some((age, c));
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
                        let rel_x = (pointer_pos.x - grid_rect.min.x).max(0.0);
                        let rel_y = (pointer_pos.y - grid_rect.min.y).max(0.0);
                        let c = ((rel_x / char_width).floor() as u16).min(self.cols.saturating_sub(1));
                        let r = ((rel_y / row_height).floor() as u16).min(self.rows.saturating_sub(1));
                        let age = self.scroll_offset as i64 + (self.rows as i64 - 1 - r as i64);
                        self.selection_end = Some((age, c));
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

                if !in_alternate {
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

                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let screen = self.parser.screen();
                    let (rows, cols) = screen.size();
                    let (cursor_r, cursor_c) = screen.cursor_position();
                    let hide_cursor = screen.hide_cursor();

                    let show_cursor = active_focus
                        && !hide_cursor
                        && self.scroll_offset == 0
                        && (!settings.cursor_blink
                            || (ui.input(|i| (i.time * 2.0).fract() < 0.5)));

                    for r in 0..rows {
                        let row_y = grid_rect.min.y + r as f32 * row_height;
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

                            let cell_x = grid_rect.min.x + c as f32 * char_width;
                            let cell_width = if cell.is_wide() { char_width * 2.0 } else { char_width };
                            let cell_rect = egui::Rect::from_min_size(
                                egui::pos2(cell_x, row_y),
                                egui::vec2(cell_width, row_height),
                            );

                            let mut fg = vt_to_egui_color(cell.fgcolor(), false, theme);
                            let mut bg = vt_to_egui_color(cell.bgcolor(), true, theme);

                            if is_selected {
                                fg = theme.bg_main_color();
                                bg = theme.accent_color();
                            } else if cell.inverse() {
                                std::mem::swap(&mut fg, &mut bg);
                            }

                            // Always fill background to cleanly overwrite previous frame data
                            ui.painter().rect_filled(cell_rect, 0.0, bg);

                            if is_cursor {
                                ui.painter().rect_filled(
                                    cell_rect,
                                    1.0,
                                    theme.accent_color().linear_multiply(0.85),
                                );
                                fg = theme.bg_main_color();
                            }

                            if !cell_text.is_empty() && cell_text != " " {
                                ui.painter().text(
                                    egui::pos2(cell_x, row_y + 1.0),
                                    egui::Align2::LEFT_TOP,
                                    &cell_text,
                                    font_id.clone(),
                                    fg,
                                );
                            }

                            if cell.underline() {
                                ui.painter().line_segment(
                                    [
                                        egui::pos2(cell_rect.min.x, cell_rect.max.y - 1.0),
                                        egui::pos2(cell_rect.max.x, cell_rect.max.y - 1.0),
                                    ],
                                    egui::Stroke::new(1.0_f32, fg),
                                );
                            }
                        }
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
            });

        user_clicked_pane
    }
}
