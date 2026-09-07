use crate::settings::{AppSettings, BackspaceSequence};
use crate::theme::*;
use eframe::egui;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtyPair, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

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

pub struct TerminalSession {
    pub id: usize,
    pub title: String,
    pub parser: vt100::Parser,
    pub rx: Receiver<Vec<u8>>,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub master_pty: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub rows: u16,
    pub cols: u16,
    pub detected_2fa: bool,
    pub otp_input: String,
}

impl TerminalSession {
    pub fn new(id: usize, title: String, cmd: CommandBuilder, ctx: egui::Context) -> Self {
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
            parser: vt100::Parser::new(rows, cols, 2000),
            rx,
            writer,
            master_pty,
            rows,
            cols,
            detected_2fa: false,
            otp_input: String::new(),
        }
    }

    pub fn send_input(&self, text: &str) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(text.as_bytes());
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
            let text = String::from_utf8_lossy(&bytes).to_lowercase();
            for kw in &keywords {
                if text.contains(kw) {
                    self.detected_2fa = true;
                    break;
                }
            }
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui, settings: &AppSettings) {
        // Interactive 2FA Banner
        if self.detected_2fa {
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(49, 46, 129))
                .inner_margin(egui::Margin::symmetric(14.0, 8.0))
                .rounding(6.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("[2FA / OTP Verification Requested]:")
                                .color(egui::Color32::from_rgb(244, 114, 182))
                                .strong(),
                        );
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.otp_input)
                                .desired_width(140.0)
                                .hint_text("Enter OTP Code"),
                        );
                        if (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                            || ui.button("Submit").clicked()
                        {
                            let mut code = self.otp_input.clone();
                            code.push('\r');
                            self.send_input(&code);
                            self.otp_input.clear();
                            self.detected_2fa = false;
                        }
                        if ui.button("Dismiss").clicked() {
                            self.detected_2fa = false;
                        }
                    });
                });
            ui.add_space(4.0);
        }

        // Key Routing
        ui.input(|i| {
            for event in &i.events {
                match event {
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
                        let mut bytes: Option<Vec<u8>> = None;
                        if modifiers.ctrl {
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
                        } else {
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
                    egui::Event::Paste(text) => {
                        self.send_input(text);
                    }
                    _ => {}
                }
            }
        });

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
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

                    let screen = self.parser.screen();
                    let (rows, cols) = screen.size();
                    let (cursor_r, cursor_c) = screen.cursor_position();
                    let hide_cursor = screen.hide_cursor();

                    let show_cursor = !hide_cursor
                        && (!settings.cursor_blink
                            || (ui.input(|i| (i.time * 2.0).fract() < 0.5)));

                    for r in 0..rows {
                        let mut job = egui::text::LayoutJob::default();
                        job.wrap.max_width = f32::INFINITY;

                        for c in 0..cols {
                            let is_cursor = show_cursor && (r == cursor_r && c == cursor_c);
                            let default_cell = vt100::Cell::default();
                            let cell = screen.cell(r, c).unwrap_or(&default_cell);
                            let cell_text = cell.contents();
                            let display_char: &str = if cell_text.is_empty() { " " } else { &cell_text };

                            let mut fg = vt_to_egui_color(cell.fgcolor(), false);
                            let mut bg = vt_to_egui_color(cell.bgcolor(), true);

                            if cell.inverse() || is_cursor {
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
                        let label_resp = ui.label(job);

                        if settings.paste_on_right_click
                            && label_resp.clicked_by(egui::PointerButton::Secondary)
                        {
                            let clip = ui.output(|o| o.copied_text.clone());
                            if !clip.is_empty() {
                                self.send_input(&clip);
                            }
                        }
                    }
                });
            });
    }
}
