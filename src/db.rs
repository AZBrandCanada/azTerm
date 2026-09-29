// src/db.rs
use crate::settings::AppSettings;
use crate::ssh::SshProfile;
use crate::theme::ThemeConfig;
use crate::tiling::WorkspaceTab;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSessionState {
    #[serde(default)]
    pub id: usize,
    pub kind: String,
    pub title: String,
    pub target: String,
}

pub struct Database;

impl Database {
    pub fn db_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let dir = PathBuf::from(home).join(".config/azterm");
        let _ = fs::create_dir_all(&dir);
        dir.join("azterm.db")
    }

    pub fn get_connection() -> Option<Connection> {
        let path = Self::db_path();
        let conn = Connection::open(path).ok()?;
        Self::init_tables(&conn).ok()?;
        Some(conn)
    }

    fn init_tables(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                data TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS ssh_profiles (
                id TEXT PRIMARY KEY,
                data TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS open_sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_uid INTEGER NOT NULL DEFAULT 0,
                tab_order INTEGER NOT NULL,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                target TEXT NOT NULL
            )",
            [],
        )?;

        // Migration: older DBs lack the session_uid column.
        let has_session_uid: bool = conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('open_sessions') WHERE name = 'session_uid'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if !has_session_uid {
            let _ = conn.execute(
                "ALTER TABLE open_sessions ADD COLUMN session_uid INTEGER NOT NULL DEFAULT 0",
                [],
            );
        }

        conn.execute(
            "CREATE TABLE IF NOT EXISTS custom_themes (
                id TEXT PRIMARY KEY,
                data TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS active_theme (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                data TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS saved_workspaces (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                data TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS ssh_last_paths (
                profile_id TEXT PRIMARY KEY,
                path TEXT NOT NULL
            )",
            [],
        )?;

        Ok(())
    }

    pub fn save_ssh_last_path(profile_id: &str, path: &str) {
        if let Some(conn) = Self::get_connection() {
            let _ = conn.execute(
                "INSERT INTO ssh_last_paths (profile_id, path) VALUES (?1, ?2)
                 ON CONFLICT(profile_id) DO UPDATE SET path = excluded.path",
                params![profile_id, path],
            );
        }
    }

    pub fn load_ssh_last_path(profile_id: &str) -> Option<String> {
        let conn = Self::get_connection()?;
        let mut stmt = conn.prepare("SELECT path FROM ssh_last_paths WHERE profile_id = ?1").ok()?;
        let path: String = stmt.query_row(params![profile_id], |row| row.get(0)).ok()?;
        if path.trim().is_empty() {
            None
        } else {
            Some(path)
        }
    }

    pub fn save_settings(settings: &AppSettings) {
        if let Some(conn) = Self::get_connection() {
            if let Ok(json) = serde_json::to_string(settings) {
                let _ = conn.execute(
                    "INSERT INTO settings (id, data) VALUES (1, ?1)
                     ON CONFLICT(id) DO UPDATE SET data = excluded.data",
                    params![json],
                );
            }
        }
    }

    pub fn load_settings() -> Option<AppSettings> {
        let conn = Self::get_connection()?;
        let mut stmt = conn.prepare("SELECT data FROM settings WHERE id = 1").ok()?;
        let json: String = stmt.query_row([], |row| row.get(0)).ok()?;
        serde_json::from_str(&json).ok()
    }

    pub fn save_profiles(profiles: &[SshProfile]) {
        if let Some(mut conn) = Self::get_connection() {
            if let Ok(tx) = conn.transaction() {
                let _ = tx.execute("DELETE FROM ssh_profiles", []);
                for p in profiles {
                    if let Ok(json) = serde_json::to_string(p) {
                        let _ = tx.execute(
                            "INSERT INTO ssh_profiles (id, data) VALUES (?1, ?2)",
                            params![p.id, json],
                        );
                    }
                }
                let _ = tx.commit();
            }
        }
    }

    pub fn load_profiles() -> Option<Vec<SshProfile>> {
        let conn = Self::get_connection()?;
        let mut stmt = conn.prepare("SELECT data FROM ssh_profiles").ok()?;
        let rows = stmt.query_map([], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        }).ok()?;

        let mut list = Vec::new();
        for r in rows.flatten() {
            if let Ok(profile) = serde_json::from_str(&r) {
                list.push(profile);
            }
        }
        if list.is_empty() {
            None
        } else {
            Some(list)
        }
    }

    pub fn save_sessions(sessions: &[SavedSessionState]) {
        if let Some(mut conn) = Self::get_connection() {
            if let Ok(tx) = conn.transaction() {
                let _ = tx.execute("DELETE FROM open_sessions", []);
                for (idx, s) in sessions.iter().enumerate() {
                    let _ = tx.execute(
                        "INSERT INTO open_sessions (session_uid, tab_order, kind, title, target)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![s.id as i64, idx as i32, s.kind, s.title, s.target],
                    );
                }
                let _ = tx.commit();
            }
        }
    }

    pub fn load_sessions() -> Vec<SavedSessionState> {
        let mut list = Vec::new();
        if let Some(conn) = Self::get_connection() {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT session_uid, kind, title, target
                 FROM open_sessions ORDER BY tab_order ASC",
            ) {
                if let Ok(rows) = stmt.query_map([], |row| {
                    Ok(SavedSessionState {
                        id: row.get::<_, i64>(0).unwrap_or(0) as usize,
                        kind: row.get(1)?,
                        title: row.get(2)?,
                        target: row.get(3)?,
                    })
                }) {
                    for r in rows.flatten() {
                        list.push(r);
                    }
                }
            }
        }
        list
    }

    pub fn save_active_theme(theme: &ThemeConfig) {
        if let Some(conn) = Self::get_connection() {
            if let Ok(json) = serde_json::to_string(theme) {
                let _ = conn.execute(
                    "INSERT INTO active_theme (id, data) VALUES (1, ?1)
                     ON CONFLICT(id) DO UPDATE SET data = excluded.data",
                    params![json],
                );
            }
        }
    }

    pub fn load_active_theme() -> Option<ThemeConfig> {
        let conn = Self::get_connection()?;
        let mut stmt = conn.prepare("SELECT data FROM active_theme WHERE id = 1").ok()?;
        let json: String = stmt.query_row([], |row| row.get(0)).ok()?;
        serde_json::from_str(&json).ok()
    }

    pub fn save_custom_themes(themes: &[ThemeConfig]) {
        if let Some(mut conn) = Self::get_connection() {
            if let Ok(tx) = conn.transaction() {
                let _ = tx.execute("DELETE FROM custom_themes", []);
                for t in themes {
                    if let Ok(json) = serde_json::to_string(t) {
                        let _ = tx.execute(
                            "INSERT INTO custom_themes (id, data) VALUES (?1, ?2)",
                            params![t.id, json],
                        );
                    }
                }
                let _ = tx.commit();
            }
        }
    }

    pub fn load_custom_themes() -> Vec<ThemeConfig> {
        let mut list = Vec::new();
        if let Some(conn) = Self::get_connection() {
            if let Ok(mut stmt) = conn.prepare("SELECT data FROM custom_themes") {
                if let Ok(rows) = stmt.query_map([], |row| {
                    let json: String = row.get(0)?;
                    Ok(json)
                }) {
                    for r in rows.flatten() {
                        if let Ok(theme) = serde_json::from_str(&r) {
                            list.push(theme);
                        }
                    }
                }
            }
        }
        list
    }

    pub fn save_workspaces(workspaces: &[WorkspaceTab]) {
        if let Some(conn) = Self::get_connection() {
            if let Ok(json) = serde_json::to_string(workspaces) {
                let _ = conn.execute(
                    "INSERT INTO saved_workspaces (id, data) VALUES (1, ?1)
                     ON CONFLICT(id) DO UPDATE SET data = excluded.data",
                    params![json],
                );
            }
        }
    }

    pub fn load_workspaces() -> Option<Vec<WorkspaceTab>> {
        let conn = Self::get_connection()?;
        let mut stmt = conn.prepare("SELECT data FROM saved_workspaces WHERE id = 1").ok()?;
        let json: String = stmt.query_row([], |row| row.get(0)).ok()?;
        serde_json::from_str(&json).ok()
    }
}
