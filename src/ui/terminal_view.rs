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

    let available_rect = ui.available_rect_before_wrap();
    let (term_area_rect, sftp_pane_rect) = if app.settings.show_sftp_split_view {
        let w = available_rect.width() * 0.65;
        (
            egui::Rect::from_min_size(available_rect.min, egui::vec2(w, available_rect.height())),
            Some(egui::Rect::from_min_size(egui::pos2(available_rect.min.x + w, available_rect.min.y), egui::vec2(available_rect.width() - w, available_rect.height()))),
        )
    } else {
        (available_rect, None)
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
                    6.0,
                    egui::Color32::from_rgba_unmultiplied(app.theme.accent[0], app.theme.accent[1], app.theme.accent[2], 75),
                );
                ui.painter().rect_stroke(
                    snap_rect,
                    6.0,
                    egui::Stroke::new(2.0_f32, app.theme.accent_color()),
                );
                ui.painter().text(
                    snap_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop to Tile Here",
                    egui::FontId::proportional(14.0),
                    egui::Color32::WHITE,
                );
            } else if ptr.y < term_area_rect.min.y && app.dragging_pane_id.is_some() {
                let badge_rect = egui::Rect::from_center_size(ptr, egui::vec2(160.0, 26.0));
                ui.painter().rect_filled(badge_rect, 4.0, app.theme.bg_card_color());
                ui.painter().rect_stroke(badge_rect, 4.0, egui::Stroke::new(1.0_f32, app.theme.accent_color()));
                ui.painter().text(badge_rect.center(), egui::Align2::CENTER_CENTER, "Drop to Pop Out as Tab", egui::FontId::proportional(12.0), app.theme.accent_color());
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
                                            app.set_toast("Cannot dock: workspace limit of 16 panes reached");
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

    if let Some(sftp_rect) = sftp_pane_rect {
        app.theme.card_frame().show(ui, |ui| {
            ui.allocate_ui_at_rect(sftp_rect, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("SFTP Sync Pane");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Upload Selected").clicked() {
                            app.sftp.upload_selected();
                        }
                    });
                });
                ui.separator();
                app.sftp.right_pane.render(ui, &app.theme);
            });
        });
    }

    app.toast_message = toast;
}
