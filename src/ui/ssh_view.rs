use crate::sftp::SftpTarget;
use crate::ssh::{SshAuthType, SshProfile, SshStore};
use crate::{ActiveView, AppState, SshSubView};
use eframe::egui;

pub fn render_ssh_view(app: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("SSH Manager").color(app.theme.text_primary_color()));
            ui.add_space(16.0);
            if ui.selectable_label(app.ssh_subview == SshSubView::Profiles, "Connections").clicked() {
                app.ssh_subview = SshSubView::Profiles;
            }
            if ui.selectable_label(app.ssh_subview == SshSubView::KeysManager, "Saved Keypairs").clicked() {
                app.ssh_subview = SshSubView::KeysManager;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+ New SSH Profile").clicked() {
                    app.open_create_profile_modal();
                }
                if ui.button("Key Generator").clicked() {
                    app.show_keygen_modal = true;
                }
            });
        });

        ui.add_space(12.0);

        match app.ssh_subview {
            SshSubView::Profiles => {
                let profiles = app.ssh_store.profiles.clone();
                let mut delete_idx: Option<usize> = None;
                let mut connect_profile: Option<SshProfile> = None;
                let mut edit_profile: Option<SshProfile> = None;
                let mut sftp_profile: Option<SshProfile> = None;

                for (idx, profile) in profiles.iter().enumerate() {
                    app.theme.card_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&profile.name).strong().size(15.0).color(app.theme.text_primary_color()));
                                    let auth_badge = match &profile.auth_type {
                                        SshAuthType::PasswordOrAgent => "[Password/Agent]".to_string(),
                                        SshAuthType::KeyFile(path) => {
                                            let key_name = std::path::Path::new(path)
                                                .file_name()
                                                .map(|n| n.to_string_lossy().to_string())
                                                .unwrap_or_else(|| "Key File".to_string());
                                            format!("[Key: {}]", key_name)
                                        }
                                        SshAuthType::PastedKey { .. } => "[Inline Key]".to_string(),
                                    };
                                    ui.label(egui::RichText::new(&auth_badge).color(app.theme.accent_color()).small());
                                });
                                ui.label(
                                    egui::RichText::new(format!("{}@{}:{}", profile.username, profile.host, profile.port))
                                        .color(app.theme.text_muted_color()),
                                );
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Delete").clicked() {
                                    delete_idx = Some(idx);
                                }
                                if ui.button("Edit").clicked() {
                                    edit_profile = Some(profile.clone());
                                }
                                if ui.button("SFTP").clicked() {
                                    sftp_profile = Some(profile.clone());
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
                    app.ssh_store.profiles.remove(i);
                    app.ssh_store.save();
                }
                if let Some(p) = edit_profile {
                    app.open_edit_profile_modal(&p);
                }
                if let Some(p) = sftp_profile {
                    app.sftp.right_pane.set_target(SftpTarget::RemoteSsh(p));
                    app.active_view = ActiveView::SftpBrowser;
                }
                if let Some(profile) = connect_profile {
                    app.spawn_ssh_terminal(&profile, ctx.clone());
                }
            }
            SshSubView::KeysManager => {
                let saved_keys = SshStore::list_saved_keys();
                if saved_keys.is_empty() {
                    app.theme.card_frame().show(ui, |ui| {
                        ui.label(egui::RichText::new("No SSH keys stored in ~/.config/azterm/keys yet.").color(app.theme.text_muted_color()));
                    });
                } else {
                    let mut key_to_delete: Option<String> = None;
                    let mut key_to_create_profile: Option<std::path::PathBuf> = None;
                    let mut key_name_for_profile: Option<String> = None;

                    for key in saved_keys {
                        app.theme.card_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(&key.file_name).strong().color(app.theme.accent_color()));
                                    ui.label(egui::RichText::new(format!("Path: {}", key.priv_path.display())).small().color(app.theme.text_muted_color()));
                                });

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("Delete Key").clicked() {
                                        key_to_delete = Some(key.file_name.clone());
                                    }
                                    if let Some(ref pub_k) = key.pub_key_content {
                                        if ui.button("Copy Public Key").clicked() {
                                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                                let _ = cb.set_text(pub_k.clone());
                                                app.set_toast("Public key copied to clipboard");
                                            }
                                        }
                                    }
                                    if ui.button("+ New Profile with Key").on_hover_text("Create a new SSH connection using this key").clicked() {
                                        key_to_create_profile = Some(key.priv_path.clone());
                                        key_name_for_profile = Some(key.file_name.clone());
                                    }
                                });
                            });
                        });
                        ui.add_space(8.0);
                    }

                    if let (Some(path), Some(name)) = (key_to_create_profile, key_name_for_profile) {
                        app.open_create_profile_modal();
                        app.new_ssh_auth_choice = 1;
                        app.new_ssh_key_path = path.to_string_lossy().to_string();
                        let clean_title = name.trim_start_matches("id_ed25519_")
                            .trim_start_matches("id_rsa_")
                            .trim_start_matches("id_ecdsa384_")
                            .trim_start_matches("id_ecdsa256_");
                        if !clean_title.is_empty() {
                            app.new_ssh_name = format!("Server ({})", clean_title);
                        }
                    }

                    if let Some(name) = key_to_delete {
                        SshStore::delete_key_files(&name);
                        app.set_toast("Key files removed");
                    }
                }
            }
        }
    });
}
