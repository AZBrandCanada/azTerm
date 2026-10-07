// src/fonts.rs
//
// System font discovery + egui application.
//
// The terminal font (Monospace family) is EMBEDDED into the binary
// via include_bytes!. This guarantees Braille glyphs (U+2800–U+28FF),
// box-drawing, and Nerd Font icons render identically no matter how
// AZTerm is installed (AppImage, .deb, Flatpak, cargo install,
// manual build). No system font dependency, no fontconfig lookup,
// no "works on my machine".
//
// The UI font (Proportional family) is still user-selectable and
// loaded from disk — it only affects chrome (labels, buttons, nav),
// never terminal cells, so a missing/broken UI font is a cosmetic
// issue, not a correctness one.

use eframe::egui;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

// -- Embedded terminal font ------------------------------------------------
//
// JetBrainsMono Nerd Font includes the full Braille block, all
// Powerline/box-drawing glyphs, and the Nerd Font icon set.
//
// To update: drop the new .ttf into src/fonts/ and rebuild.
const TERMINAL_FONT_REGULAR: &[u8] =
    include_bytes!("fonts/JetBrainsMonoNerdFont-Regular.ttf");

const EMBEDDED_FONT_KEY_REGULAR: &str = "azterm_terminal_font";

#[derive(Debug, Clone)]
pub struct FontEntry {
    pub family: String,
    pub path: PathBuf,
    pub is_mono: bool,
}

pub fn preview_family_name(idx: usize) -> String {
    format!("azterm_font_preview_{}", idx)
}

pub fn is_loadable_font(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    matches!(
        &data[..4],
        [0x00, 0x01, 0x00, 0x00] | [b't', b'r', b'u', b'e'] | [b'O', b'T', b'T', b'O']
    )
}

fn file_looks_loadable(path: &Path) -> bool {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; 4];
    f.read_exact(&mut buf).is_ok() && is_loadable_font(&buf)
}

pub fn list_system_fonts() -> Vec<FontEntry> {
    let mono_families = mono_family_set();
    let mut entries = list_via_fc_list(&mono_families).unwrap_or_default();
    if entries.is_empty() {
        entries = list_via_dir_scan();
    }
    entries.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()));
    entries.dedup_by(|a, b| a.family.eq_ignore_ascii_case(&b.family));
    entries
}

fn mono_family_set() -> HashSet<String> {
    let mut set = HashSet::new();
    let out = match Command::new("fc-list")
        .args([":spacing=90", "-f", "%{family[0]}\n"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return set,
    };
    if !out.status.success() {
        return set;
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let name = line.trim().to_lowercase();
        if !name.is_empty() {
            set.insert(name);
        }
    }
    set
}

fn list_via_fc_list(mono_families: &HashSet<String>) -> Option<Vec<FontEntry>> {
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
        if !path.exists() || !file_looks_loadable(&path) {
            continue;
        }
        let is_mono = mono_families.contains(&family.to_lowercase());
        list.push(FontEntry {
            family,
            path,
            is_mono,
        });
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
                if !file_looks_loadable(&path) {
                    continue;
                }
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
                if family.is_empty() {
                    continue;
                }
                let is_mono = family.to_lowercase().contains("mono");
                out.push(FontEntry {
                    family,
                    path,
                    is_mono,
                });
            }
        }
    }
}

pub fn apply_to_egui(
    ctx: &egui::Context,
    ui_font_path: &str,
    previews: &[(usize, PathBuf)],
) -> Vec<(usize, PathBuf)> {
    let mut fonts = egui::FontDefinitions::default();

    // ---- UI font (Proportional family) --------------------------------
    if !ui_font_path.trim().is_empty() {
        try_register_font(
            &mut fonts,
            "azterm_ui_font",
            ui_font_path,
            Some(egui::FontFamily::Proportional),
        );
    }

    // ---- Terminal font (Monospace family) -----------------------------
    // EMBEDDED. Always available, always has Braille.
    fonts.font_data.insert(
        EMBEDDED_FONT_KEY_REGULAR.to_owned(),
        egui::FontData::from_static(TERMINAL_FONT_REGULAR),
    );
    if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        list.insert(0, EMBEDDED_FONT_KEY_REGULAR.to_owned());
    }

    // ---- Bounded preview families -------------------------------------
    let mut actually_loaded: Vec<(usize, PathBuf)> = Vec::new();
    for (idx, path) in previews {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        if !is_loadable_font(&data) {
            continue;
        }
        let data_key = format!("preview_data_{}", idx);
        fonts
            .font_data
            .insert(data_key.clone(), egui::FontData::from_owned(data));
        fonts.families.insert(
            egui::FontFamily::Name(preview_family_name(*idx).into()),
            vec![data_key],
        );
        actually_loaded.push((*idx, path.clone()));
    }

    ctx.set_fonts(fonts);
    actually_loaded
}

fn try_register_font(
    fonts: &mut egui::FontDefinitions,
    key: &str,
    path: &str,
    family: Option<egui::FontFamily>,
) -> bool {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[fonts] failed to read {}: {}", path, e);
            return false;
        }
    };
    if !is_loadable_font(&data) {
        eprintln!(
            "[fonts] skipping {}: not a valid TTF/OTF (probably .ttc, .woff, or corrupt)",
            path
        );
        return false;
    }
    fonts
        .font_data
        .insert(key.to_string(), egui::FontData::from_owned(data));
    if let Some(fam) = family {
        if let Some(list) = fonts.families.get_mut(&fam) {
            list.insert(0, key.to_string());
        }
    }
    true
}
