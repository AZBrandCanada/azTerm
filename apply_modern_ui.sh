#!/usr/bin/env bash
set -euo pipefail
[ -f Cargo.toml ] || { echo "error: run from project root"; exit 1; }

# ---------------------------------------------------------------------------
# 1. Create src/modern.rs
# ---------------------------------------------------------------------------
cat > src/modern.rs << 'MODERN_RS'
// src/modern.rs
//
// Modern depth helpers: soft shadows, neon accent glows, and gradient
// rectangles drawn with egui primitives. No new dependencies.

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

/// Fast vertical gradient using a mesh (one draw call).
pub fn gradient_rect(
    painter: &egui::Painter,
    rect: egui::Rect,
    top: egui::Color32,
    bottom: egui::Color32,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// Neon glow around a focused element. Multiple expanding strokes with
/// quadratic alpha falloff — reads like light bleeding outward.
pub fn accent_glow(
    painter: &egui::Painter,
    rect: egui::Rect,
    accent: egui::Color32,
    corner_radius: f32,
    intensity: f32,
) {
    let layers = 7;
    let k = intensity.clamp(0.0, 1.5);
    for i in 0..layers {
        let t = i as f32 / layers as f32;
        let grow = 1.0 + i as f32 * 2.0;
        let alpha = ((1.0 - t).powi(2) * 120.0 * k) as u8;
        if alpha == 0 {
            continue;
        }
        painter.rect_stroke(
            rect.expand(grow),
            egui::Rounding::same(corner_radius + grow),
            egui::Stroke::new(
                1.6,
                egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), alpha),
            ),
        );
    }
}

/// Soft multi-layer shadow drawn behind a rect. Use for elements that
/// live outside of egui::Frame (custom-drawn cards, chrome, etc.).
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
        if alpha == 0 {
            continue;
        }
        painter.rect_filled(
            rect.translate(egui::vec2(0.0, offset_y)).expand(grow),
            egui::Rounding::same(corner_radius + grow * 0.5),
            egui::Color32::from_black_alpha(alpha),
        );
    }
}
MODERN_RS
echo "  created src/modern.rs"

# ---------------------------------------------------------------------------
# 2. Register the module in main.rs
# ---------------------------------------------------------------------------
python3 - << 'PY_EOF'
path = "src/main.rs"
with open(path) as f:
    src = f.read()

if "mod modern;" not in src:
    anchor = "mod debug_log;\n"
    assert anchor in src, "mod debug_log anchor not found"
    src = src.replace(anchor, anchor + "mod modern;\n", 1)
    with open(path, "w") as f:
        f.write(src)
    print("  main.rs: registered mod modern")
else:
    print("  main.rs: already registered")
PY_EOF

# ---------------------------------------------------------------------------
# 3. theme.rs: card_frame gets a real drop shadow
# ---------------------------------------------------------------------------
python3 - << 'PY_EOF'
path = "src/theme.rs"
with open(path) as f:
    src = f.read()

old = """    pub fn card_frame(&self) -> egui::Frame {
        egui::Frame::none()
            .fill(self.bg_card_color())
            .stroke(egui::Stroke::new(1.0_f32, self.border_color()))
            .rounding(8.0)
            .inner_margin(egui::Margin::same(14.0))
    }"""

new = """    pub fn card_frame(&self) -> egui::Frame {
        // Modern elevation: soft drop shadow so the card reads as
        // floating slightly above the panel background.
        egui::Frame::none()
            .fill(self.bg_card_color())
            .stroke(egui::Stroke::new(1.0_f32, self.border_color()))
            .rounding(10.0)
            .inner_margin(egui::Margin::same(14.0))
            .shadow(egui::epaint::Shadow {
                offset: egui::vec2(0.0, 6.0),
                blur: 20.0,
                spread: 0.0,
                color: egui::Color32::from_black_alpha(110),
            })
    }"""

assert old in src, "card_frame anchor not found in theme.rs"
src = src.replace(old, new, 1)

with open(path, "w") as f:
    f.write(src)
print("  theme.rs: card_frame now has a drop shadow")
PY_EOF

# ---------------------------------------------------------------------------
# 4. tiling.rs: focused pane gets an accent glow
# ---------------------------------------------------------------------------
python3 - << 'PY_EOF'
path = "src/tiling.rs"
with open(path) as f:
    src = f.read()

old = """    ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(1.0_f32, border_color));"""

new = """    // Neon accent glow around the currently focused pane. Layers of
    // expanding accent-tinted strokes make the border bleed outward
    // without any post-processing.
    if is_focused {
        crate::modern::accent_glow(
            ui.painter(),
            rect,
            theme.accent_color(),
            6.0,
            1.0,
        );
    }
    ui.painter().rect_stroke(rect, 6.0, egui::Stroke::new(1.4_f32, border_color));"""

assert old in src, "focused pane stroke anchor not found in tiling.rs"
src = src.replace(old, new, 1)

with open(path, "w") as f:
    f.write(src)
print("  tiling.rs: focused pane now glows")
PY_EOF

# ---------------------------------------------------------------------------
# 5. navbar.rs: gradient chrome on top nav, tabs bar, and status bar
# ---------------------------------------------------------------------------
python3 - << 'PY_EOF'
path = "src/ui/navbar.rs"
with open(path) as f:
    src = f.read()

# --- 5a. Top nav gradient --------------------------------------------------
old = """    egui::TopBottomPanel::top("top_nav")
        .frame(egui::Frame::none().fill(app.theme.bg_panel_color()).inner_margin(egui::Margin::symmetric(14.0, 7.0)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {"""

new = """    egui::TopBottomPanel::top("top_nav")
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
            ui.horizontal(|ui| {"""

assert old in src, "top_nav anchor not found in navbar.rs"
src = src.replace(old, new, 1)

# --- 5b. Tabs bar gradient -------------------------------------------------
old = """        .frame(
            egui::Frame::none()
                .fill(app.theme.bg_panel_color().linear_multiply(0.85))
                .stroke(egui::Stroke::new(1.0_f32, app.theme.border_color()))
                .inner_margin(egui::Margin::symmetric(14.0, 4.0)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {"""

new = """        .frame(
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
            ui.horizontal(|ui| {"""

assert old in src, "tabs_bar anchor not found in navbar.rs"
src = src.replace(old, new, 1)

# --- 5c. Status bar gradient ----------------------------------------------
old = """    egui::TopBottomPanel::bottom("bottom_status_bar")
        .frame(egui::Frame::none().fill(app.theme.bg_panel_color()).inner_margin(egui::Margin::symmetric(14.0, 4.0)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {"""

new = """    egui::TopBottomPanel::bottom("bottom_status_bar")
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
            ui.horizontal(|ui| {"""

assert old in src, "status_bar anchor not found in navbar.rs"
src = src.replace(old, new, 1)

with open(path, "w") as f:
    f.write(src)
print("  navbar.rs: gradients applied to top nav, tabs bar, status bar")
PY_EOF

echo
echo "All patches applied."
echo
echo "Next:  cargo build --release"
echo "       sudo install -Dm755 target/release/azterm /usr/local/bin/azterm"
echo "       pkill -9 azterm ; launch from the menu"
