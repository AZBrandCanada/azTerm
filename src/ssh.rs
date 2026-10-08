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

        // Resilient SSH keepalive & connection management
        cmd.arg("-o");
        cmd.arg("ControlMaster=auto");
        cmd.arg("-o");
        cmd.arg(format!("ControlPath={}", socket_path.to_string_lossy()));
        cmd.arg("-o");
        cmd.arg("ControlPersist=5m");
        cmd.arg("-o");
        cmd.arg("ServerAliveInterval=15");
        cmd.arg("-o");
        cmd.arg("ServerAliveCountMax=3");
        cmd.arg("-o");
        cmd.arg("ConnectTimeout=10");
        cmd.arg("-o");
        cmd.arg("TCPKeepAlive=yes");

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

    pub fn cleanup_stale_socket(profile_id: &str) {
        let t0 = std::time::Instant::now();
        crate::dbg_log!("ssh_cleanup_stale begin profile={}", profile_id);
        let socket_path = Self::sockets_dir().join(format!("{}.sock", profile_id));
        if socket_path.exists() {
            let output = Command::new("ssh")
                .args([
                    "-O", "check",
                    "-o", &format!("ControlPath={}", socket_path.to_string_lossy()),
                    "dummy_check_target",
                ])
                .output();

            match output {
                Ok(out) if out.status.success() => {
                    // Socket is healthy and actively serving
                }
                _ => {
                    // Master process terminated or socket is dead; remove file
                    let _ = fs::remove_file(&socket_path);
                }
            }
        }
        crate::dbg_log!(
            "ssh_cleanup_stale end profile={} elapsed_ms={}",
            profile_id,
            t0.elapsed().as_millis()
        );
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

    /// Does `s` plausibly look like an armored PEM private key?
    /// Used to reject obvious paste mistakes (a public key, a shell
    /// transcript, a PuTTY .ppk, markdown junk) before we write a
    /// file that ssh will later reject with "invalid format".
    pub fn looks_like_private_key(s: &str) -> bool {
        let t = s.trim_start_matches('\u{FEFF}').trim_start();
        // Accept both modern OpenSSH armor and legacy RSA/EC/DSA armor.
        t.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
            || t.starts_with("-----BEGIN RSA PRIVATE KEY-----")
            || t.starts_with("-----BEGIN EC PRIVATE KEY-----")
            || t.starts_with("-----BEGIN DSA PRIVATE KEY-----")
            || t.starts_with("-----BEGIN PRIVATE KEY-----")
            || t.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----")
    }

    /// Normalize a pasted key so OpenSSH's PEM parser accepts it.
    ///
    /// Handles the damage that a plain clipboard paste actually
    /// introduces in the wild:
    ///   * Windows / browser CRLF inside the body (ssh 8.x rejects
    ///     stray CRs in the base64 payload);
    ///   * a leading UTF-8 BOM from certain web pages (Rust's
    ///     str::trim does NOT strip U+FEFF);
    ///   * markdown ``` or ~~~ fences when copying from docs;
    ///   * a missing trailing newline (ssh-keygen always emits one).
    pub fn normalize_pasted_key(raw: &str) -> String {
        // 1. Strip BOM, then optional markdown fence.
        let mut s = raw.trim_start_matches('\u{FEFF}').trim();

        for fence in ["```", "~~~"] {
            if s.starts_with(fence) {
                if let Some(rest) = s.strip_prefix(fence) {
                    // Drop the remainder of the opening line (may be
                    // a language tag, e.g. ```openssh).
                    let body = rest.split_once('\n').map(|(_, b)| b).unwrap_or(rest);
                    if let Some(idx) = body.rfind(fence) {
                        s = body[..idx].trim();
                    } else {
                        s = body.trim();
                    }
                }
                break;
            }
        }

        // 2. Line endings: CRLF -> LF, lone CR -> LF.
        let mut out = String::with_capacity(s.len() + 1);
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            if c == '\r' {
                out.push('\n');
                if it.peek() == Some(&'\n') {
                    it.next();
                }
            } else {
                out.push(c);
            }
        }

        // 3. Trailing newline. ssh tolerates its absence, but every
        //    ssh-keygen output ends with one, and some tooling (and
        //    any user who later `cat`s the file) expects it.
        if !out.ends_with('\n') {
            out.push('\n');
        }

        out
    }

    pub fn save_pasted_key(key_id: &str, content: &str) -> std::io::Result<PathBuf> {
        let dir = Self::keys_dir();
        let key_file = dir.join(format!("{}.pem", key_id));

        let normalized = Self::normalize_pasted_key(content);

        if !Self::looks_like_private_key(&normalized) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Pasted text does not look like a PEM private key \
                 (expected a -----BEGIN ... PRIVATE KEY----- header)",
            ));
        }

        crate::dbg_log!(
            "save_pasted_key key_id={} raw_len={} normalized_len={} lines={}",
            key_id,
            content.len(),
            normalized.len(),
            normalized.lines().count()
        );

        fs::write(&key_file, &normalized)?;
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
}
