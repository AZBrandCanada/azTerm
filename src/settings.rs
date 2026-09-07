use crate::db::Database;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackspaceSequence {
    Delete127, // ^? (0x7F)
    Backspace8, // ^H (0x08)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    // Shell Configuration
    pub default_shell: String,

    // 2FA / Auth
    pub two_factor_keywords: String,

    // General & Application
    pub auto_refresh_sftp: bool,
    pub show_hidden_sftp: bool,
    pub support_screen_reader: bool,
    pub open_default_tab: bool,
    pub disable_connection_history: bool,
    pub disable_sftp_history: bool,
    pub check_updates: bool,
    pub use_system_titlebar: bool,
    pub confirm_before_exit: bool,
    pub hide_ip: bool,
    pub allow_multi_instance: bool,
    pub disable_developer_tools: bool,
    pub debug_mode: bool,

    // Terminal Interaction
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
            two_factor_keywords: "verification code,otp,one-time,two-factor,2fa,totp,authenticator,duo,yubikey".to_string(),
            auto_refresh_sftp: false,
            show_hidden_sftp: true,
            support_screen_reader: false,
            open_default_tab: true,
            disable_connection_history: false,
            disable_sftp_history: false,
            check_updates: false,
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
