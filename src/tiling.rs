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

    pub fn split_leaf(
        &mut self,
        target_id: usize,
        new_id: usize,
        dir: SplitDirection,
        insert_after: bool,
        split_id: usize,
    ) -> bool {
        match self {
            TileNode::Leaf(id) if *id == target_id => {
                let (first, second) = if insert_after {
                    (TileNode::Leaf(target_id), TileNode::Leaf(new_id))
                } else {
                    (TileNode::Leaf(new_id), TileNode::Leaf(target_id))
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
                if first.split_leaf(target_id, new_id, dir, insert_after, split_id) {
                    true
                } else {
                    second.split_leaf(target_id, new_id, dir, insert_after, split_id)
                }
            }
            _ => false,
        }
    }

    pub fn remove_leaf(&mut self, target_id: usize) -> bool {
        match self {
            TileNode::Leaf(id) if *id == target_id => false,
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
            _ => false,
        }
    }

    pub fn first_leaf(&self) -> usize {
        match self {
            TileNode::Leaf(id) => *id,
            TileNode::Split { first, .. } => first.first_leaf(),
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
    if !rect.contains(pos) {
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
