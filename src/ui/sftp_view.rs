// src/ui/sftp_view.rs
use crate::sftp::SftpTarget;
use crate::ssh::SshStore;
use crate::AppState;
use eframe::egui;

pub fn render_sftp_browser_view(app: &mut AppState, ui: &mut egui::Ui) {
    ui.vertical(|ui| {
        ui.add_space(4.0);

        // Compact Top Toolbar
        app.theme.card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Dual-Session SFTP File Transfer")
                        .strong()
                        .size(15.0)
                        .color(app.theme.accent_color()),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Download (Right -> Left)").on_hover_text("Download selected file/folder from Right to Left").clicked() {
                        app.sftp.download_selected();
                    }
                    if ui.button("Upload (Left -> Right)").on_hover_text("Upload selected file/folder from Left to Right").clicked() {
                        app.sftp.upload_selected();
                    }

                    let transfer_count = app.sftp.transfers.lock().map(|t| t.len()).unwrap_or(0);
                    let badge = if transfer_count > 0 {
                        format!("⇅ Queue & History ({})", transfer_count)
                    } else {
                        "⇅ Transfers".to_string()
                    };
                    if ui.button(badge).on_hover_text("View active file transfers and status log").clicked() {
                        app.sftp.show_transfer_history = !app.sftp.show_transfer_history;
                    }
                });
            });
        });

        ui.add_space(4.0);

        // Side-by-side equal columns directly filling available space
        ui.columns(2, |cols| {
            // LEFT COLUMN
            cols[0].vertical(|ui| {
                app.theme.card_frame().show(ui, |ui| {
                    let mut connect_profile = None;

                    // Single Compact Line: Target + Connect + Up + Refresh + Path
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Left:").strong());

                        let is_local = app.sftp.left_pane.target == SftpTarget::Local;
                        let desc = match &app.sftp.left_pane.target {
                            SftpTarget::Local => "Local".to_string(),
                            SftpTarget::RemoteSsh(p) => format!("SSH: {}", p.name),
                        };

                        egui::ComboBox::from_id_source("sftp_left_combo")
                            .width(115.0)
                            .selected_text(desc)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(is_local, "Local Filesystem").clicked() {
                                    app.sftp.left_pane.set_target(SftpTarget::Local);
                                }
                                for p in &app.ssh_store.profiles {
                                    let is_this = app.sftp.left_pane.target == SftpTarget::RemoteSsh(p.clone());
                                    if ui.selectable_label(is_this, format!("SSH: {}", p.name)).clicked() {
                                        app.sftp.left_pane.set_target(SftpTarget::RemoteSsh(p.clone()));
                                    }
                                }
                            });

                        if let SftpTarget::RemoteSsh(ref p) = app.sftp.left_pane.target {
                            let sock = SshStore::sockets_dir().join(format!("{}.sock", p.id));
                            if !sock.exists() {
                                if ui.small_button(egui::RichText::new("⚡ Connect").strong().color(app.theme.accent_color())).clicked() {
                                    connect_profile = Some(p.clone());
                                }
                            }
                        }

                        if ui.small_button("Up").on_hover_text("Go to parent directory").clicked() {
                            app.sftp.left_pane.go_up();
                        }
                        if ui.small_button("⟳").on_hover_text("Reload directory").clicked() {
                            app.sftp.left_pane.refresh();
                        }

                        let path_w = ui.available_width().max(40.0);
                        if ui.add(
                            egui::TextEdit::singleline(&mut app.sftp.left_pane.current_path)
                                .desired_width(path_w)
                        ).lost_focus() {
                            app.sftp.left_pane.refresh();
                        }
                    });

                    if let Some(p) = connect_profile {
                        app.open_ssh_auth_modal(p, "sftp_left".to_string(), ui.ctx().clone());
                    }

                    ui.add_space(2.0);
                    ui.separator();

                    let auth_req = app.sftp.left_pane.render_file_list(ui, &app.theme);
                    if let Some((p, pane_id)) = auth_req {
                        app.open_ssh_auth_modal(p, pane_id, ui.ctx().clone());
                    }
                });
            });

            // RIGHT COLUMN
            cols[1].vertical(|ui| {
                app.theme.card_frame().show(ui, |ui| {
                    let mut connect_profile = None;

                    // Single Compact Line: Target + Connect + Up + Refresh + Path
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Right:").strong());

                        let is_local = app.sftp.right_pane.target == SftpTarget::Local;
                        let desc = match &app.sftp.right_pane.target {
                            SftpTarget::Local => "Local".to_string(),
                            SftpTarget::RemoteSsh(p) => format!("SSH: {}", p.name),
                        };

                        egui::ComboBox::from_id_source("sftp_right_combo")
                            .width(115.0)
                            .selected_text(desc)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(is_local, "Local Filesystem").clicked() {
                                    app.sftp.right_pane.set_target(SftpTarget::Local);
                                }
                                for p in &app.ssh_store.profiles {
                                    let is_this = app.sftp.right_pane.target == SftpTarget::RemoteSsh(p.clone());
                                    if ui.selectable_label(is_this, format!("SSH: {}", p.name)).clicked() {
                                        app.sftp.right_pane.set_target(SftpTarget::RemoteSsh(p.clone()));
                                    }
                                }
                            });

                        if let SftpTarget::RemoteSsh(ref p) = app.sftp.right_pane.target {
                            let sock = SshStore::sockets_dir().join(format!("{}.sock", p.id));
                            if !sock.exists() {
                                if ui.small_button(egui::RichText::new("⚡ Connect").strong().color(app.theme.accent_color())).clicked() {
                                    connect_profile = Some(p.clone());
                                }
                            }
                        }

                        if ui.small_button("Up").on_hover_text("Go to parent directory").clicked() {
                            app.sftp.right_pane.go_up();
                        }
                        if ui.small_button("⟳").on_hover_text("Reload directory").clicked() {
                            app.sftp.right_pane.refresh();
                        }

                        let path_w = ui.available_width().max(40.0);
                        if ui.add(
                            egui::TextEdit::singleline(&mut app.sftp.right_pane.current_path)
                                .desired_width(path_w)
                        ).lost_focus() {
                            app.sftp.right_pane.refresh();
                        }
                    });

                    if let Some(p) = connect_profile {
                        app.open_ssh_auth_modal(p, "sftp_right".to_string(), ui.ctx().clone());
                    }

                    ui.add_space(2.0);
                    ui.separator();

                    let auth_req = app.sftp.right_pane.render_file_list(ui, &app.theme);
                    if let Some((p, pane_id)) = auth_req {
                        app.open_ssh_auth_modal(p, pane_id, ui.ctx().clone());
                    }
                });
            });
        });
    });
}
