// src/modern.rs
//
// Modern depth helpers: soft shadows, neon accent glows, gradient rects,
// and chunky 3D buttons with hover-lift / press-in behaviour.

use crate::theme::ThemeConfig;
use eframe::egui;

pub fn lighten(c: egui::Color32, a: u8) -> egui::Color32 {
    egui::Color32::from_rgb(
        c.r().saturating_add(a),
        c.g().saturating_add(a),
        c.b().saturating_add(a),
    )
}

pub fn darken(c: egui::Color32, a: u8) -> egui::Color32 {
    egui::Color32::from_rgb(
        c.r().saturating_sub(a),
        c.g().saturating_sub(a),
        c.b().saturating_sub(a),
    )
}

pub fn is_dark(c: egui::Color32) -> bool {
    let lum = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    lum < 140.0
}

pub fn gradient_rect(
    painter: &egui::Painter,
    rect: egui::Rect,
    top: egui::Color32,
    bottom: egui::Color32,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 { return; }
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

pub fn accent_glow(
    painter: &egui::Painter,
    rect: egui::Rect,
    accent: egui::Color32,
    corner_radius: f32,
    intensity: f32,
    spread: f32,
) {
    let layers = 7;
    let k = intensity.clamp(0.0, 1.5);
    let s = spread.clamp(0.15, 2.0);
    for i in 0..layers {
        let t = i as f32 / layers as f32;
        let grow = (1.0 + i as f32 * 2.0) * s;
        let alpha = ((1.0 - t).powi(2) * 120.0 * k) as u8;
        if alpha == 0 { continue; }
        painter.rect_stroke(
            rect.expand(grow),
            egui::Rounding::same(corner_radius + grow),
            egui::Stroke::new(
                1.4,
                egui::Color32::from_rgba_unmultiplied(
                    accent.r(), accent.g(), accent.b(), alpha,
                ),
            ),
        );
    }
}

#[allow(dead_code)]
pub fn soft_shadow(
    painter: &egui::Painter,
    rect: egui::Rect,
    corner_radius: f32,
    offset_y: f32,
    strength: u8,
) {
    let layers = 5;
    for i in (0..layers).rev() {
        let t = i as f32 / layers as f32;
        let grow = 1.0 + i as f32 * 2.5;
        let alpha = ((1.0 - t) * (strength as f32) * 0.30) as u8;
        if alpha == 0 { continue; }
        painter.rect_filled(
            rect.translate(egui::vec2(0.0, offset_y)).expand(grow),
            egui::Rounding::same(corner_radius + grow * 0.5),
            egui::Color32::from_black_alpha(alpha),
        );
    }
}

// ---------------------------------------------------------------------------
// Chunky 3D button
// ---------------------------------------------------------------------------

pub struct Button3D {
    text: String,
    min_size: egui::Vec2,
    fill: Option<egui::Color32>,
    edge: Option<egui::Color32>,
    fg: Option<egui::Color32>,
    border: Option<egui::Color32>,
    rounding: f32,
    depth: f32,
    font_size: f32,
    padding: egui::Vec2,
    hover_lift: bool,
}

impl Button3D {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            min_size: egui::vec2(0.0, 0.0),
            fill: None,
            edge: None,
            fg: None,
            border: None,
            rounding: 8.0,
            depth: 4.0,
            font_size: 13.5,
            padding: egui::vec2(16.0, 8.0),
            hover_lift: true,
        }
    }

    pub fn small(mut self) -> Self {
        self.depth = 3.0;
        self.rounding = 6.0;
        self.font_size = 12.0;
        self.padding = egui::vec2(10.0, 4.0);
        self
    }

    pub fn compact(mut self) -> Self {
        self.depth = 2.0;
        self.rounding = 5.0;
        self.font_size = 11.0;
        self.padding = egui::vec2(7.0, 3.0);
        self
    }

    pub fn min_size(mut self, size: egui::Vec2) -> Self { self.min_size = size; self }
    pub fn fill(mut self, c: egui::Color32) -> Self { self.fill = Some(c); self }
    pub fn edge(mut self, c: egui::Color32) -> Self { self.edge = Some(c); self }
    pub fn text_color(mut self, c: egui::Color32) -> Self { self.fg = Some(c); self }
    pub fn border(mut self, c: egui::Color32) -> Self { self.border = Some(c); self }
    pub fn no_lift(mut self) -> Self { self.hover_lift = false; self }

    pub fn show(self, ui: &mut egui::Ui, theme: &ThemeConfig) -> egui::Response {
        let font_id = egui::FontId::proportional(self.font_size);
        let base_fill = self.fill.unwrap_or_else(|| theme.bg_card_color());
        let text_color = self.fg.unwrap_or_else(|| {
            if is_dark(base_fill) { egui::Color32::WHITE }
            else { egui::Color32::from_rgb(15, 23, 42) }
        });

        let galley = ui.painter().layout_no_wrap(self.text.clone(), font_id, text_color);
        let text_size = galley.size();
        let body_size = egui::vec2(
            (text_size.x + self.padding.x * 2.0).max(self.min_size.x),
            (text_size.y + self.padding.y * 2.0).max(self.min_size.y),
        );

        let lift_margin = if self.hover_lift { 2.5 } else { 0.0 };
        let total_size = egui::vec2(body_size.x, body_size.y + self.depth + lift_margin);
        let (outer_rect, response) = ui.allocate_exact_size(total_size, egui::Sense::click());

        let enabled = ui.is_enabled();
        let hovered = response.hovered() && enabled;
        let pressed = response.is_pointer_button_down_on() && enabled;

        let body_rest = egui::Rect::from_min_size(
            egui::pos2(outer_rect.min.x, outer_rect.min.y + lift_margin),
            body_size,
        );

        let body_rect = if pressed {
            body_rest.translate(egui::vec2(0.0, self.depth))
        } else if hovered && self.hover_lift {
            body_rest.translate(egui::vec2(0.0, -lift_margin * 0.55))
        } else {
            body_rest
        };

        let shadow_rect = egui::Rect::from_min_size(
            egui::pos2(body_rect.min.x, body_rect.min.y + self.depth),
            body_size,
        );

        let base_edge = self.edge.unwrap_or_else(|| darken(base_fill, 45));
        let (body_fill, edge_fill) = if !enabled {
            (darken(base_fill, 25), darken(base_edge, 20))
        } else if pressed {
            (darken(base_fill, 18), base_edge)
        } else if hovered {
            (lighten(base_fill, 16), lighten(base_edge, 12))
        } else {
            (base_fill, base_edge)
        };

        if !pressed {
            ui.painter().rect_filled(
                shadow_rect,
                egui::Rounding::same(self.rounding + 1.0),
                edge_fill,
            );
        }

        if let Some(border) = self.border {
            ui.painter().rect(
                body_rect,
                egui::Rounding::same(self.rounding),
                body_fill,
                egui::Stroke::new(1.5, border),
            );
        } else {
            ui.painter().rect_filled(
                body_rect,
                egui::Rounding::same(self.rounding),
                body_fill,
            );
        }

        let text_pos = egui::pos2(
            body_rect.center().x - text_size.x / 2.0,
            body_rect.center().y - text_size.y / 2.0,
        );
        ui.painter().galley(text_pos, galley, text_color);

        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        response
    }
}

pub fn button(ui: &mut egui::Ui, theme: &ThemeConfig, text: &str) -> egui::Response {
    Button3D::new(text).show(ui, theme)
}

pub fn button_small(ui: &mut egui::Ui, theme: &ThemeConfig, text: &str) -> egui::Response {
    Button3D::new(text).small().show(ui, theme)
}

pub fn button_accent(ui: &mut egui::Ui, theme: &ThemeConfig, text: &str) -> egui::Response {
    let fill = theme.accent_color();
    Button3D::new(text)
        .fill(fill)
        .edge(darken(fill, 55))
        .text_color(egui::Color32::from_rgb(15, 23, 42))
        .show(ui, theme)
}

pub fn button_outline(ui: &mut egui::Ui, theme: &ThemeConfig, text: &str) -> egui::Response {
    Button3D::new(text)
        .fill(theme.bg_main_color())
        .edge(theme.accent_color())
        .text_color(theme.text_primary_color())
        .border(theme.accent_color())
        .show(ui, theme)
}

pub fn button_danger(ui: &mut egui::Ui, theme: &ThemeConfig, text: &str) -> egui::Response {
    let fill = theme.danger_color();
    Button3D::new(text)
        .fill(fill)
        .edge(darken(fill, 55))
        .text_color(egui::Color32::WHITE)
        .show(ui, theme)
}
