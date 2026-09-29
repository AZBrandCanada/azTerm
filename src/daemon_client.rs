// src/daemon_client.rs
//
// GUI-side client for azterm-daemon. One DaemonClient owns a single
// control connection for List/New/Kill. Each attach() call opens its
// own dedicated Unix socket for streaming, so a slow session never
// blocks a control request.

use crate::daemon::{read_msg, write_msg, Request, Response, SessionInfo, SshSpec, socket_path};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

pub struct DaemonClient {
    ctrl: Mutex<UnixStream>,
}

pub struct AttachedSession {
    pub out_rx: Receiver<Vec<u8>>,
    pub in_tx: SyncSender<Vec<u8>>,
    pub resize_tx: SyncSender<(u16, u16)>,
    pub id: u64,
}

impl DaemonClient {
    pub fn try_connect() -> Option<Self> {
        let path = socket_path();
        if !path.exists() {
            return None;
        }
        let mut s = UnixStream::connect(&path).ok()?;
        s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        s.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
        write_msg(&mut s, &Request::Ping).ok()?;
        match read_msg::<_, Response>(&mut s) {
            Ok(Response::Pong) => {}
            _ => return None,
        }
        s.set_read_timeout(None).ok()?;
        s.set_write_timeout(None).ok()?;
        Some(Self {
            ctrl: Mutex::new(s),
        })
    }

    fn send(&self, req: &Request) -> Option<Response> {
        let mut s = self.ctrl.lock().ok()?;
        write_msg(&mut *s, req).ok()?;
        read_msg::<_, Response>(&mut *s).ok()
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        match self.send(&Request::List) {
            Some(Response::Sessions(v)) => v,
            _ => Vec::new(),
        }
    }

    pub fn new_local(
        &self,
        id: u64,
        title: &str,
        cwd: Option<&str>,
        shell: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        let req = Request::NewLocal {
            id,
            title: title.to_string(),
            cwd: cwd.map(String::from),
            shell: shell.to_string(),
            cols,
            rows,
        };
        match self.send(&req) {
            Some(Response::Ok) => Ok(()),
            Some(Response::Error(e)) => Err(e),
            _ => Err("no response from daemon".into()),
        }
    }

    pub fn new_ssh(
        &self,
        id: u64,
        title: &str,
        spec: SshSpec,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        let req = Request::NewSsh {
            id,
            title: title.to_string(),
            spec,
            cols,
            rows,
        };
        match self.send(&req) {
            Some(Response::Ok) => Ok(()),
            Some(Response::Error(e)) => Err(e),
            _ => Err("no response from daemon".into()),
        }
    }

    pub fn kill(&self, id: u64) {
        let _ = self.send(&Request::Kill { id });
    }

    pub fn attach(&self, id: u64, cols: u16, rows: u16) -> Result<AttachedSession, String> {
        let path = socket_path();
        let stream = UnixStream::connect(&path).map_err(|e| e.to_string())?;
        let mut write_stream = stream.try_clone().map_err(|e| e.to_string())?;
        let mut read_stream = stream;

        write_msg(&mut write_stream, &Request::Attach { id, cols, rows })
            .map_err(|e| e.to_string())?;

        match read_msg::<_, Response>(&mut read_stream) {
            Ok(Response::Attached { .. }) => {}
            Ok(Response::Error(e)) => return Err(e),
            _ => return Err("attach handshake failed".into()),
        }

        let (out_tx, out_rx): (SyncSender<Vec<u8>>, Receiver<Vec<u8>>) = sync_channel(1024);
        let (in_tx, in_rx): (SyncSender<Vec<u8>>, Receiver<Vec<u8>>) = sync_channel(4096);
        let (rsz_tx, rsz_rx): (SyncSender<(u16, u16)>, Receiver<(u16, u16)>) = sync_channel(64);

        // Reader thread: daemon -> out_rx. An empty Vec is the EOF sentinel.
        thread::spawn(move || loop {
            match read_msg::<_, Response>(&mut read_stream) {
                Ok(Response::Output { bytes_b64 }) => {
                    let bytes = crate::daemon::b64_decode(&bytes_b64);
                    if out_tx.send(bytes).is_err() {
                        break;
                    }
                }
                Ok(Response::Closed) | Err(_) => {
                    let _ = out_tx.send(Vec::new());
                    break;
                }
                _ => {}
            }
        });

        // Writer thread: input + resize, interleaved via short recv timeout.
        thread::spawn(move || loop {
            match in_rx.recv_timeout(Duration::from_millis(20)) {
                Ok(bytes) => {
                    let b64 = crate::daemon::b64_encode(&bytes);
                    if write_msg(&mut write_stream, &Request::Input { bytes_b64: b64 }).is_err() {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            while let Ok((cols, rows)) = rsz_rx.try_recv() {
                if write_msg(&mut write_stream, &Request::Resize { cols, rows }).is_err() {
                    break;
                }
            }
        });

        Ok(AttachedSession {
            out_rx,
            in_tx,
            resize_tx: rsz_tx,
            id,
        })
    }
}

/// Try to reach a running daemon. If none is running, spawn the
/// azterm-daemon sibling binary and wait up to ~1s for it to bind.
/// Returns the connectable client if one was or could be started.
pub fn ensure_daemon_running() -> Option<DaemonClient> {
    // Retry a few times before spawning. A daemon that is mid-stream
    // on a PTY read may not answer our initial Ping within a single
    // 2-second window; treating that as "no daemon" caused duplicate
    // daemons in the wild. Three tries over ~600 ms is enough to
    // distinguish "busy" from "actually absent."
    for attempt in 0..3 {
        if let Some(c) = DaemonClient::try_connect() {
            return Some(c);
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    // Re-exec THIS binary with --daemon. Single-binary install: the
    // same executable is both the GUI and the daemon, so nothing else
    // needs to be shipped alongside it.
    let exe = std::env::current_exe().ok()?;

    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(&exe);
    let _ = std::fs::remove_file("/tmp/azterm-daemon.log");
    let stderr_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/azterm-daemon.log")
        .ok();
    cmd.arg("--daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    if let Some(f) = stderr_file {
        cmd.stderr(f);
    } else {
        cmd.stderr(Stdio::null());
    }
    unsafe {
        cmd.pre_exec(|| {
            // Detach from the parent's controlling terminal so the
            // daemon survives the GUI process exiting.
            libc::setsid();
            Ok(())
        });
    }
    let _ = cmd.spawn();

    // Wait for the freshly-spawned daemon to bind and answer a Ping.
    // 40 attempts x 50 ms = up to 2 s, plenty for a cold process.
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(50));
        if let Some(c) = DaemonClient::try_connect() {
            return Some(c);
        }
    }
    eprintln!("[daemon_client] timed out waiting for daemon to start");
    None
}
