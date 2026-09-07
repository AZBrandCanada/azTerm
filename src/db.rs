use crate::settings::AppSettings;
use crate::ssh::SshProfile;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSessionState {
    pub kind: String, // "local" or "ssh"
    pub title: String,
    pub target: String, // working directory for local, profile id for ssh
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
                tab_order INTEGER NOT NULL,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                target TEXT NOT NULL
            )",
            [],
        )?;

        Ok(())
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
                        "INSERT INTO open_sessions (tab_order, kind, title, target) VALUES (?1, ?2, ?3, ?4)",
                        params![idx as i32, s.kind, s.title, s.target],
                    );
                }
                let _ = tx.commit();
            }
        }
    }

    pub fn load_sessions() -> Vec<SavedSessionState> {
        let mut list = Vec::new();
        if let Some(conn) = Self::get_connection() {
            if let Ok(mut stmt) = conn.prepare("SELECT kind, title, target FROM open_sessions ORDER BY tab_order ASC") {
                if let Ok(rows) = stmt.query_map([], |row| {
                    Ok(SavedSessionState {
                        kind: row.get(0)?,
                        title: row.get(1)?,
                        target: row.get(2)?,
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
}
