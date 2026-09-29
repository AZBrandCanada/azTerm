// src/daemon.rs
//
// Shared protocol types for the azterm-daemon backend.
//
// Framing: every message is [u32 BE length][serde_json bytes]. The
// control channel carries Request/Response frames; after an Attach
// handshake, the same channel streams Response::Output frames one-way
// and accepts Request::Input / Request::Resize the other way.
//
// This file is compiled into BOTH the GUI binary (as `mod daemon`) and
// the daemon binary (via #[path] include). It must therefore not
// reference any other module in the crate — only serde + std.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_COLS: u16 = 120;
pub const DEFAULT_ROWS: u16 = 40;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: u64,
    pub title: String,
    /// "local" | "ssh"
    pub kind: String,
    /// cwd for local, profile-id for ssh
    pub target: String,
    pub cols: u16,
    pub rows: u16,
    pub alive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshSpec {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub identity_file: Option<String>,
    pub control_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Ping,
    List,
    NewLocal {
        id: u64,
        title: String,
        cwd: Option<String>,
        shell: String,
        cols: u16,
        rows: u16,
    },
    NewSsh {
        id: u64,
        title: String,
        spec: SshSpec,
        cols: u16,
        rows: u16,
    },
    Kill {
        id: u64,
    },
    Attach {
        id: u64,
        cols: u16,
        rows: u16,
    },
    Input {
        /// base64-encoded raw PTY bytes
        bytes_b64: String,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Pong,
    Sessions(Vec<SessionInfo>),
    Attached { id: u64 },
    /// base64-encoded raw PTY bytes
    Output { bytes_b64: String },
    /// Session PTY closed.
    Closed,
    Ok,
    Error(String),
}

pub fn socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".config/azterm");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("daemon.sock")
}

// ---- minimal base64 (RFC 4648) --------------------------------------------
// We use base64 inside the JSON envelope because serde would otherwise
// serialize Vec<u8> as a JSON array of numbers — ~5x bloat on the wire.
// Terminal throughput is high enough that this matters.

pub fn b64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            CHARS[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            CHARS[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

pub fn b64_decode(s: &str) -> Vec<u8> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut buf = [0u32; 4];
    let mut len = 0;
    for &c in bytes {
        if c == b'=' {
            break;
        }
        if let Some(v) = val(c) {
            buf[len] = v;
            len += 1;
            if len == 4 {
                let n = (buf[0] << 18) | (buf[1] << 12) | (buf[2] << 6) | buf[3];
                out.push((n >> 16) as u8);
                out.push((n >> 8) as u8);
                out.push(n as u8);
                len = 0;
            }
        }
    }
    if len == 2 {
        let n = (buf[0] << 18) | (buf[1] << 12);
        out.push((n >> 16) as u8);
    } else if len == 3 {
        let n = (buf[0] << 18) | (buf[1] << 12) | (buf[2] << 6);
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
    }
    out
}

pub fn write_msg<W: std::io::Write, T: Serialize>(w: &mut W, msg: &T) -> std::io::Result<()> {
    let data = serde_json::to_vec(msg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = data.len() as u32;
    w.write_all(&len.to_be_bytes())?;
    w.write_all(&data)?;
    w.flush()?;
    Ok(())
}

pub fn read_msg<R: std::io::Read, T: for<'a> Deserialize<'a>>(
    r: &mut R,
) -> std::io::Result<T> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    serde_json::from_slice(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
