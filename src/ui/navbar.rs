// src/ui/navbar.rs
use crate::terminal::SessionType;
use crate::theme::{nav_action_button, nav_tab_button, session_tab_chip};
use crate::tiling::SplitDirection;
use crate::{ActiveView, AppState};
use eframe::egui;

pub fn render_top_nav(app: &mut AppState, ctx: &egui::Context) {
    egui::TopBottomPanel::top("top_nav")
        .frame(egui::Frame::none().fill(app.theme.bg_panel_color()).inner_margin(egui::Margin::symmetric(14.0, 7.0)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let brand_resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new("AZTerm")
                            .color(app.theme.accent_color())
                            .strong()
                            .size(16.0),
                    ).sense(egui::Sense::click_and_drag())
                );
                if !app.settings.use_system_titlebar {
                    if brand_resp.drag_started_by(egui::PointerButton::Primary)
                        || (brand_resp.hovered() && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)))
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if brand_resp.double_clicked() {
                        let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
                    }
                }

                ui.add_space(8.0);

                if nav_tab_button(ui, "Terminal", app.active_view == ActiveView::Terminal, &app.theme) {
                    app.active_view = ActiveView::Terminal;
                }
                if nav_tab_button(ui, "SSH Profiles", app.active_view == ActiveView::SshBookmarks, &app.theme) {
                    app.active_view = ActiveView::SshBookmarks;
                }
                if nav_tab_button(ui, "SFTP Explorer", app.active_view == ActiveView::SftpBrowser, &app.theme) {
                    app.active_view = ActiveView::SftpBrowser;
                }
                if nav_tab_button(ui, "Settings", app.active_view == ActiveView::Settings, &app.theme) {
                    app.active_view = ActiveView::Settings;
                }

                ui.separator();

                if nav_action_button(ui, "+ New Shell", &app.theme) {
                    app.spawn_local_terminal(ctx.clone(), None);
                }

                if app.active_view == ActiveView::Terminal {
                    if ui.button("Split Right").on_hover_text("Split active pane side-by-side (Ctrl+Shift+D)").clicked() {
                        app.split_active_pane(SplitDirection::Horizontal, ctx.clone());
                    }
                    if ui.button("Split Down").on_hover_text("Split active pane top-and-bottom (Ctrl+Shift+E)").clicked() {
                        app.split_active_pane(SplitDirection::Vertical, ctx.clone());
                    }
                    if let Some(ws) = app.workspaces.get(app.active_workspace_idx) {
                        if !ws.is_single_pane() {
                            let max_label = if ws.maximized_session.is_some() { "Restore Splits" } else { "Maximize Pane" };
                            if ui.button(max_label).on_hover_text("Toggle maximize active pane (Ctrl+Shift+M)").clicked() {
                                if let Some(ws_mut) = app.workspaces.get_mut(app.active_workspace_idx) {
                                    ws_mut.maximized_session = if ws_mut.maximized_session.is_some() { None } else { Some(app.active_session_id) };
                                }
                            }
                        }
                    }

                    let current_is_multi = app.workspaces.get(app.active_workspace_idx).map_or(false, |w| !w.is_single_pane());
                    if current_is_multi {
                        if ui.button("Untile Active Tab").on_hover_text("Detach tiled panes in this tab into separate tabs").clicked() {
                            app.untile_all_to_tabs();
                        }
                    }

                    let all_sessions_count = app.sessions.len();
                    let optimal_tabs_count = (all_sessions_count + 15) / 16;
                    let can_tile_more = all_sessions_count >= 2
                        && (app.workspaces.len() > optimal_tabs_count || app.workspaces.iter().any(|w| w.is_single_pane()));

                    if can_tile_more {
                        if ui.button("Tile All Tabs").on_hover_text("Tile all open tabs into balanced grids (batches of 16 per tab)").clicked() {
                            app.tile_all_tabs();
                        }
                    }
                }

                if !app.settings.use_system_titlebar {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let close_resp = ui.add(
                            egui::Button::new(egui::RichText::new("X").size(12.0).color(app.theme.text_primary_color()))
                                .min_size(egui::vec2(28.0, 22.0))
                                .fill(egui::Color32::TRANSPARENT)
                        );
                        if close_resp.hovered() {
                            ui.painter().rect_filled(close_resp.rect, 3.0, app.theme.danger_color());
                        }
                        if close_resp.clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }

                        let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        let max_icon = if is_max { "Restore" } else { "Max" };
                        if ui.add(
                            egui::Button::new(egui::RichText::new(max_icon).size(11.0).color(app.theme.text_primary_color()))
                                .min_size(egui::vec2(44.0, 22.0))
                                .fill(egui::Color32::TRANSPARENT)
                        ).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
                        }

                        if ui.add(
                            egui::Button::new(egui::RichText::new("-").size(14.0).color(app.theme.text_primary_color()))
                                .min_size(egui::vec2(28.0, 22.0))
                                .fill(egui::Color32::TRANSPARENT)
                        ).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }

                        let drag_w = ui.available_width().max(0.0);
                        if drag_w > 10.0 {
                            let (_drag_rect, drag_resp) = ui.allocate_exact_size(
                                egui::vec2(drag_w, 24.0),
                                egui::Sense::click_and_drag(),
                            );
                            if drag_resp.drag_started_by(egui::PointerButton::Primary)
                                || (drag_resp.hovered() && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)))
                            {
                                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                            }
                            if drag_resp.double_clicked() {
                                let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
                            }
                        }
                    });
                }
            });
        });
}

pub fn render_tabs_bar(app: &mut AppState, ctx: &egui::Context) {
    if app.workspaces.len() <= 1 {
        return;
    }

    egui::TopBottomPanel::top("session_tabs_bar")
        .frame(
            egui::Frame::none()
                .fill(app.theme.bg_panel_color().linear_multiply(0.85))
                .stroke(egui::Stroke::new(1.0_f32, app.theme.border_color()))
                .inner_margin(egui::Margin::symmetric(14.0, 4.0)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let mut tab_to_close: Option<usize> = None;
                let avail_w = (ui.available_width() - 20.0).max(100.0);
                let num_tabs = app.workspaces.len() as f32;
                let computed_tab_width = ((avail_w / num_tabs) - 6.0).clamp(90.0, 200.0);
                let max_chars = ((computed_tab_width - 32.0) / 7.2).max(4.0) as usize;

                egui::ScrollArea::horizontal()
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (i, ws) in app.workspaces.iter().enumerate() {
                                let is_active = app.active_view == ActiveView::Terminal && app.active_workspace_idx == i;
                                let tab_title = if ws.is_single_pane() {
                                    ws.title.clone()
                                } else {
                                    format!("{} ({} Panes)", ws.title, ws.leaves().len())
                                };

                                let label_text = if tab_title.len() > max_chars {
                                    format!("{}...", &tab_title[..max_chars.saturating_sub(3)])
                                } else {
                                    tab_title
                                };

                                let (tab_clicked, close_clicked) = session_tab_chip(
                                    ui,
                                    ws.id,
                                    &label_text,
                                    is_active,
                                    computed_tab_width,
                                    true,
                                    &app.theme,
                                );

                                let chip_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(computed_tab_width, 26.0));
                                let chip_resp = ui.interact(chip_rect, ui.id().with("chip_drag").with(ws.id), egui::Sense::drag());
                                if chip_resp.drag_started_by(egui::PointerButton::Primary) {
                                    app.dragging_tab_idx = Some(i);
                                }

                                if tab_clicked {
                                    app.active_workspace_idx = i;
                                    app.active_session_id = ws.root.first_leaf();
                                    app.active_view = ActiveView::Terminal;
                                }

                                if close_clicked {
                                    tab_to_close = Some(i);
                                }

                                ui.add_space(4.0);
                            }
                        });
                    });

                if let Some(i) = tab_to_close {
                    let leaves = app.workspaces[i].leaves();
                    for leaf_id in leaves {
                        app.sessions.retain(|s| s.id != leaf_id);
                    }
                    app.workspaces.remove(i);
                    if app.workspaces.is_empty() {
                        app.spawn_local_terminal(ctx.clone(), None);
                    } else {
                        if app.active_workspace_idx >= app.workspaces.len() {
                            app.active_workspace_idx = app.workspaces.len() - 1;
                        }
                        if let Some(ws) = app.workspaces.get(app.active_workspace_idx) {
                            app.active_session_id = ws.root.first_leaf();
                        }
                    }
                    app.persist_sessions();
                }
            });
        });
}

pub fn render_status_bar(app: &mut AppState, ctx: &egui::Context) {
    app.sftp.poll_transfers();

    egui::TopBottomPanel::bottom("bottom_status_bar")
        .frame(egui::Frame::none().fill(app.theme.bg_panel_color()).inner_margin(egui::Margin::symmetric(14.0, 4.0)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(session) = app.sessions.iter().find(|s| s.id == app.active_session_id) {
                    let info = match &session.session_type {
                        SessionType::Local { working_dir } => format!("Active Shell [{}]: {}", session.id, working_dir),
                        SessionType::Ssh { profile_id } => format!("Active Target [{}]: {}", session.id, profile_id),
                    };
                    ui.label(egui::RichText::new(info).small().color(app.theme.text_muted_color()));
                }

                ui.separator();

                let sftp_btn_text = if app.settings.show_sftp_split_view {
                    "SFTP Drawer: OPEN"
                } else {
                    "SFTP Drawer: CLOSED"
                };
                if nav_tab_button(ui, sftp_btn_text, app.settings.show_sftp_split_view, &app.theme) {
                    app.settings.show_sftp_split_view = !app.settings.show_sftp_split_view;
                    app.settings.save();
                    app.set_toast(if app.settings.show_sftp_split_view {
                        "SFTP split panel opened"
                    } else {
                        "SFTP split panel closed"
                    });
                }

                if app.settings.check_updates {
                    if let Some(ref update_tag) = app.available_update {
                        ui.separator();
                        if nav_action_button(ui, &format!("Update: {}", update_tag), &app.theme) {
                            app.show_update_modal = true;
                        }
                    }
                }

                // Live Transfer Status Badge
                if let Some((ref text, is_error, _)) = app.sftp.transfer_status {
                    ui.separator();
                    let col = if is_error { app.theme.danger_color() } else { app.theme.accent_color() };
                    let resp = ui.add(
                        egui::Button::new(egui::RichText::new(text).small().color(col))
                            .fill(egui::Color32::TRANSPARENT)
                    );
                    if resp.clicked() {
                        app.sftp.show_transfer_history = true;
                    }
                }

                if let Some((msg, time)) = &app.toast_message {
                    if time.elapsed().as_secs_f32() < 3.0 {
                        ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                            egui::Frame::none()
                                .fill(app.theme.bg_card_color())
                                .stroke(egui::Stroke::new(1.0_f32, app.theme.accent_color()))
                                .rounding(4.0)
                                .inner_margin(egui::Margin::symmetric(12.0, 2.0))
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new(msg).color(app.theme.accent_color()).strong().small());
                                });
                        });
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(session) = app.sessions.iter().find(|s| s.id == app.active_session_id) {
                        let scroll_info = if session.scroll_offset > 0 {
                            format!("-{} lines | {}x{}", session.scroll_offset, session.cols, session.rows)
                        } else {
                            format!("{}x{}", session.cols, session.rows)
                        };
                        let col = if session.scroll_offset > 0 { app.theme.accent_color() } else { app.theme.text_muted_color() };
                        ui.label(egui::RichText::new(scroll_info).small().color(col));
                    }
                });
            });
        });
}
