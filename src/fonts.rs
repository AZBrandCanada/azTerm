// src/fonts.rs
//
// System font discovery + egui application.
//
// Terminal cells are laid out on a fixed grid: every glyph occupies one
// cell of identical width. That only works if the chosen font is
// monospaced. We detect monospace-ness via fontconfig's `spacing`
// property (90 = mono, 100 = proportional) and expose it as
// `FontEntry::is_mono` so the Terminal Font dropdown can filter
// accordingly. Selecting a proportional font there would produce
// "text only fills half the pane" artifacts.
//
// Additionally, epaint PANICS on any file it can't parse as plain
// TrueType/OpenType, so every byte buffer handed to `FontData::from_owned`
// is validated against a magic-byte whitelist first.

use eframe::egui;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct FontEntry {
    pub family: String,
    pub path: PathBuf,
    /// True if fontconfig reports `spacing=90` (monospaced). Terminal
    /// sessions require a monospaced font; the Terminal Font dropdown
    /// uses this flag to hide non-mono entries.
    pub is_mono: bool,
}

/// Whitelist check: does this byte buffer start with a magic number
/// that epaint's font parser accepts?
///
/// Accepts:
///   * `00 01 00 00` — TrueType
///   * `74 72 75 65` — "true" — Apple TrueType
///   * `4F 54 54 4F` — "OTTO" — OpenType/CFF
///
/// Rejects (these panic inside epaint if passed through):
///   * `74 74 63 66` — "ttcf" — TrueType Collection (.ttc)
///   * `77 4F 46 46` — "wOFF" — WOFF
///   * `77 4F 46 32` — "wOF2" — WOFF2
///   * Anything else (truncated, corrupt, etc.)
pub fn is_loadable_font(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    matches!(
        &data[..4],
        [0x00, 0x01, 0x00, 0x00] | [b't', b'r', b'u', b'e'] | [b'O', b'T', b'T', b'O']
    )
}

/// Read the first 4 bytes of a file and check the font magic number.
fn file_looks_loadable(path: &Path) -> bool {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; 4];
    f.read_exact(&mut buf).is_ok() && is_loadable_font(&buf)
}

/// Enumerate installed fonts. Prefers fontconfig (`fc-list`); falls back
/// to walking the standard font directories if fontconfig is missing.
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

/// Ask fontconfig for the set of monospaced family names. `spacing=90`
/// is the standard fontconfig value for "monospace".
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
    // Directory scan can't reliably determine monospace-ness without
    // parsing font files. Heuristic: anything with "mono" in the name
    // is treated as monospace, everything else as proportional.
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

/// Apply UI + terminal font selections to egui.
///
/// Every load goes through `is_loadable_font()`. A bad file is skipped
/// silently; the built-in fallback is used instead of panicking.
pub fn apply_to_egui(ctx: &egui::Context, ui_font_path: &str) {
    let mut fonts = egui::FontDefinitions::default();

    // ---- UI font (Proportional family) --------------------------------
    // Users can still pick this one; it only affects labels, buttons,
    // and window chrome, never terminal cells.
    if !ui_font_path.trim().is_empty() {
        try_register_font(
            &mut fonts,
            "azterm_ui_font",
            ui_font_path,
            Some(egui::FontFamily::Proportional),
        );
    }

    // ---- Terminal font (Monospace family) -----------------------------
    // Fixed to the built-in fallback chain. No preview font families
    // are registered any more either — loading every installed font
    // into egui's atlas made the Settings tab crawl on systems with
    // hundreds of fonts.
    if let Some(fb) = default_mono_font_path() {
        let _ = try_register_font(
            &mut fonts,
            "azterm_terminal_font",
            &fb,
            Some(egui::FontFamily::Monospace),
        );
    }

    ctx.set_fonts(fonts);
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
        if Path::new(p).exists() && file_looks_loadable(Path::new(p)) {
            return Some(p.to_string());
        }
    }
    None
}