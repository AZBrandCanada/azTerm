// src/fonts.rs
//
// System font discovery + egui application. Used by the Settings
// "Terminal Font" picker so users can pick any installed typeface
// for terminal sessions (UI keeps egui's default font).

use eframe::egui;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct FontEntry {
    pub family: String,
    pub path: PathBuf,
}

/// Enumerate installed fonts. Prefers `fc-list` (fontconfig), which
/// gives canonical family names; falls back to walking the standard
/// font directories if fontconfig isn't available.
pub fn list_system_fonts() -> Vec<FontEntry> {
    let mut entries = list_via_fc_list().unwrap_or_default();
    if entries.is_empty() {
        entries = list_via_dir_scan();
    }
    entries.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()));
    // Deduplicate by family — keep the first path per family.
    entries.dedup_by(|a, b| a.family.eq_ignore_ascii_case(&b.family));
    entries
}

fn list_via_fc_list() -> Option<Vec<FontEntry>> {
    let out = Command::new("fc-list")
        .args(["-f", "%{family[0]}\t%{file}\n"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut list = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '\t');
        let family = match parts.next() {
            Some(s) => s.trim().to_string(),
            None => continue,
        };
        let path = match parts.next() {
            Some(s) => s.trim().to_string(),
            None => continue,
        };
        if family.is_empty() || path.is_empty() {
            continue;
        }
        let path = PathBuf::from(path);
        if !path.exists() {
            continue;
        }
        list.push(FontEntry { family, path });
    }
    Some(list)
}

fn list_via_dir_scan() -> Vec<FontEntry> {
    let mut roots: Vec<PathBuf> = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    if let Ok(home) = std::env::var("HOME") {
        roots.push(PathBuf::from(&home).join(".local/share/fonts"));
        roots.push(PathBuf::from(&home).join(".fonts"));
    }
    let mut list = Vec::new();
    for root in roots {
        walk_fonts(&root, &mut list);
    }
    list
}

fn walk_fonts(dir: &Path, out: &mut Vec<FontEntry>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_fonts(&path, out);
        } else {
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf") {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let family = stem
                    .replace('-', " ")
                    .replace('_', " ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                if !family.is_empty() {
                    out.push(FontEntry { family, path });
                }
            }
        }
    }
}

/// Apply the UI and terminal font selections to egui.
///
/// `ui_font_path`      — absolute path to a font file to use for the
///                       Proportional family (buttons, labels, nav,
///                       settings, everything except terminal cells).
///                       Empty string = keep egui's built-in default.
/// `terminal_font_path`— absolute path to a font file to use for the
///                       Monospace family (terminal content, code
///                       blocks). Empty string = use AZTerm's built-in
///                       fallback list.
pub fn apply_to_egui(
    ctx: &egui::Context,
    ui_font_path: &str,
    terminal_font_path: &str,
    preview_fonts: &[FontEntry],
) {
    let mut fonts = egui::FontDefinitions::default();

    // ---- UI font: Proportional family ---------------------------------
    if !ui_font_path.trim().is_empty() {
        match std::fs::read(ui_font_path) {
            Ok(data) => {
                fonts.font_data.insert(
                    "azterm_ui_font".to_string(),
                    egui::FontData::from_owned(data),
                );
                if let Some(prop) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                    prop.insert(0, "azterm_ui_font".to_string());
                }
            }
            Err(e) => eprintln!("[fonts] failed to read ui font {}: {}", ui_font_path, e),
        }
    }

    // ---- Terminal font: Monospace family ------------------------------
    let resolved_mono: Option<String> = if !terminal_font_path.trim().is_empty() {
        Some(terminal_font_path.to_string())
    } else {
        default_mono_font_path()
    };

    if let Some(path) = resolved_mono {
        match std::fs::read(&path) {
            Ok(data) => {
                fonts.font_data.insert(
                    "azterm_terminal_font".to_string(),
                    egui::FontData::from_owned(data),
                );
                if let Some(mono) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                    mono.insert(0, "azterm_terminal_font".to_string());
                }
            }
            Err(e) => eprintln!("[fonts] failed to read terminal font {}: {}", path, e),
        }
    }

    // ---- Preview families -------------------------------------------
    // Register each installed font under a named family so the combo
    // boxes can render each dropdown item in its own typeface. Capped
    // at PREVIEW_LIMIT fonts to avoid pathological startup cost on
    // systems with truly huge font libraries.
    for (idx, entry) in preview_fonts.iter().enumerate().take(PREVIEW_LIMIT) {
        let data = match std::fs::read(&entry.path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let data_key = format!("preview_data_{}", idx);
        fonts
            .font_data
            .insert(data_key.clone(), egui::FontData::from_owned(data));
        fonts.families.insert(
            egui::FontFamily::Name(preview_family_name(idx).into()),
            vec![data_key],
        );
    }

    ctx.set_fonts(fonts);
}

/// Maximum number of installed fonts that get a preview family.
/// Dropdowns must check `idx < PREVIEW_LIMIT` before requesting a
/// preview font, otherwise egui panics on an unbound family.
pub const PREVIEW_LIMIT: usize = 400;

/// Family name used for the Nth installed font in preview mode. The
/// font combo boxes use this to render each item in the actual font
/// the item names. Only valid for `idx < PREVIEW_LIMIT`.
pub fn preview_family_name(idx: usize) -> String {
    format!("azterm_font_preview_{}", idx)
}

/// True if the given font index has a registered preview family.
pub fn has_preview(idx: usize) -> bool {
    idx < PREVIEW_LIMIT
}

/// Family name used for a preview by font path. Returns None if the
/// path isn't currently registered.
pub fn preview_family_for_path(fonts: &[FontEntry], path: &str) -> Option<String> {
    fonts
        .iter()
        .position(|f| f.path.to_string_lossy() == path)
        .map(preview_family_name)
}

/// The built-in fallback list used when the user hasn't picked a font.
fn default_mono_font_path() -> Option<String> {
    let candidates = [
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
        "/usr/share/fonts/google-noto/NotoSansMono-Regular.ttf",
        "/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf",
        "/usr/share/fonts/TTF/JetBrainsMonoNerdFontMono-Regular.ttf",
        "/usr/share/fonts/TTF/SymbolsNerdFontMono-Regular.ttf",
        "/usr/share/fonts/TTF/SymbolsNerdFont-Regular.ttf",
        "/usr/share/fonts/nerd-fonts/SymbolsNerdFontMono-Regular.ttf",
        "/usr/share/fonts/truetype/nerd-fonts/SymbolsNerdFontMono-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/liberation-mono/LiberationMono-Regular.ttf",
        "/usr/share/fonts/TTF/LiberationMono-Regular.ttf",
    ];
    for p in candidates {
        if Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    None
}
