mod db;
mod settings;
mod sftp;
mod ssh;
mod terminal;
mod theme;
mod tiling;

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
use tiling::*;

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
    let border_thickness = 7.0_f32;

    let pointer_pos = match ctx.input(|i| i.pointer.hover_pos()) {
        Some(pos) => pos,
        None => return,
    };

    let on_top = pointer_pos.y <= screen_rect.min.y + border_thickness;
    let on_bottom = pointer_pos.y >= screen_rect.max.y - border_thickness;
    let on_left = pointer_pos.x <= screen_rect.min.x + border_thickness;
    let on_right = pointer_pos.x >= screen_rect.max.x - border_thickness;

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

pub enum PaneAction {
    Split(usize, SplitDirection),
    ToggleMaximize(usize),
    PopToTab(usize),
    Close(usize),
    Focus(usize),
    StartDrag(usize),
}

fn render_tile_tree(
    ui: &mut egui::Ui,
    node: &mut TileNode,
    total_rect: egui::Rect,
    theme: &ThemeConfig,
    settings: &AppSettings,
    sessions: &mut [TerminalSession],
    active_session_id: &mut usize,
    maximized_session: Option<usize>,
    toast: &mut Option<(String, std::time::Instant)>,
    actions: &mut Vec<PaneAction>,
    pane_rects: &mut Vec<(usize, egui::Rect)>,
    is_multi_pane: bool,
) {
    if let Some(max_id) = maximized_session {
        if node.contains(max_id) {
            pane_rects.push((max_id, total_rect));
            if let Some(session) = sessions.iter_mut().find(|s| s.id == max_id) {
                render_single_pane(
                    ui,
                    session,
                    total_rect,
                    theme,
                    settings,
                    active_session_id,
                    true,
                    true,
                    toast,
                    actions,
                );
            }
            return;
        }
    }

    match node {
        TileNode::Leaf(session_id) => {
            pane_rects.push((*session_id, total_rect));
            if let Some(session) = sessions.iter_mut().find(|s| s.id == *session_id) {
                render_single_pane(
                    ui,
                    session,
                    total_rect,
                    theme,
                    settings,
                    active_session_id,
                    is_multi_pane,
                    false,
                    toast,
                    actions,
                );
            }
        }
        TileNode::Split { id, dir, ratio, first, second } => {
            let divider_thick = 5.0_f32;
            match dir {
                SplitDirection::Horizontal => {
                    let avail_w = (total_rect.width() - divider_thick).max(10.0);
                    let w1 = (avail_w * *ratio).clamp(60.0, avail_w - 60.0);
                    let w2 = avail_w - w1;

                    let r1 = egui::Rect::from_min_size(total_rect.min, egui::vec2(w1, total_rect.height()));
                    let div_rect = egui::Rect::from_min_size(egui::pos2(total_rect.min.x + w1, total_rect.min.y), egui::vec2(divider_thick, total_rect.height()));
                    let r2 = egui::Rect::from_min_size(egui::pos2(total_rect.min.x + w1 + divider_thick, total_rect.min.y), egui::vec2(w2, total_rect.height()));

                    let div_id = ui.id().with("split_div_h").with(*id);
                    let div_resp = ui.interact(div_rect, div_id, egui::Sense::click_and_drag());
                    if div_resp.hovered() || div_resp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    if div_resp.dragged() {
                        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                            let new_r = ((pos.x - total_rect.min.x) / avail_w).clamp(0.08, 0.92);
                            *ratio = new_r;
                        }
                    }
                    let div_color = if div_resp.dragged() {
                        theme.accent_color()
                    } else if div_resp.hovered() {
                        theme.accent_hover_color()
                    } else {
                        theme.border_color()
                    };
                    ui.painter().rect_filled(div_rect, 0.0, div_color);

                    render_tile_tree(ui, first, r1, theme, settings, sessions, active_session_id, maximized_session, toast, actions, pane_rects, true);
                    render_tile_tree(ui, second, r2, theme, settings, sessions, active_session_id, maximized_session, toast, actions, pane_rects, true);
                }
                SplitDirection::Vertical => {
                    let avail_h = (total_rect.height() - divider_thick).max(10.0);
                    let h1 = (avail_h * *ratio).clamp(40.0, avail_h - 40.0);
                    let h2 = avail_h - h1;

                    let r1 = egui::Rect::from_min_size(total_rect.min, egui::vec2(total_rect.width(), h1));
                    let div_rect = egui::Rect::from_min_size(egui::pos2(total_rect.min.x, total_rect.min.y + h1), egui::vec2(total_rect.width(), divider_thick));
                    let r2 = egui::Rect::from_min_size(egui::pos2(total_rect.min.x, total_rect.min.y + h1 + divider_thick), egui::vec2(total_rect.width(), h2));

                    let div_id = ui.id().with("split_div_v").with(*id);
                    let div_resp = ui.interact(div_rect, div_id, egui::Sense::click_and_drag());
                    if div_resp.hovered() || div_resp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    if div_resp.dragged() {
                        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                            let new_r = ((pos.y - total_rect.min.y) / avail_h).clamp(0.08, 0.92);
                            *ratio = new_r;
                        }
                    }
                    let div_color = if div_resp.dragged() {
                        theme.accent_color()
                    } else if div_resp.hovered() {
                        theme.accent_hover_color()
                    } else {
                        theme.border_color()
                    };
                    ui.painter().rect_filled(div_rect, 0.0, div_color);

                    render_tile_tree(ui, first, r1, theme, settings, sessions, active_session_id, maximized_session, toast, actions, pane_rects, true);
                    render_tile_tree(ui, second, r2, theme, settings, sessions, active_session_id, maximized_session, toast, actions, pane_rects, true);
                }
            }
        }
    }
}

fn render_single_pane(
    ui: &mut egui::Ui,
    session: &mut TerminalSession,
    rect: egui::Rect,
    theme: &ThemeConfig,
    settings: &AppSettings,
    active_session_id: &mut usize,
    show_header: bool,
    is_maximized: bool,
    toast: &mut Option<(String, std::time::Instant)>,
    actions: &mut Vec<PaneAction>,
) {
    let is_focused = *active_session_id == session.id;
    let border_color = if is_focused { theme.accent_color() } else { theme.border_color() };

    let header_height = if show_header { 24.0_f32 } else { 0.0_f32 };
    let body_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, rect.min.y + header_height),
        rect.max,
    );

    ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(1.0_f32, border_color));

    if show_header {
        let header_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), header_height));
        let header_bg = if is_focused { theme.bg_card_color() } else { theme.bg_panel_color() };
        ui.painter().rect_filled(header_rect, egui::Rounding { nw: 4.0, ne: 4.0, sw: 0.0, se: 0.0 }, header_bg);

        let drag_area_w = (header_rect.width() - 250.0).max(40.0);
        let drag_area_rect = egui::Rect::from_min_size(header_rect.min, egui::vec2(drag_area_w, header_height));
        let drag_resp = ui.interact(drag_area_rect, ui.id().with("pane_hdr_drag").with(session.id), egui::Sense::click_and_drag());

        if drag_resp.clicked() {
            *active_session_id = session.id;
        }
        if drag_resp.drag_started_by(egui::PointerButton::Primary) {
            actions.push(PaneAction::StartDrag(session.id));
        }

        ui.allocate_ui_at_rect(header_rect, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("::").weak().color(theme.text_muted_color()));
                let title_color = if is_focused { theme.accent_color() } else { theme.text_muted_color() };
                ui.label(egui::RichText::new(&session.title).strong().small().color(title_color));

                if is_focused {
                    ui.label(egui::RichText::new("●").small().color(theme.accent_color()));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(egui::RichText::new("X").strong().color(theme.danger_color())).on_hover_text("Close Pane (Ctrl+Shift+W)").clicked() {
                        actions.push(PaneAction::Close(session.id));
                    }
                    if ui.small_button("Pop").on_hover_text("Pop out to a separate tab").clicked() {
                        actions.push(PaneAction::PopToTab(session.id));
                    }
                    let max_text = if is_maximized { "Restore" } else { "Max" };
                    if ui.small_button(max_text).on_hover_text("Maximize / Restore Pane (Ctrl+Shift+M)").clicked() {
                        actions.push(PaneAction::ToggleMaximize(session.id));
                    }
                    if ui.small_button("Split -").on_hover_text("Split Down (Ctrl+Shift+E)").clicked() {
                        actions.push(PaneAction::Split(session.id, SplitDirection::Vertical));
                    }
                    if ui.small_button("Split |").on_hover_text("Split Right (Ctrl+Shift+D)").clicked() {
                        actions.push(PaneAction::Split(session.id, SplitDirection::Horizontal));
                    }
                });
            });
        });
    }

    let mut pane_clicked = false;
    ui.allocate_ui_at_rect(body_rect, |ui| {
        pane_clicked = session.render(ui, settings, theme, is_focused, toast);
    });

    if pane_clicked {
        *active_session_id = session.id;
        ui.ctx().request_repaint();
    }
}

struct AppState {
    settings: AppSettings,
    ssh_store: SshStore,
    sessions: Vec<TerminalSession>,
    workspaces: Vec<WorkspaceTab>,
    active_workspace_idx: usize,
    active_session_id: usize,
    dragging_tab_idx: Option<usize>,
    dragging_pane_id: Option<usize>,
    next_split_id: usize,

    sftp: SftpManager,
    next_tab_id: usize,
    active_view: ActiveView,
    ssh_subview: SshSubView,
    settings_category: SettingsCategory,

    theme: ThemeConfig,
    custom_themes: Vec<ThemeConfig>,
    new_theme_name: String,

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

    fn spawn_local_terminal(&mut self, ctx: egui::Context, custom_dir: Option<String>) {
        let id = self.create_local_session(ctx, custom_dir);
        let ws = WorkspaceTab::new(id, id, format!("Local #{}", id));
        self.workspaces.push(ws);
        self.active_workspace_idx = self.workspaces.len() - 1;
        self.active_session_id = id;
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

    fn split_active_pane(&mut self, dir: SplitDirection, ctx: egui::Context) {
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

    fn close_session(&mut self, session_id: usize, ctx: egui::Context) {
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

    fn tile_quad_grid(&mut self) {
        if self.workspaces.len() < 2 {
            self.set_toast("Need at least 2 open tabs to arrange a grid");
            return;
        }

        let curr_ws = &self.workspaces[self.active_workspace_idx];
        let base_id = curr_ws.root.first_leaf();

        let mut other_leaves = Vec::new();
        let mut consumed_ws_indices = Vec::new();

        for (i, ws) in self.workspaces.iter().enumerate() {
            if i != self.active_workspace_idx {
                for l in ws.leaves() {
                    other_leaves.push(l);
                }
                consumed_ws_indices.push(i);
                if other_leaves.len() >= 3 {
                    break;
                }
            }
        }

        if other_leaves.is_empty() {
            return;
        }

        let s2 = other_leaves[0];
        let s3 = other_leaves.get(1).copied();
        let s4 = other_leaves.get(2).copied();

        let top_split = TileNode::Split {
            id: 101,
            dir: SplitDirection::Horizontal,
            ratio: 0.5,
            first: Box::new(TileNode::Leaf(base_id)),
            second: Box::new(TileNode::Leaf(s2)),
        };

        let bottom_split = match (s3, s4) {
            (Some(id3), Some(id4)) => Some(TileNode::Split {
                id: 102,
                dir: SplitDirection::Horizontal,
                ratio: 0.5,
                first: Box::new(TileNode::Leaf(id3)),
                second: Box::new(TileNode::Leaf(id4)),
            }),
            (Some(id3), None) => Some(TileNode::Leaf(id3)),
            _ => None,
        };

        let quad_root = if let Some(b) = bottom_split {
            TileNode::Split {
                id: 100,
                dir: SplitDirection::Vertical,
                ratio: 0.5,
                first: Box::new(top_split),
                second: Box::new(b),
            }
        } else {
            top_split
        };

        let mut i = 0;
        self.workspaces.retain(|_| {
            let retain = !consumed_ws_indices.contains(&i);
            i += 1;
            retain
        });

        if let Some(ws) = self.workspaces.get_mut(0) {
            ws.root = quad_root;
            ws.maximized_session = None;
            self.active_workspace_idx = 0;
            self.active_session_id = base_id;
        }

        self.set_toast("Arranged into tiled grid");
        self.persist_sessions();
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
        if !self.settings.use_system_titlebar {
            let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            handle_window_resize_borders(ctx, is_max);
        }

        // Global Zoom Shortcuts (Ctrl +, Ctrl -, Ctrl 0, and Ctrl + MouseWheel)
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

        // Modals
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
                                    .color(self.theme.accent_color()),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("Current version: v{}", env!("CARGO_PKG_VERSION")))
                                    .small()
                                    .color(self.theme.text_muted_color()),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("Detected installation: {}", self.install_method.display_name()))
                                    .small()
                                    .color(self.theme.text_primary_color()),
                            );

                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(10.0);

                            match &self.install_method {
                                InstallMethod::ScriptInstalled | InstallMethod::PackageManager(_) => {
                                    ui.label("Would you like to run the official updater script in a new terminal session?");
                                    ui.add_space(6.0);
                                    egui::Frame::none()
                                        .fill(self.theme.bg_panel_color())
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

        let modal_open = self.show_update_modal || self.show_profile_modal || self.show_keygen_modal;
        if self.active_view == ActiveView::Terminal && !modal_open {
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

        // LINE 1: Top Navigation and Window Bar
        egui::TopBottomPanel::top("top_nav")
            .frame(egui::Frame::none().fill(self.theme.bg_panel_color()).inner_margin(egui::Margin::symmetric(14.0, 7.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let brand_resp = ui.add(
                        egui::Label::new(
                            egui::RichText::new("AZTerm")
                                .color(self.theme.accent_color())
                                .strong()
                                .size(16.0),
                        ).sense(egui::Sense::click_and_drag())
                    );
                    if !self.settings.use_system_titlebar {
                        if brand_resp.drag_started_by(egui::PointerButton::Primary)
                            || (brand_resp.hovered() && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)))
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }
                        if brand_resp.double_clicked() {
                            let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
                        }
                    }

                    ui.add_space(8.0);

                    if nav_tab_button(ui, "Terminal", self.active_view == ActiveView::Terminal, &self.theme) {
                        self.active_view = ActiveView::Terminal;
                    }
                    if nav_tab_button(ui, "SSH Profiles", self.active_view == ActiveView::SshBookmarks, &self.theme) {
                        self.active_view = ActiveView::SshBookmarks;
                    }
                    if nav_tab_button(ui, "SFTP Explorer", self.active_view == ActiveView::SftpBrowser, &self.theme) {
                        self.active_view = ActiveView::SftpBrowser;
                    }
                    if nav_tab_button(ui, "Settings", self.active_view == ActiveView::Settings, &self.theme) {
                        self.active_view = ActiveView::Settings;
                    }

                    ui.separator();

                    if nav_action_button(ui, "+ New Shell", &self.theme) {
                        self.spawn_local_terminal(ctx.clone(), None);
                    }

                    if self.active_view == ActiveView::Terminal {
                        if ui.button("Split Right").on_hover_text("Split active pane side-by-side (Ctrl+Shift+D)").clicked() {
                            self.split_active_pane(SplitDirection::Horizontal, ctx.clone());
                        }
                        if ui.button("Split Down").on_hover_text("Split active pane top-and-bottom (Ctrl+Shift+E)").clicked() {
                            self.split_active_pane(SplitDirection::Vertical, ctx.clone());
                        }
                        if let Some(ws) = self.workspaces.get(self.active_workspace_idx) {
                            if !ws.is_single_pane() {
                                let max_label = if ws.maximized_session.is_some() { "Restore Splits" } else { "Maximize Pane" };
                                if ui.button(max_label).on_hover_text("Toggle maximize active pane (Ctrl+Shift+M)").clicked() {
                                    if let Some(ws_mut) = self.workspaces.get_mut(self.active_workspace_idx) {
                                        ws_mut.maximized_session = if ws_mut.maximized_session.is_some() { None } else { Some(self.active_session_id) };
                                    }
                                }
                            }
                        }
                        if self.workspaces.len() > 1 {
                            if ui.button("Tile All Tabs (Grid)").on_hover_text("Tile all open tabs into an even 2x2 grid").clicked() {
                                self.tile_quad_grid();
                            }
                        }
                    }

                    if !self.settings.use_system_titlebar {
                        let controls_w = 96.0_f32;
                        let drag_width = (ui.available_width() - controls_w).max(10.0);
                        let (_drag_rect, drag_resp) = ui.allocate_exact_size(
                            egui::vec2(drag_width, 26.0),
                            egui::Sense::click_and_drag(),
                        );

                        if drag_resp.drag_started_by(egui::PointerButton::Primary)
                            || (drag_resp.hovered() && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)))
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }
                        if drag_resp.double_clicked() {
                            let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let close_resp = ui.add(
                                egui::Button::new(egui::RichText::new("X").size(12.0).color(self.theme.text_primary_color()))
                                    .min_size(egui::vec2(28.0, 22.0))
                                    .fill(egui::Color32::TRANSPARENT)
                            );
                            if close_resp.hovered() {
                                ui.painter().rect_filled(close_resp.rect, 3.0, self.theme.danger_color());
                            }
                            if close_resp.clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }

                            let is_max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                            let max_icon = if is_max { "Restore" } else { "Max" };
                            if ui.add(
                                egui::Button::new(egui::RichText::new(max_icon).size(11.0).color(self.theme.text_primary_color()))
                                    .min_size(egui::vec2(44.0, 22.0))
                                    .fill(egui::Color32::TRANSPARENT)
                            ).clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_max));
                            }

                            if ui.add(
                                egui::Button::new(egui::RichText::new("-").size(14.0).color(self.theme.text_primary_color()))
                                    .min_size(egui::vec2(28.0, 22.0))
                                    .fill(egui::Color32::TRANSPARENT)
                            ).clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                        });
                    }
                });
            });

        // LINE 2: Dedicated Tab Bar (Hidden if only 1 single-pane tab is open)
        if self.workspaces.len() > 1 {
            egui::TopBottomPanel::top("session_tabs_bar")
                .frame(
                    egui::Frame::none()
                        .fill(self.theme.bg_panel_color().linear_multiply(0.85))
                        .stroke(egui::Stroke::new(1.0_f32, self.theme.border_color()))
                        .inner_margin(egui::Margin::symmetric(14.0, 4.0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let mut tab_to_close: Option<usize> = None;
                        let avail_w = (ui.available_width() - 20.0).max(100.0);
                        let num_tabs = self.workspaces.len() as f32;
                        let computed_tab_width = ((avail_w / num_tabs) - 6.0).clamp(90.0, 200.0);
                        let max_chars = ((computed_tab_width - 32.0) / 7.2).max(4.0) as usize;

                        egui::ScrollArea::horizontal()
                            .auto_shrink([false, false])
                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for (i, ws) in self.workspaces.iter().enumerate() {
                                        let is_active = self.active_view == ActiveView::Terminal && self.active_workspace_idx == i;
                                        let tab_title = if ws.is_single_pane() {
                                            ws.title.clone()
                                        } else {
                                            format!("{} ({} Panes)", ws.title, ws.leaves().len())
                                        };

                                        let label_text = if tab_title.len() > max_chars {
                                            format!("{}...", &tab_title[..max_chars.saturating_sub(3)])
                                        } else {
                                            tab_title
                                        };

                                        let (tab_clicked, close_clicked) = session_tab_chip(
                                            ui,
                                            ws.id,
                                            &label_text,
                                            is_active,
                                            computed_tab_width,
                                            true,
                                            &self.theme,
                                        );

                                        let chip_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(computed_tab_width, 26.0));
                                        let chip_resp = ui.interact(chip_rect, ui.id().with("chip_drag").with(ws.id), egui::Sense::drag());
                                        if chip_resp.drag_started_by(egui::PointerButton::Primary) {
                                            self.dragging_tab_idx = Some(i);
                                        }

                                        if tab_clicked {
                                            self.active_workspace_idx = i;
                                            self.active_session_id = ws.root.first_leaf();
                                            self.active_view = ActiveView::Terminal;
                                        }

                                        if close_clicked {
                                            tab_to_close = Some(i);
                                        }

                                        ui.add_space(4.0);
                                    }
                                });
                            });

                        if let Some(i) = tab_to_close {
                            let leaves = self.workspaces[i].leaves();
                            for leaf_id in leaves {
                                self.sessions.retain(|s| s.id != leaf_id);
                            }
                            self.workspaces.remove(i);
                            if self.workspaces.is_empty() {
                                self.spawn_local_terminal(ctx.clone(), None);
                            } else {
                                if self.active_workspace_idx >= self.workspaces.len() {
                                    self.active_workspace_idx = self.workspaces.len() - 1;
                                }
                                if let Some(ws) = self.workspaces.get(self.active_workspace_idx) {
                                    self.active_session_id = ws.root.first_leaf();
                                }
                            }
                            self.persist_sessions();
                        }
                    });
                });
        }

        // Bottom Status Bar
        egui::TopBottomPanel::bottom("bottom_status_bar")
            .frame(egui::Frame::none().fill(self.theme.bg_panel_color()).inner_margin(egui::Margin::symmetric(14.0, 4.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(session) = self.sessions.iter().find(|s| s.id == self.active_session_id) {
                        let info = match &session.session_type {
                            SessionType::Local { working_dir } => format!("Active Shell [{}]: {}", session.id, working_dir),
                            SessionType::Ssh { profile_id } => format!("Active Target [{}]: {}", session.id, profile_id),
                        };
                        ui.label(egui::RichText::new(info).small().color(self.theme.text_muted_color()));
                    }

                    ui.separator();

                    let sftp_btn_text = if self.settings.show_sftp_split_view {
                        "SFTP Drawer: OPEN"
                    } else {
                        "SFTP Drawer: CLOSED"
                    };
                    if nav_tab_button(ui, sftp_btn_text, self.settings.show_sftp_split_view, &self.theme) {
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
                            if nav_action_button(ui, &format!("Update: {}", update_tag), &self.theme) {
                                self.show_update_modal = true;
                            }
                        }
                    }

                    if let Some(ref status) = self.sftp.transfer_status {
                        ui.separator();
                        ui.label(egui::RichText::new(status).small().color(self.theme.accent_color()));
                    }

                    if let Some((msg, time)) = &self.toast_message {
                        if time.elapsed().as_secs_f32() < 3.0 {
                            ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                                egui::Frame::none()
                                    .fill(self.theme.bg_card_color())
                                    .stroke(egui::Stroke::new(1.0_f32, self.theme.accent_color()))
                                    .rounding(4.0)
                                    .inner_margin(egui::Margin::symmetric(12.0, 2.0))
                                    .show(ui, |ui| {
                                        ui.label(egui::RichText::new(msg).color(self.theme.accent_color()).strong().small());
                                    });
                            });
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(session) = self.sessions.iter().find(|s| s.id == self.active_session_id) {
                            let scroll_info = if session.scroll_offset > 0 {
                                format!("-{} lines | {}x{}", session.scroll_offset, session.cols, session.rows)
                            } else {
                                format!("{}x{}", session.cols, session.rows)
                            };
                            let col = if session.scroll_offset > 0 { self.theme.accent_color() } else { self.theme.text_muted_color() };
                            ui.label(egui::RichText::new(scroll_info).small().color(col));
                        }
                    });
                });
            });

        // Central Workspace Area
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.theme.bg_main_color()))
            .show(ctx, |ui| match self.active_view {
                ActiveView::Terminal => {
                    let mut toast = self.toast_message.clone();

                    if self.workspaces.is_empty() {
                        ui.centered_and_justified(|ui| {
                            if ui.button("Open Shell Session").clicked() {
                                self.spawn_local_terminal(ctx.clone(), None);
                            }
                        });
                        return;
                    }

                    let available_rect = ui.available_rect_before_wrap();
                    let (term_area_rect, sftp_pane_rect) = if self.settings.show_sftp_split_view {
                        let w = available_rect.width() * 0.65;
                        (
                            egui::Rect::from_min_size(available_rect.min, egui::vec2(w, available_rect.height())),
                            Some(egui::Rect::from_min_size(egui::pos2(available_rect.min.x + w, available_rect.min.y), egui::vec2(available_rect.width() - w, available_rect.height()))),
                        )
                    } else {
                        (available_rect, None)
                    };

                    let mut actions = Vec::new();
                    let mut pane_rects = Vec::new();

                    if let Some(ws) = self.workspaces.get_mut(self.active_workspace_idx) {
                        let is_multi_pane = !ws.is_single_pane();
                        let max_session = ws.maximized_session;
                        render_tile_tree(
                            ui,
                            &mut ws.root,
                            term_area_rect,
                            &self.theme,
                            &self.settings,
                            &mut self.sessions,
                            &mut self.active_session_id,
                            max_session,
                            &mut toast,
                            &mut actions,
                            &mut pane_rects,
                            is_multi_pane,
                        );
                    }

                    // Drag-and-Drop Docking & Moving
                    let is_primary_down = ui.input(|i| i.pointer.primary_down());
                    let pointer_pos = ui.input(|i| i.pointer.hover_pos());

                    let is_dragging_own_tab = self.dragging_tab_idx == Some(self.active_workspace_idx);

                    let active_drag_session: Option<usize> = if self.dragging_pane_id.is_some() {
                        self.dragging_pane_id
                    } else if let Some(idx) = self.dragging_tab_idx {
                        if !is_dragging_own_tab {
                            self.workspaces.get(idx).map(|w| w.root.first_leaf())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let (Some(dragged_sess_id), Some(ptr)) = (active_drag_session, pointer_pos) {
                        if is_primary_down {
                            let mut hovered_zone: Option<(usize, DockZone, egui::Rect)> = None;
                            for &(pane_id, p_rect) in &pane_rects {
                                if pane_id != dragged_sess_id {
                                    if let Some(zone) = detect_dock_zone(p_rect, ptr) {
                                        let snap_rect = dock_zone_preview_rect(p_rect, zone);
                                        hovered_zone = Some((pane_id, zone, snap_rect));
                                        break;
                                    }
                                }
                            }

                            if let Some((_, _, snap_rect)) = hovered_zone {
                                ui.painter().rect_filled(
                                    snap_rect,
                                    6.0,
                                    egui::Color32::from_rgba_unmultiplied(self.theme.accent[0], self.theme.accent[1], self.theme.accent[2], 75),
                                );
                                ui.painter().rect_stroke(
                                    snap_rect,
                                    6.0,
                                    egui::Stroke::new(2.0_f32, self.theme.accent_color()),
                                );
                                ui.painter().text(
                                    snap_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "Drop to Tile Here",
                                    egui::FontId::proportional(14.0),
                                    egui::Color32::WHITE,
                                );
                            } else if ptr.y < term_area_rect.min.y && self.dragging_pane_id.is_some() {
                                let badge_rect = egui::Rect::from_center_size(ptr, egui::vec2(160.0, 26.0));
                                ui.painter().rect_filled(badge_rect, 4.0, self.theme.bg_card_color());
                                ui.painter().rect_stroke(badge_rect, 4.0, egui::Stroke::new(1.0_f32, self.theme.accent_color()));
                                ui.painter().text(badge_rect.center(), egui::Align2::CENTER_CENTER, "Drop to Pop Out as Tab", egui::FontId::proportional(12.0), self.theme.accent_color());
                            }
                        } else {
                            if let Some(ptr) = pointer_pos {
                                let mut docked = false;
                                for &(pane_id, p_rect) in &pane_rects {
                                    if pane_id != dragged_sess_id {
                                        if let Some(zone) = detect_dock_zone(p_rect, ptr) {
                                            let dir = match zone {
                                                DockZone::Left | DockZone::Right => SplitDirection::Horizontal,
                                                DockZone::Top | DockZone::Bottom => SplitDirection::Vertical,
                                            };
                                            let insert_after = matches!(zone, DockZone::Right | DockZone::Bottom);

                                            if let Some(drag_ws_idx) = self.dragging_tab_idx {
                                                if drag_ws_idx < self.workspaces.len() && drag_ws_idx != self.active_workspace_idx {
                                                    let incoming_tree = self.workspaces[drag_ws_idx].root.clone();
                                                    self.workspaces.remove(drag_ws_idx);
                                                    if drag_ws_idx < self.active_workspace_idx {
                                                        self.active_workspace_idx -= 1;
                                                    }
                                                    let split_id = self.next_split_id;
                                                    self.next_split_id += 1;

                                                    if let Some(ws) = self.workspaces.get_mut(self.active_workspace_idx) {
                                                        ws.root.split_leaf_with_node(pane_id, incoming_tree, dir, insert_after, split_id);
                                                        self.active_session_id = dragged_sess_id;
                                                        self.set_toast("Tiled tab into workspace");
                                                        self.persist_sessions();
                                                    }
                                                }
                                            } else if self.dragging_pane_id.is_some() {
                                                if let Some(ws) = self.workspaces.get_mut(self.active_workspace_idx) {
                                                    if !ws.is_single_pane() {
                                                        ws.root.remove_leaf(dragged_sess_id);
                                                        let split_id = self.next_split_id;
                                                        self.next_split_id += 1;
                                                        ws.root.split_leaf(pane_id, dragged_sess_id, dir, insert_after, split_id);
                                                        self.active_session_id = dragged_sess_id;
                                                        self.set_toast("Moved tile");
                                                        self.persist_sessions();
                                                    }
                                                }
                                            }
                                            docked = true;
                                            break;
                                        }
                                    }
                                }

                                if !docked && ptr.y < term_area_rect.min.y && self.dragging_pane_id.is_some() {
                                    if let Some(ws) = self.workspaces.get_mut(self.active_workspace_idx) {
                                        if !ws.is_single_pane() {
                                            ws.root.remove_leaf(dragged_sess_id);
                                            let session_title = self.sessions.iter().find(|s| s.id == dragged_sess_id).map(|s| s.title.clone()).unwrap_or_else(|| format!("Local #{}", dragged_sess_id));
                                            let new_ws = WorkspaceTab::new(dragged_sess_id, dragged_sess_id, session_title);
                                            self.workspaces.push(new_ws);
                                            self.active_workspace_idx = self.workspaces.len() - 1;
                                            self.active_session_id = dragged_sess_id;
                                            self.set_toast("Popped out to its own tab");
                                            self.persist_sessions();
                                        }
                                    }
                                }
                            }
                            self.dragging_tab_idx = None;
                            self.dragging_pane_id = None;
                        }
                    }

                    for action in actions {
                        match action {
                            PaneAction::Split(id, dir) => {
                                self.active_session_id = id;
                                self.split_active_pane(dir, ctx.clone());
                            }
                            PaneAction::ToggleMaximize(id) => {
                                if let Some(ws) = self.workspaces.get_mut(self.active_workspace_idx) {
                                    if !ws.is_single_pane() {
                                        ws.maximized_session = if ws.maximized_session == Some(id) { None } else { Some(id) };
                                    }
                                }
                            }
                            PaneAction::PopToTab(id) => {
                                if let Some(ws) = self.workspaces.get_mut(self.active_workspace_idx) {
                                    if ws.is_single_pane() {
                                        self.set_toast("Pane is already in its own tab");
                                    } else {
                                        ws.root.remove_leaf(id);
                                        if ws.maximized_session == Some(id) {
                                            ws.maximized_session = None;
                                        }
                                        let session_title = self.sessions.iter().find(|s| s.id == id).map(|s| s.title.clone()).unwrap_or_else(|| format!("Local #{}", id));
                                        let new_ws = WorkspaceTab::new(id, id, session_title);
                                        self.workspaces.push(new_ws);
                                        self.active_workspace_idx = self.workspaces.len() - 1;
                                        self.active_session_id = id;
                                        self.set_toast("Popped out to its own tab");
                                        self.persist_sessions();
                                    }
                                }
                            }
                            PaneAction::Close(id) => {
                                self.close_session(id, ctx.clone());
                            }
                            PaneAction::Focus(id) => {
                                self.active_session_id = id;
                            }
                            PaneAction::StartDrag(id) => {
                                self.dragging_pane_id = Some(id);
                                self.active_session_id = id;
                            }
                        }
                    }

                    if let Some(sftp_rect) = sftp_pane_rect {
                        self.theme.card_frame().show(ui, |ui| {
                            ui.allocate_ui_at_rect(sftp_rect, |ui| {
                                ui.horizontal(|ui| {
                                    ui.heading("SFTP Sync Pane");
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.button("Upload Selected").clicked() {
                                            self.sftp.upload_selected();
                                        }
                                    });
                                });
                                ui.separator();
                                self.sftp.right_pane.render(ui, &self.theme);
                            });
                        });
                    }

                    self.toast_message = toast;
                }
                ActiveView::SshBookmarks => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            ui.heading(egui::RichText::new("SSH Manager").color(self.theme.text_primary_color()));
                            ui.add_space(16.0);
                            if ui.selectable_label(self.ssh_subview == SshSubView::Profiles, "Connections").clicked() {
                                self.ssh_subview = SshSubView::Profiles;
                            }
                            if ui.selectable_label(self.ssh_subview == SshSubView::KeysManager, "Saved Keypairs").clicked() {
                                self.ssh_subview = SshSubView::KeysManager;
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("+ New SSH Profile").clicked() {
                                    self.open_create_profile_modal();
                                }
                                if ui.button("Key Generator").clicked() {
                                    self.show_keygen_modal = true;
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
                                    self.theme.card_frame().show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.vertical(|ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(egui::RichText::new(&profile.name).strong().size(15.0).color(self.theme.text_primary_color()));
                                                    let auth_badge = match &profile.auth_type {
                                                        SshAuthType::PasswordOrAgent => "[Password/Agent]",
                                                        SshAuthType::KeyFile(_) => "[Key File]",
                                                        SshAuthType::PastedKey { .. } => "[Inline Key]",
                                                    };
                                                    ui.label(egui::RichText::new(auth_badge).color(self.theme.accent_color()).small());
                                                });
                                                ui.label(
                                                    egui::RichText::new(format!("{}@{}:{}", profile.username, profile.host, profile.port))
                                                        .color(self.theme.text_muted_color()),
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
                                    self.theme.card_frame().show(ui, |ui| {
                                        ui.label(egui::RichText::new("No SSH keys stored in ~/.config/azterm/keys yet.").color(self.theme.text_muted_color()));
                                    });
                                } else {
                                    let mut key_to_delete: Option<String> = None;

                                    for key in saved_keys {
                                        self.theme.card_frame().show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new(&key.file_name).strong().color(self.theme.accent_color()));
                                                    ui.label(egui::RichText::new(format!("Path: {}", key.priv_path.display())).small().color(self.theme.text_muted_color()));
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

                        self.theme.card_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Dual-Session SFTP File Transfer").strong().color(self.theme.accent_color()));

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
                            self.theme.card_frame().show(&mut cols[0], |ui| {
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
                                self.sftp.left_pane.render(ui, &self.theme);
                            });

                            self.theme.card_frame().show(&mut cols[1], |ui| {
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
                                self.sftp.right_pane.render(ui, &self.theme);
                            });
                        });
                    });
                }
                ActiveView::Settings => {
                    ui.columns(2, |columns| {
                        columns[0].set_max_width(210.0);
                        columns[0].vertical(|ui| {
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new("Preferences").strong().size(16.0).color(self.theme.text_primary_color()));
                            ui.add_space(12.0);

                            let nav_item = |ui: &mut egui::Ui, cat: SettingsCategory, label: &str, current: SettingsCategory, theme: &ThemeConfig| -> bool {
                                let is_active = current == cat;
                                let bg = if is_active { theme.bg_card_color() } else { egui::Color32::TRANSPARENT };
                                let stroke = if is_active { egui::Stroke::new(1.0_f32, theme.accent_color()) } else { egui::Stroke::NONE };

                                egui::Frame::none()
                                    .fill(bg)
                                    .stroke(stroke)
                                    .rounding(4.0)
                                    .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                                    .show(ui, |ui| {
                                        ui.set_width(180.0);
                                        let text_color = if is_active { theme.accent_color() } else { theme.text_primary_color() };
                                        ui.selectable_label(is_active, egui::RichText::new(label).color(text_color)).clicked()
                                    }).inner
                            };

                            if nav_item(ui, SettingsCategory::Appearance, "Themes & Window", self.settings_category, &self.theme) {
                                self.settings_category = SettingsCategory::Appearance;
                            }
                            ui.add_space(4.0);
                            if nav_item(ui, SettingsCategory::Terminal, "Terminal Interaction", self.settings_category, &self.theme) {
                                self.settings_category = SettingsCategory::Terminal;
                            }
                            ui.add_space(4.0);
                            if nav_item(ui, SettingsCategory::ShellEnv, "Shell & Environment", self.settings_category, &self.theme) {
                                self.settings_category = SettingsCategory::ShellEnv;
                            }
                            ui.add_space(4.0);
                            if nav_item(ui, SettingsCategory::Sftp, "SFTP & Transfers", self.settings_category, &self.theme) {
                                self.settings_category = SettingsCategory::Sftp;
                            }
                            ui.add_space(4.0);
                            if nav_item(ui, SettingsCategory::System, "Application & System", self.settings_category, &self.theme) {
                                self.settings_category = SettingsCategory::System;
                            }

                            ui.add_space(20.0);
                            ui.separator();
                            ui.add_space(8.0);
                            if ui.button("Restore Defaults").clicked() {
                                self.settings = AppSettings::default();
                                self.settings.save();
                                self.theme = ThemeConfig::default();
                                Database::save_active_theme(&self.theme);
                                ctx.set_zoom_factor(1.0);
                                self.set_toast("Defaults Restored");
                            }
                        });

                        columns[1].vertical(|ui| {
                            ui.add_space(10.0);
                            let mut changed = false;

                            self.theme.card_frame().show(ui, |ui| {
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    match self.settings_category {
                                        SettingsCategory::Appearance => {
                                            ui.label(egui::RichText::new("Themes & Window Appearance").strong().size(16.0).color(self.theme.accent_color()));
                                            ui.label(egui::RichText::new("Choose from built-in themes, customize colors, transparency, and window decorations.").small().color(self.theme.text_muted_color()));
                                            ui.add_space(12.0);

                                            if setting_row_toggle(
                                                ui,
                                                "Use OS Native Title Bar",
                                                "Use system window decorations. Toggle off to use the sleek custom integrated AZTerm titlebar.",
                                                &mut self.settings.use_system_titlebar,
                                                &self.theme,
                                            ) {
                                                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(self.settings.use_system_titlebar));
                                                changed = true;
                                            }

                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new("Window Zoom Level").strong().color(self.theme.text_primary_color()));
                                                    ui.label(egui::RichText::new("Current scale (Ctrl +, Ctrl -, Ctrl 0 to reset).").small().color(self.theme.text_muted_color()));
                                                });
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if ui.button("Reset (100%)").clicked() {
                                                        self.settings.zoom_factor = 1.0;
                                                        ctx.set_zoom_factor(1.0);
                                                        changed = true;
                                                    }
                                                    ui.label(format!("{}%", (self.settings.zoom_factor * 100.0).round() as u32));
                                                });
                                            });
                                            ui.add_space(8.0);
                                            ui.separator();
                                            ui.add_space(8.0);

                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new("Window Background Opacity").strong().color(self.theme.text_primary_color()));
                                                    ui.label(egui::RichText::new("Set terminal transparency (20% to 100%). Live preview as you drag.").small().color(self.theme.text_muted_color()));
                                                });
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    let pct = (self.theme.opacity * 100.0).round() as u32;
                                                    ui.label(format!("{}%", pct));
                                                    if ui.add(egui::Slider::new(&mut self.theme.opacity, 0.20..=1.0).show_value(false)).changed() {
                                                        Database::save_active_theme(&self.theme);
                                                    }
                                                });
                                            });
                                            ui.add_space(8.0);
                                            ui.separator();
                                            ui.add_space(8.0);

                                            ui.label(egui::RichText::new("Select Theme Preset").strong().color(self.theme.text_primary_color()));
                                            ui.add_space(4.0);

                                            let builtins = ThemeConfig::builtins();
                                            let mut selected_theme_to_apply: Option<ThemeConfig> = None;

                                            ui.horizontal_wrapped(|ui| {
                                                for preset in &builtins {
                                                    let is_active = self.theme.id == preset.id;
                                                    let btn = ui.selectable_label(is_active, &preset.name);
                                                    if btn.clicked() {
                                                        let mut new_th = preset.clone();
                                                        new_th.opacity = self.theme.opacity;
                                                        selected_theme_to_apply = Some(new_th);
                                                    }
                                                }
                                                for custom in &self.custom_themes {
                                                    let is_active = self.theme.id == custom.id;
                                                    let label = format!("* {}", custom.name);
                                                    let btn = ui.selectable_label(is_active, label);
                                                    if btn.clicked() {
                                                        let mut new_th = custom.clone();
                                                        new_th.opacity = self.theme.opacity;
                                                        selected_theme_to_apply = Some(new_th);
                                                    }
                                                }
                                            });

                                            if let Some(new_th) = selected_theme_to_apply {
                                                self.theme = new_th;
                                                Database::save_active_theme(&self.theme);
                                                self.set_toast(format!("Theme applied: {}", self.theme.name));
                                            }

                                            ui.add_space(8.0);
                                            ui.horizontal(|ui| {
                                                ui.text_edit_singleline(&mut self.new_theme_name);
                                                if ui.button("+ Duplicate Current as Custom").clicked() {
                                                    let mut custom = self.theme.clone();
                                                    custom.id = format!("custom_{}", chrono::Utc::now().timestamp_millis());
                                                    custom.name = self.new_theme_name.clone();
                                                    custom.is_builtin = false;
                                                    self.custom_themes.push(custom.clone());
                                                    self.theme = custom;
                                                    Database::save_custom_themes(&self.custom_themes);
                                                    Database::save_active_theme(&self.theme);
                                                    self.set_toast("Custom Theme Created");
                                                }

                                                if !self.theme.is_builtin {
                                                    if ui.button("Delete This Custom Theme").clicked() {
                                                        let delete_id = self.theme.id.clone();
                                                        self.custom_themes.retain(|t| t.id != delete_id);
                                                        Database::save_custom_themes(&self.custom_themes);
                                                        self.theme = ThemeConfig::cyber_cyan();
                                                        Database::save_active_theme(&self.theme);
                                                        self.set_toast("Custom Theme Deleted");
                                                    }
                                                }
                                            });

                                            ui.add_space(10.0);
                                            ui.separator();
                                            ui.add_space(10.0);

                                            ui.label(egui::RichText::new("Theme Colors (Real-Time Customization)").strong().color(self.theme.accent_color()));
                                            ui.label(egui::RichText::new("Click any color swatch below to open the color picker. Changes take effect instantly.").small().color(self.theme.text_muted_color()));
                                            ui.add_space(8.0);

                                            let mut color_modified = false;

                                            egui::Grid::new("colors_grid").num_columns(4).spacing([18.0, 8.0]).show(ui, |ui| {
                                                ui.label("Terminal Background:");
                                                color_modified |= ui.color_edit_button_srgb(&mut self.theme.bg_main).changed();

                                                ui.label("Panel & Nav Background:");
                                                color_modified |= ui.color_edit_button_srgb(&mut self.theme.bg_panel).changed();
                                                ui.end_row();

                                                ui.label("Card Background:");
                                                color_modified |= ui.color_edit_button_srgb(&mut self.theme.bg_card).changed();

                                                ui.label("Borders & Dividers:");
                                                color_modified |= ui.color_edit_button_srgb(&mut self.theme.border).changed();
                                                ui.end_row();

                                                ui.label("Primary Accent:");
                                                color_modified |= ui.color_edit_button_srgb(&mut self.theme.accent).changed();

                                                ui.label("Accent Hover:");
                                                color_modified |= ui.color_edit_button_srgb(&mut self.theme.accent_hover).changed();
                                                ui.end_row();

                                                ui.label("Primary Text:");
                                                color_modified |= ui.color_edit_button_srgb(&mut self.theme.text_primary).changed();

                                                ui.label("Muted Text:");
                                                color_modified |= ui.color_edit_button_srgb(&mut self.theme.text_muted).changed();
                                                ui.end_row();

                                                ui.label("Success Badge:");
                                                color_modified |= ui.color_edit_button_srgb(&mut self.theme.success).changed();

                                                ui.label("Danger / Close:");
                                                color_modified |= ui.color_edit_button_srgb(&mut self.theme.danger).changed();
                                                ui.end_row();
                                            });

                                            ui.add_space(10.0);
                                            ui.label(egui::RichText::new("Terminal 16 ANSI Palette").strong().color(self.theme.text_primary_color()));
                                            ui.add_space(6.0);

                                            egui::Grid::new("ansi_grid").num_columns(8).spacing([10.0, 6.0]).show(ui, |ui| {
                                                let labels = ["Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White"];
                                                for (i, name) in labels.iter().enumerate() {
                                                    ui.vertical(|ui| {
                                                        ui.label(egui::RichText::new(*name).small().color(self.theme.text_muted_color()));
                                                        color_modified |= ui.color_edit_button_srgb(&mut self.theme.ansi_colors[i]).changed();
                                                    });
                                                }
                                                ui.end_row();

                                                let bright_labels = ["Br-Black", "Br-Red", "Br-Green", "Br-Yellow", "Br-Blue", "Br-Magenta", "Br-Cyan", "Br-White"];
                                                for (i, name) in bright_labels.iter().enumerate() {
                                                    ui.vertical(|ui| {
                                                        ui.label(egui::RichText::new(*name).small().color(self.theme.text_muted_color()));
                                                        color_modified |= ui.color_edit_button_srgb(&mut self.theme.ansi_colors[i + 8]).changed();
                                                    });
                                                }
                                                ui.end_row();
                                            });

                                            if color_modified {
                                                if !self.theme.is_builtin {
                                                    if let Some(existing) = self.custom_themes.iter_mut().find(|t| t.id == self.theme.id) {
                                                        *existing = self.theme.clone();
                                                        Database::save_custom_themes(&self.custom_themes);
                                                    }
                                                }
                                                Database::save_active_theme(&self.theme);
                                            }
                                        }
                                        SettingsCategory::Terminal => {
                                            ui.label(egui::RichText::new("Terminal Interaction").strong().size(16.0).color(self.theme.accent_color()));
                                            ui.label(egui::RichText::new("Configure mouse behavior, clipboard actions, and scrollback depth.").small().color(self.theme.text_muted_color()));
                                            ui.add_space(12.0);

                                            changed |= setting_row_toggle(ui, "Cursor Blink", "Animate cursor blinking in the active terminal buffer.", &mut self.settings.cursor_blink, &self.theme);
                                            changed |= setting_row_toggle(ui, "Copy Selected Text on Select", "Automatically copy highlighted text to OS clipboard on drag release.", &mut self.settings.copy_on_select, &self.theme);
                                            changed |= setting_row_toggle(ui, "Paste on Right Click", "Immediately write clipboard text into the terminal on right click.", &mut self.settings.paste_on_right_click, &self.theme);

                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new("Scrollback Buffer Depth").strong().color(self.theme.text_primary_color()));
                                                    ui.label(egui::RichText::new("Total lines of output history retained per tab (scroll with mouse wheel).").small().color(self.theme.text_muted_color()));
                                                });
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    egui::ComboBox::from_id_source("scrollback_depth_combo")
                                                        .selected_text(format!("{} lines", self.settings.scrollback_lines))
                                                        .show_ui(ui, |ui| {
                                                            if ui.selectable_value(&mut self.settings.scrollback_lines, 2000, "2,000 lines").clicked() { changed = true; }
                                                            if ui.selectable_value(&mut self.settings.scrollback_lines, 5000, "5,000 lines").clicked() { changed = true; }
                                                            if ui.selectable_value(&mut self.settings.scrollback_lines, 10000, "10,000 lines").clicked() { changed = true; }
                                                            if ui.selectable_value(&mut self.settings.scrollback_lines, 25000, "25,000 lines").clicked() { changed = true; }
                                                            if ui.selectable_value(&mut self.settings.scrollback_lines, 50000, "50,000 lines").clicked() { changed = true; }
                                                        });
                                                });
                                            });
                                            ui.add_space(6.0);
                                            ui.separator();
                                            ui.add_space(6.0);

                                            setting_row_disabled(ui, "Right Click Auto Select Word", "Double click/right click to select full alphanumeric words.", self.settings.right_click_select_word);
                                            setting_row_disabled(ui, "Hold Ctrl / Meta to Open Links", "Require modifier key press before launching detected URL hyperlinks.", self.settings.must_hold_ctrl_for_links);
                                            setting_row_disabled(ui, "Command Suggestions", "Display autocompletion hints based on history.", self.settings.show_command_suggestions);
                                            setting_row_disabled(ui, "Auto Reconnect on Disconnect", "Automatically retry remote SSH sessions when connection drops.", self.settings.auto_reconnect_terminal);
                                        }
                                        SettingsCategory::ShellEnv => {
                                            ui.label(egui::RichText::new("Shell & Environment").strong().size(16.0).color(self.theme.accent_color()));
                                            ui.label(egui::RichText::new("Set your default command interpreter, session log directories, and keycodes.").small().color(self.theme.text_muted_color()));
                                            ui.add_space(12.0);

                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new("Default Shell Path").strong().color(self.theme.text_primary_color()));
                                                    ui.label(egui::RichText::new("Executable path spawned when opening new local tabs.").small().color(self.theme.text_muted_color()));
                                                });
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if ui.add(egui::TextEdit::singleline(&mut self.settings.default_shell).desired_width(180.0)).changed() {
                                                        changed = true;
                                                    }
                                                });
                                            });
                                            ui.add_space(4.0);
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new("Shell Presets:").small().color(self.theme.text_muted_color()));
                                                if ui.small_button("bash").clicked() { self.settings.default_shell = "/bin/bash".to_string(); changed = true; }
                                                if ui.small_button("zsh").clicked() { self.settings.default_shell = "/bin/zsh".to_string(); changed = true; }
                                                if ui.small_button("fish").clicked() { self.settings.default_shell = "/bin/fish".to_string(); changed = true; }
                                            });
                                            ui.add_space(8.0);
                                            ui.separator();
                                            ui.add_space(8.0);

                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new("Backspace Keycode Sequence").strong().color(self.theme.text_primary_color()));
                                                    ui.label(egui::RichText::new("Control character code sent to PTY upon pressing Backspace.").small().color(self.theme.text_muted_color()));
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
                                            ui.label(egui::RichText::new("SFTP & File Transfers").strong().size(16.0).color(self.theme.accent_color()));
                                            ui.label(egui::RichText::new("Manage remote directory traversal, split paneling, and file syncing.").small().color(self.theme.text_muted_color()));
                                            ui.add_space(12.0);

                                            changed |= setting_row_toggle(ui, "Split View SFTP Explorer", "Show terminal on the left and directory browser on the right.", &mut self.settings.show_sftp_split_view, &self.theme);

                                            setting_row_disabled(ui, "Synchronize SFTP with Terminal Path", "Automatically follow the current directory of the active shell.", self.settings.sftp_path_sync);
                                            setting_row_disabled(ui, "Auto Refresh on Tab Switch", "Query remote directory metadata when navigating between sessions.", self.settings.auto_refresh_sftp);
                                            setting_row_disabled(ui, "Show Hidden Dotfiles", "Display files and folders prefixed with a dot by default.", self.settings.show_hidden_sftp);
                                            setting_row_disabled(ui, "Disable SFTP Transfer History", "Do not write upload/download records to disk.", self.settings.disable_sftp_history);
                                        }
                                        SettingsCategory::System => {
                                            ui.label(egui::RichText::new("Application & System").strong().size(16.0).color(self.theme.accent_color()));
                                            ui.label(egui::RichText::new("Window behavior, multi-instance options, and update checks.").small().color(self.theme.text_muted_color()));
                                            ui.add_space(12.0);

                                            changed |= setting_row_toggle(ui, "Open Default Tab on Startup", "Spawn a fresh local shell if no previous session was restored.", &mut self.settings.open_default_tab, &self.theme);

                                            if setting_row_toggle(ui, "Check for Updates on Startup", "Check for newer releases on GitHub once daily.", &mut self.settings.check_updates, &self.theme) {
                                                changed = true;
                                                if !self.settings.check_updates {
                                                    self.available_update = None;
                                                    self.show_update_modal = false;
                                                }
                                            }

                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(egui::RichText::new("Manual Update Check").strong().color(self.theme.text_primary_color()));
                                                    let status_text = if self.is_checking_update {
                                                        "Checking GitHub releases...".to_string()
                                                    } else if let Some(ref tag) = self.available_update {
                                                        format!("New version available: {}", tag)
                                                    } else {
                                                        format!("Current version is v{}", env!("CARGO_PKG_VERSION"))
                                                    };
                                                    ui.label(egui::RichText::new(status_text).small().color(self.theme.text_muted_color()));
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
