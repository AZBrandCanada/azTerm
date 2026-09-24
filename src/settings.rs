// src/settings.rs
use crate::db::Database;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackspaceSequence {
    Delete127, // ^? (0x7F)
    Backspace8, // ^H (0x08)
}

fn default_scrollback_lines() -> usize {
    10000
}

fn default_zoom_factor() -> f32 {
    1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WheelScrollAmount {
    Lines1,
    Lines3,
    Lines5,
    Lines10,
    HalfPage,
    FullPage,
}

impl WheelScrollAmount {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Lines1 => "1 line per notch",
            Self::Lines3 => "3 lines per notch (default)",
            Self::Lines5 => "5 lines per notch",
            Self::Lines10 => "10 lines per notch",
            Self::HalfPage => "Half page per notch",
            Self::FullPage => "Full page per notch",
        }
    }

    /// Convert to concrete line count given the current terminal height.
    pub fn lines(&self, rows: u16) -> usize {
        let r = rows.max(1) as usize;
        match self {
            Self::Lines1 => 1,
            Self::Lines3 => 3,
            Self::Lines5 => 5,
            Self::Lines10 => 10,
            Self::HalfPage => (r / 2).max(1),
            Self::FullPage => r.saturating_sub(1).max(1),
        }
    }
}

impl Default for WheelScrollAmount {
    fn default() -> Self {
        Self::Lines3
    }
}

fn default_wheel_scroll() -> WheelScrollAmount {
    WheelScrollAmount::Lines3
}

fn default_debug_log_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{}/.config/azterm/debug.log", home)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub default_shell: String,
    pub auto_refresh_sftp: bool,
    pub show_hidden_sftp: bool,
    pub support_screen_reader: bool,
    pub open_default_tab: bool,
    pub disable_connection_history: bool,
    pub disable_sftp_history: bool,
    pub check_updates: bool,
    pub last_update_check_date: String,
    pub use_system_titlebar: bool,
    pub confirm_before_exit: bool,
    pub hide_ip: bool,
    pub allow_multi_instance: bool,
    pub disable_developer_tools: bool,
    pub debug_mode: bool,

    pub terminal_log_path: String,
    pub save_terminal_log: bool,
    pub add_timestamp_to_log: bool,
    pub cursor_blink: bool,
    pub right_click_select_word: bool,
    pub paste_on_right_click: bool,
    pub copy_on_select: bool,
    pub must_hold_ctrl_for_links: bool,
    pub sftp_path_sync: bool,
    pub show_sftp_split_view: bool,
    pub show_command_suggestions: bool,
    pub auto_reconnect_terminal: bool,
    pub backspace_sequence: BackspaceSequence,

    #[serde(default = "default_scrollback_lines")]
    pub scrollback_lines: usize,

    #[serde(default = "default_zoom_factor")]
    pub zoom_factor: f32,

    #[serde(default = "default_debug_log_path")]
    pub debug_log_path: String,

    #[serde(default = "default_wheel_scroll")]
    pub mouse_wheel_scroll: WheelScrollAmount,

    #[serde(default)]
    pub pending_update: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let default_shell = if cfg!(windows) {
            "powershell.exe".to_string()
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
        };

        Self {
            default_shell,
            auto_refresh_sftp: false,
            show_hidden_sftp: true,
            support_screen_reader: false,
            open_default_tab: true,
            disable_connection_history: false,
            disable_sftp_history: false,
            check_updates: true,
            last_update_check_date: String::new(),
            use_system_titlebar: false,
            confirm_before_exit: false,
            hide_ip: false,
            allow_multi_instance: true,
            disable_developer_tools: false,
            debug_mode: false,

            terminal_log_path: format!("{}/.config/azterm/session_logs", home),
            save_terminal_log: false,
            add_timestamp_to_log: false,
            cursor_blink: true,
            right_click_select_word: false,
            paste_on_right_click: true,
            copy_on_select: true,
            must_hold_ctrl_for_links: false,
            sftp_path_sync: true,
            show_sftp_split_view: false,
            show_command_suggestions: true,
            auto_reconnect_terminal: false,
            backspace_sequence: BackspaceSequence::Delete127,
            scrollback_lines: 10000,
            zoom_factor: 1.0,
            debug_log_path: default_debug_log_path(),
            mouse_wheel_scroll: WheelScrollAmount::Lines3,
            pending_update: None,
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        Database::load_settings().unwrap_or_default()
    }

    pub fn save(&self) {
        Database::save_settings(self);
    }
}
