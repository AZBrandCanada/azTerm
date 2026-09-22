// src/sftp.rs
use crate::db::Database;
use crate::ssh::{SshAuthType, SshProfile, SshStore};
use crate::theme::ThemeConfig;
use eframe::egui;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Size,
    Permissions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferDirection {
    Upload,
    Download,
    RemoteToRemote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferStatus {
    Queued,
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
    pub transferred_bytes: u64,
    pub speed_bytes_sec: u64,
    pub batch_index: usize,
    pub batch_total: usize,
    pub status: TransferStatus,
    pub time: chrono::DateTime<chrono::Local>,
}

pub struct PaneBrowser {
    pub id: String,
    pub target: SftpTarget,
    pub current_path: String,
    pub entries: Vec<FileEntry>,
    pub selected_items: Vec<String>,
    pub is_loading: bool,
    pub error_message: Option<String>,
    pub last_socket_state: bool,

    pub sort_column: SortColumn,
    pub sort_direction: SortDirection,

    pub show_create_dir_modal: bool,
    pub new_dir_name: String,
    pub show_delete_confirm_modal: bool,
    pub items_to_delete: Vec<String>,

    pub show_rename_modal: bool,
    pub rename_old_name: String,
    pub rename_new_name: String,

    pub show_move_modal: bool,
    pub move_items: Vec<String>,
    pub move_dest_path: String,

    pub last_search_char: Option<char>,
    pub last_search_match_idx: usize,
    pub scroll_to_selected: bool,

    rx: Option<Receiver<Result<Vec<FileEntry>, String>>>,
}

pub fn build_ssh_base_command(profile: &SshProfile) -> Command {
    let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));
    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("BatchMode=yes");
    cmd.arg("-o").arg("ConnectTimeout=10");
    cmd.arg("-o").arg("ServerAliveInterval=10");
    cmd.arg("-o").arg("ServerAliveCountMax=2");

    if socket_path.exists() {
        cmd.arg("-o").arg(format!("ControlPath={}", socket_path.to_string_lossy()));
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
    cmd
}

pub fn calculate_local_dir_stats(path: &Path) -> (u64, usize) {
    let mut total_bytes = 0u64;
    let mut total_files = 0usize;
    let mut stack = vec![path.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_dir() {
                        stack.push(entry.path());
                    } else {
                        total_bytes += meta.len();
                        total_files += 1;
                    }
                }
            }
        }
    }
    (total_bytes, total_files)
}

pub fn calculate_remote_dir_stats(profile: &SshProfile, remote_path: &str) -> (u64, usize) {
    let safe_path = remote_path.replace('\'', "'\\''");
    let cmd_str = format!(
        "du -sb '{}' 2>/dev/null | awk '{{print $1}}'; find '{}' -type f 2>/dev/null | wc -l",
        safe_path, safe_path
    );
    let mut cmd = build_ssh_base_command(profile);
    cmd.arg(cmd_str);
    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let out = String::from_utf8_lossy(&output.stdout);
            let mut lines = out.lines();
            let size = lines.next().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
            let count = lines.next().and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0);
            return (size, count);
        }
    }
    (0, 0)
}

pub fn fit_filename_to_width(
    painter: &egui::Painter,
    name: &str,
    prefix: &str,
    font_id: &egui::FontId,
    max_width: f32,
) -> String {
    let full = format!("{}{}", prefix, name);
    let probe = painter.layout_no_wrap(full.clone(), font_id.clone(), egui::Color32::WHITE);
    if probe.size().x <= max_width {
        return full;
    }

    let (stem, ext) = if let Some(dot_pos) = name.rfind('.') {
        if dot_pos > 0 && dot_pos < name.len() - 1 && (name.len() - dot_pos) <= 10 {
            (&name[..dot_pos], &name[dot_pos..])
        } else {
            (name, "")
        }
    } else {
        (name, "")
    };

    let stem_chars: Vec<char> = stem.chars().collect();
    let mut low = 1;
    let mut high = stem_chars.len();
    let mut best = format!("{}...{}", prefix, ext);

    while low <= high {
        let mid = (low + high) / 2;
        let candidate_stem: String = stem_chars[..mid].iter().collect();
        let candidate = format!("{}{}...{}", prefix, candidate_stem, ext);
        let size_x = painter.layout_no_wrap(candidate.clone(), font_id.clone(), egui::Color32::WHITE).size().x;

        if size_x <= max_width {
            best = candidate;
            low = mid + 1;
        } else {
            if mid == 0 {
                break;
            }
            high = mid - 1;
        }
    }

    best
}

impl PaneBrowser {
    pub fn new(id: impl Into<String>, target: SftpTarget) -> Self {
        let id = id.into();
        let initial_path = match &target {
            SftpTarget::Local => std::env::var("HOME").unwrap_or_else(|_| "/".to_string()),
            SftpTarget::RemoteSsh(p) => Database::load_ssh_last_path(&p.id).unwrap_or_else(|| {
                if p.username == "root" {
                    "/root".to_string()
                } else {
                    format!("/home/{}", p.username)
                }
            }),
        };

        let mut pane = Self {
            id,
            target,
            current_path: initial_path,
            entries: Vec::new(),
            selected_items: Vec::new(),
            is_loading: false,
            error_message: None,
            last_socket_state: false,

            sort_column: SortColumn::Name,
            sort_direction: SortDirection::Ascending,

            show_create_dir_modal: false,
            new_dir_name: "new_folder".to_string(),
            show_delete_confirm_modal: false,
            items_to_delete: Vec::new(),

            show_rename_modal: false,
            rename_old_name: String::new(),
            rename_new_name: String::new(),

            show_move_modal: false,
            move_items: Vec::new(),
            move_dest_path: String::new(),

            last_search_char: None,
            last_search_match_idx: 0,
            scroll_to_selected: false,

            rx: None,
        };
        pane.refresh();
        pane
    }

    pub fn has_open_modal(&self) -> bool {
        self.show_create_dir_modal
            || self.show_delete_confirm_modal
            || self.show_rename_modal
            || self.show_move_modal
    }

    pub fn set_target(&mut self, target: SftpTarget) {
        if self.target == target {
            return;
        }
        self.target = target;
        self.current_path = match &self.target {
            SftpTarget::Local => std::env::var("HOME").unwrap_or_else(|_| "/".to_string()),
            SftpTarget::RemoteSsh(p) => Database::load_ssh_last_path(&p.id).unwrap_or_else(|| {
                if p.username == "root" {
                    "/root".to_string()
                } else {
                    format!("/home/{}", p.username)
                }
            }),
        };
        self.selected_items.clear();
        self.entries.clear();
        self.is_loading = false;
        self.rx = None;
        self.last_socket_state = false;
        if let SftpTarget::RemoteSsh(ref p) = self.target {
            let socket_path = SshStore::sockets_dir().join(format!("{}.sock", p.id));
            self.last_socket_state = socket_path.exists();
        }
        self.refresh();
    }

    pub fn set_path(&mut self, path: String) {
        let clean = path.trim().to_string();
        if clean.is_empty() {
            return;
        }
        self.current_path = clean.clone();
        if let SftpTarget::RemoteSsh(ref p) = self.target {
            Database::save_ssh_last_path(&p.id, &clean);
        }
        self.selected_items.clear();
        self.refresh();
    }

    pub fn go_home(&mut self) {
        match &self.target {
            SftpTarget::Local => {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
                self.set_path(home);
            }
            SftpTarget::RemoteSsh(p) => {
                let home = if p.username == "root" {
                    "/root".to_string()
                } else {
                    format!("/home/{}", p.username)
                };
                self.set_path(home);
            }
        }
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
                let clean = self.current_path.trim_end_matches('/');
                if clean.is_empty() || clean == "." || clean == "/" {
                    self.set_path("/".to_string());
                } else if let Some(idx) = clean.rfind('/') {
                    if idx == 0 {
                        self.set_path("/".to_string());
                    } else {
                        self.set_path(clean[..idx].to_string());
                    }
                } else {
                    self.set_path("/".to_string());
                }
            }
        }
    }

    pub fn apply_sorting(&mut self) {
        let col = self.sort_column;
        let dir = self.sort_direction;

        self.entries.sort_by(|a, b| {
            if a.is_dir != b.is_dir {
                return b.is_dir.cmp(&a.is_dir);
            }

            let ord = match col {
                SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortColumn::Size => a.size.cmp(&b.size).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                SortColumn::Permissions => a.permissions.cmp(&b.permissions).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            };

            match dir {
                SortDirection::Ascending => ord,
                SortDirection::Descending => ord.reverse(),
            }
        });
    }

    pub fn toggle_sort(&mut self, col: SortColumn) {
        if self.sort_column == col {
            self.sort_direction = match self.sort_direction {
                SortDirection::Ascending => SortDirection::Descending,
                SortDirection::Descending => SortDirection::Ascending,
            };
        } else {
            self.sort_column = col;
            self.sort_direction = SortDirection::Ascending;
        }
        self.apply_sorting();
    }

    fn run_remote_ssh_cmd(profile: &SshProfile, remote_cmd: &str) -> Result<(), String> {
        let mut cmd = build_ssh_base_command(profile);
        cmd.arg(remote_cmd);

        let output = cmd.output().map_err(|e| format!("SSH command failed: {}", e))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(err);
        }
        Ok(())
    }

    pub fn create_directory(&mut self, dir_name: String) {
        let clean_name = dir_name.trim().to_string();
        if clean_name.is_empty() {
            return;
        }

        match &self.target {
            SftpTarget::Local => {
                let target_path = PathBuf::from(&self.current_path).join(&clean_name);
                if let Err(e) = fs::create_dir_all(&target_path) {
                    self.error_message = Some(format!("Failed to create folder: {}", e));
                }
                self.refresh();
            }
            SftpTarget::RemoteSsh(profile) => {
                let target_dir = if self.current_path.ends_with('/') {
                    format!("{}{}", self.current_path, clean_name)
                } else {
                    format!("{}/{}", self.current_path, clean_name)
                };
                let profile_clone = profile.clone();
                let escaped = target_dir.replace('\'', "'\\''");
                let remote_cmd = format!("mkdir -p '{}'", escaped);

                self.is_loading = true;
                let (tx, rx): (Sender<Result<Vec<FileEntry>, String>>, Receiver<Result<Vec<FileEntry>, String>>) = channel();
                self.rx = Some(rx);
                let path_clone = self.current_path.clone();

                thread::spawn(move || {
                    let _ = Self::run_remote_ssh_cmd(&profile_clone, &remote_cmd);
                    let res = Self::fetch_remote_listing(&profile_clone, &path_clone);
                    let _ = tx.send(res);
                });
            }
        }
    }

    pub fn delete_items(&mut self, items: Vec<String>) {
        if items.is_empty() {
            return;
        }

        let active_dir = self.current_path.clone();

        match &self.target {
            SftpTarget::Local => {
                let current_dir = active_dir.clone();
                for name in &items {
                    let p = PathBuf::from(&current_dir).join(name);
                    if p.is_dir() {
                        let _ = fs::remove_dir_all(&p);
                    } else {
                        let _ = fs::remove_file(&p);
                    }
                }
                self.current_path = active_dir;
                self.refresh();
            }
            SftpTarget::RemoteSsh(profile) => {
                let current_dir = active_dir.clone();
                let profile_clone = profile.clone();

                if let SftpTarget::RemoteSsh(ref p) = self.target {
                    Database::save_ssh_last_path(&p.id, &active_dir);
                }

                self.is_loading = true;
                let (tx, rx): (Sender<Result<Vec<FileEntry>, String>>, Receiver<Result<Vec<FileEntry>, String>>) = channel();
                self.rx = Some(rx);
                let path_clone = active_dir.clone();

                thread::spawn(move || {
                    for name in items {
                        let full_path = if current_dir.ends_with('/') {
                            format!("{}{}", current_dir, name)
                        } else {
                            format!("{}/{}", current_dir, name)
                        };
                        let escaped = full_path.replace('\'', "'\\''");
                        let remote_cmd = format!("rm -rf '{}'", escaped);
                        let _ = Self::run_remote_ssh_cmd(&profile_clone, &remote_cmd);
                    }
                    let res = Self::fetch_remote_listing(&profile_clone, &path_clone);
                    let _ = tx.send(res);
                });

                self.current_path = active_dir;
            }
        }
    }

    pub fn rename_item(&mut self, old_name: String, new_name: String) {
        let clean_new = new_name.trim().to_string();
        if clean_new.is_empty() || clean_new == old_name {
            return;
        }

        match &self.target {
            SftpTarget::Local => {
                let src = PathBuf::from(&self.current_path).join(&old_name);
                let dst = PathBuf::from(&self.current_path).join(&clean_new);
                if let Err(e) = fs::rename(&src, &dst) {
                    self.error_message = Some(format!("Rename failed: {}", e));
                }
                self.refresh();
            }
            SftpTarget::RemoteSsh(profile) => {
                let src = format!("{}/{}", self.current_path.trim_end_matches('/'), old_name);
                let dst = format!("{}/{}", self.current_path.trim_end_matches('/'), clean_new);
                let profile_clone = profile.clone();
                let escaped_src = src.replace('\'', "'\\''");
                let escaped_dst = dst.replace('\'', "'\\''");
                let remote_cmd = format!("mv '{}' '{}'", escaped_src, escaped_dst);

                self.is_loading = true;
                let (tx, rx): (Sender<Result<Vec<FileEntry>, String>>, Receiver<Result<Vec<FileEntry>, String>>) = channel();
                self.rx = Some(rx);
                let path_clone = self.current_path.clone();

                thread::spawn(move || {
                    let _ = Self::run_remote_ssh_cmd(&profile_clone, &remote_cmd);
                    let res = Self::fetch_remote_listing(&profile_clone, &path_clone);
                    let _ = tx.send(res);
                });
            }
        }
    }

    pub fn move_items_to_dest(&mut self, items: Vec<String>, dest_dir: String) {
        let clean_dest = dest_dir.trim().to_string();
        if items.is_empty() || clean_dest.is_empty() || clean_dest == self.current_path {
            return;
        }

        match &self.target {
            SftpTarget::Local => {
                let current_dir = self.current_path.clone();
                let dest_path = PathBuf::from(&clean_dest);
                let _ = fs::create_dir_all(&dest_path);
                for name in &items {
                    let src = PathBuf::from(&current_dir).join(name);
                    let dst = dest_path.join(name);
                    let _ = fs::rename(&src, &dst);
                }
                self.refresh();
            }
            SftpTarget::RemoteSsh(profile) => {
                let current_dir = self.current_path.clone();
                let profile_clone = profile.clone();
                self.is_loading = true;
                let (tx, rx): (Sender<Result<Vec<FileEntry>, String>>, Receiver<Result<Vec<FileEntry>, String>>) = channel();
                self.rx = Some(rx);
                let path_clone = self.current_path.clone();

                thread::spawn(move || {
                    let safe_dest = clean_dest.trim_end_matches('/').replace('\'', "'\\''");
                    let _ = Self::run_remote_ssh_cmd(&profile_clone, &format!("mkdir -p '{}'", safe_dest));
                    for name in items {
                        let src = format!("{}/{}", current_dir.trim_end_matches('/'), name);
                        let dst = format!("{}/{}", clean_dest.trim_end_matches('/'), name);
                        let escaped_src = src.replace('\'', "'\\''");
                        let escaped_dst = dst.replace('\'', "'\\''");
                        let _ = Self::run_remote_ssh_cmd(&profile_clone, &format!("mv '{}' '{}'", escaped_src, escaped_dst));
                    }
                    let res = Self::fetch_remote_listing(&profile_clone, &path_clone);
                    let _ = tx.send(res);
                });
            }
        }
    }

    pub fn refresh(&mut self) {
        if self.is_loading {
            return;
        }

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
        Ok(list)
    }

    fn fetch_remote_listing(profile: &SshProfile, dir_path: &str) -> Result<Vec<FileEntry>, String> {
        let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));
        let mut cmd = build_ssh_base_command(profile);

        let target_dir = if dir_path.trim().is_empty() || dir_path == "." { "$PWD" } else { dir_path };
        cmd.arg(format!("ls -la \"{}\"", target_dir));

        let output = cmd.output().map_err(|e| format!("SSH command failed: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if socket_path.exists() {
                SshStore::cleanup_stale_socket(&profile.id);
            }
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
        Ok(list)
    }

    pub fn poll(&mut self) {
        if let SftpTarget::RemoteSsh(profile) = &self.target {
            let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));
            let is_connected = socket_path.exists();

            if is_connected != self.last_socket_state {
                self.last_socket_state = is_connected;
                if is_connected && !self.is_loading {
                    self.refresh();
                }
            }
        }

        if let Some(ref rx) = self.rx {
            if let Ok(res) = rx.try_recv() {
                self.is_loading = false;
                match res {
                    Ok(entries) => {
                        self.entries = entries;
                        self.apply_sorting();
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

    pub fn format_speed(bytes_per_sec: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = 1024 * KB;
        const GB: u64 = 1024 * MB;

        if bytes_per_sec >= GB {
            format!("{:.1} GB/s", bytes_per_sec as f64 / GB as f64)
        } else if bytes_per_sec >= MB {
            format!("{:.1} MB/s", bytes_per_sec as f64 / MB as f64)
        } else if bytes_per_sec >= KB {
            format!("{:.1} KB/s", bytes_per_sec as f64 / KB as f64)
        } else {
            format!("{} B/s", bytes_per_sec)
        }
    }

    pub fn format_eta(seconds: u64) -> String {
        if seconds == 0 {
            "0s".to_string()
        } else if seconds < 60 {
            format!("{}s", seconds)
        } else if seconds < 3600 {
            format!("{}m {:02}s", seconds / 60, seconds % 60)
        } else {
            format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60)
        }
    }

    pub fn render_modals(&mut self, ctx: &egui::Context, theme: &ThemeConfig) {
        if self.show_create_dir_modal {
            let mut close_modal = false;
            let mut create_dir_target: Option<String> = None;

            egui::Window::new(format!("New Directory - {}", self.id))
                .collapsible(false)
                .resizable(false)
                .default_width(320.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Enter folder name:").strong());
                        ui.add_space(4.0);
                        let resp = ui.text_edit_singleline(&mut self.new_dir_name);
                        if !resp.has_focus() {
                            resp.request_focus();
                        }

                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Create").clicked() || enter {
                                let name = self.new_dir_name.trim().to_string();
                                if !name.is_empty() {
                                    create_dir_target = Some(name);
                                }
                                close_modal = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_modal = true;
                            }
                        });
                    });
                });

            if let Some(name) = create_dir_target {
                self.create_directory(name);
                self.new_dir_name = "new_folder".to_string();
            }
            if close_modal {
                self.show_create_dir_modal = false;
            }
        }

        if self.show_rename_modal {
            let mut close_modal = false;
            let mut execute_rename: Option<(String, String)> = None;

            egui::Window::new(format!("Rename - {}", self.rename_old_name))
                .collapsible(false)
                .resizable(false)
                .default_width(320.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Enter new name:").strong());
                        ui.add_space(4.0);
                        let resp = ui.text_edit_singleline(&mut self.rename_new_name);
                        if !resp.has_focus() {
                            resp.request_focus();
                        }

                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Rename").clicked() || enter {
                                let new_name = self.rename_new_name.trim().to_string();
                                if !new_name.is_empty() {
                                    execute_rename = Some((self.rename_old_name.clone(), new_name));
                                }
                                close_modal = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_modal = true;
                            }
                        });
                    });
                });

            if let Some((old_n, new_n)) = execute_rename {
                self.rename_item(old_n, new_n);
            }
            if close_modal {
                self.show_rename_modal = false;
            }
        }

        if self.show_move_modal {
            let mut close_modal = false;
            let mut execute_move: Option<(Vec<String>, String)> = None;

            egui::Window::new("Move Item(s)")
                .collapsible(false)
                .resizable(false)
                .default_width(360.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(format!("Move {} item(s) to destination folder:", self.move_items.len())).strong());
                        ui.add_space(4.0);
                        let resp = ui.text_edit_singleline(&mut self.move_dest_path);
                        if !resp.has_focus() {
                            resp.request_focus();
                        }

                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Move").clicked() || enter {
                                let dest = self.move_dest_path.trim().to_string();
                                if !dest.is_empty() {
                                    execute_move = Some((self.move_items.clone(), dest));
                                }
                                close_modal = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_modal = true;
                            }
                        });
                    });
                });

            if let Some((items, dest)) = execute_move {
                self.move_items_to_dest(items, dest);
            }
            if close_modal {
                self.show_move_modal = false;
                self.move_items.clear();
            }
        }

        if self.show_delete_confirm_modal {
            let mut close_modal = false;
            let mut execute_delete = false;

            egui::Window::new(format!("Confirm Deletion - {}", self.id))
                .collapsible(false)
                .resizable(false)
                .default_width(380.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "Are you sure you want to permanently delete {} selected item(s)?",
                                self.items_to_delete.len()
                            ))
                            .strong()
                            .color(theme.danger_color()),
                        );
                        ui.add_space(6.0);

                        egui::ScrollArea::vertical()
                            .max_height(120.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for item in &self.items_to_delete {
                                    ui.label(egui::RichText::new(format!("- {}", item)).monospace().small());
                                }
                            });

                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("This action cannot be undone.").small().color(theme.text_muted_color()));
                        ui.add_space(10.0);

                        ui.horizontal(|ui| {
                            if ui.button(egui::RichText::new("Delete Permanently").strong().color(theme.danger_color())).clicked() {
                                execute_delete = true;
                                close_modal = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_modal = true;
                            }
                        });
                    });
                });

            if execute_delete {
                let items = self.items_to_delete.clone();
                self.delete_items(items);
                self.selected_items.clear();
            }
            if close_modal {
                self.show_delete_confirm_modal = false;
                self.items_to_delete.clear();
            }
        }
    }

    pub fn render_file_list(&mut self, ui: &mut egui::Ui, theme: &ThemeConfig) -> Option<(SshProfile, String)> {
        self.poll();
        self.render_modals(ui.ctx(), theme);

        let mut auth_request = None;
        let pane_id = self.id.clone();

        let is_typing_in_input = ui.memory(|m| m.focused().is_some()) || ui.ctx().wants_keyboard_input();
        let mut pressed_char: Option<char> = None;
        let mut delete_pressed = false;

        if !is_typing_in_input {
            ui.input(|i| {
                if i.key_pressed(egui::Key::Delete) && !self.selected_items.is_empty() {
                    delete_pressed = true;
                }

                if !i.modifiers.ctrl && !i.modifiers.alt && !i.modifiers.command {
                    for event in &i.events {
                        if let egui::Event::Text(t) = event {
                            if let Some(c) = t.chars().next() {
                                if c.is_alphanumeric() || c == '.' || c == '_' || c == '-' {
                                    pressed_char = Some(c.to_ascii_lowercase());
                                }
                            }
                        }
                    }
                }
            });
        }

        if delete_pressed {
            self.items_to_delete = self.selected_items.clone();
            self.show_delete_confirm_modal = true;
        }

        if let Some(c) = pressed_char {
            let matching_indices: Vec<usize> = self.entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.name.to_lowercase().starts_with(c))
                .map(|(idx, _)| idx)
                .collect();

            if !matching_indices.is_empty() {
                let next_match_idx = if self.last_search_char == Some(c) {
                    (self.last_search_match_idx + 1) % matching_indices.len()
                } else {
                    0
                };

                self.last_search_char = Some(c);
                self.last_search_match_idx = next_match_idx;

                let target_idx = matching_indices[next_match_idx];
                self.selected_items = vec![self.entries[target_idx].name.clone()];
                self.scroll_to_selected = true;
            }
        }

        ui.push_id(pane_id, |ui| {
            ui.vertical(|ui| {
                if let SftpTarget::RemoteSsh(ref profile) = self.target {
                    let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));
                    let needs_auth = !socket_path.exists()
                        || self.error_message.as_deref() == Some("Authentication required");

                    if needs_auth {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            theme.card_frame().show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new("[SSH Authentication Required]")
                                        .strong()
                                        .size(14.0)
                                        .color(theme.accent_color()),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(format!("Connection: {}@{}:{}", profile.username, profile.host, profile.port))
                                        .small()
                                        .color(theme.text_muted_color()),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new("Session multiplexing is inactive. Authenticate with your SSH key, password, or 2FA token to browse remote files.")
                                        .small()
                                        .color(theme.text_primary_color()),
                                );
                                ui.add_space(12.0);
                                if ui.button(egui::RichText::new("Login & Authenticate").strong().size(13.0)).clicked() {
                                    auth_request = Some((profile.clone(), self.id.clone()));
                                }
                            });
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

                let size_col_w = 75.0_f32;
                let perm_col_w = 85.0_f32;
                let right_reserved = size_col_w + perm_col_w + 14.0_f32;
                let name_col_w = (ui.available_width() - right_reserved).max(60.0);

                ui.horizontal(|ui| {
                    let name_indicator = if self.sort_column == SortColumn::Name {
                        if self.sort_direction == SortDirection::Ascending { " [^]" } else { " [v]" }
                    } else { "" };

                    let (name_hdr_rect, name_hdr_resp) = ui.allocate_exact_size(egui::vec2(name_col_w, 18.0), egui::Sense::click());
                    if ui.is_rect_visible(name_hdr_rect) {
                        if name_hdr_resp.hovered() {
                            ui.painter().rect_filled(name_hdr_rect, 2.0, theme.bg_card_color().linear_multiply(0.4));
                        }
                        let name_col = if self.sort_column == SortColumn::Name { theme.accent_color() } else { theme.text_muted_color() };
                        ui.painter().text(
                            egui::pos2(name_hdr_rect.min.x + 4.0, name_hdr_rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            format!("Name{}", name_indicator),
                            egui::FontId::proportional(12.0),
                            name_col,
                        );
                    }
                    if name_hdr_resp.clicked() {
                        self.toggle_sort(SortColumn::Name);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let perm_indicator = if self.sort_column == SortColumn::Permissions {
                            if self.sort_direction == SortDirection::Ascending { " [^]" } else { " [v]" }
                        } else { "" };
                        let perm_btn = egui::Button::new(
                            egui::RichText::new(format!("Permissions{}", perm_indicator))
                                .strong()
                                .color(if self.sort_column == SortColumn::Permissions { theme.accent_color() } else { theme.text_muted_color() })
                        ).fill(egui::Color32::TRANSPARENT).min_size(egui::vec2(perm_col_w, 18.0));

                        if ui.add(perm_btn).on_hover_text("Sort by permissions").clicked() {
                            self.toggle_sort(SortColumn::Permissions);
                        }

                        let size_indicator = if self.sort_column == SortColumn::Size {
                            if self.sort_direction == SortDirection::Ascending { " [^]" } else { " [v]" }
                        } else { "" };
                        let size_btn = egui::Button::new(
                            egui::RichText::new(format!("Size{}", size_indicator))
                                .strong()
                                .color(if self.sort_column == SortColumn::Size { theme.accent_color() } else { theme.text_muted_color() })
                        ).fill(egui::Color32::TRANSPARENT).min_size(egui::vec2(size_col_w, 18.0));

                        if ui.add(size_btn).on_hover_text("Sort by file size").clicked() {
                            self.toggle_sort(SortColumn::Size);
                        }
                    });
                });

                ui.separator();

                let mut toggled_item: Option<(String, bool)> = None;
                let mut nav_to: Option<String> = None;
                let is_ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command || i.modifiers.shift);
                let font_id = egui::TextStyle::Body.resolve(ui.style());

                egui::ScrollArea::vertical()
                    .id_source(format!("{}_file_scroll", self.id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for entry in &self.entries {
                            let prefix = if entry.is_dir { "[DIR] " } else { "[FILE] " };
                            let is_selected = self.selected_items.contains(&entry.name);

                            ui.horizontal(|ui| {
                                let (row_rect, mut row_resp) = ui.allocate_exact_size(egui::vec2(name_col_w, 19.0), egui::Sense::click_and_drag());
                                if ui.is_rect_visible(row_rect) {
                                    let bg = if is_selected {
                                        theme.bg_card_color()
                                    } else if row_resp.hovered() {
                                        theme.bg_card_color().linear_multiply(0.4)
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };

                                    if bg != egui::Color32::TRANSPARENT {
                                        ui.painter().rect_filled(row_rect, 2.0, bg);
                                    }

                                    let text_color = if is_selected {
                                        theme.accent_color()
                                    } else {
                                        theme.text_primary_color()
                                    };

                                    let display_text = fit_filename_to_width(
                                        ui.painter(),
                                        &entry.name,
                                        prefix,
                                        &font_id,
                                        name_col_w - 8.0,
                                    );

                                    let galley = ui.painter().layout_no_wrap(
                                        display_text,
                                        font_id.clone(),
                                        text_color,
                                    );
                                    let text_pos = egui::pos2(row_rect.min.x + 4.0, row_rect.center().y - galley.size().y / 2.0);
                                    ui.painter().galley(text_pos, galley, egui::Color32::WHITE);
                                }

                                row_resp = row_resp.on_hover_text(format!(
                                    "{}\nSize: {}\nPermissions: {}",
                                    entry.name,
                                    if entry.is_dir { "Directory".to_string() } else { Self::format_size(entry.size) },
                                    entry.permissions
                                ));

                                // Right-click Context Menu
                                row_resp.context_menu(|ui| {
                                    ui.set_min_width(130.0);
                                    if ui.button("Rename").clicked() {
                                        self.show_rename_modal = true;
                                        self.rename_old_name = entry.name.clone();
                                        self.rename_new_name = entry.name.clone();
                                        ui.close_menu();
                                    }
                                    if ui.button("Move to...").clicked() {
                                        self.show_move_modal = true;
                                        self.move_items = if self.selected_items.contains(&entry.name) {
                                            self.selected_items.clone()
                                        } else {
                                            vec![entry.name.clone()]
                                        };
                                        self.move_dest_path = self.current_path.clone();
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    if ui.button(egui::RichText::new("Delete").color(theme.danger_color())).clicked() {
                                        self.items_to_delete = if self.selected_items.contains(&entry.name) {
                                            self.selected_items.clone()
                                        } else {
                                            vec![entry.name.clone()]
                                        };
                                        self.show_delete_confirm_modal = true;
                                        ui.close_menu();
                                    }
                                });

                                if self.scroll_to_selected && is_selected {
                                    row_resp.scroll_to_me(Some(egui::Align::Center));
                                }

                                if row_resp.clicked() {
                                    toggled_item = Some((entry.name.clone(), is_ctrl));
                                }

                                if row_resp.secondary_clicked() {
                                    if !self.selected_items.contains(&entry.name) {
                                        self.selected_items = vec![entry.name.clone()];
                                    }
                                }

                                if row_resp.double_clicked() && entry.is_dir {
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
                                        egui::vec2(perm_col_w, 19.0),
                                        egui::Label::new(
                                            egui::RichText::new(&entry.permissions).small().color(theme.text_muted_color())
                                        ).truncate()
                                    );

                                    ui.add_sized(
                                        egui::vec2(size_col_w, 19.0),
                                        egui::Label::new(
                                            egui::RichText::new(if entry.is_dir { "-".to_string() } else { Self::format_size(entry.size) })
                                                .small()
                                                .color(theme.text_muted_color())
                                        ).truncate()
                                    );
                                });
                            });
                        }
                    });

                self.scroll_to_selected = false;

                if let Some((item, multi)) = toggled_item {
                    if multi {
                        if let Some(pos) = self.selected_items.iter().position(|x| x == &item) {
                            self.selected_items.remove(pos);
                        } else {
                            self.selected_items.push(item);
                        }
                    } else {
                        self.selected_items = vec![item];
                    }
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
            if ui.small_button("Home").on_hover_text("Go to home folder").clicked() {
                self.go_home();
            }
            if ui.small_button("Reload").on_hover_text("Reload directory").clicked() {
                self.refresh();
            }
            if ui.small_button("+ Folder").on_hover_text("Create new directory").clicked() {
                self.show_create_dir_modal = true;
                self.new_dir_name = "new_folder".to_string();
            }

            if !self.selected_items.is_empty() {
                let del_label = if self.selected_items.len() > 1 {
                    format!("Delete ({})", self.selected_items.len())
                } else {
                    "Delete".to_string()
                };
                if ui.small_button(egui::RichText::new(del_label).color(theme.danger_color())).on_hover_text("Delete selected item(s)").clicked() {
                    self.items_to_delete = self.selected_items.clone();
                    self.show_delete_confirm_modal = true;
                }
            }

            let path_w = ui.available_width().max(40.0);
            let p_edit = ui.add(
                egui::TextEdit::singleline(&mut self.current_path)
                    .desired_width(path_w)
            );
            if p_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.set_path(self.current_path.clone());
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
    pub transfer_status: Option<(String, bool, Instant)>,
    last_notified_transfer_id: Option<usize>,
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
            last_notified_transfer_id: None,
            next_transfer_id: 1,
        }
    }

    pub fn upload_selected(&mut self) {
        self.execute_transfer(true);
    }

    pub fn download_selected(&mut self) {
        self.execute_transfer(false);
    }

    pub fn execute_transfer(&mut self, left_to_right: bool) {
        let (src_pane, dest_pane) = if left_to_right {
            (&self.left_pane, &self.right_pane)
        } else {
            (&self.right_pane, &self.left_pane)
        };

        let selected = src_pane.selected_items.clone();
        if selected.is_empty() {
            return;
        }

        let src_target = src_pane.target.clone();
        let dest_target = dest_pane.target.clone();
        let src_dir = src_pane.current_path.clone();
        let dest_dir = dest_pane.current_path.clone();
        let src_entries = src_pane.entries.clone();
        let total_batch = selected.len();

        let transfer_direction = match (&src_target, &dest_target) {
            (SftpTarget::Local, SftpTarget::RemoteSsh(_)) => TransferDirection::Upload,
            (SftpTarget::RemoteSsh(_), SftpTarget::Local) => TransferDirection::Download,
            (SftpTarget::RemoteSsh(_), SftpTarget::RemoteSsh(_)) => TransferDirection::RemoteToRemote,
            (SftpTarget::Local, SftpTarget::Local) => TransferDirection::Upload,
        };

        let mut batch_records = Vec::new();
        for (idx, name) in selected.into_iter().enumerate() {
            let entry_opt = src_entries.iter().find(|e| e.name == name);
            let is_dir = entry_opt.map(|e| e.is_dir).unwrap_or(false);
            let initial_size = match &src_target {
                SftpTarget::Local => {
                    let local_path = PathBuf::from(&src_dir).join(&name);
                    if is_dir {
                        calculate_local_dir_stats(&local_path).0
                    } else {
                        fs::metadata(&local_path).map(|m| m.len()).unwrap_or(0)
                    }
                }
                SftpTarget::RemoteSsh(_) => entry_opt.map(|e| e.size).unwrap_or(0),
            };

            let from_str = match &src_target {
                SftpTarget::Local => PathBuf::from(&src_dir).join(&name).to_string_lossy().to_string(),
                SftpTarget::RemoteSsh(p) => format!("{}@{}:{}/{}", p.username, p.host, src_dir.trim_end_matches('/'), name),
            };

            let to_str = match &dest_target {
                SftpTarget::Local => PathBuf::from(&dest_dir).join(&name).to_string_lossy().to_string(),
                SftpTarget::RemoteSsh(p) => format!("{}@{}:{}/{}", p.username, p.host, dest_dir.trim_end_matches('/'), name),
            };

            let transfer_id = self.next_transfer_id;
            self.next_transfer_id += 1;

            batch_records.push((
                FileTransferRecord {
                    id: transfer_id,
                    file_name: name,
                    direction: transfer_direction.clone(),
                    from: from_str,
                    to: to_str,
                    file_size: initial_size,
                    transferred_bytes: 0,
                    speed_bytes_sec: 0,
                    batch_index: idx + 1,
                    batch_total: total_batch,
                    status: TransferStatus::Queued,
                    time: chrono::Local::now(),
                },
                is_dir,
            ));
        }

        let transfers_clone = self.transfers.clone();

        if let Ok(mut list) = transfers_clone.lock() {
            for (rec, _) in &batch_records {
                list.insert(0, rec.clone());
            }
        }

        let batch_label = match transfer_direction {
            TransferDirection::Upload => "upload",
            TransferDirection::Download => "download",
            TransferDirection::RemoteToRemote => "VPS-to-VPS",
        };

        self.transfer_status = Some((
            format!("[1/{}] Queuing {} batch...", total_batch, batch_label),
            false,
            Instant::now(),
        ));

        thread::spawn(move || {
            for (rec, is_dir) in batch_records {
                let tid = rec.id;
                let file_name = rec.file_name.clone();
                let start_t = Instant::now();

                // Compute exact recursive directory payload before transfer
                let mut actual_file_size = rec.file_size;
                if is_dir {
                    let dir_stats = match &src_target {
                        SftpTarget::Local => {
                            let local_path = PathBuf::from(&src_dir).join(&file_name);
                            calculate_local_dir_stats(&local_path)
                        }
                        SftpTarget::RemoteSsh(prof) => {
                            let remote_path = format!("{}/{}", src_dir.trim_end_matches('/'), file_name);
                            calculate_remote_dir_stats(prof, &remote_path)
                        }
                    };
                    if dir_stats.0 > 0 {
                        actual_file_size = dir_stats.0;
                    }
                }

                if let Ok(mut list) = transfers_clone.lock() {
                    if let Some(item) = list.iter_mut().find(|t| t.id == tid) {
                        item.file_size = actual_file_size;
                        item.status = TransferStatus::InProgress;
                    }
                }

                let mut transfer_success = false;
                let mut error_msg = String::new();
                let mut total_transferred = 0u64;

                match (&src_target, &dest_target) {
                    // REMOTE TO REMOTE: In-Memory Piped Proxy Streaming
                    (SftpTarget::RemoteSsh(src_prof), SftpTarget::RemoteSsh(dest_prof)) => {
                        let remote_src_path = format!("{}/{}", src_dir.trim_end_matches('/'), file_name);
                        let remote_dest_path = format!("{}/{}", dest_dir.trim_end_matches('/'), file_name);
                        let safe_src = remote_src_path.replace('\'', "'\\''");
                        let safe_dest = remote_dest_path.replace('\'', "'\\''");
                        let safe_dest_dir = dest_dir.trim_end_matches('/').replace('\'', "'\\''");

                        let mut src_cmd = build_ssh_base_command(src_prof);
                        let mut dest_cmd = build_ssh_base_command(dest_prof);

                        if !is_dir {
                            src_cmd.arg(format!("cat '{}'", safe_src));
                            dest_cmd.arg(format!("mkdir -p '{}' && cat > '{}'", safe_dest_dir, safe_dest));
                        } else {
                            let safe_parent_src = src_dir.trim_end_matches('/').replace('\'', "'\\''");
                            let safe_folder_name = file_name.replace('\'', "'\\''");
                            src_cmd.arg(format!("tar -czf - -C '{}' '{}'", safe_parent_src, safe_folder_name));
                            dest_cmd.arg(format!("mkdir -p '{}' && tar -xzf - -C '{}'", safe_dest_dir, safe_dest_dir));
                        }

                        src_cmd.stdout(Stdio::piped());
                        src_cmd.stderr(Stdio::piped());
                        dest_cmd.stdin(Stdio::piped());
                        dest_cmd.stderr(Stdio::piped());
                        dest_cmd.stdout(Stdio::null());

                        match (src_cmd.spawn(), dest_cmd.spawn()) {
                            (Ok(mut src_child), Ok(mut dest_child)) => {
                                if let (Some(mut src_out), Some(mut dest_in)) = (src_child.stdout.take(), dest_child.stdin.take()) {
                                    let mut buf = [0u8; 65536];
                                    let mut last_sample_t = Instant::now();
                                    let mut last_sample_bytes = 0u64;
                                    let mut stream_err = false;
                                    let mut filtered_speed = 0.0f64;

                                    while let Ok(n) = src_out.read(&mut buf) {
                                        if n == 0 { break; }
                                        if dest_in.write_all(&buf[..n]).is_err() {
                                            stream_err = true;
                                            break;
                                        }
                                        let _ = dest_in.flush();
                                        total_transferred += n as u64;

                                        let now = Instant::now();
                                        let sample_dt = now.duration_since(last_sample_t).as_secs_f64();
                                        if sample_dt >= 0.10 {
                                            let raw_speed = (total_transferred.saturating_sub(last_sample_bytes)) as f64 / sample_dt;
                                            filtered_speed = if filtered_speed == 0.0 { raw_speed } else { 0.35 * raw_speed + 0.65 * filtered_speed };

                                            last_sample_t = now;
                                            last_sample_bytes = total_transferred;

                                            if let Ok(mut list) = transfers_clone.lock() {
                                                if let Some(item) = list.iter_mut().find(|t| t.id == tid) {
                                                    item.transferred_bytes = total_transferred;
                                                    item.speed_bytes_sec = filtered_speed.round() as u64;
                                                }
                                            }
                                        }
                                    }

                                    drop(dest_in);
                                    drop(src_out);

                                    let src_wait = src_child.wait_with_output();
                                    let dest_wait = dest_child.wait_with_output();

                                    let src_ok = src_wait.as_ref().map(|o| o.status.success()).unwrap_or(false);
                                    let dest_ok = dest_wait.as_ref().map(|o| o.status.success()).unwrap_or(false);

                                    if !stream_err && src_ok && dest_ok {
                                        transfer_success = true;
                                    } else {
                                        if !src_ok {
                                            if let Ok(ref o) = src_wait {
                                                error_msg.push_str(&String::from_utf8_lossy(&o.stderr));
                                            }
                                        }
                                        if !dest_ok {
                                            if let Ok(ref o) = dest_wait {
                                                error_msg.push_str(&String::from_utf8_lossy(&o.stderr));
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {
                                error_msg = "Failed to spawn SSH streaming processes".to_string();
                            }
                        }
                    }

                    // LOCAL TO REMOTE: Upload
                    (SftpTarget::Local, SftpTarget::RemoteSsh(profile)) => {
                        let local_path = PathBuf::from(&src_dir).join(&file_name);
                        let remote_file = format!("{}/{}", dest_dir.trim_end_matches('/'), file_name);
                        let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));

                        if local_path.is_file() {
                            if let Ok(mut file) = fs::File::open(&local_path) {
                                let mut cmd = build_ssh_base_command(profile);
                                let safe_remote = remote_file.replace('\'', "'\\''");
                                let safe_dest_dir = dest_dir.trim_end_matches('/').replace('\'', "'\\''");
                                cmd.arg(format!("mkdir -p '{}' && cat > '{}'", safe_dest_dir, safe_remote));
                                cmd.stdin(Stdio::piped());
                                cmd.stdout(Stdio::null());
                                cmd.stderr(Stdio::piped());

                                if let Ok(mut child) = cmd.spawn() {
                                    if let Some(mut stdin) = child.stdin.take() {
                                        let mut buf = [0u8; 32768];
                                        let mut last_sample_t = Instant::now();
                                        let mut last_sample_bytes = 0u64;
                                        let mut stream_err = false;
                                        let mut filtered_speed = 0.0f64;

                                        while let Ok(n) = file.read(&mut buf) {
                                            if n == 0 { break; }
                                            if stdin.write_all(&buf[..n]).is_err() {
                                                stream_err = true;
                                                break;
                                            }
                                            let _ = stdin.flush();
                                            total_transferred += n as u64;

                                            let now = Instant::now();
                                            let sample_dt = now.duration_since(last_sample_t).as_secs_f64();
                                            if sample_dt >= 0.10 {
                                                let raw_speed = (total_transferred.saturating_sub(last_sample_bytes)) as f64 / sample_dt;
                                                filtered_speed = if filtered_speed == 0.0 { raw_speed } else { 0.35 * raw_speed + 0.65 * filtered_speed };

                                                last_sample_t = now;
                                                last_sample_bytes = total_transferred;

                                                if let Ok(mut list) = transfers_clone.lock() {
                                                    if let Some(item) = list.iter_mut().find(|t| t.id == tid) {
                                                        item.transferred_bytes = total_transferred;
                                                        item.speed_bytes_sec = filtered_speed.round() as u64;
                                                    }
                                                }
                                            }
                                        }

                                        drop(stdin);
                                        if !stream_err {
                                            if let Ok(out) = child.wait_with_output() {
                                                if out.status.success() {
                                                    transfer_success = true;
                                                } else {
                                                    error_msg = String::from_utf8_lossy(&out.stderr).to_string();
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if !transfer_success && error_msg.is_empty() {
                            let mut cmd = Command::new("scp");
                            cmd.arg("-o").arg("BatchMode=yes");
                            cmd.arg("-o").arg("ConnectTimeout=10");
                            cmd.arg("-o").arg("ServerAliveInterval=10");
                            cmd.arg("-o").arg("ServerAliveCountMax=2");

                            if socket_path.exists() {
                                cmd.arg("-o").arg(format!("ControlPath={}", socket_path.to_string_lossy()));
                            }
                            cmd.arg("-P").arg(profile.port.to_string());
                            cmd.arg("-r");

                            match &profile.auth_type {
                                SshAuthType::KeyFile(path) => {
                                    if !path.trim().is_empty() {
                                        cmd.arg("-i").arg(path.trim());
                                    }
                                }
                                SshAuthType::PastedKey { key_id } => {
                                    let key_path = SshStore::keys_dir().join(format!("{}.pem", key_id));
                                    if key_path.exists() {
                                        cmd.arg("-i").arg(key_path.to_string_lossy().to_string());
                                    }
                                }
                                SshAuthType::PasswordOrAgent => {}
                            }

                            cmd.arg(local_path.to_string_lossy().to_string());
                            cmd.arg(format!("{}@{}:{}/", profile.username, profile.host, dest_dir.trim_end_matches('/')));

                            if let Ok(out) = cmd.output() {
                                if out.status.success() {
                                    transfer_success = true;
                                    total_transferred = actual_file_size;
                                } else {
                                    error_msg = String::from_utf8_lossy(&out.stderr).to_string();
                                }
                            }
                        }
                    }

                    // REMOTE TO LOCAL: Download
                    (SftpTarget::RemoteSsh(profile), SftpTarget::Local) => {
                        let local_dir = dest_dir.clone();
                        let remote_file = format!("{}/{}", src_dir.trim_end_matches('/'), file_name);
                        let dest_local_file = PathBuf::from(&local_dir).join(&file_name);
                        let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));

                        let is_active = Arc::new(AtomicBool::new(true));
                        let is_active_clone = is_active.clone();
                        let dest_check = dest_local_file.clone();
                        let transfers_clone_monitor = transfers_clone.clone();

                        let monitor_handle = thread::spawn(move || {
                            let mut last_bytes = 0u64;
                            let mut last_time = Instant::now();

                            while is_active_clone.load(Ordering::Relaxed) {
                                thread::sleep(Duration::from_millis(250));
                                let current_bytes = if is_dir {
                                    calculate_local_dir_stats(&dest_check).0
                                } else {
                                    fs::metadata(&dest_check).map(|m| m.len()).unwrap_or(0)
                                };
                                let now = Instant::now();
                                let dt = now.duration_since(last_time).as_secs_f64().max(0.001);
                                let delta_bytes = current_bytes.saturating_sub(last_bytes);
                                let speed = (delta_bytes as f64 / dt).round() as u64;

                                last_bytes = current_bytes;
                                last_time = now;

                                if let Ok(mut list) = transfers_clone_monitor.lock() {
                                    if let Some(item) = list.iter_mut().find(|t| t.id == tid) {
                                        item.transferred_bytes = current_bytes;
                                        if speed > 0 {
                                            item.speed_bytes_sec = speed;
                                        }
                                    }
                                }
                            }
                        });

                        let mut cmd = Command::new("scp");
                        cmd.arg("-o").arg("BatchMode=yes");
                        cmd.arg("-o").arg("ConnectTimeout=10");
                        cmd.arg("-o").arg("ServerAliveInterval=10");
                        cmd.arg("-o").arg("ServerAliveCountMax=2");

                        if socket_path.exists() {
                            cmd.arg("-o").arg(format!("ControlPath={}", socket_path.to_string_lossy()));
                        }
                        cmd.arg("-P").arg(profile.port.to_string());
                        cmd.arg("-r");

                        match &profile.auth_type {
                            SshAuthType::KeyFile(path) => {
                                if !path.trim().is_empty() {
                                    cmd.arg("-i").arg(path.trim());
                                }
                            }
                            SshAuthType::PastedKey { key_id } => {
                                let key_path = SshStore::keys_dir().join(format!("{}.pem", key_id));
                                if key_path.exists() {
                                    cmd.arg("-i").arg(key_path.to_string_lossy().to_string());
                                }
                            }
                            SshAuthType::PasswordOrAgent => {}
                        }

                        cmd.arg(format!("{}@{}:{}", profile.username, profile.host, remote_file));
                        cmd.arg(&local_dir);

                        let output_res = cmd.output();
                        is_active.store(false, Ordering::Relaxed);
                        let _ = monitor_handle.join();

                        let final_bytes = if is_dir {
                            calculate_local_dir_stats(&dest_local_file).0
                        } else {
                            fs::metadata(&dest_local_file).map(|m| m.len()).unwrap_or(actual_file_size)
                        };
                        total_transferred = final_bytes;

                        if let Ok(ref out) = output_res {
                            if out.status.success() {
                                transfer_success = true;
                            } else {
                                error_msg = String::from_utf8_lossy(&out.stderr).to_string();
                            }
                        } else {
                            error_msg = "Failed to execute SCP".to_string();
                        }
                    }

                    // LOCAL TO LOCAL: File System Copy
                    (SftpTarget::Local, SftpTarget::Local) => {
                        let local_src = PathBuf::from(&src_dir).join(&file_name);
                        let local_dest = PathBuf::from(&dest_dir).join(&file_name);
                        if local_src.is_file() {
                            if let Ok(b) = fs::copy(&local_src, &local_dest) {
                                transfer_success = true;
                                total_transferred = b;
                            } else {
                                error_msg = "Local file copy failed".to_string();
                            }
                        } else {
                            error_msg = "Directory copy within local system not implemented".to_string();
                        }
                    }
                }

                let elapsed_sec = start_t.elapsed().as_secs_f64().max(0.001);
                let final_transferred = if total_transferred > 0 { total_transferred } else { actual_file_size };
                let avg_speed = (final_transferred as f64 / elapsed_sec).round() as u64;

                if let Ok(mut list) = transfers_clone.lock() {
                    if let Some(item) = list.iter_mut().find(|t| t.id == tid) {
                        item.speed_bytes_sec = avg_speed;
                        item.transferred_bytes = final_transferred;
                        if transfer_success {
                            item.status = TransferStatus::Completed;
                        } else {
                            if error_msg.trim().is_empty() {
                                error_msg = "Transfer failed".to_string();
                            }
                            item.status = TransferStatus::Failed(error_msg);
                        }
                    }
                }
            }
        });
    }

    pub fn poll_transfers(&mut self, ctx: &egui::Context) {
        if let Ok(list) = self.transfers.lock() {
            if let Some(in_progress) = list.iter().find(|t| t.status == TransferStatus::InProgress) {
                ctx.request_repaint_after(Duration::from_millis(50));

                let speed_str = PaneBrowser::format_speed(in_progress.speed_bytes_sec);
                let remaining_bytes = in_progress.file_size.saturating_sub(in_progress.transferred_bytes);
                let remaining_str = PaneBrowser::format_size(remaining_bytes);
                let transferred_str = PaneBrowser::format_size(in_progress.transferred_bytes);
                let total_str = PaneBrowser::format_size(in_progress.file_size);

                let eta_str = if in_progress.speed_bytes_sec > 0 && remaining_bytes > 0 {
                    let eta_secs = remaining_bytes / in_progress.speed_bytes_sec;
                    format!(" | ETA: {}", PaneBrowser::format_eta(eta_secs))
                } else {
                    String::new()
                };

                let progress_pct = if in_progress.file_size > 0 {
                    ((in_progress.transferred_bytes as f64 / in_progress.file_size as f64) * 100.0).clamp(0.0, 100.0).round() as u32
                } else {
                    0
                };

                let dir_label = match in_progress.direction {
                    TransferDirection::Upload => "Uploading",
                    TransferDirection::Download => "Downloading",
                    TransferDirection::RemoteToRemote => "Streaming VPS->VPS",
                };

                let progress_label = format!(
                    "[{}/{}] {} {} ({} / {}, {} left, {}%){}{}",
                    in_progress.batch_index,
                    in_progress.batch_total,
                    dir_label,
                    in_progress.file_name,
                    transferred_str,
                    total_str,
                    remaining_str,
                    progress_pct,
                    if in_progress.speed_bytes_sec > 0 { format!(" @ {}", speed_str) } else { String::new() },
                    eta_str
                );
                self.transfer_status = Some((progress_label, false, Instant::now()));
                return;
            }

            if let Some(first) = list.first() {
                match &first.status {
                    TransferStatus::Queued | TransferStatus::InProgress => {}
                    TransferStatus::Completed => {
                        if self.last_notified_transfer_id != Some(first.id) {
                            self.last_notified_transfer_id = Some(first.id);
                            let speed_info = if first.speed_bytes_sec > 0 {
                                format!(" @ {}", PaneBrowser::format_speed(first.speed_bytes_sec))
                            } else {
                                String::new()
                            };
                            self.transfer_status = Some((
                                format!("[Done {}/{}] {}{}", first.batch_index, first.batch_total, first.file_name, speed_info),
                                false,
                                Instant::now(),
                            ));
                            self.left_pane.refresh();
                            self.right_pane.refresh();
                        } else if let Some((_, _, time)) = &self.transfer_status {
                            if time.elapsed().as_secs() > 6 {
                                self.transfer_status = None;
                            }
                        }
                    }
                    TransferStatus::Failed(err) => {
                        if self.last_notified_transfer_id != Some(first.id) {
                            self.last_notified_transfer_id = Some(first.id);
                            let err_preview = if err.len() > 40 { format!("{}...", &err[..37]) } else { err.clone() };
                            self.transfer_status = Some((
                                format!("[Failed {}/{}] {}", first.batch_index, first.batch_total, err_preview.trim()),
                                true,
                                Instant::now(),
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
            .default_width(680.0)
            .default_height(380.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Transfer Activity Log").strong().size(15.0).color(theme.accent_color()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Clear Finished").clicked() {
                                if let Ok(mut list) = self.transfers.lock() {
                                    list.retain(|t| matches!(t.status, TransferStatus::InProgress | TransferStatus::Queued));
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
                                        let (dir_label, dir_color) = match t.direction {
                                            TransferDirection::Upload => ("[Upload]", theme.success_color()),
                                            TransferDirection::Download => ("[Download]", theme.accent_color()),
                                            TransferDirection::RemoteToRemote => ("[VPS -> VPS]", theme.accent_hover_color()),
                                        };

                                        ui.label(egui::RichText::new(dir_label).strong().color(dir_color));
                                        ui.label(
                                            egui::RichText::new(format!("[{}/{}]", t.batch_index, t.batch_total))
                                                .small()
                                                .color(theme.accent_color()),
                                        );
                                        ui.label(egui::RichText::new(&t.file_name).strong().color(theme.text_primary_color()));

                                        if t.file_size > 0 {
                                            ui.label(egui::RichText::new(format!("({})", PaneBrowser::format_size(t.file_size))).small().color(theme.text_muted_color()));
                                        }

                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            match &t.status {
                                                TransferStatus::Completed => {
                                                    let speed = if t.speed_bytes_sec > 0 {
                                                        format!(" ({})", PaneBrowser::format_speed(t.speed_bytes_sec))
                                                    } else {
                                                        String::new()
                                                    };
                                                    ui.label(egui::RichText::new(format!("[Completed{}]", speed)).strong().color(theme.success_color()));
                                                }
                                                TransferStatus::InProgress => {
                                                    let speed = if t.speed_bytes_sec > 0 {
                                                        format!(" ({})", PaneBrowser::format_speed(t.speed_bytes_sec))
                                                    } else {
                                                        String::new()
                                                    };
                                                    ui.label(egui::RichText::new(format!("[In Progress{}]", speed)).strong().color(theme.accent_color()));
                                                }
                                                TransferStatus::Queued => {
                                                    ui.label(egui::RichText::new("[Queued]").color(theme.text_muted_color()));
                                                }
                                                TransferStatus::Failed(err) => {
                                                    ui.label(egui::RichText::new("[Failed]").strong().color(theme.danger_color())).on_hover_text(err);
                                                }
                                            }
                                            ui.label(egui::RichText::new(t.time.format("%H:%M:%S").to_string()).small().color(theme.text_muted_color()));
                                        });
                                    });

                                    if matches!(t.status, TransferStatus::InProgress) && t.file_size > 0 {
                                        ui.add_space(4.0);
                                        let frac = (t.transferred_bytes as f32 / t.file_size as f32).clamp(0.0, 1.0);
                                        ui.add(egui::ProgressBar::new(frac).show_percentage());

                                        let remaining = t.file_size.saturating_sub(t.transferred_bytes);
                                        let eta_str = if t.speed_bytes_sec > 0 && remaining > 0 {
                                            format!(" | ETA: {}", PaneBrowser::format_eta(remaining / t.speed_bytes_sec))
                                        } else {
                                            String::new()
                                        };

                                        ui.label(
                                            egui::RichText::new(format!(
                                                "Transferred: {} / {} | Remaining: {} | Speed: {}{}",
                                                PaneBrowser::format_size(t.transferred_bytes),
                                                PaneBrowser::format_size(t.file_size),
                                                PaneBrowser::format_size(remaining),
                                                PaneBrowser::format_speed(t.speed_bytes_sec),
                                                eta_str
                                            ))
                                            .small()
                                            .color(theme.text_muted_color()),
                                        );
                                    }

                                    ui.add_space(2.0);
                                    ui.label(
                                        egui::RichText::new(format!("{}  ->  {}", t.from, t.to))
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
