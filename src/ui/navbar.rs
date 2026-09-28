// src/ui/navbar.rs
use crate::ssh::SshProfile;
use crate::terminal::SessionType;
use crate::theme::{nav_action_button, nav_tab_button, session_tab_chip};
use crate::tiling::SplitDirection;
use crate::{ActiveView, AppState};
use eframe::egui;

pub fn render_top_nav(app: &mut AppState, ctx: &egui::Context) {
    egui::TopBottomPanel::top("top_nav")
        .frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(14.0, 7.0)))
        .show(ctx, |ui| {
            // Vertical gradient behind the nav content, from a lighter
            // shade at the top to a darker shade at the bottom. This is
            // what gives the bar physical "chrome" depth instead of
            // looking like a flat filled rectangle.
            let bar_rect = ui.painter().clip_rect();
            let base = app.theme.bg_panel_color();
            crate::modern::gradient_rect(
                ui.painter(),
                bar_rect,
                crate::modern::lighten(base, 14),
                crate::modern::darken(base, 10),
            );
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
                    if app.active_view != ActiveView::Terminal {
                        app.active_view = ActiveView::Terminal;
                        app.trigger_transition();
                    }
                }
                if nav_tab_button(ui, "SSH Profiles", app.active_view == ActiveView::SshBookmarks, &app.theme) {
                    if app.active_view != ActiveView::SshBookmarks {
                        app.active_view = ActiveView::SshBookmarks;
                        app.trigger_transition();
                    }
                }
                if nav_tab_button(ui, "SFTP Explorer", app.active_view == ActiveView::SftpBrowser, &app.theme) {
                    if app.active_view != ActiveView::SftpBrowser {
                        app.active_view = ActiveView::SftpBrowser;
                        app.trigger_transition();
                    }
                }
                if nav_tab_button(ui, "Settings", app.active_view == ActiveView::Settings, &app.theme) {
                    if app.active_view != ActiveView::Settings {
                        app.active_view = ActiveView::Settings;
                        app.trigger_transition();
                    }
                }

                ui.separator();

                if nav_action_button(ui, "+ New Shell", &app.theme) {
                    app.spawn_local_terminal(ctx.clone(), None);
                }

                // "+ New SSH" — click opens a popup of saved profiles, with a
                // search field that auto-focuses on open.
                {
                    let ssh_btn = crate::modern::Button3D::new("+ New SSH")
                        .small()
                        .fill(app.theme.accent_color())
                        .edge(crate::modern::darken(app.theme.accent_color(), 55))
                        .text_color(app.theme.on_accent_color())
                        .show(ui, &app.theme);

                    let popup_id = ui.make_persistent_id("azterm_new_ssh_popup");
                    let search_key = popup_id.with("search_query");
                    let fresh_key = popup_id.with("fresh");

                    // When the popup is closed, wipe the search query and
                    // mark the next open as "fresh" (so the search field
                    // auto-focuses once, then yields to normal focus).
                    if !ui.memory(|m| m.is_popup_open(popup_id)) {
                        ui.memory_mut(|m| {
                            m.data.remove::<String>(search_key);
                            m.data.insert_temp(fresh_key, true);
                        });
                    }

                    if ssh_btn.clicked() {
                        ui.memory_mut(|m| m.toggle_popup(popup_id));
                    }

                    // Deferred actions — the popup closure borrows `app`
                    // immutably, so anything needing `&mut app` runs after.
                    let mut connect_profile: Option<SshProfile> = None;
                    let mut open_new_profile = false;
                    let mut close_menu = false;

                    egui::popup::popup_below_widget(
                        ui,
                        popup_id,
                        &ssh_btn,
                        egui::PopupCloseBehavior::CloseOnClickOutside,
                        |ui: &mut egui::Ui| {
                            ui.set_min_width(300.0);
                            ui.set_max_width(380.0);

                            ui.label(
                                egui::RichText::new("Connect to SSH profile")
                                    .strong()
                                    .color(app.theme.text_primary_color()),
                            );
                            ui.add_space(4.0);

                            // ---- Search field --------------------------
                            let mut query: String = ui
                                .memory(|m| m.data.get_temp(search_key).unwrap_or_default());

                            let search_resp = ui.add(
                                egui::TextEdit::singleline(&mut query)
                                    .hint_text("Search name, host, or user...")
                                    .desired_width(f32::INFINITY),
                            );

                            // Auto-focus the search field on the first
                            // frame the popup is open. Subsequent frames
                            // respect whatever the user focuses.
                            let is_fresh: bool = ui
                                .memory(|m| m.data.get_temp(fresh_key).unwrap_or(true));
                            if is_fresh {
                                search_resp.request_focus();
                                ui.memory_mut(|m| m.data.insert_temp(fresh_key, false));
                            }

                            ui.memory_mut(|m| {
                                m.data.insert_temp(search_key, query.clone());
                            });

                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(2.0);

                            if app.ssh_store.profiles.is_empty() {
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new("No saved SSH profiles yet.")
                                        .small()
                                        .color(app.theme.text_muted_color()),
                                );
                                ui.add_space(6.0);
                            } else {
                                let q = query.to_lowercase();
                                let matches: Vec<&SshProfile> = app
                                    .ssh_store
                                    .profiles
                                    .iter()
                                    .filter(|p| {
                                        q.is_empty()
                                            || p.name.to_lowercase().contains(&q)
                                            || p.host.to_lowercase().contains(&q)
                                            || p.username.to_lowercase().contains(&q)
                                    })
                                    .collect();

                                if matches.is_empty() {
                                    ui.add_space(10.0);
                                    ui.label(
                                        egui::RichText::new("No profiles match your search.")
                                            .small()
                                            .color(app.theme.text_muted_color()),
                                    );
                                    ui.add_space(10.0);
                                } else {
                                    // Scroll list — pinned to a fixed
                                    // max height so many profiles stay
                                    // usable without overflowing the
                                    // screen. egui also repositions the
                                    // popup above the button if there
                                    // isn't room below.
                                    egui::ScrollArea::vertical()
                                        .max_height(320.0)
                                        .auto_shrink([false, true])
                                        .show(ui, |ui| {
                                            for profile in matches {
                                                let title = profile.name.clone();
                                                let subtitle = format!(
                                                    "{}@{}:{}",
                                                    profile.username,
                                                    profile.host,
                                                    profile.port
                                                );

                                                let row_height = 46.0_f32;
                                                let row_width =
                                                    ui.available_width().max(220.0);
                                                let (row_rect, _) = ui.allocate_exact_size(
                                                    egui::vec2(row_width, row_height),
                                                    egui::Sense::hover(),
                                                );

                                                let pointer_over_row =
                                                    ui.rect_contains_pointer(row_rect);
                                                if pointer_over_row {
                                                    ui.painter().rect_filled(
                                                        row_rect,
                                                        6.0,
                                                        app.theme.bg_card_color(),
                                                    );
                                                }

                                                let inner = row_rect
                                                    .shrink2(egui::vec2(12.0, 8.0));
                                                ui.allocate_ui_at_rect(inner, |ui| {
                                                    ui.vertical(|ui| {
                                                        ui.label(
                                                            egui::RichText::new(&title)
                                                                .strong()
                                                                .color(app.theme.text_primary_color()),
                                                        );
                                                        ui.label(
                                                            egui::RichText::new(&subtitle)
                                                                .small()
                                                                .color(app.theme.text_muted_color()),
                                                        );
                                                    });
                                                });

                                                // Full-row click target, added
                                                // AFTER the labels so it wins
                                                // hit-testing.
                                                let row_id = ui
                                                    .id()
                                                    .with("ssh_profile_row")
                                                    .with(&profile.id);
                                                let row_resp = ui.interact(
                                                    row_rect,
                                                    row_id,
                                                    egui::Sense::click(),
                                                );

                                                if row_resp.hovered() {
                                                    ui.ctx().set_cursor_icon(
                                                        egui::CursorIcon::PointingHand,
                                                    );
                                                }
                                                if row_resp.clicked() {
                                                    connect_profile = Some(profile.clone());
                                                    close_menu = true;
                                                }

                                                ui.add_space(4.0);
                                                ui.separator();
                                            }
                                        });
                                }
                            }

                            ui.add_space(2.0);
                            ui.separator();
                            ui.add_space(2.0);
                            if ui.button("+ New SSH Profile…").clicked() {
                                open_new_profile = true;
                                close_menu = true;
                            }
                        },
                    );

                    if close_menu {
                        ui.memory_mut(|m| m.close_popup());
                    }
                    if let Some(p) = connect_profile {
                        app.spawn_ssh_terminal(&p, ctx.clone());
                    }
                    if open_new_profile {
                        app.open_create_profile_modal();
                    }
                }

                // Divider between the “open new session” group and the
                // active-tab layout controls.
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                                if app.active_view == ActiveView::Terminal {
                    if crate::modern::accent_button_small(ui, &app.theme, "Split H").on_hover_text("Split active pane side-by-side (Ctrl+Shift+D)").clicked() {
                        app.split_active_pane(SplitDirection::Horizontal, ctx.clone());
                    }
                    if crate::modern::accent_button_small(ui, &app.theme, "Split V").on_hover_text("Split active pane top-and-bottom (Ctrl+Shift+E)").clicked() {
                        app.split_active_pane(SplitDirection::Vertical, ctx.clone());
                    }
                    if let Some(ws) = app.workspaces.get(app.active_workspace_idx) {
                        if !ws.is_single_pane() {
                            let max_label = if ws.maximized_session.is_some() { "Restore" } else { "Max" };
                            if crate::modern::accent_button_small(ui, &app.theme, max_label).on_hover_text("Toggle maximize active pane (Ctrl+Shift+M)").clicked() {
                                if let Some(ws_mut) = app.workspaces.get_mut(app.active_workspace_idx) {
                                    ws_mut.maximized_session = if ws_mut.maximized_session.is_some() { None } else { Some(app.active_session_id) };
                                }
                            }
                        }
                    }

                    let current_is_multi = app.workspaces.get(app.active_workspace_idx).map_or(false, |w| !w.is_single_pane());
                    if current_is_multi {
                        if crate::modern::accent_button_small(ui, &app.theme, "Untile").on_hover_text("Detach tiled panes in this tab into separate tabs").clicked() {
                            app.untile_all_to_tabs();
                        }
                    }

                    let all_sessions_count = app.sessions.len();
                    let optimal_tabs_count = (all_sessions_count + 15) / 16;
                    let can_tile_more = all_sessions_count >= 2
                        && (app.workspaces.len() > optimal_tabs_count || app.workspaces.iter().any(|w| w.is_single_pane()));

                    if can_tile_more {
                        if crate::modern::accent_button_small(ui, &app.theme, "Tile All").on_hover_text("Tile all open tabs into balanced grids (batches of 16 per tab)").clicked() {
                            app.tile_all_tabs();
                        }
                    }
                }

                if !app.settings.use_system_titlebar {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::modern::Button3D::new("X")
                            .compact()
                            .min_size(egui::vec2(28.0, 22.0))
                            .fill(app.theme.bg_card_color())
                            .edge(crate::modern::darken(app.theme.bg_card_color(), 40))
                            .text_color(app.theme.text_primary_color())
                            .show(ui, &app.theme)
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }

                        let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        let max_icon = if is_max { "Restore" } else { "Max" };
                        if crate::modern::Button3D::new(max_icon)
                            .compact()
                            .min_size(egui::vec2(44.0, 22.0))
                            .fill(app.theme.bg_card_color())
                            .edge(crate::modern::darken(app.theme.bg_card_color(), 40))
                            .text_color(app.theme.text_primary_color())
                            .show(ui, &app.theme)
                            .clicked()
                        {
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
                .stroke(egui::Stroke::new(1.0_f32, app.theme.border_color()))
                .inner_margin(egui::Margin::symmetric(14.0, 4.0)),
        )
        .show(ctx, |ui| {
            let bar_rect = ui.painter().clip_rect();
            let base = app.theme.bg_panel_color().linear_multiply(0.85);
            crate::modern::gradient_rect(
                ui.painter(),
                bar_rect,
                crate::modern::lighten(base, 10),
                crate::modern::darken(base, 12),
            );
            let mut transition_pending = false;
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
                                    // Deferred: firing trigger_transition() here would
                                    // need &mut app while the workspaces iterator is
                                    // still borrowing app. Set a flag and fire after
                                    // the loop exits.
                                    transition_pending = true;
                                }

                                if close_clicked {
                                    tab_to_close = Some(i);
                                }

                                ui.add_space(4.0);
                            }
                        });
                    });

                if transition_pending {
                    app.trigger_transition();
                }

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
    app.sftp.poll_transfers(ctx);

    egui::TopBottomPanel::bottom("bottom_status_bar")
        .frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(14.0, 4.0)))
        .show(ctx, |ui| {
            let bar_rect = ui.painter().clip_rect();
            let base = app.theme.bg_panel_color();
            crate::modern::gradient_rect(
                ui.painter(),
                bar_rect,
                crate::modern::lighten(base, 6),
                crate::modern::darken(base, 14),
            );
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
                    app.trigger_transition();
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
