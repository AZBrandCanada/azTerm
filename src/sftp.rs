// src/sftp.rs
use crate::ssh::{SshAuthType, SshProfile, SshStore};
use crate::theme::ThemeConfig;
use eframe::egui;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferStatus {
    InProgress,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct FileTransferRecord {
    pub id: usize,
    pub file_name: String,
    pub direction: TransferDirection,
    pub from: String,
    pub to: String,
    pub file_size: u64,
    pub status: TransferStatus,
    pub time: chrono::DateTime<chrono::Local>,
}

pub struct PaneBrowser {
    pub id: String,
    pub target: SftpTarget,
    pub current_path: String,
    pub entries: Vec<FileEntry>,
    pub selected_item: Option<String>,
    pub is_loading: bool,
    pub error_message: Option<String>,
    pub last_socket_state: bool,
    rx: Option<Receiver<Result<Vec<FileEntry>, String>>>,
}

impl PaneBrowser {
    pub fn new(id: impl Into<String>, target: SftpTarget) -> Self {
        let id = id.into();
        let initial_path = match &target {
            SftpTarget::Local => std::env::var("HOME").unwrap_or_else(|_| "/".to_string()),
            SftpTarget::RemoteSsh(_) => ".".to_string(),
        };

        let mut pane = Self {
            id,
            target,
            current_path: initial_path,
            entries: Vec::new(),
            selected_item: None,
            is_loading: false,
            error_message: None,
            last_socket_state: false,
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
        self.last_socket_state = false;
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

        if socket_path.exists() {
            cmd.arg("-o").arg(format!("ControlPath={}", socket_path.to_string_lossy()));
        } else {
            cmd.arg("-o").arg("ControlMaster=auto");
            cmd.arg("-o").arg(format!("ControlPath={}", socket_path.to_string_lossy()));
            cmd.arg("-o").arg("ControlPersist=10m");
            cmd.arg("-o").arg("BatchMode=yes");
            cmd.arg("-o").arg("ConnectTimeout=3");
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
            if !socket_path.exists() {
                return Err("Authentication required".to_string());
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
        if let SftpTarget::RemoteSsh(profile) = &self.target {
            let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));
            let is_connected = socket_path.exists();

            if is_connected && (!self.last_socket_state || (self.entries.is_empty() && !self.is_loading)) {
                self.last_socket_state = true;
                self.refresh();
            } else if !is_connected && self.last_socket_state {
                self.last_socket_state = false;
            }
        }

        if let Some(ref rx) = self.rx {
            if let Ok(res) = rx.try_recv() {
                self.is_loading = false;
                match res {
                    Ok(entries) => {
                        self.entries = entries;
                        self.error_message = None;
                    }
                    Err(err) => {
                        self.error_message = Some(err);
                    }
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

    pub fn render_file_list(&mut self, ui: &mut egui::Ui, theme: &ThemeConfig) -> Option<(SshProfile, String)> {
        self.poll();

        let mut auth_request = None;
        let pane_id = self.id.clone();

        ui.push_id(pane_id, |ui| {
            ui.vertical(|ui| {
                if let SftpTarget::RemoteSsh(ref profile) = self.target {
                    let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));
                    if !socket_path.exists() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(24.0);
                            ui.label(
                                egui::RichText::new("🔒 SSH Session Not Connected")
                                    .strong()
                                    .size(15.0)
                                    .color(theme.accent_color()),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("Target: {}@{}:{}", profile.username, profile.host, profile.port))
                                    .small()
                                    .color(theme.text_muted_color()),
                            );
                            ui.add_space(12.0);
                            if ui.button(egui::RichText::new("⚡ Connect & Authenticate").strong().size(13.0)).clicked() {
                                auth_request = Some((profile.clone(), self.id.clone()));
                            }
                            ui.add_space(16.0);
                        });
                        return;
                    }
                }

                if self.is_loading {
                    ui.label(egui::RichText::new("Loading directory contents...").small().color(theme.accent_color()));
                    ui.add_space(2.0);
                } else if let Some(ref err) = self.error_message {
                    ui.label(egui::RichText::new(err).small().color(theme.danger_color()));
                    ui.add_space(2.0);
                }

                // Table Header Row
                ui.horizontal(|ui| {
                    let right_space = 155.0_f32;
                    let name_w = (ui.available_width() - right_space).max(60.0);

                    ui.add_sized(egui::vec2(name_w, 18.0), egui::Label::new(
                        egui::RichText::new("Name").strong().color(theme.text_muted_color())
                    ));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_sized(egui::vec2(65.0, 18.0), egui::Label::new(
                            egui::RichText::new("Size").strong().color(theme.text_muted_color())
                        ));
                        ui.add_sized(egui::vec2(80.0, 18.0), egui::Label::new(
                            egui::RichText::new("Permissions").strong().color(theme.text_muted_color())
                        ));
                    });
                });

                ui.separator();

                let mut clicked_item: Option<String> = None;
                let mut nav_to: Option<String> = None;

                egui::ScrollArea::vertical()
                    .id_source(format!("{}_file_scroll", self.id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for entry in &self.entries {
                            // Clean text prefix instead of broken emoji glyphs
                            let prefix = if entry.is_dir { "[DIR] " } else { "[FILE] " };
                            let is_selected = self.selected_item.as_deref() == Some(&entry.name);
                            let full_label = format!("{}{}", prefix, entry.name);

                            ui.horizontal(|ui| {
                                let right_space = 155.0_f32;
                                let name_w = (ui.available_width() - right_space).max(60.0);

                                let resp = ui.add_sized(
                                    egui::vec2(name_w, 19.0),
                                    egui::SelectableLabel::new(is_selected, &full_label),
                                );

                                if resp.clicked() {
                                    clicked_item = Some(entry.name.clone());
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

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.add_sized(
                                        egui::vec2(65.0, 19.0),
                                        egui::Label::new(
                                            egui::RichText::new(if entry.is_dir { "-".to_string() } else { Self::format_size(entry.size) })
                                                .small()
                                                .color(theme.text_muted_color())
                                        ).truncate()
                                    );

                                    ui.add_sized(
                                        egui::vec2(80.0, 19.0),
                                        egui::Label::new(
                                            egui::RichText::new(&entry.permissions).small().color(theme.text_muted_color())
                                        ).truncate()
                                    );
                                });
                            });
                        }
                    });

                if let Some(item) = clicked_item {
                    self.selected_item = Some(item);
                }

                if let Some(p) = nav_to {
                    self.set_path(p);
                }
            });
        });

        auth_request
    }

    pub fn render(&mut self, ui: &mut egui::Ui, theme: &ThemeConfig) -> Option<(SshProfile, String)> {
        let mut auth_req = None;

        ui.horizontal(|ui| {
            if ui.small_button("Up").on_hover_text("Go to parent directory").clicked() {
                self.go_up();
            }
            if ui.small_button("⟳").on_hover_text("Reload directory").clicked() {
                self.refresh();
            }

            let path_w = ui.available_width().max(40.0);
            if ui.add(
                egui::TextEdit::singleline(&mut self.current_path)
                    .desired_width(path_w)
            ).lost_focus() {
                self.refresh();
            }
        });

        ui.add_space(2.0);
        ui.separator();

        if let Some(req) = self.render_file_list(ui, theme) {
            auth_req = Some(req);
        }

        auth_req
    }
}

pub struct SftpManager {
    pub left_pane: PaneBrowser,
    pub right_pane: PaneBrowser,
    pub transfers: Arc<Mutex<Vec<FileTransferRecord>>>,
    pub show_transfer_history: bool,
    pub transfer_status: Option<(String, bool, std::time::Instant)>,
    next_transfer_id: usize,
}

impl SftpManager {
    pub fn new() -> Self {
        Self {
            left_pane: PaneBrowser::new("sftp_left", SftpTarget::Local),
            right_pane: PaneBrowser::new("sftp_right", SftpTarget::Local),
            transfers: Arc::new(Mutex::new(Vec::new())),
            show_transfer_history: false,
            transfer_status: None,
            next_transfer_id: 1,
        }
    }

    pub fn upload_selected(&mut self) {
        if let (Some(selected_name), SftpTarget::Local, SftpTarget::RemoteSsh(profile)) = (
            self.left_pane.selected_item.clone(),
            &self.left_pane.target,
            &self.right_pane.target,
        ) {
            let local_path = PathBuf::from(&self.left_pane.current_path).join(&selected_name);
            let file_size = fs::metadata(&local_path).map(|m| m.len()).unwrap_or(0);
            let remote_dir = self.right_pane.current_path.clone();
            let profile_clone = profile.clone();
            let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));

            let transfer_id = self.next_transfer_id;
            self.next_transfer_id += 1;

            let dest_display = format!("{}@{}:{}/", profile.username, profile.host, remote_dir);
            let record = FileTransferRecord {
                id: transfer_id,
                file_name: selected_name.clone(),
                direction: TransferDirection::Upload,
                from: local_path.to_string_lossy().to_string(),
                to: dest_display.clone(),
                file_size,
                status: TransferStatus::InProgress,
                time: chrono::Local::now(),
            };

            if let Ok(mut list) = self.transfers.lock() {
                list.insert(0, record);
            }

            self.transfer_status = Some((
                format!("⏳ Uploading {} ({})...", selected_name, PaneBrowser::format_size(file_size)),
                false,
                std::time::Instant::now(),
            ));

            let transfers_clone = self.transfers.clone();

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
                let remote_dest = if remote_dir.ends_with('/') {
                    format!("{}@{}:{}", profile_clone.username, profile_clone.host, remote_dir)
                } else {
                    format!("{}@{}:{}/", profile_clone.username, profile_clone.host, remote_dir)
                };
                cmd.arg(remote_dest);

                let output_res = cmd.output();
                let success = match output_res {
                    Ok(ref out) if out.status.success() => true,
                    _ => false,
                };

                let error_msg = match output_res {
                    Ok(out) if !out.status.success() => {
                        let err = String::from_utf8_lossy(&out.stderr).to_string();
                        if err.trim().is_empty() {
                            format!("SCP exited with code {:?}", out.status.code())
                        } else {
                            err
                        }
                    }
                    Err(e) => format!("Failed to run scp: {}", e),
                    _ => String::new(),
                };

                if let Ok(mut list) = transfers_clone.lock() {
                    if let Some(item) = list.iter_mut().find(|t| t.id == transfer_id) {
                        if success {
                            item.status = TransferStatus::Completed;
                        } else {
                            item.status = TransferStatus::Failed(error_msg);
                        }
                    }
                }
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

            let transfer_id = self.next_transfer_id;
            self.next_transfer_id += 1;

            let source_display = format!("{}@{}:{}", profile.username, profile.host, remote_file);
            let record = FileTransferRecord {
                id: transfer_id,
                file_name: selected_name.clone(),
                direction: TransferDirection::Download,
                from: source_display.clone(),
                to: local_dir.clone(),
                file_size: 0,
                status: TransferStatus::InProgress,
                time: chrono::Local::now(),
            };

            if let Ok(mut list) = self.transfers.lock() {
                list.insert(0, record);
            }

            self.transfer_status = Some((
                format!("⏳ Downloading {}...", selected_name),
                false,
                std::time::Instant::now(),
            ));

            let transfers_clone = self.transfers.clone();

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

                let output_res = cmd.output();
                let success = match output_res {
                    Ok(ref out) if out.status.success() => true,
                    _ => false,
                };

                let error_msg = match output_res {
                    Ok(out) if !out.status.success() => {
                        let err = String::from_utf8_lossy(&out.stderr).to_string();
                        if err.trim().is_empty() {
                            format!("SCP exited with code {:?}", out.status.code())
                        } else {
                            err
                        }
                    }
                    Err(e) => format!("Failed to run scp: {}", e),
                    _ => String::new(),
                };

                if let Ok(mut list) = transfers_clone.lock() {
                    if let Some(item) = list.iter_mut().find(|t| t.id == transfer_id) {
                        if success {
                            item.status = TransferStatus::Completed;
                        } else {
                            item.status = TransferStatus::Failed(error_msg);
                        }
                    }
                }
            });
        }
    }

    pub fn poll_transfers(&mut self) {
        if let Ok(list) = self.transfers.lock() {
            if let Some(first) = list.first() {
                match &first.status {
                    TransferStatus::InProgress => {}
                    TransferStatus::Completed => {
                        if let Some((_, _, time)) = &self.transfer_status {
                            if time.elapsed().as_secs() > 6 {
                                self.transfer_status = None;
                            }
                        } else {
                            self.transfer_status = Some((
                                format!("✓ Transfer completed: {}", first.file_name),
                                false,
                                std::time::Instant::now(),
                            ));
                            self.left_pane.refresh();
                            self.right_pane.refresh();
                        }
                    }
                    TransferStatus::Failed(err) => {
                        if self.transfer_status.as_ref().map(|(_, is_err, _)| !*is_err).unwrap_or(true) {
                            let err_preview = if err.len() > 40 { format!("{}...", &err[..37]) } else { err.clone() };
                            self.transfer_status = Some((
                                format!("✕ Failed: {}", err_preview.trim()),
                                true,
                                std::time::Instant::now(),
                            ));
                        }
                    }
                }
            }
        }
    }

    pub fn render_transfer_history_window(&mut self, ctx: &egui::Context, theme: &ThemeConfig) {
        if !self.show_transfer_history {
            return;
        }

        let mut open = self.show_transfer_history;
        egui::Window::new("SFTP Transfers & History")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .default_height(340.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Transfer Activity Log").strong().size(15.0).color(theme.accent_color()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Clear Finished").clicked() {
                                if let Ok(mut list) = self.transfers.lock() {
                                    list.retain(|t| matches!(t.status, TransferStatus::InProgress));
                                }
                            }
                        });
                    });

                    ui.add_space(4.0);
                    ui.separator();

                    let transfers = self.transfers.lock().map(|l| l.clone()).unwrap_or_default();

                    if transfers.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(egui::RichText::new("No active or recent transfers").color(theme.text_muted_color()));
                        });
                        return;
                    }

                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for t in transfers {
                                theme.card_frame().show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let (dir_icon, dir_color) = match t.direction {
                                            TransferDirection::Upload => ("↑ Upload", theme.success_color()),
                                            TransferDirection::Download => ("↓ Download", theme.accent_color()),
                                        };

                                        ui.label(egui::RichText::new(dir_icon).strong().color(dir_color));
                                        ui.label(egui::RichText::new(&t.file_name).strong().color(theme.text_primary_color()));

                                        if t.file_size > 0 {
                                            ui.label(egui::RichText::new(format!("({})", PaneBrowser::format_size(t.file_size))).small().color(theme.text_muted_color()));
                                        }

                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            match &t.status {
                                                TransferStatus::Completed => {
                                                    ui.label(egui::RichText::new("● Completed").strong().color(theme.success_color()));
                                                }
                                                TransferStatus::InProgress => {
                                                    ui.label(egui::RichText::new("⏳ In Progress...").strong().color(theme.accent_color()));
                                                }
                                                TransferStatus::Failed(err) => {
                                                    ui.label(egui::RichText::new("✕ Failed").strong().color(theme.danger_color())).on_hover_text(err);
                                                }
                                            }
                                            ui.label(egui::RichText::new(t.time.format("%H:%M:%S").to_string()).small().color(theme.text_muted_color()));
                                        });
                                    });

                                    ui.add_space(2.0);
                                    ui.label(
                                        egui::RichText::new(format!("{}  ➜  {}", t.from, t.to))
                                            .small()
                                            .monospace()
                                            .color(theme.text_muted_color()),
                                    );

                                    if let TransferStatus::Failed(ref err) = t.status {
                                        ui.add_space(2.0);
                                        ui.label(egui::RichText::new(format!("Error: {}", err.trim())).small().color(theme.danger_color()));
                                    }
                                });
                                ui.add_space(4.0);
                            }
                        });
                });
            });

        self.show_transfer_history = open;
    }
}
