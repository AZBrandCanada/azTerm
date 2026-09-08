mod db;
mod settings;
mod sftp;
mod ssh;
mod terminal;
mod theme;

use db::{Database, SavedSessionState};
use eframe::egui;
use portable_pty::CommandBuilder;
use settings::{AppSettings, BackspaceSequence};
use sftp::{SftpManager, SftpTarget};
use ssh::{SshAuthType, SshProfile, SshStore};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use terminal::{SessionType, TerminalSession};
use theme::*;

#[derive(PartialEq, Eq)]
enum ActiveView {
    Terminal,
    SshBookmarks,
    SftpBrowser,
    Settings,
}

#[derive(PartialEq, Eq)]
enum SshSubView {
    Profiles,
    KeysManager,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum SettingsCategory {
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

struct AppState {
    settings: AppSettings,
    ssh_store: SshStore,
    sessions: Vec<TerminalSession>,
    sftp: SftpManager,
    active_tab_idx: usize,
    next_tab_id: usize,
    active_view: ActiveView,
    ssh_subview: SshSubView,
    settings_category: SettingsCategory,

    toast_message: Option<(String, std::time::Instant)>,

    available_update: Option<String>,
    update_rx: Option<Receiver<Option<String>>>,
    is_checking_update: bool,
    show_update_modal: bool,
    install_method: InstallMethod,

    show_profile_modal: bool,
    editing_profile_id: Option<String>,
    new_ssh_name: String,
    new_ssh_host: String,
    new_ssh_port: String,
    new_ssh_user: String,
    new_ssh_auth_choice: usize,
    new_ssh_key_path: String,
    new_ssh_pasted_key: String,

    show_keygen_modal: bool,
    keygen_name: String,
    generated_pub_key: String,
    keygen_status: String,
}

impl AppState {
    fn new(cc: &eframe::CreationContext<'_>, cli: CliLaunchOptions) -> Self {
        let settings = AppSettings::load();
        let ssh_store = SshStore::load();
        let install_method = InstallMethod::detect();

        let mut app = Self {
            settings,
            ssh_store,
            sessions: Vec::new(),
            sftp: SftpManager::new(),
            active_tab_idx: 0,
            next_tab_id: 1,
            active_view: if cli.open_sftp_only { ActiveView::SftpBrowser } else { ActiveView::Terminal },
            ssh_subview: SshSubView::Profiles,
            settings_category: SettingsCategory::Terminal,
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

    fn trigger_update_check(&mut self, force: bool, ctx: egui::Context) {
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

    fn run_script_update_in_terminal(&mut self, ctx: egui::Context) {
        let update_cmd = "curl -sSL https://raw.githubusercontent.com/AZBrandCanada/azTerm/main/install.sh | bash\n";
        self.spawn_local_terminal(ctx, None);

        if let Some(active_session) = self.sessions.get_mut(self.active_tab_idx) {
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

    fn set_toast(&mut self, text: impl Into<String>) {
        self.toast_message = Some((text.into(), std::time::Instant::now()));
    }

    fn persist_sessions(&self) {
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
        }
    }

    fn spawn_local_terminal(&mut self, ctx: egui::Context, custom_dir: Option<String>) {
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
        );
        self.sessions.push(session);
        self.active_tab_idx = self.sessions.len() - 1;
        self.active_view = ActiveView::Terminal;
        self.persist_sessions();
    }

    fn spawn_ssh_terminal(&mut self, profile: &SshProfile, ctx: egui::Context) {
        let cmd = profile.to_command();
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let session = TerminalSession::new(
            id,
            format!("SSH: {}", profile.name),
            SessionType::Ssh { profile_id: profile.id.clone() },
            cmd,
            ctx,
        );
        self.sessions.push(session);
        self.active_tab_idx = self.sessions.len() - 1;
        self.active_view = ActiveView::Terminal;

        self.sftp.right_pane.set_target(SftpTarget::RemoteSsh(profile.clone()));
        self.persist_sessions();
    }

    fn sync_sftp_with_active_session(&mut self) {
        if let Some(session) = self.sessions.get(self.active_tab_idx) {
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

    fn open_create_profile_modal(&mut self) {
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

    fn open_edit_profile_modal(&mut self, profile: &SshProfile) {
        self.editing_profile_id = Some(profile.id.clone());
        self.new_ssh_name = profile.name.clone();
        self.new_ssh_host = profile.host.clone();
        self.new_ssh_port = profile.port.to_string();
        self.new_ssh_user = profile.username.clone();

        match &profile.auth_type {
            SshAuthType::PasswordOrAgent => {
                self.new_ssh_auth_choice = 0;
                self.new_ssh_key_path.clear();
                self.new_ssh_pasted_key.clear();
            }
            SshAuthType::KeyFile(path) => {
                self.new_ssh_auth_choice = 1;
                self.new_ssh_key_path = path.clone();
                self.new_ssh_pasted_key.clear();
            }
            SshAuthType::PastedKey { key_id } => {
                self.new_ssh_auth_choice = 2;
                self.new_ssh_key_path.clear();
                let key_path = SshStore::keys_dir().join(format!("{}.pem", key_id));
                self.new_ssh_pasted_key = std::fs::read_to_string(key_path).unwrap_or_default();
            }
        }
        self.show_profile_modal = true;
    }
}

impl eframe::App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

        // Top Navigation Bar
        egui::TopBottomPanel::top("top_nav")
            .frame(egui::Frame::none().fill(COLOR_BG_PANEL).inner_margin(egui::Margin::symmetric(14.0, 8.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("AZTerm")
                            .color(COLOR_ACCENT)
                            .strong()
                            .size(16.0),
                    );
                    ui.add_space(8.0);

                    if ui.selectable_label(self.active_view == ActiveView::Terminal, "Terminal").clicked() {
                        self.active_view = ActiveView::Terminal;
                    }
                    if ui.selectable_label(self.active_view == ActiveView::SshBookmarks, "SSH Profiles").clicked() {
                        self.active_view = ActiveView::SshBookmarks;
                    }
                    if ui.selectable_label(self.active_view == ActiveView::SftpBrowser, "SFTP Explorer").clicked() {
                        self.active_view = ActiveView::SftpBrowser;
                    }
                    if ui.selectable_label(self.active_view == ActiveView::Settings, "Settings").clicked() {
                        self.active_view = ActiveView::Settings;
                    }

                    ui.separator();

                    if ui.button("+ New Shell").clicked() {
                        self.spawn_local_terminal(ctx.clone(), None);
                    }

                    ui.separator();

                    let mut tab_to_close: Option<usize> = None;
                    let avail_w = (ui.available_width() - 16.0).max(80.0);
                    let num_tabs = self.sessions.len().max(1) as f32;
                    let computed_tab_width = ((avail_w / num_tabs) - 6.0).clamp(65.0, 160.0);
                    let max_chars = ((computed_tab_width - 26.0) / 7.2).max(3.0) as usize;

                    egui::ScrollArea::horizontal()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for (i, session) in self.sessions.iter().enumerate() {
                                    let is_active = self.active_view == ActiveView::Terminal && self.active_tab_idx == i;
                                    let tab_bg = if is_active { COLOR_BG_CARD } else { COLOR_BG_MAIN };

                                    egui::Frame::none()
                                        .fill(tab_bg)
                                        .stroke(egui::Stroke::new(1.0_f32, if is_active { COLOR_ACCENT } else { COLOR_BORDER }))
                                        .rounding(4.0)
                                        .inner_margin(egui::Margin::symmetric(6.0, 4.0))
                                        .show(ui, |ui| {
                                            ui.set_width(computed_tab_width);
                                            ui.horizontal(|ui| {
                                                let label_text = if session.title.len() > max_chars {
                                                    format!("{}...", &session.title[..max_chars.saturating_sub(3)])
                                                } else {
                                                    session.title.clone()
                                                };
                                                if ui.selectable_label(is_active, label_text).clicked() {
                                                    self.active_tab_idx = i;
                                                    self.active_view = ActiveView::Terminal;

                                                    if let SessionType::Ssh { profile_id } = &session.session_type {
                                                        if let Some(prof) = self.ssh_store.profiles.iter().find(|p| p.id == *profile_id) {
                                                            self.sftp.right_pane.set_target(SftpTarget::RemoteSsh(prof.clone()));
                                                        }
                                                    }
                                                }
                                                if self.sessions.len() > 1 && ui.small_button("×").clicked() {
                                                    tab_to_close = Some(i);
                                                }
                                            });
                                        });
                                    ui.add_space(3.0);
                                }
                            });
                        });

                    if let Some(i) = tab_to_close {
                        self.sessions.remove(i);
                        if self.active_tab_idx >= self.sessions.len() && !self.sessions.is_empty() {
                            self.active_tab_idx = self.sessions.len() - 1;
                        }
                        self.persist_sessions();
                    }
                });
            });

        // Bottom Status Bar
        egui::TopBottomPanel::bottom("bottom_status_bar")
            .frame(egui::Frame::none().fill(COLOR_BG_PANEL).inner_margin(egui::Margin::symmetric(14.0, 4.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(session) = self.sessions.get(self.active_tab_idx) {
                        let info = match &session.session_type {
                            SessionType::Local { working_dir } => format!("Local Shell: {}", working_dir),
                            SessionType::Ssh { profile_id } => format!("SSH Target: {}", profile_id),
                        };
                        ui.label(egui::RichText::new(info).small().color(COLOR_TEXT_MUTED));
                    }

                    ui.separator();

                    let sftp_btn_text = if self.settings.show_sftp_split_view {
                        "SFTP Drawer: OPEN"
                    } else {
                        "SFTP Drawer: CLOSED"
                    };
                    if ui.selectable_label(self.settings.show_sftp_split_view, sftp_btn_text).clicked() {
                        self.settings.show_sftp_split_view = !self.settings.show_sftp_split_view;
                        self.settings.save();
                        self.set_toast(if self.settings.show_sftp_split_view {
                            "SFTP split panel opened"
                        } else {
                            "SFTP split panel closed"
                        });
                    }

                    if self.settings.check_updates {
                        if let Some(ref update_tag) = self.available_update {
                            ui.separator();
                            let btn = egui::Button::new(
                                egui::RichText::new(format!("⭐ Update: {}", update_tag))
                                    .small()
                                    .strong()
                                    .color(COLOR_ACCENT),
                            )
                            .fill(COLOR_BG_CARD)
                            .stroke(egui::Stroke::new(1.0, COLOR_ACCENT));

                            if ui.add(btn).on_hover_text(format!("Click to view update options for {}", update_tag)).clicked() {
                                self.show_update_modal = true;
                            }
                        }
                    }

                    if let Some(ref status) = self.sftp.transfer_status {
                        ui.separator();
                        ui.label(egui::RichText::new(status).small().color(COLOR_ACCENT));
                    }

                    if let Some((msg, time)) = &self.toast_message {
                        if time.elapsed().as_secs_f32() < 3.0 {
                            ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                                egui::Frame::none()
                                    .fill(COLOR_BG_CARD)
                                    .stroke(egui::Stroke::new(1.0_f32, COLOR_ACCENT))
                                    .rounding(4.0)
                                    .inner_margin(egui::Margin::symmetric(12.0, 2.0))
                                    .show(ui, |ui| {
                                        ui.label(egui::RichText::new(msg).color(COLOR_ACCENT).strong().small());
                                    });
                            });
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(session) = self.sessions.get(self.active_tab_idx) {
                            ui.label(egui::RichText::new(format!("{}x{}", session.cols, session.rows)).small().color(COLOR_TEXT_MUTED));
                        }
                    });
                });
            });

        // Update Confirmation Modal
        if self.show_update_modal {
            if let Some(ref new_tag) = self.available_update.clone() {
                egui::Window::new("AZTerm Update Available")
                    .collapsible(false)
                    .resizable(false)
                    .default_width(460.0)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(format!("A new version ({}) of AZTerm is ready!", new_tag))
                                    .strong()
                                    .size(15.0)
                                    .color(COLOR_ACCENT),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("Current version: v{}", env!("CARGO_PKG_VERSION")))
                                    .small()
                                    .color(COLOR_TEXT_MUTED),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("Detected installation: {}", self.install_method.display_name()))
                                    .small()
                                    .color(COLOR_TEXT_PRIMARY),
                            );

                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(10.0);

                            match &self.install_method {
                                InstallMethod::ScriptInstalled | InstallMethod::PackageManager(_) => {
                                    ui.label("Would you like to run the official updater script in a new terminal session?");
                                    ui.add_space(6.0);
                                    egui::Frame::none()
                                        .fill(COLOR_BG_PANEL)
                                        .rounding(4.0)
                                        .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                                        .show(ui, |ui| {
                                            ui.monospace("curl -sSL https://raw.githubusercontent.com/AZBrandCanada/azTerm/main/install.sh | bash");
                                        });
                                }
                                InstallMethod::AppImage => {
                                    ui.label("Download the latest standalone AppImage binary from GitHub:");
                                }
                                InstallMethod::Windows => {
                                    ui.label("Download the latest Windows ZIP archive from GitHub:");
                                }
                                InstallMethod::MacOS => {
                                    ui.label("Download the latest macOS universal package from GitHub:");
                                }
                                InstallMethod::ManualBuild => {
                                    ui.label("You can recompile with cargo or run the installer script:");
                                }
                            }

                            ui.add_space(14.0);
                            ui.horizontal(|ui| {
                                match &self.install_method {
                                    InstallMethod::ScriptInstalled | InstallMethod::ManualBuild => {
                                        if ui.button(egui::RichText::new("Update Now (Run in Shell)").strong()).clicked() {
                                            self.run_script_update_in_terminal(ctx.clone());
                                        }
                                    }
                                    _ => {}
                                }

                                let release_url = format!("https://github.com/AZBrandCanada/azTerm/releases/tag/{}", new_tag);
                                if ui.button("Open GitHub Release").clicked() {
                                    ctx.open_url(egui::OpenUrl::new_tab(release_url));
                                    self.show_update_modal = false;
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("Later").clicked() {
                                        self.show_update_modal = false;
                                    }
                                });
                            });
                        });
                    });
            }
        }

        // Keygen Modal Window
        if self.show_keygen_modal {
            egui::Window::new("Generate Ed25519 SSH Keypair")
                .collapsible(false)
                .resizable(true)
                .default_width(520.0)
                .max_height(480.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label("Key identifier name:");
                                ui.text_edit_singleline(&mut self.keygen_name);
                                ui.add_space(8.0);

                                if ui.button("Generate Keypair").clicked() {
                                    match SshStore::generate_ed25519_keypair(&self.keygen_name) {
                                        Ok((priv_path, pub_key)) => {
                                            self.generated_pub_key = pub_key;
                                            self.keygen_status = format!("Key generated and saved to: {}", priv_path);
                                        }
                                        Err(e) => {
                                            self.keygen_status = format!("Error: {}", e);
                                        }
                                    }
                                }

                                if !self.generated_pub_key.is_empty() {
                                    ui.add_space(8.0);
                                    ui.label(egui::RichText::new("Public Key (Paste into remote ~/.ssh/authorized_keys):").strong());
                                    ui.add(
                                        egui::TextEdit::multiline(&mut self.generated_pub_key)
                                            .desired_rows(5)
                                            .desired_width(f32::INFINITY),
                                    );
                                    if ui.button("Copy Public Key to Clipboard").clicked() {
                                        if let Ok(mut cb) = arboard::Clipboard::new() {
                                            let _ = cb.set_text(self.generated_pub_key.clone());
                                            self.set_toast("Public key copied to clipboard");
                                        }
                                    }
                                }

                                if !self.keygen_status.is_empty() {
                                    ui.add_space(6.0);
                                    ui.label(&self.keygen_status);
                                }
                            });

                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("Close").clicked() {
                                self.show_keygen_modal = false;
                            }
                        });
                    });
                });
        }

        // Profile Modal Window
        if self.show_profile_modal {
            let modal_title = if self.editing_profile_id.is_some() {
                "Edit SSH Profile"
            } else {
                "Create New SSH Profile"
            };

            egui::Window::new(modal_title)
                .collapsible(false)
                .resizable(true)
                .default_width(540.0)
                .max_height(540.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical()
                            .max_height(420.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                egui::Grid::new("profile_grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                                    ui.label("Profile Name:");
                                    ui.text_edit_singleline(&mut self.new_ssh_name);
                                    ui.end_row();

                                    ui.label("Host / IP:");
                                    ui.text_edit_singleline(&mut self.new_ssh_host);
                                    ui.end_row();

                                    ui.label("Port:");
                                    ui.text_edit_singleline(&mut self.new_ssh_port);
                                    ui.end_row();

                                    ui.label("Username:");
                                    ui.text_edit_singleline(&mut self.new_ssh_user);
                                    ui.end_row();

                                    ui.label("Authentication:");
                                    ui.horizontal(|ui| {
                                        ui.radio_value(&mut self.new_ssh_auth_choice, 0, "Password / Agent");
                                        ui.radio_value(&mut self.new_ssh_auth_choice, 1, "Key File");
                                        ui.radio_value(&mut self.new_ssh_auth_choice, 2, "Paste Key");
                                    });
                                    ui.end_row();

                                    if self.new_ssh_auth_choice == 1 {
                                        ui.label("Key File Path:");
                                        ui.text_edit_singleline(&mut self.new_ssh_key_path);
                                        ui.end_row();
                                    } else if self.new_ssh_auth_choice == 2 {
                                        ui.label("Paste Private Key:");
                                        ui.add(
                                            egui::TextEdit::multiline(&mut self.new_ssh_pasted_key)
                                                .desired_rows(6)
                                                .desired_width(f32::INFINITY)
                                                .hint_text("-----BEGIN OPENSSH PRIVATE KEY-----\n..."),
                                        );
                                        ui.end_row();
                                    }
                                });
                            });

                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("Save Profile").clicked() {
                                let port = self.new_ssh_port.parse().unwrap_or(22);
                                let auth_type = if self.new_ssh_auth_choice == 1 && !self.new_ssh_key_path.trim().is_empty() {
                                    SshStore::ensure_secure_permissions(&self.new_ssh_key_path);
                                    SshAuthType::KeyFile(self.new_ssh_key_path.clone())
                                } else if self.new_ssh_auth_choice == 2 && !self.new_ssh_pasted_key.trim().is_empty() {
                                    let key_id = format!("{}_{}", self.new_ssh_host, port);
                                    let _ = SshStore::save_pasted_key(&key_id, &self.new_ssh_pasted_key);
                                    SshAuthType::PastedKey { key_id }
                                } else {
                                    SshAuthType::PasswordOrAgent
                                };

                                if let Some(ref edit_id) = self.editing_profile_id {
                                    if let Some(existing) = self.ssh_store.profiles.iter_mut().find(|p| p.id == *edit_id) {
                                        existing.name = self.new_ssh_name.clone();
                                        existing.host = self.new_ssh_host.clone();
                                        existing.port = port;
                                        existing.username = self.new_ssh_user.clone();
                                        existing.auth_type = auth_type;
                                    }
                                    self.set_toast("SSH Profile Updated");
                                } else {
                                    let mut profile = SshProfile::new(&self.new_ssh_name, &self.new_ssh_host, port, &self.new_ssh_user);
                                    profile.auth_type = auth_type;
                                    self.ssh_store.profiles.push(profile);
                                    self.set_toast("SSH Profile Created");
                                }

                                self.ssh_store.save();
                                self.show_profile_modal = false;
                            }
                            if ui.button("Cancel").clicked() {
                                self.show_profile_modal = false;
                            }
                        });
                    });
                });
        }

        // Central Workspace Area
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(COLOR_BG_MAIN))
            .show(ctx, |ui| match self.active_view {
                ActiveView::Terminal => {
                    let mut toast = self.toast_message.clone();
                    if self.settings.show_sftp_split_view {
                        ui.columns(2, |columns| {
                            if let Some(session) = self.sessions.get_mut(self.active_tab_idx) {
                                session.render(&mut columns[0], &self.settings, &mut toast);
                            }
                            card_frame().show(&mut columns[1], |ui| {
                                ui.horizontal(|ui| {
                                    ui.heading("SFTP Sync Pane");
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.button("Upload Selected").clicked() {
                                            self.sftp.upload_selected();
                                        }
                                    });
                                });
                                ui.separator();
                                self.sftp.right_pane.render(ui);
                            });
                        });
                    } else if let Some(session) = self.sessions.get_mut(self.active_tab_idx) {
                        session.render(ui, &self.settings, &mut toast);
                    } else {
                        ui.centered_and_justified(|ui| {
                            if ui.button("Open Shell Session").clicked() {
                                self.spawn_local_terminal(ctx.clone(), None);
                            }
                        });
                    }
                    self.toast_message = toast;
                }
                ActiveView::SshBookmarks => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            ui.heading(egui::RichText::new("SSH Manager").color(COLOR_TEXT_PRIMARY));
                            ui.add_space(16.0);
                            if ui.selectable_label(self.ssh_subview == SshSubView::Profiles, "Connections").clicked() {
                                self.ssh_subview = SshSubView::Profiles;
                            }
                            if ui.selectable_label(self.ssh_subview == SshSubView::KeysManager, "Saved Keypairs").clicked() {
                                self.ssh_subview = SshSubView::KeysManager;
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Key Generator").clicked() {
                                    self.show_keygen_modal = true;
                                }
                                if ui.button("+ New SSH Profile").clicked() {
                                    self.open_create_profile_modal();
                                }
                            });
                        });

                        ui.add_space(12.0);

                        match self.ssh_subview {
                            SshSubView::Profiles => {
                                let profiles = self.ssh_store.profiles.clone();
                                let mut delete_idx: Option<usize> = None;
                                let mut connect_profile: Option<SshProfile> = None;
                                let mut edit_profile: Option<SshProfile> = None;
                                let mut sftp_profile: Option<SshProfile> = None;

                                for (idx, profile) in profiles.iter().enumerate() {
                                    card_frame().show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.vertical(|ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(egui::RichText::new(&profile.name).strong().size(15.0).color(COLOR_TEXT_PRIMARY));
                                                    let auth_badge = match &profile.auth_type {
                                                        SshAuthType::PasswordOrAgent => "[Password/Agent]",
                                                        SshAuthType::KeyFile(_) => "[Key File]",
                                                        SshAuthType::PastedKey { .. } => "[Inline Key]",
                                                    };
                                                    ui.label(egui::RichText::new(auth_badge).color(COLOR_ACCENT).small());
                                                });
                                                ui.label(
                                                    egui::RichText::new(format!("{}@{}:{}", profile.username, profile.host, profile.port))
                                                        .color(COLOR_TEXT_MUTED),
                                                );
                                            });

                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if ui.button("Delete").clicked() {
                                                    delete_idx = Some(idx);
                                                }
                                                if ui.button("Edit").clicked() {
                                                    edit_profile = Some(profile.clone());
                                                }
                                                if ui.button("SFTP").clicked() {
                                                    sftp_profile = Some(profile.clone());
                                                }
                                                if ui.button("Connect").clicked() {
                                                    connect_profile = Some(profile.clone());
                                                }
                                            });
                                        });
                                    });
                                    ui.add_space(8.0);
                                }

                                if let Some(i) = delete_idx {
                                    self.ssh_store.profiles.remove(i);
                                    self.ssh_store.save();
                                }
                                if let Some(p) = edit_profile {
                                    self.open_edit_profile_modal(&p);
                                }
                                if let Some(p) = sftp_profile {
                                    self.sftp.right_pane.set_target(SftpTarget::RemoteSsh(p));
                                    self.active_view = ActiveView::SftpBrowser;
                                }
                                if let Some(profile) = connect_profile {
                                    self.spawn_ssh_terminal(&profile, ctx.clone());
                                }
                            }
                            SshSubView::KeysManager => {
                                let saved_keys = SshStore::list_saved_keys();
                                if saved_keys.is_empty() {
                                    card_frame().show(ui, |ui| {
                                        ui.label(egui::RichText::new("No SSH keys stored in ~/.config/azterm/keys yet.").color(COLOR_TEXT_MUTED));
                                    });
                                } else {
                                    let mut key_to_delete: Option<String> = None;

                                    for key in saved_keys {
                                        card_frame().show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new(&key.file_name).strong().color(COLOR_ACCENT));
                                                    ui.label(egui::RichText::new(format!("Path: {}", key.priv_path.display())).small().color(COLOR_TEXT_MUTED));
                                                });

                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if ui.button("Delete Key").clicked() {
                                                        key_to_delete = Some(key.file_name.clone());
                                                    }
                                                    if let Some(ref pub_k) = key.pub_key_content {
                                                        if ui.button("Copy Public Key").clicked() {
                                                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                                                let _ = cb.set_text(pub_k.clone());
                                                                self.set_toast("Public key copied to clipboard");
                                                            }
                                                        }
                                                    }
                                                });
                                            });
                                        });
                                        ui.add_space(8.0);
                                    }

                                    if let Some(name) = key_to_delete {
                                        SshStore::delete_key_files(&name);
                                        self.set_toast("Key files removed");
                                    }
                                }
                            }
                        }
                    });
                }
                ActiveView::SftpBrowser => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(10.0);

                        card_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Dual-Session SFTP File Transfer").strong().color(COLOR_ACCENT));

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("Download (Right -> Left)").clicked() {
                                        self.sftp.download_selected();
                                    }
                                    if ui.button("Upload (Left -> Right)").clicked() {
                                        self.sftp.upload_selected();
                                    }
                                });
                            });
                        });

                        ui.add_space(10.0);

                        ui.columns(2, |cols| {
                            card_frame().show(&mut cols[0], |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Left Pane:").strong());
                                    let is_local = self.sftp.left_pane.target == SftpTarget::Local;
                                    egui::ComboBox::from_id_source("left_pane_target_combo")
                                        .selected_text(if is_local { "Local Filesystem" } else { "Remote SSH Target" })
                                        .show_ui(ui, |ui| {
                                            if ui.selectable_label(is_local, "Local Filesystem").clicked() {
                                                self.sftp.left_pane.set_target(SftpTarget::Local);
                                            }
                                            for p in &self.ssh_store.profiles {
                                                let is_this = self.sftp.left_pane.target == SftpTarget::RemoteSsh(p.clone());
                                                if ui.selectable_label(is_this, format!("SSH: {}", p.name)).clicked() {
                                                    self.sftp.left_pane.set_target(SftpTarget::RemoteSsh(p.clone()));
                                                }
                                            }
                                        });
                                });
                                ui.separator();
                                self.sftp.left_pane.render(ui);
                            });

                            card_frame().show(&mut cols[1], |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Right Pane:").strong());
                                    let right_desc = match &self.sftp.right_pane.target {
                                        SftpTarget::Local => "Local Filesystem".to_string(),
                                        SftpTarget::RemoteSsh(p) => format!("SSH: {}", p.name),
                                    };
                                    egui::ComboBox::from_id_source("right_pane_target_combo")
                                        .selected_text(right_desc)
                                        .show_ui(ui, |ui| {
                                            let is_local = self.sftp.right_pane.target == SftpTarget::Local;
                                            if ui.selectable_label(is_local, "Local Filesystem").clicked() {
                                                self.sftp.right_pane.set_target(SftpTarget::Local);
                                            }
                                            for p in &self.ssh_store.profiles {
                                                let is_this = self.sftp.right_pane.target == SftpTarget::RemoteSsh(p.clone());
                                                if ui.selectable_label(is_this, format!("SSH: {}", p.name)).clicked() {
                                                    self.sftp.right_pane.set_target(SftpTarget::RemoteSsh(p.clone()));
                                                }
                                            }
                                        });
                                });
                                ui.separator();
                                self.sftp.right_pane.render(ui);
                            });
                        });
                    });
                }
                ActiveView::Settings => {
                    ui.columns(2, |columns| {
                        columns[0].set_max_width(210.0);
                        columns[0].vertical(|ui| {
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new("Preferences").strong().size(16.0).color(COLOR_TEXT_PRIMARY));
                            ui.add_space(12.0);

                            let nav_item = |ui: &mut egui::Ui, cat: SettingsCategory, label: &str, current: SettingsCategory| -> bool {
                                let is_active = current == cat;
                                let bg = if is_active { COLOR_BG_CARD } else { egui::Color32::TRANSPARENT };
                                let stroke = if is_active { egui::Stroke::new(1.0_f32, COLOR_ACCENT) } else { egui::Stroke::NONE };

                                egui::Frame::none()
                                    .fill(bg)
                                    .stroke(stroke)
                                    .rounding(4.0)
                                    .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                                    .show(ui, |ui| {
                                        ui.set_width(180.0);
                                        let text_color = if is_active { COLOR_ACCENT } else { COLOR_TEXT_PRIMARY };
                                        ui.selectable_label(is_active, egui::RichText::new(label).color(text_color)).clicked()
                                    }).inner
                            };

                            if nav_item(ui, SettingsCategory::Terminal, "Terminal Interaction", self.settings_category) {
                                self.settings_category = SettingsCategory::Terminal;
                            }
                            ui.add_space(4.0);
                            if nav_item(ui, SettingsCategory::ShellEnv, "Shell & Environment", self.settings_category) {
                                self.settings_category = SettingsCategory::ShellEnv;
                            }
                            ui.add_space(4.0);
                            if nav_item(ui, SettingsCategory::Sftp, "SFTP & Transfers", self.settings_category) {
                                self.settings_category = SettingsCategory::Sftp;
                            }
                            ui.add_space(4.0);
                            if nav_item(ui, SettingsCategory::System, "Application & System", self.settings_category) {
                                self.settings_category = SettingsCategory::System;
                            }

                            ui.add_space(20.0);
                            ui.separator();
                            ui.add_space(8.0);
                            if ui.button("Restore Defaults").clicked() {
                                self.settings = AppSettings::default();
                                self.settings.save();
                                self.set_toast("Defaults Restored");
                            }
                        });

                        columns[1].vertical(|ui| {
                            ui.add_space(10.0);
                            let mut changed = false;

                            card_frame().show(ui, |ui| {
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    match self.settings_category {
                                        SettingsCategory::Terminal => {
                                            ui.label(egui::RichText::new("Terminal Interaction").strong().size(16.0).color(COLOR_ACCENT));
                                            ui.label(egui::RichText::new("Configure mouse behavior, clipboard actions, and visual cues.").small().color(COLOR_TEXT_MUTED));
                                            ui.add_space(12.0);

                                            changed |= setting_row_toggle(ui, "Cursor Blink", "Animate cursor blinking in the active terminal buffer.", &mut self.settings.cursor_blink);
                                            changed |= setting_row_toggle(ui, "Copy Selected Text on Select", "Automatically copy highlighted text to OS clipboard on drag release.", &mut self.settings.copy_on_select);
                                            changed |= setting_row_toggle(ui, "Paste on Right Click", "Immediately write clipboard text into the terminal on right click.", &mut self.settings.paste_on_right_click);
                                            
                                            setting_row_disabled(ui, "Right Click Auto Select Word", "Double click/right click to select full alphanumeric words.", self.settings.right_click_select_word);
                                            setting_row_disabled(ui, "Hold Ctrl / Meta to Open Links", "Require modifier key press before launching detected URL hyperlinks.", self.settings.must_hold_ctrl_for_links);
                                            setting_row_disabled(ui, "Command Suggestions", "Display autocompletion hints based on history.", self.settings.show_command_suggestions);
                                            setting_row_disabled(ui, "Auto Reconnect on Disconnect", "Automatically retry remote SSH sessions when connection drops.", self.settings.auto_reconnect_terminal);
                                        }
                                        SettingsCategory::ShellEnv => {
                                            ui.label(egui::RichText::new("Shell & Environment").strong().size(16.0).color(COLOR_ACCENT));
                                            ui.label(egui::RichText::new("Set your default command interpreter, session log directories, and keycodes.").small().color(COLOR_TEXT_MUTED));
                                            ui.add_space(12.0);

                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new("Default Shell Path").strong().color(COLOR_TEXT_PRIMARY));
                                                    ui.label(egui::RichText::new("Executable path spawned when opening new local tabs.").small().color(COLOR_TEXT_MUTED));
                                                });
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if ui.add(egui::TextEdit::singleline(&mut self.settings.default_shell).desired_width(180.0)).changed() {
                                                        changed = true;
                                                    }
                                                });
                                            });
                                            ui.add_space(4.0);
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new("Shell Presets:").small().color(COLOR_TEXT_MUTED));
                                                if ui.small_button("bash").clicked() { self.settings.default_shell = "/bin/bash".to_string(); changed = true; }
                                                if ui.small_button("zsh").clicked() { self.settings.default_shell = "/bin/zsh".to_string(); changed = true; }
                                                if ui.small_button("fish").clicked() { self.settings.default_shell = "/bin/fish".to_string(); changed = true; }
                                            });
                                            ui.add_space(8.0);
                                            ui.separator();
                                            ui.add_space(8.0);

                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new("Backspace Keycode Sequence").strong().color(COLOR_TEXT_PRIMARY));
                                                    ui.label(egui::RichText::new("Control character code sent to PTY upon pressing Backspace.").small().color(COLOR_TEXT_MUTED));
                                                });
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    egui::ComboBox::from_id_source("backspace_seq_select")
                                                        .selected_text(match self.settings.backspace_sequence {
                                                            BackspaceSequence::Delete127 => "^? (Delete 0x7F)",
                                                            BackspaceSequence::Backspace8 => "^H (Backspace 0x08)",
                                                        })
                                                        .show_ui(ui, |ui| {
                                                            if ui.selectable_value(&mut self.settings.backspace_sequence, BackspaceSequence::Delete127, "^? (Delete 0x7F)").clicked() { changed = true; }
                                                            if ui.selectable_value(&mut self.settings.backspace_sequence, BackspaceSequence::Backspace8, "^H (Backspace 0x08)").clicked() { changed = true; }
                                                        });
                                                });
                                            });
                                            ui.add_space(8.0);
                                            ui.separator();
                                            ui.add_space(8.0);

                                            setting_row_disabled(ui, "Terminal Log Directory", "Target filesystem folder for saved session transcripts.", false);
                                            setting_row_disabled(ui, "Save Terminal Session Logs", "Write all output streams into timestamped log files.", self.settings.save_terminal_log);
                                            setting_row_disabled(ui, "Timestamp Log Entries", "Prefix each logged output line with local ISO timestamp.", self.settings.add_timestamp_to_log);
                                        }
                                        SettingsCategory::Sftp => {
                                            ui.label(egui::RichText::new("SFTP & File Transfers").strong().size(16.0).color(COLOR_ACCENT));
                                            ui.label(egui::RichText::new("Manage remote directory traversal, split paneling, and file syncing.").small().color(COLOR_TEXT_MUTED));
                                            ui.add_space(12.0);

                                            changed |= setting_row_toggle(ui, "Split View SFTP Explorer", "Show terminal on the left and directory browser on the right.", &mut self.settings.show_sftp_split_view);
                                            
                                            setting_row_disabled(ui, "Synchronize SFTP with Terminal Path", "Automatically follow the current directory of the active shell.", self.settings.sftp_path_sync);
                                            setting_row_disabled(ui, "Auto Refresh on Tab Switch", "Query remote directory metadata when navigating between sessions.", self.settings.auto_refresh_sftp);
                                            setting_row_disabled(ui, "Show Hidden Dotfiles", "Display files and folders prefixed with a dot by default.", self.settings.show_hidden_sftp);
                                            setting_row_disabled(ui, "Disable SFTP Transfer History", "Do not write upload/download records to disk.", self.settings.disable_sftp_history);
                                        }
                                        SettingsCategory::System => {
                                            ui.label(egui::RichText::new("Application & System").strong().size(16.0).color(COLOR_ACCENT));
                                            ui.label(egui::RichText::new("Window behavior, multi-instance options, and update checks.").small().color(COLOR_TEXT_MUTED));
                                            ui.add_space(12.0);

                                            changed |= setting_row_toggle(ui, "Open Default Tab on Startup", "Spawn a fresh local shell if no previous session was restored.", &mut self.settings.open_default_tab);
                                            
                                            if setting_row_toggle(ui, "Check for Updates on Startup", "Check for newer releases on GitHub once daily.", &mut self.settings.check_updates) {
                                                changed = true;
                                                if !self.settings.check_updates {
                                                    self.available_update = None;
                                                    self.show_update_modal = false;
                                                }
                                            }

                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new("Manual Update Check").strong().color(COLOR_TEXT_PRIMARY));
                                                    let status_text = if self.is_checking_update {
                                                        "Checking GitHub releases...".to_string()
                                                    } else if let Some(ref tag) = self.available_update {
                                                        format!("New version available: {}", tag)
                                                    } else {
                                                        format!("Current version is v{}", env!("CARGO_PKG_VERSION"))
                                                    };
                                                    ui.label(egui::RichText::new(status_text).small().color(COLOR_TEXT_MUTED));
                                                });
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if ui.button("Check for Updates Now").clicked() {
                                                        self.trigger_update_check(true, ctx.clone());
                                                        self.set_toast("Checking for updates...");
                                                    }
                                                });
                                            });
                                            ui.add_space(6.0);
                                            ui.separator();
                                            ui.add_space(6.0);
                                            
                                            setting_row_disabled(ui, "Allow Multi-Instance Execution", "Permit launching multiple independent AZTerm window processes.", self.settings.allow_multi_instance);
                                            setting_row_disabled(ui, "Confirm Before Window Exit", "Ask for confirmation before terminating running session processes.", self.settings.confirm_before_exit);
                                            setting_row_disabled(ui, "Mask Host IP Address", "Hide server IPs from status bars and session titles.", self.settings.hide_ip);
                                            setting_row_disabled(ui, "Use System Title Bar", "Delegate window decorations to your desktop window manager.", self.settings.use_system_titlebar);
                                            setting_row_disabled(ui, "Disable Connection History", "Do not cache recent SSH session targets in SQLite.", self.settings.disable_connection_history);
                                            setting_row_disabled(ui, "Debug Logging Mode", "Emit verbose PTY and layout traces to stderr.", self.settings.debug_mode);
                                        }
                                    }
                                });
                            });

                            if changed {
                                self.settings.save();
                            }
                        });
                    });
                }
            });
    }
}

fn main() -> eframe::Result<()> {
    let cli_opts = parse_cli_arguments();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1160.0, 740.0])
            .with_title("AZTerm")
            .with_app_id("azterm"),
        ..Default::default()
    };
    eframe::run_native(
        "AZTerm",
        options,
        Box::new(|cc| Ok(Box::new(AppState::new(cc, cli_opts)))),
    )
}
