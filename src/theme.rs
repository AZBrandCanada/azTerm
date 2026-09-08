use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub id: String,
    pub name: String,
    pub is_builtin: bool,
    pub opacity: f32,

    pub bg_main: [u8; 3],
    pub bg_panel: [u8; 3],
    pub bg_card: [u8; 3],
    pub border: [u8; 3],
    pub accent: [u8; 3],
    pub accent_hover: [u8; 3],
    pub text_primary: [u8; 3],
    pub text_muted: [u8; 3],
    pub success: [u8; 3],
    pub danger: [u8; 3],

    pub ansi_colors: [[u8; 3]; 16],
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self::cyber_cyan()
    }
}

impl ThemeConfig {
    pub fn bg_main_color(&self) -> egui::Color32 {
        let a = (self.opacity.clamp(0.20, 1.0) * 255.0).round() as u8;
        egui::Color32::from_rgba_unmultiplied(self.bg_main[0], self.bg_main[1], self.bg_main[2], a)
    }

    pub fn bg_panel_color(&self) -> egui::Color32 {
        let a = (self.opacity.clamp(0.20, 1.0) * 255.0).round() as u8;
        egui::Color32::from_rgba_unmultiplied(self.bg_panel[0], self.bg_panel[1], self.bg_panel[2], a)
    }

    pub fn bg_card_color(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.bg_card[0], self.bg_card[1], self.bg_card[2])
    }

    pub fn border_color(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.border[0], self.border[1], self.border[2])
    }

    pub fn accent_color(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.accent[0], self.accent[1], self.accent[2])
    }

    pub fn accent_hover_color(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.accent_hover[0], self.accent_hover[1], self.accent_hover[2])
    }

    pub fn text_primary_color(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.text_primary[0], self.text_primary[1], self.text_primary[2])
    }

    pub fn text_muted_color(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.text_muted[0], self.text_muted[1], self.text_muted[2])
    }

    pub fn success_color(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.success[0], self.success[1], self.success[2])
    }

    pub fn danger_color(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.danger[0], self.danger[1], self.danger[2])
    }

    pub fn ansi_color(&self, idx: u8) -> egui::Color32 {
        if (idx as usize) < 16 {
            let c = self.ansi_colors[idx as usize];
            egui::Color32::from_rgb(c[0], c[1], c[2])
        } else {
            ansi_idx_to_color_extended(idx)
        }
    }

    pub fn card_frame(&self) -> egui::Frame {
        egui::Frame::none()
            .fill(self.bg_card_color())
            .stroke(egui::Stroke::new(1.0_f32, self.border_color()))
            .rounding(8.0)
            .inner_margin(egui::Margin::same(14.0))
    }

    pub fn cyber_cyan() -> Self {
        Self {
            id: "cyber_cyan".to_string(),
            name: "Cyber Cyan (Default)".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [11, 15, 25],
            bg_panel: [17, 24, 39],
            bg_card: [26, 34, 52],
            border: [39, 49, 73],
            accent: [6, 182, 212],
            accent_hover: [34, 211, 238],
            text_primary: [243, 244, 246],
            text_muted: [156, 163, 175],
            success: [34, 197, 94],
            danger: [239, 68, 68],
            ansi_colors: [
                [11, 15, 25], [239, 68, 68], [34, 197, 94], [234, 179, 8],
                [99, 102, 241], [168, 85, 247], [6, 182, 212], [203, 213, 225],
                [71, 85, 105], [248, 113, 113], [74, 222, 128], [250, 204, 21],
                [129, 140, 248], [192, 132, 252], [34, 211, 238], [255, 255, 255],
            ],
        }
    }

    pub fn dracula() -> Self {
        Self {
            id: "dracula".to_string(),
            name: "Dracula".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [40, 42, 54],
            bg_panel: [33, 34, 44],
            bg_card: [55, 58, 74],
            border: [98, 114, 164],
            accent: [189, 147, 249],
            accent_hover: [255, 121, 198],
            text_primary: [248, 248, 242],
            text_muted: [139, 147, 168],
            success: [80, 250, 123],
            danger: [255, 85, 85],
            ansi_colors: [
                [33, 34, 44], [255, 85, 85], [80, 250, 123], [241, 250, 140],
                [189, 147, 249], [255, 121, 198], [139, 233, 253], [248, 248, 242],
                [98, 114, 164], [255, 110, 110], [105, 255, 148], [255, 255, 165],
                [214, 172, 255], [255, 146, 223], [164, 255, 255], [255, 255, 255],
            ],
        }
    }

    pub fn nord() -> Self {
        Self {
            id: "nord".to_string(),
            name: "Nord".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [46, 52, 64],
            bg_panel: [40, 45, 56],
            bg_card: [59, 66, 82],
            border: [76, 86, 106],
            accent: [136, 192, 208],
            accent_hover: [129, 161, 193],
            text_primary: [236, 239, 244],
            text_muted: [148, 156, 172],
            success: [163, 190, 140],
            danger: [191, 97, 106],
            ansi_colors: [
                [46, 52, 64], [191, 97, 106], [163, 190, 140], [235, 203, 139],
                [129, 161, 193], [180, 142, 173], [136, 192, 208], [229, 233, 240],
                [76, 86, 106], [209, 115, 124], [181, 208, 158], [253, 221, 157],
                [147, 179, 211], [198, 160, 191], [143, 188, 187], [236, 239, 244],
            ],
        }
    }

    pub fn tokyo_night() -> Self {
        Self {
            id: "tokyo_night".to_string(),
            name: "Tokyo Night".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [26, 27, 38],
            bg_panel: [22, 22, 30],
            bg_card: [36, 40, 59],
            border: [65, 72, 104],
            accent: [122, 162, 247],
            accent_hover: [187, 154, 247],
            text_primary: [192, 202, 245],
            text_muted: [115, 126, 166],
            success: [158, 206, 106],
            danger: [247, 118, 142],
            ansi_colors: [
                [21, 22, 30], [247, 118, 142], [158, 206, 106], [224, 175, 104],
                [122, 162, 247], [187, 154, 247], [125, 207, 255], [192, 202, 245],
                [86, 95, 137], [255, 138, 162], [178, 226, 126], [244, 195, 124],
                [142, 182, 255], [207, 174, 255], [145, 227, 255], [255, 255, 255],
            ],
        }
    }

    pub fn one_dark() -> Self {
        Self {
            id: "one_dark".to_string(),
            name: "One Dark".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [40, 44, 52],
            bg_panel: [33, 37, 43],
            bg_card: [49, 54, 63],
            border: [62, 68, 81],
            accent: [97, 175, 239],
            accent_hover: [198, 120, 221],
            text_primary: [171, 178, 191],
            text_muted: [115, 121, 132],
            success: [152, 195, 121],
            danger: [224, 108, 117],
            ansi_colors: [
                [40, 44, 52], [224, 108, 117], [152, 195, 121], [229, 192, 123],
                [97, 175, 239], [198, 120, 221], [86, 182, 194], [171, 178, 191],
                [75, 82, 99], [235, 129, 138], [173, 216, 142], [240, 203, 134],
                [118, 196, 255], [219, 141, 242], [107, 203, 215], [255, 255, 255],
            ],
        }
    }

    pub fn monokai_pro() -> Self {
        Self {
            id: "monokai_pro".to_string(),
            name: "Monokai Pro".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [45, 42, 46],
            bg_panel: [34, 31, 34],
            bg_card: [64, 61, 65],
            border: [82, 79, 83],
            accent: [255, 216, 102],
            accent_hover: [255, 97, 136],
            text_primary: [252, 252, 250],
            text_muted: [147, 146, 147],
            success: [169, 220, 103],
            danger: [255, 97, 136],
            ansi_colors: [
                [45, 42, 46], [255, 97, 136], [169, 220, 103], [255, 216, 102],
                [120, 220, 232], [171, 157, 242], [120, 220, 232], [252, 252, 250],
                [114, 112, 114], [255, 117, 156], [189, 240, 123], [255, 236, 122],
                [140, 240, 252], [191, 177, 255], [140, 240, 252], [255, 255, 255],
            ],
        }
    }

    pub fn matrix_green() -> Self {
        Self {
            id: "matrix_green".to_string(),
            name: "Matrix Green".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [10, 16, 12],
            bg_panel: [6, 10, 7],
            bg_card: [18, 30, 22],
            border: [24, 48, 30],
            accent: [34, 197, 94],
            accent_hover: [74, 222, 128],
            text_primary: [134, 239, 172],
            text_muted: [74, 140, 95],
            success: [34, 197, 94],
            danger: [239, 68, 68],
            ansi_colors: [
                [10, 16, 12], [239, 68, 68], [34, 197, 94], [234, 179, 8],
                [34, 197, 94], [74, 222, 128], [134, 239, 172], [187, 247, 208],
                [24, 48, 30], [248, 113, 113], [74, 222, 128], [250, 204, 21],
                [74, 222, 128], [134, 239, 172], [187, 247, 208], [255, 255, 255],
            ],
        }
    }

    pub fn solarized_dark() -> Self {
        Self {
            id: "solarized_dark".to_string(),
            name: "Solarized Dark".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [0, 43, 54],
            bg_panel: [0, 33, 41],
            bg_card: [7, 54, 66],
            border: [88, 110, 117],
            accent: [38, 139, 210],
            accent_hover: [42, 161, 152],
            text_primary: [147, 161, 161],
            text_muted: [101, 123, 131],
            success: [133, 153, 0],
            danger: [220, 50, 47],
            ansi_colors: [
                [7, 54, 66], [220, 50, 47], [133, 153, 0], [181, 137, 0],
                [38, 139, 210], [211, 54, 130], [42, 161, 152], [238, 232, 213],
                [0, 43, 54], [203, 75, 22], [88, 110, 117], [101, 123, 131],
                [131, 148, 150], [108, 113, 196], [147, 161, 161], [253, 246, 227],
            ],
        }
    }

    pub fn builtins() -> Vec<Self> {
        vec![
            Self::cyber_cyan(),
            Self::dracula(),
            Self::nord(),
            Self::tokyo_night(),
            Self::one_dark(),
            Self::monokai_pro(),
            Self::matrix_green(),
            Self::solarized_dark(),
        ]
    }
}

pub fn ansi_idx_to_color_extended(idx: u8) -> egui::Color32 {
    match idx {
        16..=231 => {
            let i = idx - 16;
            let r = (i / 36) * 51;
            let g = ((i / 6) % 6) * 51;
            let b = (i % 6) * 51;
            egui::Color32::from_rgb(r, g, b)
        }
        232..=255 => {
            let gray = 8 + (idx - 232) * 10;
            egui::Color32::from_rgb(gray, gray, gray)
        }
        _ => egui::Color32::WHITE,
    }
}

pub fn nav_tab_button(ui: &mut egui::Ui, text: &str, is_active: bool, theme: &ThemeConfig) -> bool {
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let padding = egui::vec2(10.0, 5.0);
    let text_color = if is_active { theme.accent_color() } else { theme.text_primary_color() };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, text_color);
    let size = egui::vec2(galley.size().x + padding.x * 2.0, 26.0);

    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if is_active {
            theme.bg_card_color()
        } else if response.hovered() {
            theme.bg_card_color().linear_multiply(0.6)
        } else {
            egui::Color32::TRANSPARENT
        };
        let stroke = if is_active {
            egui::Stroke::new(1.0_f32, theme.accent_color())
        } else {
            egui::Stroke::NONE
        };
        ui.painter().rect(rect, 4.0, bg, stroke);
        let text_pos = egui::pos2(rect.min.x + padding.x, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(text_pos, galley, egui::Color32::WHITE);
    }
    response.clicked()
}

pub fn nav_action_button(ui: &mut egui::Ui, text: &str, theme: &ThemeConfig) -> bool {
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let padding = egui::vec2(8.0, 4.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font_id, theme.text_primary_color());
    let size = egui::vec2(galley.size().x + padding.x * 2.0, 24.0);

    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if response.is_pointer_button_down_on() {
            theme.bg_panel_color()
        } else if response.hovered() {
            theme.bg_card_color()
        } else {
            theme.bg_main_color()
        };
        ui.painter().rect(rect, 4.0, bg, egui::Stroke::new(1.0_f32, theme.border_color()));
        let text_pos = egui::pos2(rect.min.x + padding.x, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(text_pos, galley, egui::Color32::WHITE);
    }
    response.clicked()
}

pub fn session_tab_chip(
    ui: &mut egui::Ui,
    id_salt: usize,
    title: &str,
    is_active: bool,
    width: f32,
    show_close: bool,
    theme: &ThemeConfig,
) -> (bool, bool) {
    let size = egui::vec2(width, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let mut close_clicked = response.middle_clicked();
    let mut clicked = response.clicked();

    if ui.is_rect_visible(rect) {
        let bg = if is_active { theme.bg_card_color() } else { theme.bg_main_color() };
        let stroke = if is_active {
            egui::Stroke::new(1.0_f32, theme.accent_color())
        } else {
            egui::Stroke::new(1.0_f32, theme.border_color())
        };
        ui.painter().rect(rect, 4.0, bg, stroke);

        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let text_color = if is_active { theme.text_primary_color() } else { theme.text_muted_color() };
        let galley = ui.painter().layout_no_wrap(title.to_string(), font_id.clone(), text_color);
        let text_pos = egui::pos2(rect.min.x + 8.0, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(text_pos, galley, egui::Color32::WHITE);

        if show_close {
            let close_rect = egui::Rect::from_center_size(
                egui::pos2(rect.right() - 12.0, rect.center().y),
                egui::vec2(16.0, 16.0),
            );
            let close_resp = ui.interact(
                close_rect,
                ui.id().with(id_salt).with("tab_close_btn"),
                egui::Sense::click(),
            );

            if close_resp.clicked() {
                close_clicked = true;
                clicked = false;
            }

            let (x_color, x_bg) = if close_resp.hovered() {
                (egui::Color32::WHITE, theme.danger_color())
            } else {
                (theme.text_muted_color(), egui::Color32::TRANSPARENT)
            };

            if x_bg != egui::Color32::TRANSPARENT {
                ui.painter().circle_filled(close_rect.center(), 7.0, x_bg);
            }

            ui.painter().text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                "×",
                font_id,
                x_color,
            );
        }
    }

    (clicked, close_clicked)
}

pub fn toggle_switch(ui: &mut egui::Ui, value: &mut bool, text: &str, theme: &ThemeConfig) -> egui::Response {
    ui.horizontal(|ui| {
        let desired_size = egui::vec2(36.0, 18.0);
        let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        if response.clicked() {
            *value = !*value;
            response.mark_changed();
        }
        if ui.is_rect_visible(rect) {
            let how_on = if *value { 1.0 } else { 0.0 };
            let bg = if *value { theme.accent_color() } else { egui::Color32::from_rgb(51, 65, 85) };
            let radius = 0.5 * rect.height();
            ui.painter().rect_filled(rect, radius, bg);
            let circle_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
            let center = egui::pos2(circle_x, rect.center().y);
            ui.painter().circle_filled(center, radius - 2.5, egui::Color32::WHITE);
        }
        if !text.is_empty() {
            ui.label(egui::RichText::new(text).color(theme.text_primary_color()));
        }
        response
    }).inner
}

pub fn setting_row_toggle(ui: &mut egui::Ui, title: &str, desc: &str, value: &mut bool, theme: &ThemeConfig) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).strong().color(theme.text_primary_color()));
            if !desc.is_empty() {
                ui.label(egui::RichText::new(desc).small().color(theme.text_muted_color()));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if toggle_switch(ui, value, "", theme).changed() {
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
