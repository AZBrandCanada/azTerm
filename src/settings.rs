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
            sftp_path_sync: false,
            show_sftp_split_view: false,
            show_command_suggestions: true,
            auto_reconnect_terminal: false,
            backspace_sequence: BackspaceSequence::Delete127,
            scrollback_lines: 10000,
            zoom_factor: 1.0,
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
