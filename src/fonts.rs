// src/fonts.rs
//
// System font discovery + egui application. Used by the Settings
// "Terminal Font" picker so users can pick any installed typeface
// for terminal sessions (UI keeps egui's default font).
//
// Important: epaint PANICS on any file it can't parse as plain
// TrueType/OpenType. It does not return an Err. So every byte buffer
// we hand to `FontData::from_owned` must be validated against the
// magic-byte whitelist first. `is_loadable_font()` below is that gate.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct FontEntry {
    pub family: String,
    pub path: PathBuf,
}

/// Maximum number of installed fonts that get a preview family.
/// Dropdowns must check `idx < PREVIEW_LIMIT` before requesting a
/// preview font, otherwise egui panics on an unbound family.
pub const PREVIEW_LIMIT: usize = 400;

/// True if the given font index has a registered preview family.
pub fn has_preview(idx: usize) -> bool {
    idx < PREVIEW_LIMIT
}

/// Family name used for the Nth installed font in preview mode. The
/// font combo boxes use this to render each item in the actual font
/// the item names. Only valid for `idx < PREVIEW_LIMIT`.
pub fn preview_family_name(idx: usize) -> String {
    format!("azterm_font_preview_{}", idx)
}

/// Family name used for a preview by font path. Returns None if the
/// path isn't currently registered.
#[allow(dead_code)]
pub fn preview_family_for_path(fonts: &[FontEntry], path: &str) -> Option<String> {
    fonts
        .iter()
        .position(|f| f.path.to_string_lossy() == path)
        .map(preview_family_name)
}

/// Whitelist check: does this byte buffer start with a magic number
/// that epaint's font parser accepts?
///
/// Accepts:
///   * `00 01 00 00` — TrueType outlines
///   * `74 72 75 65` — "true" — Apple-flavoured TrueType
///   * `4F 54 54 4F` — "OTTO"  — OpenType with CFF outlines
///
/// Rejects (these cause epaint to panic if passed through):
///   * `74 74 63 66` — "ttcf" — TrueType Collection (.ttc)
///   * `77 4F 46 46` — "wOFF" — WOFF web font
///   * `77 4F 46 32` — "wOF2" — WOFF2 web font
///   * Anything else, including truncated / corrupt files.
pub fn is_loadable_font(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    let tag = &data[..4];
    matches!(
        tag,
        [0x00, 0x01, 0x00, 0x00]
            | [b't', b'r', b'u', b'e']
            | [b'O', b'T', b'T', b'O']
    )
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
        // Skip files whose magic bytes indicate a format epaint can't
        // parse. Cheap check: read only the first 4 bytes.
        if !file_looks_loadable(&path) {
            continue;
        }
        list.push(FontEntry { family, path });
    }
    Some(list)
}

/// Read the first 4 bytes of a file and check the font magic number.
/// Returns false for unreadable files or formats epaint rejects.
fn file_looks_loadable(path: &Path) -> bool {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; 4];
    match f.read_exact(&mut buf) {
        Ok(_) => is_loadable_font(&buf),
        Err(_) => false,
    }
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
/// `preview_fonts`     — installed fonts to expose as preview families.
///
/// Every load goes through `is_loadable_font()` — a bad file is skipped
/// silently rather than panicking. If the terminal font is bad, we fall
/// back to `default_mono_font_path()`. If that's also missing, egui's
/// built-in monospace is used.
pub fn apply_to_egui(
    ctx: &egui::Context,
    ui_font_path: &str,
    terminal_font_path: &str,
    preview_fonts: &[FontEntry],
) {
    let mut fonts = egui::FontDefinitions::default();

    // ---- UI font: Proportional family ---------------------------------
    if !ui_font_path.trim().is_empty() {
        try_register_font(
            &mut fonts,
            "azterm_ui_font",
            ui_font_path,
            Some(egui::FontFamily::Proportional),
        );
    }

    // ---- Terminal font: Monospace family ------------------------------
    // Try the user's pick first; if that fails, fall back to the built-in
    // candidate list; if that fails too, egui's default Monospace stays.
    let primary_mono = if terminal_font_path.trim().is_empty() {
        default_mono_font_path()
    } else {
        Some(terminal_font_path.to_string())
    };

    let registered = match &primary_mono {
        Some(p) => try_register_font(
            &mut fonts,
            "azterm_terminal_font",
            p,
            Some(egui::FontFamily::Monospace),
        ),
        None => false,
    };

    if !registered {
        if let Some(fallback) = default_mono_font_path() {
            let _ = try_register_font(
                &mut fonts,
                "azterm_terminal_font",
                &fallback,
                Some(egui::FontFamily::Monospace),
            );
        }
    }

    // ---- Preview families --------------------------------------------
    // Register installed fonts under named families so dropdown items
    // can render in their own typeface. Capped at PREVIEW_LIMIT.
    for (idx, entry) in preview_fonts.iter().enumerate().take(PREVIEW_LIMIT) {
        let data = match std::fs::read(&entry.path) {
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
            egui::FontFamily::Name(preview_family_name(idx).into()),
            vec![data_key],
        );
    }

    ctx.set_fonts(fonts);
}

/// Attempt to load `path` and register it as `key`. When `family` is
/// Some, the new font is prepended to that family so it takes priority
/// over the built-in default. Returns true on success, false if the
/// file couldn't be read or failed the magic-byte check.
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
        if Path::new(p).exists() && file_looks_loadable(Path::new(p)) {
            return Some(p.to_string());
        }
    }
    None
}