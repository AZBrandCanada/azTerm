use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SshAuthType {
    PasswordOrAgent,
    KeyFile(String),
    PastedKey { key_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: SshAuthType,
    pub requires_2fa: bool,
    pub group_tag: String,
}

impl SshProfile {
    pub fn new(name: &str, host: &str, port: u16, username: &str) -> Self {
        Self {
            id: format!("{}-{}-{}", host, port, username),
            name: name.to_string(),
            host: host.to_string(),
            port,
            username: username.to_string(),
            auth_type: SshAuthType::PasswordOrAgent,
            requires_2fa: true,
            group_tag: "Default".to_string(),
        }
    }

    pub fn to_command(&self) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("ssh");
        cmd.arg("-p");
        cmd.arg(self.port.to_string());

        match &self.auth_type {
            SshAuthType::KeyFile(path) => {
                if !path.trim().is_empty() {
                    cmd.arg("-i");
                    cmd.arg(path.trim());
                }
            }
            SshAuthType::PastedKey { key_id } => {
                let key_path = SshStore::keys_dir().join(format!("{}.pem", key_id));
                if key_path.exists() {
                    cmd.arg("-i");
                    cmd.arg(key_path.to_string_lossy().to_string());
                }
            }
            SshAuthType::PasswordOrAgent => {}
        }

        cmd.arg(format!("{}@{}", self.username, self.host));
        cmd.env("TERM", "xterm-256color");
        cmd
    }
}

#[derive(Debug, Clone)]
pub struct SavedKeyEntry {
    pub file_name: String,
    pub priv_path: PathBuf,
    pub pub_key_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SshStore {
    pub profiles: Vec<SshProfile>,
}

impl SshStore {
    pub fn base_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/azterm")
    }

    pub fn keys_dir() -> PathBuf {
        Self::base_dir().join("keys")
    }

    pub fn path() -> PathBuf {
        Self::base_dir().join("ssh_profiles.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(store) = serde_json::from_str(&data) {
                return store;
            }
        }
        let mut default_store = Self::default();
        default_store.profiles.push(SshProfile::new(
            "Localhost Test",
            "127.0.0.1",
            22,
            "root",
        ));
        default_store
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, data);
        }
    }

    pub fn list_saved_keys() -> Vec<SavedKeyEntry> {
        let dir = Self::keys_dir();
        let mut list = Vec::new();
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                    if ext != "pub" {
                        let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                        let pub_path = dir.join(format!("{}.pub", file_name));
                        let pub_content = fs::read_to_string(&pub_path).ok();
                        list.push(SavedKeyEntry {
                            file_name,
                            priv_path: path,
                            pub_key_content: pub_content,
                        });
                    }
                }
            }
        }
        list.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        list
    }

    pub fn delete_key_files(file_name: &str) {
        let dir = Self::keys_dir();
        let priv_path = dir.join(file_name);
        let pub_path = dir.join(format!("{}.pub", file_name));
        let _ = fs::remove_file(&priv_path);
        let _ = fs::remove_file(&pub_path);
    }

    pub fn save_pasted_key(key_id: &str, content: &str) -> std::io::Result<PathBuf> {
        let dir = Self::keys_dir();
        fs::create_dir_all(&dir)?;
        let key_file = dir.join(format!("{}.pem", key_id));
        fs::write(&key_file, content.trim())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&key_file)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&key_file, perms)?;
        }

        Ok(key_file)
    }

    pub fn generate_ed25519_keypair(name: &str) -> Result<(String, String), String> {
        let dir = Self::keys_dir();
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

        let key_path = dir.join(format!("id_ed25519_{}", name));
        let pub_path = dir.join(format!("id_ed25519_{}.pub", name));

        if key_path.exists() {
            let _ = fs::remove_file(&key_path);
            let _ = fs::remove_file(&pub_path);
        }

        let output = Command::new("ssh-keygen")
            .arg("-t")
            .arg("ed25519")
            .arg("-f")
            .arg(&key_path)
            .arg("-N")
            .arg("")
            .arg("-C")
            .arg(format!("azterm-{}", name))
            .output()
            .map_err(|e| format!("Failed to execute ssh-keygen: {}", e))?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(&key_path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o600);
                let _ = fs::set_permissions(&key_path, perms);
            }
        }

        let pub_key = fs::read_to_string(&pub_path).map_err(|e| e.to_string())?;
        let priv_key_path = key_path.to_string_lossy().to_string();

        Ok((priv_key_path, pub_key))
    }
}
