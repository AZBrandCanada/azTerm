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
            max_scroll: 0,
            selection_start: None,
            selection_end: None,
            is_dragging_selection: false,
        }
    }

    /// Safely clamps scrollback within vt100's real allocated buffer bounds to avoid subtraction underflow
    pub fn safe_set_scrollback(&mut self, target: usize) {
        if target == 0 {
            self.scroll_offset = 0;
            self.parser.set_scrollback(0);
            return;
        }

        // Check if target offset is safe to read
        self.parser.set_scrollback(target);
        let valid = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.parser.screen().cell(0, 0);
        })).is_ok();

        if valid {
            self.scroll_offset = target;
            return;
        }

        // Binary search the exact maximum safe offset currently in vt100 history
        let mut low = 0;
        let mut high = target;
        let mut best = 0;

        while low <= high {
            let mid = low + (high - low) / 2;
            self.parser.set_scrollback(mid);
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.parser.screen().cell(0, 0);
            })).is_ok();

            if ok {
                best = mid;
                low = mid + 1;
            } else {
                if mid == 0 {
                    break;
                }
                high = mid - 1;
            }
        }

        self.scroll_offset = best;
        self.max_scroll = best;
        self.parser.set_scrollback(best);
    }

    pub fn send_input(&mut self, text: &str) {
        self.safe_set_scrollback(0);
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(text.as_bytes());
            let _ = w.flush();
        }
    }

    pub fn poll_updates(&mut self) {
        let mut received = false;
        while let Ok(bytes) = self.rx.try_recv() {
            let newlines = bytes.iter().filter(|&&b| b == b'\n').count();
            self.max_scroll = (self.max_scroll + newlines).min(10000);

            // Safe cursor guard
            let (cur_r, _) = self.parser.screen().cursor_position();
            if cur_r >= self.rows {
                let safe_r = self.rows.saturating_sub(1);
                let cup = format!("\x1b[{};1H", safe_r + 1);
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.parser.process(cup.as_bytes());
                }));
            }

            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.parser.process(&bytes);
            }));
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

    fn extract_selected_text(&mut self) -> String {
        let mut result = String::new();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
                let (mut l1, mut c1) = start;
                let (mut l2, mut c2) = end;
                if l1 > l2 || (l1 == l2 && c1 > c2) {
                    std::mem::swap(&mut l1, &mut l2);
                    std::mem::swap(&mut c1, &mut c2);
                }

                let screen = self.parser.screen();

                for l in l1..=l2 {
                    let screen_r = (l.max(0) as u16).min(self.rows.saturating_sub(1));
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
            }
        }));
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

                        if modifiers.shift {
                            if *key == egui::Key::PageUp {
                                let jump = (self.rows.saturating_sub(2) as usize).max(1);
                                self.safe_set_scrollback(self.scroll_offset + jump);
                                continue;
                            }
                            if *key == egui::Key::PageDown {
                                let jump = (self.rows.saturating_sub(2) as usize).max(1);
                                self.safe_set_scrollback(self.scroll_offset.saturating_sub(jump));
                                continue;
                            }
                            if *key == egui::Key::Home {
                                self.safe_set_scrollback(usize::MAX);
                                continue;
                            }
                            if *key == egui::Key::End {
                                self.safe_set_scrollback(0);
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
        // Enforce safe scrollback on every frame
        self.safe_set_scrollback(self.scroll_offset);

        let font_size = 14.0;
        let char_width = 8.4;
        let row_height = 17.5;
        let scrollbar_width = 14.0;
        let inner_padding = 16.0;

        let avail = ui.available_size();
        let usable_w = (avail.x - inner_padding - scrollbar_width).max(80.0);
        let usable_h = (avail.y - inner_padding).max(40.0);
        let new_cols = ((usable_w / char_width).floor() as u16).max(15);
        let new_rows = ((usable_h / row_height).floor() as u16).max(4);

        if new_cols != self.cols || new_rows != self.rows {
            self.cols = new_cols;
            self.rows = new_rows;
            self.safe_set_scrollback(0);

            // Safe cursor reset before and after resizing
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.parser.process(b"\x1b[1;1H");
            }));

            self.parser.set_size(new_rows, new_cols);

            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.parser.process(b"\x1b[1;1H");
            }));

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

        let mut user_clicked_pane = false;

        egui::Frame::none()
            .fill(theme.bg_main_color())
            .inner_margin(egui::Margin::same(6.0))
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

                let pointer_pos = ui.input(|i| i.pointer.hover_pos().unwrap_or(egui::Pos2::ZERO));
                let is_hovered = full_rect.contains(pointer_pos);
                let is_primary_down = ui.input(|i| i.pointer.primary_down());
                let is_ctrl = ui.input(|i| i.modifiers.ctrl);

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

                if is_hovered && !is_ctrl {
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
                            self.safe_set_scrollback(self.scroll_offset + lines);
                        } else {
                            self.safe_set_scrollback(self.scroll_offset.saturating_sub(lines));
                        }

                        if self.is_dragging_selection && is_primary_down {
                            let rel_x = (pointer_pos.x - grid_rect.min.x).max(0.0);
                            let rel_y = (pointer_pos.y - grid_rect.min.y).clamp(0.0, grid_rect.height() - 1.0);
                            let c = ((rel_x / char_width) as u16).min(self.cols.saturating_sub(1));
                            let r = ((rel_y / row_height) as u16).min(self.rows.saturating_sub(1));
                            self.selection_end = Some((r as i64, c));
                        }
                        ui.ctx().request_repaint();
                    }
                }

                if self.is_dragging_selection && is_primary_down {
                    if pointer_pos.y < grid_rect.min.y {
                        let dist = (grid_rect.min.y - pointer_pos.y).max(0.0);
                        let auto_scroll_lines = ((dist / 14.0).clamp(1.0, 10.0)) as usize;
                        self.safe_set_scrollback(self.scroll_offset + auto_scroll_lines);
                        self.selection_end = Some((0, 0));
                        ui.ctx().request_repaint();
                    } else if pointer_pos.y > grid_rect.max.y {
                        let dist = (pointer_pos.y - grid_rect.max.y).max(0.0);
                        let auto_scroll_lines = ((dist / 14.0).clamp(1.0, 10.0)) as usize;
                        self.safe_set_scrollback(self.scroll_offset.saturating_sub(auto_scroll_lines));
                        self.selection_end = Some((self.rows.saturating_sub(1) as i64, self.cols.saturating_sub(1)));
                        ui.ctx().request_repaint();
                    } else {
                        let rel_x = (pointer_pos.x - grid_rect.min.x).max(0.0);
                        let rel_y = (pointer_pos.y - grid_rect.min.y).max(0.0);
                        let c = ((rel_x / char_width) as u16).min(self.cols.saturating_sub(1));
                        let r = ((rel_y / row_height) as u16).min(self.rows.saturating_sub(1));
                        self.selection_end = Some((r as i64, c));
                    }
                } else if let Some(pos) = response.interact_pointer_pos() {
                    if grid_rect.contains(pos) && response.drag_started_by(egui::PointerButton::Primary) {
                        let rel_x = (pos.x - grid_rect.min.x).max(0.0);
                        let rel_y = (pos.y - grid_rect.min.y).max(0.0);
                        let c = ((rel_x / char_width) as u16).min(self.cols.saturating_sub(1));
                        let r = ((rel_y / row_height) as u16).min(self.rows.saturating_sub(1));

                        self.selection_start = Some((r as i64, c));
                        self.selection_end = Some((r as i64, c));
                        self.is_dragging_selection = true;
                    }
                }

                if self.is_dragging_selection && !is_primary_down {
                    self.is_dragging_selection = false;
                    if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
                        if start != end && settings.copy_on_select {
                            let selected = self.extract_selected_text();
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

                ui.painter().rect_filled(sb_track, 4.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 6));

                let sb_id = ui.id().with(self.id).with("term_sb");
                let sb_resp = ui.interact(sb_track, sb_id, egui::Sense::click_and_drag());

                let total_lines = (self.max_scroll + self.rows as usize).max(1) as f32;
                let visible_ratio = (self.rows as f32 / total_lines).clamp(0.04, 1.0);
                let max_thumb = sb_track.height().max(1.0);
                let min_thumb = 20.0_f32.min(max_thumb);
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
                        self.safe_set_scrollback(target_offset);
                        ui.ctx().request_repaint();
                    }
                }

                let thumb_color = if sb_resp.dragged() {
                    theme.accent_color()
                } else if sb_resp.hovered() {
                    theme.accent_hover_color()
                } else if self.scroll_offset > 0 {
                    theme.accent_color().linear_multiply(0.8)
                } else {
                    egui::Color32::from_rgb(55, 65, 81)
                };
                ui.painter().rect_filled(sb_thumb, 4.0, thumb_color);

                // Safe terminal screen drawing
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
                        let mut job = egui::text::LayoutJob::default();
                        job.wrap.max_width = f32::INFINITY;

                        for c in 0..cols {
                            let default_cell = vt100::Cell::default();
                            let cell = screen.cell(r, c).unwrap_or(&default_cell);

                            if cell.is_wide_continuation() {
                                continue;
                            }

                            let is_cursor = show_cursor && (r == cursor_r && c == cursor_c);
                            let is_selected = self.is_cell_selected(r as i64, c);
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
                                    font_id: egui::FontId::monospace(font_size),
                                    color: fg,
                                    background: if bg != theme.bg_main_color() {
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
                }));

                if self.scroll_offset > 0 {
                    let chip_w = 160.0;
                    let chip_h = 22.0;
                    let chip_rect = egui::Rect::from_min_size(
                        egui::pos2(grid_rect.max.x - chip_w - 8.0, grid_rect.max.y - chip_h - 6.0),
                        egui::vec2(chip_w, chip_h),
                    );
                    let chip_resp = ui.interact(chip_rect, ui.id().with(self.id).with("jump_chip"), egui::Sense::click());
                    let is_chip_hov = chip_resp.hovered();

                    ui.painter().rect(
                        chip_rect,
                        4.0,
                        if is_chip_hov { theme.bg_card_color() } else { theme.bg_panel_color() },
                        egui::Stroke::new(1.0_f32, theme.accent_color()),
                    );
                    ui.painter().text(
                        chip_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("↓ Scrolled (-{}) • Live", self.scroll_offset),
                        egui::FontId::proportional(11.0),
                        if is_chip_hov { theme.accent_hover_color() } else { theme.accent_color() },
                    );

                    if chip_resp.clicked() {
                        self.safe_set_scrollback(0);
                        ui.ctx().request_repaint();
                    }
                }
            });

        user_clicked_pane
    }
}
