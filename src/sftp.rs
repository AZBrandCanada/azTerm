use crate::ssh::{SshAuthType, SshProfile, SshStore};
use crate::theme::*;
use eframe::egui;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SftpTarget {
    Local,
    RemoteSsh(SshProfile),
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub permissions: String,
}

pub struct PaneBrowser {
    pub target: SftpTarget,
    pub current_path: String,
    pub entries: Vec<FileEntry>,
    pub selected_item: Option<String>,
    pub is_loading: bool,
    pub error_message: Option<String>,
    rx: Option<Receiver<Result<Vec<FileEntry>, String>>>,
}

impl PaneBrowser {
    pub fn new(target: SftpTarget) -> Self {
        let initial_path = match &target {
            SftpTarget::Local => std::env::var("HOME").unwrap_or_else(|_| "/".to_string()),
            SftpTarget::RemoteSsh(_) => ".".to_string(),
        };

        let mut pane = Self {
            target,
            current_path: initial_path,
            entries: Vec::new(),
            selected_item: None,
            is_loading: false,
            error_message: None,
            rx: None,
        };
        pane.refresh();
        pane
    }

    pub fn set_target(&mut self, target: SftpTarget) {
        if self.target == target {
            return;
        }
        self.target = target;
        self.current_path = match &self.target {
            SftpTarget::Local => std::env::var("HOME").unwrap_or_else(|_| "/".to_string()),
            SftpTarget::RemoteSsh(_) => ".".to_string(),
        };
        self.selected_item = None;
        self.refresh();
    }

    pub fn set_path(&mut self, path: String) {
        self.current_path = path;
        self.selected_item = None;
        self.refresh();
    }

    pub fn go_up(&mut self) {
        match &self.target {
            SftpTarget::Local => {
                let p = PathBuf::from(&self.current_path);
                if let Some(parent) = p.parent() {
                    self.set_path(parent.to_string_lossy().to_string());
                }
            }
            SftpTarget::RemoteSsh(_) => {
                if self.current_path == "." || self.current_path == "/" {
                    self.set_path("/".to_string());
                } else if let Some(idx) = self.current_path.rfind('/') {
                    if idx == 0 {
                        self.set_path("/".to_string());
                    } else {
                        self.set_path(self.current_path[..idx].to_string());
                    }
                } else {
                    self.set_path("..".to_string());
                }
            }
        }
    }

    pub fn refresh(&mut self) {
        self.is_loading = true;
        self.error_message = None;
        let (tx, rx): (Sender<Result<Vec<FileEntry>, String>>, Receiver<Result<Vec<FileEntry>, String>>) = channel();
        self.rx = Some(rx);

        let target_clone = self.target.clone();
        let path_clone = self.current_path.clone();

        thread::spawn(move || {
            let res = match target_clone {
                SftpTarget::Local => Self::fetch_local_listing(&path_clone),
                SftpTarget::RemoteSsh(profile) => Self::fetch_remote_listing(&profile, &path_clone),
            };
            let _ = tx.send(res);
        });
    }

    fn fetch_local_listing(dir_path: &str) -> Result<Vec<FileEntry>, String> {
        let p = Path::new(dir_path);
        if !p.exists() {
            return Err("Directory does not exist".to_string());
        }
        let read_dir = fs::read_dir(p).map_err(|e| e.to_string())?;
        let mut list = Vec::new();

        for item in read_dir.flatten() {
            if let Ok(meta) = item.metadata() {
                let name = item.file_name().to_string_lossy().to_string();
                list.push(FileEntry {
                    name,
                    is_dir: meta.is_dir(),
                    size: meta.len(),
                    permissions: if meta.is_dir() { "drwxr-xr-x".to_string() } else { "-rw-r--r--".to_string() },
                });
            }
        }
        list.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(list)
    }

    fn fetch_remote_listing(profile: &SshProfile, dir_path: &str) -> Result<Vec<FileEntry>, String> {
        let socket_dir = SshStore::sockets_dir();
        let socket_path = socket_dir.join(format!("{}.sock", profile.id));

        let mut cmd = Command::new("ssh");

        // Reuse live authenticated socket from terminal session
        if socket_path.exists() {
            cmd.arg("-o").arg(format!("ControlPath={}", socket_path.to_string_lossy()));
        } else {
            cmd.arg("-o").arg("ControlMaster=auto");
            cmd.arg("-o").arg(format!("ControlPath={}", socket_path.to_string_lossy()));
            cmd.arg("-o").arg("ControlPersist=10m");
            cmd.arg("-o").arg("BatchMode=yes");
            cmd.arg("-o").arg("ConnectTimeout=5");
        }

        cmd.arg("-p").arg(profile.port.to_string());

        match &profile.auth_type {
            SshAuthType::KeyFile(path) => {
                if !path.trim().is_empty() {
                    SshStore::ensure_secure_permissions(path);
                    cmd.arg("-i").arg(path.trim());
                }
            }
            SshAuthType::PastedKey { key_id } => {
                let key_path = SshStore::keys_dir().join(format!("{}.pem", key_id));
                if key_path.exists() {
                    SshStore::ensure_secure_permissions(&key_path.to_string_lossy());
                    cmd.arg("-i").arg(key_path.to_string_lossy().to_string());
                }
            }
            SshAuthType::PasswordOrAgent => {}
        }

        cmd.arg(format!("{}@{}", profile.username, profile.host));
        let target_dir = if dir_path.trim().is_empty() || dir_path == "." { "$PWD" } else { dir_path };
        cmd.arg(format!("ls -la \"{}\"", target_dir));

        let output = cmd.output().map_err(|e| format!("SSH command failed: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if stderr.contains("Permission denied") && !socket_path.exists() {
                return Err("Please connect to the SSH Terminal first to authenticate session.".to_string());
            }
            return Err(stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut list = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 9 {
                let perms = parts[0];
                let is_dir = perms.starts_with('d');
                let size = parts[4].parse::<u64>().unwrap_or(0);
                let name = parts[8..].join(" ");

                if name == "." || name == ".." {
                    continue;
                }

                list.push(FileEntry {
                    name,
                    is_dir,
                    size,
                    permissions: perms.to_string(),
                });
            }
        }
        list.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(list)
    }

    pub fn poll(&mut self) {
        if let Some(ref rx) = self.rx {
            if let Ok(res) = rx.try_recv() {
                self.is_loading = false;
                match res {
                    Ok(entries) => self.entries = entries,
                    Err(err) => self.error_message = Some(err),
                }
            }
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
        self.poll();

        ui.vertical(|ui| {
            // Path Navigation Header
            ui.horizontal(|ui| {
                if ui.button("Up").clicked() {
                    self.go_up();
                }
                if ui.button("Refresh").clicked() {
                    self.refresh();
                }

                if ui.add(egui::TextEdit::singleline(&mut self.current_path).desired_width(f32::INFINITY)).lost_focus() {
                    self.refresh();
                }
            });

            ui.add_space(4.0);

            if self.is_loading {
                ui.label(egui::RichText::new("Loading directory contents...").small().color(COLOR_ACCENT));
            } else if let Some(ref err) = self.error_message {
                ui.label(egui::RichText::new(format!("Error: {}", err)).small().color(COLOR_DANGER));
            }

            ui.separator();

            // File Table Listing
            let mut nav_to: Option<String> = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new(ui.id().with("pane_grid"))
                        .num_columns(3)
                        .striped(true)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("Name").strong().color(COLOR_TEXT_MUTED));
                            ui.label(egui::RichText::new("Permissions").strong().color(COLOR_TEXT_MUTED));
                            ui.label(egui::RichText::new("Size").strong().color(COLOR_TEXT_MUTED));
                            ui.end_row();

                            for entry in &self.entries {
                                let type_prefix = if entry.is_dir { "[DIR] " } else { "[FILE] " };
                                let is_selected = self.selected_item.as_deref() == Some(&entry.name);
                                let label_text = format!("{}{}", type_prefix, entry.name);

                                let resp = ui.selectable_label(is_selected, label_text);
                                if resp.clicked() {
                                    self.selected_item = Some(entry.name.clone());
                                }
                                if resp.double_clicked() && entry.is_dir {
                                    let next_path = match &self.target {
                                        SftpTarget::Local => {
                                            PathBuf::from(&self.current_path).join(&entry.name).to_string_lossy().to_string()
                                        }
                                        SftpTarget::RemoteSsh(_) => {
                                            if self.current_path.ends_with('/') {
                                                format!("{}{}", self.current_path, entry.name)
                                            } else {
                                                format!("{}/{}", self.current_path, entry.name)
                                            }
                                        }
                                    };
                                    nav_to = Some(next_path);
                                }

                                ui.label(egui::RichText::new(&entry.permissions).small().color(COLOR_TEXT_MUTED));
                                if entry.is_dir {
                                    ui.label("-");
                                } else {
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

pub struct SftpManager {
    pub left_pane: PaneBrowser,
    pub right_pane: PaneBrowser,
    pub transfer_status: Option<String>,
}

impl SftpManager {
    pub fn new() -> Self {
        Self {
            left_pane: PaneBrowser::new(SftpTarget::Local),
            right_pane: PaneBrowser::new(SftpTarget::Local),
            transfer_status: None,
        }
    }

    pub fn upload_selected(&mut self) {
        if let (Some(selected_name), SftpTarget::Local, SftpTarget::RemoteSsh(profile)) = (
            self.left_pane.selected_item.clone(),
            &self.left_pane.target,
            &self.right_pane.target,
        ) {
            let local_path = PathBuf::from(&self.left_pane.current_path).join(&selected_name);
            let remote_dir = self.right_pane.current_path.clone();
            let profile_clone = profile.clone();
            let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));

            self.transfer_status = Some(format!("Uploading {} to remote...", selected_name));

            thread::spawn(move || {
                let mut cmd = Command::new("scp");
                if socket_path.exists() {
                    cmd.arg("-o").arg(format!("ControlPath={}", socket_path.to_string_lossy()));
                }
                cmd.arg("-P").arg(profile_clone.port.to_string());
                cmd.arg("-r");

                match &profile_clone.auth_type {
                    SshAuthType::KeyFile(path) => {
                        if !path.trim().is_empty() {
                            SshStore::ensure_secure_permissions(path);
                            cmd.arg("-i").arg(path.trim());
                        }
                    }
                    SshAuthType::PastedKey { key_id } => {
                        let key_path = SshStore::keys_dir().join(format!("{}.pem", key_id));
                        if key_path.exists() {
                            SshStore::ensure_secure_permissions(&key_path.to_string_lossy());
                            cmd.arg("-i").arg(key_path.to_string_lossy().to_string());
                        }
                    }
                    SshAuthType::PasswordOrAgent => {}
                }

                cmd.arg(local_path.to_string_lossy().to_string());
                cmd.arg(format!("{}@{}:{}", profile_clone.username, profile_clone.host, remote_dir));
                let _ = cmd.output();
            });
        }
    }

    pub fn download_selected(&mut self) {
        if let (Some(selected_name), SftpTarget::RemoteSsh(profile), SftpTarget::Local) = (
            self.right_pane.selected_item.clone(),
            &self.right_pane.target,
            &self.left_pane.target,
        ) {
            let remote_file = if self.right_pane.current_path.ends_with('/') {
                format!("{}{}", self.right_pane.current_path, selected_name)
            } else {
                format!("{}/{}", self.right_pane.current_path, selected_name)
            };
            let local_dir = self.left_pane.current_path.clone();
            let profile_clone = profile.clone();
            let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));

            self.transfer_status = Some(format!("Downloading {} to local...", selected_name));

            thread::spawn(move || {
                let mut cmd = Command::new("scp");
                if socket_path.exists() {
                    cmd.arg("-o").arg(format!("ControlPath={}", socket_path.to_string_lossy()));
                }
                cmd.arg("-P").arg(profile_clone.port.to_string());
                cmd.arg("-r");

                match &profile_clone.auth_type {
                    SshAuthType::KeyFile(path) => {
                        if !path.trim().is_empty() {
                            SshStore::ensure_secure_permissions(path);
                            cmd.arg("-i").arg(path.trim());
                        }
                    }
                    SshAuthType::PastedKey { key_id } => {
                        let key_path = SshStore::keys_dir().join(format!("{}.pem", key_id));
                        if key_path.exists() {
                            SshStore::ensure_secure_permissions(&key_path.to_string_lossy());
                            cmd.arg("-i").arg(key_path.to_string_lossy().to_string());
                        }
                    }
                    SshAuthType::PasswordOrAgent => {}
                }

                cmd.arg(format!("{}@{}:{}", profile_clone.username, profile_clone.host, remote_file));
                cmd.arg(local_dir);
                let _ = cmd.output();
            });
        }
    }
}
