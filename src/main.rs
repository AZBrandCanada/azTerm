// src/main.rs
mod daemon;
mod daemon_client;
mod daemon_server;
mod db;
mod debug_log;
mod fonts;
mod modern;
mod settings;
mod sftp;
mod ssh;
mod terminal;
mod theme;
mod tiling;
mod ui;

use daemon_client::DaemonClient;
use db::{Database, SavedSessionState};
use eframe::egui;
use portable_pty::CommandBuilder;
use settings::AppSettings;
use sftp::{SftpManager, SftpTarget};
use ssh::{SshProfile, SshStore};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use terminal::{SessionType, TerminalSession};
use theme::*;
use tiling::*;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
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
            InstallMethod::ScriptInstalled => "Shell Script (install.sh)".to_string(),
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
        if arg.starts_with("--working-directory=") {
            opts.working_directory = Some(arg.trim_start_matches("--working-directory=").to_string());
        } else if arg.starts_with("--dir=") {
            opts.working_directory = Some(arg.trim_start_matches("--dir=").to_string());
        } else {
            match arg.as_str() {
                "-d" | "--working-directory" | "--dir" | "-w" => {
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
                    } else {
                        let path_candidate = if other.starts_with("file://") {
                            other.trim_start_matches("file://").to_string()
                        } else {
                            other.to_string()
                        };
                        let decoded = path_candidate.replace("%20", " ");
                        if Path::new(&decoded).exists() && Path::new(&decoded).is_dir() {
                            opts.working_directory = Some(decoded);
                        }
                    }
                }
            }
        }
    }
    opts
}

pub fn is_newer_version(latest_tag: &str, current_ver: &str) -> bool {
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

#[derive(Debug, Clone)]
pub enum UpdateCheckResult {
    NewVersion(String),
    AlreadyUpToDate,
    CheckFailed,
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

fn collect_max_split_id(node: &TileNode, max: &mut usize) {
    match node {
        TileNode::Leaf(_) => {}
        TileNode::Split { id, first, second, .. } => {
            if *id >= *max {
                *max = *id + 1;
            }
            collect_max_split_id(first, max);
            collect_max_split_id(second, max);
        }
    }
}

pub struct SshAuthModalState {
    pub profile: SshProfile,
    pub output: Arc<Mutex<String>>,
    pub writer: Arc<Mutex<Box<dyn std::io::Write + Send>>>,
    pub input_text: String,
    pub show_plain: bool,
    pub is_connected: bool,
    pub target_pane_id: String,
}

pub struct AppState {
    pub settings: AppSettings,
    pub ssh_store: SshStore,
    pub sessions: Vec<TerminalSession>,
    pub workspaces: Vec<WorkspaceTab>,
    pub active_workspace_idx: usize,
    pub active_session_id: usize,
    pub last_synced_session_id: Option<usize>,
    pub dragging_tab_idx: Option<usize>,
    pub dragging_pane_id: Option<usize>,
    pub next_split_id: usize,
    pub last_pane_rects: Vec<(usize, egui::Rect)>,
    pub last_focused_session_id: Option<usize>,

    pub sftp: SftpManager,
    pub next_tab_id: usize,
    pub active_view: ActiveView,
    pub ssh_subview: SshSubView,
    pub settings_category: SettingsCategory,

    pub theme: ThemeConfig,
    pub custom_themes: Vec<ThemeConfig>,
    pub new_theme_name: String,

    /// Daemon client when use_daemon is enabled and the daemon is
    /// reachable. When None, sessions live in-process (legacy path).
    pub daemon: Option<Arc<DaemonClient>>,

    pub toast_message: Option<(String, std::time::Instant)>,
    /// Set whenever a view / tab / settings category changes so the
    /// update() pass knows to play a short fade + sweep. Cleared
    /// automatically once the transition finishes.
    pub last_transition: Option<std::time::Instant>,
    pub last_heartbeat: std::time::Instant,

    /// Last UI font path applied via `ctx.set_fonts()`. Used to detect
    /// when the user changes the font in Settings so we can re-apply.
    pub applied_ui_font_path: Option<String>,
    /// Last terminal font path applied via `ctx.set_fonts()`.
    pub applied_terminal_font_path: Option<String>,
    /// Cache of installed fonts, populated lazily by the Settings picker.
    pub cached_fonts: Option<Vec<fonts::FontEntry>>,
    /// Whether the current `cached_fonts` list has been pushed into
    /// egui as named preview families. Reset whenever cached_fonts is
    /// invalidated (font rescan).
    pub preview_fonts_loaded: bool,

    pub available_update: Option<String>,
    pub update_rx: Option<Receiver<UpdateCheckResult>>,
    pub is_checking_update: bool,
    pub show_update_modal: bool,
    /// Set when the user toggles off "Keep Sessions Running in
    /// Background" while daemon-backed sessions exist. Renders a
    /// confirmation modal; confirming calls `disable_daemon()`.
    pub show_disable_daemon_modal: bool,
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
    pub keygen_algo: usize,
    pub generated_pub_key: String,
    pub keygen_status: String,

    pub ssh_auth_modal: Option<SshAuthModalState>,
}


fn modal_open_flag(app: &AppState) -> bool {
    app.show_update_modal
        || app.show_profile_modal
        || app.show_keygen_modal
        || app.ssh_auth_modal.is_some()
        || app.sftp.has_open_modal()
}

impl AppState {
    fn new(cc: &eframe::CreationContext<'_>, cli: CliLaunchOptions) -> Self {
        let mut settings = AppSettings::load();
        let ssh_store = SshStore::load();
        let install_method = InstallMethod::detect();
        let custom_themes = Database::load_custom_themes();
        let theme = Database::load_active_theme().unwrap_or_default();

        cc.egui_ctx.set_zoom_factor(settings.zoom_factor);

        // Apply the user's chosen fonts. UI font fills egui's
        // Proportional family, terminal font fills Monospace. See
        // src/fonts.rs for discovery; pickers live in Settings →
        // Themes & Window Appearance.
        fonts::apply_to_egui(
            &cc.egui_ctx,
            &settings.ui_font_path,
            &settings.terminal_font_path,
            &[],
        );
        let applied_ui_font_init = settings.ui_font_path.clone();
        let applied_terminal_font_init = settings.terminal_font_path.clone();

        // Daemon: if enabled, ensure a background process is running
        // and connect to it. Falls back to in-process mode cleanly if
        // the daemon binary is missing or fails to start.
        let daemon = if settings.use_daemon {
            daemon_client::ensure_daemon_running().map(Arc::new)
        } else {
            None
        };

        let current_version = env!("CARGO_PKG_VERSION");
        let initial_available_update = if let Some(ref tag) = settings.pending_update {
            if is_newer_version(tag, current_version) {
                Some(tag.clone())
            } else {
                settings.pending_update = None;
                settings.save();
                None
            }
        } else {
            None
        };

        let mut app = Self {
            settings,
            ssh_store,
            sessions: Vec::new(),
            workspaces: Vec::new(),
            active_workspace_idx: 0,
            active_session_id: 1,
            last_synced_session_id: None,
            dragging_tab_idx: None,
            dragging_pane_id: None,
            next_split_id: 1,
            last_pane_rects: Vec::new(),
            last_focused_session_id: None,

            sftp: SftpManager::new(),
            next_tab_id: 1,
            active_view: if cli.open_sftp_only { ActiveView::SftpBrowser } else { ActiveView::Terminal },
            ssh_subview: SshSubView::Profiles,
            settings_category: SettingsCategory::Appearance,

            theme,
            custom_themes,
            new_theme_name: "Custom Theme".to_string(),

            daemon,
            toast_message: None,
            last_transition: None,
            last_heartbeat: std::time::Instant::now(),

            applied_ui_font_path: Some(applied_ui_font_init),
            applied_terminal_font_path: Some(applied_terminal_font_init),
            cached_fonts: None,
            preview_fonts_loaded: false,

            available_update: initial_available_update,
            update_rx: None,
            is_checking_update: false,
            show_update_modal: false,
            show_disable_daemon_modal: false,
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
            keygen_algo: 0,
            generated_pub_key: String::new(),
            keygen_status: String::new(),

            ssh_auth_modal: None,
        };

        app.schedule_boot_update_check(cc.egui_ctx.clone());

        if let Some(dir) = cli.working_directory {
            app.spawn_local_terminal(cc.egui_ctx.clone(), Some(dir));
        } else if let Some(url) = cli.ssh_url {
            app.handle_ssh_url_launch(&url, cc.egui_ctx.clone());
        } else {
            app.restore_saved_sessions(cc.egui_ctx.clone());
        }

        app
    }

    pub fn perform_github_update_check(current_version: &str) -> UpdateCheckResult {
        let output = std::process::Command::new("curl")
            .args([
                "-s",
                "--max-time", "10",
                "-H", "User-Agent: AZTerm-App",
                "https://api.github.com/repos/AZBrandCanada/azTerm/releases/latest",
            ])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                    if let Some(tag) = json.get("tag_name").and_then(|v| v.as_str()) {
                        if is_newer_version(tag, current_version) {
                            return UpdateCheckResult::NewVersion(tag.to_string());
                        } else {
                            return UpdateCheckResult::AlreadyUpToDate;
                        }
                    }
                }
            }
        }
        UpdateCheckResult::CheckFailed
    }

    pub fn schedule_boot_update_check(&mut self, ctx: egui::Context) {
        if !self.settings.check_updates {
            return;
        }

        if self.available_update.is_some() {
            return;
        }

        let (tx, rx) = channel::<UpdateCheckResult>();
        self.update_rx = Some(rx);

        let current_version = env!("CARGO_PKG_VERSION").to_string();

        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_secs(30));

            let res = Self::perform_github_update_check(&current_version);
            let _ = tx.send(res);
            ctx.request_repaint();
        });
    }

    pub fn trigger_update_check(&mut self, force: bool, ctx: egui::Context) {
        if !self.settings.check_updates && !force {
            return;
        }

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if !force && self.settings.last_update_check_date == today && self.available_update.is_some() {
            return;
        }

        self.settings.last_update_check_date = today;
        self.settings.save();

        self.is_checking_update = true;
        let (tx, rx) = channel::<UpdateCheckResult>();
        self.update_rx = Some(rx);

        let current_version = env!("CARGO_PKG_VERSION").to_string();

        thread::spawn(move || {
            let res = Self::perform_github_update_check(&current_version);
            let _ = tx.send(res);
            ctx.request_repaint();
        });
    }

    pub fn open_ssh_auth_modal(&mut self, profile: SshProfile, target_pane_id: String, ctx: egui::Context) {
        let socket_path = SshStore::sockets_dir().join(format!("{}.sock", profile.id));
        if socket_path.exists() {
            // Fire-and-forget probe; do not block the UI thread.
            let pid = profile.id.clone();
            std::thread::spawn(move || {
                SshStore::cleanup_stale_socket(&pid);
            });
        }
        if socket_path.exists() {
            if target_pane_id == "sftp_left" {
                self.sftp.left_pane.refresh();
            } else {
                self.sftp.right_pane.refresh();
            }
            return;
        }

        let pty_system = portable_pty::native_pty_system();
        let pair = match pty_system.openpty(portable_pty::PtySize {
            rows: 16,
            cols: 72,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(p) => p,
            Err(e) => {
                self.set_toast(format!("Failed to open PTY: {}", e));
                return;
            }
        };

        let cmd = profile.to_command();
        if let Err(e) = pair.slave.spawn_command(cmd) {
            self.set_toast(format!("Failed to spawn SSH: {}", e));
            return;
        }

        let mut reader = match pair.master.try_clone_reader() {
            Ok(r) => r,
            Err(e) => {
                self.set_toast(format!("Failed to clone PTY reader: {}", e));
                return;
            }
        };

        let writer = match pair.master.take_writer() {
            Ok(w) => Arc::new(Mutex::new(w)),
            Err(e) => {
                self.set_toast(format!("Failed to take PTY writer: {}", e));
                return;
            }
        };

        let output = Arc::new(Mutex::new(String::new()));
        let output_clone = output.clone();
        let ctx_clone = ctx.clone();

        std::thread::spawn(move || {
            use std::io::Read;
            const MAX_OUTPUT: usize = 64 * 1024;
            let mut buf = [0u8; 1024];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = String::from_utf8_lossy(&buf[..n]);
                if let Ok(mut text) = output_clone.lock() {
                    text.push_str(&chunk);
                    if text.len() > MAX_OUTPUT {
                        // Drain from the front, respecting char boundaries.
                        let mut cut = text.len() - MAX_OUTPUT;
                        while cut < text.len() && !text.is_char_boundary(cut) {
                            cut += 1;
                        }
                        text.drain(..cut);
                    }
                }
                ctx_clone.request_repaint();
            }
        });

        self.ssh_auth_modal = Some(SshAuthModalState {
            profile,
            output,
            writer,
            input_text: String::new(),
            show_plain: false,
            is_connected: false,
            target_pane_id,
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

    /// Call after any UI state change that should trigger the fade + sweep.
    pub fn trigger_transition(&mut self) {
        self.last_transition = Some(std::time::Instant::now());
    }

    pub fn set_toast(&mut self, text: impl Into<String>) {
        self.toast_message = Some((text.into(), std::time::Instant::now()));
    }

    pub fn persist_sessions(&mut self) {
        let saved: Vec<SavedSessionState> = self
            .sessions
            .iter()
            .map(|s| {
                let (kind, target) = match &s.session_type {
                    SessionType::Local { working_dir } => {
                        ("local".to_string(), working_dir.clone())
                    }
                    SessionType::Ssh { profile_id } => {
                        ("ssh".to_string(), profile_id.clone())
                    }
                };
                SavedSessionState {
                    id: s.id,
                    kind,
                    title: s.title.clone(),
                    target,
                }
            })
            .collect();
        Database::save_sessions(&saved);
        Database::save_workspaces(&self.workspaces);

        // Persist terminal scrollback for any session whose output has
        // changed since the last save. Idle sessions cost nothing.
        // Daemon-backed sessions are skipped: their scrollback lives in
        // the daemon's replay buffer and is streamed on attach, so
        // writing it to disk too would produce duplicates on restart.
        for s in self.sessions.iter_mut() {
            if s.is_daemon() {
                s.history_dirty = false;
                continue;
            }
            if s.history_dirty {
                Database::save_scrollback(s.id, &s.history_buf);
                s.history_dirty = false;
            }
        }
    }

    fn restore_saved_sessions(&mut self, ctx: egui::Context) {
        // Daemon path first: if the daemon already owns live sessions
        // from a previous run, reattach to them instead of spawning new
        // ones. Their IDs are preserved so saved layouts still match.
        if let Some(ref daemon) = self.daemon {
            let live = daemon.list();
            if !live.is_empty() {
                let saved_ws = Database::load_workspaces();
                for info in &live {
                    let id = info.id as usize;
                    let session_type = if info.kind == "ssh" {
                        SessionType::Ssh {
                            profile_id: info.target.clone(),
                        }
                    } else {
                        SessionType::Local {
                            working_dir: info.target.clone(),
                        }
                    };
                    if let Some(mut s) = TerminalSession::new_daemon(
                        id,
                        info.title.clone(),
                        session_type,
                        daemon.clone(),
                        ctx.clone(),
                        self.settings.scrollback_lines,
                        info.cols,
                        info.rows,
                    ) {
                        s.recovery_note = Some((
                            format!("\u{2713} Reattached  #{}  ", info.id),
                            (100, 220, 140),
                        ));
                        self.sessions.push(s);
                        if id >= self.next_tab_id {
                            self.next_tab_id = id + 1;
                        }
                    }
                }

                // Reattach saved layout if every leaf maps to a live session.
                if let Some(ws_list) = saved_ws {
                    if !ws_list.is_empty() {
                        let session_ids: std::collections::HashSet<usize> =
                            self.sessions.iter().map(|s| s.id).collect();
                        let layout_valid = ws_list
                            .iter()
                            .all(|ws| ws.leaves().iter().all(|id| session_ids.contains(id)));
                        if layout_valid {
                            let mut max_split = 0usize;
                            let mut max_ws_id = 0usize;
                            for ws in &ws_list {
                                collect_max_split_id(&ws.root, &mut max_split);
                                if ws.id >= max_ws_id {
                                    max_ws_id = ws.id + 1;
                                }
                            }
                            self.next_split_id = self.next_split_id.max(max_split + 1);
                            // Bump next_tab_id above every loaded workspace
                            // id, not just every daemon session id. On the
                            // previous run, a workspace could have been
                            // created with id = next_tab_id (e.g. tile_all),
                            // so its id may be HIGHER than any session id.
                            // Without this bump, the next new tab spawns
                            // with the same id, and their per-tab widget
                            // ids collide — egui keeps only the newest, so
                            // the older tab's close button goes dead.
                            self.next_tab_id = self.next_tab_id.max(max_ws_id);
                            self.workspaces = ws_list;
                            self.active_workspace_idx = 0;
                            if let Some(ws) = self.workspaces.first() {
                                self.active_session_id = ws.root.first_leaf();
                            }
                            self.active_view = ActiveView::Terminal;
                            return;
                        }
                    }
                }

                // Fallback: one tab per reattached session.
                for s in &self.sessions {
                    let ws = WorkspaceTab::new(s.id, s.id, s.title.clone());
                    self.workspaces.push(ws);
                }
                self.active_workspace_idx = 0;
                if let Some(ws) = self.workspaces.first() {
                    self.active_session_id = ws.root.first_leaf();
                }
                self.active_view = ActiveView::Terminal;
                return;
            }
        }

        let saved = Database::load_sessions();
        if saved.is_empty() {
            if self.settings.open_default_tab {
                self.spawn_local_terminal(ctx, None);
            }
            return;
        }

        let saved_ws = Database::load_workspaces();

        // Recreate every session with its ORIGINAL saved ID so the saved
        // workspace TileNode::Leaf(id) references remain valid.
        let mut max_id = 0usize;
        for (idx, item) in saved.iter().enumerate() {
            // Migration: pre-patch DBs stored session_uid=0 for every row.
            // Assign sequential IDs in that case.
            let id = if item.id == 0 { idx + 1 } else { item.id };

            match item.kind.as_str() {
                "ssh" => {
                    if let Some(profile) = self
                        .ssh_store
                        .profiles
                        .iter()
                        .find(|p| p.id == item.target)
                        .cloned()
                    {
                        self.create_ssh_session_with_id(
                            &profile,
                            id,
                            ctx.clone(),
                            Some(item.title.clone()),
                        );
                    } else {
                        self.create_local_session_with_id(
                            id,
                            ctx.clone(),
                            None,
                            Some(item.title.clone()),
                        );
                    }
                }
                _ => {
                    let dir = if item.target.is_empty() {
                        None
                    } else {
                        Some(item.target.clone())
                    };
                    self.create_local_session_with_id(
                        id,
                        ctx.clone(),
                        dir,
                        Some(item.title.clone()),
                    );
                }
            }
            if id >= max_id {
                max_id = id + 1;
            }
        }
        self.next_tab_id = self.next_tab_id.max(max_id);

        // Replay persisted scrollback into each session's parser so tab
        // history survives a restart. This runs BEFORE the shell's first
        // bytes are drained from the channel, so ordering is guaranteed:
        // history first, then the fresh prompt on top of a cleared pane.
        for s in &mut self.sessions {
            if s.is_daemon() {
                continue;
            }
            if let Some(bytes) = Database::load_scrollback(s.id) {
                s.feed_restore_history(&bytes);
            }
        }

        // Restore the saved tiling layout, but only if every leaf still maps
        // to a restored session (guards against deleted SSH profiles and
        // pre-patch DBs where IDs were never persisted).
        if let Some(ws_list) = saved_ws {
            if !ws_list.is_empty() {
                let session_ids: std::collections::HashSet<usize> =
                    self.sessions.iter().map(|s| s.id).collect();
                let layout_valid = ws_list
                    .iter()
                    .all(|ws| ws.leaves().iter().all(|id| session_ids.contains(id)));

                if layout_valid {
                    let mut max_split = 0usize;
                    let mut max_ws_id = 0usize;
                    for ws in &ws_list {
                        collect_max_split_id(&ws.root, &mut max_split);
                        if ws.id >= max_ws_id {
                            max_ws_id = ws.id + 1;
                        }
                    }
                    self.next_split_id = self.next_split_id.max(max_split + 1);
                    // See daemon reattach path above for the reasoning.
                    self.next_tab_id = self.next_tab_id.max(max_ws_id);

                    self.workspaces = ws_list;
                    self.active_workspace_idx = 0;
                    if let Some(ws) = self.workspaces.first() {
                        self.active_session_id = ws.root.first_leaf();
                    }
                    return;
                }
            }
        }

        // Fallback: no usable layout — one tab per session.
        for s in &self.sessions {
            let ws = WorkspaceTab::new(s.id, s.id, s.title.clone());
            self.workspaces.push(ws);
        }
        self.active_workspace_idx = 0;
        if let Some(ws) = self.workspaces.first() {
            self.active_session_id = ws.root.first_leaf();
        }
    }

    fn create_local_session_with_id(
        &mut self,
        id: usize,
        ctx: egui::Context,
        custom_dir: Option<String>,
        title_override: Option<String>,
    ) -> usize {
        let shell = if !self.settings.default_shell.trim().is_empty() {
            self.settings.default_shell.clone()
        } else if cfg!(windows) {
            "powershell.exe".to_string()
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
        };

        let work_dir = custom_dir.unwrap_or_else(|| {
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        });

        let title = title_override.unwrap_or_else(|| format!("Local #{}", id));

        // DAEMON PATH: when a daemon is running, create the shell there
        // so it survives window close / app restart. The ID is passed
        // through unchanged so saved layouts still match.
        //
        // This path is what makes restore-after-reboot work. After a
        // reboot the daemon is empty (it died with the OS), but a fresh
        // one has just been spawned. Without this check, restore fell
        // straight through to the in-process path and recovered tabs
        // were never daemon-backed — so they could not be recovered a
        // second time.
        //
        // Also covers split_active_pane, which used to always produce
        // in-process sessions regardless of the daemon setting.
        let daemon_opt = self.daemon.clone();
        if let Some(ref daemon) = daemon_opt {
            if daemon
                .new_local(
                    id as u64,
                    &title,
                    Some(&work_dir),
                    &shell,
                    daemon::DEFAULT_COLS,
                    daemon::DEFAULT_ROWS,
                )
                .is_ok()
            {
                let session_type = SessionType::Local {
                    working_dir: work_dir.clone(),
                };
                if let Some(s) = TerminalSession::new_daemon(
                    id,
                    title.clone(),
                    session_type,
                    daemon.clone(),
                    ctx.clone(),
                    self.settings.scrollback_lines,
                    daemon::DEFAULT_COLS,
                    daemon::DEFAULT_ROWS,
                ) {
                    self.sessions.push(s);
                    return id;
                }
            }
            // Daemon refused. Fall through to in-process so the user
            // still gets a working shell rather than a silent failure.
        }

        let mut c = CommandBuilder::new(shell);
        c.env("TERM", "xterm-256color");
        c.env("COLORTERM", "truecolor");
        c.env_remove("LINES");
        c.env_remove("COLUMNS");

        let lang = std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".to_string());
        c.env("LANG", lang);

        c.cwd(&work_dir);

        let session = TerminalSession::new(
            id,
            title,
            SessionType::Local { working_dir: work_dir },
            c,
            ctx,
            self.settings.scrollback_lines,
        );
        self.sessions.push(session);
        id
    }

    fn create_local_session(&mut self, ctx: egui::Context, custom_dir: Option<String>) -> usize {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.create_local_session_with_id(id, ctx, custom_dir, None)
    }

    fn create_ssh_session_with_id(
        &mut self,
        profile: &SshProfile,
        id: usize,
        ctx: egui::Context,
        title_override: Option<String>,
    ) {
        // Cleanup can shell out to `ssh -O check` and block for seconds on a
        // dead socket — do it on a worker thread so the UI never freezes.
        let pid_for_cleanup = profile.id.clone();
        std::thread::spawn(move || {
            SshStore::cleanup_stale_socket(&pid_for_cleanup);
        });

        let title = title_override.unwrap_or_else(|| format!("SSH: {}", profile.name));

        // DAEMON PATH: mirror spawn_ssh_terminal's setup so restored SSH
        // tabs are daemon-backed and survive close / restart. Without
        // this, recovered SSH sessions were in-process and could not be
        // recovered a second time.
        let daemon_opt = self.daemon.clone();
        if let Some(ref daemon) = daemon_opt {
            let identity_file = match &profile.auth_type {
                ssh::SshAuthType::KeyFile(p) => {
                    if p.trim().is_empty() {
                        None
                    } else {
                        SshStore::ensure_secure_permissions(p);
                        Some(p.clone())
                    }
                }
                ssh::SshAuthType::PastedKey { key_id } => {
                    let kp = SshStore::keys_dir().join(format!("{}.pem", key_id));
                    if kp.exists() {
                        let s = kp.to_string_lossy().to_string();
                        SshStore::ensure_secure_permissions(&s);
                        Some(s)
                    } else {
                        None
                    }
                }
                ssh::SshAuthType::PasswordOrAgent => None,
            };
            let control_path = SshStore::sockets_dir()
                .join(format!("{}.sock", profile.id))
                .to_string_lossy()
                .to_string();

            let spec = daemon::SshSpec {
                host: profile.host.clone(),
                port: profile.port,
                username: profile.username.clone(),
                identity_file,
                control_path: Some(control_path),
                profile_id: Some(profile.id.clone()),
            };

            if daemon
                .new_ssh(
                    id as u64,
                    &title,
                    spec,
                    daemon::DEFAULT_COLS,
                    daemon::DEFAULT_ROWS,
                )
                .is_ok()
            {
                let session_type = SessionType::Ssh {
                    profile_id: profile.id.clone(),
                };
                if let Some(s) = TerminalSession::new_daemon(
                    id,
                    title.clone(),
                    session_type,
                    daemon.clone(),
                    ctx.clone(),
                    self.settings.scrollback_lines,
                    daemon::DEFAULT_COLS,
                    daemon::DEFAULT_ROWS,
                ) {
                    self.sessions.push(s);
                    return;
                }
            }
            // Daemon refused. Fall through to in-process.
        }

        let mut cmd = profile.to_command();
        cmd.env("COLORTERM", "truecolor");
        cmd.env_remove("LINES");
        cmd.env_remove("COLUMNS");

        let session = TerminalSession::new(
            id,
            title,
            SessionType::Ssh { profile_id: profile.id.clone() },
            cmd,
            ctx,
            self.settings.scrollback_lines,
        );
        self.sessions.push(session);
    }

    pub fn spawn_local_terminal(&mut self, ctx: egui::Context, custom_dir: Option<String>) {
        let id = self.next_tab_id;
        self.next_tab_id += 1;

        if let Some(ref daemon) = self.daemon {
            let shell = if !self.settings.default_shell.trim().is_empty() {
                self.settings.default_shell.clone()
            } else if cfg!(windows) {
                "powershell.exe".to_string()
            } else {
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
            };
            let title = format!("Local #{}", id);
            let work_dir = custom_dir
                .clone()
                .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| ".".into()));

            match daemon.new_local(
                id as u64,
                &title,
                Some(&work_dir),
                &shell,
                daemon::DEFAULT_COLS,
                daemon::DEFAULT_ROWS,
            ) {
                Ok(()) => {
                    let session_type = SessionType::Local {
                        working_dir: work_dir,
                    };
                    match TerminalSession::new_daemon(
                        id,
                        title.clone(),
                        session_type,
                        daemon.clone(),
                        ctx.clone(),
                        self.settings.scrollback_lines,
                        daemon::DEFAULT_COLS,
                        daemon::DEFAULT_ROWS,
                    ) {
                        Some(s) => {
                            self.sessions.push(s);
                            let ws = WorkspaceTab::new(id, id, title);
                            self.workspaces.push(ws);
                            self.active_workspace_idx = self.workspaces.len() - 1;
                            self.active_session_id = id;
                            self.active_view = ActiveView::Terminal;
                            self.persist_sessions();
                        }
                        None => {
                            self.set_toast("Failed to attach to daemon session");
                        }
                    }
                }
                Err(e) => {
                    self.set_toast(format!("Daemon refused session: {}", e));
                }
            }
            return;
        }

        // Legacy in-process path.
        self.create_local_session_with_id(id, ctx, custom_dir, None);
        let ws = WorkspaceTab::new(id, id, format!("Local #{}", id));
        self.workspaces.push(ws);
        self.active_workspace_idx = self.workspaces.len() - 1;
        self.active_session_id = id;
        self.active_view = ActiveView::Terminal;
        self.persist_sessions();
    }

    pub fn spawn_ssh_terminal(&mut self, profile: &SshProfile, ctx: egui::Context) {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let title = format!("SSH: {}", profile.name);

        if let Some(ref daemon) = self.daemon {
            // Resolve identity-file + control-path on the GUI side so the
            // daemon doesn't need to know about SshStore.
            let identity_file = match &profile.auth_type {
                ssh::SshAuthType::KeyFile(p) => {
                    if p.trim().is_empty() {
                        None
                    } else {
                        SshStore::ensure_secure_permissions(p);
                        Some(p.clone())
                    }
                }
                ssh::SshAuthType::PastedKey { key_id } => {
                    let kp = SshStore::keys_dir().join(format!("{}.pem", key_id));
                    if kp.exists() {
                        let s = kp.to_string_lossy().to_string();
                        SshStore::ensure_secure_permissions(&s);
                        Some(s)
                    } else {
                        None
                    }
                }
                ssh::SshAuthType::PasswordOrAgent => None,
            };
            let control_path = SshStore::sockets_dir()
                .join(format!("{}.sock", profile.id))
                .to_string_lossy()
                .to_string();

            let spec = daemon::SshSpec {
                host: profile.host.clone(),
                port: profile.port,
                username: profile.username.clone(),
                identity_file,
                control_path: Some(control_path),
                profile_id: Some(profile.id.clone()),
            };

            match daemon.new_ssh(
                id as u64,
                &title,
                spec,
                daemon::DEFAULT_COLS,
                daemon::DEFAULT_ROWS,
            ) {
                Ok(()) => {
                    let session_type = SessionType::Ssh {
                        profile_id: profile.id.clone(),
                    };
                    match TerminalSession::new_daemon(
                        id,
                        title.clone(),
                        session_type,
                        daemon.clone(),
                        ctx.clone(),
                        self.settings.scrollback_lines,
                        daemon::DEFAULT_COLS,
                        daemon::DEFAULT_ROWS,
                    ) {
                        Some(s) => {
                            self.sessions.push(s);
                            let ws = WorkspaceTab::new(id, id, title);
                            self.workspaces.push(ws);
                            self.active_workspace_idx = self.workspaces.len() - 1;
                            self.active_session_id = id;
                            self.active_view = ActiveView::Terminal;
                            self.sftp
                                .right_pane
                                .set_target(SftpTarget::RemoteSsh(profile.clone()));
                            self.persist_sessions();
                        }
                        None => {
                            self.set_toast("Failed to attach to daemon session");
                        }
                    }
                }
                Err(e) => {
                    self.set_toast(format!("Daemon refused SSH session: {}", e));
                }
            }
            return;
        }

        // Legacy in-process path.
        self.create_ssh_session_with_id(profile, id, ctx, None);

        let ws = WorkspaceTab::new(id, id, title);
        self.workspaces.push(ws);
        self.active_workspace_idx = self.workspaces.len() - 1;
        self.active_session_id = id;
        self.active_view = ActiveView::Terminal;

        self.sftp
            .right_pane
            .set_target(SftpTarget::RemoteSsh(profile.clone()));
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

    /// Self-heal the workspace tree: drop leaves whose sessions no
    /// longer exist (daemon death, PTY exit, race during close),
    /// collapse single-child splits, drop empty workspaces, fix
    /// active indices, and spawn a fresh shell if the user has
    /// nothing left. Cheap; safe to call every frame.
    pub fn reconcile_workspaces(&mut self, ctx: egui::Context) -> bool {
        let live: std::collections::HashSet<usize> =
            self.sessions.iter().map(|s| s.id).collect();

        let before_lens: Vec<usize> =
            self.workspaces.iter().map(|w| w.root.leaves().len()).collect();
        let before_count = self.workspaces.len();
        let before_titles: Vec<String> =
            self.workspaces.iter().map(|w| w.title.clone()).collect();

        for ws in &mut self.workspaces {
            ws.root.prune(&live);
            if let Some(m) = ws.maximized_session {
                if !live.contains(&m) {
                    ws.maximized_session = None;
                }
            }
        }
        self.workspaces.retain(|ws| ws.root.contains_live(&live));

        // Refresh titles to reflect post-prune pane counts. A workspace
        // whose tree collapsed to a single leaf takes its session's
        // title; a multi-pane workspace keeps its "Tiled ..." prefix
        // but gets an accurate pane count. Without this, a tab that was
        // tiled then had panes closed keeps showing the old count —
        // e.g. "Tiled (3 Panes)" on a workspace that now has 1 leaf.
        let session_titles: Vec<(usize, String)> = self
            .sessions
            .iter()
            .map(|s| (s.id, s.title.clone()))
            .collect();
        for ws in &mut self.workspaces {
            let n = ws.root.leaves().len();
            if n == 1 {
                let sid = ws.root.first_leaf();
                if let Some((_, t)) = session_titles.iter().find(|(id, _)| *id == sid) {
                    ws.title = t.clone();
                }
            } else if n > 1 {
                if ws.title.starts_with("Tiled (") {
                    ws.title = format!("Tiled ({} Panes)", n);
                } else if ws.title.starts_with("Tiled Group ") {
                    if let Some(open) = ws.title.rfind('(') {
                        ws.title = format!("{} ({} Panes)", ws.title[..open].trim_end(), n);
                    }
                }
            }
        }

        let after_lens: Vec<usize> =
            self.workspaces.iter().map(|w| w.root.leaves().len()).collect();
        let after_titles: Vec<String> =
            self.workspaces.iter().map(|w| w.title.clone()).collect();
        let changed = before_lens != after_lens
            || before_count != self.workspaces.len()
            || before_titles != after_titles;

        if self.workspaces.is_empty() {
            if self.settings.open_default_tab {
                self.spawn_local_terminal(ctx, None);
            }
            return true;
        }

        if self.active_workspace_idx >= self.workspaces.len() {
            self.active_workspace_idx = self.workspaces.len() - 1;
        }
        let cur = &self.workspaces[self.active_workspace_idx];
        if !cur.root.leaves().contains(&self.active_session_id) {
            self.active_session_id = cur.root.first_leaf();
        }

        changed
    }

    /// Revive a dead session: tear down the exhausted PTY / daemon
    /// attachment and spawn a fresh shell or SSH connection under the
    /// SAME session id, so the tile tree and workspace layout keep
    /// working without any further bookkeeping.
    ///
    /// Called when the user presses any key inside a dead pane, or
    /// clicks the Reconnect overlay button.
    pub fn reconnect_session(&mut self, session_id: usize, ctx: egui::Context) {
        let info = self
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| (s.title.clone(), s.session_type.clone()));
        let (title, session_type) = match info {
            Some(i) => i,
            None => return,
        };

        // If this was daemon-backed, tell the daemon to drop the stale
        // session so the id is free for immediate reuse. Any attached
        // streaming socket closes as a side effect; the old
        // TerminalSession is about to be dropped anyway.
        if let Some(ref daemon) = self.daemon {
            let was_daemon = self
                .sessions
                .iter()
                .find(|s| s.id == session_id)
                .map(|s| s.is_daemon())
                .unwrap_or(false);
            if was_daemon {
                daemon.kill(session_id as u64);
            }
        }

        self.sessions.retain(|s| s.id != session_id);
        Database::delete_scrollback(session_id);

        // Respawn with the same id and title so the tile tree keeps
        // pointing at a live session.
        match session_type {
            SessionType::Local { working_dir } => {
                self.create_local_session_with_id(
                    session_id,
                    ctx,
                    Some(working_dir),
                    Some(title),
                );
            }
            SessionType::Ssh { profile_id } => {
                if let Some(profile) = self
                    .ssh_store
                    .profiles
                    .iter()
                    .find(|p| p.id == profile_id)
                    .cloned()
                {
                    self.create_ssh_session_with_id(
                        &profile,
                        session_id,
                        ctx,
                        Some(title),
                    );
                } else {
                    self.set_toast(format!(
                        "Cannot reconnect: SSH profile '{}' no longer exists",
                        profile_id
                    ));
                }
            }
        }

        self.active_session_id = session_id;
    }

    pub fn close_session(&mut self, session_id: usize, ctx: egui::Context) {
        if let Some(s) = self.sessions.iter().find(|s| s.id == session_id) {
            if let SessionType::Ssh { profile_id } = &s.session_type {
                let remaining_ssh_count = self.sessions.iter()
                    .filter(|other| other.id != session_id && matches!(&other.session_type, SessionType::Ssh { profile_id: p } if p == profile_id))
                    .count();
                if remaining_ssh_count == 0 {
                    let pid = profile_id.clone();
                    std::thread::spawn(move || {
                        SshStore::cleanup_stale_socket(&pid);
                    });
                }
            }
        }

        // If this session was daemon-backed, tell the daemon to tear
        // down the PTY and kill the child process.
        if let Some(ref daemon) = self.daemon {
            let was_daemon = self
                .sessions
                .iter()
                .find(|s| s.id == session_id)
                .map(|s| s.is_daemon())
                .unwrap_or(false);
            if was_daemon {
                daemon.kill(session_id as u64);
            }
        }

        Database::delete_scrollback(session_id);
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
            self.set_toast(format!("Tiled {} sessions across {} workspace tabs", all_sessions.len(), total_tabs_created));
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
        // TARGET SYNC: when the active session changes, point the SFTP
        // pane at the matching remote (if it's an SSH session). Runs
        // BEFORE the path sync so the path sync sees the correct target
        // on the same frame the tab is clicked.
        //
        // Ordering matters: the previous version ran the path sync first,
        // which updated `last_detected_dir` even though the SFTP target
        // was still Local. The target sync then reset the pane to its
        // default path, and the path sync never fired again because
        // `last_detected_dir` already matched — the pane was stuck at
        // the default until the user cd'd again.
        if self.active_view == ActiveView::Terminal
            && self.last_synced_session_id != Some(self.active_session_id)
        {
            self.last_synced_session_id = Some(self.active_session_id);

            // Clone the profile out so we don't hold an immutable borrow
            // on `self.sessions` while mutating `self.sftp`.
            let prof_opt: Option<SshProfile> = {
                let session = self
                    .sessions
                    .iter()
                    .find(|s| s.id == self.active_session_id);
                match session.map(|s| &s.session_type) {
                    Some(SessionType::Ssh { profile_id }) => self
                        .ssh_store
                        .profiles
                        .iter()
                        .find(|p| p.id == *profile_id)
                        .cloned(),
                    _ => None,
                }
            };
            if let Some(prof) = prof_opt {
                let target = SftpTarget::RemoteSsh(prof);
                if self.sftp.right_pane.target != target {
                    self.sftp.right_pane.set_target(target);
                }
            }
        }

        // PATH SYNC: follow the active shell's cwd into the SFTP pane.
        //
        // `last_detected_dir` is updated ONLY when the path is actually
        // applied. Otherwise a detection that fired while the target was
        // still Local (or while a directory listing was in flight)
        // would be silently swallowed and the next cd would be missed.
        if self.settings.sftp_path_sync {
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == self.active_session_id) {
                match &session.session_type {
                    SessionType::Local { .. } => {
                        if self.sftp.left_pane.target == SftpTarget::Local {
                            if let Some(detected_dir) = session.detect_current_working_dir(None) {
                                if session.last_detected_dir.as_deref() != Some(&detected_dir)
                                    && !self.sftp.left_pane.is_loading
                                {
                                    session.last_detected_dir = Some(detected_dir.clone());
                                    self.sftp.left_pane.set_path(detected_dir);
                                }
                            }
                        }
                    }
                    SessionType::Ssh { profile_id } => {
                        let profile_id = profile_id.clone();
                        let prof_opt = self
                            .ssh_store
                            .profiles
                            .iter()
                            .find(|p| p.id == profile_id)
                            .cloned();
                        if let Some(prof) = prof_opt {
                            if let Some(detected_dir) =
                                session.detect_current_working_dir(Some(&prof.username))
                            {
                                let target_matches = matches!(
                                    &self.sftp.right_pane.target,
                                    SftpTarget::RemoteSsh(p) if p.id == prof.id
                                );
                                if target_matches
                                    && !self.sftp.right_pane.is_loading
                                    && session.last_detected_dir.as_deref()
                                        != Some(&detected_dir)
                                {
                                    session.last_detected_dir = Some(detected_dir.clone());
                                    self.sftp.right_pane.set_path(detected_dir);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Turn off the background daemon at runtime. Kills every session
    /// the daemon owns, exits the daemon process, and drops our client
    /// handle. After this call, new sessions spawn in-process via the
    /// legacy path. Used by the "Keep Sessions Running in Background"
    /// toggle in Settings → Terminal Interaction.
    pub fn disable_daemon(&mut self, ctx: egui::Context) {
        // 1. Take the client handle. If we have one, send the graceful
        //    Shutdown request and hand it to a watchdog thread. The
        //    watchdog gives the daemon a short grace period to exit on
        //    its own; if the socket is still there afterwards, it
        //    SIGTERMs the daemon by name. That covers the case where
        //    the running daemon predates the Shutdown variant (protocol
        //    skew across an update) or has simply wedged.
        //
        //    Doing this on a background thread matters: the UI thread
        //    must not block for the poll loop, and it must not block on
        //    the socket write either.
        if let Some(daemon) = self.daemon.take() {
            daemon.shutdown();
            std::thread::spawn(move || {
                let sock = crate::daemon::socket_path();

                // Grace window: give the daemon up to 500 ms to exit
                // cleanly. It unlinks its own socket on the way out, so
                // socket-gone is our success signal.
                let deadline =
                    std::time::Instant::now() + std::time::Duration::from_millis(500);
                while sock.exists() && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }

                if sock.exists() {
                    // Daemon ignored Shutdown (old protocol, wedged, or
                    // stuck in a blocking PTY read). Force it. The match
                    // pattern deliberately requires "--daemon" so it can
                    // never hit the GUI process, which never has that
                    // flag on its own argv.
                    let _ = std::process::Command::new("pkill")
                        .args(["-TERM", "-f", "azterm.*--daemon"])
                        .output();

                    let deadline2 =
                        std::time::Instant::now() + std::time::Duration::from_millis(300);
                    while sock.exists() && std::time::Instant::now() < deadline2 {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }

                    // Last resort: SIGTERM was ignored or the process is
                    // a zombie whose parent hasn't reaped it yet. Remove
                    // the socket file ourselves so a fresh daemon can
                    // bind on the next launch.
                    if sock.exists() {
                        let _ = std::fs::remove_file(&sock);
                    }
                }
            });
        }

        // 2. Close every daemon-backed GUI session. close_session prunes
        //    the tile tree, removes scrollback files, and spawns a fresh
        //    local shell if that was the last workspace. Since
        //    `self.daemon` is None now, its internal daemon.kill() is
        //    skipped — the daemon is being torn down anyway.
        let ids: Vec<usize> = self
            .sessions
            .iter()
            .filter(|s| s.is_daemon())
            .map(|s| s.id)
            .collect();
        for id in ids {
            self.close_session(id, ctx.clone());
        }

        self.set_toast("Background daemon stopped. New sessions will run in-process.");
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
        // Tab is handled inside TerminalSession::handle_keyboard_events.
        // We set a focus-lock filter on the terminal widget so egui does
        // not consume Tab for its own focus navigation. No interception
        // is needed here anymore.

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
    /// Tell eframe what to clear the framebuffer to.
    ///
    /// Returning the theme's bg_main_color means the window surface
    /// itself becomes transparent when the user slides opacity down.
    /// Without this override, eframe clears to an opaque dark grey
    /// (its default), which is what shows through even on light themes
    /// at 0% opacity.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = self.theme.bg_main_color();
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
            c.a() as f32 / 255.0,
        ]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Flush session/layout state on window-close request so a divider
        // drag in the same tick as Alt+F4 still persists.
        if ctx.input(|i| i.viewport().close_requested()) {
            self.persist_sessions();
        }

        // Push the current theme into egui's global visuals so all
        // native widgets (Window, TextEdit, ComboBox dropdowns, popup
        // menus, ScrollArea backgrounds) pick up the same color scheme
        // as the rest of the UI. Cheap: mutates the style in place.
        self.theme.apply_to_egui(ctx);

        // If the user picked a different UI or terminal font in Settings,
        // rebuild egui's font atlas. This is the only place we call
        // set_fonts() after startup — it's expensive (full atlas rebuild)
        // so we guard it behind path comparisons.
        let ui_changed = self.applied_ui_font_path.as_deref()
            != Some(self.settings.ui_font_path.as_str());
        let term_changed = self.applied_terminal_font_path.as_deref()
            != Some(self.settings.terminal_font_path.as_str());
        // Also rebuild the atlas the first time the installed-font list
        // is populated, so the dropdown can render every item in its own
        // typeface.
        let previews_need_load = self.cached_fonts.is_some() && !self.preview_fonts_loaded;

        if ui_changed || term_changed || previews_need_load {
            let preview_list: Vec<fonts::FontEntry> =
                self.cached_fonts.clone().unwrap_or_default();
            fonts::apply_to_egui(
                ctx,
                &self.settings.ui_font_path,
                &self.settings.terminal_font_path,
                &preview_list,
            );
            self.applied_ui_font_path = Some(self.settings.ui_font_path.clone());
            self.applied_terminal_font_path = Some(self.settings.terminal_font_path.clone());
            if previews_need_load {
                self.preview_fonts_loaded = true;
            }
        }

        // Reconcile debug logging with current settings (cheap no-op if unchanged).
        debug_log::init(self.settings.debug_mode, &self.settings.debug_log_path);

        // Heartbeat: proves the UI thread is still ticking. If the app
        // ever locks up, the log will simply stop emitting these every
        // 5 seconds — that timestamp is where the freeze began.
        if self.settings.debug_mode
            && self.last_heartbeat.elapsed() >= std::time::Duration::from_secs(5)
        {
            self.last_heartbeat = std::time::Instant::now();
            debug_log::log(format!(
                "heartbeat view={:?} sess={} ws={} modal={} dragging_pane={:?} dragging_tab={:?} sftp_modal={}",
                self.active_view,
                self.active_session_id,
                self.active_workspace_idx,
                modal_open_flag(self),
                self.dragging_pane_id,
                self.dragging_tab_idx,
                self.sftp.has_open_modal(),
            ));
        }

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

        if self.ssh_auth_modal.is_some() {
            ui::modals::render_ssh_auth_modal(self, ctx);
        }

        if self.show_disable_daemon_modal {
            ui::modals::render_disable_daemon_modal(self, ctx);
        }

        self.sftp.render_transfer_history_window(ctx, &self.theme);

        let modal_open = self.show_update_modal
            || self.show_profile_modal
            || self.show_keygen_modal
            || self.ssh_auth_modal.is_some()
            || self.sftp.has_open_modal();

        if self.active_view == ActiveView::Terminal && !modal_open {
            self.handle_terminal_shortcuts(ctx);
        }

        for s in &mut self.sessions {
            s.poll_updates();
        }

        // Reconcile every frame: catches daemon deaths, PTY exits, and
        // any drift where a workspace still lists a leaf whose session
        // has been dropped. Fixes stale "(N Panes)" tab labels and the
        // "can't close tab" symptom that comes from a tree with
        // phantom leaves.
        if self.reconcile_workspaces(ctx.clone()) {
            self.persist_sessions();
        }

        if let Some(ref rx) = self.update_rx {
            if let Ok(res) = rx.try_recv() {
                self.is_checking_update = false;
                match res {
                    UpdateCheckResult::NewVersion(tag) => {
                        self.available_update = Some(tag.clone());
                        self.settings.pending_update = Some(tag);
                        self.settings.save();
                    }
                    UpdateCheckResult::AlreadyUpToDate => {
                        self.available_update = None;
                        self.settings.pending_update = None;
                        self.settings.save();
                    }
                    UpdateCheckResult::CheckFailed => {}
                }
            }
        }

        self.sync_sftp_with_active_session();

        ui::navbar::render_top_nav(self, ctx);
        ui::navbar::render_tabs_bar(self, ctx);
        ui::navbar::render_status_bar(self, ctx);

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.theme.bg_main_color()))
            .show(ctx, |ui| {
                let content_rect = ui.max_rect();

                match self.active_view {
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
                }

                // Transition overlay: fade mask + accent sweep. Painted
                // AFTER content so it covers whatever just rendered and
                // dissolves to reveal it.
                if let Some(t0) = self.last_transition {
                    const DURATION_SECS: f32 = 0.24;
                    let elapsed = t0.elapsed().as_secs_f32();
                    if elapsed >= DURATION_SECS {
                        self.last_transition = None;
                    } else {
                        let progress = elapsed / DURATION_SECS;
                        crate::modern::transition_overlay(
                            ui.painter(),
                            content_rect,
                            self.theme.bg_main_color(),
                            self.theme.accent_color(),
                            progress,
                        );
                        ctx.request_repaint();
                    }
                }
            });
    }
}

fn main() -> eframe::Result<()> {
    // Single-binary daemon: when we re-exec ourselves with --daemon we
    // take the daemon path before any GUI startup work (settings load,
    // panic hook, argument parsing). This is what makes the install
    // story one file — the same azterm binary is both the GUI and the
    // background process it spawns.
    if std::env::args().nth(1).as_deref() == Some("--daemon") {
        daemon_server::run();
        return Ok(());
    }

    let cli_opts = parse_cli_arguments();
    let initial_settings = AppSettings::load();

    // Install panic hook FIRST so any panic during startup is captured.
    debug_log::install_panic_hook();
    debug_log::init(initial_settings.debug_mode, &initial_settings.debug_log_path);

    if initial_settings.debug_mode {
        debug_log::log(format!(
            "boot: debug_mode=on log_path={} version={}",
            initial_settings.debug_log_path,
            env!("CARGO_PKG_VERSION"),
        ));
    }

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
