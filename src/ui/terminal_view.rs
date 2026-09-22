// src/ui/terminal_view.rs
use crate::sftp::{PaneBrowser, SftpTarget};
use crate::ssh::SshStore;
use crate::tiling::{detect_dock_zone, dock_zone_preview_rect, render_tile_tree, DockZone, PaneAction, SplitDirection, WorkspaceTab};
use crate::AppState;
use eframe::egui;

pub fn render_terminal_workspace(app: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui) {
    let mut toast = app.toast_message.clone();

    if app.workspaces.is_empty() {
        ui.centered_and_justified(|ui| {
            if ui.button("Open Shell Session").clicked() {
                app.spawn_local_terminal(ctx.clone(), None);
            }
        });
        return;
    }

    let total_area = ui.available_rect_before_wrap();

    let (term_area_rect, sftp_pane_rect) = if app.settings.show_sftp_split_view {
        let total_w = total_area.width();
        let sftp_w = (total_w * 0.38).clamp(340.0, 520.0);
        let split_w = (total_w - sftp_w - 6.0).max(120.0);
        (
            egui::Rect::from_min_size(total_area.min, egui::vec2(split_w, total_area.height())),
            Some(egui::Rect::from_min_size(
                egui::pos2(total_area.min.x + split_w + 6.0, total_area.min.y),
                egui::vec2(sftp_w, total_area.height()),
            )),
        )
    } else {
        (total_area, None)
    };

    let mut actions = Vec::new();
    let mut pane_rects = Vec::new();

    if let Some(ws) = app.workspaces.get_mut(app.active_workspace_idx) {
        let is_multi_pane = !ws.is_single_pane();
        let max_session = ws.maximized_session;
        render_tile_tree(
            ui,
            &mut ws.root,
            term_area_rect,
            &app.theme,
            &app.settings,
            &mut app.sessions,
            &mut app.active_session_id,
            max_session,
            &mut toast,
            &mut actions,
            &mut pane_rects,
            is_multi_pane,
        );
    }

    app.last_pane_rects = pane_rects.clone();

    let is_primary_down = ui.input(|i| i.pointer.primary_down());
    let pointer_pos = ui.input(|i| i.pointer.hover_pos());

    let is_dragging_own_tab = app.dragging_tab_idx == Some(app.active_workspace_idx);

    let active_drag_session: Option<usize> = if app.dragging_pane_id.is_some() {
        app.dragging_pane_id
    } else if let Some(idx) = app.dragging_tab_idx {
        if !is_dragging_own_tab {
            app.workspaces.get(idx).map(|w| w.root.first_leaf())
        } else {
            None
        }
    } else {
        None
    };

    if let (Some(dragged_sess_id), Some(ptr)) = (active_drag_session, pointer_pos) {
        if is_primary_down {
            let mut hovered_zone: Option<(usize, DockZone, egui::Rect)> = None;
            for &(pane_id, p_rect) in &pane_rects {
                if pane_id != dragged_sess_id {
                    if let Some(zone) = detect_dock_zone(p_rect, ptr) {
                        let snap_rect = dock_zone_preview_rect(p_rect, zone);
                        hovered_zone = Some((pane_id, zone, snap_rect));
                        break;
                    }
                }
            }

            if let Some((_, _, snap_rect)) = hovered_zone {
                ui.painter().rect_filled(
                    snap_rect,
                    4.0,
                    egui::Color32::from_rgba_unmultiplied(app.theme.accent[0], app.theme.accent[1], app.theme.accent[2], 75),
                );
                ui.painter().rect_stroke(
                    snap_rect,
                    4.0,
                    egui::Stroke::new(2.0_f32, app.theme.accent_color()),
                );
                ui.painter().text(
                    snap_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Tile Here",
                    egui::FontId::proportional(14.0),
                    egui::Color32::WHITE,
                );
            } else if ptr.y < term_area_rect.min.y && app.dragging_pane_id.is_some() {
                let badge_rect = egui::Rect::from_center_size(ptr, egui::vec2(160.0, 26.0));
                ui.painter().rect_filled(badge_rect, 4.0, app.theme.bg_card_color());
                ui.painter().rect_stroke(badge_rect, 4.0, egui::Stroke::new(1.0_f32, app.theme.accent_color()));
                ui.painter().text(badge_rect.center(), egui::Align2::CENTER_CENTER, "Pop Out as Tab", egui::FontId::proportional(12.0), app.theme.accent_color());
            }
        } else {
            if let Some(ptr) = pointer_pos {
                let mut docked = false;
                for &(pane_id, p_rect) in &pane_rects {
                    if pane_id != dragged_sess_id {
                        if let Some(zone) = detect_dock_zone(p_rect, ptr) {
                            let dir = match zone {
                                DockZone::Left | DockZone::Right => SplitDirection::Horizontal,
                                DockZone::Top | DockZone::Bottom => SplitDirection::Vertical,
                            };
                            let insert_after = matches!(zone, DockZone::Right | DockZone::Bottom);

                            if let Some(drag_ws_idx) = app.dragging_tab_idx {
                                if drag_ws_idx < app.workspaces.len() && drag_ws_idx != app.active_workspace_idx {
                                    if let Some(target_ws) = app.workspaces.get(app.active_workspace_idx) {
                                        let incoming_leaves = app.workspaces[drag_ws_idx].leaves().len();
                                        if target_ws.leaves().len() + incoming_leaves > 16 {
                                            app.set_toast("Cannot dock: limit of 16 panes reached");
                                            docked = true;
                                            break;
                                        }
                                    }

                                    let incoming_tree = app.workspaces[drag_ws_idx].root.clone();
                                    app.workspaces.remove(drag_ws_idx);
                                    if drag_ws_idx < app.active_workspace_idx {
                                        app.active_workspace_idx -= 1;
                                    }
                                    let split_id = app.next_split_id;
                                    app.next_split_id += 1;

                                    if let Some(ws) = app.workspaces.get_mut(app.active_workspace_idx) {
                                        ws.root.split_leaf_with_node(pane_id, incoming_tree, dir, insert_after, split_id);
                                        app.active_session_id = dragged_sess_id;
                                        app.set_toast("Tiled tab into workspace");
                                        app.persist_sessions();
                                    }
                                }
                            } else if app.dragging_pane_id.is_some() {
                                if let Some(ws) = app.workspaces.get_mut(app.active_workspace_idx) {
                                    if !ws.is_single_pane() {
                                        ws.root.remove_leaf(dragged_sess_id);
                                        let split_id = app.next_split_id;
                                        app.next_split_id += 1;
                                        ws.root.split_leaf(pane_id, dragged_sess_id, dir, insert_after, split_id);
                                        app.active_session_id = dragged_sess_id;
                                        app.set_toast("Moved tile");
                                        app.persist_sessions();
                                    }
                                }
                            }
                            docked = true;
                            break;
                        }
                    }
                }

                if !docked && ptr.y < term_area_rect.min.y && app.dragging_pane_id.is_some() {
                    if let Some(ws) = app.workspaces.get_mut(app.active_workspace_idx) {
                        if !ws.is_single_pane() {
                            ws.root.remove_leaf(dragged_sess_id);
                            let session_title = app.sessions.iter().find(|s| s.id == dragged_sess_id).map(|s| s.title.clone()).unwrap_or_else(|| format!("Local #{}", dragged_sess_id));
                            let new_ws = WorkspaceTab::new(dragged_sess_id, dragged_sess_id, session_title);
                            app.workspaces.push(new_ws);
                            app.active_workspace_idx = app.workspaces.len() - 1;
                            app.active_session_id = dragged_sess_id;
                            app.set_toast("Popped out to its own tab");
                            app.persist_sessions();
                        }
                    }
                }
            }
            app.dragging_tab_idx = None;
            app.dragging_pane_id = None;
        }
    }

    for action in actions {
        match action {
            PaneAction::Split(id, dir) => {
                app.active_session_id = id;
                app.split_active_pane(dir, ctx.clone());
            }
            PaneAction::ToggleMaximize(id) => {
                if let Some(ws) = app.workspaces.get_mut(app.active_workspace_idx) {
                    if !ws.is_single_pane() {
                        ws.maximized_session = if ws.maximized_session == Some(id) { None } else { Some(id) };
                    }
                }
            }
            PaneAction::PopToTab(id) => {
                if let Some(ws) = app.workspaces.get_mut(app.active_workspace_idx) {
                    if ws.is_single_pane() {
                        app.set_toast("Pane is already in its own tab");
                    } else {
                        ws.root.remove_leaf(id);
                        if ws.maximized_session == Some(id) {
                            ws.maximized_session = None;
                        }
                        let session_title = app.sessions.iter().find(|s| s.id == id).map(|s| s.title.clone()).unwrap_or_else(|| format!("Local #{}", id));
                        let new_ws = WorkspaceTab::new(id, id, session_title);
                        app.workspaces.push(new_ws);
                        app.active_workspace_idx = app.workspaces.len() - 1;
                        app.active_session_id = id;
                        app.set_toast("Popped out to its own tab");
                        app.persist_sessions();
                    }
                }
            }
            PaneAction::Close(id) => {
                app.close_session(id, ctx.clone());
            }
            PaneAction::Focus(id) => {
                app.active_session_id = id;
            }
            PaneAction::StartDrag(id) => {
                app.dragging_pane_id = Some(id);
                app.active_session_id = id;
            }
        }
    }

    // Two-Tier Vertical SFTP Sync Explorer in Split Drawer
    if let Some(sftp_rect) = sftp_pane_rect {
        ui.allocate_ui_at_rect(sftp_rect, |ui| {
            app.theme.card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("SFTP Sync Explorer").strong().color(app.theme.accent_color()));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Close [X]").clicked() {
                            app.settings.show_sftp_split_view = false;
                            app.settings.save();
                        }
                        let transfer_count = app.sftp.transfers.lock().map(|t| t.len()).unwrap_or(0);
                        let badge_text = if transfer_count > 0 {
                            format!("Transfers ({})", transfer_count)
                        } else {
                            "Transfers".to_string()
                        };
                        if ui.small_button(badge_text).clicked() {
                            app.sftp.show_transfer_history = !app.sftp.show_transfer_history;
                        }
                    });
                });

                ui.add_space(2.0);
                ui.separator();

                let avail_h = ui.available_height().max(160.0);
                let bridge_h = 36.0_f32;
                let sub_pane_h = ((avail_h - bridge_h - 10.0) * 0.5).max(60.0);

                let mut auth_to_open = None;

                // TOP PANE (Pane 1: Local / Source)
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), sub_pane_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let mut connect_profile = None;

                        ui.horizontal(|ui| {
                            let is_local = app.sftp.left_pane.target == SftpTarget::Local;
                            let desc = match &app.sftp.left_pane.target {
                                SftpTarget::Local => "Local".to_string(),
                                SftpTarget::RemoteSsh(p) => format!("SSH: {}", p.name),
                            };

                            egui::ComboBox::from_id_source("sync_drawer_top_combo")
                                .width(110.0)
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
                                    if ui.small_button(egui::RichText::new("Connect").strong().color(app.theme.accent_color())).clicked() {
                                        connect_profile = Some(p.clone());
                                    }
                                }
                            }

                            if ui.small_button("Up").on_hover_text("Parent folder").clicked() {
                                app.sftp.left_pane.go_up();
                            }
                            if ui.small_button("Home").on_hover_text("Home folder").clicked() {
                                app.sftp.left_pane.go_home();
                            }
                            if ui.small_button("Reload").on_hover_text("Reload").clicked() {
                                app.sftp.left_pane.refresh();
                            }
                            if ui.small_button("+ Folder").on_hover_text("New directory").clicked() {
                                app.sftp.left_pane.show_create_dir_modal = true;
                                app.sftp.left_pane.new_dir_name = "new_folder".to_string();
                            }

                            if !app.sftp.left_pane.selected_items.is_empty() {
                                let del_label = if app.sftp.left_pane.selected_items.len() > 1 {
                                    format!("Del ({})", app.sftp.left_pane.selected_items.len())
                                } else {
                                    "Del".to_string()
                                };
                                if ui.small_button(egui::RichText::new(del_label).color(app.theme.danger_color())).clicked() {
                                    app.sftp.left_pane.items_to_delete = app.sftp.left_pane.selected_items.clone();
                                    app.sftp.left_pane.show_delete_confirm_modal = true;
                                }
                            }

                            let path_w = ui.available_width().max(40.0);
                            let p_edit = ui.add(
                                egui::TextEdit::singleline(&mut app.sftp.left_pane.current_path)
                                    .desired_width(path_w)
                            );
                            if p_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                app.sftp.left_pane.set_path(app.sftp.left_pane.current_path.clone());
                            }
                        });

                        if let Some(p) = connect_profile {
                            auth_to_open = Some((p, "sftp_left".to_string()));
                        }

                        let auth_req = app.sftp.left_pane.render_file_list(ui, &app.theme);
                        if let Some(req) = auth_req {
                            auth_to_open = Some(req);
                        }
                    },
                );

                // MIDDLE ACTION BAR (Context-aware Top <-> Bottom transfer)
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), bridge_h),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let top_to_bot_title = match (&app.sftp.left_pane.target, &app.sftp.right_pane.target) {
                            (SftpTarget::Local, SftpTarget::RemoteSsh(_)) => "[v] Upload",
                            (SftpTarget::RemoteSsh(_), SftpTarget::Local) => "[v] Download",
                            (SftpTarget::RemoteSsh(_), SftpTarget::RemoteSsh(_)) => "[v] VPS -> VPS",
                            _ => "[v] Copy",
                        };

                        let l_count = app.sftp.left_pane.selected_items.len();
                        let l_size: u64 = app.sftp.left_pane.entries
                            .iter()
                            .filter(|e| app.sftp.left_pane.selected_items.contains(&e.name))
                            .map(|e| e.size)
                            .sum();

                        let l_summary = if l_count == 0 {
                            "0 sel".to_string()
                        } else if l_count == 1 {
                            let name = &app.sftp.left_pane.selected_items[0];
                            let short = if name.len() > 10 { format!("{}...", &name[..8]) } else { name.clone() };
                            format!("{} ({})", short, PaneBrowser::format_size(l_size))
                        } else {
                            format!("Multiple ({}) ({})", l_count, PaneBrowser::format_size(l_size))
                        };

                        let up_btn = egui::Button::new(
                            egui::RichText::new(format!("{} {}", top_to_bot_title, l_summary))
                                .strong()
                                .small()
                                .color(if l_count > 0 { app.theme.text_primary_color() } else { app.theme.text_muted_color() })
                        )
                        .min_size(egui::vec2(130.0, 24.0))
                        .fill(if l_count > 0 { app.theme.accent_color().linear_multiply(0.8) } else { egui::Color32::TRANSPARENT });

                        if ui.add_enabled(l_count > 0, up_btn).on_hover_text("Transfer selected files from Top to Bottom").clicked() {
                            app.sftp.upload_selected();
                        }

                        ui.add_space(8.0);

                        let bot_to_top_title = match (&app.sftp.right_pane.target, &app.sftp.left_pane.target) {
                            (SftpTarget::RemoteSsh(_), SftpTarget::Local) => "[^] Download",
                            (SftpTarget::Local, SftpTarget::RemoteSsh(_)) => "[^] Upload",
                            (SftpTarget::RemoteSsh(_), SftpTarget::RemoteSsh(_)) => "[^] VPS -> VPS",
                            _ => "[^] Copy",
                        };

                        let r_count = app.sftp.right_pane.selected_items.len();
                        let r_size: u64 = app.sftp.right_pane.entries
                            .iter()
                            .filter(|e| app.sftp.right_pane.selected_items.contains(&e.name))
                            .map(|e| e.size)
                            .sum();

                        let r_summary = if r_count == 0 {
                            "0 sel".to_string()
                        } else if r_count == 1 {
                            let name = &app.sftp.right_pane.selected_items[0];
                            let short = if name.len() > 10 { format!("{}...", &name[..8]) } else { name.clone() };
                            format!("{} ({})", short, PaneBrowser::format_size(r_size))
                        } else {
                            format!("Multiple ({}) ({})", r_count, PaneBrowser::format_size(r_size))
                        };

                        let dl_btn = egui::Button::new(
                            egui::RichText::new(format!("{} {}", bot_to_top_title, r_summary))
                                .strong()
                                .small()
                                .color(if r_count > 0 { app.theme.text_primary_color() } else { app.theme.text_muted_color() })
                        )
                        .min_size(egui::vec2(130.0, 24.0))
                        .fill(if r_count > 0 { app.theme.accent_color().linear_multiply(0.8) } else { egui::Color32::TRANSPARENT });

                        if ui.add_enabled(r_count > 0, dl_btn).on_hover_text("Transfer selected files from Bottom to Top").clicked() {
                            app.sftp.download_selected();
                        }
                    },
                );

                ui.separator();

                // BOTTOM PANE (Pane 2: Remote / Target)
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), sub_pane_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let mut connect_profile = None;

                        ui.horizontal(|ui| {
                            let is_local = app.sftp.right_pane.target == SftpTarget::Local;
                            let desc = match &app.sftp.right_pane.target {
                                SftpTarget::Local => "Local".to_string(),
                                SftpTarget::RemoteSsh(p) => format!("SSH: {}", p.name),
                            };

                            egui::ComboBox::from_id_source("sync_drawer_bottom_combo")
                                .width(110.0)
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
                                    if ui.small_button(egui::RichText::new("Connect").strong().color(app.theme.accent_color())).clicked() {
                                        connect_profile = Some(p.clone());
                                    }
                                }
                            }

                            if ui.small_button("Up").on_hover_text("Parent folder").clicked() {
                                app.sftp.right_pane.go_up();
                            }
                            if ui.small_button("Home").on_hover_text("Home folder").clicked() {
                                app.sftp.right_pane.go_home();
                            }
                            if ui.small_button("Reload").on_hover_text("Reload").clicked() {
                                app.sftp.right_pane.refresh();
                            }
                            if ui.small_button("+ Folder").on_hover_text("New directory").clicked() {
                                app.sftp.right_pane.show_create_dir_modal = true;
                                app.sftp.right_pane.new_dir_name = "new_folder".to_string();
                            }

                            if !app.sftp.right_pane.selected_items.is_empty() {
                                let del_label = if app.sftp.right_pane.selected_items.len() > 1 {
                                    format!("Del ({})", app.sftp.right_pane.selected_items.len())
                                } else {
                                    "Del".to_string()
                                };
                                if ui.small_button(egui::RichText::new(del_label).color(app.theme.danger_color())).clicked() {
                                    app.sftp.right_pane.items_to_delete = app.sftp.right_pane.selected_items.clone();
                                    app.sftp.right_pane.show_delete_confirm_modal = true;
                                }
                            }

                            let path_w = ui.available_width().max(40.0);
                            let p_edit = ui.add(
                                egui::TextEdit::singleline(&mut app.sftp.right_pane.current_path)
                                    .desired_width(path_w)
                            );
                            if p_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                app.sftp.right_pane.set_path(app.sftp.right_pane.current_path.clone());
                            }
                        });

                        if let Some(p) = connect_profile {
                            auth_to_open = Some((p, "sftp_right".to_string()));
                        }

                        let auth_req = app.sftp.right_pane.render_file_list(ui, &app.theme);
                        if let Some(req) = auth_req {
                            auth_to_open = Some(req);
                        }
                    },
                );

                if let Some((p, pane_id)) = auth_to_open {
                    app.open_ssh_auth_modal(p, pane_id, ctx.clone());
                }
            });
        });
    }

    app.toast_message = toast;
}
