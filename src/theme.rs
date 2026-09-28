// src/theme.rs
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

    pub fn sakura_blossom() -> Self {
        Self {
            id: "sakura_blossom".to_string(),
            name: "Sakura Blossom".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [30, 22, 28],
            bg_panel: [24, 17, 22],
            bg_card: [44, 32, 41],
            border: [78, 56, 73],
            accent: [244, 114, 182],
            accent_hover: [249, 168, 212],
            text_primary: [253, 242, 248],
            text_muted: [190, 150, 175],
            success: [52, 211, 153],
            danger: [251, 113, 133],
            ansi_colors: [
                [30, 22, 28], [251, 113, 133], [52, 211, 153], [251, 191, 36],
                [167, 139, 250], [244, 114, 182], [103, 232, 249], [253, 242, 248],
                [92, 68, 87], [253, 164, 175], [110, 231, 183], [253, 224, 71],
                [196, 181, 253], [249, 168, 212], [165, 243, 252], [255, 255, 255],
            ],
        }
    }

    pub fn rose_pine() -> Self {
        Self {
            id: "rose_pine".to_string(),
            name: "Rose Pine".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [25, 23, 36],
            bg_panel: [31, 29, 46],
            bg_card: [38, 35, 58],
            border: [68, 65, 90],
            accent: [235, 188, 186],
            accent_hover: [235, 111, 146],
            text_primary: [224, 222, 244],
            text_muted: [144, 140, 170],
            success: [49, 116, 143],
            danger: [235, 111, 146],
            ansi_colors: [
                [38, 35, 58], [235, 111, 146], [49, 116, 143], [246, 193, 119],
                [156, 207, 216], [196, 167, 231], [235, 188, 186], [224, 222, 244],
                [110, 106, 134], [235, 111, 146], [49, 116, 143], [246, 193, 119],
                [156, 207, 216], [196, 167, 231], [235, 188, 186], [224, 222, 244],
            ],
        }
    }

    pub fn bubblegum_pink() -> Self {
        Self {
            id: "bubblegum_pink".to_string(),
            name: "Bubblegum Pink".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [28, 18, 25],
            bg_panel: [21, 13, 19],
            bg_card: [48, 30, 42],
            border: [90, 48, 76],
            accent: [236, 72, 153],
            accent_hover: [244, 114, 182],
            text_primary: [255, 241, 242],
            text_muted: [194, 138, 168],
            success: [74, 222, 128],
            danger: [244, 63, 94],
            ansi_colors: [
                [28, 18, 25], [244, 63, 94], [74, 222, 128], [250, 204, 21],
                [192, 132, 252], [236, 72, 153], [56, 189, 248], [255, 241, 242],
                [90, 48, 76], [251, 113, 133], [134, 239, 172], [253, 224, 71],
                [216, 180, 254], [244, 114, 182], [125, 211, 252], [255, 255, 255],
            ],
        }
    }

    pub fn lavender_mist() -> Self {
        Self {
            id: "lavender_mist".to_string(),
            name: "Lavender Mist".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [26, 24, 38],
            bg_panel: [20, 18, 30],
            bg_card: [40, 36, 56],
            border: [70, 62, 94],
            accent: [196, 167, 231],
            accent_hover: [216, 180, 254],
            text_primary: [245, 243, 255],
            text_muted: [167, 155, 194],
            success: [110, 231, 183],
            danger: [248, 113, 113],
            ansi_colors: [
                [26, 24, 38], [248, 113, 113], [110, 231, 183], [253, 224, 71],
                [167, 139, 250], [196, 167, 231], [147, 197, 253], [245, 243, 255],
                [76, 68, 102], [252, 165, 165], [167, 243, 208], [254, 240, 138],
                [196, 181, 253], [216, 180, 254], [191, 219, 254], [255, 255, 255],
            ],
        }
    }

    pub fn sunset_coral() -> Self {
        Self {
            id: "sunset_coral".to_string(),
            name: "Sunset Coral".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [28, 21, 24],
            bg_panel: [22, 16, 18],
            bg_card: [46, 32, 36],
            border: [86, 54, 60],
            accent: [251, 146, 60],
            accent_hover: [244, 114, 182],
            text_primary: [255, 247, 237],
            text_muted: [194, 148, 138],
            success: [52, 211, 153],
            danger: [239, 68, 68],
            ansi_colors: [
                [28, 21, 24], [239, 68, 68], [52, 211, 153], [251, 146, 60],
                [147, 197, 253], [244, 114, 182], [94, 234, 212], [255, 247, 237],
                [86, 54, 60], [248, 113, 113], [110, 231, 183], [253, 186, 116],
                [191, 219, 254], [249, 168, 212], [153, 246, 228], [255, 255, 255],
            ],
        }
    }

    pub fn catppuccin_frappe() -> Self {
        Self {
            id: "catppuccin_frappe".to_string(),
            name: "Catppuccin Frappe".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [48, 52, 70],
            bg_panel: [41, 44, 60],
            bg_card: [65, 69, 89],
            border: [98, 104, 128],
            accent: [234, 153, 156],
            accent_hover: [202, 158, 230],
            text_primary: [198, 208, 245],
            text_muted: [131, 139, 167],
            success: [166, 209, 137],
            danger: [231, 130, 132],
            ansi_colors: [
                [41, 44, 60], [231, 130, 132], [166, 209, 137], [229, 200, 144],
                [140, 170, 238], [202, 158, 230], [129, 200, 190], [198, 208, 245],
                [98, 104, 128], [234, 153, 156], [166, 209, 137], [229, 200, 144],
                [140, 170, 238], [244, 184, 228], [129, 200, 190], [255, 255, 255],
            ],
        }
    }

    pub fn emerald_forest() -> Self {
        Self {
            id: "emerald_forest".to_string(),
            name: "Emerald Forest".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [15, 24, 20],
            bg_panel: [10, 18, 15],
            bg_card: [24, 40, 32],
            border: [38, 66, 52],
            accent: [16, 185, 129],
            accent_hover: [52, 211, 153],
            text_primary: [236, 253, 245],
            text_muted: [110, 160, 135],
            success: [16, 185, 129],
            danger: [239, 68, 68],
            ansi_colors: [
                [15, 24, 20], [239, 68, 68], [16, 185, 129], [245, 158, 11],
                [59, 130, 246], [168, 85, 247], [20, 184, 166], [236, 253, 245],
                [38, 66, 52], [248, 113, 113], [52, 211, 153], [251, 191, 36],
                [96, 165, 250], [192, 132, 252], [45, 212, 191], [255, 255, 255],
            ],
        }
    }

    pub fn amber_glow() -> Self {
        Self {
            id: "amber_glow".to_string(),
            name: "Amber Glow".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [24, 20, 15],
            bg_panel: [18, 14, 10],
            bg_card: [40, 32, 24],
            border: [70, 55, 38],
            accent: [245, 158, 11],
            accent_hover: [251, 191, 36],
            text_primary: [254, 243, 199],
            text_muted: [180, 150, 115],
            success: [34, 197, 94],
            danger: [239, 68, 68],
            ansi_colors: [
                [24, 20, 15], [239, 68, 68], [34, 197, 94], [245, 158, 11],
                [99, 102, 241], [217, 70, 239], [20, 184, 166], [254, 243, 199],
                [70, 55, 38], [248, 113, 113], [74, 222, 128], [251, 191, 36],
                [129, 140, 248], [232, 121, 249], [45, 212, 191], [255, 255, 255],
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


    pub fn synthwave() -> Self {
        Self {
            id: "synthwave".to_string(),
            name: "Synthwave".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [10, 4, 20],
            bg_panel: [18, 8, 32],
            bg_card: [32, 14, 54],
            border: [80, 36, 120],
            accent: [255, 60, 180],
            accent_hover: [255, 120, 220],
            text_primary: [245, 230, 255],
            text_muted: [165, 135, 205],
            success: [0, 255, 200],
            danger: [255, 30, 90],
            ansi_colors: [
                [10, 4, 20], [255, 30, 90], [0, 255, 200], [255, 220, 80],
                [120, 140, 255], [255, 60, 180], [0, 220, 255], [245, 230, 255],
                [70, 50, 100], [255, 100, 150], [120, 255, 220], [255, 240, 140],
                [160, 180, 255], [255, 140, 220], [100, 240, 255], [255, 255, 255],
            ],
        }
    }

    pub fn toxic_lime() -> Self {
        Self {
            id: "toxic_lime".to_string(),
            name: "Toxic Lime".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [5, 10, 5],
            bg_panel: [8, 18, 8],
            bg_card: [16, 32, 16],
            border: [46, 90, 46],
            accent: [170, 255, 0],
            accent_hover: [200, 255, 60],
            text_primary: [230, 255, 220],
            text_muted: [135, 185, 135],
            success: [100, 255, 100],
            danger: [255, 60, 60],
            ansi_colors: [
                [5, 10, 5], [255, 60, 60], [170, 255, 0], [255, 220, 40],
                [80, 200, 120], [200, 255, 80], [0, 240, 180], [230, 255, 220],
                [46, 90, 46], [255, 110, 110], [200, 255, 80], [255, 240, 120],
                [120, 220, 140], [230, 255, 130], [80, 255, 210], [255, 255, 255],
            ],
        }
    }

    pub fn hot_magenta() -> Self {
        Self {
            id: "hot_magenta".to_string(),
            name: "Hot Magenta".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [15, 0, 15],
            bg_panel: [25, 0, 25],
            bg_card: [42, 6, 42],
            border: [100, 24, 100],
            accent: [255, 0, 200],
            accent_hover: [255, 90, 225],
            text_primary: [255, 230, 255],
            text_muted: [195, 135, 195],
            success: [0, 255, 150],
            danger: [255, 20, 80],
            ansi_colors: [
                [15, 0, 15], [255, 20, 80], [0, 255, 150], [255, 220, 60],
                [90, 130, 255], [255, 0, 200], [0, 230, 255], [255, 230, 255],
                [75, 20, 75], [255, 90, 140], [90, 255, 190], [255, 240, 130],
                [140, 180, 255], [255, 100, 220], [90, 240, 255], [255, 255, 255],
            ],
        }
    }

    pub fn infrared() -> Self {
        Self {
            id: "infrared".to_string(),
            name: "Infrared".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [12, 4, 4],
            bg_panel: [22, 8, 8],
            bg_card: [38, 14, 14],
            border: [90, 30, 24],
            accent: [255, 80, 20],
            accent_hover: [255, 130, 60],
            text_primary: [255, 235, 220],
            text_muted: [205, 135, 115],
            success: [60, 255, 120],
            danger: [255, 20, 20],
            ansi_colors: [
                [12, 4, 4], [255, 20, 20], [60, 255, 120], [255, 200, 40],
                [80, 140, 255], [255, 60, 180], [0, 220, 220], [255, 235, 220],
                [90, 30, 24], [255, 90, 90], [120, 255, 160], [255, 225, 120],
                [130, 170, 255], [255, 120, 210], [100, 240, 240], [255, 255, 255],
            ],
        }
    }

    pub fn ultraviolet() -> Self {
        Self {
            id: "ultraviolet".to_string(),
            name: "Ultraviolet".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [8, 4, 20],
            bg_panel: [14, 8, 32],
            bg_card: [26, 14, 52],
            border: [66, 44, 116],
            accent: [150, 60, 255],
            accent_hover: [180, 100, 255],
            text_primary: [235, 225, 255],
            text_muted: [155, 135, 205],
            success: [80, 255, 180],
            danger: [255, 60, 120],
            ansi_colors: [
                [8, 4, 20], [255, 60, 120], [80, 255, 180], [255, 220, 90],
                [110, 140, 255], [200, 90, 255], [0, 220, 255], [235, 225, 255],
                [66, 44, 116], [255, 110, 160], [130, 255, 210], [255, 240, 140],
                [160, 185, 255], [220, 140, 255], [100, 240, 255], [255, 255, 255],
            ],
        }
    }

    pub fn voltage() -> Self {
        Self {
            id: "voltage".to_string(),
            name: "Voltage".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [0, 5, 15],
            bg_panel: [0, 12, 25],
            bg_card: [5, 26, 48],
            border: [24, 68, 110],
            accent: [0, 180, 255],
            accent_hover: [60, 210, 255],
            text_primary: [220, 245, 255],
            text_muted: [130, 180, 215],
            success: [0, 255, 180],
            danger: [255, 60, 90],
            ansi_colors: [
                [0, 5, 15], [255, 60, 90], [0, 255, 180], [255, 220, 80],
                [0, 180, 255], [200, 90, 255], [0, 230, 230], [220, 245, 255],
                [24, 68, 110], [255, 110, 140], [90, 255, 210], [255, 240, 140],
                [100, 210, 255], [220, 140, 255], [90, 240, 240], [255, 255, 255],
            ],
        }
    }


    pub fn sunshine() -> Self {
        Self {
            id: "sunshine".to_string(),
            name: "Sunshine".to_string(),
            is_builtin: true,
            opacity: 1.0,
            // Panel/card hierarchy: pure white terminal, warm off-white
            // panels, light grey cards. Keeps the same 3-step depth the
            // dark themes use, just inverted.
            bg_main: [255, 255, 255],
            bg_panel: [250, 247, 238],
            bg_card: [238, 235, 224],
            border: [198, 192, 176],

            // Bright amber/gold — reads as "yellow" but dark enough that
            // selected-item text (accent used as text color) is still
            // legible against bg_card.
            accent: [240, 180, 0],
            accent_hover: [255, 205, 60],

            // Near-black primary text, warm grey muted.
            text_primary: [28, 26, 20],
            text_muted: [110, 104, 92],

            success: [26, 140, 56],
            danger: [200, 42, 42],

            // ANSI palette tuned for white backgrounds. Every "dark"
            // colour is a mid-tone so it stays readable when an app
            // writes black-on-white or uses low-intensity attributes.
            ansi_colors: [
                [40, 40, 40],       // Black (readable on white)
                [200, 40, 40],      // Red
                [26, 140, 56],      // Green
                [170, 130, 0],      // Yellow (dark amber — must read on white)
                [30, 90, 200],      // Blue
                [160, 40, 160],     // Magenta
                [0, 130, 150],      // Cyan
                [90, 90, 90],       // White (grey — must read on white)

                [110, 110, 110],    // Bright Black (grey)
                [230, 60, 60],      // Bright Red
                [40, 170, 70],      // Bright Green
                [210, 160, 0],      // Bright Yellow
                [50, 120, 235],     // Bright Blue
                [200, 60, 200],     // Bright Magenta
                [0, 160, 190],      // Bright Cyan
                [20, 20, 20],       // Bright White (near-black — readable on white)
            ],
        }
    }


    pub fn sky_blue() -> Self {
        Self {
            id: "sky_blue".to_string(),
            name: "Sky Blue".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [255, 255, 255],
            bg_panel: [240, 246, 252],
            bg_card: [222, 234, 246],
            border: [178, 198, 220],
            accent: [30, 110, 210],
            accent_hover: [70, 145, 235],
            text_primary: [18, 26, 40],
            text_muted: [95, 115, 140],
            success: [26, 140, 56],
            danger: [200, 42, 42],
            ansi_colors: [
                [40, 48, 60], [200, 40, 40], [26, 140, 56], [170, 130, 0],
                [30, 110, 210], [160, 40, 160], [0, 130, 150], [90, 100, 115],
                [110, 120, 135], [230, 60, 60], [40, 170, 70], [210, 160, 0],
                [50, 140, 240], [200, 60, 200], [0, 160, 190], [20, 26, 34],
            ],
        }
    }

    pub fn mint_fresh() -> Self {
        Self {
            id: "mint_fresh".to_string(),
            name: "Mint Fresh".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [255, 255, 255],
            bg_panel: [238, 248, 240],
            bg_card: [218, 238, 222],
            border: [172, 202, 180],
            accent: [22, 140, 90],
            accent_hover: [55, 175, 120],
            text_primary: [20, 34, 26],
            text_muted: [92, 122, 100],
            success: [22, 140, 90],
            danger: [200, 42, 42],
            ansi_colors: [
                [38, 46, 40], [200, 40, 40], [22, 140, 90], [170, 130, 0],
                [30, 90, 200], [160, 40, 160], [0, 130, 150], [90, 100, 95],
                [110, 125, 115], [230, 60, 60], [40, 170, 110], [210, 160, 0],
                [50, 120, 235], [200, 60, 200], [0, 160, 190], [20, 30, 24],
            ],
        }
    }

    pub fn sakura_light() -> Self {
        Self {
            id: "sakura_light".to_string(),
            name: "Sakura Light".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [255, 255, 255],
            bg_panel: [252, 240, 246],
            bg_card: [246, 222, 234],
            border: [216, 178, 198],
            accent: [210, 60, 130],
            accent_hover: [235, 100, 165],
            text_primary: [40, 22, 30],
            text_muted: [135, 95, 112],
            success: [26, 140, 56],
            danger: [200, 42, 42],
            ansi_colors: [
                [50, 38, 45], [200, 40, 40], [26, 140, 56], [170, 130, 0],
                [30, 90, 200], [210, 60, 130], [0, 130, 150], [100, 85, 95],
                [125, 105, 115], [230, 60, 60], [40, 170, 70], [210, 160, 0],
                [50, 120, 235], [235, 100, 165], [0, 160, 190], [30, 22, 28],
            ],
        }
    }

    pub fn lavender_light() -> Self {
        Self {
            id: "lavender_light".to_string(),
            name: "Lavender Light".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [255, 255, 255],
            bg_panel: [242, 240, 252],
            bg_card: [226, 222, 246],
            border: [188, 182, 218],
            accent: [110, 60, 210],
            accent_hover: [145, 100, 235],
            text_primary: [28, 24, 44],
            text_muted: [108, 100, 140],
            success: [26, 140, 56],
            danger: [200, 42, 42],
            ansi_colors: [
                [45, 40, 60], [200, 40, 40], [26, 140, 56], [170, 130, 0],
                [110, 60, 210], [160, 40, 160], [0, 130, 150], [95, 90, 115],
                [118, 112, 138], [230, 60, 60], [40, 170, 70], [210, 160, 0],
                [145, 100, 235], [200, 60, 200], [0, 160, 190], [22, 20, 34],
            ],
        }
    }

    pub fn peach_cream() -> Self {
        Self {
            id: "peach_cream".to_string(),
            name: "Peach Cream".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [255, 253, 248],
            bg_panel: [252, 240, 226],
            bg_card: [248, 224, 200],
            border: [218, 188, 160],
            accent: [220, 95, 40],
            accent_hover: [240, 130, 75],
            text_primary: [44, 28, 18],
            text_muted: [140, 108, 80],
            success: [26, 140, 56],
            danger: [200, 42, 42],
            ansi_colors: [
                [52, 40, 30], [200, 40, 40], [26, 140, 56], [170, 130, 0],
                [30, 90, 200], [160, 40, 160], [0, 130, 150], [110, 95, 80],
                [135, 118, 100], [230, 60, 60], [40, 170, 70], [210, 160, 0],
                [50, 120, 235], [200, 60, 200], [0, 160, 190], [32, 22, 14],
            ],
        }
    }

    pub fn paper_slate() -> Self {
        Self {
            id: "paper_slate".to_string(),
            name: "Paper Slate".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [252, 252, 253],
            bg_panel: [240, 240, 244],
            bg_card: [222, 224, 230],
            border: [180, 184, 194],
            accent: [55, 60, 75],
            accent_hover: [90, 96, 115],
            text_primary: [24, 26, 34],
            text_muted: [108, 112, 125],
            success: [26, 140, 56],
            danger: [200, 42, 42],
            ansi_colors: [
                [42, 44, 52], [200, 40, 40], [26, 140, 56], [170, 130, 0],
                [30, 90, 200], [160, 40, 160], [0, 130, 150], [90, 92, 100],
                [112, 116, 128], [230, 60, 60], [40, 170, 70], [210, 160, 0],
                [50, 120, 235], [200, 60, 200], [0, 160, 190], [20, 22, 28],
            ],
        }
    }

    pub fn ocean_foam() -> Self {
        Self {
            id: "ocean_foam".to_string(),
            name: "Ocean Foam".to_string(),
            is_builtin: true,
            opacity: 1.0,
            bg_main: [253, 255, 255],
            bg_panel: [232, 246, 248],
            bg_card: [206, 232, 236],
            border: [160, 198, 204],
            accent: [0, 130, 145],
            accent_hover: [30, 165, 180],
            text_primary: [18, 34, 38],
            text_muted: [82, 122, 130],
            success: [26, 140, 56],
            danger: [200, 42, 42],
            ansi_colors: [
                [34, 44, 48], [200, 40, 40], [26, 140, 56], [170, 130, 0],
                [30, 90, 200], [160, 40, 160], [0, 130, 145], [86, 100, 106],
                [104, 122, 130], [230, 60, 60], [40, 170, 70], [210, 160, 0],
                [50, 120, 235], [200, 60, 200], [30, 165, 180], [16, 26, 30],
            ],
        }
    }

    pub fn builtins() -> Vec<Self> {
        vec![
            Self::cyber_cyan(),
            Self::sakura_blossom(),
            Self::rose_pine(),
            Self::bubblegum_pink(),
            Self::lavender_mist(),
            Self::sunset_coral(),
            Self::catppuccin_frappe(),
            Self::emerald_forest(),
            Self::amber_glow(),
            Self::dracula(),
            Self::nord(),
            Self::tokyo_night(),
            Self::one_dark(),
            Self::monokai_pro(),
            Self::matrix_green(),
            Self::solarized_dark(),
            Self::synthwave(),
            Self::toxic_lime(),
            Self::hot_magenta(),
            Self::infrared(),
            Self::ultraviolet(),
            Self::voltage(),
            Self::sunshine(),
            Self::sky_blue(),
            Self::mint_fresh(),
            Self::sakura_light(),
            Self::lavender_light(),
            Self::peach_cream(),
            Self::paper_slate(),
            Self::ocean_foam(),
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
    let btn = if is_active {
        let fill = theme.accent_color();
        crate::modern::Button3D::new(text)
            .small()
            .fill(fill)
            .edge(crate::modern::darken(fill, 55))
            .text_color(egui::Color32::from_rgb(15, 23, 42))
    } else {
        crate::modern::Button3D::new(text)
            .small()
            .fill(theme.bg_card_color())
            .edge(crate::modern::darken(theme.bg_card_color(), 40))
            .text_color(theme.text_primary_color())
    };
    btn.show(ui, theme).clicked()
}

pub fn nav_action_button(ui: &mut egui::Ui, text: &str, theme: &ThemeConfig) -> bool {
    // Accent face + darker-accent depth band — same recipe as the active
    // nav tab and the toolbar buttons, so every "primary action" button
    // in the app shares one visual language.
    let fill = theme.accent_color();
    crate::modern::Button3D::new(text)
        .small()
        .fill(fill)
        .edge(crate::modern::darken(fill, 55))
        .text_color(egui::Color32::from_rgb(15, 23, 42))
        .show(ui, theme)
        .clicked()
}

pub const SESSION_TAB_HEIGHT: f32 = 30.0;
pub const SESSION_TAB_DEPTH: f32 = 3.0;
pub const SESSION_TAB_LIFT: f32 = 2.0;
pub const SESSION_TAB_TOTAL_H: f32 = SESSION_TAB_HEIGHT + SESSION_TAB_DEPTH + SESSION_TAB_LIFT;

pub fn session_tab_chip(
    ui: &mut egui::Ui,
    id_salt: usize,
    title: &str,
    is_active: bool,
    width: f32,
    show_close: bool,
    theme: &ThemeConfig,
) -> (bool, bool) {
    use crate::modern::{accent_glow, darken, lighten};

    let height = SESSION_TAB_HEIGHT;
    let depth = SESSION_TAB_DEPTH;
    let lift = if is_active { 0.0 } else { SESSION_TAB_LIFT };

    let total_size = egui::vec2(width, height + depth + lift);
    let (outer_rect, response) = ui.allocate_exact_size(total_size, egui::Sense::click());

    let mut close_clicked = response.middle_clicked();
    let mut clicked = response.clicked();

    let hovered = response.hovered();
    let pressed = response.is_pointer_button_down_on();

    // Idle tabs sit at +lift so the depth band below is visible. Hover
    // lifts them the rest of the way; pressing pushes them down onto the
    // shadow (classic 3D button behaviour).
    let base_y = outer_rect.min.y + lift;
    let body_top = if pressed {
        base_y + depth
    } else if hovered && !is_active {
        base_y - lift * 0.6
    } else {
        base_y
    };

    let body_rect = egui::Rect::from_min_size(
        egui::pos2(outer_rect.min.x, body_top),
        egui::vec2(width, height),
    );
    let shadow_rect = egui::Rect::from_min_size(
        egui::pos2(body_rect.min.x, body_rect.min.y + depth),
        egui::vec2(width, height),
    );

    if !ui.is_rect_visible(outer_rect) {
        return (clicked, close_clicked);
    }

    // Colour palette per state.
    let (body_fill, edge_fill, text_color) = if is_active {
        (
            lighten(theme.bg_card_color(), 10),
            theme.accent_color(),
            theme.text_primary_color(),
        )
    } else if hovered {
        (
            lighten(theme.bg_main_color(), 24),
            darken(theme.bg_main_color(), 60),
            theme.text_primary_color(),
        )
    } else {
        (
            theme.bg_main_color(),
            darken(theme.bg_main_color(), 55),
            theme.text_muted_color(),
        )
    };

    // Border colour: full accent when active, translucent accent on hover,
    // neutral otherwise.
    let border_color = if is_active {
        theme.accent_color()
    } else if hovered {
        egui::Color32::from_rgba_unmultiplied(
            theme.accent_color().r(),
            theme.accent_color().g(),
            theme.accent_color().b(),
            150,
        )
    } else {
        theme.border_color()
    };

    // Soft accent halo behind the active tab.
    if is_active {
        accent_glow(ui.painter(), body_rect, theme.accent_color(), 8.0, 0.55, 1.0);
    }

    // Depth band beneath the body — this is the visual that sells 3D.
    if !pressed {
        ui.painter()
            .rect_filled(shadow_rect, egui::Rounding::same(8.0), edge_fill);
    }

    // Body.
    ui.painter()
        .rect_filled(body_rect, egui::Rounding::same(7.0), body_fill);

    // Border ring.
    ui.painter().rect_stroke(
        body_rect,
        egui::Rounding::same(7.0),
        egui::Stroke::new(if is_active { 1.6_f32 } else { 1.0_f32 }, border_color),
    );

    // Accent stripe across the top of the active tab — reads like a
    // bookmark tab marker.
    if is_active {
        let stripe_rect = egui::Rect::from_min_size(
            egui::pos2(body_rect.min.x + 3.0, body_rect.min.y + 1.5),
            egui::vec2(body_rect.width() - 6.0, 2.5),
        );
        ui.painter()
            .rect_filled(stripe_rect, egui::Rounding::same(2.0), theme.accent_color());
    }

    // Title.
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_string(), font_id.clone(), text_color);
    let text_pos = egui::pos2(
        body_rect.min.x + 10.0,
        body_rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(text_pos, galley, egui::Color32::WHITE);

    // Close button — always-visible soft circle, bright red on hover.
    if show_close {
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(body_rect.right() - 14.0, body_rect.center().y),
            egui::vec2(18.0, 18.0),
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

        let (x_color, circle_fill) = if close_resp.hovered() {
            (egui::Color32::WHITE, theme.danger_color())
        } else {
            let alpha = if is_active { 60 } else { 35 };
            (
                text_color,
                egui::Color32::from_rgba_unmultiplied(
                    theme.text_muted_color().r(),
                    theme.text_muted_color().g(),
                    theme.text_muted_color().b(),
                    alpha,
                ),
            )
        };

        ui.painter()
            .circle_filled(close_rect.center(), 8.5, circle_fill);

        ui.painter().text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            egui::FontId::proportional(13.0),
            x_color,
        );
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
