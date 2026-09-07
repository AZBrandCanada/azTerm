use eframe::egui;

pub const COLOR_BG_MAIN: egui::Color32 = egui::Color32::from_rgb(11, 15, 25);
pub const COLOR_BG_PANEL: egui::Color32 = egui::Color32::from_rgb(17, 24, 39);
pub const COLOR_BG_CARD: egui::Color32 = egui::Color32::from_rgb(26, 34, 52);
pub const COLOR_BORDER: egui::Color32 = egui::Color32::from_rgb(39, 49, 73);
pub const COLOR_ACCENT: egui::Color32 = egui::Color32::from_rgb(6, 182, 212); // Cyan
pub const COLOR_ACCENT_HOVER: egui::Color32 = egui::Color32::from_rgb(34, 211, 238);
pub const COLOR_INDIGO: egui::Color32 = egui::Color32::from_rgb(99, 102, 241);
pub const COLOR_TEXT_PRIMARY: egui::Color32 = egui::Color32::from_rgb(243, 244, 246);
pub const COLOR_TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(156, 163, 175);
pub const COLOR_SUCCESS: egui::Color32 = egui::Color32::from_rgb(34, 197, 94);
pub const COLOR_DANGER: egui::Color32 = egui::Color32::from_rgb(239, 68, 68);

pub fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(COLOR_BG_CARD)
        .stroke(egui::Stroke::new(1.0_f32, COLOR_BORDER))
        .rounding(8.0)
        .inner_margin(egui::Margin::same(14.0))
}

pub fn toggle_switch(ui: &mut egui::Ui, value: &mut bool, text: &str) -> egui::Response {
    ui.horizontal(|ui| {
        let desired_size = egui::vec2(36.0, 18.0);
        let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        if response.clicked() {
            *value = !*value;
            response.mark_changed();
        }
        if ui.is_rect_visible(rect) {
            let how_on = if *value { 1.0 } else { 0.0 };
            let bg = if *value { COLOR_ACCENT } else { egui::Color32::from_rgb(51, 65, 85) };
            let radius = 0.5 * rect.height();
            ui.painter().rect_filled(rect, radius, bg);
            let circle_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
            let center = egui::pos2(circle_x, rect.center().y);
            ui.painter().circle_filled(center, radius - 2.5, egui::Color32::WHITE);
        }
        if !text.is_empty() {
            ui.label(egui::RichText::new(text).color(COLOR_TEXT_PRIMARY));
        }
        response
    }).inner
}

pub fn setting_row_toggle(ui: &mut egui::Ui, title: &str, desc: &str, value: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).strong().color(COLOR_TEXT_PRIMARY));
            if !desc.is_empty() {
                ui.label(egui::RichText::new(desc).small().color(COLOR_TEXT_MUTED));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if toggle_switch(ui, value, "").changed() {
                changed = true;
            }
        });
    });
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);
    changed
}

pub fn setting_row_disabled(ui: &mut egui::Ui, title: &str, desc: &str, value: bool) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).strong().color(egui::Color32::from_rgb(148, 163, 184)));
                ui.label(
                    egui::RichText::new("[Soon]")
                        .small()
                        .color(egui::Color32::from_rgb(100, 116, 139)),
                );
            });
            if !desc.is_empty() {
                ui.label(egui::RichText::new(desc).small().color(egui::Color32::from_rgb(71, 85, 105)));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let desired_size = egui::vec2(36.0, 18.0);
            let (rect, _response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
            let radius = 0.5 * rect.height();
            ui.painter().rect_filled(rect, radius, egui::Color32::from_rgb(30, 41, 59));
            let circle_x = if value { rect.right() - radius } else { rect.left() + radius };
            let center = egui::pos2(circle_x, rect.center().y);
            ui.painter().circle_filled(center, radius - 2.5, egui::Color32::from_rgb(71, 85, 105));
        });
    });
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);
}
