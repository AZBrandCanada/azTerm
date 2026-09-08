use crate::sftp::SftpTarget;
use crate::AppState;
use eframe::egui;

pub fn render_sftp_browser_view(app: &mut AppState, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(10.0);

        app.theme.card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Dual-Session SFTP File Transfer").strong().color(app.theme.accent_color()));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Download (Right -> Left)").clicked() {
                        app.sftp.download_selected();
                    }
                    if ui.button("Upload (Left -> Right)").clicked() {
                        app.sftp.upload_selected();
                    }
                });
            });
        });

        ui.add_space(10.0);

        ui.columns(2, |cols| {
            app.theme.card_frame().show(&mut cols[0], |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Left Pane:").strong());
                    let is_local = app.sftp.left_pane.target == SftpTarget::Local;
                    egui::ComboBox::from_id_source("left_pane_target_combo")
                        .selected_text(if is_local { "Local Filesystem" } else { "Remote SSH Target" })
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
                });
                ui.separator();
                app.sftp.left_pane.render(ui, &app.theme);
            });

            app.theme.card_frame().show(&mut cols[1], |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Right Pane:").strong());
                    let right_desc = match &app.sftp.right_pane.target {
                        SftpTarget::Local => "Local Filesystem".to_string(),
                        SftpTarget::RemoteSsh(p) => format!("SSH: {}", p.name),
                    };
                    egui::ComboBox::from_id_source("right_pane_target_combo")
                        .selected_text(right_desc)
                        .show_ui(ui, |ui| {
                            let is_local = app.sftp.right_pane.target == SftpTarget::Local;
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
                });
                ui.separator();
                app.sftp.right_pane.render(ui, &app.theme);
            });
        });
    });
}
