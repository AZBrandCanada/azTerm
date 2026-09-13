// src/ssh.rs
use crate::db::Database;
use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SshAuthType {
    PasswordOrAgent,
    KeyFile(String),
    PastedKey { key_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshKeyAlgorithm {
    Ed25519,
    Rsa4096,
    Ecdsa384,
    Ecdsa256,
}

impl SshKeyAlgorithm {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Ed25519 => "Ed25519 (Recommended, Fast & Secure)",
            Self::Rsa4096 => "RSA 4096-bit (Universal Compatibility)",
            Self::Ecdsa384 => "ECDSA 384-bit (NIST P-384)",
            Self::Ecdsa256 => "ECDSA 256-bit (NIST P-256)",
        }
    }

    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Ed25519 => "id_ed25519",
            Self::Rsa4096 => "id_rsa",
            Self::Ecdsa384 => "id_ecdsa384",
            Self::Ecdsa256 => "id_ecdsa256",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
        let socket_dir = SshStore::sockets_dir();
        let socket_path = socket_dir.join(format!("{}.sock", self.id));

        let mut cmd = CommandBuilder::new("ssh");

        // Enable SSH Connection Multiplexing so SFTP shares this authenticated session
        cmd.arg("-o");
        cmd.arg("ControlMaster=auto");
        cmd.arg("-o");
        cmd.arg(format!("ControlPath={}", socket_path.to_string_lossy()));
        cmd.arg("-o");
        cmd.arg("ControlPersist=10m");
        cmd.arg("-p");
        cmd.arg(self.port.to_string());

        match &self.auth_type {
            SshAuthType::KeyFile(path) => {
                if !path.trim().is_empty() {
                    SshStore::ensure_secure_permissions(path);
                    cmd.arg("-i");
                    cmd.arg(path.trim());
                }
            }
            SshAuthType::PastedKey { key_id } => {
                let key_path = SshStore::keys_dir().join(format!("{}.pem", key_id));
                if key_path.exists() {
                    SshStore::ensure_secure_permissions(&key_path.to_string_lossy());
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
        let dir = Self::base_dir().join("keys");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    pub fn sockets_dir() -> PathBuf {
        let dir = Self::base_dir().join("sockets");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    pub fn ensure_secure_permissions(path_str: &str) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = Path::new(path_str);
            if p.exists() {
                if let Ok(meta) = fs::metadata(p) {
                    let mut perms = meta.permissions();
                    let mode = perms.mode() & 0o777;
                    if mode != 0o600 && mode != 0o400 {
                        perms.set_mode(0o600);
                        let _ = fs::set_permissions(p, perms);
                    }
                }
            }
        }
    }

    pub fn load() -> Self {
        if let Some(profiles) = Database::load_profiles() {
            Self { profiles }
        } else {
            let mut default_store = Self::default();
            default_store.profiles.push(SshProfile::new(
                "Localhost Test",
                "127.0.0.1",
                22,
                "root",
            ));
            default_store.save();
            default_store
        }
    }

    pub fn save(&self) {
        Database::save_profiles(&self.profiles);
    }

    pub fn list_saved_keys() -> Vec<SavedKeyEntry> {
        let dir = Self::keys_dir();
        let mut list = Vec::new();
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                    if ext != "pub" && ext != "sock" {
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
        let key_file = dir.join(format!("{}.pem", key_id));
        fs::write(&key_file, content.trim())?;
        Self::ensure_secure_permissions(&key_file.to_string_lossy());
        Ok(key_file)
    }

    pub fn generate_keypair(name: &str, algo: SshKeyAlgorithm) -> Result<(String, String), String> {
        let clean_name = name.trim().replace(' ', "_");
        if clean_name.is_empty() {
            return Err("Key identifier name cannot be empty".to_string());
        }

        let dir = Self::keys_dir();
        let key_file_name = format!("{}_{}", algo.prefix(), clean_name);
        let key_path = dir.join(&key_file_name);
        let pub_path = dir.join(format!("{}.pub", key_file_name));

        if key_path.exists() {
            let _ = fs::remove_file(&key_path);
            let _ = fs::remove_file(&pub_path);
        }

        let mut cmd = Command::new("ssh-keygen");
        match algo {
            SshKeyAlgorithm::Ed25519 => {
                cmd.arg("-t").arg("ed25519");
            }
            SshKeyAlgorithm::Rsa4096 => {
                cmd.arg("-t").arg("rsa").arg("-b").arg("4096");
            }
            SshKeyAlgorithm::Ecdsa384 => {
                cmd.arg("-t").arg("ecdsa").arg("-b").arg("384");
            }
            SshKeyAlgorithm::Ecdsa256 => {
                cmd.arg("-t").arg("ecdsa").arg("-b").arg("256");
            }
        }

        cmd.arg("-f")
            .arg(&key_path)
            .arg("-N")
            .arg("")
            .arg("-C")
            .arg(format!("azterm-{}", clean_name));

        let output = cmd.output().map_err(|e| format!("Failed to execute ssh-keygen: {}", e))?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }

        Self::ensure_secure_permissions(&key_path.to_string_lossy());

        let pub_key = fs::read_to_string(&pub_path).map_err(|e| e.to_string())?;
        let priv_key_path = key_path.to_string_lossy().to_string();

        Ok((priv_key_path, pub_key))
    }

    pub fn generate_ed25519_keypair(name: &str) -> Result<(String, String), String> {
        Self::generate_keypair(name, SshKeyAlgorithm::Ed25519)
    }
}
