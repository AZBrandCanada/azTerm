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
    // 1. Try Wayland native wl-paste (cross-app compatible on Wayland)
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

    // 2. Try arboard (with native Wayland data-control + X11 + Windows + macOS)
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

    // 3. Fallback to xclip on X11
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

    // 4. Fallback to xsel on X11
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

#[derive(Debug, Clone)]
pub struct AuthPrompt {
    pub title: String,
    pub prompt_line: String,
    pub is_secret: bool,
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

    // Center Auth Popup State
    pub active_auth_prompt: Option<AuthPrompt>,
    pub auth_input: String,
    pub auth_show_secret: bool,

    // Selection Tracking
    pub selection_start: Option<(u16, u16)>,
    pub selection_end: Option<(u16, u16)>,
    pub is_dragging_selection: bool,
}

impl TerminalSession {
    pub fn new(
        id: usize,
        title: String,
        session_type: SessionType,
        cmd: CommandBuilder,
        ctx: egui::Context,
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
            parser: vt100::Parser::new(rows, cols, 2000),
            rx,
            writer,
            master_pty,
            rows,
            cols,
            active_auth_prompt: None,
            auth_input: String::new(),
            auth_show_secret: false,
            selection_start: None,
            selection_end: None,
            is_dragging_selection: false,
        }
    }

    pub fn send_input(&self, text: &str) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(text.as_bytes());
            let _ = w.flush();
        }
    }

    pub fn send_auth_response(&self, text: &str) {
        if let Ok(mut w) = self.writer.lock() {
            let clean = text.trim_end_matches(&['\r', '\n'][..]);
            let mut data = clean.as_bytes().to_vec();
            data.push(b'\r');
            let _ = w.write_all(&data);
            let _ = w.flush();
        }
    }

    pub fn poll_updates(&mut self, settings: &AppSettings) {
        let keywords: Vec<String> = settings
            .two_factor_keywords
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        while let Ok(bytes) = self.rx.try_recv() {
            self.parser.process(&bytes);
            let text = String::from_utf8_lossy(&bytes);
            let lower = text.to_lowercase();

            if self.active_auth_prompt.is_none() {
                let mut found_2fa = false;
                for kw in &keywords {
                    if lower.contains(kw) {
                        self.active_auth_prompt = Some(AuthPrompt {
                            title: "2FA / OTP Verification".to_string(),
                            prompt_line: text.trim().to_string(),
                            is_secret: false,
                        });
                        self.auth_input.clear();
                        found_2fa = true;
                        break;
                    }
                }

                if !found_2fa {
                    if lower.contains("passphrase for key") || lower.contains("enter passphrase") {
                        self.active_auth_prompt = Some(AuthPrompt {
                            title: "SSH Key Passphrase".to_string(),
                            prompt_line: text.trim().to_string(),
                            is_secret: true,
                        });
                        self.auth_input.clear();
                    } else if (lower.contains("password:") || lower.contains("'s password:")) && !lower.contains("one-time") {
                        self.active_auth_prompt = Some(AuthPrompt {
                            title: "SSH Password Authentication".to_string(),
                            prompt_line: text.trim().to_string(),
                            is_secret: true,
                        });
                        self.auth_input.clear();
                    }
                }
            }
        }
    }

    fn is_cell_selected(&self, r: u16, c: u16) -> bool {
        if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
            if start == end {
                return false;
            }
            let (mut r1, mut c1) = start;
            let (mut r2, mut c2) = end;
            if r1 > r2 || (r1 == r2 && c1 > c2) {
                std::mem::swap(&mut r1, &mut r2);
                std::mem::swap(&mut c1, &mut c2);
            }
            if r < r1 || r > r2 {
                return false;
            }
            if r == r1 && r == r2 {
                return c >= c1 && c <= c2;
            }
            if r == r1 {
                return c >= c1;
            }
            if r == r2 {
                return c <= c2;
            }
            true
        } else {
            false
        }
    }

    fn extract_selected_text(&self) -> String {
        if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
            let (mut r1, mut c1) = start;
            let (mut r2, mut c2) = end;
            if r1 > r2 || (r1 == r2 && c1 > c2) {
                std::mem::swap(&mut r1, &mut r2);
                std::mem::swap(&mut c1, &mut c2);
            }

            let screen = self.parser.screen();
            let mut result = String::new();

            for r in r1..=r2 {
                let start_c = if r == r1 { c1 } else { 0 };
                let end_c = if r == r2 { c2 } else { self.cols.saturating_sub(1) };
                let mut line = String::new();

                for c in start_c..=end_c {
                    if let Some(cell) = screen.cell(r, c) {
                        let text = cell.contents();
                        if text.is_empty() {
                            line.push(' ');
                        } else {
                            line.push_str(&text);
                        }
                    }
                }
                result.push_str(line.trim_end());
                if r != r2 {
                    result.push('\n');
                }
            }
            result
        } else {
            String::new()
        }
    }

    fn render_auth_modal(&mut self, ctx: &egui::Context, toast: &mut Option<(String, std::time::Instant)>) {
        let prompt = match self.active_auth_prompt.clone() {
            Some(p) => p,
            None => return,
        };

        egui::Area::new(egui::Id::new("center_auth_prompt"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(COLOR_BG_CARD)
                    .stroke(egui::Stroke::new(1.5_f32, COLOR_ACCENT))
                    .rounding(8.0)
                    .inner_margin(egui::Margin::same(20.0))
                    .show(ui, |ui| {
                        ui.set_width(420.0);

                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new(&prompt.title)
                                    .strong()
                                    .size(16.0)
                                    .color(COLOR_ACCENT),
                            );
                        });
                        ui.add_space(8.0);

                        if !prompt.prompt_line.is_empty() {
                            egui::Frame::none()
                                .fill(COLOR_BG_PANEL)
                                .rounding(4.0)
                                .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(&prompt.prompt_line)
                                            .small()
                                            .color(COLOR_TEXT_PRIMARY),
                                    );
                                });
                            ui.add_space(8.0);
                        }

                        let label_text = if prompt.is_secret {
                            "Enter Password / Passphrase:"
                        } else {
                            "Enter 6-Digit OTP / Token:"
                        };
                        ui.label(egui::RichText::new(label_text).small().color(COLOR_TEXT_MUTED));
                        ui.add_space(4.0);

                        let mut submit = false;

                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            submit = true;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            self.active_auth_prompt = None;
                        }

                        ui.horizontal(|ui| {
                            let edit = egui::TextEdit::singleline(&mut self.auth_input)
                                .password(prompt.is_secret && !self.auth_show_secret)
                                .desired_width(
                                    ui.available_width()
                                        - if prompt.is_secret { 65.0 } else { 0.0 },
                                );
                            let res = ui.add(edit);
                            res.request_focus();

                            if prompt.is_secret {
                                if ui.button(if self.auth_show_secret { "Hide" } else { "Show" }).clicked() {
                                    self.auth_show_secret = !self.auth_show_secret;
                                }
                            }
                        });

                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            if ui.button(egui::RichText::new("Submit").strong()).clicked() {
                                submit = true;
                            }

                            if !prompt.is_secret {
                                if ui.button("Paste Clipboard").clicked() {
                                    if let Some(text) = get_system_clipboard_text() {
                                        self.auth_input = text.trim().to_string();
                                    }
                                }
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Dismiss / Terminal").clicked() {
                                    self.active_auth_prompt = None;
                                }
                            });
                        });

                        if submit {
                            self.send_auth_response(&self.auth_input);
                            self.auth_input.clear();
                            self.active_auth_prompt = None;
                            *toast = Some((
                                "Authentication submitted".to_string(),
                                std::time::Instant::now(),
                            ));
                        }
                    });
            });
    }

    fn handle_keyboard_events(&mut self, ctx: &egui::Context, settings: &AppSettings) {
        ctx.input(|i| {
            // Direct Ctrl shortcuts (SIGINT / line clearing / navigation)
            if i.modifiers.ctrl && !i.modifiers.shift && !i.modifiers.alt {
                if i.key_pressed(egui::Key::C) {
                    self.send_input("\x03"); // Cancel current command/input line
                    return;
                }
                if i.key_pressed(egui::Key::X) {
                    self.send_input("\x18"); // Cancel / CAN
                    return;
                }
                if i.key_pressed(egui::Key::U) {
                    self.send_input("\x15"); // Clear line backwards
                    return;
                }
                if i.key_pressed(egui::Key::K) {
                    self.send_input("\x0b"); // Kill line forwards
                    return;
                }
                if i.key_pressed(egui::Key::L) {
                    self.send_input("\x0c"); // Clear screen
                    return;
                }
                if i.key_pressed(egui::Key::D) {
                    self.send_input("\x04"); // EOF
                    return;
                }
                if i.key_pressed(egui::Key::Z) {
                    self.send_input("\x1a"); // Suspend
                    return;
                }
                if i.key_pressed(egui::Key::A) {
                    self.send_input("\x01"); // Start of line
                    return;
                }
                if i.key_pressed(egui::Key::E) {
                    self.send_input("\x05"); // End of line
                    return;
                }
                if i.key_pressed(egui::Key::W) {
                    self.send_input("\x17"); // Delete word
                    return;
                }
            }

            for event in &i.events {
                match event {
                    egui::Event::Copy => {
                        // When Ctrl+C is intercepted by egui, send SIGINT to cancel line if not selecting text
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
                        // Keyboard shortcut pasting (Ctrl+Shift+V or Shift+Insert)
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
                                egui::Key::Tab => Some(b"\t".to_vec()),
                                egui::Key::Escape => Some(b"\x1b".to_vec()),
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
        // 1. Auth modal (if active)
        self.render_auth_modal(ui.ctx(), toast);

        // 2. Keyboard routing (if no modal & no text widget has focus)
        if self.active_auth_prompt.is_none() && !ui.ctx().wants_keyboard_input() {
            self.handle_keyboard_events(ui.ctx(), settings);
        }

        // 3. Dynamic Resize & Painter
        let font_size = 14.0;
        let char_width = 8.4;
        let row_height = 17.5;

        let avail = ui.available_size();
        let new_cols = ((avail.x - 20.0) / char_width).max(20.0) as u16;
        let new_rows = ((avail.y - 20.0) / row_height).max(5.0) as u16;

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

        egui::Frame::none()
            .fill(COLOR_BG_MAIN)
            .inner_margin(egui::Margin::same(10.0))
            .show(ui, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(
                        self.cols as f32 * char_width,
                        self.rows as f32 * row_height,
                    ),
                    egui::Sense::click_and_drag(),
                );

                // Mouse Pointer Selection Handling
                let is_primary_down = ui.input(|i| i.pointer.primary_down());

                if let Some(pos) = response.interact_pointer_pos() {
                    let rel_x = (pos.x - rect.min.x).max(0.0);
                    let rel_y = (pos.y - rect.min.y).max(0.0);
                    let c = ((rel_x / char_width) as u16).min(self.cols.saturating_sub(1));
                    let r = ((rel_y / row_height) as u16).min(self.rows.saturating_sub(1));

                    if response.drag_started_by(egui::PointerButton::Primary) {
                        self.selection_start = Some((r, c));
                        self.selection_end = Some((r, c));
                        self.is_dragging_selection = true;
                    } else if self.is_dragging_selection && is_primary_down {
                        self.selection_end = Some((r, c));
                    }
                }

                // Copy on Drag Release & Dismiss highlight immediately
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
                    self.selection_start = None;
                    self.selection_end = None;
                }

                // Single click clears any lingering selection
                if response.clicked_by(egui::PointerButton::Primary) {
                    self.selection_start = None;
                    self.selection_end = None;
                    self.is_dragging_selection = false;
                }

                // Paste on Right Click from system clipboard
                let pointer_pos = ui.input(|i| i.pointer.hover_pos().unwrap_or(egui::Pos2::ZERO));
                let is_hovered = rect.contains(pointer_pos);

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

                // Draw Terminal Character Grid
                let screen = self.parser.screen();
                let (rows, cols) = screen.size();
                let (cursor_r, cursor_c) = screen.cursor_position();
                let hide_cursor = screen.hide_cursor();

                let show_cursor = !hide_cursor
                    && (!settings.cursor_blink
                        || (ui.input(|i| (i.time * 2.0).fract() < 0.5)));

                for r in 0..rows {
                    let row_y = rect.min.y + r as f32 * row_height;
                    let mut job = egui::text::LayoutJob::default();
                    job.wrap.max_width = f32::INFINITY;

                    for c in 0..cols {
                        let is_cursor = show_cursor && (r == cursor_r && c == cursor_c);
                        let is_selected = self.is_cell_selected(r, c);
                        let default_cell = vt100::Cell::default();
                        let cell = screen.cell(r, c).unwrap_or(&default_cell);
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
                    ui.painter().galley(egui::pos2(rect.min.x, row_y), galley, egui::Color32::WHITE);
                }
            });
    }
}
