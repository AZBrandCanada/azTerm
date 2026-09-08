use crate::settings::{AppSettings, BackspaceSequence};
use crate::theme::*;
use eframe::egui;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtyPair, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

#[cfg(target_os = "linux")]
use arboard::{GetExtLinux, SetExtLinux};

pub fn get_system_clipboard_text() -> Option<String> {
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
        }
    }

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

    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("xsel")
            .args(["-b", "-o"])
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

    None
}

pub fn set_system_clipboard_text(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
        #[cfg(target_os = "linux")]
        {
            let _ = cb.set().clipboard(arboard::LinuxClipboardKind::Primary).text(text.to_string());
        }
    }

    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            if let Ok(mut child) = std::process::Command::new("wl-copy")
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
            }
        }
    }
}

fn vt_to_egui_color(color: vt100::Color, is_bg: bool) -> egui::Color32 {
    match color {
        vt100::Color::Default => {
            if is_bg {
                COLOR_BG_MAIN
            } else {
                COLOR_TEXT_PRIMARY
            }
        }
        vt100::Color::Idx(idx) => ansi_idx_to_color(idx),
        vt100::Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
    }
}

fn ansi_idx_to_color(idx: u8) -> egui::Color32 {
    match idx {
        0 => COLOR_BG_MAIN,
        1 => egui::Color32::from_rgb(239, 68, 68),
        2 => egui::Color32::from_rgb(34, 197, 94),
        3 => egui::Color32::from_rgb(234, 179, 8),
        4 => egui::Color32::from_rgb(99, 102, 241),
        5 => egui::Color32::from_rgb(168, 85, 247),
        6 => egui::Color32::from_rgb(6, 182, 212),
        7 => egui::Color32::from_rgb(203, 213, 225),
        8 => egui::Color32::from_rgb(71, 85, 105),
        9 => egui::Color32::from_rgb(248, 113, 113),
        10 => egui::Color32::from_rgb(74, 222, 128),
        11 => egui::Color32::from_rgb(250, 204, 21),
        12 => egui::Color32::from_rgb(129, 140, 248),
        13 => egui::Color32::from_rgb(192, 132, 252),
        14 => egui::Color32::from_rgb(34, 211, 238),
        15 => egui::Color32::from_rgb(255, 255, 255),
        16..=231 => {
            let i = idx - 16;
            let r = (i / 36) * 51;
            let g = ((i / 6) % 6) * 51;
            let b = (i % 6) * 51;
            egui::Color32::from_rgb(r, g, b)
        }
        232..=255 => {
            let gray = 8 + (idx - 232) * 10;
            egui::Color32::from_rgb(gray, gray, gray)
        }
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

    // Selection Tracking in absolute buffer line coordinates (abs_line, col)
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
        let rows = 28;
        let cols = 90;

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

        let (tx, rx) = channel::<Vec<u8>>();

        thread::spawn(move || {
            let mut buf = [0u8; 8192];
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
            selection_start: None,
            selection_end: None,
            is_dragging_selection: false,
        }
    }

    pub fn send_input(&mut self, text: &str) {
        self.scroll_offset = 0;
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(text.as_bytes());
            let _ = w.flush();
        }
    }

    pub fn poll_updates(&mut self) {
        let mut received = false;
        while let Ok(bytes) = self.rx.try_recv() {
            self.parser.process(&bytes);
            received = true;
        }
        if received && self.scroll_offset == 0 {
            self.parser.set_scrollback(0);
        }
    }

    fn is_cell_selected(&self, abs_line: i64, c: u16) -> bool {
        if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
            if start == end {
                return false;
            }
            let (mut l1, mut c1) = start;
            let (mut l2, mut c2) = end;
            if l1 > l2 || (l1 == l2 && c1 > c2) {
                std::mem::swap(&mut l1, &mut l2);
                std::mem::swap(&mut c1, &mut c2);
            }
            if abs_line < l1 || abs_line > l2 {
                return false;
            }
            if abs_line == l1 && abs_line == l2 {
                return c >= c1 && c <= c2;
            }
            if abs_line == l1 {
                return c >= c1;
            }
            if abs_line == l2 {
                return c <= c2;
            }
            true
        } else {
            false
        }
    }

    fn extract_selected_text(&mut self, max_scroll: usize) -> String {
        if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
            let (mut l1, mut c1) = start;
            let (mut l2, mut c2) = end;
            if l1 > l2 || (l1 == l2 && c1 > c2) {
                std::mem::swap(&mut l1, &mut l2);
                std::mem::swap(&mut c1, &mut c2);
            }

            let mut result = String::new();

            for l in l1..=l2 {
                let needed_offset = (max_scroll as i64 - l).max(0) as usize;
                self.parser.set_scrollback(needed_offset);
                let screen = self.parser.screen();

                let screen_r = (l - (max_scroll as i64 - needed_offset as i64)).clamp(0, self.rows.saturating_sub(1) as i64) as u16;
                let start_c = if l == l1 { c1 } else { 0 };
                let end_c = if l == l2 { c2 } else { self.cols.saturating_sub(1) };
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
                if l != l2 {
                    result.push('\n');
                }
            }

            // Restore user's current scroll view
            self.parser.set_scrollback(self.scroll_offset);
            result
        } else {
            String::new()
        }
    }

    fn handle_keyboard_events(&mut self, ctx: &egui::Context, settings: &AppSettings, max_scroll: usize) {
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
                        if self.selection_start.is_none() {
                            self.send_input("\x03");
                        }
                    }
                    egui::Event::Cut => {
                        self.send_input("\x18");
                    }
                    egui::Event::Paste(text) => {
                        self.send_input(text);
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

                        // Shift + Page Navigation for Scrollback History
                        if modifiers.shift {
                            if *key == egui::Key::PageUp {
                                let jump = (self.rows.saturating_sub(2) as usize).max(1);
                                self.scroll_offset = (self.scroll_offset + jump).min(max_scroll);
                                continue;
                            }
                            if *key == egui::Key::PageDown {
                                let jump = (self.rows.saturating_sub(2) as usize).max(1);
                                self.scroll_offset = self.scroll_offset.saturating_sub(jump);
                                continue;
                            }
                            if *key == egui::Key::Home {
                                self.scroll_offset = max_scroll;
                                continue;
                            }
                            if *key == egui::Key::End {
                                self.scroll_offset = 0;
                                continue;
                            }
                        }

                        if (modifiers.shift && *key == egui::Key::Insert)
                            || (modifiers.ctrl && modifiers.shift && *key == egui::Key::V)
                        {
                            if let Some(clip) = get_system_clipboard_text() {
                                self.send_input(&clip);
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
                            self.scroll_offset = 0;
                            if let Ok(mut w) = self.writer.lock() {
                                let _ = w.write_all(&b);
                                let _ = w.flush();
                            }
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
        toast: &mut Option<(String, std::time::Instant)>,
    ) {
        // Query max available scrollback lines from vt100 engine
        self.parser.set_scrollback(usize::MAX);
        let max_scroll = self.parser.screen().scrollback();
        self.scroll_offset = self.scroll_offset.min(max_scroll);
        self.parser.set_scrollback(self.scroll_offset);

        self.handle_keyboard_events(ui.ctx(), settings, max_scroll);

        let font_size = 14.0;
        let char_width = 8.4;
        let row_height = 17.5;
        let scrollbar_width = 14.0;
        let inner_padding = 16.0;

        let avail = ui.available_size();
        let usable_w = (avail.x - inner_padding - scrollbar_width).max(120.0);
        let usable_h = (avail.y - inner_padding).max(60.0);
        let new_cols = ((usable_w / char_width).floor() as u16).max(20);
        let new_rows = ((usable_h / row_height).floor() as u16).max(5);

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
        let total_size = egui::vec2(term_grid_size.x + scrollbar_width + 6.0, term_grid_size.y);

        egui::Frame::none()
            .fill(COLOR_BG_MAIN)
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                let (full_rect, response) = ui.allocate_exact_size(
                    total_size,
                    egui::Sense::click_and_drag(),
                );

                let grid_rect = egui::Rect::from_min_size(full_rect.min, term_grid_size);
                let sb_track = egui::Rect::from_min_max(
                    egui::pos2(grid_rect.max.x + 4.0, grid_rect.min.y),
                    egui::pos2(full_rect.max.x, grid_rect.max.y),
                );

                if response.clicked() || response.dragged() || !response.has_focus() {
                    response.request_focus();
                }

                let pointer_pos = ui.input(|i| i.pointer.hover_pos().unwrap_or(egui::Pos2::ZERO));
                let is_hovered = full_rect.contains(pointer_pos);
                let is_primary_down = ui.input(|i| i.pointer.primary_down());

                // Mouse Wheel Scrolling
                if is_hovered {
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
                            self.scroll_offset = (self.scroll_offset + lines).min(max_scroll);
                        } else {
                            self.scroll_offset = self.scroll_offset.saturating_sub(lines);
                        }
                        self.parser.set_scrollback(self.scroll_offset);

                        // If user is actively dragging while scrolling with wheel, dynamically extend selection
                        if self.is_dragging_selection && is_primary_down {
                            let visible_top = max_scroll as i64 - self.scroll_offset as i64;
                            let rel_x = (pointer_pos.x - grid_rect.min.x).max(0.0);
                            let rel_y = (pointer_pos.y - grid_rect.min.y).clamp(0.0, grid_rect.height() - 1.0);
                            let c = ((rel_x / char_width) as u16).min(self.cols.saturating_sub(1));
                            let r = ((rel_y / row_height) as u16).min(self.rows.saturating_sub(1));
                            self.selection_end = Some((visible_top + r as i64, c));
                        }
                        ui.ctx().request_repaint();
                    }
                }

                // Selection & Drag Auto-scrolling
                let visible_top_line = max_scroll as i64 - self.scroll_offset as i64;

                if self.is_dragging_selection && is_primary_down {
                    if pointer_pos.y < grid_rect.min.y {
                        // Dragged above the top -> auto-scroll up
                        let dist = (grid_rect.min.y - pointer_pos.y).max(0.0);
                        let auto_scroll_lines = ((dist / 14.0).clamp(1.0, 10.0)) as usize;
                        self.scroll_offset = (self.scroll_offset + auto_scroll_lines).min(max_scroll);
                        self.parser.set_scrollback(self.scroll_offset);

                        let new_top = max_scroll as i64 - self.scroll_offset as i64;
                        self.selection_end = Some((new_top, 0));
                        ui.ctx().request_repaint();
                    } else if pointer_pos.y > grid_rect.max.y {
                        // Dragged below the bottom -> auto-scroll down
                        let dist = (pointer_pos.y - grid_rect.max.y).max(0.0);
                        let auto_scroll_lines = ((dist / 14.0).clamp(1.0, 10.0)) as usize;
                        self.scroll_offset = self.scroll_offset.saturating_sub(auto_scroll_lines);
                        self.parser.set_scrollback(self.scroll_offset);

                        let new_top = max_scroll as i64 - self.scroll_offset as i64;
                        let bottom_line = new_top + self.rows.saturating_sub(1) as i64;
                        self.selection_end = Some((bottom_line, self.cols.saturating_sub(1)));
                        ui.ctx().request_repaint();
                    } else {
                        // Pointer is within vertical bounds
                        let rel_x = (pointer_pos.x - grid_rect.min.x).max(0.0);
                        let rel_y = (pointer_pos.y - grid_rect.min.y).max(0.0);
                        let c = ((rel_x / char_width) as u16).min(self.cols.saturating_sub(1));
                        let r = ((rel_y / row_height) as u16).min(self.rows.saturating_sub(1));
                        self.selection_end = Some((visible_top_line + r as i64, c));
                    }
                } else if let Some(pos) = response.interact_pointer_pos() {
                    if grid_rect.contains(pos) && response.drag_started_by(egui::PointerButton::Primary) {
                        let rel_x = (pos.x - grid_rect.min.x).max(0.0);
                        let rel_y = (pos.y - grid_rect.min.y).max(0.0);
                        let c = ((rel_x / char_width) as u16).min(self.cols.saturating_sub(1));
                        let r = ((rel_y / row_height) as u16).min(self.rows.saturating_sub(1));
                        let abs_l = visible_top_line + r as i64;

                        self.selection_start = Some((abs_l, c));
                        self.selection_end = Some((abs_l, c));
                        self.is_dragging_selection = true;
                    }
                }

                // Mouse release after dragging selection
                if self.is_dragging_selection && !is_primary_down {
                    self.is_dragging_selection = false;
                    if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
                        if start != end && settings.copy_on_select {
                            let selected = self.extract_selected_text(max_scroll);
                            if !selected.trim().is_empty() {
                                set_system_clipboard_text(&selected);
                                let preview = if selected.len() > 24 {
                                    format!("{}...", &selected[..21].replace('\n', " "))
                                } else {
                                    selected.replace('\n', " ")
                                };
                                *toast = Some((
                                    format!("Copied: \"{}\"", preview),
                                    std::time::Instant::now(),
                                ));
                            }
                        }
                    }
                }

                // Simple click clears active selection
                if response.clicked_by(egui::PointerButton::Primary)
                    && !sb_track.contains(pointer_pos)
                    && !self.is_dragging_selection
                {
                    self.selection_start = None;
                    self.selection_end = None;
                }

                let right_clicked = response.clicked_by(egui::PointerButton::Secondary)
                    || response.secondary_clicked()
                    || (is_hovered && ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary) || i.pointer.button_released(egui::PointerButton::Secondary)));

                if settings.paste_on_right_click && right_clicked {
                    if let Some(clip) = get_system_clipboard_text() {
                        if !clip.is_empty() {
                            self.send_input(&clip);
                            *toast = Some((
                                "Pasted from clipboard".to_string(),
                                std::time::Instant::now(),
                            ));
                        }
                    }
                }

                // Interactive Scrollbar
                ui.painter().rect_filled(sb_track, 4.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 6));

                let sb_id = ui.id().with(self.id).with("term_sb");
                let sb_resp = ui.interact(sb_track, sb_id, egui::Sense::click_and_drag());

                if max_scroll > 0 {
                    let total_lines = (max_scroll + self.rows as usize) as f32;
                    let visible_ratio = (self.rows as f32 / total_lines).clamp(0.04, 1.0);
                    let thumb_height = (sb_track.height() * visible_ratio).clamp(20.0, sb_track.height());
                    let scroll_ratio = (self.scroll_offset as f32 / max_scroll as f32).clamp(0.0, 1.0);
                    let thumb_y = sb_track.bottom() - thumb_height - scroll_ratio * (sb_track.height() - thumb_height);

                    let sb_thumb = egui::Rect::from_min_size(
                        egui::pos2(sb_track.left() + 1.0, thumb_y),
                        egui::vec2(sb_track.width() - 2.0, thumb_height),
                    );

                    if sb_resp.clicked() || sb_resp.dragged() {
                        if let Some(ptr) = sb_resp.interact_pointer_pos() {
                            let rel_y = (sb_track.bottom() - ptr.y) / sb_track.height();
                            self.scroll_offset = (rel_y.clamp(0.0, 1.0) * max_scroll as f32).round() as usize;
                            self.scroll_offset = self.scroll_offset.min(max_scroll);
                            self.parser.set_scrollback(self.scroll_offset);
                            ui.ctx().request_repaint();
                        }
                    }

                    let thumb_color = if sb_resp.dragged() {
                        COLOR_ACCENT
                    } else if sb_resp.hovered() {
                        COLOR_ACCENT_HOVER
                    } else if self.scroll_offset > 0 {
                        COLOR_INDIGO
                    } else {
                        egui::Color32::from_rgb(55, 65, 81)
                    };
                    ui.painter().rect_filled(sb_thumb, 4.0, thumb_color);
                }

                // Render Terminal Screen Grid
                let screen = self.parser.screen();
                let (rows, cols) = screen.size();
                let (cursor_r, cursor_c) = screen.cursor_position();
                let hide_cursor = screen.hide_cursor();

                let show_cursor = !hide_cursor
                    && self.scroll_offset == 0
                    && (!settings.cursor_blink
                        || (ui.input(|i| (i.time * 2.0).fract() < 0.5)));

                for r in 0..rows {
                    let row_y = grid_rect.min.y + r as f32 * row_height;
                    let cell_abs_line = visible_top_line + r as i64;
                    let mut job = egui::text::LayoutJob::default();
                    job.wrap.max_width = f32::INFINITY;

                    for c in 0..cols {
                        let default_cell = vt100::Cell::default();
                        let cell = screen.cell(r, c).unwrap_or(&default_cell);

                        if cell.is_wide_continuation() {
                            continue;
                        }

                        let is_cursor = show_cursor && (r == cursor_r && c == cursor_c);
                        let is_selected = self.is_cell_selected(cell_abs_line, c);
                        let cell_text = cell.contents();
                        let display_char: &str = if cell_text.is_empty() { " " } else { &cell_text };

                        let mut fg = vt_to_egui_color(cell.fgcolor(), false);
                        let mut bg = vt_to_egui_color(cell.bgcolor(), true);

                        if is_selected {
                            fg = egui::Color32::from_rgb(11, 15, 25);
                            bg = COLOR_ACCENT;
                        } else if cell.inverse() || is_cursor {
                            std::mem::swap(&mut fg, &mut bg);
                            if is_cursor && bg == fg {
                                fg = COLOR_BG_MAIN;
                                bg = COLOR_TEXT_PRIMARY;
                            }
                        }

                        job.append(
                            display_char,
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::monospace(font_size),
                                color: fg,
                                background: if bg != COLOR_BG_MAIN {
                                    bg
                                } else {
                                    egui::Color32::TRANSPARENT
                                },
                                ..Default::default()
                            },
                        );
                    }

                    let galley = ui.painter().layout_job(job);
                    ui.painter().galley(egui::pos2(grid_rect.min.x, row_y), galley, egui::Color32::WHITE);
                }

                // Floating Jump to Bottom Chip
                if self.scroll_offset > 0 {
                    let chip_w = 175.0;
                    let chip_h = 24.0;
                    let chip_rect = egui::Rect::from_min_size(
                        egui::pos2(grid_rect.max.x - chip_w - 8.0, grid_rect.max.y - chip_h - 6.0),
                        egui::vec2(chip_w, chip_h),
                    );
                    let chip_resp = ui.interact(chip_rect, ui.id().with(self.id).with("jump_chip"), egui::Sense::click());
                    let is_chip_hov = chip_resp.hovered();

                    ui.painter().rect(
                        chip_rect,
                        4.0,
                        if is_chip_hov { COLOR_BG_CARD } else { COLOR_BG_PANEL },
                        egui::Stroke::new(1.0_f32, COLOR_ACCENT),
                    );
                    ui.painter().text(
                        chip_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("↓ Scrolled (-{}) • Live View", self.scroll_offset),
                        egui::FontId::proportional(12.0),
                        if is_chip_hov { COLOR_ACCENT_HOVER } else { COLOR_ACCENT },
                    );

                    if chip_resp.clicked() {
                        self.scroll_offset = 0;
                        self.parser.set_scrollback(0);
                        ui.ctx().request_repaint();
                    }
                }
            });
    }
}
