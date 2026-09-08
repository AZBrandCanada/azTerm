mod db;
mod settings;
mod sftp;
mod ssh;
mod terminal;
mod theme;
mod tiling;
mod ui;

use db::{Database, SavedSessionState};
use eframe::egui;
use portable_pty::CommandBuilder;
use settings::AppSettings;
use sftp::{SftpManager, SftpTarget};
use ssh::{SshProfile, SshStore};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use terminal::{SessionType, TerminalSession};
use theme::*;
use tiling::*;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum ActiveView {
    Terminal,
    SshBookmarks,
    SftpBrowser,
    Settings,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum SshSubView {
    Profiles,
    KeysManager,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum SettingsCategory {
    Appearance,
    Terminal,
    ShellEnv,
    Sftp,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallMethod {
    AppImage,
    ScriptInstalled,
    PackageManager(String),
    Windows,
    MacOS,
    ManualBuild,
}

impl InstallMethod {
    pub fn detect() -> Self {
        if cfg!(windows) {
            return InstallMethod::Windows;
        }
        if cfg!(target_os = "macos") {
            return InstallMethod::MacOS;
        }
        if std::env::var_os("APPIMAGE").is_some() {
            return InstallMethod::AppImage;
        }

        let exe_path = std::env::current_exe().unwrap_or_default();
        let exe_str = exe_path.to_string_lossy();

        if exe_str.contains("/target/debug/") || exe_str.contains("/target/release/") {
            return InstallMethod::ManualBuild;
        }

        if exe_str == "/usr/bin/azterm" {
            if let Ok(out) = std::process::Command::new("pacman").args(["-Q", "azterm"]).output() {
                if out.status.success() {
                    return InstallMethod::PackageManager("Arch Linux (pacman)".to_string());
                }
            }
            if let Ok(out) = std::process::Command::new("dpkg").args(["-s", "azterm"]).output() {
                if out.status.success() {
                    return InstallMethod::PackageManager("Debian/Ubuntu (dpkg)".to_string());
                }
            }
        }

        if exe_str.starts_with("/usr/local/bin") || exe_str.contains("/.local/bin") || exe_str == "/usr/bin/azterm" {
            return InstallMethod::ScriptInstalled;
        }

        InstallMethod::ManualBuild
    }

    pub fn display_name(&self) -> String {
        match self {
            InstallMethod::AppImage => "AppImage (Standalone)".to_string(),
            InstallMethod::ScriptInstalled => "One-Line Shell Script (install.sh)".to_string(),
            InstallMethod::PackageManager(pkg) => format!("Package Manager: {}", pkg),
            InstallMethod::Windows => "Windows Executable".to_string(),
            InstallMethod::MacOS => "macOS Universal Binary".to_string(),
            InstallMethod::ManualBuild => "Manual Source Build".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct CliLaunchOptions {
    working_directory: Option<String>,
    ssh_url: Option<String>,
    open_sftp_only: bool,
    execute_command: Option<Vec<String>>,
}

fn parse_cli_arguments() -> CliLaunchOptions {
    let mut opts = CliLaunchOptions::default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-d" | "--working-directory" | "--dir" => {
                if let Some(dir) = iter.next() {
                    opts.working_directory = Some(dir);
                }
            }
            "--sftp" => {
                opts.open_sftp_only = true;
            }
            "-e" | "--execute" => {
                let rest: Vec<String> = iter.collect();
                if !rest.is_empty() {
                    opts.execute_command = Some(rest);
                }
                break;
            }
            other => {
                if other.starts_with("ssh://") || other.starts_with("ssh:") {
                    opts.ssh_url = Some(other.to_string());
                } else if other.starts_with("sftp://") || other.starts_with("sftp:") {
                    opts.ssh_url = Some(other.replace("sftp://", "ssh://"));
                    opts.open_sftp_only = true;
                } else if Path::new(other).exists() && Path::new(other).is_dir() {
                    opts.working_directory = Some(other.to_string());
                }
            }
        }
    }
    opts
}

fn is_newer_version(latest_tag: &str, current_ver: &str) -> bool {
    let parse_v = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .filter_map(|s| s.parse::<u64>().ok())
            .collect()
    };
    let l_parts = parse_v(latest_tag);
    let c_parts = parse_v(current_ver);
    l_parts > c_parts
}

fn handle_window_resize_borders(ctx: &egui::Context, is_maximized: bool) {
    if is_maximized {
        return;
    }

    let screen_rect = ctx.screen_rect();
    let border_thickness = 5.0_f32;

    let pointer_pos = match ctx.input(|i| i.pointer.hover_pos()) {
        Some(pos) => pos,
        None => return,
    };

    let on_top = pointer_pos.y >= screen_rect.min.y && pointer_pos.y <= screen_rect.min.y + border_thickness;
    let on_bottom = pointer_pos.y <= screen_rect.max.y && pointer_pos.y >= screen_rect.max.y - border_thickness;
    let on_left = pointer_pos.x >= screen_rect.min.x && pointer_pos.x <= screen_rect.min.x + border_thickness;
    let on_right = pointer_pos.x <= screen_rect.max.x && pointer_pos.x >= screen_rect.max.x - border_thickness;

    if !on_top && !on_bottom && !on_left && !on_right {
        return;
    }

    let (dir, cursor) = match (on_top, on_bottom, on_left, on_right) {
        (true, false, true, false) => (
            egui::viewport::ResizeDirection::NorthWest,
            egui::CursorIcon::ResizeNorthWest,
        ),
        (true, false, false, true) => (
            egui::viewport::ResizeDirection::NorthEast,
            egui::CursorIcon::ResizeNorthEast,
        ),
        (false, true, true, false) => (
            egui::viewport::ResizeDirection::SouthWest,
            egui::CursorIcon::ResizeSouthWest,
        ),
        (false, true, false, true) => (
            egui::viewport::ResizeDirection::SouthEast,
            egui::CursorIcon::ResizeSouthEast,
        ),
        (true, false, false, false) => (
            egui::viewport::ResizeDirection::North,
            egui::CursorIcon::ResizeNorth,
        ),
        (false, true, false, false) => (
            egui::viewport::ResizeDirection::South,
            egui::CursorIcon::ResizeSouth,
        ),
        (false, false, true, false) => (
            egui::viewport::ResizeDirection::West,
            egui::CursorIcon::ResizeWest,
        ),
        (false, false, false, true) => (
            egui::viewport::ResizeDirection::East,
            egui::CursorIcon::ResizeEast,
        ),
        _ => return,
    };

    ctx.set_cursor_icon(cursor);

    if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
    }
}

pub struct AppState {
    pub settings: AppSettings,
    pub ssh_store: SshStore,
    pub sessions: Vec<TerminalSession>,
    pub workspaces: Vec<WorkspaceTab>,
    pub active_workspace_idx: usize,
    pub active_session_id: usize,
    pub dragging_tab_idx: Option<usize>,
    pub dragging_pane_id: Option<usize>,
    pub next_split_id: usize,
    pub last_pane_rects: Vec<(usize, egui::Rect)>,

    pub sftp: SftpManager,
    pub next_tab_id: usize,
    pub active_view: ActiveView,
    pub ssh_subview: SshSubView,
    pub settings_category: SettingsCategory,

    pub theme: ThemeConfig,
    pub custom_themes: Vec<ThemeConfig>,
    pub new_theme_name: String,

    pub toast_message: Option<(String, std::time::Instant)>,

    pub available_update: Option<String>,
    pub update_rx: Option<Receiver<Option<String>>>,
    pub is_checking_update: bool,
    pub show_update_modal: bool,
    pub install_method: InstallMethod,

    pub show_profile_modal: bool,
    pub editing_profile_id: Option<String>,
    pub new_ssh_name: String,
    pub new_ssh_host: String,
    pub new_ssh_port: String,
    pub new_ssh_user: String,
    pub new_ssh_auth_choice: usize,
    pub new_ssh_key_path: String,
    pub new_ssh_pasted_key: String,

    pub show_keygen_modal: bool,
    pub keygen_name: String,
    pub generated_pub_key: String,
    pub keygen_status: String,
}

impl AppState {
    fn new(cc: &eframe::CreationContext<'_>, cli: CliLaunchOptions) -> Self {
        let settings = AppSettings::load();
        let ssh_store = SshStore::load();
        let install_method = InstallMethod::detect();
        let custom_themes = Database::load_custom_themes();
        let theme = Database::load_active_theme().unwrap_or_default();

        cc.egui_ctx.set_zoom_factor(settings.zoom_factor);

        let mut app = Self {
            settings,
            ssh_store,
            sessions: Vec::new(),
            workspaces: Vec::new(),
            active_workspace_idx: 0,
            active_session_id: 1,
            dragging_tab_idx: None,
            dragging_pane_id: None,
            next_split_id: 1,
            last_pane_rects: Vec::new(),

            sftp: SftpManager::new(),
            next_tab_id: 1,
            active_view: if cli.open_sftp_only { ActiveView::SftpBrowser } else { ActiveView::Terminal },
            ssh_subview: SshSubView::Profiles,
            settings_category: SettingsCategory::Appearance,

            theme,
            custom_themes,
            new_theme_name: "My Custom Theme".to_string(),

            toast_message: None,

            available_update: None,
            update_rx: None,
            is_checking_update: false,
            show_update_modal: false,
            install_method,

            show_profile_modal: false,
            editing_profile_id: None,
            new_ssh_name: String::new(),
            new_ssh_host: String::new(),
            new_ssh_port: "22".to_string(),
            new_ssh_user: "root".to_string(),
            new_ssh_auth_choice: 0,
            new_ssh_key_path: String::new(),
            new_ssh_pasted_key: String::new(),

            show_keygen_modal: false,
            keygen_name: "prod_server".to_string(),
            generated_pub_key: String::new(),
            keygen_status: String::new(),
        };

        app.trigger_update_check(false, cc.egui_ctx.clone());

        if let Some(dir) = cli.working_directory {
            app.spawn_local_terminal(cc.egui_ctx.clone(), Some(dir));
        } else if let Some(url) = cli.ssh_url {
            app.handle_ssh_url_launch(&url, cc.egui_ctx.clone());
        } else {
            app.restore_saved_sessions(cc.egui_ctx.clone());
        }

        app
    }

    pub fn trigger_update_check(&mut self, force: bool, ctx: egui::Context) {
        if !self.settings.check_updates && !force {
            return;
        }

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if !force && self.settings.last_update_check_date == today {
            return;
        }

        self.settings.last_update_check_date = today;
        self.settings.save();

        self.is_checking_update = true;
        let (tx, rx) = channel::<Option<String>>();
        self.update_rx = Some(rx);

        let current_version = env!("CARGO_PKG_VERSION").to_string();

        thread::spawn(move || {
            let output = std::process::Command::new("curl")
                .args([
                    "-s",
                    "-H", "User-Agent: AZTerm-App",
                    "https://api.github.com/repos/AZBrandCanada/azTerm/releases/latest",
                ])
                .output();

            let mut update_found: Option<String> = None;

            if let Ok(out) = output {
                if out.status.success() {
                    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                        if let Some(tag) = json.get("tag_name").and_then(|v| v.as_str()) {
                            if is_newer_version(tag, &current_version) {
                                update_found = Some(tag.to_string());
                            }
                        }
                    }
                }
            }

            let _ = tx.send(update_found);
            ctx.request_repaint();
        });
    }

    pub fn run_script_update_in_terminal(&mut self, ctx: egui::Context) {
        let update_cmd = "curl -sSL https://raw.githubusercontent.com/AZBrandCanada/azTerm/main/install.sh | bash\n";
        self.spawn_local_terminal(ctx, None);

        if let Some(active_session) = self.sessions.iter_mut().find(|s| s.id == self.active_session_id) {
            active_session.title = "AZTerm Updater".to_string();
            active_session.send_input(update_cmd);
        }
        self.show_update_modal = false;
        self.active_view = ActiveView::Terminal;
        self.set_toast("Running update script in terminal tab...");
    }

    fn handle_ssh_url_launch(&mut self, url: &str, ctx: egui::Context) {
        let clean = url.trim_start_matches("ssh://").trim_start_matches("sftp://");
        let (user_host, port_str) = if let Some((uh, p)) = clean.split_once(':') {
            (uh, p)
        } else {
            (clean, "22")
        };

        let (user, host) = if let Some((u, h)) = user_host.split_once('@') {
            (u.to_string(), h.to_string())
        } else {
            ("root".to_string(), user_host.to_string())
        };

        let port: u16 = port_str.parse().unwrap_or(22);
        let profile = SshProfile::new(&format!("Direct: {}", host), &host, port, &user);
        self.spawn_ssh_terminal(&profile, ctx);
    }

    pub fn set_toast(&mut self, text: impl Into<String>) {
        self.toast_message = Some((text.into(), std::time::Instant::now()));
    }

    pub fn persist_sessions(&self) {
        let saved: Vec<SavedSessionState> = self
            .sessions
            .iter()
            .map(|s| match &s.session_type {
                SessionType::Local { working_dir } => SavedSessionState {
                    kind: "local".to_string(),
                    title: s.title.clone(),
                    target: working_dir.clone(),
                },
                SessionType::Ssh { profile_id } => SavedSessionState {
                    kind: "ssh".to_string(),
                    title: s.title.clone(),
                    target: profile_id.clone(),
                },
            })
            .collect();
        Database::save_sessions(&saved);
        Database::save_workspaces(&self.workspaces);
    }

    fn restore_saved_sessions(&mut self, ctx: egui::Context) {
        let saved = Database::load_sessions();
        if saved.is_empty() {
            if self.settings.open_default_tab {
                self.spawn_local_terminal(ctx, None);
            }
        } else {
            for item in saved {
                if item.kind == "ssh" {
                    if let Some(profile) = self.ssh_store.profiles.iter().find(|p| p.id == item.target) {
                        let profile_clone = profile.clone();
                        self.spawn_ssh_terminal(&profile_clone, ctx.clone());
                    } else {
                        self.spawn_local_terminal(ctx.clone(), None);
                    }
                } else {
                    let dir = if item.target.is_empty() { None } else { Some(item.target) };
                    self.spawn_local_terminal(ctx.clone(), dir);
                }
            }

            if let Some(saved_ws) = Database::load_workspaces() {
                if !saved_ws.is_empty() {
                    self.workspaces = saved_ws;
                }
            }
        }
    }

    fn create_local_session(&mut self, ctx: egui::Context, custom_dir: Option<String>) -> usize {
        let shell = if !self.settings.default_shell.trim().is_empty() {
            self.settings.default_shell.clone()
        } else if cfg!(windows) {
            "powershell.exe".to_string()
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
        };

        let mut c = CommandBuilder::new(shell);
        c.env("TERM", "xterm-256color");
        let work_dir = custom_dir.unwrap_or_else(|| {
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        });
        c.cwd(&work_dir);

        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let session = TerminalSession::new(
            id,
            format!("Local #{}", id),
            SessionType::Local { working_dir: work_dir },
            c,
            ctx,
            self.settings.scrollback_lines,
        );
        self.sessions.push(session);
        id
    }

    pub fn spawn_local_terminal(&mut self, ctx: egui::Context, custom_dir: Option<String>) {
        let id = self.create_local_session(ctx, custom_dir);
        let ws = WorkspaceTab::new(id, id, format!("Local #{}", id));
        self.workspaces.push(ws);
        self.active_workspace_idx = self.workspaces.len() - 1;
        self.active_session_id = id;
        self.active_view = ActiveView::Terminal;
        self.persist_sessions();
    }

    pub fn spawn_ssh_terminal(&mut self, profile: &SshProfile, ctx: egui::Context) {
        let cmd = profile.to_command();
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let session = TerminalSession::new(
            id,
            format!("SSH: {}", profile.name),
            SessionType::Ssh { profile_id: profile.id.clone() },
            cmd,
            ctx,
            self.settings.scrollback_lines,
        );
        self.sessions.push(session);

        let ws = WorkspaceTab::new(id, id, format!("SSH: {}", profile.name));
        self.workspaces.push(ws);
        self.active_workspace_idx = self.workspaces.len() - 1;
        self.active_session_id = id;
        self.active_view = ActiveView::Terminal;

        self.sftp.right_pane.set_target(SftpTarget::RemoteSsh(profile.clone()));
        self.persist_sessions();
    }

    pub fn split_active_pane(&mut self, dir: SplitDirection, ctx: egui::Context) {
        if let Some(ws) = self.workspaces.get(self.active_workspace_idx) {
            if ws.leaves().len() >= 16 {
                self.set_toast("Maximum split limit reached (16 panes per tab)");
                return;
            }
        }

        if let Some((_, r)) = self.last_pane_rects.iter().find(|(id, _)| *id == self.active_session_id) {
            match dir {
                SplitDirection::Horizontal => {
                    if r.width() < 80.0 {
                        self.set_toast("Pane is too narrow to split further (min 80px)");
                        return;
                    }
                }
                SplitDirection::Vertical => {
                    if r.height() < 50.0 {
                        self.set_toast("Pane is too short to split further (min 50px)");
                        return;
                    }
                }
            }
        }

        let new_id = self.create_local_session(ctx, None);
        let split_id = self.next_split_id;
        self.next_split_id += 1;

        if let Some(ws) = self.workspaces.get_mut(self.active_workspace_idx) {
            ws.maximized_session = None;
            ws.root.split_leaf(self.active_session_id, new_id, dir, true, split_id);
            self.active_session_id = new_id;
            self.persist_sessions();
        }
    }

    pub fn close_session(&mut self, session_id: usize, ctx: egui::Context) {
        self.sessions.retain(|s| s.id != session_id);

        let mut ws_idx_to_remove: Option<usize> = None;

        for (w_idx, ws) in self.workspaces.iter_mut().enumerate() {
            if ws.contains(session_id) {
                if ws.is_single_pane() {
                    ws_idx_to_remove = Some(w_idx);
                } else {
                    ws.root.remove_leaf(session_id);
                    if ws.maximized_session == Some(session_id) {
                        ws.maximized_session = None;
                    }
                    self.active_session_id = ws.root.first_leaf();
                }
                break;
            }
        }

        if let Some(idx) = ws_idx_to_remove {
            self.workspaces.remove(idx);
            if self.workspaces.is_empty() {
                self.spawn_local_terminal(ctx, None);
            } else {
                if self.active_workspace_idx >= self.workspaces.len() {
                    self.active_workspace_idx = self.workspaces.len() - 1;
                }
                if let Some(ws) = self.workspaces.get(self.active_workspace_idx) {
                    self.active_session_id = ws.root.first_leaf();
                }
            }
        }

        self.persist_sessions();
    }

    pub fn tile_all_tabs(&mut self) {
        let mut all_sessions = Vec::new();
        for ws in &self.workspaces {
            for leaf in ws.leaves() {
                if !all_sessions.contains(&leaf) && self.sessions.iter().any(|s| s.id == leaf) {
                    all_sessions.push(leaf);
                }
            }
        }

        for s in &self.sessions {
            if !all_sessions.contains(&s.id) {
                all_sessions.push(s.id);
            }
        }

        if all_sessions.len() < 2 {
            self.set_toast("Need at least 2 sessions to tile");
            return;
        }

        const CHUNK_SIZE: usize = 16;
        let mut new_workspaces = Vec::new();

        if all_sessions.len() <= CHUNK_SIZE {
            let new_root = build_balanced_tree(&all_sessions, &mut self.next_split_id, SplitDirection::Horizontal);
            let ws_id = self.next_tab_id;
            self.next_tab_id += 1;

            new_workspaces.push(WorkspaceTab {
                id: ws_id,
                title: format!("Tiled ({} Panes)", all_sessions.len()),
                root: new_root,
                maximized_session: None,
            });
        } else {
            let chunks: Vec<&[usize]> = all_sessions.chunks(CHUNK_SIZE).collect();
            let total_groups = chunks.len();

            for (group_idx, chunk) in chunks.into_iter().enumerate() {
                let chunk_root = build_balanced_tree(chunk, &mut self.next_split_id, SplitDirection::Horizontal);
                let ws_id = self.next_tab_id;
                self.next_tab_id += 1;

                new_workspaces.push(WorkspaceTab {
                    id: ws_id,
                    title: format!("Tiled Group {}/{} ({} Panes)", group_idx + 1, total_groups, chunk.len()),
                    root: chunk_root,
                    maximized_session: None,
                });
            }
        }

        let mut found_ws_idx = 0;
        for (idx, ws) in new_workspaces.iter().enumerate() {
            if ws.contains(self.active_session_id) {
                found_ws_idx = idx;
                break;
            }
        }

        let total_tabs_created = new_workspaces.len();
        self.workspaces = new_workspaces;
        self.active_workspace_idx = found_ws_idx;
        if let Some(active_ws) = self.workspaces.get(self.active_workspace_idx) {
            if !active_ws.contains(self.active_session_id) {
                self.active_session_id = active_ws.root.first_leaf();
            }
        }

        if total_tabs_created > 1 {
            self.set_toast(format!("Tiled {} sessions across {} workspace tabs (max 16/tab)", all_sessions.len(), total_tabs_created));
        } else {
            self.set_toast(format!("Tiled all {} sessions into 1 tab", all_sessions.len()));
        }

        self.persist_sessions();
    }

    pub fn untile_all_to_tabs(&mut self) {
        if self.active_workspace_idx < self.workspaces.len() {
            let ws = &self.workspaces[self.active_workspace_idx];
            let leaves = ws.leaves();
            if leaves.len() <= 1 {
                return;
            }

            let mut expanded_tabs = Vec::new();
            for id in &leaves {
                let title = self.sessions.iter().find(|s| s.id == *id).map(|s| s.title.clone()).unwrap_or_else(|| format!("Local #{}", id));
                expanded_tabs.push(WorkspaceTab::new(*id, *id, title));
            }

            let curr_active_sess = self.active_session_id;
            let current_idx = self.active_workspace_idx;

            self.workspaces.remove(current_idx);
            for (offset, new_tab) in expanded_tabs.into_iter().enumerate() {
                self.workspaces.insert(current_idx + offset, new_tab);
            }

            if let Some(new_idx) = self.workspaces.iter().position(|w| w.contains(curr_active_sess)) {
                self.active_workspace_idx = new_idx;
                self.active_session_id = curr_active_sess;
            } else if current_idx < self.workspaces.len() {
                self.active_workspace_idx = current_idx;
                self.active_session_id = self.workspaces[current_idx].root.first_leaf();
            }

            self.set_toast(format!("Detached {} panes into individual tabs", leaves.len()));
            self.persist_sessions();
        }
    }

    fn sync_sftp_with_active_session(&mut self) {
        if let Some(session) = self.sessions.iter().find(|s| s.id == self.active_session_id) {
            if let SessionType::Ssh { profile_id } = &session.session_type {
                if let Some(prof) = self.ssh_store.profiles.iter().find(|p| p.id == *profile_id) {
                    let target = SftpTarget::RemoteSsh(prof.clone());
                    if self.sftp.right_pane.target != target {
                        self.sftp.right_pane.set_target(target);
                    }
                }
            }
        }
    }

    pub fn open_create_profile_modal(&mut self) {
        self.editing_profile_id = None;
        self.new_ssh_name = "My Server".to_string();
        self.new_ssh_host = "192.168.1.100".to_string();
        self.new_ssh_port = "22".to_string();
        self.new_ssh_user = "root".to_string();
        self.new_ssh_auth_choice = 0;
        self.new_ssh_key_path.clear();
        self.new_ssh_pasted_key.clear();
        self.show_profile_modal = true;
    }

    pub fn open_edit_profile_modal(&mut self, profile: &SshProfile) {
        self.editing_profile_id = Some(profile.id.clone());
        self.new_ssh_name = profile.name.clone();
        self.new_ssh_host = profile.host.clone();
        self.new_ssh_port = profile.port.to_string();
        self.new_ssh_user = profile.username.clone();

        match &profile.auth_type {
            ssh::SshAuthType::PasswordOrAgent => {
                self.new_ssh_auth_choice = 0;
                self.new_ssh_key_path.clear();
                self.new_ssh_pasted_key.clear();
            }
            ssh::SshAuthType::KeyFile(path) => {
                self.new_ssh_auth_choice = 1;
                self.new_ssh_key_path = path.clone();
                self.new_ssh_pasted_key.clear();
            }
            ssh::SshAuthType::PastedKey { key_id } => {
                self.new_ssh_auth_choice = 2;
                self.new_ssh_key_path.clear();
                let key_path = SshStore::keys_dir().join(format!("{}.pem", key_id));
                self.new_ssh_pasted_key = std::fs::read_to_string(key_path).unwrap_or_default();
            }
        }
        self.show_profile_modal = true;
    }

    fn handle_zoom_shortcuts(&mut self, ctx: &egui::Context) {
        let is_ctrl = ctx.input(|i| i.modifiers.ctrl && !i.modifiers.alt);
        let mut zoom_delta = 0.0_f32;

        if is_ctrl {
            let plus_pressed = ctx.input(|i| {
                i.key_pressed(egui::Key::Plus)
                    || i.key_pressed(egui::Key::Equals)
                    || i.events.iter().any(|e| match e {
                        egui::Event::Key { key: egui::Key::Plus, pressed: true, .. }
                        | egui::Event::Key { key: egui::Key::Equals, pressed: true, .. } => true,
                        egui::Event::Text(t) => t == "+" || t == "=",
                        _ => false,
                    })
            });

            let minus_pressed = ctx.input(|i| {
                i.key_pressed(egui::Key::Minus)
                    || i.events.iter().any(|e| match e {
                        egui::Event::Key { key: egui::Key::Minus, pressed: true, .. } => true,
                        egui::Event::Text(t) => t == "-",
                        _ => false,
                    })
            });

            let zero_pressed = ctx.input(|i| {
                i.key_pressed(egui::Key::Num0)
                    || i.events.iter().any(|e| match e {
                        egui::Event::Key { key: egui::Key::Num0, pressed: true, .. } => true,
                        egui::Event::Text(t) => t == "0",
                        _ => false,
                    })
            });

            let wheel_delta = ctx.input(|i| {
                if i.raw_scroll_delta.y != 0.0 {
                    i.raw_scroll_delta.y
                } else {
                    i.smooth_scroll_delta.y
                }
            });

            if zero_pressed {
                self.settings.zoom_factor = 1.0;
                ctx.set_zoom_factor(1.0);
                self.settings.save();
                self.set_toast("Zoom Reset (100%)");
            } else if plus_pressed || wheel_delta > 10.0 {
                zoom_delta += 0.1;
            } else if minus_pressed || wheel_delta < -10.0 {
                zoom_delta -= 0.1;
            }
        }

        if zoom_delta != 0.0 {
            let new_zoom = (self.settings.zoom_factor + zoom_delta).clamp(0.6, 2.5);
            if (new_zoom - self.settings.zoom_factor).abs() > 0.01 {
                self.settings.zoom_factor = (new_zoom * 10.0).round() / 10.0;
                ctx.set_zoom_factor(self.settings.zoom_factor);
                self.settings.save();
                self.set_toast(format!("Zoom: {}%", (self.settings.zoom_factor * 100.0).round() as u32));
            }
        }
    }

    fn handle_terminal_shortcuts(&mut self, ctx: &egui::Context) {
        let mut send_tab = false;

        ctx.input_mut(|i| {
            i.events.retain(|event| match event {
                egui::Event::Key { key: egui::Key::Tab, pressed, .. } => {
                    if *pressed {
                        send_tab = true;
                    }
                    false
                }
                egui::Event::Text(t) if t == "\t" => {
                    send_tab = true;
                    false
                }
                _ => true,
            });
        });

        if send_tab {
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == self.active_session_id) {
                session.send_input("\t");
            }
        }

        let (ctrl_shift, alt_pressed) = ctx.input(|i| {
            (
                i.modifiers.ctrl && i.modifiers.shift && !i.modifiers.alt,
                i.modifiers.alt && !i.modifiers.ctrl && !i.modifiers.shift,
            )
        });

        if ctrl_shift {
            if ctx.input(|i| i.key_pressed(egui::Key::D)) {
                self.split_active_pane(SplitDirection::Horizontal, ctx.clone());
            } else if ctx.input(|i| i.key_pressed(egui::Key::E)) {
                self.split_active_pane(SplitDirection::Vertical, ctx.clone());
            } else if ctx.input(|i| i.key_pressed(egui::Key::M)) {
                if let Some(ws) = self.workspaces.get_mut(self.active_workspace_idx) {
                    ws.maximized_session = if ws.maximized_session.is_some() { None } else { Some(self.active_session_id) };
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::W)) {
                self.close_session(self.active_session_id, ctx.clone());
            }
        }

        if alt_pressed {
            if let Some(ws) = self.workspaces.get(self.active_workspace_idx) {
                let leaves = ws.leaves();
                if let Some(curr_idx) = leaves.iter().position(|id| *id == self.active_session_id) {
                    if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowDown)) {
                        let next_idx = (curr_idx + 1) % leaves.len();
                        self.active_session_id = leaves[next_idx];
                    } else if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowUp)) {
                        let prev_idx = if curr_idx == 0 { leaves.len() - 1 } else { curr_idx - 1 };
                        self.active_session_id = leaves[prev_idx];
                    }
                }
            }
        }
    }
}

impl eframe::App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.settings.use_system_titlebar {
            let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            handle_window_resize_borders(ctx, is_max);
        }

        self.handle_zoom_shortcuts(ctx);

        if self.show_update_modal {
            ui::modals::render_update_modal(self, ctx);
        }

        if self.show_keygen_modal {
            ui::modals::render_keygen_modal(self, ctx);
        }

        if self.show_profile_modal {
            ui::modals::render_profile_modal(self, ctx);
        }

        let modal_open = self.show_update_modal || self.show_profile_modal || self.show_keygen_modal;
        if self.active_view == ActiveView::Terminal && !modal_open {
            self.handle_terminal_shortcuts(ctx);
        }

        for s in &mut self.sessions {
            s.poll_updates();
        }

        if let Some(ref rx) = self.update_rx {
            if let Ok(res) = rx.try_recv() {
                self.is_checking_update = false;
                if self.settings.check_updates {
                    self.available_update = res;
                }
            }
        }

        self.sync_sftp_with_active_session();

        ui::navbar::render_top_nav(self, ctx);
        ui::navbar::render_tabs_bar(self, ctx);
        ui::navbar::render_status_bar(self, ctx);

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.theme.bg_main_color()))
            .show(ctx, |ui| match self.active_view {
                ActiveView::Terminal => {
                    ui::terminal_view::render_terminal_workspace(self, ctx, ui);
                }
                ActiveView::SshBookmarks => {
                    ui::ssh_view::render_ssh_view(self, ctx, ui);
                }
                ActiveView::SftpBrowser => {
                    ui::sftp_view::render_sftp_browser_view(self, ui);
                }
                ActiveView::Settings => {
                    ui::settings_view::render_settings_view(self, ctx, ui);
                }
            });
    }
}

fn main() -> eframe::Result<()> {
    let cli_opts = parse_cli_arguments();
    let initial_settings = AppSettings::load();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1160.0, 740.0])
            .with_title("AZTerm")
            .with_app_id("azterm")
            .with_transparent(true)
            .with_decorations(initial_settings.use_system_titlebar)
            .with_resizable(true),
        ..Default::default()
    };
    eframe::run_native(
        "AZTerm",
        options,
        Box::new(|cc| Ok(Box::new(AppState::new(cc, cli_opts)))),
    )
}
