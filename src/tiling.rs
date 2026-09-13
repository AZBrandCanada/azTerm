// src
use crate::settings::AppSettings;
use crate::terminal::TerminalSession;
use crate::theme::ThemeConfig;
use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TileNode {
    Leaf(usize),
    Split {
        id: usize,
        dir: SplitDirection,
        ratio: f32,
        first: Box<TileNode>,
        second: Box<TileNode>,
    },
}

impl TileNode {
    pub fn leaves(&self) -> Vec<usize> {
        let mut list = Vec::new();
        self.collect_leaves(&mut list);
        list
    }

    fn collect_leaves(&self, list: &mut Vec<usize>) {
        match self {
            TileNode::Leaf(id) => list.push(*id),
            TileNode::Split { first, second, .. } => {
                first.collect_leaves(list);
                second.collect_leaves(list);
            }
        }
    }

    pub fn contains(&self, session_id: usize) -> bool {
        match self {
            TileNode::Leaf(id) => *id == session_id,
            TileNode::Split { first, second, .. } => {
                first.contains(session_id) || second.contains(session_id)
            }
        }
    }

    pub fn is_single_leaf(&self) -> bool {
        matches!(self, TileNode::Leaf(_))
    }

    pub fn first_leaf(&self) -> usize {
        match self {
            TileNode::Leaf(id) => *id,
            TileNode::Split { first, .. } => first.first_leaf(),
        }
    }

    pub fn split_leaf_with_node(
        &mut self,
        target_id: usize,
        incoming: TileNode,
        dir: SplitDirection,
        insert_after: bool,
        split_id: usize,
    ) -> bool {
        match self {
            TileNode::Leaf(id) if *id == target_id => {
                let (first, second) = if insert_after {
                    (TileNode::Leaf(target_id), incoming)
                } else {
                    (incoming, TileNode::Leaf(target_id))
                };
                *self = TileNode::Split {
                    id: split_id,
                    dir,
                    ratio: 0.5,
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            TileNode::Split { first, second, .. } => {
                if first.split_leaf_with_node(target_id, incoming.clone(), dir, insert_after, split_id) {
                    true
                } else {
                    second.split_leaf_with_node(target_id, incoming, dir, insert_after, split_id)
                }
            }
            _ => false,
        }
    }

    pub fn split_leaf(
        &mut self,
        target_id: usize,
        new_session_id: usize,
        dir: SplitDirection,
        insert_after: bool,
        split_id: usize,
    ) -> bool {
        self.split_leaf_with_node(target_id, TileNode::Leaf(new_session_id), dir, insert_after, split_id)
    }

    pub fn remove_leaf(&mut self, target_id: usize) -> bool {
        match self {
            TileNode::Leaf(_) => false,
            TileNode::Split { first, second, .. } => {
                if let TileNode::Leaf(id) = **first {
                    if id == target_id {
                        *self = *second.clone();
                        return true;
                    }
                }
                if let TileNode::Leaf(id) = **second {
                    if id == target_id {
                        *self = *first.clone();
                        return true;
                    }
                }
                if first.remove_leaf(target_id) {
                    true
                } else {
                    second.remove_leaf(target_id)
                }
            }
        }
    }
}

/// Recursively builds a clean, balanced binary tree for any number of leaves
pub fn build_balanced_tree(leaves: &[usize], next_split_id: &mut usize, dir: SplitDirection) -> TileNode {
    match leaves.len() {
        0 => TileNode::Leaf(1),
        1 => TileNode::Leaf(leaves[0]),
        2 => {
            let id = *next_split_id;
            *next_split_id += 1;
            TileNode::Split {
                id,
                dir,
                ratio: 0.5,
                first: Box::new(TileNode::Leaf(leaves[0])),
                second: Box::new(TileNode::Leaf(leaves[1])),
            }
        }
        n => {
            let mid = n / 2;
            let first_slice = &leaves[..mid];
            let second_slice = &leaves[mid..];
            let id = *next_split_id;
            *next_split_id += 1;
            let next_dir = match dir {
                SplitDirection::Horizontal => SplitDirection::Vertical,
                SplitDirection::Vertical => SplitDirection::Horizontal,
            };
            TileNode::Split {
                id,
                dir,
                ratio: 0.5,
                first: Box::new(build_balanced_tree(first_slice, next_split_id, next_dir)),
                second: Box::new(build_balanced_tree(second_slice, next_split_id, next_dir)),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceTab {
    pub id: usize,
    pub title: String,
    pub root: TileNode,
    pub maximized_session: Option<usize>,
}

impl WorkspaceTab {
    pub fn new(id: usize, session_id: usize, title: String) -> Self {
        Self {
            id,
            title,
            root: TileNode::Leaf(session_id),
            maximized_session: None,
        }
    }

    pub fn is_single_pane(&self) -> bool {
        self.root.is_single_leaf()
    }

    pub fn leaves(&self) -> Vec<usize> {
        self.root.leaves()
    }

    pub fn contains(&self, session_id: usize) -> bool {
        self.root.contains(session_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockZone {
    Left,
    Right,
    Top,
    Bottom,
}

pub fn detect_dock_zone(rect: egui::Rect, pos: egui::Pos2) -> Option<DockZone> {
    if !rect.contains(pos) || rect.width() < 60.0 || rect.height() < 40.0 {
        return None;
    }
    let rel_x = (pos.x - rect.min.x) / rect.width();
    let rel_y = (pos.y - rect.min.y) / rect.height();

    let thresh = 0.28;
    if rel_x < thresh {
        Some(DockZone::Left)
    } else if rel_x > 1.0 - thresh {
        Some(DockZone::Right)
    } else if rel_y < thresh {
        Some(DockZone::Top)
    } else if rel_y > 1.0 - thresh {
        Some(DockZone::Bottom)
    } else {
        None
    }
}

pub fn dock_zone_preview_rect(pane_rect: egui::Rect, zone: DockZone) -> egui::Rect {
    match zone {
        DockZone::Left => egui::Rect::from_min_max(
            pane_rect.min,
            egui::pos2(pane_rect.center().x, pane_rect.max.y),
        ),
        DockZone::Right => egui::Rect::from_min_max(
            egui::pos2(pane_rect.center().x, pane_rect.min.y),
            pane_rect.max,
        ),
        DockZone::Top => egui::Rect::from_min_max(
            pane_rect.min,
            egui::pos2(pane_rect.max.x, pane_rect.center().y),
        ),
        DockZone::Bottom => egui::Rect::from_min_max(
            egui::pos2(pane_rect.min.x, pane_rect.center().y),
            pane_rect.max,
        ),
    }
}

#[allow(dead_code)]
pub enum PaneAction {
    Split(usize, SplitDirection),
    ToggleMaximize(usize),
    PopToTab(usize),
    Close(usize),
    Focus(usize),
    StartDrag(usize),
}

pub fn render_tile_tree(
    ui: &mut egui::Ui,
    node: &mut TileNode,
    total_rect: egui::Rect,
    theme: &ThemeConfig,
    settings: &AppSettings,
    sessions: &mut [TerminalSession],
    active_session_id: &mut usize,
    maximized_session: Option<usize>,
    toast: &mut Option<(String, std::time::Instant)>,
    actions: &mut Vec<PaneAction>,
    pane_rects: &mut Vec<(usize, egui::Rect)>,
    is_multi_pane: bool,
) {
    if let Some(max_id) = maximized_session {
        if node.contains(max_id) {
            pane_rects.push((max_id, total_rect));
            if let Some(session) = sessions.iter_mut().find(|s| s.id == max_id) {
                render_single_pane(
                    ui,
                    session,
                    total_rect,
                    theme,
                    settings,
                    active_session_id,
                    true,
                    true,
                    toast,
                    actions,
                );
            }
            return;
        }
    }

    match node {
        TileNode::Leaf(session_id) => {
            pane_rects.push((*session_id, total_rect));
            if let Some(session) = sessions.iter_mut().find(|s| s.id == *session_id) {
                render_single_pane(
                    ui,
                    session,
                    total_rect,
                    theme,
                    settings,
                    active_session_id,
                    is_multi_pane,
                    false,
                    toast,
                    actions,
                );
            }
        }
        TileNode::Split { id, dir, ratio, first, second } => {
            let divider_thick = 5.0_f32;
            let safe_ratio = if ratio.is_nan() || *ratio <= 0.0 || *ratio >= 1.0 { 0.5 } else { *ratio };
            match dir {
                SplitDirection::Horizontal => {
                    let avail_w = (total_rect.width() - divider_thick).max(1.0);
                    let min_pane_w = 30.0_f32;
                    let max_pane_w = (avail_w - min_pane_w).max(min_pane_w);
                    let w1 = if avail_w <= min_pane_w * 2.0 {
                        (avail_w * 0.5).max(1.0)
                    } else {
                        (avail_w * safe_ratio).clamp(min_pane_w, max_pane_w)
                    };
                    let w2 = (avail_w - w1).max(1.0);

                    let r1 = egui::Rect::from_min_size(total_rect.min, egui::vec2(w1, total_rect.height()));
                    let div_rect = egui::Rect::from_min_size(egui::pos2(total_rect.min.x + w1, total_rect.min.y), egui::vec2(divider_thick, total_rect.height()));
                    let r2 = egui::Rect::from_min_size(egui::pos2(total_rect.min.x + w1 + divider_thick, total_rect.min.y), egui::vec2(w2, total_rect.height()));

                    let div_id = ui.id().with("split_div_h").with(*id);
                    let div_resp = ui.interact(div_rect, div_id, egui::Sense::click_and_drag());
                    if div_resp.hovered() || div_resp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    if div_resp.dragged() && avail_w > 10.0 {
                        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                            let new_r = ((pos.x - total_rect.min.x) / avail_w).clamp(0.08, 0.92);
                            if !new_r.is_nan() {
                                *ratio = new_r;
                            }
                        }
                    }
                    let div_color = if div_resp.dragged() {
                        theme.accent_color()
                    } else if div_resp.hovered() {
                        theme.accent_hover_color()
                    } else {
                        theme.border_color()
                    };
                    ui.painter().rect_filled(div_rect, 0.0, div_color);

                    render_tile_tree(ui, first, r1, theme, settings, sessions, active_session_id, maximized_session, toast, actions, pane_rects, true);
                    render_tile_tree(ui, second, r2, theme, settings, sessions, active_session_id, maximized_session, toast, actions, pane_rects, true);
                }
                SplitDirection::Vertical => {
                    let avail_h = (total_rect.height() - divider_thick).max(1.0);
                    let min_pane_h = 25.0_f32;
                    let max_pane_h = (avail_h - min_pane_h).max(min_pane_h);
                    let h1 = if avail_h <= min_pane_h * 2.0 {
                        (avail_h * 0.5).max(1.0)
                    } else {
                        (avail_h * safe_ratio).clamp(min_pane_h, max_pane_h)
                    };
                    let h2 = (avail_h - h1).max(1.0);

                    let r1 = egui::Rect::from_min_size(total_rect.min, egui::vec2(total_rect.width(), h1));
                    let div_rect = egui::Rect::from_min_size(egui::pos2(total_rect.min.x, total_rect.min.y + h1), egui::vec2(total_rect.width(), divider_thick));
                    let r2 = egui::Rect::from_min_size(egui::pos2(total_rect.min.x, total_rect.min.y + h1 + divider_thick), egui::vec2(total_rect.width(), h2));

                    let div_id = ui.id().with("split_div_v").with(*id);
                    let div_resp = ui.interact(div_rect, div_id, egui::Sense::click_and_drag());
                    if div_resp.hovered() || div_resp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    if div_resp.dragged() && avail_h > 10.0 {
                        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                            let new_r = ((pos.y - total_rect.min.y) / avail_h).clamp(0.08, 0.92);
                            if !new_r.is_nan() {
                                *ratio = new_r;
                            }
                        }
                    }
                    let div_color = if div_resp.dragged() {
                        theme.accent_color()
                    } else if div_resp.hovered() {
                        theme.accent_hover_color()
                    } else {
                        theme.border_color()
                    };
                    ui.painter().rect_filled(div_rect, 0.0, div_color);

                    render_tile_tree(ui, first, r1, theme, settings, sessions, active_session_id, maximized_session, toast, actions, pane_rects, true);
                    render_tile_tree(ui, second, r2, theme, settings, sessions, active_session_id, maximized_session, toast, actions, pane_rects, true);
                }
            }
        }
    }
}

pub fn render_single_pane(
    ui: &mut egui::Ui,
    session: &mut TerminalSession,
    rect: egui::Rect,
    theme: &ThemeConfig,
    settings: &AppSettings,
    active_session_id: &mut usize,
    show_header: bool,
    is_maximized: bool,
    toast: &mut Option<(String, std::time::Instant)>,
    actions: &mut Vec<PaneAction>,
) {
    if rect.width() < 10.0 || rect.height() < 10.0 {
        return;
    }

    let is_focused = *active_session_id == session.id;
    let border_color = if is_focused { theme.accent_color() } else { theme.border_color() };

    let header_height = if show_header && rect.height() > 34.0 { 24.0_f32 } else { 0.0_f32 };
    let body_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, (rect.min.y + header_height).min(rect.max.y)),
        rect.max,
    );

    ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(1.0_f32, border_color));

    if header_height > 0.0 {
        let header_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), header_height));
        let header_bg = if is_focused { theme.bg_card_color() } else { theme.bg_panel_color() };
        ui.painter().rect_filled(header_rect, egui::Rounding { nw: 4.0, ne: 4.0, sw: 0.0, se: 0.0 }, header_bg);

        let drag_area_w = (header_rect.width() - 140.0).max(10.0);
        let drag_area_rect = egui::Rect::from_min_size(header_rect.min, egui::vec2(drag_area_w, header_height));
        let drag_resp = ui.interact(drag_area_rect, ui.id().with("pane_hdr_drag").with(session.id), egui::Sense::click_and_drag());

        if drag_resp.clicked() {
            *active_session_id = session.id;
        }
        if drag_resp.drag_started_by(egui::PointerButton::Primary) {
            actions.push(PaneAction::StartDrag(session.id));
        }

        ui.allocate_ui_at_rect(header_rect, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("::").weak().color(theme.text_muted_color()));
                let title_color = if is_focused { theme.accent_color() } else { theme.text_muted_color() };
                let max_title_chars = ((drag_area_w - 20.0) / 7.2).max(1.0) as usize;
                let title_disp = if session.title.len() > max_title_chars {
                    format!("{}…", &session.title[..max_title_chars.saturating_sub(1)])
                } else {
                    session.title.clone()
                };
                ui.label(egui::RichText::new(title_disp).strong().small().color(title_color));

                if is_focused {
                    ui.label(egui::RichText::new("●").small().color(theme.accent_color()));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(egui::RichText::new("×").strong().color(theme.danger_color())).on_hover_text("Close Pane (Ctrl+Shift+W)").clicked() {
                        actions.push(PaneAction::Close(session.id));
                    }
                    if rect.width() >= 130.0 {
                        if ui.small_button("Pop").on_hover_text("Pop out to a separate tab").clicked() {
                            actions.push(PaneAction::PopToTab(session.id));
                        }
                    }
                    if rect.width() >= 160.0 {
                        let max_text = if is_maximized { "Restore" } else { "Max" };
                        if ui.small_button(max_text).on_hover_text("Maximize / Restore Pane (Ctrl+Shift+M)").clicked() {
                            actions.push(PaneAction::ToggleMaximize(session.id));
                        }
                    }
                    if rect.width() >= 90.0 {
                        if ui.small_button("-").on_hover_text("Split Down (Ctrl+Shift+E)").clicked() {
                            actions.push(PaneAction::Split(session.id, SplitDirection::Vertical));
                        }
                        if ui.small_button("|").on_hover_text("Split Right (Ctrl+Shift+D)").clicked() {
                            actions.push(PaneAction::Split(session.id, SplitDirection::Horizontal));
                        }
                    }
                });
            });
        });
    }

    if body_rect.width() >= 10.0 && body_rect.height() >= 10.0 {
        let mut pane_clicked = false;
        ui.push_id(session.id, |ui| {
            ui.allocate_ui_at_rect(body_rect, |ui| {
                pane_clicked = session.render(ui, settings, theme, is_focused, toast);
            });
        });

        if pane_clicked {
            *active_session_id = session.id;
            ui.ctx().request_repaint();
        }
    }
}
