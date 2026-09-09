use crate::ssh::{SshAuthType, SshKeyAlgorithm, SshProfile, SshStore};
use crate::{AppState, InstallMethod};
use eframe::egui;
use std::io::Write;

pub fn render_update_modal(app: &mut AppState, ctx: &egui::Context) {
    if let Some(ref new_tag) = app.available_update.clone() {
        egui::Window::new("AZTerm Update Available")
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(format!("A new version ({}) of AZTerm is ready!", new_tag))
                            .strong()
                            .size(15.0)
                            .color(app.theme.accent_color()),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("Current version: v{}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(app.theme.text_muted_color()),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("Detected installation: {}", app.install_method.display_name()))
                            .small()
                            .color(app.theme.text_primary_color()),
                    );

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    match &app.install_method {
                        InstallMethod::ScriptInstalled | InstallMethod::PackageManager(_) => {
                            ui.label("Would you like to run the official updater script in a new terminal session?");
                            ui.add_space(6.0);
                            egui::Frame::none()
                                .fill(app.theme.bg_panel_color())
                                .rounding(4.0)
                                .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                                .show(ui, |ui| {
                                    ui.monospace("curl -sSL https://raw.githubusercontent.com/AZBrandCanada/azTerm/main/install.sh | bash");
                                });
                        }
                        InstallMethod::AppImage => {
                            ui.label("Download the latest standalone AppImage binary from GitHub:");
                        }
                        InstallMethod::Windows => {
                            ui.label("Download the latest Windows ZIP archive from GitHub:");
                        }
                        InstallMethod::MacOS => {
                            ui.label("Download the latest macOS universal package from GitHub:");
                        }
                        InstallMethod::ManualBuild => {
                            ui.label("You can recompile with cargo or run the installer script:");
                        }
                    }

                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        match &app.install_method {
                            InstallMethod::ScriptInstalled | InstallMethod::ManualBuild => {
                                if ui.button(egui::RichText::new("Update Now (Run in Shell)").strong()).clicked() {
                                    app.run_script_update_in_terminal(ctx.clone());
                                }
                            }
                            _ => {}
                        }

                        let release_url = format!("https://github.com/AZBrandCanada/azTerm/releases/tag/{}", new_tag);
                        if ui.button("Open GitHub Release").clicked() {
                            ctx.open_url(egui::OpenUrl::new_tab(release_url));
                            app.show_update_modal = false;
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Later").clicked() {
                                app.show_update_modal = false;
                            }
                        });
                    });
                });
            });
    }
}

pub fn render_keygen_modal(app: &mut AppState, ctx: &egui::Context) {
    egui::Window::new("Generate SSH Keypair")
        .collapsible(false)
        .resizable(true)
        .default_width(540.0)
        .max_height(520.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label("Key identifier name:");
                        ui.text_edit_singleline(&mut app.keygen_name);
                        ui.add_space(8.0);

                        ui.label("Key Type / Algorithm:");
                        let algo_label = match app.keygen_algo {
                            1 => SshKeyAlgorithm::Rsa4096.display_name(),
                            2 => SshKeyAlgorithm::Ecdsa384.display_name(),
                            3 => SshKeyAlgorithm::Ecdsa256.display_name(),
                            _ => SshKeyAlgorithm::Ed25519.display_name(),
                        };

                        egui::ComboBox::from_id_source("keygen_algo_combo")
                            .width(360.0)
                            .selected_text(algo_label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut app.keygen_algo, 0, SshKeyAlgorithm::Ed25519.display_name());
                                ui.selectable_value(&mut app.keygen_algo, 1, SshKeyAlgorithm::Rsa4096.display_name());
                                ui.selectable_value(&mut app.keygen_algo, 2, SshKeyAlgorithm::Ecdsa384.display_name());
                                ui.selectable_value(&mut app.keygen_algo, 3, SshKeyAlgorithm::Ecdsa256.display_name());
                            });

                        ui.add_space(10.0);

                        if ui.button(egui::RichText::new("⚡ Generate Keypair").strong()).clicked() {
                            let algo = match app.keygen_algo {
                                1 => SshKeyAlgorithm::Rsa4096,
                                2 => SshKeyAlgorithm::Ecdsa384,
                                3 => SshKeyAlgorithm::Ecdsa256,
                                _ => SshKeyAlgorithm::Ed25519,
                            };
                            match SshStore::generate_keypair(&app.keygen_name, algo) {
                                Ok((priv_path, pub_key)) => {
                                    app.generated_pub_key = pub_key;
                                    app.keygen_status = format!("Key generated and saved to: {}", priv_path);
                                }
                                Err(e) => {
                                    app.keygen_status = format!("Error: {}", e);
                                }
                            }
                        }

                        if !app.generated_pub_key.is_empty() {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new("Public Key (Paste into remote ~/.ssh/authorized_keys):").strong());
                            ui.add(
                                egui::TextEdit::multiline(&mut app.generated_pub_key)
                                    .desired_rows(5)
                                    .desired_width(f32::INFINITY),
                            );
                            if ui.button("Copy Public Key to Clipboard").clicked() {
                                if let Ok(mut cb) = arboard::Clipboard::new() {
                                    let _ = cb.set_text(app.generated_pub_key.clone());
                                    app.set_toast("Public key copied to clipboard");
                                }
                            }
                        }

                        if !app.keygen_status.is_empty() {
                            ui.add_space(6.0);
                            ui.label(&app.keygen_status);
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        app.show_keygen_modal = false;
                    }
                });
            });
        });
}

pub fn render_profile_modal(app: &mut AppState, ctx: &egui::Context) {
    let modal_title = if app.editing_profile_id.is_some() {
        "Edit SSH Profile"
    } else {
        "Create New SSH Profile"
    };

    egui::Window::new(modal_title)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .max_height(580.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                egui::ScrollArea::vertical()
                    .max_height(460.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("profile_grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                            ui.label("Profile Name:");
                            ui.text_edit_singleline(&mut app.new_ssh_name);
                            ui.end_row();

                            ui.label("Host / IP:");
                            ui.text_edit_singleline(&mut app.new_ssh_host);
                            ui.end_row();

                            ui.label("Port:");
                            ui.text_edit_singleline(&mut app.new_ssh_port);
                            ui.end_row();

                            ui.label("Username:");
                            ui.text_edit_singleline(&mut app.new_ssh_user);
                            ui.end_row();

                            ui.label("Authentication:");
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut app.new_ssh_auth_choice, 0, "Password / Agent");
                                ui.radio_value(&mut app.new_ssh_auth_choice, 1, "Key File / Saved Key");
                                ui.radio_value(&mut app.new_ssh_auth_choice, 2, "Paste Key");
                            });
                            ui.end_row();

                            if app.new_ssh_auth_choice == 1 {
                                let saved_keys = SshStore::list_saved_keys();
                                if !saved_keys.is_empty() {
                                    ui.label("Saved Keypair:");
                                    ui.horizontal(|ui| {
                                        let current_label = saved_keys.iter()
                                            .find(|k| k.priv_path.to_string_lossy() == app.new_ssh_key_path)
                                            .map(|k| k.file_name.as_str())
                                            .unwrap_or("-- Select from saved keys --");

                                        egui::ComboBox::from_id_source("profile_saved_keys_combo")
                                            .width(230.0)
                                            .selected_text(current_label)
                                            .show_ui(ui, |ui| {
                                                for key in &saved_keys {
                                                    let is_selected = key.priv_path.to_string_lossy() == app.new_ssh_key_path;
                                                    if ui.selectable_label(is_selected, &key.file_name).clicked() {
                                                        app.new_ssh_key_path = key.priv_path.to_string_lossy().to_string();
                                                    }
                                                }
                                            });

                                        if ui.small_button("+ Gen New").on_hover_text("Open SSH Key Generator modal").clicked() {
                                            app.show_keygen_modal = true;
                                        }
                                    });
                                    ui.end_row();
                                }

                                ui.label("Key File Path:");
                                ui.horizontal(|ui| {
                                    ui.text_edit_singleline(&mut app.new_ssh_key_path);
                                    if saved_keys.is_empty() {
                                        if ui.small_button("+ Generate Key").clicked() {
                                            app.show_keygen_modal = true;
                                        }
                                    }
                                });
                                ui.end_row();
                            } else if app.new_ssh_auth_choice == 2 {
                                ui.label("Paste Private Key:");
                                ui.add(
                                    egui::TextEdit::multiline(&mut app.new_ssh_pasted_key)
                                        .desired_rows(6)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("-----BEGIN OPENSSH PRIVATE KEY-----\n..."),
                                );
                                ui.end_row();
                            }
                        });
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Save Profile").clicked() {
                        let port = app.new_ssh_port.parse().unwrap_or(22);
                        let auth_type = if app.new_ssh_auth_choice == 1 && !app.new_ssh_key_path.trim().is_empty() {
                            SshStore::ensure_secure_permissions(&app.new_ssh_key_path);
                            SshAuthType::KeyFile(app.new_ssh_key_path.clone())
                        } else if app.new_ssh_auth_choice == 2 && !app.new_ssh_pasted_key.trim().is_empty() {
                            let key_id = format!("{}_{}", app.new_ssh_host, port);
                            let _ = SshStore::save_pasted_key(&key_id, &app.new_ssh_pasted_key);
                            SshAuthType::PastedKey { key_id }
                        } else {
                            SshAuthType::PasswordOrAgent
                        };

                        if let Some(ref edit_id) = app.editing_profile_id {
                            if let Some(existing) = app.ssh_store.profiles.iter_mut().find(|p| p.id == *edit_id) {
                                existing.name = app.new_ssh_name.clone();
                                existing.host = app.new_ssh_host.clone();
                                existing.port = port;
                                existing.username = app.new_ssh_user.clone();
                                existing.auth_type = auth_type;
                            }
                            app.set_toast("SSH Profile Updated");
                        } else {
                            let mut profile = SshProfile::new(&app.new_ssh_name, &app.new_ssh_host, port, &app.new_ssh_user);
                            profile.auth_type = auth_type;
                            app.ssh_store.profiles.push(profile);
                            app.set_toast("SSH Profile Created");
                        }

                        app.ssh_store.save();
                        app.show_profile_modal = false;
                    }
                    if ui.button("Cancel").clicked() {
                        app.show_profile_modal = false;
                    }
                });
            });
        });
}

pub fn render_ssh_auth_modal(app: &mut AppState, ctx: &egui::Context) {
    let mut should_close = false;
    let mut refresh_pane_id: Option<String> = None;

    if let Some(ref mut modal) = app.ssh_auth_modal {
        let socket_path = SshStore::sockets_dir().join(format!("{}.sock", modal.profile.id));
        if socket_path.exists() {
            modal.is_connected = true;
        }

        egui::Window::new(format!("SSH Login: {}", modal.profile.name))
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Target:").strong());
                        ui.label(
                            egui::RichText::new(format!("{}@{}:{}", modal.profile.username, modal.profile.host, modal.profile.port))
                                .color(app.theme.accent_color()),
                        );
                    });
                    ui.add_space(4.0);

                    if modal.is_connected {
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new("✓ Successfully Authenticated!")
                                .strong()
                                .size(16.0)
                                .color(app.theme.success_color()),
                        );
                        ui.label("Remote session multiplexed. SFTP file transfer is now ready.");
                        ui.add_space(12.0);
                        if ui.button(egui::RichText::new("Continue to SFTP").strong()).clicked() {
                            refresh_pane_id = Some(modal.target_pane_id.clone());
                            should_close = true;
                        }
                        return;
                    }

                    ui.label(egui::RichText::new("Server Output:").small().color(app.theme.text_muted_color()));

                    let out_str = modal.output.lock().map(|s| s.clone()).unwrap_or_default();
                    egui::Frame::none()
                        .fill(app.theme.bg_main_color())
                        .stroke(egui::Stroke::new(1.0_f32, app.theme.border_color()))
                        .rounding(4.0)
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_source("ssh_modal_out_scroll")
                                .max_height(160.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&out_str)
                                                .monospace()
                                                .size(12.5)
                                                .color(app.theme.text_primary_color()),
                                        )
                                        .wrap(),
                                    );
                                });
                        });

                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Input:").strong());
                        let input_w = (ui.available_width() - 170.0).max(120.0);

                        let text_edit = egui::TextEdit::singleline(&mut modal.input_text)
                            .password(!modal.show_plain)
                            .hint_text("Password, OTP, or 'yes'...")
                            .desired_width(input_w);

                        let edit_resp = ui.add(text_edit);

                        if !edit_resp.has_focus() && !ui.input(|i| i.pointer.any_pressed()) {
                            edit_resp.request_focus();
                        }

                        let enter_pressed = edit_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if enter_pressed || ui.button(egui::RichText::new("Send ↵").strong()).clicked() {
                            let to_send = format!("{}\r", modal.input_text);
                            if let Ok(mut w) = modal.writer.lock() {
                                let _ = w.write_all(to_send.as_bytes());
                                let _ = w.flush();
                            }
                            modal.input_text.clear();
                            edit_resp.request_focus();
                            ui.ctx().request_repaint();
                        }

                        ui.checkbox(&mut modal.show_plain, "Show");
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            should_close = true;
                        }
                    });
                });
            });
    }

    if let Some(pane_id) = refresh_pane_id {
        if pane_id == "sftp_left" {
            app.sftp.left_pane.refresh();
        } else {
            app.sftp.right_pane.refresh();
        }
    }

    if should_close {
        app.ssh_auth_modal = None;
    }
}
