use crate::theme::*;
use eframe::egui;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub path: PathBuf,
}

pub struct SftpBrowser {
    pub current_path: PathBuf,
    pub entries: Vec<FileEntry>,
    pub selected_item: Option<PathBuf>,
    pub status_message: String,
}

impl SftpBrowser {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        let mut browser = Self {
            current_path: PathBuf::from(home),
            entries: Vec::new(),
            selected_item: None,
            status_message: "Ready".to_string(),
        };
        browser.refresh();
        browser
    }

    pub fn set_path(&mut self, path: PathBuf) {
        if path.exists() && path.is_dir() {
            self.current_path = path;
            self.refresh();
        }
    }

    pub fn refresh(&mut self) {
        self.entries.clear();
        if let Ok(read_dir) = fs::read_dir(&self.current_path) {
            let mut list = Vec::new();
            for item in read_dir.flatten() {
                if let Ok(meta) = item.metadata() {
                    let name = item.file_name().to_string_lossy().to_string();
                    list.push(FileEntry {
                        name,
                        is_dir: meta.is_dir(),
                        size: meta.len(),
                        path: item.path(),
                    });
                }
            }
            list.sort_by(|a, b| {
                b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            self.entries = list;
            self.status_message = format!("{} items in directory", self.entries.len());
        } else {
            self.status_message = "Failed to read directory".to_string();
        }
    }

    pub fn format_size(bytes: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = 1024 * KB;
        const GB: u64 = 1024 * MB;

        if bytes >= GB {
            format!("{:.1} GB", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.1} MB", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.1} KB", bytes as f64 / KB as f64)
        } else {
            format!("{} B", bytes)
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            // Path Navigation & Controls Header
            ui.horizontal(|ui| {
                if ui.button("Parent Dir").clicked() {
                    if let Some(parent) = self.current_path.parent() {
                        let p = parent.to_path_buf();
                        self.set_path(p);
                    }
                }
                if ui.button("Refresh").clicked() {
                    self.refresh();
                }

                let mut path_str = self.current_path.to_string_lossy().to_string();
                if ui.add(egui::TextEdit::singleline(&mut path_str).desired_width(f32::INFINITY)).lost_focus() {
                    let new_p = PathBuf::from(path_str);
                    if new_p.is_dir() {
                        self.set_path(new_p);
                    }
                }
            });

            ui.separator();

            // File Listing Table
            let mut nav_to: Option<PathBuf> = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("sftp_grid")
                    .num_columns(3)
                    .striped(true)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Name").strong().color(COLOR_TEXT_MUTED));
                        ui.label(egui::RichText::new("Type").strong().color(COLOR_TEXT_MUTED));
                        ui.label(egui::RichText::new("Size").strong().color(COLOR_TEXT_MUTED));
                        ui.end_row();

                        for entry in &self.entries {
                            let type_str = if entry.is_dir { "[DIR]" } else { "[FILE]" };
                            let is_selected = self.selected_item.as_ref() == Some(&entry.path);

                            let item_btn = ui.selectable_label(is_selected, &entry.name);
                            if item_btn.clicked() {
                                self.selected_item = Some(entry.path.clone());
                            }
                            if item_btn.double_clicked() && entry.is_dir {
                                nav_to = Some(entry.path.clone());
                            }

                            if entry.is_dir {
                                ui.label(egui::RichText::new(type_str).color(COLOR_INDIGO));
                                ui.label("-");
                            } else {
                                ui.label(egui::RichText::new(type_str).color(COLOR_TEXT_MUTED));
                                ui.label(Self::format_size(entry.size));
                            }
                            ui.end_row();
                        }
                    });
            });

            if let Some(p) = nav_to {
                self.set_path(p);
            }
        });
    }
}
