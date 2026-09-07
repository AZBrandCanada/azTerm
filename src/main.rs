mod settings;
mod sftp;
mod ssh;
mod terminal;
mod theme;

use eframe::egui;
use portable_pty::CommandBuilder;
use settings::{AppSettings, BackspaceSequence};
use sftp::SftpBrowser;
use ssh::{SavedKeyEntry, SshAuthType, SshProfile, SshStore};
use terminal::TerminalSession;
use theme::*;

#[derive(PartialEq, Eq)]
enum ActiveView {
    Terminal,
    SshBookmarks,
    SftpBrowser,
    Settings,
}

#[derive(PartialEq, Eq)]
enum SshSubView {
    Profiles,
    KeysManager,
}

struct AppState {
    settings: AppSettings,
    ssh_store: SshStore,
    sessions: Vec<TerminalSession>,
    sftp: SftpBrowser,
    active_tab_idx: usize,
    next_tab_id: usize,
    active_view: ActiveView,
    ssh_subview: SshSubView,

    // Profile Modal / Form
    show_new_profile_modal: bool,
    new_ssh_name: String,
    new_ssh_host: String,
    new_ssh_port: String,
    new_ssh_user: String,
    new_ssh_auth_choice: usize, // 0: Password, 1: Key File, 2: Paste Key
    new_ssh_key_path: String,
    new_ssh_pasted_key: String,

    // Key Generator Modal
    show_keygen_modal: bool,
    keygen_name: String,
    generated_pub_key: String,
    keygen_status: String,

    // Settings search
    settings_search: String,
}

impl AppState {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = AppSettings::load();
        let ssh_store = SshStore::load();

        let mut app = Self {
            settings,
            ssh_store,
            sessions: Vec::new(),
            sftp: SftpBrowser::new(),
            active_tab_idx: 0,
            next_tab_id: 1,
            active_view: ActiveView::Terminal,
            ssh_subview: SshSubView::Profiles,

            show_new_profile_modal: false,
            new_ssh_name: String::new(),
            new_ssh_host: String::new(),
            new_ssh_port: "22".to_string(),
            new_ssh_user: "root".to_string(),
            new_ssh_auth_choice: 0,
            new_ssh_key_path: String::new(),
            new_ssh_pasted_key: String::new(),

            show_keygen_modal: false,
            keygen_name: "prod_server".to_string(),
            generated_pub_key: String::new(),
            keygen_status: String::new(),

            settings_search: String::new(),
        };

        app.spawn_local_terminal(cc.egui_ctx.clone());
        app
    }

    fn spawn_local_terminal(&mut self, ctx: egui::Context) {
        let shell = if !self.settings.default_shell.trim().is_empty() {
            self.settings.default_shell.clone()
        } else if cfg!(windows) {
            "powershell.exe".to_string()
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
        };

        let mut c = CommandBuilder::new(shell);
        c.env("TERM", "xterm-256color");

        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let session = TerminalSession::new(id, format!("Local #{}", id), c, ctx);
        self.sessions.push(session);
        self.active_tab_idx = self.sessions.len() - 1;
        self.active_view = ActiveView::Terminal;
    }

    fn spawn_ssh_terminal(&mut self, profile: &SshProfile, ctx: egui::Context) {
        let cmd = profile.to_command();
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let session = TerminalSession::new(id, format!("SSH: {}", profile.name), cmd, ctx);
        self.sessions.push(session);
        self.active_tab_idx = self.sessions.len() - 1;
        self.active_view = ActiveView::Terminal;
    }
}

impl eframe::App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll output on active terminal sessions
        for s in &mut self.sessions {
            s.poll_updates(&self.settings);
        }

        // Top Navigation Bar
        egui::TopBottomPanel::top("top_nav")
            .frame(egui::Frame::none().fill(COLOR_BG_PANEL).inner_margin(egui::Margin::symmetric(14.0, 8.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("AZTerm")
                            .color(COLOR_ACCENT)
                            .strong()
                            .size(16.0),
                    );
                    ui.add_space(12.0);

                    if ui.selectable_label(self.active_view == ActiveView::Terminal, "Terminal").clicked() {
                        self.active_view = ActiveView::Terminal;
                    }
                    if ui.selectable_label(self.active_view == ActiveView::SshBookmarks, "SSH Profiles").clicked() {
                        self.active_view = ActiveView::SshBookmarks;
                    }
                    if ui.selectable_label(self.active_view == ActiveView::SftpBrowser, "SFTP Explorer").clicked() {
                        self.active_view = ActiveView::SftpBrowser;
                    }
                    if ui.selectable_label(self.active_view == ActiveView::Settings, "Settings").clicked() {
                        self.active_view = ActiveView::Settings;
                    }

                    ui.separator();

                    let mut tab_to_close: Option<usize> = None;
                    for (i, session) in self.sessions.iter().enumerate() {
                        let is_active = self.active_view == ActiveView::Terminal && self.active_tab_idx == i;
                        ui.horizontal(|ui| {
                            let tab_btn = ui.selectable_label(is_active, &session.title);
                            if tab_btn.clicked() {
                                self.active_tab_idx = i;
                                self.active_view = ActiveView::Terminal;
                            }
                            if self.sessions.len() > 1 && ui.small_button("×").clicked() {
                                tab_to_close = Some(i);
                            }
                        });
                    }

                    if let Some(i) = tab_to_close {
                        self.sessions.remove(i);
                        if self.active_tab_idx >= self.sessions.len() && !self.sessions.is_empty() {
                            self.active_tab_idx = self.sessions.len() - 1;
                        }
                    }

                    if ui.button("+ New Shell").clicked() {
                        self.spawn_local_terminal(ctx.clone());
                    }
                });
            });

        // Keygen Modal Window
        if self.show_keygen_modal {
            egui::Window::new("Generate Ed25519 SSH Keypair")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_width(480.0);
                    ui.add_space(6.0);
                    ui.label("Key identifier name:");
                    ui.text_edit_singleline(&mut self.keygen_name);
                    ui.add_space(8.0);

                    if ui.button("Generate Keypair").clicked() {
                        match SshStore::generate_ed25519_keypair(&self.keygen_name) {
                            Ok((priv_path, pub_key)) => {
                                self.generated_pub_key = pub_key;
                                self.keygen_status = format!("Key generated and saved to: {}", priv_path);
                            }
                            Err(e) => {
                                self.keygen_status = format!("Error: {}", e);
                            }
                        }
                    }

                    if !self.generated_pub_key.is_empty() {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Public Key (Paste into remote ~/.ssh/authorized_keys):").strong());
                        ui.text_edit_multiline(&mut self.generated_pub_key);
                        if ui.button("Copy Public Key to Clipboard").clicked() {
                            ui.output_mut(|o| o.copied_text = self.generated_pub_key.clone());
                        }
                    }

                    if !self.keygen_status.is_empty() {
                        ui.add_space(6.0);
                        ui.label(&self.keygen_status);
                    }

                    ui.separator();
                    if ui.button("Close").clicked() {
                        self.show_keygen_modal = false;
                    }
                });
        }

        // New Profile Modal Window
        if self.show_new_profile_modal {
            egui::Window::new("Create / Edit SSH Profile")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_width(520.0);
                    egui::Grid::new("profile_grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                        ui.label("Profile Name:");
                        ui.text_edit_singleline(&mut self.new_ssh_name);
                        ui.end_row();

                        ui.label("Host / IP:");
                        ui.text_edit_singleline(&mut self.new_ssh_host);
                        ui.end_row();

                        ui.label("Port:");
                        ui.text_edit_singleline(&mut self.new_ssh_port);
                        ui.end_row();

                        ui.label("Username:");
                        ui.text_edit_singleline(&mut self.new_ssh_user);
                        ui.end_row();

                        ui.label("Authentication:");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.new_ssh_auth_choice, 0, "Password / Agent");
                            ui.radio_value(&mut self.new_ssh_auth_choice, 1, "Key File");
                            ui.radio_value(&mut self.new_ssh_auth_choice, 2, "Paste Key");
                        });
                        ui.end_row();

                        if self.new_ssh_auth_choice == 1 {
                            ui.label("Key File Path:");
                            ui.text_edit_singleline(&mut self.new_ssh_key_path);
                            ui.end_row();
                        } else if self.new_ssh_auth_choice == 2 {
                            ui.label("Paste Private Key:");
                            ui.text_edit_multiline(&mut self.new_ssh_pasted_key);
                            ui.end_row();
                        }
                    });

                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save Profile").clicked() {
                            let port = self.new_ssh_port.parse().unwrap_or(22);
                            let mut profile = SshProfile::new(&self.new_ssh_name, &self.new_ssh_host, port, &self.new_ssh_user);

                            if self.new_ssh_auth_choice == 1 && !self.new_ssh_key_path.trim().is_empty() {
                                profile.auth_type = SshAuthType::KeyFile(self.new_ssh_key_path.clone());
                            } else if self.new_ssh_auth_choice == 2 && !self.new_ssh_pasted_key.trim().is_empty() {
                                let key_id = format!("{}_{}", self.new_ssh_host, port);
                                let _ = SshStore::save_pasted_key(&key_id, &self.new_ssh_pasted_key);
                                profile.auth_type = SshAuthType::PastedKey { key_id };
                            }

                            self.ssh_store.profiles.push(profile);
                            self.ssh_store.save();
                            self.show_new_profile_modal = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_new_profile_modal = false;
                        }
                    });
                });
        }

        // Central Workspace Area
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(COLOR_BG_MAIN))
            .show(ctx, |ui| match self.active_view {
                ActiveView::Terminal => {
                    if self.settings.show_sftp_split_view {
                        ui.columns(2, |columns| {
                            if let Some(session) = self.sessions.get_mut(self.active_tab_idx) {
                                session.render(&mut columns[0], &self.settings);
                            }
                            card_frame().show(&mut columns[1], |ui| {
                                ui.heading("SFTP Sync Explorer");
                                ui.separator();
                                self.sftp.render(ui);
                            });
                        });
                    } else if let Some(session) = self.sessions.get_mut(self.active_tab_idx) {
                        session.render(ui, &self.settings);
                    } else {
                        ui.centered_and_justified(|ui| {
                            if ui.button("Open Shell Session").clicked() {
                                self.spawn_local_terminal(ctx.clone());
                            }
                        });
                    }
                }
                ActiveView::SshBookmarks => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            ui.heading(egui::RichText::new("SSH Manager").color(COLOR_TEXT_PRIMARY));
                            ui.add_space(16.0);
                            if ui.selectable_label(self.ssh_subview == SshSubView::Profiles, "Connections").clicked() {
                                self.ssh_subview = SshSubView::Profiles;
                            }
                            if ui.selectable_label(self.ssh_subview == SshSubView::KeysManager, "Saved Keypairs").clicked() {
                                self.ssh_subview = SshSubView::KeysManager;
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Key Generator").clicked() {
                                    self.show_keygen_modal = true;
                                }
                                if ui.button("+ New SSH Profile").clicked() {
                                    self.new_ssh_name = "My Server".to_string();
                                    self.new_ssh_host = "192.168.1.100".to_string();
                                    self.show_new_profile_modal = true;
                                }
                            });
                        });

                        ui.add_space(12.0);

                        match self.ssh_subview {
                            SshSubView::Profiles => {
                                let profiles = self.ssh_store.profiles.clone();
                                let mut delete_idx: Option<usize> = None;
                                let mut connect_profile: Option<SshProfile> = None;
                                let mut open_sftp = false;

                                for (idx, profile) in profiles.iter().enumerate() {
                                    card_frame().show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.vertical(|ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(egui::RichText::new(&profile.name).strong().size(15.0).color(COLOR_TEXT_PRIMARY));
                                                    let auth_badge = match &profile.auth_type {
                                                        SshAuthType::PasswordOrAgent => "[Password/Agent]",
                                                        SshAuthType::KeyFile(_) => "[Key File]",
                                                        SshAuthType::PastedKey { .. } => "[Inline Key]",
                                                    };
                                                    ui.label(egui::RichText::new(auth_badge).color(COLOR_ACCENT).small());
                                                });
                                                ui.label(
                                                    egui::RichText::new(format!("{}@{}:{}", profile.username, profile.host, profile.port))
                                                        .color(COLOR_TEXT_MUTED),
                                                );
                                            });

                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if ui.button("Delete").clicked() {
                                                    delete_idx = Some(idx);
                                                }
                                                if ui.button("SFTP").clicked() {
                                                    open_sftp = true;
                                                }
                                                if ui.button("Connect").clicked() {
                                                    connect_profile = Some(profile.clone());
                                                }
                                            });
                                        });
                                    });
                                    ui.add_space(8.0);
                                }

                                if let Some(i) = delete_idx {
                                    self.ssh_store.profiles.remove(i);
                                    self.ssh_store.save();
                                }
                                if open_sftp {
                                    self.active_view = ActiveView::SftpBrowser;
                                }
                                if let Some(profile) = connect_profile {
                                    self.spawn_ssh_terminal(&profile, ctx.clone());
                                }
                            }
                            SshSubView::KeysManager => {
                                let saved_keys = SshStore::list_saved_keys();
                                if saved_keys.is_empty() {
                                    card_frame().show(ui, |ui| {
                                        ui.label(egui::RichText::new("No SSH keys stored in ~/.config/azterm/keys yet.").color(COLOR_TEXT_MUTED));
                                    });
                                } else {
                                    let mut key_to_delete: Option<String> = None;

                                    for key in saved_keys {
                                        card_frame().show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new(&key.file_name).strong().color(COLOR_ACCENT));
                                                    ui.label(egui::RichText::new(format!("Path: {}", key.priv_path.display())).small().color(COLOR_TEXT_MUTED));
                                                });

                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if ui.button("Delete Key").clicked() {
                                                        key_to_delete = Some(key.file_name.clone());
                                                    }
                                                    if let Some(ref pub_k) = key.pub_key_content {
                                                        if ui.button("Copy Public Key").clicked() {
                                                            ui.output_mut(|o| o.copied_text = pub_k.clone());
                                                        }
                                                    }
                                                });
                                            });
                                        });
                                        ui.add_space(8.0);
                                    }

                                    if let Some(name) = key_to_delete {
                                        SshStore::delete_key_files(&name);
                                    }
                                }
                            }
                        }
                    });
                }
                ActiveView::SftpBrowser => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(14.0);
                        card_frame().show(ui, |ui| {
                            self.sftp.render(ui);
                        });
                    });
                }
                ActiveView::Settings => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            ui.heading(egui::RichText::new("Preferences & Configuration").color(COLOR_TEXT_PRIMARY));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add(egui::TextEdit::singleline(&mut self.settings_search).hint_text("Search settings...").desired_width(180.0));
                            });
                        });
                        ui.add_space(14.0);

                        let mut changed = false;

                        // 2-Column Dashboard Layout
                        ui.columns(2, |columns| {
                            // Left Column: Shell & Terminal Settings + 2FA
                            columns[0].vertical(|ui| {
                                card_frame().show(ui, |ui| {
                                    ui.label(egui::RichText::new("Shell & Terminal Environment").strong().size(14.0).color(COLOR_ACCENT));
                                    ui.add_space(6.0);

                                    ui.label("Default Shell Path:");
                                    if ui.text_edit_singleline(&mut self.settings.default_shell).changed() {
                                        changed = true;
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Presets:").small().color(COLOR_TEXT_MUTED));
                                        if ui.small_button("bash").clicked() {
                                            self.settings.default_shell = "/bin/bash".to_string();
                                            changed = true;
                                        }
                                        if ui.small_button("zsh").clicked() {
                                            self.settings.default_shell = "/bin/zsh".to_string();
                                            changed = true;
                                        }
                                        if ui.small_button("fish").clicked() {
                                            self.settings.default_shell = "/bin/fish".to_string();
                                            changed = true;
                                        }
                                    });

                                    ui.add_space(10.0);
                                    ui.label("Terminal Log Directory:");
                                    if ui.text_edit_singleline(&mut self.settings.terminal_log_path).changed() {
                                        changed = true;
                                    }

                                    ui.add_space(10.0);
                                    ui.horizontal(|ui| {
                                        ui.label("Backspace Keycode:");
                                        egui::ComboBox::from_id_source("backspace_seq_select")
                                            .selected_text(match self.settings.backspace_sequence {
                                                BackspaceSequence::Delete127 => "^? (Delete 0x7F)",
                                                BackspaceSequence::Backspace8 => "^H (Backspace 0x08)",
                                            })
                                            .show_ui(ui, |ui| {
                                                if ui.selectable_value(&mut self.settings.backspace_sequence, BackspaceSequence::Delete127, "^? (Delete 0x7F)").clicked() {
                                                    changed = true;
                                                }
                                                if ui.selectable_value(&mut self.settings.backspace_sequence, BackspaceSequence::Backspace8, "^H (Backspace 0x08)").clicked() {
                                                    changed = true;
                                                }
                                            });
                                    });
                                });

                                ui.add_space(12.0);

                                card_frame().show(ui, |ui| {
                                    ui.label(egui::RichText::new("Terminal Interaction").strong().size(14.0).color(COLOR_ACCENT));
                                    ui.add_space(6.0);

                                    changed |= toggle_switch(ui, &mut self.settings.cursor_blink, "Cursor blink").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.paste_on_right_click, "Paste when right click").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.copy_on_select, "Copy selected text when select").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.right_click_select_word, "Right click auto select word").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.must_hold_ctrl_for_links, "Hold Ctrl/Meta to open links").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.show_command_suggestions, "Show command suggestions").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.save_terminal_log, "Save terminal log to file").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.add_timestamp_to_log, "Add timestamp to terminal log").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.auto_reconnect_terminal, "Auto reconnect on disconnect").changed();
                                });

                                ui.add_space(12.0);

                                card_frame().show(ui, |ui| {
                                    ui.label(egui::RichText::new("2FA & Security Triggers").strong().size(14.0).color(COLOR_ACCENT));
                                    ui.add_space(6.0);
                                    ui.label(egui::RichText::new("Keywords triggering verification prompt:").color(COLOR_TEXT_MUTED));
                                    if ui.text_edit_singleline(&mut self.settings.two_factor_keywords).changed() {
                                        changed = true;
                                    }
                                });
                            });

                            // Right Column: SFTP, Application & System
                            columns[1].vertical(|ui| {
                                card_frame().show(ui, |ui| {
                                    ui.label(egui::RichText::new("SFTP & File Transfer").strong().size(14.0).color(COLOR_ACCENT));
                                    ui.add_space(6.0);

                                    changed |= toggle_switch(ui, &mut self.settings.show_sftp_split_view, "Show terminal and SFTP in split view").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.sftp_path_sync, "SFTP path sync with terminal").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.auto_refresh_sftp, "Auto refresh when switch to SFTP").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.show_hidden_sftp, "Show hidden files on SFTP start").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.disable_sftp_history, "Disable SFTP transfer history").changed();
                                });

                                ui.add_space(12.0);

                                card_frame().show(ui, |ui| {
                                    ui.label(egui::RichText::new("Application & System").strong().size(14.0).color(COLOR_ACCENT));
                                    ui.add_space(6.0);

                                    changed |= toggle_switch(ui, &mut self.settings.open_default_tab, "Open default tab when app starts").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.disable_connection_history, "Disable connection history").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.support_screen_reader, "Support screen reader in terminal").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.use_system_titlebar, "Use system title bar").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.confirm_before_exit, "Confirm before exit").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.hide_ip, "Hide IP address in status").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.allow_multi_instance, "Allow multi-instance").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.disable_developer_tools, "Disable developer tools").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.debug_mode, "Debug mode").changed();
                                    changed |= toggle_switch(ui, &mut self.settings.check_updates, "Check update on app start").changed();
                                });

                                ui.add_space(12.0);

                                card_frame().show(ui, |ui| {
                                    ui.label(egui::RichText::new("Preferences Management").strong().size(14.0).color(COLOR_ACCENT));
                                    ui.add_space(6.0);
                                    if ui.button("Restore All Defaults").clicked() {
                                        self.settings = AppSettings::default();
                                        self.settings.save();
                                    }
                                });
                            });
                        });

                        if changed {
                            self.settings.save();
                        }
                    });
                }
            });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1080.0, 700.0])
            .with_title("AZTerm"),
        ..Default::default()
    };
    eframe::run_native(
        "AZTerm",
        options,
        Box::new(|cc| Ok(Box::new(AppState::new(cc)))),
    )
}
